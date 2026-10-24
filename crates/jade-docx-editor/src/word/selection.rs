//! Ways of selecting Word has beyond a drag: the word a double-click takes
//! (a run of one script in Japanese, a word and the space after it in
//! English), the rectangle an ⌥-drag takes (矩形選択), several ranges at
//! once (⌘-drag), and where each range is drawn.
//!
//! Positions are a paragraph and a byte offset into its text; rectangles are
//! in the document's coordinates, as the caret's are.

use jade_docx_layout::selection::{LineLayout, SelectionManager};

use super::Position;

/// The kind of character a double-click selects a run of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharClass {
    /// 漢字, with 々 〆 〇 ヶ.
    Kanji,
    Hiragana,
    /// カタカナ, full and half width.
    Katakana,
    /// Letters and digits of an alphabet, full width included.
    Word,
    Space,
    /// Punctuation and symbols: each a word of its own.
    Other,
}

/// What `ch` is to a double-click.
pub fn char_class(ch: char) -> CharClass {
    let code = ch as u32;
    match code {
        0x3005 | 0x3006 | 0x3007 | 0x30F6 => CharClass::Kanji,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF => CharClass::Kanji,
        0x3041..=0x309F => CharClass::Hiragana,
        0x30A1..=0x30FA | 0x30FD..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => {
            CharClass::Katakana
        }
        _ if ch.is_whitespace() => CharClass::Space,
        _ if ch.is_alphanumeric() || ch == '_' => CharClass::Word,
        _ => CharClass::Other,
    }
}

/// The byte range a double-click at `offset` in `text` selects: the run of
/// characters of the class there — 東京 out of 東京へ行く — and, after an
/// alphabetic word, the spaces that follow it, as Word takes them.
/// ー (the long-vowel mark) goes with the kana before it.
pub fn word_at(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    // The character under the pointer: the one starting at the offset, or
    // the last one at the paragraph's end.
    let index = chars
        .iter()
        .position(|(at, _)| *at >= offset)
        .unwrap_or(chars.len() - 1);
    let class_of = |i: usize| -> CharClass {
        let ch = chars[i].1;
        if ch == 'ー' || ch == 'ｰ' {
            // The class of the kana it lengthens.
            let mut j = i;
            while j > 0 {
                j -= 1;
                let previous = chars[j].1;
                if previous != 'ー' && previous != 'ｰ' {
                    return match char_class(previous) {
                        CharClass::Hiragana => CharClass::Hiragana,
                        _ => CharClass::Katakana,
                    };
                }
            }
            return CharClass::Katakana;
        }
        char_class(ch)
    };
    let class = class_of(index);
    if class == CharClass::Other {
        let (at, ch) = chars[index];
        return (at, at + ch.len_utf8());
    }
    let mut first = index;
    while first > 0 && class_of(first - 1) == class {
        first -= 1;
    }
    let mut last = index;
    while last + 1 < chars.len() && class_of(last + 1) == class {
        last += 1;
    }
    if class == CharClass::Word {
        while last + 1 < chars.len() && chars[last + 1].1 == ' ' {
            last += 1;
        }
    }
    let end = chars[last].0 + chars[last].1.len_utf8();
    (chars[first].0, end)
}

/// The x of `offset` on `line`, in the document's coordinates.
pub fn x_on_line(line: &LineLayout, offset: usize) -> f64 {
    for fragment in line.fragments.iter() {
        if offset >= fragment.start && offset < fragment.end {
            return fragment.x + fragment.relative_x_for_start(offset);
        }
        if offset == fragment.end {
            return fragment.x + fragment.relative_x_for_end(offset);
        }
    }
    match line.logical_source_range() {
        Some((start, _)) if offset <= start => line.start_x,
        _ => line.cursor_end_x,
    }
}

/// A highlight rectangle: x, y (document), width and height.
pub type Rect = (f64, f64, f64, f64);

/// Where a caret at `position` would be drawn — x, top and height — from
/// the last layout, without moving the document's own caret: the mark a
/// drag-and-drop shows where the text will land.
pub fn caret_rect(manager: &SelectionManager, position: Position) -> Option<Rect> {
    let paragraph = manager.paragraphs().get(position.0)?;
    let line = paragraph
        .lines
        .iter()
        .find(|line| {
            line.logical_source_range()
                .is_some_and(|(start, end)| position.1 >= start && position.1 <= end)
        })
        .or_else(|| paragraph.lines.last())?;
    Some((
        x_on_line(line, position.1),
        line.absolute_top,
        1.5,
        line.height,
    ))
}

/// Where a range from `start` to `end` is drawn: a rectangle per line it
/// covers, as the selection's highlight is.
pub fn range_rects(manager: &SelectionManager, start: Position, end: Position) -> Vec<Rect> {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut rects = Vec::new();
    let paragraphs = manager.paragraphs();
    for index in start.0..=end.0.min(paragraphs.len().saturating_sub(1)) {
        let Some(paragraph) = paragraphs.get(index) else {
            continue;
        };
        let from = if index == start.0 { start.1 } else { 0 };
        let to = if index == end.0 {
            end.1
        } else {
            paragraph.text.len()
        };
        let runs_on = index < end.0;
        for line in &paragraph.lines {
            let Some((line_start, line_end)) = line.logical_source_range() else {
                // An empty line inside the range shows a sliver.
                if index > start.0 && index < end.0 {
                    rects.push((line.start_x, line.absolute_top, 6.0, line.height));
                }
                continue;
            };
            let a = from.max(line_start);
            let b = to.min(line_end);
            let last_line = line_end >= paragraph.text.len();
            if a > b || (a == b && !(runs_on && last_line && a == to)) {
                continue;
            }
            let x0 = x_on_line(line, a);
            let mut x1 = x_on_line(line, b);
            // Past the line's end into the next paragraph: the mark too.
            if runs_on && last_line {
                x1 = x1.max(line.cursor_end_x) + 6.0;
            }
            if x1 > x0 {
                rects.push((x0, line.absolute_top, x1 - x0, line.height));
            }
        }
    }
    rects
}

/// 矩形選択: the ranges an ⌥-drag from one point to another takes — on
/// every line the rectangle crosses, the text between its left and right
/// edges. `paragraphs` limits the search to the paragraphs that can be in
/// the rectangle (the body's, not a header's).
pub fn block_ranges(
    manager: &SelectionManager,
    from: (f64, f64),
    to: (f64, f64),
) -> Vec<(Position, Position)> {
    let (left, right) = (from.0.min(to.0), from.0.max(to.0));
    let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
    let mut ranges = Vec::new();
    for (index, paragraph) in manager.paragraphs().iter().enumerate() {
        for line in &paragraph.lines {
            let middle = line.absolute_top + line.height / 2.0;
            if middle < top || middle > bottom {
                continue;
            }
            let Some((line_start, line_end)) = line.logical_source_range() else {
                continue;
            };
            let offset_at = |x: f64| -> usize {
                // The first character boundary whose x is past `x`.
                let mut best = line_start;
                let mut offset = line_start;
                while offset <= line_end {
                    if x_on_line(line, offset) <= x + 0.5 {
                        best = offset;
                    } else {
                        break;
                    }
                    let Some(ch) = paragraph
                        .text
                        .get(offset..)
                        .and_then(|rest| rest.chars().next())
                    else {
                        break;
                    };
                    offset += ch.len_utf8();
                }
                best
            };
            let a = offset_at(left);
            let b = offset_at(right).max(a);
            if b > a {
                ranges.push(((index, a), (index, b)));
            }
        }
    }
    ranges
}

/// The text of several ranges, as copying them gives it: one after another,
/// a line each.
pub fn ranges_text(
    paragraph_text: impl Fn(usize) -> Option<String>,
    ranges: &[(Position, Position)],
) -> String {
    let mut out = Vec::new();
    for &(start, end) in ranges {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let mut piece = String::new();
        for index in start.0..=end.0 {
            let text = paragraph_text(index).unwrap_or_default();
            let from = if index == start.0 { start.1 } else { 0 };
            let to = if index == end.0 { end.1 } else { text.len() };
            if index > start.0 {
                piece.push('\n');
            }
            piece.push_str(
                text.get(from.min(text.len())..to.min(text.len()))
                    .unwrap_or(""),
            );
        }
        out.push(piece);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, at: &str) -> String {
        let offset = text.find(at).unwrap();
        let (start, end) = word_at(text, offset);
        text[start..end].to_string()
    }

    #[test]
    fn a_double_click_takes_a_run_of_one_script() {
        let text = "東京都へ行くテストデータ。";
        assert_eq!(word(text, "京"), "東京都");
        assert_eq!(word(text, "行"), "行");
        assert_eq!(word(text, "く"), "く");
        assert_eq!(word(text, "へ"), "へ");
        assert_eq!(word(text, "デ"), "テストデータ");
        assert_eq!(word(text, "。"), "。");
        assert_eq!(word("コーヒーを", "ヒ"), "コーヒー");
        assert_eq!(word("すきー", "き"), "すきー");
    }

    #[test]
    fn an_english_word_takes_the_space_after_it() {
        assert_eq!(word("one two, three", "tw"), "two");
        assert_eq!(word("one two three", "tw"), "two ");
        assert_eq!(word("ＡＢＣ漢字", "Ｂ"), "ＡＢＣ");
        assert_eq!(word("a,b", ","), ",");
        assert_eq!(word_at("", 0), (0, 0));
        // At the paragraph's end: the last word.
        assert_eq!(word_at("abc", 3), (0, 3));
    }

    #[test]
    fn several_ranges_copy_a_line_each() {
        let texts = ["first line".to_string(), "second line".to_string()];
        let text = |index: usize| texts.get(index).cloned();
        assert_eq!(
            ranges_text(text, &[((0, 0), (0, 5)), ((1, 7), (1, 11))]),
            "first\nline"
        );
        assert_eq!(ranges_text(text, &[((0, 6), (1, 6))]), "line\nsecond");
    }
}
