//! Hyperlink authoring commands.
//!
//! Hyperlinks touch both paragraph XML and the main document relationship
//! part, so undo snapshots the package as one atomic unit.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

pub struct InsertHyperlinkCommand {
    range: SelectionRange,
    display_text: Option<String>,
    target: String,
    tooltip: Option<String>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertHyperlinkCommand {
    pub fn new(
        range: SelectionRange,
        display_text: Option<String>,
        target: String,
        tooltip: Option<String>,
    ) -> Self {
        Self {
            range,
            display_text,
            target,
            tooltip,
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertHyperlinkCommand {
    fn name(&self) -> &'static str {
        "insert_hyperlink"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let previous_selection = *ctx.selection;
        match ctx.document.insert_hyperlink_at_selection(
            self.range,
            self.display_text.as_deref(),
            &self.target,
            self.tooltip.as_deref(),
        ) {
            Ok(selection_after) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                *ctx.selection = Some(selection_after);
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
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("hyperlink snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

pub struct RemoveHyperlinkCommand {
    range: SelectionRange,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl RemoveHyperlinkCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for RemoveHyperlinkCommand {
    fn name(&self) -> &'static str {
        "remove_hyperlink"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let previous_selection = *ctx.selection;
        match ctx.document.remove_hyperlink_at_selection(self.range) {
            Ok(selection_after) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                *ctx.selection = Some(selection_after);
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
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("hyperlink snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx(paragraphs: &[String]) -> Vec<u8> {
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
        let body = paragraphs
            .iter()
            .map(|paragraph| {
                let runs = paragraph
                    .chars()
                    .map(|ch| {
                        format!(
                            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                            xml_escape(&ch.to_string())
                        )
                    })
                    .collect::<String>();
                format!("<w:p>{runs}</w:p>")
            })
            .collect::<String>();
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
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
                zip.start_file(path, options).expect("start entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    fn part(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open docx");
        let mut entry = archive.by_name(path).expect("part");
        let mut value = String::new();
        entry.read_to_string(&mut value).expect("read part");
        value
    }

    fn text_boundaries(text: &str) -> Vec<usize> {
        let mut boundaries = text
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(text.len());
        boundaries
    }

    fn linked_spans(
        document: &WordDocument,
        paragraph_index: usize,
    ) -> Vec<(usize, usize, String)> {
        let paragraph = document.paragraph(paragraph_index).expect("paragraph");
        let mut offset = 0usize;
        let mut spans = Vec::new();
        for run in &paragraph.runs {
            let end = offset + run.text.len();
            if let Some(link) = run.hyperlink.as_ref() {
                let target = link
                    .anchor
                    .as_ref()
                    .map(|anchor| format!("#{anchor}"))
                    .or_else(|| link.target.clone())
                    .unwrap_or_default();
                spans.push((offset, end, target));
            }
            offset = end;
        }
        spans
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn authored_links_cross_run_boundaries_and_round_trip_at_utf8_boundaries(
            chars in proptest::collection::vec(
                prop::sample::select(vec!['a', 'Z', '0', ' ', 'あ', '界', '😀', 'é']),
                1..24,
            ),
            first_pick in any::<usize>(),
            second_pick in any::<usize>(),
            internal in any::<bool>(),
        ) {
            let text = chars.into_iter().collect::<String>();
            let boundaries = text_boundaries(&text);
            let first = first_pick % boundaries.len();
            let mut second = second_pick % boundaries.len();
            if second == first {
                second = (second + 1) % boundaries.len();
            }
            let (start_index, end_index) = if first < second { (first, second) } else { (second, first) };
            let start = boundaries[start_index];
            let end = boundaries[end_index];
            prop_assume!(start < end);

            let source = build_docx(std::slice::from_ref(&text));
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load generated docx");
            let baseline = document.export_docx().expect("baseline export");
            let range = SelectionRange::new(TextPosition::new(0, start), TextPosition::new(0, end));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let target = if internal { "#章_一" } else { "https://example.com/資料?q=1&x=2" };
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(InsertHyperlinkCommand::new(
                        range,
                        None,
                        target.to_string(),
                        Some("説明 & <link>".to_string()),
                    )),
                    &mut ctx,
                ).expect("author hyperlink");
            }

            let spans = linked_spans(&document, 0);
            prop_assert!(!spans.is_empty());
            prop_assert_eq!(spans.first().map(|span| span.0), Some(start));
            prop_assert_eq!(spans.last().map(|span| span.1), Some(end));
            prop_assert!(spans.iter().all(|span| span.2 == target));

            let applied = document.export_docx().expect("export linked docx");
            let document_xml = part(&applied, "word/document.xml");
            prop_assert_eq!(document_xml.matches("<w:hyperlink").count(), 1);
            prop_assert!(document_xml.contains("w:tooltip=\"説明 &amp; &lt;link>\""));
            prop_assert!(document_xml.contains("w:rStyle"));
            prop_assert!(document_xml.contains("w:val=\"Hyperlink\""));
            let rels = part(&applied, "word/_rels/document.xml.rels");
            if internal {
                prop_assert!(document_xml.contains("w:anchor=\"章_一\""));
                prop_assert!(!document_xml.contains("r:id="));
                prop_assert!(!rels.contains("relationships/hyperlink"));
            } else {
                prop_assert!(document_xml.contains("r:id=\"rId"));
                prop_assert!(rels.contains("relationships/hyperlink"));
                prop_assert!(rels.contains("TargetMode=\"External\""));
                prop_assert!(rels.contains("Target=\"https://example.com/資料?q=1&amp;x=2\""));
            }

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen linked docx");
            let reopened_spans = linked_spans(&reopened, 0);
            prop_assert_eq!(reopened_spans.first().map(|span| span.0), Some(start));
            prop_assert_eq!(reopened_spans.last().map(|span| span.1), Some(end));
            prop_assert!(reopened_spans.iter().all(|span| span.2 == target));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo link"));
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), baseline);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo link"));
            }
            let redone = document.export_docx().expect("export redo");
            prop_assert_eq!(redone.as_slice(), applied.as_slice());

            // Removing at any valid caret inside the authored span must clear the entire
            // contiguous hyperlink, including when its text crosses arbitrary UTF-8/run
            // boundaries. History must restore both the document XML and relationship state.
            let caret_boundary = start_index + (end_index - start_index) / 2;
            let caret = SelectionRange::collapsed(TextPosition::new(0, boundaries[caret_boundary]));
            selection = Some(caret);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(RemoveHyperlinkCommand::new(caret)),
                    &mut ctx,
                ).expect("remove hyperlink at caret");
            }
            prop_assert!(linked_spans(&document, 0).is_empty());
            let removed = document.export_docx().expect("export removed link");
            prop_assert!(!part(&removed, "word/document.xml").contains("<w:hyperlink"));
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo hyperlink removal"));
            }
            let restored = document.export_docx().expect("export restored link");
            prop_assert_eq!(restored.as_slice(), applied.as_slice());
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo hyperlink removal"));
            }
            let re_removed = document.export_docx().expect("export re-removed link");
            prop_assert_eq!(re_removed.as_slice(), removed.as_slice());
        }

        #[test]
        fn collapsed_insertions_replace_no_text_and_round_trip(
            prefix in proptest::collection::vec(prop::sample::select(vec!['a', 'あ', '😀']), 0..12),
            suffix in proptest::collection::vec(prop::sample::select(vec!['b', '界', 'é']), 0..12),
            label in proptest::collection::vec(prop::sample::select(vec!['L', 'i', 'n', 'k', '文', '😀']), 1..10),
        ) {
            let prefix = prefix.into_iter().collect::<String>();
            let suffix = suffix.into_iter().collect::<String>();
            let label = label.into_iter().collect::<String>();
            let original = format!("{prefix}{suffix}");
            let source = build_docx(std::slice::from_ref(&original));
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load generated docx");
            let range = SelectionRange::collapsed(TextPosition::new(0, prefix.len()));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(InsertHyperlinkCommand::new(
                        range,
                        Some(label.clone()),
                        "mailto:test@example.com".to_string(),
                        None,
                    )),
                    &mut ctx,
                ).expect("insert hyperlink text");
            }
            let text_after = document.paragraph(0).expect("paragraph").runs.iter()
                .map(|run| run.text.as_str()).collect::<String>();
            prop_assert_eq!(text_after, format!("{prefix}{label}{suffix}"));
            let spans = linked_spans(&document, 0);
            prop_assert_eq!(spans.first().map(|span| (span.0, span.1)), Some((prefix.len(), prefix.len() + label.len())));
            prop_assert_eq!(selection, Some(SelectionRange::collapsed(TextPosition::new(0, prefix.len() + label.len()))));

            let applied = document.export_docx().expect("export");
            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen");
            prop_assert_eq!(linked_spans(&reopened, 0).first().map(|span| (span.0, span.1)), Some((prefix.len(), prefix.len() + label.len())));
        }
    }

    #[test]
    fn multi_paragraph_selection_authors_separate_wrappers_and_can_be_removed() {
        let source = build_docx(&["first".to_string(), "second".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertHyperlinkCommand::new(
                        range,
                        None,
                        "https://example.com/multi".to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("apply link");
        }
        let applied = document.export_docx().expect("export linked");
        assert_eq!(
            part(&applied, "word/document.xml")
                .matches("<w:hyperlink")
                .count(),
            2
        );
        let first_spans = linked_spans(&document, 0);
        let second_spans = linked_spans(&document, 1);
        assert_eq!(first_spans.first().map(|span| span.0), Some(2));
        assert_eq!(first_spans.last().map(|span| span.1), Some(5));
        assert_eq!(second_spans.first().map(|span| span.0), Some(0));
        assert_eq!(second_spans.last().map(|span| span.1), Some(3));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(RemoveHyperlinkCommand::new(range)), &mut ctx)
                .expect("remove link");
        }
        assert!(linked_spans(&document, 0).is_empty());
        assert!(linked_spans(&document, 1).is_empty());
        assert!(!part(
            &document.export_docx().expect("export removed"),
            "word/document.xml"
        )
        .contains("<w:hyperlink"));
    }

    #[test]
    fn invalid_targets_and_utf8_offsets_are_atomic_and_do_not_enter_history() {
        let source = build_docx(&["あA".to_string()]);
        for (range, target, display) in [
            (
                SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(0, 3)),
                "https://example.com",
                None,
            ),
            (
                SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 3)),
                "javascript:alert(1)",
                None,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 0)),
                "https://example.com",
                None,
            ),
        ] {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load docx");
            let baseline = document.export_docx().expect("baseline");
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor
                .execute(
                    Box::new(InsertHyperlinkCommand::new(
                        range,
                        display.map(str::to_string),
                        target.to_string(),
                        None,
                    )),
                    &mut ctx,
                )
                .is_err());
            assert!(!executor.can_undo());
            assert_eq!(document.export_docx().expect("export unchanged"), baseline);
        }
    }
}
