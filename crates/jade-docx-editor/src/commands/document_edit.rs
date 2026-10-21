//! An edit a host makes from several [`WordDocument`] calls — a dialog's
//! settings applied to every section, every note converted — as one undo
//! step, undone from a snapshot of the whole package.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

/// What the edit does: changes the document, and may move the selection.
pub type DocumentEditFn =
    Box<dyn FnMut(&mut WordDocument, &mut Option<SelectionRange>) -> Result<(), String>>;

pub struct DocumentEditCommand {
    name: &'static str,
    edit: DocumentEditFn,
    snapshot: Option<(WordDocumentSnapshot, Option<SelectionRange>)>,
}

impl DocumentEditCommand {
    pub fn new(name: &'static str, edit: DocumentEditFn) -> Self {
        Self {
            name,
            edit,
            snapshot: None,
        }
    }
}

impl Command for DocumentEditCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        match (self.edit)(ctx.document, ctx.selection) {
            Ok(()) => {
                self.snapshot = Some((snapshot, selection));
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (snapshot, selection) = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom(format!("{} snapshot is missing", self.name)))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = selection;
        Ok(())
    }
}
