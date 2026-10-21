//! Geometry of positioned paragraph text frames (`w:framePr`, ECMA-376 Part 1
//! §17.3.1.11).
//!
//! A frame takes its paragraphs out of the text flow and places them in a box
//! at a fixed position; the surrounding body text wraps around that box
//! through the same [`FloatExclusion`] planner as floating drawings. This
//! module owns the frame's size, position and wrap region so renderers only
//! consume the result.
//!
//! Attribute defaults follow ECMA-376 (`hAnchor`/`vAnchor` default to `page`),
//! except that a frame without any horizontal (`x`/`xAlign`) or vertical
//! (`y`/`yAlign`) position stays where its paragraph would have been: the
//! text column horizontally and the flow position vertically. Word-authored
//! frames always write both anchors, so the default only affects third-party
//! producers. Word pixel parity is still unverified (issue #248).

use crate::line_wrap::{
    resolve_indent_right_pt, FloatExclusion, WrappedLine, DEFAULT_INDENT_CHAR_UNIT_PT,
};
use crate::units::points_to_px;
use jade_docx_reader::document::{
    FrameAlign, FrameAnchor, FrameHeightRule, FrameWrap, Indent, ParagraphFrame, WrapTextSide,
};

/// Twentieths of a point to CSS pixels at 96 dpi.
pub fn frame_twips_to_px(twips: i32) -> f64 {
    twips as f64 / 15.0
}

/// Page-local reference boxes (CSS px) a frame can be positioned against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReferenceBoxes {
    pub page_width: f64,
    pub page_height: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    /// Left edge and width of the current text column.
    pub column_left: f64,
    pub column_width: f64,
    /// Top of the anchor paragraph: where the frame paragraph would have
    /// started in the flow.
    pub anchor_top: f64,
    /// 1-based page number, for `inside`/`outside` alignment.
    pub page_number: usize,
}

/// A resolved frame box in page-local CSS px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphFrameRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl ParagraphFrameRect {
    pub fn right(&self) -> f64 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// Whether a paragraph's frame continues the previous paragraph's frame.
/// ECMA-376 §17.3.1.11 groups adjacent paragraphs into one frame when their
/// frame properties are identical.
pub fn paragraph_frames_share_box(
    previous: Option<&ParagraphFrame>,
    next: Option<&ParagraphFrame>,
) -> bool {
    matches!((previous, next), (Some(a), Some(b)) if a == b)
}

/// Frame width: `w:w` when authored, otherwise the widest content line
/// (`natural_content_width`) bounded by the text column, as Word sizes an
/// auto-width frame to its text.
pub fn resolve_paragraph_frame_width(
    frame: &ParagraphFrame,
    column_width: f64,
    natural_content_width: f64,
) -> f64 {
    match frame.width_twips {
        Some(width) => frame_twips_to_px(width).max(1.0),
        None => natural_content_width
            .ceil()
            .clamp(1.0, column_width.max(1.0)),
    }
}

/// Widest line of one paragraph (or one hard-break chunk of it) of an
/// auto-width frame. `lines` are the chunk's lines wrapped against an
/// unbounded right edge; the line extent is its cluster advances plus the
/// paragraph's start indent, the first-line indent on the chunk's first line
/// only (`first_line_indent_px`, `0` for a continuation chunk) and the end
/// indent from `indent`. The frame width is the maximum over its paragraphs,
/// passed to [`resolve_paragraph_frame_width`].
pub fn paragraph_frame_natural_line_width(
    lines: &[WrappedLine],
    indent_left_px: f64,
    first_line_indent_px: f64,
    indent: Option<&Indent>,
) -> f64 {
    let indent_right = indent
        .and_then(|indent| resolve_indent_right_pt(indent, false, DEFAULT_INDENT_CHAR_UNIT_PT))
        .map(|points| points_to_px(points as f32))
        .unwrap_or(0.0)
        .max(0.0);
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let advance: f64 = line.clusters.iter().map(|cluster| cluster.width).sum();
            let first_line = if index == 0 {
                first_line_indent_px
            } else {
                0.0
            };
            (indent_left_px + first_line).max(0.0) + advance + indent_right
        })
        .fold(0.0, f64::max)
}

/// Frame height from `w:h`/`w:hRule` and the laid-out content height.
pub fn resolve_paragraph_frame_height(frame: &ParagraphFrame, content_height: f64) -> f64 {
    let authored = frame.height_twips.map(frame_twips_to_px);
    let height = match (frame.height_rule, authored) {
        (FrameHeightRule::Exact, Some(height)) => height,
        (FrameHeightRule::AtLeast, Some(height)) => height.max(content_height),
        _ => content_height,
    };
    height.max(0.0)
}

/// Whether `height` cuts the frame content off (`hRule="exact"`).
pub fn paragraph_frame_clips_content(frame: &ParagraphFrame) -> bool {
    frame.height_rule == FrameHeightRule::Exact && frame.height_twips.is_some()
}

fn align_within(align: FrameAlign, start: f64, extent: f64, size: f64, page: usize) -> f64 {
    let odd_page = page % 2 == 1;
    match align {
        FrameAlign::Start | FrameAlign::Inline => start,
        FrameAlign::Center => start + (extent - size) / 2.0,
        FrameAlign::End => start + extent - size,
        // Inside is the binding edge: left/top on odd (recto) pages.
        FrameAlign::Inside if odd_page => start,
        FrameAlign::Inside => start + extent - size,
        FrameAlign::Outside if odd_page => start + extent - size,
        FrameAlign::Outside => start,
    }
}

/// Places a `width` × `height` frame against the reference boxes.
pub fn resolve_paragraph_frame_rect(
    frame: &ParagraphFrame,
    boxes: &FrameReferenceBoxes,
    width: f64,
    height: f64,
) -> ParagraphFrameRect {
    let horizontal_unpositioned = frame.x_twips.is_none() && frame.x_align.is_none();
    let h_anchor = if horizontal_unpositioned && frame.h_anchor.is_none() {
        FrameAnchor::Text
    } else {
        frame.h_anchor.unwrap_or(FrameAnchor::Page)
    };
    let (box_left, box_width) = match h_anchor {
        FrameAnchor::Text => (boxes.column_left, boxes.column_width),
        FrameAnchor::Margin => (
            boxes.margin_left,
            (boxes.page_width - boxes.margin_left - boxes.margin_right).max(0.0),
        ),
        FrameAnchor::Page => (0.0, boxes.page_width),
    };
    let left = match frame.x_align {
        Some(align) => align_within(align, box_left, box_width, width, boxes.page_number),
        None => box_left + frame.x_twips.map(frame_twips_to_px).unwrap_or(0.0),
    };

    let vertical_unpositioned = frame.y_twips.is_none()
        && frame
            .y_align
            .is_none_or(|align| align == FrameAlign::Inline);
    let top = if vertical_unpositioned {
        boxes.anchor_top
    } else {
        let v_anchor = frame.v_anchor.unwrap_or(FrameAnchor::Page);
        let (box_top, box_height) = match v_anchor {
            // A paragraph anchor has no extent; alignment collapses to it.
            FrameAnchor::Text => (boxes.anchor_top, height),
            FrameAnchor::Margin => (
                boxes.margin_top,
                (boxes.page_height - boxes.margin_top - boxes.margin_bottom).max(0.0),
            ),
            FrameAnchor::Page => (0.0, boxes.page_height),
        };
        match frame.y_align {
            Some(align) if align != FrameAlign::Inline => {
                align_within(align, box_top, box_height, height, 1)
            }
            _ => box_top + frame.y_twips.map(frame_twips_to_px).unwrap_or(0.0),
        }
    };

    ParagraphFrameRect {
        left,
        top,
        width,
        height,
    }
}

/// The region body text wraps around, or `None` when text runs through the
/// frame (`w:wrap="none"`). `notBeside` keeps the lines beside the frame
/// empty; every other mode wraps on the open side(s), with `w:hSpace` /
/// `w:vSpace` as the distance from the surrounding text.
pub fn paragraph_frame_wrap_exclusion(
    frame: &ParagraphFrame,
    rect: &ParagraphFrameRect,
) -> Option<FloatExclusion> {
    let wrap = frame.wrap.unwrap_or(FrameWrap::Auto);
    if wrap == FrameWrap::None || rect.height <= 0.0 {
        return None;
    }
    let h_space = frame.h_space_twips.map(frame_twips_to_px).unwrap_or(0.0);
    let v_space = frame.v_space_twips.map(frame_twips_to_px).unwrap_or(0.0);
    let (left, right) = if wrap == FrameWrap::NotBeside {
        (f64::NEG_INFINITY, f64::INFINITY)
    } else {
        (rect.left - h_space, rect.right() + h_space)
    };
    Some(FloatExclusion {
        top: rect.top - v_space,
        bottom: rect.bottom() + v_space,
        left,
        right,
        wrap_text: WrapTextSide::BothSides,
        polygon: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_wrap::plan_usable_line_segments_around_floats;

    fn boxes() -> FrameReferenceBoxes {
        // Letter page, 1" margins, single column, anchor paragraph at 200px.
        FrameReferenceBoxes {
            page_width: 816.0,
            page_height: 1056.0,
            margin_left: 96.0,
            margin_right: 96.0,
            margin_top: 96.0,
            margin_bottom: 96.0,
            column_left: 96.0,
            column_width: 624.0,
            anchor_top: 200.0,
            page_number: 1,
        }
    }

    #[test]
    fn page_anchored_offsets_measure_from_the_page_edge() {
        let frame = ParagraphFrame {
            width_twips: Some(2880),
            height_twips: Some(1440),
            height_rule: FrameHeightRule::Exact,
            x_twips: Some(4320),
            y_twips: Some(2880),
            h_anchor: Some(FrameAnchor::Page),
            v_anchor: Some(FrameAnchor::Page),
            ..ParagraphFrame::default()
        };
        let width = resolve_paragraph_frame_width(&frame, 624.0, 10.0);
        let height = resolve_paragraph_frame_height(&frame, 400.0);
        assert_eq!((width, height), (192.0, 96.0));
        assert!(paragraph_frame_clips_content(&frame));
        let rect = resolve_paragraph_frame_rect(&frame, &boxes(), width, height);
        assert_eq!((rect.left, rect.top), (288.0, 192.0));
    }

    #[test]
    fn text_and_margin_anchors_and_alignment() {
        let frame = ParagraphFrame {
            width_twips: Some(1440),
            x_align: Some(FrameAlign::End),
            h_anchor: Some(FrameAnchor::Margin),
            v_anchor: Some(FrameAnchor::Text),
            y_twips: Some(150),
            ..ParagraphFrame::default()
        };
        let rect = resolve_paragraph_frame_rect(&frame, &boxes(), 96.0, 40.0);
        assert_eq!(rect.left, 96.0 + 624.0 - 96.0);
        assert_eq!(rect.top, 210.0);

        let centered = ParagraphFrame {
            x_align: Some(FrameAlign::Center),
            y_align: Some(FrameAlign::Center),
            h_anchor: Some(FrameAnchor::Page),
            v_anchor: Some(FrameAnchor::Margin),
            ..ParagraphFrame::default()
        };
        let rect = resolve_paragraph_frame_rect(&centered, &boxes(), 216.0, 64.0);
        assert_eq!(rect.left, 300.0);
        assert_eq!(rect.top, 96.0 + (864.0 - 64.0) / 2.0);
    }

    #[test]
    fn inside_and_outside_follow_the_binding_edge() {
        let frame = ParagraphFrame {
            x_align: Some(FrameAlign::Outside),
            h_anchor: Some(FrameAnchor::Margin),
            ..ParagraphFrame::default()
        };
        let odd = resolve_paragraph_frame_rect(&frame, &boxes(), 100.0, 20.0);
        let even = resolve_paragraph_frame_rect(
            &frame,
            &FrameReferenceBoxes {
                page_number: 2,
                ..boxes()
            },
            100.0,
            20.0,
        );
        assert_eq!(odd.left, 96.0 + 624.0 - 100.0);
        assert_eq!(even.left, 96.0);
    }

    #[test]
    fn unpositioned_frame_stays_at_its_flow_position() {
        let frame = ParagraphFrame {
            width_twips: Some(1440),
            wrap: Some(FrameWrap::Around),
            ..ParagraphFrame::default()
        };
        let rect = resolve_paragraph_frame_rect(&frame, &boxes(), 96.0, 30.0);
        assert_eq!((rect.left, rect.top), (96.0, 200.0));
    }

    #[test]
    fn auto_width_shrinks_to_content_within_the_column() {
        let frame = ParagraphFrame::default();
        assert_eq!(resolve_paragraph_frame_width(&frame, 624.0, 120.2), 121.0);
        assert_eq!(resolve_paragraph_frame_width(&frame, 624.0, 9_000.0), 624.0);
    }

    fn line_of(advances: &[f64]) -> WrappedLine {
        use crate::line_wrap::{InlineCluster, InlineClusterKind, LineBreakKind};
        WrappedLine {
            segments: Vec::new(),
            break_kind: LineBreakKind::Soft,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: advances
                .iter()
                .map(|width| InlineCluster {
                    start: 0,
                    end: 1,
                    width: *width,
                    text: "a".into(),
                    kind: InlineClusterKind::Grapheme,
                    is_whitespace: false,
                    forbid_line_start: false,
                    forbid_line_end: false,
                    can_hang_line_end: false,
                    auto_space_leading: 0.0,
                    auto_space_trailing: 0.0,
                    layout: None,
                    inline_object: None,
                })
                .collect(),
            hyphen: None,
        }
    }

    #[test]
    fn natural_width_adds_indents_and_the_first_line_indent_only_once() {
        let lines = [line_of(&[10.0, 10.0, 10.0]), line_of(&[10.0; 5])];
        let indent = Indent {
            right: Some(6.0),
            ..Indent::default()
        };
        // Line 1: 5 + 20 + 30 + 8; line 2: 5 + 50 + 8.
        let width = paragraph_frame_natural_line_width(&lines, 5.0, 20.0, Some(&indent));
        assert!((width - 63.0).abs() < 1e-9, "{width}");
        let wide_first = paragraph_frame_natural_line_width(&lines, 5.0, 40.0, None);
        assert!((wide_first - 75.0).abs() < 1e-9, "{wide_first}");
        // A hanging indent never pulls a line before the frame edge.
        let hanging = paragraph_frame_natural_line_width(&lines[..1], 5.0, -20.0, None);
        assert!((hanging - 30.0).abs() < 1e-9, "{hanging}");
        assert_eq!(paragraph_frame_natural_line_width(&[], 5.0, 0.0, None), 0.0);
    }

    #[test]
    fn height_rules() {
        let at_least = ParagraphFrame {
            height_twips: Some(1500),
            height_rule: FrameHeightRule::AtLeast,
            ..ParagraphFrame::default()
        };
        assert_eq!(resolve_paragraph_frame_height(&at_least, 50.0), 100.0);
        assert_eq!(resolve_paragraph_frame_height(&at_least, 150.0), 150.0);
        assert!(!paragraph_frame_clips_content(&at_least));
        let auto = ParagraphFrame {
            height_twips: Some(1500),
            height_rule: FrameHeightRule::Auto,
            ..ParagraphFrame::default()
        };
        assert_eq!(resolve_paragraph_frame_height(&auto, 50.0), 50.0);
    }

    #[test]
    fn wrap_modes_produce_the_expected_exclusions() {
        let rect = ParagraphFrameRect {
            left: 96.0,
            top: 200.0,
            width: 192.0,
            height: 96.0,
        };
        let around = ParagraphFrame {
            wrap: Some(FrameWrap::Around),
            h_space_twips: Some(180),
            v_space_twips: Some(60),
            ..ParagraphFrame::default()
        };
        let exclusion = paragraph_frame_wrap_exclusion(&around, &rect).unwrap();
        assert_eq!(exclusion.left, 84.0);
        assert_eq!(exclusion.right, 300.0);
        assert_eq!(exclusion.top, 196.0);
        assert_eq!(exclusion.bottom, 300.0);

        // Lines beside a left frame start after it and its distance.
        let segments =
            plan_usable_line_segments_around_floats(200.0, 20.0, 6, 96.0, 720.0, &[exclusion]);
        assert_eq!(segments[0].start_x, 300.0);
        assert_eq!(segments[0].width, 420.0);
        assert_eq!(segments[5].start_x, 96.0);

        let not_beside = ParagraphFrame {
            wrap: Some(FrameWrap::NotBeside),
            ..ParagraphFrame::default()
        };
        let exclusion = paragraph_frame_wrap_exclusion(&not_beside, &rect).unwrap();
        let segments =
            plan_usable_line_segments_around_floats(200.0, 24.0, 6, 96.0, 720.0, &[exclusion]);
        assert_eq!(segments[0].skipped_lines_before, 4);
        assert_eq!(segments[0].start_x, 96.0);

        let none = ParagraphFrame {
            wrap: Some(FrameWrap::None),
            ..ParagraphFrame::default()
        };
        assert!(paragraph_frame_wrap_exclusion(&none, &rect).is_none());
    }

    #[test]
    fn identical_frames_share_one_box() {
        let a = ParagraphFrame {
            width_twips: Some(1000),
            ..ParagraphFrame::default()
        };
        let b = ParagraphFrame {
            width_twips: Some(1001),
            ..ParagraphFrame::default()
        };
        assert!(paragraph_frames_share_box(Some(&a), Some(&a)));
        assert!(!paragraph_frames_share_box(Some(&a), Some(&b)));
        assert!(!paragraph_frames_share_box(None, Some(&a)));
    }
}
