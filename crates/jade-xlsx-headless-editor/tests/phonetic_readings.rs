//! PHONETIC keeps reading the furigana Excel saved as the workbook is edited:
//! a formula typed later reads it, edits elsewhere keep it, and text typed
//! over a cell drops it — typed here, it has no reading, so PHONETIC gives
//! the text, as Excel does for text typed without an IME.

use jade_xlsx_headless_api::{CellInput, CellKind};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use std::path::PathBuf;

const SHEET: &str = "名簿";

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/furigana-workbook.xlsx"
    ));
    HeadlessWorkbook::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn set(workbook: &mut HeadlessWorkbook, reference: &str, value: CellInput) {
    workbook
        .set_cells(SHEET, &[(reference.to_string(), value)])
        .expect("set");
    let summary = workbook.recalculate().expect("recalculate");
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
}

fn text(workbook: &HeadlessWorkbook, reference: &str) -> String {
    let view = workbook.cell(SHEET, reference).expect("cell");
    assert_eq!(view.kind, CellKind::Text, "{reference}: {view:?}");
    view.text.expect("text")
}

#[test]
fn a_typed_formula_reads_the_saved_furigana() {
    let mut workbook = open();
    set(
        &mut workbook,
        "C1",
        CellInput::Formula("=PHONETIC(A1)".into()),
    );
    assert_eq!(text(&workbook, "C1"), "ヤマダタロウ");
    set(
        &mut workbook,
        "C2",
        CellInput::Formula("=PHONETIC(A1:A2)".into()),
    );
    assert_eq!(text(&workbook, "C2"), "ヤマダタロウサトウハナコ");
    // The saved formulas still read it after the edits.
    assert_eq!(text(&workbook, "B1"), "ヤマダタロウ");
    assert_eq!(text(&workbook, "B3"), "ヤマダタロウサトウハナコabc");
}

#[test]
fn text_typed_over_a_cell_has_no_reading() {
    let mut workbook = open();
    set(&mut workbook, "A1", CellInput::Text("鈴木".into()));
    assert_eq!(text(&workbook, "B1"), "鈴木");
    assert_eq!(text(&workbook, "B2"), "サトウハナコ");
    assert_eq!(text(&workbook, "B3"), "鈴木サトウハナコabc");
}
