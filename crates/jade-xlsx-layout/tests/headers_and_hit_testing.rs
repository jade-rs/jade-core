//! Column names, header bands, and finding the cell under a point.

use jade_xlsx_layout::{
    column_label, layout_sheet, HeaderMetrics, LayoutOptions, Rect, SheetLayout,
};
use jade_xlsx_reader::{Cell, CellRange, CellRef, CellValue, FreezePane, Row, Sheet, StyleTable};

fn sheet(rows: u32, merges: Vec<&str>) -> Sheet {
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: (0..rows)
            .map(|index| Row {
                index,
                height_points: None,
                custom_height: false,
                custom_format: false,
                hidden: false,
                style_index: None,
                cells: vec![Cell {
                    reference: CellRef::new(0, index),
                    value: CellValue::Number(f64::from(index)),
                    style_index: None,
                    formula: None,
                }],
            })
            .collect(),
        merges: merges
            .into_iter()
            .map(|range| CellRange::parse(range).expect("range"))
            .collect(),
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
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
    }
}

fn laid_out(sheet: &Sheet) -> SheetLayout {
    layout_sheet(
        sheet,
        &StyleTable::default(),
        &LayoutOptions {
            min_column_count: 6,
            ..LayoutOptions::default()
        },
    )
}

#[test]
fn column_labels_are_base_26_with_no_zero() {
    // After Z comes AA, not BA — which is why this is not a radix conversion.
    assert_eq!(column_label(0), "A");
    assert_eq!(column_label(25), "Z");
    assert_eq!(column_label(26), "AA");
    assert_eq!(column_label(27), "AB");
    assert_eq!(column_label(51), "AZ");
    assert_eq!(column_label(52), "BA");
    assert_eq!(column_label(701), "ZZ");
    assert_eq!(column_label(702), "AAA");
    // Excel's last column.
    assert_eq!(column_label(16_383), "XFD");
}

#[test]
fn the_row_band_widens_with_the_number_of_digits() {
    let short = HeaderMetrics::for_rows(30);
    let long = HeaderMetrics::for_rows(10_000);
    assert!(
        long.row_width > short.row_width,
        "a five-digit row number needs more room than a two-digit one"
    );
    assert_eq!(short.column_height, long.column_height);
}

#[test]
fn a_point_lands_on_the_cell_under_it() {
    let layout = laid_out(&sheet(10, Vec::new()));
    // Default geometry: 64 px columns, 20 px rows.
    assert_eq!(layout.cell_at(0.0, 0.0), Some(CellRef::new(0, 0)));
    assert_eq!(layout.cell_at(63.9, 19.9), Some(CellRef::new(0, 0)));
    assert_eq!(layout.cell_at(64.0, 20.0), Some(CellRef::new(1, 1)));
    assert_eq!(layout.cell_at(130.0, 45.0), Some(CellRef::new(2, 2)));
}

#[test]
fn a_point_outside_the_grid_lands_on_nothing() {
    let layout = laid_out(&sheet(10, Vec::new()));
    assert_eq!(layout.cell_at(-1.0, 5.0), None);
    assert_eq!(layout.cell_at(5.0, -1.0), None);
    assert_eq!(layout.cell_at(100_000.0, 5.0), None);
    assert_eq!(layout.cell_at(5.0, 100_000.0), None);
}

#[test]
fn a_point_inside_a_merge_lands_on_its_anchor() {
    // Excel selects the whole merged range wherever you click in it, and the
    // anchor is what identifies that range.
    let layout = laid_out(&sheet(10, vec!["A1:C2"]));
    assert_eq!(layout.cell_at(10.0, 10.0), Some(CellRef::new(0, 0)));
    assert_eq!(layout.cell_at(150.0, 30.0), Some(CellRef::new(0, 0)));
    // Just past the merge, the cells are their own again.
    assert_eq!(layout.cell_at(200.0, 30.0), Some(CellRef::new(3, 1)));
}

#[test]
fn a_hidden_row_cannot_be_landed_on() {
    let mut sheet = sheet(10, Vec::new());
    sheet.rows[2].hidden = true;
    let layout = laid_out(&sheet);

    // Row 2 has no height, so the point that would have been in it is row 3.
    assert_eq!(layout.cell_at(10.0, 40.0), Some(CellRef::new(0, 3)));
}

#[test]
fn frozen_boundaries_define_render_and_hit_test_regions_once() {
    let mut sheet = sheet(100, Vec::new());
    sheet.freeze_pane = Some(FreezePane {
        frozen_columns: 1,
        frozen_rows: 1,
        top_left_cell: Some(CellRef::new(1, 1)),
    });
    let layout = laid_out(&sheet);
    let regions = layout.viewport_regions(Rect {
        x: 128.0,
        y: 400.0,
        width: 300.0,
        height: 120.0,
    });

    assert_eq!(regions.frozen_width, 64.0);
    assert_eq!(regions.frozen_height, 20.0);
    assert_eq!(regions.main.x, 192.0);
    assert_eq!(regions.main.y, 420.0);
    assert_eq!(regions.main.width, 236.0);
    assert_eq!(regions.main.height, 100.0);
    assert_eq!(
        layout.sheet_point_in_viewport(10.0, 10.0, 128.0, 400.0),
        (10.0, 10.0)
    );
    assert_eq!(
        layout.sheet_point_in_viewport(80.0, 30.0, 128.0, 400.0),
        (208.0, 430.0)
    );
}
