//! LaTeX, Word's second linear format (Word 2016 for Windows and Mac 16.x
//! accept it in the Equation tab's Convert group): `\frac{a}{b}`,
//! `\sqrt[n]{x}`, `x^{2}`, `\sum_{i=1}^{n}`, `\left( … \right)`,
//! `\begin{matrix} … \end{matrix}` and the rest of the common math subset.
use super::read::{child, children, flag, is_content, property, run_style, text_of};
use super::{autocorrect::math_autocorrect, is_function_name, is_nary, FractionKind, Node};
use jade_docx_reader::document::{MathElement, MathElementKind};

/// Reads LaTeX into structure.
pub fn parse(text: &str) -> Vec<Node> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    parser.expression(&[])
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

/// LaTeX names Word maps differently from its Math AutoCorrect list.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "cdot" => "⋅",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "to" | "rightarrow" => "→",
        "infty" => "∞",
        "lbrace" | "{" => "{",
        "rbrace" | "}" => "}",
        "mid" => "∣",
        "lvert" | "rvert" | "vert" => "|",
        "lVert" | "rVert" | "Vert" | "|" => "‖",
        "langle" => "⟨",
        "rangle" => "⟩",
        "quad" => "\u{2003}",
        "qquad" => "\u{2003}\u{2003}",
        "," => "\u{2006}",
        ":" | ">" => "\u{205F}",
        ";" => "\u{2005}",
        "!" => "",
        " " => " ",
        "%" => "%",
        "$" => "$",
        "&" => "&",
        "#" => "#",
        "_" => "_",
        "lnot" | "neg" => "¬",
        "land" => "∧",
        "lor" => "∨",
        "iff" => "⟺",
        "implies" => "⟹",
        "dagger" => "†",
        "ddagger" => "‡",
        "colon" => ":",
        _ => return math_autocorrect(name),
    })
}

fn accent_for(name: &str) -> Option<char> {
    Some(match name {
        "hat" | "widehat" => '\u{0302}',
        "tilde" | "widetilde" => '\u{0303}',
        "bar" => '\u{0305}',
        "vec" | "overrightarrow" => '\u{20D7}',
        "overleftarrow" => '\u{20D6}',
        "overleftrightarrow" => '\u{20E1}',
        "dot" => '\u{0307}',
        "ddot" => '\u{0308}',
        "dddot" => '\u{20DB}',
        "check" => '\u{030C}',
        "breve" => '\u{0306}',
        "acute" => '\u{0301}',
        "grave" => '\u{0300}',
        "mathring" => '\u{030A}',
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn command_name(&mut self) -> String {
        // After the backslash: letters, or one symbol.
        let mut name = String::new();
        while let Some(c) = self.peek().filter(|c| c.is_ascii_alphabetic()) {
            name.push(c);
            self.at += 1;
        }
        if name.is_empty() {
            if let Some(c) = self.peek() {
                name.push(c);
                self.at += 1;
            }
        }
        name
    }

    /// Elements up to a closing brace, `\right`, `&`, `\\`, `\end` (left
    /// unread) or one of `stops`.
    fn expression(&mut self, stops: &[char]) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        while let Some(c) = self.peek() {
            if stops.contains(&c) || c == '}' || c == '&' {
                break;
            }
            if c == '\\' {
                let save = self.at;
                self.at += 1;
                let name = self.command_name();
                if matches!(name.as_str(), "right" | "end" | "\\" | "cr") {
                    self.at = save;
                    break;
                }
                if let Some(node) = self.command(&name, &mut out) {
                    out.push(node);
                }
                continue;
            }
            match c {
                '^' | '_' => {
                    let node = self.scripts(&mut out);
                    out.push(node);
                }
                '{' => {
                    let group = self.group();
                    out.push(Node::Group(group));
                }
                c if c.is_whitespace() => {
                    self.at += 1;
                }
                '\'' => {
                    self.at += 1;
                    push_text(&mut out, "′");
                }
                '-' => {
                    self.at += 1;
                    push_text(&mut out, "−");
                }
                _ => {
                    self.at += 1;
                    push_text(&mut out, &c.to_string());
                }
            }
        }
        out
    }

    /// `{…}`, or the next single token as an argument.
    fn group(&mut self) -> Vec<Node> {
        self.skip_spaces();
        match self.peek() {
            Some('{') => {
                self.at += 1;
                let content = self.expression(&[]);
                if self.peek() == Some('}') {
                    self.at += 1;
                }
                content
            }
            Some('\\') => {
                self.at += 1;
                let name = self.command_name();
                let mut scratch = Vec::new();
                self.command(&name, &mut scratch)
                    .map(|n| vec![n])
                    .unwrap_or(scratch)
            }
            Some(c) => {
                self.at += 1;
                vec![Node::Text(if c == '-' {
                    "−".into()
                } else {
                    c.to_string()
                })]
            }
            None => Vec::new(),
        }
    }

    /// `[…]` after `\sqrt`.
    fn optional(&mut self) -> Option<Vec<Node>> {
        self.skip_spaces();
        if self.peek() != Some('[') {
            return None;
        }
        self.at += 1;
        let content = self.expression(&[']']);
        if self.peek() == Some(']') {
            self.at += 1;
        }
        Some(content)
    }

    fn text_argument(&mut self) -> String {
        self.skip_spaces();
        if self.peek() != Some('{') {
            return self
                .peek()
                .map(|c| {
                    self.at += 1;
                    c.to_string()
                })
                .unwrap_or_default();
        }
        self.at += 1;
        let mut depth = 1;
        let mut text = String::new();
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            text.push(c);
        }
        text
    }

    fn scripts(&mut self, out: &mut Vec<Node>) -> Node {
        let base = match out.pop() {
            Some(Node::Text(text)) => {
                let mut chars: Vec<char> = text.chars().collect();
                let last = chars.pop().map(String::from).unwrap_or_default();
                let rest: String = chars.into_iter().collect();
                if !rest.is_empty() {
                    out.push(Node::Text(rest));
                }
                vec![Node::Text(last)]
            }
            Some(Node::Group(nodes)) => nodes,
            Some(other) => vec![other],
            None => Vec::new(),
        };
        let mut sub = None;
        let mut sup = None;
        while let Some(c) = self.peek().filter(|c| *c == '^' || *c == '_') {
            self.at += 1;
            let arg = self.group();
            if c == '_' && sub.is_none() {
                sub = Some(arg);
            } else if c == '^' && sup.is_none() {
                sup = Some(arg);
            }
            self.skip_spaces();
        }
        // Limits on an operator name or an n-ary operator stay with it.
        match base.as_slice() {
            [Node::Nary {
                chr, body, over, ..
            }] => Node::Nary {
                chr: *chr,
                sub,
                sup,
                body: body.clone(),
                over: *over,
            },
            [Node::Function { name, arg }] if arg.is_empty() => {
                let name = name.clone();
                if let (Some(lim), None) = (&sub, &sup) {
                    Node::Function {
                        name: vec![Node::Limit {
                            lower: true,
                            base: name,
                            lim: lim.clone(),
                        }],
                        arg: Vec::new(),
                    }
                } else {
                    Node::Function {
                        name: vec![Node::Script {
                            base: name,
                            sub,
                            sup,
                        }],
                        arg: Vec::new(),
                    }
                }
            }
            _ if base.is_empty() => Node::PreScript {
                sub: sub.unwrap_or_default(),
                sup: sup.unwrap_or_default(),
                base: {
                    self.skip_spaces();
                    self.group()
                },
            },
            _ => Node::Script { base, sub, sup },
        }
    }

    fn command(&mut self, name: &str, out: &mut Vec<Node>) -> Option<Node> {
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.group();
                let den = self.group();
                Some(Node::Fraction {
                    kind: FractionKind::Bar,
                    num,
                    den,
                })
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.group();
                let den = self.group();
                Some(Node::Delimiter {
                    open: "(".into(),
                    close: ")".into(),
                    separator: None,
                    items: vec![vec![Node::Fraction {
                        kind: FractionKind::NoBar,
                        num,
                        den,
                    }]],
                })
            }
            "sqrt" => {
                let degree = self.optional();
                let base = self.group();
                Some(Node::Radical { degree, base })
            }
            "left" => {
                self.skip_spaces();
                let open = self.delimiter_token();
                let mut items = vec![self.expression(&[])];
                // \middle separators.
                let mut separator = None;
                loop {
                    let save = self.at;
                    if self.peek() == Some('\\') {
                        self.at += 1;
                        let next = self.command_name();
                        if next == "middle" {
                            self.skip_spaces();
                            separator = Some(self.delimiter_token());
                            items.push(self.expression(&[]));
                            continue;
                        }
                        if next == "right" {
                            self.skip_spaces();
                            let close = self.delimiter_token();
                            return Some(Node::Delimiter {
                                open,
                                close,
                                separator,
                                items,
                            });
                        }
                    }
                    self.at = save;
                    break;
                }
                Some(Node::Delimiter {
                    open,
                    close: String::new(),
                    separator,
                    items,
                })
            }
            "begin" => {
                let env = self.text_argument();
                if env == "array" {
                    let _ = self.text_argument();
                }
                let rows = self.rows();
                // \end{…}
                if self.peek() == Some('\\') {
                    self.at += 1;
                    let _ = self.command_name();
                    let _ = self.text_argument();
                }
                let matrix = Node::Matrix(rows.clone());
                let (open, close) = match env.as_str() {
                    "pmatrix" => ("(", ")"),
                    "bmatrix" => ("[", "]"),
                    "Bmatrix" => ("{", "}"),
                    "vmatrix" => ("|", "|"),
                    "Vmatrix" => ("‖", "‖"),
                    "cases" => {
                        let rows = rows
                            .into_iter()
                            .map(|cells| join_cells(cells, true))
                            .collect();
                        return Some(Node::Delimiter {
                            open: "{".into(),
                            close: String::new(),
                            separator: None,
                            items: vec![vec![Node::EquationArray(rows)]],
                        });
                    }
                    "aligned" | "align" | "align*" | "eqnarray" | "split" | "gathered"
                    | "gather" | "gather*" => {
                        let rows = rows
                            .into_iter()
                            .map(|cells| join_cells(cells, true))
                            .collect();
                        return Some(Node::EquationArray(rows));
                    }
                    _ => return Some(matrix),
                };
                Some(Node::Delimiter {
                    open: open.into(),
                    close: close.into(),
                    separator: None,
                    items: vec![vec![matrix]],
                })
            }
            "overline" => Some(Node::Bar {
                top: true,
                base: self.group(),
            }),
            "underline" => Some(Node::Bar {
                top: false,
                base: self.group(),
            }),
            "overbrace" | "underbrace" => {
                let top = name == "overbrace";
                let base = self.group();
                let group = Node::GroupChar {
                    chr: if top { '⏞' } else { '⏟' },
                    top,
                    base,
                };
                self.skip_spaces();
                let script = if top { '^' } else { '_' };
                if self.peek() == Some(script) {
                    self.at += 1;
                    let lim = self.group();
                    return Some(Node::Limit {
                        lower: !top,
                        base: vec![group],
                        lim,
                    });
                }
                Some(group)
            }
            "underset" | "overset" => {
                let lim = self.group();
                let base = self.group();
                Some(Node::Limit {
                    lower: name == "underset",
                    base,
                    lim,
                })
            }
            "boxed" => Some(Node::BorderBox(self.group())),
            "phantom" => Some(Node::Phantom(self.group())),
            "text" | "textrm" | "mbox" | "textnormal" => Some(Node::Normal(self.text_argument())),
            "mathrm" | "operatorname" | "mathup" | "rm" => {
                let text = self.text_argument();
                if name == "operatorname" || is_function_name(&text) {
                    return Some(Node::Function {
                        name: vec![Node::Text(text)],
                        arg: Vec::new(),
                    });
                }
                Some(Node::Styled {
                    text,
                    sty: "p",
                    scr: None,
                })
            }
            "mathbf" | "mathit" | "mathbb" | "mathcal" | "mathscr" | "mathfrak" | "mathsf"
            | "mathtt" | "boldsymbol" | "bm" => {
                let text = self.text_argument();
                let (sty, scr) = match name {
                    "mathbf" => ("b", None),
                    "boldsymbol" | "bm" => ("bi", None),
                    "mathit" => ("i", None),
                    "mathbb" => ("p", Some("double-struck")),
                    "mathcal" | "mathscr" => ("p", Some("script")),
                    "mathfrak" => ("p", Some("fraktur")),
                    "mathsf" => ("p", Some("sans-serif")),
                    _ => ("p", Some("monospace")),
                };
                Some(Node::Styled { text, sty, scr })
            }
            "limits" | "nolimits" | "displaystyle" | "textstyle" | "scriptstyle" | "mathop" => {
                if let Some(Node::Nary { over, .. }) = out.last_mut() {
                    if name == "limits" {
                        *over = Some(true);
                    } else if name == "nolimits" {
                        *over = Some(false);
                    }
                }
                None
            }
            name if accent_for(name).is_some() => Some(Node::Accent {
                chr: accent_for(name)?,
                base: self.group(),
            }),
            name if is_function_name(name) => Some(Node::Function {
                name: vec![Node::Text(name.to_string())],
                arg: Vec::new(),
            }),
            name => {
                let value = symbol(name)?;
                let mut chars = value.chars();
                let first = chars.next();
                if let (Some(ch), None) = (first, chars.next()) {
                    if is_nary(ch) {
                        return Some(Node::Nary {
                            chr: ch,
                            sub: None,
                            sup: None,
                            body: Vec::new(),
                            over: None,
                        });
                    }
                }
                push_text(out, value);
                None
            }
        }
    }

    fn delimiter_token(&mut self) -> String {
        match self.peek() {
            Some('.') => {
                self.at += 1;
                String::new()
            }
            Some('\\') => {
                self.at += 1;
                let name = self.command_name();
                symbol(&name).unwrap_or("").to_string()
            }
            Some(c) => {
                self.at += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    /// Rows of an environment: cells split at `&`, rows at `\\`, up to
    /// `\end`.
    fn rows(&mut self) -> Vec<Vec<Vec<Node>>> {
        let mut rows = vec![vec![]];
        loop {
            let cell = self.expression(&[]);
            rows.last_mut().unwrap().push(cell);
            match self.peek() {
                Some('&') => self.at += 1,
                Some('\\') => {
                    let save = self.at;
                    self.at += 1;
                    let name = self.command_name();
                    if name == "\\" || name == "cr" {
                        rows.push(Vec::new());
                        continue;
                    }
                    self.at = save;
                    break;
                }
                Some('}') => {
                    self.at += 1;
                }
                _ => break,
            }
        }
        if rows.last().is_some_and(|r| r.iter().all(Vec::is_empty)) && rows.len() > 1 {
            rows.pop();
        }
        rows
    }
}

/// An aligned row's cells joined back with the `&` marks Word aligns on.
fn join_cells(cells: Vec<Vec<Node>>, marks: bool) -> Vec<Node> {
    let mut row = Vec::new();
    for (i, cell) in cells.into_iter().enumerate() {
        if i > 0 && marks {
            row.push(Node::Text("&".into()));
        }
        row.extend(cell);
    }
    row
}

fn push_text(out: &mut Vec<Node>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Node::Text(last)) = out.last_mut() {
        last.push_str(text);
        return;
    }
    out.push(Node::Text(text.to_string()));
}

/// Folds a function name and the element after it into the function,
/// and an n-ary operator and what follows into its operand.
fn finish(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let mut iter = nodes.into_iter().peekable();
    while let Some(node) = iter.next() {
        match node {
            Node::Function { name, arg } if arg.is_empty() => {
                let arg = match iter.next() {
                    Some(Node::Text(text)) => {
                        let mut chars = text.chars();
                        let first: String = chars.next().map(String::from).unwrap_or_default();
                        let rest: String = chars.collect();
                        if !rest.is_empty() {
                            out.push(Node::Function {
                                name,
                                arg: vec![Node::Text(first)],
                            });
                            out.push(Node::Text(rest));
                            continue;
                        }
                        vec![Node::Text(first)]
                    }
                    Some(other) => vec![other],
                    None => Vec::new(),
                };
                out.push(Node::Function {
                    name,
                    arg: finish(arg),
                });
            }
            Node::Nary {
                chr,
                sub,
                sup,
                body,
                over,
            } if body.is_empty() => {
                let rest: Vec<Node> = iter.by_ref().collect();
                out.push(Node::Nary {
                    chr,
                    sub,
                    sup,
                    body: finish(rest),
                    over,
                });
            }
            other => out.push(recurse(other)),
        }
    }
    out
}

fn recurse(node: Node) -> Node {
    match node {
        Node::Group(nodes) => Node::Group(finish(nodes)),
        Node::Fraction { kind, num, den } => Node::Fraction {
            kind,
            num: finish(num),
            den: finish(den),
        },
        Node::Script { base, sub, sup } => Node::Script {
            base: finish(base),
            sub: sub.map(finish),
            sup: sup.map(finish),
        },
        Node::Radical { degree, base } => Node::Radical {
            degree: degree.map(finish),
            base: finish(base),
        },
        Node::Delimiter {
            open,
            close,
            separator,
            items,
        } => Node::Delimiter {
            open,
            close,
            separator,
            items: items.into_iter().map(finish).collect(),
        },
        Node::Matrix(rows) => Node::Matrix(
            rows.into_iter()
                .map(|row| row.into_iter().map(finish).collect())
                .collect(),
        ),
        Node::EquationArray(rows) => Node::EquationArray(rows.into_iter().map(finish).collect()),
        other => other,
    }
}

/// Reads LaTeX into structure, folding functions and n-ary operands.
pub fn parse_finished(text: &str) -> Vec<Node> {
    finish(parse(text))
}

// ---- Writing OMML as LaTeX ------------------------------------------------

/// Writes math elements as LaTeX.
pub fn write(elements: &[MathElement]) -> String {
    let mut out = String::new();
    for e in elements.iter().filter(|e| is_content(e)) {
        write_element(e, &mut out);
    }
    out.trim().to_string()
}

fn content(e: Option<&MathElement>) -> String {
    let mut out = String::new();
    if let Some(e) = e {
        for c in e.children.iter().filter(|c| is_content(c)) {
            write_element(c, &mut out);
        }
    }
    out.trim().to_string()
}

fn braced(e: Option<&MathElement>) -> String {
    format!("{{{}}}", content(e))
}

/// The LaTeX for a character Word stores in OMML.
fn latex_char(c: char, out: &mut String) {
    let name = match c {
        '−' => {
            out.push('-');
            return;
        }
        '{' | '}' | '%' | '$' | '#' | '_' => {
            out.push('\\');
            out.push(c);
            return;
        }
        '≤' => "le",
        '≥' => "ge",
        '≠' => "neq",
        '→' => "rightarrow",
        '∞' => "infty",
        '⋅' => "cdot",
        '⋯' => "cdots",
        '…' => "ldots",
        '′' => {
            out.push('\'');
            return;
        }
        _ => {
            if let Some((name, _)) = super::MATH_AUTOCORRECT.iter().find(|(name, value)| {
                value.chars().count() == 1
                    && value.starts_with(c)
                    && !c.is_ascii()
                    && name.chars().all(|ch| ch.is_ascii_alphabetic())
            }) {
                name
            } else {
                out.push(c);
                return;
            }
        }
    };
    out.push('\\');
    out.push_str(name);
    out.push(' ');
}

fn write_element(e: &MathElement, out: &mut String) {
    use MathElementKind as K;
    match e.kind {
        K::Run => {
            let (sty, scr, nor) = run_style(e);
            let text = text_of(e);
            if nor {
                out.push_str(&format!("\\text{{{text}}}"));
                return;
            }
            let wrapper = match (sty.as_deref(), scr.as_deref()) {
                (_, Some("double-struck")) => Some("mathbb"),
                (_, Some("script")) => Some("mathcal"),
                (_, Some("fraktur")) => Some("mathfrak"),
                (_, Some("sans-serif")) => Some("mathsf"),
                (_, Some("monospace")) => Some("mathtt"),
                (Some("b"), _) => Some("mathbf"),
                (Some("bi"), _) => Some("boldsymbol"),
                (Some("p"), _) if !is_function_name(&text) => Some("mathrm"),
                _ => None,
            };
            if let Some(wrapper) = wrapper {
                out.push_str(&format!("\\{wrapper}{{{text}}}"));
            } else if is_function_name(&text) {
                out.push('\\');
                out.push_str(&text);
                out.push(' ');
            } else {
                for c in text.chars() {
                    latex_char(c, out);
                }
            }
        }
        K::Text => out.push_str(e.text.as_deref().unwrap_or_default()),
        K::Fraction => match property(e, "fPr", "type") {
            Some("lin") | Some("skw") => {
                out.push_str(&format!(
                    "{}/{}",
                    braced(child(e, K::Numerator)),
                    braced(child(e, K::Denominator))
                ));
            }
            Some("noBar") => out.push_str(&format!(
                "\\genfrac{{}}{{}}{{0pt}}{{}}{}{}",
                braced(child(e, K::Numerator)),
                braced(child(e, K::Denominator))
            )),
            _ => out.push_str(&format!(
                "\\frac{}{}",
                braced(child(e, K::Numerator)),
                braced(child(e, K::Denominator))
            )),
        },
        K::Superscript | K::Subscript | K::SubSup => {
            out.push_str(&base(child(e, K::Base)));
            if let Some(sub) = child(e, K::SubArgument) {
                out.push('_');
                out.push_str(&braced(Some(sub)));
            }
            if let Some(sup) = child(e, K::SuperArgument) {
                out.push('^');
                out.push_str(&braced(Some(sup)));
            }
        }
        K::PreSubSuper => {
            out.push_str(&format!(
                "{{}}_{}^{}{}",
                braced(child(e, K::SubArgument)),
                braced(child(e, K::SuperArgument)),
                base(child(e, K::Base))
            ));
        }
        K::Radical => {
            let degree = child(e, K::Degree)
                .filter(|_| !flag(e, "radPr", "degHide"))
                .map(|d| content(Some(d)))
                .filter(|d| !d.is_empty());
            match degree {
                Some(degree) => {
                    out.push_str(&format!("\\sqrt[{degree}]{}", braced(child(e, K::Base))))
                }
                None => out.push_str(&format!("\\sqrt{}", braced(child(e, K::Base)))),
            }
        }
        K::Nary => {
            let chr = property(e, "naryPr", "chr").unwrap_or("∫");
            let name = super::MATH_AUTOCORRECT
                .iter()
                .find(|(_, value)| *value == chr)
                .map(|(name, _)| *name)
                .unwrap_or("int");
            out.push('\\');
            out.push_str(name);
            match property(e, "naryPr", "limLoc") {
                Some("undOvr") if super::is_integral(chr.chars().next().unwrap_or('∫')) => {
                    out.push_str("\\limits")
                }
                Some("subSup") if !super::is_integral(chr.chars().next().unwrap_or('∫')) => {
                    out.push_str("\\nolimits")
                }
                _ => {}
            }
            if !flag(e, "naryPr", "subHide") {
                if let Some(sub) = child(e, K::SubArgument).filter(|s| !s.children.is_empty()) {
                    out.push('_');
                    out.push_str(&braced(Some(sub)));
                }
            }
            if !flag(e, "naryPr", "supHide") {
                if let Some(sup) = child(e, K::SuperArgument).filter(|s| !s.children.is_empty()) {
                    out.push('^');
                    out.push_str(&braced(Some(sup)));
                }
            }
            out.push(' ');
            out.push_str(&content(child(e, K::Base)));
        }
        K::Delimiter => {
            let open = property(e, "dPr", "begChr").unwrap_or("(");
            let close = property(e, "dPr", "endChr").unwrap_or(")");
            let separator = property(e, "dPr", "sepChr").unwrap_or("|");
            let token = |c: &str| -> String {
                match c {
                    "" => ".".into(),
                    "{" => "\\{".into(),
                    "}" => "\\}".into(),
                    "⟨" => "\\langle ".into(),
                    "⟩" => "\\rangle ".into(),
                    "‖" => "\\|".into(),
                    other => other.into(),
                }
            };
            // A matrix in delimiters is one of the matrix environments.
            let items: Vec<&MathElement> = children(e, K::Base).collect();
            if let [item] = items.as_slice() {
                let mut inner = item.children.iter().filter(|c| is_content(c));
                if let (Some(only), None) = (inner.next(), inner.next()) {
                    let env = match (open, close) {
                        ("(", ")") => Some("pmatrix"),
                        ("[", "]") => Some("bmatrix"),
                        ("{", "}") => Some("Bmatrix"),
                        ("|", "|") => Some("vmatrix"),
                        ("‖", "‖") => Some("Vmatrix"),
                        _ => None,
                    };
                    if only.kind == K::Matrix {
                        if let Some(env) = env {
                            out.push_str(&environment(env, only));
                            return;
                        }
                    }
                    if only.kind == K::EquationArray && open == "{" && close.is_empty() {
                        out.push_str(&environment("cases", only));
                        return;
                    }
                }
            }
            out.push_str(&format!("\\left{}", token(open)));
            let parts: Vec<String> = items.iter().map(|i| content(Some(i))).collect();
            out.push_str(&parts.join(&format!("\\middle{}", token(separator))));
            out.push_str(&format!("\\right{}", token(close)));
        }
        K::Function => {
            let name = child(e, K::FunctionName);
            out.push_str(&content(name));
            out.push(' ');
            out.push_str(&content(child(e, K::Base)));
        }
        K::Matrix => out.push_str(&environment("matrix", e)),
        K::EquationArray => out.push_str(&environment("aligned", e)),
        K::Accent => {
            let chr = property(e, "accPr", "chr").unwrap_or("\u{0302}");
            let name = match chr.chars().next().unwrap_or('\u{0302}') {
                '\u{0303}' => "tilde",
                '\u{0305}' | '\u{0304}' => "bar",
                '\u{20D7}' => "vec",
                '\u{20D6}' => "overleftarrow",
                '\u{20E1}' => "overleftrightarrow",
                '\u{0307}' => "dot",
                '\u{0308}' => "ddot",
                '\u{20DB}' => "dddot",
                '\u{030C}' => "check",
                '\u{0306}' => "breve",
                '\u{0301}' => "acute",
                '\u{0300}' => "grave",
                '\u{030A}' => "mathring",
                _ => "hat",
            };
            out.push_str(&format!("\\{name}{}", braced(child(e, K::Base))));
        }
        K::Bar => {
            let top = property(e, "barPr", "pos") == Some("top");
            let name = if top { "overline" } else { "underline" };
            out.push_str(&format!("\\{name}{}", braced(child(e, K::Base))));
        }
        K::GroupChar => {
            let top = property(e, "groupChrPr", "pos") == Some("top");
            let name = if top { "overbrace" } else { "underbrace" };
            out.push_str(&format!("\\{name}{}", braced(child(e, K::Base))));
        }
        K::LimitLower | K::LimitUpper => {
            let lower = e.kind == K::LimitLower;
            let b = child(e, K::Base);
            let group = b.and_then(|b| {
                let mut c = b.children.iter().filter(|c| is_content(c));
                let first = c.next()?;
                (c.next().is_none() && first.kind == K::GroupChar).then_some(first)
            });
            let base_text = content(b);
            if group.is_some() || is_function_name(base_text.trim_start_matches('\\').trim()) {
                out.push_str(&base_text);
                out.push(if lower { '_' } else { '^' });
                out.push_str(&braced(child(e, K::Limit)));
            } else {
                let name = if lower { "underset" } else { "overset" };
                out.push_str(&format!(
                    "\\{name}{}{{{base_text}}}",
                    braced(child(e, K::Limit))
                ));
            }
        }
        K::BorderBox => out.push_str(&format!("\\boxed{}", braced(child(e, K::Base)))),
        K::Phantom => out.push_str(&format!("\\phantom{}", braced(child(e, K::Base)))),
        K::Box => out.push_str(&content(child(e, K::Base))),
        _ => {
            for c in e.children.iter().filter(|c| is_content(c)) {
                write_element(c, out);
            }
        }
    }
}

fn base(e: Option<&MathElement>) -> String {
    let text = content(e);
    if text.chars().count() <= 1 || text.starts_with('\\') && !text.contains(' ') {
        text
    } else {
        format!("{{{text}}}")
    }
}

fn environment(name: &str, e: &MathElement) -> String {
    let rows: Vec<String> = if e.kind == MathElementKind::Matrix {
        children(e, MathElementKind::MatrixRow)
            .map(|row| {
                children(row, MathElementKind::Base)
                    .map(|c| content(Some(c)))
                    .collect::<Vec<_>>()
                    .join(" & ")
            })
            .collect()
    } else {
        children(e, MathElementKind::Base)
            .map(|c| content(Some(c)))
            .collect()
    };
    format!("\\begin{{{name}}}{}\\end{{{name}}}", rows.join("\\\\"))
}
