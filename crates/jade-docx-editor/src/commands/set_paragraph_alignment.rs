use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
    sync_paragraph_alignment_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// Applies paragraph alignment to a contiguous paragraph range.
pub struct SetParagraphAlignmentCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    alignment: String,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetParagraphAlignmentCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, alignment: impl Into<String>) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            alignment: alignment.into(),
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
            changed: false,
        }
    }

    fn normalize_alignment(value: &str) -> Result<String, CommandError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(CommandError::Custom(
                "alignment must be a non-empty string".into(),
            ));
        }
        let lowered = trimmed.to_ascii_lowercase();
        let normalized = match lowered.as_str() {
            "start" => "start",
            "left" => "left",
            "center" => "center",
            "end" => "end",
            "right" => "right",
            "both" | "justify" => "justify",
            _ => trimmed,
        };
        Ok(normalized.to_string())
    }

    fn normalized_alignment(&self) -> Result<String, CommandError> {
        Self::normalize_alignment(&self.alignment)
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }
}

impl Command for SetParagraphAlignmentCommand {
    fn name(&self) -> &'static str {
        "set_paragraph_alignment"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let alignment = self.normalized_alignment()?;
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.previous_raw_properties.clear();
        self.previous_selection = *ctx.selection;
        self.changed = false;

        for paragraph_index in start..=end {
            self.previous_raw_properties.push((
                paragraph_index,
                snapshot_paragraph_properties_xml(ctx.document, paragraph_index),
            ));
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties.push((paragraph_index, previous));

            let current_alignment = ctx
                .document
                .paragraph(paragraph_index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.alignment.as_deref());
            let already_applied = match current_alignment {
                Some(current) => {
                    Self::normalize_alignment(current).is_ok_and(|current| current == alignment)
                }
                None => alignment == "left",
            };
            if already_applied {
                continue;
            }
            self.changed = true;

            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                props.alignment = Some(alignment.as_str().into());
            }
            sync_paragraph_alignment_xml(ctx.document, paragraph_index, Some(alignment.as_str()));
        }

        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous) in &self.previous_properties {
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
            }
            let raw_snapshot = self
                .previous_raw_properties
                .iter()
                .find(|(index, _)| index == paragraph_index)
                .and_then(|(_, snapshot)| snapshot.as_ref());
            restore_paragraph_properties_xml(ctx.document, *paragraph_index, raw_snapshot);
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::set_paragraph_visuals::SetParagraphVisualsCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};

    #[test]
    fn justify_uses_words_both_token_and_undo_restores_the_original_xml() {
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:jc w:val="center" w:hint="preserve"/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
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
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Hello".to_string(),
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
                    properties: None,
                    node_id: Some(paragraph_id),
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        document.raw_document = Some(raw);
        let original_xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphAlignmentCommand::new(0, 0, "justify")),
                    &mut ctx,
                )
                .expect("justify");
        }
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:val=\"both\""), "{xml}");
        assert!(!xml.contains("w:val=\"justify\""), "{xml}");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let restored_xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert_eq!(restored_xml, original_xml);
    }

    #[test]
    fn choosing_the_current_alignment_does_not_consume_undo() {
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: Vec::new(),
                    properties: Some(ParagraphProperties {
                        alignment: Some("center".into()),
                        ..ParagraphProperties::default()
                    }),
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphVisualsCommand::shading(
                        0,
                        0,
                        Some("#FFF2CC".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set shading");
            executor
                .execute(
                    Box::new(SetParagraphAlignmentCommand::new(0, 0, "center")),
                    &mut ctx,
                )
                .expect("reapply alignment");
            assert!(executor.undo(&mut ctx).expect("undo shading"));
        }
        assert!(document
            .paragraph(0)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.shading.as_ref())
            .is_none());
    }
}
