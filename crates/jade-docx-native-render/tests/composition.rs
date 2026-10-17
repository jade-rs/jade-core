//! The two operations an IME needs: read a paragraph, and replace a range in it.
//!
//! Composition replaces the preedit in place every time the reading changes, so
//! an insert-only path would leave every intermediate behind. These check the
//! pieces before the IME plumbing is built on them.

mod common;
use common::open_contract;

#[test]
fn a_paragraph_reads_back_as_text() {
    let document = open_contract();

    let text = document.paragraph_text(6).expect("paragraph 6 text");
    assert!(!text.is_empty(), "paragraph 6 came back empty");
}

#[test]
fn replacing_a_range_changes_only_that_range() {
    let mut document = open_contract();
    let before = document.paragraph_text(6).expect("paragraph 6 text");
    let tail: String = before.chars().skip(3).collect();

    // Replace the first three characters, which is the shape a committed
    // composition takes.
    let head_bytes: usize = before.chars().take(3).map(char::len_utf8).sum();
    document
        .replace_range(6, 0, head_bytes, "置換", 1)
        .expect("replace and repaint");

    let after = document.paragraph_text(6).expect("paragraph 6 text");
    assert_eq!(
        after,
        format!("置換{tail}"),
        "the replacement did not land exactly on the range"
    );
}

/// A composition, in miniature: the same range replaced repeatedly as the
/// reading grows, then committed. The paragraph must hold one result, not a
/// trail of them.
#[test]
fn a_preedit_replaced_in_place_leaves_no_trail() {
    let mut document = open_contract();
    let before = document.paragraph_text(6).expect("paragraph 6 text");

    let mut marked_len = 0usize;
    for reading in ["に", "にほ", "にほん", "日本"] {
        document
            .replace_range(6, 0, marked_len, reading, 1)
            .expect("replace the preedit");
        marked_len = reading.len();
    }

    let after = document.paragraph_text(6).expect("paragraph 6 text");
    assert_eq!(
        after,
        format!("日本{before}"),
        "the paragraph kept intermediate readings"
    );
}
