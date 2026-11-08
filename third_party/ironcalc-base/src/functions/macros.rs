#[macro_export]
macro_rules! single_number_fn {
    ($fn_name:ident, $op:expr) => {
        pub(crate) fn $fn_name(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
            if args.len() != 1 {
                return CalcResult::new_args_number_error(cell);
            }
            self.apply_number_unary(&args[0], cell, |number| {
                if !number.is_finite() {
                    return Err(Error::NUM);
                }
                let result: Result<f64, Error> = ($op)(number);
                match result {
                    Ok(number) if number.is_finite() => Ok(number),
                    Ok(_) => Err(Error::NUM),
                    Err(error) => Err(error),
                }
            })
        }
    };
}
