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

// Define each ordered public list and its membership predicate together so
// the hot path needs neither initialization nor a runtime set search.
macro_rules! character_set {
    ($(#[$meta:meta])* $name:ident, $contains:ident, [$($character:literal),+ $(,)?]) => {
        $(#[$meta])*
        pub const $name: &str = concat!($($character),+);

        fn $contains(c: char) -> bool {
            matches!(c, $($character)|+)
        }
    };
}

character_set! {
    /// 行頭禁則文字, 標準.
    STANDARD_NO_LINE_START, is_standard_no_line_start, [
        '!', '%', ')', ',', '.', ':', ';', '?',
        ']', '}', '¢', '°', '’', '”', '‰', '′',
        '″', '℃', '、', '。', '々', '〉', '》', '」',
        '』', '】', '〕', '゛', '゜', 'ゝ', 'ゞ', '・',
        'ヽ', 'ヾ', '！', '％', '）', '，', '．', '：',
        '；', '？', '］', '｝', '｡', '｣', '､', '･',
        'ﾞ', 'ﾟ', '￠',
    ]
}

character_set! {
    /// What 高レベル adds to [`STANDARD_NO_LINE_START`]: the small kana and the
    /// prolonged sound mark, full and half width.
    STRICT_ADDED_NO_LINE_START, is_strict_added_no_line_start, [
        'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ',
        'ょ', 'ゎ', 'ゕ', 'ゖ', 'ァ', 'ィ', 'ゥ', 'ェ',
        'ォ', 'ッ', 'ャ', 'ュ', 'ョ', 'ヮ', 'ヵ', 'ヶ',
        'ー', 'ｧ', 'ｨ', 'ｩ', 'ｪ', 'ｫ', 'ｬ', 'ｭ',
        'ｮ', 'ｯ', 'ｰ',
    ]
}

/// 行頭禁則文字, 高レベル, in Word's order.
pub const STRICT_NO_LINE_START: &str = "!%),.:;?]}¢°’”‰′″℃、。々〉》」』】〕ぁぃぅぇぉっゃゅょゎゕゖ゛゜ゝゞァィゥェォッャュョヮヵヶ・ーヽヾ！％），．：；？］｝｡｣､･ｧｨｩｪｫｬｭｮｯｰﾞﾟ￠";

character_set! {
    /// 行末禁則文字, both levels.
    NO_LINE_END, is_no_line_end, [
        '$', '(', '[', '\\', '{', '£', '¥', '‘',
        '“', '〈', '《', '「', '『', '【', '〔', '＄',
        '（', '［', '｛', '｢', '￡', '￥',
    ]
}

/// Whether `c` may not start a line at the 標準 (`strict == false`) or
/// 高レベル level.
pub fn forbids_line_start(c: char, strict: bool) -> bool {
    is_standard_no_line_start(c) || strict && is_strict_added_no_line_start(c)
}

/// Whether `c` may not end a line.
pub fn forbids_line_end(c: char) -> bool {
    is_no_line_end(c)
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
    fn fixed_sets_match_word_lists_for_every_unicode_scalar() {
        use std::collections::HashSet;
        const WORD_STANDARD_NO_LINE_START: &str =
            "!%),.:;?]}¢°’”‰′″℃、。々〉》」』】〕゛゜ゝゞ・ヽヾ！％），．：；？］｝｡｣､･ﾞﾟ￠";
        assert_eq!(STANDARD_NO_LINE_START, WORD_STANDARD_NO_LINE_START);
        const WORD_STRICT_ADDED_NO_LINE_START: &str =
            "ぁぃぅぇぉっゃゅょゎゕゖァィゥェォッャュョヮヵヶーｧｨｩｪｫｬｭｮｯｰ";
        assert_eq!(STRICT_ADDED_NO_LINE_START, WORD_STRICT_ADDED_NO_LINE_START);
        const WORD_STRICT_NO_LINE_START: &str = "!%),.:;?]}¢°’”‰′″℃、。々〉》」』】〕ぁぃぅぇぉっゃゅょゎゕゖ゛゜ゝゞァィゥェォッャュョヮヵヶ・ーヽヾ！％），．：；？］｝｡｣､･ｧｨｩｪｫｬｭｮｯｰﾞﾟ￠";
        assert_eq!(STRICT_NO_LINE_START, WORD_STRICT_NO_LINE_START);
        const WORD_NO_LINE_END: &str = "$([\\{£¥‘“〈《「『【〔＄（［｛｢￡￥";
        assert_eq!(NO_LINE_END, WORD_NO_LINE_END);
        let standard: HashSet<_> = WORD_STANDARD_NO_LINE_START.chars().collect();
        let strict: HashSet<_> = WORD_STRICT_NO_LINE_START.chars().collect();
        let end: HashSet<_> = WORD_NO_LINE_END.chars().collect();
        for c in (0..=0x10ffff).filter_map(char::from_u32) {
            assert_eq!(
                forbids_line_start(c, false),
                standard.contains(&c),
                "U+{:04X}",
                c as u32
            );
            assert_eq!(
                forbids_line_start(c, true),
                strict.contains(&c),
                "U+{:04X}",
                c as u32
            );
            assert_eq!(forbids_line_end(c), end.contains(&c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn public_kinsoku_rules_preserve_custom_and_multi_scalar_entries() {
        use crate::line_wrap::{EastAsianLineBreakingRules, ParagraphStyle};
        let entries = ["", "!!", "!\u{0301}", "。\u{fe0f}", "👩\u{200d}🚀"];
        for strict in [false, true] {
            let mut style = ParagraphStyle {
                enable_line_break_rules: true,
                east_asian_rules: EastAsianLineBreakingRules {
                    strict_first_and_last_chars: strict,
                    ..EastAsianLineBreakingRules::default()
                },
                ..ParagraphStyle::default()
            };
            for entry in entries {
                assert!(!style.forbids_line_start(entry), "{entry:?}");
                assert!(!style.forbids_line_end(entry), "{entry:?}");
            }
            style.east_asian_rules.no_line_break_before = entries.map(str::to_owned).to_vec();
            style.east_asian_rules.no_line_break_after = entries.map(str::to_owned).to_vec();
            for replaces in [false, true] {
                style.east_asian_rules.no_line_break_before_replaces_default = replaces;
                style.east_asian_rules.no_line_break_after_replaces_default = replaces;
                for entry in entries {
                    assert!(style.forbids_line_start(entry), "{entry:?}");
                    assert!(style.forbids_line_end(entry), "{entry:?}");
                }
                assert_eq!(style.forbids_line_start("!"), !replaces);
                assert_eq!(style.forbids_line_end("「"), !replaces);
                assert_eq!(style.forbids_line_start("ッ"), strict && !replaces);
            }
            style.east_asian_rules.kinsoku = Some(false);
            for entry in entries {
                assert!(!style.forbids_line_start(entry));
                assert!(!style.forbids_line_end(entry));
            }
            style.east_asian_rules.kinsoku = None;
            style.enable_line_break_rules = false;
            for entry in entries {
                assert!(!style.forbids_line_start(entry));
                assert!(!style.forbids_line_end(entry));
            }
        }
    }

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
