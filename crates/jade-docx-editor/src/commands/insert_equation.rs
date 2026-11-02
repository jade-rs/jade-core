//! Native OMML equation insertion with atomic history.

use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{EquationPlacement, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

pub struct InsertEquationCommand {
    range: SelectionRange,
    omml: String,
    placement: EquationPlacement,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertEquationCommand {
    pub fn new(range: SelectionRange, omml: String, placement: EquationPlacement) -> Self {
        Self {
            range,
            omml,
            placement,
            previous_document: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertEquationCommand {
    fn name(&self) -> &'static str {
        "insert_equation"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "equation insertion requires a collapsed selection".into(),
            ));
        }
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;

        match ctx
            .document
            .insert_equation_at(self.range.focus, &self.omml, self.placement)
        {
            Ok(caret) => {
                self.previous_document = Some(snapshot);
                self.previous_selection = previous_selection;
                *ctx.selection = Some(SelectionRange::collapsed(caret));
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("equation history is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::insert_text::InsertTextCommand;
    use crate::commands::CommandExecutor;
    use crate::headless_api::apply_headless_commands_with_executor_to_document;
    use crate::word_document::{WordDocument, MAX_AUTHORED_OMML_BYTES};
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest, HeadlessEquationPlacement};
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const INLINE: &str = r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath>"#;
    const BLOCK: &str = r#"<m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMath><m:r><m:t>x=1</m:t></m:r></m:oMath></m:oMathPara>"#;

    fn escape_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx(text: &str) -> Vec<u8> {
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p></w:body></w:document>"#,
            escape_text(text)
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn part(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
        let mut entry = archive.by_name(path).expect("part");
        let mut value = String::new();
        entry.read_to_string(&mut value).expect("read part");
        value
    }

    fn apply(
        document: &mut WordDocument,
        executor: &mut CommandExecutor,
        selection: &mut Option<SelectionRange>,
        omml: &str,
        placement: EquationPlacement,
    ) -> CommandResult<()> {
        let range = selection.expect("selection");
        let mut context = CommandContext::new(document, selection);
        executor.execute(
            Box::new(InsertEquationCommand::new(
                range,
                omml.to_string(),
                placement,
            )),
            &mut context,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn inline_equation_round_trips_across_arbitrary_unicode_run_boundaries(
            chars in proptest::collection::vec(
                prop::sample::select(vec!['a', 'Z', '0', ' ', 'あ', '界', '😀', 'é']),
                0..24,
            ),
            boundary_pick in any::<usize>(),
        ) {
            let text = chars.into_iter().collect::<String>();
            let mut boundaries = text.char_indices().map(|(index, _)| index).collect::<Vec<_>>();
            boundaries.push(text.len());
            let offset = boundaries[boundary_pick % boundaries.len()];
            let source = build_docx(&text);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load generated document");
            let baseline = document.export_docx().expect("baseline");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, offset)));
            let mut command_executor = CommandExecutor::new();

            apply(&mut document, &mut command_executor, &mut selection, INLINE, EquationPlacement::Inline)
                .expect("insert inline equation");
            let paragraph = document.paragraph(0).expect("paragraph");
            prop_assert_eq!(paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>(), text);
            prop_assert_eq!(paragraph.runs.iter().filter(|run| run.math.is_some()).count(), 1);

            let applied = document.export_docx().expect("export equation");
            let xml = part(&applied, "word/document.xml");
            prop_assert_eq!(xml.matches("<m:oMath").count(), 1);
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen equation");
            prop_assert_eq!(reopened.paragraph(0).expect("reopened paragraph").runs.iter().filter(|run| run.math.is_some()).count(), 1);

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.undo(&mut context).expect("undo"));
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.redo(&mut context).expect("redo"));
            }
            prop_assert_eq!(document.export_docx().expect("export redo"), applied);
        }
    }

    mod simple_field_history {
        use super::*;

        #[derive(Clone, Debug)]
        enum Operation {
            InsertEquation(&'static str),
            Append(&'static str),
            DeleteSuffix,
            Undo,
            Redo,
            Reopen,
            RejectMalformedEquation,
        }

        #[derive(Clone, Debug, Default, PartialEq)]
        struct Model {
            equations: Vec<String>,
            suffix: String,
        }

        impl Model {
            fn rendered_end(&self) -> usize {
                // Public offsets count each inline equation as the UTF-8
                // object replacement character, even though its run has no text.
                6 + self.suffix.len() + self.equations.len() * '\u{fffc}'.len_utf8()
            }
        }

        fn source() -> Vec<u8> {
            let bytes = build_docx("");
            let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("source zip");
            let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for index in 0..input.len() {
                let mut entry = input.by_index(index).expect("source part");
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).expect("read part");
                let name = entry.name().expect("valid source part name").into_owned();
                if name == "word/document.xml" {
                    let xml = String::from_utf8(bytes).expect("document XML");
                    let empty = r#"<w:r><w:t xml:space="preserve"></w:t></w:r>"#;
                    assert_eq!(xml.matches(empty).count(), 1);
                    bytes = xml.replace(empty, r#"<w:r><w:t>ab</w:t></w:r><w:fldSimple w:instr=" HYPERLINK &quot;https://example.test&quot; "><w:r><w:t>c</w:t></w:r><w:r><w:t>d</w:t></w:r><w:r><w:t>ef</w:t></w:r></w:fldSimple>"#).into_bytes();
                }
                output
                    .start_file(name, SimpleFileOptions::default())
                    .expect("write part");
                output.write_all(&bytes).expect("part bytes");
            }
            output.finish().expect("finish source").into_inner()
        }

        fn observed(document: &WordDocument) -> (String, Vec<String>) {
            let paragraph = document.paragraph(0).expect("one paragraph");
            (
                paragraph.runs.iter().map(|run| run.text.as_str()).collect(),
                paragraph
                    .runs
                    .iter()
                    .filter_map(|run| run.math.as_ref())
                    .map(|math| math.preview_text(100).expect("equation value"))
                    .collect(),
            )
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 512, ..ProptestConfig::default() })]
            #[test]
            fn simple_field_equation_edits_match_a_state_machine_after_every_saved_operation(
                operations in prop::collection::vec(prop_oneof![
                    4 => prop::sample::select(vec!["x", "y", "z", "a+b"]).prop_map(Operation::InsertEquation),
                    2 => prop::sample::select(vec!["A", "日", "&<>"]).prop_map(Operation::Append),
                    2 => Just(Operation::DeleteSuffix),
                    2 => Just(Operation::Undo),
                    2 => Just(Operation::Redo),
                    1 => Just(Operation::Reopen),
                    1 => Just(Operation::RejectMalformedEquation),
                ], 1..33),
            ) {
                // The model knows values and ordinary text only. It shares no
                // source node IDs, run matching, OMML parser or snapshot logic.
                let mut document = executor::block_on(WordDocument::load(&source(), &Options::default())).expect("load field");
                let mut command_executor = CommandExecutor::new();
                let mut selection = None;
                let mut model = Model::default();
                let mut undo = Vec::new();
                let mut redo = Vec::new();
                for (step, operation) in operations.iter().enumerate() {
                    let before = model.clone();
                    let mut command = None;
                    match operation {
                        Operation::InsertEquation(value) => {
                            model.equations.insert(0, (*value).to_string());
                            command = Some(HeadlessCommand::InsertEquation {
                                paragraph: 0, offset: 3,
                                omml: format!("<m:oMath><m:r><m:t>{value}</m:t></m:r></m:oMath>"),
                                placement: HeadlessEquationPlacement::Inline,
                            });
                        }
                        Operation::Append(text) => {
                            command = Some(HeadlessCommand::InsertText { paragraph: 0, offset: model.rendered_end(), text: (*text).to_string() });
                            model.suffix.push_str(text);
                        }
                        Operation::DeleteSuffix => {
                            command = Some(HeadlessCommand::DeleteText {
                                paragraph: 0, offset: model.rendered_end(), count: 1,
                                direction: if model.suffix.is_empty() { jade_docx_headless_api::HeadlessDeleteDirection::Forward } else { jade_docx_headless_api::HeadlessDeleteDirection::Backward },
                            });
                            model.suffix.pop();
                        }
                        Operation::Undo => {
                            let expected = if let Some(previous) = undo.pop() { redo.push(model.clone()); model = previous; true } else { false };
                            let actual = command_executor.undo(&mut CommandContext::new(&mut document, &mut selection)).expect("undo");
                            prop_assert_eq!(actual, expected, "step {} {:?}", step, operation);
                        }
                        Operation::Redo => {
                            let expected = if let Some(next) = redo.pop() { undo.push(model.clone()); model = next; true } else { false };
                            let actual = command_executor.redo(&mut CommandContext::new(&mut document, &mut selection)).expect("redo");
                            prop_assert_eq!(actual, expected, "step {} {:?}", step, operation);
                        }
                        Operation::Reopen => {
                            document = executor::block_on(WordDocument::load(&document.export_docx().expect("save"), &Options::default())).expect("reopen");
                            command_executor = CommandExecutor::new();
                            selection = None;
                            undo.clear(); redo.clear();
                        }
                        Operation::RejectMalformedEquation => {
                            let bytes = document.export_docx().expect("before rejection");
                            let result = apply_headless_commands_with_executor_to_document(&mut document, &mut command_executor, selection, &HeadlessEditRequest {
                                commands: vec![HeadlessCommand::InsertEquation { paragraph: 0, offset: 3, omml: "<m:oMath>".to_string(), placement: HeadlessEquationPlacement::Inline }],
                            });
                            prop_assert!(result.is_err(), "step {} malformed equation accepted", step);
                            prop_assert_eq!(document.export_docx().expect("after rejection"), bytes, "step {} rejection changed package", step);
                        }
                    }
                    if let Some(command) = command {
                        selection = apply_headless_commands_with_executor_to_document(&mut document, &mut command_executor, selection, &HeadlessEditRequest { commands: vec![command] }).expect("public edit");
                        if model != before { undo.push(before); redo.clear(); }
                    }
                    let expected = (format!("abcdef{}", model.suffix), model.equations.clone());
                    prop_assert_eq!(observed(&document), expected.clone(), "step {} {:?} live", step, operation);
                    prop_assert_eq!(command_executor.undo_depth(), undo.len(), "step {} {:?} history", step, operation);
                    prop_assert_eq!(command_executor.can_redo(), !redo.is_empty(), "step {} {:?} redo", step, operation);
                    let bytes = document.export_docx().expect("save each operation");
                    let reopened = executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("read each operation");
                    prop_assert_eq!(observed(&reopened), expected, "step {} {:?} saved", step, operation);
                    prop_assert_eq!(part(&bytes, "word/document.xml").matches("<w:fldSimple ").count(), 1, "step {} {:?} field retained", step, operation);
                }
            }
        }
    }

    #[test]
    fn block_equation_splits_paragraph_and_reopens_without_phantom_coordinate() {
        let source = build_docx("beforeafter");
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 6)));
        let mut executor = CommandExecutor::new();
        apply(
            &mut document,
            &mut executor,
            &mut selection,
            BLOCK,
            EquationPlacement::Block,
        )
        .expect("insert block equation");
        assert_eq!(document.paragraph_count(), 3);
        assert_eq!(document.paragraph(0).unwrap().runs[0].text, "before");
        assert!(document.paragraph(1).is_none());
        assert_eq!(document.paragraph(2).unwrap().runs[0].text, "after");
        assert_eq!(selection.unwrap().focus, TextPosition::new(2, 0));
        let body = document.document.as_ref().unwrap().body.as_ref().unwrap();
        assert!(matches!(
            body.elements.as_slice(),
            [
                crate::document::BodyElement::Paragraph(_),
                crate::document::BodyElement::Math(_),
                crate::document::BodyElement::Paragraph(_)
            ]
        ));
        assert_eq!(body.elements.len(), 3);
        assert_eq!(document.document.as_ref().unwrap().sections[0].end, 3);

        {
            let range = selection.expect("tail selection");
            let mut context = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "tail:".to_string(), None)),
                    &mut context,
                )
                .expect("insert into the paragraph after the equation");
        }
        assert_eq!(document.paragraph(2).unwrap().runs[0].text, "tail:after");

        let exported = document.export_docx().expect("export block equation");
        let xml = part(&exported, "word/document.xml");
        assert!(xml.contains("<w:p><m:oMathPara"));
        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen block equation");
        assert_eq!(reopened.paragraph_count(), 3);
        assert_eq!(reopened.paragraph(0).unwrap().runs[0].text, "before");
        assert!(reopened.paragraph(1).is_none());
        assert_eq!(reopened.paragraph(2).unwrap().runs[0].text, "tail:after");
        let body = reopened.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(
            body.elements
                .iter()
                .filter(|element| matches!(element, crate::document::BodyElement::Math(_)))
                .count(),
            1
        );
    }

    #[test]
    fn rejected_equations_are_atomic_and_do_not_enter_history() {
        let source = build_docx("あA");
        let invalid = [
            ("", EquationPlacement::Inline),
            ("<m:oMath>", EquationPlacement::Inline),
            (BLOCK, EquationPlacement::Inline),
            (INLINE, EquationPlacement::Block),
            ("<!DOCTYPE x><m:oMath/>", EquationPlacement::Inline),
        ];
        for (omml, placement) in invalid {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::collapsed(TextPosition::new(0, 3));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            assert!(apply(
                &mut document,
                &mut executor,
                &mut selection,
                omml,
                placement
            )
            .is_err());
            assert_eq!(document.export_docx().expect("rejected export"), baseline);
            assert!(!executor.can_undo());
            assert_eq!(selection, Some(range));
        }

        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let baseline = document.export_docx().expect("baseline");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 3)));
        let mut executor = CommandExecutor::new();
        let oversized = format!("<m:oMath>{}</m:oMath>", "x".repeat(MAX_AUTHORED_OMML_BYTES));
        assert!(apply(
            &mut document,
            &mut executor,
            &mut selection,
            &oversized,
            EquationPlacement::Inline
        )
        .is_err());
        assert_eq!(document.export_docx().expect("rejected export"), baseline);
        assert!(!executor.can_undo());
    }

    #[test]
    fn headless_command_sequence_uses_the_post_equation_paragraph_coordinates() {
        let source = build_docx("beforeafter");
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let mut executor = CommandExecutor::new();
        let selection = apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertEquation {
                        paragraph: 0,
                        offset: 6,
                        omml: BLOCK.to_string(),
                        placement: HeadlessEquationPlacement::Block,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 2,
                        offset: 0,
                        text: "tail:".to_string(),
                    },
                ],
            },
        )
        .expect("apply command sequence");

        assert_eq!(document.paragraph(0).unwrap().runs[0].text, "before");
        assert_eq!(document.paragraph(2).unwrap().runs[0].text, "tail:after");
        assert_eq!(selection.unwrap().focus, TextPosition::new(2, 5));
        let reopened = executor::block_on(WordDocument::load(
            &document.export_docx().expect("export command sequence"),
            &Options::default(),
        ))
        .expect("reopen command sequence");
        assert_eq!(reopened.paragraph(2).unwrap().runs[0].text, "tail:after");
    }

    #[test]
    fn second_block_equation_uses_body_paragraph_after_existing_math() {
        let source = build_docx("beforeafter");
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 6)));
        let mut command_executor = CommandExecutor::new();
        apply(
            &mut document,
            &mut command_executor,
            &mut selection,
            BLOCK,
            EquationPlacement::Block,
        )
        .expect("insert first equation");
        assert_eq!(selection.unwrap().focus, TextPosition::new(2, 0));

        selection = Some(SelectionRange::collapsed(TextPosition::new(2, 5)));
        apply(
            &mut document,
            &mut command_executor,
            &mut selection,
            BLOCK,
            EquationPlacement::Block,
        )
        .expect("insert second equation");
        assert_eq!(selection.unwrap().focus, TextPosition::new(4, 0));
        assert_eq!(document.paragraph_count(), 5);
        assert_eq!(document.paragraph(0).unwrap().runs[0].text, "before");
        assert!(document.paragraph(1).is_none());
        assert_eq!(document.paragraph(2).unwrap().runs[0].text, "after");
        assert!(document.paragraph(3).is_none());
        assert!(document.paragraph(4).unwrap().runs.is_empty());

        let reopened = executor::block_on(WordDocument::load(
            &document.export_docx().expect("export equations"),
            &Options::default(),
        ))
        .expect("reopen equations");
        assert_eq!(reopened.paragraph_count(), 5);
        assert_eq!(reopened.paragraph(2).unwrap().runs[0].text, "after");
        assert!(reopened.paragraph(4).unwrap().runs.is_empty());
    }
}
