//! Font size adjustment command implementation.
//!
//! Applies an explicit point size to the current selection, ensuring both the
//! in-memory document model and the serialized DOCX output stay in sync.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::apply_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, RunProperties};
use jade_docx_layout::selection::SelectionRange;

/// Word's font size box accepts 1-1638pt.
pub const WORD_MAX_FONT_SIZE_PT: f32 = 1638.0;

/// Snaps a requested size to what Word stores: `w:sz` is in half-points, so
/// Word rounds a typed 10.3 to 10.5 and clamps into 1-1638pt. Normalizing at
/// set time keeps the model (and the size box) equal to what the file saves.
pub fn word_font_size_points(font_size: f32) -> f32 {
    ((font_size * 2.0).round() / 2.0).clamp(1.0, WORD_MAX_FONT_SIZE_PT)
}

/// Assigns the provided font size (in points) to the selected runs.
pub struct SetFontSizeCommand {
    range: SelectionRange,
    font_size: f32,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl SetFontSizeCommand {
    pub fn new(range: SelectionRange, font_size: f32) -> Self {
        Self {
            range,
            font_size,
            previous_runs: None,
            previous_selection: None,
        }
    }

    fn validate_font_size(font_size: f32) -> Result<f32, CommandError> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(CommandError::Custom(
                "font size must be a positive finite value".into(),
            ));
        }
        Ok(word_font_size_points(font_size))
    }
}

impl Command for SetFontSizeCommand {
    fn name(&self) -> &'static str {
        "set_font_size"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let font_size = Self::validate_font_size(self.font_size)?;

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

        let font_props = RunProperties {
            font_size: Some(font_size),
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
                &font_props,
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

    #[test]
    fn applies_font_size_to_selection_within_single_paragraph() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(SetFontSizeCommand::new(range, 18.0)), &mut ctx)
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
                .any(|run| run.properties.as_ref().and_then(|p| p.font_size) == Some(18.0)),
            "expected at least one run to have font size 18pt after applying command"
        );
        assert_eq!(
            selection.unwrap(),
            SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5),)
        );
    }

    #[test]
    fn apply_rejects_collapsed_selection() {
        let mut document = sample_document();
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(collapsed);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let result = executor.execute(Box::new(SetFontSizeCommand::new(collapsed, 14.0)), &mut ctx);
        assert!(
            matches!(result, Err(CommandError::InvalidSelection)),
            "collapsed selections should be rejected"
        );
    }

    #[test]
    fn applies_font_size_across_multiple_paragraphs() {
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
                .execute(Box::new(SetFontSizeCommand::new(range, 20.0)), &mut ctx)
                .unwrap();
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
                    .all(|run| run.properties.as_ref().and_then(|p| p.font_size) == Some(20.0)),
                "expected paragraph runs to carry the updated font size"
            );
        }
    }

    #[test]
    fn undo_and_redo_restore_font_size_across_multiple_paragraphs() {
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
                .execute(Box::new(SetFontSizeCommand::new(range, 20.0)), &mut ctx)
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
                    paragraph.runs.iter().all(|run| run
                        .properties
                        .as_ref()
                        .and_then(|p| p.font_size)
                        .is_none()),
                    "undo should clear applied font size",
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
                    .all(|run| run.properties.as_ref().and_then(|p| p.font_size) == Some(20.0)),
                "redo should re-apply font size",
            );
        }
        assert_eq!(selection.unwrap(), range);
    }

    #[test]
    fn font_size_command_updates_exported_document_xml() {
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
                .execute(Box::new(SetFontSizeCommand::new(range, 24.0)), &mut ctx)
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
            document_xml.contains(r#"w:sz w:val="48""#),
            "expected exported XML to contain updated font size: {document_xml}"
        );
    }

    fn export_document_xml(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        document_xml
    }

    fn apply_size_to_hello(size: f32) -> WordDocument {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(SetFontSizeCommand::new(range, size)), &mut ctx)
                .unwrap();
        }
        word_doc
    }

    fn first_run_font_size(word_doc: &WordDocument) -> Option<f32> {
        word_doc
            .document
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .find_map(|element| element.as_paragraph())
            .and_then(|paragraph| paragraph.runs.first())
            .and_then(|run| run.properties.as_ref())
            .and_then(|props| props.font_size)
    }

    /// Word's font size box accepts 1-1638pt, stored as half-points
    /// (`ST_HpsMeasure`), so 1000pt must survive as `w:sz="2000"`.
    #[test]
    fn exports_font_sizes_up_to_words_1638pt_limit() {
        let word_doc = apply_size_to_hello(1000.0);
        let document_xml = export_document_xml(&word_doc);
        assert!(
            document_xml.contains(r#"w:sz w:val="2000""#),
            "1000pt must export as 2000 half-points: {document_xml}"
        );

        let word_doc = apply_size_to_hello(1638.0);
        let document_xml = export_document_xml(&word_doc);
        assert!(
            document_xml.contains(r#"w:sz w:val="3276""#),
            "1638pt must export as 3276 half-points: {document_xml}"
        );
    }

    /// Word snaps a typed size to the nearest half point (10.3 → 10.5) and
    /// clamps it into 1-1638pt; the model must hold what the file will store.
    #[test]
    fn snaps_font_size_to_half_points_like_word() {
        assert_eq!(first_run_font_size(&apply_size_to_hello(10.3)), Some(10.5));
        assert_eq!(first_run_font_size(&apply_size_to_hello(10.2)), Some(10.0));
        assert_eq!(first_run_font_size(&apply_size_to_hello(0.2)), Some(1.0));
        assert_eq!(
            first_run_font_size(&apply_size_to_hello(5000.0)),
            Some(1638.0)
        );
    }
}
