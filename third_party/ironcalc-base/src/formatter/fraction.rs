//! Fractions (Jade addition): `# ?/?`, `?/??`, `# ?/8` and the like, as the
//! TEXT function and the display print them. Excel picks the closest
//! fraction whose denominator fits the placeholders (or the fixed one),
//! pads `?` with spaces and `0` with zeros, and drops a zero whole part under
//! `#`.

use crate::locale::Locale;

use super::{
    format::{format_number, select_conditional_section, Formatted},
    lexer::{split_sections, Compare, Lexer, Token},
};

/// One section of `format` laid out for `value`, when the section that
/// applies is a fraction; `None` sends the format to the general path.
pub(crate) fn format_fraction(value: f64, format: &str, locale: &Locale) -> Option<Formatted> {
    let sections = split_sections(format);
    if sections.len() > 4 {
        return None;
    }
    let metadata = sections
        .iter()
        .map(|section| prefix(section))
        .collect::<Vec<_>>();
    let patterns = metadata
        .iter()
        .map(|(body, _, _)| Pattern::parse(body))
        .collect::<Vec<_>>();
    if !patterns.iter().take(3).any(Option::is_some) {
        return None;
    }
    let conditional = sections.len() > 1
        && metadata
            .iter()
            .take(2)
            .any(|(_, condition, _)| condition.is_some());
    let (index, signed_value) = if conditional {
        let conditions = metadata
            .iter()
            .map(|(_, condition, _)| condition.as_ref())
            .collect::<Vec<_>>();
        let index = select_conditional_section(&conditions, value)?;
        (index, value)
    } else {
        match sections.len() {
            2..=4 if value < 0.0 => (1, -value),
            3 | 4 if value == 0.0 => (2, 0.0),
            _ => (0, value),
        }
    };
    let Some(pattern) = &patterns[index] else {
        // A mixed format may select a decimal, date, literal or General section.
        // Pass only that section so it cannot be selected again by sign.
        return Some(format_number(signed_value, &sections[index], locale));
    };
    let mut text = pattern.render(signed_value.abs());
    if signed_value < 0.0 {
        text.insert(0, '-');
    }
    Some(Formatted {
        text,
        color: metadata[index].2,
        error: None,
    })
}

/// Remove only recognized condition/color prefixes. Unknown bracket codes and
/// quoted or escaped text remain part of the pattern and must not be discarded.
fn prefix(mut section: &str) -> (&str, Option<(Compare, f64)>, Option<i32>) {
    let (mut condition, mut color) = (None, None);
    while section.starts_with('[') {
        let Some(end) = section.find(']') else { break };
        let mut lexer = Lexer::new(&section[..=end]);
        match lexer.next_token() {
            Token::Condition(compare, threshold) => condition = Some((compare, threshold)),
            Token::Color(index) => color = Some(index),
            _ => break,
        }
        section = &section[end + 1..];
    }
    (section, condition, color)
}

struct Pattern {
    /// Placeholders for the whole part, when the pattern has one.
    whole: Option<String>,
    numerator: String,
    denominator: Denominator,
}

enum Denominator {
    /// The most digits a denominator may have, and the placeholders.
    Placeholders(String),
    Fixed(u64),
}

fn placeholder(c: char) -> bool {
    matches!(c, '#' | '0' | '?')
}

impl Pattern {
    /// `[whole ]numerator/denominator`, nothing else: anything around it is
    /// a format this does not take on.
    fn parse(section: &str) -> Option<Pattern> {
        let section = section.trim();
        let (left, right) = section.split_once('/')?;
        let denominator = if !right.is_empty() && right.chars().all(placeholder) {
            Denominator::Placeholders(right.to_string())
        } else if !right.is_empty() && right.chars().all(|c| c.is_ascii_digit()) {
            Denominator::Fixed(right.parse().ok().filter(|d| *d > 0)?)
        } else {
            return None;
        };
        let left = left.trim_end();
        let (whole, numerator) = match left.rsplit_once(' ') {
            Some((whole, numerator)) => (Some(whole.trim_end().to_string()), numerator),
            None => (None, left),
        };
        if numerator.is_empty()
            || !numerator.chars().all(placeholder)
            || whole
                .as_ref()
                .is_some_and(|whole| whole.is_empty() || !whole.chars().all(placeholder))
        {
            return None;
        }
        Some(Pattern {
            whole,
            numerator: numerator.to_string(),
            denominator,
        })
    }

    fn render(&self, value: f64) -> String {
        let (mut whole, fraction) = if self.whole.is_some() {
            (value.trunc() as u64, value.fract())
        } else {
            (0, value)
        };
        let (mut numerator, denominator) = match &self.denominator {
            Denominator::Fixed(denominator) => (
                (fraction * *denominator as f64).round() as u64,
                *denominator,
            ),
            Denominator::Placeholders(places) => {
                closest(fraction, 10u64.pow(places.len().min(6) as u32) - 1)
            }
        };
        if self.whole.is_some() && numerator >= denominator {
            whole += numerator / denominator;
            numerator %= denominator;
        }
        let mut out = String::new();
        if let Some(places) = &self.whole {
            out.push_str(&pad_left(whole, places, true));
            out.push(' ');
        }
        if numerator == 0 && self.whole.is_some() {
            // A whole number shows its whole part and blanks for the rest.
            let width = self.numerator.len() + 1 + self.denominator_width();
            out.push_str(&" ".repeat(width));
            return out;
        }
        out.push_str(&pad_left(numerator, &self.numerator, false));
        out.push('/');
        let denominator_text = denominator.to_string();
        out.push_str(&denominator_text);
        if let Denominator::Placeholders(places) = &self.denominator {
            for c in places.chars().skip(denominator_text.len()) {
                if c == '?' {
                    out.push(' ');
                }
            }
        }
        out
    }

    fn denominator_width(&self) -> usize {
        match &self.denominator {
            Denominator::Placeholders(places) => places.len(),
            Denominator::Fixed(denominator) => denominator.to_string().len(),
        }
    }
}

/// `value` right-aligned in `places`: `0` pads with a zero, `?` with a
/// space, `#` with nothing. A zero whole part under `#` alone is blank.
fn pad_left(value: u64, places: &str, whole: bool) -> String {
    let digits = if value == 0 && whole && !places.contains('0') {
        String::new()
    } else {
        value.to_string()
    };
    let mut out = String::new();
    let missing = places.chars().count().saturating_sub(digits.len());
    for c in places.chars().take(missing) {
        match c {
            '0' => out.push('0'),
            '?' => out.push(' '),
            _ => {}
        }
    }
    out.push_str(&digits);
    out
}

/// The fraction closest to `value` with a denominator up to `limit`.
fn closest(value: f64, limit: u64) -> (u64, u64) {
    let mut best = (value.round() as u64, 1);
    let mut best_error = (value - best.0 as f64).abs();
    for denominator in 2..=limit.max(1) {
        let numerator = (value * denominator as f64).round() as u64;
        let error = (value - numerator as f64 / denominator as f64).abs();
        if error < best_error - 1e-12 {
            best = (numerator, denominator);
            best_error = error;
        }
    }
    best
}
