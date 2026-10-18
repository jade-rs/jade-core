//! Recalculating a workbook's formulas.
//!
//! The engine is [ironcalc](https://ironcalc.com), used the way the plan
//! says: jade's model is projected into an ironcalc model, evaluated, and the
//! results read back — never adopted as the model, because that would lose
//! the OOXML fidelity the reader exists to keep.
//!
//! ## What the projection refuses to guess
//!
//! The projection scopes the engine's shared date conversions to the workbook's
//! 1900 or 1904 epoch; ordinary numeric cells retain their stored values.
//!
//! - A formula the engine cannot make sense of — one that evaluates to
//!   `#NAME?` or `#ERROR!` where the file's cached value was something else —
//!   is **skipped**: the cell keeps its cached value, the skip is reported,
//!   and the cell is re-projected as that cached literal so its dependents
//!   still evaluate against what Excel last computed. A recomputed `#DIV/0!`
//!   or `#REF!` is a real result and comes through.

use ironcalc_base::cell::CellValue as EngineValue;
use ironcalc_base::Model;
use jade_xlsx_reader::{CellRef, CellValue, DateSystem, Workbook};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CalcError {
    /// Retained for API compatibility; evaluation now supports both epochs.
    #[error("the workbook counts dates from 1904, which the engine does not")]
    Unsupported1904,
    #[error("engine: {0}")]
    Engine(String),
}

pub type Result<T> = std::result::Result<T, CalcError>;

/// One formula cell's freshly computed value.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedCell {
    /// Tab-order index into [`Workbook::sheets`].
    pub sheet: usize,
    pub reference: CellRef,
    pub value: CellValue,
}

/// A formula cell the engine could not evaluate, left at its cached value.
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedCell {
    pub sheet: usize,
    pub reference: CellRef,
    /// The engine result that disqualified itself, e.g. `#NAME?`.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recalculated {
    /// Every formula cell the engine evaluated, cached-equal results included.
    pub cells: Vec<ComputedCell>,
    pub skipped: Vec<SkippedCell>,
}

/// Evaluates every formula in the workbook.
pub fn evaluate(workbook: &Workbook) -> Result<Recalculated> {
    evaluate_in_time_zone(workbook, "UTC")
}

/// Validate an IANA time-zone identifier against the calculation backend.
pub fn validate_time_zone(time_zone: &str) -> Result<()> {
    Model::new_empty("workbook", "en", time_zone, "en")
        .map(|_| ())
        .map_err(CalcError::Engine)
}

/// Evaluate with the host's time zone and one clock instant for the whole pass,
/// including any re-projections needed for unsupported formulas.
pub fn evaluate_in_time_zone(workbook: &Workbook, time_zone: &str) -> Result<Recalculated> {
    evaluate_with_manual_filter_rows(workbook, time_zone, &std::collections::BTreeMap::new())
}

/// Preserve known manual hides while evaluating a nondynamic filter snapshot.
pub fn evaluate_with_manual_filter_rows(
    workbook: &Workbook,
    time_zone: &str,
    manual: &std::collections::BTreeMap<String, HashSet<u32>>,
) -> Result<Recalculated> {
    let instant = ironcalc_base::get_milliseconds_since_epoch();
    let visibility = workbook
        .sheets
        .iter()
        .map(|sheet| {
            let (filtered_ranges, unknown_filtered_range) = sheet.filtered_row_ranges();
            ironcalc_base::row_visibility::SheetVisibility {
                default_hidden: sheet.default_rows_hidden,
                overrides: sheet
                    .rows
                    .iter()
                    .filter(|row| {
                        row.hidden != sheet.default_rows_hidden
                            && row.index < jade_xlsx_reader::model::MAX_ROW_COUNT
                    })
                    .map(|row| (row.index as i32 + 1, row.hidden))
                    .collect(),
                filtered_ranges: filtered_ranges
                    .into_iter()
                    .map(|(start, end)| (start as i32 + 1, end as i32 + 1))
                    .collect(),
                unknown_filtered_range,
                manual_hidden_rows: manual
                    .get(&sheet.name)
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .copied()
                    .chain(
                        sheet
                            .filter_row_state
                            .as_ref()
                            .filter(|state| !state.invalid)
                            .into_iter()
                            .flat_map(|state| state.manual.difference(&state.filtered).copied()),
                    )
                    .filter(|row| *row < jade_xlsx_reader::model::MAX_ROW_COUNT)
                    .map(|row| row as i32 + 1)
                    .collect(),
                uncertain_filtered_ranges: sheet
                    .filter_row_state
                    .as_ref()
                    .is_some_and(|state| state.invalid),
            }
        })
        .collect();
    ironcalc_base::row_visibility::with_visibility(visibility, || {
        ironcalc_base::clock::with_milliseconds(instant, || {
            evaluate_at_instant(workbook, time_zone)
        })
    })
}

fn evaluate_at_instant(workbook: &Workbook, time_zone: &str) -> Result<Recalculated> {
    let mut skipped: HashSet<(usize, CellRef)> = HashSet::new();
    let formula_count: usize = workbook
        .sheets
        .iter()
        .map(|sheet| {
            sheet
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .filter(|cell| cell.formula.is_some())
                .count()
        })
        .sum();

    // Fixed point over the skip set: a formula the engine answers with an
    // engine artifact is re-projected as its cached value and the model is
    // evaluated again. One cell per round, in document order — an artifact
    // often merely propagates from another one, and skipping the first lets
    // its dependents come back clean the next round instead of being written
    // off with it. `formula_count` bounds the rounds.
    let mut reasons: Vec<SkippedCell> = Vec::new();
    for _ in 0..=formula_count {
        let (model, projected, unparsed) = project(workbook, &skipped, time_zone)?;
        let round = read_back(&model, workbook, &projected)?;

        let Some(first) = round.disqualified.into_iter().next() else {
            reasons.extend(unparsed);
            return Ok(Recalculated {
                cells: round.cells,
                skipped: reasons,
            });
        };
        skipped.insert((first.sheet, first.reference));
        reasons.push(first);
    }
    Err(CalcError::Engine(
        "the skip set failed to converge".to_string(),
    ))
}

/// The engine model, which formula cells were projected as formulas, and the
/// formulas the engine's parser refused (projected as their cached values).
type Projection<'a> = (Model<'a>, Vec<(usize, CellRef)>, Vec<SkippedCell>);

fn project<'a>(
    workbook: &Workbook,
    skipped: &HashSet<(usize, CellRef)>,
    time_zone: &'a str,
) -> Result<Projection<'a>> {
    use ironcalc_base::date_system::{with_date_system, DateSystem as EngineDateSystem};
    let system = match workbook.date_system {
        DateSystem::Nineteen00 => EngineDateSystem::Excel1900,
        DateSystem::Nineteen04 => EngineDateSystem::Excel1904,
    };
    with_date_system(system, || project_with_epoch(workbook, skipped, time_zone))
}

fn project_with_epoch<'a>(
    workbook: &Workbook,
    skipped: &HashSet<(usize, CellRef)>,
    time_zone: &'a str,
) -> Result<Projection<'a>> {
    let mut model =
        Model::new_empty("workbook", "en", time_zone, "en").map_err(CalcError::Engine)?;

    for (index, sheet) in workbook.sheets.iter().enumerate() {
        if index == 0 {
            if sheet.name != "Sheet1" {
                model
                    .rename_sheet_by_index(0, &sheet.name)
                    .map_err(CalcError::Engine)?;
            }
        } else {
            model.add_sheet(&sheet.name).map_err(CalcError::Engine)?;
        }
    }

    // Names before formulas, so a projected formula that uses one parses.
    // A name the engine rejects is left out; formulas using it disqualify
    // themselves through the `#NAME?` path.
    for name in &workbook.defined_names {
        let scope = name
            .local_sheet_index
            .and_then(|index| u32::try_from(index).ok());
        let formula = jade_xlsx_reader::collapse_ref_errors(&name.formula);
        let _ = model.new_defined_name(&name.name, scope, &formula);
    }

    let mut projected = Vec::new();
    let mut unparsed = Vec::new();
    for (index, sheet) in workbook.sheets.iter().enumerate() {
        let engine_sheet = index as u32;
        for row in &sheet.rows {
            let engine_row = row.index as i32 + 1;
            for cell in &row.cells {
                let engine_column = cell.reference.column as i32 + 1;
                let as_cached = skipped.contains(&(index, cell.reference));
                if let (Some(formula), false) = (&cell.formula, as_cached) {
                    if model
                        .update_cell_with_formula(
                            engine_sheet,
                            engine_row,
                            engine_column,
                            // A reference into a deleted sheet reads `#REF!B4`
                            // in the file; Excel evaluates it as `#REF!`.
                            format!("={}", jade_xlsx_reader::collapse_ref_errors(formula)),
                        )
                        .is_ok()
                    {
                        projected.push((index, cell.reference));
                        continue;
                    }
                    // The formula did not parse: fall through to the cached
                    // value and report the skip.
                    unparsed.push(SkippedCell {
                        sheet: index,
                        reference: cell.reference,
                        reason: "the engine could not parse the formula".to_string(),
                    });
                }
                set_literal(&mut model, engine_sheet, engine_row, engine_column, cell);
            }
        }
    }
    model.evaluate();
    Ok((model, projected, unparsed))
}

fn set_literal(
    model: &mut Model<'_>,
    sheet: u32,
    row: i32,
    column: i32,
    cell: &jade_xlsx_reader::Cell,
) {
    // Projection failures here are deliberate no-ops: an unset cell reads as
    // empty, which is the conservative meaning for a value the engine cannot
    // hold (an error literal, say — the engine has no setter for those).
    let _ = match &cell.value {
        CellValue::Number(number) => model.update_cell_with_number(sheet, row, column, *number),
        CellValue::Text(text) => model.update_cell_with_text(sheet, row, column, text),
        CellValue::Bool(flag) => model.update_cell_with_bool(sheet, row, column, *flag),
        CellValue::Empty | CellValue::Error(_) => Ok(()),
    };
}

struct Round {
    cells: Vec<ComputedCell>,
    disqualified: Vec<SkippedCell>,
}

fn read_back(
    model: &Model<'_>,
    workbook: &Workbook,
    projected: &[(usize, CellRef)],
) -> Result<Round> {
    let mut cells = Vec::new();
    let mut disqualified = Vec::new();
    for (sheet_index, reference) in projected {
        let value = model
            .get_cell_value_by_index(
                *sheet_index as u32,
                reference.row as i32 + 1,
                reference.column as i32 + 1,
            )
            .map_err(CalcError::Engine)?;
        let value = to_cell_value(value);
        if is_engine_artifact(&value) && cached_value(workbook, *sheet_index, *reference) != value {
            disqualified.push(SkippedCell {
                sheet: *sheet_index,
                reference: *reference,
                reason: match &value {
                    CellValue::Error(text) => text.clone(),
                    _ => "engine artifact".to_string(),
                },
            });
            continue;
        }
        cells.push(ComputedCell {
            sheet: *sheet_index,
            reference: *reference,
            value,
        });
    }
    Ok(Round {
        cells,
        disqualified,
    })
}

fn cached_value(workbook: &Workbook, sheet: usize, reference: CellRef) -> CellValue {
    workbook.sheets[sheet]
        .rows
        .iter()
        .find(|row| row.index == reference.row)
        .and_then(|row| row.cells.iter().find(|cell| cell.reference == reference))
        .map(|cell| cell.value.clone())
        .unwrap_or(CellValue::Empty)
}

/// `#NAME?` and `#ERROR!` are what the engine says for a function or syntax
/// it does not know — results Excel would not have produced from a file whose
/// cached value differs, so they must never overwrite one.
fn is_engine_artifact(value: &CellValue) -> bool {
    matches!(value, CellValue::Error(text) if text == "#NAME?" || text == "#ERROR!")
}

fn to_cell_value(value: EngineValue) -> CellValue {
    match value {
        EngineValue::None => CellValue::Empty,
        EngineValue::Number(number) => CellValue::Number(number),
        EngineValue::Boolean(flag) => CellValue::Bool(flag),
        EngineValue::String(text) => {
            if is_excel_error(&text) {
                CellValue::Error(text)
            } else {
                CellValue::Text(text)
            }
        }
    }
}

fn is_excel_error(text: &str) -> bool {
    matches!(
        text,
        "#DIV/0!"
            | "#N/A"
            | "#NAME?"
            | "#NULL!"
            | "#NUM!"
            | "#REF!"
            | "#VALUE!"
            | "#ERROR!"
            | "#CALC!"
            | "#SPILL!"
            | "#CIRC!"
    )
}
