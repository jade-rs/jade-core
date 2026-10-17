use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

/// Undoable section page-number format and restart settings.
pub struct SetPageNumberFormatCommand {
    format: String,
    start: Option<u32>,
    section: Option<usize>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetPageNumberFormatCommand {
    pub fn new(format: String, start: Option<u32>, section: Option<usize>) -> Self {
        Self {
            format,
            start,
            section,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetPageNumberFormatCommand {
    fn name(&self) -> &'static str {
        "set_page_number_format"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx
            .document
            .set_page_number_format(&self.format, self.start, self.section)
        {
            Ok(changed) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.changed = changed;
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
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("missing page-number format snapshot for undo".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::CommandExecutor, word_document::WordDocument, Options};

    #[test]
    fn numbering_history_restores_package_and_selection_and_ignores_noops() {
        let bytes = include_bytes!("../../../../public/fixtures/hello.docx");
        let mut document =
            futures::executor::block_on(WordDocument::load(bytes, &Options::default())).unwrap();
        let mut selection = Some(SelectionRange::collapsed(
            jade_docx_layout::selection::TextPosition::new(0, 1),
        ));
        let original_selection = selection;
        let before = document.export_docx().unwrap();
        let mut executor = CommandExecutor::new();
        for _ in 0..2 {
            executor
                .execute(
                    Box::new(SetPageNumberFormatCommand::new(
                        "upperRoman".into(),
                        Some(3),
                        Some(0),
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
        }
        let after = document.export_docx().unwrap();
        assert_ne!(before, after);
        assert!(executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), before);
        assert_eq!(selection, original_selection);
        assert!(!executor.can_undo());
        assert!(executor
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), after);
        let result = executor.execute(
            Box::new(SetPageNumberFormatCommand::new(
                "invalid".into(),
                Some(0),
                Some(0),
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        );
        assert!(result.is_err());
        assert_eq!(document.export_docx().unwrap(), after);
    }
}
