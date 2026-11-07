//! PowerPoint's animation presets: what the アニメーション gallery and the
//! その他の効果 lists offer, and the behaviours each one writes.
//!
//! An effect is identified the way PowerPoint identifies it in the file —
//! `presetClass`, `presetID`, `presetSubtype` — and its markup built from
//! the same behaviours PowerPoint writes for it, so a deck animated here
//! plays (and edits) the same in PowerPoint. Time-node ids are written as
//! `0`; the timing serialiser numbers them.

use jade_pptx_reader::animation::{AnimationTarget, EffectClass, EffectStart, TargetPart, XmlNode};

/// One preset in the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectPreset {
    pub class: PresetClass,
    pub id: u32,
    pub default_subtype: u32,
    /// English name, also the i18n key's tail.
    pub name: &'static str,
    /// In the ribbon gallery (the rest are under その他の効果).
    pub gallery: bool,
    /// The default 継続時間, ms.
    pub duration_ms: u32,
    /// What 効果のオプション offers: subtypes with their option key.
    pub options: &'static [(u32, &'static str)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetClass {
    Entrance,
    Emphasis,
    Exit,
    Path,
}

impl PresetClass {
    pub fn effect_class(self) -> EffectClass {
        match self {
            Self::Entrance => EffectClass::Entrance,
            Self::Emphasis => EffectClass::Emphasis,
            Self::Exit => EffectClass::Exit,
            Self::Path => EffectClass::MotionPath,
        }
    }

    pub fn from_class(class: &EffectClass) -> Option<Self> {
        match class {
            EffectClass::Entrance => Some(Self::Entrance),
            EffectClass::Emphasis => Some(Self::Emphasis),
            EffectClass::Exit => Some(Self::Exit),
            EffectClass::MotionPath => Some(Self::Path),
            _ => None,
        }
    }
}

/// Fly In / Fly Out and Wipe: from (or to) the eight sides.
const DIRECTIONS_8: &[(u32, &str)] = &[
    (4, "from_bottom"),
    (12, "from_bottom_left"),
    (8, "from_left"),
    (9, "from_top_left"),
    (1, "from_top"),
    (3, "from_top_right"),
    (2, "from_right"),
    (6, "from_bottom_right"),
];
const DIRECTIONS_4: &[(u32, &str)] = &[
    (4, "from_bottom"),
    (8, "from_left"),
    (2, "from_right"),
    (1, "from_top"),
];
const SPLIT: &[(u32, &str)] = &[
    (42, "horizontal_out"),
    (26, "horizontal_in"),
    (37, "vertical_out"),
    (21, "vertical_in"),
];
const IN_OUT: &[(u32, &str)] = &[(16, "in"), (32, "out")];
const SPOKES: &[(u32, &str)] = &[
    (1, "spoke_1"),
    (2, "spoke_2"),
    (3, "spoke_3"),
    (4, "spoke_4"),
    (8, "spoke_8"),
];
const BARS: &[(u32, &str)] = &[(10, "horizontal"), (5, "vertical")];
const ZOOM: &[(u32, &str)] = &[(16, "object_center"), (528, "slide_center")];
const SPIN: &[(u32, &str)] = &[(0, "clockwise"), (1, "counterclockwise")];
const GROW: &[(u32, &str)] = &[(0, "both"), (1, "horizontal"), (2, "vertical")];
const NONE: &[(u32, &str)] = &[];

macro_rules! preset {
    ($class:ident, $id:expr, $sub:expr, $name:expr, $gallery:expr, $ms:expr, $options:expr) => {
        EffectPreset {
            class: PresetClass::$class,
            id: $id,
            default_subtype: $sub,
            name: $name,
            gallery: $gallery,
            duration_ms: $ms,
            options: $options,
        }
    };
}

/// The catalogue, in gallery order within each class.
pub const PRESETS: &[EffectPreset] = &[
    // 開始
    preset!(Entrance, 1, 0, "appear", true, 0, NONE),
    preset!(Entrance, 10, 0, "fade", true, 500, NONE),
    preset!(Entrance, 2, 4, "fly_in", true, 500, DIRECTIONS_8),
    preset!(Entrance, 42, 0, "float_in", true, 1000, NONE),
    preset!(Entrance, 16, 37, "split", true, 500, SPLIT),
    preset!(Entrance, 22, 4, "wipe", true, 500, DIRECTIONS_4),
    preset!(Entrance, 6, 32, "shape", true, 2000, IN_OUT),
    preset!(Entrance, 21, 1, "wheel", true, 2000, SPOKES),
    preset!(Entrance, 14, 10, "random_bars", true, 500, BARS),
    preset!(Entrance, 31, 0, "grow_turn", true, 500, NONE),
    preset!(Entrance, 53, 16, "zoom", true, 500, ZOOM),
    preset!(Entrance, 45, 0, "swivel", true, 2000, NONE),
    preset!(Entrance, 26, 0, "bounce", true, 2000, NONE),
    preset!(Entrance, 3, 10, "blinds", false, 500, BARS),
    preset!(Entrance, 4, 16, "box", false, 500, IN_OUT),
    preset!(Entrance, 5, 10, "checkerboard", false, 500, BARS),
    preset!(Entrance, 8, 16, "diamond", false, 500, IN_OUT),
    preset!(Entrance, 9, 0, "dissolve_in", false, 500, NONE),
    preset!(Entrance, 13, 16, "plus", false, 500, IN_OUT),
    preset!(Entrance, 12, 4, "peek_in", false, 500, DIRECTIONS_4),
    preset!(Entrance, 18, 6, "strips", false, 500, DIRECTIONS_8),
    preset!(Entrance, 20, 0, "wedge", false, 500, NONE),
    preset!(Entrance, 47, 0, "float_down", false, 1000, NONE),
    preset!(Entrance, 55, 0, "expand", false, 500, NONE),
    preset!(Entrance, 37, 0, "rise_up", false, 1000, NONE),
    preset!(Entrance, 23, 16, "basic_zoom", false, 500, IN_OUT),
    // 強調
    preset!(Emphasis, 26, 0, "pulse", true, 500, NONE),
    preset!(Emphasis, 27, 0, "color_pulse", true, 500, NONE),
    preset!(Emphasis, 32, 0, "teeter", true, 1000, NONE),
    preset!(Emphasis, 8, 0, "spin", true, 2000, SPIN),
    preset!(Emphasis, 6, 0, "grow_shrink", true, 2000, GROW),
    preset!(Emphasis, 25, 0, "desaturate", true, 2000, NONE),
    preset!(Emphasis, 24, 0, "darken", true, 2000, NONE),
    preset!(Emphasis, 30, 0, "lighten", true, 2000, NONE),
    preset!(Emphasis, 9, 0, "transparency", true, 2000, NONE),
    preset!(Emphasis, 19, 0, "object_color", true, 2000, NONE),
    preset!(Emphasis, 21, 0, "complementary_color", true, 2000, NONE),
    preset!(Emphasis, 7, 2, "line_color", true, 2000, NONE),
    preset!(Emphasis, 1, 2, "fill_color", true, 2000, NONE),
    preset!(Emphasis, 16, 0, "brush_color", true, 1000, NONE),
    preset!(Emphasis, 3, 2, "font_color", true, 2000, NONE),
    preset!(Emphasis, 18, 0, "underline", true, 1000, NONE),
    preset!(Emphasis, 10, 0, "bold_flash", true, 1000, NONE),
    preset!(Emphasis, 15, 0, "bold_reveal", true, 1000, NONE),
    preset!(Emphasis, 34, 0, "wave", true, 1000, NONE),
    preset!(Emphasis, 35, 0, "blink", false, 500, NONE),
    // 終了
    preset!(Exit, 1, 0, "disappear", true, 0, NONE),
    preset!(Exit, 10, 0, "fade", true, 500, NONE),
    preset!(Exit, 2, 4, "fly_out", true, 500, DIRECTIONS_8),
    preset!(Exit, 42, 0, "float_out", true, 1000, NONE),
    preset!(Exit, 16, 37, "split", true, 500, SPLIT),
    preset!(Exit, 22, 4, "wipe", true, 500, DIRECTIONS_4),
    preset!(Exit, 6, 32, "shape", true, 2000, IN_OUT),
    preset!(Exit, 21, 1, "wheel", true, 2000, SPOKES),
    preset!(Exit, 14, 10, "random_bars", true, 500, BARS),
    preset!(Exit, 31, 0, "shrink_turn", true, 500, NONE),
    preset!(Exit, 53, 16, "zoom", true, 500, ZOOM),
    preset!(Exit, 45, 0, "swivel", true, 2000, NONE),
    preset!(Exit, 26, 0, "bounce", true, 2000, NONE),
    preset!(Exit, 3, 10, "blinds", false, 500, BARS),
    preset!(Exit, 4, 32, "box", false, 500, IN_OUT),
    preset!(Exit, 9, 0, "dissolve_out", false, 500, NONE),
    preset!(Exit, 47, 0, "sink_down", false, 1000, NONE),
    // アニメーションの軌跡
    preset!(Path, 42, 0, "lines", true, 2000, NONE),
    preset!(Path, 37, 0, "arcs", true, 2000, NONE),
    preset!(Path, 50, 0, "turns", true, 2000, NONE),
    preset!(Path, 1, 0, "shapes", true, 2000, NONE),
    preset!(Path, 24, 0, "loops", true, 2000, NONE),
    preset!(Path, 0, 0, "custom", true, 2000, NONE),
    preset!(Path, 35, 0, "left", false, 2000, NONE),
    preset!(Path, 63, 0, "up", false, 2000, NONE),
    preset!(Path, 64, 0, "right", false, 2000, NONE),
    preset!(Path, 51, 0, "arc_left", false, 2000, NONE),
    preset!(Path, 58, 0, "arc_right", false, 2000, NONE),
    preset!(Path, 44, 0, "arc_up", false, 2000, NONE),
    preset!(Path, 36, 0, "turn_right", false, 2000, NONE),
    preset!(Path, 43, 0, "turn_up", false, 2000, NONE),
    preset!(Path, 57, 0, "turn_up_right", false, 2000, NONE),
    preset!(Path, 3, 0, "diamond", false, 2000, NONE),
    preset!(Path, 7, 0, "square", false, 2000, NONE),
    preset!(Path, 13, 0, "equal_triangle", false, 2000, NONE),
    preset!(Path, 8, 0, "trapezoid", false, 2000, NONE),
    preset!(Path, 10, 0, "octagon", false, 2000, NONE),
    preset!(Path, 5, 0, "five_point_star", false, 2000, NONE),
    preset!(Path, 9, 0, "heart", false, 2000, NONE),
    preset!(Path, 49, 0, "diagonal_down_right", false, 2000, NONE),
    preset!(Path, 56, 0, "diagonal_up_right", false, 2000, NONE),
    preset!(Path, 47, 0, "wave", false, 2000, NONE),
    preset!(Path, 38, 0, "zigzag", false, 2000, NONE),
];

/// The preset for a class and id.
pub fn preset(class: PresetClass, id: u32) -> Option<&'static EffectPreset> {
    PRESETS.iter().find(|p| p.class == class && p.id == id)
}

/// Parameters an effect's options set beyond its subtype.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectParams {
    /// 方向 / subtype; `None` takes the preset's default.
    pub subtype: Option<u32>,
    /// A colour option (`RRGGBB`) for colour emphases.
    pub color: Option<String>,
    /// スピン's amount in degrees, 拡大/収縮's in percent.
    pub amount: Option<f64>,
    /// A motion path's points (slide fractions, relative to the start), for
    /// ユーザー設定パス; `closed` ends it with `Z`.
    pub path: Option<Vec<(f64, f64)>>,
    pub closed: bool,
}

/// `<p:tgtEl>` for a target.
pub fn target_xml(target: &AnimationTarget) -> String {
    let inner = match &target.part {
        TargetPart::Shape => String::new(),
        TargetPart::Background => "<p:bg/>".to_string(),
        TargetPart::Paragraphs { start, end } => {
            format!(r#"<p:txEl><p:pRg st="{start}" end="{end}"/></p:txEl>"#)
        }
        TargetPart::Characters { start, end } => {
            format!(r#"<p:txEl><p:charRg st="{start}" end="{end}"/></p:txEl>"#)
        }
        TargetPart::Chart {
            series,
            category,
            step,
        } => format!(
            r#"<p:graphicEl><a:chart seriesIdx="{series}" categoryIdx="{category}" bldStep="{step}"/></p:graphicEl>"#
        ),
        TargetPart::Diagram { id, step } => {
            format!(r#"<p:graphicEl><a:dgm id="{id}" bldStep="{step}"/></p:graphicEl>"#)
        }
        TargetPart::Subshape { id } => format!(r#"<p:subSp spid="{id}"/>"#),
        TargetPart::OleChart { kind, level } => {
            format!(r#"<p:oleChartEl type="{kind}" lvl="{level}"/>"#)
        }
    };
    if inner.is_empty() {
        format!(
            r#"<p:tgtEl><p:spTgt spid="{}"/></p:tgtEl>"#,
            target.shape_id
        )
    } else {
        format!(
            r#"<p:tgtEl><p:spTgt spid="{}">{inner}</p:spTgt></p:tgtEl>"#,
            target.shape_id
        )
    }
}

/// Builds behaviours as markup.
struct Behaviors<'a> {
    tgt: String,
    out: String,
    _target: &'a AnimationTarget,
}

impl<'a> Behaviors<'a> {
    fn new(target: &'a AnimationTarget) -> Self {
        Self {
            tgt: target_xml(target),
            out: String::new(),
            _target: target,
        }
    }

    fn visibility(&mut self, value: &str, delay: u32) {
        self.out.push_str(&format!(
            r#"<p:set><p:cBhvr><p:cTn id="0" dur="1" fill="hold"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst></p:cTn>{}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="{value}"/></p:to></p:set>"#,
            self.tgt
        ));
    }

    fn set(&mut self, attr: &str, value: &str, ms: u32, extra: &str) {
        self.out.push_str(&format!(
            r#"<p:set><p:cBhvr><p:cTn id="0" dur="{ms}" fill="hold"{extra}/>{}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="{value}"/></p:to></p:set>"#,
            self.tgt
        ));
    }

    fn effect(&mut self, transition: &str, filter: &str, ms: u32) {
        self.out.push_str(&format!(
            r#"<p:animEffect transition="{transition}" filter="{filter}"><p:cBhvr><p:cTn id="0" dur="{ms}"/>{}</p:cBhvr></p:animEffect>"#,
            self.tgt
        ));
    }

    /// `<p:anim>` from `from` to `to` (formulas as PowerPoint writes them).
    fn anim(&mut self, attr: &str, from: &str, to: &str, ms: u32, extra: &str) {
        self.anim_keys(attr, &[(0, from, None), (100_000, to, None)], ms, extra);
    }

    fn anim_keys(&mut self, attr: &str, keys: &[(u32, &str, Option<&str>)], ms: u32, extra: &str) {
        let tavs: String = keys
            .iter()
            .map(|(tm, value, fmla)| {
                let fmla = fmla.map(|f| format!(r#" fmla="{f}""#)).unwrap_or_default();
                let val = if let Ok(number) = value.parse::<f64>() {
                    format!(r#"<p:fltVal val="{number}"/>"#)
                } else {
                    format!(r#"<p:strVal val="{value}"/>"#)
                };
                format!(r#"<p:tav tm="{tm}"{fmla}><p:val>{val}</p:val></p:tav>"#)
            })
            .collect();
        self.out.push_str(&format!(
            r#"<p:anim calcmode="lin" valueType="num"><p:cBhvr additive="base"><p:cTn id="0" dur="{ms}" fill="hold"{extra}/>{}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:tavLst>{tavs}</p:tavLst></p:anim>"#,
            self.tgt
        ));
    }

    fn scale(&mut self, x: i64, y: i64, ms: u32, extra: &str) {
        self.out.push_str(&format!(
            r#"<p:animScale><p:cBhvr><p:cTn id="0" dur="{ms}" fill="hold"{extra}/>{}</p:cBhvr><p:by x="{x}" y="{y}"/></p:animScale>"#,
            self.tgt
        ));
    }

    fn rotate(&mut self, by_degrees: f64, ms: u32, delay: Option<u32>) {
        let timing = match delay {
            Some(delay) => format!(
                r#"<p:cTn id="0" dur="{ms}" fill="hold"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst></p:cTn>"#
            ),
            None => format!(r#"<p:cTn id="0" dur="{ms}" fill="hold"/>"#),
        };
        self.out.push_str(&format!(
            r#"<p:animRot by="{}"><p:cBhvr>{timing}{}<p:attrNameLst><p:attrName>r</p:attrName></p:attrNameLst></p:cBhvr></p:animRot>"#,
            (by_degrees * 60_000.0).round() as i64,
            self.tgt
        ));
    }

    fn color(&mut self, attr: &str, to: &str, ms: u32, extra: &str) {
        self.out.push_str(&format!(
            r#"<p:animClr clrSpc="rgb" dir="cw"><p:cBhvr override="childStyle"><p:cTn id="0" dur="{ms}" fill="hold"{extra}/>{}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:to>{to}</p:to></p:animClr>"#,
            self.tgt
        ));
    }

    fn color_by_hsl(&mut self, attr: &str, h: i64, s: i64, l: i64, ms: u32) {
        self.out.push_str(&format!(
            r#"<p:animClr clrSpc="hsl" dir="cw"><p:cBhvr override="childStyle"><p:cTn id="0" dur="{ms}" fill="hold"/>{}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:by><p:hsl h="{h}" s="{s}" l="{l}"/></p:by></p:animClr>"#,
            self.tgt
        ));
    }

    fn motion(&mut self, path: &str, points: &str, ms: u32, center: (f64, f64)) {
        self.out.push_str(&format!(
            r#"<p:animMotion origin="layout" path="{path}" pathEditMode="relative" rAng="0" ptsTypes="{points}"><p:cBhvr><p:cTn id="0" dur="{ms}" fill="hold"/>{}<p:attrNameLst><p:attrName>ppt_x</p:attrName><p:attrName>ppt_y</p:attrName></p:attrNameLst></p:cBhvr><p:rCtr x="{}" y="{}"/></p:animMotion>"#,
            self.tgt,
            (center.0 * 100_000.0).round() as i64,
            (center.1 * 100_000.0).round() as i64,
        ));
    }
}

/// The colour markup for a `RRGGBB` option, or the preset's scheme default.
fn color_xml(color: Option<&str>, default_scheme: &str) -> String {
    match color {
        Some(rgb) => format!(r#"<a:srgbClr val="{rgb}"/>"#),
        None => format!(r#"<a:schemeClr val="{default_scheme}"/>"#),
    }
}

/// Wipe / Peek / Strips direction filters, for an entrance; an exit runs
/// the other way.
fn wipe_filter(subtype: u32) -> &'static str {
    match subtype {
        1 => "wipe(down)",
        2 => "wipe(left)",
        8 => "wipe(right)",
        _ => "wipe(up)",
    }
}

fn split_filter(subtype: u32) -> &'static str {
    match subtype {
        21 => "barn(inVertical)",
        26 => "barn(inHorizontal)",
        37 => "barn(outVertical)",
        _ => "barn(outHorizontal)",
    }
}

/// Fly In's starting point (Fly Out's end) for a direction subtype.
fn fly_offscreen(subtype: u32) -> (&'static str, &'static str) {
    let x = match subtype {
        8 | 9 | 12 => "0-#ppt_w/2",
        2 | 3 | 6 => "1+#ppt_w/2",
        _ => "#ppt_x",
    };
    let y = match subtype {
        1 | 3 | 9 => "0-#ppt_h/2",
        4 | 6 | 12 => "1+#ppt_h/2",
        _ => "#ppt_y",
    };
    (x, y)
}

/// A motion path's markup: the path, its point types and its rotation
/// centre (half its extent).
pub fn path_markup(id: u32, params: &EffectParams) -> (String, String) {
    if let Some(points) = params.path.as_ref().filter(|p| p.len() >= 2) {
        let mut path = String::new();
        for (index, (x, y)) in points.iter().enumerate() {
            path.push_str(if index == 0 { "M " } else { " L " });
            path.push_str(&format!("{} {}", fmt(*x), fmt(*y)));
        }
        path.push_str(if params.closed { " Z" } else { " E" });
        return (path, "A".repeat(points.len()));
    }
    let (path, types) = match id {
        35 => ("M 0 0 L -0.25 0 E", "AA"),
        63 => ("M 0 0 L 0 -0.25 E", "AA"),
        64 => ("M 0 0 L 0.25 0 E", "AA"),
        49 => ("M 0 0 L 0.25 0.25 E", "AA"),
        56 => ("M 0 0 L 0.25 -0.25 E", "AA"),
        37 => ("M 0 0 C 0.069 0 0.125 0.054 0.125 0.121 C 0.125 0.188 0.069 0.242 0 0.242 E", "AAA"),
        51 => ("M 0 0 C 0 0.069 -0.054 0.125 -0.121 0.125 C -0.188 0.125 -0.242 0.069 -0.242 0 E", "AAA"),
        58 => ("M 0 0 C 0 -0.069 0.054 -0.125 0.121 -0.125 C 0.188 -0.125 0.242 -0.069 0.242 0 E", "AAA"),
        44 => ("M 0 0 C -0.069 0 -0.125 -0.054 -0.125 -0.121 C -0.125 -0.188 -0.069 -0.242 0 -0.242 E", "AAA"),
        50 => ("M 0 0 L 0.125 0 C 0.194 0 0.25 0.056 0.25 0.125 L 0.25 0.25 E", "AAAA"),
        36 => ("M 0 0 L 0 0.125 C 0 0.194 0.056 0.25 0.125 0.25 L 0.25 0.25 E", "AAAA"),
        43 => ("M 0 0 L 0.125 0 C 0.194 0 0.25 -0.056 0.25 -0.125 L 0.25 -0.25 E", "AAAA"),
        57 => ("M 0 0 L 0 -0.125 C 0 -0.194 0.056 -0.25 0.125 -0.25 L 0.25 -0.25 E", "AAAA"),
        1 => ("M 0 0 C 0 0.069 0.056 0.125 0.125 0.125 C 0.194 0.125 0.25 0.069 0.25 0 C 0.25 -0.069 0.194 -0.125 0.125 -0.125 C 0.056 -0.125 0 -0.069 0 0 Z", "AAAAA"),
        3 => ("M 0 0 L 0.125 0.125 L 0.25 0 L 0.125 -0.125 L 0 0 Z", "AAAAA"),
        7 => ("M 0 0 L 0.25 0 L 0.25 0.25 L 0 0.25 L 0 0 Z", "AAAAA"),
        13 => ("M 0 0 L 0.125 -0.217 L 0.25 0 L 0 0 Z", "AAAA"),
        8 => ("M 0 0 L 0.0625 -0.2 L 0.1875 -0.2 L 0.25 0 L 0 0 Z", "AAAAA"),
        10 => ("M 0 0 L 0.073 0.073 L 0.177 0.073 L 0.25 0 L 0.25 -0.104 L 0.177 -0.177 L 0.073 -0.177 L 0 -0.104 L 0 0 Z", "AAAAAAAAA"),
        5 => ("M 0 0 L 0.095 0 L 0.125 -0.09 L 0.155 0 L 0.25 0 L 0.173 0.056 L 0.202 0.147 L 0.125 0.091 L 0.048 0.147 L 0.077 0.056 L 0 0 Z", "AAAAAAAAAAA"),
        9 => ("M 0 0 C 0 -0.07 0.06 -0.1 0.125 -0.04 C 0.19 -0.1 0.25 -0.07 0.25 0 C 0.25 0.07 0.19 0.12 0.125 0.18 C 0.06 0.12 0 0.07 0 0 Z", "AAAAA"),
        24 => ("M 0 0 C 0.1 0 0.15 -0.08 0.1 -0.13 C 0.05 -0.18 -0.02 -0.1 0.05 -0.03 C 0.1 0.02 0.18 0 0.25 0 E", "AAAA"),
        47 => ("M 0 0 C 0.04 -0.06 0.08 -0.06 0.125 0 C 0.17 0.06 0.21 0.06 0.25 0 E", "AAA"),
        38 => ("M 0 0 L 0.05 0.05 L 0.1 0 L 0.15 0.05 L 0.2 0 L 0.25 0.05 E", "AAAAAA"),
        // 42 and anything unknown: 直線 (下へ).
        _ => ("M 0 0 L 0 0.25 E", "AA"),
    };
    (path.to_string(), types.to_string())
}

fn fmt(value: f64) -> String {
    let rounded = (value * 100_000.0).round() / 100_000.0;
    if rounded == 0.0 {
        "0".to_string()
    } else {
        format!("{rounded}")
    }
}

/// The extent of a path's points, halved: PowerPoint's `rCtr`.
fn path_center(path: &str) -> (f64, f64) {
    let numbers: Vec<f64> = path
        .split_whitespace()
        .filter_map(|token| token.parse::<f64>().ok())
        .collect();
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for pair in numbers.chunks(2) {
        if let [x, y] = pair {
            min_x = min_x.min(*x);
            max_x = max_x.max(*x);
            min_y = min_y.min(*y);
            max_y = max_y.max(*y);
        }
    }
    ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
}

/// Builds an effect `<p:par>` for a preset: the effect's own time node
/// with its behaviours. `duration_ms` overrides the preset's length.
pub fn build_effect_node(
    preset: &EffectPreset,
    params: &EffectParams,
    target: &AnimationTarget,
    start: EffectStart,
    duration_ms: Option<u32>,
) -> XmlNode {
    let subtype = params.subtype.unwrap_or(preset.default_subtype);
    let ms = duration_ms.unwrap_or(preset.duration_ms);
    let mut b = Behaviors::new(target);
    let exit = preset.class == PresetClass::Exit;
    let mut accel_decel = "";
    match (preset.class, preset.id) {
        (PresetClass::Entrance | PresetClass::Exit, 1) => {
            b.visibility(if exit { "hidden" } else { "visible" }, 0);
        }
        (PresetClass::Entrance | PresetClass::Exit, 2) => {
            let (x, y) = fly_offscreen(subtype);
            if !exit {
                b.visibility("visible", 0);
                b.anim("ppt_x", x, "#ppt_x", ms, "");
                b.anim("ppt_y", y, "#ppt_y", ms, "");
            } else {
                b.anim("ppt_x", "#ppt_x", x, ms, "");
                b.anim("ppt_y", "#ppt_y", y, ms, "");
                b.visibility("hidden", ms.saturating_sub(1));
            }
        }
        (PresetClass::Entrance, 42) | (PresetClass::Entrance, 47) | (PresetClass::Entrance, 37) => {
            let offset = if preset.id == 47 {
                "#ppt_y-.1"
            } else {
                "#ppt_y+.1"
            };
            b.visibility("visible", 0);
            b.effect("in", "fade", ms);
            b.anim("ppt_x", "#ppt_x", "#ppt_x", ms, r#" decel="100000""#);
            b.anim("ppt_y", offset, "#ppt_y", ms, r#" decel="100000""#);
        }
        (PresetClass::Exit, 42) | (PresetClass::Exit, 47) => {
            let offset = if preset.id == 47 {
                "#ppt_y+.1"
            } else {
                "#ppt_y-.1"
            };
            b.effect("out", "fade", ms);
            b.anim("ppt_x", "#ppt_x", "#ppt_x", ms, r#" accel="100000""#);
            b.anim("ppt_y", "#ppt_y", offset, ms, r#" accel="100000""#);
            b.visibility("hidden", ms.saturating_sub(1));
        }
        (PresetClass::Entrance | PresetClass::Exit, 31) => {
            let (w0, w1, r0, r1) = if exit {
                ("#ppt_w", "0", "0", "90")
            } else {
                ("0", "#ppt_w", "-90", "0")
            };
            let (h0, h1) = if exit {
                ("#ppt_h", "0")
            } else {
                ("0", "#ppt_h")
            };
            if !exit {
                b.visibility("visible", 0);
            }
            b.anim("ppt_w", w0, w1, ms, "");
            b.anim("ppt_h", h0, h1, ms, "");
            b.anim("style.rotation", r0, r1, ms, "");
            b.effect(if exit { "out" } else { "in" }, "fade", ms);
            if exit {
                b.visibility("hidden", ms.saturating_sub(1));
            }
        }
        (PresetClass::Entrance | PresetClass::Exit, 53)
        | (PresetClass::Entrance | PresetClass::Exit, 23) => {
            let slide_center = subtype == 528;
            if !exit {
                b.visibility("visible", 0);
                b.anim("ppt_w", "0", "#ppt_w", ms, "");
                b.anim("ppt_h", "0", "#ppt_h", ms, "");
                if slide_center {
                    b.anim("ppt_x", "0.5", "#ppt_x", ms, "");
                    b.anim("ppt_y", "0.5", "#ppt_y", ms, "");
                }
                if preset.id == 53 {
                    b.effect("in", "fade", ms);
                }
            } else {
                b.anim("ppt_w", "#ppt_w", "0", ms, "");
                b.anim("ppt_h", "#ppt_h", "0", ms, "");
                if slide_center {
                    b.anim("ppt_x", "#ppt_x", "0.5", ms, "");
                    b.anim("ppt_y", "#ppt_y", "0.5", ms, "");
                }
                if preset.id == 53 {
                    b.effect("out", "fade", ms);
                }
                b.visibility("hidden", ms.saturating_sub(1));
            }
        }
        (PresetClass::Entrance | PresetClass::Exit, 45) => {
            if !exit {
                b.visibility("visible", 0);
                b.effect("in", "fade", ms);
                b.anim_keys(
                    "ppt_w",
                    &[(0, "0", Some("#ppt_w*sin(2.5*pi*$)")), (100_000, "1", None)],
                    ms,
                    "",
                );
            } else {
                b.effect("out", "fade", ms);
                b.anim_keys(
                    "ppt_w",
                    &[(0, "0", Some("#ppt_w*cos(2.5*pi*$)")), (100_000, "1", None)],
                    ms,
                    "",
                );
                b.visibility("hidden", ms.saturating_sub(1));
            }
        }
        (PresetClass::Entrance, 26) => {
            b.visibility("visible", 0);
            b.anim("ppt_x", "#ppt_x-0.25", "#ppt_x", ms, "");
            b.anim_keys(
                "ppt_y",
                &[
                    (0, "#ppt_y-0.25", None),
                    (40_000, "#ppt_y", None),
                    (60_000, "#ppt_y-0.08", None),
                    (80_000, "#ppt_y", None),
                    (90_000, "#ppt_y-0.02", None),
                    (100_000, "#ppt_y", None),
                ],
                ms,
                "",
            );
            b.effect("in", "fade", ms / 4);
        }
        (PresetClass::Exit, 26) => {
            b.anim("ppt_x", "#ppt_x", "#ppt_x+0.25", ms, "");
            b.anim_keys(
                "ppt_y",
                &[
                    (0, "#ppt_y", None),
                    (10_000, "#ppt_y-0.02", None),
                    (20_000, "#ppt_y", None),
                    (40_000, "#ppt_y-0.08", None),
                    (60_000, "#ppt_y", None),
                    (100_000, "1+#ppt_h/2", None),
                ],
                ms,
                "",
            );
            b.visibility("hidden", ms.saturating_sub(1));
        }
        (PresetClass::Entrance | PresetClass::Exit, id) => {
            let filter = match id {
                10 => "fade".to_string(),
                22 => wipe_filter(if exit { opposite(subtype) } else { subtype }).to_string(),
                16 => split_filter(subtype).to_string(),
                6 => format!("circle({})", in_out(subtype)),
                4 => format!("box({})", in_out(subtype)),
                8 => format!("diamond({})", in_out(subtype)),
                13 => format!("plus({})", in_out(subtype)),
                21 => format!("wheel({})", subtype.max(1)),
                14 => format!(
                    "randombar({})",
                    if subtype == 5 {
                        "vertical"
                    } else {
                        "horizontal"
                    }
                ),
                3 => format!(
                    "blinds({})",
                    if subtype == 5 {
                        "vertical"
                    } else {
                        "horizontal"
                    }
                ),
                5 => format!(
                    "checkerboard({})",
                    if subtype == 5 { "down" } else { "across" }
                ),
                9 => "dissolve".to_string(),
                12 => format!(
                    "slide(from{})",
                    side(if exit { opposite(subtype) } else { subtype })
                ),
                18 => format!(
                    "strips({})",
                    match subtype {
                        3 => "downLeft",
                        9 => "downRight",
                        12 => "upRight",
                        _ => "upLeft",
                    }
                ),
                20 => "wedge".to_string(),
                55 => "fade".to_string(),
                _ => "fade".to_string(),
            };
            if exit {
                b.effect("out", &filter, ms);
                b.visibility("hidden", ms.saturating_sub(1));
            } else {
                b.visibility("visible", 0);
                b.effect("in", &filter, ms);
            }
        }
        (PresetClass::Emphasis, 26) => {
            b.scale(105_000, 105_000, ms / 2, r#" autoRev="1""#);
        }
        (PresetClass::Emphasis, 27) => {
            b.set("fill.type", "solid", 1, "");
            b.set("fill.on", "true", 1, "");
            b.color(
                "fillcolor",
                &color_xml(params.color.as_deref(), "accent2"),
                ms / 2,
                r#" autoRev="1""#,
            );
        }
        (PresetClass::Emphasis, 32) => {
            let step = ms / 10;
            b.rotate(2.0, step, None);
            b.rotate(-4.0, step * 2, Some(step));
            b.rotate(4.0, step * 2, Some(step * 3));
            b.rotate(-4.0, step * 2, Some(step * 5));
            b.rotate(2.0, step * 3, Some(step * 7));
        }
        (PresetClass::Emphasis, 8) => {
            let amount = params.amount.unwrap_or(360.0);
            let amount = if subtype == 1 { -amount } else { amount };
            b.rotate(amount, ms, None);
        }
        (PresetClass::Emphasis, 6) => {
            let amount = params.amount.unwrap_or(150.0);
            let full = (amount * 1000.0).round() as i64;
            let (x, y) = match subtype {
                1 => (full, 100_000),
                2 => (100_000, full),
                _ => (full, full),
            };
            b.scale(x, y, ms, "");
        }
        (PresetClass::Emphasis, 25) => {
            b.color_by_hsl("fillcolor", 0, -100_000, 0, ms);
        }
        (PresetClass::Emphasis, 24) => {
            b.color_by_hsl("fillcolor", 0, 0, -25_000, ms);
        }
        (PresetClass::Emphasis, 30) => {
            b.color_by_hsl("fillcolor", 0, 0, 25_000, ms);
        }
        (PresetClass::Emphasis, 9) => {
            let amount = params.amount.unwrap_or(50.0);
            b.set(
                "style.opacity",
                &format!("{}", 1.0 - amount / 100.0),
                ms,
                "",
            );
        }
        (PresetClass::Emphasis, 19) | (PresetClass::Emphasis, 1) => {
            b.set("fill.type", "solid", 1, "");
            b.set("fill.on", "true", 1, "");
            b.color(
                "fillcolor",
                &color_xml(params.color.as_deref(), "accent2"),
                ms,
                "",
            );
        }
        (PresetClass::Emphasis, 21) => {
            b.color_by_hsl("fillcolor", 10_800_000, 0, 0, ms);
        }
        (PresetClass::Emphasis, 7) => {
            b.set("stroke.on", "true", 1, "");
            b.color(
                "stroke.color",
                &color_xml(params.color.as_deref(), "accent2"),
                ms,
                "",
            );
        }
        (PresetClass::Emphasis, 16) | (PresetClass::Emphasis, 3) => {
            b.color(
                "style.color",
                &color_xml(params.color.as_deref(), "accent2"),
                ms,
                "",
            );
        }
        (PresetClass::Emphasis, 18) => {
            b.set("style.textDecorationUnderline", "true", ms, "");
        }
        (PresetClass::Emphasis, 10) => {
            b.set("style.fontWeight", "bold", ms, r#" autoRev="1""#);
        }
        (PresetClass::Emphasis, 15) => {
            b.set("style.fontWeight", "bold", ms, "");
        }
        (PresetClass::Emphasis, 34) => {
            b.anim_keys(
                "ppt_y",
                &[
                    (0, "#ppt_y", None),
                    (25_000, "#ppt_y-0.02", None),
                    (75_000, "#ppt_y+0.02", None),
                    (100_000, "#ppt_y", None),
                ],
                ms,
                "",
            );
        }
        (PresetClass::Emphasis, 35) => {
            b.set("style.visibility", "hidden", ms / 2, r#" autoRev="1""#);
        }
        (PresetClass::Emphasis, _) => {
            b.scale(150_000, 150_000, ms, "");
        }
        (PresetClass::Path, id) => {
            let (path, types) = path_markup(id, params);
            let center = path_center(&path);
            b.motion(&path, &types, ms, center);
            accel_decel = r#" accel="50000" decel="50000""#;
        }
    }
    let class = preset.class.effect_class();
    let markup = format!(
        r#"<p:par xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cTn id="0" presetID="{}" presetClass="{}" presetSubtype="{subtype}"{accel_decel} fill="hold" nodeType="{}"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>"#,
        preset.id,
        class.as_str(),
        start.node_type(),
        b.out
    );
    let mut node = XmlNode::parse(&markup).expect("preset markup is well formed");
    // The declarations were only for parsing; the slide declares them.
    node.attrs.clear();
    node
}

fn in_out(subtype: u32) -> &'static str {
    if subtype == 16 {
        "in"
    } else {
        "out"
    }
}

fn opposite(subtype: u32) -> u32 {
    match subtype {
        1 => 4,
        4 => 1,
        2 => 8,
        8 => 2,
        other => other,
    }
}

fn side(subtype: u32) -> &'static str {
    match subtype {
        1 => "Top",
        2 => "Right",
        8 => "Left",
        _ => "Bottom",
    }
}
