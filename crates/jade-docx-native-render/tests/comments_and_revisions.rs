//! Comments and change tracking — the review loop a contract goes through.

mod common;
use common::{open_contract, paragraph_on_first_page};
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_native_render::NativeDocument;

#[test]
fn a_comment_can_be_added_to_a_selection() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 12), 1)
        .expect("select");
    let before = document.comment_count();

    document
        .add_comment("確認してください", "レビュアー")
        .expect("add a comment");

    assert_eq!(document.comment_count(), before + 1);
}

/// A comment has to survive being written out, or it never reaches the other
/// side — which is the whole point of leaving one.
#[test]
fn a_comment_survives_export_and_reload() {
    use jade_docx_native_render::{FaceSpec, NativeDocument};

    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.place_caret(paragraph, 0);
    document
        .add_comment("往復テスト", "レビュアー")
        .expect("add a comment");
    let expected = document.comment_count();

    let bytes = document.export_docx().expect("export");
    let regular = common::repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let faces = [FaceSpec {
        family: "Noto Sans JP",
        weight: 400,
        path: &regular,
    }];
    let reloaded = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).expect("reopen");

    assert_eq!(
        reloaded.comment_count(),
        expected,
        "the comment did not survive the round trip"
    );
}

/// With tracking on, an edit is recorded as a revision rather than applied
/// outright — so the text still changes, but the document carries the mark.
#[test]
fn change_tracking_can_be_turned_on_and_off() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.place_caret(paragraph, 0);

    document
        .set_change_tracking(true, "レビュアー")
        .expect("start tracking");
    document
        .insert_text(paragraph, 0, "追記", 1)
        .expect("tracked insert");
    assert!(
        document
            .paragraph_text(paragraph)
            .is_some_and(|text| text.starts_with("追記")),
        "the tracked edit did not reach the text"
    );

    document
        .set_change_tracking(false, "レビュアー")
        .expect("stop");
}

/// The renderer's markup colours, as RGB: an insertion's, a deletion's.
const INSERTED: [u8; 3] = [0x1b, 0x5e, 0x20];
const DELETED: [u8; 3] = [0xb7, 0x1c, 0x1c];

/// How many pixels of the document's pages are drawn in `color`, and the
/// longest horizontal line of them — an underline or a strikethrough is far
/// longer than any stroke of a glyph.
fn painted_in(document: &NativeDocument, [red, green, blue]: [u8; 3]) -> (usize, usize) {
    let (mut count, mut longest) = (0, 0);
    for index in 0..document.page_count() {
        let (pixels, width, _) = document.page_pixels(index).unwrap();
        for row in pixels.chunks(width as usize * 4) {
            let mut run = 0;
            for pixel in row.chunks(4) {
                // BGRA.
                let close = [pixel[2], pixel[1], pixel[0]]
                    .iter()
                    .zip([red, green, blue])
                    .all(|(&have, want)| have.abs_diff(want) <= 24);
                if close {
                    count += 1;
                    run += 1;
                    longest = longest.max(run);
                } else {
                    run = 0;
                }
            }
        }
    }
    (count, longest)
}

/// As Word shows tracked changes unless the document was saved with a view
/// that leaves them out: an insertion coloured and underlined, a deletion
/// kept, struck through. Rejected, the markup goes with them.
#[test]
fn tracked_changes_are_drawn_as_markup() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    assert_eq!(painted_in(&document, INSERTED).0, 0);
    assert_eq!(painted_in(&document, DELETED).0, 0);
    let before = document.paragraph_text(paragraph).unwrap();
    document.set_change_tracking(true, "jade").unwrap();

    let typed = "Inserted words ";
    document.insert_text(paragraph, 0, typed, 1).unwrap();
    let (count, longest) = painted_in(&document, INSERTED);
    assert!(count > 50, "the insertion is coloured: {count} pixels");
    assert!(longest > 40, "and underlined: its longest line {longest}px");

    // The paragraph's first three characters, deleted.
    let start = typed.len();
    let end = start + before.char_indices().nth(3).unwrap().0;
    document
        .apply_command(
            HeadlessCommand::ReplaceRange {
                start_paragraph: paragraph,
                start_offset: start,
                end_paragraph: paragraph,
                end_offset: end,
                text: String::new(),
            },
            true,
        )
        .unwrap();
    let (count, longest) = painted_in(&document, DELETED);
    assert!(count > 20, "the deletion is coloured: {count} pixels");
    assert!(
        longest > 15,
        "and struck through: its longest line {longest}px"
    );

    document
        .apply_command(
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            },
            true,
        )
        .unwrap();
    assert_eq!(document.paragraph_text(paragraph).unwrap(), before);
    assert_eq!(painted_in(&document, INSERTED).0, 0, "rejected, no markup");
    assert_eq!(painted_in(&document, DELETED).0, 0, "rejected, no markup");
}
