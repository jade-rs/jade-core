//! Colour resolution: scheme names through the theme, then the DrawingML
//! colour transforms (`lumMod`, `lumOff`, `shade`, `tint`, `alpha`) that
//! PowerPoint's "lighter 40%" / "darker 25%" swatches are written as.

use jade_pptx_reader::{Color, ThemeColors};

/// Resolves a colour to CSS against the theme. Unknown scheme names and
/// unparsable hex fall back to black rather than dropping the shape.
pub fn resolve_color(color: &Color, theme: &ThemeColors) -> String {
    let (r, g, b, a) = resolve_rgba(color, theme);
    if (a - 1.0).abs() < 1e-6 {
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    } else {
        // Trim the alpha the way `0.5` reads best: no trailing zeros.
        let alpha = format!("{:.3}", a);
        let alpha = alpha.trim_end_matches('0').trim_end_matches('.');
        format!("rgba({r},{g},{b},{alpha})")
    }
}

fn resolve_rgba(color: &Color, theme: &ThemeColors) -> (u8, u8, u8, f64) {
    match color {
        Color::Srgb(hex) => {
            let (r, g, b) = parse_hex(hex).unwrap_or((0, 0, 0));
            (r, g, b, 1.0)
        }
        Color::Scheme(name) => {
            let (r, g, b) = theme.get(name).and_then(parse_hex).unwrap_or((0, 0, 0));
            (r, g, b, 1.0)
        }
        Color::Adjusted {
            base,
            alpha,
            lum_mod,
            lum_off,
            shade,
            tint,
        } => {
            let (r, g, b, base_alpha) = resolve_rgba(base, theme);
            let mut rgb = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
            if lum_mod.is_some() || lum_off.is_some() {
                let (h, s, l) = rgb_to_hsl(rgb);
                let l = (l * lum_mod.unwrap_or(1.0) as f64 + lum_off.unwrap_or(0.0) as f64)
                    .clamp(0.0, 1.0);
                rgb = hsl_to_rgb((h, s, l));
            }
            if let Some(shade) = shade {
                let shade = (*shade as f64).clamp(0.0, 1.0);
                rgb = (rgb.0 * shade, rgb.1 * shade, rgb.2 * shade);
            }
            if let Some(tint) = tint {
                let tint = (*tint as f64).clamp(0.0, 1.0);
                let lift = |channel: f64| channel * tint + (1.0 - tint);
                rgb = (lift(rgb.0), lift(rgb.1), lift(rgb.2));
            }
            let to_byte = |channel: f64| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
            (
                to_byte(rgb.0),
                to_byte(rgb.1),
                to_byte(rgb.2),
                base_alpha * alpha.map(|a| (a as f64).clamp(0.0, 1.0)).unwrap_or(1.0),
            )
        }
        Color::Transformed { base, ops } => {
            let (r, g, b, mut alpha) = resolve_rgba(base, theme);
            let mut rgb = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
            for (name, value) in ops {
                let v = *value as f64;
                let hsl = |rgb, f: &dyn Fn((f64, f64, f64)) -> (f64, f64, f64)| {
                    hsl_to_rgb(f(rgb_to_hsl(rgb)))
                };
                rgb = match name.as_str() {
                    "alpha" => {
                        alpha = v;
                        rgb
                    }
                    "alphaMod" => {
                        alpha *= v;
                        rgb
                    }
                    "alphaOff" => {
                        alpha += v;
                        rgb
                    }
                    "lum" => hsl(rgb, &|(h, s, _)| (h, s, v.clamp(0.0, 1.0))),
                    "lumMod" => hsl(rgb, &|(h, s, l)| (h, s, (l * v).clamp(0.0, 1.0))),
                    "lumOff" => hsl(rgb, &|(h, s, l)| (h, s, (l + v).clamp(0.0, 1.0))),
                    "sat" => hsl(rgb, &|(h, _, l)| (h, v.clamp(0.0, 1.0), l)),
                    "satMod" => hsl(rgb, &|(h, s, l)| (h, (s * v).clamp(0.0, 1.0), l)),
                    "satOff" => hsl(rgb, &|(h, s, l)| (h, (s + v).clamp(0.0, 1.0), l)),
                    "hue" => hsl(rgb, &|(_, s, l)| (v.rem_euclid(360.0), s, l)),
                    "hueMod" => hsl(rgb, &|(h, s, l)| ((h * v).rem_euclid(360.0), s, l)),
                    "hueOff" => hsl(rgb, &|(h, s, l)| ((h + v).rem_euclid(360.0), s, l)),
                    "comp" => hsl(rgb, &|(h, s, l)| ((h + 180.0).rem_euclid(360.0), s, l)),
                    "inv" => (1.0 - rgb.0, 1.0 - rgb.1, 1.0 - rgb.2),
                    "gray" => {
                        let y = 0.2126 * rgb.0 + 0.7152 * rgb.1 + 0.0722 * rgb.2;
                        (y, y, y)
                    }
                    "shade" => (rgb.0 * v, rgb.1 * v, rgb.2 * v),
                    "tint" => {
                        let lift = |c: f64| c * v + (1.0 - v);
                        (lift(rgb.0), lift(rgb.1), lift(rgb.2))
                    }
                    "red" => (v, rgb.1, rgb.2),
                    "redMod" => (rgb.0 * v, rgb.1, rgb.2),
                    "redOff" => (rgb.0 + v, rgb.1, rgb.2),
                    "green" => (rgb.0, v, rgb.2),
                    "greenMod" => (rgb.0, rgb.1 * v, rgb.2),
                    "greenOff" => (rgb.0, rgb.1 + v, rgb.2),
                    "blue" => (rgb.0, rgb.1, v),
                    "blueMod" => (rgb.0, rgb.1, rgb.2 * v),
                    "blueOff" => (rgb.0, rgb.1, rgb.2 + v),
                    _ => rgb,
                };
                let clamp = |c: f64| c.clamp(0.0, 1.0);
                rgb = (clamp(rgb.0), clamp(rgb.1), clamp(rgb.2));
            }
            let to_byte = |channel: f64| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
            (
                to_byte(rgb.0),
                to_byte(rgb.1),
                to_byte(rgb.2),
                alpha.clamp(0.0, 1.0),
            )
        }
    }
}

/// Linear blend of two resolved CSS colours (`#RRGGBB` or `rgba(...)`),
/// `t` toward `b`. An unparsable colour yields the nearer one unchanged.
pub(crate) fn mix(a: &str, b: &str, t: f64) -> String {
    match (parse_css(a), parse_css(b)) {
        (Some(a), Some(b)) => {
            let mix = |x: f64, y: f64| x + (y - x) * t;
            let r = mix(a.0, b.0).round() as u8;
            let g = mix(a.1, b.1).round() as u8;
            let bl = mix(a.2, b.2).round() as u8;
            let alpha = mix(a.3, b.3);
            if (alpha - 1.0).abs() < 1e-6 {
                format!("#{r:02X}{g:02X}{bl:02X}")
            } else {
                format!("rgba({r},{g},{bl},{alpha:.3})")
            }
        }
        _ => {
            if t < 0.5 {
                a.to_string()
            } else {
                b.to_string()
            }
        }
    }
}

fn parse_css(color: &str) -> Option<(f64, f64, f64, f64)> {
    if let Some(hex) = color.strip_prefix('#') {
        let (r, g, b) = parse_hex(hex)?;
        return Some((f64::from(r), f64::from(g), f64::from(b), 1.0));
    }
    let inner = color.strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|part| part.trim().parse::<f64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next()??,
        parts.next()??,
    ))
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// Hue in degrees, saturation and lightness in 0..=1.
fn rgb_to_hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta.abs() < 1e-12 {
        return (0.0, 0.0, l);
    }
    let s = delta / (1.0 - (2.0 * l - 1.0).abs());
    let h = if (max - r).abs() < 1e-12 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-12 {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, s, l)
}

fn hsl_to_rgb((h, s, l): (f64, f64, f64)) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transformed(hex: &str, ops: &[(&str, f32)]) -> String {
        let color = Color::Transformed {
            base: Box::new(Color::Srgb(hex.to_string())),
            ops: ops.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
        };
        resolve_color(&color, &ThemeColors::default())
    }

    #[test]
    fn transforms_apply_in_document_order() {
        assert_eq!(transformed("FF0000", &[("comp", 0.0)]), "#00FFFF");
        assert_eq!(transformed("FF0000", &[("inv", 0.0)]), "#00FFFF");
        assert_eq!(transformed("FF0000", &[("satMod", 0.0)]), "#808080");
        assert_eq!(transformed("FF0000", &[("hueOff", 120.0)]), "#00FF00");
        assert_eq!(transformed("FFFFFF", &[("gray", 0.0)]), "#FFFFFF");
        assert_eq!(
            transformed("000000", &[("alpha", 0.5), ("alphaMod", 0.5)]),
            "rgba(0,0,0,0.25)"
        );
    }
}
