//! Comments and change tracking — the review loop a contract goes through.

mod common;
use common::{open_contract, paragraph_on_first_page};

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
