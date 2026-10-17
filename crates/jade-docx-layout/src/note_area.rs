//! Footnote area pagination decisions (#294).
//!
//! Word keeps the footnotes referenced on a page in an area at the bottom of
//! that page's body: body text stops above the area, and a reference line
//! only stays on a page when at least the start of its note fits below it.
//! A note too long for the space left continues in the next page's area,
//! below the `w:continuationSeparator` story, and the page it breaks on can
//! end with the `w:continuationNotice` story.
//!
//! The renderer lays the notes and the body out; these functions only make
//! the pagination decisions from measured heights, so the rules live with the
//! rest of the layout engine and can be tested without a canvas.

/// Tolerance for comparing accumulated pixel heights.
const EPSILON: f64 = 0.5;

/// Height of one footnote text line used to decide whether the start of a
/// note fits below its reference: Word's `footnote text` style is 10pt with
/// the 1.15 single-spacing multiplier.
pub const MIN_NOTE_LINE_PT: f64 = 10.0 * 1.15;

/// How much of the page bottom a page's footnote area takes once a reference
/// line is about to be placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteAreaReserve {
    /// Every pending note fits below the reference line: reserve their full
    /// height.
    Fits(f64),
    /// The notes do not fit completely, but the earlier ones and the start of
    /// the newest do: the area takes the rest of the page and the newest note
    /// continues on the next page.
    Split(f64),
    /// Not even the start of the newest note fits below its reference line:
    /// the line moves to the next page with its note.
    MoveReference,
}

/// Decides the area for the notes pending on the current page.
///
/// * `full_height` — separator plus every pending note, measured in full.
/// * `committed_height` — the area already reserved before this reference
///   (earlier notes on the page, or a continuation carried onto it).
/// * `space_left` — distance from the layout cursor to the bottom of the body
///   area, ignoring any reservation.
/// * `allowance` — height of what must still fit above the area: the
///   reference line (or table row) being placed.
/// * `min_note_line` — height of the first line of a note.
/// * `separator_height` — height of the separator above the area.
/// * `at_page_top` — nothing has been placed on the page yet, so moving the
///   reference could not make progress; the note is split instead.
pub fn reserve_note_area(
    full_height: f64,
    committed_height: f64,
    space_left: f64,
    allowance: f64,
    min_note_line: f64,
    separator_height: f64,
    at_page_top: bool,
) -> NoteAreaReserve {
    if full_height <= 0.0 {
        return NoteAreaReserve::Fits(0.0);
    }
    let room = space_left - allowance;
    if full_height <= room + EPSILON {
        return NoteAreaReserve::Fits(full_height);
    }
    let minimum = committed_height.max(separator_height) + min_note_line;
    if minimum <= room + EPSILON {
        return NoteAreaReserve::Split(room.max(minimum).min(full_height));
    }
    if at_page_top {
        // Keep at least the start of the note with its reference even when
        // that leaves the reference line itself no room on an empty page.
        return NoteAreaReserve::Split(minimum.min(full_height));
    }
    NoteAreaReserve::MoveReference
}

/// Top of a page's footnote area: flush with the bottom of the body area when
/// the notes fit under the body text, otherwise right below the text plus
/// `crowded_gap` (the notes then continue on the next page).
pub fn note_area_top(body_bottom: f64, area_height: f64, body_end: f64, crowded_gap: f64) -> f64 {
    let flush = body_bottom - area_height;
    if flush + EPSILON >= body_end {
        flush.max(body_end)
    } else {
        body_end + crowded_gap
    }
}

/// Height of the complete prefix of a measured note that fits in an area.
/// `line_bottoms` are cumulative advances from shared paragraph layout, including
/// preceding paragraph spacing. An empty plan cannot split an oversized note.
pub fn fitting_note_prefix_height(
    height: f64,
    line_bottoms: &[f64],
    available: f64,
) -> Option<f64> {
    if height <= available + EPSILON {
        return Some(height);
    }
    if line_bottoms.is_empty() {
        return None;
    }
    let end = line_bottoms.partition_point(|bottom| *bottom <= available + EPSILON);
    Some(end.checked_sub(1).map_or(0.0, |index| line_bottoms[index]))
}

/// Whether a note line of `line_height` starting at `cursor` overflows a
/// footnote area ending at `area_bottom`. The first line placed in an area
/// always stays, so a note continued over several pages makes progress on
/// every page even when the area is shorter than one line.
pub fn note_line_overflows(
    cursor: f64,
    line_height: f64,
    area_bottom: f64,
    lines_placed: usize,
) -> bool {
    lines_placed > 0 && cursor + line_height > area_bottom + EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f64 = 15.0;
    const SEPARATOR: f64 = 16.0;

    #[test]
    fn note_fragments_use_complete_measured_lines_including_spacing() {
        let lines = [20.0, 40.0, 65.0, 85.0];
        assert_eq!(fitting_note_prefix_height(85.0, &lines, 64.0), Some(40.0));
        assert_eq!(fitting_note_prefix_height(85.0, &lines, 65.0), Some(65.0));
        assert_eq!(fitting_note_prefix_height(85.0, &lines, 19.0), Some(0.0));
        assert_eq!(fitting_note_prefix_height(85.0, &lines, 85.0), Some(85.0));
        assert_eq!(fitting_note_prefix_height(85.0, &[], 65.0), None);
        assert_eq!(fitting_note_prefix_height(85.0, &[], 85.0), Some(85.0));
    }

    #[test]
    fn notes_that_fit_reserve_their_full_height() {
        assert_eq!(
            reserve_note_area(60.0, 0.0, 400.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::Fits(60.0)
        );
        assert_eq!(
            reserve_note_area(0.0, 0.0, 10.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::Fits(0.0)
        );
    }

    #[test]
    fn a_long_note_splits_when_its_start_fits_below_the_reference() {
        // 100px left on the page, a 20px reference line, a 600px note.
        assert_eq!(
            reserve_note_area(600.0, 0.0, 100.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::Split(80.0)
        );
        // Earlier notes on the page keep their full height.
        assert_eq!(
            reserve_note_area(600.0, 50.0, 100.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::Split(80.0)
        );
    }

    #[test]
    fn a_reference_without_room_for_its_note_moves_to_the_next_page() {
        // 30px left: the 20px reference line leaves no room for the separator
        // and a first note line.
        assert_eq!(
            reserve_note_area(60.0, 0.0, 30.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::MoveReference
        );
        // Earlier notes fill the room: the newest reference moves.
        assert_eq!(
            reserve_note_area(200.0, 90.0, 120.0, 20.0, LINE, SEPARATOR, false),
            NoteAreaReserve::MoveReference
        );
        // ...unless the page is still empty, where moving cannot help.
        assert_eq!(
            reserve_note_area(2000.0, 0.0, 10.0, 20.0, LINE, SEPARATOR, true),
            NoteAreaReserve::Split(SEPARATOR + LINE)
        );
    }

    #[test]
    fn the_area_sits_at_the_page_bottom_unless_the_body_overruns_it() {
        assert_eq!(note_area_top(900.0, 100.0, 700.0, 12.0), 800.0);
        assert_eq!(note_area_top(900.0, 100.0, 800.0, 12.0), 800.0);
        assert_eq!(note_area_top(900.0, 100.0, 850.0, 12.0), 862.0);
    }

    #[test]
    fn the_first_line_of_an_area_always_stays() {
        assert!(!note_line_overflows(890.0, 20.0, 900.0, 0));
        assert!(note_line_overflows(890.0, 20.0, 900.0, 1));
        assert!(!note_line_overflows(880.0, 20.0, 900.0, 3));
    }
}
