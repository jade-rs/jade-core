use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::apply_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, RunProperties};
use jade_docx_layout::selection::SelectionRange;

/// Assigns an explicit bold state to the selected runs.
pub struct SetBoldCommand {
    range: SelectionRange,
    value: bool,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl SetBoldCommand {
    pub fn new(range: SelectionRange, value: bool) -> Self {
        Self {
            range,
            value,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for SetBoldCommand {
    fn name(&self) -> &'static str {
        "set_bold"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }

        let (start, end) = self.range.normalized();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let bold_props = RunProperties {
            bold: Some(self.value),
            ..Default::default()
        };

        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);
            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };

            if range_start >= range_end {
                continue;
            }

            let restyled = apply_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &bold_props,
                &zero_width,
            );
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
        }

        *ctx.selection = Some(self.range);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    fn sample_document() -> WordDocument {
        WordDocument::from_document(Document {
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
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn set_bold_false_disables_existing_bold() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetBoldCommand::new(
                        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5)),
                        true,
                    )),
                    &mut ctx,
                )
                .expect("set bold true");
            executor
                .execute(
                    Box::new(SetBoldCommand::new(
                        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5)),
                        false,
                    )),
                    &mut ctx,
                )
                .expect("set bold false");
        }

        let paragraph = document.paragraph(0).expect("paragraph");
        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            Some(false)
        );
    }
}
