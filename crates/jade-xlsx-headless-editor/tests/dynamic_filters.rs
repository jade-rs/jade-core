//! Date Filters ▸ Today, This Month… and Above/Below Average: a dynamic
//! filter worked out when applied, kept as Excel keeps it.

use jade_xlsx_headless_api::{AutoFilterChange, AutoFilterTarget, CellInput};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;

const SHEET: &str = "Operations";

fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/date-group-1900-workbook.xlsx"
    ))
    .unwrap()
}

fn apply(book: &mut HeadlessWorkbook, column: u32, kind: &str) {
    book.change_auto_filter(
        SHEET,
        &AutoFilterChange::Dynamic {
            target: AutoFilterTarget::Worksheet,
            reference: None,
            column,
            kind: kind.into(),
        },
    )
    .unwrap();
}

fn hidden(book: &HeadlessWorkbook) -> Vec<bool> {
    (1..=3)
        .map(|index| {
            book.workbook().sheets[0]
                .rows
                .iter()
                .any(|row| row.index == index && row.hidden)
        })
        .collect()
}

#[test]
fn today_shows_only_todays_rows() {
    let mut book = open();
    let today = jade_xlsx_calc::today("UTC").expect("today").floor();
    book.set_cells(
        SHEET,
        &[
            ("A2".into(), CellInput::Number(today + 0.5)),
            ("A3".into(), CellInput::Number(today - 40.0)),
            ("A4".into(), CellInput::Number(today + 400.0)),
        ],
    )
    .unwrap();
    apply(&mut book, 0, "today");
    assert_eq!(hidden(&book), [false, true, true]);
    apply(&mut book, 0, "yesterday");
    assert_eq!(hidden(&book), [true, true, true]);
}

#[test]
fn above_average_keeps_the_rows_over_the_columns_average_and_saves_it() {
    let mut book = open();
    // Amounts 10, 20 and 30: only 30 is above their average.
    apply(&mut book, 1, "aboveAverage");
    assert_eq!(hidden(&book), [true, true, false]);
    apply(&mut book, 1, "belowAverage");
    assert_eq!(hidden(&book), [false, true, true]);
    let saved = book.export_bytes().unwrap();
    let xml = OpcPackage::open(&saved)
        .unwrap()
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(
        xml.contains(r#"<dynamicFilter type="belowAverage" val="20"/>"#),
        "{xml}"
    );
    // Reopened, the filter reapplies.
    let mut reopened = HeadlessWorkbook::open(&saved).unwrap();
    reopened
        .change_auto_filter(
            SHEET,
            &AutoFilterChange::Reapply {
                target: AutoFilterTarget::Worksheet,
            },
        )
        .unwrap();
    assert_eq!(hidden(&reopened), [false, true, true]);
}

#[test]
fn an_unknown_kind_is_refused() {
    let mut book = open();
    assert!(book
        .change_auto_filter(
            SHEET,
            &AutoFilterChange::Dynamic {
                target: AutoFilterTarget::Worksheet,
                reference: None,
                column: 0,
                kind: "someday".into(),
            },
        )
        .is_err());
}
