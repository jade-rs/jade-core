//! Exact-selection authoring for Word's common inline fields.

use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::RunFieldKind;
use crate::word_document::WordDocumentSnapshot;
use jade_docx_headless_api::HeadlessFieldKind;
use jade_docx_layout::selection::SelectionRange;

pub struct InsertFieldCommand {
    range: SelectionRange,
    kind: HeadlessFieldKind,
    cached_text: String,
    format: Option<String>,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertFieldCommand {
    pub fn new(range: SelectionRange, kind: HeadlessFieldKind, cached_text: String) -> Self {
        Self {
            range,
            kind,
            cached_text,
            format: None,
            previous_document: None,
            previous_selection: None,
        }
    }

    /// A `DATE` or `TIME` field with a `\@` date picture, as Word's 日付と
    /// 時刻 dialog inserts one with 自動的に更新する checked.
    pub fn with_format(mut self, format: Option<String>) -> Self {
        self.format = format.filter(|picture| !picture.trim().is_empty());
        self
    }

    fn run_field_kind(&self) -> RunFieldKind {
        match self.kind {
            HeadlessFieldKind::Date => RunFieldKind::Date {
                format: self.format.clone(),
            },
            HeadlessFieldKind::Time => RunFieldKind::Time {
                format: self.format.clone(),
            },
            HeadlessFieldKind::Page => RunFieldKind::PageNumber,
            HeadlessFieldKind::NumPages => RunFieldKind::NumPages,
            // Jade deliberately leaves FILENAME cached: the package contains
            // no authoritative filesystem name, and the host owns that value.
            HeadlessFieldKind::FileName => RunFieldKind::Unsupported {
                instruction: "FILENAME".to_string(),
            },
        }
    }
}

impl Command for InsertFieldCommand {
    fn name(&self) -> &'static str {
        "insert_field"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        ctx.document
            .validate_inline_field_selection(self.range, &self.cached_text)
            .map_err(CommandError::Custom)?;
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;

        let result = (|| {
            // Reuse the editor's complete replacement path so a selection can
            // cross run or paragraph boundaries without introducing a second
            // paragraph-merge/comment-rebase implementation.
            let mut insert = InsertTextCommand::new(self.range, self.cached_text.clone(), None);
            insert.apply(ctx)?;
            let rendered_end = ctx
                .selection
                .as_ref()
                .map(|selection| selection.focus)
                .ok_or(CommandError::InvalidSelection)?;
            ctx.document
                .promote_inserted_text_to_field(
                    rendered_end,
                    &self.cached_text,
                    self.run_field_kind(),
                )
                .map_err(CommandError::Custom)
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

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("field snapshot is missing".into()))?;
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
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const FIELD_KINDS: [HeadlessFieldKind; 5] = [
        HeadlessFieldKind::Date,
        HeadlessFieldKind::Time,
        HeadlessFieldKind::Page,
        HeadlessFieldKind::NumPages,
        HeadlessFieldKind::FileName,
    ];

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

    fn boundaries(value: &str) -> Vec<usize> {
        let mut result = value
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        result.push(value.len());
        result
    }

    fn field_matches(kind: HeadlessFieldKind, field: &RunFieldKind) -> bool {
        match (kind, field) {
            (HeadlessFieldKind::Date, RunFieldKind::Date { format: None })
            | (HeadlessFieldKind::Time, RunFieldKind::Time { format: None })
            | (HeadlessFieldKind::Page, RunFieldKind::PageNumber)
            | (HeadlessFieldKind::NumPages, RunFieldKind::NumPages) => true,
            (HeadlessFieldKind::FileName, RunFieldKind::Unsupported { instruction }) => {
                instruction.trim().eq_ignore_ascii_case("FILENAME")
            }
            _ => false,
        }
    }

    fn instruction(kind: HeadlessFieldKind) -> &'static str {
        match kind {
            HeadlessFieldKind::Date => "DATE",
            HeadlessFieldKind::Time => "TIME",
            HeadlessFieldKind::Page => "PAGE",
            HeadlessFieldKind::NumPages => "NUMPAGES",
            HeadlessFieldKind::FileName => "FILENAME",
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn fields_replace_arbitrary_unicode_run_ranges_and_round_trip(
            text_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', 'z', 'あ', '界', '😀', 'é', ' ']),
                1..32,
            ),
            cached_chars in proptest::collection::vec(
                prop::sample::select(vec!['1', 'A', '日', '🙂', 'é', '&', '<', '>', ' ', '.', '-']),
                1..32,
            ),
            first_pick in any::<usize>(),
            second_pick in any::<usize>(),
            kind_pick in any::<usize>(),
        ) {
            let text = text_chars.into_iter().collect::<String>();
            let cached = cached_chars.into_iter().collect::<String>();
            let positions = boundaries(&text);
            let a = positions[first_pick % positions.len()];
            let b = positions[second_pick % positions.len()];
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            let kind = FIELD_KINDS[kind_pick % FIELD_KINDS.len()];
            let source = build_docx(std::slice::from_ref(&text));
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::new(
                TextPosition::new(0, start),
                TextPosition::new(0, end),
            );
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(InsertFieldCommand::new(range, kind, cached.clone())),
                    &mut context,
                ).expect("insert field");
            }

            let expected = format!("{}{}{}", &text[..start], cached, &text[end..]);
            let paragraph = document.paragraph(0).expect("field paragraph");
            prop_assert_eq!(
                paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
                expected,
            );
            let authored = paragraph.runs.iter().filter_map(|run| run.field.as_ref()).collect::<Vec<_>>();
            prop_assert_eq!(authored.len(), 1);
            prop_assert!(field_matches(kind, &authored[0].kind));
            prop_assert_eq!(authored[0].cached_text.as_deref(), Some(cached.as_str()));
            prop_assert_eq!(selection, Some(SelectionRange::collapsed(TextPosition::new(0, start + cached.len()))));

            let applied = document.export_docx().expect("export field");
            let xml = document_xml(&applied);
            let expected_instruction = format!("w:instr=\" {} \"", instruction(kind));
            prop_assert!(xml.contains(&expected_instruction), "{xml}");
            prop_assert!(xml.contains("w:dirty=\"true\""));
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen field");
            let reopened_fields = reopened.paragraph(0).expect("reopened paragraph").runs.iter()
                .filter_map(|run| run.field.as_ref()).collect::<Vec<_>>();
            prop_assert_eq!(reopened_fields.len(), 1);
            prop_assert!(field_matches(kind, &reopened_fields[0].kind));

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo field"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo field"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn current_state_batches_follow_prior_paragraph_merge_coordinates(
            left_chars in proptest::collection::vec(
                prop::sample::select(vec!['L', '左', '🙂', 'é']), 1..16,
            ),
            right_chars in proptest::collection::vec(
                prop::sample::select(vec!['R', '右', '界', 'ß']), 1..16,
            ),
            cached_chars in proptest::collection::vec(
                prop::sample::select(vec!['2', '頁', '🙂', '&', '<', '>']), 1..20,
            ),
            kind_pick in any::<usize>(),
        ) {
            let left = left_chars.into_iter().collect::<String>();
            let right = right_chars.into_iter().collect::<String>();
            let cached = cached_chars.into_iter().collect::<String>();
            let kind = FIELD_KINDS[kind_pick % FIELD_KINDS.len()];
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
                        HeadlessCommand::InsertField {
                            start_paragraph: 0,
                            start_offset: left.len(),
                            end_paragraph: 0,
                            end_offset: left.len() + 1,
                            kind,
                            cached_text: cached.clone(),
                        },
                    ],
                },
            ).expect("apply current-state field batch");

            prop_assert_eq!(document.paragraph_count(), 1);
            let paragraph = document.paragraph(0).expect("merged paragraph");
            prop_assert_eq!(
                paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
                format!("{left}{cached}{right}"),
            );
            prop_assert_eq!(paragraph.runs.iter().filter(|run| run.field.is_some()).count(), 1);
            let exported = document.export_docx().expect("export batch field");
            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen batch field");
            prop_assert_eq!(reopened.paragraph_count(), 1);
            prop_assert_eq!(reopened.paragraph(0).expect("reopened paragraph").runs.iter()
                .filter(|run| run.field.is_some()).count(), 1);
        }
    }

    #[test]
    fn invalid_or_deleted_field_targets_fail_atomically_at_the_command_index() {
        let source = build_docx(&["A😀B".to_string()]);
        let invalid_ranges = [
            SelectionRange::collapsed(TextPosition::new(0, 2)),
            SelectionRange::collapsed(TextPosition::new(9, 0)),
            SelectionRange::collapsed(TextPosition::new(0, 99)),
        ];
        for range in invalid_ranges {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load invalid document");
            let baseline = document.export_docx().expect("baseline");
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(
                    Box::new(InsertFieldCommand::new(
                        range,
                        HeadlessFieldKind::Date,
                        "2026/08/09".to_string(),
                    )),
                    &mut context,
                )
                .is_err());
            assert_eq!(document.export_docx().expect("invalid export"), baseline);
            assert!(!commands.can_undo());
        }

        for cached in [String::new(), "line\nbreak".to_string(), "x".repeat(1025)] {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load invalid cache document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::collapsed(TextPosition::new(0, 0));
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(
                    Box::new(InsertFieldCommand::new(
                        range,
                        HeadlessFieldKind::Time,
                        cached
                    )),
                    &mut context,
                )
                .is_err());
            assert_eq!(
                document.export_docx().expect("invalid cache export"),
                baseline
            );
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
                    HeadlessCommand::InsertField {
                        start_paragraph: 1,
                        start_offset: 0,
                        end_paragraph: 1,
                        end_offset: 0,
                        kind: HeadlessFieldKind::FileName,
                        cached_text: "missing.docx".to_string(),
                    },
                ],
            },
        )
        .expect_err("deleted field target must fail");
        assert!(error.to_string().contains("index 1"), "{error}");
        assert_eq!(document.paragraph_count(), 1);
        assert!(document
            .paragraph(0)
            .expect("survivor")
            .runs
            .iter()
            .all(|run| run.field.is_none()));
    }
}
