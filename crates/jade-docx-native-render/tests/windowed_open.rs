//! Opening with a paint window lays the document out once and draws only the
//! pages a shell shows first; the rest come out, when asked for, exactly as
//! the plain open draws them.

mod common;

use common::repo_path;
use jade_docx_native_render::document::full_layout_passes;
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

#[test]
fn a_windowed_open_draws_the_window_and_the_rest_later_as_open_draws_them() {
    let bytes = yokohama();
    let faces = faces();
    let full = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
    assert!(!full.has_stale_pages(), "a plain open draws every page");

    let passes_before = full_layout_passes();
    let mut windowed =
        NativeDocument::open_with_font_faces_and_paint_window(&bytes, 816.0, 1056.0, &faces, 0..3)
            .unwrap();
    assert_eq!(
        full_layout_passes() - passes_before,
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
    let before = full_layout_passes();
    let document = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces()).unwrap();
    assert_eq!(full_layout_passes() - before, 1);
    assert!(!document.has_stale_pages());
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

/// `cargo test --release -p jade-docx-native-render --test windowed_open -- --ignored --nocapture`
#[test]
#[ignore]
fn open_timing_on_yokohama() {
    let bytes = yokohama();
    let faces = faces();
    let time = |windowed: bool| {
        median(
            (0..10)
                .map(|_| {
                    let started = Instant::now();
                    let document = if windowed {
                        NativeDocument::open_with_font_faces_and_paint_window(
                            &bytes,
                            816.0,
                            1056.0,
                            &faces,
                            0..3,
                        )
                    } else {
                        NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces)
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
    eprintln!("open (every page drawn): median {full:?}");
    eprintln!("open with paint window 0..3: median {windowed:?}");
}
