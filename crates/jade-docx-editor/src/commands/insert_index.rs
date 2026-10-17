//! Word index-entry marking and generated INDEX field insertion.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

struct SnapshotState {
    document: WordDocumentSnapshot,
    selection: Option<SelectionRange>,
}

fn snapshot(ctx: &mut CommandContext<'_>) -> CommandResult<SnapshotState> {
    Ok(SnapshotState {
        document: ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?,
        selection: *ctx.selection,
    })
}

fn restore(ctx: &mut CommandContext<'_>, state: SnapshotState) {
    ctx.document.restore_snapshot(state.document);
    *ctx.selection = state.selection;
}

pub struct MarkIndexEntryCommand {
    range: SelectionRange,
    term: String,
    previous: Option<SnapshotState>,
}

impl MarkIndexEntryCommand {
    pub fn new(range: SelectionRange, term: String) -> Self {
        Self {
            range,
            term,
            previous: None,
        }
    }
}

impl Command for MarkIndexEntryCommand {
    fn name(&self) -> &'static str {
        "mark_index_entry"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx
            .document
            .mark_index_entry_at_selection_end(self.range, &self.term)
        {
            Ok(selection) => {
                self.previous = Some(previous);
                *ctx.selection = Some(selection);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("index-entry snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

pub struct InsertIndexCommand {
    after_paragraph: usize,
    previous: Option<SnapshotState>,
}

impl InsertIndexCommand {
    pub fn new(after_paragraph: usize) -> Self {
        Self {
            after_paragraph,
            previous: None,
        }
    }
}

impl Command for InsertIndexCommand {
    fn name(&self) -> &'static str {
        "insert_index"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx
            .document
            .insert_index_after_paragraph(self.after_paragraph)
        {
            Ok(selection) => {
                self.previous = Some(previous);
                *ctx.selection = Some(selection);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("index snapshot is missing".into()))?;
        restore(ctx, previous);
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

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn unicode_index_entries_round_trip_at_arbitrary_run_boundaries(
            text_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', 'z', 'あ', '界', '😀', 'é', ' ']),
                1..28,
            ),
            term_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', '索', '😀', 'é', '&', '<', '>', '\\', ' ']),
                1..28,
            ),
            boundary_pick in any::<usize>(),
        ) {
            let text = text_chars.into_iter().collect::<String>();
            let term = term_chars.into_iter().collect::<String>();
            let normalized = term.replace(['"', '\\'], "").trim().to_string();
            prop_assume!(!normalized.is_empty());
            let offset = boundaries(&text)[boundary_pick % boundaries(&text).len()];
            let source = build_docx(&[text]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(MarkIndexEntryCommand::new(range, term.clone())),
                    &mut context,
                ).expect("mark index entry");
            }

            prop_assert_eq!(document.index_entries(), vec![normalized.clone()]);
            let applied = document.export_docx().expect("export marked entry");
            let xml = document_xml(&applied);
            prop_assert!(xml.contains("XE &quot;") || xml.contains("XE \"") || xml.contains("XE &amp;quot;"));
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen marked entry");
            prop_assert_eq!(reopened.index_entries(), vec![normalized]);

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo mark"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo mark"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn current_state_batches_follow_prior_structural_coordinates_and_generate_index(
            terms in proptest::collection::vec(
                proptest::collection::vec(
                    prop::sample::select(vec!['a', '索', '🙂', '&', '<', ' ', '"']),
                    1..18,
                ),
                1..8,
            ),
        ) {
            let terms = terms
                .into_iter()
                .map(|characters| characters.into_iter().collect::<String>())
                .filter(|term| {
                    !term.replace(['"', '\\'], "").trim().is_empty()
                })
                .collect::<Vec<_>>();
            prop_assume!(!terms.is_empty());
            let source = build_docx(&["anchor".to_string()]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let mut commands = Vec::new();
            for (index, term) in terms.iter().enumerate() {
                commands.push(HeadlessCommand::MarkIndexEntry {
                    paragraph: 0,
                    offset: index.min("anchor".len()),
                    term: term.clone(),
                });
            }
            commands.push(HeadlessCommand::InsertCaption {
                after_paragraph: 0,
                label: "図".to_string(),
                description: "anchor".to_string(),
            });
            commands.push(HeadlessCommand::InsertIndex { after_paragraph: 1 });
            apply_headless_commands_to_document(
                &mut document,
                &HeadlessEditRequest { commands },
            ).expect("apply current-state batch");

            let unique = document.index_entries();
            let marker_count = document
                .paragraph(0)
                .expect("marked paragraph")
                .runs
                .iter()
                .filter(|run| {
                    matches!(
                        run.field.as_ref().map(|field| &field.kind),
                        Some(crate::document::RunFieldKind::NonDisplay { instruction })
                            if instruction.starts_with("XE ")
                    )
                })
                .count();
            prop_assert_eq!(marker_count, terms.len());
            prop_assert!(!unique.is_empty());
            prop_assert_eq!(document.paragraph_count(), unique.len() + 2);
            let exported = document.export_docx().expect("export index");
            let xml = document_xml(&exported);
            prop_assert!(xml.contains("INDEX \\c &quot;2&quot;") || xml.contains("INDEX \\c \"2\""));
            prop_assert!(xml.contains("w:dirty=\"true\""));
            prop_assert_eq!(xml.matches("XE ").count(), terms.len(), "{}", xml);
            let model = document.document.as_ref().expect("document model");
            let body_len = model.body.as_ref().expect("body").elements.len();
            prop_assert_eq!(model.sections.len(), 1);
            prop_assert_eq!(model.sections[0].end, body_len);

            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen generated index");
            prop_assert_eq!(reopened.index_entries(), unique.clone());
            prop_assert_eq!(reopened.paragraph_count(), unique.len() + 2);
            for (offset, expected) in unique.iter().enumerate() {
                let paragraph = reopened.paragraph(offset + 2).expect("reopened index line");
                let visible = paragraph.runs.iter().map(|run| {
                    if run.text.is_empty() {
                        run.field.as_ref().and_then(|field| field.cached_text.clone()).unwrap_or_default()
                    } else {
                        run.text.clone()
                    }
                }).collect::<String>();
                prop_assert!(visible.starts_with(expected), "{visible:?} did not start with {expected:?}");
            }
        }
    }

    #[test]
    fn two_entries_marked_at_the_same_position_survive_a_reopen() {
        // The e2e index flow marks two XE entries at the same collapsed
        // position, giving the paragraph two adjacent authored w:fldSimple
        // containers. Export and reopen must keep both.
        let source = build_docx(&["Hello".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let mut selection = None;
        let mut commands = CommandExecutor::new();
        for term in ["索引 & 項目", " A\"B\\C "] {
            let range = SelectionRange::collapsed(TextPosition::new(0, 0));
            let mut context = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(MarkIndexEntryCommand::new(range, term.to_string())),
                    &mut context,
                )
                .expect("mark index entry");
        }
        let mut entries = document.index_entries();
        entries.sort();
        assert_eq!(entries, ["ABC", "索引 & 項目"]);

        let applied = document.export_docx().expect("export both entries");
        let reopened =
            executor::block_on(WordDocument::load(&applied, &Options::default())).expect("reopen");
        let mut reopened_entries = reopened.index_entries();
        reopened_entries.sort();
        assert_eq!(reopened_entries, ["ABC", "索引 & 項目"]);
    }

    #[test]
    fn invalid_index_targets_are_atomic_and_deleted_paragraph_is_reported() {
        let source = build_docx(&["A😀B".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let baseline = document.export_docx().expect("baseline");
        let mut selection = None;
        let mut commands = CommandExecutor::new();
        for (range, term) in [
            (SelectionRange::collapsed(TextPosition::new(0, 2)), "valid"),
            (SelectionRange::collapsed(TextPosition::new(7, 0)), "valid"),
            (SelectionRange::collapsed(TextPosition::new(0, 0)), "\n"),
        ] {
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(commands
                .execute(
                    Box::new(MarkIndexEntryCommand::new(range, term.to_string())),
                    &mut context,
                )
                .is_err());
            assert_eq!(
                document.export_docx().expect("after invalid mark"),
                baseline
            );
            assert!(!commands.can_undo());
        }
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertIndex { after_paragraph: 0 }],
        };
        assert!(apply_headless_commands_to_document(&mut document, &request).is_err());
        assert_eq!(document.export_docx().expect("after empty index"), baseline);

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
                    HeadlessCommand::MarkIndexEntry {
                        // Current-state coordinates: paragraph 1 was removed by
                        // the preceding command and must not silently retarget.
                        paragraph: 1,
                        offset: 0,
                        term: "removed anchor".to_string(),
                    },
                ],
            },
        )
        .expect_err("deleted paragraph target must fail");
        assert!(error.to_string().contains("index 1"), "{error}");
        assert!(
            error.to_string().contains("target paragraph is missing")
                || error
                    .to_string()
                    .contains("index entries can only be marked in body paragraphs"),
            "{error}"
        );
        assert_eq!(document.paragraph_count(), 1);
        assert!(document.index_entries().is_empty());
    }
}
