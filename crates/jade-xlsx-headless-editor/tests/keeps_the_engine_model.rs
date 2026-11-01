//! Typing value after value into the same workbook keeps the calculation
//! engine's model between them and sets only the typed values in it; the
//! answers are those of a model built afresh (which every test build checks
//! too), and any other write starts from a fresh model again.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

const SHEET: &str = "売上サマリ";

fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures/sample-workbook.xlsx"
        ))
        .unwrap(),
    )
    .unwrap()
}

fn number(book: &HeadlessWorkbook, reference: &str) -> f64 {
    book.cell(SHEET, reference).unwrap().number.unwrap()
}

#[test]
fn values_typed_one_after_another_recalculate_their_formulas() {
    let mut book = open();
    book.set_cells(
        SHEET,
        &[
            ("H20".into(), CellInput::Number(1.0)),
            ("I20".into(), CellInput::Formula("=H20*2+J20".into())),
            ("J20".into(), CellInput::Number(0.0)),
        ],
    )
    .unwrap();
    // Plain values, one write each: the first builds the model, the rest
    // type into it.
    for value in [2.0, 3.0, 5.0] {
        book.set(SHEET, "H20", CellInput::Number(value)).unwrap();
        assert_eq!(number(&book, "I20"), value * 2.0);
    }
    book.set(SHEET, "J20", CellInput::Number(10.0)).unwrap();
    assert_eq!(number(&book, "I20"), 20.0);
    // Text into a cell the formula reads makes it an error, as in Excel.
    book.set(SHEET, "J20", CellInput::Text("x".into())).unwrap();
    assert_eq!(
        book.cell(SHEET, "I20").unwrap().display,
        "#VALUE!",
        "{:?}",
        book.cell(SHEET, "I20")
    );

    // An undo writes the package back: the next typing builds afresh and
    // still answers right.
    assert!(book.undo().unwrap());
    assert_eq!(number(&book, "I20"), 20.0);
    book.set(SHEET, "H20", CellInput::Number(7.0)).unwrap();
    assert_eq!(number(&book, "I20"), 24.0);
}

/// A workbook without a formula has nothing to recalculate: typed values
/// land without a model, and the first formula typed is answered and then
/// recalculated by the values typed after it.
#[test]
fn a_workbook_without_formulas_types_without_a_model_until_one_is_typed() {
    let mut book = HeadlessWorkbook::open_for_editing(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures/large-workbook.xlsx"
        ))
        .unwrap(),
    )
    .unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    for value in [1.0, 2.0] {
        book.set(&sheet, "B2", CellInput::Number(value)).unwrap();
        assert_eq!(book.cell(&sheet, "B2").unwrap().number, Some(value));
    }
    book.set(&sheet, "Z2", CellInput::Formula("=B2*10".into()))
        .unwrap();
    assert_eq!(book.cell(&sheet, "Z2").unwrap().number, Some(20.0));
    book.set(&sheet, "B2", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(&sheet, "Z2").unwrap().number, Some(30.0));
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(reopened.cell(&sheet, "B2").unwrap().number, Some(3.0));
    assert_eq!(reopened.cell(&sheet, "Z2").unwrap().number, Some(30.0));
}

/// Typing taken back puts back what it overwrote — a value a formula
/// reads, the formula's result, a cell in a row that was not there — as
/// the package it restores reads (which every test build checks), and the
/// write can be redone and taken back again.
#[test]
fn typing_is_taken_back_in_place() {
    let mut book = open();
    book.set_cells(
        SHEET,
        &[
            ("H20".into(), CellInput::Number(1.0)),
            ("I20".into(), CellInput::Formula("=H20*2".into())),
        ],
    )
    .unwrap();
    book.set(SHEET, "H20", CellInput::Number(5.0)).unwrap();
    book.set(SHEET, "Z900", CellInput::Text("new".into()))
        .unwrap();
    assert_eq!(book.cell(SHEET, "Z900").unwrap().display, "new");

    assert!(book.undo().unwrap());
    assert_eq!(book.cell(SHEET, "Z900").unwrap().display, "");
    assert!(book.workbook().sheets[0]
        .rows
        .iter()
        .all(|row| row.index != 899));
    assert!(book.undo().unwrap());
    assert_eq!(number(&book, "H20"), 1.0);
    assert_eq!(number(&book, "I20"), 2.0);

    assert!(book.redo().unwrap());
    assert_eq!(number(&book, "I20"), 10.0);
    assert!(book.redo().unwrap());
    assert_eq!(book.cell(SHEET, "Z900").unwrap().display, "new");
    assert!(book.undo().unwrap());
    assert!(book.undo().unwrap());
    assert_eq!(number(&book, "I20"), 2.0);
    // Typing again after it starts from the model put back.
    book.set(SHEET, "H20", CellInput::Number(4.0)).unwrap();
    assert_eq!(number(&book, "I20"), 8.0);
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(reopened.cell(SHEET, "I20").unwrap().number, Some(8.0));
}

/// A recalculation changes caches without creating a history entry. Returning
/// to the original time zone must not make its old typing reversal applicable
/// to that newer model.
#[test]
fn typing_undo_after_a_time_zone_round_trip_restores_the_package_model() {
    let mut book = open();
    book.set_calculation_time_zone("Etc/GMT+12").unwrap();
    book.set(SHEET, "H20", CellInput::Formula("=TODAY()".into()))
        .unwrap();
    let original_today = number(&book, "H20");
    let before = book.bytes().to_vec();
    book.set(SHEET, "Z900", CellInput::Text("new".into()))
        .unwrap();
    book.set_calculation_time_zone("Pacific/Kiritimati")
        .unwrap();
    book.recalculate().unwrap();
    assert_ne!(number(&book, "H20"), original_today);
    book.set_calculation_time_zone("Etc/GMT+12").unwrap();
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(number(&book, "H20"), number(&reopened, "H20"));
}
