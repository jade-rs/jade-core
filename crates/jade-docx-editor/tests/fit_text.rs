//! Word's 文字の均等割り付け as `set_run_format`'s `fit_text`: the selected
//! characters spread over a width as one `w:fitText` group (one `w:id`),
//! laid out that wide, read back, taken off with `0` (解除), and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest, HeadlessRunFormat};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn document_xml(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document part")
        .read_to_string(&mut xml)
        .expect("read");
    xml
}

fn fit(paragraph: usize, end: usize, width: f32) -> HeadlessEditRequest {
    HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetRunFormat {
            start_paragraph: paragraph,
            start_offset: 0,
            end_paragraph: paragraph,
            end_offset: end,
            format: Box::new(HeadlessRunFormat {
                fit_text: Some(width),
                ..Default::default()
            }),
        }],
    }
}

/// The widths of the `w:fitText` runs of `paragraph` in twips, with their ids.
fn fitted(document: &WordDocument, paragraph: usize) -> Vec<(u32, Option<i32>)> {
    document
        .paragraph(paragraph)
        .expect("paragraph")
        .runs
        .iter()
        .filter_map(|run| run.properties.as_ref()?.fit_text.as_ref())
        .map(|fit| (fit.val, fit.id))
        .collect()
}

#[test]
fn selected_text_is_spread_over_a_width_read_back_and_taken_off() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).expect("fixture"));
    let paragraph = (0..document.paragraph_count())
        .find(|&index| {
            document.is_body_paragraph_surface(index)
                && document.paragraph(index).is_some_and(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.len())
                        .sum::<usize>()
                        >= 4
                })
        })
        .expect("a paragraph with text");
    let text: String = document
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    let end: usize = text.chars().take(4).map(char::len_utf8).sum();
    let before = document_xml(&document);
    let mut executor = CommandExecutor::new();

    // Four characters over 63 pt (six 10.5 pt 字).
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &fit(paragraph, end, 63.0),
    )
    .expect("apply");
    let groups = fitted(&document, paragraph);
    assert!(!groups.is_empty());
    assert!(groups.iter().all(|(val, _)| *val == 1260), "{groups:?}");
    let id = groups[0].1.expect("a group id");
    assert!(
        groups.iter().all(|(_, other)| *other == Some(id)),
        "one group"
    );
    let xml = document_xml(&document);
    assert!(
        xml.contains(&format!(r#"<w:fitText w:id="{id}" w:val="1260"/>"#)),
        "{xml}"
    );

    // Saved and opened again, the group is the same.
    let reopened = load_bytes(&document.export_docx().expect("export"));
    assert_eq!(fitted(&reopened, paragraph), groups);

    // 0 takes it off (解除).
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &fit(paragraph, end, 0.0),
    )
    .expect("apply");
    assert!(fitted(&document, paragraph).is_empty());
    assert!(!document_xml(&document).contains("w:fitText"));

    let mut selection = None;
    {
        let mut context = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut context).expect("undo");
    }
    assert_eq!(fitted(&document, paragraph), groups);
    {
        let mut context = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut context).expect("undo");
    }
    assert!(fitted(&document, paragraph).is_empty());
    assert_eq!(document_xml(&document), before);
}
