//! ルビ: a ruby put over text, changed and taken off, written as Word
//! writes it and read back.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::{json, Value};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn apply(document: &mut WordDocument, commands: Value) {
    let request: HeadlessEditRequest =
        serde_json::from_value(json!({ "commands": commands })).unwrap();
    apply_headless_commands_to_document(document, &request).unwrap();
}

fn xml(document: &WordDocument) -> String {
    let exported = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

fn text(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect()
}

#[test]
fn a_ruby_is_put_on_changed_and_taken_off() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).unwrap());
    let paragraph = (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .unwrap();
    let before = text(&document, paragraph);
    apply(
        &mut document,
        json!([{ "type": "set_ruby", "paragraph": paragraph, "start_offset": 1,
                 "end_offset": 3, "ruby": { "text": "える", "font": "游明朝" } }]),
    );
    let written = xml(&document);
    assert_eq!(written.matches("<w:ruby>").count(), 1, "{written}");
    assert!(written.contains(r#"<w:rubyAlign w:val="distributeSpace"/>"#));
    assert!(written.contains(">える</w:t>"));
    assert_eq!(text(&document, paragraph), before, "the base text stays");

    // Read back: the base carries the guide.
    let reread = load_bytes(&document.export_docx().unwrap());
    let guides: Vec<String> = reread
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .filter_map(|run| run.properties.as_ref()?.ruby.as_ref())
        .filter(|ruby| !ruby.continuation)
        .map(|ruby| ruby.text.clone())
        .collect();
    assert_eq!(guides, ["える"]);
    assert_eq!(text(&reread, paragraph), before);

    // Changed: one ruby, the new alignment and guide.
    apply(
        &mut document,
        json!([{ "type": "set_ruby", "paragraph": paragraph, "start_offset": 1,
                 "end_offset": 3, "ruby": { "text": "エル", "align": "center", "size": 6 } }]),
    );
    let written = xml(&document);
    assert_eq!(written.matches("<w:ruby>").count(), 1, "{written}");
    assert!(written.contains(r#"<w:rubyAlign w:val="center"/>"#));
    assert!(written.contains(r#"<w:hps w:val="12"/>"#));
    assert!(!written.contains("える"));

    // Taken off.
    apply(
        &mut document,
        json!([{ "type": "set_ruby", "paragraph": paragraph, "start_offset": 0,
                 "end_offset": 4, "ruby": null }]),
    );
    let written = xml(&document);
    assert!(!written.contains("<w:ruby>"));
    assert_eq!(text(&document, paragraph), before);
}
