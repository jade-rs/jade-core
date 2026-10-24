//! The spreadsheet number formats a chart's axis and data labels state
//! (`c:numFmt formatCode`, or the series' cached `c:formatCode`): enough of
//! the format language for what charts carry — sections, digits, grouping,
//! percent, scaling commas, scientific notation, literal text and currency
//! tags. Dates and fractions fall back to `General`.

/// `value` written in `code`; `General` (or no code) when the code is
/// absent or beyond this subset.
pub fn format_chart_number(value: f64, code: Option<&str>) -> String {
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        return general(value);
    };
    if code.eq_ignore_ascii_case("general") {
        return general(value);
    }
    let sections = split_sections(code);
    // positive;negative;zero — a negative value written by its own section
    // drops its sign, as the section supplies any it wants.
    let (section, value) = match sections.len() {
        0 => return general(value),
        1 => (sections[0].as_str(), value),
        _ if value < 0.0 => (sections[1].as_str(), -value),
        _ if value == 0.0 && sections.len() > 2 => (sections[2].as_str(), value),
        _ => (sections[0].as_str(), value),
    };
    format_section(section, value).unwrap_or_else(|| general(value))
}

/// `General`: up to ten significant digits, no grouping, trailing zeros
/// trimmed, scientific for the very large and very small.
pub fn general(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if !(1e-9..1e11).contains(&magnitude) {
        let text = format!("{value:.5E}");
        return tidy_exponent(&text);
    }
    let digits = (9 - magnitude.log10().floor() as i32).clamp(0, 10) as usize;
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

/// Rust's `1.5E3` as a spreadsheet's `1.5E+03`, mantissa zeros trimmed.
fn tidy_exponent(text: &str) -> String {
    let Some((mantissa, exponent)) = text.split_once('E') else {
        return text.to_string();
    };
    let mantissa = if mantissa.contains('.') {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    } else {
        mantissa
    };
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", exponent.abs())
}

/// Splits on `;` outside quotes and escapes.
fn split_sections(code: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut chars = code.chars();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                sections.last_mut().unwrap().push(c);
            }
            '\\' if !quoted => {
                let last = sections.last_mut().unwrap();
                last.push(c);
                if let Some(next) = chars.next() {
                    last.push(next);
                }
            }
            ';' if !quoted => sections.push(String::new()),
            _ => sections.last_mut().unwrap().push(c),
        }
    }
    sections
}

/// One piece of a section: a literal or the number's placeholders.
enum Token {
    Literal(String),
    Number,
}

fn format_section(section: &str, value: f64) -> Option<String> {
    // Literal pieces around one run of number placeholders.
    let mut tokens: Vec<Token> = Vec::new();
    let mut pattern = String::new();
    let mut literal = String::new();
    let mut chars = section.chars().peekable();
    let mut percent = 0;
    let mut in_number = false;
    let mut number_done = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    literal.push(q);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    literal.push(next);
                }
            }
            // `_x` pads the width of x; `*x` repeats x to fill the cell.
            '_' => {
                chars.next();
                literal.push(' ');
            }
            '*' => {
                chars.next();
            }
            '[' => {
                // `[Red]`, `[>100]`, `[$€-407]`: a currency tag keeps its
                // symbol, the rest draw nothing.
                let mut tag = String::new();
                for t in chars.by_ref() {
                    if t == ']' {
                        break;
                    }
                    tag.push(t);
                }
                if let Some(currency) = tag.strip_prefix('$') {
                    literal.push_str(currency.split('-').next().unwrap_or(""));
                }
            }
            '%' => {
                percent += 1;
                literal.push('%');
            }
            '0' | '#' | '?' | '.' | ',' if !number_done => {
                if !in_number {
                    if !literal.is_empty() {
                        tokens.push(Token::Literal(std::mem::take(&mut literal)));
                    }
                    tokens.push(Token::Number);
                    in_number = true;
                }
                pattern.push(c);
            }
            'E' | 'e' if in_number && matches!(chars.peek(), Some('+') | Some('-')) => {
                pattern.push('E');
                if let Some(sign) = chars.next() {
                    pattern.push(sign);
                }
                while let Some(&digit) = chars.peek() {
                    if matches!(digit, '0' | '#') {
                        pattern.push(digit);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            // Date and time letters: beyond this subset.
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M' if !in_number => return None,
            '/' if in_number => return None,
            _ => {
                if in_number {
                    in_number = false;
                    number_done = true;
                }
                literal.push(c);
            }
        }
        if in_number && !matches!(c, '0' | '#' | '?' | '.' | ',' | 'E' | 'e') {
            in_number = false;
            number_done = true;
        }
    }
    if !literal.is_empty() {
        tokens.push(Token::Literal(literal));
    }
    if !tokens.iter().any(|t| matches!(t, Token::Number)) {
        // Text only: the section is the whole label.
        return Some(
            tokens
                .into_iter()
                .map(|t| match t {
                    Token::Literal(text) => text,
                    Token::Number => String::new(),
                })
                .collect(),
        );
    }
    let value = value * 100f64.powi(percent);
    let number = format_pattern(&pattern, value)?;
    Some(
        tokens
            .into_iter()
            .map(|t| match t {
                Token::Literal(text) => text,
                Token::Number => number.clone(),
            })
            .collect(),
    )
}

/// The digits of `value` by a placeholder run like `#,##0.00` or `0.0E+00`.
fn format_pattern(pattern: &str, mut value: f64) -> Option<String> {
    let (mantissa, exponent) = match pattern.find('E') {
        Some(at) => (&pattern[..at], Some(&pattern[at + 1..])),
        None => (pattern, None),
    };
    // Commas after the last digit placeholder scale by a thousand each.
    let trailing_commas = mantissa.len() - mantissa.trim_end_matches(',').len();
    let mantissa = mantissa.trim_end_matches(',');
    value /= 1000f64.powi(trailing_commas as i32);
    let (integer_part, fraction_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa, ""),
    };
    let decimals = fraction_part
        .chars()
        .filter(|c| matches!(c, '0' | '#' | '?'))
        .count();
    let required_decimals = fraction_part.chars().filter(|c| *c == '0').count();
    let grouping = integer_part.contains(',');
    let min_integer_digits = integer_part.chars().filter(|c| *c == '0').count();
    let negative = value < 0.0;
    let mut magnitude = value.abs();
    let mut exponent_text = String::new();
    if let Some(exponent) = exponent {
        let power = if magnitude > 0.0 {
            magnitude.log10().floor() as i32
        } else {
            0
        };
        magnitude /= 10f64.powi(power);
        // Rounding can carry the mantissa to 10.
        if format!("{magnitude:.decimals$}").starts_with("10") {
            magnitude /= 10.0;
            let power = power + 1;
            exponent_text = exponent_string(exponent, power);
        } else {
            exponent_text = exponent_string(exponent, power);
        }
    }
    // Halves round away from zero, as spreadsheets do (Rust's formatting
    // rounds them to even), with a nudge for binary fractions just under.
    let power = 10f64.powi(decimals as i32);
    let magnitude = (magnitude * power * (1.0 + 4.0 * f64::EPSILON)).round() / power;
    let text = format!("{magnitude:.decimals$}");
    let (int_digits, frac_digits) = match text.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (text, String::new()),
    };
    // Optional decimals (`#`) drop their trailing zeros.
    let mut frac_digits = frac_digits;
    while frac_digits.len() > required_decimals && frac_digits.ends_with('0') {
        frac_digits.pop();
    }
    let mut int_digits = int_digits.trim_start_matches('0').to_string();
    while int_digits.len() < min_integer_digits {
        int_digits.insert(0, '0');
    }
    if grouping {
        int_digits = group_thousands(&int_digits);
    }
    let mut out = String::new();
    let zero =
        int_digits.chars().all(|c| c == '0' || c == ',') && frac_digits.chars().all(|c| c == '0');
    if negative && !zero {
        out.push('-');
    }
    out.push_str(&int_digits);
    if !frac_digits.is_empty() {
        out.push('.');
        out.push_str(&frac_digits);
    } else if mantissa.ends_with('.') {
        out.push('.');
    }
    out.push_str(&exponent_text);
    Some(out)
}

fn exponent_string(pattern: &str, power: i32) -> String {
    let sign = if power < 0 {
        "-"
    } else if pattern.starts_with('+') {
        "+"
    } else {
        ""
    };
    let width = pattern.chars().filter(|c| *c == '0').count();
    format!("E{sign}{:0width$}", power.abs())
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::format_chart_number as f;

    #[test]
    fn general_trims_and_keeps_ten_digits() {
        assert_eq!(f(8.200000000000001, None), "8.2");
        assert_eq!(f(1234567.0, Some("General")), "1234567");
        assert_eq!(f(-0.5, None), "-0.5");
        assert_eq!(f(0.0, None), "0");
    }

    #[test]
    fn digits_grouping_and_decimals() {
        assert_eq!(f(1234.5, Some("#,##0")), "1,235");
        assert_eq!(f(1234.5, Some("#,##0.00")), "1,234.50");
        assert_eq!(f(0.25, Some("0.0")), "0.3");
        assert_eq!(f(3.0, Some("0.0#")), "3.0");
        assert_eq!(f(0.4, Some("#.##")), ".4");
        assert_eq!(f(12_500_000.0, Some("#,##0,,\"M\"")), "13M");
    }

    #[test]
    fn percent_currency_literals_and_sections() {
        assert_eq!(f(0.256, Some("0%")), "26%");
        assert_eq!(f(0.256, Some("0.0%")), "25.6%");
        assert_eq!(f(1500.0, Some("\"$\"#,##0")), "$1,500");
        assert_eq!(f(1500.0, Some("[$¥-411]#,##0")), "¥1,500");
        assert_eq!(f(-1500.0, Some("#,##0;(#,##0)")), "(1,500)");
        assert_eq!(f(-1500.0, Some("#,##0;[Red]\\-#,##0")), "-1,500");
        assert_eq!(f(0.0, Some("0;-0;\"-\"")), "-");
        assert_eq!(f(42.0, Some("0\"件\"")), "42件");
        assert_eq!(f(5.0, Some("_-* #,##0_-")), " 5 ");
    }

    #[test]
    fn scientific_and_dates() {
        assert_eq!(f(12345.0, Some("0.00E+00")), "1.23E+04");
        assert_eq!(f(0.00012, Some("0.0E+00")), "1.2E-04");
        // Dates are beyond the subset: General.
        assert_eq!(f(45000.0, Some("yyyy/mm/dd")), "45000");
    }
}
