// DOLLAR, FIXED, NUMBERVALUE, PROPER, REPLACE, ARRAYTOTEXT

use crate::{
    calc_result::CalcResult,
    cast::array_node_to_string,
    expressions::{
        parser::{ArrayNode, Node},
        token::Error,
        types::CellReferenceIndex,
    },
    model::Model,
};

use super::common::{broadcast_text, TextArg};

fn format_thousands(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Round decimal digits directly, without narrowing large integers to u64 or
/// scaling a floating-point number beyond its finite range.
fn format_abs(value: f64, decimals: i32, use_thousands: bool) -> String {
    let decimal = crate::number_format::format_decimal_abs(value, decimals);
    if !use_thousands {
        return decimal;
    }
    match decimal.split_once('.') {
        Some((integer, fraction)) => format!("{}.{}", format_thousands(integer), fraction),
        None => format_thousands(&decimal),
    }
}

fn decimal_places(value: f64) -> Option<i32> {
    if !value.is_finite() || value.trunc() > 127.0 {
        return None;
    }
    // Every finite Excel number rounds to zero below this decimal position.
    Some(value.trunc().max(-309.0) as i32)
}
impl<'a> Model<'a> {
    /// DOLLAR(number, [decimals])
    pub(crate) fn fn_dollar(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.fixed_text(args, cell, true)
    }

    /// FIXED(number, [decimals], [no_commas])
    pub(crate) fn fn_fixed(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.fixed_text(args, cell, false)
    }

    fn fixed_text(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        currency: bool,
    ) -> CalcResult {
        if args.is_empty() || args.len() > if currency { 2 } else { 3 } {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(3);
        for arg in args {
            match self.text_arg(arg, cell) {
                Ok(value) => operands.push(value),
                Err(error) => return error,
            }
        }
        if operands.len() == 1 {
            operands.push(TextArg::Scalar(ArrayNode::Number(2.0)));
        }
        if operands.len() == 2 {
            operands.push(TextArg::Scalar(ArrayNode::Boolean(false)));
        }
        let refs: Vec<_> = operands.iter().collect();
        broadcast_text(cell, &refs, |i, j| {
            let values: Option<Vec<_>> = operands.iter().map(|arg| arg.elem(i, j)).collect();
            let values = values.ok_or(Error::NA)?;
            let number = |value: &ArrayNode| -> Result<f64, Error> {
                match value {
                    ArrayNode::Number(n) => Ok(*n),
                    ArrayNode::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
                    ArrayNode::Empty => Ok(0.0),
                    ArrayNode::String(s) => self.cast_number(s).ok_or(Error::VALUE),
                    ArrayNode::Error(error) => Err(error.clone()),
                }
            };
            let value = number(values[0])?;
            let decimals = decimal_places(number(values[1])?).ok_or(Error::VALUE)?;
            let no_commas = match values[2] {
                ArrayNode::Boolean(b) => *b,
                ArrayNode::Number(n) => *n != 0.0,
                ArrayNode::Empty => false,
                ArrayNode::String(s) if s.eq_ignore_ascii_case("true") => true,
                ArrayNode::String(s) if s.eq_ignore_ascii_case("false") => false,
                ArrayNode::String(_) => return Err(Error::VALUE),
                ArrayNode::Error(error) => return Err(error.clone()),
            };
            if !value.is_finite() {
                return Err(Error::VALUE);
            }
            let text = format_abs(value.abs(), decimals, !no_commas);
            Ok(ArrayNode::String(if currency {
                if value < 0.0 {
                    format!("(${text})")
                } else {
                    format!("${text}")
                }
            } else if value < 0.0 {
                format!("-{text}")
            } else {
                text
            }))
        })
    }
    /// NUMBERVALUE(text, [decimal_separator], [group_separator])
    /// Converts text to a number using the given separators.
    pub(crate) fn fn_numbervalue(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() || args.len() > 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(3);
        for index in 0..3 {
            let value = match args.get(index) {
                Some(arg) if index == 0 || !matches!(arg, Node::EmptyArgKind) => {
                    match self.text_arg(arg, cell) {
                        Ok(value) => value,
                        Err(error) => return error,
                    }
                }
                _ => TextArg::Scalar(ArrayNode::String(if index == 1 {
                    self.locale.numbers.symbols.decimal.clone()
                } else {
                    self.locale.numbers.symbols.group.clone()
                })),
            };
            operands.push(value);
        }
        let refs: Vec<_> = operands.iter().collect();
        broadcast_text(cell, &refs, |i, j| {
            let values: Option<Vec<_>> = operands.iter().map(|arg| arg.elem(i, j)).collect();
            let values = values.ok_or(Error::NA)?;
            let text = array_node_to_string(values[0])?;
            let decimal = array_node_to_string(values[1])?;
            let group = array_node_to_string(values[2])?;
            Ok(crate::cast::calc_result_to_array_node(parse_numbervalue(
                text, decimal, group, cell,
            )))
        })
    }
    /// PROPER(text) — Converts text to title case (first letter of each word uppercase).
    pub(crate) fn fn_proper(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.apply_text_unary(args, cell, proper_case)
    }

    /// REPLACE(old_text, start_num, num_chars, new_text)
    /// Replaces num_chars characters starting at start_num (1-indexed) with new_text.
    pub(crate) fn fn_replace(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 4 {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(4);
        for arg in args {
            match self.text_arg(arg, cell) {
                Ok(value) => operands.push(value),
                Err(error) => return error,
            }
        }
        let refs: Vec<_> = operands.iter().collect();
        broadcast_text(cell, &refs, |i, j| {
            let values: Option<Vec<_>> = operands.iter().map(|arg| arg.elem(i, j)).collect();
            let values = values.ok_or(Error::NA)?;
            let old = array_node_to_string(values[0])?;
            let number = |value: &ArrayNode| -> Result<f64, Error> {
                match value {
                    ArrayNode::Number(n) => Ok(*n),
                    ArrayNode::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
                    ArrayNode::Empty => Ok(0.0),
                    ArrayNode::String(s) => self.cast_number(s).ok_or(Error::VALUE),
                    ArrayNode::Error(error) => Err(error.clone()),
                }
            };
            let start = number(values[1])?.floor();
            let count = number(values[2])?.floor();
            if !start.is_finite() || !count.is_finite() || start < 1.0 || count < 0.0 {
                return Err(Error::VALUE);
            }
            let new = array_node_to_string(values[3])?;
            let chars: Vec<_> = old.chars().collect();
            let start = (start - 1.0).min(chars.len() as f64) as usize;
            let count = count.min(chars.len() as f64) as usize;
            let end = start.saturating_add(count).min(chars.len());
            let result: String = chars[..start]
                .iter()
                .copied()
                .chain(new.chars())
                .chain(chars[end..].iter().copied())
                .collect();
            if result.encode_utf16().count() > 32767 {
                return Err(Error::VALUE);
            }
            Ok(ArrayNode::String(result))
        })
    }
}
fn proper_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if capitalize_next {
                result.extend(c.to_uppercase());
                capitalize_next = false;
            } else {
                result.extend(c.to_lowercase());
            }
        } else {
            result.push(c);
            capitalize_next = true;
        }
    }
    result
}

fn parse_numbervalue(
    text: String,
    decimal_sep: String,
    group_sep: String,
    cell: CellReferenceIndex,
) -> CalcResult {
    let Some(decimal_sep) = decimal_sep.chars().next() else {
        return CalcResult::new_error(Error::VALUE, cell, "Invalid separators".to_string());
    };
    let group_sep = group_sep.chars().next();
    if Some(decimal_sep) == group_sep {
        return CalcResult::new_error(Error::VALUE, cell, "Invalid separators".to_string());
    }

    // Excel ignores spaces throughout the text, including between percent
    // suffixes. Separator arguments use their first Unicode character.
    let text: String = text.chars().filter(|c| *c != ' ').collect();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return CalcResult::Number(0.0);
    }

    // Handle percentage suffix
    let (trimmed, pct_count) = {
        let mut t = trimmed;
        let mut count = 0u32;
        while t.ends_with('%') {
            t = &t[..t.len() - 1];
            count += 1;
        }
        (t.trim_end(), count)
    };

    if trimmed.is_empty() {
        return CalcResult::new_error(Error::VALUE, cell, "Empty string".to_string());
    }

    let mut cleaned = String::new();
    let mut decimal_seen = false;
    for c in trimmed.chars() {
        if c == decimal_sep {
            if decimal_seen {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Multiple decimal separators".into(),
                );
            }
            decimal_seen = true;
            cleaned.push('.');
        } else if Some(c) == group_sep {
            if decimal_seen {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Group separator after decimal separator".into(),
                );
            }
        } else if c.is_ascii_digit() || matches!(c, '+' | '-' | 'e' | 'E') {
            cleaned.push(c);
        } else {
            return CalcResult::new_error(Error::VALUE, cell, "Invalid numeric character".into());
        }
    }

    match cleaned.parse::<f64>() {
        Ok(mut v) if v.is_finite() => {
            for _ in 0..pct_count {
                v /= 100.0;
            }
            CalcResult::Number(v)
        }
        _ => CalcResult::new_error(Error::VALUE, cell, "Cannot parse number".to_string()),
    }
}
