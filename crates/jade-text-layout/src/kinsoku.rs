//! Where a line may not break.
//!
//! These are properties of the script, not of a file format: the same
//! characters may not start or end a line in a Word paragraph, an Excel cell or
//! a PowerPoint text box. They live here so there is one table rather than one
//! per format — `jade-docx-layout` reads them alongside the per-document
//! overrides Word allows, and the spreadsheet side reads them as-is, because a
//! workbook has no such setting.

/// Characters that may not start a line: the ECMA-376 Japanese default set
/// (Part 1, §17.15.1.59, the set Word calls 標準), plus the small kana and `ー`
/// of its strict set (`w:strictFirstAndLastChars`) that jade has always
/// forbidden. The ASCII members of the standard set other than `)` (`!%,.:;?]}`)
/// are left out until Word confirms whether they bind outside a Japanese
/// context (jlsi/jade#467).
///
/// Stored as `char` rather than `&str` so a per-grapheme hot path compares a
/// single `u32` instead of doing a length-checked `str` memcmp against every
/// entry. Every member is a single code point; a multi-code-point grapheme can
/// never match and callers short-circuit those.
pub const NO_LINE_BREAK_BEFORE: &[char] = &[
    '、', '。', '，', '．', '・', '：', '；', '！', '？', ')', '）', '】', '』', '」', '〉', '》',
    '］', '｝', '〕', '〗', '〙', '〛', 'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ', 'ょ', 'ゎ',
    'ァ', 'ィ', 'ゥ', 'ェ', 'ォ', 'ッ', 'ャ', 'ュ', 'ョ', 'ヮ', 'ヵ', 'ヶ', 'ー',
    // Iteration and sound marks.
    '々', 'ゝ', 'ゞ', 'ヽ', 'ヾ', '゛', '゜', // Closing quotes and unit suffixes.
    '’', '”', '‰', '′', '″', '℃', '¢', '°', '％', // Halfwidth forms.
    '｡', '｣', '､', '･', 'ﾞ', 'ﾟ',
];

/// Characters that may not end a line: the ECMA-376 Japanese default set
/// (Part 1, §17.15.1.58). Its ASCII members other than `(` (`$[\{`) are left
/// out for the same reason as in [`NO_LINE_BREAK_BEFORE`].
pub const NO_LINE_BREAK_AFTER: &[char] = &[
    '(', '（', '【', '『', '「', '〈', '《', '［', '｛', '〔',
    // Opening quotes and currency prefixes.
    '‘', '“', '£', '¥', '＄', '￡', '￥', // Halfwidth forms.
    '｢',
];

/// Whether `text` may not begin a line.
pub fn forbids_line_start(text: &str) -> bool {
    single_char(text).is_some_and(|c| NO_LINE_BREAK_BEFORE.contains(&c))
}

/// Whether `text` may not end a line.
pub fn forbids_line_end(text: &str) -> bool {
    single_char(text).is_some_and(|c| NO_LINE_BREAK_AFTER.contains(&c))
}

/// The one `char` of a single-code-point grapheme, or `None`.
fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ECMA-376 Part 1 §17.15.1.58-59, the Japanese default sets, without the
    /// ASCII members that wait on a Word trace (jlsi/jade#467).
    const STANDARD_NO_LINE_BREAK_BEFORE: &str =
        ")¢°’”‰′″℃、。々〉》」』】〕゛゜ゝゞ・ヽヾ！％），．：；？］｝｡｣､･ﾞﾟ";
    const STANDARD_NO_LINE_BREAK_AFTER: &str = "(£¥‘“〈《「『【〔＄（［｛｢￡￥";

    #[test]
    fn default_tables_cover_the_ecma_japanese_standard_sets() {
        for c in STANDARD_NO_LINE_BREAK_BEFORE.chars() {
            assert!(forbids_line_start(&c.to_string()), "{c} may start a line");
        }
        for c in STANDARD_NO_LINE_BREAK_AFTER.chars() {
            assert!(forbids_line_end(&c.to_string()), "{c} may end a line");
        }
    }

    #[test]
    fn default_tables_keep_ascii_word_punctuation_breakable() {
        for text in ["!", "%", ",", ".", ":", ";", "?", "]", "}"] {
            assert!(!forbids_line_start(text), "{text}");
        }
        for text in ["$", "[", "\\", "{"] {
            assert!(!forbids_line_end(text), "{text}");
        }
    }

    #[test]
    fn default_tables_have_no_duplicates() {
        for table in [NO_LINE_BREAK_BEFORE, NO_LINE_BREAK_AFTER] {
            let mut sorted = table.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), table.len());
        }
    }
}
