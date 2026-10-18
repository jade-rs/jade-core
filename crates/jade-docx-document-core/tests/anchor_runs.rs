//! Text typed into a paragraph that holds only a note reference or a text
//! box goes into a run of its own after it. Typed into the reference's run,
//! it made one run carry both, and the next edit that split the run (a
//! formatting change, a comment) copied the reference into both halves: the
//! document then referenced the note twice.

use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::document::Run;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::selection::TextPosition;
use jade_docx_document_core::word_document::{ParagraphAnchorKind, WordDocument};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

fn load(name: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    block_on(WordDocument::load(
        &std::fs::read(path).unwrap(),
        &Options::default(),
    ))
    .unwrap()
}

fn apply(word: &mut WordDocument, command: HeadlessCommand) {
    apply_headless_commands_to_document(
        word,
        &HeadlessEditRequest {
            commands: vec![command],
        },
    )
    .unwrap();
}

fn rendered(word: &WordDocument, paragraph: usize, offset: usize) -> usize {
    word.model_to_rendered_text_position(TextPosition::new(paragraph, offset))
        .offset
}

fn anchored(run: &Run) -> bool {
    run.text_box.is_some()
        || !run.footnote_reference_ids.is_empty()
        || !run.endnote_reference_ids.is_empty()
}

/// Splits paragraph 0 before its anchor (at `offset`) and types into the
/// new paragraph, which holds only the anchor.
fn type_after_a_lone_anchor(name: &str, offset: usize) -> WordDocument {
    let mut word = load(name);
    let at = rendered(&word, 0, offset);
    apply(
        &mut word,
        HeadlessCommand::InsertParagraphBreak {
            paragraph: 0,
            offset: at,
            count: 1,
        },
    );
    assert_eq!(word.paragraph_text_len_at(1), Some(0));
    apply(
        &mut word,
        HeadlessCommand::InsertText {
            paragraph: 1,
            offset: 0,
            text: "typed".into(),
        },
    );
    word
}

#[test]
fn text_typed_after_a_lone_note_reference_gets_its_own_run() {
    let mut word = type_after_a_lone_anchor("footnote.docx", 5);
    let runs = &word.paragraph(1).unwrap().runs;
    assert!(
        runs.iter().all(|run| !anchored(run) || run.text.is_empty()),
        "{runs:?}"
    );
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "typed"
    );
    // The reference stays before the text.
    let anchors = word.paragraph_anchors(1);
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].offset, 0);

    // Formatting part of the text (run-text offsets) splits its run; the
    // reference stays one.
    apply(
        &mut word,
        HeadlessCommand::ApplyBold {
            start_paragraph: 1,
            start_offset: 1,
            end_paragraph: 1,
            end_offset: 3,
            enabled: true,
        },
    );
    let references: Vec<_> = word
        .paragraph_anchors(1)
        .into_iter()
        .map(|a| a.kind)
        .collect();
    assert_eq!(
        references,
        vec![ParagraphAnchorKind::Footnote {
            note_id: "1".into()
        }]
    );
    // And so it does in the file.
    let exported = block_on(WordDocument::load(
        &word.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap();
    assert_eq!(exported.paragraph_anchors(1).len(), 1);
    assert_eq!(exported.paragraph_text_len_at(1), Some(5));
}

#[test]
fn text_typed_after_a_lone_text_box_gets_its_own_run() {
    let word = type_after_a_lone_anchor("textbox.docx", 5);
    let runs = &word.paragraph(1).unwrap().runs;
    assert!(
        runs.iter().all(|run| !anchored(run) || run.text.is_empty()),
        "{runs:?}"
    );
    assert_eq!(word.paragraph_anchors(1).len(), 1);
    let exported = block_on(WordDocument::load(
        &word.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap();
    let runs = &exported.paragraph(1).unwrap().runs;
    assert!(
        runs.iter().all(|run| !anchored(run) || run.text.is_empty()),
        "{runs:?}"
    );
    assert_eq!(exported.paragraph_text_len_at(1), Some(5));
}
