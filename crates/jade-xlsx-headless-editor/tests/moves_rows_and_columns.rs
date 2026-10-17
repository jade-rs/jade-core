//! Rows and columns inserted and deleted from a caller with no window.

use jade_xlsx_headless_api::{CellInput, CellKind};
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

fn display(workbook: &HeadlessWorkbook, reference: &str) -> String {
    workbook.cell(SHEET, reference).expect("cell").display
}

fn formula(workbook: &HeadlessWorkbook, reference: &str) -> Option<String> {
    workbook.cell(SHEET, reference).expect("cell").formula
}

#[test]
fn inserting_rows_moves_what_is_below_and_recalculates() {
    let mut workbook = open();
    let summary = workbook.insert_rows(SHEET, 5, 2).expect("insert");
    assert_eq!(summary.axis, "rows");
    assert_eq!(summary.at, "5");
    assert_eq!(summary.count, 2);
    assert!(summary.inserted);

    assert_eq!(display(&workbook, "B4"), "¥12,850,000");
    assert_eq!(display(&workbook, "B5"), "");
    assert_eq!(display(&workbook, "B7"), "¥9,420,000");
    assert_eq!(formula(&workbook, "B9").as_deref(), Some("SUM(B4:B8)"));
    // The total still adds up: recalculation ran over the moved cells.
    assert_eq!(display(&workbook, "B9"), "¥28,400,000");

    // Typing into a new row feeds the grown range.
    workbook
        .set_cells(SHEET, &[("B5".to_string(), CellInput::Number(1_000_000.0))])
        .expect("set");
    assert_eq!(display(&workbook, "B9"), "¥29,400,000");

    // Undo takes the write and then the insertion back.
    assert!(workbook.undo().expect("undo"));
    assert!(workbook.undo().expect("undo"));
    assert_eq!(formula(&workbook, "B7").as_deref(), Some("SUM(B4:B6)"));
    assert_eq!(display(&workbook, "B5"), "¥9,420,000");
}

#[test]
fn deleting_rows_and_columns_shrinks_the_ranges_over_them() {
    let mut workbook = open();
    workbook.delete_rows(SHEET, 5, 1).expect("delete");
    assert_eq!(formula(&workbook, "B6").as_deref(), Some("SUM(B4:B5)"));
    assert_eq!(display(&workbook, "B6"), "¥18,980,000");

    let summary = workbook.delete_columns(SHEET, "B", 1).expect("delete");
    assert_eq!(summary.axis, "columns");
    assert_eq!(summary.at, "B");
    assert!(!summary.inserted);
    assert_eq!(formula(&workbook, "C4").as_deref(), Some("SUM(B4:B4)"));
    assert_eq!(display(&workbook, "C4"), "¥15,230,000");
}

#[test]
fn inserting_columns_widens_a_row_formula_and_the_title_merge() {
    let mut workbook = open();
    workbook.insert_columns(SHEET, "C", 1).expect("insert");
    assert_eq!(formula(&workbook, "E4").as_deref(), Some("SUM(B4:D4)"));
    assert_eq!(display(&workbook, "E4"), "¥28,080,000");
    let merge = workbook.workbook().sheets[0].merges[0];
    assert_eq!(merge.end.column, 4);
}

#[test]
fn bad_positions_are_the_callers_mistake_and_change_nothing() {
    let mut workbook = open();
    for result in [
        workbook.insert_rows(SHEET, 0, 1),
        workbook.insert_rows(SHEET, 5, 0),
        workbook.delete_rows(SHEET, 1_048_576, 2),
        workbook.insert_columns(SHEET, "3", 1),
        workbook.insert_columns("Nope", "A", 1),
    ] {
        let error = result.expect_err("refused");
        assert!(error.is_invalid_input(), "{error}");
    }
    assert!(!workbook.can_undo());
    assert_eq!(
        workbook.cell(SHEET, "B4").expect("cell").kind,
        CellKind::Number
    );
}
