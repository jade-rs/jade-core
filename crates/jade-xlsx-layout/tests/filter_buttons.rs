use jade_xlsx_layout::filter_button::FilterButtonTarget;
use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::{auto_filter::FilterColumn, read_xlsx, CellRange, CellRef, CellValue};
fn fixture() -> jade_xlsx_reader::Workbook {
    read_xlsx(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap()
}
#[test]
fn table_owner_wins_overlapping_headers_and_missing_column_metadata_defaults_visible() {
    let mut book = fixture();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    assert_eq!(layout.filter_buttons.len(), 3);
    for (at, button) in layout.filter_buttons.iter().enumerate() {
        assert_eq!(button.column, at as u32);
        assert!(
            matches!(&button.target,FilterButtonTarget::Table{name} if name=="OperationsTable")
        );
        assert!(!button.active);
        assert_eq!(
            layout.filter_button_at(button.rect.x + 2.0, button.rect.y + 2.0),
            Some(button)
        );
    }
    book.sheets[0].tables.clear();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    assert!(layout.filter_buttons[0].active);
    assert!(!layout.filter_buttons[1].active);
    assert_eq!(
        layout
            .filter_button_for_cell(CellRef::new(0, 3))
            .unwrap()
            .column,
        0
    );
    assert!(layout.filter_button_for_cell(CellRef::new(0, 5)).is_none());
    book.sheets[0].filter_mode = Some(false);
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    assert!(!layout.filter_buttons[0].active);
}
#[test]
fn saved_button_flags_hidden_axes_and_headerless_tables_do_not_leave_ghost_hits() {
    let mut book = fixture();
    let filter = book.sheets[0].tables[0].filter.as_mut().unwrap();
    filter.columns = vec![
        FilterColumn {
            column_id: 0,
            hidden_button: true,
            show_button: true,
            criterion: None,
        },
        FilterColumn {
            column_id: 1,
            hidden_button: false,
            show_button: false,
            criterion: None,
        },
    ];
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    assert_eq!(layout.filter_buttons.len(), 1);
    assert_eq!(layout.filter_buttons[0].reference, CellRef::new(2, 0));
    assert!(layout.filter_button_for_cell(CellRef::new(0, 2)).is_none());
    book.sheets[0].auto_filter = None;
    book.sheets[0].rows[0].hidden = true;
    assert!(
        layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default())
            .filter_buttons
            .is_empty()
    );
    book.sheets[0].rows[0].hidden = false;
    book.sheets[0].tables[0].header_row_count = 0;
    assert!(
        layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default())
            .filter_buttons
            .is_empty()
    );
}
#[test]
fn unstored_headers_have_button_axes_and_text_clips_reserve_space_without_changing_cell_geometry() {
    let mut book = fixture();
    book.sheets[0].tables.clear();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let cell = layout.cell(CellRef::new(1, 0)).unwrap();
    let button = &layout.filter_buttons[1];
    assert_eq!(cell.text_rect().right(), button.rect.x);
    assert!(cell.text_rect().width < cell.rect.width);
    let original = cell.rect;
    book.sheets[0].auto_filter = None;
    let without = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    assert_eq!(without.cell(CellRef::new(1, 0)).unwrap().rect, original);
    book.sheets[0].auto_filter = fixture().sheets[0].auto_filter.clone();
    let filter = book.sheets[0].auto_filter.as_mut().unwrap();
    filter.reference = Some(CellRange::parse("AZ50:BA52").unwrap());
    filter.columns.clear();
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_row_count: 1,
            min_column_count: 1,
            ..Default::default()
        },
    );
    assert_eq!(layout.filter_buttons.len(), 2);
    assert!(layout.column(52).is_some());
    assert!(layout.row(49).is_some());
    assert!(layout.cell(CellRef::new(51, 49)).is_none());
}
#[test]
fn frozen_header_hit_mapping_and_header_numeric_fitting_use_resolved_geometry() {
    let mut book = fixture();
    book.sheets[0].tables.clear();
    book.sheets[0].freeze_pane = Some(jade_xlsx_reader::FreezePane {
        frozen_columns: 1,
        frozen_rows: 1,
        top_left_cell: None,
    });
    let cell = &mut book.sheets[0].rows[0].cells[1];
    cell.value = CellValue::Number(123456789012345.0);
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let button = &layout.filter_buttons[0];
    let (x, y) =
        layout.sheet_point_in_viewport(button.rect.x + 4.0, button.rect.y + 4.0, 100.0, 100.0);
    assert_eq!(layout.filter_button_at(x, y), Some(button));
    // General: twelve or more integer digits are scientific, as in Excel.
    assert_eq!(layout.cell(CellRef::new(1, 0)).unwrap().text, "1.23457E+14");
    assert_ne!(
        layout.cell(CellRef::new(1, 0)).unwrap().lines[0],
        "123456789012345"
    );
}
