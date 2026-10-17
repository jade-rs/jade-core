use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::read_xlsx;
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/charts-workbook.xlsx");
#[test]
fn unchanged_calculated_formula_values_refresh_caches_while_unsupported_formulas_stay_pending() {
    let mut package = OpcPackage::open(BYTES).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "<c r=\"B2\"><v>10</v></c>",
            "<c r=\"B2\"><f>2+2</f><v>4</v></c>",
        )
        .replace(
            "<c r=\"B3\"><v>20</v></c>",
            "<c r=\"B3\"><f>UNSUPPORTED_FOR_COMPAT()</f><v>20</v></c>",
        );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let xml = package
        .part_str("xl/workbook.xml")
        .unwrap()
        .unwrap()
        .replace("</workbook>", "<calcPr fullCalcOnLoad=\"1\"/></workbook>");
    package.replace("xl/workbook.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let summary = book.recalculate().unwrap();
    assert!(summary.unchanged > 0);
    assert!(!summary.skipped.is_empty());
    let model = book.workbook();
    let chart = model.sheets[0].shapes[0].chart.as_ref().unwrap();
    assert_eq!(chart.series[0].values.points[0].as_deref(), Some("4"));
    assert_eq!(chart.series[0].values.points[1].as_deref(), Some("99"));
    assert!(model.full_calculation_on_load);
}
#[test]
fn explicit_recalculation_refreshes_formula_driven_chart_caches_in_the_export() {
    let mut package = OpcPackage::open(BYTES).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "<c r=\"B2\"><v>10</v></c>",
            "<c r=\"B2\"><f>2+2</f><v>1</v></c>",
        );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    book.recalculate().unwrap();
    let model = read_xlsx(book.bytes()).unwrap();
    assert_eq!(
        model.sheets[0].shapes[0].chart.as_ref().unwrap().series[0]
            .values
            .points[0]
            .as_deref(),
        Some("4")
    );
}

#[test]
fn export_refreshes_the_original_and_undone_snapshot_without_changing_history() {
    use jade_xlsx_headless_api::CellInput;
    let mut book = HeadlessWorkbook::open(BYTES).unwrap();
    let cache = |bytes: &[u8]| {
        read_xlsx(bytes).unwrap().sheets[0].shapes[0]
            .chart
            .as_ref()
            .unwrap()
            .series[0]
            .values
            .points[0]
            .clone()
            .unwrap()
    };
    assert_eq!(cache(&book.export_bytes().unwrap()), "10");
    assert_eq!(book.bytes(), BYTES);
    book.set("Charts", "B2", CellInput::Number(35.0)).unwrap();
    assert_eq!(cache(&book.export_bytes().unwrap()), "35");
    book.undo().unwrap();
    assert_eq!(book.bytes(), BYTES);
    assert_eq!(cache(&book.export_bytes().unwrap()), "10");
    assert!(book.can_redo());
    book.redo().unwrap();
    assert_eq!(cache(&book.export_bytes().unwrap()), "35");
}
