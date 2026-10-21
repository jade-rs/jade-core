/// Default page geometry used when section settings omit an attribute.
pub const DEFAULT_PAGE_WIDTH_PT: f32 = 595.0;
pub const DEFAULT_PAGE_HEIGHT_PT: f32 = 842.0;
pub const DEFAULT_MARGIN_PT: f32 = 72.0;

/// Word section break behavior from `w:sectPr/w:type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionBreakType {
    Continuous,
    NextColumn,
    NextPage,
    EvenPage,
    OddPage,
}

/// Layout action at a Word section boundary.
///
/// This is intentionally independent of renderer coordinates and canvas state:
/// `jade-docx-layout` decides *which flow container comes next*, while the renderer
/// only materializes that decision using its current page and column geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionTransition {
    /// Continue at the current vertical position. Used by a same-geometry
    /// `continuous` section break.
    Stay,
    /// Restart at the top of the next column on the current page.
    NextColumn,
    /// Advance one or more physical pages, restarting in column one. Values
    /// greater than one represent Word's inserted odd/even parity page.
    NextPage { page_advances: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedPageMargins {
    pub left: f32,
    pub right: f32,
    pub top: f32,
}

/// Resolves Word's logical facing-page margins into physical page edges.
pub fn resolve_page_margins(
    margin_left_pt: f32,
    margin_right_pt: f32,
    margin_top_pt: f32,
    gutter_pt: f32,
    gutter_at_top: bool,
    mirror_margins: bool,
    page_number: usize,
) -> ResolvedPageMargins {
    let (mut left, top) =
        apply_page_gutter(margin_left_pt, margin_top_pt, gutter_pt, gutter_at_top);
    let mut right = margin_right_pt;
    if mirror_margins && page_number.is_multiple_of(2) {
        std::mem::swap(&mut left, &mut right);
    }
    ResolvedPageMargins { left, right, top }
}

/// The page a sheet carries when it is printed two pages to a sheet
/// (`w:printTwoOnOne`, Word's 袋とじ) or folded into a booklet
/// (`w:bookFoldPrinting`, 本 (縦方向に谷折り)): half the sheet. A book fold
/// halves the long side, the way the sheet folds; two pages to a sheet halve
/// the sheet along its longer side too, so an upright sheet carries two pages
/// one above the other.
pub fn folded_page_size(
    width_pt: f32,
    height_pt: f32,
    print_two_on_one: bool,
    book_fold: bool,
) -> (f32, f32) {
    if book_fold {
        let (short, long) = (width_pt.min(height_pt), width_pt.max(height_pt));
        return (long / 2.0, short);
    }
    if print_two_on_one {
        return if width_pt > height_pt {
            (width_pt / 2.0, height_pt)
        } else {
            (width_pt, height_pt / 2.0)
        };
    }
    (width_pt, height_pt)
}

/// Applies a section gutter to the edge selected by `w:gutterAtTop`.
pub fn apply_page_gutter(
    margin_left_pt: f32,
    margin_top_pt: f32,
    gutter_pt: f32,
    gutter_at_top: bool,
) -> (f32, f32) {
    if gutter_at_top {
        (margin_left_pt, margin_top_pt + gutter_pt)
    } else {
        (margin_left_pt + gutter_pt, margin_top_pt)
    }
}

impl SectionBreakType {
    pub fn from_word_value(value: Option<&str>) -> Self {
        match value {
            Some("continuous") => Self::Continuous,
            Some("nextColumn") => Self::NextColumn,
            Some("evenPage") => Self::EvenPage,
            Some("oddPage") => Self::OddPage,
            // ECMA-376 defaults an omitted or unknown section type to nextPage.
            Some("nextPage") | None | Some(_) => Self::NextPage,
        }
    }
}

/// Resolve the next flow container before laying out a section.
///
/// `current_page` is one-based. `has_next_column` describes the section being
/// left, because its final column determines whether `nextColumn` can remain on
/// the current physical page. A page-geometry change wins over a column break:
/// Word cannot apply two page sizes or margin boxes to one physical page.
pub fn resolve_section_transition(
    section_type: SectionBreakType,
    current_page: usize,
    page_geometry_changed: bool,
    has_next_column: bool,
) -> SectionTransition {
    match section_type {
        SectionBreakType::Continuous if !page_geometry_changed => SectionTransition::Stay,
        SectionBreakType::NextColumn if !page_geometry_changed && has_next_column => {
            SectionTransition::NextColumn
        }
        SectionBreakType::Continuous
        | SectionBreakType::NextColumn
        | SectionBreakType::NextPage => SectionTransition::NextPage { page_advances: 1 },
        SectionBreakType::OddPage => {
            let next_page = current_page.saturating_add(1);
            let page_advances = if !next_page.is_multiple_of(2) { 1 } else { 2 };
            SectionTransition::NextPage { page_advances }
        }
        SectionBreakType::EvenPage => {
            let next_page = current_page.saturating_add(1);
            let page_advances = if next_page.is_multiple_of(2) { 1 } else { 2 };
            SectionTransition::NextPage { page_advances }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folded_sheet_carries_half_pages() {
        // A4 landscape folded into a booklet: A5 upright pages.
        assert_eq!(folded_page_size(842.0, 595.0, false, true), (421.0, 595.0));
        // A4 upright, two pages to a sheet: two A5 landscape halves.
        assert_eq!(folded_page_size(595.0, 842.0, true, false), (595.0, 421.0));
        assert_eq!(folded_page_size(595.0, 842.0, false, false), (595.0, 842.0));
    }

    #[test]
    fn omitted_section_type_defaults_to_next_page() {
        assert_eq!(
            SectionBreakType::from_word_value(None),
            SectionBreakType::NextPage
        );
        assert_eq!(
            resolve_section_transition(SectionBreakType::from_word_value(None), 1, false, false),
            SectionTransition::NextPage { page_advances: 1 }
        );
    }

    #[test]
    fn continuous_only_advances_when_page_geometry_changes() {
        assert_eq!(
            resolve_section_transition(SectionBreakType::Continuous, 1, false, true),
            SectionTransition::Stay
        );
        assert_eq!(
            resolve_section_transition(SectionBreakType::Continuous, 1, true, true),
            SectionTransition::NextPage { page_advances: 1 }
        );
    }

    #[test]
    fn next_column_uses_the_remaining_column_then_rolls_to_a_page() {
        // Section content ending in column one of a two-column page continues
        // at the top of column two without manufacturing a page break.
        assert_eq!(
            resolve_section_transition(SectionBreakType::NextColumn, 1, false, true),
            SectionTransition::NextColumn
        );

        // The same break at the last column has no on-page container left, so
        // Word advances to column one of the following page.
        assert_eq!(
            resolve_section_transition(SectionBreakType::NextColumn, 1, false, false),
            SectionTransition::NextPage { page_advances: 1 }
        );
    }

    #[test]
    fn next_column_with_new_page_geometry_starts_a_new_page() {
        // Even if an old-layout column remains, the next section's page box
        // cannot begin midway through the current physical page.
        assert_eq!(
            resolve_section_transition(SectionBreakType::NextColumn, 1, true, true),
            SectionTransition::NextPage { page_advances: 1 }
        );
    }

    #[test]
    fn odd_and_even_breaks_insert_a_parity_page_when_needed() {
        assert_eq!(
            resolve_section_transition(SectionBreakType::OddPage, 1, false, false),
            SectionTransition::NextPage { page_advances: 2 }
        );
        assert_eq!(
            resolve_section_transition(SectionBreakType::OddPage, 2, false, false),
            SectionTransition::NextPage { page_advances: 1 }
        );
        assert_eq!(
            resolve_section_transition(SectionBreakType::EvenPage, 1, false, false),
            SectionTransition::NextPage { page_advances: 1 }
        );
        assert_eq!(
            resolve_section_transition(SectionBreakType::EvenPage, 2, false, false),
            SectionTransition::NextPage { page_advances: 2 }
        );
    }

    #[test]
    fn page_gutter_uses_top_edge_when_gutter_at_top_is_enabled() {
        assert_eq!(apply_page_gutter(72.0, 72.0, 18.0, false), (90.0, 72.0));
        assert_eq!(apply_page_gutter(72.0, 72.0, 18.0, true), (72.0, 90.0));
    }

    #[test]
    fn mirrored_page_margins_put_the_gutter_on_the_inside_edge() {
        let odd = resolve_page_margins(72.0, 54.0, 72.0, 18.0, false, true, 1);
        assert_eq!((odd.left, odd.right, odd.top), (90.0, 54.0, 72.0));

        let even = resolve_page_margins(72.0, 54.0, 72.0, 18.0, false, true, 2);
        assert_eq!((even.left, even.right, even.top), (54.0, 90.0, 72.0));

        let top_bound_even = resolve_page_margins(72.0, 54.0, 72.0, 18.0, true, true, 2);
        assert_eq!(
            (
                top_bound_even.left,
                top_bound_even.right,
                top_bound_even.top
            ),
            (54.0, 72.0, 90.0)
        );
    }
}
