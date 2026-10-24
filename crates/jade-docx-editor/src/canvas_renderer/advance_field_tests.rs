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

/// hello.docx's first paragraph as `a`, `ADVANCE <switches>` when given,
/// then `after`, with `indent_pt` of left indent.
fn render_paragraph(switches: Option<&str>, after: &str, indent_pt: f32) -> Vec<Op> {
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
        let properties = paragraph.properties.get_or_insert_with(Default::default);
        properties.indent = Some(jade_docx_reader::document::Indent {
            left: Some(indent_pt),
            ..Default::default()
        });
        let mut a = paragraph.runs[0].clone();
        a.text = "a".into();
        a.field = None;
        let mut rest = a.clone();
        rest.text = after.into();
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
        runs.push(rest);
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

#[test]
fn advance_x_is_measured_from_the_column_edge_not_the_indent() {
    let (column_a, _) = drawn_at(&render_paragraph(None, "b", 0.0), "a");
    // 36pt of indent: `a` starts 48px in, but \x 72 still puts `b` 96px from
    // the column's edge.
    let indented = render_paragraph(Some("\\x 72"), "b", 36.0);
    let (ax, _) = drawn_at(&indented, "a");
    assert!((ax - (column_a + 48.0)).abs() < 0.01, "{ax} vs {column_a}");
    let (bx, _) = drawn_at(&indented, "b");
    assert!((bx - (column_a + 96.0)).abs() < 0.01, "{bx} vs {column_a}");
}

#[test]
fn advance_up_and_down_move_the_rest_of_their_line_only() {
    // Enough words after the field to fill a second line, which ends in `z`.
    let words = format!(" {}z", "word ".repeat(30));
    let plain = render_paragraph(None, &words, 0.0);
    let lowered = render_paragraph(Some("\\d 6"), &words, 0.0);
    let (_, plain_w) = drawn_at(&plain, "w");
    let (_, lowered_w) = drawn_at(&lowered, "w");
    assert!(
        (lowered_w - (plain_w + 8.0)).abs() < 0.01,
        "the field's line drops"
    );
    // The next line is laid out at its own baseline, where the plain
    // paragraph's second line is.
    let (_, plain_z) = drawn_at(&plain, "z");
    let (_, lowered_z) = drawn_at(&lowered, "z");
    assert!(plain_z > plain_w + 1.0, "z is on a later line");
    assert!(
        (lowered_z - plain_z).abs() < 0.01,
        "the next line is not lowered: {lowered_z} vs {plain_z}"
    );
}
