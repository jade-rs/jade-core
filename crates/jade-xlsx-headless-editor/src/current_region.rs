use super::*;
use jade_xlsx_headless_api::{AutoFilterChange, AutoFilterSummary, AutoFilterTarget};
use std::collections::BTreeSet;

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.column <= b.end.column
        && b.start.column <= a.end.column
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}
fn range_text(range: CellRange) -> String {
    let cell =
        |reference: CellRef| format!("{}{}", column_label(reference.column), reference.row + 1);
    if range.start == range.end {
        cell(range.start)
    } else {
        format!("{}:{}", cell(range.start), cell(range.end))
    }
}

impl HeadlessWorkbook {
    /// Discover a data region without visiting the unallocated worksheet grid.
    /// Formatting-only cells do not connect regions; formula cells do.
    pub fn current_region(&self, sheet_name: &str, reference: &str) -> Result<String> {
        let index = self.sheet_index(sheet_name)?;
        let sheet = &self.workbook.sheets[index];
        jade_xlsx_writer::auto_filter::validate_filter_edit(
            &self.source,
            &self.workbook,
            &sheet.name,
            true,
        )?;
        let seed =
            CellRef::parse(reference).map_err(|_| HeadlessError::BadReference(reference.into()))?;
        let mut bounds = sheet.merge_at(seed).copied().unwrap_or(CellRange {
            start: seed,
            end: seed,
        });
        let mut rows: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut columns: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut inspected = 0u64;
        for row in &sheet.rows {
            for cell in &row.cells {
                inspected += 1;
                if inspected > RANGE_CELL_LIMIT {
                    return Err(HeadlessError::AutoFilter(
                        "current region inspection limit exceeded".into(),
                    ));
                }
                if cell.formula.is_some() || !cell.value.is_empty() {
                    rows.entry(cell.reference.row)
                        .or_default()
                        .insert(cell.reference.column);
                    columns
                        .entry(cell.reference.column)
                        .or_default()
                        .insert(cell.reference.row);
                }
            }
        }
        inspected += sheet.merges.len() as u64;
        if inspected > RANGE_CELL_LIMIT {
            return Err(HeadlessError::AutoFilter(
                "current region inspection limit exceeded".into(),
            ));
        }
        let merges: Vec<_> = sheet
            .merges
            .iter()
            .filter(|range| {
                rows.get(&range.start.row)
                    .is_some_and(|row| row.contains(&range.start.column))
            })
            .copied()
            .collect();
        let occupied = rows
            .range(bounds.start.row..=bounds.end.row)
            .any(|(_, row)| {
                row.range(bounds.start.column..=bounds.end.column)
                    .next()
                    .is_some()
            });
        if !occupied {
            return Ok(range_text(bounds));
        }
        loop {
            let before = bounds;
            let left = bounds.start.column.saturating_sub(1);
            let right = (bounds.end.column + 1).min(jade_xlsx_reader::model::MAX_COLUMN_COUNT - 1);
            let top = bounds.start.row.saturating_sub(1);
            let bottom = (bounds.end.row + 1).min(jade_xlsx_reader::model::MAX_ROW_COUNT - 1);
            let row_has_data = |row| {
                rows.get(&row)
                    .is_some_and(|cells| cells.range(left..=right).next().is_some())
            };
            let col_has_data = |column| {
                columns
                    .get(&column)
                    .is_some_and(|cells| cells.range(top..=bottom).next().is_some())
            };
            if row_has_data(top) {
                bounds.start.row = top;
            }
            if row_has_data(bottom) {
                bounds.end.row = bottom;
            }
            if col_has_data(left) {
                bounds.start.column = left;
            }
            if col_has_data(right) {
                bounds.end.column = right;
            }
            // Filled merged cells occupy rectangles, without allocating one entry per cell.
            let halo = CellRange {
                start: CellRef::new(left, top),
                end: CellRef::new(right, bottom),
            };
            for merge in &merges {
                inspected += 1;
                if inspected > RANGE_CELL_LIMIT {
                    return Err(HeadlessError::AutoFilter(
                        "current region inspection limit exceeded".into(),
                    ));
                }
                if overlaps(*merge, halo) {
                    bounds.start.column = bounds.start.column.min(merge.start.column);
                    bounds.start.row = bounds.start.row.min(merge.start.row);
                    bounds.end.column = bounds.end.column.max(merge.end.column);
                    bounds.end.row = bounds.end.row.max(merge.end.row);
                }
            }
            if before == bounds {
                return Ok(range_text(bounds));
            }
        }
    }

    /// Toggle the selected table, otherwise the worksheet filter. A single
    /// cell creates a filter for its current data region.
    pub fn toggle_auto_filter(
        &mut self,
        sheet_name: &str,
        reference: &str,
    ) -> Result<AutoFilterSummary> {
        let index = self.sheet_index(sheet_name)?;
        let sheet = &self.workbook.sheets[index];
        let range = CellRange::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.into()))?;
        if range.start.row > range.end.row || range.start.column > range.end.column {
            return Err(HeadlessError::BadReference(reference.into()));
        }
        let change = if let Some(table) = sheet
            .tables
            .iter()
            .find(|table| table.reference.contains(range.start))
        {
            let target = AutoFilterTarget::Table {
                name: table.name.clone(),
            };
            if table.filter.is_some() {
                AutoFilterChange::Disable { target }
            } else {
                AutoFilterChange::Enable {
                    target,
                    reference: None,
                }
            }
        } else if sheet.auto_filter.is_some() {
            AutoFilterChange::Disable {
                target: AutoFilterTarget::Worksheet,
            }
        } else {
            let reference = if range.start == range.end {
                self.current_region(sheet_name, reference)?
            } else {
                range_text(range)
            };
            let range = CellRange::parse(&reference)?;
            if sheet
                .tables
                .iter()
                .any(|table| overlaps(table.reference, range))
            {
                return Err(HeadlessError::AutoFilter(
                    "worksheet filter range overlaps a table".into(),
                ));
            }
            AutoFilterChange::Enable {
                target: AutoFilterTarget::Worksheet,
                reference: Some(reference),
            }
        };
        self.change_auto_filter(sheet_name, &change)
    }
}
