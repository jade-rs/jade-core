use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, token::Error, types::CellReferenceIndex},
    model::Model,
    number_format::to_precision,
};

impl<'a> Model<'a> {
    pub(crate) fn fn_delta(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.engineering_comparison(args, cell, false)
    }

    pub(crate) fn fn_gestep(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.engineering_comparison(args, cell, true)
    }

    fn engineering_comparison(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        step: bool,
    ) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0; 2];
            for (number, value) in numbers.iter_mut().zip(values) {
                if matches!(value, CalcResult::Boolean(_)) {
                    return CalcResult::new_error(Error::VALUE, cell, "Expecting number".into());
                }
                *number = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
            }
            if numbers.iter().any(|number| !number.is_finite()) {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid comparison parameter".into(),
                );
            }
            // Retain the established comparison rounding for both functions.
            let left = to_precision(numbers[0], 16);
            let right = to_precision(numbers[1], 16);
            let matches = if step { left >= right } else { left == right };
            CalcResult::Number(if matches { 1.0 } else { 0.0 })
        })
    }
}
