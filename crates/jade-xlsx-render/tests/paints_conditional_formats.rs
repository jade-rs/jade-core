use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef};
use jade_xlsx_render::{render_viewport, RenderOptions};

#[test]
fn conditional_fills_and_unstored_blank_fills_reach_the_painter() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
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
    let painter = RecordingPainter::default();
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 300.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(painter.ops().contains(&Op::SetFillStyle("#c6efce".into())));
    assert!(painter.ops().contains(&Op::SetFillStyle("#ffc7ce".into())));
    assert!(painter.ops().contains(&Op::SetFillStyle("#ffff00".into())));
    let rect = layout.cell_rect(CellRef::parse("D2").unwrap()).unwrap();
    assert!(painter
        .ops()
        .contains(&Op::FillRect(rect.x, rect.y, rect.width, rect.height)));
    let ops = painter.ops();
    let grid = ops.iter().position(|op| matches!(op, Op::Stroke)).unwrap();
    let stored_fill = ops
        .iter()
        .position(|op| *op == Op::SetFillStyle("#c6efce".into()))
        .unwrap();
    let blank_fill = ops
        .iter()
        .position(|op| *op == Op::FillRect(rect.x, rect.y, rect.width, rect.height))
        .unwrap();
    assert!(grid < stored_fill && grid < blank_fill);
}

#[test]
fn underline_and_strike_are_painted_in_the_text_color_even_after_a_dashed_border() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    let mut layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let cell = layout
        .cells
        .iter_mut()
        .find(|cell| cell.reference == CellRef::new(0, 0))
        .unwrap();
    cell.font.underline = true;
    cell.font.strike = true;
    let rect = cell.rect;
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, rect, &RenderOptions::default()).unwrap();
    let ops = painter.ops();
    let text = ops
        .iter()
        .position(|op| matches!(op, Op::FillText(..)))
        .unwrap();
    assert_eq!(
        ops[text + 1],
        Op::SetLineWidth((11.0_f64 * 96.0 / 72.0 / 16.0).max(0.75))
    );
    assert_eq!(ops[text + 2], Op::SetLineDash(vec![]));
    assert_eq!(
        ops[text..]
            .iter()
            .filter(|op| matches!(op, Op::Stroke))
            .count(),
        2
    );
    assert!(ops.contains(&Op::SetStrokeStyle("#006100".into())));
}

#[test]
fn visual_fills_and_gradient_bars_use_layout_geometry_and_hide_only_the_painted_value() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/visual-conditional-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let painter = RecordingPainter::default();
    let rect = layout.cell_rect(CellRef::parse("C3").unwrap()).unwrap();
    render_viewport(&painter, &layout, rect, &RenderOptions::default()).unwrap();
    let ops = painter.ops();
    let bar = layout
        .cell(CellRef::parse("C3").unwrap())
        .unwrap()
        .data_bar
        .as_ref()
        .unwrap();
    assert!(ops.contains(&Op::HorizontalGradientRect(
        jade_painter::ImageRect {
            x: bar.rect.x,
            y: bar.rect.y,
            width: bar.rect.width,
            height: bar.rect.height
        },
        [99, 142, 198, 255],
        [255; 4]
    )));
    assert!(!ops.iter().any(|op| matches!(op, Op::FillText(..))));
    assert_eq!(
        layout.cell(CellRef::parse("C3").unwrap()).unwrap().text,
        "50"
    );
    let painter = RecordingPainter::default();
    let rect = layout.cell_rect(CellRef::parse("D3").unwrap()).unwrap();
    render_viewport(&painter, &layout, rect, &RenderOptions::default()).unwrap();
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op,Op::FillText(text,..) if text=="50")));
    let painter = RecordingPainter::default();
    let rect = layout.cell_rect(CellRef::parse("A3").unwrap()).unwrap();
    render_viewport(&painter, &layout, rect, &RenderOptions::default()).unwrap();
    assert!(painter.ops().contains(&Op::SetFillStyle("#808000".into())));
}

#[test]
fn dark_automatic_text_remains_readable_over_visuals_without_changing_authored_colors() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/visual-conditional-workbook.xlsx"
    ))
    .unwrap();
    let mut layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    for name in ["B3", "D5"] {
        let cell = layout.cell(CellRef::parse(name).unwrap()).unwrap();
        let painter = RecordingPainter::default();
        render_viewport(&painter, &layout, cell.rect, &RenderOptions::dark()).unwrap();
        assert!(painter
            .ops()
            .iter()
            .any(|op| matches!(op,Op::StrokeText(text,..) if text==&cell.text)));
        assert!(painter
            .ops()
            .contains(&Op::SetStrokeStyle("#000000".into())));
        assert!(painter
            .ops()
            .contains(&Op::SetFillStyle(RenderOptions::dark().default_text_color)));
    }
    let cell = layout
        .cells
        .iter_mut()
        .find(|cell| cell.reference == CellRef::parse("D5").unwrap())
        .unwrap();
    cell.text_color = Some(jade_xlsx_reader::Color::Argb([255, 255, 0, 0]));
    let rect = cell.rect;
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, rect, &RenderOptions::dark()).unwrap();
    assert!(!painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::StrokeText(..))));
    assert!(painter.ops().contains(&Op::SetFillStyle("#ff0000".into())));
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, rect, &RenderOptions::default()).unwrap();
    assert!(!painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::StrokeText(..))));
}

#[test]
fn modern_signed_bars_paint_solid_negative_fills_borders_axes_and_reversed_gradients() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/x14-bars-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let cell = layout.cell(CellRef::parse("A1").unwrap()).unwrap();
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, cell.rect, &RenderOptions::default()).unwrap();
    assert!(painter
        .ops()
        .contains(&Op::SetFillStyle("rgba(255, 0, 0, 1)".into())));
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::StrokeRect(..))));
    assert!(!painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::HorizontalGradientRect(..))));
    let cell = layout.cell(CellRef::parse("D5").unwrap()).unwrap();
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, cell.rect, &RenderOptions::default()).unwrap();
    assert!(painter.ops().iter().any(|op|matches!(op,Op::HorizontalGradientRect(_,start,end) if *start==[255;4] && *end==[99,142,198,255])));
    let cell = layout.cell(CellRef::parse("A3").unwrap()).unwrap();
    let painter = RecordingPainter::default();
    render_viewport(&painter, &layout, cell.rect, &RenderOptions::default()).unwrap();
    let axis = cell.data_bar.as_ref().unwrap().axis.as_ref().unwrap().rect;
    assert!(painter
        .ops()
        .contains(&Op::FillRect(axis.x, axis.y, axis.width, axis.height)));
}
