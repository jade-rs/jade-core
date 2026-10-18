//! Japanese Excel (Jade addition): what a ja-JP Excel computes for its
//! double-byte text and its Japanese-only functions.
//!
//! - ASC / JIS turn full-width text half-width and back, katakana included
//!   (ガ ⇄ ｶﾞ).
//! - The *B functions count bytes as Shift_JIS does: one for ASCII and
//!   half-width katakana, two for everything else.
//! - DATESTRING writes a date in its era (令和06年01月15日); NUMBERSTRING
//!   writes a number in kanji (百二十三, 壱百弐拾参, 一二三).

use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, token::Error, types::CellReferenceIndex},
    formatter::dates::from_excel_date,
    model::Model,
};

/// Full-width katakana and their half-width spelling (with ﾞ/ﾟ).
const KATAKANA: &[(char, &str)] = &[
    ('ァ', "ｧ"),
    ('ア', "ｱ"),
    ('ィ', "ｨ"),
    ('イ', "ｲ"),
    ('ゥ', "ｩ"),
    ('ウ', "ｳ"),
    ('ェ', "ｪ"),
    ('エ', "ｴ"),
    ('ォ', "ｫ"),
    ('オ', "ｵ"),
    ('カ', "ｶ"),
    ('ガ', "ｶﾞ"),
    ('キ', "ｷ"),
    ('ギ', "ｷﾞ"),
    ('ク', "ｸ"),
    ('グ', "ｸﾞ"),
    ('ケ', "ｹ"),
    ('ゲ', "ｹﾞ"),
    ('コ', "ｺ"),
    ('ゴ', "ｺﾞ"),
    ('サ', "ｻ"),
    ('ザ', "ｻﾞ"),
    ('シ', "ｼ"),
    ('ジ', "ｼﾞ"),
    ('ス', "ｽ"),
    ('ズ', "ｽﾞ"),
    ('セ', "ｾ"),
    ('ゼ', "ｾﾞ"),
    ('ソ', "ｿ"),
    ('ゾ', "ｿﾞ"),
    ('タ', "ﾀ"),
    ('ダ', "ﾀﾞ"),
    ('チ', "ﾁ"),
    ('ヂ', "ﾁﾞ"),
    ('ッ', "ｯ"),
    ('ツ', "ﾂ"),
    ('ヅ', "ﾂﾞ"),
    ('テ', "ﾃ"),
    ('デ', "ﾃﾞ"),
    ('ト', "ﾄ"),
    ('ド', "ﾄﾞ"),
    ('ナ', "ﾅ"),
    ('ニ', "ﾆ"),
    ('ヌ', "ﾇ"),
    ('ネ', "ﾈ"),
    ('ノ', "ﾉ"),
    ('ハ', "ﾊ"),
    ('バ', "ﾊﾞ"),
    ('パ', "ﾊﾟ"),
    ('ヒ', "ﾋ"),
    ('ビ', "ﾋﾞ"),
    ('ピ', "ﾋﾟ"),
    ('フ', "ﾌ"),
    ('ブ', "ﾌﾞ"),
    ('プ', "ﾌﾟ"),
    ('ヘ', "ﾍ"),
    ('ベ', "ﾍﾞ"),
    ('ペ', "ﾍﾟ"),
    ('ホ', "ﾎ"),
    ('ボ', "ﾎﾞ"),
    ('ポ', "ﾎﾟ"),
    ('マ', "ﾏ"),
    ('ミ', "ﾐ"),
    ('ム', "ﾑ"),
    ('メ', "ﾒ"),
    ('モ', "ﾓ"),
    ('ャ', "ｬ"),
    ('ヤ', "ﾔ"),
    ('ュ', "ｭ"),
    ('ユ', "ﾕ"),
    ('ョ', "ｮ"),
    ('ヨ', "ﾖ"),
    ('ラ', "ﾗ"),
    ('リ', "ﾘ"),
    ('ル', "ﾙ"),
    ('レ', "ﾚ"),
    ('ロ', "ﾛ"),
    ('ワ', "ﾜ"),
    ('ヲ', "ｦ"),
    ('ン', "ﾝ"),
    ('ヴ', "ｳﾞ"),
    ('ー', "ｰ"),
    ('。', "｡"),
    ('「', "｢"),
    ('」', "｣"),
    ('、', "､"),
    ('・', "･"),
    ('゛', "ﾞ"),
    ('゜', "ﾟ"),
];

/// ASC: full-width letters, digits, symbols, space and katakana half-width.
pub(crate) fn to_halfwidth(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let code = c as u32;
        if code == 0x3000 {
            out.push(' ');
        } else if (0xFF01..=0xFF5E).contains(&code) {
            out.push(char::from_u32(code - 0xFF01 + 0x21).unwrap_or(c));
        } else if let Some((_, half)) = KATAKANA.iter().find(|(full, _)| *full == c) {
            out.push_str(half);
        } else {
            out.push(c);
        }
    }
    out
}

/// JIS: the reverse, a half-width kana and its sound mark becoming one
/// full-width kana.
pub(crate) fn to_fullwidth(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 3);
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let code = c as u32;
        // A kana with its voiced or semi-voiced mark first, then alone.
        if let Some(&next) = chars.get(index + 1) {
            if matches!(next, 'ﾞ' | 'ﾟ') {
                let pair: String = [c, next].iter().collect();
                if let Some((full, _)) = KATAKANA.iter().find(|(_, half)| *half == pair) {
                    out.push(*full);
                    index += 2;
                    continue;
                }
            }
        }
        if c == ' ' {
            out.push('\u{3000}');
        } else if (0x21..=0x7E).contains(&code) {
            out.push(char::from_u32(code - 0x21 + 0xFF01).unwrap_or(c));
        } else if let Some((full, _)) = KATAKANA
            .iter()
            .find(|(_, half)| half.chars().count() == 1 && half.starts_with(c))
        {
            out.push(*full);
        } else {
            out.push(c);
        }
        index += 1;
    }
    out
}

/// A character's width in Shift_JIS bytes.
fn byte_width(c: char) -> usize {
    let code = c as u32;
    if code <= 0x7F || (0xFF61..=0xFF9F).contains(&code) {
        1
    } else {
        2
    }
}

pub(crate) fn byte_length(text: &str) -> usize {
    text.chars().map(byte_width).sum()
}

/// The characters whose bytes fall wholly inside `first..first + count`
/// (bytes counted from 1).
fn byte_slice(text: &str, first: usize, count: usize) -> String {
    let last = first.saturating_add(count);
    let mut position = 1;
    let mut out = String::new();
    for c in text.chars() {
        let end = position + byte_width(c);
        if position >= first && end <= last {
            out.push(c);
        }
        position = end;
    }
    out
}

/// The byte position (from 1) of the character at `chars` (from 1).
fn byte_position(text: &str, chars: usize) -> usize {
    1 + text
        .chars()
        .take(chars.saturating_sub(1))
        .map(byte_width)
        .sum::<usize>()
}

/// The eras a date serial can reach: first day, name.
const ERAS: [((i32, u32, u32), &str); 5] = [
    ((1868, 1, 1), "明治"),
    ((1912, 7, 30), "大正"),
    ((1926, 12, 25), "昭和"),
    ((1989, 1, 8), "平成"),
    ((2019, 5, 1), "令和"),
];

fn datestring(serial: i64) -> Option<String> {
    use chrono::Datelike;
    let date = from_excel_date(serial).ok()?;
    let key = (date.year(), date.month(), date.day());
    let (start, name) = ERAS.iter().rev().find(|(start, _)| key >= *start)?;
    Some(format!(
        "{name}{:02}年{:02}月{:02}日",
        date.year() - start.0 + 1,
        date.month(),
        date.day()
    ))
}

/// NUMBERSTRING's three styles: kanji with units (1), the formal numerals
/// of contracts and receipts (2), and digit by digit (3).
fn numberstring(value: u64, style: u8) -> String {
    const DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    const FORMAL: [&str; 10] = ["〇", "壱", "弐", "参", "四", "伍", "六", "七", "八", "九"];
    if style == 3 {
        return value
            .to_string()
            .chars()
            .map(|d| DIGITS[d.to_digit(10).unwrap_or(0) as usize])
            .collect();
    }
    if value == 0 {
        return "〇".to_string();
    }
    let (digits, small, large): (&[&str; 10], [&str; 3], [&str; 4]) = if style == 2 {
        (&FORMAL, ["拾", "百", "阡"], ["", "萬", "億", "兆"])
    } else {
        (&DIGITS, ["十", "百", "千"], ["", "万", "億", "兆"])
    };
    let mut out = String::new();
    let mut groups = Vec::new();
    let mut rest = value;
    while rest > 0 {
        groups.push((rest % 10_000) as usize);
        rest /= 10_000;
    }
    for (power, group) in groups.iter().enumerate().rev() {
        if *group == 0 {
            continue;
        }
        let places = [group / 1000, group / 100 % 10, group / 10 % 10, group % 10];
        for (place, digit) in places.iter().enumerate() {
            if *digit == 0 {
                continue;
            }
            let unit = 3 - place;
            // Plain style drops the 一 before 十, 百 and 千.
            if !(style != 2 && *digit == 1 && unit > 0) {
                out.push_str(digits[*digit]);
            }
            if unit > 0 {
                out.push_str(small[unit - 1]);
            }
        }
        out.push_str(large.get(power).copied().unwrap_or(""));
    }
    out
}

impl<'a> Model<'a> {
    pub(crate) fn fn_jis(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.get_string(&args[0], cell) {
            Ok(text) => CalcResult::String(to_fullwidth(&text)),
            Err(error) => error,
        }
    }

    pub(crate) fn fn_datestring(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial = match self.get_number(&args[0], cell) {
            Ok(value) => value.floor() as i64,
            Err(error) => return error,
        };
        match datestring(serial) {
            Some(text) => CalcResult::String(text),
            None => CalcResult::new_error(Error::VALUE, cell, "Not a date".to_string()),
        }
    }

    pub(crate) fn fn_numberstring(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let style = match self.get_number(&args[1], cell) {
            Ok(style) => style.trunc(),
            Err(error) => return error,
        };
        if !(1.0..=3.0).contains(&style) || !(0.0..1e16).contains(&value) {
            return CalcResult::new_error(Error::NUM, cell, "Out of range".to_string());
        }
        CalcResult::String(numberstring(value.round() as u64, style as u8))
    }

    /// LENB: Shift_JIS bytes.
    pub(crate) fn dbcs_lenb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.get_string(&args[0], cell) {
            Ok(text) => CalcResult::Number(byte_length(&text) as f64),
            Err(error) => error,
        }
    }

    /// LEFTB, RIGHTB, MIDB: the characters wholly inside a byte range.
    pub(crate) fn dbcs_slice(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        which: char,
    ) -> CalcResult {
        let (text_arg, rest) = match args.split_first() {
            Some(split) => split,
            None => return CalcResult::new_args_number_error(cell),
        };
        let text = match self.get_string(text_arg, cell) {
            Ok(text) => text,
            Err(error) => return error,
        };
        let mut numbers = Vec::new();
        for arg in rest {
            match self.get_number(arg, cell) {
                Ok(value) if value >= 0.0 => numbers.push(value.trunc() as usize),
                Ok(_) => return CalcResult::new_error(Error::VALUE, cell, "Negative count".into()),
                Err(error) => return error,
            }
        }
        let total = byte_length(&text);
        let slice = match (which, numbers.as_slice()) {
            ('L', []) => byte_slice(&text, 1, 1),
            ('L', [count]) => byte_slice(&text, 1, *count),
            ('R', []) => byte_slice(&text, total, 1),
            ('R', [count]) => byte_slice(&text, total.saturating_sub(*count) + 1, *count),
            ('M', [start, count]) if *start >= 1 => byte_slice(&text, *start, *count),
            ('M', [_, _]) => {
                return CalcResult::new_error(Error::VALUE, cell, "Start below 1".into())
            }
            _ => return CalcResult::new_args_number_error(cell),
        };
        CalcResult::String(slice)
    }

    /// REPLACEB: `new_text` in place of the characters whose bytes start
    /// inside `start..start + count` (bytes counted from 1).
    pub(crate) fn dbcs_replace(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 4 {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.get_string(&args[0], cell) {
            Ok(text) => text,
            Err(error) => return error,
        };
        let start = match self.get_number(&args[1], cell) {
            Ok(value) if value >= 1.0 => value.trunc() as usize,
            Ok(_) => return CalcResult::new_error(Error::VALUE, cell, "Start below 1".into()),
            Err(error) => return error,
        };
        let count = match self.get_number(&args[2], cell) {
            Ok(value) if value >= 0.0 => value.trunc() as usize,
            Ok(_) => return CalcResult::new_error(Error::VALUE, cell, "Negative count".into()),
            Err(error) => return error,
        };
        let new_text = match self.get_string(&args[3], cell) {
            Ok(text) => text,
            Err(error) => return error,
        };
        let end = start.saturating_add(count);
        let mut out = String::new();
        let mut position = 1;
        let mut inserted = false;
        for c in text.chars() {
            if position >= start && position < end {
                if !inserted {
                    out.push_str(&new_text);
                    inserted = true;
                }
            } else {
                if position >= end && !inserted {
                    out.push_str(&new_text);
                    inserted = true;
                }
                out.push(c);
            }
            position += byte_width(c);
        }
        if !inserted {
            out.push_str(&new_text);
        }
        CalcResult::String(out)
    }

    /// FINDB, SEARCHB: the character position FIND or SEARCH gives, in
    /// bytes.
    pub(crate) fn dbcs_position(
        &mut self,
        found: CalcResult,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        let CalcResult::Number(position) = found else {
            return found;
        };
        match args.get(1).map(|arg| self.get_string(arg, cell)) {
            Some(Ok(text)) => CalcResult::Number(byte_position(&text, position as usize) as f64),
            Some(Err(error)) => error,
            None => CalcResult::new_args_number_error(cell),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_round_trip() {
        assert_eq!(to_halfwidth("ＡＢＣ　ガパー"), "ABC ｶﾞﾊﾟｰ");
        assert_eq!(to_fullwidth("ABC ｶﾞﾊﾟｰ"), "ＡＢＣ　ガパー");
        assert_eq!(byte_length("aあｱ"), 4);
        assert_eq!(byte_slice("あいう", 1, 4), "あい");
        assert_eq!(byte_position("aあいう", 3), 4);
    }

    #[test]
    fn numbers_in_kanji() {
        assert_eq!(numberstring(123456, 1), "十二万三千四百五十六");
        assert_eq!(numberstring(123456, 2), "壱拾弐萬参阡四百伍拾六");
        assert_eq!(numberstring(100_000_000, 1), "一億");
        assert_eq!(numberstring(1_010, 1), "千十");
        assert_eq!(numberstring(0, 1), "〇");
    }
}
