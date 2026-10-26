use super::paragraph_property_sync::sync_paragraph_drop_cap_xml;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{DropCap, DropCapKind, ParagraphProperties};
use jade_docx_layout::selection::SelectionRange;

/// Applies or removes a Word drop cap on a contiguous range of top-level body
/// paragraphs. Coordinates are resolved against the document's current state;
/// the command never clamps a missing paragraph to a surviving neighbour.
pub struct SetDropCapCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    kind: Option<DropCapKind>,
    lines: u8,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetDropCapCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        kind: Option<DropCapKind>,
        lines: u8,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            kind,
            lines,
            previous_properties: Vec::new(),
            previous_selection: None,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }

    fn requested_drop_cap(&self) -> Result<Option<DropCap>, CommandError> {
        let Some(kind) = self.kind else {
            return Ok(None);
        };
        if !(1..=10).contains(&self.lines) {
            return Err(CommandError::Custom(
                "drop cap lines must be between 1 and 10".to_string(),
            ));
        }
        Ok(Some(DropCap {
            kind,
            lines: self.lines,
        }))
    }
}

impl Command for SetDropCapCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_drop_cap"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let requested = self.requested_drop_cap()?;
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }
        for paragraph_index in start..=end {
            if !ctx
                .document
                .is_top_level_body_paragraph_surface(paragraph_index)
            {
                return Err(CommandError::Custom(
                    "drop caps can only be authored in top-level body paragraphs".to_string(),
                ));
            }
            let paragraph = ctx
                .document
                .paragraph(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            if requested.is_some()
                && !paragraph.runs.iter().any(|run| {
                    run.text
                        .chars()
                        .any(|character| !character.is_control() && !character.is_whitespace())
                })
            {
                return Err(CommandError::Custom(
                    "drop caps require a paragraph with visible text".to_string(),
                ));
            }
        }

        self.previous_properties.clear();
        self.previous_selection = *ctx.selection;
        for paragraph_index in start..=end {
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties.push((paragraph_index, previous));
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            paragraph
                .properties
                .get_or_insert_with(Default::default)
                .drop_cap = requested;
            sync_paragraph_drop_cap_xml(ctx.document, paragraph_index, requested);
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous) in &self.previous_properties {
            let restored = previous.as_ref().and_then(|props| props.drop_cap);
            let paragraph = ctx
                .document
                .paragraph_mut(*paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            paragraph.properties = previous.clone();
            sync_paragraph_drop_cap_xml(ctx.document, *paragraph_index, restored);
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::headless_api::apply_headless_commands_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessDropCapKind, HeadlessEditRequest};
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn escape_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx(paragraphs: &[String]) -> Vec<u8> {
        let body = paragraphs
            .iter()
            .map(|text| {
                format!(
                    "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    escape_text(text),
                )
            })
            .collect::<String>();
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
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

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive.by_name("word/document.xml").expect("document part");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read document");
        value
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

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn drop_cap_kinds_and_lines_round_trip_byte_exact_history(
            first in prop::sample::select(vec!['A', '界', '🙂', 'é', 'ש', 'क']),
            tail in proptest::collection::vec(
                prop::sample::select(vec!['a', ' ', '界', '🙂', 'é', '&', '<', '>']), 1..40,
            ),
            lines in 1_u8..=10,
            margin in any::<bool>(),
        ) {
            let text = format!("{first}{}", tail.into_iter().collect::<String>());
            let source = build_docx(std::slice::from_ref(&text));
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load drop-cap document");
            let baseline = document.export_docx().expect("baseline");
            let kind = if margin { DropCapKind::Margin } else { DropCapKind::Drop };
            let mut selection = Some(SelectionRange::collapsed(
                jade_docx_layout::selection::TextPosition::new(0, first.len_utf8()),
            ));
            let original_selection = selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(SetDropCapCommand::new(0, 0, Some(kind), lines)),
                        &mut context,
                    )
                    .expect("set drop cap");
            }

            prop_assert_eq!(paragraph_text(&document, 0), text.as_str());
            prop_assert_eq!(selection, original_selection);
            prop_assert_eq!(
                document.paragraph(0).and_then(|paragraph| paragraph.properties.as_ref()).and_then(|props| props.drop_cap),
                Some(DropCap { kind, lines }),
            );
            let applied = document.export_docx().expect("export drop cap");
            let xml = document_xml(&applied);
            let kind_token = if margin { "margin" } else { "drop" };
            let kind_attribute = format!("w:dropCap=\"{}\"", kind_token);
            let lines_attribute = format!("w:lines=\"{}\"", lines);
            prop_assert!(xml.contains(&kind_attribute));
            prop_assert!(xml.contains(&lines_attribute));
            prop_assert_eq!(xml.matches("<w:framePr").count(), 1);
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen drop cap");
            prop_assert_eq!(
                reopened.paragraph(0).and_then(|paragraph| paragraph.properties.as_ref()).and_then(|props| props.drop_cap),
                Some(DropCap { kind, lines }),
            );

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo drop cap"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, original_selection);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo drop cap"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn current_state_batches_target_the_paragraph_after_a_prior_merge(
            left in "[A-Za-z]{1,18}",
            right in "[A-Za-z]{1,18}",
            lines in 1_u8..=10,
            margin in any::<bool>(),
        ) {
            let source = build_docx(&[left.clone(), right.clone()]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load current-state document");
            let kind = if margin { HeadlessDropCapKind::Margin } else { HeadlessDropCapKind::Drop };
            apply_headless_commands_to_document(
                &mut document,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::ReplaceRange {
                            start_paragraph: 0,
                            start_offset: left.len(),
                            end_paragraph: 1,
                            end_offset: 0,
                            text: "-".to_string(),
                        },
                        HeadlessCommand::SetDropCap {
                            start_paragraph: 0,
                            end_paragraph: 0,
                            kind: Some(kind),
                            lines,
                        },
                    ],
                },
            )
            .expect("apply current-state drop-cap batch");

            prop_assert_eq!(document.paragraph_count(), 1);
            prop_assert_eq!(paragraph_text(&document, 0), format!("{left}-{right}"));
            let expected_kind = if margin { DropCapKind::Margin } else { DropCapKind::Drop };
            prop_assert_eq!(
                document.paragraph(0).and_then(|paragraph| paragraph.properties.as_ref()).and_then(|props| props.drop_cap),
                Some(DropCap { kind: expected_kind, lines }),
            );
            let exported = document.export_docx().expect("export batch");
            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen batch");
            prop_assert_eq!(paragraph_text(&reopened, 0), paragraph_text(&document, 0));
            prop_assert_eq!(
                reopened.paragraph(0).and_then(|paragraph| paragraph.properties.as_ref()).and_then(|props| props.drop_cap),
                Some(DropCap { kind: expected_kind, lines }),
            );
        }
    }

    #[test]
    fn removal_invalid_lines_and_deleted_targets_are_atomic() {
        let source = build_docx(&["Alpha".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load removal document");
        let baseline = document.export_docx().expect("baseline");
        let mut selection = Some(SelectionRange::collapsed(
            jade_docx_layout::selection::TextPosition::new(0, 0),
        ));
        let mut commands = CommandExecutor::new();
        {
            let mut context = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(SetDropCapCommand::new(0, 0, Some(DropCapKind::Drop), 3)),
                    &mut context,
                )
                .expect("set drop cap");
            commands
                .execute(
                    Box::new(SetDropCapCommand::new(0, 0, None, 0)),
                    &mut context,
                )
                .expect("remove drop cap ignores line count");
        }
        assert_eq!(document.export_docx().expect("removed export"), baseline);
        assert!(!document_xml(&baseline).contains("w:framePr"));

        for lines in [0, 11] {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load invalid-lines document");
            let baseline = document.export_docx().expect("invalid baseline");
            let mut selection = None;
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(
                    Box::new(SetDropCapCommand::new(0, 0, Some(DropCapKind::Drop), lines,)),
                    &mut context,
                )
                .is_err());
            assert_eq!(document.export_docx().expect("invalid export"), baseline);
            assert!(!commands.can_undo());
        }

        for text in ["", " \t\n\u{3000}"] {
            let source = build_docx(&[text.to_string()]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load invisible-text document");
            let baseline = document.export_docx().expect("invisible baseline");
            let mut selection = None;
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            let error = commands
                .execute(
                    Box::new(SetDropCapCommand::new(0, 0, Some(DropCapKind::Drop), 3)),
                    &mut context,
                )
                .expect_err("invisible text must reject drop caps");
            assert!(error.to_string().contains("visible text"), "{error}");
            assert_eq!(document.export_docx().expect("invisible export"), baseline);
            assert!(!commands.can_undo());
        }

        let source = build_docx(&["removed".to_string(), "survivor".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load deleted-target document");
        let error = apply_headless_commands_to_document(
            &mut document,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 1,
                        end_offset: 0,
                        text: String::new(),
                    },
                    HeadlessCommand::SetDropCap {
                        start_paragraph: 1,
                        end_paragraph: 1,
                        kind: Some(HeadlessDropCapKind::Drop),
                        lines: 3,
                    },
                ],
            },
        )
        .expect_err("deleted target must fail");
        assert!(error.to_string().contains("index 1"), "{error}");
        assert_eq!(document.paragraph_count(), 1);
        assert!(document
            .paragraph(0)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.drop_cap)
            .is_none());
    }
}
