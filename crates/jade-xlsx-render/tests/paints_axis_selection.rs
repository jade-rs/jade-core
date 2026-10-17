use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{
    layout_sheet,
    selection::{SelectionArea, SelectionKind},
    LayoutOptions, Rect,
};
use jade_xlsx_reader::{read_xlsx, CellRef};
use jade_xlsx_render::{render_grid, GridSelection, RenderOptions};
#[test]
fn a_full_column_selection_paints_only_existing_axes_and_has_no_fill_handle() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let area = SelectionArea {
        kind: SelectionKind::Columns,
        anchor: CellRef::new(1, 1),
        focus: CellRef::new(1, 1),
    };
    let range = area.range();
    let mut selection = GridSelection::range(range.start, range.end).without_fill_handle();
    selection.active = area.anchor;
    let painter = RecordingPainter::default();
    render_grid(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 200.0,
        },
        Some(selection),
        &RenderOptions::default(),
    )
    .unwrap();
    let ops = painter.ops();
    assert!(ops
        .iter()
        .any(|op| matches!(op,Op::StrokeRect(_,_,width,height) if *width==64.0 && *height>=600.0)));
    assert!(!ops
        .iter()
        .any(|op| matches!(op,Op::FillRect(_,_,w,h) if *w==6.0&&*h==6.0)));
    assert!(ops.len() < 4000);
}
