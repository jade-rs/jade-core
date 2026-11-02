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
    use crate::commands::document_edit::DocumentEditCommand;
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

    #[derive(Clone, Debug)]
    enum CatalogOperation {
        Add(u8, u8),
        Replace(u8, u8, u8),
        Remove(u8),
        Invalid(u8, char),
        Undo,
        Redo,
        Reload,
    }

    fn catalog_operations() -> impl Strategy<Value = CatalogOperation> {
        prop_oneof![
            5 => (0u8..5, 0u8..6).prop_map(|(key, value)| CatalogOperation::Add(key, value)),
            5 => (0u8..5, 0u8..5, 0u8..6).prop_map(|(key, new_key, value)| CatalogOperation::Replace(key, new_key, value)),
            3 => (0u8..5).prop_map(CatalogOperation::Remove),
            3 => (0u8..6, prop::sample::select(vec!['\0', '\u{b}', '\u{fffe}', '\u{ffff}'])).prop_map(|(field, character)| CatalogOperation::Invalid(field, character)),
            2 => Just(CatalogOperation::Undo),
            2 => Just(CatalogOperation::Redo),
            1 => Just(CatalogOperation::Reload),
        ]
    }

    fn catalog_source(key: u8, value: u8) -> CitationSourceInfo {
        let character = [
            '\u{d7ff}',
            '\u{e000}',
            '\u{fffd}',
            '\u{10000}',
            '\u{10ffff}',
            '😀',
        ][usize::from(value)];
        source(
            format!("Source{key}"),
            "Book".into(),
            "著者 & <名>".into(),
            format!("題名{value}{character}"),
            "2026".into(),
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn citation_catalog_matches_history_model_after_every_operation(
            operations in proptest::collection::vec(catalog_operations(), 1..49),
            prefix in prop::sample::select(vec!["", "b:", "bib:"]),
        ) {
            let namespace = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
            let declaration = if prefix.is_empty() { "xmlns".into() } else { format!("xmlns:{}", prefix.trim_end_matches(':')) };
            let catalog = format!("<{prefix}Sources {declaration}=\"{namespace}\" StyleName=\"Custom\"/>");
            let foreign = format!("<x:Sources xmlns:x=\"urn:foreign\" xmlns:b=\"{namespace}\"><x:Source><x:Tag>Foreign</x:Tag></x:Source></x:Sources>");
            let relationships = concat!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId7\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../customXml/item1.xml\"/>",
                "<Relationship Id=\"rId8\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../customXml/item2.xml\"/>",
                "</Relationships>"
            );
            let bytes = build_docx("body", &[("customXml/item1.xml", &foreign), ("customXml/item2.xml", &catalog), ("word/_rels/document.xml.rels", relationships)]);
            let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 2)));
            let expected_selection = selection;
            let mut commands = CommandExecutor::new();
            let mut model = Vec::<CitationSourceInfo>::new();
            let mut undo = Vec::<Vec<CitationSourceInfo>>::new();
            let mut redo = Vec::<Vec<CitationSourceInfo>>::new();
            for (step, operation) in operations.iter().enumerate() {
                let before = document.export_docx().expect("before");
                let previous = model.clone();
                let mut context = CommandContext::new(&mut document, &mut selection);
                let mutation = match operation {
                    CatalogOperation::Add(key, value) => {
                        let entry = catalog_source(*key, *value);
                        let expected = !model.iter().any(|source| source.tag == entry.tag);
                        if expected { model.push(entry.clone()); }
                        let result = commands.execute(Box::new(AddCitationSourceCommand::new(entry)), &mut context);
                        prop_assert_eq!(result.is_ok(), expected, "step {}: {:?}", step, operation);
                        Some(expected)
                    }
                    CatalogOperation::Replace(key, new_key, value) => {
                        let tag = format!("Source{key}");
                        let entry = catalog_source(*new_key, *value);
                        let index = model.iter().position(|source| source.tag == tag);
                        let expected = index.is_some() && (key == new_key || !model.iter().any(|source| source.tag == entry.tag));
                        if expected { model[index.expect("model source")] = entry.clone(); }
                        let result = commands.execute(Box::new(DocumentEditCommand::new("replace_source", Box::new(move |document, _| document.replace_citation_source(&tag, entry.clone())))), &mut context);
                        prop_assert_eq!(result.is_ok(), expected, "step {}: {:?}", step, operation);
                        Some(expected)
                    }
                    CatalogOperation::Remove(key) => {
                        let tag = format!("Source{key}");
                        let index = model.iter().position(|source| source.tag == tag);
                        let expected = index.is_some();
                        if let Some(index) = index { model.remove(index); }
                        let result = commands.execute(Box::new(DocumentEditCommand::new("remove_source", Box::new(move |document, _| document.remove_citation_source(&tag)))), &mut context);
                        prop_assert_eq!(result.is_ok(), expected, "step {}: {:?}", step, operation);
                        Some(expected)
                    }
                    CatalogOperation::Invalid(field, character) => {
                        let mut entry = catalog_source(9, 0);
                        let invalid = format!("a{character}b");
                        match field {
                            0 => entry.tag = invalid,
                            1 => entry.author = invalid,
                            2 => entry.title = invalid,
                            3 => entry.year = invalid,
                            4 => entry.publisher = Some(invalid),
                            _ => entry.url = Some(invalid),
                        }
                        prop_assert!(commands.execute(Box::new(AddCitationSourceCommand::new(entry)), &mut context).is_err());
                        Some(false)
                    }
                    CatalogOperation::Undo => {
                        let expected = undo.pop();
                        prop_assert_eq!(commands.undo(&mut context).expect("undo"), expected.is_some());
                        if let Some(state) = expected { redo.push(model); model = state; }
                        None
                    }
                    CatalogOperation::Redo => {
                        let expected = redo.pop();
                        prop_assert_eq!(commands.redo(&mut context).expect("redo"), expected.is_some());
                        if let Some(state) = expected { undo.push(model); model = state; }
                        None
                    }
                    CatalogOperation::Reload => None,
                };
                if let Some(succeeded) = mutation {
                    if succeeded { undo.push(previous); redo.clear(); }
                    else { prop_assert_eq!(context.document.export_docx().expect("rejected"), before); }
                }
                let exported = context.document.export_docx().expect("export");
                prop_assert_eq!(context.document.citation_sources(), model.clone(), "step {}: {:?}", step, operation);
                prop_assert_eq!(*context.selection, expected_selection);
                prop_assert_eq!(zip_part(&exported, "customXml/item1.xml"), foreign.clone());
                let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
                prop_assert_eq!(reloaded.citation_sources(), model.clone(), "reload step {}: {:?}", step, operation);
                if let Some(directory) = std::env::var_os("JADE_FIELD_PBT_EXPORT_DIR") {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static NEXT: AtomicUsize = AtomicUsize::new(0);
                    std::fs::create_dir_all(&directory).expect("create export directory");
                    std::fs::write(std::path::Path::new(&directory).join(format!("catalog-sequence-{:06}.docx", NEXT.fetch_add(1, Ordering::Relaxed))), &exported).expect("write PBT export");
                }
                if matches!(operation, CatalogOperation::Reload) {
                    document = reloaded;
                    commands = CommandExecutor::new();
                    undo.clear();
                    redo.clear();
                }
            }
        }

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
            imported_empty_catalog in any::<bool>(),
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
            let bytes = if imported_empty_catalog {
                build_docx(&text, &[("customXml/item1.xml", "<b:Sources xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\"/>")])
            } else {
                build_docx(&text, &[])
            };
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
    fn appending_to_empty_catalog_preserves_attributes_trailing_xml_and_undo() {
        let namespace = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
        for (root, declaration) in [
            ("b:Sources", format!("xmlns:b=\"{namespace}\"")),
            ("Sources", format!("xmlns=\"{namespace}\"")),
        ] {
            for ending in ["/>", " />", "></PLACEHOLDER>"] {
                let ending = ending.replace("PLACEHOLDER", root);
                let opening = format!(
                    "<?xml version=\"1.0\"?>\n<{root} {declaration} StyleName='Custom > style' "
                );
                // A close-tag-like string after the root must not become the insertion point.
                let trailing = format!("\n<!-- </{root}> -->\n<?keep value?>\n");
                let original = format!("{opening}{ending}{trailing}");
                let bytes = build_docx("body", &[("customXml/item1.xml", &original)]);
                let mut document =
                    executor::block_on(WordDocument::load(&bytes, &Options::default()))
                        .expect("load empty catalog");
                let expected = source(
                    "First".into(),
                    "Book".into(),
                    "Author".into(),
                    "Title".into(),
                    "2026".into(),
                );
                let mut selection = None;
                let mut commands = CommandExecutor::new();
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(AddCitationSourceCommand::new(expected.clone())),
                        &mut context,
                    )
                    .expect("append to empty catalog");
                assert_eq!(context.document.citation_sources(), vec![expected.clone()]);
                let exported = context.document.export_docx().expect("export catalog");
                let xml = zip_part(&exported, "customXml/item1.xml");
                assert!(xml.starts_with(&opening), "{xml}");
                assert!(xml.ends_with(&trailing), "{xml}");
                let reloaded =
                    executor::block_on(WordDocument::load(&exported, &Options::default()))
                        .expect("reload exported catalog");
                assert_eq!(reloaded.citation_sources(), vec![expected.clone()]);
                assert!(commands.undo(&mut context).expect("undo first source"));
                assert_eq!(
                    zip_part(
                        &context.document.export_docx().expect("undo export"),
                        "customXml/item1.xml"
                    ),
                    original
                );
                assert!(commands.redo(&mut context).expect("redo first source"));
                assert_eq!(context.document.citation_sources(), vec![expected]);
                assert_eq!(
                    zip_part(
                        &context.document.export_docx().expect("redo export"),
                        "customXml/item1.xml"
                    ),
                    xml
                );
            }
        }
    }

    #[test]
    fn replacing_sources_is_atomic_and_preserves_catalog_order_and_unrelated_xml() {
        let namespace = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
        for prefix in ["", "b:", "bib:"] {
            let declaration = if prefix.is_empty() {
                "xmlns".into()
            } else {
                format!("xmlns:{}", prefix.trim_end_matches(':'))
            };
            let first = format!("<{prefix}Source><{prefix}Tag>A</{prefix}Tag><{prefix}SourceType>Book</{prefix}SourceType><{prefix}Title>First</{prefix}Title></{prefix}Source>");
            let last = format!("<{prefix}Source><{prefix}Tag>B</{prefix}Tag><{prefix}SourceType>Book</{prefix}SourceType><{prefix}Title>Last</{prefix}Title><{prefix}Volume>42</{prefix}Volume></{prefix}Source>");
            let catalog = format!("<?xml version=\"1.0\"?><{prefix}Sources {declaration}=\"{namespace}\" StyleName=\"Custom\"><!--before-->{first}<!--between--><x:Extra xmlns:x=\"urn:foreign\"/>{last}</{prefix}Sources><!--after-->");
            let bytes = build_docx("body", &[("customXml/item1.xml", &catalog)]);
            let mut document =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let baseline = document.export_docx().expect("baseline");
            for (tag, mut replacement) in [
                ("A", catalog_source(1, 0)),
                ("A", catalog_source(2, 0)),
                ("Missing", catalog_source(3, 0)),
            ] {
                if replacement.tag == "Source1" {
                    replacement.tag = "B".into();
                }
                if replacement.tag == "Source2" {
                    replacement.title = "a\u{ffff}b".into();
                }
                assert!(document.replace_citation_source(tag, replacement).is_err());
                assert_eq!(document.export_docx().expect("rejected export"), baseline);
            }
            let mut replacement = catalog_source(4, 0);
            replacement.tag = "Renamed".into();
            document
                .replace_citation_source("A", replacement.clone())
                .expect("replace first");
            let exported = document.export_docx().expect("export");
            let xml = zip_part(&exported, "customXml/item1.xml");
            let before = catalog.split_once(&first).expect("first entry");
            assert!(xml.starts_with(before.0));
            assert!(xml.ends_with(before.1));
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            let sources = reloaded.citation_sources();
            assert_eq!(sources[0], replacement);
            assert_eq!(sources[1].tag, "B");
            assert!(xml.contains(&last));
            document
                .replace_citation_source("B", catalog_source(0, 1))
                .expect("replace last after empty foreign element");
            assert_eq!(
                document
                    .citation_sources()
                    .iter()
                    .map(|source| source.tag.as_str())
                    .collect::<Vec<_>>(),
                ["Renamed", "Source0"]
            );
        }
    }

    #[test]
    fn authored_fields_reject_invalid_xml_before_changing_the_document() {
        use crate::word_document::FieldBlockLine;
        for character in ['\0', '\u{b}', '\u{fffe}', '\u{ffff}'] {
            let bytes = build_docx("body", &[]);
            let mut document =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let at = TextPosition::new(0, 2);
            document
                .insert_inline_field(at, "CITATION Source", "citation")
                .expect("inline field");
            document
                .add_citation_source(catalog_source(0, 0))
                .expect("source");
            document
                .insert_field_block(
                    0,
                    "BIBLIOGRAPHY",
                    &[FieldBlockLine {
                        text: "entry".into(),
                        ..Default::default()
                    }],
                )
                .expect("field block");
            let inline = document.inline_fields("CITATION").remove(0);
            let block = document
                .field_blocks()
                .into_iter()
                .find(|block| block.name() == "BIBLIOGRAPHY")
                .expect("block");
            let baseline = document.export_docx().expect("baseline");
            let invalid = format!("a{character}b");
            let instruction = format!("CITATION {invalid}");
            assert!(document
                .insert_inline_field(at, &instruction, "text")
                .is_err());
            assert!(document
                .insert_inline_field(at, "CITATION Source", &invalid)
                .is_err());
            assert!(document
                .set_inline_field(inline.paragraph, inline.run, Some(&instruction), "text")
                .is_err());
            assert!(document
                .set_inline_field(inline.paragraph, inline.run, None, &invalid)
                .is_err());
            assert!(document.insert_field_block(0, &instruction, &[]).is_err());
            assert!(document
                .replace_field_block(&block, &instruction, &[])
                .is_err());
            for field in 0..5 {
                let mut line = FieldBlockLine {
                    text: "entry".into(),
                    page: Some("1".into()),
                    ..Default::default()
                };
                match field {
                    0 => line.text = invalid.clone(),
                    1 => line.page = Some(invalid.clone()),
                    2 => line.separator = Some(invalid.clone()),
                    3 => line.leader = Some(invalid.clone()),
                    _ => line.style = Some(invalid.clone()),
                }
                assert!(document
                    .insert_field_block(0, "BIBLIOGRAPHY", &[line.clone()])
                    .is_err());
                assert!(document
                    .replace_field_block(&block, "BIBLIOGRAPHY", &[line])
                    .is_err());
            }
            assert!(document
                .set_bibliography_style(&invalid, "APA.xsl")
                .is_err());
            assert!(document.set_bibliography_style("APA", &invalid).is_err());
            assert_eq!(
                document.export_docx().expect("rejected operations"),
                baseline
            );
        }
    }

    #[test]
    fn citation_commands_reject_xml_noncharacters_without_mutating_history() {
        for character in ['\u{fffe}', '\u{ffff}'] {
            for field in 0..7 {
                let bytes = build_docx("body", &[]);
                let mut document =
                    executor::block_on(WordDocument::load(&bytes, &Options::default()))
                        .expect("load document");
                let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 2)));
                let mut commands = CommandExecutor::new();
                let mut context = CommandContext::new(&mut document, &mut selection);
                let valid = source(
                    "Valid".into(),
                    "Book".into(),
                    "著者".into(),
                    "題名".into(),
                    "2026".into(),
                );
                commands
                    .execute(
                        Box::new(AddCitationSourceCommand::new(valid.clone())),
                        &mut context,
                    )
                    .expect("add valid source");
                let baseline = context.document.export_docx().expect("baseline export");
                let original_selection = *context.selection;
                let mut invalid = source(
                    "Invalid".into(),
                    "Book".into(),
                    "著者".into(),
                    "題名".into(),
                    "2026".into(),
                );
                let text = format!("a{character}b");
                match field {
                    0 => invalid.tag = text.clone(),
                    1 => invalid.author = text.clone(),
                    2 => invalid.title = text.clone(),
                    3 => invalid.year = text.clone(),
                    4 => invalid.publisher = Some(text.clone()),
                    5 => invalid.url = Some(text.clone()),
                    _ => {}
                }
                let command: Box<dyn Command> = if field == 6 {
                    Box::new(InsertBibliographyCommand::new(0, text))
                } else {
                    Box::new(AddCitationSourceCommand::new(invalid))
                };
                let result = commands.execute(command, &mut context);
                assert!(result.is_err(), "accepted {character:?} in field {field}");
                assert_eq!(
                    context.document.export_docx().expect("rejected export"),
                    baseline
                );
                assert_eq!(*context.selection, original_selection);
                assert!(commands.undo(&mut context).expect("undo valid addition"));
                assert!(context.document.citation_sources().is_empty());
                assert!(commands.redo(&mut context).expect("redo valid addition"));
                assert_eq!(context.document.citation_sources(), vec![valid]);
            }
        }
    }

    #[test]
    fn bibliography_children_resolve_inherited_and_shadowed_namespaces() {
        let namespace = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
        for prefix in ["", "b:", "bib:"] {
            let declaration = if prefix.is_empty() {
                "xmlns".into()
            } else {
                format!("xmlns:{}", prefix.trim_end_matches(':'))
            };
            let foreign = "<x:Source xmlns:x=\"urn:foreign\"><x:Tag>A</x:Tag><x:Title>Foreign</x:Title></x:Source>";
            let shadowed = "<s:Source xmlns:s=\"urn:foreign\"><s:Tag>A</s:Tag></s:Source>";
            let actual = format!("<c:Source xmlns:c=\"{namespace}\" xmlns:x=\"urn:foreign\"><x:Tag>Wrong</x:Tag><x:Title>Wrong</x:Title><c:Tag>A</c:Tag><c:SourceType>Book</c:SourceType><c:Title>Actual</c:Title><c:Author><c:Author xmlns:c=\"urn:foreign\"><c:Last>Wrong</c:Last></c:Author><c:Author><c:NameList><c:Person><c:Last>Author</c:Last></c:Person></c:NameList></c:Author></c:Author></c:Source>");
            let catalog = format!("<{prefix}Sources {declaration}=\"{namespace}\">{foreign}{shadowed}{actual}</{prefix}Sources>");
            let bytes = build_docx("body", &[("customXml/item1.xml", &catalog)]);
            let mut document =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let sources = document.citation_sources();
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].tag, "A");
            assert_eq!(sources[0].title, "Actual");
            assert_eq!(sources[0].author, "Author");
            let replacement = catalog_source(0, 0);
            document
                .replace_citation_source("A", replacement.clone())
                .expect("replace namespaced source");
            let exported = document.export_docx().expect("export");
            let xml = zip_part(&exported, "customXml/item1.xml");
            assert!(xml.contains(foreign));
            assert!(xml.contains(shadowed));
            let mut reloaded =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("reload");
            assert_eq!(reloaded.citation_sources(), vec![replacement]);
            reloaded
                .remove_citation_source("Source0")
                .expect("remove actual source");
            assert!(reloaded.citation_sources().is_empty());
            assert!(reloaded.remove_citation_source("A").is_err());
            let xml = zip_part(
                &reloaded.export_docx().expect("removed export"),
                "customXml/item1.xml",
            );
            assert!(xml.contains(foreign));
            assert!(xml.contains(shadowed));
        }
    }

    #[test]
    fn foreign_sources_roots_are_not_bibliography_catalogs() {
        let namespace = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
        for root in ["Sources", "x:Sources"] {
            let foreign = format!(
                "<{root} xmlns=\"urn:foreign\" xmlns:x=\"urn:foreign\" xmlns:b=\"{namespace}\"><x:Source><x:Tag>Foreign</x:Tag></x:Source></{root}>"
            );
            let catalog = format!("<bib:Sources xmlns:bib=\"{namespace}\"/>");
            let bytes = build_docx(
                "body",
                &[
                    ("customXml/item1.xml", &foreign),
                    ("customXml/item2.xml", &catalog),
                ],
            );
            let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default()))
                .expect("load custom XML");
            assert!(
                document.citation_sources().is_empty(),
                "read unrelated custom XML"
            );
            let expected = source(
                "New".into(),
                "Book".into(),
                "著者".into(),
                "題名".into(),
                "2026".into(),
            );
            let mut selection = None;
            let mut commands = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(AddCitationSourceCommand::new(expected.clone())),
                    &mut context,
                )
                .expect("append actual bibliography");
            let exported = context.document.export_docx().expect("export");
            assert_eq!(zip_part(&exported, "customXml/item1.xml"), foreign);
            assert!(zip_part(&exported, "customXml/item2.xml").contains("<bib:Tag>New</bib:Tag>"));
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(reloaded.citation_sources(), vec![expected]);
            assert!(commands.undo(&mut context).expect("undo"));
            assert_eq!(
                zip_part(
                    &context.document.export_docx().expect("undo export"),
                    "customXml/item2.xml"
                ),
                catalog
            );
            assert!(commands.redo(&mut context).expect("redo"));
            assert_eq!(
                context.document.export_docx().expect("redo export"),
                exported
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
