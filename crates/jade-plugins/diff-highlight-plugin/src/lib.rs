//! Shades the parts of a paragraph that a document comparison reported as
//! changed.
//!
//! The plugin knows nothing about comparing documents. Whatever ran the
//! comparison hands it the answer through `plugin_configure`, and this
//! crate's whole job is to look up a paragraph's text and return the ranges
//! to shade.
//!
//! ## Why the configuration is keyed on text
//!
//! The host caches decoration results per paragraph *text* and shares one
//! entry between two paragraphs with the same body, so the plugin is only
//! ever asked "what should this string look like" — never "what should
//! paragraph 12 look like". The configuration follows the same shape.
//!
//! The consequence is worth stating plainly: two paragraphs with byte-identical
//! text get identical decoration, even if the comparison only flagged one of
//! them. In a document where that matters, the shading is too generous rather
//! than wrong — nothing changed is ever left unshaded.
//!
//! ## Configuration format
//!
//! ```json
//! {
//!   "paragraphs": [
//!     {
//!       "text": "第3条（委託料） 委託料は月額 550,000 円とする。",
//!       "ranges": [{ "start": 30, "end": 31, "change": "insert" }]
//!     }
//!   ],
//!   "colors": { "insert": "rgba(34,197,94,0.28)", "delete": "rgba(239,68,68,0.28)" }
//! }
//! ```
//!
//! `start` / `end` are byte offsets into `text` and must land on UTF-8
//! character boundaries — the host drops ranges that do not.

use std::cell::RefCell;
use std::collections::HashMap;

use jade_preview_plugin_api::{DecoratedRange, DecorationStyle, export_plugin};
use serde::Deserialize;

/// One shaded span within a paragraph.
#[derive(Debug, Clone, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub change: Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Text present only in the newer document.
    #[default]
    Insert,
    /// Text present only in the older document.
    Delete,
    /// Unchanged text that moved somewhere else.
    Move,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paragraph {
    pub text: String,
    #[serde(default)]
    pub ranges: Vec<Span>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Colors {
    #[serde(default = "default_insert")]
    pub insert: String,
    #[serde(default = "default_delete")]
    pub delete: String,
    #[serde(default = "default_move")]
    pub r#move: String,
}

fn default_insert() -> String {
    "rgba(34,197,94,0.28)".to_string()
}

fn default_delete() -> String {
    "rgba(239,68,68,0.28)".to_string()
}

fn default_move() -> String {
    "rgba(56,189,248,0.28)".to_string()
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            insert: default_insert(),
            delete: default_delete(),
            r#move: default_move(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub paragraphs: Vec<Paragraph>,
    #[serde(default)]
    pub colors: Colors,
}

/// What the plugin actually consults at draw time.
///
/// The host asks about one paragraph at a time and does so on the render
/// path, so the lookup has to be a hash of the text rather than a scan.
#[derive(Default)]
struct State {
    by_text: HashMap<String, Vec<Span>>,
    colors: Colors,
}

thread_local! {
    // The guest is single-threaded (wasm32-unknown-unknown), so a
    // thread-local is the whole story here.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Accepts a configuration blob from the host.
///
/// A blob that does not parse is rejected and the previous configuration
/// stays in place. Half-applying a comparison would shade the wrong text,
/// which is worse than shading nothing.
pub fn configure(payload: &str) -> bool {
    let Ok(config) = serde_json::from_str::<Config>(payload) else {
        return false;
    };

    let mut by_text: HashMap<String, Vec<Span>> = HashMap::new();
    for paragraph in config.paragraphs {
        if paragraph.ranges.is_empty() {
            continue;
        }
        // The same text can arrive more than once (identical paragraphs in
        // different places). Merge rather than overwrite so nothing is lost.
        by_text
            .entry(paragraph.text)
            .or_default()
            .extend(paragraph.ranges);
    }

    STATE.with(|state| {
        *state.borrow_mut() = State {
            by_text,
            colors: config.colors,
        };
    });
    true
}

/// Returns the ranges to shade within `text`.
pub fn decorate(text: &str) -> Vec<DecoratedRange> {
    STATE.with(|state| {
        let state = state.borrow();
        let Some(spans) = state.by_text.get(text) else {
            return Vec::new();
        };
        spans
            .iter()
            .filter(|span| span.start < span.end && span.end <= text.len())
            .map(|span| DecoratedRange {
                start: span.start,
                end: span.end,
                style: DecorationStyle::Highlight {
                    color: match span.change {
                        Change::Insert => state.colors.insert.clone(),
                        Change::Delete => state.colors.delete.clone(),
                        Change::Move => state.colors.r#move.clone(),
                    },
                },
            })
            .collect()
    })
}

export_plugin!(decorate, configure);

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_of(range: &DecoratedRange) -> &str {
        match &range.style {
            DecorationStyle::Highlight { color } => color,
            other => panic!("highlight のはず: {other:?}"),
        }
    }

    #[test]
    fn shades_the_configured_range() {
        assert!(configure(
            r#"{"paragraphs":[{"text":"委託料は月額 550,000 円","ranges":[{"start":21,"end":22,"change":"insert"}]}]}"#
        ));

        let ranges = decorate("委託料は月額 550,000 円");
        assert_eq!(ranges.len(), 1);
        assert_eq!((ranges[0].start, ranges[0].end), (21, 22));
        assert_eq!(colour_of(&ranges[0]), default_insert());
    }

    #[test]
    fn leaves_unconfigured_text_alone() {
        assert!(configure(r#"{"paragraphs":[]}"#));
        assert!(decorate("何も設定されていない段落").is_empty());
    }

    #[test]
    fn deletions_and_moves_get_their_own_colour() {
        assert!(configure(
            r#"{"paragraphs":[{"text":"abcdef","ranges":[
                 {"start":0,"end":2,"change":"delete"},
                 {"start":3,"end":5,"change":"move"}]}]}"#
        ));

        let ranges = decorate("abcdef");
        assert_eq!(ranges.len(), 2);
        assert_eq!(colour_of(&ranges[0]), default_delete());
        assert_eq!(colour_of(&ranges[1]), default_move());
    }

    #[test]
    fn colours_can_be_overridden() {
        // 色の指定に "#" が入るので、生文字列の区切りを長くしてある。
        assert!(configure(
            r##"{"paragraphs":[{"text":"abc","ranges":[{"start":0,"end":1}]}],
                 "colors":{"insert":"#123456"}}"##
        ));
        assert_eq!(colour_of(&decorate("abc")[0]), "#123456");
    }

    #[test]
    fn a_range_past_the_end_of_the_text_is_dropped() {
        // 設定と実際の本文が食い違うことはある (別の版を渡された場合など)。
        // ホストも弾くが、こちらでも出さない。
        assert!(configure(
            r#"{"paragraphs":[{"text":"abc","ranges":[{"start":0,"end":99}]}]}"#
        ));
        assert!(decorate("abc").is_empty());
    }

    #[test]
    fn identical_paragraphs_merge_rather_than_overwrite() {
        assert!(configure(
            r#"{"paragraphs":[
                 {"text":"同じ本文","ranges":[{"start":0,"end":3}]},
                 {"text":"同じ本文","ranges":[{"start":6,"end":9}]}]}"#
        ));
        assert_eq!(decorate("同じ本文").len(), 2);
    }

    #[test]
    fn a_blob_that_does_not_parse_keeps_the_previous_configuration() {
        assert!(configure(
            r#"{"paragraphs":[{"text":"abc","ranges":[{"start":0,"end":1}]}]}"#
        ));
        assert!(!configure("{ これは JSON ではない"));
        assert_eq!(decorate("abc").len(), 1, "前の設定が残っていること");
    }
}
