//! Deleting text that holds note reference marks deletes those notes, as
//! Word does: a footnote or endnote lives only as long as its mark.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

/// The notes whose reference marks lie wholly inside a rendered range of
/// body paragraphs: `(note id, is footnote)`, each once.
pub fn notes_in_rendered_range(
    document: &WordDocument,
    range: SelectionRange,
) -> Vec<(String, bool)> {
    let (start, end) = range.normalized();
    let mut notes: Vec<(String, bool)> = Vec::new();
    if start == end {
        return notes;
    }
    for paragraph in start.paragraph..=end.paragraph {
        let mut rendered = 0usize;
        for span in document.reference_marker_spans(paragraph) {
            let mark_start = span.model_offset + rendered;
            let mark_end = mark_start + span.rendered_len;
            rendered += span.rendered_len;
            let after_start = paragraph > start.paragraph || mark_start >= start.offset;
            let before_end = paragraph < end.paragraph || mark_end <= end.offset;
            if after_start && before_end {
                let note = (span.note_id, span.is_footnote);
                if !notes.contains(&note) {
                    notes.push(note);
                }
            }
        }
    }
    notes
}

/// Runs a text edit over a range, then deletes the notes whose marks the
/// range held. Undone from a snapshot of the whole package, which the notes
/// parts are in.
pub struct DeleteRangeNotesCommand {
    inner: Box<dyn Command>,
    notes: Vec<(String, bool)>,
    snapshot: Option<(WordDocumentSnapshot, Option<SelectionRange>)>,
}

impl DeleteRangeNotesCommand {
    pub fn new(inner: Box<dyn Command>, notes: Vec<(String, bool)>) -> Self {
        Self {
            inner,
            notes,
            snapshot: None,
        }
    }
}

impl Command for DeleteRangeNotesCommand {
    fn name(&self) -> &'static str {
        "delete_range_notes"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let outcome = self.inner.apply(ctx).and_then(|()| {
            for (id, is_footnote) in &self.notes {
                let deleted = if *is_footnote {
                    ctx.document.delete_footnote(id)
                } else {
                    ctx.document.delete_endnote(id)
                };
                deleted.map_err(CommandError::Custom)?;
            }
            Ok(())
        });
        match outcome {
            Ok(()) => {
                self.snapshot = Some((snapshot, selection));
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = selection;
                Err(error)
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (snapshot, selection) = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("note deletion snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = selection;
        Ok(())
    }
}
