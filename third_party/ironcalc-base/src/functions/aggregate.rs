//! AGGREGATE (Jade addition): one of nineteen aggregates over references or
//! an array, leaving out what its `options` say — nested SUBTOTAL and
//! AGGREGATE results, hidden rows, error values.
//!
//! ```text
//! AGGREGATE(function_num, options, ref1, [ref2], …)   function_num 1–13
//! AGGREGATE(function_num, options, array, k)          function_num 14–19
//! ```

use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, token::Error, types::CellReferenceIndex},
    functions::Function,
    model::Model,
};

/// What `options` leaves out.
#[derive(Clone, Copy)]
struct Skip {
    nested: bool,
    hidden: bool,
    errors: bool,
}

impl Skip {
    fn of(options: i32) -> Option<Skip> {
        let (nested, hidden, errors) = match options {
            0 => (true, false, false),
            1 => (true, true, false),
            2 => (true, false, true),
            3 => (true, true, true),
            4 => (false, false, false),
            5 => (false, true, false),
            6 => (false, false, true),
            7 => (false, true, true),
            _ => return None,
        };
        Some(Skip {
            nested,
            hidden,
            errors,
        })
    }
}

/// The values gathered: numbers, and how many other non-empty values there
/// were (which only COUNTA counts).
#[derive(Default)]
struct Gathered {
    numbers: Vec<f64>,
    others: usize,
}

fn is_nested_aggregate(node: &Node) -> bool {
    matches!(
        node,
        Node::FunctionKind {
            kind: Function::Subtotal | Function::Aggregate,
            ..
        }
    )
}

impl<'a> Model<'a> {
    /// Whether the cell's formula is a SUBTOTAL or an AGGREGATE.
    fn cell_is_aggregate(&self, sheet: u32, row: i32, column: i32) -> bool {
        let Some(cell) = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .and_then(|worksheet| worksheet.sheet_data.get(&row))
            .and_then(|row| row.get(&column))
        else {
            return false;
        };
        cell.get_formula().is_some_and(|formula| {
            self.parsed_formulas
                .get(sheet as usize)
                .and_then(|formulas| formulas.get(formula as usize))
                .is_some_and(|parsed| is_nested_aggregate(&parsed.0))
        })
    }

    /// Adds one value to `gathered`, or fails with it when it is an error
    /// the options do not skip.
    fn aggregate_take(
        value: CalcResult,
        skip: Skip,
        gathered: &mut Gathered,
    ) -> Result<(), CalcResult> {
        match value {
            CalcResult::Number(number) => gathered.numbers.push(number),
            CalcResult::String(_) | CalcResult::Boolean(_) => gathered.others += 1,
            error @ CalcResult::Error { .. } => {
                if !skip.errors {
                    return Err(error);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn aggregate_gather(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        skip: Skip,
    ) -> Result<Gathered, CalcResult> {
        let mut gathered = Gathered::default();
        for arg in args {
            if skip.nested && is_nested_aggregate(arg) {
                continue;
            }
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        ));
                    }
                    for row in left.row..=right.row {
                        if skip.hidden
                            && self.is_row_hidden(left.sheet, row).map_err(|message| {
                                CalcResult::new_error(Error::ERROR, cell, message)
                            })?
                        {
                            continue;
                        }
                        for column in left.column..=right.column {
                            if skip.nested && self.cell_is_aggregate(left.sheet, row, column) {
                                continue;
                            }
                            let value = self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            });
                            Self::aggregate_take(value, skip, &mut gathered)?;
                        }
                    }
                }
                CalcResult::Array(rows) => {
                    for value in rows.into_iter().flatten() {
                        let value = match value {
                            crate::expressions::parser::ArrayNode::Number(number) => {
                                CalcResult::Number(number)
                            }
                            crate::expressions::parser::ArrayNode::Boolean(value) => {
                                CalcResult::Boolean(value)
                            }
                            crate::expressions::parser::ArrayNode::String(text) => {
                                CalcResult::String(text)
                            }
                            crate::expressions::parser::ArrayNode::Error(error) => {
                                CalcResult::new_error(error, cell, String::new())
                            }
                            _ => continue,
                        };
                        Self::aggregate_take(value, skip, &mut gathered)?;
                    }
                }
                CalcResult::Lambda(_) => {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "A function is not a value".to_string(),
                    ))
                }
                value => Self::aggregate_take(value, skip, &mut gathered)?,
            }
        }
        Ok(gathered)
    }

    pub(crate) fn fn_aggregate(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() < 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let function = match self.get_number(&args[0], cell) {
            Ok(value) => value.trunc() as i32,
            Err(error) => return error,
        };
        let Some(skip) = (match self.get_number(&args[1], cell) {
            Ok(value) => Skip::of(value.trunc() as i32),
            Err(error) => return error,
        }) else {
            return CalcResult::new_error(Error::VALUE, cell, "Invalid AGGREGATE options".into());
        };
        let invalid =
            || CalcResult::new_error(Error::VALUE, cell, "Invalid AGGREGATE function".into());
        let div0 = || CalcResult::new_error(Error::DIV, cell, "Division by 0!".to_string());
        let num = || CalcResult::new_error(Error::NUM, cell, "Out of range".to_string());

        if (14..=19).contains(&function) {
            if args.len() != 4 {
                return CalcResult::new_args_number_error(cell);
            }
            let k = match self.get_number(&args[3], cell) {
                Ok(k) => k,
                Err(error) => return error,
            };
            let mut values = match self.aggregate_gather(&args[2..3], cell, skip) {
                Ok(gathered) => gathered.numbers,
                Err(error) => return error,
            };
            values.sort_by(|a, b| a.total_cmp(b));
            let result = match function {
                14 | 15 => {
                    let k = k.trunc();
                    if k < 1.0 || k as usize > values.len() {
                        return num();
                    }
                    let index = k as usize - 1;
                    if function == 14 {
                        values[values.len() - 1 - index]
                    } else {
                        values[index]
                    }
                }
                16 => match percentile_inc(&values, k) {
                    Some(value) => value,
                    None => return num(),
                },
                17 => {
                    let quart = k.trunc();
                    if !(0.0..=4.0).contains(&quart) {
                        return num();
                    }
                    match percentile_inc(&values, quart / 4.0) {
                        Some(value) => value,
                        None => return num(),
                    }
                }
                18 => match percentile_exc(&values, k) {
                    Some(value) => value,
                    None => return num(),
                },
                _ => {
                    let quart = k.trunc();
                    if !(1.0..=3.0).contains(&quart) {
                        return num();
                    }
                    match percentile_exc(&values, quart / 4.0) {
                        Some(value) => value,
                        None => return num(),
                    }
                }
            };
            return CalcResult::Number(result);
        }
        if !(1..=13).contains(&function) {
            return invalid();
        }

        let gathered = match self.aggregate_gather(&args[2..], cell, skip) {
            Ok(gathered) => gathered,
            Err(error) => return error,
        };
        let values = gathered.numbers;
        let count = values.len() as f64;
        let sum: f64 = values.iter().sum();
        let variance = |sample: bool| -> Option<f64> {
            let divisor = if sample { count - 1.0 } else { count };
            if divisor <= 0.0 {
                return None;
            }
            let mean = sum / count;
            Some(
                values
                    .iter()
                    .map(|value| (value - mean).powi(2))
                    .sum::<f64>()
                    / divisor,
            )
        };
        let result = match function {
            1 => {
                if values.is_empty() {
                    return div0();
                }
                sum / count
            }
            2 => count,
            3 => count + gathered.others as f64,
            4 => values.iter().copied().reduce(f64::max).unwrap_or(0.0),
            5 => values.iter().copied().reduce(f64::min).unwrap_or(0.0),
            6 => {
                if values.is_empty() {
                    0.0
                } else {
                    values.iter().product()
                }
            }
            7 => match variance(true) {
                Some(variance) => variance.sqrt(),
                None => return div0(),
            },
            8 => match variance(false) {
                Some(variance) => variance.sqrt(),
                None => return div0(),
            },
            9 => sum,
            10 => match variance(true) {
                Some(variance) => variance,
                None => return div0(),
            },
            11 => match variance(false) {
                Some(variance) => variance,
                None => return div0(),
            },
            12 => {
                if values.is_empty() {
                    return num();
                }
                let mut sorted = values.clone();
                sorted.sort_by(|a, b| a.total_cmp(b));
                let middle = sorted.len() / 2;
                if sorted.len() % 2 == 0 {
                    (sorted[middle - 1] + sorted[middle]) / 2.0
                } else {
                    sorted[middle]
                }
            }
            _ => {
                // MODE.SNGL: the most frequent value, the first to reach that
                // count when several tie; none repeated is #N/A.
                let mut best: Option<(f64, usize)> = None;
                for value in &values {
                    let frequency = values.iter().filter(|other| *other == value).count();
                    if frequency > 1 && best.is_none_or(|(_, most)| frequency > most) {
                        best = Some((*value, frequency));
                    }
                }
                match best {
                    Some((value, _)) => value,
                    None => {
                        return CalcResult::new_error(Error::NA, cell, "No repeated value".into())
                    }
                }
            }
        };
        CalcResult::Number(result)
    }
}

/// PERCENTILE.INC over sorted `values`.
fn percentile_inc(values: &[f64], k: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=1.0).contains(&k) {
        return None;
    }
    let position = k * (values.len() - 1) as f64;
    let below = position.floor() as usize;
    let above = position.ceil() as usize;
    Some(values[below] + (position - below as f64) * (values[above] - values[below]))
}

/// PERCENTILE.EXC over sorted `values`.
fn percentile_exc(values: &[f64], k: f64) -> Option<f64> {
    let n = values.len() as f64;
    let rank = k * (n + 1.0);
    if values.is_empty() || k <= 0.0 || k >= 1.0 || rank < 1.0 || rank > n {
        return None;
    }
    let below = rank.floor() as usize;
    let above = rank.ceil() as usize;
    Some(values[below - 1] + (rank - below as f64) * (values[above - 1] - values[below - 1]))
}
