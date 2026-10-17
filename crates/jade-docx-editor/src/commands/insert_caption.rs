//! Word caption insertion backed by live `SEQ` fields.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

pub struct InsertCaptionCommand {
    range: SelectionRange,
    label: String,
    description: String,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertCaptionCommand {
    pub fn new(range: SelectionRange, label: String, description: String) -> Self {
        Self {
            range,
            label,
            description,
            previous_document: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertCaptionCommand {
    fn name(&self) -> &'static str {
        "insert_caption"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "caption insertion requires a collapsed selection".into(),
            ));
        }
        let rendered_position = self.range.focus;
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
        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
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
        match ctx.document.insert_caption_after_paragraph(
            rendered_position.paragraph,
            &self.label,
            &self.description,
        ) {
            Ok(selection) => {
                self.previous_document = Some(snapshot);
                self.previous_selection = previous_selection;
                *ctx.selection = Some(selection);
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("caption snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::RunFieldKind;
    use crate::headless_api::apply_headless_commands_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn escape_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx_paragraphs(texts: &[&str]) -> Vec<u8> {
        let paragraphs = texts
            .iter()
            .map(|text| {
                let runs = text
                    .chars()
                    .map(|character| {
                        format!(
                            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                            escape_text(&character.to_string())
                        )
                    })
                    .collect::<String>();
                format!("<w:p>{runs}</w:p>")
            })
            .collect::<String>();
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{paragraphs}</w:body></w:document>"
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, value) in [
                (
                    "[Content_Types].xml",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
                ),
                (
                    "_rels/.rels",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
                ),
                ("word/document.xml", document.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(value.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    fn build_docx(text: &str) -> Vec<u8> {
        build_docx_paragraphs(&[text])
    }

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive.by_name("word/document.xml").expect("document part");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read document");
        value
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_unicode_captions_keep_independent_sequence_and_history(
            anchor_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', '前', '😀', 'é', ' ']),
                1..20,
            ),
            descriptions in proptest::collection::vec(
                proptest::collection::vec(
                    prop::sample::select(vec!['x', '説', '🙂', '&', '<', ' ']),
                    0..18,
                ),
                1..7,
            ),
            use_japanese_label in any::<bool>(),
        ) {
            let anchor = anchor_chars.into_iter().collect::<String>();
            let descriptions = descriptions
                .into_iter()
                .map(|characters| characters.into_iter().collect::<String>())
                .collect::<Vec<_>>();
            let label = if use_japanese_label { "図" } else { "Figure" };
            let source = build_docx(&anchor);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
            let mut commands = CommandExecutor::new();

            for (index, description) in descriptions.iter().enumerate() {
                let paragraph = index;
                let range = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(InsertCaptionCommand::new(
                        range,
                        label.to_string(),
                        description.clone(),
                    )),
                    &mut context,
                ).expect("insert caption");
            }

            let applied = document.export_docx().expect("applied export");
            let xml = document_xml(&applied);
            let model = document.document.as_ref().expect("document model");
            let body_len = model.body.as_ref().expect("document body").elements.len();
            prop_assert_eq!(body_len, descriptions.len() + 1);
            prop_assert_eq!(model.sections.len(), 1);
            prop_assert_eq!(model.sections[0].start, 0);
            prop_assert_eq!(model.sections[0].end, body_len);
            prop_assert_eq!(xml.matches("<w:fldSimple").count(), descriptions.len());
            prop_assert_eq!(xml.matches("w:dirty=\"true\"").count(), descriptions.len());
            prop_assert_eq!(xml.matches("w:val=\"center\"").count(), descriptions.len());
            let instruction = format!("SEQ {label} \\* ARABIC");
            prop_assert_eq!(xml.matches(&instruction).count(), descriptions.len());
            for (index, description) in descriptions.iter().enumerate() {
                let saved_number = format!(">{}<", index + 1);
                prop_assert!(xml.contains(&saved_number));
                if !description.trim().is_empty() {
                    prop_assert!(xml.contains(&escape_text(description.trim())));
                }
            }

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen captions");
            let fields = (0..reopened.paragraph_count())
                .filter_map(|paragraph| reopened.paragraph(paragraph))
                .flat_map(|paragraph| paragraph.runs.iter())
                .filter_map(|run| run.field.as_ref())
                .filter_map(|field| match &field.kind {
                    RunFieldKind::Seq { identifier, .. } if identifier == label => {
                        field.resolved_value.clone().or_else(|| field.cached_text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            prop_assert_eq!(
                fields,
                (1..=descriptions.len()).map(|value| value.to_string()).collect::<Vec<_>>()
            );

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                for _ in &descriptions {
                    prop_assert!(commands.undo(&mut context).expect("undo caption"));
                }
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                for _ in &descriptions {
                    prop_assert!(commands.redo(&mut context).expect("redo caption"));
                }
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn headless_caption_batches_use_coordinates_after_prior_structural_commands(
            descriptions in proptest::collection::vec(
                proptest::collection::vec(
                    prop::sample::select(vec!['a', '図', '🙂', '&', '<', ' ']),
                    0..16,
                ),
                1..8,
            ),
        ) {
            let descriptions = descriptions
                .into_iter()
                .map(|characters| characters.into_iter().collect::<String>())
                .collect::<Vec<_>>();
            let source = build_docx("anchor");
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let request = HeadlessEditRequest {
                commands: descriptions
                    .iter()
                    .enumerate()
                    .map(|(index, description)| HeadlessCommand::InsertCaption {
                        // Every command targets the caption created by the
                        // immediately preceding command. These are current-state
                        // coordinates, not coordinates from the request snapshot.
                        after_paragraph: index,
                        label: "図".to_string(),
                        description: description.clone(),
                    })
                    .collect(),
            };

            apply_headless_commands_to_document(&mut document, &request)
                .expect("apply caption batch");

            prop_assert_eq!(document.paragraph_count(), descriptions.len() + 1);
            for (index, description) in descriptions.iter().enumerate() {
                let paragraph = document.paragraph(index + 1).expect("caption paragraph");
                let field = paragraph
                    .runs
                    .iter()
                    .find_map(|run| run.field.as_ref())
                    .expect("SEQ field");
                let expected_number = (index + 1).to_string();
                prop_assert_eq!(field.resolved_value.as_deref(), Some(expected_number.as_str()));
                let text = paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>();
                prop_assert!(text.ends_with(description.trim()));
            }
            let model = document.document.as_ref().expect("document model");
            let body_len = model.body.as_ref().expect("document body").elements.len();
            prop_assert_eq!(model.sections.len(), 1);
            prop_assert_eq!(model.sections[0].end, body_len);
        }
    }

    #[test]
    fn caption_labels_count_independently_and_invalid_inputs_are_atomic() {
        let source = build_docx("anchor");
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let range = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(range);
        let mut commands = CommandExecutor::new();
        for (paragraph, label) in [(0, "Figure"), (1, "Table"), (2, "Figure")] {
            let mut context = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(InsertCaptionCommand::new(
                        SelectionRange::collapsed(TextPosition::new(paragraph, 0)),
                        label.to_string(),
                        String::new(),
                    )),
                    &mut context,
                )
                .expect("insert caption");
        }
        let xml = document_xml(&document.export_docx().expect("export captions"));
        assert_eq!(xml.matches("SEQ Figure \\* ARABIC").count(), 2);
        assert_eq!(xml.matches("SEQ Table \\* ARABIC").count(), 1);

        for (label, description) in [
            ("", "ok"),
            ("1Figure", "ok"),
            ("has space", "ok"),
            ("punct!", "ok"),
            ("Figure", "line\nbreak"),
        ] {
            let before = document.export_docx().expect("before invalid caption");
            let undo_before = commands.can_undo();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(
                    Box::new(InsertCaptionCommand::new(
                        SelectionRange::collapsed(TextPosition::new(0, 0)),
                        label.to_string(),
                        description.to_string(),
                    )),
                    &mut context,
                )
                .is_err());
            assert_eq!(
                document.export_docx().expect("after invalid caption"),
                before
            );
            assert_eq!(commands.can_undo(), undo_before);
        }
    }
}
