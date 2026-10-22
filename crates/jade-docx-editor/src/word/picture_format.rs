//! 図の形式's 調整, 図のスタイル and トリミング over any [`WordSession`]:
//! 図の変更 and 図のリセット, 修整 (brightness and contrast), 色 (saturation,
//! tone, recolouring), アート効果, 透明度, 背景の削除, the picture's outline
//! and effects (影, 反射, 光彩, ぼかし, 面取り), cropping — to a rectangle,
//! to a shape, to an aspect ratio.
//!
//! Each is written as Word writes it. What DrawingML draws from the
//! picture's own pixels (`a:lum`, `a:grayscl`, `a:duotone`, `a:biLevel`,
//! `a:alphaModFix`) stays a blip effect; what Word 2010 bakes into a copy of
//! the picture (saturation, tone, artistic effects, background removal) is
//! baked here too, with the original kept in `a14:imgLayer` so the picture
//! can be reset.

use crate::document::{ImageCrop, TextBoxShape};
use crate::docx_xml::{XmlElement, XmlNode};
use crate::drawing_xml::{
    self, child, child_mut, descendant, descendant_mut, get_attr, put_shape_property,
    remove_children, set_attr,
};
use crate::picture_effects;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, WordDocument};

use super::objects::{fragment, WordObjects};
use super::picture::picture_type;
use super::WordSession;

/// `a14:imgProps`' extension id.
const IMAGE_PROPERTIES_EXTENSION: &str = "{BEBA8EAE-BF5A-486C-A8C5-ECC9F3942E4B}";
/// `asvg:svgBlip`'s extension id.
const SVG_EXTENSION: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";

/// 図の枠線's dash styles (`a:prstDash`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dash {
    Solid,
    RoundDot,
    SquareDot,
    Dash,
    DashDot,
    LongDash,
    LongDashDot,
    LongDashDotDot,
}

impl Dash {
    pub const ALL: [Dash; 8] = [
        Dash::Solid,
        Dash::RoundDot,
        Dash::SquareDot,
        Dash::Dash,
        Dash::DashDot,
        Dash::LongDash,
        Dash::LongDashDot,
        Dash::LongDashDotDot,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            Dash::Solid => "solid",
            Dash::RoundDot => "sysDot",
            Dash::SquareDot => "sysDash",
            Dash::Dash => "dash",
            Dash::DashDot => "dashDot",
            Dash::LongDash => "lgDash",
            Dash::LongDashDot => "lgDashDot",
            Dash::LongDashDotDot => "lgDashDotDot",
        }
    }
}

/// An outline: colour (`RRGGBB`), width in points, dash.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub color: String,
    pub width_pt: f64,
    pub dash: Dash,
}

/// What 図の枠線 / 図形の枠線 changes: one aspect, the others kept.
#[derive(Clone, Debug, PartialEq)]
pub enum OutlineEdit {
    /// 枠線なし.
    None,
    Color(String),
    Width(f64),
    Dash(Dash),
    /// 矢印: the start and end marker (`a:headEnd` / `a:tailEnd` type).
    Arrows(Option<String>, Option<String>),
}

/// 図の効果 / 図形の効果: a preset of one kind, or that kind removed.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// 影: outer shadow `blur`/`distance` in points, `direction` in degrees,
    /// or none.
    Shadow(Option<ShadowPreset>),
    /// 反射: Word's three variations (tight, half, full), or none.
    Reflection(Option<ReflectionPreset>),
    /// 光彩: radius in points and colour (`RRGGBB`), or none.
    Glow(Option<(f64, String)>),
    /// ぼかし: soft edge radius in points, or none.
    SoftEdge(Option<f64>),
    /// 面取り: `a:bevelT` preset, or none.
    Bevel(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowPreset {
    pub blur_pt: f64,
    pub distance_pt: f64,
    pub direction_deg: f64,
    /// Opacity, 0.0–1.0.
    pub alpha: f64,
    /// 内側: an inner shadow.
    pub inner: bool,
}

impl ShadowPreset {
    /// Word's 影 → 外側, in its gallery's order: the offset directions
    /// around the compass, then centred.
    pub const OUTER: [ShadowPreset; 9] = [
        Self::outer(135.0),
        Self::outer(90.0),
        Self::outer(45.0),
        Self::outer(180.0),
        ShadowPreset {
            blur_pt: 5.0,
            distance_pt: 0.0,
            direction_deg: 0.0,
            alpha: 0.4,
            inner: false,
        },
        Self::outer(0.0),
        Self::outer(225.0),
        Self::outer(270.0),
        Self::outer(315.0),
    ];
    pub const INNER: [ShadowPreset; 3] = [
        ShadowPreset {
            blur_pt: 5.0,
            distance_pt: 0.0,
            direction_deg: 0.0,
            alpha: 0.5,
            inner: true,
        },
        ShadowPreset {
            blur_pt: 4.0,
            distance_pt: 3.0,
            direction_deg: 45.0,
            alpha: 0.5,
            inner: true,
        },
        ShadowPreset {
            blur_pt: 4.0,
            distance_pt: 3.0,
            direction_deg: 225.0,
            alpha: 0.5,
            inner: true,
        },
    ];

    const fn outer(direction_deg: f64) -> Self {
        ShadowPreset {
            blur_pt: 4.0,
            distance_pt: 3.0,
            direction_deg,
            alpha: 0.4,
            inner: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectionPreset {
    /// 反射 (弱): 接触.
    Tight,
    /// 反射 (中): 接触.
    Half,
    /// 反射 (強): 接触.
    Full,
}

/// 色 → 色の変更: what the picture is recoloured to.
#[derive(Clone, Debug, PartialEq)]
pub enum Recolor {
    /// 色の変更なし.
    None,
    /// グレースケール.
    Grayscale,
    /// セピア.
    Sepia,
    /// 白黒: 50%.
    BlackAndWhite(f64),
    /// One of the theme's accents, dark or light (`accent1`…).
    Accent { slot: String, light: bool },
}

/// 修整's preset grid: brightness and contrast in -0.4 … 0.4 steps of 0.2.
pub const CORRECTION_STEPS: [f64; 5] = [-0.4, -0.2, 0.0, 0.2, 0.4];
/// 色の彩度's presets.
pub const SATURATIONS: [f64; 7] = [0.0, 0.33, 0.66, 1.0, 1.33, 1.66, 2.0];
/// 色のトーン's presets, in kelvin.
pub const TEMPERATURES: [f64; 7] = [4700.0, 5300.0, 5900.0, 6500.0, 7200.0, 8800.0, 11200.0];
/// アート効果 jade draws.
pub const ARTISTIC_EFFECTS: [&str; 3] = [
    "artisticPencilGrayscale",
    "artisticLineDrawing",
    "artisticBlur",
];
/// トリミング → 縦横比, as Word lists them.
pub const ASPECT_RATIOS: [(u32, u32); 13] = [
    (1, 1),
    (2, 3),
    (3, 4),
    (3, 5),
    (4, 5),
    (3, 2),
    (4, 3),
    (5, 3),
    (5, 4),
    (16, 9),
    (16, 10),
    (9, 16),
    (10, 16),
];

/// The `a:blip` of a picture's drawing.
fn blip_mut(drawing: &mut XmlElement) -> Result<&mut XmlElement, String> {
    descendant_mut(drawing, "a:blip").ok_or_else(|| "the object is not a picture".to_string())
}

/// The order of an `a:effectLst`'s children (ECMA-376 Part 1 §20.1.8.26).
const EFFECT_ORDER: [&str; 8] = [
    "a:blur",
    "a:fillOverlay",
    "a:glow",
    "a:innerShdw",
    "a:outerShdw",
    "a:prstShdw",
    "a:reflection",
    "a:softEdge",
];

fn sort_effects(list: &mut XmlElement) {
    list.children.sort_by_key(|node| match node {
        XmlNode::Element(element) => EFFECT_ORDER
            .iter()
            .position(|name| *name == element.name)
            .unwrap_or(EFFECT_ORDER.len()),
        _ => EFFECT_ORDER.len(),
    });
}

/// Applies an effect to shape properties (`pic:spPr` or `wps:spPr`).
pub fn apply_effect(sp_pr: &mut XmlElement, effect: &Effect) -> Result<(), String> {
    if let Effect::Bevel(bevel) = effect {
        remove_children(sp_pr, &["a:scene3d", "a:sp3d"]);
        if let Some(preset) = bevel {
            put_shape_property(
                sp_pr,
                fragment(
                    r#"<a:scene3d><a:camera prst="orthographicFront"/><a:lightRig rig="threePt" dir="t"/></a:scene3d>"#,
                )?,
                &["a:scene3d"],
            );
            put_shape_property(
                sp_pr,
                fragment(&format!(
                    r#"<a:sp3d><a:bevelT w="63500" h="25400" prst="{preset}"/></a:sp3d>"#
                ))?,
                &["a:sp3d"],
            );
        }
        return Ok(());
    }
    if child(sp_pr, "a:effectLst").is_none() {
        put_shape_property(sp_pr, fragment("<a:effectLst/>")?, &["a:effectLst"]);
        // An effect list replaces an effect DAG.
        remove_children(sp_pr, &["a:effectDag"]);
    }
    let list = child_mut(sp_pr, "a:effectLst").expect("added above");
    let emu = |points: f64| (points * 12_700.0).round() as i64;
    let (names, element): (&[&str], Option<XmlElement>) = match effect {
        Effect::Shadow(shadow) => (
            &["a:outerShdw", "a:innerShdw", "a:prstShdw"],
            match shadow {
                Some(shadow) if shadow.inner => Some(fragment(&format!(
                    r#"<a:innerShdw blurRad="{}" dist="{}" dir="{}"><a:prstClr val="black"><a:alpha val="{}"/></a:prstClr></a:innerShdw>"#,
                    emu(shadow.blur_pt),
                    emu(shadow.distance_pt),
                    (shadow.direction_deg * 60_000.0).round() as i64,
                    (shadow.alpha * 100_000.0).round() as i64,
                ))?),
                Some(shadow) => Some(fragment(&format!(
                    r#"<a:outerShdw blurRad="{}" dist="{}" dir="{}" algn="ctr" rotWithShape="0"><a:prstClr val="black"><a:alpha val="{}"/></a:prstClr></a:outerShdw>"#,
                    emu(shadow.blur_pt),
                    emu(shadow.distance_pt),
                    (shadow.direction_deg * 60_000.0).round() as i64,
                    (shadow.alpha * 100_000.0).round() as i64,
                ))?),
                None => None,
            },
        ),
        Effect::Reflection(reflection) => (
            &["a:reflection"],
            reflection
                .map(|preset| {
                    let (end_alpha, end_position) = match preset {
                        ReflectionPreset::Tight => (300, 35_000),
                        ReflectionPreset::Half => (300, 55_000),
                        ReflectionPreset::Full => (300, 90_000),
                    };
                    fragment(&format!(
                        r#"<a:reflection blurRad="6350" stA="50000" endA="{end_alpha}" endPos="{end_position}" dist="0" dir="5400000" sy="-100000" algn="bl" rotWithShape="0"/>"#
                    ))
                })
                .transpose()?,
        ),
        Effect::Glow(glow) => (
            &["a:glow"],
            glow.as_ref()
                .map(|(radius, color)| {
                    fragment(&format!(
                        r#"<a:glow rad="{}"><a:srgbClr val="{}"><a:alpha val="40000"/></a:srgbClr></a:glow>"#,
                        emu(*radius),
                        color.trim_start_matches('#').to_ascii_uppercase()
                    ))
                })
                .transpose()?,
        ),
        Effect::SoftEdge(radius) => (
            &["a:softEdge"],
            radius
                .map(|radius| fragment(&format!(r#"<a:softEdge rad="{}"/>"#, emu(radius))))
                .transpose()?,
        ),
        Effect::Bevel(_) => unreachable!(),
    };
    remove_children(list, names);
    if let Some(element) = element {
        list.children.push(XmlNode::Element(element));
    }
    sort_effects(list);
    Ok(())
}

/// Applies an outline edit to shape properties.
pub fn apply_outline(sp_pr: &mut XmlElement, edit: &OutlineEdit) -> Result<(), String> {
    if child(sp_pr, "a:ln").is_none() {
        put_shape_property(sp_pr, fragment("<a:ln/>")?, &["a:ln"]);
    }
    let line = child_mut(sp_pr, "a:ln").expect("added above");
    const FILLS: [&str; 4] = ["a:noFill", "a:solidFill", "a:gradFill", "a:pattFill"];
    let has_visible_fill =
        child(line, "a:solidFill").is_some() || child(line, "a:gradFill").is_some();
    let solid = |color: &str| {
        fragment(&format!(
            r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
            color.trim_start_matches('#').to_ascii_uppercase()
        ))
    };
    match edit {
        OutlineEdit::None => {
            remove_children(line, &FILLS);
            line.children
                .insert(0, XmlNode::Element(fragment("<a:noFill/>")?));
        }
        OutlineEdit::Color(color) => {
            remove_children(line, &FILLS);
            line.children.insert(0, XmlNode::Element(solid(color)?));
            if get_attr(line, "w").is_none() {
                set_attr(line, "w", Some("9525".into()));
            }
        }
        OutlineEdit::Width(points) => {
            set_attr(
                line,
                "w",
                Some(((points * 12_700.0).round() as i64).to_string()),
            );
            if !has_visible_fill {
                remove_children(line, &FILLS);
                line.children.insert(0, XmlNode::Element(solid("000000")?));
            }
        }
        OutlineEdit::Dash(dash) => {
            remove_children(line, &["a:prstDash", "a:custDash"]);
            let at = drawing_xml::child_index(
                line,
                &[
                    "a:round",
                    "a:bevel",
                    "a:miter",
                    "a:headEnd",
                    "a:tailEnd",
                    "a:extLst",
                ],
            )
            .unwrap_or(line.children.len());
            line.children.insert(
                at,
                XmlNode::Element(fragment(&format!(
                    r#"<a:prstDash val="{}"/>"#,
                    dash.ooxml()
                ))?),
            );
            if !has_visible_fill {
                remove_children(line, &FILLS);
                line.children.insert(0, XmlNode::Element(solid("000000")?));
            }
        }
        OutlineEdit::Arrows(head, tail) => {
            remove_children(line, &["a:headEnd", "a:tailEnd"]);
            let at = drawing_xml::child_index(line, &["a:extLst"]).unwrap_or(line.children.len());
            let mut ends = Vec::new();
            if let Some(head) = head {
                ends.push(fragment(&format!(r#"<a:headEnd type="{head}"/>"#))?);
            }
            if let Some(tail) = tail {
                ends.push(fragment(&format!(r#"<a:tailEnd type="{tail}"/>"#))?);
            }
            for (offset, end) in ends.into_iter().enumerate() {
                line.children.insert(at + offset, XmlNode::Element(end));
            }
        }
    }
    Ok(())
}

/// The blip effects a recolouring writes.
fn recolor_elements(recolor: &Recolor) -> Result<Vec<XmlElement>, String> {
    Ok(match recolor {
        Recolor::None => Vec::new(),
        Recolor::Grayscale => vec![fragment("<a:grayscl/>")?],
        Recolor::Sepia => vec![fragment(
            r#"<a:duotone><a:prstClr val="black"/><a:srgbClr val="D9C3A5"><a:tint val="50000"/></a:srgbClr></a:duotone>"#,
        )?],
        Recolor::BlackAndWhite(threshold) => vec![fragment(&format!(
            r#"<a:biLevel thresh="{}"/>"#,
            (threshold * 100_000.0).round() as i64
        ))?],
        Recolor::Accent { slot, light } => vec![fragment(&if *light {
            format!(
                r#"<a:duotone><a:prstClr val="black"/><a:schemeClr val="{slot}"><a:tint val="45000"/></a:schemeClr></a:duotone>"#
            )
        } else {
            format!(
                r#"<a:duotone><a:schemeClr val="{slot}"><a:shade val="45000"/></a:schemeClr><a:prstClr val="white"/></a:duotone>"#
            )
        })?],
    })
}

const RECOLOR_NAMES: [&str; 3] = ["a:grayscl", "a:duotone", "a:biLevel"];

/// What Word bakes into the picture: kept in `a14:imgLayer` with the
/// original.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedCorrections {
    pub saturation: Option<f64>,
    pub temperature: Option<f64>,
    pub artistic: Option<String>,
    pub background_removed: bool,
}

/// The relationship of the picture without Word's baked corrections.
fn original_relationship(blip: &XmlElement) -> Option<String> {
    descendant(blip, "a14:imgLayer")
        .and_then(|layer| get_attr(layer, "r:embed"))
        .map(str::to_string)
        .or_else(|| get_attr(blip, "r:embed").map(str::to_string))
}

/// Reads what Word baked into the picture from `a14:imgProps`.
fn baked_corrections(blip: &XmlElement) -> BakedCorrections {
    let mut baked = BakedCorrections::default();
    let Some(layer) = descendant(blip, "a14:imgLayer") else {
        return baked;
    };
    for effect in drawing_xml::elements(layer).filter(|e| e.name == "a14:imgEffect") {
        for element in drawing_xml::elements(effect) {
            let fraction = |name: &str| {
                get_attr(element, name)
                    .and_then(|value| value.parse::<f64>().ok())
                    .map(|value| value / 100_000.0)
            };
            match element.name.as_str() {
                "a14:saturation" => baked.saturation = fraction("sat"),
                "a14:colorTemperature" => {
                    baked.temperature = get_attr(element, "colorTemp").and_then(|v| v.parse().ok())
                }
                "a14:backgroundRemoval" => baked.background_removed = true,
                name if name.starts_with("a14:artistic") => {
                    baked.artistic = Some(name[4..].to_string())
                }
                _ => {}
            }
        }
    }
    baked
}

/// Bakes `baked` into a copy of the original picture, as Word does, and
/// points the blip at it; with nothing to bake, back to the original.
fn bake(
    document: &mut WordDocument,
    blip: &mut XmlElement,
    baked: &BakedCorrections,
) -> Result<(), String> {
    let original = original_relationship(blip).ok_or("the picture has no image")?;
    // Out with the old layer either way.
    if let Some(extensions) = child_mut(blip, "a:extLst") {
        extensions.children.retain(|node| {
            !matches!(node, XmlNode::Element(element)
                if element.name == "a:ext" && get_attr(element, "uri") == Some(IMAGE_PROPERTIES_EXTENSION))
        });
    }
    if baked == &BakedCorrections::default() {
        set_attr(blip, "r:embed", Some(original));
        return Ok(());
    }
    let bytes = document
        .object_media(&original)
        .ok_or("the picture's image is missing")?;
    let mut pixels =
        picture_effects::decode(&bytes).ok_or("only PNG and JPEG pictures can be adjusted")?;
    let mut effects = Vec::new();
    if baked.background_removed {
        picture_effects::remove_background(&mut pixels, [0.0, 0.0, 1.0, 1.0]);
        effects.push(r#"<a14:imgEffect><a14:backgroundRemoval t="0" b="100000" l="0" r="100000"/></a14:imgEffect>"#.to_string());
    }
    if let Some(effect) = baked.artistic.as_deref() {
        picture_effects::artistic(&mut pixels, effect);
        effects.push(format!("<a14:imgEffect><a14:{effect}/></a14:imgEffect>"));
    }
    if let Some(saturation) = baked.saturation {
        picture_effects::saturate(&mut pixels, saturation);
        effects.push(format!(
            r#"<a14:imgEffect><a14:saturation sat="{}"/></a14:imgEffect>"#,
            (saturation * 100_000.0).round() as i64
        ));
    }
    if let Some(temperature) = baked.temperature {
        picture_effects::warm(&mut pixels, temperature);
        effects.push(format!(
            r#"<a14:imgEffect><a14:colorTemperature colorTemp="{}"/></a14:imgEffect>"#,
            temperature.round() as i64
        ));
    }
    let png = picture_effects::encode_png(&pixels).ok_or("the picture cannot be written")?;
    let baked_relationship = document.add_object_media(&png, "png", "image/png")?;
    set_attr(blip, "r:embed", Some(baked_relationship));
    if child(blip, "a:extLst").is_none() {
        blip.children
            .push(XmlNode::Element(fragment("<a:extLst/>")?));
    }
    let extension = fragment(&format!(
        r#"<a:ext uri="{IMAGE_PROPERTIES_EXTENSION}"><a14:imgProps xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><a14:imgLayer r:embed="{original}">{}</a14:imgLayer></a14:imgProps></a:ext>"#,
        effects.concat()
    ))?;
    let extensions = child_mut(blip, "a:extLst").expect("added above");
    extensions.children.insert(0, XmlNode::Element(extension));
    Ok(())
}

/// Puts blip effects into `blip` in DrawingML's order, after removing those
/// named in `replaces`.
fn put_blip_effects(blip: &mut XmlElement, replaces: &[&str], effects: Vec<XmlElement>) {
    remove_children(blip, replaces);
    // Effects come before `a:extLst`.
    let at = drawing_xml::child_index(blip, &["a:extLst"]).unwrap_or(blip.children.len());
    for (offset, effect) in effects.into_iter().enumerate() {
        blip.children.insert(at + offset, XmlNode::Element(effect));
    }
}

fn require_picture(target: FloatingObjectTarget) -> Result<(), String> {
    if target.kind == FloatingObjectKind::Image {
        Ok(())
    } else {
        Err("only a picture has picture tools".into())
    }
}

/// The picture tools, for every session.
pub trait WordPictureFormat: WordSession + WordObjects {
    /// 修整: brightness and contrast, each -1.0–1.0 (`a:lum`).
    fn set_picture_corrections(
        &mut self,
        target: FloatingObjectTarget,
        brightness: f64,
        contrast: f64,
    ) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, "picture_corrections", move |_, drawing| {
            let blip = blip_mut(drawing)?;
            let lum = (brightness.abs() > 1e-6 || contrast.abs() > 1e-6)
                .then(|| {
                    fragment(&format!(
                        r#"<a:lum bright="{}" contrast="{}"/>"#,
                        (brightness * 100_000.0).round() as i64,
                        (contrast * 100_000.0).round() as i64
                    ))
                })
                .transpose()?;
            put_blip_effects(blip, &["a:lum"], lum.into_iter().collect());
            Ok(())
        })
    }

    /// 色 → 色の変更.
    fn recolor_picture(
        &mut self,
        target: FloatingObjectTarget,
        recolor: Recolor,
    ) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, "picture_recolor", move |_, drawing| {
            let blip = blip_mut(drawing)?;
            put_blip_effects(blip, &RECOLOR_NAMES, recolor_elements(&recolor)?);
            Ok(())
        })
    }

    /// 透明度: 0.0 opaque to 1.0 invisible (`a:alphaModFix`).
    fn set_picture_transparency(
        &mut self,
        target: FloatingObjectTarget,
        transparency: f64,
    ) -> Result<(), String> {
        require_picture(target)?;
        let amount = (1.0 - transparency.clamp(0.0, 1.0)) * 100_000.0;
        self.edit_object(target, "picture_transparency", move |_, drawing| {
            let blip = blip_mut(drawing)?;
            let effect = (amount < 100_000.0)
                .then(|| {
                    fragment(&format!(
                        r#"<a:alphaModFix amt="{}"/>"#,
                        amount.round() as i64
                    ))
                })
                .transpose()?;
            put_blip_effects(blip, &["a:alphaModFix"], effect.into_iter().collect());
            Ok(())
        })
    }

    /// 色 → 透明色を指定: pixels of `color` (`RRGGBB`) become transparent.
    fn set_transparent_color(
        &mut self,
        target: FloatingObjectTarget,
        color: &str,
    ) -> Result<(), String> {
        require_picture(target)?;
        let color = color.trim_start_matches('#').to_ascii_uppercase();
        self.edit_object(target, "picture_transparent_color", move |_, drawing| {
            let blip = blip_mut(drawing)?;
            let effect = fragment(&format!(
                r#"<a:clrChange><a:clrFrom><a:srgbClr val="{color}"/></a:clrFrom><a:clrTo><a:srgbClr val="{color}"><a:alpha val="0"/></a:srgbClr></a:clrTo></a:clrChange>"#
            ))?;
            put_blip_effects(blip, &["a:clrChange"], vec![effect]);
            Ok(())
        })
    }

    /// 色 → 色の彩度 / 色のトーン, アート効果 and 背景の削除, which Word bakes
    /// into a copy of the picture. `edit` changes what is baked.
    fn bake_picture(
        &mut self,
        target: FloatingObjectTarget,
        name: &'static str,
        edit: impl Fn(&mut BakedCorrections) + 'static,
    ) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, name, move |document, drawing| {
            let blip = blip_mut(drawing)?;
            let mut baked = baked_corrections(blip);
            edit(&mut baked);
            bake(document, blip, &baked)
        })
    }

    fn set_picture_saturation(
        &mut self,
        target: FloatingObjectTarget,
        saturation: f64,
    ) -> Result<(), String> {
        self.bake_picture(target, "picture_saturation", move |baked| {
            baked.saturation = ((saturation - 1.0).abs() > 1e-6).then_some(saturation)
        })
    }

    fn set_picture_temperature(
        &mut self,
        target: FloatingObjectTarget,
        kelvin: f64,
    ) -> Result<(), String> {
        self.bake_picture(target, "picture_temperature", move |baked| {
            baked.temperature = ((kelvin - 6500.0).abs() > 1.0).then_some(kelvin)
        })
    }

    fn set_artistic_effect(
        &mut self,
        target: FloatingObjectTarget,
        effect: Option<&'static str>,
    ) -> Result<(), String> {
        self.bake_picture(target, "picture_artistic_effect", move |baked| {
            baked.artistic = effect.map(str::to_string)
        })
    }

    /// 背景の削除 (and 変更を破棄 with `false`).
    fn remove_picture_background(
        &mut self,
        target: FloatingObjectTarget,
        remove: bool,
    ) -> Result<(), String> {
        self.bake_picture(target, "picture_background", move |baked| {
            baked.background_removed = remove
        })
    }

    /// 図の枠線.
    fn set_picture_outline(
        &mut self,
        target: FloatingObjectTarget,
        edit: OutlineEdit,
    ) -> Result<(), String> {
        self.edit_object(target, "object_outline", move |_, drawing| {
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object has no outline")?;
            apply_outline(sp_pr, &edit)
        })
    }

    /// 図の効果 / 図形の効果.
    fn set_object_effect(
        &mut self,
        target: FloatingObjectTarget,
        effect: Effect,
    ) -> Result<(), String> {
        self.edit_object(target, "object_effect", move |_, drawing| {
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object has no effects")?;
            apply_effect(sp_pr, &effect)
        })
    }

    /// トリミング: `crop` as fractions of the picture's edges.
    fn crop_picture(
        &mut self,
        target: FloatingObjectTarget,
        crop: Option<ImageCrop>,
    ) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, "picture_crop", move |_, drawing| {
            let blip_fill =
                descendant_mut(drawing, "pic:blipFill").ok_or("the object is not a picture")?;
            remove_children(blip_fill, &["a:srcRect"]);
            if let Some(crop) = crop {
                let value = |fraction: f64| (fraction * 100_000.0).round() as i64;
                let at = drawing_xml::child_index(blip_fill, &["a:tile", "a:stretch"])
                    .unwrap_or(blip_fill.children.len());
                blip_fill.children.insert(
                    at,
                    XmlNode::Element(fragment(&format!(
                        r#"<a:srcRect l="{}" t="{}" r="{}" b="{}"/>"#,
                        value(crop.left),
                        value(crop.top),
                        value(crop.right),
                        value(crop.bottom)
                    ))?),
                );
            }
            Ok(())
        })
    }

    /// トリミング with the crop handles: the picture's new box and what is
    /// cut from each edge, as one step. `rect` is the visible part's size in
    /// CSS px; its position is the caller's move.
    fn crop_picture_to(
        &mut self,
        target: FloatingObjectTarget,
        crop: ImageCrop,
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        require_picture(target)?;
        let crop = (crop.left.abs() + crop.right.abs() + crop.top.abs() + crop.bottom.abs() > 1e-6)
            .then_some(crop);
        let (width, height) = (width.max(1.0), height.max(1.0));
        self.edit_object(target, "picture_crop", move |_, drawing| {
            let blip_fill =
                descendant_mut(drawing, "pic:blipFill").ok_or("the object is not a picture")?;
            remove_children(blip_fill, &["a:srcRect"]);
            if let Some(crop) = crop {
                let value = |fraction: f64| (fraction * 100_000.0).round() as i64;
                let at = drawing_xml::child_index(blip_fill, &["a:tile", "a:stretch"])
                    .unwrap_or(blip_fill.children.len());
                blip_fill.children.insert(
                    at,
                    XmlNode::Element(fragment(&format!(
                        r#"<a:srcRect l="{}" t="{}" r="{}" b="{}"/>"#,
                        value(crop.left),
                        value(crop.top),
                        value(crop.right),
                        value(crop.bottom)
                    ))?),
                );
            }
            resize_drawing(drawing, width, height)
        })
    }

    /// A crop handle's drag: the new crop, and the visible part's box on
    /// the page — moved when a floating picture's left or top edge was cut,
    /// so the picture itself stays where it is.
    fn crop_picture_in_place(
        &mut self,
        target: FloatingObjectTarget,
        crop: ImageCrop,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> Result<(), String> {
        use crate::commands::transform_object::TransformObjectCommand;
        use crate::commands::Command;
        use crate::word_document::FloatingObjectTransform;
        require_picture(target)?;
        let format = self.object_format(target).ok_or("the picture is gone")?;
        let crop = (crop.left.abs() + crop.right.abs() + crop.top.abs() + crop.bottom.abs() > 1e-6)
            .then_some(crop);
        let (width, height) = (width.max(1.0), height.max(1.0));
        let mut commands: Vec<Box<dyn Command>> = vec![Box::new(
            crate::commands::edit_drawing::edit_drawing_command(
                "picture_crop",
                target,
                move |_, drawing| {
                    let blip_fill = descendant_mut(drawing, "pic:blipFill")
                        .ok_or("the object is not a picture")?;
                    remove_children(blip_fill, &["a:srcRect"]);
                    if let Some(crop) = crop {
                        let value = |fraction: f64| (fraction * 100_000.0).round() as i64;
                        let at = drawing_xml::child_index(blip_fill, &["a:tile", "a:stretch"])
                            .unwrap_or(blip_fill.children.len());
                        blip_fill.children.insert(
                            at,
                            XmlNode::Element(fragment(&format!(
                                r#"<a:srcRect l="{}" t="{}" r="{}" b="{}"/>"#,
                                value(crop.left),
                                value(crop.top),
                                value(crop.right),
                                value(crop.bottom)
                            ))?),
                        );
                    }
                    resize_drawing(drawing, width, height)
                },
            ),
        )];
        if format.wrap != super::picture::Wrap::Inline {
            commands.push(Box::new(TransformObjectCommand::new(
                target,
                FloatingObjectTransform {
                    x,
                    y,
                    width,
                    height,
                    rotation_deg: format.rotation,
                },
            )));
        }
        self.execute_command(
            Box::new(super::objects::Sequence::new("picture_crop", commands)),
            true,
        )
    }

    /// トリミング → 図形に合わせてトリミング: the picture's geometry.
    fn crop_picture_to_shape(
        &mut self,
        target: FloatingObjectTarget,
        shape: TextBoxShape,
    ) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, "picture_crop_to_shape", move |_, drawing| {
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a picture")?;
            let geometry = fragment(&format!(
                r#"<a:prstGeom prst="{}"><a:avLst/></a:prstGeom>"#,
                shape.preset_name()
            ))?;
            put_shape_property(sp_pr, geometry, &["a:custGeom", "a:prstGeom"]);
            Ok(())
        })
    }

    /// トリミング → 縦横比: the largest centred part of the picture with
    /// `width`:`height` proportions, keeping the picture's width.
    fn crop_picture_to_aspect(
        &mut self,
        target: FloatingObjectTarget,
        (ratio_width, ratio_height): (u32, u32),
    ) -> Result<(), String> {
        require_picture(target)?;
        let format = self.object_format(target).ok_or("the picture is gone")?;
        let image = super::objects::object_run(self.word_document(), target)
            .and_then(|run| run.image.clone())
            .ok_or("the picture is gone")?;
        // The picture's own proportions, uncropped.
        let crop = image.crop.unwrap_or_default();
        let visible_x = (1.0 - crop.left - crop.right).max(0.01);
        let visible_y = (1.0 - crop.top - crop.bottom).max(0.01);
        let full_width = format.width / visible_x;
        let full_height = format.height / visible_y;
        let ratio = f64::from(ratio_width) / f64::from(ratio_height.max(1));
        let (keep_x, keep_y) = if full_width / full_height > ratio {
            (full_height * ratio / full_width, 1.0)
        } else {
            (1.0, full_width / ratio / full_height)
        };
        let crop = ImageCrop {
            left: (1.0 - keep_x) / 2.0,
            right: (1.0 - keep_x) / 2.0,
            top: (1.0 - keep_y) / 2.0,
            bottom: (1.0 - keep_y) / 2.0,
        };
        let width = format.width;
        let height = width / ratio;
        self.crop_picture_to(target, crop, width, height)
    }

    /// 図のリセット: the picture's adjustments, effects, outline, shape and
    /// crop removed; with `size`, 図とサイズのリセット also gives it back its
    /// own size (96 pixels an inch).
    fn reset_picture(&mut self, target: FloatingObjectTarget, size: bool) -> Result<(), String> {
        require_picture(target)?;
        self.edit_object(target, "picture_reset", move |document, drawing| {
            let blip = blip_mut(drawing)?;
            bake(document, blip, &BakedCorrections::default())?;
            remove_children(
                blip,
                &[
                    "a:lum",
                    "a:grayscl",
                    "a:duotone",
                    "a:biLevel",
                    "a:alphaModFix",
                    "a:clrChange",
                    "a:clrRepl",
                    "a:tint",
                    "a:hsl",
                    "a:alphaBiLevel",
                    "a:alphaCeiling",
                    "a:alphaFloor",
                    "a:alphaInv",
                    "a:alphaMod",
                    "a:alphaRepl",
                    "a:blur",
                    "a:fillOverlay",
                ],
            );
            let relationship = get_attr(blip, "r:embed").map(str::to_string);
            if let Some(blip_fill) = descendant_mut(drawing, "pic:blipFill") {
                remove_children(blip_fill, &["a:srcRect"]);
            }
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a picture")?;
            remove_children(
                sp_pr,
                &[
                    "a:ln",
                    "a:effectLst",
                    "a:effectDag",
                    "a:scene3d",
                    "a:sp3d",
                    "a:custGeom",
                ],
            );
            put_shape_property(
                sp_pr,
                fragment(r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#)?,
                &["a:custGeom", "a:prstGeom"],
            );
            if let Some(xfrm) = child_mut(sp_pr, "a:xfrm") {
                for name in ["rot", "flipH", "flipV"] {
                    set_attr(xfrm, name, None);
                }
            }
            if size {
                let bytes = relationship
                    .as_deref()
                    .and_then(|id| document.object_media(id))
                    .ok_or("the picture's image is missing")?;
                let (width, height) =
                    super::picture::picture_size(&bytes).ok_or("the picture's size is unknown")?;
                resize_drawing(drawing, f64::from(width), f64::from(height))?;
            }
            Ok(())
        })
    }

    /// 図の変更: another picture in this one's place, keeping its size,
    /// position, wrap, outline and effects.
    fn replace_picture(
        &mut self,
        target: FloatingObjectTarget,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        require_picture(target)?;
        let picture = super::picture::prepare_picture(bytes)?;
        self.edit_object(target, "picture_replace", move |document, drawing| {
            let relationship =
                document.add_object_media(&picture.bitmap, picture.extension, picture.mime)?;
            let svg = picture
                .svg
                .as_ref()
                .map(|svg| document.add_object_media(svg, "svg", "image/svg+xml"))
                .transpose()?;
            let blip = blip_mut(drawing)?;
            // Word's corrections were of the old picture.
            bake(document, blip, &BakedCorrections::default())?;
            set_attr(blip, "r:embed", Some(relationship));
            set_svg(blip, svg.as_deref())?;
            Ok(())
        })
    }
}

impl<T: WordSession + ?Sized> WordPictureFormat for T {}

/// Points an `a:blip`'s SVG extension at `relationship`, or removes it.
pub fn set_svg(blip: &mut XmlElement, relationship: Option<&str>) -> Result<(), String> {
    if let Some(extensions) = child_mut(blip, "a:extLst") {
        extensions.children.retain(|node| {
            !matches!(node, XmlNode::Element(element)
                if element.name == "a:ext" && get_attr(element, "uri") == Some(SVG_EXTENSION))
        });
    }
    let Some(relationship) = relationship else {
        return Ok(());
    };
    if child(blip, "a:extLst").is_none() {
        blip.children
            .push(XmlNode::Element(fragment("<a:extLst/>")?));
    }
    let extension = fragment(&format!(
        r#"<a:ext uri="{SVG_EXTENSION}"><asvg:svgBlip xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" r:embed="{relationship}"/></a:ext>"#
    ))?;
    child_mut(blip, "a:extLst")
        .expect("added above")
        .children
        .push(XmlNode::Element(extension));
    Ok(())
}

/// Sets a drawing's extent and its shape's `a:ext` to `width` × `height` CSS
/// px.
pub fn resize_drawing(drawing: &mut XmlElement, width: f64, height: f64) -> Result<(), String> {
    let (cx, cy) = (
        drawing_xml::px_to_emu(width).max(1).to_string(),
        drawing_xml::px_to_emu(height).max(1).to_string(),
    );
    let frame = super::objects::frame_mut(drawing)?;
    if let Some(extent) = child_mut(frame, "wp:extent") {
        set_attr(extent, "cx", Some(cx.clone()));
        set_attr(extent, "cy", Some(cy.clone()));
    }
    if let Some(sp_pr) = drawing_xml::shape_properties_mut(drawing) {
        if let Some(ext) = child_mut(sp_pr, "a:xfrm").and_then(|xfrm| child_mut(xfrm, "a:ext")) {
            set_attr(ext, "cx", Some(cx));
            set_attr(ext, "cy", Some(cy));
        }
    }
    Ok(())
}

/// Whether `bytes` are a picture the picture tools can take.
pub fn is_picture(bytes: &[u8]) -> bool {
    picture_type(bytes).is_some()
}
