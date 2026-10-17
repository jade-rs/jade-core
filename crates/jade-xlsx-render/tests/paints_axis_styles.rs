use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellFormat, CellRef, Color, ColumnSpan, Fill};
use jade_xlsx_render::{render_viewport, RenderOptions};
#[test]
fn a_column_default_fill_is_painted_for_unstored_cells_in_each_viewport() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    book.styles.fills.push(Fill {
        pattern: Some("solid".into()),
        foreground: Some(Color::Argb([255, 51, 170, 102])),
        ..Default::default()
    });
    book.styles.cell_formats.push(CellFormat {
        fill_index: Some(0),
        ..Default::default()
    });
    book.sheets[0].columns.push(ColumnSpan {
        first: 1,
        last: 1,
        width_chars: None,
        custom_width: false,
        hidden: false,
        style_index: Some(0),
    });
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_row_count: 30,
            min_column_count: 12,
            ..Default::default()
        },
    );
    let at = layout.cell_rect(CellRef::new(1, 15)).unwrap();
    let painter = RecordingPainter::default();
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: at.x,
            y: at.y,
            width: 128.0,
            height: 60.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(painter.ops().contains(&Op::SetFillStyle("#33aa66".into())));
    assert!(painter
        .ops()
        .contains(&Op::FillRect(at.x, at.y, at.width, at.height)));
    let ops = painter.ops();
    let grid = ops.iter().position(|op| matches!(op, Op::Stroke)).unwrap();
    let fill = ops
        .iter()
        .position(|op| *op == Op::FillRect(at.x, at.y, at.width, at.height))
        .unwrap();
    assert!(grid < fill, "the fill must cover the gridline");
    assert!(layout.cells.len() < 20);
    assert!(painter.ops().len() < 250);
}
