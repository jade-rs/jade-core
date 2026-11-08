use crate::constants::{EXCEL_PRECISION, LAST_COLUMN, LAST_ROW};
use crate::expressions::parser::ArrayNode;
use crate::expressions::types::CellReferenceIndex;
use crate::number_format::to_excel_precision;
use crate::single_number_fn;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};
use std::f64::consts::PI;

// Parse up to 255 base-36 digits exactly, then round once to binary64.
// This avoids imposing i64 limits or accumulating per-digit float rounding.
fn decimal_integer(text: &str, radix: u32) -> Result<f64, Error> {
    if !(2..=36).contains(&radix) || text.chars().count() > 255 {
        return Err(Error::NUM);
    }
    // Preserve the existing signed-text parsing policy pending native captures.
    let (negative, digits) = if let Some(digits) = text.strip_prefix('-') {
        (true, digits)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if digits.is_empty() {
        return Err(Error::VALUE);
    }
    let mut limbs = vec![0_u32];
    for digit in digits.chars() {
        let mut carry = u64::from(digit.to_digit(radix).ok_or(Error::VALUE)?);
        for limb in &mut limbs {
            let value = u64::from(*limb) * u64::from(radix) + carry;
            *limb = value as u32;
            carry = value >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
    }
    let bits = (limbs.len() - 1) * 32 + (32 - limbs[limbs.len() - 1].leading_zeros()) as usize;
    if bits > 1024 {
        return Err(Error::NUM);
    }
    let bit = |index: usize| (limbs[index / 32] >> (index % 32)) & 1 != 0;
    let discarded = bits.saturating_sub(53);
    let mut mantissa = (discarded..bits)
        .rev()
        .fold(0_u64, |value, index| (value << 1) | u64::from(bit(index)));
    if discarded > 0 && bit(discarded - 1) && ((0..discarded - 1).any(bit) || mantissa & 1 != 0) {
        mantissa += 1;
    }
    let number = mantissa as f64 * 2.0_f64.powi(discarded as i32);
    if !number.is_finite() {
        return Err(Error::NUM);
    }
    Ok(if negative && number != 0.0 {
        -number
    } else {
        number
    })
}

impl<'a> Model<'a> {
    pub(crate) fn fn_min(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let mut result = f64::NAN;
        for arg in args {
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Number(value) => result = value.min(result),
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        );
                    }
                    for row in left.row..(right.row + 1) {
                        for column in left.column..(right.column + 1) {
                            match self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            }) {
                                CalcResult::Number(value) => {
                                    result = value.min(result);
                                }
                                error @ CalcResult::Error { .. } => return error,
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in array {
                        for node in row {
                            match node {
                                ArrayNode::Number(value) => result = value.min(result),
                                ArrayNode::Error(error) => {
                                    return CalcResult::Error {
                                        error,
                                        origin: cell,
                                        message: String::new(),
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                _ => {
                    // We ignore booleans and strings
                }
            };
        }
        if result.is_nan() || result.is_infinite() {
            return CalcResult::Number(0.0);
        }
        CalcResult::Number(result)
    }

    pub(crate) fn fn_max(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let mut result = f64::NAN;
        for arg in args {
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Number(value) => result = value.max(result),
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        );
                    }
                    for row in left.row..(right.row + 1) {
                        for column in left.column..(right.column + 1) {
                            match self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            }) {
                                CalcResult::Number(value) => {
                                    result = value.max(result);
                                }
                                error @ CalcResult::Error { .. } => return error,
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in array {
                        for node in row {
                            match node {
                                ArrayNode::Number(value) => result = value.max(result),
                                ArrayNode::Error(error) => {
                                    return CalcResult::Error {
                                        error,
                                        origin: cell,
                                        message: String::new(),
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                _ => {
                    // We ignore booleans and strings
                }
            };
        }
        if result.is_nan() || result.is_infinite() {
            return CalcResult::Number(0.0);
        }
        CalcResult::Number(result)
    }

    pub(crate) fn fn_base(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0; 3];
            for (number, value) in numbers.iter_mut().zip(values) {
                *number = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number.trunc(),
                    Err(error) => return error,
                };
            }
            let [number, radix, min_length] = numbers;
            if numbers.iter().any(|number| !number.is_finite())
                || !(0.0..9007199254740992.0).contains(&number)
                || !(2.0..=36.0).contains(&radix)
                || !(0.0..=255.0).contains(&min_length)
            {
                return CalcResult::new_error(Error::NUM, cell, "Invalid BASE parameters".into());
            }
            let mut number = number as u64;
            let radix = radix as u64;
            // At most 53 binary digits; padding is bounded to 255 characters.
            let mut digits = Vec::with_capacity(53);
            loop {
                let digit = (number % radix) as u8;
                digits.push(if digit < 10 {
                    b'0' + digit
                } else {
                    b'A' + digit - 10
                });
                number /= radix;
                if number == 0 {
                    break;
                }
            }
            let min_length = min_length as usize;
            let mut result = String::with_capacity(digits.len().max(min_length));
            for _ in digits.len()..min_length {
                result.push('0');
            }
            result.extend(digits.iter().rev().map(|digit| *digit as char));
            CalcResult::String(result)
        })
    }

    pub(crate) fn fn_decimal(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let text = match model.cast_to_string(values[0].clone(), cell) {
                Ok(text) => text,
                Err(error) => return error,
            };
            let radix = match model.cast_to_number(values[1].clone(), cell) {
                Ok(radix) if radix.is_finite() && (2.0..=36.0).contains(&radix.trunc()) => {
                    radix.trunc() as u32
                }
                Ok(_) => {
                    return CalcResult::new_error(Error::NUM, cell, "Invalid DECIMAL radix".into())
                }
                Err(error) => return error,
            };
            match decimal_integer(&text, radix) {
                Ok(number) => CalcResult::Number(number),
                Err(error) => CalcResult::new_error(error, cell, "Invalid DECIMAL text".into()),
            }
        })
    }

    /// SUM(number1, [number2], ...)
    ///
    /// The SUM function adds values. You can add individual values, cell references or ranges or a mix of all three.
    ///
    /// # Details
    /// Generally each argument is evaluated and the result is converted to a number. However there are some special
    /// cases:
    /// - **Args evaluating to CalcResult::Range**: The sum of all the numbers in the range is calculated. No
    ///   conversions are performed, non-numeric values are ignored and errors are propagated.
    /// - **Node::ReferenceKind args**: Similar to ranges, conversions are not performed on the evaulated value of the
    ///   referenced cell and errors are propagated.
    /// - **Args evaluating to CalcResult::Error**: Errors are propagated.
    pub(crate) fn fn_sum(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }

        let mut result = 0.0;
        for arg in args {
            // Handle reference special case. Behaviour of a reference is equivalent to a range of one cell.
            let arg = match arg {
                Node::ReferenceKind {
                    column,
                    absolute_column,
                    row,
                    absolute_row,
                    sheet_index,
                    sheet_name,
                } => &Node::RangeKind {
                    sheet_name: sheet_name.clone(),
                    sheet_index: *sheet_index,
                    absolute_row1: *absolute_row,
                    absolute_column1: *absolute_column,
                    row1: *row,
                    column1: *column,
                    absolute_row2: *absolute_row,
                    absolute_column2: *absolute_column,
                    row2: *row,
                    column2: *column,
                },
                _ => arg,
            };

            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        );
                    }
                    // TODO: We should do this for all functions that run through ranges
                    // Running cargo test for the ironcalc takes around .8 seconds with this speedup
                    // and ~ 3.5 seconds without it. Note that once properly in place sheet.dimension should be almost a noop
                    let row1 = left.row;
                    let mut row2 = right.row;
                    let column1 = left.column;
                    let mut column2 = right.column;
                    if row1 == 1 && row2 == LAST_ROW {
                        row2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_row,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    if column1 == 1 && column2 == LAST_COLUMN {
                        column2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_column,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    for row in row1..row2 + 1 {
                        for column in column1..(column2 + 1) {
                            match self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            }) {
                                CalcResult::Number(value) => {
                                    result += value;
                                }
                                error @ CalcResult::Error { .. } => return error,
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in array {
                        for value in row {
                            match value {
                                ArrayNode::Number(value) => {
                                    result += value;
                                }
                                ArrayNode::Error(error) => {
                                    return CalcResult::Error {
                                        error,
                                        origin: cell,
                                        message: "Error in array".to_string(),
                                    }
                                }
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                calc_result => {
                    let cast_result = self.cast_to_number(calc_result, cell);
                    match cast_result {
                        Ok(f) => result += f,
                        Err(s) => return s,
                    }
                }
            };
        }
        CalcResult::Number(result)
    }

    pub(crate) fn fn_sumsq(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }

        let mut result = 0.0;
        for arg in args {
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Number(value) => result += value * value,
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        );
                    }
                    // TODO: We should do this for all functions that run through ranges
                    // Running cargo test for the ironcalc takes around .8 seconds with this speedup
                    // and ~ 3.5 seconds without it. Note that once properly in place sheet.dimension should be almost a noop
                    let row1 = left.row;
                    let mut row2 = right.row;
                    let column1 = left.column;
                    let mut column2 = right.column;
                    if row1 == 1 && row2 == LAST_ROW {
                        row2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_row,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    if column1 == 1 && column2 == LAST_COLUMN {
                        column2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_column,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    for row in row1..row2 + 1 {
                        for column in column1..(column2 + 1) {
                            match self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            }) {
                                CalcResult::Number(value) => {
                                    result += value * value;
                                }
                                error @ CalcResult::Error { .. } => return error,
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in array {
                        for value in row {
                            match value {
                                ArrayNode::Number(value) => {
                                    result += value * value;
                                }
                                ArrayNode::Error(error) => {
                                    return CalcResult::Error {
                                        error,
                                        origin: cell,
                                        message: "Error in array".to_string(),
                                    }
                                }
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                _ => {
                    // We ignore booleans and strings
                }
            };
        }
        CalcResult::Number(result)
    }

    /// PRODUCT(number1, [number2], ...)
    ///
    /// The PRODUCT function multiplies values. You can add individual values, cell references or ranges or a mix of all
    /// three.
    ///
    /// # Details
    /// Generally each argument is evaluated and the result is converted to a number. However there are some special
    /// cases:
    /// - **Args evaluating to CalcResult::Range**: The product of all the numbers in the range is calculated. No
    ///   conversions are performed, non-numeric values are ignored and errors are propagated.
    /// - **Node::ReferenceKind args**: Similar to ranges, conversions are not performed on the evaulated value of the
    ///   referenced cell and errors are propagated.
    /// - **Args evaluating to CalcResult::Error**: Errors are propagated.
    pub(crate) fn fn_product(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }
        let mut result = 1.0;
        let mut seen_value = false;

        for arg in args {
            // Handle reference special case. Behaviour of a reference is equivalent to a range of one cell.
            let arg = match arg {
                Node::ReferenceKind {
                    column,
                    absolute_column,
                    row,
                    absolute_row,
                    sheet_index,
                    sheet_name,
                } => &Node::RangeKind {
                    sheet_name: sheet_name.clone(),
                    sheet_index: *sheet_index,
                    absolute_row1: *absolute_row,
                    absolute_column1: *absolute_column,
                    row1: *row,
                    column1: *column,
                    absolute_row2: *absolute_row,
                    absolute_column2: *absolute_column,
                    row2: *row,
                    column2: *column,
                },
                _ => arg,
            };
            let cell_value = self.evaluate_node_in_context(arg, cell);

            match cell_value {
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        );
                    }
                    // TODO: We should do this for all functions that run through ranges. See fn_sum for more details
                    let row1 = left.row;
                    let mut row2 = right.row;
                    let column1 = left.column;
                    let mut column2 = right.column;
                    if row1 == 1 && row2 == LAST_ROW {
                        row2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_row,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    if column1 == 1 && column2 == LAST_COLUMN {
                        column2 = match self.workbook.worksheet(left.sheet) {
                            Ok(s) => s.dimension().max_column,
                            Err(_) => {
                                return CalcResult::new_error(
                                    Error::ERROR,
                                    cell,
                                    format!("Invalid worksheet index: '{}'", left.sheet),
                                );
                            }
                        };
                    }
                    for row in row1..row2 + 1 {
                        for column in column1..(column2 + 1) {
                            let cell_value = self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            });

                            match cell_value {
                                CalcResult::Number(value) => {
                                    seen_value = true;
                                    result *= value;
                                }
                                error @ CalcResult::Error { .. } => return error,
                                _ => {
                                    // We ignore booleans and strings
                                }
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                calc_result => {
                    seen_value = true;
                    let cast_result = self.cast_to_number(calc_result, cell);
                    match cast_result {
                        Ok(f) => result *= f,
                        Err(s) => return s,
                    }
                }
            };
        }
        if !seen_value {
            return CalcResult::Number(0.0);
        }
        CalcResult::Number(result)
    }

    /// SUMIF(criteria_range, criteria, [sum_range])
    /// if sum_range is missing then criteria_range will be used.
    ///
    /// The `criteria` argument may be a single value, a range or an array; in the
    /// latter two cases SUMIF spills one sum per criterion (see [`Model::sumif`]).
    pub(crate) fn fn_sumif(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        // When sum_range is missing, criteria_range doubles as the sum_range.
        let (criteria_range, criteria, sum_range) = match args.len() {
            2 => (&args[0], &args[1], &args[0]),
            3 => (&args[0], &args[1], &args[2]),
            _ => return CalcResult::new_args_number_error(cell),
        };
        self.sumif(criteria_range, criteria, sum_range, cell)
    }

    /// SUMIFS(sum_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)
    pub(crate) fn fn_sumifs(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let mut total = 0.0;
        let sum = |value| total += value;
        if let Err(e) = self.apply_ifs(args, cell, sum) {
            return e;
        }
        CalcResult::Number(total)
    }

    pub(crate) fn fn_round(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_arguments(args, cell, crate::number_format::DecimalRounding::Nearest)
    }

    pub(crate) fn fn_roundup(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_arguments(
            args,
            cell,
            crate::number_format::DecimalRounding::AwayFromZero,
        )
    }

    pub(crate) fn fn_rounddown(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_arguments(
            args,
            cell,
            crate::number_format::DecimalRounding::TowardZero,
        )
    }

    fn round_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        mode: crate::number_format::DecimalRounding,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let value = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let decimals = match model.cast_to_number(values[1].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            match crate::number_format::round_decimal(value, decimals, mode) {
                Some(number) => CalcResult::Number(number),
                None => {
                    CalcResult::new_error(Error::NUM, cell, "Rounding result out of range".into())
                }
            }
        })
    }

    // (number, divisor)
    pub(crate) fn fn_mod(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let value = match model.cast_to_number(values[0].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let divisor = match model.cast_to_number(values[1].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            if divisor == 0.0 {
                return CalcResult::Error {
                    error: Error::DIV,
                    origin: cell,
                    message: "Divide by 0".to_string(),
                };
            }
            let result = value - divisor * (value / divisor).floor();
            if value.is_finite() && divisor.is_finite() && result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Remainder out of range".into())
            }
        })
    }

    pub(crate) fn fn_quotient(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let number = |model: &mut Self, value: &CalcResult| {
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
            let value = match number(model, &values[0]) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let divisor = match number(model, &values[1]) {
                Ok(f) => f,
                Err(s) => return s,
            };
            if divisor == 0.0 {
                return CalcResult::Error {
                    error: Error::DIV,
                    origin: cell,
                    message: "Divide by 0".to_string(),
                };
            }

            let result = value / divisor;
            if value.is_finite() && divisor.is_finite() && result.is_finite() {
                CalcResult::Number(result.trunc())
            } else {
                CalcResult::new_error(Error::NUM, cell, "Quotient out of range".into())
            }
        })
    }

    pub(crate) fn fn_floor(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_legacy_multiple(args, cell, false)
    }

    pub(crate) fn fn_ceiling(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_legacy_multiple(args, cell, true)
    }

    fn round_legacy_multiple(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        ceiling: bool,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let value = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let significance = match model.cast_to_number(values[1].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !value.is_finite()
                || !significance.is_finite()
                || (value > 0.0 && significance < 0.0)
            {
                return CalcResult::new_error(Error::NUM, cell, "Invalid rounding operands".into());
            }
            if significance == 0.0 {
                return if ceiling || value == 0.0 {
                    CalcResult::Number(0.0)
                } else {
                    CalcResult::new_error(Error::DIV, cell, "Divide by 0".into())
                };
            }
            let ratio = to_excel_precision(value / significance, EXCEL_PRECISION);
            let result = if ceiling { ratio.ceil() } else { ratio.floor() } * significance;
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Rounded multiple out of range".into())
            }
        })
    }

    pub(crate) fn fn_ceiling_math(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.round_modern_multiple(args, cell, true, true)
    }

    pub(crate) fn fn_ceiling_precise(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.round_modern_multiple(args, cell, true, false)
    }

    pub(crate) fn fn_iso_ceiling(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.fn_ceiling_precise(args, cell)
    }

    pub(crate) fn fn_floor_math(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.round_modern_multiple(args, cell, false, true)
    }

    pub(crate) fn fn_floor_precise(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.round_modern_multiple(args, cell, false, false)
    }

    fn round_modern_multiple(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        ceiling: bool,
        allow_mode: bool,
    ) -> CalcResult {
        if !(1..=if allow_mode { 3 } else { 2 }).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut numbers = [0.0, 1.0, 0.0];
            for (index, value) in values.iter().enumerate() {
                numbers[index] = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
            }
            if numbers.iter().any(|number| !number.is_finite()) {
                return CalcResult::new_error(Error::NUM, cell, "Invalid rounding operands".into());
            }
            let [value, significance, mode] = numbers;
            let significance = significance.abs();
            if significance == 0.0 {
                return CalcResult::Number(0.0);
            }
            let ratio = to_excel_precision(value / significance, EXCEL_PRECISION);
            let round_up = ceiling ^ (value < 0.0 && mode != 0.0);
            let result = if round_up {
                ratio.ceil()
            } else {
                ratio.floor()
            } * significance;
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Rounded multiple out of range".into())
            }
        })
    }

    pub(crate) fn fn_mround(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        // MROUND(number, multiple)
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }

        self.map_scalar_arguments(args, cell, |model, values| {
            let value = values[0].clone();
            let multiple = values[1].clone();

            // if either is empty => #N/A
            if matches!(value, CalcResult::EmptyArg) || matches!(multiple, CalcResult::EmptyArg) {
                return CalcResult::Error {
                    error: Error::NA,
                    origin: cell,
                    message: "Bad argument for MROUND".to_string(),
                };
            }

            // Booleans are not cast
            if matches!(value, CalcResult::Boolean(_)) {
                return CalcResult::new_error(Error::VALUE, cell, "Expecting number".to_string());
            }

            if matches!(multiple, CalcResult::Boolean(_)) {
                return CalcResult::new_error(Error::VALUE, cell, "Expecting number".to_string());
            }

            let value = match model.cast_to_number(value, cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let multiple = match model.cast_to_number(multiple, cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            if multiple == 0.0 {
                return CalcResult::Number(0.0);
            }
            if (value > 0.0 && multiple < 0.0) || (value < 0.0 && multiple > 0.0) {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "number and multiple must have the same sign".to_string(),
                };
            }
            let result = (value / multiple).round() * multiple;
            if value.is_finite() && multiple.is_finite() && result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Rounded multiple out of range".into())
            }
        })
    }

    pub(crate) fn fn_trunc(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let value = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let digits = if values.len() == 2 {
                match model.cast_to_number(values[1].clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                }
            } else {
                0.0
            };
            match crate::number_format::round_decimal(
                value,
                digits,
                crate::number_format::DecimalRounding::TowardZero,
            ) {
                Some(number) => CalcResult::Number(number),
                None => {
                    CalcResult::new_error(Error::NUM, cell, "Truncation result out of range".into())
                }
            }
        })
    }

    single_number_fn!(fn_log10, |f| if f <= 0.0 {
        Err(Error::NUM)
    } else {
        Ok(f64::log10(f))
    });
    single_number_fn!(fn_ln, |f| if f <= 0.0 {
        Err(Error::NUM)
    } else {
        Ok(f64::ln(f))
    });
    single_number_fn!(fn_sin, |f| Ok(f64::sin(f)));
    single_number_fn!(fn_cos, |f| Ok(f64::cos(f)));
    single_number_fn!(fn_tan, |f| Ok(f64::tan(f)));
    single_number_fn!(fn_sinh, |f| Ok(f64::sinh(f)));
    single_number_fn!(fn_cosh, |f| Ok(f64::cosh(f)));
    single_number_fn!(fn_tanh, |f| Ok(f64::tanh(f)));
    single_number_fn!(fn_asin, |f| Ok(f64::asin(f)));
    single_number_fn!(fn_acos, |f| Ok(f64::acos(f)));
    single_number_fn!(fn_atan, |f| Ok(f64::atan(f)));
    single_number_fn!(fn_asinh, |f| Ok(f64::asinh(f)));
    single_number_fn!(fn_acosh, |f| Ok(f64::acosh(f)));
    single_number_fn!(fn_atanh, |f| Ok(f64::atanh(f)));
    single_number_fn!(fn_abs, |f| Ok(f64::abs(f)));
    single_number_fn!(fn_sqrt, |f| if f < 0.0 {
        Err(Error::NUM)
    } else {
        Ok(f64::sqrt(f))
    });
    single_number_fn!(fn_sqrtpi, |f: f64| if f < 0.0 {
        Err(Error::NUM)
    } else {
        Ok((f * PI).sqrt())
    });

    single_number_fn!(fn_acot, |f| {
        let v = f64::atan(1.0 / f);
        if f >= 0.0 {
            Ok(v)
        } else {
            // To be compatible with Excel we need a different branch
            // when f < 0
            Ok(v + PI)
        }
    });
    single_number_fn!(fn_acoth, |f: f64| if f.abs() == 1.0 {
        Err(Error::NUM)
    } else {
        Ok(0.5 * (f64::ln((f + 1.0) / (f - 1.0))))
    });
    single_number_fn!(fn_cot, |f| if f == 0.0 {
        Err(Error::DIV)
    } else {
        Ok(f64::cos(f) / f64::sin(f))
    });
    single_number_fn!(fn_coth, |f: f64| if f == 0.0 {
        Err(Error::DIV)
    } else if f.abs() > 20.0 {
        // for values > 20.0 this is exact in f64
        Ok(f.signum())
    } else {
        Ok(f64::cosh(f) / f64::sinh(f))
    });
    single_number_fn!(fn_csc, |f| if f == 0.0 {
        Err(Error::DIV)
    } else {
        Ok(1.0 / f64::sin(f))
    });
    single_number_fn!(fn_csch, |f| if f == 0.0 {
        Err(Error::DIV)
    } else {
        Ok(1.0 / f64::sinh(f))
    });
    single_number_fn!(fn_sec, |f| Ok(1.0 / f64::cos(f)));
    single_number_fn!(fn_sech, |f| Ok(1.0 / f64::cosh(f)));
    single_number_fn!(fn_exp, |f: f64| Ok(f64::exp(f)));
    pub(crate) fn fn_fact(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.factorial_argument(args, cell, false)
    }

    pub(crate) fn fn_factdouble(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.factorial_argument(args, cell, true)
    }

    fn factorial_argument(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        double: bool,
    ) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let number = match model.cast_to_number(value, cell) {
                Ok(number) => number.floor(),
                Err(error) => return error,
            };
            // 170! and 300!! are finite; the next respective integers overflow.
            let maximum = if double { 300.0 } else { 170.0 };
            if !number.is_finite() || !(0.0..=maximum).contains(&number) {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid or overflowing factorial".into(),
                );
            }
            let mut result = 1.0;
            let step = if double { 2 } else { 1 };
            let first = if double && number as u32 % 2 == 1 {
                1
            } else {
                2
            };
            for factor in (first..=number as u32).step_by(step) {
                result *= factor as f64;
            }
            CalcResult::Number(result)
        })
    }

    single_number_fn!(fn_sign, |f| {
        if f == 0.0 {
            Ok(0.0)
        } else {
            Ok(f64::signum(f))
        }
    });
    single_number_fn!(fn_degrees, |f| Ok(f * (180.0 / PI)));
    single_number_fn!(fn_radians, |f| Ok(f * (PI / 180.0)));
    single_number_fn!(fn_odd, |f| {
        let sign = f64::signum(f);
        Ok(sign * (f64::ceil((f64::abs(f) - 1.0) / 2.0) * 2.0 + 1.0))
    });
    single_number_fn!(fn_even, |f| Ok(f64::signum(f)
        * f64::ceil(f64::abs(f) / 2.0)
        * 2.0));
    single_number_fn!(fn_int, |f| Ok(f64::floor(f)));

    pub(crate) fn fn_pi(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }
        CalcResult::Number(PI)
    }

    pub(crate) fn fn_atan2(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let x = match model.cast_to_number(values[0].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let y = match model.cast_to_number(values[1].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            if !x.is_finite() || !y.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, "Invalid coordinates".into());
            }
            if x == 0.0 && y == 0.0 {
                return CalcResult::Error {
                    error: Error::DIV,
                    origin: cell,
                    message: "Arguments can't be both zero".to_string(),
                };
            }
            let angle = f64::atan2(y, x);
            // Excel's principal interval excludes -pi, including signed-zero y.
            CalcResult::Number(if angle == -PI { PI } else { angle })
        })
    }

    pub(crate) fn fn_log(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let n_args = args.len();
        if !(1..=2).contains(&n_args) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let x = match model.cast_to_number(values[0].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let y = if n_args == 1 {
                10.0
            } else {
                match model.cast_to_number(values[1].clone(), cell) {
                    Ok(f) => f,
                    Err(s) => return s,
                }
            };
            if !x.is_finite() || !y.is_finite() || x <= 0.0 {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Number must be positive".to_string(),
                };
            }
            if y == 1.0 {
                return CalcResult::Error {
                    error: Error::DIV,
                    origin: cell,
                    message: "Logarithm base cannot be 1".to_string(),
                };
            }
            if y <= 0.0 {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Logarithm base must be positive".to_string(),
                };
            }
            let result = f64::log(x, y);
            if result.is_finite() {
                CalcResult::Number(result)
            } else {
                CalcResult::new_error(Error::NUM, cell, "Logarithm out of range".into())
            }
        })
    }

    pub(crate) fn fn_power(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let x = match model.cast_to_number(values[0].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            let y = match model.cast_to_number(values[1].clone(), cell) {
                Ok(f) => f,
                Err(s) => return s,
            };
            if !x.is_finite() || !y.is_finite() {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Invalid arguments for POWER".into(),
                );
            }
            if x == 0.0 && y < 0.0 {
                return CalcResult::new_error(
                    Error::DIV,
                    cell,
                    "Zero raised to a negative power".into(),
                );
            }
            if x == 0.0 && y == 0.0 {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Arguments can't be both zero".to_string(),
                };
            }
            if y == 0.0 {
                return CalcResult::Number(1.0);
            }
            let result = x.powf(y);
            if result.is_infinite() {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "POWER returned infinity".to_string(),
                };
            }
            if result.is_nan() {
                // This might happen for some combinations of negative base and exponent
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Invalid arguments for POWER".to_string(),
                };
            }
            CalcResult::Number(result)
        })
    }

    pub(crate) fn fn_combin(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.combination_arguments(args, cell, false)
    }

    fn combination_arguments(
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
                    "Invalid or overflowing combination".into(),
                )
            };
            if !n.is_finite()
                || !k.is_finite()
                || n < 0.0
                || k < 0.0
                || (!repetitions && k > n)
                || (repetitions && n == 0.0 && k > 0.0)
            {
                return invalid();
            }
            if k == 0.0 || (repetitions && n == 1.0) {
                return CalcResult::Number(1.0);
            }
            if k == 1.0 {
                return CalcResult::Number(n);
            }
            // Keep exactly representable integer inputs in integer arithmetic.
            // For repetitions, form n+k-1 before converting back to f64.
            let exact_total = if n <= 9_007_199_254_740_992.0 && k <= 9_007_199_254_740_992.0 {
                Some(if repetitions {
                    n as u128 + k as u128 - 1
                } else {
                    n as u128
                })
            } else {
                None
            };
            let (total, steps) = if repetitions {
                (n + k - 1.0, k.min(n - 1.0))
            } else {
                (n, k.min(n - k))
            };
            // Symmetry makes steps <= total/2, so C(total, steps) >= 2^steps.
            // At 1024 steps even that lower bound exceeds finite f64 range.
            if !total.is_finite() || steps >= 1024.0 {
                return invalid();
            }
            let mut result = 1.0;
            let mut exact_result = exact_total.map(|_| 1_u128);
            for index in 0..steps as usize {
                let next_exact = exact_total.zip(exact_result).and_then(|(total, previous)| {
                    let mut numerator = total - index as u128;
                    let mut denominator = index as u128 + 1;
                    let (mut a, mut b) = (numerator, denominator);
                    while b != 0 {
                        (a, b) = (b, a % b);
                    }
                    numerator /= a;
                    denominator /= a;
                    // The remaining denominator divides the previous binomial.
                    (previous / denominator).checked_mul(numerator)
                });
                if let Some(integer) = next_exact {
                    result = integer as f64;
                } else {
                    result *= (total - index as f64) / (index as f64 + 1.0);
                }
                exact_result = next_exact;
                if !result.is_finite() {
                    return invalid();
                }
            }
            CalcResult::Number(result)
        })
    }

    // PERCENTOF(part, total) — returns SUM(part) / SUM(total) as a decimal fraction.
    pub(crate) fn fn_percentof(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let part = match self.fn_sum(&args[0..1], cell) {
            CalcResult::Number(v) => v,
            other => return other,
        };
        let total = match self.fn_sum(&args[1..2], cell) {
            CalcResult::Number(v) => v,
            other => return other,
        };
        if total == 0.0 {
            return CalcResult::new_error(Error::DIV, cell, "Division by zero".to_string());
        }
        CalcResult::Number(part / total)
    }

    pub(crate) fn fn_combina(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.combination_arguments(args, cell, true)
    }
}

#[cfg(test)]
mod decimal_tests {
    use super::*;

    #[test]
    fn decimal_accepts_unsigned_values_beyond_i64() {
        let mut model = crate::test::util::new_empty_model();
        let cell = CellReferenceIndex {
            sheet: 0,
            row: 1,
            column: 1,
        };
        let result = model.fn_decimal(
            &[
                Node::StringKind("FFFFFFFFFFFFFFFF".into()),
                Node::NumberKind(16.0),
            ],
            cell,
        );
        assert!(matches!(result, CalcResult::Number(number) if number == 2.0_f64.powi(64)));
    }
    #[test]
    fn decimal_integer_rounds_exactly_across_all_radices() {
        let mut values = vec![
            0,
            1,
            (1_u128 << 53) - 1,
            (1_u128 << 53) + 1,
            (1_u128 << 53) + 3,
            u64::MAX as u128,
            (1_u128 << 127) - 1,
            u128::MAX,
        ];
        let mut state = 0x123456789abcdef_u128;
        for _ in 0..128 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            values.push(state);
        }
        for radix in 2..=36_u32 {
            for &value in &values {
                let mut number = value;
                let mut digits = Vec::new();
                loop {
                    let digit = (number % u128::from(radix)) as u32;
                    digits.push(char::from_digit(digit, radix).unwrap_or('0'));
                    number /= u128::from(radix);
                    if number == 0 {
                        break;
                    }
                }
                let text: String = digits.into_iter().rev().collect();
                let actual = decimal_integer(&text, radix);
                assert!(
                    matches!(actual, Ok(number) if number.to_bits()==(value as f64).to_bits()),
                    "base={radix}, text={text}"
                );
            }
        }
    }

    #[test]
    fn decimal_integer_checks_length_overflow_and_wide_values() {
        assert_eq!(
            decimal_integer(&format!("1{}", "0".repeat(64)), 16),
            Ok(2.0_f64.powi(256))
        );
        assert_eq!(
            decimal_integer(&"F".repeat(255), 16),
            Ok(2.0_f64.powi(1020))
        );
        assert_eq!(decimal_integer(&"Z".repeat(255), 36), Err(Error::NUM));
        assert_eq!(decimal_integer(&"0".repeat(256), 2), Err(Error::NUM));
        assert_eq!(decimal_integer("-FF", 16), Ok(-255.0));
        assert_eq!(decimal_integer("G", 16), Err(Error::VALUE));
        assert_eq!(decimal_integer("", 16), Err(Error::VALUE));
    }
}
