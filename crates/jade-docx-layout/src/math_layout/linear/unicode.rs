//! UnicodeMath, Word's default linear format (Unicode Technical Note #28):
//! `x=(−b±√(b^2−4ac))/2a` builds up to the quadratic formula.
//!
//! Operators build up around operands: `/` (`∕` linear, `⁄` skewed, `¦`
//! stacked) makes a fraction of the operands on either side, `^` and `_`
//! scripts, `√` `∛` `∜` radicals (`√(n&x)` with a degree), n-ary operators
//! take `_` `^` limits and an operand after `▒`, `■(a&b@c&d)` is a matrix,
//! `█(x&=1@y&=2)` an equation array, `□` `▭` `¯` `▁` `⟡` box, border box,
//! overbar, underbar and phantom, `⏞` `⏟` group characters, `┬` `┴`
//! limits below and above, a combining mark an accent, `"…"` normal text,
//! and `sin`, `lim` and the other function names apply to what follows.
//! Parentheses around a fraction's or a script's operand group it and are
//! dropped; elsewhere they are delimiters, which grow.
use super::read::{child, children, flag, is_content, property, run_style, text_of};
use super::{apply_math_autocorrect, is_function_name, is_nary, FractionKind, Node};
use jade_docx_reader::document::{MathElement, MathElementKind};

/// Reads UnicodeMath (with Math AutoCorrect control words) into structure.
pub fn parse(text: &str) -> Vec<Node> {
    let text = apply_math_autocorrect(text);
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    let nodes = parser.expression(&[]);
    apply_functions(nodes)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

fn is_combining_accent(ch: char) -> bool {
    matches!(ch, '\u{0300}'..='\u{036F}' | '\u{20D0}'..='\u{20FF}')
}

/// Characters that are part of an operand run (`2x`, `ab`, `3.14`).
fn is_operand_char(ch: char) -> bool {
    // ⬚ is an empty argument in the Equation tab's templates.
    ch.is_alphanumeric()
        || matches!(
            ch,
            '.' | '′' | '″' | '‴' | '∞' | '∂' | '∅' | 'ℏ' | 'ℓ' | '⬚'
        )
}

fn closing_for(open: char) -> Option<char> {
    Some(match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '⟨' => '⟩',
        '〈' => '〉',
        '⌈' => '⌉',
        '⌊' => '⌋',
        '⟦' => '⟧',
        '|' => '|',
        '‖' => '‖',
        _ => return None,
    })
}

fn is_closing(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '⟩' | '〉' | '⌉' | '⌋' | '⟧' | '┤')
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek() == Some(' ') {
            self.at += 1;
        }
    }

    /// Elements up to one of `stops` (left unread) or the end.
    fn expression(&mut self, stops: &[char]) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        while let Some(c) = self.peek() {
            if stops.contains(&c) {
                break;
            }
            match c {
                ' ' => {
                    self.at += 1;
                    // A space ends an operand.
                    if !matches!(out.last(), Some(Node::Text(t)) if t.is_empty()) {
                        out.push(Node::Text(String::new()));
                    }
                }
                '/' | '∕' | '⁄' | '¦' | '⊘' => {
                    self.at += 1;
                    let kind = match c {
                        '/' => FractionKind::Bar,
                        '∕' => FractionKind::Linear,
                        '¦' => FractionKind::NoBar,
                        _ => FractionKind::Skewed,
                    };
                    let num = take_operand(&mut out);
                    self.skip_spaces();
                    let den = self.operand_with_scripts();
                    out.push(Node::Fraction { kind, num, den });
                }
                '^' | '_' => {
                    let node = self.scripts(&mut out);
                    out.push(node);
                }
                '┬' | '┴' => {
                    self.at += 1;
                    let base = strip_parens(take_base(&mut out));
                    let lim = self.argument();
                    out.push(Node::Limit {
                        lower: c == '┬',
                        base,
                        lim,
                    });
                }
                '⁡' => {
                    // Function application: what precedes is the name.
                    self.at += 1;
                    let name = take_operand(&mut out);
                    self.skip_spaces();
                    let arg = self.operand_with_scripts_keep();
                    out.push(Node::Function { name, arg });
                }
                '"' => {
                    self.at += 1;
                    let mut text = String::new();
                    while let Some(ch) = self.peek() {
                        self.at += 1;
                        if ch == '"' {
                            break;
                        }
                        text.push(ch);
                    }
                    out.push(Node::Normal(text));
                }
                '\\' => {
                    // An escaped character is literal.
                    self.at += 1;
                    if let Some(ch) = self.peek() {
                        self.at += 1;
                        push_char(&mut out, ch);
                    }
                }
                ch if is_combining_accent(ch) => {
                    self.at += 1;
                    let base = strip_parens(take_base(&mut out));
                    out.push(Node::Accent { chr: ch, base });
                }
                _ => {
                    if let Some(node) = self.structure() {
                        out.push(node);
                    } else {
                        self.at += 1;
                        push_char(&mut out, c);
                    }
                }
            }
        }
        out
    }

    /// A structure starting at the current character, if it starts one.
    fn structure(&mut self) -> Option<Node> {
        let c = self.peek()?;
        match c {
            '√' | '∛' | '∜' => {
                self.at += 1;
                let mut degree = match c {
                    '∛' => Some(vec![Node::Text("3".into())]),
                    '∜' => Some(vec![Node::Text("4".into())]),
                    _ => None,
                };
                self.skip_spaces();
                let base = if self.peek() == Some('(') {
                    let start = self.at;
                    self.at += 1;
                    let first = self.expression(&['&', ')']);
                    if self.peek() == Some('&') {
                        self.at += 1;
                        let base = self.expression(&[')']);
                        if self.peek() == Some(')') {
                            self.at += 1;
                        }
                        degree = Some(first);
                        base
                    } else if self.peek() == Some(')') {
                        self.at += 1;
                        let base = first;
                        self.attach_scripts(base)
                    } else {
                        self.at = start;
                        self.operand_with_scripts()
                    }
                } else {
                    self.operand_with_scripts()
                };
                Some(Node::Radical { degree, base })
            }
            ch if is_nary(ch) => {
                self.at += 1;
                let mut sub = None;
                let mut sup = None;
                for _ in 0..2 {
                    match self.peek() {
                        Some('_') if sub.is_none() => {
                            self.at += 1;
                            sub = Some(self.argument());
                        }
                        Some('^') if sup.is_none() => {
                            self.at += 1;
                            sup = Some(self.argument());
                        }
                        _ => {}
                    }
                }
                self.skip_spaces();
                let body = if self.peek() == Some('▒') {
                    self.at += 1;
                    self.skip_spaces();
                    self.operand_with_scripts_keep()
                } else if self
                    .peek()
                    .is_some_and(|c| !is_closing(c) && c != '@' && c != '&')
                {
                    self.operand_with_scripts_keep()
                } else {
                    Vec::new()
                };
                Some(Node::Nary {
                    chr: ch,
                    sub,
                    sup,
                    body,
                    over: None,
                })
            }
            '〖' => {
                self.at += 1;
                let content = self.expression(&['〗']);
                if self.peek() == Some('〗') {
                    self.at += 1;
                }
                Some(Node::Group(content))
            }
            '├' => {
                // An opening with no delimiter (or the next one): `├x┤`.
                self.at += 1;
                let open = match self.peek() {
                    Some(ch) if closing_for(ch).is_some() || ch == '.' => {
                        self.at += 1;
                        if ch == '.' {
                            String::new()
                        } else {
                            ch.to_string()
                        }
                    }
                    _ => String::new(),
                };
                let items = self.delimited_items(&['┤']);
                let mut close = String::new();
                if self.peek() == Some('┤') {
                    self.at += 1;
                    if let Some(ch) = self
                        .peek()
                        .filter(|c| is_closing(*c) || *c == '|' || *c == '.')
                    {
                        self.at += 1;
                        if ch != '.' {
                            close = ch.to_string();
                        }
                    }
                }
                Some(Node::Delimiter {
                    open,
                    close,
                    separator: None,
                    items,
                })
            }
            open if closing_for(open).is_some() => {
                let close = closing_for(open)?;
                // A bar opens a delimiter only when another closes it.
                if (open == '|' || open == '‖')
                    && !self.chars[self.at + 1..].contains(&close)
                    && !self.chars[self.at + 1..].contains(&'┤')
                {
                    return None;
                }
                let start = self.at;
                self.at += 1;
                let items = self.delimited_items(&[close, '┤']);
                let separator = (items.len() > 1).then(|| "|".to_string());
                match self.peek() {
                    Some(c) if c == close => {
                        self.at += 1;
                        Some(Node::Delimiter {
                            open: open.to_string(),
                            close: close.to_string(),
                            separator,
                            items,
                        })
                    }
                    Some('┤') => {
                        // `{■(…)┤`: closed by nothing, or by what follows ┤.
                        self.at += 1;
                        let mut close = String::new();
                        if let Some(ch) = self
                            .peek()
                            .filter(|c| is_closing(*c) || *c == '|' || *c == '.')
                        {
                            self.at += 1;
                            if ch != '.' {
                                close = ch.to_string();
                            }
                        }
                        Some(Node::Delimiter {
                            open: open.to_string(),
                            close,
                            separator,
                            items,
                        })
                    }
                    _ => {
                        // Unclosed: the bracket is text.
                        self.at = start;
                        None
                    }
                }
            }
            '■' | '⒨' | '⒩' => {
                self.at += 1;
                if self.peek() != Some('(') {
                    return None;
                }
                self.at += 1;
                let mut rows = vec![vec![]];
                loop {
                    let cell = self.expression(&['&', '@', ')']);
                    rows.last_mut().unwrap().push(cell);
                    match self.peek() {
                        Some('&') => self.at += 1,
                        Some('@') => {
                            self.at += 1;
                            rows.push(Vec::new());
                        }
                        Some(')') => {
                            self.at += 1;
                            break;
                        }
                        _ => break,
                    }
                }
                let matrix = Node::Matrix(rows);
                Some(match c {
                    '⒨' => Node::Delimiter {
                        open: "(".into(),
                        close: ")".into(),
                        separator: None,
                        items: vec![vec![matrix]],
                    },
                    '⒩' => Node::Delimiter {
                        open: "‖".into(),
                        close: "‖".into(),
                        separator: None,
                        items: vec![vec![matrix]],
                    },
                    _ => matrix,
                })
            }
            '█' | 'Ⓒ' => {
                self.at += 1;
                if self.peek() != Some('(') {
                    return None;
                }
                self.at += 1;
                let mut rows = Vec::new();
                loop {
                    rows.push(self.expression(&['@', ')']));
                    match self.peek() {
                        Some('@') => self.at += 1,
                        Some(')') => {
                            self.at += 1;
                            break;
                        }
                        _ => break,
                    }
                }
                let array = Node::EquationArray(rows);
                Some(if c == 'Ⓒ' {
                    Node::Delimiter {
                        open: "{".into(),
                        close: String::new(),
                        separator: None,
                        items: vec![vec![array]],
                    }
                } else {
                    array
                })
            }
            '□' | '▭' | '¯' | '▁' | '⟡' | '⏞' | '⏟' | '⏜' | '⏝' | '⎴' | '⎵' => {
                self.at += 1;
                self.skip_spaces();
                let base = self.argument();
                Some(match c {
                    '□' => Node::BoxGroup(base),
                    '▭' => Node::BorderBox(base),
                    '¯' => Node::Bar { top: true, base },
                    '▁' => Node::Bar { top: false, base },
                    '⟡' => Node::Phantom(base),
                    _ => {
                        let top = matches!(c, '⏞' | '⏜' | '⎴');
                        let group = Node::GroupChar { chr: c, top, base };
                        // A label over or under the group: `⏟(a+b)_k`.
                        let script = if top { '^' } else { '_' };
                        if self.peek() == Some(script) {
                            self.at += 1;
                            let lim = self.argument();
                            Node::Limit {
                                lower: !top,
                                base: vec![group],
                                lim,
                            }
                        } else {
                            group
                        }
                    }
                })
            }
            _ => None,
        }
    }

    /// Items of a delimiter, split at separators, up to `close`.
    fn delimited_items(&mut self, close: &[char]) -> Vec<Vec<Node>> {
        let mut stops: Vec<char> = close.to_vec();
        stops.extend(['│', '∣']);
        let mut items = Vec::new();
        loop {
            items.push(apply_functions(self.expression(&stops)));
            match self.peek() {
                Some('│') | Some('∣') => self.at += 1,
                _ => break,
            }
        }
        items
    }

    /// `^` or `_` after a base, and the other one if it follows.
    fn scripts(&mut self, out: &mut Vec<Node>) -> Node {
        let mut sub = None;
        let mut sup = None;
        let base = take_base(out);
        while let Some(c) = self.peek().filter(|c| *c == '^' || *c == '_') {
            self.at += 1;
            let arg = self.argument();
            if c == '_' && sub.is_none() {
                sub = Some(arg);
            } else if c == '^' && sup.is_none() {
                sup = Some(arg);
            } else {
                break;
            }
        }
        if base.is_empty() {
            // `_1^2 F`: scripts before a base are prescripts.
            self.skip_spaces();
            let base = self.operand_with_scripts_keep();
            return Node::PreScript {
                sub: sub.unwrap_or_default(),
                sup: sup.unwrap_or_default(),
                base,
            };
        }
        // `lim_(n→∞)`: a limit under the name.
        if let ([Node::Text(name)], Some(lim), None) = (base.as_slice(), &sub, &sup) {
            if matches!(
                name.as_str(),
                "lim" | "max" | "min" | "sup" | "inf" | "liminf" | "limsup"
            ) {
                return Node::Limit {
                    lower: true,
                    base: vec![Node::Text(name.clone())],
                    lim: lim.clone(),
                };
            }
        }
        Node::Script { base, sub, sup }
    }

    /// A script's or a group character's argument: one operand, its
    /// parentheses dropped.
    fn argument(&mut self) -> Vec<Node> {
        match self.peek() {
            Some('(') => {
                let start = self.at;
                self.at += 1;
                let content = self.expression(&[')']);
                if self.peek() == Some(')') {
                    self.at += 1;
                    apply_functions(content)
                } else {
                    self.at = start + 1;
                    vec![Node::Text("(".into())]
                }
            }
            Some('〖') => {
                self.at += 1;
                let content = self.expression(&['〗']);
                if self.peek() == Some('〗') {
                    self.at += 1;
                }
                apply_functions(content)
            }
            Some(c) if c == '+' || c == '-' || c == '−' || c == '±' || c == '∓' => {
                self.at += 1;
                let mut text = c.to_string();
                text.push_str(&self.operand_run());
                vec![Node::Text(text)]
            }
            Some(c) if is_operand_char(c) => vec![Node::Text(self.operand_run())],
            Some(_) => {
                if let Some(node) = self.structure() {
                    return vec![node];
                }
                let c = self.peek().unwrap_or(' ');
                self.at += 1;
                vec![Node::Text(c.to_string())]
            }
            None => Vec::new(),
        }
    }

    fn operand_run(&mut self) -> String {
        let mut text = String::new();
        while let Some(c) = self.peek().filter(|c| is_operand_char(*c)) {
            text.push(c);
            self.at += 1;
        }
        text
    }

    /// An operand (parentheses dropped) with any scripts it takes.
    fn operand_with_scripts(&mut self) -> Vec<Node> {
        let base = self.argument();
        self.attach_scripts(base)
    }

    /// An operand that keeps its parentheses as a delimiter (an n-ary
    /// operand, a function argument), with any scripts it takes.
    fn operand_with_scripts_keep(&mut self) -> Vec<Node> {
        let base = match self.peek() {
            Some(c) if closing_for(c).is_some() || c == '〖' || c == '├' => self
                .structure()
                .map_or_else(|| self.argument(), |node| vec![node]),
            _ => self.argument(),
        };
        self.attach_scripts(base)
    }

    fn attach_scripts(&mut self, base: Vec<Node>) -> Vec<Node> {
        if matches!(self.peek(), Some('^') | Some('_')) {
            let mut nodes = base;
            let node = self.scripts(&mut nodes);
            nodes.push(node);
            nodes
        } else {
            base
        }
    }
}

fn push_char(out: &mut Vec<Node>, ch: char) {
    match out.last_mut() {
        Some(Node::Text(text)) if !text.is_empty() => text.push(ch),
        Some(Node::Text(_)) => {
            // After a space separator, a new run.
            out.pop();
            out.push(Node::Text(String::new()));
            out.push(Node::Text(ch.to_string()));
        }
        _ => out.push(Node::Text(ch.to_string())),
    }
}

fn strip_parens(nodes: Vec<Node>) -> Vec<Node> {
    if let [Node::Delimiter {
        open, close, items, ..
    }] = nodes.as_slice()
    {
        if open == "(" && close == ")" && items.len() == 1 {
            return items[0].clone();
        }
    }
    nodes
}

/// Removes the operand before a fraction bar from `out`: the run of operands
/// back to the last operator or space, its grouping parentheses dropped.
fn take_operand(out: &mut Vec<Node>) -> Vec<Node> {
    let mut taken: Vec<Node> = Vec::new();
    while let Some(last) = out.pop() {
        match last {
            Node::Text(text) => {
                if text.is_empty() {
                    if taken.is_empty() {
                        continue;
                    }
                    out.push(Node::Text(text));
                    break;
                }
                let chars: Vec<char> = text.chars().collect();
                let mut split = chars.len();
                while split > 0 && is_operand_char(chars[split - 1]) {
                    split -= 1;
                }
                let operand: String = chars[split..].iter().collect();
                let rest: String = chars[..split].iter().collect();
                if !operand.is_empty() {
                    taken.insert(0, Node::Text(operand));
                }
                if !rest.is_empty() {
                    out.push(Node::Text(rest));
                    break;
                }
            }
            other => taken.insert(0, other),
        }
    }
    strip_parens(taken)
}

/// Removes a script's base from `out`: a function name, a number, one
/// character, or the structure before it.
fn take_base(out: &mut Vec<Node>) -> Vec<Node> {
    match out.pop() {
        None => Vec::new(),
        Some(Node::Text(text)) => {
            if text.is_empty() {
                return Vec::new();
            }
            let chars: Vec<char> = text.chars().collect();
            let last = chars[chars.len() - 1];
            let mut split = chars.len() - 1;
            if last.is_ascii_digit() {
                while split > 0 && (chars[split - 1].is_ascii_digit() || chars[split - 1] == '.') {
                    split -= 1;
                }
            } else if last.is_alphabetic() {
                // A function name at the end is the base whole.
                let mut start = chars.len();
                while start > 0 && chars[start - 1].is_ascii_alphabetic() {
                    start -= 1;
                }
                if let Some(name_start) = (start..chars.len()).find(|&s| {
                    is_function_name(&chars[s..].iter().collect::<String>())
                        && (s == start || s == 0)
                }) {
                    split = name_start;
                }
            } else if super::super::styled::alphabet::char_class(last)
                != super::super::styled::AtomClass::Ord
            {
                // After an operator there is no base.
                out.push(Node::Text(text));
                return Vec::new();
            }
            let base: String = chars[split..].iter().collect();
            let rest: String = chars[..split].iter().collect();
            if !rest.is_empty() {
                out.push(Node::Text(rest));
            }
            vec![Node::Text(base)]
        }
        Some(other) => vec![other],
    }
}

/// `sin x`, `log(x)`, `lim_(n→∞) …`: a function name applies to the operand
/// after it.
fn apply_functions(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let mut iter = nodes.into_iter().peekable();
    while let Some(node) = iter.next() {
        // The name, if this node ends with one.
        let name: Option<(Option<String>, Node)> = match &node {
            Node::Text(text) => {
                let chars: Vec<char> = text.chars().collect();
                let mut start = chars.len();
                while start > 0 && chars[start - 1].is_ascii_alphabetic() {
                    start -= 1;
                }
                let word: String = chars[start..].iter().collect();
                (is_function_name(&word)).then(|| {
                    let rest: String = chars[..start].iter().collect();
                    ((!rest.is_empty()).then_some(rest), Node::Text(word))
                })
            }
            Node::Script {
                base,
                sub: _,
                sup: _,
            } if matches!(base.as_slice(), [Node::Text(t)] if is_function_name(t)) => {
                Some((None, node.clone()))
            }
            Node::Limit { base, .. } if matches!(base.as_slice(), [Node::Text(t)] if is_function_name(t)) => {
                Some((None, node.clone()))
            }
            _ => None,
        };
        let Some((rest, name)) = name else {
            out.push(node);
            continue;
        };
        // Skip the space separators to the argument.
        while matches!(iter.peek(), Some(Node::Text(t)) if t.is_empty()) {
            iter.next();
        }
        let Some(next) = iter.next() else {
            out.push(node);
            continue;
        };
        let (arg, remainder) = match next {
            Node::Text(text) => {
                // The argument is the operand at the start of the text.
                let chars: Vec<char> = text.chars().collect();
                let mut end = 0;
                while end < chars.len() && is_operand_char(chars[end]) {
                    end += 1;
                }
                if end == 0 {
                    end = 1.min(chars.len());
                }
                let arg: String = chars[..end].iter().collect();
                let rest: String = chars[end..].iter().collect();
                (
                    vec![Node::Text(arg)],
                    (!rest.is_empty()).then_some(Node::Text(rest)),
                )
            }
            other => (vec![other], None),
        };
        if let Some(rest) = rest {
            out.push(Node::Text(rest));
        }
        out.push(Node::Function {
            name: vec![name],
            arg: apply_functions(arg),
        });
        if let Some(remainder) = remainder {
            out.push(remainder);
        }
    }
    out.into_iter()
        .filter(|n| !matches!(n, Node::Text(t) if t.is_empty()))
        .collect()
}

// ---- Writing OMML as UnicodeMath ------------------------------------------

/// Writes math elements as UnicodeMath.
pub fn write(elements: &[MathElement]) -> String {
    let mut out = String::new();
    for e in elements.iter().filter(|e| is_content(e)) {
        write_element(e, &mut out);
    }
    out
}

fn write_children(e: &MathElement) -> String {
    let mut out = String::new();
    for c in e.children.iter().filter(|c| is_content(c)) {
        write_element(c, &mut out);
    }
    out
}

/// An argument: in parentheses unless it is one operand.
fn argument(e: Option<&MathElement>) -> String {
    let text = e.map(write_children).unwrap_or_default();
    if is_single_operand(&text) {
        text
    } else {
        format!("({text})")
    }
}

fn is_single_operand(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    match chars.as_slice() {
        [] => false,
        [_] => true,
        _ => {
            chars.iter().all(|c| is_operand_char(*c))
                || (text.starts_with('(') && text.ends_with(')') && balanced_outer(text))
        }
    }
}

/// Whether the outer parentheses of `text` enclose all of it.
fn balanced_outer(text: &str) -> bool {
    let mut depth = 0i32;
    let count = text.chars().count();
    for (i, c) in text.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i + 1 != count {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn write_element(e: &MathElement, out: &mut String) {
    use MathElementKind as K;
    match e.kind {
        K::Run => {
            let (_, _, nor) = run_style(e);
            let text = text_of(e);
            if nor {
                out.push('"');
                out.push_str(&text);
                out.push('"');
            } else {
                for c in text.chars() {
                    if matches!(c, '/' | '^' | '_' | '▒' | '"' | '\\' | '@' | '&') {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
        K::Text => out.push_str(e.text.as_deref().unwrap_or_default()),
        K::Fraction => {
            let op = match property(e, "fPr", "type") {
                Some("skw") => '⁄',
                Some("lin") => '∕',
                Some("noBar") => '¦',
                _ => '/',
            };
            out.push_str(&argument(child(e, K::Numerator)));
            out.push(op);
            out.push_str(&argument(child(e, K::Denominator)));
        }
        K::Superscript | K::Subscript | K::SubSup => {
            out.push_str(&script_base(child(e, K::Base)));
            if let Some(sub) = child(e, K::SubArgument) {
                out.push('_');
                out.push_str(&argument(Some(sub)));
            }
            if let Some(sup) = child(e, K::SuperArgument) {
                out.push('^');
                out.push_str(&argument(Some(sup)));
            }
        }
        K::PreSubSuper => {
            out.push('_');
            out.push_str(&argument(child(e, K::SubArgument)));
            out.push('^');
            out.push_str(&argument(child(e, K::SuperArgument)));
            out.push(' ');
            out.push_str(&argument(child(e, K::Base)));
        }
        K::Radical => {
            let degree = child(e, K::Degree)
                .filter(|_| !flag(e, "radPr", "degHide"))
                .map(write_children)
                .filter(|d| !d.is_empty());
            let base = child(e, K::Base).map(write_children).unwrap_or_default();
            match degree.as_deref() {
                Some("3") => {
                    out.push('∛');
                    out.push_str(&wrap(&base));
                }
                Some("4") => {
                    out.push('∜');
                    out.push_str(&wrap(&base));
                }
                Some(degree) => {
                    out.push_str(&format!("√({degree}&{base})"));
                }
                None => {
                    out.push('√');
                    out.push_str(&wrap(&base));
                }
            }
        }
        K::Nary => {
            let chr = property(e, "naryPr", "chr").unwrap_or("∫");
            out.push_str(chr);
            if !flag(e, "naryPr", "subHide") {
                if let Some(sub) = child(e, K::SubArgument).filter(|s| !s.children.is_empty()) {
                    out.push('_');
                    out.push_str(&argument(Some(sub)));
                }
            }
            if !flag(e, "naryPr", "supHide") {
                if let Some(sup) = child(e, K::SuperArgument).filter(|s| !s.children.is_empty()) {
                    out.push('^');
                    out.push_str(&argument(Some(sup)));
                }
            }
            out.push('▒');
            out.push_str(&argument(child(e, K::Base)));
        }
        K::Delimiter => {
            let open = property(e, "dPr", "begChr").unwrap_or("(");
            let close = property(e, "dPr", "endChr").unwrap_or(")");
            let separator = property(e, "dPr", "sepChr").unwrap_or("│");
            let separator = if separator == "|" { "│" } else { separator };
            let plain = closing_for(open.chars().next().unwrap_or(' '))
                .is_some_and(|c| c.to_string() == close);
            if !plain {
                out.push('├');
            }
            out.push_str(if open.is_empty() { "." } else { open });
            let items: Vec<String> = children(e, K::Base).map(write_children).collect();
            out.push_str(&items.join(separator));
            if !plain {
                out.push('┤');
            }
            out.push_str(if close.is_empty() { "." } else { close });
        }
        K::Function => {
            out.push_str(
                &child(e, K::FunctionName)
                    .map(write_children)
                    .unwrap_or_default(),
            );
            out.push('⁡');
            out.push_str(&argument(child(e, K::Base)));
        }
        K::Matrix => {
            let rows: Vec<String> = children(e, K::MatrixRow)
                .map(|row| {
                    children(row, K::Base)
                        .map(write_children)
                        .collect::<Vec<_>>()
                        .join("&")
                })
                .collect();
            out.push_str(&format!("■({})", rows.join("@")));
        }
        K::EquationArray => {
            let rows: Vec<String> = children(e, K::Base).map(write_children).collect();
            out.push_str(&format!("█({})", rows.join("@")));
        }
        K::Accent => {
            let chr = property(e, "accPr", "chr").unwrap_or("\u{0302}");
            out.push_str(&script_base(child(e, K::Base)));
            out.push_str(chr);
        }
        K::Bar => {
            let top = property(e, "barPr", "pos") == Some("top");
            out.push(if top { '¯' } else { '▁' });
            out.push_str(&wrap(
                &child(e, K::Base).map(write_children).unwrap_or_default(),
            ));
        }
        K::GroupChar => {
            let chr = property(e, "groupChrPr", "chr").unwrap_or("⏟");
            out.push_str(chr);
            out.push_str(&wrap(
                &child(e, K::Base).map(write_children).unwrap_or_default(),
            ));
        }
        K::LimitLower | K::LimitUpper => {
            let base = child(e, K::Base);
            // A group character's label is written as its script.
            let group = base.and_then(|b| {
                let mut content = b.children.iter().filter(|c| is_content(c));
                let first = content.next()?;
                (content.next().is_none() && first.kind == K::GroupChar).then_some(first)
            });
            if let Some(group) = group {
                write_element(group, out);
                out.push(if e.kind == K::LimitLower { '_' } else { '^' });
            } else {
                out.push_str(&script_base(base));
                out.push(if e.kind == K::LimitLower {
                    '┬'
                } else {
                    '┴'
                });
            }
            out.push_str(&argument(child(e, K::Limit)));
        }
        K::Box => {
            out.push('□');
            out.push_str(&wrap(
                &child(e, K::Base).map(write_children).unwrap_or_default(),
            ));
        }
        K::BorderBox => {
            out.push('▭');
            out.push_str(&wrap(
                &child(e, K::Base).map(write_children).unwrap_or_default(),
            ));
        }
        K::Phantom => {
            out.push('⟡');
            out.push_str(&wrap(
                &child(e, K::Base).map(write_children).unwrap_or_default(),
            ));
        }
        _ => out.push_str(&write_children(e)),
    }
}

fn wrap(text: &str) -> String {
    if text.chars().count() == 1 {
        text.to_string()
    } else {
        format!("({text})")
    }
}

/// A script's base: one operand, else grouped invisibly.
fn script_base(e: Option<&MathElement>) -> String {
    let text = e.map(write_children).unwrap_or_default();
    if is_single_operand(&text) || text.is_empty() {
        text
    } else {
        format!("〖{text}〗")
    }
}
