//! Ribbon "increase/decrease indent" command.
//!
//! Word steps each selected paragraph independently: a list paragraph
//! promotes/demotes its list level (`w:ilvl`, clamped to 0..=8), a paragraph
//! indented in character units moves to the next/previous whole-character
//! multiple, and a paragraph indented in points snaps to the next/previous
//! multiple of the document's default tab stop. Neither unit is converted
//! into the other, so a 720-twip Western indent never collapses into a 1字
//! indent.
//!
//! The same stepping serves Word's Ctrl+T / Ctrl+Shift+T ("hanging indent")
//! through [`IndentTarget::Hanging`]: the left edge steps on the same grid
//! but the first line stays where it is (Word's `TabHangingIndent`), so the
//! difference lands in `w:hanging` / `w:firstLine`. List levels are never
//! touched by that variant.

use super::paragraph_property_sync::{sync_paragraph_indent_xml, sync_paragraph_numbering_xml};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Indent, ParagraphProperties};
use jade_docx_layout::selection::SelectionRange;

/// One ribbon click's worth of indent, in `w:leftChars` units (100 = 1字).
const CHAR_STEP: i32 = 100;
/// OOXML's deepest list level (`w:ilvl`), Word's nine levels 0..=8.
const MAX_LIST_LEVEL: u32 = 8;
/// Fallback tab stop for point-based stepping when the document does not
/// declare `w:defaultTabStop` (Word's implicit default, 0.5" = 36pt).
const DEFAULT_TAB_STOP_PT: f32 = 36.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IndentDirection {
    Increase,
    Decrease,
}

/// Which edge a step moves.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IndentTarget {
    /// Ctrl+M: the logical start edge (list level on list items).
    Start,
    /// Ctrl+T: the start edge of every line but the first, which stays put.
    Hanging,
}

/// Steps the left indent of every paragraph in a contiguous range, each
/// paragraph relative to its own current indent.
pub struct AdjustParagraphIndentCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    direction: IndentDirection,
    target: IndentTarget,
    /// Whether any paragraph actually changed; a fully clamped step (for
    /// example decrease at list level 0) records no history.
    changed: bool,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_selection: Option<SelectionRange>,
}

impl AdjustParagraphIndentCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, direction: IndentDirection) -> Self {
        Self::with_target(
            start_paragraph,
            end_paragraph,
            direction,
            IndentTarget::Start,
        )
    }

    pub fn with_target(
        start_paragraph: usize,
        end_paragraph: usize,
        direction: IndentDirection,
        target: IndentTarget,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            direction,
            target,
            changed: false,
            previous_properties: Vec::new(),
            previous_selection: None,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }

    /// Next whole multiple of `step` strictly above `current` (increase) or
    /// the previous multiple below it (decrease), never below zero.
    fn snapped(current: i32, step: i32, direction: IndentDirection) -> i32 {
        debug_assert!(step > 0);
        match direction {
            IndentDirection::Increase => {
                let next = (current.div_euclid(step) + 1) * step;
                next.max(step)
            }
            IndentDirection::Decrease => {
                if current <= 0 {
                    0
                } else if current % step == 0 {
                    current - step
                } else {
                    current.div_euclid(step) * step
                }
            }
        }
    }

    /// Steps the paragraph's logical start edge (physical left for LTR,
    /// physical right for RTL) and returns the adjusted indent plus which
    /// physical edges were rewritten (for the XML sync flags).
    fn adjusted_indent(
        indent: Option<&Indent>,
        bidi: bool,
        default_tab_stop_pt: f32,
        direction: IndentDirection,
    ) -> Indent {
        let mut result = indent.cloned().unwrap_or_default();
        // The logical-start aliases (`w:start`/`w:startChars`) supersede the
        // physical attributes, mirroring Indent::physical_*_chars.
        let chars_current = if bidi {
            result.start_chars.or(result.right_chars)
        } else {
            result.start_chars.or(result.left_chars)
        };
        let pt_current = if bidi {
            result.start.or(result.right)
        } else {
            result.start.or(result.left)
        };

        // An explicit 0 chars re-enables the twips value (main's resolve
        // semantics), so it does not force character mode by itself.
        let effective_chars = chars_current.filter(|chars| *chars != 0);
        let use_point_mode =
            effective_chars.is_none() && pt_current.is_some_and(|value| value > 0.0);

        let (new_chars, new_pt) = if use_point_mode {
            // Point-based paragraphs keep point units and snap on the default
            // tab grid, like Word's Western increase/decrease indent.
            let step_centi = (default_tab_stop_pt.max(1.0) * 100.0).round() as i32;
            let current_centi = (pt_current.unwrap_or(0.0) * 100.0).round() as i32;
            let snapped = Self::snapped(current_centi, step_centi, direction);
            (None, Some(snapped as f32 / 100.0))
        } else {
            // Character-based (or unindented) paragraphs step by one
            // character, Word's Japanese ribbon behaviour. A zero point
            // indent counts as "no indent" (style flattening bakes
            // `w:ind w:left="0"` into most paragraphs).
            let current = chars_current.unwrap_or(0);
            (Some(Self::snapped(current, CHAR_STEP, direction)), None)
        };

        // Rewrite the logical start edge on its physical attributes and drop
        // the `w:start` aliases so the new value is unambiguous.
        result.start = None;
        result.start_chars = None;
        if bidi {
            result.right_chars = new_chars;
            result.right = new_pt;
        } else {
            result.left_chars = new_chars;
            result.left = new_pt;
        }
        result
    }

    /// Steps the paragraph's start edge while keeping the first line where it
    /// is (Word's `TabHangingIndent`): the difference between the new edge
    /// and the unchanged first-line position becomes the hanging indent, or
    /// a first-line indent once the edge drops below it. Units follow the
    /// same rule as [`Self::adjusted_indent`]; character values supersede
    /// their point twins, so in character mode any point special indent is
    /// dropped rather than converted. Returns `None` when the edge cannot
    /// move (decrease at the margin), so the paragraph is left untouched.
    fn adjusted_hanging_indent(
        indent: Option<&Indent>,
        bidi: bool,
        default_tab_stop_pt: f32,
        direction: IndentDirection,
    ) -> Option<Indent> {
        let mut result = indent.cloned().unwrap_or_default();
        let chars_left = if bidi {
            result.start_chars.or(result.right_chars)
        } else {
            result.start_chars.or(result.left_chars)
        };
        let pt_left = if bidi {
            result.start.or(result.right)
        } else {
            result.start.or(result.left)
        };
        let has_chars = chars_left.is_some_and(|value| value != 0)
            || result.hanging_chars.is_some_and(|value| value != 0)
            || result.first_line_chars.is_some_and(|value| value != 0);
        let has_points = pt_left.is_some_and(|value| value > 0.0)
            || result.hanging.is_some_and(|value| value > 0.0)
            || result.first_line.is_some_and(|value| value > 0.0);
        let use_point_mode = !has_chars && has_points;

        result.start = None;
        result.start_chars = None;

        // Work in integer hundredths (centipoints or w:*Chars units) so the
        // grid arithmetic is exact, then split the first-line offset back
        // into the mutually exclusive hanging / first-line attributes.
        let (left, offset, step) = if use_point_mode {
            let to_centi = |value: f32| (value * 100.0).round() as i32;
            let offset = match result.hanging.filter(|value| *value > 0.0) {
                Some(hanging) => -to_centi(hanging),
                None => to_centi(result.first_line.unwrap_or(0.0)),
            };
            (
                to_centi(pt_left.unwrap_or(0.0)),
                offset,
                to_centi(default_tab_stop_pt.max(1.0)),
            )
        } else {
            let offset = match result.hanging_chars.filter(|value| *value > 0) {
                Some(hanging) => -hanging,
                None => result.first_line_chars.unwrap_or(0),
            };
            (chars_left.unwrap_or(0), offset, CHAR_STEP)
        };
        let first_line_at = left + offset;
        let new_left = Self::snapped(left, step, direction);
        if new_left == left {
            return None;
        }
        let relative = first_line_at - new_left;
        let (hanging, first_line) = if relative < 0 {
            (Some(-relative), None)
        } else if relative > 0 {
            (None, Some(relative))
        } else {
            (None, None)
        };

        if use_point_mode {
            let from_centi = |value: i32| value as f32 / 100.0;
            if bidi {
                result.right = Some(from_centi(new_left));
                result.right_chars = None;
            } else {
                result.left = Some(from_centi(new_left));
                result.left_chars = None;
            }
            result.hanging = hanging.map(from_centi);
            result.first_line = first_line.map(from_centi);
            result.hanging_chars = None;
            result.first_line_chars = None;
        } else {
            if bidi {
                result.right_chars = Some(new_left);
                result.right = None;
            } else {
                result.left_chars = Some(new_left);
                result.left = None;
            }
            result.hanging_chars = hanging;
            result.first_line_chars = first_line;
            result.hanging = None;
            result.first_line = None;
        }
        Some(result)
    }
}

impl Command for AdjustParagraphIndentCommand {
    fn name(&self) -> &'static str {
        "adjust_paragraph_indent"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        let default_tab_stop_pt = ctx
            .document
            .document
            .as_ref()
            .and_then(|document| document.default_tab_stop)
            .unwrap_or(DEFAULT_TAB_STOP_PT);

        self.previous_properties.clear();
        self.previous_selection = *ctx.selection;
        self.changed = false;

        for paragraph_index in start..=end {
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties.push((paragraph_index, previous));

            // Word's ribbon promotes/demotes the LIST LEVEL of a numbered or
            // bulleted paragraph instead of moving its left indent (which the
            // numbering definition supplies).
            let list_step = {
                let paragraph = ctx
                    .document
                    .paragraph(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.numbering.as_ref())
                    .and_then(|numbering| {
                        // `w:numId="0"` is no list: it indents like body text.
                        let id = numbering.list_id()?.to_string();
                        Some((id, numbering.level.unwrap_or(0)))
                    })
            };
            if let Some((num_id, current_level)) =
                list_step.filter(|_| self.target == IndentTarget::Start)
            {
                let next_level = match self.direction {
                    IndentDirection::Increase => (current_level + 1).min(MAX_LIST_LEVEL),
                    IndentDirection::Decrease => current_level.saturating_sub(1),
                };
                if next_level != current_level {
                    let paragraph = ctx
                        .document
                        .paragraph_mut(paragraph_index)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    if let Some(numbering) = paragraph
                        .properties
                        .as_mut()
                        .and_then(|props| props.numbering.as_mut())
                    {
                        numbering.level = Some(next_level);
                    }
                    sync_paragraph_numbering_xml(
                        ctx.document,
                        paragraph_index,
                        Some(num_id.as_str()),
                        Some(next_level),
                    );
                    self.changed = true;
                }
                continue;
            }

            let (applied_indent, bidi, indent_changed) = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                let bidi = props.bidi.unwrap_or(false);
                let adjusted = match self.target {
                    IndentTarget::Start => Some(Self::adjusted_indent(
                        props.indent.as_ref(),
                        bidi,
                        default_tab_stop_pt,
                        self.direction,
                    )),
                    IndentTarget::Hanging => Self::adjusted_hanging_indent(
                        props.indent.as_ref(),
                        bidi,
                        default_tab_stop_pt,
                        self.direction,
                    ),
                };
                // A clamped hanging step moves nothing: skip the sync so the
                // paragraph keeps its exact previous properties.
                let Some(adjusted) = adjusted else {
                    continue;
                };
                let indent_changed = props.indent.as_ref() != Some(&adjusted);
                props.indent = Some(adjusted.clone());
                (adjusted, bidi, indent_changed)
            };
            self.changed |= indent_changed;
            sync_paragraph_indent_xml(
                ctx.document,
                paragraph_index,
                Some(&applied_indent),
                !bidi,
                bidi,
            );
        }

        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous) in &self.previous_properties {
            let restored_indent = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.indent.clone())
            };
            // A list item stepped its `w:ilvl`, not its indent: put the level
            // back in the file too, or the saved list keeps the undone step.
            if let Some((num_id, level)) = previous
                .as_ref()
                .and_then(|props| props.numbering.as_ref())
                .and_then(|numbering| {
                    Some((
                        numbering.list_id()?.to_string(),
                        numbering.level.unwrap_or(0),
                    ))
                })
            {
                sync_paragraph_numbering_xml(
                    ctx.document,
                    *paragraph_index,
                    Some(num_id.as_str()),
                    Some(level),
                );
            }
            sync_paragraph_indent_xml(
                ctx.document,
                *paragraph_index,
                restored_indent.as_ref(),
                true,
                true,
            );
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use proptest::prelude::*;

    fn run(text: &str) -> Run {
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

    fn paragraph_with_indent(text: &str, indent: Option<Indent>) -> Paragraph {
        Paragraph {
            runs: vec![run(text)],
            properties: indent.map(|indent| ParagraphProperties {
                indent: Some(indent),
                ..Default::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    fn document_with(paragraphs: Vec<Paragraph>) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: paragraphs
                    .into_iter()
                    .map(BodyElement::from_paragraph)
                    .collect(),
            }),
            ..Document::default()
        })
    }

    fn indent_of(document: &WordDocument, index: usize) -> Option<Indent> {
        document
            .paragraph(index)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.indent.clone())
    }

    fn adjust(document: &mut WordDocument, start: usize, end: usize, direction: IndentDirection) {
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(document, &mut selection);
        executor
            .execute(
                Box::new(AdjustParagraphIndentCommand::new(start, end, direction)),
                &mut ctx,
            )
            .expect("adjust indent");
    }

    #[test]
    fn list_paragraphs_step_their_level_instead_of_the_indent() {
        use crate::document::Numbering;
        // Word's ribbon: increase indent on a list item promotes its list
        // level; the left indent (driven by the numbering definition) stays
        // untouched. A plain paragraph in the same selection still steps its
        // own indent.
        let mut list_paragraph = paragraph_with_indent("item", None);
        list_paragraph.properties = Some(ParagraphProperties {
            numbering: Some(Numbering {
                id: Some("1".to_string()),
                level: Some(0),
            }),
            ..Default::default()
        });
        let mut document =
            document_with(vec![list_paragraph, paragraph_with_indent("plain", None)]);

        adjust(&mut document, 0, 1, IndentDirection::Increase);

        let list_props = document
            .paragraph(0)
            .and_then(|p| p.properties.clone())
            .expect("list props");
        assert_eq!(
            list_props.numbering.as_ref().and_then(|n| n.level),
            Some(1),
            "increase indent promotes the list level"
        );
        assert!(
            list_props
                .indent
                .as_ref()
                .is_none_or(|indent| indent.left_chars.is_none() && indent.left.is_none()),
            "the list paragraph's own left indent stays untouched"
        );
        assert_eq!(
            indent_of(&document, 1).unwrap().left_chars,
            Some(100),
            "the plain paragraph still steps in character units"
        );

        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        let list_props = document
            .paragraph(0)
            .and_then(|p| p.properties.clone())
            .expect("list props");
        assert_eq!(
            list_props.numbering.as_ref().and_then(|n| n.level),
            Some(0),
            "decrease indent demotes the list level"
        );
    }

    #[test]
    fn list_level_clamps_to_words_range_and_noops_skip_history() {
        use crate::commands::{CommandContext, CommandExecutor};
        use crate::document::Numbering;

        let mut list_paragraph = paragraph_with_indent("item", None);
        list_paragraph.properties = Some(ParagraphProperties {
            numbering: Some(Numbering {
                id: Some("1".to_string()),
                level: Some(0),
            }),
            ..Default::default()
        });
        let mut document = document_with(vec![list_paragraph]);
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        // Decrease at level 0 changes nothing and must not pollute history
        // (a no-op command records no undo step).
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(AdjustParagraphIndentCommand::new(
                        0,
                        0,
                        IndentDirection::Decrease,
                    )),
                    &mut ctx,
                )
                .expect("decrease at level 0");
        }
        assert_eq!(
            executor.undo_depth(),
            0,
            "a level-0 decrease is a no-op and records no history"
        );

        // Level clamps at Word's maximum (8).
        document
            .paragraph_mut(0)
            .unwrap()
            .properties
            .as_mut()
            .unwrap()
            .numbering
            .as_mut()
            .unwrap()
            .level = Some(8);
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(AdjustParagraphIndentCommand::new(
                        0,
                        0,
                        IndentDirection::Increase,
                    )),
                    &mut ctx,
                )
                .expect("increase at max level");
        }
        assert_eq!(
            document
                .paragraph(0)
                .and_then(|p| p.properties.as_ref())
                .and_then(|props| props.numbering.as_ref())
                .and_then(|n| n.level),
            Some(8),
            "the level clamps at 8"
        );
        assert_eq!(
            executor.undo_depth(),
            0,
            "clamped steps stay out of history"
        );
    }

    #[test]
    fn steps_each_paragraph_independently() {
        let mut document = document_with(vec![
            paragraph_with_indent("a", None),
            paragraph_with_indent(
                "b",
                Some(Indent {
                    left_chars: Some(200),
                    ..Default::default()
                }),
            ),
        ]);
        adjust(&mut document, 0, 1, IndentDirection::Increase);
        assert_eq!(indent_of(&document, 0).unwrap().left_chars, Some(100));
        assert_eq!(indent_of(&document, 1).unwrap().left_chars, Some(300));
    }

    #[test]
    fn point_based_indent_snaps_on_the_default_tab_grid_without_unit_change() {
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left: Some(40.0),
                ..Default::default()
            }),
        )]);
        // 40pt with a 36pt default tab stop: increase snaps up to 72pt.
        adjust(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left, Some(72.0));
        assert_eq!(indent.left_chars, None);

        // Decrease from 72pt lands on 36pt, then 0, and stays at an
        // effective zero (a zero indent re-enters character-unit stepping).
        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        assert_eq!(indent_of(&document, 0).unwrap().left, Some(36.0));
        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        assert_eq!(indent_of(&document, 0).unwrap().left, Some(0.0));
        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        let floored = indent_of(&document, 0).unwrap();
        assert_eq!(floored.left.unwrap_or(0.0), 0.0);
        assert_eq!(floored.left_chars.unwrap_or(0), 0);
    }

    #[test]
    fn zero_point_indent_steps_in_character_units() {
        // Style flattening bakes `w:ind w:left="0"` into most paragraphs; a
        // zero indent must still step 1字, not jump to a 36pt tab stop.
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left: Some(0.0),
                right: Some(0.0),
                first_line: Some(0.0),
                ..Default::default()
            }),
        )]);
        adjust(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(100));
        assert_eq!(indent.left, None);
    }

    #[test]
    fn decrease_floors_partial_character_indents_before_stepping() {
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left_chars: Some(150),
                ..Default::default()
            }),
        )]);
        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        assert_eq!(indent_of(&document, 0).unwrap().left_chars, Some(100));
        adjust(&mut document, 0, 0, IndentDirection::Decrease);
        assert_eq!(indent_of(&document, 0).unwrap().left_chars, Some(0));
    }

    #[test]
    fn hanging_and_first_line_survive_indent_stepping() {
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left_chars: Some(100),
                hanging_chars: Some(100),
                first_line: Some(10.5),
                ..Default::default()
            }),
        )]);
        adjust(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(200));
        assert_eq!(indent.hanging_chars, Some(100));
        assert_eq!(indent.first_line, Some(10.5));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// Increase then decrease returns to the starting indent whenever the
        /// start already sits on the step grid, and the indent never goes
        /// negative regardless of history.
        #[test]
        fn increase_then_decrease_round_trips_on_grid(
            whole_chars in 0_i32..=40,
            use_chars in proptest::bool::ANY,
        ) {
            // A zero point indent deliberately re-enters character stepping,
            // so the point-unit round-trip starts at one tab stop or more.
            let whole = if use_chars { whole_chars } else { whole_chars.max(1) };
            let indent = if use_chars {
                Indent { left_chars: Some(whole * 100), ..Default::default() }
            } else {
                Indent { left: Some(whole as f32 * 36.0), ..Default::default() }
            };
            let mut document = document_with(vec![paragraph_with_indent("a", Some(indent))]);
            adjust(&mut document, 0, 0, IndentDirection::Increase);
            adjust(&mut document, 0, 0, IndentDirection::Decrease);
            let result = indent_of(&document, 0).unwrap();
            if use_chars {
                prop_assert_eq!(result.left_chars, Some(whole * 100));
            } else {
                prop_assert_eq!(result.left, Some(whole as f32 * 36.0));
            }
        }

        #[test]
        fn decrease_never_goes_negative(
            start_chars in proptest::option::of(0_i32..=500),
            presses in 1_usize..=8,
        ) {
            let indent = start_chars.map(|chars| Indent {
                left_chars: Some(chars),
                ..Default::default()
            });
            let mut document = document_with(vec![paragraph_with_indent("a", indent)]);
            for _ in 0..presses {
                adjust(&mut document, 0, 0, IndentDirection::Decrease);
            }
            let left_chars = indent_of(&document, 0)
                .and_then(|indent| indent.left_chars)
                .unwrap_or(0);
            prop_assert!(left_chars >= 0);
        }
    }

    fn adjust_hanging(
        document: &mut WordDocument,
        start: usize,
        end: usize,
        direction: IndentDirection,
    ) -> usize {
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(document, &mut selection);
        executor
            .execute(
                Box::new(AdjustParagraphIndentCommand::with_target(
                    start,
                    end,
                    direction,
                    IndentTarget::Hanging,
                )),
                &mut ctx,
            )
            .expect("adjust hanging indent");
        executor.undo_depth()
    }

    /// Where the first line starts, in the paragraph's own unit: the left
    /// edge plus the first-line offset (negative for a hanging indent).
    fn first_line_position_chars(indent: &Indent) -> i32 {
        let left = indent.left_chars.unwrap_or(0);
        let hanging = indent.hanging_chars.unwrap_or(0);
        if hanging > 0 {
            left - hanging
        } else {
            left + indent.first_line_chars.unwrap_or(0)
        }
    }

    #[test]
    fn hanging_step_creates_a_one_character_hanging_indent_from_nothing() {
        // Word's Ctrl+T on an unindented paragraph: the left edge moves one
        // step in and the first line stays at the margin, so the paragraph
        // gains a hanging indent of exactly that step.
        let mut document = document_with(vec![paragraph_with_indent("a", None)]);
        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(100));
        assert_eq!(indent.hanging_chars, Some(100));
        assert_eq!(indent.first_line_chars, None);
        assert_eq!(indent.hanging, None);
        assert_eq!(indent.first_line, None);

        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(200));
        assert_eq!(indent.hanging_chars, Some(200));
    }

    #[test]
    fn hanging_step_keeps_the_first_line_where_it_was() {
        // A 1字 first-line indent on a 1字 left indent puts the first line at
        // 2字. Stepping the hanging indent moves the left edge to 2字, so the
        // first line no longer needs an offset; one more step and it hangs.
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left_chars: Some(100),
                first_line_chars: Some(100),
                ..Default::default()
            }),
        )]);
        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(200));
        assert_eq!(indent.first_line_chars, None);
        assert_eq!(indent.hanging_chars, None);
        assert_eq!(first_line_position_chars(&indent), 200);

        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left_chars, Some(300));
        assert_eq!(indent.hanging_chars, Some(100));
        assert_eq!(first_line_position_chars(&indent), 200);
    }

    #[test]
    fn hanging_step_uses_point_units_for_point_paragraphs() {
        // A Western paragraph (40pt left, first line hanging 20pt back at
        // 20pt) steps its left edge on the 36pt tab grid; the first line
        // stays at 20pt throughout, flipping from hanging to first-line
        // indent once the left edge drops below it.
        let mut document = document_with(vec![paragraph_with_indent(
            "a",
            Some(Indent {
                left: Some(40.0),
                hanging: Some(20.0),
                ..Default::default()
            }),
        )]);
        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left, Some(72.0));
        assert_eq!(indent.hanging, Some(52.0));
        assert_eq!(indent.left_chars, None);
        assert_eq!(indent.hanging_chars, None);

        adjust_hanging(&mut document, 0, 0, IndentDirection::Decrease);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left, Some(36.0));
        assert_eq!(indent.hanging, Some(16.0));

        adjust_hanging(&mut document, 0, 0, IndentDirection::Decrease);
        let indent = indent_of(&document, 0).unwrap();
        assert_eq!(indent.left, Some(0.0));
        assert_eq!(indent.hanging, None);
        assert_eq!(indent.first_line, Some(20.0));
    }

    #[test]
    fn hanging_decrease_at_the_margin_records_no_history() {
        let mut document = document_with(vec![paragraph_with_indent("a", None)]);
        let depth = adjust_hanging(&mut document, 0, 0, IndentDirection::Decrease);
        assert_eq!(depth, 0, "a fully clamped step must not pollute undo");
        assert!(
            indent_of(&document, 0).is_none_or(|indent| indent == Indent::default()),
            "nothing to step back from: {:?}",
            indent_of(&document, 0)
        );
    }

    #[test]
    fn hanging_step_leaves_list_levels_alone() {
        use crate::document::Numbering;
        // Ctrl+T on a list item adjusts the paragraph's own hanging indent;
        // unlike Ctrl+M it never promotes the list level.
        let mut list_paragraph = paragraph_with_indent("item", None);
        list_paragraph.properties = Some(ParagraphProperties {
            numbering: Some(Numbering {
                id: Some("1".to_string()),
                level: Some(0),
            }),
            ..Default::default()
        });
        let mut document = document_with(vec![list_paragraph]);
        adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
        let props = document
            .paragraph(0)
            .and_then(|p| p.properties.clone())
            .expect("list props");
        assert_eq!(props.numbering.as_ref().and_then(|n| n.level), Some(0));
        let indent = props.indent.expect("hanging indent applied");
        assert_eq!(indent.left_chars, Some(100));
        assert_eq!(indent.hanging_chars, Some(100));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// Word's TabHangingIndent contract: the left edge steps on the grid
        /// but the first line never moves, whichever direction is pressed and
        /// whatever special indent the paragraph started with.
        #[test]
        fn hanging_steps_never_move_the_first_line(
            left_chars in 0_i32..=600,
            special in proptest::option::of((proptest::bool::ANY, 1_i32..=300)),
            presses in proptest::collection::vec(proptest::bool::ANY, 1..=6),
        ) {
            let (first_line_chars, hanging_chars) = match special {
                Some((true, value)) => (None, Some(value)),
                Some((false, value)) => (Some(value), None),
                None => (None, None),
            };
            let indent = Indent {
                left_chars: Some(left_chars),
                first_line_chars,
                hanging_chars,
                ..Default::default()
            };
            let mut document = document_with(vec![paragraph_with_indent("a", Some(indent.clone()))]);
            let expected = first_line_position_chars(&indent);
            for increase in presses {
                let direction = if increase {
                    IndentDirection::Increase
                } else {
                    IndentDirection::Decrease
                };
                adjust_hanging(&mut document, 0, 0, direction);
                let result = indent_of(&document, 0).unwrap();
                prop_assert_eq!(first_line_position_chars(&result), expected);
                prop_assert!(result.left_chars.unwrap_or(0) >= 0);
                prop_assert!(result.hanging_chars.unwrap_or(0) >= 0);
                prop_assert!(result.first_line_chars.unwrap_or(0) >= 0);
                prop_assert!(
                    !(result.hanging_chars.is_some() && result.first_line_chars.is_some()),
                    "hanging and first line are mutually exclusive: {:?}",
                    result
                );
            }
        }

        /// On the grid, one hanging step in and one back is the identity.
        #[test]
        fn hanging_increase_then_decrease_round_trips_on_grid(
            whole_chars in 0_i32..=20,
            hanging_whole in 0_i32..=5,
        ) {
            let indent = Indent {
                left_chars: Some(whole_chars * 100),
                hanging_chars: (hanging_whole > 0).then_some(hanging_whole * 100),
                ..Default::default()
            };
            let mut document = document_with(vec![paragraph_with_indent("a", Some(indent.clone()))]);
            adjust_hanging(&mut document, 0, 0, IndentDirection::Increase);
            adjust_hanging(&mut document, 0, 0, IndentDirection::Decrease);
            let result = indent_of(&document, 0).unwrap();
            prop_assert_eq!(result.left_chars, Some(whole_chars * 100));
            prop_assert_eq!(first_line_position_chars(&result), first_line_position_chars(&indent));
        }
    }
}
