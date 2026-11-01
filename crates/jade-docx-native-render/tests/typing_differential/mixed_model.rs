//! Mixed editing is modeled as characters with flags, independently of runs.

use super::{assert_page_pixels, assert_same, open_bytes, package, Doc};
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_native_render::CharacterFormat;
use jade_selection::{SelectionRange, TextPosition};
use proptest::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct Character {
    value: char,
    flags: [bool; 3],
}

type State = Vec<Character>;

#[derive(Debug)]
struct Model {
    text: State,
    undo: Vec<State>,
    redo: Vec<State>,
}

fn initial() -> State {
    [
        ("ab", [true, false, false]),
        ("日本", [false, true, false]),
        ("e\u{301}😀", [false, false, true]),
        ("終", [false; 3]),
    ]
    .into_iter()
    .flat_map(|(text, flags)| text.chars().map(move |value| Character { value, flags }))
    .collect()
}

fn open() -> Doc {
    open_bytes(&package(
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>ab</w:t></w:r>\
         <w:r><w:rPr><w:i/></w:rPr><w:t>日本</w:t></w:r>\
         <w:r><w:rPr><w:u w:val=\"single\"/></w:rPr><w:t>é😀</w:t></w:r>\
         <w:r><w:t>終</w:t></w:r></w:p>",
        "",
    ))
}

fn text(state: &State) -> String {
    state.iter().map(|character| character.value).collect()
}

fn offset(state: &State, index: usize) -> usize {
    state[..index]
        .iter()
        .map(|character| character.value.len_utf8())
        .sum()
}

fn range(state: &State, start: usize, end: usize) -> SelectionRange {
    SelectionRange::new(
        TextPosition::new(0, offset(state, start)),
        TextPosition::new(0, offset(state, end)),
    )
}

fn replaced(state: &State, from: usize, to: usize, replacement: &str) -> State {
    // IME replacement preserves unchanged prefix/suffix characters and their
    // formatting. Only the changed middle acquires the replacement style.
    let replacement: Vec<_> = replacement.chars().collect();
    let selected = &state[from..to];
    let mut prefix = 0;
    while prefix < selected.len().min(replacement.len())
        && selected[prefix].value == replacement[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < (selected.len() - prefix).min(replacement.len() - prefix)
        && selected[selected.len() - suffix - 1].value
            == replacement[replacement.len() - suffix - 1]
    {
        suffix += 1;
    }
    let start = from + prefix;
    let end = to - suffix;
    let flags = state[if start == end {
        start.saturating_sub(1)
    } else {
        start
    }]
    .flags;
    let mut next = state.clone();
    next.splice(
        start..end,
        replacement[prefix..replacement.len() - suffix]
            .iter()
            .map(|&value| Character { value, flags }),
    );
    next
}

fn format_command(range: SelectionRange, kind: u8) -> HeadlessCommand {
    let (start, end) = range.normalized();
    match kind {
        0 => HeadlessCommand::ApplyBold {
            start_paragraph: 0,
            start_offset: start.offset,
            end_paragraph: 0,
            end_offset: end.offset,
            enabled: true,
        },
        1 => HeadlessCommand::ApplyItalic {
            start_paragraph: 0,
            start_offset: start.offset,
            end_paragraph: 0,
            end_offset: end.offset,
            enabled: true,
        },
        2 => HeadlessCommand::ApplyUnderline {
            start_paragraph: 0,
            start_offset: start.offset,
            end_paragraph: 0,
            end_offset: end.offset,
            enabled: true,
            kind: None,
        },
        3 => HeadlessCommand::ClearFormatting {
            start_paragraph: 0,
            start_offset: start.offset,
            end_paragraph: 0,
            end_offset: end.offset,
        },
        _ => unreachable!(),
    }
}

impl Model {
    fn change(&mut self, next: State) {
        self.undo.push(std::mem::replace(&mut self.text, next));
        self.redo.clear();
    }

    fn check_content(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 1, "{context}: paragraphs");
        assert_eq!(
            document.paragraph_text(0),
            Some(text(&self.text)),
            "{context}: text"
        );
        let mut byte = 0;
        for character in &self.text {
            let style = document.run_style_at(0, byte).unwrap();
            assert_eq!(
                [
                    style.bold.unwrap_or(false),
                    style.italic.unwrap_or(false),
                    style.underline.unwrap_or(false)
                ],
                character.flags,
                "{context}: style at byte {byte} ({:?})",
                character.value
            );
            byte += character.value.len_utf8();
        }
    }

    fn check(&self, document: &Doc, context: &str) {
        self.check_content(document, context);
        assert_eq!(
            document.history_len(),
            self.undo.len(),
            "{context}: history"
        );
        assert_eq!(
            document.can_undo(),
            !self.undo.is_empty(),
            "{context}: undo"
        );
        assert_eq!(
            document.can_redo(),
            !self.redo.is_empty(),
            "{context}: redo"
        );
        assert_eq!(
            document.selection_range(),
            document.renderer().selection_range(),
            "{context}: selection"
        );
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Format(u8, u8, u8),
    Insert(u8, &'static str),
    Replace(u8, u8, &'static str),
    Delete(u8, u8),
    ReplaceAll(&'static str, &'static str),
    FailedBatch,
    Merge(u8),
    Undo,
    Redo,
}

fn check_step(
    model: &mut Model,
    actual: &mut Doc,
    reference: &mut Doc,
    operation: &Operation,
    step: usize,
) {
    let context = format!("mixed step {step}: {operation:?}, before={model:?}");
    // Keep the terminal character as an anchor. Empty-paragraph formatting
    // carriers are a separate contract, covered by the paragraph tests.
    let index = |raw: u8| usize::from(raw) % model.text.len();
    match *operation {
        Operation::Format(kind, from, to) => {
            let from = usize::from(from) % (model.text.len() + 1);
            let to = usize::from(to) % (model.text.len() + 1);
            let selection = range(&model.text, from, to);
            actual.set_selection_range(Some(selection));
            reference.set_selection_range(Some(selection));
            let result = actual.format_selection(
                [
                    CharacterFormat::Bold,
                    CharacterFormat::Italic,
                    CharacterFormat::Underline,
                    CharacterFormat::Clear,
                ][usize::from(kind)],
                1,
            );
            let full = reference.apply_command(format_command(selection, kind), true);
            if from == to {
                assert!(result.is_err(), "{context}");
                assert!(full.is_err(), "{context}");
            } else {
                result.unwrap();
                full.unwrap();
                let mut next = model.text.clone();
                for character in &mut next[from.min(to)..from.max(to)] {
                    if kind == 3 {
                        character.flags = [false; 3];
                    } else {
                        character.flags[usize::from(kind)] = true;
                    }
                }
                model.change(next);
            }
        }
        Operation::Insert(at, inserted) => {
            let at = index(at);
            let byte = offset(&model.text, at);
            actual.insert_text(0, byte, inserted, 1).unwrap();
            reference
                .apply_command(
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: byte,
                        text: inserted.into(),
                    },
                    false,
                )
                .unwrap();
            if !inserted.is_empty() {
                // Typing at a boundary continues the character on the left;
                // at the paragraph start it continues the first character.
                let flags = model.text[at.saturating_sub(1)].flags;
                let mut next = model.text.clone();
                next.splice(
                    at..at,
                    inserted.chars().map(|value| Character { value, flags }),
                );
                model.change(next);
            }
        }
        Operation::Replace(from, to, replacement) => {
            let from = index(from);
            let to = index(to);
            let start = from.min(to);
            let end = from.max(to);
            let selected = range(&model.text, start, end);
            actual
                .replace_range(
                    0,
                    selected.anchor.offset,
                    selected.focus.offset,
                    replacement,
                    1,
                )
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: selected.anchor.offset,
                        end_paragraph: 0,
                        end_offset: selected.focus.offset,
                        text: replacement.into(),
                    },
                    false,
                )
                .unwrap();
            let next = replaced(&model.text, start, end, replacement);
            if text(&next) != text(&model.text) {
                model.change(next);
            }
        }
        Operation::Delete(from, count) => {
            let start = usize::from(from) % (model.text.len() - 1).max(1);
            // The public API treats count=0 as one character. Once only the
            // anchor remains, exercise Backspace at the start (a true no-op).
            let end = (start + usize::from(count).max(1)).min(model.text.len() - 1);
            let selection = range(&model.text, start, end);
            let backward = start == end;
            let requested = if count == 0 { 0 } else { end - start };
            actual
                .delete_text(0, selection.anchor.offset, backward, requested, 1)
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::DeleteText {
                        paragraph: 0,
                        offset: selection.anchor.offset,
                        direction: if backward {
                            jade_docx_headless_api::HeadlessDeleteDirection::Backward
                        } else {
                            jade_docx_headless_api::HeadlessDeleteDirection::Forward
                        },
                        count: requested,
                    },
                    false,
                )
                .unwrap();
            if start != end {
                let mut next = model.text.clone();
                next.drain(start..end);
                model.change(next);
            }
        }
        Operation::ReplaceAll(query, replacement) => {
            let query_chars: Vec<_> = query.chars().collect();
            let values: Vec<_> = model.text.iter().map(|character| character.value).collect();
            let mut next = Vec::new();
            let mut matches = 0;
            let mut at = 0;
            while at < values.len() {
                if !query_chars.is_empty() && values[at..].starts_with(&query_chars) {
                    let flags = model.text[at].flags;
                    next.extend(replacement.chars().map(|value| Character { value, flags }));
                    matches += 1;
                    at += query_chars.len();
                } else {
                    next.push(model.text[at].clone());
                    at += 1;
                }
            }
            assert_eq!(
                actual.replace_all(query, replacement, 1).unwrap(),
                matches,
                "{context}: matches"
            );
            if matches > 0 {
                reference
                    .apply_commands(
                        vec![HeadlessCommand::ReplacePlaceholder {
                            placeholder: query.into(),
                            text: replacement.into(),
                            max_replacements: None,
                        }],
                        false,
                    )
                    .unwrap();
                model.change(next);
            }
        }
        Operation::FailedBatch => {
            let selection = actual.selection_range();
            let error = actual
                .apply_commands(
                    vec![
                        format_command(range(&model.text, 0, model.text.len()), 3),
                        HeadlessCommand::InsertText {
                            paragraph: 0,
                            offset: 0,
                            text: "日😀".into(),
                        },
                        HeadlessCommand::ReplacePlaceholder {
                            placeholder: "ab".into(),
                            text: "日本".into(),
                            max_replacements: None,
                        },
                        HeadlessCommand::InsertText {
                            paragraph: 1,
                            offset: 0,
                            text: "x".into(),
                        },
                    ],
                    true,
                )
                .unwrap_err();
            assert!(
                error.contains("paragraph 1 does not exist"),
                "{context}: {error}"
            );
            assert_eq!(
                actual.selection_range(),
                selection,
                "{context}: failed batch selection"
            );
            // A failed batch repaints every page. Setting a selection does
            // not, so a highlight an earlier step left on both pages was
            // gone from this side only: repaint the reference too.
            reference.repaint_all().unwrap();
        }
        Operation::Merge(mark) => {
            let mark = usize::from(mark);
            actual.merge_history_since(mark);
            reference.merge_history_since(mark);
            if mark < model.undo.len() {
                model.undo.truncate(mark + 1);
            }
        }
        Operation::Undo => {
            actual.undo().unwrap();
            reference.undo().unwrap();
            if let Some(previous) = model.undo.pop() {
                model
                    .redo
                    .push(std::mem::replace(&mut model.text, previous));
            }
        }
        Operation::Redo => {
            actual.redo().unwrap();
            reference.redo().unwrap();
            if let Some(next) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.text, next));
            }
        }
    }
    model.check(actual, &context);
    model.check(reference, &context);
    assert_same(actual, reference, &context);
}

fn check_sequence(operations: &[Operation]) {
    let mut model = Model {
        text: initial(),
        undo: Vec::new(),
        redo: Vec::new(),
    };
    let mut actual = open();
    let mut reference = open();
    model.check(&actual, "initial mixed state");
    for (step, operation) in operations.iter().enumerate() {
        check_step(&mut model, &mut actual, &mut reference, operation, step);
    }
    let mut step = operations.len();
    while !model.undo.is_empty() {
        check_step(
            &mut model,
            &mut actual,
            &mut reference,
            &Operation::Undo,
            step,
        );
        step += 1;
    }
    assert_eq!(
        model.text,
        initial(),
        "undo restores original characters and styles"
    );
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    assert_page_pixels(&actual, &open(), 0, "mixed undo restores original pixels");
    while !model.redo.is_empty() {
        check_step(
            &mut model,
            &mut actual,
            &mut reference,
            &Operation::Redo,
            step,
        );
        step += 1;
    }
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    let reopened = open_bytes(&actual.export_docx().unwrap());
    model.check_content(&reopened, "mixed export/reopen");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    assert_eq!(actual.page_count(), reopened.page_count());
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "mixed export/reopen pixels");
    }
}

#[test]
fn styled_replacements_and_typing_preserve_surrounding_characters() {
    check_sequence(&[
        Operation::Replace(2, 4, "日😀"),
        Operation::Insert(2, "e\u{301}"),
        Operation::Format(0, 6, 1),
        Operation::ReplaceAll("日😀", "日本"),
        Operation::Delete(1, 2),
        Operation::Format(3, 0, 3),
        Operation::Undo,
        Operation::Redo,
    ]);
}

#[test]
fn failed_mixed_batch_preserves_styles_and_grouped_redo() {
    check_sequence(&[
        Operation::Format(0, 0, 6),
        Operation::Insert(2, "😀"),
        Operation::ReplaceAll("日本", "日"),
        Operation::Merge(0),
        Operation::Undo,
        Operation::FailedBatch,
        Operation::Redo,
        Operation::Undo,
        Operation::Insert(0, ""),
        Operation::Format(1, 2, 2),
        Operation::Redo,
    ]);
}

#[test]
fn failed_batch_after_a_rejected_collapsed_format_matches_the_reference() {
    // Found by CI: the selection highlight the first step painted stayed on
    // both pages after the collapsed format failed, and only the failed batch
    // repainted it away.
    check_sequence(&[
        Operation::Format(0, 0, 1),
        Operation::Format(0, 8, 8),
        Operation::FailedBatch,
    ]);
}

#[test]
fn replacement_after_undo_discards_the_grouped_redo_branch() {
    check_sequence(&[
        Operation::Format(1, 0, 4),
        Operation::Insert(4, "😀"),
        Operation::Merge(0),
        Operation::Undo,
        Operation::ReplaceAll("ab", "日"),
        Operation::Redo,
        Operation::Undo,
        Operation::Redo,
    ]);
}

fn operation() -> impl Strategy<Value = Operation> {
    let replacement = prop::sample::select(vec!["", "a", "日本", "😀", "e\u{301}"]);
    prop_oneof![
        4 => (0u8..4, 0u8..32, 0u8..32).prop_map(|(kind, from, to)| Operation::Format(kind, from, to)),
        3 => (0u8..32, replacement.clone()).prop_map(|(at, text)| Operation::Insert(at, text)),
        3 => (0u8..32, 0u8..32, replacement).prop_map(|(from, to, text)| Operation::Replace(from, to, text)),
        2 => (0u8..32, 0u8..5).prop_map(|(from, count)| Operation::Delete(from, count)),
        3 => prop::sample::select(vec![
            Operation::ReplaceAll("a", "日本"), Operation::ReplaceAll("日本", "😀"),
            Operation::ReplaceAll("😀", "e\u{301}"), Operation::ReplaceAll("e\u{301}", ""),
            Operation::ReplaceAll("ab", "日"), Operation::ReplaceAll("missing", "x"),
            Operation::ReplaceAll("", "x"),
        ]),
        1 => Just(Operation::FailedBatch),
        2 => (0u8..8).prop_map(Operation::Merge),
        3 => Just(Operation::Undo),
        3 => Just(Operation::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/mixed_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn editing_and_formatting_follow_an_independent_character_model(
        operations in prop::collection::vec(operation(), 1..=24)
    ) {
        check_sequence(&operations);
    }
}
