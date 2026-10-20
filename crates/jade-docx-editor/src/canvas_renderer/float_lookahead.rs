//! Word for Mac's look-ahead float wrap in compatibility mode 14 and earlier.
//!
//! Word lays a paragraph out around a square-wrapped shape anchored in the
//! paragraph after it (`floating-square-wrap`, `floating-wrap-sides`,
//! `floating-simple-position`, Word for Mac 16.114). A shape placed from the
//! anchor paragraph counts when its first-pass top reaches the end of the
//! earlier paragraph's space after (touching counts, 0.25pt below does not)
//! and that paragraph's last line crosses the shape; the earlier paragraph
//! then wraps as if the shape sat at its own top plus the shape's offset. A
//! shape placed on the page wraps the lines it overlaps. The shape stays where
//! the first pass put it: when the earlier paragraph grows, the anchor
//! paragraph moves down and the shape does not. The same documents saved in
//! compatibility mode 15 keep the earlier paragraph at the full width.

use super::*;

/// A floating object of the paragraph after the one being laid out.
pub(super) struct LookaheadObject {
    run_index: usize,
    kind: FloatingObjectKind,
    width: Option<f64>,
    height: Option<f64>,
    layout: RunImageLayout,
}

/// The floating objects of the next body paragraph, offered to the paragraph
/// being laid out.
pub(super) struct FloatLookahead {
    /// Body element index of the paragraph that anchors the objects.
    anchor_element: usize,
    /// How far below the end of the current paragraph's space after the
    /// anchor paragraph starts (its collapsed space before), px.
    anchor_gap: f64,
    objects: Vec<LookaheadObject>,
}

impl FloatLookahead {
    pub(super) fn anchor_element(&self) -> usize {
        self.anchor_element
    }
}

/// Holds an anchor paragraph's objects where the first pass put them after
/// the paragraph above grew around them.
#[derive(Clone, Copy, Debug)]
pub(super) struct FloatLookaheadPin {
    pub(super) element: usize,
    pub(super) page_number: usize,
    pub(super) shift: f64,
}

/// What the earlier paragraph needs to pin its successor's objects once its
/// lines are placed.
pub(super) struct FloatLookaheadApplied {
    pub(super) anchor_element: usize,
    pub(super) natural_lines: usize,
}

/// Square-wrapped floating objects only: the probes show nothing else.
fn lookahead_layout(layout: &RunImageLayout) -> bool {
    matches!(layout.kind, RunImageKind::Floating)
        && matches!(layout.wrap, WrapMode::Square)
        && !layout.behind_doc
}

/// Positioned on the page rather than from the anchor paragraph.
fn page_positioned(layout: &RunImageLayout) -> bool {
    layout.simple_position.is_some()
        || layout
            .position_v
            .as_ref()
            .and_then(|position| position.relative_from.as_deref())
            .is_some_and(|relative| {
                matches!(relative, "page" | "margin" | "topMargin" | "bottomMargin")
            })
}

/// Placed from the anchor paragraph's top by an explicit offset.
fn paragraph_offset(layout: &RunImageLayout) -> Option<f64> {
    if layout.simple_position.is_some() {
        return None;
    }
    let position = layout.position_v.as_ref()?;
    if !matches!(position.relative_from.as_deref(), None | Some("paragraph")) {
        return None;
    }
    if position.align.is_some() && position.offset_px.is_none() {
        return None;
    }
    Some(position.offset_px.unwrap_or(0.0))
}

impl<P: Painter> CanvasRenderer<P>
where
    P::Error: PainterError,
{
    /// The objects the body paragraph at `index` must wrap around ahead of
    /// their anchor, the next paragraph. Cheap rejections first: this runs
    /// for every body paragraph.
    pub(super) fn float_lookahead_for(
        &self,
        elements: &[BodyElement],
        index: usize,
        section_end: usize,
    ) -> Option<FloatLookahead> {
        if self
            .document_east_asian_rules
            .compatibility_mode
            .is_none_or(|mode| mode > 14)
            || jade_docx_layout::line_metrics::word_layout_platform()
                != jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
            || self.measuring_keep_next
            || self.pageless_mode
            || self.rendering_header_footer_story
            || self.rendering_note_story
        {
            return None;
        }
        let anchor_element = index + 1;
        if anchor_element >= section_end {
            return None;
        }
        let anchor = elements.get(anchor_element)?.as_paragraph()?;
        if !anchor.runs.iter().any(|run| {
            run.image
                .as_ref()
                .is_some_and(|image| lookahead_layout(&image.layout))
                || run
                    .text_box
                    .as_ref()
                    .is_some_and(|text_box| lookahead_layout(&text_box.layout))
        }) {
            return None;
        }
        let paragraph = elements.get(index)?.as_paragraph()?;
        let page_break = |paragraph: &Paragraph| {
            paragraph
                .runs
                .iter()
                .any(|run| run.text.contains(PAGE_BREAK_CHAR))
        };
        if page_break(paragraph)
            || page_break(anchor)
            || paragraph
                .properties
                .as_ref()
                .is_some_and(|props| props.frame.is_some() || props.section.is_some())
            || anchor.properties.as_ref().is_some_and(|props| {
                props.frame.is_some() || props.page_break_before.unwrap_or(false)
            })
        {
            return None;
        }
        let objects = anchor
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.revision.is_none())
            .filter_map(|(run_index, run)| {
                if let Some(text_box) = run.text_box.as_ref() {
                    return lookahead_layout(&text_box.layout).then(|| LookaheadObject {
                        run_index,
                        kind: FloatingObjectKind::TextBox,
                        width: text_box.width_px,
                        height: text_box.height_px,
                        layout: text_box.layout.clone(),
                    });
                }
                let image = run.image.as_ref()?;
                lookahead_layout(&image.layout).then(|| LookaheadObject {
                    run_index,
                    kind: FloatingObjectKind::Image,
                    width: image.width_px,
                    height: image.height_px,
                    layout: image.layout.clone(),
                })
            })
            .collect::<Vec<_>>();
        if objects.is_empty() {
            return None;
        }
        // Line heights only scale `w:beforeLines`/`w:afterLines`; any such
        // spacing already keeps the anchor well below the paragraph.
        const SPACING_LINE_PT: f32 = 12.0;
        let anchor_gap = if self.contextual_suppresses_spacing_before(anchor_element) {
            0.0
        } else {
            let after = if self.contextual_suppresses_spacing_after(index) {
                0.0
            } else {
                jade_docx_layout::text_extract::spacing_after_px(paragraph, SPACING_LINE_PT)
            };
            jade_docx_layout::line_metrics::collapsed_spacing_before(
                jade_docx_layout::text_extract::spacing_before_px(anchor, SPACING_LINE_PT),
                after,
            )
        };
        Some(FloatLookahead {
            anchor_element,
            anchor_gap,
            objects,
        })
    }

    /// Wrap exclusions for the next paragraph's objects that this paragraph
    /// must wrap around, given its lines laid out without them. `None` when
    /// none applies.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn float_lookahead_exclusions(
        &self,
        lookahead: &FloatLookahead,
        paragraph_index: usize,
        natural_lines: &[WrappedLine],
        line_left: f64,
        line_right: f64,
        paragraph_style: &ParagraphStyle,
        paragraph_top_y: f64,
        line_height: f64,
    ) -> Option<Vec<FloatExclusion>> {
        let last = natural_lines.last()?;
        let text_width = last
            .clusters
            .iter()
            .rposition(|cluster| !cluster.is_whitespace)
            .map(|end| {
                last.clusters[..=end]
                    .iter()
                    .map(|cluster| cluster.width)
                    .sum::<f64>()
            })?;
        let start = line_left
            + if natural_lines.len() == 1 {
                paragraph_style.first_line_indent
            } else {
                0.0
            };
        let free = (line_right - start - text_width).max(0.0);
        let text_left = start
            + match paragraph_style.alignment.as_str() {
                "center" => free / 2.0,
                "right" | "end" => free,
                _ => 0.0,
            };
        let text_right = text_left + text_width;
        // Allow for rounding where the shape only touches the space after.
        const TOUCH_EPSILON_PX: f64 = 0.01;
        let exclusions = lookahead
            .objects
            .iter()
            .filter(|object| {
                page_positioned(&object.layout)
                    || paragraph_offset(&object.layout)
                        .is_some_and(|offset| offset + lookahead.anchor_gap <= TOUCH_EPSILON_PX)
            })
            .filter_map(|object| {
                // Laid out as if the paragraph itself anchored the object.
                self.floating_object_wrap_exclusion(
                    FloatingObjectTarget {
                        paragraph_index: paragraph_index + 1,
                        run_index: object.run_index,
                        kind: object.kind,
                    },
                    object.width.unwrap_or(line_height).max(1.0),
                    object.height.unwrap_or(line_height).max(1.0),
                    &object.layout,
                    paragraph_top_y,
                    line_height,
                )
            })
            .filter(|bounds| text_right > bounds.left && text_left < bounds.right)
            .collect::<Vec<_>>();
        (!exclusions.is_empty()).then_some(exclusions)
    }

    /// Where the anchor paragraph at `body_element_index` places its floating
    /// objects: its top, or the first pass's top while a pin holds them.
    pub(super) fn float_anchor_top(&self, body_element_index: usize, paragraph_top_y: f64) -> f64 {
        self.float_lookahead_pin
            .filter(|pin| {
                pin.element == body_element_index && pin.page_number == self.current_page_number
            })
            .map_or(paragraph_top_y, |pin| paragraph_top_y - pin.shift)
    }
}
