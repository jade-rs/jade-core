//! Sort ▸ Order ▸ Custom List as Excel's: the values in the list's order
//! (turned round for descending), the rest after them, blanks last, and the
//! list saved on the sort condition as `customList`.

use jade_xlsx_headless_api::{CellInput, SortKey, SortRequest};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;

fn book() -> (HeadlessWorkbook, String) {
    let bytes = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let mut book = HeadlessWorkbook::open_for_editing(bytes).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let cells: Vec<(String, CellInput)> = [
        ("H40", "Priority"),
        ("I40", "Task"),
        ("H41", "Low"),
        ("I41", "a"),
        ("H42", "high"),
        ("I42", "b"),
        ("H43", "Urgent"),
        ("I43", "c"),
        ("H44", "Medium"),
        ("I44", "d"),
        ("H45", "Low"),
        ("I45", "e"),
        ("I46", "f"),
    ]
    .iter()
    .map(|(at, value)| (at.to_string(), CellInput::parse(value)))
    .collect();
    book.set_cells(&sheet, &cells).unwrap();
    (book, sheet)
}

fn request(descending: bool) -> SortRequest {
    SortRequest {
        reference: "H40:I46".into(),
        header: true,
        case_sensitive: false,
        locale: None,
        phonetic: None,
        keys: vec![SortKey {
            column: 0,
            descending,
            color: None,
            custom_list: Some(vec!["High".into(), "Medium".into(), "Low".into()]),
        }],
        by_columns: false,
    }
}

fn tasks(book: &HeadlessWorkbook, sheet: &str) -> String {
    (41..=46)
        .map(|row| book.cell(sheet, &format!("I{row}")).unwrap().display)
        .collect()
}

#[test]
fn a_custom_list_sorts_in_its_order_with_the_rest_after_and_blanks_last() {
    let (mut book, sheet) = book();
    book.sort_range(&sheet, &request(false)).unwrap();
    // high (matched without case), Medium, the two Lows in their order,
    // Urgent (not in the list), the blank last.
    assert_eq!(tasks(&book, &sheet), "bdaecf");
    let part = book.workbook().sheets[0].part_name.clone();
    let xml = OpcPackage::open(book.bytes())
        .unwrap()
        .part_str(&part)
        .unwrap()
        .unwrap();
    assert!(
        xml.contains(r#"customList="High,Medium,Low""#),
        "the sort condition keeps its list"
    );
}

#[test]
fn descending_turns_the_list_round() {
    let (mut book, sheet) = book();
    book.sort_range(&sheet, &request(true)).unwrap();
    // Urgent, outside the list, comes before it turned round; blank last.
    assert_eq!(tasks(&book, &sheet), "caedbf");
}
