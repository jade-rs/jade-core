//! The parts of a `w:drawing` read beside its frame and its fill and line:
//! the anchor's options and the object's name, a picture's colour
//! adjustments and Word's corrections, reflection and bevel, a freeform's
//! paths, a connector's ends, WordArt's warp and linked text boxes.
//!
//! `parse_drawing` walks the drawing once; it hands every element to
//! [`DrawingExtras`] as it goes, which keeps its own place in the tree.

use quick_xml::events::BytesStart;

use crate::document::*;

/// A colour being read, and where it goes when it ends.
#[derive(Clone, Copy, PartialEq)]
enum ColorTarget {
    Duotone,
    ChangeFrom,
    ChangeTo,
}

#[derive(Default)]
pub(super) struct DrawingExtras {
    pub object_id: Option<u32>,
    pub name: Option<String>,
    pub hidden: bool,
    pub locked: bool,
    pub allow_overlap: Option<bool>,
    pub layout_in_cell: Option<bool>,
    pub blip_effects: Vec<BlipEffect>,
    pub corrections: PictureCorrections,
    pub svg_relationship_id: Option<String>,
    pub reflection: Option<ShapeReflection>,
    pub bevel: Option<ShapeBevel>,
    pub custom_geometry: Option<CustomGeometry>,
    pub connector: ConnectorEnds,
    pub text_warp: Option<TextWarp>,
    pub link: Option<TextBoxLink>,
    pub wordart: Option<WordArtText>,
    pub ink_relationship_id: Option<String>,

    in_blip: bool,
    in_effect_list: bool,
    duotone: Option<Vec<DrawingColor>>,
    color_change: Option<(Option<DrawingColor>, Option<DrawingColor>)>,
    color_target: Option<ColorTarget>,
    color: Option<DrawingColor>,
    in_custom_geometry: bool,
    path: Option<GeometryPath>,
    /// The segment whose points are being read, and the points so far.
    segment: Option<(&'static str, Vec<(f64, f64)>)>,
    in_text_warp: bool,
}

fn attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref() == name)
        .map(|attribute| {
            attribute
                .unescape_value()
                .map(|value| value.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attribute.value).into_owned())
        })
}

fn number(element: &BytesStart<'_>, name: &[u8]) -> Option<f64> {
    attribute(element, name)?.trim().parse().ok()
}

fn flag(element: &BytesStart<'_>, name: &[u8]) -> Option<bool> {
    attribute(element, name).map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on"
        )
    })
}

/// DrawingML percentages: thousandths of a percent (`40000` is 40%).
fn fraction(element: &BytesStart<'_>, name: &[u8]) -> Option<f64> {
    number(element, name).map(|value| value / 100_000.0)
}

fn emu_to_px(value: f64) -> f64 {
    value / 9525.0
}

impl DrawingExtras {
    /// An element starting; `empty` when it has no children.
    pub fn start(&mut self, element: &BytesStart<'_>, empty: bool) {
        let name = element.name();
        let name = name.as_ref();
        match name {
            b"wp:anchor" => {
                self.locked = flag(element, b"locked").unwrap_or(false);
                self.allow_overlap = flag(element, b"allowOverlap");
                self.layout_in_cell = flag(element, b"layoutInCell");
                if flag(element, b"hidden") == Some(true) {
                    self.hidden = true;
                }
            }
            b"wp:docPr" => {
                self.object_id = number(element, b"id").map(|id| id as u32);
                self.name = attribute(element, b"name");
                if flag(element, b"hidden") == Some(true) {
                    self.hidden = true;
                }
            }
            b"a:blip" => self.in_blip = !empty,
            b"a:lum" if self.in_blip => self.blip_effects.push(BlipEffect::Luminance {
                brightness: fraction(element, b"bright").unwrap_or(0.0),
                contrast: fraction(element, b"contrast").unwrap_or(0.0),
            }),
            b"a:grayscl" if self.in_blip => self.blip_effects.push(BlipEffect::Grayscale),
            b"a:biLevel" if self.in_blip => self.blip_effects.push(BlipEffect::BiLevel {
                threshold: fraction(element, b"thresh").unwrap_or(0.5),
            }),
            b"a:alphaModFix" if self.in_blip => self.blip_effects.push(BlipEffect::AlphaModFix {
                amount: fraction(element, b"amt").unwrap_or(1.0),
            }),
            b"a:duotone" if self.in_blip && !empty => self.duotone = Some(Vec::new()),
            b"a:clrChange" if self.in_blip && !empty => self.color_change = Some((None, None)),
            b"a:clrFrom" if self.color_change.is_some() => {
                self.color_target = Some(ColorTarget::ChangeFrom)
            }
            b"a:clrTo" if self.color_change.is_some() => {
                self.color_target = Some(ColorTarget::ChangeTo)
            }
            b"a:srgbClr" | b"a:prstClr" | b"a:sysClr" | b"a:scrgbClr" | b"a:hslClr"
            | b"a:schemeClr"
                if self.duotone.is_some() || self.color_target.is_some() =>
            {
                let mut color = DrawingColor::default();
                if name == b"a:schemeClr" {
                    color.theme = attribute(element, b"val");
                } else {
                    color.rgb = crate::color::parse_explicit_color(element);
                }
                if self.duotone.is_some() && self.color_target.is_none() {
                    self.color_target = Some(ColorTarget::Duotone);
                }
                if empty {
                    self.finish_color(color);
                } else {
                    self.color = Some(color);
                }
            }
            b"a:alpha" if self.color.is_some() => {
                if let Some(color) = self.color.as_mut() {
                    color.alpha = fraction(element, b"val");
                }
            }
            b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" if self.color.is_some() => {
                if let (Some(color), Some(transform)) = (
                    self.color.as_mut(),
                    crate::color::parse_color_transform(element),
                ) {
                    color.transforms.push(transform);
                }
            }
            // Word 2010's corrections, stored with the original picture.
            b"a14:imgLayer" => {
                self.corrections.original_relationship_id = attribute(element, b"r:embed");
            }
            b"a14:brightnessContrast" => {
                self.corrections.brightness = fraction(element, b"bright");
                self.corrections.contrast = fraction(element, b"contrast");
            }
            b"a14:saturation" => {
                self.corrections.saturation = fraction(element, b"sat");
            }
            b"a14:colorTemperature" => {
                self.corrections.temperature = number(element, b"colorTemp");
            }
            b"a14:sharpenSoften" => {
                self.corrections.sharpness = fraction(element, b"amount");
            }
            b"a14:backgroundRemoval" => {
                self.corrections.background_removal = Some([
                    fraction(element, b"l").unwrap_or(0.0),
                    fraction(element, b"t").unwrap_or(0.0),
                    fraction(element, b"r").unwrap_or(0.0),
                    fraction(element, b"b").unwrap_or(0.0),
                ]);
            }
            _ if name.starts_with(b"a14:artistic") => {
                self.corrections.artistic_effect =
                    Some(String::from_utf8_lossy(&name[4..]).into_owned());
            }
            b"w14:contentPart" => {
                self.ink_relationship_id = attribute(element, b"r:id");
            }
            b"asvg:svgBlip" => {
                self.svg_relationship_id = attribute(element, b"r:embed");
            }
            b"a:effectLst" => self.in_effect_list = !empty,
            b"a:reflection" if self.in_effect_list => {
                self.reflection = Some(ShapeReflection {
                    blur_radius_px: emu_to_px(number(element, b"blurRad").unwrap_or(0.0)),
                    start_opacity: fraction(element, b"stA").unwrap_or(1.0),
                    end_opacity: fraction(element, b"endA").unwrap_or(0.0),
                    end_position: fraction(element, b"endPos").unwrap_or(1.0),
                    distance_px: emu_to_px(number(element, b"dist").unwrap_or(0.0)),
                    direction_deg: number(element, b"dir").unwrap_or(0.0) / 60_000.0,
                    scale_y: fraction(element, b"sy").unwrap_or(1.0),
                });
            }
            b"a:bevelT" => {
                self.bevel = Some(ShapeBevel {
                    preset: attribute(element, b"prst").unwrap_or_else(|| "circle".into()),
                    width_px: emu_to_px(number(element, b"w").unwrap_or(76_200.0)),
                    height_px: emu_to_px(number(element, b"h").unwrap_or(76_200.0)),
                });
            }
            b"a:custGeom" => {
                self.in_custom_geometry = !empty;
                self.custom_geometry = Some(CustomGeometry::default());
            }
            b"a:path" if self.in_custom_geometry => {
                let path = GeometryPath {
                    width: number(element, b"w").unwrap_or(0.0),
                    height: number(element, b"h").unwrap_or(0.0),
                    fill: attribute(element, b"fill").as_deref() != Some("none"),
                    stroke: flag(element, b"stroke").unwrap_or(true),
                    commands: Vec::new(),
                };
                if empty {
                    if let Some(geometry) = self.custom_geometry.as_mut() {
                        geometry.paths.push(path);
                    }
                } else {
                    self.path = Some(path);
                }
            }
            b"a:moveTo" | b"a:lnTo" | b"a:cubicBezTo" | b"a:quadBezTo"
                if self.path.is_some() && !empty =>
            {
                let kind = match name {
                    b"a:moveTo" => "move",
                    b"a:lnTo" => "line",
                    b"a:cubicBezTo" => "cubic",
                    _ => "quad",
                };
                self.segment = Some((kind, Vec::new()));
            }
            b"a:pt" => {
                if let Some((_, points)) = self.segment.as_mut() {
                    points.push((
                        number(element, b"x").unwrap_or(0.0),
                        number(element, b"y").unwrap_or(0.0),
                    ));
                }
            }
            b"a:arcTo" if self.path.is_some() => {
                if let Some(path) = self.path.as_mut() {
                    path.commands.push(PathCommand::ArcTo {
                        width_radius: number(element, b"wR").unwrap_or(0.0),
                        height_radius: number(element, b"hR").unwrap_or(0.0),
                        start_deg: number(element, b"stAng").unwrap_or(0.0) / 60_000.0,
                        sweep_deg: number(element, b"swAng").unwrap_or(0.0) / 60_000.0,
                    });
                }
            }
            b"a:close" if self.path.is_some() => {
                if let Some(path) = self.path.as_mut() {
                    path.commands.push(PathCommand::Close);
                }
            }
            b"a:stCxn" | b"a:endCxn" => {
                let reference = number(element, b"id").map(|id| ConnectionReference {
                    id: id as u32,
                    index: number(element, b"idx").unwrap_or(0.0) as u32,
                });
                if name == b"a:stCxn" {
                    self.connector.start = reference;
                } else {
                    self.connector.end = reference;
                }
            }
            b"a:prstTxWarp" => {
                self.text_warp = Some(TextWarp {
                    preset: attribute(element, b"prst").unwrap_or_else(|| "textNoShape".into()),
                    adjustments: Vec::new(),
                });
                self.in_text_warp = !empty;
            }
            b"a:gd" if self.in_text_warp => {
                let value = attribute(element, b"fmla").and_then(|formula| {
                    formula
                        .trim()
                        .strip_prefix("val ")
                        .and_then(|value| value.trim().parse::<i64>().ok())
                });
                if let (Some(warp), Some(name), Some(value)) =
                    (self.text_warp.as_mut(), attribute(element, b"name"), value)
                {
                    warp.adjustments.push((name, value));
                }
            }
            b"wps:txbx" => {
                if let Some(id) = number(element, b"id") {
                    self.link = Some(TextBoxLink {
                        id: id as u32,
                        sequence: 0,
                    });
                }
            }
            b"wps:linkedTxbx" => {
                if let Some(id) = number(element, b"id") {
                    self.link = Some(TextBoxLink {
                        id: id as u32,
                        sequence: number(element, b"seq").unwrap_or(1.0) as u32,
                    });
                }
            }
            _ => {}
        }
    }

    pub fn end(&mut self, name: &[u8]) {
        match name {
            b"a:blip" => self.in_blip = false,
            b"a:effectLst" => self.in_effect_list = false,
            b"a:srgbClr" | b"a:prstClr" | b"a:sysClr" | b"a:scrgbClr" | b"a:hslClr"
            | b"a:schemeClr" => {
                if let Some(color) = self.color.take() {
                    self.finish_color(color);
                }
            }
            b"a:clrFrom" | b"a:clrTo" => self.color_target = None,
            b"a:duotone" => {
                if let Some(colors) = self.duotone.take() {
                    if let [dark, light] = colors.as_slice() {
                        self.blip_effects.push(BlipEffect::Duotone {
                            dark: dark.clone(),
                            light: light.clone(),
                        });
                    }
                }
                self.color_target = None;
            }
            b"a:clrChange" => {
                if let Some((Some(from), Some(to))) = self.color_change.take() {
                    self.blip_effects.push(BlipEffect::ColorChange { from, to });
                }
            }
            b"a:custGeom" => self.in_custom_geometry = false,
            b"a:path" => {
                if let (Some(path), Some(geometry)) =
                    (self.path.take(), self.custom_geometry.as_mut())
                {
                    geometry.paths.push(path);
                }
            }
            b"a:moveTo" | b"a:lnTo" | b"a:cubicBezTo" | b"a:quadBezTo" => {
                if let (Some((kind, points)), Some(path)) =
                    (self.segment.take(), self.path.as_mut())
                {
                    let command = match (kind, points.as_slice()) {
                        ("move", [(x, y), ..]) => Some(PathCommand::MoveTo(*x, *y)),
                        ("line", [(x, y), ..]) => Some(PathCommand::LineTo(*x, *y)),
                        ("cubic", [(x1, y1), (x2, y2), (x, y), ..]) => {
                            Some(PathCommand::CubicTo(*x1, *y1, *x2, *y2, *x, *y))
                        }
                        ("quad", [(x1, y1), (x, y), ..]) => {
                            Some(PathCommand::QuadTo(*x1, *y1, *x, *y))
                        }
                        _ => None,
                    };
                    path.commands.extend(command);
                }
            }
            b"a:prstTxWarp" => self.in_text_warp = false,
            _ => {}
        }
    }

    fn finish_color(&mut self, color: DrawingColor) {
        match self.color_target {
            Some(ColorTarget::Duotone) => {
                if let Some(colors) = self.duotone.as_mut() {
                    colors.push(color);
                }
            }
            Some(ColorTarget::ChangeFrom) => {
                if let Some(change) = self.color_change.as_mut() {
                    change.0 = Some(color);
                }
            }
            Some(ColorTarget::ChangeTo) => {
                if let Some(change) = self.color_change.as_mut() {
                    change.1 = Some(color);
                }
            }
            None => {}
        }
        if self.duotone.is_some() && self.color_change.is_none() {
            // The next colour of the pair.
            self.color_target = Some(ColorTarget::Duotone);
        }
    }

    /// Puts what was read onto a picture.
    pub fn apply_to_image(self, image: &mut RunImage, mut format: PictureFormat) {
        self.apply_to_layout(&mut image.layout);
        format.blip_effects = self.blip_effects;
        format.corrections = (!self.corrections.is_empty()
            || self.corrections.original_relationship_id.is_some())
        .then_some(self.corrections);
        format.svg_relationship_id = self.svg_relationship_id;
        format.ink_relationship_id = self.ink_relationship_id;
        format.reflection = self.reflection;
        format.bevel = self.bevel;
        image.format = (!format.is_empty()).then(|| Box::new(format));
    }

    /// Puts what was read onto a shape or text box.
    pub fn apply_to_text_box(self, text_box: &mut RunTextBox) {
        self.apply_to_layout(&mut text_box.layout);
        let extras = ShapeExtras {
            reflection: self.reflection,
            bevel: self.bevel,
            custom_geometry: self
                .custom_geometry
                .filter(|geometry| !geometry.paths.is_empty()),
            connector: (self.connector != ConnectorEnds::default()).then_some(self.connector),
            text_warp: self.text_warp,
            link: self.link,
            wordart: self.wordart,
        };
        text_box.extras = (!extras.is_empty()).then(|| Box::new(extras));
    }

    fn apply_to_layout(&self, layout: &mut RunImageLayout) {
        let options = AnchorOptions {
            object_id: self.object_id,
            name: self.name.clone(),
            hidden: self.hidden,
            locked: self.locked,
            allow_overlap: self.allow_overlap,
            layout_in_cell: self.layout_in_cell,
        };
        layout.options = (options != AnchorOptions::default()).then(|| Box::new(options));
    }
}
