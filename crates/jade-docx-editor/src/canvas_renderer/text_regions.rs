//! Where a run of text was drawn: a rectangle a line, in the document's
//! coordinates, from the last layout. A content control's box, a merge
//! field's shading and a form field's grey are drawn from these.

use super::*;

/// The part of one line a run of text covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRegion {
    /// 0-based page.
    pub page: usize,
    pub paragraph: usize,
    /// In the document's coordinates: the pages stacked with their gaps.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl<HostPainter> CanvasRenderer<HostPainter>
where
    HostPainter: Painter,
    HostPainter::Error: PainterError,
{
    /// The rectangles the text from `start` to `end` (rendered positions)
    /// was laid out in, one a line. An empty range gives a zero-width
    /// rectangle where it is, so an empty content control still has a box.
    pub fn text_range_regions(&self, start: TextPosition, end: TextPosition) -> Vec<TextRegion> {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let page_with_gap = self.page_height + PAGE_GAP_PX;
        let page_of = |top: f64| {
            if self.pageless_mode || page_with_gap <= 0.0 {
                0
            } else {
                let value = (top / page_with_gap).floor();
                if value.is_finite() && value >= 0.0 {
                    value as usize
                } else {
                    0
                }
            }
        };
        let paragraphs = self.selection.paragraphs();
        let mut regions = Vec::new();
        for paragraph_index in start.paragraph..=end.paragraph {
            let Some(layout) = paragraphs.get(paragraph_index) else {
                continue;
            };
            let from = if paragraph_index == start.paragraph {
                start.offset
            } else {
                0
            };
            let to = if paragraph_index == end.paragraph {
                end.offset
            } else {
                usize::MAX
            };
            for line in &layout.lines {
                if line.vertical {
                    continue;
                }
                let mut left: Option<f64> = None;
                let mut right: Option<f64> = None;
                for fragment in line.fragments.iter() {
                    let (a, b) = (from.max(fragment.start), to.min(fragment.end));
                    let touches =
                        a < b || (from == to && fragment.start <= from && from <= fragment.end);
                    if !touches {
                        continue;
                    }
                    let Some(x0) = Self::fragment_x_at_offset(fragment, a) else {
                        continue;
                    };
                    let x1 = Self::fragment_x_at_offset(fragment, b.max(a)).unwrap_or(x0);
                    left = Some(left.map_or(x0.min(x1), |left: f64| left.min(x0.min(x1))));
                    right = Some(right.map_or(x0.max(x1), |right: f64| right.max(x0.max(x1))));
                }
                // An empty line inside the range (an empty paragraph of a
                // block control) still shows where it is.
                let empty_inside = line.fragments.is_empty() && from == 0 && to == usize::MAX;
                let (left, right) = match (left, right) {
                    (Some(left), Some(right)) => (left, right),
                    _ if empty_inside || (line.fragments.is_empty() && from == to) => {
                        (line.start_x, line.start_x)
                    }
                    _ => continue,
                };
                regions.push(TextRegion {
                    page: page_of(line.absolute_content_top),
                    paragraph: paragraph_index,
                    x: left,
                    y: line.absolute_content_top,
                    width: (right - left).max(0.0),
                    height: line.content_height.max(1.0),
                });
            }
        }
        regions
    }
}
