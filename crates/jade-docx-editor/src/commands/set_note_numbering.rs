use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{ReferenceMarkerSpan, WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

// Numbering changes computed marker lengths without changing stored text.
// Keep reference affinity (including immediately after a zero-text reference)
// and the byte position within the note body while those marker lengths change.
struct NumberingPosition {
    position: TextPosition,
    prefix: usize,
    references: Vec<ReferenceMarkerSpan>,
}

impl NumberingPosition {
    fn capture(document: &WordDocument, position: TextPosition) -> Self {
        Self {
            position,
            prefix: document
                .model_to_rendered_text_position(TextPosition::new(position.paragraph, 0))
                .offset,
            references: document.reference_marker_spans(position.paragraph),
        }
    }

    fn restore(self, document: &WordDocument) -> TextPosition {
        let prefix = document
            .model_to_rendered_text_position(TextPosition::new(self.position.paragraph, 0))
            .offset;
        let mut offset = if self.position.offset < self.prefix {
            self.position.offset.min(prefix)
        } else {
            self.position.offset - self.prefix + prefix
        };
        let mut previous_marker_bytes = 0;
        let current = document.reference_marker_spans(self.position.paragraph);
        for (before, after) in self.references.iter().zip(&current) {
            let start = before.model_offset + previous_marker_bytes;
            let end = start + before.rendered_len;
            if self.position.offset >= end {
                offset = offset.saturating_sub(before.rendered_len) + after.rendered_len;
            } else if self.position.offset > start {
                let consumed = self.position.offset - start;
                offset = offset.saturating_sub(consumed) + consumed.min(after.rendered_len);
            }
            previous_marker_bytes += before.rendered_len;
        }
        TextPosition::new(self.position.paragraph, offset)
    }
}

/// Undoable section note-numbering and restart settings.
pub struct SetNoteNumberingCommand {
    is_footnote: bool,
    format: String,
    start: u32,
    restart: String,
    section: Option<usize>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetNoteNumberingCommand {
    pub fn new(
        is_footnote: bool,
        format: String,
        start: u32,
        restart: String,
        section: Option<usize>,
    ) -> Self {
        Self {
            is_footnote,
            format,
            start,
            restart,
            section,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetNoteNumberingCommand {
    fn name(&self) -> &'static str {
        "set_note_numbering"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        let positions = previous_selection.map(|range| {
            [range.anchor, range.focus]
                .map(|position| NumberingPosition::capture(ctx.document, position))
        });
        match ctx.document.set_note_numbering(
            self.is_footnote,
            &self.format,
            self.start,
            &self.restart,
            self.section,
        ) {
            Ok(changed) => {
                if changed {
                    *ctx.selection = positions.map(|positions| {
                        let [anchor, focus] =
                            positions.map(|position| position.restore(ctx.document));
                        SelectionRange::new(anchor, focus)
                    });
                }
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.changed = changed;
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
            CommandError::Custom("missing note-numbering snapshot for undo".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
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
    use crate::{commands::CommandExecutor, word_document::WordDocument, Options};

    #[test]
    fn numbering_keeps_selection_after_references_and_in_note_text() {
        use jade_docx_layout::selection::TextPosition;
        for is_footnote in [true, false] {
            let bytes = include_bytes!("../../../../public/fixtures/hello.docx");
            let mut document =
                futures::executor::block_on(WordDocument::load(bytes, &Options::default()))
                    .unwrap();
            let note = if is_footnote {
                document.insert_footnote_at(0, 0, "note text").unwrap()
            } else {
                document.insert_endnote_at(0, 0, "note text").unwrap()
            };
            let surface = if is_footnote {
                document.footnote_paragraph_index(&note).unwrap()
            } else {
                document
                    .surface_index_for_story("endnote", 0, None, Some(&note))
                    .unwrap()
            };
            let mut selection = Some(SelectionRange::new(
                TextPosition::new(0, 1),
                TextPosition::new(surface, 5),
            ));
            let before = selection;
            let mut command = SetNoteNumberingCommand::new(
                is_footnote,
                "upperRoman".into(),
                8,
                "continuous".into(),
                Some(0),
            );
            command
                .apply(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(
                selection.unwrap().anchor.offset,
                4,
                "caret stays after the VIII reference"
            );
            assert_eq!(
                selection.unwrap().focus.offset,
                8,
                "caret stays at the same note text byte"
            );
            command
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(selection, before);
        }
    }

    #[test]
    fn numbering_history_restores_package_and_selection_and_ignores_noops() {
        let bytes = include_bytes!("../../../../public/fixtures/hello.docx");
        let mut document =
            futures::executor::block_on(WordDocument::load(bytes, &Options::default())).unwrap();
        let mut selection = Some(SelectionRange::collapsed(
            jade_docx_layout::selection::TextPosition::new(0, 1),
        ));
        let original_selection = selection;
        let before = document.export_docx().unwrap();
        let mut executor = CommandExecutor::new();
        for _ in 0..2 {
            executor
                .execute(
                    Box::new(SetNoteNumberingCommand::new(
                        true,
                        "upperRoman".into(),
                        3,
                        "eachSect".into(),
                        Some(0),
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
        }
        let after = document.export_docx().unwrap();
        assert_ne!(before, after);
        assert!(executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), before);
        assert_eq!(selection, original_selection);
        assert!(!executor.can_undo());
        assert!(executor
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), after);
        let result = executor.execute(
            Box::new(SetNoteNumberingCommand::new(
                true,
                "invalid".into(),
                0,
                "continuous".into(),
                Some(0),
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        );
        assert!(result.is_err());
        assert_eq!(document.export_docx().unwrap(), after);
    }
}
