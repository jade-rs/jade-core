//! A paragraph replayed from the render cache sets its own fonts on the
//! surface. The live drawing after it must not believe the font it set
//! before the replay is still the surface's: a plain paragraph drawn after a
//! cached italic one came out in italics until the next full repaint.

use super::recording_host::RecordingHost;
use super::*;
use jade_painter::recording::{Op, RecordingPainter};

fn run(text: &str, italic: bool) -> Run {
    Run {
        text: text.into(),
        properties: Some(RunProperties {
            italic: italic.then_some(true),
            ..RunProperties::default()
        }),
        image: None,
        text_box: None,
        math: None,
        field: None,
        hyperlink: None,
        node_id: None,
        text_node_id: None,
        revision_node_id: None,
        comment_reference_ids: Vec::new(),
        footnote_reference_ids: Vec::new(),
        endnote_reference_ids: Vec::new(),
        note_separator: None,
        has_footnote_ref: false,
        has_endnote_ref: false,
        revision: None,
    }
}

fn paragraph(text: &str, italic: bool) -> BodyElement {
    BodyElement::from_paragraph(Paragraph {
        runs: vec![run(text, italic)],
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    })
}

fn document(first: &str, last: &str) -> Document {
    Document {
        body: Some(Body {
            elements: vec![
                paragraph(first, false),
                paragraph("slanted", true),
                paragraph(last, false),
            ],
        }),
        ..Document::default()
    }
}

/// The font the surface has when `text`'s first glyph is drawn.
fn font_at(ops: &[Op], text: &str) -> String {
    let at = ops
        .iter()
        .position(|op| matches!(op, Op::FillText(drawn, _, _) if drawn == text))
        .unwrap_or_else(|| panic!("{text} drawn"));
    ops[..at]
        .iter()
        .rev()
        .find_map(|op| match op {
            Op::SetFont(font) => Some(font.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn a_live_paragraph_after_a_cached_italic_one_is_not_drawn_in_italics() {
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    let options = Options {
        show_formatting_marks: false,
        ..Options::default()
    };
    renderer
        .render(&document("plain", "after"), &options)
        .unwrap();
    assert!(!font_at(&painter.ops(), "a").contains("italic"));
    // The paragraphs around the italic one change; it is replayed from the
    // cache between them.
    painter.clear();
    renderer
        .render(&document("plainer", "later"), &options)
        .unwrap();
    assert!(renderer.paragraph_render_cache_stats().hits > 0, "a replay");
    let ops = painter.ops();
    assert!(
        font_at(&ops, "s").contains("italic"),
        "{}",
        font_at(&ops, "s")
    );
    assert!(
        !font_at(&ops, "l").contains("italic"),
        "the paragraph after the replay: {}",
        font_at(&ops, "l")
    );
}

/// Each glyph drawn and the font the surface has for it, as a canvas keeps
/// its font: set by `SetFont`, saved and restored with the rest of its state.
fn glyph_fonts(ops: &[Op]) -> Vec<((String, i64, i64), String)> {
    let mut font = String::new();
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::SetFont(value) => font = value.clone(),
            Op::Save => stack.push(font.clone()),
            Op::Restore => font = stack.pop().unwrap_or_default(),
            Op::FillText(text, x, y) => out.push((
                (
                    text.clone(),
                    (x * 100.0).round() as i64,
                    (y * 100.0).round() as i64,
                ),
                font.clone(),
            )),
            _ => {}
        }
    }
    out
}

/// Many paragraphs, every third italic, with `pasted` in at `at` — after
/// a picture when `picture`.
fn long_document(pasted: Option<(usize, &str, bool)>, picture: bool) -> Document {
    let mut elements: Vec<BodyElement> = (0..30)
        .map(|index| paragraph(&format!("para{index} text"), index % 3 == 1))
        .collect();
    if let Some((at, text, italic)) = pasted {
        let mut element = paragraph(text, italic);
        if picture {
            if let BodyElement::Paragraph(paragraph) = &mut element {
                let mut image = run("", false);
                image.image = Some(RunImage {
                    data_uri: Some("data:image/png;base64,iVBORw0KGgo=".into()),
                    width_px: Some(40.0),
                    height_px: Some(30.0),
                    ..RunImage::default()
                });
                paragraph.runs.insert(0, image);
            }
        }
        elements.insert(at, element);
    }
    Document {
        body: Some(Body { elements }),
        ..Document::default()
    }
}

/// A paste of an italic paragraph, repainted the way the browser build's
/// editor repaints an edit — the edited page's band, then on down from the
/// paragraph after it — draws every glyph in the font a full render does.
#[test]
fn a_band_repaint_after_a_paste_draws_every_paragraph_in_its_own_font() {
    for (at, italic, picture) in [
        (4usize, true, false),
        (5, false, false),
        (7, true, false),
        (4, true, true),
        (5, false, true),
    ] {
        let painter = RecordingPainter::new();
        let mut renderer = CanvasRenderer::with_host(
            Box::new(RecordingHost {
                painter: painter.clone(),
            }),
            false,
        )
        .expect("recording renderer");
        let options = Options {
            show_formatting_marks: false,
            ..Options::default()
        };
        renderer
            .render(&long_document(None, false), &options)
            .unwrap();
        painter.clear();
        let edited = Rc::new(long_document(Some((at, "pasted words", italic)), picture));
        renderer.set_band_redraw_enabled(true);
        renderer.prepare_incremental_repaint(at, 1);
        renderer
            .render_bounded_to_page_with_parts(Rc::clone(&edited), &options, None, None, None, 1)
            .unwrap();
        renderer.set_band_redraw_enabled(false);
        renderer.prepare_incremental_repaint(at + 1, usize::MAX);
        renderer
            .render_bounded_to_page_with_parts(
                Rc::clone(&edited),
                &options,
                None,
                None,
                None,
                usize::MAX,
            )
            .unwrap();
        let incremental = glyph_fonts(&painter.ops());

        let fresh_painter = RecordingPainter::new();
        let mut fresh = CanvasRenderer::with_host(
            Box::new(RecordingHost {
                painter: fresh_painter.clone(),
            }),
            false,
        )
        .expect("recording renderer");
        fresh.render(&edited, &options).unwrap();
        let full = glyph_fonts(&fresh_painter.ops());
        // Every glyph the repaint drew is in the font the full render drew
        // the glyph at that place in.
        let full: std::collections::HashMap<_, _> = full.into_iter().collect();
        assert!(!incremental.is_empty());
        for (glyph, font) in &incremental {
            assert_eq!(
                full.get(glyph),
                Some(font),
                "{glyph:?} repainted in {font:?} (paste at {at})"
            );
        }
    }
}
