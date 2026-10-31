//! Keep partial repaints pending across edits, rather than completing layout
//! after every operation. Only pages within the requested bound are current.

use super::{assert_page_pixels, assert_same, open_bytes, package, rich_body, Doc};
use jade_docx_headless_api::HeadlessCommand;
use proptest::prelude::*;

fn initial() -> Vec<String> {
    [
        "first 日本語 alpha",
        "bold plain e\u{301} 😀",
        "first link",
        "",
        "second page",
        "",
        "cell 日本語",
        "second link",
        "third page",
        "last 日本語 omega",
    ]
    .map(String::from)
    .into()
}

#[derive(Debug)]
struct Model {
    text: Vec<String>,
    undo: Vec<Vec<String>>,
    redo: Vec<Vec<String>>,
}

impl Model {
    fn change(&mut self, next: Vec<String>) {
        if self.text != next {
            self.undo.push(std::mem::replace(&mut self.text, next));
            self.redo.clear();
        }
    }

    fn check(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), self.text.len(), "{context}");
        for (paragraph, expected) in self.text.iter().enumerate() {
            assert_eq!(
                document.paragraph_text(paragraph).as_ref(),
                Some(expected),
                "{context}: paragraph {paragraph}"
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
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Insert(u8, u8, String, u8),
    Replace(u8, u8, u8, String, u8),
    RejectedInsert,
    Merge(u8),
    Undo,
    Redo,
    CompleteLayout,
}

fn operation() -> impl Strategy<Value = Operation> {
    let paragraph = 0u8..10;
    let at = prop_oneof![3 => 0u8..32, 1 => Just(u8::MAX)];
    let text = prop::sample::select(vec![
        String::new(),
        "日".into(),
        "😀e\u{301}".into(),
        "\tZ".into(),
        "折り返し 日本語 ab ".repeat(48),
    ]);
    let bound = 1u8..=3;
    prop_oneof![
        6 => (paragraph.clone(), at.clone(), text.clone(), bound.clone())
            .prop_map(|(p, at, text, bound)| Operation::Insert(p, at, text, bound)),
        6 => (paragraph, at.clone(), at, text, bound)
            .prop_map(|(p, a, b, text, bound)| Operation::Replace(p, a, b, text, bound)),
        2 => Just(Operation::RejectedInsert),
        1 => (0u8..8).prop_map(Operation::Merge),
        2 => Just(Operation::Undo),
        2 => Just(Operation::Redo),
        1 => Just(Operation::CompleteLayout),
    ]
}

fn byte(text: &str, character: u8) -> usize {
    if character == u8::MAX {
        return text.len();
    }
    text.char_indices()
        .nth(usize::from(character))
        .map_or(text.len(), |(offset, _)| offset)
}

fn check_visible(actual: &Doc, reference: &Doc, bound: usize, context: &str) {
    // Offscreen surfaces deliberately may still contain the previous layout.
    // Comparing them here would accidentally require a full-document repaint.
    for page in 0..bound.min(reference.page_count()) {
        assert_page_pixels(actual, reference, page, context);
    }
    assert_eq!(
        actual.selection_range(),
        reference.selection_range(),
        "{context}"
    );
}

fn check_step(
    model: &mut Model,
    actual: &mut Doc,
    reference: &mut Doc,
    operation: &Operation,
    step: usize,
) {
    let context = format!(
        "step {step}: {operation:?}, lengths={:?}, undo={}, redo={}",
        model.text.iter().map(String::len).collect::<Vec<_>>(),
        model.undo.len(),
        model.redo.len(),
    );
    match operation {
        Operation::Insert(p, at, text, bound) => {
            let paragraph = usize::from(*p);
            let offset = byte(&model.text[paragraph], *at);
            let mut next = model.text.clone();
            next[paragraph].insert_str(offset, text);
            actual
                .insert_text(paragraph, offset, text, usize::from(*bound))
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::InsertText {
                        paragraph,
                        offset,
                        text: text.clone(),
                    },
                    false,
                )
                .unwrap();
            model.change(next);
            check_visible(actual, reference, usize::from(*bound), &context);
        }
        Operation::Replace(p, a, b, text, bound) => {
            let paragraph = usize::from(*p);
            let a = byte(&model.text[paragraph], *a);
            let b = byte(&model.text[paragraph], *b);
            let (start, end) = (a.min(b), a.max(b));
            let mut next = model.text.clone();
            next[paragraph].replace_range(start..end, text);
            actual
                .replace_range(paragraph, start, end, text, usize::from(*bound))
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: paragraph,
                        start_offset: start,
                        end_paragraph: paragraph,
                        end_offset: end,
                        text: text.clone(),
                    },
                    false,
                )
                .unwrap();
            model.change(next);
            check_visible(actual, reference, usize::from(*bound), &context);
        }
        Operation::RejectedInsert => {
            let before = actual.selection_range();
            let pixels: Vec<_> = (0..actual.page_count())
                .map(|p| actual.page_pixels(p))
                .collect();
            let error = actual
                .insert_text(model.text.len(), 0, "日本😀", 1)
                .unwrap_err();
            assert!(
                error.contains("paragraph 10 does not exist"),
                "{context}: {error}"
            );
            assert_eq!(
                actual.selection_range(),
                before,
                "{context}: rejected selection"
            );
            assert_eq!(
                actual.page_count(),
                pixels.len(),
                "{context}: rejected page count"
            );
            for (page, before) in pixels.into_iter().enumerate() {
                assert!(
                    actual.page_pixels(page) == before,
                    "{context}: rejected edit changed page {page}"
                );
            }
        }
        Operation::Merge(mark) => {
            let mark = usize::from(*mark);
            actual.merge_history_since(mark);
            reference.merge_history_since(mark);
            if mark < model.undo.len() {
                model.undo.truncate(mark + 1);
            }
        }
        Operation::Undo => {
            actual.undo().unwrap();
            reference.undo().unwrap();
            // An unavailable history step does not repaint pending pages.
            if let Some(previous) = model.undo.pop() {
                model
                    .redo
                    .push(std::mem::replace(&mut model.text, previous));
                assert_same(actual, reference, &context);
            }
        }
        Operation::Redo => {
            actual.redo().unwrap();
            reference.redo().unwrap();
            if let Some(next) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.text, next));
                assert_same(actual, reference, &context);
            }
        }
        Operation::CompleteLayout => {
            actual.repaint_all().unwrap();
            assert_same(actual, reference, &context);
        }
    }
    model.check(actual, &context);
    model.check(reference, &context);
    assert_eq!(
        actual.selection_range(),
        reference.selection_range(),
        "{context}: selection"
    );
}

fn check_sequence(operations: &[Operation]) {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    let initial_pixels: Vec<_> = (0..actual.page_count())
        .map(|page| actual.page_pixels(page))
        .collect();
    let mut model = Model {
        text: initial(),
        undo: Vec::new(),
        redo: Vec::new(),
    };
    model.check(&actual, "initial viewport state");
    for (step, operation) in operations.iter().enumerate() {
        check_step(&mut model, &mut actual, &mut reference, operation, step);
    }
    // Export while downstream pages may still have pending layout. Opening
    // those bytes gives a renderer with no access to either session's caches.
    let mut reopened = open_bytes(&actual.export_docx().unwrap());
    for document in [&mut actual, &mut reference, &mut reopened] {
        document.set_selection_range(None);
        document.repaint_all().unwrap();
    }
    assert_same(&mut actual, &mut reference, "complete deferred layout");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    assert_eq!(reopened.paragraph_count(), model.text.len());
    for (paragraph, text) in model.text.iter().enumerate() {
        assert_eq!(reopened.paragraph_text(paragraph).as_ref(), Some(text));
    }
    assert_eq!(actual.page_count(), reopened.page_count());
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "deferred layout export/reopen");
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
        "undo restores all original paragraphs"
    );
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    assert_eq!(
        actual.page_count(),
        initial_pixels.len(),
        "undo restores original pagination"
    );
    for (page, pixels) in initial_pixels.into_iter().enumerate() {
        assert!(
            actual.page_pixels(page) == pixels,
            "undo restores original page {page}"
        );
    }
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
}

#[test]
fn deferred_reflow_survives_edits_on_both_sides_of_the_viewport() {
    check_sequence(&[
        Operation::Insert(0, 0, "長い折り返し日本語 ".repeat(72), 1),
        Operation::Insert(9, 0, "😀".into(), 1),
        Operation::Replace(1, 0, u8::MAX, "短い".into(), 2),
        Operation::RejectedInsert,
        Operation::Replace(0, 0, u8::MAX, "戻す".into(), 1),
        Operation::Insert(6, 0, "セル".into(), 2),
        Operation::CompleteLayout,
    ]);
}

#[test]
fn grouped_partial_edits_preserve_redo_after_rejection_and_empty_typing() {
    check_sequence(&[
        Operation::Insert(2, 0, "日本".into(), 1),
        Operation::Insert(6, 0, "e\u{301}😀".into(), 2),
        Operation::Insert(9, 0, "後方".into(), 1),
        Operation::Merge(0),
        Operation::Undo,
        Operation::RejectedInsert,
        Operation::Insert(5, 0, String::new(), 2),
        Operation::Redo,
        Operation::Insert(0, 0, "先頭".into(), 1),
        Operation::CompleteLayout,
    ]);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/viewport_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn partial_repaints_follow_an_independent_history_model(operations in prop::collection::vec(operation(), 1..=20)) {
        check_sequence(&operations);
    }
}
