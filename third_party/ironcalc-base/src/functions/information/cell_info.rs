//! CELL's answers about a cell's format: the code its number format reads
//! as, and whether it shows negative numbers in colour or positive ones in
//! parentheses.

/// The sections of a number format, split at the semicolons that are not
/// quoted, escaped or inside brackets.
fn sections(format: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    let mut bracket = false;
    let mut escaped = false;
    for character in format.chars() {
        if escaped {
            escaped = false;
        } else if quoted {
            quoted = character != '"';
        } else if bracket {
            bracket = character != ']';
        } else {
            match character {
                '"' => quoted = true,
                '[' => bracket = true,
                '\\' => escaped = true,
                ';' => {
                    sections.push(String::new());
                    continue;
                }
                _ => {}
            }
        }
        if let Some(last) = sections.last_mut() {
            last.push(character);
        }
    }
    sections
}

/// What a section says outside its quoted text, escapes and brackets, and
/// what its brackets hold.
fn bare(section: &str) -> (String, Vec<String>) {
    let mut text = String::new();
    let mut brackets = Vec::new();
    let mut characters = section.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                for inner in characters.by_ref() {
                    if inner == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                characters.next();
            }
            '[' => {
                let mut inner = String::new();
                for next in characters.by_ref() {
                    if next == ']' {
                        break;
                    }
                    inner.push(next);
                }
                brackets.push(inner);
            }
            _ => text.push(character),
        }
    }
    (text, brackets)
}

const COLOURS: [&str; 8] = [
    "black", "blue", "cyan", "green", "magenta", "red", "white", "yellow",
];

fn has_colour(brackets: &[String]) -> bool {
    brackets.iter().any(|inner| {
        let inner = inner.to_lowercase();
        COLOURS.contains(&inner.as_str()) || inner.starts_with("color")
    })
}

/// Whether negative numbers show in a colour: CELL("color").
pub(crate) fn negative_in_colour(format: &str) -> bool {
    sections(format)
        .get(1)
        .is_some_and(|section| has_colour(&bare(section).1))
}

/// Whether positive (or all) numbers show in parentheses:
/// CELL("parentheses").
pub(crate) fn positive_in_parentheses(format: &str) -> bool {
    sections(format)
        .first()
        .is_some_and(|section| bare(section).0.contains('('))
}

/// The code CELL("format") gives a number format: G, F2, ,0, C2, P0, S2,
/// D1 to D9, with "-" for negatives in colour and "()" for parentheses.
pub(crate) fn format_code(format: &str) -> String {
    let all = sections(format);
    let first = all.first().cloned().unwrap_or_default();
    let (text, brackets) = bare(&first);
    let lower = text.to_lowercase();
    let mut code = classify(&lower, &brackets, &first);
    if negative_in_colour(format) {
        code.push('-');
    }
    if positive_in_parentheses(format) {
        code.push_str("()");
    }
    code
}

fn decimals(text: &str) -> usize {
    match text.find('.') {
        Some(point) => text[point + 1..]
            .chars()
            .take_while(|c| matches!(c, '0' | '#' | '?' | ','))
            .filter(|c| matches!(c, '0' | '#' | '?'))
            .count(),
        None => 0,
    }
}

fn classify(lower: &str, brackets: &[String], raw: &str) -> String {
    if lower.trim().is_empty() || lower.contains("general") {
        return "G".to_string();
    }
    let elapsed = brackets.iter().any(|inner| {
        let inner = inner.to_lowercase();
        !inner.is_empty() && inner.chars().all(|c| matches!(c, 'h' | 'm' | 's'))
    });
    let has_digits = lower.contains(['0', '#', '?']);
    let date_letters =
        lower.contains(['d', 'y', 'e', 'g']) && !lower.contains("e+") && !lower.contains("e-");
    let time_letters = lower.contains(['h', 's']) || lower.contains("am/pm") || elapsed;
    let months = lower.contains('m');
    if date_letters || time_letters || (months && !has_digits) {
        return date_code(lower, time_letters);
    }
    if !has_digits {
        return "G".to_string();
    }
    if lower.contains('/') {
        // Fractions.
        return "G".to_string();
    }
    let places = decimals(lower);
    if lower.contains('%') {
        return format!("P{places}");
    }
    if lower.contains("e+") || lower.contains("e-") {
        return format!("S{places}");
    }
    // A currency sign, quoted or not.
    let currency = raw.contains(['$', '¥', '€', '£', '￥'])
        || brackets.iter().any(|inner| inner.starts_with('$'));
    if currency {
        return format!("C{places}");
    }
    let integer = match lower.find('.') {
        Some(point) => &lower[..point],
        None => lower,
    };
    if integer.contains(',') {
        return format!(",{places}");
    }
    format!("F{places}")
}

fn date_code(lower: &str, time: bool) -> String {
    let has_day = lower.contains('d');
    let has_year = lower.contains('y') || lower.contains('e');
    // A month: an m not part of a time (beside h or s).
    let compact: String = lower
        .chars()
        .filter(|c| c.is_alphabetic() || *c == ':')
        .collect();
    let text_month = lower.contains("mmm");
    let has_month = text_month
        || (lower.contains('m') && !(compact.contains("h:m") || compact.contains("m:s"))) && !time;
    if !has_day && !has_year && !has_month {
        let seconds = lower.contains('s');
        let meridiem = lower.contains("am/pm") || lower.contains("a/p");
        return match (meridiem, seconds) {
            (true, true) => "D6",
            (true, false) => "D7",
            (false, true) => "D8",
            (false, false) => "D9",
        }
        .to_string();
    }
    match (has_day, text_month, has_year) {
        (true, true, true) => "D1",
        (true, true, false) => "D2",
        (false, true, _) => "D3",
        (true, false, false) => "D5",
        (false, false, true) if has_month => "D3",
        _ => "D4",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_as_excel_lists_them() {
        for (format, code) in [
            ("General", "G"),
            ("0", "F0"),
            ("#,##0", ",0"),
            ("0.00", "F2"),
            ("#,##0.00", ",2"),
            ("$#,##0_);($#,##0)", "C0"),
            ("$#,##0_);[Red]($#,##0)", "C0-"),
            ("$#,##0.00_);($#,##0.00)", "C2"),
            ("$#,##0.00_);[Red]($#,##0.00)", "C2-"),
            ("0%", "P0"),
            ("0.00%", "P2"),
            ("0.00E+00", "S2"),
            ("# ?/?", "G"),
            ("# ??/??", "G"),
            ("m/d/yy", "D4"),
            ("m/d/yy h:mm", "D4"),
            ("mm/dd/yy", "D4"),
            ("d-mmm-yy", "D1"),
            ("dd-mmm-yy", "D1"),
            ("d-mmm", "D2"),
            ("dd-mmm", "D2"),
            ("mmm-yy", "D3"),
            ("mm/dd", "D5"),
            ("h:mm AM/PM", "D7"),
            ("h:mm:ss AM/PM", "D6"),
            ("h:mm", "D9"),
            ("h:mm:ss", "D8"),
            ("@", "G"),
            ("(0.00)", "F2()"),
            (r##""$"#,##0.00_);[Red]\("$"#,##0.00\)"##, "C2-"),
            ("0;[Red]-0", "F0-"),
        ] {
            assert_eq!(format_code(format), code, "{format}");
        }
    }

    #[test]
    fn colour_and_parentheses() {
        assert!(negative_in_colour("0;[Red]0"));
        assert!(!negative_in_colour("[Red]0;0"));
        assert!(positive_in_parentheses("(0)"));
        assert!(!positive_in_parentheses("0_);(0)"));
    }
}
