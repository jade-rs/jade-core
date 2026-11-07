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
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let x = match self.get_number_no_bools(&args[0], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = match self.get_number_no_bools(&args[1], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = n.trunc() as i32;
        let result = bessel_i(n, x);
        if result.is_infinite() || result.is_nan() {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        CalcResult::Number(result)
    }
    pub(crate) fn fn_besselj(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let x = match self.get_number_no_bools(&args[0], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = match self.get_number_no_bools(&args[1], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = n.trunc() as i32;
        if n < 0 {
            // In Excel this ins #NUM!
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        let result = bessel_j(n, x);
        if result.is_infinite() || result.is_nan() {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        CalcResult::Number(result)
    }

    pub(crate) fn fn_besselk(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let x = match self.get_number_no_bools(&args[0], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = match self.get_number_no_bools(&args[1], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = n.trunc() as i32;
        let result = bessel_k(n, x);
        if result.is_infinite() || result.is_nan() {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        CalcResult::Number(result)
    }

    pub(crate) fn fn_bessely(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let x = match self.get_number_no_bools(&args[0], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = match self.get_number_no_bools(&args[1], cell) {
            Ok(f) => f,
            Err(s) => return s,
        };
        let n = n.trunc() as i32;
        if n < 0 {
            // In Excel this ins #NUM!
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        let result = bessel_y(n, x);
        if result.is_infinite() || result.is_nan() {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid parameter for Bessel function".to_string(),
            };
        }
        CalcResult::Number(result)
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
