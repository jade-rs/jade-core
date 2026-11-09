//! Predict marks by decoding source scalars, then observe the actual draw path.
use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

fn reference_symbol(text: &str, offset: usize, mask: u8) -> Option<&'static str> {
    match text.get(offset..)?.chars().next()? {
        ' ' | '\u{a0}' if mask & 1 != 0 => Some("·"),
        '\u{3000}' if mask & 1 != 0 => Some("□"),
        '\u{ad}' if mask & 2 != 0 => Some("¬"),
        _ => None,
    }
}

fn kinds(mask: u8) -> crate::FormattingMarkKinds {
    crate::FormattingMarkKinds {
        spaces: mask & 1 != 0,
        optional_hyphens: mask & 2 != 0,
        ..Default::default()
    }
}

fn check_marks(
    renderer: &mut CanvasRenderer<RecordingPainter>,
    painter: &RecordingPainter,
    text: &str,
    offsets: &[usize],
    mask: u8,
    properties: &Rc<RunProperties>,
    context: &str,
) {
    renderer.mark_kinds = kinds(mask);
    let mut segment = mark_lookup_segment(0, text.len(), text.to_owned());
    segment.properties = Some(Rc::clone(properties));
    let line = SelectionLineLayout {
        fragments: Rc::from([SelectionFragmentLayout {
            x: 10.0,
            glyphs: offsets
                .iter()
                .enumerate()
                .map(|(i, offset)| GlyphBox {
                    start: *offset,
                    end: offset.saturating_add(1),
                    x: i as f64 * 20.0,
                    width: 14.0,
                })
                .collect::<Vec<_>>()
                .into(),
            ..Default::default()
        }]),
        ..Default::default()
    };
    // Retain raw text and offsets; do not call the renderer's lookup or infer
    // valid glyph boundaries to compute expected marks.
    let expected: Vec<_> = offsets
        .iter()
        .enumerate()
        .filter_map(|(i, offset)| {
            reference_symbol(text, *offset, mask)
                .map(|symbol| (symbol.to_owned(), 12.0 + i as f64 * 20.0, 34.0))
        })
        .collect();
    painter.clear();
    let mut commands = Vec::new();
    renderer
        .draw_whitespace_marks(&[&segment], &line, 34.0, false, Some(&mut commands))
        .unwrap();
    let actual: Vec<_> = commands
        .iter()
        .map(|command| match command {
            DrawCommand::ControlMark {
                symbol,
                x,
                baseline,
                style,
            } => {
                assert_eq!(
                    *style,
                    expected_control_mark_style(renderer, Some(properties)),
                    "{context}, mask={mask}"
                );
                (symbol.to_string(), *x, *baseline)
            }
            other => panic!("unexpected command: {other:?}; {context}"),
        })
        .collect();
    assert_eq!(actual, expected, "{context}, mask={mask}");
    let replay = RecordingPainter::with_width_per_char(6.0);
    crate::draw_command::executor::execute_commands(&replay, &commands).unwrap();
    let ink = |ops: Vec<Op>| {
        ops.into_iter()
            .filter(|op| matches!(op, Op::FillText(..) | Op::StrokeText(..)))
            .collect::<Vec<_>>()
    };
    assert_eq!(ink(painter.ops()), ink(replay.ops()), "{context}");
    assert_eq!(
        effective_fill_text_fonts(&painter.ops()),
        effective_fill_text_fonts(&replay.ops()),
        "{context}"
    );
    assert_eq!(unbalanced_saves(painter), 0, "{context}");
    assert_eq!(unbalanced_saves(&replay), 0, "{context}");
}

fn check_offsets(text: &str) {
    let (mut renderer, painter) = recording_renderer();
    let properties = Rc::new(RunProperties::default());
    let offsets: Vec<_> = (0..=text.len() + 1).chain([usize::MAX]).collect();
    for mask in 0..4 {
        check_marks(
            &mut renderer,
            &painter,
            text,
            &offsets,
            mask,
            &properties,
            text,
        );
    }
}

#[test]
fn whitespace_mark_commands_match_every_unicode_scalar_and_byte_boundary() {
    check_offsets("");
    // Bound memory while passing every scalar and every intervening byte
    // offset through segment lookup, mark collection, styling and replay.
    for start in (0..=0x10ffff).step_by(1024) {
        let text: String = (start..(start + 1024).min(0x110000))
            .filter_map(char::from_u32)
            .collect();
        check_offsets(&text);
    }
}

#[derive(Clone, Debug)]
enum Edit {
    Text(String),
    Offsets(Vec<u8>),
    Kinds(u8),
    Font(bool),
    Repaint,
}

fn edits() -> impl Strategy<Value = Edit> {
    let character = prop_oneof![
        6 => prop::sample::select(vec![' ', '\u{a0}', '\u{3000}', '\u{ad}', '日', '🦀', '\0']),
        1 => any::<char>(),
    ];
    prop_oneof![
        4 => prop::collection::vec(character, 0..18)
            .prop_map(|chars| Edit::Text(chars.into_iter().collect())),
        3 => prop::collection::vec(prop_oneof![4 => 0u8..64, 1 => any::<u8>()], 0..20).prop_map(Edit::Offsets),
        3 => (0u8..4).prop_map(Edit::Kinds),
        2 => any::<bool>().prop_map(Edit::Font),
        1 => Just(Edit::Repaint),
    ]
}

fn check_history(history: &[Edit]) {
    let (mut renderer, painter) = recording_renderer();
    let mut text = " 日\u{a0}🦀\u{3000}\u{ad}".to_owned();
    let mut offsets: Vec<usize> = (0..24).collect();
    let mut mask = 3;
    let mut properties = Rc::new(RunProperties::default());
    for (step, edit) in history.iter().enumerate() {
        match edit {
            Edit::Text(value) => text.clone_from(value),
            Edit::Offsets(value) => offsets = value.iter().map(|x| usize::from(*x)).collect(),
            Edit::Kinds(value) => mask = *value,
            Edit::Font(value) => {
                let properties = Rc::make_mut(&mut properties);
                properties.font_family = Some(if *value { "Calibri" } else { "Arial" }.into());
                properties.font_size = Some(if *value { 24.0 } else { 12.0 });
            }
            Edit::Repaint => {}
        }
        check_marks(
            &mut renderer,
            &painter,
            &text,
            &offsets,
            mask,
            &properties,
            &format!("step={step}, edit={edit:?}"),
        );
    }
}

#[test]
fn whitespace_mark_histories_preserve_partial_offsets_and_returned_settings() {
    check_history(&[
        Edit::Repaint,
        Edit::Kinds(0),
        Edit::Text("🦀\u{a0} \u{3000}\u{ad}\0".into()),
        Edit::Kinds(3),
        Edit::Font(true),
        Edit::Offsets(vec![3, 4, 5, 6, 9, 8, 10, 255, 6, 0]),
        Edit::Repaint,
        Edit::Kinds(1),
        Edit::Font(false),
        Edit::Kinds(2),
        Edit::Text(String::new()),
        Edit::Text(" \u{ad}\u{3000}".into()),
        Edit::Kinds(3),
    ]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn whitespace_mark_commands_match_decoded_mixed_text(text in ".{0,80}") {
        check_offsets(&text);
    }

    #[test]
    fn whitespace_mark_commands_follow_source_and_setting_histories(
        history in prop::collection::vec(edits(), 1..32),
    ) {
        check_history(&history);
    }
}
