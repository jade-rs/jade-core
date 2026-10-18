//! After a body paragraph is split, jade lists the new paragraph before the
//! footnotes the first half references until the surfaces are listed again.
//! A later split of a note or text box paragraph relists them, and then
//! wrote its halves by surface index — after the relisting those named
//! other paragraphs, so the file kept the note's paragraph whole beside its
//! second half.

use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::selection::TextPosition;
use jade_docx_document_core::word_document::{ParagraphStory, WordDocument};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).unwrap()
}

fn load(name: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    load_bytes(&std::fs::read(path).unwrap())
}

fn text(word: &WordDocument, surface: usize) -> String {
    word.paragraph(surface)
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .unwrap_or_default()
}

/// Every paragraph as (story, text), sorted: the order jade lists them in
/// memory can differ from the file's until it relists them.
fn stories(word: &WordDocument) -> Vec<(Option<ParagraphStory>, String)> {
    let mut out: Vec<_> = (0..word.paragraph_count())
        .map(|s| (word.paragraph_story(s), text(word, s)))
        .collect();
    out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    out
}

fn split(word: &mut WordDocument, surface: usize, model_offset: usize) {
    let offset = word
        .model_to_rendered_text_position(TextPosition::new(surface, model_offset))
        .offset;
    apply_headless_commands_to_document(
        word,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertParagraphBreak {
                paragraph: surface,
                offset,
                count: 1,
            }],
        },
    )
    .unwrap();
}

fn find(word: &WordDocument, wanted: &str) -> usize {
    (0..word.paragraph_count())
        .find(|&s| text(word, s) == wanted)
        .unwrap_or_else(|| panic!("no paragraph {wanted:?}"))
}

#[test]
fn a_note_split_after_a_body_split_reaches_the_file() {
    let mut word = load("footnote-table-textbox-order.docx");
    // "Host" ⁵ [box] " after box" ³ → "Host⁵[box] after " | "box³".
    let host = find(&word, "Host after box");
    split(&mut word, host, "Host after ".len());
    let note = find(&word, " Note for Host");
    split(&mut word, note, " Note ".len());
    let expected = stories(&word);
    assert!(expected.iter().any(|(_, t)| t == " Note "));
    let reloaded = load_bytes(&word.export_docx().unwrap());
    assert_eq!(stories(&reloaded), expected);
}

#[test]
fn a_text_box_split_after_a_body_split_reaches_the_file() {
    let mut word = load("footnote-table-textbox-order.docx");
    let host = find(&word, "Host after box");
    split(&mut word, host, "Host after ".len());
    let in_box = find(&word, "Box text");
    split(&mut word, in_box, "Box ".len());
    let expected = stories(&word);
    assert!(expected.iter().any(|(_, t)| t == "Box "));
    let reloaded = load_bytes(&word.export_docx().unwrap());
    assert_eq!(stories(&reloaded), expected);
}
