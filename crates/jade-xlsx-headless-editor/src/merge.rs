//! Merging and unmerging cells.

use jade_xlsx_headless_api::MergeSummary;

use crate::{HeadlessWorkbook, Result};

impl HeadlessWorkbook {
    /// Merges `range` into one cell, as Excel's Merge Cells does.
    pub fn merge_cells(&mut self, sheet: &str, range: &str) -> Result<MergeSummary> {
        let _ = (sheet, range);
        todo!("merge_cells")
    }

    /// Unmerges every merged range that `range` touches.
    pub fn unmerge_cells(&mut self, sheet: &str, range: &str) -> Result<Vec<String>> {
        let _ = (sheet, range);
        todo!("unmerge_cells")
    }
}
