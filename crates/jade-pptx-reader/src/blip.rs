//! What a picture's `<a:blip>` says about its pixels: the colour
//! transforms ECMA-376 defines on the blip itself (`a:lum`, `a:grayscl`,
//! `a:biLevel`, `a:duotone`, `a:clrChange`), which readers apply, and the
//! image effects Office 2010 added in the `a14:imgProps` extension
//! (corrections, colour saturation and tone, artistic effects, background
//! removal), which Office bakes into the picture the blip shows — keeping
//! the original in `a14:imgLayer` — so they say what was done; plus the
//! SVG an `asvg:svgBlip` extension carries beside the bitmap fallback.

use serde::{Deserialize, Serialize};

use crate::slide::{parse_color, Color};
use crate::xml_tree::XmlElement;

/// One adjustment of a picture, in document order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlipEffect {
    /// `a:lum`: brightness and contrast, −1…1.
    Luminance { brightness: f32, contrast: f32 },
    /// `a:grayscl`.
    Grayscale,
    /// `a:biLevel thresh`, 0…1.
    BiLevel { threshold: f32 },
    /// `a:duotone`: the dark and light colours.
    Duotone { dark: Color, light: Color },
    /// `a:clrChange`: 透明色を指定 and its kin.
    ColorChange { from: Color, to: Color },
    /// `a14:saturation sat`: 1 unchanged.
    Saturation(f32),
    /// `a14:colorTemperature colorTemp`, kelvin.
    Temperature(f32),
    /// `a14:brightnessContrast`, −1…1 each.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// `a14:sharpenSoften amount`, −1…1.
    Sharpen(f32),
    /// An `a14:artistic…` effect, by its element name.
    Artistic(String),
    /// `a14:backgroundRemoval`: the kept rectangle (left, top, right,
    /// bottom fractions).
    BackgroundRemoval([f32; 4]),
}

fn fraction(element: &XmlElement, name: &str) -> Option<f32> {
    element
        .attr(name)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value / 100_000.0)
}

fn color_of(child: &XmlElement) -> Option<Color> {
    parse_color(&XmlElement {
        name: String::new(),
        attrs: Vec::new(),
        children: vec![child.clone()],
        text: String::new(),
    })
}

/// The effects of the `<a:blip>` in `blip_fill`.
pub(crate) fn blip_effects(blip_fill: &XmlElement) -> Vec<BlipEffect> {
    let Some(blip) = blip_fill.child("blip") else {
        return Vec::new();
    };
    let mut effects = Vec::new();
    for child in &blip.children {
        let effect = match child.name.as_str() {
            "lum" => Some(BlipEffect::Luminance {
                brightness: fraction(child, "bright").unwrap_or(0.0),
                contrast: fraction(child, "contrast").unwrap_or(0.0),
            }),
            "grayscl" => Some(BlipEffect::Grayscale),
            "biLevel" => Some(BlipEffect::BiLevel {
                threshold: fraction(child, "thresh").unwrap_or(0.5),
            }),
            "duotone" => {
                let mut colors = child.children.iter().filter_map(color_of);
                match (colors.next(), colors.next()) {
                    (Some(dark), Some(light)) => Some(BlipEffect::Duotone { dark, light }),
                    _ => None,
                }
            }
            "clrChange" => {
                let from = child.child("clrFrom").and_then(parse_color);
                let to = child.child("clrTo").and_then(parse_color);
                from.zip(to)
                    .map(|(from, to)| BlipEffect::ColorChange { from, to })
            }
            _ => None,
        };
        effects.extend(effect);
    }
    if let Some(layer) = blip.descendant("imgLayer") {
        for wrapper in layer.children.iter().filter(|c| c.name == "imgEffect") {
            for effect in &wrapper.children {
                let read = match effect.name.as_str() {
                    "saturation" => Some(BlipEffect::Saturation(
                        fraction(effect, "sat").unwrap_or(1.0),
                    )),
                    "colorTemperature" => Some(BlipEffect::Temperature(
                        effect
                            .attr("colorTemp")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(6500.0),
                    )),
                    "brightnessContrast" => Some(BlipEffect::BrightnessContrast {
                        brightness: fraction(effect, "bright").unwrap_or(0.0),
                        contrast: fraction(effect, "contrast").unwrap_or(0.0),
                    }),
                    "sharpenSoften" => Some(BlipEffect::Sharpen(
                        fraction(effect, "amount").unwrap_or(0.0),
                    )),
                    "backgroundRemoval" => Some(BlipEffect::BackgroundRemoval([
                        fraction(effect, "l").unwrap_or(0.0),
                        fraction(effect, "t").unwrap_or(0.0),
                        fraction(effect, "r").unwrap_or(1.0),
                        fraction(effect, "b").unwrap_or(1.0),
                    ])),
                    name if name.starts_with("artistic") => {
                        Some(BlipEffect::Artistic(name.to_string()))
                    }
                    _ => None,
                };
                effects.extend(read);
            }
        }
    }
    effects
}

/// The `r:embed` of the blip's `asvg:svgBlip`: the SVG the bitmap stands in
/// for.
pub(crate) fn svg_rel_id(blip_fill: &XmlElement) -> Option<String> {
    blip_fill
        .child("blip")?
        .descendant("svgBlip")?
        .attr("embed")
        .map(str::to_string)
}
