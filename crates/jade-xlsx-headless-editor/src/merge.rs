//! Merging and unmerging cells.
//!
//! Excel's Merge Cells keeps what the top-left cell holds and discards the
//! value or formula of every other cell in the range — its own warning says
//! it "only keeps the upper-left value and discards other values" — and the
//! merged cell wears the top-left cell's format. The members are written
//! with that format too, as Excel writes them, which is why every former
//! member still wears it after Unmerge. Plain Merge Cells changes no
//! alignment; centring is Merge & Center, a separate command.
//!
//! Excel will not merge into a table, and a merge that overlaps one the
//! sheet already has is refused here rather than silently combined.

use jade_xlsx_headless_api::MergeSummary;
use jade_xlsx_reader::{CellRange, CellRef};
use jade_xlsx_writer::{CellEdit, NewValue};

use crate::{
    effective_style, ensure_dense_result_fits, find_cell, find_row, format_range, format_reference,
    parse_range, HeadlessError, HeadlessWorkbook, Result,
};

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.column <= b.end.column
        && b.start.column <= a.end.column
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}

impl HeadlessWorkbook {
    /// Merges `range` (`B2:C3`, either way round) into one cell, as Excel's
    /// Merge Cells does: the top-left cell keeps its value or formula, every
    /// other cell's value and formula is cleared, and every cell of the
    /// range takes the top-left cell's format. Formulas that read the
    /// cleared cells are recalculated.
    ///
    /// Refused, changing nothing: a single cell, a range overlapping a merge
    /// the sheet already has, and a range touching an Excel table. One
    /// write, one undo step.
    pub fn merge_cells(&mut self, sheet: &str, range: &str) -> Result<MergeSummary> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "merging cells is not allowed", |p| p.format_cells)?;
        let range = parse_range(range)?;
        if range.start == range.end {
            return Err(HeadlessError::MergeTooSmall(format_reference(range.start)));
        }
        ensure_dense_result_fits(
            range.end.column - range.start.column + 1,
            range.end.row - range.start.row + 1,
        )?;
        let model = &self.workbook.sheets[index];
        if let Some(existing) = model.merges.iter().find(|merge| overlaps(**merge, range)) {
            return Err(HeadlessError::MergeOverlaps {
                range: format_range(range),
                existing: format_range(*existing),
            });
        }
        if let Some(table) = model
            .tables
            .iter()
            .find(|table| overlaps(table.reference, range))
        {
            return Err(HeadlessError::MergeInTable {
                range: format_range(range),
                table: table.name.clone(),
            });
        }

        let anchor_style = effective_style(model, range.start);
        let mut cleared = Vec::new();
        let mut edits = Vec::new();
        for row in range.start.row..=range.end.row {
            let source_row = find_row(model, row);
            for column in range.start.column..=range.end.column {
                let at = CellRef::new(column, row);
                if at == range.start {
                    continue;
                }
                let holds_something = source_row
                    .and_then(|row| find_cell(row, column))
                    .is_some_and(|cell| !cell.value.is_empty() || cell.formula.is_some());
                let value = if holds_something {
                    cleared.push(format_reference(at));
                    NewValue::Blank
                } else if effective_style(model, at) != anchor_style {
                    NewValue::Unchanged
                } else {
                    continue;
                };
                edits.push((
                    index,
                    CellEdit {
                        reference: at,
                        value,
                        style_index: Some(anchor_style),
                    },
                ));
            }
        }

        let part_name = model.part_name.clone();
        let mut merges = model.merges.clone();
        merges.push(range);
        let sheet_name = model.name.clone();
        let patched = jade_xlsx_writer::set_merges(&self.sheet_part(&part_name)?, &merges)?;
        self.write_edits_with_parts(edits, None, vec![(part_name, patched)])?;
        Ok(MergeSummary {
            sheet: sheet_name,
            range: format_range(range),
            cleared,
        })
    }

    /// Unmerges every merged range that `range` touches — the exact merged
    /// range, one of its cells, or a selection across several — as Excel's
    /// Unmerge Cells does on a selection. Values and formats stay where they
    /// are: the top-left cell keeps what the merge kept, and the others are
    /// empty and wear its format. Returns the ranges unmerged, in the order
    /// the sheet lists them; a range touching no merge is refused. One undo
    /// step.
    pub fn unmerge_cells(&mut self, sheet: &str, range: &str) -> Result<Vec<String>> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "merging cells is not allowed", |p| p.format_cells)?;
        let range = parse_range(range)?;
        let model = &self.workbook.sheets[index];
        let (removed, kept): (Vec<CellRange>, Vec<CellRange>) = model
            .merges
            .iter()
            .partition(|merge| overlaps(**merge, range));
        if removed.is_empty() {
            return Err(HeadlessError::NoSuchMerge(format_range(range)));
        }
        let part_name = model.part_name.clone();
        let patched = jade_xlsx_writer::set_merges(&self.sheet_part(&part_name)?, &kept)?;
        self.replace_parts(vec![(part_name, patched)])?;
        Ok(removed.into_iter().map(format_range).collect())
    }

    fn sheet_part(&self, part_name: &str) -> Result<String> {
        Ok(jade_opc::OpcPackage::open(&self.source)?
            .part_str(part_name)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(part_name.to_string()))?)
    }
}
