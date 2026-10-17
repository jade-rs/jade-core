use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Run;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

pub struct InsertTextBoxCommand {
    paragraph_index: usize,
    offset: usize,
    previous_runs: Option<Vec<Run>>,
    previous_selection: Option<SelectionRange>,
    inserted_run_index: Option<usize>,
}

impl InsertTextBoxCommand {
    pub fn new(paragraph_index: usize, offset: usize) -> Self {
        Self {
            paragraph_index,
            offset,
            previous_runs: None,
            previous_selection: None,
            inserted_run_index: None,
        }
    }

    pub fn inserted_target(&self) -> Option<FloatingObjectTarget> {
        Some(FloatingObjectTarget {
            paragraph_index: self.paragraph_index,
            run_index: self.inserted_run_index?,
            kind: FloatingObjectKind::TextBox,
        })
    }
}

impl Command for InsertTextBoxCommand {
    fn name(&self) -> &'static str {
        "insert_text_box"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        self.previous_runs = Some(
            ctx.document
                .paragraph(self.paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .runs
                .clone(),
        );

        let run_index = ctx
            .document
            .insert_text_box_at(self.paragraph_index, self.offset)
            .map_err(CommandError::Custom)?;
        self.inserted_run_index = Some(run_index);
        *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
            self.paragraph_index,
            self.offset,
        )));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_runs = self
            .previous_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("insert_text_box backup is missing".into()))?;
        let paragraph = ctx
            .document
            .paragraph_mut(self.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        paragraph.runs = previous_runs.clone();
        ctx.document.sync_paragraph_text(self.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;

    fn run(text: &str) -> Run {
        Run {
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
        }
    }

    #[test]
    fn inserts_text_box_run_and_restores_it_with_undo() {
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![run("Hello")],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(InsertTextBoxCommand::new(0, 5)), &mut ctx)
                .expect("insert text box");
        }

        let paragraph = document.paragraph(0).expect("paragraph");
        assert_eq!(paragraph.runs.len(), 2);
        assert!(paragraph.runs[1].text_box.is_some());
        assert_eq!(document.paragraph_count(), 2);
        assert_eq!(
            document.first_text_box_paragraph_for_target(FloatingObjectTarget {
                paragraph_index: 0,
                run_index: 1,
                kind: FloatingObjectKind::TextBox,
            }),
            Some(1)
        );
        assert_eq!(
            document.last_text_box_position_for_target(FloatingObjectTarget {
                paragraph_index: 0,
                run_index: 1,
                kind: FloatingObjectKind::TextBox,
            }),
            Some(TextPosition::new(1, 0))
        );
        assert!(document.text_box_contains_paragraph(
            FloatingObjectTarget {
                paragraph_index: 0,
                run_index: 1,
                kind: FloatingObjectKind::TextBox,
            },
            1
        ));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor.undo(&mut ctx).expect("undo insert text box");
        }
        let paragraph = document.paragraph(0).expect("paragraph");
        assert_eq!(paragraph.runs.len(), 1);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(document.paragraph_count(), 1);
    }
}
