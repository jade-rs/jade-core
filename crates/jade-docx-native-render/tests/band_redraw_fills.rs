//! A band redraw leaves fills as a full repaint draws them.
//!
//! The band path clears and repaints only the edited paragraph's slice of its
//! page, under a clip. Glyphs come out exactly under that clip; fills do not:
//! a neighbour's shading the clear reaches is lost when the neighbour is not
//! repainted, a neighbour's highlight the clip cuts has its antialiased
//! corners rasterised from the clipped piece, and an erasure the edited line
//! paints past its band is cut short. Each class is one fixture here, typed
//! into and deleted from on page one, against the same edits followed by a
//! full repaint.

mod common;

use common::{open_fixture, Doc};

fn first_difference(actual: &Doc, reference: &Doc, page: usize) -> Option<(usize, usize)> {
    let (actual, width, _) = actual.page_pixels(page).expect("actual page");
    let (expected, _, _) = reference.page_pixels(page).expect("reference page");
    assert_eq!(actual.len(), expected.len(), "page {page} size");
    actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .position(|(a, b)| a != b)
        .map(|index| (index % width as usize, index / width as usize))
}

/// The first `count` paragraphs with text whose start is on page one.
fn paragraphs_on_first_page(document: &mut Doc, count: usize) -> Vec<usize> {
    let mut found = Vec::new();
    for paragraph in 0..80 {
        if found.len() == count {
            break;
        }
        let has_text = document
            .paragraph_text(paragraph)
            .is_some_and(|text| text.chars().count() > 4);
        let on_first_page = has_text
            && document
                .place_caret(paragraph, 0)
                .is_some_and(|caret| document.page_and_offset(caret.document_y).0 == 0);
        if on_first_page {
            found.push(paragraph);
        }
    }
    found
}

#[derive(Clone, Copy)]
enum Edit<'a> {
    Insert(&'a str),
    Delete(usize),
}

/// Applies `edits` at the start of `paragraph` on a document bounded to page
/// one (the keystroke path) and on one that repaints in full, comparing page
/// one after each.
fn check_edits(fixture: &str, paragraph: usize, edits: &[Edit]) {
    let path = format!("public/fixtures/{fixture}.docx");
    let mut actual = open_fixture(&path);
    let mut reference = open_fixture(&path);
    for (step, edit) in edits.iter().enumerate() {
        for (document, bound) in [(&mut actual, 1), (&mut reference, usize::MAX)] {
            match *edit {
                Edit::Insert(text) => document.insert_text(paragraph, 0, text, bound),
                Edit::Delete(count) => document.delete_text(paragraph, 0, false, count, bound),
            }
            .expect("edit");
        }
        reference.repaint_all().expect("full repaint");
        assert_eq!(
            first_difference(&actual, &reference, 0),
            None,
            "{fixture}: paragraph {paragraph}, step {step}: page 1 differs from a full repaint",
        );
    }
}

fn check_typing(fixture: &str, paragraphs: usize) {
    let mut document = open_fixture(&format!("public/fixtures/{fixture}.docx"));
    let targets = paragraphs_on_first_page(&mut document, paragraphs);
    assert!(!targets.is_empty(), "{fixture}: no paragraph on page 1");
    for paragraph in targets {
        check_edits(
            fixture,
            paragraph,
            &[Edit::Insert("a"), Edit::Delete(1), Edit::Insert("a")],
        );
    }
}

/// Every paragraph is shaded, so each one's shading meets its neighbours'.
#[test]
fn typing_between_adjacent_shaded_paragraphs_matches_a_full_repaint() {
    check_typing(
        "word-windows-styled-arial-pair-matrix-size12-bold-compat15",
        3,
    );
    check_typing(
        "word-windows-styled-arial-pair-matrix-size24-italic-compat15",
        3,
    );
}

/// The paragraph after the first holds a moved-from run, highlighted from the
/// top of its line, which the first paragraph's band clip reaches.
#[test]
fn typing_above_a_highlighted_revision_matches_a_full_repaint() {
    check_typing("review-kinds", 3);
}

/// Sixty characters make the last line on the page overflow its right edge.
/// The erasure behind the overflow follows the line's clip, which passes the
/// band and covers the corner crop mark below; removing them again brings
/// the crop mark back.
#[test]
fn an_overflowing_line_on_the_band_matches_a_full_repaint() {
    let long = "wrap ".repeat(12);
    check_edits(
        "word-mac-probe-ruby-wide-grid",
        28,
        &[Edit::Insert(&long), Edit::Delete(60), Edit::Insert("a")],
    );
}
