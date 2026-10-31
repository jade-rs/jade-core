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
//! - With nothing cached — a formula just typed — there is nothing to keep:
//!   the cell takes the engine's answer, the `#NAME?` Excel shows for a
//!   misspelt function, and is still reported as skipped, so the file asks
//!   for a full calculation on load and an Excel that knows the function
//!   computes it. While the file asks for one, a cached artifact is not
//!   trusted as Excel's and the cell stays skipped.
//! - A skipped cell is projected as its value — an error included — so its
//!   dependents read what it shows, as in Excel. Only a cell whose own
//!   formula the engine fails is skipped; one that merely passes another
//!   cell's error along takes it.

pub mod conditional;
pub mod formula_text;
mod pivot;
pub mod validation;

use ironcalc_base::cell::CellValue as EngineValue;
use ironcalc_base::Model;
use jade_xlsx_reader::{CellRef, CellValue, DateSystem, Workbook};
use std::collections::{HashMap, HashSet};
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
    /// The array formulas that spilled beyond their anchor cell, in document
    /// order: plain cells in the file, but the formula's values.
    pub spills: Vec<Spill>,
    /// Present only when explicitly requested by a dependency-aware consumer.
    pub dependencies: Option<ValueDependencies>,
}

/// Observed value dependencies from one actual calculation. Rectangles include
/// empty cells and are never expanded into whole-axis coordinate lists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValueDependencies {
    /// (formula sheet index, formula cell) -> (read sheet index, read range).
    pub reads:
        std::collections::BTreeMap<(usize, CellRef), Vec<(usize, jade_xlsx_reader::CellRange)>>,
    pub incomplete: bool,
}
fn value_dependencies(model: &mut Model<'_>) -> ValueDependencies {
    let trace = model.take_value_reads();
    let mut result = ValueDependencies {
        incomplete: trace.incomplete,
        ..Default::default()
    };
    for (cell, reads) in trace.cells {
        if cell.row <= 0 || cell.column <= 0 {
            result.incomplete = true;
            continue;
        }
        let key = (
            cell.sheet as usize,
            CellRef::new((cell.column - 1) as u32, (cell.row - 1) as u32),
        );
        let entries = result.reads.entry(key).or_default();
        for area in reads {
            if area.first_row <= 0 || area.first_column <= 0 {
                result.incomplete = true;
                continue;
            }
            entries.push((
                area.sheet as usize,
                jade_xlsx_reader::CellRange {
                    start: CellRef::new(
                        (area.first_column - 1) as u32,
                        (area.first_row - 1) as u32,
                    ),
                    end: CellRef::new((area.last_column - 1) as u32, (area.last_row - 1) as u32),
                },
            ));
        }
        entries.sort_by_key(|(sheet, r)| (*sheet, r.start, r.end));
    }
    result
}

/// What one array formula spilled.
#[derive(Debug, Clone, PartialEq)]
pub struct Spill {
    pub sheet: usize,
    pub anchor: CellRef,
    /// The anchor and every cell the formula filled.
    pub range: jade_xlsx_reader::CellRange,
    /// The cells beside the anchor, row by row, with their values.
    pub cells: Vec<ComputedCell>,
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
    evaluate_with_dependency_setting(workbook, time_zone, manual, false)
}
/// Evaluate normally and retain actual cell/range reads for refresh scheduling.
pub fn evaluate_with_value_dependencies(
    workbook: &Workbook,
    time_zone: &str,
    manual: &std::collections::BTreeMap<String, HashSet<u32>>,
) -> Result<Recalculated> {
    evaluate_with_dependency_setting(workbook, time_zone, manual, true)
}
fn evaluate_with_dependency_setting(
    workbook: &Workbook,
    time_zone: &str,
    manual: &std::collections::BTreeMap<String, HashSet<u32>>,
    trace: bool,
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
            // The furigana saved with text cells, for PHONETIC.
            let readings = workbook
                .sheets
                .iter()
                .enumerate()
                .flat_map(|(index, sheet)| {
                    sheet.phonetic_readings.iter().map(move |(at, reading)| {
                        (
                            (index as u32, at.row as i32 + 1, at.column as i32 + 1),
                            reading.clone(),
                        )
                    })
                })
                .collect();
            ironcalc_base::phonetic::with_readings(readings, || {
                evaluate_at_instant(workbook, time_zone, trace)
            })
        })
    })
}

fn evaluate_at_instant(workbook: &Workbook, time_zone: &str, trace: bool) -> Result<Recalculated> {
    // Each skipped cell and what it holds: its cached value, or the engine's
    // answer when nothing is cached.
    let mut skipped: HashMap<(usize, CellRef), CellValue> = HashMap::new();
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
        let (mut model, projected, unparsed) = project(workbook, &skipped, time_zone, trace)?;
        let round = read_back(&model, workbook, &projected)?;

        let Some(first) = round.disqualified.first().cloned() else {
            let mut cells = round.cells;
            // A skipped cell with nothing cached shows the engine's answer.
            for cell in &reasons {
                let value = &skipped[&(cell.sheet, cell.reference)];
                if cached_value(workbook, cell.sheet, cell.reference) != *value {
                    cells.push(ComputedCell {
                        sheet: cell.sheet,
                        reference: cell.reference,
                        value: value.clone(),
                    });
                }
            }
            reasons.extend(unparsed);
            return Ok(Recalculated {
                cells,
                skipped: reasons,
                spills: round.spills,
                dependencies: trace.then(|| value_dependencies(&mut model)),
            });
        };
        let holds = match cached_value(workbook, first.sheet, first.reference) {
            CellValue::Empty => CellValue::Error(first.reason.clone()),
            cached => cached,
        };
        skipped.insert((first.sheet, first.reference), holds);
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
    skipped: &HashMap<(usize, CellRef), CellValue>,
    time_zone: &'a str,
    trace: bool,
) -> Result<Projection<'a>> {
    use ironcalc_base::date_system::{with_date_system, DateSystem as EngineDateSystem};
    let system = match workbook.date_system {
        DateSystem::Nineteen00 => EngineDateSystem::Excel1900,
        DateSystem::Nineteen04 => EngineDateSystem::Excel1904,
    };
    with_date_system(system, || {
        project_with_epoch(workbook, skipped, time_zone, trace)
    })
}

fn project_with_epoch<'a>(
    workbook: &Workbook,
    skipped: &HashMap<(usize, CellRef), CellValue>,
    time_zone: &'a str,
    trace: bool,
) -> Result<Projection<'a>> {
    let mut model = project_structure(workbook, time_zone)?;
    let mut projected = Vec::new();
    let mut unparsed = Vec::new();
    for (index, sheet) in workbook.sheets.iter().enumerate() {
        let engine_sheet = index as u32;
        for row in &sheet.rows {
            let engine_row = row.index as i32 + 1;
            for cell in &row.cells {
                let engine_column = cell.reference.column as i32 + 1;
                // An array formula's output, stored beside its anchor: the
                // engine spills it again, and a literal there would block
                // the spill (`#SPILL!`).
                if cell.formula.is_none()
                    && sheet
                        .array_formulas
                        .iter()
                        .any(|array| array.outputs_to(cell.reference))
                {
                    continue;
                }
                let held = skipped.get(&(index, cell.reference));
                if let (Some(formula), None) = (&cell.formula, held) {
                    if model
                        .update_cell_with_formula(
                            engine_sheet,
                            engine_row,
                            engine_column,
                            // A reference into a deleted sheet reads `#REF!B4`
                            // in the file; Excel evaluates it as `#REF!`.
                            format!(
                                "={}",
                                jade_xlsx_reader::collapse_ref_errors(&table_formula(
                                    workbook,
                                    index,
                                    Some(cell.reference),
                                    formula
                                ))
                            ),
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
                let value = held.unwrap_or(&cell.value);
                set_literal(&mut model, engine_sheet, engine_row, engine_column, value);
            }
        }
    }
    if trace {
        model.trace_value_reads(1_000_000);
    }
    model.evaluate();
    Ok((model, projected, unparsed))
}

/// An engine model with the workbook's sheets, tables and names, and no
/// cells yet.
fn project_structure<'a>(workbook: &Workbook, time_zone: &'a str) -> Result<Model<'a>> {
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

    model.set_pivot_regions(pivot::regions(workbook));

    // Tables before formulas, so structured references (`Table1[Amount]`)
    // parse.
    for sheet in &workbook.sheets {
        for table in &sheet.tables {
            if let Some(table) = engine_table(&sheet.name, table) {
                model.add_table(table);
            }
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
    Ok(model)
}

/// A table as the engine holds it: its range in A1 form and its columns.
fn engine_table(
    sheet: &str,
    table: &jade_xlsx_reader::Table,
) -> Option<ironcalc_base::types::Table> {
    use ironcalc_base::expressions::utils::number_to_column;
    let corner = |reference: CellRef| {
        Some(format!(
            "{}{}",
            number_to_column(reference.column as i32 + 1)?,
            reference.row + 1
        ))
    };
    Some(ironcalc_base::types::Table {
        name: table.name.clone(),
        display_name: table.display_name.clone(),
        sheet_name: sheet.to_string(),
        reference: format!(
            "{}:{}",
            corner(table.reference.start)?,
            corner(table.reference.end)?
        ),
        totals_row_count: table.totals_row_count,
        header_row_count: table.header_row_count,
        header_row_dxf_id: None,
        data_dxf_id: None,
        totals_row_dxf_id: None,
        columns: table
            .columns
            .iter()
            .enumerate()
            .map(|(index, name)| ironcalc_base::types::TableColumn {
                id: index as u32 + 1,
                name: name.clone(),
                ..Default::default()
            })
            .collect(),
        style_info: Default::default(),
        has_filters: table.auto_filter,
    })
}

fn set_literal(model: &mut Model<'_>, sheet: u32, row: i32, column: i32, value: &CellValue) {
    // Projection failures here are deliberate no-ops: an unset cell reads as
    // empty, the conservative meaning for a value the engine cannot hold.
    let _ = match value {
        CellValue::Number(number) => model.update_cell_with_number(sheet, row, column, *number),
        CellValue::Text(text) => model.update_cell_with_text(sheet, row, column, text),
        CellValue::Bool(flag) => model.update_cell_with_bool(sheet, row, column, *flag),
        // The engine has no setter for an error; a formula that is only the
        // error literal holds it, so cells reading it see the error, as in
        // Excel, not an empty cell.
        CellValue::Error(error) => {
            model.update_cell_with_formula(sheet, row, column, format!("={error}"))
        }
        CellValue::Empty => Ok(()),
    };
}

struct Round {
    cells: Vec<ComputedCell>,
    disqualified: Vec<SkippedCell>,
    spills: Vec<Spill>,
}

fn read_back(
    model: &Model<'_>,
    workbook: &Workbook,
    projected: &[(usize, CellRef)],
) -> Result<Round> {
    let mut cells = Vec::new();
    let mut disqualified = Vec::new();
    let mut spills = Vec::new();
    for (sheet_index, reference) in projected {
        spills.extend(spill_of(model, *sheet_index, *reference)?);
        let value = model
            .get_cell_value_by_index(
                *sheet_index as u32,
                reference.row as i32 + 1,
                reference.column as i32 + 1,
            )
            .map_err(CalcError::Engine)?;
        let value = to_cell_value(value);
        // An artifact the cell's own formula produced overwrites nothing
        // Excel computed only when the cache holds the same answer and the
        // file is not waiting on a full calculation that would replace it.
        if is_engine_artifact(&value)
            && !genuine_name_error(model, workbook, *sheet_index, *reference)
            && artifact_starts_here(model, *sheet_index, *reference)
            && (cached_value(workbook, *sheet_index, *reference) != value
                || workbook.full_calculation_on_load)
        {
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
        spills,
    })
}

/// What an array formula at `anchor` spilled beside it, if it spilled.
fn spill_of(model: &Model<'_>, sheet: usize, anchor: CellRef) -> Result<Option<Spill>> {
    use ironcalc_base::types::Cell;
    let Ok(worksheet) = model.workbook.worksheet(sheet as u32) else {
        return Ok(None);
    };
    let (row, column) = (anchor.row as i32 + 1, anchor.column as i32 + 1);
    let Some(Cell::ArrayFormula {
        r: (width, height), ..
    }) = worksheet.cell(row, column)
    else {
        return Ok(None);
    };
    if *width <= 1 && *height <= 1 {
        return Ok(None);
    }
    let mut values = Vec::new();
    for down in 0..*height {
        for across in 0..*width {
            if (down, across) == (0, 0) {
                continue;
            }
            let value = model
                .get_cell_value_by_index(sheet as u32, row + down, column + across)
                .map_err(CalcError::Engine)?;
            values.push(ComputedCell {
                sheet,
                reference: CellRef::new(anchor.column + across as u32, anchor.row + down as u32),
                value: to_cell_value(value),
            });
        }
    }
    Ok(Some(Spill {
        sheet,
        anchor,
        range: jade_xlsx_reader::CellRange {
            start: anchor,
            end: CellRef::new(
                anchor.column + (*width as u32).saturating_sub(1),
                anchor.row + (*height as u32).saturating_sub(1),
            ),
        },
        cells: values,
    }))
}

fn cached_value(workbook: &Workbook, sheet: usize, reference: CellRef) -> CellValue {
    workbook.sheets[sheet]
        .cell_at(reference)
        .map(|cell| cell.value.clone())
        .unwrap_or(CellValue::Empty)
}

/// Whether an engine artifact in this cell comes from its own formula, not
/// from a cell it reads. One that only passes another cell's along is that
/// cell's to answer for: skipped there, this one evaluates clean next round;
/// shown there — a misspelt function just typed — Excel shows it here too.
/// An error the engine did not trace to a cell counts as this one's.
fn artifact_starts_here(model: &Model<'_>, sheet: usize, reference: CellRef) -> bool {
    use ironcalc_base::types::{Cell, FormulaValue};
    let Ok(worksheet) = model.workbook.worksheet(sheet as u32) else {
        return true;
    };
    let origin = match worksheet.cell(reference.row as i32 + 1, reference.column as i32 + 1) {
        Some(Cell::CellFormula {
            v: FormulaValue::Error { o, .. },
            ..
        })
        | Some(Cell::ArrayFormula {
            v: FormulaValue::Error { o, .. },
            ..
        }) => o,
        _ => return true,
    };
    let Some((origin_sheet, address)) = origin.rsplit_once('!') else {
        return true;
    };
    origin_sheet == worksheet.name
        && CellRef::parse(address).is_ok_and(|origin| origin == reference)
}

/// A removed/unknown name is a real #NAME?, while a known definition the
/// backend could not register must retain its cache like an unknown function.
/// Empty messages are explicit error literals, including error-valued names.
fn genuine_name_error(
    model: &Model<'_>,
    workbook: &Workbook,
    sheet: usize,
    reference: CellRef,
) -> bool {
    use ironcalc_base::{
        expressions::token::Error,
        types::{Cell, FormulaValue},
    };
    let Ok(worksheet) = model.workbook.worksheet(sheet as u32) else {
        return false;
    };
    let message = match worksheet.cell(reference.row as i32 + 1, reference.column as i32 + 1) {
        Some(Cell::CellFormula {
            v: FormulaValue::Error {
                ei: Error::NAME, m, ..
            },
            ..
        })
        | Some(Cell::ArrayFormula {
            v: FormulaValue::Error {
                ei: Error::NAME, m, ..
            },
            ..
        }) => m,
        _ => return false,
    };
    if message.is_empty() {
        return true;
    }
    let Some(name) = message
        .strip_prefix("Variable name \"")
        .and_then(|m| m.strip_suffix("\" not found."))
    else {
        return false;
    };
    !workbook.defined_names.iter().any(|n| {
        n.name.to_lowercase() == name.to_lowercase()
            && (n.local_sheet_index.is_none() || n.local_sheet_index == Some(sheet))
    })
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

/// Static structured references are projected to their current A1 ranges for
/// the backend. Saved formula text stays unchanged; @ keeps a relative row.
pub(crate) fn table_formula(
    workbook: &Workbook,
    sheet: usize,
    at: Option<CellRef>,
    formula: &str,
) -> String {
    let context = workbook
        .sheets
        .get(sheet)
        .and_then(|s| at.and_then(|at| s.table_at(at)));
    jade_xlsx_reader::rewrite_table_references(formula, |name, selector| {
        let name = name.or_else(|| context.map(|t| t.name.as_str()))?;
        let (source, table) = workbook.sheets.iter().find_map(|s| {
            s.tables
                .iter()
                .find(|t| {
                    t.name.to_lowercase() == name.to_lowercase()
                        || t.display_name.to_lowercase() == name.to_lowercase()
                })
                .map(|t| (s, t))
        })?;
        Some(jade_xlsx_reader::table_reference::to_a1(
            table,
            &source.name,
            selector,
            at,
        ))
    })
}
