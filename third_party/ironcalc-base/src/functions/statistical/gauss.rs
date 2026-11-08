use statrs::distribution::{ContinuousCDF, Normal};

use crate::expressions::token::Error;
use crate::expressions::types::CellReferenceIndex;
use crate::{calc_result::CalcResult, expressions::parser::Node, model::Model};

impl<'a> Model<'a> {
    pub(crate) fn fn_gauss(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let dist = match Normal::new(0.0, 1.0) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::ERROR,
                    origin: cell,
                    message: "Failed to construct standard normal distribution".to_string(),
                }
            }
        };

        self.map_scalar_values(&args[0], cell, |model, value| {
            let z = match model.cast_to_number(value, cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !z.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, "Invalid GAUSS parameter".into());
            }
            let result = dist.cdf(z) - 0.5;
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Invalid GAUSS result".into())
            }
        })
    }
}
