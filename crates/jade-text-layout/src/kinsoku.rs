//! Where a line may not break.
//!
//! These are properties of the script, not of a file format: the same
//! characters may not start or end a line in a Word paragraph, an Excel cell or
//! a PowerPoint text box. They live here so there is one table rather than one
//! per format — `jade-docx-layout` reads them alongside the per-document
//! overrides Word allows, and the spreadsheet side reads them as-is, because a
//! workbook has no such setting.

/// Common East Asian punctuation that may not start a line.
///
/// Stored as `char` rather than `&str` so a per-grapheme hot path compares a
/// single `u32` instead of doing a length-checked `str` memcmp against every
/// entry. Every member is a single code point; a multi-code-point grapheme can
/// never match and callers short-circuit those.
pub const NO_LINE_BREAK_BEFORE: &[char] = &[
    '、', '。', '，', '．', '・', '：', '；', '！', '？', ')', '）', '】', '』', '」', '〉', '》',
    '］', '｝', '〕', '〗', '〙', '〛', 'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ', 'ょ', 'ゎ',
    'ァ', 'ィ', 'ゥ', 'ェ', 'ォ', 'ッ', 'ャ', 'ュ', 'ョ', 'ヮ', 'ヵ', 'ヶ', 'ー',
];

/// Common East Asian punctuation that may not end a line.
pub const NO_LINE_BREAK_AFTER: &[char] =
    &['(', '（', '【', '『', '「', '〈', '《', '［', '｛', '〔'];

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
