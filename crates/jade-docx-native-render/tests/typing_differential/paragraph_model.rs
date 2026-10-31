//! Paragraph boundaries are newlines in this independent text/history model.

use super::{assert_page_pixels, assert_same, open_bytes, package, Doc};
use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection};
use jade_selection::{SelectionRange, TextPosition};
use proptest::prelude::*;

const INITIAL: &str = "日本😀ab\n\n終わりxyz";

fn open() -> Doc {
    open_bytes(&package(
        "<w:p><w:r><w:t>日本😀ab</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>終わりxyz</w:t></w:r></w:p>",
        "",
    ))
}

fn byte(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(at, _)| at)
}

fn position(text: &str, at: usize) -> TextPosition {
    let prefix = &text[..at];
    TextPosition::new(
        prefix.bytes().filter(|&b| b == b'\n').count(),
        prefix.rsplit('\n').next().unwrap_or_default().len(),
    )
}

#[derive(Debug)]
struct Model {
    text: String,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl Model {
    fn new() -> Self {
        Self {
            text: INITIAL.into(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn change(&mut self, text: String) {
        if self.text != text {
            self.undo.push(std::mem::replace(&mut self.text, text));
            self.redo.clear();
        }
    }

    fn assert_document(&self, document: &Doc, context: &str) {
        let paragraphs: Vec<_> = self.text.split('\n').collect();
        assert_eq!(document.paragraph_count(), paragraphs.len(), "{context}");
        for (index, text) in paragraphs.into_iter().enumerate() {
            assert_eq!(
                document.paragraph_text(index).as_deref(),
                Some(text),
                "{context}: paragraph {index}"
            );
        }
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
            document.renderer().selection_range(),
            document.selection_range(),
            "{context}: renderer selection"
        );
        if let Some(range) = document.selection_range() {
            for point in [range.anchor, range.focus] {
                let text = document
                    .paragraph_text(point.paragraph)
                    .expect("selection paragraph exists");
                assert!(
                    point.offset <= text.len() && text.is_char_boundary(point.offset),
                    "{context}: invalid selection {range:?}"
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Insert(u8, String),
    Split(u8, u8),
    Delete(u8, u8, bool),
    Replace(u8, u8, String, bool),
    FailedBatch,
    Undo,
    Redo,
}

fn operation() -> impl Strategy<Value = Operation> {
    let at = prop_oneof![4 => 0u8..48, 1 => Just(u8::MAX)];
    let text = prop::sample::select(vec![
        "x".to_string(),
        "日本".into(),
        "😀".into(),
        "a b".into(),
    ]);
    prop_oneof![
        4 => (at.clone(), text.clone()).prop_map(|(at, text)| Operation::Insert(at, text)),
        3 => (at.clone(), 0u8..4).prop_map(|(at, count)| Operation::Split(at, count)),
        4 => (at.clone(), 0u8..9, any::<bool>()).prop_map(|(at, count, back)| Operation::Delete(at, count, back)),
        4 => (at.clone(), at, prop_oneof![text, Just(String::new())], any::<bool>()).prop_map(|(a, b, text, reverse)| Operation::Replace(a, b, text, reverse)),
        2 => Just(Operation::FailedBatch),
        3 => Just(Operation::Undo),
        3 => Just(Operation::Redo),
    ]
}

fn check_step(
    model: &mut Model,
    actual: &mut Doc,
    reference: &mut Doc,
    operation: &Operation,
    step: usize,
) {
    let context = format!("step {step}: {operation:?}, before={model:?}");
    let mut next = model.text.clone();
    let mut expected_caret = None;
    match operation {
        Operation::Insert(at, text) => {
            let at = byte(&next, usize::from(*at));
            let point = position(&next, at);
            actual
                .insert_text(point.paragraph, point.offset, text, usize::MAX)
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::InsertText {
                        paragraph: point.paragraph,
                        offset: point.offset,
                        text: text.clone(),
                    },
                    false,
                )
                .unwrap();
            next.insert_str(at, text);
            expected_caret = Some(position(&next, at + text.len()));
        }
        Operation::Split(at, count) => {
            let at = byte(&next, usize::from(*at));
            let point = position(&next, at);
            let command = HeadlessCommand::InsertParagraphBreak {
                paragraph: point.paragraph,
                offset: point.offset,
                count: usize::from(*count),
            };
            if *count == 1 {
                actual
                    .insert_paragraph_break(point.paragraph, point.offset)
                    .unwrap();
            } else {
                actual.apply_command(command.clone(), false).unwrap();
            }
            reference.apply_command(command, false).unwrap();
            let count = usize::from(*count).max(1);
            next.insert_str(at, &"\n".repeat(count));
            expected_caret = Some(position(&next, at + count));
        }
        Operation::Delete(at, count, backwards) => {
            let caret = usize::from(*at).min(next.chars().count());
            let at = byte(&next, caret);
            let point = position(&next, at);
            let count = usize::from(*count);
            actual
                .delete_text(point.paragraph, point.offset, *backwards, count, usize::MAX)
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::DeleteText {
                        paragraph: point.paragraph,
                        offset: point.offset,
                        count,
                        direction: if *backwards {
                            HeadlessDeleteDirection::Backward
                        } else {
                            HeadlessDeleteDirection::Forward
                        },
                    },
                    false,
                )
                .unwrap();
            let (from, to) = if *backwards {
                (caret.saturating_sub(count.max(1)), caret)
            } else {
                (caret, caret.saturating_add(count.max(1)))
            };
            let start = byte(&next, from);
            next.replace_range(start..byte(&model.text, to), "");
            expected_caret = Some(position(&next, start));
        }
        Operation::Replace(a, b, text, reverse) => {
            let a = byte(&next, usize::from(*a));
            let b = byte(&next, usize::from(*b));
            let (start, end) = (a.min(b), a.max(b));
            let from = position(&next, start);
            let to = position(&next, end);
            let range = if *reverse {
                SelectionRange::new(to, from)
            } else {
                SelectionRange::new(from, to)
            };
            actual.set_selection_range(Some(range));
            reference.set_selection_range(Some(range));
            if start == end {
                let error = actual.replace_selection(text, usize::MAX).unwrap_err();
                assert_eq!(error, "no selection to replace", "{context}");
                assert!(reference.replace_selection(text, usize::MAX).is_err());
                expected_caret = Some(from);
                // The setter does not paint. Compare both sessions after the
                // same repaint, including the rejection's unchanged selection.
                actual.repaint_all().unwrap();
                reference.repaint_all().unwrap();
            } else {
                assert_eq!(
                    actual.selected_text().as_deref(),
                    Some(&next[start..end]),
                    "{context}: clipboard"
                );
                let caret = actual.replace_selection(text, usize::MAX).unwrap();
                reference
                    .apply_command(
                        HeadlessCommand::ReplaceRange {
                            start_paragraph: from.paragraph,
                            start_offset: from.offset,
                            end_paragraph: to.paragraph,
                            end_offset: to.offset,
                            text: text.clone(),
                        },
                        false,
                    )
                    .unwrap();
                next.replace_range(start..end, text);
                let expected = position(&next, start + text.len());
                assert_eq!(
                    caret,
                    (expected.paragraph, expected.offset),
                    "{context}: returned caret"
                );
                expected_caret = Some(expected);
            }
        }
        Operation::FailedBatch => {
            let selection = actual.selection_range();
            let commands = vec![
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 0,
                    count: 2,
                },
                HeadlessCommand::InsertText {
                    paragraph: next.split('\n').count() + 2,
                    offset: 0,
                    text: "invalid".into(),
                },
            ];
            for document in [&mut *actual, &mut *reference] {
                let error = document
                    .apply_commands(commands.clone(), false)
                    .unwrap_err();
                assert!(error.contains("does not exist"), "{context}: {error}");
                assert_eq!(
                    document.selection_range(),
                    selection,
                    "{context}: rolled back selection"
                );
            }
        }
        Operation::Undo => {
            let expected = !model.undo.is_empty();
            assert_eq!(actual.undo().unwrap().is_some(), expected, "{context}");
            assert_eq!(reference.undo().unwrap().is_some(), expected, "{context}");
            if let Some(text) = model.undo.pop() {
                model.redo.push(std::mem::replace(&mut model.text, text));
            }
            next = model.text.clone();
        }
        Operation::Redo => {
            let expected = !model.redo.is_empty();
            assert_eq!(actual.redo().unwrap().is_some(), expected, "{context}");
            assert_eq!(reference.redo().unwrap().is_some(), expected, "{context}");
            if let Some(text) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.text, text));
            }
            next = model.text.clone();
        }
    }
    model.change(next);
    if let Some(caret) = expected_caret {
        assert_eq!(
            actual.selection_range(),
            Some(SelectionRange::collapsed(caret)),
            "{context}: caret"
        );
    }
    model.assert_document(actual, &context);
    model.assert_document(reference, &format!("{context}: full repaint"));
    assert_same(actual, reference, &context);
}

fn check_sequence(operations: &[Operation]) {
    let mut actual = open();
    let mut reference = open();
    let mut model = Model::new();
    for (step, operation) in operations.iter().enumerate() {
        check_step(&mut model, &mut actual, &mut reference, operation, step);
    }
    for step in 0..=model.undo.len() {
        check_step(
            &mut model,
            &mut actual,
            &mut reference,
            &Operation::Undo,
            step,
        );
    }
    for step in 0..=model.redo.len() {
        check_step(
            &mut model,
            &mut actual,
            &mut reference,
            &Operation::Redo,
            step,
        );
    }
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    let reopened = open_bytes(&actual.export_docx().unwrap());
    assert_eq!(reopened.paragraph_count(), model.text.split('\n').count());
    for (index, text) in model.text.split('\n').enumerate() {
        assert_eq!(reopened.paragraph_text(index).as_deref(), Some(text));
    }
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    assert_eq!(actual.page_count(), reopened.page_count());
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "paragraph sequence export/reopen");
    }
}

#[test]
fn cross_paragraph_replacement_supports_both_selection_directions() {
    for reverse in [false, true] {
        for replacement in ["", "日本", "x"] {
            check_sequence(&[
                Operation::Replace(2, 9, replacement.into(), reverse),
                Operation::Undo,
                Operation::Redo,
                Operation::Insert(255, "tail".into()),
            ]);
        }
    }
}

#[test]
fn counted_deletion_crosses_empty_and_unicode_paragraphs() {
    for backwards in [false, true] {
        for count in [0, 1, 3, 8, 30] {
            check_sequence(&[Operation::Delete(
                if backwards { 9 } else { 3 },
                count,
                backwards,
            )]);
        }
    }
}

#[test]
fn failed_structural_batch_preserves_the_redo_branch() {
    check_sequence(&[
        Operation::Split(2, 3),
        Operation::Undo,
        Operation::FailedBatch,
        Operation::Redo,
    ]);
}

#[test]
fn bounded_cross_page_replacement_updates_page_count_and_history() {
    let bytes = package(
        r#"<w:p><w:r><w:t>head日本</w:t></w:r></w:p>
        <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>middle</w:t></w:r></w:p>
        <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>tail😀</w:t></w:r></w:p>"#,
        "",
    );
    for reverse in [false, true] {
        let mut actual = open_bytes(&bytes);
        let mut reference = open_bytes(&bytes);
        assert_eq!(actual.page_count(), 3);
        let from = TextPosition::new(0, 4);
        let to = TextPosition::new(2, 4);
        let selection = if reverse {
            SelectionRange::new(to, from)
        } else {
            SelectionRange::new(from, to)
        };
        actual.set_selection_range(Some(selection));
        reference.set_selection_range(Some(selection));
        assert_eq!(actual.replace_selection("x", 1).unwrap(), (0, 5));
        reference
            .apply_command(
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: 4,
                    end_paragraph: 2,
                    end_offset: 4,
                    text: "x".into(),
                },
                false,
            )
            .unwrap();
        assert_eq!(actual.paragraph_text(0).as_deref(), Some("headx😀"));
        assert_eq!(
            actual.page_count(),
            1,
            "completed bounded reflow must remove deleted pages"
        );
        assert_same(&mut actual, &mut reference, "cross-page replacement");
        actual.undo().unwrap();
        reference.undo().unwrap();
        assert_eq!(actual.page_count(), 3);
        assert_same(&mut actual, &mut reference, "cross-page replacement undo");
        actual.redo().unwrap();
        reference.redo().unwrap();
        assert_eq!(actual.page_count(), 1);
        assert_same(&mut actual, &mut reference, "cross-page replacement redo");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/paragraph_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn paragraph_edits_follow_an_independent_model(operations in prop::collection::vec(operation(), 1..=24)) {
        check_sequence(&operations);
    }
}
