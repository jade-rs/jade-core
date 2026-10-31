//! Search and replacement use a plain string/snapshot oracle, including history.

use super::{assert_page_pixels, assert_same, open_bytes, package, rich_body, Doc};
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_native_render::SearchHit;
use proptest::prelude::*;

const INITIAL: [&str; 10] = [
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

fn texts(document: &Doc) -> Vec<String> {
    (0..document.paragraph_count())
        .map(|paragraph| document.paragraph_text(paragraph).unwrap())
        .collect()
}

fn hits(text: &[String], query: &str, insensitive: bool) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }
    // Generated queries contain ASCII letters and caseless Japanese/emoji.
    // ASCII folding keeps UTF-8 offsets intact in the surrounding text.
    let query = if insensitive {
        query.to_ascii_lowercase()
    } else {
        query.to_string()
    };
    text.iter()
        .enumerate()
        .flat_map(|(paragraph, text)| {
            let text = if insensitive {
                text.to_ascii_lowercase()
            } else {
                text.clone()
            };
            text.match_indices(&query)
                .map(|(start, _)| SearchHit {
                    paragraph,
                    start_offset: start,
                    end_offset: start + query.len(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[derive(Debug)]
struct Model {
    text: Vec<String>,
    undo: Vec<Vec<String>>,
    redo: Vec<Vec<String>>,
}

impl Model {
    fn new(text: Vec<String>) -> Self {
        Self {
            text,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn change(&mut self, text: Vec<String>) {
        if self.text != text {
            self.undo.push(std::mem::replace(&mut self.text, text));
            self.redo.clear();
        }
    }

    fn assert_document(&self, document: &Doc, context: &str) {
        assert_eq!(texts(document), self.text, "{context}: text");
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
    ReplaceAll(String, String),
    ReplaceOne(String, u8, String),
    Insert(u8, u8, String),
    Search(String, bool),
    ClearSelection,
    Merge(u8),
    Undo,
    Redo,
}

fn operation() -> impl Strategy<Value = Operation> {
    let replacements = prop::sample::select(vec![
        ("日本", "😀"),
        ("😀", "日本😀"),
        ("e\u{301}", "é"),
        ("aa", "a"),
        ("a", "aa"),
        ("link", "日"),
        ("page", ""),
        ("first", "first😀"),
        ("ld pl", "B"),
        ("missing", "x"),
        ("", "x"),
    ]);
    let query = prop::sample::select(vec!["", "a", "A", "日本", "😀", "link", "LINK", "e\u{301}"]);
    let text = prop::sample::select(vec!["", "x", "aa", "日本", "😀", "e\u{301}"]);
    prop_oneof![
        5 => replacements.prop_map(|(query, text)| Operation::ReplaceAll(query.into(), text.into())),
        3 => (query.clone(), any::<u8>(), text.clone()).prop_map(|(query, at, text)| Operation::ReplaceOne(query.into(), at, text.into())),
        3 => (any::<u8>(), any::<u8>(), text).prop_map(|(paragraph, at, text)| Operation::Insert(paragraph, at, text.into())),
        2 => (query, any::<bool>()).prop_map(|(query, insensitive)| Operation::Search(query.into(), insensitive)),
        1 => Just(Operation::ClearSelection),
        2 => (0u8..8).prop_map(Operation::Merge),
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
    match operation {
        Operation::ReplaceAll(query, replacement) => {
            let found = hits(&model.text, query, false);
            let expected: Vec<_> = model
                .text
                .iter()
                .map(|text| {
                    if query.is_empty() {
                        text.clone()
                    } else {
                        text.replace(query, replacement)
                    }
                })
                .collect();
            assert_eq!(
                actual.replace_all(query, replacement, usize::MAX).unwrap(),
                found.len(),
                "{context}: replacement count"
            );
            if !found.is_empty() {
                // The reference groups the executor commands itself and always
                // fully repaints; the native replace-all groups by match count.
                reference
                    .apply_commands(
                        vec![HeadlessCommand::ReplacePlaceholder {
                            placeholder: query.clone(),
                            text: replacement.clone(),
                            max_replacements: None,
                        }],
                        false,
                    )
                    .unwrap();
            }
            model.change(expected);
        }
        Operation::ReplaceOne(query, ordinal, replacement) => {
            let found = hits(&model.text, query, false);
            if let Some(hit) = found.get(usize::from(*ordinal) % found.len().max(1)) {
                let mut expected = model.text.clone();
                expected[hit.paragraph]
                    .replace_range(hit.start_offset..hit.end_offset, replacement);
                actual.replace_one(*hit, replacement, usize::MAX).unwrap();
                reference
                    .apply_command(
                        HeadlessCommand::ReplaceRange {
                            start_paragraph: hit.paragraph,
                            start_offset: hit.start_offset,
                            end_paragraph: hit.paragraph,
                            end_offset: hit.end_offset,
                            text: replacement.clone(),
                        },
                        false,
                    )
                    .unwrap();
                model.change(expected);
            }
        }
        Operation::Insert(paragraph, character, text) => {
            let paragraph = usize::from(*paragraph) % model.text.len();
            let original = &model.text[paragraph];
            let offset = original
                .char_indices()
                .nth(usize::from(*character))
                .map_or(original.len(), |(offset, _)| offset);
            let mut expected = model.text.clone();
            expected[paragraph].insert_str(offset, text);
            actual
                .insert_text(paragraph, offset, text, usize::MAX)
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
            model.change(expected);
        }
        Operation::Search(query, insensitive) => {
            let mut expected = hits(&model.text, query, *insensitive);
            expected.truncate(500);
            for document in [&*actual, &*reference] {
                assert_eq!(
                    document.find(query, *insensitive),
                    expected,
                    "{context}: hits"
                );
            }
        }
        Operation::Merge(mark) => {
            let mark = usize::from(*mark);
            if mark < model.undo.len() {
                model.undo.truncate(mark + 1);
            }
            actual.merge_history_since(mark);
            reference.merge_history_since(mark);
        }
        Operation::ClearSelection => {
            actual.set_selection_range(None);
            reference.set_selection_range(None);
        }
        Operation::Undo => {
            if let Some(text) = model.undo.pop() {
                model.redo.push(std::mem::replace(&mut model.text, text));
            }
            actual.undo().unwrap();
            reference.undo().unwrap();
        }
        Operation::Redo => {
            if let Some(text) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.text, text));
            }
            actual.redo().unwrap();
            reference.redo().unwrap();
        }
    }
    model.assert_document(actual, &context);
    model.assert_document(reference, &context);
    assert_same(actual, reference, &context);
}

fn check_sequence(operations: &[Operation]) {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    let initial: Vec<_> = INITIAL.into_iter().map(str::to_owned).collect();
    assert_eq!(texts(&actual), initial);
    let original = open_bytes(&bytes);
    let mut model = Model::new(initial.clone());
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
    assert_eq!(model.text, initial);
    for document in [&mut actual, &mut reference] {
        document.set_selection_range(None);
        document.repaint_all().unwrap();
    }
    assert_eq!(actual.page_count(), original.page_count());
    for page in 0..original.page_count() {
        assert_page_pixels(&actual, &original, page, "undo restores rich formatting");
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
    assert_eq!(texts(&reopened), model.text);
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    assert_eq!(actual.page_count(), reopened.page_count());
    for page in 0..actual.page_count() {
        assert_page_pixels(&actual, &reopened, page, "search sequence export/reopen");
    }
}

#[test]
fn replacement_across_runs_links_cells_and_pages_is_one_undo_step() {
    check_sequence(&[
        Operation::ReplaceAll("日本".into(), "日本😀".into()),
        Operation::ReplaceAll("ld pl".into(), "B".into()),
        Operation::ReplaceAll("link".into(), "日".into()),
        Operation::ReplaceAll("aa".into(), "a".into()),
        Operation::Undo,
        Operation::Redo,
        Operation::ReplaceOne("日本".into(), 1, "e\u{301}".into()),
        Operation::Search("日".into(), false),
    ]);
}

#[test]
fn searches_and_empty_or_absent_replacements_preserve_grouped_redo() {
    check_sequence(&[
        Operation::Insert(0, 0, "aa".into()),
        Operation::ReplaceAll("a".into(), "aa".into()),
        Operation::Merge(0),
        Operation::Undo,
        Operation::Search("日本".into(), false),
        Operation::Search("FIRST".into(), true),
        Operation::ReplaceAll("".into(), "x".into()),
        Operation::ReplaceAll("missing".into(), "x".into()),
        Operation::Redo,
        Operation::Undo,
        Operation::Insert(5, 0, "😀".into()),
        Operation::Redo,
    ]);
}

#[test]
fn typing_after_replace_all_undo_clears_selection_pixels_on_retained_pages() {
    for clear_selection in [false, true] {
        let mut operations = vec![
            Operation::ReplaceAll("a".into(), "aa".into()),
            Operation::Undo,
        ];
        if clear_selection {
            operations.push(Operation::ClearSelection);
        }
        operations.push(Operation::Insert(0, 0, "x".into()));
        check_sequence(&operations);
    }
}

#[test]
fn replace_all_exceeding_search_limit_reports_and_undoes_every_match() {
    let original = "x".repeat(501);
    let bytes = package(
        &format!("<w:p><w:r><w:rPr><w:sz w:val=\"4\"/></w:rPr><w:t>{original}</w:t></w:r></w:p>"),
        "",
    );
    let mut document = open_bytes(&bytes);
    document.insert_text(0, 0, "prefix ", usize::MAX).unwrap();
    assert_eq!(document.find("x", false).len(), 500, "public search cap");
    // The prefix contains one further x. Replacing all must count and group
    // every occurrence, even beyond the interactive search result limit.
    assert_eq!(document.replace_all("x", "y", usize::MAX).unwrap(), 502);
    assert_eq!(
        document.paragraph_text(0).unwrap(),
        format!("prefiy {}", "y".repeat(501))
    );
    assert_eq!(document.history_len(), 2);
    document.undo().unwrap();
    assert_eq!(
        document.paragraph_text(0).unwrap(),
        format!("prefix {original}")
    );
    assert_eq!(document.history_len(), 1);
    document.undo().unwrap();
    assert_eq!(document.paragraph_text(0).unwrap(), original);
    assert!(!document.can_undo());
    document.redo().unwrap();
    document.redo().unwrap();
    assert_eq!(
        document.paragraph_text(0).unwrap(),
        format!("prefiy {}", "y".repeat(501))
    );
    assert_eq!(document.history_len(), 2);
    let reopened = open_bytes(&document.export_docx().unwrap());
    assert_eq!(texts(&document), texts(&reopened));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/search_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn search_and_replacement_follow_an_independent_model(operations in prop::collection::vec(operation(), 1..=24)) {
        check_sequence(&operations);
    }
}
