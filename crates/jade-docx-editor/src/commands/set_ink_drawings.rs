//! The 描画 tab's edits to Word ink, as one undo step: the body's ink
//! drawings replaced by the ones the edit leaves (see
//! [`crate::word_document::ink_drawing`]).

use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::ink_drawing::InkDrawingSpec;
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

pub struct SetInkDrawingsCommand {
    specs: Vec<InkDrawingSpec>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl SetInkDrawingsCommand {
    pub fn new(specs: Vec<InkDrawingSpec>) -> Self {
        Self {
            specs,
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for SetInkDrawingsCommand {
    fn name(&self) -> &'static str {
        "set_ink_drawings"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".to_string()))?;
        let previous_selection = *ctx.selection;
        match ctx.document.set_ink_drawings(&self.specs) {
            Ok(()) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("set_ink_drawings snapshot is missing".to_string())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }
}
