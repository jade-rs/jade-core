//! Expand at word separators and block-script boundaries without splitting clusters.
use jade_text_layout::{forbids_line_end, forbids_line_start};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

type Segment = (usize, String, Vec<usize>);
fn block(value: &str) -> bool {
    value.chars().next().is_some_and(|c| {
        matches!(
            c.script(),
            Script::Han
                | Script::Hiragana
                | Script::Katakana
                | Script::Hangul
                | Script::Bopomofo
                | Script::Yi
        )
    })
}
fn spaced_letters(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|c| matches!(c.script(), Script::Latin | Script::Greek | Script::Cyrillic))
}
fn separator(value: &str) -> bool {
    matches!(value, " " | "\u{3000}")
}
fn slot(left: &str, right: &str, distributed: bool) -> bool {
    if separator(left) {
        return !right.chars().all(char::is_whitespace);
    }
    if left.chars().any(char::is_whitespace)
        || right.chars().any(char::is_whitespace)
        || forbids_line_end(left)
        || forbids_line_start(right)
    {
        return false;
    }
    block(left) || block(right) || distributed && (spaced_letters(left) || spaced_letters(right))
}
/// Style/run boundaries alone do not create expansion opportunities.
/// Returned flags identify the extra advance following each output segment.
pub(crate) fn split(segments: Vec<Segment>, distributed: bool) -> (Vec<Segment>, Vec<bool>) {
    let graphemes: Vec<_> = segments
        .iter()
        .flat_map(|(_, text, _)| text.graphemes(true))
        .collect();
    let slots: Vec<_> = graphemes
        .windows(2)
        .map(|pair| slot(pair[0], pair[1], distributed))
        .chain(std::iter::once(false))
        .collect();
    let mut output = Vec::new();
    let mut expansion = Vec::new();
    let mut at = 0;
    for (run, text, offsets) in &segments {
        let mut start_byte = 0;
        let mut start_grapheme = 0;
        let parts: Vec<_> = text.grapheme_indices(true).collect();
        for (index, (byte, g)) in parts.iter().enumerate() {
            let last = index + 1 == parts.len();
            // A capitals expansion still belongs to one source cluster.
            let expand = slots[at] && offsets[index + 1] > offsets[index];
            if expand || last {
                output.push((
                    *run,
                    text[start_byte..byte + g.len()].to_owned(),
                    offsets[start_grapheme..=index + 1].to_vec(),
                ));
                expansion.push(expand);
                start_byte = byte + g.len();
                start_grapheme = index + 1;
            }
            at += 1;
        }
    }
    (output, expansion)
}
