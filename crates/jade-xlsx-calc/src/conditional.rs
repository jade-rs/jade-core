//! Conditional formats whose rule is a formula.
//!
//! A `cfRule` of type `expression` holds a formula written for the top-left
//! cell of the rule's ranges; Excel evaluates it at every cell of the range
//! with its relative references moved along, and the cell takes the format
//! where the result is TRUE or a non-zero number. A `cellIs` rule whose
//! operand is a reference or a formula (`$B$1`, `B2*2`) is the same thing
//! with the comparison spelt out, and a `timePeriod` rule ("yesterday",
//! "last week") is too: Excel saves the formula it stands for, and one
//! without it gets the formula Excel would have written.
//!
//! The formulas are evaluated against the workbook's current values — every
//! cell projected as the value it shows — so a rule sees what is on screen.

use crate::{set_literal, CalcError, Result};
use ironcalc_base::Model;
use jade_xlsx_reader::{
    conditional_format::ConditionalRule, CellRange, CellRef, DateSystem, Workbook,
};

/// Cells a single rule is evaluated at, at most; a rule over whole columns
/// is clipped to the rows the sheet shows.
const MAX_CELLS_PER_RULE: u64 = 400_000;

/// Where one formula rule of a sheet holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormulaRuleMatches {
    /// Index into the sheet's `conditional_formats`.
    pub format: usize,
    /// Index into that format's `rules`.
    pub rule: usize,
    /// The cells evaluated: the rule's ranges clipped to the extent asked for.
    pub evaluated: Vec<CellRange>,
    /// The cells where the formula held, sorted.
    pub matched: Vec<CellRef>,
}

/// The formula a rule is decided by, relative to `anchor`, or `None` for a
/// rule decided some other way (or a `cellIs` with literal operands, which
/// the layout compares itself).
pub fn rule_formula(rule: &ConditionalRule, anchor: CellRef) -> Option<String> {
    if !rule.issues.is_empty() {
        return None;
    }
    let first = rule.formulas.first().map(|formula| strip(formula));
    match rule.rule_type.as_str() {
        "expression" => first
            .filter(|formula| !formula.is_empty())
            .map(str::to_string),
        "cellIs" => {
            if rule.formulas.iter().all(|formula| is_literal(formula)) {
                return None;
            }
            let at = a1(anchor);
            let one = format!("({})", first?);
            let two = || {
                rule.formulas
                    .get(1)
                    .map(|formula| format!("({})", strip(formula)))
            };
            Some(match rule.operator.as_deref()? {
                "equal" => format!("{at}={one}"),
                "notEqual" => format!("{at}<>{one}"),
                "greaterThan" => format!("{at}>{one}"),
                "greaterThanOrEqual" => format!("{at}>={one}"),
                "lessThan" => format!("{at}<{one}"),
                "lessThanOrEqual" => format!("{at}<={one}"),
                operator @ ("between" | "notBetween") => {
                    let two = two()?;
                    let between =
                        format!("OR(AND({at}>={one},{at}<={two}),AND({at}>={two},{at}<={one}))");
                    if operator == "between" {
                        between
                    } else {
                        format!("NOT({between})")
                    }
                }
                _ => return None,
            })
        }
        "timePeriod" => first
            .filter(|formula| !formula.is_empty())
            .map(str::to_string)
            .or_else(|| time_period_formula(rule.time_period.as_deref()?, anchor)),
        _ => None,
    }
}

/// The formula Excel writes for a "date occurring" rule.
pub fn time_period_formula(period: &str, anchor: CellRef) -> Option<String> {
    let at = a1(anchor);
    Some(match period {
        "today" => format!("FLOOR({at},1)=TODAY()"),
        "yesterday" => format!("FLOOR({at},1)=TODAY()-1"),
        "tomorrow" => format!("FLOOR({at},1)=TODAY()+1"),
        "last7Days" => format!("AND(TODAY()-FLOOR({at},1)<=6,FLOOR({at},1)<=TODAY())"),
        "thisWeek" => format!(
            "AND(TODAY()-ROUNDDOWN({at},0)<=WEEKDAY(TODAY())-1,ROUNDDOWN({at},0)-TODAY()<=7-WEEKDAY(TODAY()))"
        ),
        "lastWeek" => format!(
            "AND(TODAY()-ROUNDDOWN({at},0)>=(WEEKDAY(TODAY())),TODAY()-ROUNDDOWN({at},0)<(WEEKDAY(TODAY())+7))"
        ),
        "nextWeek" => format!(
            "AND(ROUNDDOWN({at},0)-TODAY()>(7-WEEKDAY(TODAY())),ROUNDDOWN({at},0)-TODAY()<(15-WEEKDAY(TODAY())))"
        ),
        "thisMonth" => format!("AND(MONTH({at})=MONTH(TODAY()),YEAR({at})=YEAR(TODAY()))"),
        "lastMonth" => format!(
            "AND(MONTH({at})=MONTH(EDATE(TODAY(),0-1)),YEAR({at})=YEAR(EDATE(TODAY(),0-1)))"
        ),
        "nextMonth" => format!(
            "AND(MONTH({at})=MONTH(EDATE(TODAY(),0+1)),YEAR({at})=YEAR(EDATE(TODAY(),0+1)))"
        ),
        _ => return None,
    })
}

/// The cell a rule's relative references are written against: the top-left
/// corner of all its ranges.
pub fn anchor(ranges: &[CellRange]) -> Option<CellRef> {
    let row = ranges.iter().map(|range| range.start.row).min()?;
    let column = ranges.iter().map(|range| range.start.column).min()?;
    Some(CellRef::new(column, row))
}

/// Whether the sheet has a rule only a formula can decide.
pub fn has_formula_rules(workbook: &Workbook, sheet: usize) -> bool {
    workbook.sheets.get(sheet).is_some_and(|sheet| {
        sheet.conditional_formats.iter().any(|format| {
            anchor(&format.ranges).is_some_and(|anchor| {
                format
                    .rules
                    .iter()
                    .any(|rule| rule_formula(rule, anchor).is_some())
            })
        })
    })
}

/// Evaluates every formula rule of `sheet` over its ranges, clipped to the
/// first `columns` × `rows` cells. Rules whose formula the engine cannot
/// parse are left out, so the caller treats them as unsupported.
pub fn conditional_formula_matches(
    workbook: &Workbook,
    sheet: usize,
    columns: u32,
    rows: u32,
    time_zone: &str,
) -> Result<Vec<FormulaRuleMatches>> {
    let Some(target) = workbook.sheets.get(sheet) else {
        return Ok(Vec::new());
    };
    let mut wanted = Vec::new();
    for (format_index, format) in target.conditional_formats.iter().enumerate() {
        let Some(anchor) = anchor(&format.ranges) else {
            continue;
        };
        for (rule_index, rule) in format.rules.iter().enumerate() {
            if let Some(formula) = rule_formula(rule, anchor) {
                wanted.push((format_index, rule_index, anchor, formula));
            }
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let mut model = project_values(workbook, time_zone)?;
    let engine_sheet = sheet as u32;
    let mut results = Vec::new();
    for (format_index, rule_index, anchor, formula) in wanted {
        let Some(parsed) = model.parse_anchored_formula(
            &jade_xlsx_reader::collapse_ref_errors(&formula),
            engine_sheet,
            anchor.row as i32 + 1,
            anchor.column as i32 + 1,
        ) else {
            continue;
        };
        let mut budget = MAX_CELLS_PER_RULE;
        let mut evaluated = Vec::new();
        let mut matched = Vec::new();
        for range in &target.conditional_formats[format_index].ranges {
            if columns == 0 || rows == 0 {
                break;
            }
            let last_column = range.end.column.min(columns - 1);
            let mut last_row = range.end.row.min(rows - 1);
            if range.start.column > last_column || range.start.row > last_row {
                continue;
            }
            let width = u64::from(last_column - range.start.column + 1);
            let height = u64::from(last_row - range.start.row + 1);
            if width * height > budget {
                let fit = (budget / width) as u32;
                if fit == 0 {
                    break;
                }
                last_row = range.start.row + fit - 1;
            }
            budget -= width * u64::from(last_row - range.start.row + 1);
            evaluated.push(CellRange {
                start: range.start,
                end: CellRef::new(last_column, last_row),
            });
            for row in range.start.row..=last_row {
                for column in range.start.column..=last_column {
                    if model.anchored_formula_holds(
                        &parsed,
                        engine_sheet,
                        row as i32 + 1,
                        column as i32 + 1,
                    ) {
                        matched.push(CellRef::new(column, row));
                    }
                }
            }
        }
        matched.sort_unstable();
        matched.dedup();
        results.push(FormulaRuleMatches {
            format: format_index,
            rule: rule_index,
            evaluated,
            matched,
        });
    }
    Ok(results)
}

/// The workbook as the engine holds it with every cell a literal: its
/// sheets, tables and names, and each cell's current value.
fn project_values<'a>(workbook: &Workbook, time_zone: &'a str) -> Result<Model<'a>> {
    use ironcalc_base::date_system::{with_date_system, DateSystem as EngineDateSystem};
    let system = match workbook.date_system {
        DateSystem::Nineteen00 => EngineDateSystem::Excel1900,
        DateSystem::Nineteen04 => EngineDateSystem::Excel1904,
    };
    with_date_system(system, || {
        let mut model = crate::project_structure(workbook, time_zone)
            .or_else(|_| crate::project_structure(workbook, "UTC"))?;
        for (index, sheet) in workbook.sheets.iter().enumerate() {
            for row in &sheet.rows {
                for cell in &row.cells {
                    set_literal(
                        &mut model,
                        index as u32,
                        row.index as i32 + 1,
                        cell.reference.column as i32 + 1,
                        &cell.value,
                    );
                }
            }
        }
        Ok::<_, CalcError>(model)
    })
}

fn strip(formula: &str) -> &str {
    let formula = formula.trim();
    formula.strip_prefix('=').unwrap_or(formula).trim()
}

/// A number, TRUE/FALSE or a quoted string: what the layout compares itself.
fn is_literal(formula: &str) -> bool {
    let formula = strip(formula);
    formula.parse::<f64>().is_ok_and(f64::is_finite)
        || formula.eq_ignore_ascii_case("TRUE")
        || formula.eq_ignore_ascii_case("FALSE")
        || (formula.len() >= 2
            && formula.starts_with('"')
            && formula.ends_with('"')
            && !formula[1..formula.len() - 1]
                .replace("\"\"", "")
                .contains('"'))
}

fn a1(reference: CellRef) -> String {
    let mut column = reference.column + 1;
    let mut letters = Vec::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        letters.push(b'A' + remainder as u8);
        column = (column - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        String::from_utf8(letters).unwrap_or_default(),
        reference.row + 1
    )
}
