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
