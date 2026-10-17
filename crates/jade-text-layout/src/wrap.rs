//! A greedy line filler.
//!
//! Greedy rather than optimal on purpose: it is what a spreadsheet cell and a
//! slide text box do, and what a reader expects when they widen a column and
//! watch the last word move up.

use unicode_segmentation::UnicodeSegmentation;

use crate::kinsoku;

#[derive(Debug, Clone, Copy)]
pub struct WrapOptions {
    /// Whether a run of Latin letters may be split when it does not fit on its
    /// own. A word longer than the line has to break somewhere either way; this
    /// decides whether a word that would fit on the *next* line moves whole.
    pub break_inside_words: bool,
    /// Whether the East Asian start/end prohibitions apply.
    pub kinsoku: bool,
}

impl Default for WrapOptions {
    fn default() -> Self {
        Self {
            break_inside_words: false,
            kinsoku: true,
        }
    }
}

/// Breaks `text` into lines no wider than `max_width`.
///
/// `measure` is called per grapheme and the results are summed, which is the
/// same shape as the advance-width measurement the renderers already do. It
/// ignores kerning across a pair, as canvas per-glyph measurement does.
///
/// An explicit newline always breaks. A line is never empty unless the text
/// asked for it, so a `max_width` too small for even one grapheme still makes
/// progress rather than looping.
pub fn wrap_text(
    text: &str,
    max_width: f64,
    measure: &mut dyn FnMut(&str) -> f64,
    options: WrapOptions,
) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(paragraph, max_width, measure, options, &mut lines);
    }
    lines
}

fn wrap_paragraph(
    text: &str,
    max_width: f64,
    measure: &mut dyn FnMut(&str) -> f64,
    options: WrapOptions,
    lines: &mut Vec<String>,
) {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    if graphemes.is_empty() {
        lines.push(String::new());
        return;
    }

    let widths: Vec<f64> = graphemes.iter().map(|grapheme| measure(grapheme)).collect();

    let mut start = 0usize;
    while start < graphemes.len() {
        let mut width = 0.0;
        let mut end = start;
        while end < graphemes.len() {
            let next = width + widths[end];
            // The first grapheme always goes on the line: a column narrower
            // than one character still has to show something.
            if next > max_width && end > start {
                break;
            }
            width = next;
            end += 1;
        }

        if end < graphemes.len() {
            end = adjust_break(&graphemes, start, end, options);
        }
        lines.push(graphemes[start..end].concat());
        start = end;

        // A break at a space swallows the space rather than starting the next
        // line with it, which is what every renderer does with a soft wrap.
        while start < graphemes.len() && graphemes[start] == " " {
            start += 1;
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }
}

/// Moves a candidate break earlier until it is allowed.
///
/// `end` is the first grapheme that did not fit. Three things can move it:
/// a character that may not start a line, one that may not end a line, and a
/// Latin word that should move to the next line whole.
fn adjust_break(graphemes: &[&str], start: usize, end: usize, options: WrapOptions) -> usize {
    let mut candidate = end;

    if options.kinsoku {
        // Walk back over a run of prohibitions rather than one: "」。" at a
        // break needs both to move.
        while candidate > start + 1 && breaks_a_prohibition(graphemes, candidate) {
            candidate -= 1;
        }
    }

    if !options.break_inside_words
        && candidate > start
        && is_word_grapheme(graphemes[candidate.min(end)])
        && is_word_grapheme(graphemes[candidate - 1])
    {
        // Inside a Latin word: move the break to the word's start, which is
        // the space before it or — in mixed text — the East Asian character
        // before it, where a break is allowed anyway. A word that itself
        // starts the line has nowhere better to go and splits.
        let mut word_start = candidate;
        while word_start > start && is_word_grapheme(graphemes[word_start - 1]) {
            word_start -= 1;
        }
        if word_start > start {
            candidate = word_start;
            if options.kinsoku {
                // The character now ending the line may be one that must not
                // (an opening bracket before the word).
                while candidate > start + 1 && breaks_a_prohibition(graphemes, candidate) {
                    candidate -= 1;
                }
            }
        }
    }

    // A break right after a space would leave the space dangling at the
    // line's end, where it widens a right-aligned line for nothing.
    while candidate > start + 1 && graphemes[candidate - 1] == " " {
        candidate -= 1;
    }

    candidate
}

/// Whether breaking before `index` leaves a prohibited character on either
/// side of the break, as a reader would see it.
///
/// Not `graphemes[index]` and `graphemes[index - 1]`. A soft wrap swallows the
/// spaces at the break, so the first grapheme on the next line is the first
/// non-space from `index` on, and the last one on the line it ends is the last
/// non-space before it. Asking about the neighbours directly let a break land
/// on the space in "横浜 、市" and report itself satisfied — a space is
/// prohibited at neither end — and the "、" the walk was moving away started
/// the next line regardless.
fn breaks_a_prohibition(graphemes: &[&str], index: usize) -> bool {
    let visible = |grapheme: &&&str| **grapheme != " ";
    let next_line_starts_with = graphemes[index..].iter().find(visible);
    let this_line_ends_with = graphemes[..index].iter().rev().find(visible);
    next_line_starts_with.is_some_and(|grapheme| kinsoku::forbids_line_start(grapheme))
        || this_line_ends_with.is_some_and(|grapheme| kinsoku::forbids_line_end(grapheme))
}

/// Whether the grapheme is part of a word that should not be split.
///
/// Latin letters, accented Latin, and digits. East Asian text is deliberately
/// excluded: it breaks anywhere the prohibitions allow, which is the whole
/// reason those prohibitions exist.
fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '\u{00C0}'..='\u{024F}'))
}
