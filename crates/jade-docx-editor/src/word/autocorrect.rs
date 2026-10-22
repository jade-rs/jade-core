//! オートコレクト and 入力オートフォーマット: what Word changes as it is
//! typed.
//!
//! - オートコレクト: the replacement list ((c) → ©, teh → the), 2 文字目を
//!   小文字にする (TWo → Two), 文の先頭文字を大文字にする, 曜日の先頭文字を
//!   大文字にする.
//! - 入力オートフォーマット: "" → “”, 序数 (1st with "st" raised), 分数
//!   (1/2 → ½), ハイフンをダッシュに (-- → —, " - " → " – "), 箇条書きと
//!   段落番号 ("* ", "1. ", "・" at a paragraph's start), 罫線 (---, ===,
//!   ___ and Enter), 見出し, and the Japanese ones: 長音とダッシュを正しく使い
//!   分ける, かっこを正しく組み合わせる, 頭語に対応する結語を挿入する (拝啓 →
//!   敬具), 記書きに対応する"以上"を挿入する.
//!
//! The decisions are pure functions over the paragraph's text and the
//! caret; [`WordAutoCorrect`] makes them edits on any session, each one undo
//! step after the typing that set it off — so ⌘Z takes the correction back
//! and leaves what was typed, as in Word.

use jade_docx_headless_api::{HeadlessBorderEdge, HeadlessCommand};

use super::lists::WordLists;
use super::{Position, WordSession};

/// Word's AutoCorrect settings.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoCorrectOptions {
    /// 2 文字目を小文字にする.
    pub two_initial_capitals: bool,
    /// 文の先頭文字を大文字にする.
    pub capitalize_sentences: bool,
    /// 曜日の先頭文字を大文字にする.
    pub capitalize_days: bool,
    /// 入力中に自動修正する: the replacement list.
    pub replace_text: bool,
    pub entries: Vec<(String, String)>,
    /// 左右の区別がない引用符を、区別がある引用符に変更する.
    pub smart_quotes: bool,
    /// 序数 (1st, 2nd) を上付き文字に変更する.
    pub ordinals: bool,
    /// 分数 (1/2, 1/4, 3/4) を分数文字に変更する.
    pub fractions: bool,
    /// ハイフンをダッシュに変更する.
    pub hyphens: bool,
    /// 箇条書き (行頭文字).
    pub bulleted_lists: bool,
    /// 段落番号.
    pub numbered_lists: bool,
    /// 罫線.
    pub borders: bool,
    /// 組み込みの見出しスタイル (off, as in Word).
    pub headings: bool,
    /// 長音とダッシュを正しく使い分ける.
    pub long_vowels: bool,
    /// かっこを正しく組み合わせる.
    pub brackets: bool,
    /// 頭語に対応する結語を挿入する.
    pub closing_phrases: bool,
    /// 記書きに対応する"以上"を挿入する.
    pub kiki: bool,
    /// 日本語と英数字の間の不要なスペースを削除する.
    pub cjk_spaces: bool,
}

impl Default for AutoCorrectOptions {
    fn default() -> Self {
        AutoCorrectOptions {
            two_initial_capitals: true,
            capitalize_sentences: true,
            capitalize_days: true,
            replace_text: true,
            entries: default_entries(),
            smart_quotes: true,
            ordinals: true,
            fractions: true,
            hyphens: true,
            bulleted_lists: true,
            numbered_lists: true,
            borders: true,
            headings: false,
            long_vowels: true,
            brackets: true,
            closing_phrases: true,
            kiki: true,
            cjk_spaces: true,
        }
    }
}

/// The replacement list Word starts with (a part of it).
pub fn default_entries() -> Vec<(String, String)> {
    [
        ("(c)", "©"),
        ("(r)", "®"),
        ("(tm)", "™"),
        ("(e)", "€"),
        ("...", "…"),
        (":)", "☺"),
        (":(", "☹"),
        ("-->", "→"),
        ("<--", "←"),
        ("==>", "⇒"),
        ("<==", "⇐"),
        ("<=>", "⇔"),
        ("abbout", "about"),
        ("accomodate", "accommodate"),
        ("acheive", "achieve"),
        ("adn", "and"),
        ("alot", "a lot"),
        ("beleive", "believe"),
        ("cant", "can't"),
        ("definately", "definitely"),
        ("didnt", "didn't"),
        ("doesnt", "doesn't"),
        ("dont", "don't"),
        ("hte", "the"),
        ("i", "I"),
        ("isnt", "isn't"),
        ("occured", "occurred"),
        ("recieve", "receive"),
        ("seperate", "separate"),
        ("teh", "the"),
        ("thier", "their"),
        ("wasnt", "wasn't"),
        ("wierd", "weird"),
    ]
    .into_iter()
    .map(|(from, to)| (from.to_string(), to.to_string()))
    .collect()
}

/// One change to the paragraph: a byte range of its text and what goes
/// there, and a part of the new text to raise (序数's "st").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// Bytes of `text` to make superscript.
    pub superscript: Option<(usize, usize)>,
}

/// What a paragraph's start turns into: a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListStart {
    /// How many bytes of the paragraph the marker took ("1. ").
    pub marker: usize,
    /// The list's number format (`decimal`, `bullet`…) and level text.
    pub format: String,
    pub text: String,
    /// The bullet's font, when it needs one.
    pub font: Option<String>,
    /// The number typed, which the list starts at.
    pub start: u32,
}

/// What Enter makes of the paragraph it ends.
#[derive(Clone, Debug, PartialEq)]
pub enum EnterFormat {
    /// The paragraph's dashes become a border under it.
    Border(HeadlessBorderEdge),
    /// 頭語: the 結語 goes in, right-aligned, a paragraph below.
    Closing(&'static str),
    /// 記: centred, and 以上 right-aligned a paragraph below.
    Kiki,
}

/// The characters that end a word and set off a correction.
pub fn is_trigger(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '.' | ',' | ';' | ':' | '!' | '?' | ')' | '\u{3000}' | '。' | '、' | '\n'
    )
}

const DAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// The corrections a character just typed sets off. `text` is the
/// paragraph as it now is and `caret` the byte offset after the typed
/// character. The fixes are in the order they are to be made, last in the
/// text first, so each leaves the offsets of the next as they were.
pub fn corrections(text: &str, caret: usize, options: &AutoCorrectOptions) -> Vec<Fix> {
    let caret = caret.min(text.len());
    let Some(typed) = text[..caret].chars().next_back() else {
        return Vec::new();
    };
    let typed_at = caret - typed.len_utf8();
    let mut fixes = Vec::new();

    // Characters corrected as they are typed, not at a word's end.
    if options.smart_quotes && matches!(typed, '"' | '\'') {
        let before = text[..typed_at].chars().next_back();
        let opening = before.is_none_or(|ch| {
            ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '“' | '‘' | '「' | '（')
        });
        let quote = match (typed, opening) {
            ('"', true) => "“",
            ('"', false) => "”",
            (_, true) => "‘",
            _ => "’",
        };
        return vec![fix(typed_at, caret, quote)];
    }
    if options.long_vowels {
        let before = text[..typed_at].chars().next_back();
        let kana = before.is_some_and(|ch| {
            matches!(
                super::selection::char_class(ch),
                super::selection::CharClass::Katakana | super::selection::CharClass::Hiragana
            ) || ch == 'ー'
        });
        if typed == 'ー' && before.is_some() && !kana {
            return vec![fix(typed_at, caret, "―")];
        }
        if matches!(typed, '―' | '－' | '—') && kana {
            return vec![fix(typed_at, caret, "ー")];
        }
    }
    if options.brackets {
        if let Some(fixed) = matching_bracket(&text[..typed_at], typed) {
            if fixed != typed {
                return vec![fix(typed_at, caret, &fixed.to_string())];
            }
        }
    }
    // A replacement whose last character is the one typed: (c), :), -->.
    if options.replace_text {
        for (from, to) in &options.entries {
            if from.ends_with(typed)
                && from.chars().count() > 1
                && text[..caret].ends_with(from.as_str())
            {
                let start = caret - from.len();
                let edge = text[..start].chars().next_back();
                let symbol_led = from.chars().next().is_some_and(|ch| !ch.is_alphanumeric());
                if (symbol_led || edge.is_none_or(|ch| ch.is_whitespace()))
                    && !from.chars().all(char::is_alphanumeric)
                {
                    return vec![fix(start, caret, to)];
                }
            }
        }
    }
    // 日本語と英数字の間の不要なスペース: "日本 Word" as the W goes in, or
    // "Word 日本" as the 日 does.
    if options.cjk_spaces && !typed.is_whitespace() {
        let mut back = text[..typed_at].chars().rev();
        if let (Some(' '), Some(before)) = (back.next(), back.next()) {
            let japanese = |ch: char| {
                matches!(
                    super::selection::char_class(ch),
                    super::selection::CharClass::Kanji
                        | super::selection::CharClass::Hiragana
                        | super::selection::CharClass::Katakana
                )
            };
            let latin = |ch: char| ch.is_ascii_alphanumeric();
            if (japanese(before) && latin(typed)) || (latin(before) && japanese(typed)) {
                return vec![fix(typed_at - 1, typed_at, "")];
            }
        }
    }
    if !is_trigger(typed) {
        return Vec::new();
    }

    // The word the trigger ended.
    // A Japanese character ends a word as a space does: 情報jdo's word is
    // jdo.
    let japanese = |ch: char| {
        matches!(
            super::selection::char_class(ch),
            super::selection::CharClass::Kanji
                | super::selection::CharClass::Hiragana
                | super::selection::CharClass::Katakana
        ) || ('\u{3000}'..='\u{303F}').contains(&ch)
    };
    let word_start = text[..typed_at]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| !ch.is_whitespace() && !japanese(*ch))
        .last()
        .map_or(typed_at, |(at, _)| at);
    let mut word = &text[word_start..typed_at];
    // Punctuation that ends a sentence is not part of the word.
    let mut word_end = typed_at;
    while let Some(last) = word.chars().next_back() {
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '"' | '”' | '’') && word.len() > 1
        {
            word_end -= last.len_utf8();
            word = &text[word_start..word_end];
        } else {
            break;
        }
    }
    if word.is_empty() {
        return Vec::new();
    }

    if options.replace_text {
        if let Some((_, to)) = options.entries.iter().find(|(from, _)| from == word) {
            let mut replaced = to.clone();
            if word_start == sentence_start(text, word_start) && options.capitalize_sentences {
                replaced = capitalize(&replaced);
            }
            return vec![fix(word_start, word_end, &replaced)];
        }
    }
    if options.fractions {
        let fraction = match word {
            "1/2" => Some("½"),
            "1/4" => Some("¼"),
            "3/4" => Some("¾"),
            _ => None,
        };
        if let Some(fraction) = fraction {
            return vec![fix(word_start, word_end, fraction)];
        }
    }
    if options.ordinals {
        if let Some(suffix_at) = ordinal_suffix(word) {
            return vec![Fix {
                start: word_start,
                end: word_end,
                text: word.to_string(),
                superscript: Some((suffix_at, word.len())),
            }];
        }
    }
    if options.hyphens {
        // word--word: an em dash.
        if let Some(at) = word.find("--") {
            let (left, right) = (&word[..at], &word[at + 2..]);
            if left.chars().next_back().is_some_and(char::is_alphanumeric)
                && right.chars().next().is_some_and(char::is_alphanumeric)
            {
                return vec![fix(word_start + at, word_start + at + 2, "—")];
            }
        }
        // word - word: an en dash.
        let before = &text[..word_start];
        if let Some(ahead) = before.strip_suffix(" - ") {
            if ahead.chars().next_back().is_some_and(char::is_alphanumeric) {
                let at = word_start - 2;
                fixes.push(fix(at, at + 1, "–"));
            }
        }
    }
    let lower = word.to_lowercase();
    if options.capitalize_days
        && DAYS.contains(&lower.as_str())
        && word.chars().next().is_some_and(char::is_lowercase)
    {
        fixes.insert(0, fix(word_start, word_end, &capitalize(word)));
        return fixes;
    }
    if options.two_initial_capitals {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() >= 3
            && chars[0].is_ascii_uppercase()
            && chars[1].is_ascii_uppercase()
            && chars[2..].iter().all(|ch| ch.is_ascii_lowercase())
        {
            let fixed: String = chars[..1]
                .iter()
                .chain(std::iter::once(&chars[1].to_ascii_lowercase()))
                .chain(chars[2..].iter())
                .collect();
            fixes.insert(0, fix(word_start, word_end, &fixed));
            return fixes;
        }
    }
    if options.capitalize_sentences
        && word_start == sentence_start(text, word_start)
        && word
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && word
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || ch == '\'')
        && sentence_has_ended(text, word_start)
    {
        fixes.insert(0, fix(word_start, word_end, &capitalize(word)));
    }
    fixes
}

fn fix(start: usize, end: usize, text: &str) -> Fix {
    Fix {
        start,
        end,
        text: text.to_string(),
        superscript: None,
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Where the sentence `at` is in starts: after the last . ! or ? and the
/// spaces after it, or at the paragraph's start.
fn sentence_start(text: &str, at: usize) -> usize {
    let before = &text[..at];
    let trimmed = before.trim_end_matches([' ', '\t', '\u{3000}']);
    if trimmed.is_empty() {
        return 0;
    }
    if trimmed.ends_with(['.', '!', '?']) && trimmed.len() < before.len() {
        return at;
    }
    usize::MAX
}

/// Whether what came before `at` is a finished sentence (or nothing): not
/// an abbreviation such as "e.g." or "Mr.".
fn sentence_has_ended(text: &str, at: usize) -> bool {
    let before = text[..at].trim_end();
    if before.is_empty() {
        return true;
    }
    let last_word = before.rsplit(char::is_whitespace).next().unwrap_or("");
    !matches!(
        last_word.to_lowercase().as_str(),
        "e.g." | "i.e." | "etc." | "mr." | "mrs." | "ms." | "dr." | "vs." | "no." | "p."
    )
}

/// The byte offset of an ordinal's suffix in `word` (1st, 22nd, 103rd,
/// 11th), when `word` is one.
fn ordinal_suffix(word: &str) -> Option<usize> {
    let digits = word.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let (number, suffix) = word.split_at(digits);
    let value: u64 = number.parse().ok()?;
    let wanted = match (value % 100, value % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    (suffix == wanted).then_some(digits)
}

const BRACKETS: [(char, char); 14] = [
    ('（', '）'),
    ('「', '」'),
    ('『', '』'),
    ('【', '】'),
    ('［', '］'),
    ('〔', '〕'),
    ('｛', '｝'),
    ('〈', '〉'),
    ('《', '》'),
    ('〘', '〙'),
    ('〖', '〗'),
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
];

/// かっこを正しく組み合わせる: the closing bracket that answers the last
/// one still open before `typed`, when `typed` closes a bracket at all.
fn matching_bracket(before: &str, typed: char) -> Option<char> {
    if !BRACKETS.iter().any(|(_, close)| *close == typed) {
        return None;
    }
    let mut open = Vec::new();
    for ch in before.chars() {
        if let Some((_, close)) = BRACKETS.iter().find(|(opening, _)| *opening == ch) {
            open.push(*close);
        } else if BRACKETS.iter().any(|(_, close)| *close == ch) {
            open.pop();
        }
    }
    let wanted = *open.last()?;
    // Only between kinds of the same width: a full-width bracket is not
    // answered with an ASCII one.
    (typed.is_ascii() == wanted.is_ascii()).then_some(wanted)
}

/// Whether the paragraph's start, as typed so far, is a list marker — "1. ",
/// "* ", "・" — that 入力オートフォーマット turns into a list. Called when a
/// space or a tab is typed after it.
pub fn list_start(text: &str, caret: usize, options: &AutoCorrectOptions) -> Option<ListStart> {
    let caret = caret.min(text.len());
    let typed = text[..caret].chars().next_back()?;
    if !matches!(typed, ' ' | '\t' | '\u{3000}') {
        return None;
    }
    let marker = &text[..caret - typed.len_utf8()];
    // Only at the paragraph's start, with nothing after the caret.
    if marker.is_empty() || marker.contains(char::is_whitespace) {
        return None;
    }
    if options.bulleted_lists {
        let bullet = match marker {
            "*" | "＊" => Some(("●", None)),
            "-" | "－" => Some(("–", None)),
            ">" => Some(("Ø", Some("Wingdings"))),
            "・" | "●" | "■" | "◆" | "□" | "○" | "◇" | "◎" | "★" | "☆" => {
                Some((marker, None))
            }
            _ => None,
        };
        if let Some((glyph, font)) = bullet {
            return Some(ListStart {
                marker: caret,
                format: "bullet".to_string(),
                text: glyph.to_string(),
                font: font.map(str::to_string),
                start: 1,
            });
        }
    }
    if options.numbered_lists {
        if let Some((format, text, start)) = numbered_marker(marker) {
            return Some(ListStart {
                marker: caret,
                format: format.to_string(),
                text,
                font: None,
                start,
            });
        }
    }
    None
}

/// A typed number marker's format, level text and value: 1. 1) (1) a.
/// A) i. １． ① 第1.
fn numbered_marker(marker: &str) -> Option<(&'static str, String, u32)> {
    let circled = |ch: char| -> Option<u32> {
        let code = ch as u32;
        match code {
            0x2460..=0x2473 => Some(code - 0x2460 + 1),
            _ => None,
        }
    };
    let mut chars = marker.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        if let Some(value) = circled(first) {
            return Some(("decimalEnclosedCircle", "%1".to_string(), value));
        }
    }
    let (open, body, close) = if let Some(inner) = marker
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        ("(", inner, ")")
    } else if let Some(inner) = marker
        .strip_prefix('（')
        .and_then(|rest| rest.strip_suffix('）'))
    {
        ("（", inner, "）")
    } else if let Some(inner) = marker.strip_suffix('.') {
        ("", inner, ".")
    } else if let Some(inner) = marker.strip_suffix(')') {
        ("", inner, ")")
    } else if let Some(inner) = marker.strip_suffix('．') {
        ("", inner, "．")
    } else {
        return None;
    };
    if body.is_empty() || body.chars().count() > 4 {
        return None;
    }
    let level_text = format!("{open}%1{close}");
    if body.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(("decimal", level_text, body.parse().ok()?));
    }
    if body.chars().all(|ch| ('０'..='９').contains(&ch)) {
        let value: String = body
            .chars()
            .map(|ch| char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or('0'))
            .collect();
        return Some(("decimalFullWidth", level_text, value.parse().ok()?));
    }
    // Roman numerals before letters: "i." is i, not the ninth letter.
    let roman = |text: &str| -> Option<u32> {
        let table = [("x", 10), ("ix", 9), ("v", 5), ("iv", 4), ("i", 1)];
        let mut rest = text;
        let mut value = 0;
        while !rest.is_empty() {
            let (prefix, number) = table.iter().find(|(prefix, _)| rest.starts_with(prefix))?;
            value += number;
            rest = &rest[prefix.len()..];
        }
        Some(value)
    };
    if matches!(
        body,
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x"
    ) {
        return Some(("lowerRoman", level_text, roman(body)?));
    }
    if matches!(
        body,
        "I" | "II" | "III" | "IV" | "V" | "VI" | "VII" | "VIII" | "IX" | "X"
    ) {
        return Some(("upperRoman", level_text, roman(&body.to_lowercase())?));
    }
    let mut letters = body.chars();
    if let (Some(letter), None) = (letters.next(), letters.next()) {
        if letter.is_ascii_lowercase() {
            return Some(("lowerLetter", level_text, letter as u32 - 'a' as u32 + 1));
        }
        if letter.is_ascii_uppercase() {
            return Some(("upperLetter", level_text, letter as u32 - 'A' as u32 + 1));
        }
    }
    None
}

/// The 結語 Word answers a 頭語 with.
pub fn closing_for(opening: &str) -> Option<&'static str> {
    match opening {
        "拝啓" | "拝復" | "拝呈" | "啓上" => Some("敬具"),
        "謹啓" | "謹呈" | "恭啓" => Some("謹白"),
        "前略" | "冠省" | "急啓" | "急白" => Some("草々"),
        _ => None,
    }
}

/// What Enter makes of the paragraph `text` it ends; `following` is the
/// document's text after it, to see whether a 結語 or 以上 is already
/// there.
pub fn on_enter(text: &str, following: &str, options: &AutoCorrectOptions) -> Option<EnterFormat> {
    let trimmed = text.trim();
    if options.borders && trimmed.chars().count() >= 3 {
        let first = trimmed.chars().next()?;
        if trimmed.chars().all(|ch| ch == first) {
            let edge = |style: &str, width: f32| HeadlessBorderEdge {
                style: Some(style.to_string()),
                width: Some(width),
                color: Some("auto".to_string()),
            };
            let border = match first {
                '-' => Some(edge("single", 0.75)),
                '_' => Some(edge("single", 1.5)),
                '=' => Some(edge("double", 0.75)),
                '*' => Some(edge("dotted", 2.25)),
                '~' => Some(edge("wave", 0.75)),
                '#' => Some(edge("thinThickThinSmallGap", 2.25)),
                _ => None,
            };
            if let Some(border) = border {
                return Some(EnterFormat::Border(border));
            }
        }
    }
    if options.closing_phrases {
        let opening: String = trimmed.chars().take(2).collect();
        if let Some(closing) = closing_for(&opening) {
            if !following.contains(closing) {
                return Some(EnterFormat::Closing(closing));
            }
        }
    }
    if options.kiki && trimmed == "記" && !following.contains("以上") {
        return Some(EnterFormat::Kiki);
    }
    None
}

/// 組み込みの見出しスタイル: a short line ended by two Enters is a heading.
/// `previous` is the paragraph before the empty one Enter was pressed in.
pub fn heading_from(previous: &str, options: &AutoCorrectOptions) -> bool {
    let text = previous.trim();
    options.headings
        && !text.is_empty()
        && text.chars().count() <= 60
        && !text.ends_with(['.', '。', ',', '、', ':', ';', '!', '?', '！', '？'])
        && text.chars().next().is_some_and(|ch| !ch.is_lowercase())
}

/// The corrections, made on any session.
pub trait WordAutoCorrect: WordSession {
    /// Makes `fixes` in paragraph `paragraph` as one undo step; answers how
    /// far the caret at `caret` moves (bytes, after the fixes).
    fn apply_fixes(
        &mut self,
        paragraph: usize,
        fixes: &[Fix],
        caret: usize,
    ) -> Result<usize, String> {
        if fixes.is_empty() {
            return Ok(caret);
        }
        let mut commands = Vec::new();
        let mut caret = caret as isize;
        let mut sorted = fixes.to_vec();
        sorted.sort_by(|a, b| b.start.cmp(&a.start));
        for fix in &sorted {
            commands.push(HeadlessCommand::ReplaceRange {
                start_paragraph: paragraph,
                start_offset: fix.start,
                end_paragraph: paragraph,
                end_offset: fix.end,
                text: fix.text.clone(),
            });
            if let Some((from, to)) = fix.superscript {
                commands.push(HeadlessCommand::ApplySuperscript {
                    start_paragraph: paragraph,
                    start_offset: fix.start + from,
                    end_paragraph: paragraph,
                    end_offset: fix.start + to,
                    enabled: true,
                });
            }
            if (fix.end as isize) <= caret {
                caret += fix.text.len() as isize - (fix.end - fix.start) as isize;
            }
        }
        self.apply_commands(commands, false)?;
        Ok(caret.max(0) as usize)
    }

    /// The paragraph's marker taken out and the paragraph made a list.
    fn start_list(&mut self, paragraph: usize, list: &ListStart) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::ReplaceRange {
                start_paragraph: paragraph,
                start_offset: 0,
                end_paragraph: paragraph,
                end_offset: list.marker,
                text: String::new(),
            },
            false,
        )?;
        self.set_selection_range(Some(super::SelectionRangeOf::collapsed(
            jade_selection::TextPosition::new(paragraph, 0),
        )));
        self.apply_list_format(&list.format, &list.text, list.font.as_deref())?;
        if list.start != 1 {
            self.set_list_start(list.start)?;
        }
        Ok(())
    }

    /// Enter's formats for paragraph `paragraph` (the one Enter ended; the
    /// caret is in the new one after it).
    fn format_on_enter(&mut self, paragraph: usize, format: &EnterFormat) -> Result<(), String> {
        match format {
            EnterFormat::Border(edge) => {
                let length = self
                    .word_document()
                    .paragraph(paragraph)
                    .map(|p| p.runs.iter().map(|run| run.text.len()).sum())
                    .unwrap_or(0);
                self.apply_commands(
                    vec![
                        HeadlessCommand::ReplaceRange {
                            start_paragraph: paragraph,
                            start_offset: 0,
                            end_paragraph: paragraph,
                            end_offset: length,
                            text: String::new(),
                        },
                        HeadlessCommand::SetParagraphBorders {
                            start_paragraph: paragraph,
                            end_paragraph: paragraph,
                            top: None,
                            bottom: Some(edge.clone()),
                            left: None,
                            right: None,
                        },
                    ],
                    false,
                )
            }
            EnterFormat::Closing(closing) => {
                let at = paragraph + 1;
                self.apply_commands(
                    vec![
                        HeadlessCommand::InsertParagraphBreak {
                            paragraph: at,
                            offset: 0,
                            count: 1,
                        },
                        HeadlessCommand::InsertText {
                            paragraph: at + 1,
                            offset: 0,
                            text: closing.to_string(),
                        },
                        HeadlessCommand::SetParagraphAlignment {
                            start_paragraph: at + 1,
                            end_paragraph: at + 1,
                            alignment: "right".to_string(),
                        },
                    ],
                    false,
                )
            }
            EnterFormat::Kiki => {
                let at = paragraph + 1;
                self.apply_commands(
                    vec![
                        HeadlessCommand::SetParagraphAlignment {
                            start_paragraph: paragraph,
                            end_paragraph: paragraph,
                            alignment: "center".to_string(),
                        },
                        HeadlessCommand::SetParagraphAlignment {
                            start_paragraph: at,
                            end_paragraph: at,
                            alignment: "left".to_string(),
                        },
                        HeadlessCommand::InsertParagraphBreak {
                            paragraph: at,
                            offset: 0,
                            count: 1,
                        },
                        HeadlessCommand::InsertText {
                            paragraph: at + 1,
                            offset: 0,
                            text: "以上".to_string(),
                        },
                        HeadlessCommand::SetParagraphAlignment {
                            start_paragraph: at + 1,
                            end_paragraph: at + 1,
                            alignment: "right".to_string(),
                        },
                    ],
                    false,
                )
            }
        }
    }

    /// 組み込みの見出しスタイル: paragraph `paragraph` becomes 見出し 1.
    fn make_heading(&mut self, paragraph: usize) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::SetParagraphStyle {
                start_paragraph: paragraph,
                end_paragraph: paragraph,
                style_id: Some("Heading1".to_string()),
            },
            false,
        )
    }
}

impl<T: WordSession + ?Sized> WordAutoCorrect for T {}

/// The settings as lines of `key=value`, the replacement list as
/// `entry=from\tto`: what the shells save between runs.
pub fn options_to_text(options: &AutoCorrectOptions) -> String {
    let mut out = String::new();
    for (key, value) in flags(options) {
        out.push_str(&format!("{key}={}\n", if value { 1 } else { 0 }));
    }
    for (from, to) in &options.entries {
        out.push_str(&format!("entry={from}\t{to}\n"));
    }
    out
}

/// [`options_to_text`] read back; what it does not name keeps its default.
pub fn options_from_text(text: &str) -> AutoCorrectOptions {
    let mut options = AutoCorrectOptions::default();
    let mut entries = Vec::new();
    let mut saw_entries = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "entry" {
            saw_entries = true;
            if let Some((from, to)) = value.split_once('\t') {
                entries.push((from.to_string(), to.to_string()));
            }
            continue;
        }
        if let Some(flag) = flag_mut(&mut options, key) {
            *flag = value.trim() == "1";
        }
    }
    if saw_entries {
        options.entries = entries;
    }
    options
}

fn flags(options: &AutoCorrectOptions) -> Vec<(&'static str, bool)> {
    vec![
        ("two_initial_capitals", options.two_initial_capitals),
        ("capitalize_sentences", options.capitalize_sentences),
        ("capitalize_days", options.capitalize_days),
        ("replace_text", options.replace_text),
        ("smart_quotes", options.smart_quotes),
        ("ordinals", options.ordinals),
        ("fractions", options.fractions),
        ("hyphens", options.hyphens),
        ("bulleted_lists", options.bulleted_lists),
        ("numbered_lists", options.numbered_lists),
        ("borders", options.borders),
        ("headings", options.headings),
        ("long_vowels", options.long_vowels),
        ("brackets", options.brackets),
        ("closing_phrases", options.closing_phrases),
        ("kiki", options.kiki),
        ("cjk_spaces", options.cjk_spaces),
    ]
}

/// A setting by its key, to change.
pub fn flag_mut<'a>(options: &'a mut AutoCorrectOptions, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "two_initial_capitals" => &mut options.two_initial_capitals,
        "capitalize_sentences" => &mut options.capitalize_sentences,
        "capitalize_days" => &mut options.capitalize_days,
        "replace_text" => &mut options.replace_text,
        "smart_quotes" => &mut options.smart_quotes,
        "ordinals" => &mut options.ordinals,
        "fractions" => &mut options.fractions,
        "hyphens" => &mut options.hyphens,
        "bulleted_lists" => &mut options.bulleted_lists,
        "numbered_lists" => &mut options.numbered_lists,
        "borders" => &mut options.borders,
        "headings" => &mut options.headings,
        "long_vowels" => &mut options.long_vowels,
        "brackets" => &mut options.brackets,
        "closing_phrases" => &mut options.closing_phrases,
        "kiki" => &mut options.kiki,
        "cjk_spaces" => &mut options.cjk_spaces,
        _ => return None,
    })
}

/// The keys of the settings, in the dialog's order: オートコレクト's, then
/// 入力オートフォーマット's.
pub const AUTOCORRECT_KEYS: [&str; 4] = [
    "two_initial_capitals",
    "capitalize_sentences",
    "capitalize_days",
    "replace_text",
];
pub const AUTOFORMAT_KEYS: [&str; 13] = [
    "smart_quotes",
    "ordinals",
    "fractions",
    "hyphens",
    "bulleted_lists",
    "numbered_lists",
    "borders",
    "headings",
    "long_vowels",
    "brackets",
    "closing_phrases",
    "kiki",
    "cjk_spaces",
];

/// Where a position ends up after `fixes` in its paragraph.
pub fn position_after(position: Position, paragraph: usize, fixes: &[Fix]) -> Position {
    if position.0 != paragraph {
        return position;
    }
    let mut offset = position.1 as isize;
    for fix in fixes {
        if fix.end <= position.1 {
            offset += fix.text.len() as isize - (fix.end - fix.start) as isize;
        }
    }
    (paragraph, offset.max(0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types `typed` at the end of `text` and makes the fixes.
    fn after(text: &str, typed: &str) -> String {
        let options = AutoCorrectOptions::default();
        let mut text = format!("{text}{typed}");
        let caret = text.len();
        let mut fixes = corrections(&text, caret, &options);
        fixes.sort_by(|a, b| b.start.cmp(&a.start));
        for fix in fixes {
            text.replace_range(fix.start..fix.end, &fix.text);
        }
        text
    }

    #[test]
    fn the_replacement_list_and_capitals() {
        assert_eq!(after("I saw teh", " "), "I saw the ");
        assert_eq!(after("Copyright (c", ")"), "Copyright ©");
        assert_eq!(after("wait..", "."), "wait…");
        assert_eq!(after("go -->", " "), "go → ");
        assert_eq!(after("the end. next", " "), "the end. Next ");
        assert_eq!(after("hello", " "), "Hello ");
        assert_eq!(after("see e.g. this", " "), "see e.g. this ");
        assert_eq!(after("THe", " "), "The ");
        assert_eq!(after("on monday", " "), "on Monday ");
        assert_eq!(after("I said dont", "."), "I said don't.");
    }

    #[test]
    fn autoformat_as_you_type() {
        assert_eq!(after("He said ", "\""), "He said “");
        assert_eq!(after("He said “yes", "\""), "He said “yes”");
        assert_eq!(after("it", "'"), "it’");
        assert_eq!(after("add 1/2", " "), "add ½ ");
        assert_eq!(after("well--done", " "), "well—done ");
        assert_eq!(after("Tokyo - Osaka", " "), "Tokyo – Osaka ");
        let options = AutoCorrectOptions::default();
        let fixes = corrections("the 21st ", 9, &options);
        assert!(
            fixes.iter().any(|fix| fix.superscript == Some((2, 4))),
            "{fixes:?}"
        );
    }

    #[test]
    fn the_japanese_rules() {
        // 長音とダッシュ.
        assert_eq!(after("コ", "ー"), "コー");
        assert_eq!(after("東京", "ー"), "東京―");
        assert_eq!(after("テスト", "－"), "テストー");
        // かっこ.
        assert_eq!(after("（注意", "」"), "（注意）");
        assert_eq!(after("「引用", "」"), "「引用」");
        assert_eq!(after("「a（b", "」"), "「a（b）");
        // 日本語と英数字の間の不要なスペース.
        assert_eq!(after("日本 ", "W"), "日本W");
        assert_eq!(after("Word ", "の"), "Wordの");
        assert_eq!(after("日本 ", "語"), "日本 語");
        // A word right after Japanese text is still a word.
        assert_eq!(after("情報teh", " "), "情報the ");
    }

    #[test]
    fn markers_start_lists() {
        let options = AutoCorrectOptions::default();
        let list = list_start("1. ", 3, &options).unwrap();
        assert_eq!(
            (list.format.as_str(), list.text.as_str(), list.start),
            ("decimal", "%1.", 1)
        );
        let list = list_start("3) ", 3, &options).unwrap();
        assert_eq!((list.text.as_str(), list.start), ("%1)", 3));
        assert_eq!(list_start("(2)\t", 4, &options).unwrap().text, "(%1)");
        assert_eq!(
            list_start("a. ", 3, &options).unwrap().format,
            "lowerLetter"
        );
        assert_eq!(
            list_start("iv. ", 4, &options).unwrap().format,
            "lowerRoman"
        );
        assert_eq!(
            list_start("① ", 4, &options).unwrap().format,
            "decimalEnclosedCircle"
        );
        assert_eq!(
            list_start("１． ", 7, &options).unwrap().format,
            "decimalFullWidth"
        );
        assert_eq!(list_start("* ", 2, &options).unwrap().format, "bullet");
        let dot = list_start("・ ", 4, &options).unwrap();
        assert_eq!((dot.format.as_str(), dot.text.as_str()), ("bullet", "・"));
        assert!(list_start("word ", 5, &options).is_none());
        assert!(list_start("1.5 ", 4, &options).is_none());
        let off = AutoCorrectOptions {
            numbered_lists: false,
            ..AutoCorrectOptions::default()
        };
        assert!(list_start("1. ", 3, &off).is_none());
    }

    #[test]
    fn enter_formats() {
        let options = AutoCorrectOptions::default();
        match on_enter("---", "", &options) {
            Some(EnterFormat::Border(edge)) => assert_eq!(edge.style.as_deref(), Some("single")),
            other => panic!("{other:?}"),
        }
        match on_enter("===", "", &options) {
            Some(EnterFormat::Border(edge)) => assert_eq!(edge.style.as_deref(), Some("double")),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            on_enter("拝啓　時下ますます", "", &options),
            Some(EnterFormat::Closing("敬具"))
        );
        assert_eq!(on_enter("拝啓", "本文\n敬具", &options), None);
        assert_eq!(on_enter("記", "", &options), Some(EnterFormat::Kiki));
        assert_eq!(on_enter("--", "", &options), None);
        assert!(!heading_from("Introduction", &options));
        let headings = AutoCorrectOptions {
            headings: true,
            ..options
        };
        assert!(heading_from("Introduction", &headings));
        assert!(!heading_from("A sentence.", &headings));
    }

    #[test]
    fn the_settings_round_trip() {
        let mut options = AutoCorrectOptions {
            ordinals: false,
            ..AutoCorrectOptions::default()
        };
        options.entries.push(("jp".to_string(), "日本".to_string()));
        let read = options_from_text(&options_to_text(&options));
        assert_eq!(read, options);
        assert_eq!(options_from_text(""), AutoCorrectOptions::default());
    }
}
