//! The pixel work: decoding, effects, SVG, GIF frames, compression.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::{parse_key, Effect};

/// Bytes a host draws: the part's own, or ones made for a key.
#[derive(Clone, Debug)]
pub enum Bytes<'a> {
    Part(&'a [u8]),
    Made(Rc<Vec<u8>>),
}

impl std::ops::Deref for Bytes<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Part(bytes) => bytes,
            Self::Made(bytes) => bytes,
        }
    }
}

struct CachedImage {
    source: Box<[u8]>,
    made: Option<Rc<Vec<u8>>>,
}

thread_local! {
    static MADE: RefCell<HashMap<String, CachedImage>> = RefCell::new(HashMap::new());
}

/// Whether `bytes` are an SVG document.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    text.contains("<svg") || (text.trim_start().starts_with("<?xml") && text.contains("svg"))
}

/// The bytes to draw for `key`, the image parts looked up by `lookup`: an
/// SVG part drawn to a PNG, the effects applied, cached per thread while the
/// source bytes are unchanged. Part names alone do not identify an image:
/// different documents commonly reuse the same name.
pub fn image_bytes<'a>(key: &str, lookup: impl Fn(&str) -> Option<&'a [u8]>) -> Option<Bytes<'a>> {
    let (part, effects) = parse_key(key);
    let source = lookup(part)?;
    let svg = part.to_ascii_lowercase().ends_with(".svg") || is_svg(source);
    if effects.is_empty() && !svg {
        return Some(Bytes::Part(source));
    }
    if let Some(made) = MADE.with(|cache| {
        cache
            .borrow()
            .get(key)
            .filter(|cached| cached.source.as_ref() == source)
            .map(|cached| cached.made.clone())
    }) {
        return made.map(Bytes::Made);
    }
    let made = make(source, svg, &effects).map(Rc::new);
    MADE.with(|cache| {
        let mut cache = cache.borrow_mut();
        // Keep the cache from growing without bound.
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(
            key.to_string(),
            CachedImage {
                source: source.into(),
                made: made.clone(),
            },
        );
    });
    made.map(Bytes::Made)
}

fn make(source: &[u8], svg: bool, effects: &[Effect]) -> Option<Vec<u8>> {
    let mut pixels = if svg {
        rasterize_svg(source, 1600)?
    } else if let Some(Effect::Frame(index)) =
        effects.iter().find(|e| matches!(e, Effect::Frame(_)))
    {
        gif_frames(source)?.into_iter().nth(*index)?.0
    } else {
        decode(source)?
    };
    apply(&mut pixels, effects);
    encode_png(&pixels)
}

/// Decodes PNG, JPEG or GIF bytes to RGBA.
pub fn decode(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.to_rgba8())
}

pub fn encode_png(pixels: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

/// A picture's size in px without decoding it: a bitmap's header, or an
/// SVG's declared size.
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if is_svg(bytes) {
        return svg_size(bytes).map(|(w, h)| (w.round() as u32, h.round() as u32));
    }
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// An SVG drawn to RGBA, its longer side `longest` px.
pub fn rasterize_svg(bytes: &[u8], longest: u32) -> Option<image::RgbaImage> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = longest as f32 / size.width().max(size.height()).max(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // Premultiplied to straight alpha.
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(width, height, data)
}

/// The size an SVG declares, in its own units (px).
pub fn svg_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    Some((tree.size().width(), tree.size().height()))
}

/// An animated GIF's frames with their delays (ms); a still picture is one
/// frame.
pub fn gif_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, u32)>> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    Some(
        frames
            .into_iter()
            .map(|frame| {
                let (numerator, denominator) = frame.delay().numer_denom_ms();
                let delay = numerator / denominator.max(1);
                // Browsers and PowerPoint play a 0 delay at 100 ms.
                (frame.into_buffer(), if delay < 20 { 100 } else { delay })
            })
            .collect(),
    )
}

fn luminance(pixel: &[u8]) -> f64 {
    (0.299 * f64::from(pixel[0]) + 0.587 * f64::from(pixel[1]) + 0.114 * f64::from(pixel[2]))
        / 255.0
}

fn luminance_contrast(pixels: &mut image::RgbaImage, brightness: f64, contrast: f64) {
    // Contrast scales about mid-grey: +100% is a hard step, −100% flat
    // grey; brightness then shifts every channel.
    let factor = if contrast >= 0.0 {
        1.0 / (1.0 - contrast.min(0.999))
    } else {
        1.0 + contrast.max(-1.0)
    };
    for pixel in pixels.pixels_mut() {
        for channel in 0..3 {
            let value = f64::from(pixel[channel]) / 255.0;
            let value = (value - 0.5) * factor + 0.5 + brightness;
            pixel[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// Applies `effects` in order.
pub fn apply(pixels: &mut image::RgbaImage, effects: &[Effect]) {
    for effect in effects {
        match effect {
            Effect::Luminance {
                brightness,
                contrast,
            }
            | Effect::BrightnessContrast {
                brightness,
                contrast,
            } => luminance_contrast(pixels, *brightness, *contrast),
            Effect::Grayscale => {
                for pixel in pixels.pixels_mut() {
                    let value = (luminance(&pixel.0) * 255.0).round() as u8;
                    pixel[0] = value;
                    pixel[1] = value;
                    pixel[2] = value;
                }
            }
            Effect::BiLevel { threshold } => {
                for pixel in pixels.pixels_mut() {
                    let value = if luminance(&pixel.0) >= *threshold {
                        255
                    } else {
                        0
                    };
                    pixel[0] = value;
                    pixel[1] = value;
                    pixel[2] = value;
                }
            }
            Effect::Duotone { dark, light } => {
                for pixel in pixels.pixels_mut() {
                    let level = luminance(&pixel.0);
                    for channel in 0..3 {
                        let (d, l) = (f64::from(dark[channel]), f64::from(light[channel]));
                        pixel[channel] = (d + (l - d) * level).clamp(0.0, 255.0).round() as u8;
                    }
                }
            }
            Effect::ColorChange { from, to, alpha } => {
                // Matched with a small tolerance, so a JPEG's noisy flat
                // colour still goes.
                const TOLERANCE: i32 = 12;
                for pixel in pixels.pixels_mut() {
                    let near = (0..3)
                        .all(|c| (i32::from(pixel[c]) - i32::from(from[c])).abs() <= TOLERANCE);
                    if near {
                        pixel[0] = to[0];
                        pixel[1] = to[1];
                        pixel[2] = to[2];
                        pixel[3] = ((u32::from(pixel[3]) * u32::from(*alpha)) / 255) as u8;
                    }
                }
            }
            Effect::Saturation(amount) => saturate(pixels, *amount),
            Effect::Temperature(kelvin) => {
                let shift = ((kelvin - 6500.0) / 5000.0).clamp(-1.0, 1.0) * 40.0;
                for pixel in pixels.pixels_mut() {
                    pixel[0] = (f64::from(pixel[0]) + shift).clamp(0.0, 255.0).round() as u8;
                    pixel[2] = (f64::from(pixel[2]) - shift).clamp(0.0, 255.0).round() as u8;
                }
            }
            Effect::Sharpen(amount) => sharpen(pixels, *amount),
            Effect::Artistic(name) => artistic(pixels, name),
            Effect::BackgroundRemoval(marquee) => {
                remove_background(pixels, *marquee);
            }
            Effect::Frame(_) => {}
        }
    }
}

/// 色の彩度: 0 grey, 1 unchanged, up to 4.
pub fn saturate(pixels: &mut image::RgbaImage, amount: f64) {
    for pixel in pixels.pixels_mut() {
        let grey = luminance(&pixel.0) * 255.0;
        for channel in 0..3 {
            let value = grey + (f64::from(pixel[channel]) - grey) * amount;
            pixel[channel] = value.clamp(0.0, 255.0).round() as u8;
        }
    }
}

/// A box blur of `radius` px, run twice (close to a Gaussian).
pub fn blur(pixels: &mut image::RgbaImage, radius: u32) {
    if radius == 0 || pixels.width() == 0 || pixels.height() == 0 {
        return;
    }
    for _ in 0..2 {
        for horizontal in [true, false] {
            let source = pixels.clone();
            let (width, height) = (i64::from(source.width()), i64::from(source.height()));
            for y in 0..height {
                for x in 0..width {
                    let mut sum = [0u32; 4];
                    let mut count = 0;
                    for offset in -i64::from(radius)..=i64::from(radius) {
                        let (sx, sy) = if horizontal {
                            ((x + offset).clamp(0, width - 1), y)
                        } else {
                            (x, (y + offset).clamp(0, height - 1))
                        };
                        let pixel = source.get_pixel(sx as u32, sy as u32);
                        for channel in 0..4 {
                            sum[channel] += u32::from(pixel[channel]);
                        }
                        count += 1;
                    }
                    let pixel = pixels.get_pixel_mut(x as u32, y as u32);
                    for channel in 0..4 {
                        pixel[channel] = (sum[channel] / count) as u8;
                    }
                }
            }
        }
    }
}

/// シャープネス: unsharp masking above 0, a blur below.
pub fn sharpen(pixels: &mut image::RgbaImage, amount: f64) {
    if amount.abs() < 0.01 {
        return;
    }
    let radius = (pixels.width().max(pixels.height()) / 300).clamp(1, 4);
    let mut blurred = pixels.clone();
    blur(&mut blurred, radius);
    for (pixel, soft) in pixels.pixels_mut().zip(blurred.pixels()) {
        for channel in 0..3 {
            let value = f64::from(pixel[channel]);
            let soft = f64::from(soft[channel]);
            let out = if amount > 0.0 {
                value + (value - soft) * amount * 2.0
            } else {
                value + (soft - value) * -amount
            };
            pixel[channel] = out.clamp(0.0, 255.0).round() as u8;
        }
    }
}

/// アート効果. Each is drawn in its spirit: a blur, a pencil sketch, line
/// drawing, a poster's flat colours, paint strokes, glow edges, cement and
/// film grain, a mosaic, …; an unknown one leaves the picture.
pub fn artistic(pixels: &mut image::RgbaImage, effect: &str) {
    let (width, height) = (pixels.width(), pixels.height());
    if width == 0 || height == 0 {
        return;
    }
    let scale = (width.max(height) / 100).clamp(2, 12);
    let edges = |source: &image::RgbaImage| {
        let level = |x: u32, y: u32| {
            luminance(&source.get_pixel(x.min(width - 1), y.min(height - 1)).0) * 255.0
        };
        let mut out = vec![0.0f64; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let dx = level(x + 1, y) - level(x.saturating_sub(1), y);
                let dy = level(x, y + 1) - level(x, y.saturating_sub(1));
                out[(y * width + x) as usize] = (dx * dx + dy * dy).sqrt();
            }
        }
        out
    };
    match effect {
        "artisticBlur" => blur(pixels, scale),
        "artisticPencilGrayscale" | "artisticLineDrawing" | "artisticPencilSketch" => {
            let edge = edges(pixels);
            for (index, pixel) in pixels.pixels_mut().enumerate() {
                let e = edge[index];
                let value = if effect == "artisticLineDrawing" {
                    if e > 48.0 {
                        40.0
                    } else {
                        255.0
                    }
                } else {
                    255.0 - (e * 1.6).min(215.0)
                };
                pixel[0] = value as u8;
                pixel[1] = value as u8;
                pixel[2] = value as u8;
            }
        }
        "artisticGlowEdges" => {
            let edge = edges(pixels);
            for (index, pixel) in pixels.pixels_mut().enumerate() {
                let k = (edge[index] / 120.0).min(1.0);
                for channel in 0..3 {
                    pixel[channel] = (f64::from(pixel[channel]) * k).round() as u8;
                }
            }
        }
        "artisticPosterize" | "artisticCutout" => {
            let levels = if effect == "artisticCutout" { 3.0 } else { 5.0 };
            for pixel in pixels.pixels_mut() {
                for channel in 0..3 {
                    let v = f64::from(pixel[channel]) / 255.0;
                    pixel[channel] = ((v * (levels - 1.0)).round() / (levels - 1.0) * 255.0) as u8;
                }
            }
        }
        "artisticMosaicBubbles" | "artisticPastelsSmooth" | "artisticWatercolorSponge" => {
            // Cells of flat colour.
            let cell = scale * 2;
            for y in (0..height).step_by(cell as usize) {
                for x in (0..width).step_by(cell as usize) {
                    let mut sum = [0u32; 3];
                    let mut count = 0;
                    for yy in y..(y + cell).min(height) {
                        for xx in x..(x + cell).min(width) {
                            let p = pixels.get_pixel(xx, yy);
                            for c in 0..3 {
                                sum[c] += u32::from(p[c]);
                            }
                            count += 1;
                        }
                    }
                    for yy in y..(y + cell).min(height) {
                        for xx in x..(x + cell).min(width) {
                            let p = pixels.get_pixel_mut(xx, yy);
                            for c in 0..3 {
                                p[c] = (sum[c] / count.max(1)) as u8;
                            }
                        }
                    }
                }
            }
        }
        "artisticFilmGrain" | "artisticCement" | "artisticTexturizer" | "artisticPhotocopy" => {
            // A fixed-seed grain over the picture.
            let mut seed: u32 = 0x9E37_79B9;
            let strength = if effect == "artisticFilmGrain" {
                24.0
            } else {
                40.0
            };
            if effect == "artisticPhotocopy" {
                apply(pixels, &[Effect::Grayscale]);
            }
            for pixel in pixels.pixels_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (f64::from(seed % 1000) / 1000.0 - 0.5) * strength;
                for channel in 0..3 {
                    pixel[channel] = (f64::from(pixel[channel]) + noise).clamp(0.0, 255.0) as u8;
                }
            }
        }
        "artisticPaintStrokes"
        | "artisticPaintBrush"
        | "artisticChalkSketch"
        | "artisticPlasticWrap"
        | "artisticMarker"
        | "artisticLightScreen"
        | "artisticCrisscrossEtching"
        | "artisticGlass" => {
            blur(pixels, scale / 2 + 1);
            sharpen(pixels, 0.8);
        }
        _ => {}
    }
}

/// 背景の削除: the colour round the edges, and everything like it that
/// touches them, made transparent; outside `marquee` (the subject's
/// rectangle as fractions: left, top, right, bottom) everything goes.
/// Returns how many pixels went.
pub fn remove_background(pixels: &mut image::RgbaImage, marquee: [f64; 4]) -> usize {
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    if width == 0 || height == 0 {
        return 0;
    }
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    let quantize = |pixel: &image::Rgba<u8>| [pixel[0] >> 3, pixel[1] >> 3, pixel[2] >> 3];
    for x in 0..width {
        for y in [0, height - 1] {
            *counts
                .entry(quantize(pixels.get_pixel(x as u32, y as u32)))
                .or_default() += 1;
        }
    }
    for y in 0..height {
        for x in [0, width - 1] {
            *counts
                .entry(quantize(pixels.get_pixel(x as u32, y as u32)))
                .or_default() += 1;
        }
    }
    let Some((&background, _)) = counts.iter().max_by_key(|(_, count)| **count) else {
        return 0;
    };
    let background = background.map(|c| f64::from(c) * 8.0 + 4.0);
    const TOLERANCE: f64 = 40.0;
    let similar = |pixel: &image::Rgba<u8>| {
        pixel[3] > 0
            && (0..3)
                .all(|channel| (f64::from(pixel[channel]) - background[channel]).abs() <= TOLERANCE)
    };
    let outside = |x: usize, y: usize| {
        let (fx, fy) = (
            (x as f64 + 0.5) / width as f64,
            (y as f64 + 0.5) / height as f64,
        );
        fx < marquee[0] || fx > marquee[2] || fy < marquee[1] || fy > marquee[3]
    };
    let mut removed = 0;
    let mut seen = vec![false; width * height];
    let mut stack = Vec::new();
    for y in 0..height {
        for x in 0..width {
            if outside(x, y) {
                let pixel = pixels.get_pixel_mut(x as u32, y as u32);
                if pixel[3] != 0 {
                    pixel[3] = 0;
                    removed += 1;
                }
            }
        }
    }
    for x in 0..width {
        stack.push((x, 0));
        stack.push((x, height - 1));
    }
    for y in 0..height {
        stack.push((0, y));
        stack.push((width - 1, y));
    }
    while let Some((x, y)) = stack.pop() {
        let index = y * width + x;
        if seen[index] {
            continue;
        }
        seen[index] = true;
        let pixel = *pixels.get_pixel(x as u32, y as u32);
        if !similar(&pixel) && (!outside(x, y) || pixel[3] != 0) {
            continue;
        }
        if pixel[3] != 0 {
            pixels.get_pixel_mut(x as u32, y as u32)[3] = 0;
            removed += 1;
        }
        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < width {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < height {
            stack.push((x, y + 1));
        }
    }
    removed
}

/// 図の圧縮: the picture at no more than `max_width` × `max_height` px
/// (its shown size at the chosen resolution), with the cropped-away part
/// gone when `crop` (left, top, right, bottom fractions) is given.
/// Returns new bytes and their MIME type, or `None` when that would not
/// make it smaller.
pub fn compress(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
    crop: Option<[f64; 4]>,
) -> Option<(Vec<u8>, &'static str)> {
    let mut pixels = decode(bytes)?;
    if let Some([l, t, r, b]) = crop {
        let (w, h) = (f64::from(pixels.width()), f64::from(pixels.height()));
        let x0 = (l.max(0.0) * w).round() as u32;
        let y0 = (t.max(0.0) * h).round() as u32;
        let x1 = ((1.0 - r.max(0.0)) * w).round() as u32;
        let y1 = ((1.0 - b.max(0.0)) * h).round() as u32;
        if x1 > x0 && y1 > y0 {
            pixels = image::imageops::crop_imm(&pixels, x0, y0, x1 - x0, y1 - y0).to_image();
        }
    }
    let (width, height) = (pixels.width(), pixels.height());
    let k = (f64::from(max_width.max(1)) / f64::from(width))
        .min(f64::from(max_height.max(1)) / f64::from(height))
        .min(1.0);
    if k < 1.0 {
        let (nw, nh) = (
            ((f64::from(width) * k).round() as u32).max(1),
            ((f64::from(height) * k).round() as u32).max(1),
        );
        pixels = image::imageops::resize(&pixels, nw, nh, image::imageops::FilterType::Lanczos3);
    }
    let opaque = pixels.pixels().all(|p| p[3] == 255);
    let (out, mime) = if opaque {
        let rgb = image::DynamicImage::ImageRgba8(pixels).to_rgb8();
        let mut out = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .ok()?;
        (out.into_inner(), "image/jpeg")
    } else {
        (encode_png(&pixels)?, "image/png")
    };
    (out.len() < bytes.len()).then_some((out, mime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key;

    fn png(pixels: &image::RgbaImage) -> Vec<u8> {
        encode_png(pixels).unwrap()
    }

    #[test]
    fn a_key_draws_the_picture_with_its_effects_once() {
        let source = png(&image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([255, 0, 0, 255]),
        ));
        let lookup = |part: &str| (part == "a.png").then_some(source.as_slice());
        let plain = image_bytes("a.png", lookup).unwrap();
        assert!(matches!(plain, Bytes::Part(_)));
        let grey = image_bytes(&key("a.png", &[Effect::Grayscale]), lookup).unwrap();
        let pixels = decode(&grey).unwrap();
        let p = pixels.get_pixel(0, 0).0;
        assert!(p[0] == p[1] && p[1] == p[2]);
    }

    #[test]
    fn cached_effects_follow_source_bytes_when_documents_share_a_part_name() {
        let first = png(&image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 255, 255, 255]),
        ));
        let second = png(&image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([0, 0, 0, 255]),
        ));
        let effect_key = key("ppt/media/image1.png", &[Effect::Grayscale]);
        let first_draw = image_bytes(&effect_key, |_| Some(first.as_slice())).unwrap();
        let second_draw = image_bytes(&effect_key, |_| Some(second.as_slice())).unwrap();
        assert_eq!(decode(&first_draw).unwrap().get_pixel(0, 0).0, [255; 4]);
        assert_eq!(
            decode(&second_draw).unwrap().get_pixel(0, 0).0,
            [0, 0, 0, 255]
        );

        // The held image must remain valid when the same key is replaced.
        assert_eq!(decode(&first_draw).unwrap().get_pixel(0, 0).0, [255; 4]);
        let repeated = image_bytes(&effect_key, |_| Some(second.as_slice())).unwrap();
        match (second_draw, repeated) {
            (Bytes::Made(first), Bytes::Made(second)) => assert!(Rc::ptr_eq(&first, &second)),
            _ => panic!("unchanged source bytes should reuse the rendered image"),
        }
    }

    #[test]
    fn cached_decode_failure_does_not_hide_a_replacement_image() {
        let effect_key = key("ppt/media/replaced.png", &[Effect::Grayscale]);
        assert!(image_bytes(&effect_key, |_| Some(b"invalid image")).is_none());
        let source = png(&image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 255, 255, 255]),
        ));
        let drawn = image_bytes(&effect_key, |_| Some(source.as_slice()))
            .expect("a valid replacement must be decoded despite a cached failure");
        assert_eq!(decode(&drawn).unwrap().get_pixel(0, 0).0, [255; 4]);
    }

    #[test]
    fn corrections_colour_and_transparency() {
        let mut pixels = image::RgbaImage::from_pixel(2, 1, image::Rgba([128, 128, 128, 255]));
        apply(
            &mut pixels,
            &[Effect::Luminance {
                brightness: 0.2,
                contrast: 0.0,
            }],
        );
        assert_eq!(pixels.get_pixel(0, 0).0[0], 179);
        let mut pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        apply(
            &mut pixels,
            &[Effect::ColorChange {
                from: [255, 255, 255],
                to: [255, 255, 255],
                alpha: 0,
            }],
        );
        assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
        let mut pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
        saturate(&mut pixels, 0.0);
        let [r, g, b, _] = pixels.get_pixel(0, 0).0;
        assert!(r == g && g == b);
    }

    #[test]
    fn every_artistic_effect_changes_the_picture() {
        let source = image::RgbaImage::from_fn(40, 40, |x, y| {
            image::Rgba([(x * 6) as u8, (y * 6) as u8, ((x + y) * 3) as u8, 255])
        });
        for effect in [
            "artisticBlur",
            "artisticPencilGrayscale",
            "artisticLineDrawing",
            "artisticGlowEdges",
            "artisticPosterize",
            "artisticMosaicBubbles",
            "artisticFilmGrain",
            "artisticPaintStrokes",
        ] {
            let mut pixels = source.clone();
            artistic(&mut pixels, effect);
            assert_ne!(pixels, source, "{effect}");
        }
    }

    #[test]
    fn svgs_are_drawn_and_gifs_give_their_frames() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#00ff00"/></svg>"##;
        let pixels = rasterize_svg(svg, 100).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (50, 100));
        assert_eq!(pixels.get_pixel(25, 50).0, [0, 255, 0, 255]);
        assert_eq!(svg_size(svg), Some((10.0, 20.0)));

        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for color in [[255u8, 0, 0, 255], [0, 0, 255, 255]] {
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(2, 2, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(200, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        let frames = gif_frames(&gif).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].1, 200);
        let lookup = |_: &str| Some(gif.as_slice());
        let second = image_bytes(&key("a.gif", &[Effect::Frame(1)]), lookup).unwrap();
        assert_eq!(decode(&second).unwrap().get_pixel(0, 0).0[2], 255);
    }

    #[test]
    fn compression_downsamples_and_drops_the_cropped_part() {
        let pixels = image::RgbaImage::from_fn(400, 200, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
        });
        let bytes = png(&pixels);
        let (out, mime) = compress(&bytes, 100, 100, Some([0.0, 0.0, 0.5, 0.0])).unwrap();
        assert_eq!(mime, "image/jpeg");
        let read = decode(&out).unwrap();
        assert_eq!((read.width(), read.height()), (100, 100));
    }
}
