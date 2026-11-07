#[cfg(target_arch = "wasm32")]
use regex_lite as regex;

use crate::{
    arithmetic::bcast_idx,
    calc_result::CalcResult,
    cast::{array_node_to_string, calc_result_to_array_node},
    expressions::{
        parser::{ArrayNode, Node},
        token::Error,
        types::CellReferenceIndex,
    },
    formatter::format::{format_number, parse_date, parse_formatted_number},
    functions::{
        text::util::{substitute, text_after, text_before, Case},
        util::wildcard_regex_pattern,
    },
    model::Model,
    number_format::to_precision,
};

/// A text-function argument: a single scalar value or a 2-D array of values.
/// When any argument is an array (e.g. a SEQUENCE or a multi-cell range) the
/// scalar arguments are broadcast across it and the result is an array.
pub(super) enum TextArg {
    Scalar(ArrayNode),
    Array(Vec<Vec<ArrayNode>>),
}

/// Time constants for VALUE retain complete elapsed days, unlike TIMEVALUE's
/// time-of-day result. A date prefix is handled separately in the cell's locale.
fn value_time_fraction(text: &str) -> Option<f64> {
    let text = text.trim();
    let period = [("am", false), ("pm", true), (" a", false), (" p", true)]
        .into_iter()
        .find_map(|(suffix, pm)| {
            text.get(text.len().checked_sub(suffix.len())?..)
                .filter(|end| end.eq_ignore_ascii_case(suffix))
                .map(|_| (suffix.len(), pm))
        });
    let time = period.map_or(text, |(length, _)| text[..text.len() - length].trim_end());
    let parts: Vec<_> = time.split(':').collect();
    if !(1..=3).contains(&parts.len()) || (parts.len() == 1 && period.is_none()) {
        return None;
    }
    let integer = |part: &str| -> Option<u32> {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let mut hours = integer(parts[0])?;
    let minutes = parts.get(1).map_or(Some(0), |part| integer(part))?;
    let seconds = match parts.get(2) {
        None => 0.0,
        Some(part) => {
            if !part.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return None;
            }
            part.parse::<f64>().ok()?
        }
    };
    if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    if let Some((_, pm)) = period {
        if !(1..=12).contains(&hours) {
            return None;
        }
        hours = hours % 12 + if pm { 12 } else { 0 };
    }
    Some((f64::from(hours) * 3600.0 + f64::from(minutes) * 60.0 + seconds) / 86400.0)
}

/// A bounded TEXTJOIN result. Empty runs advance delimiter cycling without
/// allocating one string per unused worksheet cell.
struct TextJoinResult {
    delimiters: Vec<String>,
    delimiter_lengths: Vec<u64>,
    cycle_length: u64,
    ignore_empty: bool,
    items: u64,
    length: u64,
    text: String,
}

impl TextJoinResult {
    fn new(delimiters: Vec<String>, ignore_empty: bool) -> Self {
        let delimiter_lengths: Vec<_> = delimiters
            .iter()
            .map(|s| s.encode_utf16().count() as u64)
            .collect();
        let cycle_length = delimiter_lengths
            .iter()
            .fold(0u64, |sum, n| sum.saturating_add(*n));
        Self {
            delimiters,
            delimiter_lengths,
            cycle_length,
            ignore_empty,
            items: 0,
            length: 0,
            text: String::new(),
        }
    }

    fn delimit(&mut self, first: usize, gaps: u64) {
        let count = self.delimiters.len() as u64;
        let cycles = gaps / count;
        let tail = (gaps % count) as usize;
        let tail_length = (0..tail).fold(0u64, |sum, n| {
            sum.saturating_add(self.delimiter_lengths[(first + n) % self.delimiters.len()])
        });
        self.length = self
            .length
            .saturating_add(self.cycle_length.saturating_mul(cycles))
            .saturating_add(tail_length);
        if self.length > 32767 {
            return;
        }
        if cycles > 0 && self.cycle_length > 0 {
            let mut cycle = String::new();
            for n in 0..self.delimiters.len() {
                cycle.push_str(&self.delimiters[(first + n) % self.delimiters.len()]);
            }
            self.text.push_str(&cycle.repeat(cycles as usize));
        }
        for n in 0..tail {
            self.text
                .push_str(&self.delimiters[(first + n) % self.delimiters.len()]);
        }
    }

    fn blanks(&mut self, count: u64) {
        if self.ignore_empty || count == 0 {
            return;
        }
        let first = self.items.saturating_sub(1) % self.delimiters.len() as u64;
        self.delimit(first as usize, count - u64::from(self.items == 0));
        self.items = self.items.saturating_add(count);
    }

    fn push(&mut self, value: &str) {
        if self.ignore_empty && value.is_empty() {
            return;
        }
        if self.items > 0 {
            self.delimit(
                ((self.items - 1) % self.delimiters.len() as u64) as usize,
                1,
            );
        }
        self.items = self.items.saturating_add(1);
        self.length = self
            .length
            .saturating_add(value.encode_utf16().count() as u64);
        if self.length <= 32767 {
            self.text.push_str(value);
        }
    }

    fn value(&mut self, value: CalcResult, cell: CellReferenceIndex) -> Result<(), CalcResult> {
        match value {
            CalcResult::Number(n) => self.push(&format!("{n}")),
            CalcResult::String(s) => self.push(&s),
            CalcResult::Boolean(b) => self.push(if b { "TRUE" } else { "FALSE" }),
            CalcResult::EmptyCell => self.blanks(1),
            CalcResult::EmptyArg => {}
            error @ CalcResult::Error { .. } => return Err(error),
            CalcResult::Array(array) => {
                for item in array.into_iter().flatten() {
                    let text = array_node_to_string(&item)
                        .map_err(|error| CalcResult::new_error(error, cell, String::new()))?;
                    self.push(&text);
                }
            }
            _ => {
                return Err(CalcResult::new_error(
                    Error::NIMPL,
                    cell,
                    "Unsupported TEXTJOIN value".into(),
                ))
            }
        }
        Ok(())
    }

    fn finish(self, cell: CellReferenceIndex) -> CalcResult {
        if self.length > 32767 {
            CalcResult::new_error(
                Error::VALUE,
                cell,
                "TEXTJOIN exceeds the cell text limit".into(),
            )
        } else {
            CalcResult::String(self.text)
        }
    }
}

impl TextArg {
    /// The (rows, columns) shape of this argument.
    fn dims(&self) -> (usize, usize) {
        match self {
            TextArg::Scalar(_) => (1, 1),
            TextArg::Array(a) => (a.len(), a.first().map(|r| r.len()).unwrap_or(0)),
        }
    }

    /// Returns the element at (`i`, `j`), broadcasting scalars and length-1 dimensions.
    /// A `None` result means the dimensions are incompatible at that position.
    pub(super) fn elem(&self, i: usize, j: usize) -> Option<&ArrayNode> {
        match self {
            TextArg::Scalar(n) => Some(n),
            TextArg::Array(a) => bcast_idx(a.len(), i)
                .and_then(|ri| a.get(ri))
                .and_then(|row| bcast_idx(row.len(), j).and_then(|cj| row.get(cj))),
        }
    }
}

/// Coerces a single array element to a number for the start/length arguments of
/// LEFT/RIGHT/MID using the same number coercion as scalar function arguments.
fn text_num_arg(node: &ArrayNode, model: &Model<'_>) -> Result<f64, Error> {
    let value = match node {
        ArrayNode::Number(v) => *v,
        ArrayNode::Empty => 0.0,
        ArrayNode::Error(e) => return Err(e.clone()),
        ArrayNode::Boolean(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        ArrayNode::String(text) => model.cast_number(text).ok_or(Error::VALUE)?,
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::VALUE)
    }
}

/// Computes a single LEFT result from one element of each argument.
fn left_element(
    text: Option<&ArrayNode>,
    num: Option<&ArrayNode>,
    model: &Model<'_>,
) -> Result<ArrayNode, Error> {
    let (text, num) = match (text, num) {
        (Some(t), Some(n)) => (t, n),
        // A broadcast hole (incompatible dimensions) yields #N/A, as in Excel.
        _ => return Err(Error::NA),
    };
    let s = array_node_to_string(text)?;
    let num = text_num_arg(num, model)?;
    if num < 0.0 {
        return Err(Error::VALUE);
    }
    let num_chars = num.floor() as usize;
    Ok(ArrayNode::String(s.chars().take(num_chars).collect()))
}

/// Computes a single RIGHT result from one element of each argument.
fn right_element(
    text: Option<&ArrayNode>,
    num: Option<&ArrayNode>,
    model: &Model<'_>,
) -> Result<ArrayNode, Error> {
    let (text, num) = match (text, num) {
        (Some(t), Some(n)) => (t, n),
        _ => return Err(Error::NA),
    };
    let s = array_node_to_string(text)?;
    let num = text_num_arg(num, model)?;
    if num < 0.0 {
        return Err(Error::VALUE);
    }
    let num_chars = num.floor() as usize;
    let skip = s.chars().count().saturating_sub(num_chars);
    Ok(ArrayNode::String(s.chars().skip(skip).collect()))
}

/// Computes a single MID result from one element of each argument.
fn mid_element(
    text: Option<&ArrayNode>,
    start: Option<&ArrayNode>,
    length: Option<&ArrayNode>,
    model: &Model<'_>,
) -> Result<ArrayNode, Error> {
    let (text, start, length) = match (text, start, length) {
        (Some(t), Some(s), Some(l)) => (t, s, l),
        _ => return Err(Error::NA),
    };
    let s = array_node_to_string(text)?;
    let start = text_num_arg(start, model)?;
    let length = text_num_arg(length, model)?;
    if start < 1.0 {
        return Err(Error::VALUE);
    }
    if length < 0.0 {
        return Err(Error::VALUE);
    }
    let start_num = start.floor() as usize;
    let num_chars = length.floor() as usize;
    let mut result = String::new();
    let mut count: usize = 0;
    for (index, ch) in s.chars().enumerate() {
        if count >= num_chars {
            break;
        }
        if index + 1 >= start_num {
            result.push(ch);
            count += 1;
        }
    }
    Ok(ArrayNode::String(result))
}

/// Broadcasts `compute` over the (possibly array) `operands`. When every operand
/// is a scalar the result is a single value; otherwise it is an array whose shape
/// is the element-wise maximum of the operand shapes.
pub(super) fn broadcast_text(
    cell: CellReferenceIndex,
    operands: &[&TextArg],
    mut compute: impl FnMut(usize, usize) -> Result<ArrayNode, Error>,
) -> CalcResult {
    let mut rows = 1;
    let mut cols = 1;
    for operand in operands {
        let (r, c) = operand.dims();
        rows = rows.max(r);
        cols = cols.max(c);
    }
    if let Some((error, message)) = crate::functions::check_array_size(rows, cols) {
        return CalcResult::new_error(error, cell, message);
    }

    // All scalars: return a single value.
    if operands.iter().all(|arg| matches!(arg, TextArg::Scalar(_))) {
        return match compute(0, 0) {
            Ok(ArrayNode::String(s)) => CalcResult::String(s),
            Ok(ArrayNode::Number(n)) => CalcResult::Number(n),
            Ok(ArrayNode::Boolean(b)) => CalcResult::Boolean(b),
            Ok(ArrayNode::Empty) => CalcResult::String(String::new()),
            Ok(ArrayNode::Error(e)) | Err(e) => CalcResult::new_error(e, cell, String::new()),
        };
    }

    let mut result = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut data_row = Vec::with_capacity(cols);
        for j in 0..cols {
            data_row.push(compute(i, j).unwrap_or_else(ArrayNode::Error));
        }
        result.push(data_row);
    }
    CalcResult::Array(result)
}

/// Finds the first instance of 'search_for' in text starting at char index start
fn find(search_for: &str, text: &str, start: usize) -> Option<i32> {
    let ch = text.chars();
    let mut byte_index = 0;
    for (char_index, c) in ch.enumerate() {
        if char_index + 1 >= start && text[byte_index..].starts_with(search_for) {
            return Some((char_index + 1) as i32);
        }
        byte_index += c.len_utf8();
    }
    None
}

/// You can use the wildcard characters — the question mark (?) and asterisk (*) — in the find_text argument.
/// * A question mark matches any single character.
/// * An asterisk matches any sequence of characters.
/// * If you want to find an actual question mark or asterisk, type a tilde (~) before the character.
fn search(re: &regex::Regex, text: &str, start: usize) -> Option<i32> {
    let byte_index = text.char_indices().nth(start.checked_sub(1)?)?.0;
    let matched = re.find(&text[byte_index..])?;
    Some(text[..byte_index + matched.start()].chars().count() as i32 + 1)
}

impl<'a> Model<'a> {
    pub(crate) fn fn_concat(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=253).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let mut result = TextJoinResult::new(vec![String::new()], true);
        for arg in args {
            let value = self.evaluate_node_in_context(arg, cell);
            let status = match value {
                CalcResult::Range { left, right } => {
                    self.textjoin_range(&mut result, left, right, cell)
                }
                value => result.value(value, cell),
            };
            if let Err(error) = status {
                return error;
            }
        }
        result.finish(cell)
    }
    pub(crate) fn fn_text(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let values = match self.text_arg(&args[0], cell) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let formats = match self.text_arg(&args[1], cell) {
            Ok(formats) => formats,
            Err(error) => return error,
        };
        broadcast_text(cell, &[&values, &formats], |i, j| {
            let (Some(value), Some(format)) = (values.elem(i, j), formats.elem(i, j)) else {
                return Err(Error::NA);
            };
            if let ArrayNode::Error(error) = value {
                return Err(error.clone());
            }
            let code = array_node_to_string(format)?;
            let number = match value {
                ArrayNode::Number(number) => *number,
                ArrayNode::Empty => 0.0,
                ArrayNode::String(text) => match self.cast_number(text) {
                    Some(number) if number.is_finite() => number,
                    _ => return Ok(ArrayNode::String(text.clone())),
                },
                ArrayNode::Boolean(value) => {
                    return Ok(ArrayNode::String(
                        if *value { "TRUE" } else { "FALSE" }.into(),
                    ))
                }
                ArrayNode::Error(error) => return Err(error.clone()),
            };
            let formatted = format_number(number, &code, self.locale);
            if formatted.error.is_some() || formatted.text.encode_utf16().count() > 32767 {
                Err(Error::VALUE)
            } else {
                Ok(ArrayNode::String(formatted.text))
            }
        })
    }
    /// FIND(find_text, within_text, [start_num])
    ///  * FIND and FINDB are case sensitive and don't allow wildcard characters.
    ///  * If find_text is "" (empty text), FIND matches the first character in the search string (that is, the character numbered start_num or 1).
    ///  * Find_text cannot contain any wildcard characters.
    ///  * If find_text does not appear in within_text, FIND and FINDB return the #VALUE! error value.
    ///  * If start_num is not greater than zero, FIND and FINDB return the #VALUE! error value.
    ///  * If start_num is greater than the length of within_text, FIND and FINDB return the #VALUE! error value.
    ///    NB: FINDB is not implemented. It is the same as FIND function unless locale is a DBCS (Double Byte Character Set)
    pub(crate) fn fn_find(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.text_search(args, cell, false)
    }

    fn text_search(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        wildcard: bool,
    ) -> CalcResult {
        if args.len() < 2 || args.len() > 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(3);
        for arg in args {
            match self.text_arg(arg, cell) {
                Ok(value) => operands.push(value),
                Err(error) => return error,
            }
        }
        let refs: Vec<_> = operands.iter().collect();
        // Reuse common patterns within one formula evaluation. Keep the cache
        // bounded when the search argument itself contains many distinct texts.
        let mut patterns = std::collections::HashMap::<String, Option<regex::Regex>>::new();
        broadcast_text(cell, &refs, |i, j| {
            let values: Option<Vec<_>> = operands.iter().map(|arg| arg.elem(i, j)).collect();
            let values = values.ok_or(Error::NA)?;
            let needle = array_node_to_string(values[0])?;
            let text = array_node_to_string(values[1])?;
            let start = if let Some(value) = values.get(2) {
                match value {
                    ArrayNode::Number(n) => *n,
                    ArrayNode::Boolean(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    ArrayNode::Empty => 0.0,
                    ArrayNode::String(s) => self.cast_number(s).ok_or(Error::VALUE)?,
                    ArrayNode::Error(error) => return Err(error.clone()),
                }
                .floor()
            } else {
                1.0
            };
            if !start.is_finite() || start < 1.0 || start > text.chars().count() as f64 {
                return Err(Error::VALUE);
            }
            let position = if wildcard {
                if patterns.len() >= 8 && !patterns.contains_key(&needle) {
                    patterns.clear();
                }
                let regex = patterns.entry(needle.clone()).or_insert_with(|| {
                    regex::RegexBuilder::new(&wildcard_regex_pattern(&needle, false))
                        .case_insensitive(true)
                        .dot_matches_new_line(true)
                        .build()
                        .ok()
                });
                regex
                    .as_ref()
                    .and_then(|regex| search(regex, &text, start as usize))
            } else {
                find(&needle, &text, start as usize)
            };
            position
                .map(|position| ArrayNode::Number(position as f64))
                .ok_or(Error::VALUE)
        })
    }
    /// Same API as FIND but:
    ///  * Allows wildcards
    ///  * It is case insensitive
    ///    SEARCH(find_text, within_text, [start_num])
    pub(crate) fn fn_search(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.text_search(args, cell, true)
    }
    // LEN, LEFT, RIGHT, MID, LOWER, UPPER, TRIM
    pub(crate) fn fn_len(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |_, value| {
            let s = match value {
                CalcResult::Number(v) => format!("{v}"),
                CalcResult::String(v) => v,
                CalcResult::Boolean(b) => {
                    if b {
                        "TRUE".to_string()
                    } else {
                        "FALSE".to_string()
                    }
                }
                error @ CalcResult::Error { .. } => return error,
                CalcResult::EmptyCell | CalcResult::EmptyArg => "".to_string(),
                CalcResult::Range { .. } | CalcResult::Array(_) | CalcResult::Lambda(_) => {
                    return CalcResult::Error {
                        error: Error::NIMPL,
                        origin: cell,
                        message: "Arrays not supported yet".to_string(),
                    }
                }
            };
            CalcResult::Number(s.chars().count() as f64)
        })
    }

    pub(crate) fn fn_trim(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let text = match model.cast_to_string(value, cell) {
                Ok(text) => text,
                Err(error) => return error,
            };
            // Excel trims ASCII spaces only; tabs, NBSP and other Unicode
            // whitespace remain part of the text.
            CalcResult::String(
                text.split(' ')
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
    }
    pub(crate) fn fn_lower(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.apply_text_unary(args, cell, |s| s.to_lowercase())
    }

    /// Applies a single-argument string transform element-wise.
    /// A scalar argument yields a single String; a range or array argument yields
    /// an array (which spills, or is consumed element-wise in array contexts such
    /// as SUMPRODUCT). Used by text functions like UPPER and LOWER.
    pub(crate) fn apply_text_unary(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        f: impl Fn(&str) -> String,
    ) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            match model.cast_to_string(value, cell) {
                Ok(text) => {
                    let text = f(&text);
                    if text.encode_utf16().count() > 32767 {
                        CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Text exceeds the cell text limit".into(),
                        )
                    } else {
                        CalcResult::String(text)
                    }
                }
                Err(error) => error,
            }
        })
    }
    pub(crate) fn fn_unicode(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let text = match model.cast_to_string(value, cell) {
                Ok(text) => text,
                Err(error) => return error,
            };
            match text.chars().next() {
                Some(character) => CalcResult::Number(character as u32 as f64),
                None => CalcResult::new_error(Error::VALUE, cell, "Empty text".into()),
            }
        })
    }
    pub(crate) fn fn_upper(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.apply_text_unary(args, cell, |s| s.to_uppercase())
    }

    /// Evaluates a LEFT/RIGHT/MID argument into a scalar value or a 2-D array of
    /// values. References to a single cell collapse to a scalar; multi-cell ranges
    /// and array literals/spills become arrays that the result is broadcast over.
    pub(super) fn text_arg(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> Result<TextArg, CalcResult> {
        match self.evaluate_node_in_context(node, cell) {
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            } => Err(error),
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "3D ranges are not allowed".into(),
                    ));
                }
                let rows = (i64::from(right.row) - i64::from(left.row) + 1).max(0) as usize;
                let columns =
                    (i64::from(right.column) - i64::from(left.column) + 1).max(0) as usize;
                if let Some((error, message)) = crate::functions::check_array_size(rows, columns) {
                    return Err(CalcResult::new_error(error, cell, message));
                }
                Ok(TextArg::Array(self.evaluate_range(left, right)))
            }
            CalcResult::Array(a) => Ok(TextArg::Array(a)),
            CalcResult::Lambda(_) => Err(CalcResult::new_error(
                Error::VALUE,
                cell,
                "Expecting a value".to_string(),
            )),
            other => Ok(TextArg::Scalar(calc_result_to_array_node(other))),
        }
    }

    pub(crate) fn fn_left(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() > 2 || args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.text_arg(&args[0], cell) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let num = if args.len() == 2 {
            match self.text_arg(&args[1], cell) {
                Ok(o) => o,
                Err(e) => return e,
            }
        } else {
            TextArg::Scalar(ArrayNode::Number(1.0))
        };
        broadcast_text(cell, &[&text, &num], |i, j| {
            left_element(text.elem(i, j), num.elem(i, j), self)
        })
    }

    pub(crate) fn fn_right(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() > 2 || args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.text_arg(&args[0], cell) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let num = if args.len() == 2 {
            match self.text_arg(&args[1], cell) {
                Ok(o) => o,
                Err(e) => return e,
            }
        } else {
            TextArg::Scalar(ArrayNode::Number(1.0))
        };
        broadcast_text(cell, &[&text, &num], |i, j| {
            right_element(text.elem(i, j), num.elem(i, j), self)
        })
    }

    pub(crate) fn fn_mid(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.text_arg(&args[0], cell) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let start = match self.text_arg(&args[1], cell) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let length = match self.text_arg(&args[2], cell) {
            Ok(o) => o,
            Err(e) => return e,
        };
        broadcast_text(cell, &[&text, &start, &length], |i, j| {
            mid_element(text.elem(i, j), start.elem(i, j), length.elem(i, j), self)
        })
    }

    // REPT(text, number_times)
    pub(crate) fn fn_rept(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.text_arg(&args[0], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let number_times = match self.text_arg(&args[1], cell) {
            Ok(value) => value,
            Err(s) => return s,
        };
        broadcast_text(cell, &[&text, &number_times], |i, j| {
            let (Some(text), Some(times)) = (text.elem(i, j), number_times.elem(i, j)) else {
                return Err(Error::NA);
            };
            let text = array_node_to_string(text)?;
            let times = match times {
                ArrayNode::Number(n) => *n,
                ArrayNode::Boolean(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                ArrayNode::Empty => 0.0,
                ArrayNode::String(s) => self.cast_number(s).ok_or(Error::VALUE)?,
                ArrayNode::Error(error) => return Err(error.clone()),
            };
            if !times.is_finite() || times < 0.0 {
                return Err(Error::VALUE);
            }
            let times = times.trunc();
            let units = text.encode_utf16().count();
            if times == 0.0 || units == 0 {
                return Ok(ArrayNode::String(String::new()));
            }
            // Compare before narrowing or multiplying: very large counts must
            // return an Excel error instead of overflowing or allocating.
            if times > (32767 / units) as f64 {
                return Err(Error::VALUE);
            }
            Ok(ArrayNode::String(text.repeat(times as usize)))
        })
    }

    // TEXTAFTER(text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found])
    pub(crate) fn fn_textafter(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if !(2..=6).contains(&arg_count) {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.get_string(&args[0], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let delimiter = match self.get_string(&args[1], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let instance_num = if arg_count > 2 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f.floor() as i32,
                Err(s) => return s,
            }
        } else {
            1
        };
        let match_mode = if arg_count > 3 {
            match self.get_number(&args[3], cell) {
                Ok(f) => {
                    if f == 0.0 {
                        Case::Sensitive
                    } else {
                        Case::Insensitive
                    }
                }
                Err(s) => return s,
            }
        } else {
            Case::Sensitive
        };

        let match_end = if arg_count > 4 {
            match self.get_number(&args[4], cell) {
                Ok(f) => f,
                Err(s) => return s,
            }
        } else {
            // disabled by default
            // the delimiter is specified in the formula
            0.0
        };
        if instance_num == 0 {
            return CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "instance_num must be <> 0".to_string(),
            };
        }
        if delimiter.len() > text.len() {
            // so this is fun(!)
            // if the function was provided with two arguments is a #VALUE!
            // if it had more is a #N/A (irrespective of their values)
            if arg_count > 2 {
                return CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "The delimiter is longer than the text is trying to match".to_string(),
                };
            } else {
                return CalcResult::Error {
                    error: Error::NA,
                    origin: cell,
                    message: "The delimiter is longer than the text is trying to match".to_string(),
                };
            }
        }
        if match_end != 0.0 && match_end != 1.0 {
            return CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "argument must be 0 or 1".to_string(),
            };
        };
        match text_after(&text, &delimiter, instance_num, match_mode) {
            Some(s) => CalcResult::String(s),
            None => {
                if match_end == 1.0 {
                    if instance_num == 1 {
                        return CalcResult::String("".to_string());
                    } else if instance_num == -1 {
                        return CalcResult::String(text);
                    }
                }
                if arg_count == 6 {
                    // An empty cell is converted to empty string (not 0)
                    match self.evaluate_node_in_context(&args[5], cell) {
                        CalcResult::EmptyCell => CalcResult::String("".to_string()),
                        result => result,
                    }
                } else {
                    CalcResult::Error {
                        error: Error::NA,
                        origin: cell,
                        message: "Value not found".to_string(),
                    }
                }
            }
        }
    }

    pub(crate) fn fn_textbefore(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if !(2..=6).contains(&arg_count) {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.get_string(&args[0], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let delimiter = match self.get_string(&args[1], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let instance_num = if arg_count > 2 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f.floor() as i32,
                Err(s) => return s,
            }
        } else {
            1
        };
        let match_mode = if arg_count > 3 {
            match self.get_number(&args[3], cell) {
                Ok(f) => {
                    if f == 0.0 {
                        Case::Sensitive
                    } else {
                        Case::Insensitive
                    }
                }
                Err(s) => return s,
            }
        } else {
            Case::Sensitive
        };

        let match_end = if arg_count > 4 {
            match self.get_number(&args[4], cell) {
                Ok(f) => f,
                Err(s) => return s,
            }
        } else {
            // disabled by default
            // the delimiter is specified in the formula
            0.0
        };
        if instance_num == 0 {
            return CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "instance_num must be <> 0".to_string(),
            };
        }
        if delimiter.len() > text.len() {
            // so this is fun(!)
            // if the function was provided with two arguments is a #VALUE!
            // if it had more is a #N/A (irrespective of their values)
            if arg_count > 2 {
                return CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "The delimiter is longer than the text is trying to match".to_string(),
                };
            } else {
                return CalcResult::Error {
                    error: Error::NA,
                    origin: cell,
                    message: "The delimiter is longer than the text is trying to match".to_string(),
                };
            }
        }
        if match_end != 0.0 && match_end != 1.0 {
            return CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "argument must be 0 or 1".to_string(),
            };
        };
        match text_before(&text, &delimiter, instance_num, match_mode) {
            Some(s) => CalcResult::String(s),
            None => {
                if match_end == 1.0 {
                    if instance_num == -1 {
                        return CalcResult::String("".to_string());
                    } else if instance_num == 1 {
                        return CalcResult::String(text);
                    }
                }
                if arg_count == 6 {
                    // An empty cell is converted to empty string (not 0)
                    match self.evaluate_node_in_context(&args[5], cell) {
                        CalcResult::EmptyCell => CalcResult::String("".to_string()),
                        result => result,
                    }
                } else {
                    CalcResult::Error {
                        error: Error::NA,
                        origin: cell,
                        message: "Value not found".to_string(),
                    }
                }
            }
        }
    }

    fn textjoin_delimiters(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<String>, CalcResult> {
        let array = match self.evaluate_node_in_context(node, cell) {
            CalcResult::Array(array) => array,
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "3D delimiter ranges are not allowed".into(),
                    ));
                }
                let rows = (i64::from(right.row) - i64::from(left.row) + 1).max(0) as usize;
                let columns =
                    (i64::from(right.column) - i64::from(left.column) + 1).max(0) as usize;
                if let Some((error, message)) = crate::functions::check_array_size(rows, columns) {
                    return Err(CalcResult::new_error(error, cell, message));
                }
                self.evaluate_range(left, right)
            }
            scalar => return self.cast_to_string(scalar, cell).map(|text| vec![text]),
        };
        let mut delimiters = Vec::new();
        for item in array.into_iter().flatten() {
            delimiters.push(
                array_node_to_string(&item)
                    .map_err(|error| CalcResult::new_error(error, cell, String::new()))?,
            );
        }
        if delimiters.is_empty() {
            delimiters.push(String::new());
        }
        Ok(delimiters)
    }

    // TEXTJOIN(delimiter, ignore_empty, text1, [text2], …)
    fn textjoin_range_cells(
        &self,
        left: CellReferenceIndex,
        right: CellReferenceIndex,
        cell: CellReferenceIndex,
    ) -> Result<Vec<CellReferenceIndex>, CalcResult> {
        let sheet = self
            .workbook
            .worksheet(left.sheet)
            .map_err(|message| CalcResult::new_error(Error::ERROR, cell, message))?;
        let mut cells = Vec::new();
        for (&row, columns) in &sheet.sheet_data {
            if row < left.row || row > right.row {
                continue;
            }
            for &column in columns.keys() {
                if column >= left.column && column <= right.column {
                    cells.push(CellReferenceIndex {
                        sheet: left.sheet,
                        row,
                        column,
                    });
                }
            }
        }
        cells.sort_unstable_by_key(|at| (at.row, at.column));
        Ok(cells)
    }

    fn textjoin_range(
        &mut self,
        result: &mut TextJoinResult,
        left: CellReferenceIndex,
        right: CellReferenceIndex,
        cell: CellReferenceIndex,
    ) -> Result<(), CalcResult> {
        if left.sheet != right.sheet {
            return Err(CalcResult::new_error(
                Error::VALUE,
                cell,
                "Ranges are in different sheets".into(),
            ));
        }
        // Source formulas can create/remove spill members while evaluated.
        // Materialize them before taking the sparse row-major snapshot.
        for at in self.textjoin_range_cells(left, right, cell)? {
            let _ = self.evaluate_cell(at);
        }
        let columns = (i64::from(right.column) - i64::from(left.column) + 1).max(0) as u64;
        let rows = (i64::from(right.row) - i64::from(left.row) + 1).max(0) as u64;
        let mut next = 0;
        for at in self.textjoin_range_cells(left, right, cell)? {
            let offset = (at.row - left.row) as u64 * columns + (at.column - left.column) as u64;
            result.blanks(offset.saturating_sub(next));
            result.value(self.evaluate_cell(at), cell)?;
            next = offset + 1;
        }
        result.blanks(rows.saturating_mul(columns).saturating_sub(next));
        Ok(())
    }

    pub(crate) fn fn_textjoin(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(3..=254).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let delimiters = match self.textjoin_delimiters(&args[0], cell) {
            Ok(s) => s,
            Err(error) => return error,
        };
        let ignore_empty = match self.get_boolean(&args[1], cell) {
            Ok(b) => b,
            Err(error) => return error,
        };
        let mut result = TextJoinResult::new(delimiters, ignore_empty);
        for arg in &args[2..] {
            let value = self.evaluate_node_in_context(arg, cell);
            let status = match value {
                CalcResult::Range { left, right } => {
                    self.textjoin_range(&mut result, left, right, cell)
                }
                value => result.value(value, cell),
            };
            if let Err(error) = status {
                return error;
            }
        }
        result.finish(cell)
    }

    // SUBSTITUTE(text, old_text, new_text, [instance_num])
    pub(crate) fn fn_substitute(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(3..=4).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(args.len());
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
            let text = array_node_to_string(values[0])?;
            let old = array_node_to_string(values[1])?;
            let new = array_node_to_string(values[2])?;
            let instance = if let Some(value) = values.get(3) {
                let number = match value {
                    ArrayNode::Number(n) => *n,
                    ArrayNode::Boolean(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    ArrayNode::Empty => 0.0,
                    ArrayNode::String(s) => self.cast_number(s).ok_or(Error::VALUE)?,
                    ArrayNode::Error(error) => return Err(error.clone()),
                }
                .floor();
                if !number.is_finite() || number < 1.0 {
                    return Err(Error::VALUE);
                }
                Some(number)
            } else {
                None
            };
            substitute(&text, &old, &new, instance).map(ArrayNode::String)
        })
    }
    pub(crate) fn fn_concatenate(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=255).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let mut operands = Vec::with_capacity(args.len());
        for arg in args {
            match self.text_arg(arg, cell) {
                Ok(value) => operands.push(value),
                Err(error) => return error,
            }
        }
        let refs: Vec<_> = operands.iter().collect();
        broadcast_text(cell, &refs, |i, j| {
            let mut result = String::new();
            let mut units = 0usize;
            for operand in &operands {
                let value = operand.elem(i, j).ok_or(Error::NA)?;
                let text = array_node_to_string(value)?;
                units = units.saturating_add(text.encode_utf16().count());
                if units > 32767 {
                    return Err(Error::VALUE);
                }
                result.push_str(&text);
            }
            Ok(ArrayNode::String(result))
        })
    }
    pub(crate) fn fn_exact(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let left = match self.text_arg(&args[0], cell) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let right = match self.text_arg(&args[1], cell) {
            Ok(value) => value,
            Err(error) => return error,
        };
        broadcast_text(cell, &[&left, &right], |i, j| {
            let (Some(left), Some(right)) = (left.elem(i, j), right.elem(i, j)) else {
                return Err(Error::NA);
            };
            let equal = if let (ArrayNode::Number(left), ArrayNode::Number(right)) = (left, right) {
                // Retain the scalar comparison's 15-digit numeric precision.
                to_precision(*left, 15) == to_precision(*right, 15)
            } else {
                array_node_to_string(left)? == array_node_to_string(right)?
            };
            Ok(ArrayNode::Boolean(equal))
        })
    }
    // VALUE(text)
    pub(crate) fn fn_value(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| match value {
            CalcResult::String(text) => {
                let currency = model.locale.currency.symbol.as_str();
                let mut currencies = vec!["$", "€"];
                if !currencies.contains(&currency) {
                    currencies.push(currency);
                }
                if let Ok((value, _)) = parse_formatted_number(&text, &currencies, model.locale) {
                    if value.is_finite() {
                        return CalcResult::Number(value);
                    }
                };
                if let Some(time) = value_time_fraction(&text) {
                    return CalcResult::Number(time);
                }
                let text = text.trim();
                for (at, separator) in text.char_indices() {
                    if !matches!(separator, ' ' | 'T') {
                        continue;
                    }
                    if let (Ok((day, _)), Some(time)) = (
                        parse_date(text[..at].trim_end(), model.locale),
                        value_time_fraction(text[at + separator.len_utf8()..].trim_start()),
                    ) {
                        if time < 1.0 {
                            return CalcResult::Number(f64::from(day) + time);
                        }
                    }
                }
                CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid number".to_string(),
                }
            }
            CalcResult::Number(f) => CalcResult::Number(f),
            CalcResult::Boolean(_) => CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid number".to_string(),
            },
            error @ CalcResult::Error { .. } => error,
            CalcResult::EmptyCell | CalcResult::EmptyArg => CalcResult::Number(0.0),
            CalcResult::Range { .. } | CalcResult::Array(_) | CalcResult::Lambda(_) => {
                CalcResult::Error {
                    error: Error::NIMPL,
                    origin: cell,
                    message: "Arrays not supported yet".to_string(),
                }
            }
        })
    }

    pub(crate) fn fn_t(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |_, value| match value {
            CalcResult::String(_) | CalcResult::Error { .. } => value,
            _ => CalcResult::String("".to_string()),
        })
    }

    // VALUETOTEXT(value, [format])
    pub(crate) fn fn_valuetotext(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let format = if args.len() == 2 {
            match self.get_number(&args[1], cell) {
                Ok(format) => format,
                Err(error) => return error,
            }
        } else {
            0.0
        };
        if format != 0.0 && format != 1.0 {
            return CalcResult::new_error(Error::VALUE, cell, "format must be 0 or 1".into());
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let text = match value {
                CalcResult::String(text) if format == 1.0 => {
                    format!("\"{}\"", text.replace('"', "\"\""))
                }
                CalcResult::Error { error, .. } => error.to_string(),
                value => match model.cast_to_string(value, cell) {
                    Ok(text) => text,
                    Err(error) => return error,
                },
            };
            if text.encode_utf16().count() > 32767 {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "VALUETOTEXT exceeds the cell text limit".into(),
                );
            }
            CalcResult::String(text)
        })
    }
}
