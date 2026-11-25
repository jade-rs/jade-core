use super::insert_paragraph_break::InsertParagraphBreakCommand;
use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use jade_docx_reader::PAGE_BREAK_STR;

/// Word inserts a dedicated paragraph for the ribbon/keyboard page break.
pub struct InsertManualPageBreakCommand {
    range: SelectionRange,
    before: Option<(WordDocumentSnapshot, Option<SelectionRange>)>,
}

impl InsertManualPageBreakCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            before: None,
        }
    }

    /// Whether the selection has a supported body paragraph or table anchor.
    pub fn supports(document: &WordDocument, range: SelectionRange) -> bool {
        let (start, end) = range.normalized();
        let start = document.manual_page_break_anchor(start.paragraph);
        let end = document.manual_page_break_anchor(end.paragraph);
        matches!((start, end), (Some((_, false, _)), Some((_, false, _))))
            || matches!(start, Some((_, true, _))) && start == end
    }

    fn insert(&self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, _) = self.range.normalized();
        let (_, before_table, _) = ctx
            .document
            .manual_page_break_anchor(start.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;

        let anchor = if before_table {
            start.paragraph
        } else {
            *ctx.selection = Some(self.range);
            if !self.range.is_collapsed() {
                InsertTextCommand::new(self.range, String::new(), None).apply(ctx)?;
            }
            let caret = ctx.selection.ok_or(CommandError::InvalidSelection)?.focus;
            if caret.offset == 0 {
                caret.paragraph
            } else {
                InsertParagraphBreakCommand::new(SelectionRange::collapsed(caret)).apply(ctx)?;
                ctx.selection
                    .ok_or(CommandError::InvalidSelection)?
                    .focus
                    .paragraph
            }
        };

        let paragraph = ctx
            .document
            .insert_manual_page_break_paragraph(anchor)
            .map_err(CommandError::Custom)?;
        let at_break = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
        InsertTextCommand::new(at_break, PAGE_BREAK_STR.to_owned(), None).apply(ctx)?;
        if ctx.document.revision_session().is_some() {
            ctx.document
                .mark_inserted_paragraph_break(paragraph)
                .map_err(CommandError::Custom)?;
        }
        if !before_table {
            *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
                paragraph + 1,
                0,
            )));
        }
        Ok(())
    }
}

impl Command for InsertManualPageBreakCommand {
    fn name(&self) -> &'static str {
        "insert_manual_page_break"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.before = None;
        if !Self::supports(ctx.document, self.range) {
            return Err(CommandError::UnsupportedSelection);
        }
        let (start, end) = self.range.normalized();
        // A text edit with a paragraph boundary has the same protection
        // requirements, including the restrictions inside content controls.
        let structural_edit = HeadlessCommand::ReplaceRange {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
            text: "\n".to_owned(),
        };
        if let Some(reason) = ctx.document.headless_edit_refusal(&structural_edit) {
            return Err(CommandError::Custom(reason.to_owned()));
        }
        for position in [start, end] {
            ctx.document
                .validate_text_position_boundary(
                    ctx.document.rendered_to_model_text_position(position),
                )
                .map_err(CommandError::Custom)?;
        }
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        if let Err(error) = self.insert(ctx) {
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = previous_selection;
            return Err(error);
        }
        self.before = Some((snapshot, previous_selection));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (snapshot, selection) = self
            .before
            .take()
            .ok_or_else(|| CommandError::Custom("missing page-break undo state".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = selection;
        Ok(())
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }
}
