//! Document settings toggle command.
//!
//! Enables or disables a boolean on/off element directly under `<w:settings>`
//! (e.g. `w:evenAndOddHeaders`, `w:autoHyphenation`, `w:mirrorMargins`,
//! `w:trackChanges`). This is the headless entry point for editing
//! `word/settings.xml`, which is otherwise only ever read.

use super::{Command, CommandContext, CommandError, CommandResult};

/// Toggles a single boolean setting and supports undo by restoring the previous
/// `settings.xml` snapshot.
pub struct SetDocumentSettingCommand {
    name: String,
    enabled: bool,
    previous_settings_xml: Option<String>,
}

impl SetDocumentSettingCommand {
    pub fn new(name: impl Into<String>, enabled: bool) -> Self {
        Self {
            name: name.into(),
            enabled,
            previous_settings_xml: None,
        }
    }
}

impl Command for SetDocumentSettingCommand {
    fn name(&self) -> &'static str {
        "set_document_setting"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = ctx
            .document
            .set_settings_flag(&self.name, self.enabled)
            .map_err(CommandError::Custom)?;
        self.previous_settings_xml = Some(previous);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous_settings_xml
            .take()
            .ok_or_else(|| CommandError::Custom("missing settings backup for undo".into()))?;
        ctx.document.restore_settings_xml(previous);
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

    fn exported_settings(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive");
        let mut settings = String::new();
        archive
            .by_name("word/settings.xml")
            .expect("settings.xml present")
            .read_to_string(&mut settings)
            .expect("read settings.xml");
        settings
    }

    #[test]
    fn set_document_setting_applies_and_undoes() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        assert!(!exported_settings(&word_doc).contains("trackChanges"));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetDocumentSettingCommand::new("w:trackChanges", true)),
                    &mut ctx,
                )
                .expect("apply succeeds");
        }
        assert!(exported_settings(&word_doc).contains("<w:trackChanges"));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo succeeds"));
        }
        assert!(!exported_settings(&word_doc).contains("trackChanges"));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo succeeds"));
        }
        assert!(exported_settings(&word_doc).contains("<w:trackChanges"));
    }
}
