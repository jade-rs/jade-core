use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::{CalculationMode, CellRef, Color, OpcPackage};
fn fixture() -> &'static [u8] {
    include_bytes!("../../../public/fixtures/visual-conditional-workbook.xlsx")
}
fn fill(book: &HeadlessWorkbook, cell: &str) -> Option<Color> {
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    layout
        .cell(CellRef::parse(cell).unwrap())
        .unwrap()
        .fill
        .as_ref()
        .and_then(|fill| fill.foreground)
}
fn export(book: &HeadlessWorkbook, name: &str) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(name),
            book.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn visual_results_and_preserved_rule_xml_survive_edits_history_and_reopening() {
    let mut book = HeadlessWorkbook::open_for_editing(fixture()).unwrap();
    let status = book.conditional_formatting_status();
    assert_eq!((status.rule_count, status.supported_rules), (13, 12));
    assert_eq!(
        status
            .unsupported
            .iter()
            .map(|rule| rule.rule_type.as_str())
            .collect::<Vec<_>>(),
        ["colorScale"]
    );
    let before = book.bytes().to_vec();
    let original = book.workbook().sheets[0].conditional_formats.clone();
    book.set("Visual", "A5", CellInput::Number(200.0)).unwrap();
    assert_eq!(fill(&book, "A3"), Some(Color::Argb([255, 191, 64, 0])));
    assert_eq!(book.cell("Visual", "E5").unwrap().display, "200");
    assert_eq!(book.workbook().sheets[0].conditional_formats, original);
    let after = book.bytes().to_vec();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.bytes(), after);
    let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
    assert_eq!(fill(&reopened, "A3"), fill(&book, "A3"));
    // Preserving writer leaves each standard/extension rule fragment unchanged.
    let source = OpcPackage::open(fixture()).unwrap();
    let written = OpcPackage::open(&after).unwrap();
    let source = source
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let written = written
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let rules = &source[source.find("<conditionalFormatting").unwrap()
        ..source.rfind("</conditionalFormatting>").unwrap() + "</conditionalFormatting>".len()];
    assert!(written.contains(rules));
    export(&book, "visual-conditional-edited.xlsx");
}
#[test]
fn manual_formula_caches_are_used_until_f9_and_the_original_numeric_value_is_always_available() {
    let mut book = HeadlessWorkbook::open_for_editing(fixture()).unwrap();
    book.set_calculation_properties(CalculationMode::Manual, false)
        .unwrap();
    book.set("Visual", "A5", CellInput::Number(200.0)).unwrap();
    assert_eq!(book.cell("Visual", "E5").unwrap().display, "100");
    assert_eq!(book.cell("Visual", "C3").unwrap().display, "50");
    book.recalculate().unwrap();
    assert_eq!(book.cell("Visual", "E5").unwrap().display, "200");
    export(&book, "visual-conditional-manual.xlsx");
}
