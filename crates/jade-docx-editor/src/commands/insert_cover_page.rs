//! Atomic insertion of a styled cover page at the document start.

use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{CoverPageParagraphSpec, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

pub struct InsertCoverPageCommand {
    paragraphs: Vec<CoverPageParagraphSpec>,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertCoverPageCommand {
    pub fn new(paragraphs: Vec<CoverPageParagraphSpec>) -> Self {
        Self {
            paragraphs,
            previous_document: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertCoverPageCommand {
    fn name(&self) -> &'static str {
        "insert_cover_page"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx.document.insert_cover_page(&self.paragraphs) {
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
            .ok_or_else(|| CommandError::Custom("cover-page history is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
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
    use jade_docx_headless_api::{HeadlessCommand, HeadlessCoverParagraph, HeadlessEditRequest};
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

    fn build_docx(text: &str) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
              xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" \
              xmlns:x=\"urn:jade:cover-test\"><w:body>\
             <w:bookmarkStart w:id=\"7\" w:name=\"KeepOriginal\"/>\
             <w:p w14:paraId=\"00ABCDEF\"><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r><x:keep x:value=\"yes\"/></w:p>\
             <w:bookmarkEnd w:id=\"7\"/><w:sectPr/></w:body></w:document>",
            escape_text(text)
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

    fn cover_paragraph_strategy() -> impl Strategy<Value = CoverPageParagraphSpec> {
        (
            proptest::collection::vec(
                prop::sample::select(vec![
                    'A', 'z', ' ', '&', '<', '>', '表', '紙', '界', 'é', 'Ж', '🙂',
                ]),
                0..24,
            ),
            any::<bool>(),
            any::<bool>(),
            prop::option::of(prop::sample::select(vec![
                "#1F3864".to_string(),
                "2B579A".to_string(),
                "ffffff".to_string(),
                "595959".to_string(),
            ])),
            1u16..=144u16,
            prop::sample::select(vec![
                None,
                Some("left".to_string()),
                Some("center".to_string()),
                Some("right".to_string()),
            ]),
            0u16..=270u16,
            0u16..=270u16,
            prop::option::of(prop::sample::select(vec![
                "#262626".to_string(),
                "1C7C74".to_string(),
                "FFFFFF".to_string(),
            ])),
            0u8..16u8,
            0u16..=108u16,
            0u16..=108u16,
        )
            .prop_map(
                |(
                    text,
                    bold,
                    italic,
                    color,
                    half_points,
                    alignment,
                    before,
                    after,
                    shading_color,
                    border_mask,
                    indent_left,
                    indent_right,
                )| CoverPageParagraphSpec {
                    text: text.into_iter().collect(),
                    bold,
                    italic,
                    color,
                    font_size: Some(half_points as f32 / 2.0),
                    alignment,
                    space_before: Some(before as f32),
                    space_after: Some(after as f32),
                    shading_color,
                    border_top: border_mask & 1 != 0,
                    border_bottom: border_mask & 2 != 0,
                    border_left: border_mask & 4 != 0,
                    border_right: border_mask & 8 != 0,
                    indent_left: Some(indent_left as f32),
                    indent_right: Some(indent_right as f32),
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_cover_blocks_round_trip_preserve_original_and_restore_history(
            original_chars in proptest::collection::vec(
                prop::sample::select(vec!['B', '本', '文', '🙂', 'é', '&', '<', ' ']),
                1..28,
            ),
            paragraphs in proptest::collection::vec(cover_paragraph_strategy(), 1..8),
        ) {
            let original = original_chars.into_iter().collect::<String>();
            let source = build_docx(&original);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load cover document");
            let baseline = document.export_docx().expect("baseline");
            let baseline_bookmark = document
                .document
                .as_deref()
                .and_then(|document| document.bookmarks.iter().find(|bookmark| bookmark.name == "KeepOriginal"))
                .cloned()
                .expect("baseline bookmark");
            let original_selection = Some(SelectionRange::collapsed(TextPosition::new(0, original.len())));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(InsertCoverPageCommand::new(paragraphs.clone())),
                        &mut context,
                    )
                    .expect("insert cover page");
            }

            let inserted_count = paragraphs.len() + 1;
            prop_assert_eq!(document.paragraph_count(), inserted_count + 1);
            prop_assert_eq!(paragraph_text(&document, inserted_count), original.clone());
            prop_assert_eq!(
                document
                    .paragraph(paragraphs.len())
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|properties| properties.page_break_before),
                Some(true),
            );
            let title_index = paragraphs
                .iter()
                .position(|paragraph| !paragraph.text.is_empty())
                .unwrap_or(0);
            prop_assert_eq!(
                selection,
                Some(SelectionRange::collapsed(TextPosition::new(
                    title_index,
                    paragraphs[title_index].text.len(),
                ))),
            );
            let bookmark = document
                .document
                .as_deref()
                .and_then(|document| document.bookmarks.iter().find(|bookmark| bookmark.name == "KeepOriginal"))
                .expect("bookmark");
            prop_assert_eq!(
                bookmark.start_paragraph,
                baseline_bookmark.start_paragraph + inserted_count,
            );
            prop_assert_eq!(
                bookmark.end_paragraph,
                baseline_bookmark.end_paragraph + inserted_count,
            );

            let applied = document.export_docx().expect("applied export");
            let xml = document_xml(&applied);
            let cover_at = xml.find("<w:p").expect("cover paragraph XML");
            let bookmark_at = xml.find("<w:bookmarkStart").expect("bookmark XML");
            prop_assert!(cover_at < bookmark_at);
            prop_assert!(xml.contains("<x:keep x:value=\"yes\"/>"));
            prop_assert!(xml.contains("<w:pageBreakBefore"));

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen cover document");
            prop_assert_eq!(reopened.paragraph_count(), inserted_count + 1);
            prop_assert_eq!(paragraph_text(&reopened, inserted_count), original);
            for (index, expected) in paragraphs.iter().enumerate() {
                let paragraph = reopened.paragraph(index).expect("cover paragraph");
                prop_assert_eq!(paragraph_text(&reopened, index), expected.text.clone());
                let properties = paragraph.properties.as_ref().expect("paragraph properties");
                prop_assert_eq!(properties.alignment.as_deref(), expected.alignment.as_deref());
                prop_assert_eq!(properties.spacing.as_ref().and_then(|spacing| spacing.before), expected.space_before);
                prop_assert_eq!(properties.spacing.as_ref().and_then(|spacing| spacing.after), expected.space_after);
                prop_assert_eq!(properties.indent.as_ref().and_then(|indent| indent.left), expected.indent_left);
                prop_assert_eq!(properties.indent.as_ref().and_then(|indent| indent.right), expected.indent_right);
                if !expected.text.is_empty() {
                    let run = paragraph.runs.first().expect("cover run");
                    let run_properties = run.properties.as_ref().expect("run properties");
                    prop_assert_eq!(run_properties.bold == Some(true), expected.bold);
                    prop_assert_eq!(run_properties.italic == Some(true), expected.italic);
                    prop_assert_eq!(run_properties.font_size, expected.font_size);
                }
            }

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo cover"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, original_selection);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo cover"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn invalid_cover_specs_are_atomic_and_record_no_history() {
        let invalid = vec![
            Vec::new(),
            vec![CoverPageParagraphSpec {
                text: "X".repeat(2049),
                ..CoverPageParagraphSpec::default()
            }],
            vec![CoverPageParagraphSpec {
                text: "line\nbreak".to_string(),
                ..CoverPageParagraphSpec::default()
            }],
            vec![CoverPageParagraphSpec {
                text: "title".to_string(),
                color: Some("#xyzxyz".to_string()),
                ..CoverPageParagraphSpec::default()
            }],
            vec![CoverPageParagraphSpec {
                text: "title".to_string(),
                alignment: Some("justify".to_string()),
                ..CoverPageParagraphSpec::default()
            }],
            vec![CoverPageParagraphSpec {
                text: "title".to_string(),
                font_size: Some(f32::NAN),
                ..CoverPageParagraphSpec::default()
            }],
            vec![CoverPageParagraphSpec::default(); 17],
        ];

        for paragraphs in invalid {
            let source = build_docx("original");
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let original_selection = Some(SelectionRange::collapsed(TextPosition::new(0, 2)));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            let error = {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(InsertCoverPageCommand::new(paragraphs)),
                        &mut context,
                    )
                    .expect_err("reject invalid cover")
            };
            assert!(error.to_string().contains("cover page"));
            assert_eq!(document.export_docx().expect("after invalid"), baseline);
            assert_eq!(selection, original_selection);
            assert!(!commands.can_undo());
        }
    }

    #[test]
    fn headless_batch_uses_the_document_state_created_before_and_after_cover_insertion() {
        let source = build_docx("body");
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "EDIT-".to_string(),
                },
                HeadlessCommand::InsertCoverPage {
                    paragraphs: vec![HeadlessCoverParagraph {
                        text: "TITLE".to_string(),
                        bold: true,
                        alignment: Some("center".to_string()),
                        ..HeadlessCoverParagraph::default()
                    }],
                },
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 5,
                    text: "!".to_string(),
                },
                HeadlessCommand::InsertText {
                    paragraph: 2,
                    offset: 5,
                    text: "LIVE-".to_string(),
                },
            ],
        };
        apply_headless_commands_to_document(&mut document, &request).expect("apply batch");

        assert_eq!(document.paragraph_count(), 3);
        assert_eq!(paragraph_text(&document, 0), "TITLE!");
        assert!(document
            .paragraph(1)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .is_some_and(|properties| properties.page_break_before == Some(true)));
        assert_eq!(paragraph_text(&document, 2), "EDIT-LIVE-body");
        let reopened = executor::block_on(WordDocument::load(
            &document.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reopen");
        assert_eq!(paragraph_text(&reopened, 0), "TITLE!");
        assert_eq!(paragraph_text(&reopened, 2), "EDIT-LIVE-body");
    }
}
