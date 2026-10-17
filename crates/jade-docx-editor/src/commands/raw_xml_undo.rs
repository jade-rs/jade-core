//! Undo for the round-trip XML of structural text edits.
//!
//! A paragraph merge, a delete across paragraphs or a replacement that folds
//! paragraphs used to keep a deep copy of the whole `document.xml` tree for
//! undo. On a long document that copy was most of the cost of a Backspace at
//! the start of a paragraph (~38 of ~45 ms on the 149-page fixture). The
//! commands record a journal of what they change instead, and undo rolls it
//! back.

use super::{CommandContext, CommandError, CommandResult};
use crate::docx_xml::{DocxXmlDocument, XmlEditJournal};

#[derive(Debug, Default)]
pub(crate) struct RawXmlUndo {
    journal: Option<XmlEditJournal>,
    recording: bool,
}

impl RawXmlUndo {
    /// Starts recording the edit. A document without round-trip XML records
    /// nothing, as it had nothing to copy either.
    pub(crate) fn begin(&mut self, ctx: &mut CommandContext<'_>) {
        self.journal = None;
        self.recording = ctx
            .document
            .raw_document
            .as_mut()
            .map(DocxXmlDocument::begin_edit_journal)
            .is_some();
    }

    /// Stops recording and keeps what was recorded for [`Self::undo`]. Runs on
    /// every exit from the edit, including failures, so no journal outlives it.
    /// A failed edit may have put a whole-document snapshot back, taking the
    /// journal with it; it is not undone, so nothing is kept.
    pub(crate) fn finish(&mut self, ctx: &mut CommandContext<'_>, succeeded: bool) {
        if !std::mem::take(&mut self.recording) {
            return;
        }
        self.journal = ctx
            .document
            .raw_document
            .as_mut()
            .and_then(DocxXmlDocument::finish_edit_journal)
            .filter(|_| succeeded);
        debug_assert!(
            !succeeded || self.journal.is_some(),
            "the round-trip XML was replaced while an edit journal was recording"
        );
    }

    /// Stops recording and drops it, for a branch that undoes another way.
    pub(crate) fn discard(&mut self, ctx: &mut CommandContext<'_>) {
        self.finish(ctx, false);
    }

    /// Stops recording and rolls back what was recorded, for an edit that
    /// failed after it had started changing the XML.
    pub(crate) fn abort(&mut self, ctx: &mut CommandContext<'_>) {
        if !std::mem::take(&mut self.recording) {
            return;
        }
        self.journal = None;
        if let Some(raw) = ctx.document.raw_document.as_mut() {
            if let Some(journal) = raw.finish_edit_journal() {
                let _ = raw.roll_back(journal);
            }
        }
    }

    /// Whether undo will put the round-trip XML back itself, so restoring the
    /// model should keep its XML bindings.
    pub(crate) fn restores_xml(&self) -> bool {
        self.journal.is_some()
    }

    /// Restores the model with `restore_model`, then returns the round-trip
    /// XML to its state before the edit.
    ///
    /// Restoring the model rewrites the XML of the paragraphs it touches. Those
    /// writes are journaled and rolled back first, which leaves the XML as the
    /// edit left it, and then the edit itself is rolled back.
    pub(crate) fn undo(
        self,
        ctx: &mut CommandContext<'_>,
        restore_model: impl FnOnce(&mut CommandContext<'_>) -> CommandResult<()>,
    ) -> CommandResult<()> {
        let Some(edit) = self.journal else {
            return restore_model(ctx);
        };
        if let Some(raw) = ctx.document.raw_document.as_mut() {
            raw.begin_edit_journal();
        }
        let restored = restore_model(ctx);
        let restore = ctx
            .document
            .raw_document
            .as_mut()
            .and_then(DocxXmlDocument::finish_edit_journal);
        restored?;
        let (Some(raw), Some(restore)) = (ctx.document.raw_document.as_mut(), restore) else {
            return Err(CommandError::Custom(
                "round-trip XML changed while the edit was undone".into(),
            ));
        };
        raw.roll_back(restore)
            .and_then(|()| raw.roll_back(edit))
            .map_err(|error| CommandError::Custom(error.to_string()))
    }
}
