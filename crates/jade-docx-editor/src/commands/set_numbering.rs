//! Paragraph numbering command implementation.
//!
//! Sets or removes the numbering (bullet/numbered list) properties on a range
//! of paragraphs.  When `num_id` is `None` the numbering is removed; when
//! `Some(id)` the paragraph is associated with the given numbering definition at
//! the specified level.

use super::paragraph_property_sync::sync_paragraph_numbering_xml;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Numbering, ParagraphProperties};
use jade_docx_layout::selection::SelectionRange;

/// `w:numId` value that turns numbering off.
const NUMBERING_OFF: &str = "0";

/// Whether the paragraph style (or a style it is based on) numbers its
/// paragraphs.
fn style_supplies_numbering(document: &crate::word_document::WordDocument, style: &str) -> bool {
    document
        .styles_part
        .as_ref()
        .and_then(|styles| styles.resolve_numbering(style))
        .is_some_and(|numbering| numbering.list_id().is_some())
}

/// Sets paragraph numbering for a contiguous paragraph range.
pub struct SetNumberingCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    num_id: Option<String>,
    level: Option<u32>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetNumberingCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        num_id: Option<String>,
        level: Option<u32>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            num_id,
            level,
            previous_properties: Vec::new(),
            previous_selection: None,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }
}

impl Command for SetNumberingCommand {
    fn name(&self) -> &'static str {
        "set_numbering"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.previous_selection = *ctx.selection;

        for paragraph_index in start..=end {
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|p| p.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            // Word removes numbering a paragraph style supplies with an
            // explicit `<w:numId w:val="0"/>`. Dropping `w:numPr` instead
            // would let the style number the paragraph again on open.
            let style_numbered = self.num_id.is_none()
                && previous
                    .as_ref()
                    .and_then(|props| props.style.as_deref())
                    .is_some_and(|style| style_supplies_numbering(ctx.document, style));
            self.previous_properties.push((paragraph_index, previous));
            let (num_id, level) = if style_numbered {
                (Some(NUMBERING_OFF.to_string()), None)
            } else {
                (self.num_id.clone(), self.level)
            };

            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                props.numbering = num_id.as_ref().map(|id| Numbering {
                    id: Some(id.clone()),
                    level,
                });
            }
            sync_paragraph_numbering_xml(ctx.document, paragraph_index, num_id.as_deref(), level);
        }

        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous) in &self.previous_properties {
            let (restored_num_id, restored_level) = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
                let numbering = paragraph
                    .properties
                    .as_ref()
                    .and_then(|p| p.numbering.as_ref());
                (
                    numbering.and_then(|n| n.id.clone()),
                    numbering.and_then(|n| n.level),
                )
            };
            sync_paragraph_numbering_xml(
                ctx.document,
                *paragraph_index,
                restored_num_id.as_deref(),
                restored_level,
            );
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use std::io::Read;

    fn load(bytes: &[u8]) -> WordDocument {
        futures::executor::block_on(WordDocument::load(bytes, &Options::default()))
            .expect("load docx")
    }

    fn wrong_num() -> WordDocument {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures/wrong-num.docx"
        ))
        .expect("read fixture");
        load(&bytes)
    }

    fn paragraphs_xml(bytes: &[u8]) -> Vec<String> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document part")
            .read_to_string(&mut xml)
            .expect("read document part");
        let mut paragraphs = Vec::new();
        let mut rest = xml.as_str();
        while let Some(start) = rest.find("<w:p ").or_else(|| rest.find("<w:p>")) {
            let end = rest[start..].find("</w:p>").map(|at| start + at + 6);
            let Some(end) = end else { break };
            paragraphs.push(rest[start..end].to_string());
            rest = &rest[end..];
        }
        paragraphs
    }

    fn num_id(word_doc: &WordDocument, index: usize) -> Option<String> {
        word_doc
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.numbering.as_ref())
            .and_then(|numbering| numbering.id.clone())
    }

    #[test]
    fn removing_style_numbering_writes_num_id_zero() {
        // 「（目的）」 is a heading 2: the style itself is numbered. Word turns
        // its number off with `<w:numId w:val="0"/>`; dropping `w:numPr`
        // would hand the paragraph back to the style's numbering on open.
        let mut word_doc = wrong_num();
        assert_eq!(num_id(&word_doc, 1).as_deref(), Some("23"));
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(SetNumberingCommand::new(1, 1, None, None)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect("remove numbering");
        assert_eq!(num_id(&word_doc, 1).as_deref(), Some("0"));

        let exported = word_doc.export_docx().expect("export");
        let paragraph = &paragraphs_xml(&exported)[1];
        assert!(paragraph.contains("（目的）"), "{paragraph}");
        assert!(paragraph.contains(r#"<w:numId w:val="0"/>"#), "{paragraph}");
        // CT_PPr order: pStyle comes before numPr.
        assert!(
            paragraph.find("<w:pStyle").unwrap() < paragraph.find("<w:numPr>").unwrap(),
            "{paragraph}"
        );
        let reloaded = load(&exported);
        assert_eq!(num_id(&reloaded, 1).as_deref(), Some("0"));

        executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo");
        assert_eq!(num_id(&word_doc, 1).as_deref(), Some("23"));
        let paragraph = &paragraphs_xml(&word_doc.export_docx().unwrap())[1];
        assert!(
            paragraph.contains(r#"<w:numId w:val="23"/>"#),
            "{paragraph}"
        );
    }

    #[test]
    fn removing_direct_numbering_without_style_numbering_drops_num_pr() {
        let mut word_doc = wrong_num();
        // Paragraph 4 has no style; give it a list first.
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        for num_id in [Some("23".to_string()), None] {
            executor
                .execute(
                    Box::new(SetNumberingCommand::new(4, 4, num_id, Some(0))),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .expect("set numbering");
        }
        assert_eq!(num_id(&word_doc, 4), None);
        let paragraph = &paragraphs_xml(&word_doc.export_docx().unwrap())[4];
        assert!(!paragraph.contains("<w:numPr>"), "{paragraph}");
    }
}
