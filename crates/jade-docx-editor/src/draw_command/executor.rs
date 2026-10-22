//! Draw command executor for executing DrawCommand on a Canvas context.
//!
//! This module provides functions to execute DrawCommand against any
//! [`Painter`] backend. The browser renderer supplies the canvas-backed one.

use std::rc::Rc;

use crate::document::FontHint;
use crate::draw_command::{DrawCommand, TabLeaderStyle, TextStyle};
use jade_painter::Painter;
use rustc_hash::{FxHashMap, FxHasher};
use unicode_script::{Script as UnicodeScriptKind, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Maximum number of entries in the grapheme width cache.
/// Graphemes are typically single characters or small clusters,
/// so this provides good coverage for most text rendering scenarios.
/// Grapheme width cache capacity. CJK documents have many distinct graphemes,
/// so a larger capacity reduces eviction and expensive measure_text calls.
const GRAPHEME_CACHE_CAPACITY: usize = 2048;

/// Session-lifetime diagnostics for the draw-command REPLAY glyph path,
/// surfaced via `debugRendererStats.glyph_batch.replay_*`. The replay path is a
/// free function (no `&CanvasRenderer`), so counters live in a thread-local
/// (WASM is single-threaded; mirrors the thread-local batch scratch buffers).
/// Monotonic — never reset per render — so a probe observes cumulative shares.
#[derive(Default, Clone, Copy)]
struct ReplayGlyphBatchStats {
    /// `DrawCommand::TextRun` runs (and glyphs) drawn via the batched helper.
    batch_runs: u64,
    batch_glyphs: u64,
    /// `DrawCommand::TextRun` runs (and glyphs) drawn via the per-glyph loop.
    fallback_runs: u64,
    fallback_glyphs: u64,
    /// `DrawCommand::Text` whole-string replays (font-grouped segment fillText,
    /// never per-glyph batched); `glyphs` counts graphemes.
    text_runs: u64,
    text_glyphs: u64,
}

thread_local! {
    static REPLAY_GLYPH_BATCH_STATS: std::cell::Cell<ReplayGlyphBatchStats> =
        const { std::cell::Cell::new(ReplayGlyphBatchStats {
            batch_runs: 0,
            batch_glyphs: 0,
            fallback_runs: 0,
            fallback_glyphs: 0,
            text_runs: 0,
            text_glyphs: 0,
        }) };
}

#[inline]
fn record_replay_batch_run(glyphs: u64) {
    REPLAY_GLYPH_BATCH_STATS.with(|c| {
        let mut s = c.get();
        s.batch_runs += 1;
        s.batch_glyphs += glyphs;
        c.set(s);
    });
}

#[inline]
fn record_replay_fallback_run(glyphs: u64) {
    REPLAY_GLYPH_BATCH_STATS.with(|c| {
        let mut s = c.get();
        s.fallback_runs += 1;
        s.fallback_glyphs += glyphs;
        c.set(s);
    });
}

#[inline]
fn record_replay_text_run(glyphs: u64) {
    REPLAY_GLYPH_BATCH_STATS.with(|c| {
        let mut s = c.get();
        s.text_runs += 1;
        s.text_glyphs += glyphs;
        c.set(s);
    });
}

/// Returns cumulative replay glyph-batch diagnostics as
/// `(batch_runs, batch_glyphs, fallback_runs, fallback_glyphs, text_runs, text_glyphs)`.
pub fn replay_glyph_batch_stats() -> (u64, u64, u64, u64, u64, u64) {
    REPLAY_GLYPH_BATCH_STATS.with(|c| {
        let s = c.get();
        (
            s.batch_runs,
            s.batch_glyphs,
            s.fallback_runs,
            s.fallback_glyphs,
            s.text_runs,
            s.text_glyphs,
        )
    })
}

/// Tracks the current font state to avoid redundant `set_font()` calls.
///
/// Setting fonts on a Canvas context is relatively expensive because the browser
/// must parse the font descriptor string and resolve the font. By tracking the
/// current font, we can skip redundant calls when drawing text with the same font.
///
/// Also includes a fast grapheme width cache for frequently measured single-character
/// strings, which avoids expensive WASM↔JS boundary crossings for measure_text calls.
///
/// ## Usage
///
/// ```ignore
/// let mut font_state = FontStateTracker::new();
/// font_state.set_font_if_changed(context, "12pt Arial")?;
/// // ... draw text ...
/// font_state.set_font_if_changed(context, "12pt Arial")?; // Skipped!
/// ```
#[derive(Clone, Debug, Default)]
pub struct FontStateTracker {
    /// The current font set on the canvas context, stored as Rc<str> so that
    /// pointer comparison with incoming Rc<str> font descriptors is O(1).
    current_font: Option<Rc<str>>,
    current_font_hash: Option<u64>,
    /// Cached fill style to avoid redundant `set_fill_style_str()` calls.
    current_fill_style: Option<String>,
    /// Cached line width to avoid redundant `set_line_width()` calls.
    current_line_width: Option<f64>,
    /// Whether textBaseline is already set to "alphabetic".
    /// Avoids redundant WASM→JS string transfer for this constant value.
    baseline_is_alphabetic: bool,
    /// Fast cache for grapheme widths keyed by (font_hash, grapheme_hash).
    /// Uses u64 hash keys to avoid string allocations during lookup.
    grapheme_cache: FxHashMap<u64, f64>,
    /// Reusable buffer for accumulating text segments in execute_text.
    /// Avoids allocating a new String on every text draw call.
    segment_buf: String,
    /// Reusable buffer for pair kerning measurement.
    pair_buf: String,
}

impl FontStateTracker {
    /// Creates a new font state tracker with no current font.
    pub fn new() -> Self {
        Self {
            current_font: None,
            current_font_hash: None,
            current_fill_style: None,
            current_line_width: None,
            baseline_is_alphabetic: false,
            grapheme_cache: FxHashMap::with_capacity_and_hasher(
                GRAPHEME_CACHE_CAPACITY,
                Default::default(),
            ),
            segment_buf: String::new(),
            pair_buf: String::new(),
        }
    }

    /// Sets the font on the context only if it differs from the current font.
    /// Returns true if the font was changed, false if skipped.
    ///
    /// Uses Rc pointer comparison as the fastest path (O(1) when the Rc
    /// came from the same source), then hash + length before full memcmp.
    #[inline]
    pub fn set_font_if_changed<P: Painter>(&mut self, context: &P, font: &str) -> bool {
        // Fast path: data pointer + length comparison.
        // When `font` is a borrow from the same Rc<str>, its data pointer
        // matches the cached Rc's data pointer, making this O(1).
        if let Some(ref current) = self.current_font {
            let current_str: &str = current;
            if std::ptr::eq(current_str.as_ptr(), font.as_ptr()) && current_str.len() == font.len()
            {
                return false;
            }
        }
        // Medium path: hash + length + content comparison
        let new_hash = Self::compute_font_hash(font);
        if self.current_font_hash == Some(new_hash) {
            if let Some(ref current) = self.current_font {
                let current_str: &str = current;
                if current_str.len() == font.len() && current_str == font {
                    return false;
                }
            }
        }
        context.set_font(font);
        self.current_font = Some(Rc::from(font));
        self.current_font_hash = Some(new_hash);
        true
    }

    /// Sets the font from an Rc<str>, enabling O(1) pointer comparison.
    /// Prefer this over `set_font_if_changed` when the font comes from
    /// an existing Rc<str> (e.g. TextStyle or GlyphRunStep fonts).
    #[inline]
    pub fn set_font_rc_if_changed<P: Painter>(&mut self, context: &P, font: &Rc<str>) -> bool {
        // O(1) pointer comparison: if the Rc points to the same allocation, skip.
        if let Some(ref current) = self.current_font {
            if Rc::ptr_eq(current, font) {
                return false;
            }
        }
        context.set_font(font);
        self.current_font = Some(Rc::clone(font));
        self.current_font_hash = Some(Self::compute_font_hash(font));
        true
    }

    /// Clears all tracked canvas state. Call this after `save()`/`restore()` pairs
    /// that may have modified the context's state.
    #[inline]
    pub fn invalidate(&mut self) {
        self.current_font = None;
        self.current_font_hash = None;
        self.current_fill_style = None;
        self.current_line_width = None;
        self.baseline_is_alphabetic = false;
    }

    /// Sets textBaseline to "alphabetic" only if not already set.
    /// This is the most common baseline and avoids a WASM→JS string transfer.
    #[inline]
    pub fn ensure_text_baseline_alphabetic<P: Painter>(&mut self, context: &P) {
        if !self.baseline_is_alphabetic {
            context.set_text_baseline("alphabetic");
            self.baseline_is_alphabetic = true;
        }
    }

    /// Marks only the fill style as unknown. Use this when fill_style was changed
    /// without a full save()/restore() cycle, keeping font state intact.
    #[inline]
    pub fn invalidate_fill_style(&mut self) {
        self.current_fill_style = None;
    }

    /// Sets the fill style on the context only if it differs from the cached value.
    /// Uses length pre-check before string comparison.
    #[inline]
    pub fn set_fill_style_if_changed<P: Painter>(&mut self, context: &P, color: &str) -> bool {
        if let Some(ref current) = self.current_fill_style {
            if current.len() == color.len() && current.as_str() == color {
                return false;
            }
        }
        context.set_fill_style_str(color);
        self.current_fill_style = Some(color.to_string());
        true
    }

    /// Sets the line width on the context only if it differs from the cached value.
    #[inline]
    pub fn set_line_width_if_changed<P: Painter>(&mut self, context: &P, width: f64) -> bool {
        if self.current_line_width == Some(width) {
            return false;
        }
        context.set_line_width(width);
        self.current_line_width = Some(width);
        true
    }

    /// Returns the currently tracked font, if any.
    #[allow(dead_code)]
    pub fn current(&self) -> Option<&str> {
        self.current_font.as_deref()
    }

    /// Measures text width with caching for single graphemes.
    /// For graphemes, uses a fast hash-based cache to avoid repeated measure_text calls.
    ///
    /// # Safety note
    /// This uses unchecked hash operations for performance. The hash collision
    /// probability is negligible for the cache sizes used here.
    #[inline]
    pub fn measure_text_cached<P: Painter>(
        &mut self,
        context: &P,
        text: &str,
        font: &str,
    ) -> Result<f64, P::Error> {
        // For short strings (graphemes), use the fast cache
        if text.len() <= 8 {
            // Reuse the cached font hash when available (set by set_font_if_changed).
            let font_hash = self
                .current_font_hash
                .filter(|_| self.current_font.as_deref() == Some(font))
                .unwrap_or_else(|| Self::compute_font_hash(font));
            let cache_key = Self::compute_cache_key_from_font_hash(font_hash, text);

            // Fast path: check cache first
            if let Some(&width) = self.grapheme_cache.get(&cache_key) {
                return Ok(width);
            }

            // Cache miss: measure and store
            let width = context.measure_text_width(text)?;

            // Evict oldest entry if at capacity (simple strategy)
            if self.grapheme_cache.len() >= GRAPHEME_CACHE_CAPACITY {
                // Remove a random entry (first one found in iteration)
                if let Some(&key) = self.grapheme_cache.keys().next() {
                    self.grapheme_cache.remove(&key);
                }
            }

            self.grapheme_cache.insert(cache_key, width);
            Ok(width)
        } else {
            // For longer strings, just measure directly
            context.measure_text_width(text)
        }
    }

    /// Computes a cache key from font and text using fast hashing.
    #[inline]
    #[allow(dead_code)]
    fn compute_cache_key(font: &str, text: &str) -> u64 {
        Self::compute_cache_key_from_font_hash(Self::compute_font_hash(font), text)
    }

    #[inline]
    fn compute_font_hash(font: &str) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = FxHasher::default();
        font.hash(&mut hasher);
        hasher.finish()
    }

    #[inline]
    fn compute_cache_key_from_font_hash(font_hash: u64, text: &str) -> u64 {
        if let Some(text_key) = Self::pack_short_text_key(text) {
            return font_hash.rotate_left(7) ^ text_key.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }

        use std::hash::{Hash, Hasher};

        let mut hasher = FxHasher::default();
        font_hash.hash(&mut hasher);
        text.hash(&mut hasher);
        hasher.finish()
    }

    #[inline]
    fn pack_short_text_key(text: &str) -> Option<u64> {
        let bytes = text.as_bytes();
        if bytes.len() > 8 {
            return None;
        }

        let mut packed = (bytes.len() as u64) << 56;
        for (index, byte) in bytes.iter().enumerate() {
            packed |= (*byte as u64) << (index * 8);
        }
        Some(packed)
    }

    /// Clears the grapheme width cache.
    #[inline]
    pub fn clear_cache(&mut self) {
        self.grapheme_cache.clear();
    }
}

/// Script category for font selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptCategory {
    Latin,
    EastAsia,
    Complex,
}

/// Classifies a grapheme into a script category for font selection.
///
/// ## Performance
///
/// Uses fast paths for ASCII and common CJK ranges before falling back to
/// full Unicode script lookup. This avoids the expensive `UnicodeScript` trait
/// method for the vast majority of characters in Japanese/Chinese documents.
#[inline]
pub fn classify_script(grapheme: &str) -> ScriptCategory {
    let bytes = grapheme.as_bytes();
    if bytes.is_empty() {
        return ScriptCategory::Latin;
    }
    let first_byte = bytes[0];

    // Fast path 1: ASCII (0x00-0x7F) — single byte, covers Latin text
    if first_byte < 0x80 {
        return ScriptCategory::Latin;
    }

    // Fast path 2: decode the first codepoint for common CJK / Latin ranges.
    // This avoids the full `UnicodeScript` lookup for >95% of Japanese text.
    if let Some(ch) = grapheme.chars().next() {
        let cp = ch as u32;
        // CJK Unified Ideographs (most common CJK block)
        if (0x4E00..=0x9FFF).contains(&cp)
            // Hiragana
            || (0x3040..=0x309F).contains(&cp)
            // Katakana
            || (0x30A0..=0x30FF).contains(&cp)
            // CJK Symbols and Punctuation + Fullwidth Forms
            || (0x3000..=0x303F).contains(&cp)
            || (0xFF00..=0xFFEF).contains(&cp)
            // Hangul Syllables
            || (0xAC00..=0xD7AF).contains(&cp)
            // CJK Unified Ideographs Extension A
            || (0x3400..=0x4DBF).contains(&cp)
            // CJK Compatibility Ideographs
            || (0xF900..=0xFAFF).contains(&cp)
        {
            return ScriptCategory::EastAsia;
        }
        // Latin Extended (covers accented Latin, common in European text)
        if (0x0080..=0x024F).contains(&cp) {
            return ScriptCategory::Latin;
        }
    }

    // Slow path: full Unicode script analysis
    classify_script_slow(grapheme)
}

/// Slow path for script classification with full Unicode analysis.
#[inline(never)]
fn classify_script_slow(grapheme: &str) -> ScriptCategory {
    for ch in grapheme.chars() {
        match ch.script() {
            UnicodeScriptKind::Han
            | UnicodeScriptKind::Hiragana
            | UnicodeScriptKind::Katakana
            | UnicodeScriptKind::Bopomofo
            | UnicodeScriptKind::Hangul => {
                return ScriptCategory::EastAsia;
            }
            UnicodeScriptKind::Common => {
                if is_east_asian_punctuation(ch) {
                    return ScriptCategory::EastAsia;
                }
            }
            UnicodeScriptKind::Arabic
            | UnicodeScriptKind::Hebrew
            | UnicodeScriptKind::Syriac
            | UnicodeScriptKind::Thaana
            | UnicodeScriptKind::Thai
            | UnicodeScriptKind::Lao
            | UnicodeScriptKind::Devanagari
            | UnicodeScriptKind::Bengali
            | UnicodeScriptKind::Gurmukhi
            | UnicodeScriptKind::Gujarati
            | UnicodeScriptKind::Oriya
            | UnicodeScriptKind::Tamil
            | UnicodeScriptKind::Telugu
            | UnicodeScriptKind::Kannada
            | UnicodeScriptKind::Malayalam
            | UnicodeScriptKind::Sinhala
            | UnicodeScriptKind::Tibetan
            | UnicodeScriptKind::Myanmar => {
                return ScriptCategory::Complex;
            }
            _ => {}
        }

        if is_east_asian_punctuation(ch) {
            return ScriptCategory::EastAsia;
        }
    }
    ScriptCategory::Latin
}

fn apply_hint_to_script(
    script: ScriptCategory,
    hint: Option<&FontHint>,
    grapheme: &str,
) -> ScriptCategory {
    // w:hint moves shared-range Latin to the EastAsia/Complex font in Word;
    // Basic Latin always keeps the ascii font (ECMA-376 Part 1 §17.3.2.26).
    if grapheme.is_ascii() {
        return script;
    }
    match (script, hint) {
        (ScriptCategory::Latin, Some(FontHint::EastAsia)) => ScriptCategory::EastAsia,
        (ScriptCategory::Latin, Some(FontHint::Complex)) => ScriptCategory::Complex,
        _ => script,
    }
}

#[inline]
fn is_east_asian_punctuation(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x3000..=0x303F   // CJK Symbols and Punctuation
            | 0xFE30..=0xFE4F // CJK Compatibility Forms
            | 0xFF00..=0xFFEF // Halfwidth and Fullwidth Forms
            | 0x31C0..=0x31EF // CJK Strokes (wide forms)
    )
}

impl TextStyle {
    /// Returns the font descriptor for the given script category.
    pub fn font_for_script(&self, script: ScriptCategory) -> &str {
        match script {
            ScriptCategory::Latin => &self.font_descriptor_latin_ascii,
            ScriptCategory::EastAsia => &self.font_descriptor_east_asia,
            ScriptCategory::Complex => &self.font_descriptor_complex,
        }
    }

    pub fn font_for_grapheme(&self, grapheme: &str, script: ScriptCategory) -> &str {
        // Apply w:hint before deciding which font to use for this grapheme.
        let script = apply_hint_to_script(script, self.font_hint.as_ref(), grapheme);
        match script {
            ScriptCategory::Latin => self.latin_descriptor_for_grapheme(grapheme),
            ScriptCategory::EastAsia => &self.font_descriptor_east_asia,
            ScriptCategory::Complex => &self.font_descriptor_complex,
        }
    }

    fn latin_descriptor_for_grapheme(&self, grapheme: &str) -> &str {
        // Word treats ASCII as w:ascii and non-ASCII Latin as w:hAnsi.
        if grapheme.is_ascii() || self.font_descriptor_latin_hansi.is_empty() {
            &self.font_descriptor_latin_ascii
        } else {
            &self.font_descriptor_latin_hansi
        }
    }

    fn requires_advanced_layout(&self) -> bool {
        (self.text_scale_x - 1.0).abs() > f64::EPSILON || self.enable_pair_kerning
    }
}

/// Converts points to CSS pixels.
#[inline]
pub fn points_to_px(points: f32) -> f64 {
    (points as f64) * 96.0 / 72.0
}

/// Executes a single DrawCommand on the given canvas context.
pub fn execute_command<P: Painter>(context: &P, command: &DrawCommand) -> Result<(), P::Error> {
    let mut font_state = FontStateTracker::new();
    execute_command_with_font_state(context, command, &mut font_state)
}

/// Executes a single DrawCommand on the given canvas context with font state tracking.
pub fn execute_command_with_font_state<P: Painter>(
    context: &P,
    command: &DrawCommand,
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    match command {
        DrawCommand::TextRun {
            text,
            x,
            y,
            style,
            fonts,
            glyphs,
        } => {
            execute_text_run_with_font_state(
                context, text, *x, *y, style, fonts, glyphs, font_state,
            )?;
        }
        DrawCommand::Text { text, x, y, style } => {
            execute_text_with_font_state(context, text, *x, *y, style, font_state)?;
        }
        DrawCommand::TextWithSpacing {
            text,
            x,
            y,
            letter_spacing_px,
            style,
        } => {
            execute_text_with_spacing_and_font_state(
                context,
                text,
                *x,
                *y,
                *letter_spacing_px,
                style,
                font_state,
            )?;
        }
        DrawCommand::StrikeThrough {
            x,
            y,
            width,
            font_size_pt,
            color,
            double,
        } => {
            execute_strike_through(context, *x, *y, *width, *font_size_pt, color, *double)?;
        }
        DrawCommand::Underline {
            x,
            y,
            width,
            font_size_pt,
            color,
            kind,
        } => {
            execute_underline(
                context,
                *x,
                *y,
                *width,
                *font_size_pt,
                color,
                kind.as_deref(),
            )?;
        }
        DrawCommand::SquigglyUnderline {
            x,
            y,
            width,
            font_size_pt,
            color,
        } => {
            execute_squiggly_underline(context, *x, *y, *width, *font_size_pt, color)?;
        }
        DrawCommand::EmphasisMark {
            x,
            y,
            radius,
            filled,
            color,
        } => {
            execute_emphasis_mark(context, *x, *y, *radius, *filled, color, font_state)?;
        }
        DrawCommand::FillRect {
            x,
            y,
            width,
            height,
            color,
        } => {
            // Avoid save()/restore() which resets all canvas state and forces
            // expensive font re-application.  Instead, change only fill_style
            // and let the FontStateTracker's fill-style cache handle the
            // restore to the previous color on the next draw call.
            context.set_fill_style_str(color);
            context.fill_rect(*x, *y, *width, *height);
            font_state.invalidate_fill_style();
        }
        DrawCommand::StrokePolyline {
            points,
            line_width,
            color,
        } => {
            context.save();
            context.set_line_dash(&[]);
            context.set_stroke_style_str(color);
            context.set_line_width(*line_width);
            context.begin_path();
            for (index, &(x, y)) in points.iter().enumerate() {
                if index == 0 {
                    context.move_to(x, y);
                } else {
                    context.line_to(x, y);
                }
            }
            context.stroke();
            context.restore();
            font_state.invalidate();
        }
        DrawCommand::ClearRect {
            x,
            y,
            width,
            height,
        } => {
            context.clear_rect(*x, *y, *width, *height);
        }
        DrawCommand::SelectionHighlight {
            x,
            y,
            width,
            height,
        } => {
            const SELECTION_FILL_STYLE: &str = "rgba(51, 153, 255, 0.35)";
            context.set_fill_style_str(SELECTION_FILL_STYLE);
            context.fill_rect(*x, *y, *width, *height);
            font_state.invalidate_fill_style();
        }
        DrawCommand::CommentHighlight {
            x,
            y,
            width,
            height,
        } => {
            const COMMENT_HIGHLIGHT_FILL_STYLE: &str = "rgba(255, 213, 79, 0.35)";
            context.set_fill_style_str(COMMENT_HIGHLIGHT_FILL_STYLE);
            context.fill_rect(*x, *y, *width, *height);
            font_state.invalidate_fill_style();
        }
        DrawCommand::TabLeader {
            baseline,
            leader,
            style,
            glyph_positions,
            ..
        } => {
            execute_tab_leader_with_font_state(
                context,
                *baseline,
                *leader,
                style,
                glyph_positions,
                font_state,
            )?;
        }
        DrawCommand::ControlMark {
            symbol,
            x,
            baseline,
            style,
        } => {
            execute_control_mark_with_font_state(
                context, symbol, *x, *baseline, style, font_state,
            )?;
        }
        DrawCommand::RevisionHighlight {
            x,
            y,
            width,
            height,
            is_inserted,
        } => {
            const REVISION_INSERTED_FILL: &str = "rgba(129, 199, 132, 0.35)";
            const REVISION_DELETED_FILL: &str = "rgba(239, 154, 154, 0.35)";
            let fill_color = if *is_inserted {
                REVISION_INSERTED_FILL
            } else {
                REVISION_DELETED_FILL
            };
            context.set_fill_style_str(fill_color);
            context.fill_rect(*x, *y, *width, *height);
            font_state.invalidate_fill_style();
        }
        DrawCommand::Save => {
            context.save();
            // Note: save() pushes state; we don't invalidate here but on restore()
        }
        DrawCommand::ClipRect {
            x,
            y,
            width,
            height,
        } => {
            context.begin_path();
            context.rect(*x, *y, *width, *height);
            context.clip();
            font_state.invalidate();
        }
        DrawCommand::Restore => {
            context.restore();
            // restore() pops state which may have a different font
            font_state.invalidate();
        }
        DrawCommand::SetFillStyle { color } => {
            font_state.set_fill_style_if_changed(context, color);
        }
        DrawCommand::SetFont { font } => {
            font_state.set_font_rc_if_changed(context, font);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_text_run_with_font_state<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    if glyphs.is_empty() {
        return Ok(());
    }
    if let Some(enclosure) = style
        .effects
        .as_ref()
        .and_then(|effects| effects.enclosure.as_ref())
    {
        return draw_enclosed_character(
            context, text, x, y, enclosure, style, fonts, glyphs, font_state,
        );
    }
    if let Some(combine) = style
        .effects
        .as_ref()
        .and_then(|effects| effects.combine.as_ref())
    {
        return draw_combined_characters(
            context, text, x, y, combine, style, fonts, glyphs, font_state,
        );
    }
    // `w:shadow` (§17.3.2.31) / `w:emboss` (§17.3.2.13) / `w:imprint`
    // (§17.3.2.18): apply a native canvas shadow around the glyph fills, then
    // always clear it so later draws are unaffected. The shadow propagates to
    // every subsequent fill_text on this context (including the batched BMP
    // glyph helper), so a single set/clear pair covers both the scaled and
    // unscaled draw paths below. emboss/imprint use a light highlight shadow
    // offset in opposite directions to fake the raised/engraved look on a light
    // page; emboss takes priority over imprint over shadow (mutually exclusive
    // in Word).
    // `w14:glow`: the glyphs drawn once with a soft, unoffset shadow of the
    // glow's colour, which haloes them; their own fill draws over it.
    if let Some((color, radius)) = style
        .effects
        .as_ref()
        .and_then(|effects| effects.glow.as_ref())
    {
        context.set_shadow_color(color);
        context.set_shadow_offset_x(0.0);
        context.set_shadow_offset_y(0.0);
        context.set_shadow_blur(*radius);
        // Twice: one blurred pass is fainter near the glyph than Word's glow.
        let halo = draw_text_run_glyphs(context, text, x, y, style, fonts, glyphs, font_state)
            .and_then(|()| {
                draw_text_run_glyphs(context, text, x, y, style, fonts, glyphs, font_state)
            });
        clear_text_shadow(context);
        halo?;
    }
    let shadow_applied = if style.emboss {
        apply_emboss_shadow(context, style.font_size_pt, false);
        true
    } else if style.imprint {
        apply_emboss_shadow(context, style.font_size_pt, true);
        true
    } else if style.shadow {
        apply_text_shadow(context, style.font_size_pt);
        true
    } else {
        false
    };

    let result = draw_text_run_glyphs(context, text, x, y, style, fonts, glyphs, font_state);

    if shadow_applied {
        clear_text_shadow(context);
    }
    result?;
    // `w14:textOutline`: the glyph outlines stroked in their own colour over
    // the fill.
    if let Some((color, width)) = style
        .effects
        .as_ref()
        .and_then(|effects| effects.outline.as_ref())
    {
        let mut stroked = style.clone();
        stroked.outline = true;
        stroked.fill_color = Rc::clone(color);
        stroked.effects = None;
        context.set_stroke_style_str(color);
        context.set_line_width(width.max(0.5));
        stroke_text_run_glyphs(context, text, x, y, &stroked, fonts, glyphs, font_state)?;
        font_state.invalidate();
    }
    Ok(())
}

/// 組み文字 / 割注: the run's glyphs as two lines, the first half raised and
/// the second lowered, each starting where the run does, unscaled; 割注's
/// brackets at full size either side.
#[allow(clippy::too_many_arguments)]
fn draw_combined_characters<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    combine: &crate::draw_command::CombinedCharacters,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    let split = glyphs.len().div_ceil(2);
    let mut start_x = x + glyphs.first().map_or(0.0, |glyph| glyph.x_offset);
    font_state.set_fill_style_if_changed(context, &style.fill_color);
    font_state.ensure_text_baseline_alphabetic(context);
    let first_font = glyphs
        .first()
        .and_then(|glyph| fonts.get(glyph.font_index as usize));
    // A bracket at twice the run's size, sitting on the line's baseline.
    let bracket =
        |mark: &str, at: f64, font_state: &mut FontStateTracker| -> Result<f64, P::Error> {
            if let Some(font) = first_font {
                font_state.set_font_rc_if_changed(context, font);
            }
            let width = context.measure_text_width(mark)? * 2.0;
            context.save();
            context.translate(at, y + combine.lower)?;
            context.scale(2.0, 2.0)?;
            context.fill_text(mark, 0.0, 0.0)?;
            context.restore();
            font_state.invalidate();
            Ok(width)
        };
    if let Some((open, _)) = combine.brackets.as_ref() {
        start_x += bracket(open, start_x, font_state)?;
    }
    let mut widest = start_x;
    for (line, baseline) in [
        (&glyphs[..split], y - combine.raise),
        (&glyphs[split..], y + combine.lower),
    ] {
        let mut pen = start_x;
        for glyph in line {
            let (Some(font_descriptor), Some(glyph_text)) =
                (fonts.get(glyph.font_index as usize), glyph.text(text))
            else {
                continue;
            };
            font_state.set_font_rc_if_changed(context, font_descriptor);
            context.fill_text(glyph_text, pen, baseline)?;
            pen += context.measure_text_width(glyph_text)?;
        }
        widest = widest.max(pen);
    }
    if let Some((_, close)) = combine.brackets.as_ref() {
        bracket(close, widest, font_state)?;
    }
    font_state.invalidate();
    Ok(())
}

/// 囲い文字: the enclosure at the run's size and the character at two
/// thirds of it, centred inside, as Word's 文字のサイズを合わせる draws it.
#[allow(clippy::too_many_arguments)]
fn draw_enclosed_character<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    enclosure: &str,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    let Some(first) = glyphs.first() else {
        return Ok(());
    };
    let Some(font) = fonts.get(first.font_index as usize) else {
        return Ok(());
    };
    let start_x = x + first.x_offset;
    let inner: String = glyphs.iter().filter_map(|glyph| glyph.text(text)).collect();
    font_state.set_fill_style_if_changed(context, &style.fill_color);
    font_state.ensure_text_baseline_alphabetic(context);
    font_state.set_font_rc_if_changed(context, font);
    let enclosure_width = context.measure_text_width(enclosure)?;
    context.fill_text(enclosure, start_x, y)?;
    let inner_width = context.measure_text_width(&inner)?;
    const SHRINK: f64 = 2.0 / 3.0;
    let em = style.font_size_pt as f64 * 96.0 / 72.0;
    context.save();
    context.translate(
        start_x + (enclosure_width - inner_width * SHRINK) * 0.5,
        // Raised so the smaller character sits in the middle of the
        // enclosure rather than on the baseline.
        y - em * 0.11,
    )?;
    context.scale(SHRINK, SHRINK)?;
    context.fill_text(&inner, 0.0, 0.0)?;
    context.restore();
    font_state.invalidate();
    Ok(())
}

/// Strokes each glyph of a run with the current stroke style.
#[allow(clippy::too_many_arguments)]
fn stroke_text_run_glyphs<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    let scale = style.text_scale_x;
    context.save();
    context.translate(0.0, y)?;
    context.scale(scale, 1.0)?;
    for glyph in glyphs {
        let (Some(font_descriptor), Some(glyph_text)) =
            (fonts.get(glyph.font_index as usize), glyph.text(text))
        else {
            continue;
        };
        font_state.set_font_rc_if_changed(context, font_descriptor);
        context.stroke_text(glyph_text, (x + glyph.x_offset) / scale, 0.0)?;
    }
    context.restore();
    font_state.invalidate();
    Ok(())
}

/// Applies a Word-style drop shadow to the canvas context. The offset and blur
/// scale with the font size so larger glyphs cast a proportionally larger
/// shadow. The shadow stays active until [`clear_text_shadow`] is called.
fn apply_text_shadow<P: Painter>(context: &P, font_size_pt: f32) {
    let offset = text_shadow_offset_px(font_size_pt);
    context.set_shadow_color("rgba(0, 0, 0, 0.35)");
    context.set_shadow_offset_x(offset);
    context.set_shadow_offset_y(offset);
    context.set_shadow_blur(offset * 0.5);
}

/// Applies a light highlight shadow to fake an embossed (raised) or imprinted
/// (engraved) look. On a light page, a white highlight cast below-right reads as
/// raised, while the same highlight cast above-left reads as pressed in. The
/// `engrave` flag flips the offset direction. The effect stays active until
/// [`clear_text_shadow`] is called.
fn apply_emboss_shadow<P: Painter>(context: &P, font_size_pt: f32, engrave: bool) {
    let offset = emboss_shadow_offset_px(font_size_pt);
    let signed = if engrave { -offset } else { offset };
    context.set_shadow_color("rgba(255, 255, 255, 0.7)");
    context.set_shadow_offset_x(signed);
    context.set_shadow_offset_y(signed);
    context.set_shadow_blur(0.0);
}

/// Highlight offset (in CSS px) for an emboss/imprint run, scaled with the font
/// size and clamped to at least 1px so the effect stays visible at small sizes.
fn emboss_shadow_offset_px(font_size_pt: f32) -> f64 {
    ((font_size_pt as f64) * 0.05).max(1.0)
}

/// Stroke width (in CSS px) for a `w:outline` run, scaled with the font size and
/// clamped to at least 1px so the hollow glyph outline stays visible at small
/// sizes without flooding the interior at large sizes.
fn outline_line_width_px(font_size_pt: f32) -> f64 {
    ((font_size_pt as f64) * 0.04).max(1.0)
}

/// Drop-shadow offset (in CSS px) for a `w:shadow` run, scaled with the font
/// size and clamped to at least 1px so small text still casts a visible shadow.
fn text_shadow_offset_px(font_size_pt: f32) -> f64 {
    ((font_size_pt as f64) * 0.08).max(1.0)
}

/// Resets the canvas drop shadow to the inert default so subsequent draws are
/// not shadowed.
fn clear_text_shadow<P: Painter>(context: &P) {
    context.set_shadow_color("rgba(0, 0, 0, 0)");
    context.set_shadow_offset_x(0.0);
    context.set_shadow_offset_y(0.0);
    context.set_shadow_blur(0.0);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_run_glyphs<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    font_state.set_fill_style_if_changed(context, &style.fill_color);
    font_state.ensure_text_baseline_alphabetic(context);

    // `w:outline` (§17.3.2.23): draw the glyphs as hollow stroked outlines using
    // the run's color, rather than solid fills. The glyph-batch fast path only
    // fills, so outline runs always take the per-glyph stroke path below.
    if style.outline {
        context.set_stroke_style_str(&style.fill_color);
        context.set_line_width(outline_line_width_px(style.font_size_pt));
    }

    let needs_scale = (style.text_scale_x - 1.0).abs() > f64::EPSILON;

    if needs_scale {
        record_replay_fallback_run(glyphs.len() as u64);
        // For scaled text, apply transform once for the entire run.
        context.save();
        context.translate(0.0, y)?;
        context.scale(style.text_scale_x, 1.0)?;
        for glyph in glyphs {
            let Some(font_descriptor) = fonts.get(glyph.font_index as usize) else {
                continue;
            };
            let Some(glyph_text) = glyph.text(text) else {
                continue;
            };
            font_state.set_font_rc_if_changed(context, font_descriptor);
            let gx = (x + glyph.x_offset) / style.text_scale_x;
            if style.outline {
                context.stroke_text(glyph_text, gx, 0.0)?;
            } else {
                context.fill_text(glyph_text, gx, 0.0)?;
            }
        }
        context.restore();
        font_state.invalidate();
        return Ok(());
    }

    if style.outline {
        record_replay_fallback_run(glyphs.len() as u64);
        for glyph in glyphs {
            let Some(font_descriptor) = fonts.get(glyph.font_index as usize) else {
                continue;
            };
            let Some(glyph_text) = glyph.text(text) else {
                continue;
            };
            font_state.set_font_rc_if_changed(context, font_descriptor);
            context.stroke_text(glyph_text, x + glyph.x_offset, y)?;
        }
        return Ok(());
    }

    // Draw each glyph at its pre-calculated position.
    //
    // NOTE: We intentionally do NOT batch consecutive same-font glyphs into a
    // single fillText call here. While batching reduces WASM→JS boundary
    // crossings, it causes the browser to apply its own kerning/letter-spacing
    // to the concatenated string, producing a different total width than the
    // individually measured glyphs used during layout. This mismatch causes
    // visible paragraph expansion/contraction when cache-replayed commands are
    // drawn at positions computed from individual glyph measurements.
    let characters: Vec<PlacedCharacter<'_>> = if fonts
        .iter()
        .any(|font| jade_painter::font_features::has_features(font))
    {
        glyphs
            .iter()
            .filter_map(|glyph| {
                Some(PlacedCharacter {
                    start: glyph.text_start as usize,
                    end: glyph.text_end as usize,
                    font: fonts.get(glyph.font_index as usize)?.as_ref(),
                    x_offset: glyph.x_offset,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    if !characters.is_empty()
        && try_fill_featured_text(context, text, x, y, &characters, &mut |context, font| {
            font_state.set_font_if_changed(context, font);
        })?
    {
        record_replay_batch_run(glyphs.len() as u64);
        return Ok(());
    }
    if try_execute_bmp_glyph_run_batch(context, text, x, y, fonts, glyphs, font_state)? {
        record_replay_batch_run(glyphs.len() as u64);
        return Ok(());
    }
    record_replay_fallback_run(glyphs.len() as u64);
    for glyph in glyphs {
        let Some(font_descriptor) = fonts.get(glyph.font_index as usize) else {
            continue;
        };
        let Some(glyph_text) = glyph.text(text) else {
            continue;
        };
        font_state.set_font_rc_if_changed(context, font_descriptor);
        context.fill_text(glyph_text, x + glyph.x_offset, y)?;
    }
    Ok(())
}

/// One character of a text run as layout placed it: its byte range in the
/// run's text, its font, and its x from the run's origin.
pub(crate) struct PlacedCharacter<'a> {
    pub start: usize,
    pub end: usize,
    pub font: &'a str,
    pub x_offset: f64,
}

/// Draws a run whose fonts carry OpenType features (Word 2010 typography:
/// ligatures, figure styles, stylistic sets) shaped a font's stretch at a
/// time, so a ligature is drawn as the font's ligature glyph rather than its
/// letters side by side. Each character keeps the x layout gave it.
///
/// `Ok(false)` — nothing drawn — when no font in the run carries features;
/// the caller draws it as before. A stretch the painter cannot shape is
/// drawn a character at a time.
pub(crate) fn try_fill_featured_text<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    characters: &[PlacedCharacter<'_>],
    set_font: &mut dyn FnMut(&P, &str),
) -> Result<bool, P::Error> {
    if !characters
        .iter()
        .any(|character| jade_painter::font_features::has_features(character.font))
    {
        return Ok(false);
    }
    let mut index = 0;
    while index < characters.len() {
        let first = &characters[index];
        let mut end = index + 1;
        while end < characters.len()
            && characters[end].font == first.font
            && characters[end].start == characters[end - 1].end
        {
            end += 1;
        }
        let group = &characters[index..end];
        set_font(context, first.font);
        let shaped = if jade_painter::font_features::has_features(first.font) {
            let start = first.start;
            let stop = group[group.len() - 1].end;
            match text.get(start..stop) {
                Some(stretch) => {
                    let clusters: Vec<(usize, f64)> = group
                        .iter()
                        .map(|character| (character.start - start, x + character.x_offset))
                        .collect();
                    context.try_fill_shaped_text(stretch, &clusters, y)?
                }
                None => false,
            }
        } else {
            false
        };
        if !shaped {
            for character in group {
                if let Some(glyph) = text.get(character.start..character.end) {
                    context.fill_text(glyph, x + character.x_offset, y)?;
                }
            }
        }
        index = end;
    }
    Ok(true)
}

fn try_execute_bmp_glyph_run_batch<P: Painter>(
    context: &P,
    run_text: &str,
    x: f64,
    y: f64,
    fonts: &[Rc<str>],
    glyphs: &[crate::draw_command::GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<bool, P::Error> {
    let mut text = String::with_capacity(glyphs.len());
    let mut x_positions = Vec::with_capacity(glyphs.len());
    // Most replayed runs use one font and take the single-font JS path. Allocate
    // indexes only after discovering a font change that requires the multi-font path.
    let mut font_indexes = Vec::new();
    let mut single_font_index: Option<u16> = None;
    let mut single_font_run = true;

    for glyph in glyphs {
        if fonts.get(glyph.font_index as usize).is_none() {
            return Ok(false);
        }
        let Some(glyph_text) = glyph.text(run_text) else {
            return Ok(false);
        };
        let mut chars = glyph_text.chars();
        let Some(ch) = chars.next() else {
            return Ok(false);
        };
        if chars.next().is_some() || ch.len_utf16() != 1 {
            return Ok(false);
        }
        match single_font_index {
            Some(index) if index != glyph.font_index && single_font_run => {
                single_font_run = false;
                font_indexes.reserve(glyphs.len());
                font_indexes.resize(x_positions.len(), index);
            }
            None => single_font_index = Some(glyph.font_index),
            _ => {}
        }
        text.push(ch);
        x_positions.push(x + glyph.x_offset);
        if !single_font_run {
            font_indexes.push(glyph.font_index);
        }
    }

    if single_font_run {
        let Some(font_descriptor) = single_font_index.and_then(|index| fonts.get(index as usize))
        else {
            return Ok(false);
        };
        font_state.set_font_rc_if_changed(context, font_descriptor);
        if !context.try_fill_glyph_run_batch(&text, &x_positions, y)? {
            return Ok(false);
        }
    } else {
        if !context.try_fill_glyph_run_batch_multi_font(
            &text,
            &x_positions,
            &font_indexes,
            fonts,
            y,
        )? {
            return Ok(false);
        }
        // The multi-font JS helper mutates `ctx.font` directly while preserving exact
        // glyph positions. Invalidate tracked canvas state so later Rust-side draws do
        // not assume the pre-batch font is still active.
        font_state.invalidate();
    }
    Ok(true)
}

/// Executes a list of DrawCommands on the given canvas context.
pub fn execute_commands<P: Painter>(context: &P, commands: &[DrawCommand]) -> Result<(), P::Error> {
    let mut font_state = FontStateTracker::new();
    execute_commands_with_font_state(context, commands, &mut font_state)
}

/// Executes a list of DrawCommands on the given canvas context with font state tracking.
///
/// This variant allows callers to share a font state tracker across multiple
/// command batches, maximizing the benefit of font caching.
pub fn execute_commands_with_font_state<P: Painter>(
    context: &P,
    commands: &[DrawCommand],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    for command in commands {
        execute_command_with_font_state(context, command, font_state)?;
    }
    Ok(())
}

/// Executes draw commands with a y-axis offset applied via canvas translate,
/// avoiding the need to clone each command with `with_y_offset`.
///
/// When `y_offset` is zero (or negligible), delegates directly to
/// `execute_commands_with_font_state` without any translate overhead.
pub fn execute_commands_with_y_offset<P: Painter>(
    context: &P,
    commands: &[DrawCommand],
    y_offset: f64,
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    if y_offset.abs() < f64::EPSILON {
        return execute_commands_with_font_state(context, commands, font_state);
    }
    context.save();
    context.translate(0.0, y_offset)?;
    // Invalidate font/fill state because save() pushes canvas state.
    font_state.invalidate();
    let result = execute_commands_with_font_state(context, commands, font_state);
    context.restore();
    font_state.invalidate();
    result
}

/// Executes text drawing without letter spacing.
#[allow(dead_code)]
fn execute_text<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
) -> Result<f64, P::Error> {
    let mut font_state = FontStateTracker::new();
    execute_text_with_font_state(context, text, x, y, style, &mut font_state)
}

/// Executes text drawing without letter spacing, with font state tracking.
fn execute_text_with_font_state<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    font_state: &mut FontStateTracker,
) -> Result<f64, P::Error> {
    if text.is_empty() {
        return Ok(0.0);
    }

    // Keep replayed body text on alphabetic baseline even if callers changed it.
    font_state.ensure_text_baseline_alphabetic(context);
    if style.requires_advanced_layout() {
        return execute_text_with_spacing_and_font_state(
            context, text, x, y, 0.0, style, font_state,
        );
    }

    font_state.set_fill_style_if_changed(context, &style.fill_color);
    record_replay_text_run(text.graphemes(true).count() as u64);

    let mut width = 0.0;
    let mut cursor_x = x;
    // Group consecutive graphemes by the resolved font descriptor.
    let mut current_font: Option<&str> = None;
    // Take the tracker's segment buffer to avoid per-call allocation.
    // The buffer retains its heap capacity across calls.
    let mut segment = std::mem::take(&mut font_state.segment_buf);
    segment.clear();

    for grapheme in text.graphemes(true) {
        let script = classify_script(grapheme);
        let font = style.font_for_grapheme(grapheme, script);
        match current_font {
            Some(active) if active == font => {}
            Some(active) => {
                // Flush the current segment before changing font
                if !segment.is_empty() {
                    font_state.set_font_if_changed(context, active);
                    // Measure first (with caching), then draw
                    let segment_width =
                        font_state.measure_text_cached(context, &segment, active)?;
                    context.fill_text(&segment, cursor_x, y)?;
                    cursor_x += segment_width;
                    width += segment_width;
                    segment.clear();
                }
                current_font = Some(font);
            }
            None => {
                current_font = Some(font);
            }
        }
        segment.push_str(grapheme);
    }

    // Flush any remaining segment
    if let Some(active) = current_font {
        if !segment.is_empty() {
            font_state.set_font_if_changed(context, active);
            // Measure first (with caching), then draw
            let segment_width = font_state.measure_text_cached(context, &segment, active)?;
            context.fill_text(&segment, cursor_x, y)?;
            width += segment_width;
        }
    }

    // Return the buffer to the tracker for reuse in future calls.
    font_state.segment_buf = segment;

    Ok(width)
}

/// Executes text drawing with letter spacing.
#[allow(dead_code)]
fn execute_text_with_spacing<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    letter_spacing: f64,
    style: &TextStyle,
) -> Result<f64, P::Error> {
    let mut font_state = FontStateTracker::new();
    execute_text_with_spacing_and_font_state(
        context,
        text,
        x,
        y,
        letter_spacing,
        style,
        &mut font_state,
    )
}

/// Executes text drawing with letter spacing and font state tracking.
fn execute_text_with_spacing_and_font_state<P: Painter>(
    context: &P,
    text: &str,
    mut x: f64,
    y: f64,
    letter_spacing: f64,
    style: &TextStyle,
    font_state: &mut FontStateTracker,
) -> Result<f64, P::Error> {
    if text.is_empty() {
        return Ok(0.0);
    }

    // Keep replayed body text on alphabetic baseline even if callers changed it.
    font_state.ensure_text_baseline_alphabetic(context);
    font_state.set_fill_style_if_changed(context, &style.fill_color);

    if letter_spacing.abs() < f64::EPSILON && !style.requires_advanced_layout() {
        return execute_text_with_font_state(context, text, x, y, style, font_state);
    }

    let text_scale_x = if style.text_scale_x.is_finite() && style.text_scale_x > 0.0 {
        style.text_scale_x
    } else {
        1.0
    };
    let needs_scale = (text_scale_x - 1.0).abs() > f64::EPSILON;
    let mut width = 0.0;
    let mut iter = text.graphemes(true).peekable();
    // Use per-grapheme font switching to mirror Word's ascii/hAnsi behavior.
    let mut current_font = style.font_for_script(ScriptCategory::Latin);
    let mut prev_grapheme: Option<&str> = None;
    let mut prev_font: Option<&str> = None;
    let mut prev_raw_glyph_width: Option<f64> = None;
    font_state.set_font_if_changed(context, current_font);

    // Apply horizontal scale once for the entire run instead of per-grapheme.
    // Each grapheme's x position is pre-divided by text_scale_x so that after
    // the canvas transform the visual position is correct.
    if needs_scale {
        context.save();
        context.translate(0.0, y)?;
        context.scale(text_scale_x, 1.0)?;
    }

    while let Some(grapheme) = iter.next() {
        let script = classify_script(grapheme);
        let font = style.font_for_grapheme(grapheme, script);
        if font != current_font {
            current_font = font;
            font_state.set_font_if_changed(context, current_font);
        }
        // Measure first (with caching), then draw
        let raw_glyph_width = font_state.measure_text_cached(context, grapheme, current_font)?;
        let glyph_width = raw_glyph_width * text_scale_x;
        if style.enable_pair_kerning
            && prev_grapheme.is_some()
            && prev_font == Some(current_font)
            && prev_raw_glyph_width.is_some()
        {
            // Reuse the tracker's pair buffer to avoid per-iteration allocation.
            let mut pair = std::mem::take(&mut font_state.pair_buf);
            pair.clear();
            pair.push_str(prev_grapheme.unwrap_or_default());
            pair.push_str(grapheme);
            let pair_raw_width = font_state.measure_text_cached(context, &pair, current_font)?;
            font_state.pair_buf = pair;
            let raw_adjustment =
                pair_raw_width - prev_raw_glyph_width.unwrap_or_default() - raw_glyph_width;
            if raw_adjustment.is_finite() {
                let kerning_adjustment = raw_adjustment * text_scale_x;
                x += kerning_adjustment;
                width += kerning_adjustment;
            }
        }
        if needs_scale {
            // Canvas is already scaled; draw at unscaled x position so the
            // scale transform produces the correct visual position.
            context.fill_text(grapheme, x / text_scale_x, 0.0)?;
        } else {
            context.fill_text(grapheme, x, y)?;
        }
        x += glyph_width;
        width += glyph_width;

        if iter.peek().is_some() {
            x += letter_spacing;
            width += letter_spacing;
        }

        prev_grapheme = Some(grapheme);
        prev_font = Some(current_font);
        prev_raw_glyph_width = Some(raw_glyph_width);
    }

    if needs_scale {
        context.restore();
        font_state.invalidate();
    }

    Ok(width)
}

/// Executes strikethrough drawing.
fn execute_strike_through<P: Painter>(
    context: &P,
    start_x: f64,
    baseline_y: f64,
    width: f64,
    font_size_pt: f32,
    color: &str,
    double: bool,
) -> Result<(), P::Error> {
    if width <= 0.0 {
        return Ok(());
    }

    let font_px = points_to_px(font_size_pt);
    let strike_y = baseline_y - font_px * 0.3;

    context.save();
    context.set_stroke_style_str(color);
    context.set_line_width((font_px * 0.05).max(1.0));
    context.begin_path();
    if double {
        // Two parallel lines straddling the single-strike baseline.
        let gap = (font_px * 0.06).max(1.0);
        context.move_to(start_x, strike_y - gap);
        context.line_to(start_x + width, strike_y - gap);
        context.move_to(start_x, strike_y + gap);
        context.line_to(start_x + width, strike_y + gap);
    } else {
        context.move_to(start_x, strike_y);
        context.line_to(start_x + width, strike_y);
    }
    context.stroke();
    context.restore();

    Ok(())
}

/// Executes underline drawing.
pub(crate) fn execute_underline<P: Painter>(
    context: &P,
    start_x: f64,
    baseline_y: f64,
    width: f64,
    font_size_pt: f32,
    color: &str,
    kind: Option<&str>,
) -> Result<(), P::Error> {
    if width <= 0.0 {
        return Ok(());
    }

    context.save();
    context.set_stroke_style_str(color);
    stroke_underline_with_kind(context, start_x, baseline_y, width, font_size_pt, kind)?;
    context.restore();

    Ok(())
}

/// Strokes an underline of the given `w:u` kind (§17.3.2.40) with the current
/// stroke style. The caller owns save/restore; line width and dash pattern are
/// set here. Unknown kinds fall back to the single rule. `words` is split into
/// non-whitespace spans by the renderer before commands reach this executor.
pub(crate) fn stroke_underline_with_kind<P: Painter>(
    context: &P,
    start_x: f64,
    baseline_y: f64,
    width: f64,
    font_size_pt: f32,
    kind: Option<&str>,
) -> Result<(), P::Error> {
    if width <= 0.0 {
        return Ok(());
    }
    let font_px = points_to_px(font_size_pt);
    let underline_y = baseline_y + font_px * 0.15;
    let base = (font_px * 0.05).max(1.0);
    let kind = kind.unwrap_or("single");
    let heavy = matches!(
        kind,
        "thick"
            | "dottedHeavy"
            | "dashedHeavy"
            | "dashLongHeavy"
            | "dashDotHeavy"
            | "dashDotDotHeavy"
            | "wavyHeavy"
    );
    let line_width = if heavy { base * 2.0 } else { base };
    context.set_line_width(line_width);

    if matches!(kind, "wave" | "wavyHeavy" | "wavyDouble") {
        stroke_wave(context, start_x, underline_y, width, font_px);
        if kind == "wavyDouble" {
            stroke_wave(
                context,
                start_x,
                underline_y + line_width * 2.5,
                width,
                font_px,
            );
        }
        return Ok(());
    }

    if kind == "double" {
        // Keep a visible gap between the two rules; at small sizes a 2×
        // offset collapses into one thick line after canvas scaling.
        for y in [underline_y, underline_y + (line_width + base * 2.0)] {
            context.begin_path();
            context.move_to(start_x, y);
            context.line_to(start_x + width, y);
            context.stroke();
        }
        return Ok(());
    }

    let dash = underline_dash_pattern(kind, base);
    if let Some(pattern) = dash.as_ref() {
        context.set_line_dash(pattern);
    }
    context.begin_path();
    context.move_to(start_x, underline_y);
    context.line_to(start_x + width, underline_y);
    context.stroke();
    if dash.is_some() {
        context.set_line_dash(&[]);
    }
    Ok(())
}

/// Dash pattern for a `w:u` kind, scaled by the base line width. `None`
/// means a solid stroke. Heavy variants share their base pattern; the stroke
/// width difference is applied by the caller.
pub(crate) fn underline_dash_pattern(kind: &str, base: f64) -> Option<Vec<f64>> {
    match kind {
        "dotted" | "dottedHeavy" => Some(vec![base, base * 1.6]),
        "dash" | "dashedHeavy" => Some(vec![base * 4.0, base * 2.0]),
        "dashLong" | "dashLongHeavy" => Some(vec![base * 8.0, base * 3.0]),
        "dotDash" | "dashDotHeavy" => Some(vec![base * 4.0, base * 2.0, base, base * 2.0]),
        "dotDotDash" | "dashDotDotHeavy" => Some(vec![
            base * 4.0,
            base * 2.0,
            base,
            base * 2.0,
            base,
            base * 2.0,
        ]),
        _ => None,
    }
}

/// One zigzag wave pass centered on `y`, matching the squiggly geometry so
/// `w:u wave` and lint squiggles stay visually consistent.
fn stroke_wave<P: Painter>(context: &P, start_x: f64, y: f64, width: f64, font_px: f64) {
    let amplitude = (font_px * 0.10).clamp(1.0, 3.0);
    let wavelength = (font_px * 0.30).clamp(3.0, 6.0);
    context.begin_path();
    let mut x = start_x;
    context.move_to(x, y);
    let end_x = start_x + width;
    let mut up = true;
    while x < end_x {
        let next_x = (x + wavelength * 0.5).min(end_x);
        let wave_y = if up { y - amplitude } else { y + amplitude };
        context.line_to(next_x, wave_y);
        up = !up;
        x = next_x;
    }
    context.stroke();
}

/// Executes a squiggly ("wavy") underline — the lint / spellcheck
/// decoration pattern. Draws a sequence of short diagonal strokes that
/// alternate above and below an underline baseline, with wavelength
/// and amplitude proportional to the font size.
///
/// Shared helper reused by the plugin decoration pass so sync and
/// cache-replay rendering agree byte-for-byte.
pub(crate) fn execute_squiggly_underline<P: Painter>(
    context: &P,
    start_x: f64,
    baseline_y: f64,
    width: f64,
    font_size_pt: f32,
    color: &str,
) -> Result<(), P::Error> {
    if width <= 0.0 {
        return Ok(());
    }
    let font_px = points_to_px(font_size_pt);
    // Offset below baseline roughly matches a spellchecker underline.
    let baseline = baseline_y + font_px * 0.20;
    // Amplitude and wavelength scale with font size so the wave stays
    // visually consistent across heading and body text.
    let amplitude = (font_px * 0.10).clamp(1.0, 3.0);
    let wavelength = (font_px * 0.30).clamp(3.0, 6.0);
    let line_width = (font_px * 0.05).max(1.0);

    context.save();
    context.set_stroke_style_str(color);
    context.set_line_width(line_width);
    context.begin_path();
    let mut x = start_x;
    context.move_to(x, baseline);
    let end_x = start_x + width;
    let mut up = true;
    while x < end_x {
        let next_x = (x + wavelength * 0.5).min(end_x);
        let y = if up {
            baseline - amplitude
        } else {
            baseline + amplitude
        };
        context.line_to(next_x, y);
        up = !up;
        x = next_x;
    }
    context.line_to(end_x, baseline);
    context.stroke();
    context.restore();
    Ok(())
}

/// Executes a single emphasis mark (`w:em`, ECMA-376 Part 1 §17.3.2.12) — a
/// small circle centered at (`x`, `y`). A filled mark is a solid dot
/// (dot/comma/underDot); an unfilled mark is an open ring (circle).
fn execute_emphasis_mark<P: Painter>(
    context: &P,
    x: f64,
    y: f64,
    radius: f64,
    filled: bool,
    color: &str,
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    if radius <= 0.0 {
        return Ok(());
    }
    let two_pi = std::f64::consts::PI * 2.0;
    if filled {
        context.set_fill_style_str(color);
        context.begin_path();
        context.arc(x, y, radius, 0.0, two_pi)?;
        context.fill();
        font_state.invalidate_fill_style();
    } else {
        context.save();
        context.set_stroke_style_str(color);
        context.set_line_width((radius * 0.5).max(1.0));
        context.begin_path();
        context.arc(x, y, radius, 0.0, two_pi)?;
        context.stroke();
        context.restore();
    }
    Ok(())
}

fn execute_tab_leader_with_font_state<P: Painter>(
    context: &P,
    baseline: f64,
    leader: TabLeaderStyle,
    style: &TextStyle,
    glyph_positions: &[f64],
    font_state: &mut FontStateTracker,
) -> Result<(), P::Error> {
    if matches!(leader, TabLeaderStyle::None) {
        return Ok(());
    }

    let character = leader.character();
    if character.is_empty() {
        return Ok(());
    }

    font_state.set_fill_style_if_changed(context, &style.fill_color);
    let font = &style.font_descriptor_latin_ascii;
    font_state.set_font_if_changed(context, font);

    for &x in glyph_positions {
        context.fill_text(character, x, baseline + style.baseline_shift_px)?;
    }

    Ok(())
}

/// Executes control mark drawing.
#[allow(dead_code)]
fn execute_control_mark<P: Painter>(
    context: &P,
    symbol: &str,
    x: f64,
    baseline: f64,
    style: &TextStyle,
) -> Result<f64, P::Error> {
    let mut font_state = FontStateTracker::new();
    execute_control_mark_with_font_state(context, symbol, x, baseline, style, &mut font_state)
}

fn execute_control_mark_with_font_state<P: Painter>(
    context: &P,
    symbol: &str,
    x: f64,
    baseline: f64,
    style: &TextStyle,
    font_state: &mut FontStateTracker,
) -> Result<f64, P::Error> {
    if symbol.is_empty() {
        return Ok(0.0);
    }

    const CONTROL_MARK_FONT_SCALE: f64 = 0.75;

    font_state.set_fill_style_if_changed(context, &style.fill_color);

    let scaled_font_size_pt = (style.font_size_pt as f64 * CONTROL_MARK_FONT_SCALE).max(1.0) as f32;
    let scaled_descriptor = format!(
        "{:0.2}pt {}",
        scaled_font_size_pt,
        extract_font_family(&style.font_descriptor_latin_ascii)
    );
    font_state.set_font_if_changed(context, &scaled_descriptor);

    let reduction_points = style.font_size_pt * ((1.0 - CONTROL_MARK_FONT_SCALE) * 0.5) as f32;
    let baseline_lift = points_to_px(reduction_points);
    let text_y = baseline + style.baseline_shift_px * CONTROL_MARK_FONT_SCALE - baseline_lift;

    // Measure first (with caching), then draw
    let width = font_state.measure_text_cached(context, symbol, &scaled_descriptor)?;
    context.fill_text(symbol, x, text_y)?;

    Ok(width)
}

/// Extracts font family from a font descriptor string.
fn extract_font_family(descriptor: &str) -> &str {
    // Font descriptor format: "12pt 'Times New Roman', serif"
    // We need to extract "'Times New Roman', serif"
    if let Some(idx) = descriptor.find("pt ") {
        &descriptor[idx + 3..]
    } else {
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_script_latin() {
        assert_eq!(classify_script("a"), ScriptCategory::Latin);
        assert_eq!(classify_script("A"), ScriptCategory::Latin);
        assert_eq!(classify_script("1"), ScriptCategory::Latin);
    }

    #[test]
    fn test_classify_script_east_asia() {
        assert_eq!(classify_script("あ"), ScriptCategory::EastAsia);
        assert_eq!(classify_script("漢"), ScriptCategory::EastAsia);
        assert_eq!(classify_script("カ"), ScriptCategory::EastAsia);
        assert_eq!(classify_script("。"), ScriptCategory::EastAsia);
    }

    #[test]
    fn test_classify_script_complex() {
        assert_eq!(classify_script("ا"), ScriptCategory::Complex);
        assert_eq!(classify_script("א"), ScriptCategory::Complex);
    }

    #[test]
    fn test_extract_font_family() {
        assert_eq!(
            extract_font_family("12pt 'Times New Roman', serif"),
            "'Times New Roman', serif"
        );
        assert_eq!(extract_font_family("10.5pt Arial"), "Arial");
    }

    #[test]
    fn test_points_to_px() {
        assert!((points_to_px(72.0) - 96.0).abs() < 0.001);
        assert!((points_to_px(12.0) - 16.0).abs() < 0.001);
    }

    #[test]
    fn test_text_style_font_for_script() {
        let style = TextStyle {
            font_size_pt: 12.0,
            letter_spacing_px: 0.0,
            text_scale_x: 1.0,
            enable_pair_kerning: false,
            baseline_shift_px: 0.0,
            strike: false,
            underline: false,
            shadow: false,
            outline: false,
            emboss: false,
            imprint: false,
            effects: None,
            font_descriptor_latin_ascii: Rc::from("12pt Arial"),
            font_descriptor_latin_hansi: Rc::from("12pt Arial Unicode"),
            font_descriptor_east_asia: Rc::from("12pt 'MS Mincho'"),
            font_descriptor_complex: Rc::from("12pt 'Times New Roman'"),
            fill_color: Rc::from("#000000"),
            highlight_color: None,
            font_hint: None,
        };

        assert_eq!(style.font_for_script(ScriptCategory::Latin), "12pt Arial");
        assert_eq!(
            style.font_for_script(ScriptCategory::EastAsia),
            "12pt 'MS Mincho'"
        );
        assert_eq!(
            style.font_for_script(ScriptCategory::Complex),
            "12pt 'Times New Roman'"
        );
    }

    #[test]
    fn test_text_style_font_for_grapheme_uses_hansi() {
        let style = TextStyle {
            font_size_pt: 12.0,
            letter_spacing_px: 0.0,
            text_scale_x: 1.0,
            enable_pair_kerning: false,
            baseline_shift_px: 0.0,
            strike: false,
            underline: false,
            shadow: false,
            outline: false,
            emboss: false,
            imprint: false,
            effects: None,
            font_descriptor_latin_ascii: Rc::from("12pt Ascii"),
            font_descriptor_latin_hansi: Rc::from("12pt HAnsi"),
            font_descriptor_east_asia: Rc::from("12pt EastAsia"),
            font_descriptor_complex: Rc::from("12pt Complex"),
            fill_color: Rc::from("#000000"),
            highlight_color: None,
            font_hint: None,
        };

        assert_eq!(
            style.font_for_grapheme("A", ScriptCategory::Latin),
            "12pt Ascii"
        );
        assert_eq!(
            style.font_for_grapheme("é", ScriptCategory::Latin),
            "12pt HAnsi"
        );
    }

    #[test]
    fn test_text_style_font_for_grapheme_falls_back_to_ascii_when_hansi_empty() {
        let style = TextStyle {
            font_size_pt: 12.0,
            letter_spacing_px: 0.0,
            text_scale_x: 1.0,
            enable_pair_kerning: false,
            baseline_shift_px: 0.0,
            strike: false,
            underline: false,
            shadow: false,
            outline: false,
            emboss: false,
            imprint: false,
            effects: None,
            font_descriptor_latin_ascii: Rc::from("12pt Ascii"),
            font_descriptor_latin_hansi: Rc::from(""),
            font_descriptor_east_asia: Rc::from("12pt EastAsia"),
            font_descriptor_complex: Rc::from("12pt Complex"),
            fill_color: Rc::from("#000000"),
            highlight_color: None,
            font_hint: None,
        };

        assert_eq!(
            style.font_for_grapheme("é", ScriptCategory::Latin),
            "12pt Ascii"
        );
    }

    #[test]
    fn test_text_style_font_for_grapheme_applies_hint() {
        let style = TextStyle {
            font_size_pt: 12.0,
            letter_spacing_px: 0.0,
            text_scale_x: 1.0,
            enable_pair_kerning: false,
            baseline_shift_px: 0.0,
            strike: false,
            underline: false,
            shadow: false,
            outline: false,
            emboss: false,
            imprint: false,
            effects: None,
            font_descriptor_latin_ascii: Rc::from("12pt Ascii"),
            font_descriptor_latin_hansi: Rc::from("12pt HAnsi"),
            font_descriptor_east_asia: Rc::from("12pt EastAsia"),
            font_descriptor_complex: Rc::from("12pt Complex"),
            fill_color: Rc::from("#000000"),
            highlight_color: None,
            font_hint: Some(FontHint::EastAsia),
        };

        assert_eq!(
            style.font_for_grapheme("\u{201C}", ScriptCategory::Latin),
            "12pt EastAsia"
        );
        // Basic Latin keeps the ascii font whatever the hint says.
        assert_eq!(
            style.font_for_grapheme("A", ScriptCategory::Latin),
            "12pt Ascii"
        );
    }

    /// The `w:shadow` drop-shadow offset scales with the font size and is
    /// clamped so small text still casts a visible shadow.
    #[test]
    fn text_shadow_offset_scales_with_font_size_and_clamps() {
        // Clamped to 1px for small fonts (8pt * 0.08 = 0.64 < 1.0).
        assert!((text_shadow_offset_px(8.0) - 1.0).abs() < 1e-9);
        // Scales linearly for larger fonts.
        assert!((text_shadow_offset_px(48.0) - 48.0 * 0.08).abs() < 1e-9);
        // The clamp boundary sits at 12.5pt.
        assert!(text_shadow_offset_px(20.0) > 1.0);
    }

    /// The `w:outline` stroke width scales with the font size and is clamped so
    /// the hollow glyph outline stays visible at small sizes.
    #[test]
    fn outline_line_width_scales_with_font_size_and_clamps() {
        // Clamped to 1px for small fonts (10pt * 0.04 = 0.4 < 1.0).
        assert!((outline_line_width_px(10.0) - 1.0).abs() < 1e-9);
        // Scales linearly for larger fonts (40pt * 0.04 = 1.6).
        assert!((outline_line_width_px(40.0) - 40.0 * 0.04).abs() < 1e-9);
        // The clamp boundary sits at 25pt.
        assert!(outline_line_width_px(48.0) > 1.0);
    }

    /// The emboss/imprint highlight offset scales with the font size and is
    /// clamped so the raised/engraved effect stays visible at small sizes.
    #[test]
    fn emboss_shadow_offset_scales_with_font_size_and_clamps() {
        // Clamped to 1px for small fonts (10pt * 0.05 = 0.5 < 1.0).
        assert!((emboss_shadow_offset_px(10.0) - 1.0).abs() < 1e-9);
        // Scales linearly for larger fonts (40pt * 0.05 = 2.0).
        assert!((emboss_shadow_offset_px(40.0) - 40.0 * 0.05).abs() < 1e-9);
        // The clamp boundary sits at 20pt.
        assert!(emboss_shadow_offset_px(48.0) > 1.0);
    }

    #[test]
    fn text_style_requires_advanced_layout_when_text_scale_is_applied() {
        let style = TextStyle {
            text_scale_x: 1.1,
            ..TextStyle::default()
        };
        assert!(style.requires_advanced_layout());
    }

    #[test]
    fn text_style_requires_advanced_layout_when_pair_kerning_is_enabled() {
        let style = TextStyle {
            enable_pair_kerning: true,
            ..TextStyle::default()
        };
        assert!(style.requires_advanced_layout());
    }

    #[test]
    fn test_font_state_tracker_new() {
        let tracker = FontStateTracker::new();
        assert!(tracker.current().is_none());
    }

    #[test]
    fn test_font_state_tracker_tracks_font() {
        let mut tracker = FontStateTracker::new();

        // Manually set font (simulating set_font_if_changed without context)
        tracker.current_font = Some(Rc::from("12pt Arial"));
        tracker.current_font_hash = Some(FontStateTracker::compute_font_hash("12pt Arial"));

        assert_eq!(tracker.current(), Some("12pt Arial"));
    }

    #[test]
    fn test_font_state_tracker_invalidate() {
        let mut tracker = FontStateTracker::new();
        tracker.current_font = Some(Rc::from("12pt Arial"));
        tracker.current_font_hash = Some(FontStateTracker::compute_font_hash("12pt Arial"));

        tracker.invalidate();

        assert!(tracker.current().is_none());
        assert!(tracker.current_font_hash.is_none());
    }

    #[test]
    fn test_font_state_tracker_detects_change() {
        let mut tracker = FontStateTracker::new();

        // First font should be "changed" (from None)
        tracker.current_font = None;
        let needs_change1 = tracker.current_font.as_deref() != Some("12pt Arial");
        assert!(needs_change1);

        // Set to "12pt Arial"
        tracker.current_font = Some(Rc::from("12pt Arial"));
        tracker.current_font_hash = Some(FontStateTracker::compute_font_hash("12pt Arial"));

        // Same font should not need change
        let needs_change2 = tracker.current_font.as_deref() != Some("12pt Arial");
        assert!(!needs_change2);

        // Different font should need change
        let needs_change3 = tracker.current_font.as_deref() != Some("14pt Arial");
        assert!(needs_change3);
    }

    #[test]
    fn test_compute_cache_key_from_font_hash_matches_full_key() {
        let font = "12pt 'Times New Roman', serif";
        let text = "詳細";
        let font_hash = FontStateTracker::compute_font_hash(font);
        assert_eq!(
            FontStateTracker::compute_cache_key(font, text),
            FontStateTracker::compute_cache_key_from_font_hash(font_hash, text)
        );
    }

    #[test]
    fn test_pack_short_text_key_distinguishes_common_inputs() {
        assert_ne!(
            FontStateTracker::pack_short_text_key("A"),
            FontStateTracker::pack_short_text_key("B")
        );
        assert_ne!(
            FontStateTracker::pack_short_text_key("A"),
            FontStateTracker::pack_short_text_key("AA")
        );
        assert_eq!(
            FontStateTracker::pack_short_text_key("詳細"),
            FontStateTracker::pack_short_text_key("詳細")
        );
        assert_eq!(FontStateTracker::pack_short_text_key("123456789"), None);
    }

    #[test]
    #[ignore]
    fn benchmark_font_cache_key_fast_path() {
        use std::time::Instant;

        let font = "12pt 'Times New Roman', serif";
        let text = "詳細";
        let iterations = 500_000;
        let font_hash = FontStateTracker::compute_font_hash(font);
        let packed_text = FontStateTracker::pack_short_text_key(text).expect("short text");

        let old_started = Instant::now();
        let mut old_acc = 0_u64;
        for _ in 0..iterations {
            old_acc ^= FontStateTracker::compute_cache_key(font, text);
        }
        let old_elapsed = old_started.elapsed();

        let new_started = Instant::now();
        let mut new_acc = 0_u64;
        for _ in 0..iterations {
            new_acc ^= FontStateTracker::compute_cache_key_from_font_hash(font_hash, text);
        }
        let new_elapsed = new_started.elapsed();

        let packed_started = Instant::now();
        let mut packed_acc = 0_u64;
        for _ in 0..iterations {
            packed_acc ^=
                font_hash.rotate_left(7) ^ packed_text.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        let packed_elapsed = packed_started.elapsed();

        assert_eq!(new_acc, packed_acc);
        assert_eq!(old_acc, new_acc);
        println!(
            "font cache key benchmark: old={}us new={}us packed={}us iterations={}",
            old_elapsed.as_micros(),
            new_elapsed.as_micros(),
            packed_elapsed.as_micros(),
            iterations
        );
    }
}

#[cfg(test)]
mod painter_backend_tests {
    use super::*;
    use crate::draw_command::{GlyphRunStep, TextStyle};
    use jade_painter::recording::{Op, RecordingPainter};
    use std::rc::Rc;

    fn style() -> TextStyle {
        TextStyle {
            font_size_pt: 10.5,
            font_descriptor_east_asia: Rc::from("10.50pt 'Noto Sans JP'"),
            fill_color: Rc::from("#000000"),
            ..TextStyle::default()
        }
    }

    /// The executor used to require a live `CanvasRenderingContext2d`, so none
    /// of it could run outside a browser. Going through `Painter` means the
    /// draw-command path is now exercised natively.
    #[test]
    fn commands_execute_against_a_non_canvas_backend() {
        let painter = RecordingPainter::new();

        execute_commands(
            &painter,
            &[
                DrawCommand::FillRect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                    color: Rc::from("#ff0000"),
                },
                DrawCommand::Text {
                    text: Rc::from("横浜"),
                    x: 10.0,
                    y: 20.0,
                    style: style(),
                },
            ],
        )
        .expect("recording backend is infallible");

        let ops = painter.ops();
        assert!(ops.contains(&Op::FillRect(1.0, 2.0, 3.0, 4.0)), "{ops:?}");
        assert!(
            ops.contains(&Op::SetFillStyle("#ff0000".to_string())),
            "{ops:?}"
        );
        assert!(
            ops.contains(&Op::FillText("横浜".to_string(), 10.0, 20.0)),
            "{ops:?}"
        );
    }

    /// Backends without the batched glyph fast path must still paint every
    /// glyph, via the same fallback the canvas backend uses when it declines.
    #[test]
    fn glyph_runs_fall_back_to_per_glyph_drawing_without_a_batch_path() {
        let painter = RecordingPainter::new();
        let text = "横浜市市税条例";
        let glyphs: Vec<GlyphRunStep> = text
            .char_indices()
            .enumerate()
            .map(|(index, (offset, ch))| GlyphRunStep {
                text_start: offset as u32,
                text_end: (offset + ch.len_utf8()) as u32,
                font_index: 0,
                x_offset: index as f64 * 14.0,
            })
            .collect();
        let glyph_count = glyphs.len();

        execute_commands(
            &painter,
            &[DrawCommand::TextRun {
                text: Rc::from(text),
                x: 0.0,
                y: 100.0,
                style: style(),
                fonts: vec![Rc::from("10.50pt 'Noto Sans JP'")],
                glyphs,
            }],
        )
        .expect("recording backend is infallible");

        assert_eq!(
            painter.count(|op| matches!(op, Op::FillText(..))),
            glyph_count,
            "every glyph should be painted individually: {:?}",
            painter.ops()
        );
    }
}

#[cfg(test)]
mod underline_kind_tests {
    use super::underline_dash_pattern;

    /// Word's dashed underline family must produce distinct, gap-bearing
    /// patterns while single/double/wave stroke solid lines.
    #[test]
    fn underline_dash_patterns_cover_the_word_kinds() {
        for kind in ["single", "double", "thick", "wave", "wavyDouble", "words"] {
            assert!(underline_dash_pattern(kind, 1.0).is_none(), "{kind}");
        }
        for kind in [
            "dotted",
            "dottedHeavy",
            "dash",
            "dashedHeavy",
            "dashLong",
            "dashLongHeavy",
            "dotDash",
            "dashDotHeavy",
            "dotDotDash",
            "dashDotDotHeavy",
        ] {
            let pattern = underline_dash_pattern(kind, 2.0).expect(kind);
            assert!(!pattern.is_empty(), "{kind}");
            assert!(pattern.iter().all(|len| *len > 0.0), "{kind}");
        }
        assert!(
            underline_dash_pattern("dotDotDash", 1.0).unwrap().len()
                > underline_dash_pattern("dotted", 1.0).unwrap().len()
        );
    }
}
