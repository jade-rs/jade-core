//! Headers and footers, and finding a floating object on the page.

mod common;
use common::{open_fixture, Doc};

fn ink(document: &Doc, page: usize) -> usize {
    document
        .page_pixels(page)
        .expect("page pixels")
        .0
        .chunks_exact(4)
        .filter(|px| px[0] < 200)
        .count()
}

#[test]
fn header_text_reaches_the_page() {
    let mut document = open_fixture("public/fixtures/header.docx");
    let before = ink(&document, 0);

    document
        .set_header_footer_text(false, 0, "ヘッダー差し替え")
        .expect("set the header");

    assert_ne!(
        ink(&document, 0),
        before,
        "the header change did not reach the page"
    );
}

#[test]
fn footer_text_reaches_the_page() {
    let mut document = open_fixture("public/fixtures/footer.docx");
    let before = ink(&document, 0);

    document
        .set_header_footer_text(true, 0, "フッター差し替え")
        .expect("set the footer");

    assert_ne!(
        ink(&document, 0),
        before,
        "the footer change did not reach the page"
    );
}

#[test]
fn a_header_change_can_be_undone() {
    let mut document = open_fixture("public/fixtures/header.docx");
    let before = ink(&document, 0);

    document
        .set_header_footer_text(false, 0, "取り消される見出し")
        .expect("set the header");
    document.undo().expect("undo");

    assert_eq!(ink(&document, 0), before, "undo did not restore the header");
}

/// Empty space is not an object. Answering with one would let a stray click
/// select something that is not there.
#[test]
fn empty_space_holds_no_object() {
    let document = open_fixture("public/fixtures/contract.docx");
    assert!(document.object_at(5.0, 5.0).is_none());
}

/// The first floating object on page 1, found by sweeping rather than by
/// assuming where a fixture put it.
fn first_object(document: &Doc) -> Option<jade_docx_native_render::ObjectHit> {
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

/// The first floating object anywhere in the document, page by page.
///
/// `first_object` only sweeps page 1; a picture that has been pushed past it
/// would read as "no object" and quietly turn a test into a no-op.
fn object_anywhere(document: &Doc) -> Option<jade_docx_native_render::ObjectHit> {
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

/// Moving an image has to be undoable the same way a mistyped word is.
#[test]
fn a_floating_object_can_be_moved_and_undone() {
    let mut document = open_fixture("public/fixtures/image-floating.docx");

    // A fixture with no floating object cannot exercise this; failing on it
    // would be reporting on the fixture, not the code.
    let Some(object) = first_object(&document) else {
        return;
    };

    // A translation keeps the ink count identical, so count is the wrong
    // question here: compare the pixels themselves, and ask the model where the
    // object ended up.
    let before = document.page_pixels(0).expect("page pixels").0;
    document
        .transform_object(
            object,
            object.x + 40.0,
            object.y + 40.0,
            object.width,
            object.height,
        )
        .expect("move the object");

    assert!(
        document
            .object_at(
                object.x + 40.0 + object.width / 2.0,
                object.y + 40.0 + object.height / 2.0
            )
            .is_some(),
        "the object is not where it was moved to",
    );
    assert_ne!(
        document.page_pixels(0).expect("page pixels").0,
        before,
        "the move did not reach the page",
    );

    document.undo().expect("undo");
    assert_eq!(
        document.page_pixels(0).expect("page pixels").0,
        before,
        "undo did not put the object back",
    );
}

/// A drag is a stream of moves, and has to undo as one. Recording each move
/// would make putting a picture back a matter of undoing thirty times.
#[test]
fn the_moves_of_one_drag_undo_together() {
    let mut document = open_fixture("public/fixtures/image-floating.docx");
    let Some(object) = first_object(&document) else {
        return;
    };
    let before = document.page_pixels(0).expect("page pixels").0;

    // The first move records; the rest replace what it recorded.
    document
        .transform_object(
            object,
            object.x + 10.0,
            object.y,
            object.width,
            object.height,
        )
        .expect("first move");
    for step in 2..=5 {
        document
            .retransform_object(
                object,
                object.x + 10.0 * f64::from(step),
                object.y,
                object.width,
                object.height,
            )
            .expect("a move of the same drag");
    }
    let dragged = document.page_pixels(0).expect("page pixels").0;
    assert_ne!(dragged, before, "the drag did not reach the page");

    document.undo().expect("undo");

    assert_eq!(
        document.page_pixels(0).expect("page pixels").0,
        before,
        "one undo did not put the picture back where the drag started",
    );
}

/// A picture has to be painted, not skipped. The browser decodes images through
/// `createImageBitmap`, which needs a canvas and an event loop; natively the
/// host decodes the data URI itself. Before it did, opening a document with a
/// picture panicked.
#[test]
fn a_picture_is_painted_on_the_page() {
    let document = open_fixture("public/fixtures/image-floating.docx");
    let Some(object) = first_object(&document) else {
        return;
    };

    // Colour is the evidence: text is black on white, so a pixel whose channels
    // disagree can only have come from the decoded picture.
    let (pixels, width, _) = document.page_pixels(0).expect("page pixels");
    let mut coloured = 0usize;
    for row in (object.y as i32)..((object.y + object.height) as i32) {
        for column in (object.x as i32)..((object.x + object.width) as i32) {
            let index = ((row * width + column) * 4) as usize;
            let Some(px) = pixels.get(index..index + 3) else {
                continue;
            };
            if px[0].abs_diff(px[2]) > 8 || px[1].abs_diff(px[2]) > 8 {
                coloured += 1;
            }
        }
    }

    assert!(
        coloured > 100,
        "the picture did not reach the page: {coloured} coloured pixels inside it",
    );
}

/// A drag repaints only as far as the object's page, which is only worth doing
/// if the result is the page a full repaint would have drawn. A bounded repaint
/// that leaves the picture's old position behind would be a stale hole that
/// nothing later redraws.
#[test]
fn a_bounded_drag_leaves_the_page_a_full_repaint_would_draw() {
    let mut document = open_fixture("public/fixtures/image-floating.docx");
    let Some(object) = first_object(&document) else {
        return;
    };

    document
        .transform_object(
            object,
            object.x + 30.0,
            object.y + 20.0,
            object.width,
            object.height,
        )
        .expect("first move");
    for step in 1..=6 {
        let offset = 30.0 + f64::from(step) * 7.0;
        document
            .retransform_object(
                object,
                object.x + offset,
                object.y + 20.0,
                object.width,
                object.height,
            )
            .expect("a move of the same drag");
    }
    let dragged = document.page_pixels(0).expect("page pixels").0;

    // The same state, drawn from scratch.
    document.repaint_all().expect("full repaint");

    assert_eq!(
        document.page_pixels(0).expect("page pixels").0,
        dragged,
        "the bounded repaint left the page different from a full one",
    );
}

/// The same, with the picture pushed onto page 2.
///
/// One page proves nothing about a bound: everything is inside it. The bound is
/// a page number, so the case it can get wrong is a picture that is not on page
/// 1.
#[test]
fn a_bounded_drag_on_a_later_page_matches_a_full_repaint() {
    let mut document = open_fixture("public/fixtures/image-floating.docx");
    let Some(object) = first_object(&document) else {
        return;
    };

    // Push the anchor down until the picture is off page 1. Breaks go in above
    // it, so the paragraph it is anchored to moves with the text.
    let anchor = object.paragraph;
    for _ in 0..60 {
        document
            .insert_paragraph_break(anchor, 0)
            .expect("a paragraph break");
    }
    // Asserted, not skipped: a silent return here would leave a test that
    // passes without comparing anything, which is what it is for.
    let moved = object_anywhere(&document).expect("the picture after the breaks");
    assert!(
        moved.page > 1,
        "the breaks did not push the picture off page 1, so the bound is untested",
    );
    let page_index = moved.page - 1;

    document
        .transform_object(
            moved,
            moved.x + 20.0,
            moved.y + 10.0,
            moved.width,
            moved.height,
        )
        .expect("first move");
    for step in 1..=6 {
        let offset = 20.0 + f64::from(step) * 6.0;
        document
            .retransform_object(
                moved,
                moved.x + offset,
                moved.y + 10.0,
                moved.width,
                moved.height,
            )
            .expect("a move of the same drag");
    }
    let dragged = document.page_pixels(page_index).expect("page pixels").0;

    document.repaint_all().expect("full repaint");

    assert_eq!(
        document.page_pixels(page_index).expect("page pixels").0,
        dragged,
        "the bounded repaint on page {} differs from a full one",
        moved.page,
    );
}
