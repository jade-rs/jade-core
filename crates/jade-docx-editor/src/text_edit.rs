//! Text being typed into a cell, a name box, a sheet tab or a shape: the
//! text, the caret, an IME's composition, and a whole-text selection. Both
//! shells edit through it, so the caret moves the same way in each.
//!
//! Offsets are bytes into the text; the input handlers' UTF-16 offsets are
//! converted with [`utf16_to_byte`] and [`byte_to_utf16`].

use std::ops::Range;

/// UTF-16 offset → byte offset in `text`, clamped to its end.
pub fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

/// Byte offset → UTF-16 offset in `text`.
pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    // An offset inside a character counts up to the character's start.
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].encode_utf16().count()
}

/// Where ⌥→ (Ctrl+→ elsewhere) goes in `text` from `offset`: past any
/// spaces and punctuation, then over a run of letters and digits — a
/// Japanese run as a whole, having no spaces to break on.
pub fn word_end_after(text: &str, offset: usize) -> usize {
    let offset = floor_boundary(text, offset);
    let rest = &text[offset..];
    let skipped = rest
        .char_indices()
        .find(|(_, ch)| ch.is_alphanumeric())
        .map_or(rest.len(), |(at, _)| at);
    let word = rest[skipped..]
        .char_indices()
        .find(|(_, ch)| !ch.is_alphanumeric())
        .map_or(rest.len() - skipped, |(at, _)| at);
    offset + skipped + word
}

/// Where ⌥← goes in `text` from `offset`: back past spaces and punctuation,
/// then to the start of the run of letters and digits before them.
pub fn word_start_before(text: &str, offset: usize) -> usize {
    let before = &text[..floor_boundary(text, offset)];
    let skipped = before
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_alphanumeric())
        .map_or(0, |(at, ch)| at + ch.len_utf8());
    before[..skipped]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_alphanumeric())
        .map_or(0, |(at, ch)| at + ch.len_utf8())
}

/// `offset`, clamped to `text` and moved back onto a character boundary.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Text being typed: into a cell or into a shape.
pub struct TextEdit {
    pub text: String,
    /// The IME's composition, as a byte range of `text`.
    pub marked: Option<Range<usize>>,
    /// Where typing goes, as a byte offset into `text`.
    pub caret: usize,
    /// The whole text is selected: what is typed next replaces it, as when
    /// a sheet tab is renamed.
    pub whole: bool,
}

impl TextEdit {
    pub fn new(text: String) -> Self {
        let caret = text.len();
        Self {
            text,
            marked: None,
            caret,
            whole: false,
        }
    }

    /// The text with all of it selected.
    pub fn selected(text: String) -> Self {
        Self {
            whole: true,
            ..Self::new(text)
        }
    }

    /// The caret, kept on a character boundary inside the text.
    pub fn caret(&self) -> usize {
        let mut caret = self.caret.min(self.text.len());
        while !self.text.is_char_boundary(caret) {
            caret -= 1;
        }
        caret
    }

    /// Replaces a UTF-16 range — or the composition, or nothing at the
    /// caret — with `text`, marking it as the composition when `mark` is
    /// set. The caret ends after what went in.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str, mark: bool) {
        if std::mem::take(&mut self.whole) && range.is_none() && self.marked.is_none() {
            self.text.clear();
            self.caret = 0;
        }
        let caret = self.caret();
        let bytes = match range {
            Some(range) => {
                utf16_to_byte(&self.text, range.start)..utf16_to_byte(&self.text, range.end)
            }
            None => self.marked.clone().unwrap_or(caret..caret),
        };
        let start = bytes.start;
        self.text.replace_range(bytes, text);
        self.marked = (mark && !text.is_empty()).then(|| start..start + text.len());
        self.caret = start + text.len();
    }

    pub fn insert(&mut self, text: &str) {
        self.replace(None, text, false);
    }

    /// Backspace: the character before the caret.
    pub fn backspace(&mut self) {
        if std::mem::take(&mut self.whole) {
            self.text.clear();
            self.caret = 0;
            return;
        }
        let caret = self.caret();
        if let Some((start, _)) = self.text[..caret].char_indices().next_back() {
            self.text.replace_range(start..caret, "");
            self.caret = start;
        }
    }

    /// Forward delete: the character after the caret.
    pub fn delete(&mut self) {
        if std::mem::take(&mut self.whole) {
            self.text.clear();
            self.caret = 0;
            return;
        }
        let caret = self.caret();
        if let Some(ch) = self.text[caret..].chars().next() {
            self.text.replace_range(caret..caret + ch.len_utf8(), "");
        }
    }

    /// The caret one character left or right, or to either end.
    pub fn step(&mut self, forward: bool) {
        if std::mem::take(&mut self.whole) {
            self.caret = if forward { self.text.len() } else { 0 };
            return;
        }
        let caret = self.caret();
        self.caret = if forward {
            self.text[caret..]
                .chars()
                .next()
                .map_or(caret, |ch| caret + ch.len_utf8())
        } else {
            self.text[..caret]
                .char_indices()
                .next_back()
                .map_or(0, |(start, _)| start)
        };
    }

    pub fn to_end(&mut self, end: bool) {
        self.whole = false;
        self.caret = if end { self.text.len() } else { 0 };
    }

    /// A field's own editing keys — Backspace, Delete, the arrows, Home and
    /// End — by gpui's key name. Returns whether `key` was one.
    pub fn edit_key(&mut self, key: &str) -> bool {
        match key {
            "backspace" => self.backspace(),
            "delete" => self.delete(),
            "left" => self.step(false),
            "right" => self.step(true),
            "home" | "up" => self.to_end(false),
            "end" | "down" => self.to_end(true),
            _ => return false,
        }
        true
    }

    /// The text either side of the caret, for drawing it.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.caret())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caret_moves_by_characters_and_a_whole_selection_is_replaced() {
        let mut text = TextEdit::new("日本".to_string());
        text.step(false);
        text.insert("x");
        assert_eq!(text.text, "日x本");
        text.backspace();
        text.delete();
        assert_eq!(text.text, "日");
        text.to_end(false);
        text.replace(None, "に", true);
        assert_eq!(text.marked, Some(0..3));
        let mut tab = TextEdit::selected("Sheet1".to_string());
        tab.insert("集計");
        assert_eq!(tab.text, "集計");
        let mut tab = TextEdit::selected("Sheet1".to_string());
        tab.step(true);
        tab.backspace();
        assert_eq!(tab.text, "Sheet");
        assert_eq!(utf16_to_byte("a日b", 2), 4);
        assert_eq!(byte_to_utf16("a日b", 4), 2);
    }

    /// ⌥← and ⌥→ skip spaces and punctuation, then a run of letters and
    /// digits; a Japanese run counts as one word, as it has no spaces.
    #[test]
    fn words_are_runs_of_letters_and_digits() {
        let text = "Hello, wide 世界!";
        assert_eq!(word_end_after(text, 0), 5);
        assert_eq!(word_end_after(text, 5), 11);
        assert_eq!(word_end_after(text, 11), 18);
        assert_eq!(word_end_after(text, 18), text.len());
        assert_eq!(word_end_after(text, text.len()), text.len());
        assert_eq!(word_start_before(text, text.len()), 12);
        assert_eq!(word_start_before(text, 12), 7);
        assert_eq!(word_start_before(text, 7), 0);
        assert_eq!(word_start_before(text, 0), 0);
        // Mid-word, to that word's own edge.
        assert_eq!(word_start_before(text, 3), 0);
        assert_eq!(word_end_after(text, 3), 5);
    }
}
