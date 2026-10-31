//! A text/history oracle independent of the editor and its command executor.

use super::{assert_page_pixels, assert_same, open_bytes, package, Doc};
use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection};
use jade_selection::{SelectionRange, TextPosition};
use proptest::prelude::*;

const INITIAL: &str = "日本😀abc";

fn open() -> Doc {
    open_bytes(&package(
        &format!("<w:p><w:r><w:t>{INITIAL}</w:t></w:r></w:p>"),
        "",
    ))
}

fn insert(paragraph: usize, offset: usize, text: &str) -> HeadlessCommand {
    HeadlessCommand::InsertText {
        paragraph,
        offset,
        text: text.into(),
    }
}

fn edit(document: &mut Doc, command: HeadlessCommand, full: bool) -> Result<(), String> {
    if full {
        return document.apply_command(command, false);
    }
    match command {
        HeadlessCommand::InsertText {
            paragraph,
            offset,
            text,
        } => document.insert_text(paragraph, offset, &text, 1),
        HeadlessCommand::DeleteText {
            paragraph,
            offset,
            direction,
            count,
        } => document.delete_text(
            paragraph,
            offset,
            matches!(direction, HeadlessDeleteDirection::Backward),
            count,
            1,
        ),
        HeadlessCommand::ReplaceRange {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            text,
        } => {
            assert_eq!(start_paragraph, end_paragraph);
            document.replace_range(start_paragraph, start_offset, end_offset, &text, 1)
        }
        _ => unreachable!("the model only generates text edits"),
    }
    .map(|_| ())
}

#[test]
fn rejected_edits_preserve_selection_and_history() {
    for full in [false, true] {
        for (paragraph, offset, message) in [
            (1, 0, "paragraph 1 does not exist"),
            (0, INITIAL.len() + 1, "past the end"),
            (0, 1, "inside a character"),
        ] {
            let mut document = open();
            let selection = Some(SelectionRange::new(
                TextPosition::new(0, 0),
                TextPosition::new(0, 6),
            ));
            document.set_selection_range(selection);
            let before = document.page_pixels(0).unwrap().0;
            let error = edit(&mut document, insert(paragraph, offset, "x"), full).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert_eq!(document.paragraph_text(0).as_deref(), Some(INITIAL));
            assert_eq!(
                document.selection_range(),
                selection,
                "full={full}: {error}"
            );
            assert_eq!(document.history_len(), 0, "full={full}: {error}");
            assert!(!document.can_undo());
            assert!(!document.can_redo());
            assert_eq!(document.page_pixels(0).unwrap().0, before);
        }
    }
}

#[test]
fn no_op_edits_do_not_create_history() {
    for full in [false, true] {
        for command in [
            insert(0, 0, ""),
            HeadlessCommand::DeleteText {
                paragraph: 0,
                offset: 0,
                direction: HeadlessDeleteDirection::Backward,
                count: 3,
            },
            HeadlessCommand::DeleteText {
                paragraph: 0,
                offset: INITIAL.len(),
                direction: HeadlessDeleteDirection::Forward,
                count: 0,
            },
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: INITIAL.len(),
                text: INITIAL.into(),
            },
        ] {
            let mut document = open();
            edit(&mut document, command.clone(), full).unwrap();
            assert_eq!(document.paragraph_text(0).as_deref(), Some(INITIAL));
            assert_eq!(document.history_len(), 0, "full={full}: {command:?}");
            assert!(!document.can_undo());
            assert!(document.undo().unwrap().is_none());
        }
    }
}

#[test]
fn empty_insertions_preserve_grouped_redo_and_still_validate_offsets() {
    for full in [false, true] {
        let mut actual = open();
        let mut reference = open();
        let mut model = Model::new();
        for (step, operation) in [
            Operation::Insert(0, "日本".into(), full),
            Operation::Insert(0, "😀".into(), full),
            Operation::Merge(0),
            Operation::Undo,
            Operation::Insert(1, String::new(), full),
        ]
        .iter()
        .enumerate()
        {
            check_step(&mut model, &mut actual, &mut reference, operation, step);
        }
        let selection = actual.selection_range();
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, "日".len())))
        );
        for (paragraph, offset, message) in [
            (1, 0, "does not exist"),
            (0, INITIAL.len() + 1, "past the end"),
            (0, 1, "inside a character"),
        ] {
            let command = insert(paragraph, offset, "");
            let error = edit(&mut actual, command.clone(), full).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(reference.apply_command(command, false).is_err());
            assert_eq!(actual.selection_range(), selection);
            model.assert_document(&actual, "rejected empty insertion");
            assert_same(&mut actual, &mut reference, "rejected empty insertion");
        }
        check_step(&mut model, &mut actual, &mut reference, &Operation::Redo, 5);
        assert_eq!(model.text, format!("😀日本{INITIAL}"));
        check_step(&mut model, &mut actual, &mut reference, &Operation::Undo, 6);
    }
}

#[test]
fn rejected_and_no_op_edits_preserve_grouped_redo() {
    for full in [false, true] {
        for no_op in [false, true] {
            let mut document = open();
            edit(&mut document, insert(0, 0, "one"), full).unwrap();
            edit(&mut document, insert(0, 3, "two"), full).unwrap();
            document.merge_history_since(0);
            document.undo().unwrap();
            assert_eq!(document.paragraph_text(0).as_deref(), Some(INITIAL));
            if no_op {
                edit(
                    &mut document,
                    HeadlessCommand::DeleteText {
                        paragraph: 0,
                        offset: 0,
                        direction: HeadlessDeleteDirection::Backward,
                        count: 1,
                    },
                    full,
                )
                .unwrap();
            } else {
                assert!(edit(&mut document, insert(1, 0, "invalid"), full).is_err());
            }
            assert_eq!(document.history_len(), 0);
            assert!(document.can_redo());
            document.redo().unwrap();
            assert_eq!(
                document.paragraph_text(0).unwrap(),
                format!("onetwo{INITIAL}"),
                "full={full}, no_op={no_op}: redo must restore the entire user action"
            );
            assert!(!document.can_redo());
            document.undo().unwrap();
            assert_eq!(document.paragraph_text(0).as_deref(), Some(INITIAL));
            assert!(!document.can_undo());
        }
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Insert(u8, String, bool),
    Delete(u8, u8, bool, bool),
    Replace(u8, u8, String, bool),
    NoOp(bool),
    Invalid(u8, bool),
    Batch(String),
    FailedBatch,
    Merge(u8),
    Undo,
    Redo,
}

#[test]
fn renderer_selection_tracks_edits_and_history() {
    let mut actual = open();
    let mut reference = open();
    let mut model = Model::new();
    for (step, operation) in [
        Operation::Insert(3, "日本".into(), false),
        Operation::Insert(u8::MAX, "😀".into(), true),
        Operation::Delete(u8::MAX, 3, true, false),
        Operation::Undo,
        Operation::Insert(0, "x".into(), false),
        Operation::Replace(0, 2, "y".into(), false),
        Operation::Undo,
        Operation::Insert(0, "branch".into(), false),
        Operation::Undo,
        Operation::Redo,
    ]
    .iter()
    .enumerate()
    {
        check_step(&mut model, &mut actual, &mut reference, operation, step);
    }
}

#[test]
fn renderer_selection_survives_page_box_rebuild_and_history() {
    let mut document = open();
    let selection = Some(SelectionRange::new(
        TextPosition::new(0, 0),
        TextPosition::new(0, 6),
    ));
    document.set_selection_range(selection);
    let original_size = document.layout_page_size();
    document
        .apply_command(
            HeadlessCommand::SetPageSize {
                portrait_width: 500.0,
                portrait_height: 700.0,
            },
            true,
        )
        .unwrap();
    assert_ne!(document.layout_page_size(), original_size);
    for history in [None, Some(false), Some(true)] {
        match history {
            Some(false) => {
                document.undo().unwrap();
                assert_eq!(document.layout_page_size(), original_size);
            }
            Some(true) => {
                document.redo().unwrap();
                assert_ne!(document.layout_page_size(), original_size);
            }
            None => {}
        }
        assert_eq!(document.selection_range(), selection);
        assert_eq!(document.renderer().selection_range(), selection);
        assert_eq!(
            document.renderer().selection_text().as_deref(),
            Some("日本")
        );
        let mut reopened = open_bytes(&document.export_docx().unwrap());
        reopened.set_selection_range(selection);
        reopened.repaint_all().unwrap();
        assert_eq!(document.page_count(), reopened.page_count());
        for page in 0..document.page_count() {
            assert_page_pixels(&document, &reopened, page, "page box selection");
        }
    }
}

fn operation() -> impl Strategy<Value = Operation> {
    let text = prop::sample::select(vec![
        "",
        "x",
        "日本",
        "😀",
        "e\u{301}",
        "👩\u{200d}💻",
        "a b",
        "XYZ",
    ])
    .prop_map(str::to_owned);
    let position = prop_oneof![5 => 0u8..16, 1 => Just(u8::MAX)];
    prop_oneof![
        5 => (position.clone(), text.clone(), any::<bool>())
            .prop_map(|(at, text, full)| Operation::Insert(at, text, full)),
        4 => (position.clone(), 0u8..5, any::<bool>(), any::<bool>())
            .prop_map(|(at, count, backwards, full)| Operation::Delete(at, count, backwards, full)),
        3 => (position.clone(), position, text.clone(), any::<bool>())
            .prop_map(|(a, b, text, full)| Operation::Replace(a, b, text, full)),
        2 => any::<bool>().prop_map(Operation::NoOp),
        2 => (0u8..3, any::<bool>()).prop_map(|(kind, full)| Operation::Invalid(kind, full)),
        2 => text.prop_map(Operation::Batch),
        2 => Just(Operation::FailedBatch),
        3 => prop_oneof![0u8..8, Just(u8::MAX)].prop_map(Operation::Merge),
        4 => Just(Operation::Undo),
        4 => Just(Operation::Redo),
    ]
}

// Snapshots of just the observable text, with no editor/command objects.
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
        if text != self.text {
            self.undo.push(std::mem::replace(&mut self.text, text));
            self.redo.clear();
        }
    }

    fn undo(&mut self) {
        if let Some(text) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.text, text));
        }
    }

    fn redo(&mut self) {
        if let Some(text) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.text, text));
        }
    }

    fn merge(&mut self, mark: usize) {
        // A group restores the snapshot before its first edit, including
        // when its edits cancel each other and leave the text unchanged.
        if mark < self.undo.len() {
            self.undo.truncate(mark + 1);
        }
    }

    fn assert_document(&self, document: &Doc, context: &str) {
        assert_eq!(
            document.renderer().selection_range(),
            document.selection_range(),
            "{context}: renderer selection"
        );
        assert_eq!(document.paragraph_count(), 1, "{context}");
        assert_eq!(
            document.paragraph_text(0).as_deref(),
            Some(self.text.as_str()),
            "{context}"
        );
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
    }
}

// Generated positions are character counts, clamped at the end. This stays
// total when shrinking removes earlier insertions or introduces more undos.
fn offset(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(at, _)| at)
}

fn check_step(
    model: &mut Model,
    actual: &mut Doc,
    reference: &mut Doc,
    op: &Operation,
    step: usize,
) {
    let context = format!("step {step}: {op:?}, before={model:?}");
    let before_selection = actual.selection_range();
    let mut next = model.text.clone();
    let command = match op {
        Operation::Insert(at, text, full) => {
            let at = offset(&next, usize::from(*at));
            next.insert_str(at, text);
            Some((insert(0, at, text), *full))
        }
        Operation::Delete(at, count, backwards, full) => {
            let caret = usize::from(*at).min(next.chars().count());
            let requested_count = usize::from(*count);
            let count = requested_count.max(1);
            let (start, end) = if *backwards {
                (caret.saturating_sub(count), caret)
            } else {
                (caret, caret.saturating_add(count))
            };
            let at = offset(&next, caret);
            next.replace_range(offset(&model.text, start)..offset(&model.text, end), "");
            Some((
                HeadlessCommand::DeleteText {
                    paragraph: 0,
                    offset: at,
                    count: requested_count,
                    direction: if *backwards {
                        HeadlessDeleteDirection::Backward
                    } else {
                        HeadlessDeleteDirection::Forward
                    },
                },
                *full,
            ))
        }
        Operation::Replace(a, b, text, full) => {
            let a = offset(&next, usize::from(*a));
            let b = offset(&next, usize::from(*b));
            let (start, end) = (a.min(b), a.max(b));
            next.replace_range(start..end, text);
            Some((
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: start,
                    end_paragraph: 0,
                    end_offset: end,
                    text: text.clone(),
                },
                *full,
            ))
        }
        Operation::NoOp(full) => Some((
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: next.len(),
                text: next.clone(),
            },
            *full,
        )),
        Operation::Invalid(kind, full) => {
            let (paragraph, at, message) = match kind {
                0 => (1, 0, "paragraph 1 does not exist"),
                1 => (0, next.len() + 1, "past the end"),
                _ => match next.char_indices().find(|(_, c)| c.len_utf8() > 1) {
                    Some((at, _)) => (0, at + 1, "inside a character"),
                    None => (0, next.len() + 1, "past the end"),
                },
            };
            let error = edit(actual, insert(paragraph, at, "x"), *full).unwrap_err();
            assert!(error.contains(message), "{context}: {error}");
            assert!(reference
                .apply_command(insert(paragraph, at, "x"), false)
                .is_err());
            assert_eq!(
                actual.selection_range(),
                before_selection,
                "{context}: rejected selection"
            );
            None
        }
        Operation::Batch(text) => {
            let commands = vec![insert(0, 0, text), insert(0, text.len(), "z")];
            actual.apply_commands(commands.clone(), false).unwrap();
            reference.apply_commands(commands, false).unwrap();
            next.insert_str(0, &format!("{text}z"));
            None
        }
        Operation::FailedBatch => {
            let commands = vec![insert(0, 0, "temporary"), insert(1, 0, "invalid")];
            assert!(
                actual.apply_commands(commands.clone(), false).is_err(),
                "{context}"
            );
            assert!(
                reference.apply_commands(commands, false).is_err(),
                "{context}"
            );
            assert_eq!(
                actual.selection_range(),
                before_selection,
                "{context}: rolled-back selection"
            );
            None
        }
        Operation::Undo => {
            actual.undo().unwrap();
            reference.undo().unwrap();
            model.undo();
            next = model.text.clone();
            None
        }
        Operation::Merge(mark) => {
            let mark = if *mark == u8::MAX {
                model.undo.len()
            } else {
                usize::from(*mark)
            };
            actual.merge_history_since(mark);
            reference.merge_history_since(mark);
            model.merge(mark);
            None
        }
        Operation::Redo => {
            actual.redo().unwrap();
            reference.redo().unwrap();
            model.redo();
            next = model.text.clone();
            None
        }
    };
    if let Some((command, full)) = command {
        edit(actual, command.clone(), full).unwrap();
        reference.apply_command(command, false).unwrap();
    }
    model.change(next);
    model.assert_document(actual, &context);
    model.assert_document(reference, &format!("{context}: full repaint"));
    assert_same(actual, reference, &context);
}

#[test]
fn nested_history_groups_preserve_cancelled_composition_and_redo_boundaries() {
    let mut actual = open();
    let mut reference = open();
    let mut model = Model::new();
    for (step, operation) in [
        Operation::Insert(0, "prefix".into(), false),
        Operation::Insert(0, "に".into(), false),
        Operation::Replace(0, 1, "にほん😀".into(), false),
        Operation::Merge(1),
        Operation::Replace(0, 4, String::new(), false),
        Operation::Merge(1),
        Operation::Undo,
        Operation::Merge(0),
        Operation::Redo,
        Operation::Undo,
        Operation::Invalid(2, false),
        Operation::NoOp(false),
        Operation::FailedBatch,
        Operation::Redo,
        Operation::Merge(0),
        Operation::Undo,
        Operation::Redo,
        Operation::Undo,
        Operation::Insert(0, "branch".into(), false),
        Operation::Redo,
        Operation::Merge(u8::MAX),
        Operation::Undo,
    ]
    .iter()
    .enumerate()
    {
        check_step(&mut model, &mut actual, &mut reference, operation, step);
    }
    assert_eq!(model.text, INITIAL);
    assert!(!actual.can_undo());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/history_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn text_and_history_follow_an_independent_model(operations in prop::collection::vec(operation(), 1..=32)) {
        let mut actual = open();
        let mut reference = open();
        let mut model = Model::new();
        for (step, operation) in operations.iter().enumerate() {
            check_step(&mut model, &mut actual, &mut reference, operation, step);
        }
        // Exercise exhaustion as well as a branch after redo, regardless of
        // the random mix. Check every intermediate document, not only the end.
        let steps = model.undo.len();
        for step in 0..=steps {
            check_step(&mut model, &mut actual, &mut reference, &Operation::Undo, step);
        }
        let steps = model.redo.len();
        for step in 0..=steps {
            check_step(&mut model, &mut actual, &mut reference, &Operation::Redo, step);
        }
        check_step(&mut model, &mut actual, &mut reference, &Operation::Undo, 0);
        check_step(&mut model, &mut actual, &mut reference, &Operation::Insert(0, "branch".into(), false), 1);
        check_step(&mut model, &mut actual, &mut reference, &Operation::Redo, 2);

        // Persistence boundary: compare text and pixels with a new DOCX open,
        // whose layout caches and history have never seen the edit sequence.
        actual.set_selection_range(None);
        actual.repaint_all().unwrap();
        let reopened = open_bytes(&actual.export_docx().unwrap());
        assert_eq!(reopened.paragraph_text(0).as_deref(), Some(model.text.as_str()));
        assert!(!reopened.can_undo());
        assert!(!reopened.can_redo());
        assert_eq!(actual.page_count(), reopened.page_count());
        for page in 0..actual.page_count() {
            assert_page_pixels(&actual, &reopened, page, "model sequence export/reopen");
        }
    }
}
