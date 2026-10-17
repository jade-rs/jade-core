//! Bold formatting command implementation.
//!
//! The command applies inline bold styling to the current logical selection
//! while keeping the XML representation in sync so that exported DOCX files
//! reflect the styling changes.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::apply_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, RunProperties};
use jade_docx_layout::selection::SelectionRange;

/// Applies bold styling to the current selection.
///
/// Multi-paragraph selections are supported by splitting runs at the selection
/// boundaries and updating only the affected spans.
pub struct ApplyBoldCommand {
    range: SelectionRange,
    enabled: bool,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl ApplyBoldCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self::new_with_enabled(range, true)
    }

    pub fn new_with_enabled(range: SelectionRange, enabled: bool) -> Self {
        Self {
            range,
            enabled,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for ApplyBoldCommand {
    fn name(&self) -> &'static str {
        "apply_bold"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        // Collapsed selections represent caret positions; bold styling is
        // handled elsewhere by recording inline formatting state instead.
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }

        let (start, end) = self.range.normalized();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let bold_props = RunProperties {
            bold: Some(self.enabled),
            bold_complex: Some(self.enabled),
            ..Default::default()
        };

        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);
            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };

            if range_start >= range_end {
                continue;
            }

            let restyled = apply_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &bold_props,
                &zero_width,
            );
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
        }

        *ctx.selection = Some(self.range);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn sample_document() -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![
                Run {
                    text: "Hello ".to_string(),
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
                },
                Run {
                    text: "World".to_string(),
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
                },
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    fn sample_two_paragraph_document() -> WordDocument {
        let first = Paragraph {
            runs: vec![Run {
                text: "Hello".to_string(),
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
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let second = Paragraph {
            runs: vec![Run {
                text: "World".to_string(),
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
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(first),
                    BodyElement::from_paragraph(second),
                ],
            }),
            ..Document::default()
        })
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    /// Builds a minimal DOCX package around the given `<w:body>` content. The
    /// shipped fixtures only contain empty self-closing `w:fldSimple`
    /// elements, so field-container regressions need a synthetic package.
    fn build_docx_with_body(body_xml: &str) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
"#;
        const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"#;
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", RELS),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    /// Paragraph with a plain run, then a `w:fldSimple` (MERGEFIELD result run
    /// "John Smith"), then a plain sibling run — the shape whose trailing plain
    /// run must never be absorbed into the field container on writeback.
    /// Model text: "Before " (0..7) + "John Smith" (7..17) + " follows the field.".
    fn fld_simple_body() -> &'static str {
        r#"<w:p><w:r><w:t xml:space="preserve">Before </w:t></w:r><w:fldSimple w:instr=" MERGEFIELD Name "><w:r><w:t>John Smith</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> follows the field.</w:t></w:r></w:p>"#
    }

    fn exported_document_xml(word_doc: &mut WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut xml)
            .expect("read document.xml");
        xml
    }

    /// Field-container integrity: a plain text replacement *inside* a
    /// `w:fldSimple` result run edits the run text in place (no split), so the
    /// following plain run must keep its identity and stay outside the field
    /// container — pin for the in-place `apply_text_edit` path.
    #[test]
    fn editing_inside_fld_simple_does_not_absorb_following_run() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = build_docx_with_body(fld_simple_body());
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Replace "Smi" (12..15) inside the field result "John Smith".
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 12,
                end_paragraph: 0,
                end_offset: 15,
                text: "Z".to_string(),
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let xml = exported_document_xml(&mut word_doc);
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple element body");
        assert!(
            !field_body.contains("follows the field."),
            "following plain run must NOT be inside the fldSimple: {field_body}"
        );
        assert!(
            xml.contains("follows the field."),
            "following plain run text must survive outside the field: {xml}"
        );
        // The edited field result itself stays inside the container.
        assert!(
            field_body.contains("John Z") && field_body.contains("th"),
            "the edited field result must remain inside the fldSimple: {field_body}"
        );
    }

    /// Field-container integrity: formatting a sub-range inside a
    /// `w:fldSimple` result run splits that run, and the split-off fragments
    /// carry no `node_id`. The container-absorption loop must not also suck
    /// the following plain run into the field container — same shape as the
    /// hyperlink bug guarded by
    /// `editing_inside_hyperlink_does_not_absorb_following_run`.
    #[test]
    fn formatting_inside_fld_simple_does_not_absorb_following_run() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = build_docx_with_body(fld_simple_body());
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Bold "ohn " (8..12), strictly inside the field result "John Smith",
        // so the container is still visited via the surviving head fragment.
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ApplyBold {
                start_paragraph: 0,
                start_offset: 8,
                end_paragraph: 0,
                end_offset: 12,
                enabled: true,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let xml = exported_document_xml(&mut word_doc);
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple element body");
        assert!(
            !field_body.contains("follows the field."),
            "following plain run must NOT be inside the fldSimple: {field_body}"
        );
        assert!(
            xml.contains("follows the field."),
            "following plain run text must survive outside the field: {xml}"
        );
    }

    /// Field-container integrity: bolding a sub-range inside the `w:fldSimple`
    /// result run splits that run, but every fragment must stay inside the
    /// single field container and `w:instr` must survive — mirror of
    /// `bolding_inside_hyperlink_keeps_all_fragments_wrapped`.
    #[test]
    fn bolding_inside_fld_simple_keeps_all_fragments_wrapped() {
        let data = build_docx_with_body(fld_simple_body());
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Bold "ohn " (8..12) to force a split inside the field result run.
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 8),
            TextPosition::new(0, 12),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let xml = exported_document_xml(&mut word_doc);
        // Exactly one fldSimple element (open + close tags == 2 substring hits).
        assert_eq!(
            xml.matches("w:fldSimple").count(),
            2,
            "fldSimple must not be duplicated or dropped: {xml}"
        );
        assert!(
            xml.contains("MERGEFIELD Name"),
            "field instruction must survive: {xml}"
        );
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple element body");
        assert!(
            field_body.contains("<w:b/>"),
            "the bolded fragment must be inside the fldSimple: {field_body}"
        );
        // The split produced "J" + "ohn " (bold) + "Smith"; all stay inside.
        assert!(
            field_body.contains(">J<")
                && field_body.contains("ohn ")
                && field_body.contains("Smith"),
            "all split fragments must stay inside the fldSimple: {field_body}"
        );
        assert!(
            !field_body.contains("follows the field."),
            "following plain run must NOT be inside the fldSimple: {field_body}"
        );
    }

    /// Paragraph that STARTS with the `w:fldSimple` container (no plain run
    /// before it). Splits starting at paragraph offset 0 null the `node_id` of
    /// every run in the paragraph, which used to make the writeback discard the
    /// whole container. Model text: "John Smith" (0..10) + " follows the field.".
    fn fld_simple_leading_body() -> &'static str {
        r#"<w:p><w:fldSimple w:instr=" MERGEFIELD Name "><w:r><w:t>John Smith</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> follows the field.</w:t></w:r></w:p>"#
    }

    /// Paragraph that STARTS with a `w:hyperlink` container (anchor-based, so
    /// no relationship part is needed). Model text: "Example link" (0..12) +
    /// " after link.".
    fn hyperlink_leading_body() -> &'static str {
        r#"<w:p><w:hyperlink w:anchor="section1" w:history="1"><w:r><w:t>Example link</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> after link.</w:t></w:r></w:p>"#
    }

    /// Field-container integrity at paragraph offset 0: bolding a range that
    /// began at the very start of the paragraph used to null `node_id` on ALL
    /// runs (`split_runs_at` clears the whole tail), so the writeback treated
    /// the `w:fldSimple` container as if its runs were deleted and drop it —
    /// `w:instr` included — re-creating the field result as plain runs.
    #[test]
    fn bolding_from_paragraph_start_keeps_fld_simple_container() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = build_docx_with_body(fld_simple_leading_body());
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Bold "John" (0..4): the range starts at paragraph offset 0 and ends
        // inside the field result run "John Smith".
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ApplyBold {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 4,
                enabled: true,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let xml = exported_document_xml(&mut word_doc);
        // Exactly one fldSimple element (open + close tags == 2 substring hits).
        assert_eq!(
            xml.matches("w:fldSimple").count(),
            2,
            "fldSimple must not be duplicated or dropped: {xml}"
        );
        assert!(
            xml.contains("MERGEFIELD Name"),
            "field instruction must survive: {xml}"
        );
        let field_body = xml
            .split("<w:fldSimple")
            .nth(1)
            .and_then(|tail| tail.split("</w:fldSimple>").next())
            .expect("fldSimple element body");
        assert!(
            field_body.contains("<w:b/>"),
            "the bolded fragment must be inside the fldSimple: {field_body}"
        );
        // The split produced "John" (bold) + " Smith"; both stay inside.
        assert!(
            field_body.contains("John") && field_body.contains("Smith"),
            "all split fragments must stay inside the fldSimple: {field_body}"
        );
        assert!(
            !field_body.contains("follows the field."),
            "following plain run must NOT be inside the fldSimple: {field_body}"
        );
        assert!(
            xml.contains("follows the field."),
            "following plain run text must survive outside the field: {xml}"
        );
    }

    /// Hyperlink-container integrity at paragraph offset 0: same shape as
    /// `bolding_from_paragraph_start_keeps_fld_simple_container` but for a
    /// `w:hyperlink` that starts the paragraph — the wrapper and its anchor
    /// attribute must survive a bold range beginning at offset 0.
    #[test]
    fn bolding_from_paragraph_start_keeps_hyperlink_container() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = build_docx_with_body(hyperlink_leading_body());
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Bold "Exam" (0..4): the range starts at paragraph offset 0 and ends
        // inside the hyperlink text "Example link".
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ApplyBold {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 4,
                enabled: true,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let xml = exported_document_xml(&mut word_doc);
        // Exactly one hyperlink element (open + close tags == 2 substring hits).
        assert_eq!(
            xml.matches("w:hyperlink").count(),
            2,
            "hyperlink must not be duplicated or dropped: {xml}"
        );
        assert!(
            xml.contains("w:anchor=\"section1\""),
            "hyperlink anchor must survive: {xml}"
        );
        let hyperlink_body = xml
            .split("<w:hyperlink")
            .nth(1)
            .and_then(|tail| tail.split("</w:hyperlink>").next())
            .expect("hyperlink element body");
        assert!(
            hyperlink_body.contains("<w:b/>"),
            "the bolded fragment must be inside the hyperlink: {hyperlink_body}"
        );
        assert!(
            hyperlink_body.contains("Exam") && hyperlink_body.contains("ple link"),
            "all split fragments must stay inside the hyperlink: {hyperlink_body}"
        );
        assert!(
            !hyperlink_body.contains("after link."),
            "following plain run must NOT be inside the hyperlink: {hyperlink_body}"
        );
        assert!(
            xml.contains("after link."),
            "following plain run text must survive outside the hyperlink: {xml}"
        );
    }

    #[test]
    fn applies_bold_to_selection_within_single_paragraph() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 3),
            TextPosition::new(0, 8),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let paragraph = document
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .unwrap();

        let bold_runs: Vec<_> = paragraph
            .runs
            .iter()
            .filter(|run| run.properties.as_ref().and_then(|p| p.bold) == Some(true))
            .collect();
        assert!(
            !bold_runs.is_empty(),
            "expected at least one run to be bold after applying command"
        );
        assert_eq!(
            selection.unwrap(),
            SelectionRange::new(TextPosition::new(0, 3), TextPosition::new(0, 8),)
        );
    }

    #[test]
    fn disables_bold_for_selected_text() {
        let mut document = sample_document();
        document.paragraph_mut(0).unwrap().runs[0].properties = Some(RunProperties {
            bold: Some(true),
            bold_complex: Some(true),
            ..Default::default()
        });
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(ApplyBoldCommand::new_with_enabled(range, false)),
                &mut ctx,
            )
            .unwrap();

        let properties = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(properties.bold, Some(false));
        assert_eq!(properties.bold_complex, Some(false));
    }

    #[test]
    fn apply_rejects_collapsed_selection() {
        let mut document = sample_document();
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(collapsed);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let result = executor.execute(Box::new(ApplyBoldCommand::new(collapsed)), &mut ctx);
        assert!(
            matches!(result, Err(CommandError::InvalidSelection)),
            "collapsed selections should be rejected"
        );
    }

    #[test]
    fn applies_bold_to_sub_range_without_affecting_prefix() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 3),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let paragraph = document
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .unwrap();

        assert_eq!(
            paragraph.runs.len(),
            4,
            "applying bold should split the run into styled and unstyled segments"
        );
        assert_eq!(
            paragraph.runs[0].text, "Hel",
            "text before the selection should remain untouched"
        );
        assert!(
            paragraph.runs[0].properties.is_none(),
            "text preceding the selection should not become bold"
        );
        assert_eq!(
            paragraph.runs[1].text, "lo",
            "selected text should be isolated into its own run"
        );
        assert_eq!(
            paragraph.runs[1].properties.as_ref().and_then(|p| p.bold),
            Some(true),
            "selected span should receive the bold property"
        );
        assert_eq!(
            paragraph.runs[2].text, " ",
            "content immediately after the selection should remain intact"
        );
        assert!(
            paragraph.runs[2].properties.is_none(),
            "text after the selection should not inherit bold styling"
        );
        assert_eq!(
            paragraph.runs[3].text, "World",
            "runs outside the selection should remain unchanged"
        );
        assert!(
            paragraph.runs[3].properties.is_none(),
            "runs outside the selection should remain unstyled"
        );
    }

    #[test]
    fn applies_bold_across_multiple_paragraphs() {
        let mut document = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(1, 5),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let body = document.document.as_ref().unwrap().body.as_ref().unwrap();

        let first_paragraph = body
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .unwrap();
        let second_paragraph = body
            .elements
            .get(1)
            .and_then(|element| element.as_paragraph())
            .unwrap();

        assert!(
            first_paragraph
                .runs
                .iter()
                .all(|run| run.properties.as_ref().and_then(|p| p.bold) == Some(true)),
            "expected first paragraph runs to be bold",
        );
        assert!(
            second_paragraph
                .runs
                .iter()
                .all(|run| run.properties.as_ref().and_then(|p| p.bold) == Some(true)),
            "expected second paragraph runs to be bold",
        );
    }

    #[test]
    fn undo_and_redo_restore_bold_across_multiple_paragraphs() {
        let mut document = sample_two_paragraph_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(1, 5),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
        }
        {
            let body = document.document.as_ref().unwrap().body.as_ref().unwrap();
            for paragraph in body
                .elements
                .iter()
                .filter_map(|element| element.as_paragraph())
            {
                assert!(
                    paragraph
                        .runs
                        .iter()
                        .all(|run| run.properties.as_ref().and_then(|p| p.bold) != Some(true)),
                    "undo should remove bold property from all runs",
                );
            }
        }

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.redo(&mut ctx).unwrap());
        }

        let body = document.document.as_ref().unwrap().body.as_ref().unwrap();
        for paragraph in body
            .elements
            .iter()
            .filter_map(|element| element.as_paragraph())
        {
            assert!(
                paragraph
                    .runs
                    .iter()
                    .all(|run| run.properties.as_ref().and_then(|p| p.bold) == Some(true)),
                "redo should re-apply bold property to all runs",
            );
        }
        assert_eq!(selection.unwrap(), range);
    }

    #[test]
    fn bold_command_updates_exported_document_xml() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");

        assert!(
            document_xml.contains("<w:b"),
            "expected exported XML to contain bold run properties: {document_xml}"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyBoldCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }
        let exported = word_doc.export_docx().expect("export disabled bold");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported.clone())).expect("zip archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        assert!(
            document_xml.contains(r#"<w:b w:val="0""#),
            "disabling bold must override Word character styles: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"<w:bCs w:val="0""#),
            "disabling bold must also override complex-script styles: {document_xml}"
        );
        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen disabled bold");
        assert!(reopened
            .paragraph(0)
            .expect("first paragraph")
            .runs
            .iter()
            .all(|run| {
                let properties = run.properties.as_ref();
                properties.and_then(|props| props.bold) != Some(true)
                    && properties.and_then(|props| props.bold_complex) != Some(true)
            }));
    }

    /// P2 regression: splitting a run that carries an unmodeled `w:rPr` child
    /// (here `w:lang`) must not drop that child from the split-off fragment.
    /// `align-center.docx` has a single run "Hello World" whose rPr contains
    /// `<w:lang w:val="en-US"/>`. Bolding the "World" sub-range splits the run
    /// into "Hello " + "World"; both fragments must keep `w:lang` (preserved via
    /// `RunProperties::preserved_rpr`, captured at load).
    #[test]
    fn splitting_run_preserves_unmodeled_rpr_children() {
        let data = std::fs::read(fixture_path("align-center.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        // "Hello World" → bold only "World" (offsets 6..11), forcing a split.
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 6),
            TextPosition::new(0, 11),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");

        let lang_count = document_xml.matches("w:lang").count();
        assert_eq!(
            lang_count, 2,
            "both split fragments must retain w:lang; got {lang_count}: {document_xml}"
        );
    }
    #[test]
    /// P3 integrity: bolding a sub-range *inside* a hyperlink run splits that
    /// run, but every fragment must stay inside the single `<w:hyperlink>`
    /// wrapper and keep its `w:rStyle` (preserved via P2's preserved_rpr). This
    /// guards against split fragments leaking out of the hyperlink or dropping
    /// the relationship reference.
    fn bolding_inside_hyperlink_keeps_all_fragments_wrapped() {
        let data = std::fs::read(fixture_path("hyperlink.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Paragraph 0 = "Visit " (6) + hyperlink "example.com" (6..17) + " to learn more.".
        // Bold "exam" (6..10) to force a split inside the hyperlink run.
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 6),
            TextPosition::new(0, 10),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .unwrap();
        }
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut xml)
            .expect("read document.xml");

        // Exactly one hyperlink element (open + close tags == 2 substring hits).
        assert_eq!(
            xml.matches("w:hyperlink").count(),
            2,
            "hyperlink must not be duplicated or dropped: {xml}"
        );
        // The relationship reference must survive.
        assert!(
            xml.contains("r:id=\"rId7\""),
            "hyperlink relationship reference must survive: {xml}"
        );
        // All split fragments inside the hyperlink keep the Hyperlink run style.
        let hyperlink_body = xml
            .split("<w:hyperlink")
            .nth(1)
            .and_then(|tail| tail.split("</w:hyperlink>").next())
            .expect("hyperlink element body");
        assert_eq!(
            hyperlink_body.matches("w:val=\"Hyperlink\"").count(),
            3,
            "each of the 3 split fragments must keep its w:rStyle: {hyperlink_body}"
        );
        // The split-off bold fragment must be present and still inside the link.
        assert!(
            hyperlink_body.contains("<w:b/>"),
            "the bolded fragment must be inside the hyperlink: {hyperlink_body}"
        );
    }

    /// P3 integrity: replacing text *inside* a hyperlink run must not pull the
    /// following non-hyperlink run into the `<w:hyperlink>` wrapper. `split_runs_at`
    /// nulls `node_id` on every trailing run (even untouched ones), and the
    /// container-absorption loop would otherwise suck the plain run into the link,
    /// turning "to learn more." into clickable link text.
    #[test]
    fn editing_inside_hyperlink_does_not_absorb_following_run() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = std::fs::read(fixture_path("hyperlink.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // Model text is "Visit" + "example.com"(5..16) + "to learn more.\u{c}".
        // Replace "amp" (7..10) inside the link, splitting the hyperlink run.
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 7,
                end_paragraph: 0,
                end_offset: 10,
                text: "Z".to_string(),
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut xml)
            .expect("read document.xml");

        let hyperlink_body = xml
            .split("<w:hyperlink")
            .nth(1)
            .and_then(|tail| tail.split("</w:hyperlink>").next())
            .expect("hyperlink element body");
        assert!(
            !hyperlink_body.contains("to learn more."),
            "following non-link run must NOT be inside the hyperlink: {hyperlink_body}"
        );
        // The edited link text itself stays inside the hyperlink.
        assert!(
            hyperlink_body.contains("exZle.com")
                || (hyperlink_body.contains('Z') && hyperlink_body.contains("le.com")),
            "the edited link text must remain inside the hyperlink: {hyperlink_body}"
        );
    }

    /// P5 regression: splitting a paragraph with `InsertParagraphBreak` must copy
    /// the full `w:pPr` — including unmodeled children like `w:bidi` — onto the
    /// newly created paragraph, not just the modeled subset. `hello.docx`'s
    /// paragraph carries `<w:pStyle .../><w:bidi w:val="0"/>`; after a break both
    /// halves must keep both children.
    #[test]
    fn paragraph_break_copies_unmodeled_ppr_children_to_new_paragraph() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};

        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertParagraphBreak {
                paragraph: 0,
                offset: 2,
                count: 1,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut xml)
            .expect("read document.xml");

        let body = xml
            .split("<w:body>")
            .nth(1)
            .and_then(|tail| tail.split("</w:body>").next())
            .expect("body");
        // The split produced two new paragraphs from the first; every paragraph's
        // pPr keeps both pStyle and the unmodeled w:bidi.
        let first_two: Vec<&str> = body.split("</w:p>").take(2).collect();
        for (idx, para) in first_two.iter().enumerate() {
            assert!(
                para.contains("<w:pStyle"),
                "paragraph {idx} must keep w:pStyle: {para}"
            );
            assert!(
                para.contains("<w:bidi"),
                "paragraph {idx} must keep unmodeled w:bidi: {para}"
            );
        }
    }
}
