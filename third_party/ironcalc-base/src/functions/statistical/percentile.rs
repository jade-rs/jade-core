use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    pub(crate) fn collect_sorted_values(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<f64>, CalcResult> {
        let raw = match self.evaluate_node_in_context(arg, cell) {
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Ranges are in different sheets".into(),
                    ));
                }
                let v = self.values_from_range(left, right)?;
                v.into_iter().flatten().collect::<Vec<_>>()
            }
            CalcResult::Array(arr) => match self.values_from_array(arr) {
                Ok(v) => v.into_iter().flatten().collect::<Vec<_>>(),
                Err(e) => return Err(CalcResult::new_error(e, cell, "Error in data array".into())),
            },
            CalcResult::Number(n) => vec![n],
            CalcResult::EmptyCell | CalcResult::EmptyArg => vec![],
            error @ CalcResult::Error { .. } => return Err(error),
            _ => {
                return Err(CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Expected numeric range".to_string(),
                ))
            }
        };

        let mut sorted = raw;
        if sorted.iter().any(|number| !number.is_finite()) {
            return Err(CalcResult::new_error(
                Error::NUM,
                cell,
                "Invalid data number".into(),
            ));
        }
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Ok(sorted)
    }

    pub(crate) fn fn_percentile_inc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentile_arguments(args, cell, false, false)
    }

    pub(crate) fn fn_percentile_exc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentile_arguments(args, cell, true, false)
    }

    pub(crate) fn percentile_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        exclusive: bool,
        quartile: bool,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let sorted = self.collect_sorted_values(&args[0], cell);
        if let Err(
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            },
        ) = &sorted
        {
            return error.clone();
        }
        self.map_scalar_values(&args[1], cell, |model, value| {
            let sorted = match &sorted {
                Ok(values) => values,
                Err(error) => return error.clone(),
            };
            let invalid =
                || CalcResult::new_error(Error::NUM, cell, "Percentile outside data bounds".into());
            if sorted.is_empty() {
                return invalid();
            }
            let mut k = match model.cast_to_number(value, cell) {
                Ok(number) if number.is_finite() => number,
                Ok(_) => return invalid(),
                Err(error) => return error,
            };
            if quartile {
                let quart = k.floor();
                let valid = if exclusive {
                    (1.0..=3.0).contains(&quart)
                } else {
                    (0.0..=4.0).contains(&quart)
                };
                if !valid {
                    return invalid();
                }
                k = quart / 4.0;
            }
            let result = if exclusive {
                let n = sorted.len() as f64;
                if k <= 0.0 || k >= 1.0 || k < 1.0 / (n + 1.0) || k > n / (n + 1.0) {
                    return invalid();
                }
                let rank = (k * (n + 1.0) - 1.0).clamp(0.0, n - 1.0);
                let low = (rank.floor() as usize).min(sorted.len() - 1);
                let high = (low + 1).min(sorted.len() - 1);
                let fraction = rank - rank.floor();
                interpolate_percentile(sorted[low], sorted[high], fraction)
            } else {
                if !(0.0..=1.0).contains(&k) {
                    return invalid();
                }
                percentile_inc_impl(sorted, k)
            };
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                invalid()
            }
        })
    }

    pub(crate) fn fn_percentrank_inc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentrank_arguments(args, cell, false)
    }

    pub(crate) fn fn_percentrank_exc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentrank_arguments(args, cell, true)
    }

    fn percentrank_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        exclusive: bool,
    ) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let sorted = self.collect_sorted_values(&args[0], cell);
        if let Err(
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            },
        ) = &sorted
        {
            return error.clone();
        }
        self.map_scalar_arguments(&args[1..], cell, |model, values| {
            let sorted = match &sorted {
                Ok(values) => values,
                Err(error) => return error.clone(),
            };
            if sorted.is_empty() {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Empty percentile-rank data".into(),
                );
            }
            if !exclusive && sorted.len() < 2 {
                return CalcResult::new_error(
                    Error::NA,
                    cell,
                    "Insufficient percentile-rank data".into(),
                );
            }
            let x = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let significance = if values.len() == 2 {
                match model.cast_to_number(values[1].clone(), cell) {
                    Ok(number) => number.floor(),
                    Err(error) => return error,
                }
            } else {
                3.0
            };
            if !x.is_finite() || !significance.is_finite() || significance < 1.0 {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid percentile-rank arguments".into(),
                );
            }
            if x < sorted[0] || x > sorted[sorted.len() - 1] {
                return CalcResult::new_error(Error::NA, cell, "Value outside data range".into());
            }
            let rank = if exclusive {
                percentrank_exc_impl(sorted, x)
            } else {
                percentrank_inc_impl(sorted, x)
            };
            match crate::number_format::round_decimal(
                rank,
                significance,
                crate::number_format::DecimalRounding::TowardZero,
            ) {
                Some(number) => CalcResult::Number(number),
                None => {
                    CalcResult::new_error(Error::NUM, cell, "Percentile rank out of range".into())
                }
            }
        })
    }
}

/// PERCENTILE.INC interpolation: rank = k*(n-1) in 0-indexed sorted array.
pub(crate) fn percentile_inc_impl(sorted: &[f64], k: f64) -> f64 {
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let rank = k * (n - 1) as f64;
    let low = rank.floor() as usize;
    let high = (low + 1).min(n - 1);
    let frac = rank - rank.floor();
    interpolate_percentile(sorted[low], sorted[high], frac)
}

fn interpolate_percentile(low: f64, high: f64, fraction: f64) -> f64 {
    if fraction == 0.0 || low == high {
        return low;
    }
    if fraction == 1.0 {
        return high;
    }
    if low.is_sign_negative() != high.is_sign_negative() {
        // Opposite-sign endpoints can have an infinite difference even when
        // every interpolated value is finite and within the endpoints.
        low * (1.0 - fraction) + high * fraction
    } else {
        low + fraction * (high - low)
    }
}

/// PERCENTRANK.INC: virtual position in sorted array, divided by (n-1).
pub(crate) fn percentrank_inc_impl(sorted: &[f64], x: f64) -> f64 {
    let n = sorted.len();
    // Number of values strictly less than x
    let k = sorted.partition_point(|&v| v < x);

    if k == n || (k < n && sorted[k] == x) {
        // x exactly matches sorted[k] or x >= all values
        return k as f64 / (n - 1) as f64;
    }

    // x is between sorted[k-1] and sorted[k]
    let frac = percentile_fraction(sorted[k - 1], sorted[k], x);
    (k as f64 - 1.0 + frac) / (n - 1) as f64
}

/// PERCENTRANK.EXC: rank = (virtual_position + 1) / (n + 1).
pub(crate) fn percentrank_exc_impl(sorted: &[f64], x: f64) -> f64 {
    let n = sorted.len();
    let k = sorted.partition_point(|&v| v < x);

    let virtual_pos = if k < n && sorted[k] == x {
        k as f64
    } else if k > 0 && k < n {
        let frac = percentile_fraction(sorted[k - 1], sorted[k], x);
        k as f64 - 1.0 + frac
    } else {
        k as f64
    };

    (virtual_pos + 1.0) / (n + 1) as f64
}

fn percentile_fraction(low: f64, high: f64, value: f64) -> f64 {
    if low.is_sign_negative() != high.is_sign_negative() {
        let scale = low.abs().max(high.abs());
        (value / scale - low / scale) / (high / scale - low / scale)
    } else {
        (value - low) / (high - low)
    }
}
