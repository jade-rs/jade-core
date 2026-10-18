use jade_xlsx_headless_api::{CellInput, FormulaState};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRef, OpcPackage};

fn workbook(date1904: bool) -> HeadlessWorkbook {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    if date1904 {
        let xml = package
            .part_str("xl/workbook.xml")
            .unwrap()
            .unwrap()
            .replace("<sheets>", "<workbookPr date1904=\"1\"/><sheets>");
        package.replace("xl/workbook.xml", xml.into_bytes());
    }
    HeadlessWorkbook::open(&package.write().unwrap()).unwrap()
}

#[test]
fn cached_empty_text_is_a_result_and_value_cells_have_no_formula_state() {
    let book = workbook(false);
    let status = book.calculation_status();
    assert_eq!(status.formula_count, 2);
    assert_eq!(status.cached, 2);
    assert!(!status.requires_recalculation);
    assert_eq!(
        book.formula_state(0, CellRef::parse("C2").unwrap()),
        Some(FormulaState::Cached)
    );
    assert_eq!(book.formula_state(0, CellRef::parse("A1").unwrap()), None);
}

#[test]
fn skipped_formulas_are_reported_and_undo_restores_the_original_cache_state() {
    let mut book = workbook(false);
    book.set(
        "Compatibility",
        "B2",
        CellInput::Formula("UNSUPPORTED_FOR_COMPAT()".into()),
    )
    .unwrap();
    let status = book.calculation_status();
    assert_eq!(status.formula_count, 3);
    // The typed formula shows the engine's `#NAME?` rather than nothing,
    // held as stale until the full calculation the file now asks for.
    assert_eq!(status.uncalculated, 0);
    assert_eq!(status.stale, 3);
    assert!(status.requires_recalculation);
    assert_eq!(book.cell("Compatibility", "B2").unwrap().display, "#NAME?");
    assert_eq!(
        book.cell("Compatibility", "B2").unwrap().formula_state,
        book.formula_state(0, CellRef::parse("B2").unwrap())
    );
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(reopened.calculation_status(), status);
    book.undo().unwrap();
    assert_eq!(book.calculation_status().cached, 2);
    assert!(!book.calculation_status().requires_recalculation);
    book.redo().unwrap();
    assert_eq!(book.calculation_status(), status);
}

#[test]
fn a_1904_edit_refreshes_supported_formula_results() {
    let mut book = workbook(true);
    book.set("Compatibility", "A1", CellInput::Text("changed".into()))
        .unwrap();
    let status = book.calculation_status();
    assert_eq!(status.date_system, 1904);
    assert_eq!(status.stale, 0);
    assert_eq!(status.cached, 2);
    assert!(!status.requires_recalculation);
}

#[test]
fn supported_recalculation_is_reflected_in_the_current_snapshot() {
    let mut book = workbook(false);
    book.set("Compatibility", "A8", CellInput::Formula("A6+1".into()))
        .unwrap();
    let status = book.calculation_status();
    assert_eq!(status.cached, 2);
    assert_eq!(status.stale + status.uncalculated, 0);
    assert!(!status.requires_recalculation);
}

#[test]
fn unsupported_conditional_formulas_are_reported_and_rules_survive_edits_and_undo() {
    let mut book = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    let status = book.conditional_formatting_status();
    assert_eq!(status.rule_count, 12);
    assert_eq!(status.supported_rules, 11);
    assert_eq!(status.unsupported.len(), 1);
    assert_eq!(status.unsupported[0].rule_type, "expression");
    assert_eq!(book.cell("Conditional", "A3").unwrap().display, "$120.00");
    assert_eq!(
        book.cell("Conditional", "A3")
            .unwrap()
            .number_format
            .as_deref(),
        Some("$0.00")
    );
    assert_eq!(
        book.sheet_text("Conditional").unwrap().rows[2][0],
        "$120.00"
    );
    assert_eq!(book.find("$120", true)[0].reference, "A3");
    book.set("Conditional", "A1", CellInput::Number(20.0))
        .unwrap();
    assert_eq!(book.conditional_formatting_status(), status);
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(
        reopened.workbook().styles.differential_styles,
        book.workbook().styles.differential_styles
    );
    assert_eq!(reopened.conditional_formatting_status(), status);
    book.undo().unwrap();
    assert_eq!(book.conditional_formatting_status(), status);
}

#[test]
fn differential_border_clears_survive_public_model_serialization() {
    let style = jade_xlsx_reader::conditional_format::DifferentialStyle {
        left: Some(jade_xlsx_reader::conditional_format::DifferentialBorder::Clear),
        ..Default::default()
    };
    let json = serde_json::to_string(&style).unwrap();
    let reopened: jade_xlsx_reader::conditional_format::DifferentialStyle =
        serde_json::from_str(&json).unwrap();
    assert_eq!(style, reopened);
    assert_eq!(reopened.right, None);
}
