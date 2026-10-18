//! Adding, renaming, deleting and moving sheets.
//!
//! Each is a patch of the package by [`jade_xlsx_writer::sheets`] — see
//! there for what Excel does and so what each one rewrites — followed by a
//! reread, one undo step, and a recalculation, since a renamed or deleted
//! sheet changes what formulas say and a moved one can change what a 3-D
//! reference spans.

use std::collections::BTreeMap;

use crate::{HeadlessWorkbook, Result};

impl HeadlessWorkbook {
    /// Adds an empty worksheet called `name` right after the sheet `after`
    /// (a name or an index, as everywhere here), or after the last sheet
    /// when `after` is `None`. Returns the new sheet's position in the tab
    /// order. The name must be one Excel accepts — 1 to 31 characters, none
    /// of `[ ] : * ? / \`, no apostrophe at either end, not `History` — and
    /// no other sheet's, compared without regard to case. One undo step.
    pub fn add_sheet(&mut self, name: &str, after: Option<&str>) -> Result<usize> {
        let after = after
            .map(|selector| {
                self.sheet_index(selector)
                    .map(|index| self.workbook.sheets[index].name.clone())
            })
            .transpose()?;
        let (written, at) =
            jade_xlsx_writer::sheets::add_sheet(&self.source, name, after.as_deref())?;
        self.commit_sheet_change(written, |_| {})?;
        Ok(at)
    }

    /// Renames the sheet `old` to `new`, rewriting — as Excel does — every
    /// formula, defined name, conditional format and validation formula,
    /// table formula and chart series that names it (`Old!A1` becomes
    /// `'New name'!A1`, quoted as the new name needs), and a pivot cache's
    /// source sheet. Text inside string literals is not a reference and is
    /// left alone, as are hyperlink locations, which Excel does not follow
    /// either. The new name is validated as for [`Self::add_sheet`]; a
    /// change of case alone is allowed, and renaming a sheet to exactly the
    /// name it has writes nothing. One undo step.
    pub fn rename_sheet(&mut self, old: &str, new: &str) -> Result<()> {
        let index = self.sheet_index(old)?;
        let current = self.workbook.sheets[index].name.clone();
        let written = jade_xlsx_writer::sheets::rename_sheet(&self.source, &current, new)?;
        if written == self.source {
            return Ok(());
        }
        self.commit_sheet_change(written, |states| {
            if let Some(state) = states.remove(&current) {
                states.insert(new.to_string(), state);
            }
        })
    }

    /// Deletes the sheet `name`. As in Excel, every reference into it
    /// elsewhere becomes `#REF!` (`Sheet2!A1` reads `#REF!A1`; a structured
    /// reference into one of its tables reads `#REF!`), the names scoped
    /// to it are removed, and its worksheet part goes with every part only
    /// it reached — tables, drawings, charts, comments. When it was the
    /// active tab, the next visible sheet becomes active. The last visible
    /// sheet cannot be deleted. One undo step.
    pub fn delete_sheet(&mut self, name: &str) -> Result<()> {
        let index = self.sheet_index(name)?;
        let current = self.workbook.sheets[index].name.clone();
        let written = jade_xlsx_writer::sheets::delete_sheet(&self.source, &current)?;
        self.commit_sheet_change(written, |states| {
            states.remove(&current);
        })
    }

    /// Moves the sheet `name` to position `to_index` of the tab order, 0
    /// being the first tab; `to_index` must be less than the number of
    /// sheets. Formulas are unchanged — they name sheets — and the names
    /// scoped to sheets and the active tab follow their sheets. Moving a
    /// sheet to where it is writes nothing. One undo step.
    pub fn move_sheet(&mut self, name: &str, to_index: usize) -> Result<()> {
        let index = self.sheet_index(name)?;
        let current = self.workbook.sheets[index].name.clone();
        let written = jade_xlsx_writer::sheets::move_sheet(&self.source, &current, to_index)?;
        if written == self.source {
            return Ok(());
        }
        self.commit_sheet_change(written, |_| {})
    }

    /// Makes `written` the workbook — one undo step — with the per-sheet
    /// filter state, which is keyed by sheet name, following the change
    /// before formulas are recalculated against it.
    fn commit_sheet_change(
        &mut self,
        written: Vec<u8>,
        follow: impl FnOnce(&mut BTreeMap<String, crate::auto_filter::FilterRowState>),
    ) -> Result<()> {
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        self.workbook = workbook;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        follow(&mut self.filter_row_state);
        self.recalculate_after_edit();
        Ok(())
    }
}
