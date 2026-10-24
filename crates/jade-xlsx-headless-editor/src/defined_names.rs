//! The Name Manager's edits, with recalculation and a single undo step.

use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::DefinedName;

impl HeadlessWorkbook {
    /// Adds or updates the name in its workbook or worksheet scope.
    pub fn set_defined_name(&mut self, definition: &DefinedName) -> Result<bool> {
        let written = jade_xlsx_writer::defined_names::set_defined_name(&self.source, definition)?;
        if written == self.source {
            return Ok(false);
        }
        self.commit_sheet_change(written, |_| {})?;
        Ok(true)
    }

    /// Deletes a name in this scope, returning false if it did not exist.
    pub fn delete_defined_name(&mut self, name: &str, scope: Option<usize>) -> Result<bool> {
        let written =
            jade_xlsx_writer::defined_names::delete_defined_name(&self.source, name, scope)?;
        if written == self.source {
            return Ok(false);
        }
        self.commit_sheet_change(written, |_| {})?;
        Ok(true)
    }
}
