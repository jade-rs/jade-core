//! Shared fixture setup for the native render tests.
//!
//! Each test binary compiles this module separately, so anything a particular
//! binary does not use is dead code there.
#![allow(dead_code)]

use jade_docx_native_render::{FaceSpec, NativeDocument};
use std::path::PathBuf;

/// The size `open` is *asked* for. The document's own page box wins — this
/// fixture is A4 — so these are a starting point, not what the surfaces end up
/// being.
pub const PAGE_WIDTH_PX: f64 = 816.0;
pub const PAGE_HEIGHT_PX: f64 = 1056.0;

pub fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Alias so tests can name the type in a closure argument. Each test binary
/// compiles this module separately, so it is dead code in the ones that do not
/// use it.
pub type Doc = NativeDocument;

/// A non-empty paragraph on page 1, found rather than assumed.
///
/// Paragraph indices are not stable against page-size or formatting-mark
/// changes, and a test that edits a paragraph on another page repaints an
/// unchanged one and reads as "typing does nothing".
pub fn paragraph_on_first_page(document: &mut NativeDocument) -> usize {
    for paragraph in 0..80 {
        let has_text = document
            .paragraph_text(paragraph)
            .is_some_and(|text| !text.trim().is_empty());
        if !has_text {
            continue;
        }
        if let Some(caret) = document.place_caret(paragraph, 0) {
            if document.page_and_offset(caret.document_y).0 == 0 {
                return paragraph;
            }
        }
    }
    panic!("no paragraph with text on page 1");
}

/// Opens any fixture with the standard face set.
pub fn open_fixture(relative: &str) -> NativeDocument {
    let bytes = std::fs::read(repo_path(relative)).expect("read fixture");
    open_bytes(&bytes)
}

/// Opens a document from bytes with the same faces `open_fixture` uses.
///
/// Lets a test reopen what it exported, which is the only oracle that a
/// rendered page is *right* rather than merely the same as another render of
/// the same session — caches are shared between those two.
pub fn open_bytes(bytes: &[u8]) -> NativeDocument {
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
    // The shell registers this too, as a per-character fallback for the marks a
    // Japanese text face does not carry. A test that leaves it out is not
    // opening the document the way the app does.
    let symbols = PathBuf::from("/System/Library/Fonts/Apple Symbols.ttf");
    if symbols.exists() {
        faces.push(FaceSpec {
            family: "Symbol Fallback",
            weight: 400,
            path: &symbols,
        });
    }
    NativeDocument::open(bytes, PAGE_WIDTH_PX, PAGE_HEIGHT_PX, &faces).expect("open the document")
}

pub fn open_contract() -> NativeDocument {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
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
    NativeDocument::open(&bytes, PAGE_WIDTH_PX, PAGE_HEIGHT_PX, &faces).expect("open the document")
}

/// Writes a page's BGRA pixels out as a PNG, for looking at a failure rather
/// than counting it.
pub fn write_bgra_png(path: &str, bgra: &[u8], width: i32, height: i32) {
    let mut rgba = Vec::with_capacity(bgra.len());
    for pixel in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let buffer: image::RgbaImage =
        image::ImageBuffer::from_raw(width as u32, height as u32, rgba).expect("a page buffer");
    buffer.save(path).expect("write the page");
}
