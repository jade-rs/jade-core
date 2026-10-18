//! Dynamic arrays edit as in Excel: a typed `=SEQUENCE(3)` fills the cells
//! below it and is saved as a dynamic array; the spill grows and shrinks
//! with its inputs; typing into a spilled cell blocks the spill (`#SPILL!`)
//! until the cell is cleared again; and undo takes it all back.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, CellRef};

const SHEET: &str = "Operations";

fn book() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap()
}

fn shown(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell(SHEET, reference).unwrap().display
}

fn column(book: &HeadlessWorkbook, cells: &[&str]) -> Vec<String> {
    cells.iter().map(|cell| shown(book, cell)).collect()
}

fn array_ranges(book: &HeadlessWorkbook) -> Vec<(CellRef, CellRange)> {
    let workbook = jade_xlsx_reader::read_xlsx(book.bytes()).unwrap();
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == SHEET)
        .unwrap();
    sheet
        .array_formulas
        .iter()
        .map(|array| (array.anchor, array.range))
        .collect()
}

fn range(reference: &str) -> CellRange {
    CellRange::parse(reference).unwrap()
}

#[test]
fn a_typed_spill_fills_its_cells_and_is_saved_as_one() {
    let mut book = book();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4"]),
        ["1", "2", "3", ""]
    );
    assert_eq!(
        array_ranges(&book),
        [(CellRef::parse("J1").unwrap(), range("J1:J3"))]
    );
    // Reopened, the spill is still there and still the formula's.
    let reopened = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
    assert_eq!(column(&reopened, &["J1", "J2", "J3"]), ["1", "2", "3"]);
    assert_eq!(reopened.bytes(), book.bytes(), "reopening changed the file");

    // One undo takes the formula and its spill back.
    assert!(book.undo().unwrap());
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["", "", ""]);
}

#[test]
fn a_spill_grows_and_shrinks_with_its_input() {
    let mut book = book();
    book.set(SHEET, "K1", CellInput::Number(5.0)).unwrap();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(K1)".into()))
        .unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4", "J5", "J6"]),
        ["1", "2", "3", "4", "5", ""]
    );
    book.set(SHEET, "K1", CellInput::Number(2.0)).unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4", "J5"]),
        ["1", "2", "", "", ""]
    );
    assert_eq!(
        array_ranges(&book),
        [(CellRef::parse("J1").unwrap(), range("J1:J2"))]
    );
}

#[test]
fn typing_into_a_spill_blocks_it_until_the_cell_is_cleared() {
    let mut book = book();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    book.set(SHEET, "J2", CellInput::Text("x".into())).unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["#SPILL!", "x", ""]);
    book.set(SHEET, "J2", CellInput::Blank).unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["1", "2", "3"]);
}
