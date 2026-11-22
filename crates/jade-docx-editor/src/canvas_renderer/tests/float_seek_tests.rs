//! Visible-window seek in documents with wrapping floats: snapshots carry the
//! live float state, so a resumed walk must paint exactly what a walk from
//! the document start paints. Paragraph text is unique and aperiodic, so a
//! whole-paragraph shift changes the painted page.

use super::*;
use jade_painter::recording::{Op, RecordingPainter};

const PARAGRAPHS: usize = 300;
/// Unset (no look-ahead), and the two modes that run `float_lookahead_for`.
const MODES: [Option<u32>; 3] = [None, Some(14), Some(15)];

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

/// `PARAGRAPHS` unique paragraphs of about `words` words, with a
/// square-wrapped image of the given height anchored in each listed paragraph.
fn float_document(words: usize, anchors: &[(usize, f64)], mode: Option<u32>) -> Document {
    let owned: Vec<String> = (0..PARAGRAPHS)
        .map(|index| format!("p{index} {}", "word ".repeat(words + (index * 7) % 11)))
        .collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut document = doc_with_paragraph_texts(&texts);
    document.east_asian_line_breaking.compatibility_mode = mode;
    for &(anchor, height) in anchors {
        paragraph_mut(&mut document, anchor)
            .runs
            .insert(0, square_image_run(height));
    }
    document
}

fn paragraph_mut(document: &mut Document, index: usize) -> &mut Paragraph {
    let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[index]
    else {
        unreachable!()
    };
    paragraph
}

fn image_mut(paragraph: &mut Paragraph) -> &mut RunImage {
    paragraph.runs[0].image.as_mut().unwrap()
}

fn type_text(paragraph: &mut Paragraph) {
    paragraph
        .runs
        .last_mut()
        .unwrap()
        .text
        .insert_str(0, "typed ");
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

fn renderer_with_pages() -> (CanvasRenderer<RecordingPainter>, Vec<RecordingPainter>) {
    let (host, pages) = PerPageRecordingHost::new(400);
    (
        CanvasRenderer::with_host(Box::new(host), false).unwrap(),
        pages,
    )
}

/// Page `page` (1-based) of an independent from-start render.
fn reference(document: &Document, page: usize) -> Vec<Op> {
    let (mut renderer, pages) = renderer_with_pages();
    renderer.set_visible_page_range(Some(page..=page));
    renderer.set_layout_page_limit(Some(page + 1));
    renderer
        .render_with_numbering(document, &options_for_seek_tests(), None, None, None, None)
        .unwrap();
    painted(&pages[page - 1])
}

fn assert_page_matches(
    pages: &[RecordingPainter],
    document: &Document,
    page: usize,
    context: &str,
) {
    let ours = painted(&pages[page - 1]);
    assert!(!ours.is_empty(), "{context}: page {page} was painted");
    assert!(
        ours == reference(document, page),
        "{context}: page {page} differs from a from-start render"
    );
}

fn starts_on(renderer: &CanvasRenderer<RecordingPainter>, page: usize) -> Vec<usize> {
    (0..PARAGRAPHS)
        .filter(|&index| renderer.cached_paragraph_start_page(index) == Some(page))
        .collect()
}

#[derive(Debug)]
struct SeekOutcome {
    walked: u64,
    hits: u64,
    dirty: usize,
    resume: Option<usize>,
    restored_live_float: bool,
}

/// Lays the document out, applies `mutate` to the paragraph `pick` chooses,
/// repaints its page with the lazy-edit seek and then scrolls one page on.
/// Both pages are compared with independent from-start renders.
fn edit_and_compare(
    mut document: Document,
    pick: impl Fn(&CanvasRenderer<RecordingPainter>) -> usize,
    mutate: impl Fn(&mut Paragraph),
    context: &str,
) -> SeekOutcome {
    let options = options_for_seek_tests();
    let (mut renderer, pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let dirty = pick(&renderer);
    let visible = renderer.cached_paragraph_start_page(dirty).unwrap();
    let last_page = renderer
        .cached_paragraph_start_page(PARAGRAPHS - 1)
        .unwrap();
    assert!(
        visible > 2,
        "{context}: the edit must sit deep in the document"
    );
    mutate(paragraph_mut(&mut document, dirty));
    let mut outcome = None;
    for page in (visible..=visible + 1).filter(|&page| page <= last_page) {
        for painter in &pages {
            painter.clear();
        }
        renderer.set_visible_page_range(Some(page..=page));
        renderer.set_layout_page_limit(Some(page + 1));
        renderer.prefer_visible_window_seek_once();
        if page == visible {
            renderer.set_incremental_dirty_from(dirty);
            renderer.invalidate_layout_from(dirty);
            renderer.invalidate_paragraph_wrap_cache_from(dirty);
            renderer.mark_paragraph_dirty(dirty);
        } else {
            renderer.skip_change_detection_once();
        }
        // The snapshot the render below consumes (same inputs, same choice).
        let chosen = renderer.try_prepare_visible_window_page_seek(&options);
        let before = renderer.window_seek_stats();
        let spans_before = renderer.span_seek_consumed.get();
        renderer
            .render_with_numbering(&document, &options, None, None, None, None)
            .unwrap();
        let after = renderer.window_seek_stats();
        assert_page_matches(&pages, &document, page, context);
        if page == visible {
            outcome = Some(SeekOutcome {
                walked: renderer.last_render_body_element_count(),
                hits: (after.0 + after.1) - (before.0 + before.1)
                    + (renderer.span_seek_consumed.get() - spans_before),
                dirty,
                resume: chosen.as_ref().map(|snapshot| snapshot.paragraph_index),
                restored_live_float: chosen
                    .as_ref()
                    .is_some_and(|snapshot| !snapshot.floats.exclusions.is_empty()),
            });
        }
    }
    outcome.unwrap()
}

fn on_page(page: usize, which: usize) -> impl Fn(&CanvasRenderer<RecordingPainter>) -> usize {
    move |renderer| {
        let starts = starts_on(renderer, page);
        starts[which * (starts.len() - 1) / 2]
    }
}

#[test]
fn square_float_before_the_window_keeps_the_visible_window_seek() {
    for mode in MODES {
        let outcome = edit_and_compare(
            float_document(12, &[(2, 60.0)], mode),
            |_| PARAGRAPHS - 1,
            type_text,
            &format!("mode={mode:?}"),
        );
        assert!(outcome.hits > 0, "mode={mode:?}: {outcome:?}");
        assert!(outcome.walked < 60, "mode={mode:?}: {outcome:?}");
    }
}

#[test]
fn square_float_on_the_visible_page_matches_a_from_start_render() {
    for mode in MODES {
        let outcome = edit_and_compare(
            float_document(12, &[(2, 60.0), (PARAGRAPHS - 4, 80.0)], mode),
            |_| PARAGRAPHS - 1,
            type_text,
            &format!("mode={mode:?}"),
        );
        assert!(outcome.hits > 0, "mode={mode:?}: {outcome:?}");
        assert!(outcome.walked < 60, "mode={mode:?}: {outcome:?}");
    }
}

#[test]
fn snapshots_restore_live_exclusions_mid_page() {
    // Long paragraphs cross pages, so the window seek resumes at a spanning
    // paragraph start on an earlier page. Tall floats on every other
    // paragraph keep an exclusion live at those resume points.
    for mode in MODES {
        let anchors: Vec<_> = (0..PARAGRAPHS).step_by(2).map(|i| (i, 400.0)).collect();
        let outcome = edit_and_compare(
            float_document(90, &anchors, mode),
            |_| PARAGRAPHS / 3,
            type_text,
            &format!("mode={mode:?}"),
        );
        assert!(outcome.hits > 0, "mode={mode:?}: {outcome:?}");
        assert!(outcome.restored_live_float, "mode={mode:?}: {outcome:?}");
        assert!(outcome.walked < 20, "mode={mode:?}: {outcome:?}");
    }
}

type ParagraphEdit = (&'static str, fn(&mut Paragraph));

#[test]
fn float_geometry_edits_at_the_dirty_paragraph_match_a_from_start_render() {
    // Every paragraph anchors a float, so the paragraph above the edit also
    // wraps around the edited float (look-ahead in modes 14 and 15).
    let edits: [ParagraphEdit; 6] = [
        ("type", type_text),
        ("grow", |p| image_mut(p).height_px = Some(160.0)),
        ("shrink", |p| image_mut(p).height_px = Some(5.0)),
        ("tight", |p| image_mut(p).layout.wrap = WrapMode::Tight),
        ("top-and-bottom", |p| {
            image_mut(p).layout.wrap = WrapMode::TopAndBottom
        }),
        ("move", |p| {
            image_mut(p).layout.position_v.as_mut().unwrap().offset_px = Some(70.0)
        }),
    ];
    for mode in MODES {
        let anchors: Vec<_> = (0..PARAGRAPHS).map(|i| (i, 30.0)).collect();
        for (name, edit) in edits {
            for which in [0, 2] {
                let outcome = edit_and_compare(
                    float_document(12, &anchors, mode),
                    on_page(4, which),
                    edit,
                    &format!("mode={mode:?} {name} which={which}"),
                );
                assert!(
                    outcome.resume.is_none_or(|resume| resume < outcome.dirty),
                    "mode={mode:?} {name}: {outcome:?}"
                );
            }
        }
    }
}

#[test]
fn removing_the_only_float_still_rejects_a_snapshot_at_the_edit() {
    for mode in MODES {
        // The first paragraph of page 4, found on a float-free layout.
        let (mut probe, _) = renderer_with_pages();
        probe
            .render_with_numbering(
                &float_document(12, &[], mode),
                &options_for_seek_tests(),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let target = starts_on(&probe, 4)[0];
        let outcome = edit_and_compare(
            float_document(12, &[(target, 200.0)], mode),
            |_| target,
            |paragraph| {
                paragraph.runs.remove(0);
            },
            &format!("mode={mode:?}"),
        );
        assert!(
            outcome.resume.is_none_or(|resume| resume < outcome.dirty),
            "mode={mode:?}: {outcome:?}"
        );
    }
}

type DocumentEdit = (&'static str, fn(&mut Document) -> [usize; 2]);

/// Resize, wrap change and re-anchor of the float in paragraph 2; each
/// returns the old and new anchor paragraphs.
const OBJECT_EDITS: [DocumentEdit; 3] = [
    ("resize", |document| {
        image_mut(paragraph_mut(document, 2)).height_px = Some(400.0);
        [2, 2]
    }),
    ("wrap", |document| {
        image_mut(paragraph_mut(document, 2)).layout.wrap = WrapMode::TopAndBottom;
        [2, 2]
    }),
    ("reanchor", |document| {
        let run = paragraph_mut(document, 2).runs.remove(0);
        let paragraph = paragraph_mut(document, 5);
        paragraph.runs.insert(0, run);
        image_mut(paragraph).height_px = Some(300.0);
        [2, 5]
    }),
];

/// The editor's object command (`commit_selected_object_command_async`):
/// full repaint of the first pages, bounded layout, previous limit back.
fn object_edit(
    mode: Option<u32>,
    edit: fn(&mut Document) -> [usize; 2],
) -> (
    CanvasRenderer<RecordingPainter>,
    Vec<RecordingPainter>,
    Document,
) {
    let options = options_for_seek_tests();
    let mut document = float_document(12, &[(2, 60.0)], mode);
    let (mut renderer, pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let previous = renderer.layout_page_limit();
    let anchors = edit(&mut document);
    renderer.mark_full_repaint();
    renderer.invalidate_layout_for_object_edit(&anchors);
    renderer.set_visible_page_range(Some(1..=3));
    renderer.set_layout_page_limit(Some(3));
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.set_layout_page_limit(previous);
    renderer.clear_dirty_regions();
    for painter in &pages {
        painter.clear();
    }
    (renderer, pages, document)
}

#[test]
fn object_edits_then_window_seek_scroll_match_a_from_start_render() {
    let options = options_for_seek_tests();
    for mode in MODES {
        for (name, edit) in OBJECT_EDITS {
            let (mut renderer, pages, document) = object_edit(mode, edit);
            renderer.set_visible_page_range(Some(5..=5));
            renderer.set_layout_page_limit(Some(6));
            renderer.skip_change_detection_once();
            renderer.prefer_visible_window_seek_once();
            renderer
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            assert_page_matches(&pages, &document, 5, &format!("scroll {name} {mode:?}"));
        }
    }
}

#[test]
fn object_edits_then_page_bitmap_rerender_match_a_from_start_render() {
    let options = options_for_seek_tests();
    for mode in MODES {
        for (name, edit) in OBJECT_EDITS {
            let (mut renderer, pages, document) = object_edit(mode, edit);
            renderer.set_visible_page_range(Some(5..=5));
            renderer.set_layout_page_limit(Some(5));
            renderer.prepare_rerender_for_page(4);
            renderer
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            assert_page_matches(&pages, &document, 5, &format!("bitmap {name} {mode:?}"));
        }
    }
}

#[test]
fn object_edits_then_text_edit_on_page_five_match_a_from_start_render() {
    let options = options_for_seek_tests();
    for mode in MODES {
        for (name, edit) in OBJECT_EDITS {
            let (mut renderer, pages, mut document) = object_edit(mode, edit);
            // Page five as a from-start layout of the edited document has it.
            let (mut fresh, _) = renderer_with_pages();
            fresh
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            let dirty = on_page(5, 1)(&fresh);
            type_text(paragraph_mut(&mut document, dirty));
            renderer.set_visible_page_range(Some(5..=5));
            renderer.set_layout_page_limit(Some(6));
            renderer.prefer_visible_window_seek_once();
            renderer.set_incremental_dirty_from(dirty);
            renderer.invalidate_layout_from(dirty);
            renderer.invalidate_paragraph_wrap_cache_from(dirty);
            renderer.mark_paragraph_dirty(dirty);
            renderer
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            assert_page_matches(&pages, &document, 5, &format!("type {name} {mode:?}"));
        }
    }
}

#[test]
fn bitmap_rerender_rejects_a_page_top_snapshot_the_layout_cache_disowns() {
    let options = options_for_seek_tests();
    let mut document = float_document(12, &[(2, 60.0)], None);
    paragraph_mut(&mut document, 150).properties = Some(ParagraphProperties {
        page_break_before: Some(true),
        ..Default::default()
    });
    let (mut renderer, _) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let page = *renderer
        .page_start_seek_cache_by_page
        .borrow()
        .iter()
        .find(|(&page, snapshot)| page > 1 && !snapshot.spanning)
        .expect("a page-top snapshot")
        .0;
    renderer.prepare_rerender_for_page(page - 1);
    assert!(
        renderer.forced_seek_snapshot.is_some(),
        "a valid snapshot seeks"
    );
    // As after the rerender that preparation was for.
    renderer.skip_layout_cache_update = false;
    let mut stale = renderer.page_start_seek_cache_by_page.borrow()[&page].clone();
    stale.page_offset_y += 40.0;
    renderer
        .page_start_seek_cache_by_page
        .borrow_mut()
        .insert(page, stale);
    renderer.prepare_rerender_for_page(page - 1);
    assert!(renderer.forced_seek_snapshot.is_none());
}

#[test]
fn snapshots_store_only_current_and_later_floating_table_frames() {
    use jade_docx_reader::document::{TableRow, TableRowHeightRule};
    let tables = 400;
    let owned: Vec<String> = (0..tables)
        .map(|i| format!("p{i} {}", "word ".repeat(30 + (i * 7) % 11)))
        .collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let base = doc_with_paragraph_texts(&texts);
    let mut elements = Vec::new();
    for element in &base.body.as_ref().unwrap().elements {
        let table = Table {
            rows: vec![TableRow {
                cells: vec![single_text_cell("x")],
                height: Some(40.0),
                height_rule: Some(TableRowHeightRule::Exact),
                ..Default::default()
            }],
            style: [
                ("table-position", "floating"),
                ("table-position-vertical-anchor", "text"),
                ("table-position-y", "0px"),
                ("table-overlap", "never"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
            ..Default::default()
        };
        elements.push(BodyElement::Table(Box::new(table)));
        elements.push(element.clone());
    }
    let document = Document {
        body: Some(Body { elements }),
        ..base
    };
    let (mut renderer, _) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options_for_seek_tests(), None, None, None, None)
        .unwrap();
    let frames = &renderer.floating_table_frames;
    assert!(frames.len() >= tables);
    let max_per_page = frames
        .iter()
        .map(|frame| {
            frames
                .iter()
                .filter(|other| other.page_number == frame.page_number)
                .count()
        })
        .max()
        .unwrap();
    let cache = renderer.paragraph_seek_cache.borrow();
    assert!(cache.len() >= tables);
    let stored: usize = cache
        .values()
        .map(|snapshot| snapshot.floats.floating_table_frames.len())
        .sum();
    // Each snapshot holds at most its own page's frames: linear in the
    // document, where cloning every earlier frame was quadratic.
    assert!(
        stored <= cache.len() * max_per_page,
        "stored {stored} frames for {} snapshots ({max_per_page} per page)",
        cache.len()
    );
    assert!(stored < tables * tables / 20, "stored {stored} frames");
    assert!(cache.values().all(|snapshot| snapshot
        .floats
        .floating_table_frames
        .iter()
        .all(|frame| frame.page_number >= snapshot.page_number)));
}

#[test]
fn float_documents_only_trust_snapshots_before_the_dirty_paragraph() {
    let mut renderer = test_renderer();
    let snapshot = PageStartSeekSnapshot {
        paragraph_index: 10,
        ..page_top_snapshot(&renderer, 2)
    };
    for guard in [true, false] {
        renderer.horizontal_float_wrap_cache.seek_guard = guard;
        renderer.forced_dirty_from = Some(10);
        assert_eq!(
            renderer.window_seek_float_state_is_current(&snapshot),
            !guard
        );
        renderer.forced_dirty_from = Some(11);
        assert!(renderer.window_seek_float_state_is_current(&snapshot));
        renderer.forced_dirty_from = None;
        assert!(renderer.window_seek_float_state_is_current(&snapshot));
    }
}

#[test]
fn the_seek_guard_covers_the_render_that_removes_the_last_float() {
    let options = options_for_seek_tests();
    let mut document = float_document(4, &[(2, 60.0)], None);
    let (mut renderer, _) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    assert!(renderer.horizontal_float_wrap_cache.seek_guard);
    renderer.drop_document_cache();
    paragraph_mut(&mut document, 2).runs.remove(0);
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    assert!(
        renderer.horizontal_float_wrap_cache.seek_guard,
        "the old document wrapped"
    );
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    assert!(!renderer.horizontal_float_wrap_cache.seek_guard);
}

/// A header edit that grows the header from one line to `lines`, repainted
/// like `prepare_hf_edit_rerender` does: full repaint of a bounded window.
fn header_edit(
    anchors: &[(usize, f64)],
    lines: usize,
) -> (
    CanvasRenderer<RecordingPainter>,
    Vec<RecordingPainter>,
    Document,
    Body,
) {
    let options = options_for_seek_tests();
    let document = float_document(12, anchors, None);
    let short = doc_with_paragraph_texts(&["Header"]).body.unwrap();
    let texts = vec!["Tall header"; lines];
    let tall = doc_with_paragraph_texts(&texts).body.unwrap();
    let (mut renderer, pages) = renderer_with_pages();
    renderer.set_header_footer_bodies(Some(Rc::new(short)), None, None, None, None, None);
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let previous = renderer.layout_page_limit();
    renderer.set_header_footer_bodies(Some(Rc::new(tall.clone())), None, None, None, None, None);
    renderer.begin_header_footer_edit();
    renderer.set_visible_page_range(Some(1..=3));
    renderer.set_layout_page_limit(Some(3));
    renderer.mark_full_repaint();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.set_layout_page_limit(previous);
    renderer.clear_dirty_regions();
    for painter in &pages {
        painter.clear();
    }
    (renderer, pages, document, tall)
}

#[test]
fn header_height_edits_then_seek_match_a_from_start_render() {
    let options = options_for_seek_tests();
    for anchors in [vec![(2, 60.0)], vec![]] {
        for path in ["window", "bitmap"] {
            let (mut renderer, pages, document, tall) = header_edit(&anchors, 8);
            renderer.set_visible_page_range(Some(5..=5));
            if path == "window" {
                renderer.set_layout_page_limit(Some(6));
                renderer.skip_change_detection_once();
                renderer.prefer_visible_window_seek_once();
            } else {
                renderer.set_layout_page_limit(Some(5));
                renderer.prepare_rerender_for_page(4);
            }
            renderer
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            let (mut reference, reference_pages) = renderer_with_pages();
            reference.set_header_footer_bodies(Some(Rc::new(tall)), None, None, None, None, None);
            reference.set_visible_page_range(Some(5..=5));
            reference.set_layout_page_limit(Some(6));
            reference
                .render_with_numbering(&document, &options, None, None, None, None)
                .unwrap();
            let ours = painted(&pages[4]);
            assert!(!ours.is_empty(), "{path} floats={}", anchors.len());
            assert!(
                ours == painted(&reference_pages[4]),
                "{path} floats={}: page 5 differs from a from-start render",
                anchors.len()
            );
        }
    }
}

#[test]
fn header_edits_that_keep_the_height_keep_the_layout_cache() {
    let (renderer, _, _, _) = header_edit(&[(2, 60.0)], 1);
    assert!(renderer
        .cached_paragraph_start_page(PARAGRAPHS - 1)
        .is_some());
    let (renderer, _, _, _) = header_edit(&[(2, 60.0)], 8);
    assert!(renderer
        .cached_paragraph_start_page(PARAGRAPHS - 1)
        .is_none());
}
