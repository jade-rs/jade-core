//! What a viewport sees. This is what keeps a big sheet drawable: the browser
//! paints a screenful, not the whole grid.

use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{Cell, CellRange, CellRef, CellValue, Row, Sheet, StyleTable};

fn cell(reference: &str, value: CellValue) -> Cell {
    Cell {
        reference: CellRef::parse(reference).expect("reference"),
        value,
        style_index: None,
        formula: None,
    }
}

fn row(index: u32, cells: Vec<Cell>) -> Row {
    Row {
        index,
        height_points: None,
        custom_height: false,
        custom_format: false,
        hidden: false,
        style_index: None,
        cells,
    }
}

/// A sheet with `rows` rows of one cell each, all default sized: 64 px columns
/// and 20 px rows (15 pt).
fn tall_sheet(rows: u32) -> Sheet {
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: (0..rows)
            .map(|index| {
                row(
                    index,
                    vec![cell(
                        &format!("A{}", index + 1),
                        CellValue::Number(f64::from(index)),
                    )],
                )
            })
            .collect(),
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: Some(15.0),
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        show_formulas: false,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        ignored_errors: Vec::new(),
        outline: Default::default(),
        protection: None,
    }
}

fn laid_out(sheet: &Sheet) -> SheetLayout {
    layout_sheet(sheet, &StyleTable::default(), &LayoutOptions::default())
}

#[test]
fn a_viewport_sees_only_the_rows_it_covers() {
    let layout = laid_out(&tall_sheet(1000));
    // 15 pt = 20 px per row.
    let range = layout
        .visible_range(Rect {
            x: 0.0,
            y: 400.0,
            width: 64.0,
            height: 100.0,
        })
        .expect("range");

    assert_eq!(range.first_row, 20);
    assert_eq!(range.last_row, 24);
    assert_eq!(range.first_column, 0);
    assert_eq!(range.last_column, 0);
}

#[test]
fn a_row_touching_the_viewport_edge_by_a_pixel_is_included() {
    let layout = laid_out(&tall_sheet(100));
    let range = layout
        .visible_range(Rect {
            x: 0.0,
            y: 39.0,
            width: 64.0,
            height: 2.0,
        })
        .expect("range");
    // Rows 1 and 2 span 20..40 and 40..60; the viewport is 39..41.
    assert_eq!((range.first_row, range.last_row), (1, 2));
}

#[test]
fn a_viewport_that_ends_where_a_row_starts_does_not_include_it() {
    let layout = laid_out(&tall_sheet(100));
    let range = layout
        .visible_range(Rect {
            x: 0.0,
            y: 20.0,
            width: 64.0,
            height: 20.0,
        })
        .expect("range");
    assert_eq!((range.first_row, range.last_row), (1, 1));
}

#[test]
fn a_viewport_past_the_grid_sees_nothing() {
    let layout = laid_out(&tall_sheet(10));
    assert!(layout
        .visible_range(Rect {
            x: 0.0,
            y: 10_000.0,
            width: 64.0,
            height: 100.0,
        })
        .is_none());
    assert!(layout
        .visible_range(Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 100.0,
        })
        .is_none());
}

#[test]
fn a_hidden_row_does_not_swallow_the_range_around_it() {
    let mut sheet = tall_sheet(10);
    sheet.rows[2].hidden = true;
    let layout = laid_out(&sheet);

    // Row 2 is zero-height, so rows 3.. shift up by 20 px and the viewport that
    // used to land on row 3 now lands on row 4.
    let range = layout
        .visible_range(Rect {
            x: 0.0,
            y: 60.0,
            width: 64.0,
            height: 10.0,
        })
        .expect("range");
    assert_eq!((range.first_row, range.last_row), (4, 4));
}

#[test]
fn cells_in_a_viewport_are_the_ones_that_overlap_it() {
    let layout = laid_out(&tall_sheet(1000));
    let visible: Vec<u32> = layout
        .cells_in(Rect {
            x: 0.0,
            y: 400.0,
            width: 64.0,
            height: 100.0,
        })
        .map(|cell| cell.reference.row)
        .collect();
    assert_eq!(visible, vec![20, 21, 22, 23, 24]);
}

#[test]
fn a_merge_anchored_above_the_viewport_still_shows_in_it() {
    let mut sheet = tall_sheet(10);
    // A1:A4 is anchored on row 0 but reaches down into a viewport that starts
    // below it. Filtering by index range rather than by rectangle would lose it.
    sheet.merges = vec![CellRange::parse("A1:A4").expect("range")];
    let layout = laid_out(&sheet);

    let visible: Vec<u32> = layout
        .cells_in(Rect {
            x: 0.0,
            y: 50.0,
            width: 64.0,
            height: 10.0,
        })
        .map(|cell| cell.reference.row)
        .collect();
    assert_eq!(visible, vec![0]);
}

#[test]
fn laying_out_a_tall_sheet_stays_linear() {
    // Guards the row axis: looking each row up in the row list per index is
    // quadratic, and at this size that is a hundred million comparisons. The
    // assertion is that this returns at all within the test timeout, plus the
    // geometry being right at the far end.
    let layout = laid_out(&tall_sheet(10_000));
    assert_eq!(layout.rows.len(), 10_000);
    assert_eq!(layout.content_height, 200_000.0);
    assert_eq!(layout.row(9_999).expect("last row").offset, 199_980.0);
}
