use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef};
use jade_xlsx_render::{render_viewport, PainterMeasure, RenderOptions};
use std::rc::Rc;

#[test]
fn overflow_is_painted_from_an_offscreen_cell_using_the_shared_clip() {
    let workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    let measure = RecordingPainter::with_width_per_char(10.0);
    let layout = layout_sheet(
        &workbook.sheets[0],
        &workbook.styles,
        &LayoutOptions {
            min_column_count: 12,
            measure: Rc::new(PainterMeasure::new(measure)),
            ..Default::default()
        },
    );
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 64.0,
            y: 0.0,
            width: 64.0,
            height: 20.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    let cell = layout.cell(CellRef::new(0, 0)).unwrap();
    let clip = cell.text_rect();
    assert!(painter
        .ops()
        .contains(&Op::Rect(clip.x, clip.y, clip.width, clip.height)));
    assert_eq!(
        painter
            .count(|op| matches!(op, Op::FillText(text, x, _) if text == &cell.text && *x == 3.0)),
        1
    );
    assert_eq!(
        painter.count(|op| matches!(op, Op::FillText(text, _, _) if text == "STOP")),
        0
    );
}

#[test]
fn right_aligned_text_stays_anchored_to_its_own_cell_after_overflow() {
    let workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(
        &workbook.sheets[0],
        &workbook.styles,
        &LayoutOptions {
            min_column_count: 12,
            measure: Rc::new(PainterMeasure::new(RecordingPainter::with_width_per_char(
                10.0,
            ))),
            ..Default::default()
        },
    );
    let painter = RecordingPainter::with_width_per_char(10.0);
    let cell = layout.cell(CellRef::new(2, 2)).unwrap();
    render_viewport(&painter, &layout, cell.rect, &RenderOptions::default()).unwrap();
    let expected_x = cell.rect.right() - 3.0 - cell.text.chars().count() as f64 * 10.0;
    assert_eq!(
        painter.count(
            |op| matches!(op, Op::FillText(text, x, _) if text == &cell.text && *x == expected_x)
        ),
        1
    );
}

#[test]
fn an_overwide_number_never_paints_a_misleading_numeric_suffix() {
    let workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(
        &workbook.sheets[0],
        &workbook.styles,
        &LayoutOptions {
            min_column_count: 12,
            measure: Rc::new(PainterMeasure::new(RecordingPainter::with_width_per_char(
                10.0,
            ))),
            ..Default::default()
        },
    );
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: layout.rows[5].offset,
            width: 128.0,
            height: layout.rows[5].size,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    assert_eq!(
        painter.count(|op| matches!(op, Op::FillText(text, _, _) if text == "1E+14")),
        1
    );
    assert_eq!(
        painter.count(|op| matches!(op, Op::FillText(text, _, _) if text == "#####")),
        1
    );
    assert_eq!(
        painter
            .count(|op| matches!(op, Op::FillText(text, _, _) if text.contains("123456789012345"))),
        0
    );
}
