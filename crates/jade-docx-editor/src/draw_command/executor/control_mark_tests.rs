use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;
use std::collections::HashSet;

const SOURCES: [&str; 7] = [
    "bold italic 72pt 'Times New Roman', serif",
    "2pt 'MS Mincho', serif",
    "Arial",
    "12pt Family pt suffix",
    "12pt ",
    "",
    "ot:liga 12px 'Noto Sans JP'",
];

// Literal expectations deliberately do not call the font-family, point-size,
// baseline or formatting helpers used by the executor. In particular, the size
// in the source descriptor is independent of TextStyle::font_size_pt.
const CASES: [(usize, f32, &str, f64); 12] = [
    (0, 12.0, "9.00pt 'Times New Roman', serif", 2.0),
    (0, 18.0, "13.50pt 'Times New Roman', serif", 3.0),
    (0, 10.5, "7.88pt 'Times New Roman', serif", 1.75),
    (1, 2.0, "1.50pt 'MS Mincho', serif", 1.0 / 3.0),
    (1, 1.0, "1.00pt 'MS Mincho', serif", 1.0 / 6.0),
    (1, 0.0, "1.00pt 'MS Mincho', serif", 0.0),
    (1, -6.0, "1.00pt 'MS Mincho', serif", -1.0),
    (2, 12.0, "9.00pt Arial", 2.0),
    (3, 12.0, "9.00pt Family pt suffix", 2.0),
    (4, 12.0, "9.00pt ", 2.0),
    (5, 12.0, "9.00pt ", 2.0),
    (6, 12.0, "9.00pt ot:liga 12px 'Noto Sans JP'", 2.0),
];

const SYMBOLS: [&str; 10] = [
    "",
    "·",
    "→",
    "¶",
    "♥️",
    "👍🏽",
    "👩‍💻",
    "12345678",
    "123456789",
    "日本語",
];
const COLORS: [&str; 2] = ["#708090", "rgba(12, 34, 56, 0.75)"];

fn mark_style(case: usize, source: Rc<str>, baseline_shift_px: f64) -> TextStyle {
    TextStyle {
        font_size_pt: CASES[case].1,
        font_descriptor_latin_ascii: source,
        // These fonts and ordinary text decorations must not affect a mark.
        font_descriptor_latin_hansi: Rc::from("99pt Wrong HAnsi"),
        font_descriptor_east_asia: Rc::from("99pt Wrong East Asia"),
        font_descriptor_complex: Rc::from("99pt Wrong Complex"),
        baseline_shift_px,
        fill_color: Rc::from(COLORS[case % COLORS.len()]),
        letter_spacing_px: 19.0,
        text_scale_x: 2.0,
        enable_pair_kerning: true,
        highlight_color: Some(Rc::from("#ff0000")),
        strike: true,
        underline: true,
        shadow: true,
        outline: true,
        emboss: true,
        imprint: true,
        ..TextStyle::default()
    }
}

#[test]
fn control_mark_geometry_and_font_match_literal_matrix() {
    // 12 descriptor/size cases × 10 symbols × 3 shifts, including empty marks,
    // the 8-byte width-cache boundary, composed symbols and a ZWJ sequence.
    for (case, &(source, _, font, lift)) in CASES.iter().enumerate() {
        for symbol in SYMBOLS {
            for (shift, shifted_baseline) in [(-4.0, 37.0), (0.0, 40.0), (8.0, 46.0)] {
                let style = mark_style(case, Rc::from(SOURCES[source]), shift);
                let expected = if symbol.is_empty() {
                    Vec::new()
                } else {
                    vec![
                        Op::SetFillStyle(COLORS[case % COLORS.len()].into()),
                        Op::SetFont(font.into()),
                        Op::MeasureText(symbol.into()),
                        Op::FillText(symbol.into(), 3.0, shifted_baseline - lift),
                    ]
                };
                let painter = RecordingPainter::with_width_per_char(7.0);
                let width = execute_control_mark(&painter, symbol, 3.0, 40.0, &style).unwrap();
                assert_eq!(width, symbol.chars().count() as f64 * 7.0);
                assert_eq!(
                    painter.ops(),
                    expected,
                    "case={case}, symbol={symbol:?}, shift={shift}"
                );

                // Check the public command dispatch as well as the width-returning
                // helper, so a regression cannot bypass the control-mark path.
                painter.clear();
                execute_command(
                    &painter,
                    &DrawCommand::ControlMark {
                        symbol: symbol.into(),
                        x: 3.0,
                        baseline: 40.0,
                        style,
                    },
                )
                .unwrap();
                assert_eq!(
                    painter.ops(),
                    expected,
                    "dispatch: case={case}, symbol={symbol:?}, shift={shift}"
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Action {
    Mark {
        case: usize,
        symbol: usize,
        fresh: bool,
    },
    Leader(usize),
    Save,
    Restore,
    Invalidate,
    InvalidateFill,
    ClearWidths,
    Reset,
    Rectangle,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        12 => (0usize..CASES.len(), 0usize..SYMBOLS.len(), any::<bool>())
            .prop_map(|(case, symbol, fresh)| Action::Mark { case, symbol, fresh }),
        3 => (0usize..CASES.len()).prop_map(Action::Leader),
        1 => Just(Action::Save),
        2 => Just(Action::Restore),
        2 => Just(Action::Invalidate),
        1 => Just(Action::InvalidateFill),
        2 => Just(Action::ClearWidths),
        1 => Just(Action::Reset),
        2 => Just(Action::Rectangle),
    ]
}

fn check_replay(actions: &[Action]) -> Result<(), TestCaseError> {
    let painter = RecordingPainter::with_width_per_char(7.0);
    let sources: Vec<Rc<str>> = SOURCES.iter().map(|source| Rc::from(*source)).collect();
    let mut tracker = FontStateTracker::new();

    // The reference state contains string values, not Rc identities, hashes or
    // any executor helper. Widths survive painter-state invalidation, but not
    // an explicit clear or a new tracker.
    let mut font: Option<&str> = None;
    let mut color: Option<&str> = None;
    let mut measured = HashSet::new();
    for (step, action) in actions.iter().enumerate() {
        painter.clear();
        let mut expected = Vec::new();
        let command = match *action {
            Action::Mark {
                case,
                symbol,
                fresh,
            } => {
                let (source, _, scaled_font, lift) = CASES[case];
                let symbol = SYMBOLS[symbol];
                let fill = COLORS[case % COLORS.len()];
                if !symbol.is_empty() {
                    if color != Some(fill) {
                        expected.push(Op::SetFillStyle(fill.into()));
                    }
                    color = Some(fill);
                    if font != Some(scaled_font) {
                        expected.push(Op::SetFont(scaled_font.into()));
                    }
                    font = Some(scaled_font);
                    if symbol.len() > 8 || measured.insert((scaled_font, symbol)) {
                        expected.push(Op::MeasureText(symbol.into()));
                    }
                    expected.push(Op::FillText(symbol.into(), 3.0, 46.0 - lift));
                }
                let source = if fresh {
                    Rc::from(SOURCES[source])
                } else {
                    Rc::clone(&sources[source])
                };
                Some(DrawCommand::ControlMark {
                    symbol: symbol.into(),
                    x: 3.0,
                    baseline: 40.0,
                    style: mark_style(case, source, 8.0),
                })
            }
            Action::Leader(case) => {
                let source = CASES[case].0;
                let fill = COLORS[case % COLORS.len()];
                if color != Some(fill) {
                    expected.push(Op::SetFillStyle(fill.into()));
                }
                color = Some(fill);
                if font != Some(SOURCES[source]) {
                    expected.push(Op::SetFont(SOURCES[source].into()));
                }
                font = Some(SOURCES[source]);
                expected.extend([
                    Op::FillText(".".into(), 3.0, 48.0),
                    Op::FillText(".".into(), 13.0, 48.0),
                ]);
                Some(DrawCommand::TabLeader {
                    baseline: 40.0,
                    start_x: 3.0,
                    end_x: 23.0,
                    leader: TabLeaderStyle::Dot,
                    glyph_positions: vec![3.0, 13.0].into(),
                    style: mark_style(case, Rc::clone(&sources[source]), 8.0),
                })
            }
            Action::Save => {
                expected.push(Op::Save);
                Some(DrawCommand::Save)
            }
            Action::Restore => {
                expected.push(Op::Restore);
                font = None;
                color = None;
                Some(DrawCommand::Restore)
            }
            Action::Invalidate => {
                font = None;
                color = None;
                tracker.invalidate();
                None
            }
            Action::InvalidateFill => {
                color = None;
                tracker.invalidate_fill_style();
                None
            }
            Action::ClearWidths => {
                measured.clear();
                tracker.clear_cache();
                None
            }
            Action::Reset => {
                font = None;
                color = None;
                measured.clear();
                tracker = FontStateTracker::new();
                None
            }
            Action::Rectangle => {
                expected.extend([
                    Op::SetFillStyle("#abcdef".into()),
                    Op::FillRect(1.0, 2.0, 3.0, 4.0),
                ]);
                color = None;
                Some(DrawCommand::FillRect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                    color: Rc::from("#abcdef"),
                })
            }
        };
        if let Some(command) = command {
            prop_assert!(
                execute_command_with_font_state(&painter, &command, &mut tracker).is_ok(),
                "step {step}, action {action:?}"
            );
        }
        prop_assert_eq!(
            painter.ops(),
            expected,
            "step {}, action {:?}",
            step,
            action
        );
    }
    Ok(())
}

#[test]
fn control_mark_replay_covers_repeated_fonts_and_state_changes() {
    let mark = |case, symbol, fresh| Action::Mark {
        case,
        symbol,
        fresh,
    };
    check_replay(&[
        mark(0, 1, false),
        mark(0, 1, false),
        mark(0, 1, true),
        // The same source Rc at a different size must select a new font/width.
        mark(1, 1, false),
        mark(0, 1, false),
        // Empty marks must leave the font and color unchanged.
        mark(7, 0, true),
        mark(0, 1, false),
        Action::Save,
        Action::Leader(7),
        mark(0, 1, false),
        Action::Restore,
        mark(0, 1, false),
        Action::Rectangle,
        mark(0, 1, false),
        Action::InvalidateFill,
        mark(0, 1, false),
        Action::Invalidate,
        mark(0, 1, false),
        Action::ClearWidths,
        mark(0, 1, false),
        mark(0, 5, false),
        mark(0, 5, true),
        mark(0, 6, false),
        mark(0, 6, true),
        mark(0, 7, false),
        mark(0, 7, true),
        mark(0, 8, false),
        mark(0, 8, true),
        Action::Reset,
        mark(0, 1, false),
        mark(1, 1, true),
    ])
    .unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn control_mark_replay_matches_value_model(actions in prop::collection::vec(action(), 1..97)) {
        check_replay(&actions)?;
    }
}
