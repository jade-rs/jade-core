//! Opening with a paint window lays the document out once and draws only the
//! pages a shell shows first; the rest come out, when asked for, exactly as
//! the plain open draws them.

mod common;

use common::repo_path;
use jade_docx_native_render::document::surface_document_renders;
use jade_docx_native_render::{FontFace, NativeDocument};
use std::time::{Duration, Instant};

fn faces() -> Vec<FontFace> {
    let fonts = repo_path("fonts/Noto_Sans_JP/static");
    [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ]
    .into_iter()
    .map(|(weight, file)| FontFace {
        family: "Noto Sans JP".into(),
        weight,
        italic: false,
        path: fonts.join(file),
        index: 0,
    })
    .collect()
}

fn yokohama() -> Vec<u8> {
    std::fs::read(repo_path("public/fixtures/yokohama.docx")).unwrap()
}

/// Sections on different paper (portrait, landscape, portrait), so the box
/// page 2 starts on is not the one layout ends on.
fn sections_mac() -> Vec<u8> {
    std::fs::read(repo_path(
        "tests/word-oracle/mac-elsa-synthetic/sections-mac/source.docx",
    ))
    .unwrap()
}

#[test]
fn a_windowed_open_draws_the_window_and_the_rest_later_as_open_draws_them() {
    let bytes = yokohama();
    let faces = faces();
    let full = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
    assert!(!full.has_stale_pages(), "a plain open draws every page");

    let passes_before = surface_document_renders();
    let mut windowed =
        NativeDocument::open_with_font_faces_and_paint_window(&bytes, 816.0, 1056.0, &faces, 0..3)
            .unwrap();
    assert_eq!(
        surface_document_renders() - passes_before,
        1,
        "opening walks the whole body once"
    );

    let pages = full.page_count();
    assert!(pages > 100, "a long document: {pages} pages");
    assert_eq!(windowed.page_count(), pages);
    assert_eq!(windowed.layout_page_size(), full.layout_page_size());
    for page in 0..3 {
        assert!(
            !windowed.is_page_stale(page),
            "page {page} is in the window"
        );
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }
    assert!(windowed.is_page_stale(3));
    assert!(windowed.is_page_stale(pages - 1));
    assert!(windowed.has_stale_pages());

    assert!(windowed.paint_stale_pages(0..usize::MAX).unwrap());
    assert!(!windowed.has_stale_pages());
    assert_eq!(windowed.page_count(), pages);
    for page in 0..pages {
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }
}

#[test]
fn a_plain_open_also_walks_the_body_once() {
    let bytes = yokohama();
    let before = surface_document_renders();
    let document = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces()).unwrap();
    assert_eq!(surface_document_renders() - before, 1);
    assert!(!document.has_stale_pages());
}

#[test]
fn a_document_ending_on_other_paper_is_built_to_that_box_in_one_render() {
    let bytes = sections_mac();
    let faces = faces();
    let before = surface_document_renders();
    let full = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
    assert_eq!(surface_document_renders() - before, 1, "no rebuild at open");

    let pages = full.page_count();
    assert!(pages > 1, "{pages} pages");
    // Portrait, landscape, portrait: a probe that stops once page 2 starts
    // reads the landscape box, and the host had to be built again.
    let boxes: Vec<_> = (0..pages).map(|page| full.page_box(page)).collect();
    let last = boxes[pages - 1];
    assert!(
        boxes.iter().any(|&page_box| page_box != last),
        "the sections are on different paper: {boxes:?}"
    );
    assert_eq!(full.layout_page_size(), last, "layout ends on the last box");

    let before = surface_document_renders();
    let mut windowed =
        NativeDocument::open_with_font_faces_and_paint_window(&bytes, 816.0, 1056.0, &faces, 0..1)
            .unwrap();
    assert_eq!(surface_document_renders() - before, 1, "no rebuild at open");
    assert_eq!(windowed.page_count(), pages);
    assert_eq!(windowed.layout_page_size(), full.layout_page_size());
    for page in 0..pages {
        assert_eq!(windowed.page_box(page), full.page_box(page), "page {page}");
        assert_eq!(windowed.is_page_stale(page), page != 0, "page {page}");
    }
    assert_eq!(windowed.page_pixels(0), full.page_pixels(0));

    assert!(windowed.paint_stale_pages(0..usize::MAX).unwrap());
    assert!(!windowed.has_stale_pages());
    for page in 0..pages {
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

/// `cargo test --release -p jade-docx-native-render --test windowed_open -- --ignored --nocapture`
#[test]
#[ignore]
fn open_timing() {
    // sections-mac has three pages, so its window is the first alone.
    for (name, bytes, window) in [
        ("yokohama", yokohama(), 0..3),
        ("sections-mac", sections_mac(), 0..1),
    ] {
        open_timing_on(name, &bytes, window);
    }
}

fn open_timing_on(name: &str, bytes: &[u8], window: std::ops::Range<usize>) {
    let faces = faces();
    let time = |windowed: bool| {
        median(
            (0..10)
                .map(|_| {
                    let started = Instant::now();
                    let document = if windowed {
                        NativeDocument::open_with_font_faces_and_paint_window(
                            bytes,
                            816.0,
                            1056.0,
                            &faces,
                            window.clone(),
                        )
                    } else {
                        NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces)
                    }
                    .unwrap();
                    let elapsed = started.elapsed();
                    drop(document);
                    elapsed
                })
                .collect(),
        )
    };
    let full = time(false);
    let windowed = time(true);
    eprintln!("{name}: open (every page drawn): median {full:?}");
    eprintln!("{name}: open with paint window {window:?}: median {windowed:?}");
}
