use super::*;
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Command {
    BeginFull,
    Begin(Option<usize>),
    Write(usize, String, usize),
    Append(usize, String),
    Ensure(usize),
    Reset(usize),
    Selectable(usize, bool),
    Freeze(bool),
    TakeSuffix(usize),
    RestoreSuffix,
    Select(usize, usize),
}

fn command() -> impl Strategy<Value = Command> {
    let text =
        prop::sample::select(vec!["", "abc", "日🦀", "e\u{301}", "\t  "]).prop_map(str::to_owned);
    prop_oneof![
        1 => Just(Command::BeginFull),
        3 => prop::option::of(0usize..14).prop_map(Command::Begin),
        5 => (0usize..12, text.clone(), 0usize..5)
            .prop_map(|(index, text, lines)| Command::Write(index, text, lines)),
        2 => (0usize..12, text).prop_map(|(index, text)| Command::Append(index, text)),
        1 => (0usize..12).prop_map(Command::Ensure),
        1 => (0usize..12).prop_map(Command::Reset),
        1 => (0usize..12, any::<bool>()).prop_map(|(index, value)| Command::Selectable(index, value)),
        1 => any::<bool>().prop_map(Command::Freeze),
        2 => (0usize..14).prop_map(Command::TakeSuffix),
        2 => Just(Command::RestoreSuffix),
        1 => (0usize..14, 0usize..16).prop_map(|(p, o)| Command::Select(p, o)),
    ]
}

#[derive(Clone, Debug, Default)]
struct Paragraph {
    text: String,
    // Each line has its own source string and disjoint vertical interval.
    lines: Vec<(String, usize)>,
    registered_at: usize,
}

fn touch(paragraphs: &mut Vec<Paragraph>, index: usize, pass: usize) -> &mut Paragraph {
    while paragraphs.len() <= index {
        paragraphs.push(Paragraph::default());
    }
    let paragraph = &mut paragraphs[index];
    if paragraph.registered_at != pass {
        paragraph.text = String::new();
        paragraph.lines = Vec::new();
        paragraph.registered_at = pass;
    }
    paragraph
}

fn line(text: &str, slot: usize) -> LineLayout {
    let top = slot as f64 * 20.0;
    LineLayout {
        top,
        absolute_top: top,
        content_top: top,
        absolute_content_top: top,
        height: 10.0,
        content_height: 10.0,
        baseline: top + 8.0,
        start_x: 10.0,
        cursor_end_x: 30.0,
        fragments: Rc::from([FragmentLayout {
            start: 0,
            end: text.len(),
            x: 10.0,
            width: 20.0,
            glyphs: Rc::from([GlyphBox {
                start: 0,
                end: text.len(),
                x: 0.0,
                width: 20.0,
            }]),
            ..FragmentLayout::default()
        }]),
        ..LineLayout::default()
    }
}

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 192,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn render_registration_histories_match_independent_model(
        commands in prop::collection::vec(command(), 1..97),
    ) {
        let mut manager = SelectionManager::new();
        let mut paragraphs = Vec::<Paragraph>::new();
        let mut saved = None;
        let mut excluded = HashSet::new();
        let mut caret_origins = HashSet::new();
        let mut pass = 0;
        let mut frozen = false;
        let mut selection = None;
        let mut preserved = 0;
        for (step, command) in commands.iter().enumerate() {
            match command {
                Command::BeginFull | Command::Begin(_) => {
                    let dirty = match command { Command::Begin(dirty) => *dirty, _ => None };
                    if matches!(command, Command::BeginFull) { manager.begin_render(); }
                    else { manager.begin_render_incremental(dirty); }
                    if !frozen {
                        preserved = dirty.filter(|&from| from > 0 && from <= paragraphs.len()).unwrap_or(0);
                        paragraphs.truncate(preserved);
                        excluded.retain(|&index| index < preserved);
                        caret_origins.retain(|&(index, _)| index < preserved);
                        pass += 1;
                    }
                }
                Command::Write(index, text, count) => {
                    manager.reset_paragraph_registration(*index);
                    manager.append_paragraph_text(*index, text);
                    for part in 0..*count {
                        manager.record_line_with_caret_start(*index, line(text, step * 5 + part), 12.0);
                    }
                    if !frozen {
                        let paragraph = touch(&mut paragraphs, *index, pass);
                        paragraph.text = text.clone();
                        paragraph.lines = (0..*count).map(|part| (text.clone(), step * 5 + part)).collect();
                        caret_origins.extend((0..*count).map(|part| (*index, part)));
                    }
                }
                Command::Append(index, text) => {
                    manager.append_paragraph_text(*index, text);
                    if !frozen {
                        touch(&mut paragraphs, *index, pass).text.push_str(text);
                    }
                }
                Command::Ensure(index) => {
                    manager.ensure_paragraph(*index);
                    if !frozen {
                        let _ = touch(&mut paragraphs, *index, pass);
                    }
                }
                Command::Reset(index) => {
                    manager.reset_paragraph_registration(*index);
                    if !frozen {
                        let paragraph = touch(&mut paragraphs, *index, pass);
                        paragraph.text = String::new();
                        paragraph.lines = Vec::new();
                    }
                }
                Command::Selectable(index, selectable) => {
                    manager.set_paragraph_selectable(*index, *selectable);
                    if !frozen {
                        let _ = touch(&mut paragraphs, *index, pass);
                        if *selectable { excluded.remove(index); } else { excluded.insert(*index); }
                    }
                }
                Command::Freeze(value) => {
                    frozen = *value;
                    if frozen { manager.freeze(); } else { manager.unfreeze(); }
                }
                Command::TakeSuffix(from) => {
                    let tail = manager.take_selection_suffix(*from);
                    let model_tail = paragraphs.iter().skip(*from).cloned().collect::<Vec<_>>();
                    paragraphs.truncate(*from);
                    let tail_excluded = excluded.iter().copied().filter(|p| p >= from).collect::<Vec<_>>();
                    excluded.retain(|p| p < from);
                    let tail_origins = caret_origins.iter().copied().filter(|(p, _)| p >= from).collect::<Vec<_>>();
                    caret_origins.retain(|(p, _)| p < from);
                    saved = Some((*from, tail, model_tail, tail_excluded, tail_origins));
                }
                Command::RestoreSuffix => {
                    if let Some((from, tail, model_tail, tail_excluded, tail_origins)) = saved.take() {
                        manager.restore_selection_suffix(from, tail);
                        // An empty tail leaves the active paragraph registry alone.
                        if !model_tail.is_empty() {
                            paragraphs.truncate(from);
                            paragraphs.extend(model_tail);
                        }
                        // Selection metadata is replaced even for an empty tail.
                        excluded.retain(|&index| index < from);
                        excluded.extend(tail_excluded);
                        caret_origins.retain(|&(index, _)| index < from);
                        caret_origins.extend(tail_origins);
                    }
                }
                Command::Select(paragraph, offset) => {
                    let position = TextPosition::new(*paragraph, *offset);
                    selection = Some(SelectionRange::new(position, position));
                    manager.set_selection_for_render(selection);
                }
            }
            prop_assert_eq!(manager.is_frozen(), frozen, "step {}: {:?}", step, command);
            prop_assert_eq!(manager.selection_range(), selection, "step {}: {:?}", step, command);
            prop_assert_eq!(manager.preserved_prefix_len(), preserved, "step {}: {:?}", step, command);
            prop_assert_eq!(manager.paragraphs().len(), paragraphs.len(), "step {}: {:?}", step, command);
            for (index, (actual, expected)) in manager.paragraphs().iter().zip(&paragraphs).enumerate() {
                prop_assert_eq!(&actual.text, &expected.text, "step {}: {:?}, paragraph {}", step, command, index);
                prop_assert_eq!(actual.lines.len(), expected.lines.len(), "step {}: {:?}, paragraph {}", step, command, index);
                for (line_index, (actual_line, (text, slot))) in actual.lines.iter().zip(&expected.lines).enumerate() {
                    let caret_x = if caret_origins.contains(&(index, line_index)) { 12.0 }
                        else if text.is_empty() { 30.0 } else { 10.0 };
                    prop_assert_eq!(manager.caret_x_on_line(index, line_index, 0), Some(caret_x), "step {}: {:?}", step, command);
                    let top = *slot as f64 * 20.0;
                    prop_assert_eq!((actual_line.absolute_top, actual_line.height), (top, 10.0));
                    prop_assert_eq!(actual_line.fragments[0].end, text.len());
                    for (x, offset) in [(10.0, 0), (30.0, text.len())] {
                        let hit = manager.hit_test_matching(x, top + 5.0, |p| p == index);
                        let expected_hit = (!excluded.contains(&index)).then_some(TextPosition::new(index, offset));
                        prop_assert_eq!(hit, expected_hit, "step {}: {:?}, paragraph {}", step, command, index);
                    }
                }
            }
        }
    }
}

#[test]
fn recycling_releases_line_contents_and_reuses_only_empty_buffers() {
    let mut manager = SelectionManager::new();
    manager.append_paragraph_text(0, "old paragraph text");
    let old_line = line("old paragraph text", 0);
    let fragments = Rc::downgrade(&old_line.fragments);
    manager.record_line(0, old_line);
    assert!(manager.hit_test(10.0, 5.0).is_some());
    let text_buffer = manager.paragraphs()[0].text.as_ptr();
    let line_buffer = manager.paragraphs()[0].lines.as_ptr();

    manager.begin_render();
    assert!(manager.paragraphs().is_empty());
    assert!(fragments.upgrade().is_none());
    manager.ensure_paragraph(0);
    let paragraph = &manager.paragraphs()[0];
    assert!(paragraph.text.is_empty());
    assert!(paragraph.lines.is_empty());
    assert_eq!(paragraph.bounding_box(), None);
    assert_eq!(paragraph.text.as_ptr(), text_buffer);
    assert_eq!(paragraph.lines.as_ptr(), line_buffer);
    manager.append_paragraph_text(0, "new");
    manager.record_line(0, line("new", 50));
    assert_eq!(manager.paragraphs()[0].text, "new");
    assert_eq!(
        manager.hit_test(30.0, 1005.0),
        Some(TextPosition::new(0, 3))
    );
}

#[test]
fn incremental_recycling_keeps_prefix_and_bounds_retained_capacity() {
    let mut manager = SelectionManager::new();
    for index in 0..300 {
        manager.append_paragraph_text(index, "text");
        manager.record_line(index, line("text", index));
    }
    let prefix_text = manager.paragraphs()[0].text.as_ptr();
    let prefix_lines = manager.paragraphs()[0].lines.as_ptr();
    manager.begin_render_incremental(Some(1));
    assert_eq!(manager.paragraphs().len(), 1);
    assert_eq!(manager.paragraphs()[0].text, "text");
    assert_eq!(manager.paragraphs()[0].text.as_ptr(), prefix_text);
    assert_eq!(manager.paragraphs()[0].lines.as_ptr(), prefix_lines);
    assert!(manager.paragraph_buffers.len() <= 256);
    let retained: usize = manager
        .paragraph_buffers
        .iter()
        .map(|p| {
            assert!(p.text.is_empty() && p.lines.is_empty());
            p.text.capacity() + p.lines.capacity() * std::mem::size_of::<LineLayout>()
        })
        .sum();
    assert!(retained <= 1024 * 1024);
    // A second pass without registration releases unused previous buffers.
    manager.begin_render_incremental(Some(1));
    assert!(manager.paragraph_buffers.is_empty());

    manager.append_paragraph_text(1, &"x".repeat(1024 * 1024 + 1));
    manager.begin_render_incremental(Some(1));
    assert!(manager.paragraph_buffers.is_empty());
    assert_eq!(manager.paragraphs()[0].text, "text");
}
