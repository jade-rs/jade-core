//! A paint window draws the pages in view when an edit re-flows the whole
//! document, and leaves the rest stale until they are asked for — then they
//! come out exactly as a full repaint draws them.

mod common;

use common::repo_path;
use jade_docx_native_render::{FontFace, NativeDocument};

fn open(bytes: &[u8]) -> NativeDocument {
    let fonts = repo_path("fonts/Noto_Sans_JP/static");
    let faces: Vec<FontFace> = [
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
    .collect();
    NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces).unwrap()
}

#[test]
fn pages_outside_the_window_are_drawn_later_as_a_full_repaint_draws_them() {
    let bytes = std::fs::read(repo_path("public/fixtures/yokohama.docx")).unwrap();
    let mut windowed = open(&bytes);
    let mut full = open(&bytes);
    let pages = windowed.page_count();
    assert!(pages > 100, "a long document: {pages} pages");
    assert!(!windowed.has_stale_pages(), "opening draws every page");

    windowed.set_paint_window(Some(0..3));
    let far = pages - 2;
    let far_before = windowed.page_raster_generation(far);
    for document in [&mut windowed, &mut full] {
        document.insert_paragraph_break(40, 3).unwrap();
    }
    assert_eq!(windowed.page_count(), full.page_count());

    // The window is drawn; the pages after it are laid out but not drawn.
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
    assert!(windowed.is_page_stale(far));
    assert_eq!(
        windowed.page_raster_generation(far),
        far_before,
        "not drawn yet"
    );
    assert!(windowed.has_stale_pages());

    // Asked for, a stale page is drawn as the full repaint drew it.
    assert!(windowed.paint_stale_pages(far..far + 1).unwrap());
    assert!(!windowed.is_page_stale(far));
    assert_eq!(windowed.page_pixels(far), full.page_pixels(far));
    assert!(
        !windowed.paint_stale_pages(far..far + 1).unwrap(),
        "nothing left there"
    );

    // And the rest, all at once.
    assert!(windowed.paint_stale_pages(0..usize::MAX).unwrap());
    assert!(!windowed.has_stale_pages());
    for page in 0..windowed.page_count() {
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }

    // Undo re-flows again, and leaves the same stale pages behind.
    windowed.undo().unwrap();
    full.undo().unwrap();
    assert!(windowed.is_page_stale(far));
    windowed.paint_stale_pages(0..usize::MAX).unwrap();
    for page in 0..windowed.page_count() {
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }
}

#[test]
fn without_a_window_every_page_is_drawn() {
    let bytes = std::fs::read(repo_path("public/fixtures/yokohama.docx")).unwrap();
    let mut document = open(&bytes);
    document.insert_paragraph_break(40, 3).unwrap();
    assert!(!document.has_stale_pages());
    document.set_paint_window(Some(0..2));
    document.set_paint_window(None);
    document.undo().unwrap();
    assert!(!document.has_stale_pages());
}
