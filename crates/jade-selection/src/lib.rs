//! Renderer-independent logical positions used by DOCX editing operations.
//!
//! This crate deliberately contains no layout or rendering code. Both the
//! headless document engine and visual layout layer use these types so callers
//! can move selections between them without conversion.

use serde::{Deserialize, Serialize};

/// Compatibility module for callers that historically imported these types
/// from `jade_docx_layout::selection`.
pub mod selection {
    pub use super::{SelectionRange, TextPosition};
}

/// A UTF-8 byte offset within a paragraph in document order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TextPosition {
    /// Zero-based paragraph index in the document's flattened paragraph order.
    pub paragraph: usize,
    /// Zero-based UTF-8 byte offset within the paragraph text.
    pub offset: usize,
}

impl TextPosition {
    /// Creates a logical text position.
    #[must_use]
    pub const fn new(paragraph: usize, offset: usize) -> Self {
        Self { paragraph, offset }
    }
}

/// A directional text selection represented by anchor and focus positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRange {
    /// The fixed end of the selection.
    pub anchor: TextPosition,
    /// The moving/caret end of the selection.
    pub focus: TextPosition,
}

impl SelectionRange {
    /// Creates a directional range without reordering its endpoints.
    #[must_use]
    pub const fn new(anchor: TextPosition, focus: TextPosition) -> Self {
        Self { anchor, focus }
    }

    /// Creates a collapsed selection (caret) at `position`.
    #[must_use]
    pub const fn collapsed(position: TextPosition) -> Self {
        Self {
            anchor: position,
            focus: position,
        }
    }

    /// Returns whether the anchor and focus refer to the same position.
    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// Returns endpoints ordered by document position.
    #[must_use]
    pub fn normalized(&self) -> (TextPosition, TextPosition) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SelectionRange, TextPosition};

    #[test]
    fn normalizes_backward_selection() {
        let start = TextPosition::new(1, 2);
        let end = TextPosition::new(3, 4);
        assert_eq!(SelectionRange::new(end, start).normalized(), (start, end));
    }
}

#[cfg(kani)]
mod verification {
    use super::{SelectionRange, TextPosition};

    #[kani::proof]
    fn normalized_orders_both_endpoints() {
        let anchor = TextPosition::new(kani::any(), kani::any());
        let focus = TextPosition::new(kani::any(), kani::any());
        let range = SelectionRange::new(anchor, focus);
        let (start, end) = range.normalized();

        assert!(start <= end);
        assert!((start == anchor && end == focus) || (start == focus && end == anchor));
    }

    #[kani::proof]
    fn collapsed_matches_equal_endpoints() {
        let anchor = TextPosition::new(kani::any(), kani::any());
        let focus = TextPosition::new(kani::any(), kani::any());
        let range = SelectionRange::new(anchor, focus);

        assert_eq!(range.is_collapsed(), anchor == focus);
    }
}
