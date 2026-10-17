//! Word's Ctrl+0: add or remove one line (12pt) of space before a paragraph.

use super::set_paragraph_line_spacing::SetParagraphLineSpacingCommand;
use super::{Command, CommandContext, CommandError, CommandResult};

/// The "one line" Word adds above a paragraph for Ctrl+0.
pub const WORD_SPACE_BEFORE_PT: f32 = 12.0;

/// Word's Ctrl+0 over a paragraph range. Ribbon toggle semantics: the space
/// is removed only when every paragraph in the range already carries exactly
/// 12pt before; otherwise every paragraph gains it. Only direct paragraph
/// spacing is consulted, matching how the spacing command reports changes.
pub struct ToggleParagraphSpaceBeforeCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    inner: Option<SetParagraphLineSpacingCommand>,
}

impl ToggleParagraphSpaceBeforeCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            inner: None,
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

impl Command for ToggleParagraphSpaceBeforeCommand {
    fn name(&self) -> &'static str {
        "toggle_paragraph_space_before"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }
        let all_have_space = (start..=end).all(|index| {
            ctx.document
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.spacing.as_ref())
                .and_then(|spacing| spacing.before)
                .is_some_and(|before| (before - WORD_SPACE_BEFORE_PT).abs() < 0.01)
        });
        let target = if all_have_space {
            0.0
        } else {
            WORD_SPACE_BEFORE_PT
        };
        let mut inner = SetParagraphLineSpacingCommand::with_spacing(
            start,
            end,
            None,
            None,
            Some(target),
            None,
        );
        inner.apply(ctx)?;
        self.inner = Some(inner);
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.should_record_history())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        match self.inner.as_mut() {
            Some(inner) => inner.undo(ctx),
            None => Ok(()),
        }
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        match self.inner.as_mut() {
            Some(inner) => inner.redo(ctx),
            None => self.apply(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run, Spacing};
    use crate::word_document::WordDocument;

    fn paragraph(text: &str, before: Option<f32>) -> Paragraph {
        Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: before.map(|before| crate::document::ParagraphProperties {
                spacing: Some(Spacing {
                    before: Some(before),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    fn document_with(paragraphs: Vec<Paragraph>) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: paragraphs
                    .into_iter()
                    .map(BodyElement::from_paragraph)
                    .collect(),
            }),
            ..Document::default()
        })
    }

    fn before_of(document: &WordDocument, index: usize) -> Option<f32> {
        document
            .paragraph(index)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.spacing.as_ref())
            .and_then(|spacing| spacing.before)
    }

    fn toggle(
        executor: &mut CommandExecutor,
        document: &mut WordDocument,
        start: usize,
        end: usize,
    ) {
        let mut selection = None;
        let mut ctx = CommandContext::new(document, &mut selection);
        executor
            .execute(
                Box::new(ToggleParagraphSpaceBeforeCommand::new(start, end)),
                &mut ctx,
            )
            .expect("toggle space before");
    }

    #[test]
    fn toggles_twelve_points_on_and_off_with_undo() {
        let mut document = document_with(vec![paragraph("a", None)]);
        let mut executor = CommandExecutor::new();

        toggle(&mut executor, &mut document, 0, 0);
        assert_eq!(before_of(&document, 0), Some(WORD_SPACE_BEFORE_PT));
        assert_eq!(executor.undo_depth(), 1);

        toggle(&mut executor, &mut document, 0, 0);
        assert_eq!(before_of(&document, 0), Some(0.0));
        assert_eq!(executor.undo_depth(), 2);

        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(before_of(&document, 0), Some(WORD_SPACE_BEFORE_PT));
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(before_of(&document, 0), None);
    }

    #[test]
    fn mixed_range_unifies_to_twelve_points_before_removing() {
        // Ribbon toggle semantics: the space is removed only when every
        // paragraph in the range already carries it; otherwise all gain it.
        let mut document = document_with(vec![
            paragraph("a", Some(WORD_SPACE_BEFORE_PT)),
            paragraph("b", None),
            paragraph("c", Some(6.0)),
        ]);
        let mut executor = CommandExecutor::new();

        toggle(&mut executor, &mut document, 0, 2);
        for index in 0..3 {
            assert_eq!(before_of(&document, index), Some(WORD_SPACE_BEFORE_PT));
        }

        toggle(&mut executor, &mut document, 0, 2);
        for index in 0..3 {
            assert_eq!(before_of(&document, index), Some(0.0));
        }
    }

    #[test]
    fn writes_the_spacing_into_the_paragraph_xml() {
        use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};
        let mut document = document_with(vec![paragraph("a", None)]);
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>a</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .expect("raw document");
        let paragraph_id = raw
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) => body.children.iter().find_map(|node| match node {
                    XmlNode::Element(paragraph) if paragraph.name == "w:p" => Some(paragraph.id),
                    _ => None,
                }),
                _ => None,
            })
            .expect("paragraph id");
        document.raw_document = Some(raw);
        document.paragraph_mut(0).expect("paragraph").node_id = Some(paragraph_id);

        let mut executor = CommandExecutor::new();
        toggle(&mut executor, &mut document, 0, 0);
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(
            xml.contains(r#"w:before="240""#),
            "12pt is 240 twips in w:spacing: {xml}"
        );

        toggle(&mut executor, &mut document, 0, 0);
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains(r#"w:before="0""#), "{xml}");
    }
}
