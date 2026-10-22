//! 表示 › イマーシブ リーダー: the document's text alone, in a column, for
//! reading — 列幅 (狭い / 普通 / 広い), ページの色 (白 / セピア / 反転),
//! 行フォーカス (1, 3 or 5 lines lit, the rest dimmed, moved with ↑ ↓),
//! テキストの間隔 (wider words and lines) and 音節 (words broken into
//! syllables with a ·), and 読み上げ with the word being read lit.
//!
//! The shells wrap the text into lines with their own text system; what
//! each line shows and which part of the document it comes from is worked
//! out here, so a reading's word can be found on its line.

use super::{Helpers, WordSession};

/// 列幅.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnWidth {
    Narrow,
    #[default]
    Moderate,
    Wide,
}

impl ColumnWidth {
    /// The column's width in pixels at the reader's font size.
    pub fn pixels(self) -> f32 {
        match self {
            ColumnWidth::Narrow => 460.0,
            ColumnWidth::Moderate => 640.0,
            ColumnWidth::Wide => 860.0,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            ColumnWidth::Narrow => "immersive.column.narrow",
            ColumnWidth::Moderate => "immersive.column.moderate",
            ColumnWidth::Wide => "immersive.column.wide",
        }
    }

    pub const ALL: [ColumnWidth; 3] = [
        ColumnWidth::Narrow,
        ColumnWidth::Moderate,
        ColumnWidth::Wide,
    ];
}

/// ページの色.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageColor {
    #[default]
    White,
    Sepia,
    Inverse,
}

impl PageColor {
    /// The page and its text, as `0xRRGGBB`.
    pub fn colors(self) -> (u32, u32) {
        match self {
            PageColor::White => (0xFFFFFF, 0x1F1F1F),
            PageColor::Sepia => (0xF4ECD8, 0x5B4636),
            PageColor::Inverse => (0x121212, 0xF0F0F0),
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            PageColor::White => "immersive.page.white",
            PageColor::Sepia => "immersive.page.sepia",
            PageColor::Inverse => "immersive.page.inverse",
        }
    }

    pub const ALL: [PageColor; 3] = [PageColor::White, PageColor::Sepia, PageColor::Inverse];
}

/// The reader's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ImmersiveSettings {
    pub column: ColumnWidth,
    pub page: PageColor,
    /// 行フォーカス: how many lines are lit (1, 3 or 5), or off.
    pub line_focus: Option<usize>,
    /// テキストの間隔.
    pub text_spacing: bool,
    /// 音節.
    pub syllables: bool,
    /// The text's size in pixels.
    pub font_size: f32,
}

impl Default for ImmersiveSettings {
    fn default() -> Self {
        Self {
            column: ColumnWidth::default(),
            page: PageColor::default(),
            line_focus: None,
            text_spacing: false,
            syllables: false,
            font_size: 22.0,
        }
    }
}

impl ImmersiveSettings {
    /// A line's height, in pixels.
    pub fn line_height(&self) -> f32 {
        self.font_size * if self.text_spacing { 2.0 } else { 1.6 }
    }

    /// The space between paragraphs, in pixels.
    pub fn paragraph_gap(&self) -> f32 {
        self.font_size * if self.text_spacing { 1.4 } else { 0.9 }
    }

    /// 行フォーカス's choices, in the order the menu lists them.
    pub const LINE_FOCUS: [Option<usize>; 4] = [None, Some(1), Some(3), Some(5)];
}

/// A paragraph as the reader shows it: `text` with its spacing and
/// syllables, and for each of its bytes (and its end) the byte of the
/// document's paragraph it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownParagraph {
    pub paragraph: usize,
    pub text: String,
    pub source: Vec<usize>,
}

/// The middle dot between syllables.
pub const SYLLABLE_MARK: char = '·';

/// `text` as the reader shows it.
pub fn shown(paragraph: usize, text: &str, settings: &ImmersiveSettings) -> ShownParagraph {
    let mut out = String::with_capacity(text.len() * 2);
    let mut source = Vec::with_capacity(text.len() * 2);
    let push = |out: &mut String, source: &mut Vec<usize>, piece: &str, at: usize| {
        out.push_str(piece);
        source.extend(std::iter::repeat_n(at, piece.len()));
    };
    let mut rest = text;
    let mut offset = 0;
    while !rest.is_empty() {
        let word_len = rest
            .char_indices()
            .find(|(_, ch)| !ch.is_alphabetic())
            .map(|(at, _)| at)
            .unwrap_or(rest.len());
        if word_len > 0 {
            let word = &rest[..word_len];
            let mut at = offset;
            if settings.syllables && word.chars().all(|ch| ch.is_ascii_alphabetic()) {
                for (index, syllable) in hypher::hyphenate(word, hypher::Lang::English).enumerate()
                {
                    if index > 0 {
                        push(&mut out, &mut source, &SYLLABLE_MARK.to_string(), at);
                    }
                    for (byte, ch) in syllable.char_indices() {
                        push(
                            &mut out,
                            &mut source,
                            ch.encode_utf8(&mut [0; 4]),
                            at + byte,
                        );
                    }
                    at += syllable.len();
                }
            } else {
                for (byte, ch) in word.char_indices() {
                    push(
                        &mut out,
                        &mut source,
                        ch.encode_utf8(&mut [0; 4]),
                        at + byte,
                    );
                }
            }
            offset += word_len;
            rest = &rest[word_len..];
            continue;
        }
        let ch = rest.chars().next().expect("not empty");
        let piece = if settings.text_spacing && ch == ' ' {
            "   ".to_string()
        } else if ch == '\t' || ch == '\n' {
            " ".to_string()
        } else {
            ch.to_string()
        };
        push(&mut out, &mut source, &piece, offset);
        offset += ch.len_utf8();
        rest = &rest[ch.len_utf8()..];
    }
    source.push(text.len());
    ShownParagraph {
        paragraph,
        text: out,
        source,
    }
}

/// Every paragraph with text, as the reader shows it.
pub fn shown_paragraphs<S: WordSession + ?Sized>(
    session: &S,
    settings: &ImmersiveSettings,
) -> Vec<ShownParagraph> {
    (0..session.paragraph_total())
        .filter_map(|paragraph| {
            let text = session.text_of(paragraph)?;
            (!text.trim().is_empty()).then(|| shown(paragraph, &text, settings))
        })
        .collect()
}

/// One line of the reader: part of a shown paragraph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderLine {
    pub paragraph: usize,
    /// The line's bytes of the shown text.
    pub start: usize,
    pub end: usize,
    /// Whether it is the first line of its paragraph.
    pub first: bool,
}

/// The paragraph's lines, from where the shell's wrapper broke it (the
/// shown text's byte offsets each new line starts at).
pub fn lines_of(shown: &ShownParagraph, breaks: &[usize]) -> Vec<ReaderLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    for &at in breaks.iter().chain(std::iter::once(&shown.text.len())) {
        if at <= start && at != shown.text.len() {
            continue;
        }
        lines.push(ReaderLine {
            paragraph: shown.paragraph,
            start,
            end: at,
            first: start == 0,
        });
        start = at;
        if start >= shown.text.len() {
            break;
        }
    }
    lines
}

/// The shown bytes of `line` that show `range` of the document's
/// paragraph, if any.
pub fn shown_range(
    shown: &ShownParagraph,
    line: &ReaderLine,
    range: (usize, usize),
) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for at in line.start..line.end {
        let source = shown.source[at];
        if source >= range.0 && source < range.1 && shown.text.is_char_boundary(at) {
            found = Some(match found {
                Some((start, _)) => (start, at),
                None => (at, at),
            });
        }
    }
    found.map(|(start, last)| {
        let end = shown.text[last..]
            .chars()
            .next()
            .map(|ch| last + ch.len_utf8())
            .unwrap_or(last);
        (start, end)
    })
}

/// 行フォーカス: the first lit line for `count` lines lit around `at`,
/// kept within the `total` lines.
pub fn focus_top(at: usize, count: usize, total: usize) -> usize {
    let top = at.saturating_sub(count / 2);
    top.min(total.saturating_sub(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllables_and_spacing_keep_their_place_in_the_text() {
        let settings = ImmersiveSettings {
            syllables: true,
            text_spacing: true,
            ..ImmersiveSettings::default()
        };
        let shown = shown(4, "Reading is wonderful 日本", &settings);
        assert_eq!(shown.text, "Read·ing   is   won·der·ful   日本");
        assert_eq!(shown.source.len(), shown.text.len() + 1);
        // "der" shows bytes 14..17 of "wonderful", which start at 11.
        let der = shown.text.find("der").unwrap();
        assert_eq!(shown.source[der], 14);
        let line = ReaderLine {
            paragraph: 4,
            start: 0,
            end: shown.text.len(),
            first: true,
        };
        let (start, end) = shown_range(&shown, &line, (11, 20)).unwrap();
        assert_eq!(&shown.text[start..end], "won·der·ful");
        let (start, end) = shown_range(&shown, &line, (21, 27)).unwrap();
        assert_eq!(&shown.text[start..end], "日本");
    }

    #[test]
    fn lines_split_where_the_wrapper_broke_them() {
        let shown = shown(0, "one two three four", &ImmersiveSettings::default());
        let lines = lines_of(&shown, &[8, 14]);
        let texts: Vec<&str> = lines
            .iter()
            .map(|line| &shown.text[line.start..line.end])
            .collect();
        assert_eq!(texts, ["one two ", "three ", "four"]);
        assert!(lines[0].first && !lines[1].first);
        assert_eq!(lines_of(&shown, &[]).len(), 1);
    }

    #[test]
    fn line_focus_stays_within_the_lines() {
        assert_eq!(focus_top(0, 3, 10), 0);
        assert_eq!(focus_top(5, 3, 10), 4);
        assert_eq!(focus_top(9, 5, 10), 5);
        assert_eq!(focus_top(1, 5, 3), 0);
    }
}
