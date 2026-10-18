use unicode_segmentation::UnicodeSegmentation;

use std::collections::HashSet;

use crate::document::{Paragraph, Run};
use crate::docx_xml::{XmlNode, XmlNodeId};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

use super::paragraph_undo::{commented_body_paragraph, ParagraphRangeUndo};
use super::raw_xml_undo::RawXmlUndo;
use super::text_edit::{
    append_rebased_tail_comment_anchors, apply_text_edit, range_contains_deleted_revision,
    rebase_comment_anchors_for_edit, split_runs_at_preserving_source, unprotected_spans,
    CommentAnchors, DeletedFields, ZeroWidthRuns,
};
use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};

/// Direction for the deletion command when applied to a collapsed selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteDirection {
    Backward,
    Forward,
}

/// Command that removes the current selection or a single grapheme relative to
/// the caret. When the selection spans text the command behaves like a range
/// deletion. When the selection is collapsed the direction controls whether the
/// grapheme preceding (`Backward`) or following (`Forward`) the caret is
/// removed. Selections that cross paragraph boundaries now merge runs while
/// preserving inline styling metadata.
pub struct DeleteTextCommand {
    range: SelectionRange,
    direction: DeleteDirection,
    previous_runs: Option<Vec<Run>>,
    previous_anchors: Option<CommentAnchors>,
    previous_paragraphs: Option<Vec<Paragraph>>,
    raw_xml_undo: RawXmlUndo,
    /// Set by a tracked join, which only marks the paragraph mark deleted in
    /// the round-trip XML: undo rolls the XML back and puts the revision id
    /// counter back to this value so a redo reuses the same id.
    tracked_join_revision_id: Option<u32>,
    previous_selection: Option<SelectionRange>,
    /// Package snapshot for edits that change a note part as well as its model.
    previous_snapshot: Option<WordDocumentSnapshot>,
    /// Undo for a delete that rewrites its paragraphs without removing any:
    /// a tracked one across sibling paragraphs (it marks the text and the
    /// paragraph marks deleted) or one across containers.
    in_place: Option<ParagraphRangeUndo>,
    paragraph_index: usize,
    no_op: bool,
    /// Whether the last `apply` only rewrote runs of `paragraph_index` (no
    /// merge, no note removal). Recorded once per `apply` because `undo` takes
    /// the backups the other branches keep, so they cannot be re-inspected
    /// before a redo.
    single_paragraph_edit: bool,
}

impl DeleteTextCommand {
    pub fn new(range: SelectionRange, direction: DeleteDirection) -> Self {
        let paragraph_index = range.focus.paragraph;
        Self {
            range,
            direction,
            previous_runs: None,
            previous_anchors: None,
            previous_paragraphs: None,
            raw_xml_undo: RawXmlUndo::default(),
            tracked_join_revision_id: None,
            previous_selection: None,
            previous_snapshot: None,
            in_place: None,
            paragraph_index,
            no_op: false,
            single_paragraph_edit: false,
        }
    }

    /// The note whose reference mark sits right before (Backward) or right
    /// after (Forward) the rendered caret, if any. Deleting the mark deletes
    /// the note, as in Word.
    fn adjacent_reference_mark(
        &self,
        ctx: &CommandContext<'_>,
        paragraph: usize,
        rendered_offset: usize,
    ) -> Option<(usize, String, bool)> {
        adjacent_reference_mark(ctx.document, self.direction, paragraph, rendered_offset)
    }

    fn compute_deletion(
        &self,
        paragraph_text: &str,
        start_offset: usize,
        end_offset: usize,
    ) -> (usize, usize, usize) {
        if start_offset != end_offset {
            // Non-collapsed selections remove the highlighted range.
            return (start_offset, end_offset, start_offset);
        }

        match self.direction {
            DeleteDirection::Backward => {
                if start_offset == 0 {
                    return (start_offset, start_offset, start_offset);
                }
                let previous = previous_grapheme_boundary(paragraph_text, start_offset);
                (previous, start_offset, previous)
            }
            DeleteDirection::Forward => {
                if start_offset >= paragraph_text.len() {
                    return (start_offset, start_offset, start_offset);
                }
                let next = next_grapheme_boundary(paragraph_text, start_offset);
                (start_offset, next, start_offset)
            }
        }
    }

    fn merge_with_previous(
        &mut self,
        ctx: &mut CommandContext<'_>,
        paragraph_index: usize,
    ) -> CommandResult<TextPosition> {
        if paragraph_index == 0 {
            return Ok(TextPosition::new(paragraph_index, 0));
        }
        if ctx.document.revision_session().is_some() {
            let previous = paragraph_index - 1;
            let previous_len = ctx
                .document
                .paragraph(previous)
                .map(paragraph_len)
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.mark_tracked_join(ctx, previous)?;
            self.paragraph_index = previous;
            return Ok(TextPosition::new(previous, previous_len));
        }

        // Word keeps the formatting of the paragraph that has text: Backspace
        // into an empty paragraph removes that paragraph, so a heading or
        // 「第N条」 below it keeps its style and numbering.
        if paragraph_is_removable_empty(ctx, paragraph_index - 1) {
            let previous = paragraph_index - 1;
            self.remove_empty_paragraph(ctx, previous)?;
            return Ok(TextPosition::new(previous, 0));
        }

        self.previous_paragraphs = Some(capture_paragraph_range(
            ctx,
            paragraph_index - 1,
            paragraph_index,
        )?);
        self.capture_structural_xml(ctx, paragraph_index - 1)?;
        self.previous_runs = None;

        let prev_index = paragraph_index - 1;
        let prev_len = ctx
            .document
            .paragraph(prev_index)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;

        let tail_paragraph = ctx
            .document
            .paragraph(paragraph_index)
            .cloned()
            .ok_or(CommandError::ParagraphUnavailable)?;
        let tail_inline_xml = paragraph_inline_xml(ctx, paragraph_index);

        // Backspace deletes the previous paragraph's mark and keeps this one
        // (Word stores a section's properties on the mark that ends it): a
        // break on the previous mark goes, joining the sections, and one on
        // this mark now ends the merged paragraph.
        ctx.document
            .move_section_break(paragraph_index, prev_index)
            .map_err(CommandError::Custom)?;

        let moved_runs = {
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            std::mem::take(&mut paragraph.runs)
        };

        remove_paragraph_at_checked(ctx, paragraph_index)?;
        append_inline_xml(ctx, prev_index, tail_inline_xml)?;

        if let Some(prev) = ctx.document.paragraph_mut(prev_index) {
            // All of this paragraph's XML moved, so its runs stay bound to it:
            // a field's code has nothing in the model to be written anew from.
            let moved = prev.runs.len()..prev.runs.len() + moved_runs.len();
            prev.runs.extend(moved_runs);
            append_rebased_tail_comment_anchors(prev, &tail_paragraph, 0, prev_len);
            ctx.document
                .restyle_runs_from(prev_index, moved, style_of(&tail_paragraph));
            ctx.document
                .sync_paragraph_text_preserving_drawings(prev_index);
        }
        show_moved_text_boxes(ctx, prev_index);

        self.paragraph_index = prev_index;
        Ok(TextPosition::new(prev_index, prev_len))
    }

    fn merge_with_next(
        &mut self,
        ctx: &mut CommandContext<'_>,
        paragraph_index: usize,
    ) -> CommandResult<TextPosition> {
        if ctx.document.revision_session().is_some() {
            let current_len = ctx
                .document
                .paragraph(paragraph_index)
                .map(paragraph_len)
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.mark_tracked_join(ctx, paragraph_index)?;
            self.paragraph_index = paragraph_index;
            return Ok(TextPosition::new(paragraph_index, current_len));
        }
        let next_index = paragraph_index + 1;
        // Delete in an empty paragraph removes it, as Backspace from the
        // other side does: the next paragraph keeps its own formatting.
        if paragraph_is_removable_empty(ctx, paragraph_index) {
            self.remove_empty_paragraph(ctx, paragraph_index)?;
            return Ok(TextPosition::new(paragraph_index, 0));
        }
        self.previous_paragraphs = Some(capture_paragraph_range(ctx, paragraph_index, next_index)?);
        self.capture_structural_xml(ctx, paragraph_index)?;
        self.previous_runs = None;

        let caret_offset = ctx
            .document
            .paragraph(paragraph_index)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;

        let tail_paragraph = ctx
            .document
            .paragraph(next_index)
            .cloned()
            .ok_or(CommandError::ParagraphUnavailable)?;
        let tail_inline_xml = paragraph_inline_xml(ctx, next_index);

        // Delete at the end of a paragraph deletes its mark and keeps the next
        // one, as Backspace from the other side does, section breaks included.
        ctx.document
            .move_section_break(next_index, paragraph_index)
            .map_err(CommandError::Custom)?;

        let moved_runs = {
            let paragraph = ctx
                .document
                .paragraph_mut(next_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            std::mem::take(&mut paragraph.runs)
        };

        remove_paragraph_at_checked(ctx, next_index)?;
        append_inline_xml(ctx, paragraph_index, tail_inline_xml)?;

        if let Some(current) = ctx.document.paragraph_mut(paragraph_index) {
            // As for Backspace: the runs stay bound to the XML that moved.
            let moved = current.runs.len()..current.runs.len() + moved_runs.len();
            current.runs.extend(moved_runs);
            append_rebased_tail_comment_anchors(current, &tail_paragraph, 0, caret_offset);
            ctx.document
                .restyle_runs_from(paragraph_index, moved, style_of(&tail_paragraph));
            ctx.document
                .sync_paragraph_text_preserving_drawings(paragraph_index);
        }
        show_moved_text_boxes(ctx, paragraph_index);

        self.paragraph_index = paragraph_index;
        Ok(TextPosition::new(paragraph_index, caret_offset))
    }

    /// Drops the empty paragraph at `index` (mark included) so the paragraph
    /// after it keeps its own properties. Undo restores the document snapshot.
    /// Records a tracked deletion of the mark ending `paragraph_index`.
    ///
    /// Only the round-trip XML and the revision id counter change, so undo
    /// rolls those back rather than restoring a snapshot of the whole
    /// document (~250 ms per tracked Backspace on yokohama).
    fn mark_tracked_join(
        &mut self,
        ctx: &mut CommandContext<'_>,
        paragraph_index: usize,
    ) -> CommandResult<()> {
        let revision_id = ctx.document.revision_id_counter();
        self.raw_xml_undo.begin(ctx);
        if let Err(error) = ctx.document.mark_deleted_paragraph_break(paragraph_index) {
            self.raw_xml_undo.abort(ctx);
            ctx.document.restore_revision_id_counter(revision_id);
            return Err(CommandError::Custom(error));
        }
        self.tracked_join_revision_id = Some(revision_id);
        Ok(())
    }

    fn remove_empty_paragraph(
        &mut self,
        ctx: &mut CommandContext<'_>,
        index: usize,
    ) -> CommandResult<()> {
        let note = ctx.document.is_note_paragraph_surface(index);
        if note || !ctx.document.paragraphs_share_a_container(index, index + 1) {
            let snapshot = if note {
                ctx.document.snapshot_including_package()
            } else {
                ctx.document.snapshot()
            }
            .ok_or(CommandError::DocumentUnavailable)?;
            if let Err(error) = remove_whole_paragraphs(ctx, index, index) {
                ctx.document.restore_snapshot(snapshot);
                return Err(error);
            }
            self.previous_snapshot = Some(snapshot);
            self.paragraph_index = index;
            return Ok(());
        }
        // The result is a join with the following paragraph that keeps that
        // paragraph's formatting, so undo is the join's: split the survivor
        // back and restore both from these backups, and roll back the XML.
        // A snapshot of the whole document here cost ~100 ms on yokohama.
        let backups = capture_paragraph_range(ctx, index, index + 1)?;
        self.raw_xml_undo.begin(ctx);
        if let Err(error) = remove_whole_paragraphs(ctx, index, index) {
            self.raw_xml_undo.abort(ctx);
            return Err(error);
        }
        self.previous_paragraphs = Some(backups);
        self.paragraph_index = index;
        Ok(())
    }
}

/// An empty paragraph Word removes whole when Backspace/Delete joins it with
/// a neighbour: no text or inline objects, no comment anchors, and no
/// section break on its mark (deleting that mark joins two sections, which
/// the ordinary join handles).
fn paragraph_is_removable_empty(ctx: &CommandContext<'_>, index: usize) -> bool {
    ctx.document.paragraph(index).is_some_and(|paragraph| {
        paragraph_mark_is_removable(paragraph)
            && paragraph.runs.iter().all(|run| {
                run.text.is_empty()
                    && run.image.is_none()
                    && run.text_box.is_none()
                    && run.math.is_none()
                    && run.field.is_none()
                    && run.comment_reference_ids.is_empty()
                    && run.footnote_reference_ids.is_empty()
                    && run.endnote_reference_ids.is_empty()
            })
    })
}

/// Whether a paragraph can disappear whole without leaving the document
/// inconsistent: its mark carries no section break and nothing in it anchors
/// a comment.
pub(super) fn paragraph_mark_is_removable(paragraph: &Paragraph) -> bool {
    paragraph.comment_ranges.is_empty()
        && paragraph.comment_markers.is_empty()
        && paragraph
            .properties
            .as_ref()
            .and_then(|props| props.section.as_ref())
            .is_none()
}

/// Removes paragraphs `first..=last`, paragraph marks included, so the
/// paragraph after them keeps its own properties (Word deletes a selected
/// paragraph mark; the following mark survives). Range markers such as
/// bookmarks move to the start of that paragraph rather than vanishing.
pub(super) fn remove_whole_paragraphs(
    ctx: &mut CommandContext<'_>,
    first: usize,
    last: usize,
) -> CommandResult<()> {
    ctx.document
        .paragraph(last + 1)
        .ok_or(CommandError::ParagraphUnavailable)?;
    let mut markers = Vec::new();
    for index in first..=last {
        markers.extend(
            paragraph_inline_xml(ctx, index)
                .into_iter()
                .filter(is_range_marker),
        );
    }
    for _ in first..=last {
        remove_paragraph_at_checked(ctx, first)?;
    }
    ctx.document
        .insert_paragraph_inline_xml(first, markers, true)
        .map_err(CommandError::Custom)?;
    Ok(())
}

/// `w:bookmarkStart`, `w:permEnd`, `w:moveFromRangeStart`, …: zero-width
/// anchors that must survive the paragraph they sat in.
fn is_range_marker(node: &XmlNode) -> bool {
    matches!(node, XmlNode::Element(element) if {
        let local = element.name.rsplit(':').next().unwrap_or_default();
        local.ends_with("Start") || local.ends_with("End")
    })
}

impl Command for DeleteTextCommand {
    fn name(&self) -> &'static str {
        "delete_text"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.single_paragraph_edit = false;
        let result = self.apply_edit(ctx);
        self.raw_xml_undo.finish(ctx, result.is_ok());
        result?;
        self.single_paragraph_edit = self.previous_snapshot.is_none()
            && self.previous_paragraphs.is_none()
            && self.in_place.is_none()
            && !self.raw_xml_undo.restores_xml();
        Ok(())
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        // A delete inside one paragraph (the common Backspace) restores that
        // paragraph's runs on undo and re-applies the same branch on redo, so
        // it is layout-equivalent to typing and can take the incremental
        // render path. Merges and note-mark deletions stay structural.
        if self.single_paragraph_edit {
            HistoryRenderFootprint::SingleParagraph {
                index: self.paragraph_index,
            }
        } else {
            HistoryRenderFootprint::Structural
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.undo_edit(ctx)
    }

    fn should_record_history(&self) -> bool {
        !self.no_op
    }
}

impl DeleteTextCommand {
    fn apply_edit(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let (start, end) = self.range.normalized();

        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.paragraph_index = start.paragraph;
        self.previous_runs = None;
        self.previous_paragraphs = None;
        self.raw_xml_undo = RawXmlUndo::default();
        self.tracked_join_revision_id = None;
        self.previous_snapshot = None;
        self.in_place = None;
        self.no_op = false;

        // Deleting a footnote/endnote reference mark removes the note with
        // it (Word). The mark is rendered text that no run holds, so it is
        // matched in rendered coordinates before the offsets are converted.
        if start == end {
            if let Some((mark_start, note_id, is_footnote)) =
                self.adjacent_reference_mark(ctx, start.paragraph, start.offset)
            {
                let snapshot = ctx
                    .document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?;
                let result = if is_footnote {
                    ctx.document.delete_footnote(&note_id)
                } else {
                    ctx.document.delete_endnote(&note_id)
                };
                if let Err(error) = result {
                    ctx.document.restore_snapshot(snapshot);
                    return Err(CommandError::Custom(error));
                }
                self.previous_snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
                    start.paragraph,
                    mark_start,
                )));
                return Ok(());
            }
        }

        // Offsets arrive in rendered space; the model has no bytes for the
        // note marks a body paragraph renders.
        let start = ctx.document.rendered_to_model_text_position(start);
        let end = ctx.document.rendered_to_model_text_position(end);

        if start.paragraph == end.paragraph {
            let paragraph_len = ctx
                .document
                .paragraph(start.paragraph)
                .map(paragraph_len)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let start_offset = start.offset.min(paragraph_len);
            let end_offset = end.offset.min(paragraph_len);

            let recording = ctx.document.revision_session().is_some();

            let (paragraph_text, recorded_runs): (String, Option<Vec<Run>>) = {
                let paragraph = ctx
                    .document
                    .paragraph(start.paragraph)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let text = paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>();
                let runs_snapshot = if recording {
                    Some(paragraph.runs.clone())
                } else {
                    None
                };
                (text, runs_snapshot)
            };
            let (delete_start, delete_end, caret_offset) =
                self.compute_deletion(&paragraph_text, start_offset, end_offset);

            if delete_start == delete_end {
                // Merging paragraphs is only allowed between siblings. At the
                // edge of a table cell Word does nothing, and doing it here
                // would leave the cell without a paragraph — a document Word
                // cannot open.
                if self.direction == DeleteDirection::Backward
                    && start_offset == 0
                    && start.paragraph > 0
                    && ctx
                        .document
                        .paragraphs_share_a_container(start.paragraph, start.paragraph - 1)
                {
                    let new_focus = self.merge_with_previous(ctx, start.paragraph)?;
                    self.previous_selection = previous_selection;
                    *ctx.selection = Some(SelectionRange::collapsed(
                        ctx.document.model_to_rendered_text_position(new_focus),
                    ));
                    return Ok(());
                }

                if self.direction == DeleteDirection::Forward
                    && start_offset >= paragraph_len
                    && start.paragraph + 1 < paragraph_count
                    && ctx
                        .document
                        .paragraphs_share_a_container(start.paragraph, start.paragraph + 1)
                {
                    let new_focus = self.merge_with_next(ctx, start.paragraph)?;
                    self.previous_selection = previous_selection;
                    *ctx.selection = Some(SelectionRange::collapsed(
                        ctx.document.model_to_rendered_text_position(new_focus),
                    ));
                    return Ok(());
                }

                self.previous_runs = None;
                self.previous_selection = previous_selection;
                self.no_op = true;
                return Ok(());
            }

            // Before any branch below edits the paragraph.
            self.previous_anchors = ctx
                .document
                .paragraph(start.paragraph)
                .map(CommentAnchors::of);
            // Comment markers sit between runs in the XML. Once an edit has
            // pushed some together, re-syncing the restored runs cannot tell
            // which side of a marker the text goes back on, and the comment
            // lost or gained text after an undo; journal this paragraph's XML.
            // (Note paragraphs keep their XML in their own part.)
            let mut anchored = if commented_body_paragraph(ctx, start.paragraph) {
                Some(ParagraphRangeUndo::begin(
                    ctx,
                    Some((start.paragraph, start.paragraph)),
                )?)
            } else {
                None
            };
            let protected_spans =
                ctx.document
                    .paragraph(start.paragraph)
                    .is_some_and(|paragraph| {
                        range_contains_deleted_revision(&paragraph.runs, delete_start, delete_end)
                    });
            if protected_spans {
                let previous_runs = ctx
                    .document
                    .paragraph(start.paragraph)
                    .map(|paragraph| paragraph.runs.clone())
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let spans = unprotected_spans(&previous_runs, delete_start, delete_end);
                if spans.is_empty() {
                    self.no_op = true;
                } else if recording {
                    for (from, to) in spans.iter().rev() {
                        if !ctx
                            .document
                            .apply_deletion_revision(start.paragraph, *from, to - from)
                        {
                            return Err(CommandError::Custom(
                                "could not record deletion outside an existing revision".into(),
                            ));
                        }
                    }
                } else {
                    let zero_width = ctx.document.zero_width_runs(start.paragraph);
                    for (from, to) in spans.iter().rev() {
                        let paragraph = ctx
                            .document
                            .paragraph_mut(start.paragraph)
                            .ok_or(CommandError::ParagraphUnavailable)?;
                        apply_text_edit(&mut paragraph.runs, *from, *to, "", &zero_width);
                        rebase_comment_anchors_for_edit(paragraph, *from, *to, 0);
                    }
                }
                if !spans.is_empty() {
                    ctx.document
                        .sync_paragraph_text_preserving_drawings(start.paragraph);
                    self.previous_runs = Some(previous_runs);
                }
                self.keep_anchored_undo(ctx, anchored.take());
                self.previous_selection = previous_selection;
                let new_focus = TextPosition::new(start.paragraph, delete_start);
                *ctx.selection = Some(SelectionRange::collapsed(
                    ctx.document.model_to_rendered_text_position(new_focus),
                ));
                return Ok(());
            }

            if recording {
                let length = delete_end.saturating_sub(delete_start);
                if length > 0
                    && ctx
                        .document
                        .apply_deletion_revision(start.paragraph, delete_start, length)
                {
                    ctx.document
                        .sync_paragraph_text_preserving_drawings(start.paragraph);
                    self.previous_runs = recorded_runs;
                    self.keep_anchored_undo(ctx, anchored.take());
                    self.previous_selection = previous_selection;
                    let new_focus = TextPosition::new(start.paragraph, caret_offset);
                    *ctx.selection = Some(SelectionRange::collapsed(
                        ctx.document.model_to_rendered_text_position(new_focus),
                    ));
                    return Ok(());
                }
            }

            let zero_width = ctx.document.zero_width_runs(start.paragraph);
            let paragraph = ctx
                .document
                .paragraph_mut(start.paragraph)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let previous_runs = paragraph.runs.clone();
            apply_text_edit(
                &mut paragraph.runs,
                delete_start,
                delete_end,
                "",
                &zero_width,
            );
            rebase_comment_anchors_for_edit(paragraph, delete_start, delete_end, 0);
            ctx.document
                .sync_paragraph_text_preserving_drawings(start.paragraph);

            let new_focus = TextPosition::new(start.paragraph, caret_offset);
            self.previous_runs = Some(previous_runs);
            self.keep_anchored_undo(ctx, anchored.take());
            self.previous_selection = previous_selection;
            *ctx.selection = Some(SelectionRange::collapsed(
                ctx.document.model_to_rendered_text_position(new_focus),
            ));
            return Ok(());
        }

        self.previous_paragraphs = Some(capture_paragraph_range(
            ctx,
            start.paragraph,
            end.paragraph,
        )?);
        self.capture_structural_xml(ctx, start.paragraph)?;
        self.previous_runs = None;

        let start_offset = {
            let len = ctx
                .document
                .paragraph(start.paragraph)
                .map(paragraph_len)
                .ok_or(CommandError::ParagraphUnavailable)?;
            start.offset.min(len)
        };
        let end_offset = {
            let len = ctx
                .document
                .paragraph(end.paragraph)
                .map(paragraph_len)
                .ok_or(CommandError::ParagraphUnavailable)?;
            end.offset.min(len)
        };

        let start_len = ctx
            .document
            .paragraph(start.paragraph)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;

        // A selection that crosses a container boundary (cell to cell, body
        // to cell) cannot fold its paragraphs into one: every cell keeps its
        // paragraph. Word clears the selected text paragraph by paragraph
        // instead, and so does this path. Folding used to remove a cell's only
        // paragraph and leave the layout and the model disagreeing on how
        // many paragraphs there were.
        let crosses_containers = (start.paragraph + 1..=end.paragraph).any(|index| {
            !ctx.document
                .paragraphs_share_a_container(start.paragraph, index)
        });
        if crosses_containers {
            // Nothing is removed here, so the paragraphs, a journal of the
            // XML and the revision id counter restore the cleared cells
            // exactly; a note keeps its XML in its own part, which the
            // journal does not cover.
            self.previous_paragraphs = None;
            self.raw_xml_undo.discard(ctx);
            let (mut in_place, snapshot) = if (start.paragraph..=end.paragraph)
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
            let result = (|| -> CommandResult<()> {
                let recording = ctx.document.revision_session().is_some();
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
                            .ok_or(CommandError::ParagraphUnavailable)?;
                        (0, len)
                    };
                    if from >= to {
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
                    ctx.document.sync_paragraph_text_preserving_drawings(index);
                }
                Ok(())
            })();
            if let Err(error) = result {
                if let Some(in_place) = in_place {
                    in_place.abort(ctx);
                } else if let Some(snapshot) = snapshot {
                    ctx.document.restore_snapshot(snapshot);
                }
                return Err(error);
            }
            if let Some(in_place) = in_place.as_mut() {
                in_place.finish(ctx);
            }
            self.in_place = in_place;
            self.previous_snapshot = snapshot;
            let new_focus = TextPosition::new(start.paragraph, start_offset);
            self.previous_selection = previous_selection;
            *ctx.selection = Some(SelectionRange::collapsed(
                ctx.document.model_to_rendered_text_position(new_focus),
            ));
            return Ok(());
        }

        if ctx.document.revision_session().is_some() {
            // Marking the selection deleted removes no paragraph, so the
            // paragraphs, a journal of the XML and the revision id counter
            // undo it; a note keeps its XML in its own part.
            self.previous_paragraphs = None;
            self.raw_xml_undo.discard(ctx);
            let (mut in_place, snapshot) =
                if ctx.document.is_note_paragraph_surface(start.paragraph) {
                    let snapshot = ctx
                        .document
                        .snapshot_including_package()
                        .ok_or(CommandError::DocumentUnavailable)?;
                    (None, Some(snapshot))
                } else {
                    let range = Some((start.paragraph, end.paragraph));
                    (Some(ParagraphRangeUndo::begin(ctx, range)?), None)
                };
            let result = (|| -> Result<(), String> {
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
                            return Err("could not record cross-paragraph deletion".into());
                        }
                    }
                    if !spans.is_empty() {
                        ctx.document.sync_paragraph_text_preserving_drawings(index);
                    }
                }
                for index in start.paragraph..end.paragraph {
                    ctx.document.mark_deleted_paragraph_break(index)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                if let Some(in_place) = in_place {
                    in_place.abort(ctx);
                } else if let Some(snapshot) = snapshot {
                    ctx.document.restore_snapshot(snapshot);
                }
                return Err(CommandError::Custom(error));
            }
            if let Some(in_place) = in_place.as_mut() {
                in_place.finish(ctx);
            }
            self.in_place = in_place;
            self.previous_snapshot = snapshot;
            self.previous_selection = previous_selection;
            *ctx.selection = Some(SelectionRange::collapsed(
                ctx.document
                    .model_to_rendered_text_position(TextPosition::new(
                        start.paragraph,
                        start_offset,
                    )),
            ));
            return Ok(());
        }

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
                            "could not record cross-paragraph deletion".into(),
                        ));
                    }
                }
            }
        }
        let folded = FoldedSelection::new(
            ctx,
            (start.paragraph, start_offset),
            (end.paragraph, end_offset),
            recording,
        )?;
        let retained_len = folded.retained_len();

        let tail_paragraph = ctx
            .document
            .paragraph(end.paragraph)
            .cloned()
            .ok_or(CommandError::ParagraphUnavailable)?;
        let (mut tail_inline_xml, transferred_comment_reference_runs) =
            range_tail_inline_xml(ctx, start.paragraph, end.paragraph, end_offset);
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

        // The selection deletes every mark but the last paragraph's, so the
        // merged paragraph ends with that mark and its section break.
        ctx.document
            .move_section_break(end.paragraph, start.paragraph)
            .map_err(CommandError::Custom)?;
        for idx in (start.paragraph + 1..=end.paragraph).rev() {
            remove_paragraph_at_checked(ctx, idx)?;
        }
        append_inline_xml(ctx, start.paragraph, tail_inline_xml)?;

        if let Some(paragraph) = ctx.document.paragraph_mut(start.paragraph) {
            paragraph.runs.extend(folded.retained);
            paragraph.runs.extend(transferred_comment_reference_runs);
            // The suffix raw nodes were moved into this paragraph as well, so
            // unsplit runs may keep their IDs and retain anchor ordering.
            let moved = paragraph.runs.len()..paragraph.runs.len() + suffix_runs.len();
            paragraph.runs.extend(suffix_runs);
            rebase_comment_anchors_for_edit(paragraph, start_offset, start_len, retained_len);
            append_rebased_tail_comment_anchors(
                paragraph,
                &tail_paragraph,
                end_offset,
                start_offset + retained_len,
            );
            ctx.document
                .restyle_runs_from(start.paragraph, moved, style_of(&tail_paragraph));
            ctx.document
                .sync_paragraph_text_preserving_drawings(start.paragraph);
        }
        show_moved_text_boxes(ctx, start.paragraph);

        let new_focus = TextPosition::new(start.paragraph, start_offset);
        self.previous_selection = previous_selection;
        *ctx.selection = Some(SelectionRange::collapsed(
            ctx.document.model_to_rendered_text_position(new_focus),
        ));
        Ok(())
    }

    fn capture_structural_xml(
        &mut self,
        ctx: &mut CommandContext<'_>,
        index: usize,
    ) -> CommandResult<()> {
        if ctx.document.is_note_paragraph_surface(index) {
            // Note paragraphs live in a separate OPC part. Re-splitting model
            // backups cannot restore its paragraph attributes and raw anchors.
            self.previous_snapshot = Some(
                ctx.document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?,
            );
            self.previous_paragraphs = None;
            self.raw_xml_undo.discard(ctx);
        } else {
            self.raw_xml_undo.begin(ctx);
        }
        Ok(())
    }

    /// Ends the journal a single-paragraph edit took (`anchored`) and undoes
    /// through it.
    fn keep_anchored_undo(
        &mut self,
        ctx: &mut CommandContext<'_>,
        anchored: Option<ParagraphRangeUndo>,
    ) {
        if let Some(mut undo) = anchored {
            undo.finish(ctx);
            self.in_place = Some(undo);
        }
    }

    fn undo_edit(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
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
        if let Some(revision_id) = self.tracked_join_revision_id.take() {
            std::mem::take(&mut self.raw_xml_undo).undo(ctx, |_| Ok(()))?;
            ctx.document.restore_revision_id_counter(revision_id);
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
            // Restoring paragraph properties can bring a section break back;
            // the split used to re-create the paragraphs computed the ranges
            // before those properties were put back.
            if let Some(document) = ctx.document.document_mut() {
                crate::word_document::WordDocument::rebuild_section_ranges(document);
            }
            ctx.document.rebuild_paragraph_surfaces();
            *ctx.selection = self.previous_selection;
            return Ok(());
        }

        if self.no_op {
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

/// The note reference mark right before (Backward) or right after (Forward)
/// the rendered offset: `(mark start, note id, is footnote)`.
fn adjacent_reference_mark(
    document: &WordDocument,
    direction: DeleteDirection,
    paragraph: usize,
    rendered_offset: usize,
) -> Option<(usize, String, bool)> {
    let mut rendered = 0usize;
    for span in document.reference_marker_spans(paragraph) {
        let mark_start = span.model_offset + rendered;
        let mark_end = mark_start + span.rendered_len;
        let adjacent = match direction {
            DeleteDirection::Backward => rendered_offset == mark_end,
            DeleteDirection::Forward => rendered_offset == mark_start,
        };
        if adjacent {
            return Some((mark_start, span.note_id, span.is_footnote));
        }
        rendered += span.rendered_len;
    }
    None
}

/// How many characters (code points) one Backspace / Delete removes at the
/// rendered `caret`: the whole grapheme cluster next to it, as Word and the
/// caret movement treat an IVS (葛󠄀), a flag (🇯🇵) or a letter with combining
/// marks as one character. A note reference mark, or a paragraph edge where
/// the step joins paragraphs, counts as one.
///
/// [`DeleteTextRunCommand`] counts code points because the headless API's
/// `count` (and its snapshot rebasing) is defined that way; the browser keys
/// convert one grapheme into that count here.
#[allow(
    dead_code,
    reason = "Shared with the headless core, which has no browser keyboard handler"
)]
pub(crate) fn grapheme_delete_count(
    document: &WordDocument,
    caret: TextPosition,
    direction: DeleteDirection,
) -> usize {
    if adjacent_reference_mark(document, direction, caret.paragraph, caret.offset).is_some() {
        return 1;
    }
    let Some(paragraph) = document.paragraph(caret.paragraph) else {
        return 1;
    };
    let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
    let offset = document.rendered_to_model_text_position(caret).offset;
    let cluster = match direction {
        DeleteDirection::Backward => text
            .get(..offset)
            .and_then(|before| before.graphemes(true).next_back()),
        DeleteDirection::Forward => text
            .get(offset..)
            .and_then(|after| after.graphemes(true).next()),
    };
    cluster.map_or(1, |cluster| cluster.chars().count().max(1))
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

pub(super) fn capture_paragraph_range(
    ctx: &mut CommandContext<'_>,
    start: usize,
    end: usize,
) -> CommandResult<Vec<Paragraph>> {
    let mut paragraphs = Vec::with_capacity(end.saturating_sub(start) + 1);
    for paragraph_index in start..=end {
        let paragraph = ctx
            .document
            .paragraph(paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        paragraphs.push(paragraph.clone());
    }
    Ok(paragraphs)
}

/// Gives the text boxes runs brought into `paragraph_index` their surfaces
/// back: removing the paragraph they came from took its text boxes'
/// paragraphs out of the paragraph order, and joining does not add a box, so
/// nothing else rebuilds them.
fn show_moved_text_boxes(ctx: &mut CommandContext<'_>, paragraph_index: usize) {
    let hosts_text_box = ctx
        .document
        .paragraph(paragraph_index)
        .is_some_and(|paragraph| paragraph.runs.iter().any(|run| run.text_box.is_some()));
    if hosts_text_box && ctx.document.hosted_text_box_surface_count(paragraph_index) == 0 {
        ctx.document.rebuild_paragraph_surfaces();
    }
}

fn remove_paragraph_at_checked(
    ctx: &mut CommandContext<'_>,
    paragraph_index: usize,
) -> CommandResult<()> {
    let before_count = ctx.document.paragraph_count();
    if paragraph_index >= before_count {
        return Err(CommandError::ParagraphUnavailable);
    }

    // A paragraph hosting a text box takes the box's paragraphs with it. The
    // check counted one paragraph only, failed after the paragraph and its
    // runs were gone, and Backspace at the start of such a paragraph lost
    // its text and the text box.
    let hosted = ctx.document.hosted_text_box_surface_count(paragraph_index);
    let removed = ctx.document.remove_paragraph_at(paragraph_index);
    if !removed {
        // remove_paragraph_at() can return false when removed paragraph has no node_id.
        ctx.document.rebuild_paragraph_surfaces();
    }

    let after_count = ctx.document.paragraph_count();
    if before_count != after_count.saturating_add(1 + hosted) {
        return Err(CommandError::ParagraphUnavailable);
    }

    Ok(())
}

/// What a selection from `start` to `end`, in different paragraphs, leaves
/// when those paragraphs are folded into the first: its zero-width runs (see
/// [`ZeroWidthRuns`]) and tracked deletions.
///
/// The selection goes as one range by the rule a deletion inside a paragraph
/// follows ([`DeletedFields`]): a complex field goes, code and result, when
/// the selection holds it from its begin through its end, and keeps its code
/// otherwise. So the fields before and after the selection stay, and so does
/// one it cuts into, or one that runs on to a paragraph outside it (a TOC):
/// the part of its code the selection took stays at the cut, where it is
/// whole again with the part left in the first or the last paragraph.
pub(super) struct FoldedSelection {
    start_paragraph: usize,
    end_paragraph: usize,
    /// The zero-width runs of every paragraph folded, for the runs they hand
    /// to the first.
    pub(super) zero_width: ZeroWidthRuns,
    /// The zero-width runs the selection takes.
    dropped: HashSet<XmlNodeId>,
    /// What stays at the cut, in document order: tracked deletions, written
    /// anew from the model, and field code, bound to its XML.
    pub(super) retained: Vec<Run>,
    /// The field code in `retained` from a paragraph the fold removes.
    moved_code: HashSet<XmlNodeId>,
}

impl FoldedSelection {
    /// Reads the paragraphs from `start` to `end`, each a paragraph index and
    /// an offset in it. `keeps_every_field` keeps all of their code, for a
    /// selection whose text stays as a tracked deletion.
    pub(super) fn new(
        ctx: &CommandContext<'_>,
        (start_paragraph, start_offset): (usize, usize),
        (end_paragraph, end_offset): (usize, usize),
        keeps_every_field: bool,
    ) -> CommandResult<Self> {
        let mut zero_width = ZeroWidthRuns::default();
        let mut paragraphs = Vec::with_capacity(end_paragraph + 1 - start_paragraph);
        for index in start_paragraph..=end_paragraph {
            paragraphs.push(
                ctx.document
                    .paragraph(index)
                    .ok_or(CommandError::ParagraphUnavailable)?,
            );
            zero_width.extend(ctx.document.zero_width_runs(index));
        }

        // Each paragraph mark counts as a character, so that runs at the end
        // of one paragraph and the start of the next are inside the range.
        let mut runs = Vec::new();
        let mut paragraph_start = 0usize;
        let mut selection_end = 0usize;
        for (index, paragraph) in (start_paragraph..).zip(&paragraphs) {
            if index == end_paragraph {
                selection_end = paragraph_start + end_offset;
            }
            let mut position = paragraph_start;
            for run in &paragraph.runs {
                runs.push((run, position));
                position += run.text.len();
            }
            paragraph_start = position + 1;
        }
        let dropped = if keeps_every_field {
            HashSet::new()
        } else {
            let fields = DeletedFields::new(&runs, start_offset, selection_end, &zero_width);
            runs.iter()
                .enumerate()
                .filter(|&(index, &(run, position))| {
                    zero_width.keeps(run) && !fields.keeps_zero_width(index, position)
                })
                .filter_map(|(_, (run, _))| run.node_id)
                .collect()
        };

        // The selected runs are cut as the first and the last paragraph are,
        // so that each zero-width run is in exactly one of the head, the
        // selection and the suffix.
        let mut retained = Vec::new();
        let mut moved_code = HashSet::new();
        for (index, paragraph) in (start_paragraph..).zip(&paragraphs) {
            let mut selected = paragraph.runs.clone();
            if index == start_paragraph {
                selected =
                    split_runs_at_preserving_source(&mut selected, start_offset, &zero_width);
            } else if index == end_paragraph {
                let _ = split_runs_at_preserving_source(&mut selected, end_offset, &zero_width);
            }
            for mut run in selected {
                if zero_width.keeps(&run) {
                    let Some(id) = run.node_id.filter(|id| !dropped.contains(id)) else {
                        continue;
                    };
                    if index != start_paragraph {
                        moved_code.insert(id);
                    }
                    retained.push(run);
                } else if run.revision.as_ref().is_some_and(|mark| {
                    matches!(
                        mark.kind,
                        crate::document::RevisionKind::Deleted
                            | crate::document::RevisionKind::MovedFrom
                    )
                }) {
                    run.node_id = None;
                    run.text_node_id = None;
                    run.revision_node_id = None;
                    retained.push(run);
                }
            }
        }

        Ok(Self {
            start_paragraph,
            end_paragraph,
            zero_width,
            dropped,
            retained,
            moved_code,
        })
    }

    pub(super) fn retained_len(&self) -> usize {
        self.retained.iter().map(|run| run.text.len()).sum()
    }

    /// Whether `run`, before or after the selection, goes with it: a
    /// zero-width run of a field the selection holds whole.
    pub(super) fn drops(&self, run: &Run) -> bool {
        self.zero_width.keeps(run) && run.node_id.is_some_and(|id| self.dropped.contains(&id))
    }

    /// Puts the XML of the field code kept from the paragraphs the fold
    /// removes, in document order, at the front of `tail`, the XML moving
    /// into the first paragraph: its runs go before the rest of the runs
    /// moving there too.
    ///
    /// A code run inside a container there (a hyperlink, a revision) moves
    /// out of it; the sync writes a revision back from the run. A copy the
    /// cut left in `tail` (a container cut in two takes its zero-width runs
    /// to both halves) goes, so the code is there once.
    pub(super) fn prepend_moved_code(&self, ctx: &CommandContext<'_>, tail: &mut Vec<XmlNode>) {
        fn is_moved(node: &XmlNode, ids: &HashSet<XmlNodeId>) -> bool {
            matches!(node, XmlNode::Element(element) if element.name == "w:r" && ids.contains(&element.id))
        }
        fn collect(node: XmlNode, ids: &HashSet<XmlNodeId>, out: &mut Vec<XmlNode>) {
            if is_moved(&node, ids) {
                out.push(node);
            } else if let XmlNode::Element(element) = node {
                if element.name != "w:r" {
                    for child in element.children {
                        collect(child, ids, out);
                    }
                }
            }
        }
        fn strip(nodes: &mut Vec<XmlNode>, ids: &HashSet<XmlNodeId>) {
            nodes.retain(|node| !is_moved(node, ids));
            for node in nodes {
                if let XmlNode::Element(element) = node {
                    if element.name != "w:r" {
                        strip(&mut element.children, ids);
                    }
                }
            }
        }

        if self.moved_code.is_empty() {
            return;
        }
        let mut code = Vec::new();
        for index in self.start_paragraph + 1..=self.end_paragraph {
            for node in paragraph_inline_xml(ctx, index) {
                collect(node, &self.moved_code, &mut code);
            }
        }
        strip(tail, &self.moved_code);
        tail.splice(0..0, code);
    }
}

pub(super) fn range_tail_inline_xml(
    ctx: &CommandContext<'_>,
    start_paragraph: usize,
    end_paragraph: usize,
    end_offset: usize,
) -> (Vec<XmlNode>, Vec<Run>) {
    // The first paragraph's raw markers remain when its text is synced.
    // Only markers in paragraphs that are removed need to be transferred.
    let first_inline_xml = paragraph_inline_xml(ctx, start_paragraph);
    let mut deleted_range_markers = Vec::new();
    for paragraph_index in start_paragraph + 1..end_paragraph {
        deleted_range_markers.extend(
            paragraph_inline_xml(ctx, paragraph_index)
                .into_iter()
                .filter(|node| {
                    range_marker_key(node, true).is_some()
                        || range_marker_key(node, false).is_some()
                        || !comment_reference_ids(node).is_empty()
                }),
        );
    }
    let (tail_inline_xml, transferred_comment_reference_ids) = paragraph_inline_xml_after_offset(
        ctx,
        end_paragraph,
        end_offset,
        deleted_range_markers,
        &first_inline_xml,
    );
    let transferred_comment_reference_runs = (start_paragraph + 1..=end_paragraph)
        .filter_map(|index| ctx.document.paragraph(index))
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| comment_reference_only_run(run, &transferred_comment_reference_ids))
        .collect::<Vec<_>>();

    (tail_inline_xml, transferred_comment_reference_runs)
}

fn paragraph_inline_xml(ctx: &CommandContext<'_>, paragraph_index: usize) -> Vec<XmlNode> {
    // The run model does not contain every inline OOXML node. Carry the raw
    // children across the join so bookmarks and other opaque anchors are not
    // discarded with the second paragraph. Its paragraph properties belong
    // to the removed paragraph mark, so they do not move.
    ctx.document
        .paragraph_xml_element(paragraph_index)
        .map(|paragraph| {
            paragraph
                .children
                .into_iter()
                .filter(|node| {
                    !matches!(node, XmlNode::Element(element) if element.name.rsplit(':').next() == Some("pPr"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn paragraph_inline_xml_after_offset(
    ctx: &CommandContext<'_>,
    paragraph_index: usize,
    byte_offset: usize,
    mut deleted_range_markers: Vec<XmlNode>,
    first_inline_xml: &[XmlNode],
) -> (Vec<XmlNode>, std::collections::HashSet<String>) {
    let (before, mut after) =
        paragraph_inline_xml_split_at_offset(ctx, paragraph_index, byte_offset);
    deleted_range_markers.extend(before);
    // A range may cross the cut in either direction: its start can be deleted
    // while its end survives, or its start can survive while its end is deleted.
    // Move the missing marker to the cut, preserving deleted-marker order.
    let ending_ids = after
        .iter()
        .filter_map(|node| range_marker_key(node, false))
        .map(|(kind, id)| (kind, id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    let starting_ids = after
        .iter()
        .filter_map(|node| range_marker_key(node, true))
        .map(|(kind, id)| (kind, id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    let first_starting_ids = first_inline_xml
        .iter()
        .filter_map(|node| range_marker_key(node, true))
        .map(|(kind, id)| (kind, id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    let first_ending_ids = first_inline_xml
        .iter()
        .filter_map(|node| range_marker_key(node, false))
        .map(|(kind, id)| (kind, id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    let existing_reference_ids = first_inline_xml
        .iter()
        .chain(after.iter())
        .flat_map(comment_reference_ids)
        .map(str::to_string)
        .collect::<std::collections::HashSet<_>>();
    let moved_comment_end_ids = deleted_range_markers
        .iter()
        .filter_map(|node| range_marker_key(node, false))
        .filter(|(kind, id)| {
            *kind == "comment" && {
                let key = (*kind, id.to_string());
                first_starting_ids.contains(&key)
                    && !first_ending_ids.contains(&key)
                    && !ending_ids.contains(&key)
                    && !existing_reference_ids.contains(*id)
            }
        })
        .map(|(_, id)| id.to_string())
        .collect::<std::collections::HashSet<_>>();
    let crossing_markers = deleted_range_markers
        .into_iter()
        .filter_map(|node| {
            if let Some((kind, id)) = range_marker_key(&node, true) {
                let key = (kind, id.to_string());
                return (ending_ids.contains(&key) && !starting_ids.contains(&key)).then_some(node);
            }
            if let Some((kind, id)) = range_marker_key(&node, false) {
                let key = (kind, id.to_string());
                return (first_starting_ids.contains(&key)
                    && !first_ending_ids.contains(&key)
                    && !ending_ids.contains(&key))
                .then_some(node);
            }
            retained_comment_reference_run(node, &moved_comment_end_ids)
        })
        .collect::<Vec<_>>();
    let transferred_comment_reference_ids = crossing_markers
        .iter()
        .flat_map(comment_reference_ids)
        .map(str::to_string)
        .collect();
    after.splice(0..0, crossing_markers);
    (after, transferred_comment_reference_ids)
}

fn paragraph_inline_xml_split_at_offset(
    ctx: &CommandContext<'_>,
    paragraph_index: usize,
    byte_offset: usize,
) -> (Vec<XmlNode>, Vec<XmlNode>) {
    let Some(paragraph) = ctx.document.paragraph(paragraph_index) else {
        return (Vec::new(), Vec::new());
    };
    let text = paragraph
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    // Selection offsets are UTF-8 bytes; raw OOXML splitting counts characters.
    let mut boundary = byte_offset.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let char_offset = text[..boundary].chars().count();
    // The runs after the cut move with this XML and stay bound to it, so it
    // has to be cut where they were.
    let mut run_lens = std::collections::HashMap::new();
    for run in &paragraph.runs {
        if let Some(id) = run.node_id {
            *run_lens.entry(id).or_insert(0) += run.text.chars().count();
        }
    }
    let inline = paragraph_inline_xml(ctx, paragraph_index);
    crate::word_document::WordDocument::split_paragraph_children_at_model_offset(
        inline,
        char_offset,
        &run_lens,
    )
}

fn range_marker_key(node: &XmlNode, start: bool) -> Option<(&'static str, &str)> {
    let XmlNode::Element(element) = node else {
        return None;
    };
    let kind = match (element.name.rsplit(':').next()?, start) {
        ("bookmarkStart", true) | ("bookmarkEnd", false) => "bookmark",
        ("commentRangeStart", true) | ("commentRangeEnd", false) => "comment",
        _ => return None,
    };
    let id = element
        .attributes
        .iter()
        .find(|attribute| attribute.name.rsplit(':').next() == Some("id"))
        .map(|attribute| attribute.value.as_str())?;
    Some((kind, id))
}

fn comment_reference_id(node: &XmlNode) -> Option<&str> {
    let XmlNode::Element(element) = node else {
        return None;
    };
    if element.name.rsplit(':').next() != Some("commentReference") {
        return None;
    }
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.rsplit(':').next() == Some("id"))
        .map(|attribute| attribute.value.as_str())
}

fn comment_reference_ids(node: &XmlNode) -> Vec<&str> {
    let XmlNode::Element(run) = node else {
        return Vec::new();
    };
    if run.name.rsplit(':').next() != Some("r") {
        return Vec::new();
    }
    run.children
        .iter()
        .filter_map(comment_reference_id)
        .collect()
}

fn retained_comment_reference_run(
    node: XmlNode,
    moved_end_ids: &std::collections::HashSet<String>,
) -> Option<XmlNode> {
    let XmlNode::Element(mut run) = node else {
        return None;
    };
    if run.name.rsplit(':').next() != Some("r") {
        return None;
    }
    let mut has_reference = false;
    run.children.retain(|child| {
        if matches!(child, XmlNode::Element(element) if element.name.rsplit(':').next() == Some("rPr")) {
            return true;
        }
        let matches = comment_reference_id(child).is_some_and(|id| moved_end_ids.contains(id));
        has_reference |= matches;
        matches
    });
    has_reference.then_some(XmlNode::Element(run))
}

fn comment_reference_only_run(
    run: &Run,
    transferred_ids: &std::collections::HashSet<String>,
) -> Option<Run> {
    let ids = run
        .comment_reference_ids
        .iter()
        .filter(|id| transferred_ids.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return None;
    }
    let mut reference = run.clone();
    reference.text.clear();
    reference.text_node_id = None;
    reference.image = None;
    reference.text_box = None;
    reference.math = None;
    reference.field = None;
    reference.hyperlink = None;
    reference.footnote_reference_ids.clear();
    reference.endnote_reference_ids.clear();
    reference.has_footnote_ref = false;
    reference.has_endnote_ref = false;
    reference.revision = None;
    reference.revision_node_id = None;
    reference.comment_reference_ids = ids;
    Some(reference)
}

/// The paragraph style of `paragraph`, the one runs moved out of it carry.
pub(crate) fn style_of(paragraph: &Paragraph) -> Option<&str> {
    paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
}

fn append_inline_xml(
    ctx: &mut CommandContext<'_>,
    paragraph_index: usize,
    inline: Vec<XmlNode>,
) -> CommandResult<()> {
    ctx.document
        .insert_paragraph_inline_xml(paragraph_index, inline, false)
        .map_err(CommandError::Custom)
}

fn clear_run_xml_bindings(runs: &mut [Run]) {
    for run in runs {
        run.node_id = None;
        run.text_node_id = None;
        run.revision_node_id = None;
    }
}

fn restore_paragraph_content(
    target: &mut Paragraph,
    source: &Paragraph,
    preserve_xml_bindings: bool,
) {
    target.properties = source.properties.clone();
    target.runs = source.runs.clone();
    target.comment_ranges = source.comment_ranges.clone();
    target.comment_markers = source.comment_markers.clone();
    if preserve_xml_bindings {
        // The journal restores the original XML node, not the temporary split node.
        // A subsequent structural undo must remove that original node as well.
        target.node_id = source.node_id;
    } else {
        clear_run_xml_bindings(&mut target.runs);
    }
}

/// Puts back the paragraphs a structural edit folded together, by restoring
/// the first and splitting the others off it.
///
/// With `preserve_xml_bindings` the caller rolls the round-trip XML back
/// afterwards, to exactly its state before the edit. Splitting and syncing the
/// restored paragraphs binds them and their runs to elements that roll-back
/// removes, so the backups, taken before the edit and matching that XML, are
/// put back whole last; otherwise later edits to them never reached the saved
/// XML or rewrote it from runs it no longer had.
pub(super) fn restore_multi_paragraph_edit(
    ctx: &mut CommandContext<'_>,
    start_paragraph: usize,
    previous_paragraphs: &[Paragraph],
    preserve_xml_bindings: bool,
) -> CommandResult<()> {
    let Some(first) = previous_paragraphs.first() else {
        return Err(CommandError::Custom(
            "missing backup paragraphs for undo".into(),
        ));
    };

    {
        let paragraph = ctx
            .document
            .paragraph_mut(start_paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        restore_paragraph_content(paragraph, first, preserve_xml_bindings);
    }
    ctx.document
        .sync_paragraph_text_preserving_drawings(start_paragraph);

    for (offset, source) in previous_paragraphs.iter().enumerate().skip(1) {
        let insert_after = start_paragraph + offset - 1;
        let split_offset = ctx
            .document
            .paragraph(insert_after)
            .map(paragraph_len)
            .ok_or(CommandError::ParagraphUnavailable)?;
        if !ctx.document.split_paragraph_at(insert_after, split_offset) {
            return Err(CommandError::ParagraphUnavailable);
        }

        let restore_index = insert_after + 1;
        {
            let paragraph = ctx
                .document
                .paragraph_mut(restore_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            restore_paragraph_content(paragraph, source, preserve_xml_bindings);
        }
        ctx.document
            .sync_paragraph_text_preserving_drawings(restore_index);
    }

    if previous_paragraphs.len() > 1 {
        // A split moves a section break onto the tail (the paragraph mark
        // that ended the paragraph, as in Word), and the loop then overwrites
        // that tail with the next backup. Put the head's own properties back
        // so an undone join keeps its section break.
        let paragraph = ctx
            .document
            .paragraph_mut(start_paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        paragraph.properties = first.properties.clone();
    }

    if preserve_xml_bindings {
        for (offset, source) in previous_paragraphs.iter().enumerate() {
            *ctx.document
                .paragraph_mut(start_paragraph + offset)
                .ok_or(CommandError::ParagraphUnavailable)? = source.clone();
        }
    }

    Ok(())
}

fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    if text.is_empty() || offset == 0 {
        return offset;
    }

    let mut prev = 0usize;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= offset {
            break;
        }
        prev = idx;
    }
    prev
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    if text.is_empty() {
        return offset;
    }

    for (idx, _) in text.grapheme_indices(true) {
        if idx > offset {
            return idx;
        }
    }

    text.len()
}

/// Deletes a run of `count` characters as **one** history step.
///
/// Backspace held down is one action to the person doing it, and the headless
/// API promises one undo step per command. The part of the run that lies inside
/// the caret's own paragraph is taken in a single range delete — so a deleted
/// phrase is one tracked change rather than one per character — and whatever is
/// left continues into the neighbouring paragraph one step at a time, because
/// each of those steps joins two paragraphs.
pub struct DeleteTextRunCommand {
    range: SelectionRange,
    direction: DeleteDirection,
    count: usize,
    applied: Vec<DeleteTextCommand>,
    previous_selection: Option<SelectionRange>,
}

impl DeleteTextRunCommand {
    pub fn new(range: SelectionRange, direction: DeleteDirection, count: usize) -> Self {
        Self {
            range,
            direction,
            count,
            applied: Vec::new(),
            previous_selection: None,
        }
    }

    /// How much of the run fits inside the caret's paragraph, as
    /// `(start, end, characters)`. `None` when the caret is already at the edge
    /// it is deleting towards.
    fn span_within_paragraph(
        ctx: &CommandContext<'_>,
        paragraph: usize,
        offset: usize,
        direction: DeleteDirection,
        count: usize,
    ) -> Option<(usize, usize, usize)> {
        // `DeleteTextCommand` measures against the paragraph's run text, so the
        // span is measured there too. An offset that does not address that text
        // falls through to `None` and the run is taken one character at a time,
        // exactly as it was before.
        let text: String = ctx
            .document
            .paragraph(paragraph)?
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        // `offset` is rendered (note marks count); the run text is not.
        let rendered = |model: usize| {
            ctx.document
                .model_to_rendered_text_position(TextPosition::new(paragraph, model))
                .offset
        };
        let offset = ctx
            .document
            .rendered_to_model_text_position(TextPosition::new(paragraph, offset))
            .offset;
        match direction {
            DeleteDirection::Backward => {
                let mut start = offset;
                let mut characters = 0usize;
                for (at, _) in text.get(..offset)?.char_indices().rev() {
                    if characters == count {
                        break;
                    }
                    start = at;
                    characters += 1;
                }
                (characters > 0).then_some((rendered(start), rendered(offset), characters))
            }
            DeleteDirection::Forward => {
                let mut end = offset;
                let mut characters = 0usize;
                for (at, character) in text.get(offset..)?.char_indices() {
                    if characters == count {
                        break;
                    }
                    end = offset + at + character.len_utf8();
                    characters += 1;
                }
                (characters > 0).then_some((rendered(offset), rendered(end), characters))
            }
        }
    }

    fn apply_step(
        &mut self,
        ctx: &mut CommandContext<'_>,
        range: SelectionRange,
    ) -> CommandResult<()> {
        let mut step = DeleteTextCommand::new(range, self.direction);
        match step.apply(ctx) {
            Ok(()) => {
                self.applied.push(step);
                Ok(())
            }
            Err(err) => {
                // Leave nothing half-done: what has been applied comes back out
                // before the error reaches the caller.
                for applied in self.applied.iter_mut().rev() {
                    applied.undo(ctx)?;
                }
                self.applied.clear();
                Err(err)
            }
        }
    }
}

impl Command for DeleteTextRunCommand {
    fn name(&self) -> &'static str {
        "delete_text"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        self.applied.clear();

        let (start, _) = self.range.normalized();
        // The headless dispatcher sets this selection before the first apply,
        // but redo calls the command directly after redoing the preceding
        // history entry. Do not continue a cross-paragraph delete from that
        // unrelated command's caret.
        *ctx.selection = Some(self.range);
        let mut done = 0usize;
        // A note's reference mark next to the caret is deleted (with its
        // note) as one step by the collapsed command; the character span
        // below only sees run text and would skip past the mark.
        let mark_probe = DeleteTextCommand::new(SelectionRange::collapsed(start), self.direction);
        if self.count > 0
            && mark_probe
                .adjacent_reference_mark(ctx, start.paragraph, start.offset)
                .is_some()
        {
            self.apply_step(ctx, SelectionRange::collapsed(start))?;
            done = 1;
        }
        if done == 0 {
            if let Some((span_start, span_end, characters)) = Self::span_within_paragraph(
                ctx,
                start.paragraph,
                start.offset,
                self.direction,
                self.count,
            ) {
                let range = SelectionRange::new(
                    TextPosition::new(start.paragraph, span_start),
                    TextPosition::new(start.paragraph, span_end),
                );
                self.apply_step(ctx, range)?;
                *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
                    start.paragraph,
                    span_start,
                )));
                done = characters;
            }
        }

        for _ in done..self.count {
            let range = ctx
                .selection
                .unwrap_or_else(|| SelectionRange::collapsed(start));
            self.apply_step(ctx, range)?;
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for applied in self.applied.iter_mut().rev() {
            applied.undo(ctx)?;
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.apply(ctx)
    }

    fn should_record_history(&self) -> bool {
        self.applied.iter().any(|step| step.should_record_history())
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        // The UI records Backspace/Delete through this command. It is a
        // single-paragraph edit only when every step stayed in one paragraph;
        // redo re-runs `apply` on the same range and document state, so it
        // takes the same steps. `applied` survives undo, unlike the steps'
        // backups, which is why each step records its own classification.
        let mut index = None;
        for step in &self.applied {
            match step.history_render_footprint() {
                HistoryRenderFootprint::SingleParagraph { index: step_index }
                    if index.is_none_or(|index| index == step_index) =>
                {
                    index = Some(step_index);
                }
                _ => return HistoryRenderFootprint::Structural,
            }
        }
        match index {
            Some(index) => HistoryRenderFootprint::SingleParagraph { index },
            None => HistoryRenderFootprint::Structural,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
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

        let document_xml = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for (path, content) in [
                (
                    "[Content_Types].xml",
                    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    #[test]
    fn backspace_inside_a_paragraph_reports_single_paragraph_footprint() {
        let mut word_doc = sample_document("Hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        // Undo and redo both restore/re-apply runs on paragraph 0 only, so
        // they can take the same incremental render path as typing.
        for step in ["undo", "redo"] {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            let footprint = if step == "undo" {
                executor.undo_with_footprint(&mut ctx).unwrap()
            } else {
                executor.redo_with_footprint(&mut ctx).unwrap()
            };
            assert_eq!(
                footprint,
                Some(HistoryRenderFootprint::SingleParagraph { index: 0 }),
                "{step}",
            );
        }
    }

    #[test]
    fn backspace_merge_stays_structural_for_undo_and_redo() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(1, 0)));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(1, 0)),
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        assert_eq!(word_doc.paragraph_count(), 1);
        // Undo consumes the paragraph backup; the redo footprint must still
        // report the merge as structural rather than re-reading empty backups.
        for step in ["undo", "redo"] {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            let footprint = if step == "undo" {
                executor.undo_with_footprint(&mut ctx).unwrap()
            } else {
                executor.redo_with_footprint(&mut ctx).unwrap()
            };
            assert_eq!(
                footprint,
                Some(HistoryRenderFootprint::Structural),
                "{step}"
            );
        }
    }

    #[test]
    fn delete_run_footprint_follows_its_steps() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(1, 3)));
        let mut executor = CommandExecutor::new();
        // Two characters inside paragraph 1, then a run that crosses into
        // paragraph 0 and merges.
        for (offset, count) in [(3usize, 2usize), (1, 3)] {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextRunCommand::new(
                        SelectionRange::collapsed(TextPosition::new(1, offset)),
                        DeleteDirection::Backward,
                        count,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        assert_eq!(
            executor.undo_with_footprint(&mut ctx).unwrap(),
            Some(HistoryRenderFootprint::Structural),
        );
        assert_eq!(
            executor.undo_with_footprint(&mut ctx).unwrap(),
            Some(HistoryRenderFootprint::SingleParagraph { index: 1 }),
        );
        assert_eq!(
            executor.redo_with_footprint(&mut ctx).unwrap(),
            Some(HistoryRenderFootprint::SingleParagraph { index: 1 }),
        );
    }

    #[test]
    fn delete_footprint_is_structural_before_apply() {
        let command = DeleteTextCommand::new(
            SelectionRange::collapsed(TextPosition::new(0, 1)),
            DeleteDirection::Backward,
        );
        assert_eq!(
            command.history_render_footprint(),
            HistoryRenderFootprint::Structural,
        );
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

    fn paragraph_text(paragraph: &Paragraph) -> String {
        paragraph.runs.iter().map(|run| run.text.as_str()).collect()
    }

    fn load_hello() -> WordDocument {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn run_delete(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        range: SelectionRange,
        direction: DeleteDirection,
    ) -> Option<SelectionRange> {
        let mut selection = Some(range);
        let mut ctx = CommandContext::new(word_doc, &mut selection);
        executor
            .execute(Box::new(DeleteTextCommand::new(range, direction)), &mut ctx)
            .expect("delete");
        selection
    }

    /// After undoing a structural edit, every paragraph undo split back off
    /// must still be bound to its own XML element: typing into it has to reach
    /// the saved document.
    #[test]
    fn edits_after_undoing_a_structural_edit_reach_the_saved_xml() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        let load =
            || executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        let probe = load();
        let count = probe.paragraph_count();
        let k = (1..count.saturating_sub(3))
            .find(|&k| {
                (k - 1..=k + 2).all(|index| {
                    probe
                        .paragraph(index)
                        .is_some_and(|p| paragraph_len(p) >= 2)
                        && probe.paragraphs_share_a_container(k - 1, index)
                })
            })
            .expect("four neighbouring body paragraphs with text");
        let at = |doc: &WordDocument, index: usize, offset: usize| {
            doc.model_to_rendered_text_position(TextPosition::new(index, offset))
        };
        let end_of = |doc: &WordDocument, index: usize| {
            let len = paragraph_len(doc.paragraph(index).expect("paragraph"));
            at(doc, index, len)
        };
        type MakeEdit<'a> = Box<dyn Fn(&WordDocument) -> Box<dyn Command> + 'a>;
        let edits: Vec<(&str, std::ops::RangeInclusive<usize>, MakeEdit)> = vec![
            (
                "backspace join",
                k - 1..=k,
                Box::new(|doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(at(doc, k, 0)),
                        DeleteDirection::Backward,
                    ))
                }),
            ),
            (
                "delete join",
                k..=k + 1,
                Box::new(|doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(end_of(doc, k)),
                        DeleteDirection::Forward,
                    ))
                }),
            ),
            (
                "range delete",
                k..=k + 2,
                Box::new(|doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(at(doc, k, 1), at(doc, k + 2, 1)),
                        DeleteDirection::Forward,
                    ))
                }),
            ),
            (
                "range replace",
                k..=k + 2,
                Box::new(|doc| {
                    Box::new(crate::commands::insert_text::InsertTextCommand::new(
                        SelectionRange::new(at(doc, k, 1), at(doc, k + 2, 1)),
                        "置換".to_string(),
                        None,
                    ))
                }),
            ),
        ];

        for (label, restored, edit) in &edits {
            let mut doc = load();
            let mut executor = CommandExecutor::new();
            let mut selection = None;
            {
                let command = edit(&doc);
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor.execute(command, &mut ctx).expect(label);
                assert!(executor.undo(&mut ctx).expect("undo"), "{label}");
            }
            assert_eq!(doc.paragraph_count(), count, "{label}");
            for index in restored.clone() {
                let marker = format!("戻した後の入力{index}");
                let command = Box::new(crate::commands::insert_text::InsertTextCommand::new(
                    SelectionRange::collapsed(at(&doc, index, 0)),
                    marker.clone(),
                    None,
                ));
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor.execute(command, &mut ctx).expect("type");
            }
            let xml = exported_document_xml(&doc);
            for index in restored.clone() {
                assert!(
                    xml.contains(&format!("戻した後の入力{index}")),
                    "{label}: text typed into paragraph {index} is missing from the saved XML"
                );
            }
        }
    }

    /// Backspace or Delete in an empty body paragraph removes it with a join's
    /// undo (paragraph backups plus an XML journal) instead of a snapshot of
    /// the whole document, and undo still restores the exact saved bytes and
    /// leaves both paragraphs editable.
    #[test]
    fn removing_an_empty_paragraph_undoes_without_a_document_snapshot() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        let probe =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        let k = (0..probe.paragraph_count().saturating_sub(2))
            .find(|&k| {
                (k..=k + 1).all(|index| {
                    probe
                        .paragraph(index)
                        .is_some_and(|p| paragraph_len(p) >= 2)
                        && probe.paragraphs_share_a_container(k, index)
                })
            })
            .expect("two neighbouring body paragraphs with text");

        // Backspace at the start of the paragraph after the empty one, and
        // Delete inside the empty one: both remove paragraph k + 1.
        for (label, focus, direction) in [
            ("backspace", k + 2, DeleteDirection::Backward),
            ("delete", k + 1, DeleteDirection::Forward),
        ] {
            let mut doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            let len = paragraph_len(doc.paragraph(k).expect("paragraph"));
            assert!(doc.split_paragraph_at(k, len), "make an empty paragraph");
            assert!(paragraph_is_empty(
                doc.paragraph(k + 1).expect("empty paragraph")
            ));
            let count = doc.paragraph_count();
            let before = doc.export_docx().expect("export");
            let before_model = format!("{:?}", [doc.paragraph(k + 1), doc.paragraph(k + 2)]);

            let at = doc.model_to_rendered_text_position(TextPosition::new(focus, 0));
            let mut command = DeleteTextCommand::new(SelectionRange::collapsed(at), direction);
            let mut selection = None;
            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                command.apply(&mut ctx).expect(label);
            }
            assert_eq!(doc.paragraph_count(), count - 1, "{label}");
            assert!(
                command.previous_snapshot.is_none(),
                "{label}: no document snapshot"
            );
            assert!(command.previous_paragraphs.is_some(), "{label}");
            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                command.undo(&mut ctx).expect("undo");
            }
            assert_eq!(doc.paragraph_count(), count, "{label}");
            assert_eq!(
                format!("{:?}", [doc.paragraph(k + 1), doc.paragraph(k + 2)]),
                before_model,
                "{label}"
            );
            assert!(
                doc.export_docx().expect("export") == before,
                "{label}: undo exports the same bytes"
            );

            for index in [k + 1, k + 2] {
                let at = doc.model_to_rendered_text_position(TextPosition::new(index, 0));
                let mut insert = crate::commands::insert_text::InsertTextCommand::new(
                    SelectionRange::collapsed(at),
                    format!("空段落の後{index}"),
                    None,
                );
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                insert.apply(&mut ctx).expect("type");
            }
            let xml = exported_document_xml(&doc);
            for index in [k + 1, k + 2] {
                assert!(
                    xml.contains(&format!("空段落の後{index}")),
                    "{label}: paragraph {index}"
                );
            }
        }
    }

    fn paragraph_is_empty(paragraph: &Paragraph) -> bool {
        paragraph.runs.iter().all(|run| run.text.is_empty())
    }

    /// A tracked join only marks the paragraph mark deleted in the round-trip
    /// XML, so it undoes through the XML journal and the revision id counter,
    /// not a snapshot of the whole document.
    #[test]
    fn tracked_joins_undo_without_a_document_snapshot() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        for direction in [DeleteDirection::Backward, DeleteDirection::Forward] {
            let mut doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-10-05T00:00:00Z".into()),
                date_utc: None,
            }));
            let k = (1..doc.paragraph_count().saturating_sub(1))
                .find(|&k| {
                    doc.paragraph(k - 1).is_some_and(|p| paragraph_len(p) >= 2)
                        && doc.paragraph(k).is_some_and(|p| paragraph_len(p) >= 2)
                        && doc.paragraphs_share_a_container(k - 1, k)
                })
                .expect("two neighbouring body paragraphs with text");
            let original = doc.export_docx().expect("export");
            let original_paragraphs = format!("{:?}", [doc.paragraph(k - 1), doc.paragraph(k)]);
            let paragraphs = doc.paragraph_count();
            let revision_id = doc.revision_id_counter();
            let caret = match direction {
                DeleteDirection::Backward => TextPosition::new(k, 0),
                DeleteDirection::Forward => TextPosition::new(
                    k - 1,
                    paragraph_len(doc.paragraph(k - 1).expect("paragraph")),
                ),
            };
            let range = SelectionRange::collapsed(doc.model_to_rendered_text_position(caret));
            let mut command = DeleteTextCommand::new(range, direction);
            let mut selection = Some(range);

            let mut applied = None;
            for round in ["apply", "redo"] {
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.apply(&mut ctx).expect(round);
                }
                assert!(
                    command.previous_snapshot.is_none(),
                    "{direction:?}: no snapshot"
                );
                assert!(command.tracked_join_revision_id.is_some(), "{direction:?}");
                assert_eq!(
                    doc.paragraph_count(),
                    paragraphs,
                    "a tracked join keeps both"
                );
                let exported = doc.export_docx().expect("export");
                assert!(exported != original, "{direction:?}: the join is recorded");
                match &applied {
                    None => applied = Some(exported),
                    Some(first) => assert!(exported == *first, "{direction:?}: same revision id"),
                }
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.undo(&mut ctx).expect("undo");
                }
                assert_eq!(doc.revision_id_counter(), revision_id, "{direction:?}");
                assert_eq!(
                    format!("{:?}", [doc.paragraph(k - 1), doc.paragraph(k)]),
                    original_paragraphs,
                    "{direction:?}"
                );
                assert!(
                    doc.export_docx().expect("export") == original,
                    "{direction:?}: {round} undone exports the original bytes"
                );
            }
        }
    }

    /// Structural edits record an XML edit journal instead of copying the
    /// whole round-trip tree; undo must still give back the very bytes the
    /// document exported before, also after a redo.
    #[test]
    fn journaled_structural_edits_undo_to_the_original_export() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        let mut doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        let original = doc.export_docx().expect("export");
        let count = doc.paragraph_count();
        let model = |doc: &WordDocument, range: std::ops::RangeInclusive<usize>| {
            format!(
                "{:?}",
                range.map(|index| doc.paragraph(index)).collect::<Vec<_>>()
            )
        };
        let k = (1..count.saturating_sub(3))
            .find(|&k| {
                (k - 1..=k + 2).all(|index| {
                    doc.paragraph(index).is_some_and(|p| paragraph_len(p) >= 2)
                        && doc.paragraphs_share_a_container(k - 1, index)
                })
            })
            .expect("four neighbouring body paragraphs with text");
        let end_of = |doc: &WordDocument, index: usize| {
            doc.model_to_rendered_text_position(TextPosition::new(
                index,
                paragraph_len(doc.paragraph(index).expect("paragraph")),
            ))
        };
        let at = |doc: &WordDocument, index: usize, offset: usize| {
            doc.model_to_rendered_text_position(TextPosition::new(index, offset))
        };

        type MakeEdit = Box<dyn Fn(&WordDocument) -> Box<dyn Command>>;
        let edits: Vec<(&str, MakeEdit)> = vec![
            (
                "backspace merges with the previous paragraph",
                Box::new(move |doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(at(doc, k, 0)),
                        DeleteDirection::Backward,
                    ))
                }),
            ),
            (
                "delete merges with the next paragraph",
                Box::new(move |doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(end_of(doc, k)),
                        DeleteDirection::Forward,
                    ))
                }),
            ),
            (
                "a range across three paragraphs is deleted",
                Box::new(move |doc| {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(at(doc, k, 1), at(doc, k + 2, 1)),
                        DeleteDirection::Forward,
                    ))
                }),
            ),
            (
                "a range across three paragraphs is replaced",
                Box::new(move |doc| {
                    Box::new(crate::commands::insert_text::InsertTextCommand::new(
                        SelectionRange::new(at(doc, k, 1), at(doc, k + 2, 1)),
                        "置換".to_string(),
                        None,
                    ))
                }),
            ),
        ];

        let original_model = model(&doc, k - 1..=k + 2);
        let mut executor = CommandExecutor::new();
        for (label, edit) in &edits {
            let command = edit(&doc);
            let mut selection = None;
            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor.execute(command, &mut ctx).expect(label);
            }
            assert!(doc.paragraph_count() < count, "{label}: paragraphs fold");
            assert!(
                doc.raw_document
                    .as_mut()
                    .is_some_and(|raw| raw.finish_edit_journal().is_none()),
                "{label}: no journal is left recording"
            );
            for round in ["undo", "undo after redo"] {
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    assert!(executor.undo(&mut ctx).expect("undo"), "{label}");
                }
                assert_eq!(doc.paragraph_count(), count, "{label}: {round}");
                assert_eq!(
                    model(&doc, k - 1..=k + 2),
                    original_model,
                    "{label}: {round}"
                );
                assert!(
                    doc.export_docx().expect("export") == original,
                    "{label}: {round} exports the original bytes"
                );
                if round == "undo" {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    assert!(executor.redo(&mut ctx).expect("redo"), "{label}");
                }
            }
        }
    }

    /// Applies and redoes `command` on `doc`, undoing each time: undo must
    /// give back the bytes and the model from before, and redo the bytes of
    /// the first application, without a snapshot of the whole document.
    fn assert_in_place_delete_round_trips(doc: &mut WordDocument, range: SelectionRange) {
        let original = doc.export_docx().expect("export");
        let original_model = format!("{:?}", doc.document);
        let revision_id = doc.revision_id_counter();
        let mut command = DeleteTextCommand::new(range, DeleteDirection::Backward);
        let mut selection = Some(range);
        let mut applied = None;
        for round in ["apply", "redo"] {
            {
                let mut ctx = CommandContext::new(doc, &mut selection);
                command.apply(&mut ctx).expect(round);
            }
            assert!(command.in_place.is_some(), "{round}: in place");
            assert!(command.previous_snapshot.is_none(), "{round}: no snapshot");
            assert_eq!(
                command.history_render_footprint(),
                HistoryRenderFootprint::Structural,
                "{round}: several paragraphs change"
            );
            let exported = doc.export_docx().expect("export");
            assert!(exported != original, "{round} changes the document");
            match &applied {
                None => applied = Some(exported),
                Some(first) => assert!(exported == *first, "redo saves the same bytes"),
            }
            {
                let mut ctx = CommandContext::new(doc, &mut selection);
                command.undo(&mut ctx).expect("undo");
            }
            assert_eq!(selection, Some(range));
            assert!(
                doc.export_docx().expect("export") == original,
                "{round} undone saves the original bytes"
            );
            assert_eq!(
                format!("{:?}", doc.document),
                original_model,
                "{round} undone"
            );
            assert_eq!(doc.revision_id_counter(), revision_id, "{round} undone");
        }
    }

    /// Deleting a tracked selection across paragraphs only marks it deleted,
    /// and deleting across table cells only clears text: neither removes a
    /// paragraph, so their undo keeps those paragraphs and a journal of the
    /// XML instead of a copy of the whole document (~40 ms on yokohama).
    #[test]
    fn in_place_deletions_undo_without_a_document_snapshot() {
        let mut doc = load_hello();
        doc.set_revision_session(Some(crate::word_document::RevisionSession {
            author: Some("reviewer".into()),
            date: Some("2026-10-05T00:00:00Z".into()),
            date_utc: None,
        }));
        assert_in_place_delete_round_trips(
            &mut doc,
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3)),
        );

        for tracking in [false, true] {
            let mut doc = load_hello();
            doc.insert_table_after_paragraph(0, 1, 2, &[])
                .expect("insert table");
            for (paragraph, text) in [(1, "AB"), (2, "CD")] {
                crate::headless_api::apply_headless_commands_to_document(
                    &mut doc,
                    &jade_docx_headless_api::HeadlessEditRequest {
                        commands: vec![jade_docx_headless_api::HeadlessCommand::InsertText {
                            paragraph,
                            offset: 0,
                            text: text.to_string(),
                        }],
                    },
                )
                .expect("fill cell");
            }
            if tracking {
                doc.set_revision_session(Some(crate::word_document::RevisionSession {
                    author: Some("reviewer".into()),
                    date: Some("2026-10-05T00:00:00Z".into()),
                    date_utc: None,
                }));
            }
            assert_in_place_delete_round_trips(
                &mut doc,
                SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(2, 1)),
            );
        }
    }

    #[test]
    fn a_selection_across_cells_clears_them_without_folding_paragraphs() {
        let mut word_doc = load_hello();
        word_doc
            .insert_table_after_paragraph(0, 1, 2, &[])
            .expect("insert table");
        for (paragraph, text) in [(1, "AB"), (2, "CD")] {
            crate::headless_api::apply_headless_commands_to_document(
                &mut word_doc,
                &jade_docx_headless_api::HeadlessEditRequest {
                    commands: vec![jade_docx_headless_api::HeadlessCommand::InsertText {
                        paragraph,
                        offset: 0,
                        text: text.to_string(),
                    }],
                },
            )
            .expect("fill cell");
        }
        let before = word_doc.paragraph_count();
        let mut executor = CommandExecutor::new();
        let selection = run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(2, 1)),
            DeleteDirection::Forward,
        );
        assert_eq!(
            word_doc.paragraph_count(),
            before,
            "every cell keeps its paragraph"
        );
        assert_eq!(paragraph_text(word_doc.paragraph(1).unwrap()), "A");
        assert_eq!(paragraph_text(word_doc.paragraph(2).unwrap()), "D");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(1, 1)))
        );
        // Backspace right after the table stays put.
        let last = word_doc.paragraph_count() - 1;
        run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(last, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(word_doc.paragraph_count(), before);
        {
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(
            word_doc.paragraph_count(),
            before,
            "undo re-creates no paragraph"
        );
        assert_eq!(paragraph_text(word_doc.paragraph(1).unwrap()), "AB");
        assert_eq!(paragraph_text(word_doc.paragraph(2).unwrap()), "CD");
    }

    #[test]
    fn deleting_across_a_cell_preserves_existing_deletion() {
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
                doc.set_revision_session(Some(crate::word_document::RevisionSession {
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
                        Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
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
                    && run.revision.as_ref().is_some_and(|mark| matches!(
                        mark.kind,
                        crate::document::RevisionKind::Deleted
                    ))));
            }
            let paragraph = doc.paragraph(0).unwrap();
            assert_eq!(
                paragraph
                    .runs
                    .iter()
                    .filter(|run| run.revision.as_ref().is_some_and(|mark| matches!(
                        mark.kind,
                        crate::document::RevisionKind::Deleted
                    )))
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                "Hello"
            );
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
    fn backspace_after_a_section_break_joins_the_sections() {
        use crate::word_document::SectionBreakType;
        let mut word_doc = load_hello();
        word_doc
            .insert_section_break_at(0, 2, SectionBreakType::NextPage)
            .expect("section break");
        assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 2);
        let mut executor = CommandExecutor::new();
        let selection = run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(1, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 1);
        // hello.docx ends its first paragraph with a page break character.
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "Hello\u{c}");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 2)))
        );
        let xml = {
            let exported = word_doc.export_docx().expect("export");
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            xml
        };
        assert_eq!(xml.matches("<w:sectPr").count(), 1, "{xml}");
        {
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 2);
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "He");
    }

    /// A portrait A4 first section (closed by paragraph 3's `w:sectPr`)
    /// followed by landscape A4 paragraphs under the body `w:sectPr`.
    fn portrait_then_landscape_docx() -> Vec<u8> {
        let margins = r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
        let paragraph = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
        let mut body = String::new();
        for index in 0..3 {
            body.push_str(&paragraph(&format!("Portrait {index}")));
        }
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/>{margins}</w:sectPr></w:pPr><w:r><w:t>Portrait 3</w:t></w:r></w:p>"#
        ));
        for _ in 0..3 {
            body.push_str(&paragraph(&"abcd ".repeat(16)));
        }
        body.push_str(&format!(
            r#"<w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>{margins}</w:sectPr>"#
        ));
        build_docx_with_body(&body)
    }

    /// What layout reads from the section model, serialized for comparison.
    fn section_layout(word_doc: &WordDocument) -> (serde_json::Value, serde_json::Value) {
        let document = word_doc.document.as_ref().expect("document");
        (
            serde_json::to_value(&document.page_settings).expect("page settings"),
            serde_json::to_value(&document.sections).expect("sections"),
        )
    }

    /// Deleting the break that closes the first section joins it with the
    /// next, whose properties apply — and the renderer lays the first
    /// section out with `document.page_settings`. That has to follow the
    /// join, as a reload of the saved file does, instead of keeping the
    /// removed section's portrait page.
    #[test]
    fn joining_the_first_section_lays_it_out_like_a_reload() {
        let data = portrait_then_landscape_docx();
        for (position, direction) in [
            (TextPosition::new(4, 0), DeleteDirection::Backward),
            (
                TextPosition::new(3, "Portrait 3".len()),
                DeleteDirection::Forward,
            ),
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            let loaded = section_layout(&word_doc);
            let width = |word_doc: &WordDocument| {
                word_doc
                    .document
                    .as_ref()
                    .and_then(|document| document.page_settings.as_ref())
                    .and_then(|settings| settings.width)
            };
            assert_eq!(width(&word_doc), Some(595.3));
            assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 2);

            let mut executor = CommandExecutor::new();
            run_delete(
                &mut word_doc,
                &mut executor,
                SelectionRange::collapsed(position),
                direction,
            );
            assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 1);
            assert_eq!(width(&word_doc), Some(841.9), "{direction:?}");
            let exported = word_doc.export_docx().expect("export");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(
                section_layout(&word_doc),
                section_layout(&reloaded),
                "{direction:?}"
            );

            // Undo brings the break back, and the portrait first page with it.
            {
                let mut selection = None;
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                assert!(executor.undo(&mut ctx).expect("undo"));
            }
            assert_eq!(section_layout(&word_doc), loaded, "{direction:?}");
        }
    }

    /// Joining two paragraphs inside the first section moves the start of
    /// the next one up by a paragraph; a stale range would lay the first
    /// landscape paragraph out on the portrait page.
    #[test]
    fn joining_paragraphs_moves_the_following_section_ranges() {
        let data = portrait_then_landscape_docx();
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        let mut executor = CommandExecutor::new();
        run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(2, 0)),
            DeleteDirection::Backward,
        );
        let ranges = |word_doc: &WordDocument| {
            word_doc
                .document
                .as_ref()
                .unwrap()
                .sections
                .iter()
                .map(|section| (section.start, section.end))
                .collect::<Vec<_>>()
        };
        assert_eq!(ranges(&word_doc), [(0, 3), (3, 6)]);
        let exported = word_doc.export_docx().expect("export");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(section_layout(&word_doc), section_layout(&reloaded));
    }

    #[test]
    fn backspace_after_a_reference_mark_deletes_the_footnote() {
        use crate::commands::insert_footnote::InsertFootnoteCommand;
        let mut word_doc = load_hello();
        let mut executor = CommandExecutor::new();
        {
            let range = SelectionRange::collapsed(TextPosition::new(0, 5));
            let mut selection = Some(range);
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, "note".to_string())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }
        let notes = |doc: &WordDocument| {
            doc.document
                .as_ref()
                .map(|document| {
                    document
                        .footnotes
                        .values()
                        .filter(|note| note.note_type.is_none())
                        .count()
                })
                .unwrap_or(0)
        };
        assert_eq!(notes(&word_doc), 1);
        // The mark renders as "1" after "Hello": rendered offset 6.
        let selection = run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(0, 6)),
            DeleteDirection::Backward,
        );
        assert_eq!(notes(&word_doc), 0);
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "Hello\u{c}");
        assert!(word_doc
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .all(|run| run.footnote_reference_ids.is_empty()));
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 5)))
        );
        {
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(notes(&word_doc), 1);
    }

    #[test]
    fn headless_backspace_after_a_reference_mark_deletes_the_footnote() {
        use crate::commands::insert_footnote::InsertFootnoteCommand;
        let mut word_doc = load_hello();
        {
            let range = SelectionRange::collapsed(TextPosition::new(0, 5));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, "note".to_string())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }
        let count_notes = |doc: &WordDocument| {
            doc.document
                .as_ref()
                .map(|document| {
                    document
                        .footnotes
                        .values()
                        .filter(|note| note.note_type.is_none())
                        .count()
                })
                .unwrap_or(0)
        };
        assert_eq!(count_notes(&word_doc), 1);
        // The editor's Backspace goes through the headless run command.
        crate::headless_api::apply_headless_commands_to_document(
            &mut word_doc,
            &jade_docx_headless_api::HeadlessEditRequest {
                commands: vec![jade_docx_headless_api::HeadlessCommand::DeleteText {
                    paragraph: 0,
                    offset: 6,
                    direction: jade_docx_headless_api::HeadlessDeleteDirection::Backward,
                    count: 1,
                }],
            },
        )
        .expect("backspace after the mark");
        assert_eq!(count_notes(&word_doc), 0);
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "Hello\u{c}");
    }

    #[test]
    fn deletes_selected_range() {
        let mut word_doc = sample_document("abcdef");
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 1),
            TextPosition::new(0, 4),
        ));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(0, 4)),
                        DeleteDirection::Forward,
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
        assert_eq!(paragraph.runs.first().unwrap().text, "aef");
        assert_eq!(selection.unwrap().focus.offset, 1);
    }

    #[test]
    fn deletion_rebases_comment_ranges_and_markers() {
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
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                    &mut ctx,
                )
                .expect("delete text");
        }

        let paragraph = word_doc.paragraph(0).expect("paragraph");
        assert_eq!(
            (
                paragraph.comment_ranges[0].start,
                paragraph.comment_ranges[0].end
            ),
            (0, 3)
        );
        assert_eq!(paragraph.comment_markers[0].position, 3);
    }

    #[test]
    fn deletes_previous_grapheme_on_backward() {
        let mut word_doc = sample_document("a😀b");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Backward,
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
        assert_eq!(paragraph.runs.first().unwrap().text, "ab");
        assert_eq!(selection.unwrap().focus.offset, 1);
    }

    #[test]
    fn deletes_next_grapheme_on_forward() {
        let mut word_doc = sample_document("ab😀c");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 2)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 2)),
                        DeleteDirection::Forward,
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
        assert_eq!(paragraph.runs.first().unwrap().text, "abc");
        assert_eq!(selection.unwrap().focus.offset, 2);
    }

    #[test]
    fn undo_restores_state() {
        let mut word_doc = sample_document("hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Backward,
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
        assert_eq!(paragraph.runs.first().unwrap().text, "hell");

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
        assert_eq!(paragraph.runs.first().unwrap().text, "hello");
        assert_eq!(selection.unwrap().focus.offset, 5);
    }

    #[test]
    fn noop_backspace_at_empty_document_start_does_not_enter_history() {
        let mut word_doc = sample_document("");
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        original_selection,
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .expect("noop delete should apply");
        }

        assert!(!executor.can_undo());
        assert!(!executor.can_redo());

        let paragraph = word_doc.paragraph(0).expect("paragraph remains loaded");
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn noop_delete_at_document_end_does_not_enter_history() {
        let mut word_doc = sample_document("hello");
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 5));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        original_selection,
                        DeleteDirection::Forward,
                    )),
                    &mut ctx,
                )
                .expect("noop delete should apply");
        }

        assert!(!executor.can_undo());
        assert!(!executor.can_redo());

        let paragraph = word_doc.paragraph(0).expect("paragraph remains loaded");
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "hello");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn noop_delete_does_not_clear_redo_history() {
        let mut word_doc = sample_document("hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .expect("delete should apply");
            assert!(executor.undo(&mut ctx).expect("undo delete"));
        }
        assert!(!executor.can_undo());
        assert!(executor.can_redo());

        {
            let noop_start = SelectionRange::collapsed(TextPosition::new(0, 0));
            selection = Some(noop_start);
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        noop_start,
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .expect("noop delete should apply");
        }

        assert!(!executor.can_undo());
        assert!(executor.can_redo());
        let paragraph = word_doc.paragraph(0).expect("paragraph remains loaded");
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "hello");
    }

    #[test]
    fn delete_with_record_changes_creates_revision_wrapper() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut options = Options::default();
        options.set_record_changes(true);
        options.set_revision_author(Some("jade tester".to_string()));

        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &options)).expect("load docx");

        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 4),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 4), TextPosition::new(0, 5)),
                        DeleteDirection::Forward,
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
            document_xml.contains(r#"<w:del "#),
            "exported XML should contain deletion wrapper: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"w:author="jade tester""#),
            "deletion revision should include provided author metadata: {document_xml}"
        );
        assert!(
            document_xml.contains("<w:delText>o</w:delText>"),
            "deleted character should be wrapped in w:delText: {document_xml}"
        );
    }

    #[test]
    fn consecutive_backspaces_share_one_deletion_revision() {
        let mut word_doc = sample_document("abc");
        word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
            author: Some("jade tester".to_string()),
            date: Some("2026-09-13T00:00:00Z".to_string()),
            date_utc: Some("2026-09-13T00:00:00Z".to_string()),
        }));
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 3)));
        let mut executor = CommandExecutor::new();

        for _ in 0..3 {
            let range = selection.expect("caret after tracked deletion");
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                    &mut ctx,
                )
                .expect("backspace");
        }

        let revisions = crate::headless_api::revision_snapshots(&word_doc);
        assert_eq!(revisions.len(), 1, "one continuous edit is one revision");
        assert_eq!(revisions[0].kind, "deleted");
        assert_eq!(revisions[0].text, "abc");
        assert_eq!(revisions[0].author.as_deref(), Some("jade tester"));
    }

    #[test]
    fn deleting_inside_existing_deletion_keeps_original_review_text() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        for (direction, offset) in [
            (DeleteDirection::Backward, 2),
            (DeleteDirection::Forward, 1),
        ] {
            for track_changes in [false, true] {
                let mut word_doc =
                    executor::block_on(WordDocument::load(&data, &Options::default()))
                        .expect("load docx");
                if track_changes {
                    word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                        author: Some("reviewer".to_string()),
                        date: Some("2026-09-24T00:00:00Z".to_string()),
                        date_utc: None,
                    }));
                }
                let mut executor = CommandExecutor::new();
                let range = SelectionRange::collapsed(TextPosition::new(0, offset));
                run_delete(&mut word_doc, &mut executor, range, direction);

                let deleted: String = word_doc
                    .paragraph(0)
                    .expect("paragraph")
                    .runs
                    .iter()
                    .filter(|run| {
                        run.revision.as_ref().is_some_and(|mark| {
                            matches!(mark.kind, crate::document::RevisionKind::Deleted)
                        })
                    })
                    .map(|run| run.text.as_str())
                    .collect();
                assert_eq!(deleted, "Hello", "{direction:?}, {track_changes}");
                let exported = word_doc.export_docx().expect("export docx");
                let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
                let mut xml = String::new();
                archive
                    .by_name("word/document.xml")
                    .expect("document.xml")
                    .read_to_string(&mut xml)
                    .expect("read XML");
                assert!(xml.contains("<w:delText>Hello</w:delText>"));
            }
        }
    }

    #[test]
    fn deleting_across_existing_deletion_only_removes_ordinary_text() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        for track_changes in [false, true] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            {
                let paragraph = word_doc.paragraph_mut(0).expect("paragraph");
                apply_text_edit(&mut paragraph.runs, 0, 0, "A", &ZeroWidthRuns::default());
                apply_text_edit(&mut paragraph.runs, 6, 6, "next", &ZeroWidthRuns::default());
            }
            word_doc.sync_paragraph_text_preserving_drawings(0);
            if track_changes {
                word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                    author: Some("reviewer".to_string()),
                    date: Some("2026-09-24T00:00:00Z".to_string()),
                    date_utc: None,
                }));
            }
            let original = word_doc.paragraph(0).expect("paragraph").runs.clone();
            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 8));
            let mut selection = Some(range);
            let mut command = DeleteTextCommand::new(range, DeleteDirection::Backward);
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                command.apply(&mut ctx).expect("delete mixed range");
            }
            let paragraph = word_doc.paragraph(0).expect("paragraph");
            let deleted: String = paragraph
                .runs
                .iter()
                .filter(|run| {
                    run.revision.as_ref().is_some_and(|mark| {
                        matches!(mark.kind, crate::document::RevisionKind::Deleted)
                    })
                })
                .map(|run| run.text.as_str())
                .collect();
            assert!(deleted.contains("Hello"), "track changes: {track_changes}");
            assert_eq!(
                paragraph_text(paragraph),
                if track_changes {
                    "AHellonext"
                } else {
                    "Helloxt"
                },
                "track changes: {track_changes}"
            );
            let exported = word_doc.export_docx().expect("export");
            let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .expect("document.xml")
                .read_to_string(&mut xml)
                .expect("read XML");
            assert!(xml.contains("<w:delText>Hello</w:delText>"), "{xml}");
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                command.undo(&mut ctx).expect("undo deletion");
            }
            let restored = word_doc.paragraph(0).expect("paragraph");
            assert_eq!(paragraph_text(restored), "AHellonext");
            assert_eq!(restored.runs.len(), original.len());
            for (actual, expected) in restored.runs.iter().zip(&original) {
                assert_eq!(actual.text, expected.text);
                assert_eq!(actual.revision, expected.revision);
            }
        }
    }

    #[test]
    fn deleting_across_paragraphs_preserves_review_deletions() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            let before_count = word_doc.paragraph_count();
            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 3));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                        &mut ctx,
                    )
                    .expect("delete across paragraphs");
            }
            assert_eq!(word_doc.paragraph_count(), before_count - 1);
            let paragraph = word_doc.paragraph(0).expect("merged paragraph");
            assert_eq!(paragraph_text(paragraph), "Hellold");
            assert!(paragraph.runs.iter().any(|run| {
                run.text == "Hello"
                    && run
                        .revision
                        .as_ref()
                        .is_some_and(|mark| mark.kind == crate::document::RevisionKind::Deleted)
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
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor.undo(&mut ctx).expect("undo");
            }
            assert_eq!(word_doc.paragraph_count(), before_count);
            assert_eq!(
                paragraph_text(word_doc.paragraph(0).expect("first")),
                "Hello"
            );
            assert_eq!(
                paragraph_text(word_doc.paragraph(1).expect("second")),
                "World"
            );
        }
    }

    #[test]
    fn tracked_range_deletion_refuses_an_existing_paragraph_mark() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        let range = SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(2, 3));
        for direction in [DeleteDirection::Backward, DeleteDirection::Forward] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: None,
                date_utc: None,
            }));
            let before_count = word_doc.paragraph_count();
            let before_xml = word_doc
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML");
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let error = executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, direction)),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .expect_err("an existing paragraph mark must not be overwritten");
            assert!(error.to_string().contains("already has a review marker"));
            assert_eq!(word_doc.paragraph_count(), before_count);
            assert_eq!(selection, Some(range));
            assert_eq!(
                word_doc
                    .raw_document
                    .as_ref()
                    .expect("raw XML")
                    .to_string()
                    .expect("serialize XML"),
                before_xml
            );
        }
    }

    #[test]
    fn tracked_range_deletion_records_text_and_paragraph_mark_atomically() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        for direction in [DeleteDirection::Backward, DeleteDirection::Forward] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-09-25T00:00:00Z".into()),
                date_utc: None,
            }));
            let original_text = (0..2)
                .map(|index| paragraph_text(word_doc.paragraph(index).unwrap()))
                .collect::<Vec<_>>();
            let before_xml = word_doc
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML");
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, direction)),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .expect("record selected text and paragraph mark");
            assert_eq!(word_doc.paragraph_count(), 2);
            let exported = word_doc.export_docx().expect("export");
            let mut reloaded =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("reload");
            let xml = reloaded
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML");
            assert!(xml.contains("<w:del "), "{xml}");
            assert!(xml.contains("<w:delText>llo</w:delText>"), "{xml}");
            assert!(xml.contains("<w:delText>Wor</w:delText>"), "{xml}");
            assert_eq!(reloaded.accept_revisions(None, None, None, None), Ok(4));
            assert_eq!(reloaded.paragraph_count(), 1);
            assert_eq!(paragraph_text(reloaded.paragraph(0).unwrap()), "Held");
            let mut rejected =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("reload rejection copy");
            assert_eq!(rejected.reject_revisions(None, None, None, None), Ok(4));
            assert_eq!(rejected.paragraph_count(), 2);
            assert_eq!(
                paragraph_text(rejected.paragraph(0).unwrap()),
                original_text[0]
            );
            assert_eq!(
                paragraph_text(rejected.paragraph(1).unwrap()),
                original_text[1]
            );
            executor
                .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("undo");
            assert_eq!(selection, Some(range));
            assert_eq!(
                word_doc.raw_document.as_ref().unwrap().to_string().unwrap(),
                before_xml
            );
        }
    }

    #[test]
    fn tracked_range_deletion_records_every_boundary_across_three_paragraphs() {
        let data = build_docx_with_body(
            r#"<w:p><w:r><w:t>Aaa</w:t></w:r></w:p><w:p><w:r><w:t>Bbb</w:t></w:r></w:p><w:p><w:r><w:t>Ccc</w:t></w:r></w:p>"#,
        );
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
        word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
            author: Some("reviewer".into()),
            date: Some("2026-09-25T00:00:00Z".into()),
            date_utc: None,
        }));
        let range = SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(2, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect("record all selected text and both paragraph marks");
        let exported = word_doc.export_docx().expect("export");
        let mut accepted = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reload acceptance copy");
        assert_eq!(accepted.accept_revisions(None, None, None, None), Ok(5));
        assert_eq!(accepted.paragraph_count(), 1);
        assert_eq!(paragraph_text(accepted.paragraph(0).unwrap()), "Ac");
        let mut rejected = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reload rejection copy");
        assert_eq!(rejected.reject_revisions(None, None, None, None), Ok(5));
        assert_eq!(rejected.paragraph_count(), 3);
        assert_eq!(paragraph_text(rejected.paragraph(0).unwrap()), "Aaa");
        assert_eq!(paragraph_text(rejected.paragraph(1).unwrap()), "Bbb");
        assert_eq!(paragraph_text(rejected.paragraph(2).unwrap()), "Ccc");
    }

    #[test]
    fn tracked_backspace_and_delete_record_reviewable_paragraph_boundaries() {
        let data =
            std::fs::read(fixture_path("numbering-with-review.docx")).expect("read review fixture");
        for (range, direction) in [
            (
                SelectionRange::collapsed(TextPosition::new(1, 0)),
                DeleteDirection::Backward,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 5)),
                DeleteDirection::Forward,
            ),
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-09-25T00:00:00Z".into()),
                date_utc: None,
            }));
            let before_count = word_doc.paragraph_count();
            let before_xml = word_doc
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML");
            let mark_id = word_doc
                .snapshot()
                .expect("snapshot")
                .next_revision_id
                .to_string();
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, direction)),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .expect("record paragraph break deletion");
            assert_eq!(word_doc.paragraph_count(), before_count);
            let exported = word_doc.export_docx().expect("export");
            let mut reloaded =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("reload");
            let marked_xml = reloaded
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML");
            assert!(marked_xml.contains(&format!("<w:del w:id=\"{mark_id}\"")));
            assert!(marked_xml.contains("w:author=\"reviewer\""));
            let duplicate = executor.execute(
                Box::new(DeleteTextCommand::new(range, direction)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            );
            assert!(duplicate
                .expect_err("a second deletion of the same boundary must fail")
                .to_string()
                .contains("already has a review marker"));
            assert_eq!(
                word_doc
                    .raw_document
                    .as_ref()
                    .expect("raw XML")
                    .to_string()
                    .expect("serialize XML"),
                marked_xml
            );
            assert_eq!(
                reloaded.reject_revisions(None, None, None, Some(&mark_id)),
                Ok(1)
            );
            assert_eq!(reloaded.paragraph_count(), before_count);
            assert!(reloaded
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML")
                .contains("w:id=\"2\""));
            let mut accepted =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("reload acceptance copy");
            assert_eq!(
                accepted.accept_revisions(None, None, None, Some(&mark_id)),
                Ok(1)
            );
            assert_eq!(accepted.paragraph_count(), before_count - 1);
            assert!(accepted
                .raw_document
                .as_ref()
                .expect("raw XML")
                .to_string()
                .expect("serialize XML")
                .contains("w:id=\"2\""));
            executor
                .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("undo");
            assert_eq!(word_doc.paragraph_count(), before_count);
            assert_eq!(selection, Some(range));
            assert_eq!(
                word_doc
                    .raw_document
                    .as_ref()
                    .expect("raw XML")
                    .to_string()
                    .expect("serialize XML"),
                before_xml
            );
        }
    }

    #[test]
    fn delete_syncs_raw_document_xml() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Backward,
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
        assert!(raw.contains("Hell"));
        assert!(!raw.contains("Hello"));
    }

    #[test]
    fn deletes_selection_spanning_multiple_paragraphs() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 2),
            TextPosition::new(1, 2),
        ));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2)),
                        DeleteDirection::Forward,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 1);
        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.elements.len(), 1);
        let text: String = body
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap();
        assert_eq!(text, "Herld");
        assert_eq!(selection.unwrap().focus, TextPosition::new(0, 2));
    }

    #[test]
    fn backspace_merges_with_previous_paragraph() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(1, 0)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(1, 0)),
                        DeleteDirection::Backward,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 1);
        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.elements.len(), 1);
        let text: String = body
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap();
        assert_eq!(text, "HelloWorld");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 5);
    }

    #[test]
    fn delete_merges_with_next_paragraph() {
        let mut word_doc = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 5)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Forward,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }

        assert_eq!(word_doc.paragraph_count(), 1);
        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.elements.len(), 1);
        let text: String = body
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap();
        assert_eq!(text, "HelloWorld");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 5);
    }

    #[test]
    fn backspace_and_delete_take_one_grapheme_cluster() {
        // Word deletes what the caret steps over: an IVS (葛 + VS17), a flag
        // (two regional indicators) or a letter with a combining mark is one
        // character to the person editing. The browser asks the run command
        // for that many code points.
        let ivs = "葛\u{E0100}";
        let flag = "🇯🇵";
        let combining = "e\u{301}";
        let text = format!("a{ivs}{flag}{combining}");
        let mut word_doc = sample_document(&text);
        let end = text.len();
        assert_eq!(
            grapheme_delete_count(
                &word_doc,
                TextPosition::new(0, end),
                DeleteDirection::Backward
            ),
            2
        );
        assert_eq!(
            grapheme_delete_count(&word_doc, TextPosition::new(0, 1), DeleteDirection::Forward),
            2
        );
        assert_eq!(
            grapheme_delete_count(
                &word_doc,
                TextPosition::new(0, 1),
                DeleteDirection::Backward
            ),
            1
        );
        // At a paragraph edge the step joins paragraphs: one character.
        assert_eq!(
            grapheme_delete_count(
                &word_doc,
                TextPosition::new(0, 0),
                DeleteDirection::Backward
            ),
            1
        );

        let mut executor = CommandExecutor::new();
        for (caret, direction, expected) in [
            (end, DeleteDirection::Backward, format!("a{ivs}{flag}")),
            (
                end - combining.len(),
                DeleteDirection::Backward,
                format!("a{ivs}"),
            ),
            (1, DeleteDirection::Forward, "a".to_string()),
        ] {
            let caret = TextPosition::new(0, caret);
            let count = grapheme_delete_count(&word_doc, caret, direction);
            let mut selection = Some(SelectionRange::collapsed(caret));
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextRunCommand::new(
                        SelectionRange::collapsed(caret),
                        direction,
                        count,
                    )),
                    &mut ctx,
                )
                .expect("delete");
            assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), expected);
        }
    }

    fn load_body(body_xml: &str) -> WordDocument {
        executor::block_on(WordDocument::load(
            &build_docx_with_body(body_xml),
            &Options::default(),
        ))
        .expect("load docx")
    }

    fn exported_document_xml(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document part")
            .read_to_string(&mut xml)
            .expect("read document part");
        xml
    }

    fn paragraph_style(word_doc: &WordDocument, index: usize) -> Option<String> {
        word_doc
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.style.as_deref().map(str::to_string))
    }

    /// An empty paragraph above a heading (「第1条」) followed by its body.
    const EMPTY_BEFORE_HEADING: &str = r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:bookmarkStart w:id="7" w:name="keep"/><w:bookmarkEnd w:id="7"/></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:t>第1条</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p>"#;

    #[test]
    fn backspace_after_an_empty_paragraph_keeps_the_current_paragraph_formatting() {
        // Word: Backspace at the start of a paragraph whose previous paragraph
        // is empty removes that empty paragraph; the heading keeps its style
        // and numbering instead of taking the empty paragraph's.
        let mut word_doc = load_body(EMPTY_BEFORE_HEADING);
        let mut executor = CommandExecutor::new();
        let selection = run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(1, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(word_doc.paragraph_count(), 2);
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "第1条");
        assert_eq!(paragraph_style(&word_doc, 0).as_deref(), Some("Heading1"));
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
        let xml = exported_document_xml(&word_doc);
        assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#), "{xml}");
        assert!(xml.contains(r#"<w:numId w:val="3"/>"#), "{xml}");
        assert!(!xml.contains(r#"<w:jc w:val="center"/>"#), "{xml}");
        // The empty paragraph's bookmark moves to the start of the heading.
        assert!(
            xml.contains(r#"<w:bookmarkStart w:id="7" w:name="keep"/>"#),
            "{xml}"
        );

        let mut selection = None;
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo"));
        assert_eq!(word_doc.paragraph_count(), 3);
        assert_eq!(paragraph_style(&word_doc, 0), None);
        assert_eq!(paragraph_style(&word_doc, 1).as_deref(), Some("Heading1"));
        let xml = exported_document_xml(&word_doc);
        assert_eq!(xml.matches(r#"w:name="keep""#).count(), 1, "{xml}");
    }

    #[test]
    fn delete_in_an_empty_paragraph_keeps_the_next_paragraph_formatting() {
        let mut word_doc = load_body(EMPTY_BEFORE_HEADING);
        let mut executor = CommandExecutor::new();
        let selection = run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(0, 0)),
            DeleteDirection::Forward,
        );
        assert_eq!(word_doc.paragraph_count(), 2);
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "第1条");
        assert_eq!(paragraph_style(&word_doc, 0).as_deref(), Some("Heading1"));
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
        let xml = exported_document_xml(&word_doc);
        assert!(xml.contains(r#"<w:numId w:val="3"/>"#), "{xml}");
        assert!(!xml.contains(r#"<w:jc w:val="center"/>"#), "{xml}");
    }

    #[test]
    fn joining_into_a_non_empty_paragraph_still_keeps_the_first_paragraph_formatting() {
        // Only an empty paragraph gives way; two paragraphs with text join
        // into the first one's formatting as before.
        let mut word_doc = load_body(
            r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>Head</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Tail</w:t></w:r></w:p>"#,
        );
        let mut executor = CommandExecutor::new();
        run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(1, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(paragraph_text(word_doc.paragraph(0).unwrap()), "HeadTail");
        assert_eq!(paragraph_style(&word_doc, 0), None);
    }

    #[test]
    fn an_empty_paragraph_ending_a_section_is_joined_as_a_section_break() {
        // Deleting a section break joins the sections; that path is kept
        // rather than dropping the empty paragraph with its sectPr.
        let mut word_doc = load_body(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Next</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="16838" w:h="11906"/></w:sectPr>"#,
        );
        assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 2);
        let mut executor = CommandExecutor::new();
        run_delete(
            &mut word_doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(1, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(word_doc.document.as_ref().unwrap().sections.len(), 1);
        let xml = exported_document_xml(&word_doc);
        assert_eq!(xml.matches("<w:sectPr").count(), 1, "{xml}");
    }

    /// A portrait section ended by "DDDD", then a landscape one.
    const TWO_SECTIONS: &str = r#"<w:p><w:r><w:t>AAAA</w:t></w:r></w:p><w:p><w:r><w:t>BBBB</w:t></w:r></w:p><w:p><w:r><w:t>CCCC</w:t></w:r></w:p><w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr><w:r><w:t>DDDD</w:t></w:r></w:p><w:p><w:r><w:t>EEEE</w:t></w:r></w:p><w:p><w:r><w:t>FFFF</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/></w:sectPr>"#;

    #[test]
    fn folding_paragraphs_keeps_the_section_break_of_the_surviving_mark() {
        // Word deletes the paragraph marks inside a join or a selection and
        // keeps the last one. The section break stored on that mark stays,
        // now ending the merged paragraph; a break on a deleted mark goes.
        fn sections(doc: &WordDocument) -> String {
            format!("{:?}", doc.document.as_ref().unwrap().sections)
        }
        fn section_of(doc: &WordDocument, index: usize) -> String {
            format!(
                "{:?}",
                doc.paragraph(index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.section.as_ref())
            )
        }
        fn texts(doc: &WordDocument) -> Vec<String> {
            (0..doc.paragraph_count())
                .map(|index| paragraph_text(doc.paragraph(index).unwrap()))
                .collect()
        }
        fn reload(bytes: &[u8]) -> WordDocument {
            executor::block_on(WordDocument::load(bytes, &Options::default())).expect("reload")
        }

        let original = load_body(TWO_SECTIONS);
        let original_bytes = original.export_docx().expect("export");
        let original_sections = sections(&original);
        let original_texts = texts(&original);
        let portrait = section_of(&original, 3);
        assert_ne!(portrait, "None");
        assert_eq!(original.document.as_ref().unwrap().sections.len(), 2);

        type MakeEdit = fn() -> Box<dyn Command>;
        // (label, edit, paragraph texts, paragraph ending section 1)
        let cases: [(&str, MakeEdit, &[&str], Option<usize>); 7] = [
            (
                "backspace at the start of the paragraph ending the section",
                || {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(3, 0)),
                        DeleteDirection::Backward,
                    ))
                },
                &["AAAA", "BBBB", "CCCCDDDD", "EEEE", "FFFF"],
                Some(2),
            ),
            (
                "delete at the end of the paragraph before it",
                || {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(2, 4)),
                        DeleteDirection::Forward,
                    ))
                },
                &["AAAA", "BBBB", "CCCCDDDD", "EEEE", "FFFF"],
                Some(2),
            ),
            (
                "deleting a range that ends inside it",
                || {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(3, 1)),
                        DeleteDirection::Backward,
                    ))
                },
                &["AAAA", "BDDD", "EEEE", "FFFF"],
                Some(1),
            ),
            (
                "clearing a range that ends inside it",
                || {
                    Box::new(crate::commands::insert_text::InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(3, 1)),
                        String::new(),
                        None,
                    ))
                },
                &["AAAA", "BDDD", "EEEE", "FFFF"],
                Some(1),
            ),
            (
                "replacing a range that ends inside it",
                || {
                    Box::new(crate::commands::insert_text::InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(1, 1), TextPosition::new(3, 1)),
                        "x".to_string(),
                        None,
                    ))
                },
                &["AAAA", "BxDDD", "EEEE", "FFFF"],
                Some(1),
            ),
            (
                "deleting a range that starts inside it",
                || {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(3, 1), TextPosition::new(4, 1)),
                        DeleteDirection::Backward,
                    ))
                },
                &["AAAA", "BBBB", "CCCC", "DEEE", "FFFF"],
                None,
            ),
            (
                "clearing a range that starts inside it",
                || {
                    Box::new(crate::commands::insert_text::InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(3, 1), TextPosition::new(4, 1)),
                        String::new(),
                        None,
                    ))
                },
                &["AAAA", "BBBB", "CCCC", "DEEE", "FFFF"],
                None,
            ),
        ];

        for (label, edit, expected_texts, terminator) in cases {
            let mut doc = load_body(TWO_SECTIONS);
            let mut executor = CommandExecutor::new();
            let mut selection = None;
            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor.execute(edit(), &mut ctx).expect(label);
            }
            let check_edited = |doc: &WordDocument, round: &str| {
                assert_eq!(texts(doc), expected_texts, "{label}: {round}");
                let expected_sections = if terminator.is_some() { 2 } else { 1 };
                assert_eq!(
                    doc.document.as_ref().unwrap().sections.len(),
                    expected_sections,
                    "{label}: {round}"
                );
                for index in 0..doc.paragraph_count() {
                    let expected = if Some(index) == terminator {
                        portrait.as_str()
                    } else {
                        "None"
                    };
                    assert_eq!(
                        section_of(doc, index),
                        expected,
                        "{label}: {round} #{index}"
                    );
                }
                let bytes = doc.export_docx().expect("export");
                let xml = exported_document_xml(doc);
                assert_eq!(
                    xml.matches("<w:sectPr").count(),
                    expected_sections,
                    "{label}: {round}: {xml}"
                );
                let reloaded = reload(&bytes);
                assert_eq!(texts(&reloaded), expected_texts, "{label}: {round}");
                assert_eq!(sections(doc), sections(&reloaded), "{label}: {round}");
                if let Some(index) = terminator {
                    assert_eq!(section_of(&reloaded, index), portrait, "{label}: {round}");
                }
            };
            check_edited(&doc, "apply");

            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                assert!(executor.undo(&mut ctx).expect("undo"), "{label}");
            }
            assert_eq!(texts(&doc), original_texts, "{label}: undo");
            assert_eq!(sections(&doc), original_sections, "{label}: undo");
            assert_eq!(section_of(&doc, 3), portrait, "{label}: undo");
            assert!(
                doc.export_docx().expect("export") == original_bytes,
                "{label}: undo exports the original bytes"
            );

            {
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                assert!(executor.redo(&mut ctx).expect("redo"), "{label}");
            }
            check_edited(&doc, "redo");
        }
    }

    #[test]
    fn joining_two_section_ending_paragraphs_keeps_the_second_break() {
        // Backspace deletes the first mark's break and keeps the second's.
        let mut doc = load_body(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr><w:r><w:t>AAAA</w:t></w:r></w:p><w:p><w:pPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:titlePg/></w:sectPr></w:pPr><w:r><w:t>BBBB</w:t></w:r></w:p><w:p><w:r><w:t>CCCC</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/></w:sectPr>"#,
        );
        let original = doc.export_docx().expect("export");
        let original_sections = format!("{:?}", doc.document.as_ref().unwrap().sections);
        let second = format!("{:?}", doc.paragraph(1).unwrap().properties);
        assert_eq!(doc.document.as_ref().unwrap().sections.len(), 3);
        let mut executor = CommandExecutor::new();
        run_delete(
            &mut doc,
            &mut executor,
            SelectionRange::collapsed(TextPosition::new(1, 0)),
            DeleteDirection::Backward,
        );
        assert_eq!(paragraph_text(doc.paragraph(0).unwrap()), "AAAABBBB");
        assert_eq!(
            format!("{:?}", doc.paragraph(0).unwrap().properties),
            second
        );
        assert_eq!(doc.document.as_ref().unwrap().sections.len(), 2);
        let xml = exported_document_xml(&doc);
        assert_eq!(xml.matches("<w:sectPr").count(), 2, "{xml}");
        assert!(
            xml.contains(r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:titlePg/></w:sectPr>"#),
            "{xml}"
        );
        assert!(!xml.contains(r#"w:w="11906""#), "{xml}");
        let reloaded = executor::block_on(WordDocument::load(
            &doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(
            format!("{:?}", doc.document.as_ref().unwrap().sections),
            format!("{:?}", reloaded.document.as_ref().unwrap().sections)
        );

        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo"));
        assert_eq!(
            format!("{:?}", doc.document.as_ref().unwrap().sections),
            original_sections
        );
        assert!(doc.export_docx().expect("export") == original);
    }

    #[test]
    fn adjacent_paragraph_merge_preserves_tail_bookmark_xml() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p>
</w:body></w:document>"#;
        for (position, direction) in [
            (TextPosition::new(1, 0), DeleteDirection::Backward),
            (TextPosition::new(0, 5), DeleteDirection::Forward),
        ] {
            let mut document = jade_docx_reader::DocumentParser::new()
                .parse_document_xml(xml)
                .expect("parse document");
            let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
            crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
            let mut word_doc = WordDocument::from_document(document);
            word_doc.raw_document = Some(raw);

            let mut selection = Some(SelectionRange::collapsed(position));
            let mut executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(position),
                        direction,
                    )),
                    &mut ctx,
                )
                .expect("merge paragraphs");

            let merged_xml = ctx
                .document
                .raw_document
                .as_ref()
                .expect("raw xml")
                .to_string()
                .expect("serialize raw xml");
            assert_eq!(ctx.document.paragraph_count(), 1);
            assert!(
                merged_xml.contains(r#"<w:bookmarkStart w:id="3" w:name="target"/>"#),
                "{merged_xml}"
            );
            assert!(
                merged_xml.contains(r#"<w:bookmarkEnd w:id="3"/>"#),
                "{merged_xml}"
            );
            let hello = merged_xml.find("Hello").expect("first text");
            let start = merged_xml.find("<w:bookmarkStart").expect("bookmark start");
            let world = merged_xml.find("World").expect("second text");
            let end = merged_xml.find("<w:bookmarkEnd").expect("bookmark end");
            assert!(
                hello < start && start < world && world < end,
                "{merged_xml}"
            );
            let parsed = jade_docx_reader::DocumentParser::new()
                .parse_document_xml(&merged_xml)
                .expect("merged XML must parse");
            let body = parsed.body.expect("body");
            assert_eq!(body.elements.len(), 1);
            let text = body.elements[0]
                .as_paragraph()
                .expect("merged paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>();
            assert_eq!(text, "HelloWorld");

            executor.undo(&mut ctx).expect("undo merge");
            let restored_xml = word_doc
                .raw_document
                .as_ref()
                .expect("raw XML after undo")
                .to_string()
                .expect("serialize restored XML");
            assert_eq!(word_doc.paragraph_count(), 2);
            assert!(restored_xml.contains(r#"<w:bookmarkStart w:id="3" w:name="target"/>"#));
        }
    }

    #[test]
    fn adjacent_paragraph_merge_does_not_duplicate_tail_hyperlink_text() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:hyperlink w:anchor="target"><w:r><w:t>World</w:t></w:r></w:hyperlink></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(1, 0)));
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(
                    SelectionRange::collapsed(TextPosition::new(1, 0)),
                    DeleteDirection::Backward,
                )),
                &mut ctx,
            )
            .expect("merge paragraphs");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
        assert!(merged_xml.contains("w:anchor=\"target\""), "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_preserves_bookmark_after_selected_prefix() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:r><w:t>X</w:t></w:r><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert!(
            merged_xml.contains(r#"<w:bookmarkStart w:id="3" w:name="target"/>"#),
            "{merged_xml}"
        );
        assert!(
            merged_xml.contains(r#"<w:bookmarkEnd w:id="3"/>"#),
            "{merged_xml}"
        );
        assert_eq!(merged_xml.matches("<w:bookmarkStart").count(), 1);
        assert_eq!(merged_xml.matches("<w:bookmarkEnd").count(), 1);
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("World") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
        let parsed = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(&merged_xml)
            .expect("merged XML must parse");
        let text = parsed.body.expect("body").elements[0]
            .as_paragraph()
            .expect("merged paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text, "HelloWorld");
    }

    #[test]
    fn cross_paragraph_selection_keeps_bookmark_pair_spanning_cut() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert!(
            merged_xml.contains(r#"<w:bookmarkStart w:id="3" w:name="target"/>"#),
            "{merged_xml}"
        );
        assert!(
            merged_xml.contains(r#"<w:bookmarkEnd w:id="3"/>"#),
            "{merged_xml}"
        );
        assert_eq!(merged_xml.matches("<w:bookmarkStart").count(), 1);
        assert_eq!(merged_xml.matches("<w:bookmarkEnd").count(), 1);
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("World") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_comment_pair_spanning_cut() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:commentRangeStart w:id="7"/><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r><w:commentRangeEnd w:id="7"/><w:r><w:commentReference w:id="7"/></w:r></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert!(
            merged_xml.contains("<w:commentRangeStart w:id=\"7\"/>"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.contains("<w:commentRangeEnd w:id=\"7\"/>"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.contains("<w:commentReference w:id=\"7\"/>"),
            "{merged_xml}"
        );
        assert_eq!(merged_xml.matches("<w:commentRangeStart").count(), 1);
        assert_eq!(merged_xml.matches("<w:commentRangeEnd").count(), 1);
        assert!(
            merged_xml.find("<w:commentRangeStart") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("World") < merged_xml.find("<w:commentRangeEnd"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:commentRangeEnd") < merged_xml.find("<w:commentReference"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_bookmark_started_in_removed_middle_paragraph() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>Removed</w:t></w:r></w:p>
<w:p><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(2, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across three paragraphs");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            merged_xml.matches("<w:bookmarkStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:bookmarkEnd").count(),
            1,
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("World") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains("Removed"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_bookmark_started_in_removed_first_suffix() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t>X</w:t></w:r><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>Y</w:t></w:r></w:p>
<w:p><w:r><w:t>Z</w:t></w:r><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            merged_xml.matches("<w:bookmarkStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:bookmarkEnd").count(),
            1,
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("World") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert!(!merged_xml.contains(">Y<"), "{merged_xml}");
        assert!(!merged_xml.contains(">Z<"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_bookmark_ending_in_removed_tail_prefix() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:bookmarkEnd w:id="3"/><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            merged_xml.matches("<w:bookmarkStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:bookmarkEnd").count(),
            1,
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("Hello"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("Hello") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkEnd") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_bookmark_ending_in_removed_middle_paragraph() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:bookmarkStart w:id="3" w:name="target"/><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:bookmarkEnd w:id="3"/><w:r><w:t>Removed</w:t></w:r></w:p>
<w:p><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(2, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across three paragraphs");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            merged_xml.matches("<w:bookmarkStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:bookmarkEnd").count(),
            1,
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkStart") < merged_xml.find("Hello"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("Hello") < merged_xml.find("<w:bookmarkEnd"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:bookmarkEnd") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains("Removed"), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_keeps_comment_reference_after_moving_end() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:commentRangeStart w:id="7"/><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:commentRangeEnd w:id="7"/><w:r><w:commentReference w:id="7"/></w:r><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            merged_xml.matches("<w:commentRangeStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:commentRangeEnd").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:commentReference").count(),
            1,
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("Hello") < merged_xml.find("<w:commentRangeEnd"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:commentRangeEnd") < merged_xml.find("<w:commentReference"),
            "{merged_xml}"
        );
        assert!(
            merged_xml.find("<w:commentReference") < merged_xml.find("World"),
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_drops_deleted_text_from_moved_comment_reference_run() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:commentRangeStart w:id="7"/><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:commentRangeEnd w:id="7"/><w:r><w:commentReference w:id="7"/><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(
            merged_xml.matches("<w:commentRangeStart").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:commentRangeEnd").count(),
            1,
            "{merged_xml}"
        );
        assert_eq!(
            merged_xml.matches("<w:commentReference").count(),
            1,
            "{merged_xml}"
        );
        assert!(!merged_xml.contains(">X<"), "{merged_xml}");
        assert!(merged_xml.contains("Hello"), "{merged_xml}");
        assert!(merged_xml.contains("World"), "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_comment_ranges_survive_package_export_and_reload() {
        let bytes = std::fs::read(fixture_path("comment.docx")).expect("read comment fixture");
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).expect("open fixture");
        let mut files = std::collections::HashMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("read package entry");
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).expect("read part");
            files.insert(entry.name().to_string(), contents);
        }
        let mut xml = String::from_utf8(
            files
                .get("word/document.xml")
                .expect("document part")
                .clone(),
        )
        .expect("UTF-8 document");
        let start = xml.find("<w:p ").expect("source paragraph");
        let end = start + xml[start..].find("</w:p>").expect("paragraph end") + "</w:p>".len();
        xml.replace_range(
            start..end,
            r#"<w:p><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r><w:r><w:t>X</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>"#,
        );
        let input = jade_docx_writer::write_docx(
            &files,
            &std::collections::HashMap::from([("word/document.xml".to_string(), xml.into_bytes())]),
        )
        .expect("build source package");
        let mut word_doc =
            executor::block_on(WordDocument::load(&input, &Options::default())).expect("load");
        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 1));
        run_delete(
            &mut word_doc,
            &mut CommandExecutor::new(),
            range,
            DeleteDirection::Forward,
        );

        let exported = word_doc.export_docx().expect("export edited package");
        let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reload edited package");
        assert_eq!(reloaded.paragraph_count(), 1);
        assert_eq!(
            paragraph_text(reloaded.paragraph(0).expect("merged paragraph")),
            "HelloWorld"
        );
        let exported_xml = reloaded
            .raw_document
            .as_ref()
            .expect("raw XML")
            .to_string()
            .expect("serialize XML");
        for id in ["0", "1"] {
            assert!(
                exported_xml.contains(&format!("<w:commentRangeStart w:id=\"{id}\"/>")),
                "{exported_xml}"
            );
            assert!(
                exported_xml.contains(&format!("<w:commentRangeEnd w:id=\"{id}\"/>")),
                "{exported_xml}"
            );
            assert!(
                exported_xml.contains(&format!("<w:commentReference w:id=\"{id}\"/>")),
                "{exported_xml}"
            );
        }
        assert!(!exported_xml.contains(">X<"), "{exported_xml}");
        let mut package = ZipArchive::new(std::io::Cursor::new(exported)).expect("exported ZIP");
        let mut comments = String::new();
        package
            .by_name("word/comments.xml")
            .expect("comments part")
            .read_to_string(&mut comments)
            .expect("read comments");
        for id in ["0", "1"] {
            assert!(comments.contains(&format!("w:id=\"{id}\"")), "{comments}");
        }
    }

    #[test]
    fn cross_paragraph_selection_preserves_unicode_hyperlink_suffix() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:r><w:t>あ</w:t></w:r><w:hyperlink w:anchor="target"><w:r><w:t>World</w:t></w:r></w:hyperlink></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 3));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert!(!merged_xml.contains("あ"), "{merged_xml}");
        assert!(merged_xml.contains("w:anchor=\"target\""), "{merged_xml}");
        assert_eq!(merged_xml.matches("World").count(), 1, "{merged_xml}");
    }

    #[test]
    fn cross_paragraph_selection_splits_tail_hyperlink_run() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:hyperlink w:anchor="target"><w:r><w:t>World</w:t></w:r></w:hyperlink></w:p>
</w:body></w:document>"#;
        let mut document = jade_docx_reader::DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        let raw = crate::docx_xml::DocxXmlDocument::parse(xml).expect("parse raw xml");
        crate::docx_xml::attach_document_xml_ids(&mut document, &raw).expect("attach xml ids");
        let mut word_doc = WordDocument::from_document(document);
        word_doc.raw_document = Some(raw);

        let range = SelectionRange::new(TextPosition::new(0, 5), TextPosition::new(1, 2));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                &mut ctx,
            )
            .expect("delete selection across paragraph boundary");

        let merged_xml = word_doc
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize raw xml");
        assert_eq!(word_doc.paragraph_count(), 1);
        assert!(merged_xml.contains("w:anchor=\"target\""), "{merged_xml}");
        assert!(merged_xml.contains("<w:t>rld</w:t>"), "{merged_xml}");
        assert_eq!(merged_xml.matches("rld").count(), 1, "{merged_xml}");
        assert!(!merged_xml.contains("World"), "{merged_xml}");
    }

    #[test]
    fn undo_restores_multi_paragraph_delete_without_snapshot() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection =
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2)),
                        DeleteDirection::Forward,
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
        assert_eq!(word_doc.paragraph(0).unwrap().runs[0].text, "Hello");
        assert_eq!(word_doc.paragraph(1).unwrap().runs[0].text, "World");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn undo_multi_paragraph_delete_preserves_raw_paragraph_and_run_properties() {
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
                    Box::new(DeleteTextCommand::new(
                        original_selection,
                        DeleteDirection::Forward,
                    )),
                    &mut ctx,
                )
                .expect("delete across paragraphs");
            executor.undo(&mut ctx).expect("undo deletion");
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
    fn multi_paragraph_delete_moves_tail_comment_anchors_to_merged_paragraph() {
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
                    Box::new(DeleteTextCommand::new(range, DeleteDirection::Forward)),
                    &mut ctx,
                )
                .expect("delete paragraphs");
        }

        let merged = word_doc.paragraph(0).expect("merged paragraph");
        assert_eq!(
            merged
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Herld"
        );
        assert_eq!(merged.comment_ranges.len(), 1);
        assert_eq!(
            (merged.comment_ranges[0].start, merged.comment_ranges[0].end),
            (3, 5)
        );
        assert_eq!(merged.comment_markers[0].position, 5);
    }

    #[test]
    fn undo_restores_backspace_merge_without_snapshot() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(1, 0));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(1, 0)),
                        DeleteDirection::Backward,
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
        assert_eq!(word_doc.paragraph(0).unwrap().runs[0].text, "Hello");
        assert_eq!(word_doc.paragraph(1).unwrap().runs[0].text, "World");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn undo_restores_forward_merge_without_snapshot() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 5));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Forward,
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
        assert_eq!(word_doc.paragraph(0).unwrap().runs[0].text, "Hello");
        assert_eq!(word_doc.paragraph(1).unwrap().runs[0].text, "World");
        assert_eq!(selection, Some(original_selection));
    }

    #[test]
    fn redo_reapplies_multi_paragraph_delete_after_undo() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection =
            SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 2)),
                        DeleteDirection::Forward,
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
        assert_eq!(text, "Herld");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 2);
    }

    #[test]
    fn redo_reapplies_backspace_merge_after_undo() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(1, 0));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(1, 0)),
                        DeleteDirection::Backward,
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
        assert_eq!(paragraph.runs.len(), 2);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, "World");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 5);
    }

    #[test]
    fn redo_reapplies_forward_merge_after_undo() {
        let mut word_doc = sample_two_paragraph_document();
        let original_selection = SelectionRange::collapsed(TextPosition::new(0, 5));
        let mut selection = Some(original_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 5)),
                        DeleteDirection::Forward,
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
        assert_eq!(paragraph.runs.len(), 2);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, "World");
        let focus = selection.unwrap().focus;
        assert_eq!(focus.paragraph, 0);
        assert_eq!(focus.offset, 5);
    }

    /// `AB`, a complex `MERGEFIELD` showing `«Name»`, then `CD`.
    const COMPLEX_FIELD_BODY: &str = r#"<w:p><w:r><w:t>AB</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>«Name»</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>CD</w:t></w:r></w:p>"#;

    fn delete_range(
        word_doc: &mut WordDocument,
        range: SelectionRange,
        direction: DeleteDirection,
    ) {
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(word_doc, &mut selection);
        executor
            .execute(Box::new(DeleteTextCommand::new(range, direction)), &mut ctx)
            .expect("delete");
    }

    /// How many times each part of the field's code is in `xml`, in order:
    /// begin, instruction, separate, end.
    fn complex_field_code(xml: &str) -> [usize; 4] {
        [
            r#"w:fldCharType="begin""#,
            " MERGEFIELD Name ",
            r#"w:fldCharType="separate""#,
            r#"w:fldCharType="end""#,
        ]
        .map(|part| xml.matches(part).count())
    }

    #[test]
    fn deleting_next_to_or_into_a_complex_field_keeps_its_code() {
        let field_end = 2 + "«Name»".len();
        let caret = |offset| SelectionRange::collapsed(TextPosition::new(0, offset));
        let range = |start, end| {
            SelectionRange::new(TextPosition::new(0, start), TextPosition::new(0, end))
        };
        for (label, selection, direction, text) in [
            (
                "backspace before",
                caret(2),
                DeleteDirection::Backward,
                "A«Name»CD",
            ),
            (
                "delete after",
                caret(field_end),
                DeleteDirection::Forward,
                "AB«Name»D",
            ),
            (
                "range before",
                range(0, 1),
                DeleteDirection::Backward,
                "B«Name»CD",
            ),
            // A range that cuts into the result takes the text, not the code.
            (
                "range into the result",
                range(1, 2 + "«".len()),
                DeleteDirection::Backward,
                "AName»CD",
            ),
            (
                "range out of the result",
                range(field_end - "»".len(), field_end + 1),
                DeleteDirection::Backward,
                "AB«NameD",
            ),
        ] {
            let mut word_doc = load_body(COMPLEX_FIELD_BODY);
            delete_range(&mut word_doc, selection, direction);

            assert_eq!(
                paragraph_text(word_doc.paragraph(0).unwrap()),
                text,
                "{label}"
            );
            let xml = exported_document_xml(&word_doc);
            assert_eq!(complex_field_code(&xml), [1; 4], "{label}: {xml}");
            let begin = xml.find(r#"w:fldCharType="begin""#).unwrap();
            let separate = xml.find(r#"w:fldCharType="separate""#).unwrap();
            let end = xml.find(r#"w:fldCharType="end""#).unwrap();
            assert!(begin < separate && separate < end, "{label}: {xml}");
        }
    }

    #[test]
    fn deleting_a_whole_complex_field_removes_its_code() {
        let field_end = 2 + "«Name»".len();
        for (label, start, end, text) in [
            ("the field", 2, field_end, "ABCD"),
            ("the field and its neighbours", 1, field_end + 1, "AD"),
        ] {
            let mut word_doc = load_body(COMPLEX_FIELD_BODY);
            delete_range(
                &mut word_doc,
                SelectionRange::new(TextPosition::new(0, start), TextPosition::new(0, end)),
                DeleteDirection::Backward,
            );

            assert_eq!(
                paragraph_text(word_doc.paragraph(0).unwrap()),
                text,
                "{label}"
            );
            let xml = exported_document_xml(&word_doc);
            assert_eq!(complex_field_code(&xml), [0; 4], "{label}: {xml}");
            assert!(!xml.contains("Name"), "{label}: {xml}");
        }
    }

    #[test]
    fn tracked_deletion_next_to_a_complex_field_keeps_its_code() {
        let mut word_doc = load_body(COMPLEX_FIELD_BODY);
        word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
            author: Some("Reviewer".to_string()),
            date: None,
            date_utc: None,
        }));
        delete_range(
            &mut word_doc,
            SelectionRange::collapsed(TextPosition::new(0, 1)),
            DeleteDirection::Backward,
        );

        let xml = exported_document_xml(&word_doc);
        assert!(xml.contains("<w:delText>A</w:delText>"), "{xml}");
        assert_eq!(complex_field_code(&xml), [1; 4], "{xml}");
    }

    /// A complex `MERGEFIELD` showing `«Name»`, as paragraph content.
    const MERGE_FIELD_RUNS: &str = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>«Name»</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#;

    const FIELD_BEGIN: &str = r#"w:fldCharType="begin""#;
    const FIELD_SEPARATE: &str = r#"w:fldCharType="separate""#;
    const FIELD_END: &str = r#"w:fldCharType="end""#;

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

    fn range_across(start: (usize, usize), end: (usize, usize)) -> SelectionRange {
        SelectionRange::new(
            TextPosition::new(start.0, start.1),
            TextPosition::new(end.0, end.1),
        )
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

    fn paragraph_texts(word_doc: &WordDocument) -> Vec<String> {
        (0..word_doc.paragraph_count())
            .map(|index| paragraph_text(word_doc.paragraph(index).unwrap()))
            .collect()
    }

    /// The `w:fldCharType` of every `w:fldChar` in `xml`, in order.
    fn field_char_types(xml: &str) -> Vec<&str> {
        xml.split(r#"w:fldCharType=""#)
            .skip(1)
            .map(|rest| &rest[..rest.find('"').expect("closing quote")])
            .collect()
    }

    #[test]
    fn deleting_across_paragraphs_keeps_the_fields_it_does_not_touch() {
        let name = "«Name»".len();
        let other_field = MERGE_FIELD_RUNS.replace("Name", "Other");
        let one_field = ["begin", "separate", "end"].as_slice();
        let two_fields = ["begin", "separate", "end", "begin", "separate", "end"].as_slice();
        for (label, body, range, text, chars, order) in [
            (
                "a field before the selection",
                [
                    paragraph_of(&[MERGE_FIELD_RUNS, "AB"]),
                    paragraph_of(&["CD"]),
                ]
                .concat(),
                range_across((0, name + 1), (1, 1)),
                "«Name»AD",
                one_field,
                vec![" MERGEFIELD Name ", "«Name»", FIELD_END, "A", "D"],
            ),
            (
                "a field after the selection",
                [
                    paragraph_of(&["AB"]),
                    paragraph_of(&["CD", MERGE_FIELD_RUNS, "EF"]),
                ]
                .concat(),
                range_across((0, 1), (1, 1)),
                "AD«Name»EF",
                one_field,
                vec!["A", "D", FIELD_BEGIN, " MERGEFIELD Name ", "«Name»", "EF"],
            ),
            (
                "a field before and one after the paragraphs removed",
                [
                    paragraph_of(&["A", MERGE_FIELD_RUNS]),
                    paragraph_of(&["Middle"]),
                    paragraph_of(&[MERGE_FIELD_RUNS, "B"]),
                ]
                .concat(),
                range_across((0, 1 + name), (2, 0)),
                "A«Name»«Name»B",
                two_fields,
                vec!["A", "B"],
            ),
            (
                // The last paragraph's XML is cut where its runs are, past
                // the result of the field the selection takes.
                "a field after the selection, past one it takes",
                [
                    paragraph_of(&["AB"]),
                    paragraph_of(&["C", MERGE_FIELD_RUNS, "D", &other_field, "EF"]),
                ]
                .concat(),
                range_across((0, 1), (1, 1 + name + 1)),
                "A«Other»EF",
                one_field,
                vec![
                    "A",
                    FIELD_BEGIN,
                    " MERGEFIELD Other ",
                    "«Other»",
                    FIELD_END,
                    "EF",
                ],
            ),
        ] {
            let mut word_doc = load_body(&body);
            delete_range(&mut word_doc, range, DeleteDirection::Backward);

            assert_eq!(paragraph_texts(&word_doc), [text], "{label}");
            let xml = exported_document_xml(&word_doc);
            assert_eq!(field_char_types(&xml), chars, "{label}: {xml}");
            assert_eq!(
                xml.matches(" MERGEFIELD ").count(),
                chars.len() / 3,
                "{label}: {xml}"
            );
            assert_in_order(&xml, &order, label);
        }
    }

    #[test]
    fn deleting_across_paragraphs_takes_a_field_it_holds_whole() {
        let name = "«Name»".len();
        for (label, body, range, text) in [
            (
                "in a paragraph it removes",
                [
                    paragraph_of(&["AB"]),
                    paragraph_of(&[MERGE_FIELD_RUNS]),
                    paragraph_of(&["CD"]),
                ]
                .concat(),
                range_across((0, 1), (2, 1)),
                "AD",
            ),
            (
                "after the start",
                [paragraph_of(&["A", MERGE_FIELD_RUNS, "B"]), paragraph_of(&["CD"])].concat(),
                range_across((0, 1), (1, 1)),
                "AD",
            ),
            (
                "before the end",
                [paragraph_of(&["AB"]), paragraph_of(&["C", MERGE_FIELD_RUNS, "D"])].concat(),
                range_across((0, 1), (1, 1 + name)),
                "AD",
            ),
            (
                "from its begin in the first paragraph to its end in the last",
                [
                    paragraph_of(&[
                        "A",
                        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> MERGEFIELD Name </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
                        "«Na",
                    ]),
                    paragraph_of(&["me»", r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#, "B"]),
                ]
                .concat(),
                range_across((0, 1), (1, "me»".len())),
                "AB",
            ),
        ] {
            let mut word_doc = load_body(&body);
            delete_range(&mut word_doc, range, DeleteDirection::Backward);

            assert_eq!(paragraph_texts(&word_doc), [text], "{label}");
            let xml = exported_document_xml(&word_doc);
            assert_eq!(complex_field_code(&xml), [0; 4], "{label}: {xml}");
            assert!(!xml.contains("Na"), "{label}: {xml}");
        }
    }

    #[test]
    fn deleting_across_paragraphs_never_leaves_half_a_field() {
        // The code the selection took stays at the cut, so the field is whole
        // with the result left of it.
        let mut word_doc = load_body(
            &[
                paragraph_of(&["AB", MERGE_FIELD_RUNS, "CD"]),
                paragraph_of(&["EF"]),
            ]
            .concat(),
        );
        let range = range_across((0, 2 + "«N".len()), (1, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                    &mut ctx,
                )
                .expect("delete");
        }
        assert_eq!(paragraph_texts(&word_doc), ["AB«NF"]);
        let xml = exported_document_xml(&word_doc);
        assert_in_order(
            &xml,
            &[
                FIELD_BEGIN,
                " MERGEFIELD Name ",
                FIELD_SEPARATE,
                "«N",
                FIELD_END,
                "F",
            ],
            "end taken",
        );

        // Undo puts the field back where it was.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(paragraph_texts(&word_doc), ["AB«Name»CD", "EF"]);
        let xml = exported_document_xml(&word_doc);
        assert_in_order(
            &xml,
            &[
                FIELD_BEGIN,
                " MERGEFIELD Name ",
                FIELD_SEPARATE,
                "«Name»",
                FIELD_END,
                "CD",
                "EF",
            ],
            "undo",
        );

        let mut word_doc = load_body(
            &[
                paragraph_of(&["AB"]),
                paragraph_of(&["CD", MERGE_FIELD_RUNS, "EF"]),
            ]
            .concat(),
        );
        delete_range(
            &mut word_doc,
            range_across((0, 1), (1, 2 + "«Na".len())),
            DeleteDirection::Backward,
        );
        assert_eq!(paragraph_texts(&word_doc), ["Ame»EF"]);
        let xml = exported_document_xml(&word_doc);
        assert_in_order(
            &xml,
            &[
                "A",
                FIELD_BEGIN,
                " MERGEFIELD Name ",
                FIELD_SEPARATE,
                "me»",
                FIELD_END,
                "EF",
            ],
            "begin taken",
        );
    }

    #[test]
    fn deleting_across_paragraphs_into_a_field_in_a_link_keeps_it_once() {
        // The cut splits the link, which takes the field's zero-width runs to
        // both halves; the code the selection took is at the cut only.
        let field = MERGE_FIELD_RUNS.replace(
            "<w:t>«Name»</w:t>",
            "<w:t>«Na</w:t></w:r><w:r><w:t>me»</w:t>",
        );
        let mut word_doc = load_body(
            &[
                paragraph_of(&["AB"]),
                paragraph_of(&[
                    &format!(
                        r#"<w:hyperlink w:anchor="target"><w:r><w:t>CD</w:t></w:r>{field}</w:hyperlink>"#
                    ),
                    "EF",
                ]),
            ]
            .concat(),
        );
        delete_range(
            &mut word_doc,
            range_across((0, 1), (1, 2 + "«Na".len())),
            DeleteDirection::Backward,
        );

        assert_eq!(paragraph_texts(&word_doc), ["Ame»EF"]);
        let xml = exported_document_xml(&word_doc);
        assert_in_order(
            &xml,
            &[
                FIELD_BEGIN,
                " MERGEFIELD Name ",
                FIELD_SEPARATE,
                "me»",
                FIELD_END,
                "EF",
            ],
            "link",
        );
    }

    #[test]
    fn deleting_into_a_field_spanning_paragraphs_keeps_it_whole() {
        const TOC_BEGIN: &str = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
        const TOC_END: &str = r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#;
        let toc_code = |xml: &str| {
            [FIELD_BEGIN, " TOC ", FIELD_SEPARATE, FIELD_END].map(|part| xml.matches(part).count())
        };
        for (label, body, range, texts, order) in [
            (
                "inside its result",
                [
                    paragraph_of(&[TOC_BEGIN, "Entry1"]),
                    paragraph_of(&["Entry2"]),
                    paragraph_of(&["Entry3", TOC_END]),
                ]
                .concat(),
                range_across((0, 3), (1, 3)),
                vec!["Entry2", "Entry3"],
                vec![" TOC ", "Ent", "ry2", "Entry3", FIELD_END],
            ),
            (
                "from before its begin",
                [
                    paragraph_of(&["Intro"]),
                    paragraph_of(&[TOC_BEGIN, "Entry1"]),
                    paragraph_of(&["Entry2", TOC_END]),
                ]
                .concat(),
                range_across((0, 2), (1, 2)),
                vec!["Intry1", "Entry2"],
                vec![
                    "In",
                    FIELD_BEGIN,
                    " TOC ",
                    FIELD_SEPARATE,
                    "try1",
                    "Entry2",
                    FIELD_END,
                ],
            ),
            (
                "past its end",
                [
                    paragraph_of(&[TOC_BEGIN, "Entry1"]),
                    paragraph_of(&["Entry2", TOC_END, "After"]),
                    paragraph_of(&["Next"]),
                ]
                .concat(),
                range_across((1, 3), (2, 1)),
                vec!["Entry1", "Entext"],
                vec![FIELD_BEGIN, " TOC ", "Entry1", "Ent", FIELD_END, "ext"],
            ),
        ] {
            let mut word_doc = load_body(&body);
            delete_range(&mut word_doc, range, DeleteDirection::Backward);

            assert_eq!(paragraph_texts(&word_doc), texts, "{label}");
            let xml = exported_document_xml(&word_doc);
            assert_eq!(toc_code(&xml), [1; 4], "{label}: {xml}");
            assert_in_order(&xml, &order, label);
        }
    }

    #[test]
    fn joining_paragraphs_keeps_the_fields_in_the_one_moved() {
        for (label, caret, direction) in [
            (
                "backspace",
                TextPosition::new(1, 0),
                DeleteDirection::Backward,
            ),
            ("delete", TextPosition::new(0, 2), DeleteDirection::Forward),
        ] {
            let mut word_doc = load_body(
                &[
                    paragraph_of(&["AB"]),
                    paragraph_of(&["CD", MERGE_FIELD_RUNS, "EF"]),
                ]
                .concat(),
            );
            let range = SelectionRange::collapsed(caret);
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(Box::new(DeleteTextCommand::new(range, direction)), &mut ctx)
                    .expect("join");
            }

            assert_eq!(paragraph_texts(&word_doc), ["ABCD«Name»EF"], "{label}");
            let xml = exported_document_xml(&word_doc);
            assert_in_order(
                &xml,
                &[
                    "AB",
                    "CD",
                    FIELD_BEGIN,
                    " MERGEFIELD Name ",
                    FIELD_SEPARATE,
                    "«Name»",
                    FIELD_END,
                    "EF",
                ],
                label,
            );

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                assert!(executor.undo(&mut ctx).expect("undo"), "{label}");
            }
            assert_eq!(paragraph_texts(&word_doc), ["AB", "CD«Name»EF"], "{label}");
            assert_eq!(
                field_char_types(&exported_document_xml(&word_doc)),
                ["begin", "separate", "end"],
                "{label}"
            );
        }
    }
}
