//! Input validation against current workbook values, with the candidate value
//! installed before a custom predicate, relative bound or list is evaluated.
use crate::{conditional::project_values, set_literal, Result};
use ironcalc_base::{
    date_system::{with_date_system, DateSystem as EngineDateSystem},
    AnchoredValue, Model,
};
use jade_xlsx_reader::{CellRef, CellValue, DataValidation, DateSystem, Workbook};

#[derive(Debug, Clone, PartialEq)]
pub enum Candidate {
    Value(CellValue),
    Formula(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationResult {
    Valid,
    Invalid,
    Unsupported,
}

pub fn validate(
    workbook: &Workbook,
    sheet: usize,
    at: CellRef,
    rule: &DataValidation,
    candidate: &Candidate,
    time_zone: &str,
) -> Result<ValidationResult> {
    if !matches!(
        rule.validation_type.as_deref().unwrap_or("none"),
        "none" | "whole" | "decimal" | "list" | "date" | "time" | "textLength" | "custom"
    ) {
        return Ok(ValidationResult::Unsupported);
    }
    if rule.validation_type.as_deref().unwrap_or("none") == "none" {
        return Ok(ValidationResult::Valid);
    }
    if rule.allow_blank && matches!(candidate, Candidate::Value(CellValue::Empty)) {
        return Ok(ValidationResult::Valid);
    }
    if sheet >= workbook.sheets.len() || !rule.ranges.iter().any(|r| r.contains(at)) {
        return Ok(ValidationResult::Unsupported);
    }
    let epoch = match workbook.date_system {
        DateSystem::Nineteen00 => EngineDateSystem::Excel1900,
        DateSystem::Nineteen04 => EngineDateSystem::Excel1904,
    };
    with_date_system(epoch, || {
        let mut model = project_values(workbook, time_zone)?;
        let normalized;
        let candidate = if let Candidate::Formula(formula) = candidate {
            normalized =
                Candidate::Formula(crate::table_formula(workbook, sheet, Some(at), formula));
            &normalized
        } else {
            candidate
        };
        let value = install(&mut model, sheet, at, candidate)?;
        if rule.allow_blank && value == AnchoredValue::Empty {
            return Ok(ValidationResult::Valid);
        }
        let kind = rule.validation_type.as_deref().unwrap_or("none");
        if kind == "list" {
            let Some(items) = list_values(&mut model, sheet, at, rule)? else {
                return Ok(ValidationResult::Unsupported);
            };
            return Ok(if items.iter().any(|item| same_value(item, &value)) {
                ValidationResult::Valid
            } else {
                ValidationResult::Invalid
            });
        }
        if matches!(kind, "whole" | "decimal" | "date" | "time")
            && !matches!(value, AnchoredValue::Number(_))
        {
            return Ok(ValidationResult::Invalid);
        }
        if kind == "whole" && matches!(value,AnchoredValue::Number(n) if n.fract()!=0.0) {
            return Ok(ValidationResult::Invalid);
        }
        let Some(first) = rule.formula1.as_deref() else {
            return Ok(ValidationResult::Unsupported);
        };
        let at_name = a1(at);
        let operand = if kind == "textLength" {
            format!("LEN({at_name})")
        } else {
            at_name
        };
        let Some(anchor) = rule.ranges.first().map(|r| r.start) else {
            return Ok(ValidationResult::Unsupported);
        };
        let predicate = if kind == "custom" {
            jade_xlsx_reader::translate_formula(
                first.strip_prefix('=').unwrap_or(first),
                anchor,
                at,
            )
        } else {
            let first = jade_xlsx_reader::translate_formula(first, anchor, at);
            let second = rule
                .formula2
                .as_deref()
                .map(|f| jade_xlsx_reader::translate_formula(f, anchor, at));
            let Some(predicate) = comparison(
                &operand,
                rule.operator.as_deref().unwrap_or("between"),
                &first,
                second.as_deref(),
            ) else {
                return Ok(ValidationResult::Unsupported);
            };
            predicate
        };
        let Some(parsed) = model.parse_anchored_formula(
            &crate::table_formula(workbook, sheet, Some(at), &predicate),
            sheet as u32,
            at.row as i32 + 1,
            at.column as i32 + 1,
        ) else {
            return Ok(ValidationResult::Unsupported);
        };
        let result = model.anchored_formula_value(
            &parsed,
            sheet as u32,
            at.row as i32 + 1,
            at.column as i32 + 1,
        );
        Ok(match result {
            AnchoredValue::Boolean(true) => ValidationResult::Valid,
            AnchoredValue::Number(n) if n != 0.0 && n.is_finite() => ValidationResult::Valid,
            AnchoredValue::Error(ref e) if e == "#NAME?" || e == "#ERROR!" => {
                ValidationResult::Unsupported
            }
            _ => ValidationResult::Invalid,
        })
    })
}

fn comparison(operand: &str, operator: &str, first: &str, second: Option<&str>) -> Option<String> {
    let f = |s: &str| format!("({})", s.strip_prefix('=').unwrap_or(s));
    let one = f(first);
    Some(match operator {
        "equal" => format!("{operand}={one}"),
        "notEqual" => format!("{operand}<>{one}"),
        "greaterThan" => format!("{operand}>{one}"),
        "greaterThanOrEqual" => format!("{operand}>={one}"),
        "lessThan" => format!("{operand}<{one}"),
        "lessThanOrEqual" => format!("{operand}<={one}"),
        op @ ("between" | "notBetween") => {
            let both = format!("AND({operand}>={one},{operand}<={})", f(second?));
            if op == "notBetween" {
                format!("NOT({both})")
            } else {
                both
            }
        }
        _ => return None,
    })
}

fn install(
    model: &mut Model<'_>,
    sheet: usize,
    at: CellRef,
    candidate: &Candidate,
) -> Result<AnchoredValue> {
    match candidate {
        Candidate::Value(value) => set_literal(
            model,
            sheet as u32,
            at.row as i32 + 1,
            at.column as i32 + 1,
            value,
        ),
        Candidate::Formula(formula) => {
            model
                .update_cell_with_formula(
                    sheet as u32,
                    at.row as i32 + 1,
                    at.column as i32 + 1,
                    format!("={}", formula.strip_prefix('=').unwrap_or(formula)),
                )
                .map_err(crate::CalcError::Engine)?;
            model.evaluate();
        }
    }
    model
        .get_cell_value_by_index(sheet as u32, at.row as i32 + 1, at.column as i32 + 1)
        .map(|v| from_cell(crate::to_cell_value(v)))
        .map_err(crate::CalcError::Engine)
}
fn from_cell(v: CellValue) -> AnchoredValue {
    match v {
        CellValue::Number(n) => AnchoredValue::Number(n),
        CellValue::Text(s) => AnchoredValue::Text(s),
        CellValue::Bool(b) => AnchoredValue::Boolean(b),
        CellValue::Error(e) => AnchoredValue::Error(e),
        CellValue::Empty => AnchoredValue::Empty,
    }
}
fn same_value(a: &AnchoredValue, b: &AnchoredValue) -> bool {
    match (a, b) {
        (AnchoredValue::Text(a), AnchoredValue::Text(b)) => a.to_lowercase() == b.to_lowercase(),
        (AnchoredValue::Text(a), AnchoredValue::Number(b)) => {
            a.parse::<f64>().is_ok_and(|a| a == *b)
        }
        _ => a == b,
    }
}
fn a1(at: CellRef) -> String {
    let mut column = at.column + 1;
    let mut letters = Vec::new();
    while column > 0 {
        letters.push(b'A' + ((column - 1) % 26) as u8);
        column = (column - 1) / 26;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8(letters).unwrap(), at.row + 1)
}

/// List source values, including names and dynamic range-producing formulas.
/// Large or unsupported sources are explicit None, never an empty allowed list.
pub fn list(
    workbook: &Workbook,
    sheet: usize,
    at: CellRef,
    rule: &DataValidation,
    time_zone: &str,
) -> Result<Option<Vec<CellValue>>> {
    let epoch = match workbook.date_system {
        DateSystem::Nineteen00 => EngineDateSystem::Excel1900,
        DateSystem::Nineteen04 => EngineDateSystem::Excel1904,
    };
    with_date_system(epoch, || {
        let mut model = project_values(workbook, time_zone)?;
        Ok(list_values(&mut model, sheet, at, rule)?.map(|v| {
            v.into_iter()
                .map(|v| match v {
                    AnchoredValue::Number(n) => CellValue::Number(n),
                    AnchoredValue::Text(s) => CellValue::Text(s),
                    AnchoredValue::Boolean(b) => CellValue::Bool(b),
                    AnchoredValue::Error(e) => CellValue::Error(e),
                    _ => CellValue::Empty,
                })
                .collect()
        }))
    })
}
fn list_values(
    model: &mut Model<'_>,
    sheet: usize,
    at: CellRef,
    rule: &DataValidation,
) -> Result<Option<Vec<AnchoredValue>>> {
    if let Some(literal) = &rule.literal_list {
        return Ok(
            jade_xlsx_reader::data_validations::parse_literal_list(literal)
                .map(|items| items.into_iter().map(AnchoredValue::Text).collect()),
        );
    }
    let Some(source) = rule.formula1.as_deref().map(str::trim) else {
        return Ok(None);
    };
    let source = source.strip_prefix('=').unwrap_or(source);
    if let Some(text) = source.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Ok(Some(
            text.replace("\"\"", "\"")
                .split(',')
                .map(|s| AnchoredValue::Text(s.into()))
                .collect(),
        ));
    }
    let Some(anchor) = rule.ranges.first().map(|r| r.start) else {
        return Ok(None);
    };
    let Some(formula) = model.parse_anchored_formula(
        source,
        sheet as u32,
        anchor.row as i32 + 1,
        anchor.column as i32 + 1,
    ) else {
        return Ok(None);
    };
    let result = model.anchored_formula_value(
        &formula,
        sheet as u32,
        at.row as i32 + 1,
        at.column as i32 + 1,
    );
    Ok(match result {
        AnchoredValue::Range { left, right }
            if left.sheet == right.sheet
                && right.row >= left.row
                && right.column >= left.column
                && left.row > 0
                && left.column > 0 =>
        {
            let count =
                i64::from(right.row - left.row + 1) * i64::from(right.column - left.column + 1);
            if count > 10000 {
                return Ok(None);
            }
            let mut items = Vec::new();
            for row in left.row..=right.row {
                for column in left.column..=right.column {
                    let v = model
                        .get_cell_value_by_index(left.sheet, row, column)
                        .map_err(crate::CalcError::Engine)?;
                    items.push(from_cell(crate::to_cell_value(v)));
                }
            }
            Some(items)
        }
        AnchoredValue::Array(rows) => {
            Some(rows.into_iter().flatten().take(10001).collect::<Vec<_>>())
                .filter(|v| v.len() <= 10000)
        }
        AnchoredValue::Text(s) => Some(vec![AnchoredValue::Text(s)]),
        _ => None,
    })
}
