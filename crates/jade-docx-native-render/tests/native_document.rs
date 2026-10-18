//! Covers `NativeDocument`, the type the app actually uses.
//!
//! `keystroke_cost.rs` measures the repaint sequence written out by hand; this
//! checks the packaged version behaves the same, so the app is not relying on a
//! sequence only a benchmark ever exercised.

mod common;

use jade_docx_native_render::{FaceSpec, NativeDocument};
use std::path::PathBuf;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn open_contract() -> NativeDocument {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    open_bytes(&bytes)
}

fn open_bytes(bytes: &[u8]) -> NativeDocument {
    let regular = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let bold = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Bold.ttf");
    let mut faces = Vec::new();
    for family in [
        "Noto Sans JP",
        "Times New Roman",
        "Yu Mincho",
        "MS Mincho",
        "serif",
    ] {
        faces.push(FaceSpec {
            family,
            weight: 400,
            path: &regular,
        });
        faces.push(FaceSpec {
            family,
            weight: 700,
            path: &bold,
        });
    }
    NativeDocument::open(bytes, PAGE_WIDTH_PX, PAGE_HEIGHT_PX, &faces).expect("open the document")
}

#[test]
fn opening_paints_every_page() {
    let document = open_contract();

    // The browser paginates this fixture to three pages, and the native path
    // uses the same layout code.
    assert_eq!(document.page_count(), 3);

    let (pixels, width, height) = document.page_pixels(0).expect("page 1 pixels");
    // Against the box the document asked for, not a hardcoded one: this fixture
    // is A4, and a surface sized to anything else draws each page short.
    let (expected_width, expected_height) = document.layout_page_size();
    assert_eq!(
        (width, height),
        (
            expected_width.round() as i32,
            expected_height.round() as i32
        )
    );
    let inked = pixels
        .chunks_exact(4)
        .filter(|px| px[0] < 200 || px[1] < 200 || px[2] < 200)
        .count();
    assert!(inked > 5_000, "page 1 looks blank: {inked} inked pixels");
}

#[test]
fn typing_changes_the_page_it_is_typed_on() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    let before = document.page_pixels(0).expect("page 1 pixels").0;

    document
        .insert_text(edit_paragraph, 0, "ネイティブ", 1)
        .expect("insert and repaint");

    let after = document.page_pixels(0).expect("page 1 pixels").0;
    assert_ne!(
        before, after,
        "the page is unchanged — a bounded repaint that draws nothing would be \
         fast for the wrong reason"
    );
}

/// Typing repeatedly must stay cheap, not just be cheap once: a repaint that
/// quietly fell back to a full render would still pass the tests above.
#[test]
fn repeated_typing_stays_far_cheaper_than_the_open() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);

    let mut repaints = Vec::new();
    for _ in 0..10 {
        repaints.push(
            document
                .insert_text(edit_paragraph, 0, "あ", 1)
                .expect("insert and repaint"),
        );
    }

    // The minimum, not the median. This asserts a property of the code — that a
    // repaint is not a full render — and the fastest run is the one least
    // contaminated by whatever else the machine was doing. A median here failed
    // once simply because a build was running alongside it.
    let fastest = repaints.iter().min().copied().expect("ten repaints");

    // Loose on purpose: the claim is the order of magnitude. A full render of
    // this fixture is tens of milliseconds.
    assert!(
        fastest.as_millis() < 25,
        "fastest keystroke repaint {fastest:?} looks like a full render"
    );
}

/// Every keystroke changes the page and matches an independent full render.
/// Added text can push existing lines onto the next page, decreasing total ink.
#[test]
fn every_keystroke_matches_a_fresh_render() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);

    let original_text = document.paragraph_text(edit_paragraph).expect("paragraph");
    let mut previous = document.page_pixels(0).expect("page 1 pixels").0;
    for keystroke in 1..=5 {
        document
            .insert_text(edit_paragraph, 0, "国国国国", 1)
            .expect("insert and repaint");
        assert_eq!(
            document.paragraph_text(edit_paragraph).expect("paragraph"),
            "国国国国".repeat(keystroke) + &original_text
        );
        let current = document.page_pixels(0).expect("page 1 pixels").0;
        assert!(
            current != previous,
            "keystroke {keystroke} did not change the page"
        );
        let reopened = open_bytes(&document.export_docx().expect("export edited document"));
        let expected = reopened.page_pixels(0).expect("fresh page 1 pixels").0;
        assert!(
            current == expected,
            "keystroke {keystroke} repaint differs from a fresh render of the edited document"
        );
        previous = current;
    }
}

/// Editing a paragraph on a later page has to draw on that page.
///
/// The repaint is bounded to a page, so the bound must follow the edit. Bounded
/// to page 1 while editing page 2, the repaint runs, reports a plausible time,
/// and redraws an unchanged page 1 — which is indistinguishable from "typing
/// does nothing" and cost a round of debugging once already.
#[test]
fn typing_on_a_later_page_matches_a_fresh_render() {
    let mut document = open_contract();

    // A paragraph on page 2, found by asking rather than assuming.
    let (page, paragraph) = (0..60)
        .find_map(|paragraph| {
            let caret = document.place_caret(paragraph, 0)?;
            let (page, _) = document.page_and_offset(caret.document_y);
            (page == 1).then_some((page, paragraph))
        })
        .expect("a paragraph on page 2");

    let before = document.page_pixels(page).expect("page pixels").0;

    // Bounded to the page being edited, as the shell does.
    document
        .insert_text(paragraph, 0, "国国国国国", page + 1)
        .expect("insert and repaint");

    let after = document.page_pixels(page).expect("edited page pixels").0;
    assert_ne!(
        after,
        before,
        "page {} did not change after editing one of its paragraphs",
        page + 1
    );
    let bytes = document.export_docx().expect("export edited document");
    let fresh = open_bytes(&bytes);
    assert_eq!(
        after,
        fresh.page_pixels(page).expect("fresh page pixels").0,
        "the later-page repaint differs from a fresh render"
    );
}

#[test]
fn a_raster_scale_paints_more_pixels_without_moving_the_layout() {
    let mut document = open_contract();
    let pages = document.page_count();
    let (page_width, page_height) = document.layout_page_size();
    let caret_before = document.place_caret(0, 0);

    assert!(document.set_raster_scale(2.0).expect("scale to 2x"));
    // Setting the same scale again is free.
    assert!(!document.set_raster_scale(2.0).expect("same scale"));

    assert_eq!(document.page_count(), pages);
    assert_eq!(document.layout_page_size(), (page_width, page_height));
    assert_eq!(document.place_caret(0, 0), caret_before);
    for page in 0..pages {
        let (pixels, width, height) = document.page_pixels(page).expect("page pixels");
        assert_eq!(
            (width, height),
            (
                (page_width * 2.0).ceil() as i32,
                (page_height * 2.0).ceil() as i32
            )
        );
        let inked = pixels
            .chunks_exact(4)
            .filter(|px| px[0] < 200 || px[1] < 200 || px[2] < 200)
            .count();
        assert!(inked > 20_000, "page {page} looks blank at 2x: {inked}");
    }
    if let Ok(directory) = std::env::var("JADE_DUMP_PAGE") {
        let (pixels, width, height) = document.page_pixels(0).unwrap();
        common::write_bgra_png(&format!("{directory}/page-2x.png"), &pixels, width, height);
        document.set_raster_scale(1.0).unwrap();
        let (pixels, width, height) = document.page_pixels(0).unwrap();
        common::write_bgra_png(&format!("{directory}/page-1x.png"), &pixels, width, height);
    }

    assert!(document.set_raster_scale(0.0).is_err());
    assert!(document.set_raster_scale(f64::NAN).is_err());
}

#[test]
fn dark_paper_turns_the_page_over_on_screen_but_not_in_the_pdf() {
    let mut document = open_contract();
    let (light, width, _) = document.page_pixels(0).expect("page 1 pixels");
    let printed = document.export_pdf().expect("export on light paper");

    assert!(document.set_dark_paper(true).expect("switch to dark paper"));
    assert!(
        !document.set_dark_paper(true).expect("no-op"),
        "already dark"
    );
    let (dark, _, _) = document.page_pixels(0).expect("page 1 pixels");
    // BGRA, premultiplied and opaque. The margin is paper, so a dark grey.
    let corner = &dark[(4 * width as usize + 4) * 4..][..4];
    assert!(
        corner[..3]
            .iter()
            .all(|&channel| (20..60).contains(&channel)),
        "{corner:?}"
    );
    // The ink went the other way: what was dark text is now light on it.
    let lit = dark
        .chunks_exact(4)
        .filter(|px| px[0] > 150 && px[1] > 150 && px[2] > 150)
        .count();
    assert!(lit > 5_000, "no light ink on the dark page: {lit}");

    assert_eq!(
        document.export_pdf().expect("export on dark paper"),
        printed,
        "print and export keep the document's own colours"
    );

    assert!(document.set_dark_paper(false).expect("back to light paper"));
    let (back, _, _) = document.page_pixels(0).expect("page 1 pixels");
    assert!(back == light, "switching back restores the page exactly");
}
