//! Extended grapheme clusters, with a fast path for text where every
//! character is a cluster of its own.
//!
//! Line wrapping and measuring walk every run's clusters, and
//! `unicode-segmentation`'s state machine was a large share of laying out a
//! long Japanese document. Most such text holds only characters that never
//! join a neighbour into one cluster (UAX #29: no Extend, ZWJ, SpacingMark,
//! Prepend, Regional Indicator or Hangul syllable, and no CR to pair with an
//! LF), and there clusters are characters.

use std::str::CharIndices;
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};

/// Whether `ch` always forms a cluster on its own, whatever surrounds it.
/// Conservative: a character left out only costs the slow path.
fn stands_alone(ch: char) -> bool {
    matches!(
        ch as u32,
        // Controls break on both sides (GB4, GB5) except CR before LF.
        0x00..=0x0C | 0x0E..=0x1F
            | 0x20..=0x7E
            // Latin-1, less the soft hyphen (a control).
            | 0xA0..=0xAC
            | 0xAE..=0xFF
            // General punctuation, less zero-width and format characters.
            | 0x2010..=0x2027
            | 0x2030..=0x205E
            // CJK symbols and punctuation, less the combining tone marks.
            | 0x3000..=0x3029
            | 0x3030..=0x303F
            // Hiragana, less the combining voiced sound marks.
            | 0x3041..=0x3096
            | 0x309B..=0x309F
            | 0x30A0..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            // Full-width forms and half-width katakana, less the half-width
            // voiced sound marks (Extend).
            | 0xFF01..=0xFF9D
    )
}

/// `text`'s extended grapheme clusters with their byte offsets, as
/// `text.grapheme_indices(true)` yields them.
pub fn grapheme_indices(text: &str) -> Clusters<'_> {
    if text.chars().all(stands_alone) {
        Clusters::Chars(text, text.char_indices())
    } else {
        Clusters::Segmented(text.grapheme_indices(true))
    }
}

pub enum Clusters<'a> {
    Chars(&'a str, CharIndices<'a>),
    Segmented(GraphemeIndices<'a>),
}

impl<'a> Iterator for Clusters<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Clusters::Chars(text, chars) => chars
                .next()
                .map(|(index, ch)| (index, &text[index..index + ch.len_utf8()])),
            Clusters::Segmented(graphemes) => graphemes.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Clusters::Chars(_, chars) => chars.size_hint(),
            Clusters::Segmented(graphemes) => graphemes.size_hint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn standalone_char() -> impl Strategy<Value = char> {
        prop_oneof![
            (0x00u32..=0x7Fu32),
            (0xA0u32..=0xFFu32),
            (0x2000u32..=0x206Fu32),
            (0x3000u32..=0x30FFu32),
            (0x4E00u32..=0x4E40u32),
            (0xFF00u32..=0xFFA0u32),
        ]
        .prop_filter_map("assigned", char::from_u32)
    }

    proptest! {
        /// The fast path yields what `unicode-segmentation` does, for text it
        /// takes and text it hands back.
        #[test]
        fn clusters_match_unicode_segmentation(text in proptest::collection::vec(standalone_char(), 0..40)) {
            let text: String = text.into_iter().collect();
            let fast: Vec<_> = grapheme_indices(&text).collect();
            let full: Vec<_> = text.grapheme_indices(true).collect();
            prop_assert_eq!(fast, full);
        }
    }

    #[test]
    fn joining_characters_take_the_slow_path() {
        for text in ["か\u{3099}", "a\u{301}", "\r\n", "ｶﾞ", "👍🏽", "e\u{200D}"] {
            assert!(
                matches!(grapheme_indices(text), Clusters::Segmented(_)),
                "{text:?}"
            );
        }
        assert!(matches!(
            grapheme_indices("漢字かな、ABC\t"),
            Clusters::Chars(..)
        ));
    }
}
