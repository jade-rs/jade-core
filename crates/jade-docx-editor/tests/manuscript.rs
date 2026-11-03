//! Word's 原稿用紙設定 drawing: the `Genko:` VML group of squares, lines or a
//! ruled frame written into the header, read back from it, taken away with
//! なし, and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_reader::document::{ManuscriptGrid, ManuscriptStyle};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

/// Every header part of the saved package, joined.
fn headers(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut all = String::new();
    for index in 1..10 {
        if let Ok(mut file) = archive.by_name(&format!("word/header{index}.xml")) {
            file.read_to_string(&mut all).expect("read");
        }
    }
    all
}

fn grid_command(style: Option<&str>, chars: u32, lines: u32) -> HeadlessEditRequest {
    HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetManuscriptGrid {
            style: style.map(str::to_string),
            chars,
            lines,
            color: "#00b050".to_string(),
        }],
    }
}

fn undo(document: &mut WordDocument, executor: &mut CommandExecutor) {
    let mut selection = None;
    let mut context = CommandContext::new(document, &mut selection);
    executor.undo(&mut context).expect("undo");
}

#[test]
fn a_manuscript_grid_is_written_read_back_removed_and_undone() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).expect("fixture"));
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());
    let before = headers(&document);
    let mut executor = CommandExecutor::new();

    // マス目付き 20 × 20: the frame and 400 squares in the header.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("squares"), 20, 20),
    )
    .expect("apply");
    let xml = headers(&document);
    assert!(xml.contains(r#"id="Genko:A4:20:20:P:0::""#), "{xml}");
    assert_eq!(xml.matches("<v:rect ").count(), 401);
    assert!(xml.contains(r##"strokecolor="#00B050""##));
    assert_eq!(
        document.document.as_deref().unwrap().manuscript_grid,
        Some(ManuscriptGrid {
            style: ManuscriptStyle::Squares,
            chars: 20,
            lines: 20,
            color: "#00B050".to_string(),
        })
    );

    // Saved and opened again, the grid is read back from the drawing.
    let reopened = load_bytes(&document.export_docx().expect("export"));
    let read = reopened
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .clone();
    assert_eq!(
        read.as_ref().map(|grid| grid.style),
        Some(ManuscriptStyle::Squares)
    );
    assert_eq!(
        read.as_ref().map(|grid| (grid.chars, grid.lines)),
        Some((20, 20))
    );

    // 下線付き 20 × 10 replaces it: ten rules, no squares.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("lines"), 20, 10),
    )
    .expect("apply");
    let xml = headers(&document);
    assert_eq!(xml.matches("Genko:").count(), 1, "one grid, not two");
    assert_eq!(xml.matches("<v:line ").count(), 10);
    assert_eq!(xml.matches("<v:rect ").count(), 0);
    let reopened = load_bytes(&document.export_docx().expect("export"));
    assert_eq!(
        reopened
            .document
            .as_deref()
            .unwrap()
            .manuscript_grid
            .as_ref()
            .map(|grid| grid.style),
        Some(ManuscriptStyle::Lines)
    );

    // なし takes it away.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(None, 20, 10),
    )
    .expect("apply");
    assert!(!headers(&document).contains("Genko:"));
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());

    // A style it does not know is refused.
    assert!(apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("dots"), 20, 10),
    )
    .is_err());

    undo(&mut document, &mut executor);
    assert!(headers(&document).contains("<v:line "));
    undo(&mut document, &mut executor);
    undo(&mut document, &mut executor);
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());
    assert!(!headers(&document).contains("Genko:"));
    let _ = before;
}
