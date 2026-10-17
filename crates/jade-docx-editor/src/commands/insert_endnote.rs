use super::{Command, CommandContext, CommandError, CommandResult};
use crate::commands::snapshot_edit::SnapshotEditCommand;
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts an endnote reference at an exact collapsed body caret and creates
/// the matching `word/endnotes.xml` entry as one atomic, undoable edit.
pub struct InsertEndnoteCommand {
    range: SelectionRange,
    initial_text: String,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertEndnoteCommand {
    pub fn new(range: SelectionRange, initial_text: String) -> Self {
        Self {
            range,
            initial_text,
            snapshot: None,
            previous_selection: None,
        }
    }
}

pub fn update_endnote_command(endnote_id: String, text: String) -> SnapshotEditCommand {
    SnapshotEditCommand::new("update_endnote", move |document| {
        document.update_endnote_text(&endnote_id, &text)
    })
}

pub fn delete_endnote_command(endnote_id: String) -> SnapshotEditCommand {
    SnapshotEditCommand::new("delete_endnote", move |document| {
        document.delete_endnote(&endnote_id)
    })
}

impl Command for InsertEndnoteCommand {
    fn name(&self) -> &'static str {
        "insert_endnote"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "endnote insertion requires a collapsed selection".into(),
            ));
        }

        let rendered_position = self.range.focus;
        if !ctx
            .document
            .is_body_paragraph_surface(rendered_position.paragraph)
        {
            return Err(CommandError::Custom(
                "endnotes can only be inserted in body paragraphs".into(),
            ));
        }
        let paragraph = ctx
            .document
            .paragraph(rendered_position.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let model_len = paragraph.runs.iter().map(|run| run.text.len()).sum();
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
        if let Err(error) = ctx.document.insert_endnote_at(
            model_position.paragraph,
            model_position.offset,
            &self.initial_text,
        ) {
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = previous_selection;
            return Err(CommandError::Custom(error));
        }

        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        *ctx.selection = Some(SelectionRange::collapsed(rendered_position));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::insert_footnote::InsertFootnoteCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::headless_api::{
        apply_headless_commands_with_executor_to_document,
        apply_snapshot_headless_commands_with_executor_to_document,
    };
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn build_document_docx(run_xml: &str) -> Vec<u8> {
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
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p>{run_xml}</w:p></w:body></w:document>"#
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

    fn build_document_with_existing_endnote() -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/notes/custom-endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>ABC</w:t></w:r><w:r><w:endnoteReference w:id="7"/></w:r></w:p></w:body></w:document>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="notes/custom-endnotes.xml"/>
</Relationships>"#;
        const ENDNOTES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:endnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:endnote w:id="0" w:type="separator"><w:p><w:r><w:separator/></w:r></w:p></w:endnote>
  <w:endnote w:id="1" w:type="continuationSeparator"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:endnote>
  <w:customXml w:uri="urn:jade:preserve"><w:p><w:r><w:t>unknown</w:t></w:r></w:p></w:customXml>
  <w:endnote w:id="7"><w:p><w:r><w:endnoteRef/></w:r><w:r><w:tab/><w:t>existing</w:t></w:r></w:p></w:endnote>
</w:endnotes>"#;

        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", DOCUMENT),
                ("word/_rels/document.xml.rels", DOC_RELS),
                ("word/notes/custom-endnotes.xml", ENDNOTES),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn exported_part(document: &WordDocument, path: &str) -> Option<String> {
        let bytes = document.export_docx().expect("export docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut part = archive.by_name(path).ok()?;
        let mut xml = String::new();
        part.read_to_string(&mut xml).expect("read xml part");
        Some(xml)
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

    fn endnote_reference_offset(document: &WordDocument, paragraph: usize, id: &str) -> usize {
        let mut offset = 0usize;
        for run in &document.paragraph(paragraph).expect("paragraph").runs {
            if run
                .endnote_reference_ids
                .iter()
                .any(|candidate| candidate == id)
            {
                return offset;
            }
            offset = offset.saturating_add(run.text.len());
        }
        panic!("endnote reference {id} was not found");
    }

    fn generated_text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("A"),
                Just("z"),
                Just("7"),
                Just("é"),
                Just("界"),
                Just("🙂"),
            ],
            1..32,
        )
        .prop_map(|parts| parts.concat())
    }

    fn generated_note_text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("A"),
                Just("界"),
                Just("🙂"),
                Just("&"),
                Just("<"),
                Just(">"),
                Just("\""),
                Just("'"),
                Just(" "),
            ],
            0..32,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_endnotes_preserve_run_text_package_history_and_round_trip(
            text in generated_text(),
            note_text in generated_text(),
            raw_offset in 0usize..128,
            raw_first_break in 0usize..128,
            raw_second_break in 0usize..128,
        ) {
            let boundaries = text.char_indices().map(|(index, _)| index)
                .chain(std::iter::once(text.len())).collect::<Vec<_>>();
            let offset = boundaries[raw_offset % boundaries.len()];
            let mut run_breaks = [
                boundaries[raw_first_break % boundaries.len()],
                boundaries[raw_second_break % boundaries.len()],
            ];
            run_breaks.sort_unstable();
            let runs = format!(
                "<w:r><w:rPr><w:b/></w:rPr><w:t>{}</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>{}</w:t></w:r><w:r><w:rPr><w:u w:val=\"single\"/></w:rPr><w:t>{}</w:t></w:r>",
                &text[..run_breaks[0]],
                &text[run_breaks[0]..run_breaks[1]],
                &text[run_breaks[1]..],
            );
            let data = build_document_docx(&runs);
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let before_document = exported_part(&document, "word/document.xml").expect("document");
            let before_rels = exported_part(&document, "word/_rels/document.xml.rels")
                .expect("document relationships");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(InsertEndnoteCommand::new(range, note_text.clone())),
                    &mut ctx,
                ).expect("insert endnote");
            }

            prop_assert_eq!(paragraph_text(&document, 0), text.clone());
            prop_assert_eq!(selection, Some(range));
            let parsed = document.document.as_ref().expect("document model");
            prop_assert_eq!(parsed.endnotes.values().filter(|note| note.note_type.is_none()).count(), 1);
            let note = parsed.endnotes.get("2").expect("generated endnote id");
            let stored_text = note.body.iter().filter_map(|element| element.as_paragraph())
                .flat_map(|paragraph| paragraph.runs.iter())
                .map(|run| run.text.as_str()).collect::<String>();
            prop_assert_eq!(stored_text, format!("\t{note_text}"));
            let reference_ids = document.paragraph(0).expect("body paragraph").runs.iter()
                .flat_map(|run| run.endnote_reference_ids.iter()).cloned().collect::<Vec<_>>();
            prop_assert_eq!(reference_ids, vec!["2".to_string()]);

            let applied_document = exported_part(&document, "word/document.xml").expect("document");
            let applied_rels = exported_part(&document, "word/_rels/document.xml.rels")
                .expect("document relationships");
            let applied_endnotes = exported_part(&document, "word/endnotes.xml").expect("endnotes");
            let applied_content_types = exported_part(&document, "[Content_Types].xml")
                .expect("content types");
            prop_assert!(applied_document.contains("<w:endnoteReference w:id=\"2\""));
            prop_assert!(applied_rels.contains("relationships/endnotes"));
            prop_assert!(applied_content_types.contains("/word/endnotes.xml"));
            prop_assert!(applied_endnotes.contains("<w:endnote w:id=\"2\""));
            prop_assert!(applied_endnotes.contains("<w:endnoteRef"));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo"));
            }
            prop_assert_eq!(exported_part(&document, "word/document.xml"), Some(before_document));
            prop_assert_eq!(exported_part(&document, "word/_rels/document.xml.rels"), Some(before_rels));
            prop_assert!(exported_part(&document, "word/endnotes.xml").is_none());
            prop_assert_eq!(selection, Some(range));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo"));
            }
            prop_assert_eq!(exported_part(&document, "word/document.xml"), Some(applied_document));
            prop_assert_eq!(exported_part(&document, "word/_rels/document.xml.rels"), Some(applied_rels));
            prop_assert_eq!(exported_part(&document, "word/endnotes.xml"), Some(applied_endnotes));

            let reopened_bytes = document.export_docx().expect("export for reopen");
            let reopened = executor::block_on(WordDocument::load(&reopened_bytes, &Options::default()))
                .expect("reopen docx");
            prop_assert_eq!(paragraph_text(&reopened, 0), text);
            prop_assert_eq!(
                reopened.document.as_ref().expect("reopened model").endnotes.values()
                    .filter(|note| note.note_type.is_none()).count(),
                1
            );
        }
    }

    #[test]
    fn invalid_offsets_are_rejected_atomically_without_history() {
        let data = build_document_docx("<w:r><w:t>A界B</w:t></w:r>");
        for offset in [2usize, "A界B".len() + 1] {
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let before_document = exported_part(&document, "word/document.xml");
            let before_rels = exported_part(&document, "word/_rels/document.xml.rels");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let error = {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor
                    .execute(
                        Box::new(InsertEndnoteCommand::new(range, "note".to_string())),
                        &mut ctx,
                    )
                    .expect_err("invalid offset must fail")
            };
            assert!(error.to_string().contains("offset"));
            assert!(!executor.can_undo());
            assert_eq!(selection, Some(range));
            assert_eq!(
                exported_part(&document, "word/document.xml"),
                before_document
            );
            assert_eq!(
                exported_part(&document, "word/_rels/document.xml.rels"),
                before_rels
            );
            assert!(exported_part(&document, "word/endnotes.xml").is_none());
        }
    }

    #[test]
    fn noncollapsed_selection_is_rejected_without_mutation() {
        let data = build_document_docx("<w:r><w:t>ABC</w:t></w:r>");
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before = exported_part(&document, "word/document.xml");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let error = {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertEndnoteCommand::new(range, "note".to_string())),
                    &mut ctx,
                )
                .expect_err("selection must be collapsed")
        };
        assert!(error.to_string().contains("collapsed selection"));
        assert!(!executor.can_undo());
        assert_eq!(exported_part(&document, "word/document.xml"), before);
    }

    #[test]
    fn missing_and_nested_note_paragraphs_are_rejected_atomically() {
        let data = build_document_docx("<w:r><w:t>ABC</w:t></w:r>");

        let mut missing_document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before_missing = exported_part(&missing_document, "word/document.xml");
        let missing_range = SelectionRange::collapsed(TextPosition::new(9, 0));
        let mut missing_selection = Some(missing_range);
        let mut missing_executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut missing_document, &mut missing_selection);
            missing_executor
                .execute(
                    Box::new(InsertEndnoteCommand::new(missing_range, "note".to_string())),
                    &mut ctx,
                )
                .expect_err("a deleted or missing paragraph must fail");
        }
        assert!(!missing_executor.can_undo());
        assert_eq!(
            exported_part(&missing_document, "word/document.xml"),
            before_missing
        );
        assert!(exported_part(&missing_document, "word/endnotes.xml").is_none());

        let mut nested_document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let body_range = SelectionRange::collapsed(TextPosition::new(0, 3));
        let mut nested_selection = Some(body_range);
        let mut nested_executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut nested_document, &mut nested_selection);
            nested_executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(
                        body_range,
                        "footnote".to_string(),
                    )),
                    &mut ctx,
                )
                .expect("insert a footnote");
        }
        let nested_range = nested_selection.expect("footnote caret");
        let before_nested = exported_part(&nested_document, "word/document.xml");
        let before_footnotes = exported_part(&nested_document, "word/footnotes.xml");
        {
            let mut ctx = CommandContext::new(&mut nested_document, &mut nested_selection);
            let error = nested_executor
                .execute(
                    Box::new(InsertEndnoteCommand::new(
                        nested_range,
                        "nested".to_string(),
                    )),
                    &mut ctx,
                )
                .expect_err("a note nested in another note must fail");
            assert!(error.to_string().contains("body paragraphs"));
        }
        assert_eq!(
            exported_part(&nested_document, "word/document.xml"),
            before_nested
        );
        assert_eq!(
            exported_part(&nested_document, "word/footnotes.xml"),
            before_footnotes
        );
        assert!(exported_part(&nested_document, "word/endnotes.xml").is_none());
        {
            let mut ctx = CommandContext::new(&mut nested_document, &mut nested_selection);
            assert!(nested_executor.undo(&mut ctx).expect("undo footnote"));
        }
        assert_eq!(paragraph_text(&nested_document, 0), "ABC");
        assert!(!nested_executor.can_undo());
    }

    #[test]
    fn existing_custom_endnote_part_keeps_path_unknown_nodes_and_allocates_next_id() {
        let data = build_document_with_existing_endnote();
        let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load existing endnote docx");
        let range = SelectionRange::collapsed(TextPosition::new(0, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertEndnoteCommand::new(range, "second".to_string())),
                    &mut ctx,
                )
                .expect("insert into custom endnote part");
        }

        let endnotes = exported_part(&document, "word/notes/custom-endnotes.xml")
            .expect("custom endnote part");
        assert!(endnotes.contains("w:uri=\"urn:jade:preserve\""));
        assert!(endnotes.contains("<w:endnote w:id=\"7\""));
        assert!(endnotes.contains("<w:endnote w:id=\"8\""));
        assert!(endnotes.contains(">second</w:t>"));
        assert!(exported_part(&document, "word/endnotes.xml").is_none());
        let relationships = exported_part(&document, "word/_rels/document.xml.rels")
            .expect("document relationships");
        assert!(relationships.contains("Target=\"notes/custom-endnotes.xml\""));
        assert_eq!(relationships.matches("relationships/endnotes").count(), 1);
        let content_types = exported_part(&document, "[Content_Types].xml").expect("content types");
        assert!(content_types.contains("/word/notes/custom-endnotes.xml"));
        assert!(!content_types.contains("PartName=\"/word/endnotes.xml\""));
    }

    #[test]
    fn imported_endnote_can_be_listed_updated_and_deleted_without_touching_siblings() {
        let data = build_document_with_existing_endnote();
        let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load existing endnote docx");
        assert_eq!(
            document.endnote_infos(),
            vec![crate::word_document::EndnoteInfo {
                id: "7".to_string(),
                text: "existing".to_string(),
                reference_count: 1,
            }]
        );

        let selection_before = Some(SelectionRange::collapsed(TextPosition::new(0, 2)));
        let mut selection = selection_before;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(update_endnote_command(
                        "7".to_string(),
                        "更新<&>🙂".to_string(),
                    )),
                    &mut ctx,
                )
                .expect("update imported endnote");
        }
        assert_eq!(selection, selection_before);
        assert_eq!(document.endnote_infos()[0].text, "更新<&>🙂");
        let updated = exported_part(&document, "word/notes/custom-endnotes.xml")
            .expect("custom endnotes part");
        assert!(updated.contains("w:uri=\"urn:jade:preserve\""));
        assert!(updated.contains("更新&lt;&amp;&gt;🙂"));
        let updated_bytes = document.export_docx().expect("export updated endnote");
        let reopened = executor::block_on(WordDocument::load(&updated_bytes, &Options::default()))
            .expect("reopen updated endnote");
        assert_eq!(reopened.endnote_infos()[0].text, "更新<&>🙂");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(delete_endnote_command("7".to_string())), &mut ctx)
                .expect("delete imported endnote");
        }
        assert_eq!(selection, selection_before);
        assert!(document.endnote_infos().is_empty());
        assert!(document
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .all(|run| run.endnote_reference_ids.is_empty()));
        let deleted = exported_part(&document, "word/notes/custom-endnotes.xml")
            .expect("custom endnotes part after delete");
        assert!(deleted.contains("w:uri=\"urn:jade:preserve\""));
        assert!(!deleted.contains("<w:endnote w:id=\"7\""));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo delete"));
        }
        assert_eq!(
            exported_part(&document, "word/notes/custom-endnotes.xml"),
            Some(updated)
        );
        assert_eq!(document.endnote_infos()[0].text, "更新<&>🙂");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn generated_endnote_lifecycle_is_atomic_and_round_trips(
            body_text in generated_text(),
            initial_text in generated_note_text(),
            updated_text in generated_note_text(),
            raw_offset in 0usize..128,
        ) {
            let boundaries = body_text.char_indices().map(|(index, _)| index)
                .chain(std::iter::once(body_text.len())).collect::<Vec<_>>();
            let offset = boundaries[raw_offset % boundaries.len()];
            let runs = format!("<w:r><w:t>{body_text}</w:t></w:r>");
            let data = build_document_docx(&runs);
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let before_document = exported_part(&document, "word/document.xml");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(InsertEndnoteCommand::new(range, initial_text.clone())),
                    &mut ctx,
                ).expect("insert endnote");
            }
            let inserted_document = exported_part(&document, "word/document.xml");
            let inserted_endnotes = exported_part(&document, "word/endnotes.xml");

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(update_endnote_command("2".to_string(), updated_text.clone())),
                    &mut ctx,
                ).expect("update endnote");
            }
            prop_assert_eq!(document.endnote_infos(), vec![crate::word_document::EndnoteInfo {
                id: "2".to_string(),
                text: updated_text.clone(),
                reference_count: 1,
            }]);
            let updated_document = exported_part(&document, "word/document.xml");
            let updated_endnotes = exported_part(&document, "word/endnotes.xml");
            prop_assert_eq!(&updated_document, &inserted_document);

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(delete_endnote_command("2".to_string())),
                    &mut ctx,
                ).expect("delete endnote");
            }
            prop_assert_eq!(paragraph_text(&document, 0), body_text.clone());
            prop_assert!(document.endnote_infos().is_empty());
            let deleted_document = exported_part(&document, "word/document.xml")
                .expect("document after delete");
            let deleted_endnotes = exported_part(&document, "word/endnotes.xml")
                .expect("endnotes after delete");
            prop_assert!(!deleted_document.contains("endnoteReference"));
            prop_assert!(!deleted_endnotes.contains("<w:endnote w:id=\"2\""));
            prop_assert_eq!(selection, Some(range));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo delete"));
                prop_assert!(executor.undo(&mut ctx).expect("undo update"));
            }
            prop_assert_eq!(exported_part(&document, "word/document.xml"), inserted_document);
            prop_assert_eq!(exported_part(&document, "word/endnotes.xml"), inserted_endnotes);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo insert"));
            }
            prop_assert_eq!(exported_part(&document, "word/document.xml"), before_document);
            prop_assert!(exported_part(&document, "word/endnotes.xml").is_none());

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo insert"));
                prop_assert!(executor.redo(&mut ctx).expect("redo update"));
            }
            prop_assert_eq!(exported_part(&document, "word/document.xml"), updated_document);
            prop_assert_eq!(exported_part(&document, "word/endnotes.xml"), updated_endnotes);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo delete"));
            }
            let reopened_bytes = document.export_docx().expect("export deleted endnote");
            let reopened = executor::block_on(WordDocument::load(&reopened_bytes, &Options::default()))
                .expect("reopen deleted endnote");
            prop_assert_eq!(paragraph_text(&reopened, 0), body_text);
            prop_assert!(reopened.endnote_infos().is_empty());
        }

        #[test]
        fn generated_headless_batches_rebase_endnote_coordinates_and_report_deleted_ids(
            body_text in generated_text(),
            prefix in generated_text(),
            note_text in generated_note_text(),
            updated_text in generated_note_text(),
            raw_offset in 0usize..128,
            snapshot_coordinates in any::<bool>(),
        ) {
            let boundaries = body_text.char_indices().map(|(index, _)| index)
                .chain(std::iter::once(body_text.len())).collect::<Vec<_>>();
            let original_offset = boundaries[raw_offset % boundaries.len()];
            let runs = format!("<w:r><w:t>{body_text}</w:t></w:r>");
            let data = build_document_docx(&runs);
            let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
            let mut executor = CommandExecutor::new();
            let endnote_offset = if snapshot_coordinates {
                original_offset
            } else {
                prefix.len().saturating_add(original_offset)
            };
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: prefix.clone(),
                    },
                    HeadlessCommand::InsertEndnote {
                        paragraph: 0,
                        offset: endnote_offset,
                        text: note_text,
                    },
                    HeadlessCommand::UpdateEndnote {
                        endnote_id: "2".to_string(),
                        text: updated_text.clone(),
                    },
                ],
            };
            selection = if snapshot_coordinates {
                apply_snapshot_headless_commands_with_executor_to_document(
                    &mut document,
                    &mut executor,
                    selection,
                    &request,
                )
            } else {
                apply_headless_commands_with_executor_to_document(
                    &mut document,
                    &mut executor,
                    selection,
                    &request,
                )
            }.expect("apply endnote batch");

            prop_assert_eq!(paragraph_text(&document, 0), format!("{prefix}{body_text}"));
            // Snapshot coordinates use left affinity at an exact insertion
            // point: a later zero-width endnote at the same original offset
            // remains before the newly inserted text. Positions strictly
            // after the insertion are shifted by its byte length.
            let expected_reference_offset = if snapshot_coordinates && original_offset == 0 {
                0
            } else {
                prefix.len().saturating_add(original_offset)
            };
            prop_assert_eq!(
                endnote_reference_offset(&document, 0, "2"),
                expected_reference_offset,
            );
            let infos = document.endnote_infos();
            prop_assert_eq!(infos[0].text.as_str(), updated_text.as_str());

            let delete = HeadlessEditRequest {
                commands: vec![HeadlessCommand::DeleteEndnote {
                    endnote_id: "2".to_string(),
                }],
            };
            selection = apply_headless_commands_with_executor_to_document(
                &mut document,
                &mut executor,
                selection,
                &delete,
            ).expect("delete endnote through current-state batch");
            prop_assert!(document.endnote_infos().is_empty());
            let deleted_document = exported_part(&document, "word/document.xml");
            let deleted_endnotes = exported_part(&document, "word/endnotes.xml");

            let missing_update = HeadlessEditRequest {
                commands: vec![HeadlessCommand::UpdateEndnote {
                    endnote_id: "2".to_string(),
                    text: "must fail".to_string(),
                }],
            };
            let error = apply_headless_commands_with_executor_to_document(
                &mut document,
                &mut executor,
                selection,
                &missing_update,
            ).expect_err("a deleted endnote id must be reported");
            prop_assert!(error.to_string().contains("endnote 2 was not found"));
            prop_assert_eq!(exported_part(&document, "word/document.xml"), deleted_document);
            prop_assert_eq!(exported_part(&document, "word/endnotes.xml"), deleted_endnotes);

            let mut ctx = CommandContext::new(&mut document, &mut selection);
            prop_assert!(executor.undo(&mut ctx).expect("undo must target delete, not failed update"));
            let infos = document.endnote_infos();
            prop_assert_eq!(infos[0].text.as_str(), updated_text.as_str());
        }
    }
}
