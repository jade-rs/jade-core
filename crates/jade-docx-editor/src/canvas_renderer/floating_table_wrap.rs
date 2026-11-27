//! Windows Word places some floating tables above their logical body
//! position. Measure their page-local frames before laying out the text
//! that precedes them in document order.

use super::*;

// One twip of contact does not make a preceding line wrap.
const LOWER_EDGE_CONTACT_PT: f32 = 0.05;

#[derive(Clone, Debug, PartialEq)]
struct TableWrapAnchor {
    element: usize,
    page: usize,
    top: f64,
    displaced_by_overlap: bool,
    bounds: FloatExclusion,
}

#[derive(Clone, Copy)]
struct ParagraphPageAdvance {
    from: usize,
    to: usize,
    first_line_top: f64,
}

pub(super) struct FloatingTableWrap {
    anchors: Vec<TableWrapAnchor>,
    paragraph_pages: FxHashMap<usize, ParagraphPageAdvance>,
    measuring: bool,
}

impl FloatingTableWrap {
    pub(super) fn start_measurement() -> Self {
        Self {
            anchors: Vec::new(),
            paragraph_pages: FxHashMap::default(),
            measuring: true,
        }
    }

    pub(super) fn finish_measurement(&mut self) {
        self.measuring = false;
    }

    pub(super) fn pinned_placement(&self, element: usize, page: usize) -> Option<(f64, bool)> {
        if self.measuring {
            return None;
        }
        let anchor = self
            .anchors
            .binary_search_by_key(&element, |anchor| anchor.element)
            .ok()
            .map(|index| &self.anchors[index])?;
        (anchor.page == page).then_some((anchor.top, anchor.displaced_by_overlap))
    }

    pub(super) fn record(
        &mut self,
        element: usize,
        frame: table::FloatingTableFrame,
        displaced_by_overlap: bool,
        bounds: FloatExclusion,
    ) {
        if !self.measuring {
            return;
        }
        self.anchors.push(TableWrapAnchor {
            element,
            page: frame.page_number,
            top: frame.top,
            displaced_by_overlap,
            bounds,
        });
    }

    pub(super) fn add_exclusions(
        &self,
        element: usize,
        page: usize,
        line_top: f64,
        carry_to_next_page: bool,
        exclusions: &mut Vec<FloatExclusion>,
    ) {
        if !self.may_wrap(element) {
            return;
        }
        // A paragraph moved by widow or keep rules wraps on the page
        // where it places its first line.
        let (wrap_page, wrap_top) = self
            .paragraph_pages
            .get(&element)
            .filter(|advance| carry_to_next_page && advance.from == page)
            .map_or((page, line_top), |advance| {
                (advance.to, advance.first_line_top)
            });
        let shift = line_top - wrap_top;
        exclusions.extend(
            self.anchors
                .iter()
                .filter(|anchor| {
                    anchor.element > element
                        && anchor.page == wrap_page
                        && anchor.bounds.bottom > wrap_top
                })
                .map(|anchor| {
                    // A twip of contact below a table clears the next line
                    // in preceding text, matching the one-twip Word probes.
                    FloatExclusion {
                        top: anchor.bounds.top + shift,
                        bottom: anchor.bounds.bottom + shift - points_to_px(LOWER_EDGE_CONTACT_PT),
                        ..anchor.bounds.clone()
                    }
                }),
        );
    }

    pub(super) fn record_paragraph_page(
        &mut self,
        element: usize,
        from: usize,
        to: usize,
        first_line_top: f64,
    ) {
        if self.measuring && from != to {
            self.paragraph_pages.insert(
                element,
                ParagraphPageAdvance {
                    from,
                    to,
                    first_line_top,
                },
            );
        }
    }

    pub(super) fn may_wrap(&self, element: usize) -> bool {
        !self.measuring
            && self
                .anchors
                .last()
                .is_some_and(|anchor| anchor.element > element)
    }
}

pub(super) fn first_word_between_float_edges(
    text: &str,
    lines: &[WrappedLine],
    style: &ParagraphStyle,
    full_width: f64,
) -> Option<usize> {
    lines.windows(2).enumerate().find_map(|(index, pair)| {
        (style.line_base_width(full_width, index) + f64::EPSILON < full_width
            && pair[0].splits_latin_word_before(text, &pair[1]))
        .then_some(index)
    })
}

pub(super) fn whole_word_blockers(
    mut future: Vec<FloatExclusion>,
    active: &[PageFloatExclusion],
    page: usize,
    content_left: f64,
    content_right: f64,
) -> Vec<FloatExclusion> {
    // The one-twip contact tolerance applies to ordinary wrap bands. Moving
    // a whole word below the table uses its actual lower edge.
    for bounds in &mut future {
        bounds.bottom += points_to_px(LOWER_EDGE_CONTACT_PT);
    }
    future.extend(
        active
            .iter()
            .filter(|entry| entry.is_table && entry.page_number == page)
            .map(|entry| entry.bounds.clone()),
    );
    future.retain(|bounds| bounds.left < content_right && bounds.right > content_left);
    future
}

pub(super) fn word_clearance(
    bounds: &[FloatExclusion],
    line_top: f64,
    line_height: f64,
    round_to_line: Option<f64>,
) -> Option<FloatExclusion> {
    let blocker = bounds
        .iter()
        .filter(|bounds| bounds.top < line_top + line_height && bounds.bottom > line_top)
        .min_by(|left, right| left.bottom.total_cmp(&right.bottom))?;
    Some(FloatExclusion {
        // Keep earlier lines that could use the side of this table.
        top: line_top.max(blocker.top) + 1.0e-6,
        bottom: round_to_line.map_or(blocker.bottom, |origin| {
            let bands = ((blocker.bottom - origin - 1.0e-6) / line_height).ceil();
            origin + bands * line_height
        }),
        left: f64::NEG_INFINITY,
        right: f64::INFINITY,
        ..blocker.clone()
    })
}

pub(super) fn document_may_wrap_before_floating_tables(document: &Document) -> bool {
    let compatibility = document.east_asian_line_breaking.compatibility_mode;
    document.body.as_ref().is_some_and(|body| {
        body.elements.iter().skip(1).any(|element| {
            matches!(element, BodyElement::Table(table) if table::windows_float_may_wrap_before(table, compatibility))
        })
    })
}
