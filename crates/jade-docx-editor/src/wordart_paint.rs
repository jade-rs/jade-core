//! WordArt's text drawn as Word draws it: its fill, outline, shadow and
//! glow (`w14:` effects), and its text transform (`a:prstTxWarp`) — each
//! character set along the transform's path or stretched between its
//! envelope's edges.

use jade_painter::Painter;

/// How WordArt's text is painted.
pub struct WordArtPaint {
    /// The run's font, as a canvas descriptor (with its size).
    pub font: String,
    /// The font's size in px, which the descriptor carries.
    pub size_px: f64,
    pub fill: Option<String>,
    pub outline: Option<(String, f64)>,
    pub shadow: bool,
    pub glow: Option<(String, f64)>,
}

/// The envelope a warp stretches each character between: its top and
/// bottom, as fractions of the box's height, at `u` across it (0–1).
fn envelope(preset: &str, u: f64) -> Option<(f64, f64)> {
    use std::f64::consts::PI;
    let mid = (2.0 * u - 1.0).abs();
    Some(match preset {
        "textSlantUp" => (0.4 * (1.0 - u), 0.4 * (1.0 - u) + 0.6),
        "textSlantDown" => (0.4 * u, 0.4 * u + 0.6),
        "textWave1" => (0.15 - 0.15 * (2.0 * PI * u).sin(), 0.85 - 0.15 * (2.0 * PI * u).sin()),
        "textWave2" | "textWave4" => {
            (0.15 + 0.15 * (2.0 * PI * u).sin(), 0.85 + 0.15 * (2.0 * PI * u).sin())
        }
        "textDoubleWave1" => (0.15 - 0.15 * (4.0 * PI * u).sin(), 0.85 - 0.15 * (4.0 * PI * u).sin()),
        "textTriangle" => (0.6 * mid, 1.0),
        "textTriangleInverted" => (0.0, 1.0 - 0.6 * mid),
        "textChevron" => (0.3 * mid, 0.7 + 0.3 * mid),
        "textChevronInverted" => (0.3 * (1.0 - mid), 1.0 - 0.3 * mid),
        "textInflate" => (0.2 * (1.0 - (PI * u).sin()), 1.0 - 0.2 * (1.0 - (PI * u).sin())),
        "textDeflate" => (0.2 * (PI * u).sin(), 1.0 - 0.2 * (PI * u).sin()),
        "textFadeRight" => (0.3 * u, 1.0 - 0.3 * u),
        "textFadeLeft" => (0.3 * (1.0 - u), 1.0 - 0.3 * (1.0 - u)),
        "textCanUp" => (0.25 * (1.0 - (PI * u).sin()), 1.0 - 0.25 * (PI * u).sin()),
        "textCanDown" => (0.25 * (PI * u).sin(), 1.0 - 0.25 * (1.0 - (PI * u).sin())),
        "textPlain" | "textStop" | "textDeflateBottom" | "textInflateBottom" => (0.0, 1.0),
        _ => return None,
    })
}

/// Where a character's baseline centre goes on a path warp, and its angle:
/// `u` across the text (0–1), in a `width` × `height` box at the origin.
fn path_point(preset: &str, u: f64, width: f64, height: f64) -> Option<((f64, f64), f64)> {
    use std::f64::consts::PI;
    let (rx, ry) = (width / 2.0, height * 0.9);
    let (cx, cy) = (width / 2.0, height);
    let point = |theta: f64, center: (f64, f64), radii: (f64, f64)| {
        (center.0 + radii.0 * theta.cos(), center.1 - radii.1 * theta.sin())
    };
    let (theta, center, radii, clockwise) = match preset {
        "textArchUp" | "textButton" | "textArchUpPour" => (PI * (1.0 - u), (cx, cy), (rx, ry), true),
        "textArchDown" | "textArchDownPour" => (PI * (1.0 + u), (cx, 0.0), (rx, ry), false),
        "textCircle" | "textCirclePour" => (
            PI - 2.0 * PI * u,
            (width / 2.0, height / 2.0),
            (width / 2.0, height / 2.0),
            true,
        ),
        _ => return None,
    };
    let at = point(theta, center, radii);
    // The tangent, in the reading direction.
    let step = if clockwise { -0.001 } else { 0.001 };
    let ahead = point(theta + step, center, radii);
    let angle = (ahead.1 - at.1).atan2(ahead.0 - at.0);
    Some((at, angle))
}

fn draw_glyph<P: Painter>(painter: &P, ch: &str, paint: &WordArtPaint) {
    let _ = paint.glow.as_ref().map(|(color, radius)| {
        painter.save();
        painter.set_shadow_color(color);
        painter.set_shadow_blur(*radius * 2.0);
        painter.set_fill_style_str(color);
        let _ = painter.fill_text(ch, 0.0, 0.0);
        painter.restore();
    });
    if paint.shadow {
        painter.save();
        painter.set_fill_style_str("rgba(0, 0, 0, 0.35)");
        let _ = painter.fill_text(ch, paint.size_px * 0.04, paint.size_px * 0.04);
        painter.restore();
    }
    if let Some(fill) = paint.fill.as_deref() {
        painter.set_fill_style_str(fill);
        let _ = painter.fill_text(ch, 0.0, 0.0);
    }
    if let Some((color, width)) = paint.outline.as_ref() {
        painter.set_stroke_style_str(color);
        painter.set_line_width(*width);
        let _ = painter.stroke_text(ch, 0.0, 0.0);
    }
}

/// Draws `text` in the box (`x`, `y`, `width`, `height`) with `preset`'s
/// transform; `textNoShape` (no transform) sets it centred at its own size.
pub fn paint_wordart<P: Painter>(
    painter: &P,
    text: &str,
    preset: &str,
    (x, y, width, height): (f64, f64, f64, f64),
    paint: &WordArtPaint,
) {
    let text = text.lines().next().unwrap_or("").trim_end();
    if text.is_empty() || width <= 0.0 || height <= 0.0 {
        return;
    }
    painter.save();
    painter.set_font(&paint.font);
    painter.set_text_baseline("alphabetic");
    let characters: Vec<String> = unicode_segmentation::UnicodeSegmentation::graphemes(text, true)
        .map(str::to_string)
        .collect();
    let advances: Vec<f64> = characters
        .iter()
        .map(|ch| painter.measure_text_width(ch).unwrap_or(paint.size_px * 0.6).max(0.1))
        .collect();
    let total: f64 = advances.iter().sum::<f64>().max(0.1);
    // The glyphs' box: ascent about 0.88 of the size, descent 0.24.
    let (ascent, line) = (paint.size_px * 0.88, paint.size_px * 1.12);
    if let Some(_) = path_point(preset, 0.5, width, height) {
        // Along a path: the text stretched over the path's length.
        let samples = 64;
        let mut length = 0.0;
        let mut previous = path_point(preset, 0.0, width, height).map(|(at, _)| at).unwrap_or_default();
        for step in 1..=samples {
            let at = path_point(preset, step as f64 / samples as f64, width, height)
                .map(|(at, _)| at)
                .unwrap_or_default();
            length += ((at.0 - previous.0).powi(2) + (at.1 - previous.1).powi(2)).sqrt();
            previous = at;
        }
        // Word spreads the text over the whole path, the glyphs at their
        // size (smaller only when the box is too low for them).
        let scale = (height * 0.5 / line).min(length * 0.95 / total).min(1.0).max(0.05);
        let mut cursor = 0.0;
        for (ch, advance) in characters.iter().zip(&advances) {
            let u = (cursor + advance / 2.0) / total;
            cursor += advance;
            let Some(((px, py), angle)) = path_point(preset, u.clamp(0.0, 1.0), width, height) else {
                continue;
            };
            painter.save();
            let _ = painter.translate(x + px, y + py);
            let _ = painter.rotate(angle);
            let _ = painter.scale(scale, scale);
            let _ = painter.translate(-advance / 2.0, 0.0);
            draw_glyph(painter, ch, paint);
            painter.restore();
        }
    } else if envelope(preset, 0.5).is_some() {
        // Between an envelope: each glyph stretched to fill it.
        let scale_x = width / total;
        let mut cursor = 0.0;
        for (ch, advance) in characters.iter().zip(&advances) {
            let u = (cursor + advance / 2.0) / total;
            let (top, bottom) = envelope(preset, u).unwrap_or((0.0, 1.0));
            let (top, bottom) = (y + top * height, y + bottom * height);
            let scale_y = (bottom - top) / line;
            painter.save();
            let _ = painter.translate(x + cursor * scale_x, top);
            let _ = painter.scale(scale_x, scale_y);
            let _ = painter.translate(0.0, ascent);
            draw_glyph(painter, ch, paint);
            painter.restore();
            cursor += advance;
        }
    } else {
        // No transform: at its own size, centred.
        let left = x + (width - total) / 2.0;
        let baseline = y + (height - line) / 2.0 + ascent;
        let mut cursor = 0.0;
        for (ch, advance) in characters.iter().zip(&advances) {
            painter.save();
            let _ = painter.translate(left + cursor, baseline);
            draw_glyph(painter, ch, paint);
            painter.restore();
            cursor += advance;
        }
    }
    painter.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arches_and_envelopes_stay_in_their_box() {
        for preset in ["textArchUp", "textArchDown", "textCircle"] {
            for step in 0..=10 {
                let ((px, py), _) = path_point(preset, step as f64 / 10.0, 200.0, 100.0).unwrap();
                assert!((-0.5..=200.5).contains(&px) && (-0.5..=100.5).contains(&py), "{preset} {px} {py}");
            }
        }
        // An arch's ends are low and its middle high.
        let left = path_point("textArchUp", 0.0, 200.0, 100.0).unwrap().0;
        let middle = path_point("textArchUp", 0.5, 200.0, 100.0).unwrap().0;
        assert!(middle.1 < left.1);
        for preset in ["textSlantUp", "textWave1", "textTriangle", "textInflate", "textFadeRight"] {
            for step in 0..=10 {
                let (top, bottom) = envelope(preset, step as f64 / 10.0).unwrap();
                assert!(top >= -0.01 && bottom <= 1.31 && bottom > top, "{preset}");
            }
        }
        assert!(envelope("textNoShape", 0.5).is_none());
    }
}
