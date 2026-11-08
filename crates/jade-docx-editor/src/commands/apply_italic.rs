//! Italic formatting for selected text and paragraph marks.

use super::set_run_format::SetRunFormatCommand;
use super::{Command, CommandContext, CommandResult};
use jade_docx_headless_api::HeadlessRunFormat;
use jade_docx_layout::selection::SelectionRange;

/// Applies italic styling to the current selection, including complex script.
pub struct ApplyItalicCommand {
    inner: SetRunFormatCommand,
}

impl ApplyItalicCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self::new_with_enabled(range, true)
    }

    pub fn new_with_enabled(range: SelectionRange, enabled: bool) -> Self {
        Self {
            inner: SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    italic: Some(enabled),
                    complex_italic: Some(enabled),
                    ..Default::default()
                },
            ),
        }
    }
}

impl Command for ApplyItalicCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "apply_italic"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.apply(ctx)
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.undo(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandError, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run, RunProperties};
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

    #[test]
    fn applies_italic_to_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyItalicCommand::new(range)), &mut ctx)
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
                .any(|run| run.properties.as_ref().and_then(|p| p.italic) == Some(true)),
            "expected at least one run to be italic after applying command"
        );
    }

    #[test]
    fn disables_italic_for_selected_text() {
        let mut document = sample_document();
        document.paragraph_mut(0).unwrap().runs[0].properties = Some(RunProperties {
            italic: Some(true),
            italic_complex: Some(true),
            ..Default::default()
        });
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(ApplyItalicCommand::new_with_enabled(range, false)),
                &mut ctx,
            )
            .unwrap();

        let properties = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(properties.italic, Some(false));
        assert_eq!(properties.italic_complex, Some(false));
    }

    #[test]
    fn rejects_collapsed_selection() {
        let mut document = sample_document();
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(collapsed);
        let mut executor = CommandExecutor::new();

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let result = executor.execute(Box::new(ApplyItalicCommand::new(collapsed)), &mut ctx);
        assert!(
            matches!(result, Err(CommandError::InvalidSelection)),
            "collapsed selections should be rejected"
        );
    }

    #[test]
    fn undo_and_redo_restore_italic_selection() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let range = selection.unwrap();
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(ApplyItalicCommand::new(range)), &mut ctx)
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
                .all(|run| run.properties.as_ref().and_then(|p| p.italic) != Some(true)),
            "undo should remove italic style"
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
                .any(|run| run.properties.as_ref().and_then(|p| p.italic) == Some(true)),
            "redo should re-apply italic style"
        );
        assert_eq!(selection.unwrap(), range);
    }

    #[test]
    fn italic_command_updates_exported_document_xml() {
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
                .execute(Box::new(ApplyItalicCommand::new(range)), &mut ctx)
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
            document_xml.contains("<w:i"),
            "expected exported XML to contain italic run properties: {document_xml}"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyItalicCommand::new_with_enabled(range, false)),
                    &mut ctx,
                )
                .unwrap();
        }
        let exported = word_doc.export_docx().expect("export disabled italic");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported.clone())).expect("zip archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        assert!(
            document_xml.contains(r#"<w:i w:val="0""#),
            "disabling italic must override Word character styles: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"<w:iCs w:val="0""#),
            "disabling italic must also override complex-script styles: {document_xml}"
        );
        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen disabled italic");
        assert!(reopened
            .paragraph(0)
            .expect("first paragraph")
            .runs
            .iter()
            .all(|run| {
                let properties = run.properties.as_ref();
                properties.and_then(|props| props.italic) != Some(true)
                    && properties.and_then(|props| props.italic_complex) != Some(true)
            }));
    }
}
