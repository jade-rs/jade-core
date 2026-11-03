//! Sparklines made and changed as Excel's Insert ▸ Sparklines and the
//! Sparkline tab do: a group over a data range, a sparkline a row; its
//! look and axes; grouped, ungrouped and cleared. Each is read back as
//! made, drawn, saved in the worksheet's `x14:sparklineGroups` as Excel
//! writes it, and one undo step.

use jade_xlsx_headless_editor::{HeadlessWorkbook, SparklineGroup};
use jade_xlsx_reader::model::Color;
use jade_xlsx_reader::CellRef;
use std::path::PathBuf;

/// Operations!A1:C4: Name, Amount, Double over Alpha, Beta, Gamma; 10, 20,
/// 30; and twice those.
const SHEET: &str = "Operations";

fn open(fixture: &str) -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(fixture);
    HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopened(book: &mut HeadlessWorkbook) -> Vec<SparklineGroup> {
    let bytes = book.export_bytes().unwrap();
    HeadlessWorkbook::open(&bytes)
        .unwrap()
        .sparkline_groups(SHEET)
        .unwrap()
}

fn cell(text: &str) -> CellRef {
    CellRef::parse(text).unwrap()
}

#[test]
fn insert_sparklines_makes_a_group_of_a_sparkline_a_row() {
    let mut book = open("grid-operations-workbook.xlsx");
    book.insert_sparklines(SHEET, "line", "B2:C4", "D2:D4")
        .unwrap();
    for groups in [book.sparkline_groups(SHEET).unwrap(), reopened(&mut book)] {
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.kind, "line");
        assert_eq!(group.empty_cells, "gap");
        assert_eq!(group.line_weight_points, 0.75);
        assert!(group.color_series.is_some() && group.color_markers.is_some());
        let made: Vec<(&str, CellRef)> = group
            .sparklines
            .iter()
            .map(|one| (one.formula.as_str(), one.cell))
            .collect();
        assert_eq!(
            made,
            [
                ("Operations!B2:C2", cell("D2")),
                ("Operations!B3:C3", cell("D3")),
                ("Operations!B4:C4", cell("D4")),
            ]
        );
    }
    // Drawn in its cells.
    let layout = jade_xlsx_layout::layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &jade_xlsx_layout::LayoutOptions::default(),
    );
    assert_eq!(layout.sparklines.len(), 3);
    // Sparklines put where others are take their place.
    book.insert_sparklines(SHEET, "column", "B4:C4", "D4")
        .unwrap();
    let groups = book.sparkline_groups(SHEET).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].sparklines.len(), 2);
    assert_eq!(groups[1].kind, "column");
    assert!(book.undo().unwrap());
    assert_eq!(book.sparkline_groups(SHEET).unwrap().len(), 1);
    assert!(book
        .insert_sparklines(SHEET, "line", "B2:C4", "D2:D5")
        .is_err());
}

#[test]
fn the_look_and_axes_are_saved_as_excel_writes_them() {
    let mut book = open("grid-operations-workbook.xlsx");
    book.insert_sparklines(SHEET, "column", "B2:C4", "D2:D4")
        .unwrap();
    let mut groups = book.sparkline_groups(SHEET).unwrap();
    let group = &mut groups[0];
    group.kind = "stacked".into();
    group.high = true;
    group.low = true;
    group.negative = true;
    group.first = true;
    group.last = true;
    group.markers = true;
    group.display_axis = true;
    group.display_hidden = true;
    group.right_to_left = true;
    group.line_weight_points = 2.25;
    group.empty_cells = "span".into();
    group.min_axis = "custom".into();
    group.manual_min = Some(-5.0);
    group.max_axis = "group".into();
    group.color_series = Some(Color::Argb([255, 0x37, 0x60, 0x92]));
    group.date_axis = true;
    group.date_range = Some("Operations!B1:C1".into());
    assert!(book.set_sparkline_groups(SHEET, &groups).unwrap());
    let read = reopened(&mut book);
    assert_eq!(read, groups);
    // Nothing changed writes nothing.
    assert!(!book.set_sparkline_groups(SHEET, &groups).unwrap());
    // Ungroup: a group each, with the look; then none at all.
    let one_each: Vec<SparklineGroup> = groups[0]
        .sparklines
        .iter()
        .map(|one| SparklineGroup {
            sparklines: vec![one.clone()],
            ..groups[0].clone()
        })
        .collect();
    assert!(book.set_sparkline_groups(SHEET, &one_each).unwrap());
    assert_eq!(reopened(&mut book).len(), 3);
    assert!(book.set_sparkline_groups(SHEET, &[]).unwrap());
    assert!(reopened(&mut book).is_empty());
    assert!(book.undo().unwrap());
    assert_eq!(book.sparkline_groups(SHEET).unwrap().len(), 3);
}

#[test]
fn two_sparklines_in_one_cell_and_unknown_types_are_refused() {
    let mut book = open("grid-operations-workbook.xlsx");
    book.insert_sparklines(SHEET, "line", "B2:C4", "D2:D4")
        .unwrap();
    let mut groups = book.sparkline_groups(SHEET).unwrap();
    let copy = groups[0].clone();
    groups.push(copy);
    assert!(book.set_sparkline_groups(SHEET, &groups).is_err());
    let mut groups = book.sparkline_groups(SHEET).unwrap();
    groups[0].kind = "pie".into();
    assert!(book.set_sparkline_groups(SHEET, &groups).is_err());
}
