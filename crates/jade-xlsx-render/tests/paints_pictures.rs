use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::read_xlsx;
use jade_xlsx_render::{render_viewport, RenderOptions};
#[test]
fn images_use_the_shared_painter_with_crops_rotation_flip_and_opacity() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/pictures-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let painter = RecordingPainter::default();
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 220.0,
        },
        &RenderOptions::default(),
    )
    .unwrap();
    let ops = painter.ops();
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Op::DrawImage { .. }))
            .count(),
        8
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        Op::DrawImage {
            source: Some([0.5, 0.0, 0.5, 1.0]),
            ..
        }
    )));
    assert!(ops.contains(&Op::Rotate(std::f64::consts::FRAC_PI_2)));
    assert!(ops.contains(&Op::Scale(-1.0, 1.0)));
    assert!(ops.contains(&Op::SetGlobalAlpha(0.5)));
    assert_eq!(
        ops.iter().filter(|op| matches!(op, Op::Save)).count(),
        ops.iter().filter(|op| matches!(op, Op::Restore)).count()
    );
}

#[test]
fn unsupported_effects_fall_back_to_frames_and_fully_cropped_images_are_invisible() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/pictures-workbook.xlsx"
    ))
    .unwrap();
    book.sheets[0].shapes.truncate(1);
    book.sheets[0].shapes[0]
        .picture
        .as_mut()
        .unwrap()
        .unsupported_properties
        .push("tile".into());
    let paint = |book: &jade_xlsx_reader::Workbook| {
        let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
        let painter = RecordingPainter::default();
        render_viewport(
            &painter,
            &layout,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 100.0,
            },
            &RenderOptions::default(),
        )
        .unwrap();
        painter.ops().to_vec()
    };
    let ops = paint(&book);
    assert!(!ops.iter().any(|op| matches!(op, Op::DrawImage { .. })));
    assert!(ops
        .iter()
        .any(|op| matches!(op,Op::FillText(text,..) if text.contains("Original"))));
    let picture = book.sheets[0].shapes[0].picture.as_mut().unwrap();
    picture.unsupported_properties.clear();
    picture.crop = [100000, 0, 0, 0];
    let ops = paint(&book);
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::DrawImage { .. } | Op::FillText(..))));
}
