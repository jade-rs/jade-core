use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    // Helper to collect numeric values from the 2nd argument of RANK.*
    fn collect_rank_values(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<f64>, CalcResult> {
        let values = match self.evaluate_node_in_context(arg, cell) {
            CalcResult::Array(array) => match self.values_from_array(array) {
                Ok(v) => v,
                Err(e) => {
                    return Err(CalcResult::Error {
                        error: e,
                        origin: cell,
                        message: "Error in rank data array".into(),
                    })
                }
            },
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Ranges are in different sheets".into(),
                    ));
                }
                self.values_from_range(left, right)?
            }
            CalcResult::Number(number) => vec![Some(number)],
            error @ CalcResult::Error { .. } => return Err(error),
            CalcResult::EmptyCell | CalcResult::EmptyArg => vec![],
            CalcResult::Boolean(value) => {
                if !matches!(arg, Node::ReferenceKind { .. }) {
                    vec![Some(if value { 1.0 } else { 0.0 })]
                } else {
                    return Err(CalcResult::Error {
                        error: Error::NUM,
                        origin: cell,
                        message: "Unsupported argument type".to_string(),
                    });
                }
            }
            _ => {
                return Err(CalcResult::Error {
                    error: Error::NIMPL,
                    origin: cell,
                    message: "Unsupported argument type".to_string(),
                })
            }
        };

        let mut numeric_values: Vec<f64> = values.into_iter().flatten().collect();
        if numeric_values.iter().any(|number| !number.is_finite()) {
            return Err(CalcResult::new_error(
                Error::NUM,
                cell,
                "Invalid rank data number".into(),
            ));
        }
        numeric_values.sort_by(f64::total_cmp);
        Ok(numeric_values)
    }

    pub(crate) fn fn_rank_eq(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.rank_arguments(args, cell, false)
    }

    pub(crate) fn fn_rank_avg(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.rank_arguments(args, cell, true)
    }

    fn rank_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        average: bool,
    ) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let data = self.collect_rank_values(&args[1], cell);
        if let Err(
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            },
        ) = &data
        {
            return error.clone();
        }
        let mut operands = vec![args[0].clone()];
        if let Some(order) = args.get(2) {
            operands.push(order.clone());
        }
        self.map_scalar_arguments(&operands, cell, |model, values| {
            let numeric = |model: &mut Self, value: &CalcResult| {
                if matches!(value, CalcResult::Boolean(_)) {
                    Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Expecting number".into(),
                    ))
                } else {
                    model.cast_to_number(value.clone(), cell)
                }
            };
            let number = match numeric(model, &values[0]) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !number.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, "Invalid rank number".into());
            }
            let data = match &data {
                Ok(data) => data,
                Err(error) => return error.clone(),
            };
            if data.is_empty() {
                return CalcResult::new_error(Error::NUM, cell, "Empty rank data".into());
            }
            let order = if values.len() == 2 {
                match numeric(model, &values[1]) {
                    Ok(number) => number,
                    Err(error) => return error,
                }
            } else {
                0.0
            };
            if !order.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, "Invalid rank order".into());
            }
            let lower = data.partition_point(|value| *value < number);
            let upper = data.partition_point(|value| *value <= number);
            let equal = upper - lower;
            if equal == 0 {
                return CalcResult::new_error(
                    Error::NA,
                    cell,
                    "Number not found in rank data".into(),
                );
            }
            let better = if order == 0.0 {
                data.len() - upper
            } else {
                lower
            };
            CalcResult::Number(
                better as f64
                    + if average {
                        (equal as f64 + 1.0) / 2.0
                    } else {
                        1.0
                    },
            )
        })
    }
}
