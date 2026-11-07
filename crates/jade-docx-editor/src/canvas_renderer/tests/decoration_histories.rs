//! Compare bounded pixel refreshes with a renderer that has no retained state.

use super::*;
use crate::plugins::{DecoratedRange, DecorationPlugin, DecorationStyle};
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

const REPEATED: &str = "repeat 日本語 😀";
const PAGES: usize = 6;

// Results are supplied asynchronously through DecorationCache, as in the host.
struct AsyncPlugin;

impl DecorationPlugin for AsyncPlugin {
    fn name(&self) -> &str {
        "async-test"
    }

    fn decorate(&self, _text: &str) -> Vec<DecoratedRange> {
        Vec::new()
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Decorate(u8, bool),
    Visit(usize, bool),
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        3 => (0u8..3, any::<bool>()).prop_map(|(style, forget_index)| Operation::Decorate(style, forget_index)),
        2 => (1usize..=PAGES, any::<bool>()).prop_map(|(page, pair)| Operation::Visit(page, pair)),
    ]
}

fn ranges(style: u8) -> Vec<DecoratedRange> {
    let style = match style {
        0 => return Vec::new(),
        1 => DecorationStyle::Highlight {
            color: "#ff0123".into(),
        },
        _ => DecorationStyle::Underline {
            color: "#0123ff".into(),
            thickness_px: None,
        },
    };
    vec![DecoratedRange {
        start: 0,
        end: REPEATED.len(),
        style,
    }]
}

#[derive(Clone, Debug, Default)]
struct PaintState {
    fill: String,
    stroke: String,
    font: String,
    line_width: f64,
}

// State-setting calls can be elided on replay. Compare their effect on ink,
// including decoration colors and text fonts, rather than call counts.
fn ink(painter: &RecordingPainter) -> Vec<(Op, String)> {
    let mut state = PaintState::default();
    let mut stack = Vec::new();
    let mut result = Vec::new();
    for op in painter.ops() {
        match &op {
            Op::Save => stack.push(state.clone()),
            Op::Restore => state = stack.pop().expect("balanced painter state"),
            Op::SetFillStyle(value) => state.fill = value.clone(),
            Op::SetStrokeStyle(value) => state.stroke = value.clone(),
            Op::SetFont(value) => state.font = value.clone(),
            Op::SetLineWidth(value) => state.line_width = *value,
            Op::FillText(..) | Op::FillShapedText(..) => {
                result.push((op, format!("{} {}", state.fill, state.font)));
            }
            Op::FillRect(..) => result.push((op, state.fill.clone())),
            Op::MoveTo(..) | Op::LineTo(..) | Op::Stroke => {
                result.push((op, format!("{} {}", state.stroke, state.line_width)));
            }
            _ => {}
        }
    }
    result
}

fn geometry(renderer: &CanvasRenderer<RecordingPainter>) -> Vec<(String, String)> {
    renderer
        .selection
        .paragraphs()
        .iter()
        .map(|p| (p.text.clone(), format!("{:?}", p.lines)))
        .collect()
}

fn check_sequence(operations: &[Operation], host_limit: Option<usize>) {
    let mut document = doc_with_paragraph_texts(&[REPEATED; PAGES * 3]);
    for (index, element) in document
        .body
        .as_mut()
        .unwrap()
        .elements
        .iter_mut()
        .enumerate()
    {
        let BodyElement::Paragraph(paragraph) = element else {
            unreachable!()
        };
        paragraph.properties = Some(ParagraphProperties {
            page_break_before: Some(index > 0 && index % 3 == 0),
            ..Default::default()
        });
    }
    let document = Rc::new(document);
    let mut options = options_for_seek_tests();
    options.show_crop_marks = false;
    let (host, pages) = PerPageRecordingHost::new(PAGES + 2);
    let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
    renderer.register_plugin(Box::new(AsyncPlugin));
    renderer.set_layout_page_limit(host_limit);
    renderer.set_visible_page_range(Some(2..=3));
    renderer
        .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
        .unwrap();
    assert!(renderer.last_layout_complete());
    let expected_geometry = geometry(&renderer);
    let selected = Some(SelectionRange::collapsed(TextPosition::new(4, 7)));
    renderer.selection.set_selection(selected);
    let mut window = 2..=3;
    let mut style = 0;

    for (step, operation) in operations.iter().enumerate() {
        let context = format!("step {step}: {operation:?}, window={window:?}, style={style}");
        renderer.clear_dirty_regions();
        for page in &pages {
            page.clear();
        }
        match operation {
            Operation::Decorate(next_style, forget_index) => {
                style = *next_style;
                if *forget_index {
                    renderer.decoration_cache.borrow_mut().clear();
                }
                renderer
                    .decoration_cache
                    .borrow_mut()
                    .put(REPEATED.into(), ranges(style));
                // Simulate evicted draw commands while geometry/page anchors
                // remain authoritative. Both indexed and unknown-text fallbacks
                // must refresh every visible occurrence of the repeated text.
                renderer.clear_paragraph_render_cache();
                renderer.prepare_decoration_repaint(&[REPEATED.into()]);
            }
            Operation::Visit(page, pair) => {
                window = *page..=if *pair { (*page + 1).min(PAGES) } else { *page };
                renderer.set_visible_page_range(Some(window.clone()));
                renderer.mark_full_repaint();
                renderer.freeze_selection_for_pixel_rerender();
                renderer.prefer_visible_window_seek_once();
            }
        }
        let previous_limit = renderer.layout_page_limit();
        renderer.set_layout_page_limit(renderer.decoration_repaint_page_limit());
        let result = renderer.rerender_with_cached_state();
        renderer.set_layout_page_limit(previous_limit);
        result.unwrap();
        assert_eq!(
            renderer.layout_page_limit(),
            host_limit,
            "{context}: host limit"
        );
        assert!(renderer.last_layout_complete(), "{context}: pagination");
        assert_eq!(renderer.selection_range(), selected, "{context}: selection");
        assert_eq!(
            geometry(&renderer),
            expected_geometry,
            "{context}: geometry"
        );

        // This renderer starts from page one with no draw/seek/decorations
        // history; its only state is the model's current style and window.
        let (reference_host, reference_pages) = PerPageRecordingHost::new(PAGES + 2);
        let mut reference = CanvasRenderer::with_host(Box::new(reference_host), false).unwrap();
        reference.register_plugin(Box::new(AsyncPlugin));
        reference
            .decoration_cache
            .borrow_mut()
            .put(REPEATED.into(), ranges(style));
        reference.set_visible_page_range(Some(window.clone()));
        reference
            .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
            .unwrap();
        for page in window.clone() {
            let actual = ink(&pages[page - 1]);
            let expected = ink(&reference_pages[page - 1]);
            assert!(!expected.is_empty(), "{context}: reference page {page}");
            assert_eq!(actual, expected, "{context}: page {page}");
            if style == 1 {
                assert!(
                    actual
                        .iter()
                        .any(|(op, color)| matches!(op, Op::FillRect(..)) && color == "#ff0123"),
                    "{context}: highlight must be visible"
                );
            } else if style == 2 {
                assert!(
                    actual.iter().any(|(op, stroke)| {
                        matches!(op, Op::Stroke) && stroke.starts_with("#0123ff ")
                    }),
                    "{context}: underline must be visible"
                );
            }
        }
        for page in 1..=PAGES {
            if !window.contains(&page) {
                assert!(
                    ink(&pages[page - 1]).is_empty(),
                    "{context}: offscreen page {page} was painted"
                );
            }
        }
    }
}

#[test]
fn decoration_changes_survive_cache_eviction_and_scroll_in_both_directions() {
    let operations = [
        Operation::Decorate(1, false),
        Operation::Visit(5, true),
        Operation::Decorate(2, true),
        Operation::Visit(1, false),
        Operation::Decorate(0, false),
        Operation::Visit(3, true),
        Operation::Decorate(1, true),
        Operation::Visit(6, true),
        Operation::Visit(1, true),
        Operation::Decorate(0, true),
    ];
    for host_limit in [None, Some(PAGES)] {
        check_sequence(&operations, host_limit);
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_shrink_iters: 512,
        ..ProptestConfig::default()
    })]

    #[test]
    fn decoration_and_scroll_histories_match_fresh_render(
        operations in prop::collection::vec(operation(), 1..=20),
        bounded_host in any::<bool>(),
    ) {
        check_sequence(&operations, bounded_host.then_some(PAGES));
    }
}
