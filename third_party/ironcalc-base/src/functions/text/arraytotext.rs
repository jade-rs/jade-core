// DOLLAR, FIXED, NUMBERVALUE, PROPER, REPLACE, ARRAYTOTEXT

use crate::{
    calc_result::CalcResult,
    expressions::{parser::ArrayNode, parser::Node, token::Error, types::CellReferenceIndex},
    model::Model,
};

impl<'a> Model<'a> {
    /// ARRAYTOTEXT(array, [format])
    /// format=0 (default, concise): values joined with ", " in row order
    /// format=1 (strict): same but wrapped in {} and strings are quoted
    pub(crate) fn fn_arraytotext(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() || args.len() > 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let format = if args.len() == 2 {
            match self.get_number(&args[1], cell) {
                Ok(f) => f,
                Err(e) => return e,
            }
        } else {
            0.0
        };
        if format != 0.0 && format != 1.0 {
            return CalcResult::new_error(Error::VALUE, cell, "format must be 0 or 1".to_string());
        }
        let strict = format == 1.0;

        let array = match self.evaluate_node_in_context(&args[0], cell) {
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "3D ranges are unsupported".into(),
                    );
                }
                let mut output = ArrayText::new(strict);
                for row in left.row..=right.row {
                    for column in left.column..=right.column {
                        let value = self.evaluate_cell(CellReferenceIndex {
                            sheet: left.sheet,
                            row,
                            column,
                        });
                        let value = match value {
                            CalcResult::Number(n) => ArrayNode::Number(n),
                            CalcResult::String(s) => ArrayNode::String(s),
                            CalcResult::Boolean(b) => ArrayNode::Boolean(b),
                            CalcResult::Error { error, .. } => ArrayNode::Error(error),
                            CalcResult::EmptyCell | CalcResult::EmptyArg => ArrayNode::Empty,
                            _ => {
                                return CalcResult::new_error(
                                    Error::VALUE,
                                    cell,
                                    "Invalid array value".into(),
                                )
                            }
                        };
                        if !output.push(&value, column == left.column && row != left.row) {
                            return text_limit_error(cell);
                        }
                    }
                }
                return output.finish(cell);
            }
            CalcResult::Array(a) => a,
            CalcResult::Number(n) => {
                return CalcResult::String(format_array_value(&ArrayNode::Number(n), strict));
            }
            CalcResult::String(s) => {
                let text = if strict {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                };
                return checked_text(text, cell);
            }
            CalcResult::Boolean(b) => {
                return CalcResult::String(if b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                });
            }
            CalcResult::EmptyCell | CalcResult::EmptyArg => {
                return CalcResult::String(String::new());
            }
            error @ CalcResult::Error { .. } => return error,
            CalcResult::Lambda(_) => {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Lambda not supported".to_string(),
                );
            }
        };

        let mut output = ArrayText::new(strict);
        for (row_index, row) in array.iter().enumerate() {
            for (column_index, value) in row.iter().enumerate() {
                if !output.push(value, row_index != 0 && column_index == 0) {
                    return text_limit_error(cell);
                }
            }
        }
        output.finish(cell)
    }
}

fn text_limit_error(cell: CellReferenceIndex) -> CalcResult {
    CalcResult::new_error(
        Error::VALUE,
        cell,
        "ARRAYTOTEXT exceeds the cell text limit".into(),
    )
}

fn checked_text(text: String, cell: CellReferenceIndex) -> CalcResult {
    if text.encode_utf16().count() > 32767 {
        text_limit_error(cell)
    } else {
        CalcResult::String(text)
    }
}

struct ArrayText {
    text: String,
    units: usize,
    strict: bool,
    first: bool,
}

impl ArrayText {
    fn new(strict: bool) -> Self {
        Self {
            text: if strict { "{".into() } else { String::new() },
            units: usize::from(strict),
            strict,
            first: true,
        }
    }

    fn push(&mut self, value: &ArrayNode, new_row: bool) -> bool {
        let separator = if self.first {
            ""
        } else if !self.strict {
            ", "
        } else if new_row {
            ";"
        } else {
            ","
        };
        let text = format_array_value(value, self.strict);
        let units = self.units + separator.len() + text.encode_utf16().count();
        // Reserve the closing brace while building strict output.
        if units + usize::from(self.strict) > 32767 {
            return false;
        }
        self.text.push_str(separator);
        self.text.push_str(&text);
        self.units = units;
        self.first = false;
        true
    }

    fn finish(mut self, cell: CellReferenceIndex) -> CalcResult {
        if self.strict {
            self.text.push('}');
        }
        checked_text(self.text, cell)
    }
}

fn format_array_value(node: &ArrayNode, strict: bool) -> String {
    match node {
        ArrayNode::Number(n) => {
            if *n == n.floor() && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        ArrayNode::Boolean(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        ArrayNode::String(s) => {
            if strict {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        ArrayNode::Error(e) => format!("{}", e),
        ArrayNode::Empty => String::new(),
    }
}
