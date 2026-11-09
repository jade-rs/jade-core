use statrs::function::erf::{erf, erfc};

use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, token::Error, types::CellReferenceIndex},
    model::Model,
};

use super::transcendental::{bessel_i, bessel_j, bessel_k, bessel_y};
// https://root.cern/doc/v610/TMath_8cxx_source.html

// Notice that the parameters for Bessel functions in Excel and here have inverted order
// EXCEL_BESSEL(x, n) => bessel(n, x)

impl<'a> Model<'a> {
    pub(crate) fn fn_besseli(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bessel_arguments(args, cell, bessel_i)
    }
    pub(crate) fn fn_besselj(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bessel_arguments(args, cell, bessel_j)
    }
    pub(crate) fn fn_besselk(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bessel_arguments(args, cell, bessel_k)
    }
    pub(crate) fn fn_bessely(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bessel_arguments(args, cell, bessel_y)
    }

    fn bessel_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        function: fn(i32, f64) -> f64,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let x = match model.cast_to_number_no_bools(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let order = match model.cast_to_number_no_bools(values[1].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !x.is_finite() || !order.is_finite() || order.trunc() < 0.0 {
                return CalcResult::new_error(Error::NUM, cell, "Invalid Bessel parameter".into());
            }
            // Retain the existing order truncation and numerical routines.
            let result = function(order.trunc() as i32, x);
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Invalid Bessel result".into())
            }
        })
    }

    pub(crate) fn fn_erf(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.error_function_arguments(args, cell, false, true)
    }

    pub(crate) fn fn_erfprecise(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.error_function_arguments(args, cell, false, false)
    }

    pub(crate) fn fn_erfc(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.error_function_arguments(args, cell, true, false)
    }

    pub(crate) fn fn_erfcprecise(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.error_function_arguments(args, cell, true, false)
    }

    fn error_function_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        complement: bool,
        allow_upper: bool,
    ) -> CalcResult {
        if args.is_empty() || args.len() > if allow_upper { 2 } else { 1 } {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0; 2];
            for (number, value) in numbers.iter_mut().zip(values) {
                // Preserve the existing no-Boolean coercion for all variants.
                if matches!(value, CalcResult::Boolean(_)) {
                    return CalcResult::new_error(Error::VALUE, cell, "Expecting number".into());
                }
                *number = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
                if !number.is_finite() {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        "Invalid error-function parameter".into(),
                    );
                }
            }
            let result = if complement {
                erfc(numbers[0])
            } else if values.len() == 2 {
                erf(numbers[1]) - erf(numbers[0])
            } else {
                erf(numbers[0])
            };
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Invalid error-function result".into())
            }
        })
    }
}
