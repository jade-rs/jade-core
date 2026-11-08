use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    // PHI(x) = standard normal PDF at x
    pub(crate) fn fn_phi(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }

        self.map_scalar_values(&args[0], cell, |model, value| {
            if matches!(value, CalcResult::Boolean(_)) {
                return CalcResult::new_error(Error::VALUE, cell, "Expecting number".into());
            }
            let x = match model.cast_to_number(value, cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !x.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, "Invalid PHI parameter".into());
            }
            // Retain the standard normal density expression and tail behavior.
            let result = (-(x * x) / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
            CalcResult::Number(result)
        })
    }
}
