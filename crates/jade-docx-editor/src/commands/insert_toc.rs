use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::TocOptions;
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

/// Inserts a generated table of contents after the given paragraph (Word's
/// References ▸ Table of Contents) and supports undo by restoring a full
/// document-and-package snapshot.
///
/// A TOC insert touches several places at once — it appends multiple entry
/// paragraphs (model + raw XML) and may create `_Toc…` bookmarks inside existing
/// heading paragraphs. Reversing each mutation individually is error-prone, and
/// a TOC insert is a rare, deliberate action, so we snapshot the document before
/// applying and restore it on undo. The package state is included because later
/// comments can coexist with TOC bookmarks; undoing the commands in sequence
/// must not leave an unanchored `comments.xml` entry behind.
pub struct InsertTableOfContentsCommand {
    after_paragraph: usize,
    options: TocOptions,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertTableOfContentsCommand {
    pub fn new(after_paragraph: usize, options: TocOptions) -> Self {
        Self {
            after_paragraph,
            options,
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for InsertTableOfContentsCommand {
    fn name(&self) -> &'static str {
        "insert_table_of_contents"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;

        ctx.document
            .insert_table_of_contents(self.after_paragraph, &self.options)
            .map_err(CommandError::Custom)?;

        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("insert_table_of_contents snapshot is missing".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// Regenerates the first existing table of contents from the current headings
/// (Word's References ▸ Update Table). Snapshot-based undo, like insert.
pub struct UpdateTableOfContentsCommand {
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl Default for UpdateTableOfContentsCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateTableOfContentsCommand {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for UpdateTableOfContentsCommand {
    fn name(&self) -> &'static str {
        "update_table_of_contents"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;

        ctx.document
            .update_table_of_contents()
            .map_err(CommandError::Custom)?;

        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("update_table_of_contents snapshot is missing".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, ParagraphProperties, Run, RunField, RunFieldKind,
        TocEntryField, TocSequencePageNumber,
    };
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn heading(text: &str, level: u8) -> BodyElement {
        // 0-based applied outline level: Heading 1 -> w:outlineLvl 0.
        let props = ParagraphProperties {
            outline_level: Some(level - 1),
            ..ParagraphProperties::default()
        };
        BodyElement::from_paragraph(Paragraph {
            runs: vec![run(text)],
            properties: Some(props),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        })
    }

    fn styled_paragraph(text: &str, style: &str) -> BodyElement {
        BodyElement::from_paragraph(Paragraph {
            runs: vec![run(text)],
            properties: Some(ParagraphProperties {
                style: Some(style.into()),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        })
    }

    fn run(text: &str) -> Run {
        Run {
            text: text.to_string(),
            properties: None,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        }
    }

    fn doc_with_headings() -> WordDocument {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![run("Title")],
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    heading("Chapter 1", 1),
                    heading("Section 1.1", 2),
                    heading("Chapter 2", 1),
                ],
            }),
            ..Document::default()
        };
        WordDocument::from_document(document)
    }

    fn paragraph_count(doc: &WordDocument) -> usize {
        doc.document
            .as_ref()
            .and_then(|d| d.body.as_ref())
            .map(|b| b.elements.len())
            .unwrap_or(0)
    }

    #[test]
    fn insert_table_of_contents_supports_undo_and_redo() {
        let mut doc = doc_with_headings();
        let before = paragraph_count(&doc);
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: false,
                            hide_page_number_levels: None,
                            custom_style_levels: Vec::new(),
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert toc");
        }
        // Three headings (Chapter 1, Section 1.1, Chapter 2) -> three entries.
        assert_eq!(paragraph_count(&doc), before + 3);
        // A new _Toc bookmark per heading was created (none existed before).
        assert_eq!(
            doc.document.as_ref().unwrap().bookmarks.len(),
            3,
            "one _Toc bookmark per heading"
        );

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo");
        }
        assert_eq!(paragraph_count(&doc), before);
        assert_eq!(doc.document.as_ref().unwrap().bookmarks.len(), 0);

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.redo(&mut ctx).expect("redo");
        }
        assert_eq!(paragraph_count(&doc), before + 3);
    }

    #[test]
    fn insert_table_of_contents_honors_instr_text_page_number_omission() {
        let mut doc = doc_with_headings();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: true,
                            hide_page_number_levels: None,
                            custom_style_levels: Vec::new(),
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert toc without page numbers");
        }

        let body = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .expect("document body");
        // Insertion after the leading body paragraph places the three generated
        // entries at body indices 1..=3. The `TOC \\n` switch suppresses the
        // complete page-number presentation: tab, leader stop, and PAGEREF.
        for element in &body.elements[1..=3] {
            let paragraph = element.as_paragraph().expect("TOC paragraph");
            assert!(paragraph.properties.as_ref().unwrap().tabs.is_empty());
            assert!(paragraph.runs.iter().all(|run| run.text != "\t"));
            assert!(paragraph.runs.iter().all(|run| run.field.is_none()));
        }
    }

    #[test]
    fn insert_tc_table_of_contents_honors_per_entry_page_number_omission() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("Insert after me", "Normal"),
                    styled_paragraph("Architecture source", "Normal"),
                    styled_paragraph("Deployment source", "Normal"),
                ],
            }),
            toc_entry_fields: vec![
                TocEntryField {
                    text: "Architecture overview".to_string(),
                    identifier: Some("i".to_string()),
                    level: 2,
                    hide_page_number: false,
                    paragraph: 1,
                },
                TocEntryField {
                    text: "Deployment map".to_string(),
                    identifier: Some("i".to_string()),
                    level: 3,
                    hide_page_number: true,
                    paragraph: 2,
                },
            ],
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);
        doc.insert_table_of_contents(
            0,
            &TocOptions {
                use_toc_entry_fields: true,
                toc_entry_identifier: Some("i".to_string()),
                toc_entry_levels: Some((2, 3)),
                hyperlinks: true,
                ..TocOptions::default()
            },
        )
        .expect("insert TC-backed TOC");

        let body = doc.document.as_ref().unwrap().body.as_ref().unwrap();
        let first = body.elements[1].as_paragraph().unwrap();
        let second = body.elements[2].as_paragraph().unwrap();
        assert!(first.runs.iter().any(|run| run.field.is_some()));
        assert!(second.runs.iter().all(|run| run.field.is_none()));
        assert!(second.runs.iter().all(|run| run.text != "\t"));

        // Exercise the raw OOXML update path on the generated fixture too. It
        // must retain the TC selector switches instead of rewriting the field
        // as a heading-based `TOC \o` instruction.
        let data = std::fs::read(fixture_path("toc-tc-field-edge-cases.docx"))
            .expect("read TC field fixture");
        let mut loaded = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load TC field fixture");
        loaded
            .update_table_of_contents()
            .expect("update TC-backed TOC");
        let xml = loaded
            .raw_document
            .as_ref()
            .expect("raw document")
            .to_string()
            .expect("serialize raw document XML");
        assert!(xml.contains(r#"TOC \h \z \f i \l "2-3""#), "{xml}");
    }

    #[test]
    fn insert_ordinary_toc_merges_untyped_tc_fields_with_headings() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("Insert after me", "Normal"),
                    heading("First heading", 1),
                    styled_paragraph("Manual source", "Normal"),
                    styled_paragraph("Typed source", "Normal"),
                    heading("Last heading", 2),
                ],
            }),
            toc_entry_fields: vec![
                // Word treats a TC field without `\\f` as part of an ordinary
                // heading TOC. Its `\\n` switch applies only to this entry.
                TocEntryField {
                    text: "Manual entry".to_string(),
                    identifier: None,
                    level: 2,
                    hide_page_number: true,
                    paragraph: 2,
                },
                // A typed TC field is reserved for the corresponding
                // `TOC \\f i` field and must not leak into the default TOC.
                TocEntryField {
                    text: "Illustration only".to_string(),
                    identifier: Some("i".to_string()),
                    level: 2,
                    hide_page_number: false,
                    paragraph: 3,
                },
            ],
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);
        doc.insert_table_of_contents(0, &TocOptions::default())
            .expect("insert ordinary TOC with an untyped TC entry");

        let body = doc.document.as_ref().unwrap().body.as_ref().unwrap();
        let generated: Vec<&Paragraph> = body.elements[1..=3]
            .iter()
            .map(|element| element.as_paragraph().expect("generated TOC paragraph"))
            .collect();
        let labels: Vec<&str> = generated
            .iter()
            .map(|paragraph| paragraph.runs[0].text.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["First heading", "Manual entry", "Last heading"]
        );

        assert!(generated[0].runs.iter().any(|run| run.field.is_some()));
        assert!(generated[1].runs.iter().all(|run| run.field.is_none()));
        assert!(generated[1].runs.iter().all(|run| run.text != "\t"));
        assert!(generated[2].runs.iter().any(|run| run.field.is_some()));
    }

    #[test]
    fn insert_table_of_contents_honors_ranged_instr_text_page_number_omission() {
        let mut doc = doc_with_headings();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: false,
                            hide_page_number_levels: Some((2, 2)),
                            custom_style_levels: Vec::new(),
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert toc with ranged page-number omission");
        }

        let body = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .expect("document body");
        let entries: Vec<_> = body.elements[1..=3]
            .iter()
            .map(|element| element.as_paragraph().expect("TOC paragraph"))
            .collect();

        // The source headings are levels 1, 2, 1. `TOC \\n "2-2"` removes
        // only the middle entry's page presentation and preserves both level-1
        // PAGEREFs.
        assert!(entries[0].runs.iter().any(|run| run.field.is_some()));
        assert!(entries[1].runs.iter().all(|run| run.field.is_none()));
        assert!(entries[1].properties.as_ref().unwrap().tabs.is_empty());
        assert!(entries[2].runs.iter().any(|run| run.field.is_some()));
    }

    #[test]
    fn insert_table_of_contents_collects_instr_text_custom_styles() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("Title", "Normal"),
                    styled_paragraph("Appendix A", "AppendixTitle"),
                ],
            }),
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: false,
                            hide_page_number_levels: None,
                            custom_style_levels: vec![("AppendixTitle".to_string(), 2)],
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert TOC using custom style mapping");
        }

        let inserted = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .expect("inserted TOC paragraph");
        assert_eq!(inserted.runs[0].text, "Appendix A");
        assert_eq!(
            inserted.properties.as_ref().unwrap().style.as_deref(),
            Some("TOC2")
        );
    }

    #[test]
    fn insert_table_of_contents_honors_missing_instr_text_hyperlink_switch() {
        let mut doc = doc_with_headings();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: false,
                            hide_page_numbers: false,
                            hide_page_number_levels: None,
                            custom_style_levels: Vec::new(),
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert non-hyperlinked TOC");
        }

        let body = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .expect("document body");
        for element in &body.elements[1..=3] {
            let paragraph = element.as_paragraph().expect("TOC paragraph");
            assert!(paragraph.runs.iter().all(|run| run.hyperlink.is_none()));
            // Omitting `\h` removes navigation, not the visible page number.
            assert!(paragraph.runs.iter().any(|run| run.field.is_some()));
        }
    }

    #[test]
    fn insert_table_of_contents_honors_instr_text_page_number_separator() {
        let mut doc = doc_with_headings();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: false,
                            hide_page_number_levels: None,
                            custom_style_levels: Vec::new(),
                            page_number_separator: Some("---".to_string()),
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert TOC with a custom page-number separator");
        }

        let body = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .expect("document body");
        for element in &body.elements[1..=3] {
            let paragraph = element.as_paragraph().expect("TOC paragraph");
            // Word's `TOC \\p \"---\"` replaces the complete default tab
            // presentation, not merely the tab glyph. No right-aligned stop or
            // dot leader remains, while the PAGEREF result is still present.
            assert!(paragraph.properties.as_ref().unwrap().tabs.is_empty());
            assert!(paragraph.runs.iter().all(|run| run.text != "\t"));
            assert!(paragraph.runs.iter().any(|run| run.text == "-"));
            assert!(paragraph.runs.iter().any(|run| run.field.is_some()));
        }
    }

    #[test]
    fn insert_table_of_contents_prefixes_pages_with_the_latest_sequence_value() {
        let sequence_heading = |number: &str, title: &str, level: u8| {
            let mut sequence_run = run("");
            sequence_run.field = Some(RunField {
                word_mac_body_result: None,
                kind: RunFieldKind::Seq {
                    identifier: "Chapter".to_string(),
                    restart_at: None,
                    repeat_previous: false,
                    hide_result: false,
                    restart_after_heading: None,
                    bookmark: None,
                    number_format: None,
                    case_format: None,
                },
                cached_text: Some(number.to_string()),
                source_field_id: None,
                resolved_value: Some(number.to_string()),
            });
            let mut element = heading("", level);
            element.as_paragraph_mut().expect("heading paragraph").runs =
                vec![sequence_run, run(title)];
            element
        };
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("Title", "Normal"),
                    sequence_heading("1", " Introduction", 1),
                    heading("First section", 2),
                    sequence_heading("2", " Next chapter", 1),
                ],
            }),
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);

        doc.insert_table_of_contents(
            0,
            &TocOptions {
                outline_min: 1,
                outline_max: 2,
                hyperlinks: true,
                sequence_page_number: Some(TocSequencePageNumber {
                    identifier: "Chapter".to_string(),
                    separator: Some(":".to_string()),
                }),
                use_applied_outline_levels: true,
                ..TocOptions::default()
            },
        )
        .expect("insert sequence-prefixed TOC");

        let body = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .expect("document body");
        let prefixes = body.elements[1..=3]
            .iter()
            .map(|element| {
                let paragraph = element.as_paragraph().expect("TOC entry paragraph");
                // The prefix must not be cached inside PAGEREF. Keeping the two
                // runs separate lets repagination replace only the page number.
                assert!(paragraph.runs.iter().any(|run| {
                    matches!(
                        run.field.as_ref().map(|field| &field.kind),
                        Some(RunFieldKind::PageRef { .. })
                    ) && run.field.as_ref().unwrap().cached_text.as_deref() == Some("1")
                }));
                paragraph
                    .runs
                    .iter()
                    .find(|run| run.text.ends_with(':'))
                    .map(|run| run.text.as_str())
                    .expect("plain sequence prefix")
            })
            .collect::<Vec<_>>();
        assert_eq!(prefixes, vec!["1:", "1:", "2:"]);
    }

    #[test]
    fn update_table_of_contents_round_trips_sequence_switches_and_prefix_runs() {
        let data = std::fs::read(fixture_path("toc-sequence-page-number.docx"))
            .expect("read sequence-prefixed TOC fixture");
        let mut doc = executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load sequence-prefixed TOC fixture");

        doc.update_table_of_contents()
            .expect("regenerate sequence-prefixed TOC");

        let exported = doc.export_docx().expect("export regenerated TOC");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("exported DOCX");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("exported document.xml")
            .read_to_string(&mut document_xml)
            .expect("read exported document.xml");

        // Preserve the switches themselves for another Word update. Prefixes
        // are separate text runs immediately before PAGEREF, never baked into
        // that field's cached page result.
        assert!(document_xml.contains(r#"\s "Chapter" \d ":""#));
        assert!(document_xml.contains(r#"<w:t xml:space="preserve">1:</w:t>"#));
        assert!(document_xml.contains(r#"<w:t xml:space="preserve">2:</w:t>"#));
        assert!(document_xml.contains("PAGEREF _Toc"));
    }

    #[test]
    fn insert_table_of_contents_honors_instr_text_whitespace_switches() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("Title", "Normal"),
                    heading("Part\tOne\nOverview", 1),
                ],
            }),
            ..Document::default()
        };

        let generate = |preserve_tabs, preserve_line_breaks| {
            let mut doc = WordDocument::from_document(document.clone());
            doc.insert_table_of_contents(
                0,
                &TocOptions {
                    outline_min: 1,
                    outline_max: 3,
                    hyperlinks: true,
                    hide_page_numbers: false,
                    hide_page_number_levels: None,
                    custom_style_levels: Vec::new(),
                    page_number_separator: None,
                    preserve_tabs,
                    preserve_line_breaks,
                    bookmark_name: None,
                    caption_label: None,
                    omit_caption_label_and_number: false,
                    use_toc_entry_fields: false,
                    toc_entry_identifier: None,
                    toc_entry_levels: None,
                    sequence_page_number: None,
                    use_applied_outline_levels: true,
                },
            )
            .expect("insert TOC with whitespace switches");
            doc.document
                .as_ref()
                .and_then(|document| document.body.as_ref())
                .and_then(|body| body.elements.get(1))
                .and_then(BodyElement::as_paragraph)
                .map(|paragraph| paragraph.runs[0].text.clone())
                .expect("generated TOC entry")
        };

        // Without either switch Word flattens heading tabs and manual breaks
        // to spaces. Each switch independently opts its character back in.
        assert_eq!(generate(false, false), "Part One Overview");
        assert_eq!(generate(true, false), "Part\tOne Overview");
        assert_eq!(generate(false, true), "Part One\nOverview");
        assert_eq!(generate(true, true), "Part\tOne\nOverview");
    }

    #[test]
    fn insert_table_of_contents_builds_table_of_figures_from_caption_sequence() {
        let mut seq_run = run("");
        seq_run.field = Some(RunField {
            word_mac_body_result: None,
            kind: RunFieldKind::Seq {
                identifier: "Figure".to_string(),
                restart_at: None,
                repeat_previous: false,
                hide_result: false,
                restart_after_heading: None,
                bookmark: None,
                number_format: None,
                case_format: None,
            },
            cached_text: Some("8".to_string()),
            source_field_id: None,
            resolved_value: Some("1".to_string()),
        });
        let caption = Paragraph {
            runs: vec![run("Figure "), seq_run, run(": Architecture")],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let document = Document {
            body: Some(Body {
                elements: vec![
                    styled_paragraph("List of figures", "Normal"),
                    BodyElement::from_paragraph(caption),
                    heading("Ordinary heading", 1),
                ],
            }),
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);
        doc.insert_table_of_contents(
            0,
            &TocOptions {
                caption_label: Some("Figure".to_string()),
                hyperlinks: true,
                ..TocOptions::default()
            },
        )
        .expect("insert table of figures");

        let entry = doc
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .expect("generated table-of-figures entry");
        assert_eq!(entry.runs[0].text, "Figure 1: Architecture");
        assert_eq!(
            entry.properties.as_ref().unwrap().style.as_deref(),
            Some("TableofFigures")
        );
    }

    #[test]
    fn insert_table_of_contents_without_headings_inserts_placeholder() {
        // Word inserts a TOC field with the placeholder result
        // "目次項目が見つかりません。" rather than failing when there are no
        // headings to collect, so the right-click "目次の挿入" always produces a
        // visible result instead of silently doing nothing.
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![run("just body text")],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        };
        let mut doc = WordDocument::from_document(document);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(0, TocOptions::default())),
                    &mut ctx,
                )
                .expect("insert toc placeholder");
        }
        // The original paragraph plus one placeholder paragraph.
        assert_eq!(paragraph_count(&doc), 2);
        let placeholder_text: String = doc
            .document
            .as_ref()
            .and_then(|d| d.body.as_ref())
            .and_then(|b| b.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(placeholder_text, "目次項目が見つかりません。");

        // Undo removes the placeholder, restoring the single original paragraph.
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo");
        }
        assert_eq!(paragraph_count(&doc), 1);
    }

    #[test]
    fn update_table_of_contents_regenerates_existing_toc() {
        // wrong-num.docx uses the built-in heading styles "1"/"2"/"3" (which
        // define w:outlineLvl), so the load-time style fill resolves applied
        // outline levels and the outline generator collects them. Loading from a
        // real docx gives us a populated raw XML tree, which the update path
        // scans to locate the TOC field.
        let data = std::fs::read(fixture_path("wrong-num.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let mut executor = CommandExecutor::new();

        let count = |doc: &WordDocument| {
            doc.document
                .as_ref()
                .and_then(|d| d.body.as_ref())
                .map(|b| b.elements.len())
                .unwrap_or(0)
        };
        let before_insert = count(&word_doc);

        // Insert a TOC at the top.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableOfContentsCommand::new(
                        0,
                        TocOptions {
                            outline_min: 1,
                            outline_max: 3,
                            hyperlinks: true,
                            hide_page_numbers: false,
                            hide_page_number_levels: None,
                            custom_style_levels: Vec::new(),
                            page_number_separator: None,
                            preserve_tabs: false,
                            preserve_line_breaks: false,
                            bookmark_name: None,
                            caption_label: None,
                            omit_caption_label_and_number: false,
                            use_toc_entry_fields: false,
                            toc_entry_identifier: None,
                            toc_entry_levels: None,
                            sequence_page_number: None,
                            use_applied_outline_levels: true,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert toc");
        }
        let after_insert = count(&word_doc);
        let entry_count = after_insert - before_insert;
        assert!(entry_count > 0, "wrong-num.docx should have headings");

        // Update: headings are unchanged, so the regenerated TOC has the same
        // number of entry paragraphs (old removed, fresh inserted in place).
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(UpdateTableOfContentsCommand::new()), &mut ctx)
                .expect("update toc");
        }
        assert_eq!(
            count(&word_doc),
            after_insert,
            "regenerated TOC keeps the same paragraph count"
        );

        // Undo the update, then the insert, restoring the original document.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo update");
        }
        assert_eq!(count(&word_doc), after_insert);
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo insert");
        }
        assert_eq!(count(&word_doc), before_insert);
    }

    #[test]
    fn update_table_of_contents_without_toc_errors() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        let result = executor.execute(Box::new(UpdateTableOfContentsCommand::new()), &mut ctx);
        assert!(result.is_err(), "no TOC field should error");
    }
}
