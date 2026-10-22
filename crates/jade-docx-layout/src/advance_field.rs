//! Word's `ADVANCE` field (ECMA-376 Part 1 §17.16.5.3): no result of its own,
//! it moves the text that follows it — `\d` down, `\u` up, `\l` left, `\r`
//! right by so many points, `\x` to a distance from the column's left edge,
//! `\y` the whole line to a position down the page.

/// One `ADVANCE` field's switches, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdvanceField {
    pub down: f32,
    pub up: f32,
    pub left: f32,
    pub right: f32,
    /// `\x`: from the left edge of the column (or frame, or text box).
    pub x: Option<f32>,
    /// `\y`: the line's position down the page.
    pub y: Option<f32>,
}

impl AdvanceField {
    /// Parses an `ADVANCE` instruction; `None` for any other field.
    pub fn parse(instruction: &str) -> Option<Self> {
        let mut tokens = instruction.split_whitespace();
        if !tokens.next()?.eq_ignore_ascii_case("ADVANCE") {
            return None;
        }
        let mut field = AdvanceField::default();
        let tokens: Vec<&str> = tokens.collect();
        let mut index = 0;
        while index < tokens.len() {
            let switch = tokens[index].to_ascii_lowercase();
            let value = tokens
                .get(index + 1)
                .map(|value| value.trim_matches('"'))
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite());
            if let Some(value) = value {
                match switch.as_str() {
                    "\\d" => field.down += value,
                    "\\u" => field.up += value,
                    "\\l" => field.left += value,
                    "\\r" => field.right += value,
                    "\\x" => field.x = Some(value),
                    "\\y" => field.y = Some(value),
                    _ => {}
                }
                index += 2;
            } else {
                index += 1;
            }
        }
        Some(field)
    }

    /// The horizontal move in points (right positive), apart from `\x`.
    pub fn dx(&self) -> f32 {
        self.right - self.left
    }

    /// How far the following text rises, in points (`w:position`'s sense).
    pub fn raise(&self) -> f32 {
        self.up - self.down
    }
}

/// Where an `ADVANCE` field sits in a paragraph's layout text, and what it
/// does to the text after it there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphAdvance {
    /// Byte offset in the paragraph's layout text.
    pub offset: usize,
    /// Horizontal move in px.
    pub dx_px: f64,
    /// `\x` in px from the paragraph's text edge (its left indent): the
    /// field's distance from the column edge less that indent. The line's
    /// own first-line or hanging indent is taken off where it applies.
    pub x_px: Option<f64>,
    /// `\y` in px down the page: where the line holding the field goes.
    pub y_px: Option<f64>,
    /// `\u` less `\d`, in points: how far the text after the field rises
    /// on the field's line. The caller raises every run after the field by
    /// it; the wrapper lowers the runs on later lines back (Word moves the
    /// rest of that line only).
    pub raise_pt: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_are_read_in_points() {
        let field = AdvanceField::parse("ADVANCE \\d 4 \\r 10.5").unwrap();
        assert_eq!(field.down, 4.0);
        assert_eq!(field.raise(), -4.0);
        assert_eq!(field.dx(), 10.5);
        let field = AdvanceField::parse("advance \\u 3 \\l 2 \\x 72 \\y 144").unwrap();
        assert_eq!((field.raise(), field.dx()), (3.0, -2.0));
        assert_eq!((field.x, field.y), (Some(72.0), Some(144.0)));
        assert!(AdvanceField::parse("PAGE").is_none());
    }
}
