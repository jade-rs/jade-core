//! The picture of ink kept beside it (`mc:Fallback`) for consumers that do
//! not read InkML: a transparent PNG of the strokes.

use crate::inkml::{InkTool, InkTrace};

/// A transparent PNG of `traces` (box px), the fallback picture Word and
/// PowerPoint keep beside the ink for consumers that do not read it.
pub fn rasterize_ink(traces: &[InkTrace], width: f64, height: f64) -> Vec<u8> {
    let (w, h) = (
        width.ceil().max(1.0) as usize,
        height.ceil().max(1.0) as usize,
    );
    let (w, h) = (w.min(4096), h.min(4096));
    let mut pixels = vec![0f32; w * h * 4];
    for trace in traces {
        let color = parse_color(&trace.brush.color);
        let opacity = match trace.brush.tool {
            InkTool::Highlighter => 0.5,
            InkTool::Pencil => 0.8,
            InkTool::Pen => 1.0,
        } * (1.0 - f32::from(trace.brush.transparency) / 255.0);
        let half = (trace.brush.width_px.max(1.0) / 2.0) as f32;
        let points: Vec<(f32, f32)> = trace
            .points
            .iter()
            .map(|&(x, y)| (x as f32, y as f32))
            .collect();
        let segments: Vec<((f32, f32), (f32, f32))> = if points.len() == 1 {
            vec![(points[0], points[0])]
        } else {
            points.windows(2).map(|pair| (pair[0], pair[1])).collect()
        };
        // Coverage per pixel for the whole stroke, so overlaps do not darken.
        let mut coverage = vec![0f32; w * h];
        for (a, b) in segments {
            let left = (a.0.min(b.0) - half - 1.0).floor().max(0.0) as usize;
            let top = (a.1.min(b.1) - half - 1.0).floor().max(0.0) as usize;
            let right = ((a.0.max(b.0) + half + 1.0).ceil() as usize).min(w);
            let bottom = ((a.1.max(b.1) + half + 1.0).ceil() as usize).min(h);
            for y in top..bottom {
                for x in left..right {
                    let p = (x as f32 + 0.5, y as f32 + 0.5);
                    let distance = segment_distance(p, a, b);
                    let value = (half - distance + 0.5).clamp(0.0, 1.0);
                    let slot = &mut coverage[y * w + x];
                    *slot = slot.max(value);
                }
            }
        }
        for (index, value) in coverage.into_iter().enumerate() {
            if value <= 0.0 {
                continue;
            }
            let alpha = value * opacity;
            let pixel = &mut pixels[index * 4..index * 4 + 4];
            let out_alpha = alpha + pixel[3] * (1.0 - alpha);
            for channel in 0..3 {
                let source = color[channel] * alpha;
                let destination = pixel[channel] * pixel[3] * (1.0 - alpha);
                pixel[channel] = if out_alpha > 0.0 {
                    (source + destination) / out_alpha
                } else {
                    0.0
                };
            }
            pixel[3] = out_alpha;
        }
    }
    let bytes: Vec<u8> = pixels
        .chunks(4)
        .flat_map(|pixel| {
            [
                (pixel[0] * 255.0).round() as u8,
                (pixel[1] * 255.0).round() as u8,
                (pixel[2] * 255.0).round() as u8,
                (pixel[3] * 255.0).round() as u8,
            ]
        })
        .collect();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&bytes);
        }
    }
    png
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt()
}

fn parse_color(color: &str) -> [f32; 3] {
    let hex = color.trim_start_matches('#');
    let channel = |index: usize| {
        hex.get(index..index + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .unwrap_or(0) as f32
            / 255.0
    };
    [channel(0), channel(2), channel(4)]
}
