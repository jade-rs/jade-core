//! 引用文献と文献目録: the document's sources and the master list kept
//! beside the app (資料文献の管理), a citation at the caret (`CITATION`),
//! the style the citations and the bibliography are set in (スタイル: APA,
//! MLA, Chicago, IEEE, …), and the bibliography itself (`BIBLIOGRAPHY`),
//! inserted and set again whenever the sources or the style change.

use crate::word_document::FieldBlockLine;
use jade_selection::SelectionRange;

use super::edit_document;
use crate::word::{Position, WordSession};

pub use crate::word_document::CitationSourceInfo as Source;

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
                let name = if initials.is_empty() {
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
                let name = if initials.is_empty() {
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
                "[{number}] {}{last}, {title}, {}{year}.",
                if initials.is_empty() {
                    String::new()
                } else {
                    format!("{initials} ")
                },
                if publisher.is_empty() {
                    String::new()
                } else {
                    format!("{publisher}, ")
                }
            ),
            BibliographyStyle::Gb7714 => format!(
                "[{number}] {last}{}. {title}[M]. {}{year}.",
                if initials.is_empty() {
                    String::new()
                } else {
                    format!(" {}", initials.replace('.', ""))
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

/// 資料文献の種類 Word lists, as `b:SourceType` values.
pub const SOURCE_TYPES: [&str; 6] = [
    "Book",
    "JournalArticle",
    "ArticleInAPeriodical",
    "ConferenceProceedings",
    "Report",
    "InternetSite",
];

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
    fn text_of(source: &str, tag: &str) -> Option<String> {
        let open = format!("<b:{tag}>");
        let close = format!("</b:{tag}>");
        let start = source.find(&open)? + open.len();
        let end = source[start..].find(&close)? + start;
        let value = source[start..end]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&");
        Some(value)
    }
    let mut sources = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<b:Source>") {
        let Some(end) = rest[start..].find("</b:Source>") else {
            break;
        };
        let body = &rest[start..start + end];
        rest = &rest[start + end..];
        let Some(tag) = text_of(body, "Tag") else {
            continue;
        };
        let author = text_of(body, "Corporate").unwrap_or_else(|| {
            let last = text_of(body, "Last").unwrap_or_default();
            match text_of(body, "First") {
                Some(first) => format!("{last}, {first}"),
                None => last,
            }
        });
        sources.push(Source {
            tag,
            source_type: text_of(body, "SourceType").unwrap_or_else(|| "Book".to_string()),
            author,
            title: text_of(body, "Title").unwrap_or_default(),
            year: text_of(body, "Year").unwrap_or_default(),
            publisher: text_of(body, "Publisher"),
            url: text_of(body, "URL"),
        });
    }
    sources
}

/// The tag a `CITATION` instruction cites.
fn cited_tag(instruction: &str) -> Option<String> {
    instruction.split_whitespace().nth(1).map(str::to_string)
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
            if let Some(tag) = cited_tag(&field.instruction) {
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
        let style = self.bibliography_style();
        edit_document(self, "edit_source", true, move |document, _| {
            document.replace_citation_source(&tag, source.clone())?;
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
                let caret =
                    document.insert_inline_field(at, &format!("CITATION {tag} \\l 1041"), "")?;
                *selection = Some(caret);
                restyle(document, style)?;
                // The citation's text came after the caret was placed.
                if let Some(field) = document
                    .inline_fields("CITATION")
                    .into_iter()
                    .find(|field| {
                        field.paragraph == at.paragraph
                            && cited_tag(&field.instruction).as_deref() == Some(tag.as_str())
                    })
                {
                    let end: usize = document
                        .paragraph(field.paragraph)
                        .map(|paragraph| {
                            paragraph.runs[..=field.run]
                                .iter()
                                .map(|run| run.text.len())
                                .sum()
                        })
                        .unwrap_or(0);
                    *selection = Some(SelectionRange::collapsed(
                        document.model_to_rendered_text_position(
                            jade_selection::TextPosition::new(field.paragraph, end),
                        ),
                    ));
                }
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
            document.set_bibliography_style(style.name(), style.stylesheet())?;
            restyle(document, style)
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

/// The bibliography's entries: sorted by author for author-date styles, in
/// citation order for numbered ones.
fn bibliography_lines(
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
            text: style.entry(source, index + 1),
            indent: if style.numbered() { 0.0 } else { 36.0 },
            hanging: if style.numbered() { 0.0 } else { 36.0 },
            ..FieldBlockLine::default()
        })
        .collect()
}

/// Every source, cited ones first in the order first cited.
fn citation_order(
    document: &crate::word_document::WordDocument,
    sources: &[Source],
) -> Vec<String> {
    let mut order = Vec::new();
    for field in document.inline_fields("CITATION") {
        if let Some(tag) = cited_tag(&field.instruction) {
            if !order.contains(&tag) {
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

/// Sets every citation and bibliography in `style`.
fn restyle(
    document: &mut crate::word_document::WordDocument,
    style: BibliographyStyle,
) -> Result<(), String> {
    let sources = document.citation_sources();
    let order = citation_order(document, &sources);
    for field in document.inline_fields("CITATION") {
        let Some(tag) = cited_tag(&field.instruction) else {
            continue;
        };
        let Some(source) = sources.iter().find(|source| source.tag == tag) else {
            continue;
        };
        let number = order.iter().position(|cited| *cited == tag).unwrap_or(0) + 1;
        let text = style.citation(source, number);
        if text != field.text {
            document.set_inline_field(field.paragraph, field.run, None, &text)?;
        }
    }
    let lines = bibliography_lines(document, style);
    for block in document.field_blocks().into_iter().rev() {
        if block.name() != "BIBLIOGRAPHY" {
            continue;
        }
        document.replace_field_block(&block, &block.instruction, &lines)?;
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
            BibliographyStyle::from_name("mla"),
            Some(BibliographyStyle::Mla)
        );
    }

    #[test]
    fn the_master_list_round_trips() {
        let sources = vec![source()];
        let xml = master_list_xml(&sources);
        assert_eq!(parse_master_list(&xml), sources);
        assert_eq!(make_tag(&sources[0], &["Smi20".to_string()]), "Smi201");
    }
}
