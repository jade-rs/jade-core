//! Properties every wrap must satisfy, checked over generated text.
//!
//! `wraps_lines.rs` pins the line ends of hand-picked strings; this file states
//! the rules those examples are instances of and lets proptest look for a
//! string that breaks one. The two are complementary: an example says what the
//! output is, a property says what it may never be.

use jade_text_layout::{forbids_line_end, forbids_line_start, wrap_text, WrapOptions};
use proptest::prelude::*;

/// One unit per ASCII character, two per anything wider — the measurement
/// `wraps_lines.rs` uses, so a failure there and a failure here are comparable.
fn monospace(text: &str) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 1.0 } else { 2.0 })
        .sum()
}

/// Latin letters, a space and a newline to drive the word and paragraph paths,
/// and the East Asian punctuation the prohibitions are about.
const ALPHABET: &[&str] = &[
    "a", "b", " ", "\n", "横", "浜", "、", "。", "」", "「", "（", "）", "ー", "っ",
];

/// [`ALPHABET`] without the newline.
const PARAGRAPH_ALPHABET: &[&str] = &[
    "a", "b", " ", "横", "浜", "、", "。", "」", "「", "（", "）", "ー", "っ",
];

fn text_over(alphabet: &'static [&'static str]) -> impl Strategy<Value = String> {
    proptest::collection::vec(0..alphabet.len(), 0..12)
        .prop_map(|indices| indices.into_iter().map(|i| alphabet[i]).collect())
}

fn text() -> impl Strategy<Value = String> {
    text_over(ALPHABET)
}

/// Text with no newline in it.
///
/// The prohibitions are about breaks the wrapper chose. A newline is a break
/// the author chose, and a paragraph may legitimately end with an opening
/// bracket or begin with a full stop; `"aa\n、"` is not a wrap that went wrong.
/// Properties about where a break may land therefore hold within a paragraph,
/// which is what this generates.
fn paragraph() -> impl Strategy<Value = String> {
    text_over(&ALPHABET[..3]).prop_flat_map(|_| text_over(PARAGRAPH_ALPHABET))
}

fn wrap(text: &str, max_width: f64) -> Vec<String> {
    wrap_text(text, max_width, &mut monospace, WrapOptions::default())
}

/// The graphemes of a line, as the wrapper counts them.
fn graphemes(line: &str) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    line.graphemes(true).map(str::to_string).collect()
}

/// The graphemes a reader can see. A space at either end of a line renders as
/// nothing, so it is not the character the prohibitions are about.
fn visible(line: &str) -> Vec<String> {
    let cells = graphemes(line);
    let first = cells.iter().position(|cell| cell != " ");
    match first {
        Some(first) => {
            let last = cells.iter().rposition(|cell| cell != " ").unwrap_or(first);
            cells[first..=last].to_vec()
        }
        None => Vec::new(),
    }
}

proptest! {
    /// A line may only be wider than the column when it holds a single
    /// grapheme, which a column narrower than one character forces.
    #[test]
    fn a_line_overflows_only_when_it_is_a_single_grapheme(text in text(), width in 1usize..=10) {
        let max_width = width as f64;
        for line in wrap(&text, max_width) {
            let count = graphemes(&line).len();
            prop_assert!(
                monospace(&line) <= max_width || count <= 1,
                "line {line:?} ({} units, {count} graphemes) overflows a {max_width}-unit column",
                monospace(&line),
            );
        }
    }

    /// Wrapping decides where lines end, not what they say. Soft wraps swallow
    /// the space they break at and paragraph breaks consume their newline, so
    /// those are the only characters allowed to disappear.
    #[test]
    fn wrapping_changes_nothing_but_the_spaces_it_breaks_at(text in text(), width in 1usize..=10) {
        let joined: String = wrap(&text, width as f64).concat();
        let kept: String = joined.chars().filter(|c| *c != ' ').collect();
        let original: String = text.chars().filter(|c| *c != ' ' && *c != '\n').collect();
        prop_assert_eq!(kept, original);
    }

    /// A character that may not start a line only starts one when the wrapper
    /// had nowhere else to put it.
    ///
    /// "Nowhere else" is not a figure of speech: `adjust_break` may walk a
    /// break back until a single grapheme is left on the line before, and no
    /// further — a line has to hold something. So a prohibited character at a
    /// line start is forced exactly when the line before it holds one
    /// grapheme, or when there is no line before it at all.
    #[test]
    fn a_prohibited_character_starts_a_line_only_when_it_is_forced(
        text in paragraph(),
        width in 1usize..=10,
    ) {
        let lines = wrap(&text, width as f64);
        for (index, line) in lines.iter().enumerate().skip(1) {
            let Some(first) = visible(line).into_iter().next() else {
                continue;
            };
            if !forbids_line_start(&first) {
                continue;
            }
            let previous = &lines[index - 1];
            prop_assert!(
                visible(previous).len() <= 1,
                "line {line:?} starts with {first:?}, but the break could have \
                 moved back into {previous:?}",
            );
        }
    }

    /// A character that may not end a line only ends one when the line holds
    /// nothing else, or when nothing follows it.
    #[test]
    fn a_prohibited_character_ends_a_line_only_when_it_is_forced(
        text in paragraph(),
        width in 1usize..=10,
    ) {
        let lines = wrap(&text, width as f64);
        for line in lines.iter().take(lines.len().saturating_sub(1)) {
            let mut cells = visible(line);
            let Some(last) = cells.pop() else { continue };
            if !forbids_line_end(&last) {
                continue;
            }
            prop_assert!(
                cells.is_empty(),
                "line {line:?} ends with {last:?}, but the break could have \
                 moved back",
            );
        }
    }

    /// A wider column never needs more lines.
    #[test]
    fn a_wider_column_never_needs_more_lines(text in text(), width in 1usize..=9) {
        let narrow = wrap(&text, width as f64).len();
        let wide = wrap(&text, width as f64 + 1.0).len();
        prop_assert!(
            wide <= narrow,
            "{text:?}: {width} units needs {narrow} lines but {} units needs {wide}",
            width + 1,
        );
    }
}
