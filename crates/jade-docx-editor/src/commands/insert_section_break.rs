use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{SectionBreakType, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts a Word section boundary at the caret as one undoable structural
/// edit. The underlying document mutation snapshots the whole package because
/// it changes both parsed section ranges and raw `sectPr` nodes.
pub struct InsertSectionBreakCommand {
    range: SelectionRange,
    break_type: SectionBreakType,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertSectionBreakCommand {
    pub fn new(range: SelectionRange, break_type: SectionBreakType) -> Self {
        Self {
            range,
            break_type,
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertSectionBreakCommand {
    fn name(&self) -> &'static str {
        "insert_section_break"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::UnsupportedSelection);
        }
        let rendered_position = self.range.focus;
        let paragraph = ctx
            .document
            .paragraph(rendered_position.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let model_len = paragraph
            .runs
            .iter()
            .map(|run| run.text.len())
            .sum::<usize>();
        let rendered_len = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(
                rendered_position.paragraph,
                model_len,
            ))
            .offset;
        if rendered_position.offset > rendered_len {
            return Err(CommandError::Custom(format!(
                "offset {} is past the end of paragraph {}, which is {rendered_len} bytes long",
                rendered_position.offset, rendered_position.paragraph
            )));
        }
        let model_position = ctx
            .document
            .rendered_to_model_text_position(rendered_position);
        let text = ctx
            .document
            .paragraph(model_position.paragraph)
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .ok_or(CommandError::ParagraphUnavailable)?;
        if !text.is_char_boundary(model_position.offset) {
            return Err(CommandError::Custom(format!(
                "offset {} is inside a character in paragraph {}",
                rendered_position.offset, rendered_position.paragraph
            )));
        }

        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let following = match ctx.document.insert_section_break_at(
            model_position.paragraph,
            model_position.offset,
            self.break_type,
        ) {
            Ok(following) => following,
            Err(error) => {
                // Some structural failures can only be discovered after the
                // raw paragraph has been split. Keep the command atomic even
                // in that case: a rejected edit must not leave partial XML or
                // shifted paragraph coordinates behind.
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                return Err(CommandError::Custom(error));
            }
        };

        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(following, 0)));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("section-break snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::BodyElement;
    use crate::headless_api::apply_snapshot_headless_commands_with_executor_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest, HeadlessSectionBreakType};
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn build_document_docx(body_xml: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body_xml}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", document_xml.as_str()),
                ("word/_rels/document.xml.rels", DOC_RELS),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn single_section_docx(text: &str) -> Vec<u8> {
        build_document_docx(&format!(
            r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>
<w:sectPr w:rsidR="00112233"><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:cols w:num="2" w:space="720"/><w:docGrid w:type="lines" w:linePitch="360"/><w:printerSettings r:id="rId77"/></w:sectPr>"#
        ))
    }

    fn multi_run_section_docx(first: &str, second: &str, third: &str) -> Vec<u8> {
        build_document_docx(&format!(
            r#"<w:p>
<w:r><w:rPr><w:b/></w:rPr><w:t>{first}</w:t></w:r>
<w:r><w:rPr><w:i/></w:rPr><w:t>{second}</w:t></w:r>
<w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>{third}</w:t></w:r>
</w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>"#
        ))
    }

    fn export_document_xml(document: &WordDocument) -> String {
        let bytes = document.export_docx().expect("export docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .expect("read document.xml");
        xml
    }

    fn paragraph_text(document: &WordDocument, index: usize) -> String {
        document
            .paragraph(index)
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect()
    }

    fn styled_bytes(document: &WordDocument, index: usize) -> Vec<(u8, u8)> {
        document
            .paragraph(index)
            .expect("paragraph")
            .runs
            .iter()
            .flat_map(|run| {
                let style = match run.properties.as_ref() {
                    Some(properties) if properties.bold == Some(true) => 1,
                    Some(properties) if properties.italic == Some(true) => 2,
                    Some(properties) if properties.underline == Some(true) => 3,
                    _ => 0,
                };
                run.text.bytes().map(move |byte| (byte, style))
            })
            .collect()
    }

    fn break_type(index: u8) -> (SectionBreakType, HeadlessSectionBreakType, &'static str) {
        match index % 4 {
            0 => (
                SectionBreakType::NextPage,
                HeadlessSectionBreakType::NextPage,
                "nextPage",
            ),
            1 => (
                SectionBreakType::Continuous,
                HeadlessSectionBreakType::Continuous,
                "continuous",
            ),
            2 => (
                SectionBreakType::EvenPage,
                HeadlessSectionBreakType::EvenPage,
                "evenPage",
            ),
            _ => (
                SectionBreakType::OddPage,
                HeadlessSectionBreakType::OddPage,
                "oddPage",
            ),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn generated_section_breaks_split_preserve_round_trip_and_undo(
            text in "[A-Za-z0-9]{1,48}",
            raw_offset in 0usize..128,
            kind in 0u8..4,
        ) {
            let offset = raw_offset % (text.len() + 1);
            let (break_type, _, token) = break_type(kind);
            let data = single_section_docx(&text);
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let before = export_document_xml(&document);
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut command_executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                command_executor
                    .execute(Box::new(InsertSectionBreakCommand::new(range, break_type)), &mut ctx)
                    .expect("insert section break");
            }

            prop_assert_eq!(paragraph_text(&document, 0), text[..offset].to_string());
            prop_assert_eq!(paragraph_text(&document, 1), text[offset..].to_string());
            prop_assert_eq!(
                selection,
                Some(SelectionRange::collapsed(TextPosition::new(1, 0)))
            );
            let sections = &document.document.as_ref().expect("document").sections;
            prop_assert_eq!(sections.len(), 2);
            prop_assert_eq!(
                sections[1]
                    .properties
                    .as_ref()
                    .and_then(|props| props.section_type.as_deref()),
                Some(token)
            );

            let applied = export_document_xml(&document);
            prop_assert_eq!(applied.matches("<w:sectPr").count(), 2);
            prop_assert_eq!(applied.matches("<w:cols").count(), 2);
            prop_assert_eq!(applied.matches("<w:docGrid").count(), 2);
            prop_assert_eq!(applied.matches("w:rsidR=\"00112233\"").count(), 2);
            prop_assert_eq!(applied.matches("r:id=\"rId77\"").count(), 2);
            let type_xml = format!("<w:type w:val=\"{}\"", token);
            prop_assert!(applied.contains(&type_xml));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.undo(&mut ctx).expect("undo"));
            }
            prop_assert_eq!(export_document_xml(&document), before);
            prop_assert_eq!(selection, Some(range));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.redo(&mut ctx).expect("redo"));
            }
            prop_assert_eq!(export_document_xml(&document), applied.clone());

            let reopened_bytes = document.export_docx().expect("export for reopen");
            let reopened = executor::block_on(WordDocument::load(
                &reopened_bytes,
                &Options::default(),
            ))
            .expect("reopen docx");
            prop_assert_eq!(paragraph_text(&reopened, 0), text[..offset].to_string());
            prop_assert_eq!(paragraph_text(&reopened, 1), text[offset..].to_string());
            prop_assert_eq!(reopened.document.as_ref().expect("document").sections.len(), 2);
        }

        #[test]
        fn generated_section_breaks_preserve_styles_across_run_boundaries(
            first in "[A-Z]{1,12}",
            second in "[a-z]{1,12}",
            third in "[0-9]{1,12}",
            raw_offset in 0usize..128,
            kind in 0u8..4,
        ) {
            let text = format!("{first}{second}{third}");
            let offset = raw_offset % (text.len() + 1);
            let mut expected = first.bytes().map(|byte| (byte, 1)).collect::<Vec<_>>();
            expected.extend(second.bytes().map(|byte| (byte, 2)));
            expected.extend(third.bytes().map(|byte| (byte, 3)));
            let (break_type, _, _) = break_type(kind);
            let data = multi_run_section_docx(&first, &second, &third);
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut command_executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                command_executor
                    .execute(Box::new(InsertSectionBreakCommand::new(range, break_type)), &mut ctx)
                    .expect("insert section break across styled runs");
            }

            prop_assert_eq!(styled_bytes(&document, 0), expected[..offset].to_vec());
            prop_assert_eq!(styled_bytes(&document, 1), expected[offset..].to_vec());

            let reopened_bytes = document.export_docx().expect("export for reopen");
            let reopened = executor::block_on(WordDocument::load(
                &reopened_bytes,
                &Options::default(),
            ))
            .expect("reopen docx");
            prop_assert_eq!(styled_bytes(&reopened, 0), expected[..offset].to_vec());
            prop_assert_eq!(styled_bytes(&reopened, 1), expected[offset..].to_vec());
        }
    }

    fn two_row_table_docx() -> Vec<u8> {
        build_document_docx(
            r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>
<w:tr><w:tc><w:p><w:r><w:t>Alpha</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Bravo</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>After</w:t></w:r></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>"#,
        )
    }

    fn body_shape(document: &WordDocument) -> Vec<String> {
        document
            .document
            .as_ref()
            .and_then(|doc| doc.body.as_ref())
            .map(|body| {
                body.elements
                    .iter()
                    .map(|element| match element {
                        BodyElement::Paragraph(paragraph) => {
                            let has_break = paragraph
                                .properties
                                .as_ref()
                                .and_then(|props| props.section.as_ref())
                                .is_some();
                            format!("p{}", if has_break { "+sectPr" } else { "" })
                        }
                        BodyElement::Table(table) => format!("tbl{}", table.rows.len()),
                        BodyElement::Math(_) => "math".to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn section_break_in_a_table_cell_splits_the_table_at_the_caret_row() {
        // Word cannot keep a section break inside a cell: the table is split
        // in front of the caret's row and the break paragraph sits between the
        // two parts, so the new section opens with the rest of the table.
        let data = two_row_table_docx();
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        assert_eq!(paragraph_text(&document, 2), "Bravo");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(2, 0)));
        let mut command_executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            command_executor
                .execute(
                    Box::new(InsertSectionBreakCommand::new(
                        SelectionRange::collapsed(TextPosition::new(2, 0)),
                        SectionBreakType::NextPage,
                    )),
                    &mut ctx,
                )
                .expect("section break in a cell");
        }
        assert_eq!(
            body_shape(&document),
            ["p", "tbl1", "p+sectPr", "tbl1", "p"]
        );
        assert_eq!(paragraph_text(&document, 1), "Alpha");
        assert_eq!(paragraph_text(&document, 2), "");
        assert_eq!(paragraph_text(&document, 3), "Bravo");
        // The caret stays with its cell, now the first of the second table.
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(3, 0)))
        );
        let sections = &document.document.as_ref().expect("document").sections;
        assert_eq!(sections.len(), 2);
        assert_eq!((sections[0].start, sections[0].end), (0, 3));
        assert_eq!(
            document
                .document
                .as_ref()
                .and_then(|doc| doc.default_section.as_ref())
                .and_then(|props| props.section_type.as_deref()),
            Some("nextPage")
        );

        let exported = document.export_docx().expect("export");
        let reopened =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reopen");
        assert_eq!(
            body_shape(&reopened),
            ["p", "tbl1", "p+sectPr", "tbl1", "p"]
        );
        assert_eq!(paragraph_text(&reopened, 3), "Bravo");
        assert_eq!(
            reopened.document.as_ref().expect("document").sections.len(),
            2
        );

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(command_executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(body_shape(&document), ["p", "tbl2", "p"]);
        assert_eq!(paragraph_text(&document, 2), "Bravo");
        assert_eq!(
            document.document.as_ref().expect("document").sections.len(),
            1
        );
    }

    #[test]
    fn section_break_in_the_first_row_moves_the_whole_table_into_the_new_section() {
        let data = two_row_table_docx();
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        assert_eq!(paragraph_text(&document, 1), "Alpha");
        let following = document
            .insert_section_break_at(1, 0, SectionBreakType::Continuous)
            .expect("section break in the first row");
        assert_eq!(body_shape(&document), ["p", "p+sectPr", "tbl2", "p"]);
        assert_eq!(following, 2);
        assert_eq!(paragraph_text(&document, 2), "Alpha");
        let xml = export_document_xml(&document);
        assert_eq!(xml.matches("<w:sectPr").count(), 2);
        assert_eq!(xml.matches("<w:tbl>").count(), 1);
        assert_eq!(xml.matches("w:val=\"continuous\"").count(), 1);
    }

    #[test]
    fn insert_section_break_inside_existing_section_updates_only_its_following_boundary() {
        let data = build_document_docx(
            r#"<w:p><w:r><w:t>Alpha</w:t></w:r></w:p>
<w:p><w:pPr><w:sectPr><w:pgSz w:w="10000" w:h="15000"/><w:cols w:num="2"/></w:sectPr></w:pPr><w:r><w:t>Bravo</w:t></w:r></w:p>
<w:p><w:r><w:t>Charlie</w:t></w:r></w:p>
<w:sectPr><w:type w:val="oddPage"/><w:pgSz w:w="12000" w:h="16000"/><w:cols w:num="3"/></w:sectPr>"#,
        );
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        document
            .insert_section_break_at(0, 2, SectionBreakType::Continuous)
            .expect("insert section break");

        assert_eq!(paragraph_text(&document, 0), "Al");
        assert_eq!(paragraph_text(&document, 1), "pha");
        assert_eq!(paragraph_text(&document, 2), "Bravo");
        let sections = &document.document.as_ref().expect("document").sections;
        assert_eq!(sections.len(), 3);
        assert_eq!(
            sections[1]
                .properties
                .as_ref()
                .and_then(|props| props.section_type.as_deref()),
            Some("continuous")
        );
        assert_eq!(
            sections[2]
                .properties
                .as_ref()
                .and_then(|props| props.section_type.as_deref()),
            Some("oddPage"),
            "the later final section must keep its own start type"
        );
        let xml = export_document_xml(&document);
        assert_eq!(xml.matches("<w:sectPr").count(), 3);
        assert_eq!(xml.matches("w:val=\"continuous\"").count(), 1);
        assert_eq!(xml.matches("w:val=\"oddPage\"").count(), 1);
    }

    #[test]
    fn snapshot_batch_rebases_following_text_across_a_section_break() {
        let data = single_section_docx("abcdef");
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut command_executor = CommandExecutor::new();
        let selection = apply_snapshot_headless_commands_with_executor_to_document(
            &mut document,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertSectionBreak {
                        paragraph: 0,
                        offset: 2,
                        break_type: HeadlessSectionBreakType::EvenPage,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 5,
                        text: "X".to_string(),
                    },
                ],
            },
        )
        .expect("apply snapshot batch");

        assert_eq!(paragraph_text(&document, 0), "ab");
        assert_eq!(paragraph_text(&document, 1), "cdeXf");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(1, 4)))
        );
    }

    #[test]
    fn invalid_section_break_coordinates_leave_document_unchanged() {
        let data = single_section_docx("日本語");
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before = export_document_xml(&document);
        for offset in [1, 100] {
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut command_executor = CommandExecutor::new();
            let error = {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertSectionBreakCommand::new(
                            range,
                            SectionBreakType::NextPage,
                        )),
                        &mut ctx,
                    )
                    .expect_err("invalid offset must fail")
            };
            assert!(
                error.to_string().contains("inside a character")
                    || error.to_string().contains("past the end")
            );
            assert_eq!(export_document_xml(&document), before);
            assert!(!command_executor.can_undo());
        }
    }

    #[test]
    fn section_break_rejects_nested_and_noncollapsed_targets_atomically() {
        // A cell of a top-level table splits the table (see the tests above);
        // a table nested inside a cell is still refused.
        let data = build_document_docx(
            r#"<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr/>"#,
        );
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before = export_document_xml(&document);

        let nested = SelectionRange::collapsed(TextPosition::new(0, 2));
        let mut selection = Some(nested);
        let mut command_executor = CommandExecutor::new();
        let nested_error = {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            command_executor
                .execute(
                    Box::new(InsertSectionBreakCommand::new(
                        nested,
                        SectionBreakType::NextPage,
                    )),
                    &mut ctx,
                )
                .expect_err("nested paragraph must fail")
        };
        assert!(nested_error.to_string().contains("top-level body"));
        assert_eq!(export_document_xml(&document), before);
        assert_eq!(selection, Some(nested));
        assert!(!command_executor.can_undo());

        let expanded = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(2, 2));
        selection = Some(expanded);
        let expanded_error = {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            command_executor
                .execute(
                    Box::new(InsertSectionBreakCommand::new(
                        expanded,
                        SectionBreakType::Continuous,
                    )),
                    &mut ctx,
                )
                .expect_err("expanded selection must fail")
        };
        assert!(matches!(expanded_error, CommandError::UnsupportedSelection));
        assert_eq!(export_document_xml(&document), before);
        assert_eq!(selection, Some(expanded));
        assert!(!command_executor.can_undo());
    }
}
