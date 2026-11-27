//! Visual text order for direct paint operations without layout positions.

use std::borrow::Cow;
use unicode_bidi::{BidiInfo, Level};

use crate::{is_arabic, is_join_control};

/// Visual runs for a left-to-right canvas. Each Arabic run retains its
/// logical characters so OpenType can select joining forms.
pub struct VisualText<'a> {
    unidirectional: Option<&'a str>,
    runs: Vec<Cow<'a, str>>,
}

impl<'a> VisualText<'a> {
    pub fn new(text: &'a str) -> Self {
        if text.is_ascii() || !text.chars().any(is_arabic) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
