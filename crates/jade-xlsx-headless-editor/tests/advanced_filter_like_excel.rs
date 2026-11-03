//! Data ▸ Advanced filtering a list in place as Excel does: the records
//! left out hidden as filtered rows, the sheet in filter mode, the list and
//! criteria named `_xlnm._FilterDatabase` and `_xlnm.Criteria`, one undo
//! step — and Data ▸ Clear showing them all again.

use std::collections::BTreeSet;

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, OpcPackage};

fn book() -> (HeadlessWorkbook, String) {
    let bytes = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let mut book = HeadlessWorkbook::open_for_editing(bytes).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    // A list in H40:I44: a name and sales, its header on top.
    let cells: Vec<(String, CellInput)> = [
        ("H40", "Name"),
        ("I40", "Sales"),
        ("H41", "Davolio"),
        ("I41", "10"),
        ("H42", "Buchanan"),
        ("I42", "40"),
        ("H43", "Davis"),
        ("I43", "20"),
        ("H44", "Suyama"),
        ("I44", "50"),
    ]
    .iter()
    .map(|(at, value)| (at.to_string(), CellInput::parse(value)))
    .collect();
    book.set_cells(&sheet, &cells).unwrap();
    (book, sheet)
}

fn hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    let sheet = &book.workbook().sheets[0];
    sheet
        .rows
        .iter()
        .find(|found| found.index == row)
        .map(|found| found.hidden)
        .unwrap_or(sheet.default_rows_hidden)
}

fn sheet_xml(book: &HeadlessWorkbook) -> String {
    let part = book.workbook().sheets[0].part_name.clone();
    OpcPackage::open(book.bytes())
        .unwrap()
        .part_str(&part)
        .unwrap()
        .unwrap()
}

#[test]
fn filtering_in_place_hides_the_records_left_out_as_filtered_rows_in_one_step() {
    let (mut book, sheet) = book();
    let before = book.bytes().to_vec();
    let list = CellRange::parse("H40:I44").unwrap();
    let criteria = CellRange::parse("K40:K41").unwrap();
    // Rows 42 and 44 (0-based 41 and 43) meet the criteria.
    let shown: BTreeSet<u32> = [41, 43].into_iter().collect();
    let count = book
        .advanced_filter(&sheet, list, &shown, Some((0, criteria)))
        .unwrap();
    assert_eq!(count, 2);
    assert!(hidden(&book, 40));
    assert!(!hidden(&book, 41));
    assert!(hidden(&book, 42));
    assert!(!hidden(&book, 43));
    assert!(!hidden(&book, 39), "the field names stay");
    assert_eq!(book.workbook().sheets[0].filter_mode, Some(true));
    assert!(sheet_xml(&book).contains("filterMode=\"1\""));
    let names = &book.workbook().defined_names;
    let database = names
        .iter()
        .find(|name| name.name == "_xlnm._FilterDatabase")
        .unwrap();
    assert!(database.hidden);
    assert_eq!(database.local_sheet_index, Some(0));
    assert!(
        database.formula.ends_with("!$H$40:$I$44"),
        "{}",
        database.formula
    );
    assert!(names
        .iter()
        .any(|name| name.name == "_xlnm.Criteria" && name.formula.ends_with("!$K$40:$K$41")));
    assert_eq!(
        book.filter_database(&sheet).unwrap(),
        Some(CellRange::parse("H40:I44").unwrap())
    );
    // A SUBTOTAL leaves the filtered records out, as it does Excel's.
    book.set_cells(
        &sheet,
        &[(
            "I46".into(),
            CellInput::Formula("SUBTOTAL(9,I41:I44)".into()),
        )],
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "I46").unwrap().display, "90");
    // Opened again, as Excel saves it (filter mode, no AutoFilter), the
    // hidden records are still the filter's.
    let mut reopened = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
    reopened
        .set_cells(
            &sheet,
            &[(
                "I47".into(),
                CellInput::Formula("SUBTOTAL(9,I41:I44)".into()),
            )],
        )
        .unwrap();
    assert_eq!(reopened.cell(&sheet, "I47").unwrap().display, "90");
    book.undo().unwrap();
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before.as_slice());
}

#[test]
fn clear_shows_every_record_again_and_leaves_filter_mode() {
    let (mut book, sheet) = book();
    let list = CellRange::parse("H40:I44").unwrap();
    let shown: BTreeSet<u32> = [42].into_iter().collect();
    book.advanced_filter(&sheet, list, &shown, None).unwrap();
    assert!(hidden(&book, 40));
    assert!(book.show_all_records(&sheet).unwrap());
    for row in 40..=43 {
        assert!(!hidden(&book, row), "row {row}");
    }
    assert_eq!(book.workbook().sheets[0].filter_mode, Some(false));
    // Nothing filtered: nothing to clear.
    assert!(!book.show_all_records(&sheet).unwrap());
    // Filtered again, every record kept: all shown.
    let all: BTreeSet<u32> = (40..=43).collect();
    assert_eq!(book.advanced_filter(&sheet, list, &all, None).unwrap(), 4);
    assert!((40..=43).all(|row| !hidden(&book, row)));
}
