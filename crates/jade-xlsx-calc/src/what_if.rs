//! What-if questions — Goal Seek, data tables, Solver, a scenario's
//! results: a few input cells given trial values and the workbook evaluated
//! again in a kept engine model, the file itself untouched.

use std::collections::{BTreeMap, HashSet};

use ironcalc_base::Model;
use jade_xlsx_reader::{CellRef, CellValue, DateSystem, Workbook};

use crate::{
    cached_value, set_literal, settle, table_formula, to_cell_value, with_workbook_scopes,
    CalcError, Result,
};

/// The workbook evaluated once and kept, to be evaluated again with trial
/// values in its input cells.
pub struct WhatIf {
    model: Model<'static>,
    manual: BTreeMap<String, HashSet<u32>>,
    instant: i64,
    /// The cells a probe set, each put back as the workbook holds it before
    /// a probe that leaves it out.
    set: Vec<(usize, CellRef)>,
}

/// The workbook evaluated as a recalculation would, kept for [`WhatIf::probe`].
/// A formula the engine cannot answer holds its cached value, as it does in
/// a recalculation.
pub fn what_if(
    workbook: &Workbook,
    time_zone: &str,
    manual: &BTreeMap<String, HashSet<u32>>,
) -> Result<WhatIf> {
    let instant = ironcalc_base::get_milliseconds_since_epoch();
    let (_, model, _, _) = with_workbook_scopes(workbook, manual, instant, || {
        settle(workbook, time_zone, false)
    })?;
    Ok(WhatIf {
        model,
        manual: manual.clone(),
        instant,
        set: Vec::new(),
    })
}

impl WhatIf {
    /// What `outputs` evaluate to with each of `inputs` holding its value.
    /// The inputs an earlier probe set and this one leaves out hold what the
    /// workbook does again. `workbook` is the one the model was kept for.
    pub fn probe(
        &mut self,
        workbook: &Workbook,
        inputs: &[(usize, CellRef, CellValue)],
        outputs: &[(usize, CellRef)],
    ) -> Result<Vec<CellValue>> {
        let system = match workbook.date_system {
            DateSystem::Nineteen00 => ironcalc_base::date_system::DateSystem::Excel1900,
            DateSystem::Nineteen04 => ironcalc_base::date_system::DateSystem::Excel1904,
        };
        let restore: Vec<(usize, CellRef)> = self
            .set
            .iter()
            .filter(|cell| !inputs.iter().any(|(sheet, at, _)| (*sheet, *at) == **cell))
            .copied()
            .collect();
        let model = &mut self.model;
        let values = with_workbook_scopes(workbook, &self.manual, self.instant, || {
            ironcalc_base::date_system::with_date_system(system, || {
                for &(sheet, at) in &restore {
                    put_back(model, workbook, sheet, at);
                }
                for (sheet, at, value) in inputs {
                    clear(model, *sheet, *at);
                    set_literal(
                        model,
                        *sheet as u32,
                        at.row as i32 + 1,
                        at.column as i32 + 1,
                        value,
                    );
                }
                model.evaluate();
                outputs
                    .iter()
                    .map(|(sheet, at)| {
                        model
                            .get_cell_value_by_index(
                                *sheet as u32,
                                at.row as i32 + 1,
                                at.column as i32 + 1,
                            )
                            .map(to_cell_value)
                            .map_err(CalcError::Engine)
                    })
                    .collect::<Result<Vec<_>>>()
            })
        })?;
        self.set = inputs.iter().map(|(sheet, at, _)| (*sheet, *at)).collect();
        Ok(values)
    }

    /// The number `output` evaluates to with `input` holding `value`; `None`
    /// for anything else — text, an error, an empty cell.
    pub fn number(
        &mut self,
        workbook: &Workbook,
        input: (usize, CellRef),
        value: f64,
        output: (usize, CellRef),
    ) -> Result<Option<f64>> {
        let values = self.probe(
            workbook,
            &[(input.0, input.1, CellValue::Number(value))],
            &[output],
        )?;
        Ok(match values.first() {
            Some(CellValue::Number(number)) => Some(*number),
            _ => None,
        })
    }
}

/// Empties a cell of the model, so a value set in it replaces a formula.
fn clear(model: &mut Model<'static>, sheet: usize, at: CellRef) {
    if let Ok(worksheet) = model.workbook.worksheet_mut(sheet as u32) {
        let _ = worksheet.cell_clear_contents(at.row as i32 + 1, at.column as i32 + 1);
    }
}

/// A cell of the model as the workbook holds it again: its formula, its
/// value, or nothing.
fn put_back(model: &mut Model<'static>, workbook: &Workbook, sheet: usize, at: CellRef) {
    clear(model, sheet, at);
    let (row, column) = (at.row as i32 + 1, at.column as i32 + 1);
    let formula = workbook
        .sheets
        .get(sheet)
        .and_then(|found| found.cell_at(at))
        .and_then(|cell| cell.formula.clone());
    match formula {
        Some(formula) => {
            let text = format!(
                "={}",
                jade_xlsx_reader::collapse_ref_errors(&table_formula(
                    workbook,
                    sheet,
                    Some(at),
                    &formula
                ))
            );
            let _ = model.update_cell_with_formula(sheet as u32, row, column, text);
        }
        None => set_literal(
            model,
            sheet as u32,
            row,
            column,
            &cached_value(workbook, sheet, at),
        ),
    }
}
