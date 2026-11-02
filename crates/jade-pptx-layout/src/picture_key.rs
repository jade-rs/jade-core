//! The image a picture draws: its part, or — when the blip carries
//! corrections, colour or artistic effects, or the picture is an SVG — an
//! effect key a host turns into pixels with `jade_picture_effects`.

use jade_picture_effects::Effect;
use jade_pptx_reader::{BlipEffect, Picture, ThemeColors};

use crate::color::resolve_rgba;

fn rgb(color: &jade_pptx_reader::Color, theme: &ThemeColors) -> ([u8; 3], u8) {
    let (r, g, b, a) = resolve_rgba(color, theme);
    ([r, g, b], (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The effects of `picture`'s blip a reader draws, with their colours
/// resolved. Office 2010's image effects (`a14:imgProps`) are not among
/// them: Office bakes those into the picture the blip shows and keeps the
/// original in `a14:imgLayer`.
pub fn picture_effects(picture: &Picture, theme: &ThemeColors) -> Vec<Effect> {
    picture
        .blip_effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                BlipEffect::Luminance { .. }
                    | BlipEffect::Grayscale
                    | BlipEffect::BiLevel { .. }
                    | BlipEffect::Duotone { .. }
                    | BlipEffect::ColorChange { .. }
            )
        })
        .map(|effect| match effect {
            BlipEffect::Luminance {
                brightness,
                contrast,
            } => Effect::Luminance {
                brightness: f64::from(*brightness),
                contrast: f64::from(*contrast),
            },
            BlipEffect::Grayscale => Effect::Grayscale,
            BlipEffect::BiLevel { threshold } => Effect::BiLevel {
                threshold: f64::from(*threshold),
            },
            BlipEffect::Duotone { dark, light } => Effect::Duotone {
                dark: rgb(dark, theme).0,
                light: rgb(light, theme).0,
            },
            BlipEffect::ColorChange { from, to } => {
                let (to, alpha) = rgb(to, theme);
                Effect::ColorChange {
                    from: rgb(from, theme).0,
                    to,
                    alpha,
                }
            }
            BlipEffect::Saturation(amount) => Effect::Saturation(f64::from(*amount)),
            BlipEffect::Temperature(kelvin) => Effect::Temperature(f64::from(*kelvin)),
            BlipEffect::BrightnessContrast {
                brightness,
                contrast,
            } => Effect::BrightnessContrast {
                brightness: f64::from(*brightness),
                contrast: f64::from(*contrast),
            },
            BlipEffect::Sharpen(amount) => Effect::Sharpen(f64::from(*amount)),
            BlipEffect::Artistic(name) => Effect::Artistic(name.clone()),
            BlipEffect::BackgroundRemoval(m) => Effect::BackgroundRemoval(m.map(f64::from)),
        })
        .collect()
}

/// The image key `picture` draws: the SVG when it has one, else the bitmap,
/// with its effects.
pub fn picture_image_key(picture: &Picture, theme: &ThemeColors) -> Option<String> {
    let part = picture.svg_part.as_ref().or(picture.image_part.as_ref())?;
    Some(jade_picture_effects::key(
        part,
        &picture_effects(picture, theme),
    ))
}
