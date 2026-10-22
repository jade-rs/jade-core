//! 文字カウント over any [`WordSession`]: words and characters, of the
//! document or of the selection, counted the way Word counts them — which the
//! status bar shows, as Word's does.
//!
//! Word counts a run of letters or digits between spaces as one word, and
//! every East Asian character — kanji, kana, full-width forms and
//! punctuation, hangul — as a word of its own: 「日本語の文書」 is six words,
//! `jade 文書` three. Characters are counted without spaces (文字数 (スペースを
//! 含めない)) and with them; paragraph marks and line breaks are neither.

use jade_docx_headless_api::{HeadlessQuery, HeadlessQueryResult};

use super::{Helpers, WordSession};

/// What 文字カウント shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WordCount {
    /// 単語数.
    pub words: usize,
    /// 文字数 (スペースを含めない).
    pub characters: usize,
    /// 文字数 (スペースを含める).
    pub characters_with_spaces: usize,
    /// 段落数: the paragraphs with text.
    pub paragraphs: usize,
    /// 全角文字 + 半角カタカナ.
    pub east_asian: usize,
    /// 半角英数の単語数.
    pub latin_words: usize,
}

impl std::ops::AddAssign for WordCount {
    fn add_assign(&mut self, other: Self) {
        self.words += other.words;
        self.characters += other.characters;
        self.characters_with_spaces += other.characters_with_spaces;
        self.paragraphs += other.paragraphs;
        self.east_asian += other.east_asian;
        self.latin_words += other.latin_words;
    }
}

/// Whether Word counts `ch` as a word on its own: the East Asian scripts,
/// their punctuation and the full- and half-width forms.
pub fn is_east_asian(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x11FF       // Hangul Jamo
        | 0x2E80..=0x2FDF     // CJK radicals, Kangxi radicals
        | 0x3001..=0x303F     // CJK symbols and punctuation (not the ideographic space)
        | 0x3040..=0x30FF     // Hiragana, Katakana
        | 0x3100..=0x31FF     // Bopomofo, Hangul compatibility Jamo, Katakana extensions
        | 0x3200..=0x33FF     // Enclosed CJK letters, CJK compatibility
        | 0x3400..=0x4DBF     // CJK extension A
        | 0x4E00..=0x9FFF     // CJK unified ideographs
        | 0xA960..=0xA97F     // Hangul Jamo extended A
        | 0xAC00..=0xD7FF     // Hangul syllables, Jamo extended B
        | 0xF900..=0xFAFF     // CJK compatibility ideographs
        | 0xFE30..=0xFE4F     // CJK compatibility forms
        | 0xFF01..=0xFFEF     // Full-width forms, half-width katakana
        | 0x20000..=0x3FFFF   // CJK extensions B and on
    )
}

/// Counts one paragraph's text (or a piece of one).
pub fn count_text(text: &str) -> WordCount {
    let mut count = WordCount::default();
    let mut in_word = false;
    for ch in text.chars() {
        if matches!(ch, '\n' | '\r') {
            in_word = false;
            continue;
        }
        count.characters_with_spaces += 1;
        if ch.is_whitespace() || ch.is_control() {
            in_word = false;
            continue;
        }
        count.characters += 1;
        if is_east_asian(ch) {
            count.east_asian += 1;
            count.words += 1;
            in_word = false;
        } else if !in_word {
            count.words += 1;
            count.latin_words += 1;
            in_word = true;
        }
    }
    if count.characters > 0 {
        count.paragraphs = 1;
    }
    count
}

/// A count as the status bar shows it, in thousands: `12,345`.
pub fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// 文字カウント over every session.
pub trait WordCounting: WordSession {
    /// The whole document: every paragraph, in tables, text boxes and notes
    /// too.
    fn document_count(&self) -> WordCount {
        let total = self.paragraph_total();
        let Some(HeadlessQueryResult::ParagraphSlice { paragraphs, .. }) =
            self.query(HeadlessQuery::ParagraphSlice {
                start_paragraph: 0,
                max_paragraphs: total.max(1),
            })
        else {
            return WordCount::default();
        };
        let mut count = WordCount::default();
        for paragraph in paragraphs {
            count += count_text(&paragraph.text);
        }
        count
    }

    /// The selection, when there is one.
    fn selection_count(&self) -> Option<WordCount> {
        let range = self
            .selection_range()
            .filter(|range| !range.is_collapsed())?;
        let (start, end) = range.normalized();
        let mut count = WordCount::default();
        for paragraph in start.paragraph..=end.paragraph {
            let text = self.text_of(paragraph).unwrap_or_default();
            let from = if paragraph == start.paragraph {
                start.offset.min(text.len())
            } else {
                0
            };
            let to = if paragraph == end.paragraph {
                end.offset.min(text.len())
            } else {
                text.len()
            };
            if let Some(piece) = text.get(from..to.max(from)) {
                count += count_text(piece);
            }
        }
        Some(count)
    }
}

impl<T: WordSession + ?Sized> WordCounting for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn east_asian_characters_are_words_of_their_own() {
        let count = count_text("日本語の文書");
        assert_eq!((count.words, count.characters), (6, 6));
        assert_eq!(count.east_asian, 6);

        let count = count_text("jade 文書、two  words.");
        // jade, 文, 書, 、, two, words.
        assert_eq!(count.words, 6);
        assert_eq!(count.latin_words, 3);
        assert_eq!(count.east_asian, 3);
        assert_eq!(count.characters, "jade文書、twowords.".chars().count());
        assert_eq!(
            count.characters_with_spaces,
            "jade 文書、two  words.".chars().count()
        );
    }

    #[test]
    fn counts_are_grouped_in_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1234), "1,234");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn spaces_and_breaks_are_not_characters() {
        let count = count_text("a\u{3000}b\tc\nd");
        assert_eq!(count.words, 4);
        assert_eq!(count.characters, 4);
        // The ideographic space and the tab are spaces; the break is not.
        assert_eq!(count.characters_with_spaces, 6);
        assert_eq!(count_text("   ").paragraphs, 0);
        // Half-width katakana are East Asian; full-width digits too.
        assert_eq!(count_text("ｶﾀｶﾅ１２").words, 6);
    }
}
