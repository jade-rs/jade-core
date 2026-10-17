use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{effective_style, FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::CellRef;
const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
fn bold(workbook: &HeadlessWorkbook, reference: &str) -> bool {
    let sheet = &workbook.workbook().sheets[0];
    let styles = &workbook.workbook().styles;
    let index = effective_style(sheet, CellRef::parse(reference).unwrap());
    styles
        .cell_format(Some(index))
        .and_then(|format| styles.font(format))
        .is_some_and(|font| font.bold)
}
#[test]
fn column_format_survives_future_writes_undo_redo_and_export() {
    let mut workbook = HeadlessWorkbook::open(ORIGINAL).unwrap();
    let change = FormatChange {
        bold: Some(true),
        number_format: Some("0.00".into()),
        ..Default::default()
    };
    workbook.set_format("Operations", "$B:$B", &change).unwrap();
    assert!(bold(&workbook, "B100"));
    assert!(!bold(&workbook, "A100"));
    let styles = workbook.workbook().styles.cell_formats.len();
    workbook.set_format("Operations", "B:B", &change).unwrap();
    assert_eq!(styles, workbook.workbook().styles.cell_formats.len());
    workbook.undo().unwrap();
    workbook.undo().unwrap();
    assert!(!bold(&workbook, "B100"));
    workbook.redo().unwrap();
    workbook
        .set_cells("Operations", &[("B100".into(), CellInput::Number(1.2))])
        .unwrap();
    assert_eq!(workbook.cell("Operations", "B100").unwrap().display, "1.20");
    assert!(bold(
        &HeadlessWorkbook::open(&workbook.export_bytes().unwrap()).unwrap(),
        "B100"
    ));
}
#[test]
fn row_and_sheet_references_format_empty_future_cells() {
    let mut workbook = HeadlessWorkbook::open(ORIGINAL).unwrap();
    let change = FormatChange {
        bold: Some(true),
        ..Default::default()
    };
    workbook.set_format("Operations", "6:5", &change).unwrap();
    assert!(bold(&workbook, "XFD5"));
    assert!(!bold(&workbook, "XFD7"));
    workbook
        .set_format("Operations", "A1:XFD1048576", &change)
        .unwrap();
    assert!(bold(&workbook, "XFD1048576"));
    assert_eq!(workbook.workbook().sheets[0].rows.len(), 6);
}
