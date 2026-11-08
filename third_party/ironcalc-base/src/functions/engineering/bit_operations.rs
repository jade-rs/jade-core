use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, token::Error, types::CellReferenceIndex},
    model::Model,
};

const MAX: u128 = (1 << 48) - 1;

#[derive(Clone, Copy)]
enum BitOperation {
    And,
    Or,
    Xor,
    Left,
    Right,
}

impl<'a> Model<'a> {
    pub(crate) fn fn_bitand(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bit_arguments(args, cell, BitOperation::And)
    }
    pub(crate) fn fn_bitor(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bit_arguments(args, cell, BitOperation::Or)
    }
    pub(crate) fn fn_bitxor(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bit_arguments(args, cell, BitOperation::Xor)
    }
    pub(crate) fn fn_bitlshift(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bit_arguments(args, cell, BitOperation::Left)
    }
    pub(crate) fn fn_bitrshift(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.bit_arguments(args, cell, BitOperation::Right)
    }

    fn bit_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        operation: BitOperation,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0; 2];
            for (number, value) in numbers.iter_mut().zip(values) {
                *number = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
            }
            let invalid = || {
                CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid bit-operation parameter or result".into(),
                )
            };
            let valid_integer = |number: f64| {
                number.is_finite()
                    && number >= 0.0
                    && number <= MAX as f64
                    && number.trunc() == number
            };
            if !valid_integer(numbers[0]) || !numbers[1].is_finite() {
                return invalid();
            }
            let number = numbers[0] as u128;
            let result = match operation {
                BitOperation::And | BitOperation::Or | BitOperation::Xor => {
                    if !valid_integer(numbers[1]) {
                        return invalid();
                    }
                    let other = numbers[1] as u128;
                    match operation {
                        BitOperation::And => number & other,
                        BitOperation::Or => number | other,
                        _ => number ^ other,
                    }
                }
                BitOperation::Left | BitOperation::Right => {
                    if numbers[1].abs() > 53.0 {
                        return invalid();
                    }
                    // Retain the existing shift-count truncation policy.
                    let shift = numbers[1].trunc() as i32;
                    let left = matches!(operation, BitOperation::Left) == (shift >= 0);
                    // At most 48 + 53 bits: u128 preserves all shifted bits so
                    // the Excel 48-bit limit can be checked without wrapping.
                    if left {
                        number << shift.unsigned_abs()
                    } else {
                        number >> shift.unsigned_abs()
                    }
                }
            };
            if result > MAX {
                invalid()
            } else {
                CalcResult::Number(result as f64)
            }
        })
    }
}
