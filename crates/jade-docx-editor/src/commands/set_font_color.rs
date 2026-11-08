//! Explicit text color for selected text and paragraph marks.

use super::set_run_format::SetRunFormatCommand;
use super::{Command, CommandContext, CommandResult};
use jade_docx_headless_api::HeadlessRunFormat;
use jade_docx_layout::selection::SelectionRange;

/// Assigns the provided font color to selected text and paragraph marks.
pub struct SetFontColorCommand {
    inner: SetRunFormatCommand,
}

impl SetFontColorCommand {
    pub fn new(range: SelectionRange, color: impl Into<String>) -> Self {
        let color = color.into();
        // The document model also accepts #auto; keep the OOXML enum lowercase.
        let color = if color
            .trim()
            .trim_start_matches('#')
            .eq_ignore_ascii_case("auto")
        {
            "auto".to_string()
        } else {
            color
        };
        Self {
            inner: SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    color: Some(color),
                    ..Default::default()
                },
            ),
        }
    }
}

impl Command for SetFontColorCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_font_color"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.apply(ctx)
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.undo(ctx)
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
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

    #[test]
    fn color_command_updates_exported_document_xml() {
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
                .execute(
                    Box::new(SetFontColorCommand::new(range, "#00FF00")),
                    &mut ctx,
                )
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
            document_xml.contains("<w:color"),
            "expected exported XML to contain color run properties: {document_xml}"
        );
    }

    #[test]
    fn auto_color_exports_the_word_auto_value() {
        // Word's 自動 entry writes `w:color w:val="auto"`; the model stores it
        // as "#auto" (the parser's convention), and export must strip the
        // prefix back off rather than emitting an invalid hex value.
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
                .execute(Box::new(SetFontColorCommand::new(range, "#auto")), &mut ctx)
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
            document_xml.contains(r#"<w:color w:val="auto""#),
            "expected exported XML to carry the auto color: {document_xml}"
        );
    }
}
