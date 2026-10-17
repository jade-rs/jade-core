//! Selection and undo, the two things an editor needs beyond typing.

mod common;
use common::{open_contract, paragraph_on_first_page};

#[test]
fn a_selection_is_reported_and_a_caret_is_not() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);

    document.place_caret(edit_paragraph, 0);
    assert!(
        document.selection().is_none(),
        "a collapsed caret should not read as a selection"
    );

    document
        .select((edit_paragraph, 0), (edit_paragraph, 6), 1)
        .expect("select");
    assert_eq!(
        document.selection(),
        Some(((edit_paragraph, 0), (edit_paragraph, 6)))
    );
}

/// The highlight is painted by the renderer, so selecting has to change pixels.
#[test]
fn selecting_paints_a_highlight() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    // The whole buffer, not an ink count: a selection highlight is a pale wash
    // behind the text, and counting dark pixels does not see it.
    let page = |document: &common::Doc| document.page_pixels(0).expect("pixels").0;
    let before = page(&document);

    document
        .select((edit_paragraph, 0), (edit_paragraph, 12), 1)
        .expect("select");

    assert_ne!(before, page(&document), "selecting drew no highlight");
}

#[test]
fn replacing_a_selection_removes_exactly_it() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    let before = document.paragraph_text(edit_paragraph).expect("text");
    let cut: usize = before.chars().take(5).map(char::len_utf8).sum();
    let tail: String = before.chars().skip(5).collect();

    document
        .select((edit_paragraph, 0), (edit_paragraph, cut), 1)
        .expect("select");
    let caret = document.replace_selection("", 1).expect("delete selection");

    assert_eq!(
        document.paragraph_text(edit_paragraph).as_deref(),
        Some(tail.as_str())
    );
    assert_eq!(
        caret,
        (edit_paragraph, 0),
        "the caret should collapse to the start"
    );
    assert!(
        document.selection().is_none(),
        "the selection should be gone after replacing it"
    );
}

#[test]
fn an_edit_can_be_undone_and_redone() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    let original = document.paragraph_text(edit_paragraph).expect("text");

    assert!(!document.can_undo(), "nothing has happened yet");
    document
        .insert_text(edit_paragraph, 0, "取り消す", 1)
        .expect("insert");
    assert!(document.can_undo(), "the edit was not recorded");
    let typed = document.paragraph_text(edit_paragraph).expect("text");
    assert_ne!(typed, original);

    document.undo().expect("undo");
    assert_eq!(
        document.paragraph_text(edit_paragraph).as_deref(),
        Some(original.as_str()),
        "undo did not restore the paragraph"
    );

    assert!(document.can_redo(), "the undo was not recorded");
    document.redo().expect("redo");
    assert_eq!(
        document.paragraph_text(edit_paragraph).as_deref(),
        Some(typed.as_str()),
        "redo did not reapply the edit"
    );
}

/// Undo has to reach the pixels, not only the model — a repaint that skipped
/// it would leave the old text on screen with the right text underneath.
#[test]
fn undo_repaints_the_page() {
    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    let page = |document: &common::Doc| document.page_pixels(0).expect("pixels").0;
    let before = page(&document);

    document
        .insert_text(edit_paragraph, 0, "国国国国国国", 1)
        .expect("insert");
    let after_typing = page(&document);
    assert_ne!(after_typing, before, "typing did not repaint the page");

    document.undo().expect("undo");
    assert_eq!(
        page(&document),
        before,
        "undo did not restore the original page pixels"
    );

    document.redo().expect("redo");
    assert_eq!(
        page(&document),
        after_typing,
        "redo did not restore the edited page pixels"
    );
}

/// Saving has to produce a .docx that reads back with the edit in it, not just
/// bytes that were written somewhere.
#[test]
fn an_edited_document_exports_and_reloads() {
    use jade_docx_native_render::{FaceSpec, NativeDocument};

    let mut document = open_contract();
    let edit_paragraph = common::paragraph_on_first_page(&mut document);
    const INSERTED: &str = "保存テストマーカー";
    document
        .insert_text(edit_paragraph, 0, INSERTED, 1)
        .expect("insert");

    let bytes = document.export_docx().expect("export");
    assert!(
        bytes.len() > 1000,
        "suspiciously small export: {}",
        bytes.len()
    );

    // Reopened the same way the app opens a file, so this covers the round trip
    // an editor actually performs rather than only the writer.
    let regular = common::repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let faces = [FaceSpec {
        family: "Noto Sans JP",
        weight: 400,
        path: &regular,
    }];
    let reloaded = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).expect("reopen the export");

    assert!(
        reloaded
            .paragraph_text(edit_paragraph)
            .is_some_and(|text| text.contains(INSERTED)),
        "the export did not carry the edit"
    );
}

/// Selecting everything needs to know where the document ends.
#[test]
fn the_paragraph_count_covers_the_whole_document() {
    let document = open_contract();
    let count = document.paragraph_count();

    assert!(count > 1, "expected several paragraphs, got {count}");
    assert!(
        document.paragraph_text(count - 1).is_some(),
        "the last paragraph the count promises is not there",
    );
    assert!(
        document.paragraph_text(count).is_none(),
        "there is a paragraph past the count",
    );
}

/// Typing over a selection replaces it. Inserting at the caret instead leaves
/// the selected text on the page with the new character beside it.
#[test]
fn typing_over_a_selection_replaces_it() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).expect("paragraph text");
    // Two characters in, so the replacement is in the middle rather than at an
    // edge where an off-by-one would not show.
    let start = text.char_indices().nth(2).expect("a third character").0;
    let end = text.char_indices().nth(5).expect("a sixth character").0;
    document
        .select((paragraph, start), (paragraph, end), 1)
        .expect("select");

    let caret = document.replace_selection("X", 1).expect("replace");

    let after = document.paragraph_text(paragraph).expect("paragraph text");
    assert_eq!(after, format!("{}X{}", &text[..start], &text[end..]));
    assert_eq!(caret, (paragraph, start + "X".len()));
    assert!(
        document.selection().is_none(),
        "the selection outlived the text it covered",
    );
}
