//! A protected sheet refuses what Excel refuses on one: typing into a locked
//! cell (cells are locked unless their format says otherwise), pasting or
//! filling over one, formatting, inserting and deleting rows and columns,
//! resizing them, and notes — except what the protection allows. Unlocked
//! cells and unprotected sheets edit as ever, and a refused edit changes
//! nothing.

use jade_xlsx_headless_api::{CellInput, CellKind};
use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook};
use jade_xlsx_reader::Axis;
use jade_xlsx_writer::FormatChange;
use std::path::PathBuf;

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/protected-sheets-workbook.xlsx"
    ));
    HeadlessWorkbook::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn number(workbook: &HeadlessWorkbook, sheet: &str, reference: &str) -> f64 {
    let view = workbook.cell(sheet, reference).expect("cell");
    assert_eq!(view.kind, CellKind::Number, "{reference}: {view:?}");
    view.number.expect("number")
}

fn protected<T: std::fmt::Debug>(result: Result<T, HeadlessError>) {
    assert!(
        matches!(result, Err(HeadlessError::Protected { .. })),
        "{result:?}"
    );
}

#[test]
fn the_protection_and_locked_cells_are_read() {
    let workbook = open();
    let sheets = &workbook.workbook().sheets;
    let locked = sheets[0].protection.as_ref().expect("Locked is protected");
    assert!(!locked.insert_rows && !locked.format_cells);
    assert!(sheets[1].protection.is_none());
    let rows = sheets[2].protection.as_ref().expect("Rows is protected");
    assert!(rows.insert_rows && rows.format_cells && !rows.delete_rows);
    let styles = &workbook.workbook().styles.cell_formats;
    assert!(!styles[0].unlocked);
    assert!(styles[1].unlocked);
}

#[test]
fn a_locked_cell_on_a_protected_sheet_refuses_input() {
    let mut workbook = open();
    protected(workbook.set_cells("Locked", &[("A1".into(), CellInput::Number(1.0))]));
    // One locked cell in a batch refuses the whole batch.
    protected(workbook.set_cells(
        "Locked",
        &[
            ("B1".into(), CellInput::Number(2.0)),
            ("A1".into(), CellInput::Number(1.0)),
        ],
    ));
    assert_eq!(number(&workbook, "Locked", "A1"), 10.0);
    assert_eq!(number(&workbook, "Locked", "B1"), 20.0);
    assert!(!workbook.can_undo());
    // An empty cell is locked too: its format is the default one.
    protected(workbook.set_cells("Locked", &[("D9".into(), CellInput::Text("x".into()))]));
    // B1's format unlocks it, and the unprotected sheet edits as ever.
    workbook
        .set_cells("Locked", &[("B1".into(), CellInput::Number(25.0))])
        .unwrap();
    workbook
        .set_cells("Open", &[("A1".into(), CellInput::Number(11.0))])
        .unwrap();
    assert_eq!(number(&workbook, "Locked", "B1"), 25.0);
    assert_eq!(number(&workbook, "Open", "A1"), 11.0);
}

#[test]
fn formatting_structure_sizes_fills_and_pastes_are_refused_unless_allowed() {
    let mut workbook = open();
    let bold = FormatChange {
        bold: Some(true),
        ..FormatChange::default()
    };
    protected(workbook.set_format("Locked", "B1", &bold));
    protected(workbook.insert_rows("Locked", 1, 1));
    protected(workbook.delete_columns("Locked", "A", 1));
    protected(workbook.set_axis_size("Locked", Axis::Rows, 0, 0, 30.0));
    protected(workbook.fill("Locked", "A1", "A1:A3"));
    protected(workbook.set_note("Locked", "A1", "hi", "me"));
    protected(workbook.merge_cells("Locked", "A1:B1"));
    let copy = workbook.copy_snapshot("Open", "A1", false).unwrap();
    protected(workbook.paste_snapshot(&copy, "Locked", "A1", false));
    assert!(!workbook.can_undo());
    // "Rows" allows inserting rows and formatting cells, nothing else.
    workbook.insert_rows("Rows", 1, 1).unwrap();
    workbook.set_format("Rows", "A1", &bold).unwrap();
    protected(workbook.delete_rows("Rows", 1, 1));
    // The unprotected sheet does it all.
    workbook.insert_rows("Open", 1, 1).unwrap();
    workbook.set_format("Open", "A1", &bold).unwrap();
}
