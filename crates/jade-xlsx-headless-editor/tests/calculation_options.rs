//! Iterative calculation and precision as displayed, as Excel keeps them in
//! `calcPr` and calculates with them.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::Iteration;

const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");

fn value(book: &HeadlessWorkbook, cell: &str) -> String {
    book.cell("Operations", cell).unwrap().display
}

fn workbook_xml(bytes: &[u8]) -> String {
    let package = jade_opc::OpcPackage::open(bytes).unwrap();
    package.part_str("xl/workbook.xml").unwrap().unwrap()
}

#[test]
fn iterative_calculation_settles_a_circular_reference_and_is_saved() {
    let mut book = HeadlessWorkbook::open(ORIGINAL).unwrap();
    // A balance including the interest on itself.
    book.set("Operations", "H1", CellInput::Number(1000.0))
        .unwrap();
    book.set("Operations", "H2", CellInput::Formula("=H1+H3".into()))
        .unwrap();
    book.set(
        "Operations",
        "H3",
        CellInput::Formula("=ROUND(H2*0.1,2)".into()),
    )
    .unwrap();
    assert_eq!(value(&book, "H2"), "#CIRC!");

    let iteration = Iteration {
        enabled: true,
        count: 100,
        delta: 0.001,
    };
    book.set_calculation_options(iteration, true).unwrap();
    assert_eq!(value(&book, "H2"), "1111.11");
    assert_eq!(value(&book, "H3"), "111.11");

    let saved = book.save_bytes().unwrap();
    let xml = workbook_xml(&saved);
    assert!(xml.contains("iterate=\"1\""), "{xml}");
    assert!(xml.contains("iterateCount=\"100\""), "{xml}");
    assert!(xml.contains("iterateDelta=\"0.001\""), "{xml}");
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().iteration, iteration);
    assert_eq!(value(&reopened, "H2"), "1111.11");

    // Not an undo step: undoing the typing keeps the setting.
    assert!(book.undo().unwrap());
    assert_eq!(book.workbook().iteration, iteration);
}

#[test]
fn a_counter_steps_by_the_maximum_iterations() {
    let mut book = HeadlessWorkbook::open(ORIGINAL).unwrap();
    book.set_calculation_options(
        Iteration {
            enabled: true,
            count: 10,
            delta: 0.001,
        },
        true,
    )
    .unwrap();
    book.set("Operations", "H5", CellInput::Formula("=H5+1".into()))
        .unwrap();
    assert_eq!(value(&book, "H5"), "10");
    // Each recalculation goes round again from the value it has.
    book.recalculate().unwrap();
    assert_eq!(value(&book, "H5"), "20");
}

#[test]
fn precision_as_displayed_calculates_with_what_cells_show() {
    let mut book = HeadlessWorkbook::open(ORIGINAL).unwrap();
    book.set("Operations", "H1", CellInput::Formula("=1/3".into()))
        .unwrap();
    book.set("Operations", "H2", CellInput::Formula("=H1*3".into()))
        .unwrap();
    let two_places = FormatChange {
        number_format: Some("0.00".into()),
        ..Default::default()
    };
    book.set_format("Operations", "H1", &two_places).unwrap();
    let full = FormatChange {
        number_format: Some("0.0000".into()),
        ..Default::default()
    };
    book.set_format("Operations", "H2", &full).unwrap();
    assert_eq!(value(&book, "H2"), "1.0000");

    book.set_calculation_options(Iteration::default(), false)
        .unwrap();
    assert_eq!(value(&book, "H2"), "0.9900");
    let saved = book.save_bytes().unwrap();
    assert!(workbook_xml(&saved).contains("fullPrecision=\"0\""));
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert!(!reopened.workbook().full_precision);
    assert_eq!(value(&reopened, "H2"), "0.9900");
}
