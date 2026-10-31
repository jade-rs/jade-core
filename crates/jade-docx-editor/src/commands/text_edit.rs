use super::edit_offset::rebase_byte_offset;
use crate::document::{
    Paragraph, RevisionKind, RubyAnnotation, Run, RunFonts, RunProperties, PAGE_BREAK_CHAR,
    PAGE_BREAK_STR,
};
use crate::docx_xml::XmlNodeId;
use std::collections::{HashMap, HashSet};

fn clamp_to_char_boundary(text: &str, idx: usize) -> usize {
    let mut i = idx.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

fn slice_by_byte_range(text: &str, start: usize, end: usize) -> String {
    let start = clamp_to_char_boundary(text, start);
    let end = clamp_to_char_boundary(text, end);
    if start >= end {
        return String::new();
    }
    text.get(start..end).unwrap_or("").to_string()
}

pub(super) fn range_contains_deleted_revision(runs: &[Run], start: usize, end: usize) -> bool {
    let mut position = 0;
    for run in runs {
        let run_end = position + run.text.len();
        if start < run_end
            && end > position
            && run.revision.as_ref().is_some_and(|mark| {
                matches!(mark.kind, RevisionKind::Deleted | RevisionKind::MovedFrom)
            })
        {
            return true;
        }
        position = run_end;
    }
    false
}

pub(super) fn unprotected_spans(runs: &[Run], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut position = 0;
    let mut spans = Vec::new();
    for run in runs {
        let run_end = position + run.text.len();
        let from = start.max(position);
        let to = end.min(run_end);
        if from < to
            && !run.revision.as_ref().is_some_and(|mark| {
                matches!(mark.kind, RevisionKind::Deleted | RevisionKind::MovedFrom)
            })
        {
            spans.push((from, to));
        }
        position = run_end;
    }
    spans
}

fn append_text(run: &mut Run, insert: &str) {
    run.text.push_str(insert);
}

/// The saved or resolved result of a field whose result is editable text.
/// A computed or hidden result (PAGE, DATE, SET...) belongs to Word, which
/// rewrites it whenever it updates fields.
fn editable_field_result(run: &Run) -> Option<&str> {
    let field = run.field.as_ref()?;
    if !field.kind.keeps_result_in_run_text() {
        return None;
    }
    field
        .resolved_value
        .as_deref()
        .or(field.cached_text.as_deref())
}

fn run_can_seed_text_style(run: &Run) -> bool {
    !run.text.is_empty() || run.properties.is_some()
}

fn template_run_for_edit(runs: &[Run], offset: usize) -> Option<Run> {
    let mut cumulative = 0usize;
    let mut previous: Option<&Run> = None;
    let mut next: Option<&Run> = None;

    for run in runs {
        let len = run.text.len();
        let run_end = cumulative + len;

        if run_can_seed_text_style(run) {
            if offset < run_end {
                return Some(run.clone());
            }
            if offset == cumulative && next.is_none() {
                next = Some(run);
            }
            previous = Some(run);
        }

        cumulative = run_end;
    }

    previous.or(next).cloned()
}

pub(crate) fn create_run_from_template(template: Option<&Run>, text: &str) -> Run {
    if let Some(source) = template {
        let mut run = source.clone();
        run.text = text.to_string();
        // New text takes its neighbour's formatting, not its phonetic guide:
        // it is not part of that ruby base.
        if let Some(props) = run.properties.as_mut() {
            props.ruby = None;
        }
        run.image = None;
        run.text_box = None;
        run.math = None;
        run.field = None;
        run.node_id = None;
        run.text_node_id = None;
        run.revision_node_id = None;
        run.comment_reference_ids.clear();
        run.footnote_reference_ids.clear();
        run.endnote_reference_ids.clear();
        run.has_footnote_ref = false;
        run.has_endnote_ref = false;
        run
    } else {
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
}

/// Removes the text in `[start, end)` from `runs`, and returns the index the
/// removed text was at among the runs left.
///
/// The zero-width runs in `zero_width` stay unless the range takes them: a
/// complex field goes, code and result, when the range holds it from its
/// begin through its end, and another zero-width run outside a field when
/// text on both sides of it goes. A field the range only cuts into keeps its
/// code, so that no field is left with a begin and no end.
fn remove_range_from_runs(
    runs: &mut Vec<Run>,
    start: usize,
    end: usize,
    zero_width: &ZeroWidthRuns,
) -> usize {
    if start >= end {
        return runs.len();
    }

    let mut starts = Vec::with_capacity(runs.len());
    let mut cumulative = 0usize;
    for run in runs.iter() {
        starts.push(cumulative);
        cumulative += run.text.len();
    }
    let removed_at = runs
        .iter()
        .zip(&starts)
        .position(|(run, &run_start)| run_start < end && run_start + run.text.len() > start)
        .unwrap_or(runs.len());

    let fields = DeletedFields::new(
        &runs.iter().zip(starts.iter().copied()).collect::<Vec<_>>(),
        start,
        end,
        zero_width,
    );

    for (run, &run_start) in runs.iter_mut().zip(&starts) {
        let len = run.text.len();
        let run_end = run_start + len;

        if end <= run_start || start >= run_end {
            continue;
        }

        let local_start = start.saturating_sub(run_start).min(len);
        let local_end = end.saturating_sub(run_start).min(len);

        let left = slice_by_byte_range(&run.text, 0, local_start);
        let right = slice_by_byte_range(&run.text, local_end, len);
        run.text = left + &right;
    }

    promote_emptied_ruby_starts(runs);
    // This prunes the *whole* run list, not only the edited range: a comment
    // anchor anywhere in the paragraph has to survive an edit somewhere else
    // in it.
    let mut index = 0usize;
    let mut kept_before = 0usize;
    runs.retain(|run| {
        let run_index = index;
        index += 1;
        let keep = if fields.takes_field_of(run_index) {
            run_carries_content_besides_field(run)
        } else {
            run_carries_content(run)
                || (zero_width.keeps(run) && fields.keeps_zero_width(run_index, starts[run_index]))
        };
        if keep && run_index < removed_at {
            kept_before += 1;
        }
        keep
    });
    kept_before
}

/// What deleting `[start, end)` does to the complex fields among runs given
/// with the offsets they start at: the fields the range holds from their
/// begin through their end, which go code and result, and the zero-width runs
/// that stay.
///
/// The runs may come from several paragraphs, at offsets that count each
/// paragraph mark between them, so that a selection joining paragraphs
/// follows the rule a deletion inside one does. A field whose begin or end is
/// not among them (a TOC running on past them) is never held whole.
pub(crate) struct DeletedFields {
    start: usize,
    end: usize,
    /// Runs of a field the range holds whole, from its begin through its end.
    removed: Vec<bool>,
    /// Zero-width runs inside a field, or holding a `w:fldChar` of one.
    inside: Vec<bool>,
}

impl DeletedFields {
    pub(crate) fn new(
        runs: &[(&Run, usize)],
        start: usize,
        end: usize,
        zero_width: &ZeroWidthRuns,
    ) -> Self {
        // A begin is in the range when text after it goes, and an end when
        // text before it goes, so an empty field right at either edge stays.
        let mut removed = vec![false; runs.len()];
        let mut inside = vec![false; runs.len()];
        let mut open_fields: Vec<(usize, usize)> = Vec::new();
        for (index, &(run, position)) in runs.iter().enumerate() {
            let Some(chars) = zero_width.field_chars(run) else {
                continue;
            };
            inside[index] = !open_fields.is_empty() || chars != FieldChars::default();
            for _ in 0..chars.closes {
                if let Some((begin, begin_position)) = open_fields.pop() {
                    if start <= begin_position
                        && begin_position < end
                        && start < position
                        && position <= end
                    {
                        removed[begin..=index].fill(true);
                    }
                }
            }
            open_fields.extend(std::iter::repeat_n((index, position), chars.opens));
        }
        Self {
            start,
            end,
            removed,
            inside,
        }
    }

    /// Whether run `index` belongs to a field the range holds whole.
    pub(crate) fn takes_field_of(&self, index: usize) -> bool {
        self.removed[index]
    }

    /// Whether zero-width run `index`, at `position`, stays: its field is not
    /// held whole, and it is part of a field or not strictly inside the range.
    pub(crate) fn keeps_zero_width(&self, index: usize, position: usize) -> bool {
        !self.removed[index]
            && (self.inside[index] || position <= self.start || position >= self.end)
    }
}

fn ruby_of(run: &Run) -> Option<&RubyAnnotation> {
    run.properties.as_ref()?.ruby.as_deref()
}

/// For each run, the index of the run that starts the ruby base it belongs
/// to. A zero-width run (a comment reference) inside a base does not end it.
fn ruby_bases(runs: &[Run]) -> Vec<Option<usize>> {
    let mut current = None;
    runs.iter()
        .enumerate()
        .map(|(index, run)| match ruby_of(run) {
            Some(ruby) => {
                if !ruby.continuation || current.is_none() {
                    current = Some(index);
                }
                current
            }
            None => {
                if !run.text.is_empty() {
                    current = None;
                }
                None
            }
        })
        .collect()
}

/// The ruby base of the character before `offset` (`before`) or at it.
fn ruby_base_at(
    runs: &[Run],
    bases: &[Option<usize>],
    offset: usize,
    before: bool,
) -> Option<usize> {
    let mut position = 0usize;
    for (run, base) in runs.iter().zip(bases) {
        let end = position + run.text.len();
        let covers = if before {
            position < offset && offset <= end
        } else {
            position <= offset && offset < end
        };
        if covers {
            return *base;
        }
        position = end;
    }
    None
}

/// Whether text inserted over a range from `start` belongs to a ruby base: the
/// character before `start` and the one at it (the first one replaced, or the
/// one after a caret) are in the same base. Text typed or pasted at either
/// edge of a base stays outside it, so typing after a word with furigana does
/// not stretch the guide over the new text.
fn insertion_joins_ruby_base(runs: &[Run], start: usize) -> bool {
    let bases = ruby_bases(runs);
    let before = ruby_base_at(runs, &bases, start, true);
    before.is_some() && before == ruby_base_at(runs, &bases, start, false)
}

/// Inserts `insert` as a run of its own when [`insert_into_runs`] would add it
/// to a ruby base it does not belong to. The new run takes that base run's
/// formatting, as typed text takes its neighbour's, but not the guide.
fn insert_beside_ruby_base(
    runs: &mut Vec<Run>,
    position: usize,
    insert: &str,
    zero_width: &ZeroWidthRuns,
) -> bool {
    let target = match insertion_point(runs, position, zero_width) {
        Some(InsertionPoint::Run { index, .. }) => index,
        // `insert_into_runs` already gives this text a run of its own.
        Some(InsertionPoint::BeforeField { .. }) => return false,
        None => match runs.len().checked_sub(1) {
            Some(last) => last,
            None => return false,
        },
    };
    if ruby_of(&runs[target]).is_none() {
        return false;
    }
    let inserted = create_run_from_template(Some(&runs[target]), insert);
    let tail = split_runs_at_preserving_source(runs, position, zero_width);
    runs.push(inserted);
    runs.extend(tail);
    true
}

/// Makes a run that continues a ruby base start it when nothing before it in
/// the paragraph is part of that base: a split moved it into a new paragraph
/// or joined it after other text. Without this its guide would not be drawn.
pub(crate) fn normalize_ruby_bases(runs: &mut [Run]) {
    let mut in_base = false;
    for run in runs.iter_mut() {
        let has_text = !run.text.is_empty();
        match run
            .properties
            .as_mut()
            .and_then(|props| props.ruby.as_mut())
        {
            Some(ruby) => {
                if !in_base {
                    ruby.continuation = false;
                }
                in_base = true;
            }
            None => {
                if has_text {
                    in_base = false;
                }
            }
        }
    }
}

/// When an edit empties the run that starts a ruby base, the next run of that
/// base starts it instead, so the guide survives over the remaining text.
fn promote_emptied_ruby_starts(runs: &mut [Run]) {
    let mut promote = false;
    for run in runs.iter_mut() {
        let emptied = !run_carries_content(run);
        match run
            .properties
            .as_mut()
            .and_then(|props| props.ruby.as_mut())
        {
            Some(ruby) if !ruby.continuation => promote = emptied,
            Some(ruby) => {
                if promote && !emptied {
                    ruby.continuation = false;
                    promote = false;
                }
            }
            None => {
                if !emptied {
                    promote = false;
                }
            }
        }
    }
}

/// Where [`insert_into_runs`] puts text inserted at a paragraph offset.
enum InsertionPoint {
    /// Into the run at `index`, which starts at offset `start`.
    Run { index: usize, start: usize },
    /// Into a run of its own at `index`, where the marker runs of a field
    /// with a computed or hidden result start.
    BeforeField { index: usize },
}

/// `None` means past every run.
fn insertion_point(
    runs: &[Run],
    position: usize,
    zero_width: &ZeroWidthRuns,
) -> Option<InsertionPoint> {
    let mut cumulative = 0usize;
    // First zero-width run at the caret. A field's fldChar/instrText marker
    // runs start here, ahead of the run that carries its result.
    let mut zero_width_start = None;
    for (index, run) in runs.iter().enumerate() {
        let len = run.text.len();
        let run_end = cumulative + len;

        if position < run_end || (position == run_end && len > 0) {
            return Some(InsertionPoint::Run {
                index,
                start: cumulative,
            });
        }

        // Deleting all text can leave an empty text run at the old text's
        // position among note references and other structural runs. Reuse it
        // so the next keystroke stays on the same side of those objects.
        if position == cumulative && len == 0 && !run_carries_content(run) && !zero_width.keeps(run)
        {
            return Some(InsertionPoint::Run {
                index,
                start: cumulative,
            });
        }

        // Complex fields contain zero-width fldChar/instrText runs before the
        // result. At a field-only paragraph offset, typing belongs in an
        // editable result rather than inside the first structural marker run.
        // A computed or hidden result (PAGE, DATE, SET...) is rewritten
        // whenever Word updates fields, so the typed text becomes its own run
        // ahead of the field instead, where Word puts the caret.
        if position == cumulative && len == 0 {
            let field_start = *zero_width_start.get_or_insert(index);
            if let Some(field) = run.field.as_ref() {
                if !field.kind.keeps_result_in_run_text() {
                    return Some(InsertionPoint::BeforeField { index: field_start });
                }
                if editable_field_result(run).is_some() {
                    return Some(InsertionPoint::Run {
                        index,
                        start: cumulative,
                    });
                }
            }
        }

        cumulative = run_end;
    }
    None
}

fn insert_into_runs(
    runs: &mut Vec<Run>,
    position: usize,
    insert: &str,
    template: Option<&Run>,
    zero_width: &ZeroWidthRuns,
) {
    if insert.is_empty() {
        return;
    }

    if let Some(point) = insertion_point(runs, position, zero_width) {
        let (index, run_start) = match point {
            InsertionPoint::Run { index, start } => (index, start),
            InsertionPoint::BeforeField { index } => {
                runs.insert(index, create_run_from_template(template, insert));
                return;
            }
        };
        let run = &mut runs[index];
        if run.text.is_empty() {
            if let Some(result) = editable_field_result(run).map(str::to_string) {
                run.text = insert.to_string() + &result;
                return;
            }
        }
        let local = clamp_to_char_boundary(&run.text, position.saturating_sub(run_start));
        run.text.insert_str(local, insert);
        return;
    }

    // Text typed into a run kept for its `w:fldChar` or `w:instrText` would
    // be written into the field's code.
    if let Some(index) = runs
        .iter()
        .rposition(|run| !zero_width.protects_field_code(run))
    {
        if run_holds_object(&runs[index]) {
            // Typed into a note reference's or a text box's run, the text
            // shared it with the object, and the next edit that split the
            // run copied the object into both halves. It takes a run of
            // its own after it instead, formatted like it.
            let run = create_run_from_template(Some(&runs[index]), insert);
            runs.insert(index + 1, run);
        } else {
            append_text(&mut runs[index], insert);
        }
    } else {
        let run = create_run_from_template(runs.last(), insert);
        runs.push(run);
    }
}

/// Whether `run` holds an object that is not text: a text box, an image,
/// an equation, a comment/note reference or a note's own number.
fn run_holds_object(run: &Run) -> bool {
    run.text_box.is_some()
        || run.image.is_some()
        || run.math.is_some()
        || !run.comment_reference_ids.is_empty()
        || !run.footnote_reference_ids.is_empty()
        || !run.endnote_reference_ids.is_empty()
        || run.has_footnote_ref
        || run.has_endnote_ref
}

/// Whether `run` places a text box or references a note.
fn places_a_story(run: &Run) -> bool {
    run.text_box.is_some()
        || !run.footnote_reference_ids.is_empty()
        || !run.endnote_reference_ids.is_empty()
}

/// Applies a text edit to the provided run sequence without discarding style
/// or breaking existing run boundaries more than necessary.
#[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(start, end, insert_len = insert.len())))]
pub fn apply_text_edit(
    runs: &mut Vec<Run>,
    start: usize,
    end: usize,
    insert: &str,
    zero_width: &ZeroWidthRuns,
) {
    let mut start = start;
    let mut end = end;
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }

    // A caret in a tracked change must create ordinary text outside that
    // revision. Editing the deleted run directly writes into <w:delText>, so
    // the user's new characters disappear when Word shows final markup; and
    // text typed into an insertion would become part of somebody else's
    // change. With tracking on, the caller marks the new run as an insertion
    // of its own.
    if start == end && !insert.is_empty() {
        let mut cumulative = 0usize;
        let mut protected_template = None;
        for run in runs.iter() {
            let len = run.text.len();
            let run_end = cumulative + len;
            if start < run_end || (start == run_end && len > 0) {
                if run.revision.is_some() {
                    protected_template = Some(run.clone());
                }
                break;
            }
            cumulative = run_end;
        }
        if let Some(template) = protected_template.as_ref() {
            let mut inserted = create_run_from_template(Some(template), insert);
            inserted.revision = None;
            let mut tail = split_runs_at_preserving_source(runs, start, zero_width);
            // At a paragraph's start, typing goes after the text boxes and
            // note references there, as it does without a revision around.
            if start == 0 {
                let leading = tail
                    .iter()
                    .take_while(|run| run.text.is_empty() && places_a_story(run))
                    .count();
                runs.extend(tail.drain(..leading));
            }
            runs.push(inserted);
            runs.extend(tail);
            normalize_break_runs(runs);
            return;
        }
    }

    let inserts_into_existing_text = start == end
        && !insert.is_empty()
        && matches!(
            insertion_point(runs, start, zero_width),
            Some(InsertionPoint::Run { index, .. }) if !runs[index].text.is_empty()
        );
    // Caret insertion into existing text retains its run's formatting. Other
    // paths may need a template saved before the selected runs are removed.
    let template = if inserts_into_existing_text {
        None
    } else {
        template_run_for_edit(runs, start)
    };
    let outside_ruby_base = !insert.is_empty()
        && runs.iter().any(|run| ruby_of(run).is_some())
        && !insertion_joins_ruby_base(runs, start);

    let removed_at = remove_range_from_runs(runs, start, end, zero_width);
    // A replacement takes the selected text's formatting. Inserting through
    // the caret path instead joins the run on the left at a run boundary,
    // silently changing (for example) italic replacement text to bold.
    let replacement_needs_own_style = start < end
        && !insert.is_empty()
        && match (template.as_ref(), insertion_point(runs, start, zero_width)) {
            (Some(selected), Some(InsertionPoint::Run { index, .. })) => {
                ruby_of(selected).is_none()
                    && (selected.properties != runs[index].properties
                        || selected.hyperlink != runs[index].hyperlink)
            }
            _ => false,
        };
    if replacement_needs_own_style {
        let mut tail = split_runs_at_preserving_source(runs, start, zero_width);
        if start == 0 {
            let leading = tail
                .iter()
                .take_while(|run| run.text.is_empty() && places_a_story(run))
                .count();
            runs.extend(tail.drain(..leading));
        }
        let mut replacement = create_run_from_template(template.as_ref(), insert);
        replacement.revision = None;
        runs.push(replacement);
        runs.extend(tail);
        normalize_break_runs(runs);
        return;
    }
    // With no run left but the zero-width ones kept for their XML, the text
    // goes where the removed text was among them, as a run of its own.
    let only_zero_width = |runs: &[Run]| runs.iter().all(|run| zero_width.keeps(run));
    if runs.iter().all(|run| zero_width.protects_field_code(run))
        || (start < end && only_zero_width(runs))
    {
        if let Some(template_run) = template.as_ref() {
            runs.insert(
                removed_at.min(runs.len()),
                create_run_from_template(Some(template_run), insert),
            );
        } else {
            insert_into_runs(runs, start, insert, None, zero_width);
        }
    } else if !(outside_ruby_base && insert_beside_ruby_base(runs, start, insert, zero_width)) {
        insert_into_runs(runs, start, insert, template.as_ref(), zero_width);
    }

    if only_zero_width(runs) {
        if let Some(template_run) = template.as_ref() {
            runs.insert(
                removed_at.min(runs.len()),
                create_run_from_template(Some(template_run), ""),
            );
        }
    }

    // Typed text is never part of somebody's tracked change, whichever run
    // took it (an insertion emptied by a deletion, say); with tracking on,
    // the caller marks it as an insertion of its own.
    if !insert.is_empty() {
        unmark_range(runs, start, start + insert.len(), zero_width);
    }

    normalize_break_runs(runs);
}

/// Takes the revision off the text in `start..end`, cutting runs there.
fn unmark_range(runs: &mut Vec<Run>, start: usize, end: usize, zero_width: &ZeroWidthRuns) {
    let mut offset = 0;
    let marked = runs.iter().any(|run| {
        let (from, to) = (offset, offset + run.text.len());
        offset = to;
        run.revision.is_some() && from < end && start < to
    });
    if !marked {
        return;
    }
    let tail = split_runs_at_preserving_source(runs, end, zero_width);
    let mut middle = split_runs_at_preserving_source(runs, start, zero_width);
    for run in middle.iter_mut().filter(|run| !run.text.is_empty()) {
        run.revision = None;
        run.revision_node_id = None;
    }
    runs.extend(middle);
    runs.extend(tail);
}

/// A paragraph's comment anchors, which hold offsets into its text and move
/// with every edit (see [`rebase_comment_anchors_for_edit`]). An undo that
/// puts the runs back has to put these back with them, or the comment is
/// drawn over other text until the document is reopened.
#[derive(Clone, Debug, Default)]
pub struct CommentAnchors {
    ranges: Vec<crate::document::CommentRange>,
    markers: Vec<crate::document::CommentMarker>,
}

impl CommentAnchors {
    pub fn of(paragraph: &Paragraph) -> Self {
        Self {
            ranges: paragraph.comment_ranges.clone(),
            markers: paragraph.comment_markers.clone(),
        }
    }

    pub fn restore(&self, paragraph: &mut Paragraph) {
        paragraph.comment_ranges = self.ranges.clone();
        paragraph.comment_markers = self.markers.clone();
    }
}

/// Rebases model-only comment anchors after replacing `[start, end)` with
/// `replacement_len` bytes. Raw OOXML markers remain in their structural
/// positions; these offsets drive editor highlighting and headless APIs.
pub fn rebase_comment_anchors_for_edit(
    paragraph: &mut Paragraph,
    start: usize,
    end: usize,
    replacement_len: usize,
) {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    for range in &mut paragraph.comment_ranges {
        range.start = rebase_byte_offset(range.start, start, end, replacement_len);
        range.end = rebase_byte_offset(range.end, start, end, replacement_len).max(range.start);
    }
    for marker in &mut paragraph.comment_markers {
        marker.position = rebase_byte_offset(marker.position, start, end, replacement_len);
    }
}

/// Moves surviving comment metadata from the tail paragraph into a paragraph
/// produced by a cross-paragraph edit.
pub fn append_rebased_tail_comment_anchors(
    target: &mut Paragraph,
    tail: &Paragraph,
    tail_cut: usize,
    junction: usize,
) {
    let map = |position: usize| junction.saturating_add(position.saturating_sub(tail_cut));

    for range in &tail.comment_ranges {
        if range.end <= tail_cut {
            continue;
        }
        let mapped_start = map(range.start.max(tail_cut));
        let mapped_end = map(range.end).max(mapped_start);
        if let Some(existing) = target
            .comment_ranges
            .iter_mut()
            .find(|existing| existing.id == range.id)
        {
            existing.end = existing.end.max(mapped_end);
        } else {
            target.comment_ranges.push(crate::document::CommentRange {
                id: range.id.clone(),
                start: mapped_start,
                end: mapped_end,
            });
        }
    }

    for marker in &tail.comment_markers {
        if marker.position < tail_cut {
            continue;
        }
        let mapped_position = map(marker.position);
        if let Some(existing) = target
            .comment_markers
            .iter_mut()
            .find(|existing| existing.id == marker.id)
        {
            existing.position = existing.position.max(mapped_position);
        } else {
            target.comment_markers.push(crate::document::CommentMarker {
                id: marker.id.clone(),
                position: mapped_position,
            });
        }
    }
}

/// Whether a run is worth keeping when it holds no text.
///
/// A run whose only child is a `commentReference`, `footnoteReference` or
/// `endnoteReference` carries no text, but it is the anchor Word draws the
/// comment or footnote at. Dropping it as "empty" leaves a
/// `commentRangeStart`/`commentRangeEnd` pair with nothing to attach to, which
/// is a comment Word cannot show — so editing anywhere in the paragraph used
/// to silently delete every comment anchor in it.
///
/// Every place that prunes "empty" runs has to agree on this, which is why it
/// lives here rather than being spelled out at each call site.
pub(crate) fn run_carries_content(run: &Run) -> bool {
    run.field.is_some() || run_carries_content_besides_field(run)
}

/// [`run_carries_content`], except that a field result emptied of its text
/// does not count: it goes when its field does.
fn run_carries_content_besides_field(run: &Run) -> bool {
    !run.text.is_empty()
        || run.image.is_some()
        || run.text_box.is_some()
        || run.math.is_some()
        || !run.comment_reference_ids.is_empty()
        || !run.footnote_reference_ids.is_empty()
        || !run.endnote_reference_ids.is_empty()
        || run.has_footnote_ref
        || run.has_endnote_ref
}

/// The zero-width runs of a paragraph that an edit inside it keeps, by the XML
/// run each is bound to; `WordDocument::zero_width_runs` reads them.
///
/// A complex field keeps its code in `w:r` elements the run model knows only
/// as empty runs: `w:fldChar` begin, separate and end, and `w:instrText`. So
/// does run content the model does not read. Pruned as empty, their elements
/// had no run left to own them and the sync discarded them, so a comment, a
/// link, formatting or a deletion anywhere in a paragraph removed every field
/// in it.
///
/// An empty run with nothing but `w:rPr` (or an empty `w:t`) is not one of
/// these and goes as before: kept, it would hand its formatting to the text
/// typed after a deletion empties the paragraph.
#[derive(Clone, Debug, Default)]
pub(crate) struct ZeroWidthRuns {
    field_chars: HashMap<XmlNodeId, FieldChars>,
    field_code: HashSet<XmlNodeId>,
}

/// The complex fields a zero-width run ends and then begins with its
/// `w:fldChar`s. A field it begins and ends counts in neither.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FieldChars {
    pub closes: usize,
    pub opens: usize,
}

impl ZeroWidthRuns {
    pub(crate) fn insert(&mut self, run_id: XmlNodeId, field_chars: FieldChars) {
        self.field_chars.insert(run_id, field_chars);
    }

    /// Adds another paragraph's zero-width runs, for runs that paragraph
    /// hands to this one.
    pub(crate) fn extend(&mut self, other: ZeroWidthRuns) {
        self.field_chars.extend(other.field_chars);
        self.field_code.extend(other.field_code);
    }

    /// The `w:fldChar`s of `run` when it is one of the zero-width runs kept.
    fn field_chars(&self, run: &Run) -> Option<FieldChars> {
        if !run.text.is_empty() {
            return None;
        }
        self.field_chars.get(&run.node_id?).copied()
    }

    /// Whether `run` is one of the zero-width runs an edit keeps.
    pub(crate) fn keeps(&self, run: &Run) -> bool {
        self.field_chars(run).is_some()
    }

    pub(crate) fn mark_field_code(&mut self, id: XmlNodeId) {
        self.field_code.insert(id);
    }

    fn protects_field_code(&self, run: &Run) -> bool {
        run.text.is_empty() && run.node_id.is_some_and(|id| self.field_code.contains(&id))
    }

    fn closes_field(&self, run: &Run) -> bool {
        self.field_chars(run).is_some_and(|chars| chars.closes > 0)
    }
}

/// Splits a run sequence at the specified byte offset, returning the tail
/// portion while leaving the head in-place. Unsplit source runs keep their
/// XML identifiers, so unrelated run attributes survive the text edit, and so
/// do the zero-width runs in `zero_width`, which keep their elements. A tail
/// that moves to another paragraph stays bound only when its XML moves too.
pub(crate) fn split_runs_at_preserving_source(
    runs: &mut Vec<Run>,
    offset: usize,
    zero_width: &ZeroWidthRuns,
) -> Vec<Run> {
    let mut cumulative = 0usize;
    let mut index = 0usize;

    while index < runs.len() {
        let len = runs[index].text.len();
        let run_start = cumulative;
        let run_end = cumulative + len;

        // The split point sits before this run, so everything from here on is
        // the tail. `<=` matters at offset 0 of the first run: without it the
        // loop walked past that run and left it on the head side, so splitting
        // a paragraph at its very start kept its first run.
        if offset <= run_start {
            break;
        }

        if offset == run_end {
            index += 1;
            // A zero-width run at the split goes with the text after it, as a
            // field's begin does, except the end of a field whose result ends
            // here: it stays with that result, so that a range ending here (a
            // comment, a link) holds the whole field or none of it.
            while runs
                .get(index)
                .is_some_and(|run| zero_width.closes_field(run))
            {
                index += 1;
            }
            break;
        }

        if offset > run_start && offset < run_end {
            let split_point = offset - run_start;
            let split_point = clamp_to_char_boundary(&runs[index].text, split_point);
            let right_text = runs[index]
                .text
                .get(split_point..)
                .unwrap_or("")
                .to_string();
            runs[index].text.truncate(split_point);

            let mut new_run = runs[index].clone();
            new_run.text = right_text;
            new_run.node_id = None;
            new_run.text_node_id = None;
            new_run.revision_node_id = None;
            // The right half continues the left half's ruby base; the guide
            // stays with the run that starts it.
            if let Some(ruby) = new_run
                .properties
                .as_mut()
                .and_then(|props| props.ruby.as_mut())
            {
                ruby.continuation = true;
            }

            runs.insert(index + 1, new_run);
            index += 1;
            break;
        }

        cumulative = run_end;
        index += 1;
    }

    let mut right = runs.split_off(index);
    let keep = |run: &Run| run_carries_content(run) || zero_width.keeps(run);
    runs.retain(keep);
    right.retain(keep);
    right
}

/// Appends the provided runs to the target sequence, resetting XML identifiers
/// so that new nodes are generated during serialization.
pub fn append_runs(target: &mut Vec<Run>, mut tail: Vec<Run>) {
    for run in &mut tail {
        run.node_id = None;
        run.text_node_id = None;
        run.revision_node_id = None;
    }
    target.extend(tail);
}

/// Applies the provided run properties to the text range `[start, end)` within
/// `runs`, splitting runs as needed so that the styling affects only the
/// desired span. Returns the restyled source runs, as
/// [`mutate_run_properties`] does.
#[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(start, end)))]
pub fn apply_run_properties(
    runs: &mut Vec<Run>,
    start: usize,
    end: usize,
    props: &RunProperties,
    zero_width: &ZeroWidthRuns,
) -> Vec<XmlNodeId> {
    mutate_run_properties(runs, start, end, zero_width, |target| {
        merge_run_properties(target, props);
    })
}

/// Splits the runs at the range and hands each covered run's properties to
/// `mutate`.
///
/// [`apply_run_properties`] merges only the fields its `RunProperties` sets, so
/// it can turn a property on but never off. Removing a character style — Word's
/// 「標準の文字」 — has to write `None`, which that merge cannot express.
///
/// Every run stays in this paragraph, so runs keep their XML ids and the
/// writer updates their source elements: a text box or image inside or after
/// the range keeps the XML the model does not carry. Returns the ids of the
/// covered source runs whose properties changed. Patching a `w:rPr` in place
/// cannot drop a property the mutation cleared, so the caller syncs with
/// `WordDocument::sync_paragraph_run_properties`, which rebuilds those runs'
/// `w:rPr` from the model.
pub fn mutate_run_properties(
    runs: &mut Vec<Run>,
    start: usize,
    end: usize,
    zero_width: &ZeroWidthRuns,
    mutate: impl Fn(&mut RunProperties),
) -> Vec<XmlNodeId> {
    if start >= end {
        return Vec::new();
    }

    let right = split_runs_at_preserving_source(runs, end, zero_width);
    let mut middle = split_runs_at_preserving_source(runs, start, zero_width);
    let mut left = std::mem::take(runs);

    if middle.is_empty() {
        if start == 0 && !left.is_empty() {
            middle = left;
            left = Vec::new();
        } else {
            // Nothing to style; restore original layout.
            left.extend(right);
            *runs = left;
            normalize_break_runs(runs);
            return Vec::new();
        }
    }

    let mut restyled = Vec::new();
    let mut selected_bytes = 0usize;
    for run in &mut middle {
        // Rendered selections after a note marker collapse onto the following
        // text's model offset. Leading references belong before that text.
        if selected_bytes == 0
            && run.text.is_empty()
            && (!run.footnote_reference_ids.is_empty() || !run.endnote_reference_ids.is_empty())
        {
            continue;
        }
        // An inline equation keeps the formatting its OMML gives it: the
        // equation is saved as that OMML, so formatting set on its run here
        // showed until the document was saved and was gone once reopened.
        if run.math.is_some() {
            continue;
        }
        selected_bytes += run.text.len();
        let before = run.properties.clone().unwrap_or_default();
        let mut props = before.clone();
        mutate(&mut props);
        if props != before {
            // Text still a tracked insertion is new as it stands: Word
            // records no formatting change on it.
            let inserted = run
                .revision
                .as_ref()
                .is_some_and(|mark| !mark.is_deletion());
            if !inserted {
                record_format_change(&before, &mut props);
            }
            restyled.extend(run.node_id);
        }
        run.properties = Some(props);
    }

    left.extend(middle);
    left.extend(right);
    *runs = left;
    normalize_break_runs(runs);
    restyled
}

/// Who is changing formatting while changes are tracked, and the next
/// `w:id` to give a change: set by the command executor around each command
/// while a revision session is open.
#[derive(Clone, Debug)]
pub(crate) struct FormatRevisionStamp {
    pub author: Option<String>,
    pub date: Option<String>,
    pub next_id: u32,
}

thread_local! {
    static FORMAT_REVISION: std::cell::RefCell<Option<FormatRevisionStamp>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `apply` with formatting edits recorded as tracked changes under
/// `stamp`, and returns its result and the next id left unused.
pub(crate) fn with_format_revision<R>(
    stamp: Option<FormatRevisionStamp>,
    apply: impl FnOnce() -> R,
) -> (R, Option<u32>) {
    let previous = FORMAT_REVISION.with(|slot| slot.replace(stamp));
    let result = apply();
    let left = FORMAT_REVISION.with(|slot| slot.replace(previous));
    (result, left.map(|stamp| stamp.next_id))
}

/// A formatting edit with changes tracked: the run keeps what its
/// formatting was before the first change (`w:rPrChange`), and an edit that
/// puts that formatting back leaves no change.
fn record_format_change(before: &RunProperties, after: &mut RunProperties) {
    FORMAT_REVISION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(stamp) = slot.as_mut() else {
            return;
        };
        match before.change.clone() {
            Some(change) => {
                let mut bare = after.clone();
                bare.change = None;
                after.change = (bare != change.previous).then_some(change);
            }
            None => {
                let mut previous = before.clone();
                previous.change = None;
                after.change = Some(Box::new(crate::document::RunPropertiesChange {
                    id: Some(stamp.next_id.to_string()),
                    author: stamp.author.clone(),
                    date: stamp.date.clone(),
                    previous,
                }));
                stamp.next_id = stamp.next_id.saturating_add(1);
            }
        }
    });
}

pub(crate) fn merge_run_properties(target: &mut RunProperties, source: &RunProperties) {
    target.mark_font_properties_direct_from(source);
    if let Some(value) = source.bold {
        target.bold = Some(value);
    }
    if let Some(value) = source.bold_complex {
        target.bold_complex = Some(value);
    }
    if let Some(value) = source.italic {
        target.italic = Some(value);
    }
    if let Some(value) = source.italic_complex {
        target.italic_complex = Some(value);
    }
    if let Some(value) = source.underline {
        target.underline = Some(value);
    }
    if let Some(value) = source.underline_kind.clone() {
        target.underline_kind = Some(value);
    }
    if let Some(value) = source.strike {
        target.strike = Some(value);
    }
    if let Some(value) = source.baseline.clone() {
        target.baseline = Some(value);
    }
    if let Some(value) = source.char_spacing {
        target.char_spacing = Some(value);
    }
    if let Some(value) = source.kerning {
        target.kerning = Some(value);
    }
    if let Some(value) = source.text_scale {
        target.text_scale = Some(value);
    }
    if let Some(value) = source.font_size {
        target.font_size = Some(value);
        // Match the direct formatting written to both w:sz and w:szCs.
        target.font_size_complex = Some(value);
    }
    if let Some(value) = source.font_size_complex {
        target.font_size_complex = Some(value);
    }
    if let Some(value) = source.font_family.clone() {
        target.font_family = Some(value);
    }
    if let Some(fonts) = source.fonts.as_ref() {
        let target_fonts = target.fonts.get_or_insert_with(RunFonts::default);
        target_fonts.merge_from(fonts);
    }
    if let Some(value) = source.color.clone() {
        target.color = Some(value);
    }
    if let Some(value) = source.highlight.clone() {
        target.highlight = Some(value);
    }
}

fn split_run_into_break_segments(run: Run) -> Vec<Run> {
    // A source run keeps its raw XML identity, including whether Word encoded
    // an existing newline as w:cr or w:br. Newly authored runs are split below.
    if (!run.text.contains('\n')
        && !run.text.contains(PAGE_BREAK_STR)
        && !run.text.contains(crate::document::COLUMN_BREAK_CHAR))
        || run.node_id.is_some()
        || run.image.is_some()
        || run.text_box.is_some()
        || run.math.is_some()
        || run.field.is_some()
    {
        return vec![run];
    }

    let mut segments = Vec::new();
    let mut template = run.clone();
    template.node_id = None;
    template.text_node_id = None;

    let mut buffer = String::new();

    for ch in run.text.chars() {
        if ch == '\n' || ch == PAGE_BREAK_CHAR || ch == crate::document::COLUMN_BREAK_CHAR {
            if !buffer.is_empty() {
                let mut text_run = template.clone();
                text_run.text = std::mem::take(&mut buffer);
                segments.push(text_run);
            }
            let mut break_run = template.clone();
            break_run.text = ch.to_string();
            segments.push(break_run);
        } else {
            buffer.push(ch);
        }
    }

    if !buffer.is_empty() {
        let mut trailing = template;
        trailing.text = buffer;
        segments.push(trailing);
    }

    if segments.is_empty() {
        vec![run]
    } else {
        segments
    }
}

/// ランに含まれるソフト改行 (`\n`) や手動改ページ (`PAGE_BREAK_CHAR`) を専用ランへ分割する。
#[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
pub fn normalize_break_runs(runs: &mut Vec<Run>) {
    if runs.iter().all(|run| {
        !run.text.contains('\n')
            && !run.text.contains(PAGE_BREAK_STR)
            && !run.text.contains(crate::document::COLUMN_BREAK_CHAR)
    }) {
        return;
    }

    let mut normalized = Vec::with_capacity(runs.len());
    for run in runs.drain(..) {
        normalized.extend(split_run_into_break_segments(run));
    }
    *runs = normalized;
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn text_run(text: &str) -> Run {
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

    fn ruby_run(text: &str, continuation: bool) -> Run {
        Run {
            properties: Some(RunProperties {
                ruby: Some(Box::new(RubyAnnotation {
                    text: "kana".to_string(),
                    size_pt: None,
                    align: None,
                    continuation,
                    ..RubyAnnotation::default()
                })),
                ..RunProperties::default()
            }),
            ..text_run(text)
        }
    }

    /// Each run's text and, for a ruby base run, whether it continues the base.
    fn ruby_layout(runs: &[Run]) -> Vec<(&str, Option<bool>)> {
        runs.iter()
            .map(|run| {
                (
                    run.text.as_str(),
                    ruby_of(run).map(|ruby| ruby.continuation),
                )
            })
            .collect()
    }

    #[test]
    fn inserted_text_joins_a_ruby_base_only_strictly_inside_it() {
        let paragraph = || {
            vec![
                text_run("a"),
                ruby_run("X", false),
                ruby_run("Y", true),
                text_run("b"),
            ]
        };

        // Between two base runs: base text.
        let mut runs = paragraph();
        apply_text_edit(&mut runs, 2, 2, "Z", &ZeroWidthRuns::default());
        assert_eq!(
            ruby_layout(&runs),
            [
                ("a", None),
                ("XZ", Some(false)),
                ("Y", Some(true)),
                ("b", None)
            ]
        );

        // After the base: a run of its own without the guide.
        let mut runs = paragraph();
        apply_text_edit(&mut runs, 3, 3, "Z", &ZeroWidthRuns::default());
        assert_eq!(
            ruby_layout(&runs),
            [
                ("a", None),
                ("X", Some(false)),
                ("Y", Some(true)),
                ("Z", None),
                ("b", None)
            ]
        );
        // Replacing that text again (an IME composition) stays outside too.
        apply_text_edit(&mut runs, 3, 4, "ZZ", &ZeroWidthRuns::default());
        assert_eq!(ruby_layout(&runs)[3], ("ZZ", None));

        // Before the base at the start of the paragraph.
        let mut runs = vec![ruby_run("X", false), ruby_run("Y", true)];
        apply_text_edit(&mut runs, 0, 0, "Z", &ZeroWidthRuns::default());
        assert_eq!(
            ruby_layout(&runs),
            [("Z", None), ("X", Some(false)), ("Y", Some(true))]
        );

        // Replacing base text that follows base text keeps it in the base.
        let mut runs = paragraph();
        apply_text_edit(&mut runs, 2, 3, "Q", &ZeroWidthRuns::default());
        assert_eq!(
            ruby_layout(&runs),
            [("a", None), ("XQ", Some(false)), ("b", None)]
        );
    }

    #[test]
    fn splitting_or_emptying_ruby_base_runs_keeps_one_guide_start() {
        // Formatting part of the starting run: the right half continues it.
        let mut runs = vec![ruby_run("XY", false), text_run("b")];
        apply_run_properties(
            &mut runs,
            1,
            2,
            &RunProperties {
                bold: Some(true),
                ..RunProperties::default()
            },
            &ZeroWidthRuns::default(),
        );
        assert_eq!(
            ruby_layout(&runs),
            [("X", Some(false)), ("Y", Some(true)), ("b", None)]
        );

        // Deleting the starting run hands the guide to the next base run.
        let mut runs = vec![text_run("a"), ruby_run("X", false), ruby_run("Y", true)];
        apply_text_edit(&mut runs, 1, 2, "", &ZeroWidthRuns::default());
        assert_eq!(ruby_layout(&runs), [("a", None), ("Y", Some(false))]);

        // Even when another ruby ends right before it.
        let mut runs = vec![
            ruby_run("W", false),
            ruby_run("X", false),
            ruby_run("Y", true),
        ];
        apply_text_edit(&mut runs, 1, 2, "", &ZeroWidthRuns::default());
        assert_eq!(ruby_layout(&runs), [("W", Some(false)), ("Y", Some(false))]);

        // A continuation that a split left first in its paragraph starts it.
        let mut runs = vec![ruby_run("Y", true), text_run("b"), ruby_run("Z", true)];
        normalize_ruby_bases(&mut runs);
        assert_eq!(
            ruby_layout(&runs),
            [("Y", Some(false)), ("b", None), ("Z", Some(false))]
        );
    }

    #[test]
    fn normalize_break_runs_splits_text_at_newline() {
        let mut runs = vec![text_run("Hello\nWorld")];
        normalize_break_runs(&mut runs);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "Hello");
        assert_eq!(runs[1].text, "\n");
        assert_eq!(runs[2].text, "World");
    }

    #[test]
    fn normalize_break_runs_keeps_multiple_breaks() {
        let mut runs = vec![text_run("\n\n")];
        normalize_break_runs(&mut runs);
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.text == "\n"));
    }

    #[test]
    fn normalize_break_runs_splits_page_break() {
        let mut runs = vec![text_run(PAGE_BREAK_STR)];
        normalize_break_runs(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, PAGE_BREAK_STR);

        let mut mixed = vec![text_run(&format!("A{PAGE_BREAK_STR}B"))];
        normalize_break_runs(&mut mixed);
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[0].text, "A");
        assert_eq!(mixed[1].text, PAGE_BREAK_STR);
        assert_eq!(mixed[2].text, "B");
    }

    #[test]
    fn apply_text_edit_keeps_formatting_carrier_when_delete_empties_paragraph() {
        let props = RunProperties {
            font_size: Some(10.5),
            bold: Some(true),
            font_family: Some("ＭＳ 明朝".into()),
            ..RunProperties::default()
        };
        let mut runs = vec![Run {
            text: "a".into(),
            properties: Some(props.clone()),
            ..text_run("")
        }];

        apply_text_edit(&mut runs, 0, 1, "", &ZeroWidthRuns::default());

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "");
        assert_eq!(runs[0].properties, Some(props));
    }

    #[test]
    fn apply_text_edit_preserves_style_for_full_run_replacement() {
        let props = RunProperties {
            font_size: Some(12.0),
            italic: Some(true),
            ..RunProperties::default()
        };
        let mut runs = vec![Run {
            text: "a".into(),
            properties: Some(props.clone()),
            ..text_run("")
        }];

        apply_text_edit(&mut runs, 0, 1, "b", &ZeroWidthRuns::default());

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "b");
        assert_eq!(runs[0].properties, Some(props));
    }

    /// A zero-width run bound to XML run `id`, kept with `field_chars`.
    fn zero_width_run(id: u32, field_chars: FieldChars, kept: &mut ZeroWidthRuns) -> Run {
        kept.insert(XmlNodeId::new(id), field_chars);
        kept.mark_field_code(XmlNodeId::new(id));
        Run {
            node_id: Some(XmlNodeId::new(id)),
            ..text_run("")
        }
    }

    fn run_ids(runs: &[Run]) -> Vec<Option<u32>> {
        runs.iter()
            .map(|run| run.node_id.map(|id| id.raw()))
            .collect()
    }

    #[test]
    fn typing_after_deleting_body_text_stays_before_the_note_reference() {
        for endnote in [false, true] {
            let id = XmlNodeId::new(42);
            let mut kept = ZeroWidthRuns::default();
            kept.insert(id, FieldChars::default());
            let mut reference = text_run("");
            reference.node_id = Some(id);
            if endnote {
                reference.endnote_reference_ids.push("9".into());
            } else {
                reference.footnote_reference_ids.push("7".into());
            }
            let mut runs = vec![text_run("body 日本語"), reference];
            let end = runs[0].text.len();
            apply_text_edit(&mut runs, 0, end, "", &kept);
            apply_text_edit(&mut runs, 0, 0, "日", &kept);
            assert_eq!(runs.len(), 2, "endnote={endnote}");
            assert_eq!(runs[0].text, "日", "endnote={endnote}");
            assert!(runs[0].footnote_reference_ids.is_empty());
            assert!(runs[0].endnote_reference_ids.is_empty());
            assert_eq!(runs[1].node_id, Some(id));
            assert!(runs[1].text.is_empty());
        }
    }

    #[test]
    fn typing_in_object_only_paragraph_reuses_source_run() {
        let id = XmlNodeId::new(42);
        let mut kept = ZeroWidthRuns::default();
        kept.insert(id, FieldChars::default());
        let mut runs = vec![Run {
            node_id: Some(id),
            ..text_run("")
        }];
        apply_text_edit(&mut runs, 0, 0, "X", &kept);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].node_id, Some(id));
        assert_eq!(runs[0].text, "X");
    }

    #[test]
    fn deleting_text_never_leaves_half_a_field() {
        let opens = FieldChars {
            closes: 0,
            opens: 1,
        };
        // A TOC's begin, instruction and separate, whose end is in a later
        // paragraph: no deletion in this one holds the whole field.
        let mut kept = ZeroWidthRuns::default();
        let mut runs = vec![
            zero_width_run(1, opens, &mut kept),
            zero_width_run(2, FieldChars::default(), &mut kept),
            zero_width_run(3, FieldChars::default(), &mut kept),
            text_run("Entry"),
        ];
        apply_text_edit(&mut runs, 0, 5, "X", &kept);
        assert_eq!(run_ids(&runs), [Some(1), Some(2), Some(3), None]);
        assert_eq!(runs[3].text, "X", "the text goes where the entry was");

        // Run content the model does not read, between two characters, goes
        // only with the text on both sides of it.
        for (start, end, ids) in [
            (0, 1, vec![Some(4), None]),
            (1, 2, vec![None, Some(4)]),
            (0, 2, vec![None]),
        ] {
            let mut kept = ZeroWidthRuns::default();
            let mut runs = vec![
                text_run("A"),
                zero_width_run(4, FieldChars::default(), &mut kept),
                text_run("B"),
            ];
            apply_text_edit(&mut runs, start, end, "", &kept);
            assert_eq!(run_ids(&runs), ids, "deleting {start}..{end}");
        }
    }

    /// A run carrying a distinct marker style per index.
    fn styled_run(text: &str, marker: usize) -> Run {
        Run {
            text: text.to_string(),
            properties: Some(RunProperties {
                font_size: Some((marker + 1) as f32),
                bold: Some(marker.is_multiple_of(2)),
                ..RunProperties::default()
            }),
            ..text_run("")
        }
    }

    /// Per-byte view of which run's properties each byte belongs to. Lets us
    /// assert that bytes *outside* an edit keep exactly their original style —
    /// i.e. a cross-run replace must not scramble neighbouring formatting.
    fn byte_styles(runs: &[Run]) -> Vec<Option<RunProperties>> {
        let mut styles = Vec::new();
        for run in runs {
            for _ in 0..run.text.len() {
                styles.push(run.properties.clone());
            }
        }
        styles
    }

    #[test]
    fn apply_text_edit_replaces_across_run_boundary_and_keeps_outer_styles() {
        // "Hello" split as runs ["He", "ll", "o"] with distinct styles.
        let mut runs = vec![styled_run("He", 0), styled_run("ll", 1), styled_run("o", 2)];
        let before = byte_styles(&runs);

        // Replace bytes 1..4 ("ell") which spans all three runs.
        apply_text_edit(&mut runs, 1, 4, "X", &ZeroWidthRuns::default());

        let flat: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(flat, "HXo");
        let after = byte_styles(&runs);
        // Byte 0 ("H") keeps run-0 style; the trailing "o" keeps run-2 style.
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[4]);
    }

    #[test]
    fn apply_text_edit_handles_multibyte_runs_across_boundary() {
        // Japanese text split across runs; replace a span crossing the boundary.
        // "あい" = 6 bytes, "うえ" = 6 bytes. Replace bytes 3..9 ("いう").
        let mut runs = vec![styled_run("あい", 0), styled_run("うえ", 1)];
        apply_text_edit(&mut runs, 3, 9, "X", &ZeroWidthRuns::default());
        let flat: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(flat, "あXえ");
    }

    #[test]
    fn apply_text_edit_caret_insertion_preserves_run_styles_and_source_nodes() {
        for (position, left, right) in [
            (0, "界Aあ", "😀Z"),
            (2, "A界あ", "😀Z"),
            (4, "Aあ界", "😀Z"),
            (6, "Aあ", "界😀Z"),
            (8, "Aあ", "😀界Z"),
            (9, "Aあ", "😀Z界"),
            (10, "Aあ", "😀Z界"),
        ] {
            let mut original = vec![styled_run("Aあ", 0), styled_run("😀Z", 1)];
            for (index, run) in original.iter_mut().enumerate() {
                run.node_id = Some(XmlNodeId::new(10 + index as u32));
                run.text_node_id = Some(XmlNodeId::new(20 + index as u32));
                run.hyperlink = Some(jade_docx_reader::document::RunHyperlink {
                    anchor: Some(format!("link-{index}")),
                    ..Default::default()
                });
            }
            let mut runs = original.clone();

            apply_text_edit(
                &mut runs,
                position,
                position,
                "界",
                &ZeroWidthRuns::default(),
            );

            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0].text, left, "inserting at byte {position}");
            assert_eq!(runs[1].text, right, "inserting at byte {position}");
            for (run, source) in runs.iter().zip(&original) {
                assert_eq!(run.properties, source.properties);
                assert_eq!(run.hyperlink, source.hyperlink);
                assert_eq!(run.node_id, source.node_id);
                assert_eq!(run.text_node_id, source.text_node_id);
            }
        }
    }

    #[test]
    fn insert_into_runs_clamps_multibyte_offsets_and_preserves_run_metadata() {
        for (position, expected) in [
            (0, "界Aあ😀Z"),
            (1, "A界あ😀Z"),
            (2, "A界あ😀Z"),
            (3, "A界あ😀Z"),
            (4, "Aあ界😀Z"),
            (5, "Aあ界😀Z"),
            (6, "Aあ界😀Z"),
            (7, "Aあ界😀Z"),
            (8, "Aあ😀界Z"),
            (9, "Aあ😀Z界"),
            (usize::MAX, "Aあ😀Z界"),
        ] {
            let mut original = styled_run("Aあ😀Z", 1);
            original.node_id = Some(XmlNodeId::new(10));
            original.text_node_id = Some(XmlNodeId::new(11));
            let mut runs = vec![original.clone()];

            insert_into_runs(&mut runs, position, "界", None, &ZeroWidthRuns::default());

            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, expected, "inserting at byte {position}");
            assert_eq!(runs[0].properties, original.properties);
            assert_eq!(runs[0].node_id, original.node_id);
            assert_eq!(runs[0].text_node_id, original.text_node_id);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, max_shrink_iters: 4000, ..ProptestConfig::default() })]

        #[test]
        fn font_size_merge_keeps_effective_and_direct_sizes_in_sync(
            initial in prop::option::of(2u16..200),
            initial_complex in prop::option::of(2u16..200),
            edited in prop::option::of(2u16..200),
            edited_complex in prop::option::of(2u16..200),
            known in any::<bool>(),
        ) {
            let size = |value: Option<u16>| value.map(|value| f32::from(value) / 2.0);
            let mut target = RunProperties {
                font_size: size(initial),
                font_size_complex: size(initial_complex),
                bold: Some(true),
                ..RunProperties::default()
            };
            if known {
                target.capture_direct_font_properties();
            }
            let original_direct = target.direct_font_properties.clone();
            let source = RunProperties {
                font_size: size(edited),
                font_size_complex: size(edited_complex),
                ..RunProperties::default()
            };
            merge_run_properties(&mut target, &source);
            let expected = size(edited.or(initial));
            let expected_complex = size(edited_complex.or(edited).or(initial_complex));
            prop_assert_eq!(target.font_size, expected);
            prop_assert_eq!(target.font_size_complex, expected_complex);
            prop_assert_eq!(target.bold, Some(true));
            if known {
                prop_assert_eq!(target.direct_font_properties.font_size, expected);
                prop_assert_eq!(target.direct_font_properties.font_size_complex, expected_complex);
            } else {
                prop_assert_eq!(target.direct_font_properties, original_direct);
            }
        }

        /// Replacing an arbitrary byte range in a paragraph split into arbitrary
        /// runs must (a) yield the same text as a flat-string model replace, and
        /// (b) leave every byte outside the edited range with its original run
        /// style. ASCII keeps every byte a char boundary so the model is exact.
        #[test]
        fn apply_text_edit_across_arbitrary_runs_matches_model_and_preserves_outer_styles(
            base in "[a-z ]{0,40}",
            raw_splits in prop::collection::vec(0usize..40, 0..6),
            raw_start in 0usize..41,
            raw_end in 0usize..41,
            insert in "[a-z ]{0,8}",
        ) {
            let len = base.len();
            let mut start = raw_start.min(len);
            let mut end = raw_end.min(len);
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }

            // Build runs by splitting `base` at sorted, unique points.
            let mut points: Vec<usize> = raw_splits.into_iter().map(|p| p.min(len)).collect();
            points.push(0);
            points.push(len);
            points.sort_unstable();
            points.dedup();
            let mut runs = Vec::new();
            for window in points.windows(2) {
                let (a, b) = (window[0], window[1]);
                if a == b {
                    continue;
                }
                let marker = runs.len();
                runs.push(styled_run(&base[a..b], marker));
            }

            let before = byte_styles(&runs);
            apply_text_edit(&mut runs, start, end, &insert, &ZeroWidthRuns::default());

            // (a) text correctness vs a plain String model
            let mut model = base.clone();
            model.replace_range(start..end, &insert);
            let flat: String = runs.iter().map(|r| r.text.as_str()).collect();
            prop_assert_eq!(&flat, &model);

            // run text length never desyncs from the flat byte length
            let after = byte_styles(&runs);
            prop_assert_eq!(after.len(), model.len());

            // (b) bytes before `start` and after the inserted text keep their
            // original per-byte style — no cross-run formatting corruption.
            prop_assert_eq!(&after[..start], &before[..start]);
            prop_assert_eq!(&after[start + insert.len()..], &before[end..]);
        }
    }
}
