//! Underline formatting command implementation.
//!
//! Applies inline underline styling to selections so that text can be visually
//! emphasised without altering surrounding runs.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::mutate_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Paragraph;
use jade_docx_layout::selection::SelectionRange;

/// Applies underline styling to the current selection.
pub struct ApplyUnderlineCommand {
    range: SelectionRange,
    kind: Option<String>,
    enabled: bool,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl ApplyUnderlineCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self::new_with_enabled(range, true)
    }

    pub fn new_with_enabled(range: SelectionRange, enabled: bool) -> Self {
        Self {
            range,
            kind: None,
            enabled,
            previous_runs: None,
            previous_selection: None,
        }
    }

    pub fn new_with_kind(range: SelectionRange, kind: impl Into<String>) -> Self {
        Self {
            range,
            kind: Some(kind.into()),
            enabled: true,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for ApplyUnderlineCommand {
    fn name(&self) -> &'static str {
        "apply_underline"
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
                    properties.underline = Some(self.enabled);
                    properties.underline_kind = self
                        .enabled
                        .then(|| self.kind.as_deref().map(Into::into))
                        .flatten();
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
                text: "Underline me".to_string(),
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
    fn applies_underline_to_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 12),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyUnderlineCommand::new(range)), &mut ctx)
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
                .any(|run| run.properties.as_ref().and_then(|p| p.underline) == Some(true)),
            "expected underline property to be applied"
        );
    }

    #[test]
    fn disables_underline_and_clears_its_word_style() {
        let mut document = sample_document();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_kind(range, "words")),
                    &mut ctx,
                )
                .unwrap();
        }
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }

        let props = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(props.underline, Some(false));
        assert_eq!(props.underline_kind, None);
    }

    #[test]
    fn applies_word_only_underline_kind_to_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 12),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_kind(range, "words")),
                    &mut ctx,
                )
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
        assert_eq!(properties.underline, Some(true));
        assert_eq!(properties.underline_kind.as_deref(), Some("words"));
    }

    #[test]
    fn rejects_collapsed_selection() {
        let mut document = sample_document();
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(collapsed);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let result = executor.execute(Box::new(ApplyUnderlineCommand::new(collapsed)), &mut ctx);
        assert!(
            matches!(result, Err(CommandError::InvalidSelection)),
            "collapsed selections should be rejected"
        );
    }

    #[test]
    fn undo_and_redo_restore_underline_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 12),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyUnderlineCommand::new(range)), &mut ctx)
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
                .all(|run| run.properties.as_ref().and_then(|p| p.underline) != Some(true)),
            "undo should remove underline style"
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
                .any(|run| run.properties.as_ref().and_then(|p| p.underline) == Some(true)),
            "redo should re-apply underline style"
        );
        assert_eq!(selection.unwrap(), range);
    }

    #[test]
    fn underline_command_updates_exported_document_xml() {
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
                .execute(Box::new(ApplyUnderlineCommand::new(range)), &mut ctx)
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
            document_xml.contains("<w:u"),
            "expected exported XML to contain underline run properties: {document_xml}"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }
        let exported = word_doc.export_docx().expect("export disabled underline");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported.clone())).expect("zip archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        assert!(
            document_xml.contains(r#"<w:u w:val="none""#),
            "disabling underline must override Word character styles: {document_xml}"
        );
        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen disabled underline");
        assert!(reopened
            .paragraph(0)
            .expect("first paragraph")
            .runs
            .iter()
            .all(|run| {
                run.properties.as_ref().and_then(|props| props.underline) != Some(true)
            }));
    }

    #[test]
    fn word_only_underline_kind_updates_exported_document_xml() {
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
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_kind(range, "words")),
                    &mut ctx,
                )
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
            document_xml.contains("<w:u w:val=\"words\""),
            "expected Word's words underline token in exported XML: {document_xml}"
        );
    }
}
