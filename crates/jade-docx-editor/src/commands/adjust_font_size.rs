//! Ribbon "grow/shrink font" command.
//!
//! Word scales each run in the selection relative to its own size — a
//! selection mixing 12pt and 20pt grows to 14pt and 22pt — stepping through
//! the ribbon's size list between 8pt and 72pt, by 1pt below it, and by 10pt
//! above it. The old behaviour (flattening the whole selection to one size
//! derived from the toolbar label) is exactly what this command replaces.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::apply_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, RunProperties};
use jade_docx_layout::selection::SelectionRange;

/// The size list Word's ribbon steps through between 8pt and 72pt.
pub const WORD_FONT_SIZE_LADDER: [f32; 17] = [
    8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0,
    72.0,
];

/// Word's maximum font size.
const MAX_FONT_SIZE_PT: f32 = 1638.0;
const MIN_FONT_SIZE_PT: f32 = 1.0;
/// Sizes without explicit or style-provided values resolve to the Japanese
/// body default.
const DEFAULT_FONT_SIZE_PT: f32 = 10.5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontSizeDirection {
    Grow,
    Shrink,
}

/// How far one step moves.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum FontSizeStep {
    /// The ribbon's grow/shrink (Ctrl+Shift+> / <): through Word's size list.
    #[default]
    Ladder,
    /// Word's Ctrl+] / Ctrl+[: exactly one point.
    Point,
}

/// Word's Ctrl+]: one point larger, capped at Word's maximum size.
pub fn word_grow_font_size_by_point(size: f32) -> f32 {
    if !size.is_finite() || size <= 0.0 {
        return MIN_FONT_SIZE_PT + 1.0;
    }
    (size + 1.0).min(MAX_FONT_SIZE_PT)
}

/// Word's Ctrl+[: one point smaller, flooring at 1pt.
pub fn word_shrink_font_size_by_point(size: f32) -> f32 {
    if !size.is_finite() || size <= 0.0 {
        return MIN_FONT_SIZE_PT;
    }
    (size - 1.0).max(MIN_FONT_SIZE_PT)
}

/// One grow-font step: through the ladder in [8, 72], +1pt below 8, and to
/// the next multiple of 10 above 72 (Word continues 80, 90, … up to 1638).
pub fn word_grow_font_size(size: f32) -> f32 {
    if !size.is_finite() || size <= 0.0 {
        return WORD_FONT_SIZE_LADDER[0];
    }
    if size >= 72.0 {
        let next = ((size / 10.0).floor() + 1.0) * 10.0;
        return next.min(MAX_FONT_SIZE_PT);
    }
    if size < 8.0 {
        return size + 1.0;
    }
    WORD_FONT_SIZE_LADDER
        .iter()
        .copied()
        .find(|candidate| *candidate > size)
        .unwrap_or(72.0)
}

/// One shrink-font step, the mirror of [`word_grow_font_size`]: back to the
/// previous multiple of 10 above 72 (flooring at 72), through the ladder in
/// (8, 72], −1pt at 8 and below (flooring at 1pt).
pub fn word_shrink_font_size(size: f32) -> f32 {
    if !size.is_finite() || size <= 0.0 {
        return MIN_FONT_SIZE_PT;
    }
    if size > 72.0 {
        let previous = ((size / 10.0).ceil() - 1.0) * 10.0;
        return previous.max(72.0);
    }
    if size <= 8.0 {
        return (size - 1.0).max(MIN_FONT_SIZE_PT);
    }
    WORD_FONT_SIZE_LADDER
        .iter()
        .rev()
        .copied()
        .find(|candidate| *candidate < size)
        .unwrap_or(8.0)
}

/// Grows or shrinks every run in a selection, each run relative to its own
/// effective size.
pub struct AdjustFontSizeCommand {
    range: SelectionRange,
    direction: FontSizeDirection,
    step: FontSizeStep,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl AdjustFontSizeCommand {
    pub fn new(range: SelectionRange, direction: FontSizeDirection) -> Self {
        Self::with_step(range, direction, FontSizeStep::Ladder)
    }

    pub fn with_step(
        range: SelectionRange,
        direction: FontSizeDirection,
        step: FontSizeStep,
    ) -> Self {
        Self {
            range,
            direction,
            step,
            previous_runs: None,
            previous_selection: None,
        }
    }

    fn step(&self, size: f32) -> f32 {
        step_font_size(size, self.direction, self.step)
    }
}

/// One grow/shrink step of the given kind, shared with the caret's pending
/// formatting so a collapsed caret and a selection step identically.
pub fn step_font_size(size: f32, direction: FontSizeDirection, step: FontSizeStep) -> f32 {
    match (step, direction) {
        (FontSizeStep::Ladder, FontSizeDirection::Grow) => word_grow_font_size(size),
        (FontSizeStep::Ladder, FontSizeDirection::Shrink) => word_shrink_font_size(size),
        (FontSizeStep::Point, FontSizeDirection::Grow) => word_grow_font_size_by_point(size),
        (FontSizeStep::Point, FontSizeDirection::Shrink) => word_shrink_font_size_by_point(size),
    }
}

impl Command for AdjustFontSizeCommand {
    fn name(&self) -> &'static str {
        "adjust_font_size"
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

        for paragraph_index in start.paragraph..=end.paragraph {
            // Collect per-run segments and their target sizes before mutating:
            // `apply_run_properties` splits runs, but byte offsets over the
            // paragraph text stay stable, so the collected ranges remain valid.
            let segments: Vec<(usize, usize, f32)> = {
                let paragraph = ctx
                    .document
                    .paragraph(paragraph_index)
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
                let para_style_id = paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.style.as_deref());

                let mut segments = Vec::new();
                let mut run_start = 0usize;
                for run in &paragraph.runs {
                    let run_end = run_start + run.text.len();
                    let seg_start = run_start.max(range_start);
                    let seg_end = run_end.min(range_end);
                    if seg_start < seg_end {
                        let effective_size = run
                            .properties
                            .as_ref()
                            .and_then(|props| props.font_size)
                            .or_else(|| {
                                ctx.document.styles_part.as_ref().and_then(|styles| {
                                    styles
                                        .resolve_effective_run_properties(
                                            para_style_id,
                                            run.properties.as_ref(),
                                        )
                                        .and_then(|props| props.font_size)
                                })
                            })
                            .unwrap_or(DEFAULT_FONT_SIZE_PT);
                        segments.push((seg_start, seg_end, self.step(effective_size)));
                    }
                    run_start = run_end;
                }
                segments
            };

            if segments.is_empty() {
                continue;
            }

            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let mut restyled = Vec::new();
            for (seg_start, seg_end, new_size) in &segments {
                let props = RunProperties {
                    font_size: Some(*new_size),
                    ..Default::default()
                };
                restyled.extend(apply_run_properties(
                    &mut paragraph.runs,
                    *seg_start,
                    *seg_end,
                    &props,
                    &zero_width,
                ));
            }
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
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;

    fn run_with_size(text: &str, size: Option<f32>) -> Run {
        Run {
            text: text.to_string(),
            properties: size.map(|size| RunProperties {
                font_size: Some(size),
                ..Default::default()
            }),
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

    fn document_with_runs(runs: Vec<Run>) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs,
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    fn sizes_of(document: &WordDocument) -> Vec<(String, Option<f32>)> {
        document
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| {
                (
                    run.text.clone(),
                    run.properties.as_ref().and_then(|props| props.font_size),
                )
            })
            .collect()
    }

    #[test]
    fn grows_each_run_relative_to_its_own_size() {
        // "abc" at 12pt and "def" at 20pt: Word grows them to 14pt and 22pt.
        let mut document = document_with_runs(vec![
            run_with_size("abc", Some(12.0)),
            run_with_size("def", Some(20.0)),
        ]);
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 6),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(AdjustFontSizeCommand::new(range, FontSizeDirection::Grow)),
                    &mut ctx,
                )
                .expect("grow font");
        }
        let sizes = sizes_of(&document);
        assert_eq!(sizes[0], ("abc".to_string(), Some(14.0)));
        assert_eq!(sizes[1], ("def".to_string(), Some(22.0)));
    }

    fn apply_point_step(document: &mut WordDocument, end: usize, direction: FontSizeDirection) {
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, end),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(document, &mut selection);
        executor
            .execute(
                Box::new(AdjustFontSizeCommand::with_step(
                    range,
                    direction,
                    FontSizeStep::Point,
                )),
                &mut ctx,
            )
            .expect("step font by one point");
    }

    #[test]
    fn point_step_moves_each_run_by_exactly_one_point() {
        // Word's Ctrl+] / Ctrl+[: 12pt and 20pt runs become 13pt and 21pt —
        // one point each, ignoring the ribbon ladder (which would give 14/22).
        let mut document = document_with_runs(vec![
            run_with_size("abc", Some(12.0)),
            run_with_size("def", Some(20.0)),
            run_with_size("ghi", Some(10.5)),
        ]);
        apply_point_step(&mut document, 9, FontSizeDirection::Grow);
        let sizes = sizes_of(&document);
        assert_eq!(sizes[0], ("abc".to_string(), Some(13.0)));
        assert_eq!(sizes[1], ("def".to_string(), Some(21.0)));
        assert_eq!(sizes[2], ("ghi".to_string(), Some(11.5)));

        apply_point_step(&mut document, 9, FontSizeDirection::Shrink);
        apply_point_step(&mut document, 9, FontSizeDirection::Shrink);
        let sizes = sizes_of(&document);
        assert_eq!(sizes[0].1, Some(11.0));
        assert_eq!(sizes[1].1, Some(19.0));
        assert_eq!(sizes[2].1, Some(9.5));
    }

    #[test]
    fn point_step_clamps_to_words_size_range() {
        assert_eq!(word_grow_font_size_by_point(1638.0), 1638.0);
        assert_eq!(word_grow_font_size_by_point(1637.5), 1638.0);
        assert_eq!(word_shrink_font_size_by_point(1.0), 1.0);
        assert_eq!(word_shrink_font_size_by_point(1.5), 1.0);
        // Unset sizes resolve to the Japanese body default before stepping.
        let mut document = document_with_runs(vec![run_with_size("abc", None)]);
        apply_point_step(&mut document, 3, FontSizeDirection::Grow);
        assert_eq!(sizes_of(&document)[0].1, Some(11.5));
    }

    #[test]
    fn continues_beyond_72pt_and_below_8pt_like_word() {
        assert_eq!(word_grow_font_size(72.0), 80.0);
        assert_eq!(word_grow_font_size(80.0), 90.0);
        assert_eq!(word_grow_font_size(75.0), 80.0);
        assert_eq!(word_shrink_font_size(80.0), 72.0);
        assert_eq!(word_shrink_font_size(90.0), 80.0);
        assert_eq!(word_shrink_font_size(85.0), 80.0);
        assert_eq!(word_shrink_font_size(8.0), 7.0);
        assert_eq!(word_shrink_font_size(2.0), 1.0);
        assert_eq!(word_shrink_font_size(1.0), 1.0);
        assert_eq!(word_grow_font_size(7.0), 8.0);
        assert_eq!(word_grow_font_size(10.5), 11.0);
        assert_eq!(word_shrink_font_size(11.0), 10.5);
    }

    #[test]
    fn undo_restores_the_original_mixed_sizes() {
        let mut document = document_with_runs(vec![
            run_with_size("abc", Some(12.0)),
            run_with_size("def", None),
        ]);
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 6),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(AdjustFontSizeCommand::new(range, FontSizeDirection::Grow)),
                    &mut ctx,
                )
                .expect("grow font");
        }
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let sizes = sizes_of(&document);
        assert_eq!(sizes[0], ("abc".to_string(), Some(12.0)));
        assert_eq!(sizes[1], ("def".to_string(), None));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

        /// Growing strictly increases (until Word's 1638pt cap) and shrinking
        /// strictly decreases (until 1pt); both stay within Word's bounds.
        #[test]
        fn stepping_is_strictly_monotonic_within_bounds(size in 1.0_f32..=1638.0) {
            let grown = word_grow_font_size(size);
            prop_assert!(grown >= size);
            prop_assert!(grown <= 1638.0);
            if size < 1628.0 {
                prop_assert!(grown > size, "grow must move: {size} -> {grown}");
            }

            let shrunk = word_shrink_font_size(size);
            prop_assert!(shrunk <= size);
            prop_assert!(shrunk >= 1.0);
            if size > 1.0 {
                prop_assert!(shrunk < size, "shrink must move: {size} -> {shrunk}");
            }
        }

        /// Ladder members round-trip exactly: shrink(grow(x)) == x for every
        /// ribbon list size below the top entry.
        /// One point up then one point down is the identity everywhere the
        /// clamp does not bite, and the step never leaves Word's range.
        #[test]
        fn point_steps_round_trip_and_stay_in_range(size in 1.0_f32..=1637.0) {
            let up = word_grow_font_size_by_point(size);
            prop_assert!((up - (size + 1.0)).abs() < 1e-4);
            prop_assert!((word_shrink_font_size_by_point(up) - size).abs() < 1e-4);
            prop_assert!(word_shrink_font_size_by_point(size) >= 1.0);
            prop_assert!(word_grow_font_size_by_point(size) <= 1638.0);
        }

        #[test]
        fn ladder_members_round_trip(index in 0_usize..16) {
            let size = WORD_FONT_SIZE_LADDER[index];
            prop_assert_eq!(word_shrink_font_size(word_grow_font_size(size)), size);
        }
    }
}
