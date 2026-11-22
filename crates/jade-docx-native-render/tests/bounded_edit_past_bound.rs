//! A keystroke bounded to the caret's page that gives a paragraph another
//! line moves everything after it, past the bound too. The pages there are
//! left stale rather than showing the old layout, caret queries past the
//! bound answer from the new one, and drawing the stale pages gives what a
//! fresh open of the edited document draws.

mod common;

use common::{open_bytes, open_fixture, Doc};

/// The first paragraph with some text on `page` (0-based).
fn paragraph_on_page(document: &mut Doc, page: usize) -> Option<usize> {
    (0..document.paragraph_count()).find(|&paragraph| {
        document
            .paragraph_text(paragraph)
            .is_some_and(|text| text.chars().count() > 4)
            && document
                .position_rect(paragraph, 0)
                .is_some_and(|rect| document.page_and_offset(rect.document_y).0 == page)
    })
}

/// Types into a page-1 paragraph, bounded to page 1, until the paragraph
/// after it moves down a line. Returns the paragraph typed into.
fn type_until_it_wraps(document: &mut Doc) -> usize {
    let paragraph = paragraph_on_page(document, 0).expect("a paragraph on page 1");
    let next = paragraph + 1;
    let next_before = document.position_rect(next, 0).unwrap().document_y;
    let mut typed = 0;
    while document.position_rect(next, 0).unwrap().document_y == next_before {
        document.insert_text(paragraph, 0, "あ", 1).unwrap();
        typed += 1;
        assert!(typed < 400, "never wrapped");
    }
    paragraph
}

/// Caret rectangle and hit-test answer at the start of each paragraph, in
/// hundredths of a pixel.
type Geometry = Vec<(usize, Option<(i64, i64)>, Option<(usize, usize)>)>;

fn geometry(document: &mut Doc, paragraphs: &[usize]) -> Geometry {
    paragraphs
        .iter()
        .map(|&paragraph| {
            let rect = document.position_rect(paragraph, 0);
            let hit = rect.and_then(|rect| {
                document.position_at(rect.x + 1.0, rect.document_y + rect.height / 2.0)
            });
            (
                paragraph,
                rect.map(|rect| ((rect.x * 100.0) as i64, (rect.document_y * 100.0) as i64)),
                hit,
            )
        })
        .collect()
}

/// A few paragraphs on every page of `document`.
fn sample_paragraphs(document: &mut Doc) -> Vec<usize> {
    let count = document.paragraph_count();
    let step = (count / 60).max(1);
    (0..count).step_by(step).collect()
}

fn check(fixture: &str) {
    let mut document = open_fixture(fixture);
    let pages_before = document.page_count();
    assert!(pages_before >= 3, "{fixture}: {pages_before} pages");
    type_until_it_wraps(&mut document);

    let fresh = open_bytes(&document.export_docx().unwrap());
    // Every page after the bound is either marked stale or already right.
    for page in 1..document.page_count() {
        assert!(
            document.is_page_stale(page) || document.page_pixels(page) == fresh.page_pixels(page),
            "{fixture}: page {page} shows the old layout without being stale"
        );
    }
    assert!(
        document.has_stale_pages(),
        "{fixture}: the later pages moved"
    );
    assert_eq!(
        document.page_pixels(0),
        fresh.page_pixels(0),
        "{fixture}: page 1"
    );

    // Caret queries past the bound answer from the new layout before any
    // stale page is drawn.
    let mut fresh = fresh;
    let paragraphs = sample_paragraphs(&mut fresh);
    assert_eq!(
        geometry(&mut document, &paragraphs),
        geometry(&mut fresh, &paragraphs),
        "{fixture}: geometry before the stale pages are drawn"
    );

    assert!(document.paint_stale_pages(0..usize::MAX).unwrap());
    assert!(!document.has_stale_pages());
    assert_eq!(document.page_count(), fresh.page_count(), "{fixture}");
    for page in 0..fresh.page_count() {
        assert_eq!(
            document.page_pixels(page),
            fresh.page_pixels(page),
            "{fixture}: page {page}"
        );
    }
    assert_eq!(
        geometry(&mut document, &paragraphs),
        geometry(&mut fresh, &paragraphs),
        "{fixture}: geometry after the stale pages are drawn"
    );
}

#[test]
fn a_wrapping_keystroke_on_page_one_leaves_yokohama_right() {
    check("public/fixtures/yokohama.docx");
}

#[test]
fn a_wrapping_keystroke_on_page_one_leaves_contract_right() {
    check("public/fixtures/contract.docx");
}

#[test]
fn a_wrapping_keystroke_on_page_one_leaves_nda_right() {
    check("public/fixtures/nda.docx");
}

/// A keystroke whose paragraph keeps its height leaves the later pages as
/// they are: nothing to redraw, no layout past the bound.
#[test]
fn a_keystroke_that_keeps_the_height_leaves_no_page_stale() {
    let mut document = open_fixture("public/fixtures/yokohama.docx");
    let paragraph = paragraph_on_page(&mut document, 0).unwrap();
    let next = paragraph + 1;
    let next_before = document.position_rect(next, 0).unwrap().document_y;
    document.insert_text(paragraph, 0, "a", 1).unwrap();
    assert_eq!(
        document.position_rect(next, 0).unwrap().document_y,
        next_before,
        "one character does not wrap"
    );
    assert!(!document.has_stale_pages());
}
