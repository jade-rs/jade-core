//! Visible-window seek in documents with wrapping floats: snapshots carry the
//! live float state, so the resumed walk must paint exactly what a walk from
//! the document start paints.

use super::*;
use jade_painter::recording::{Op, RecordingPainter};

const PARAGRAPHS: usize = 300;

fn square_image_run(height: f64) -> Run {
    let mut run = band_plain_run();
    run.text = String::new();
    let mut image = RunImage {
        width_px: Some(150.0),
        height_px: Some(height),
        ..RunImage::default()
    };
    image.layout.kind = RunImageKind::Floating;
    image.layout.wrap = WrapMode::Square;
    image.layout.wrap_text = WrapTextSide::BothSides;
    image.layout.position_h = Some(ImagePosition {
        relative_from: Some("margin".to_string()),
        align: Some("center".to_string()),
        offset_px: None,
    });
    image.layout.position_v = Some(ImagePosition {
        relative_from: Some("paragraph".to_string()),
        offset_px: Some(0.0),
        align: None,
    });
    run.image = Some(image);
    run
}

/// `PARAGRAPHS` paragraphs of `words` words each, with a square-wrapped image
/// of `height` px anchored at the start of every paragraph in `anchors`.
fn float_document(words: usize, anchors: &[(usize, f64)]) -> Document {
    let text = "word ".repeat(words);
    let texts = vec![text.as_str(); PARAGRAPHS];
    let mut document = doc_with_paragraph_texts(&texts);
    for &(anchor, height) in anchors {
        let BodyElement::Paragraph(paragraph) =
            &mut document.body.as_mut().unwrap().elements[anchor]
        else {
            unreachable!()
        };
        paragraph.runs.insert(0, square_image_run(height));
    }
    document
}

fn painted(painter: &RecordingPainter) -> Vec<Op> {
    painter
        .ops()
        .into_iter()
        .filter(|op| {
            matches!(
                op,
                Op::FillText(..)
                    | Op::FillShapedText(..)
                    | Op::FillRect(..)
                    | Op::StrokeRect(..)
                    | Op::Rect(..)
                    | Op::MoveTo(..)
                    | Op::LineTo(..)
                    | Op::Arc(..)
            )
        })
        .collect()
}

struct SeekOutcome {
    walked: u64,
    window_hits: u64,
    span_hits: u64,
    /// Exclusions restored from the consumed snapshot (any kind).
    restored_live_float: bool,
    dirty: usize,
    resume: Option<usize>,
}

/// Lays the document out, edits `dirty`, repaints the visible window
/// `visible..=visible` (1-based) with the lazy-edit seek and compares every
/// visible page with an independent from-start render of the edited document.
fn edit_and_compare(
    mut document: Document,
    dirty: impl Fn(&CanvasRenderer<RecordingPainter>) -> usize,
) -> SeekOutcome {
    let options = options_for_seek_tests();
    let (host, pages) = PerPageRecordingHost::new(200);
    let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let dirty = dirty(&renderer);
    let visible = renderer.cached_paragraph_start_page(dirty).unwrap();
    assert!(visible > 2, "the edit must sit deep in the document");

    let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[dirty]
    else {
        unreachable!()
    };
    paragraph
        .runs
        .last_mut()
        .unwrap()
        .text
        .insert_str(0, "typed ");
    for page in &pages {
        page.clear();
    }
    renderer.set_visible_page_range(Some(visible..=visible));
    renderer.set_layout_page_limit(Some(visible + 1));
    renderer.prefer_visible_window_seek_once();
    renderer.set_incremental_dirty_from(dirty);
    renderer.invalidate_layout_from(dirty);
    renderer.invalidate_paragraph_wrap_cache_from(dirty);
    renderer.mark_paragraph_dirty(dirty);
    // The snapshot the render below will consume (same inputs, same choice).
    let chosen = renderer.try_prepare_visible_window_page_seek(&options);
    let before = renderer.window_seek_stats();
    let spans_before = renderer.span_seek_consumed.get();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let after = renderer.window_seek_stats();

    let (reference_host, reference_pages) = PerPageRecordingHost::new(200);
    let mut reference = CanvasRenderer::with_host(Box::new(reference_host), false).unwrap();
    reference.set_visible_page_range(Some(visible..=visible));
    reference.set_layout_page_limit(Some(visible + 1));
    reference
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let page = visible - 1;
    let ours = painted(&pages[page]);
    assert!(!ours.is_empty(), "page {visible} was painted");
    assert_eq!(ours, painted(&reference_pages[page]), "page {visible}");

    SeekOutcome {
        walked: renderer.last_render_body_element_count(),
        window_hits: (after.0 + after.1) - (before.0 + before.1),
        span_hits: renderer.span_seek_consumed.get() - spans_before,
        restored_live_float: chosen
            .as_ref()
            .is_some_and(|snapshot| !snapshot.floats.exclusions.is_empty()),
        dirty,
        resume: chosen.map(|snapshot| snapshot.paragraph_index),
    }
}

fn last_page_paragraph(_: &CanvasRenderer<RecordingPainter>) -> usize {
    PARAGRAPHS - 1
}

#[test]
fn square_float_before_the_window_keeps_the_visible_window_seek() {
    let outcome = edit_and_compare(float_document(12, &[(2, 60.0)]), last_page_paragraph);
    assert!(
        outcome.window_hits + outcome.span_hits > 0,
        "the window seek must hit"
    );
    assert!(
        outcome.walked < 60,
        "walked {} of {PARAGRAPHS} body elements",
        outcome.walked
    );
}

#[test]
fn square_float_on_the_visible_page_matches_a_from_start_render() {
    // A second float anchored on the last page wraps text in the window.
    let outcome = edit_and_compare(
        float_document(12, &[(2, 60.0), (PARAGRAPHS - 4, 80.0)]),
        last_page_paragraph,
    );
    assert!(outcome.window_hits + outcome.span_hits > 0);
    assert!(outcome.walked < 60, "walked {}", outcome.walked);
}

#[test]
fn snapshots_restore_live_exclusions_mid_page() {
    // Long paragraphs cross pages, so the window seek resumes at a spanning
    // paragraph start on an earlier page. Tall floats anchored on every
    // paragraph keep an exclusion live at those resume points.
    let anchors: Vec<_> = (0..PARAGRAPHS)
        .step_by(2)
        .map(|index| (index, 400.0))
        .collect();
    let outcome = edit_and_compare(float_document(90, &anchors), |_| PARAGRAPHS / 3);
    assert!(outcome.window_hits + outcome.span_hits > 0);
    assert!(
        outcome.restored_live_float,
        "a snapshot carried live exclusions"
    );
    assert!(outcome.walked < 20, "walked {}", outcome.walked);
}

#[test]
fn editing_a_float_anchor_at_the_page_top_never_resumes_at_it() {
    // Every paragraph anchors a float, so the edited paragraph's objects also
    // shaped the paragraph above it. A snapshot at the edit is untrusted.
    let anchors: Vec<_> = (0..PARAGRAPHS).map(|index| (index, 30.0)).collect();
    let outcome = edit_and_compare(float_document(12, &anchors), |renderer| {
        (0..PARAGRAPHS)
            .find(|&index| renderer.cached_paragraph_start_page(index) == Some(3))
            .unwrap()
    });
    assert!(
        outcome.resume.is_none_or(|resume| resume < outcome.dirty),
        "resumed at {:?} for an edit at {}",
        outcome.resume,
        outcome.dirty
    );
}

#[test]
fn dirty_resume_paragraph_is_rejected_in_float_documents() {
    let mut renderer = test_renderer();
    renderer.horizontal_float_wrap_cache.witness = Some(2);
    let snapshot = PageStartSeekSnapshot {
        paragraph_index: 10,
        ..page_top_snapshot(&renderer, 2)
    };
    renderer.forced_dirty_from = Some(10);
    assert!(!renderer.window_seek_float_state_is_current(&snapshot));
    renderer.forced_dirty_from = Some(11);
    assert!(renderer.window_seek_float_state_is_current(&snapshot));
    renderer.forced_dirty_from = None;
    assert!(renderer.window_seek_float_state_is_current(&snapshot));
    renderer.horizontal_float_wrap_cache.witness = None;
    renderer.forced_dirty_from = Some(0);
    assert!(renderer.window_seek_float_state_is_current(&snapshot));
}
