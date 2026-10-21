//! Word's `=` (formula) field: arithmetic and comparison over numbers,
//! bookmarks and table cells, with Word's functions (ABS, AND, AVERAGE,
//! COUNT, DEFINED, FALSE, IF, INT, MAX, MIN, MOD, NOT, OR, PRODUCT, ROUND,
//! SIGN, SUM, TRUE) and its positional table arguments (ABOVE, BELOW,
//! LEFT, RIGHT) and cell references (A1, B2:C4, R1C2).

use super::scan::{CellRef, ScannedTable};
use jade_docx_reader::field_format::parse_field_number;

/// Why a formula has no value, as Word reports it in place of the result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FormulaError {
    Syntax(String),
    ZeroDivide,
    UndefinedBookmark(String),
    UnexpectedEnd,
}

impl FormulaError {
    pub fn message(&self, japanese: bool) -> String {
        match (self, japanese) {
            (FormulaError::Syntax(token), false) => format!("!Syntax Error, {token}"),
            (FormulaError::Syntax(token), true) => format!("!構文エラー、{token}"),
            (FormulaError::ZeroDivide, false) => "!Zero Divide".to_string(),
            (FormulaError::ZeroDivide, true) => "!0 で除算しました".to_string(),
            (FormulaError::UndefinedBookmark(name), false) => {
                format!("!Undefined Bookmark, {name}")
            }
            (FormulaError::UndefinedBookmark(name), true) => {
                format!("!ブックマークが定義されていません、{name}")
            }
            (FormulaError::UnexpectedEnd, false) => "!Unexpected End of Formula".to_string(),
            (FormulaError::UnexpectedEnd, true) => "!数式の終わりが不正です".to_string(),
        }
    }
}

/// What a formula can see: the table it sits in, its cell, and bookmarks.
pub(crate) struct FormulaContext<'a> {
    pub table: Option<&'a ScannedTable>,
    pub cell: Option<CellRef>,
    pub bookmark: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Text(String),
    Operator(&'static str),
    Open,
    Close,
    Separator,
    Colon,
}

fn tokenize(expression: &str) -> Result<Vec<Token>, FormulaError> {
    let chars: Vec<char> = expression.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        if character.is_ascii_digit()
            || (character == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit))
        {
            let start = index;
            while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
                index += 1;
            }
            // A cell reference like 1:1 is not a number followed by a range.
            let text: String = chars[start..index].iter().collect();
            tokens.push(Token::Number(
                text.parse()
                    .map_err(|_| FormulaError::Syntax(text.clone()))?,
            ));
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            tokens.push(Token::Name(chars[start..index].iter().collect()));
            continue;
        }
        if character == '"' {
            let mut text = String::new();
            index += 1;
            while index < chars.len() && chars[index] != '"' {
                text.push(chars[index]);
                index += 1;
            }
            index += 1;
            tokens.push(Token::Text(text));
            continue;
        }
        let two: String = chars[index..(index + 2).min(chars.len())].iter().collect();
        let operator = match two.as_str() {
            "<=" => Some("<="),
            ">=" => Some(">="),
            "<>" => Some("<>"),
            _ => None,
        };
        if let Some(operator) = operator {
            tokens.push(Token::Operator(operator));
            index += 2;
            continue;
        }
        tokens.push(match character {
            '+' => Token::Operator("+"),
            '-' | '−' => Token::Operator("-"),
            '*' | '×' => Token::Operator("*"),
            '/' | '÷' => Token::Operator("/"),
            '^' => Token::Operator("^"),
            '%' => Token::Operator("%"),
            '=' => Token::Operator("="),
            '<' => Token::Operator("<"),
            '>' => Token::Operator(">"),
            '(' => Token::Open,
            ')' => Token::Close,
            ',' | ';' => Token::Separator,
            ':' => Token::Colon,
            other => return Err(FormulaError::Syntax(other.to_string())),
        });
        index += 1;
    }
    Ok(tokens)
}

/// Evaluates a formula's expression.
pub(crate) fn evaluate_formula(
    expression: &str,
    context: &FormulaContext<'_>,
) -> Result<f64, FormulaError> {
    let tokens = tokenize(expression)?;
    if tokens.is_empty() {
        return Err(FormulaError::UnexpectedEnd);
    }
    let mut parser = Parser {
        tokens,
        position: 0,
        context,
    };
    let value = parser.comparison()?;
    if let Some(token) = parser.tokens.get(parser.position) {
        return Err(FormulaError::Syntax(token_text(token)));
    }
    Ok(value)
}

fn token_text(token: &Token) -> String {
    match token {
        Token::Number(number) => number.to_string(),
        Token::Name(name) => name.clone(),
        Token::Text(text) => text.clone(),
        Token::Operator(operator) => operator.to_string(),
        Token::Open => "(".to_string(),
        Token::Close => ")".to_string(),
        Token::Separator => ",".to_string(),
        Token::Colon => ":".to_string(),
    }
}

/// A cell address: zero-based row and grid column.
fn parse_cell(name: &str) -> Option<(usize, usize)> {
    let upper = name.to_ascii_uppercase();
    // R1C1
    if let Some(rest) = upper.strip_prefix('R') {
        if let Some((row, column)) = rest.split_once('C') {
            if let (Ok(row), Ok(column)) = (row.parse::<usize>(), column.parse::<usize>()) {
                if row > 0 && column > 0 {
                    return Some((row - 1, column - 1));
                }
            }
        }
    }
    let letters: String = upper
        .chars()
        .take_while(|c| c.is_ascii_uppercase())
        .collect();
    let digits = &upper[letters.len()..];
    if letters.is_empty() || letters.len() > 2 || digits.is_empty() {
        return None;
    }
    let row: usize = digits.parse().ok()?;
    if row == 0 {
        return None;
    }
    let column = letters
        .chars()
        .fold(0usize, |acc, c| acc * 26 + (c as usize - 'A' as usize + 1));
    Some((row - 1, column - 1))
}

struct Parser<'a, 'b> {
    tokens: Vec<Token>,
    position: usize,
    context: &'a FormulaContext<'b>,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn expect_close(&mut self) -> Result<(), FormulaError> {
        match self.next() {
            Some(Token::Close) => Ok(()),
            Some(token) => Err(FormulaError::Syntax(token_text(&token))),
            None => Err(FormulaError::UnexpectedEnd),
        }
    }

    fn comparison(&mut self) -> Result<f64, FormulaError> {
        let left = self.additive()?;
        let operator = match self.peek() {
            Some(Token::Operator(operator @ ("=" | "<" | ">" | "<=" | ">=" | "<>"))) => *operator,
            _ => return Ok(left),
        };
        self.position += 1;
        let right = self.additive()?;
        let truth = match operator {
            "=" => left == right,
            "<" => left < right,
            ">" => left > right,
            "<=" => left <= right,
            ">=" => left >= right,
            _ => left != right,
        };
        Ok(if truth { 1.0 } else { 0.0 })
    }

    fn additive(&mut self) -> Result<f64, FormulaError> {
        let mut value = self.term()?;
        while let Some(Token::Operator(operator @ ("+" | "-"))) = self.peek() {
            let operator = *operator;
            self.position += 1;
            let right = self.term()?;
            value = if operator == "+" {
                value + right
            } else {
                value - right
            };
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, FormulaError> {
        let mut value = self.power()?;
        while let Some(Token::Operator(operator @ ("*" | "/"))) = self.peek() {
            let operator = *operator;
            self.position += 1;
            let right = self.power()?;
            if operator == "*" {
                value *= right;
            } else {
                if right == 0.0 {
                    return Err(FormulaError::ZeroDivide);
                }
                value /= right;
            }
        }
        Ok(value)
    }

    fn power(&mut self) -> Result<f64, FormulaError> {
        let base = self.unary()?;
        if let Some(Token::Operator("^")) = self.peek() {
            self.position += 1;
            let exponent = self.power()?;
            return Ok(base.powf(exponent));
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<f64, FormulaError> {
        match self.peek() {
            Some(Token::Operator("-")) => {
                self.position += 1;
                Ok(-self.unary()?)
            }
            Some(Token::Operator("+")) => {
                self.position += 1;
                self.unary()
            }
            _ => {
                let value = self.primary()?;
                if let Some(Token::Operator("%")) = self.peek() {
                    self.position += 1;
                    return Ok(value / 100.0);
                }
                Ok(value)
            }
        }
    }

    fn primary(&mut self) -> Result<f64, FormulaError> {
        match self.next() {
            None => Err(FormulaError::UnexpectedEnd),
            Some(Token::Number(number)) => Ok(number),
            Some(Token::Text(text)) => parse_field_number(&text).ok_or(FormulaError::Syntax(text)),
            Some(Token::Open) => {
                let value = self.comparison()?;
                self.expect_close()?;
                Ok(value)
            }
            Some(Token::Name(name)) => self.name(name),
            Some(token) => Err(FormulaError::Syntax(token_text(&token))),
        }
    }

    fn name(&mut self, name: String) -> Result<f64, FormulaError> {
        let upper = name.to_ascii_uppercase();
        if matches!(self.peek(), Some(Token::Open)) {
            self.position += 1;
            return self.function(&upper);
        }
        match upper.as_str() {
            "TRUE" => return Ok(1.0),
            "FALSE" => return Ok(0.0),
            _ => {}
        }
        if let Some(cell) = parse_cell(&name).filter(|_| self.context.table.is_some()) {
            if matches!(self.peek(), Some(Token::Colon)) {
                // A range on its own sums, as Word reads `=A1:A3`.
                return Ok(self.range_from(cell)?.iter().sum());
            }
            return Ok(self.cell_value(cell).unwrap_or(0.0));
        }
        match (self.context.bookmark)(&name) {
            Some(text) => Ok(parse_field_number(&text).unwrap_or(0.0)),
            None => Err(FormulaError::UndefinedBookmark(name)),
        }
    }

    fn range_from(&mut self, start: (usize, usize)) -> Result<Vec<f64>, FormulaError> {
        self.position += 1; // ':'
        let end = match self.next() {
            Some(Token::Name(name)) => parse_cell(&name).ok_or(FormulaError::Syntax(name))?,
            Some(token) => return Err(FormulaError::Syntax(token_text(&token))),
            None => return Err(FormulaError::UnexpectedEnd),
        };
        let mut values = Vec::new();
        for row in start.0.min(end.0)..=start.0.max(end.0) {
            for column in start.1.min(end.1)..=start.1.max(end.1) {
                if let Some(value) = self.cell_value((row, column)) {
                    values.push(value);
                }
            }
        }
        Ok(values)
    }

    fn cell_text(&self, (row, column): (usize, usize)) -> Option<&str> {
        let table = self.context.table?;
        table
            .rows
            .get(row)?
            .iter()
            .find(|cell| (cell.column..cell.column + cell.span).contains(&column))
            .map(|cell| cell.text.as_str())
    }

    fn cell_value(&self, cell: (usize, usize)) -> Option<f64> {
        parse_field_number(self.cell_text(cell)?)
    }

    /// The values ABOVE, BELOW, LEFT or RIGHT name: the numbers next to the
    /// formula's cell in that direction, up to the first cell without one.
    fn direction(&self, direction: &str) -> Option<Vec<f64>> {
        let table = self.context.table?;
        let here = self.context.cell?;
        let mut values = Vec::new();
        let mut take = |cell: (usize, usize)| -> bool {
            match self.cell_value(cell) {
                Some(value) => {
                    values.push(value);
                    true
                }
                None => false,
            }
        };
        match direction {
            "ABOVE" => {
                for row in (0..here.row).rev() {
                    if !take((row, here.column)) {
                        break;
                    }
                }
            }
            "BELOW" => {
                for row in here.row + 1..table.rows.len() {
                    if !take((row, here.column)) {
                        break;
                    }
                }
            }
            "LEFT" | "RIGHT" => {
                let row = table.rows.get(here.row)?;
                let position = row.iter().position(|cell| cell.column == here.column)?;
                let cells: Vec<usize> = if direction == "LEFT" {
                    (0..position).rev().map(|index| row[index].column).collect()
                } else {
                    (position + 1..row.len())
                        .map(|index| row[index].column)
                        .collect()
                };
                for column in cells {
                    if !take((here.row, column)) {
                        break;
                    }
                }
            }
            _ => return None,
        }
        Some(values)
    }

    /// One function argument: a list of values (a range or a direction
    /// expands to several).
    fn argument(&mut self) -> Result<Vec<f64>, FormulaError> {
        if let Some(Token::Name(name)) = self.peek().cloned() {
            let upper = name.to_ascii_uppercase();
            let follows = self.tokens.get(self.position + 1);
            if matches!(upper.as_str(), "ABOVE" | "BELOW" | "LEFT" | "RIGHT")
                && matches!(follows, None | Some(Token::Close | Token::Separator))
            {
                self.position += 1;
                return Ok(self.direction(&upper).unwrap_or_default());
            }
            if let Some(cell) = parse_cell(&name).filter(|_| self.context.table.is_some()) {
                if matches!(follows, Some(Token::Colon)) {
                    self.position += 1;
                    return self.range_from(cell);
                }
            }
        }
        Ok(vec![self.comparison()?])
    }

    fn arguments(&mut self) -> Result<Vec<Vec<f64>>, FormulaError> {
        let mut arguments = Vec::new();
        if matches!(self.peek(), Some(Token::Close)) {
            self.position += 1;
            return Ok(arguments);
        }
        loop {
            arguments.push(self.argument()?);
            match self.next() {
                Some(Token::Separator) => continue,
                Some(Token::Close) => break,
                Some(token) => return Err(FormulaError::Syntax(token_text(&token))),
                None => return Err(FormulaError::UnexpectedEnd),
            }
        }
        Ok(arguments)
    }

    fn function(&mut self, name: &str) -> Result<f64, FormulaError> {
        if name == "DEFINED" {
            // DEFINED(x) is 1 when x evaluates without an error.
            let start = self.position;
            let mut depth = 1;
            while let Some(token) = self.next() {
                match token {
                    Token::Open => depth += 1,
                    Token::Close => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner = self.tokens[start..self.position.saturating_sub(1)].to_vec();
            let mut parser = Parser {
                tokens: inner,
                position: 0,
                context: self.context,
            };
            let defined = parser.comparison().is_ok() && parser.position == parser.tokens.len();
            return Ok(if defined { 1.0 } else { 0.0 });
        }
        let arguments = self.arguments()?;
        let flat: Vec<f64> = arguments.iter().flatten().copied().collect();
        let single = |index: usize| -> Result<f64, FormulaError> {
            arguments
                .get(index)
                .and_then(|values| values.first().copied())
                .ok_or(FormulaError::UnexpectedEnd)
        };
        let truth = |value: f64| value != 0.0;
        Ok(match name {
            "SUM" => flat.iter().sum(),
            "PRODUCT" => flat.iter().product(),
            "AVERAGE" => {
                if flat.is_empty() {
                    return Err(FormulaError::ZeroDivide);
                }
                flat.iter().sum::<f64>() / flat.len() as f64
            }
            "COUNT" => flat.len() as f64,
            "MAX" => {
                flat.iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
                    .max(if flat.is_empty() {
                        0.0
                    } else {
                        f64::NEG_INFINITY
                    })
            }
            "MIN" => flat
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                .min(if flat.is_empty() { 0.0 } else { f64::INFINITY }),
            "ABS" => single(0)?.abs(),
            "INT" => single(0)?.trunc(),
            "SIGN" => {
                let value = single(0)?;
                if value > 0.0 {
                    1.0
                } else if value < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            "MOD" => {
                let divisor = single(1)?;
                if divisor == 0.0 {
                    return Err(FormulaError::ZeroDivide);
                }
                single(0)? % divisor
            }
            "ROUND" => {
                let places = single(1)?.trunc() as i32;
                let scale = 10f64.powi(places);
                let value = single(0)? * scale;
                // Round half away from zero, as Word does.
                (value.abs() + 1e-9).round().copysign(value) / scale
            }
            "AND" => (truth(single(0)?) && truth(single(1)?)) as i32 as f64,
            "OR" => (truth(single(0)?) || truth(single(1)?)) as i32 as f64,
            "NOT" => (!truth(single(0)?)) as i32 as f64,
            "IF" => {
                if truth(single(0)?) {
                    single(1)?
                } else {
                    single(2)?
                }
            }
            "TRUE" => 1.0,
            "FALSE" => 0.0,
            _ => return Err(FormulaError::Syntax(name.to_string())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::scan::ScannedCell;

    fn table(rows: &[&[&str]]) -> ScannedTable {
        ScannedTable {
            rows: rows
                .iter()
                .map(|row| {
                    row.iter()
                        .enumerate()
                        .map(|(column, text)| ScannedCell {
                            column,
                            span: 1,
                            text: text.to_string(),
                        })
                        .collect()
                })
                .collect(),
        }
    }

    fn eval(expression: &str) -> Result<f64, FormulaError> {
        let bookmark = |name: &str| (name == "Price").then(|| "1,200".to_string());
        evaluate_formula(
            expression,
            &FormulaContext {
                table: None,
                cell: None,
                bookmark: &bookmark,
            },
        )
    }

    #[test]
    fn arithmetic_precedence_and_functions() {
        assert_eq!(eval("1+2*3"), Ok(7.0));
        assert_eq!(eval("(1+2)*3"), Ok(9.0));
        assert_eq!(eval("2^3^2"), Ok(512.0));
        assert_eq!(eval("-2^2"), Ok(4.0));
        assert_eq!(eval("50%"), Ok(0.5));
        assert_eq!(
            eval("Price * 1.1").map(|v| (v * 100.0).round() / 100.0),
            Ok(1320.0)
        );
        assert_eq!(eval("SUM(1, 2, 3)"), Ok(6.0));
        assert_eq!(eval("AVERAGE(2,4)"), Ok(3.0));
        assert_eq!(eval("MAX(1,9,3)"), Ok(9.0));
        assert_eq!(eval("MIN(4,2)"), Ok(2.0));
        assert_eq!(eval("ROUND(2.345, 2)"), Ok(2.35));
        assert_eq!(eval("ROUND(-2.5, 0)"), Ok(-3.0));
        assert_eq!(eval("INT(7.9)"), Ok(7.0));
        assert_eq!(eval("MOD(7,3)"), Ok(1.0));
        assert_eq!(eval("IF(3>2, 10, 20)"), Ok(10.0));
        assert_eq!(eval("AND(1, 0)"), Ok(0.0));
        assert_eq!(eval("OR(1, 0)"), Ok(1.0));
        assert_eq!(eval("NOT(0)"), Ok(1.0));
        assert_eq!(eval("SIGN(-4)"), Ok(-1.0));
        assert_eq!(eval("ABS(-4)"), Ok(4.0));
        assert_eq!(eval("COUNT(1,2,3)"), Ok(3.0));
        assert_eq!(eval("PRODUCT(2,3,4)"), Ok(24.0));
        assert_eq!(eval("DEFINED(Price)"), Ok(1.0));
        assert_eq!(eval("DEFINED(Missing)"), Ok(0.0));
        assert_eq!(eval("TRUE"), Ok(1.0));
        assert_eq!(eval("3 <> 4"), Ok(1.0));
    }

    #[test]
    fn errors_read_as_word_writes_them() {
        assert_eq!(eval("1/0"), Err(FormulaError::ZeroDivide));
        assert_eq!(
            eval("Missing + 1"),
            Err(FormulaError::UndefinedBookmark("Missing".into()))
        );
        assert_eq!(eval("1 +"), Err(FormulaError::UnexpectedEnd));
        assert_eq!(eval("1 )"), Err(FormulaError::Syntax(")".into())));
        assert_eq!(FormulaError::ZeroDivide.message(false), "!Zero Divide");
        assert_eq!(
            FormulaError::Syntax(")".into()).message(true),
            "!構文エラー、)"
        );
    }

    #[test]
    fn table_directions_and_references() {
        let grid = table(&[
            &["Item", "Q1", "Q2", "Total"],
            &["A", "10", "20", ""],
            &["B", "1,000", "5", ""],
            &["Sum", "", "", ""],
        ]);
        let bookmark = |_: &str| None;
        let at = |row, column| FormulaContext {
            table: Some(&grid),
            cell: Some(CellRef {
                table: 0,
                row,
                column,
            }),
            bookmark: &bookmark,
        };
        assert_eq!(evaluate_formula("SUM(LEFT)", &at(1, 3)), Ok(30.0));
        assert_eq!(evaluate_formula("SUM(ABOVE)", &at(3, 1)), Ok(1010.0));
        assert_eq!(evaluate_formula("SUM(ABOVE)", &at(3, 2)), Ok(25.0));
        assert_eq!(evaluate_formula("SUM(B2:C3)", &at(3, 3)), Ok(1035.0));
        assert_eq!(evaluate_formula("B2*C2", &at(3, 3)), Ok(200.0));
        assert_eq!(evaluate_formula("R2C2+R2C3", &at(3, 3)), Ok(30.0));
        assert_eq!(evaluate_formula("AVERAGE(ABOVE)", &at(3, 1)), Ok(505.0));
        assert_eq!(evaluate_formula("SUM(RIGHT)", &at(1, 0)), Ok(30.0));
        assert_eq!(evaluate_formula("SUM(BELOW)", &at(0, 1)), Ok(1010.0));
    }
}
