//! Paragraph properties are checked against their public units, including DOCX round trips.

use super::{assert_page_pixels, open_bytes, package, Doc, COORD_EPSILON};
use jade_docx_headless_api::HeadlessCommand;
use proptest::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Paragraph {
    text: String,
    alignment: &'static str,
    indent: Option<i16>,
    // Hundredths of a single line, independent of the implementation's floats.
    line: Option<u16>,
    before: Option<u16>,
    after: Option<u16>,
    shade: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State([Paragraph; 3]);

impl Default for State {
    fn default() -> Self {
        Self(
            [
                "first 日本語 ab 😀 ".repeat(3),
                String::new(),
                "last 日本語 e\u{301} ab ".repeat(4),
            ]
            .map(|text| Paragraph {
                text,
                alignment: "left",
                indent: None,
                line: None,
                before: None,
                after: None,
                shade: None,
            }),
        )
    }
}

impl State {
    fn package(&self) -> Vec<u8> {
        // Build OOXML from plain model values. Never export/copy the actual
        // document or use its formatting commands to construct the oracle.
        let mut body = String::new();
        for paragraph in &self.0 {
            body.push_str("<w:p><w:pPr>");
            let alignment = match paragraph.alignment {
                "justify" => "both",
                other => other,
            };
            body.push_str(&format!("<w:jc w:val=\"{alignment}\"/>"));
            if let Some(points) = paragraph.indent {
                body.push_str(&format!("<w:ind w:left=\"{}\"/>", i32::from(points) * 20));
            }
            body.push_str("<w:spacing");
            if let Some(line) = paragraph.line {
                body.push_str(&format!(
                    " w:line=\"{}\" w:lineRule=\"auto\"",
                    u32::from(line) * 240 / 100
                ));
            }
            for (name, value) in [("before", paragraph.before), ("after", paragraph.after)] {
                if let Some(points) = value {
                    body.push_str(&format!(" w:{name}=\"{}\"", u32::from(points) * 20));
                }
            }
            body.push_str("/>");
            if let Some(fill) = paragraph.shade {
                body.push_str(&format!(
                    "<w:shd w:val=\"clear\" w:fill=\"{}\"/>",
                    &fill[1..]
                ));
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
        package(&body, "")
    }

    fn check_state(&self, document: &Doc, context: &str) {
        assert_eq!(document.paragraph_count(), 3, "{context}: paragraph count");
        for (index, expected) in self.0.iter().enumerate() {
            let context = format!("{context}: paragraph {index}");
            assert_eq!(
                document.paragraph_text(index).as_deref(),
                Some(expected.text.as_str()),
                "{context}: text"
            );
            let props = document
                .word_document()
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref();
            assert_eq!(
                props
                    .and_then(|props| props.alignment.as_deref())
                    .unwrap_or("left"),
                expected.alignment,
                "{context}: alignment"
            );
            let indent = props.and_then(|props| props.indent.as_ref());
            assert_eq!(
                indent.and_then(|indent| indent.left),
                expected.indent.map(f32::from),
                "{context}: indent"
            );
            let spacing = props.and_then(|props| props.spacing.as_ref());
            assert_eq!(
                spacing.and_then(|spacing| spacing.line),
                expected.line.map(|line| f32::from(line) / 100.0),
                "{context}: line multiplier"
            );
            assert_eq!(
                spacing.and_then(|spacing| spacing.line_rule.as_deref()),
                expected.line.map(|_| "auto"),
                "{context}: line rule"
            );
            assert_eq!(
                spacing.and_then(|spacing| spacing.before),
                expected.before.map(f32::from),
                "{context}: space before"
            );
            assert_eq!(
                spacing.and_then(|spacing| spacing.after),
                expected.after.map(f32::from),
                "{context}: space after"
            );
            assert_eq!(
                spacing.and_then(|spacing| spacing.before_lines),
                None,
                "{context}: before lines"
            );
            assert_eq!(
                spacing.and_then(|spacing| spacing.after_lines),
                None,
                "{context}: after lines"
            );
            let shading = props.and_then(|props| props.shading.as_ref());
            assert_eq!(
                shading.and_then(|shading| shading.fill.as_deref()),
                expected.shade,
                "{context}: shading"
            );
            assert_eq!(
                shading.and_then(|shading| shading.val.as_deref()),
                expected.shade.map(|_| "clear"),
                "{context}: shading pattern"
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
    fn change(&mut self, next: State, force_record: bool) {
        if next != self.state || force_record {
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
        let start = usize::from(self.0.min(self.1)) % 3;
        let end = usize::from(self.0.max(self.1)) % 3;
        let first = state.0[start].text.chars().next().map_or(0, char::len_utf8);
        let end_offset = if self.3 { 0 } else { state.0[end].text.len() };
        let mut anchor = (start, first);
        let mut focus = (end, end_offset);
        if self.2 {
            std::mem::swap(&mut anchor, &mut focus);
        }
        // Rejections and no-ops preserve the most recent paint, so paint the
        // selection explicitly before issuing a command.
        document.select(anchor, focus, usize::MAX).unwrap();
        (start, end)
    }
}

#[derive(Clone, Debug)]
enum Setting {
    Alignment(&'static str, &'static str),
    Indent(i16),
    Line(u16),
    ClearLine,
    Spacing(Option<u16>, Option<u16>),
    Shade(Option<&'static str>),
}

#[derive(Clone, Debug)]
enum Operation {
    Format(Range, Setting),
    Append(u8, String),
    Reject(Range, u8),
    FailedBatch,
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Format(range, setting) => {
            let (start, end) = range.select(&model.state, document);
            let selection = document.selection_range();
            let mut next = model.state.clone();
            match setting {
                Setting::Alignment(input, canonical) => {
                    document.set_alignment(input, usize::MAX).unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        paragraph.alignment = canonical;
                    }
                }
                Setting::Indent(points) => {
                    document
                        .shift_indent(f32::from(*points), usize::MAX)
                        .unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        paragraph.indent = Some(*points);
                    }
                }
                Setting::Line(line) => {
                    document
                        .set_line_spacing(f32::from(*line) / 100.0, usize::MAX)
                        .unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        paragraph.line = Some(*line);
                    }
                }
                Setting::ClearLine => {
                    document
                        .apply_commands(
                            vec![HeadlessCommand::SetParagraphLineSpacing {
                                start_paragraph: start,
                                end_paragraph: end,
                                line: Some(None),
                                line_rule: None,
                                before: None,
                                after: None,
                            }],
                            false,
                        )
                        .unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        paragraph.line = None;
                    }
                }
                Setting::Spacing(before, after) => {
                    document
                        .set_paragraph_spacing(
                            before.map(f32::from),
                            after.map(f32::from),
                            usize::MAX,
                        )
                        .unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        if before.is_some() {
                            paragraph.before = *before;
                        }
                        if after.is_some() {
                            paragraph.after = *after;
                        }
                    }
                }
                Setting::Shade(color) => {
                    document.set_paragraph_shading(*color, usize::MAX).unwrap();
                    for paragraph in &mut next.0[start..=end] {
                        paragraph.shade = *color;
                    }
                }
            }
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: formatting selection"
            );
            // The public indent setter records explicit assignments even if
            // repeated; the other modeled format setters suppress no-ops.
            model.change(next, matches!(setting, Setting::Indent(_)));
        }
        Operation::Append(paragraph, text) => {
            let paragraph = usize::from(*paragraph) % 3;
            let offset = model.state.0[paragraph].text.len();
            document
                .select((paragraph, offset), (paragraph, offset), usize::MAX)
                .unwrap();
            document
                .insert_text(paragraph, offset, text, usize::MAX)
                .unwrap();
            let mut next = model.state.clone();
            next.0[paragraph].text.push_str(text);
            model.change(next, false);
        }
        Operation::Reject(range, kind) => {
            range.select(&model.state, document);
            let selection = document.selection_range();
            let (result, message) = match kind % 6 {
                0 => (
                    document.set_alignment("  ", usize::MAX),
                    "alignment must be a non-empty string",
                ),
                1 => (
                    document.set_line_spacing(0.0, usize::MAX),
                    "line spacing must be a positive number",
                ),
                2 => (
                    document.set_line_spacing(f32::NAN, usize::MAX),
                    "line spacing must be a positive number",
                ),
                3 => (
                    document.set_paragraph_spacing(Some(-1.0), Some(6.0), usize::MAX),
                    "paragraph spacing before must be a non-negative number",
                ),
                4 => (
                    document.set_paragraph_spacing(Some(6.0), Some(f32::INFINITY), usize::MAX),
                    "paragraph spacing after must be a non-negative number",
                ),
                _ => (
                    document.shift_indent(f32::NAN, usize::MAX),
                    "indent values must be finite numbers",
                ),
            };
            let error = result.expect_err(&context);
            assert!(error.contains(message), "{context}: error {error:?}");
            assert_eq!(
                document.selection_range(),
                selection,
                "{context}: rejected selection"
            );
        }
        Operation::FailedBatch => {
            let selection = document.selection_range();
            assert!(
                document
                    .apply_commands(
                        vec![
                            HeadlessCommand::SetParagraphAlignment {
                                start_paragraph: 0,
                                end_paragraph: 2,
                                alignment: "right".into()
                            },
                            HeadlessCommand::SetParagraphLineSpacing {
                                start_paragraph: 0,
                                end_paragraph: 2,
                                line: Some(Some(2.0)),
                                line_rule: Some("auto".into()),
                                before: Some(18.0),
                                after: Some(6.0)
                            },
                            HeadlessCommand::SetParagraphShading {
                                start_paragraph: 1,
                                end_paragraph: 2,
                                color: Some("#DDEEFF".into())
                            },
                            HeadlessCommand::SetParagraphAlignment {
                                start_paragraph: 3,
                                end_paragraph: 3,
                                alignment: "center".into()
                            },
                        ],
                        false
                    )
                    .is_err(),
                "{context}: unavailable paragraph"
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
fn line_spacing_uses_a_multiplier_and_preserves_paragraph_spacing() {
    let mut document = open_bytes(&package("<w:p><w:r><w:t>日本語 line</w:t></w:r></w:p>", ""));
    document.select((0, 0), (0, 0), usize::MAX).unwrap();
    document
        .set_paragraph_spacing(Some(6.0), Some(18.0), usize::MAX)
        .unwrap();
    document.set_line_spacing(1.5, usize::MAX).unwrap();
    let check = |document: &super::Doc| {
        let spacing = document
            .word_document()
            .paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .spacing
            .as_ref()
            .unwrap();
        assert_eq!(
            spacing.line,
            Some(1.5),
            "line spacing must stay in multiples"
        );
        assert_eq!(spacing.line_rule.as_deref(), Some("auto"));
        assert_eq!(spacing.before, Some(6.0));
        assert_eq!(spacing.after, Some(18.0));
    };
    check(&document);
    check(&open_bytes(&document.export_docx().unwrap()));
}

#[test]
fn paragraph_formats_cover_reversed_ranges_and_empty_paragraphs() {
    use Operation::*;
    use Setting::*;
    check_sequence(&[
        Format(Range(0, 2, true, true), Alignment("  BoTh  ", "justify")),
        Format(Range(0, 2, false, false), Line(150)),
        Format(Range(1, 1, true, true), Spacing(Some(6), Some(18))),
        Format(Range(0, 1, true, false), Indent(18)),
        Format(Range(0, 2, false, false), Shade(Some("#FFF2CC"))),
        Append(1, "empty → 日本語 😀".into()),
        Merge(1),
        Undo,
        Redo,
        Format(Range(2, 2, false, true), Line(115)),
        Format(Range(0, 1, true, true), ClearLine),
        Format(Range(1, 2, true, false), Shade(None)),
        Undo,
        Redo,
    ]);
}

#[test]
fn repeated_formatting_and_empty_spacing_preserve_redo() {
    use Operation::*;
    use Setting::*;
    let all = Range(0, 2, false, false);
    check_sequence(&[
        Format(all, Alignment("center", "center")),
        Format(all, Line(200)),
        Format(all, Shade(Some("#DDEEFF"))),
        Append(2, "pending".into()),
        Undo,
        Format(all, Alignment(" CENTER ", "center")),
        Format(all, Line(200)),
        Format(all, Shade(Some("#DDEEFF"))),
        Format(all, Spacing(None, None)),
        Append(1, String::new()),
        Redo,
        Undo,
        Format(all, Spacing(Some(0), None)),
        Redo,
        Format(all, Shade(None)),
        Format(all, ClearLine),
    ]);
}

#[test]
fn failed_paragraph_format_batches_restore_properties_and_redo() {
    use Operation::*;
    use Setting::*;
    check_sequence(&[
        Format(Range(0, 1, true, false), Alignment("center", "center")),
        Format(Range(1, 2, false, false), Indent(36)),
        Format(Range(0, 2, true, true), Spacing(Some(18), Some(6))),
        Append(1, "pending".into()),
        Undo,
        FailedBatch,
        Redo,
        Format(Range(0, 2, false, false), Line(150)),
        Format(Range(1, 2, true, false), Shade(Some("#FFF2CC"))),
        Merge(0),
        Undo,
        FailedBatch,
        Redo,
    ]);
}

#[test]
fn invalid_paragraph_formats_preserve_selection_history_and_redo() {
    use Operation::*;
    check_sequence(&[
        Append(0, "pending".into()),
        Undo,
        Reject(Range(0, 2, true, true), 0),
        Reject(Range(1, 1, false, false), 1),
        Reject(Range(0, 2, true, false), 2),
        Reject(Range(1, 2, false, true), 3),
        Reject(Range(0, 1, true, false), 4),
        Reject(Range(2, 2, true, true), 5),
        Redo,
    ]);
}

fn operation() -> impl Strategy<Value = Operation> {
    let range = (0u8..3, 0u8..3, any::<bool>(), any::<bool>())
        .prop_map(|(start, end, reverse, at_start)| Range(start, end, reverse, at_start));
    let points = prop::option::of(prop::sample::select(vec![0u16, 6, 18]));
    let setting = prop_oneof![
        prop::sample::select(vec![
            ("left", "left"),
            (" CENTER ", "center"),
            ("right", "right"),
            ("both", "justify")
        ])
        .prop_map(|(input, canonical)| Setting::Alignment(input, canonical)),
        prop::sample::select(vec![0i16, 18, 36]).prop_map(Setting::Indent),
        prop::sample::select(vec![100u16, 115, 150, 200]).prop_map(Setting::Line),
        Just(Setting::ClearLine),
        (points.clone(), points).prop_map(|(before, after)| Setting::Spacing(before, after)),
        prop::sample::select(vec![None, Some("#FFF2CC"), Some("#DDEEFF")]).prop_map(Setting::Shade),
    ];
    prop_oneof![
        8 => (range.clone(), setting).prop_map(|(range, setting)| Operation::Format(range, setting)),
        3 => (0u8..3, prop::sample::select(vec![String::new(), "日".into(), "😀e\u{301}".into(), " & <xml> ".into(), "折り返し ab ".repeat(4)])).prop_map(|(paragraph, text)| Operation::Append(paragraph, text)),
        2 => (range, 0u8..6).prop_map(|(range, kind)| Operation::Reject(range, kind)),
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/paragraph_format_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn paragraph_formats_follow_an_independent_history_model(
        operations in prop::collection::vec(operation(), 1..=16)
    ) {
        check_sequence(&operations);
    }
}
