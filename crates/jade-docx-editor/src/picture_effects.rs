//! What DrawingML does to a picture's pixels as it draws them: the colour
//! adjustments under `a:blip` (brightness and contrast, grayscale, black and
//! white, recolouring, transparency, a transparent colour) and the soft
//! edges of `a:softEdge`. Hosts only blit, so the picture is drawn from a
//! copy with these applied, made once per picture and effect set.
//!
//! Also the pixel work of the picture tools: Word's 背景の削除 (a picture's
//! background made transparent from its edges inwards).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::document::{BlipEffect, DrawingColor, RunImage};

/// Pictures larger than this many pixels are drawn as they are rather than
/// processed on every paint.
const MAX_PIXELS: u64 = 24_000_000;

thread_local! {
    static PROCESSED: RefCell<HashMap<u64, Option<String>>> = RefCell::new(HashMap::new());
}

/// The data URI to draw `image` from: its own, or a copy with its colour
/// adjustments and soft edges applied.
pub fn effective_data_uri(image: &RunImage) -> Option<Cow<'_, str>> {
    let data_uri = image.data_uri.as_deref()?;
    let soft_edge = image
        .format()
        .soft_edge_radius_px
        .filter(|radius| *radius > 0.0)
        .map(|radius| {
            let width = image.width_px.unwrap_or(1.0).max(1.0);
            let height = image.height_px.unwrap_or(1.0).max(1.0);
            (radius / width, radius / height)
        });
    if image.format().blip_effects.is_empty() && soft_edge.is_none() {
        return Some(Cow::Borrowed(data_uri));
    }
    let mut hasher = rustc_hash::FxHasher::default();
    data_uri.hash(&mut hasher);
    format!("{:?}{:?}", image.format().blip_effects, soft_edge).hash(&mut hasher);
    let key = hasher.finish();
    let processed = PROCESSED.with(|cache| {
        if let Some(found) = cache.borrow().get(&key) {
            return found.clone();
        }
        let made = process(data_uri, &image.format().blip_effects, soft_edge);
        let mut cache = cache.borrow_mut();
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(key, made.clone());
        made
    });
    Some(match processed {
        Some(uri) => Cow::Owned(uri),
        None => Cow::Borrowed(data_uri),
    })
}

/// The bytes of a base64 data URI.
pub fn data_uri_bytes(data_uri: &str) -> Option<Vec<u8>> {
    let (_, payload) = data_uri.split_once(";base64,")?;
    BASE64.decode(payload.trim()).ok()
}

/// Decodes a PNG or JPEG into RGBA pixels.
pub fn decode(bytes: &[u8]) -> Option<image::RgbaImage> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return None;
    }
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > MAX_PIXELS {
        return None;
    }
    Some(decoded.to_rgba8())
}

pub fn encode_png(pixels: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    pixels
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(bytes)
}

fn process(
    data_uri: &str,
    effects: &[BlipEffect],
    soft_edge: Option<(f64, f64)>,
) -> Option<String> {
    let bytes = data_uri_bytes(data_uri)?;
    let mut pixels = decode(&bytes)?;
    apply_effects(&mut pixels, effects);
    if let Some((x, y)) = soft_edge {
        soften_edges(&mut pixels, x, y);
    }
    let png = encode_png(&pixels)?;
    Some(format!("data:image/png;base64,{}", BASE64.encode(png)))
}

fn rgb(color: &DrawingColor) -> Option<[f64; 3]> {
    let hex = color.rgb.as_deref()?.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok().map(f64::from);
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn luminance(pixel: &[u8]) -> f64 {
    (0.299 * f64::from(pixel[0]) + 0.587 * f64::from(pixel[1]) + 0.114 * f64::from(pixel[2]))
        / 255.0
}

/// Applies the colour adjustments in the order DrawingML lists them.
pub fn apply_effects(pixels: &mut image::RgbaImage, effects: &[BlipEffect]) {
    for effect in effects {
        match effect {
            BlipEffect::Luminance {
                brightness,
                contrast,
            } => {
                // Contrast scales about mid-grey: +100% is a hard step,
                // -100% flat grey; brightness then shifts every channel.
                let factor = if *contrast >= 0.0 {
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
            BlipEffect::Grayscale => {
                for pixel in pixels.pixels_mut() {
                    let value = (luminance(&pixel.0) * 255.0).round() as u8;
                    pixel[0] = value;
                    pixel[1] = value;
                    pixel[2] = value;
                }
            }
            BlipEffect::BiLevel { threshold } => {
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
            BlipEffect::Duotone { dark, light } => {
                let (Some(dark), Some(light)) = (rgb(dark), rgb(light)) else {
                    continue;
                };
                for pixel in pixels.pixels_mut() {
                    let level = luminance(&pixel.0);
                    for channel in 0..3 {
                        let value = dark[channel] + (light[channel] - dark[channel]) * level;
                        pixel[channel] = value.clamp(0.0, 255.0).round() as u8;
                    }
                }
            }
            BlipEffect::AlphaModFix { amount } => {
                for pixel in pixels.pixels_mut() {
                    let alpha = f64::from(pixel[3]) * amount.clamp(0.0, 1.0);
                    pixel[3] = alpha.round() as u8;
                }
            }
            BlipEffect::ColorChange { from, to } => {
                let (Some(from), Some(target)) = (rgb(from), rgb(to)) else {
                    continue;
                };
                let alpha = (to.alpha.unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                // Word matches the picked colour with a small tolerance, so
                // a JPEG's noisy flat colour still goes.
                const TOLERANCE: f64 = 12.0;
                for pixel in pixels.pixels_mut() {
                    let near = (0..3).all(|channel| {
                        (f64::from(pixel[channel]) - from[channel]).abs() <= TOLERANCE
                    });
                    if near {
                        pixel[0] = target[0] as u8;
                        pixel[1] = target[1] as u8;
                        pixel[2] = target[2] as u8;
                        pixel[3] = alpha;
                    }
                }
            }
        }
    }
}

/// Fades the picture to transparent over the last `x` (of its width) and
/// `y` (of its height) at each edge, as `a:softEdge` does.
pub fn soften_edges(pixels: &mut image::RgbaImage, x: f64, y: f64) {
    let (width, height) = (pixels.width() as f64, pixels.height() as f64);
    let reach_x = (x * width).max(1.0);
    let reach_y = (y * height).max(1.0);
    for (px, py, pixel) in pixels.enumerate_pixels_mut() {
        let (px, py) = (px as f64 + 0.5, py as f64 + 0.5);
        let horizontal = (px.min(width - px) / reach_x).min(1.0);
        let vertical = (py.min(height - py) / reach_y).min(1.0);
        let fade = horizontal.min(vertical);
        pixel[3] = (f64::from(pixel[3]) * fade).round() as u8;
    }
}

/// Word's 背景の削除 in its simplest form: the colour around the picture's
/// edges, and everything like it that touches the edges, made transparent.
/// `marquee` is the rectangle (left, top, right, bottom, as fractions from
/// the left and top) the subject is in, as `a14:backgroundRemoval` stores
/// it: outside it everything goes. Returns how many pixels went.
pub fn remove_background(pixels: &mut image::RgbaImage, marquee: [f64; 4]) -> usize {
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    if width == 0 || height == 0 {
        return 0;
    }
    // The background is the commonest colour along the border.
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
    let background = [
        f64::from(background[0]) * 8.0 + 4.0,
        f64::from(background[1]) * 8.0 + 4.0,
        f64::from(background[2]) * 8.0 + 4.0,
    ];
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
    let mut seen = vec![false; width * height];
    let mut stack = Vec::new();
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

/// Word's 色の彩度: 0.0 grey, 1.0 unchanged, up to 4.0.
pub fn saturate(pixels: &mut image::RgbaImage, amount: f64) {
    for pixel in pixels.pixels_mut() {
        let grey = luminance(&pixel.0) * 255.0;
        for channel in 0..3 {
            let value = grey + (f64::from(pixel[channel]) - grey) * amount;
            pixel[channel] = value.clamp(0.0, 255.0).round() as u8;
        }
    }
}

/// Word's 色のトーン: warmer above 6500 K, cooler below.
pub fn warm(pixels: &mut image::RgbaImage, kelvin: f64) {
    let shift = ((kelvin - 6500.0) / 5000.0).clamp(-1.0, 1.0) * 40.0;
    for pixel in pixels.pixels_mut() {
        pixel[0] = (f64::from(pixel[0]) + shift).clamp(0.0, 255.0).round() as u8;
        pixel[2] = (f64::from(pixel[2]) - shift).clamp(0.0, 255.0).round() as u8;
    }
}

/// Brightness and contrast as Word's 修整 applies them (-1.0–1.0 each).
pub fn brighten(pixels: &mut image::RgbaImage, brightness: f64, contrast: f64) {
    apply_effects(
        pixels,
        &[BlipEffect::Luminance {
            brightness,
            contrast,
        }],
    );
}

/// A box blur of `radius` pixels, run twice (close to a Gaussian).
pub fn blur(pixels: &mut image::RgbaImage, radius: u32) {
    if radius == 0 {
        return;
    }
    for _ in 0..2 {
        for horizontal in [true, false] {
            let source = pixels.clone();
            let (width, height) = (source.width() as i64, source.height() as i64);
            for y in 0..height {
                for x in 0..width {
                    let mut sum = [0u32; 4];
                    let mut count = 0;
                    for offset in -(radius as i64)..=(radius as i64) {
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

/// Word's アート効果, the ones jade draws: `artisticBlur`,
/// `artisticPencilGrayscale` (a pencil sketch) and `artisticLineDrawing`.
pub fn artistic(pixels: &mut image::RgbaImage, effect: &str) {
    match effect {
        "artisticBlur" => {
            let radius = (pixels.width().max(pixels.height()) / 100).clamp(2, 12);
            blur(pixels, radius);
        }
        "artisticPencilGrayscale" | "artisticLineDrawing" => {
            // Edges, as dark strokes on white: the luminance gradient,
            // inverted. A line drawing keeps only the strong ones.
            let source = pixels.clone();
            let (width, height) = (source.width(), source.height());
            let level = |x: u32, y: u32| {
                luminance(&source.get_pixel(x.min(width - 1), y.min(height - 1)).0) * 255.0
            };
            for y in 0..height {
                for x in 0..width {
                    let dx = level(x + 1, y) - level(x.saturating_sub(1), y);
                    let dy = level(x, y + 1) - level(x, y.saturating_sub(1));
                    let edge = (dx * dx + dy * dy).sqrt();
                    let value = if effect == "artisticLineDrawing" {
                        if edge > 48.0 {
                            40.0
                        } else {
                            255.0
                        }
                    } else {
                        255.0 - (edge * 1.6).min(215.0)
                    };
                    let pixel = pixels.get_pixel_mut(x, y);
                    pixel[0] = value as u8;
                    pixel[1] = value as u8;
                    pixel[2] = value as u8;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(pixels: &[[u8; 4]], width: u32) -> image::RgbaImage {
        let height = pixels.len() as u32 / width;
        image::RgbaImage::from_raw(width, height, pixels.concat()).unwrap()
    }

    #[test]
    fn grayscale_and_black_and_white() {
        let mut pixels = picture(&[[255, 0, 0, 255], [250, 250, 250, 255]], 2);
        apply_effects(&mut pixels, &[BlipEffect::Grayscale]);
        assert_eq!(pixels.get_pixel(0, 0).0, [76, 76, 76, 255]);
        apply_effects(&mut pixels, &[BlipEffect::BiLevel { threshold: 0.5 }]);
        assert_eq!(pixels.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn brightness_and_contrast() {
        let mut pixels = picture(&[[128, 128, 128, 255], [200, 100, 50, 255]], 2);
        apply_effects(
            &mut pixels,
            &[BlipEffect::Luminance {
                brightness: 0.2,
                contrast: 0.0,
            }],
        );
        assert_eq!(pixels.get_pixel(0, 0).0[0], 179);
        let mut pixels = picture(&[[100, 100, 100, 255]], 1);
        apply_effects(
            &mut pixels,
            &[BlipEffect::Luminance {
                brightness: 0.0,
                contrast: 0.5,
            }],
        );
        assert!(
            pixels.get_pixel(0, 0).0[0] < 100,
            "more contrast darkens below grey"
        );
    }

    #[test]
    fn duotone_maps_dark_to_light() {
        let mut pixels = picture(&[[0, 0, 0, 255], [255, 255, 255, 255]], 2);
        apply_effects(
            &mut pixels,
            &[BlipEffect::Duotone {
                dark: DrawingColor {
                    rgb: Some("#203864".into()),
                    ..Default::default()
                },
                light: DrawingColor {
                    rgb: Some("#FFFFFF".into()),
                    ..Default::default()
                },
            }],
        );
        assert_eq!(pixels.get_pixel(0, 0).0, [0x20, 0x38, 0x64, 255]);
        assert_eq!(pixels.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn transparent_colour_and_transparency() {
        let mut pixels = picture(&[[255, 255, 255, 255], [10, 20, 30, 255]], 2);
        apply_effects(
            &mut pixels,
            &[
                BlipEffect::ColorChange {
                    from: DrawingColor {
                        rgb: Some("#FFFFFF".into()),
                        ..Default::default()
                    },
                    to: DrawingColor {
                        rgb: Some("#FFFFFF".into()),
                        alpha: Some(0.0),
                        ..Default::default()
                    },
                },
                BlipEffect::AlphaModFix { amount: 0.5 },
            ],
        );
        assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
        assert_eq!(pixels.get_pixel(1, 0).0[3], 128);
    }

    #[test]
    fn background_goes_from_the_edges() {
        // A white frame around a red square, and a white dot inside it.
        let mut raw = Vec::new();
        for y in 0..6 {
            for x in 0..6 {
                let inside = (1..5).contains(&x) && (1..5).contains(&y);
                raw.push(if inside && !(x == 2 && y == 2) {
                    [200, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                });
            }
        }
        let mut pixels = picture(&raw, 6);
        let removed = remove_background(&mut pixels, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(removed, 20, "the frame, not the enclosed dot");
        assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
        assert_eq!(pixels.get_pixel(2, 2).0[3], 255);
        assert_eq!(pixels.get_pixel(1, 1).0[3], 255);
    }

    #[test]
    fn soft_edges_fade_to_the_border() {
        let mut pixels = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 255]));
        soften_edges(&mut pixels, 0.3, 0.3);
        assert!(pixels.get_pixel(0, 5).0[3] < 60);
        assert_eq!(pixels.get_pixel(5, 5).0[3], 255);
    }

    #[test]
    fn saturation_and_artistic_effects() {
        let mut pixels = picture(&[[200, 100, 50, 255]], 1);
        saturate(&mut pixels, 0.0);
        let [r, g, b, _] = pixels.get_pixel(0, 0).0;
        assert!(r == g && g == b);
        let mut pixels = image::RgbaImage::from_fn(8, 8, |x, _| {
            if x < 4 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        artistic(&mut pixels, "artisticLineDrawing");
        assert_eq!(pixels.get_pixel(0, 0).0[0], 255, "flat areas go white");
        assert_eq!(pixels.get_pixel(4, 4).0[0], 40, "the edge is drawn");
    }
}
