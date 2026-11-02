//! Excel's calculation options that change what a formula's value is:
//! iterative calculation, which lets a circular reference settle instead of
//! being an error, and precision as displayed, which keeps each result only
//! to the digits its number format shows.

/// A cell's number format, by sheet index and one-based row and column.
pub type NumberFormats = Box<dyn Fn(u32, i32, i32) -> Option<String>>;

/// How the model calculates.
#[derive(Default)]
pub struct Calculation {
    /// Iterative calculation: at most this many passes, stopping once no
    /// value changes by more than the second number.
    pub iteration: Option<(u32, f64)>,
    /// Precision as displayed: each formula's number kept as its cell's
    /// format shows it.
    pub displayed: Option<NumberFormats>,
}

/// `value` as `format` shows it, which is what precision as displayed
/// keeps: `0.00` keeps two decimals, `0%` whole percents, `#,##0,` whole
/// thousands, `0.0E+00` two significant digits, a date with no time whole
/// days and a time to the second (or to the fractions of a second shown).
/// `General`, text and fractions keep the number as it is.
pub fn displayed_value(value: f64, format: &str) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let sections = split_sections(format);
    let section = match (sections.len(), value) {
        (0, _) => return value,
        (_, v) if v > 0.0 => sections[0].as_str(),
        (1, _) => sections[0].as_str(),
        (2, v) if v < 0.0 => sections[1].as_str(),
        (2, _) => sections[0].as_str(),
        (_, v) if v < 0.0 => sections[1].as_str(),
        (_, _) => sections[2].as_str(),
    };
    let bare = bare_section(section);
    let lower = bare.to_ascii_lowercase();
    if lower.trim().is_empty() || lower.contains("general") || bare.contains('@') {
        return value;
    }
    if bare.contains('/') && !lower.contains(['y', 'd', 'h', 's']) {
        // A fraction shows a nearby value, not a rounded one.
        return value;
    }
    if lower.contains(['y', 'd', 'h', 's']) || lower.contains('m') && !lower.contains(['0', '#']) {
        return displayed_date(value, &lower);
    }
    let scientific = lower.find('e');
    let mantissa = match scientific {
        Some(at) => &bare[..at],
        None => bare.as_str(),
    };
    let decimals = match mantissa.find('.') {
        Some(at) => mantissa[at + 1..]
            .chars()
            .filter(|c| matches!(c, '0' | '#' | '?'))
            .count() as i32,
        None => 0,
    };
    if scientific.is_some() {
        if value == 0.0 {
            return 0.0;
        }
        let exponent = value.abs().log10().floor() as i32;
        return round_to(value / 10f64.powi(exponent), decimals) * 10f64.powi(exponent);
    }
    let percents = bare.matches('%').count() as i32;
    // Commas right after the last digit placeholder scale by a thousand each.
    let last_digit = mantissa.rfind(['0', '#', '?']);
    let thousands = match last_digit {
        Some(at) => mantissa[at + 1..].chars().take_while(|c| *c == ',').count() as i32,
        None => 0,
    };
    // Each % shows two more decimals of the number, each scaling comma three
    // fewer.
    round_to(value, decimals + 2 * percents - 3 * thousands)
}

fn displayed_date(value: f64, lower: &str) -> f64 {
    let lower = lower.replace("am/pm", "").replace("a/p", "");
    let units = if lower.contains('s') {
        // Seconds and their shown fractions: `ss.00`.
        let fractions = match lower.find("s.") {
            Some(at) => lower[at + 2..].chars().take_while(|c| *c == '0').count() as i32,
            None => 0,
        };
        86_400.0 * 10f64.powi(fractions)
    } else if lower.contains('h') && lower.contains('m') {
        1_440.0
    } else if lower.contains('h') {
        24.0
    } else {
        // A date alone: whole days.
        1.0
    };
    (value * units).round() / units
}

/// `value` to `decimals` places, or to tens, hundreds … when negative.
fn round_to(value: f64, decimals: i32) -> f64 {
    // Excel rounds half away from zero, after trimming float noise.
    let trim = |scaled: f64| ((scaled * 1e9).round() / 1e9).round();
    if decimals >= 0 {
        let factor = 10f64.powi(decimals);
        trim(value * factor) / factor
    } else {
        let factor = 10f64.powi(-decimals);
        trim(value / factor) * factor
    }
}

/// The format's sections, split on `;` outside quotes and escapes.
fn split_sections(format: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                if let Some(section) = sections.last_mut() {
                    section.push(c);
                }
            }
            '\\' | '_' | '*' if !quoted => {
                let section = sections.last_mut().expect("a section");
                section.push(c);
                if let Some(next) = chars.next() {
                    section.push(next);
                }
            }
            ';' if !quoted => sections.push(String::new()),
            _ => {
                if let Some(section) = sections.last_mut() {
                    section.push(c);
                }
            }
        }
    }
    sections
}

/// A section without its quoted text, escaped and padding characters and
/// bracketed colours, conditions and locales.
fn bare_section(section: &str) -> String {
    let mut bare = String::new();
    let mut chars = section.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for next in chars.by_ref() {
                    if next == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            '[' => {
                let inner: String = chars.by_ref().take_while(|c| *c != ']').collect();
                // Elapsed time, `[h]:mm`, is time.
                if matches!(
                    inner.to_ascii_lowercase().as_str(),
                    "h" | "hh" | "m" | "mm" | "s" | "ss"
                ) {
                    bare.push_str(&inner);
                }
            }
            _ => bare.push(c),
        }
    }
    bare
}

#[cfg(test)]
mod tests {
    use super::displayed_value;

    #[test]
    fn numbers_keep_the_digits_shown() {
        assert_eq!(displayed_value(1.23456, "0.00"), 1.23);
        assert_eq!(displayed_value(2.5, "0"), 3.0);
        assert_eq!(displayed_value(-2.5, "0"), -3.0);
        assert_eq!(displayed_value(0.12345, "0.0%"), 0.123);
        assert_eq!(displayed_value(1234567.0, "#,##0,"), 1235000.0);
        assert_eq!(
            displayed_value(1234.5678, "$#,##0.00_);[Red]($#,##0.00)"),
            1234.57
        );
        assert_eq!(
            displayed_value(-1234.5678, "$#,##0.00_);[Red]($#,##0.00)"),
            -1234.57
        );
        assert_eq!(displayed_value(12345.678, "0.0E+00"), 12000.0);
        assert_eq!(displayed_value(1.23456, "General"), 1.23456);
        assert_eq!(displayed_value(1.23456, "\"Total: \"0.0"), 1.2);
        assert_eq!(displayed_value(0.3, "# ?/?"), 0.3);
    }

    #[test]
    fn dates_and_times_keep_what_is_shown() {
        assert_eq!(displayed_value(45000.75, "yyyy-mm-dd"), 45001.0);
        assert_eq!(displayed_value(45000.4, "d-mmm-yy"), 45000.0);
        let noon_and_a_bit = 0.5 + 0.4 / 86_400.0;
        assert_eq!(displayed_value(noon_and_a_bit, "h:mm:ss"), 0.5);
        assert_eq!(displayed_value(1.5, "[h]:mm"), 1.5);
        assert_eq!(displayed_value(0.5 + 20.0 / 86_400.0, "h:mm"), 0.5);
        assert_eq!(displayed_value(0.26, "h AM/PM"), 0.25);
    }
}
