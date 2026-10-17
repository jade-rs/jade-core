use crate::document::ThemeColorTransform;
use quick_xml::events::BytesStart;

fn color_attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find_map(|attr| {
        (attr.key.as_ref() == name)
            .then(|| attr.unescape_value().ok().map(|value| value.into_owned()))
            .flatten()
    })
}

fn parse_percentage(value: &str) -> Option<f64> {
    let value = value.trim();
    let percentage = if let Some(percent) = value.strip_suffix('%') {
        let digits = percent.trim_start_matches(['-', '+']);
        let is_digits =
            |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        let valid = match digits.split_once('.') {
            Some((whole, fraction)) => is_digits(whole) && is_digits(fraction),
            None => is_digits(digits),
        };
        if !valid {
            return None;
        }
        percent.parse::<f64>().ok()? / 100.0
    } else {
        f64::from(value.parse::<i32>().ok()?) / 100_000.0
    };
    percentage.is_finite().then(|| percentage.clamp(0.0, 1.0))
}

/// ECMA-376 ST_PresetColorVal, including Office 2010 and grey aliases.
/// Names: Open XML SDK; RGB values: Microsoft System.Drawing.KnownColor.
fn preset_color(value: &str) -> Option<&'static str> {
    Some(match value {
        "aliceBlue" => "#F0F8FF",
        "antiqueWhite" => "#FAEBD7",
        "aqua" | "cyan" => "#00FFFF",
        "aquamarine" => "#7FFFD4",
        "azure" => "#F0FFFF",
        "beige" => "#F5F5DC",
        "bisque" => "#FFE4C4",
        "black" => "#000000",
        "blanchedAlmond" => "#FFEBCD",
        "blue" => "#0000FF",
        "blueViolet" => "#8A2BE2",
        "brown" => "#A52A2A",
        "burlyWood" => "#DEB887",
        "cadetBlue" => "#5F9EA0",
        "chartreuse" => "#7FFF00",
        "chocolate" => "#D2691E",
        "coral" => "#FF7F50",
        "cornflowerBlue" => "#6495ED",
        "cornsilk" => "#FFF8DC",
        "crimson" => "#DC143C",
        "dkBlue" | "darkBlue" => "#00008B",
        "dkCyan" | "darkCyan" => "#008B8B",
        "dkGoldenrod" | "darkGoldenrod" => "#B8860B",
        "dkGray" | "darkGray" | "darkGrey" | "dkGrey" => "#A9A9A9",
        "dkGreen" | "darkGreen" => "#006400",
        "dkKhaki" | "darkKhaki" => "#BDB76B",
        "dkMagenta" | "darkMagenta" => "#8B008B",
        "dkOliveGreen" | "darkOliveGreen" => "#556B2F",
        "dkOrange" | "darkOrange" => "#FF8C00",
        "dkOrchid" | "darkOrchid" => "#9932CC",
        "dkRed" | "darkRed" => "#8B0000",
        "dkSalmon" | "darkSalmon" => "#E9967A",
        "dkSeaGreen" | "darkSeaGreen" => "#8FBC8F",
        "dkSlateBlue" | "darkSlateBlue" => "#483D8B",
        "dkSlateGray" | "darkSlateGray" | "darkSlateGrey" | "dkSlateGrey" => "#2F4F4F",
        "dkTurquoise" | "darkTurquoise" => "#00CED1",
        "dkViolet" | "darkViolet" => "#9400D3",
        "deepPink" => "#FF1493",
        "deepSkyBlue" => "#00BFFF",
        "dimGray" | "dimGrey" => "#696969",
        "dodgerBlue" => "#1E90FF",
        "firebrick" => "#B22222",
        "floralWhite" => "#FFFAF0",
        "forestGreen" => "#228B22",
        "fuchsia" | "magenta" => "#FF00FF",
        "gainsboro" => "#DCDCDC",
        "ghostWhite" => "#F8F8FF",
        "gold" => "#FFD700",
        "goldenrod" => "#DAA520",
        "gray" | "grey" => "#808080",
        "green" => "#008000",
        "greenYellow" => "#ADFF2F",
        "honeydew" => "#F0FFF0",
        "hotPink" => "#FF69B4",
        "indianRed" => "#CD5C5C",
        "indigo" => "#4B0082",
        "ivory" => "#FFFFF0",
        "khaki" => "#F0E68C",
        "lavender" => "#E6E6FA",
        "lavenderBlush" => "#FFF0F5",
        "lawnGreen" => "#7CFC00",
        "lemonChiffon" => "#FFFACD",
        "ltBlue" | "lightBlue" => "#ADD8E6",
        "ltCoral" | "lightCoral" => "#F08080",
        "ltCyan" | "lightCyan" => "#E0FFFF",
        "ltGoldenrodYellow" | "lightGoldenrodYellow" => "#FAFAD2",
        "ltGray" | "lightGray" | "lightGrey" | "ltGrey" => "#D3D3D3",
        "ltGreen" | "lightGreen" => "#90EE90",
        "ltPink" | "lightPink" => "#FFB6C1",
        "ltSalmon" | "lightSalmon" => "#FFA07A",
        "ltSeaGreen" | "lightSeaGreen" => "#20B2AA",
        "ltSkyBlue" | "lightSkyBlue" => "#87CEFA",
        "ltSlateGray" | "lightSlateGray" | "lightSlateGrey" | "ltSlateGrey" => "#778899",
        "ltSteelBlue" | "lightSteelBlue" => "#B0C4DE",
        "ltYellow" | "lightYellow" => "#FFFFE0",
        "lime" => "#00FF00",
        "limeGreen" => "#32CD32",
        "linen" => "#FAF0E6",
        "maroon" => "#800000",
        "medAquamarine" | "mediumAquamarine" => "#66CDAA",
        "medBlue" | "mediumBlue" => "#0000CD",
        "medOrchid" | "mediumOrchid" => "#BA55D3",
        "medPurple" | "mediumPurple" => "#9370DB",
        "medSeaGreen" | "mediumSeaGreen" => "#3CB371",
        "medSlateBlue" | "mediumSlateBlue" => "#7B68EE",
        "medSpringGreen" | "mediumSpringGreen" => "#00FA9A",
        "medTurquoise" | "mediumTurquoise" => "#48D1CC",
        "medVioletRed" | "mediumVioletRed" => "#C71585",
        "midnightBlue" => "#191970",
        "mintCream" => "#F5FFFA",
        "mistyRose" => "#FFE4E1",
        "moccasin" => "#FFE4B5",
        "navajoWhite" => "#FFDEAD",
        "navy" => "#000080",
        "oldLace" => "#FDF5E6",
        "olive" => "#808000",
        "oliveDrab" => "#6B8E23",
        "orange" => "#FFA500",
        "orangeRed" => "#FF4500",
        "orchid" => "#DA70D6",
        "paleGoldenrod" => "#EEE8AA",
        "paleGreen" => "#98FB98",
        "paleTurquoise" => "#AFEEEE",
        "paleVioletRed" => "#DB7093",
        "papayaWhip" => "#FFEFD5",
        "peachPuff" => "#FFDAB9",
        "peru" => "#CD853F",
        "pink" => "#FFC0CB",
        "plum" => "#DDA0DD",
        "powderBlue" => "#B0E0E6",
        "purple" => "#800080",
        "red" => "#FF0000",
        "rosyBrown" => "#BC8F8F",
        "royalBlue" => "#4169E1",
        "saddleBrown" => "#8B4513",
        "salmon" => "#FA8072",
        "sandyBrown" => "#F4A460",
        "seaGreen" => "#2E8B57",
        "seaShell" => "#FFF5EE",
        "sienna" => "#A0522D",
        "silver" => "#C0C0C0",
        "skyBlue" => "#87CEEB",
        "slateBlue" => "#6A5ACD",
        "slateGray" | "slateGrey" => "#708090",
        "snow" => "#FFFAFA",
        "springGreen" => "#00FF7F",
        "steelBlue" => "#4682B4",
        "tan" => "#D2B48C",
        "teal" => "#008080",
        "thistle" => "#D8BFD8",
        "tomato" => "#FF6347",
        "turquoise" => "#40E0D0",
        "violet" => "#EE82EE",
        "wheat" => "#F5DEB3",
        "white" => "#FFFFFF",
        "whiteSmoke" => "#F5F5F5",
        "yellow" => "#FFFF00",
        "yellowGreen" => "#9ACD32",
        _ => return None,
    })
}

/// Resolve an authored color before applying its ordered child transforms.
pub(crate) fn parse_explicit_color(element: &BytesStart<'_>) -> Option<String> {
    let value = match element.local_name().as_ref() {
        b"scrgbClr" => return parse_linear_rgb(element),
        b"hslClr" => return parse_hsl_color(element),
        b"srgbClr" => color_attribute(element, b"val"),
        b"sysClr" => color_attribute(element, b"lastClr"),
        b"prstClr" => return preset_color(&color_attribute(element, b"val")?).map(str::to_owned),
        _ => return None,
    }?;
    let hex = value.trim();
    (hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_uppercase()))
}

/// Convert DrawingML's linear scRGB percentages to the sRGB display space.
pub(crate) fn parse_linear_rgb(element: &BytesStart<'_>) -> Option<String> {
    let component = |name: &[u8]| {
        let linear = parse_percentage(&color_attribute(element, name)?)?;
        // IEC 61966-2-1 transfer function; linear 50% becomes sRGB BC.
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        Some((srgb * 255.0).round() as u8)
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        component(b"r")?,
        component(b"g")?,
        component(b"b")?
    ))
}

/// DrawingML HSL uses perceptual RGB, with hue in 1/60000 degree units.
pub(crate) fn parse_hsl_color(element: &BytesStart<'_>) -> Option<String> {
    let hue = color_attribute(element, b"hue")?
        .trim()
        .parse::<i32>()
        .ok()?;
    if !(0..21_600_000).contains(&hue) {
        return None;
    }
    let saturation = parse_percentage(&color_attribute(element, b"sat")?)?;
    let luminance = parse_percentage(&color_attribute(element, b"lum")?)?;
    let (red, green, blue) = hsl_to_rgb(f64::from(hue) / 60_000.0, saturation, luminance);
    let to_byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        to_byte(red),
        to_byte(green),
        to_byte(blue)
    ))
}

pub(crate) fn parse_color_transform(element: &BytesStart<'_>) -> Option<ThemeColorTransform> {
    let value = element.attributes().flatten().find_map(|attr| {
        (attr.key.as_ref() == b"val")
            .then(|| attr.unescape_value().ok()?.trim().parse::<f64>().ok())
            .flatten()
    })? / 100_000.0;
    if !value.is_finite() {
        return None;
    }
    match element.local_name().as_ref() {
        b"lumMod" => Some(ThemeColorTransform::LuminanceMod(value)),
        b"lumOff" => Some(ThemeColorTransform::LuminanceOffset(value)),
        b"tint" => Some(ThemeColorTransform::Tint(value)),
        b"shade" => Some(ThemeColorTransform::Shade(value)),
        _ => None,
    }
}

/// Resolve ordered DrawingML transforms for a concrete RGB color.
pub fn apply_color_transforms(color: &str, transforms: &[ThemeColorTransform]) -> Option<String> {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let component = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    let mut red = component(0..2)?;
    let mut green = component(2..4)?;
    let mut blue = component(4..6)?;

    for transform in transforms {
        match transform {
            ThemeColorTransform::LuminanceMod(value)
            | ThemeColorTransform::LuminanceOffset(value) => {
                let (hue, saturation, luminance) = rgb_to_hsl(red, green, blue);
                let luminance = match transform {
                    ThemeColorTransform::LuminanceMod(_) => luminance * value,
                    ThemeColorTransform::LuminanceOffset(_) => luminance + value,
                    _ => unreachable!("matched luminance transform"),
                }
                .clamp(0.0, 1.0);
                (red, green, blue) = hsl_to_rgb(hue, saturation, luminance);
            }
            ThemeColorTransform::Tint(value) => {
                // ECMA-376 §20.1.2.3.34: the percentage is the retained
                // input color; the remainder is supplied by white.
                let input = value.clamp(0.0, 1.0);
                red = red * input + (1.0 - input);
                green = green * input + (1.0 - input);
                blue = blue * input + (1.0 - input);
            }
            ThemeColorTransform::Shade(value) => {
                // ECMA-376 §20.1.2.3.31: the percentage is the retained
                // input color; the remainder is supplied by black.
                let input = value.clamp(0.0, 1.0);
                red *= input;
                green *= input;
                blue *= input;
            }
        }
    }

    let to_byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        to_byte(red),
        to_byte(green),
        to_byte(blue)
    ))
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
    let hue_sector = hue / 60.0;
    let secondary = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
    let (red_prime, green_prime, blue_prime) = match hue_sector.floor() as i32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let match_value = luminance - chroma / 2.0;
    (
        red_prime + match_value,
        green_prime + match_value,
        blue_prime + match_value,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_rgb_values_are_rejected_without_slicing_unicode() {
        for color in ["#12345", "#GG0000", "#aé000", "#💙00"] {
            assert_eq!(
                apply_color_transforms(color, &[ThemeColorTransform::Shade(0.5)]),
                None
            );
        }
    }
}
