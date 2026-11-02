//! How a character stands in a column of vertical text (縦書き).
//!
//! Word draws upright Japanese with the font's vertical alternates (the
//! OpenType `vert` feature): 、 and 。 move to the top right of their cell,
//! brackets and parentheses turn to open downwards, 「ー」 and 「〜」 run
//! down the column, and small kana sit nearer the top right. A painter that
//! shapes text asks the font for those glyphs; one that cannot falls back to
//! [`vertical_form`]: the Unicode vertical presentation form when there is
//! one (ECMA-376 leaves the glyphs to the font; Unicode's UAX #50 names which
//! characters change), else the character turned a quarter, or nudged
//! towards the top right as the font's alternates are.

/// What happens to one character in an upright column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VerticalForm {
    /// Drawn as it is.
    Upright,
    /// Drawn as this vertical presentation form (U+FE10–U+FE4F).
    Substitute(char),
    /// Turned a quarter clockwise about the centre of its cell.
    Rotate,
    /// Moved right and up by these fractions of the font size.
    Shift { right: f64, up: f64 },
}

/// The font feature that gives upright text its vertical alternates.
pub const VERT_FEATURE: &str = "vert";

/// [`VerticalForm`] of `grapheme` in an upright column. Only a single
/// character changes; clusters and Latin runs stand as they are.
pub fn vertical_form(grapheme: &str) -> VerticalForm {
    let mut chars = grapheme.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return VerticalForm::Upright;
    };
    if let Some(form) = presentation_form(ch) {
        return VerticalForm::Substitute(form);
    }
    if turns_in_vertical(ch) {
        return VerticalForm::Rotate;
    }
    if is_small_kana(ch) {
        // The alternates of Yu Mincho, MS Mincho and Noto Sans CJK move small
        // kana about an eighth of the em towards the top right.
        return VerticalForm::Shift {
            right: 0.125,
            up: 0.125,
        };
    }
    match ch {
        // Full-width full stop has no presentation form: its alternate sits
        // in the top-right quarter, where 。's does.
        '．' => VerticalForm::Shift {
            right: 0.6,
            up: 0.6,
        },
        _ => VerticalForm::Upright,
    }
}

/// Whether `grapheme` draws differently in an upright column.
pub fn changes_in_vertical(grapheme: &str) -> bool {
    vertical_form(grapheme) != VerticalForm::Upright
}

/// The vertical presentation form Unicode gives `ch`, if any.
fn presentation_form(ch: char) -> Option<char> {
    Some(match ch {
        '，' => '\u{FE10}',
        '、' => '\u{FE11}',
        '。' => '\u{FE12}',
        '：' => '\u{FE13}',
        '；' => '\u{FE14}',
        '！' => '\u{FE15}',
        '？' => '\u{FE16}',
        '〖' => '\u{FE17}',
        '〗' => '\u{FE18}',
        '…' => '\u{FE19}',
        '‥' => '\u{FE30}',
        '—' => '\u{FE31}',
        '–' => '\u{FE32}',
        '＿' => '\u{FE33}',
        '（' => '\u{FE35}',
        '）' => '\u{FE36}',
        '｛' => '\u{FE37}',
        '｝' => '\u{FE38}',
        '〔' => '\u{FE39}',
        '〕' => '\u{FE3A}',
        '【' => '\u{FE3B}',
        '】' => '\u{FE3C}',
        '《' => '\u{FE3D}',
        '》' => '\u{FE3E}',
        '〈' => '\u{FE3F}',
        '〉' => '\u{FE40}',
        '「' => '\u{FE41}',
        '」' => '\u{FE42}',
        '『' => '\u{FE43}',
        '』' => '\u{FE44}',
        '［' => '\u{FE47}',
        '］' => '\u{FE48}',
        _ => return None,
    })
}

/// Characters whose vertical alternate is the glyph turned a quarter: the
/// long-vowel mark, dashes, waves, full-width signs and arrows that run
/// along the line (UAX #50's `Tr`, and the `Tu` ones Japanese fonts turn).
fn turns_in_vertical(ch: char) -> bool {
    matches!(
        ch,
        'ー' | 'ｰ'
            | '〜'
            | '～'
            | '―'
            | '‐'
            | '－'
            | '＝'
            | '＜'
            | '＞'
            | '≒'
            | '≠'
            | '〰'
            | '─'
            | '━'
            | '→'
            | '←'
            | '↑'
            | '↓'
            | '⇒'
            | '⇔'
            | '〓'
            | '｜'
    )
}

/// Small hiragana and katakana: ぁ ぃ … ゎ ゕ ゖ, ァ ィ … ヮ ヵ ヶ, ㇰ–ㇿ.
fn is_small_kana(ch: char) -> bool {
    matches!(
        ch,
        'ぁ' | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ゕ'
            | 'ゖ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
            | '\u{31F0}'..='\u{31FF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_takes_its_presentation_form() {
        assert_eq!(vertical_form("、"), VerticalForm::Substitute('︑'));
        assert_eq!(vertical_form("。"), VerticalForm::Substitute('︒'));
        assert_eq!(vertical_form("「"), VerticalForm::Substitute('﹁'));
        assert_eq!(vertical_form("』"), VerticalForm::Substitute('﹄'));
        assert_eq!(vertical_form("（"), VerticalForm::Substitute('︵'));
        assert_eq!(vertical_form("…"), VerticalForm::Substitute('︙'));
        assert_eq!(vertical_form("【"), VerticalForm::Substitute('︻'));
    }

    #[test]
    fn long_vowel_and_waves_turn() {
        for ch in ["ー", "〜", "～", "―", "→", "＝"] {
            assert_eq!(vertical_form(ch), VerticalForm::Rotate, "{ch}");
        }
    }

    #[test]
    fn small_kana_move_towards_the_top_right() {
        for ch in ["ゃ", "ッ", "ぁ", "ヶ", "ㇷ"] {
            assert!(
                matches!(vertical_form(ch), VerticalForm::Shift { right, up } if right > 0.0 && up > 0.0),
                "{ch}"
            );
        }
    }

    #[test]
    fn ideographs_kana_and_clusters_stand_upright() {
        for text in ["漢", "あ", "カ", "Ａ", "1", "ab", "", "か\u{3099}"] {
            assert_eq!(vertical_form(text), VerticalForm::Upright, "{text:?}");
            assert!(!changes_in_vertical(text));
        }
    }
}
