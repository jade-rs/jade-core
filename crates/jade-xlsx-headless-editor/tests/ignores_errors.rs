//! Ignore Error: a cell's background error check turned off and kept with
//! the sheet, following the cell when rows move.

use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRange;
use jade_xlsx_writer::ignored_errors::ErrorCheck;
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

fn ignored(workbook: &HeadlessWorkbook) -> Vec<CellRange> {
    workbook.workbook().sheets[0]
        .ignored_errors
        .iter()
        .flat_map(|entry| entry.ranges.clone())
        .collect()
}

#[test]
fn ignoring_an_error_saves_it_and_moves_it_with_the_rows() {
    let mut workbook = open();
    let g20 = CellRange::parse("G20").unwrap();
    assert!(workbook
        .ignore_error(SHEET, g20, ErrorCheck::NumberStoredAsText)
        .expect("ignore"));
    assert_eq!(ignored(&workbook), vec![g20]);
    let entry = &workbook.workbook().sheets[0].ignored_errors[0];
    assert!(entry.number_stored_as_text && !entry.eval_error);
    // Already ignored there: nothing to do.
    assert!(!workbook
        .ignore_error(SHEET, g20, ErrorCheck::NumberStoredAsText)
        .expect("ignore"));

    // Two rows above push it down; deleting its row takes the list away.
    workbook.insert_rows(SHEET, 5, 2).expect("insert");
    assert_eq!(ignored(&workbook), vec![CellRange::parse("G22").unwrap()]);
    workbook.delete_rows(SHEET, 22, 1).expect("delete");
    assert!(ignored(&workbook).is_empty());
    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    assert!(ignored(&reopened).is_empty());

    // Undo brings it back where it was.
    assert!(workbook.undo().expect("undo"));
    assert!(workbook.undo().expect("undo"));
    assert_eq!(ignored(&workbook), vec![g20]);
}
