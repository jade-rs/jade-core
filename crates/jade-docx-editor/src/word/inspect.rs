//! ファイル › 情報 › ドキュメント検査: what a document carries that its
//! author may not mean to share — comments and tracked changes, who wrote
//! and last saved it, custom XML, hidden text, headers and footers — and
//! すべて削除 for each.
//!
//! Comments, changes and hidden text are removed from the document itself,
//! as undoable edits. Who wrote it and custom XML live in the package: their
//! removal is done to what each later save writes ([`clean_package`]),
//! which also honours a document Word marked 保存時にファイルのプロパティ
//! から個人情報を削除する (`w:removePersonalInformation`).

use jade_docx_headless_api::{
    HeadlessCommand, HeadlessHeaderFooterSnapshot, HeadlessQuery, HeadlessQueryResult,
};

use super::review::WordReview;
use super::WordSession;

/// What the inspector reports for a category.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InspectReport {
    pub comments: usize,
    pub revisions: usize,
    /// Who wrote, last saved and manages it, by property name.
    pub personal: Vec<(String, String)>,
    pub custom_xml_parts: usize,
    /// Runs of hidden text (`w:vanish`).
    pub hidden_runs: usize,
    /// Header and footer lines with text in them.
    pub header_footer_lines: usize,
}

impl InspectReport {
    pub fn is_clean(&self) -> bool {
        self.comments == 0
            && self.revisions == 0
            && self.personal.is_empty()
            && self.custom_xml_parts == 0
            && self.hidden_runs == 0
            && self.header_footer_lines == 0
    }
}

/// The categories すべて削除 works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectCategory {
    CommentsAndRevisions,
    Personal,
    CustomXml,
    HiddenText,
    HeadersFooters,
}

/// What a save is to leave out of the package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackageCleaning {
    pub personal: bool,
    pub custom_xml: bool,
}

/// The properties that name people, as the package metadata reads them.
const PERSONAL: [&str; 4] = ["author", "lastsavedby", "manager", "company"];

/// The hidden runs of paragraph `index`, as byte ranges.
fn hidden_ranges<S: WordSession + ?Sized>(session: &S, index: usize) -> Vec<(usize, usize)> {
    let Some(paragraph) = session.word_document().paragraph(index) else {
        return Vec::new();
    };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0;
    for run in &paragraph.runs {
        let (start, end) = (offset, offset + run.text.len());
        offset = end;
        let hidden = run
            .properties
            .as_ref()
            .is_some_and(|properties| properties.vanish == Some(true));
        if hidden && start < end {
            match ranges.last_mut() {
                Some(last) if last.1 == start => last.1 = end,
                _ => ranges.push((start, end)),
            }
        }
    }
    ranges
}

/// The header and footer stories the document has.
fn header_footer_stories<S: WordSession + ?Sized>(
    session: &S,
) -> Vec<HeadlessHeaderFooterSnapshot> {
    match super::query(session.word_document(), HeadlessQuery::HeaderFooterText) {
        Some(HeadlessQueryResult::HeaderFooterText { stories }) => stories,
        _ => Vec::new(),
    }
}

/// ドキュメント検査 for every session.
pub trait WordInspect: WordSession {
    /// 検査.
    fn inspect(&self) -> InspectReport {
        let document = self.word_document();
        let properties = document.document_properties();
        let personal = PERSONAL
            .iter()
            .filter_map(|name| {
                properties
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .filter(|(_, value)| !value.trim().is_empty())
                    .map(|(key, value)| (key.clone(), value.clone()))
            })
            .collect();
        let custom_xml_parts = document
            .package_part_names()
            .iter()
            .filter(|name| is_custom_xml_item(name))
            .count();
        let hidden_runs = (0..document.paragraph_count())
            .map(|index| hidden_ranges(self, index).len())
            .sum();
        fn count(comments: &[super::review::Comment]) -> usize {
            comments
                .iter()
                .map(|comment| 1 + count(&comment.replies))
                .sum()
        }
        InspectReport {
            comments: count(&self.comments()),
            revisions: self.revisions().len(),
            personal,
            custom_xml_parts,
            hidden_runs,
            header_footer_lines: header_footer_stories(self)
                .iter()
                .flat_map(|story| &story.lines)
                .filter(|line| !line.text.trim().is_empty())
                .count(),
        }
    }

    /// すべて削除 for comments and tracked changes: every comment deleted
    /// and every change accepted, as Word's inspector does.
    fn remove_comments_and_revisions(&mut self) -> Result<(), String> {
        let ids: Vec<String> = self
            .comments()
            .into_iter()
            .map(|comment| comment.id)
            .collect();
        let mut commands: Vec<HeadlessCommand> = ids
            .into_iter()
            .map(|comment_id| HeadlessCommand::DeleteComment { comment_id })
            .collect();
        commands.push(HeadlessCommand::AcceptRevisions {
            kind: None,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids: None,
            revision_targets: None,
        });
        self.apply_commands(commands, false)
    }

    /// すべて削除 for hidden text.
    fn remove_hidden_text(&mut self) -> Result<(), String> {
        let count = self.word_document().paragraph_count();
        let mut commands = Vec::new();
        for index in (0..count).rev() {
            for (start, end) in hidden_ranges(self, index).into_iter().rev() {
                commands.push(HeadlessCommand::ReplaceRange {
                    start_paragraph: index,
                    start_offset: start,
                    end_paragraph: index,
                    end_offset: end,
                    text: String::new(),
                });
            }
        }
        if commands.is_empty() {
            return Ok(());
        }
        self.apply_commands(commands, false)
    }

    /// すべて削除 for headers and footers: every line of them emptied.
    fn remove_headers_footers(&mut self) -> Result<(), String> {
        let commands: Vec<HeadlessCommand> = header_footer_stories(self)
            .into_iter()
            .filter(|story| story.variant == "default")
            .flat_map(|story| {
                let header = story.role == "header";
                story
                    .lines
                    .into_iter()
                    .filter(|line| !line.text.is_empty())
                    .map(move |line| {
                        if header {
                            HeadlessCommand::SetHeaderParagraphText {
                                paragraph: line.paragraph,
                                text: String::new(),
                            }
                        } else {
                            HeadlessCommand::SetFooterParagraphText {
                                paragraph: line.paragraph,
                                text: String::new(),
                            }
                        }
                    })
            })
            .collect();
        if commands.is_empty() {
            return Ok(());
        }
        self.apply_commands(commands, false)
    }

    /// すべて削除 for `category`, where it is an edit of the document.
    /// Personal information and custom XML are left out by the save
    /// ([`clean_package`]).
    fn remove_inspected(&mut self, category: InspectCategory) -> Result<(), String> {
        match category {
            InspectCategory::CommentsAndRevisions => self.remove_comments_and_revisions(),
            InspectCategory::HiddenText => self.remove_hidden_text(),
            InspectCategory::HeadersFooters => self.remove_headers_footers(),
            InspectCategory::Personal | InspectCategory::CustomXml => Ok(()),
        }
    }
}

impl<T: WordSession + ?Sized> WordInspect for T {}

/// The package a save writes, with what `cleaning` asks left out: who
/// wrote and saved it (core and extended properties, and every comment's
/// and change's author, as Word's 作成者), and custom XML with its
/// relationships and content types.
pub fn clean_package(package: &[u8], cleaning: PackageCleaning) -> Result<Vec<u8>, String> {
    let mut opc = jade_opc::OpcPackage::open(package).map_err(|error| error.to_string())?;
    let marked = opc
        .part_str("word/settings.xml")
        .map_err(|error| error.to_string())?
        .is_some_and(|settings| {
            settings.contains("<w:removePersonalInformation/>")
                || settings.contains("<w:removePersonalInformation ")
                    && !settings.contains("removePersonalInformation w:val=\"0\"")
                    && !settings.contains("removePersonalInformation w:val=\"false\"")
        });
    if !cleaning.personal && !cleaning.custom_xml && !marked {
        return Ok(package.to_vec());
    }
    if cleaning.personal || marked {
        let blank = |xml: String, tags: &[&str]| -> String {
            let mut xml = xml;
            for tag in tags {
                let open = format!("<{tag}>");
                let close = format!("</{tag}>");
                while let (Some(start), Some(end)) = (xml.find(&open), xml.find(&close)) {
                    if end < start {
                        break;
                    }
                    xml.replace_range(start..end + close.len(), &format!("<{tag}/>"));
                }
            }
            xml
        };
        if let Some(core) = opc
            .part_str("docProps/core.xml")
            .map_err(|error| error.to_string())?
        {
            let core = blank(core, &["dc:creator", "cp:lastModifiedBy"]);
            opc.replace("docProps/core.xml", core.into_bytes());
        }
        if let Some(app) = opc
            .part_str("docProps/app.xml")
            .map_err(|error| error.to_string())?
        {
            let app = blank(app, &["Company", "Manager"]);
            opc.replace("docProps/app.xml", app.into_bytes());
        }
        for part in [
            "word/document.xml",
            "word/comments.xml",
            "word/commentsExtended.xml",
            "word/people.xml",
        ] {
            if let Some(xml) = opc.part_str(part).map_err(|error| error.to_string())? {
                opc.replace(part, anonymise_authors(&xml).into_bytes());
            }
        }
    }
    if cleaning.custom_xml {
        let names: Vec<String> = opc
            .part_names()
            .iter()
            .filter(|name| name.starts_with("customXml/"))
            .cloned()
            .collect();
        for name in &names {
            opc.remove(name);
        }
        if let Some(rels) = opc
            .part_str("word/_rels/document.xml.rels")
            .map_err(|error| error.to_string())?
        {
            let kept: String = rels
                .split("<Relationship ")
                .enumerate()
                .filter(|(index, chunk)| *index == 0 || !chunk.contains("customXml/"))
                .map(|(index, chunk)| {
                    if index == 0 {
                        chunk.to_string()
                    } else {
                        format!("<Relationship {chunk}")
                    }
                })
                .collect();
            opc.replace("word/_rels/document.xml.rels", kept.into_bytes());
        }
        if let Some(types) = opc
            .part_str("[Content_Types].xml")
            .map_err(|error| error.to_string())?
        {
            let kept: String = types
                .split("<Override ")
                .enumerate()
                .filter(|(index, chunk)| *index == 0 || !chunk.contains("/customXml/"))
                .map(|(index, chunk)| {
                    if index == 0 {
                        chunk.to_string()
                    } else {
                        format!("<Override {chunk}")
                    }
                })
                .collect();
            opc.replace("[Content_Types].xml", kept.into_bytes());
        }
    }
    opc.write().map_err(|error| error.to_string())
}

/// A custom XML data part (not its properties part).
fn is_custom_xml_item(name: &str) -> bool {
    name.starts_with("customXml/item") && name.ends_with(".xml") && !name.contains("Props")
}

/// Every `author="…"` (`w:author`, `w15:author`) made Word's anonymous
/// 作成者, and every `w:initials` its initial.
fn anonymise_authors(xml: &str) -> String {
    let mut out = xml.to_string();
    for (needle, replacement) in [("author=\"", "Author"), ("w:initials=\"", "A")] {
        let mut result = String::with_capacity(out.len());
        let mut tail = out.as_str();
        while let Some(at) = tail.find(needle) {
            let value_start = at + needle.len();
            let Some(value_end) = tail[value_start..].find('"') else {
                break;
            };
            result.push_str(&tail[..value_start]);
            result.push_str(replacement);
            tail = &tail[value_start + value_end..];
        }
        result.push_str(tail);
        out = result;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authors_become_anonymous() {
        let xml = r#"<w:ins w:id="1" w:author="Ann Lee" w:date="x"/><w:comment w:author="Bob" w:initials="BB"/>"#;
        assert_eq!(
            anonymise_authors(xml),
            r#"<w:ins w:id="1" w:author="Author" w:date="x"/><w:comment w:author="Author" w:initials="A"/>"#
        );
    }
}
