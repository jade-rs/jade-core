//! What Office's picture tools do to a picture's pixels.
//!
//! PowerPoint keeps a picture's bytes as they were inserted and records its
//! corrections, colour and artistic effects on the `a:blip` (and in the
//! `a14` image-effect extension); every reader applies them when drawing.
//! A layout names the picture it wants as an *effect key* — the image part
//! and its effects, `ppt/media/image1.png#jade-fx=gray;lum:0.2,0` — and a
//! host turns the key into bytes with [`image_bytes`], which applies the
//! effects (and draws an SVG, or picks one frame of an animated GIF) once
//! and keeps the result.
//!
//! The same operations serve the writer's 図の圧縮 ([`compress`]) and 背景の
//! 削除 preview.

/// One adjustment of a picture, in the order the blip lists them.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `a:lum`: brightness and contrast, −1…1.
    Luminance { brightness: f64, contrast: f64 },
    /// `a:grayscl`.
    Grayscale,
    /// `a:biLevel`: black and white at a luminance threshold, 0…1.
    BiLevel { threshold: f64 },
    /// `a:duotone`: dark to light between two colours.
    Duotone { dark: [u8; 3], light: [u8; 3] },
    /// `a:clrChange`: one colour to another (透明色を指定).
    ColorChange {
        from: [u8; 3],
        to: [u8; 3],
        alpha: u8,
    },
    /// `a14:saturation`: 0 grey, 1 unchanged, up to 4.
    Saturation(f64),
    /// `a14:colorTemperature`, in kelvin; 6500 unchanged.
    Temperature(f64),
    /// `a14:brightnessContrast`, −1…1 each.
    BrightnessContrast { brightness: f64, contrast: f64 },
    /// `a14:sharpenSoften`, −1 (soft) … 1 (sharp).
    Sharpen(f64),
    /// `a14:artistic…`: an artistic effect by its element name.
    Artistic(String),
    /// `a14:backgroundRemoval`: the subject's rectangle, fractions.
    BackgroundRemoval([f64; 4]),
    /// One frame of an animated GIF.
    Frame(usize),
}

const MARK: &str = "#jade-fx=";

fn hex(color: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn unhex(text: &str) -> Option<[u8; 3]> {
    let value = u32::from_str_radix(text, 16).ok()?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

fn number(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Effect {
    fn encode(&self) -> String {
        match self {
            Self::Luminance {
                brightness,
                contrast,
            } => format!("lum:{},{}", number(*brightness), number(*contrast)),
            Self::Grayscale => "gray".into(),
            Self::BiLevel { threshold } => format!("bilevel:{}", number(*threshold)),
            Self::Duotone { dark, light } => format!("duo:{},{}", hex(*dark), hex(*light)),
            Self::ColorChange { from, to, alpha } => {
                format!("clr:{},{},{alpha}", hex(*from), hex(*to))
            }
            Self::Saturation(amount) => format!("sat:{}", number(*amount)),
            Self::Temperature(kelvin) => format!("temp:{}", number(*kelvin)),
            Self::BrightnessContrast {
                brightness,
                contrast,
            } => format!("bc:{},{}", number(*brightness), number(*contrast)),
            Self::Sharpen(amount) => format!("sharp:{}", number(*amount)),
            Self::Artistic(name) => format!("art:{name}"),
            Self::BackgroundRemoval(m) => format!(
                "bg:{},{},{},{}",
                number(m[0]),
                number(m[1]),
                number(m[2]),
                number(m[3])
            ),
            Self::Frame(index) => format!("frame:{index}"),
        }
    }

    fn decode(token: &str) -> Option<Self> {
        let (name, rest) = token.split_once(':').unwrap_or((token, ""));
        let numbers: Vec<f64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
        let parts: Vec<&str> = rest.split(',').collect();
        Some(match name {
            "lum" => Self::Luminance {
                brightness: *numbers.first()?,
                contrast: *numbers.get(1)?,
            },
            "gray" => Self::Grayscale,
            "bilevel" => Self::BiLevel {
                threshold: *numbers.first()?,
            },
            "duo" => Self::Duotone {
                dark: unhex(parts.first()?)?,
                light: unhex(parts.get(1)?)?,
            },
            "clr" => Self::ColorChange {
                from: unhex(parts.first()?)?,
                to: unhex(parts.get(1)?)?,
                alpha: parts.get(2)?.parse().ok()?,
            },
            "sat" => Self::Saturation(*numbers.first()?),
            "temp" => Self::Temperature(*numbers.first()?),
            "bc" => Self::BrightnessContrast {
                brightness: *numbers.first()?,
                contrast: *numbers.get(1)?,
            },
            "sharp" => Self::Sharpen(*numbers.first()?),
            "art" => Self::Artistic(rest.to_string()),
            "bg" => Self::BackgroundRemoval([
                *numbers.first()?,
                *numbers.get(1)?,
                *numbers.get(2)?,
                *numbers.get(3)?,
            ]),
            "frame" => Self::Frame(rest.parse().ok()?),
            _ => return None,
        })
    }
}

/// The key a layout gives a host for `part` with `effects`: `part` itself
/// when there are none.
pub fn key(part: &str, effects: &[Effect]) -> String {
    if effects.is_empty() {
        return part.to_string();
    }
    let tokens: Vec<String> = effects.iter().map(Effect::encode).collect();
    format!("{part}{MARK}{}", tokens.join(";"))
}

/// A key's image part and effects.
pub fn parse_key(key: &str) -> (&str, Vec<Effect>) {
    match key.split_once(MARK) {
        Some((part, effects)) => (
            part,
            effects.split(';').filter_map(Effect::decode).collect(),
        ),
        None => (key, Vec::new()),
    }
}

#[cfg(feature = "pixels")]
mod pixels;
#[cfg(feature = "pixels")]
pub use pixels::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_the_effects_and_read_back() {
        let effects = vec![
            Effect::Luminance {
                brightness: 0.2,
                contrast: -0.1,
            },
            Effect::Duotone {
                dark: [0x20, 0x38, 0x64],
                light: [255, 255, 255],
            },
            Effect::Artistic("artisticBlur".into()),
            Effect::BackgroundRemoval([0.1, 0.1, 0.9, 0.9]),
        ];
        let key = key("ppt/media/image1.png", &effects);
        let (part, read) = parse_key(&key);
        assert_eq!(part, "ppt/media/image1.png");
        assert_eq!(read, effects);
        assert_eq!(super::key("ppt/media/a.png", &[]), "ppt/media/a.png");
    }
}
