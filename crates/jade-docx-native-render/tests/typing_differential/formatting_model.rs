//! Character formatting is checked against per-character flags, not editor runs.

use super::{assert_page_pixels, assert_same, open_bytes, package, rich_body, Doc};
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_native_render::CharacterFormat;
use jade_selection::{SelectionRange, TextPosition};
use proptest::prelude::*;

const TEXT: [&str; 10] = [
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
];
type Flags = Vec<Vec<[bool; 3]>>;

#[derive(Debug)]
struct Model {
    flags: Flags,
    undo: Vec<Flags>,
    redo: Vec<Flags>,
}

impl Model {
    fn new() -> Self {
        let mut flags: Flags = TEXT
            .iter()
            .map(|text| vec![[false; 3]; text.chars().count()])
            .collect();
        for character in &mut flags[1][..4] {
            character[0] = true;
        }
        Self {
            flags,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn format(&mut self, range: SelectionRange, kind: u8) {
        self.undo.push(self.flags.clone());
        self.redo.clear();
        let (start, end) = range.normalized();
        for (paragraph, text) in TEXT.iter().enumerate() {
            for (character, (offset, _)) in text.char_indices().enumerate() {
                let position = TextPosition::new(paragraph, offset);
                if (position.paragraph, position.offset) >= (start.paragraph, start.offset)
                    && (position.paragraph, position.offset) < (end.paragraph, end.offset)
                {
                    if kind == 3 {
                        self.flags[paragraph][character] = [false; 3];
                    } else {
                        self.flags[paragraph][character][usize::from(kind)] = true;
                    }
                }
            }
        }
    }

    fn check(&self, document: &Doc, context: &str) {
        assert_eq!(
            document.paragraph_count(),
            TEXT.len(),
            "{context}: paragraphs"
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
        assert_flags(document, &self.flags, context);
        assert_eq!(
            document.selection_range(),
            document.renderer().selection_range(),
            "{context}: renderer selection"
        );
    }
}

fn assert_flags(document: &Doc, flags: &Flags, context: &str) {
    for (paragraph, text) in TEXT.iter().enumerate() {
        assert_eq!(
            document.paragraph_text(paragraph).as_deref(),
            Some(*text),
            "{context}: text {paragraph}"
        );
        for (character, (offset, _)) in text.char_indices().enumerate() {
            let style = document.run_style_at(paragraph, offset).unwrap();
            assert_eq!(
                [
                    style.bold.unwrap_or(false),
                    style.italic.unwrap_or(false),
                    style.underline.unwrap_or(false)
                ],
                flags[paragraph][character],
                "{context}: style {paragraph}:{offset}"
            );
        }
    }
}

fn position(paragraph: u8, character: u8) -> TextPosition {
    let paragraph = usize::from(paragraph) % TEXT.len();
    let offset = TEXT[paragraph]
        .char_indices()
        .nth(usize::from(character))
        .map_or(TEXT[paragraph].len(), |(offset, _)| offset);
    TextPosition::new(paragraph, offset)
}

fn command(range: SelectionRange, kind: u8) -> HeadlessCommand {
    let (start, end) = range.normalized();
    match kind {
        0 => HeadlessCommand::ApplyBold {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
            enabled: true,
        },
        1 => HeadlessCommand::ApplyItalic {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
            enabled: true,
        },
        2 => HeadlessCommand::ApplyUnderline {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
            enabled: true,
            kind: None,
        },
        3 => HeadlessCommand::ClearFormatting {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
        },
        _ => unreachable!(),
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Format(u8, u8, u8, u8, u8),
    Rejected(u8),
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
    let context = format!("format step {step}: {operation:?}");
    match *operation {
        Operation::Format(kind, paragraph, character, last, to) => {
            let range = SelectionRange::new(position(paragraph, character), position(last, to));
            actual.set_selection_range(Some(range));
            reference.set_selection_range(Some(range));
            let format = [
                CharacterFormat::Bold,
                CharacterFormat::Italic,
                CharacterFormat::Underline,
                CharacterFormat::Clear,
            ][usize::from(kind)];
            let result = actual.format_selection(format, actual.page_count());
            let full = reference.apply_command(command(range, kind), true);
            if range.is_collapsed() {
                assert!(result.is_err(), "{context}");
                assert!(full.is_err(), "{context}");
            } else {
                result.unwrap();
                full.unwrap();
                model.format(range, kind);
            }
            assert_eq!(
                actual.selection_range(),
                Some(range),
                "{context}: selection"
            );
        }
        Operation::Rejected(kind) => {
            let before = actual.selection_range();
            // Both are invalid UTF-8 offsets; another case is a missing paragraph.
            let range = if kind == 0 {
                SelectionRange::new(TextPosition::new(0, 7), TextPosition::new(0, 10))
            } else {
                SelectionRange::new(
                    TextPosition::new(TEXT.len(), 0),
                    TextPosition::new(TEXT.len(), 1),
                )
            };
            for document in [&mut *actual, &mut *reference] {
                document.set_selection_range(Some(range));
            }
            let rendered_selection = actual.renderer().selection_range();
            assert!(
                actual
                    .format_selection(CharacterFormat::Bold, actual.page_count())
                    .is_err(),
                "{context}"
            );
            assert!(
                reference.apply_command(command(range, 0), true).is_err(),
                "{context}"
            );
            assert_eq!(
                actual.selection_range(),
                Some(range),
                "{context}: rejected selection"
            );
            assert_eq!(
                reference.selection_range(),
                Some(range),
                "{context}: rejected full selection"
            );
            assert_eq!(
                actual.renderer().selection_range(),
                rendered_selection,
                "{context}: rejected renderer selection"
            );
            // The renderer clamps caller-provided invalid positions to its layout.
            // Restore the valid caller selection before continuing the workflow.
            actual.set_selection_range(before);
            reference.set_selection_range(before);
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
            if let Some(previous) = model.undo.pop() {
                model
                    .redo
                    .push(std::mem::replace(&mut model.flags, previous));
            }
            actual.undo().unwrap();
            reference.undo().unwrap();
        }
        Operation::Redo => {
            if let Some(next) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.flags, next));
            }
            actual.redo().unwrap();
            reference.redo().unwrap();
        }
    }
    model.check(actual, &context);
    model.check(reference, &context);
    assert_same(actual, reference, &context);
}

fn check_sequence(operations: &[Operation]) {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    let mut model = Model::new();
    model.check(&actual, "initial");
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
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    let original = open_bytes(&bytes);
    assert_flags(&actual, &Model::new().flags, "all formats undone");
    for page in 0..original.page_count() {
        assert_page_pixels(&actual, &original, page, "all formats undone");
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
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    let reopened = open_bytes(&actual.export_docx().unwrap());
    assert_flags(&reopened, &model.flags, "persisted formatting");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    assert_eq!(actual.page_count(), reopened.page_count());
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "persisted formatting");
    }
}

#[test]
fn italic_after_bold_and_clear_remeasures_every_affected_paragraph() {
    check_sequence(&[
        Operation::Format(0, 0, 0, 5, 0),
        Operation::Format(3, 0, 0, 2, 0),
        Operation::Format(1, 2, 0, 0, 0),
    ]);
}

#[test]
fn overlapping_formats_clear_and_grouped_history_preserve_rich_runs() {
    check_sequence(&[
        Operation::Format(1, 1, 2, 1, 8),
        Operation::Format(2, 9, 9, 1, 3),
        Operation::Format(0, 2, 0, 7, 6),
        Operation::Format(3, 1, 3, 9, 7),
        Operation::Merge(0),
        Operation::Undo,
        Operation::Rejected(0),
        Operation::Redo,
        Operation::Format(1, 6, 2, 6, 6),
        Operation::Undo,
        Operation::Format(2, 9, 5, 9, 7),
    ]);
}

#[test]
fn collapsed_formatting_preserves_grouped_redo() {
    check_sequence(&[
        Operation::Format(0, 1, 1, 1, 9),
        Operation::Format(1, 2, 0, 2, 8),
        Operation::Merge(0),
        Operation::Undo,
        Operation::Format(3, 6, 5, 6, 5),
        Operation::Rejected(1),
        Operation::Redo,
    ]);
}

#[test]
fn failed_formatting_batches_restore_styles_selection_pixels_and_grouped_redo() {
    for invalid in [
        SelectionRange::new(TextPosition::new(0, 7), TextPosition::new(0, 10)),
        SelectionRange::new(TextPosition::new(10, 0), TextPosition::new(10, 1)),
        SelectionRange::collapsed(TextPosition::new(6, 5)),
    ] {
        let bytes = package(rich_body(), "");
        let mut actual = open_bytes(&bytes);
        let mut reference = open_bytes(&bytes);
        let mut model = Model::new();
        for (step, operation) in [
            Operation::Format(1, 1, 2, 1, 8),
            Operation::Format(2, 9, 8, 2, 1),
            Operation::Merge(0),
            Operation::Undo,
        ]
        .iter()
        .enumerate()
        {
            check_step(&mut model, &mut actual, &mut reference, operation, step);
        }
        let selection = Some(SelectionRange::new(position(9, 9), position(9, 1)));
        for document in [&mut actual, &mut reference] {
            document.set_selection_range(selection);
            document.repaint_all().unwrap();
        }
        let error = actual
            .apply_commands(
                vec![
                    command(SelectionRange::new(position(2, 1), position(7, 8)), 0),
                    command(SelectionRange::new(position(1, 1), position(9, 7)), 3),
                    command(invalid, 1),
                ],
                true,
            )
            .unwrap_err();
        let context = format!("failed formatting batch {invalid:?}: {error}");
        model.check(&actual, &context);
        assert_eq!(actual.selection_range(), selection, "{context}");
        assert_same(&mut actual, &mut reference, &context);
        check_step(&mut model, &mut actual, &mut reference, &Operation::Redo, 4);
        check_step(&mut model, &mut actual, &mut reference, &Operation::Undo, 5);
        actual.set_selection_range(None);
        actual.repaint_all().unwrap();
        let original = open_bytes(&bytes);
        for page in 0..original.page_count() {
            assert_page_pixels(&actual, &original, page, &context);
        }
        let reopened = open_bytes(&actual.export_docx().unwrap());
        assert_flags(&reopened, &model.flags, &context);
    }
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        5 => (0u8..4, 0u8..10, 0u8..18, 0u8..10, 0u8..18).prop_map(|(kind, paragraph, from, last, to)| Operation::Format(kind, paragraph, from, last, to)),
        3 => (0u8..4, 0u8..10, 0u8..18, 0u8..18).prop_map(|(kind, paragraph, from, to)| Operation::Format(kind, paragraph, from, paragraph, to)),
        1 => (0u8..2).prop_map(Operation::Rejected),
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/formatting_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn formatting_follows_an_independent_character_model(operations in prop::collection::vec(operation(), 1..=20)) {
        check_sequence(&operations);
    }
}

#[test]
fn font_size_edits_keep_complex_script_size_equal_to_saved_model() {
    let text = "Latin العربية 日本語 😀";
    let bytes = package(
        &format!(
            r#"<w:p><w:r><w:rPr><w:sz w:val="24"/><w:szCs w:val="36"/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#
        ),
        "",
    );
    let original = open_bytes(&bytes);
    let mut actual = open_bytes(&bytes);
    actual
        .select((0, 0), (0, text.len()), actual.page_count())
        .unwrap();
    actual.set_font_size(10.5, actual.page_count()).unwrap();
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    let reopened = open_bytes(&actual.export_docx().unwrap());
    let sizes = |document: &Doc| {
        document
            .word_document()
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .map(|run| {
                let props = run.properties.as_ref().unwrap();
                (props.font_size, props.font_size_complex)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(sizes(&actual), vec![(Some(10.5), Some(10.5))]);
    assert_eq!(sizes(&actual), sizes(&reopened));
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "font size export/reopen");
    }
    actual.undo().unwrap();
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    assert_eq!(sizes(&actual), sizes(&original));
    for page in 0..original.page_count() {
        assert_page_pixels(&actual, &original, page, "font size undo");
    }
    actual.redo().unwrap();
    actual.set_selection_range(None);
    actual.repaint_all().unwrap();
    assert_eq!(sizes(&actual), sizes(&reopened));
    for page in 0..reopened.page_count() {
        assert_page_pixels(&actual, &reopened, page, "font size redo");
    }
}
