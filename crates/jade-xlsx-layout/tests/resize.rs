use jade_xlsx_layout::{
    layout_sheet,
    resize::{auto_fit_columns, header_resize_hit, ResizeGesture},
    units, LayoutOptions,
};
use jade_xlsx_reader::{read_xlsx, Axis, CellValue};
const SOURCE: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
#[test]
fn boundary_hits_use_raw_axes_and_frozen_pane_positions() {
    let book = read_xlsx(SOURCE).unwrap();
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let hit = header_resize_hit(&layout, 88.0, 8.0, 128.0, 100.0).unwrap();
    assert_eq!(hit.axis, Axis::Columns);
    assert_eq!(hit.index, 0);
    let hit = header_resize_hit(&layout, 152.0, 8.0, 128.0, 100.0).unwrap();
    assert_eq!(hit.index, 3);
    assert!(header_resize_hit(&layout, 120.0, 8.0, 0.0, 0.0).is_none());
    let hit = header_resize_hit(&layout, 8.0, 40.0, 0.0, 100.0).unwrap();
    assert_eq!(hit.axis, Axis::Rows);
    assert_eq!(hit.index, 0);
}
#[test]
fn pixel_conversion_round_trips_and_resize_gestures_use_total_delta() {
    for pixels in [0.0, 1.0, 6.0, 12.0, 64.0, 90.0, 200.0] {
        let width = units::column_width_px_to_chars(pixels, 7.0);
        assert_eq!(units::column_width_chars_to_px(width, 7.0), pixels);
    }
    let hit = jade_xlsx_layout::resize::ResizeHit {
        axis: Axis::Columns,
        index: 1,
        size_px: 64.0,
    };
    let mut gesture = ResizeGesture {
        hit,
        pointer_start: 150.0,
        size_px: 64.0,
    };
    gesture.update(160.0, 0.0);
    gesture.update(180.0, 0.0);
    assert_eq!(gesture.size_px, 94.0);
    let mut row = ResizeGesture {
        hit: jade_xlsx_layout::resize::ResizeHit {
            axis: Axis::Rows,
            index: 0,
            size_px: 20.0,
        },
        pointer_start: 40.0075,
        size_px: 20.0,
    };
    row.update(0.0, 70.0);
    assert_eq!(row.stored_size(7.0), 37.5);
}

#[test]
fn a_hidden_column_at_a_visible_boundary_can_be_revealed() {
    let mut book = read_xlsx(SOURCE).unwrap();
    book.sheets[0].columns.push(jade_xlsx_reader::ColumnSpan {
        first: 1,
        last: 1,
        width_chars: Some(10.0),
        custom_width: true,
        hidden: true,
        style_index: None,
    });
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            ..Default::default()
        },
    );
    let hit = header_resize_hit(&layout, 88.0, 8.0, 0.0, 0.0).unwrap();
    assert_eq!(hit.index, 1);
    assert_eq!(hit.size_px, 0.0);
}
#[test]
fn auto_fit_uses_full_numeric_text_and_ignores_merged_surfaces() {
    let mut book = read_xlsx(SOURCE).unwrap();
    book.sheets[0].rows[1].cells[1].value = CellValue::Number(123456789012.0);
    let options = LayoutOptions::default();
    let widths = auto_fit_columns(&book.sheets[0], &book.styles, 1, 5, &options);
    assert!(widths[&1] > 8.43);
    assert_eq!(
        widths[&4],
        units::DEFAULT_COLUMN_WIDTH_CHARS,
        "merged header must not expand an individual column"
    );
    assert_eq!(widths[&5], units::DEFAULT_COLUMN_WIDTH_CHARS);
}

#[test]
fn hidden_defaults_do_not_walk_to_the_end_of_the_worksheet_for_a_viewport() {
    let sheet = jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="25" customHeight="1" zeroHeight="1"/><sheetData><row r="3" hidden="0" ht="30"><c r="A3" t="inlineStr"><is><t>Visible</t></is></c></row></sheetData></worksheet>"#, &[]).unwrap();
    let layout = layout_sheet(
        &sheet,
        &Default::default(),
        &LayoutOptions {
            min_row_count: 30,
            ..Default::default()
        },
    );
    assert_eq!(layout.rows.len(), 30);
    assert!(layout.rows[0].hidden);
    assert!(!layout.rows[2].hidden);
    assert_eq!(layout.rows[2].offset, 0.0);
    assert_eq!(layout.rows[2].size, 40.0);
    assert_eq!(layout.content_height, 40.0);
    assert_eq!(
        layout
            .cells_in(jade_xlsx_layout::Rect {
                x: 0.0,
                y: -1.0,
                width: 500.0,
                height: 300.0
            })
            .count(),
        1
    );
}

#[test]
fn a_pinned_default_height_clips_wrapped_content_until_explicit_auto_fit() {
    use jade_xlsx_reader::{Alignment, CellFormat, StyleTable};
    let sheet = jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="15" customHeight="1"/><cols><col min="1" max="1" width="5"/></cols><sheetData><row r="1"><c r="A1" s="0" t="inlineStr"><is><t>many words wrapping across several lines</t></is></c></row><row r="2" customHeight="0"><c r="A2" s="0" t="inlineStr"><is><t>many words wrapping across several lines</t></is></c></row></sheetData></worksheet>"#, &[]).unwrap();
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            wrap_text: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let layout = layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            min_row_count: 3,
            ..Default::default()
        },
    );
    assert_eq!(layout.rows[0].size, 20.0);
    assert!(layout.rows[1].size > 20.0);
    assert_eq!(layout.rows[2].size, 20.0);
    let fits = jade_xlsx_layout::resize::auto_fit_rows(&sheet, &styles, 0, 0, &Default::default());
    assert!(fits[&0] > 15.0);
}

#[test]
fn auto_fit_can_shrink_a_manually_pinned_worksheet_default() {
    let sheet = jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="40" customHeight="1"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Short</t></is></c></row></sheetData></worksheet>"#, &[]).unwrap();
    let before = layout_sheet(&sheet, &Default::default(), &Default::default());
    assert_eq!(before.rows[0].size, units::points_to_px(40.0));
    let heights = jade_xlsx_layout::resize::auto_fit_rows(
        &sheet,
        &Default::default(),
        0,
        1,
        &Default::default(),
    );
    assert_eq!(heights[&0], 15.0);
    assert_eq!(heights[&1], 15.0);
    assert_eq!(sheet.default_row_height_points, Some(40.0));
}

#[test]
fn whole_sheet_fit_uses_only_content_exceptions_and_matches_partial_measurement() {
    let sheet = jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="40" customHeight="1" zeroHeight="1"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Short</t></is></c></row><row r="2"><c r="A2" s="0" t="inlineStr"><is><t>one&#10;two&#10;three</t></is></c></row><row r="3"><c r="C3" s="0" t="inlineStr"><is><t>merged&#10;text&#10;ignored</t></is></c></row></sheetData><mergeCells count="1"><mergeCell ref="C3:E3"/></mergeCells></worksheet>"#, &[]).unwrap();
    let mut styles = jade_xlsx_reader::StyleTable::default();
    styles.cell_formats.push(jade_xlsx_reader::CellFormat {
        alignment: jade_xlsx_reader::Alignment {
            wrap_text: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let plan = jade_xlsx_layout::resize::auto_fit_all_rows(&sheet, &styles, &Default::default());
    assert_eq!(plan.default_points, 15.0);
    assert_eq!(
        plan.overrides,
        std::collections::BTreeMap::from([(1, 45.0)])
    );
    let partial =
        jade_xlsx_layout::resize::auto_fit_rows(&sheet, &styles, 0, 9, &Default::default());
    for (&row, &height) in &partial {
        assert_eq!(
            height,
            plan.overrides
                .get(&row)
                .copied()
                .unwrap_or(plan.default_points)
        );
    }
    assert!(sheet.default_rows_hidden && sheet.default_row_custom_height);
}
