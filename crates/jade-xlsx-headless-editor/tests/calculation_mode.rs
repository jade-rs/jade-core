use jade_xlsx_headless_api::{CellInput, FormulaState};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CalculationMode;

fn book(mode: CalculationMode, on_save: bool) -> HeadlessWorkbook {
    let bytes = jade_xlsx_writer::set_calculation_properties(
        include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx"),
        mode,
        on_save,
    )
    .unwrap();
    HeadlessWorkbook::open(&bytes).unwrap()
}
fn value(book: &HeadlessWorkbook, cell: &str) -> String {
    book.cell("Operations", cell).unwrap().display
}
fn export(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(format!("calculation-{name}.xlsx")),
            bytes,
        )
        .unwrap();
    }
}

#[test]
fn manual_edits_keep_cached_values_until_explicit_recalculation_and_undo_preserves_preferences() {
    let mut book = book(CalculationMode::Manual, false);
    let before = book.bytes().to_vec();
    book.set("Operations", "B2", CellInput::Number(40.0))
        .unwrap();
    assert_eq!(value(&book, "C2"), "20");
    assert_eq!(
        book.cell("Operations", "C2").unwrap().formula_state,
        Some(FormulaState::Stale)
    );
    assert!(book.calculation_status().requires_recalculation);
    book.recalculate().unwrap();
    assert_eq!(value(&book, "C2"), "80");
    assert!(!book.calculation_status().requires_recalculation);
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    assert!(book.redo().unwrap());
    assert_eq!(value(&book, "C2"), "80");
    assert_eq!(book.workbook().calculation_mode, CalculationMode::Manual);
    export("manual", &book.export_bytes().unwrap());
}

#[test]
fn calculate_on_save_updates_caches_only_when_the_saved_preference_requests_it() {
    let mut book = book(CalculationMode::Manual, false);
    book.set("Operations", "B2", CellInput::Number(40.0))
        .unwrap();
    let stale = book.save_bytes().unwrap();
    assert_eq!(value(&HeadlessWorkbook::open(&stale).unwrap(), "C2"), "20");
    assert_eq!(value(&book, "C2"), "20");
    book.set_calculation_properties(CalculationMode::Manual, true)
        .unwrap();
    assert_eq!(value(&book, "C2"), "20");
    let fresh = book.save_bytes().unwrap();
    assert_eq!(value(&book, "C2"), "80");
    let reopened = HeadlessWorkbook::open(&fresh).unwrap();
    assert_eq!(value(&reopened, "C2"), "80");
    assert_eq!(reopened.calculation_status().calculation_mode, "manual");
    assert!(!reopened.calculation_status().requires_recalculation);
    export("saved", &fresh);
}

#[test]
fn switching_to_automatic_refreshes_values_and_history_does_not_switch_the_host_mode_back() {
    let mut book = book(CalculationMode::Manual, false);
    book.set("Operations", "B2", CellInput::Number(40.0))
        .unwrap();
    book.set_calculation_properties(CalculationMode::Auto, true)
        .unwrap();
    assert_eq!(value(&book, "C2"), "80");
    assert!(book.undo().unwrap());
    assert_eq!(value(&book, "B2"), "10");
    assert_eq!(book.workbook().calculation_mode, CalculationMode::Auto);
    assert!(book.redo().unwrap());
    assert_eq!(value(&book, "C2"), "80");
    book.set_calculation_properties(CalculationMode::AutoNoTable, true)
        .unwrap();
    book.set("Operations", "B2", CellInput::Number(50.0))
        .unwrap();
    assert_eq!(value(&book, "C2"), "100");
    assert_eq!(book.calculation_status().calculation_mode, "autoNoTable");
}

#[test]
fn manual_mode_defers_structural_and_clipboard_recalculation() {
    let mut book = book(CalculationMode::Manual, false);
    book.set("Operations", "B2", CellInput::Number(40.0))
        .unwrap();
    book.insert_rows("Operations", 2, 1).unwrap();
    assert_eq!(value(&book, "C3"), "20");
    book.delete_rows("Operations", 2, 1).unwrap();
    assert_eq!(value(&book, "C2"), "20");
    let copied = book.copy_snapshot("Operations", "B2:C2", false).unwrap();
    book.paste_snapshot(&copied, "Operations", "B3", false)
        .unwrap();
    assert_eq!(value(&book, "C3"), "20");
    book.recalculate().unwrap();
    assert_eq!(value(&book, "C2"), "80");
    assert_eq!(value(&book, "C3"), "80");
}
