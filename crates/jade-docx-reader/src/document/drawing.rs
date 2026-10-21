//! What a drawing object carries beyond its frame: a picture's colour
//! adjustments and effects, a shape's reflection and bevel, a freeform's
//! points, a connector's ends, WordArt's warp. Read so the object is drawn as
//! Word draws it; written by editing the drawing's XML (the editor's
//! `word_document::objects`), so these mirror DrawingML closely.

use serde::{Deserialize, Serialize};

use super::{RunImage, RunImageLayout, RunTextBox, ShapeGlow, ShapeShadow, TextBoxBorder, TextBoxShape, ThemeColorTransform};

/// A DrawingML colour: explicit (`a:srgbClr`, `a:prstClr`, `a:sysClr`) or a
/// theme slot (`a:schemeClr`) with its transforms, which loading resolves
/// against the theme into `rgb`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DrawingColor {
    /// `#RRGGBB`, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transforms: Vec<ThemeColorTransform>,
    /// `a:alpha`, 0.0–1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
}

/// A colour adjustment DrawingML applies to a picture's pixels as it draws
/// them (children of `a:blip`, ECMA-376 Part 1 §20.1.8).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BlipEffect {
    /// `a:lum`: brightness and contrast, each -1.0–1.0.
    Luminance { brightness: f64, contrast: f64 },
    /// `a:grayscl`.
    Grayscale,
    /// `a:biLevel`: black or white by luminance against `threshold`.
    BiLevel { threshold: f64 },
    /// `a:duotone`: luminance mapped from `dark` to `light` (Word's 色の変更).
    Duotone {
        dark: DrawingColor,
        light: DrawingColor,
    },
    /// `a:alphaModFix`: opacity multiplied by `amount` (Word's 透明度).
    AlphaModFix { amount: f64 },
    /// `a:clrChange`: pixels of `from` become `to` (透明色を指定).
    ColorChange {
        from: DrawingColor,
        to: DrawingColor,
    },
}

/// Word 2010's picture corrections (`a14:imgProps`). Word writes the
/// corrected picture as the blip itself and keeps the original in
/// `a14:imgLayer`, so these say what was done rather than what to draw.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PictureCorrections {
    /// The uncorrected picture (`a14:imgLayer@r:embed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_relationship_id: Option<String>,
    /// `a14:brightnessContrast@bright`, -1.0–1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f64>,
    /// `a14:saturation@sat`, 0.0–4.0 (1.0 unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
    /// `a14:colorTemperature@colorTemp`, in kelvin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// `a14:sharpenSoften@amount`, -1.0–1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharpness: Option<f64>,
    /// The artistic effect's element name without its prefix
    /// (`artisticPencilSketch`, `artisticBlur`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artistic_effect: Option<String>,
    /// `a14:backgroundRemoval`: the kept rectangle, as fractions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_removal: Option<[f64; 4]>,
}

impl PictureCorrections {
    pub fn is_empty(&self) -> bool {
        self.brightness.is_none()
            && self.contrast.is_none()
            && self.saturation.is_none()
            && self.temperature.is_none()
            && self.sharpness.is_none()
            && self.artistic_effect.is_none()
            && self.background_removal.is_none()
    }
}

/// `a:reflection` (ECMA-376 Part 1 §20.1.8.50).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeReflection {
    pub blur_radius_px: f64,
    /// Opacity where the reflection starts and ends, 0.0–1.0.
    pub start_opacity: f64,
    pub end_opacity: f64,
    /// How far down the object the reflection fades out, 0.0–1.0.
    pub end_position: f64,
    pub distance_px: f64,
    /// Direction of the offset, clockwise from the right, in degrees.
    pub direction_deg: f64,
    /// Vertical scale; Word's reflections mirror with -1.0.
    pub scale_y: f64,
}

impl Default for ShapeReflection {
    fn default() -> Self {
        Self {
            blur_radius_px: 0.0,
            start_opacity: 1.0,
            end_opacity: 0.0,
            end_position: 1.0,
            distance_px: 0.0,
            direction_deg: 0.0,
            scale_y: 1.0,
        }
    }
}

/// The top bevel of `a:sp3d` (`a:bevelT`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeBevel {
    /// `ST_BevelPresetType`: `circle`, `relaxedInset`, `cross`, …
    pub preset: String,
    pub width_px: f64,
    pub height_px: f64,
}

/// One segment of a freeform path (`a:path`, ECMA-376 Part 1 §20.1.9.15),
/// in the path's own coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PathCommand {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    /// Two control points and the end point.
    CubicTo(f64, f64, f64, f64, f64, f64),
    QuadTo(f64, f64, f64, f64),
    /// `a:arcTo`: radii, start angle and sweep in degrees.
    ArcTo {
        width_radius: f64,
        height_radius: f64,
        start_deg: f64,
        sweep_deg: f64,
    },
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeometryPath {
    /// The path's coordinate space (`a:path@w`/`@h`); zero means the shape's
    /// own extent in EMU.
    pub width: f64,
    pub height: f64,
    pub fill: bool,
    pub stroke: bool,
    pub commands: Vec<PathCommand>,
}

/// `a:custGeom`: a freeform's paths.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomGeometry {
    pub paths: Vec<GeometryPath>,
}

/// One end of a connector glued to a shape (`a:stCxn` / `a:endCxn`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReference {
    /// The shape's drawing id (`wp:docPr@id`).
    pub id: u32,
    /// The connection site on it.
    pub index: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorEnds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<ConnectionReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<ConnectionReference>,
}

/// WordArt's text transform (`a:prstTxWarp`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextWarp {
    /// `ST_TextShapeType`: `textArchUp`, `textWave1`, …
    pub preset: String,
    /// `a:avLst` guides, by name, in DrawingML units.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<(String, i64)>,
}

/// WordArt's text effects (Word 2010's `w14:` run properties) on the first
/// run of a shape's text, which jade draws the shape's text with.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WordArtText {
    /// `w14:textFill`'s colour; `None` with `no_fill` for an outline only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<DrawingColor>,
    #[serde(default)]
    pub no_fill: bool,
    /// `w14:textOutline`'s colour and width (px).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<(DrawingColor, f64)>,
    #[serde(default)]
    pub shadow: bool,
    /// `w14:glow`'s colour and radius (px).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<(DrawingColor, f64)>,
}

impl WordArtText {
    /// Reads the first run's `w14:` effects from a text box's content.
    pub fn from_content(xml: &str) -> Option<Self> {
        if !xml.contains("w14:") {
            return None;
        }
        let document = crate::docx_xml::DocxXmlDocument::parse(&format!("<root>{xml}</root>")).ok()?;
        fn find<'a>(element: &'a crate::docx_xml::XmlElement, name: &str) -> Option<&'a crate::docx_xml::XmlElement> {
            for node in &element.children {
                if let crate::docx_xml::XmlNode::Element(child) = node {
                    if child.name == name {
                        return Some(child);
                    }
                    if let Some(found) = find(child, name) {
                        return Some(found);
                    }
                }
            }
            None
        }
        fn attr<'a>(element: &'a crate::docx_xml::XmlElement, name: &str) -> Option<&'a str> {
            element
                .attributes
                .iter()
                .find(|attribute| attribute.name == name)
                .map(|attribute| attribute.value.as_str())
        }
        fn color(element: &crate::docx_xml::XmlElement) -> Option<DrawingColor> {
            if let Some(rgb) = find(element, "w14:srgbClr").and_then(|c| attr(c, "w14:val")) {
                return Some(DrawingColor {
                    rgb: Some(format!("#{}", rgb.to_ascii_uppercase())),
                    ..Default::default()
                });
            }
            let scheme = find(element, "w14:schemeClr")?;
            let mut color = DrawingColor {
                theme: attr(scheme, "w14:val").map(str::to_string),
                ..Default::default()
            };
            for node in &scheme.children {
                if let crate::docx_xml::XmlNode::Element(transform) = node {
                    let value = attr(transform, "w14:val").and_then(|v| v.parse::<f64>().ok());
                    let transform = match (transform.name.as_str(), value) {
                        ("w14:lumMod", Some(v)) => Some(super::ThemeColorTransform::LuminanceMod(v / 100_000.0)),
                        ("w14:lumOff", Some(v)) => Some(super::ThemeColorTransform::LuminanceOffset(v / 100_000.0)),
                        ("w14:tint", Some(v)) => Some(super::ThemeColorTransform::Tint(v / 100_000.0)),
                        ("w14:shade", Some(v)) => Some(super::ThemeColorTransform::Shade(v / 100_000.0)),
                        _ => None,
                    };
                    color.transforms.extend(transform);
                }
            }
            Some(color)
        }
        let properties = find(document.root(), "w:r").and_then(|run| find(run, "w:rPr"))?;
        let mut effects = WordArtText::default();
        if let Some(fill) = find(properties, "w14:textFill") {
            if find(fill, "w14:noFill").is_some() {
                effects.no_fill = true;
            } else {
                effects.fill = color(fill);
            }
        }
        if let Some(outline) = find(properties, "w14:textOutline") {
            if find(outline, "w14:noFill").is_none() {
                let width = attr(outline, "w14:w")
                    .and_then(|w| w.parse::<f64>().ok())
                    .map(|emu| emu / 9525.0)
                    .unwrap_or(0.75);
                if let Some(color) = color(outline) {
                    effects.outline = Some((color, width.max(0.5)));
                }
            }
        }
        effects.shadow = find(properties, "w14:shadow").is_some();
        if let Some(glow) = find(properties, "w14:glow") {
            let radius = attr(glow, "w14:rad")
                .and_then(|r| r.parse::<f64>().ok())
                .map(|emu| emu / 9525.0)
                .unwrap_or(4.0);
            if let Some(color) = color(glow) {
                effects.glow = Some((color, radius));
            }
        }
        (effects != WordArtText::default()).then_some(effects)
    }
}

/// A text box whose text continues from or into another (`wps:txbx@id`,
/// `wps:linkedTxbx`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextBoxLink {
    /// The story's id, shared by every box of the chain.
    pub id: u32,
    /// The box's place in the chain; 0 holds the text.
    pub sequence: u32,
}

/// A picture's 図の書式: outline, effects, shape, colour adjustments.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PictureFormat {
    /// `pic:spPr/a:ln` (図の枠線).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<TextBoxBorder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShapeShadow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<ShapeGlow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_edge_radius_px: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<ShapeReflection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<ShapeBevel>,
    /// The picture's shape when it is not a rectangle (図形に合わせてトリミング).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<TextBoxShape>,
    /// Colour adjustments drawn over the picture's pixels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blip_effects: Vec<BlipEffect>,
    /// Word's corrections, artistic effects and background removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrections: Option<PictureCorrections>,
    /// The SVG a picture shows where it can (`asvg:svgBlip@r:embed`); the
    /// blip itself is the PNG fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svg_relationship_id: Option<String>,
}

impl PictureFormat {
    pub fn is_empty(&self) -> bool {
        self == &PictureFormat::default()
    }
}

/// A shape's extras beyond its frame, fill and line.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeExtras {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<ShapeReflection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<ShapeBevel>,
    /// A freeform's own paths (`a:custGeom`), drawn instead of the preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_geometry: Option<CustomGeometry>,
    /// The shapes a connector's ends are glued to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<ConnectorEnds>,
    /// WordArt's text transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_warp: Option<TextWarp>,
    /// The chain of linked text boxes this one belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<TextBoxLink>,
    /// WordArt's text effects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wordart: Option<WordArtText>,
}

impl ShapeExtras {
    pub fn is_empty(&self) -> bool {
        self == &ShapeExtras::default()
    }
}

/// The anchor's options and the object's name.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnchorOptions {
    /// `wp:docPr@id`: the object's id, which connectors refer to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u32>,
    /// `wp:docPr@name`, what the Selection Pane lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `wp:docPr@hidden`: hidden with the Selection Pane's eye.
    #[serde(default)]
    pub hidden: bool,
    /// `wp:anchor@locked` (アンカーを段落に固定する).
    #[serde(default)]
    pub locked: bool,
    /// `wp:anchor@allowOverlap` (重なりを許可する); `None` when not written,
    /// which Word reads as allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_overlap: Option<bool>,
    /// `wp:anchor@layoutInCell` (表のセル内に配置する).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_in_cell: Option<bool>,
}

static NO_PICTURE_FORMAT: std::sync::OnceLock<PictureFormat> = std::sync::OnceLock::new();
static NO_SHAPE_EXTRAS: std::sync::OnceLock<ShapeExtras> = std::sync::OnceLock::new();
static NO_ANCHOR_OPTIONS: std::sync::OnceLock<AnchorOptions> = std::sync::OnceLock::new();

impl RunImage {
    /// The picture's format; empty when it has none.
    pub fn format(&self) -> &PictureFormat {
        self.format
            .as_deref()
            .unwrap_or_else(|| NO_PICTURE_FORMAT.get_or_init(PictureFormat::default))
    }

    pub fn format_mut(&mut self) -> &mut PictureFormat {
        self.format.get_or_insert_with(Default::default)
    }
}

impl RunTextBox {
    /// The shape's extras; empty when it has none.
    pub fn extras(&self) -> &ShapeExtras {
        self.extras
            .as_deref()
            .unwrap_or_else(|| NO_SHAPE_EXTRAS.get_or_init(ShapeExtras::default))
    }

    pub fn extras_mut(&mut self) -> &mut ShapeExtras {
        self.extras.get_or_insert_with(Default::default)
    }
}

impl RunImageLayout {
    /// The anchor's options; defaults when none were read.
    pub fn options(&self) -> &AnchorOptions {
        self.options
            .as_deref()
            .unwrap_or_else(|| NO_ANCHOR_OPTIONS.get_or_init(AnchorOptions::default))
    }

    pub fn options_mut(&mut self) -> &mut AnchorOptions {
        self.options.get_or_insert_with(Default::default)
    }

    pub fn hidden(&self) -> bool {
        self.options().hidden
    }
}
