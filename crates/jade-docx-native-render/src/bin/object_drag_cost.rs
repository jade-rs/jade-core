//! What one move of a picture drag costs, and where the cost is.
//!
//! Dragging goes through `repaint_all`, which redraws every page. That is
//! obviously wasteful and just as obviously might not matter — a claim about
//! sluggishness needs a number before it earns a change. Prints the per-move
//! cost for a document, and the same measurement with the pages the drag cannot
//! touch left out, so the saving is visible rather than assumed.

use jade_docx_native_render::{FaceSpec, NativeDocument, ObjectHit};
use std::path::PathBuf;
use std::time::Instant;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;
/// How many moves a drag of a second or so produces at 60Hz.
const MOVES: usize = 60;

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

/// The first floating object anywhere in the document.
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

/// The first floating object on page 1, found by sweeping.
fn first_object(document: &NativeDocument) -> Option<ObjectHit> {
    let (page_width, page_height) = document.layout_page_size();
    let mut y = 0.0;
    while y < page_height {
        let mut x = 0.0;
        while x < page_width {
            if let Some(hit) = document.object_at(x, y) {
                return Some(hit);
            }
            x += 16.0;
        }
        y += 16.0;
    }
    None
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    values[values.len() / 2]
}

fn main() {
    // A move costs one full repaint, and the fixture with a picture in it is a
    // single page — which says nothing about the case the cost would show up
    // in. This is that case: the same repaint over a document with pages.
    for fixture in [
        "public/fixtures/contract.docx",
        "public/fixtures/large.docx",
    ] {
        let Ok(bytes) = std::fs::read(repo_path(fixture)) else {
            continue;
        };
        drop(bytes);
        let mut document = open(fixture);
        let pages = document.page_count();
        let mut per_repaint = Vec::with_capacity(MOVES);
        for _ in 0..MOVES {
            let elapsed = document.repaint_all().expect("repaint");
            per_repaint.push(elapsed.as_secs_f64() * 1000.0);
        }
        // The shell rebuilds every page's bitmap after a move as well, which
        // is a copy per page on top of the repaint.
        let mut per_read = Vec::with_capacity(MOVES);
        for _ in 0..MOVES {
            let started = Instant::now();
            for page in 0..pages {
                document.page_pixels(page).expect("page pixels");
            }
            per_read.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "{fixture}: {pages} page(s), {:.2} ms per full repaint, \
             {:.2} ms to read every page's pixels (medians of {MOVES})",
            median(per_repaint),
            median(per_read),
        );
    }

    // The fixture with a picture is one page, which is the case a bound cannot
    // help. Breaks push it onto a later page, which is the case that matters.
    for breaks in [0, 60, 240] {
        let fixture = "public/fixtures/image-floating.docx";
        let mut document = open(fixture);
        let Some(object) = first_object(&document) else {
            println!("{fixture}: no floating object");
            continue;
        };
        for _ in 0..breaks {
            document
                .insert_paragraph_break(object.paragraph, 0)
                .expect("a paragraph break");
        }
        let pages = document.page_count();
        let object = object_anywhere(&document).expect("the picture after the breaks");

        // The first move records; the rest replace it, which is what a real
        // drag does.
        document
            .transform_object(object, object.x, object.y, object.width, object.height)
            .expect("first move");

        let mut per_move = Vec::with_capacity(MOVES);
        for step in 0..MOVES {
            let offset = f64::from(step as u32 % 20) - 10.0;
            let started = Instant::now();
            document
                .retransform_object(
                    object,
                    object.x + offset,
                    object.y + offset,
                    object.width,
                    object.height,
                )
                .expect("a move of the same drag");
            per_move.push(started.elapsed().as_secs_f64() * 1000.0);
        }

        // What the same drag would cost repainting everything, for comparison.
        let mut per_full = Vec::with_capacity(MOVES);
        for _ in 0..MOVES {
            per_full.push(document.repaint_all().expect("repaint").as_secs_f64() * 1000.0);
        }

        println!(
            "picture on page {} of {pages}: {:.2} ms per bounded move, \
             {:.2} ms per full repaint (medians of {MOVES})",
            object.page,
            median(per_move),
            median(per_full),
        );
    }
}
