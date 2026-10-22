//! What a screen reader is given of a document: its text, a paragraph a
//! line, with the caret and the selection in it as UTF-16 offsets (what
//! macOS's accessibility API and the browser's DOM count in), and the
//! paragraph the caret is in, for announcing as the caret moves. The shells
//! hand it to the system: an accessibility element over the pages in the
//! native app, an offscreen ARIA text box in the browser.

use super::{Helpers, WordSession};

/// The longest text handed over, in UTF-16 units: a screen reader reads a
/// window of the document, not a book at once.
pub const MAX_UNITS: usize = 200_000;

/// A document as a screen reader is given it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenReaderText {
    pub text: String,
    /// The selection, start and end, in UTF-16 units of `text`; equal for
    /// a caret.
    pub selection: (usize, usize),
    /// The caret's paragraph, by index, and its text.
    pub paragraph: usize,
    pub paragraph_text: String,
}

/// The text and where the selection is in it.
pub fn screen_reader_text<S: WordSession + ?Sized>(session: &S) -> ScreenReaderText {
    let (start, end) = session
        .selection_range()
        .map(|range| {
            let (start, end) = range.normalized();
            ((start.paragraph, start.offset), (end.paragraph, end.offset))
        })
        .unwrap_or(((0, 0), (0, 0)));
    let mut out = ScreenReaderText {
        paragraph: end.0,
        ..ScreenReaderText::default()
    };
    let mut units = 0usize;
    for index in 0..session.paragraph_total() {
        let text = session.text_of(index).unwrap_or_default();
        let at = |offset: usize| {
            let offset = offset.min(text.len());
            let mut offset = offset;
            while !text.is_char_boundary(offset) {
                offset -= 1;
            }
            units + text[..offset].encode_utf16().count()
        };
        if index == start.0 {
            out.selection.0 = at(start.1);
        }
        if index == end.0 {
            out.selection.1 = at(end.1);
            out.paragraph_text = text.clone();
        }
        let length = text.encode_utf16().count();
        if units + length + 1 > MAX_UNITS {
            break;
        }
        if index > 0 {
            out.text.push('\n');
            units += 1;
            if index == start.0 {
                out.selection.0 += 1;
            }
            if index == end.0 {
                out.selection.1 += 1;
            }
        }
        out.text.push_str(&text);
        units += length;
    }
    out
}
