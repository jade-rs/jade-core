use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::CellRef;
fn fixture() -> &'static [u8] {
    include_bytes!("../../../public/fixtures/x14-bars-workbook.xlsx")
}
fn bar(book: &HeadlessWorkbook, name: &str) -> jade_xlsx_layout::PlacedDataBar {
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    layout
        .cell(CellRef::parse(name).unwrap())
        .unwrap()
        .data_bar
        .clone()
        .unwrap()
}
fn export(book: &HeadlessWorkbook, name: &str) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(name),
            book.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn edits_recompute_signed_geometry_with_exact_history_and_save_reopen() {
    let mut book = HeadlessWorkbook::open_for_editing(fixture()).unwrap();
    let status = book.conditional_formatting_status();
    assert_eq!((status.rule_count, status.supported_rules), (16, 16));
    assert!(status.unsupported.is_empty());
    let before = book.bytes().to_vec();
    let old = bar(&book, "A2");
    let metadata = book.workbook().sheets[0].conditional_formats.clone();
    book.set("Modern", "A5", CellInput::Number(100.0)).unwrap();
    let new = bar(&book, "A2");
    assert!(new.rect.width > old.rect.width);
    assert_eq!(new.color, old.color);
    assert_ne!(new.axis, old.axis);
    assert_eq!(metadata, book.workbook().sheets[0].conditional_formats);
    let after = book.bytes().to_vec();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.bytes(), after);
    let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
    assert_eq!(bar(&reopened, "A2"), new);
    assert_eq!(reopened.conditional_formatting_status().supported_rules, 16);
    export(&book, "x14-bars-edited.xlsx");
}
#[test]
fn structural_edits_keep_standard_and_xm_ranges_linked_and_restore_on_undo() {
    let mut book = HeadlessWorkbook::open_for_editing(fixture()).unwrap();
    let before = book.bytes().to_vec();
    book.insert_rows("Modern", 2, 1).unwrap();
    assert!(book.conditional_formatting_status().unsupported.is_empty());
    assert_eq!(bar(&book, "A4").rect.width, 0.0);
    book.insert_columns("Modern", "B", 1).unwrap();
    assert!(book.conditional_formatting_status().unsupported.is_empty());
    assert!(bar(&book, "C1").border.is_some());
    let after = book.export_bytes().unwrap();
    let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
    assert_eq!(reopened.conditional_formatting_status().supported_rules, 16);
    export(&book, "x14-bars-structural.xlsx");
    book.undo().unwrap();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}
