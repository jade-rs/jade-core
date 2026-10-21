//! Evaluating a story's fields as Word does when it updates them: each kind
//! from the data it depends on — the clock, the package properties, the
//! document's statistics and layout, its bookmarks, tables and paragraphs,
//! the user's information, prompts and local files — with the general
//! switches applied to the result.

use super::formula::{evaluate_formula, FormulaContext};
use super::properties::{DocumentProperties, PropertyValue};
use super::scan::{Role, ScannedField, Story};
use super::write::NewResult;
use super::FieldEnvironment;
use crate::word_document::CitationSourceInfo;
use jade_docx_reader::field_datetime::{
    datetime_from_unix, format_word_date, parse_core_property_datetime, unix_seconds, FieldDateTime,
};
use jade_docx_reader::field_format::{
    apply_general_format, format_plain_number, FieldCode, FieldValue, GeneralFormat,
};
use jade_docx_reader::numbering::format_number_value;
use std::collections::{HashMap, HashSet};

/// Which story is being evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoryKind {
    Body,
    HeaderFooter,
    Notes,
}

/// What the whole document tells field evaluation.
#[derive(Clone, Debug, Default)]
pub(crate) struct DocumentData {
    pub properties: DocumentProperties,
    pub variables: Vec<(String, String)>,
    /// Paragraph style ids by normalized name and by id.
    pub style_ids: HashMap<String, String>,
    /// Outline level (0-based) each paragraph style gives.
    pub style_outline: HashMap<String, u8>,
    /// AutoText entries (`word/glossary/document.xml`) by lowercased name.
    pub glossary: HashMap<String, String>,
    pub sources: Vec<CitationSourceInfo>,
    pub words: usize,
    pub characters: usize,
    pub characters_with_spaces: usize,
    /// Footnote and endnote references of the body, in order.
    pub note_references: Vec<(bool, String)>,
    pub footnote_format: Option<String>,
    pub endnote_format: Option<String>,
    /// The body's paragraphs, which a header's STYLEREF reads.
    pub body_paragraphs: Vec<super::scan::ScannedParagraph>,
}

impl DocumentData {
    pub fn style_id_for(&self, name: &str) -> Option<String> {
        let key = normalize_style_name(name);
        self.style_ids.get(&key).cloned()
    }
}

/// Lowercased, spaceless, with Word's Japanese names of the built-in styles
/// and the bare heading numbers STYLEREF accepts mapped to the stored ones.
pub(crate) fn normalize_style_name(name: &str) -> String {
    let trimmed = name.trim();
    if let Ok(level) = trimmed.parse::<u8>() {
        if (1..=9).contains(&level) {
            return format!("heading{level}");
        }
    }
    let lowered = trimmed.to_lowercase().replace([' ', '\u{3000}'], "");
    if let Some(level) = lowered.strip_prefix("見出し") {
        return format!("heading{level}");
    }
    match lowered.as_str() {
        "標準" => "normal".to_string(),
        "表題" => "title".to_string(),
        "副題" => "subtitle".to_string(),
        "図表番号" => "caption".to_string(),
        _ => lowered,
    }
}

/// A new result and how to format it.
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub result: NewResult,
    pub format: GeneralFormat,
    /// SYMBOL's `\f` font and `\s` size.
    pub symbol_font: Option<(Option<String>, Option<f32>)>,
}

/// The raw result of one field, before the general switches.
enum Evaluated {
    Value(FieldValue),
    /// Final text the general switches do not touch (an error message).
    Final(String),
    /// No visible result.
    Hidden,
    Bookmarked {
        name: String,
        text: String,
    },
    Paragraphs(Vec<String>),
    Picture {
        bytes: Vec<u8>,
        name: String,
    },
    /// The current result stays.
    Keep,
}

#[derive(Default)]
struct SeqState {
    value: u32,
    last_paragraph: Option<usize>,
}

pub(crate) struct Evaluation<'a> {
    story: &'a Story,
    data: &'a DocumentData,
    env: &'a FieldEnvironment,
    kind: StoryKind,
    /// The physical page (1-based) of a story paragraph, when known.
    page_of: &'a dyn Fn(usize) -> Option<usize>,
    /// The fields to write; `None` writes every one.
    selected: Option<&'a HashSet<usize>>,
    /// The key prompts are answered by: the story's part name.
    part: &'a str,
    values: Vec<Option<String>>,
    pub outcomes: Vec<Option<Outcome>>,
    pub property_changes: Vec<(String, String)>,
    seq: HashMap<String, SeqState>,
    /// Values SEQ fields had, by identifier, with the paragraph they sat in.
    seq_history: HashMap<String, Vec<(usize, u32)>>,
    listnum: HashMap<String, [u32; 9]>,
    listnum_paragraph: Option<(usize, usize)>,
    autonum: [u32; 9],
    assigned: HashMap<String, String>,
}

fn error_text(japanese: bool, english: &str, japanese_text: &str) -> String {
    if japanese {
        japanese_text.to_string()
    } else {
        english.to_string()
    }
}

/// `C:\\dir\\file.docx` with Word's doubled backslashes made single.
fn unescape_path(path: &str) -> String {
    path.replace("\\\\", "\\")
}

impl<'a> Evaluation<'a> {
    pub fn new(
        story: &'a Story,
        data: &'a DocumentData,
        env: &'a FieldEnvironment,
        kind: StoryKind,
        part: &'a str,
        page_of: &'a dyn Fn(usize) -> Option<usize>,
        selected: Option<&'a HashSet<usize>>,
    ) -> Self {
        Self {
            story,
            data,
            env,
            kind,
            page_of,
            selected,
            part,
            values: vec![None; story.fields.len()],
            outcomes: vec![None; story.fields.len()],
            property_changes: Vec::new(),
            seq: HashMap::new(),
            seq_history: HashMap::new(),
            listnum: HashMap::new(),
            listnum_paragraph: None,
            autonum: [0; 9],
            assigned: HashMap::new(),
        }
    }

    /// The key a prompt field's answer is stored under.
    pub fn prompt_key(part: &str, field: usize) -> String {
        format!("{part}#{field}")
    }

    fn is_selected(&self, index: usize) -> bool {
        self.selected
            .is_none_or(|selected| selected.contains(&index))
    }

    /// Evaluates every field in document order.
    pub fn run(&mut self) {
        let regenerating: Vec<usize> = Vec::new();
        for index in 0..self.story.fields.len() {
            if matches!(self.story.fields[index].parent, Some((_, Role::Operand))) {
                continue;
            }
            if regenerating
                .iter()
                .any(|&ancestor| self.story.in_result_of(index, ancestor))
            {
                continue;
            }
            self.evaluate(index);
        }
        // A SEQ that names a bookmark reads the sequence where it stands.
        for index in 0..self.story.fields.len() {
            self.resolve_seq_bookmark(index);
        }
    }

    /// The value field `index` contributes to an enclosing instruction.
    fn evaluate(&mut self, index: usize) -> String {
        if let Some(value) = &self.values[index] {
            return value.clone();
        }
        let field = &self.story.fields[index];
        let instruction = {
            let pieces: Vec<usize> = field
                .pieces
                .iter()
                .filter_map(|piece| match piece {
                    super::scan::Piece::Field(child) => Some(*child),
                    _ => None,
                })
                .collect();
            let mut nested = HashMap::new();
            for child in pieces {
                let value = self.evaluate(child);
                nested.insert(child, value);
            }
            field.instruction_with(|child| nested.remove(&child).unwrap_or_default())
        };
        let current = field.result_text.clone();
        let Some(code) = FieldCode::parse(&instruction) else {
            self.values[index] = Some(current.clone());
            return current;
        };
        let locked = field.locked;
        let evaluated = if locked {
            Evaluated::Keep
        } else {
            self.evaluate_code(index, &code)
        };
        let (text, outcome) = match evaluated {
            Evaluated::Keep => (current, None),
            Evaluated::Hidden => (String::new(), Some(NewResult::Text(String::new()))),
            Evaluated::Final(text) => (text.clone(), Some(NewResult::Text(text))),
            Evaluated::Value(value) => {
                let text = apply_general_format(&value, &code.format);
                (text.clone(), Some(NewResult::Text(text)))
            }
            Evaluated::Bookmarked { name, text } => {
                (String::new(), Some(NewResult::Bookmarked { name, text }))
            }
            Evaluated::Paragraphs(paragraphs) => (
                paragraphs.join("\n"),
                Some(NewResult::Paragraphs(paragraphs)),
            ),
            Evaluated::Picture { bytes, name } => {
                (String::new(), Some(NewResult::Picture { bytes, name }))
            }
        };
        if let Some(result) = outcome {
            if self.is_selected(index) {
                let symbol_font = (code.name == "SYMBOL").then(|| {
                    (
                        code.switch_value("f").map(str::to_string),
                        code.switch_value("s").and_then(|size| size.parse().ok()),
                    )
                });
                self.outcomes[index] = Some(Outcome {
                    result,
                    format: code.format.clone(),
                    symbol_font,
                });
            }
        }
        self.values[index] = Some(text.clone());
        text
    }

    fn field(&self, index: usize) -> &ScannedField {
        &self.story.fields[index]
    }

    fn japanese(&self) -> bool {
        self.env.japanese
    }

    /// The text a bookmark holds now: what ASK or SET last assigned, else
    /// its text in the story.
    fn bookmark_text(&self, name: &str) -> Option<String> {
        let key = name.to_lowercase();
        self.assigned.get(&key).cloned().or_else(|| {
            self.story
                .bookmarks
                .get(&key)
                .map(|bookmark| bookmark.text.clone())
        })
    }

    fn local_date(&self, w3cdtf: &str) -> Option<FieldDateTime> {
        let utc = parse_core_property_datetime(w3cdtf)?;
        Some(datetime_from_unix(
            unix_seconds(&utc),
            self.env.utc_offset_seconds,
        ))
    }

    fn page_number(&self, paragraph: usize) -> Option<(u32, Option<String>)> {
        let physical = (self.page_of)(paragraph)?;
        let section = self.story.paragraphs.get(paragraph)?.section;
        let numbering = self.story.sections.get(section);
        let start = numbering.and_then(|numbering| numbering.start);
        let format = numbering.and_then(|numbering| numbering.format.clone());
        let displayed = match start {
            Some(start) => {
                // Restart at `start` on the section's first page.
                let first = self
                    .story
                    .paragraphs
                    .iter()
                    .position(|candidate| candidate.section == section)
                    .and_then(|first| (self.page_of)(first))
                    .unwrap_or(physical);
                start + physical.saturating_sub(first) as u32
            }
            None => physical as u32,
        };
        Some((displayed, format))
    }

    fn section_page_count(&self, section: usize) -> Option<usize> {
        let pages: Vec<usize> = self
            .story
            .paragraphs
            .iter()
            .enumerate()
            .filter(|(_, paragraph)| paragraph.section == section)
            .filter_map(|(index, _)| (self.page_of)(index))
            .collect();
        let first = *pages.iter().min()?;
        let last = *pages.iter().max()?;
        Some(last - first + 1)
    }

    fn property(&self, name: &str) -> Option<Evaluated> {
        let value = self.data.properties.lookup(name)?;
        Some(match value {
            PropertyValue::Text(text) => Evaluated::Value(FieldValue::Text(text)),
            PropertyValue::Number(number) => Evaluated::Value(FieldValue::Number(number)),
            PropertyValue::Date(date) => match self.local_date(&date) {
                Some(date) => Evaluated::Value(FieldValue::Date(date)),
                None => Evaluated::Value(FieldValue::Text(date)),
            },
        })
    }

    fn statistic(&self, name: &str) -> Option<f64> {
        Some(match name {
            "NUMWORDS" => self.data.words as f64,
            "NUMCHARS" => self.data.characters as f64,
            "NUMPAGES" => self.env.page_count? as f64,
            _ => return None,
        })
    }

    fn date_value(
        &self,
        code: &FieldCode,
        date: FieldDateTime,
        default_picture: &str,
    ) -> Evaluated {
        if code.format.date_picture.is_some() {
            return Evaluated::Value(FieldValue::Date(date));
        }
        let text = format_word_date(default_picture, &date);
        Evaluated::Value(FieldValue::Text(text))
    }

    fn default_date_picture(&self) -> &'static str {
        if self.japanese() {
            "yyyy/MM/dd"
        } else {
            "M/d/yyyy"
        }
    }

    fn default_time_picture(&self) -> &'static str {
        if self.japanese() {
            "H:mm"
        } else {
            "h:mm AM/PM"
        }
    }

    fn default_datetime_picture(&self) -> &'static str {
        if self.japanese() {
            "yyyy/MM/dd H:mm:ss"
        } else {
            "M/d/yyyy h:mm:ss AM/PM"
        }
    }

    fn evaluate_code(&mut self, index: usize, code: &FieldCode) -> Evaluated {
        let japanese = self.japanese();
        let first = code.operand(0).map(str::to_string);
        match code.name.as_str() {
            "PAGE" => {
                let paragraph = self.field(index).paragraph;
                match (self.kind, self.page_number(paragraph)) {
                    (StoryKind::HeaderFooter, _) | (_, None) => {
                        if self.kind == StoryKind::Body {
                            return Evaluated::Keep;
                        }
                        // A header or footer shows each page's own number;
                        // the stored result is the first page's.
                        let start = self
                            .story
                            .sections
                            .first()
                            .and_then(|s| s.start)
                            .unwrap_or(1);
                        Evaluated::Value(FieldValue::Number(start as f64))
                    }
                    (_, Some((page, format))) => {
                        self.number_with_section_format(code, page, format)
                    }
                }
            }
            "NUMPAGES" => match self.env.page_count {
                Some(count) => Evaluated::Value(FieldValue::Number(count as f64)),
                None => Evaluated::Keep,
            },
            "SECTIONPAGES" => {
                let section = self
                    .story
                    .paragraphs
                    .get(self.field(index).paragraph)
                    .map(|paragraph| paragraph.section)
                    .unwrap_or(0);
                match (self.kind, self.section_page_count(section)) {
                    (StoryKind::Body, Some(count)) => {
                        Evaluated::Value(FieldValue::Number(count as f64))
                    }
                    _ => match self.env.page_count {
                        Some(count) if self.story.sections.len() <= 1 => {
                            Evaluated::Value(FieldValue::Number(count as f64))
                        }
                        _ => Evaluated::Keep,
                    },
                }
            }
            "SECTION" => {
                let section = self
                    .story
                    .paragraphs
                    .get(self.field(index).paragraph)
                    .map(|paragraph| paragraph.section)
                    .unwrap_or(0);
                Evaluated::Value(FieldValue::Number((section + 1) as f64))
            }
            "DATE" => self.date_value(code, self.env.now, self.default_date_picture()),
            "TIME" => self.date_value(code, self.env.now, self.default_time_picture()),
            "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                let stored = match code.name.as_str() {
                    "CREATEDATE" => &self.data.properties.created,
                    "SAVEDATE" => &self.data.properties.modified,
                    _ => &self.data.properties.last_printed,
                };
                match self.local_date(stored) {
                    Some(date) => self.date_value(code, date, self.default_datetime_picture()),
                    // Never printed (or saved): Word's zero date.
                    None => Evaluated::Final(if japanese {
                        "0000/00/00 0:00:00".to_string()
                    } else {
                        "0/0/0000 0:00:00 AM".to_string()
                    }),
                }
            }
            "EDITTIME" => Evaluated::Value(FieldValue::Number(
                self.data
                    .properties
                    .total_time
                    .trim()
                    .parse()
                    .unwrap_or(0.0),
            )),
            "AUTHOR" | "TITLE" | "SUBJECT" | "KEYWORDS" | "COMMENTS" | "LASTSAVEDBY" => {
                if let Some(value) = first.filter(|_| code.name != "LASTSAVEDBY") {
                    if self.is_selected(index) {
                        self.property_changes
                            .push((code.name.clone(), value.clone()));
                    }
                    return Evaluated::Value(FieldValue::Text(value));
                }
                let name = match code.name.as_str() {
                    "LASTSAVEDBY" => "LastSavedBy",
                    other => other,
                };
                self.property(name)
                    .unwrap_or(Evaluated::Value(FieldValue::Text(String::new())))
            }
            "REVNUM" => self
                .property("Revision")
                .unwrap_or(Evaluated::Value(FieldValue::Text(String::new()))),
            "TEMPLATE" => {
                let template = self.data.properties.template.clone();
                Evaluated::Value(FieldValue::Text(if template.is_empty() {
                    "Normal.dotm".to_string()
                } else {
                    template
                }))
            }
            "DOCPROPERTY" => match first {
                Some(name) => match self.statistic_property(&name) {
                    Some(value) => value,
                    None => self.property(&name).unwrap_or_else(|| {
                        Evaluated::Final(error_text(
                            japanese,
                            "Error! Unknown document property name.",
                            "エラー! 文書プロパティ名が正しくありません。",
                        ))
                    }),
                },
                None => Evaluated::Keep,
            },
            "INFO" => {
                let Some(name) = first else {
                    return Evaluated::Keep;
                };
                if let Some(value) = code.operand(1) {
                    if self.is_selected(index) {
                        self.property_changes
                            .push((name.to_ascii_uppercase(), value.to_string()));
                    }
                    return Evaluated::Value(FieldValue::Text(value.to_string()));
                }
                let upper = name.to_ascii_uppercase();
                if let Some(number) = self.statistic(&upper) {
                    return Evaluated::Value(FieldValue::Number(number));
                }
                match upper.as_str() {
                    "FILENAME" | "FILESIZE" | "TEMPLATE" | "EDITTIME" | "AUTHOR" | "TITLE"
                    | "SUBJECT" | "KEYWORDS" | "COMMENTS" | "LASTSAVEDBY" | "REVNUM"
                    | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                        let nested = FieldCode {
                            name: upper,
                            operands: Vec::new(),
                            switches: code.switches.clone(),
                            format: code.format.clone(),
                            expression: None,
                        };
                        self.evaluate_code(index, &nested)
                    }
                    _ => self.property(&name).unwrap_or(Evaluated::Keep),
                }
            }
            "DOCVARIABLE" => match first {
                Some(name) => match self
                    .data
                    .variables
                    .iter()
                    .find(|(variable, _)| variable.eq_ignore_ascii_case(&name))
                {
                    Some((_, value)) => Evaluated::Value(FieldValue::Text(value.clone())),
                    None => Evaluated::Final(error_text(
                        japanese,
                        "Error! No document variable supplied.",
                        "エラー! 文書変数が指定されていません。",
                    )),
                },
                None => Evaluated::Keep,
            },
            "FILENAME" => {
                let name = if code.has_switch("p") {
                    self.env
                        .file_path
                        .clone()
                        .or_else(|| self.env.file_name.clone())
                } else {
                    self.env.file_name.clone()
                };
                match name {
                    Some(name) => Evaluated::Value(FieldValue::Text(name)),
                    None => Evaluated::Value(FieldValue::Text(
                        if japanese { "文書1" } else { "Document1" }.to_string(),
                    )),
                }
            }
            "FILESIZE" => match self.env.file_size {
                Some(bytes) => {
                    let value = if code.has_switch("m") {
                        (bytes as f64 / (1024.0 * 1024.0)).round()
                    } else if code.has_switch("k") {
                        (bytes as f64 / 1024.0).round()
                    } else {
                        bytes as f64
                    };
                    Evaluated::Value(FieldValue::Number(value))
                }
                None => Evaluated::Keep,
            },
            "NUMWORDS" | "NUMCHARS" => match self.statistic(&code.name) {
                Some(number) => Evaluated::Value(FieldValue::Number(number)),
                None => Evaluated::Keep,
            },
            "USERNAME" | "USERINITIALS" | "USERADDRESS" => {
                let value = first.unwrap_or_else(|| match code.name.as_str() {
                    "USERNAME" => self.env.user_name.clone(),
                    "USERINITIALS" => self.env.user_initials.clone(),
                    _ => self.env.user_address.clone(),
                });
                Evaluated::Value(FieldValue::Text(value))
            }
            "QUOTE" => Evaluated::Value(FieldValue::Text(
                code.operands
                    .iter()
                    .map(|operand| operand.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            )),
            "SYMBOL" => {
                let Some(value) = first else {
                    return Evaluated::Keep;
                };
                let number = if let Some(hex) = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    value.parse::<u32>().ok()
                };
                match number.and_then(|code_point| {
                    // A symbol font's private-use range maps onto its
                    // ASCII-range glyphs.
                    char::from_u32(code_point)
                }) {
                    Some(character) => Evaluated::Final(character.to_string()),
                    None => Evaluated::Keep,
                }
            }
            "SET" => {
                let (Some(name), Some(value)) = (first, code.operand(1)) else {
                    return Evaluated::Keep;
                };
                self.assigned.insert(name.to_lowercase(), value.to_string());
                Evaluated::Bookmarked {
                    name,
                    text: value.to_string(),
                }
            }
            "ASK" => {
                let Some(name) = first else {
                    return Evaluated::Keep;
                };
                let answer = self
                    .env
                    .prompt_answers
                    .get(&Self::prompt_key(self.part, index))
                    .cloned()
                    .filter(|_| self.is_selected(index))
                    .or_else(|| code.switch_value("d").map(str::to_string))
                    .or_else(|| self.bookmark_text(&name));
                let Some(answer) = answer else {
                    return Evaluated::Keep;
                };
                self.assigned.insert(name.to_lowercase(), answer.clone());
                Evaluated::Bookmarked { name, text: answer }
            }
            "FILLIN" => {
                let answer = self
                    .env
                    .prompt_answers
                    .get(&Self::prompt_key(self.part, index))
                    .cloned()
                    .filter(|_| self.is_selected(index));
                match answer {
                    Some(answer) => Evaluated::Value(FieldValue::Text(answer)),
                    None if self.field(index).result_text.is_empty() => code
                        .switch_value("d")
                        .map(|default| Evaluated::Value(FieldValue::Text(default.to_string())))
                        .unwrap_or(Evaluated::Keep),
                    None => Evaluated::Keep,
                }
            }
            "REF" => match first {
                Some(name) => self.reference(index, code, &name),
                None => Evaluated::Keep,
            },
            "PAGEREF" => {
                let Some(name) = first else {
                    return Evaluated::Keep;
                };
                let Some(bookmark) = self.story.bookmarks.get(&name.to_lowercase()) else {
                    return Evaluated::Final(error_text(
                        japanese,
                        "Error! Bookmark not defined.",
                        "エラー! ブックマークが定義されていません。",
                    ));
                };
                if code.has_switch("p") {
                    let here = self.field(index).paragraph;
                    if let (Some(target), Some(own)) =
                        ((self.page_of)(bookmark.paragraph), (self.page_of)(here))
                    {
                        if target == own {
                            return Evaluated::Final(self.relative_word(bookmark.paragraph < here));
                        }
                        return Evaluated::Final(if japanese {
                            format!("{target} ページ")
                        } else {
                            format!("on page {target}")
                        });
                    }
                }
                match self.page_number(bookmark.paragraph) {
                    Some((page, format)) => self.number_with_section_format(code, page, format),
                    None => Evaluated::Keep,
                }
            }
            "NOTEREF" => {
                let Some(name) = first else {
                    return Evaluated::Keep;
                };
                let Some(bookmark) = self.story.bookmarks.get(&name.to_lowercase()) else {
                    return Evaluated::Final(error_text(
                        japanese,
                        "Error! Bookmark not defined.",
                        "エラー! ブックマークが定義されていません。",
                    ));
                };
                if code.has_switch("p") {
                    let here = self.field(index).paragraph;
                    return Evaluated::Final(self.relative_word(bookmark.paragraph < here));
                }
                let Some((footnote, id)) = bookmark.note.clone() else {
                    return Evaluated::Keep;
                };
                let ordinal = self
                    .data
                    .note_references
                    .iter()
                    .filter(|(kind, _)| *kind == footnote)
                    .position(|(_, candidate)| *candidate == id)
                    .map(|position| position as u32 + 1);
                let Some(ordinal) = ordinal else {
                    return Evaluated::Keep;
                };
                let format = if footnote {
                    self.data.footnote_format.as_deref().unwrap_or("decimal")
                } else {
                    self.data.endnote_format.as_deref().unwrap_or("lowerRoman")
                };
                Evaluated::Value(FieldValue::Text(format_number_value(
                    format,
                    ordinal,
                    Some("en-US"),
                )))
            }
            "STYLEREF" => self.style_reference(index, code),
            "SEQ" => self.sequence(index, code),
            "LISTNUM" => self.list_number(index, code),
            "AUTONUM" | "AUTONUMLGL" | "AUTONUMOUT" => self.auto_number(index, code),
            "IF" => self.condition(code),
            "COMPARE" => match self.compare(&code.operands) {
                Some(result) => {
                    Evaluated::Value(FieldValue::Number(if result { 1.0 } else { 0.0 }))
                }
                None => Evaluated::Keep,
            },
            "=" => self.formula(index, code),
            "INCLUDETEXT" => self.include_text(code),
            "INCLUDEPICTURE" => {
                let (Some(path), Some(read)) = (first, self.env.read_file.as_ref()) else {
                    return Evaluated::Keep;
                };
                let path = unescape_path(&path);
                match read(&path).filter(|bytes| super::picture::image_info(bytes).is_some()) {
                    Some(bytes) => Evaluated::Picture {
                        bytes,
                        name: path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string(),
                    },
                    // A file Jade cannot read keeps the picture Word saved.
                    None => Evaluated::Keep,
                }
            }
            "HYPERLINK" => Evaluated::Keep,
            "MERGEFIELD" => match first {
                Some(name) => {
                    let before = code.switch_value("b").unwrap_or_default();
                    let after = code.switch_value("f").unwrap_or_default();
                    Evaluated::Final(format!("{before}«{name}»{after}"))
                }
                None => Evaluated::Keep,
            },
            "MACROBUTTON" | "GOTOBUTTON" => {
                let text = code
                    .operands
                    .iter()
                    .skip(1)
                    .map(|operand| operand.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                Evaluated::Final(text)
            }
            "AUTOTEXT" | "GLOSSARY" => match first {
                Some(name) => match self.data.glossary.get(&name.to_lowercase()) {
                    Some(text) => Evaluated::Value(FieldValue::Text(text.clone())),
                    None => Evaluated::Final(error_text(
                        japanese,
                        "Error! AutoText entry not defined.",
                        "エラー! 定型句が定義されていません。",
                    )),
                },
                None => Evaluated::Keep,
            },
            "AUTOTEXTLIST" => match first {
                Some(text) => Evaluated::Value(FieldValue::Text(text)),
                None => Evaluated::Keep,
            },
            "CITATION" => match first.and_then(|tag| {
                self.data
                    .sources
                    .iter()
                    .find(|source| source.tag.eq_ignore_ascii_case(&tag))
                    .cloned()
            }) {
                Some(source) => {
                    let mut text = source.citation_text();
                    if let Some(page) = code.switch_value("p") {
                        text.insert_str(text.len() - 1, &format!(", {page}"));
                    }
                    if code.has_switch("n") {
                        // Suppress the author.
                        text = format!("({})", source.year);
                    }
                    if code.has_switch("y") {
                        text = format!("({})", source.author);
                    }
                    if let Some(prefix) = code.switch_value("f") {
                        text.insert_str(1, prefix);
                    }
                    if let Some(suffix) = code.switch_value("s") {
                        text.insert_str(text.len() - 1, suffix);
                    }
                    Evaluated::Final(text)
                }
                None => Evaluated::Keep,
            },
            "BIBLIOGRAPHY" => {
                if self.data.sources.is_empty() {
                    return Evaluated::Final(error_text(
                        japanese,
                        "There are no sources in the current document.",
                        "現在の文書には資料文献がありません。",
                    ));
                }
                let mut sources = self.data.sources.clone();
                sources.sort_by(|a, b| a.author.to_lowercase().cmp(&b.author.to_lowercase()));
                Evaluated::Paragraphs(sources.iter().map(|s| s.bibliography_line()).collect())
            }
            "INDEX" => self.index(code),
            "TOA" => self.table_of_authorities(code),
            // No result: these mark text for other fields or hold data.
            "TC" | "TA" | "XE" | "PRIVATE" | "ADDIN" | "RD" | "ADVANCE" => Evaluated::Keep,
            // Fields whose result Jade keeps: tables of contents are rebuilt
            // by their own command, mail-merge and linked objects need data
            // Jade has no source for, form fields hold what was typed and EQ
            // draws its equation from its code.
            "TOC" | "EQ" | "LINK" | "EMBED" | "DDE" | "DDEAUTO" | "DATABASE" | "NEXT"
            | "NEXTIF" | "SKIPIF" | "MERGEREC" | "MERGESEQ" | "FORMTEXT" | "FORMCHECKBOX"
            | "FORMDROPDOWN" | "PRINT" | "BARCODE" | "DISPLAYBARCODE" | "MERGEBARCODE"
            | "GREETINGLINE" | "ADDRESSBLOCK" | "BIDIOUTLINE" | "SHAPE" | "CONTROL" | "OCX"
            | "IMPORT" | "SUBJECTS" => Evaluated::Keep,
            // A field named by a bookmark alone is a REF.
            name => {
                let bookmark = self
                    .story
                    .bookmarks
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case(name))
                    || self.assigned.contains_key(&name.to_lowercase());
                if bookmark {
                    let original = code
                        .operands
                        .first()
                        .map(|_| name.to_string())
                        .unwrap_or_else(|| name.to_string());
                    self.reference(index, code, &original)
                } else {
                    Evaluated::Final(error_text(
                        japanese,
                        "Error! Bookmark not defined.",
                        "エラー! ブックマークが定義されていません。",
                    ))
                }
            }
        }
    }

    fn relative_word(&self, above: bool) -> String {
        match (self.japanese(), above) {
            (false, true) => "above".to_string(),
            (false, false) => "below".to_string(),
            (true, true) => "上".to_string(),
            (true, false) => "下".to_string(),
        }
    }

    /// A number in the section's page number format, unless a `\*` or `\#`
    /// switch asks for another.
    fn number_with_section_format(
        &self,
        code: &FieldCode,
        number: u32,
        format: Option<String>,
    ) -> Evaluated {
        let has_number_switch = code.format.numeric_picture.is_some()
            || code.format.formats.iter().any(|name| {
                jade_docx_reader::field_format::format_number_switch(1.0, name).is_some()
            });
        match format.filter(|format| !has_number_switch && format != "decimal") {
            Some(format) => Evaluated::Value(FieldValue::Text(format_number_value(
                &format,
                number,
                Some("en-US"),
            ))),
            None => Evaluated::Value(FieldValue::Number(number as f64)),
        }
    }

    fn statistic_property(&self, name: &str) -> Option<Evaluated> {
        let key = name.to_ascii_lowercase().replace(' ', "");
        let number = match key.as_str() {
            "pages" => self.env.page_count? as f64,
            "words" => self.data.words as f64,
            "characters" => self.data.characters as f64,
            "characterswithspaces" => self.data.characters_with_spaces as f64,
            "bytes" => self.env.file_size? as f64,
            _ => return None,
        };
        Some(Evaluated::Value(FieldValue::Number(number)))
    }

    fn reference(&mut self, index: usize, code: &FieldCode, name: &str) -> Evaluated {
        let japanese = self.japanese();
        let Some(text) = self.bookmark_text(name) else {
            return Evaluated::Final(error_text(
                japanese,
                "Error! Reference source not found.",
                "エラー! 参照元が見つかりません。",
            ));
        };
        if code.has_switch("p")
            && !(code.has_switch("n") || code.has_switch("r") || code.has_switch("w"))
        {
            let here = self.field(index).paragraph;
            let target = self
                .story
                .bookmarks
                .get(&name.to_lowercase())
                .map(|b| b.paragraph);
            if let Some(target) = target {
                return Evaluated::Final(self.relative_word(target < here));
            }
        }
        if code.has_switch("n") || code.has_switch("r") || code.has_switch("w") {
            // Paragraph numbers need list context: keep Word's.
            return Evaluated::Keep;
        }
        // A bookmark spanning a paragraph mark ends its text at the mark.
        let text = text.trim_end_matches('\n').to_string();
        Evaluated::Value(FieldValue::Text(text))
    }

    fn style_reference(&self, index: usize, code: &FieldCode) -> Evaluated {
        let japanese = self.japanese();
        let Some(name) = code.operand(0) else {
            return Evaluated::Keep;
        };
        let Some(style_id) = self.data.style_id_for(name) else {
            return Evaluated::Final(error_text(
                japanese,
                "Error! No text of specified style in document.",
                "エラー! 指定したスタイルは使われていません。",
            ));
        };
        if code.has_switch("n") || code.has_switch("r") || code.has_switch("w") {
            return Evaluated::Keep;
        }
        let paragraphs = if self.kind == StoryKind::HeaderFooter {
            &self.data.body_paragraphs
        } else {
            &self.story.paragraphs
        };
        let matches = |paragraph: &&super::scan::ScannedParagraph| {
            paragraph.style_id.as_deref() == Some(style_id.as_str())
                && !paragraph.text.trim().is_empty()
        };
        let found = match self.kind {
            _ if paragraphs.is_empty() => None,
            StoryKind::Body | StoryKind::Notes => {
                let here = self.field(index).paragraph;
                paragraphs[..=here.min(paragraphs.len().saturating_sub(1))]
                    .iter()
                    .rev()
                    .find(matches)
                    .or_else(|| paragraphs[here..].iter().find(matches))
            }
            // A header reads the body, which is not this story; the caller
            // passes the body's paragraphs for headers.
            StoryKind::HeaderFooter => {
                if code.has_switch("l") {
                    paragraphs.iter().rev().find(matches)
                } else {
                    paragraphs.iter().find(matches)
                }
            }
        };
        match found {
            Some(paragraph) => {
                let text = paragraph.text.trim_end_matches('\n').to_string();
                if code.has_switch("p") {
                    let here = self.field(index).paragraph;
                    let target = paragraphs
                        .iter()
                        .position(|candidate| candidate.id == paragraph.id)
                        .unwrap_or(0);
                    return Evaluated::Final(self.relative_word(target < here));
                }
                Evaluated::Value(FieldValue::Text(text))
            }
            None => Evaluated::Final(error_text(
                japanese,
                "Error! No text of specified style in document.",
                "エラー! 指定したスタイルは使われていません。",
            )),
        }
    }

    fn heading_level(&self, paragraph: usize) -> Option<u8> {
        let paragraph = self.story.paragraphs.get(paragraph)?;
        let level = paragraph.outline_level.or_else(|| {
            paragraph
                .style_id
                .as_ref()
                .and_then(|style| self.data.style_outline.get(style).copied())
        })?;
        (level <= 8).then_some(level + 1)
    }

    fn sequence(&mut self, index: usize, code: &FieldCode) -> Evaluated {
        let Some(identifier) = code.operand(0).map(str::to_lowercase) else {
            return Evaluated::Keep;
        };
        if code.operand(1).is_some() {
            // Resolved after the pass, from the bookmark's position.
            return Evaluated::Keep;
        }
        let paragraph = self.field(index).paragraph;
        let restart_level = code
            .switch_value("s")
            .and_then(|level| level.parse::<u8>().ok());
        let state = self.seq.entry(identifier.clone()).or_default();
        if let Some(level) = restart_level {
            let from = state.last_paragraph.map_or(0, |last| last + 1);
            let restarted = (from..=paragraph).any(|candidate| {
                self.story
                    .paragraphs
                    .get(candidate)
                    .and_then(|p| {
                        p.outline_level.or_else(|| {
                            p.style_id
                                .as_ref()
                                .and_then(|style| self.data.style_outline.get(style).copied())
                        })
                    })
                    .is_some_and(|outline| outline < level)
            });
            if restarted {
                state.value = 0;
            }
        }
        let value =
            if let Some(restart) = code.switch_value("r").and_then(|v| v.parse::<u32>().ok()) {
                restart
            } else if code.has_switch("c") {
                state.value
            } else {
                state.value + 1
            };
        state.value = value;
        state.last_paragraph = Some(paragraph);
        self.seq_history
            .entry(identifier)
            .or_default()
            .push((paragraph, value));
        if code.has_switch("h") && code.format.formats.is_empty() {
            return Evaluated::Hidden;
        }
        Evaluated::Value(FieldValue::Number(value as f64))
    }

    fn resolve_seq_bookmark(&mut self, index: usize) {
        let field = &self.story.fields[index];
        if field.locked || !self.is_selected(index) {
            return;
        }
        let instruction =
            field.instruction_with(|child| self.values[child].clone().unwrap_or_default());
        let Some(code) = FieldCode::parse(&instruction) else {
            return;
        };
        if code.name != "SEQ" {
            return;
        }
        let (Some(identifier), Some(bookmark)) = (code.operand(0), code.operand(1)) else {
            return;
        };
        let Some(target) = self.story.bookmarks.get(&bookmark.to_lowercase()) else {
            return;
        };
        let value = self
            .seq_history
            .get(&identifier.to_lowercase())
            .and_then(|history| {
                history
                    .iter()
                    .filter(|(paragraph, _)| *paragraph <= target.paragraph)
                    .last()
                    .map(|(_, value)| *value)
            })
            .unwrap_or(0);
        let text = apply_general_format(&FieldValue::Number(value as f64), &code.format);
        self.values[index] = Some(text.clone());
        self.outcomes[index] = Some(Outcome {
            result: NewResult::Text(text),
            format: code.format.clone(),
            symbol_font: None,
        });
    }

    fn list_number(&mut self, index: usize, code: &FieldCode) -> Evaluated {
        // LISTNUM [name] [\l level] [\s start]
        let name = code
            .operand(0)
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_else(|| "numberdefault".to_string());
        let paragraph = self.field(index).paragraph;
        // Each further LISTNUM in a paragraph goes one level deeper.
        let default_level = match self.listnum_paragraph {
            Some((last, level)) if last == paragraph => (level + 1).min(8),
            _ => self
                .story
                .paragraphs
                .get(paragraph)
                .and_then(|p| p.numbering.as_ref().map(|(_, level)| *level as usize))
                .unwrap_or(0),
        };
        let level = code
            .switch_value("l")
            .and_then(|level| level.parse::<usize>().ok())
            .map(|level| level.clamp(1, 9) - 1)
            .unwrap_or(default_level);
        self.listnum_paragraph = Some((paragraph, level));
        let counters = self.listnum.entry(name.clone()).or_insert([0; 9]);
        counters[level] = match code.switch_value("s").and_then(|v| v.parse::<u32>().ok()) {
            Some(start) => start,
            None => counters[level] + 1,
        };
        for deeper in counters.iter_mut().skip(level + 1) {
            *deeper = 0;
        }
        let counters = *counters;
        let text = match name.as_str() {
            "legaldefault" => {
                let mut parts = Vec::new();
                for value in counters.iter().take(level + 1) {
                    parts.push(value.max(&1).to_string());
                }
                format!("{}.", parts.join("."))
            }
            "outlinedefault" => {
                const LEVELS: [(&str, &str, &str); 9] = [
                    ("", "upperRoman", "."),
                    ("", "upperLetter", "."),
                    ("", "decimal", "."),
                    ("", "lowerLetter", ")"),
                    ("(", "decimal", ")"),
                    ("(", "lowerLetter", ")"),
                    ("(", "lowerRoman", ")"),
                    ("(", "lowerLetter", ")"),
                    ("(", "lowerRoman", ")"),
                ];
                let (before, format, after) = LEVELS[level];
                format!(
                    "{before}{}{after}",
                    format_number_value(format, counters[level].max(1), Some("en-US"))
                )
            }
            _ => {
                const LEVELS: [(&str, &str, &str); 9] = [
                    ("", "decimal", ")"),
                    ("", "lowerLetter", ")"),
                    ("", "lowerRoman", ")"),
                    ("(", "decimal", ")"),
                    ("(", "lowerLetter", ")"),
                    ("(", "lowerRoman", ")"),
                    ("", "decimal", "."),
                    ("", "lowerLetter", "."),
                    ("", "lowerRoman", "."),
                ];
                let (before, format, after) = LEVELS[level];
                format!(
                    "{before}{}{after}",
                    format_number_value(format, counters[level].max(1), Some("en-US"))
                )
            }
        };
        Evaluated::Value(FieldValue::Text(text))
    }

    fn auto_number(&mut self, index: usize, code: &FieldCode) -> Evaluated {
        let paragraph = self.field(index).paragraph;
        let level = self
            .heading_level(paragraph)
            .map(|level| level as usize - 1)
            .unwrap_or(0);
        self.autonum[level] += 1;
        for deeper in self.autonum.iter_mut().skip(level + 1) {
            *deeper = 0;
        }
        let separator = code.switch_value("s").unwrap_or(".");
        let text = match code.name.as_str() {
            "AUTONUMLGL" => {
                let parts: Vec<String> = self
                    .autonum
                    .iter()
                    .take(level + 1)
                    .map(|value| value.max(&1).to_string())
                    .collect();
                let mut text = parts.join(".");
                if !code.has_switch("e") {
                    text.push('.');
                }
                text
            }
            "AUTONUMOUT" => {
                const FORMATS: [(&str, &str); 9] = [
                    ("upperRoman", "."),
                    ("upperLetter", "."),
                    ("decimal", "."),
                    ("lowerLetter", ")"),
                    ("decimal", ")"),
                    ("lowerLetter", ")"),
                    ("lowerRoman", ")"),
                    ("lowerLetter", "."),
                    ("lowerRoman", "."),
                ];
                let (format, after) = FORMATS[level];
                format!(
                    "{}{after}",
                    format_number_value(format, self.autonum[level], Some("en-US"))
                )
            }
            _ => format!("{}{separator}", self.autonum[level]),
        };
        Evaluated::Value(FieldValue::Text(text))
    }

    /// An IF/COMPARE operand's value: a bookmark name stands for its text.
    fn operand_value(&self, operand: &jade_docx_reader::field_format::FieldOperand) -> String {
        if operand.quoted {
            return operand.text.clone();
        }
        self.bookmark_text(&operand.text)
            .unwrap_or_else(|| operand.text.clone())
    }

    fn compare(&self, operands: &[jade_docx_reader::field_format::FieldOperand]) -> Option<bool> {
        let (left, operator, right) = match operands {
            [left, operator, right, ..] if !operator.quoted => {
                (left, operator.text.as_str(), right)
            }
            [only, ..] => {
                // An expression on its own: true when not zero.
                let value = self.operand_value(only);
                return Some(
                    jade_docx_reader::field_format::parse_field_number(&value)
                        .map(|number| number != 0.0)
                        .unwrap_or(!value.is_empty()),
                );
            }
            _ => return None,
        };
        let left_value = self.operand_value(left);
        let right_value = self.operand_value(right);
        let numbers = (
            jade_docx_reader::field_format::parse_field_number(&left_value),
            jade_docx_reader::field_format::parse_field_number(&right_value),
        );
        if let (Some(a), Some(b), false) = (
            numbers.0,
            numbers.1,
            right.quoted && (right_value.contains('*') || right_value.contains('?')),
        ) {
            return Some(match operator {
                "=" => a == b,
                "<>" => a != b,
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                ">=" => a >= b,
                _ => return None,
            });
        }
        Some(match operator {
            "=" => wildcard_match(&right_value, &left_value),
            "<>" => !wildcard_match(&right_value, &left_value),
            "<" => left_value < right_value,
            "<=" => left_value <= right_value,
            ">" => left_value > right_value,
            ">=" => left_value >= right_value,
            _ => return None,
        })
    }

    fn condition(&self, code: &FieldCode) -> Evaluated {
        let operands = &code.operands;
        // IF left op right "true" ["false"], or IF expression "true" ["false"].
        let comparison_len = if operands.len() >= 4
            && !operands[1].quoted
            && matches!(
                operands[1].text.as_str(),
                "=" | "<>" | "<" | "<=" | ">" | ">="
            ) {
            3
        } else {
            1
        };
        let Some(result) = self.compare(&operands[..comparison_len.min(operands.len())]) else {
            return Evaluated::Keep;
        };
        let pick = if result {
            operands.get(comparison_len)
        } else {
            operands.get(comparison_len + 1)
        };
        Evaluated::Value(FieldValue::Text(
            pick.map(|operand| operand.text.clone()).unwrap_or_default(),
        ))
    }

    fn formula(&self, index: usize, code: &FieldCode) -> Evaluated {
        let field = self.field(index);
        let table = field
            .cell
            .and_then(|cell| self.story.tables.get(cell.table));
        let bookmark = |name: &str| self.bookmark_text(name);
        let context = FormulaContext {
            table,
            cell: field.cell,
            bookmark: &bookmark,
        };
        match evaluate_formula(code.expression.as_deref().unwrap_or_default(), &context) {
            Ok(value) => {
                if code.format.numeric_picture.is_none()
                    && !code.format.formats.iter().any(|name| {
                        jade_docx_reader::field_format::format_number_switch(1.0, name).is_some()
                            || name.eq_ignore_ascii_case("DOLLARTEXT")
                    })
                {
                    return Evaluated::Value(FieldValue::Text(format_plain_number(value)));
                }
                Evaluated::Value(FieldValue::Number(value))
            }
            Err(error) => Evaluated::Final(error.message(self.japanese())),
        }
    }

    fn include_text(&self, code: &FieldCode) -> Evaluated {
        let japanese = self.japanese();
        let Some(path) = code.operand(0) else {
            return Evaluated::Final(error_text(
                japanese,
                "Error! No filename specified.",
                "エラー! ファイル名が指定されていません。",
            ));
        };
        let path = unescape_path(path);
        let Some(read) = self.env.read_file.as_ref() else {
            return Evaluated::Keep;
        };
        let Some(bytes) = read(&path) else {
            // Unreachable files keep the result Word saved.
            return Evaluated::Keep;
        };
        let bookmark = code.operand(1);
        let paragraphs = match included_paragraphs(&bytes, bookmark) {
            Some(paragraphs) => paragraphs,
            None => return Evaluated::Keep,
        };
        if paragraphs.len() <= 1 {
            Evaluated::Value(FieldValue::Text(
                paragraphs.into_iter().next().unwrap_or_default(),
            ))
        } else {
            Evaluated::Paragraphs(paragraphs)
        }
    }

    fn index(&self, code: &FieldCode) -> Evaluated {
        // Entries from the story's XE fields: "main:sub" builds levels.
        let mut entries: Vec<(String, Option<String>, usize, Option<String>)> = Vec::new();
        for field in &self.story.fields {
            let instruction = field.instruction_with(|_| String::new());
            let Some(xe) = FieldCode::parse(&instruction) else {
                continue;
            };
            if xe.name != "XE" {
                continue;
            }
            if let (Some(kind), Some(wanted)) = (xe.switch_value("f"), code.switch_value("f")) {
                if !kind.eq_ignore_ascii_case(wanted) {
                    continue;
                }
            }
            let Some(term) = xe.operand(0) else {
                continue;
            };
            let mut levels = term.splitn(2, ':');
            let main = levels.next().unwrap_or_default().trim().to_string();
            let sub = levels.next().map(|sub| sub.trim().to_string());
            let reading = xe.switch_value("y").map(str::to_string);
            entries.push((main, sub, field.paragraph, reading));
        }
        if entries.is_empty() {
            return Evaluated::Final(error_text(
                self.japanese(),
                "No index entries found.",
                "索引項目が見つかりません。",
            ));
        }
        let separator = code
            .switch_value("e")
            .map(str::to_string)
            .unwrap_or_else(|| ", ".to_string());
        let list_separator = code.switch_value("l").unwrap_or(", ").to_string();
        let mut grouped: Vec<((String, String), Option<String>, Vec<usize>)> = Vec::new();
        for (main, sub, paragraph, reading) in entries {
            let key = (
                reading.unwrap_or_else(|| main.clone()).to_lowercase(),
                main.clone(),
            );
            let page = (self.page_of)(paragraph);
            match grouped
                .iter_mut()
                .find(|(existing, existing_sub, _)| existing.1 == main && *existing_sub == sub)
            {
                Some((_, _, pages)) => pages.extend(page),
                None => grouped.push((key, sub, page.into_iter().collect())),
            }
        }
        grouped.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut lines = Vec::new();
        let mut last_main: Option<String> = None;
        for ((_, main), sub, mut pages) in grouped {
            pages.sort_unstable();
            pages.dedup();
            let pages = pages
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(&list_separator);
            match sub {
                Some(sub) => {
                    if last_main.as_deref() != Some(main.as_str()) {
                        lines.push(main.clone());
                    }
                    lines.push(format!("\u{3000}{sub}{separator}{pages}"));
                }
                None => lines.push(if pages.is_empty() {
                    main.clone()
                } else {
                    format!("{main}{separator}{pages}")
                }),
            }
            last_main = Some(main);
        }
        Evaluated::Paragraphs(lines)
    }

    fn table_of_authorities(&self, code: &FieldCode) -> Evaluated {
        let category = code.switch_value("c").unwrap_or("1");
        let mut entries: Vec<(String, Vec<usize>)> = Vec::new();
        let mut short_names: HashMap<String, String> = HashMap::new();
        for field in &self.story.fields {
            let instruction = field.instruction_with(|_| String::new());
            let Some(ta) = FieldCode::parse(&instruction) else {
                continue;
            };
            if ta.name != "TA" {
                continue;
            }
            let long = ta.switch_value("l").map(str::to_string);
            let short = ta.switch_value("s").map(str::to_string);
            if let (Some(long), Some(short)) = (&long, &short) {
                short_names.insert(short.to_lowercase(), long.clone());
            }
            let entry_category = ta.switch_value("c").unwrap_or("1");
            let name = long.clone().or_else(|| {
                short
                    .as_ref()
                    .and_then(|short| short_names.get(&short.to_lowercase()).cloned())
            });
            let Some(name) = name else { continue };
            if entry_category != category && category != "0" {
                continue;
            }
            let page = (self.page_of)(field.paragraph);
            match entries.iter_mut().find(|(existing, _)| *existing == name) {
                Some((_, pages)) => pages.extend(page),
                None => entries.push((name, page.into_iter().collect())),
            }
        }
        if entries.is_empty() {
            return Evaluated::Final(error_text(
                self.japanese(),
                "No table of authorities entries found.",
                "引用文献一覧の項目が見つかりません。",
            ));
        }
        entries.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        let separator = code.switch_value("e").unwrap_or("\t").to_string();
        let lines = entries
            .into_iter()
            .map(|(name, mut pages)| {
                pages.sort_unstable();
                pages.dedup();
                let pages = if code.has_switch("p") && pages.len() >= 5 {
                    "passim".to_string()
                } else {
                    pages
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!("{name}{separator}{pages}")
            })
            .collect();
        Evaluated::Paragraphs(lines)
    }
}

/// Word's `?` (one character) and `*` (any run) in IF/COMPARE's right
/// operand.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return pattern == text;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.first() {
            None => text.is_empty(),
            Some('*') => (0..=text.len()).any(|skip| matches(&pattern[1..], &text[skip..])),
            Some('?') => !text.is_empty() && matches(&pattern[1..], &text[1..]),
            Some(c) => text.first() == Some(c) && matches(&pattern[1..], &text[1..]),
        }
    }
    matches(&pattern, &text)
}

/// The paragraphs of an included file: a `.docx`'s body (or one bookmark
/// of it), or plain text.
pub(crate) fn included_paragraphs(bytes: &[u8], bookmark: Option<&str>) -> Option<Vec<String>> {
    if bytes.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
        let mut xml = String::new();
        {
            use std::io::Read;
            archive
                .by_name("word/document.xml")
                .ok()?
                .read_to_string(&mut xml)
                .ok()?;
        }
        let doc = crate::docx_xml::DocxXmlDocument::parse(&xml).ok()?;
        let story = super::scan::scan_document(doc.root());
        if let Some(name) = bookmark {
            let bookmark = story.bookmarks.get(&name.to_lowercase())?;
            return Some(bookmark.text.split('\n').map(str::to_string).collect());
        }
        return Some(
            story
                .paragraphs
                .iter()
                .filter(|paragraph| paragraph.cell.is_none())
                .map(|paragraph| paragraph.text.clone())
                .collect(),
        );
    }
    let text = String::from_utf8(bytes.to_vec()).ok()?;
    let text = text.trim_start_matches('\u{feff}');
    Some(text.lines().map(str::to_string).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_as_words_if_does() {
        assert!(wildcard_match("Jo*", "John"));
        assert!(wildcard_match("J?hn", "John"));
        assert!(!wildcard_match("J?hn", "Joahn"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("Jo", "John"));
    }

    #[test]
    fn style_names_match_japanese_and_numbered_headings() {
        assert_eq!(normalize_style_name("見出し 1"), "heading1");
        assert_eq!(normalize_style_name("Heading 2"), "heading2");
        assert_eq!(normalize_style_name("3"), "heading3");
        assert_eq!(normalize_style_name("標準"), "normal");
    }
}
