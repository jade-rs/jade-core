//! Adding, renaming, deleting and moving sheets.

use crate::{HeadlessWorkbook, Result};

impl HeadlessWorkbook {
    /// Adds an empty worksheet called `name` after the sheet `after`, or
    /// after the last sheet. Returns its position.
    pub fn add_sheet(&mut self, name: &str, after: Option<&str>) -> Result<usize> {
        let _ = after;
        todo!("add_sheet {name}")
    }

    /// Renames the sheet `old` to `new`.
    pub fn rename_sheet(&mut self, old: &str, new: &str) -> Result<()> {
        todo!("rename_sheet {old} {new}")
    }

    /// Deletes the sheet `name`.
    pub fn delete_sheet(&mut self, name: &str) -> Result<()> {
        todo!("delete_sheet {name}")
    }

    /// Moves the sheet `name` to position `to_index`.
    pub fn move_sheet(&mut self, name: &str, to_index: usize) -> Result<()> {
        todo!("move_sheet {name} {to_index}")
    }
}
