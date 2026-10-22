//! 校閲 › 音声読み上げ › 読み上げ: the document read out from the caret,
//! a paragraph at a time, with the word being read selected as it is
//! spoken, as Word does; 再生 / 一時停止, 前へ / 次へ (a paragraph back or
//! on) and 読み上げの速度.
//!
//! The voice is the platform's — `NSSpeechSynthesizer` in the native app,
//! `speechSynthesis` in the browser. Both report where they are as a UTF-16
//! range of the string they were given; [`ReadAloud::word`] turns that into
//! the document's byte positions.

use super::{Helpers, Position, WordSession};

/// One paragraph to speak, from `offset` (a byte offset) on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utterance {
    pub paragraph: usize,
    pub offset: usize,
    pub text: String,
}

/// The speeds the 読み上げ settings offer, as a multiple of the voice's
/// normal rate.
pub const SPEEDS: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Words a minute at speed 1, for voices that are set in words a minute.
pub const WORDS_PER_MINUTE: f32 = 180.0;

/// A reading in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadAloud {
    utterances: Vec<Utterance>,
    current: usize,
    /// The word last reported, in document positions.
    word: Option<(Position, Position)>,
    pub paused: bool,
    /// A multiple of the voice's normal rate (one of [`SPEEDS`]).
    pub speed: f32,
}

/// The paragraphs from `from` to the end of the document that have
/// something to say. Table cells are read in their order in the document.
pub fn utterances<S: WordSession + ?Sized>(session: &S, from: Position) -> Vec<Utterance> {
    let count = session.paragraph_total();
    let mut out = Vec::new();
    for paragraph in from.0..count {
        let Some(text) = session.text_of(paragraph) else {
            continue;
        };
        let offset = if paragraph == from.0 {
            let mut offset = from.1.min(text.len());
            while !text.is_char_boundary(offset) {
                offset -= 1;
            }
            // From the start of the word the caret is in.
            text[..offset]
                .char_indices()
                .rev()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(at, ch)| at + ch.len_utf8())
                .unwrap_or(0)
        } else {
            0
        };
        let rest = &text[offset..];
        if rest.trim().is_empty() {
            continue;
        }
        out.push(Utterance {
            paragraph,
            offset,
            text: rest.to_string(),
        });
    }
    out
}

/// The byte range of `text` that the UTF-16 range `start..start + length`
/// covers, clamped to the text.
pub fn utf16_to_bytes(text: &str, start: usize, length: usize) -> (usize, usize) {
    let mut units = 0;
    let mut begin = None;
    let mut end = text.len();
    for (at, ch) in text.char_indices() {
        if begin.is_none() && units >= start {
            begin = Some(at);
        }
        if units >= start + length {
            end = at;
            break;
        }
        units += ch.len_utf16();
    }
    let begin = begin.unwrap_or(text.len());
    (begin, end.max(begin))
}

impl ReadAloud {
    /// 読み上げ from `from`; `None` when there is nothing after it to read.
    pub fn start<S: WordSession + ?Sized>(session: &S, from: Position, speed: f32) -> Option<Self> {
        let utterances = utterances(session, from);
        if utterances.is_empty() {
            return None;
        }
        Some(Self {
            utterances,
            current: 0,
            word: None,
            paused: false,
            speed,
        })
    }

    /// What is being, or is to be, spoken now.
    pub fn utterance(&self) -> Option<&Utterance> {
        self.utterances.get(self.current)
    }

    /// The voice is at `start..start + length` (UTF-16) of the utterance:
    /// the word, as the document's positions to select.
    pub fn word(&mut self, start: usize, length: usize) -> Option<(Position, Position)> {
        let utterance = self.utterance()?;
        let (begin, end) = utf16_to_bytes(&utterance.text, start, length);
        let word = (
            (utterance.paragraph, utterance.offset + begin),
            (utterance.paragraph, utterance.offset + end),
        );
        self.word = Some(word);
        Some(word)
    }

    /// The word last read.
    pub fn current_word(&self) -> Option<(Position, Position)> {
        self.word
    }

    /// 次へ, or the voice finished the paragraph: the next one; `false`
    /// when the reading is over.
    pub fn forward(&mut self) -> bool {
        self.word = None;
        if self.current + 1 < self.utterances.len() {
            self.current += 1;
            true
        } else {
            self.current = self.utterances.len();
            false
        }
    }

    /// 前へ: the paragraph before, or this one again from its start.
    pub fn back(&mut self) {
        self.word = None;
        self.current = self.current.saturating_sub(1);
    }

    /// 読み上げの速度: the next speed up (`true`) or down.
    pub fn step_speed(&mut self, faster: bool) {
        let at = SPEEDS
            .iter()
            .position(|speed| (speed - self.speed).abs() < 0.01)
            .unwrap_or(2);
        let at = if faster {
            (at + 1).min(SPEEDS.len() - 1)
        } else {
            at.saturating_sub(1)
        };
        self.speed = SPEEDS[at];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_ranges_become_byte_ranges() {
        let text = "日本語 and 😀 words";
        assert_eq!(utf16_to_bytes(text, 0, 3), (0, 9));
        assert_eq!(utf16_to_bytes(text, 4, 3), (10, 13));
        // The emoji is two UTF-16 units.
        assert_eq!(utf16_to_bytes(text, 8, 2), (14, 18));
        assert_eq!(utf16_to_bytes(text, 11, 5), (19, 24));
        assert_eq!(utf16_to_bytes(text, 40, 2), (24, 24));
    }

    #[test]
    fn speeds_step_within_the_list() {
        let mut reading = ReadAloud {
            utterances: Vec::new(),
            current: 0,
            word: None,
            paused: false,
            speed: 1.0,
        };
        reading.step_speed(true);
        assert_eq!(reading.speed, 1.25);
        for _ in 0..10 {
            reading.step_speed(false);
        }
        assert_eq!(reading.speed, 0.5);
    }
}
