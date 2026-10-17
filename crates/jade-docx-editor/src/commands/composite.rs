//! One undo step for an edit the UI performs as several commands.
//!
//! Word's table ribbon works on the selected cell range: 行の削除 with three
//! rows selected removes three rows, 塗りつぶし fills every selected cell.
//! jade expresses those as one headless command per row/cell, which would
//! otherwise leave three (or six) entries on the undo stack for a single
//! click. [`CompositeCommand`] holds the already-applied children so the
//! executor can record them as one entry: undo reverses them last-to-first,
//! redo re-applies them first-to-last.

use super::{Command, CommandContext, CommandResult};

/// A batch of commands that were applied in order and must undo as one.
pub struct CompositeCommand {
    name: &'static str,
    /// Application order. Undo walks this in reverse.
    children: Vec<Box<dyn Command>>,
}

impl CompositeCommand {
    /// Wraps children that have **already** been applied to the document. The
    /// group borrows the first child's name so history keeps reading as the
    /// operation the user asked for (`delete_table_row`, …).
    pub fn from_applied(children: Vec<Box<dyn Command>>) -> Self {
        let name = children
            .first()
            .map(|child| child.name())
            .unwrap_or("composite");
        Self { name, children }
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl Command for CompositeCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    /// Only ever reached as a redo — the children are applied before the group
    /// is built — so each child takes its own redo path.
    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for child in self.children.iter_mut() {
            child.redo(ctx)?;
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for child in self.children.iter_mut().rev() {
            child.undo(ctx)?;
        }
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        !self.children.is_empty()
    }
}
