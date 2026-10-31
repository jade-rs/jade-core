//! Style-inherited lists and direct overrides across editing history and pages.

use super::{assert_page_pixels, open_bytes, package, Doc, COORD_EPSILON};
use jade_docx_headless_api::HeadlessCommand;
use proptest::prelude::*;
use std::io::{Cursor, Read, Write};

const STYLES: &str = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="Noto Sans JP" w:hAnsi="Noto Sans JP" w:eastAsia="Noto Sans JP"/><w:b w:val="0"/><w:sz w:val="24"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="ListBase"><w:name w:val="ListBase"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="ListChild"><w:name w:val="ListChild"/><w:basedOn w:val="ListBase"/></w:style>
  <w:style w:type="paragraph" w:styleId="PlainHeading"><w:name w:val="PlainHeading"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
</w:styles>"#;

const NUMBERING: &str = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="multilevel"/>
    <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="480" w:hanging="240"/></w:pPr></w:lvl>
    <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="960" w:hanging="360"/></w:pPr></w:lvl>
  </w:abstractNum>
  <w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperLetter"/><w:lvlText w:val="%1)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="600" w:hanging="240"/></w:pPr></w:lvl></w:abstractNum>
  <w:num w:numId="7"><w:abstractNumId w:val="1"/></w:num>
  <w:num w:numId="8"><w:abstractNumId w:val="2"/></w:num>
</w:numbering>"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Default,
    Normal,
    ListBase,
    ListChild,
    PlainHeading,
}

impl Style {
    fn id(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Normal => Some("Normal"),
            Self::ListBase => Some("ListBase"),
            Self::ListChild => Some("ListChild"),
            Self::PlainHeading => Some("PlainHeading"),
        }
    }

    fn numbered(self) -> bool {
        matches!(self, Self::ListBase | Self::ListChild)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Numbering {
    Inherit,
    Off,
    List(u8, u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Paragraph {
    text: String,
    style: Style,
    numbering: Numbering,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State([Paragraph; 4]);

impl Default for State {
    fn default() -> Self {
        Self([
            Paragraph {
                text: "first 日本語 ab 😀 ".repeat(3),
                style: Style::Normal,
                numbering: Numbering::Inherit,
            },
            Paragraph {
                text: "inherited 日本語 e\u{301}".into(),
                style: Style::ListChild,
                numbering: Numbering::Inherit,
            },
            Paragraph {
                text: "later page 日本語 ab".into(),
                style: Style::Normal,
                numbering: Numbering::List(7, 1),
            },
            Paragraph {
                text: String::new(),
                style: Style::ListChild,
                numbering: Numbering::Inherit,
            },
        ])
    }
}

impl State {
    fn package(&self) -> Vec<u8> {
        // Only plain model values feed the oracle; no SUT export or command
        // implementation is used to construct the expected document.
        let mut body = String::new();
        for (index, paragraph) in self.0.iter().enumerate() {
            body.push_str("<w:p><w:pPr>");
            if let Some(style) = paragraph.style.id() {
                body.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
            }
            match paragraph.numbering {
                Numbering::Inherit => {}
                Numbering::Off => body.push_str("<w:numPr><w:numId w:val=\"0\"/></w:numPr>"),
                Numbering::List(id, level) => body.push_str(&format!(
                    "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{id}\"/></w:numPr>"
                )),
            }
            if index == 2 {
                body.push_str("<w:pageBreakBefore/>");
            }
            let text = paragraph
                .text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            body.push_str(&format!(
                "</w:pPr><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>"
            ));
        }
        let mut input = zip::ZipArchive::new(Cursor::new(package(&body, ""))).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..input.len() {
            let mut part = input.by_index(index).unwrap();
            let name = part.name().unwrap().into_owned();
            let mut xml = String::new();
            part.read_to_string(&mut xml).unwrap();
            if name == "[Content_Types].xml" {
                xml = xml.replace("</Types>", r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#);
            } else if name == "word/_rels/document.xml.rels" {
                xml = xml.replace("</Relationships>", r#"<Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="numbering" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#);
            }
            output
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(xml.as_bytes()).unwrap();
        }
        for (name, xml) in [
            ("word/styles.xml", STYLES),
            ("word/numbering.xml", NUMBERING),
        ] {
            output
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(xml.as_bytes()).unwrap();
        }
        output.finish().unwrap().into_inner()
    }

    fn check_state(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 4, "{context}: paragraphs");
        for (index, paragraph) in self.0.iter().enumerate() {
            let props = document
                .word_document()
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref();
            assert_eq!(
                document.paragraph_text(index).as_deref(),
                Some(paragraph.text.as_str()),
                "{context}: paragraph {index} text"
            );
            assert_eq!(
                props.and_then(|props| props.style.as_deref()),
                paragraph.style.id(),
                "{context}: paragraph {index} style"
            );
            let expected = match paragraph.numbering {
                Numbering::Inherit if paragraph.style.numbered() => Some(("7".into(), Some(0))),
                Numbering::Inherit => None,
                Numbering::Off => Some(("0".into(), None)),
                Numbering::List(id, level) => Some((id.to_string(), Some(u32::from(level)))),
            };
            assert_eq!(
                props.and_then(|props| props.numbering.as_ref()).map(|num| (
                    num.id.clone().unwrap_or_default(),
                    if num.id.as_deref() == Some("0") {
                        None
                    } else {
                        num.level
                    }
                )),
                expected,
                "{context}: paragraph {index} effective numbering"
            );
        }
    }

    fn check_render(&self, actual: &mut Doc, context: &str) {
        let mut expected = open_bytes(&self.package());
        expected.set_selection_range(actual.selection_range());
        expected.repaint_all().unwrap();
        assert_eq!(
            actual.selection_range(),
            expected.selection_range(),
            "{context}: reference selection"
        );
        assert_eq!(
            actual.page_count(),
            expected.page_count(),
            "{context}: pages"
        );
        for page in 0..expected.page_count() {
            assert_page_pixels(actual, &expected, page, context);
        }
        for (paragraph, model) in self.0.iter().enumerate() {
            for offset in [0, model.text.len()] {
                let have = actual
                    .position_rect(paragraph, offset)
                    .expect("actual caret");
                let want = expected
                    .position_rect(paragraph, offset)
                    .expect("expected caret");
                for (have, want) in [
                    (have.x, want.x),
                    (have.document_y, want.document_y),
                    (have.height, want.height),
                ] {
                    assert!(
                        (have - want).abs() < COORD_EPSILON,
                        "{context}: caret {paragraph}:{offset}: {have} != {want}"
                    );
                }
            }
        }
        assert_eq!(
            actual.renderer().selection_range(),
            actual.selection_range(),
            "{context}: geometry preserves selection"
        );
    }
}

#[derive(Default)]
struct Model {
    state: State,
    undo: Vec<State>,
    redo: Vec<State>,
}

impl Model {
    fn change(&mut self, next: State) {
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
            document.renderer().selection_range(),
            document.selection_range(),
            "{context}: selection"
        );
        self.state.check_render(document, context);
    }
}

#[derive(Clone, Copy, Debug)]
struct Range(u8, u8, bool, bool);

impl Range {
    fn select(self, state: &State, document: &mut Doc) -> (usize, usize) {
        let first = usize::from(self.0.min(self.1));
        let last = usize::from(self.0.max(self.1));
        let mut anchor = (first, 0);
        let mut focus = (last, if self.3 { 0 } else { state.0[last].text.len() });
        if self.2 {
            std::mem::swap(&mut anchor, &mut focus);
        }
        document.select(anchor, focus, usize::MAX).unwrap();
        (first, last)
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Restyle(Range, Style),
    List(Range, Option<u8>, u8),
    Append(u8, String),
    Reject(bool),
    FailedBatch,
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Restyle(range, style) => {
            let (first, last) = range.select(&model.state, document);
            let selection = document.selection_range();
            // Include raw whitespace and clearing at the native public boundary.
            let input = style.id().map_or("  ".into(), |id| format!("  {id}  "));
            document.set_paragraph_style(&input, usize::MAX).unwrap();
            let mut next = model.state.clone();
            for paragraph in &mut next.0[first..=last] {
                paragraph.style = *style;
            }
            // Explicit style/list assignments record a command even if repeated.
            model.change(next);
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: style selection"
            );
        }
        Operation::List(range, id, level) => {
            let (first, last) = range.select(&model.state, document);
            let selection = document.selection_range();
            let id_string = id.map(|id| id.to_string());
            if *level == 0 {
                document
                    .set_numbering(id_string.as_deref(), usize::MAX)
                    .unwrap();
            } else {
                document
                    .apply_command(
                        HeadlessCommand::SetNumbering {
                            start_paragraph: last,
                            end_paragraph: first,
                            num_id: id_string,
                            level: id.map(|_| u32::from(*level)),
                        },
                        true,
                    )
                    .unwrap();
            }
            let mut next = model.state.clone();
            for paragraph in &mut next.0[first..=last] {
                paragraph.numbering = match id {
                    Some(id) => Numbering::List(*id, *level),
                    None if paragraph.style.numbered() => Numbering::Off,
                    None => Numbering::Inherit,
                };
            }
            model.change(next);
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: list selection"
            );
        }
        Operation::Append(paragraph, text) => {
            let paragraph = usize::from(*paragraph);
            let offset = model.state.0[paragraph].text.len();
            document
                .select((paragraph, offset), (paragraph, offset), usize::MAX)
                .unwrap();
            document
                .insert_text(paragraph, offset, text, usize::MAX)
                .unwrap();
            if !text.is_empty() {
                let mut next = model.state.clone();
                next.0[paragraph].text.push_str(text);
                model.change(next);
            }
        }
        Operation::Reject(style) => {
            Range(0, 3, true, false).select(&model.state, document);
            let selection = document.selection_range();
            let command = if *style {
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 0,
                    end_paragraph: 4,
                    style_id: Some("PlainHeading".into()),
                }
            } else {
                HeadlessCommand::SetNumbering {
                    start_paragraph: 4,
                    end_paragraph: 0,
                    num_id: Some("8".into()),
                    level: Some(0),
                }
            };
            let error = document.apply_command(command, true).expect_err(&context);
            assert_eq!(
                error, "edit failed: Command { index: 0, source: ParagraphUnavailable }",
                "{context}: rejection"
            );
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: rejected selection"
            );
        }
        Operation::FailedBatch => {
            Range(0, 3, true, true).select(&model.state, document);
            let selection = document.selection_range();
            let error = document
                .apply_commands(
                    vec![
                        HeadlessCommand::SetParagraphStyle {
                            start_paragraph: 3,
                            end_paragraph: 0,
                            style_id: Some("PlainHeading".into()),
                        },
                        HeadlessCommand::SetNumbering {
                            start_paragraph: 0,
                            end_paragraph: 3,
                            num_id: Some("8".into()),
                            level: Some(0),
                        },
                        HeadlessCommand::SetParagraphStyle {
                            start_paragraph: 4,
                            end_paragraph: 4,
                            style_id: Some("Normal".into()),
                        },
                    ],
                    false,
                )
                .expect_err(&context);
            assert_eq!(
                error, "edit failed: Command { index: 2, source: ParagraphUnavailable }",
                "{context}: rollback error"
            );
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: rollback selection"
            );
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
    assert_eq!(document.page_count(), 2, "fixture spans two pages");
    model.check(&mut document, "initial");
    for (index, operation) in operations.iter().enumerate() {
        step(&mut model, &mut document, operation, index);
    }
    let mut reopened = open_bytes(&document.export_docx().unwrap());
    model.state.check_state(&reopened, "export/reopen");
    model.state.check_render(&mut reopened, "export/reopen");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
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
    assert_eq!(model.state, State::default(), "full undo");
    while !model.redo.is_empty() {
        step(&mut model, &mut document, &Operation::Redo, index);
        index += 1;
    }
    assert_eq!(model.state, tip, "full redo");
}

#[test]
fn undo_typing_restores_empty_styled_paragraph() {
    // Rebuilding the restored empty run used to prune its <w:t>, and the run
    // itself, losing the inherited font size both on screen and after saving.
    for style in [Style::Normal, Style::ListChild, Style::PlainHeading] {
        check_sequence(&[
            Operation::Restyle(Range(3, 3, false, false), style),
            Operation::Append(3, "日".into()),
            Operation::Undo,
        ]);
    }
}

#[test]
fn style_numbering_can_be_disabled_and_restored_across_pages() {
    use Operation::*;
    check_sequence(&[
        List(Range(1, 3, true, true), None, 0),
        Restyle(Range(0, 3, false, false), Style::ListBase),
        Append(3, "空段落からの入力 😀".into()),
        Undo,
        Undo,
        Undo,
        Redo,
        Redo,
        Redo,
        Restyle(Range(1, 3, true, true), Style::Normal),
        List(Range(0, 3, false, false), None, 0),
    ]);
}

#[test]
fn direct_lists_override_style_changes_and_survive_typing() {
    use Operation::*;
    check_sequence(&[
        Restyle(Range(1, 1, false, false), Style::Normal),
        List(Range(0, 3, true, true), Some(8), 0),
        Restyle(Range(1, 2, false, false), Style::ListChild),
        Append(1, " & <xml> e\u{301}".into()),
        List(Range(0, 2, true, false), Some(7), 1),
        Restyle(Range(0, 3, false, true), Style::PlainHeading),
        Append(3, "new heading".into()),
        Restyle(Range(0, 3, true, false), Style::Default),
    ]);
}

#[test]
fn list_style_groups_and_rejected_batches_preserve_redo() {
    use Operation::*;
    check_sequence(&[
        Restyle(Range(0, 2, true, true), Style::ListChild),
        List(Range(1, 3, false, false), Some(8), 0),
        Append(2, "日本語".into()),
        Merge(0),
        Undo,
        FailedBatch,
        Reject(true),
        Reject(false),
        Redo,
        Undo,
        Append(3, String::new()),
        Redo,
        Restyle(Range(0, 3, false, false), Style::PlainHeading),
        Undo,
        List(Range(0, 3, true, true), None, 0),
    ]);
}

#[test]
fn repeated_style_and_list_assignments_round_trip_through_history() {
    use Operation::*;
    check_sequence(&[
        Restyle(Range(1, 1, false, false), Style::ListChild),
        Restyle(Range(1, 1, false, false), Style::ListChild),
        List(Range(2, 2, false, false), Some(7), 1),
        List(Range(2, 2, false, false), Some(7), 1),
        List(Range(1, 1, false, false), None, 0),
        List(Range(1, 1, false, false), None, 0),
        Undo,
        Undo,
        Redo,
        Append(1, String::new()),
        Redo,
    ]);
}

fn operation() -> impl Strategy<Value = Operation> {
    let range = (0u8..4, 0u8..4, any::<bool>(), any::<bool>())
        .prop_map(|(a, b, reverse, end_at_zero)| Range(a, b, reverse, end_at_zero));
    let style = prop::sample::select(vec![
        Style::Default,
        Style::Normal,
        Style::ListBase,
        Style::ListChild,
        Style::PlainHeading,
    ]);
    let list = prop::sample::select(vec![(None, 0), (Some(7), 0), (Some(7), 1), (Some(8), 0)]);
    prop_oneof![
        5 => (range.clone(), style).prop_map(|(range, style)| Operation::Restyle(range, style)),
        5 => (range, list).prop_map(|(range, (id, level))| Operation::List(range, id, level)),
        3 => (0u8..4, prop::sample::select(vec![String::new(), "日".into(), "😀e\u{301}".into(), " & <xml> ".into(), "折り返し ab ".repeat(3)]))
            .prop_map(|(paragraph, text)| Operation::Append(paragraph, text)),
        1 => any::<bool>().prop_map(Operation::Reject),
        1 => Just(Operation::FailedBatch),
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/numbering_style_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn numbering_and_styles_follow_an_independent_history_model(
        operations in prop::collection::vec(operation(), 1..=16)
    ) {
        check_sequence(&operations);
    }
}
