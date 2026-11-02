//! What-If data tables: made over a range as Excel's Data Table dialog
//! makes one, and worked out again — each result cell its formula with the
//! input cells given the values at the head of its row and column — with
//! every recalculation.

use std::collections::BTreeMap;

use jade_xlsx_reader::{CellRange, CellRef, CellValue, DataTable};
use jade_xlsx_writer::spills::SheetSpills;
use jade_xlsx_writer::CellEdit;

use crate::{spilled_value, HeadlessError, HeadlessWorkbook, Result};

impl HeadlessWorkbook {
    /// Data ▸ What-If Analysis ▸ Data Table over `range` — input values
    /// along its top row and down its left column, formulas round them —
    /// with `row` taking the values along the top and `column` those down
    /// the side. One of them makes a one-way table, both a two-way one with
    /// the formula in the corner. The results fill the range less its top
    /// row and left column; one undo step.
    pub fn set_data_table(
        &mut self,
        sheet: &str,
        range: CellRange,
        row: Option<CellRef>,
        column: Option<CellRef>,
    ) -> Result<DataTable> {
        let index = self.sheet_index(sheet)?;
        if range.end.row <= range.start.row
            || range.end.column <= range.start.column
            || (row.is_none() && column.is_none())
        {
            return Err(HeadlessError::BadReference(
                "a data table needs input values, a formula and an input cell".to_string(),
            ));
        }
        let results = CellRange {
            start: CellRef::new(range.start.column + 1, range.start.row + 1),
            end: range.end,
        };
        let table = match (row, column) {
            (Some(row), Some(column)) => DataTable {
                range: results,
                two_way: true,
                row: true,
                first: Some(row),
                second: Some(column),
            },
            (Some(row), None) => DataTable {
                range: results,
                two_way: false,
                row: true,
                first: Some(row),
                second: None,
            },
            (None, column) => DataTable {
                range: results,
                two_way: false,
                row: false,
                first: column,
                second: None,
            },
        };
        let cells: Vec<CellRef> = cells(results).collect();
        self.guard_cells(index, cells.iter().copied())?;
        let mut probe = self.what_if()?;
        let values = table_values(&mut probe, &self.workbook, index, &table)?;
        let part_name = self.workbook.sheets[index].part_name.clone();
        let edits = SheetSpills {
            part_name: part_name.clone(),
            values: values
                .into_iter()
                .map(|(reference, value)| CellEdit {
                    reference,
                    value: spilled_value(&value),
                    style_index: None,
                })
                .collect(),
            ranges: Vec::new(),
        };
        let written = jade_xlsx_writer::spills::write_spills(&self.source, &[edits], false)?;
        let mut tables: Vec<DataTable> = self.workbook.sheets[index]
            .data_tables
            .iter()
            .filter(|other| !overlaps(other.range, results))
            .copied()
            .collect();
        tables.push(table);
        let written = mark(&written, &[(part_name, tables)])?;
        self.commit_sheet_change(written, |_| {})?;
        Ok(table)
    }

    /// The data tables' results worked out again and written, as part of a
    /// recalculation — no undo step of their own. With `complete`, the
    /// workbook stops asking for a full calculation on load.
    pub(crate) fn write_data_tables(&mut self, complete: bool) -> Result<()> {
        if self
            .workbook
            .sheets
            .iter()
            .all(|sheet| sheet.data_tables.is_empty())
        {
            return Ok(());
        }
        let mut probe = self.what_if()?;
        let mut sheets: BTreeMap<usize, SheetSpills> = BTreeMap::new();
        for (index, sheet) in self.workbook.sheets.iter().enumerate() {
            for table in &sheet.data_tables {
                for (reference, value) in table_values(&mut probe, &self.workbook, index, table)? {
                    if self.cached_value(index, reference) == value {
                        continue;
                    }
                    sheets
                        .entry(index)
                        .or_insert_with(|| SheetSpills {
                            part_name: sheet.part_name.clone(),
                            values: Vec::new(),
                            ranges: Vec::new(),
                        })
                        .values
                        .push(CellEdit {
                            reference,
                            value: spilled_value(&value),
                            style_index: None,
                        });
                }
            }
        }
        if sheets.is_empty() {
            return Ok(());
        }
        let marks: Vec<(String, Vec<DataTable>)> = sheets
            .keys()
            .map(|&index| {
                let sheet = &self.workbook.sheets[index];
                (sheet.part_name.clone(), sheet.data_tables.clone())
            })
            .collect();
        let sheets: Vec<SheetSpills> = sheets.into_values().collect();
        let written = jade_xlsx_writer::spills::write_spills(&self.source, &sheets, complete)?;
        let written = mark(&written, &marks)?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        if self.automatic_height_base.is_none() {
            self.automatic_height_base = Some(self.source.clone());
        }
        self.source = written;
        self.range_auth_generation = self.range_auth_generation.wrapping_add(1);
        Ok(())
    }
}

/// Each cell of `range`, row by row.
fn cells(range: CellRange) -> impl Iterator<Item = CellRef> {
    (range.start.row..=range.end.row).flat_map(move |row| {
        (range.start.column..=range.end.column).map(move |column| CellRef::new(column, row))
    })
}

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.column <= b.end.column
        && b.start.column <= a.end.column
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}

/// What each result cell of `table` comes to: the formula heading its row
/// (a one-way table along the top), its column (down the side) or in the
/// corner (two-way), with the input cells given the values heading its
/// column and row. A table without a top row or left column has none.
fn table_values(
    probe: &mut jade_xlsx_calc::WhatIf,
    workbook: &jade_xlsx_reader::Workbook,
    sheet: usize,
    table: &DataTable,
) -> Result<Vec<(CellRef, CellValue)>> {
    let range = table.range;
    let (Some(top), Some(left)) = (
        range.start.row.checked_sub(1),
        range.start.column.checked_sub(1),
    ) else {
        return Ok(Vec::new());
    };
    let (row_input, column_input) = table.inputs();
    // The values heading the rows and columns, as the workbook works them
    // out now.
    let heads: Vec<(usize, CellRef)> = (range.start.column..=range.end.column)
        .map(|column| (sheet, CellRef::new(column, top)))
        .chain((range.start.row..=range.end.row).map(|row| (sheet, CellRef::new(left, row))))
        .collect();
    let values = probe.probe(workbook, &[], &heads)?;
    let width = (range.end.column - range.start.column + 1) as usize;
    let mut out = Vec::new();
    for (down, row) in (range.start.row..=range.end.row).enumerate() {
        for (across, column) in (range.start.column..=range.end.column).enumerate() {
            let mut inputs = Vec::new();
            if let Some(cell) = row_input {
                inputs.push((sheet, cell, values[across].clone()));
            }
            if let Some(cell) = column_input {
                inputs.push((sheet, cell, values[width + down].clone()));
            }
            let formula = if table.two_way {
                CellRef::new(left, top)
            } else if table.row {
                CellRef::new(left, row)
            } else {
                CellRef::new(column, top)
            };
            let value = probe
                .probe(workbook, &inputs, &[(sheet, formula)])?
                .pop()
                .unwrap_or(CellValue::Empty);
            out.push((CellRef::new(column, row), value));
        }
    }
    Ok(out)
}

/// The written package with each part's data tables marked on their first
/// cells.
fn mark(written: &[u8], parts: &[(String, Vec<DataTable>)]) -> Result<Vec<u8>> {
    let mut package = jade_opc::OpcPackage::open_preserving(written)?;
    for (part_name, tables) in parts {
        if let Some(xml) = package.part_str(part_name)? {
            let marked = jade_xlsx_writer::data_tables::mark_data_tables(&xml, tables)?;
            package.replace(part_name, marked.into_bytes());
        }
    }
    Ok(package.write()?)
}
