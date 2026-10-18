//! A text box's paragraphs can be split and joined like any others, and the
//! file jade writes keeps them: before, a split or a join inside a text box
//! changed only the in-memory model, and the exported `w:txbxContent` still
//! held the original paragraphs.

use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::selection::TextPosition;
use jade_docx_document_core::word_document::{ParagraphStory, WordDocument};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;

const FIXTURES: &[&str] = &[
    "textbox.docx",
    "footnote-table-textbox-order.docx",
    "textbox-body-properties.docx",
    "textbox-dashed.docx",
];

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

/// Every paragraph surface as (story, text), in jade's order.
fn listing(word: &WordDocument) -> Vec<(Option<ParagraphStory>, String)> {
    (0..word.paragraph_count())
        .map(|s| (word.paragraph_story(s), text(word, s)))
        .collect()
}

/// The surfaces of the text boxes' paragraphs.
fn text_box_surfaces(word: &WordDocument) -> Vec<usize> {
    (0..word.paragraph_count())
        .filter(|&s| {
            matches!(
                word.paragraph_story(s),
                Some(ParagraphStory::TextBox { .. })
            )
        })
        .collect()
}

fn rendered(word: &WordDocument, surface: usize, offset: usize) -> usize {
    word.model_to_rendered_text_position(TextPosition::new(surface, offset))
        .offset
}

fn split(word: &mut WordDocument, surface: usize, offset: usize) {
    let at = rendered(word, surface, offset);
    apply(
        word,
        HeadlessCommand::InsertParagraphBreak {
            paragraph: surface,
            offset: at,
            count: 1,
        },
    )
    .unwrap();
}

/// Joins the paragraph at `surface` with the next one.
fn join(word: &mut WordDocument, surface: usize) {
    let end = rendered(word, surface, text(word, surface).len());
    let start = rendered(word, surface + 1, 0);
    apply(
        word,
        HeadlessCommand::ReplaceRange {
            start_paragraph: surface,
            start_offset: end,
            end_paragraph: surface + 1,
            end_offset: start,
            text: String::new(),
        },
    )
    .unwrap();
}

#[test]
fn a_split_text_box_paragraph_is_written_to_the_file() {
    let mut word = load("textbox.docx");
    assert_eq!(
        listing(&word)
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>(),
        ["Hello", "Hello", "World"]
    );
    split(&mut word, 1, 2);
    apply(
        &mut word,
        HeadlessCommand::InsertText {
            paragraph: 2,
            offset: 3,
            text: " there".into(),
        },
    )
    .unwrap();
    let expected = listing(&word);
    assert_eq!(
        expected.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(),
        ["Hello", "He", "llo there", "World"]
    );
    assert_eq!(
        expected[2].0,
        Some(ParagraphStory::TextBox {
            host: 0,
            text_box: 0,
            paragraph: 1
        })
    );
    let reloaded = reload(&word);
    assert_eq!(listing(&reloaded), expected);
    // Both the DrawingML box and its VML fallback hold the new paragraph.
    let xml =
        String::from_utf8(zip_part(&word.export_docx().unwrap(), "word/document.xml").unwrap())
            .unwrap();
    for content in xml.split("<w:txbxContent").skip(1) {
        let content = &content[..content.find("</w:txbxContent>").unwrap()];
        assert_eq!(
            content.matches("<w:p>").count() + content.matches("<w:p ").count(),
            2,
            "{content}"
        );
    }
}

#[test]
fn joined_text_box_paragraphs_are_written_to_the_file() {
    let mut word = load("textbox.docx");
    split(&mut word, 1, 2);
    split(&mut word, 2, 1);
    assert_eq!(text(&word, 3), "lo");
    join(&mut word, 1);
    assert_eq!(text(&word, 1), "Hel");
    let expected = listing(&word);
    assert_eq!(listing(&reload(&word)), expected);
    join(&mut word, 1);
    assert_eq!(text(&word, 1), "Hello");
    let expected = listing(&word);
    assert_eq!(listing(&reload(&word)), expected);
}

fn zip_part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    file.read_to_end(&mut out).ok()?;
    Some(out)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random splits, joins and typing inside text boxes: the file jade
    /// writes reads back as the document jade holds.
    #[test]
    fn text_box_paragraph_edits_survive_export(
        fixture in prop::sample::select(FIXTURES.to_vec()),
        steps in prop::collection::vec((0..3u8, any::<prop::sample::Index>(), any::<prop::sample::Index>()), 1..12),
    ) {
        let mut word = load(fixture);
        for (kind, which, at) in steps {
            let surfaces = text_box_surfaces(&word);
            if surfaces.is_empty() {
                break;
            }
            let surface = *which.get(&surfaces);
            let body = text(&word, surface);
            let boundaries: Vec<usize> = body
                .char_indices()
                .map(|(i, _)| i)
                .chain([body.len()])
                .collect();
            let offset = *at.get(&boundaries);
            match kind {
                0 => split(&mut word, surface, offset),
                1 => {
                    let at = rendered(&word, surface, offset);
                    apply(&mut word, HeadlessCommand::InsertText {
                        paragraph: surface,
                        offset: at,
                        text: "xy".into(),
                    }).unwrap();
                }
                _ => {
                    // Only within one text box story.
                    let (Some(ParagraphStory::TextBox { host, text_box, paragraph }), Some(next)) =
                        (word.paragraph_story(surface), word.paragraph_story(surface + 1))
                    else { continue };
                    if next != (ParagraphStory::TextBox { host, text_box, paragraph: paragraph + 1 }) {
                        continue;
                    }
                    join(&mut word, surface);
                }
            }
            let expected = listing(&word);
            prop_assert_eq!(listing(&reload(&word)), expected);
        }
    }
}
