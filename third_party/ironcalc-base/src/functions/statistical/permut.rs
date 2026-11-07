use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    pub(crate) fn fn_permut(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.permutation_arguments(args, cell, false)
    }

    pub(crate) fn fn_permutationa(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.permutation_arguments(args, cell, true)
    }

    fn permutation_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        repetitions: bool,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let n = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number.floor(),
                Err(error) => return error,
            };
            let k = match model.cast_to_number(values[1].clone(), cell) {
                Ok(number) => number.floor(),
                Err(error) => return error,
            };
            let invalid = || {
                CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid or overflowing permutation".into(),
                )
            };
            if !n.is_finite()
                || !k.is_finite()
                || n < 0.0
                || k < 0.0
                || (!repetitions && (n == 0.0 || k > n))
                || (repetitions && n == 0.0 && k > 0.0)
            {
                return invalid();
            }
            let result = if repetitions {
                n.powf(k)
            } else {
                // n!/(n-k)! >= k!, and 171! exceeds the finite numeric range.
                if k > 170.0 {
                    return invalid();
                }
                let mut result = 1.0;
                let mut exact = if n <= 9_007_199_254_740_992.0 {
                    Some(1_u128)
                } else {
                    None
                };
                for index in 0..k as usize {
                    let next =
                        exact.and_then(|previous| previous.checked_mul(n as u128 - index as u128));
                    result = if let Some(integer) = next {
                        integer as f64
                    } else {
                        result * (n - index as f64)
                    };
                    exact = next;
                    if !result.is_finite() {
                        return invalid();
                    }
                }
                result
            };
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                invalid()
            }
        })
    }
}
