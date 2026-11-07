//! Direct formatting edits must match a fresh render, including inherited
//! flags, explicit flags, numeric metrics, and restoration of defaults.
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

type MetricSetter = fn(&mut RunProperties, Option<f32>);
const METRICS: [(&str, MetricSetter, [f32; 5]); 5] = [
    (
        "text_position",
        |p, v| p.text_position = v,
        [-3.0, -1.0, 0.0, 1.0, 3.0],
    ),
    (
        "char_spacing",
        |p, v| p.char_spacing = v,
        [-1.0, -0.0, 0.0, 0.5, 1.25],
    ),
    ("kerning", |p, v| p.kerning = v, [0.0, 4.0, 8.0, 12.0, 24.0]),
    (
        "text_scale",
        |p, v| p.text_scale = v,
        [50.0, 75.0, 100.0, 125.0, 150.0],
    ),
    (
        "font_size",
        |p, v| p.font_size = v,
        [8.0, 10.0, 12.0, 16.0, 24.0],
    ),
];

// Raw optional strings are modeled separately from the renderer's cache key.
// The shading field uses a clear fill, whose resolved background is the fill.
type TextSetter = fn(&mut RunProperties, Option<&str>);
const RUN_TEXT: [(&str, TextSetter, [&str; 5]); 12] = [
    (
        "underline_kind",
        |p, v| p.underline_kind = v.map(Into::into),
        ["single", "double", "wave", "", "none"],
    ),
    (
        "underline_color",
        |p, v| p.underline_color = v.map(Into::into),
        ["#102030", "#8090A0", "auto", "", "#000000"],
    ),
    (
        "emphasis",
        |p, v| p.emphasis = v.map(Into::into),
        ["none", "dot", "circle", "comma", "underdot"],
    ),
    (
        "baseline",
        |p, v| p.baseline = v.map(Into::into),
        ["baseline", "superscript", "subscript", "", "baseline"],
    ),
    (
        "font_family",
        |p, v| p.font_family = v.map(Into::into),
        ["Arial", "Courier New", "Noto Sans", "游明朝", ""],
    ),
    (
        "ascii",
        |p, v| {
            if v.is_some() || p.fonts.is_some() {
                p.fonts.get_or_insert_default().ascii = v.map(Into::into);
            }
        },
        ["Arial", "Courier New", "Noto Sans", "游明朝", ""],
    ),
    (
        "h_ansi",
        |p, v| {
            if v.is_some() || p.fonts.is_some() {
                p.fonts.get_or_insert_default().h_ansi = v.map(Into::into);
            }
        },
        ["Arial", "Courier New", "Noto Sans", "游明朝", ""],
    ),
    (
        "east_asia",
        |p, v| {
            if v.is_some() || p.fonts.is_some() {
                p.fonts.get_or_insert_default().east_asia = v.map(Into::into);
            }
        },
        ["Arial", "Courier New", "Noto Sans", "游明朝", ""],
    ),
    (
        "complex",
        |p, v| {
            if v.is_some() || p.fonts.is_some() {
                p.fonts.get_or_insert_default().complex = v.map(Into::into);
            }
        },
        ["Arial", "Courier New", "Noto Sans", "游明朝", ""],
    ),
    (
        "color",
        |p, v| p.color = v.map(Into::into),
        ["#000000", "#E01020", "", "auto", "#20A030"],
    ),
    (
        "highlight",
        |p, v| p.highlight = v.map(Into::into),
        ["yellow", "cyan", "none", "", "green"],
    ),
    (
        "shading",
        |p, v| {
            p.shading = v.map(|fill| jade_docx_reader::document::Shading {
                fill: Some(fill.into()),
                ..Default::default()
            })
        },
        ["#000000", "#E01020", "", "#8090A0", "#20A030"],
    ),
];

#[test]
fn render_hash_distinguishes_run_text_presence_positions_and_empty_fonts() {
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
        for value in ["", "same", "\0", "税😀e\u{301}"] {
            let mut keys = std::collections::HashSet::new();
            let mut check = |props, state: String| {
                paragraph.runs[0].properties = props;
                let key = renderer.compute_paragraph_render_hash(&paragraph, &options, content);
                assert!(
                    keys.insert(key),
                    "{state}, value={value:?}, content={content:?}"
                );
            };
            check(None, "absent properties".into());
            check(Some(RunProperties::default()), "absent fonts".into());
            for presence in 0..(1 << RUN_TEXT.len()) {
                let mut props = RunProperties {
                    fonts: Some(RunFonts::default()),
                    ..Default::default()
                };
                for (index, (_, setter, _)) in RUN_TEXT.iter().enumerate() {
                    setter(&mut props, (presence & (1 << index) != 0).then_some(value));
                }
                check(Some(props), format!("presence={presence:#x}"));
            }
            for hint in [FontHint::EastAsia, FontHint::Complex] {
                let props = RunProperties {
                    fonts: Some(RunFonts {
                        hint: Some(hint.clone()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                check(Some(props), format!("hint={hint:?}"));
            }
            assert_eq!(keys.len(), 4100);
        }
    }
}

#[test]
fn render_hash_distinguishes_metric_presence_positions_and_float_bits() {
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
        let absent = renderer.compute_paragraph_render_hash(&paragraph, &options, content);
        // Key the reference state by exact bits, including NaN payloads. These
        // values are hashed only; invalid layout metrics are not rendered.
        let mut states = std::collections::BTreeMap::new();
        let mut keys = std::collections::HashMap::new();
        {
            let mut check = |state: [Option<u32>; METRICS.len()]| {
                let mut props = RunProperties::default();
                for ((_, setter, _), bits) in METRICS.into_iter().zip(state) {
                    setter(&mut props, bits.map(f32::from_bits));
                }
                paragraph.runs[0].properties = Some(props);
                let key = renderer.compute_paragraph_render_hash(&paragraph, &options, content);
                assert_ne!(key, absent, "state={state:?}, content={content:?}");
                if let Some(previous) = states.insert(state, key) {
                    assert_eq!(key, previous, "revisit {state:?}, content={content:?}");
                }
                if let Some(previous) = keys.insert(key, state) {
                    assert_eq!(state, previous, "aliased states, content={content:?}");
                }
            };
            for bits in [
                0,
                0x8000_0000,
                1,
                0x8000_0001,
                0x3f80_0000,
                0xbf80_0000,
                0x7f80_0000,
                0xff80_0000,
                0x7fc0_0000,
                0x7fc0_0001,
                0x7fa0_0001,
                0xffc0_0001,
            ] {
                for present in 0..(1 << METRICS.len()) {
                    check(std::array::from_fn(|index| {
                        (present & (1 << index) != 0).then_some(bits)
                    }));
                }
            }
            for left in 0..METRICS.len() {
                for right in left + 1..METRICS.len() {
                    for (a, b) in [
                        (0x3f80_0000, 0xc000_0000),
                        (0, 0x8000_0000),
                        (0x7fc0_0001, 0x7fc0_0002),
                    ] {
                        for (a, b) in [(a, b), (b, a)] {
                            let mut state = [None; METRICS.len()];
                            state[left] = Some(a);
                            state[right] = Some(b);
                            check(state);
                        }
                    }
                }
            }
        }
        for (&state, &expected) in states.iter().rev() {
            let mut props = RunProperties::default();
            for ((_, setter, _), bits) in METRICS.into_iter().zip(state) {
                setter(&mut props, bits.map(f32::from_bits));
            }
            paragraph.runs[0].properties = Some(props);
            assert_eq!(
                renderer.compute_paragraph_render_hash(&paragraph, &options, content),
                expected,
                "revisit {state:?}, content={content:?}"
            );
        }
        assert_eq!(keys.len(), 433);
    }
}

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
    Metric(usize, Option<f32>),
    RunText(usize, Option<usize>),
    ClearFonts,
    ClearProperties,
    Text(usize),
    Repaint,
}

fn edit() -> impl Strategy<Value = Edit> {
    let metric = (0..METRICS.len()).prop_flat_map(|index| {
        prop::option::of(prop::sample::select(METRICS[index].2.to_vec()))
            .prop_map(move |value| Edit::Metric(index, value))
    });
    prop_oneof![
        8 => (0..FLAGS.len(), prop::option::of(any::<bool>())).prop_map(|(i, v)| Edit::Flag(i, v)),
        8 => metric,
        12 => (0..RUN_TEXT.len(), prop::option::of(0..5_usize)).prop_map(|(i, v)| Edit::RunText(i, v)),
        1 => Just(Edit::ClearFonts),
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

// Compare device-space ink and its effective drawing state. Cache replay may
// batch scaled text or translate a retained paragraph without changing ink.
// Keep painter state across paints; these documents use axis-aligned transforms.
type PaintState = std::collections::BTreeMap<&'static str, String>;
fn ink(painter: &RecordingPainter, state: &mut PaintState) -> Vec<(Op, PaintState)> {
    let mut stack = Vec::new();
    let mut result = Vec::new();
    let mut transform = [1.0, 1.0, 0.0, 0.0]; // sx, sy, tx, ty
    let rounded = |value: f64| (value * 1e9).round() / 1e9;
    for op in painter.ops() {
        let setting = match &op {
            Op::Save => {
                stack.push((state.clone(), transform));
                continue;
            }
            Op::Restore => {
                (*state, transform) = stack.pop().expect("balanced paint state");
                continue;
            }
            Op::Translate(x, y) => {
                transform[2] += transform[0] * x;
                transform[3] += transform[1] * y;
                continue;
            }
            Op::Scale(x, y) => {
                transform[0] *= x;
                transform[1] *= y;
                continue;
            }
            Op::SetFillStyle(v) => Some(("fill", v.clone())),
            Op::SetStrokeStyle(v) => Some(("stroke", v.clone())),
            Op::SetFont(v) => Some(("font", v.clone())),
            Op::SetTextBaseline(v) => Some(("baseline", v.clone())),
            Op::SetLineWidth(v) => Some(("width", format!("{v:?}"))),
            Op::SetLineDash(v) => Some(("dash", format!("{v:?}"))),
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
            let [sx, sy, tx, ty] = transform;
            let point = |x, y| (rounded(sx * x + tx), rounded(sy * y + ty));
            let op = match op {
                Op::FillText(text, x, y) => {
                    let (x, y) = point(x, y);
                    Op::FillText(text, x, y)
                }
                Op::StrokeText(text, x, y) => {
                    let (x, y) = point(x, y);
                    Op::StrokeText(text, x, y)
                }
                Op::FillShapedText(text, clusters, y) => Op::FillShapedText(
                    text,
                    clusters
                        .into_iter()
                        .map(|(i, x)| (i, point(x, y).0))
                        .collect(),
                    point(0.0, y).1,
                ),
                Op::Rect(x, y, w, h) | Op::FillRect(x, y, w, h) | Op::StrokeRect(x, y, w, h) => {
                    let (x, y) = point(x, y);
                    let (w, h) = (rounded(sx * w), rounded(sy * h));
                    match op {
                        Op::Rect(..) => Op::Rect(x, y, w, h),
                        Op::FillRect(..) => Op::FillRect(x, y, w, h),
                        _ => Op::StrokeRect(x, y, w, h),
                    }
                }
                Op::MoveTo(x, y) => {
                    let (x, y) = point(x, y);
                    Op::MoveTo(x, y)
                }
                Op::LineTo(x, y) => {
                    let (x, y) = point(x, y);
                    Op::LineTo(x, y)
                }
                Op::Arc(x, y, radius, start, end) => {
                    assert!(
                        sx > 0.0 && sy > 0.0,
                        "positive axis-aligned formatting transforms"
                    );
                    let (x, y) = point(x, y);
                    Op::Ellipse(
                        x,
                        y,
                        rounded(sx * radius),
                        rounded(sy * radius),
                        0.0,
                        start,
                        end,
                    )
                }
                Op::QuadraticCurveTo(cx, cy, x, y) => {
                    let (cx, cy) = point(cx, cy);
                    let (x, y) = point(x, y);
                    Op::QuadraticCurveTo(cx, cy, x, y)
                }
                Op::BezierCurveTo(ax, ay, bx, by, x, y) => {
                    let (ax, ay) = point(ax, ay);
                    let (bx, by) = point(bx, by);
                    let (x, y) = point(x, y);
                    Op::BezierCurveTo(ax, ay, bx, by, x, y)
                }
                Op::BeginPath | Op::ClosePath | Op::Fill | Op::Stroke | Op::Clip => op,
                _ => panic!("unexpected ink operation in formatting history: {op:?}"),
            };
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
            if matches!(
                op,
                Op::FillText(..) | Op::StrokeText(..) | Op::FillShapedText(..)
            ) {
                effective.insert("text scale", format!("{:?}", [rounded(sx), rounded(sy)]));
            }
            if matches!(op, Op::Stroke | Op::StrokeRect(..) | Op::StrokeText(..)) {
                effective.insert(
                    "dash",
                    state.get("dash").cloned().unwrap_or_else(|| "[]".into()),
                );
            }
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
    assert_eq!(transform, [1.0, 1.0, 0.0, 0.0]);
    result
}

#[test]
fn ink_resolves_translation_and_preserves_text_scale() {
    use jade_painter::Painter;
    let direct = RecordingPainter::new();
    direct.fill_text("A", 32.0, 20.0).unwrap();
    let replay = RecordingPainter::new();
    replay.save();
    replay.translate(0.0, 10.0).unwrap();
    replay.fill_text("A", 32.0, 10.0).unwrap();
    replay.restore();
    assert_eq!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );

    replay.clear();
    replay.save();
    replay.translate(0.0, 20.0).unwrap();
    replay.scale(0.5, 1.0).unwrap();
    replay.fill_text("A", 64.0, 0.0).unwrap();
    replay.restore();
    // The origin is unchanged, but the glyph's width is different.
    assert_ne!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );
    direct.clear();
    direct.save();
    direct.translate(32.0, 20.0).unwrap();
    direct.scale(0.5, 1.0).unwrap();
    direct.fill_text("A", 0.0, 0.0).unwrap();
    direct.restore();
    assert_eq!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );
}

#[test]
fn ink_resolves_emphasis_arcs_and_preserves_dash_state() {
    use jade_painter::Painter;
    let direct = RecordingPainter::new();
    direct.begin_path();
    direct
        .arc(32.0, 20.0, 4.0, 0.0, std::f64::consts::TAU)
        .unwrap();
    direct.fill();
    let replay = RecordingPainter::new();
    replay.save();
    replay.translate(0.0, 10.0).unwrap();
    replay.scale(0.5, 0.5).unwrap();
    replay.begin_path();
    replay
        .arc(64.0, 20.0, 8.0, 0.0, std::f64::consts::TAU)
        .unwrap();
    replay.fill();
    replay.restore();
    assert_eq!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );

    direct.clear();
    replay.clear();
    direct.set_line_dash(&[2.0, 3.0]);
    direct.stroke();
    replay.stroke();
    assert_ne!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );
    replay.clear();
    replay.set_line_dash(&[2.0, 3.0]);
    replay.stroke();
    assert_eq!(
        ink(&direct, &mut PaintState::new()),
        ink(&replay, &mut PaintState::new())
    );
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
    let mut metrics = [None; METRICS.len()];
    let mut strings = [None; RUN_TEXT.len()];
    let mut fonts_present = false;
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
            Edit::Metric(index, value) => {
                metrics[index] = value;
                has_properties = true;
                (METRICS[index].1)(paragraph.runs[0].properties.get_or_insert_default(), value);
            }
            Edit::RunText(index, value) => {
                strings[index] = value;
                has_properties = true;
                if (5..9).contains(&index) && value.is_some() {
                    fonts_present = true;
                }
                (RUN_TEXT[index].1)(
                    paragraph.runs[0].properties.get_or_insert_default(),
                    value.map(|i| RUN_TEXT[index].2[i]),
                );
            }
            Edit::ClearFonts => {
                strings[5..9].fill(None);
                fonts_present = false;
                has_properties = true;
                paragraph.runs[0].properties.get_or_insert_default().fonts = None;
            }
            Edit::ClearProperties => {
                strings.fill(None);
                fonts_present = false;
                flags.fill(None);
                metrics.fill(None);
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
            for ((_, setter, _), value) in METRICS.into_iter().zip(metrics) {
                setter(&mut props, value);
            }
            if fonts_present {
                props.fonts = Some(RunFonts::default());
            }
            for ((_, setter, values), value) in RUN_TEXT.into_iter().zip(strings) {
                setter(&mut props, value.map(|index| values[index]));
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
            format!("step {step}: {edit:?}, flags={flags:?}, metrics={metrics:?}, strings={strings:?}, fonts_present={fonts_present}, text={text}, windows={windows}");
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

#[test]
fn every_run_metric_survives_in_place_edits_and_restoration() {
    for windows in [false, true] {
        for text in 0..TEXTS.len() {
            for (index, (_, _, values)) in METRICS.into_iter().enumerate() {
                check_history(
                    &[
                        Edit::Text(text),
                        Edit::Metric(index, Some(values[0])),
                        Edit::Repaint,
                        Edit::Metric(index, Some(values[4])),
                        Edit::Metric(index, None),
                        Edit::Metric(index, Some(values[0])),
                        Edit::ClearProperties,
                        Edit::Repaint,
                    ],
                    windows,
                );
            }
        }
    }
}

#[test]
fn every_run_text_field_survives_in_place_edits_and_restoration() {
    for windows in [false, true] {
        for text in 0..TEXTS.len() {
            for index in 0..RUN_TEXT.len() {
                check_history(
                    &[
                        Edit::Text(text),
                        Edit::RunText(index, Some(0)),
                        Edit::Repaint,
                        Edit::RunText(index, Some(4)),
                        Edit::RunText(index, None),
                        Edit::Repaint,
                        Edit::RunText(index, Some(0)),
                        Edit::ClearFonts,
                        Edit::Repaint,
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
    fn run_style_edits_match_fresh_render_after_every_step(
        edits in prop::collection::vec(edit(), 1..25), windows in any::<bool>()
    ) {
        check_history(&edits, windows);
    }
}
