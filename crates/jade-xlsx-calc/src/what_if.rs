//! What-if questions — Goal Seek, data tables, Solver, a scenario's
//! results: a few input cells given trial values and the workbook evaluated
//! again in a kept engine model, the file itself untouched.

use std::collections::{BTreeMap, HashMap, HashSet};

use ironcalc_base::Model;
use jade_xlsx_reader::{CellRef, CellValue, DateSystem, Workbook};

use crate::{
    cached_value, read_cell_value, set_literal, settle, table_formula, with_workbook_scopes, Result,
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
    held: HashMap<(usize, CellRef), CellValue>,
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
    let (outcome, model, _, _) = with_workbook_scopes(workbook, manual, instant, || {
        settle(workbook, time_zone, false)
    })?;
    let mut held = HashMap::new();
    for skipped in outcome.skipped {
        let value = read_cell_value(
            &model,
            skipped.sheet as u32,
            skipped.reference.row as i32 + 1,
            skipped.reference.column as i32 + 1,
        )?;
        held.insert((skipped.sheet, skipped.reference), value);
    }
    Ok(WhatIf {
        model,
        manual: manual.clone(),
        instant,
        set: Vec::new(),
        held,
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
        let held = &self.held;
        let values = with_workbook_scopes(workbook, &self.manual, self.instant, || {
            ironcalc_base::date_system::with_date_system(system, || {
                for &(sheet, at) in &restore {
                    put_back(model, workbook, sheet, at, held.get(&(sheet, at)));
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
                        read_cell_value(
                            model,
                            *sheet as u32,
                            at.row as i32 + 1,
                            at.column as i32 + 1,
                        )
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

    /// Whether `formula`, written for the cell `anchor` of `sheet`, holds at
    /// each of `cells` of that sheet — its relative references moved along,
    /// as an Advanced Filter's computed criterion is decided for each
    /// record — with the workbook's values as they are. `None` when the
    /// engine cannot read the formula.
    pub fn formula_holds(
        &mut self,
        workbook: &Workbook,
        sheet: usize,
        anchor: CellRef,
        formula: &str,
        cells: &[CellRef],
    ) -> Result<Option<Vec<bool>>> {
        let system = match workbook.date_system {
            DateSystem::Nineteen00 => ironcalc_base::date_system::DateSystem::Excel1900,
            DateSystem::Nineteen04 => ironcalc_base::date_system::DateSystem::Excel1904,
        };
        let restore = std::mem::take(&mut self.set);
        let model = &mut self.model;
        let held = &self.held;
        let formula = jade_xlsx_reader::collapse_ref_errors(&table_formula(
            workbook,
            sheet,
            Some(anchor),
            formula,
        ));
        with_workbook_scopes(workbook, &self.manual, self.instant, || {
            ironcalc_base::date_system::with_date_system(system, || {
                if !restore.is_empty() {
                    for &(on, at) in &restore {
                        put_back(model, workbook, on, at, held.get(&(on, at)));
                    }
                    model.evaluate();
                }
                let Some(parsed) = model.parse_anchored_formula(
                    &formula,
                    sheet as u32,
                    anchor.row as i32 + 1,
                    anchor.column as i32 + 1,
                ) else {
                    return Ok(None);
                };
                Ok(Some(
                    cells
                        .iter()
                        .map(|at| {
                            model.anchored_formula_holds(
                                &parsed,
                                sheet as u32,
                                at.row as i32 + 1,
                                at.column as i32 + 1,
                            )
                        })
                        .collect(),
                ))
            })
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
fn put_back(
    model: &mut Model<'static>,
    workbook: &Workbook,
    sheet: usize,
    at: CellRef,
    held: Option<&CellValue>,
) {
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
            if let Some(value) = held {
                set_literal(model, sheet as u32, row, column, value);
            } else {
                let _ = model.update_cell_with_formula(sheet as u32, row, column, text);
            }
            model.set_source_formula(
                sheet as u32,
                row,
                column,
                Some(format!(
                    "={}",
                    crate::formula_text::to_display_formula(
                        formula.strip_prefix('=').unwrap_or(&formula)
                    )
                )),
            );
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
