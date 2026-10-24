//! 挿入's 記号と特殊文字 and 日付と時刻 over any [`WordSession`]: a symbol
//! or special character typed in at the caret (or over the selection), and
//! today's date or the time in one of Word's formats — as text, or with
//! 自動的に更新する as a `DATE` / `TIME` field that shows the date it is
//! opened on.

use crate::field_datetime::format_word_date;
pub use crate::field_datetime::FieldDateTime;
use jade_docx_headless_api::HeadlessCommand;

use super::{Position, WordSession};

/// A block of characters the 記号と特殊文字 dialog's 種類 list offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolSubset {
    /// The shell's i18n key for its name.
    pub key: &'static str,
    pub first: u32,
    pub last: u32,
}

/// Word's 種類, in Unicode order.
pub const SYMBOL_SUBSETS: [SymbolSubset; 20] = [
    SymbolSubset {
        key: "chars.symbol.subset.latin",
        first: 0x21,
        last: 0x7E,
    },
    SymbolSubset {
        key: "chars.symbol.subset.latin1",
        first: 0xA1,
        last: 0xFF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.greek",
        first: 0x391,
        last: 0x3C9,
    },
    SymbolSubset {
        key: "chars.symbol.subset.cyrillic",
        first: 0x410,
        last: 0x44F,
    },
    SymbolSubset {
        key: "chars.symbol.subset.punctuation",
        first: 0x2010,
        last: 0x205E,
    },
    SymbolSubset {
        key: "chars.symbol.subset.currency",
        first: 0x20A0,
        last: 0x20C0,
    },
    SymbolSubset {
        key: "chars.symbol.subset.letterlike",
        first: 0x2100,
        last: 0x214F,
    },
    SymbolSubset {
        key: "chars.symbol.subset.number_forms",
        first: 0x2150,
        last: 0x218B,
    },
    SymbolSubset {
        key: "chars.symbol.subset.arrows",
        first: 0x2190,
        last: 0x21FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.math",
        first: 0x2200,
        last: 0x22FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.technical",
        first: 0x2300,
        last: 0x23FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.enclosed",
        first: 0x2460,
        last: 0x24FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.box",
        first: 0x2500,
        last: 0x257F,
    },
    SymbolSubset {
        key: "chars.symbol.subset.shapes",
        first: 0x25A0,
        last: 0x25FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.misc",
        first: 0x2600,
        last: 0x26FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.cjk",
        first: 0x3001,
        last: 0x303F,
    },
    SymbolSubset {
        key: "chars.symbol.subset.hiragana",
        first: 0x3041,
        last: 0x309F,
    },
    SymbolSubset {
        key: "chars.symbol.subset.katakana",
        first: 0x30A0,
        last: 0x30FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.enclosed_cjk",
        first: 0x3220,
        last: 0x33FF,
    },
    SymbolSubset {
        key: "chars.symbol.subset.fullwidth",
        first: 0xFF01,
        last: 0xFFEE,
    },
];

impl SymbolSubset {
    /// The subset's characters, without the unassigned and control points.
    pub fn characters(&self) -> Vec<char> {
        (self.first..=self.last)
            .filter_map(char::from_u32)
            .filter(|ch| !ch.is_control() && !ch.is_whitespace())
            .collect()
    }

    /// The subset `ch` is in, by index.
    pub fn of(ch: char) -> Option<usize> {
        SYMBOL_SUBSETS
            .iter()
            .position(|subset| (subset.first..=subset.last).contains(&(ch as u32)))
    }
}

/// 特殊文字: Word's list, as the character and the i18n key of its name.
pub const SPECIAL_CHARACTERS: [(char, &str); 16] = [
    ('\u{2014}', "chars.special.em_dash"),
    ('\u{2013}', "chars.special.en_dash"),
    ('\u{2011}', "chars.special.nonbreaking_hyphen"),
    ('\u{00AD}', "chars.special.optional_hyphen"),
    ('\u{2003}', "chars.special.em_space"),
    ('\u{2002}', "chars.special.en_space"),
    ('\u{2005}', "chars.special.quarter_em_space"),
    ('\u{00A0}', "chars.special.nonbreaking_space"),
    ('\u{00A9}', "chars.special.copyright"),
    ('\u{00AE}', "chars.special.registered"),
    ('\u{2122}', "chars.special.trademark"),
    ('\u{00A7}', "chars.special.section"),
    ('\u{00B6}', "chars.special.paragraph"),
    ('\u{2026}', "chars.special.ellipsis"),
    ('\u{200B}', "chars.special.no_width_break"),
    ('\u{200D}', "chars.special.no_width_join"),
];

/// A `U+XXXX` (or bare hex) code typed into 文字コード, as a character.
pub fn character_from_code(code: &str) -> Option<char> {
    let code = code.trim();
    let hex = code
        .strip_prefix("U+")
        .or_else(|| code.strip_prefix("u+"))
        .unwrap_or(code);
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

/// 日付と時刻's formats for 日本語 (グレゴリオ暦), as Word's `\@` pictures.
pub const JAPANESE_DATE_FORMATS: [&str; 12] = [
    "yyyy/MM/dd",
    "yyyy年M月d日",
    "yyyy年M月d日 (aaa)",
    "yyyy年M月d日 aaaa",
    "yyyy/M/d",
    "yyyy-MM-dd",
    "M月d日",
    "yyyy年M月",
    "H:mm",
    "H時m分",
    "H時m分s秒",
    "yyyy/MM/dd H:mm",
];

/// The 和暦 formats.
pub const JAPANESE_ERA_FORMATS: [&str; 5] = [
    "ggge年M月d日",
    "ggge年M月d日 (aaa)",
    "gge年M月d日",
    "gee.MM.dd",
    "ggge年M月",
];

/// The 英語 (米国) formats.
pub const ENGLISH_DATE_FORMATS: [&str; 14] = [
    "M/d/yyyy",
    "dddd, MMMM d, yyyy",
    "MMMM d, yyyy",
    "M/d/yy",
    "yyyy-MM-dd",
    "d-MMM-yy",
    "M.d.yyyy",
    "d MMMM yyyy",
    "MMMM yy",
    "MMM-yy",
    "M/d/yyyy h:mm AM/PM",
    "h:mm AM/PM",
    "h:mm:ss AM/PM",
    "HH:mm",
];

/// `now` in Word's `picture`.
pub fn format_date(picture: &str, now: &FieldDateTime) -> String {
    format_word_date(picture, now)
}

/// Whether a picture shows only the time, and so is a `TIME` field.
pub fn is_time_picture(picture: &str) -> bool {
    let unquoted: String = picture.split('\'').step_by(2).collect();
    let date = ['y', 'Y', 'M', 'd', 'D', 'g', 'e', 'a']
        .iter()
        .any(|token| {
            unquoted
                .replace("AM/PM", "")
                .replace("am/pm", "")
                .contains(*token)
        });
    !date
}

/// The commands, for every session.
pub trait WordInsert: WordSession {
    /// Types `text` over the selection, or at the caret, as one undo step.
    /// Returns where the caret goes: after it.
    fn insert_at_selection(&mut self, text: &str) -> Result<Position, String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        let command = if range.is_collapsed() {
            HeadlessCommand::InsertText {
                paragraph: start.paragraph,
                offset: start.offset,
                text: text.to_string(),
            }
        } else {
            HeadlessCommand::ReplaceRange {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                text: text.to_string(),
            }
        };
        self.apply_command(command, false)?;
        Ok((start.paragraph, start.offset + text.len()))
    }

    /// 記号と特殊文字's 挿入.
    fn insert_symbol(&mut self, symbol: char) -> Result<Position, String> {
        self.insert_at_selection(&symbol.to_string())
    }

    /// 日付と時刻's OK: `now` in `picture`, as text, or with `auto_update`
    /// as a field that shows the date it is next opened on.
    fn insert_date_time(
        &mut self,
        picture: &str,
        now: &FieldDateTime,
        auto_update: bool,
    ) -> Result<Position, String> {
        let text = format_date(picture, now);
        if !auto_update {
            return self.insert_at_selection(&text);
        }
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        self.apply_command(
            HeadlessCommand::InsertDateTimeField {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                time: is_time_picture(picture),
                format: picture.to_string(),
                cached_text: text.clone(),
            },
            false,
        )?;
        Ok((start.paragraph, start.offset + text.len()))
    }
}

impl<T: WordSession + ?Sized> WordInsert for T {}

/// Whether any paragraph shows a `DATE` or `TIME` field — what a host
/// gives the clock to, and repaints, when it opens the document.
pub fn has_clock_fields(document: &crate::word_document::WordDocument) -> bool {
    use crate::document::RunFieldKind;
    (0..document.paragraph_count()).any(|index| {
        document.paragraph(index).is_some_and(|paragraph| {
            paragraph.runs.iter().any(|run| {
                run.field.as_ref().is_some_and(|field| {
                    matches!(
                        field.kind.base_kind(),
                        RunFieldKind::Date { .. } | RunFieldKind::Time { .. }
                    )
                })
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_and_codes() {
        let now = FieldDateTime {
            year: 2026,
            month: 10,
            day: 21,
            hour: 14,
            minute: 5,
            second: 9,
            weekday: 3,
        };
        assert_eq!(
            format_date("yyyy年M月d日 (aaa)", &now),
            "2026年10月21日 (水)"
        );
        assert_eq!(format_date("ggge年M月d日", &now), "令和8年10月21日");
        assert_eq!(
            format_date("dddd, MMMM d, yyyy", &now),
            "Wednesday, October 21, 2026"
        );
        assert!(is_time_picture("H時m分"));
        assert!(is_time_picture("h:mm AM/PM"));
        assert!(!is_time_picture("yyyy/MM/dd H:mm"));
        assert!(!is_time_picture("M月d日"));
        assert_eq!(character_from_code("U+2603"), Some('☃'));
        assert_eq!(character_from_code("3042"), Some('あ'));
        assert_eq!(SymbolSubset::of('あ'), Some(16));
        assert!(SYMBOL_SUBSETS[0].characters().contains(&'A'));
    }
}
