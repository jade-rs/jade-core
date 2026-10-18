//! `delete_text_box` removes a text box — the anchoring run's drawing and
//! everything in the box — named by its paragraph and its place among that
//! paragraph's text boxes, and the file jade writes no longer has it.

use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::word_document::{ParagraphAnchorKind, ParagraphStory, WordDocument};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest, HeadlessShapeKind};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).unwrap()
}

fn load(name: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    load_bytes(&std::fs::read(path).unwrap())
}

fn reload(word: &WordDocument) -> WordDocument {
    load_bytes(&word.export_docx().unwrap())
}

fn apply(word: &mut WordDocument, command: HeadlessCommand) -> Result<(), String> {
    apply_headless_commands_to_document(
        word,
        &HeadlessEditRequest {
            commands: vec![command],
        },
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn text(word: &WordDocument, surface: usize) -> String {
    word.paragraph(surface)
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .unwrap_or_default()
}

/// Every paragraph surface as (story, text, anchors), in jade's order.
fn listing(word: &WordDocument) -> Vec<(Option<ParagraphStory>, String, Vec<String>)> {
    (0..word.paragraph_count())
        .map(|s| {
            let anchors = word
                .paragraph_anchors(s)
                .into_iter()
                .map(|a| match a.kind {
                    ParagraphAnchorKind::TextBox { .. } => format!("{} box", a.offset),
                    ParagraphAnchorKind::Footnote { note_id } => {
                        format!("{} footnote {note_id}", a.offset)
                    }
                    ParagraphAnchorKind::Endnote { note_id } => {
                        format!("{} endnote {note_id}", a.offset)
                    }
                })
                .collect();
            (word.paragraph_story(s), text(word, s), anchors)
        })
        .collect()
}

fn text_boxes(word: &WordDocument) -> usize {
    (0..word.paragraph_count())
        .filter(|&s| {
            matches!(
                word.paragraph_story(s),
                Some(ParagraphStory::TextBox { .. })
            )
        })
        .count()
}

#[test]
fn a_text_box_is_deleted_with_its_content() {
    let mut word = load("textbox.docx");
    assert_eq!(text_boxes(&word), 1);
    apply(
        &mut word,
        HeadlessCommand::DeleteTextBox {
            paragraph: 0,
            text_box: 0,
        },
    )
    .unwrap();
    assert_eq!(text_boxes(&word), 0);
    assert!(word.paragraph_anchors(0).is_empty());
    assert_eq!(text(&word, 0), "Hello");
    let expected = listing(&word);
    let reloaded = reload(&word);
    assert_eq!(listing(&reloaded), expected);
    assert_eq!(text_boxes(&reloaded), 0);
}

#[test]
fn a_text_box_among_note_references_goes_alone() {
    let mut word = load("footnote-table-textbox-order.docx");
    let host = (0..word.paragraph_count())
        .find(|&s| text(&word, s) == "Host after box")
        .unwrap();
    apply(
        &mut word,
        HeadlessCommand::DeleteTextBox {
            paragraph: host,
            text_box: 0,
        },
    )
    .unwrap();
    let anchors: Vec<String> = listing(&word)[host].2.clone();
    assert_eq!(anchors, ["4 footnote 5", "14 footnote 3"]);
    assert_eq!(text_boxes(&word), 0);
    let expected = listing(&word);
    assert_eq!(listing(&reload(&word)), expected);
}

#[test]
fn an_inserted_text_box_can_be_deleted_again() {
    let mut word = load("textbox.docx");
    let before = listing(&reload(&word));
    apply(
        &mut word,
        HeadlessCommand::InsertShape {
            paragraph: 2,
            offset: 2,
            shape: HeadlessShapeKind::Rectangle,
            width_px: 120.0,
            height_px: 60.0,
            fill_color: None,
            border_color: None,
            border_width_px: 1.0,
            text: "boxed".into(),
        },
    )
    .unwrap();
    assert_eq!(text_boxes(&word), 2);
    apply(
        &mut word,
        HeadlessCommand::DeleteTextBox {
            paragraph: 2,
            text_box: 0,
        },
    )
    .unwrap();
    assert_eq!(listing(&reload(&word)), before);
}

#[test]
fn a_missing_text_box_is_an_error() {
    let mut word = load("textbox.docx");
    for (paragraph, text_box) in [(0, 1), (2, 0), (9, 0)] {
        assert!(
            apply(
                &mut word,
                HeadlessCommand::DeleteTextBox {
                    paragraph,
                    text_box
                }
            )
            .is_err(),
            "{paragraph} {text_box}"
        );
    }
    assert_eq!(text_boxes(&word), 1);
}

#[test]
fn delete_text_box_has_a_wire_format() {
    let command: HeadlessCommand =
        serde_json::from_str(r#"{"type":"delete_text_box","paragraph":3,"text_box":1}"#).unwrap();
    assert_eq!(
        command,
        HeadlessCommand::DeleteTextBox {
            paragraph: 3,
            text_box: 1
        }
    );
    assert_eq!(command.name(), "delete_text_box");
}

/// `insert_shape` takes a rendered offset, like every text command: a note
/// reference before it counts its mark. It once used the number as an
/// offset into the runs' text, so a shape after a reference landed one
/// character too far on per reference.
#[test]
fn insert_shape_offsets_are_rendered_offsets() {
    let mut word = load("footnote.docx");
    apply(
        &mut word,
        HeadlessCommand::InsertFootnote {
            paragraph: 0,
            offset: 2,
            text: String::new(),
        },
    )
    .unwrap();
    // "He" ² "llo" ¹: rendered "He2llo1"; offset 4 is after "He2l".
    apply(
        &mut word,
        HeadlessCommand::InsertShape {
            paragraph: 0,
            offset: 4,
            shape: HeadlessShapeKind::Rectangle,
            width_px: 120.0,
            height_px: 60.0,
            fill_color: None,
            border_color: None,
            border_width_px: 1.0,
            text: "boxed".into(),
        },
    )
    .unwrap();
    let anchors: Vec<String> = listing(&word)[0].2.clone();
    assert_eq!(anchors, ["2 footnote 2", "3 box", "5 footnote 1"]);
    let expected = listing(&word);
    assert_eq!(listing(&reload(&word)), expected);
}
