//! What a colour scheme (`dgm:colorsDef`) and a SmartArt style
//! (`dgm:styleDef`) give each shape: per style label — `node1` for a
//! bullet's shape, `sibTrans2D1` for the arrow between two, `parChTrans1D1`
//! for a connector line, … — a list of fill, line and text colours and how
//! they spread over the shapes, and the theme's line, fill and effect
//! styles to draw with.
//!
//! A diagram PowerPoint saved carries both parts and is read from them;
//! one jade made names a built-in scheme and style, whose meaning is here.

use std::collections::HashMap;

use crate::xml::{self, Element};

/// How a colour list spreads over a label's shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The list again and again: shape `i` takes colour `i mod len`.
    Repeat,
    /// From the first colour to the last over the shapes.
    Span,
    /// From the first colour to the last over half the shapes, and back.
    Cycle,
}

/// A colour of a list: a theme colour (`accent1`, `lt1`, …) and its
/// transforms (`shade`, `tint`, `alpha`, … in thousandths of a percent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clr {
    pub base: String,
    pub mods: Vec<(String, i64)>,
}

impl Clr {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.to_string(),
            mods: Vec::new(),
        }
    }

    fn with(mut self, name: &str, value: i64) -> Self {
        self.mods.push((name.to_string(), value));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClrList {
    pub method: Method,
    pub colors: Vec<Clr>,
}

impl ClrList {
    fn repeat(colors: Vec<Clr>) -> Self {
        Self {
            method: Method::Repeat,
            colors,
        }
    }

    fn one(color: Clr) -> Self {
        Self::repeat(vec![color])
    }

    fn span(colors: Vec<Clr>) -> Self {
        Self {
            method: Method::Span,
            colors,
        }
    }

    fn cycle(colors: Vec<Clr>) -> Self {
        Self {
            method: Method::Cycle,
            colors,
        }
    }
}

/// A label's colours. An empty list leaves it to the style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelColors {
    pub fill: Option<ClrList>,
    pub line: Option<ClrList>,
    pub text: Option<ClrList>,
}

/// A colour with the offsets that put it at its place in a span, as the
/// drawing writes it: `hueOff` in 60000ths of a degree, the others in
/// thousandths of a percent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub color: Clr,
    pub offsets: [i64; 4],
}

/// The theme's colours by name (`accent1` → `4472C4`), which a span needs
/// to work out the hue, saturation and luminance between its ends.
pub type ThemePalette = HashMap<String, (u8, u8, u8)>;

fn rgb_of(clr: &Clr, theme: &ThemePalette) -> Option<(f64, f64, f64, f64)> {
    let name = match clr.base.as_str() {
        "tx1" => "dk1",
        "bg1" => "lt1",
        "tx2" => "dk2",
        "bg2" => "lt2",
        other => other,
    };
    let (r, g, b) = *theme.get(name)?;
    let (mut r, mut g, mut b, mut a) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        1.0,
    );
    for (name, value) in &clr.mods {
        let v = *value as f64 / 100_000.0;
        match name.as_str() {
            "shade" => {
                r *= v;
                g *= v;
                b *= v;
            }
            "tint" => {
                r = 1.0 - (1.0 - r) * v;
                g = 1.0 - (1.0 - g) * v;
                b = 1.0 - (1.0 - b) * v;
            }
            "alpha" => a = v,
            _ => {}
        }
    }
    Some((r, g, b, a))
}

fn hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    (h, s, l)
}

impl ClrList {
    /// The colour shape `index` of `count` takes.
    pub fn pick(&self, index: usize, count: usize, theme: Option<&ThemePalette>) -> Option<Picked> {
        let first = self.colors.first()?;
        let len = self.colors.len();
        let t = match self.method {
            Method::Repeat => {
                return Some(Picked {
                    color: self.colors[index % len].clone(),
                    offsets: [0; 4],
                })
            }
            Method::Span => {
                if count <= 1 {
                    0.0
                } else {
                    index.min(count - 1) as f64 / (count - 1) as f64
                }
            }
            Method::Cycle => {
                let half = (count / 2).max(1);
                index.min(count.saturating_sub(index)) as f64 / half as f64
            }
        };
        if len == 1 || t <= 0.0 {
            return Some(Picked {
                color: first.clone(),
                offsets: [0; 4],
            });
        }
        // Between which two colours of the list, and how far along.
        let position = t * (len - 1) as f64;
        let segment = (position.floor() as usize).min(len - 2);
        let along = position - segment as f64;
        let (from, to) = (&self.colors[segment], &self.colors[segment + 1]);
        let mut offsets = [0i64; 4];
        let alpha = |clr: &Clr| {
            clr.mods
                .iter()
                .find(|(name, _)| name == "alpha")
                .map_or(100_000.0, |(_, value)| *value as f64)
        };
        // The offsets are from the list's first colour.
        let target_alpha = alpha(from) + (alpha(to) - alpha(from)) * along;
        offsets[3] = (target_alpha - alpha(first)).round() as i64;
        if let Some(theme) = theme {
            if let (Some(a), Some(b), Some(c)) =
                (rgb_of(first, theme), rgb_of(from, theme), rgb_of(to, theme))
            {
                let base = hsl((a.0, a.1, a.2));
                let (h0, s0, l0) = hsl((b.0, b.1, b.2));
                let (h1, s1, l1) = hsl((c.0, c.1, c.2));
                let mut dh = h1 - h0;
                if dh > 180.0 {
                    dh -= 360.0;
                } else if dh < -180.0 {
                    dh += 360.0;
                }
                let h = h0 + dh * along;
                let s = s0 + (s1 - s0) * along;
                let l = l0 + (l1 - l0) * along;
                let mut dh = h - base.0;
                if dh > 180.0 {
                    dh -= 360.0;
                } else if dh < -180.0 {
                    dh += 360.0;
                }
                offsets[0] = (dh * 60_000.0).round() as i64;
                offsets[1] = ((s - base.1) * 100_000.0).round() as i64;
                offsets[2] = ((l - base.2) * 100_000.0).round() as i64;
            }
        }
        Some(Picked {
            color: first.clone(),
            offsets,
        })
    }
}

/// A colour scheme: colours per style label.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorsDef {
    labels: HashMap<String, LabelColors>,
    /// The built-in scheme it is, when it is one.
    builtin: Option<String>,
}

fn read_list(element: &Element) -> Option<ClrList> {
    let colors: Vec<Clr> = element
        .elements()
        .map(|color| Clr {
            base: color
                .attr("val")
                .map(str::to_string)
                .unwrap_or_else(|| color.local().to_string()),
            mods: color
                .elements()
                .filter_map(|m| Some((m.local().to_string(), m.attr("val")?.parse().ok()?)))
                .collect(),
        })
        .collect();
    if colors.is_empty() {
        return None;
    }
    let method = match element.attr("meth") {
        Some("repeat") => Method::Repeat,
        Some("cycle") => Method::Cycle,
        _ => Method::Span,
    };
    Some(ClrList { method, colors })
}

impl ColorsDef {
    /// Reads a `dgm:colorsDef` part.
    pub fn parse(xml: &str) -> Option<Self> {
        let root = xml::parse(xml).ok()?;
        let mut labels = HashMap::new();
        for label in root.children_named("styleLbl") {
            let Some(name) = label.attr("name") else {
                continue;
            };
            labels.insert(
                name.to_string(),
                LabelColors {
                    fill: label.child("fillClrLst").and_then(read_list),
                    line: label.child("linClrLst").and_then(read_list),
                    text: label.child("txFillClrLst").and_then(read_list),
                },
            );
        }
        Some(Self {
            labels,
            builtin: None,
        })
    }

    /// A built-in scheme by id or short name; an unknown one is
    /// 塗りつぶし - アクセント 1, PowerPoint's default.
    pub fn builtin(id: &str) -> Self {
        let short = id
            .split('#')
            .next()
            .unwrap_or(id)
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .to_string();
        Self {
            labels: HashMap::new(),
            builtin: Some(short),
        }
    }

    /// The colours of `label`.
    pub fn label(&self, label: &str) -> LabelColors {
        if let Some(short) = &self.builtin {
            return builtin_label(short, label);
        }
        if let Some(found) = self.labels.get(label) {
            return found.clone();
        }
        // A label the part leaves out: the nearest one it has.
        for fallback in [family_of(label), "node1"] {
            if let Some(found) = self.labels.get(fallback) {
                return found.clone();
            }
        }
        builtin_label("accent1_2", label)
    }
}

fn family_of(label: &str) -> &'static str {
    match label_group(label) {
        Group::Node | Group::Node0 => "node1",
        Group::Venn => "vennNode1",
        Group::Sib2D => "sibTrans2D1",
        Group::Sib1D => "sibTrans1D1",
        Group::ParCh => "parChTrans1D1",
        Group::Acc | Group::TrAcc => "fgAcc1",
        Group::AccFollow => "fgAccFollowNode1",
        Group::Img => "fgImgPlace1",
        Group::Bg => "bgShp",
        Group::Fg => "fgShp",
        Group::SolidAcc => "solidFgAcc1",
        Group::Text => "revTx",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Node,
    Node0,
    Venn,
    Sib2D,
    Sib1D,
    ParCh,
    Acc,
    TrAcc,
    AccFollow,
    Img,
    Bg,
    Fg,
    SolidAcc,
    Text,
}

fn label_group(label: &str) -> Group {
    match label {
        "node0" | "asst0" => Group::Node0,
        "vennNode1" => Group::Venn,
        l if l.starts_with("sibTrans2D") || l.ends_with("SibTrans2D1") => Group::Sib2D,
        l if l.starts_with("sibTrans1D") => Group::Sib1D,
        l if l.starts_with("parChTrans") || l == "callout" => Group::ParCh,
        "trAlignAcc1" => Group::TrAcc,
        l if l.contains("AccFollowNode") => Group::AccFollow,
        l if l.contains("ImgPlace") => Group::Img,
        "bgShp" | "dkBgShp" | "trBgShp" => Group::Bg,
        "fgShp" => Group::Fg,
        l if l.starts_with("solid") => Group::SolidAcc,
        l if l.contains("Acc") => Group::Acc,
        "revTx" => Group::Text,
        _ => Group::Node,
    }
}

/// The families the built-in schemes come in.
enum Family {
    /// 枠線のみ / 濃色 アウトライン: white shapes outlined in `X`.
    Outline { x: String, text: String },
    /// 塗りつぶし: shapes filled with `X`, outlined in `line`.
    Fill { x: String, line: String },
    /// グラデーション範囲: from a shade of `X` to a tint of it.
    Gradient(String),
    /// グラデーション ループ: there and back.
    Loop(String),
    /// 透過グラデーション範囲.
    Transparent(String),
    /// カラフル - アクセント カラー.
    Colorful,
    /// カラフル範囲 - アクセント カラー k から k+1.
    ColorfulRange(u8),
}

fn family(short: &str) -> Family {
    match short {
        "accent0_1" => Family::Outline {
            x: "dk1".into(),
            text: "dk1".into(),
        },
        "accent0_2" => Family::Outline {
            x: "dk2".into(),
            text: "dk2".into(),
        },
        "accent0_3" => Family::Fill {
            x: "dk2".into(),
            line: "lt2".into(),
        },
        "colorful1" => Family::Colorful,
        s if s.starts_with("colorful") => {
            Family::ColorfulRange(s[8..].parse::<u8>().unwrap_or(2).clamp(2, 5))
        }
        s if s.starts_with("accent") && s.len() == 9 => {
            let x = format!("accent{}", &s[6..7]);
            match &s[8..] {
                "1" => Family::Outline {
                    x,
                    text: "dk1".into(),
                },
                "3" => Family::Gradient(x),
                "4" => Family::Loop(x),
                "5" => Family::Transparent(x),
                _ => Family::Fill {
                    x,
                    line: "lt1".into(),
                },
            }
        }
        _ => Family::Fill {
            x: "accent1".into(),
            line: "lt1".into(),
        },
    }
}

fn c(base: &str) -> Clr {
    Clr::new(base)
}

fn colors(fill: ClrList, line: ClrList, text: Option<ClrList>) -> LabelColors {
    LabelColors {
        fill: Some(fill),
        line: Some(line),
        text,
    }
}

/// The colours a built-in scheme gives a label.
pub fn builtin_label(short: &str, label: &str) -> LabelColors {
    use ClrList as L;
    let group = label_group(label);
    let dk1 = || Some(L::one(c("dk1")));
    let tx1 = || Some(L::one(c("tx1")));
    if group == Group::Text {
        return LabelColors {
            fill: None,
            line: None,
            text: tx1(),
        };
    }
    match family(short) {
        Family::Outline { x, text } => {
            let t = || Some(L::one(c(&text)));
            let xs80 = || L::one(c(&x).with("shade", 80_000));
            match group {
                Group::Node | Group::Node0 => colors(L::one(c("lt1")), xs80(), t()),
                Group::Venn => colors(L::one(c("lt1").with("alpha", 50_000)), xs80(), None),
                Group::Sib2D => colors(
                    L::one(c(&x).with("tint", 60_000)),
                    L::one(c(&x).with("tint", 60_000)),
                    t(),
                ),
                Group::Sib1D => colors(L::one(c(&x)), L::one(c(&x)), tx1()),
                Group::ParCh => colors(L::one(c(&x)), L::one(c(&x).with("shade", 60_000)), tx1()),
                Group::Acc | Group::AccFollow => colors(
                    L::one(c(&x).with("alpha", 90_000).with("tint", 40_000)),
                    L::one(c(&x)),
                    t(),
                ),
                Group::TrAcc => colors(
                    L::one(c(&x).with("alpha", 40_000).with("tint", 40_000)),
                    L::one(c(&x)),
                    t(),
                ),
                Group::Img => colors(
                    L::one(c(&x).with("tint", 40_000)),
                    xs80(),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(L::one(c(&x).with("tint", 40_000)), L::one(c(&x)), dk1()),
                Group::Fg => colors(L::one(c(&x).with("tint", 60_000)), L::one(c("lt1")), t()),
                Group::SolidAcc => colors(L::one(c("lt1")), L::one(c(&x)), t()),
                Group::Text => unreachable!(),
            }
        }
        Family::Fill { x, line } => match group {
            Group::Node | Group::Node0 => colors(L::one(c(&x)), L::one(c(&line)), None),
            Group::Venn => colors(L::one(c(&x).with("alpha", 50_000)), L::one(c(&line)), None),
            Group::Sib2D => colors(
                L::one(c(&x).with("tint", 60_000)),
                L::one(c(&x).with("tint", 60_000)),
                None,
            ),
            Group::Sib1D => colors(L::one(c(&x)), L::one(c(&x)), tx1()),
            Group::ParCh => colors(L::one(c(&x)), L::one(c(&x).with("shade", 60_000)), tx1()),
            Group::Acc => colors(
                L::one(c(if line == "lt2" { "lt2" } else { "lt1" }).with("alpha", 90_000)),
                L::one(c(&x)),
                dk1(),
            ),
            Group::TrAcc => colors(L::one(c("lt1").with("alpha", 40_000)), L::one(c(&x)), dk1()),
            Group::AccFollow => colors(
                L::one(c(&x).with("alpha", 90_000).with("tint", 40_000)),
                L::one(c(&x).with("alpha", 90_000)),
                dk1(),
            ),
            Group::Img => colors(
                L::one(c(&x).with("tint", 50_000)),
                L::one(c(&line)),
                Some(L::one(c(&line))),
            ),
            Group::Bg => colors(L::one(c(&x).with("tint", 40_000)), L::one(c(&x)), dk1()),
            Group::Fg => colors(L::one(c(&x).with("tint", 60_000)), L::one(c(&line)), None),
            Group::SolidAcc => colors(L::one(c("lt1")), L::one(c(&x)), dk1()),
            Group::Text => unreachable!(),
        },
        Family::Gradient(x) => {
            let range = || vec![c(&x).with("shade", 80_000), c(&x).with("tint", 70_000)];
            match group {
                Group::Node => colors(L::span(range()), L::one(c("lt1")), None),
                Group::Node0 => colors(L::one(c(&x).with("shade", 80_000)), L::one(c("lt1")), None),
                Group::Venn => colors(
                    L::span(vec![
                        c(&x).with("shade", 80_000).with("alpha", 50_000),
                        c(&x).with("tint", 70_000).with("alpha", 50_000),
                    ]),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Sib2D | Group::Sib1D => {
                    let l = || {
                        L::span(vec![
                            c(&x).with("shade", 90_000),
                            c(&x).with("tint", 70_000),
                        ])
                    };
                    colors(l(), l(), None)
                }
                Group::ParCh => colors(
                    L::one(c(&x).with("shade", 80_000)),
                    L::one(c(&x).with("shade", 80_000)),
                    tx1(),
                ),
                Group::Acc | Group::SolidAcc | Group::AccFollow => colors(
                    L::one(c("lt1").with("alpha", 90_000)),
                    L::span(range()),
                    dk1(),
                ),
                Group::TrAcc => {
                    colors(L::one(c("lt1").with("alpha", 40_000)), L::one(c(&x)), dk1())
                }
                Group::Img => colors(
                    L::span(vec![c(&x).with("tint", 50_000), c(&x).with("tint", 20_000)]),
                    L::one(c("lt1")),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(L::one(c(&x).with("tint", 40_000)), L::one(c("dk1")), dk1()),
                Group::Fg => colors(L::one(c(&x).with("tint", 60_000)), L::one(c("lt1")), None),
                Group::Text => unreachable!(),
            }
        }
        Family::Loop(x) => {
            let range = || vec![c(&x).with("shade", 50_000), c(&x).with("tint", 55_000)];
            match group {
                Group::Node => colors(L::cycle(range()), L::one(c("lt1")), None),
                Group::Node0 => colors(L::one(c(&x).with("shade", 60_000)), L::one(c("lt1")), None),
                Group::Venn => colors(
                    L::cycle(vec![
                        c(&x).with("shade", 80_000).with("alpha", 50_000),
                        c(&x).with("tint", 50_000).with("alpha", 50_000),
                    ]),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Sib2D | Group::Sib1D => {
                    let l = || {
                        L::cycle(vec![
                            c(&x).with("shade", 90_000),
                            c(&x).with("tint", 50_000),
                        ])
                    };
                    colors(l(), l(), None)
                }
                Group::ParCh => colors(
                    L::one(c(&x).with("shade", 80_000)),
                    L::one(c(&x).with("shade", 80_000)),
                    tx1(),
                ),
                Group::Acc | Group::SolidAcc | Group::AccFollow => colors(
                    L::one(c("lt1").with("alpha", 90_000)),
                    L::cycle(range()),
                    dk1(),
                ),
                Group::TrAcc => {
                    colors(L::one(c("lt1").with("alpha", 55_000)), L::one(c(&x)), dk1())
                }
                Group::Img => colors(
                    L::span(vec![c(&x).with("tint", 50_000), c(&x).with("tint", 55_000)]),
                    L::one(c("lt1")),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(L::one(c(&x).with("tint", 55_000)), L::one(c("dk1")), dk1()),
                Group::Fg => colors(L::one(c(&x).with("tint", 60_000)), L::one(c("lt1")), None),
                Group::Text => unreachable!(),
            }
        }
        Family::Transparent(x) => {
            let range = || vec![c(&x).with("alpha", 90_000), c(&x).with("alpha", 50_000)];
            match group {
                Group::Node => colors(L::span(range()), L::one(c("lt1")), None),
                Group::Node0 => colors(L::one(c(&x).with("alpha", 80_000)), L::one(c("lt1")), None),
                Group::Venn => colors(
                    L::span(vec![
                        c(&x).with("shade", 80_000).with("alpha", 50_000),
                        c(&x).with("alpha", 80_000),
                    ]),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Sib2D | Group::Sib1D => {
                    let l = || {
                        L::span(vec![
                            c(&x).with("shade", 90_000),
                            c(&x).with("tint", 50_000),
                        ])
                    };
                    colors(l(), l(), None)
                }
                Group::ParCh => colors(
                    L::one(c(&x).with("shade", 80_000)),
                    L::one(c(&x).with("shade", 80_000)),
                    tx1(),
                ),
                Group::Acc | Group::SolidAcc | Group::AccFollow => colors(
                    L::one(c("lt1").with("alpha", 90_000)),
                    L::span(range()),
                    dk1(),
                ),
                Group::TrAcc => colors(
                    L::one(c("lt1").with("alpha", 40_000)),
                    L::span(range()),
                    dk1(),
                ),
                Group::Img => colors(
                    L::span(vec![
                        c(&x).with("tint", 50_000).with("alpha", 90_000),
                        c(&x).with("tint", 20_000).with("alpha", 50_000),
                    ]),
                    L::one(c("lt1")),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(L::one(c(&x).with("tint", 40_000)), L::one(c("dk1")), dk1()),
                Group::Fg => colors(L::one(c(&x).with("tint", 60_000)), L::one(c("lt1")), None),
                Group::Text => unreachable!(),
            }
        }
        Family::Colorful => {
            let all = |f: &dyn Fn(Clr) -> Clr| {
                (2..=6)
                    .map(|i| f(c(&format!("accent{i}"))))
                    .collect::<Vec<_>>()
            };
            match group {
                Group::Node => colors(L::repeat(all(&|x| x)), L::one(c("lt1")), None),
                Group::Node0 => colors(L::one(c("accent1")), L::one(c("lt1")), None),
                Group::Venn => colors(
                    L::repeat(all(&|x| x.with("alpha", 50_000))),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Sib2D => colors(L::repeat(all(&|x| x)), L::one(c("lt1")), None),
                Group::Sib1D => colors(L::repeat(all(&|x| x)), L::repeat(all(&|x| x)), tx1()),
                Group::ParCh => colors(L::one(c("accent2")), L::one(c("accent1")), tx1()),
                Group::Acc | Group::SolidAcc | Group::AccFollow => colors(
                    L::one(c("lt1").with("alpha", 90_000)),
                    L::repeat(all(&|x| x)),
                    dk1(),
                ),
                Group::TrAcc => colors(
                    L::one(c("lt1").with("alpha", 40_000)),
                    L::one(c("accent1")),
                    dk1(),
                ),
                Group::Img => colors(
                    L::repeat(all(&|x| x.with("tint", 50_000))),
                    L::one(c("lt1")),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(
                    L::one(c("accent2").with("tint", 40_000)),
                    L::one(c("dk1")),
                    dk1(),
                ),
                Group::Fg => colors(
                    L::one(c("accent2").with("tint", 60_000)),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Text => unreachable!(),
            }
        }
        Family::ColorfulRange(k) => {
            let a = format!("accent{k}");
            let b = format!("accent{}", k + 1);
            let before = format!("accent{}", k - 1);
            let pair = |f: &dyn Fn(Clr) -> Clr| vec![f(c(&a)), f(c(&b))];
            match group {
                Group::Node => colors(L::span(pair(&|x| x)), L::one(c("lt1")), None),
                Group::Node0 => colors(L::one(c(&before)), L::one(c("lt1")), None),
                Group::Venn => colors(
                    L::span(pair(&|x| x.with("alpha", 50_000))),
                    L::one(c("lt1")),
                    None,
                ),
                Group::Sib2D => colors(L::span(pair(&|x| x)), L::one(c("lt1")), None),
                Group::Sib1D => colors(L::span(pair(&|x| x)), L::span(pair(&|x| x)), tx1()),
                Group::ParCh => colors(L::one(c(&a)), L::one(c(&a)), tx1()),
                Group::Acc | Group::SolidAcc | Group::AccFollow => colors(
                    L::one(c("lt1").with("alpha", 90_000)),
                    L::span(pair(&|x| x)),
                    dk1(),
                ),
                Group::TrAcc => {
                    colors(L::one(c("lt1").with("alpha", 40_000)), L::one(c(&a)), dk1())
                }
                Group::Img => colors(
                    L::span(pair(&|x| x.with("tint", 50_000))),
                    L::one(c("lt1")),
                    Some(L::one(c("lt1"))),
                ),
                Group::Bg => colors(L::one(c(&a).with("tint", 40_000)), L::one(c("dk1")), dk1()),
                Group::Fg => colors(L::one(c(&a).with("tint", 60_000)), L::one(c("lt1")), None),
                Group::Text => unreachable!(),
            }
        }
    }
}

/// What a SmartArt style gives a label: the theme's line, fill and effect
/// styles by index (`lnRef`, `fillRef`, `effectRef`), and whether it is
/// raised in 3-D.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelStyle {
    pub line: u32,
    pub fill: u32,
    pub effect: u32,
    /// A bevel's width in EMU, for the 3-D styles.
    pub bevel: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleDef {
    labels: HashMap<String, LabelStyle>,
    builtin: Option<String>,
}

impl StyleDef {
    pub fn parse(xml: &str) -> Option<Self> {
        let root = xml::parse(xml).ok()?;
        let mut labels = HashMap::new();
        for label in root.children_named("styleLbl") {
            let Some(name) = label.attr("name") else {
                continue;
            };
            let Some(style) = label.child("style") else {
                continue;
            };
            let idx = |name: &str| {
                style
                    .child(name)
                    .and_then(|r| r.attr("idx"))
                    .and_then(|i| i.parse().ok())
                    .unwrap_or(0)
            };
            let bevel = label
                .child("sp3d")
                .and_then(|sp| sp.child("bevelT"))
                .map(|bevel| {
                    bevel
                        .attr("w")
                        .and_then(|w| w.parse().ok())
                        .unwrap_or(76_200)
                });
            labels.insert(
                name.to_string(),
                LabelStyle {
                    line: idx("lnRef"),
                    fill: idx("fillRef"),
                    effect: idx("effectRef"),
                    bevel,
                },
            );
        }
        Some(Self {
            labels,
            builtin: None,
        })
    }

    pub fn builtin(id: &str) -> Self {
        let short = id
            .split('#')
            .next()
            .unwrap_or(id)
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .to_string();
        Self {
            labels: HashMap::new(),
            builtin: Some(short),
        }
    }

    pub fn label(&self, label: &str) -> LabelStyle {
        if let Some(short) = &self.builtin {
            return builtin_style(short, label);
        }
        if let Some(found) = self.labels.get(label) {
            return *found;
        }
        if let Some(found) = self.labels.get(family_of(label)) {
            return *found;
        }
        builtin_style("simple1", label)
    }
}

/// The line, fill and effect indices of the built-in styles, per kind of
/// label: (bullets, arrows, connectors, accents, backgrounds).
pub fn builtin_style(short: &str, label: &str) -> LabelStyle {
    let s = |line, fill, effect| LabelStyle {
        line,
        fill,
        effect,
        bevel: None,
    };
    let b = |line, fill, effect, bevel| LabelStyle {
        line,
        fill,
        effect,
        bevel: Some(bevel),
    };
    let group = label_group(label);
    let kind = match group {
        Group::Node | Group::Node0 | Group::Venn | Group::Img => 0,
        Group::Sib2D => 1,
        Group::Sib1D | Group::ParCh => 2,
        Group::Acc | Group::TrAcc | Group::AccFollow | Group::SolidAcc => 3,
        Group::Bg | Group::Fg => 4,
        Group::Text => 5,
    };
    if kind == 5 {
        return s(0, 0, 0);
    }
    let venn = group == Group::Venn;
    match (short, kind) {
        ("simple2", 0) => s(3, 1, if venn { 0 } else { 1 }),
        ("simple2", 1) => s(0, 1, 1),
        ("simple3", 0) => b(0, 2, if venn { 0 } else { 1 }, 8_200),
        ("simple3", 1) => s(0, 2, 1),
        ("simple3", 3) => s(1, 1, 0),
        ("simple3", 4) => s(0, 1, 1),
        ("simple4", 0) => s(0, 3, if venn { 0 } else { 2 }),
        ("simple4", 1) => s(0, 3, 2),
        ("simple4", 2) => s(1, 0, 0),
        ("simple4", 3) => s(1, 1, 0),
        ("simple4", 4) => s(0, 1, 2),
        ("simple5", 0) | ("simple5", 1) => s(0, 3, 3),
        ("simple5", 3) => s(1, 1, 2),
        ("simple5", 4) => s(0, 1, 2),
        ("3d1", 0) => b(
            0,
            if venn { 1 } else { 3 },
            if venn { 1 } else { 2 },
            120_900,
        ),
        ("3d1", 1) => b(0, 3, 2, 50_800),
        ("3d2", 0) => b(0, 3, 2, 127_000),
        ("3d2", 1) => b(0, 1, 2, 25_400),
        ("3d3", 0) => b(0, 1, if venn { 0 } else { 2 }, 88_900),
        ("3d4", 0) => b(0, 1, 0, 127_000),
        ("3d6", 0) => b(0, 1, 2, 50_800),
        ("3d6", 1) => b(1, 1, 2, 25_400),
        ("3d7", 0) => b(0, 1, if venn { 0 } else { 1 }, 101_600),
        ("3d7", 1) => b(0, 1, 2, 40_600),
        ("3d9", 0) => b(0, 1, 2, 165_100),
        (s3, 0) if s3.starts_with("3d") => s(0, 1, 0),
        (s3, 1) if s3.starts_with("3d") => s(0, 1, 0),
        (s3, 3) if s3.starts_with("3d") => s(1, 1, 0),
        (s3, 4) if s3.starts_with("3d") => s(0, 1, 0),
        (_, 0) => s(2, 1, 0),
        (_, 1) => s(0, 1, 0),
        (_, 2) => s(2, 0, 0),
        (_, 3) => s(2, 1, 0),
        _ => s(0, 1, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colourful_repeats_the_accents_and_ranges_span() {
        let colorful = ColorsDef::builtin("colorful1").label("node1");
        let fill = colorful.fill.unwrap();
        assert_eq!(fill.pick(0, 7, None).unwrap().color.base, "accent2");
        assert_eq!(fill.pick(5, 7, None).unwrap().color.base, "accent2");
        assert_eq!(fill.pick(6, 7, None).unwrap().color.base, "accent3");

        let transparent = ColorsDef::builtin("accent1_5").label("node1").fill.unwrap();
        // 90% to 50% opacity over seven shapes: the last is 40% off.
        assert_eq!(transparent.pick(6, 7, None).unwrap().offsets[3], -40_000);
        assert_eq!(transparent.pick(1, 7, None).unwrap().offsets[3], -6_667);

        let looped = ColorsDef::builtin("accent1_4").label("node1").fill.unwrap();
        let mut theme = ThemePalette::new();
        theme.insert("accent1".into(), (0x44, 0x72, 0xC4));
        let lum = |i| looped.pick(i, 7, Some(&theme)).unwrap().offsets[2];
        assert_eq!(lum(0), 0);
        assert!(lum(3) > lum(1));
        assert_eq!(lum(4), lum(3));
        assert_eq!(lum(6), lum(1));
    }

    #[test]
    fn parses_a_colors_part() {
        let xml = r#"<dgm:colorsDef xmlns:dgm="d" xmlns:a="a" uniqueId="x"><dgm:styleLbl name="node1"><dgm:fillClrLst meth="repeat"><a:schemeClr val="accent2"/></dgm:fillClrLst><dgm:linClrLst meth="repeat"><a:schemeClr val="lt1"/></dgm:linClrLst></dgm:styleLbl></dgm:colorsDef>"#;
        let def = ColorsDef::parse(xml).unwrap();
        let node = def.label("node1");
        assert_eq!(node.fill.unwrap().colors[0].base, "accent2");
        // Unknown labels fall back to node1.
        assert_eq!(def.label("asst1").fill.unwrap().colors[0].base, "accent2");
        let style = StyleDef::builtin("simple4").label("node1");
        assert_eq!((style.fill, style.effect), (3, 2));
    }
}
