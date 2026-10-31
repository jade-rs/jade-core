use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

#[test]
fn replay_fill_style_retains_the_command_color() {
    let painter = RecordingPainter::new();
    let color: Rc<str> = Rc::from("rgba(12, 34, 56, 0.75)");
    let command = DrawCommand::SetFillStyle {
        color: Rc::clone(&color),
    };
    let mut tracker = FontStateTracker::new();
    execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
    assert_eq!(
        tracker.current_fill_style.as_deref().unwrap().as_ptr(),
        color.as_ptr(),
        "replay must retain the existing color allocation instead of copying it"
    );
    drop(command);
    assert_eq!(Rc::strong_count(&color), 2);
    tracker.invalidate_fill_style();
    assert_eq!(Rc::strong_count(&color), 1);
}

#[test]
fn replay_text_retains_the_style_color() {
    let painter = RecordingPainter::with_width_per_char(7.0);
    let color: Rc<str> = Rc::from("#123456");
    let command = DrawCommand::Text {
        text: "abc".into(),
        x: 3.0,
        y: 15.0,
        style: TextStyle {
            fill_color: Rc::clone(&color),
            ..TextStyle::default()
        },
    };
    let mut tracker = FontStateTracker::new();
    execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
    assert_eq!(
        tracker.current_fill_style.as_deref().unwrap().as_ptr(),
        color.as_ptr()
    );
    assert_eq!(
        painter.count(|op| matches!(op, Op::SetFillStyle(value) if value == "#123456")),
        1
    );
    assert!(painter
        .ops()
        .contains(&Op::FillText("abc".into(), 3.0, 15.0)));
}

#[derive(Clone, Debug)]
enum FillAction {
    Shared(usize),
    Fresh(usize),
    Borrowed(usize),
    Rectangle(usize),
    Selection,
    Save,
    Restore,
    Invalidate,
    InvalidateFill,
    Reset,
}

fn fill_action() -> impl Strategy<Value = FillAction> {
    prop_oneof![
        5 => (0usize..5).prop_map(FillAction::Shared),
        5 => (0usize..5).prop_map(FillAction::Fresh),
        3 => (0usize..5).prop_map(FillAction::Borrowed),
        2 => (0usize..5).prop_map(FillAction::Rectangle),
        1 => Just(FillAction::Selection),
        1 => Just(FillAction::Save),
        1 => Just(FillAction::Restore),
        1 => Just(FillAction::Invalidate),
        1 => Just(FillAction::InvalidateFill),
        1 => Just(FillAction::Reset),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn fill_style_replay_matches_value_model(actions in prop::collection::vec(fill_action(), 1..97)) {
        // The model only knows color values and when the canvas state becomes
        // unknown. It does not know about Rc identity or the cache storage type.
        let colors = ["#000000", "#ffffff", "rgba(12, 34, 56, 0.75)", "", "色🎨"];
        let shared: Vec<Rc<str>> = colors.iter().map(|value| Rc::from(*value)).collect();
        let mut known_color: Option<&str> = None;
        let painter = RecordingPainter::new();
        let mut tracker = FontStateTracker::new();

        for (step, action) in actions.iter().enumerate() {
            painter.clear();
            let mut expected = Vec::new();
            let command = match *action {
                FillAction::Shared(index) | FillAction::Fresh(index) | FillAction::Borrowed(index) => {
                    let color = colors[index];
                    let changed = known_color != Some(color);
                    if changed {
                        expected.push(Op::SetFillStyle(color.into()));
                    }
                    known_color = Some(color);
                    match action {
                        FillAction::Shared(_) => Some(DrawCommand::SetFillStyle { color: Rc::clone(&shared[index]) }),
                        FillAction::Fresh(_) => Some(DrawCommand::SetFillStyle { color: Rc::from(color) }),
                        _ => {
                            prop_assert_eq!(tracker.set_fill_style_if_changed(&painter, color), changed,
                                "step {}, action {:?}", step, action);
                            None
                        }
                    }
                }
                FillAction::Rectangle(index) => {
                    expected.push(Op::SetFillStyle(colors[index].into()));
                    expected.push(Op::FillRect(1.0, 2.0, 3.0, 4.0));
                    known_color = None;
                    Some(DrawCommand::FillRect { x: 1.0, y: 2.0, width: 3.0, height: 4.0, color: Rc::clone(&shared[index]) })
                }
                FillAction::Selection => {
                    expected.push(Op::SetFillStyle("rgba(51, 153, 255, 0.35)".into()));
                    expected.push(Op::FillRect(1.0, 2.0, 3.0, 4.0));
                    known_color = None;
                    Some(DrawCommand::SelectionHighlight { x: 1.0, y: 2.0, width: 3.0, height: 4.0 })
                }
                FillAction::Save => {
                    expected.push(Op::Save);
                    Some(DrawCommand::Save)
                }
                FillAction::Restore => {
                    expected.push(Op::Restore);
                    known_color = None;
                    Some(DrawCommand::Restore)
                }
                FillAction::Invalidate => {
                    known_color = None;
                    tracker.invalidate();
                    None
                }
                FillAction::InvalidateFill => {
                    known_color = None;
                    tracker.invalidate_fill_style();
                    None
                }
                FillAction::Reset => {
                    known_color = None;
                    tracker = FontStateTracker::new();
                    None
                }
            };
            if let Some(command) = command {
                prop_assert!(execute_command_with_font_state(&painter, &command, &mut tracker).is_ok(),
                    "step {}, action {:?}", step, action);
            }
            prop_assert_eq!(painter.ops(), expected, "step {}, action {:?}", step, action);
        }
    }
}
