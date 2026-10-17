use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Run;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

pub struct DeleteObjectCommand {
    target: FloatingObjectTarget,
    previous_runs: Option<Vec<Run>>,
    previous_selection: Option<SelectionRange>,
}

impl DeleteObjectCommand {
    pub fn new(target: FloatingObjectTarget) -> Self {
        Self {
            target,
            previous_runs: None,
            previous_selection: None,
        }
    }

    fn collapse_selection(&self) -> SelectionRange {
        SelectionRange::collapsed(TextPosition::new(self.target.paragraph_index, 0))
    }

    fn run_is_empty(run: &Run) -> bool {
        run.text.is_empty()
            && run.image.is_none()
            && run.text_box.is_none()
            && run.math.is_none()
            && run.field.is_none()
            && run.hyperlink.is_none()
    }
}

impl Command for DeleteObjectCommand {
    fn name(&self) -> &'static str {
        "delete_object"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;

        let paragraph = ctx
            .document
            .paragraph_mut(self.target.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        if self.target.run_index >= paragraph.runs.len() {
            return Err(CommandError::Custom(format!(
                "run {} is unavailable in paragraph {}",
                self.target.run_index, self.target.paragraph_index
            )));
        }

        self.previous_runs = Some(paragraph.runs.clone());

        let remove_entire_run = {
            let run = paragraph
                .runs
                .get_mut(self.target.run_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            match self.target.kind {
                FloatingObjectKind::Image => {
                    if run.image.is_none() {
                        return Err(CommandError::Custom(format!(
                            "run {} does not contain an image",
                            self.target.run_index
                        )));
                    }
                    run.image = None;
                }
                FloatingObjectKind::TextBox => {
                    if run.text_box.is_none() {
                        return Err(CommandError::Custom(format!(
                            "run {} does not contain a text box",
                            self.target.run_index
                        )));
                    }
                    run.text_box = None;
                }
            }
            Self::run_is_empty(run)
        };

        if remove_entire_run {
            paragraph.runs.remove(self.target.run_index);
        }

        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = Some(self.collapse_selection());
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let paragraph = ctx
            .document
            .paragraph_mut(self.target.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let backup = self
            .previous_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("missing object backup for undo".into()))?;
        paragraph.runs = backup.clone();
        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, RunImage, RunImageKind, RunImageLayout,
        RunTextBox,
    };
    use crate::word_document::WordDocument;

    fn paragraph_with_runs(runs: Vec<Run>) -> Paragraph {
        Paragraph {
            runs,
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    fn image_run() -> Run {
        Run {
            text: String::new(),
            properties: None,
            image: Some(RunImage {
                relationship_id: Some("rIdImage".to_string()),
                ole_relationship_id: None,
                ole_program_id: None,
                width_px: Some(120.0),
                height_px: Some(80.0),
                title: Some("floating image".to_string()),
                description: Some("floating image".to_string()),
                placeholder_label: None,
                data_uri: Some("data:image/png;base64,AAAA".to_string()),
                chart: None,
                shape_group: None,
                diagram_data_relationship_id: None,
                crop: None,
                layout: RunImageLayout {
                    kind: RunImageKind::Floating,
                    ..RunImageLayout::default()
                },
            }),
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
        }
    }

    fn text_box_run() -> Run {
        Run {
            text: String::new(),
            properties: None,
            image: None,
            text_box: Some(RunTextBox {
                width_px: Some(180.0),
                height_px: Some(90.0),
                text: "Inside textbox".to_string(),
                layout: RunImageLayout {
                    kind: RunImageKind::Floating,
                    ..RunImageLayout::default()
                },
                blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(vec![
                    Run {
                        text: "Inside textbox".to_string(),
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
                    },
                ]))],
                border: Default::default(),
                shadow: None,
                inner_shadow: None,
                glow: None,
                blur_radius_px: None,
                soft_edge_radius_px: None,
                fill_color: None,
                fill_opacity: None,
                fill_theme_color: None,
                fill_theme_transforms: Vec::new(),
                insets: Default::default(),
                vertical_alignment: Default::default(),
                shape: Default::default(),
            }),
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
        }
    }

    fn sample_image_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_runs(vec![image_run()])),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![Run {
                        text: "After image".to_string(),
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
                    }])),
                ],
            }),
            ..Document::default()
        })
    }

    fn sample_text_box_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_runs(vec![text_box_run()])),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![Run {
                        text: "After textbox".to_string(),
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
                    }])),
                ],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn deletes_floating_image_and_restores_it_with_undo_redo() {
        let mut word_doc = sample_image_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteObjectCommand::new(FloatingObjectTarget {
                        paragraph_index: 0,
                        run_index: 0,
                        kind: FloatingObjectKind::Image,
                    })),
                    &mut ctx,
                )
                .expect("delete image");
        }

        assert!(word_doc
            .paragraph(0)
            .expect("host paragraph")
            .runs
            .is_empty());
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo image delete"));
        }

        let restored = word_doc.paragraph(0).expect("restored host paragraph");
        assert_eq!(restored.runs.len(), 1);
        assert!(restored.runs[0].image.is_some());
        assert_eq!(selection, Some(original_selection));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo image delete"));
        }

        assert!(word_doc
            .paragraph(0)
            .expect("host paragraph")
            .runs
            .is_empty());
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
    }

    #[test]
    fn deletes_floating_text_box_and_rebuilds_surfaces_on_undo_redo() {
        let mut word_doc = sample_text_box_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        assert_eq!(word_doc.paragraph_count(), 3);
        assert_eq!(
            word_doc.paragraph(1).expect("text box paragraph").runs[0].text,
            "Inside textbox"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteObjectCommand::new(FloatingObjectTarget {
                        paragraph_index: 0,
                        run_index: 0,
                        kind: FloatingObjectKind::TextBox,
                    })),
                    &mut ctx,
                )
                .expect("delete text box");
        }

        assert_eq!(word_doc.paragraph_count(), 2);
        assert!(word_doc
            .paragraph(0)
            .expect("host paragraph")
            .runs
            .is_empty());
        assert_eq!(
            word_doc.paragraph(1).expect("following paragraph").runs[0].text,
            "After textbox"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo text box delete"));
        }

        assert_eq!(word_doc.paragraph_count(), 3);
        assert!(word_doc.paragraph(0).expect("host paragraph").runs[0]
            .text_box
            .is_some());
        assert_eq!(
            word_doc.paragraph(1).expect("text box paragraph").runs[0].text,
            "Inside textbox"
        );
        assert_eq!(selection, Some(original_selection));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo text box delete"));
        }

        assert_eq!(word_doc.paragraph_count(), 2);
        assert!(word_doc
            .paragraph(0)
            .expect("host paragraph")
            .runs
            .is_empty());
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
    }
}
