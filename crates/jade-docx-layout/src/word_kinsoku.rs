//! Word's Japanese line-breaking character sets (禁則処理), exactly as Word
//! lists them in 文字体裁 (日本語): 標準 and 高レベル for 行頭禁則文字, and the
//! one 行末禁則文字 list both levels share. The 体裁 dialog shows these same
//! strings, so what it offers is what layout applies.
//!
//! Word applies them to Japanese and Latin text alike when kinsoku is on.
//! [`jade_text_layout::kinsoku`] keeps the sets Excel and PowerPoint use.
//!
//! 分割禁止 (JIS X 4051's inseparable characters): Word does not break a
//! line between two dashes or two leaders of a pair such as `――` or `……`.

/// 行頭禁則文字, 標準.
pub const STANDARD_NO_LINE_START: &str =
    "!%),.:;?]}¢°’”‰′″℃、。々〉》」』】〕゛゜ゝゞ・ヽヾ！％），．：；？］｝｡｣､･ﾞﾟ￠";

/// What 高レベル adds to [`STANDARD_NO_LINE_START`]: the small kana and the
/// prolonged sound mark, full and half width.
pub const STRICT_ADDED_NO_LINE_START: &str =
    "ぁぃぅぇぉっゃゅょゎゕゖァィゥェォッャュョヮヵヶーｧｨｩｪｫｬｭｮｯｰ";

/// 行頭禁則文字, 高レベル, in Word's order.
pub const STRICT_NO_LINE_START: &str = "!%),.:;?]}¢°’”‰′″℃、。々〉》」』】〕ぁぃぅぇぉっゃゅょゎゕゖ゛゜ゝゞァィゥェォッャュョヮヵヶ・ーヽヾ！％），．：；？］｝｡｣､･ｧｨｩｪｫｬｭｮｯｰﾞﾟ￠";

/// 行末禁則文字, both levels.
pub const NO_LINE_END: &str = "$([\\{£¥‘“〈《「『【〔＄（［｛｢￡￥";

/// Whether `c` may not start a line at the 標準 (`strict == false`) or
/// 高レベル level.
pub fn forbids_line_start(c: char, strict: bool) -> bool {
    STANDARD_NO_LINE_START.contains(c) || strict && STRICT_ADDED_NO_LINE_START.contains(c)
}

/// Whether `c` may not end a line.
pub fn forbids_line_end(c: char) -> bool {
    NO_LINE_END.contains(c)
}

/// JIS X 4051's inseparable characters (分離禁止文字, cl-08): dashes,
/// leaders and the vertical repeat marks.
pub fn is_inseparable(c: char) -> bool {
    matches!(c, '—' | '―' | '…' | '‥' | '〳' | '〴' | '〵')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_is_standard_plus_the_small_kana() {
        let mut strict: Vec<char> = STRICT_NO_LINE_START.chars().collect();
        let mut combined: Vec<char> = STANDARD_NO_LINE_START
            .chars()
            .chain(STRICT_ADDED_NO_LINE_START.chars())
            .collect();
        strict.sort_unstable();
        combined.sort_unstable();
        assert_eq!(strict, combined);
        for c in STRICT_NO_LINE_START.chars() {
            assert!(forbids_line_start(c, true), "{c}");
        }
        assert!(!forbids_line_start('ッ', false));
        assert!(forbids_line_start('ッ', true));
        assert!(forbids_line_start('!', false) && forbids_line_start('￠', false));
        assert!(forbids_line_end('$') && forbids_line_end('「') && !forbids_line_end('」'));
        assert!(!forbids_line_start('〗', true), "not in Word's list");
    }
}
