//! The rulers' measure: the unit Word shows lengths in (ルーラーの単位 —
//! mm, cm, inches, points, or 字, the width of a character of the body
//! text), the ticks and numbers a ruler draws in it, where a click on the
//! ruler snaps to, and how a length is written and read in a dialog's
//! field ("12.7 mm", "3 字", "0.5\"").
//!
//! Lengths are points throughout; only what is shown is in the unit.

/// A unit lengths are shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RulerUnit {
    Millimeters,
    Centimeters,
    Inches,
    Points,
    /// 字: one character of the document's body font size.
    Characters,
}

impl RulerUnit {
    pub const ALL: [RulerUnit; 5] = [
        RulerUnit::Characters,
        RulerUnit::Millimeters,
        RulerUnit::Centimeters,
        RulerUnit::Inches,
        RulerUnit::Points,
    ];

    /// A stable name, for keeping the choice.
    pub fn key(self) -> &'static str {
        match self {
            RulerUnit::Millimeters => "mm",
            RulerUnit::Centimeters => "cm",
            RulerUnit::Inches => "in",
            RulerUnit::Points => "pt",
            RulerUnit::Characters => "char",
        }
    }

    pub fn from_key(key: &str) -> Option<RulerUnit> {
        RulerUnit::ALL
            .into_iter()
            .find(|unit| unit.key() == key.trim())
    }

    /// The suffix a dialog writes after a length.
    pub fn suffix(self) -> &'static str {
        match self {
            RulerUnit::Millimeters => "mm",
            RulerUnit::Centimeters => "cm",
            RulerUnit::Inches => "\"",
            RulerUnit::Points => "pt",
            RulerUnit::Characters => "字",
        }
    }

    /// Points in one unit; a character is `char_width` points.
    pub fn points(self, char_width: f32) -> f32 {
        match self {
            RulerUnit::Millimeters => 72.0 / 25.4,
            RulerUnit::Centimeters => 72.0 / 2.54,
            RulerUnit::Inches => 72.0,
            RulerUnit::Points => 1.0,
            RulerUnit::Characters => char_width.max(1.0),
        }
    }

    /// The spacing of the ruler's small ticks, and how many make a numbered
    /// one, as Word draws them.
    fn ticks(self) -> (f32, usize) {
        match self {
            // A tick every 2 mm, numbered every 10.
            RulerUnit::Millimeters => (2.0, 5),
            // A tick every 2.5 mm, numbered every centimetre... in cm.
            RulerUnit::Centimeters => (0.25, 4),
            RulerUnit::Inches => (0.125, 8),
            RulerUnit::Points => (6.0, 6),
            // A tick every character, numbered every two.
            RulerUnit::Characters => (1.0, 2),
        }
    }
}

/// One tick of a ruler.
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    /// Points from the ruler's zero (the left margin).
    pub at: f32,
    /// A numbered tick carries its number.
    pub label: Option<String>,
    /// Halfway between numbered ticks, drawn longer.
    pub half: bool,
}

/// The ticks from `from` to `to` points (either side of zero, which is the
/// margin), in `unit`.
pub fn ticks(unit: RulerUnit, char_width: f32, from: f32, to: f32) -> Vec<Tick> {
    let (step, every) = unit.ticks();
    let step_points = step * unit.points(char_width);
    if step_points <= 0.0 || to <= from {
        return Vec::new();
    }
    let first = (from / step_points).ceil() as i64;
    let last = (to / step_points).floor() as i64;
    (first..=last)
        .map(|index| {
            let at = index as f32 * step_points;
            let numbered = index % every as i64 == 0 && index != 0;
            let half = every >= 4 && index % (every as i64 / 2) == 0 && !numbered;
            let value = (index as f32 * step).abs();
            Tick {
                at,
                label: numbered.then(|| trim_number(value)),
                half: half && index != 0,
            }
        })
        .collect()
}

fn trim_number(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Where a click on the ruler lands: the nearest small tick.
pub fn snap(unit: RulerUnit, char_width: f32, at: f32) -> f32 {
    let (step, _) = unit.ticks();
    let step_points = step * unit.points(char_width);
    if step_points <= 0.0 {
        return at;
    }
    (at / step_points).round() * step_points
}

/// A length in `unit`, as a field shows it: `12.7 mm`, `3 字`.
pub fn format_length(unit: RulerUnit, char_width: f32, points: f32) -> String {
    let value = points / unit.points(char_width);
    let digits = match unit {
        RulerUnit::Points | RulerUnit::Millimeters => 1,
        _ => 2,
    };
    let text = format!("{value:.digits$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    let text = if text == "-0" { "0" } else { text };
    format!("{text} {}", unit.suffix())
}

/// A length typed into a field, in points: a number, with or without a unit
/// (`mm`, `cm`, `in`, `"`, `pt`, `字`, `ch`); a bare number is in `unit`.
pub fn parse_length(unit: RulerUnit, char_width: f32, text: &str) -> Option<f32> {
    let text = text.trim().replace(['，', '．'], ".");
    let text: String = text
        .chars()
        .map(|ch| match ch {
            '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
            '－' => '-',
            ch => ch,
        })
        .collect();
    let split = text
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+'))
        .unwrap_or(text.len());
    let (number, suffix) = text.split_at(split);
    let value: f32 = number.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let unit = match suffix.trim().to_ascii_lowercase().as_str() {
        "" => unit,
        "mm" | "ミリ" => RulerUnit::Millimeters,
        "cm" | "センチ" => RulerUnit::Centimeters,
        "in" | "\"" | "inch" | "インチ" => RulerUnit::Inches,
        "pt" | "ポイント" => RulerUnit::Points,
        "字" | "ch" | "char" | "chars" => RulerUnit::Characters,
        _ => return None,
    };
    Some(value * unit.points(char_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_numbered_in_the_unit() {
        let ticks = ticks(RulerUnit::Centimeters, 10.5, 0.0, 72.0);
        let labels: Vec<_> = ticks.iter().filter_map(|tick| tick.label.clone()).collect();
        assert_eq!(labels, ["1", "2"]);
        let chars = super::ticks(RulerUnit::Characters, 10.5, -21.0, 42.0);
        assert_eq!(chars.first().unwrap().at, -21.0);
        assert_eq!(
            chars
                .iter()
                .filter_map(|tick| tick.label.clone())
                .collect::<Vec<_>>(),
            ["2", "2", "4"]
        );
    }

    #[test]
    fn lengths_read_and_write_in_any_unit() {
        assert_eq!(format_length(RulerUnit::Millimeters, 10.5, 36.0), "12.7 mm");
        assert_eq!(format_length(RulerUnit::Characters, 10.5, 21.0), "2 字");
        assert_eq!(format_length(RulerUnit::Inches, 10.5, 36.0), "0.5 \"");
        assert_eq!(parse_length(RulerUnit::Characters, 10.5, "4"), Some(42.0));
        assert_eq!(
            parse_length(RulerUnit::Characters, 10.5, "１２．７ mm"),
            Some(36.0)
        );
        assert_eq!(parse_length(RulerUnit::Points, 10.5, "1in"), Some(72.0));
        assert_eq!(parse_length(RulerUnit::Points, 10.5, "2 字"), Some(21.0));
        assert_eq!(parse_length(RulerUnit::Points, 10.5, "wide"), None);
        assert_eq!(
            snap(RulerUnit::Millimeters, 10.5, 30.0),
            2.0 * 72.0 / 25.4 * 5.0
        );
    }
}
