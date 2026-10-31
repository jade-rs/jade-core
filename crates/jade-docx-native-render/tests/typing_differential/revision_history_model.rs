//! Revision decisions, view changes, and text edits share one undo history.

use super::{assert_page_pixels, open_bytes, package, Doc, COORD_EPSILON};
use jade_docx_headless_api::{HeadlessCommand, HeadlessRevisionKindFilter};
use proptest::prelude::*;

const CHANGES: [(usize, bool, &str); 4] = [
    (0, true, "追記 日本語"),
    (0, false, "削除 😀"),
    (1, true, "insert e\u{301}"),
    (1, false, "deleted wrapping words 日本語 "),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pending,
    Plain,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    changes: [Status; 4],
    tail: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            changes: [Status::Pending; 4],
            tail: "本文 日本語😀e\u{301}".into(),
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn run(text: &str, deleted: bool) -> String {
    let tag = if deleted { "delText" } else { "t" };
    format!(
        "<w:r><w:{tag} xml:space=\"preserve\">{}</w:{tag}></w:r>",
        escape(text)
    )
}

impl State {
    fn text(&self, paragraph: usize) -> String {
        if paragraph == 2 {
            return self.tail.clone();
        }
        let mut text = format!("start {paragraph} ");
        for (index, (owner, _, value)) in CHANGES.iter().enumerate() {
            if *owner == paragraph {
                if self.changes[index] != Status::Removed {
                    text.push_str(value);
                }
                text.push_str(" middle ");
            }
        }
        text.push_str("end");
        text
    }

    fn package(&self) -> Vec<u8> {
        let mut body = String::new();
        for paragraph in 0..3 {
            body.push_str("<w:p>");
            if paragraph > 0 {
                body.push_str("<w:pPr><w:pageBreakBefore/></w:pPr>");
            }
            if paragraph == 2 {
                body.push_str(&run(&self.tail, false));
            } else {
                body.push_str(&run(&format!("start {paragraph} "), false));
                for (index, (owner, inserted, text)) in CHANGES.iter().enumerate() {
                    if *owner != paragraph {
                        continue;
                    }
                    match self.changes[index] {
                        Status::Pending => {
                            let tag = if *inserted { "ins" } else { "del" };
                            body.push_str(&format!(
                                "<w:{tag} w:id=\"{}\" w:author=\"reviewer{index}\" w:date=\"2026-08-01T00:00:00Z\">{}</w:{tag}>",
                                index + 10,
                                run(text, !inserted)
                            ));
                        }
                        Status::Plain => body.push_str(&run(text, false)),
                        Status::Removed => {}
                    }
                    body.push_str(&run(" middle ", false));
                }
                body.push_str(&run("end", false));
            }
            body.push_str("</w:p>");
        }
        package(&body, "")
    }

    fn decide(&mut self, accept: bool, mask: Option<u8>, kind: u8, start: usize, end: usize) {
        for (index, (paragraph, inserted, _)) in CHANGES.iter().enumerate() {
            let selected = mask.is_none_or(|mask| mask & (1 << index) != 0);
            let kind_matches = kind == 0 || (*inserted == (kind == 1));
            if self.changes[index] == Status::Pending
                && selected
                && kind_matches
                && (start..=end).contains(paragraph)
            {
                self.changes[index] = if accept == *inserted {
                    Status::Plain
                } else {
                    Status::Removed
                };
            }
        }
    }

    fn check_state(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 3, "{context}: paragraphs");
        let mut have = Vec::new();
        for paragraph in 0..3 {
            assert_eq!(
                document.paragraph_text(paragraph).as_deref(),
                Some(self.text(paragraph).as_str()),
                "{context}: paragraph {paragraph}"
            );
            for run in &document.word_document().paragraph(paragraph).unwrap().runs {
                if let Some(mark) = &run.revision {
                    have.push((
                        paragraph,
                        !mark.is_deletion(),
                        run.text.clone(),
                        mark.author.clone(),
                    ));
                }
            }
        }
        let want: Vec<_> = CHANGES
            .iter()
            .enumerate()
            .filter(|(index, _)| self.changes[*index] == Status::Pending)
            .map(|(index, (paragraph, inserted, text))| {
                (
                    *paragraph,
                    *inserted,
                    text.to_string(),
                    Some(format!("reviewer{index}")),
                )
            })
            .collect();
        assert_eq!(have, want, "{context}: unresolved revisions");
    }

    fn check_render(&self, actual: &mut Doc, view: u8, context: &str) {
        let mut expected = open_bytes(&self.package());
        set_view(&mut expected, view);
        expected.set_selection_range(actual.selection_range());
        expected.repaint_all().unwrap();
        assert_eq!(
            actual.page_count(),
            expected.page_count(),
            "{context}: pages"
        );
        for page in 0..expected.page_count() {
            assert_page_pixels(actual, &expected, page, context);
        }
        let selection = actual.selection_range();
        for paragraph in 0..3 {
            for offset in [0, self.text(paragraph).len()] {
                let have = actual.position_rect(paragraph, offset).unwrap();
                let want = expected.position_rect(paragraph, offset).unwrap();
                for (have, want) in [
                    (have.x, want.x),
                    (have.document_y, want.document_y),
                    (have.height, want.height),
                ] {
                    assert!(
                        (have - want).abs() < COORD_EPSILON,
                        "{context}: paragraph {paragraph} caret {offset}: {have} != {want}"
                    );
                }
            }
        }
        assert_eq!(
            actual.selection_range(),
            selection,
            "{context}: queries preserve selection"
        );
        assert_eq!(
            actual.renderer().selection_range(),
            selection,
            "{context}: renderer selection"
        );
    }

    fn check_persistence(&self, document: &mut Doc, view: u8, context: &str) {
        let bytes = document.export_docx().unwrap();
        let mut reopened = open_bytes(&bytes);
        self.check_state(&reopened, context);
        assert_eq!(reopened.history_len(), 0, "{context}: reopened history");
        set_view(&mut reopened, view);
        reopened.set_selection_range(document.selection_range());
        reopened.repaint_all().unwrap();
        self.check_render(&mut reopened, view, context);
    }
}

fn decision(accept: bool, mask: Option<u8>, kind: u8) -> HeadlessCommand {
    // Bit four deliberately addresses an unknown id. An explicit empty id
    // list selects nothing; an omitted list addresses all pending changes.
    let revision_ids = mask.map(|mask| {
        (0..5)
            .filter(|index| mask & (1 << index) != 0)
            .map(|index| (index + 10).to_string())
            .collect()
    });
    let kind = match kind {
        0 => None,
        1 => Some(HeadlessRevisionKindFilter::Inserted),
        2 => Some(HeadlessRevisionKindFilter::Deleted),
        _ => panic!("invalid revision kind"),
    };
    if accept {
        HeadlessCommand::AcceptRevisions {
            kind,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids,
            revision_targets: None,
        }
    } else {
        HeadlessCommand::RejectRevisions {
            kind,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids,
            revision_targets: None,
        }
    }
}

fn set_view(document: &mut Doc, view: u8) {
    document
        .set_revision_display(view >= 2, view == 1 || view == 3, Default::default())
        .unwrap();
}

#[derive(Default)]
struct Model {
    state: State,
    undo: Vec<State>,
    redo: Vec<State>,
    view: u8,
}

impl Model {
    fn record(&mut self, next: State) {
        self.undo.push(std::mem::replace(&mut self.state, next));
        self.redo.clear();
    }

    fn check(&self, document: &mut Doc, context: &str) {
        self.state.check_state(document, context);
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
            document.revision_display(),
            (self.view >= 2, self.view == 1),
            "{context}: view"
        );
        self.state.check_render(document, self.view, context);
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Decide(bool, Option<u8>, u8),
    ScopedDecision(bool, Option<u8>, u8, Option<u8>, Option<u8>),
    Append(String),
    View(u8),
    FailedBatch(bool, Option<u8>),
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Decide(accept, mask, kind) => {
            document
                .apply_command(decision(*accept, *mask, *kind), true)
                .unwrap();
            // A review decision is an undoable action even when its nonempty
            // filter finds no remaining changes. Explicitly empty lists skip it.
            if *mask != Some(0) {
                let mut next = model.state.clone();
                next.decide(*accept, *mask, *kind, 0, 2);
                model.record(next);
            }
        }
        Operation::ScopedDecision(accept, mask, kind, start, end) => {
            let start = start.map(usize::from);
            let end = end.map(usize::from);
            let mut command = decision(*accept, *mask, *kind);
            match &mut command {
                HeadlessCommand::AcceptRevisions {
                    start_paragraph,
                    end_paragraph,
                    ..
                }
                | HeadlessCommand::RejectRevisions {
                    start_paragraph,
                    end_paragraph,
                    ..
                } => {
                    *start_paragraph = start;
                    *end_paragraph = end;
                }
                _ => unreachable!("decision returns a review command"),
            }
            let before = document.selection_range();
            let result = document.apply_command(command, true);
            let (start, end) = (start.unwrap_or(0), end.unwrap_or(2));
            // Empty ID lists skip the command before paragraph validation.
            if *mask == Some(0) {
                result.unwrap();
                assert_eq!(
                    document.selection_range(),
                    before,
                    "{context}: empty filter"
                );
            } else if start > end || end >= 3 {
                assert_eq!(
                    result.unwrap_err(),
                    format!("edit failed: Command {{ index: 0, source: Custom(\"revision paragraph range is out of bounds: start={start}, end={end}, paragraph_count=3\") }}"),
                    "{context}"
                );
                assert_eq!(
                    document.selection_range(),
                    before,
                    "{context}: rejected range"
                );
                model
                    .state
                    .check_persistence(document, model.view, &context);
            } else {
                result.unwrap();
                let mut next = model.state.clone();
                next.decide(*accept, *mask, *kind, start, end);
                model.record(next);
            }
        }
        Operation::Append(text) => {
            document
                .insert_text(2, model.state.tail.len(), text, usize::MAX)
                .unwrap();
            if !text.is_empty() {
                let mut next = model.state.clone();
                next.tail.push_str(text);
                model.record(next);
            }
        }
        Operation::View(view) => {
            let before = document.selection_range();
            set_view(document, *view);
            model.view = if *view == 3 { 2 } else { *view };
            assert_eq!(
                document.selection_range(),
                before,
                "{context}: view preserves selection"
            );
        }
        Operation::FailedBatch(accept, mask) => {
            let before = document.selection_range();
            let error = document
                .apply_commands(
                    vec![
                        decision(*accept, *mask, 0),
                        HeadlessCommand::InsertText {
                            paragraph: 3,
                            offset: 0,
                            text: "invalid".into(),
                        },
                    ],
                    false,
                )
                .unwrap_err();
            assert_eq!(
                error,
                "edit failed: Command { index: 1, source: Custom(\"paragraph 3 does not exist\") }",
                "{context}"
            );
            assert_eq!(
                document.selection_range(),
                before,
                "{context}: rejected selection"
            );
            model
                .state
                .check_persistence(document, model.view, &context);
        }
        Operation::Merge(mark) => {
            let mark = usize::from(*mark);
            document.merge_history_since(mark);
            if mark < model.undo.len() {
                model.undo.truncate(mark + 1);
            }
        }
        Operation::Undo => {
            document.undo().unwrap();
            if let Some(previous) = model.undo.pop() {
                model
                    .redo
                    .push(std::mem::replace(&mut model.state, previous));
            }
        }
        Operation::Redo => {
            document.redo().unwrap();
            if let Some(next) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.state, next));
            }
        }
    }
    model.check(document, &context);
}

fn check_sequence(operations: &[Operation]) {
    let mut model = Model {
        view: 2,
        ..Model::default()
    };
    let mut document = open_bytes(&model.state.package());
    model.check(&mut document, "initial");
    for (index, operation) in operations.iter().enumerate() {
        step(&mut model, &mut document, operation, index);
    }
    model
        .state
        .check_persistence(&mut document, model.view, "final persistence");
    let tip = model
        .redo
        .first()
        .cloned()
        .unwrap_or_else(|| model.state.clone());
    let mut index = operations.len();
    while !model.undo.is_empty() {
        step(&mut model, &mut document, &Operation::Undo, index);
        index += 1;
    }
    assert_eq!(model.state, State::default(), "undo restores all revisions");
    model
        .state
        .check_persistence(&mut document, model.view, "all undone persistence");
    while !model.redo.is_empty() {
        step(&mut model, &mut document, &Operation::Redo, index);
        index += 1;
    }
    assert_eq!(model.state, tip, "redo restores branch tip");
    model
        .state
        .check_persistence(&mut document, model.view, "all redone persistence");
}

#[test]
fn revision_decisions_intersect_ids_and_kinds_and_survive_history() {
    use Operation::*;
    check_sequence(&[
        Decide(true, Some(3), 1),
        Decide(false, Some(3), 2),
        Decide(false, Some(12), 1),
        Decide(true, Some(12), 2),
        Undo,
        Undo,
        Redo,
        Redo,
    ]);
}

#[test]
fn grouped_revision_decisions_and_typing_keep_the_redo_branch() {
    use Operation::*;
    check_sequence(&[
        Append(" & <xml>".into()),
        Decide(true, Some(5), 0),
        Decide(false, Some(10), 0),
        Merge(0),
        View(1),
        Undo,
        FailedBatch(true, None),
        View(0),
        Redo,
        View(3),
    ]);
}

#[test]
fn empty_revision_filters_and_view_changes_preserve_redo() {
    use Operation::*;
    check_sequence(&[
        Decide(true, None, 0),
        Undo,
        View(0),
        View(1),
        Decide(false, Some(0), 0),
        View(2),
        Redo,
    ]);
}

#[test]
fn rejected_revision_batches_restore_xml_selection_and_history() {
    use Operation::*;
    check_sequence(&[
        Decide(false, Some(2), 0),
        Append("😀e\u{301}".into()),
        Undo,
        FailedBatch(true, None),
        FailedBatch(false, Some(15)),
        Redo,
    ]);
}

#[test]
fn repeated_revision_decisions_remain_undoable_without_changing_text() {
    use Operation::*;
    check_sequence(&[
        Decide(true, Some(1), 0),
        Decide(true, Some(1), 0),
        Decide(false, Some(16), 0),
        Undo,
        Undo,
        Undo,
        Redo,
    ]);
}

#[test]
fn paragraph_bounds_intersect_revision_ids_and_kinds_through_history() {
    use Operation::*;
    check_sequence(&[
        ScopedDecision(true, Some(15), 1, Some(1), None),
        ScopedDecision(false, Some(15), 2, None, Some(0)),
        ScopedDecision(true, Some(3), 0, Some(1), Some(1)),
        View(0),
        Undo,
        Undo,
        View(1),
        Redo,
        ScopedDecision(false, None, 0, Some(2), Some(2)),
        View(2),
    ]);
}

#[test]
fn invalid_review_ranges_and_empty_filters_preserve_selection_and_redo() {
    use Operation::*;
    check_sequence(&[
        Append("日本語😀".into()),
        Decide(true, None, 0),
        Undo,
        ScopedDecision(true, None, 0, Some(2), Some(0)),
        ScopedDecision(false, Some(15), 1, None, Some(3)),
        ScopedDecision(true, None, 2, Some(3), None),
        ScopedDecision(false, Some(0), 0, Some(3), Some(0)),
        Redo,
        Undo,
        ScopedDecision(false, None, 0, Some(0), Some(1)),
    ]);
}

fn operation() -> impl Strategy<Value = Operation> {
    let mask = prop_oneof![Just(None), (0u8..32).prop_map(Some)];
    let bound = prop_oneof![Just(None), (0u8..4).prop_map(Some)];
    prop_oneof![
        5 => (any::<bool>(), mask.clone(), 0u8..3).prop_map(|(accept, mask, kind)| Operation::Decide(accept, mask, kind)),
        4 => (any::<bool>(), mask.clone(), 0u8..3, bound.clone(), bound).prop_map(|(accept, mask, kind, start, end)| Operation::ScopedDecision(accept, mask, kind, start, end)),
        3 => prop::sample::select(vec![String::new(), "日".into(), "😀e\u{301}".into(), " & <xml> ".into(), "wrapped 日本語 ".repeat(6)]).prop_map(Operation::Append),
        3 => (0u8..4).prop_map(Operation::View),
        2 => (any::<bool>(), mask).prop_map(|(accept, mask)| Operation::FailedBatch(accept, mask)),
        1 => (0u8..6).prop_map(Operation::Merge),
        3 => Just(Operation::Undo),
        3 => Just(Operation::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/revision_history_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn revision_decisions_and_views_follow_an_independent_history_model(operations in prop::collection::vec(operation(), 1..=16)) {
        check_sequence(&operations);
    }
}
