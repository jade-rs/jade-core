//! 参考資料 in the engine: notes deleted with their reference marks, and
//! the commands the reference dialogs drive.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::CommandExecutor;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::{json, Value};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn hello() -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    load_bytes(&std::fs::read(path).unwrap())
}

fn apply(document: &mut WordDocument, executor: &mut CommandExecutor, commands: Value) {
    let request: HeadlessEditRequest =
        serde_json::from_value(json!({ "commands": commands })).unwrap();
    apply_headless_commands_with_executor_to_document(document, executor, None, &request)
        .unwrap();
}

#[allow(dead_code)]
fn part(document: &WordDocument, name: &str) -> String {
    let exported = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
    let mut xml = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

fn body_paragraph(document: &WordDocument) -> usize {
    (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .unwrap()
}

/// Where each reference mark of a paragraph is drawn.
fn marks(document: &WordDocument, paragraph: usize) -> Vec<(String, usize)> {
    let mut rendered = 0;
    document
        .reference_marker_spans(paragraph)
        .into_iter()
        .map(|span| {
            let at = span.model_offset + rendered;
            rendered += span.rendered_len;
            (span.note_id, at)
        })
        .collect()
}

#[test]
fn a_selection_over_a_reference_mark_deletes_the_note() {
    let mut document = hello();
    let mut executor = CommandExecutor::new();
    let paragraph = body_paragraph(&document);
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "insert_footnote", "paragraph": paragraph, "offset": 2, "text": "kept" }]),
    );
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "insert_footnote", "paragraph": paragraph, "offset": 1, "text": "gone" }]),
    );
    let before = document.footnote_infos().len();
    let (_, at) = marks(&document, paragraph)[0].clone();
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "replace_range", "start_paragraph": paragraph, "start_offset": at,
                 "end_paragraph": paragraph, "end_offset": at + 1, "text": "" }]),
    );
    let notes: Vec<String> = document.footnote_infos().into_iter().map(|n| n.text).collect();
    assert_eq!(notes.len(), before - 1);
    assert!(notes.iter().all(|text| text != "gone"), "{notes:?}");
    assert!(notes.iter().any(|text| text == "kept"));
    assert_eq!(marks(&document, paragraph).len(), 1);

    let mut selection = None;
    let mut context =
        jade_docx_editor::commands::CommandContext::new(&mut document, &mut selection);
    executor.undo(&mut context).unwrap();
    assert_eq!(document.footnote_infos().len(), before);
}
