//! Body commands and package-backed stories share one user-visible history.

use super::{assert_page_pixels, open_bytes, Doc};
use jade_selection::{SelectionRange, TextPosition};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    body: [String; 2],
    header: [String; 2],
    footer: [String; 2],
}

impl Default for State {
    fn default() -> Self {
        Self {
            body: ["body 日本語".into(), "second 😀e\u{301}".into()],
            header: ["header 日本語".into(), "header two".into()],
            footer: ["footer 😀".into(), "footer two".into()],
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn paragraphs(text: &[String; 2], page_break: bool) -> String {
    text.iter()
        .enumerate()
        .map(|(index, text)| {
            let properties = if page_break && index == 1 {
                "<w:pPr><w:pageBreakBefore/></w:pPr>"
            } else {
                ""
            };
            format!(
                "<w:p>{properties}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                escape(text)
            )
        })
        .collect()
}

fn package(state: &State) -> Vec<u8> {
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{}<w:sectPr><w:headerReference w:type="default" r:id="header"/><w:footerReference w:type="default" r:id="footer"/><w:pgSz w:w="7200" w:h="7200"/><w:pgMar w:top="1080" w:right="720" w:bottom="1080" w:left="720" w:header="360" w:footer="360"/></w:sectPr></w:body></w:document>"#,
        paragraphs(&state.body, true)
    );
    let header = format!(
        r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{}</w:hdr>"#,
        paragraphs(&state.header, false)
    );
    let footer = format!(
        r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{}</w:ftr>"#,
        paragraphs(&state.footer, false)
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, xml) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="doc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="header" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="footer" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#,
        ),
        ("word/document.xml", document.as_str()),
        ("word/header1.xml", header.as_str()),
        ("word/footer1.xml", footer.as_str()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Body,
    Story,
}

#[derive(Default)]
struct Model {
    state: State,
    undo: Vec<(State, Kind)>,
    redo: Vec<(State, Kind)>,
}

impl Model {
    fn change(&mut self, next: State, kind: Kind) {
        self.undo
            .push((std::mem::replace(&mut self.state, next), kind));
        self.redo.clear();
    }

    fn check_state(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 2, "{context}: paragraphs");
        for (index, text) in self.state.body.iter().enumerate() {
            assert_eq!(
                document.paragraph_text(index).as_ref(),
                Some(text),
                "{context}: body {index}"
            );
        }
        for (lines, expected) in [
            (&document.word_document().header_lines, &self.state.header),
            (&document.word_document().footer_lines, &self.state.footer),
        ] {
            let lines = lines.as_ref().expect("story lines");
            assert_eq!(lines.len(), 2, "{context}: story paragraphs");
            for (line, text) in lines.iter().zip(expected) {
                assert_eq!(
                    line.left.as_deref().unwrap_or_default(),
                    text,
                    "{context}: story text"
                );
                assert!(
                    line.center.as_deref().unwrap_or_default().is_empty(),
                    "{context}"
                );
                assert!(
                    line.right.as_deref().unwrap_or_default().is_empty(),
                    "{context}"
                );
            }
        }
    }

    fn check(&self, actual: &mut Doc, context: &str) {
        self.check_state(actual, context);
        assert_eq!(actual.history_len(), self.undo.len(), "{context}: history");
        assert_eq!(actual.can_undo(), !self.undo.is_empty(), "{context}: undo");
        assert_eq!(actual.can_redo(), !self.redo.is_empty(), "{context}: redo");
        assert_eq!(
            actual.renderer().selection_range(),
            actual.selection_range(),
            "{context}: renderer selection"
        );
        // Build the oracle directly from plain model strings. It has neither
        // the actual session's commands nor its package/cache history.
        let mut expected = open_bytes(&package(&self.state));
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
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Story(bool, u8, String),
    Insert(u8, String),
    RejectedStory(bool),
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Story(footer, paragraph, text) => {
            let paragraph = usize::from(*paragraph);
            document
                .set_header_footer_text(*footer, paragraph, text)
                .unwrap();
            let mut next = model.state.clone();
            let story = if *footer {
                &mut next.footer
            } else {
                &mut next.header
            };
            story[paragraph] = text.clone();
            model.change(next, Kind::Story);
        }
        Operation::Insert(paragraph, text) => {
            let paragraph = usize::from(*paragraph);
            document
                .insert_text(
                    paragraph,
                    model.state.body[paragraph].len(),
                    text,
                    usize::MAX,
                )
                .unwrap();
            if !text.is_empty() {
                let mut next = model.state.clone();
                next.body[paragraph].push_str(text);
                model.change(next, Kind::Body);
            }
        }
        Operation::RejectedStory(footer) => {
            let before = document.selection_range();
            assert!(
                document
                    .set_header_footer_text(*footer, 2, "invalid")
                    .is_err(),
                "{context}"
            );
            assert_eq!(document.selection_range(), before, "{context}: selection");
        }
        Operation::Merge(mark) => {
            let mark = usize::from(*mark);
            document.merge_history_since(mark);
            if mark < model.undo.len()
                && model.undo[mark..]
                    .iter()
                    .all(|(_, kind)| *kind == Kind::Body)
            {
                model.undo.truncate(mark + 1);
            }
        }
        Operation::Undo => {
            document.undo().unwrap();
            if let Some((previous, kind)) = model.undo.pop() {
                model
                    .redo
                    .push((std::mem::replace(&mut model.state, previous), kind));
            }
        }
        Operation::Redo => {
            document.redo().unwrap();
            if let Some((next, kind)) = model.redo.pop() {
                model
                    .undo
                    .push((std::mem::replace(&mut model.state, next), kind));
            }
        }
    }
    model.check(document, &context);
}

fn check_sequence(operations: &[Operation]) {
    let mut model = Model::default();
    let mut document = open_bytes(&package(&model.state));
    model.check(&mut document, "initial");
    for (index, operation) in operations.iter().enumerate() {
        step(&mut model, &mut document, operation, index);
    }
    let final_tip = model
        .redo
        .first()
        .map_or_else(|| model.state.clone(), |(state, _)| state.clone());
    let mut reopened = open_bytes(&document.export_docx().unwrap());
    model.check_state(&reopened, "export/reopen");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    reopened.set_selection_range(document.selection_range());
    reopened.repaint_all().unwrap();
    assert_eq!(document.page_count(), reopened.page_count());
    for page in 0..document.page_count() {
        assert_page_pixels(&document, &reopened, page, "export/reopen");
    }
    let mut index = operations.len();
    while !model.undo.is_empty() {
        step(&mut model, &mut document, &Operation::Undo, index);
        index += 1;
    }
    assert_eq!(model.state, State::default(), "undo restores all stories");
    while !model.redo.is_empty() {
        step(&mut model, &mut document, &Operation::Redo, index);
        index += 1;
    }
    // A sequence ending in Undo still has an existing redo branch.
    assert_eq!(model.state, final_tip, "redo restores the complete branch");
}

#[test]
fn wrapped_story_edits_preserve_default_metrics_through_history_and_reopen() {
    use Operation::*;
    for footer in [false, true] {
        for paragraph in 0..2 {
            check_sequence(&[
                Story(footer, paragraph, "折り返し 日本語 ab ".repeat(10)),
                Story(footer, paragraph, "短い文".into()),
                Undo,
                Undo,
                Redo,
                Redo,
            ]);
        }
    }
}

#[test]
fn story_edit_after_body_undo_invalidates_the_redo_branch() {
    use Operation::*;
    for footer in [false, true] {
        check_sequence(&[
            Insert(0, " deleted branch".into()),
            Undo,
            Story(footer, 0, "新しい本文外の編集".into()),
            Redo,
        ]);
        // Invalidate both kinds of redo, including every executor command
        // hidden behind a single grouped body action.
        check_sequence(&[
            Story(!footer, 1, "discarded story".into()),
            Insert(0, "a".into()),
            Insert(1, "b".into()),
            Merge(1),
            Undo,
            Undo,
            Story(footer, 0, "new branch".into()),
            Redo,
        ]);
    }
}

#[test]
fn interleaved_stories_and_grouped_body_commands_restore_their_own_state() {
    use Operation::*;
    check_sequence(&[
        Story(false, 1, "見出し & <日本語>".into()),
        Insert(0, "a".into()),
        Insert(1, "😀".into()),
        Merge(1),
        Story(true, 0, "footer e\u{301}".into()),
        Merge(0), // A span containing a story must not be merged.
        Undo,
        Undo,
        Undo,
        Redo,
        Redo,
        Redo,
        Undo,
        RejectedStory(true),
        Insert(0, String::new()), // No-op and rejection preserve redo.
        Redo,
    ]);
}

fn members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|index| {
            let mut part = zip.by_index(index).unwrap();
            let name = part.name().unwrap().into_owned();
            let mut content = Vec::new();
            part.read_to_end(&mut content).unwrap();
            (name, content)
        })
        .collect()
}

#[test]
fn story_creation_and_rejection_restore_the_package_and_body_selection() {
    for footer in [false, true] {
        let bytes = super::package("<w:p><w:r><w:t>body 日本語</w:t></w:r></w:p>", "");
        let mut document = open_bytes(&bytes);
        document.insert_text(0, 0, "x", usize::MAX).unwrap();
        document.undo().unwrap();
        let selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 4),
        ));
        document.set_selection_range(selection);
        document.repaint_all().unwrap();
        let original = document.export_docx().unwrap();
        let original_members = members(&original);
        let mut expected = open_bytes(&original);
        expected.set_selection_range(selection);
        expected.repaint_all().unwrap();

        // The setter creates a missing part before resolving the paragraph.
        // Rejecting paragraph 2 must roll back relationships, styles and XML.
        assert!(document
            .set_header_footer_text(footer, 2, "invalid")
            .is_err());
        assert_eq!(document.selection_range(), selection);
        assert_eq!(document.renderer().selection_range(), selection);
        assert_eq!(document.history_len(), 0);
        assert!(!document.can_undo());
        assert!(document.can_redo());
        assert!(document.word_document().header_lines.is_none());
        assert!(document.word_document().footer_lines.is_none());
        assert!(
            members(&document.export_docx().unwrap()) == original_members,
            "a rejected story creation must preserve every package member"
        );
        assert_page_pixels(&document, &expected, 0, "rejected story creation");
        document.redo().unwrap();
        assert_eq!(document.paragraph_text(0).as_deref(), Some("xbody 日本語"));
        document.undo().unwrap();

        document
            .set_header_footer_text(footer, 0, "new 日本語 & <story>")
            .unwrap();
        assert_eq!(document.history_len(), 1);
        assert!(!document.can_redo());
        let created = document.export_docx().unwrap();
        let reopened = open_bytes(&created);
        let lines = if footer {
            &reopened.word_document().footer_lines
        } else {
            &reopened.word_document().header_lines
        };
        assert_eq!(
            lines.as_ref().unwrap()[0].left.as_deref(),
            Some("new 日本語 & <story>")
        );

        document.undo().unwrap();
        assert!(
            members(&document.export_docx().unwrap()) == original_members,
            "undo must remove a newly created story and its package references"
        );
        assert!(!document.can_undo());
        assert!(document.can_redo());
        document.redo().unwrap();
        assert!(
            members(&document.export_docx().unwrap()) == members(&created),
            "redo must restore the new story and its package references"
        );
        assert!(!document.can_redo());
    }
}

fn operation() -> impl Strategy<Value = Operation> {
    let text = prop::sample::select(vec![
        String::new(),
        "日".into(),
        "😀e\u{301}".into(),
        " & <xml> ".into(),
        "header 日本語".into(),
        "折り返し 日本語 ab ".repeat(10),
    ]);
    prop_oneof![
        5 => (any::<bool>(), 0u8..2, text.clone()).prop_map(|(footer, p, text)| Operation::Story(footer, p, text)),
        4 => (0u8..2, text).prop_map(|(p, text)| Operation::Insert(p, text)),
        1 => any::<bool>().prop_map(Operation::RejectedStory),
        1 => (0u8..8).prop_map(Operation::Merge),
        3 => Just(Operation::Undo),
        3 => Just(Operation::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/story_history_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn story_and_body_edits_follow_an_independent_history_model(
        operations in prop::collection::vec(operation(), 1..=16)
    ) {
        check_sequence(&operations);
    }
}
