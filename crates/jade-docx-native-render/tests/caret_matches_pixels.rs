//! Does the caret land where the text actually is?
//!
//! The existing mapping test round-trips `document_y` against
//! `page_and_offset`, which are inverses of each other — it passes whatever the
//! page pitch is. This ties the mapping to the rendered pixels instead: the
//! caret's page-local y must have ink beside it on that page's bitmap.

mod common;
use common::{open_contract, Doc};

/// Ink in the horizontal band `y ± tolerance` on `page`.
fn ink_in_band(document: &Doc, page: usize, y: f64, tolerance: f64) -> usize {
    let (pixels, width, _height) = document.page_pixels(page).expect("page pixels");
    let top = (y - tolerance).max(0.0) as usize;
    let bottom = (y + tolerance) as usize;
    let mut inked = 0;
    for row in top..bottom {
        let start = row * width as usize * 4;
        let end = start + width as usize * 4;
        if end > pixels.len() {
            break;
        }
        inked += pixels[start..end]
            .chunks_exact(4)
            .filter(|px| px[0] < 200)
            .count();
    }
    inked
}

#[test]
fn the_caret_sits_on_a_line_of_text_on_every_page() {
    let mut document = open_contract();

    // One paragraph per page, found by asking where the caret lands. Empty
    // paragraphs are skipped: they have a caret and no ink, so they cannot say
    // anything about whether the mapping is right.
    let mut per_page: Vec<(usize, usize)> = Vec::new();
    for paragraph in 0..80 {
        let has_text = document
            .paragraph_text(paragraph)
            .is_some_and(|text| !text.trim().is_empty());
        if !has_text {
            continue;
        }
        if let Some(caret) = document.place_caret(paragraph, 0) {
            let (page, _) = document.page_and_offset(caret.document_y);
            if page < document.page_count() && !per_page.iter().any(|(p, _)| *p == page) {
                per_page.push((page, paragraph));
            }
        }
    }
    assert!(
        per_page.len() >= 3,
        "expected paragraphs on at least two pages, got {per_page:?}"
    );

    for (page, paragraph) in per_page {
        let caret = document.place_caret(paragraph, 0).expect("caret");
        let (mapped_page, y_on_page) = document.page_and_offset(caret.document_y);
        assert_eq!(mapped_page, page);

        // The caret is drawn at this y on this page's bitmap. If the mapping is
        // off, it points at whitespace between lines — or off the text
        // entirely.
        let on_the_line = ink_in_band(&document, page, y_on_page + caret.height / 2.0, 2.0);
        assert!(
            on_the_line > 0,
            "page {}: the caret's row (y={y_on_page:.1}) has no \
             text on it — the screen mapping does not match what was drawn",
            page + 1
        );
    }
}

/// Opening has to paint everything, not most of it.
///
/// A fresh renderer's first render left the first paragraph of every page after
/// the first unpainted — laid out, addressable, with a caret, and simply not
/// drawn. A later full repaint filled it in, which is what made it look like a
/// coordinate problem rather than missing content.
#[test]
fn opening_paints_the_top_of_every_page() {
    let document = open_contract();

    let first_inked_row = |page: usize| -> Option<usize> {
        let (pixels, width, height) = document.page_pixels(page)?;
        (0..height as usize).find(|row| {
            let start = row * width as usize * 4;
            let end = start + width as usize * 4;
            end <= pixels.len() && pixels[start..end].chunks_exact(4).any(|px| px[0] < 200)
        })
    };

    let top_of_page_one = first_inked_row(0).expect("page 1 has text");
    for page in 1..document.page_count() {
        let top = first_inked_row(page).unwrap_or(usize::MAX);
        // Pages share a top margin, so a later page starting much lower than
        // the first means its opening paragraph was dropped.
        assert!(
            top <= top_of_page_one + 24,
            "page {} starts at row {top}, well below page 1's {top_of_page_one} — its first \
             paragraph was not painted",
            page + 1
        );
    }
}
