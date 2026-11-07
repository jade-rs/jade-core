//! Direct formatting edits must match a fresh render, including
//! transitions between inherited, explicitly disabled and explicitly enabled.
use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

type FlagSetter = fn(&mut RunProperties, Option<bool>);
const FLAGS: [(&str, FlagSetter); 15] = [
    ("bold", |p, v| p.bold = v),
    ("italic", |p, v| p.italic = v),
    ("bold_complex", |p, v| p.bold_complex = v),
    ("italic_complex", |p, v| p.italic_complex = v),
    ("underline", |p, v| p.underline = v),
    ("strike", |p, v| p.strike = v),
    ("double_strike", |p, v| p.double_strike = v),
    ("caps", |p, v| p.caps = v),
    ("small_caps", |p, v| p.small_caps = v),
    ("vanish", |p, v| p.vanish = v),
    ("spec_vanish", |p, v| p.spec_vanish = v),
    ("shadow", |p, v| p.shadow = v),
    ("outline", |p, v| p.outline = v),
    ("emboss", |p, v| p.emboss = v),
    ("imprint", |p, v| p.imprint = v),
];
const TEXTS: [&str; 4] = ["Abc Z", "税あب e\u{301}", "A😀\0z", ""];

#[test]
fn render_hash_distinguishes_run_flag_states_and_field_pairs() {
    let renderer = test_renderer();
    let options = Options::default();
    for content in [None, Some(ContentHash::new(7))] {
        let mut paragraph = Paragraph {
            runs: vec![build_run(None)],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let mut keys = std::collections::HashSet::new();
        let mut insert = |props, description: String| {
            paragraph.runs[0].properties = props;
            let key = renderer.compute_paragraph_render_hash(&paragraph, &options, content);
            assert!(keys.insert(key), "{description}, content={content:?}");
        };
        insert(None, "absent properties".into());
        insert(Some(RunProperties::default()), "all inherited".into());
        for (index, (name, setter)) in FLAGS.into_iter().enumerate() {
            for value in [false, true] {
                let mut props = RunProperties::default();
                setter(&mut props, Some(value));
                insert(Some(props), format!("{name}={value}"));
                for (other_name, other_setter) in &FLAGS[index + 1..] {
                    for other_value in [false, true] {
                        let mut props = RunProperties::default();
                        setter(&mut props, Some(value));
                        other_setter(&mut props, Some(other_value));
                        insert(
                            Some(props),
                            format!("{name}={value}, {other_name}={other_value}"),
                        );
                    }
                }
            }
        }
        assert_eq!(keys.len(), 452);
    }
}

#[derive(Clone, Debug)]
enum Edit {
    Flag(usize, Option<bool>),
    ClearProperties,
    Text(usize),
    Repaint,
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        8 => (0..FLAGS.len(), prop::option::of(any::<bool>())).prop_map(|(i, v)| Edit::Flag(i, v)),
        1 => Just(Edit::ClearProperties),
        2 => (0..TEXTS.len()).prop_map(Edit::Text),
        2 => Just(Edit::Repaint),
    ]
}

fn renderer_with_painter() -> (CanvasRenderer<RecordingPainter>, RecordingPainter) {
    let painter = RecordingPainter::with_width_per_char(8.0);
    let renderer = CanvasRenderer::with_host(
        Box::new(NonCanvasHost {
            painter: painter.clone(),
        }),
        true,
    )
    .unwrap();
    (renderer, painter)
}

// Compare ink with its effective drawing state, since a retained command may
// omit redundant state setters. Keep state across paints of the warm canvas.
type PaintState = std::collections::BTreeMap<&'static str, String>;
fn ink(painter: &RecordingPainter, state: &mut PaintState) -> Vec<(Op, PaintState)> {
    let mut stack = Vec::new();
    let mut result = Vec::new();
    for op in painter.ops() {
        let setting = match &op {
            Op::Save => {
                stack.push(state.clone());
                continue;
            }
            Op::Restore => {
                *state = stack.pop().expect("balanced paint state");
                continue;
            }
            Op::SetFillStyle(v) => Some(("fill", v.clone())),
            Op::SetStrokeStyle(v) => Some(("stroke", v.clone())),
            Op::SetFont(v) => Some(("font", v.clone())),
            Op::SetTextBaseline(v) => Some(("baseline", v.clone())),
            Op::SetLineWidth(v) => Some(("width", format!("{v:?}"))),
            Op::SetGlobalAlpha(v) => Some(("alpha", format!("{v:?}"))),
            Op::SetShadowColor(v) => Some(("shadow color", v.clone())),
            Op::SetShadowBlur(v) => Some(("shadow blur", format!("{v:?}"))),
            Op::SetShadowOffsetX(v) => Some(("shadow x", format!("{v:?}"))),
            Op::SetShadowOffsetY(v) => Some(("shadow y", format!("{v:?}"))),
            Op::SetBlurFilter(v) => Some(("blur", format!("{v:?}"))),
            Op::MeasureText(_) | Op::ClearRect(..) => continue,
            _ => None,
        };
        if let Some((key, value)) = setting {
            state.insert(key, value);
        } else {
            // Font/fill setters have no effect on path construction or clipping,
            // and stroke settings have no effect on filled text (or vice versa).
            let keys: &[&str] = match &op {
                Op::FillText(..) | Op::FillShapedText(..) => &["fill", "font", "baseline"],
                Op::StrokeText(..) => &["stroke", "font", "baseline", "width"],
                Op::Fill | Op::FillRect(..) => &["fill"],
                Op::Stroke | Op::StrokeRect(..) => &["stroke", "width"],
                _ => &[],
            };
            let mut effective: PaintState = state
                .iter()
                .filter(|(key, _)| keys.contains(key))
                .map(|(key, value)| (*key, value.clone()))
                .collect();
            if !keys.is_empty() {
                for (key, default) in [
                    ("alpha", "1.0"),
                    ("shadow color", "rgba(0, 0, 0, 0)"),
                    ("shadow blur", "0.0"),
                    ("shadow x", "0.0"),
                    ("shadow y", "0.0"),
                    ("blur", "0.0"),
                ] {
                    effective.insert(key, state.get(key).map_or(default, String::as_str).into());
                }
            }
            result.push((op, effective));
        }
    }
    assert!(stack.is_empty());
    result
}

fn check_history(edits: &[Edit], windows: bool) {
    use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
    let _restore = RestoreWordPlatform::mac();
    if windows {
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }
    let mut document = doc_with_paragraph_texts(&[TEXTS[0], "UNCHANGED"]);
    let (mut warm, painter) = renderer_with_painter();
    let options = Options {
        show_formatting_marks: false,
        show_crop_marks: false,
        ..Default::default()
    };
    let mut state = PaintState::new();
    let mut flags = [None; FLAGS.len()];
    let mut has_properties = false;
    let mut text = 0;
    for (step, edit) in std::iter::once(&Edit::Repaint).chain(edits).enumerate() {
        let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[0]
        else {
            unreachable!()
        };
        match *edit {
            Edit::Flag(index, value) => {
                flags[index] = value;
                has_properties = true;
                (FLAGS[index].1)(paragraph.runs[0].properties.get_or_insert_default(), value);
            }
            Edit::ClearProperties => {
                flags.fill(None);
                has_properties = false;
                paragraph.runs[0].properties = None;
            }
            Edit::Text(index) => {
                text = index;
                paragraph.runs[0].text = TEXTS[index].into();
            }
            Edit::Repaint => {}
        }
        let mut expected = doc_with_paragraph_texts(&[TEXTS[text], "UNCHANGED"]);
        let BodyElement::Paragraph(paragraph) = &mut expected.body.as_mut().unwrap().elements[0]
        else {
            unreachable!()
        };
        if has_properties {
            let mut props = RunProperties::default();
            for ((_, setter), value) in FLAGS.into_iter().zip(flags) {
                setter(&mut props, value);
            }
            paragraph.runs[0].properties = Some(props);
        }
        if !matches!(edit, Edit::Repaint) {
            // Match CanvasView's inline-style edit path. Content-only change
            // detection deliberately does not detect run formatting changes.
            warm.invalidate_paragraph_render_cache_range(0, 0);
            warm.disable_seek_replay_once();
            warm.set_incremental_dirty_from(0);
            warm.mark_paragraph_dirty(0);
        }
        // Compare a complete frame while retaining caches and painter state.
        warm.mark_full_repaint();
        painter.clear();
        warm.render(&document, &options).unwrap();
        let (mut fresh, fresh_painter) = renderer_with_painter();
        fresh.render(&expected, &options).unwrap();
        let context =
            format!("step {step}: {edit:?}, flags={flags:?}, text={text}, windows={windows}");
        if step > 0 && matches!(edit, Edit::Repaint) {
            assert!(
                warm.paragraph_render_cache_stats().hits > 0,
                "{context}: expected cache replay"
            );
        }
        assert_eq!(
            serde_json::to_value(warm.layout_debug_lines()).unwrap(),
            serde_json::to_value(fresh.layout_debug_lines()).unwrap(),
            "{context}"
        );
        assert_eq!(
            ink(&painter, &mut state),
            ink(&fresh_painter, &mut PaintState::new()),
            "{context}"
        );
    }
}

#[test]
fn every_run_flag_survives_in_place_edits_and_restoration() {
    for windows in [false, true] {
        for text in 0..TEXTS.len() {
            for index in 0..FLAGS.len() {
                check_history(
                    &[
                        Edit::Text(text),
                        Edit::Flag(index, Some(true)),
                        Edit::Repaint,
                        Edit::Flag(index, Some(false)),
                        Edit::Flag(index, None),
                        Edit::Flag(index, Some(true)),
                        Edit::ClearProperties,
                        Edit::Repaint,
                    ],
                    windows,
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn run_flag_edits_match_fresh_render_after_every_step(
        edits in prop::collection::vec(edit(), 1..25), windows in any::<bool>()
    ) {
        check_history(&edits, windows);
    }
}
