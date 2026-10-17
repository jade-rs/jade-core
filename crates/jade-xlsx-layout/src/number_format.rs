//! Turning a cell value into the text Excel shows.
//!
//! A number format is a small language: up to four `;`-separated sections
//! (positive, negative, zero, text), a colour in brackets, and a pattern made
//! of digit placeholders or date tokens. This implements the numeric and
//! date/time halves of it, which is what a business workbook uses.
//!
//! Not implemented, and falling through to the general form rather than
//! guessing: fractions (`# ??/??`) and Japanese era codes (`ggge`). Month and
//! day names are English, which is Excel's `en-US` default; a locale-aware
//! build would take them from the `[$-411]`-style locale tag this ignores.

use jade_xlsx_reader::{CellValue, DateSystem};

/// A value formatted for display, with the colour the format asked for.
///
/// The colour is part of the format language (`[Red]-#,##0`), and Excel lets
/// it override the font's own colour — which is how a negative total turns red
/// without the cell being styled red.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    pub color: Option<FormatColor>,
}

impl Formatted {
    fn plain(text: String) -> Self {
        Self { text, color: None }
    }
}

/// The colours a number format can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatColor {
    Black,
    Blue,
    Cyan,
    Green,
    Magenta,
    Red,
    White,
    Yellow,
    /// `[Color12]`, a position in the legacy palette.
    Indexed(u32),
}

impl FormatColor {
    fn parse(token: &str) -> Option<Self> {
        let token = token.trim();
        if let Some(index) = token
            .strip_prefix("Color")
            .or_else(|| token.strip_prefix("COLOR"))
            .or_else(|| token.strip_prefix("color"))
        {
            return index.parse().ok().map(FormatColor::Indexed);
        }
        Some(match token.to_ascii_lowercase().as_str() {
            "black" => FormatColor::Black,
            "blue" => FormatColor::Blue,
            "cyan" => FormatColor::Cyan,
            "green" => FormatColor::Green,
            "magenta" => FormatColor::Magenta,
            "red" => FormatColor::Red,
            "white" => FormatColor::White,
            "yellow" => FormatColor::Yellow,
            _ => return None,
        })
    }
}

/// Formats `value` for display under `code`.
///
/// `code` is the format code from the style table; `None` means General.
pub fn format_value(value: &CellValue, code: Option<&str>, dates: DateSystem) -> Formatted {
    match value {
        CellValue::Empty => Formatted::plain(String::new()),
        CellValue::Bool(flag) => Formatted::plain(if *flag { "TRUE" } else { "FALSE" }.to_string()),
        CellValue::Error(message) => Formatted::plain(message.clone()),
        CellValue::Text(text) => format_text(text, code),
        CellValue::Number(number) => format_number(*number, code, dates),
    }
}

fn format_text(text: &str, code: Option<&str>) -> Formatted {
    // The fourth section is the one that applies to text; without it the text
    // shows as-is, which is also what `@` means.
    let Some(sections) = code.map(split_sections) else {
        return Formatted::plain(text.to_string());
    };
    let Some(section) = sections.get(3) else {
        return Formatted::plain(text.to_string());
    };
    let (color, pattern) = take_color(section);
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '@' => out.push_str(text),
            '"' => out.extend(chars.by_ref().take_while(|c| *c != '"')),
            '\\' => out.extend(chars.next()),
            '*' | '_' => {
                chars.next();
            }
            other => out.push(other),
        }
    }
    Formatted { text: out, color }
}

fn format_number(value: f64, code: Option<&str>, dates: DateSystem) -> Formatted {
    let Some(code) = code else {
        return Formatted::plain(general(value));
    };
    let sections = split_sections(code);
    let conditional = sections.iter().any(|section| has_condition(section));

    // Which section applies, and whether the sign has to be added back. A
    // negative that falls through to the positive section keeps its `-`; one
    // with a section of its own does not, because the section carries whatever
    // sign it wants.
    let (index, magnitude, sign) = if conditional {
        let Some(index) = select_conditional_section(&sections, value) else {
            return Formatted::plain(general(value));
        };
        // Unlike the sign-based second section, a condition receives the
        // actual value. The numeric formatter therefore keeps its minus sign.
        (index, value, "")
    } else if value < 0.0 {
        match sections.len() {
            0 | 1 => (0, value.abs(), "-"),
            _ => (1, value.abs(), ""),
        }
    } else if value == 0.0 && sections.len() >= 3 {
        (2, 0.0, "")
    } else {
        (0, value, "")
    };

    let Some(section) = sections.get(index) else {
        return Formatted::plain(general(value));
    };
    let (color, pattern) = take_color(section);
    if pattern.trim().is_empty() {
        // An empty section hides the value, which is how `0;;` shows nothing
        // for negatives and zeroes.
        return Formatted {
            text: String::new(),
            color,
        };
    }
    if pattern.eq_ignore_ascii_case("general") {
        return Formatted {
            text: format!("{sign}{}", general(magnitude)),
            color,
        };
    }

    if is_date_pattern(&pattern) {
        return match format_datetime(magnitude, &pattern, dates) {
            Some(text) => Formatted { text, color },
            None => Formatted::plain(general(value)),
        };
    }

    if let Some(text) = format_fraction(magnitude, &pattern) {
        return Formatted {
            text: format!("{sign}{text}"),
            color,
        };
    }

    if let Some(format) = parse_numeric(&pattern) {
        return Formatted {
            text: format!("{sign}{}", apply(magnitude, &format)),
            color,
        };
    }
    // A section with no placeholder at all is its literals: `0;-0;"-"` shows a
    // dash for zero rather than a zero.
    if let Some(literal) = literal_only(&pattern) {
        return Formatted {
            text: literal,
            color,
        };
    }
    // Anything else unrecognised: the stored number is visibly unfinished
    // where a guess would be quietly wrong.
    Formatted::plain(general(value))
}

/// Formats the common SpreadsheetML fraction shapes (`# ?/?`, `?/??`, and a
/// fixed denominator such as `# ?/8`). Question marks affect alignment in
/// Excel; the text model omits that padding and keeps the represented value.
fn format_fraction(value: f64, pattern: &str) -> Option<String> {
    let slash = pattern.find('/')?;
    let bytes = pattern.as_bytes();

    let mut numerator_start = slash;
    while numerator_start > 0 && matches!(bytes[numerator_start - 1], b'0' | b'#' | b'?') {
        numerator_start -= 1;
    }
    if numerator_start == slash {
        return None;
    }

    let mut denominator_end = slash + 1;
    while denominator_end < bytes.len()
        && matches!(bytes[denominator_end], b'0'..=b'9' | b'#' | b'?')
    {
        denominator_end += 1;
    }
    if denominator_end == slash + 1 || !pattern[denominator_end..].trim().is_empty() {
        return None;
    }

    let before = pattern[..numerator_start].trim_end();
    let mixed = !before.is_empty()
        && before
            .chars()
            .all(|character| matches!(character, '0' | '#' | '?'));
    if !before.is_empty() && !mixed {
        return None;
    }

    let denominator_pattern = &pattern[slash + 1..denominator_end];
    let fixed_denominator = denominator_pattern
        .chars()
        .all(|character| character.is_ascii_digit())
        .then(|| denominator_pattern.parse::<u64>().ok())
        .flatten()
        .filter(|denominator| *denominator > 0);
    let maximum_denominator = if let Some(denominator) = fixed_denominator {
        denominator
    } else {
        let width = denominator_pattern
            .chars()
            .filter(|character| matches!(character, '0' | '#' | '?'))
            .count();
        if width == 0 || width > 4 {
            return None;
        }
        10u64.pow(width as u32) - 1
    };

    let whole = if mixed { value.floor() as u64 } else { 0 };
    let target = if mixed { value.fract() } else { value };
    let (mut numerator, denominator) = if let Some(denominator) = fixed_denominator {
        ((target * denominator as f64).round() as u64, denominator)
    } else {
        best_fraction(target, maximum_denominator)
    };

    let mut whole = whole;
    if mixed && numerator >= denominator {
        whole += numerator / denominator;
        numerator %= denominator;
    }
    if numerator == 0 {
        return Some(whole.to_string());
    }
    if mixed && whole > 0 {
        Some(format!("{whole} {numerator}/{denominator}"))
    } else {
        Some(format!("{numerator}/{denominator}"))
    }
}

fn best_fraction(value: f64, maximum_denominator: u64) -> (u64, u64) {
    let mut best = (0, 1);
    let mut best_error = value.abs();
    for denominator in 1..=maximum_denominator {
        let numerator = (value * denominator as f64).round() as u64;
        let error = (value - numerator as f64 / denominator as f64).abs();
        if error < best_error {
            best = (numerator, denominator);
            best_error = error;
        }
    }
    best
}

/// The literal text of a pattern that has no digit placeholders, or `None` when
/// it has any.
fn literal_only(pattern: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '0' | '#' | '?' => return None,
            '"' => {
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    out.push(c);
                }
            }
            '\\' => out.extend(chars.next()),
            '_' | '*' => {
                chars.next();
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// Splits a code into its `;`-separated sections, ignoring separators inside
/// quotes or brackets.
fn split_sections(code: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut in_quotes = false;
    let mut in_brackets = false;
    let mut escaped = false;
    for c in code.chars() {
        if escaped {
            sections.last_mut().expect("a section").push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                sections.last_mut().expect("a section").push(c);
            }
            '"' => {
                in_quotes = !in_quotes;
                sections.last_mut().expect("a section").push(c);
            }
            '[' if !in_quotes => {
                in_brackets = true;
                sections.last_mut().expect("a section").push(c);
            }
            ']' if !in_quotes => {
                in_brackets = false;
                sections.last_mut().expect("a section").push(c);
            }
            ';' if !in_quotes && !in_brackets => sections.push(String::new()),
            other => sections.last_mut().expect("a section").push(other),
        }
    }
    sections
}

/// Whether a section selects itself by comparison rather than by sign.
fn has_condition(section: &str) -> bool {
    brackets(section)
        .into_iter()
        .any(|token| token.starts_with(['<', '>', '=']))
}

fn select_conditional_section(sections: &[String], value: f64) -> Option<usize> {
    let mut fallback = None;
    for (index, section) in sections.iter().enumerate() {
        match condition_matches(section, value) {
            Some(true) => return Some(index),
            Some(false) => {}
            None => {
                fallback.get_or_insert(index);
            }
        };
    }
    fallback
}

fn condition_matches(section: &str, value: f64) -> Option<bool> {
    let token = brackets(section)
        .into_iter()
        .find(|token| token.starts_with(['<', '>', '=']))?;
    let (operator, operand) =
        [">=", "<=", "<>", ">", "<", "="]
            .into_iter()
            .find_map(|operator| {
                token
                    .strip_prefix(operator)
                    .map(|operand| (operator, operand))
            })?;
    let Ok(operand) = operand.trim().parse::<f64>() else {
        return Some(false);
    };
    Some(match operator {
        ">=" => value >= operand,
        "<=" => value <= operand,
        "<>" => value != operand,
        ">" => value > operand,
        "<" => value < operand,
        _ => value == operand,
    })
}

/// The bracketed tokens in a section.
fn brackets(section: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current: Option<String> = None;
    let mut in_quotes = false;
    for c in section.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => current = Some(String::new()),
            ']' if !in_quotes => {
                if let Some(token) = current.take() {
                    tokens.push(token);
                }
            }
            other => {
                if let Some(token) = current.as_mut() {
                    token.push(other);
                }
            }
        }
    }
    tokens
}

/// Strips the bracketed tokens a section carries, returning the colour it named
/// and the pattern without them.
///
/// `[$¥-411]` is a currency tag: the part between `$` and `-` is a literal that
/// prints, which is how Japanese workbooks put the yen sign on a number.
/// Elapsed-time brackets (`[h]`) are left in place for the date formatter.
fn take_color(section: &str) -> (Option<FormatColor>, String) {
    let mut color = None;
    let mut out = String::new();
    let mut token: Option<String> = None;
    let mut in_quotes = false;
    for c in section.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                out.push(c);
            }
            '[' if !in_quotes => token = Some(String::new()),
            ']' if !in_quotes => {
                let Some(token) = token.take() else { continue };
                if let Some(parsed) = FormatColor::parse(&token) {
                    color = Some(parsed);
                } else if let Some(currency) = token.strip_prefix('$') {
                    let literal = currency.split('-').next().unwrap_or_default();
                    if !literal.is_empty() {
                        out.push('"');
                        out.push_str(literal);
                        out.push('"');
                    }
                } else if matches!(token.trim(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
                    // Elapsed time: kept so the date formatter sees it.
                    out.push('[');
                    out.push_str(&token);
                    out.push(']');
                }
                // Anything else — a locale tag such as `[$-411]` — is dropped.
            }
            other => match token.as_mut() {
                Some(token) => token.push(other),
                None => out.push(other),
            },
        }
    }
    (color, out)
}

// ── general ─────────────────────────────────────────────────────────────

/// Excel's General: up to 11 significant digits, no trailing zeros, and no
/// thousands separator.
fn general(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

/// Numeric drawing must not expose a clipped suffix as a different value.
/// Width affects painting only; callers still get the complete formatted text.
pub(crate) fn fit_number(
    value: f64,
    code: Option<&str>,
    text: &str,
    width: f64,
    measure: impl Fn(&str) -> f64,
) -> String {
    if text.is_empty() || measure(text) <= width || !value.is_finite() {
        return text.to_string();
    }
    if code.is_none_or(|code| code.trim().eq_ignore_ascii_case("general")) {
        if value.fract() != 0.0 && (1e-9..1e11).contains(&value.abs()) {
            for precision in (0..=10).rev() {
                let candidate = format!("{value:.precision$}");
                let candidate = if candidate.contains('.') {
                    candidate.trim_end_matches('0').trim_end_matches('.')
                } else {
                    &candidate
                };
                if candidate != "0" && candidate != "-0" && measure(candidate) <= width {
                    return candidate.to_string();
                }
            }
        }
        for precision in (0..=5).rev() {
            let candidate = format!("{value:.precision$E}");
            if let Some((mantissa, exponent)) = candidate.split_once('E') {
                let mantissa = if mantissa.contains('.') {
                    mantissa.trim_end_matches('0').trim_end_matches('.')
                } else {
                    mantissa
                };
                if let Ok(exponent) = exponent.parse::<i32>() {
                    let candidate = format!("{mantissa}E{exponent:+03}");
                    if measure(&candidate) <= width {
                        return candidate;
                    }
                }
            }
        }
    }
    let hash_width = measure("#");
    let count = if hash_width.is_finite() && hash_width > 0.0 {
        (width.max(0.0) / hash_width).floor().clamp(1.0, 1024.0) as usize
    } else {
        1
    };
    "#".repeat(count)
}

// ── numeric ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct NumericFormat {
    prefix: String,
    suffix: String,
    minimum_decimals: usize,
    maximum_decimals: usize,
    thousands: bool,
    percent: bool,
    /// Powers of a thousand to divide by, from trailing commas (`#,##0,,` shows
    /// millions).
    scale_thousands: u32,
    scientific: Option<ScientificFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScientificFormat {
    marker: char,
    positive_sign: bool,
    exponent_digits: usize,
    integer_digits: usize,
}

fn parse_numeric(section: &str) -> Option<NumericFormat> {
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut minimum_decimals = 0usize;
    let mut maximum_decimals = 0usize;
    let mut thousands = false;
    let mut percent = false;
    let mut scale_thousands = 0u32;
    let mut scientific = None;
    let mut seen_digit_placeholder = false;
    let mut integer_digits = 0usize;
    let mut after_point = false;

    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut literal = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    literal.push(c);
                }
                if seen_digit_placeholder {
                    suffix.push_str(&literal);
                } else {
                    prefix.push_str(&literal);
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    if seen_digit_placeholder {
                        suffix.push(escaped);
                    } else {
                        prefix.push(escaped);
                    }
                }
            }
            '?' => return None,
            '/' if seen_digit_placeholder => return None,
            'E' | 'e' => {
                // `0.00E+00`: the exponent's own placeholders are consumed
                // here, since they are not part of the mantissa's decimals.
                if !seen_digit_placeholder {
                    return None;
                }
                let marker = c;
                let positive_sign = matches!(chars.peek(), Some('+'));
                if matches!(chars.peek(), Some('+') | Some('-')) {
                    chars.next();
                }
                let mut exponent_digits = 0usize;
                while matches!(chars.peek(), Some('0') | Some('#')) {
                    exponent_digits += 1;
                    chars.next();
                }
                if exponent_digits == 0 {
                    return None;
                }
                scientific = Some(ScientificFormat {
                    marker,
                    positive_sign,
                    exponent_digits,
                    integer_digits: integer_digits.max(1),
                });
            }
            '0' | '#' => {
                seen_digit_placeholder = true;
                if after_point {
                    maximum_decimals += 1;
                    if c == '0' {
                        minimum_decimals += 1;
                    }
                } else {
                    integer_digits += 1;
                }
            }
            '.' => after_point = true,
            ',' => {
                if seen_digit_placeholder && matches!(chars.peek(), Some('0') | Some('#')) {
                    thousands = true;
                } else if seen_digit_placeholder {
                    scale_thousands += 1;
                }
            }
            '%' => {
                percent = true;
                if seen_digit_placeholder {
                    suffix.push('%');
                } else {
                    prefix.push('%');
                }
            }
            '_' => {
                // `_)` reserves the width of a character; a space is what it
                // renders as.
                chars.next();
                if seen_digit_placeholder {
                    suffix.push(' ');
                } else {
                    prefix.push(' ');
                }
            }
            '*' => {
                chars.next();
            }
            '@' => return None,
            other => {
                if seen_digit_placeholder {
                    suffix.push(other);
                } else {
                    prefix.push(other);
                }
            }
        }
    }

    seen_digit_placeholder.then_some(NumericFormat {
        prefix,
        suffix,
        minimum_decimals,
        maximum_decimals,
        thousands,
        percent,
        scale_thousands,
        scientific,
    })
}

fn apply(value: f64, format: &NumericFormat) -> String {
    let mut scaled = if format.percent { value * 100.0 } else { value };
    for _ in 0..format.scale_thousands {
        scaled /= 1000.0;
    }

    if let Some(scientific) = format.scientific {
        let step = scientific.integer_digits as i32;
        let mut exponent = if scaled == 0.0 {
            0
        } else {
            let order = scaled.abs().log10().floor() as i32;
            order - order.rem_euclid(step)
        };
        let mut mantissa = scaled / 10f64.powi(exponent);
        let factor = 10f64.powi(format.maximum_decimals as i32);
        mantissa = (mantissa.abs() * factor).round() / factor * mantissa.signum();
        let limit = 10f64.powi(step);
        if mantissa.abs() >= limit {
            mantissa /= limit;
            exponent += step;
        }
        let mantissa = format!("{:.*}", format.maximum_decimals, mantissa);
        let mantissa = trim_optional_decimals(&mantissa, format.minimum_decimals);
        let exponent_sign = if exponent < 0 {
            "-"
        } else if scientific.positive_sign {
            "+"
        } else {
            ""
        };
        return format!(
            "{}{mantissa}{}{exponent_sign}{:0width$}{}",
            format.prefix,
            scientific.marker,
            exponent.abs(),
            format.suffix,
            width = scientific.exponent_digits,
        );
    }

    // Excel rounds half away from zero; Rust's formatter rounds half to even,
    // so 1234.5 under `0` would print 1234 rather than the 1235 a spreadsheet
    // shows. Round first, then format the already-rounded value.
    let factor = 10f64.powi(format.maximum_decimals as i32);
    let magnitude = (scaled.abs() * factor).round() / factor;
    let rounded = format!("{:.*}", format.maximum_decimals, magnitude);
    let (integer, fraction) = match rounded.split_once('.') {
        Some((integer, fraction)) => {
            let keep = fraction
                .trim_end_matches('0')
                .len()
                .max(format.minimum_decimals);
            (integer.to_string(), Some(fraction[..keep].to_string()))
        }
        None => (rounded, None),
    };
    let integer = if format.thousands {
        group_thousands(&integer)
    } else {
        integer
    };

    let mut out = String::new();
    if scaled < 0.0 {
        out.push('-');
    }
    out.push_str(&format.prefix);
    out.push_str(&integer);
    if let Some(fraction) = fraction.filter(|fraction| !fraction.is_empty()) {
        out.push('.');
        out.push_str(&fraction);
    }
    out.push_str(&format.suffix);
    out
}

fn trim_optional_decimals(text: &str, minimum_decimals: usize) -> &str {
    let Some(point) = text.find('.') else {
        return text;
    };
    let significant = text[point + 1..].trim_end_matches('0').len();
    let keep = significant.max(minimum_decimals);
    if keep == 0 {
        &text[..point]
    } else {
        &text[..point + 1 + keep]
    }
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::new();
    let count = digits.len();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (count - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ── dates ───────────────────────────────────────────────────────────────

/// Whether a pattern is a date/time one rather than a numeric one.
///
/// Excel decides this the same way: a section holding any of `y m d h s`
/// outside a literal is a date, which is why `0 "m"` is a number and `0 m` is
/// not.
fn is_date_pattern(pattern: &str) -> bool {
    let mut in_quotes = false;
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => in_quotes = !in_quotes,
            c if !in_quotes && matches!(c.to_ascii_lowercase(), 'y' | 'm' | 'd' | 'h' | 's') => {
                return true
            }
            _ => {}
        }
    }
    false
}

/// Whether a format code shows a number as a date or a time.
///
/// The first section is the one a positive number — and so a date — reads
/// through; colours, locale tags and conditions in brackets are not part of
/// the pattern, so `[Red]0` is a number even though "Red" has a `d` in it.
pub fn is_date_format(code: &str) -> bool {
    let sections = split_sections(code);
    let Some(section) = sections.first() else {
        return false;
    };
    let (_, pattern) = take_color(section);
    is_date_pattern(&pattern)
}

#[derive(Debug, Clone, PartialEq)]
enum DateToken {
    Year(usize),
    Month(usize),
    Minute(usize),
    Day(usize),
    Hour(usize),
    Second(usize),
    /// `.0`, `.00`, `.000` following seconds.
    FractionalSecond(usize),
    /// `[h]`, `[m]`, `[s]`: time that accumulates instead of wrapping.
    Elapsed(char, usize),
    Meridiem {
        long: bool,
    },
    Literal(String),
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn format_datetime(serial: f64, pattern: &str, dates: DateSystem) -> Option<String> {
    // Excel's calendar ends at 9999-12-31: serial 2958465 in the 1900 system,
    // 2957003 in the 1904 one. Past it there is no date to show — and the day
    // arithmetic below would overflow — so the value falls back to General.
    let last_serial = match dates {
        DateSystem::Nineteen00 => 2_958_466.0,
        DateSystem::Nineteen04 => 2_957_004.0,
    };
    if !(0.0..last_serial).contains(&serial) {
        return None;
    }
    let tokens = tokenize_date(pattern);
    let twelve_hour = tokens
        .iter()
        .any(|token| matches!(token, DateToken::Meridiem { .. }));
    let fractional_width = tokens
        .iter()
        .filter_map(|token| match token {
            DateToken::FractionalSecond(width) => Some(*width),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .min(9);
    let (datetime_serial, fractional) = if fractional_width == 0 {
        (serial, 0u64)
    } else {
        let scale = 10u64.pow(fractional_width as u32);
        let day = serial.floor();
        let units_per_day = 86_400u64 * scale;
        let scaled = (serial.fract() * units_per_day as f64).round() as u64;
        let carry = scaled / units_per_day;
        let within_day = scaled % units_per_day;
        let whole_seconds = within_day / scale;
        (
            day + carry as f64 + whole_seconds as f64 / 86_400.0,
            within_day % scale,
        )
    };

    let (year, month, day, hour, minute, second) = serial_to_datetime(datetime_serial, dates)?;
    let weekday = weekday_of(year, month, day);
    let mut out = String::new();

    for token in &tokens {
        match token {
            DateToken::Year(width) => {
                if *width <= 2 {
                    out.push_str(&format!("{:02}", year % 100));
                } else {
                    out.push_str(&format!("{year:04}"));
                }
            }
            DateToken::Month(width) => match width {
                1 => out.push_str(&month.to_string()),
                2 => out.push_str(&format!("{month:02}")),
                3 => out.push_str(&MONTH_NAMES[(month - 1) as usize][..3]),
                4 => out.push_str(MONTH_NAMES[(month - 1) as usize]),
                _ => out.push_str(&MONTH_NAMES[(month - 1) as usize][..1]),
            },
            DateToken::Day(width) => match width {
                1 => out.push_str(&day.to_string()),
                2 => out.push_str(&format!("{day:02}")),
                3 => out.push_str(&DAY_NAMES[weekday as usize][..3]),
                _ => out.push_str(DAY_NAMES[weekday as usize]),
            },
            DateToken::Hour(width) => {
                let shown = if twelve_hour {
                    match hour % 12 {
                        0 => 12,
                        other => other,
                    }
                } else {
                    hour
                };
                if *width >= 2 {
                    out.push_str(&format!("{shown:02}"));
                } else {
                    out.push_str(&shown.to_string());
                }
            }
            DateToken::Minute(width) => {
                if *width >= 2 {
                    out.push_str(&format!("{minute:02}"));
                } else {
                    out.push_str(&minute.to_string());
                }
            }
            DateToken::Second(width) => {
                if *width >= 2 {
                    out.push_str(&format!("{second:02}"));
                } else {
                    out.push_str(&second.to_string());
                }
            }
            DateToken::FractionalSecond(width) => {
                let width = (*width).min(9);
                let divisor = 10u64.pow((fractional_width - width) as u32);
                out.push('.');
                out.push_str(&format!("{:0width$}", fractional / divisor));
            }
            DateToken::Elapsed(unit, width) => {
                let total_seconds = (serial * 86_400.0).round() as i64;
                let value = match unit {
                    'h' => total_seconds / 3_600,
                    'm' => total_seconds / 60,
                    _ => total_seconds,
                };
                if *width >= 2 {
                    out.push_str(&format!("{value:02}"));
                } else {
                    out.push_str(&value.to_string());
                }
            }
            DateToken::Meridiem { long } => {
                let morning = hour < 12;
                out.push_str(match (long, morning) {
                    (true, true) => "AM",
                    (true, false) => "PM",
                    (false, true) => "A",
                    (false, false) => "P",
                });
            }
            DateToken::Literal(text) => out.push_str(text),
        }
    }
    Some(out)
}

fn tokenize_date(pattern: &str) -> Vec<DateToken> {
    let mut tokens: Vec<DateToken> = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        let c = chars[index];
        let lower = c.to_ascii_lowercase();
        match lower {
            'y' | 'm' | 'd' | 'h' | 's' => {
                let mut width = 0usize;
                while index < chars.len() && chars[index].to_ascii_lowercase() == lower {
                    width += 1;
                    index += 1;
                }
                tokens.push(match lower {
                    'y' => DateToken::Year(width),
                    // Month or minute is decided below, once the neighbours are
                    // known: `m` between an hour and a second is a minute.
                    'm' => DateToken::Month(width),
                    'd' => DateToken::Day(width),
                    'h' => DateToken::Hour(width),
                    _ => DateToken::Second(width),
                });
            }
            '[' => {
                let mut token = String::new();
                index += 1;
                while index < chars.len() && chars[index] != ']' {
                    token.push(chars[index]);
                    index += 1;
                }
                index += 1;
                let unit = token.chars().next().unwrap_or(' ').to_ascii_lowercase();
                if matches!(unit, 'h' | 'm' | 's') {
                    tokens.push(DateToken::Elapsed(unit, token.chars().count()));
                }
            }
            '"' => {
                let mut literal = String::new();
                index += 1;
                while index < chars.len() && chars[index] != '"' {
                    literal.push(chars[index]);
                    index += 1;
                }
                index += 1;
                tokens.push(DateToken::Literal(literal));
            }
            '\\' => {
                index += 1;
                if index < chars.len() {
                    tokens.push(DateToken::Literal(chars[index].to_string()));
                    index += 1;
                }
            }
            'a' | 'p' => {
                let rest: String = chars[index..].iter().collect::<String>().to_lowercase();
                if rest.starts_with("am/pm") {
                    tokens.push(DateToken::Meridiem { long: true });
                    index += 5;
                } else if rest.starts_with("a/p") {
                    tokens.push(DateToken::Meridiem { long: false });
                    index += 3;
                } else {
                    tokens.push(DateToken::Literal(c.to_string()));
                    index += 1;
                }
            }
            '.' => {
                index += 1;
                let mut width = 0usize;
                while index < chars.len() && chars[index] == '0' {
                    width += 1;
                    index += 1;
                }
                if width == 0 {
                    tokens.push(DateToken::Literal(".".to_string()));
                } else {
                    tokens.push(DateToken::FractionalSecond(width.min(9)));
                }
            }
            _ => {
                tokens.push(DateToken::Literal(c.to_string()));
                index += 1;
            }
        }
    }

    resolve_minutes(&mut tokens);
    tokens
}

/// Turns the `m` runs that mean minutes into minutes.
///
/// Excel's rule, and the reason `mm` is ambiguous at all: an `m` is a minute
/// when it follows an hour or precedes a second, ignoring the literals in
/// between.
fn resolve_minutes(tokens: &mut [DateToken]) {
    let significant: Vec<usize> = (0..tokens.len())
        .filter(|index| !matches!(tokens[*index], DateToken::Literal(_)))
        .collect();

    for (position, index) in significant.iter().enumerate() {
        let DateToken::Month(width) = tokens[*index] else {
            continue;
        };
        let follows_hour = position
            .checked_sub(1)
            .and_then(|previous| significant.get(previous))
            .is_some_and(|previous| {
                matches!(
                    tokens[*previous],
                    DateToken::Hour(_) | DateToken::Elapsed('h', _)
                )
            });
        let precedes_second = significant
            .get(position + 1)
            .is_some_and(|next| matches!(tokens[*next], DateToken::Second(_)));
        if follows_hour || precedes_second {
            tokens[*index] = DateToken::Minute(width);
        }
    }
}

/// Turns a serial number into a civil date and time.
///
/// The 1900 system counts from 1899-12-31 with a deliberate off-by-one: Lotus
/// treated 1900 as a leap year and Excel kept the bug for compatibility, so
/// serial 60 is a 29 February that never happened and everything after it is
/// shifted by a day. That is why the epoch below moves once the serial passes
/// it.
fn serial_to_datetime(serial: f64, dates: DateSystem) -> Option<(i64, u32, u32, u32, u32, u32)> {
    let days = serial.trunc() as i64;
    let mut seconds = ((serial - serial.trunc()) * 86_400.0).round() as i64;
    let mut days = days;
    if seconds >= 86_400 {
        seconds -= 86_400;
        days += 1;
    }

    let epoch_days = match dates {
        // 1899-12-30 in days-from-civil terms, plus the leap-year bug.
        DateSystem::Nineteen00 => {
            if days == 60 {
                // The day Excel shows and the calendar does not have.
                return Some((
                    1900,
                    2,
                    29,
                    (seconds / 3_600) as u32,
                    ((seconds % 3_600) / 60) as u32,
                    (seconds % 60) as u32,
                ));
            }
            if days > 60 {
                days_from_civil(1899, 12, 30)
            } else {
                days_from_civil(1899, 12, 31)
            }
        }
        DateSystem::Nineteen04 => days_from_civil(1904, 1, 1),
    };

    let (year, month, day) = civil_from_days(epoch_days + days);
    Some((
        year,
        month,
        day,
        (seconds / 3_600) as u32,
        ((seconds % 3_600) / 60) as u32,
        (seconds % 60) as u32,
    ))
}

/// Calendar fields of the stored value, without display rounding across a
/// second/day boundary. Uses the same calendar and Excel leap-day mapping.
pub fn calendar_fields(serial: f64, dates: DateSystem) -> Option<[u32; 6]> {
    let limit = match dates {
        DateSystem::Nineteen00 => 2_958_466.0,
        DateSystem::Nineteen04 => 2_957_004.0,
    };
    if !(0.0..limit).contains(&serial) {
        return None;
    }
    let day = serial.floor();
    let (year, month, date, _, _, _) = serial_to_datetime(day, dates)?;
    let mut second = ((serial - day) * 86_400.0).floor().min(86_399.0) as u32;
    // Compare canonical serial boundaries instead of introducing an epsilon
    // when multiplication places an exact stored second just below its bin.
    if second > 0 && serial < day + f64::from(second) / 86_400.0 {
        second -= 1;
    } else if second < 86_399 && serial >= day + f64::from(second + 1) / 86_400.0 {
        second += 1;
    }
    Some([
        year as u32,
        month,
        date,
        second / 3_600,
        (second % 3_600) / 60,
        second % 60,
    ])
}

pub fn is_calendar_date_valid(year: u32, month: u32, day: u32, dates: DateSystem) -> bool {
    if !(1..=9_999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    if dates == DateSystem::Nineteen00 && (year, month, day) == (1900, 2, 29) {
        return true;
    }
    civil_from_days(days_from_civil(i64::from(year), month, day)) == (i64::from(year), month, day)
}

/// Days since 1970-01-01 for a civil date, and its inverse.
///
/// Howard Hinnant's `days_from_civil`, which is exact for the whole proleptic
/// Gregorian range and needs no table.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as i64 + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 0 = Sunday, matching the order of [`DAY_NAMES`].
fn weekday_of(year: i64, month: u32, day: u32) -> u32 {
    // 1970-01-01 was a Thursday, so the epoch sits at index 4.
    let days = days_from_civil(year, month, day);
    (((days % 7) + 11) % 7) as u32
}
