//! Page-break insertion against DOCX files saved before and after the action in Windows Word.
mod common;

use common::{open_bytes, repo_path};
use jade_docx_editor::commands::insert_manual_page_break::InsertManualPageBreakCommand;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use zip::ZipArchive;

fn capture_file(capture: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    capture
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

fn texts(document: &WordDocument) -> Vec<String> {
    (0..document.paragraph_count())
        .map(|paragraph| {
            document
                .paragraph(paragraph)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect()
        })
        .collect()
}

#[test]
fn inserting_page_break_matches_word_boundaries_and_round_trips() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let capture = std::fs::read(repo_path(
        "tests/word-oracle/windows/manual-page-breaks/word-16.0.19127.20936.zip",
    ))
    .unwrap();
    let mut originals = ZipArchive::new(Cursor::new(capture)).unwrap();
    let manifest = capture_file(&mut originals, "word-insert/capture.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(manifest.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&manifest))
            .unwrap();
    assert_eq!(manifest["product_version"], "16.0.19127.20936");
    let cases = manifest["documents"].as_array().unwrap();
    assert_eq!(cases.len(), 11);

    let pos = TextPosition::new;
    let caret = SelectionRange::collapsed;
    for (name, selection, after) in [
        ("start", caret(pos(0, 0)), pos(1, 0)),
        ("middle", caret(pos(0, 6)), pos(2, 0)),
        ("end", caret(pos(0, 11)), pos(2, 0)),
        ("document-end", caret(pos(0, 11)), pos(2, 0)),
        ("empty", caret(pos(0, 0)), pos(1, 0)),
        ("styled", caret(pos(0, 6)), pos(2, 0)),
        ("list", caret(pos(0, 6)), pos(2, 0)),
        (
            "replace",
            SelectionRange {
                anchor: pos(0, 2),
                focus: pos(0, 8),
            },
            pos(2, 0),
        ),
        (
            "cross-paragraph",
            SelectionRange {
                anchor: pos(0, 6),
                focus: pos(1, 2),
            },
            pos(2, 0),
        ),
        ("table", caret(pos(0, 6)), pos(0, 1)),
        (
            "table-selected",
            SelectionRange {
                anchor: pos(0, 2),
                focus: pos(0, 8),
            },
            pos(0, 1),
        ),
    ] {
        let case = cases.iter().find(|entry| entry["name"] == name).unwrap();
        let source = capture_file(&mut originals, &format!("word-insert/inputs/{name}.docx"));
        let original = capture_file(&mut originals, &format!("word-insert/{name}.docx"));
        assert_eq!(
            case["input_sha256"],
            format!("{:x}", Sha256::digest(&source))
        );
        assert_eq!(
            case["output_sha256"],
            format!("{:x}", Sha256::digest(&original))
        );
        let pages = case["page_count"].as_u64().unwrap() as usize;
        let expected =
            futures::executor::block_on(WordDocument::load(&original, &Options::default()))
                .expect("open original Word output");
        let expected_texts = texts(&expected);
        let word_position: usize = expected_texts[..after.paragraph]
            .iter()
            .map(|text| text.len() + 1)
            .sum::<usize>()
            + after.offset;
        assert_eq!(
            case["caret_after"].as_u64().unwrap() as usize,
            word_position,
            "{name}"
        );
        let mut document = open_bytes(&source);
        let before = texts(document.word_document());
        document.set_selection_range(Some(selection));
        document
            .execute_command(
                Box::new(InsertManualPageBreakCommand::new(selection)),
                false,
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(texts(document.word_document()), expected_texts, "{name}");
        assert_eq!(document.selection_range(), Some(caret(after)), "{name}");
        assert_eq!(document.page_count(), pages, "{name}");
        let exported = document.export_docx().unwrap();
        let reopened = open_bytes(&exported);
        assert_eq!(texts(reopened.word_document()), expected_texts, "{name}");
        assert_eq!(reopened.page_count(), pages, "{name}");
        document.undo().unwrap();
        assert_eq!(texts(document.word_document()), before, "{name}");
        assert_eq!(document.selection_range(), Some(selection), "{name}");
        document.redo().unwrap();
        assert_eq!(texts(document.word_document()), expected_texts, "{name}");
        assert_eq!(document.selection_range(), Some(caret(after)), "{name}");
    }
}
