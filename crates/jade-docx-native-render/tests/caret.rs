//! Can the caret be placed and located, natively?
//!
//! The shell draws the caret itself rather than having the renderer paint it,
//! so what matters is that a position maps to a rectangle that moves with it.
//! Checked before any drawing is built on top — the last three bugs in this
//! crate all looked like "the feature does not work" and were coordinates.

mod common;
use common::open_contract;

#[test]
fn a_position_maps_to_a_caret_rectangle() {
    let mut document = open_contract();

    let caret = document.place_caret(6, 0).expect("caret for paragraph 6");

    assert!(caret.height > 0.0, "zero-height caret: {caret:?}");
    assert!(caret.width > 0.0, "zero-width caret: {caret:?}");
    assert!(
        caret.x > 0.0 && caret.document_y > 0.0,
        "caret at the origin, which is outside the text: {caret:?}"
    );
}

#[test]
fn later_offsets_move_the_caret_right() {
    let mut document = open_contract();

    let start = document.place_caret(6, 0).expect("caret at the start");
    let later = document.place_caret(6, 9).expect("caret further in");

    assert_eq!(
        start.document_y, later.document_y,
        "same line, so the caret should not have changed row: {start:?} {later:?}"
    );
    assert!(
        later.x > start.x,
        "the caret did not advance along the line: {start:?} -> {later:?}"
    );
}

#[test]
fn later_paragraphs_move_the_caret_down() {
    let mut document = open_contract();

    let earlier = document.place_caret(3, 0).expect("caret in paragraph 3");
    let later = document.place_caret(10, 0).expect("caret in paragraph 10");

    assert!(
        later.document_y > earlier.document_y,
        "the caret did not move down the document: {earlier:?} -> {later:?}"
    );
}

/// The two halves have to agree, or clicking puts the caret somewhere the
/// click was not.
#[test]
fn clicking_where_the_caret_is_returns_the_same_position() {
    let mut document = open_contract();

    let caret = document.place_caret(6, 0).expect("caret for paragraph 6");
    // A hair to the right of the caret and vertically inside its line.
    let probe_x = caret.x + 2.0;
    let probe_y = caret.document_y + caret.height / 2.0;

    let hit = document.position_at(probe_x, probe_y).expect("a position");
    assert_eq!(
        hit.0, 6,
        "hit testing the caret's own position landed in another paragraph: {hit:?}"
    );
}

/// The screen-to-document mapping has to hold on *every* page.
///
/// Pages are stacked with a gap between them, so a mapping that uses the page
/// height alone is exactly right on page 1 and drifts by one gap per page after
/// it — which reads as "page 1 works, later pages are offset".
#[test]
fn the_page_mapping_round_trips_on_later_pages() {
    let mut document = open_contract();

    // A paragraph on each of the first three pages, found by asking where the
    // caret lands rather than assuming.
    let mut per_page: Vec<(usize, usize)> = Vec::new();
    for paragraph in 0..60 {
        if let Some(caret) = document.place_caret(paragraph, 0) {
            let (page, _) = document.page_and_offset(caret.document_y);
            if !per_page.iter().any(|(p, _)| *p == page) {
                per_page.push((page, paragraph));
            }
        }
    }
    assert!(
        per_page.len() >= 3,
        "expected paragraphs on at least three pages, found {per_page:?}"
    );

    for (page, paragraph) in per_page.into_iter().take(3) {
        let caret = document.place_caret(paragraph, 0).expect("caret");
        let (mapped_page, y_on_page) = document.page_and_offset(caret.document_y);
        assert_eq!(mapped_page, page, "page changed under the same caret");

        // Back the other way, as a click would: page-local to document.
        let document_y = document.document_y(mapped_page, y_on_page);
        let hit = document
            .position_at(caret.x + 2.0, document_y + caret.height / 2.0)
            .expect("a position under the caret");
        assert_eq!(
            hit.0, paragraph,
            "page {page}: a click at the caret resolved to another paragraph"
        );
    }
}
