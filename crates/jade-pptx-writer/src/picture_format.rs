//! PowerPoint for Mac's 図の形式 tab on a picture: 修整 (brightness,
//! contrast, sharpness), 色 (saturation, tone, recolour, 透明色を指定),
//! 透明度, アート効果, 背景の削除, 図のスタイル, 図の枠線, 図の効果,
//! 図形に合わせてトリミング, 縦横比を固定, 図のリセット and 図とサイズの
//! リセット, 図の圧縮, plus SVG pictures (挿入 › 図, アイコン), their
//! グラフィックの塗りつぶし and 図形に変換.
//!
//! Effects are written the way PowerPoint writes them: the colour
//! transforms on `a:blip`, which readers apply when drawing, and the
//! Office 2010 image effects in its `a14:imgProps` extension, which are
//! baked into the picture the blip names while `a14:imgLayer` keeps the
//! original, so any edit can be redone from it or reset.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_reader::Transform;
use jade_smartart::xml::{self as dom, Element, Node as DomNode};
use serde::{Deserialize, Serialize};

use crate::smartart::{add_relationship_named, drop_if_unreferenced, remove_relationship};
use crate::xml::picture_ranges::{all_pictures, picture_ranges};
use crate::{Result, WriteError};

const A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
const IMG_PROPS: &str = "{BEBA8EAE-BF5A-486C-A8C5-ECC9F3942E4B}";
const SVG_EXT: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";
const ASVG: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";
const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const EMU_PER_PX: i64 = 9525;

/// 色の変更's presets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Recolor {
    /// 色変更なし.
    None,
    /// グレースケール.
    Grayscale,
    /// セピア.
    Sepia,
    /// ウォッシュアウト.
    Washout,
    /// 白黒: 25, 50 or 75 % threshold.
    BlackWhite(u32),
    /// A theme colour's dark variant (`accent1` …).
    Dark(String),
    /// A theme colour's light variant.
    Light(String),
}

/// 図の効果, one kind at a time; `None` / `false` takes it away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PictureEffect {
    /// 影 › 外側 › オフセット (斜め右下).
    Shadow(bool),
    /// 反射 › 反射 (弱): 接触.
    Reflection(bool),
    /// 光彩: the colour (hex) and size in points.
    Glow(Option<(String, f64)>),
    /// ぼかし: the radius in points.
    SoftEdge(Option<f64>),
    /// 面取り › 丸.
    Bevel(bool),
}

/// One 図の形式 command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PictureEdit {
    /// 修整 › 明るさ/コントラスト, −1…1 each; both 0 takes it away.
    Corrections { brightness: f64, contrast: f64 },
    /// 修整 › シャープネス, −1 (soft) … 1 (sharp).
    Sharpness(f64),
    /// 色 › 彩度: 0 grey, 1 unchanged, up to 4.
    Saturation(f64),
    /// 色 › トーン, kelvin; 6500 unchanged.
    Temperature(f64),
    /// 色 › 色の変更.
    Recolor(Recolor),
    /// 透明度, 0…1.
    Transparency(f64),
    /// 透明色を指定: the colour (hex) made clear, or none.
    TransparentColor(Option<String>),
    /// アート効果 by its element name (`artisticBlur` …), or none.
    Artistic(Option<String>),
    /// 背景の削除: the subject's rectangle (left, top, right, bottom
    /// fractions), or none to keep everything.
    RemoveBackground(Option<[f64; 4]>),
    /// 図の枠線: colour (hex) or `None` for 枠線なし, and its weight.
    Border {
        color: Option<String>,
        width_pt: Option<f64>,
    },
    /// 図の効果.
    Effect(PictureEffect),
    /// 図のスタイル by its index in [`PICTURE_STYLES`].
    Style(usize),
    /// 図形に合わせてトリミング: a preset geometry.
    CropToShape(String),
    /// 縦横比を固定.
    LockAspect(bool),
    /// 図のリセット: every format change gone, size and crop kept.
    Reset,
    /// 図とサイズのリセット: also uncropped at the picture's own size.
    ResetWithSize,
    /// グラフィックの塗りつぶし on an SVG picture: every filled path in
    /// one colour (hex).
    GraphicFill(String),
}

/// A 図のスタイル: its Japanese and English names.
pub struct PictureStyle {
    pub ja: &'static str,
    pub en: &'static str,
    geometry: &'static str,
    /// Border width (EMU), colour (hex) and compound.
    line: Option<(i64, &'static str, &'static str)>,
    shadow: Option<(i64, i64, &'static str)>,
    reflection: bool,
    soft_edge: Option<i64>,
    bevel: bool,
}

impl PictureStyle {
    /// The preset geometry the style crops to.
    pub fn geometry_name(&self) -> &'static str {
        self.geometry
    }
}

/// The 図のスタイル gallery.
pub const PICTURE_STYLES: &[PictureStyle] = &[
    PictureStyle {
        ja: "シンプルな枠、白",
        en: "Simple Frame, White",
        geometry: "rect",
        line: Some((88_900, "FFFFFF", "sng")),
        shadow: Some((55_000, 18_000, "tl")),
        reflection: false,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "太枠、黒",
        en: "Thick Matte, Black",
        geometry: "rect",
        line: Some((190_500, "000000", "sng")),
        shadow: None,
        reflection: false,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "二重枠、黒",
        en: "Double Frame, Black",
        geometry: "rect",
        line: Some((57_150, "000000", "dbl")),
        shadow: None,
        reflection: false,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "影付き四角形",
        en: "Drop Shadow Rectangle",
        geometry: "rect",
        line: None,
        shadow: Some((76_200, 38_100, "tl")),
        reflection: false,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "角丸四角形、反射付き",
        en: "Reflected Rounded Rectangle",
        geometry: "roundRect",
        line: None,
        shadow: None,
        reflection: true,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "ぼかし、四角形",
        en: "Soft Edge Rectangle",
        geometry: "rect",
        line: None,
        shadow: None,
        reflection: false,
        soft_edge: Some(112_500),
        bevel: false,
    },
    PictureStyle {
        ja: "楕円、ぼかし",
        en: "Soft Edge Oval",
        geometry: "ellipse",
        line: None,
        shadow: None,
        reflection: false,
        soft_edge: Some(112_500),
        bevel: false,
    },
    PictureStyle {
        ja: "中央の影付き四角形",
        en: "Center Shadow Rectangle",
        geometry: "rect",
        line: None,
        shadow: Some((228_600, 0, "ctr")),
        reflection: false,
        soft_edge: None,
        bevel: false,
    },
    PictureStyle {
        ja: "面取り、楕円、黒",
        en: "Bevel Oval, Black",
        geometry: "ellipse",
        line: Some((38_100, "000000", "sng")),
        shadow: None,
        reflection: false,
        soft_edge: None,
        bevel: true,
    },
    PictureStyle {
        ja: "四角形、反射付き",
        en: "Reflected Bevel, White",
        geometry: "rect",
        line: Some((88_900, "FFFFFF", "sng")),
        shadow: None,
        reflection: true,
        soft_edge: None,
        bevel: true,
    },
];

/// What the 図の形式 tab shows for a picture.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PictureFormatInfo {
    pub lock_aspect: bool,
    pub geometry: Option<String>,
    /// 透明度, 0…1.
    pub transparency: f64,
    /// The picture's own size in px.
    pub pixels: Option<(u32, u32)>,
    pub svg: bool,
}

fn prefix(name: &str) -> &str {
    name.rsplit_once(':').map_or("", |(prefix, _)| prefix)
}

fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

/// Inserts `element` among `parent`'s children in schema `order`, after
/// the last child that comes before it.
fn insert_ordered(parent: &mut Element, element: Element, order: &[&str]) {
    let rank = |name: &str| order.iter().position(|n| *n == name).unwrap_or(order.len());
    let mine = rank(element.local());
    let at = parent
        .children
        .iter()
        .rposition(|node| matches!(node, DomNode::Element(e) if rank(e.local()) <= mine))
        .map_or_else(
            || {
                // Before the first later element, or at the end.
                parent
                    .children
                    .iter()
                    .position(|node| matches!(node, DomNode::Element(e) if rank(e.local()) > mine))
                    .unwrap_or(parent.children.len())
            },
            |i| i + 1,
        );
    parent.children.insert(at, DomNode::Element(element));
}

const SP_PR_ORDER: &[&str] = &[
    "xfrm",
    "custGeom",
    "prstGeom",
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
    "ln",
    "effectLst",
    "effectDag",
    "scene3d",
    "sp3d",
    "extLst",
];
const EFFECT_ORDER: &[&str] = &[
    "blur",
    "fillOverlay",
    "glow",
    "innerShdw",
    "outerShdw",
    "prstShdw",
    "reflection",
    "softEdge",
];
const BLIP_COLOR_EFFECTS: &[&str] = &[
    "alphaBiLevel",
    "alphaCeiling",
    "alphaFloor",
    "alphaInv",
    "alphaMod",
    "alphaModFix",
    "alphaRepl",
    "biLevel",
    "blur",
    "clrChange",
    "clrRepl",
    "duotone",
    "fillOverlay",
    "grayscl",
    "hsl",
    "lum",
    "tint",
];

fn percent(value: f64) -> String {
    ((value * 100_000.0).round() as i64).to_string()
}

fn srgb(a: &str, hex: &str) -> Element {
    Element::new(qualified(a, "srgbClr"))
        .with_attr("val", hex.trim_start_matches('#').to_uppercase())
}

/// The picture's `a:blip`.
fn blip(pic: &mut Element) -> Result<&mut Element> {
    pic.child_mut("blipFill")
        .and_then(|fill| fill.child_mut("blip"))
        .ok_or_else(|| WriteError::Xml("a picture without a blip".into()))
}

fn sp_pr<'e>(pic: &'e mut Element, p: &str) -> &'e mut Element {
    pic.child_or_insert(&qualified(p, "spPr"))
}

/// Sets (or with `None` removes) the blip colour effect `local`.
fn set_blip_effect(blip: &mut Element, local: &str, element: Option<Element>) {
    blip.remove_children(local);
    if let Some(element) = element {
        let mut order: Vec<&str> = BLIP_COLOR_EFFECTS.to_vec();
        order.push("extLst");
        insert_ordered(blip, element, &order);
    }
}

/// The `a14:imgLayer` of the blip, made when `make`.
fn image_layer<'b>(blip: &'b mut Element, a: &str, make: bool) -> Option<&'b mut Element> {
    let has = blip.child("extLst").is_some_and(|list| {
        list.elements()
            .any(|ext| ext.attr("uri") == Some(IMG_PROPS))
    });
    if !has {
        if !make {
            return None;
        }
        let ext = Element::new(qualified(a, "ext"))
            .with_attr("uri", IMG_PROPS)
            .with_child(
                Element::new("a14:imgProps")
                    .with_attr("xmlns:a14", A14)
                    .with_child(Element::new("a14:imgLayer")),
            );
        let list = blip.child_or_insert(&qualified(a, "extLst"));
        // The image effects come first, as PowerPoint writes them.
        list.children.insert(0, DomNode::Element(ext));
    }
    blip.child_mut("extLst")?
        .elements_mut()
        .find(|ext| ext.attr("uri") == Some(IMG_PROPS))?
        .child_mut("imgProps")?
        .child_mut("imgLayer")
}

/// The order PowerPoint lists image effects in.
const IMAGE_EFFECT_ORDER: &[&str] = &[
    "backgroundRemoval",
    "artistic",
    "sharpenSoften",
    "colorTemperature",
    "saturation",
    "brightnessContrast",
];

fn effect_rank(name: &str) -> usize {
    let name = if name.starts_with("artistic") {
        "artistic"
    } else {
        name
    };
    IMAGE_EFFECT_ORDER
        .iter()
        .position(|n| *n == name)
        .unwrap_or(IMAGE_EFFECT_ORDER.len())
}

/// Sets (or removes) the image effect whose element `matches`.
fn set_image_effect(
    blip: &mut Element,
    a: &str,
    matches: impl Fn(&str) -> bool,
    effect: Option<Element>,
) {
    let make = effect.is_some();
    let Some(layer) = image_layer(blip, a, make) else {
        return;
    };
    layer.children.retain(|node| {
        !matches!(node, DomNode::Element(wrapper)
            if wrapper.elements().any(|e| matches(e.local())))
    });
    if let Some(effect) = effect {
        let rank = effect_rank(effect.local());
        let at = layer
            .children
            .iter()
            .position(|node| {
                matches!(node, DomNode::Element(wrapper)
                if wrapper.elements().next().is_some_and(|e| effect_rank(e.local()) > rank))
            })
            .unwrap_or(layer.children.len());
        layer.children.insert(
            at,
            DomNode::Element(Element::new("a14:imgEffect").with_child(effect)),
        );
    }
    // An emptied layer goes with its extension.
    let empty = layer.elements().next().is_none();
    if empty {
        if let Some(list) = blip.child_mut("extLst") {
            list.children.retain(
                |node| !matches!(node, DomNode::Element(ext) if ext.attr("uri") == Some(IMG_PROPS)),
            );
        }
        if blip
            .child("extLst")
            .is_some_and(|list| list.elements().next().is_none())
        {
            blip.remove_children("extLst");
        }
    }
}

fn set_effect(effects: &mut Element, local: &str, element: Option<Element>) {
    effects.remove_children(local);
    if let Some(element) = element {
        insert_ordered(effects, element, EFFECT_ORDER);
    }
}

fn effect_list<'e>(pr: &'e mut Element, a: &str) -> &'e mut Element {
    if pr.child("effectLst").is_none() {
        insert_ordered(pr, Element::new(qualified(a, "effectLst")), SP_PR_ORDER);
    }
    pr.child_mut("effectLst").expect("just made")
}

fn shadow(a: &str, blur: i64, distance: i64, align: &str) -> Element {
    Element::new(qualified(a, "outerShdw"))
        .with_attr("blurRad", blur.to_string())
        .with_attr("dist", distance.to_string())
        .with_attr("dir", if distance == 0 { "0" } else { "2700000" })
        .with_attr("algn", align)
        .with_attr("rotWithShape", "0")
        .with_child(
            Element::new(qualified(a, "prstClr"))
                .with_attr("val", "black")
                .with_child(Element::new(qualified(a, "alpha")).with_attr("val", "40000")),
        )
}

fn reflection(a: &str) -> Element {
    Element::new(qualified(a, "reflection"))
        .with_attr("blurRad", "6350")
        .with_attr("stA", "50000")
        .with_attr("endA", "300")
        .with_attr("endPos", "55500")
        .with_attr("dir", "5400000")
        .with_attr("sy", "-100000")
        .with_attr("algn", "bl")
        .with_attr("rotWithShape", "0")
}

fn set_bevel(pr: &mut Element, a: &str, on: bool) {
    pr.remove_children("scene3d");
    pr.remove_children("sp3d");
    if on {
        insert_ordered(
            pr,
            Element::new(qualified(a, "scene3d"))
                .with_child(
                    Element::new(qualified(a, "camera")).with_attr("prst", "orthographicFront"),
                )
                .with_child(
                    Element::new(qualified(a, "lightRig"))
                        .with_attr("rig", "threePt")
                        .with_attr("dir", "t"),
                ),
            SP_PR_ORDER,
        );
        insert_ordered(
            pr,
            Element::new(qualified(a, "sp3d")).with_child(
                Element::new(qualified(a, "bevelT"))
                    .with_attr("w", "254000")
                    .with_attr("h", "254000"),
            ),
            SP_PR_ORDER,
        );
    }
}

fn set_geometry(pr: &mut Element, a: &str, preset: &str) {
    pr.remove_children("custGeom");
    pr.remove_children("prstGeom");
    insert_ordered(
        pr,
        Element::new(qualified(a, "prstGeom"))
            .with_attr("prst", preset)
            .with_child(Element::new(qualified(a, "avLst"))),
        SP_PR_ORDER,
    );
}

fn set_line(pr: &mut Element, a: &str, line: Option<(i64, &str, &str)>) {
    pr.remove_children("ln");
    if let Some((width, color, compound)) = line {
        let mut ln = Element::new(qualified(a, "ln")).with_attr("w", width.to_string());
        if compound != "sng" {
            ln.set_attr("cmpd", compound);
        }
        ln = ln.with_child(Element::new(qualified(a, "solidFill")).with_child(srgb(a, color)));
        insert_ordered(pr, ln, SP_PR_ORDER);
    }
}

/// Every format change taken away; with `size`, the crop too.
fn reset(pic: &mut Element, a: &str, p: &str, size: bool) -> Result<()> {
    let blip = blip(pic)?;
    for local in BLIP_COLOR_EFFECTS {
        blip.remove_children(local);
    }
    if let Some(list) = blip.child_mut("extLst") {
        list.children.retain(
            |node| !matches!(node, DomNode::Element(ext) if ext.attr("uri") == Some(IMG_PROPS)),
        );
    }
    if blip
        .child("extLst")
        .is_some_and(|list| list.elements().next().is_none())
    {
        blip.remove_children("extLst");
    }
    if size {
        if let Some(fill) = pic.child_mut("blipFill") {
            fill.remove_children("srcRect");
        }
    }
    let pr = sp_pr(pic, p);
    for local in ["ln", "effectLst", "effectDag", "scene3d", "sp3d"] {
        pr.remove_children(local);
    }
    set_geometry(pr, a, "rect");
    Ok(())
}

fn hex(value: &str) -> Result<String> {
    let value = value.trim_start_matches('#');
    if value.len() == 6 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(value.to_uppercase())
    } else {
        Err(WriteError::InvalidShapeStyle)
    }
}

fn apply(pic: &mut Element, edit: &PictureEdit) -> Result<()> {
    let p = prefix(&pic.name).to_string();
    let a = prefix(&blip(pic)?.name).to_string();
    let a = a.as_str();
    match edit {
        PictureEdit::Corrections {
            brightness,
            contrast,
        } => {
            // Drawn from the picture's pixels as `a:lum`; a baked Office
            // 2010 correction gives way to it.
            let lum = (brightness.abs() > 1e-6 || contrast.abs() > 1e-6).then(|| {
                Element::new(qualified(a, "lum"))
                    .with_attr("bright", percent(brightness.clamp(-1.0, 1.0)))
                    .with_attr("contrast", percent(contrast.clamp(-1.0, 1.0)))
            });
            set_image_effect(blip(pic)?, a, |n| n == "brightnessContrast", None);
            set_blip_effect(blip(pic)?, "lum", lum);
        }
        PictureEdit::Sharpness(amount) => {
            let effect = (amount.abs() > 1e-6).then(|| {
                Element::new("a14:sharpenSoften")
                    .with_attr("amount", percent(amount.clamp(-1.0, 1.0)))
            });
            set_image_effect(blip(pic)?, a, |n| n == "sharpenSoften", effect);
        }
        PictureEdit::Saturation(amount) => {
            let effect = ((amount - 1.0).abs() > 1e-6).then(|| {
                Element::new("a14:saturation").with_attr("sat", percent(amount.clamp(0.0, 4.0)))
            });
            set_image_effect(blip(pic)?, a, |n| n == "saturation", effect);
        }
        PictureEdit::Temperature(kelvin) => {
            let effect = ((kelvin - 6500.0).abs() > 0.5).then(|| {
                Element::new("a14:colorTemperature").with_attr(
                    "colorTemp",
                    (kelvin.clamp(1500.0, 11500.0).round() as i64).to_string(),
                )
            });
            set_image_effect(blip(pic)?, a, |n| n == "colorTemperature", effect);
        }
        PictureEdit::Recolor(recolor) => {
            let blip = blip(pic)?;
            for local in ["grayscl", "duotone", "biLevel"] {
                blip.remove_children(local);
            }
            // ウォッシュアウト is an `a:lum` of its own; corrections keep theirs.
            let washout = blip.child("lum").is_some_and(|lum| {
                lum.attr("bright") == Some("70000") && lum.attr("contrast") == Some("-70000")
            });
            if washout || *recolor == Recolor::Washout {
                blip.remove_children("lum");
            }
            let color = |scheme: &str, transform: (&str, &str), sat: &str| {
                Element::new(qualified(a, "schemeClr"))
                    .with_attr("val", scheme)
                    .with_child(
                        Element::new(qualified(a, transform.0)).with_attr("val", transform.1),
                    )
                    .with_child(Element::new(qualified(a, "satMod")).with_attr("val", sat))
            };
            let preset = |name: &str| Element::new(qualified(a, "prstClr")).with_attr("val", name);
            let element = match recolor {
                Recolor::None => None,
                Recolor::Grayscale => Some(Element::new(qualified(a, "grayscl"))),
                Recolor::Sepia => Some(
                    Element::new(qualified(a, "duotone"))
                        .with_child(preset("black"))
                        .with_child(
                            srgb(a, "D9C3A5")
                                .with_child(
                                    Element::new(qualified(a, "tint")).with_attr("val", "50000"),
                                )
                                .with_child(
                                    Element::new(qualified(a, "satMod")).with_attr("val", "180000"),
                                ),
                        ),
                ),
                Recolor::Washout => Some(
                    Element::new(qualified(a, "lum"))
                        .with_attr("bright", "70000")
                        .with_attr("contrast", "-70000"),
                ),
                Recolor::BlackWhite(threshold) => Some(
                    Element::new(qualified(a, "biLevel"))
                        .with_attr("thresh", (threshold.min(&100) * 1000).to_string()),
                ),
                Recolor::Dark(scheme) => Some(
                    Element::new(qualified(a, "duotone"))
                        .with_child(color(scheme, ("shade", "45000"), "135000"))
                        .with_child(preset("white")),
                ),
                Recolor::Light(scheme) => Some(
                    Element::new(qualified(a, "duotone"))
                        .with_child(preset("black"))
                        .with_child(color(scheme, ("tint", "45000"), "400000")),
                ),
            };
            if let Some(element) = element {
                let local = element.local().to_string();
                set_blip_effect(blip, &local, Some(element));
            }
        }
        PictureEdit::Transparency(amount) => {
            let amount = amount.clamp(0.0, 1.0);
            let element = (amount > 1e-6).then(|| {
                Element::new(qualified(a, "alphaModFix")).with_attr("amt", percent(1.0 - amount))
            });
            set_blip_effect(blip(pic)?, "alphaModFix", element);
        }
        PictureEdit::TransparentColor(color) => {
            let element = match color {
                Some(color) => {
                    let color = hex(color)?;
                    Some(
                        Element::new(qualified(a, "clrChange"))
                            .with_child(
                                Element::new(qualified(a, "clrFrom")).with_child(srgb(a, &color)),
                            )
                            .with_child(Element::new(qualified(a, "clrTo")).with_child(
                                srgb(a, &color).with_child(
                                    Element::new(qualified(a, "alpha")).with_attr("val", "0"),
                                ),
                            )),
                    )
                }
                None => None,
            };
            set_blip_effect(blip(pic)?, "clrChange", element);
        }
        PictureEdit::Artistic(name) => {
            let effect = match name {
                Some(name) if name.starts_with("artistic") => {
                    Some(Element::new(format!("a14:{name}")))
                }
                Some(_) => return Err(WriteError::InvalidShapeStyle),
                None => None,
            };
            set_image_effect(blip(pic)?, a, |n| n.starts_with("artistic"), effect);
        }
        PictureEdit::RemoveBackground(marquee) => {
            let effect = marquee.map(|[l, t, r, b]| {
                Element::new("a14:backgroundRemoval")
                    .with_attr("t", percent(t.clamp(0.0, 1.0)))
                    .with_attr("b", percent(b.clamp(0.0, 1.0)))
                    .with_attr("l", percent(l.clamp(0.0, 1.0)))
                    .with_attr("r", percent(r.clamp(0.0, 1.0)))
            });
            set_image_effect(blip(pic)?, a, |n| n == "backgroundRemoval", effect);
        }
        PictureEdit::Border { color, width_pt } => {
            let pr = sp_pr(pic, &p);
            let width = width_pt.map(|pt| (pt * 12_700.0).round() as i64);
            match color {
                Some(color) => {
                    let color = hex(color)?;
                    let old = pr
                        .child("ln")
                        .and_then(|ln| ln.attr("w"))
                        .and_then(|w| w.parse().ok());
                    let width = width.or(old).unwrap_or(9525);
                    set_line(pr, a, Some((width, &color, "sng")));
                }
                None => {
                    pr.remove_children("ln");
                    insert_ordered(
                        pr,
                        Element::new(qualified(a, "ln"))
                            .with_child(Element::new(qualified(a, "noFill"))),
                        SP_PR_ORDER,
                    );
                }
            }
        }
        PictureEdit::Effect(effect) => {
            let pr = sp_pr(pic, &p);
            match effect {
                PictureEffect::Shadow(on) => {
                    let element = on.then(|| shadow(a, 50_800, 38_100, "tl"));
                    set_effect(effect_list(pr, a), "outerShdw", element);
                }
                PictureEffect::Reflection(on) => {
                    set_effect(effect_list(pr, a), "reflection", on.then(|| reflection(a)));
                }
                PictureEffect::Glow(glow) => {
                    let element = match glow {
                        Some((color, size)) => Some(
                            Element::new(qualified(a, "glow"))
                                .with_attr("rad", ((size * 12_700.0).round() as i64).to_string())
                                .with_child(srgb(a, &hex(color)?).with_child(
                                    Element::new(qualified(a, "alpha")).with_attr("val", "40000"),
                                )),
                        ),
                        None => None,
                    };
                    set_effect(effect_list(pr, a), "glow", element);
                }
                PictureEffect::SoftEdge(radius) => {
                    let element = radius.map(|pt| {
                        Element::new(qualified(a, "softEdge"))
                            .with_attr("rad", ((pt * 12_700.0).round() as i64).to_string())
                    });
                    set_effect(effect_list(pr, a), "softEdge", element);
                }
                PictureEffect::Bevel(on) => set_bevel(pr, a, *on),
            }
            if pr
                .child("effectLst")
                .is_some_and(|list| list.elements().next().is_none())
            {
                pr.remove_children("effectLst");
            }
        }
        PictureEdit::Style(index) => {
            let style = PICTURE_STYLES
                .get(*index)
                .ok_or(WriteError::InvalidShapeStyle)?;
            let pr = sp_pr(pic, &p);
            set_geometry(pr, a, style.geometry);
            set_line(pr, a, style.line);
            pr.remove_children("effectLst");
            let mut effects = Element::new(qualified(a, "effectLst"));
            if let Some((blur, distance, align)) = style.shadow {
                set_effect(
                    &mut effects,
                    "outerShdw",
                    Some(shadow(a, blur, distance, align)),
                );
            }
            if style.reflection {
                set_effect(&mut effects, "reflection", Some(reflection(a)));
            }
            if let Some(radius) = style.soft_edge {
                set_effect(
                    &mut effects,
                    "softEdge",
                    Some(
                        Element::new(qualified(a, "softEdge")).with_attr("rad", radius.to_string()),
                    ),
                );
            }
            if effects.elements().next().is_some() {
                insert_ordered(pr, effects, SP_PR_ORDER);
            }
            set_bevel(pr, a, style.bevel);
        }
        PictureEdit::CropToShape(preset) => {
            if preset.is_empty() || !preset.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(WriteError::InvalidShapeStyle);
            }
            set_geometry(sp_pr(pic, &p), a, preset);
        }
        PictureEdit::LockAspect(on) => {
            let nv = pic
                .child_mut("nvPicPr")
                .ok_or_else(|| WriteError::Xml("a picture without nvPicPr".into()))?;
            let c_nv = nv.child_or_insert(&qualified(&p, "cNvPicPr"));
            let locks = c_nv.child_or_insert(&qualified(a, "picLocks"));
            if *on {
                locks.set_attr("noChangeAspect", "1");
            } else {
                locks.remove_attr("noChangeAspect");
            }
        }
        PictureEdit::Reset => reset(pic, a, &p, false)?,
        PictureEdit::ResetWithSize | PictureEdit::GraphicFill(_) => {
            // Handled with the package in hand.
        }
    }
    Ok(())
}

/// Rewrites picture `id`'s `<p:pic>` elements with `edit`.
fn rewrite(xml: &str, id: u32, edit: impl Fn(&mut Element) -> Result<()>) -> Result<String> {
    let mut patches = Vec::new();
    for range in picture_ranges(xml, id)? {
        let original = &xml[range.clone()];
        let mut pic = dom::parse(original).map_err(WriteError::Xml)?;
        let before = pic.to_xml();
        edit(&mut pic)?;
        let after = pic.to_xml();
        if after != before {
            patches.push((range, after));
        }
    }
    let mut out = xml.to_string();
    patches.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in patches {
        out.replace_range(range, &text);
    }
    Ok(out)
}

/// The blip's `r:embed` (and its SVG's) in the picture's first branch.
fn embeds(xml: &str, id: u32) -> Result<(Option<String>, Option<String>)> {
    let range = picture_ranges(xml, id)?.remove(0);
    let mut pic = dom::parse(&xml[range]).map_err(WriteError::Xml)?;
    let blip = blip(&mut pic)?;
    let bitmap = blip.attr("embed").map(str::to_string);
    let mut svgs = Vec::new();
    blip.descendants("svgBlip", &mut svgs);
    let svg = svgs
        .first()
        .and_then(|e| e.attr("embed"))
        .map(str::to_string);
    Ok((bitmap, svg))
}

fn target(package: &OpcPackage, slide_part: &str, rid: &str) -> Result<Option<String>> {
    let rels = parse_relationships(
        &package
            .part_str(&rels_part_name(slide_part))?
            .unwrap_or_default(),
    )?;
    Ok(rels
        .relationship(rid)
        .filter(|r| !r.external)
        .map(|r| resolve_target(slide_part, &r.target)))
}

fn finish(
    mut package: impl std::ops::DerefMut<Target = OpcPackage>,
    part: &str,
    xml: String,
) -> Result<Vec<u8>> {
    package.replace(part, xml.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// What the 図の形式 tab shows for picture `id`.
pub fn picture_format(bytes: &[u8], slide: usize, id: u32) -> Result<PictureFormatInfo> {
    let (package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let range = picture_ranges(&xml, id)?.remove(0);
    let mut pic = dom::parse(&xml[range]).map_err(WriteError::Xml)?;
    let lock_aspect = pic
        .child("nvPicPr")
        .and_then(|nv| nv.child("cNvPicPr"))
        .and_then(|c| c.child("picLocks"))
        .and_then(|l| l.attr("noChangeAspect"))
        .is_some_and(|v| matches!(v, "1" | "true"));
    let geometry = pic
        .child("spPr")
        .and_then(|pr| pr.child("prstGeom"))
        .and_then(|g| g.attr("prst"))
        .map(str::to_string);
    let blip = blip(&mut pic)?;
    let transparency = blip
        .child("alphaModFix")
        .and_then(|e| e.attr("amt"))
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(0.0, |amt| 1.0 - amt / 100_000.0);
    let (bitmap, svg) = embeds(&xml, id)?;
    let pixels = match bitmap {
        Some(rid) => target(&package, &slide_part, &rid)?
            .and_then(|part| package.part(&part).map(<[u8]>::to_vec))
            .and_then(|bytes| jade_picture_effects::image_size(&bytes)),
        None => None,
    };
    Ok(PictureFormatInfo {
        lock_aspect,
        geometry,
        transparency,
        pixels,
        svg: svg.is_some(),
    })
}

/// The picture as inserted (`a14:imgLayer@r:embed`, or the blip's own
/// when nothing is baked) and the baked copy the blip shows, if any.
fn originals(pic: &mut Element) -> Result<(Option<String>, Option<String>)> {
    let blip = blip(pic)?;
    let shown = blip.attr("embed").map(str::to_string);
    let mut layers = Vec::new();
    blip.descendants("imgLayer", &mut layers);
    match layers.first().and_then(|layer| layer.attr("embed")) {
        Some(original) => Ok((Some(original.to_string()), shown.filter(|s| s != original))),
        None => Ok((shown, None)),
    }
}

/// The Office 2010 image effects listed in the blip's `a14:imgLayer`, as
/// pixel operations.
fn baked_effects(pic: &mut Element) -> Result<Vec<jade_picture_effects::Effect>> {
    use jade_picture_effects::Effect;
    let blip = blip(pic)?;
    let mut layers = Vec::new();
    blip.descendants("imgLayer", &mut layers);
    let Some(layer) = layers.first() else {
        return Ok(Vec::new());
    };
    let fraction = |e: &Element, name: &str, default: f64| {
        e.attr(name)
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(default, |v| v / 100_000.0)
    };
    let mut effects = Vec::new();
    for wrapper in layer.elements() {
        for e in wrapper.elements() {
            effects.push(match e.local() {
                "saturation" => Effect::Saturation(fraction(e, "sat", 1.0)),
                "colorTemperature" => Effect::Temperature(
                    e.attr("colorTemp")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(6500.0),
                ),
                "sharpenSoften" => Effect::Sharpen(fraction(e, "amount", 0.0)),
                "brightnessContrast" => Effect::BrightnessContrast {
                    brightness: fraction(e, "bright", 0.0),
                    contrast: fraction(e, "contrast", 0.0),
                },
                "backgroundRemoval" => Effect::BackgroundRemoval([
                    fraction(e, "l", 0.0),
                    fraction(e, "t", 0.0),
                    fraction(e, "r", 1.0),
                    fraction(e, "b", 1.0),
                ]),
                name if name.starts_with("artistic") => Effect::Artistic(name.to_string()),
                _ => continue,
            });
        }
    }
    Ok(effects)
}

/// Whether `edit` changes what Office bakes into a copy of the picture.
fn rebakes(edit: &PictureEdit) -> bool {
    matches!(
        edit,
        PictureEdit::Corrections { .. }
            | PictureEdit::Sharpness(_)
            | PictureEdit::Saturation(_)
            | PictureEdit::Temperature(_)
            | PictureEdit::Artistic(_)
            | PictureEdit::RemoveBackground(_)
            | PictureEdit::Reset
            | PictureEdit::ResetWithSize
    )
}

/// The edits Office 2010 bakes — saturation, tone, sharpness, artistic
/// effects, background removal — applied as Office applies them: to a
/// copy of the picture as inserted, which the blip then shows, the
/// original kept in `a14:imgLayer` so they can be changed or reset.
fn rebake(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    id: u32,
    edit: impl Fn(&mut Element) -> Result<()>,
) -> Result<String> {
    let range = picture_ranges(xml, id)?.remove(0);
    let mut probe = dom::parse(&xml[range]).map_err(WriteError::Xml)?;
    let (original, old_baked) = originals(&mut probe)?;
    let original = original.ok_or_else(|| WriteError::Xml("a picture without an image".into()))?;
    edit(&mut probe)?;
    let effects = baked_effects(&mut probe)?;
    let shown = if effects.is_empty() {
        original.clone()
    } else {
        let part = target(package, slide_part, &original)?
            .ok_or(WriteError::UnsupportedObjectOperation(id))?;
        let source = package
            .part(&part)
            .ok_or(WriteError::UnsupportedObjectOperation(id))?
            .to_vec();
        let mut pixels = jade_picture_effects::decode(&source)
            .ok_or_else(|| WriteError::InvalidPicture("the picture cannot be adjusted".into()))?;
        jade_picture_effects::apply(&mut pixels, &effects);
        let png = jade_picture_effects::encode_png(&pixels)
            .ok_or_else(|| WriteError::InvalidPicture("the picture cannot be written".into()))?;
        let baked = crate::xml::pictures::media_part(package, &png, "image/png")?;
        package.replace(&baked, png);
        crate::xml::slides_content_types(
            package,
            &BTreeMap::new(),
            &BTreeSet::new(),
            Some((&baked, "image/png")),
        )?;
        add_relationship_named(
            package,
            slide_part,
            IMAGE_REL,
            &baked,
            &taken_ids(xml),
            "rIdJadePicture",
        )?
    };
    let changed = rewrite(xml, id, |pic| {
        edit(pic)?;
        let blip = blip(pic)?;
        let r = blip
            .attrs
            .iter()
            .find(|(key, _)| dom::local(key) == "embed")
            .map(|(key, _)| prefix(key).to_string())
            .unwrap_or_else(|| "r".into());
        blip.set_attr(&qualified(&r, "embed"), shown.clone());
        if let Some(layer) = image_layer(blip, "", false) {
            layer.set_attr(&qualified(&r, "embed"), original.clone());
        }
        Ok(())
    })?;
    // The old baked copy goes when nothing shows it any more.
    if let Some(old) = old_baked.filter(|old| *old != shown) {
        if !changed.contains(&format!("\"{old}\"")) {
            if let Some(part) = remove_relationship(package, slide_part, &old)? {
                drop_if_unreferenced(package, &part)?;
            }
        }
    }
    Ok(changed)
}

/// Applies one 図の形式 command to picture `id`.
pub fn format_picture(bytes: &[u8], slide: usize, id: u32, edit: &PictureEdit) -> Result<Vec<u8>> {
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let changed = match edit {
        PictureEdit::ResetWithSize => {
            let (bitmap, _) = embeds(&xml, id)?;
            let size = match bitmap {
                Some(rid) => target(&package, &slide_part, &rid)?
                    .and_then(|part| package.part(&part).map(<[u8]>::to_vec))
                    .and_then(|bytes| jade_picture_effects::image_size(&bytes)),
                None => None,
            };
            rebake(&mut package, &slide_part, &xml, id, |pic| {
                let p = prefix(&pic.name).to_string();
                let a = prefix(&blip(pic)?.name).to_string();
                reset(pic, &a, &p, true)?;
                if let Some((width, height)) = size {
                    if let Some(ext) = pic
                        .child_mut("spPr")
                        .and_then(|pr| pr.child_mut("xfrm"))
                        .and_then(|xfrm| xfrm.child_mut("ext"))
                    {
                        ext.set_attr("cx", (i64::from(width) * EMU_PER_PX).to_string());
                        ext.set_attr("cy", (i64::from(height) * EMU_PER_PX).to_string());
                    }
                }
                Ok(())
            })?
        }
        PictureEdit::GraphicFill(color) => {
            let color = hex(color)?;
            let (_, svg) = embeds(&xml, id)?;
            let svg_rid = svg.ok_or(WriteError::UnsupportedObjectOperation(id))?;
            let svg_part = target(&package, &slide_part, &svg_rid)?
                .ok_or(WriteError::UnsupportedObjectOperation(id))?;
            let source = package.required_part_str(&svg_part)?;
            let filled = fill_svg(&source, &color)?;
            let (svg_rid, bitmap_rid) =
                install_svg(&mut package, &slide_part, &xml, filled.as_bytes())?;
            rewrite(&xml, id, |pic| {
                let blip = blip(pic)?;
                blip.set_attr("r:embed", bitmap_rid.clone());
                let mut found = false;
                for ext in blip
                    .child_mut("extLst")
                    .into_iter()
                    .flat_map(|list| list.elements_mut())
                {
                    if let Some(svg) = ext.child_mut("svgBlip") {
                        svg.set_attr("r:embed", svg_rid.clone());
                        found = true;
                    }
                }
                found
                    .then_some(())
                    .ok_or(WriteError::UnsupportedObjectOperation(id))
            })?
        }
        edit if rebakes(edit) => {
            rebake(&mut package, &slide_part, &xml, id, |pic| apply(pic, edit))?
        }
        edit => rewrite(&xml, id, |pic| apply(pic, edit))?,
    };
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    finish(package, &slide_part, changed)
}

/// Every filled shape of an SVG in one colour, as グラフィックの塗りつぶし
/// does.
fn fill_svg(svg: &str, color: &str) -> Result<String> {
    let mut root = dom::parse(svg.trim_start_matches('\u{feff}')).map_err(WriteError::Xml)?;
    fn walk(element: &mut Element, color: &str) {
        const SHAPES: [&str; 7] = [
            "path", "rect", "circle", "ellipse", "polygon", "polyline", "text",
        ];
        if SHAPES.contains(&element.local()) && element.attr("fill") != Some("none") {
            element.set_attr("fill", format!("#{color}"));
            if let Some(style) = element.attr("style").map(str::to_string) {
                let kept: Vec<&str> = style
                    .split(';')
                    .filter(|rule| !rule.trim_start().starts_with("fill:"))
                    .collect();
                element.set_attr("style", kept.join(";"));
            }
        }
        for child in element.elements_mut() {
            walk(child, color);
        }
    }
    walk(&mut root, color);
    Ok(root.to_document())
}

/// Adds an SVG part and its PNG stand-in, related from the slide.
fn install_svg(
    package: &mut OpcPackage,
    slide_part: &str,
    slide_xml: &str,
    svg: &[u8],
) -> Result<(String, String)> {
    let pixels = jade_picture_effects::rasterize_svg(svg, 1024)
        .ok_or_else(|| WriteError::InvalidPicture("the SVG cannot be drawn".into()))?;
    let png = jade_picture_effects::encode_png(&pixels)
        .ok_or_else(|| WriteError::InvalidPicture("the SVG cannot be drawn".into()))?;
    let avoid = taken_ids(slide_xml);
    let mut ids = Vec::new();
    for (bytes, mime) in [(svg, "image/svg+xml"), (png.as_slice(), "image/png")] {
        let part = crate::xml::pictures::media_part(package, bytes, mime)?;
        package.replace(&part, bytes.to_vec());
        crate::xml::slides_content_types(
            package,
            &BTreeMap::new(),
            &BTreeSet::new(),
            Some((&part, mime)),
        )?;
        let mut avoid = avoid.clone();
        avoid.extend(ids.iter().cloned());
        ids.push(add_relationship_named(
            package,
            slide_part,
            IMAGE_REL,
            &part,
            &avoid,
            "rIdJadePicture",
        )?);
    }
    Ok((ids[0].clone(), ids[1].clone()))
}

fn taken_ids(xml: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut rest = xml;
    while let Some(at) = rest.find("rId") {
        let tail = &rest[at..];
        let end = tail.find(['"', '\'']).unwrap_or(tail.len());
        ids.insert(tail[..end].to_string());
        rest = &tail[end..];
    }
    ids
}

/// 挿入 › 図 with an SVG (or アイコン): the SVG with a PNG stand-in for
/// readers that cannot draw it, as PowerPoint writes one. The new
/// picture's id.
pub fn insert_svg_picture(
    bytes: &[u8],
    slide: usize,
    transform: Transform,
    svg: &[u8],
) -> Result<(Vec<u8>, u32)> {
    if jade_picture_effects::svg_size(svg).is_none() {
        return Err(WriteError::InvalidPicture("not an SVG".into()));
    }
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let (svg_rid, bitmap_rid) = install_svg(&mut package, &slide_part, &xml, svg)?;
    let (inserted, id) = crate::xml::insert_picture_shape(&xml, transform, &bitmap_rid)?;
    let with_svg = rewrite(&inserted, id, |pic| {
        let blip = blip(pic)?;
        let a = prefix(&blip.name).to_string();
        let ext = Element::new(qualified(&a, "ext"))
            .with_attr("uri", SVG_EXT)
            .with_child(
                Element::new("asvg:svgBlip")
                    .with_attr("xmlns:asvg", ASVG)
                    .with_attr("r:embed", svg_rid.clone()),
            );
        blip.child_or_insert(&qualified(&a, "extLst"))
            .children
            .push(DomNode::Element(ext));
        Ok(())
    })?;
    let fixed = fix_r_prefix(&with_svg, id)?;
    Ok((finish(package, &slide_part, fixed)?, id))
}

/// The `r:embed` the SVG extension was written with, renamed to the
/// prefix the picture binds the relationships namespace to.
fn fix_r_prefix(xml: &str, id: u32) -> Result<String> {
    rewrite(xml, id, |pic| {
        let blip = blip(pic)?;
        let r = blip
            .attrs
            .iter()
            .find(|(key, _)| dom::local(key) == "embed")
            .map(|(key, _)| prefix(key).to_string())
            .unwrap_or_else(|| "r".into());
        for ext in blip
            .child_mut("extLst")
            .into_iter()
            .flat_map(|list| list.elements_mut())
        {
            if let Some(svg) = ext.child_mut("svgBlip") {
                for (key, _) in svg.attrs.iter_mut() {
                    if key == "r:embed" {
                        *key = qualified(&r, "embed");
                    }
                }
            }
        }
        Ok(())
    })
}

/// 図の圧縮: pictures (all of them, or `only` one) brought down to `ppi`
/// at their shown size, and with `delete_cropped` their cropped-away parts
/// gone. How many pictures got smaller.
pub fn compress_pictures(
    bytes: &[u8],
    only: Option<(usize, u32)>,
    ppi: u32,
    delete_cropped: bool,
) -> Result<(Vec<u8>, usize)> {
    let count = jade_pptx_reader::read_pptx(bytes)?.slides.len();
    let mut current = bytes.to_vec();
    let mut compressed = 0;
    for slide in 0..count {
        if only.is_some_and(|(s, _)| s != slide) {
            continue;
        }
        let (_, _, xml) = crate::slide_xml(&current, slide)?;
        let ids: Vec<u32> = all_pictures(&xml)?
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| only.is_none_or(|(_, only)| only == *id))
            .collect();
        for id in ids {
            if let Some(next) = compress_one(&current, slide, id, ppi, delete_cropped)? {
                current = next;
                compressed += 1;
            }
        }
    }
    Ok((current, compressed))
}

fn compress_one(
    bytes: &[u8],
    slide: usize,
    id: u32,
    ppi: u32,
    delete_cropped: bool,
) -> Result<Option<Vec<u8>>> {
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let range = picture_ranges(&xml, id)?.remove(0);
    let pic = dom::parse(&xml[range]).map_err(WriteError::Xml)?;
    let ext = pic
        .child("spPr")
        .and_then(|pr| pr.child("xfrm"))
        .and_then(|xfrm| xfrm.child("ext"));
    let number = |name: &str| {
        ext.and_then(|e| e.attr(name))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let (cx, cy) = (number("cx"), number("cy"));
    if cx <= 0.0 || cy <= 0.0 {
        return Ok(None);
    }
    let fill = pic.child("blipFill");
    let crop = fill.and_then(|f| f.child("srcRect")).map(|rect| {
        let side = |name: &str| {
            rect.attr(name)
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(0.0, |v| v / 100_000.0)
        };
        [side("l"), side("t"), side("r"), side("b")]
    });
    let Some(rid) = fill
        .and_then(|f| f.child("blip"))
        .and_then(|b| b.attr("embed"))
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let Some(part) = target(&package, &slide_part, &rid)? else {
        return Ok(None);
    };
    let Some(source) = package.part(&part).map(<[u8]>::to_vec) else {
        return Ok(None);
    };
    if part.ends_with(".gif") || part.ends_with(".svg") {
        // An animation or a vector picture keeps its bytes.
        return Ok(None);
    }
    let visible = crop.map_or([1.0, 1.0], |[l, t, r, b]| {
        [(1.0 - l - r).max(0.01), (1.0 - t - b).max(0.01)]
    });
    let inches = |emu: f64| emu / 914_400.0;
    let max_width =
        (inches(cx) * f64::from(ppi) / if delete_cropped { 1.0 } else { visible[0] }).ceil() as u32;
    let max_height =
        (inches(cy) * f64::from(ppi) / if delete_cropped { 1.0 } else { visible[1] }).ceil() as u32;
    let crop = crop.filter(|_| delete_cropped);
    let Some((out, mime)) = jade_picture_effects::compress(&source, max_width, max_height, crop)
    else {
        return Ok(None);
    };
    // A part of its own, so other pictures showing the old one keep it.
    let new_part = crate::xml::pictures::media_part(&package, &out, mime)?;
    package.replace(&new_part, out);
    crate::xml::slides_content_types(
        &mut package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((&new_part, mime)),
    )?;
    let new_rid = add_relationship_named(
        &mut package,
        &slide_part,
        IMAGE_REL,
        &new_part,
        &taken_ids(&xml),
        "rIdJadePicture",
    )?;
    // Compressing discards the editing data: the effects stay baked in.
    let mut probe =
        dom::parse(&xml[picture_ranges(&xml, id)?.remove(0)]).map_err(WriteError::Xml)?;
    let (original, _) = originals(&mut probe)?;
    let changed = rewrite(&xml, id, |pic| {
        let blip = blip(pic)?;
        blip.set_attr("r:embed", new_rid.clone());
        if let Some(list) = blip.child_mut("extLst") {
            list.children.retain(
                |node| !matches!(node, DomNode::Element(ext) if ext.attr("uri") == Some(IMG_PROPS)),
            );
        }
        if blip
            .child("extLst")
            .is_some_and(|list| list.elements().next().is_none())
        {
            blip.remove_children("extLst");
        }
        if crop.is_some() {
            if let Some(fill) = pic.child_mut("blipFill") {
                fill.remove_children("srcRect");
            }
        }
        Ok(())
    })?;
    let mut gone = Vec::new();
    for old in std::iter::once(rid.clone()).chain(original) {
        if !changed.contains(&format!("\"{old}\"")) {
            if let Some(part) = remove_relationship(&mut package, &slide_part, &old)? {
                gone.push(part);
            }
        }
    }
    package.replace(&slide_part, changed.clone().into_bytes());
    for part in gone {
        drop_if_unreferenced(&mut package, &part)?;
    }
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(Some(result))
}

/// 図形に変換 on an SVG picture: its paths become freeform shapes in a
/// group where the picture was, each with its fill and outline. The
/// group's id.
pub fn convert_svg_to_shapes(bytes: &[u8], slide: usize, id: u32) -> Result<(Vec<u8>, u32)> {
    use jade_picture_effects::PathCommand;
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let ranges = picture_ranges(&xml, id)?;
    if ranges.len() != 1 {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let range = ranges[0].clone();
    let (_, svg) = embeds(&xml, id)?;
    let svg_rid = svg.ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let svg_part = target(&package, &slide_part, &svg_rid)?
        .ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let source = package
        .part(&svg_part)
        .ok_or(WriteError::UnsupportedObjectOperation(id))?
        .to_vec();
    let ((svg_width, svg_height), paths) = jade_picture_effects::svg_paths(&source)
        .ok_or_else(|| WriteError::InvalidPicture("the SVG cannot be read".into()))?;
    let pic = dom::parse(&xml[range.clone()]).map_err(WriteError::Xml)?;
    let xfrm = pic
        .child("spPr")
        .and_then(|pr| pr.child("xfrm"))
        .ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let number = |name: &str, attr: &str| {
        xfrm.child(name)
            .and_then(|e| e.attr(attr))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let (x, y, cx, cy) = (
        number("off", "x"),
        number("off", "y"),
        number("ext", "cx"),
        number("ext", "cy"),
    );
    let (sx, sy) = (
        cx / f64::from(svg_width.max(1.0)),
        cy / f64::from(svg_height.max(1.0)),
    );
    let (p_ns, a_ns, _) = crate::xml::smartart_frames::namespaces(&xml)?;
    let group_id = max_shape_id(&xml) + 1;
    // Keep the picture's orientation on the group, so every freeform is
    // turned and reflected around the same original picture bounds.
    let orientation: String = ["rot", "flipH", "flipV"]
        .into_iter()
        .filter_map(|name| {
            xfrm.attr(name)
                .map(|value| format!(" {name}=\"{}\"", dom::escape(value)))
        })
        .collect();
    let mut out = format!(
        "<jadeSp:grpSp xmlns:jadeSp=\"{p_ns}\" xmlns:a=\"{a_ns}\"><jadeSp:nvGrpSpPr><jadeSp:cNvPr id=\"{group_id}\" name=\"Group {group_id}\"/><jadeSp:cNvGrpSpPr/><jadeSp:nvPr/></jadeSp:nvGrpSpPr><jadeSp:grpSpPr><a:xfrm{orientation}><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/><a:chOff x=\"{x}\" y=\"{y}\"/><a:chExt cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm></jadeSp:grpSpPr>"
    );
    let mut shape_id = group_id;
    for path in &paths {
        // The path's points on the slide, in EMU.
        let mut points = Vec::new();
        for command in &path.commands {
            let pairs: &[(f32, f32)] = match command {
                PathCommand::Move(px, py) | PathCommand::Line(px, py) => &[(*px, *py)],
                PathCommand::Quad(ax, ay, px, py) => &[(*ax, *ay), (*px, *py)],
                PathCommand::Cubic(ax, ay, bx, by, px, py) => &[(*ax, *ay), (*bx, *by), (*px, *py)],
                PathCommand::Close => &[],
            };
            points.extend(
                pairs
                    .iter()
                    .map(|(px, py)| (x + f64::from(*px) * sx, y + f64::from(*py) * sy)),
            );
        }
        if points.is_empty() {
            continue;
        }
        let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let (w, h) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
        let at = |px: f32, py: f32| {
            format!(
                "<a:pt x=\"{}\" y=\"{}\"/>",
                (x + f64::from(px) * sx - min_x).round() as i64,
                (y + f64::from(py) * sy - min_y).round() as i64
            )
        };
        let mut commands = String::new();
        for command in &path.commands {
            commands.push_str(&match command {
                PathCommand::Move(px, py) => format!("<a:moveTo>{}</a:moveTo>", at(*px, *py)),
                PathCommand::Line(px, py) => format!("<a:lnTo>{}</a:lnTo>", at(*px, *py)),
                PathCommand::Quad(ax, ay, px, py) => {
                    format!(
                        "<a:quadBezTo>{}{}</a:quadBezTo>",
                        at(*ax, *ay),
                        at(*px, *py)
                    )
                }
                PathCommand::Cubic(ax, ay, bx, by, px, py) => format!(
                    "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                    at(*ax, *ay),
                    at(*bx, *by),
                    at(*px, *py)
                ),
                PathCommand::Close => "<a:close/>".to_string(),
            });
        }
        let solid = |rgb: [u8; 3], opacity: f32| {
            let alpha = if opacity < 1.0 {
                format!(
                    "<a:alpha val=\"{}\"/>",
                    (opacity * 100_000.0).round() as i64
                )
            } else {
                String::new()
            };
            format!(
                "<a:solidFill><a:srgbClr val=\"{:02X}{:02X}{:02X}\">{alpha}</a:srgbClr></a:solidFill>",
                rgb[0], rgb[1], rgb[2]
            )
        };
        let fill = path
            .fill
            .map_or_else(|| "<a:noFill/>".to_string(), |(rgb, o)| solid(rgb, o));
        let line = match path.stroke {
            Some((rgb, o, width)) => format!(
                "<a:ln w=\"{}\">{}</a:ln>",
                (f64::from(width) * sx).round() as i64,
                solid(rgb, o)
            ),
            None => "<a:ln><a:noFill/></a:ln>".to_string(),
        };
        shape_id += 1;
        out.push_str(&format!(
            "<jadeSp:sp><jadeSp:nvSpPr><jadeSp:cNvPr id=\"{shape_id}\" name=\"Freeform {shape_id}\"/><jadeSp:cNvSpPr/><jadeSp:nvPr/></jadeSp:nvSpPr><jadeSp:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"l\" t=\"t\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{}\" h=\"{}\">{commands}</a:path></a:pathLst></a:custGeom>{fill}{line}</jadeSp:spPr></jadeSp:sp>",
            min_x.round() as i64,
            min_y.round() as i64,
            w.round() as i64,
            h.round() as i64,
            w.round() as i64,
            h.round() as i64,
        ));
    }
    out.push_str("</jadeSp:grpSp>");
    if shape_id == group_id {
        return Err(WriteError::InvalidPicture("the SVG has no shapes".into()));
    }
    // The pictures the shapes replace — the SVG, its stand-in and any
    // original kept for a reset — go when nothing else names them.
    let named = taken_ids(&xml[range.clone()]);
    let mut changed = xml.clone();
    changed.replace_range(range, &out);
    let mut dropped = Vec::new();
    for rid in named {
        if !changed.contains(&format!("\"{rid}\"")) {
            dropped.extend(remove_relationship(&mut package, &slide_part, &rid)?);
        }
    }
    for part in dropped {
        drop_if_unreferenced(&mut package, &part)?;
    }
    Ok((finish(package, &slide_part, changed)?, group_id))
}

/// The largest `cNvPr` id on the slide.
fn max_shape_id(xml: &str) -> u32 {
    let mut max = 0;
    let mut rest = xml;
    while let Some(at) = rest.find("cNvPr ") {
        rest = &rest[at + 6..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        if let Some(start) = tag
            .find(" id=\"")
            .map(|i| i + 5)
            .or_else(|| tag.strip_prefix("id=\"").map(|_| 4))
        {
            let digits: String = tag[start..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            max = max.max(digits.parse().unwrap_or(0));
        }
    }
    max
}
