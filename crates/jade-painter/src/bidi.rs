//! Visual text order for direct paint operations without layout positions.

use std::borrow::Cow;
use unicode_bidi::{bidi_class, BidiClass, BidiInfo, Level};

use crate::{is_arabic, is_join_control};

/// Visual runs for a left-to-right canvas. Each Arabic run retains its
/// logical characters so OpenType can select joining forms.
pub struct VisualText<'a> {
    unidirectional: Option<&'a str>,
    runs: Vec<Cow<'a, str>>,
}

impl<'a> VisualText<'a> {
    pub fn new(text: &'a str) -> Self {
        if text.is_ascii() || !text.chars().any(is_arabic) || is_single_arabic_span(text) {
            return Self {
                unidirectional: Some(text),
                runs: Vec::new(),
            };
        }
        let bidi = BidiInfo::new(text, Some(Level::ltr()));
        let mut visual = Self {
            unidirectional: None,
            runs: Vec::new(),
        };
        for paragraph in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            for run in runs {
                if levels[run.start].is_rtl() {
                    visual.append_rtl(&text[run]);
                } else {
                    visual.append(&text[run], false);
                }
            }
        }
        visual
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.unidirectional
            .into_iter()
            .chain(self.runs.iter().map(Cow::as_ref))
    }

    fn append_rtl(&mut self, text: &'a str) {
        let shapes = |ch| is_arabic(ch) || is_join_control(ch);
        let mut end = text.len();
        let mut last: Option<bool> = None;
        for (offset, ch) in text.char_indices().rev() {
            let shaped = shapes(ch);
            if let Some(previous) = last {
                if previous != shaped {
                    let start = offset + ch.len_utf8();
                    self.append(&text[start..end], !previous);
                    end = start;
                }
            }
            last = Some(shaped);
        }
        if let Some(shaped) = last {
            self.append(&text[..end], !shaped);
        }
    }

    fn append(&mut self, text: &'a str, reverse: bool) {
        // Direction controls have already contributed to the ordering. Join
        // controls still need to reach the OpenType shaper.
        let strip = |ch| {
            matches!(
                ch,
                '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
        };
        let run = if reverse {
            Cow::Owned(
                text.chars()
                    .rev()
                    .filter(|&ch| !strip(ch))
                    .map(|ch| unicode_bidi_mirroring::get_mirrored(ch).unwrap_or(ch))
                    .collect(),
            )
        } else if text.chars().any(strip) {
            Cow::Owned(text.chars().filter(|&ch| !strip(ch)).collect())
        } else {
            Cow::Borrowed(text)
        };
        if !run.is_empty() {
            self.runs.push(run);
        }
    }
}

/// A leading strong letter keeps its following marks and join controls in
/// the same bidi run. Digits, punctuation and direction controls need UAX #9.
fn is_single_arabic_span(text: &str) -> bool {
    let arabic_glyph = |ch| ch != '\u{061c}' && is_arabic(ch);
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|ch| arabic_glyph(ch) && bidi_class(ch) == BidiClass::AL)
        && chars.all(|ch| {
            matches!(ch, '\u{200c}' | '\u{200d}')
                || (arabic_glyph(ch) && matches!(bidi_class(ch), BidiClass::AL | BidiClass::NSM))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_arabic_word_borrows_the_complete_source_span() {
        for text in ["سلام", "لَا", "س\u{200c}لام"] {
            let visual = VisualText::new(text);
            assert_eq!(visual.unidirectional, Some(text));
            assert!(visual.runs.is_empty());
        }
        for text in ["س١", "س\u{061c}", "س ل", "\u{0651}س", "\u{200c}س"] {
            assert!(VisualText::new(text).unidirectional.is_none());
        }
    }

    #[test]
    fn mixed_canvas_line_orders_words_and_digits_but_retains_joining_sources() {
        let order = |text| VisualText::new(text).iter().collect::<String>();
        assert_eq!(order("abc عربي 123"), "abc 123 عربي");
        assert_eq!(order("abc عربي ١٢٣"), "abc ١٢٣ عربي");
        assert_eq!(order("س\u{200c}لام"), "س\u{200c}لام");
        assert_eq!(order("abc عربي (123)"), "abc (123) عربي");
        assert_eq!(
            VisualText::new("عربي١٢٣").iter().collect::<Vec<_>>(),
            ["١٢٣", "عربي"],
        );
    }
}
