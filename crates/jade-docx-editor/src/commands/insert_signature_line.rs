//! 挿入 › 署名欄: the signature line's picture inserted at the caret and
//! made a VML signature line, as one undo step.

use super::insert_image::InsertImageCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use jade_docx_reader::SignatureLine;

pub struct InsertSignatureLineCommand {
    image: InsertImageCommand,
    line: SignatureLine,
}

impl InsertSignatureLineCommand {
    pub fn new(image: InsertImageCommand, line: SignatureLine) -> Self {
        Self { image, line }
    }
}

impl Command for InsertSignatureLineCommand {
    fn name(&self) -> &'static str {
        "insert_signature_line"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.image.apply(ctx)?;
        let target = self
            .image
            .inserted_target()
            .ok_or_else(|| CommandError::Custom("the picture was not inserted".into()))?;
        if let Err(error) =
            ctx.document
                .make_signature_line(target.paragraph_index, target.run_index, &self.line)
        {
            self.image.undo(ctx)?;
            return Err(CommandError::Custom(error));
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.image.undo(ctx)
    }
}
