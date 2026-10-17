use jade_xlsx_layout::{layout_sheet, units, LayoutOptions};
use jade_xlsx_reader::{
    Alignment, Cell, CellRange, CellRef, CellValue, ColumnSpan, HorizontalAlignment, Row, Sheet,
    StyleTable,
};

fn cell(reference: &str, value: CellValue) -> Cell {
    Cell {
        reference: CellRef::parse(reference).expect("reference"),
        value,
        style_index: None,
        formula: None,
    }
}

fn sheet_with(rows: Vec<Row>) -> Sheet {
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows,
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: None,
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        shapes: Vec::new(),
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

#[test]
fn the_default_column_is_excels_64_pixels() {
    // 8.43 characters is Excel's default and renders as 64 px at 96 dpi. Getting
    // this one number wrong shifts every column on the sheet.
    assert_eq!(
        units::column_width_chars_to_px(units::DEFAULT_COLUMN_WIDTH_CHARS, 7.0),
        64.0
    );
}

#[test]
fn column_offsets_accumulate_declared_widths() {
    let mut sheet = sheet_with(vec![row(0, vec![cell("C1", CellValue::Number(1.0))])]);
    sheet.columns = vec![
        ColumnSpan {
            first: 0,
            last: 0,
            width_chars: Some(20.5),
            custom_width: true,
            hidden: false,
            style_index: None,
        },
        ColumnSpan {
            first: 1,
            last: 1,
            width_chars: Some(12.0),
            custom_width: true,
            hidden: false,
            style_index: None,
        },
    ];

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    let first = layout.column(0).expect("A");
    let second = layout.column(1).expect("B");
    let third = layout.column(2).expect("C");
    assert_eq!(first.offset, 0.0);
    assert_eq!(first.size, units::column_width_chars_to_px(20.5, 7.0));
    assert_eq!(second.offset, first.size);
    // A column the sheet says nothing about falls back to the sheet default.
    assert_eq!(third.size, 64.0);
    assert_eq!(layout.content_width, first.size + second.size + third.size);
}

#[test]
fn a_hidden_column_takes_no_space() {
    let mut sheet = sheet_with(vec![row(0, vec![cell("B1", CellValue::Number(1.0))])]);
    sheet.columns = vec![ColumnSpan {
        first: 0,
        last: 0,
        width_chars: Some(20.0),
        custom_width: true,
        hidden: true,
        style_index: None,
    }];

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    assert_eq!(layout.column(0).expect("A").size, 0.0);
    assert_eq!(layout.column(1).expect("B").offset, 0.0);
}

#[test]
fn row_heights_come_from_points() {
    let mut sheet = sheet_with(vec![
        Row {
            index: 0,
            height_points: Some(30.0),
            custom_height: true,
            custom_format: false,
            hidden: false,
            style_index: None,
            cells: vec![cell("A1", CellValue::Number(1.0))],
        },
        row(1, vec![cell("A2", CellValue::Number(2.0))]),
    ]);
    sheet.default_row_height_points = Some(18.75);

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    assert_eq!(layout.row(0).expect("1").size, 40.0);
    assert_eq!(layout.row(1).expect("2").offset, 40.0);
    assert_eq!(layout.row(1).expect("2").size, 25.0);
}

#[test]
fn a_merged_range_is_one_box_anchored_at_its_top_left() {
    let mut sheet = sheet_with(vec![row(
        0,
        vec![
            cell("A1", CellValue::Text("merged".into())),
            cell("B1", CellValue::Text("covered".into())),
        ],
    )]);
    sheet.merges = vec![CellRange::parse("A1:B1").expect("range")];

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    assert_eq!(layout.cells.len(), 1, "the covered cell must not paint");
    let anchor = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert_eq!(anchor.rect.width, 128.0);
    assert!(anchor.merge.is_some());
}

#[test]
fn a_merge_past_the_last_written_cell_still_has_geometry() {
    let mut sheet = sheet_with(vec![row(0, vec![cell("A1", CellValue::Number(1.0))])]);
    sheet.merges = vec![CellRange::parse("A3:C3").expect("range")];

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    assert_eq!(layout.columns.len(), 3);
    assert_eq!(layout.rows.len(), 3);
}

#[test]
fn general_alignment_follows_the_value_type() {
    let sheet = sheet_with(vec![row(
        0,
        vec![
            cell("A1", CellValue::Text("text".into())),
            cell("B1", CellValue::Number(1.0)),
            cell("C1", CellValue::Bool(true)),
        ],
    )]);

    let layout = layout_sheet(&sheet, &StyleTable::default(), &LayoutOptions::default());

    let alignment = |reference: CellRef| layout.cell(reference).expect("cell").alignment.horizontal;
    assert_eq!(alignment(CellRef::new(0, 0)), HorizontalAlignment::Left);
    assert_eq!(alignment(CellRef::new(1, 0)), HorizontalAlignment::Right);
    assert_eq!(alignment(CellRef::new(2, 0)), HorizontalAlignment::Center);
}

#[test]
fn an_explicit_alignment_wins_over_the_value_type() {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(jade_xlsx_reader::CellFormat {
        alignment: Alignment {
            horizontal: HorizontalAlignment::Left,
            ..Alignment::default()
        },
        ..Default::default()
    });
    let mut sheet = sheet_with(vec![row(0, vec![cell("A1", CellValue::Number(1.0))])]);
    sheet.rows[0].cells[0].style_index = Some(0);

    let layout = layout_sheet(&sheet, &styles, &LayoutOptions::default());

    assert_eq!(
        layout
            .cell(CellRef::new(0, 0))
            .expect("A1")
            .alignment
            .horizontal,
        HorizontalAlignment::Left
    );
}

#[test]
fn zero_area_rectangles_never_intersect_a_viewport_or_frozen_region() {
    use jade_xlsx_layout::Rect;
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        width: 300.0,
        height: 200.0,
    };
    for rect in [
        Rect {
            x: 50.0,
            y: 40.0,
            width: 20.0,
            height: 0.0,
        },
        Rect {
            x: 50.0,
            y: 40.0,
            width: 0.0,
            height: 20.0,
        },
        Rect {
            x: 50.0,
            y: 40.0,
            width: -1.0,
            height: 20.0,
        },
    ] {
        assert!(!rect.intersects(&viewport));
        assert!(!viewport.intersects(&rect));
    }
}
