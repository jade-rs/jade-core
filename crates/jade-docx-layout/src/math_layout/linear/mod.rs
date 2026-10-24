//! Word's linear formats for equations: UnicodeMath (Unicode Technical
//! Note #28, the format Word builds up as you type) and LaTeX. Both read into
//! one structure tree and are written as OMML; any OMML is written back out
//! in either, which is Word's 2D/linear switch.
use jade_docx_reader::document::{MathElement, MathElementKind};

pub mod autocorrect;
pub mod latex;
pub mod unicode;

pub use autocorrect::{apply_math_autocorrect, math_autocorrect, MATH_AUTOCORRECT};

/// Which linear format an equation is typed and shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinearFormat {
    #[default]
    UnicodeMath,
    LaTeX,
}

/// Reads `text` in `format` into the content of an `m:oMath`.
pub fn parse_linear(text: &str, format: LinearFormat) -> Vec<MathElement> {
    let nodes = match format {
        LinearFormat::UnicodeMath => unicode::parse(text),
        LinearFormat::LaTeX => latex::parse_finished(text),
    };
    to_omml(&nodes)
}

/// Writes math elements (an `m:oMath`'s content, or the `m:oMath` itself) in
/// `format`.
pub fn to_linear(elements: &[MathElement], format: LinearFormat) -> String {
    match format {
        LinearFormat::UnicodeMath => unicode::write(elements),
        LinearFormat::LaTeX => latex::write(elements),
    }
}

/// An `m:oMath` holding `text` read in `format`, ready for a document.
pub fn linear_to_omath(text: &str, format: LinearFormat) -> MathElement {
    MathElement::new(MathElementKind::Math, parse_linear(text, format))
}

/// The kind of an `m:f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FractionKind {
    Bar,
    Skewed,
    Linear,
    NoBar,
}

/// The structure both linear formats read into.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Math text: letters italic by default.
    Text(String),
    /// Text in a math style: `sty` p/b/i/bi, `scr` an alphabet.
    Styled {
        text: String,
        sty: &'static str,
        scr: Option<&'static str>,
    },
    /// Normal (non-math) text, `m:nor`.
    Normal(String),
    Fraction {
        kind: FractionKind,
        num: Vec<Node>,
        den: Vec<Node>,
    },
    Script {
        base: Vec<Node>,
        sub: Option<Vec<Node>>,
        sup: Option<Vec<Node>>,
    },
    PreScript {
        sub: Vec<Node>,
        sup: Vec<Node>,
        base: Vec<Node>,
    },
    Radical {
        degree: Option<Vec<Node>>,
        base: Vec<Node>,
    },
    Nary {
        chr: char,
        sub: Option<Vec<Node>>,
        sup: Option<Vec<Node>>,
        body: Vec<Node>,
        /// Limits over and under (`Some(true)`), beside (`Some(false)`), or
        /// the operator's default.
        over: Option<bool>,
    },
    Delimiter {
        open: String,
        close: String,
        separator: Option<String>,
        items: Vec<Vec<Node>>,
    },
    Function {
        name: Vec<Node>,
        arg: Vec<Node>,
    },
    Matrix(Vec<Vec<Vec<Node>>>),
    EquationArray(Vec<Vec<Node>>),
    Accent {
        chr: char,
        base: Vec<Node>,
    },
    Bar {
        top: bool,
        base: Vec<Node>,
    },
    GroupChar {
        chr: char,
        top: bool,
        base: Vec<Node>,
    },
    Limit {
        lower: bool,
        base: Vec<Node>,
        lim: Vec<Node>,
    },
    BoxGroup(Vec<Node>),
    BorderBox(Vec<Node>),
    Phantom(Vec<Node>),
    /// Grouping that leaves no mark (`〖…〗`, `{…}`).
    Group(Vec<Node>),
}

/// The function names Word sets upright and applies to what follows.
pub const FUNCTION_NAMES: &[&str] = &[
    "arccos", "arccot", "arccsc", "arcsec", "arcsin", "arctan", "arg", "cos", "cosh", "cot",
    "coth", "csc", "csch", "def", "deg", "det", "dim", "exp", "gcd", "hom", "inf", "ker", "lg",
    "lim", "liminf", "limsup", "ln", "log", "max", "min", "mod", "Pr", "sec", "sech", "sin",
    "sinh", "sup", "tan", "tanh",
];

pub fn is_function_name(name: &str) -> bool {
    FUNCTION_NAMES.contains(&name)
}

/// The characters n-ary operators are.
pub fn is_nary(ch: char) -> bool {
    matches!(
        ch,
        '∑' | '∏'
            | '∐'
            | '∫'
            | '∬'
            | '∭'
            | '⨌'
            | '∮'
            | '∯'
            | '∰'
            | '∱'
            | '∲'
            | '∳'
            | '⋀'
            | '⋁'
            | '⋂'
            | '⋃'
            | '⨀'
            | '⨁'
            | '⨂'
            | '⨄'
            | '⨆'
    )
}

pub fn is_integral(ch: char) -> bool {
    matches!(
        ch,
        '∫' | '∬' | '∭' | '⨌' | '∮' | '∯' | '∰' | '∱' | '∲' | '∳'
    )
}

fn word_run_properties() -> MathElement {
    let mut fonts = MathElement::tagged("rFonts", Vec::new());
    fonts.kind = MathElementKind::Unknown;
    fonts.namespace = Some("w".into());
    fonts
        .attributes
        .insert("ascii".into(), "Cambria Math".into());
    fonts
        .attributes
        .insert("hAnsi".into(), "Cambria Math".into());
    let mut pr = MathElement::tagged("rPr", vec![fonts]);
    pr.kind = MathElementKind::Unknown;
    pr.namespace = Some("w".into());
    pr
}

/// An `m:r` of math text, in Cambria Math as Word writes it.
pub fn math_run(text: &str, math_properties: Vec<MathElement>) -> MathElement {
    let mut children = Vec::new();
    if !math_properties.is_empty() {
        let mut pr = MathElement::tagged("rPr", math_properties);
        pr.kind = MathElementKind::Unknown;
        children.push(pr);
    }
    children.push(word_run_properties());
    children.push(MathElement::text_node(text));
    MathElement::new(MathElementKind::Run, children)
}

fn property(tag: &str, value: &str) -> MathElement {
    MathElement::property(tag, value)
}

fn properties(tag: &str, children: Vec<MathElement>) -> MathElement {
    let mut pr = MathElement::tagged(tag, children);
    pr.kind = MathElementKind::Unknown;
    pr
}

fn slot(kind: MathElementKind, nodes: &[Node]) -> MathElement {
    MathElement::new(kind, to_omml(nodes))
}

/// The OMML a structure tree is written as: the children of an `m:oMath`
/// or of an argument.
pub fn to_omml(nodes: &[Node]) -> Vec<MathElement> {
    use MathElementKind as K;
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Text(text) => {
                if !text.is_empty() {
                    out.push(math_run(text, Vec::new()));
                }
            }
            Node::Styled { text, sty, scr } => {
                let mut pr = vec![property("sty", sty)];
                if let Some(scr) = scr {
                    pr.insert(0, property("scr", scr));
                }
                out.push(math_run(text, pr));
            }
            Node::Normal(text) => {
                let mut nor = MathElement::tagged("nor", Vec::new());
                nor.kind = K::Unknown;
                out.push(math_run(text, vec![nor]));
            }
            Node::Fraction { kind, num, den } => {
                let mut children = Vec::new();
                let value = match kind {
                    FractionKind::Bar => None,
                    FractionKind::Skewed => Some("skw"),
                    FractionKind::Linear => Some("lin"),
                    FractionKind::NoBar => Some("noBar"),
                };
                if let Some(value) = value {
                    children.push(properties("fPr", vec![property("type", value)]));
                }
                children.push(slot(K::Numerator, num));
                children.push(slot(K::Denominator, den));
                out.push(MathElement::new(K::Fraction, children));
            }
            Node::Script { base, sub, sup } => {
                let (kind, mut children) = match (sub, sup) {
                    (Some(_), Some(_)) => (K::SubSup, vec![slot(K::Base, base)]),
                    (Some(_), None) => (K::Subscript, vec![slot(K::Base, base)]),
                    (None, Some(_)) => (K::Superscript, vec![slot(K::Base, base)]),
                    (None, None) => {
                        out.extend(to_omml(base));
                        continue;
                    }
                };
                if let Some(sub) = sub {
                    children.push(slot(K::SubArgument, sub));
                }
                if let Some(sup) = sup {
                    children.push(slot(K::SuperArgument, sup));
                }
                out.push(MathElement::new(kind, children));
            }
            Node::PreScript { sub, sup, base } => out.push(MathElement::new(
                K::PreSubSuper,
                vec![
                    slot(K::SubArgument, sub),
                    slot(K::SuperArgument, sup),
                    slot(K::Base, base),
                ],
            )),
            Node::Radical { degree, base } => {
                let mut children = Vec::new();
                match degree {
                    Some(degree) => children.push(slot(K::Degree, degree)),
                    None => {
                        children.push(properties("radPr", vec![property("degHide", "1")]));
                        children.push(MathElement::new(K::Degree, Vec::new()));
                    }
                }
                children.push(slot(K::Base, base));
                out.push(MathElement::new(K::Radical, children));
            }
            Node::Nary {
                chr,
                sub,
                sup,
                body,
                over,
            } => {
                let mut pr = Vec::new();
                if *chr != '∫' {
                    pr.push(property("chr", &chr.to_string()));
                }
                let over = over.unwrap_or(!is_integral(*chr));
                pr.push(property("limLoc", if over { "undOvr" } else { "subSup" }));
                if sub.is_none() {
                    pr.push(property("subHide", "1"));
                }
                if sup.is_none() {
                    pr.push(property("supHide", "1"));
                }
                out.push(MathElement::new(
                    K::Nary,
                    vec![
                        properties("naryPr", pr),
                        slot(K::SubArgument, sub.as_deref().unwrap_or_default()),
                        slot(K::SuperArgument, sup.as_deref().unwrap_or_default()),
                        slot(K::Base, body),
                    ],
                ));
            }
            Node::Delimiter {
                open,
                close,
                separator,
                items,
            } => {
                let mut pr = Vec::new();
                if open != "(" {
                    pr.push(property("begChr", open));
                }
                if let Some(separator) = separator {
                    if separator != "|" {
                        pr.push(property("sepChr", separator));
                    }
                }
                if close != ")" {
                    pr.push(property("endChr", close));
                }
                let mut children = vec![properties("dPr", pr)];
                for item in items {
                    children.push(slot(K::Base, item));
                }
                out.push(MathElement::new(K::Delimiter, children));
            }
            Node::Function { name, arg } => {
                let name = name
                    .iter()
                    .map(|n| match n {
                        Node::Text(text) => Node::Styled {
                            text: text.clone(),
                            sty: "p",
                            scr: None,
                        },
                        other => other.clone(),
                    })
                    .collect::<Vec<_>>();
                out.push(MathElement::new(
                    K::Function,
                    vec![slot(K::FunctionName, &name), slot(K::Base, arg)],
                ));
            }
            Node::Matrix(rows) => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let mut children = vec![properties(
                    "mPr",
                    vec![properties(
                        "mcs",
                        vec![{
                            let mut mc = MathElement::tagged(
                                "mc",
                                vec![properties(
                                    "mcPr",
                                    vec![
                                        property("count", &columns.to_string()),
                                        property("mcJc", "center"),
                                    ],
                                )],
                            );
                            mc.kind = K::Unknown;
                            mc
                        }],
                    )],
                )];
                for row in rows {
                    let mut cells: Vec<MathElement> =
                        row.iter().map(|cell| slot(K::Base, cell)).collect();
                    while cells.len() < columns {
                        cells.push(MathElement::new(K::Base, Vec::new()));
                    }
                    children.push(MathElement::new(K::MatrixRow, cells));
                }
                out.push(MathElement::new(K::Matrix, children));
            }
            Node::EquationArray(rows) => {
                let children = rows.iter().map(|row| slot(K::Base, row)).collect();
                out.push(MathElement::new(K::EquationArray, children));
            }
            Node::Accent { chr, base } => out.push(MathElement::new(
                K::Accent,
                vec![
                    properties("accPr", vec![property("chr", &chr.to_string())]),
                    slot(K::Base, base),
                ],
            )),
            Node::Bar { top, base } => out.push(MathElement::new(
                K::Bar,
                vec![
                    properties(
                        "barPr",
                        vec![property("pos", if *top { "top" } else { "bot" })],
                    ),
                    slot(K::Base, base),
                ],
            )),
            Node::GroupChar { chr, top, base } => out.push(MathElement::new(
                K::GroupChar,
                vec![
                    properties(
                        "groupChrPr",
                        vec![
                            property("chr", &chr.to_string()),
                            property("pos", if *top { "top" } else { "bot" }),
                            property("vertJc", if *top { "bot" } else { "top" }),
                        ],
                    ),
                    slot(K::Base, base),
                ],
            )),
            Node::Limit { lower, base, lim } => out.push(MathElement::new(
                if *lower { K::LimitLower } else { K::LimitUpper },
                vec![slot(K::Base, base), slot(K::Limit, lim)],
            )),
            Node::BoxGroup(base) => {
                out.push(MathElement::new(K::Box, vec![slot(K::Base, base)]));
            }
            Node::BorderBox(base) => {
                out.push(MathElement::new(K::BorderBox, vec![slot(K::Base, base)]));
            }
            Node::Phantom(base) => out.push(MathElement::new(
                K::Phantom,
                vec![
                    properties("phantPr", vec![property("show", "0")]),
                    slot(K::Base, base),
                ],
            )),
            Node::Group(nodes) => out.extend(to_omml(nodes)),
        }
    }
    merge_runs(out)
}

/// Adjacent plain math runs become one run, as Word writes them.
fn merge_runs(elements: Vec<MathElement>) -> Vec<MathElement> {
    let mut out: Vec<MathElement> = Vec::new();
    for element in elements {
        if let (Some(last), true) = (out.last_mut(), is_plain_run(&element)) {
            if is_plain_run(last) {
                let text = run_text(&element);
                if let Some(t) = last
                    .children
                    .iter_mut()
                    .find(|c| c.kind == MathElementKind::Text)
                {
                    t.text.get_or_insert_with(String::new).push_str(&text);
                }
                continue;
            }
        }
        out.push(element);
    }
    out
}

fn is_plain_run(element: &MathElement) -> bool {
    element.kind == MathElementKind::Run
        && !element
            .children
            .iter()
            .any(|c| c.local_name() == "rPr" && c.namespace.is_none())
}

fn run_text(element: &MathElement) -> String {
    element
        .children
        .iter()
        .filter(|c| c.kind == MathElementKind::Text)
        .filter_map(|c| c.text.clone())
        .collect()
}

/// Shared helpers for the writers.
pub(crate) mod read {
    use super::*;

    pub(crate) fn property<'a>(element: &'a MathElement, pr: &str, name: &str) -> Option<&'a str> {
        element
            .children
            .iter()
            .find(|c| c.local_name() == pr)?
            .children
            .iter()
            .find(|c| c.local_name() == name)?
            .attributes
            .get("val")
            .map(String::as_str)
    }

    pub(crate) fn flag(element: &MathElement, pr: &str, name: &str) -> bool {
        element
            .children
            .iter()
            .find(|c| c.local_name() == pr)
            .and_then(|p| p.children.iter().find(|c| c.local_name() == name))
            .is_some_and(|p| {
                p.attributes
                    .get("val")
                    .is_none_or(|v| matches!(v.as_str(), "1" | "true" | "on"))
            })
    }

    pub(crate) fn child(element: &MathElement, kind: MathElementKind) -> Option<&MathElement> {
        element.children.iter().find(|c| c.kind == kind)
    }

    pub(crate) fn children(
        element: &MathElement,
        kind: MathElementKind,
    ) -> impl Iterator<Item = &MathElement> {
        element.children.iter().filter(move |c| c.kind == kind)
    }

    /// The run's math style: `sty`, `scr`, `nor`.
    pub(crate) fn run_style(run: &MathElement) -> (Option<String>, Option<String>, bool) {
        let mut sty = None;
        let mut scr = None;
        let mut nor = false;
        for pr in run
            .children
            .iter()
            .filter(|c| c.local_name() == "rPr" && c.namespace.is_none())
        {
            for p in &pr.children {
                match p.local_name() {
                    "sty" => sty = p.attributes.get("val").cloned(),
                    "scr" => scr = p.attributes.get("val").cloned(),
                    "nor" => nor = true,
                    _ => {}
                }
            }
        }
        (sty, scr, nor)
    }

    pub(crate) fn text_of(run: &MathElement) -> String {
        super::run_text(run)
    }

    pub(crate) fn is_content(element: &MathElement) -> bool {
        !(element.local_name().ends_with("Pr") || element.local_name() == "ctrlPr")
    }
}

#[cfg(test)]
mod tests;
