use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    pub(crate) fn fn_standardize(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        // STANDARDIZE(x, mean, standard_dev)
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }

        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0; 3];
            for (number, value) in numbers.iter_mut().zip(values) {
                // Retain STANDARDIZE's existing no-Boolean numeric coercion.
                if matches!(value, CalcResult::Boolean(_)) {
                    return CalcResult::new_error(Error::VALUE, cell, "Expecting number".into());
                }
                *number = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
            }
            let [x, mean, std_dev] = numbers;
            if numbers.iter().any(|number| !number.is_finite()) || std_dev <= 0.0 {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid STANDARDIZE parameters".into(),
                );
            }
            let result = (x - mean) / std_dev;
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Invalid STANDARDIZE result".into())
            }
        })
    }
}
