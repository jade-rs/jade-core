//! Laying out only the rows typing changed gives what a full layout gives.

use jade_xlsx_layout::{layout_workbook_sheet, relayout_rows, LayoutOptions};
use jade_xlsx_reader::ignored_errors::IgnoredError;
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef, CellValue};

#[test]
fn a_row_laid_out_again_keeps_the_sheets_ignored_errors() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    );
    let mut workbook = read_xlsx(&std::fs::read(path).unwrap()).unwrap();
    let options = LayoutOptions::default();
    let row = 4;
    let set = |workbook: &mut jade_xlsx_reader::Workbook, value: &str| {
        let sheet = &mut workbook.sheets[0];
        let cells = &mut sheet
            .rows
            .iter_mut()
            .find(|r| r.index == row)
            .unwrap()
            .cells;
        let cell = cells.iter_mut().find(|c| c.reference.column == 1).unwrap();
        cell.value = CellValue::Text(value.into());
        cell.formula = None;
    };
    set(&mut workbook, "0042");
    workbook.sheets[0].ignored_errors = vec![IgnoredError {
        ranges: vec![CellRange::parse("B5").unwrap()],
        number_stored_as_text: true,
        ..Default::default()
    }];
    let mut layout = layout_workbook_sheet(&workbook, 0, &options).unwrap();
    let flagged = |layout: &jade_xlsx_layout::SheetLayout| {
        layout.cell(CellRef::new(1, row)).expect("B5").error_flag
    };
    assert!(!flagged(&layout));
    // Typing another number stored as text into the same cell.
    set(&mut workbook, "0043");
    assert!(relayout_rows(
        &mut layout,
        &workbook,
        0,
        &options,
        &[row].into()
    ));
    assert!(!flagged(&layout));
    assert_eq!(
        layout,
        layout_workbook_sheet(&workbook, 0, &options).unwrap()
    );
}
