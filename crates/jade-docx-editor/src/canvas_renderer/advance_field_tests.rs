//! Word's ADVANCE field (§17.16.5.3): it shows nothing, and moves the text
//! after it — right or left, up or down, to a distance along the line, or the
//! whole line to a position down the page.

use super::recording_host::RecordingHost;
use super::*;
use crate::word_document::WordDocument;
use jade_docx_reader::{RunField, RunFieldKind};
use jade_painter::recording::{Op, RecordingPainter};

/// hello.docx's first paragraph as `ab`, with `ADVANCE <switches>` between
/// the letters when given.
fn render(switches: Option<&str>) -> Vec<Op> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let bytes = std::fs::read(path).expect("hello.docx");
    let model =
        futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
    let mut document = (**model.document.as_ref().expect("document")).clone();
    let mut first = true;
    for element in document
        .body
        .iter_mut()
        .flat_map(|body| body.elements.iter_mut())
    {
        let Some(paragraph) = element.as_paragraph_mut() else {
            continue;
        };
        if !first {
            paragraph.runs.clear();
            continue;
        }
        first = false;
        let mut a = paragraph.runs[0].clone();
        a.text = "a".into();
        a.field = None;
        let mut b = a.clone();
        b.text = "b".into();
        let mut runs = vec![a.clone()];
        if let Some(switches) = switches {
            let mut field = a.clone();
            field.text = String::new();
            field.field = Some(RunField {
                kind: RunFieldKind::Unsupported {
                    instruction: format!("ADVANCE {switches}"),
                },
                source_field_id: None,
                cached_text: None,
                resolved_value: None,
                word_mac_body_result: None,
            });
            runs.push(field);
        }
        runs.push(b);
        paragraph.runs = runs;
    }
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    renderer
        .render(&Rc::new(document), &Options::default())
        .expect("render");
    painter.ops()
}

/// Where `text` was drawn.
fn drawn_at(ops: &[Op], text: &str) -> (f64, f64) {
    ops.iter()
        .find_map(|op| match op {
            Op::FillText(drawn, x, y) if drawn == text => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text} drawn"))
}

#[test]
fn advance_moves_the_text_after_it() {
    let plain = render(None);
    let (ax, ay) = drawn_at(&plain, "a");
    let (bx, by) = drawn_at(&plain, "b");
    assert_eq!(ay, by);

    // 30pt right, 6pt down: 40px and 8px.
    let moved = render(Some("\\r 30 \\d 6"));
    assert_eq!(drawn_at(&moved, "a"), (ax, ay), "the text before stays");
    let (x, y) = drawn_at(&moved, "b");
    assert!((x - (bx + 40.0)).abs() < 0.01, "{x} vs {bx}");
    assert!((y - (by + 8.0)).abs() < 0.01, "{y} vs {by}");

    // Left and up: the raised text makes the line taller above, so it is
    // measured against the letter before it.
    let left_up = render(Some("\\l 3 \\u 3"));
    let (x, y) = drawn_at(&left_up, "b");
    let (_, a_y) = drawn_at(&left_up, "a");
    assert!((x - (bx - 4.0)).abs() < 0.01, "{x} vs {bx}");
    assert!((y - (a_y - 4.0)).abs() < 0.01, "{y} vs {a_y}");

    // \x: 72pt from where the line's text starts.
    let (x, _) = drawn_at(&render(Some("\\x 72")), "b");
    assert!((x - (ax + 96.0)).abs() < 0.01, "{x} vs {ax}");
}

#[test]
fn advance_y_puts_the_line_down_the_page() {
    // 360pt: 480px from the top of the page.
    let ops = render(Some("\\y 360"));
    let (_, ay) = drawn_at(&ops, "a");
    let (_, plain_y) = drawn_at(&render(None), "a");
    assert!(ay > plain_y + 100.0, "{ay} vs {plain_y}");
    let (_, by) = drawn_at(&ops, "b");
    assert_eq!(ay, by, "the whole line moves");
}
