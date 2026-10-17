//! Exact-selection insertion of one blank Word page.

use super::insert_paragraph_break::InsertParagraphBreakCommand;
use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts one blank page at the current selection using Word page-break
/// controls. If content follows, the blank page is surrounded by two breaks;
/// at document end one break is enough. In both cases the caret stays on the
/// blank page immediately after the first break.
pub struct InsertBlankPageCommand {
    range: SelectionRange,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertBlankPageCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            previous_document: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertBlankPageCommand {
    fn name(&self) -> &'static str {
        "insert_blank_page"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let break_count = ctx
            .document
            .blank_page_break_count(self.range)
            .map_err(CommandError::Custom)?;
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;

        let result = (|| {
            let mut insert =
                InsertTextCommand::new(self.range, "\u{000c}".repeat(break_count), None);
            insert.apply(ctx)?;

            let inserted_end = ctx
                .selection
                .as_ref()
                .map(|selection| selection.focus)
                .ok_or(CommandError::InvalidSelection)?;
            if break_count == 1 {
                // A trailing blank page needs a real empty paragraph so its
                // caret has a line box on the new page. This also mirrors
                // Word/GenOffice instead of leaving the caret attached to the
                // pre-break line in the final paragraph.
                let mut paragraph_break =
                    InsertParagraphBreakCommand::new(SelectionRange::collapsed(inserted_end));
                paragraph_break.apply(ctx)?;
                return Ok(());
            }
            let model_end = ctx.document.rendered_to_model_text_position(inserted_end);
            let blank_page_offset = model_end
                .offset
                .checked_sub(break_count.saturating_sub(1))
                .ok_or(CommandError::InvalidSelection)?;
            let caret = ctx
                .document
                .model_to_rendered_text_position(TextPosition::new(
                    model_end.paragraph,
                    blank_page_offset,
                ));
            *ctx.selection = Some(SelectionRange::collapsed(caret));
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.previous_document = Some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(error)
            }
        }
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        // Page-break controls can reflow every later page even when the
        // paragraph count is unchanged.
        HistoryRenderFootprint::Structural
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("blank-page history is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
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

    fn build_docx(paragraphs: &[String]) -> Vec<u8> {
        let body = paragraphs
            .iter()
            .map(|paragraph| {
                let runs = paragraph
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
        fn blank_page_uses_one_or_two_breaks_and_round_trips_history(
            left_chars in proptest::collection::vec(
                prop::sample::select(vec!['L', '左', '🙂', 'é', '&', '<', '>']), 0..18,
            ),
            right_chars in proptest::collection::vec(
                prop::sample::select(vec!['R', '右', '界', 'ß', ' ']), 0..18,
            ),
            trailing_paragraph in any::<bool>(),
        ) {
            let left = left_chars.into_iter().collect::<String>();
            let right = right_chars.into_iter().collect::<String>();
            let mut source_paragraphs = vec![format!("{left}{right}")];
            if trailing_paragraph {
                source_paragraphs.push("following block".to_string());
            }
            let source = build_docx(&source_paragraphs);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load blank-page document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::collapsed(TextPosition::new(0, left.len()));
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(Box::new(InsertBlankPageCommand::new(range)), &mut context)
                    .expect("insert blank page");
            }

            let break_count = if !right.is_empty() || trailing_paragraph { 2 } else { 1 };
            if break_count == 1 {
                prop_assert_eq!(document.paragraph_count(), 2);
                prop_assert_eq!(paragraph_text(&document, 0), format!("{left}\u{000c}"));
                prop_assert_eq!(paragraph_text(&document, 1), "");
                prop_assert_eq!(
                    selection,
                    Some(SelectionRange::collapsed(TextPosition::new(1, 0))),
                );
            } else {
                prop_assert_eq!(
                    paragraph_text(&document, 0),
                    format!("{left}{}{right}", "\u{000c}".repeat(break_count)),
                );
                prop_assert_eq!(
                    selection,
                    Some(SelectionRange::collapsed(TextPosition::new(0, left.len() + 1))),
                );
            }
            let applied = document.export_docx().expect("export blank page");
            prop_assert_eq!(document_xml(&applied).matches("w:type=\"page\"").count(), break_count);
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen blank page");
            prop_assert_eq!(paragraph_text(&reopened, 0), paragraph_text(&document, 0));
            prop_assert_eq!(reopened.paragraph_count(), document.paragraph_count());

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo blank page"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo blank page"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn current_state_batches_use_coordinates_after_a_prior_paragraph_merge(
            left_chars in proptest::collection::vec(
                prop::sample::select(vec!['L', '左', '🙂', 'é']), 1..16,
            ),
            right_chars in proptest::collection::vec(
                prop::sample::select(vec!['R', '右', '界', 'ß']), 1..16,
            ),
        ) {
            let left = left_chars.into_iter().collect::<String>();
            let right = right_chars.into_iter().collect::<String>();
            let source = build_docx(&[left.clone(), right.clone()]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load batch document");
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
                        HeadlessCommand::InsertBlankPage {
                            start_paragraph: 0,
                            start_offset: left.len() + 1,
                            end_paragraph: 0,
                            end_offset: left.len() + 1,
                        },
                    ],
                },
            ).expect("apply current-state blank-page batch");

            prop_assert_eq!(document.paragraph_count(), 1);
            prop_assert_eq!(paragraph_text(&document, 0), format!("{left}-\u{000c}\u{000c}{right}"));
            let exported = document.export_docx().expect("export batch blank page");
            prop_assert_eq!(document_xml(&exported).matches("w:type=\"page\"").count(), 2);
            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen batch blank page");
            prop_assert_eq!(paragraph_text(&reopened, 0), paragraph_text(&document, 0));
        }
    }

    #[test]
    fn invalid_or_deleted_blank_page_targets_fail_at_the_exact_command_index() {
        let source = build_docx(&["A😀B".to_string()]);
        for range in [
            SelectionRange::collapsed(TextPosition::new(0, 2)),
            SelectionRange::collapsed(TextPosition::new(9, 0)),
            SelectionRange::collapsed(TextPosition::new(0, 99)),
        ] {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load invalid document");
            let baseline = document.export_docx().expect("baseline");
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(Box::new(InsertBlankPageCommand::new(range)), &mut context)
                .is_err());
            assert_eq!(document.export_docx().expect("invalid export"), baseline);
            assert!(!commands.can_undo());
        }

        let source = build_docx(&["removed".to_string(), "survivor".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load deletion document");
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
                    HeadlessCommand::InsertBlankPage {
                        start_paragraph: 1,
                        start_offset: 0,
                        end_paragraph: 1,
                        end_offset: 0,
                    },
                ],
            },
        )
        .expect_err("deleted blank-page target must fail");
        assert!(error.to_string().contains("index 1"), "{error}");
        assert_eq!(document.paragraph_count(), 1);
        assert!(!paragraph_text(&document, 0).contains('\u{000c}'));
    }
}
