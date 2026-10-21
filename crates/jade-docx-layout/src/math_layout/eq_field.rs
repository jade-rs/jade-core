//! The EQ field (§17.16.5.20): Word's equation field from before Office Math,
//! still written by Japanese templates for fractions, radicals, brackets and
//! boxed characters.
//!
//! [`eq_field_elements`] reads an EQ instruction into the same element tree
//! OMML parses to, so the math layout lays it out in two dimensions: `\f` is a
//! fraction, `\r` a radical, `\b` stretchy brackets, `\i` an integral, `\x` a
//! border box. The switches OMML has no structure for — `\s` raised and
//! lowered text, `\a` arrays, `\o` overstrikes, `\d` displacement, `\l` lists
//! — become elements the layout knows by name (`eqShift`, `eqArray`,
//! `eqOverstrike`, `eqDisplace`, `eqList`).
//!
//! An EQ field is set in its run's own face and size, upright, at one size
//! throughout ([`super::styled::MathLayoutOptions::eq_field`]).
use jade_docx_reader::document::{MathElement, MathElementKind};

/// The elements an EQ instruction (`EQ \f(1,2)`) lays out as, or `None` when
/// the instruction is not an EQ field or uses none of its structures.
pub fn eq_field_elements(instruction: &str) -> Option<Vec<MathElement>> {
    let trimmed = instruction.trim();
    let (name, rest) = split_name(trimmed)?;
    if !name.eq_ignore_ascii_case("EQ") {
        return None;
    }
    let mut parser = Parser {
        chars: rest.trim().chars().collect(),
        at: 0,
    };
    // Spaces between top-level switches separate them; they are not text.
    let nodes: Vec<Node> = parser
        .sequence(false)
        .into_iter()
        .filter(|node| !matches!(node, Node::Text(text) if text.trim().is_empty()))
        .collect();
    if !nodes.iter().any(|n| matches!(n, Node::Switch { .. })) {
        return None;
    }
    Some(nodes.into_iter().map(Node::into_element).collect())
}

fn split_name(text: &str) -> Option<(&str, &str)> {
    let end = text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map_or(text.len(), |(i, _)| i);
    (end > 0).then(|| (&text[..end], &text[end..]))
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    Switch {
        name: char,
        options: Vec<(String, Option<String>)>,
        args: Vec<Vec<Node>>,
    },
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

const STRUCTURES: &str = "frsbaloxid";

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    /// Text and switches up to the end, or (in an argument) up to the comma
    /// or closing parenthesis that ends it.
    fn sequence(&mut self, in_argument: bool) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            if in_argument && (ch == ',' || ch == ';' || ch == ')') {
                break;
            }
            if ch == '\\' {
                let next = self.peek_at(1);
                match next {
                    Some(c) if c.is_ascii_alphabetic() => {
                        let name = c.to_ascii_lowercase();
                        // A structure is one letter followed by options or
                        // its argument list.
                        let after = self.peek_at(2);
                        let structure = STRUCTURES.contains(name)
                            && after.is_none_or(|a| !a.is_ascii_alphabetic());
                        if structure {
                            if !text.is_empty() {
                                nodes.push(Node::Text(std::mem::take(&mut text)));
                            }
                            self.at += 2;
                            if let Some(node) = self.switch(name) {
                                nodes.push(node);
                            }
                            continue;
                        }
                        // A field switch Word reads elsewhere (`\* jc`),
                        // or text: keep the letters.
                        self.at += 1;
                        continue;
                    }
                    Some(c) => {
                        // `\(`, `\,`, `\\`: the character itself.
                        text.push(c);
                        self.at += 2;
                        continue;
                    }
                    None => {
                        self.at += 1;
                        continue;
                    }
                }
            }
            text.push(ch);
            self.at += 1;
        }
        if !text.is_empty() {
            nodes.push(Node::Text(text));
        }
        nodes
    }

    fn switch(&mut self, name: char) -> Option<Node> {
        let mut options = Vec::new();
        loop {
            while self.peek() == Some(' ') {
                self.at += 1;
            }
            if self.peek() == Some('\\')
                && self.peek_at(1).is_some_and(|c| c.is_ascii_alphabetic())
                && self.peek_at(2).is_some_and(|c| c.is_ascii_alphabetic())
            {
                self.at += 1;
                let mut option = String::new();
                while let Some(c) = self.peek().filter(|c| c.is_ascii_alphabetic()) {
                    option.push(c.to_ascii_lowercase());
                    self.at += 1;
                    // Options are two letters: `\lc\(`, `\up8`, `\co2`.
                    if option.len() == 2 {
                        break;
                    }
                }
                let value = match option.as_str() {
                    "lc" | "rc" | "bc" | "fc" | "vc" => {
                        if self.peek() == Some('\\') {
                            self.at += 1;
                        }
                        let c = self.peek();
                        if c.is_some() {
                            self.at += 1;
                        }
                        c.map(String::from)
                    }
                    "up" | "do" | "ai" | "di" | "co" | "vs" | "hs" | "fo" | "ba" => {
                        let mut number = String::new();
                        while let Some(c) = self
                            .peek()
                            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                        {
                            number.push(c);
                            self.at += 1;
                        }
                        Some(number)
                    }
                    _ => None,
                };
                options.push((option, value));
                continue;
            }
            break;
        }
        while self.peek() == Some(' ') {
            self.at += 1;
        }
        let mut args = Vec::new();
        if self.peek() == Some('(') {
            self.at += 1;
            loop {
                args.push(self.sequence(true));
                match self.peek() {
                    Some(',') | Some(';') => {
                        self.at += 1;
                    }
                    Some(')') => {
                        self.at += 1;
                        break;
                    }
                    _ => break,
                }
            }
        }
        Some(Node::Switch {
            name,
            options,
            args,
        })
    }
}

fn run(text: &str) -> MathElement {
    MathElement::new(MathElementKind::Run, vec![MathElement::text_node(text)])
}

fn argument(kind: MathElementKind, nodes: Vec<Node>) -> MathElement {
    MathElement::new(kind, nodes.into_iter().map(Node::into_element).collect())
}

fn properties(tag: &str, values: &[(&str, &str)]) -> MathElement {
    MathElement::tagged(
        tag,
        values
            .iter()
            .map(|(name, value)| MathElement::property(name, value))
            .collect(),
    )
}

impl Node {
    fn into_element(self) -> MathElement {
        use MathElementKind::*;
        match self {
            Node::Text(text) => run(&text),
            Node::Switch {
                name,
                options,
                args,
            } => {
                let has = |n: &str| options.iter().any(|(o, _)| o == n);
                let value = |n: &str| {
                    options
                        .iter()
                        .find(|(o, _)| o == n)
                        .and_then(|(_, v)| v.clone())
                };
                let mut args = args.into_iter();
                let mut next = || args.next().unwrap_or_default();
                match name {
                    'f' => MathElement::new(
                        Fraction,
                        vec![argument(Numerator, next()), argument(Denominator, next())],
                    ),
                    'r' => {
                        let first = next();
                        let second = next();
                        if second.is_empty() {
                            MathElement::new(
                                Radical,
                                vec![
                                    properties("radPr", &[("degHide", "1")]),
                                    MathElement::new(Degree, Vec::new()),
                                    argument(Base, first),
                                ],
                            )
                        } else {
                            MathElement::new(
                                Radical,
                                vec![argument(Degree, first), argument(Base, second)],
                            )
                        }
                    }
                    'b' => {
                        let both = value("bc");
                        let (open, close) = match both.as_deref() {
                            Some(c) => (c.to_string(), matching(c).to_string()),
                            None => (
                                value("lc").unwrap_or_else(|| "(".into()),
                                value("rc").unwrap_or_else(|| ")".into()),
                            ),
                        };
                        MathElement::new(
                            Delimiter,
                            vec![
                                properties("dPr", &[("begChr", &open), ("endChr", &close)]),
                                argument(Base, next()),
                            ],
                        )
                    }
                    'x' => {
                        let sides = ["to", "bo", "le", "ri"];
                        let named = sides.iter().any(|s| has(s));
                        let mut hide = Vec::new();
                        if named {
                            for (side, tag) in
                                sides
                                    .iter()
                                    .zip(["hideTop", "hideBot", "hideLeft", "hideRight"])
                            {
                                if !has(side) {
                                    hide.push((tag, "1"));
                                }
                            }
                        }
                        MathElement::new(
                            BorderBox,
                            vec![properties("borderBoxPr", &hide), argument(Base, next())],
                        )
                    }
                    'i' => {
                        let lower = next();
                        let upper = next();
                        let body = next();
                        let chr = if let Some(c) = value("fc").or_else(|| value("vc")) {
                            c
                        } else if has("su") {
                            "∑".into()
                        } else if has("pr") {
                            "∏".into()
                        } else {
                            "∫".into()
                        };
                        let location = if has("in") || chr == "∫" {
                            "subSup"
                        } else {
                            "undOvr"
                        };
                        let mut pr = vec![("chr", chr.as_str()), ("limLoc", location)];
                        if has("vc") {
                            pr.push(("grow", "1"));
                        }
                        MathElement::new(
                            Nary,
                            vec![
                                properties("naryPr", &pr),
                                argument(SubArgument, lower),
                                argument(SuperArgument, upper),
                                argument(Base, body),
                            ],
                        )
                    }
                    's' | 'a' | 'o' | 'd' | 'l' => {
                        let tag = match name {
                            's' => "eqShift",
                            'a' => "eqArray",
                            'o' => "eqOverstrike",
                            'd' => "eqDisplace",
                            _ => "eqList",
                        };
                        let mut element = MathElement::tagged(tag, Vec::new());
                        element.kind = Unknown;
                        for (option, value) in &options {
                            let key = match (name, option.as_str()) {
                                (_, "al") => {
                                    element.attributes.insert("al".into(), "l".into());
                                    continue;
                                }
                                (_, "ar") => {
                                    element.attributes.insert("al".into(), "r".into());
                                    continue;
                                }
                                (_, "ac") => {
                                    element.attributes.insert("al".into(), "c".into());
                                    continue;
                                }
                                (_, other) => other.to_string(),
                            };
                            element
                                .attributes
                                .insert(key, value.clone().unwrap_or_default());
                        }
                        for arg in args.by_ref() {
                            element.children.push(argument(Base, arg));
                        }
                        element
                    }
                    _ => MathElement::new(Box, vec![argument(Base, next())]),
                }
            }
        }
    }
}

/// The closing bracket for an opening one (`\bc\{` draws `{` and `}`).
fn matching(open: &str) -> &str {
    match open {
        "(" => ")",
        "[" => "]",
        "{" => "}",
        "<" => ">",
        "〈" => "〉",
        "⟨" => "⟩",
        "「" => "」",
        "『" => "』",
        "【" => "】",
        "〔" => "〕",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(elements: &[MathElement]) -> Vec<String> {
        elements
            .iter()
            .map(|e| e.local_name().to_string())
            .collect()
    }

    #[test]
    fn every_eq_switch_reads_as_a_structure() {
        let f = eq_field_elements(r" EQ \f(1,2) ").unwrap();
        assert_eq!(f[0].kind, MathElementKind::Fraction);
        assert_eq!(f[0].children.len(), 2);
        let r = eq_field_elements(r"EQ \r(3,x)").unwrap();
        assert_eq!(r[0].kind, MathElementKind::Radical);
        assert_eq!(r[0].children[0].kind, MathElementKind::Degree);
        let r = eq_field_elements(r"EQ \r(x)").unwrap();
        assert_eq!(r[0].children[0].local_name(), "radPr");
        let b = eq_field_elements(r"EQ \b\lc\[\rc\](a)").unwrap();
        let pr = &b[0].children[0];
        assert_eq!(pr.children[0].attributes["val"], "[");
        assert_eq!(pr.children[1].attributes["val"], "]");
        let b = eq_field_elements(r"EQ \b\bc\{(a)").unwrap();
        assert_eq!(b[0].children[0].children[1].attributes["val"], "}");
        let s = eq_field_elements(r"EQ x\s\up8(2)").unwrap();
        assert_eq!(tags(&s), ["r", "eqShift"]);
        assert_eq!(s[1].attributes["up"], "8");
        let a = eq_field_elements(r"EQ \a\al\co2\vs3\hs3(a,b,c,d)").unwrap();
        assert_eq!(a[0].attributes["co"], "2");
        assert_eq!(a[0].attributes["al"], "l");
        assert_eq!(a[0].children.len(), 4);
        let o = eq_field_elements(r"EQ \o\ac(○,1)").unwrap();
        assert_eq!(o[0].local_name(), "eqOverstrike");
        let x = eq_field_elements(r"EQ \x\to\bo(z)").unwrap();
        let hidden: Vec<_> = x[0].children[0]
            .children
            .iter()
            .map(|c| c.local_name())
            .collect();
        assert_eq!(hidden, ["hideLeft", "hideRight"]);
        let i = eq_field_elements(r"EQ \i\su(i=1,n,i)").unwrap();
        assert_eq!(i[0].kind, MathElementKind::Nary);
        assert_eq!(i[0].children[0].children[0].attributes["val"], "∑");
        let d = eq_field_elements(r"EQ \d\fo10\li()").unwrap();
        assert_eq!(d[0].attributes["fo"], "10");
        assert!(d[0].attributes.contains_key("li"));
        let l = eq_field_elements(r"EQ \l(a,b,c)").unwrap();
        assert_eq!(l[0].children.len(), 3);
    }

    #[test]
    fn nested_switches_escapes_and_non_eq_fields() {
        let n = eq_field_elements(r"EQ \f(\r(x),2)").unwrap();
        let numerator = &n[0].children[0];
        assert_eq!(numerator.children[0].kind, MathElementKind::Radical);
        let e = eq_field_elements(r"EQ \b(a\,b)").unwrap();
        let base = &e[0].children[1];
        assert_eq!(base.children[0].children[0].text.as_deref(), Some("a,b"));
        assert!(eq_field_elements(r"PAGE \* MERGEFORMAT").is_none());
        assert!(eq_field_elements("EQ plain").is_none());
    }
}
