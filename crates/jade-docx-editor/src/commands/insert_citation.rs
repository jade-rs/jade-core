//! Word-compatible bibliography source, citation, and bibliography commands.

use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{CitationSourceInfo, WordDocumentSnapshot};
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

pub struct AddCitationSourceCommand {
    source: CitationSourceInfo,
    previous: Option<SnapshotState>,
}

impl AddCitationSourceCommand {
    pub fn new(source: CitationSourceInfo) -> Self {
        Self {
            source,
            previous: None,
        }
    }
}

impl Command for AddCitationSourceCommand {
    fn name(&self) -> &'static str {
        "add_citation_source"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx.document.add_citation_source(self.source.clone()) {
            Ok(()) => {
                self.previous = Some(previous);
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
            .ok_or_else(|| CommandError::Custom("citation source snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

pub struct InsertCitationCommand {
    range: SelectionRange,
    tag: String,
    previous: Option<SnapshotState>,
}

impl InsertCitationCommand {
    pub fn new(range: SelectionRange, tag: String) -> Self {
        Self {
            range,
            tag,
            previous: None,
        }
    }
}

impl Command for InsertCitationCommand {
    fn name(&self) -> &'static str {
        "insert_citation"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let source = ctx
            .document
            .citation_source(self.tag.trim())
            .ok_or_else(|| CommandError::Custom("citation source was not found".into()))?;
        let previous = snapshot(ctx)?;
        let mut insert = InsertTextCommand::new(self.range, source.citation_text(), None);
        match insert.apply(ctx) {
            Ok(()) => {
                self.previous = Some(previous);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(error)
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("citation snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

pub struct InsertBibliographyCommand {
    after_paragraph: usize,
    heading: String,
    previous: Option<SnapshotState>,
}

impl InsertBibliographyCommand {
    pub fn new(after_paragraph: usize, heading: String) -> Self {
        Self {
            after_paragraph,
            heading,
            previous: None,
        }
    }
}

impl Command for InsertBibliographyCommand {
    fn name(&self) -> &'static str {
        "insert_bibliography"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx
            .document
            .insert_bibliography_after_paragraph(self.after_paragraph, &self.heading)
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
            .ok_or_else(|| CommandError::Custom("bibliography snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::headless_api::apply_snapshot_headless_commands_with_executor_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCitationSource, HeadlessCommand, HeadlessEditRequest};
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

    fn build_docx(text: &str, extra_parts: &[(&str, &str)]) -> Vec<u8> {
        let runs = text
            .chars()
            .map(|character| {
                format!(
                    "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                    escape_text(&character.to_string())
                )
            })
            .collect::<String>();
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p>{runs}</w:p></w:body></w:document>"
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
                zip.start_file(path, options).expect("start base entry");
                zip.write_all(value.as_bytes()).expect("write base entry");
            }
            for (path, value) in extra_parts {
                zip.start_file(path, options).expect("start extra entry");
                zip.write_all(value.as_bytes()).expect("write extra entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    fn zip_part(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive.by_name(path).expect("open part");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read part");
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

    fn source(
        tag: String,
        source_type: String,
        author: String,
        title: String,
        year: String,
    ) -> CitationSourceInfo {
        CitationSourceInfo {
            tag,
            source_type,
            author,
            title,
            year,
            publisher: Some("出版 & <社>".to_string()),
            url: Some("https://example.test/?a=1&b=2".to_string()),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn citation_catalog_text_and_bibliography_round_trip_across_unicode_run_boundaries(
            text_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', 'z', 'あ', '界', '😀', 'é', ' ']),
                1..28,
            ),
            author_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', '著', '😀', 'é', '&', '<', '>', ',', ' ']),
                1..24,
            ),
            title_chars in proptest::collection::vec(
                prop::sample::select(vec!['T', '題', '😀', 'é', '&', '<', '>', ' ']),
                1..28,
            ),
            source_type in prop::sample::select(vec!["Book", "JournalArticle", "InternetSite", "Report", "Misc"]),
            offset_pick in any::<usize>(),
            tag_number in any::<u32>(),
        ) {
            let text = text_chars.into_iter().collect::<String>();
            let author = author_chars.into_iter().collect::<String>().trim().to_string();
            let title = title_chars.into_iter().collect::<String>().trim().to_string();
            prop_assume!(!title.is_empty());
            let offset = boundaries(&text)[offset_pick % boundaries(&text).len()];
            let source = source(
                format!("Source{tag_number}"),
                source_type.to_string(),
                author,
                title,
                "2026".to_string(),
            );
            let mut expected_source = source.clone();
            if let Some((last, first)) = expected_source.author.split_once(',') {
                let last = last.trim();
                let first = first.trim();
                if !last.is_empty() && !first.is_empty() {
                    expected_source.author = format!("{last}, {first}");
                }
            }
            let expected_citation = expected_source.citation_text();
            let expected_line = expected_source.bibliography_line();
            let bytes = build_docx(&text, &[]);
            let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let range = SelectionRange::collapsed(TextPosition::new(0, offset));
            let mut selection = Some(range);
            let mut commands = CommandExecutor::new();

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(AddCitationSourceCommand::new(source)),
                    &mut context,
                ).expect("add source");
                commands.execute(
                    Box::new(InsertCitationCommand::new(range, expected_source.tag.clone())),
                    &mut context,
                ).expect("insert citation");
                commands.execute(
                    Box::new(InsertBibliographyCommand::new(0, "参考文献".to_string())),
                    &mut context,
                ).expect("insert bibliography");
            }

            prop_assert_eq!(document.citation_sources(), vec![expected_source.clone()]);
            let first = document.paragraph(0).expect("first paragraph")
                .runs.iter().map(|run| run.text.as_str()).collect::<String>();
            prop_assert_eq!(first, format!("{}{}{}", &text[..offset], expected_citation, &text[offset..]));
            prop_assert_eq!(document.paragraph_count(), 3);
            let bibliography = document.paragraph(2).expect("bibliography line")
                .runs.iter().map(|run| run.text.as_str()).collect::<String>();
            prop_assert_eq!(bibliography, expected_line);

            let applied = document.export_docx().expect("export applied");
            let source_xml = zip_part(&applied, "customXml/item1.xml");
            prop_assert!(source_xml.contains("<b:Sources"));
            prop_assert!(source_xml.contains("出版 &amp; &lt;社&gt;"));
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen document");
            prop_assert_eq!(reopened.citation_sources(), vec![expected_source]);
            prop_assert_eq!(reopened.paragraph_count(), 3);

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                for _ in 0..3 {
                    prop_assert!(commands.undo(&mut context).expect("undo"));
                }
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                for _ in 0..3 {
                    prop_assert!(commands.redo(&mut context).expect("redo"));
                }
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }

        #[test]
        fn snapshot_batches_rebase_citations_and_following_edits(
            text_chars in proptest::collection::vec(
                prop::sample::select(vec!['a', '界', '😀', 'é']),
                2..24,
            ),
            offset_pick in any::<usize>(),
        ) {
            let text = text_chars.into_iter().collect::<String>();
            let candidates = boundaries(&text)
                .into_iter()
                .filter(|offset| *offset > 0 && *offset < text.len())
                .collect::<Vec<_>>();
            let offset = candidates[offset_pick % candidates.len()];
            let source = HeadlessCitationSource {
                tag: "Coord2026".to_string(),
                source_type: "Book".to_string(),
                author: "座標, 太郎".to_string(),
                title: "境界".to_string(),
                year: "2026".to_string(),
                publisher: None,
                url: None,
            };
            let citation = "(座標, 太郎, 2026)";
            let bytes = build_docx(&text, &[]);
            let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default()))
                .expect("load document");
            let mut executor = CommandExecutor::new();
            apply_snapshot_headless_commands_with_executor_to_document(
                &mut document,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::AddCitationSource { source },
                        HeadlessCommand::InsertText {
                            paragraph: 0,
                            offset: 0,
                            text: "前".to_string(),
                        },
                        HeadlessCommand::InsertCitation {
                            start_paragraph: 0,
                            start_offset: offset,
                            end_paragraph: 0,
                            end_offset: offset,
                            tag: "Coord2026".to_string(),
                        },
                        HeadlessCommand::InsertText {
                            paragraph: 0,
                            offset: text.len(),
                            text: "後".to_string(),
                        },
                    ],
                },
            ).expect("apply snapshot batch");
            let actual = document.paragraph(0).expect("paragraph")
                .runs.iter().map(|run| run.text.as_str()).collect::<String>();
            prop_assert_eq!(
                actual,
                format!("前{}{}{}後", &text[..offset], citation, &text[offset..])
            );
        }
    }

    #[test]
    fn appending_source_preserves_unknown_fields_and_root_style() {
        let source_xml = concat!(
            "<b:Sources SelectedStyle=\"\\CustomStyle.xsl\" StyleName=\"Custom\" Version=\"9\" ",
            "xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\">",
            "<b:Source><b:Tag>Existing</b:Tag><b:SourceType>JournalArticle</b:SourceType>",
            "<b:Author><b:Author><b:NameList><b:Person><b:Last>One</b:Last><b:First>A</b:First></b:Person>",
            "<b:Person><b:Last>Two</b:Last><b:First>B</b:First></b:Person></b:NameList></b:Author></b:Author>",
            "<b:Title>Original</b:Title><b:Volume>42</b:Volume><b:Pages>1-9</b:Pages><b:DOI>10.1/x</b:DOI></b:Source>",
            "</b:Sources>"
        );
        let relationships = concat!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
            "<Relationship Id=\"rId7\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../customXml/item1.xml\"/>",
            "</Relationships>"
        );
        let bytes = build_docx(
            "body",
            &[
                ("word/_rels/document.xml.rels", relationships),
                ("customXml/item1.xml", source_xml),
            ],
        );
        let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default()))
            .expect("load imported sources");
        document
            .add_citation_source(source(
                "New".to_string(),
                "Report".to_string(),
                "新, 著者".to_string(),
                "追加".to_string(),
                "2026".to_string(),
            ))
            .expect("append source");
        let exported = document.export_docx().expect("export sources");
        let xml = zip_part(&exported, "customXml/item1.xml");
        assert!(xml.contains("SelectedStyle=\"\\CustomStyle.xsl\""), "{xml}");
        assert!(xml.contains("<b:Volume>42</b:Volume>"), "{xml}");
        assert!(xml.contains("<b:Pages>1-9</b:Pages>"), "{xml}");
        assert!(xml.contains("<b:DOI>10.1/x</b:DOI>"), "{xml}");
        assert!(xml.contains("<b:Last>Two</b:Last>"), "{xml}");
        assert_eq!(document.citation_sources().len(), 2);
    }
}
