//! Freeze Panes set, changed and removed on a workbook, each one undo step,
//! saved so that the file reopens frozen.

use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRef, FreezePane};
use jade_xlsx_writer::freeze_panes::Frozen;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}

#[test]
fn panes_freeze_unfreeze_and_undo_as_one_step_each() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    assert_eq!(book.workbook().sheets[0].freeze_pane, None);
    book.set_freeze_panes(
        &sheet,
        Some(Frozen {
            columns: 1,
            rows: 2,
            top_left: None,
        }),
    )
    .unwrap();
    let pane = FreezePane {
        frozen_columns: 1,
        frozen_rows: 2,
        top_left_cell: Some(CellRef::new(1, 2)),
    };
    assert_eq!(book.workbook().sheets[0].freeze_pane, Some(pane.clone()));
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().sheets[0].freeze_pane, Some(pane));

    book.set_freeze_panes(&sheet, None).unwrap();
    assert_eq!(book.workbook().sheets[0].freeze_pane, None);
    // Unfreezing a sheet that is not frozen is no step at all.
    book.set_freeze_panes(&sheet, None).unwrap();
    assert!(book.undo().unwrap());
    assert!(book.workbook().sheets[0].freeze_pane.is_some());
    assert!(book.undo().unwrap());
    assert_eq!(book.workbook().sheets[0].freeze_pane, None);
    assert_eq!(book.bytes(), fixture().as_slice());
}
