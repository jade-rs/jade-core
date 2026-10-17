use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use jade_xlsx_render::{render_viewport, RenderOptions};
#[test]
fn header_controls_paint_after_text_use_theme_colors_and_skip_other_windows() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    book.sheets[0].tables.clear();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    for options in [RenderOptions::default(), RenderOptions::dark()] {
        let painter = RecordingPainter::default();
        render_viewport(
            &painter,
            &layout,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
            &options,
        )
        .unwrap();
        let ops = painter.ops();
        let button = &layout.filter_buttons[0];
        let fill = ops
            .iter()
            .position(|op| {
                *op == Op::FillRect(
                    button.rect.x,
                    button.rect.y,
                    button.rect.width,
                    button.rect.height,
                )
            })
            .unwrap();
        let text = ops
            .iter()
            .rposition(|op| matches!(op, Op::FillText(..)))
            .unwrap();
        assert!(fill > text);
        assert!(ops.contains(&Op::SetFillStyle(options.selection_border.clone())));
        assert!(ops.contains(&Op::SetFillStyle(options.header.text_color.clone())));
        let away = RecordingPainter::default();
        render_viewport(
            &away,
            &layout,
            Rect {
                x: 0.0,
                y: 80.0,
                width: 400.0,
                height: 200.0,
            },
            &options,
        )
        .unwrap();
        assert!(!away.ops().contains(&Op::FillRect(
            button.rect.x,
            button.rect.y,
            button.rect.width,
            button.rect.height
        )));
    }
}
#[test]
fn right_aligned_headers_fit_before_the_button_and_keep_full_values() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    book.sheets[0].tables.clear();
    book.sheets[0].rows[0].cells[1].value = CellValue::Number(123.0);
    let mut layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let cell = layout
        .cells
        .iter_mut()
        .find(|cell| cell.reference == CellRef::new(1, 0))
        .unwrap();
    cell.alignment.horizontal = jade_xlsx_reader::HorizontalAlignment::Right;
    assert_eq!(cell.text, "123");
    let right = cell.text_rect().right() - layout.cell_padding_px;
    let painter = RecordingPainter::default();
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 100.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op,Op::FillText(text,x,_) if text=="123" && (*x-right).abs()<1e-6)));
}
