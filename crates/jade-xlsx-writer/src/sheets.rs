//! Adding, renaming, deleting and moving sheets.

use crate::Result;

/// Checks `name` against Excel's rules for a sheet name, and against the
/// names of the `others` sheets.
pub fn validate_sheet_name(name: &str, others: &[&str]) -> Result<()> {
    let _ = others;
    todo!("validate_sheet_name {name}")
}

/// Adds an empty worksheet called `name` after the sheet `after`, or at
/// the end. Returns the new bytes and the new sheet's position.
pub fn add_sheet(original: &[u8], name: &str, after: Option<&str>) -> Result<(Vec<u8>, usize)> {
    let _ = (original, after);
    todo!("add_sheet {name}")
}

/// Renames the sheet `old` to `new`.
pub fn rename_sheet(original: &[u8], old: &str, new: &str) -> Result<Vec<u8>> {
    let _ = original;
    todo!("rename_sheet {old} {new}")
}

/// Deletes the sheet `name`.
pub fn delete_sheet(original: &[u8], name: &str) -> Result<Vec<u8>> {
    let _ = original;
    todo!("delete_sheet {name}")
}

/// Moves the sheet `name` to position `to_index`.
pub fn move_sheet(original: &[u8], name: &str, to_index: usize) -> Result<Vec<u8>> {
    let _ = original;
    todo!("move_sheet {name} {to_index}")
}
