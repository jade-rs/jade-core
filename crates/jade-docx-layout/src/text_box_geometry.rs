//! Physical text bounds within a floating shape, before shared line wrapping.

use jade_docx_reader::document::{TextBoxInsets, TextBoxShape};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBoxContentBounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Resolve the default DrawingML text rectangle, authored text margins and
/// half of the visible outline stroke. Coordinates are relative to the shape.
///
/// `roundRect` and `triangle` follow ECMA-376 Part 1's accompanying
/// `presetShapeDefinitions.xml`: roundRect's `il = ss * 16667/100000 *
/// 29289/100000`; triangle's rectangle is `(wd4, vc, 3*wd4, b)`.
/// Other presets retain the outer-rectangle fallback until their text
/// rectangles and adjustments are represented. Visible stroke clearance is
/// supported by the original Word PDFs across these three shape families.
/// This function neither measures text nor decides line breaks.
pub fn text_box_content_bounds(
    shape: TextBoxShape,
    width: f64,
    height: f64,
    insets: TextBoxInsets,
    visible_stroke_width: f64,
) -> TextBoxContentBounds {
    let width = width.max(0.0);
    let height = height.max(0.0);
    let (left, top, right, bottom) = match shape {
        TextBoxShape::RoundedRectangle => {
            let inset = width.min(height) * (16667.0 / 100000.0) * (29289.0 / 100000.0);
            (inset, inset, width - inset, height - inset)
        }
        TextBoxShape::Triangle => (width / 4.0, height / 2.0, width * 0.75, height),
        _ => (0.0, 0.0, width, height),
    };
    let half_stroke = visible_stroke_width.max(0.0) / 2.0;
    let content_left = (left + insets.left_px + half_stroke).min(right);
    let content_top = (top + insets.top_px + half_stroke).min(bottom);
    TextBoxContentBounds {
        left: content_left,
        top: content_top,
        right: (right - insets.right_px - half_stroke).max(content_left),
        bottom: (bottom - insets.bottom_px - half_stroke).max(content_top),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO_INSETS: TextBoxInsets = TextBoxInsets {
        left_px: 0.0,
        top_px: 0.0,
        right_px: 0.0,
        bottom_px: 0.0,
    };

    #[test]
    fn preset_text_rectangles_follow_the_standard_and_scale_with_the_shape() {
        for scale in [0.25, 1.0, 4.0] {
            let triangle = text_box_content_bounds(
                TextBoxShape::Triangle,
                240.0 * scale,
                192.0 * scale,
                ZERO_INSETS,
                0.0,
            );
            assert_eq!(
                triangle,
                TextBoxContentBounds {
                    left: 60.0 * scale,
                    top: 96.0 * scale,
                    right: 180.0 * scale,
                    bottom: 192.0 * scale
                }
            );
            let rounded = text_box_content_bounds(
                TextBoxShape::RoundedRectangle,
                240.0 * scale,
                100.0 * scale,
                ZERO_INSETS,
                0.0,
            );
            let inset = 100.0 * scale * 16667.0 * 29289.0 / 10000000000.0;
            assert!((rounded.left - inset).abs() < 1e-12);
            assert!((rounded.top - inset).abs() < 1e-12);
            assert!((rounded.right + inset - 240.0 * scale).abs() < 1e-12);
            assert!((rounded.bottom + inset - 100.0 * scale).abs() < 1e-12);
        }
    }

    #[test]
    fn authored_asymmetric_margins_and_visible_stroke_share_one_content_rect() {
        let margins = TextBoxInsets {
            left_px: 20.0,
            top_px: 10.0,
            right_px: 30.0,
            bottom_px: 40.0,
        };
        let borderless =
            text_box_content_bounds(TextBoxShape::Rectangle, 384.0, 192.0, margins, 0.0);
        assert_eq!(
            borderless,
            TextBoxContentBounds {
                left: 20.0,
                top: 10.0,
                right: 354.0,
                bottom: 152.0
            }
        );
        let bordered =
            text_box_content_bounds(TextBoxShape::Rectangle, 384.0, 192.0, margins, 8.0 / 3.0);
        assert_eq!(bordered.left, borderless.left + 4.0 / 3.0);
        assert_eq!(bordered.top, borderless.top + 4.0 / 3.0);
        assert_eq!(bordered.right, borderless.right - 4.0 / 3.0);
        assert_eq!(bordered.bottom, borderless.bottom - 4.0 / 3.0);
        // Symmetric stroke clearance does not move a centered single-line
        // text body vertically, even when authored top/bottom margins differ.
        assert_eq!(
            bordered.top + bordered.bottom,
            borderless.top + borderless.bottom
        );
    }

    #[test]
    fn exhausted_text_rectangle_is_nonnegative() {
        for shape in [
            TextBoxShape::Rectangle,
            TextBoxShape::RoundedRectangle,
            TextBoxShape::Triangle,
        ] {
            let bounds = text_box_content_bounds(shape, 8.0, 4.0, TextBoxInsets::default(), 20.0);
            assert!(bounds.right >= bounds.left && bounds.bottom >= bounds.top);
            assert!(bounds.right <= 8.0 && bounds.bottom <= 4.0);
        }
    }
}
