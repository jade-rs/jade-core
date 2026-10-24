//! Strikethrough formatting command implementation.
//!
//! Applies inline strikethrough styling to the selected text range.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::mutate_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Paragraph;
use jade_docx_layout::selection::SelectionRange;

/// Applies strikethrough styling to the current selection.
pub struct ApplyStrikeCommand {
    range: SelectionRange,
    double: bool,
    enabled: bool,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl ApplyStrikeCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self::new_with_enabled(range, true)
    }

    pub fn new_with_enabled(range: SelectionRange, enabled: bool) -> Self {
        Self {
            range,
            double: false,
            enabled,
            previous_runs: None,
            previous_selection: None,
        }
    }

    pub fn new_double(range: SelectionRange) -> Self {
        Self {
            range,
            double: true,
            enabled: true,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for ApplyStrikeCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "apply_strike"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
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

            let restyled = mutate_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &zero_width,
                |properties| {
                    properties.strike = Some(self.enabled && !self.double);
                    properties.double_strike = Some(self.enabled && self.double);
                },
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

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn sample_document() -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "Strike this text".to_string(),
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
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn applies_strike_to_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 17),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyStrikeCommand::new(range)), &mut ctx)
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

        assert!(
            paragraph
                .runs
                .iter()
                .any(|run| run.properties.as_ref().and_then(|p| p.strike) == Some(true)),
            "expected strike property to be applied"
        );
    }

    #[test]
    fn disables_single_and_double_strikethrough() {
        let mut document = sample_document();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyStrikeCommand::new_double(range)), &mut ctx)
                .unwrap();
        }
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(ApplyStrikeCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }

        let props = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(props.strike, Some(false));
        assert_eq!(props.double_strike, Some(false));
    }

    #[test]
    fn applies_double_strike_to_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 16),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyStrikeCommand::new_double(range)), &mut ctx)
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
        let properties = paragraph.runs[0].properties.as_ref().unwrap();
        assert_eq!(properties.strike, Some(false));
        assert_eq!(properties.double_strike, Some(true));
    }

    #[test]
    fn rejects_collapsed_selection() {
        let mut document = sample_document();
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(collapsed);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let result = executor.execute(Box::new(ApplyStrikeCommand::new(collapsed)), &mut ctx);
        assert!(
            matches!(result, Err(CommandError::InvalidSelection)),
            "collapsed selections should be rejected"
        );
    }

    #[test]
    fn undo_and_redo_restore_strike_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 17),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyStrikeCommand::new(range)), &mut ctx)
                .unwrap();
        }
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
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
        assert!(
            paragraph
                .runs
                .iter()
                .all(|run| run.properties.as_ref().and_then(|p| p.strike) != Some(true)),
            "undo should remove strike style"
        );

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.redo(&mut ctx).unwrap());
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
        assert!(
            paragraph
                .runs
                .iter()
                .any(|run| run.properties.as_ref().and_then(|p| p.strike) == Some(true)),
            "redo should re-apply strike style"
        );
        assert_eq!(selection.unwrap(), range);
    }

    #[test]
    fn strike_command_updates_exported_document_xml() {
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
                .execute(Box::new(ApplyStrikeCommand::new(range)), &mut ctx)
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
            document_xml.contains("<w:strike"),
            "expected exported XML to contain strike run properties: {document_xml}"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyStrikeCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }
        let exported = word_doc.export_docx().expect("export disabled strike");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        assert!(
            document_xml.contains(r#"<w:strike w:val="0""#),
            "disabling strike must override Word character styles: {document_xml}"
        );
    }

    #[test]
    fn double_strike_updates_exported_document_xml() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(ApplyStrikeCommand::new_double(range)), &mut ctx)
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");

        assert!(
            document_xml.contains("<w:dstrike"),
            "expected Word's double-strike element in exported XML: {document_xml}"
        );
    }

    fn exported_document_xml(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        document_xml
    }

    /// The `w:r` that hosts the fixture's text box, from its opening tag to
    /// the end of its `mc:AlternateContent`.
    fn text_box_run(document_xml: &str) -> &str {
        let start = document_xml
            .find("<w:r><mc:AlternateContent>")
            .unwrap_or_else(|| panic!("text box run missing: {document_xml}"));
        let end = start
            + document_xml[start..]
                .find("</mc:AlternateContent></w:r>")
                .expect("text box run end")
            + "</mc:AlternateContent></w:r>".len();
        &document_xml[start..end]
    }

    /// The `w:r` whose text is `text`.
    fn text_run<'a>(document_xml: &'a str, text: &str) -> &'a str {
        let text_tag = format!("<w:t>{text}</w:t>");
        let text_at = document_xml
            .find(&text_tag)
            .unwrap_or_else(|| panic!("run {text:?} missing: {document_xml}"));
        let start = document_xml[..text_at].rfind("<w:r>").expect("run start");
        &document_xml[start..text_at + text_tag.len()]
    }

    /// Formatting part of a paragraph used to clear the XML ids of every run
    /// after the range, so the writer regenerated the untouched text box run
    /// from the model: the `mc:AlternateContent` wrapper, the VML fallback,
    /// the `wp:docPr` id and name and the effect/miter markup were lost.
    #[test]
    fn strike_beside_text_box_keeps_its_source_xml() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::HeadlessEditRequest;

        let data = std::fs::read(fixture_path("textbox.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let before = exported_document_xml(&word_doc);

        let request: HeadlessEditRequest = serde_json::from_str(
            r#"{"commands":[{"type":"apply_strike","start_paragraph":0,"start_offset":0,"end_paragraph":0,"end_offset":2}]}"#,
        )
        .expect("parse request");
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply strike");
        let after = exported_document_xml(&word_doc);

        let text_box = text_box_run(&after);
        assert_eq!(
            text_box,
            text_box_run(&before),
            "the untouched text box run must keep its source XML"
        );
        assert_eq!(text_box.matches("<w:txbxContent>").count(), 2, "{text_box}");
        assert!(text_box.contains("<mc:Fallback><w:pict>"), "{text_box}");
        for preserved in [
            r#"<wp:docPr id="1073741825" name="officeArt object" descr="Hello"/>"#,
            r#"<wp:effectExtent l="0" t="0" r="0" b="0"/>"#,
            "<wp:cNvGraphicFramePr/>",
            r#"<a:miter lim="400000"/>"#,
            "<a:effectLst/>",
        ] {
            assert!(text_box.contains(preserved), "lost {preserved}: {text_box}");
        }
        assert_eq!(
            text_box
                .matches(r#"<w:pPr><w:pStyle w:val="本文"/><w:bidi w:val="0"/></w:pPr>"#)
                .count(),
            2,
            "both copies of the box paragraph keep their pPr: {text_box}"
        );

        assert!(
            text_run(&after, "He").contains("<w:strike/>"),
            "the formatted run carries the strike: {after}"
        );
        assert!(
            !text_run(&after, "llo").contains("<w:strike"),
            "the rest of the run stays unformatted: {after}"
        );
        assert!(!text_box.contains("<w:strike"), "{text_box}");
    }
}
