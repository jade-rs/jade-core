use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_workbook_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::read_xlsx;
use jade_xlsx_render::{render_viewport, RenderOptions};
#[test]
fn chart_geometry_reaches_the_shared_painter_with_balanced_clips_and_resolved_colors() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/charts-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap();
    let painter = RecordingPainter::default();
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 190.0,
            y: 0.0,
            width: 850.0,
            height: 630.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    let ops = painter.ops();
    assert!(ops.contains(&Op::SetFillStyle("#00aa55".into())));
    assert!(ops.contains(&Op::SetFillStyle("#6633ff".into())));
    assert!(ops
        .iter()
        .any(|op| matches!(op,Op::FillText(text,..) if text=="Quarter & Plan")));
    assert_eq!(
        ops.iter().filter(|op| matches!(op, Op::Save)).count(),
        ops.iter().filter(|op| matches!(op, Op::Restore)).count()
    );
    assert!(ops.iter().filter(|op| matches!(op, Op::Clip)).count() > 4);
}
