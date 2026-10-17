//! Drags a picture forever so a sampling profiler has something to attach to.
//!
//! Not a benchmark — `object_drag_cost` is. This exists because the timings
//! there say a move costs about a millisecond per page even when the picture is
//! on the last one, and a bound that stops at that page saves nothing. Where
//! that time goes is a question for a call tree, not for another guess at a
//! renderer flag.

use jade_docx_native_render::{FaceSpec, NativeDocument, ObjectHit};
use std::path::PathBuf;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;
/// Enough breaks to push the picture onto a later page, which is the case the
/// cost shows up in.
const BREAKS: usize = 240;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn open(relative: &str) -> NativeDocument {
    let bytes = std::fs::read(repo_path(relative)).expect("read fixture");
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

fn object_anywhere(document: &NativeDocument) -> Option<ObjectHit> {
    let (page_width, page_height) = document.layout_page_size();
    for page in 0..document.page_count() {
        let mut y = 0.0;
        while y < page_height {
            let document_y = document.document_y(page, y);
            let mut x = 0.0;
            while x < page_width {
                if let Some(hit) = document.object_at(x, document_y) {
                    return Some(hit);
                }
                x += 16.0;
            }
            y += 16.0;
        }
    }
    None
}

fn main() {
    let mut document = open("public/fixtures/image-floating.docx");
    let object = object_anywhere(&document).expect("a floating object");
    for _ in 0..BREAKS {
        document
            .insert_paragraph_break(object.paragraph, 0)
            .expect("a paragraph break");
    }
    let object = object_anywhere(&document).expect("the picture after the breaks");
    eprintln!(
        "picture on page {} of {}",
        object.page,
        document.page_count()
    );

    document
        .transform_object(object, object.x, object.y, object.width, object.height)
        .expect("first move");
    let mut step = 0u32;
    loop {
        let offset = f64::from(step % 20) - 10.0;
        document
            .retransform_object(
                object,
                object.x + offset,
                object.y + offset,
                object.width,
                object.height,
            )
            .expect("a move of the same drag");
        step = step.wrapping_add(1);
    }
}
