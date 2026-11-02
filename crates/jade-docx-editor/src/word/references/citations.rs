//! 引用文献と文献目録: the document's sources and the master list kept
//! beside the app (資料文献の管理), a citation at the caret (`CITATION`),
//! the style the citations and the bibliography are set in (スタイル: APA,
//! MLA, Chicago, IEEE, …), and the bibliography itself (`BIBLIOGRAPHY`),
//! inserted and set again whenever the sources or the style change.

use crate::word::fields::{build_code, parse_code, quote, unparse, unquote};
use crate::word_document::{FieldBlock, FieldBlockLine, WordDocument};
use jade_selection::SelectionRange;

use super::edit_document;
use crate::word::{Position, WordSession};

pub use crate::word_document::{
    CitationSourceInfo as Source, CITATION_SOURCE_TYPES as SOURCE_TYPES,
};

/// The bibliography styles Word for Mac offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BibliographyStyle {
    Apa,
    Chicago,
    Gb7714,
    Harvard,
    Ieee,
    Iso690Author,
    Iso690Numeric,
    Mla,
    Sist02,
    Turabian,
}

impl BibliographyStyle {
    pub const ALL: [BibliographyStyle; 10] = [
        BibliographyStyle::Apa,
        BibliographyStyle::Chicago,
        BibliographyStyle::Gb7714,
        BibliographyStyle::Harvard,
        BibliographyStyle::Ieee,
        BibliographyStyle::Iso690Author,
        BibliographyStyle::Iso690Numeric,
        BibliographyStyle::Mla,
        BibliographyStyle::Sist02,
        BibliographyStyle::Turabian,
    ];

    /// The name the list shows, which is the catalog's `StyleName`.
    pub fn name(self) -> &'static str {
        match self {
            BibliographyStyle::Apa => "APA",
            BibliographyStyle::Chicago => "Chicago",
            BibliographyStyle::Gb7714 => "GB7714",
            BibliographyStyle::Harvard => "Harvard - Anglia",
            BibliographyStyle::Ieee => "IEEE",
            BibliographyStyle::Iso690Author => "ISO 690 - First Element and Date",
            BibliographyStyle::Iso690Numeric => "ISO 690 - Numerical Reference",
            BibliographyStyle::Mla => "MLA",
            BibliographyStyle::Sist02 => "SIST02",
            BibliographyStyle::Turabian => "Turabian",
        }
    }

    /// The stylesheet Word applies (`SelectedStyle`).
    pub fn stylesheet(self) -> &'static str {
        match self {
            BibliographyStyle::Apa => "\\APASixthEditionOfficeOnline.xsl",
            BibliographyStyle::Chicago => "\\ChicagoFifteenthEditionOfficeOnline.xsl",
            BibliographyStyle::Gb7714 => "\\GB.XSL",
            BibliographyStyle::Harvard => "\\HarvardAnglia2008OfficeOnline.xsl",
            BibliographyStyle::Ieee => "\\IEEE2006OfficeOnline.xsl",
            BibliographyStyle::Iso690Author => "\\ISO690.XSL",
            BibliographyStyle::Iso690Numeric => "\\ISO690Nmerical.XSL",
            BibliographyStyle::Mla => "\\MLASeventhEditionOfficeOnline.xsl",
            BibliographyStyle::Sist02 => "\\SIST02.XSL",
            BibliographyStyle::Turabian => "\\TurabianSixthEdition.xsl",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Whether citations are numbered in the order they are first cited,
    /// rather than by author and year.
    pub fn numbered(self) -> bool {
        matches!(
            self,
            BibliographyStyle::Gb7714
                | BibliographyStyle::Ieee
                | BibliographyStyle::Iso690Numeric
                | BibliographyStyle::Sist02
        )
    }

    /// A citation in the style; `number` is its place in citation order.
    pub fn citation(self, source: &Source, number: usize) -> String {
        let last = family_name(&source.author);
        match self {
            BibliographyStyle::Apa => format!("({last}, {})", source.year),
            BibliographyStyle::Mla => format!("({last})"),
            BibliographyStyle::Chicago | BibliographyStyle::Turabian => {
                format!("({last} {})", source.year)
            }
            BibliographyStyle::Harvard => format!("({last}, {})", source.year),
            BibliographyStyle::Iso690Author => {
                format!("({}, {})", last.to_uppercase(), source.year)
            }
            BibliographyStyle::Iso690Numeric => format!("({number})"),
            BibliographyStyle::Ieee | BibliographyStyle::Gb7714 | BibliographyStyle::Sist02 => {
                format!("[{number}]")
            }
        }
    }

    /// A bibliography entry in the style.
    pub fn entry(self, source: &Source, number: usize) -> String {
        let author = source.author.trim();
        let last = family_name(author);
        let given = given_names(author);
        let initials = initials(&given);
        let title = source.title.trim();
        let year = source.year.trim();
        let publisher = source.publisher.as_deref().unwrap_or("").trim();
        let with_publisher = |text: String, separator: &str| {
            if publisher.is_empty() {
                text
            } else {
                format!("{text}{separator}{publisher}")
            }
        };
        match self {
            BibliographyStyle::Apa => {
                let name = if initials.is_empty() && !given.is_empty() {
                    author.to_string()
                } else if initials.is_empty() {
                    last.clone()
                } else {
                    format!("{last}, {initials}")
                };
                with_publisher(format!("{name} ({year}). {title}."), " ")
                    + if publisher.is_empty() { "" } else { "." }
            }
            BibliographyStyle::Mla | BibliographyStyle::Turabian => {
                format!(
                    "{}. {title}. {}{year}.",
                    author.trim_end_matches('.'),
                    if publisher.is_empty() {
                        String::new()
                    } else {
                        format!("{publisher}, ")
                    }
                )
            }
            BibliographyStyle::Chicago => {
                with_publisher(
                    format!("{}. {year}. {title}.", author.trim_end_matches('.')),
                    " ",
                ) + if publisher.is_empty() { "" } else { "." }
            }
            BibliographyStyle::Harvard => {
                let name = if initials.is_empty() && !given.is_empty() {
                    author.to_string()
                } else if initials.is_empty() {
                    last.clone()
                } else {
                    format!("{last}, {initials}")
                };
                with_publisher(format!("{name}, {year}. {title}."), " ")
                    + if publisher.is_empty() { "" } else { "." }
            }
            BibliographyStyle::Iso690Author => {
                with_publisher(
                    format!(
                        "{}{}. {year}. {title}.",
                        last.to_uppercase(),
                        if given.is_empty() {
                            String::new()
                        } else {
                            format!(", {given}")
                        }
                    ),
                    " ",
                ) + if publisher.is_empty() { "" } else { "." }
            }
            BibliographyStyle::Iso690Numeric => format!(
                "{number}. {}{}. {title}. {}{year}.",
                last.to_uppercase(),
                if given.is_empty() {
                    String::new()
                } else {
                    format!(", {given}")
                },
                if publisher.is_empty() {
                    String::new()
                } else {
                    format!("{publisher}, ")
                }
            ),
            BibliographyStyle::Ieee => format!(
                "[{number}] {}, {title}, {}{year}.",
                if initials.is_empty() && !given.is_empty() {
                    author.to_string()
                } else if initials.is_empty() {
                    last.clone()
                } else {
                    format!("{initials} {last}")
                },
                if publisher.is_empty() {
                    String::new()
                } else {
                    format!("{publisher}, ")
                }
            ),
            BibliographyStyle::Gb7714 => format!(
                "[{number}] {}. {title}[M]. {}{year}.",
                if initials.is_empty() && !given.is_empty() {
                    author.to_string()
                } else if initials.is_empty() {
                    last.clone()
                } else {
                    format!("{last} {}", initials.replace('.', ""))
                },
                if publisher.is_empty() {
                    String::new()
                } else {
                    format!("{publisher}, ")
                }
            ),
            BibliographyStyle::Sist02 => format!(
                "{number}) {author}. {title}. {}{year}.",
                if publisher.is_empty() {
                    String::new()
                } else {
                    format!("{publisher}, ")
                }
            ),
        }
    }
}

/// The family name: before the comma (`Smith, John`), else the first word
/// of a Japanese name (`山田 太郎`) or the last of a Western one.
fn family_name(author: &str) -> String {
    let author = author.trim();
    if let Some((family, _)) = author.split_once(',') {
        return family.trim().to_string();
    }
    let words: Vec<&str> = author.split_whitespace().collect();
    match words.as_slice() {
        [] => String::new(),
        [only] => only.to_string(),
        [first, ..] if !first.is_ascii() => first.to_string(),
        [.., last] => last.to_string(),
    }
}

fn given_names(author: &str) -> String {
    let author = author.trim();
    if let Some((_, given)) = author.split_once(',') {
        return given.trim().to_string();
    }
    let words: Vec<&str> = author.split_whitespace().collect();
    match words.as_slice() {
        [] | [_] => String::new(),
        [first, rest @ ..] if !first.is_ascii() => rest.join(" "),
        [rest @ .., _] => rest.join(" "),
    }
}

fn initials(given: &str) -> String {
    if !given.is_ascii() {
        return String::new();
    }
    given
        .split_whitespace()
        .filter_map(|name| name.chars().next())
        .map(|initial| format!("{initial}."))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The tag a source is cited by: the family name's first three letters and
/// the year's last two (`Smi20`), made unique against `taken`.
pub fn make_tag(source: &Source, taken: &[String]) -> String {
    let family: String = family_name(&source.author).chars().take(3).collect();
    let year: String = source
        .year
        .chars()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let base = if family.is_empty() {
        "Src".to_string()
    } else {
        family
    } + &year;
    let mut tag = base.clone();
    let mut suffix = 1;
    while taken.contains(&tag) {
        tag = format!("{base}{suffix}");
        suffix += 1;
    }
    tag
}

/// The master list kept beside the app, as Word keeps `Sources.xml`: the
/// same `b:Sources` markup the document's catalog uses.
pub fn master_list_xml(sources: &[Source]) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<b:Sources xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\" xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\">",
    );
    for source in sources {
        xml.push_str("<b:Source>");
        xml.push_str(&format!("<b:Tag>{}</b:Tag>", escape(&source.tag)));
        xml.push_str(&format!(
            "<b:SourceType>{}</b:SourceType>",
            escape(&source.source_type)
        ));
        xml.push_str(&format!(
            "<b:Author><b:Author><b:Corporate>{}</b:Corporate></b:Author></b:Author>",
            escape(&source.author)
        ));
        xml.push_str(&format!("<b:Title>{}</b:Title>", escape(&source.title)));
        xml.push_str(&format!("<b:Year>{}</b:Year>", escape(&source.year)));
        if let Some(publisher) = &source.publisher {
            xml.push_str(&format!("<b:Publisher>{}</b:Publisher>", escape(publisher)));
        }
        if let Some(url) = &source.url {
            xml.push_str(&format!("<b:URL>{}</b:URL>", escape(url)));
        }
        xml.push_str("</b:Source>");
    }
    xml.push_str("</b:Sources>");
    xml
}

/// Reads a master list written by [`master_list_xml`] — or by Word.
pub fn parse_master_list(xml: &str) -> Vec<Source> {
    WordDocument::parse_citation_sources_xml(xml, "Book")
}

/// Tags cited by the primary operand and Word's additional-source switches.
fn instruction_tags(instruction: &str) -> Vec<String> {
    let parsed = parse_code(instruction);
    parsed
        .operands
        .first()
        .into_iter()
        .chain(parsed.switches.iter().filter_map(|(switch, value)| {
            switch
                .eq_ignore_ascii_case("\\m")
                .then_some(value.as_ref())
                .flatten()
        }))
        .map(|token| unquote(token))
        .collect()
}

/// Citations and bibliographies over every session.
pub trait WordCitations: WordSession {
    /// 現在のリスト: the document's sources.
    fn sources(&self) -> Vec<Source> {
        self.word_document().citation_sources()
    }

    /// The style the citations are set in: the catalog's, APA by default.
    fn bibliography_style(&self) -> BibliographyStyle {
        self.word_document()
            .bibliography_style()
            .and_then(|name| BibliographyStyle::from_name(&name))
            .unwrap_or(BibliographyStyle::Apa)
    }

    /// The tags cited, each once, in the order first cited.
    fn cited_tags(&self) -> Vec<String> {
        let mut tags = Vec::new();
        for field in self.word_document().inline_fields("CITATION") {
            for tag in instruction_tags(&field.instruction) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        tags
    }

    /// 資料文献の追加 (or コピー from the master list), one undo step; a
    /// source with no tag gets one.
    fn add_source(&mut self, mut source: Source) -> Result<String, String> {
        source.tag = source.tag.trim().to_string();
        let taken: Vec<String> = self
            .sources()
            .into_iter()
            .map(|source| source.tag)
            .collect();
        if source.tag.trim().is_empty() {
            source.tag = make_tag(&source, &taken);
        } else if taken.contains(&source.tag) {
            return Err(format!("タグ「{}」の資料文献はすでにあります", source.tag));
        }
        let tag = source.tag.clone();
        edit_document(self, "add_source", true, move |document, _| {
            document.add_citation_source_entry(source.clone())
        })?;
        Ok(tag)
    }

    /// 編集: a source changed, its citations and the bibliography set
    /// again, one undo step.
    fn edit_source(&mut self, tag: &str, source: Source) -> Result<(), String> {
        let tag = tag.to_string();
        let new_tag = source.tag.trim().to_string();
        let style = self.bibliography_style();
        edit_document(self, "edit_source", true, move |document, _| {
            document.replace_citation_source(&tag, source.clone())?;
            if new_tag != tag {
                for field in document.inline_fields("CITATION") {
                    let mut parsed = parse_code(&field.instruction);
                    let mut changed = false;
                    for token in parsed.operands.first_mut().into_iter().chain(
                        parsed.switches.iter_mut().filter_map(|(switch, value)| {
                            if switch.eq_ignore_ascii_case("\\m") {
                                value.as_mut()
                            } else {
                                None
                            }
                        }),
                    ) {
                        if unquote(token) == tag {
                            *token = quote(&new_tag, true);
                            changed = true;
                        }
                    }
                    if changed {
                        document.set_inline_field(
                            field.paragraph,
                            field.run,
                            Some(&unparse(&parsed)),
                            &field.text,
                        )?;
                    }
                }
            }
            restyle(document, style)
        })
    }

    /// 削除: Word keeps a cited source.
    fn delete_source(&mut self, tag: &str) -> Result<(), String> {
        if self.cited_tags().iter().any(|cited| cited == tag) {
            return Err("引用されている資料文献は削除できません".to_string());
        }
        let tag = tag.to_string();
        edit_document(self, "delete_source", true, move |document, _| {
            document.remove_citation_source(&tag)
        })
    }

    /// 引用文献の挿入: a `CITATION` field at the caret, set in the style.
    fn insert_citation(&mut self, tag: &str) -> Result<Position, String> {
        let at = match self.selection_range() {
            Some(range) => range.normalized().1,
            None => jade_selection::TextPosition::new(0, 0),
        };
        let tag = tag.to_string();
        let style = self.bibliography_style();
        if !self.sources().iter().any(|source| source.tag == tag) {
            return Err(format!("資料文献「{tag}」はありません"));
        }
        edit_document(
            self,
            "insert_citation",
            false,
            move |document, selection| {
                *selection = Some(insert_citation_field(document, at, &tag, style)?);
                Ok(())
            },
        )?;
        Ok(self
            .selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((at.paragraph, at.offset)))
    }

    /// スタイル: every citation and bibliography set in another style, one
    /// undo step.
    fn set_bibliography_style(&mut self, style: BibliographyStyle) -> Result<(), String> {
        if self.sources().is_empty() {
            return Err("資料文献がありません".to_string());
        }
        edit_document(self, "bibliography_style", true, move |document, _| {
            set_style(document, style.name(), style.stylesheet())
        })
    }

    /// 文献目録 / 参考文献: a `BIBLIOGRAPHY` field after the caret's
    /// paragraph, under a heading when given, one undo step.
    fn insert_bibliography(&mut self, heading: Option<&str>) -> Result<(), String> {
        let sources = self.sources();
        if sources.is_empty() {
            return Err("資料文献がありません".to_string());
        }
        let style = self.bibliography_style();
        let after = self
            .selection_range()
            .map(|range| range.normalized().1.paragraph)
            .unwrap_or(0);
        let heading = heading.map(str::to_string);
        edit_document(
            self,
            "insert_bibliography",
            false,
            move |document, selection| {
                let mut lines = Vec::new();
                if let Some(heading) = &heading {
                    lines.push(FieldBlockLine {
                        text: heading.clone(),
                        style: Some("Heading1".to_string()),
                        ..FieldBlockLine::default()
                    });
                }
                lines.extend(bibliography_lines(document, style));
                *selection = Some(document.insert_field_block(after, "BIBLIOGRAPHY", &lines)?);
                Ok(())
            },
        )
    }

    /// 文献目録の更新: the bibliographies set again from the sources.
    fn update_bibliography(&mut self) -> Result<(), String> {
        let style = self.bibliography_style();
        edit_document(self, "update_bibliography", true, move |document, _| {
            restyle(document, style)
        })
    }
}

impl<T: WordSession + ?Sized> WordCitations for T {}

/// Shared authoring path for native sessions and the browser bridge.
pub(crate) fn insert_citation_field(
    document: &mut WordDocument,
    at: jade_selection::TextPosition,
    tag: &str,
    style: BibliographyStyle,
) -> Result<SelectionRange, String> {
    let instruction = build_code(
        "CITATION",
        &[(tag.to_string(), true)],
        &[("\\l".into(), Some("1041".into()))],
        false,
    );
    let (caret, location) = document.insert_inline_field_with_location(at, &instruction, "")?;
    let ordinal = document
        .inline_fields("CITATION")
        .iter()
        .position(|field| (field.paragraph, field.run) == location)
        .ok_or("the inserted citation is unavailable")?;
    let mut caret = caret;
    restyle(document, style)?;
    // The citation's text came after the caret was placed.
    if let Some(field) = document.inline_fields("CITATION").into_iter().nth(ordinal) {
        let end: usize = document
            .paragraph(field.paragraph)
            .map(|paragraph| {
                paragraph.runs[..=field.run]
                    .iter()
                    .map(|run| run.text.len())
                    .sum()
            })
            .unwrap_or(0);
        caret = SelectionRange::collapsed(document.model_to_rendered_text_position(
            jade_selection::TextPosition::new(field.paragraph, end),
        ));
    }
    Ok(caret)
}

/// The bibliography's entries: sorted by author for author-date styles, in
/// citation order for numbered ones.
pub(crate) fn bibliography_lines(
    document: &crate::word_document::WordDocument,
    style: BibliographyStyle,
) -> Vec<FieldBlockLine> {
    let sources = document.citation_sources();
    let order = citation_order(document, &sources);
    let mut entries: Vec<(usize, Source)> = sources
        .into_iter()
        .map(|source| {
            let number = order
                .iter()
                .position(|tag| *tag == source.tag)
                .unwrap_or(usize::MAX);
            (number, source)
        })
        .collect();
    if style.numbered() {
        entries.sort_by_key(|(number, _)| *number);
    } else {
        entries.sort_by(|(_, a), (_, b)| {
            family_name(&a.author)
                .to_lowercase()
                .cmp(&family_name(&b.author).to_lowercase())
                .then_with(|| a.year.cmp(&b.year))
        });
    }
    entries
        .iter()
        .enumerate()
        .map(|(index, (_, source))| FieldBlockLine {
            text: {
                let mut text = style.entry(source, index + 1);
                if let Some(url) = source
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                {
                    text.push(' ');
                    text.push_str(url);
                }
                text
            },
            indent: if style.numbered() { 0.0 } else { 36.0 },
            hanging: if style.numbered() { 0.0 } else { 36.0 },
            ..FieldBlockLine::default()
        })
        .collect()
}

/// Every source, cited ones first in the order first cited.
pub(crate) fn citation_order(
    document: &crate::word_document::WordDocument,
    sources: &[Source],
) -> Vec<String> {
    let mut order = Vec::new();
    for field in document.inline_fields("CITATION") {
        for tag in instruction_tags(&field.instruction) {
            if sources.iter().any(|source| source.tag == tag) && !order.contains(&tag) {
                order.push(tag);
            }
        }
    }
    for source in sources {
        if !order.contains(&source.tag) {
            order.push(source.tag.clone());
        }
    }
    order
}

/// Only regenerate forms the renderer supports; preserve Word's cached
/// combined citations and per-source formatting switches.
pub(crate) fn render_citation(
    instruction: &str,
    sources: &[Source],
    order: &[String],
    style: BibliographyStyle,
) -> Option<String> {
    let parsed = parse_code(instruction);
    if parsed.operands.len() != 1 || !supported_switches(&parsed) {
        return None;
    }
    let tag = unquote(&parsed.operands[0]);
    let source = sources.iter().find(|source| source.tag == tag)?;
    let number = order.iter().position(|cited| *cited == tag)? + 1;
    Some(style.citation(source, number))
}

fn supported_switches(parsed: &crate::word::fields::ParsedCode) -> bool {
    parsed.switches.iter().all(|(switch, value)| {
        switch.eq_ignore_ascii_case("\\l") && value.is_some()
            || switch == "\\*"
                && value
                    .as_deref()
                    .is_some_and(|value| unquote(value).eq_ignore_ascii_case("MERGEFORMAT"))
    })
}

pub(crate) fn supported_bibliography(instruction: &str) -> bool {
    let parsed = parse_code(instruction);
    parsed.name.eq_ignore_ascii_case("BIBLIOGRAPHY")
        && parsed.operands.is_empty()
        && supported_switches(&parsed)
}

/// Replace the entries while keeping the heading inside the field result.
pub(crate) fn replace_bibliography(
    document: &mut WordDocument,
    block: &FieldBlock,
    lines: &[FieldBlockLine],
) -> Result<(), String> {
    let heading = document
        .paragraph(block.start)
        .filter(|paragraph| {
            paragraph
                .properties
                .as_ref()
                .and_then(|p| p.style.as_deref())
                == Some("Heading1")
        })
        .map(|paragraph| FieldBlockLine {
            text: paragraph
                .runs
                .iter()
                .map(|run| {
                    if run.text.is_empty() {
                        run.field
                            .as_ref()
                            .and_then(|field| {
                                field
                                    .resolved_value
                                    .as_deref()
                                    .or(field.cached_text.as_deref())
                            })
                            .unwrap_or("")
                    } else {
                        run.text.as_str()
                    }
                })
                .collect(),
            style: Some("Heading1".into()),
            ..FieldBlockLine::default()
        });
    let block_lines: Vec<_> = heading.into_iter().chain(lines.iter().cloned()).collect();
    document.replace_field_block(block, &block.instruction, &block_lines)?;
    Ok(())
}

/// Set metadata and supported field caches in the same undo transaction.
pub(crate) fn set_style(
    document: &mut WordDocument,
    name: &str,
    stylesheet: &str,
) -> Result<(), String> {
    document.set_bibliography_style(name, stylesheet)?;
    if let Some(style) = BibliographyStyle::from_name(name) {
        restyle(document, style)?;
    }
    Ok(())
}

/// Sets every citation and bibliography in `style`.
fn restyle(
    document: &mut crate::word_document::WordDocument,
    style: BibliographyStyle,
) -> Result<(), String> {
    let sources = document.citation_sources();
    let order = citation_order(document, &sources);
    for field in document.inline_fields("CITATION") {
        let Some(text) = render_citation(&field.instruction, &sources, &order, style) else {
            continue;
        };
        if text != field.text {
            document.set_inline_field(field.paragraph, field.run, None, &text)?;
        }
    }
    let lines = bibliography_lines(document, style);
    for block in document.field_blocks().into_iter().rev() {
        if block.name() != "BIBLIOGRAPHY" || !supported_bibliography(&block.instruction) {
            continue;
        }
        replace_bibliography(document, &block, &lines)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{make_tag, master_list_xml, parse_master_list, BibliographyStyle, Source};

    fn source() -> Source {
        Source {
            tag: "Smi20".to_string(),
            source_type: "Book".to_string(),
            author: "Smith, John".to_string(),
            title: "On Things".to_string(),
            year: "2020".to_string(),
            publisher: Some("Acme".to_string()),
            url: None,
        }
    }

    #[test]
    fn citations_and_entries_follow_the_style() {
        let source = source();
        assert_eq!(BibliographyStyle::Apa.citation(&source, 1), "(Smith, 2020)");
        assert_eq!(BibliographyStyle::Mla.citation(&source, 1), "(Smith)");
        assert_eq!(BibliographyStyle::Ieee.citation(&source, 3), "[3]");
        assert_eq!(
            BibliographyStyle::Apa.entry(&source, 1),
            "Smith, J. (2020). On Things. Acme."
        );
        assert_eq!(
            BibliographyStyle::Ieee.entry(&source, 2),
            "[2] J. Smith, On Things, Acme, 2020."
        );
        assert_eq!(
            BibliographyStyle::Mla.entry(&source, 1),
            "Smith, John. On Things. Acme, 2020."
        );
        let japanese = Source {
            author: "山田 太郎".to_string(),
            ..source
        };
        assert_eq!(
            BibliographyStyle::Apa.citation(&japanese, 1),
            "(山田, 2020)"
        );
        assert_eq!(
            BibliographyStyle::Apa.entry(&japanese, 1),
            "山田 太郎 (2020). On Things. Acme."
        );
        assert_eq!(
            BibliographyStyle::Harvard.entry(&japanese, 1),
            "山田 太郎, 2020. On Things. Acme."
        );
        assert_eq!(
            BibliographyStyle::Ieee.entry(&japanese, 1),
            "[1] 山田 太郎, On Things, Acme, 2020."
        );
        assert_eq!(
            BibliographyStyle::Gb7714.entry(&japanese, 1),
            "[1] 山田 太郎. On Things[M]. Acme, 2020."
        );
        assert_eq!(
            BibliographyStyle::from_name("mla"),
            Some(BibliographyStyle::Mla)
        );
    }

    #[test]
    fn master_lists_accept_namespace_aliases_and_source_attributes() {
        let sources = vec![source()];
        let xml =
            master_list_xml(&sources).replace("<b:Source>", "<b:Source data-origin=\"Word\">");
        for aliased in [
            xml.clone(),
            xml.replace("b:", "bib:").replace("xmlns:b=", "xmlns:bib="),
            xml.replace("b:", "").replace("xmlns:b=", "xmlns:unused="),
        ] {
            assert_eq!(parse_master_list(&aliased), sources, "{aliased}");
        }
    }

    #[test]
    fn master_lists_read_entities_cdata_and_personal_names() {
        let xml = r#"<bib:Sources xmlns:bib="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
          <bib:Source><bib:Tag>Smi&#50;0</bib:Tag><bib:SourceType>Book</bib:SourceType>
            <bib:Author><bib:Author><bib:NameList><bib:Person>
              <bib:Last>O&apos;Brien</bib:Last><bib:First>Jane&#32;Q</bib:First>
            </bib:Person></bib:NameList></bib:Author></bib:Author>
            <bib:Title><![CDATA[On <Things> & "資料"]]></bib:Title><bib:Year>2020</bib:Year>
            <bib:Publisher>Acme &amp; Co</bib:Publisher>
          </bib:Source>
        </bib:Sources>"#;
        let expected = Source {
            author: "O'Brien, Jane Q".to_string(),
            title: "On <Things> & \"資料\"".to_string(),
            publisher: Some("Acme & Co".to_string()),
            ..source()
        };
        assert_eq!(parse_master_list(xml), [expected]);
    }

    #[test]
    fn master_lists_ignore_foreign_namespaces_and_malformed_xml() {
        let xml = master_list_xml(&[source()]);
        for invalid in [
            xml.replace(
                "http://schemas.openxmlformats.org/officeDocument/2006/bibliography",
                "urn:foreign",
            ),
            xml.replace("<b:Source>", "<b:Source xmlns:b=\"urn:foreign\">"),
            xml.replace("<b:Tag>", "<b:Tag xmlns:b=\"urn:foreign\">"),
            xml.replace("</b:Sources>", ""),
        ] {
            assert!(parse_master_list(&invalid).is_empty(), "{invalid}");
        }
    }

    #[test]
    fn the_master_list_round_trips() {
        let sources = vec![source()];
        let xml = master_list_xml(&sources);
        assert_eq!(parse_master_list(&xml), sources);
        assert_eq!(make_tag(&sources[0], &["Smi20".to_string()]), "Smi201");
    }
}
