//! Deleting and splitting, the operations a keyboard needs beyond typing.

mod common;
use common::{open_contract, paragraph_on_first_page};

#[test]
fn backspace_removes_the_character_before_the_caret() {
    let mut document = open_contract();
    document.insert_text(6, 0, "日本語", 1).expect("insert");
    let before = document.paragraph_text(6).expect("text");
    assert!(before.starts_with("日本語"));

    // Offset is after the three characters just typed.
    document
        .delete_text(6, "日本語".len(), true, 1, 1)
        .expect("backspace");

    let after = document.paragraph_text(6).expect("text");
    assert!(
        after.starts_with("日本") && !after.starts_with("日本語"),
        "backspace did not remove exactly one character: {after:?}"
    );
}

/// Counted in characters, not bytes. Deleting three bytes off a Japanese
/// character would leave a broken one, and the paragraph would still "change".
#[test]
fn backspace_removes_one_character_not_one_byte() {
    let mut document = open_contract();
    let original = document.paragraph_text(6).expect("text");
    document.insert_text(6, 0, "あ", 1).expect("insert");

    document
        .delete_text(6, "あ".len(), true, 1, 1)
        .expect("backspace");

    assert_eq!(
        document.paragraph_text(6).expect("text"),
        original,
        "one backspace after one character did not restore the paragraph"
    );
}

#[test]
fn forward_delete_removes_the_character_after_the_caret() {
    let mut document = open_contract();
    document.insert_text(6, 0, "日本語", 1).expect("insert");

    document.delete_text(6, 0, false, 1, 1).expect("delete");

    let after = document.paragraph_text(6).expect("text");
    assert!(
        after.starts_with("本語"),
        "forward delete did not remove the leading character: {after:?}"
    );
}

#[test]
fn a_paragraph_break_splits_the_paragraph_in_two() {
    let mut document = open_contract();
    let original = document.paragraph_text(6).expect("text");
    let split_at: usize = original.chars().take(4).map(char::len_utf8).sum();
    let (head, tail) = original.split_at(split_at);

    document.insert_paragraph_break(6, split_at).expect("break");

    assert_eq!(document.paragraph_text(6).as_deref(), Some(head));
    assert_eq!(
        document.paragraph_text(7).as_deref(),
        Some(tail),
        "the remainder did not become the next paragraph"
    );
}

/// A break re-flows everything below it, so the page count can change — which
/// is why it is not bounded to the visible page.
#[test]
fn many_paragraph_breaks_still_render_every_page() {
    let mut document = open_contract();
    let pages_before = document.page_count();

    for _ in 0..30 {
        document.insert_paragraph_break(6, 0).expect("break");
    }

    assert!(
        document.page_count() >= pages_before,
        "pages went backwards: {pages_before} -> {}",
        document.page_count()
    );
    let last = document.page_count() - 1;
    let inked = document
        .page_pixels(last)
        .expect("last page pixels")
        .0
        .chunks_exact(4)
        .filter(|px| px[0] < 200)
        .count();
    assert!(inked > 0, "the last page is blank after re-flowing");
}

/// A repaint bounded to page 1 must not make the document one page long.
///
/// Pages are surfaces on the host, and a bounded render stops paginating at the
/// bound. Counting the surfaces it produced would shrink a three-page document
/// to one the moment someone typed on page 1 — and the page list is what the
/// shell draws its sidebar from.
#[test]
fn typing_on_page_one_does_not_shrink_the_page_count() {
    let mut document = open_contract();
    let before = document.page_count();
    assert!(before > 1, "the fixture needs more than one page");
    let paragraph = paragraph_on_first_page(&mut document);

    document.insert_text(paragraph, 0, "あ", 1).expect("insert");

    assert_eq!(document.page_count(), before, "the page count shrank");
}
