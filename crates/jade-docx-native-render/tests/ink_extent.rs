//! A perfectly straight pen stroke — vertical or horizontal — has no extent
//! of its own across it. Its ink drawing still gets a box as wide as the
//! stroke, and the page shows the stroke, not a box-sized patch of ink.

mod common;
use common::open_bytes;
use jade_docx_editor::commands::set_ink_drawings::SetInkDrawingsCommand;
use jade_docx_editor::inkml::{InkBrush, InkTool, InkTrace};
use jade_docx_editor::word_document::ink_drawing::InkDrawingSpec;

/// Pixels of `bgra` in the stroke's red, inside and outside `area`
/// (left, top, right, bottom).
fn red_pixels(bgra: &[u8], width: i32, area: (i32, i32, i32, i32)) -> (usize, usize) {
    let (mut inside, mut outside) = (0, 0);
    for (index, pixel) in bgra.chunks(4).enumerate() {
        let (b, g, r) = (pixel[0], pixel[1], pixel[2]);
        if !(r > 150 && g < 120 && b < 120) {
            continue;
        }
        let (x, y) = (index as i32 % width, index as i32 / width);
        if x >= area.0 && x <= area.2 && y >= area.1 && y <= area.3 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

#[test]
fn straight_strokes_are_drawn_as_strokes_in_a_box_as_wide_as_the_stroke() {
    for (points, area) in [
        (
            vec![(300.0, 300.0), (300.0, 400.0), (300.0, 500.0)],
            (296, 296, 304, 504),
        ),
        (
            vec![(200.0, 600.0), (300.0, 600.0), (400.0, 600.0)],
            (196, 596, 404, 604),
        ),
    ] {
        let mut document = open_bytes(&std::fs::read(common::repo_path("contract.docx")).unwrap());
        let trace = InkTrace {
            points,
            brush: InkBrush {
                tool: InkTool::Pen,
                color: "#E71224".into(),
                width_px: 3.0,
                height_px: 3.0,
                effect: None,
                transparency: 0,
            },
        };
        document
            .execute_command(
                Box::new(SetInkDrawingsCommand::new(vec![InkDrawingSpec::New {
                    paragraph_index: 0,
                    traces: vec![trace],
                }])),
                false,
            )
            .unwrap();
        let drawings = document.word_document().ink_drawings();
        let drawing = &drawings[0];
        assert!(
            drawing.width_px.min(drawing.height_px) >= 3.0 - 1e-9,
            "the box is at least the stroke's width: {drawing:?}"
        );
        let reopened = open_bytes(&document.export_docx().unwrap());
        for document in [document, reopened] {
            let (pixels, width, _) = document.page_pixels(0).unwrap();
            let (inside, outside) = red_pixels(&pixels, width, area);
            assert!(inside > 300, "the stroke is drawn: {inside}");
            assert_eq!(outside, 0, "and nothing else in its colour");
        }
    }
}
