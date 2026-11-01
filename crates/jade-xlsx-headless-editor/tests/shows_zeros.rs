//! Excel's "Show a zero in cells that have zero value", kept with the sheet.

use jade_xlsx_headless_editor::HeadlessWorkbook;
use std::path::PathBuf;

const SHEET: &str = "売上サマリ";

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    HeadlessWorkbook::open(&bytes).expect("open")
}

#[test]
fn hiding_zeros_is_saved_and_undone_in_one_step() {
    let mut workbook = open();
    assert!(workbook.workbook().sheets[0].show_zeros);
    assert!(workbook.set_show_zeros(SHEET, false).expect("hide"));
    assert!(!workbook.workbook().sheets[0].show_zeros);
    assert!(!workbook.set_show_zeros(SHEET, false).expect("already"));
    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    assert!(!reopened.workbook().sheets[0].show_zeros);
    assert!(workbook.undo().expect("undo"));
    assert!(workbook.workbook().sheets[0].show_zeros);
}
