//! `insert_text_among_anchors` types where zero-width runs (text boxes and
//! note references) sit at one offset, after a chosen number of them.
//! `insert_text` always types before them (or, at a paragraph's start,
//! after them), so text between two such runs could not be written at all —
//! an order collaborative editing produces when text typed at a caret meets
//! a concurrent deletion that brings two anchors together.

use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::selection::TextPosition;
use jade_docx_document_core::word_document::{ParagraphAnchorKind, WordDocument};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;

fn load_bytes(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).unwrap()
}

fn load(name: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    load_bytes(&std::fs::read(path).unwrap())
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

fn rendered(word: &WordDocument, surface: usize, offset: usize) -> usize {
    word.model_to_rendered_text_position(TextPosition::new(surface, offset))
        .offset
}

/// The paragraph's text with its anchors written in: `[n]` for note n,
/// `[box]` for a text box.
fn layout(word: &WordDocument, surface: usize) -> String {
    let text: String = word
        .paragraph(surface)
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .unwrap_or_default();
    let mut out = String::new();
    let mut at = 0;
    for anchor in word.paragraph_anchors(surface) {
        out.push_str(&text[at..anchor.offset]);
        at = anchor.offset;
        out.push_str(&match anchor.kind {
            ParagraphAnchorKind::TextBox { .. } => "[box]".to_string(),
            ParagraphAnchorKind::Footnote { note_id }
            | ParagraphAnchorKind::Endnote { note_id } => {
                format!("[{note_id}]")
            }
        });
    }
    out.push_str(&text[at..]);
    out
}

fn reload(word: &WordDocument) -> WordDocument {
    load_bytes(&word.export_docx().unwrap())
}

fn among(
    word: &mut WordDocument,
    surface: usize,
    offset: usize,
    after: usize,
    text: &str,
) -> Result<(), String> {
    let at = rendered(word, surface, offset);
    apply(
        word,
        HeadlessCommand::InsertTextAmongAnchors {
            paragraph: surface,
            offset: at,
            after_anchors: after,
            text: text.into(),
        },
    )
}

/// footnote.docx's "Hello" ¹ with a second reference in front of the first.
fn two_references() -> WordDocument {
    let mut word = load("footnote.docx");
    apply(
        &mut word,
        HeadlessCommand::InsertFootnote {
            paragraph: 0,
            offset: 5,
            text: String::new(),
        },
    )
    .unwrap();
    assert_eq!(layout(&word, 0), "Hello[2][1]");
    word
}

#[test]
fn text_goes_after_the_named_number_of_anchors() {
    for (after, expected) in [
        (0, "Hellox[2][1]"),
        (1, "Hello[2]x[1]"),
        (2, "Hello[2][1]x"),
    ] {
        let mut word = two_references();
        among(&mut word, 0, 5, after, "x").unwrap();
        assert_eq!(layout(&word, 0), expected);
        assert_eq!(layout(&reload(&word), 0), expected, "after reloading");
    }
}

#[test]
fn text_between_anchors_is_formatted_like_the_text_before_it() {
    let mut word = two_references();
    among(&mut word, 0, 5, 1, "x").unwrap();
    let runs = &word.paragraph(0).unwrap().runs;
    let hello = runs.iter().find(|r| r.text.contains("Hello")).unwrap();
    let typed = runs.iter().find(|r| r.text == "x").unwrap();
    assert_eq!(typed.properties, hello.properties);
    assert!(typed.footnote_reference_ids.is_empty());
}

#[test]
fn text_goes_between_a_reference_and_a_text_box() {
    let mut word = load("textbox.docx");
    apply(
        &mut word,
        HeadlessCommand::InsertFootnote {
            paragraph: 0,
            offset: 5,
            text: String::new(),
        },
    )
    .unwrap();
    assert_eq!(layout(&word, 0), "Hello[2][box]");
    among(&mut word, 0, 5, 1, "x").unwrap();
    assert_eq!(layout(&word, 0), "Hello[2]x[box]");
    assert_eq!(layout(&reload(&word), 0), "Hello[2]x[box]");
}

#[test]
fn more_anchors_than_there_are_is_an_error() {
    let mut word = two_references();
    assert!(among(&mut word, 0, 5, 3, "x").is_err());
    assert!(among(&mut word, 0, 2, 1, "x").is_err());
    assert_eq!(layout(&word, 0), "Hello[2][1]");
}

#[test]
fn insert_text_among_anchors_has_a_wire_format() {
    let command: HeadlessCommand = serde_json::from_str(
        r#"{"type":"insert_text_among_anchors","paragraph":0,"offset":5,"after_anchors":1,"text":"x"}"#,
    )
    .unwrap();
    assert_eq!(
        command,
        HeadlessCommand::InsertTextAmongAnchors {
            paragraph: 0,
            offset: 5,
            after_anchors: 1,
            text: "x".into()
        }
    );
    assert_eq!(command.name(), "insert_text_among_anchors");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Wherever text goes among the anchors at an offset, the paragraph is
    /// the old one with the text spliced in after that many anchors, in
    /// memory and in the file.
    #[test]
    fn text_is_spliced_in_after_the_named_anchors(
        notes in 1..4usize,
        after in 0..5usize,
        text in "[a-z日]{1,3}",
    ) {
        let mut word = load("footnote.docx");
        for _ in 1..notes {
            apply(&mut word, HeadlessCommand::InsertFootnote { paragraph: 0, offset: 5, text: String::new() }).unwrap();
        }
        let before = layout(&word, 0);
        let result = among(&mut word, 0, 5, after, &text);
        if after > notes {
            prop_assert!(result.is_err());
            prop_assert_eq!(layout(&word, 0), before);
        } else {
            result.unwrap();
            // Splice into "Hello[a][b]…" after `after` references.
            let mut cut = "Hello".len();
            for _ in 0..after {
                cut += before[cut..].find(']').unwrap() + 1;
            }
            let expected = format!("{}{}{}", &before[..cut], text, &before[cut..]);
            prop_assert_eq!(layout(&word, 0), expected.clone());
            prop_assert_eq!(layout(&reload(&word), 0), expected);
        }
    }
}

/// The text is plain, whatever the run it takes its formatting from: a
/// tracked deletion before it once lent the text its revision, so the
/// text showed as deleted (and a later untracked deletion left it there).
#[test]
fn text_among_anchors_is_not_part_of_a_neighbouring_revision() {
    let mut word = load("footnote.docx");
    apply(
        &mut word,
        HeadlessCommand::StartChangeTracking {
            author: Some("Ann".into()),
            initials: None,
            date: None,
            date_utc: None,
        },
    )
    .unwrap();
    let (start, end) = (rendered(&word, 0, 4), rendered(&word, 0, 5));
    apply(
        &mut word,
        HeadlessCommand::ReplaceRange {
            start_paragraph: 0,
            start_offset: start,
            end_paragraph: 0,
            end_offset: end,
            text: String::new(),
        },
    )
    .unwrap();
    apply(&mut word, HeadlessCommand::StopChangeTracking).unwrap();
    among(&mut word, 0, 5, 1, "x").unwrap();
    assert_eq!(layout(&word, 0), "Hello[1]x");
    let typed = word
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .find(|r| r.text == "x")
        .unwrap();
    assert!(typed.revision.is_none(), "{:?}", typed.revision);
    // And an untracked deletion takes it.
    let at = rendered(&word, 0, 5) + 1;
    apply(
        &mut word,
        HeadlessCommand::ReplaceRange {
            start_paragraph: 0,
            start_offset: at,
            end_paragraph: 0,
            end_offset: at + 1,
            text: String::new(),
        },
    )
    .unwrap();
    assert_eq!(layout(&word, 0), "Hello[1]");
    assert_eq!(layout(&reload(&word), 0), "Hello[1]");
}
