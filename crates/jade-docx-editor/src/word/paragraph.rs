//! The 段落 dialog over any [`WordSession`]: インデントと行間隔 — 配置, the
//! left and right indents, 最初の行 (字下げ / ぶら下げ) and its width, the
//! space before and after, 行間 (1 行, 1.5 行, 2 行, 最小値, 固定値, 倍数) —
//! and 改ページと改行 — 改ページ時 1 行残して段落を区切らない, 次の段落と分離
//! しない, 段落を分割しない, 段落前で改ページする.
//!
//! Indents are in characters (字) in a Japanese document, as Word's dialog
//! shows them there, and written as `w:leftChars`, `w:firstLineChars`, …;
//! elsewhere in millimetres. Spacing is in points. Like Word's dialog, OK
//! writes only what was changed, for every paragraph the selection touches,
//! as one undo step.

use jade_docx_headless_api::HeadlessCommand;

use super::count::is_east_asian;
use super::{Helpers, WordFormatting, WordSession};
use crate::text_edit::TextEdit;

/// What 行間 is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineSpacing {
    /// 1 行.
    Single,
    /// 1.5 行.
    OneAndHalf,
    /// 2 行.
    Double,
    /// 最小値, in points.
    AtLeast(f32),
    /// 固定値, in points.
    Exactly(f32),
    /// 倍数, of a single line.
    Multiple(f32),
}

impl LineSpacing {
    /// The engine's terms: the line value and its rule.
    fn command_values(self) -> (f32, &'static str) {
        match self {
            LineSpacing::Single => (1.0, "auto"),
            LineSpacing::OneAndHalf => (1.5, "auto"),
            LineSpacing::Double => (2.0, "auto"),
            LineSpacing::Multiple(times) => (times, "auto"),
            LineSpacing::AtLeast(points) => (points, "atLeast"),
            LineSpacing::Exactly(points) => (points, "exact"),
        }
    }

    /// 間隔's value, when the rule has one.
    pub fn amount(self) -> Option<f32> {
        match self {
            LineSpacing::AtLeast(value)
            | LineSpacing::Exactly(value)
            | LineSpacing::Multiple(value) => Some(value),
            _ => None,
        }
    }
}

/// 最初の行.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialIndent {
    /// (なし).
    None,
    /// 字下げ.
    FirstLine,
    /// ぶら下げ.
    Hanging,
}

/// What the indents are measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentUnit {
    /// 字: a Japanese document's.
    Characters,
    Millimetres,
}

/// What the 段落 dialog shows and sets.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphFormat {
    /// `left`, `center`, `right` or `both`.
    pub alignment: String,
    pub unit: IndentUnit,
    /// In `unit`.
    pub left: f32,
    pub right: f32,
    pub special: SpecialIndent,
    /// 幅, in `unit`.
    pub special_by: f32,
    /// In points.
    pub before: f32,
    pub after: f32,
    pub line: LineSpacing,
    pub widow_control: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
}

const POINTS_PER_MM: f32 = 72.0 / 25.4;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.005
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

/// 段落 over every session.
pub trait WordParagraphs: WordSession {
    /// Whether the dialog measures indents in characters: a document whose
    /// East Asian language is Japanese (`w:lang/@w:eastAsia` of its default
    /// run properties), as a Japanese Word writes it, or one with Japanese
    /// text.
    fn is_japanese_document(&self) -> bool {
        let language = self
            .word_document()
            .styles_part
            .as_ref()
            .and_then(|styles| styles.doc_default_run_properties())
            .and_then(|run| run.east_asia_language.clone());
        if let Some(language) = language {
            return language.to_ascii_lowercase().starts_with("ja");
        }
        let total = self.paragraph_total().min(200);
        (0..total).any(|index| {
            self.text_of(index)
                .is_some_and(|text| text.chars().any(is_east_asian))
        })
    }

    /// The width of a character (字) at the start of `paragraph`: its font
    /// size, Word's 10.5 pt when it has none.
    fn character_width(&self, paragraph: usize) -> f32 {
        self.run_style_at((paragraph, 0))
            .and_then(|style| style.font_size)
            .filter(|size| *size > 0.0)
            .unwrap_or(10.5)
    }

    /// The format of the caret's paragraph, or the first the selection
    /// touches.
    fn paragraph_format(&self) -> ParagraphFormat {
        let (paragraph, _) = self.paragraph_span();
        let unit = if self.is_japanese_document() {
            IndentUnit::Characters
        } else {
            IndentUnit::Millimetres
        };
        let width = self.character_width(paragraph);
        let props = self
            .word_document()
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.clone())
            .unwrap_or_default();
        let indent = props.indent.clone().unwrap_or_default();
        // One edge in the dialog's unit, from its character or point value.
        let measure = |chars: Option<i32>, points: Option<f32>| -> f32 {
            let value = match (unit, chars, points) {
                (IndentUnit::Characters, Some(chars), _) => chars as f32 / 100.0,
                (IndentUnit::Characters, None, Some(points)) => points / width,
                (IndentUnit::Millimetres, Some(chars), None) => {
                    chars as f32 / 100.0 * width / POINTS_PER_MM
                }
                (IndentUnit::Millimetres, _, Some(points)) => points / POINTS_PER_MM,
                _ => 0.0,
            };
            round2(value)
        };
        let left = measure(
            indent.left_chars.or(indent.start_chars),
            indent.left.or(indent.start),
        );
        let right = measure(
            indent.right_chars.or(indent.end_chars),
            indent.right.or(indent.end),
        );
        let first_line = measure(indent.first_line_chars, indent.first_line);
        let hanging = measure(indent.hanging_chars, indent.hanging);
        let (special, special_by) = if hanging > 0.0 {
            (SpecialIndent::Hanging, hanging)
        } else if first_line > 0.0 {
            (SpecialIndent::FirstLine, first_line)
        } else {
            (SpecialIndent::None, 0.0)
        };
        let spacing = props.spacing.clone().unwrap_or_default();
        let line = match (spacing.line_rule.as_deref(), spacing.line) {
            (Some("atLeast"), Some(points)) => LineSpacing::AtLeast(round2(points)),
            (Some("exact" | "exactly"), Some(points)) => LineSpacing::Exactly(round2(points)),
            (_, Some(times)) if close(times, 1.0) => LineSpacing::Single,
            (_, Some(times)) if close(times, 1.5) => LineSpacing::OneAndHalf,
            (_, Some(times)) if close(times, 2.0) => LineSpacing::Double,
            (_, Some(times)) => LineSpacing::Multiple(round2(times)),
            (_, None) => LineSpacing::Single,
        };
        let alignment = match props.alignment.as_deref() {
            Some("center") => "center",
            Some("right" | "end") => "right",
            Some("both" | "justify" | "distribute") => "both",
            _ => "left",
        };
        ParagraphFormat {
            alignment: alignment.to_string(),
            unit,
            left,
            right,
            special,
            special_by,
            before: round2(spacing.before.unwrap_or(0.0)),
            after: round2(spacing.after.unwrap_or(0.0)),
            line,
            widow_control: props.widow_control.unwrap_or(false),
            keep_next: props.keep_next.unwrap_or(false),
            keep_lines: props.keep_lines.unwrap_or(false),
            page_break_before: props.page_break_before.unwrap_or(false),
        }
    }

    /// OK in the dialog: what changed from `was` to `now`, on every paragraph
    /// the selection touches, as one undo step.
    fn set_paragraph_format(
        &mut self,
        was: &ParagraphFormat,
        now: &ParagraphFormat,
    ) -> Result<(), String> {
        let commands = self.paragraph_format_commands(was, now)?;
        self.apply_commands(commands, true)
    }

    /// The commands that take the selected paragraphs from `was` to `now`:
    /// none for what is the same.
    fn paragraph_format_commands(
        &self,
        was: &ParagraphFormat,
        now: &ParagraphFormat,
    ) -> Result<Vec<HeadlessCommand>, String> {
        let (start_paragraph, end_paragraph) = self.paragraph_span();
        let width = self.character_width(start_paragraph);
        let unit = now.unit;
        let points = |value: f32| match unit {
            IndentUnit::Characters => value * width,
            IndentUnit::Millimetres => value * POINTS_PER_MM,
        };
        let chars =
            |value: f32| (unit == IndentUnit::Characters).then(|| (value * 100.0).round() as i32);
        let mut commands = Vec::new();

        if now.alignment != was.alignment {
            commands.push(HeadlessCommand::SetParagraphAlignment {
                start_paragraph,
                end_paragraph,
                alignment: now.alignment.clone(),
            });
        }

        let left = !close(now.left, was.left);
        let right = !close(now.right, was.right);
        let special = now.special != was.special || !close(now.special_by, was.special_by);
        if left || right || special {
            let by = now.special_by.max(0.0);
            let (first_line, hanging, first_line_chars, hanging_chars) =
                match (special, now.special, unit) {
                    (false, ..) | (true, SpecialIndent::None, _) => (None, None, None, None),
                    (true, SpecialIndent::FirstLine, IndentUnit::Characters) => {
                        (None, None, chars(by), None)
                    }
                    (true, SpecialIndent::Hanging, IndentUnit::Characters) => {
                        (None, None, None, chars(by))
                    }
                    (true, SpecialIndent::FirstLine, IndentUnit::Millimetres) => {
                        (Some(points(by)), None, None, None)
                    }
                    (true, SpecialIndent::Hanging, IndentUnit::Millimetres) => {
                        (None, Some(points(by)), None, None)
                    }
                };
            commands.push(HeadlessCommand::SetParagraphIndent {
                start_paragraph,
                end_paragraph,
                left: left.then(|| points(now.left.max(0.0))),
                right: right.then(|| points(now.right.max(0.0))),
                first_line,
                hanging,
                left_chars: if left { chars(now.left.max(0.0)) } else { None },
                right_chars: if right {
                    chars(now.right.max(0.0))
                } else {
                    None
                },
                first_line_chars,
                hanging_chars,
                clear_special: special && (now.special == SpecialIndent::None || by == 0.0),
            });
        }

        let before = !close(now.before, was.before);
        let after = !close(now.after, was.after);
        let line = now.line != was.line;
        if before || after || line {
            let (value, rule) = now.line.command_values();
            if value <= 0.0 {
                return Err("line spacing must be more than 0".to_string());
            }
            commands.push(HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph,
                end_paragraph,
                line: line.then_some(Some(value)),
                line_rule: line.then(|| rule.to_string()),
                before: before.then_some(now.before.max(0.0)),
                after: after.then_some(now.after.max(0.0)),
            });
        }

        let changed = |now: bool, was: bool| (now != was).then_some(now);
        let keep_next = changed(now.keep_next, was.keep_next);
        let keep_lines = changed(now.keep_lines, was.keep_lines);
        let page_break_before = changed(now.page_break_before, was.page_break_before);
        let widow_control = changed(now.widow_control, was.widow_control);
        if keep_next.is_some()
            || keep_lines.is_some()
            || page_break_before.is_some()
            || widow_control.is_some()
        {
            commands.push(HeadlessCommand::SetParagraphPagination {
                start_paragraph,
                end_paragraph,
                keep_next,
                keep_lines,
                page_break_before,
                widow_control,
            });
        }
        Ok(commands)
    }
}

impl<T: WordSession + ?Sized> WordParagraphs for T {}

/// Which of the dialog's number fields typing goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParagraphField {
    Left,
    Right,
    SpecialBy,
    Before,
    After,
    LineAmount,
}

impl ParagraphField {
    pub const ALL: [ParagraphField; 6] = [
        ParagraphField::Left,
        ParagraphField::Right,
        ParagraphField::SpecialBy,
        ParagraphField::Before,
        ParagraphField::After,
        ParagraphField::LineAmount,
    ];
}

/// The 段落 dialog while it is open: the format it opened on, and what it
/// now says — its choices, and its number fields as typed.
pub struct ParagraphForm {
    pub original: ParagraphFormat,
    pub format: ParagraphFormat,
    pub fields: [TextEdit; 6],
    /// The field typing goes into.
    pub focus: ParagraphField,
}

/// A number as the dialog shows it: no trailing zeros.
pub fn shown_number(value: f32) -> String {
    let text = format!("{:.2}", round2(value));
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A number as typed, full-width digits and a trailing unit allowed.
fn parse_number(text: &str) -> Option<f32> {
    let half: String = text
        .chars()
        .map(|ch| match ch {
            '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
            '．' => '.',
            '－' => '-',
            _ => ch,
        })
        .collect();
    let number: String = half
        .trim()
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-'))
        .collect();
    if number.is_empty() {
        // Nothing typed is 0, as Word's spin boxes read it.
        return half.trim().is_empty().then_some(0.0);
    }
    number.parse::<f32>().ok().filter(|value| value.is_finite())
}

impl ParagraphForm {
    pub fn new(format: ParagraphFormat) -> Self {
        let field = |value: f32| TextEdit::selected(shown_number(value));
        let fields = [
            field(format.left),
            field(format.right),
            field(format.special_by),
            field(format.before),
            field(format.after),
            field(format.line.amount().unwrap_or(0.0)),
        ];
        Self {
            original: format.clone(),
            format,
            fields,
            focus: ParagraphField::Left,
        }
    }

    fn index(field: ParagraphField) -> usize {
        ParagraphField::ALL
            .iter()
            .position(|candidate| *candidate == field)
            .unwrap_or(0)
    }

    pub fn field(&self, field: ParagraphField) -> &TextEdit {
        &self.fields[Self::index(field)]
    }

    pub fn focused(&self) -> &TextEdit {
        self.field(self.focus)
    }

    pub fn focused_mut(&mut self) -> &mut TextEdit {
        &mut self.fields[Self::index(self.focus)]
    }

    /// Puts typing in `field`, its text selected so typing replaces it.
    pub fn focus_on(&mut self, field: ParagraphField) {
        self.focus = field;
        let edit = self.focused_mut();
        edit.whole = true;
        edit.caret = edit.text.len();
    }

    /// Tab and ⇧Tab: the next or the previous field.
    pub fn step_focus(&mut self, forwards: bool) {
        let index = Self::index(self.focus);
        let count = ParagraphField::ALL.len();
        let next = if forwards {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        self.focus_on(ParagraphField::ALL[next]);
    }

    /// 行間 chosen from its list: the amount field takes the rule's usual
    /// value when it has none yet.
    pub fn set_line_rule(&mut self, line: LineSpacing) {
        let typed = parse_number(&self.field(ParagraphField::LineAmount).text).unwrap_or(0.0);
        let line = match line {
            LineSpacing::AtLeast(_) | LineSpacing::Exactly(_) => {
                let points = if typed >= 1.0 { typed } else { 12.0 };
                if matches!(line, LineSpacing::AtLeast(_)) {
                    LineSpacing::AtLeast(points)
                } else {
                    LineSpacing::Exactly(points)
                }
            }
            LineSpacing::Multiple(_) => LineSpacing::Multiple(if typed > 0.0 && typed < 10.0 {
                typed
            } else {
                3.0
            }),
            other => other,
        };
        if let Some(amount) = line.amount() {
            self.fields[Self::index(ParagraphField::LineAmount)] =
                TextEdit::selected(shown_number(amount));
        }
        self.format.line = line;
    }

    /// What the dialog says, its fields read: an error names a field that is
    /// not a number.
    pub fn read(&self) -> Result<ParagraphFormat, String> {
        let number = |field: ParagraphField| {
            parse_number(&self.field(field).text)
                .ok_or_else(|| format!("not a number: {}", self.field(field).text))
        };
        let mut format = self.format.clone();
        format.left = number(ParagraphField::Left)?;
        format.right = number(ParagraphField::Right)?;
        format.special_by = number(ParagraphField::SpecialBy)?;
        format.before = number(ParagraphField::Before)?;
        format.after = number(ParagraphField::After)?;
        let amount = number(ParagraphField::LineAmount)?;
        format.line = match format.line {
            LineSpacing::AtLeast(_) => LineSpacing::AtLeast(amount),
            LineSpacing::Exactly(_) => LineSpacing::Exactly(amount),
            LineSpacing::Multiple(_) => LineSpacing::Multiple(amount),
            other => other,
        };
        if format.special != SpecialIndent::None && format.special_by <= 0.0 {
            format.special = SpecialIndent::None;
        }
        Ok(format)
    }

    /// OK.
    pub fn submit<S: WordSession + ?Sized>(&self, session: &mut S) -> Result<(), String> {
        let format = self.read()?;
        session.set_paragraph_format(&self.original, &format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_read_as_typed_and_shown_short() {
        assert_eq!(parse_number("1.5"), Some(1.5));
        assert_eq!(parse_number("２字"), Some(2.0));
        assert_eq!(parse_number("12 pt"), Some(12.0));
        assert_eq!(parse_number(""), Some(0.0));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("1.2.3"), None);
        assert_eq!(shown_number(2.0), "2");
        assert_eq!(shown_number(10.5), "10.5");
        assert_eq!(shown_number(0.0), "0");
    }
}
