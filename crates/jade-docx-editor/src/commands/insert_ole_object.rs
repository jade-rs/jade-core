//! 挿入 › オブジェクト › ファイルから: the object's picture inserted at the
//! caret and made the face of the file embedded as a package, as one undo
//! step.

use super::insert_image::InsertImageCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::ole_objects::ole_package;

pub struct InsertOleObjectCommand {
    image: InsertImageCommand,
    name: String,
    bytes: Vec<u8>,
    icon: bool,
}

impl InsertOleObjectCommand {
    /// `image` is the picture shown for the object — the file's icon and
    /// name when `icon`, its first page otherwise; `name` and `bytes` the
    /// file embedded.
    pub fn new(image: InsertImageCommand, name: String, bytes: Vec<u8>, icon: bool) -> Self {
        Self {
            image,
            name,
            bytes,
            icon,
        }
    }
}

impl Command for InsertOleObjectCommand {
    fn name(&self) -> &'static str {
        "insert_ole_object"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let package = ole_package(&self.name, &self.bytes).map_err(CommandError::Custom)?;
        self.image.apply(ctx)?;
        let target = self
            .image
            .inserted_target()
            .ok_or_else(|| CommandError::Custom("the picture was not inserted".into()))?;
        if let Err(error) = ctx.document.make_ole_object(
            target.paragraph_index,
            target.run_index,
            &package,
            "Package",
            self.icon,
        ) {
            self.image.undo(ctx)?;
            return Err(CommandError::Custom(error));
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.image.undo(ctx)
    }
}
