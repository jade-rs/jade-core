//! Paragraph layout module for separating layout computation from rendering.
//!
//! This module provides helpers for paragraph layout that can be tested
//! independently from the canvas rendering.

use crate::document::{CommentMarker, CommentRange, Indent, Paragraph, ParagraphProperties, Spacing};
use crate::draw_command::{DrawCommandBuffer, TextStyle};

/// Context for paragraph layout computation.
///
/// This structure holds all the computed values needed for laying out
/// a paragraph, separated from the actual drawing operations.
#[derive(Clone, Debug)]
pub struct ParagraphLayoutContext {
    /// The X position where paragraph content starts (after indentation).
    pub content_start_x: f64,
    /// The starting X of the column.
    pub column_x: f64,
    /// The width of the current column.
    pub column_width: f64,
    /// The right edge of the column.
    pub column_right: f64,
    /// Computed line height in CSS pixels.
    pub line_height: f64,
    /// Line height in points.
    pub line_height_points: f32,
    /// Whether this paragraph has numbering.
    pub has_numbering: bool,
    /// Whether to render track changes.
    pub render_changes: bool,
    /// Whether the entire paragraph is deleted.
    pub paragraph_deleted: bool,
    /// Whether to render deleted numbering.
    pub render_deleted_number: bool,
}

impl ParagraphLayoutContext {
    /// Creates a new paragraph layout context with default values.
    pub fn new(column_x: f64, column_width: f64) -> Self {
        Self {
            content_start_x: column_x,
            column_x,
            column_width,
            column_right: column_x + column_width,
            line_height: 14.0,
            line_height_points: 12.0,
            has_numbering: false,
            render_changes: false,
            paragraph_deleted: false,
            render_deleted_number: false,
        }
    }
}

/// Result of extracting paragraph style information.
#[derive(Clone, Debug, Default)]
pub struct ExtractedParagraphStyle {
    /// Text alignment (left, center, right, justify).
    pub alignment: String,
    /// Whether `w:bidi` selects an RTL paragraph base direction.
    pub bidi: bool,
    /// First line indent in CSS pixels.
    pub first_line_indent: f64,
    /// Right indent in CSS pixels.
    pub right_indent: f64,
    /// Left indent applied to x position (not stored in style).
    pub left_indent_px: f64,
    /// Whether the paragraph has numbering.
    pub has_numbering: bool,
    /// Spacing before in points (if specified).
    pub spacing_before: Option<f32>,
    /// Spacing before in lines (if specified).
    pub spacing_before_lines: Option<f32>,
    /// Spacing after in points (if specified).
    pub spacing_after: Option<f32>,
    /// Spacing after in lines (if specified).
    pub spacing_after_lines: Option<f32>,
}

/// Extracts style information from paragraph properties.
///
/// This is a pure function that doesn't require canvas state,
/// making it easy to test.
pub fn extract_paragraph_style(props: Option<&ParagraphProperties>) -> ExtractedParagraphStyle {
    let mut result = ExtractedParagraphStyle::default();

    let Some(props) = props else {
        return result;
    };

    // Alignment
    if let Some(alignment) = &props.alignment {
        result.alignment = alignment.clone();
    }
    result.bidi = props.bidi.unwrap_or(false);
    if result.alignment.is_empty() && result.bidi {
        result.alignment = "right".to_string();
    }

    // Numbering check
    result.has_numbering = props.numbering.is_some();

    // Indent
    if let Some(indent) = &props.indent {
        if let Some(left) = indent.physical_left(result.bidi) {
            result.left_indent_px = points_to_px(left);
        }
        if let Some(first_line) = indent.first_line {
            result.first_line_indent = points_to_px(first_line);
        } else if !result.has_numbering {
            if let Some(hanging) = indent.hanging {
                result.first_line_indent = -points_to_px(hanging);
            }
        }
        if let Some(right) = indent.physical_right(result.bidi) {
            result.right_indent = points_to_px(right);
        }
    }

    // Spacing
    if let Some(spacing) = &props.spacing {
        result.spacing_before = spacing.before;
        result.spacing_before_lines = spacing.before_lines;
        result.spacing_after = spacing.after;
        result.spacing_after_lines = spacing.after_lines;
    }

    result
}

/// Converts points to CSS pixels.
/// This is duplicated here to avoid circular dependencies.
#[inline]
pub fn points_to_px(points: f32) -> f64 {
    (points as f64) * 96.0 / 72.0
}

/// Estimates the predominant font size in a paragraph.
///
/// Returns the maximum font size found in any run, defaulting to 12pt.
pub fn estimate_paragraph_font_size(paragraph: &Paragraph) -> f32 {
    let mut max_size = 12.0_f32;

    for run in &paragraph.runs {
        if let Some(props) = &run.properties {
            if let Some(size) = props.font_size {
                max_size = max_size.max(size);
            }
        }
    }

    max_size
}

/// Checks if a paragraph consists entirely of deleted runs.
pub fn paragraph_is_entirely_deleted(paragraph: &Paragraph) -> bool {
    use crate::document::RevisionKind;
    
    let mut has_deleted_content = false;

    for run in &paragraph.runs {
        let is_deleted = run
            .revision
            .as_ref()
            .map(|mark| matches!(mark.kind, RevisionKind::Deleted | RevisionKind::MovedFrom))
            .unwrap_or(false);

        if run_has_visible_payload(run) {
            if is_deleted {
                has_deleted_content = true;
            } else {
                return false;
            }
        }
    }

    has_deleted_content
}

/// Checks if a run has visible content (text, image, text box, etc.).
fn run_has_visible_payload(run: &crate::document::Run) -> bool {
    if run.image.is_some() || run.text_box.is_some() || run.math.is_some() {
        return true;
    }

    if run.field.is_some() {
        return true;
    }

    // Page break character
    const PAGE_BREAK_CHAR: char = '\u{000C}';
    if run.text.contains(PAGE_BREAK_CHAR) {
        return true;
    }

    run.text.chars().any(|ch| !ch.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Indent, Numbering, ParagraphProperties, Run, Spacing};

    #[test]
    fn test_extract_paragraph_style_default() {
        let style = extract_paragraph_style(None);
        assert!(style.alignment.is_empty());
        assert_eq!(style.first_line_indent, 0.0);
        assert_eq!(style.right_indent, 0.0);
        assert!(!style.has_numbering);
    }

    #[test]
    fn test_extract_paragraph_style_alignment() {
        let props = ParagraphProperties {
            alignment: Some("center".to_string()),
            ..Default::default()
        };
        let style = extract_paragraph_style(Some(&props));
        assert_eq!(style.alignment, "center");
    }

    #[test]
    fn test_extract_paragraph_style_indent() {
        let props = ParagraphProperties {
            indent: Some(Indent {
                left: Some(36.0), // 0.5 inch = 36pt
                first_line: Some(18.0),
                right: Some(18.0),
                hanging: None,
            }),
            ..Default::default()
        };
        let style = extract_paragraph_style(Some(&props));
        
        // 36pt = 48px (36 * 96 / 72)
        assert!((style.left_indent_px - 48.0).abs() < 0.001);
        // 18pt = 24px
        assert!((style.first_line_indent - 24.0).abs() < 0.001);
        assert!((style.right_indent - 24.0).abs() < 0.001);
    }

    #[test]
    fn test_extract_paragraph_style_hanging_indent() {
        let props = ParagraphProperties {
            indent: Some(Indent {
                left: Some(36.0),
                first_line: None,
                right: None,
                hanging: Some(18.0),
            }),
            ..Default::default()
        };
        let style = extract_paragraph_style(Some(&props));
        
        // Hanging indent should be negative first-line indent
        assert!((style.first_line_indent - (-24.0)).abs() < 0.001);
    }

    #[test]
    fn test_extract_paragraph_style_hanging_with_numbering() {
        let props = ParagraphProperties {
            indent: Some(Indent {
                left: Some(36.0),
                first_line: None,
                right: None,
                hanging: Some(18.0),
            }),
            numbering: Some(Numbering {
                id: Some("1".to_string()),
                level: Some(0),
            }),
            ..Default::default()
        };
        let style = extract_paragraph_style(Some(&props));
        
        // With numbering, hanging indent should NOT be applied as negative first-line
        assert_eq!(style.first_line_indent, 0.0);
        assert!(style.has_numbering);
    }

    #[test]
    fn test_extract_paragraph_style_spacing() {
        let props = ParagraphProperties {
            spacing: Some(Spacing {
                before: Some(12.0),
                after: Some(6.0),
                before_lines: None,
                after_lines: None,
                line: None,
                line_rule: None,
            }),
            ..Default::default()
        };
        let style = extract_paragraph_style(Some(&props));
        
        assert_eq!(style.spacing_before, Some(12.0));
        assert_eq!(style.spacing_after, Some(6.0));
    }

    #[test]
    fn test_estimate_paragraph_font_size_default() {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "Hello".to_string(),
                properties: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(estimate_paragraph_font_size(&paragraph), 12.0);
    }

    #[test]
    fn test_estimate_paragraph_font_size_max() {
        use crate::document::RunProperties;
        
        let paragraph = Paragraph {
            runs: vec![
                Run {
                    text: "Small".to_string(),
                    properties: Some(RunProperties {
                        font_size: Some(10.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Run {
                    text: "Large".to_string(),
                    properties: Some(RunProperties {
                        font_size: Some(24.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(estimate_paragraph_font_size(&paragraph), 24.0);
    }

    #[test]
    fn test_points_to_px() {
        // 72pt = 1 inch = 96px at standard DPI
        assert!((points_to_px(72.0) - 96.0).abs() < 0.001);
        // 12pt = 16px
        assert!((points_to_px(12.0) - 16.0).abs() < 0.001);
    }

    #[test]
    fn test_paragraph_is_entirely_deleted_empty() {
        let paragraph = Paragraph::default();
        assert!(!paragraph_is_entirely_deleted(&paragraph));
    }

    #[test]
    fn test_paragraph_is_entirely_deleted_no_revision() {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "Normal text".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!paragraph_is_entirely_deleted(&paragraph));
    }
}
