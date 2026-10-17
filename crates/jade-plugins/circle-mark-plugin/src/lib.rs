//! Sample jade decoration plugin.
//!
//! Built as a standalone `cdylib` targeting `wasm32-unknown-unknown`. It
//! exports the symbol set described in
//! [`jade_preview_plugin_api`][api] and is intended to be loaded
//! dynamically from JavaScript by a host (the jade editor).
//!
//! For parity with the built-in Rust-native `CircleMarkPlugin` that ships
//! inside the host, this plugin detects U+25EF ("Large Circle", `◯`) and
//! emits a red underline decoration for each occurrence.
//!
//! Build:
//!
//! ```bash
//! cargo build --release -p circle-mark-plugin --target wasm32-unknown-unknown
//! # => target/wasm32-unknown-unknown/release/circle_mark_plugin.wasm
//! ```
//!
//! [api]: ../jade_preview_plugin_api/index.html

use jade_preview_plugin_api::{DecoratedRange, DecorationStyle, export_plugin};

/// The character this plugin reacts to.
///
/// Not `U+25CB` (○, "WHITE CIRCLE") — that is a separate codepoint.
const CIRCLE_CHAR: char = '◯';

/// Underline color used for every match. Kept as a function (rather than a
/// `&'static str`) so that a future variant of this plugin could read it
/// from configuration passed through the ABI without changing the matcher
/// signature.
fn underline_color() -> String {
    "#FF0000".to_string()
}

/// Pure matcher: returns the byte range of every `◯` occurrence in `text`,
/// each tagged with a red underline style.
///
/// Split out from the exported symbol so it can be unit-tested on the host
/// toolchain (i.e. natively) without needing a WASM runtime.
pub fn decorate(text: &str) -> Vec<DecoratedRange> {
    let mut ranges = Vec::new();
    for (byte_index, ch) in text.char_indices() {
        if ch == CIRCLE_CHAR {
            ranges.push(DecoratedRange {
                start: byte_index,
                end: byte_index + ch.len_utf8(),
                style: DecorationStyle::Underline {
                    color: underline_color(),
                    thickness_px: None,
                },
            });
        }
    }
    ranges
}

// Generates `plugin_alloc`, `plugin_free`, `plugin_decorate`, and
// `plugin_abi_version` exports from `decorate` above.
export_plugin!(decorate);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_single_circle_at_start() {
        let ranges = decorate("◯abc");
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 3);
    }

    #[test]
    fn matches_multiple_circles_among_cjk() {
        let text = "あ◯い◯う";
        let ranges = decorate(text);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&text[ranges[0].start..ranges[0].end], "◯");
        assert_eq!(&text[ranges[1].start..ranges[1].end], "◯");
    }

    #[test]
    fn ignores_white_circle() {
        // U+25CB (○) is NOT U+25EF (◯) — the plugin must not confuse them.
        assert!(decorate("○ not matched").is_empty());
    }

    #[test]
    fn empty_input_yields_no_decorations() {
        assert!(decorate("").is_empty());
    }
}
