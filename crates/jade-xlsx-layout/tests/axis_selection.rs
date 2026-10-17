use jade_xlsx_layout::{
    layout_sheet,
    selection::{header_hit, occupied_cells, SelectionArea, SelectionKind},
    LayoutOptions,
};
use jade_xlsx_reader::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    read_xlsx, CellRange, CellRef,
};
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
#[test]
fn axes_have_full_logical_bounds_and_clipboard_extents_are_bounded() {
    let book = read_xlsx(BYTES).unwrap();
    let sheet = &book.sheets[0];
    let rows = SelectionArea {
        kind: SelectionKind::Rows,
        anchor: CellRef::new(2, 1),
        focus: CellRef::new(2, 3),
    };
    assert_eq!(
        rows.range(),
        CellRange {
            start: CellRef::new(0, 1),
            end: CellRef::new(MAX_COLUMN_COUNT - 1, 3)
        }
    );
    assert_eq!(rows.clipboard_range(sheet).end.column, 4);
    let cols = SelectionArea {
        kind: SelectionKind::Columns,
        anchor: CellRef::new(1, 2),
        focus: CellRef::new(2, 2),
    };
    assert_eq!(cols.range().end.row, MAX_ROW_COUNT - 1);
    assert_eq!(cols.clipboard_range(sheet).end.row, 3);
    assert_eq!(occupied_cells(sheet, rows.range()).len(), 9);
    let sheet_area = SelectionArea {
        kind: SelectionKind::Sheet,
        ..rows
    };
    assert_eq!(sheet_area.clipboard_range(sheet).end, CellRef::new(4, 3));
}
#[test]
fn header_hit_uses_raw_axes_even_over_a_merge_and_handles_frozen_scroll() {
    let book = read_xlsx(BYTES).unwrap();
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let m = layout.header_metrics();
    let f = header_hit(&layout, m.row_width + 5.0 * 64.0 + 8.0, 8.0, 0.0, 0.0).unwrap();
    assert_eq!(f.kind, SelectionKind::Columns);
    assert_eq!(f.reference.column, 5);
    let a = header_hit(&layout, m.row_width + 8.0, 8.0, 128.0, 40.0).unwrap();
    assert_eq!(a.reference.column, 0);
    let d = header_hit(&layout, m.row_width + 80.0, 8.0, 128.0, 40.0).unwrap();
    assert_eq!(d.reference.column, 3);
    let row = header_hit(&layout, 8.0, m.column_height + 30.0, 128.0, 40.0).unwrap();
    assert_eq!(row.kind, SelectionKind::Rows);
    assert_eq!(row.reference.row, 3);
    assert_eq!(
        header_hit(&layout, 8.0, 8.0, 0.0, 0.0).unwrap().kind,
        SelectionKind::Sheet
    );
}
