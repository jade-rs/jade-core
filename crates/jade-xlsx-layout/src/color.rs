//! Shared color resolution for conditional interpolation and painting.

use jade_xlsx_reader::Color;

/// The legacy indexed palette's two special entries. 64 is "system foreground"
/// and 65 "system background", which Excel writes for automatic colours.
const INDEXED_SYSTEM_FOREGROUND: u32 = 64;
const INDEXED_SYSTEM_BACKGROUND: u32 = 65;

/// Resolves a colour to `#rrggbb`, or `rgba(...)` when it is translucent.
///
/// Theme references use the base colour resolved by the reader, with Excel's
/// luminance tint applied here. The two legibility-critical slots still have a
/// safe fallback for workbooks whose theme part is missing.
pub fn to_css(color: Color, fallback: &str) -> String {
    match color {
        Color::Argb([alpha, red, green, blue]) => {
            if alpha == 0xFF {
                format!("#{red:02x}{green:02x}{blue:02x}")
            } else {
                format!(
                    "rgba({red}, {green}, {blue}, {:.3})",
                    f64::from(alpha) / 255.0
                )
            }
        }
        Color::Theme {
            tint,
            resolved: Some([red, green, blue]),
            ..
        } => {
            let [red, green, blue] = apply_tint([red, green, blue], tint);
            format!("#{red:02x}{green:02x}{blue:02x}")
        }
        Color::Theme { index: 0, .. } => "#ffffff".to_string(),
        Color::Theme { index: 1, .. } => "#000000".to_string(),
        Color::Theme { .. } => fallback.to_string(),
        Color::Indexed(INDEXED_SYSTEM_FOREGROUND) => "#000000".to_string(),
        Color::Indexed(INDEXED_SYSTEM_BACKGROUND) => "#ffffff".to_string(),
        Color::Indexed(index) => {
            match jade_xlsx_reader::styles::INDEXED_PALETTE.get(index as usize) {
                Some([red, green, blue]) => format!("#{red:02x}{green:02x}{blue:02x}"),
                None => fallback.to_string(),
            }
        }
        Color::Auto => fallback.to_string(),
    }
}

/// Resolve RGBA for interpolation; unknown automatic/indexed colors stay unknown.
pub fn rgba(color: Color) -> Option<[u8; 4]> {
    let rgb = match color {
        Color::Argb([a, r, g, b]) => return Some([r, g, b, a]),
        Color::Theme {
            tint,
            resolved: Some(rgb),
            ..
        } => apply_tint(rgb, tint),
        Color::Theme { index: 0, .. } | Color::Indexed(65) => [255; 3],
        Color::Theme { index: 1, .. } | Color::Indexed(64) => [0; 3],
        Color::Indexed(index) => *jade_xlsx_reader::styles::INDEXED_PALETTE.get(index as usize)?,
        _ => return None,
    };
    Some([rgb[0], rgb[1], rgb[2], 255])
}

fn apply_tint(color: [u8; 3], tint: f32) -> [u8; 3] {
    if tint == 0.0 {
        return color;
    }
    let [red, green, blue] = color.map(|value| f64::from(value) / 255.0);
    let (hue, saturation, mut luminance) = rgb_to_hsl(red, green, blue);
    let tint = f64::from(tint.clamp(-1.0, 1.0));
    luminance = if tint < 0.0 {
        luminance * (1.0 + tint)
    } else {
        luminance * (1.0 - tint) + tint
    };
    let (red, green, blue) = hsl_to_rgb(hue, saturation, luminance);
    [red, green, blue].map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn rgb_to_hsl(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let luminance = (max + min) / 2.0;
    let saturation = if delta <= f64::EPSILON {
        0.0
    } else {
        delta / (1.0 - (2.0 * luminance - 1.0).abs())
    };
    let hue = if delta <= f64::EPSILON {
        0.0
    } else if (max - red).abs() <= f64::EPSILON {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if (max - green).abs() <= f64::EPSILON {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    (hue, saturation, luminance)
}

fn hsl_to_rgb(hue: f64, saturation: f64, luminance: f64) -> (f64, f64, f64) {
    let chroma = (1.0 - (2.0 * luminance - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let secondary = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector.floor() as i32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = luminance - chroma / 2.0;
    (red + offset, green + offset, blue + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_theme_colors_and_tints_reach_css() {
        let base = Color::Theme {
            index: 4,
            tint: 0.0,
            resolved: Some([0x44, 0x72, 0xC4]),
        };
        let lighter = Color::Theme {
            index: 4,
            tint: 0.4,
            resolved: Some([0x44, 0x72, 0xC4]),
        };

        assert_eq!(to_css(base, "#ffffff"), "#4472c4");
        assert_eq!(to_css(lighter, "#ffffff"), "#8faadc");
    }
}
