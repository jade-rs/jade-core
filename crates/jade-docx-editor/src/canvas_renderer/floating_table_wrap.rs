//! Windows Word places some floating tables above their logical body
//! position. Measure their page-local frames before laying out the text
//! that precedes them in document order.

use super::*;

#[derive(Clone, Debug, PartialEq)]
struct TableWrapAnchor {
    element: usize,
    page: usize,
    top: f64,
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

    pub(super) fn pinned_top(&self, element: usize, page: usize) -> Option<f64> {
        if self.measuring {
            return None;
        }
        let anchor = self
            .anchors
            .binary_search_by_key(&element, |anchor| anchor.element)
            .ok()
            .map(|index| &self.anchors[index])?;
        (anchor.page == page).then_some(anchor.top)
    }

    pub(super) fn record(
        &mut self,
        element: usize,
        frame: table::FloatingTableFrame,
        bounds: FloatExclusion,
    ) {
        if !self.measuring {
            return;
        }
        self.anchors.push(TableWrapAnchor {
            element,
            page: frame.page_number,
            top: frame.top,
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
                        bottom: anchor.bounds.bottom + shift - points_to_px(0.05),
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

pub(super) fn document_may_wrap_before_floating_tables(document: &Document) -> bool {
    let compatibility = document.east_asian_line_breaking.compatibility_mode;
    document.body.as_ref().is_some_and(|body| {
        body.elements.iter().skip(1).any(|element| {
            matches!(element, BodyElement::Table(table) if table::windows_float_may_wrap_before(table, compatibility))
        })
    })
}
