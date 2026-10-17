use super::{Command, CommandContext, CommandError, CommandResult};
use crate::commands::snapshot_edit::SnapshotEditCommand;
use crate::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Replaces one footnote body as an atomic, undoable package edit.
pub fn update_footnote_command(footnote_id: String, text: String) -> SnapshotEditCommand {
    SnapshotEditCommand::new("update_footnote", move |document| {
        document.update_footnote_text(&footnote_id, &text)
    })
}

/// Deletes one footnote and every reference to it as an atomic, undoable
/// package edit.
pub fn delete_footnote_command(footnote_id: String) -> SnapshotEditCommand {
    SnapshotEditCommand::new("delete_footnote", move |document| {
        document.delete_footnote(&footnote_id)
    })
}

/// Converts either kind of note as one undoable package edit.
pub fn convert_note_command(note_id: String, to_endnote: bool) -> ConvertNoteCommand {
    let name = if to_endnote {
        "convert_footnote_to_endnote"
    } else {
        "convert_endnote_to_footnote"
    };
    ConvertNoteCommand {
        inner: SnapshotEditCommand::new(name, move |document| {
            if to_endnote {
                document.convert_footnote_to_endnote(&note_id)
            } else {
                document.convert_endnote_to_footnote(&note_id)
            }
            .map(|_| ())
        }),
    }
}

pub struct ConvertNoteCommand {
    inner: SnapshotEditCommand,
}

impl Command for ConvertNoteCommand {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = *ctx.selection;
        let positions = previous.map(|range| {
            [range.anchor, range.focus].map(|position| {
                (
                    position,
                    ctx.document.story_position_for_surface(position.paragraph),
                )
            })
        });
        // Removing a note changes the editable surface indices. Keep the
        // story-local position of other notes, and return to the reference if
        // the selected note body was converted to the other note kind.
        let reference = positions.as_ref().and_then(|positions| {
            positions.iter().find_map(|(_, story)| {
                let story = story.as_ref()?;
                if !matches!(story.story, "footnote" | "endnote") {
                    return None;
                }
                let id = story.note_id.as_deref()?;
                (0..ctx.document.paragraph_count()).find_map(|index| {
                    let marker = ctx
                        .document
                        .reference_marker_spans(index)
                        .into_iter()
                        .find(|marker| {
                            marker.is_footnote == (story.story == "footnote")
                                && marker.note_id == id
                        })?;
                    Some(TextPosition::new(index, marker.model_offset))
                })
            })
        });
        self.inner.apply(ctx)?;
        if let Some(positions) = positions {
            let restored = positions.map(|(position, story)| {
                let surface = story.and_then(|story| {
                    ctx.document.surface_index_for_story(
                        story.story,
                        story.index,
                        story.anchor_paragraph,
                        story.note_id.as_deref(),
                    )
                });
                surface
                    .map(|paragraph| TextPosition::new(paragraph, position.offset))
                    .or(reference
                        .map(|position| ctx.document.model_to_rendered_text_position(position)))
                    .or_else(|| {
                        ctx.document
                            .surface_index_for_body(0)
                            .map(|index| TextPosition::new(index, 0))
                    })
            });
            *ctx.selection = restored[0]
                .zip(restored[1])
                .map(|(anchor, focus)| SelectionRange::new(anchor, focus));
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.undo(ctx)
    }
}

/// Inserts a WordprocessingML footnote at the current caret position.
pub struct InsertFootnoteCommand {
    range: SelectionRange,
    initial_text: String,
    previous_document: Option<WordDocument>,
    previous_selection: Option<SelectionRange>,
    inserted_id: Option<String>,
}

impl InsertFootnoteCommand {
    pub fn new(range: SelectionRange, initial_text: String) -> Self {
        Self {
            range,
            initial_text,
            previous_document: None,
            previous_selection: None,
            inserted_id: None,
        }
    }
}

impl Command for InsertFootnoteCommand {
    fn name(&self) -> &'static str {
        "insert_footnote"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let previous_document = ctx.document.clone();
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "footnote insertion requires a collapsed selection".into(),
            ));
        }
        let rendered_position = self.range.focus;
        if !ctx
            .document
            .is_body_paragraph_surface(rendered_position.paragraph)
        {
            return Err(CommandError::Custom(
                "footnotes can only be inserted in body paragraphs".into(),
            ));
        }

        let paragraph_text = ctx
            .document
            .paragraph(rendered_position.paragraph)
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .ok_or(CommandError::ParagraphUnavailable)?;
        let rendered_len = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(
                rendered_position.paragraph,
                paragraph_text.len(),
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
        if !paragraph_text.is_char_boundary(model_position.offset) {
            return Err(CommandError::Custom(format!(
                "offset {} is inside a character in paragraph {}",
                rendered_position.offset, rendered_position.paragraph
            )));
        }

        let id = match ctx.document.insert_footnote_at(
            model_position.paragraph,
            model_position.offset,
            &self.initial_text,
        ) {
            Ok(id) => id,
            Err(error) => {
                *ctx.document = previous_document;
                *ctx.selection = previous_selection;
                return Err(CommandError::Custom(error));
            }
        };

        self.inserted_id = Some(id);
        let Some(footnote_paragraph) = ctx
            .document
            .footnote_paragraph_index(self.inserted_id.as_deref().unwrap_or_default())
        else {
            *ctx.document = previous_document;
            *ctx.selection = previous_selection;
            return Err(CommandError::Custom(
                "inserted footnote paragraph is missing".into(),
            ));
        };
        let footnote_offset = ctx
            .document
            .paragraph_text_len_at(footnote_paragraph)
            .unwrap_or(0);
        let rendered_focus = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(
                footnote_paragraph,
                footnote_offset,
            ));
        self.previous_document = Some(previous_document);
        self.previous_selection = previous_selection;
        *ctx.selection = Some(SelectionRange::collapsed(rendered_focus));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_document = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        *ctx.document = previous_document;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{insert_text::InsertTextCommand, CommandContext, CommandExecutor};
    use crate::Options;
    use futures::executor;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    fn build_document_docx(document_xml: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
"#;

        const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"#;

        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("[Content_Types].xml", options)
                .expect("content types entry");
            zip.write_all(CONTENT_TYPES.as_bytes())
                .expect("write content types");
            zip.start_file("_rels/.rels", options).expect("rels entry");
            zip.write_all(RELS.as_bytes()).expect("write rels");
            zip.start_file("word/document.xml", options)
                .expect("document entry");
            zip.write_all(document_xml.as_bytes())
                .expect("write document");
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    /// A package whose footnotes part lives at a custom path with an unknown
    /// sibling node, so list/update/delete must go through the relationship
    /// and leave everything it does not own untouched.
    fn build_document_with_existing_footnote() -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/notes/custom-footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>ABC</w:t></w:r><w:r><w:footnoteReference w:id="7"/></w:r></w:p></w:body></w:document>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="notes/custom-footnotes.xml"/>
</Relationships>"#;
        const FOOTNOTES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:footnote w:id="0" w:type="separator"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
  <w:footnote w:id="1" w:type="continuationSeparator"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>
  <w:customXml w:uri="urn:jade:preserve"><w:p><w:r><w:t>unknown</w:t></w:r></w:p></w:customXml>
  <w:footnote w:id="7"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:tab/><w:t>existing</w:t></w:r></w:p></w:footnote>
</w:footnotes>"#;

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
                ("word/notes/custom-footnotes.xml", FOOTNOTES),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn exported_part_opt(document: &WordDocument, path: &str) -> Option<String> {
        let bytes = document.export_docx().expect("export docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut part = archive.by_name(path).ok()?;
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut part, &mut xml).expect("read xml part");
        Some(xml)
    }

    /// Footnotes get the same management surface endnotes already have:
    /// listing in first-reference order, replacing a body, and deleting the
    /// note together with its references — each atomic, each undoable, and
    /// never touching sibling nodes of the notes part.
    #[test]
    fn imported_footnote_can_be_listed_updated_and_deleted_without_touching_siblings() {
        use crate::word_document::NoteInfo;
        let data = build_document_with_existing_footnote();
        let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load existing footnote docx");
        assert_eq!(
            document.footnote_infos(),
            vec![NoteInfo {
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
                    Box::new(update_footnote_command(
                        "7".to_string(),
                        "更新<&>🙂".to_string(),
                    )),
                    &mut ctx,
                )
                .expect("update imported footnote");
        }
        assert_eq!(selection, selection_before);
        assert_eq!(document.footnote_infos()[0].text, "更新<&>🙂");
        let updated = exported_part_opt(&document, "word/notes/custom-footnotes.xml")
            .expect("custom footnotes part");
        assert!(updated.contains("w:uri=\"urn:jade:preserve\""));
        assert!(updated.contains("更新&lt;&amp;&gt;🙂"));
        let updated_bytes = document.export_docx().expect("export updated footnote");
        let reopened = executor::block_on(WordDocument::load(&updated_bytes, &Options::default()))
            .expect("reopen updated footnote");
        assert_eq!(reopened.footnote_infos()[0].text, "更新<&>🙂");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(delete_footnote_command("7".to_string())), &mut ctx)
                .expect("delete imported footnote");
        }
        assert_eq!(selection, selection_before);
        assert!(document.footnote_infos().is_empty());
        assert!(document
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .all(|run| run.footnote_reference_ids.is_empty()));
        let deleted = exported_part_opt(&document, "word/notes/custom-footnotes.xml")
            .expect("custom footnotes part after delete");
        assert!(deleted.contains("w:uri=\"urn:jade:preserve\""));
        assert!(!deleted.contains("<w:footnote w:id=\"7\""));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo delete"));
        }
        assert_eq!(
            exported_part_opt(&document, "word/notes/custom-footnotes.xml"),
            Some(updated)
        );
        assert_eq!(document.footnote_infos()[0].text, "更新<&>🙂");
    }

    /// The headless surface (and therefore MCP) drives the same lifecycle:
    /// insert at an exact byte offset, replace the body, delete with its
    /// references — mirroring what endnotes already offer.
    #[test]
    fn converts_imported_notes_both_ways_as_one_undoable_package_edit() {
        use crate::headless_api::apply_headless_commands_with_executor_to_document;
        use jade_docx_headless_api::HeadlessEditRequest;

        let data = build_document_with_existing_footnote();
        let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load imported footnote");
        let before = document.export_docx().expect("export before conversion");
        let mut commands = CommandExecutor::new();
        let request: HeadlessEditRequest = serde_json::from_value(serde_json::json!({
            "commands": [{"type": "convert_footnote_to_endnote", "footnote_id": "7"}]
        }))
        .expect("footnote conversion is a public command");
        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut commands,
            None,
            &request,
        )
        .expect("convert to endnote");
        assert!(document.footnote_infos().is_empty());
        let endnotes = document.endnote_infos();
        assert_eq!(endnotes.len(), 1);
        assert_eq!(endnotes[0].text, "existing");
        assert_eq!(endnotes[0].reference_count, 1);
        let source_xml = exported_part_opt(&document, "word/notes/custom-footnotes.xml").unwrap();
        assert!(source_xml.contains("unknown"));
        assert!(source_xml.contains("continuationSeparator"));
        assert!(!source_xml.contains("existing"));
        let target_xml = exported_part_opt(&document, "word/endnotes.xml").unwrap();
        assert!(target_xml.contains("<w:endnoteRef"));
        assert!(target_xml.contains("<w:tab"));
        assert!(!target_xml.contains("<w:footnoteRef"));
        let converted = document.export_docx().expect("export conversion");
        let reopened =
            executor::block_on(WordDocument::load(&converted, &Options::default())).unwrap();
        assert_eq!(reopened.endnote_infos(), endnotes);
        assert!(reopened.footnote_infos().is_empty());
        assert_eq!(commands.undo_depth(), 1);
        let mut selection = None;
        assert!(commands
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), before);
        assert!(commands
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), converted);

        let reverse: HeadlessEditRequest = serde_json::from_value(serde_json::json!({
            "commands": [{"type": "convert_endnote_to_footnote", "endnote_id": endnotes[0].id}]
        }))
        .expect("endnote conversion is a public command");
        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut commands,
            None,
            &reverse,
        )
        .expect("convert back to footnote");
        assert!(document.endnote_infos().is_empty());
        assert_eq!(document.footnote_infos()[0].text, "existing");
        assert_eq!(document.footnote_infos()[0].reference_count, 1);
    }

    #[test]
    fn conversion_preserves_rich_xml_relationships_and_avoids_id_collisions() {
        use crate::headless_api::apply_headless_commands_with_executor_to_document;
        use jade_docx_headless_api::HeadlessEditRequest;
        use std::collections::BTreeMap;
        use std::io::Read;

        let data = build_document_with_existing_footnote();
        let mut archive = zip::ZipArchive::new(Cursor::new(data)).unwrap();
        let mut parts = BTreeMap::<String, Vec<u8>>::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let name = entry.name().to_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            parts.insert(name, bytes);
        }
        let source = String::from_utf8(parts["word/notes/custom-footnotes.xml"].clone()).unwrap()
            .replace("<w:footnotes ", "<w:footnotes xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:x=\"urn:rich\" ")
            .replace("<w:r><w:tab/><w:t>existing</w:t>", "<w:r><w:rPr><w:b/></w:rPr><w:tab/><w:t>existing</w:t>")
            .replace("</w:footnote>\n</w:footnotes>", "<x:payload r:embed=\"rId6\"/><w:p><w:hyperlink r:id=\"rId5\"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p></w:footnote>\n</w:footnotes>");
        parts.insert(
            "word/notes/custom-footnotes.xml".into(),
            source.into_bytes(),
        );
        let rels = String::from_utf8(parts["word/_rels/document.xml.rels"].clone()).unwrap()
            .replace("</Relationships>", "<Relationship Id=\"rId10\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes\" Target=\"endnotes.xml\"/></Relationships>");
        parts.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
        parts.insert("word/endnotes.xml".into(), br#"<w:endnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:endnote w:id="7"><w:p><w:r><w:t>other</w:t></w:r></w:p></w:endnote></w:endnotes>"#.to_vec());
        parts.insert("word/notes/_rels/custom-footnotes.xml.rels".into(), br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/source" TargetMode="External"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/preserve.bin"/></Relationships>"#.to_vec());
        parts.insert("word/_rels/endnotes.xml.rels".into(), br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/other" TargetMode="External"/></Relationships>"#.to_vec());
        parts.insert(
            "word/media/preserve.bin".into(),
            b"preserved bytes".to_vec(),
        );
        let mut cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut cursor);
        for (name, bytes) in parts {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut document = executor::block_on(WordDocument::load(
            &cursor.into_inner(),
            &Options::default(),
        ))
        .unwrap();
        let request: HeadlessEditRequest = serde_json::from_value(serde_json::json!({
            "commands": [{"type": "convert_footnote_to_endnote", "footnote_id": "7"}]
        }))
        .unwrap();
        let mut commands = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut commands,
            None,
            &request,
        )
        .unwrap();
        let notes = document.endnote_infos();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].reference_count, 1);
        assert_ne!(notes[0].id, "7");
        let xml = exported_part_opt(&document, "word/endnotes.xml").unwrap();
        assert!(xml.contains("<w:b"));
        assert!(xml.contains("xmlns:x=\"urn:rich\""));
        assert!(xml.contains("<x:payload"));
        assert!(!xml.contains("r:embed=\"rId6\""));
        let rels = exported_part_opt(&document, "word/_rels/endnotes.xml.rels").unwrap();
        assert!(rels.contains("https://example.com/source"));
        assert!(rels.contains("https://example.com/other"));
        assert!(rels.contains("/word/media/preserve.bin"));
        let output = document.export_docx().unwrap();
        let reopened =
            executor::block_on(WordDocument::load(&output, &Options::default())).unwrap();
        assert_eq!(reopened.endnote_infos(), notes);
        let mut archive = zip::ZipArchive::new(Cursor::new(output)).unwrap();
        let mut bytes = Vec::new();
        archive
            .by_name("word/media/preserve.bin")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"preserved bytes");
    }

    #[test]
    fn conversion_returns_from_the_removed_note_body_and_undo_restores_its_caret() {
        let data = build_document_with_existing_footnote();
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
        let old = Some(SelectionRange::collapsed(TextPosition::new(
            document.footnote_paragraph_index("7").unwrap(),
            2,
        )));
        let mut selection = old;
        let mut commands = CommandExecutor::new();
        commands
            .execute(
                Box::new(convert_note_command("7".into(), true)),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 3)))
        );
        commands
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert_eq!(selection, old);
        commands
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 3)))
        );
    }

    #[test]
    fn note_references_include_table_cells_and_text_boxes_once() {
        use std::collections::BTreeMap;
        use std::io::Read;

        let mut archive =
            zip::ZipArchive::new(Cursor::new(build_document_with_existing_footnote())).unwrap();
        let mut parts = BTreeMap::<String, Vec<u8>>::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            parts.insert(entry.name().to_owned(), bytes);
        }
        let fixture = include_bytes!("../../../../public/fixtures/textbox.docx");
        let mut archive = zip::ZipArchive::new(Cursor::new(fixture)).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        let reference = r#"<w:p><w:r><w:footnoteReference w:id="7"/></w:r></w:p>"#;
        // Both AlternateContent branches describe the same text box. The
        // selected model branch counts once, while serialization retains both.
        xml = xml.replace("</w:txbxContent>", &format!("{reference}</w:txbxContent>"));
        xml = xml.replace(
            "<w:body>",
            &format!("<w:body>{reference}<w:tbl><w:tr><w:tc>{reference}</w:tc></w:tr></w:tbl>"),
        );
        parts.insert("word/document.xml".into(), xml.into_bytes());
        let mut cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut cursor);
        for (name, bytes) in parts {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut document = executor::block_on(WordDocument::load(
            &cursor.into_inner(),
            &Options::default(),
        ))
        .unwrap();
        assert_eq!(document.footnote_infos()[0].reference_count, 3);
        let endnote_id = document.convert_footnote_to_endnote("7").unwrap();
        assert_eq!(document.endnote_infos()[0].reference_count, 3);
        let exported = document.export_docx().unwrap();
        let mut reopened =
            executor::block_on(WordDocument::load(&exported, &Options::default())).unwrap();
        assert_eq!(reopened.endnote_infos()[0].reference_count, 3);
        reopened.delete_endnote(&endnote_id).unwrap();
        assert!(reopened.endnote_infos().is_empty());
        let xml = exported_part_opt(&reopened, "word/document.xml").unwrap();
        assert!(!xml.contains("endnoteReference"));
        for index in 0..reopened.paragraph_count() {
            assert!(reopened
                .paragraph(index)
                .unwrap()
                .runs
                .iter()
                .all(|run| run.endnote_reference_ids.is_empty()));
        }
    }

    #[test]
    fn invalid_note_conversion_does_not_mutate_package_or_undo_history() {
        use crate::headless_api::apply_headless_commands_with_executor_to_document;
        use jade_docx_headless_api::HeadlessEditRequest;
        let data = build_document_with_existing_footnote();
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
        let before = document.export_docx().unwrap();
        let mut commands = CommandExecutor::new();
        for id in ["missing", "0", "1"] {
            let request: HeadlessEditRequest = serde_json::from_value(serde_json::json!({
                "commands": [{"type": "convert_footnote_to_endnote", "footnote_id": id}]
            }))
            .unwrap();
            assert!(apply_headless_commands_with_executor_to_document(
                &mut document,
                &mut commands,
                None,
                &request
            )
            .is_err());
            assert_eq!(document.export_docx().unwrap(), before);
            assert_eq!(commands.undo_depth(), 0);
        }
    }

    #[test]
    fn headless_footnote_lifecycle_inserts_updates_and_deletes() {
        use crate::headless_api::apply_headless_commands_with_executor_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = build_document_docx(SIMPLE_HELLO_DOCUMENT);
        let mut document =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut executor = CommandExecutor::new();

        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertFootnote {
                    paragraph: 0,
                    offset: 5,
                    text: "note".to_string(),
                }],
            },
        )
        .expect("insert footnote headlessly");
        let infos = document.footnote_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].text, "note");
        assert_eq!(infos[0].reference_count, 1);
        let id = infos[0].id.clone();

        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::UpdateFootnote {
                    footnote_id: id.clone(),
                    text: "revised".to_string(),
                }],
            },
        )
        .expect("update footnote headlessly");
        assert_eq!(document.footnote_infos()[0].text, "revised");

        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::DeleteFootnote { footnote_id: id }],
            },
        )
        .expect("delete footnote headlessly");
        assert!(document.footnote_infos().is_empty());
        assert_eq!(
            paragraph_text(document.paragraph(0).expect("paragraph")),
            "Hello"
        );
        assert_eq!(executor.undo_depth(), 3, "each step is one undo entry");
    }

    #[test]
    fn footnote_management_rejects_separators_and_unknown_ids() {
        let data = build_document_with_existing_footnote();
        let mut document = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load existing footnote docx");
        assert!(
            document.update_footnote_text("0", "x").is_err(),
            "separator"
        );
        assert!(
            document.delete_footnote("1").is_err(),
            "continuation separator"
        );
        assert!(
            document.update_footnote_text("42", "x").is_err(),
            "unknown id"
        );
        assert!(document.delete_footnote("42").is_err(), "unknown id");
        // Nothing above may have touched the note or its references.
        assert_eq!(document.footnote_infos()[0].reference_count, 1);
    }

    fn build_document_docx_with_styles(document_xml: &str, styles_xml: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>
"#;

        const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"#;

        const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>
"#;

        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", RELS),
                ("word/document.xml", document_xml),
                ("word/_rels/document.xml.rels", DOCUMENT_RELS),
                ("word/styles.xml", styles_xml),
            ] {
                zip.start_file(path, options).expect("package entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn exported_part(word_doc: &WordDocument, part: &str) -> String {
        let bytes = word_doc.export_docx().expect("export docx");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open exported docx");
        let mut file = archive.by_name(part).expect("exported part");
        let mut content = String::new();
        std::io::Read::read_to_string(&mut file, &mut content).expect("read exported part");
        content
    }

    fn insert_footnote_at_end_of_hello(word_doc: &mut WordDocument) {
        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(word_doc, &mut selection);
        executor
            .execute(
                Box::new(InsertFootnoteCommand::new(range, String::new())),
                &mut ctx,
            )
            .expect("insert footnote");
    }

    const SIMPLE_HELLO_DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;

    /// Word-authored documents localize the note style ids (`aa`/`ac` in
    /// Japanese builds) while keeping the canonical display names. Insertion
    /// must find those styles by name and attach them, exactly like Word does.
    #[test]
    fn insert_footnote_attaches_existing_note_styles_by_name() {
        let styles_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="aa"><w:name w:val="footnote text"/><w:basedOn w:val="a"/><w:pPr><w:snapToGrid w:val="0"/><w:jc w:val="left"/></w:pPr></w:style>
  <w:style w:type="character" w:styleId="ac"><w:name w:val="footnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>
</w:styles>
"#;
        let data = build_document_docx_with_styles(SIMPLE_HELLO_DOCUMENT, styles_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        insert_footnote_at_end_of_hello(&mut word_doc);

        let body_reference_style = word_doc
            .paragraph(0)
            .expect("host paragraph")
            .runs
            .iter()
            .find(|run| !run.footnote_reference_ids.is_empty())
            .and_then(|run| run.properties.as_ref())
            .and_then(|props| props.style.as_deref().map(str::to_string));
        assert_eq!(body_reference_style.as_deref(), Some("ac"));

        let note_paragraph = word_doc.paragraph(1).expect("footnote paragraph");
        // Style id is attached and its defaults (left alignment) are baked in,
        // matching what document load does for note bodies.
        let note_props = note_paragraph.properties.as_ref().expect("note props");
        assert_eq!(note_props.style.as_deref(), Some("aa"));
        assert_eq!(note_props.alignment.as_deref(), Some("left"));

        let footnotes_xml = exported_part(&word_doc, "word/footnotes.xml");
        assert!(
            footnotes_xml.contains(r#"<w:pStyle w:val="aa""#),
            "exported footnote must reference the note text style: {footnotes_xml}"
        );
        assert!(
            footnotes_xml.contains(r#"<w:rStyle w:val="ac""#),
            "exported footnote ref must reference the note reference style: {footnotes_xml}"
        );
    }

    /// A document whose styles part lacks the note styles gets them created on
    /// first insertion — the same behavior as Word — so the note text renders
    /// left-aligned off the grid and the reference is superscript by style.
    #[test]
    fn insert_footnote_creates_word_note_styles_when_missing() {
        let styles_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/></w:style>
</w:styles>
"#;
        let data = build_document_docx_with_styles(SIMPLE_HELLO_DOCUMENT, styles_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        insert_footnote_at_end_of_hello(&mut word_doc);

        let note_props = word_doc
            .paragraph(1)
            .expect("footnote paragraph")
            .properties
            .clone()
            .expect("note props");
        assert_eq!(note_props.style.as_deref(), Some("FootnoteText"));
        assert_eq!(note_props.alignment.as_deref(), Some("left"));

        let styles_out = exported_part(&word_doc, "word/styles.xml");
        assert!(
            styles_out.contains(r#"w:styleId="FootnoteText""#)
                && styles_out.contains(r#"<w:name w:val="footnote text""#),
            "note text style must be created: {styles_out}"
        );
        assert!(
            styles_out.contains(r#"w:styleId="FootnoteReference""#)
                && styles_out.contains(r#"<w:vertAlign w:val="superscript""#),
            "note reference style must be created: {styles_out}"
        );

        let footnotes_xml = exported_part(&word_doc, "word/footnotes.xml");
        assert!(
            footnotes_xml.contains(r#"<w:pStyle w:val="FootnoteText""#)
                && footnotes_xml.contains(r#"<w:rStyle w:val="FootnoteReference""#),
            "exported footnote must reference the created styles: {footnotes_xml}"
        );
    }

    /// A package without any styles part keeps working: the note simply stays
    /// unstyled and the renderer's built-in superscript fallback applies.
    #[test]
    fn insert_footnote_without_styles_part_stays_unstyled() {
        let data = build_document_docx(SIMPLE_HELLO_DOCUMENT);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        insert_footnote_at_end_of_hello(&mut word_doc);

        let note_paragraph = word_doc.paragraph(1).expect("footnote paragraph");
        let style = note_paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref().map(str::to_string));
        assert_eq!(style, None);
    }

    fn paragraph_text(paragraph: &crate::document::Paragraph) -> String {
        paragraph.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// The caret the footnote insertion leaves is a *rendered* offset: it sits
    /// after the note number the reader sees, which no run holds. The
    /// paragraph's run text for an empty footnote is just the tab, one byte.
    ///
    /// The headless offset check has to run in that same space. Comparing the
    /// rendered offset against the run text alone rejects the position the
    /// editor itself produced, and typing the first character into a freshly
    /// inserted footnote came back as
    /// `offset 2 is past the end of paragraph 1, which is 1 bytes long`.
    #[test]
    fn headless_insert_text_accepts_the_caret_inside_a_new_footnote() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }
        let caret = selection.expect("selection after footnote").focus;

        crate::headless_api::apply_headless_commands_to_document(
            &mut word_doc,
            &jade_docx_headless_api::HeadlessEditRequest {
                commands: vec![jade_docx_headless_api::HeadlessCommand::InsertText {
                    paragraph: caret.paragraph,
                    offset: caret.offset,
                    text: "Z".to_string(),
                }],
            },
        )
        .expect("typing at the caret the editor produced must be accepted");

        let footnote_paragraph = word_doc
            .paragraph(caret.paragraph)
            .expect("footnote paragraph");
        assert!(
            paragraph_text(footnote_paragraph).contains('Z'),
            "typed text should land in the footnote body, got {:?}",
            paragraph_text(footnote_paragraph)
        );
    }

    /// Deleting several characters at once now takes the whole run in one
    /// command instead of repeating a single-character delete, so that a
    /// deleted phrase is one tracked change. Inside a footnote body the offsets
    /// are rendered ones — the note number is drawn in front of runs that do
    /// not hold it — which is the case most likely to make the two disagree, so
    /// it is pinned here: whatever `count: 3` does, three `count: 1` calls must
    /// do the same.
    #[test]
    fn headless_delete_text_in_a_footnote_matches_deleting_one_at_a_time() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let request = |commands| jade_docx_headless_api::HeadlessEditRequest { commands };

        // A footnote body holding "abcd", typed at the caret the editor gives.
        let typed_footnote = || {
            let data = build_document_docx(document_xml);
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
            let mut selection = Some(range);
            let mut command_executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertFootnoteCommand::new(range, String::new())),
                        &mut ctx,
                    )
                    .expect("insert footnote");
            }
            let caret = selection.expect("selection after footnote").focus;
            crate::headless_api::apply_headless_commands_to_document(
                &mut word_doc,
                &request(vec![jade_docx_headless_api::HeadlessCommand::InsertText {
                    paragraph: caret.paragraph,
                    offset: caret.offset,
                    text: "abcd".to_string(),
                }]),
            )
            .expect("type into the footnote body");
            (word_doc, caret)
        };

        let delete =
            |offset: usize, count: usize| jade_docx_headless_api::HeadlessCommand::DeleteText {
                paragraph: 1,
                offset,
                direction: jade_docx_headless_api::HeadlessDeleteDirection::Backward,
                count,
            };

        let (mut at_once, caret) = typed_footnote();
        let end = caret.offset + "abcd".len();
        assert_eq!(caret.paragraph, 1, "the footnote body is paragraph 1");
        crate::headless_api::apply_headless_commands_to_document(
            &mut at_once,
            &request(vec![delete(end, 3)]),
        )
        .expect("delete three characters in one command");

        // Three separate backspaces: each one leaves the caret a character
        // further back, which is what `count: 3` means.
        let (mut one_by_one, _) = typed_footnote();
        for step in 0..3 {
            crate::headless_api::apply_headless_commands_to_document(
                &mut one_by_one,
                &request(vec![delete(end - step, 1)]),
            )
            .expect("delete one character");
        }

        assert_eq!(
            paragraph_text(at_once.paragraph(1).expect("footnote")),
            paragraph_text(one_by_one.paragraph(1).expect("footnote")),
            "deleting a run at once must remove what deleting one at a time removes",
        );
    }

    /// The check still has to reject a genuinely out-of-range offset — one past
    /// the rendered end, not merely past the run text.
    #[test]
    fn headless_insert_text_still_rejects_an_offset_past_the_rendered_end() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }
        let caret = selection.expect("selection after footnote").focus;

        let error = crate::headless_api::apply_headless_commands_to_document(
            &mut word_doc,
            &jade_docx_headless_api::HeadlessEditRequest {
                commands: vec![jade_docx_headless_api::HeadlessCommand::InsertText {
                    paragraph: caret.paragraph,
                    offset: caret.offset + 1,
                    text: "Z".to_string(),
                }],
            },
        )
        .expect_err("an offset past the rendered end must still be rejected");
        assert!(
            error.to_string().contains("past the end of paragraph"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn moves_selection_into_inserted_footnote_body() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }

        let footnote_index = word_doc
            .footnote_paragraph_index("2")
            .expect("inserted footnote paragraph");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                footnote_index,
                2
            ))),
            "caret should move after the displayed footnote marker and tab"
        );

        let range_after_footnote = selection.expect("selection after footnote");
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(
                        range_after_footnote,
                        "A".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("type in footnote");
        }

        let footnote_paragraph = word_doc
            .paragraph(footnote_index)
            .expect("footnote paragraph");
        let footnote_text: String = footnote_paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(footnote_text, "\tA");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                footnote_index,
                3
            ))),
            "caret should stay in rendered footnote coordinates after typing"
        );
    }

    #[test]
    fn inserted_footnote_caret_accounts_for_a_multi_digit_start_number() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
    <w:sectPr><w:footnotePr><w:numStart w:val="10"/></w:footnotePr></w:sectPr>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }

        let footnote_index = word_doc
            .footnote_paragraph_index("2")
            .expect("inserted footnote paragraph");
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                footnote_index,
                3
            ))),
            "caret should follow the displayed marker '10' and the body tab"
        );
    }

    #[test]
    fn inserted_footnote_selection_precedes_following_body_paragraph() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
    <w:p><w:r><w:t>World</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let range = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }

        let footnote_index = word_doc
            .footnote_paragraph_index("2")
            .expect("inserted footnote paragraph");
        assert_eq!(
            footnote_index, 1,
            "inserted footnote surface should be exposed before following body paragraphs"
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                footnote_index,
                2
            ))),
            "caret should move into the inserted footnote, not the following body paragraph"
        );
        assert_eq!(
            word_doc.paragraph(2).map(paragraph_text),
            Some("World".to_string()),
            "following body paragraph should keep the next paragraph surface"
        );
    }

    #[test]
    fn inserts_reference_at_a_mid_paragraph_multibyte_offset_and_returns_footnote_caret() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>こんにちは世界</w:t></w:r></w:p>
    <w:p><w:r><w:t>Next</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        // The caret sits after "こんに" (three 3-byte characters).
        let caret = TextPosition::new(0, "こんに".len());
        let range = SelectionRange::collapsed(caret);
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, "note".to_string())),
                    &mut ctx,
                )
                .expect("insert footnote");
        }

        let body = word_doc.paragraph(0).expect("body paragraph");
        let texts: Vec<&str> = body.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["こんに", "", "ちは世界"],
            "the reference run must split the text exactly at the caret offset"
        );
        assert_eq!(
            body.runs[1].footnote_reference_ids,
            vec!["2".to_string()],
            "the run at the caret offset must carry the footnote reference"
        );
        assert!(
            body.runs[0].footnote_reference_ids.is_empty()
                && body.runs[2].footnote_reference_ids.is_empty(),
            "no other run in the paragraph may reference the footnote"
        );

        let footnote_index = word_doc
            .footnote_paragraph_index("2")
            .expect("inserted footnote paragraph");
        assert_eq!(
            footnote_index, 1,
            "footnote surface follows its host paragraph"
        );
        assert_eq!(
            word_doc.paragraph(footnote_index).map(paragraph_text),
            Some("\tnote".to_string())
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                footnote_index,
                "1\t".len() + "note".len()
            ))),
            "the command must leave the caret at the end of the footnote body"
        );
        assert_eq!(
            word_doc.paragraph(2).map(paragraph_text),
            Some("Next".to_string()),
            "following body paragraphs shift by exactly one surface"
        );

        // Undo restores the untouched paragraph and the caret that requested it.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo footnote");
        }
        assert_eq!(
            word_doc.paragraph(0).map(paragraph_text),
            Some("こんにちは世界".to_string())
        );
        assert_eq!(selection, Some(range));
    }

    #[test]
    fn rejects_an_offset_inside_a_multibyte_character() {
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>こんにちは</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let data = build_document_docx(document_xml);
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::collapsed(TextPosition::new(0, 1));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let error = {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertFootnoteCommand::new(range, String::new())),
                    &mut ctx,
                )
                .expect_err("offset inside a character must be rejected")
        };
        assert!(error.to_string().contains("inside a character"), "{error}");
        assert!(word_doc.footnote_paragraph_index("2").is_none());
        assert_eq!(selection, Some(range));
    }
}
