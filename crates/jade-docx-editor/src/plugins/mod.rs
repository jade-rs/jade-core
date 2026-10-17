//! Render-time decoration plugins.
//!
//! Mirrors the SlateJS "decoration" concept used in the in-house web WYSIWYG
//! editor (e.g. `lawgue-lint-v3`): a decoration is an ephemeral, per-range
//! visual overlay that is recomputed on every render pass and **not**
//! persisted in the document model.
//!
//! ## Architecture
//!
//! The registry is owned by [`CanvasRenderer`] and queried **once per
//! paragraph** during the draw pass. Each plugin receives the paragraph's
//! concatenated text (in visual segment order) and reports byte ranges
//! relative to that text. The renderer then uses
//! [`clip_decoration_to_segment`] to project each range onto the
//! individual segments it overlaps — a single decoration can span
//! multiple runs of different styles.
//!
//! Plugins are stateless with respect to the document and may be
//! registered/unregistered dynamically from the host embedding (e.g. the
//! jade-ide front-end) via the JS API exposed on
//! [`crate::canvas_view::OffscreenDocumentView`].
//!
//! ## Wire schema
//!
//! [`DecoratedRange`] and [`DecorationStyle`] are re-exported from the
//! shared [`jade_preview_plugin_api`] crate so that guest WASM plugins and
//! the host embed the same JSON schema (`#[serde(tag = "kind")]` for the
//! style variants). This makes it trivial for a [`JsCallbackPlugin`] to
//! hand its JSON output back to the renderer without a second translation
//! step.
//!
//! ## Defaults
//!
//! There are no built-in plugins. The host (e.g. jade-ide) is responsible
//! for registering any plugins it wants active. The previous Rust-native
//! `CircleMarkPlugin` has been removed in favor of the guest WASM sample
//! at `crates/jade-plugins/circle-mark-plugin`; embedders that want the same
//! behavior load that module at runtime.
//!
//! [`CanvasRenderer`]: crate::canvas_renderer::CanvasRenderer
//! [`JsCallbackPlugin`]: js_callback::JsCallbackPlugin

pub mod js_callback;
pub mod spelling;

pub use jade_preview_plugin_api::{DecoratedRange, DecorationStyle};
pub use js_callback::JsCallbackPlugin;

use std::collections::HashMap;

/// Paragraph-keyed cache of decoration ranges.
///
/// Keyed on the exact paragraph text passed to the plugins (not a hash —
/// text-itself lookup sidesteps the u64-in-JS marshalling problem and
/// avoids hash-collision defensive coding, at the cost of a bit more
/// memory for the keys). A `registry_version` counter is bumped every
/// time the active set of plugins changes, and older entries become
/// unreachable: [`get`] only returns entries whose stored version
/// matches the current one.
///
/// [`get`]: DecorationCache::get
#[derive(Default)]
pub struct DecorationCache {
    entries: HashMap<String, CachedDecorations>,
    /// Reverse index: paragraph text → paragraph indexes that render
    /// with that text. Used by the embedder's
    /// `applyDecorationResults` path to target
    /// `invalidate_paragraph_render_cache_range` only at paragraphs
    /// that actually need a re-paint after new decorations arrive,
    /// instead of clearing the whole render cache.
    ///
    /// Stale entries (paragraphs whose text has since changed) are
    /// left in place — invalidating a paragraph that no longer
    /// contains the matching text is wasteful but safe, and bounded
    /// by the number of unique paragraph texts seen over the document
    /// lifetime. Callers that want to reset the reverse index use
    /// [`clear`](Self::clear).
    paragraph_indexes_by_text: HashMap<String, Vec<usize>>,
    registry_version: u64,
}

struct CachedDecorations {
    version: u64,
    ranges: Vec<DecoratedRange>,
}

impl DecorationCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn registry_version(&self) -> u64 {
        self.registry_version
    }

    /// Invalidates every cached entry by bumping the version counter. The
    /// old entries are left in the map (they will be overwritten on next
    /// `put` for the same key, or evicted on explicit `clear`). Done this
    /// way to avoid a full HashMap rebuild for the common case where
    /// plugins are tweaked and most paragraph texts are still the same.
    pub fn bump_version(&mut self) {
        self.registry_version = self.registry_version.wrapping_add(1);
    }

    pub fn get(&self, text: &str) -> Option<&[DecoratedRange]> {
        let entry = self.entries.get(text)?;
        if entry.version != self.registry_version {
            return None;
        }
        Some(&entry.ranges)
    }

    /// Inserts ranges for a paragraph text at the current registry
    /// version. Returns `true` iff the new entry would change what the
    /// renderer draws the next time this text is rendered.
    ///
    /// Critically, inserting an empty range vector when there was no
    /// prior entry reports `false`: the renderer's cache-miss path
    /// already draws with zero decorations, so caching that result
    /// verbatim is not a visible change and must not trigger a
    /// re-paint.
    pub fn put(&mut self, text: String, ranges: Vec<DecoratedRange>) -> bool {
        let version = self.registry_version;
        let is_change = match self.entries.get(&text) {
            Some(prev) => prev.version != version || prev.ranges != ranges,
            None => !ranges.is_empty(),
        };
        self.entries
            .insert(text, CachedDecorations { version, ranges });
        is_change
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.paragraph_indexes_by_text.clear();
    }

    /// Records that `paragraph_index` currently renders with `text`.
    /// Called once per paragraph per render pass so
    /// [`paragraph_indexes_for`] can later answer "which paragraphs
    /// should we invalidate now that decorations for this text
    /// arrived?". Duplicates (same index already recorded for this
    /// text) are silently dropped.
    ///
    /// [`paragraph_indexes_for`]: Self::paragraph_indexes_for
    pub fn record_paragraph_index(&mut self, text: &str, paragraph_index: usize) {
        let entry = self
            .paragraph_indexes_by_text
            .entry(text.to_string())
            .or_default();
        if !entry.contains(&paragraph_index) {
            entry.push(paragraph_index);
        }
    }

    /// Returns the paragraph indexes currently known to render with
    /// the supplied text. Empty slice if we have never rendered a
    /// paragraph with that text (or the cache was cleared).
    pub fn paragraph_indexes_for(&self, text: &str) -> &[usize] {
        match self.paragraph_indexes_by_text.get(text) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }
}

/// Set of paragraph texts that were rendered with no cached decorations
/// available and are therefore candidates for an async plugin run.
///
/// Entries are deduplicated by text, so two paragraphs containing the
/// same body generate only one plugin invocation (same text → same
/// cached result). Filling this during render and then draining it from
/// a follow-up scheduler tick is the core of the async-apply loop.
#[derive(Default)]
pub struct PendingDecorations {
    texts: HashMap<String, ()>,
}

impl PendingDecorations {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, text: String) {
        self.texts.entry(text).or_insert(());
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// Returns and clears the current set of pending texts. The returned
    /// order is unspecified; callers that care about stability must sort.
    pub fn drain(&mut self) -> Vec<String> {
        let out: Vec<String> = self.texts.drain().map(|(k, _)| k).collect();
        out
    }

    pub fn clear(&mut self) {
        self.texts.clear();
    }
}

/// A decoration plugin scans segment text and reports ranges to decorate.
///
/// Implementations must be stateless with respect to the document; the
/// renderer may call `decorate` many times per frame across different
/// segments and will cache nothing between calls.
///
/// This trait is intentionally **not** `Send + Sync`: the editor runs
/// single-threaded inside a WebAssembly worker, and the built-in
/// [`JsCallbackPlugin`] wraps a `js_sys::Function` which cannot cross
/// threads.
pub trait DecorationPlugin {
    /// Short identifier used for diagnostics and for host-side targeted
    /// `unregister` calls. Must be stable across versions.
    fn name(&self) -> &str;

    /// Returns the byte ranges within `text` that should be decorated.
    ///
    /// Callers guarantee nothing about `text` other than that it is the
    /// exact `TextSegment.text` string. In particular, `text` can contain
    /// arbitrary UTF-8 including CJK, emoji, and control characters.
    /// Implementations must return ranges that sit on character boundaries
    /// — the renderer silently drops any range that does not.
    fn decorate(&self, text: &str) -> Vec<DecoratedRange>;
}

/// Collection of active decoration plugins.
///
/// Queried once per text segment during the draw pass. Can be globally
/// disabled via [`set_enabled`] without tearing down the plugin instances
/// (useful for A/B tests, "preview mode", or temporarily muting all
/// overlays while the user drags a selection).
///
/// [`set_enabled`]: DecorationRegistry::set_enabled
pub struct DecorationRegistry {
    plugins: Vec<Box<dyn DecorationPlugin>>,
    enabled: bool,
}

impl DecorationRegistry {
    /// Creates an empty registry in the enabled state.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            enabled: true,
        }
    }

    /// Registers an additional plugin. Plugins are queried in insertion
    /// order. Multiple plugins with the same name are permitted — they are
    /// all invoked, and [`unregister`] removes every match.
    ///
    /// [`unregister`]: DecorationRegistry::unregister
    pub fn register(&mut self, plugin: Box<dyn DecorationPlugin>) {
        self.plugins.push(plugin);
    }

    /// Removes every plugin whose `name()` equals `name`. Returns `true`
    /// iff at least one plugin was removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|p| p.name() != name);
        self.plugins.len() != before
    }

    /// Enables or disables all plugin invocations wholesale. When
    /// disabled, [`collect`] returns an empty list without calling into
    /// any plugin.
    ///
    /// [`collect`]: DecorationRegistry::collect
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Queries every active plugin and returns the concatenated, validated
    /// list of decorated ranges. Ranges whose endpoints do not lie on
    /// UTF-8 character boundaries are dropped. Empty ranges
    /// (`start == end`) are also dropped because they produce no visible
    /// decoration.
    pub fn collect(&self, text: &str) -> Vec<DecoratedRange> {
        if !self.enabled || self.plugins.is_empty() || text.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for plugin in &self.plugins {
            for range in plugin.decorate(text) {
                if range.start >= range.end
                    || range.end > text.len()
                    || !text.is_char_boundary(range.start)
                    || !text.is_char_boundary(range.end)
                {
                    continue;
                }
                out.push(range);
            }
        }
        out
    }
}

impl Default for DecorationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the pixel-space x-range that covers the byte range
/// `start_byte..end_byte` within a segment whose per-grapheme layout is
/// given by `graphemes` (in visual order, each entry is
/// `(byte_length_of_grapheme, x_offset_at_start_of_grapheme)`) and whose
/// total advance width is `segment_width_px`.
///
/// Returns `(x_start, x_end)` in the segment's local coordinate space
/// (i.e. relative to the segment's left edge — the caller adds `start_x`
/// to both ends to put them in canvas space).
///
/// Returns `None` if the byte range does not intersect any grapheme, or if
/// the segment has no graphemes. Graphemes that start strictly after the
/// range end are ignored; graphemes that end before the range start are
/// skipped.
///
/// ## Why a grapheme-walk (not a proportional slice)
/// The renderer places each grapheme at an x-offset determined by font
/// metrics, letter spacing, and per-grapheme font fallback — not by
/// proportional split of byte width. A naive proportional mapping would
/// misplace decorations under variable-width text, so we must use the
/// authoritative per-grapheme x-offsets produced by the layout engine.
/// Projects a paragraph-scoped decoration range onto a single segment,
/// returning segment-local `(start_byte, end_byte)` offsets.
///
/// Returns `None` if `decoration` does not overlap the segment's byte
/// range `[segment_start, segment_end)`. When the decoration straddles a
/// segment boundary (valid for multi-run matchers), the returned offsets
/// are clamped to the segment's edges — the caller draws one portion per
/// overlapping segment.
pub fn clip_decoration_to_segment(
    decoration: &DecoratedRange,
    segment_start: usize,
    segment_end: usize,
) -> Option<(usize, usize)> {
    if decoration.end <= segment_start || decoration.start >= segment_end {
        return None;
    }
    let local_start = decoration.start.saturating_sub(segment_start);
    let local_end = decoration.end.min(segment_end) - segment_start;
    if local_start >= local_end {
        return None;
    }
    Some((local_start, local_end))
}

pub fn byte_range_to_x_range(
    graphemes: &[(usize, f64)],
    segment_width_px: f64,
    start_byte: usize,
    end_byte: usize,
) -> Option<(f64, f64)> {
    if graphemes.is_empty() || start_byte >= end_byte {
        return None;
    }
    let mut x_start: Option<f64> = None;
    let mut x_end: Option<f64> = None;
    let mut byte_cursor = 0usize;
    for (idx, (byte_len, x_offset)) in graphemes.iter().enumerate() {
        let g_start = byte_cursor;
        let g_end = byte_cursor + byte_len;
        byte_cursor = g_end;

        // Skip graphemes that end before the requested range starts.
        if g_end <= start_byte {
            continue;
        }
        // Stop once we are past the requested range.
        if g_start >= end_byte {
            break;
        }
        // This grapheme overlaps the requested byte range.
        if x_start.is_none() {
            x_start = Some(*x_offset);
        }
        // The end of this grapheme is the next grapheme's x_offset, or the
        // segment's total width when this is the last grapheme.
        let grapheme_end_x = graphemes
            .get(idx + 1)
            .map(|(_, next_x)| *next_x)
            .unwrap_or(segment_width_px);
        x_end = Some(grapheme_end_x);
    }
    match (x_start, x_end) {
        (Some(s), Some(e)) if e > s => Some((s, e)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedPlugin {
        name: &'static str,
        ranges: Vec<DecoratedRange>,
    }

    impl DecorationPlugin for FixedPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn decorate(&self, _text: &str) -> Vec<DecoratedRange> {
            self.ranges.clone()
        }
    }

    fn underline_red() -> DecorationStyle {
        DecorationStyle::Underline {
            color: "#FF0000".into(),
            thickness_px: None,
        }
    }

    fn range(start: usize, end: usize) -> DecoratedRange {
        DecoratedRange {
            start,
            end,
            style: underline_red(),
        }
    }

    #[test]
    fn empty_registry_returns_nothing() {
        let registry = DecorationRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.collect("◯ test").is_empty());
    }

    #[test]
    fn disabled_registry_returns_nothing_even_when_populated() {
        let mut registry = DecorationRegistry::new();
        registry.register(Box::new(FixedPlugin {
            name: "fixed",
            ranges: vec![range(0, 3)],
        }));
        registry.set_enabled(false);
        assert!(registry.collect("abc").is_empty());
    }

    #[test]
    fn collect_drops_ranges_that_straddle_char_boundaries() {
        let text = "あ◯い"; // each char is 3 bytes in UTF-8
        let mut registry = DecorationRegistry::new();
        registry.register(Box::new(FixedPlugin {
            name: "fixed",
            ranges: vec![
                range(3, 6), // valid: covers "◯"
                range(1, 6), // invalid: starts mid-codepoint
                range(3, 3), // invalid: empty
                range(3, text.len() + 1),
            ],
        }));
        let collected = registry.collect(text);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].start, 3);
        assert_eq!(collected[0].end, 6);
    }

    #[test]
    fn unregister_removes_every_plugin_with_matching_name() {
        let mut registry = DecorationRegistry::new();
        registry.register(Box::new(FixedPlugin {
            name: "a",
            ranges: vec![range(0, 1)],
        }));
        registry.register(Box::new(FixedPlugin {
            name: "a",
            ranges: vec![range(1, 2)],
        }));
        registry.register(Box::new(FixedPlugin {
            name: "b",
            ranges: vec![range(0, 1)],
        }));
        assert!(registry.unregister("a"));
        assert!(!registry.unregister("a"));
        // Only "b" remains.
        assert_eq!(registry.collect("xy").len(), 1);
    }

    #[test]
    fn byte_range_to_x_range_covers_single_grapheme() {
        let graphemes = vec![(1, 0.0), (1, 10.0), (1, 20.0)];
        let (s, e) = byte_range_to_x_range(&graphemes, 30.0, 1, 2).unwrap();
        assert!((s - 10.0).abs() < 1e-9);
        assert!((e - 20.0).abs() < 1e-9);
    }

    #[test]
    fn byte_range_to_x_range_covers_last_grapheme_uses_segment_width() {
        let graphemes = vec![(1, 0.0), (1, 10.0), (1, 20.0)];
        let (s, e) = byte_range_to_x_range(&graphemes, 30.0, 2, 3).unwrap();
        assert!((s - 20.0).abs() < 1e-9);
        assert!((e - 30.0).abs() < 1e-9);
    }

    #[test]
    fn byte_range_to_x_range_covers_multibyte_circle_among_cjk() {
        let graphemes = vec![(3, 0.0), (3, 16.0), (3, 34.0)];
        let (s, e) = byte_range_to_x_range(&graphemes, 50.0, 3, 6).unwrap();
        assert!((s - 16.0).abs() < 1e-9);
        assert!((e - 34.0).abs() < 1e-9);
    }

    #[test]
    fn byte_range_to_x_range_returns_none_for_out_of_range_bytes() {
        let graphemes = vec![(1, 0.0), (1, 10.0)];
        assert!(byte_range_to_x_range(&graphemes, 20.0, 5, 6).is_none());
        assert!(byte_range_to_x_range(&graphemes, 20.0, 1, 1).is_none());
        assert!(byte_range_to_x_range(&[], 20.0, 0, 1).is_none());
    }

    #[test]
    fn clip_decoration_fully_inside_segment() {
        // Paragraph: "abcdef" (6 bytes). Segment [2, 5) = "cde".
        // Decoration [3, 5) covers "de" → local (1, 3).
        let d = range(3, 5);
        let (s, e) = clip_decoration_to_segment(&d, 2, 5).unwrap();
        assert_eq!((s, e), (1, 3));
    }

    #[test]
    fn clip_decoration_straddles_left_boundary() {
        // Segment [5, 10). Decoration [2, 7) overlaps [5, 7) → local (0, 2).
        let d = range(2, 7);
        let (s, e) = clip_decoration_to_segment(&d, 5, 10).unwrap();
        assert_eq!((s, e), (0, 2));
    }

    #[test]
    fn clip_decoration_straddles_right_boundary() {
        // Segment [5, 10). Decoration [8, 15) overlaps [8, 10) → local (3, 5).
        let d = range(8, 15);
        let (s, e) = clip_decoration_to_segment(&d, 5, 10).unwrap();
        assert_eq!((s, e), (3, 5));
    }

    #[test]
    fn clip_decoration_enclosing_segment() {
        // Segment [5, 10). Decoration [0, 20) fully covers → local (0, 5).
        let d = range(0, 20);
        let (s, e) = clip_decoration_to_segment(&d, 5, 10).unwrap();
        assert_eq!((s, e), (0, 5));
    }

    #[test]
    fn clip_decoration_disjoint_from_segment() {
        let d = range(10, 15);
        assert!(clip_decoration_to_segment(&d, 0, 5).is_none());
        assert!(clip_decoration_to_segment(&d, 20, 30).is_none());
    }

    #[test]
    fn clip_decoration_touching_boundary_is_disjoint() {
        // Half-open intervals: [5, 10) does not include 10.
        let d = range(10, 15);
        assert!(clip_decoration_to_segment(&d, 5, 10).is_none());
        let d2 = range(0, 5);
        assert!(clip_decoration_to_segment(&d2, 5, 10).is_none());
    }

    #[test]
    fn cache_get_returns_nothing_for_miss() {
        let cache = DecorationCache::new();
        assert!(cache.get("hello").is_none());
    }

    #[test]
    fn cache_put_and_get_round_trips() {
        let mut cache = DecorationCache::new();
        let was_change = cache.put("hello".into(), vec![range(0, 5)]);
        assert!(was_change);
        let cached = cache.get("hello").unwrap();
        assert_eq!(cached, &[range(0, 5)]);
    }

    #[test]
    fn cache_put_empty_on_missing_entry_is_not_a_change() {
        // Rationale: the renderer's cache-miss path already draws with
        // zero decorations. Caching "empty ranges" for a previously
        // unseen text therefore does not change what the next render
        // will draw, so we must report no-change to avoid a useless
        // re-render loop on paragraphs that no plugin matched.
        let mut cache = DecorationCache::new();
        let was_change = cache.put("hello".into(), Vec::new());
        assert!(!was_change);
        // But the entry IS stored so subsequent renders cache-hit and
        // do not re-queue the text as pending.
        assert_eq!(cache.get("hello"), Some(&[] as &[DecoratedRange]));
    }

    #[test]
    fn cache_put_empty_over_nonempty_is_a_change() {
        // When ranges flip from non-empty to empty (e.g. the plugin
        // changed its mind after an edit), the renderer must repaint
        // to remove the old decoration.
        let mut cache = DecorationCache::new();
        cache.put("hello".into(), vec![range(0, 5)]);
        let was_change = cache.put("hello".into(), Vec::new());
        assert!(was_change);
    }

    #[test]
    fn cache_put_idempotent_returns_false_when_same() {
        let mut cache = DecorationCache::new();
        cache.put("hello".into(), vec![range(0, 5)]);
        let was_change = cache.put("hello".into(), vec![range(0, 5)]);
        assert!(
            !was_change,
            "putting identical ranges must be reported as not-a-change"
        );
    }

    #[test]
    fn cache_put_reports_change_when_ranges_differ() {
        let mut cache = DecorationCache::new();
        cache.put("hello".into(), vec![range(0, 5)]);
        let was_change = cache.put("hello".into(), vec![range(1, 4)]);
        assert!(was_change);
    }

    #[test]
    fn cache_bump_version_invalidates_existing_entries() {
        let mut cache = DecorationCache::new();
        cache.put("hello".into(), vec![range(0, 5)]);
        cache.bump_version();
        assert!(cache.get("hello").is_none());
        // After a fresh put at the new version it's visible again.
        cache.put("hello".into(), vec![range(0, 5)]);
        assert!(cache.get("hello").is_some());
    }

    #[test]
    fn pending_dedupes_identical_texts() {
        let mut pending = PendingDecorations::new();
        pending.insert("foo".into());
        pending.insert("foo".into());
        pending.insert("bar".into());
        assert_eq!(pending.len(), 2);
        let drained = pending.drain();
        assert_eq!(drained.len(), 2);
        assert!(pending.is_empty());
    }

    #[test]
    fn cache_records_and_returns_paragraph_indexes() {
        let mut cache = DecorationCache::new();
        cache.record_paragraph_index("hello", 0);
        cache.record_paragraph_index("hello", 5);
        cache.record_paragraph_index("world", 3);
        assert_eq!(cache.paragraph_indexes_for("hello"), &[0, 5]);
        assert_eq!(cache.paragraph_indexes_for("world"), &[3]);
        assert!(cache.paragraph_indexes_for("unknown").is_empty());
    }

    #[test]
    fn cache_does_not_duplicate_same_index_for_one_text() {
        // Same paragraph can go through the render seam multiple times
        // per session (incremental re-render, scroll-triggered render).
        // The reverse index must not accumulate duplicate indexes.
        let mut cache = DecorationCache::new();
        cache.record_paragraph_index("hello", 3);
        cache.record_paragraph_index("hello", 3);
        cache.record_paragraph_index("hello", 3);
        assert_eq!(cache.paragraph_indexes_for("hello"), &[3]);
    }

    #[test]
    fn cache_clear_drops_reverse_index_too() {
        let mut cache = DecorationCache::new();
        cache.record_paragraph_index("hello", 0);
        cache.put("hello".into(), vec![range(0, 3)]);
        cache.clear();
        assert!(cache.paragraph_indexes_for("hello").is_empty());
        assert!(cache.get("hello").is_none());
    }

    #[test]
    fn cache_bump_version_keeps_reverse_index() {
        // `bump_version` only invalidates decoration ranges; the
        // text->paragraph_index mapping stays valid because paragraph
        // identities are independent of the plugin registry.
        let mut cache = DecorationCache::new();
        cache.record_paragraph_index("hello", 0);
        cache.bump_version();
        assert_eq!(cache.paragraph_indexes_for("hello"), &[0]);
    }
}
