use jade_xlsx_headless_api::FormulaState;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CalculationMode, OpcPackage};
use std::collections::BTreeMap;

fn fixture(mode: CalculationMode) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let xml = package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap().replace("</sheetData>", r#"<row r="6"><c r="A6" t="inlineStr"><is><t>Totals</t></is></c><c r="B6"><f>SUBTOTAL(109,B2:B4)</f><v>60</v></c><c r="C6"><f>SUBTOTAL(9,B2:B4)</f><v>60</v></c><c r="D6"><f>SUM(B2:B4)</f><v>60</v></c></row></sheetData>"#);
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    jade_xlsx_writer::set_calculation_properties(&package.write().unwrap(), mode, false).unwrap()
}
fn value(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
}
fn export(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(format!("subtotal-{name}.xlsx")),
            bytes,
        )
        .unwrap();
    }
}

#[test]
fn hiding_and_revealing_rows_refreshes_visible_subtotals_in_one_exact_undo_transaction() {
    let bytes = fixture(CalculationMode::Auto);
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 1, 1, 0.0)
        .unwrap();
    assert_eq!(value(&book, "B6"), "50");
    assert_eq!(value(&book, "C6"), "60");
    assert_eq!(value(&book, "D6"), "60");
    assert!(!book.calculation_status().requires_recalculation);
    let hidden = book.bytes().to_vec();
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), bytes);
    assert!(!book.can_undo());
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), hidden);
    book.set_axis_size("Operations", Axis::Rows, 1, 1, 15.0)
        .unwrap();
    assert_eq!(value(&book, "B6"), "60");
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), hidden);
    let reopened = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
    assert_eq!(value(&reopened, "B6"), "50");
    export("hidden", &book.export_bytes().unwrap());
}

#[test]
fn manual_visibility_changes_mark_caches_stale_until_explicit_calculation() {
    let bytes = fixture(CalculationMode::Manual);
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 1, 1, 0.0)
        .unwrap();
    assert_eq!(value(&book, "B6"), "60");
    assert_eq!(
        book.cell("Operations", "B6").unwrap().formula_state,
        Some(FormulaState::Stale)
    );
    assert!(book.calculation_status().requires_recalculation);
    let stale = book.bytes().to_vec();
    assert!(
        HeadlessWorkbook::open(&stale)
            .unwrap()
            .calculation_status()
            .requires_recalculation
    );
    book.recalculate().unwrap();
    assert_eq!(value(&book, "B6"), "50");
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), bytes);
    assert!(!book.can_undo());
    assert!(book.redo().unwrap());
    assert_eq!(value(&book, "B6"), "50");
    export("manual", &stale);
}

#[test]
fn compact_all_row_defaults_and_fitted_reveals_update_subtotals_without_materializing_rows() {
    let mut book = HeadlessWorkbook::open(&fixture(CalculationMode::Auto)).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 0, 1_048_575, 0.0)
        .unwrap();
    assert_eq!(value(&book, "B6"), "0");
    assert!(book.workbook().sheets[0].rows.len() < 10);
    book.set_fitted_row_sizes("Operations", &BTreeMap::from([(1, 15.0)]))
        .unwrap();
    assert_eq!(value(&book, "B6"), "10");
    assert!(book.workbook().sheets[0].default_rows_hidden);
    book.set_row_sizes_with_default("Operations", 15.0, &BTreeMap::new(), false)
        .unwrap();
    assert_eq!(value(&book, "B6"), "60");
    assert!(book.undo().unwrap());
    assert_eq!(value(&book, "B6"), "10");
    export("default", &book.export_bytes().unwrap());
}

#[test]
fn nonvisibility_resizing_and_hidden_columns_do_not_refresh_volatile_formulas() {
    ironcalc_base::clock::with_milliseconds(1782873000000, || {
        let mut book = HeadlessWorkbook::open(&fixture(CalculationMode::Auto)).unwrap();
        book.set(
            "Operations",
            "E6",
            jade_xlsx_headless_api::CellInput::Formula("NOW()".into()),
        )
        .unwrap();
        let now = value(&book, "E6");
        ironcalc_base::clock::with_milliseconds(1782876600000, || {
            book.set_axis_size("Operations", Axis::Rows, 1, 1, 22.0)
                .unwrap();
            assert_eq!(value(&book, "E6"), now);
            book.set_axis_size("Operations", Axis::Columns, 1, 1, 0.0)
                .unwrap();
            assert_eq!(value(&book, "B6"), "60");
            assert_eq!(value(&book, "E6"), now);
            book.set_axis_size("Operations", Axis::Rows, 1, 1, 0.0)
                .unwrap();
            assert_eq!(value(&book, "B6"), "50");
            assert_ne!(value(&book, "E6"), now);
        });
    });
}
