//! Note stories keep their text, XML, and rendering through shared edit history.

use super::{assert_page_pixels, open_bytes, Doc, COORD_EPSILON};
use jade_docx_editor::word_document::ParagraphStory;
use jade_docx_headless_api::HeadlessCommand;
use proptest::prelude::*;
use std::io::{Cursor, Write};

#[derive(Clone, Debug, PartialEq, Eq)]
struct State([String; 6]);

impl Default for State {
    fn default() -> Self {
        Self([
            "body 日本語".into(),
            "later 😀e\u{301}".into(),
            "foot 日本語 & link".into(),
            String::new(),
            "end 😀e\u{301} link".into(),
            String::new(),
        ])
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn run(text: &str) -> String {
    format!(
        "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        escape(text)
    )
}

impl State {
    fn package(&self) -> Vec<u8> {
        self.package_with_links(false)
    }

    fn package_with_links(&self, links: bool) -> Vec<u8> {
        let document = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p>{}<w:r><w:footnoteReference w:id="7"/></w:r></w:p>
            <w:p><w:pPr><w:pageBreakBefore/></w:pPr>{}<w:r><w:endnoteReference w:id="9"/></w:r></w:p>
            <w:sectPr><w:pgSz w:w="7200" w:h="7200"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/></w:sectPr></w:body></w:document>"#,
            run(&self.0[0]),
            run(&self.0[1])
        );
        let note = |kind: &str, id: u8, first: &str, second: &str| {
            let first = if links {
                format!(
                    "<w:hyperlink w:anchor=\"destination\">{}</w:hyperlink>",
                    run(first)
                )
            } else {
                run(first)
            };
            format!(
                r#"<w:{kind}s xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:{kind} w:id="{id}">
                <w:p><w:pPr><w:pStyle w:val="Note"/></w:pPr><w:r><w:{kind}Ref/></w:r>{first}</w:p>
                <w:p><w:pPr><w:pStyle w:val="Note"/></w:pPr>{}</w:p>
                </w:{kind}></w:{kind}s>"#,
                run(second)
            )
        };
        let footnotes = note("footnote", 7, &self.0[2], &self.0[3]);
        let endnotes = note("endnote", 9, &self.0[4], &self.0[5]);
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/><Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="doc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="foot" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="end" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="endnotes.xml"/></Relationships>"#,
            ),
            (
                "word/styles.xml",
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="Noto Sans JP" w:hAnsi="Noto Sans JP" w:eastAsia="Noto Sans JP"/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Note"><w:name w:val="Note"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="28"/></w:rPr></w:style></w:styles>"#,
            ),
            ("word/document.xml", document.as_str()),
            ("word/footnotes.xml", footnotes.as_str()),
            ("word/endnotes.xml", endnotes.as_str()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn check_state(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 6, "{context}: surface count");
        for (target, text) in self.0.iter().enumerate() {
            let paragraph = surface(document, target);
            assert_eq!(
                document.paragraph_text(paragraph).as_ref(),
                Some(text),
                "{context}: target {target}"
            );
            if target >= 2 {
                assert_eq!(
                    document.paragraph_style(paragraph).as_deref(),
                    Some("Note"),
                    "{context}: note style"
                );
            }
        }
        for (infos, first) in [
            (document.word_document().footnote_infos(), 2),
            (document.word_document().endnote_infos(), 4),
        ] {
            assert_eq!(infos.len(), 1, "{context}: one note of each kind");
            assert_eq!(infos[0].reference_count, 1, "{context}: reference retained");
            assert_eq!(
                infos[0].text,
                format!("{}\n{}", self.0[first], self.0[first + 1]),
                "{context}: note summary"
            );
        }
    }

    fn check_render(&self, actual: &mut Doc, context: &str) {
        let mut expected = open_bytes(&self.package());
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
        for (target, text) in self.0.iter().enumerate() {
            for offset in [0, text.len()] {
                let paragraph = surface(actual, target);
                let rendered = offset + marker_len(target);
                let have = actual
                    .position_rect(paragraph, rendered)
                    .expect("actual caret");
                let want = expected
                    .position_rect(surface(&expected, target), rendered)
                    .expect("oracle caret");
                for (have, want) in [
                    (have.x, want.x),
                    (have.document_y, want.document_y),
                    (have.height, want.height),
                ] {
                    assert!(
                        (have - want).abs() < COORD_EPSILON,
                        "{context}: target {target} caret {offset}: {have} != {want}"
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

    fn check_persistence(&self, document: &mut Doc, context: &str) {
        let bytes = document.export_docx().unwrap();
        let mut reopened = open_bytes(&bytes);
        self.check_state(&reopened, context);
        assert_eq!(reopened.history_len(), 0, "{context}: fresh history");
        reopened.set_selection_range(document.selection_range());
        reopened.repaint_all().unwrap();
        self.check_render(&mut reopened, context);
    }
}

// Resolve public story identities instead of depending on interleaved surface
// indices. The reference state contains only strings, never production runs.
fn surface(document: &Doc, target: usize) -> usize {
    let story = match target {
        0..=1 => ParagraphStory::Body { index: target },
        2..=3 => ParagraphStory::Footnote {
            note_id: "7".into(),
            paragraph: target - 2,
        },
        4..=5 => ParagraphStory::Endnote {
            note_id: "9".into(),
            paragraph: target - 4,
        },
        _ => panic!("invalid model target {target}"),
    };
    document
        .word_document()
        .paragraph_story_surface(&story)
        .expect("story surface")
}

fn marker_len(target: usize) -> usize {
    // These fixtures have one footnote ("1") and one endnote ("i"). Only
    // their first paragraph has a rendered marker before its editable text.
    usize::from(target == 2 || target == 4)
}

fn boundary(text: &str, index: u8) -> usize {
    let boundaries: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    boundaries[usize::from(index) % boundaries.len()]
}

#[derive(Default)]
struct Model {
    state: State,
    undo: Vec<State>,
    redo: Vec<State>,
}

impl Model {
    fn change(&mut self, next: State) {
        if next != self.state {
            self.undo.push(std::mem::replace(&mut self.state, next));
            self.redo.clear();
        }
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
        self.state.check_render(document, context);
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Append(u8, String),
    Replace(u8, u8, u8, bool, String),
    ReplaceAll(u8, String),
    Repeat(u8),
    FailedBatch(u8),
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Append(target, text) => {
            let target = usize::from(*target);
            document
                .insert_text(
                    surface(document, target),
                    marker_len(target) + model.state.0[target].len(),
                    text,
                    usize::MAX,
                )
                .unwrap();
            let mut next = model.state.clone();
            next.0[target].push_str(text);
            model.change(next);
        }
        Operation::Replace(target, a, b, reverse, text) => {
            let target = usize::from(*target);
            let a = boundary(&model.state.0[target], *a);
            let b = boundary(&model.state.0[target], *b);
            let (start, end) = (a.min(b), a.max(b));
            let (anchor, focus) = if *reverse { (end, start) } else { (start, end) };
            document
                .replace_range(
                    surface(document, target),
                    marker_len(target) + anchor,
                    marker_len(target) + focus,
                    text,
                    usize::MAX,
                )
                .unwrap();
            let mut next = model.state.clone();
            next.0[target].replace_range(start..end, text);
            if *reverse && start < end && next == model.state {
                // Reverse selections apply a replacement without the forward
                // prefix/suffix optimization, even when the text is equal.
                model.undo.push(model.state.clone());
                model.redo.clear();
            } else {
                model.change(next);
            }
        }
        Operation::ReplaceAll(target, text) => {
            let target = usize::from(*target);
            document
                .replace_range(
                    surface(document, target),
                    marker_len(target),
                    marker_len(target) + model.state.0[target].len(),
                    text,
                    usize::MAX,
                )
                .unwrap();
            let mut next = model.state.clone();
            next.0[target] = text.clone();
            model.change(next);
        }
        Operation::Repeat(target) => {
            let target = usize::from(*target);
            document
                .replace_range(
                    surface(document, target),
                    marker_len(target),
                    marker_len(target) + model.state.0[target].len(),
                    &model.state.0[target],
                    usize::MAX,
                )
                .unwrap();
        }
        Operation::FailedBatch(target) => {
            let target = usize::from(*target);
            let before = document.selection_range();
            let result = document.apply_commands(
                vec![
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: surface(document, target),
                        start_offset: marker_len(target),
                        end_paragraph: surface(document, target),
                        end_offset: marker_len(target) + model.state.0[target].len(),
                        text: "temporary 日".into(),
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 6,
                        offset: 0,
                        text: "invalid".into(),
                    },
                ],
                false,
            );
            assert_eq!(
                result.unwrap_err(),
                "edit failed: Command { index: 1, source: Custom(\"paragraph 6 does not exist\") }",
                "{context}"
            );
            assert_eq!(
                document.selection_range(),
                before,
                "{context}: rejected selection"
            );
            model.state.check_persistence(document, &context);
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
    let mut model = Model::default();
    let mut document = open_bytes(&model.state.package());
    model.check(&mut document, "initial");
    for (index, operation) in operations.iter().enumerate() {
        step(&mut model, &mut document, operation, index);
    }
    model
        .state
        .check_persistence(&mut document, "final persistence");
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
    assert_eq!(model.state, State::default(), "undo restores all stories");
    model
        .state
        .check_persistence(&mut document, "all undone persistence");
    while !model.redo.is_empty() {
        step(&mut model, &mut document, &Operation::Redo, index);
        index += 1;
    }
    assert_eq!(model.state, tip, "redo restores branch tip");
    model
        .state
        .check_persistence(&mut document, "all redone persistence");
}

#[test]
fn typing_after_deleting_body_text_keeps_note_reference_rendering() {
    check_sequence(&[
        Operation::Replace(0, 117, 242, false, String::new()),
        Operation::Append(0, "日".into()),
    ]);
}

#[test]
fn typing_after_deleting_body_text_keeps_endnote_reference_rendering() {
    check_sequence(&[
        Operation::ReplaceAll(1, String::new()),
        Operation::Append(1, "日".into()),
    ]);
}

#[test]
fn replacing_all_body_text_keeps_note_reference_rendering() {
    for target in [0, 1] {
        check_sequence(&[Operation::ReplaceAll(target, "日".into())]);
    }
}

#[test]
fn replacing_note_links_and_undo_restores_saved_text() {
    for target in [2, 4] {
        let initial = State::default();
        let bytes = initial.package_with_links(true);
        let mut actual = open_bytes(&bytes);
        actual
            .replace_range(
                surface(&actual, target),
                marker_len(target),
                marker_len(target) + initial.0[target].len(),
                "置換 😀 & <xml>",
                usize::MAX,
            )
            .unwrap();
        for cycle in 0..2 {
            actual.undo().unwrap();
            initial.check_state(&actual, "undo linked note");
            let saved = actual.export_docx().unwrap();
            let mut reopened = open_bytes(&saved);
            initial.check_state(&reopened, "saved linked note after undo");
            let mut expected = open_bytes(&bytes);
            expected.set_selection_range(actual.selection_range());
            expected.repaint_all().unwrap();
            reopened.set_selection_range(actual.selection_range());
            reopened.repaint_all().unwrap();
            for page in 0..expected.page_count() {
                assert_page_pixels(&actual, &expected, page, "undo linked note");
                assert_page_pixels(&reopened, &expected, page, "saved linked note after undo");
            }
            if cycle == 0 {
                actual.redo().unwrap();
            }
        }
    }
}

#[test]
fn typing_into_empty_styled_notes_round_trips_through_history() {
    for target in [3, 5] {
        check_sequence(&[Operation::Append(target, "日".into()), Operation::Undo]);
    }
}

#[test]
fn deleting_all_note_text_preserves_saved_empty_paragraph_metrics() {
    for target in [2, 3, 4, 5] {
        check_sequence(&[
            Operation::Append(target, "日".into()),
            Operation::ReplaceAll(target, String::new()),
        ]);
    }
}

#[test]
fn body_and_both_note_kinds_share_grouped_history() {
    use Operation::*;
    check_sequence(&[
        Append(0, "本文".into()),
        Append(2, "脚注".into()),
        Append(4, "文末".into()),
        Merge(0),
        Append(3, "😀".into()),
        Append(5, "e\u{301}".into()),
        Merge(1),
        Undo,
        Undo,
        Redo,
        Redo,
    ]);
}

#[test]
fn rejected_note_replacement_preserves_redo_and_original_package() {
    use Operation::*;
    for target in [2, 3, 4, 5] {
        check_sequence(&[
            Append(0, "redo branch".into()),
            Undo,
            FailedBatch(target),
            Repeat(target),
            Append(target, String::new()),
            Redo,
        ]);
    }
}

#[test]
fn note_replacement_trims_against_text_after_the_number() {
    for (target, replacement) in [(2, "oZ"), (4, "nZ")] {
        check_sequence(&[
            Operation::Replace(target, 0, 2, false, replacement.into()),
            Operation::Repeat(target),
            Operation::Undo,
            Operation::Redo,
        ]);
    }
}

fn operation() -> impl Strategy<Value = Operation> {
    let text = prop::sample::select(vec![
        String::new(),
        "日".into(),
        "😀e\u{301}".into(),
        " & <xml> ".into(),
        "折り返し 日本語 ab ".repeat(4),
    ]);
    prop_oneof![
        5 => (0u8..6, text.clone()).prop_map(|(p, text)| Operation::Append(p, text)),
        4 => (0u8..6, any::<u8>(), any::<u8>(), any::<bool>(), text.clone()).prop_map(|(p, a, b, reverse, text)| Operation::Replace(p, a, b, reverse, text)),
        2 => (2u8..6, text).prop_map(|(p, text)| Operation::ReplaceAll(p, text)),
        1 => (0u8..6).prop_map(Operation::Repeat),
        1 => (2u8..6).prop_map(Operation::FailedBatch),
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/note_history_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn note_edits_follow_an_independent_history_model(operations in prop::collection::vec(operation(), 1..=16)) {
        check_sequence(&operations);
    }
}
