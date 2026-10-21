//! Word's 日付と時刻 with 自動的に更新する: a DATE or TIME field carrying
//! the chosen picture, which reads back as one.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::document::RunFieldKind;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::json;

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

#[test]
fn a_date_field_keeps_its_picture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).unwrap());
    let paragraph = (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .unwrap();
    let request: HeadlessEditRequest = serde_json::from_value(json!({ "commands": [
        { "type": "insert_date_time_field", "start_paragraph": paragraph, "start_offset": 0,
          "end_paragraph": paragraph, "end_offset": 0, "format": "ggge年M月d日",
          "cached_text": "令和8年10月21日" },
        { "type": "insert_date_time_field", "start_paragraph": paragraph, "start_offset": 0,
          "end_paragraph": paragraph, "end_offset": 0, "time": true, "format": "H時m分",
          "cached_text": "9時5分" },
    ]}))
    .unwrap();
    apply_headless_commands_to_document(&mut document, &request).unwrap();
    let exported = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported.clone())).unwrap();
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains(r#"DATE \@ &quot;ggge年M月d日&quot;"#), "{xml}");
    assert!(xml.contains(r#"TIME \@ &quot;H時m分&quot;"#));

    let reread = load_bytes(&exported);
    let kinds: Vec<RunFieldKind> = reread
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .filter_map(|run| {
            run.field
                .as_ref()
                .map(|field| field.kind.base_kind().clone())
        })
        .collect();
    assert!(kinds.contains(&RunFieldKind::Date {
        format: Some("ggge年M月d日".into())
    }));
    assert!(kinds.contains(&RunFieldKind::Time {
        format: Some("H時m分".into())
    }));
}

#[test]
fn an_enclosed_character_is_written_as_word_writes_it() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).unwrap());
    let paragraph = (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .unwrap();
    let request: HeadlessEditRequest = serde_json::from_value(json!({ "commands": [
        { "type": "insert_enclosed_character", "start_paragraph": paragraph, "start_offset": 0,
          "end_paragraph": paragraph, "end_offset": 0, "text": "秘", "enclosure": "○" },
    ]}))
    .unwrap();
    apply_headless_commands_to_document(&mut document, &request).unwrap();
    let exported = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported.clone())).unwrap();
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains(r#"EQ \o\ac(○,秘)"#), "{xml}");
    let reread = load_bytes(&exported);
    assert!(reread.paragraph(paragraph).unwrap().runs.iter().any(|run| {
        run.field.as_ref().is_some_and(|field| {
            field.kind.base_kind()
                == &RunFieldKind::EqEnclosed {
                    text: "秘".into(),
                    enclosure: "○".into(),
                }
        })
    }));
}
