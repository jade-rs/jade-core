//! Data ▸ Advanced, filtering the list in place: the records the criteria
//! leave out hidden as filtered rows, the sheet in filter mode, and the
//! list and its criteria named `_xlnm._FilterDatabase` and
//! `_xlnm.Criteria` as Excel names them; Data ▸ Clear shows them again.

use std::collections::{BTreeMap, BTreeSet};

use jade_xlsx_reader::{quote_sheet_name, CellRange, DefinedName};

use crate::{HeadlessError, HeadlessWorkbook, Result};

/// The name Excel gives the list a filter was last applied to.
pub const FILTER_DATABASE: &str = "_xlnm._FilterDatabase";
/// The name Excel gives an Advanced Filter's criteria range.
pub const CRITERIA: &str = "_xlnm.Criteria";
/// The name Excel gives where an Advanced Filter last copied records to.
pub const EXTRACT: &str = "_xlnm.Extract";

fn absolute(range: CellRange) -> String {
    let cell = |at: jade_xlsx_reader::CellRef| {
        format!(
            "${}${}",
            jade_xlsx_layout::column_label(at.column),
            at.row + 1
        )
    };
    if range.start == range.end {
        cell(range.start)
    } else {
        format!("{}:{}", cell(range.start), cell(range.end))
    }
}

impl HeadlessWorkbook {
    /// A range of `sheet` as a defined name's formula: `'My Data'!$A$1:$D$9`.
    fn sheet_range(&self, sheet: usize, range: CellRange) -> String {
        format!(
            "{}!{}",
            quote_sheet_name(&self.workbook.sheets[sheet].name),
            absolute(range)
        )
    }

    /// The list `sheet`'s last Advanced Filter (or AutoFilter) was applied
    /// to, from `_xlnm._FilterDatabase`.
    pub fn filter_database(&self, sheet: &str) -> Result<Option<CellRange>> {
        let index = self.sheet_index(sheet)?;
        Ok(self.workbook.filter_database(&self.workbook.sheets[index]))
    }

    /// Data ▸ Advanced ▸ Filter the list, in-place: of the records of
    /// `list` (its first row the field names), those whose sheet rows are
    /// not in `shown` hidden as filtered rows and the rest shown, the sheet
    /// in filter mode, and `list` named `_xlnm._FilterDatabase` — with
    /// `criteria`, a range on the sheet of that index, named
    /// `_xlnm.Criteria`. One undo step; how many records are shown.
    pub fn advanced_filter(
        &mut self,
        sheet: &str,
        list: CellRange,
        shown: &BTreeSet<u32>,
        criteria: Option<(usize, CellRange)>,
    ) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "filtering a protected sheet is not allowed", |p| {
            p.auto_filter
        })?;
        if list.end.row <= list.start.row {
            return Err(HeadlessError::BadReference(
                "an advanced filter's list needs a record under its field names".into(),
            ));
        }
        if self.workbook.sheets[index]
            .auto_filter
            .as_ref()
            .is_some_and(|filter| filter.has_criteria())
        {
            return Err(HeadlessError::BadReference(
                "clear the sheet's AutoFilter before an advanced filter".into(),
            ));
        }
        let name = self.workbook.sheets[index].name.clone();
        let mut definitions = vec![DefinedName {
            name: FILTER_DATABASE.into(),
            formula: self.sheet_range(index, list),
            local_sheet_index: Some(index),
            hidden: true,
        }];
        if let Some((on, range)) = criteria {
            if on >= self.workbook.sheets.len() {
                return Err(HeadlessError::BadReference("no such criteria sheet".into()));
            }
            definitions.push(DefinedName {
                name: CRITERIA.into(),
                formula: self.sheet_range(on, range),
                local_sheet_index: Some(index),
                hidden: false,
            });
        }
        let written =
            jade_xlsx_writer::defined_names::set_built_in_names(&self.source, &definitions, &[])?;
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let sheet_model = &workbook.sheets[index];
        let mut state = self
            .filter_row_state
            .get(&name)
            .cloned()
            .unwrap_or_default();
        let mut hidden = BTreeMap::new();
        let mut count = 0;
        for row in list.start.row + 1..=list.end.row {
            let was_hidden = sheet_model
                .rows
                .binary_search_by_key(&row, |found| found.index)
                .ok()
                .map(|at| sheet_model.rows[at].hidden)
                .unwrap_or(sheet_model.default_rows_hidden);
            let keep = shown.contains(&row);
            count += u32::from(keep);
            // Every record is filtered or not anew; one hidden by hand
            // and kept by the criteria is shown, as Excel shows it.
            state.manual.remove(&row);
            if keep {
                state.filtered.remove(&row);
            } else {
                state.filtered.insert(row);
            }
            if was_hidden == keep {
                hidden.insert(row, !keep);
            }
        }
        let written = jade_xlsx_writer::auto_filter::write_filter_visibility(
            &written, &workbook, &name, &hidden, true,
        )?;
        self.commit_filter_bytes(written, &name, state)?;
        Ok(count)
    }

    /// Data ▸ Clear on a sheet an Advanced Filter filtered: every record of
    /// its list shown again and the sheet out of filter mode. One undo
    /// step; false when nothing was filtered.
    pub fn show_all_records(&mut self, sheet: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let model = &self.workbook.sheets[index];
        if model.filter_mode != Some(true)
            || model
                .auto_filter
                .as_ref()
                .is_some_and(|filter| filter.has_criteria())
        {
            return Ok(false);
        }
        let Some(list) = self.filter_database(sheet)? else {
            return Ok(false);
        };
        self.guard(index, "filtering a protected sheet is not allowed", |p| {
            p.auto_filter
        })?;
        let name = model.name.clone();
        let mut state = self
            .filter_row_state
            .get(&name)
            .cloned()
            .unwrap_or_default();
        let mut hidden = BTreeMap::new();
        for row in list.start.row + 1..=list.end.row {
            let was_hidden = model
                .rows
                .binary_search_by_key(&row, |found| found.index)
                .ok()
                .map(|at| model.rows[at].hidden)
                .unwrap_or(model.default_rows_hidden);
            let manual = state.manual.contains(&row) && !state.filtered.contains(&row);
            state.filtered.remove(&row);
            if was_hidden && !manual {
                hidden.insert(row, false);
            }
        }
        let written = jade_xlsx_writer::auto_filter::write_filter_visibility(
            &self.source,
            &self.workbook,
            &name,
            &hidden,
            false,
        )?;
        self.commit_filter_bytes(written, &name, state)?;
        Ok(true)
    }
}
