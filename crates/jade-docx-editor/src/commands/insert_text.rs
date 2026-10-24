use super::delete_text::{
    capture_paragraph_range, paragraph_mark_is_removable, range_tail_inline_xml,
    remove_whole_paragraphs, restore_multi_paragraph_edit, DeleteDirection, DeleteTextCommand,
    FoldedSelection,
};
use super::paragraph_undo::{commented_body_paragraph, ParagraphRangeUndo};
use super::raw_xml_undo::RawXmlUndo;
use super::text_edit::{
    append_rebased_tail_comment_anchors, apply_run_properties, apply_text_edit,
    normalize_break_runs, range_contains_deleted_revision, rebase_comment_anchors_for_edit,
    split_runs_at_preserving_source, unprotected_spans, CommentAnchors,
};
use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::document::{Paragraph, RevisionKind, Run, RunProperties};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Command that inserts plain text at the current caret position. When the
/// selection spans characters it behaves like a replacement (the original range
/// is removed and the inserted text takes its place).
pub struct InsertTextCommand {
    range: SelectionRange,
    text: String,
    delete_selected_tables: bool,
    inline_properties: Option<RunProperties>,
    previous_runs: Option<Vec<Run>>,
    previous_anchors: Option<CommentAnchors>,
    previous_paragraphs: Option<Vec<Paragraph>>,
    raw_xml_undo: RawXmlUndo,
    previous_selection: Option<SelectionRange>,
    /// Package snapshot for cross-container replacements and structural note
    /// edits, whose raw part XML cannot be restored from paragraph models.
    previous_snapshot: Option<crate::word_document::WordDocumentSnapshot>,
    /// Undo for a replacement that rewrites its paragraphs without removing
    /// any: a tracked one across sibling paragraphs (it marks the text and
    /// paragraph marks deleted) or one across containers.
    in_place: Option<ParagraphRangeUndo>,
    note_range_deletion: Option<DeleteTextCommand>,
    paragraph_index: usize,
}

impl InsertTextCommand {
    pub fn new(
        range: SelectionRange,
        text: String,
        inline_properties: Option<RunProperties>,
    ) -> Self {
        let paragraph_index = range.focus.paragraph;
        Self {
            range,
            text,
            delete_selected_tables: false,
            inline_properties,
            previous_runs: None,
            previous_anchors: None,
            previous_paragraphs: None,
            raw_xml_undo: RawXmlUndo::default(),
            previous_selection: None,
            previous_snapshot: None,
            in_place: None,
            note_range_deletion: None,
            paragraph_index,
        }
    }

    /// Server range deletion includes tables whose complete content is selected.
    /// Browser text selections retain their existing cell-clearing behavior.
    pub(crate) fn deleting_selected_tables(mut self) -> Self {
        self.delete_selected_tables = true;
        self
    }

    fn apply_edit(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let (start, end) = self.range.normalized();
        let start = ctx.document.rendered_to_model_text_position(start);
        let end = ctx.document.rendered_to_model_text_position(end);

        let paragraph_count = ctx.document.paragraph_count();
        if start.paragraph >= paragraph_count || end.paragraph >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        let start_len = ctx
            .document
            .paragraph(start.paragraph)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let end_len = ctx
            .document
            .paragraph(end.paragraph)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;

        let start_offset = start.offset.min(start_len);
        let end_offset = end.offset.min(end_len);

        if self.delete_selected_tables && self.text.is_empty() {
            if let Some(snapshot) = super::table_range::delete_range_with_tables(ctx, start, end)? {
                self.previous_snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.in_place = None;
                self.note_range_deletion = None;
                self.previous_paragraphs = None;
                self.raw_xml_undo = RawXmlUndo::default();
                return Ok(());
            }
        }

        // A tracked selection spanning sibling paragraphs keeps both source
        // paragraphs until review. The selected text and every intervening
        // paragraph mark are deleted revisions; replacement text is inserted
        // after the struck-through text in the first paragraph.
        if start.paragraph != end.paragraph
            && ctx.document.revision_session().is_some()
            && (start.paragraph + 1..=end.paragraph).all(|index| {
                ctx.document
                    .paragraphs_share_a_container(start.paragraph, index)
            })
        {
            // A note keeps its paragraphs' XML in its own part, which the
            // journal does not cover.
            let (mut tracked_range, snapshot) =
                if ctx.document.is_note_paragraph_surface(start.paragraph)
                    || (start.paragraph..end.paragraph)
                        .any(|index| ctx.document.is_own_inserted_paragraph_break(index))
                {
                    let snapshot = ctx
                        .document
                        .snapshot_including_package()
                        .ok_or(CommandError::DocumentUnavailable)?;
                    (None, Some(snapshot))
                } else {
                    let range = Some((start.paragraph, end.paragraph));
                    (Some(ParagraphRangeUndo::begin(ctx, range)?), None)
                };
            let result = (|| -> Result<usize, String> {
                let mut insert_offset = start_len;
                for index in start.paragraph..=end.paragraph {
                    let (from, to) = if index == start.paragraph {
                        (start_offset, start_len)
                    } else if index == end.paragraph {
                        (0, end_offset)
                    } else {
                        let len = ctx
                            .document
                            .paragraph(index)
                            .map(paragraph_len)
                            .ok_or("selected paragraph is unavailable")?;
                        (0, len)
                    };
                    if from >= to {
                        continue;
                    }
                    let spans = ctx
                        .document
                        .paragraph(index)
                        .map(|paragraph| unprotected_spans(&paragraph.runs, from, to))
                        .ok_or("selected paragraph is unavailable")?;
                    for (span_start, span_end) in spans.iter().rev() {
                        if !ctx.document.apply_deletion_revision(
                            index,
                            *span_start,
                            span_end - span_start,
                        ) {
                            return Err("failed to mark selected text as deleted".into());
                        }
                    }
                    if index == start.paragraph {
                        insert_offset = ctx
                            .document
                            .paragraph(index)
                            .map(paragraph_len)
                            .ok_or("first selected paragraph is unavailable")?;
                    }
                    if !spans.is_empty() {
                        ctx.document.sync_paragraph_text_preserving_drawings(index);
                    }
                }
                for index in (start.paragraph..end.paragraph).rev() {
                    ctx.document.mark_deleted_paragraph_break(index)?;
                }
                if !self.text.is_empty() {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    let paragraph = ctx
                        .document
                        .paragraph_mut(start.paragraph)
                        .ok_or("first selected paragraph is unavailable")?;
                    apply_text_edit(
                        &mut paragraph.runs,
                        insert_offset,
                        insert_offset,
                        &self.text,
                        &zero_width,
                    );
                    rebase_comment_anchors_for_edit(
                        paragraph,
                        insert_offset,
                        insert_offset,
                        self.text.len(),
                    );
                    if let Some(props) = &self.inline_properties {
                        apply_run_properties(
                            &mut paragraph.runs,
                            insert_offset,
                            insert_offset + self.text.len(),
                            props,
                            &zero_width,
                        );
                    }
                    normalize_break_runs(&mut paragraph.runs);
                    ctx.document.apply_insertion_revision(
                        start.paragraph,
                        insert_offset,
                        self.text.len(),
                    );
                    ctx.document
                        .sync_paragraph_text_preserving_drawings(start.paragraph);
                }
                Ok(insert_offset)
            })();
            let insert_offset = match result {
                Ok(offset) => offset,
                Err(error) => {
                    if let Some(tracked_range) = tracked_range {
                        tracked_range.abort(ctx);
                    } else if let Some(snapshot) = snapshot {
                        ctx.document.restore_snapshot(snapshot);
                    }
                    return Err(CommandError::Custom(error));
                }
            };
            self.previous_selection = previous_selection;
            self.paragraph_index = start.paragraph;
            ctx.document
                .apply_run_font_defaults_at(self.paragraph_index);
            if let Some(tracked_range) = tracked_range.as_mut() {
                tracked_range.finish(ctx);
            }
            self.in_place = tracked_range;
            self.previous_snapshot = snapshot;
            *ctx.selection = Some(SelectionRange::collapsed(
                ctx.document
                    .model_to_rendered_text_position(TextPosition::new(
                        start.paragraph,
                        insert_offset + self.text.len(),
                    )),
            ));
            return Ok(());
        }

        self.paragraph_index = start.paragraph;
        self.previous_runs = None;
        self.previous_anchors = None;
        self.previous_paragraphs = None;
        self.raw_xml_undo = RawXmlUndo::default();
        self.previous_snapshot = None;
        self.in_place = None;
        self.note_range_deletion = None;

        // A selection from the start of a paragraph to the start of a later
        // one holds whole paragraphs, marks included (Word's triple-click
        // selection). Deleting it removes those paragraphs; the paragraph
        // after them keeps its own formatting instead of taking the first
        // selected paragraph's.
        if self.text.is_empty()
            && start.paragraph < end.paragraph
            && start_offset == 0
            && end_offset == 0
            && (start.paragraph + 1..=end.paragraph).all(|index| {
                ctx.document
                    .paragraphs_share_a_container(start.paragraph, index)
            })
            && (start.paragraph..end.paragraph).all(|index| {
                ctx.document
                    .paragraph(index)
                    .is_some_and(paragraph_mark_is_removable)
            })
        {
            if ctx.document.is_note_paragraph_surface(start.paragraph) {
                let snapshot = ctx
                    .document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?;
                if let Err(error) = remove_whole_paragraphs(ctx, start.paragraph, end.paragraph - 1)
                {
                    ctx.document.restore_snapshot(snapshot);
                    return Err(error);
                }
                self.previous_snapshot = Some(snapshot);
            } else {
                // Undo splits the following paragraph back into the removed
                // ones, as for a join, and rolls back the XML; a snapshot of
                // the whole document here cost ~100 ms on yokohama.
                let backups = capture_paragraph_range(ctx, start.paragraph, end.paragraph)?;
                self.raw_xml_undo.begin(ctx);
                if let Err(error) = remove_whole_paragraphs(ctx, start.paragraph, end.paragraph - 1)
                {
                    self.raw_xml_undo.abort(ctx);
                    return Err(error);
                }
                self.previous_paragraphs = Some(backups);
            }
            self.previous_selection = previous_selection;
            *ctx.selection = Some(SelectionRange::collapsed(
                ctx.document
                    .model_to_rendered_text_position(TextPosition::new(start.paragraph, 0)),
            ));
            return Ok(());
        }

        // The browser expresses selection deletion as an empty replacement.
        // Use the note-aware deletion command so raw anchors and the note
        // package history follow the same path as Backspace/Delete joins.
        if self.text.is_empty()
            && start.paragraph != end.paragraph
            && ctx.document.is_note_paragraph_surface(start.paragraph)
            && ctx
                .document
                .paragraphs_share_a_container(start.paragraph, end.paragraph)
        {
            let mut command = DeleteTextCommand::new(self.range, DeleteDirection::Backward);
            command.apply(ctx)?;
            self.note_range_deletion = Some(command);
            return Ok(());
        }

        // A replacement that crosses a container boundary (cell to cell,
        // body to cell) cannot fold the paragraphs into one: every cell keeps
        // its paragraph. Word clears the selected text of each paragraph and
        // types into the first, and so does this branch.
        if start.paragraph != end.paragraph
            && (start.paragraph + 1..=end.paragraph).any(|index| {
                !ctx.document
                    .paragraphs_share_a_container(start.paragraph, index)
            })
        {
            // Nothing is removed here, so the paragraphs, a journal of the XML
            // and the revision id counter restore the cleared cells exactly; a
            // note keeps its XML in its own part, which the journal does not
            // cover.
            let (mut range_undo, snapshot) = if (start.paragraph..=end.paragraph)
                .any(|index| ctx.document.is_note_paragraph_surface(index))
            {
                let snapshot = ctx
                    .document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?;
                (None, Some(snapshot))
            } else {
                let range = Some((start.paragraph, end.paragraph));
                (Some(ParagraphRangeUndo::begin(ctx, range)?), None)
            };
            let result = (|| -> CommandResult<usize> {
                let recording = ctx.document.revision_session().is_some();
                let mut insert_offset = start_offset;
                for index in start.paragraph..=end.paragraph {
                    let len = ctx
                        .document
                        .paragraph(index)
                        .map(paragraph_len)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    let (from, to) = if index == start.paragraph {
                        (start_offset, len)
                    } else if index == end.paragraph {
                        (0, end_offset.min(len))
                    } else {
                        (0, len)
                    };
                    if from >= to && (index != start.paragraph || self.text.is_empty()) {
                        continue;
                    }
                    let spans = ctx
                        .document
                        .paragraph(index)
                        .map(|paragraph| unprotected_spans(&paragraph.runs, from, to))
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    if recording {
                        for (span_start, span_end) in spans.iter().rev() {
                            if !ctx.document.apply_deletion_revision(
                                index,
                                *span_start,
                                span_end - span_start,
                            ) {
                                return Err(CommandError::Custom(
                                    "failed to mark selected text as deleted".into(),
                                ));
                            }
                        }
                    } else {
                        let zero_width = ctx.document.zero_width_runs(index);
                        for (span_start, span_end) in spans.iter().rev() {
                            let paragraph = ctx
                                .document
                                .paragraph_mut(index)
                                .ok_or(CommandError::ParagraphUnavailable)?;
                            apply_text_edit(
                                &mut paragraph.runs,
                                *span_start,
                                *span_end,
                                "",
                                &zero_width,
                            );
                            rebase_comment_anchors_for_edit(paragraph, *span_start, *span_end, 0);
                        }
                    }
                    if index == start.paragraph {
                        insert_offset = if recording {
                            let current_len = ctx
                                .document
                                .paragraph(index)
                                .map(paragraph_len)
                                .ok_or(CommandError::ParagraphUnavailable)?;
                            to.saturating_sub(len.saturating_sub(current_len))
                        } else {
                            to - spans.iter().map(|(a, b)| b - a).sum::<usize>()
                        };
                        let zero_width = ctx.document.zero_width_runs(index);
                        let paragraph = ctx
                            .document
                            .paragraph_mut(index)
                            .ok_or(CommandError::ParagraphUnavailable)?;
                        if !self.text.is_empty() {
                            apply_text_edit(
                                &mut paragraph.runs,
                                insert_offset,
                                insert_offset,
                                &self.text,
                                &zero_width,
                            );
                            rebase_comment_anchors_for_edit(
                                paragraph,
                                insert_offset,
                                insert_offset,
                                self.text.len(),
                            );
                        }
                        if let Some(props) = &self.inline_properties {
                            if !self.text.is_empty() {
                                apply_run_properties(
                                    &mut paragraph.runs,
                                    insert_offset,
                                    insert_offset + self.text.len(),
                                    props,
                                    &zero_width,
                                );
                            }
                        }
                        normalize_break_runs(&mut paragraph.runs);
                        if !self.text.is_empty() {
                            ctx.document.apply_insertion_revision(
                                index,
                                insert_offset,
                                self.text.len(),
                            );
                        }
                    }
                    ctx.document.sync_paragraph_text_preserving_drawings(index);
                }
                ctx.document
                    .apply_run_font_defaults_at(self.paragraph_index);
                Ok(insert_offset)
            })();
            let insert_offset = match result {
                Ok(insert_offset) => insert_offset,
                Err(error) => {
                    if let Some(range_undo) = range_undo {
                        range_undo.abort(ctx);
                    } else if let Some(snapshot) = snapshot {
                        ctx.document.restore_snapshot(snapshot);
                    }
                    return Err(error);
                }
            };
            if let Some(range_undo) = range_undo.as_mut() {
                range_undo.finish(ctx);
            }
            self.in_place = range_undo;
            self.previous_snapshot = snapshot;
            let new_focus = ctx
                .document
                .model_to_rendered_text_position(TextPosition::new(
                    start.paragraph,
                    insert_offset + self.text.len(),
                ));
            self.previous_selection = previous_selection;
            *ctx.selection = Some(SelectionRange::collapsed(new_focus));
            return Ok(());
        }

        // Where the inserted text ends up. A tracked replacement leaves the old
        // text in place, struck through, so the new text goes after it.
        let mut insert_offset = start_offset;

        if start.paragraph == end.paragraph {
            // Replacing a range while change tracking is on is a redline, not a
            // rewrite: Word keeps the old text struck through next to the new
            // one. Removing it outright would hand the reviewer a document that
            // no longer says what was taken out.
            let selected_deleted_revision = start_offset < end_offset
                && ctx
                    .document
                    .paragraph(start.paragraph)
                    .is_some_and(|paragraph| {
                        range_is_deleted_revision(&paragraph.runs, start_offset, end_offset)
                    });
            let unprotected_spans = ctx
                .document
                .paragraph(start.paragraph)
                .map(|paragraph| unprotected_spans(&paragraph.runs, start_offset, end_offset))
                .ok_or(CommandError::ParagraphUnavailable)?;
            let mixed_deleted_revision = start_offset < end_offset
                && !selected_deleted_revision
                && ctx
                    .document
                    .paragraph(start.paragraph)
                    .is_some_and(|paragraph| {
                        range_contains_deleted_revision(&paragraph.runs, start_offset, end_offset)
                    });
            let tracked_replacement = start_offset < end_offset
                && !selected_deleted_revision
                && !mixed_deleted_revision
                && ctx.document.revision_session().is_some();
            if tracked_replacement || selected_deleted_revision {
                insert_offset = end_offset;
            }
            let previous_anchors = ctx
                .document
                .paragraph(start.paragraph)
                .map(CommentAnchors::of);
            // Comment markers sit between runs in the XML. Once an edit has
            // pushed some together, re-syncing the restored runs cannot tell
            // which side of a marker the text goes back on, and the comment
            // lost or gained text after an undo; journal this paragraph's XML.
            // A replaced selection is journaled too: it can empty a simple
            // field or a link down to one run, and re-syncing the restored runs
            // into it left the replacement in the field and wrote the old text
            // again after it. Typing at a caret keeps the cheaper re-sync.
            // (Note paragraphs keep their XML in their own part.)
            let replaces_selection = start_offset < end_offset
                && !ctx.document.is_note_paragraph_surface(start.paragraph);
            let anchored = if replaces_selection || commented_body_paragraph(ctx, start.paragraph) {
                Some(ParagraphRangeUndo::begin(
                    ctx,
                    Some((start.paragraph, start.paragraph)),
                )?)
            } else {
                None
            };
            let previous_runs = if selected_deleted_revision {
                let previous_runs = ctx
                    .document
                    .paragraph(start.paragraph)
                    .map(|paragraph| paragraph.runs.clone())
                    .ok_or(CommandError::ParagraphUnavailable)?;
                if !self.text.is_empty() {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                        apply_text_edit(
                            &mut paragraph.runs,
                            end_offset,
                            end_offset,
                            &self.text,
                            &zero_width,
                        );
                        rebase_comment_anchors_for_edit(
                            paragraph,
                            end_offset,
                            end_offset,
                            self.text.len(),
                        );
                    }
                }
                previous_runs
            } else if mixed_deleted_revision {
                let previous_runs = ctx
                    .document
                    .paragraph(start.paragraph)
                    .map(|paragraph| paragraph.runs.clone())
                    .ok_or(CommandError::ParagraphUnavailable)?;
                if ctx.document.revision_session().is_some() {
                    let previous_len = previous_runs
                        .iter()
                        .map(|run| run.text.len())
                        .sum::<usize>();
                    for (from, to) in unprotected_spans.iter().rev() {
                        ctx.document
                            .apply_deletion_revision(start.paragraph, *from, to - from);
                    }
                    let current_len = ctx
                        .document
                        .paragraph(start.paragraph)
                        .map(paragraph_len)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    insert_offset =
                        end_offset.saturating_sub(previous_len.saturating_sub(current_len));
                } else {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    for (from, to) in unprotected_spans.iter().rev() {
                        if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                            apply_text_edit(&mut paragraph.runs, *from, *to, "", &zero_width);
                            rebase_comment_anchors_for_edit(paragraph, *from, *to, 0);
                        }
                    }
                    insert_offset = end_offset
                        - unprotected_spans
                            .iter()
                            .map(|(from, to)| to - from)
                            .sum::<usize>();
                }
                if !self.text.is_empty() {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                        apply_text_edit(
                            &mut paragraph.runs,
                            insert_offset,
                            insert_offset,
                            &self.text,
                            &zero_width,
                        );
                        rebase_comment_anchors_for_edit(
                            paragraph,
                            insert_offset,
                            insert_offset,
                            self.text.len(),
                        );
                    }
                }
                previous_runs
            } else if tracked_replacement {
                let previous_runs = ctx
                    .document
                    .paragraph(start.paragraph)
                    .map(|paragraph| paragraph.runs.clone())
                    .ok_or(CommandError::ParagraphUnavailable)?;
                ctx.document.apply_deletion_revision(
                    start.paragraph,
                    start_offset,
                    end_offset - start_offset,
                );
                let previous_len = previous_runs
                    .iter()
                    .map(|run| run.text.len())
                    .sum::<usize>();
                let current_len = ctx
                    .document
                    .paragraph(start.paragraph)
                    .map(paragraph_len)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                insert_offset = end_offset.saturating_sub(previous_len.saturating_sub(current_len));
                // Insert after retained deletions, accounting for canceled insertions.
                if !self.text.is_empty() {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                        apply_text_edit(
                            &mut paragraph.runs,
                            insert_offset,
                            insert_offset,
                            &self.text,
                            &zero_width,
                        );
                        rebase_comment_anchors_for_edit(
                            paragraph,
                            insert_offset,
                            insert_offset,
                            self.text.len(),
                        );
                    }
                }
                previous_runs
            } else {
                let zero_width = ctx.document.zero_width_runs(start.paragraph);
                let paragraph = ctx
                    .document
                    .paragraph_mut(start.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let previous_runs = paragraph.runs.clone();
                apply_text_edit(
                    &mut paragraph.runs,
                    start_offset,
                    end_offset,
                    &self.text,
                    &zero_width,
                );
                rebase_comment_anchors_for_edit(
                    paragraph,
                    start_offset,
                    end_offset,
                    self.text.len(),
                );
                previous_runs
            };
            if !self.text.is_empty() {
                ctx.document.apply_insertion_revision(
                    start.paragraph,
                    insert_offset,
                    self.text.len(),
                );
            }
            let zero_width = ctx.document.zero_width_runs(start.paragraph);
            if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                if let Some(props) = &self.inline_properties {
                    if !self.text.is_empty() {
                        let insert_end = insert_offset + self.text.len();
                        apply_run_properties(
                            &mut paragraph.runs,
                            insert_offset,
                            insert_end,
                            props,
                            &zero_width,
                        );
                    }
                }
                normalize_break_runs(&mut paragraph.runs);
            }
            ctx.document
                .sync_paragraph_text_preserving_drawings(start.paragraph);
            self.previous_runs = Some(previous_runs);
            self.previous_anchors = previous_anchors;
            if let Some(mut undo) = anchored {
                undo.finish(ctx);
                self.in_place = Some(undo);
            }
        } else {
            let note_story = ctx.document.is_note_paragraph_surface(start.paragraph);
            if note_story {
                self.previous_snapshot = Some(
                    ctx.document
                        .snapshot_including_package()
                        .ok_or(CommandError::DocumentUnavailable)?,
                );
            } else {
                self.raw_xml_undo.begin(ctx);
                let mut previous_paragraphs =
                    Vec::with_capacity(end.paragraph - start.paragraph + 1);
                for paragraph_index in start.paragraph..=end.paragraph {
                    let paragraph = ctx
                        .document
                        .paragraph(paragraph_index)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    previous_paragraphs.push(paragraph.clone());
                }
                self.previous_paragraphs = Some(previous_paragraphs);
            }
            // The last paragraph's remainder moves into the first as authored
            // XML, not rebuilt from the run model: the model does not carry a
            // drawing's custom geometry or every inline anchor, so a rebuild
            // turned a shape after the selection into a plain rectangle.
            let (mut tail_inline_xml, tail_comment_references) =
                range_tail_inline_xml(ctx, start.paragraph, end.paragraph, end_offset);

            // The selected paragraphs are folded below. Keep existing review
            // deletions from every selected paragraph, including paragraphs
            // that disappear in the fold. When recording, first mark ordinary
            // selected text as deleted so the reviewer can see the replacement.
            let recording = ctx.document.revision_session().is_some();
            if recording {
                for paragraph_index in start.paragraph..=end.paragraph {
                    let (from, to) = if paragraph_index == start.paragraph {
                        (start_offset, start_len)
                    } else if paragraph_index == end.paragraph {
                        (0, end_offset)
                    } else {
                        let len = ctx
                            .document
                            .paragraph(paragraph_index)
                            .map(paragraph_len)
                            .ok_or(CommandError::ParagraphUnavailable)?;
                        (0, len)
                    };
                    if from == to {
                        continue;
                    }
                    let spans = ctx
                        .document
                        .paragraph(paragraph_index)
                        .map(|paragraph| unprotected_spans(&paragraph.runs, from, to))
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    for (span_start, span_end) in spans.iter().rev() {
                        if !ctx.document.apply_deletion_revision(
                            paragraph_index,
                            *span_start,
                            span_end - span_start,
                        ) {
                            return Err(CommandError::Custom(
                                "could not record cross-paragraph replacement".into(),
                            ));
                        }
                    }
                }
            }
            // Fields go with the selection as they do in a deletion; a
            // recorded one leaves the selected text, so every field stays.
            let mut folded = FoldedSelection::new(
                ctx,
                (start.paragraph, start_offset),
                (end.paragraph, end_offset),
                recording,
            )?;
            folded.forget_transferred_references(&tail_comment_references);
            let retained_len = folded.retained_len();
            let retained = std::mem::take(&mut folded.retained);
            insert_offset = start_offset + retained_len;
            folded.prepend_moved_code(ctx, &mut tail_inline_xml);

            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(start.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let _ = split_runs_at_preserving_source(
                    &mut paragraph.runs,
                    start_offset,
                    &folded.zero_width,
                );
                paragraph.runs.retain(|run| !folded.drops(run));
            }
            let retained_first = ctx
                .document
                .paragraph(start.paragraph)
                .map_or(0, |paragraph| paragraph.runs.len());
            if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                paragraph.runs.extend(retained);
            }
            // Before the replacement goes in after them and takes their
            // formatting.
            FoldedSelection::restyle_retained(
                &folded.retained_styles,
                ctx,
                start.paragraph,
                retained_first,
            );
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(start.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                apply_text_edit(
                    &mut paragraph.runs,
                    insert_offset,
                    insert_offset,
                    &self.text,
                    &folded.zero_width,
                );
            }

            let tail_paragraph = ctx
                .document
                .paragraph(end.paragraph)
                .cloned()
                .ok_or(CommandError::ParagraphUnavailable)?;
            let suffix_runs = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(end.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let mut tail = split_runs_at_preserving_source(
                    &mut paragraph.runs,
                    end_offset,
                    &folded.zero_width,
                );
                tail.retain(|run| !folded.drops(run));
                paragraph.runs.clear();
                tail
            };

            // As for a deleted selection: the merged paragraph ends with the
            // last paragraph's mark, and its section break.
            ctx.document
                .move_section_break(end.paragraph, start.paragraph)
                .map_err(CommandError::Custom)?;
            ctx.document
                .remove_paragraph_range_at(start.paragraph + 1, end.paragraph);

            ctx.document
                .insert_paragraph_inline_xml(start.paragraph, tail_inline_xml, false)
                .map_err(CommandError::Custom)?;
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(start.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.runs.extend(tail_comment_references);
                // The raw tail nodes moved as well, so retain their source IDs.
                let moved = paragraph.runs.len()..paragraph.runs.len() + suffix_runs.len();
                paragraph.runs.extend(suffix_runs);
                ctx.document.restyle_runs_from(
                    start.paragraph,
                    moved,
                    crate::commands::delete_text::style_of(&tail_paragraph),
                );
            }

            if !self.text.is_empty() {
                ctx.document.apply_insertion_revision(
                    start.paragraph,
                    insert_offset,
                    self.text.len(),
                );
            }

            let zero_width = ctx.document.zero_width_runs(start.paragraph);
            if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
                rebase_comment_anchors_for_edit(
                    paragraph,
                    start_offset,
                    start_len,
                    retained_len + self.text.len(),
                );
                if let Some(props) = &self.inline_properties {
                    if !self.text.is_empty() {
                        let insert_end = insert_offset + self.text.len();
                        apply_run_properties(
                            &mut paragraph.runs,
                            insert_offset,
                            insert_end,
                            props,
                            &zero_width,
                        );
                    }
                }
                normalize_break_runs(&mut paragraph.runs);
                append_rebased_tail_comment_anchors(
                    paragraph,
                    &tail_paragraph,
                    end_offset,
                    insert_offset + self.text.len(),
                );
            }

            ctx.document
                .sync_paragraph_text_preserving_drawings(start.paragraph);
            ctx.document.rebuild_paragraph_surfaces();
        }

        // A run created in a paragraph that had none carries no font: nothing
        // resolved the document's defaults onto it, and only a file read does
        // that. Left alone it is drawn in the renderer's fallback font, a
        // different size from the same document reopened — visible as text in a
        // freshly inserted table cell wrapping at the wrong width.
        ctx.document
            .apply_run_font_defaults_at(self.paragraph_index);

        let caret_offset = insert_offset + self.text.len();
        let new_focus = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(self.paragraph_index, caret_offset));
        let new_selection = SelectionRange::collapsed(new_focus);

        self.previous_selection = previous_selection;
        *ctx.selection = Some(new_selection);

        Ok(())
    }
}

impl Command for InsertTextCommand {
    fn name(&self) -> &'static str {
        "insert_text"
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(text_len = self.text.len(), paragraph = self.paragraph_index)))]
    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let result = self.apply_edit(ctx);
        self.raw_xml_undo.finish(ctx, result.is_ok());
        result
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        // Single-paragraph text insertion/replacement is the common typing-undo
        // case: undo restores runs on exactly one paragraph and redo re-applies
        // within that same paragraph, with no change to the paragraph count. It
        // can therefore ride the same incremental dirty/seek/band-redraw path as
        // ordinary typing.
        //
        // Both `range` endpoints on the same paragraph proves the *redo*
        // direction stays single-paragraph (redo re-runs `apply`, which only
        // takes its single-paragraph branch when start.paragraph ==
        // end.paragraph). `previous_paragraphs.is_none()` proves the recorded
        // *undo* backup is single-paragraph. Requiring both keeps a
        // multi-paragraph replacement — which merges/splits paragraphs — on the
        // conservative full-render path.
        let single_paragraph_range = self.range.anchor.paragraph == self.range.focus.paragraph;
        if single_paragraph_range && self.previous_paragraphs.is_none() {
            HistoryRenderFootprint::SingleParagraph {
                index: self.paragraph_index,
            }
        } else {
            HistoryRenderFootprint::Structural
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(mut command) = self.note_range_deletion.take() {
            return command.undo(ctx);
        }
        if let Some(in_place) = self.in_place.take() {
            in_place.undo(ctx)?;
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        if let Some(snapshot) = self.previous_snapshot.take() {
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        if let Some(previous_paragraphs) = self.previous_paragraphs.take() {
            let raw_xml_undo = std::mem::take(&mut self.raw_xml_undo);
            let preserve_xml_bindings = raw_xml_undo.restores_xml();
            let paragraph_index = self.paragraph_index;
            raw_xml_undo.undo(ctx, |ctx| {
                restore_multi_paragraph_edit(
                    ctx,
                    paragraph_index,
                    &previous_paragraphs,
                    preserve_xml_bindings,
                )
            })?;
            // As after undoing a join: restored paragraph properties can bring
            // a section break back, and the paragraph list changed.
            if let Some(document) = ctx.document.document_mut() {
                crate::word_document::WordDocument::rebuild_section_ranges(document);
            }
            ctx.document.rebuild_paragraph_surfaces();
            *ctx.selection = self.previous_selection;
            return Ok(());
        }

        let paragraph = ctx
            .document
            .paragraph_mut(self.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let backup = self
            .previous_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        paragraph.runs = backup.clone();
        if let Some(anchors) = &self.previous_anchors {
            anchors.restore(paragraph);
        }
        ctx.document
            .sync_paragraph_text_preserving_drawings(self.paragraph_index);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

fn range_is_deleted_revision(runs: &[Run], start: usize, end: usize) -> bool {
    let mut position = 0;
    let mut covered = 0;
    for run in runs {
        let run_end = position + run.text.len();
        let overlap = end.min(run_end).saturating_sub(start.max(position));
        if overlap > 0 {
            if !run.revision.as_ref().is_some_and(|mark| {
                matches!(mark.kind, RevisionKind::Deleted | RevisionKind::MovedFrom)
            }) {
                return false;
            }
            covered += overlap;
        }
        position = run_end;
    }
    covered == end.saturating_sub(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::delete_text::{DeleteDirection, DeleteTextCommand};
    use crate::commands::text_edit::ZeroWidthRuns;
    use crate::commands::{CommandContext, CommandExecutor, HistoryRenderFootprint};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, RevisionKind, Run, RunFieldKind, RunProperties,
    };
    use crate::word_document::{RevisionSession, WordDocument};
    use crate::Options;
    use futures::executor;
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn build_docx_with_body(body_xml: &str) -> Vec<u8> {
        use zip::write::SimpleFileOptions;

        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
"#;
        const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"#;
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", RELS),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    fn exported_document_xml(word_doc: &mut WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut xml)
            .expect("read document.xml");
        xml
    }

    fn insert_text(word_doc: &mut WordDocument, paragraph: usize, offset: usize, text: &str) {
        let range = SelectionRange::collapsed(TextPosition::new(paragraph, offset));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(word_doc, &mut selection);
        executor
            .execute(
                Box::new(InsertTextCommand::new(range, text.to_string(), None)),
                &mut ctx,
            )
            .expect("insert text");
    }

    #[test]
    fn inserting_before_standalone_carriage_return_preserves_its_xml_element() {
        let data = std::fs::read(fixture_path("rich-paste-cr.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "Z");

        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(xml.matches("<w:cr/>").count(), 3, "{xml}");
        assert!(!xml.contains("<w:br w:type=\"textWrapping\"/>"), "{xml}");
        assert!(xml.contains("ZA"), "{xml}");
    }

    #[test]
    fn inserting_into_run_with_carriage_return_preserves_its_xml_element() {
        let data = build_docx_with_body(
            "<w:p><w:r><w:t>A</w:t><w:cr/><w:t>B</w:t><w:cr/><w:t>C</w:t></w:r></w:p>",
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "Z");

        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(xml.matches("<w:cr/>").count(), 2, "{xml}");
        assert!(!xml.contains("<w:br w:type=\"textWrapping\"/>"), "{xml}");
        assert!(xml.contains("ZA"), "{xml}");
    }

    #[test]
    fn inserting_before_fld_simple_preserves_empty_non_display_field() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>Visible body text for deployment.</w:t></w:r><w:fldSimple w:instr=" TC &quot;Deployment map&quot; \f i "><w:r/></w:fldSimple></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "X");

        let xml = exported_document_xml(&mut word_doc);
        assert!(
            xml.contains("<w:fldSimple"),
            "fldSimple must survive: {xml}"
        );
        assert!(
            xml.contains("TC &quot;Deployment map&quot;"),
            "field instruction must survive: {xml}"
        );
        assert!(
            xml.contains("XVisible body text for deployment."),
            "edited paragraph text must survive: {xml}"
        );
    }

    #[test]
    fn inserting_before_empty_result_fld_simple_keeps_following_run_element() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>ab</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r/></w:fldSimple><w:r><w:t>cd</w:t></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 2, "x");

        let xml = exported_document_xml(&mut word_doc);
        assert!(
            xml.contains(
                r#"<w:r><w:t>abx</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r/></w:fldSimple><w:r><w:t>cd</w:t></w:r></w:p>"#
            ),
            "{xml}"
        );
    }

    #[test]
    fn inserting_formatted_text_before_empty_result_fld_simple_keeps_field_in_place() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>ab</w:t></w:r><w:fldSimple w:instr=" PAGE "></w:fldSimple><w:r w:rsidR="00AB12CD"><w:t>cd</w:t></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::collapsed(TextPosition::new(0, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        let bold = RunProperties {
            bold: Some(true),
            ..Default::default()
        };
        executor
            .execute(
                Box::new(InsertTextCommand::new(range, "x".to_string(), Some(bold))),
                &mut ctx,
            )
            .expect("insert text");

        // The field keeps its own container after the new run (no second
        // PAGE field is authored), and "cd" keeps its element's attributes.
        let xml = exported_document_xml(&mut word_doc);
        assert!(
            xml.contains(
                r#"<w:r><w:rPr><w:b/></w:rPr><w:t>x</w:t></w:r><w:fldSimple w:instr=" PAGE "/><w:r w:rsidR="00AB12CD"><w:t>cd</w:t></w:r></w:p>"#
            ),
            "{xml}"
        );
        assert_eq!(xml.matches("<w:fldSimple").count(), 1, "{xml}");
    }

    #[test]
    fn inserting_before_complex_field_preserves_markers_and_cached_result() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t xml:space="preserve">Body title: </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DOCPROPERTY Title </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Stale Body Title</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "X");

        let xml = exported_document_xml(&mut word_doc);
        for marker in ["begin", "separate", "end"] {
            assert!(
                xml.contains(&format!(r#"w:fldCharType="{marker}""#)),
                "{marker} marker must survive: {xml}"
            );
        }
        assert!(
            xml.contains("Stale Body Title"),
            "cached field result must survive: {xml}"
        );
        assert!(
            xml.contains("XBody title: "),
            "inserted text must survive: {xml}"
        );
    }

    #[test]
    fn inserting_inside_fld_simple_updates_result_inside_field() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>Value: </w:t></w:r><w:fldSimple w:instr=" MERGEFIELD Name "><w:r><w:t>John Smith</w:t></w:r></w:fldSimple><w:r><w:t> suffix</w:t></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 11, "X");

        let xml = exported_document_xml(&mut word_doc);
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple body");
        assert!(
            field_body.contains('X'),
            "inserted text must stay in the field result: {field_body}"
        );
        assert!(
            !field_body.contains("suffix"),
            "following text must remain outside the field: {field_body}"
        );
    }

    #[test]
    fn typing_into_computed_field_only_paragraph_keeps_text_outside_field() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "x");

        let runs = &word_doc.paragraph(0).expect("paragraph").runs;
        let page = runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("PAGE field run");
        assert_eq!(page.text, "", "typed text must not enter the PAGE result");
        assert_eq!(runs[0].text, "x", "typed text must be its own run");
        assert!(runs[0].field.is_none());

        let xml = exported_document_xml(&mut word_doc);
        let paragraph = xml
            .split("<w:p>")
            .nth(1)
            .and_then(|tail| tail.split("</w:p>").next())
            .expect("exported paragraph");
        assert!(
            paragraph.starts_with(
                r#"<w:r><w:t>x</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r>"#
            ),
            "typed text must precede the field: {paragraph}"
        );
        assert!(
            paragraph.contains(r#"<w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/>"#),
            "PAGE result must stay the saved page number: {paragraph}"
        );
    }

    #[test]
    fn typing_into_computed_fld_simple_only_paragraph_keeps_text_outside_field() {
        let data = build_docx_with_body(
            r#"<w:p><w:fldSimple w:instr=" PAGE "><w:r><w:rPr><w:b/></w:rPr><w:t>1</w:t></w:r></w:fldSimple></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "x");

        let xml = exported_document_xml(&mut word_doc);
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple body");
        assert!(
            !field_body.contains('x') && field_body.contains("<w:t>1</w:t>"),
            "PAGE result must stay the saved page number: {field_body}"
        );
        assert!(
            xml.contains(r#"<w:r><w:rPr><w:b/></w:rPr><w:t>x</w:t></w:r><w:fldSimple"#),
            "typed text must precede the field with its formatting: {xml}"
        );
    }

    #[test]
    fn typing_into_hidden_fld_simple_only_paragraph_keeps_text_outside_field() {
        let data = build_docx_with_body(
            r#"<w:p><w:fldSimple w:instr=" SET a &quot;v&quot; "><w:r><w:t>v</w:t></w:r></w:fldSimple></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 0, 0, "x");

        let xml = exported_document_xml(&mut word_doc);
        assert!(
            xml.contains(r#"<w:r><w:t>x</w:t></w:r><w:fldSimple w:instr=" SET a &quot;v&quot; "><w:r><w:t>v</w:t></w:r></w:fldSimple>"#),
            "typed text must precede the SET field and leave its result alone: {xml}"
        );
    }

    /// The children of the exported body's first `w:p`.
    fn exported_first_paragraph(word_doc: &mut WordDocument) -> String {
        let xml = exported_document_xml(word_doc);
        xml.split("<w:p>")
            .nth(1)
            .and_then(|tail| tail.split("</w:p>").next())
            .expect("exported paragraph")
            .to_string()
    }

    #[test]
    fn typing_into_empty_result_fld_simple_only_paragraph_keeps_one_field() {
        // The reader models no run for `<w:r/>`, so the field run has no node
        // id to find its `w:fldSimple` by. The typed run must not carry the
        // container ahead of itself, and the field run must reuse it rather
        // than author a second, dirty field.
        for field in [
            r#"<w:fldSimple w:instr=" PAGE "><w:r/></w:fldSimple>"#,
            r#"<w:fldSimple w:instr=" SET a &quot;v&quot; "><w:r/></w:fldSimple>"#,
            r#"<w:fldSimple w:instr=" XE &quot;term&quot; "><w:r/></w:fldSimple>"#,
        ] {
            let data = build_docx_with_body(&format!("<w:p>{field}</w:p>"));
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");

            insert_text(&mut word_doc, 0, 0, "x");
            assert_eq!(
                exported_first_paragraph(&mut word_doc),
                format!("<w:r><w:t>x</w:t></w:r>{field}"),
                "typed text must precede the one saved field"
            );

            // The next edit finds the same field again.
            insert_text(&mut word_doc, 0, 1, "y");
            assert_eq!(
                exported_first_paragraph(&mut word_doc),
                format!("<w:r><w:t>xy</w:t></w:r>{field}")
            );
        }
    }

    #[test]
    fn editing_paragraph_with_empty_result_fld_simple_keeps_each_field_once() {
        const PAGE: &str = r#"<w:fldSimple w:instr=" PAGE "><w:r/></w:fldSimple>"#;
        let load = |body: &str| {
            executor::block_on(WordDocument::load(
                &build_docx_with_body(body),
                &Options::default(),
            ))
            .expect("load docx")
        };

        // The field run has no node id even when the edit is elsewhere.
        let mut word_doc = load(&format!("<w:p><w:r><w:t>ab</w:t></w:r>{PAGE}</w:p>"));
        insert_text(&mut word_doc, 0, 2, "x");
        assert_eq!(
            exported_first_paragraph(&mut word_doc),
            format!("<w:r><w:t>abx</w:t></w:r>{PAGE}")
        );

        // Identical empty fields are not fragments of one field: each keeps
        // its own container.
        let mut word_doc = load(&format!("<w:p>{PAGE}{PAGE}</w:p>"));
        insert_text(&mut word_doc, 0, 0, "x");
        assert_eq!(
            exported_first_paragraph(&mut word_doc),
            format!("<w:r><w:t>x</w:t></w:r>{PAGE}{PAGE}")
        );

        // A run split off ahead of the field has no node id either.
        let mut word_doc = load(&format!("<w:p><w:r><w:t>ab</w:t></w:r>{PAGE}</w:p>"));
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1));
        let mut selection = Some(range);
        CommandExecutor::new()
            .execute(
                Box::new(crate::commands::apply_bold::ApplyBoldCommand::new(range)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect("apply bold");
        assert_eq!(
            exported_first_paragraph(&mut word_doc),
            format!(
                "<w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t>a</w:t></w:r><w:r><w:t>b</w:t></w:r>{PAGE}"
            )
        );
    }

    #[test]
    fn typing_into_editable_field_only_paragraph_edits_field_result() {
        let data = build_docx_with_body(
            r#"<w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>Target</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> REF target </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t></w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        insert_text(&mut word_doc, 1, 0, "x");

        let runs = &word_doc.paragraph(1).expect("paragraph").runs;
        let reference = runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("REF field run");
        assert!(matches!(
            reference.field.as_ref().map(|field| &field.kind),
            Some(RunFieldKind::Ref { .. })
        ));
        assert_eq!(reference.text, "xTarget");
        assert!(
            runs.iter()
                .filter(|run| run.field.is_none())
                .all(|run| run.text.is_empty()),
            "typing must not split off a run outside the REF result: {runs:?}"
        );

        let xml = exported_document_xml(&mut word_doc);
        assert!(
            xml.contains(r#"<w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>xTarget</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/>"#),
            "typed text must edit the REF result: {xml}"
        );
    }

    fn document_xml_from_docx(data: &[u8]) -> String {
        let mut archive = ZipArchive::new(std::io::Cursor::new(data)).expect("docx archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        document_xml
    }

    fn sample_document(text: &str) -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![Run {
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
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    fn sample_document_with_runs() -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![
                Run {
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
                },
                Run {
                    text: " world".to_string(),
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
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    fn sample_two_paragraph_document() -> WordDocument {
        let first = Paragraph {
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
        };
        let second = Paragraph {
            runs: vec![Run {
                text: "World".to_string(),
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
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(first),
                    BodyElement::from_paragraph(second),
                ],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn deleting_whole_paragraphs_with_their_marks_keeps_the_next_paragraph_formatting() {
        // Triple-click selects a paragraph with its mark (Word); Delete then
        // removes the paragraph, and the following heading keeps its style.
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:bookmarkStart w:id="1" w:name="mark"/><w:r><w:t>Title</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p><w:p><w:r><w:t>Lead</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>第1条</w:t></w:r></w:p>"#,
            ),
            &Options::default(),
        ))
        .expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(2, 0));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, String::new(), None)),
                    &mut ctx,
                )
                .expect("delete paragraphs");
        }
        assert_eq!(word_doc.paragraph_count(), 1);
        let paragraph = word_doc.paragraph(0).expect("kept paragraph");
        assert_eq!(paragraph_len(paragraph), "第1条".len());
        assert_eq!(
            paragraph
                .properties
                .as_ref()
                .and_then(|props| props.style.as_deref()),
            Some("Heading1")
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
        let xml = exported_document_xml(&mut word_doc);
        assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#), "{xml}");
        assert!(!xml.contains("Title"), "{xml}");
        assert!(!xml.contains(r#"<w:jc w:val="center"/>"#), "{xml}");
        assert_eq!(xml.matches(r#"w:id="1""#).count(), 2, "{xml}");

        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo"));
        assert_eq!(word_doc.paragraph_count(), 3);
        let xml = exported_document_xml(&mut word_doc);
        assert!(xml.contains("Title"), "{xml}");
    }

    /// A tracked replacement across sibling paragraphs and a replacement
    /// across table cells rewrite their paragraphs without removing any, so
    /// undo keeps those paragraphs and a journal of the XML instead of a copy
    /// of the whole document, and still restores it exactly.
    #[test]
    fn in_place_replacements_undo_without_a_document_snapshot() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let load = || {
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load fixture")
        };
        let session = || {
            Some(RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-10-05T00:00:00Z".into()),
                date_utc: None,
            })
        };
        let mut cases = Vec::new();
        let mut doc = load();
        doc.set_revision_session(session());
        cases.push((
            doc,
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3)),
        ));
        for tracking in [false, true] {
            let mut doc = load();
            doc.insert_table_after_paragraph(0, 1, 2, &[])
                .expect("insert table");
            for (paragraph, text) in [(1usize, "AB"), (2usize, "CD")] {
                let range = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
                let mut selection = Some(range);
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                InsertTextCommand::new(range, text.to_string(), None)
                    .apply(&mut ctx)
                    .expect("fill cell");
            }
            if tracking {
                doc.set_revision_session(session());
            }
            cases.push((
                doc,
                SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(2, 1)),
            ));
        }

        for (case, (mut doc, range)) in cases.into_iter().enumerate() {
            let original = doc.export_docx().expect("export");
            let original_model = format!("{:?}", doc.document);
            let revision_id = doc.revision_id_counter();
            let mut command = InsertTextCommand::new(range, "X".into(), None);
            let mut selection = Some(range);
            let mut applied = None;
            for round in ["apply", "redo"] {
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.apply(&mut ctx).expect(round);
                }
                assert!(command.in_place.is_some(), "case {case} {round}: in place");
                assert!(command.previous_snapshot.is_none(), "case {case} {round}");
                let exported = doc.export_docx().expect("export");
                assert!(exported != original, "case {case}: {round} changes it");
                match &applied {
                    None => applied = Some(exported),
                    Some(first) => assert!(exported == *first, "case {case}: same redo"),
                }
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.undo(&mut ctx).expect("undo");
                }
                assert!(
                    doc.export_docx().expect("export") == original,
                    "case {case}: {round} undone saves the original bytes"
                );
                assert_eq!(
                    format!("{:?}", doc.document),
                    original_model,
                    "case {case}: {round} undone"
                );
                assert_eq!(doc.revision_id_counter(), revision_id, "case {case}");
            }
        }
    }

    #[test]
    fn replacing_a_range_across_cells_clears_them_and_keeps_every_paragraph() {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hello.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load fixture");
        word_doc
            .insert_table_after_paragraph(0, 1, 2, &[])
            .expect("insert table");
        let mut executor = CommandExecutor::new();
        for (paragraph, text) in [(1usize, "AB"), (2usize, "CD")] {
            let range = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
            let mut selection = Some(range);
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, text.to_string(), None)),
                    &mut ctx,
                )
                .expect("fill cell");
        }
        let before = word_doc.paragraph_count();
        let range = SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(2, 1));
        let selection = {
            let mut selection = Some(range);
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "X".to_string(), None)),
                    &mut ctx,
                )
                .expect("replace across cells");
            selection
        };
        let text = |doc: &WordDocument, index: usize| -> String {
            doc.paragraph(index)
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default()
        };
        assert_eq!(
            word_doc.paragraph_count(),
            before,
            "every cell keeps its paragraph"
        );
        assert_eq!(text(&word_doc, 1), "AX");
        assert_eq!(text(&word_doc, 2), "D");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(1, 2)))
        );
        {
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(word_doc.paragraph_count(), before);
        assert_eq!(text(&word_doc, 1), "AB");
        assert_eq!(text(&word_doc, 2), "CD");
    }

    #[test]
    fn replacing_across_a_cell_preserves_existing_deletion() {
        let data = std::fs::read(fixture_path("numbering-with-review.docx")).unwrap();
        for tracking in [false, true] {
            let mut doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
            doc.insert_table_after_paragraph(0, 1, 1, &[]).unwrap();
            apply_text_edit(
                &mut doc.paragraph_mut(1).unwrap().runs,
                0,
                0,
                "CD",
                &ZeroWidthRuns::default(),
            );
            doc.sync_paragraph_text_preserving_drawings(1);
            if tracking {
                doc.set_revision_session(Some(RevisionSession {
                    author: Some("reviewer".into()),
                    date: Some("2026-09-24T00:00:00Z".into()),
                    date_utc: None,
                }));
            }
            let count = doc.paragraph_count();
            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 1));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, "X".into(), None)),
                        &mut ctx,
                    )
                    .unwrap();
            }
            assert_eq!(doc.paragraph_count(), count);
            let cell = doc.paragraph(1).unwrap();
            assert_eq!(
                cell.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                if tracking { "CD" } else { "D" }
            );
            if tracking {
                assert!(cell.runs.iter().any(|run| run.text == "C"
                    && run
                        .revision
                        .as_ref()
                        .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))));
            }
            let paragraph = doc.paragraph(0).unwrap();
            assert_eq!(
                paragraph
                    .runs
                    .iter()
                    .filter(|run| run
                        .revision
                        .as_ref()
                        .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted)))
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                "Hello"
            );
            assert!(paragraph
                .runs
                .iter()
                .any(|run| run.text == "X" && (run.revision.is_some() == tracking)));
            let exported = doc.export_docx().unwrap();
            let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            assert!(xml.contains("<w:delText>Hello</w:delText>"));
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
            assert_eq!(
                doc.paragraph(0)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                "Hello"
            );
            assert_eq!(
                doc.paragraph(1)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                "CD"
            );
        }
    }

    #[test]
    fn inserts_text_at_caret() {
        let mut word_doc = sample_document("Hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        " world".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = match word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .unwrap()
        {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.first().unwrap().text, "Hello world");

        let new_focus = selection.unwrap().focus;
        assert_eq!(new_focus.paragraph, 0);
        assert_eq!(new_focus.offset, 11);
    }

    #[test]
    fn inserting_text_does_not_materialize_inherited_run_font_properties() {
        let data = std::fs::read(fixture_path("2col.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "X".to_string(), None)),
                    &mut ctx,
                )
                .expect("insert text");
        }

        let document_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw document")
            .to_string()
            .expect("serialize document");
        assert!(
            !document_xml.contains("<w:rPr"),
            "inherited font properties must not become direct formatting"
        );
    }

    #[test]
    fn inserting_text_keeps_authored_run_font_property_counts() {
        let data = std::fs::read(fixture_path("yokohama_table.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before = word_doc
            .raw_document
            .as_ref()
            .expect("raw document")
            .to_string()
            .expect("serialize document");
        let range = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "X".to_string(), None)),
                    &mut ctx,
                )
                .expect("insert text");
        }

        let after = word_doc
            .raw_document
            .as_ref()
            .expect("raw document")
            .to_string()
            .expect("serialize document");
        for tag in ["w:rFonts", "w:kern", "w:sz", "w:szCs", "w:w"] {
            assert_eq!(
                before.matches(&format!("<{tag}")).count(),
                after.matches(&format!("<{tag}")).count(),
                "editing must not add or remove {tag} elements"
            );
        }
    }

    #[test]
    fn insertion_rebases_comment_ranges_and_markers() {
        let mut word_doc = sample_document("abcdef");
        let paragraph = word_doc.paragraph_mut(0).expect("paragraph");
        paragraph.comment_ranges = vec![crate::document::CommentRange {
            id: "1".to_string(),
            start: 2,
            end: 5,
        }];
        paragraph.comment_markers = vec![crate::document::CommentMarker {
            id: "1".to_string(),
            position: 5,
        }];
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 1)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 1)),
                        "XY".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("insert text");
        }

        let paragraph = word_doc.paragraph(0).expect("paragraph");
        assert_eq!(
            (
                paragraph.comment_ranges[0].start,
                paragraph.comment_ranges[0].end
            ),
            (4, 7)
        );
        assert_eq!(paragraph.comment_markers[0].position, 7);
    }

    #[test]
    fn replaces_selected_range_with_text() {
        let mut word_doc = sample_document("abcdef");
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 2),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(0, 5)),
                        "XYZ".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = match word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .unwrap()
        {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.first().unwrap().text, "abXYZf");
        let new_focus = selection.unwrap().focus;
        assert_eq!(new_focus.offset, 5);
    }

    #[test]
    fn undo_restores_previous_runs_and_selection() {
        let mut word_doc = sample_document("one two");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 7)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 3)),
                        "-".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(
            selection.unwrap().focus.offset,
            4,
            "caret should advance after insertion"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).unwrap();
        }

        let paragraph = match word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .unwrap()
        {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.first().unwrap().text, "one two");
        assert_eq!(
            selection.unwrap().focus.offset,
            7,
            "caret should return to its original position after undo"
        );
    }

    #[test]
    fn insert_syncs_raw_document_xml() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        "!".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let raw = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml");
        assert!(raw.contains("Hello!"));
    }

    #[test]
    fn insert_in_review_document_creates_new_revision_wrapper() {
        let data = std::fs::read(fixture_path("review.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("bokuweb".to_string()),
            date: Some("2025-11-16T19:17:00Z".to_string()),
            date_utc: Some("2025-11-16T10:17:00Z".to_string()),
        }));

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 3)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 3)),
                        "l".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document xml");

        let expected_without_run_props = r#"<w:ins w:id="2" w:author="bokuweb" w:date="2025-11-16T19:17:00Z" w16du:dateUtc="2025-11-16T10:17:00Z"><w:r><w:t>l</w:t></w:r></w:ins>"#;
        let expected_with_run_props_prefix = r#"<w:ins w:id="2" w:author="bokuweb" w:date="2025-11-16T19:17:00Z" w16du:dateUtc="2025-11-16T10:17:00Z"><w:r><w:rPr>"#;
        let expected_with_run_props_suffix = r#"<w:t>l</w:t></w:r></w:ins>"#;
        assert!(
            document_xml.contains(expected_without_run_props)
                || (document_xml.contains(expected_with_run_props_prefix)
                    && document_xml.contains(expected_with_run_props_suffix)),
            "exported XML should contain new revision wrapper for inserted text: {}",
            document_xml
        );
        let existing_without_run_props = r#"<w:ins w:id="0" w:author="植木 智之" w:date="2025-11-12T19:17:00Z" w16du:dateUtc="2025-11-12T10:17:00Z"><w:r><w:t>Hel</w:t></w:r></w:ins>"#;
        let existing_with_run_props_prefix = r#"<w:ins w:id="0" w:author="植木 智之" w:date="2025-11-12T19:17:00Z" w16du:dateUtc="2025-11-12T10:17:00Z"><w:r><w:rPr>"#;
        let existing_with_run_props_suffix = r#"<w:t>Hel</w:t></w:r></w:ins>"#;
        assert!(
            document_xml.contains(existing_without_run_props)
                || (document_xml.contains(existing_with_run_props_prefix)
                    && document_xml.contains(existing_with_run_props_suffix)),
            "existing revision should be split before the inserted run"
        );
        assert!(
            !document_xml.contains("<w:t xml:space=\"preserve\">Hellllo </w:t>"),
            "inserted text should not appear outside revision wrapper"
        );
    }

    #[test]
    fn typing_at_a_tracked_deletion_does_not_change_deleted_text() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        for offset in [0, 2, 5] {
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, "!".to_string(), None)),
                        &mut ctx,
                    )
                    .expect("insert beside deletion");
            }

            let paragraph = word_doc.paragraph(0).expect("first paragraph");
            let deleted: String = paragraph
                .runs
                .iter()
                .filter(|run| {
                    run.revision
                        .as_ref()
                        .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
                })
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(deleted, "Hello", "offset {offset}");
            assert!(paragraph
                .runs
                .iter()
                .any(|run| run.text == "!" && run.revision.is_none()));

            let exported = word_doc.export_docx().expect("export docx");
            let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .expect("document.xml")
                .read_to_string(&mut xml)
                .expect("read XML");
            assert!(!xml.contains("<w:delText>!"), "offset {offset}: {xml}");
            assert!(xml.contains("<w:t>!</w:t>"), "offset {offset}: {xml}");
            assert!(
                xml.contains("w:rsidDel=\"00147987\""),
                "offset {offset}: original deletion run attributes must survive: {xml}"
            );
        }
    }

    #[test]
    fn replacing_selected_tracked_deletion_preserves_original_revision() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "!".to_string(), None)),
                    &mut ctx,
                )
                .expect("replace selected deletion");
        }

        let paragraph = word_doc.paragraph(0).expect("first paragraph");
        let deleted: String = paragraph
            .runs
            .iter()
            .filter(|run| {
                run.revision
                    .as_ref()
                    .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
            })
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(deleted, "Hello");
        assert!(paragraph
            .runs
            .iter()
            .any(|run| run.text == "!" && run.revision.is_none()));

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .expect("read XML");
        assert!(xml.contains("<w:delText>He</w:delText>"), "{xml}");
        assert!(xml.contains("<w:delText>llo</w:delText>"), "{xml}");
        assert!(xml.contains("<w:t>!</w:t>"));
        assert_eq!(selection.map(|range| range.focus.offset), Some(3));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo replacement");
        }
        assert_eq!(selection, Some(range));
        let deleted_after_undo: String = word_doc
            .paragraph(0)
            .expect("first paragraph")
            .runs
            .iter()
            .filter(|run| {
                run.revision
                    .as_ref()
                    .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
            })
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(deleted_after_undo, "Hello");
    }

    #[test]
    fn replacing_mixed_deleted_and_plain_text_keeps_deleted_revision() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let paragraph = word_doc.paragraph_mut(0).expect("first paragraph");
        apply_text_edit(&mut paragraph.runs, 5, 5, "next", &ZeroWidthRuns::default());
        word_doc.sync_paragraph_text_preserving_drawings(0);

        let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(0, 7));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "!".to_string(), None)),
                    &mut ctx,
                )
                .expect("replace across deletion and ordinary text");
        }
        let paragraph = word_doc.paragraph(0).expect("first paragraph");
        let deleted: String = paragraph
            .runs
            .iter()
            .filter(|run| {
                run.revision
                    .as_ref()
                    .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
            })
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(deleted, "Hello");
        let ordinary: String = paragraph
            .runs
            .iter()
            .filter(|run| run.revision.is_none())
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(ordinary, "!xt");
    }

    #[test]
    fn replacing_across_both_sides_of_deleted_revision_preserves_review_history() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        for track_changes in [false, true] {
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            {
                let paragraph = word_doc.paragraph_mut(0).expect("first paragraph");
                apply_text_edit(&mut paragraph.runs, 0, 0, "A", &ZeroWidthRuns::default());
                apply_text_edit(&mut paragraph.runs, 6, 6, "next", &ZeroWidthRuns::default());
            }
            word_doc.sync_paragraph_text_preserving_drawings(0);
            if track_changes {
                word_doc.set_revision_session(Some(RevisionSession {
                    author: Some("reviewer".to_string()),
                    date: Some("2026-09-24T00:00:00Z".to_string()),
                    date_utc: None,
                }));
            }

            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 8));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, "!".to_string(), None)),
                        &mut ctx,
                    )
                    .expect("replace around deleted revision");
            }

            let paragraph = word_doc.paragraph(0).expect("first paragraph");
            let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(
                text,
                if track_changes {
                    "AHellone!xt"
                } else {
                    "Hello!xt"
                },
                "track changes: {track_changes}",
            );
            let deleted: String = paragraph
                .runs
                .iter()
                .filter(|run| {
                    run.revision
                        .as_ref()
                        .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
                })
                .map(|run| run.text.as_str())
                .collect();
            assert!(deleted.contains("Hello"), "track changes: {track_changes}");
            assert!(paragraph.runs.iter().any(|run| {
                run.text == "!"
                    && run
                        .revision
                        .as_ref()
                        .is_some_and(|mark| matches!(mark.kind, RevisionKind::Inserted))
                        == track_changes
            }));

            let exported = word_doc.export_docx().expect("export docx");
            let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .expect("document.xml")
                .read_to_string(&mut xml)
                .expect("read XML");
            assert!(xml.contains("<w:delText>Hello</w:delText>"), "{xml}");
            assert!(xml.contains("<w:t>!</w:t>"), "{xml}");
        }
    }

    #[test]
    fn replacing_across_paragraphs_preserves_selected_deleted_revision() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>AB</w:t></w:r></w:p>
            <w:p><w:del w:id="7" w:author="reviewer"><w:r><w:delText>Hello</w:delText></w:r></w:del><w:r><w:t>tail</w:t></w:r></w:p>
            <w:p><w:r><w:t>CD</w:t></w:r></w:p>"#,
        );
        {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            let range = SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(2, 1));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, "X".into(), None)),
                        &mut ctx,
                    )
                    .expect("replace");
            }
            assert_eq!(word_doc.paragraph_count(), 1);
            let paragraph = word_doc.paragraph(0).expect("merged paragraph");
            assert_eq!(
                paragraph
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>(),
                "AHelloXD"
            );
            assert!(paragraph.runs.iter().any(|run| {
                run.text == "Hello"
                    && run.revision.as_ref().is_some_and(|mark| {
                        mark.kind == RevisionKind::Deleted
                            && mark.id.as_deref() == Some("7")
                            && mark.author.as_deref() == Some("reviewer")
                    })
            }));
            let xml = exported_document_xml(&mut word_doc);
            assert!(xml.contains("<w:delText>Hello</w:delText>"), "{xml}");
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor.undo(&mut ctx).expect("undo");
            }
            assert_eq!(word_doc.paragraph_count(), 3);
            assert_eq!(
                word_doc.paragraph(1).expect("review paragraph").runs[0].text,
                "Hello"
            );
        }
    }

    #[test]
    fn tracked_paragraph_replacement_records_reviewable_text_and_break() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("reviewer".into()),
            date: None,
            date_utc: None,
        }));
        let before = exported_document_xml(&mut word_doc);
        let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(InsertTextCommand::new(range, "X".into(), None)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect("record replacement and paragraph mark");
        assert_eq!(word_doc.paragraph_count(), 2);
        let exported = word_doc.export_docx().expect("export");
        let xml = exported_document_xml(&mut word_doc);
        assert!(xml.contains("<w:del "), "{xml}");
        assert!(xml.contains("<w:delText>llo</w:delText>"), "{xml}");
        assert!(xml.contains("<w:delText>Wor</w:delText>"), "{xml}");
        assert!(xml.contains("<w:t>X</w:t>"), "{xml}");
        let mut accepted = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reload acceptance copy");
        assert_eq!(accepted.accept_revisions(None, None, None, None), Ok(4));
        assert_eq!(accepted.paragraph_count(), 1);
        assert_eq!(
            accepted
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "HeXld"
        );
        let mut rejected = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reload rejection copy");
        assert_eq!(rejected.reject_revisions(None, None, None, None), Ok(4));
        assert_eq!(rejected.paragraph_count(), 2);
        assert_eq!(
            rejected
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Hello"
        );
        assert_eq!(
            rejected
                .paragraph(1)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "World"
        );
        executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo");
        assert_eq!(selection, Some(range));
        assert_eq!(exported_document_xml(&mut word_doc), before);
    }

    /// Typing over a selection while change tracking is on is a redline: Word
    /// strikes the old text through and puts the new text beside it. Dropping
    /// the old text would hand the reviewer a document that no longer says what
    /// was taken out — and undo has to put the paragraph back either way.
    #[test]
    fn replacing_a_range_while_tracking_keeps_the_old_text_as_a_deletion() {
        let mut word_doc = sample_document("alpha beta gamma");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("AI".to_string()),
            date: Some("2026-08-01T00:00:00Z".to_string()),
            date_utc: None,
        }));

        let range = SelectionRange::new(
            TextPosition::new(0, 6),  // "beta"
            TextPosition::new(0, 10), //
        );
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "delta".to_string(), None)),
                    &mut ctx,
                )
                .expect("tracked replacement");
        }

        let paragraph = word_doc.paragraph(0).expect("paragraph");
        let deleted: String = paragraph
            .runs
            .iter()
            .filter(|run| {
                run.revision
                    .as_ref()
                    .is_some_and(|mark| matches!(mark.kind, RevisionKind::Deleted))
            })
            .map(|run| run.text.as_str())
            .collect();
        let inserted: String = paragraph
            .runs
            .iter()
            .filter(|run| {
                run.revision
                    .as_ref()
                    .is_some_and(|mark| matches!(mark.kind, RevisionKind::Inserted))
            })
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(deleted, "beta", "the replaced text must be marked deleted");
        assert_eq!(inserted, "delta", "the new text must be marked inserted");
        // The insertion follows the struck-out text, as Word writes it.
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "alpha betadelta gamma");
        assert_eq!(
            selection.map(|range| range.focus.offset),
            Some(15),
            "the caret belongs after the inserted text, not inside the deletion",
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo");
        }
        let text: String = word_doc
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(text, "alpha beta gamma");
    }

    #[test]
    fn insert_at_paragraph_start_creates_revision_wrapper() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut options = Options::default();
        options.set_record_changes(true);
        options.set_revision_author(Some("jade tester".to_string()));

        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &options)).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 0)),
                        "X".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document xml");

        assert!(
            document_xml.contains(r#"<w:ins "#),
            "exported XML should contain insertion wrapper: {document_xml}"
        );
        assert!(
            document_xml.contains("<w:t>X</w:t>"),
            "inserted character should be wrapped inside revision markup: {document_xml}"
        );
    }

    #[test]
    fn load_with_record_changes_option_wraps_inserted_text_in_revision() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read hello fixture");
        let mut options = Options::default();
        options.set_record_changes(true);
        options.set_revision_author(Some("jade tester".to_string()));

        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &options)).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        "!".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("insert text succeeds");
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document xml");

        assert!(
            document_xml.contains(r#"<w:ins "#),
            "exported XML should contain an insertion revision wrapper: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"w:author="jade tester""#),
            "insertion revision should include provided author metadata: {document_xml}"
        );
        assert!(
            document_xml.contains("<w:t>!</w:t>"),
            "inserted character should be wrapped in revision markup: {document_xml}"
        );
    }

    #[test]
    fn insert_soft_break_syncs_br_node() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        "\n".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let raw = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml");
        assert!(
            raw.contains(r#"<w:br w:type="textWrapping"/>"#),
            "soft break should be serialized with w:br node"
        );
    }

    #[test]
    fn insert_page_break_syncs_br_node() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        crate::document::PAGE_BREAK_STR.to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let raw = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml");
        assert!(
            raw.contains(r#"<w:br w:type="page"/>"#),
            "page break should be serialized with w:br node"
        );
    }

    #[test]
    fn preserves_existing_runs_when_inserting_inside_first_run() {
        let mut word_doc = sample_document_with_runs();
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        ",".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = match word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .unwrap()
        {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.len(), 2);
        assert_eq!(paragraph.runs[0].text, "Hello,");
        assert_eq!(paragraph.runs[1].text, " world");
    }

    #[test]
    fn merges_paragraphs_when_selection_spans_multiple_blocks() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 5),
            TextPosition::new(1, 5),
        ));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 5)),
                        " ".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 1);
        let paragraph = word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .expect("paragraph missing");
        assert_eq!(paragraph.runs.len(), 1);
        assert_eq!(paragraph.runs[0].text, "Hello ");
    }

    /// Deleting whole selected paragraphs undoes through paragraph backups and
    /// an XML journal, not a snapshot of the whole document; undo restores
    /// the saved bytes and the paragraphs stay bound to their XML.
    #[test]
    fn deleting_whole_paragraphs_undoes_without_a_document_snapshot() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        let mut doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        let k = (0..doc.paragraph_count().saturating_sub(3))
            .find(|&k| {
                (k..=k + 2).all(|index| {
                    doc.paragraph(index).is_some_and(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.len())
                            .sum::<usize>()
                            >= 2
                            && paragraph_mark_is_removable(paragraph)
                    }) && doc.paragraphs_share_a_container(k, index)
                })
            })
            .expect("three neighbouring body paragraphs with text");
        let count = doc.paragraph_count();
        let before = doc.export_docx().expect("export");
        let model = |doc: &WordDocument| {
            format!(
                "{:?}",
                (k..=k + 2)
                    .map(|index| doc.paragraph(index))
                    .collect::<Vec<_>>()
            )
        };
        let before_model = model(&doc);

        let range = SelectionRange::new(
            doc.model_to_rendered_text_position(TextPosition::new(k, 0)),
            doc.model_to_rendered_text_position(TextPosition::new(k + 2, 0)),
        );
        let mut command = InsertTextCommand::new(range, String::new(), None);
        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            command.apply(&mut ctx).expect("delete whole paragraphs");
        }
        assert_eq!(doc.paragraph_count(), count - 2);
        assert!(command.previous_snapshot.is_none(), "no document snapshot");
        assert!(command.previous_paragraphs.is_some());
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            command.undo(&mut ctx).expect("undo");
        }
        assert_eq!(doc.paragraph_count(), count);
        assert_eq!(model(&doc), before_model);
        assert!(
            doc.export_docx().expect("export") == before,
            "undo exports the same bytes"
        );

        for index in k..=k + 2 {
            let at = doc.model_to_rendered_text_position(TextPosition::new(index, 0));
            let mut insert = InsertTextCommand::new(
                SelectionRange::collapsed(at),
                format!("段落削除の後{index}"),
                None,
            );
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            insert.apply(&mut ctx).expect("type");
        }
        let exported = doc.export_docx().expect("export");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .expect("read");
        for index in k..=k + 2 {
            assert!(
                xml.contains(&format!("段落削除の後{index}")),
                "paragraph {index}"
            );
        }
    }

    #[test]
    fn undo_restores_multi_paragraph_replacement_without_snapshot() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection =
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3)),
                        "X".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 1);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 2);
        let first = word_doc.paragraph(0).expect("first paragraph");
        let second = word_doc.paragraph(1).expect("second paragraph");
        assert_eq!(first.runs.len(), 1);
        assert_eq!(second.runs.len(), 1);
        assert_eq!(first.runs[0].text, "Hello");
        assert_eq!(second.runs[0].text, "World");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn undo_multi_paragraph_replacement_preserves_raw_paragraph_and_run_properties() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="FirstStyle"/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p>
    <w:p><w:pPr><w:pStyle w:val="SecondStyle"/><w:keepNext/></w:pPr><w:r><w:rPr><w:rStyle w:val="Emphasis"/><w:lang w:val="fr-FR"/><w:kern w:val="24"/></w:rPr><w:t>World</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let original_selection =
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        original_selection,
                        "X".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("replace across paragraphs");
            executor.undo(&mut ctx).expect("undo replacement");
        }

        let restored = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert!(
            restored.contains(r#"<w:pStyle w:val="SecondStyle"/>"#),
            "{restored}"
        );
        assert!(restored.contains("<w:keepNext/>"), "{restored}");
        assert!(
            restored.contains(r#"<w:rStyle w:val="Emphasis"/>"#),
            "{restored}"
        );
        assert!(
            restored.contains(r#"<w:lang w:val="fr-FR"/>"#),
            "{restored}"
        );
        assert!(restored.contains(r#"<w:kern w:val="24"/>"#), "{restored}");
    }

    #[test]
    fn multi_paragraph_replacement_moves_tail_comment_anchors_to_merged_paragraph() {
        let mut word_doc = sample_two_paragraph_document();
        let second = word_doc.paragraph_mut(1).expect("second paragraph");
        second.comment_ranges = vec![crate::document::CommentRange {
            id: "tail".to_string(),
            start: 3,
            end: 5,
        }];
        second.comment_markers = vec![crate::document::CommentMarker {
            id: "tail".to_string(),
            position: 5,
        }];
        let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "X".to_string(), None)),
                    &mut ctx,
                )
                .expect("replace paragraphs");
        }

        let merged = word_doc.paragraph(0).expect("merged paragraph");
        assert_eq!(
            merged
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "HeXrld"
        );
        assert_eq!(merged.comment_ranges.len(), 1);
        assert_eq!(
            (merged.comment_ranges[0].start, merged.comment_ranges[0].end),
            (4, 6)
        );
        assert_eq!(merged.comment_markers[0].position, 6);
    }

    #[test]
    fn redo_reapplies_multi_paragraph_replacement_after_undo() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection =
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3)),
                        "X".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
            assert!(executor.redo(&mut ctx).unwrap());
        }

        assert_eq!(word_doc.paragraph_count(), 1);
        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "HeXld");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 3);
    }

    #[test]
    fn inserting_soft_break_creates_dedicated_run() {
        let mut word_doc = sample_document("HelloWorld");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        "\n".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = match word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .unwrap()
        {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 3);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, "\n");
        assert_eq!(paragraph.runs[2].text, "World");

        let focus = selection.unwrap().focus;
        assert_eq!(focus.offset, 6);
    }

    #[test]
    fn inserted_text_applies_inline_properties() {
        let mut word_doc = sample_document("HelloWorld");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        let bold_props = RunProperties {
            bold: Some(true),
            ..Default::default()
        };

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        "!".to_string(),
                        Some(bold_props.clone()),
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .expect("paragraph missing");

        assert_eq!(paragraph.runs.len(), 3);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, "!");
        assert_eq!(paragraph.runs[2].text, "World");
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            Some(true),
            "inserted run should inherit inline bold property"
        );
        assert_ne!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            Some(true),
            "preceding run should remain unaffected"
        );
    }

    #[test]
    fn single_paragraph_insert_reports_single_paragraph_footprint() {
        // A collapsed-caret insert stays in one paragraph, so undo/redo can take
        // the incremental render fast path.
        let mut word_doc = sample_document("Hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut command = InsertTextCommand::new(
            SelectionRange::collapsed(TextPosition::new(0, 5)),
            " world".to_string(),
            None,
        );
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            command.apply(&mut ctx).unwrap();
        }
        assert_eq!(
            command.history_render_footprint(),
            HistoryRenderFootprint::SingleParagraph { index: 0 },
        );
    }

    #[test]
    fn multi_paragraph_replacement_reports_structural_footprint() {
        // A range spanning two paragraphs merges them; undo splits them back.
        // That reflow must stay on the conservative full-render path.
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 2),
            TextPosition::new(1, 3),
        ));
        let mut command = InsertTextCommand::new(
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3)),
            "X".to_string(),
            None,
        );
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            command.apply(&mut ctx).unwrap();
        }
        assert_eq!(
            command.history_render_footprint(),
            HistoryRenderFootprint::Structural,
        );
    }

    #[test]
    fn footprint_is_stable_before_apply_for_redo() {
        // `history_render_footprint` is queried before undo/redo runs. For a
        // single-paragraph range it must classify as SingleParagraph even from
        // the freshly-constructed command (redo re-runs `apply`, which stays
        // single-paragraph for a same-paragraph range).
        let command = InsertTextCommand::new(
            SelectionRange::collapsed(TextPosition::new(2, 4)),
            "abc".to_string(),
            None,
        );
        assert_eq!(
            command.history_render_footprint(),
            HistoryRenderFootprint::SingleParagraph { index: 2 },
        );

        let structural = InsertTextCommand::new(
            SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(3, 1)),
            "z".to_string(),
            None,
        );
        assert_eq!(
            structural.history_render_footprint(),
            HistoryRenderFootprint::Structural,
        );
    }

    #[test]
    fn executor_returns_footprint_for_single_paragraph_undo_and_redo() {
        let mut word_doc = sample_document("one two");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 7)));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 3)),
                        "-".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            let footprint = executor.undo_with_footprint(&mut ctx).unwrap();
            assert_eq!(
                footprint,
                Some(HistoryRenderFootprint::SingleParagraph { index: 0 }),
            );
        }
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            let footprint = executor.redo_with_footprint(&mut ctx).unwrap();
            assert_eq!(
                footprint,
                Some(HistoryRenderFootprint::SingleParagraph { index: 0 }),
            );
        }
    }

    #[test]
    fn inserting_text_preserves_anchored_drawing_xml_in_the_same_paragraph() {
        for fixture in [
            "floating-square-wrap.docx",
            "textbox.docx",
            "image-crop-edge-cases.docx",
            "shape-preset-geometries.docx",
            "yokohama-word-textbox.docx",
        ] {
            let data = std::fs::read(fixture_path(fixture)).expect("read drawing fixture");
            let before = document_xml_from_docx(&data);
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load drawing fixture");
            let paragraph_index = (0..word_doc.paragraph_count())
                .find(|&index| {
                    word_doc.paragraph(index).is_some_and(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .any(|run| run.image.is_some() || run.text_box.is_some())
                    })
                })
                .expect("paragraph containing a drawing");
            let range = SelectionRange::collapsed(TextPosition::new(paragraph_index, 0));
            let mut selection = Some(range);
            let mut command = InsertTextCommand::new(range, "X".to_string(), None);

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                command.apply(&mut ctx).expect("insert beside drawing");
            }

            let after = document_xml_from_docx(&word_doc.export_docx().expect("export docx"));
            for tag in [
                "<w:drawing",
                "<wp:anchor",
                "<wp:inline",
                "<mc:AlternateContent",
                "<mc:Fallback",
                "<w:pict",
                "<a:srcRect",
                "<wpg:wgp",
                "<a:custGeom",
                "<a:schemeClr",
            ] {
                assert_eq!(
                    after.matches(tag).count(),
                    before.matches(tag).count(),
                    "{tag} count changed after inserting into {fixture}"
                );
            }
            assert!(
                word_doc
                    .paragraph(paragraph_index)
                    .is_some_and(|paragraph| paragraph
                        .runs
                        .iter()
                        .any(|run| run.text.contains('X'))),
                "missing inserted text in {fixture}"
            );

            let delete_range = SelectionRange::new(
                TextPosition::new(paragraph_index, 0),
                TextPosition::new(paragraph_index, 1),
            );
            let mut delete = DeleteTextCommand::new(delete_range, DeleteDirection::Forward);
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                delete.apply(&mut ctx).expect("delete beside drawing");
            }
            let after_delete =
                document_xml_from_docx(&word_doc.export_docx().expect("export deleted docx"));
            for tag in [
                "<w:drawing",
                "<wp:anchor",
                "<wp:inline",
                "<mc:AlternateContent",
                "<mc:Fallback",
                "<w:pict",
                "<a:srcRect",
                "<wpg:wgp",
                "<a:custGeom",
                "<a:schemeClr",
            ] {
                assert_eq!(
                    after_delete.matches(tag).count(),
                    before.matches(tag).count(),
                    "{tag} count changed after deleting in {fixture}"
                );
            }
        }
    }

    /// How many times each part of a complex `MERGEFIELD Name`'s code is in
    /// `xml`, in order: begin, instruction, separate, end.
    fn merge_field_code(xml: &str) -> [usize; 4] {
        [
            r#"w:fldCharType="begin""#,
            " MERGEFIELD Name ",
            r#"w:fldCharType="separate""#,
            r#"w:fldCharType="end""#,
        ]
        .map(|part| xml.matches(part).count())
    }

    #[test]
    fn tracked_typing_at_the_end_of_a_complex_field_keeps_its_code() {
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                r#"<w:p><w:r><w:t>AB</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>«Name»</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>CD</w:t></w:r></w:p>"#,
            ),
            &Options::default(),
        ))
        .expect("load docx");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("Reviewer".to_string()),
            date: None,
            date_utc: None,
        }));

        insert_text(&mut word_doc, 0, 2 + "«Name»".len(), "X");

        let xml = exported_document_xml(&mut word_doc);
        assert!(xml.contains("<w:ins"), "{xml}");
        assert_eq!(merge_field_code(&xml), [1; 4], "{xml}");
    }

    #[test]
    fn typing_after_deleting_the_text_beside_an_empty_field_stays_out_of_its_code() {
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                r#"<w:p><w:r><w:t>AB</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
            ),
            &Options::default(),
        ))
        .expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 2));
        let mut selection = Some(range);
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            CommandExecutor::new()
                .execute(
                    Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                    &mut ctx,
                )
                .expect("delete");
        }
        assert_eq!(
            merge_field_code(&exported_document_xml(&mut word_doc)),
            [1; 4]
        );

        insert_text(&mut word_doc, 0, 0, "X");

        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(merge_field_code(&xml), [1; 4], "{xml}");
        let typed = xml.find("<w:t>X</w:t>").expect("typed text");
        let run_start = xml[..typed].rfind("<w:r>").expect("typed run");
        let run_end = typed + xml[typed..].find("</w:r>").expect("typed run end");
        assert!(
            !xml[run_start..run_end].contains("w:fldChar"),
            "typed into the field's code: {xml}"
        );
    }

    #[test]
    fn replacing_a_selection_holding_a_complex_field_removes_its_code() {
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                r#"<w:p><w:r><w:t>AB</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>«Name»</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>CD</w:t></w:r></w:p>"#,
            ),
            &Options::default(),
        ))
        .expect("load docx");
        let end = 2 + "«Name»".len() + 2;
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, end));
        let mut selection = Some(range);
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            CommandExecutor::new()
                .execute(
                    Box::new(InsertTextCommand::new(range, "X".to_string(), None)),
                    &mut ctx,
                )
                .expect("replace");
        }

        let paragraph = word_doc.paragraph(0).expect("paragraph");
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "X");
        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(merge_field_code(&xml), [0; 4], "{xml}");
    }

    /// A complex `MERGEFIELD` showing `«Name»`, as paragraph content.
    const MERGE_FIELD_RUNS: &str = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>«Name»</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#;

    /// A paragraph of `parts`: XML where a part starts with `<`, a text run
    /// otherwise.
    fn paragraph_of(parts: &[&str]) -> String {
        let content: String = parts
            .iter()
            .map(|part| {
                if part.starts_with('<') {
                    part.to_string()
                } else {
                    format!("<w:r><w:t>{part}</w:t></w:r>")
                }
            })
            .collect();
        format!("<w:p>{content}</w:p>")
    }

    /// Replaces the text from `start` to `end`, each a paragraph and an
    /// offset, with `text`, and returns the executor that can undo it.
    fn replace_across(
        word_doc: &mut WordDocument,
        start: (usize, usize),
        end: (usize, usize),
        text: &str,
    ) -> CommandExecutor {
        let range = SelectionRange::new(
            TextPosition::new(start.0, start.1),
            TextPosition::new(end.0, end.1),
        );
        let mut selection = Some(range);
        let mut ctx = CommandContext::new(word_doc, &mut selection);
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(InsertTextCommand::new(range, text.to_string(), None)),
                &mut ctx,
            )
            .expect("replace");
        executor
    }

    fn paragraph_texts(word_doc: &WordDocument) -> Vec<String> {
        (0..word_doc.paragraph_count())
            .map(|index| {
                word_doc
                    .paragraph(index)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect()
            })
            .collect()
    }

    /// The `w:fldCharType` of every `w:fldChar` in `xml`, in order.
    fn field_char_types(xml: &str) -> Vec<&str> {
        xml.split(r#"w:fldCharType=""#)
            .skip(1)
            .map(|rest| &rest[..rest.find('"').expect("closing quote")])
            .collect()
    }

    /// Asserts that each of `parts` is in `xml` once, in this order. A part
    /// that does not start with `w:` or a space is the text of a `w:t`.
    fn assert_in_order(xml: &str, parts: &[&str], context: &str) {
        let positions: Vec<usize> = parts
            .iter()
            .map(|part| {
                let needle = if part.starts_with("w:") || part.starts_with(' ') {
                    part.to_string()
                } else {
                    format!(">{part}</w:t>")
                };
                assert_eq!(
                    xml.matches(&needle).count(),
                    1,
                    "{context}: {needle}: {xml}"
                );
                xml.find(&needle).expect("part")
            })
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{context}: out of order {parts:?}: {xml}"
        );
    }

    /// Asserts that the run holding `text` holds no field code.
    fn assert_typed_outside_field_code(xml: &str, text: &str) {
        let typed = xml.find(&format!(">{text}</w:t>")).expect("typed text");
        let run_start = xml[..typed].rfind("<w:r>").expect("typed run");
        let run_end = typed + xml[typed..].find("</w:r>").expect("typed run end");
        assert!(
            !xml[run_start..run_end].contains("w:fldChar")
                && !xml[run_start..run_end].contains("w:instrText"),
            "typed into the field's code: {xml}"
        );
    }

    #[test]
    fn replacing_across_paragraphs_keeps_the_fields_it_does_not_touch() {
        let name = "«Name»".len();
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                &[
                    paragraph_of(&[MERGE_FIELD_RUNS, "AB"]),
                    paragraph_of(&["CD", MERGE_FIELD_RUNS, "EF"]),
                ]
                .concat(),
            ),
            &Options::default(),
        ))
        .expect("load docx");

        replace_across(&mut word_doc, (0, name + 1), (1, 1), "X");

        assert_eq!(paragraph_texts(&word_doc), ["«Name»AXD«Name»EF"]);
        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(
            field_char_types(&xml),
            ["begin", "separate", "end", "begin", "separate", "end"],
            "{xml}"
        );
        assert_eq!(merge_field_code(&xml)[1], 2, "{xml}");
        assert_in_order(&xml, &["AX", "D", "EF"], "replacement");
    }

    #[test]
    fn replacing_across_paragraphs_never_leaves_half_a_field() {
        let cut = 2 + "«Na".len();
        // The code the selection took stays at the cut. Typing goes before it
        // when text is left there, as it does in a single paragraph, and
        // otherwise into its result, never into the code itself.
        for (label, start, text, order) in [
            (
                "text before the cut",
                (0, 1),
                "AXme»EF",
                vec![
                    "AX",
                    "w:fldCharType=\"begin\"",
                    " MERGEFIELD Name ",
                    "w:fldCharType=\"separate\"",
                    "me»",
                    "w:fldCharType=\"end\"",
                    "EF",
                ],
            ),
            (
                "nothing before the cut",
                (0, 0),
                "Xme»EF",
                vec![
                    "w:fldCharType=\"begin\"",
                    " MERGEFIELD Name ",
                    "w:fldCharType=\"separate\"",
                    "X",
                    "me»",
                    "w:fldCharType=\"end\"",
                    "EF",
                ],
            ),
        ] {
            let mut word_doc = executor::block_on(WordDocument::load(
                &build_docx_with_body(
                    &[
                        paragraph_of(&["AB"]),
                        paragraph_of(&["CD", MERGE_FIELD_RUNS, "EF"]),
                    ]
                    .concat(),
                ),
                &Options::default(),
            ))
            .expect("load docx");

            let mut executor = replace_across(&mut word_doc, start, (1, cut), "X");

            assert_eq!(paragraph_texts(&word_doc), [text], "{label}");
            let xml = exported_document_xml(&mut word_doc);
            assert_eq!(merge_field_code(&xml), [1; 4], "{label}: {xml}");
            assert_in_order(&xml, &order, label);
            assert_typed_outside_field_code(&xml, if start.1 == 0 { "X" } else { "AX" });

            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"), "{label}");
            assert_eq!(paragraph_texts(&word_doc), ["AB", "CD«Name»EF"], "{label}");
            let xml = exported_document_xml(&mut word_doc);
            assert_in_order(
                &xml,
                &[
                    "CD",
                    "w:fldCharType=\"begin\"",
                    " MERGEFIELD Name ",
                    "w:fldCharType=\"separate\"",
                    "«Name»",
                    "w:fldCharType=\"end\"",
                    "EF",
                ],
                label,
            );
        }
    }

    #[test]
    fn replacing_across_paragraphs_takes_a_field_it_holds_whole() {
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                &[
                    paragraph_of(&["A", MERGE_FIELD_RUNS, "B"]),
                    paragraph_of(&["CD"]),
                ]
                .concat(),
            ),
            &Options::default(),
        ))
        .expect("load docx");

        replace_across(&mut word_doc, (0, 1), (1, 1), "X");

        assert_eq!(paragraph_texts(&word_doc), ["AXD"]);
        let xml = exported_document_xml(&mut word_doc);
        assert_eq!(merge_field_code(&xml), [0; 4], "{xml}");
        assert!(!xml.contains("Name"), "{xml}");
    }

    #[test]
    fn tracked_replacement_across_paragraphs_keeps_the_code_of_the_fields_it_deletes() {
        let mut word_doc = executor::block_on(WordDocument::load(
            &build_docx_with_body(
                &[
                    paragraph_of(&["A", MERGE_FIELD_RUNS, "B"]),
                    paragraph_of(&["CD"]),
                ]
                .concat(),
            ),
            &Options::default(),
        ))
        .expect("load docx");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("Reviewer".to_string()),
            date: None,
            date_utc: None,
        }));

        replace_across(&mut word_doc, (0, 1), (1, 1), "X");

        // The selected text stays as a deletion, so the field's code stays
        // around its deleted result.
        let xml = exported_document_xml(&mut word_doc);
        assert!(xml.contains("<w:ins"), "{xml}");
        assert!(xml.contains("<w:delText>«Name»</w:delText>"), "{xml}");
        assert_eq!(merge_field_code(&xml), [1; 4], "{xml}");
        assert_in_order(
            &xml,
            &[
                "w:fldCharType=\"begin\"",
                " MERGEFIELD Name ",
                "w:fldCharType=\"separate\"",
                "w:delText>«Name»</w:delText",
                "w:fldCharType=\"end\"",
            ],
            "tracked",
        );
    }

    #[test]
    fn deleting_selected_tables_undoes_and_redoes_the_entire_package() {
        let table = r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        let data = build_docx_with_body(&format!(
            "<w:p><w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r></w:p>"
        ));
        for tracked in [false, true] {
            let mut document =
                executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
            if tracked {
                document.set_revision_session(Some(RevisionSession {
                    author: Some("LAWGUE".into()),
                    ..Default::default()
                }));
            }
            let before = exported_document_xml(&mut document);
            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 4));
            let mut selection = Some(range);
            let mut history = CommandExecutor::new();
            let command =
                InsertTextCommand::new(range, String::new(), None).deleting_selected_tables();
            history
                .execute(
                    Box::new(command),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
            let after = exported_document_xml(&mut document);
            assert_ne!(after, before);
            assert_eq!(document.table_by_order(0).is_some(), tracked);
            assert!(history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap());
            assert_eq!(exported_document_xml(&mut document), before);
            assert_eq!(selection, Some(range));
            assert!(history
                .redo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap());
            assert_eq!(exported_document_xml(&mut document), after);
        }
    }

    #[test]
    fn deleting_selected_tables_rolls_back_when_a_later_table_refuses_deletion() {
        let inserted = r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:trPr><w:ins w:id="1" w:author="LAWGUE"/></w:trPr><w:tc><w:p><w:ins w:id="2" w:author="Reviewer"><w:r><w:t>REVIEW</w:t></w:r></w:ins></w:p></w:tc></w:tr></w:tbl>"#;
        let plain = r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        let data = build_docx_with_body(&format!("<w:p><w:r><w:t>HEAD</w:t></w:r></w:p>{inserted}{plain}<w:p><w:r><w:t>TAIL</w:t></w:r></w:p>"));
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
        document.set_revision_session(Some(RevisionSession {
            author: Some("LAWGUE".into()),
            ..Default::default()
        }));
        let before = exported_document_xml(&mut document);
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(2, 4));
        let mut selection = Some(range);
        let mut history = CommandExecutor::new();
        let command = InsertTextCommand::new(range, String::new(), None).deleting_selected_tables();
        let error = history
            .execute(
                Box::new(command),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap_err();
        assert!(error.to_string().contains("other author's revisions"));
        assert_eq!(exported_document_xml(&mut document), before);
        assert_eq!(selection, Some(range));
        assert!(!history.can_undo());
    }
}
