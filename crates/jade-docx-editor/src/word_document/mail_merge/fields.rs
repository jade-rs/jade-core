//! Merge fields evaluated for a recipient, as one more kind of field.
//!
//! The walk goes through the body in document order, finding each complex
//! field (`w:fldChar` begin … separate … end) and simple one
//! (`w:fldSimple`) among a container's runs. A nested field is evaluated
//! first and its value stands in its parent's instruction as one token, as
//! Word reads it. The record a field reads is the merge's current one;
//! `NEXT` and a true `NEXTIF` move on, a true `SKIPIF` drops the copy. What
//! happens to a merge field is the walk's [`FieldMode`]: its result
//! replaced (preview), its «name» put back (preview off), or the whole field
//! replaced by its value (a merged copy). Other fields are left as they are.

use std::collections::HashMap;

use super::address::{render_format, AddressField};
use super::{MainDocumentType, MergeRequest};
use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode, XmlNodeId};

/// What the walk does to a merge field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldMode {
    /// The whole field becomes its value: a merged copy.
    Merge,
    /// The field's result becomes its value: 結果のプレビュー.
    Preview,
    /// The field's result becomes its «name»: preview off.
    Codes,
}

/// Something wrong with a merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeProblem {
    /// The data source has no columns.
    EmptyDataSource,
    /// No recipient is chosen.
    NoRecipients,
    /// A merge field names a column the data source does not have.
    MissingColumn(String),
    /// An address block or greeting line has no name or address column to
    /// draw from: フィールドの対応 is needed.
    UnmatchedAddress,
}

/// One token of a field instruction: its text, and whether it was quoted
/// (or a nested field's value), which keeps it from being an operator.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    text: String,
    literal: bool,
}

/// A field instruction: text, and nested fields' values.
#[derive(Clone, Debug)]
enum Piece {
    Text(String),
    Nested(String),
}

fn tokenize(pieces: &[Piece]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut literal = false;
    let mut started = false;
    let mut quoted = false;
    let finish =
        |tokens: &mut Vec<Token>, current: &mut String, literal: &mut bool, started: &mut bool| {
            if *started {
                tokens.push(Token {
                    text: std::mem::take(current),
                    literal: *literal,
                });
            }
            *literal = false;
            *started = false;
        };
    for piece in pieces {
        match piece {
            Piece::Nested(value) => {
                current.push_str(value);
                literal = true;
                started = true;
            }
            Piece::Text(text) => {
                let mut chars = text.chars().peekable();
                while let Some(ch) = chars.next() {
                    if quoted {
                        match ch {
                            '\\' if matches!(chars.peek(), Some('"' | '\\')) => {
                                current.push(chars.next().unwrap_or('\\'));
                            }
                            '"' | '“' | '”' => quoted = false,
                            other => current.push(other),
                        }
                        continue;
                    }
                    match ch {
                        '"' | '“' | '”' => {
                            quoted = true;
                            literal = true;
                            started = true;
                        }
                        ch if ch.is_whitespace() => {
                            finish(&mut tokens, &mut current, &mut literal, &mut started)
                        }
                        '\\' => {
                            finish(&mut tokens, &mut current, &mut literal, &mut started);
                            current.push('\\');
                            if let Some(next) = chars.next() {
                                current.push(next);
                            }
                            started = true;
                            finish(&mut tokens, &mut current, &mut literal, &mut started);
                        }
                        other => {
                            current.push(other);
                            started = true;
                        }
                    }
                }
            }
        }
    }
    finish(&mut tokens, &mut current, &mut literal, &mut started);
    tokens
}

/// A switch's argument: `\b "text"` gives `text`.
fn switch_argument<'a>(tokens: &'a [Token], switch: &str) -> Option<&'a str> {
    let at = tokens
        .iter()
        .position(|token| !token.literal && token.text.eq_ignore_ascii_case(switch))?;
    tokens.get(at + 1).map(|token| token.text.as_str())
}

fn has_switch(tokens: &[Token], switch: &str) -> bool {
    tokens
        .iter()
        .any(|token| !token.literal && token.text.eq_ignore_ascii_case(switch))
}

/// A merge field's value with its general switches applied: `\*` case,
/// `\#` number, `\b` / `\f` text around a value.
fn apply_switches(value: String, tokens: &[Token]) -> String {
    let mut value = value;
    let mut at = 0;
    while at < tokens.len() {
        let token = &tokens[at];
        if !token.literal && token.text == "\\*" {
            if let Some(format) = tokens.get(at + 1) {
                value = match format.text.to_ascii_lowercase().as_str() {
                    "upper" => value.to_uppercase(),
                    "lower" => value.to_lowercase(),
                    "firstcap" => {
                        let mut chars = value.chars();
                        match chars.next() {
                            Some(first) => first.to_uppercase().chain(chars).collect(),
                            None => value,
                        }
                    }
                    "caps" => value
                        .split(' ')
                        .map(|word| {
                            let mut chars = word.chars();
                            match chars.next() {
                                Some(first) => first
                                    .to_uppercase()
                                    .chain(chars.flat_map(char::to_lowercase))
                                    .collect::<String>(),
                                None => String::new(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => value,
                };
            }
            at += 2;
            continue;
        }
        if !token.literal && token.text == "\\#" {
            if let (Some(picture), Ok(number)) = (
                tokens.get(at + 1),
                value.trim().replace(',', "").parse::<f64>(),
            ) {
                value = format_number(number, &picture.text);
            }
            at += 2;
            continue;
        }
        at += 1;
    }
    if !value.is_empty() {
        if let Some(before) = switch_argument(tokens, "\\b") {
            value.insert_str(0, before);
        }
        if let Some(after) = switch_argument(tokens, "\\f") {
            value.push_str(after);
        }
    }
    value
}

/// A number in a `\#` picture: its decimals from the `0`s after the point,
/// thousands separators when it has a `,`.
fn format_number(number: f64, picture: &str) -> String {
    let decimals = picture
        .split_once('.')
        .map(|(_, fraction)| {
            fraction
                .chars()
                .filter(|ch| *ch == '0' || *ch == '#')
                .count()
        })
        .unwrap_or(0);
    let text = format!("{:.*}", decimals, number.abs());
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    let whole = if picture.contains(',') {
        let digits: Vec<char> = whole.chars().collect();
        let mut grouped = String::new();
        for (index, digit) in digits.iter().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(*digit);
        }
        grouped
    } else {
        whole.to_string()
    };
    let prefix: String = picture
        .chars()
        .take_while(|ch| !matches!(ch, '0' | '#' | ',' | '.'))
        .collect();
    let sign = if number < 0.0 { "-" } else { "" };
    match fraction {
        Some(fraction) => format!("{sign}{prefix}{whole}.{fraction}"),
        None => format!("{sign}{prefix}{whole}"),
    }
}

/// How a comparison of two field values comes out (`IF`, `NEXTIF`,
/// `SKIPIF`, `COMPARE`): numbers compare as numbers; `=` and `<>` take `*`
/// and `?` in a quoted right side as wildcards.
fn compare(left: &Token, operator: &str, right: &Token) -> bool {
    let numbers = (
        left.text.trim().replace(',', "").parse::<f64>(),
        right.text.trim().replace(',', "").parse::<f64>(),
    );
    let ordering = match numbers {
        (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
        _ => left.text.cmp(&right.text),
    };
    let equal = if right.literal && (right.text.contains('*') || right.text.contains('?')) {
        wildcard(&right.text, &left.text)
    } else {
        ordering == std::cmp::Ordering::Equal
    };
    match operator {
        "=" => equal,
        "<>" => !equal,
        "<" => ordering == std::cmp::Ordering::Less,
        ">" => ordering == std::cmp::Ordering::Greater,
        "<=" => ordering != std::cmp::Ordering::Greater,
        ">=" => ordering != std::cmp::Ordering::Less,
        _ => false,
    }
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.first() {
            None => text.is_empty(),
            Some('*') => (0..=text.len()).any(|skip| matches(&pattern[1..], &text[skip..])),
            Some('?') => !text.is_empty() && matches(&pattern[1..], &text[1..]),
            Some(ch) => text.first() == Some(ch) && matches(&pattern[1..], &text[1..]),
        }
    }
    matches(&pattern, &text)
}

/// What a field comes to.
struct Value {
    /// Whether it is the merge's to rewrite: a merge field, a rule, a REF
    /// to a bookmark a rule set. Other fields are left alone.
    merge: bool,
    text: String,
    /// Its «name» with preview off.
    codes: Option<String>,
}

impl Value {
    fn other(text: String) -> Self {
        Self {
            merge: false,
            text,
            codes: None,
        }
    }
}

/// The merge's state as the walk goes: the record, the copies so far.
struct Env<'a> {
    request: &'a MergeRequest,
    /// Index into `request.records`.
    cursor: usize,
    /// The copy's number, for MERGESEQ.
    sequence: usize,
    bookmarks: HashMap<String, String>,
    problems: Vec<MergeProblem>,
    skip: bool,
    mode: FieldMode,
}

impl Env<'_> {
    fn record(&self) -> Option<usize> {
        self.request.records.get(self.cursor).copied()
    }

    fn column_value(&self, column: usize) -> String {
        self.record()
            .map(|record| self.request.data.value(record, column).to_string())
            .unwrap_or_default()
    }

    fn address_value(&self, field: AddressField) -> String {
        self.request
            .mapping
            .column(field)
            .map(|column| self.column_value(column))
            .unwrap_or_default()
    }

    fn problem(&mut self, problem: MergeProblem) {
        if !self.problems.contains(&problem) {
            self.problems.push(problem);
        }
    }

    fn advance(&mut self) {
        if self.mode != FieldMode::Codes {
            self.cursor += 1;
        }
    }

    fn evaluate(&mut self, pieces: &[Piece], result: &str) -> Value {
        let tokens = tokenize(pieces);
        let Some(name) = tokens.first().map(|token| token.text.to_ascii_uppercase()) else {
            return Value::other(result.to_string());
        };
        let rule = |text: String, codes: &str| Value {
            merge: true,
            text,
            codes: Some(codes.to_string()),
        };
        let condition = |env: &Self| {
            let left = tokens.get(1)?;
            let operator = tokens.get(2)?;
            let right = tokens.get(3)?;
            let _ = env;
            Some(compare(left, &operator.text, right))
        };
        match name.as_str() {
            "MERGEFIELD" => {
                let Some(field) = tokens.get(1).map(|token| token.text.clone()) else {
                    return Value::other(result.to_string());
                };
                let mapped = has_switch(&tokens, "\\m");
                let column = if mapped {
                    AddressField::from_odso(&field.replace('_', " "))
                        .and_then(|address| self.request.mapping.column(address))
                        .or_else(|| self.request.data.column(&field))
                } else {
                    self.request.data.column(&field)
                };
                let codes = format!("«{field}»");
                match column {
                    Some(column) => {
                        let value = apply_switches(self.column_value(column), &tokens);
                        rule(value, &codes)
                    }
                    None => {
                        self.problem(MergeProblem::MissingColumn(field.clone()));
                        let text = match self.mode {
                            FieldMode::Merge => String::new(),
                            _ => codes.clone(),
                        };
                        rule(text, &codes)
                    }
                }
            }
            "ADDRESSBLOCK" => {
                let japanese = switch_argument(&tokens, "\\l") == Some("1041");
                let format = switch_argument(&tokens, "\\f")
                    .map(str::to_string)
                    .unwrap_or_else(|| super::AddressBlockOptions::new(japanese).format());
                let mode = switch_argument(&tokens, "\\c").unwrap_or("0").to_string();
                let except = switch_argument(&tokens, "\\e").unwrap_or("").to_string();
                let has_any = [
                    AddressField::LastName,
                    AddressField::FirstName,
                    AddressField::Address1,
                    AddressField::Company,
                ]
                .into_iter()
                .any(|field| self.request.mapping.column(field).is_some());
                if !has_any {
                    self.problem(MergeProblem::UnmatchedAddress);
                }
                let text =
                    render_format(
                        &format,
                        &|field| self.address_value(field),
                        &|country| match mode.as_str() {
                            "1" => true,
                            "2" => !country.trim().eq_ignore_ascii_case(except.trim()),
                            _ => false,
                        },
                    );
                rule(text, "«AddressBlock»")
            }
            "GREETINGLINE" => {
                let japanese = switch_argument(&tokens, "\\l") == Some("1041");
                let format = switch_argument(&tokens, "\\f")
                    .map(str::to_string)
                    .unwrap_or_else(|| super::GreetingLineOptions::new(japanese).format());
                let invalid = switch_argument(&tokens, "\\e").unwrap_or("").to_string();
                let named = [
                    AddressField::LastName,
                    AddressField::FirstName,
                    AddressField::CourtesyTitle,
                ]
                .into_iter()
                .any(|field| !self.address_value(field).trim().is_empty());
                let text = if named {
                    render_format(&format, &|field| self.address_value(field), &|_| false)
                } else {
                    invalid
                };
                rule(text, "«GreetingLine»")
            }
            "IF" => {
                let chosen = match condition(self) {
                    Some(true) => tokens.get(4),
                    Some(false) => tokens.get(5),
                    None => None,
                };
                rule(
                    chosen.map(|token| token.text.clone()).unwrap_or_default(),
                    result,
                )
            }
            "COMPARE" => rule(
                if condition(self).unwrap_or(false) {
                    "1"
                } else {
                    "0"
                }
                .to_string(),
                result,
            ),
            "NEXT" => {
                self.advance();
                rule(String::new(), "«Next Record»")
            }
            "NEXTIF" => {
                if condition(self).unwrap_or(false) {
                    self.advance();
                }
                rule(String::new(), "«Next Record If»")
            }
            "SKIPIF" => {
                if self.mode == FieldMode::Merge && condition(self).unwrap_or(false) {
                    self.skip = true;
                }
                rule(String::new(), "«Skip Record If»")
            }
            "MERGEREC" => rule(
                self.record()
                    .map(|record| (record + 1).to_string())
                    .unwrap_or_default(),
                "«Merge Record #»",
            ),
            "MERGESEQ" => rule(self.sequence.to_string(), "«Merge Sequence #»"),
            "SET" => {
                if let (Some(bookmark), Some(value)) = (tokens.get(1), tokens.get(2)) {
                    self.bookmarks
                        .insert(bookmark.text.to_lowercase(), value.text.clone());
                }
                rule(String::new(), "")
            }
            "ASK" => {
                if let Some(bookmark) = tokens.get(1) {
                    let default = switch_argument(&tokens, "\\d").unwrap_or("").to_string();
                    self.bookmarks.insert(bookmark.text.to_lowercase(), default);
                }
                rule(String::new(), "")
            }
            "FILLIN" => {
                let default = switch_argument(&tokens, "\\d")
                    .map(str::to_string)
                    .unwrap_or_else(|| result.to_string());
                rule(default.clone(), &default)
            }
            "REF" => match tokens
                .get(1)
                .and_then(|bookmark| self.bookmarks.get(&bookmark.text.to_lowercase()))
            {
                Some(value) => rule(value.clone(), result),
                None => Value::other(result.to_string()),
            },
            _ => Value::other(result.to_string()),
        }
    }
}

/// A field open in the walk until its end.
struct Open {
    begin: usize,
    pieces: Vec<Piece>,
    separate: Option<usize>,
    result: String,
}

/// What to do to a span of a container's children.
enum Edit {
    /// Replace `begin..=end` with the text's runs.
    Whole {
        begin: usize,
        end: usize,
        text: String,
    },
    /// Replace the result between `separate` (if any) and `end`.
    Result {
        separate: Option<usize>,
        end: usize,
        text: String,
    },
}

fn field_char(run: &XmlElement) -> Option<&str> {
    run.children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:fldChar" => element
            .attributes
            .iter()
            .find(|attribute| attribute.name == "w:fldCharType")
            .map(|attribute| attribute.value.as_str()),
        _ => None,
    })
}

fn element_text(element: &XmlElement, name: &str) -> String {
    let mut text = String::new();
    for node in &element.children {
        if let XmlNode::Element(child) = node {
            if child.name == name {
                for node in &child.children {
                    if let XmlNode::Text(part) = node {
                        text.push_str(part);
                    }
                }
            } else if child.name == "w:tab" && name == "w:t" {
                text.push('\t');
            } else if child.name == "w:br" && name == "w:t" {
                text.push('\n');
            }
        }
    }
    text
}

/// A run's properties, for the runs a result is written in.
fn run_properties(run: &XmlElement) -> Option<XmlElement> {
    run.children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:rPr" => Some(element.clone()),
        _ => None,
    })
}

fn text_element(name: &str, text: &str) -> XmlNode {
    XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes: vec![XmlAttribute {
            name: "xml:space".to_string(),
            value: "preserve".to_string(),
        }],
        children: vec![XmlNode::Text(text.to_string())],
    })
}

fn bare(name: &str) -> XmlElement {
    XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes: Vec::new(),
        children: Vec::new(),
    }
}

/// `text` as runs in `properties`, a line break between its lines.
fn text_runs(text: &str, properties: Option<&XmlElement>) -> Vec<XmlNode> {
    let mut runs = Vec::new();
    let run = |child: XmlNode| {
        let mut element = bare("w:r");
        if let Some(properties) = properties {
            element.children.push(XmlNode::Element(properties.clone()));
        }
        element.children.push(child);
        XmlNode::Element(element)
    };
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            runs.push(run(XmlNode::Element(bare("w:br"))));
        }
        if !line.is_empty() {
            runs.push(run(text_element("w:t", line)));
        }
    }
    runs
}

/// Evaluates the fields among `children` in order, and the fields of the
/// containers among them, rewriting them as the mode says.
#[allow(clippy::needless_range_loop)]
fn walk(children: &mut Vec<XmlNode>, env: &mut Env) {
    let mut stack: Vec<Open> = Vec::new();
    let mut edits: Vec<Edit> = Vec::new();
    for index in 0..children.len() {
        if env.skip {
            return;
        }
        let XmlNode::Element(element) = &mut children[index] else {
            continue;
        };
        match element.name.as_str() {
            "w:r" => {
                match field_char(element) {
                    Some("begin") => stack.push(Open {
                        begin: index,
                        pieces: Vec::new(),
                        separate: None,
                        result: String::new(),
                    }),
                    Some("separate") => {
                        if let Some(open) = stack.last_mut() {
                            open.separate = Some(index);
                        }
                    }
                    Some("end") => {
                        if let Some(open) = stack.pop() {
                            let value = env.evaluate(&open.pieces, &open.result);
                            match stack.last_mut() {
                                Some(parent) if parent.separate.is_none() => {
                                    // A merge field in another's instruction
                                    // shows its value too, as Word keeps it:
                                    // what the parent is read from on opening.
                                    if value.merge && env.mode != FieldMode::Merge {
                                        let text = match env.mode {
                                            FieldMode::Codes => value.codes.clone(),
                                            _ => Some(value.text.clone()),
                                        };
                                        if let (Some(text), Some(_)) = (text, open.separate) {
                                            edits.push(Edit::Result {
                                                separate: open.separate,
                                                end: index,
                                                text,
                                            });
                                        }
                                    }
                                    parent.pieces.push(Piece::Nested(value.text))
                                }
                                Some(_) => {}
                                None if value.merge => edits.push(match env.mode {
                                    FieldMode::Merge => Edit::Whole {
                                        begin: open.begin,
                                        end: index,
                                        text: value.text,
                                    },
                                    FieldMode::Preview => Edit::Result {
                                        separate: open.separate,
                                        end: index,
                                        text: value.text,
                                    },
                                    FieldMode::Codes => match value.codes {
                                        Some(codes) => Edit::Result {
                                            separate: open.separate,
                                            end: index,
                                            text: codes,
                                        },
                                        None => continue,
                                    },
                                }),
                                None => {}
                            }
                        }
                    }
                    _ => {
                        if let Some(open) = stack.last_mut() {
                            if open.separate.is_none() {
                                let text = element_text(element, "w:instrText");
                                if !text.is_empty() {
                                    open.pieces.push(Piece::Text(text));
                                }
                            } else {
                                open.result.push_str(&element_text(element, "w:t"));
                            }
                        }
                        // A text box's paragraphs are inside its run.
                        for node in &mut element.children {
                            if let XmlNode::Element(child) = node {
                                if !matches!(child.name.as_str(), "w:rPr" | "w:t" | "w:instrText") {
                                    walk(&mut child.children, env);
                                }
                            }
                        }
                    }
                }
            }
            "w:fldSimple" => {
                let instruction = element
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "w:instr")
                    .map(|attribute| attribute.value.clone())
                    .unwrap_or_default();
                let result: String = element
                    .children
                    .iter()
                    .filter_map(|node| match node {
                        XmlNode::Element(run) if run.name == "w:r" => {
                            Some(element_text(run, "w:t"))
                        }
                        _ => None,
                    })
                    .collect();
                let value = env.evaluate(&[Piece::Text(instruction)], &result);
                if let Some(open) = stack.last_mut() {
                    if open.separate.is_none() {
                        open.pieces.push(Piece::Nested(value.text));
                    }
                    continue;
                }
                if !value.merge {
                    continue;
                }
                let properties = element.children.iter().find_map(|node| match node {
                    XmlNode::Element(run) if run.name == "w:r" => run_properties(run),
                    _ => None,
                });
                match env.mode {
                    FieldMode::Merge => edits.push(Edit::Whole {
                        begin: index,
                        end: index,
                        text: value.text,
                    }),
                    FieldMode::Preview => {
                        element.children = text_runs(&value.text, properties.as_ref());
                    }
                    FieldMode::Codes => {
                        if let Some(codes) = value.codes {
                            element.children = text_runs(&codes, properties.as_ref());
                        }
                    }
                }
            }
            "w:sectPr" | "w:pPr" | "w:tblPr" | "w:trPr" | "w:tcPr" | "w:sdtPr" | "w:rPr" => {}
            _ => {
                if let Some(open) = stack.last_mut() {
                    if open.separate.is_some() {
                        open.result.push_str(&element_text(element, "w:t"));
                    }
                }
                walk(&mut element.children, env);
            }
        }
    }
    // Edits from the end, so the indices before them hold.
    for edit in edits.into_iter().rev() {
        match edit {
            Edit::Whole { begin, end, text } => {
                let properties = template_properties(children, begin, end);
                let runs = text_runs(&text, properties.as_ref());
                children.splice(begin..=end, runs);
            }
            Edit::Result {
                separate,
                end,
                text,
            } => {
                let begin = separate.map(|separate| separate + 1).unwrap_or(end);
                let properties = template_properties(children, begin.saturating_sub(1), end);
                let mut runs = Vec::new();
                if separate.is_none() {
                    let mut separator = bare("w:r");
                    let mut field_char = bare("w:fldChar");
                    field_char.attributes.push(XmlAttribute {
                        name: "w:fldCharType".to_string(),
                        value: "separate".to_string(),
                    });
                    separator.children.push(XmlNode::Element(field_char));
                    runs.push(XmlNode::Element(separator));
                }
                runs.extend(text_runs(&text, properties.as_ref()));
                children.splice(begin..end, runs);
            }
        }
    }
}

/// The properties a field's value is written in: its result's first run's,
/// or else its first run's.
fn template_properties(children: &[XmlNode], begin: usize, end: usize) -> Option<XmlElement> {
    let runs = || {
        children[begin..=end.min(children.len().saturating_sub(1))]
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(run) if run.name == "w:r" => Some(run),
                _ => None,
            })
    };
    runs()
        .filter(|run| field_char(run).is_none() && !element_text(run, "w:t").is_empty())
        .find_map(run_properties)
        .or_else(|| runs().find_map(run_properties))
}

fn body_children(xml: &DocxXmlDocument) -> Option<&Vec<XmlNode>> {
    xml.root().children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:body" => Some(&element.children),
        _ => None,
    })
}

/// One merge field's value for a recipient, as the fields engine's update
/// (F9) asks for it: `instruction` is the field's code with any nested
/// fields already evaluated, `record` an index into the request's records.
/// `None` for a field that is not the merge's.
pub fn evaluate_instruction(
    request: &MergeRequest,
    record: usize,
    instruction: &str,
) -> Option<String> {
    let mut env = Env {
        request,
        cursor: record,
        sequence: record + 1,
        bookmarks: HashMap::new(),
        problems: Vec::new(),
        skip: false,
        mode: FieldMode::Preview,
    };
    let value = env.evaluate(&[Piece::Text(instruction.to_string())], "");
    value.merge.then_some(value.text)
}

/// Rewrites the merge fields of the body for `record` (an index into the
/// request's records): its values (preview) or the fields' «names».
pub fn rewrite(xml: &mut DocxXmlDocument, request: &MergeRequest, record: usize, mode: FieldMode) {
    let body_id = xml.root().children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
        _ => None,
    });
    let Some(body_id) = body_id else {
        return;
    };
    let mut env = Env {
        request,
        cursor: record,
        sequence: 1,
        bookmarks: HashMap::new(),
        problems: Vec::new(),
        skip: false,
        mode,
    };
    if let Some(body) = xml.element_mut(body_id) {
        walk(&mut body.children, &mut env);
    }
    xml.rebuild_index();
}

/// The body of the merged document: a copy of the main document's body
/// per recipient — per page of labels — the merge fields as text, a
/// section break between copies but a directory's.
pub fn merge_body(
    xml: &DocxXmlDocument,
    request: &MergeRequest,
    kind: MainDocumentType,
) -> Result<Vec<XmlNode>, String> {
    let children = body_children(xml).ok_or("the document has no body")?;
    let section = children.iter().rev().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:sectPr" => Some(element.clone()),
        _ => None,
    });
    let content: Vec<XmlNode> = children
        .iter()
        .filter(|node| !matches!(node, XmlNode::Element(element) if element.name == "w:sectPr"))
        .cloned()
        .collect();
    if request.records.is_empty() {
        return Err("no recipients are chosen for the merge".to_string());
    }
    let mut copies: Vec<Vec<XmlNode>> = Vec::new();
    let mut cursor = 0;
    let mut bookmarks = HashMap::new();
    while cursor < request.records.len() {
        let mut copy = content.clone();
        let mut env = Env {
            request,
            cursor,
            sequence: copies.len() + 1,
            bookmarks: std::mem::take(&mut bookmarks),
            problems: Vec::new(),
            skip: false,
            mode: FieldMode::Merge,
        };
        walk(&mut copy, &mut env);
        bookmarks = env.bookmarks;
        cursor = env.cursor.max(cursor) + 1;
        if !env.skip {
            copies.push(copy);
        }
    }
    let count = copies.len();
    let mut body = Vec::new();
    for (index, mut copy) in copies.into_iter().enumerate() {
        let last = index + 1 == count;
        if !last && kind != MainDocumentType::Directory {
            if let Some(section) = &section {
                let mut section = section.clone();
                section.children.retain(
                    |node| !matches!(node, XmlNode::Element(element) if element.name == "w:type"),
                );
                let mut kind_element = bare("w:type");
                kind_element.attributes.push(XmlAttribute {
                    name: "w:val".to_string(),
                    value: "nextPage".to_string(),
                });
                section.children.insert(0, XmlNode::Element(kind_element));
                put_section_on_last_paragraph(&mut copy, section);
            }
        }
        body.extend(copy);
    }
    if let Some(section) = section {
        body.push(XmlNode::Element(section));
    }
    Ok(body)
}

/// Ends a copy with `section`: on its last paragraph's properties, or on a
/// paragraph of its own after a table.
fn put_section_on_last_paragraph(copy: &mut Vec<XmlNode>, section: XmlElement) {
    let last_paragraph = copy.last_mut().and_then(|node| match node {
        XmlNode::Element(element) if element.name == "w:p" => Some(element),
        _ => None,
    });
    let paragraph = match last_paragraph {
        Some(paragraph) => paragraph,
        None => {
            copy.push(XmlNode::Element(bare("w:p")));
            match copy.last_mut() {
                Some(XmlNode::Element(paragraph)) => paragraph,
                _ => return,
            }
        }
    };
    let has_properties = paragraph
        .children
        .iter()
        .any(|node| matches!(node, XmlNode::Element(element) if element.name == "w:pPr"));
    if !has_properties {
        paragraph
            .children
            .insert(0, XmlNode::Element(bare("w:pPr")));
    }
    if let Some(XmlNode::Element(properties)) = paragraph
        .children
        .iter_mut()
        .find(|node| matches!(node, XmlNode::Element(element) if element.name == "w:pPr"))
    {
        properties.children.retain(
            |node| !matches!(node, XmlNode::Element(element) if element.name == "w:sectPr"),
        );
        properties.children.push(XmlNode::Element(section));
    }
}

/// What is wrong with merging `request` into the document.
pub fn problems(xml: &DocxXmlDocument, request: &MergeRequest) -> Vec<MergeProblem> {
    let mut problems = Vec::new();
    if request.data.columns.is_empty() {
        problems.push(MergeProblem::EmptyDataSource);
    } else if request.records.is_empty() {
        problems.push(MergeProblem::NoRecipients);
    }
    let Some(children) = body_children(xml) else {
        return problems;
    };
    let mut copy = children.clone();
    let mut env = Env {
        request,
        cursor: 0,
        sequence: 1,
        bookmarks: HashMap::new(),
        problems: Vec::new(),
        skip: false,
        mode: FieldMode::Merge,
    };
    walk(&mut copy, &mut env);
    for problem in env.problems {
        if !problems.contains(&problem) {
            problems.push(problem);
        }
    }
    problems
}

/// The names of the merge fields in the body, in order, once each.
pub fn field_names(xml: &DocxXmlDocument) -> Vec<String> {
    fn walk_names(element: &XmlElement, names: &mut Vec<String>) {
        for node in &element.children {
            let XmlNode::Element(child) = node else {
                continue;
            };
            let instruction = if child.name == "w:instrText" {
                Some(
                    child
                        .children
                        .iter()
                        .filter_map(|node| match node {
                            XmlNode::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<String>(),
                )
            } else if child.name == "w:fldSimple" {
                child
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "w:instr")
                    .map(|attribute| attribute.value.clone())
            } else {
                None
            };
            if let Some(instruction) = instruction {
                let tokens = tokenize(&[Piece::Text(instruction)]);
                if tokens
                    .first()
                    .is_some_and(|token| token.text.eq_ignore_ascii_case("MERGEFIELD"))
                {
                    if let Some(name) = tokens.get(1) {
                        if !names.contains(&name.text) {
                            names.push(name.text.clone());
                        }
                    }
                }
            }
            walk_names(child, names);
        }
    }
    let mut names = Vec::new();
    walk_names(xml.root(), &mut names);
    names
}

#[cfg(test)]
mod tokenizer_tests {
    use super::*;

    fn texts(pieces: &[Piece]) -> Vec<String> {
        tokenize(pieces)
            .into_iter()
            .map(|token| token.text)
            .collect()
    }

    #[test]
    fn splits_quotes_switches_and_nested_values() {
        assert_eq!(
            texts(&[Piece::Text(r#" MERGEFIELD 姓 \* MERGEFORMAT "#.to_string())]),
            vec!["MERGEFIELD", "姓", "\\*", "MERGEFORMAT"]
        );
        let tokens = tokenize(&[
            Piece::Text(" IF ".to_string()),
            Piece::Nested("New York".to_string()),
            Piece::Text(r#" = "New*" "東" "西" "#.to_string()),
        ]);
        assert_eq!(tokens[1].text, "New York");
        assert!(tokens[1].literal);
        assert!(compare(&tokens[1], &tokens[2].text, &tokens[3]));
        assert_eq!(tokens[4].text, "東");
    }

    #[test]
    fn numbers_and_comparisons() {
        assert_eq!(format_number(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(format_number(-3.0, "0"), "-3");
        let token = |text: &str, literal| Token {
            text: text.to_string(),
            literal,
        };
        assert!(compare(&token("10", false), ">", &token("9", true)));
        assert!(!compare(&token("abc", false), "=", &token("ABC", true)));
        assert!(compare(&token("", false), "=", &token("", true)));
    }
}
