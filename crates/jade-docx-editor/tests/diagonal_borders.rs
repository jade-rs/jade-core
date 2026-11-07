//! Word's 斜め罫線: a cell's diagonals (`w:tl2br` 右下がり, `w:tr2bl` 右上がり)
//! written at the end of its `w:tcBorders` as Word writes them, kept when
//! the cell's other edges change, read back, taken away with style `none`,
//! and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::CommandExecutor;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessBorderEdge, HeadlessCommand, HeadlessEditRequest};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn body(document: &WordDocument) -> String {
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

fn apply(document: &mut WordDocument, executor: &mut CommandExecutor, command: HeadlessCommand) {
    let request = HeadlessEditRequest {
        commands: vec![command],
    };
    apply_headless_commands_with_executor_to_document(document, executor, None, &request)
        .expect("apply");
}

fn edge(style: &str, width: Option<f32>, color: Option<&str>) -> Option<HeadlessBorderEdge> {
    Some(HeadlessBorderEdge {
        style: Some(style.to_string()),
        width,
        color: color.map(str::to_string),
    })
}

fn cell_borders(
    row: usize,
    column: usize,
    top: Option<HeadlessBorderEdge>,
    tl2br: Option<HeadlessBorderEdge>,
    tr2bl: Option<HeadlessBorderEdge>,
) -> HeadlessCommand {
    HeadlessCommand::SetTableCellBorders {
        table_order_index: 0,
        row,
        column,
        top,
        bottom: None,
        left: None,
        right: None,
        tl2br,
        tr2bl,
    }
}

/// The first `w:tcBorders` of the table's first cell.
fn first_cell_borders(xml: &str) -> String {
    let start = xml.find("<w:tc>").expect("a cell");
    let cell = &xml[start..];
    let end = cell.find("</w:tc>").expect("cell end");
    let cell = &cell[..end];
    match (cell.find("<w:tcBorders>"), cell.find("</w:tcBorders>")) {
        (Some(from), Some(to)) => cell[from..to].to_string(),
        _ => String::new(),
    }
}

#[test]
fn a_cells_diagonals_are_written_kept_read_back_and_removed() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).expect("fixture"));
    let mut executor = CommandExecutor::new();
    apply(
        &mut document,
        &mut executor,
        HeadlessCommand::InsertTable {
            after_paragraph: 0,
            rows: 2,
            columns: 2,
            column_widths: Vec::new(),
            caret_offset: None,
        },
    );
    let before = body(&document);

    // 右下がり in red, half a point.
    apply(
        &mut document,
        &mut executor,
        cell_borders(0, 0, None, edge("single", Some(0.5), Some("#FF0000")), None),
    );
    let borders = first_cell_borders(&body(&document));
    assert!(
        borders.contains(r#"<w:tl2br w:val="single" w:sz="4" w:space="0" w:color="FF0000"/>"#),
        "{borders}"
    );
    assert!(!borders.contains("w:tr2bl"), "{borders}");

    // 右上がり too, and a top edge: the first diagonal stays, both at the end.
    apply(
        &mut document,
        &mut executor,
        cell_borders(
            0,
            0,
            edge("double", Some(1.5), None),
            None,
            edge("single", Some(1.0), None),
        ),
    );
    let borders = first_cell_borders(&body(&document));
    let (top, tl2br, tr2bl) = (
        borders.find("<w:top ").expect("top"),
        borders.find("<w:tl2br ").expect("tl2br kept"),
        borders.find("<w:tr2bl ").expect("tr2bl"),
    );
    assert!(top < tl2br && tl2br < tr2bl, "schema order: {borders}");
    assert!(borders.contains(r#"<w:tr2bl w:val="single" w:sz="8" w:space="0" w:color="auto"/>"#));

    // Saved and opened again, the cell still has both diagonals.
    let reopened = load_bytes(&document.export_docx().expect("export"));
    let reopened_borders = first_cell_borders(&body(&reopened));
    assert!(reopened_borders.contains("<w:tl2br ") && reopened_borders.contains("<w:tr2bl "));
    let table = reopened.table_by_order(0).expect("table");
    let cell = &table.rows[0].cells[0];
    assert_eq!(
        cell.style.get("border-tl2br-style").map(String::as_str),
        Some("single")
    );
    assert_eq!(
        cell.style.get("border-tr2bl-style").map(String::as_str),
        Some("single")
    );

    // Style none takes one away; the other and the top stay.
    apply(
        &mut document,
        &mut executor,
        cell_borders(0, 0, None, edge("none", None, None), None),
    );
    let borders = first_cell_borders(&body(&document));
    assert!(!borders.contains("w:tl2br"), "{borders}");
    assert!(borders.contains("<w:tr2bl ") && borders.contains("<w:top "));

    // Undo, step by step, back to the plain table.
    for _ in 0..3 {
        let mut selection = None;
        let mut ctx =
            jade_docx_editor::commands::CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).expect("undo");
    }
    assert_eq!(body(&document), before);
}
