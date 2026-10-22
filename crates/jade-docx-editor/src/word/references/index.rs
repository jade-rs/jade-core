//! 索引 and 引用文献一覧: index entries marked with `XE` — main entry,
//! subentry, reading (読み), a cross-reference, the current page or a
//! bookmarked range, bold or italic numbers — and the index built from
//! them (`INDEX \e \h \c \y \z`), in Japanese reading order with あかさたな
//! headings; citations marked with `TA` and the table of authorities built
//! by category (`TOA \h \c \p \e`).

use std::collections::BTreeMap;

use crate::word_document::{FieldBlockLine, WordDocument};
use jade_selection::TextPosition;

use super::edit_document;
use crate::word::{Position, WordSession};

/// A page reference: the page's text, and whether it is bold and italic.
type PageRef = (String, bool, bool);
/// A subentry's reading, page references and cross-references (see).
type SubEntry = (Option<String>, Vec<PageRef>, Vec<String>);

/// 索引登録's ページ番号の書式 options other than the number itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryReference {
    /// 現在のページ.
    CurrentPage,
    /// 相互参照: `\t "→ 参照先"`.
    CrossReference(String),
    /// ページ範囲: the pages a bookmark spans (`\r`).
    PageRange(String),
}

/// The 索引登録 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkEntryDialog {
    pub main: String,
    pub main_reading: String,
    pub sub: String,
    pub sub_reading: String,
    pub reference: EntryReference,
    pub bold: bool,
    pub italic: bool,
}

impl MarkEntryDialog {
    /// Opened on the selected text.
    pub fn for_text(text: &str) -> Self {
        Self {
            main: text.trim().to_string(),
            main_reading: String::new(),
            sub: String::new(),
            sub_reading: String::new(),
            reference: EntryReference::CurrentPage,
            bold: false,
            italic: false,
        }
    }

    /// The `XE` instruction.
    pub fn instruction(&self) -> Result<String, String> {
        let clean = |text: &str| text.replace(['"', '\\', ':'], "").trim().to_string();
        let main = clean(&self.main);
        if main.is_empty() {
            return Err("登録 (メイン) を入力してください".to_string());
        }
        let sub = clean(&self.sub);
        let entry = if sub.is_empty() {
            main
        } else {
            format!("{main}:{sub}")
        };
        let mut instruction = format!("XE \"{entry}\"");
        let main_reading = clean(&self.main_reading);
        let sub_reading = clean(&self.sub_reading);
        if !main_reading.is_empty() || !sub_reading.is_empty() {
            let reading = if sub.is_empty() || sub_reading.is_empty() {
                main_reading
            } else {
                format!("{main_reading}:{sub_reading}")
            };
            instruction.push_str(&format!(" \\y \"{reading}\""));
        }
        match &self.reference {
            EntryReference::CurrentPage => {}
            EntryReference::CrossReference(target) => {
                let target = target.replace(['"', '\\'], "");
                instruction.push_str(&format!(" \\t \"{}\"", target.trim()));
            }
            EntryReference::PageRange(bookmark) => {
                if bookmark.trim().is_empty() {
                    return Err("ブックマークを選んでください".to_string());
                }
                instruction.push_str(&format!(" \\r {}", bookmark.trim()));
            }
        }
        if self.bold {
            instruction.push_str(" \\b");
        }
        if self.italic {
            instruction.push_str(" \\i");
        }
        Ok(instruction)
    }
}

/// A marked index entry, read back from its `XE` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub main: String,
    pub sub: Option<String>,
    pub main_reading: Option<String>,
    pub sub_reading: Option<String>,
    pub see: Option<String>,
    pub range: Option<String>,
    pub bold: bool,
    pub italic: bool,
    /// The paragraph the field is in.
    pub paragraph: usize,
}

/// Splits a field instruction into words, quoted ones kept whole.
fn words(instruction: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in instruction.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                if !quoted {
                    words.push(std::mem::take(&mut current));
                }
            }
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Reads an `XE` instruction.
pub fn parse_entry(instruction: &str, paragraph: usize) -> Option<IndexEntry> {
    let words = words(instruction);
    if !words.first()?.eq_ignore_ascii_case("XE") {
        return None;
    }
    let text = words.get(1)?.clone();
    let mut parts = text.splitn(2, ':');
    let main = parts.next()?.trim().to_string();
    let sub = parts
        .next()
        .map(|sub| sub.trim().to_string())
        .filter(|sub| !sub.is_empty());
    let mut entry = IndexEntry {
        main,
        sub,
        main_reading: None,
        sub_reading: None,
        see: None,
        range: None,
        bold: false,
        italic: false,
        paragraph,
    };
    let mut index = 2;
    while index < words.len() {
        match words[index].to_ascii_lowercase().as_str() {
            "\\y" => {
                if let Some(reading) = words.get(index + 1) {
                    let mut parts = reading.splitn(2, ':');
                    entry.main_reading = parts.next().map(str::to_string).filter(|r| !r.is_empty());
                    entry.sub_reading = parts.next().map(str::to_string).filter(|r| !r.is_empty());
                    index += 1;
                }
            }
            "\\t" => {
                entry.see = words.get(index + 1).cloned();
                index += 1;
            }
            "\\r" => {
                entry.range = words.get(index + 1).cloned();
                index += 1;
            }
            "\\b" => entry.bold = true,
            "\\i" => entry.italic = true,
            _ => {}
        }
        index += 1;
    }
    Some(entry)
}

/// 種類.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexLayout {
    /// インデント: a subentry on a line of its own.
    Indented,
    /// 追い込み: subentries after the main entry on its line (`\r`).
    RunIn,
}

/// 言語 for the order and the headings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexLanguage {
    /// 日本語: reading order, あかさたな headings (`\z "1041"`).
    Japanese,
    /// 英語: alphabetical, A–Z headings (`\z "1033"`).
    English,
}

/// The 索引 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexDialog {
    pub layout: IndexLayout,
    /// 段数.
    pub columns: u8,
    pub language: IndexLanguage,
    /// ページ番号を右揃えにする.
    pub right_align: bool,
    pub leader: &'static str,
    /// 見出し (`\h`): a heading before each letter's or row's entries.
    pub headings: bool,
}

impl Default for IndexDialog {
    fn default() -> Self {
        Self {
            layout: IndexLayout::Indented,
            columns: 2,
            language: IndexLanguage::Japanese,
            right_align: false,
            leader: "dot",
            headings: true,
        }
    }
}

impl IndexDialog {
    /// The `INDEX` instruction.
    pub fn instruction(&self) -> String {
        let mut instruction = String::from("INDEX");
        if self.right_align {
            instruction.push_str(" \\e \"\t\"");
        }
        if self.headings {
            let heading = match self.language {
                IndexLanguage::Japanese => "あ",
                IndexLanguage::English => "A",
            };
            instruction.push_str(&format!(" \\h \"{heading}\""));
        }
        instruction.push_str(&format!(" \\c \"{}\"", self.columns.max(1)));
        if self.layout == IndexLayout::RunIn {
            instruction.push_str(" \\r");
        }
        if self.language == IndexLanguage::Japanese {
            instruction.push_str(" \\y");
        }
        instruction.push_str(match self.language {
            IndexLanguage::Japanese => " \\z \"1041\"",
            IndexLanguage::English => " \\z \"1033\"",
        });
        instruction
    }
}

/// Katakana to hiragana and full-width to half-width, so readings sort as
/// one alphabet.
fn sort_key(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            'ァ'..='ヶ' => char::from_u32(ch as u32 - 0x60).unwrap_or(ch),
            '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' => {
                char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
            }
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// The あかさたな row (or the letter) an entry's reading files under.
pub fn heading_of(key: &str, language: IndexLanguage) -> String {
    let Some(first) = key.chars().next() else {
        return String::new();
    };
    if first.is_ascii_alphabetic() {
        return first.to_ascii_uppercase().to_string();
    }
    if language == IndexLanguage::Japanese {
        const ROWS: [(char, char); 10] = [
            ('あ', 'お'),
            ('か', 'ご'),
            ('さ', 'ぞ'),
            ('た', 'ど'),
            ('な', 'の'),
            ('は', 'ぽ'),
            ('ま', 'も'),
            ('ゃ', 'よ'),
            ('ら', 'ろ'),
            ('ゎ', 'ん'),
        ];
        let first = match first {
            'ぁ' => 'あ',
            'ぃ' => 'い',
            'ぅ' => 'う',
            'ぇ' => 'え',
            'ぉ' => 'お',
            other => other,
        };
        for (start, end) in ROWS {
            if (start..=end).contains(&first) {
                let label = if start == 'ゃ' {
                    'や'
                } else if start == 'ゎ' {
                    'わ'
                } else {
                    start
                };
                return label.to_string();
            }
        }
    }
    if first.is_ascii_digit() || !first.is_alphanumeric() {
        return "記号".to_string();
    }
    first.to_string()
}

/// Page numbers as an index shows them: ascending, each once, a range with
/// an en dash.
fn page_text(pages: &[(String, bool, bool)]) -> String {
    let mut seen: Vec<&(String, bool, bool)> = Vec::new();
    for page in pages {
        if !seen.iter().any(|other| other.0 == page.0) {
            seen.push(page);
        }
    }
    seen.iter()
        .map(|(text, _, _)| text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The index's lines for `entries`, with `page_of` giving a paragraph's
/// page and `range_of` a bookmark's first and last pages.
pub fn index_lines(
    entries: &[IndexEntry],
    dialog: &IndexDialog,
    page_of: &dyn Fn(usize) -> Option<usize>,
    range_of: &dyn Fn(&str) -> Option<(usize, usize)>,
) -> Vec<FieldBlockLine> {
    // main → (reading, main pages, sub → (reading, pages), see)
    #[derive(Default)]
    struct Main {
        reading: Option<String>,
        pages: Vec<PageRef>,
        see: Vec<String>,
        subs: BTreeMap<String, SubEntry>,
    }
    let mut mains: BTreeMap<String, Main> = BTreeMap::new();
    for entry in entries {
        let page = match &entry.range {
            Some(bookmark) => range_of(bookmark).map(|(first, last)| {
                if first == last {
                    first.to_string()
                } else {
                    format!("{first}–{last}")
                }
            }),
            None => page_of(entry.paragraph).map(|page| page.to_string()),
        };
        let main = mains.entry(entry.main.clone()).or_default();
        if main.reading.is_none() {
            main.reading = entry.main_reading.clone();
        }
        let (pages, see) = match &entry.sub {
            Some(sub) => {
                let slot = main.subs.entry(sub.clone()).or_default();
                if slot.0.is_none() {
                    slot.0 = entry.sub_reading.clone();
                }
                (&mut slot.1, &mut slot.2)
            }
            None => (&mut main.pages, &mut main.see),
        };
        match &entry.see {
            Some(target) => {
                if !see.contains(target) {
                    see.push(target.clone());
                }
            }
            None => {
                if let Some(page) = page {
                    pages.push((page, entry.bold, entry.italic));
                }
            }
        }
    }
    let order_key = |text: &str, reading: &Option<String>| match dialog.language {
        IndexLanguage::Japanese => sort_key(reading.as_deref().unwrap_or(text)),
        IndexLanguage::English => text.to_lowercase(),
    };
    let mut sorted: Vec<(String, String, Main)> = mains
        .into_iter()
        .map(|(text, main)| (order_key(&text, &main.reading), text, main))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let separator = if dialog.right_align {
        None
    } else {
        Some(", ".to_string())
    };
    let numbers = |pages: &[(String, bool, bool)], see: &[String]| -> Option<String> {
        let mut text = page_text(pages);
        for target in see {
            if !text.is_empty() {
                text.push_str("; ");
            }
            text.push_str(target);
        }
        (!text.is_empty()).then_some(text)
    };
    let line = |text: String, page: Option<String>, level: u8| FieldBlockLine {
        text,
        separator: if page
            .as_deref()
            .is_some_and(|page| page.starts_with('→') || page.contains("参照"))
        {
            Some(" ".to_string())
        } else {
            separator.clone()
        },
        page,
        leader: Some(dialog.leader.to_string()),
        indent: 10.5 * f32::from(level.saturating_sub(1) + 1),
        hanging: 10.5,
        style: Some(format!("Index{level}")),
        ..FieldBlockLine::default()
    };
    let mut lines = Vec::new();
    let mut last_heading = None;
    for (key, text, main) in sorted {
        if dialog.headings {
            let heading = heading_of(&key, dialog.language);
            if last_heading.as_ref() != Some(&heading) {
                lines.push(FieldBlockLine {
                    text: heading.clone(),
                    bold: true,
                    space_before: if lines.is_empty() { 0.0 } else { 6.0 },
                    style: Some("IndexHeading".to_string()),
                    ..FieldBlockLine::default()
                });
                last_heading = Some(heading);
            }
        }
        let mut subs: Vec<(String, String, SubEntry)> = main
            .subs
            .into_iter()
            .map(|(sub, value)| (order_key(&sub, &value.0), sub, value))
            .collect();
        subs.sort_by(|a, b| a.0.cmp(&b.0));
        match dialog.layout {
            IndexLayout::Indented => {
                lines.push(line(text, numbers(&main.pages, &main.see), 1));
                for (_, sub, (_, pages, see)) in subs {
                    lines.push(line(sub, numbers(&pages, &see), 2));
                }
            }
            IndexLayout::RunIn => {
                let mut run_in = text;
                if let Some(numbers) = numbers(&main.pages, &main.see) {
                    run_in.push_str(", ");
                    run_in.push_str(&numbers);
                }
                for (index, (_, sub, (_, pages, see))) in subs.into_iter().enumerate() {
                    run_in.push_str(if index == 0 { ": " } else { "; " });
                    run_in.push_str(&sub);
                    if let Some(numbers) = numbers(&pages, &see) {
                        run_in.push_str(", ");
                        run_in.push_str(&numbers);
                    }
                }
                lines.push(line(run_in, None, 1));
            }
        }
    }
    lines
}

/// 引用文献の分類 Word for Mac lists, by `\c` number.
pub const TOA_CATEGORIES: [&str; 7] = [
    "判例",
    "法令",
    "その他の資料",
    "規則",
    "論文",
    "規約",
    "憲法規定",
];

/// The 引用文の登録 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkCitationDialog {
    /// 長い引用文.
    pub long: String,
    /// 短い引用文.
    pub short: String,
    /// 1-based category.
    pub category: u8,
}

impl MarkCitationDialog {
    pub fn instruction(&self) -> Result<String, String> {
        let clean = |text: &str| text.replace(['"', '\\'], "").trim().to_string();
        let long = clean(&self.long);
        if long.is_empty() {
            return Err("引用文を入力してください".to_string());
        }
        let short = clean(&self.short);
        let short = if short.is_empty() {
            long.clone()
        } else {
            short
        };
        Ok(format!(
            "TA \\l \"{long}\" \\s \"{short}\" \\c {}",
            self.category.clamp(1, 16)
        ))
    }
}

/// A citation marked for the table of authorities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityEntry {
    pub long: Option<String>,
    pub short: Option<String>,
    pub category: u8,
    pub paragraph: usize,
}

pub fn parse_authority(instruction: &str, paragraph: usize) -> Option<AuthorityEntry> {
    let words = words(instruction);
    if !words.first()?.eq_ignore_ascii_case("TA") {
        return None;
    }
    let mut entry = AuthorityEntry {
        long: None,
        short: None,
        category: 1,
        paragraph,
    };
    let mut index = 1;
    while index < words.len() {
        match words[index].to_ascii_lowercase().as_str() {
            "\\l" => {
                entry.long = words.get(index + 1).cloned();
                index += 1;
            }
            "\\s" => {
                entry.short = words.get(index + 1).cloned();
                index += 1;
            }
            "\\c" => {
                entry.category = words
                    .get(index + 1)
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(1);
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    Some(entry)
}

/// The 引用文献一覧 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritiesDialog {
    /// The category, or every one (`None`).
    pub category: Option<u8>,
    /// "前掲" (passim) for a citation on five pages or more (`\p`).
    pub passim: bool,
    pub leader: &'static str,
}

impl Default for AuthoritiesDialog {
    fn default() -> Self {
        Self {
            category: None,
            passim: true,
            leader: "dot",
        }
    }
}

/// The table of authorities' blocks: one per category, each its heading
/// (`\h`) and its citations, long form, with their pages.
pub fn authorities_lines(
    entries: &[AuthorityEntry],
    category: u8,
    dialog: &AuthoritiesDialog,
    page_of: &dyn Fn(usize) -> Option<usize>,
) -> Vec<FieldBlockLine> {
    // A short-form citation (`TA \s`) counts for the long one it shortens.
    let mut long_of_short: BTreeMap<String, String> = BTreeMap::new();
    for entry in entries {
        if let (Some(long), Some(short)) = (&entry.long, &entry.short) {
            long_of_short.insert(short.clone(), long.clone());
        }
    }
    let mut citations: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for entry in entries {
        let long = entry.long.clone().or_else(|| {
            entry
                .short
                .as_ref()
                .and_then(|short| long_of_short.get(short).cloned())
        });
        let Some(long) = long else {
            continue;
        };
        let category_of = entries
            .iter()
            .find(|other| other.long.as_ref() == Some(&long))
            .map_or(entry.category, |other| other.category);
        if category_of != category {
            continue;
        }
        let pages = citations.entry(long).or_default();
        if let Some(page) = page_of(entry.paragraph) {
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
    }
    let mut lines = vec![FieldBlockLine {
        text: TOA_CATEGORIES
            .get(category as usize - 1)
            .copied()
            .unwrap_or("その他")
            .to_string(),
        bold: true,
        style: Some("TOAHeading".to_string()),
        ..FieldBlockLine::default()
    }];
    for (long, mut pages) in citations {
        pages.sort_unstable();
        let page = if dialog.passim && pages.len() >= 5 {
            "前掲".to_string()
        } else {
            pages
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(FieldBlockLine {
            text: long,
            page: Some(page),
            leader: Some(dialog.leader.to_string()),
            style: Some("TableofAuthorities".to_string()),
            hanging: 10.5,
            indent: 10.5,
            ..FieldBlockLine::default()
        });
    }
    lines
}

fn field_entries(document: &WordDocument) -> Vec<IndexEntry> {
    document
        .inline_fields("XE")
        .into_iter()
        .filter_map(|field| parse_entry(&field.instruction, field.paragraph))
        .collect()
}

fn authority_entries(document: &WordDocument) -> Vec<AuthorityEntry> {
    document
        .inline_fields("TA")
        .into_iter()
        .filter_map(|field| parse_authority(&field.instruction, field.paragraph))
        .collect()
}

/// Indexes and tables of authorities over every session.
pub trait WordIndex: WordSession {
    /// Every marked entry, in document order.
    fn index_entries(&self) -> Vec<IndexEntry> {
        field_entries(self.word_document())
    }

    /// 登録: an `XE` field at the end of the selection, one undo step.
    fn mark_index_entry(&mut self, dialog: &MarkEntryDialog) -> Result<(), String> {
        let instruction = dialog.instruction()?;
        let at = self
            .selection_range()
            .map(|range| range.normalized().1)
            .unwrap_or(TextPosition::new(0, 0));
        edit_document(self, "mark_index_entry", true, move |document, _| {
            document
                .insert_inline_field(at, &instruction, "")
                .map(|_| ())
        })
    }

    /// すべて登録: an `XE` after the first occurrence of the main entry's
    /// text in each paragraph, one undo step. Returns how many.
    fn mark_all_index_entries(&mut self, dialog: &MarkEntryDialog) -> Result<usize, String> {
        let instruction = dialog.instruction()?;
        let needle = dialog.main.trim().to_string();
        if needle.is_empty() {
            return Err("登録 (メイン) を入力してください".to_string());
        }
        let document = self.word_document();
        let mut places = Vec::new();
        for paragraph in 0..document.paragraph_count() {
            if !document.is_top_level_body_paragraph_surface(paragraph) {
                continue;
            }
            let Some(model) = document.paragraph(paragraph) else {
                continue;
            };
            let text: String = model.runs.iter().map(|run| run.text.as_str()).collect();
            if let Some(start) = text.find(&needle) {
                places.push(TextPosition::new(paragraph, start + needle.len()));
            }
        }
        let count = places.len();
        if count == 0 {
            return Err("文書に見つかりません".to_string());
        }
        edit_document(self, "mark_all_index_entries", true, move |document, _| {
            // From the end, so earlier places keep their offsets.
            for model in places.iter().rev() {
                let at = document.model_to_rendered_text_position(*model);
                document.insert_inline_field(at, &instruction, "")?;
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// The index's lines as the dialog would build them now.
    fn index_preview(&self, dialog: &IndexDialog) -> Vec<FieldBlockLine> {
        let entries = self.index_entries();
        let page_of = |paragraph: usize| self.paragraph_page(paragraph);
        let range_of = |name: &str| self.bookmark_pages(name);
        index_lines(&entries, dialog, &page_of, &range_of)
    }

    /// A bookmark's first and last pages.
    fn bookmark_pages(&self, name: &str) -> Option<(usize, usize)> {
        let document = self.word_document();
        let start = document.bookmark_position(name)?;
        let end = document.bookmark_end_position(name).unwrap_or(start);
        Some((
            self.paragraph_page(start.paragraph)?,
            self.paragraph_page(end.paragraph)?,
        ))
    }

    /// 索引の挿入, after the caret's paragraph, one undo step.
    fn insert_index(&mut self, dialog: &IndexDialog) -> Result<(), String> {
        let lines = self.index_preview(dialog);
        if lines.is_empty() {
            return Err("索引項目が登録されていません".to_string());
        }
        let instruction = dialog.instruction();
        let columns = dialog.columns.max(1);
        let after = self
            .selection_range()
            .map(|range| range.normalized().1.paragraph)
            .unwrap_or(0);
        edit_document(self, "insert_index", false, move |document, selection| {
            // In columns, the index gets a continuous section of its own,
            // made first: the field's paragraphs are written as they are,
            // and a break inserted beside them later would write them again
            // from the model, which has no field in it.
            let section_end = if columns > 1 {
                Some(open_section(document, after)?)
            } else {
                None
            };
            let caret = document.insert_field_block(after, &instruction, &lines)?;
            *selection = Some(caret);
            if section_end.is_some() {
                set_columns(document, after + 1, columns)?;
            }
            Ok(())
        })
    }

    /// 索引の更新: every index built again under its own instruction.
    fn update_indexes(&mut self) -> Result<usize, String> {
        let blocks: Vec<_> = self
            .word_document()
            .field_blocks()
            .into_iter()
            .filter(|block| block.name() == "INDEX")
            .collect();
        if blocks.is_empty() {
            return Err("更新する索引がありません".to_string());
        }
        let rebuilt: Vec<_> = blocks
            .iter()
            .map(|block| {
                let dialog = dialog_from_instruction(&block.instruction);
                (block.clone(), self.index_preview(&dialog))
            })
            .collect();
        let count = rebuilt.len();
        edit_document(self, "update_index", true, move |document, _| {
            for (block, lines) in rebuilt.iter().rev() {
                document.replace_field_block(block, &block.instruction, lines)?;
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// 引用文の登録: a `TA` field at the end of the selection.
    fn mark_citation(&mut self, dialog: &MarkCitationDialog) -> Result<(), String> {
        let instruction = dialog.instruction()?;
        let at = self
            .selection_range()
            .map(|range| range.normalized().1)
            .unwrap_or(TextPosition::new(0, 0));
        edit_document(self, "mark_citation", true, move |document, _| {
            document
                .insert_inline_field(at, &instruction, "")
                .map(|_| ())
        })
    }

    /// 引用文献一覧の挿入: a `TOA` field per category, one undo step.
    fn insert_table_of_authorities(&mut self, dialog: &AuthoritiesDialog) -> Result<(), String> {
        let entries = authority_entries(self.word_document());
        if entries.is_empty() {
            return Err("引用文が登録されていません".to_string());
        }
        let categories: Vec<u8> = match dialog.category {
            Some(category) => vec![category],
            None => {
                let mut categories: Vec<u8> = entries.iter().map(|entry| entry.category).collect();
                categories.sort_unstable();
                categories.dedup();
                categories
            }
        };
        let page_of = |paragraph: usize| self.paragraph_page(paragraph);
        let blocks: Vec<(String, Vec<FieldBlockLine>)> = categories
            .iter()
            .map(|category| {
                let mut instruction = format!("TOA \\h \\c \"{category}\"");
                if dialog.passim {
                    instruction.push_str(" \\p");
                }
                (
                    instruction,
                    authorities_lines(&entries, *category, dialog, &page_of),
                )
            })
            .collect();
        let after = self
            .selection_range()
            .map(|range| range.normalized().1.paragraph)
            .unwrap_or(0);
        edit_document(
            self,
            "insert_table_of_authorities",
            false,
            move |document, selection| {
                let mut anchor = after;
                for (instruction, lines) in &blocks {
                    let caret = document.insert_field_block(anchor, instruction, lines)?;
                    anchor = caret.focus.paragraph;
                    *selection = Some(caret);
                }
                Ok(())
            },
        )
    }

    /// 引用文献一覧の更新: each table built again under its instruction.
    fn update_tables_of_authorities(&mut self) -> Result<usize, String> {
        let entries = authority_entries(self.word_document());
        let blocks: Vec<_> = self
            .word_document()
            .field_blocks()
            .into_iter()
            .filter(|block| block.name() == "TOA")
            .collect();
        if blocks.is_empty() {
            return Err("更新する引用文献一覧がありません".to_string());
        }
        let page_of = |paragraph: usize| self.paragraph_page(paragraph);
        let rebuilt: Vec<_> = blocks
            .into_iter()
            .map(|block| {
                let words = words(&block.instruction);
                let category = words
                    .iter()
                    .position(|word| word.eq_ignore_ascii_case("\\c"))
                    .and_then(|index| words.get(index + 1))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                let dialog = AuthoritiesDialog {
                    category: Some(category),
                    passim: words.iter().any(|word| word.eq_ignore_ascii_case("\\p")),
                    leader: "dot",
                };
                let lines = authorities_lines(&entries, category, &dialog, &page_of);
                (block, lines)
            })
            .collect();
        let count = rebuilt.len();
        edit_document(
            self,
            "update_table_of_authorities",
            true,
            move |document, _| {
                for (block, lines) in rebuilt.iter().rev() {
                    document.replace_field_block(block, &block.instruction, lines)?;
                }
                Ok(())
            },
        )?;
        Ok(count)
    }

    /// The caret's place, as the dialogs report where things went.
    fn caret_position(&self) -> Position {
        self.selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((0, 0))
    }
}

impl<T: WordSession + ?Sized> WordIndex for T {}

/// Opens a continuous section after paragraph `after`, empty but for the
/// paragraph whose mark ends it: what goes in after `after` lands in it.
fn open_section(document: &mut WordDocument, after: usize) -> Result<usize, String> {
    use crate::word_document::SectionBreakType;
    let end = document
        .paragraph(after)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.len()).sum())
        .unwrap_or(0);
    let end = document
        .model_to_rendered_text_position(TextPosition::new(after, end))
        .offset;
    document.insert_section_break_at(after, end, SectionBreakType::Continuous)?;
    document.insert_section_break_at(after + 1, 0, SectionBreakType::Continuous)?;
    Ok(after + 1)
}

/// Lays the section holding paragraph `paragraph` out in `columns`
/// columns, as Word lays an index out.
fn set_columns(document: &mut WordDocument, paragraph: usize, columns: u8) -> Result<(), String> {
    use jade_docx_headless_api::{
        HeadlessCommand, HeadlessEditRequest, HeadlessSectionColumns, HeadlessSectionLayoutPatch,
        HeadlessSectionScope,
    };
    let section = document
        .section_index_for_paragraph(paragraph)
        .ok_or_else(|| "the index's section is unavailable".to_string())?;
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetSectionLayout {
            scope: HeadlessSectionScope::Index { index: section },
            patch: Box::new(HeadlessSectionLayoutPatch {
                columns: Some(HeadlessSectionColumns {
                    count: u32::from(columns),
                    space: 21.0,
                    equal_width: true,
                    separator: false,
                    columns: Vec::new(),
                }),
                ..HeadlessSectionLayoutPatch::default()
            }),
        }],
    };
    crate::headless_api::apply_headless_commands_to_document(document, &request)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

/// The dialog an `INDEX` instruction was built from.
pub fn dialog_from_instruction(instruction: &str) -> IndexDialog {
    let words = words(instruction);
    let mut dialog = IndexDialog {
        layout: IndexLayout::Indented,
        columns: 1,
        language: IndexLanguage::English,
        right_align: false,
        leader: "dot",
        headings: false,
    };
    let mut index = 1;
    while index < words.len() {
        match words[index].to_ascii_lowercase().as_str() {
            "\\c" => {
                dialog.columns = words
                    .get(index + 1)
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(1);
                index += 1;
            }
            "\\e" => {
                dialog.right_align = words.get(index + 1).is_some_and(|e| e.contains('\t'));
                index += 1;
            }
            "\\h" => {
                dialog.headings = true;
                index += 1;
            }
            "\\r" => dialog.layout = IndexLayout::RunIn,
            "\\y" => dialog.language = IndexLanguage::Japanese,
            "\\z" => {
                if words.get(index + 1).is_some_and(|z| z == "1041") {
                    dialog.language = IndexLanguage::Japanese;
                }
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(main: &str, sub: Option<&str>, reading: Option<&str>, paragraph: usize) -> IndexEntry {
        IndexEntry {
            main: main.to_string(),
            sub: sub.map(str::to_string),
            main_reading: reading.map(str::to_string),
            sub_reading: None,
            see: None,
            range: None,
            bold: false,
            italic: false,
            paragraph,
        }
    }

    #[test]
    fn the_mark_entry_dialog_writes_its_switches() {
        let mut dialog = MarkEntryDialog::for_text("契約");
        dialog.main_reading = "けいやく".to_string();
        dialog.sub = "解除".to_string();
        dialog.bold = true;
        assert_eq!(
            dialog.instruction().unwrap(),
            r#"XE "契約:解除" \y "けいやく" \b"#
        );
        dialog.reference = EntryReference::CrossReference("→ 合意".to_string());
        let parsed = parse_entry(&dialog.instruction().unwrap(), 4).unwrap();
        assert_eq!(parsed.sub.as_deref(), Some("解除"));
        assert_eq!(parsed.see.as_deref(), Some("→ 合意"));
        assert_eq!(parsed.main_reading.as_deref(), Some("けいやく"));
        assert!(parsed.bold);
    }

    #[test]
    fn japanese_entries_sort_by_reading_under_their_rows() {
        let entries = vec![
            entry("契約", None, Some("けいやく"), 1),
            entry("秘密", None, Some("ひみつ"), 2),
            entry("情報", None, Some("じょうほう"), 3),
            entry("契約", Some("解除"), None, 5),
            entry("API", None, None, 4),
        ];
        let dialog = IndexDialog::default();
        let lines = index_lines(&entries, &dialog, &|paragraph| Some(paragraph), &|_| None);
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            ["A", "API", "か", "契約", "解除", "さ", "情報", "は", "秘密"]
        );
        assert_eq!(lines[3].page.as_deref(), Some("1"));
        assert_eq!(lines[4].page.as_deref(), Some("5"));
        assert_eq!(heading_of("ぱん", IndexLanguage::Japanese), "は");
        assert_eq!(heading_of("ゆめ", IndexLanguage::Japanese), "や");
    }

    #[test]
    fn page_ranges_and_cross_references() {
        let mut ranged = entry("期間", None, Some("きかん"), 1);
        ranged.range = Some("Term".to_string());
        let mut see = entry("期限", None, Some("きげん"), 2);
        see.see = Some("→ 期間".to_string());
        let lines = index_lines(
            &[ranged, see],
            &IndexDialog {
                headings: false,
                ..IndexDialog::default()
            },
            &|paragraph| Some(paragraph),
            &|_| Some((3, 5)),
        );
        assert_eq!(lines[0].page.as_deref(), Some("3–5"));
        assert_eq!(lines[1].page.as_deref(), Some("→ 期間"));
        let run_in = index_lines(
            &[
                entry("契約", None, None, 1),
                entry("契約", Some("解除"), None, 2),
            ],
            &IndexDialog {
                layout: IndexLayout::RunIn,
                headings: false,
                ..IndexDialog::default()
            },
            &|paragraph| Some(paragraph),
            &|_| None,
        );
        assert_eq!(run_in[0].text, "契約, 1: 解除, 2");
    }

    #[test]
    fn the_index_instruction_round_trips() {
        let dialog = IndexDialog::default();
        assert_eq!(
            dialog.instruction(),
            "INDEX \\h \"あ\" \\c \"2\" \\y \\z \"1041\""
        );
        let back = dialog_from_instruction(&dialog.instruction());
        assert_eq!(back.columns, 2);
        assert_eq!(back.language, IndexLanguage::Japanese);
        assert!(back.headings);
    }

    #[test]
    fn authorities_list_by_category_with_passim() {
        let dialog = MarkCitationDialog {
            long: "最判平成元年1月1日".to_string(),
            short: "平成元年判決".to_string(),
            category: 1,
        };
        let marked = parse_authority(&dialog.instruction().unwrap(), 1).unwrap();
        assert_eq!(marked.short.as_deref(), Some("平成元年判決"));
        let mut entries = vec![marked];
        for paragraph in 2..7 {
            entries.push(AuthorityEntry {
                long: None,
                short: Some("平成元年判決".to_string()),
                category: 1,
                paragraph,
            });
        }
        let lines = authorities_lines(&entries, 1, &AuthoritiesDialog::default(), &|p| Some(p));
        assert_eq!(lines[0].text, "判例");
        assert_eq!(lines[1].text, "最判平成元年1月1日");
        assert_eq!(lines[1].page.as_deref(), Some("前掲"));
        let lines = authorities_lines(
            &entries,
            1,
            &AuthoritiesDialog {
                passim: false,
                ..AuthoritiesDialog::default()
            },
            &|p| Some(p),
        );
        assert_eq!(lines[1].page.as_deref(), Some("1, 2, 3, 4, 5, 6"));
    }
}
