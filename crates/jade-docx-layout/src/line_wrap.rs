//! Shared Word-compatible inline layout and line wrapping.
//!
//! # Compatibility history
//!
//! A Japanese trust-beneficiary-sale agreement exposed several inputs that
//! look interchangeable after style resolution but are not interchangeable to
//! Word: `w:rFonts/@w:hint`, `w:lang/@w:eastAsia`, an omitted
//! `w:overflowPunct`, paragraph indents, and default tab stops.
//! Collapsing any of those inputs into a single computed font or paragraph
//! style shifted later page breaks (notably Articles 9, 14, and 15).
//!
//! This module therefore keeps those OOXML inputs separate until the shared
//! advance/wrap pass. In particular:
//!
//! - a directly authored `w:rFonts` override retains enough provenance to
//!   distinguish an omitted/default hint from an inherited East-Asian hint;
//! - hanging-punctuation eligibility is language-specific and distinct from
//!   the broader kinsoku line-start table;
//! - paragraph-terminal punctuation remains inside the paragraph text extent;
//! - explicit/default tab positions use the containing text margin as their
//!   origin, while the implicit hanging-indent tab remains at the indent; and
//! - completed lines are remeasured before pagination so renderer-only width
//!   corrections cannot create a second wrapping implementation.
//!
//! The anonymized trust-agreement fixture and the focused unit tests named in
//! the relevant item documentation are the regression oracle. The editor's
//! default Word-parity path filters `w:lastRenderedPageBreak` before calling
//! this algorithm: those markers record a producer's previous rendering,
//! whereas Jade must recompute layout from the current OOXML properties and
//! available fonts. This wrapper still honors an explicit page-break control
//! supplied by callers; it does not try to guess whether a saved rendered
//! marker is stale or should be reset from a changed Y coordinate.

use crate::compact_string::CompactString;
use crate::constants::{
    self, BALANCED_SBCS_EXTRA_RATIO, DEFAULT_NO_LINE_BREAK_AFTER, DEFAULT_NO_LINE_BREAK_BEFORE,
    DEFAULT_TAB_WIDTH_PT, IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_DEFAULT,
    IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_EXPLICIT, IMPLICIT_NUMERIC_GAP_SIDE_RATIO_DEFAULT_ENABLED,
    IMPLICIT_NUMERIC_GAP_SIDE_RATIO_EXPLICIT_AUTO_DN, WRAP_SAFETY_PX,
};
use crate::font_resolver::{ResolvedFontSlot, ResolvedRunStyle};
#[cfg(test)]
use crate::line_metrics::{resolve_widow_orphan_decision, WidowOrphanDecision};
use crate::units::points_to_px;
use jade_docx_reader::document::{
    BorderEdge, DropCap, DropCapKind, EastAsianLineBreakingSettings, FontHint, Indent,
    RevisionMark, RunHyperlink, RunProperties, TabStop, WrapTextSide, PAGE_BREAK_STR,
};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use unicode_bidi::{bidi_class, BidiClass, BidiInfo, Level};
use unicode_script::{Script as UnicodeScriptKind, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub use crate::constants::EAST_ASIA_ADVANCE_SCALE;

/// Whether a Word paragraph-justification token distributes the line width.
pub fn is_justified_alignment(alignment: &str) -> bool {
    // Compared in place: the wrap loop asks this for overflowing graphemes, and
    // lowercasing into a new string allocated on every call.
    let alignment = alignment.trim();
    [
        "both",
        "justify",
        "distribute",
        "thaidistribute",
        "lowkashida",
        "mediumkashida",
        "highkashida",
    ]
    .iter()
    .any(|justified| alignment.eq_ignore_ascii_case(justified))
}

/// Maximum fraction of an ordinary word space that Word may remove while
/// choosing a line for justified Latin text.
const JUSTIFIED_WORD_SPACE_SHRINK_RATIO: f64 = 1.0 / 6.0;

fn justified_word_space_shrink_ratio(style: &ParagraphStyle) -> f64 {
    if style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac {
        // The original Yokohama text-box PDF contracts its five regular
        // Yu Mincho spaces by 17.4% of the source-size natural advance,
        // including at ParagraphEnd. One sixth cannot admit that line.
        // Keep a bounded fifth for Mac; only the necessary amount is removed.
        0.2
    } else {
        JUSTIFIED_WORD_SPACE_SHRINK_RATIO
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalParagraphAlignment {
    Left,
    Center,
    Right,
    Justify,
}

/// Resolves logical `start` / `end` alignment against the paragraph base
/// direction while leaving Word's physical `left` / `right` tokens unchanged.
pub fn resolve_physical_paragraph_alignment(
    alignment: &str,
    bidi: bool,
) -> PhysicalParagraphAlignment {
    let alignment = alignment.trim().to_ascii_lowercase();
    if is_justified_alignment(&alignment) {
        PhysicalParagraphAlignment::Justify
    } else {
        match alignment.as_str() {
            "center" => PhysicalParagraphAlignment::Center,
            "right" => PhysicalParagraphAlignment::Right,
            "start" if bidi => PhysicalParagraphAlignment::Right,
            "end" if !bidi => PhysicalParagraphAlignment::Right,
            _ => PhysicalParagraphAlignment::Left,
        }
    }
}

/// Returns per-boundary numeric auto-spacing parameters for `autoSpaceDN`.
///
/// - `ratio`: scales glyph width contribution
/// - `min_px`: lower bound in CSS px
pub fn numeric_auto_spacing_params(auto_space_dn: Option<bool>) -> (f64, f64) {
    if auto_space_dn == Some(true) {
        (
            IMPLICIT_NUMERIC_GAP_SIDE_RATIO_EXPLICIT_AUTO_DN,
            IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_EXPLICIT,
        )
    } else {
        (
            IMPLICIT_NUMERIC_GAP_SIDE_RATIO_DEFAULT_ENABLED,
            IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_DEFAULT,
        )
    }
}

/// High-level script categories used to select font descriptors and line-breaking heuristics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptCategory {
    Latin,
    EastAsia,
    Complex,
}

/// Alignment mode for paragraph tab stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAlignment {
    Left,
    Center,
    Right,
    Decimal,
    Bar,
}

/// Visual leader style rendered between tab stop gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabLeader {
    None,
    Dot,
    Hyphen,
    Underscore,
    Heavy,
    MiddleDot,
}

impl TabLeader {
    pub fn character(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Dot => ".",
            Self::Hyphen => "-",
            Self::Underscore => "_",
            Self::Heavy => "—",
            Self::MiddleDot => "·",
        }
    }
}

/// Represents a paragraph-level tab stop with alignment and leader hints.
#[derive(Clone, Debug)]
pub struct ParagraphTabStop {
    pub position_pt: f32,
    pub alignment: TabAlignment,
    pub leader: TabLeader,
}

/// Paragraph layout configuration used during line wrapping.
#[derive(Clone, Debug)]
pub struct ParagraphStyle {
    /// Which Word the paragraph is laid out for, read once from
    /// `line_metrics::word_layout_platform` when the style is made so per-glyph
    /// rules do not consult the thread-local platform for every grapheme.
    pub word_platform: crate::line_metrics::WordLayoutPlatform,
    pub alignment: String,
    /// Paragraph base direction from `w:bidi`. The wrapper owns visual-order
    /// resolution so renderers consume the same cluster order as selection.
    pub bidi: bool,
    pub first_line_indent: f64,
    pub first_line_available_adjustment: f64,
    pub continuation_available_adjustment: f64,
    pub right_indent: f64,
    pub tab_stops: Vec<ParagraphTabStop>,
    pub default_tab_width: f64,
    /// Offset from the indented paragraph base back to the containing text
    /// margin.
    ///
    /// OOXML explicit tab positions and the default tab interval are measured
    /// from that containing leading edge; applying `w:ind/@w:left` therefore
    /// moves the paragraph cursor but must not translate the tab grid by the
    /// same amount. The implicit stop created by `w:ind/@w:hanging` is the
    /// exception and remains at the indented paragraph base.
    ///
    /// This distinction fixed the `(10)\t真正譲渡` heading in the anonymized
    /// trust-agreement fixture, where treating the paragraph indent as the
    /// default-grid origin displaced the heading. It is asserted by
    /// `trust_true_sale_default_tab_grid_starts_at_text_margin_not_paragraph_indent`
    /// and the corresponding Playwright pagination test.
    pub tab_origin_offset: f64,
    /// Whether a negative first-line indent contributes the implicit custom
    /// tab stop required by OOXML `w:ind/@w:hanging`.
    pub use_hanging_indent_tab_stop: bool,
    pub east_asian_rules: EastAsianLineBreakingRules,
    pub enable_line_break_rules: bool,
    pub doc_grid_snap_to_chars: bool,
    pub doc_grid_char_space_px: Option<f64>,
    pub oracle_advance_px_by_text_offset: Option<Rc<FxHashMap<usize, f64>>>,
    /// Initial authored ASCII space in a note paragraph, immediately after a
    /// generated mark or at byte zero on subsequent paragraphs.
    /// This byte offset preserves the note text and editing coordinates.
    pub automatic_note_separator_offset: Option<usize>,
    /// Logical UTF-8 offsets of authored ASCII spaces after inline note marks.
    pub inline_note_separator_offsets: Vec<usize>,
    pub east_asian_advance_scale: f64,
    /// Whether glyphs set in a face Word's metrics are known for take Word's
    /// line box ascent/descent (`line_metrics::word_font_vertical_metrics_px`)
    /// rather than the drawing face's ink extents. Callers enable it where
    /// `line_metrics::word_east_asian_font_metrics_apply`.
    pub word_east_asian_font_line_box: bool,
    pub word_grid_line_sizing: Option<crate::line_metrics::WordGridLineSizing>,
    pub word_proportional_line_sizing: Option<crate::line_metrics::WordProportionalLineSizing>,
    /// Prepared first-letter drop-cap geometry. Its occupied width is consumed
    /// by this shared wrapper for every affected line; renderers must not
    /// independently recalculate wrap boundaries.
    pub drop_cap: Option<DropCapLayout>,
    /// Per-line horizontal segments planned around floating objects. The
    /// renderer consumes the same plan after wrapping so glyphs, selection,
    /// and caret hit-testing stay in one coordinate system.
    pub float_line_segments: Vec<FloatLineSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropCapLayout {
    pub kind: DropCapKind,
    pub lines: usize,
    pub grapheme_start: usize,
    pub grapheme_end: usize,
    pub width: f64,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            word_platform: crate::line_metrics::word_layout_platform(),
            alignment: String::new(),
            bidi: false,
            first_line_indent: 0.0,
            first_line_available_adjustment: 0.0,
            continuation_available_adjustment: 0.0,
            right_indent: 0.0,
            tab_stops: Vec::new(),
            default_tab_width: points_to_px(DEFAULT_TAB_WIDTH_PT),
            tab_origin_offset: 0.0,
            use_hanging_indent_tab_stop: true,
            east_asian_rules: EastAsianLineBreakingRules::default(),
            enable_line_break_rules: false,
            doc_grid_snap_to_chars: false,
            doc_grid_char_space_px: None,
            oracle_advance_px_by_text_offset: None,
            automatic_note_separator_offset: None,
            inline_note_separator_offsets: Vec::new(),
            east_asian_advance_scale: 1.0,
            word_east_asian_font_line_box: false,
            word_grid_line_sizing: None,
            word_proportional_line_sizing: None,
            drop_cap: None,
            float_line_segments: Vec::new(),
        }
    }
}

impl ParagraphStyle {
    /// Generated numbering retains grid and boundary spacing, but Word does
    /// not widen its ASCII digits/symbols with the body's balanced-SBCS rule.
    pub fn for_numbering_marker(&self) -> Cow<'_, Self> {
        if !self.enable_line_break_rules
            || !self.east_asian_rules.balance_single_byte_double_byte_width
            || !crate::line_metrics::word_widens_balanced_single_byte_glyphs(self.word_platform)
        {
            return Cow::Borrowed(self);
        }
        let mut marker = self.clone();
        marker
            .east_asian_rules
            .balance_single_byte_double_byte_width = false;
        Cow::Owned(marker)
    }

    /// `linesAndChars` adds character pitch to proportional advances;
    /// `snapToChars` instead places text into character-grid cells.
    pub fn apply_doc_grid_character_settings(
        &mut self,
        grid: Option<&jade_docx_reader::DocGrid>,
        snap_to_grid: bool,
    ) {
        self.doc_grid_snap_to_chars = false;
        self.doc_grid_char_space_px = None;
        let Some(grid) = grid.filter(|grid| {
            snap_to_grid && crate::line_metrics::doc_grid_has_active_char_grid(Some(grid))
        }) else {
            return;
        };
        self.doc_grid_snap_to_chars = grid
            .grid_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("snapToChars"));
        self.doc_grid_char_space_px = grid.char_space.map(points_to_px);
    }
}

/// Returns the single `char` of `text` when it is exactly one code point.
///
/// Used by the kinsoku hot path: the default break tables hold only single
/// code points, so a multi-code-point grapheme can never be a member and is
/// rejected without scanning the table.
#[inline]
/// Whether `set` lists `character`.
///
/// `str::contains(char)` runs a substring search for any non-ASCII character,
/// building a Two-Way searcher on every call. For the few dozen characters of
/// the punctuation tables this was a visible share of wrapping Japanese text,
/// where it runs once per grapheme; a plain scan is much cheaper.
fn char_set_contains(set: &str, character: char) -> bool {
    set.chars().any(|candidate| candidate == character)
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(first)
}

/// MS-OE376 §2.1.56で定義される、`w:overflowPunct`の言語別固定テーブル。
///
/// 文書や段落の`w:overflowPunct`で変更できるのは、句読点のはみ出し処理を
/// 有効にするかどうかだけであり、この文字集合自体はOOXMLから上書きできない。
/// 適用するテーブルは`w:lang/@w:eastAsia`から選択する。
///
/// `w:noLineBreaksBefore`および`w:noLineBreaksAfter`で変更できる行頭・行末の
/// 禁則文字集合とは別の規則である。たとえば日本語ではASCIIの`)`を行頭に
/// 置けない場合でも、`)`が行末からはみ出せるとは限らない。そのため、禁則文字の
/// カスタマイズ結果をこのテーブルへ合成してはならない。
const WORD_OVERFLOW_PUNCT_JAPANESE: &str = ",.’”、。」』〗），．］｝｡､";
/// 簡体字中国語の`w:overflowPunct`固定テーブル。
const WORD_OVERFLOW_PUNCT_CHINESE_SIMPLIFIED: &str =
    "!%),.:;>?]}¢°·ˇ’”‰′″℃∶、。〃〉》」』〗〕〞﹚﹜﹞！＂％＇），．：；？］｝￠";
/// 繁体字中国語の`w:overflowPunct`固定テーブル。
const WORD_OVERFLOW_PUNCT_CHINESE_TRADITIONAL: &str =
    "!),.:;?]}’”′、。〉》」』〗〕〞﹚﹜﹞！），．：；？］｝";
/// 韓国語の`w:overflowPunct`固定テーブル。
const WORD_OVERFLOW_PUNCT_KOREAN: &str = "!%),.:;?]}¢°’”′″℃〉》」』〗〕！％），．：；？］｝￠";

/// Returns Word's `w:overflowPunct` table for the run's East-Asian editing
/// language.
///
/// MS-OE376 §2.1.56 defines language-specific overflow character sets. They
/// are intentionally narrower than the kinsoku line-start prohibition table:
/// a character may be forbidden at line start without being eligible to hang
/// beyond the right text extent. `w:lang/@w:eastAsia` takes precedence over
/// `w:lang/@w:val`. When neither identifies Japanese, Chinese, or Korean, Word
/// does not select any of the language tables; using their union would enable
/// hanging punctuation in language-neutral runs that Word leaves inside the
/// text extent.
///
/// Font-slot hints are deliberately not consulted here. The trust-agreement
/// regression contains Japanese punctuation whose font hint differs from its
/// East-Asian editing language; Word still applies the Japanese overflow set.
/// See `explicit_default_font_hint_does_not_disable_japanese_overflow_punctuation`.
fn word_overflow_punctuation_set(properties: Option<&RunProperties>) -> Option<&'static str> {
    let language = properties
        .and_then(|props| {
            props
                .east_asia_language
                .as_deref()
                .or(props.language.as_deref())
        })
        .unwrap_or_default();
    let language = language.as_bytes();
    let language_prefix = language.get(..2);
    let set = if language_prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"ja")) {
        WORD_OVERFLOW_PUNCT_JAPANESE
    } else if language_prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"ko")) {
        WORD_OVERFLOW_PUNCT_KOREAN
    } else if language.get(..5).is_some_and(|prefix| {
        prefix.eq_ignore_ascii_case(b"zh-tw")
            || prefix.eq_ignore_ascii_case(b"zh-hk")
            || prefix.eq_ignore_ascii_case(b"zh-mo")
    }) || language
        .windows(4)
        .any(|window| window.eq_ignore_ascii_case(b"hant"))
    {
        WORD_OVERFLOW_PUNCT_CHINESE_TRADITIONAL
    } else if language_prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"zh")) {
        WORD_OVERFLOW_PUNCT_CHINESE_SIMPLIFIED
    } else {
        return None;
    };
    Some(set)
}

impl ParagraphStyle {
    /// Absolute line origin after applying floating-object exclusions.
    pub fn line_base_x(&self, default_base_x: f64, line_index: usize) -> f64 {
        self.float_line_segments
            .get(line_index)
            .map(|segment| segment.start_x)
            .unwrap_or(default_base_x)
    }

    /// Width of the free horizontal segment before paragraph indents.
    pub fn line_base_width(&self, default_width: f64, line_index: usize) -> f64 {
        self.float_line_segments
            .get(line_index)
            .map(|segment| segment.width)
            .unwrap_or(default_width)
    }

    /// Whether this line continues the previous line's physical band on the
    /// far side of a `bothSides` object (same top, no line advance).
    pub fn line_shares_float_band(&self, line_index: usize) -> bool {
        line_index > 0
            && self
                .float_line_segments
                .get(line_index)
                .is_some_and(|segment| segment.shares_band_with_previous)
    }

    /// Index of the first wrapped line of every physical line band, for a
    /// paragraph wrapped into `line_count` lines. Lines that continue a band
    /// past a `bothSides` float share their band's entry, so the result has
    /// one entry per vertical line advance. Pagination (widow/orphan,
    /// `keepLines`, `keepNext`) counts and breaks by band, never inside one.
    pub fn float_band_starts(&self, line_count: usize) -> Vec<usize> {
        (0..line_count)
            .filter(|&line_index| !self.line_shares_float_band(line_index))
            .collect()
    }

    /// Number of full-width physical line bands skipped before this line.
    pub fn skipped_float_lines_before(&self, line_index: usize) -> usize {
        self.float_line_segments
            .get(line_index)
            .map(|segment| segment.skipped_lines_before)
            .unwrap_or(0)
    }

    /// Vertical advance consumed before a usable float segment.
    pub fn skipped_float_height_before(&self, line_index: usize, line_height: f64) -> f64 {
        self.float_line_segments
            .get(line_index)
            .map_or(0.0, |segment| {
                if segment.exact_clearance_height > 0.0 {
                    segment.exact_clearance_height
                } else {
                    segment.skipped_lines_before as f64 * line_height
                }
            })
    }

    /// Horizontal origin relative to the paragraph base for a wrapped line.
    /// A dropped cap reserves its width on the first `lines` body lines; a
    /// margin cap shifts only its enlarged first glyph into the start margin.
    pub fn line_indent(&self, line_index: usize) -> f64 {
        let mut indent = if line_index == 0 {
            self.first_line_indent
        } else {
            0.0
        };
        if let Some(drop_cap) = self.drop_cap {
            match drop_cap.kind {
                DropCapKind::Drop if line_index > 0 && line_index < drop_cap.lines => {
                    indent += drop_cap.width;
                }
                DropCapKind::Margin if line_index == 0 => {
                    indent -= drop_cap.width;
                }
                _ => {}
            }
        }
        indent
    }

    fn available_width_for_line(&self, base_available: f64, line_index: usize) -> f64 {
        let base_available = self.line_base_width(base_available, line_index);
        let adjustment = if line_index == 0 {
            -self.first_line_available_adjustment
        } else {
            self.continuation_available_adjustment
        };
        let mut available = base_available - self.line_indent(line_index) + adjustment;
        let residual_hanging_indent = self.first_line_indent < 0.0
            && -self.tab_origin_offset + self.first_line_indent > f64::EPSILON;
        if residual_hanging_indent && self.east_asian_advance_scale > 1.0 {
            if line_index == 0 {
                // A hanging first line that remains inset after the hanging
                // amount uses the unscaled text extent in Word. Keep the FE
                // fallback scale for glyph advances, but restore the same
                // proportional capacity to this first-line content box.
                available *= self.east_asian_advance_scale;
            } else {
                // Word reserves half of the hanging interval at the wrap edge
                // of continuation lines in this FE compatibility layout. This
                // is geometry-derived and applies independently of text/runs.
                available -= -self.first_line_indent * 0.5;
            }
        }
        if available.is_finite() && available > 0.0 {
            available
        } else {
            base_available
        }
    }

    pub fn kinsoku_enabled(&self) -> bool {
        self.enable_line_break_rules && self.east_asian_rules.kinsoku != Some(false)
    }

    pub fn allows_mid_word_break(&self) -> bool {
        if !self.enable_line_break_rules {
            return true;
        }
        // Word-compatible default: missing `w:wordWrap` behaves like ON
        // (mid-word wrapping is allowed). Only explicit false disables it.
        self.east_asian_rules.word_wrap != Some(false)
    }

    /// Returns whether the paragraph enables Word's single-character
    /// East-Asian punctuation overflow behavior.
    ///
    /// OOXML defines an omitted `w:overflowPunct` as enabled, so only an
    /// explicit false disables it. Eligibility of an individual character is
    /// still decided by `word_overflow_punctuation_set`; this method must
    /// not be widened into a general "all closing punctuation may hang" rule.
    pub fn allows_hanging_punctuation(&self) -> bool {
        // ECMA-376 §17.3.1.21: omitting `w:overflowPunct` is equivalent to
        // enabling it. Only an explicit false disables hanging punctuation.
        self.kinsoku_enabled() && self.east_asian_rules.overflow_punctuation != Some(false)
    }

    /// Whether a line ending with `break_kind` may hang its final punctuation:
    /// any soft line, and the paragraph's last line as
    /// [`Self::paragraph_end_punctuation_can_hang`] allows.
    pub fn allows_hanging_punctuation_at(&self, break_kind: LineBreakKind) -> bool {
        self.allows_hanging_punctuation()
            // Word for Mac's 2013 layout keeps punctuation inside ragged
            // lines, including soft continuations. Older layouts and
            // justified paragraphs retain their punctuation overflow.
            && (self.word_platform != crate::line_metrics::WordLayoutPlatform::Mac
                || self
                    .east_asian_rules
                    .compatibility_mode
                    .is_none_or(|mode| mode < 15)
                || is_justified_alignment(&self.alignment))
            && (break_kind != LineBreakKind::ParagraphEnd
                || self.paragraph_end_punctuation_can_hang())
    }

    /// Whether the paragraph's own final punctuation may hang past the right
    /// text extent: always under the pre-2013 layout
    /// ([`EastAsianLineBreakingRules::paragraph_end_punctuation_can_hang`]),
    /// and in a justified paragraph under the 2013 one. Word for Mac 16.113
    /// sets `system-dev-contract.docx` in mode 15 with `る。` wrapped, and the
    /// same paragraph justified with `…上限とする。` hanging, as it does
    /// `…を負うものとする。` in article 13 of `trust-beneficiary-sale-keep-next`.
    pub fn paragraph_end_punctuation_can_hang(&self) -> bool {
        self.east_asian_rules.paragraph_end_punctuation_can_hang()
            || is_justified_alignment(&self.alignment)
    }

    pub fn applies_top_line_punctuation_half_width(&self) -> bool {
        self.enable_line_break_rules && self.east_asian_rules.top_line_punctuation == Some(true)
    }

    pub fn forbids_line_start(&self, text: &str) -> bool {
        if !self.kinsoku_enabled() {
            return false;
        }
        let custom = &self.east_asian_rules.no_line_break_before;
        if !custom.is_empty() && custom.iter().any(|entry| entry == text) {
            return true;
        }
        !self.east_asian_rules.no_line_break_before_replaces_default
            && single_char(text).is_some_and(|c| {
                DEFAULT_NO_LINE_BREAK_BEFORE.contains(&c)
                    && (self.east_asian_rules.strict_first_and_last_chars
                        || !STRICT_ONLY_NO_LINE_BREAK_BEFORE.contains(&c))
            })
    }

    pub fn forbids_line_end(&self, text: &str) -> bool {
        if !self.kinsoku_enabled() {
            return false;
        }
        let custom = &self.east_asian_rules.no_line_break_after;
        if !custom.is_empty() && custom.iter().any(|entry| entry == text) {
            return true;
        }
        !self.east_asian_rules.no_line_break_after_replaces_default
            && single_char(text).is_some_and(|c| DEFAULT_NO_LINE_BREAK_AFTER.contains(&c))
    }
}

/// Members of the default line-start table that Word forbids only under
/// `w:strictFirstAndLastChars`: its standard set (標準) lets the small kana and
/// `ー` start a line (trust-beneficiary-sale-keep-next page 12, `ーシング`).
const STRICT_ONLY_NO_LINE_BREAK_BEFORE: &[char] = &[
    'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ', 'ょ', 'ゎ', 'ァ', 'ィ', 'ゥ', 'ェ', 'ォ', 'ッ',
    'ャ', 'ュ', 'ョ', 'ヮ', 'ヵ', 'ヶ', 'ー',
];

/// Splits the first printable grapheme into a synthetic enlarged run and
/// returns the exact geometry consumed by [`wrap_text_into_lines`]. Both the
/// headless layout extractor and canvas renderer call this helper, keeping font
/// measurement and per-line occupied width identical across those paths.
pub fn prepare_drop_cap_layout<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &mut Vec<RunStyle>,
    drop_cap: DropCap,
    line_height_px: f64,
) -> Result<Option<DropCapLayout>, C::Error> {
    let Some((grapheme_start, grapheme)) = text.grapheme_indices(true).find(|(_, grapheme)| {
        grapheme
            .chars()
            .any(|character| !character.is_whitespace() && !character.is_control())
    }) else {
        return Ok(None);
    };
    let grapheme_end = grapheme_start + grapheme.len();
    let Some(style_index) = run_styles.iter().position(|style| {
        style.inline_object.is_none() && style.start <= grapheme_start && style.end >= grapheme_end
    }) else {
        return Ok(None);
    };

    let original = run_styles.remove(style_index);
    let lines = drop_cap.lines.clamp(1, 10) as usize;
    let line_height_pt = (line_height_px * 72.0 / 96.0).max(1.0) as f32;
    let mut properties = original.properties.as_deref().cloned().unwrap_or_default();
    let body_font_size = properties.font_size.unwrap_or(line_height_pt / 1.15);
    properties.font_size = Some((body_font_size * lines as f32).max(body_font_size));
    let lower_by_pt = line_height_pt * lines.saturating_sub(1) as f32 * 0.8;
    properties.text_position = Some(properties.text_position.unwrap_or(0.0) - lower_by_pt);
    // CSS/Word keep a narrow gap between the enlarged glyph and body text.
    let gap_pt = properties.font_size.unwrap_or(body_font_size) * 0.05;
    properties.char_spacing = Some(properties.char_spacing.unwrap_or(0.0) + gap_pt);
    let properties = Some(Rc::new(properties));

    let resolved = ctx.resolve_run_style(properties.as_deref())?;
    let script = classify_script(grapheme);
    let (descriptor, descriptor_hash) = resolved.descriptor_and_hash_for_grapheme(grapheme, script);
    let measured = ctx.measure_text_with_hash(grapheme, descriptor, descriptor_hash)?;
    let width = (measured * resolved.text_scale_x + resolved.letter_spacing_px).max(1.0);

    let mut replacement = Vec::with_capacity(3);
    if original.start < grapheme_start {
        replacement.push(RunStyle {
            start: original.start,
            end: grapheme_start,
            properties: original.properties.clone(),
            revision: original.revision.clone(),
            hyperlink: original.hyperlink.clone(),
            inline_object: None,
        });
    }
    replacement.push(RunStyle {
        start: grapheme_start,
        end: grapheme_end,
        properties,
        revision: original.revision.clone(),
        hyperlink: original.hyperlink.clone(),
        inline_object: None,
    });
    if grapheme_end < original.end {
        replacement.push(RunStyle {
            start: grapheme_end,
            end: original.end,
            properties: original.properties,
            revision: original.revision,
            hyperlink: original.hyperlink,
            inline_object: None,
        });
    }
    run_styles.splice(style_index..style_index, replacement);

    Ok(Some(DropCapLayout {
        kind: drop_cap.kind,
        lines,
        grapheme_start,
        grapheme_end,
        width,
    }))
}

/// Custom East Asian line-breaking rules supplied by the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EastAsianLineBreakingRules {
    pub use_east_asian_layout: bool,
    pub kinsoku: Option<bool>,
    pub word_wrap: Option<bool>,
    pub overflow_punctuation: Option<bool>,
    pub top_line_punctuation: Option<bool>,
    pub balance_single_byte_double_byte_width: bool,
    pub auto_space_de: Option<bool>,
    pub auto_space_dn: Option<bool>,
    pub character_spacing_control: Option<String>,
    pub no_line_break_before: Vec<String>,
    pub no_line_break_after: Vec<String>,
    /// The custom sets above replace the default tables instead of adding to
    /// them (a Japanese `w:noLineBreaksBefore`/`After`, ECMA-376 §17.15.1.58-59).
    pub no_line_break_before_replaces_default: bool,
    pub no_line_break_after_replaces_default: bool,
    /// `w:strictFirstAndLastChars` (see [`STRICT_ONLY_NO_LINE_BREAK_BEFORE`]).
    pub strict_first_and_last_chars: bool,
    /// The document's `compatibilityMode` (see
    /// [`Self::paragraph_end_punctuation_can_hang`]).
    pub compatibility_mode: Option<u32>,
}

impl EastAsianLineBreakingRules {
    /// Whether the paragraph's own final punctuation may hang past the right
    /// text extent like an internal line's. Word 2013 layout
    /// (`compatibilityMode` 15) keeps it inside and wraps the preceding
    /// character with it (`trust_article_14_2_paragraph_terminal_period_does_not_hang`);
    /// Word 2007/2010 layout (12/14) hangs it. Word for Mac 16.113 renders
    /// `system-dev-contract.docx` (mode 12) with `…上限とする。` on one line and
    /// the same document switched to mode 15 with `る。` wrapped, in either Yu
    /// Mincho or MS Mincho. A justified paragraph hangs it in either layout
    /// ([`ParagraphStyle::paragraph_end_punctuation_can_hang`]).
    pub fn paragraph_end_punctuation_can_hang(&self) -> bool {
        self.compatibility_mode.is_some_and(|mode| mode < 15)
    }

    /// Copies the configured East Asian rules from `RunProperties`.
    pub fn from_settings(settings: &EastAsianLineBreakingSettings) -> Self {
        Self {
            use_east_asian_layout: settings.use_east_asian_layout,
            kinsoku: settings.kinsoku,
            word_wrap: settings.word_wrap,
            overflow_punctuation: settings.overflow_punctuation,
            top_line_punctuation: settings.top_line_punctuation,
            balance_single_byte_double_byte_width: settings.balance_single_byte_double_byte_width,
            auto_space_de: settings.auto_space_de,
            auto_space_dn: settings.auto_space_dn,
            character_spacing_control: settings.character_spacing_control.clone(),
            no_line_break_before: settings.no_line_break_before.clone(),
            no_line_break_after: settings.no_line_break_after.clone(),
            no_line_break_before_replaces_default: settings.no_line_break_before_replaces_default,
            no_line_break_after_replaces_default: settings.no_line_break_after_replaces_default,
            strict_first_and_last_chars: settings.strict_first_and_last_chars,
            compatibility_mode: settings.compatibility_mode,
        }
    }
}

/// Merge defaults with overrides provided by a paragraph/paragraph style settings.
pub fn merge_east_asian_rules(
    defaults: &EastAsianLineBreakingRules,
    overrides: Option<&EastAsianLineBreakingSettings>,
) -> EastAsianLineBreakingRules {
    let mut merged = defaults.clone();
    let Some(settings) = overrides else {
        return merged;
    };

    if settings.use_east_asian_layout {
        merged.use_east_asian_layout = true;
    }
    if settings.kinsoku.is_some() {
        merged.kinsoku = settings.kinsoku;
    }
    if settings.word_wrap.is_some() {
        merged.word_wrap = settings.word_wrap;
    }
    if settings.overflow_punctuation.is_some() {
        merged.overflow_punctuation = settings.overflow_punctuation;
    }
    if settings.top_line_punctuation.is_some() {
        merged.top_line_punctuation = settings.top_line_punctuation;
    }
    if settings.balance_single_byte_double_byte_width {
        merged.balance_single_byte_double_byte_width = true;
    }
    if settings.auto_space_de.is_some() {
        merged.auto_space_de = settings.auto_space_de;
    }
    if settings.auto_space_dn.is_some() {
        merged.auto_space_dn = settings.auto_space_dn;
    }
    if settings.character_spacing_control.is_some() {
        merged.character_spacing_control = settings.character_spacing_control.clone();
    }
    if !settings.no_line_break_before.is_empty() {
        merged.no_line_break_before = settings.no_line_break_before.clone();
        merged.no_line_break_before_replaces_default =
            settings.no_line_break_before_replaces_default;
    }
    if !settings.no_line_break_after.is_empty() {
        merged.no_line_break_after = settings.no_line_break_after.clone();
        merged.no_line_break_after_replaces_default = settings.no_line_break_after_replaces_default;
    }
    if settings.compatibility_mode.is_some() {
        merged.compatibility_mode = settings.compatibility_mode;
    }
    merged
}

fn resolve_snap_to_chars_cell_width<C: LineWrapContext>(
    ctx: &C,
    paragraph_style: &ParagraphStyle,
    east_asia_font_descriptor: &str,
    east_asia_font_descriptor_hash: u64,
    text_scale_x: f64,
    cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<Option<f64>, C::Error> {
    if !paragraph_style.doc_grid_snap_to_chars {
        return Ok(None);
    }

    let char_space_px = paragraph_style.doc_grid_char_space_px.unwrap_or(0.0);
    let cache_key = SnapToCharsCacheKey {
        east_asia_font_descriptor_hash,
        text_scale_x_bits: text_scale_x.to_bits(),
        char_space_px_bits: char_space_px.to_bits(),
    };
    if let Some(cached) = cache.get(&cache_key) {
        return Ok(Some(*cached));
    }

    let measured = match crate::line_metrics::word_monospace_em_px(east_asia_font_descriptor) {
        Some(em) => em,
        None => ctx.measure_text("あ", east_asia_font_descriptor)?,
    };
    let width = (measured * EAST_ASIA_ADVANCE_SCALE * text_scale_x + char_space_px).max(1.0);
    cache.insert(cache_key, width);
    Ok(Some(width))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SnapToCharsCacheKey {
    east_asia_font_descriptor_hash: u64,
    text_scale_x_bits: u64,
    char_space_px_bits: u64,
}

pub fn snap_to_chars_cell_span_adjustment(
    grapheme: &str,
    script: ScriptCategory,
    core_advance: f64,
    cell_width: f64,
) -> (f64, f64) {
    if !core_advance.is_finite()
        || !cell_width.is_finite()
        || core_advance <= 0.0
        || cell_width <= 0.0
    {
        return (0.0, 0.0);
    }
    if grapheme
        .chars()
        .all(|ch| ch.is_whitespace() || ch.is_control())
    {
        return (0.0, 0.0);
    }

    let span_cells = (core_advance / cell_width).ceil().max(1.0);
    let snapped_advance = span_cells * cell_width;
    let slack = (snapped_advance - core_advance).max(0.0);
    if slack <= f64::EPSILON {
        return (0.0, 0.0);
    }

    if script == ScriptCategory::EastAsia {
        (0.0, slack)
    } else {
        let leading = slack * 0.5;
        (leading, slack - leading)
    }
}

#[inline]
pub fn adjusted_glyph_advance_width(
    script: ScriptCategory,
    measured_width: f64,
    text_scale_x: f64,
) -> f64 {
    let scaled = measured_width * text_scale_x.max(0.01);
    if script == ScriptCategory::EastAsia {
        scaled * EAST_ASIA_ADVANCE_SCALE
    } else {
        scaled
    }
}

pub struct GraphemeAdvanceContext<'a> {
    pub paragraph_style: Option<&'a ParagraphStyle>,
    pub grapheme: &'a str,
    pub script: ScriptCategory,
    pub measured_glyph_width: f64,
    pub measured_width_is_oracle: bool,
    pub text_scale_x: f64,
    pub extra_spacing: f64,
    pub kerning_adjustment: f64,
    pub is_line_start: bool,
    pub prev_prev_grapheme: Option<&'a str>,
    pub prev_grapheme: Option<&'a str>,
    pub next_grapheme: Option<&'a str>,
    pub prev_glyph_width: Option<f64>,
    pub snap_cell_width: Option<f64>,
    /// The face the grapheme is drawn with: Word's auto-spacing follows its
    /// metrics where they are known (`line_metrics::word_auto_space_metrics`).
    pub font_descriptor: Option<&'a str>,
    /// [`crate::line_metrics::WordAutoSpace`] of the previous grapheme, when
    /// it is a digit set in a face with known metrics.
    pub prev_auto_space: Option<crate::line_metrics::WordAutoSpace>,
    /// The run's letter spacing, even where `extra_spacing` is zero for its
    /// last glyph: a character grid's auto spacing follows the tracked pitch.
    pub run_letter_spacing: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GraphemeAdvance {
    pub glyph_width: f64,
    pub adjusted_glyph_width: f64,
    pub implicit_latin_gap: f64,
    /// The auto spacing after the glyph, included in `glyph_box_width`.
    pub implicit_latin_trailing_gap: f64,
    pub implicit_numeric_gap: f64,
    pub kerning_adjustment: f64,
    pub snap_leading: f64,
    pub snap_trailing: f64,
    pub balance_extra: f64,
    pub core_advance: f64,
    pub glyph_box_width: f64,
    pub total_advance: f64,
    /// Auto-spacing metrics a digit leaves for the East Asian text right after
    /// it (`line_metrics::WordAutoSpace`); `None` anywhere else.
    pub trailing_auto_space: Option<crate::line_metrics::WordAutoSpace>,
}

pub fn compute_grapheme_advance(context: GraphemeAdvanceContext<'_>) -> GraphemeAdvance {
    let requested_ascii_width = if uses_requested_ascii_width(&context) {
        context
            .font_descriptor
            .and_then(crate::line_metrics::word_ascii_metrics)
            .and_then(|metrics| metrics.advance_px(context.grapheme.as_bytes()[0]))
    } else {
        None
    };
    compute_grapheme_advance_with_ascii_width(context, requested_ascii_width)
}

fn uses_requested_ascii_width(context: &GraphemeAdvanceContext<'_>) -> bool {
    !context.measured_width_is_oracle
        && context.script == ScriptCategory::Latin
        && context.grapheme.len() == 1
        && (b' '..=b'~').contains(&context.grapheme.as_bytes()[0])
        && context
            .paragraph_style
            .map_or_else(crate::line_metrics::word_layout_platform, |style| {
                style.word_platform
            })
            == crate::line_metrics::WordLayoutPlatform::Mac
}

fn compute_resolved_grapheme_advance(
    context: GraphemeAdvanceContext<'_>,
    style: &ResolvedRunStyle,
    requested_ascii_width_override: Option<f64>,
) -> GraphemeAdvance {
    let requested_ascii_width = if requested_ascii_width_override.is_some() {
        requested_ascii_width_override
    } else if uses_requested_ascii_width(&context) {
        style.ascii_advance_px(context.grapheme.as_bytes()[0])
    } else {
        None
    };
    compute_grapheme_advance_with_ascii_width(context, requested_ascii_width)
}

#[inline]
fn automatic_note_separator_width_px(
    grapheme: &str,
    paragraph_style: Option<&ParagraphStyle>,
    text_offset: Option<usize>,
    font_descriptor: &str,
) -> Option<f64> {
    if grapheme != " " {
        return None;
    }
    let paragraph = paragraph_style?;
    let offset = text_offset?;
    if paragraph.automatic_note_separator_offset != Some(offset)
        && paragraph
            .inline_note_separator_offsets
            .binary_search(&offset)
            .is_err()
    {
        return None;
    }
    crate::line_metrics::word_automatic_note_separator_advance_px(
        paragraph.word_platform,
        font_descriptor,
    )
}

fn compute_grapheme_advance_with_ascii_width(
    context: GraphemeAdvanceContext<'_>,
    requested_ascii_width: Option<f64>,
) -> GraphemeAdvance {
    let GraphemeAdvanceContext {
        paragraph_style,
        grapheme,
        script,
        measured_glyph_width,
        measured_width_is_oracle,
        text_scale_x,
        extra_spacing,
        kerning_adjustment,
        is_line_start,
        prev_prev_grapheme,
        prev_grapheme,
        next_grapheme,
        prev_glyph_width,
        snap_cell_width,
        font_descriptor,
        prev_auto_space,
        run_letter_spacing,
    } = context;

    // Width classification and font-slot classification are independent:
    // these observed full-width HAnsi glyphs keep their requested Latin face.
    // Check the character before parsing a descriptor on this hot path.
    let observed_unicode_width = if matches!(grapheme, "…" | "—")
        && !measured_width_is_oracle
        && paragraph_style.map_or_else(crate::line_metrics::word_layout_platform, |style| {
            style.word_platform
        }) == crate::line_metrics::WordLayoutPlatform::Mac
    {
        font_descriptor.and_then(|descriptor| {
            crate::line_metrics::word_observed_unicode_advance_px(descriptor, grapheme)
        })
    } else {
        None
    };
    let full_grid_cell =
        script == ScriptCategory::EastAsia || (grapheme == "…" && observed_unicode_width.is_some());
    let glyph_width = if measured_width_is_oracle {
        measured_glyph_width
    } else {
        adjusted_glyph_advance_width(
            script,
            requested_ascii_width
                .or(observed_unicode_width)
                .unwrap_or(measured_glyph_width),
            text_scale_x,
        )
    };
    let glyph_width = if !measured_width_is_oracle
        && script == ScriptCategory::EastAsia
        && paragraph_style.is_some_and(|style| style.east_asian_advance_scale > 1.0)
    {
        glyph_width
            * paragraph_style
                .map(|style| style.east_asian_advance_scale)
                .unwrap_or(1.0)
    } else {
        glyph_width
    };
    let punctuation_adjustment = if measured_width_is_oracle {
        0.0
    } else {
        paragraph_style
            .map(|style| punctuation_compression_adjustment(style, grapheme, script, glyph_width))
            .unwrap_or(0.0)
    };
    let top_line_adjustment = if measured_width_is_oracle {
        0.0
    } else {
        paragraph_style
            .map(|style| {
                top_line_punctuation_adjustment(style, grapheme, script, glyph_width, is_line_start)
            })
            .unwrap_or(0.0)
    };
    let adjusted_glyph_width =
        (glyph_width + punctuation_adjustment + top_line_adjustment).max(0.0);
    let grid_spacing = if measured_width_is_oracle {
        0.0
    } else {
        paragraph_style
            .filter(|style| !style.doc_grid_snap_to_chars)
            .and_then(|style| style.doc_grid_char_space_px)
            .unwrap_or(0.0)
            * if full_grid_cell { 1.0 } else { 0.5 }
    };
    // In Word for Mac's East Asian loose character grid, tracking is per
    // half-cell: a full-width character consumes two tracking intervals.
    // numbering-restart's 11pt glyph + 3pt grid - 2×1pt tracking has a 12pt
    // pitch. ASCII still consumes one interval; terminal glyphs already have
    // zero extra_spacing. Cell snapping and non-grid/Windows paths retain
    // their ordinary between-character tracking.
    let extra_spacing = if extra_spacing != 0.0
        && full_grid_cell
        && paragraph_style.is_some_and(|style| {
            style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
                && style.east_asian_rules.use_east_asian_layout
                && !style.doc_grid_snap_to_chars
                && style.doc_grid_char_space_px.is_some()
        }) {
        extra_spacing * 2.0
    } else {
        extra_spacing
    };
    let core_advance = (adjusted_glyph_width + extra_spacing + grid_spacing).max(0.0);
    let balance_extra = if measured_width_is_oracle {
        0.0
    } else {
        paragraph_style
            .map(|style| {
                balanced_single_byte_extra_width(
                    style,
                    grapheme,
                    script,
                    adjusted_glyph_width,
                    prev_grapheme,
                    next_grapheme,
                )
            })
            .unwrap_or(0.0)
            .max(0.0)
    };
    let (implicit_latin_gap, implicit_latin_trailing_gap) = if measured_width_is_oracle {
        (0.0, 0.0)
    } else {
        paragraph_style
            .map(|style| {
                implicit_latin_east_asian_gap_width(
                    style,
                    grapheme,
                    script,
                    adjusted_glyph_width,
                    prev_grapheme,
                    next_grapheme,
                    font_descriptor,
                )
            })
            .unwrap_or((0.0, 0.0))
    };
    // The gap only sits at a digit/East Asian boundary, so most graphemes
    // skip the call: this runs for every grapheme measured.
    let implicit_numeric_gap = if measured_width_is_oracle
        || !(is_ascii_digit_grapheme(grapheme)
            || prev_grapheme.is_some_and(is_ascii_digit_grapheme))
    {
        0.0
    } else {
        paragraph_style
            .map(|style| {
                implicit_numeric_east_asian_gap_width(
                    style,
                    ImplicitNumericGapContext {
                        grapheme,
                        script,
                        glyph_width: adjusted_glyph_width,
                        prev_prev_grapheme,
                        prev_grapheme,
                        prev_glyph_width,
                        font_descriptor,
                        prev_auto_space,
                    },
                )
            })
            .unwrap_or(0.0)
    };
    // On a Word for Mac character grid the auto spacing is a quarter of the
    // East Asian pitch rather than of the face's em: each boundary also takes
    // a quarter of what the grid and the tracking (twice per full cell) add to
    // it. indent-with-grid's 12pt text on 20pt cells is 5pt apart from its
    // digits; numbering-restart's 11pt + 3pt − 2 × 1pt pitch, 3pt.
    let grid_auto_space = if measured_width_is_oracle {
        0.0
    } else {
        paragraph_style
            .filter(|style| {
                style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
                    && !style.doc_grid_snap_to_chars
            })
            .and_then(|style| style.doc_grid_char_space_px)
            .filter(|space| *space > 0.0)
            .map_or(0.0, |space| (space + 2.0 * run_letter_spacing) * 0.25)
    };
    let widen = |gap: f64| {
        if gap > 0.0 {
            gap + grid_auto_space
        } else {
            gap
        }
    };
    let implicit_latin_gap = widen(implicit_latin_gap);
    let implicit_latin_trailing_gap = widen(implicit_latin_trailing_gap);
    let implicit_numeric_gap = widen(implicit_numeric_gap);
    let (snap_leading, snap_trailing) = snap_cell_width
        .map(|cell_width| {
            snap_to_chars_cell_span_adjustment(grapheme, script, core_advance, cell_width)
        })
        .unwrap_or((0.0, 0.0));
    let glyph_box_width =
        core_advance + snap_trailing + balance_extra + implicit_latin_trailing_gap;
    let total_advance = (implicit_latin_gap
        + implicit_numeric_gap
        + kerning_adjustment
        + snap_leading
        + glyph_box_width)
        .max(0.0);

    // Only a digit right before East Asian text needs its face's metrics, so
    // the descriptor is parsed at those boundaries alone.
    let trailing_auto_space = (paragraph_style
        .is_some_and(|style| style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac)
        && is_ascii_digit_grapheme(grapheme)
        && next_grapheme.is_some_and(is_auto_space_east_asian_text))
    .then(|| {
        font_descriptor.and_then(|descriptor| {
            crate::line_metrics::word_auto_space_metrics_with_balance(
                descriptor,
                paragraph_style.is_some_and(|style| {
                    style.east_asian_rules.balance_single_byte_double_byte_width
                }),
            )
        })
    })
    .flatten();

    GraphemeAdvance {
        glyph_width,
        adjusted_glyph_width,
        implicit_latin_gap,
        implicit_latin_trailing_gap,
        implicit_numeric_gap,
        kerning_adjustment,
        snap_leading,
        snap_trailing,
        balance_extra,
        core_advance,
        glyph_box_width,
        total_advance,
        trailing_auto_space,
    }
}

/// Returns the additional advance width used when
/// `balanceSingleByteDoubleByteWidth` is enabled.
///
/// This helper is shared by wrapping and rendering paths so both use the
/// same SBCS compensation profile.
pub fn balanced_single_byte_extra_width(
    paragraph_style: &ParagraphStyle,
    grapheme: &str,
    script: ScriptCategory,
    glyph_width: f64,
    prev_grapheme: Option<&str>,
    next_grapheme: Option<&str>,
) -> f64 {
    if !paragraph_style.enable_line_break_rules {
        return 0.0;
    }
    if !paragraph_style
        .east_asian_rules
        .balance_single_byte_double_byte_width
        || !crate::line_metrics::word_widens_balanced_single_byte_glyphs(
            paragraph_style.word_platform,
        )
    {
        return 0.0;
    }
    if script != ScriptCategory::Latin {
        return 0.0;
    }
    if !grapheme.is_ascii()
        || grapheme
            .bytes()
            .all(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return 0.0;
    }
    if grapheme.chars().all(|ch| ch.is_ascii_alphabetic()) {
        // In Word, `balanceSingleByteDoubleByteWidth` has a much smaller
        // impact on pure alphabetic spans than on digits/symbols.
        return 0.0;
    }
    if grapheme.len() == 1 && grapheme.as_bytes()[0].is_ascii_digit() {
        let prev_char = prev_grapheme.and_then(|value| value.chars().next());
        let next_char = next_grapheme.and_then(|value| value.chars().next());
        if prev_char.is_some_and(is_numeric_group_separator)
            || next_char.is_some_and(is_numeric_group_separator)
        {
            // Comma-separated numeric groups (e.g. 1,000,000) need less
            // widening than plain ASCII, but zero widening underestimates
            // width and shifts later wrap points.
            return glyph_width * BALANCED_SBCS_EXTRA_RATIO * 0.45;
        }
    }

    glyph_width * BALANCED_SBCS_EXTRA_RATIO
}

fn is_latin_letter_for_auto_spacing(grapheme: &str, script: ScriptCategory) -> bool {
    if script != ScriptCategory::Latin {
        return false;
    }
    grapheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphabetic() && !ch.is_ascii_digit())
}

/// Returns the implicit gap between Latin and East-Asian characters as
/// `(leading_gap, trailing_gap)`.
///
/// `leading_gap` is the space inserted *before* the current Latin grapheme
/// (when the previous grapheme is East-Asian).
/// `trailing_gap` is the space inserted *after* the current Latin grapheme
/// (when the next grapheme is East-Asian).
///
/// Previously, both sides were lumped into a single leading value, which
/// caused a visible space *between* consecutive Latin characters instead of
/// at the actual script boundary.
pub fn implicit_latin_east_asian_gap_width(
    paragraph_style: &ParagraphStyle,
    grapheme: &str,
    script: ScriptCategory,
    glyph_width: f64,
    prev_grapheme: Option<&str>,
    next_grapheme: Option<&str>,
    font_descriptor: Option<&str>,
) -> (f64, f64) {
    if !paragraph_style.enable_line_break_rules {
        return (0.0, 0.0);
    }
    // Missing autoSpaceDE is interpreted as enabled by Word.
    // Only explicit false disables this implicit boundary spacing.
    if paragraph_style.east_asian_rules.auto_space_de == Some(false) {
        return (0.0, 0.0);
    }
    if !is_latin_letter_for_auto_spacing(grapheme, script) {
        return (0.0, 0.0);
    }

    let prev_is_east_asia = prev_grapheme.is_some_and(is_auto_space_east_asian_text);
    let next_is_east_asia = next_grapheme.is_some_and(is_auto_space_east_asian_text);

    if !prev_is_east_asia && !next_is_east_asia {
        return (0.0, 0.0);
    }

    let face_metrics = (paragraph_style.word_platform
        == crate::line_metrics::WordLayoutPlatform::Mac)
        .then(|| {
            font_descriptor.and_then(|descriptor| {
                crate::line_metrics::word_auto_space_metrics_with_balance(
                    descriptor,
                    paragraph_style
                        .east_asian_rules
                        .balance_single_byte_double_byte_width,
                )
            })
        })
        .flatten();
    if let Some(metrics) = face_metrics {
        let leading = if prev_is_east_asia {
            crate::line_metrics::word_auto_space_gap_px(metrics.leading_gap_px())
        } else {
            0.0
        };
        let trailing = if next_is_east_asia {
            crate::line_metrics::word_auto_space_gap_px(
                metrics.trailing_gap_px(
                    paragraph_style
                        .east_asian_rules
                        .balance_single_byte_double_byte_width,
                ),
            )
        } else {
            0.0
        };
        return (leading, trailing);
    }

    let side_gap = (glyph_width * constants::IMPLICIT_LATIN_GAP_SIDE_RATIO)
        .max(constants::IMPLICIT_LATIN_GAP_SIDE_MIN_PX);

    let leading = if prev_is_east_asia {
        crate::line_metrics::word_auto_space_gap_px(side_gap)
    } else {
        0.0
    };
    let trailing = if next_is_east_asia {
        crate::line_metrics::word_auto_space_gap_px(
            side_gap * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE,
        )
    } else {
        0.0
    };
    (leading, trailing)
}

pub struct ImplicitNumericGapContext<'a> {
    pub grapheme: &'a str,
    pub script: ScriptCategory,
    pub glyph_width: f64,
    pub prev_prev_grapheme: Option<&'a str>,
    pub prev_grapheme: Option<&'a str>,
    pub prev_glyph_width: Option<f64>,
    /// The face `grapheme` is drawn with (see [`GraphemeAdvanceContext`]).
    pub font_descriptor: Option<&'a str>,
    /// Auto-spacing metrics of the digit before `grapheme`.
    pub prev_auto_space: Option<crate::line_metrics::WordAutoSpace>,
}

pub fn implicit_numeric_east_asian_gap_width(
    paragraph_style: &ParagraphStyle,
    context: ImplicitNumericGapContext<'_>,
) -> f64 {
    let ImplicitNumericGapContext {
        grapheme,
        script,
        glyph_width,
        prev_prev_grapheme: _,
        prev_grapheme,
        prev_glyph_width,
        font_descriptor,
        prev_auto_space,
    } = context;
    if !paragraph_style.enable_line_break_rules {
        return 0.0;
    }
    if paragraph_style
        .east_asian_rules
        .balance_single_byte_double_byte_width
        && paragraph_style.east_asian_rules.auto_space_dn != Some(true)
        && crate::line_metrics::word_balance_suppresses_numeric_auto_space(
            paragraph_style.word_platform,
        )
        || paragraph_style.east_asian_rules.auto_space_dn == Some(false)
    {
        return 0.0;
    }
    if !is_ascii_digit_grapheme(grapheme) {
        let prev_is_digit = prev_grapheme.is_some_and(is_ascii_digit_grapheme);
        let current_is_east_asia = script == ScriptCategory::EastAsia;
        if !(prev_is_digit && current_is_east_asia) {
            return 0.0;
        }
    }

    // Each boundary needs only one side classified, so classify lazily: this
    // runs for every grapheme of East Asian text.
    let current_is_digit = is_ascii_digit_grapheme(grapheme);
    let boundary_weight = if current_is_digit {
        if !prev_grapheme.is_some_and(is_auto_space_east_asian_text) {
            return 0.0;
        }
        1.0
    } else if prev_grapheme.is_some_and(is_ascii_digit_grapheme)
        && is_auto_space_east_asian_text(grapheme)
    {
        // The number-to-text side is narrower than the text-to-number one;
        // see `AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE`.
        // `prev_grapheme` is the final digit of the numeric token, so this is
        // an outer number/East-Asian boundary even when the digit follows a
        // decimal or grouping separator (`14.6パーセント`, `1,000円`). Word's
        // autoSpaceDN applies here. Separators *inside* the numeric token are
        // already excluded by the `current_is_digit` branch above because the
        // preceding grapheme is not East Asian.
        constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE
    } else {
        0.0
    };
    if boundary_weight <= f64::EPSILON {
        return 0.0;
    }

    // Faces with known metrics: 1/4 em before the digit, the digit face's
    // trailing gap after it (`line_metrics::WordAutoSpace`).
    let known_gap = if paragraph_style.word_platform != crate::line_metrics::WordLayoutPlatform::Mac
    {
        None
    } else if current_is_digit {
        font_descriptor
            .and_then(|descriptor| {
                crate::line_metrics::word_auto_space_metrics_with_balance(
                    descriptor,
                    paragraph_style
                        .east_asian_rules
                        .balance_single_byte_double_byte_width,
                )
            })
            .map(crate::line_metrics::WordAutoSpace::leading_gap_px)
    } else {
        let balance = paragraph_style
            .east_asian_rules
            .balance_single_byte_double_byte_width;
        prev_auto_space.map(|metrics| metrics.trailing_gap_px(balance))
    };
    if let Some(gap) = known_gap {
        return crate::line_metrics::word_auto_space_gap_px(gap);
    }

    let (side_ratio, side_min_px) =
        numeric_auto_spacing_params(paragraph_style.east_asian_rules.auto_space_dn);
    let basis = if current_is_digit {
        glyph_width
    } else {
        let Some(previous_width) = prev_glyph_width else {
            return 0.0;
        };
        previous_width
    };
    crate::line_metrics::word_auto_space_gap_px(
        (basis * side_ratio).max(side_min_px) * boundary_weight,
    )
}

fn is_ascii_digit_grapheme(grapheme: &str) -> bool {
    grapheme.len() == 1
        && grapheme
            .as_bytes()
            .first()
            .is_some_and(|value| value.is_ascii_digit())
}

fn is_compressible_east_asian_punctuation(grapheme: &str) -> bool {
    matches!(
        grapheme,
        "、" | "。" | "，" | "．" | "・" | "：" | "；" | "！" | "？"
    )
}

pub fn punctuation_compression_adjustment(
    paragraph_style: &ParagraphStyle,
    grapheme: &str,
    script: ScriptCategory,
    glyph_width: f64,
) -> f64 {
    if !paragraph_style.enable_line_break_rules {
        return 0.0;
    }
    if paragraph_style
        .east_asian_rules
        .character_spacing_control
        .as_deref()
        != Some("compressPunctuation")
        || !crate::line_metrics::word_compresses_mid_line_punctuation(paragraph_style.word_platform)
    {
        return 0.0;
    }
    if script != ScriptCategory::EastAsia || !is_compressible_east_asian_punctuation(grapheme) {
        return 0.0;
    }

    // Keep compression conservative so punctuation rhythm remains stable.
    glyph_width * -0.18
}

fn is_top_line_half_width_target(grapheme: &str) -> bool {
    matches!(
        grapheme,
        "、" | "。"
            | "，"
            | "．"
            | "・"
            | "："
            | "；"
            | "！"
            | "？"
            | ")"
            | "]"
            | "}"
            | "）"
            | "］"
            | "｝"
            | "」"
            | "』"
            | "】"
    )
}

pub fn top_line_punctuation_adjustment(
    paragraph_style: &ParagraphStyle,
    grapheme: &str,
    script: ScriptCategory,
    glyph_width: f64,
    is_line_start: bool,
) -> f64 {
    if !paragraph_style.applies_top_line_punctuation_half_width() || !is_line_start {
        return 0.0;
    }
    if script != ScriptCategory::EastAsia || !is_top_line_half_width_target(grapheme) {
        return 0.0;
    }
    glyph_width * -0.5
}

fn is_latin_word_grapheme(grapheme: &str) -> bool {
    classify_script(grapheme) == ScriptCategory::Latin
        && grapheme.chars().all(|ch| ch.is_ascii_alphabetic())
}

/// How Word stretches the gap between two adjacent graphemes when it
/// justifies an East Asian line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EastAsianJustificationGap {
    /// An auto-spacing boundary between East Asian text and a Latin letter or
    /// digit. Word first widens it by up to its own auto-space width, given
    /// here as a share of the East Asian em.
    AutoSpace { em_share: f64 },
    /// Two East Asian graphemes: Word spreads what is left evenly over these.
    InterCharacter,
}

/// Classifies the boundary between `prev` and `next` for East Asian line
/// justification; `None` where Word adds nothing (inside a Latin word or a
/// number, around spaces).
///
/// Word for Mac justifies the hanging line `…直前12か月間に…上限とする。` of
/// `system-dev-contract.docx` by 7.6pt: `前|1` and `2|か` grow by their own
/// 2.625pt/2.1pt auto-space, and each of the 40 kana/kanji gaps by 0.073pt,
/// while `1|2` keeps its natural advance.
pub fn east_asian_justification_gap(prev: &str, next: &str) -> Option<EastAsianJustificationGap> {
    let is_latin_or_digit = |grapheme: &str| {
        grapheme
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
    };
    if is_auto_space_east_asian_text(prev) && is_latin_or_digit(next) {
        return Some(EastAsianJustificationGap::AutoSpace { em_share: 0.25 });
    }
    if is_latin_or_digit(prev) && is_auto_space_east_asian_text(next) {
        return Some(EastAsianJustificationGap::AutoSpace {
            em_share: 0.25 * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE,
        });
    }
    (is_non_whitespace_east_asian_grapheme(prev) && is_non_whitespace_east_asian_grapheme(next))
        .then_some(EastAsianJustificationGap::InterCharacter)
}

/// Whether justification widens `grapheme` as a word space. Word for Mac
/// sets the ideographic space as a full-width character: scroll-offset-repro's
/// justified `　　勤続6ヵ月…` keeps its two leading U+3000 at 10.5pt and puts
/// the slack into the auto-space gaps around `6`.
pub fn is_justification_space(
    grapheme: &str,
    platform: crate::line_metrics::WordLayoutPlatform,
) -> bool {
    !grapheme.is_empty()
        && grapheme.chars().all(char::is_whitespace)
        && !(platform == crate::line_metrics::WordLayoutPlatform::Mac && grapheme == "\u{3000}")
}

/// Splits `extra` over the gaps of an East Asian line the way Word does:
/// auto-space boundaries first, each up to its auto-space width
/// (`em_share × em`), then the remainder evenly over the inter-character gaps.
/// Returns the extra per gap (same order as `gaps`); `None` entries get 0.
/// Whatever cannot be placed (no inter-character gap) is left undistributed.
pub fn distribute_east_asian_justification(
    gaps: &[(Option<EastAsianJustificationGap>, f64)],
    extra: f64,
) -> Vec<f64> {
    let mut shares = vec![0.0; gaps.len()];
    if extra <= 0.0 {
        return shares;
    }
    let capacity: f64 = gaps
        .iter()
        .map(|(gap, em)| match gap {
            Some(EastAsianJustificationGap::AutoSpace { em_share }) => em_share * em,
            _ => 0.0,
        })
        .sum();
    let auto_space_scale = if capacity > 0.0 {
        (extra / capacity).min(1.0)
    } else {
        0.0
    };
    let inter_character_count = gaps
        .iter()
        .filter(|(gap, _)| matches!(gap, Some(EastAsianJustificationGap::InterCharacter)))
        .count();
    let remainder = (extra - capacity).max(0.0);
    let per_inter_character = if inter_character_count > 0 {
        remainder / inter_character_count as f64
    } else {
        0.0
    };
    for (share, (gap, em)) in shares.iter_mut().zip(gaps) {
        *share = match gap {
            Some(EastAsianJustificationGap::AutoSpace { em_share }) => {
                em_share * em * auto_space_scale
            }
            Some(EastAsianJustificationGap::InterCharacter) => per_inter_character,
            None => 0.0,
        };
    }
    shares
}

/// East Asian text that takes an auto-spacing boundary next to Latin letters
/// or digits: ideographs and kana, but not East Asian punctuation, the
/// katakana middle dot `・` included. Word sets `、2026` and `「M` with no gap
/// after the punctuation and `70・一部` with none before it, while `は1`, `第2`
/// and `とA` get one (`system-dev-contract.docx` and `yokohama_page6` Word for
/// Mac prints; `math` and `yokohama_page1` Word COM traces).
fn is_auto_space_east_asian_text(grapheme: &str) -> bool {
    is_non_whitespace_east_asian_grapheme(grapheme)
        && !grapheme.chars().next().is_some_and(|ch| {
            (is_east_asian_punctuation(ch) || ch == '\u{30FB}')
                && !matches!(ch, '\u{3005}'..='\u{3007}')
        })
}

fn is_non_whitespace_east_asian_grapheme(grapheme: &str) -> bool {
    classify_script(grapheme) == ScriptCategory::EastAsia
        && !grapheme
            .chars()
            .all(|ch| ch.is_whitespace() || ch.is_control())
}

fn is_numeric_group_separator(ch: char) -> bool {
    matches!(ch, ',' | '.' | '，' | '．')
}

#[cfg(test)]
fn previous_grapheme_at_absolute_offset(
    text: &str,
    base_offset: usize,
    grapheme_start_abs: usize,
) -> Option<&str> {
    let rel = grapheme_start_abs.saturating_sub(base_offset);
    text.get(..rel)?.graphemes(true).next_back()
}

fn next_grapheme_at_absolute_offset(
    text: &str,
    base_offset: usize,
    grapheme_end_abs: usize,
) -> Option<&str> {
    let rel = grapheme_end_abs.saturating_sub(base_offset);
    text.get(rel..)?.graphemes(true).next()
}

#[cfg(test)]
fn previous_previous_grapheme_at_absolute_offset(
    text: &str,
    base_offset: usize,
    grapheme_start_abs: usize,
) -> Option<&str> {
    let rel = grapheme_start_abs.saturating_sub(base_offset);
    let mut graphemes = text.get(..rel)?.graphemes(true).rev();
    graphemes.next()?;
    graphemes.next()
}

fn marker_spacing_gap_ratio(
    paragraph_style: &ParagraphStyle,
    clusters: &[InlineCluster],
    grapheme: &str,
    script: ScriptCategory,
) -> f64 {
    // Word for Mac adds nothing after a literal `(8)` or `(8) ` before East
    // Asian text (trust-beneficiary-sale-keep-next page 5).
    if !paragraph_style.east_asian_rules.use_east_asian_layout
        || paragraph_style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
    {
        return 0.0;
    }
    if script != ScriptCategory::EastAsia {
        return 0.0;
    }
    if clusters.is_empty() || clusters.len() > 4 {
        return 0.0;
    }
    if grapheme.trim().is_empty() {
        return 0.0;
    }

    let mut prefix = String::new();
    let mut seen_whitespace = false;
    for cluster in clusters {
        if cluster.is_whitespace {
            seen_whitespace = true;
            continue;
        }
        if seen_whitespace {
            return 0.0;
        }
        prefix.push_str(&cluster.text);
    }
    if prefix.is_empty() {
        return 0.0;
    }

    let marker_like = if prefix.starts_with('(') {
        prefix.ends_with(')')
            && prefix
                .chars()
                .skip(1)
                .take(prefix.chars().count().saturating_sub(2))
                .all(|ch| ch.is_ascii_digit())
    } else {
        prefix.chars().all(|ch| ch.is_ascii_digit())
    };
    if !marker_like {
        return 0.0;
    }

    let Some(prev) = clusters.last() else {
        return 0.0;
    };
    if prev.is_whitespace {
        // Full-width marker spaces are already visually wide enough.
        // Avoid adding an extra synthetic gap here.
        if prev.text.contains('\u{3000}') {
            return 0.0;
        }
        // For plain ASCII whitespace, keep a small compensation.
        return 0.5;
    }
    let Some(prev_char) = prev.text.chars().next() else {
        return 0.0;
    };
    // A bare number running straight into East Asian text is not a marker:
    // Word sets the closing line `2026年4月1日` of `system-dev-contract.docx`
    // with only the autoSpaceDN gap after `2026`.
    if prev_char == ')' {
        1.0
    } else {
        0.0
    }
}

/// Run metadata covering contiguous text ranges and their formatting overrides.
#[derive(Clone, Debug)]
pub struct RunStyle {
    pub start: usize,
    pub end: usize,
    pub properties: Option<Rc<RunProperties>>,
    pub revision: Option<RevisionMark>,
    pub hyperlink: Option<RunHyperlink>,
    pub inline_object: Option<InlineObject>,
}

/// Non-text inline content that participates in Word line layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineObject {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextAdvanceState {
    pub prev_prev_grapheme: Option<CompactString>,
    pub prev_grapheme: Option<CompactString>,
    pub prev_adjusted_glyph_width: Option<f64>,
    pub prev_raw_glyph_width: Option<f64>,
    pub prev_font_descriptor_hash: Option<u64>,
    /// See [`GraphemeAdvanceContext::prev_auto_space`].
    pub prev_auto_space: Option<crate::line_metrics::WordAutoSpace>,
}

pub struct TextAdvanceStep<'a> {
    pub rel_offset: usize,
    pub grapheme: &'a str,
    pub script: ScriptCategory,
    pub font_descriptor: &'a str,
    pub font_descriptor_hash: u64,
    pub measured_glyph_width: f64,
    pub advance: GraphemeAdvance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedTextAdvanceStep {
    pub rel_offset: usize,
    pub grapheme: Rc<str>,
    pub script: ScriptCategory,
    pub font_descriptor: Rc<str>,
    pub font_descriptor_hash: u64,
    pub measured_glyph_width: f64,
    pub advance: GraphemeAdvance,
}

struct ComputedTextAdvanceStep<'a> {
    script: ScriptCategory,
    font_descriptor: &'a str,
    font_descriptor_hash: u64,
    measured_glyph_width: f64,
    measured_width_is_oracle: bool,
    /// The advance came from a fixed-pitch face's cell, not the host.
    monospace_cell: bool,
    advance: GraphemeAdvance,
}

#[derive(Clone, Debug, Default)]
struct PairMeasureCache {
    entries: [Option<PairMeasureCacheEntry>; 2],
}

#[derive(Clone, Debug)]
struct PairMeasureCacheEntry {
    // `CompactString` keeps typical 1–4 byte graphemes inline, so a cache miss
    // (common for running text where adjacent pairs are unique) no longer costs
    // two heap allocations per grapheme on the kerning path.
    left: CompactString,
    right: CompactString,
    font_descriptor_hash: u64,
    width: f64,
}

impl PairMeasureCache {
    #[inline]
    fn get(&self, left: &str, right: &str, font_descriptor_hash: u64) -> Option<f64> {
        self.entries.iter().flatten().find_map(|entry| {
            (entry.font_descriptor_hash == font_descriptor_hash
                && entry.left == left
                && entry.right == right)
                .then_some(entry.width)
        })
    }

    #[inline]
    fn insert(&mut self, left: &str, right: &str, font_descriptor_hash: u64, width: f64) {
        let entry = PairMeasureCacheEntry {
            left: CompactString::new(left),
            right: CompactString::new(right),
            font_descriptor_hash,
            width,
        };
        self.entries[1] = self.entries[0].take();
        self.entries[0] = Some(entry);
    }
}

#[inline]
fn update_text_advance_state(
    state: &mut TextAdvanceState,
    grapheme: &str,
    measured_glyph_width: f64,
    adjusted_glyph_width: f64,
    font_descriptor_hash: u64,
    trailing_auto_space: Option<crate::line_metrics::WordAutoSpace>,
) {
    state.prev_auto_space = trailing_auto_space;
    state.prev_prev_grapheme = state.prev_grapheme.take();
    state.prev_grapheme = Some(CompactString::new(grapheme));
    state.prev_adjusted_glyph_width = Some(adjusted_glyph_width);
    state.prev_raw_glyph_width = Some(measured_glyph_width);
    state.prev_font_descriptor_hash = Some(font_descriptor_hash);
}

/// The auto-spacing metrics a digit leaves for the East Asian text after it;
/// `None` for anything else, which needs none.
pub fn digit_auto_space(
    grapheme: &str,
    font_descriptor: &str,
) -> Option<crate::line_metrics::WordAutoSpace> {
    digit_auto_space_with_balance(grapheme, font_descriptor, false)
}

/// Same digit state for a run whose paragraph requests balanced SBCS/DBCS
/// spacing. This carries its quarter em across run boundaries even when the
/// requested Latin face has no known average-width metric.
pub fn digit_auto_space_with_balance(
    grapheme: &str,
    font_descriptor: &str,
    balance_single_byte_double_byte_width: bool,
) -> Option<crate::line_metrics::WordAutoSpace> {
    is_ascii_digit_grapheme(grapheme)
        .then(|| {
            crate::line_metrics::word_auto_space_metrics_with_balance(
                font_descriptor,
                balance_single_byte_double_byte_width,
            )
        })
        .flatten()
}

#[inline]
fn measure_text_pair<C: LineWrapContext>(
    ctx: &C,
    left: &str,
    right: &str,
    font_descriptor: &str,
    font_descriptor_hash: u64,
    font_descriptor_id: Option<u32>,
    pair_cache: &mut PairMeasureCache,
) -> Result<f64, C::Error> {
    if let Some(width) = pair_cache.get(left, right, font_descriptor_hash) {
        return Ok(width);
    }

    const STACK_BUF_CAPACITY: usize = 32;
    let total_len = left.len() + right.len();
    if total_len <= STACK_BUF_CAPACITY {
        let mut buf = [0_u8; STACK_BUF_CAPACITY];
        let left_len = left.len();
        buf[..left_len].copy_from_slice(left.as_bytes());
        buf[left_len..total_len].copy_from_slice(right.as_bytes());
        let pair = std::str::from_utf8(&buf[..total_len]).expect("valid concatenated utf-8");
        let width = ctx.measure_text_with_font_key(
            pair,
            font_descriptor,
            font_descriptor_hash,
            font_descriptor_id,
        )?;
        pair_cache.insert(left, right, font_descriptor_hash, width);
        return Ok(width);
    }

    let mut pair = String::with_capacity(total_len);
    pair.push_str(left);
    pair.push_str(right);
    let width = ctx.measure_text_with_font_key(
        &pair,
        font_descriptor,
        font_descriptor_hash,
        font_descriptor_id,
    )?;
    pair_cache.insert(left, right, font_descriptor_hash, width);
    Ok(width)
}

#[allow(clippy::too_many_arguments)]
fn compute_text_advance_step<'a, C: LineWrapContext>(
    ctx: &C,
    grapheme: &'a str,
    style: &'a ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    state: &TextAdvanceState,
    next_grapheme: Option<&'a str>,
    is_line_start: bool,
    snap_cell_width: Option<f64>,
    pair_cache: &mut PairMeasureCache,
) -> Result<ComputedTextAdvanceStep<'a>, C::Error> {
    let script = classify_script(grapheme);
    let (font_descriptor, font_descriptor_hash, font_descriptor_id) =
        style.descriptor_key_for_grapheme(grapheme, script);
    compute_text_advance_step_with_descriptor(
        ctx,
        grapheme,
        style,
        paragraph_style,
        state,
        next_grapheme,
        is_line_start,
        snap_cell_width,
        pair_cache,
        script,
        font_descriptor,
        font_descriptor_hash,
        font_descriptor_id,
        None,
    )
}

// Wide/fullwidth scalar ranges with fixed defaults in the known CJK faces.
// Halfwidth, combining sequences and unclassified ranges retain host shaping.
#[inline]
fn is_fixed_fullwidth_cjk_scalar(grapheme: &str) -> bool {
    let Some(ch) = grapheme.chars().next() else {
        return false;
    };
    grapheme.len() == ch.len_utf8()
        && matches!(ch as u32,
        0x3000..=0x303E | 0x3041..=0x3096 | 0x30A1..=0x30FA
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xFF01..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x2FFFD)
}

/// Host font stand-ins can report a nominal full cell a subpixel away from
/// its requested em (Linux CI: 13.999938964px versus 14px). This tolerance
/// only classifies an already-known fixed face; it never changes an advance.
#[inline]
fn fixed_cjk_metric_tolerance_px(em: f64) -> f64 {
    (em * 1e-6).max(1.0 / 1024.0)
}

#[inline]
fn pair_has_fixed_cjk_advances(
    grapheme: &str,
    script: ScriptCategory,
    style: &ResolvedRunStyle,
    state: &TextAdvanceState,
    measured_width: f64,
    paragraph_style: Option<&ParagraphStyle>,
) -> bool {
    if script != ScriptCategory::EastAsia
        || paragraph_style.map_or_else(crate::line_metrics::word_layout_platform, |p| {
            p.word_platform
        }) != crate::line_metrics::WordLayoutPlatform::Mac
        || !is_fixed_fullwidth_cjk_scalar(grapheme)
        || !state
            .prev_grapheme
            .as_deref()
            .is_some_and(is_fixed_fullwidth_cjk_scalar)
    {
        return false;
    }
    // Adjacent CJK punctuation has required blank allocation (for example
    // comma followed by an opening quote). Preserve that existing context
    // path; fixed letter pitch must not suppress punctuation-pair compression.
    let previous = state.prev_grapheme.as_deref().unwrap_or_default();
    let punctuation = |text: &str| {
        text.chars().next().is_some_and(|ch| {
            is_east_asian_punctuation(ch) && ch.script() == UnicodeScriptKind::Common
        })
    };
    if punctuation(grapheme) && punctuation(previous) {
        return false;
    }
    let Some(em) = style.fixed_east_asia_em_px() else {
        return false;
    };
    let tolerance = fixed_cjk_metric_tolerance_px(em);
    (measured_width - em).abs() <= tolerance
        && state
            .prev_raw_glyph_width
            .is_some_and(|width| (width - em).abs() <= tolerance)
}

fn mac_fixed_punctuation_pair_adjustment(
    grapheme: &str,
    script: ScriptCategory,
    font_descriptor: &str,
    font_descriptor_hash: u64,
    state: &TextAdvanceState,
    measured_width: f64,
    paragraph_style: Option<&ParagraphStyle>,
) -> Option<f64> {
    let paragraph_style = paragraph_style?;
    // Word for Mac 16.114 shares the blank of a kerned pair whatever
    // `characterSpacingControl` says (probe pairs-kerned and its
    // doNotCompress twin are identical).
    if script != ScriptCategory::EastAsia
        || paragraph_style.word_platform != crate::line_metrics::WordLayoutPlatform::Mac
        || state.prev_font_descriptor_hash != Some(font_descriptor_hash)
    {
        return None;
    }
    let previous = state.prev_grapheme.as_deref()?;
    // The pair shares the blank on its touching sides: a closing mark's
    // trailing half and an opening mark's leading half, so 。）, ）、, 、「 and （「
    // each lose half an em. An opening mark followed by a closing one has no
    // blank between them.
    let punctuation = |text: &str| punctuation_blank_after(text) || punctuation_blank_before(text);
    let touching_blank = punctuation(previous)
        && punctuation(grapheme)
        && (punctuation_blank_after(previous) || punctuation_blank_before(grapheme));
    if !touching_blank {
        return None;
    }
    let em = crate::line_metrics::word_fixed_east_asian_em_px(font_descriptor)?;
    let previous_width = state.prev_raw_glyph_width?;
    let tolerance = fixed_cjk_metric_tolerance_px(em);
    if (measured_width - em).abs() > tolerance || (previous_width - em).abs() > tolerance {
        return None;
    }
    // Only kerned runs reach here (the caller checks `w:kern`). Word for
    // Mac's kerned MS Mincho rows in `math` and `scroll-offset-repro` set
    // `）」` `、「` `）（` `。）` `）。` `）、` this way; the unkerned `nda` and
    // `contract` keep full cells.
    Some(-0.5 * em)
}

/// Like [`compute_text_advance_step`], but the caller supplies the already
/// classified script and resolved font descriptor/hash. The hot wrap loop
/// computes these once per grapheme, so this avoids re-classifying the script
/// and re-resolving the descriptor a second time inside the advance step.
#[allow(clippy::too_many_arguments)]
fn compute_text_advance_step_with_descriptor<'a, C: LineWrapContext>(
    ctx: &C,
    grapheme: &'a str,
    style: &'a ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    state: &TextAdvanceState,
    next_grapheme: Option<&'a str>,
    is_line_start: bool,
    snap_cell_width: Option<f64>,
    pair_cache: &mut PairMeasureCache,
    script: ScriptCategory,
    font_descriptor: &'a str,
    font_descriptor_hash: u64,
    font_descriptor_id: Option<u32>,
    text_offset: Option<usize>,
) -> Result<ComputedTextAdvanceStep<'a>, C::Error> {
    let oracle_measured_glyph_width = text_offset.and_then(|offset| {
        paragraph_style
            .and_then(|style| style.oracle_advance_px_by_text_offset.as_ref())
            .and_then(|advances| advances.get(&offset).copied())
    });
    let measured_width_is_oracle = oracle_measured_glyph_width.is_some();
    // A space has no ink to embolden: Word for Mac sets scroll-offset-repro's
    // bold `第６章　賃金` ideographic space at its plain width. (A bold ASCII
    // space takes its advance from `word_ascii_metrics` before this.)
    let synthetic_bold_extra = if measured_width_is_oracle || grapheme.trim().is_empty() {
        0.0
    } else {
        crate::line_metrics::word_synthetic_bold_extra_advance_px(font_descriptor)
    };
    // ＭＳ 明朝 / ＭＳ ゴシック set every glyph on a full/half-em grid and have
    // no kerning, so neither the advance nor pair kerning comes from the
    // host, whose face may be a substitute (jlsi/jade#469).
    let monospace_width = if measured_width_is_oracle {
        None
    } else {
        crate::line_metrics::word_monospace_advance_px(font_descriptor, grapheme)
    };
    let measured_glyph_width = match (oracle_measured_glyph_width, monospace_width) {
        (Some(width), _) => width,
        (None, Some(width)) => width + synthetic_bold_extra,
        (None, None) => {
            ctx.measure_text_with_font_key(
                grapheme,
                font_descriptor,
                font_descriptor_hash,
                font_descriptor_id,
            )? + synthetic_bold_extra
        }
    };
    let extra_spacing = if !measured_width_is_oracle
        && style.letter_spacing_px.abs() > f64::EPSILON
        && next_grapheme.is_some()
    {
        style.letter_spacing_px
    } else {
        0.0
    };
    let punctuation_adjustment = if measured_width_is_oracle || !style.enable_pair_kerning {
        None
    } else {
        mac_fixed_punctuation_pair_adjustment(
            grapheme,
            script,
            font_descriptor,
            font_descriptor_hash,
            state,
            measured_glyph_width,
            paragraph_style,
        )
    };
    let kerning_adjustment = if let Some(adjustment) = punctuation_adjustment {
        adjustment * style.text_scale_x.max(0.01)
    } else if !measured_width_is_oracle
        && style.enable_pair_kerning
        && state.prev_grapheme.is_some()
        && state.prev_font_descriptor_hash == Some(font_descriptor_hash)
        && state.prev_raw_glyph_width.is_some()
        && !pair_has_fixed_cjk_advances(
            grapheme,
            script,
            style,
            state,
            measured_glyph_width,
            paragraph_style,
        )
    {
        let prev_grapheme = state.prev_grapheme.as_deref().unwrap_or_default();
        let word_pair_adjustment = if prev_grapheme.len() == 1
            && grapheme.len() == 1
            && prev_grapheme.is_ascii()
            && grapheme.is_ascii()
            && script == ScriptCategory::Latin
            && paragraph_style.map_or_else(crate::line_metrics::word_layout_platform, |style| {
                style.word_platform
            }) == crate::line_metrics::WordLayoutPlatform::Mac
        {
            style.ascii_pair_kerning_px(prev_grapheme.as_bytes()[0], grapheme.as_bytes()[0])
        } else {
            None
        };
        let raw_adjustment = if let Some(adjustment) = word_pair_adjustment {
            adjustment
        } else if monospace_width.is_some() {
            // Word's own punctuation rules above still apply; the host's pair
            // kerning belongs to whatever face stands in for the fixed one.
            0.0
        } else {
            let prev_width = state.prev_raw_glyph_width.unwrap_or_default();
            let pair_width = measure_text_pair(
                ctx,
                prev_grapheme,
                grapheme,
                font_descriptor,
                font_descriptor_hash,
                font_descriptor_id,
                pair_cache,
            )?;
            // Pair measurement contains host advances; synthetic emboldening is
            // added separately to each single-glyph advance. The descriptors match,
            // so remove that addition from both widths before finding host kerning.
            pair_width - prev_width - measured_glyph_width + 2.0 * synthetic_bold_extra
        };
        if raw_adjustment.is_finite() {
            raw_adjustment * style.text_scale_x.max(0.01)
        } else {
            0.0
        }
    } else {
        0.0
    };
    let mut advance = compute_resolved_grapheme_advance(
        GraphemeAdvanceContext {
            paragraph_style,
            grapheme,
            script,
            measured_glyph_width,
            measured_width_is_oracle,
            text_scale_x: style.text_scale_x.max(0.01),
            extra_spacing,
            kerning_adjustment,
            is_line_start,
            prev_prev_grapheme: state.prev_prev_grapheme.as_deref(),
            prev_grapheme: state.prev_grapheme.as_deref(),
            next_grapheme,
            prev_glyph_width: state.prev_adjusted_glyph_width,
            snap_cell_width,
            font_descriptor: Some(font_descriptor),
            prev_auto_space: state.prev_auto_space,
            run_letter_spacing: style.letter_spacing_px,
        },
        style,
        automatic_note_separator_width_px(grapheme, paragraph_style, text_offset, font_descriptor),
    );
    // When the resolved font slot is genuinely Default, Word for Windows keeps
    // numeric text in the ANSI classification and does not add the East-Asian
    // autoSpaceDN boundary gap. Renderer-side OOXML resolution only produces
    // this effective hint for a direct rFonts override that neither supplies
    // w:hint="eastAsia" nor names an East-Asian font. A run that explicitly
    // supplies its East-Asian face therefore retains autoSpaceDN, and Word for
    // Mac keeps it either way.
    if style.font_hint == Some(FontHint::Default)
        && advance.implicit_numeric_gap > 0.0
        && paragraph_style.is_none_or(|style| {
            crate::line_metrics::word_default_hint_suppresses_numeric_auto_space(
                style.word_platform,
            )
        })
    {
        advance.total_advance = (advance.total_advance - advance.implicit_numeric_gap).max(0.0);
        advance.implicit_numeric_gap = 0.0;
    }
    Ok(ComputedTextAdvanceStep {
        script,
        font_descriptor,
        font_descriptor_hash,
        measured_glyph_width,
        measured_width_is_oracle,
        monospace_cell: monospace_width.is_some(),
        advance,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn walk_text_advance_steps<C, F>(
    ctx: &C,
    text: &str,
    style: &ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    initial_state: &TextAdvanceState,
    next_grapheme_after_text: Option<&str>,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
    visitor: F,
) -> Result<TextAdvanceState, C::Error>
where
    C: LineWrapContext,
    F: FnMut(TextAdvanceStep<'_>) -> Result<(), C::Error>,
{
    walk_text_advance_steps_at_offset(
        ctx,
        text,
        style,
        paragraph_style,
        initial_state,
        next_grapheme_after_text,
        None,
        snap_to_chars_cell_width_cache,
        visitor,
    )
}

/// Walks text advances while preserving the absolute document offset used by
/// Word-oracle width overrides.
///
/// Normal renderer measurement passes `None` through [`walk_text_advance_steps`].
/// Completed-line remeasurement passes the segment's absolute start so it
/// consumes the same per-character Word advances as the initial wrap. Without
/// this offset, the repair pass silently falls back to a local font and can
/// move already-correct text to a duplicate continuation line, as happened in
/// the `math.docx` oracle regression.
#[allow(clippy::too_many_arguments)]
fn walk_text_advance_steps_at_offset<C, F>(
    ctx: &C,
    text: &str,
    style: &ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    initial_state: &TextAdvanceState,
    next_grapheme_after_text: Option<&str>,
    text_offset_base: Option<usize>,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
    mut visitor: F,
) -> Result<TextAdvanceState, C::Error>
where
    C: LineWrapContext,
    F: FnMut(TextAdvanceStep<'_>) -> Result<(), C::Error>,
{
    let snap_cell_width = if let Some(paragraph_style) = paragraph_style {
        resolve_snap_to_chars_cell_width(
            ctx,
            paragraph_style,
            &style.font_descriptor_east_asia,
            style.font_descriptor_east_asia_hash,
            style.text_scale_x.max(0.01),
            snap_to_chars_cell_width_cache,
        )?
    } else {
        None
    };
    let mut state = initial_state.clone();
    let mut graphemes = crate::graphemes::grapheme_indices(text).peekable();
    let mut is_line_start = initial_state.prev_grapheme.is_none();
    let mut pair_cache = PairMeasureCache::default();

    while let Some((rel_offset, grapheme)) = graphemes.next() {
        let next_grapheme = graphemes
            .peek()
            .map(|(_, value)| *value)
            .or(next_grapheme_after_text);
        let computed = if let Some(text_offset_base) = text_offset_base {
            let script = classify_script(grapheme);
            let (font_descriptor, font_descriptor_hash, font_descriptor_id) =
                style.descriptor_key_for_grapheme(grapheme, script);
            compute_text_advance_step_with_descriptor(
                ctx,
                grapheme,
                style,
                paragraph_style,
                &state,
                next_grapheme,
                is_line_start,
                snap_cell_width,
                &mut pair_cache,
                script,
                font_descriptor,
                font_descriptor_hash,
                font_descriptor_id,
                Some(text_offset_base + rel_offset),
            )?
        } else {
            compute_text_advance_step(
                ctx,
                grapheme,
                style,
                paragraph_style,
                &state,
                next_grapheme,
                is_line_start,
                snap_cell_width,
                &mut pair_cache,
            )?
        };
        visitor(TextAdvanceStep {
            rel_offset,
            grapheme,
            script: computed.script,
            font_descriptor: computed.font_descriptor,
            font_descriptor_hash: computed.font_descriptor_hash,
            measured_glyph_width: computed.measured_glyph_width,
            advance: computed.advance,
        })?;
        update_text_advance_state(
            &mut state,
            grapheme,
            computed.measured_glyph_width,
            computed.advance.adjusted_glyph_width,
            computed.font_descriptor_hash,
            computed.advance.trailing_auto_space,
        );
        is_line_start = false;
    }
    Ok(state)
}

#[allow(clippy::too_many_arguments)]
pub fn visit_owned_text_advance_steps<C, F>(
    ctx: &C,
    text: &str,
    style: &ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    initial_state: &TextAdvanceState,
    next_grapheme_after_text: Option<&str>,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
    mut visitor: F,
) -> Result<TextAdvanceState, C::Error>
where
    C: LineWrapContext,
    F: FnMut(OwnedTextAdvanceStep) -> Result<(), C::Error>,
{
    let latin_ascii_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_latin_ascii);
    let latin_hansi_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_latin_hansi);
    let east_asia_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_east_asia);
    let complex_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_complex);
    walk_text_advance_steps(
        ctx,
        text,
        style,
        paragraph_style,
        initial_state,
        next_grapheme_after_text,
        snap_to_chars_cell_width_cache,
        |step| {
            let font_descriptor =
                if step.font_descriptor_hash == style.font_descriptor_latin_ascii_hash {
                    latin_ascii_descriptor.clone()
                } else if step.font_descriptor_hash == style.font_descriptor_latin_hansi_hash {
                    latin_hansi_descriptor.clone()
                } else if step.font_descriptor_hash == style.font_descriptor_east_asia_hash {
                    east_asia_descriptor.clone()
                } else if step.font_descriptor_hash == style.font_descriptor_complex_hash {
                    complex_descriptor.clone()
                } else {
                    Rc::from(step.font_descriptor)
                };
            visitor(OwnedTextAdvanceStep {
                rel_offset: step.rel_offset,
                grapheme: Rc::from(step.grapheme),
                script: step.script,
                font_descriptor,
                font_descriptor_hash: step.font_descriptor_hash,
                measured_glyph_width: step.measured_glyph_width,
                advance: step.advance,
            })
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn collect_text_advance_steps<C>(
    ctx: &C,
    text: &str,
    style: &ResolvedRunStyle,
    paragraph_style: Option<&ParagraphStyle>,
    initial_state: &TextAdvanceState,
    next_grapheme_after_text: Option<&str>,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<(Vec<OwnedTextAdvanceStep>, TextAdvanceState), C::Error>
where
    C: LineWrapContext,
{
    let mut steps = Vec::with_capacity(crate::graphemes::grapheme_indices(text).count());
    let end_state = visit_owned_text_advance_steps(
        ctx,
        text,
        style,
        paragraph_style,
        initial_state,
        next_grapheme_after_text,
        snap_to_chars_cell_width_cache,
        |step| {
            steps.push(step);
            Ok(())
        },
    )?;
    Ok((steps, end_state))
}

/// Text segment contained within a wrapped line, sharing style metadata.
#[derive(Clone)]
pub struct TextSegment {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub properties: Option<Rc<RunProperties>>,
    pub revision: Option<RevisionMark>,
    pub hyperlink: Option<RunHyperlink>,
    pub leading_width: f64,
}

/// Reason that a wrapped line stopped (soft break, paragraph boundary, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreakKind {
    Soft,
    Hard,
    Page,
    ParagraphEnd,
}

/// Alignment excludes a soft wrap's trailing ASCII separators while glyph and
/// caret advances remain intact. Reverse glyph positions come from shared layout.
/// Unknown text, tabs, nonbreaking spaces and explicit breaks stay conservative.
pub fn soft_line_alignment_width_px<'a>(
    natural_advance: f64,
    break_kind: LineBreakKind,
    reverse_glyphs: impl IntoIterator<Item = (&'a str, f64, f64)>,
) -> f64 {
    if break_kind != LineBreakKind::Soft || !natural_advance.is_finite() {
        return natural_advance;
    }
    let mut has_separator = false;
    for (text, x, width) in reverse_glyphs {
        if text.is_empty() {
            return natural_advance;
        }
        if text.bytes().all(|byte| byte == b' ') {
            has_separator = true;
            continue;
        }
        let end = x + width;
        return if has_separator && end.is_finite() {
            end.clamp(0.0, natural_advance.max(0.0))
        } else {
            natural_advance
        };
    }
    if has_separator {
        0.0
    } else {
        natural_advance
    }
}

/// Layout result representing a single line of wrapped text.
#[derive(Clone)]
pub struct WrappedLine {
    pub segments: Vec<TextSegment>,
    pub break_kind: LineBreakKind,
    pub break_range: Option<(usize, usize)>,
    pub break_properties: Option<Rc<RunProperties>>,
    pub leaders: Vec<TabLeaderSegment>,
    pub tab_stops: Vec<TabStopSegment>,
    pub clusters: Vec<InlineCluster>,
}

impl WrappedLine {
    /// The line's shared font box, excluding blank clusters when text is
    /// visible. Missing metrics retain the paragraph's conservative estimate.
    pub fn font_box_px(&self) -> Option<(f64, f64)> {
        self.line_box_px(false)
    }

    /// Physical content bounds, including a known paragraph mark on a final
    /// object-only line. The mark does not change the object's own paint bounds.
    pub fn content_box_px_with_paragraph_mark(
        &self,
        mark_font_box: Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        let (mut ascent, mut descent) = self.line_box_px(true)?;
        if let Some((mark_ascent, mark_descent)) =
            Self::valid_mac_mark_box(mark_font_box).filter(|_| self.is_object_only_paragraph_end())
        {
            ascent = ascent.max(mark_ascent);
            descent = descent.max(mark_descent);
        }
        Some((ascent, descent))
    }

    fn valid_mac_mark_box(mark_font_box: Option<(f64, f64)>) -> Option<(f64, f64)> {
        mark_font_box.filter(|(ascent, descent)| {
            crate::line_metrics::word_layout_platform()
                == crate::line_metrics::WordLayoutPlatform::Mac
                && ascent.is_finite()
                && descent.is_finite()
                && *ascent >= 0.0
                && *descent >= 0.0
        })
    }

    fn is_object_only_paragraph_end(&self) -> bool {
        self.break_kind == LineBreakKind::ParagraphEnd
            && !self.clusters.is_empty()
            && self
                .clusters
                .iter()
                .all(|cluster| cluster.inline_object.is_some())
    }

    /// Include the final object/mark union in pagination's physical line plan.
    /// Other paragraphs retain the preceding allocation-free fast path.
    pub fn line_heights_px_with_paragraph_mark(
        lines: &[Self],
        style: &ParagraphStyle,
        fallback_px: f64,
        mark_font_box: Option<(f64, f64)>,
    ) -> Option<Vec<f64>> {
        let Some(mark_font_box) = Self::valid_mac_mark_box(mark_font_box)
            .filter(|_| lines.last().is_some_and(Self::is_object_only_paragraph_end))
        else {
            return Self::line_heights_px(lines, style, fallback_px);
        };
        Some(
            lines
                .iter()
                .map(|line| {
                    let height = line.line_height_px(style, fallback_px);
                    if !line.is_object_only_paragraph_end() {
                        return height;
                    }
                    let Some((ascent, descent)) =
                        line.content_box_px_with_paragraph_mark(Some(mark_font_box))
                    else {
                        return height;
                    };
                    style
                        .word_grid_line_sizing
                        .map_or(height.max(ascent + descent), |grid| {
                            grid.line_height_px(Some(ascent + descent), fallback_px)
                        })
                })
                .collect(),
        )
    }

    /// Extend only the first physical line for a list label's ascent. Existing
    /// grid plans are reused; unchanged constant heights allocate nothing.
    pub fn line_heights_px_with_numbering_ascent(
        lines: &[Self],
        style: &ParagraphStyle,
        fallback_px: f64,
        mark_font_box: Option<(f64, f64)>,
        label_ascent: Option<f64>,
    ) -> Option<Vec<f64>> {
        let mut heights =
            Self::line_heights_px_with_paragraph_mark(lines, style, fallback_px, mark_font_box);
        // Without a shared grid/auto plan the paragraph's resolved spacing
        // owns the height, including authored exact spacing.
        if style.word_grid_line_sizing.is_none() && style.word_proportional_line_sizing.is_none() {
            return heights;
        }
        let Some(label_ascent) = label_ascent.filter(|ascent| ascent.is_finite() && *ascent >= 0.0)
        else {
            return heights;
        };
        let first_band_end = (1..lines.len())
            .find(|&index| !style.line_shares_float_band(index))
            .unwrap_or(lines.len());
        for (index, line) in lines[..first_band_end].iter().enumerate() {
            let Some(body) = line.content_box_px_with_paragraph_mark(mark_font_box) else {
                continue;
            };
            if label_ascent <= body.0 {
                continue;
            }
            let (ascent, descent) =
                crate::line_metrics::line_font_box_with_numbering_ascent(body, Some(label_ascent));
            let natural = Some(ascent + descent);
            let height = if let Some(grid) = style.word_grid_line_sizing {
                grid.line_height_px(natural, fallback_px)
            } else if let Some(sizing) = style.word_proportional_line_sizing {
                sizing.line_height_px(natural, fallback_px)
            } else {
                fallback_px.max(ascent + descent)
            };
            let previous = heights.as_ref().map_or(fallback_px, |plan| plan[index]);
            if (height - previous).abs() > 0.000001 {
                let plan = heights.get_or_insert_with(|| vec![fallback_px; lines.len()]);
                plan[index] = height;
            }
        }
        heights
    }

    fn line_box_px(&self, include_objects: bool) -> Option<(f64, f64)> {
        if self.clusters.is_empty()
            && self.break_kind == LineBreakKind::Hard
            && crate::line_metrics::word_layout_platform()
                == crate::line_metrics::WordLayoutPlatform::Mac
        {
            // A line containing only a manual break takes that run's Latin
            // font box, not the tallest visible face elsewhere in the paragraph.
            // Require resolved size/font inputs; partial or unknown properties
            // retain the caller's conservative paragraph estimate.
            let properties = self.break_properties.as_deref()?;
            let size = properties.font_size?;
            if !size.is_finite()
                || size <= 0.0
                || !(properties.font_family.is_some()
                    || properties
                        .fonts
                        .as_ref()
                        .is_some_and(|fonts| fonts.ascii.is_some() || fonts.h_ansi.is_some()))
            {
                return None;
            }
            let style = crate::font_resolver::resolve_run_style_for_wrap(Some(properties), None);
            let (ascent, descent) = crate::line_metrics::word_font_vertical_metrics_px(
                &style.font_descriptor_latin_ascii,
            )?;
            let border = points_to_px(crate::line_metrics::word_run_border_extent_pt(Some(
                properties,
            )));
            return Some((ascent + border, descent + border));
        }
        let visible = self.clusters.iter().any(|cluster| !cluster.is_whitespace);
        let mut extent: Option<(f64, f64)> = None;
        for cluster in &self.clusters {
            if visible && cluster.is_whitespace {
                continue;
            }
            let (a, d) = if include_objects && cluster.inline_object.is_some() {
                let (top, bottom) = cluster.inline_object_bounds_px()?;
                ((-top).max(0.0), bottom.max(0.0))
            } else if cluster.inline_object.is_some() {
                return None;
            } else {
                let layout = cluster.layout.as_ref()?;
                (layout.font_ascent?, layout.font_descent?)
            };
            if !a.is_finite() || !d.is_finite() || a < 0.0 || d < 0.0 {
                return None;
            }
            extent =
                Some(extent.map_or((a, d), |(ascent, descent)| (ascent.max(a), descent.max(d))));
        }
        extent
    }

    pub fn line_height_px(&self, style: &ParagraphStyle, fallback_px: f64) -> f64 {
        if let Some(grid) = style.word_grid_line_sizing {
            return grid.line_height_px(self.line_box_px(true).map(|(a, d)| a + d), fallback_px);
        }
        style
            .word_proportional_line_sizing
            .map_or(fallback_px, |sizing| {
                sizing.line_height_px(self.font_box_px().map(|(a, d)| a + d), fallback_px)
            })
    }

    /// Allocate a physical-line plan only for grids or auto-spaced paragraphs
    /// whose lines differ from the paragraph estimate. A single rendered field
    /// can differ from its empty source-run estimate too. Matching heights keep
    /// the allocation-free path regardless of line count.
    pub fn line_heights_px(
        lines: &[Self],
        style: &ParagraphStyle,
        fallback_px: f64,
    ) -> Option<Vec<f64>> {
        if style.word_grid_line_sizing.is_some() {
            return Some(
                lines
                    .iter()
                    .map(|line| line.line_height_px(style, fallback_px))
                    .collect(),
            );
        }
        style.word_proportional_line_sizing?;
        let mut heights: Option<Vec<f64>> = None;
        for (index, line) in lines.iter().enumerate() {
            let height = line.line_height_px(style, fallback_px);
            if heights.is_none() && (height - fallback_px).abs() > 0.000001 {
                let mut plan = Vec::with_capacity(lines.len());
                plan.resize(index, fallback_px);
                heights = Some(plan);
            }
            if let Some(plan) = &mut heights {
                plan.push(height);
            }
        }
        heights
    }

    /// Expands interior word spaces of a justified soft line to its content
    /// edge. The trailing separator keeps its natural advance outside that
    /// edge. Consumers retain these cluster positions for painting and carets.
    pub fn justify_word_spaces_to_width(&mut self, available_width: f64) {
        if self.break_kind != LineBreakKind::Soft || !self.tab_stops.is_empty() {
            return;
        }
        let interior_end = self
            .clusters
            .iter()
            .rposition(|cluster| !cluster.is_whitespace)
            .map_or(0, |index| index + 1);
        let spaces = self.clusters[..interior_end]
            .iter()
            .filter(|cluster| cluster.kind == InlineClusterKind::Grapheme && cluster.is_whitespace)
            .count();
        if spaces == 0 {
            return;
        }
        let natural_advance = self.clusters.iter().map(|cluster| cluster.width).sum();
        let extra = self.justification_extra_width_px(available_width, natural_advance, 0.0);
        if extra <= 0.0 || !extra.is_finite() {
            return;
        }
        let per_space = extra / spaces as f64;
        for cluster in &mut self.clusters[..interior_end] {
            if cluster.kind == InlineClusterKind::Grapheme && cluster.is_whitespace {
                cluster.width += per_space;
                if let Some(layout) = cluster.layout.as_mut() {
                    layout.glyph_box_width += per_space;
                }
            }
        }
    }

    /// Fill a distributed line, including its final paragraph line, by
    /// expanding the advance after each character except the final one.
    /// Tab positions retain their authored stops; trailing spaces stay natural.
    pub fn distribute_characters_to_width(&mut self, available_width: f64) {
        if !self.tab_stops.is_empty() || !available_width.is_finite() {
            return;
        }
        let content_end = self
            .clusters
            .iter()
            .rposition(|c| !c.is_whitespace)
            .map_or(0, |index| index + 1);
        if content_end == 1 {
            // Word centres a lone character on a distributed line.
            let offset = (available_width - self.clusters[0].width).max(0.0) / 2.0;
            if let Some(layout) = self.clusters[0]
                .layout
                .as_mut()
                .filter(|_| offset > f64::EPSILON)
            {
                layout.leading_advance += offset;
                self.clusters[0].width += offset;
            }
            return;
        }
        if content_end < 2 {
            return;
        }
        let natural: f64 = self.clusters[..content_end].iter().map(|c| c.width).sum();
        let extra = (available_width - natural).max(0.0) / (content_end - 1) as f64;
        if extra <= f64::EPSILON {
            return;
        }
        for cluster in &mut self.clusters[..content_end - 1] {
            cluster.width += extra;
            if let Some(layout) = cluster.layout.as_mut() {
                layout.glyph_box_width += extra;
            }
        }
    }

    /// Extra advance for a distributed line. A soft line's trailing separator
    /// and hanging punctuation sit outside its content edge. Natural advance
    /// comes from this line's shared plan.
    pub fn justification_extra_width_px(
        &self,
        available_width: f64,
        natural_advance: f64,
        hanging_punctuation_width: f64,
    ) -> f64 {
        let trailing_space = if self.break_kind == LineBreakKind::Soft {
            trailing_whitespace_width(&self.clusters)
        } else {
            0.0
        };
        let hanging_width = trailing_space.max(hanging_punctuation_width);
        (available_width - (natural_advance - hanging_width)).max(0.0)
    }
}

/// Flattened line information derived from wrapped layout results.
#[derive(Clone, Debug)]
pub struct LineInfo {
    pub index: usize,
    pub text: String,
    pub width: f64,
    pub start: usize,
    pub end: usize,
    pub break_kind: LineBreakKind,
}

/// Converts wrapped lines into simplified `LineInfo` records for assertions/debugging.
pub fn collect_line_infos(lines: &[WrappedLine]) -> Vec<LineInfo> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let text = line
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>();
            let width = line.clusters.iter().map(|cluster| cluster.width).sum();
            let (start, end) = crate::line_geometry::logical_source_range(
                line.segments
                    .iter()
                    .map(|segment| (segment.start, segment.end)),
            )
            .or_else(|| crate::line_geometry::logical_source_range(line.break_range))
            .unwrap_or((0, 0));

            LineInfo {
                index,
                text,
                width,
                start,
                end,
                break_kind: line.break_kind,
            }
        })
        .collect()
}

/// One absolute point on a floating object's wrap contour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatPoint {
    pub x: f64,
    pub y: f64,
}

/// An optional non-rectangular contour for tight/through wrapping.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatWrapPolygon {
    pub points: Arc<[FloatPoint]>,
    pub distance_top: f64,
    pub distance_bottom: f64,
    pub distance_left: f64,
    pub distance_right: f64,
    /// Whether disjoint polygon slices remain separate so text may flow
    /// through concave openings (`wp:wrapThrough`). Tight wrapping keeps one
    /// conservative outer span.
    pub allow_internal_gaps: bool,
}

/// A region occupied by a floating object that body text must wrap around.
/// Coordinates are in the same px space as the paragraph content box. Square
/// wrapping uses the rectangular bounds; tight/through may additionally carry
/// a line-sensitive polygon contour.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatExclusion {
    /// Top edge of the exclusion band (px, smaller = higher on the page).
    pub top: f64,
    /// Bottom edge of the exclusion band (px).
    pub bottom: f64,
    /// Left edge of the occupied horizontal interval (px).
    pub left: f64,
    /// Right edge of the occupied horizontal interval (px).
    pub right: f64,
    /// Which side is eligible for text when this exclusion intersects a line.
    pub wrap_text: WrapTextSide,
    /// Absolute contour for tight/through wrapping. Invalid contours fall back
    /// to the rectangular bounds above.
    pub polygon: Option<FloatWrapPolygon>,
}

/// One usable physical line segment after excluding floating objects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatLineSegment {
    pub start_x: f64,
    pub width: f64,
    /// Fully covered line bands between this line and the preceding usable
    /// line. For the first result this counts bands above it.
    pub skipped_lines_before: usize,
    /// Geometric clearance before this segment, independent of the eventual
    /// text line's height. Zero retains the full-band count for line-grid plans.
    pub exact_clearance_height: f64,
    /// This segment lies on the same physical line band as the previous
    /// one: `wrapText="bothSides"` text continues on the far side of an
    /// object instead of starting a new band.
    pub shares_band_with_previous: bool,
}

/// Minimum width of each free side before `wrapText="bothSides"` fills both
/// sides of one line band; narrower slivers keep the single widest side.
/// Word applies a minimum too, but its value is not measured yet (#248).
pub const BOTH_SIDES_MIN_SEGMENT_PX: f64 = 72.0;

impl FloatExclusion {
    /// Whether this exclusion's vertical band overlaps the half-open line band
    /// `[line_top, line_bottom)`.  Touching edges do not count as overlap.
    fn overlaps_line(&self, line_top: f64, line_bottom: f64) -> bool {
        self.top < line_bottom && self.bottom > line_top && self.right > self.left
    }

    fn occupied_intervals_for_line(&self, line_top: f64, line_bottom: f64) -> Vec<(f64, f64)> {
        let Some(polygon) = self.polygon.as_ref() else {
            return vec![(self.left, self.right)];
        };
        let band_top = line_top - polygon.distance_bottom.max(0.0);
        let band_bottom = line_bottom + polygon.distance_top.max(0.0);
        if polygon.allow_internal_gaps {
            let Some(mut intervals) =
                polygon_horizontal_intervals(&polygon.points, band_top, band_bottom)
            else {
                return vec![(self.left, self.right)];
            };
            for (left, right) in &mut intervals {
                *left -= polygon.distance_left.max(0.0);
                *right += polygon.distance_right.max(0.0);
            }
            return intervals;
        }
        let Some((left, right)) = polygon_horizontal_span(&polygon.points, band_top, band_bottom)
        else {
            return vec![(self.left, self.right)];
        };
        vec![(
            left - polygon.distance_left.max(0.0),
            right + polygon.distance_right.max(0.0),
        )]
    }
}

/// Returns the conservative occupied intervals of a simple polygon over a
/// vertical line band. The band is split at polygon vertex Y coordinates; in
/// each resulting strip the active edges and their left-to-right order are
/// stable, so the endpoint extrema of each even-odd pair bound the whole strip.
/// Disjoint pairs remain separate, which is the key `wrapThrough` distinction.
fn polygon_horizontal_intervals(
    points: &[FloatPoint],
    band_top: f64,
    band_bottom: f64,
) -> Option<Vec<(f64, f64)>> {
    if points.len() < 3
        || band_bottom <= band_top
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| start.x * end.y - end.x * start.y)
        .sum::<f64>();
    if twice_area.abs() <= f64::EPSILON {
        return None;
    }

    let mut critical_y = vec![band_top, band_bottom];
    critical_y.extend(
        points
            .iter()
            .map(|point| point.y)
            .filter(|y| *y > band_top && *y < band_bottom),
    );
    critical_y.sort_by(f64::total_cmp);
    critical_y.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON);

    let mut occupied = Vec::new();
    let mut crossings = Vec::new();
    for strip in critical_y.windows(2) {
        let strip_top = strip[0];
        let strip_bottom = strip[1];
        if strip_bottom - strip_top <= f64::EPSILON {
            continue;
        }
        let mid_y = (strip_top + strip_bottom) / 2.0;
        crossings.clear();
        for (start, end) in points
            .iter()
            .copied()
            .zip(points.iter().copied().cycle().skip(1))
            .take(points.len())
        {
            let min_y = start.y.min(end.y);
            let max_y = start.y.max(end.y);
            if (end.y - start.y).abs() <= f64::EPSILON || mid_y <= min_y || mid_y >= max_y {
                continue;
            }
            let x_at = |y: f64| start.x + (y - start.y) / (end.y - start.y) * (end.x - start.x);
            crossings.push((x_at(mid_y), x_at(strip_top), x_at(strip_bottom)));
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        if crossings.len() % 2 != 0 {
            return None;
        }
        for pair in crossings.chunks_exact(2) {
            let left = pair[0].1.min(pair[0].2).min(pair[1].1).min(pair[1].2);
            let right = pair[0].1.max(pair[0].2).max(pair[1].1).max(pair[1].2);
            if right > left {
                occupied.push((left, right));
            }
        }
    }

    occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged_len = 0;
    for index in 0..occupied.len() {
        let (left, right) = occupied[index];
        if merged_len > 0 && left <= occupied[merged_len - 1].1 {
            occupied[merged_len - 1].1 = occupied[merged_len - 1].1.max(right);
        } else {
            occupied[merged_len] = (left, right);
            merged_len += 1;
        }
    }
    occupied.truncate(merged_len);
    Some(occupied)
}

/// Returns the conservative horizontal span of a closed polygon over a
/// vertical line band. Sampling vertices inside the band plus every edge's
/// intersections with the band boundaries captures the extrema of each linear
/// segment without rasterization.
fn polygon_horizontal_span(
    points: &[FloatPoint],
    band_top: f64,
    band_bottom: f64,
) -> Option<(f64, f64)> {
    if points.len() < 3 || band_bottom <= band_top {
        return None;
    }
    let mut left = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut include_x = |x: f64| {
        if x.is_finite() {
            left = left.min(x);
            right = right.max(x);
        }
    };

    for point in points {
        if point.x.is_finite()
            && point.y.is_finite()
            && point.y >= band_top
            && point.y <= band_bottom
        {
            include_x(point.x);
        }
    }
    for index in 0..points.len() {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        if !start.x.is_finite() || !start.y.is_finite() || !end.x.is_finite() || !end.y.is_finite()
        {
            continue;
        }
        let min_y = start.y.min(end.y);
        let max_y = start.y.max(end.y);
        if max_y < band_top || min_y > band_bottom {
            continue;
        }
        if (end.y - start.y).abs() <= f64::EPSILON {
            include_x(start.x);
            include_x(end.x);
            continue;
        }
        for y in [band_top, band_bottom] {
            if y >= min_y && y <= max_y {
                let ratio = (y - start.y) / (end.y - start.y);
                include_x(start.x + ratio * (end.x - start.x));
            }
        }
    }

    (left.is_finite() && right.is_finite() && right > left).then_some((left, right))
}

/// Computes the widest free horizontal segment available to a single text line
/// whose vertical band is `[line_top, line_bottom)`, given the paragraph content
/// box `[content_left, content_right]` and the set of floating-object
/// exclusions. Returns `(start_x, width)` of the widest eligible gap not covered
/// by any vertically-overlapping exclusion. Left/right-only policies close the
/// opposite side before the free intervals are selected.
///
/// MVP behaviour: a line wraps into the single widest free interval (Word can
/// split text across both sides of a narrow float, but the common case is text
/// flowing down one side of an image).  When a float spans the entire content
/// box on this line the returned width is `0.0`, signalling the caller to push
/// the line below the float.
pub fn line_available_segment(
    line_top: f64,
    line_bottom: f64,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> (f64, f64) {
    line_available_segment_with_contour_bottom(
        line_top,
        line_bottom,
        line_bottom,
        content_left,
        content_right,
        exclusions,
    )
}

fn line_available_segment_with_contour_bottom(
    line_top: f64,
    line_bottom: f64,
    contour_bottom: f64,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> (f64, f64) {
    if content_right <= content_left {
        return (content_left, 0.0);
    }

    // Most lines intersect only a few floats, so keep their intervals inline.
    let mut occupied: smallvec::SmallVec<[(f64, f64); 4]> = smallvec::SmallVec::new();
    for excl in exclusions.iter().filter(|excl| {
        excl.overlaps_line(
            line_top,
            if excl.polygon.is_some() {
                contour_bottom
            } else {
                line_bottom
            },
        )
    }) {
        let rectangle = [(excl.left, excl.right)];
        let polygon_intervals = excl
            .polygon
            .as_ref()
            .map(|_| excl.occupied_intervals_for_line(line_top, contour_bottom));
        let intervals = polygon_intervals.as_deref().unwrap_or(&rectangle);
        if intervals.is_empty() {
            continue;
        }
        let interval = match excl.wrap_text {
            WrapTextSide::Left => Some((
                intervals
                    .iter()
                    .map(|(left, _)| *left)
                    .fold(f64::INFINITY, f64::min)
                    .max(content_left),
                content_right,
            )),
            WrapTextSide::Right => Some((
                content_left,
                intervals
                    .iter()
                    .map(|(_, right)| *right)
                    .fold(f64::NEG_INFINITY, f64::max)
                    .min(content_right),
            )),
            WrapTextSide::BothSides => {
                occupied.extend(intervals.iter().filter_map(|&(left, right)| {
                    let clipped = (left.max(content_left), right.min(content_right));
                    (clipped.1 > clipped.0).then_some(clipped)
                }));
                None
            }
            // `largest` uses the polygon's outer span; `bothSides` keeps
            // internal openings available for through wrapping.
            WrapTextSide::Largest => Some((
                intervals
                    .iter()
                    .map(|(left, _)| *left)
                    .fold(f64::INFINITY, f64::min)
                    .max(content_left),
                intervals
                    .iter()
                    .map(|(_, right)| *right)
                    .fold(f64::NEG_INFINITY, f64::max)
                    .min(content_right),
            )),
        };
        if let Some(interval) = interval.filter(|(left, right)| right > left) {
            occupied.push(interval);
        }
    }

    if occupied.is_empty() {
        return (content_left, content_right - content_left);
    }

    // Merge overlapping/adjacent occupied intervals.
    occupied.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged_len = 0;
    for index in 0..occupied.len() {
        let (left, right) = occupied[index];
        if merged_len > 0 && left <= occupied[merged_len - 1].1 {
            occupied[merged_len - 1].1 = occupied[merged_len - 1].1.max(right);
        } else {
            occupied[merged_len] = (left, right);
            merged_len += 1;
        }
    }

    // Walk the free gaps between merged occupied intervals and keep the widest.
    let mut best_start = content_left;
    let mut best_width = 0.0_f64;
    let mut cursor = content_left;
    for (l, r) in &occupied[..merged_len] {
        let gap = l - cursor;
        if gap > best_width {
            best_width = gap;
            best_start = cursor;
        }
        cursor = cursor.max(*r);
    }
    let trailing = content_right - cursor;
    if trailing > best_width {
        best_width = trailing;
        best_start = cursor;
    }

    (best_start, best_width.max(0.0))
}

/// Every horizontal interval a line band may hold text in, left to right.
///
/// Word fills both sides of a square-wrapped object whose `wrapText` is
/// `bothSides`: the band's text runs up to the object and continues past it.
/// That applies when every exclusion on the band is a `bothSides`
/// region, using its rectangular bounds or sampled tight/through contour,
/// and at least two free gaps are as wide as
/// [`BOTH_SIDES_MIN_SEGMENT_PX`]. Otherwise the band has the single interval
/// of [`line_available_segment`].
pub fn line_available_segments(
    line_top: f64,
    line_bottom: f64,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> smallvec::SmallVec<[(f64, f64); 2]> {
    line_available_segments_with_contour_bottom(
        line_top,
        line_bottom,
        line_bottom,
        content_left,
        content_right,
        exclusions,
    )
}

fn line_available_segments_with_contour_bottom(
    line_top: f64,
    line_bottom: f64,
    contour_bottom: f64,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> smallvec::SmallVec<[(f64, f64); 2]> {
    let single = || {
        smallvec::smallvec![line_available_segment_with_contour_bottom(
            line_top,
            line_bottom,
            contour_bottom,
            content_left,
            content_right,
            exclusions,
        )]
    };
    if content_right <= content_left {
        return single();
    }
    let mut occupied: smallvec::SmallVec<[(f64, f64); 4]> = smallvec::SmallVec::new();
    for exclusion in exclusions.iter().filter(|exclusion| {
        exclusion.overlaps_line(
            line_top,
            if exclusion.polygon.is_some() {
                contour_bottom
            } else {
                line_bottom
            },
        )
    }) {
        if exclusion.wrap_text != WrapTextSide::BothSides {
            return single();
        }
        if exclusion.polygon.is_some() {
            for (left, right) in exclusion.occupied_intervals_for_line(line_top, contour_bottom) {
                let clipped = (left.max(content_left), right.min(content_right));
                if clipped.1 > clipped.0 {
                    occupied.push(clipped);
                }
            }
        } else {
            // Retain the allocation-free rectangular fast path.
            let clipped = (
                exclusion.left.max(content_left),
                exclusion.right.min(content_right),
            );
            if clipped.1 > clipped.0 {
                occupied.push(clipped);
            }
        }
    }
    if occupied.is_empty() {
        return single();
    }
    occupied.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut gaps: smallvec::SmallVec<[(f64, f64); 2]> = smallvec::SmallVec::new();
    let mut cursor = content_left;
    for (left, right) in occupied {
        if left - cursor >= BOTH_SIDES_MIN_SEGMENT_PX {
            gaps.push((cursor, left - cursor));
        }
        cursor = cursor.max(right);
    }
    if content_right - cursor >= BOTH_SIDES_MIN_SEGMENT_PX {
        gaps.push((cursor, content_right - cursor));
    }
    if gaps.len() >= 2 {
        gaps
    } else {
        single()
    }
}

/// Plans the horizontal text interval for each line of a paragraph that is laid
/// out starting at `first_line_top` and advances by `line_height` per line,
/// when the paragraph must wrap around the floating objects in `exclusions`.
///
/// Returns one `(start_x, width)` pair per line index in `0..line_count`, where
/// `start_x` is the left edge the line's text should begin at and `width` is the
/// usable horizontal space, both computed by intersecting the line's vertical
/// band `[top, top + line_height)` with the exclusions through
/// [`line_available_segment`].
///
/// This is the pure cumulative-Y bridge between the per-line geometry core and
/// the wrap loop / renderer: the wrap engine currently works in width-space only
/// and never tracks a line's vertical position, so this helper isolates the Y
/// walk as a side-effect-free function that can be unit tested on its own and
/// later fed line heights from the layout pass. A `width` of `0.0` signals that
/// a float fully covers the content box on that line (the caller should push the
/// line below the float). When `exclusions` is empty every entry is the full
/// content box, matching the no-float behaviour.
pub fn plan_line_segments_around_floats(
    first_line_top: f64,
    line_height: f64,
    line_count: usize,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> Vec<(f64, f64)> {
    let safe_height = if line_height.is_finite() && line_height > 0.0 {
        line_height
    } else {
        0.0
    };
    (0..line_count)
        .map(|index| {
            let line_top = first_line_top + safe_height * index as f64;
            let line_bottom = line_top + safe_height;
            line_available_segment(
                line_top,
                line_bottom,
                content_left,
                content_right,
                exclusions,
            )
        })
        .collect()
}

/// Plans only usable line bands and records how many fully covered bands must
/// be skipped before each one. This lets callers push text below a full-width
/// float without manufacturing empty text/selection lines.
pub fn plan_usable_line_segments_around_floats(
    first_line_top: f64,
    line_height: f64,
    physical_line_count: usize,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> Vec<FloatLineSegment> {
    plan_usable_line_segments_around_floats_impl::<false>(
        first_line_top,
        line_height,
        None,
        physical_line_count,
        content_left,
        content_right,
        exclusions,
    )
}

/// Plans usable bands with exact geometric clearance below fully blocking
/// floats. Nongrid text starts at the exclusion boundary rather than rounding
/// the unused vertical space to an entire line. Horizontal segments and shared
/// bands retain the same policies as the line-grid planner.
pub fn plan_usable_line_segments_around_floats_with_exact_clearance(
    first_line_top: f64,
    line_height: f64,
    physical_line_count: usize,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> Vec<FloatLineSegment> {
    plan_usable_line_segments_around_floats_impl::<true>(
        first_line_top,
        line_height,
        None,
        physical_line_count,
        content_left,
        content_right,
        exclusions,
    )
}

/// Samples tight/through contours over the font content box while advancing
/// bands by the paragraph line height. Leading is empty space, not additional
/// glyph ink that widens a sloped contour. Rectangular policies retain their
/// existing full-band contract; missing/invalid content height uses that band.
#[allow(clippy::too_many_arguments)]
pub fn plan_usable_line_segments_with_contour_font_box(
    first_line_top: f64,
    line_height: f64,
    contour_height: Option<f64>,
    physical_line_count: usize,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
    exact_clearance: bool,
) -> Vec<FloatLineSegment> {
    if exact_clearance {
        plan_usable_line_segments_around_floats_impl::<true>(
            first_line_top,
            line_height,
            contour_height,
            physical_line_count,
            content_left,
            content_right,
            exclusions,
        )
    } else {
        plan_usable_line_segments_around_floats_impl::<false>(
            first_line_top,
            line_height,
            contour_height,
            physical_line_count,
            content_left,
            content_right,
            exclusions,
        )
    }
}

fn plan_usable_line_segments_around_floats_impl<const EXACT_CLEARANCE: bool>(
    first_line_top: f64,
    line_height: f64,
    contour_height: Option<f64>,
    physical_line_count: usize,
    content_left: f64,
    content_right: f64,
    exclusions: &[FloatExclusion],
) -> Vec<FloatLineSegment> {
    let mut skipped = 0usize;
    let mut skipped_height = 0.0;
    let mut origin_shift = 0.0;
    let mut usable = Vec::with_capacity(physical_line_count);
    let safe_height = if line_height.is_finite() && line_height > 0.0 {
        line_height
    } else {
        0.0
    };
    let contour_height = contour_height
        .filter(|h| h.is_finite() && *h > 0.0)
        .unwrap_or(safe_height);
    for index in 0..physical_line_count {
        let line_top = first_line_top + safe_height * index as f64 + origin_shift;
        let segments = line_available_segments_with_contour_bottom(
            line_top,
            line_top + safe_height,
            line_top + contour_height,
            content_left,
            content_right,
            exclusions,
        );
        let mut band_used = false;
        for (start_x, width) in segments {
            if width <= f64::EPSILON {
                continue;
            }
            usable.push(FloatLineSegment {
                start_x,
                width,
                skipped_lines_before: if band_used {
                    0
                } else {
                    std::mem::take(&mut skipped)
                },
                exact_clearance_height: if band_used {
                    0.0
                } else {
                    std::mem::take(&mut skipped_height)
                },
                shares_band_with_previous: band_used,
            });
            band_used = true;
        }
        if !band_used {
            // A blocked nongrid line can start at the earliest overlapping
            // object's bottom. Recheck all exclusions there: stacked objects
            // or a remaining side obstruction may require another clearance.
            let clearance_bottom = (EXACT_CLEARANCE && safe_height > 0.0)
                .then(|| {
                    exclusions
                        .iter()
                        .filter(|exclusion| {
                            exclusion.overlaps_line(line_top, line_top + safe_height)
                        })
                        .map(|exclusion| exclusion.bottom)
                        .filter(|bottom| bottom.is_finite() && *bottom > line_top)
                        .min_by(f64::total_cmp)
                })
                .flatten();
            if let Some(bottom) = clearance_bottom {
                let advance = bottom - line_top;
                let bands = (advance / safe_height).ceil() as usize;
                skipped += bands;
                skipped_height += advance;
                origin_shift = bottom - (first_line_top + safe_height * (index + 1) as f64);
            } else {
                skipped += 1;
            }
        }
    }
    usable
}

/// Vertical layout result that keeps a single line buffer plus column ranges.
#[derive(Clone)]
pub struct VerticalWrappedLayout {
    pub lines: Vec<WrappedLine>,
    pub column_ranges: Vec<Range<usize>>,
    pub column_height: f64,
    pub max_line_width: f64,
}

/// Builds vertical-stack lines for table-like text paths while reusing the
/// same style resolution and text measurement backend as normal wrapping.
pub fn layout_text_vertically<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    prefer_horizontal_latin: bool,
    base_offset: usize,
    available_height: Option<f64>,
    line_height: f64,
) -> Result<VerticalWrappedLayout, C::Error> {
    fn push_vertical_line(
        lines: &mut Vec<WrappedLine>,
        text: String,
        range: Range<usize>,
        width: f64,
        meta: SegmentMeta<'_>,
    ) {
        // Build the (borrowing) cluster and whitespace flag before moving
        // `text` into the segment, so the grapheme string is never cloned.
        let is_whitespace = text.trim().is_empty();
        let cluster = InlineCluster::grapheme(
            range.start,
            range.end,
            width,
            &text,
            is_whitespace,
            false,
            false,
        );
        let break_properties = meta.properties.clone();
        let mut segments = Vec::with_capacity(1);
        push_text_segment(&mut segments, text, range, meta, 0.0);
        lines.push(WrappedLine {
            segments,
            break_kind: LineBreakKind::Soft,
            break_range: None,
            break_properties,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: vec![cluster],
        });
    }

    fn push_vertical_break_line(
        lines: &mut Vec<WrappedLine>,
        start: usize,
        end: usize,
        properties: &Option<Rc<RunProperties>>,
    ) {
        lines.push(WrappedLine {
            segments: Vec::new(),
            break_kind: LineBreakKind::Hard,
            break_range: Some((start, end)),
            break_properties: properties.clone(),
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: Vec::new(),
        });
    }

    struct PendingAsciiLine {
        text: String,
        start: usize,
        end: usize,
        font_descriptor: Rc<str>,
        text_scale_x: f64,
        properties: Option<Rc<RunProperties>>,
        revision: Option<RevisionMark>,
        hyperlink: Option<RunHyperlink>,
    }

    fn flush_pending_ascii<C: LineWrapContext>(
        ctx: &C,
        lines: &mut Vec<WrappedLine>,
        pending_ascii: &mut Option<PendingAsciiLine>,
        max_line_width: &mut f64,
    ) -> Result<(), C::Error> {
        let Some(pending) = pending_ascii.take() else {
            return Ok(());
        };
        let width = (ctx
            .measure_text_for_vertical_stack(&pending.text, &pending.font_descriptor)?
            * pending.text_scale_x.max(0.01))
        .max(1.0);
        *max_line_width = max_line_width.max(width);
        push_vertical_line(
            lines,
            pending.text,
            pending.start..pending.end,
            width,
            SegmentMeta {
                properties: &pending.properties,
                revision: &pending.revision,
                hyperlink: &pending.hyperlink,
            },
        );
        Ok(())
    }

    let mut lines = Vec::new();
    let mut max_line_width: f64 = 0.0;
    let mut pending_ascii: Option<PendingAsciiLine> = None;

    for run_style in run_styles {
        let start = run_style.start.min(text.len());
        let mut end = run_style.end.min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let mut start_aligned = start;
        while start_aligned > 0 && !text.is_char_boundary(start_aligned) {
            start_aligned = start_aligned.saturating_sub(1);
        }
        let run_text = text.get(start_aligned..end).unwrap_or("");
        if run_text.is_empty() {
            continue;
        }

        let style = ctx.resolve_run_style(run_style.properties.as_deref())?;
        for (rel_offset, grapheme) in crate::graphemes::grapheme_indices(run_text) {
            let grapheme_start_abs = base_offset + start_aligned + rel_offset;
            let grapheme_end_abs = grapheme_start_abs + grapheme.len();

            if grapheme == "\r" {
                continue;
            }

            if grapheme == "\n" {
                flush_pending_ascii(ctx, &mut lines, &mut pending_ascii, &mut max_line_width)?;
                push_vertical_break_line(
                    &mut lines,
                    grapheme_start_abs,
                    grapheme_end_abs,
                    &run_style.properties,
                );
                continue;
            }

            let is_ascii = grapheme.chars().all(|ch| ch.is_ascii() && !ch.is_control());
            if prefer_horizontal_latin && is_ascii {
                let font_descriptor =
                    style.descriptor_for_grapheme_rc(grapheme, ScriptCategory::Latin);
                match pending_ascii.as_mut() {
                    Some(pending)
                        if pending.font_descriptor == font_descriptor
                            && pending.properties == run_style.properties
                            && pending.revision == run_style.revision
                            && pending.hyperlink == run_style.hyperlink
                            && pending.end == grapheme_start_abs =>
                    {
                        pending.text.push_str(grapheme);
                        pending.end = grapheme_end_abs;
                    }
                    Some(_) => {
                        flush_pending_ascii(
                            ctx,
                            &mut lines,
                            &mut pending_ascii,
                            &mut max_line_width,
                        )?;
                        pending_ascii = Some(PendingAsciiLine {
                            text: grapheme.to_string(),
                            start: grapheme_start_abs,
                            end: grapheme_end_abs,
                            font_descriptor,
                            text_scale_x: style.text_scale_x,
                            properties: run_style.properties.clone(),
                            revision: run_style.revision.clone(),
                            hyperlink: run_style.hyperlink.clone(),
                        });
                    }
                    None => {
                        pending_ascii = Some(PendingAsciiLine {
                            text: grapheme.to_string(),
                            start: grapheme_start_abs,
                            end: grapheme_end_abs,
                            font_descriptor,
                            text_scale_x: style.text_scale_x,
                            properties: run_style.properties.clone(),
                            revision: run_style.revision.clone(),
                            hyperlink: run_style.hyperlink.clone(),
                        });
                    }
                }
                continue;
            }

            flush_pending_ascii(ctx, &mut lines, &mut pending_ascii, &mut max_line_width)?;
            let script = classify_script(grapheme);
            let (font_descriptor, font_descriptor_hash, font_descriptor_id) =
                style.descriptor_key_for_grapheme(grapheme, script);
            let width = (ctx.measure_text_for_vertical_stack_with_font_key(
                grapheme,
                font_descriptor,
                font_descriptor_hash,
                font_descriptor_id,
            )? * style.text_scale_x.max(0.01))
            .max(1.0);
            max_line_width = max_line_width.max(width);
            push_vertical_line(
                &mut lines,
                grapheme.to_string(),
                grapheme_start_abs..grapheme_end_abs,
                width,
                SegmentMeta {
                    properties: &run_style.properties,
                    revision: &run_style.revision,
                    hyperlink: &run_style.hyperlink,
                },
            );
        }
    }

    flush_pending_ascii(ctx, &mut lines, &mut pending_ascii, &mut max_line_width)?;

    if lines.is_empty() {
        push_vertical_break_line(&mut lines, base_offset, base_offset, &None);
    }

    let mut column_ranges = Vec::new();
    let mut column_start = 0;
    let mut consumed = 0.0_f64;
    let mut natural_height = 0.0_f64;
    let limit = available_height.map(|height| height.max(line_height));
    for (index, line) in lines.iter().enumerate() {
        // Painting advances by measured cluster widths, including tcFitText.
        // An explicit break terminates the column without wrapping its marker.
        let hard_break = line.break_kind == LineBreakKind::Hard;
        let advance = if line.clusters.is_empty() {
            line_height.max(1.0)
        } else {
            line.clusters
                .iter()
                .map(|cluster| cluster.width)
                .sum::<f64>()
                .max(1.0)
        };
        if !hard_break
            && index > column_start
            && limit.is_some_and(|height| consumed + advance > height + 1e-6)
        {
            column_ranges.push(column_start..index);
            natural_height = natural_height.max(consumed);
            column_start = index;
            consumed = 0.0;
        }
        consumed += advance;
        if hard_break {
            column_ranges.push(column_start..index + 1);
            natural_height = natural_height.max(consumed);
            column_start = index + 1;
            consumed = 0.0;
        }
    }
    if column_start < lines.len() {
        column_ranges.push(column_start..lines.len());
        natural_height = natural_height.max(consumed);
    }
    let column_height = available_height.unwrap_or(natural_height).max(line_height);

    Ok(VerticalWrappedLayout {
        lines,
        column_ranges,
        column_height,
        max_line_width,
    })
}

/// Section writing needs glyph baselines without changing table-stack layout.
/// Reuse measured advances and resolve metrics once per style range.
pub fn layout_section_text_vertically<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    prefer_horizontal_latin: bool,
    base_offset: usize,
    available_height: Option<f64>,
    line_height: f64,
) -> Result<VerticalWrappedLayout, C::Error> {
    let mut layout = layout_text_vertically(
        ctx,
        text,
        run_styles,
        prefer_horizontal_latin,
        base_offset,
        available_height,
        line_height,
    )?;
    let mut run_index = 0;
    let mut resolved_index = None;
    let mut resolved = None;
    for line in &mut layout.lines {
        for cluster in &mut line.clusters {
            while run_index + 1 < run_styles.len()
                && base_offset + run_styles[run_index].end <= cluster.start
            {
                run_index += 1;
            }
            let Some(run) = run_styles.get(run_index) else {
                continue;
            };
            if cluster.start < base_offset + run.start || cluster.end > base_offset + run.end {
                continue;
            }
            if resolved_index != Some(run_index) {
                resolved = Some(ctx.resolve_run_style(run.properties.as_deref())?);
                resolved_index = Some(run_index);
            }
            let style = resolved.as_ref().expect("resolved vertical run");
            let script = classify_script(&cluster.text);
            let (descriptor, hash, _) = style.descriptor_key_for_grapheme(&cluster.text, script);
            if let Some((ascent, descent)) =
                crate::line_metrics::word_vertical_glyph_metrics_px(descriptor)
            {
                cluster.layout = Some(InlineClusterLayout {
                    character_border_edges: 0,
                    script,
                    font_slot: style.font_slot_for_grapheme(&cluster.text, script),
                    leading_advance: 0.0,
                    glyph_box_width: cluster.width,
                    measured_glyph_width: cluster.width,
                    adjusted_glyph_width: cluster.width,
                    font_descriptor_hash: hash,
                    font_ascent: Some(ascent),
                    font_descent: Some(descent),
                });
            }
        }
    }
    Ok(layout)
}

/// Vertical cell geometry, including paragraph indents along its writing axis.
pub struct VerticalCellLayout {
    pub layout: VerticalWrappedLayout,
    pub column_start_offsets: Vec<f64>,
}

/// Turn the shared horizontal wrap into writing-axis glyph lines. Column
/// boundaries, run context, indents and advances remain those of normal wrap.
#[allow(clippy::too_many_arguments)]
pub fn layout_cell_text_turned<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    base_offset: usize,
    available_height: Option<f64>,
    line_height: f64,
    start_indent: f64,
    style: &ParagraphStyle,
) -> Result<VerticalCellLayout, C::Error> {
    let start_indent = if start_indent.is_finite() {
        start_indent
    } else {
        0.0
    };
    let capacity = (available_height.unwrap_or(1_000_000_000.0) - start_indent).max(1.0);
    let mut wrapped =
        wrap_text_into_lines(ctx, text, run_styles, 0.0, style, base_offset, capacity)?;
    if is_justified_alignment(&style.alignment) {
        for (index, column) in wrapped.iter_mut().enumerate() {
            column.justify_word_spaces_to_width(
                (capacity - style.line_indent(index) - style.right_indent).max(1.0),
            );
        }
    }
    project_cell_text_turned(
        ctx,
        wrapped,
        available_height,
        line_height,
        start_indent,
        style,
    )
}

/// Project finalized shared wrap results, including their existing justification.
/// The owned copy keeps cached columns immutable while attaching glyph font boxes.
pub fn project_cell_text_turned<C: LineWrapContext>(
    ctx: &C,
    mut wrapped: Vec<WrappedLine>,
    available_height: Option<f64>,
    line_height: f64,
    start_indent: f64,
    style: &ParagraphStyle,
) -> Result<VerticalCellLayout, C::Error> {
    let mut lines = Vec::new();
    let mut column_ranges = Vec::with_capacity(wrapped.len());
    let mut offsets = Vec::with_capacity(wrapped.len());
    let mut natural_height = line_height;
    let mut glyph_style: Option<Rc<ResolvedRunStyle>> = None;
    let mut glyph_properties: Option<Rc<RunProperties>> = None;
    for (column_index, column) in wrapped.iter_mut().enumerate() {
        let offset = start_indent + style.line_indent(column_index);
        natural_height = natural_height.max(
            offset + column.clusters.iter().map(|c| c.width).sum::<f64>() + style.right_indent,
        );
        let first = lines.len();
        if column.clusters.is_empty() {
            lines.push(column.clone());
        } else {
            let count = column.clusters.len();
            for (index, cluster) in column.clusters.iter_mut().enumerate() {
                let segment = column
                    .segments
                    .iter()
                    .find(|s| s.start <= cluster.start && cluster.end <= s.end);
                let properties = segment.and_then(|s| s.properties.as_ref());
                let same_properties = match (glyph_properties.as_ref(), properties) {
                    (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if glyph_style.is_none() || !same_properties {
                    glyph_style = Some(ctx.resolve_run_style(properties.map(|p| p.as_ref()))?);
                    glyph_properties = properties.cloned();
                }
                let resolved = glyph_style.as_ref().expect("resolved turned run");
                let (descriptor, _, _) = resolved
                    .descriptor_key_for_grapheme(&cluster.text, classify_script(&cluster.text));
                if let (Some(layout), Some((ascent, descent))) = (
                    cluster.layout.as_mut(),
                    crate::line_metrics::word_vertical_glyph_metrics_px(descriptor),
                ) {
                    layout.font_ascent = Some(ascent);
                    layout.font_descent = Some(descent);
                }
                let last = index + 1 == count;
                lines.push(WrappedLine {
                    segments: vec![TextSegment {
                        text: cluster.text.to_string(),
                        start: cluster.start,
                        end: cluster.end,
                        properties: segment.and_then(|s| s.properties.clone()),
                        revision: segment.and_then(|s| s.revision.clone()),
                        hyperlink: segment.and_then(|s| s.hyperlink.clone()),
                        leading_width: 0.0,
                    }],
                    clusters: vec![cluster.clone()],
                    break_kind: if last {
                        column.break_kind
                    } else {
                        LineBreakKind::Soft
                    },
                    break_range: if last { column.break_range } else { None },
                    break_properties: if last {
                        column.break_properties.clone()
                    } else {
                        None
                    },
                    // Retain the column's shared tab geometry on its first line.
                    leaders: if index == 0 {
                        column.leaders.clone()
                    } else {
                        Vec::new()
                    },
                    tab_stops: if index == 0 {
                        column.tab_stops.clone()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        column_ranges.push(first..lines.len());
        offsets.push(offset);
    }
    Ok(VerticalCellLayout {
        layout: VerticalWrappedLayout {
            lines,
            column_ranges,
            column_height: available_height.unwrap_or(natural_height).max(line_height),
            max_line_width: line_height,
        },
        column_start_offsets: offsets,
    })
}

/// Font baseline projected into the column pitch of a turned line.
pub fn wrapped_line_turned_baseline_offset(
    line: &WrappedLine,
    column_pitch: f64,
    clockwise: bool,
) -> Option<f64> {
    let (ascent, descent) = line.font_box_px()?;
    Some(
        (column_pitch - ascent - descent).max(0.0) * 0.5 + if clockwise { descent } else { ascent },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn layout_cell_text_vertically<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    base_offset: usize,
    available_height: Option<f64>,
    line_height: f64,
    start_indent: f64,
    style: &ParagraphStyle,
) -> Result<VerticalCellLayout, C::Error> {
    let mut layout = layout_section_text_vertically(
        ctx,
        text,
        run_styles,
        false,
        base_offset,
        None,
        line_height,
    )?;
    let finite = |value: f64| if value.is_finite() { value } else { 0.0 };
    let start_indent = finite(start_indent);
    let end_indent = finite(style.right_indent);
    let first_indent = finite(style.first_line_indent);
    let mut ranges = Vec::new();
    let mut offsets = Vec::new();
    let mut column_start = 0;
    let mut consumed = 0.0;
    let mut natural_height = 0.0_f64;
    let mut offset = start_indent + first_indent;
    for (index, line) in layout.lines.iter().enumerate() {
        let advance = if line.clusters.is_empty() {
            line_height.max(1.0)
        } else {
            line.clusters
                .iter()
                .map(|cluster| cluster.width)
                .sum::<f64>()
                .max(1.0)
        };
        let hard = line.break_kind == LineBreakKind::Hard;
        let capacity = available_height.map(|height| (height - offset - end_indent).max(1.0));
        if !hard
            && index > column_start
            && capacity.is_some_and(|height| consumed + advance > height + 1e-6)
        {
            ranges.push(column_start..index);
            offsets.push(offset);
            natural_height = natural_height.max(offset + consumed + end_indent);
            column_start = index;
            consumed = 0.0;
            offset = start_indent;
        }
        consumed += advance;
        if hard {
            ranges.push(column_start..index + 1);
            offsets.push(offset);
            natural_height = natural_height.max(offset + consumed + end_indent);
            column_start = index + 1;
            consumed = 0.0;
            offset = start_indent;
        }
    }
    if column_start < layout.lines.len() {
        ranges.push(column_start..layout.lines.len());
        offsets.push(offset);
        natural_height = natural_height.max(offset + consumed + end_indent);
    }
    if is_justified_alignment(&style.alignment) {
        if let Some(height) = available_height {
            for (column_index, range) in ranges.iter().enumerate() {
                // A wrapped column distributes its inter-character gaps. The
                // final column and explicit hard breaks retain natural advances.
                if column_index + 1 == ranges.len()
                    || range.len() < 2
                    || layout.lines[range.end - 1].break_kind == LineBreakKind::Hard
                {
                    continue;
                }
                let column = &mut layout.lines[range.clone()];
                if column.iter().any(|line| line.clusters.len() != 1) {
                    continue;
                }
                let natural: f64 = column.iter().map(|line| line.clusters[0].width).sum();
                let gap = ((height - offsets[column_index] - end_indent - natural)
                    / (column.len() - 1) as f64)
                    .max(0.0);
                let count = column.len();
                for line in &mut column[..count - 1] {
                    line.clusters[0].width += gap;
                }
            }
        }
    }
    layout.column_ranges = ranges;
    layout.column_height = available_height.unwrap_or(natural_height).max(line_height);
    Ok(VerticalCellLayout {
        layout,
        column_start_offsets: offsets,
    })
}

/// Horizontal span where a tab leader (dots, hyphen, etc.) should be painted.
#[derive(Clone, Debug)]
pub struct TabLeaderSegment {
    pub start_x: f64,
    pub end_x: f64,
    pub leader: TabLeader,
    pub properties: Option<Rc<RunProperties>>,
    /// Natural leader width resolved with the same requested-face metrics.
    pub glyph_advance: f64,
    /// Repeated glyph origins resolved by shared layout, including advances.
    pub glyph_positions: Rc<[f64]>,
}

pub fn tab_leader_positions(start_x: f64, end_x: f64, advance: f64) -> Rc<[f64]> {
    tab_leader_positions_with_grid(start_x, end_x, advance, false)
}

fn tab_leader_positions_with_grid(
    start_x: f64,
    end_x: f64,
    advance: f64,
    on_grid: bool,
) -> Rc<[f64]> {
    let mut positions = Vec::new();
    if ![start_x, end_x, advance]
        .iter()
        .all(|value| value.is_finite())
        || advance <= 0.0
    {
        return positions.into();
    }
    let mut x = if on_grid {
        (start_x / advance).ceil() * advance
    } else {
        start_x
    };
    while x + advance <= end_x + 0.5 {
        positions.push(x);
        let next = x + advance;
        if next <= x {
            break;
        }
        x = next;
    }
    positions.into()
}

/// Metadata for the gap created by a tab stop (start/end offsets in source text).
#[derive(Clone, Debug)]
pub struct TabStopSegment {
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_x: f64,
    pub end_x: f64,
}

/// Smallest unit (grapheme or tab) used for enforcing wrapping rules.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct InlineCluster {
    pub start: usize,
    pub end: usize,
    pub width: f64,
    /// The grapheme or tab character. Uses `CompactString` so that
    /// graphemes ≤ 16 bytes (the vast majority) are stored inline
    /// without a heap allocation.
    pub text: CompactString,
    pub kind: InlineClusterKind,
    pub is_whitespace: bool,
    pub forbid_line_start: bool,
    pub forbid_line_end: bool,
    /// Whether Word permits this punctuation character to extend past the
    /// right text extent for the parent run's East-Asian editing language.
    pub can_hang_line_end: bool,
    /// Auto spacing (autoSpaceDE/DN) set before and after this cluster's
    /// glyph, inside its width; Word for Mac narrows it to fit a line.
    pub auto_space_leading: f64,
    pub auto_space_trailing: f64,
    pub layout: Option<InlineClusterLayout>,
    pub inline_object: Option<InlineObject>,
}

#[derive(Clone, Debug)]
pub struct InlineClusterLayout {
    /// Bit 0/1 mark the shared left/right character-border extent. This byte
    /// fits existing layout padding; repair can remove and reapply edge space.
    pub character_border_edges: u8,
    pub script: ScriptCategory,
    pub font_slot: ResolvedFontSlot,
    pub leading_advance: f64,
    pub glyph_box_width: f64,
    pub measured_glyph_width: f64,
    pub adjusted_glyph_width: f64,
    pub font_descriptor_hash: u64,
    /// Font metrics for text; signed baseline extents for positioned objects.
    pub font_ascent: Option<f64>,
    pub font_descent: Option<f64>,
}

#[inline]
fn character_border_extent_px(border: Option<&BorderEdge>) -> f64 {
    border.map_or(0.0, |border| {
        (border.size_eighths.unwrap_or(0) as f64 / 8.0 + border.space.unwrap_or(0) as f64) * 96.0
            / 72.0
    })
}

fn line_has_character_border(line: &WrappedLine) -> bool {
    line.segments.iter().any(|segment| {
        segment
            .properties
            .as_deref()
            .and_then(|p| p.border.as_ref())
            .is_some_and(|border| border.is_visible())
    })
}

/// Border width and space belong to the line advance, outside the glyphs.
/// Equal adjacent run borders form one group (ISO/IEC 29500 §17.3.2.4).
/// Reapply after width/kinsoku repair so each physical line closes its groups.
fn apply_character_border_inline_spacing(line: &mut WrappedLine) -> f64 {
    let borders: Vec<_> = line
        .clusters
        .iter()
        .map(|cluster| {
            line.segments
                .iter()
                .find(|segment| segment.start <= cluster.start && cluster.end <= segment.end)
                .and_then(|segment| segment.properties.as_deref())
                .and_then(|properties| properties.border.as_ref())
                .filter(|border| border.is_visible())
        })
        .collect();
    let mut total = 0.0;
    for (index, cluster) in line.clusters.iter_mut().enumerate() {
        let Some(layout) = cluster.layout.as_mut() else {
            continue;
        };
        let Some(border) = borders[index] else {
            continue;
        };
        let extent = character_border_extent_px(Some(border));
        let old_left = if layout.character_border_edges & 1 != 0 {
            extent
        } else {
            0.0
        };
        let old_right = if layout.character_border_edges & 2 != 0 {
            extent
        } else {
            0.0
        };
        let left = index == 0 || borders[index - 1] != Some(border);
        let right = index + 1 == borders.len() || borders[index + 1] != Some(border);
        let new_left = if left { extent } else { 0.0 };
        let new_right = if right { extent } else { 0.0 };
        layout.leading_advance += new_left - old_left;
        layout.glyph_box_width += new_right - old_right;
        cluster.width += new_left + new_right - old_left - old_right;
        layout.character_border_edges = u8::from(left) | (u8::from(right) << 1);
        total += new_left + new_right;
    }
    total
}

/// The tallest font ascent among a wrapped line's clusters, which is where
/// Word puts the line's baseline below its top. Whitespace only counts on a
/// line without visible text: Word for Mac 16.113 sets a line of 10.5pt
/// ideographic spaces followed by 7pt Century digits 8.42pt tall, like the
/// digits alone.
pub fn wrapped_line_max_font_ascent(line: &WrappedLine) -> Option<f64> {
    let visible = line.clusters.iter().any(|cluster| !cluster.is_whitespace);
    line.clusters
        .iter()
        .filter(|cluster| !visible || !cluster.is_whitespace)
        .filter(|cluster| cluster.inline_object.is_none())
        .filter_map(|cluster| cluster.layout.as_ref()?.font_ascent)
        .reduce(f64::max)
}

/// Baseline for a known font line box in a resolved Word grid line.
/// Unknown boxes and unsnapped lines retain their preceding ascent policy.
pub fn wrapped_line_grid_baseline_offset(
    line: &WrappedLine,
    style: &ParagraphStyle,
    line_height_px: f64,
) -> Option<f64> {
    if style.word_grid_line_sizing.is_some() && line_height_px.is_finite() && line_height_px > 0.0 {
        if let Some((ascent, descent)) = line.font_box_px() {
            return Some(ascent + (line_height_px - ascent - descent).max(0.0) * 0.5);
        }
    }
    wrapped_line_max_font_ascent(line)
}

/// Applies Unicode Bidirectional Algorithm visual order to one already-wrapped
/// line. Wrapping remains defined in logical order; only clusters and their
/// matching style segments are reordered after line boundaries are final.
#[inline]
fn requires_bidi_resolution(line: &WrappedLine, base_rtl: bool) -> bool {
    if base_rtl {
        return true;
    }

    // In an LTR paragraph, LTR text, European numbers, neutrals, and combining
    // marks retain logical cluster order. RTL/Arabic classes and explicit
    // directional controls still require the full UAX #9 resolution below.
    line.clusters.iter().any(|cluster| {
        cluster.text.chars().any(|character| {
            matches!(
                bidi_class(character),
                BidiClass::R
                    | BidiClass::AL
                    | BidiClass::AN
                    | BidiClass::RLE
                    | BidiClass::RLO
                    | BidiClass::RLI
                    | BidiClass::LRE
                    | BidiClass::LRO
                    | BidiClass::LRI
                    | BidiClass::FSI
                    | BidiClass::PDF
                    | BidiClass::PDI
            )
        })
    })
}

fn apply_bidi_visual_order(line: &mut WrappedLine, base_rtl: bool) {
    if line.clusters.len() < 2 || !requires_bidi_resolution(line, base_rtl) {
        return;
    }

    let mut bidi_text = String::new();
    let mut cluster_starts = Vec::with_capacity(line.clusters.len());
    for cluster in &line.clusters {
        cluster_starts.push(bidi_text.len());
        bidi_text.push_str(cluster.text.as_str());
    }
    if bidi_text.is_empty() {
        return;
    }

    let base_level = if base_rtl { Level::rtl() } else { Level::ltr() };
    let bidi = BidiInfo::new(&bidi_text, Some(base_level));
    let paragraph = &bidi.paragraphs[0];
    let resolved_levels = bidi.reordered_levels(paragraph, 0..bidi_text.len());
    let levels = cluster_starts
        .iter()
        .map(|start| resolved_levels[*start])
        .collect::<Vec<_>>();
    let visual_order = BidiInfo::reorder_visual(&levels);
    if visual_order.iter().copied().eq(0..line.clusters.len()) {
        return;
    }

    let logical_clusters = line.clusters.clone();
    let logical_segments = line.segments.clone();
    let mut visual_clusters = Vec::with_capacity(logical_clusters.len());
    let mut visual_segments = Vec::with_capacity(logical_segments.len());
    for logical_index in visual_order {
        let cluster = logical_clusters[logical_index].clone();
        if let Some(segment) = logical_segments
            .iter()
            .find(|segment| segment.start <= cluster.start && cluster.end <= segment.end)
        {
            visual_segments.push(TextSegment {
                text: cluster.text.to_string(),
                start: cluster.start,
                end: cluster.end,
                properties: segment.properties.clone(),
                revision: segment.revision.clone(),
                hyperlink: segment.hyperlink.clone(),
                // The authoritative cluster layout already carries the
                // inter-glyph leading advance.
                leading_width: 0.0,
            });
        }
        visual_clusters.push(cluster);
    }
    line.segments = visual_segments;
    line.clusters = visual_clusters;
}

impl InlineCluster {
    pub fn grapheme(
        start: usize,
        end: usize,
        width: f64,
        text: &str,
        is_whitespace: bool,
        forbid_line_start: bool,
        forbid_line_end: bool,
    ) -> Self {
        Self {
            start,
            end,
            width,
            text: CompactString::new(text),
            kind: InlineClusterKind::Grapheme,
            is_whitespace,
            forbid_line_start,
            forbid_line_end,
            can_hang_line_end: false,
            auto_space_leading: 0.0,
            auto_space_trailing: 0.0,
            layout: None,
            inline_object: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn grapheme_with_layout(
        start: usize,
        end: usize,
        width: f64,
        text: &str,
        is_whitespace: bool,
        forbid_line_start: bool,
        forbid_line_end: bool,
        layout: InlineClusterLayout,
    ) -> Self {
        Self {
            start,
            end,
            width,
            text: CompactString::new(text),
            kind: InlineClusterKind::Grapheme,
            is_whitespace,
            forbid_line_start,
            forbid_line_end,
            can_hang_line_end: false,
            auto_space_leading: 0.0,
            auto_space_trailing: 0.0,
            layout: Some(layout),
            inline_object: None,
        }
    }

    pub fn tab(start: usize, end: usize, width: f64) -> Self {
        Self {
            start,
            end,
            width,
            text: CompactString::new("\t"),
            kind: InlineClusterKind::Tab,
            is_whitespace: false,
            forbid_line_start: false,
            forbid_line_end: false,
            can_hang_line_end: false,
            auto_space_leading: 0.0,
            auto_space_trailing: 0.0,
            layout: None,
            inline_object: None,
        }
    }

    pub fn inline_object(start: usize, end: usize, object: InlineObject) -> Self {
        Self {
            start,
            end,
            width: object.width.max(1.0),
            text: CompactString::new("\u{fffc}"),
            kind: InlineClusterKind::InlineObject,
            is_whitespace: false,
            forbid_line_start: false,
            forbid_line_end: false,
            can_hang_line_end: false,
            auto_space_leading: 0.0,
            auto_space_trailing: 0.0,
            layout: None,
            inline_object: Some(object),
        }
    }

    /// Physical bounds relative to the line baseline, including the authored
    /// run position. Positioned objects reuse the existing inline layout slot;
    /// text clusters and unpositioned objects require no additional storage.
    #[inline]
    pub fn inline_object_bounds_px(&self) -> Option<(f64, f64)> {
        let object = self.inline_object?;
        Some(
            self.layout
                .as_ref()
                .and_then(|layout| {
                    let bounds = (-layout.font_ascent?, layout.font_descent?);
                    (bounds.0.is_finite() && bounds.1.is_finite()).then_some(bounds)
                })
                .unwrap_or((-object.height.max(1.0), 0.0)),
        )
    }

    fn with_object_position(mut self, position_pt: Option<f32>) -> Self {
        let Some(position) = position_pt.filter(|value| value.is_finite() && *value != 0.0) else {
            return self;
        };
        if let Some(object) = self.inline_object {
            let raise = points_to_px(position);
            self.layout = Some(InlineClusterLayout {
                character_border_edges: 0,
                script: ScriptCategory::Latin,
                font_slot: ResolvedFontSlot::LatinAscii,
                leading_advance: 0.0,
                glyph_box_width: self.width,
                measured_glyph_width: self.width,
                adjusted_glyph_width: self.width,
                font_descriptor_hash: 0,
                font_ascent: Some(object.height.max(1.0) + raise),
                font_descent: Some(-raise),
            });
        }
        self
    }
}

/// Classification of an `InlineCluster` for specialized handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineClusterKind {
    Grapheme,
    Tab,
    InlineObject,
}

/// Environment required by `wrap_text_into_lines` (style resolution and measurement).
pub trait LineWrapContext {
    type Error;
    /// Returns the resolved run style that provides descriptors and letter spacing.
    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error>;
    /// Measures a text fragment using the provided font descriptor.
    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error>;
    /// Measures a glyph cell used along the vertical stacking axis. Horizontal
    /// oracle advances and shaping adjustments are not necessarily valid
    /// vertical advances, so specialized contexts can opt out while normal
    /// runtime contexts keep the same measurement backend.
    fn measure_text_for_vertical_stack(
        &self,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        self.measure_text(text, font_descriptor)
    }
    /// Measures a text fragment using a caller-supplied *precomputed* font
    /// descriptor hash (the `font_descriptor_hash` the wrap engine already
    /// threads next to every descriptor). Backends that use a measurement cache
    /// override this to key the lookup on the interned id resolved from the hash,
    /// avoiding a per-grapheme descriptor-string hash / byte compare (S4). The
    /// default forwards to [`Self::measure_text`], so non-caching impls (tests,
    /// tooling) stay correct with no change.
    #[inline]
    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        _font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        self.measure_text(text, font_descriptor)
    }
    /// Measures with a renderer-owned interned id when one was attached to the
    /// resolved style. The default preserves the hash-keyed path for generic
    /// layout contexts that do not own a descriptor interner.
    #[inline]
    fn measure_text_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        _font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        self.measure_text_with_hash(text, font_descriptor, font_descriptor_hash)
    }
    #[inline]
    fn measure_text_for_vertical_stack_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        _font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        self.measure_text_for_vertical_stack(text, font_descriptor)
    }
    #[inline]
    fn measure_text_for_vertical_stack_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        _font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        self.measure_text_for_vertical_stack_with_hash(text, font_descriptor, font_descriptor_hash)
    }
    /// Returns the positive trailing ink overhang beyond advance width for a grapheme.
    fn measure_trailing_overhang(
        &self,
        _text: &str,
        _font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        Ok(0.0)
    }
    /// Returns the full line box height for the given font descriptor when the
    /// backend can provide it. `text_extract` uses this as the same optional
    /// floor that browser rendering applies for grid-snapped paragraphs.
    fn measure_line_box_height(&self, _font_descriptor: &str) -> Result<Option<f64>, Self::Error> {
        Ok(None)
    }
    /// Returns ascent/descent metrics for the font descriptor when available.
    /// This lets downstream renderers reuse wrap-time font metrics instead of
    /// re-querying the backend during fragment assembly.
    fn measure_font_vertical_metrics(
        &self,
        _font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        Ok(None)
    }
}

/// Normalizes raw `TabStop` entries into `ParagraphTabStop` structures.
pub fn resolve_tab_stops(tabs: &[TabStop]) -> Vec<ParagraphTabStop> {
    let mut result: Vec<ParagraphTabStop> = Vec::new();
    for tab in tabs {
        let position_pt = tab.position.unwrap_or(0.0);
        let kind = tab.kind.as_deref().unwrap_or("left");
        if kind.eq_ignore_ascii_case("clear") {
            result.retain(|existing| (existing.position_pt - position_pt).abs() > f32::EPSILON);
            continue;
        }

        let alignment = match kind.to_ascii_lowercase().as_str() {
            "center" => TabAlignment::Center,
            "right" => TabAlignment::Right,
            "decimal" => TabAlignment::Decimal,
            "bar" => TabAlignment::Bar,
            _ => TabAlignment::Left,
        };
        let leader = tab
            .leader
            .as_deref()
            .map(tab_leader_from_str)
            .unwrap_or(TabLeader::None);

        result.retain(|existing| (existing.position_pt - position_pt).abs() > f32::EPSILON);
        result.push(ParagraphTabStop {
            position_pt,
            alignment,
            leader,
        });
    }

    result.sort_by(|a, b| {
        a.position_pt
            .partial_cmp(&b.position_pt)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

/// Applies paragraph indents while preserving Word's independent tab origin.
///
/// `x` becomes the paragraph's indented base. At the same time,
/// [`ParagraphStyle::tab_origin_offset`] records the inverse displacement so
/// explicit/default tabs continue to resolve from the containing text margin.
/// The hanging-indent tab is handled separately by `find_next_tab_stop`
/// because OOXML defines that implicit stop at the paragraph base itself.
/// Default character width for character-unit indents (`w:leftChars` etc.)
/// when the paragraph font size is unknown: the Japanese Word body default.
pub const DEFAULT_INDENT_CHAR_UNIT_PT: f64 = 10.5;

fn char_unit_indent_pt(chars: i32, char_unit_pt: f64) -> f64 {
    // 100 units = 1 character (ECMA-376 §17.3.1.12).
    chars as f64 / 100.0 * char_unit_pt
}

/// Returns the indent to apply for a paragraph honoring `w:mirrorIndents`
/// (ECMA-376 Part 1 §17.3.1.18). When mirroring is enabled and the paragraph
/// begins on a verso (even) page, the start (`left`) and end (`right`) indents
/// — both their point-based and character-based variants — are swapped so the
/// binding-side indentation stays on the inside edge of facing pages. On recto
/// (odd) pages, or when mirroring is off, the indent is returned unchanged.
pub fn mirror_indent_for_page(
    indent: &Indent,
    bidi: bool,
    mirror_indents: bool,
    page_is_even: bool,
) -> Indent {
    let mut result = Indent {
        left: indent.physical_left(bidi),
        right: indent.physical_right(bidi),
        left_chars: indent.physical_left_chars(bidi),
        right_chars: indent.physical_right_chars(bidi),
        first_line: indent.first_line,
        hanging: indent.hanging,
        first_line_chars: indent.first_line_chars,
        hanging_chars: indent.hanging_chars,
        ..Indent::default()
    };
    if mirror_indents && page_is_even {
        std::mem::swap(&mut result.left, &mut result.right);
        std::mem::swap(&mut result.left_chars, &mut result.right_chars);
    }
    result
}

/// Resolved physical left indent in points. The character-unit variant
/// supersedes the twips variant (ECMA-376 §17.3.1.12); an explicit `0` chars
/// value re-enables the twips value, which is how Japanese Word neutralizes an
/// inherited character indent when the user sets a point-based one.
pub fn resolve_indent_left_pt(indent: &Indent, bidi: bool, char_unit_pt: f64) -> Option<f64> {
    match indent.physical_left_chars(bidi) {
        Some(chars) if chars != 0 => Some(char_unit_indent_pt(chars, char_unit_pt)),
        _ => indent.physical_left(bidi).map(f64::from),
    }
}

/// Whether the physical left indent is set in character units. Word for Mac
/// 16.114 starts the first line of such a paragraph at that indent and hangs
/// the continuation lines past it, rather than subtracting the hanging from
/// the first line as it does for a point indent (`w:left` with `w:hanging`).
pub fn left_indent_is_character_unit(indent: &Indent, bidi: bool) -> bool {
    indent
        .physical_left_chars(bidi)
        .is_some_and(|chars| chars != 0)
}

/// Resolved physical right indent in points; same precedence rules.
pub fn resolve_indent_right_pt(indent: &Indent, bidi: bool, char_unit_pt: f64) -> Option<f64> {
    match indent.physical_right_chars(bidi) {
        Some(chars) if chars != 0 => Some(char_unit_indent_pt(chars, char_unit_pt)),
        _ => indent.physical_right(bidi).map(f64::from),
    }
}

/// Resolved first-line offset in points: positive = first-line indent,
/// negative = hanging indent. When both are present `hanging` supersedes
/// `firstLine` (ECMA-376 §17.3.1.12) and each character-unit variant
/// supersedes its twips counterpart. `include_hanging: false` preserves the
/// numbered-paragraph path where the hanging offset is derived from the list
/// marker instead of `w:ind`.
pub fn resolve_indent_first_line_pt(
    indent: &Indent,
    char_unit_pt: f64,
    include_hanging: bool,
) -> Option<f64> {
    if include_hanging {
        if let Some(chars) = indent.hanging_chars {
            if chars != 0 {
                return Some(-char_unit_indent_pt(chars, char_unit_pt));
            }
        }
        if let Some(hanging) = indent.hanging {
            return Some(-f64::from(hanging));
        }
    }
    if let Some(chars) = indent.first_line_chars {
        if chars != 0 {
            return Some(char_unit_indent_pt(chars, char_unit_pt));
        }
    }
    indent.first_line.map(f64::from)
}

/// Where a numbered paragraph's marker and lines start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberedParagraphOrigin {
    /// The paragraph's left edge for continuation lines.
    pub x: f64,
    /// Where the number is drawn.
    pub number_x: f64,
    /// Where the first line would start without its marker: the base the
    /// marker's extent is measured from (see [`numbering_first_line_offset`]).
    pub body_start: f64,
}

/// Resolves a numbered paragraph's geometry the way Word for Mac 16.114 does
/// (indent probe), given the numbering level's `w:ind` left and hanging in
/// points: `x` is the paragraph left after
/// [`apply_paragraph_indent_with_char_unit`] with `has_numbering`, and
/// `first_line_indent_px` the first-line offset it set.
///
/// - Without a paragraph left indent (points or characters) the level's
///   `w:left` applies.
/// - The paragraph's own hanging (characters before points) wins; a
///   paragraph first-line indent cancels the level's hanging; otherwise the
///   level's hanging applies.
/// - Under a character-unit left indent the first line stays at that indent
///   and the continuation lines hang past it
///   ([`left_indent_is_character_unit`]); under a point indent the hanging is
///   taken from the first line.
pub fn numbered_paragraph_origin(
    x: f64,
    first_line_indent_px: f64,
    paragraph: Option<&Indent>,
    level_left: Option<f32>,
    level_hanging: Option<f32>,
    bidi: bool,
    char_unit_pt: f64,
) -> NumberedParagraphOrigin {
    let mut x = x;
    let paragraph_left =
        paragraph.and_then(|indent| resolve_indent_left_pt(indent, bidi, char_unit_pt));
    if paragraph_left.is_none() {
        if let Some(left) = level_left {
            x += points_to_px(left);
        }
    }
    let paragraph_hanging =
        paragraph.and_then(
            |indent| match indent.hanging_chars.filter(|chars| *chars != 0) {
                Some(chars) => Some(char_unit_indent_pt(chars, char_unit_pt)),
                None => indent.hanging.map(f64::from),
            },
        );
    let paragraph_first_line = paragraph
        .is_some_and(|indent| indent.first_line.is_some() || indent.first_line_chars.is_some());
    let hanging_pt = paragraph_hanging.or_else(|| {
        (!paragraph_first_line)
            .then(|| level_hanging.map(f64::from))
            .flatten()
    });
    let hanging_px = hanging_pt.map_or(0.0, |pt| points_to_px(pt as f32));
    if hanging_px != 0.0
        && paragraph.is_some_and(|indent| left_indent_is_character_unit(indent, bidi))
    {
        x += hanging_px;
    }
    let body_start = x + first_line_indent_px;
    NumberedParagraphOrigin {
        x,
        number_x: body_start - hanging_px,
        body_start,
    }
}

/// Numbered first-line text follows a space/nothing marker directly. A tab
/// suffix keeps the authored body start unless the marker extends past it.
/// The caller supplies the shared marker advance; no font measurement occurs.
pub fn numbering_first_line_offset(
    body_start: f64,
    marker_end: f64,
    previous_offset: f64,
    suffix: Option<&str>,
) -> f64 {
    let offset = marker_end - body_start;
    if matches!(suffix, Some("space" | "nothing")) {
        offset
    } else if offset > 0.0 {
        previous_offset.max(offset)
    } else {
        previous_offset
    }
}

/// Applies paragraph indent settings to the working layout state using the
/// default character unit. Prefer the `_with_char_unit` variant when the
/// paragraph font size is known.
pub fn apply_paragraph_indent(
    x: &mut f64,
    paragraph_style: &mut ParagraphStyle,
    indent: &Indent,
    has_numbering: bool,
) {
    apply_paragraph_indent_with_char_unit(
        x,
        paragraph_style,
        indent,
        has_numbering,
        DEFAULT_INDENT_CHAR_UNIT_PT,
    );
}

/// Applies paragraph indent settings to the working layout state.
/// `char_unit_pt` is the width of one character-unit ("1字") used to convert
/// `w:leftChars`/`w:firstLineChars`/… — Word derives it from the paragraph
/// font size (plus the grid cell spacing under an active character grid).
pub fn apply_paragraph_indent_with_char_unit(
    x: &mut f64,
    paragraph_style: &mut ParagraphStyle,
    indent: &Indent,
    has_numbering: bool,
    char_unit_pt: f64,
) {
    let bidi = paragraph_style.bidi;
    let mut applied_left_indent = 0.0;
    if let Some(left_pt) = resolve_indent_left_pt(indent, bidi, char_unit_pt) {
        let left_px = points_to_px(left_pt as f32);
        *x += left_px;
        applied_left_indent += left_px;
    }
    if let Some(first_line_pt) = resolve_indent_first_line_pt(indent, char_unit_pt, !has_numbering)
    {
        paragraph_style.first_line_indent = points_to_px(first_line_pt as f32);
        if first_line_pt < 0.0 && left_indent_is_character_unit(indent, bidi) {
            // The first line stays at the character indent; the rest hang.
            *x -= paragraph_style.first_line_indent;
            applied_left_indent -= paragraph_style.first_line_indent;
        }
    }
    paragraph_style.tab_origin_offset -= applied_left_indent;
    if let Some(right_pt) = resolve_indent_right_pt(indent, bidi, char_unit_pt) {
        paragraph_style.right_indent = points_to_px(right_pt as f32);
    }
}

#[derive(Clone, Copy)]
enum FitTextAdjustment {
    Scale(f64),
    Mac {
        scale: f64,
        gap_px: f64,
        last_visible_end: usize,
    },
}

struct FittedVisualGaps {
    range: Range<usize>,
    gap_px: f64,
    line_epoch: usize,
    remaining: usize,
}

fn fitted_gap_group(groups: &[FittedVisualGaps], offset: usize) -> Option<usize> {
    let index = groups.partition_point(|group| group.range.end <= offset);
    groups
        .get(index)
        .filter(|group| group.range.start <= offset)
        .map(|_| index)
}

// Preserve each line's gap count when bidi changes a fitted group's visual
// order. Range lookup avoids scanning every group for every cluster.
fn reorder_fitted_visual_gaps(
    line: &mut WrappedLine,
    groups: &mut [FittedVisualGaps],
    epoch: usize,
) {
    for cluster in &line.clusters {
        if !cluster
            .layout
            .as_ref()
            .is_some_and(|l| l.adjusted_glyph_width > f64::EPSILON)
        {
            continue;
        }
        if let Some(index) = fitted_gap_group(groups, cluster.start) {
            let group = &mut groups[index];
            if group.line_epoch != epoch {
                group.line_epoch = epoch;
                group.remaining = 0;
            }
            group.remaining += usize::from(cluster.end < group.range.end);
        }
    }
    for cluster in &mut line.clusters {
        if !cluster
            .layout
            .as_ref()
            .is_some_and(|l| l.adjusted_glyph_width > f64::EPSILON)
        {
            continue;
        }
        if let Some(index) = fitted_gap_group(groups, cluster.start) {
            let group = &mut groups[index];
            let had_gap = cluster.end < group.range.end;
            let gets_gap = group.remaining > 0;
            group.remaining = group.remaining.saturating_sub(1);
            let delta = (i32::from(gets_gap) - i32::from(had_gap)) as f64 * group.gap_px;
            cluster.width += delta;
            cluster.layout.as_mut().unwrap().glyph_box_width += delta;
        }
    }
}

fn fit_text_group_adjustment<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    runs: &[RunStyle],
    start: usize,
    paragraph_style: &ParagraphStyle,
) -> Result<(usize, Option<FitTextAdjustment>), C::Error> {
    let mac = paragraph_style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac;
    let eligible = |run: &RunStyle| {
        run.inline_object.is_none()
            && text.get(run.start..run.end).is_some_and(|value| {
                !value.is_empty()
                    && !contains_fit_text_control(value)
                    && (!mac || !value.contains('\r'))
            })
            && run
                .properties
                .as_ref()
                .and_then(|properties| properties.fit_text.as_ref())
                .is_some_and(|fit| fit.val > 0)
    };
    let first = &runs[start];
    if !eligible(first) {
        return Ok((start + 1, None));
    }
    let fit = first
        .properties
        .as_ref()
        .and_then(|properties| properties.fit_text.as_ref())
        .expect("eligible fitText run");
    let mut end = start + 1;
    // ECMA-376 §17.3.2.14 links contiguous runs only when they share an
    // explicit id and target width. Formatting boundaries do not restart the
    // fitted group.
    if fit.id.is_some() {
        while let Some(next) = runs.get(end) {
            if runs[end - 1].end != next.start
                || !eligible(next)
                || next
                    .properties
                    .as_ref()
                    .and_then(|properties| properties.fit_text.as_ref())
                    != Some(fit)
            {
                break;
            }
            end += 1;
        }
    }

    let mut cache = FxHashMap::default();
    let mut measured_width = 0.0;
    let mut visible_count = 0_usize;
    let mut last_visible_end = 0;
    let mut state = TextAdvanceState::default();
    let group_end = runs[end - 1].end;
    for run in &runs[start..end] {
        let mut style = ctx.resolve_run_style(run.properties.as_deref())?;
        if mac && style.letter_spacing_px != 0.0 {
            Rc::make_mut(&mut style).letter_spacing_px = 0.0;
        }
        let next = text[run.end..group_end].graphemes(true).next();
        state = walk_text_advance_steps(
            ctx,
            &text[run.start..run.end],
            &style,
            Some(paragraph_style),
            &state,
            next,
            &mut cache,
            |step| {
                measured_width += step.advance.total_advance;
                if step.advance.adjusted_glyph_width > f64::EPSILON {
                    visible_count += 1;
                    last_visible_end = run.start + step.rel_offset + step.grapheme.len();
                }
                Ok(())
            },
        )?;
    }

    let target_width = points_to_px(fit.val as f32 / 20.0);
    let ratio = (measured_width.is_finite()
        && measured_width > f64::EPSILON
        && target_width.is_finite()
        && target_width > f64::EPSILON)
        .then(|| {
            let scale = target_width / measured_width;
            if mac {
                // Original Mac fit groups expand their inter-character gaps,
                // retaining natural glyph sizes. Condensation scales glyphs.
                let expands = target_width > measured_width && visible_count > 1;
                FitTextAdjustment::Mac {
                    scale: if expands { 1.0 } else { scale },
                    gap_px: if expands {
                        (target_width - measured_width) / (visible_count - 1) as f64
                    } else {
                        0.0
                    },
                    last_visible_end,
                }
            } else {
                FitTextAdjustment::Scale(scale)
            }
        });
    Ok((end, ratio))
}

fn fit_text_adjusted_properties(
    properties: Option<&Rc<RunProperties>>,
    adjustment: Option<FitTextAdjustment>,
) -> Option<Rc<RunProperties>> {
    let adjustment = adjustment?;
    let mut scaled = properties?.as_ref().clone();
    let ratio = match adjustment {
        FitTextAdjustment::Scale(scale) => scale,
        FitTextAdjustment::Mac { scale, .. } => {
            // The shared cluster plan supplies derived gaps; arbitrary source
            // tracking must not be added to the fixed group target.
            scaled.char_spacing = Some(0.0);
            scale
        }
    };
    let base_percent = scaled.text_scale.unwrap_or(100.0) as f64;
    scaled.text_scale = Some((base_percent * ratio).clamp(1.0, 1000.0) as f32);
    Some(Rc::new(scaled))
}

/// How a ruby base (`w:ruby`, §17.3.3.25) is spread over its guide's width,
/// and the ascent its guide needs (`crate::ruby`). Offsets are text-relative
/// starts of the first and last visible base characters.
#[derive(Clone, Copy)]
struct RubyAdjustment {
    first_visible: usize,
    last_visible: usize,
    leading_px: f64,
    gap_px: f64,
    trailing_px: f64,
    ascent_px: Option<f64>,
}

impl RubyAdjustment {
    /// Space this base character takes before and after its glyph.
    fn extra_px(&self, start: usize, visible: bool) -> (f64, f64) {
        if !visible || start < self.first_visible || start > self.last_visible {
            return (0.0, 0.0);
        }
        let leading = if start == self.first_visible {
            self.leading_px
        } else {
            0.0
        };
        let trailing = if start == self.last_visible {
            self.trailing_px
        } else {
            self.gap_px
        };
        (leading, trailing)
    }
}

/// Measures the ruby that starts at `runs[start]` (its base may span several
/// runs) and returns the index after its last base run. Word lays the ruby
/// out as one box as wide as the wider of base and guide.
fn ruby_group_adjustment<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    runs: &[RunStyle],
    start: usize,
    paragraph_style: &ParagraphStyle,
) -> Result<(usize, Option<RubyAdjustment>), C::Error> {
    let first = &runs[start];
    let Some((properties, ruby)) = first
        .properties
        .as_deref()
        .and_then(|properties| Some((properties, properties.ruby.as_ref()?)))
        .filter(|(_, ruby)| !ruby.continuation && !ruby.text.is_empty())
        .filter(|_| first.inline_object.is_none())
    else {
        return Ok((start + 1, None));
    };
    let mut end = start + 1;
    while let Some(next) = runs.get(end) {
        let continues = runs[end - 1].end == next.start
            && next.inline_object.is_none()
            && next
                .properties
                .as_ref()
                .and_then(|properties| properties.ruby.as_ref())
                .is_some_and(|next_ruby| next_ruby.continuation);
        if !continues {
            break;
        }
        end += 1;
    }

    let mut cache = FxHashMap::default();
    let mut base_width = 0.0;
    let mut visible = 0_usize;
    let mut first_visible = None;
    let mut last_visible = 0;
    let mut state = TextAdvanceState::default();
    let group_end = runs[end - 1].end;
    for run in &runs[start..end] {
        let Some(run_text) = text.get(run.start..run.end) else {
            return Ok((end, None));
        };
        let style = ctx.resolve_run_style(run.properties.as_deref())?;
        let next = text
            .get(run.end..group_end)
            .and_then(|rest| rest.graphemes(true).next());
        state = walk_text_advance_steps(
            ctx,
            run_text,
            &style,
            Some(paragraph_style),
            &state,
            next,
            &mut cache,
            |step| {
                base_width += step.advance.total_advance;
                if step.advance.adjusted_glyph_width > f64::EPSILON {
                    visible += 1;
                    let offset = run.start + step.rel_offset;
                    first_visible.get_or_insert(offset);
                    last_visible = offset;
                }
                Ok(())
            },
        )?;
    }
    let Some(first_visible) = first_visible else {
        return Ok((end, None));
    };

    let base_size = properties.font_size.unwrap_or(12.0);
    let guide_properties = crate::ruby::ruby_run_properties(Some(properties), base_size, ruby);
    let guide_style = ctx.resolve_run_style(Some(&guide_properties))?;
    let mut guide_width = 0.0;
    walk_text_advance_steps(
        ctx,
        &ruby.text,
        &guide_style,
        Some(paragraph_style),
        &TextAdvanceState::default(),
        None,
        &mut cache,
        |step| {
            guide_width += step.advance.total_advance;
            Ok(())
        },
    )?;
    let slack = guide_width - base_width;
    let (leading_px, gap_px) =
        crate::ruby::ruby_distribution(ruby.align.as_deref(), visible, slack);
    let trailing_px = if slack > 0.0 {
        slack - leading_px - gap_px * (visible - 1) as f64
    } else {
        0.0
    };

    let ascent_px = match ruby.raise_half_points {
        Some(raise) => {
            let descriptor = if crate::line_metrics::text_font_slots(&ruby.text).1 {
                &guide_style.font_descriptor_east_asia
            } else {
                &guide_style.font_descriptor_latin_ascii
            };
            let metrics = match paragraph_style
                .word_east_asian_font_line_box
                .then(|| crate::line_metrics::word_font_vertical_metrics_px(descriptor))
                .flatten()
            {
                Some(metrics) => Some(metrics),
                None => ctx.measure_font_vertical_metrics(descriptor)?,
            };
            metrics.map(|(ascent, _)| points_to_px(crate::ruby::ruby_raise_pt(raise)) + ascent)
        }
        None => None,
    };

    Ok((
        end,
        Some(RubyAdjustment {
            first_visible,
            last_visible,
            leading_px,
            gap_px,
            trailing_px,
            ascent_px,
        }),
    ))
}

/// Returns whether a run contains a control that makes `w:fitText` ineligible.
/// All three controls are single-byte ASCII, so one byte scan is equivalent to
/// three UTF-8 string searches without invoking the generic substring matcher.
#[inline]
fn contains_fit_text_control(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .any(|byte| matches!(*byte, b'\t' | b'\n' | b'\x0c'))
}

#[derive(Clone, Copy)]
struct SegmentMeta<'a> {
    properties: &'a Option<Rc<RunProperties>>,
    revision: &'a Option<RevisionMark>,
    hyperlink: &'a Option<RunHyperlink>,
}

fn push_text_segment(
    line: &mut Vec<TextSegment>,
    text: String,
    range: Range<usize>,
    meta: SegmentMeta<'_>,
    leading_width: f64,
) {
    if text.is_empty() || range.start >= range.end {
        return;
    }

    if let Some(last) = line.last_mut() {
        if last.end == range.start
            && leading_width.abs() < f64::EPSILON
            && last.leading_width.abs() < f64::EPSILON
            && last.properties == *meta.properties
            && last.revision == *meta.revision
            && last.hyperlink == *meta.hyperlink
        {
            last.text.push_str(&text);
            last.end = range.end;
            return;
        }
    }

    line.push(TextSegment {
        text,
        start: range.start,
        end: range.end,
        properties: meta.properties.clone(),
        revision: meta.revision.clone(),
        hyperlink: meta.hyperlink.clone(),
        leading_width,
    });
}

/// Converts a string from the document into the corresponding tab leader enum.
pub fn tab_leader_from_str(value: &str) -> TabLeader {
    match value.to_ascii_lowercase().as_str() {
        "dot" => TabLeader::Dot,
        "hyphen" => TabLeader::Hyphen,
        "underscore" => TabLeader::Underscore,
        "heavy" => TabLeader::Heavy,
        "middledot" => TabLeader::MiddleDot,
        _ => TabLeader::None,
    }
}

/// Performs line wrapping using resolved run styles and measurements.
/// `base_x`/`column_right_edge` define the available width, `base_offset` adjusts reported positions,
/// and `paragraph_style` supplies indent/tab/line break rules.
pub fn wrap_text_into_lines<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    base_x: f64,
    paragraph_style: &ParagraphStyle,
    base_offset: usize,
    column_right_edge: f64,
) -> Result<Vec<WrappedLine>, C::Error> {
    wrap_text_into_lines_with_terminal_candidate(
        ctx,
        text,
        run_styles,
        base_x,
        paragraph_style,
        base_offset,
        column_right_edge,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn wrap_text_into_lines_with_terminal_candidate<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    base_x: f64,
    paragraph_style: &ParagraphStyle,
    base_offset: usize,
    column_right_edge: f64,
    try_terminal_candidate: bool,
) -> Result<Vec<WrappedLine>, C::Error> {
    if try_terminal_candidate
        && mac_punctuation_compression_enabled(paragraph_style)
        && paragraph_style.float_line_segments.is_empty()
        && !paragraph_style.bidi
        && paragraph_style.tab_stops.is_empty()
        && !text.contains(['\t', '\n', jade_docx_reader::PAGE_BREAK_CHAR])
        && text.chars().any(|ch| {
            matches!(
                ch,
                '、' | '。' | '，' | '．' | '（' | '）' | '「' | '」' | '・' | '：' | '；'
            )
        })
        && run_styles.iter().all(|run| {
            run.inline_object.is_none()
                && run
                    .properties
                    .as_ref()
                    .is_none_or(|p| p.border.is_none() && p.fit_text.is_none())
        })
    {
        // Resolve the complete paragraph through this same layout path. Its
        // ParagraphEnd context may fit using the existing blank-side allocation;
        // this does not change admission of ordinary intermediate soft lines.
        let mut candidate = wrap_text_into_lines_with_terminal_candidate(
            ctx,
            text,
            run_styles,
            base_x,
            paragraph_style,
            base_offset,
            f64::INFINITY,
            false,
        )?;
        if candidate.len() == 1
            && candidate[0].break_kind == LineBreakKind::ParagraphEnd
            && !requires_bidi_resolution(&candidate[0], false)
        {
            let available = paragraph_style
                .available_width_for_line(
                    column_right_edge - paragraph_style.right_indent - base_x,
                    0,
                )
                .max(1.0);
            let natural: f64 = candidate[0]
                .clusters
                .iter()
                .map(|cluster| cluster.width)
                .sum();
            if natural - trailing_whitespace_width(&candidate[0].clusters) > available {
                let mut cache = FxHashMap::default();
                compress_justified_word_spaces_to_available_width(
                    ctx,
                    &mut candidate,
                    base_x,
                    paragraph_style,
                    column_right_edge,
                    false,
                    &mut cache,
                )?;
                let allocated: f64 = candidate[0]
                    .clusters
                    .iter()
                    .map(|cluster| cluster.width)
                    .sum();
                if allocated - trailing_whitespace_width(&candidate[0].clusters) <= available + 1e-6
                {
                    return Ok(candidate);
                }
            } else {
                return Ok(candidate);
            }
        }
    }
    let right_limit = column_right_edge - paragraph_style.right_indent;
    let base_available = right_limit - base_x;
    let mut current_available = paragraph_style
        .available_width_for_line(base_available, 0)
        .max(1.0);

    let mut lines: Vec<WrappedLine> = Vec::new();
    let mut current_line: Vec<TextSegment> = Vec::new();
    let mut current_width = 0.0;
    let mut current_line_indent = paragraph_style.line_indent(0);
    let mut current_line_leaders: Vec<TabLeaderSegment> = Vec::new();
    let mut current_line_clusters: Vec<InlineCluster> = Vec::new();
    let mut current_line_tab_stops: Vec<TabStopSegment> = Vec::new();
    let mut pending_leading_width = 0.0;
    let mut snap_to_chars_cell_width_cache: FxHashMap<SnapToCharsCacheKey, f64> =
        FxHashMap::default();
    let mut font_vertical_metrics_cache: FxHashMap<u64, Option<(f64, f64)>> = FxHashMap::default();
    let mut pair_measure_cache = PairMeasureCache::default();
    let mut advance_state_across_runs = TextAdvanceState::default();
    let mut fit_group_end = 0;
    let mut fit_group_adjustment = None;
    let mut ruby_group_end = 0;
    let mut ruby_group: Option<RubyAdjustment> = None;
    let mut fitted_visual_gaps: smallvec::SmallVec<[FittedVisualGaps; 1]> =
        smallvec::SmallVec::new();

    for (run_index, run_style) in run_styles.iter().enumerate() {
        if let Some(object) = run_style.inline_object {
            let object_width = object.width.max(1.0);
            let effective_available = (current_available - WRAP_SAFETY_PX).max(1.0);
            if current_width + object_width > effective_available && current_width > 0.0 {
                lines.push(WrappedLine {
                    segments: std::mem::take(&mut current_line),
                    break_kind: LineBreakKind::Soft,
                    break_range: None,
                    break_properties: None,
                    leaders: std::mem::take(&mut current_line_leaders),
                    tab_stops: std::mem::take(&mut current_line_tab_stops),
                    clusters: std::mem::take(&mut current_line_clusters),
                });
                current_width = 0.0;
                current_available = paragraph_style
                    .available_width_for_line(base_available, lines.len())
                    .max(1.0);
                current_line_indent = paragraph_style.line_indent(lines.len());
            }
            current_line_clusters.push(
                InlineCluster::inline_object(
                    base_offset + run_style.start,
                    base_offset + run_style.end,
                    object,
                )
                .with_object_position(
                    run_style
                        .properties
                        .as_ref()
                        .and_then(|props| props.text_position),
                ),
            );
            current_width += object_width;
            advance_state_across_runs = TextAdvanceState::default();
            continue;
        }

        let start = run_style.start.min(text.len());
        let mut end = run_style.end.min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let mut start_aligned = start;
        while start_aligned > 0 && !text.is_char_boundary(start_aligned) {
            start_aligned = start_aligned.saturating_sub(1);
        }
        let run_text = text.get(start_aligned..end).unwrap_or("");
        if run_text.is_empty() {
            continue;
        }

        if run_index >= fit_group_end {
            (fit_group_end, fit_group_adjustment) =
                fit_text_group_adjustment(ctx, text, run_styles, run_index, paragraph_style)?;
            if let Some(FitTextAdjustment::Mac {
                gap_px,
                last_visible_end,
                ..
            }) = fit_group_adjustment
            {
                if gap_px > f64::EPSILON {
                    fitted_visual_gaps.push(FittedVisualGaps {
                        range: base_offset + run_style.start..base_offset + last_visible_end,
                        gap_px,
                        line_epoch: usize::MAX,
                        remaining: 0,
                    });
                }
            }
        }
        if run_index >= ruby_group_end {
            (ruby_group_end, ruby_group) =
                ruby_group_adjustment(ctx, text, run_styles, run_index, paragraph_style)?;
        }
        let effective_properties =
            fit_text_adjusted_properties(run_style.properties.as_ref(), fit_group_adjustment);
        let run_properties = if effective_properties.is_some() {
            &effective_properties
        } else {
            &run_style.properties
        };
        let overflow_punctuation_set = word_overflow_punctuation_set(run_properties.as_deref());
        let style = ctx.resolve_run_style(run_properties.as_deref())?;
        let text_scale_x = style.text_scale_x.max(0.01);
        let mut buffer = String::new();
        let mut buffer_start_abs = base_offset + start_aligned;
        let mut buffer_is_whitespace: Option<bool> = None;
        let mut active_script = ScriptCategory::Latin;

        let mut grapheme_iter = crate::graphemes::grapheme_indices(run_text).peekable();
        while let Some((rel_offset, grapheme)) = grapheme_iter.next() {
            let grapheme_start_abs = base_offset + start_aligned + rel_offset;
            let grapheme_end_abs = grapheme_start_abs + grapheme.len();
            // Fast path: peek the next grapheme from the same run iterator.
            // Only fall back to the expensive cross-run lookup for the last grapheme in a run.
            let next_grapheme = match grapheme_iter.peek() {
                Some(&(_, next_g)) => Some(next_g),
                None => next_grapheme_at_absolute_offset(text, base_offset, grapheme_end_abs),
            };

            if grapheme == "\r" {
                continue;
            }

            if grapheme == "\n" {
                if !buffer.is_empty() {
                    let segment_text = std::mem::take(&mut buffer);
                    push_text_segment(
                        &mut current_line,
                        segment_text,
                        buffer_start_abs..grapheme_start_abs,
                        SegmentMeta {
                            properties: run_properties,
                            revision: &run_style.revision,
                            hyperlink: &run_style.hyperlink,
                        },
                        pending_leading_width,
                    );
                }
                buffer_is_whitespace = None;
                pending_leading_width = 0.0;

                lines.push(WrappedLine {
                    segments: std::mem::take(&mut current_line),
                    break_kind: LineBreakKind::Hard,
                    break_range: Some((grapheme_start_abs, grapheme_end_abs)),
                    break_properties: run_properties.clone(),
                    leaders: std::mem::take(&mut current_line_leaders),
                    tab_stops: std::mem::take(&mut current_line_tab_stops),
                    clusters: std::mem::take(&mut current_line_clusters),
                });
                current_width = 0.0;
                current_available = paragraph_style
                    .available_width_for_line(base_available, lines.len())
                    .max(1.0);
                current_line_indent = paragraph_style.line_indent(lines.len());
                advance_state_across_runs = TextAdvanceState::default();
                buffer_start_abs = grapheme_end_abs;
                continue;
            }

            if grapheme == PAGE_BREAK_STR {
                if !buffer.is_empty() {
                    let segment_text = std::mem::take(&mut buffer);
                    push_text_segment(
                        &mut current_line,
                        segment_text,
                        buffer_start_abs..grapheme_start_abs,
                        SegmentMeta {
                            properties: run_properties,
                            revision: &run_style.revision,
                            hyperlink: &run_style.hyperlink,
                        },
                        pending_leading_width,
                    );
                }
                buffer_is_whitespace = None;
                pending_leading_width = 0.0;

                lines.push(WrappedLine {
                    segments: std::mem::take(&mut current_line),
                    break_kind: LineBreakKind::Page,
                    break_range: Some((grapheme_start_abs, grapheme_end_abs)),
                    break_properties: run_style.properties.clone(),
                    leaders: std::mem::take(&mut current_line_leaders),
                    tab_stops: std::mem::take(&mut current_line_tab_stops),
                    clusters: std::mem::take(&mut current_line_clusters),
                });
                current_width = 0.0;
                current_available = paragraph_style
                    .available_width_for_line(base_available, lines.len())
                    .max(1.0);
                current_line_indent = paragraph_style.line_indent(lines.len());
                advance_state_across_runs = TextAdvanceState::default();
                buffer_start_abs = grapheme_end_abs;
                continue;
            }

            if grapheme == "\t" {
                if !buffer.is_empty() {
                    let segment_text = std::mem::take(&mut buffer);
                    push_text_segment(
                        &mut current_line,
                        segment_text,
                        buffer_start_abs..grapheme_start_abs,
                        SegmentMeta {
                            properties: run_properties,
                            revision: &run_style.revision,
                            hyperlink: &run_style.hyperlink,
                        },
                        pending_leading_width,
                    );
                    pending_leading_width = 0.0;
                }
                buffer_is_whitespace = None;

                let segment_width = measure_tab_segment(
                    ctx,
                    text,
                    run_styles,
                    run_index,
                    grapheme_end_abs,
                    base_offset,
                    paragraph_style,
                    &mut snap_to_chars_cell_width_cache,
                )?;

                let mut line_origin =
                    paragraph_style.line_base_x(base_x, lines.len()) + current_line_indent;
                let tab_stop_cursor_width = balanced_tab_stop_cursor_width(
                    current_width,
                    &current_line_clusters,
                    paragraph_style,
                );
                let cursor_x = line_origin + tab_stop_cursor_width;
                let (target_x, alignment, mut leader_kind) =
                    find_next_tab_stop(paragraph_style, base_x, cursor_x);

                let mut previous_width = current_width;
                let mut desired_width = tab_segment_start_width(
                    target_x,
                    line_origin,
                    current_width,
                    segment_width,
                    alignment,
                    paragraph_style.default_tab_width,
                );

                // Word keeps the text following a tab together when the
                // complete tabbed segment does not fit on the current line but
                // does fit at a declared stop on the continuation line. Moving
                // only the overflowing glyphs leaves the tab on the previous
                // line and incorrectly starts the remainder at the left edge.
                let effective_available = (current_available - WRAP_SAFETY_PX).max(1.0);
                if current_width > 0.0
                    && desired_width + segment_width > effective_available + WRAP_FIT_TOLERANCE_PX
                {
                    let next_line_index = lines.len() + 1;
                    let next_available = paragraph_style
                        .available_width_for_line(base_available, next_line_index)
                        .max(1.0);
                    let next_line_indent = paragraph_style.line_indent(next_line_index);
                    let next_line_origin =
                        paragraph_style.line_base_x(base_x, next_line_index) + next_line_indent;
                    let (next_target_x, next_alignment, next_leader_kind) =
                        find_next_tab_stop(paragraph_style, base_x, next_line_origin);
                    let next_desired_width = tab_segment_start_width(
                        next_target_x,
                        next_line_origin,
                        0.0,
                        segment_width,
                        next_alignment,
                        paragraph_style.default_tab_width,
                    );
                    let next_effective_available = (next_available - WRAP_SAFETY_PX).max(1.0);

                    if next_desired_width + segment_width
                        <= next_effective_available + WRAP_FIT_TOLERANCE_PX
                    {
                        lines.push(WrappedLine {
                            segments: std::mem::take(&mut current_line),
                            break_kind: LineBreakKind::Soft,
                            break_range: None,
                            break_properties: None,
                            leaders: std::mem::take(&mut current_line_leaders),
                            tab_stops: std::mem::take(&mut current_line_tab_stops),
                            clusters: std::mem::take(&mut current_line_clusters),
                        });
                        current_width = 0.0;
                        previous_width = 0.0;
                        current_available = next_available;
                        current_line_indent = next_line_indent;
                        line_origin = next_line_origin;
                        desired_width = next_desired_width;
                        leader_kind = next_leader_kind;
                    }
                }

                let gap_start = line_origin + current_width;
                let gap_end = line_origin + desired_width;
                let gap_width = (desired_width - previous_width).max(0.0);
                if leader_kind != TabLeader::None && gap_end > gap_start + 0.5 {
                    let mac_dot = leader_kind == TabLeader::Dot
                        && crate::line_metrics::word_layout_platform()
                            == crate::line_metrics::WordLayoutPlatform::Mac;
                    let requested_advance = mac_dot
                        .then(|| {
                            crate::line_metrics::word_ascii_metrics(
                                &style.font_descriptor_latin_ascii,
                            )
                            .and_then(|metrics| metrics.advance_px(b'.'))
                        })
                        .flatten();
                    let glyph_advance = match requested_advance {
                        Some(advance) => advance,
                        None => ctx.measure_text_with_font_key(
                            leader_kind.character(),
                            &style.font_descriptor_latin_ascii,
                            style.font_descriptor_latin_ascii_hash,
                            style.font_descriptor_latin_ascii_id,
                        )?,
                    };
                    current_line_leaders.push(TabLeaderSegment {
                        start_x: gap_start,
                        end_x: gap_end,
                        leader: leader_kind,
                        properties: run_style.properties.clone(),
                        glyph_advance,
                        glyph_positions: tab_leader_positions_with_grid(
                            gap_start,
                            gap_end,
                            glyph_advance,
                            mac_dot,
                        ),
                    });
                }

                current_line_tab_stops.push(TabStopSegment {
                    start_offset: grapheme_start_abs,
                    end_offset: grapheme_end_abs,
                    start_x: gap_start,
                    end_x: gap_start + gap_width,
                });

                current_line_clusters.push(InlineCluster::tab(
                    grapheme_start_abs,
                    grapheme_end_abs,
                    gap_width,
                ));
                current_width += gap_width;
                pending_leading_width += gap_width;
                buffer_start_abs = grapheme_end_abs;
                advance_state_across_runs = TextAdvanceState::default();
                continue;
            }

            let script = classify_script(grapheme);
            if script != active_script {
                active_script = script;
            }
            let snap_cell_width = resolve_snap_to_chars_cell_width(
                ctx,
                paragraph_style,
                &style.font_descriptor_east_asia,
                style.font_descriptor_east_asia_hash,
                text_scale_x,
                &mut snap_to_chars_cell_width_cache,
            )?;
            let glyph_total_width;
            let is_whitespace = grapheme.trim().is_empty();
            let auto_space;
            let cluster_layout = loop {
                let (font_slot, font_desc, font_desc_hash, font_desc_id) =
                    style.font_key_for_grapheme(grapheme, active_script);
                let computed = compute_text_advance_step_with_descriptor(
                    ctx,
                    grapheme,
                    &style,
                    Some(paragraph_style),
                    &advance_state_across_runs,
                    next_grapheme,
                    current_width <= f64::EPSILON,
                    snap_cell_width,
                    &mut pair_measure_cache,
                    active_script,
                    font_desc,
                    font_desc_hash,
                    font_desc_id,
                    Some(grapheme_start_abs),
                )?;
                let marker_gap_ratio = marker_spacing_gap_ratio(
                    paragraph_style,
                    &current_line_clusters,
                    grapheme,
                    script,
                );
                let marker_gap = if marker_gap_ratio > 0.0 {
                    let fullwidth_space_width = ctx
                        .measure_text("　", font_desc)
                        .unwrap_or(computed.measured_glyph_width)
                        * text_scale_x;
                    marker_gap_ratio
                        * fullwidth_space_width.max(adjusted_glyph_advance_width(
                            script,
                            computed.measured_glyph_width,
                            text_scale_x,
                        ))
                } else {
                    0.0
                };
                let fitted_gap = match fit_group_adjustment {
                    Some(FitTextAdjustment::Mac {
                        gap_px,
                        last_visible_end,
                        ..
                    }) if computed.advance.adjusted_glyph_width > f64::EPSILON
                        && grapheme_end_abs < base_offset + last_visible_end =>
                    {
                        gap_px
                    }
                    _ => 0.0,
                };
                let (ruby_leading, ruby_trailing) = ruby_group.map_or((0.0, 0.0), |ruby| {
                    ruby.extra_px(
                        grapheme_start_abs - base_offset,
                        computed.advance.adjusted_glyph_width > f64::EPSILON,
                    )
                });
                let candidate_width = (marker_gap
                    + computed.advance.total_advance
                    + fitted_gap
                    + ruby_leading
                    + ruby_trailing)
                    .max(0.0);
                let trailing_overhang = if computed.monospace_cell {
                    // Fixed-pitch glyphs stay inside their cell.
                    0.0
                } else {
                    ctx.measure_trailing_overhang(grapheme, font_desc)? * text_scale_x
                };
                let effective_available = (current_available - WRAP_SAFETY_PX).max(1.0);
                let zero_width_oracle_glyph =
                    computed.measured_width_is_oracle && candidate_width <= f64::EPSILON;

                let candidate_overflow =
                    current_width + candidate_width + trailing_overhang - effective_available;
                let fits_with_justified_space_shrink = candidate_overflow > 0.0
                    && is_justified_alignment(&paragraph_style.alignment)
                    && justified_word_space_shrink_capacity(
                        &current_line_clusters,
                        paragraph_style,
                    ) + f64::EPSILON
                        >= candidate_overflow;
                let candidate_auto_space_leading =
                    computed.advance.implicit_latin_gap + computed.advance.implicit_numeric_gap;
                let fits_with_mac_punctuation_squeeze = candidate_overflow > WRAP_FIT_TOLERANCE_PX
                    && !fits_with_justified_space_shrink
                    && mac_squeeze_enabled(paragraph_style)
                    && mac_squeeze_admits(
                        &current_line_clusters,
                        (
                            grapheme,
                            script,
                            computed.font_descriptor_hash,
                            computed.advance.adjusted_glyph_width,
                        ),
                        candidate_width,
                        candidate_auto_space_leading,
                        candidate_overflow,
                        effective_available - current_width,
                        mac_punctuation_compression_enabled(paragraph_style),
                    );
                // A character that may not start a line stays at this line's
                // end when it can hang there (`w:overflowPunct`) or, in a
                // justified compressPunctuation paragraph on Word for Mac,
                // when the line's punctuation can absorb it. Deciding it here
                // rather than in `enforce_line_start_rules` lets the next
                // line start after it and fill its whole width. The Windows
                // layout keeps the repair it is calibrated with (trust
                // pagination).
                let keeps_line_start_forbidden_character = candidate_overflow
                    > WRAP_FIT_TOLERANCE_PX
                    && paragraph_style.word_platform
                        == crate::line_metrics::WordLayoutPlatform::Mac
                    && !is_whitespace
                    && !fits_with_justified_space_shrink
                    && !fits_with_mac_punctuation_squeeze
                    && paragraph_style.forbids_line_start(grapheme)
                    // The character before may itself be one that may not
                    // start a line: Word keeps `…１回）、` with the comma
                    // hanging after the paren (scroll-offset-repro page 11).
                    && current_line_clusters
                        .last()
                        .is_some_and(|last| !last.is_whitespace)
                    && {
                        let paragraph_end = grapheme_end_abs >= base_offset + text.len();
                        let hangs =
                            overflow_punctuation_set.is_some_and(|set| {
                                single_char(grapheme)
                                    .is_some_and(|character| char_set_contains(set, character))
                            }) && paragraph_style.allows_hanging_punctuation_at(if paragraph_end {
                                LineBreakKind::ParagraphEnd
                            } else {
                                LineBreakKind::Soft
                            });
                        let squeeze = if mac_punctuation_compression_enabled(paragraph_style) {
                            mac_squeeze_budget_with(
                                &current_line_clusters,
                                (
                                    grapheme,
                                    script,
                                    computed.font_descriptor_hash,
                                    computed.advance.adjusted_glyph_width,
                                ),
                                candidate_width,
                                candidate_auto_space_leading,
                                true,
                            )
                            .shrink()
                        } else {
                            0.0
                        };
                        let hang = if hangs { candidate_width } else { 0.0 };
                        (hangs || squeeze > 0.0)
                            && candidate_overflow <= hang + squeeze + WRAP_FIT_TOLERANCE_PX
                    };
                if candidate_overflow > WRAP_FIT_TOLERANCE_PX
                    && current_width > 0.0
                    && !is_whitespace
                    && !zero_width_oracle_glyph
                    && !fits_with_justified_space_shrink
                    && !fits_with_mac_punctuation_squeeze
                    && !keeps_line_start_forbidden_character
                {
                    if !buffer.is_empty() {
                        let segment_text = std::mem::take(&mut buffer);
                        push_text_segment(
                            &mut current_line,
                            segment_text,
                            buffer_start_abs..grapheme_start_abs,
                            SegmentMeta {
                                properties: run_properties,
                                revision: &run_style.revision,
                                hyperlink: &run_style.hyperlink,
                            },
                            pending_leading_width,
                        );
                    }
                    buffer_is_whitespace = None;
                    pending_leading_width = 0.0;
                    lines.push(WrappedLine {
                        segments: std::mem::take(&mut current_line),
                        break_kind: LineBreakKind::Soft,
                        break_range: None,
                        break_properties: None,
                        leaders: std::mem::take(&mut current_line_leaders),
                        tab_stops: std::mem::take(&mut current_line_tab_stops),
                        clusters: std::mem::take(&mut current_line_clusters),
                    });
                    current_width = 0.0;
                    current_available = paragraph_style
                        .available_width_for_line(base_available, lines.len())
                        .max(1.0);
                    current_line_indent = paragraph_style.line_indent(lines.len());
                    advance_state_across_runs = TextAdvanceState::default();
                    buffer_start_abs = grapheme_start_abs;
                    continue;
                }

                if marker_gap > f64::EPSILON {
                    if !buffer.is_empty() {
                        let segment_text = std::mem::take(&mut buffer);
                        push_text_segment(
                            &mut current_line,
                            segment_text,
                            buffer_start_abs..grapheme_start_abs,
                            SegmentMeta {
                                properties: run_properties,
                                revision: &run_style.revision,
                                hyperlink: &run_style.hyperlink,
                            },
                            pending_leading_width,
                        );
                    }
                    buffer_is_whitespace = None;
                    pending_leading_width = marker_gap;
                    buffer_start_abs = grapheme_start_abs;
                }

                glyph_total_width = candidate_width;
                update_text_advance_state(
                    &mut advance_state_across_runs,
                    grapheme,
                    computed.measured_glyph_width,
                    computed.advance.adjusted_glyph_width,
                    computed.font_descriptor_hash,
                    computed.advance.trailing_auto_space,
                );
                let font_vertical_metrics =
                    if let Some(cached) = font_vertical_metrics_cache.get(&font_desc_hash) {
                        *cached
                    } else {
                        let word_line_box = paragraph_style
                            .word_east_asian_font_line_box
                            .then(|| crate::line_metrics::word_font_vertical_metrics_px(font_desc))
                            .flatten();
                        let measured = match word_line_box {
                            Some(metrics) => Some(metrics),
                            None => ctx.measure_font_vertical_metrics(font_desc)?,
                        };
                        font_vertical_metrics_cache.insert(font_desc_hash, measured);
                        measured
                    };
                // A character border pushes the line out by its width and
                // space (`line_metrics::word_run_border_extent_pt`).
                let border_extent_px = f64::from(crate::line_metrics::word_run_border_extent_pt(
                    run_style.properties.as_deref(),
                )) * 96.0
                    / 72.0;
                auto_space = (
                    computed.advance.implicit_latin_gap + computed.advance.implicit_numeric_gap,
                    computed.advance.implicit_latin_trailing_gap,
                );
                break InlineClusterLayout {
                    character_border_edges: 0,
                    script,
                    font_slot,
                    leading_advance: computed.advance.implicit_latin_gap
                        + computed.advance.implicit_numeric_gap
                        + computed.advance.kerning_adjustment
                        + computed.advance.snap_leading
                        + ruby_leading,
                    glyph_box_width: computed.advance.glyph_box_width + fitted_gap + ruby_trailing,
                    measured_glyph_width: computed.measured_glyph_width,
                    adjusted_glyph_width: computed.advance.adjusted_glyph_width,
                    font_descriptor_hash: computed.font_descriptor_hash,
                    font_ascent: font_vertical_metrics.map(|metrics| {
                        let ascent = metrics.0 + border_extent_px;
                        // A ruby guide raises the line above its base.
                        ruby_group
                            .and_then(|ruby| ruby.ascent_px)
                            .map_or(ascent, |guide| ascent.max(guide))
                    }),
                    font_descent: font_vertical_metrics.map(|metrics| metrics.1 + border_extent_px),
                };
            };

            if let Some(token_type) = buffer_is_whitespace {
                if token_type != is_whitespace && !buffer.is_empty() {
                    let segment_text = std::mem::take(&mut buffer);
                    push_text_segment(
                        &mut current_line,
                        segment_text,
                        buffer_start_abs..grapheme_start_abs,
                        SegmentMeta {
                            properties: run_properties,
                            revision: &run_style.revision,
                            hyperlink: &run_style.hyperlink,
                        },
                        pending_leading_width,
                    );
                    // The tab gap belongs to the first segment after the tab
                    // only. Without this reset every later whitespace
                    // transition on the line re-applied the gap, so words after
                    // "\t" rendered at successive tab stops instead of their
                    // natural spacing (Word keeps spaces after a tab ordinary).
                    pending_leading_width = 0.0;
                    buffer_start_abs = grapheme_start_abs;
                }
            }

            if buffer.is_empty() {
                buffer_is_whitespace = Some(is_whitespace);
                buffer_start_abs = grapheme_start_abs;
            }

            buffer.push_str(grapheme);
            let mut cluster = InlineCluster::grapheme_with_layout(
                grapheme_start_abs,
                grapheme_end_abs,
                glyph_total_width,
                grapheme,
                is_whitespace,
                paragraph_style.forbids_line_start(grapheme),
                paragraph_style.forbids_line_end(grapheme),
                cluster_layout,
            );
            (cluster.auto_space_leading, cluster.auto_space_trailing) = auto_space;
            cluster.can_hang_line_end = overflow_punctuation_set.is_some_and(|set| {
                single_char(grapheme).is_some_and(|character| char_set_contains(set, character))
            });
            current_line_clusters.push(cluster);
            current_width += glyph_total_width;
        }

        if !buffer.is_empty() {
            let final_end = base_offset + run_style.end;
            let segment_text = std::mem::take(&mut buffer);
            push_text_segment(
                &mut current_line,
                segment_text,
                buffer_start_abs..final_end,
                SegmentMeta {
                    properties: run_properties,
                    revision: &run_style.revision,
                    hyperlink: &run_style.hyperlink,
                },
                pending_leading_width,
            );
            pending_leading_width = 0.0;
        }
    }

    lines.push(WrappedLine {
        segments: current_line,
        break_kind: LineBreakKind::Soft,
        break_range: None,
        break_properties: None,
        leaders: current_line_leaders,
        tab_stops: current_line_tab_stops,
        clusters: current_line_clusters,
    });

    if let Some(last) = lines.last_mut() {
        last.break_kind = LineBreakKind::ParagraphEnd;
        if last.break_range.is_none() {
            let end_offset = base_offset + text.len();
            last.break_range = Some((end_offset, end_offset));
        }
    }

    prevent_mid_word_breaks(text, paragraph_style, base_available, &mut lines);
    enforce_line_start_rules(paragraph_style, &mut lines);
    // The kinsoku repair moves single characters: one that may not start a
    // line takes the digit before it along and splits the number (`2` |
    // `0％`, trust-beneficiary-sale-keep-next page 9), so mend words again.
    prevent_mid_word_breaks(text, paragraph_style, base_available, &mut lines);
    drop_line_start_auto_space(&mut lines);
    let character_borders = lines.iter().any(line_has_character_border);
    ensure_lines_fit_available_width(
        ctx,
        character_borders,
        &mut lines,
        base_x,
        paragraph_style,
        column_right_edge,
        &mut snap_to_chars_cell_width_cache,
    )?;
    // The final width repair can move a closing punctuation cluster to the
    // next line. Reapply kinsoku after that movement so a non-hanging character
    // does not leave its matching opening punctuation at the previous line end.
    enforce_line_start_rules(paragraph_style, &mut lines);
    drop_line_start_auto_space(&mut lines);
    // Border detection already checked every original segment. Ordinary text
    // cannot acquire a border while clusters move, so reuse that result and
    // avoid another full segment walk (and one more during justification).
    if character_borders {
        for line in &mut lines {
            if line_has_character_border(line) {
                apply_character_border_inline_spacing(line);
            }
        }
    }
    compress_justified_word_spaces_to_available_width(
        ctx,
        &mut lines,
        base_x,
        paragraph_style,
        column_right_edge,
        character_borders,
        &mut snap_to_chars_cell_width_cache,
    )?;

    for (epoch, line) in lines.iter_mut().enumerate() {
        let reorder_gaps =
            !fitted_visual_gaps.is_empty() && requires_bidi_resolution(line, paragraph_style.bidi);
        apply_bidi_visual_order(line, paragraph_style.bidi);
        if reorder_gaps {
            reorder_fitted_visual_gaps(line, &mut fitted_visual_gaps, epoch);
        }
    }
    if paragraph_style
        .alignment
        .trim()
        .eq_ignore_ascii_case("distribute")
    {
        for (index, line) in lines.iter_mut().enumerate() {
            line.distribute_characters_to_width(
                paragraph_style
                    .available_width_for_line(base_available, index)
                    .max(1.0),
            );
        }
    }

    Ok(lines)
}

// Blank sides follow JLREQ classes: half an em before an opening or after a
// closing/stop/comma; a centered mark has a quarter on each side. Adjacent
// punctuation already uses contextual shaping and must not be reduced twice.
fn punctuation_blank_before(text: &str) -> bool {
    matches!(
        text,
        "（" | "［" | "｛" | "「" | "『" | "【" | "〔" | "〈" | "《"
    )
}

fn punctuation_blank_after(text: &str) -> bool {
    matches!(
        text,
        "）" | "］" | "｝" | "」" | "』" | "】" | "〕" | "〉" | "》" | "、" | "，" | "。" | "．"
    )
}

fn punctuation_pair_blank_width(left: &InlineCluster, right: &InlineCluster) -> f64 {
    let (Some(a), Some(b)) = (left.layout.as_ref(), right.layout.as_ref()) else {
        return 0.0;
    };
    punctuation_pair_blank(
        (
            left.text.as_str(),
            a.script,
            a.font_descriptor_hash,
            a.adjusted_glyph_width,
        ),
        (
            right.text.as_str(),
            b.script,
            b.font_descriptor_hash,
            b.adjusted_glyph_width,
        ),
        right.width,
    )
    .0
}

/// The blank two neighbouring East Asian glyphs can give up, and how many
/// half-em blanks that is (a centered mark's quarter counts as half of one).
/// Each side is `(text, script, font hash, adjusted glyph width)`.
/// An East Asian glyph with no blank side of its own, next to which a
/// punctuation mark's blank can be narrowed: kana, kanji, the ideographic
/// space and also fullwidth digits and letters (Word narrows the comma before
/// `１０`), which [`is_auto_space_east_asian_text`] counts as fullwidth forms.
fn is_squeeze_neighbour(text: &str) -> bool {
    (text == "\u{3000}" || is_non_whitespace_east_asian_grapheme(text))
        && !punctuation_blank_before(text)
        && !punctuation_blank_after(text)
        && !matches!(text, "・" | "：" | "；")
}

fn punctuation_pair_blank(
    left: (&str, ScriptCategory, u64, f64),
    right: (&str, ScriptCategory, u64, f64),
    right_width: f64,
) -> (f64, f64) {
    let (left_text, left_script, left_font, left_glyph) = left;
    let (right_text, right_script, right_font, right_glyph) = right;
    if left_script != ScriptCategory::EastAsia
        || right_script != ScriptCategory::EastAsia
        || left_font != right_font
    {
        return (0.0, 0.0);
    }
    let (glyph, share) = if is_squeeze_neighbour(left_text) {
        if punctuation_blank_before(right_text) {
            (right_glyph, 0.5)
        } else if matches!(right_text, "・" | "：" | "；") {
            (right_glyph, 0.25)
        } else {
            (0.0, 0.0)
        }
    } else if is_squeeze_neighbour(right_text) {
        if punctuation_blank_after(left_text) {
            (left_glyph, 0.5)
        } else if matches!(left_text, "・" | "：" | "；") {
            (left_glyph, 0.25)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };
    let blank = glyph * share;
    if blank.is_finite() && blank > 0.0 {
        (blank.min(right_width.max(0.0)), share * 2.0)
    } else {
        (0.0, 0.0)
    }
}

/// How far a line may run past its room and still count as fitting. Word
/// measures in whole twips, so content that exactly fills a line fits there;
/// jade carries page sizes, margins and indents as f32 points, which leaves
/// such a line up to ~1e-4 px over. Far below a twip (0.067 px).
const WRAP_FIT_TOLERANCE_PX: f64 = 1e-3;

/// Word for Mac 16.114 squeeze probes: a justified line keeps the character
/// that overflows it by `D` while `D / shrink <= E / stretch`, where `E` is
/// the room the line would keep without it. `shrink` is what the line can
/// give up — half an em per punctuation blank in `compressPunctuation`
/// documents and half of each auto-spacing gap in any document — and
/// `stretch` what it could take — this share of an em per gap between
/// characters and [`MAC_AUTO_SPACE_STRETCH_SHARE`] of each auto-spacing gap.
/// Without auto spacing this keeps a character while the reduction per
/// half-em blank is at most 32 times the gap each character would gain.
const MAC_INTER_CHARACTER_STRETCH_EM: f64 = 1.0 / 64.0;

/// The share of an auto-spacing gap counted as its stretch (auto-space
/// probes: 2.33pt for the 2.625pt gaps of 10.5pt MS Mincho).
const MAC_AUTO_SPACE_STRETCH_SHARE: f64 = 8.0 / 9.0;

/// What a line can give up to keep one more character, and what its
/// auto-spacing gaps could stretch instead (see
/// [`MAC_INTER_CHARACTER_STRETCH_EM`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MacSqueezeBudget {
    punctuation_shrink: f64,
    auto_space_shrink: f64,
    auto_space_stretch: f64,
    /// The widest East Asian glyph seen: the em the stretch is counted in.
    east_asian_em: f64,
}

impl MacSqueezeBudget {
    fn shrink(self) -> f64 {
        self.punctuation_shrink + self.auto_space_shrink
    }

    fn add_pair(
        &mut self,
        left: &InlineCluster,
        right: (&str, ScriptCategory, u64, f64),
        right_width: f64,
        right_auto_space_leading: f64,
        punctuation: bool,
    ) {
        if punctuation {
            if let Some(left_side) = punctuation_side(left) {
                self.punctuation_shrink += punctuation_pair_blank(left_side, right, right_width).0;
            }
        }
        if right.1 == ScriptCategory::EastAsia {
            self.east_asian_em = self.east_asian_em.max(right.3);
        }
        self.add_auto_space(left.auto_space_trailing + right_auto_space_leading);
    }

    fn add_auto_space(&mut self, auto_space: f64) {
        if auto_space > 0.0 {
            self.auto_space_shrink += auto_space * 0.5;
            self.auto_space_stretch += auto_space * MAC_AUTO_SPACE_STRETCH_SHARE;
        }
    }
}

/// [`MacSqueezeBudget`] of the gaps between the characters of `clusters`;
/// punctuation blanks count only when `punctuation` (compressPunctuation).
fn mac_squeeze_budget(clusters: &[InlineCluster], punctuation: bool) -> MacSqueezeBudget {
    let mut budget = MacSqueezeBudget::default();
    if let Some(first) = clusters.first().and_then(punctuation_side) {
        if first.1 == ScriptCategory::EastAsia {
            budget.east_asian_em = first.3;
        }
    }
    for pair in clusters.windows(2) {
        if let Some(right) = punctuation_side(&pair[1]) {
            budget.add_pair(
                &pair[0],
                right,
                pair[1].width,
                pair[1].auto_space_leading,
                punctuation,
            );
        } else {
            budget.add_auto_space(pair[0].auto_space_trailing + pair[1].auto_space_leading);
        }
    }
    budget
}

/// [`mac_squeeze_budget`] once `candidate` follows `clusters`.
fn mac_squeeze_budget_with(
    clusters: &[InlineCluster],
    candidate: (&str, ScriptCategory, u64, f64),
    candidate_width: f64,
    candidate_auto_space_leading: f64,
    punctuation: bool,
) -> MacSqueezeBudget {
    let mut budget = mac_squeeze_budget(clusters, punctuation);
    if let Some(last) = clusters.last() {
        budget.add_pair(
            last,
            candidate,
            candidate_width,
            candidate_auto_space_leading,
            punctuation,
        );
    }
    budget
}

fn punctuation_side(cluster: &InlineCluster) -> Option<(&str, ScriptCategory, u64, f64)> {
    cluster.layout.as_ref().map(|layout| {
        (
            cluster.text.as_str(),
            layout.script,
            layout.font_descriptor_hash,
            layout.adjusted_glyph_width,
        )
    })
}

/// Whether squeezing `clusters` (see [`MacSqueezeBudget`]) keeps a candidate
/// that overflows by `overflow`, where `slack` is the room left without it.
#[allow(clippy::too_many_arguments)]
fn mac_squeeze_admits(
    clusters: &[InlineCluster],
    candidate: (&str, ScriptCategory, u64, f64),
    candidate_width: f64,
    candidate_auto_space_leading: f64,
    overflow: f64,
    slack: f64,
    punctuation: bool,
) -> bool {
    let trailing_whitespace = clusters
        .iter()
        .rev()
        .take_while(|cluster| cluster.is_whitespace)
        .count();
    let characters = clusters.len() - trailing_whitespace;
    // Pushed out, the candidate would leave the trailing spaces hanging and
    // the line ending at its last character.
    let slack = slack + trailing_whitespace_width(clusters);
    if characters == 0 || slack <= 0.0 {
        return false;
    }
    let budget = mac_squeeze_budget_with(
        clusters,
        candidate,
        candidate_width,
        candidate_auto_space_leading,
        punctuation,
    );
    let shrink = budget.shrink();
    if shrink <= 0.0 || overflow > shrink + WRAP_FIT_TOLERANCE_PX {
        return false;
    }
    let stretch = (characters - 1) as f64 * budget.east_asian_em * MAC_INTER_CHARACTER_STRETCH_EM
        + budget.auto_space_stretch;
    overflow * stretch <= slack * shrink + WRAP_FIT_TOLERANCE_PX
}

/// A comma or full stop, the marks Word hangs past the edge as such
/// (burasage); a closing bracket that may not start a line does not hang.
fn is_hanging_comma_or_full_stop(text: &str) -> bool {
    matches!(text, "、" | "。" | "，" | "．" | "､" | "｡" | "," | ".")
}

/// Word for Mac squeezes justified lines (auto spacing in any document,
/// punctuation in compressPunctuation ones, [`mac_punctuation_compression_enabled`]).
fn mac_squeeze_enabled(style: &ParagraphStyle) -> bool {
    style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
        && is_justified_alignment(&style.alignment)
        && style.oracle_advance_px_by_text_offset.is_none()
}

fn mac_punctuation_compression_enabled(style: &ParagraphStyle) -> bool {
    style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
        && style.east_asian_rules.character_spacing_control.as_deref()
            == Some("compressPunctuation")
        && is_justified_alignment(&style.alignment)
        && style.oracle_advance_px_by_text_offset.is_none()
}

/// Update one completed line using the width from the existing space pass.
/// No character is admitted and no break/range is moved.
fn compress_mac_punctuation_on_completed_line(
    line: &mut WrappedLine,
    style: &ParagraphStyle,
    available: f64,
    natural: f64,
) {
    if !matches!(
        line.break_kind,
        LineBreakKind::Soft | LineBreakKind::ParagraphEnd
    ) || !line.tab_stops.is_empty()
    {
        return;
    }
    let overflow = natural - trailing_whitespace_width(&line.clusters) - available;
    if !overflow.is_finite() || overflow <= WRAP_FIT_TOLERANCE_PX {
        return;
    }
    if line
        .clusters
        .iter()
        .any(|cluster| cluster.kind == InlineClusterKind::InlineObject)
    {
        return;
    }
    let punctuation = mac_punctuation_compression_enabled(style);
    let budget = mac_squeeze_budget(&line.clusters, punctuation);
    let capacity = budget.shrink();
    // A final mark that may hang (`、`, `。`, `）`…) is handled the way Word for
    // Mac 16.114 sets it (squeeze probes, scroll-offset-repro): its blank half
    // hangs while that covers the overflow, otherwise the line's punctuation
    // takes the whole overflow, then the half hangs and the punctuation takes
    // the rest, and failing that the whole mark hangs.
    // Other documents have no punctuation to narrow. A final comma or full
    // stop still hangs as before; a closing bracket cannot hang, so the auto
    // spacing alone takes the overflow (trust page 14 keeps `…含む。）` inside
    // the edge by narrowing the gaps around `10` and `A`).
    let final_mark = line.clusters.last().map(|last| last.text.as_str());
    if !punctuation && final_mark.is_some_and(is_hanging_comma_or_full_stop) {
        return;
    }
    let hang_unit = if punctuation && style.allows_hanging_punctuation_at(line.break_kind) {
        line.clusters
            .last()
            .filter(|last| last.can_hang_line_end && punctuation_blank_after(last.text.as_str()))
            .and_then(|last| last.layout.as_ref())
            .map_or(0.0, |layout| {
                (layout.adjusted_glyph_width * 0.5).min(layout.glyph_box_width)
            })
    } else {
        0.0
    };
    let (terminal_blank, remaining) = if hang_unit <= f64::EPSILON {
        (0.0, overflow)
    } else if overflow <= hang_unit + WRAP_FIT_TOLERANCE_PX {
        // The whole half hangs and the line is then justified to the edge,
        // the paragraph's last line included (scroll-offset-repro page 10:
        // `…退職とする。` puts the slack into the auto spacing around `60`).
        (hang_unit, 0.0)
    } else if overflow <= capacity + WRAP_FIT_TOLERANCE_PX {
        (0.0, overflow)
    } else if overflow <= capacity + hang_unit + WRAP_FIT_TOLERANCE_PX {
        (hang_unit, overflow - hang_unit)
    } else if punctuation {
        let whole = line
            .clusters
            .last()
            .and_then(|last| last.layout.as_ref())
            .map_or(0.0, |layout| layout.glyph_box_width);
        (whole.min(overflow), (overflow - whole).max(0.0))
    } else {
        (0.0, overflow)
    };
    if capacity + WRAP_FIT_TOLERANCE_PX < remaining {
        return;
    }
    if terminal_blank > 0.0 {
        let last = line.clusters.last_mut().unwrap();
        last.width -= terminal_blank;
        last.layout.as_mut().unwrap().glyph_box_width -= terminal_blank;
    }
    if capacity <= f64::EPSILON {
        return;
    }
    // Word narrows the punctuation first, every blank by the same share of
    // what it can give, and only then the auto spacing (math.docx keeps the
    // gaps around `8pt` while `、` and `。` narrow 4.58pt each).
    let punctuation_capacity = budget.punctuation_shrink;
    let auto_space_capacity = budget.auto_space_shrink;
    let punctuation_fraction = if punctuation_capacity > f64::EPSILON {
        (remaining / punctuation_capacity).min(1.0)
    } else {
        0.0
    };
    let left_over = (remaining - punctuation_capacity * punctuation_fraction).max(0.0);
    let auto_space_fraction = if auto_space_capacity > f64::EPSILON {
        (left_over / auto_space_capacity).min(1.0)
    } else {
        0.0
    };
    for i in 1..line.clusters.len() {
        let blank = if punctuation {
            punctuation_pair_blank_width(&line.clusters[i - 1], &line.clusters[i])
        } else {
            0.0
        };
        let auto_space =
            line.clusters[i - 1].auto_space_trailing + line.clusters[i].auto_space_leading;
        let reduction = blank * punctuation_fraction + auto_space * 0.5 * auto_space_fraction;
        if reduction > 0.0 {
            let cluster = &mut line.clusters[i];
            cluster.width -= reduction;
            if let Some(layout) = cluster.layout.as_mut() {
                layout.leading_advance -= reduction;
            }
        }
    }
}

/// Word for Mac fits a justified line whose final comma or full stop would
/// end past the edge by narrowing the tab before it by the mark's whole
/// advance; the line is then justified. A tab narrower than that closes and
/// the mark hangs the rest (probe tabshrink-comma; trust pages 22 and 34).
/// Only measured for documents without punctuation compression.
fn shrink_tab_for_final_mark(
    line: &mut WrappedLine,
    style: &ParagraphStyle,
    available: f64,
    natural: f64,
) {
    if line.break_kind != LineBreakKind::Soft
        || line.tab_stops.is_empty()
        || style.word_platform != crate::line_metrics::WordLayoutPlatform::Mac
        || mac_punctuation_compression_enabled(style)
        || !style.allows_hanging_punctuation_at(line.break_kind)
    {
        return;
    }
    let overflow = natural - trailing_whitespace_width(&line.clusters) - available;
    let Some(last) = line.clusters.last() else {
        return;
    };
    if overflow <= WRAP_FIT_TOLERANCE_PX
        || !last.can_hang_line_end
        || !is_hanging_comma_or_full_stop(last.text.as_str())
    {
        return;
    }
    let mark = last.width;
    let Some(tab_index) = line
        .clusters
        .iter()
        .rposition(|cluster| cluster.kind == InlineClusterKind::Tab)
    else {
        return;
    };
    let tab_start = line.clusters[tab_index].start;
    let shrink = line.clusters[tab_index].width.min(mark);
    if shrink <= f64::EPSILON {
        return;
    }
    line.clusters[tab_index].width -= shrink;
    if let Some(stop) = line
        .tab_stops
        .iter_mut()
        .find(|stop| stop.start_offset == tab_start)
    {
        stop.end_x -= shrink;
    }
    if let Some(segment) = line
        .segments
        .iter_mut()
        .filter(|segment| segment.start > tab_start)
        .min_by_key(|segment| segment.start)
    {
        segment.leading_width = (segment.leading_width - shrink).max(0.0);
    }
}

#[cfg(test)]
fn compress_mac_punctuation_on_completed_lines(
    lines: &mut [WrappedLine],
    style: &ParagraphStyle,
    base_available: f64,
) {
    if !mac_squeeze_enabled(style) {
        return;
    }
    for (index, line) in lines.iter_mut().enumerate() {
        let available = style
            .available_width_for_line(base_available, index)
            .max(1.0);
        let natural = line.clusters.iter().map(|cluster| cluster.width).sum();
        compress_mac_punctuation_on_completed_line(line, style, available, natural);
    }
}

fn is_shrinkable_word_space(cluster: &InlineCluster) -> bool {
    cluster.kind == InlineClusterKind::Grapheme && cluster.text.as_str() == " "
}

fn justified_word_space_shrink_capacity(
    clusters: &[InlineCluster],
    paragraph_style: &ParagraphStyle,
) -> f64 {
    let shrink_ratio = justified_word_space_shrink_ratio(paragraph_style);
    let interior_end = clusters
        .iter()
        .rposition(|cluster| !cluster.is_whitespace)
        .map_or(0, |index| index + 1);
    clusters
        .get(..interior_end)
        .unwrap_or_default()
        .iter()
        .filter(|cluster| is_shrinkable_word_space(cluster))
        .map(|cluster| cluster.width * shrink_ratio)
        .sum()
}

fn completed_space_capacity_and_width(
    clusters: &[InlineCluster],
    style: &ParagraphStyle,
) -> (f64, f64) {
    let shrink_ratio = justified_word_space_shrink_ratio(style);
    let interior_end = clusters
        .iter()
        .rposition(|cluster| !cluster.is_whitespace)
        .map_or(0, |index| index + 1);
    clusters
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(capacity, width), (index, cluster)| {
            let space = if index < interior_end && is_shrinkable_word_space(cluster) {
                cluster.width * shrink_ratio
            } else {
                0.0
            };
            (capacity + space, width + cluster.width)
        })
}

fn trailing_whitespace_width(clusters: &[InlineCluster]) -> f64 {
    clusters
        .iter()
        .rev()
        .take_while(|cluster| cluster.is_whitespace)
        .map(|cluster| cluster.width)
        .sum()
}

fn compress_justified_word_spaces_to_available_width<C: LineWrapContext>(
    ctx: &C,
    lines: &mut [WrappedLine],
    base_x: f64,
    paragraph_style: &ParagraphStyle,
    column_right_edge: f64,
    character_borders: bool,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<(), C::Error> {
    if !is_justified_alignment(&paragraph_style.alignment) {
        return Ok(());
    }

    let compress_punctuation = mac_squeeze_enabled(paragraph_style);
    let squeeze_punctuation = mac_punctuation_compression_enabled(paragraph_style);
    let base_available = column_right_edge - paragraph_style.right_indent - base_x;
    for (line_index, line) in lines.iter_mut().enumerate() {
        let available = paragraph_style
            .available_width_for_line(base_available, line_index)
            .max(1.0);
        let mut natural_cache = None;
        if line.break_kind != LineBreakKind::Soft {
            if paragraph_style.word_platform != crate::line_metrics::WordLayoutPlatform::Mac
                || line.break_kind != LineBreakKind::ParagraphEnd
                || !line.tab_stops.is_empty()
            {
                continue;
            }
            // A terminal line stays left aligned when its natural cluster
            // advances fit. Remeasure only a potentially overfull last line;
            // compression does not expand it to the content edge.
            let natural: f64 = line.clusters.iter().map(|cluster| cluster.width).sum();
            if natural - trailing_whitespace_width(&line.clusters)
                <= available + WRAP_FIT_TOLERANCE_PX
            {
                continue;
            }
            natural_cache = Some(natural);
        }
        let width = measure_wrapped_line_width(
            ctx,
            line,
            paragraph_style,
            character_borders && line_has_character_border(line),
            snap_to_chars_cell_width_cache,
        )?;
        let overflow = (width - trailing_whitespace_width(&line.clusters) - available).max(0.0);
        // Only punctuation compression revisits a line that fits as measured;
        // auto spacing narrows only on an overflowing one.
        if overflow <= f64::EPSILON && !squeeze_punctuation {
            continue;
        }
        let (capacity, natural) = if compress_punctuation {
            match natural_cache {
                Some(natural) => (
                    justified_word_space_shrink_capacity(&line.clusters, paragraph_style),
                    natural,
                ),
                None => completed_space_capacity_and_width(&line.clusters, paragraph_style),
            }
        } else {
            (
                justified_word_space_shrink_capacity(&line.clusters, paragraph_style),
                0.0,
            )
        };
        let shrink_spaces = overflow > f64::EPSILON && capacity + f64::EPSILON >= overflow;
        if shrink_spaces {
            let fraction =
                (overflow / capacity).min(1.0) * justified_word_space_shrink_ratio(paragraph_style);
            let interior_end = line
                .clusters
                .iter()
                .rposition(|cluster| !cluster.is_whitespace)
                .map_or(0, |index| index + 1);
            for cluster in line.clusters.iter_mut().take(interior_end) {
                if !is_shrinkable_word_space(cluster) {
                    continue;
                }
                let shrink = cluster.width * fraction;
                cluster.width = (cluster.width - shrink).max(0.0);
                if let Some(layout) = cluster.layout.as_mut() {
                    layout.glyph_box_width = (layout.glyph_box_width - shrink).max(0.0);
                }
            }
        }
        if compress_punctuation {
            // Recompute only when the preceding space pass changed advances;
            // otherwise reuse its exact ordered cluster-width accumulation.
            let natural = if shrink_spaces {
                line.clusters.iter().map(|cluster| cluster.width).sum()
            } else {
                natural
            };
            shrink_tab_for_final_mark(line, paragraph_style, available, natural);
            compress_mac_punctuation_on_completed_line(line, paragraph_style, available, natural);
        }
    }
    Ok(())
}

fn tab_segment_start_width(
    target_x: f64,
    line_origin: f64,
    current_width: f64,
    segment_width: f64,
    alignment: TabAlignment,
    default_tab_width: f64,
) -> f64 {
    let mut desired_width = match alignment {
        TabAlignment::Left => target_x - line_origin,
        TabAlignment::Center => target_x - line_origin - segment_width / 2.0,
        TabAlignment::Right => target_x - line_origin - segment_width,
        TabAlignment::Decimal => target_x - line_origin - segment_width / 2.0,
        TabAlignment::Bar => target_x - line_origin,
    };
    if desired_width <= current_width + 0.5 {
        desired_width = current_width + default_tab_width.max(1.0);
    }
    desired_width
}

fn balanced_tab_stop_cursor_width(
    current_width: f64,
    current_line_clusters: &[InlineCluster],
    paragraph_style: &ParagraphStyle,
) -> f64 {
    if !paragraph_style.enable_line_break_rules
        || !paragraph_style
            .east_asian_rules
            .balance_single_byte_double_byte_width
        || current_line_clusters.is_empty()
        || current_line_clusters.iter().any(|cluster| {
            cluster.kind != InlineClusterKind::Grapheme
                || cluster.inline_object.is_some()
                || !cluster.text.chars().all(|ch| ch.is_ascii_alphabetic())
        })
    {
        return current_width;
    }

    // Word's compatibility mode uses a wider SBCS cursor when it selects the
    // tab stop following a pure alphabetic line prefix. Keep ordinary glyph
    // advances unchanged: widening them globally regresses mixed CJK/Latin
    // paragraphs. Normalizing only the stop lookup also makes the decision
    // stable when Yu Mincho falls back to a narrower Linux font.
    let alphabetic_width = current_line_clusters
        .iter()
        .map(|cluster| cluster.width)
        .sum::<f64>();
    current_width + alphabetic_width * BALANCED_SBCS_EXTRA_RATIO
}

/// Finds Word's next tab stop past `cursor_x`.
///
/// The lookup observes two different origins required by OOXML:
///
/// 1. `w:ind/@w:hanging` implicitly creates a stop at `base_x`, unless the
///    document's compatibility settings disable it.
/// 2. Authored `w:tabs/w:tab` positions and the `w:defaultTabStop` grid are
///    measured from `base_x + tab_origin_offset`, the containing text margin.
///
/// Keeping those origins separate is what aligns the `(10)\t真正譲渡` heading
/// without adding paragraph-text-specific compensation. The behavior is
/// covered by
/// `trust_true_sale_default_tab_grid_starts_at_text_margin_not_paragraph_indent`.
fn find_next_tab_stop(
    paragraph_style: &ParagraphStyle,
    base_x: f64,
    cursor_x: f64,
) -> (f64, TabAlignment, TabLeader) {
    // ECMA-376 §17.3.1.38: a hanging indent implicitly creates a custom tab
    // stop at the left indent. `base_x` is that left-indent position; the
    // first line begins before it by `first_line_indent`. A declared stop
    // nearer the cursor comes first (trust-beneficiary-sale-keep-next's
    // 600-twip stop under a 630-twip indent).
    let hanging_stop = (paragraph_style.use_hanging_indent_tab_stop
        && paragraph_style.first_line_indent < -0.5
        && base_x > cursor_x + 0.5)
        .then_some((base_x, TabAlignment::Left, TabLeader::None));
    let tab_origin_x = base_x + paragraph_style.tab_origin_offset;
    for stop in &paragraph_style.tab_stops {
        let position_px = tab_origin_x + points_to_px(stop.position_pt);
        if position_px > cursor_x + 0.5 {
            return match hanging_stop {
                Some(hanging) if hanging.0 < position_px => hanging,
                _ => (position_px, stop.alignment, stop.leader),
            };
        }
    }
    if let Some(hanging) = hanging_stop {
        return hanging;
    }
    let interval = paragraph_style.default_tab_width.max(1.0);
    let offset = (cursor_x - tab_origin_x).max(0.0);
    let next_multiple = (offset / interval).floor() + 1.0;
    let position = tab_origin_x + next_multiple * interval;
    (position, TabAlignment::Left, TabLeader::None)
}

/// Measures the width of the text that follows a tab character until the next break.
#[allow(clippy::too_many_arguments)]
fn measure_tab_segment<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    run_styles: &[RunStyle],
    start_run_index: usize,
    start_abs_offset: usize,
    base_offset: usize,
    paragraph_style: &ParagraphStyle,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<f64, C::Error> {
    let mut width = 0.0;
    let mut run_index = start_run_index;
    let mut cursor_rel = start_abs_offset.saturating_sub(base_offset);
    // The actual text walk resets context after every tab. Lookahead must use
    // that same state and advance pipeline, including across source runs.
    let mut state = TextAdvanceState::default();
    let mut pair_cache = PairMeasureCache::default();
    let mut previous_border: Option<&BorderEdge> = None;

    while run_index < run_styles.len() {
        let run = &run_styles[run_index];
        if cursor_rel >= run.end {
            run_index += 1;
            continue;
        }

        let run_start = run.start;
        if cursor_rel < run_start {
            cursor_rel = run_start;
        }
        if cursor_rel >= run.end {
            run_index += 1;
            continue;
        }

        let run_start = run_start.min(text.len());
        let mut run_end = run.end.min(text.len());
        while run_end > run_start && !text.is_char_boundary(run_end) {
            run_end = run_end.saturating_sub(1);
        }
        let mut run_start_aligned = run_start;
        while run_start_aligned > 0 && !text.is_char_boundary(run_start_aligned) {
            run_start_aligned = run_start_aligned.saturating_sub(1);
        }
        let run_text = text.get(run_start_aligned..run_end).unwrap_or("");
        let initial_offset = cursor_rel - run_start_aligned;
        let style = ctx.resolve_run_style(run.properties.as_deref())?;
        let text_scale_x = style.text_scale_x.max(0.01);
        let initial_offset = initial_offset.min(run_text.len());
        let snap_cell_width = resolve_snap_to_chars_cell_width(
            ctx,
            paragraph_style,
            &style.font_descriptor_east_asia,
            style.font_descriptor_east_asia_hash,
            text_scale_x,
            snap_to_chars_cell_width_cache,
        )?;

        let border = run
            .properties
            .as_deref()
            .and_then(|p| p.border.as_ref())
            .filter(|border| border.is_visible());
        let mut first_grapheme_in_run = true;
        let mut tab_grapheme_iter = crate::graphemes::grapheme_indices(run_text).peekable();
        while let Some((rel_offset, grapheme)) = tab_grapheme_iter.next() {
            if rel_offset < initial_offset {
                continue;
            }
            if grapheme == "\r" {
                continue;
            }
            if grapheme == "\n" || grapheme == PAGE_BREAK_STR || grapheme == "\t" {
                return Ok(width + character_border_extent_px(previous_border));
            }
            if first_grapheme_in_run {
                if previous_border != border {
                    width += character_border_extent_px(previous_border)
                        + character_border_extent_px(border);
                }
                previous_border = border;
                first_grapheme_in_run = false;
            }
            let grapheme_end_abs = base_offset + run_start_aligned + rel_offset + grapheme.len();
            let next_grapheme = match tab_grapheme_iter.peek() {
                Some(&(_, next_g)) => Some(next_g),
                None => next_grapheme_at_absolute_offset(text, base_offset, grapheme_end_abs),
            };
            let script = classify_script(grapheme);
            let (font_desc, font_desc_hash, font_desc_id) =
                style.descriptor_key_for_grapheme(grapheme, script);
            let grapheme_start_abs = base_offset + run_start_aligned + rel_offset;
            let computed = compute_text_advance_step_with_descriptor(
                ctx,
                grapheme,
                &style,
                Some(paragraph_style),
                &state,
                next_grapheme,
                width <= f64::EPSILON,
                snap_cell_width,
                &mut pair_cache,
                script,
                font_desc,
                font_desc_hash,
                font_desc_id,
                Some(grapheme_start_abs),
            )?;
            width += computed.advance.total_advance;
            update_text_advance_state(
                &mut state,
                grapheme,
                computed.measured_glyph_width,
                computed.advance.adjusted_glyph_width,
                computed.font_descriptor_hash,
                computed.advance.trailing_auto_space,
            );
        }

        cursor_rel = run.end;
        run_index += 1;
    }

    Ok(width + character_border_extent_px(previous_border))
}

fn measure_wrapped_line_width<C: LineWrapContext>(
    ctx: &C,
    line: &WrappedLine,
    paragraph_style: &ParagraphStyle,
    character_borders: bool,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<f64, C::Error> {
    if character_borders {
        return Ok(line.clusters.iter().map(|cluster| cluster.width).sum());
    }
    let mut width = 0.0;
    let mut state = TextAdvanceState::default();
    for (segment_index, segment) in line.segments.iter().enumerate() {
        if segment.text.is_empty() {
            continue;
        }
        width += segment.leading_width;
        let style = ctx.resolve_run_style(segment.properties.as_deref())?;
        let next_grapheme_after_segment = line
            .segments
            .iter()
            .skip(segment_index + 1)
            .find_map(|next| next.text.graphemes(true).next());
        let segment_width = {
            let mut segment_width = 0.0;
            let end_state = walk_text_advance_steps_at_offset(
                ctx,
                &segment.text,
                &style,
                Some(paragraph_style),
                &state,
                next_grapheme_after_segment,
                Some(segment.start),
                snap_to_chars_cell_width_cache,
                |step| {
                    segment_width += step.advance.implicit_latin_gap
                        + step.advance.implicit_numeric_gap
                        + step.advance.kerning_adjustment
                        + step.advance.snap_leading
                        + step.advance.glyph_box_width;
                    Ok(())
                },
            )?;
            state = end_state;
            segment_width
        };
        width += segment_width;
    }
    Ok(width)
}

/// Remeasures completed lines and repairs only genuine wrap-boundary drift.
///
/// Incremental cluster advances are used while discovering candidate breaks,
/// but contextual shaping, auto-spacing, tab alignment, and run-boundary state
/// can make the completed line's measured extent slightly different. This
/// final pass keeps that normalization in `jade-docx-layout` rather than letting a
/// renderer invent a second wrapping rule.
///
/// Word-observed exceptions are deliberately narrow: intrinsically
/// unbreakable overfull content stays intact; one eligible punctuation mark may
/// hang on an internal soft line; and punctuation at `ParagraphEnd` remains
/// inside the paragraph text extent. The latter distinction is asserted by
/// `paragraph_terminal_punctuation_does_not_hang_past_the_text_extent` and
/// `trust_article_14_2_paragraph_terminal_period_does_not_hang`.
fn ensure_lines_fit_available_width<C: LineWrapContext>(
    ctx: &C,
    character_borders: bool,
    lines: &mut Vec<WrappedLine>,
    base_x: f64,
    paragraph_style: &ParagraphStyle,
    column_right_edge: f64,
    snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
) -> Result<(), C::Error> {
    if lines.len() < 2 && !character_borders {
        return Ok(());
    }

    // A border repair may grow an existing continuation with moved clusters.
    // Such growth is not evidence that its original content was unbreakable.
    let original_widths = character_borders.then(|| {
        lines
            .iter()
            .map(|line| line.clusters.iter().map(|c| c.width).sum::<f64>())
            .collect::<Vec<_>>()
    });
    let right_limit = column_right_edge - paragraph_style.right_indent;
    let base_available = right_limit - base_x;

    let mut line_index = 0;
    while line_index < lines.len() {
        let available = paragraph_style.available_width_for_line(base_available, line_index);
        let effective_available = (available - WRAP_SAFETY_PX).max(1.0);
        if character_borders {
            apply_character_border_inline_spacing(&mut lines[line_index]);
        }
        let mut width = measure_wrapped_line_width(
            ctx,
            &lines[line_index],
            paragraph_style,
            character_borders,
            snap_to_chars_cell_width_cache,
        )?;
        loop {
            let compressible_line = lines[line_index].break_kind == LineBreakKind::Soft
                || (paragraph_style.word_platform == crate::line_metrics::WordLayoutPlatform::Mac
                    && lines[line_index].break_kind == LineBreakKind::ParagraphEnd
                    && lines[line_index].tab_stops.is_empty());
            let fits_with_justified_space_shrink = compressible_line
                && is_justified_alignment(&paragraph_style.alignment)
                && width
                    - trailing_whitespace_width(&lines[line_index].clusters)
                    - justified_word_space_shrink_capacity(
                        &lines[line_index].clusters,
                        paragraph_style,
                    )
                    <= effective_available + WRAP_FIT_TOLERANCE_PX;
            // A line the wrap loop admitted by narrowing its punctuation or
            // auto spacing (`mac_squeeze_admits`) is compressed afterwards.
            let fits_with_mac_punctuation_squeeze =
                compressible_line && mac_squeeze_enabled(paragraph_style) && {
                    let line = &lines[line_index];
                    let hang = if paragraph_style.allows_hanging_punctuation_at(line.break_kind) {
                        line.clusters
                            .last()
                            .filter(|last| last.can_hang_line_end)
                            .map_or(0.0, |last| last.width)
                    } else {
                        0.0
                    };
                    width
                        - trailing_whitespace_width(&line.clusters)
                        - mac_squeeze_budget(
                            &line.clusters,
                            mac_punctuation_compression_enabled(paragraph_style),
                        )
                        .shrink()
                        - hang
                        <= effective_available + WRAP_FIT_TOLERANCE_PX
                };
            if width <= effective_available + WRAP_FIT_TOLERANCE_PX
                || fits_with_justified_space_shrink
                || fits_with_mac_punctuation_squeeze
            {
                break;
            }
            if lines[line_index]
                .clusters
                .last()
                .is_some_and(|cluster| cluster.width <= f64::EPSILON)
            {
                break;
            }
            // Word keeps trailing half/full-width spaces on the current line
            // when only that separator hangs past the right edge. For justified
            // Latin lines, the 2-column Word TRACE
            // (`tests/word-oracle/2col/word-trace.json`) shows that Word still
            // wraps before the preceding word when the non-space content is
            // overfull; moving one character at a time would recreate the
            // browser-only `Worl`/`d` split, so move the whole trailing word.
            if lines[line_index]
                .clusters
                .last()
                .map(|cluster| cluster.is_whitespace)
                .unwrap_or(false)
            {
                let trailing_space_width: f64 = lines[line_index]
                    .clusters
                    .iter()
                    .rev()
                    .take_while(|cluster| cluster.is_whitespace)
                    .map(|cluster| cluster.width)
                    .sum();
                if width - trailing_space_width <= effective_available + WRAP_FIT_TOLERANCE_PX {
                    break;
                }
                if is_justified_alignment(&paragraph_style.alignment) {
                    if lines[line_index].clusters.len() <= 1 {
                        break;
                    }
                    let (head, tail) = lines.split_at_mut(line_index + 1);
                    let current = &mut head[line_index];
                    let next = &mut tail[0];
                    if move_trailing_latin_word_and_spaces_to_next_line(current, next) {
                        if character_borders {
                            apply_character_border_inline_spacing(current);
                        }
                        width = measure_wrapped_line_width(
                            ctx,
                            current,
                            paragraph_style,
                            character_borders,
                            snap_to_chars_cell_width_cache,
                        )?;
                        continue;
                    }
                }
            }
            if paragraph_style.allows_hanging_punctuation_at(lines[line_index].break_kind) {
                let has_trailing_punctuation = lines[line_index]
                    .clusters
                    .last()
                    .is_some_and(|cluster| cluster.can_hang_line_end);
                // ECMA-376 §17.3.1.21 permits exactly one punctuation
                // character to extend beyond the paragraph's text extent. Do
                // not retain an arbitrarily overfull line merely because its
                // final character is punctuation. Re-measuring without that
                // cluster also excludes its contextual auto-spacing/kerning.
                if has_trailing_punctuation {
                    let mut without_trailing_punctuation = lines[line_index].clone();
                    if detach_trailing_cluster(&mut without_trailing_punctuation).is_some() {
                        if character_borders {
                            apply_character_border_inline_spacing(
                                &mut without_trailing_punctuation,
                            );
                        }
                        let width_without_trailing_punctuation = measure_wrapped_line_width(
                            ctx,
                            &without_trailing_punctuation,
                            paragraph_style,
                            character_borders,
                            snap_to_chars_cell_width_cache,
                        )?;
                        if width_without_trailing_punctuation
                            <= effective_available + WRAP_FIT_TOLERANCE_PX
                        {
                            break;
                        }
                    }
                }
            }
            if lines[line_index].clusters.len() <= 1 {
                break;
            }
            if line_index + 1 == lines.len() {
                // Repair a final line when its completed-run measurement is
                // wider than the incremental cluster advances used to wrap it.
                // Content already known to be intrinsically overfull stays
                // intact (for example an unbreakable word), except
                // paragraph-terminal overflow punctuation: Word keeps that
                // punctuation inside the text extent instead of using the
                // internal-line overhang. A continuation containing whitespace
                // is not intrinsically unbreakable: it may have been created by
                // this repair pass, so keep splitting it at its word boundary.
                let incremental_width: f64 = lines[line_index]
                    .clusters
                    .iter()
                    .map(|cluster| cluster.width)
                    .sum();
                let has_terminal_overflow_punctuation = lines[line_index].break_kind
                    == LineBreakKind::ParagraphEnd
                    && lines[line_index]
                        .clusters
                        .last()
                        .is_some_and(|cluster| cluster.can_hang_line_end);
                let has_word_break_opportunity = lines[line_index]
                    .clusters
                    .iter()
                    .any(|cluster| cluster.is_whitespace);
                let intrinsic_width = original_widths
                    .as_ref()
                    .map_or(incremental_width, |widths| {
                        widths.get(line_index).copied().unwrap_or(0.0)
                    });
                if intrinsic_width > effective_available + WRAP_FIT_TOLERANCE_PX
                    && !has_terminal_overflow_punctuation
                    && !has_word_break_opportunity
                {
                    break;
                }
                let old_break_kind = lines[line_index].break_kind;
                let old_break_range = lines[line_index].break_range.take();
                let old_break_properties = lines[line_index].break_properties.take();
                lines[line_index].break_kind = LineBreakKind::Soft;
                lines.push(WrappedLine {
                    segments: Vec::new(),
                    break_kind: old_break_kind,
                    break_range: old_break_range,
                    break_properties: old_break_properties,
                    leaders: Vec::new(),
                    tab_stops: Vec::new(),
                    clusters: Vec::new(),
                });
            }
            let (head, tail) = lines.split_at_mut(line_index + 1);
            let current = &mut head[line_index];
            let next = &mut tail[0];
            let Some((cluster, segment)) = detach_trailing_cluster(current) else {
                break;
            };
            let mut moved_width = cluster.width;
            prepend_cluster_to_line_preserving_rules(next, cluster, segment);
            moved_width += move_remaining_split_latin_word_to_next_line(current, next);
            width = if character_borders {
                apply_character_border_inline_spacing(current);
                current.clusters.iter().map(|cluster| cluster.width).sum()
            } else {
                (width - moved_width).max(0.0)
            };
        }
        line_index += 1;
    }

    Ok(())
}

/// Adjusts wrapped lines to respect East-Asian line-start/end prohibitions.
///
/// This runs after candidate wrapping and again after final-width repair. The
/// second invocation is required because remeasurement may move punctuation
/// across a boundary. A single eligible internal closing mark may be moved
/// back and hung; a paragraph-terminal-only line is not collapsed that way,
/// because Word keeps the terminal mark within the paragraph text extent.
/// Consecutive prohibited-start punctuation is kept with a preceding body
/// character rather than pretending that `overflowPunct` permits an arbitrary
/// multi-character overhang.
pub fn enforce_line_start_rules(paragraph_style: &ParagraphStyle, lines: &mut Vec<WrappedLine>) {
    if !paragraph_style.kinsoku_enabled() || lines.len() < 2 {
        return;
    }

    for idx in 1..lines.len() {
        if !matches!(lines[idx - 1].break_kind, LineBreakKind::Soft) {
            continue;
        }

        let (head, tail) = lines.split_at_mut(idx);
        let prev_line = &mut head[idx - 1];
        let next_line = &mut tail[0];

        let prev_forbids_end = prev_line
            .clusters
            .last()
            .map(|cluster| cluster.forbid_line_end)
            .unwrap_or(false);
        if prev_forbids_end && prev_line.clusters.len() > 1 {
            if let Some((cluster, segment)) = detach_trailing_cluster(prev_line) {
                prepend_cluster_to_line(next_line, cluster, segment);
                continue;
            }
        }

        let needs_start_fix = next_line
            .clusters
            .first()
            .map(|cluster| cluster.forbid_line_start)
            .unwrap_or(false);
        if !needs_start_fix {
            continue;
        }

        let terminal_punctuation_only = next_line.break_kind == LineBreakKind::ParagraphEnd
            && next_line.clusters.len() == 1
            && !paragraph_style.paragraph_end_punctuation_can_hang();
        if !terminal_punctuation_only
            && paragraph_style.allows_hanging_punctuation_at(prev_line.break_kind)
            && hang_single_forbidden_line_start_cluster(prev_line, next_line)
        {
            if next_line.clusters.is_empty() {
                // Hanging the only cluster(s) of a terminal wrap line must not
                // leave a phantom line box behind. Preserve the paragraph/hard
                // break on the line that now owns the punctuation, then let the
                // empty continuation be removed below.
                prev_line.break_kind = next_line.break_kind;
                prev_line.break_range = next_line.break_range.take();
                prev_line.break_properties = next_line.break_properties.take();
            }
            continue;
        }

        if prev_line.clusters.len() > 1 {
            // A consecutive closing-punctuation run is kept with its preceding
            // body character. Since overflowPunct can hang only one character,
            // moving just the final punctuation would leave the next line with
            // another prohibited start.
            while prev_line.clusters.len() > 1
                && prev_line
                    .clusters
                    .last()
                    .is_some_and(|cluster| cluster.forbid_line_start)
            {
                let Some((cluster, segment)) = detach_trailing_cluster(prev_line) else {
                    break;
                };
                prepend_cluster_to_line(next_line, cluster, segment);
            }
            if prev_line.clusters.len() > 1 {
                let Some((cluster, segment)) = detach_trailing_cluster(prev_line) else {
                    continue;
                };
                prepend_cluster_to_line(next_line, cluster, segment);
            }
            // Moving the body character can expose an opening punctuation
            // character at the previous line end (for example `(a)` when the
            // closing `)` cannot hang in Japanese). Move that opening
            // punctuation with the token so the repair does not create a new
            // kinsoku violation on the other side of the same boundary.
            while prev_line.clusters.len() > 1
                && prev_line
                    .clusters
                    .last()
                    .is_some_and(|cluster| cluster.forbid_line_end)
            {
                let Some((cluster, segment)) = detach_trailing_cluster(prev_line) else {
                    break;
                };
                prepend_cluster_to_line(next_line, cluster, segment);
            }
        }
    }

    lines.retain(|line| !line.clusters.is_empty() || line.break_range.is_some());
}

/// A wrapped line's first character has no East Asian neighbour before it:
/// a cluster moved there by a repair drops the auto spacing it was measured
/// with at the end of the line above. Word starts trust-beneficiary-sale-
/// keep-next's wrapped `MLPM`, `V` and `20％` lines at the edge.
fn drop_line_start_auto_space(lines: &mut [WrappedLine]) {
    for line in lines.iter_mut().skip(1) {
        let Some(first) = line.clusters.first_mut() else {
            continue;
        };
        let gap = first.auto_space_leading;
        if gap <= 0.0 {
            continue;
        }
        if let Some(layout) = first.layout.as_mut() {
            layout.leading_advance -= gap;
        }
        first.width -= gap;
        first.auto_space_leading = 0.0;
    }
}

fn prevent_mid_word_breaks(
    text: &str,
    paragraph_style: &ParagraphStyle,
    base_available: f64,
    lines: &mut [WrappedLine],
) {
    if lines.len() < 2 {
        return;
    }
    let allow_mid_word_break = paragraph_style.allows_mid_word_break();

    for idx in 1..lines.len() {
        if !matches!(lines[idx - 1].break_kind, LineBreakKind::Soft) {
            continue;
        }
        let (head, tail) = lines.split_at_mut(idx);
        let prev_line = &mut head[idx - 1];
        let next_line = &mut tail[0];
        let next_starts_latin_word =
            !next_line.clusters.is_empty() && is_latin_word_cluster(&next_line.clusters, 0);
        if !next_starts_latin_word {
            continue;
        }
        if !line_break_splits_latin_word(text, prev_line, next_line) {
            continue;
        }
        // East Asian text right before the word also ends a line there, when
        // the whole word then fits the next one: Word sets `新規` | `MLPM契約`
        // (trust-beneficiary-sale-keep-next page 8).
        if allow_mid_word_break
            && !has_preceding_latin_break_opportunity(prev_line)
            && !(follows_east_asian_text(prev_line)
                && split_latin_word_width(prev_line, next_line)
                    <= paragraph_style.available_width_for_line(base_available, idx)
                        + WRAP_FIT_TOLERANCE_PX)
        {
            continue;
        }

        // Word COM TRACE evidence: `tests/word-oracle/2col/word-trace.json`
        // wraps repeated "Hello World" text at spaces in the FE-layout
        // paragraph. Keep default mid-word wrapping for compact unspaced Latin
        // strings such as synthetic width tests, but when a previous line has a
        // legal separator before the trailing Latin word, prefer that word boundary
        // over browser-only splits such as `Worl`/`d`.
        while !prev_line.clusters.is_empty() {
            if !is_latin_word_cluster(&prev_line.clusters, prev_line.clusters.len() - 1) {
                break;
            }
            if prev_line.clusters.len() <= 1 {
                break;
            }
            if let Some((cluster, segment)) = detach_trailing_cluster(prev_line) {
                prepend_cluster_to_line(next_line, cluster, segment);
            } else {
                break;
            }
        }
        while prev_line.clusters.len() > 1
            && prev_line
                .clusters
                .last()
                .is_some_and(|cluster| cluster.forbid_line_end)
        {
            let Some((cluster, segment)) = detach_trailing_cluster(prev_line) else {
                break;
            };
            prepend_cluster_to_line_preserving_rules(next_line, cluster, segment);
        }
    }
}

fn line_break_splits_latin_word(
    text: &str,
    prev_line: &WrappedLine,
    next_line: &WrappedLine,
) -> bool {
    let Some(prev_end) = prev_line.clusters.last().map(|cluster| cluster.end) else {
        return false;
    };
    let Some(next_start) = next_line.clusters.first().map(|cluster| cluster.start) else {
        return false;
    };
    if next_start <= prev_end {
        return true;
    }
    let Some(gap) = text.get(prev_end..next_start) else {
        return false;
    };
    gap.is_empty()
}

/// Whether the cluster at `index` belongs to the Latin word around it: a
/// letter or ASCII digit (Word moves `20％` whole), or a slash right after a
/// letter (Word keeps `header/footer` whole).
fn is_latin_word_cluster(clusters: &[InlineCluster], index: usize) -> bool {
    let text = clusters[index].text.as_str();
    is_latin_word_grapheme(text)
        || is_ascii_digit_grapheme(text)
        || (text == "/" && index > 0 && is_latin_word_grapheme(&clusters[index - 1].text))
}

/// Whether the Latin word ending `line` starts right after East Asian text.
fn follows_east_asian_text(line: &WrappedLine) -> bool {
    line.clusters
        .iter()
        .enumerate()
        .rev()
        .find(|(index, _)| !is_latin_word_cluster(&line.clusters, *index))
        .is_some_and(|(_, cluster)| {
            !cluster.forbid_line_end && is_non_whitespace_east_asian_grapheme(&cluster.text)
        })
}

/// The width of a Latin word split between the end of `prev` and the start
/// of `next`.
fn split_latin_word_width(prev: &WrappedLine, next: &WrappedLine) -> f64 {
    let head: f64 = (0..prev.clusters.len())
        .rev()
        .take_while(|index| is_latin_word_cluster(&prev.clusters, *index))
        .map(|index| prev.clusters[index].width)
        .sum();
    let tail: f64 = (0..next.clusters.len())
        .take_while(|index| is_latin_word_cluster(&next.clusters, *index))
        .map(|index| next.clusters[index].width)
        .sum();
    head + tail
}

fn has_preceding_latin_break_opportunity(line: &WrappedLine) -> bool {
    let mut saw_trailing_latin = false;
    for (index, cluster) in line.clusters.iter().enumerate().rev() {
        if is_latin_word_cluster(&line.clusters, index) {
            saw_trailing_latin = true;
            continue;
        }
        // Opening punctuation carries the following word's boundary context.
        // Reuse its existing prohibition instead of introducing a character set.
        if saw_trailing_latin && cluster.forbid_line_end {
            continue;
        }
        // UAX #14 permits an internal Latin hyphen opportunity (LB21),
        // while a word-initial hyphen stays attached (LB20a). Reuse the
        // existing word-boundary preference, including custom prohibitions.
        return saw_trailing_latin
            && (cluster.is_whitespace
                || (cluster.text == "-"
                    && index > 0
                    && !line.clusters[index - 1].text.is_empty()
                    && is_latin_word_grapheme(&line.clusters[index - 1].text)));
    }
    false
}

/// Completes a Latin word move after width repair detached its final cluster.
///
/// Initial wrapping prefers a whitespace boundary, but completed-line
/// remeasurement can still move one cluster into the following line. If that
/// cluster split a Latin word which has a real preceding separator, move the
/// remainder of the word in the same repair step. Doing this before the next
/// line is measured avoids both a browser-only `Worl`/`d` split and a later
/// boundary-only pass that could overfill the receiving line.
fn move_remaining_split_latin_word_to_next_line(
    current: &mut WrappedLine,
    next: &mut WrappedLine,
) -> f64 {
    if !next
        .clusters
        .first()
        .is_some_and(|cluster| is_latin_word_grapheme(&cluster.text))
        || !has_preceding_latin_break_opportunity(current)
    {
        return 0.0;
    }

    let mut moved_width = 0.0;
    while !current.clusters.is_empty()
        && is_latin_word_cluster(&current.clusters, current.clusters.len() - 1)
    {
        let Some((cluster, segment)) = detach_trailing_cluster(current) else {
            break;
        };
        moved_width += cluster.width;
        prepend_cluster_to_line_preserving_rules(next, cluster, segment);
    }
    while current.clusters.len() > 1
        && current
            .clusters
            .last()
            .is_some_and(|cluster| cluster.forbid_line_end)
    {
        let Some((cluster, segment)) = detach_trailing_cluster(current) else {
            break;
        };
        moved_width += cluster.width;
        prepend_cluster_to_line_preserving_rules(next, cluster, segment);
    }
    moved_width
}

fn move_trailing_latin_word_and_spaces_to_next_line(
    current: &mut WrappedLine,
    next: &mut WrappedLine,
) -> bool {
    if current.clusters.len() <= 1 {
        return false;
    }
    let mut moved = false;
    while current
        .clusters
        .last()
        .map(|cluster| cluster.is_whitespace)
        .unwrap_or(false)
    {
        if current.clusters.len() <= 1 {
            break;
        }
        if let Some((cluster, segment)) = detach_trailing_cluster(current) {
            prepend_cluster_to_line(next, cluster, segment);
            moved = true;
        } else {
            return moved;
        }
    }

    let mut moved_word = false;
    while current
        .clusters
        .last()
        .map(|cluster| is_latin_word_grapheme(&cluster.text))
        .unwrap_or(false)
    {
        if current.clusters.len() <= 1 {
            break;
        }
        if let Some((cluster, segment)) = detach_trailing_cluster(current) {
            prepend_cluster_to_line(next, cluster, segment);
            moved = true;
            moved_word = true;
        } else {
            break;
        }
    }

    while moved_word
        && current.clusters.len() > 1
        && current
            .clusters
            .last()
            .is_some_and(|cluster| cluster.forbid_line_end)
    {
        let Some((cluster, segment)) = detach_trailing_cluster(current) else {
            break;
        };
        prepend_cluster_to_line_preserving_rules(next, cluster, segment);
    }
    moved && moved_word
}

/// Tries to keep one line-start-forbidden punctuation character at the
/// previous line end.
///
/// ECMA-376 §17.3.1.21 allows one punctuation character beyond the text
/// extent. If two or more forbidden characters begin the next line, hanging
/// only the first would still leave an invalid line start; the caller instead
/// moves a preceding body character to the next line with the punctuation.
fn hang_single_forbidden_line_start_cluster(
    prev_line: &mut WrappedLine,
    next_line: &mut WrappedLine,
) -> bool {
    if prev_line.clusters.is_empty() || next_line.clusters.is_empty() {
        return false;
    }
    let exactly_one_forbidden_prefix = next_line
        .clusters
        .first()
        .is_some_and(|cluster| cluster.can_hang_line_end)
        && next_line
            .clusters
            .get(1)
            .is_none_or(|cluster| !cluster.forbid_line_start)
        && prev_line
            .clusters
            .last()
            .is_some_and(|cluster| !cluster.forbid_line_start);
    if !exactly_one_forbidden_prefix {
        return false;
    }
    let Some((cluster, segment)) = detach_leading_cluster(next_line) else {
        return false;
    };
    append_cluster_to_line_end(prev_line, cluster, segment);
    true
}

/// Removes the last inline cluster and its segment from `line` so it can be prepended elsewhere.
fn detach_trailing_cluster(line: &mut WrappedLine) -> Option<(InlineCluster, TextSegment)> {
    let cluster = line.clusters.last()?.clone();
    let idx = line
        .segments
        .iter()
        .rposition(|segment| segment.end == cluster.end)?;
    let segment = line.segments.get_mut(idx)?;
    let remove_bytes = cluster.end.saturating_sub(cluster.start);
    if remove_bytes == 0 || remove_bytes > segment.text.len() {
        return None;
    }
    let split_at = segment
        .text
        .len()
        .checked_sub(remove_bytes)
        .unwrap_or_default();
    let removed_text = segment.text.split_off(split_at);
    let removed_segment = TextSegment {
        text: removed_text,
        start: cluster.start,
        end: cluster.end,
        properties: segment.properties.clone(),
        revision: segment.revision.clone(),
        hyperlink: segment.hyperlink.clone(),
        leading_width: 0.0,
    };
    segment.end = cluster.start;
    if segment.text.is_empty() {
        line.segments.remove(idx);
    }
    line.clusters.pop();
    Some((cluster, removed_segment))
}

/// Prepends a cluster+segment pair to the start of `line`, merging text when possible.
fn prepend_cluster_to_line(
    line: &mut WrappedLine,
    mut cluster: InlineCluster,
    segment: TextSegment,
) {
    cluster.forbid_line_start = false;
    cluster.forbid_line_end = false;
    prepend_cluster_to_line_preserving_rules(line, cluster, segment);
}

/// Width repair is followed by another kinsoku pass, so retain the moved
/// character's intrinsic line-start/end flags when creating its continuation.
fn prepend_cluster_to_line_preserving_rules(
    line: &mut WrappedLine,
    cluster: InlineCluster,
    mut segment: TextSegment,
) {
    line.clusters.insert(0, cluster);
    segment.leading_width = 0.0;

    if let Some(first) = line.segments.first_mut() {
        if first.properties == segment.properties
            && first.revision == segment.revision
            && first.hyperlink == segment.hyperlink
        {
            first.text = format!("{}{}", segment.text, first.text);
            first.start = segment.start;
            return;
        }
    }

    line.segments.insert(0, segment);
}

/// Removes the first inline cluster and its segment from `line`.
fn detach_leading_cluster(line: &mut WrappedLine) -> Option<(InlineCluster, TextSegment)> {
    let cluster = line.clusters.first()?.clone();
    let idx = line
        .segments
        .iter()
        .position(|segment| segment.start <= cluster.start && cluster.end <= segment.end)?;
    let segment = line.segments.get_mut(idx)?;
    let remove_start = cluster.start.saturating_sub(segment.start);
    let remove_end = cluster.end.saturating_sub(segment.start);
    if remove_start != 0 {
        return None;
    }
    if remove_end <= remove_start || remove_end > segment.text.len() {
        return None;
    }
    let removed_text = segment.text.get(remove_start..remove_end)?.to_string();
    let retained_text = segment.text.get(remove_end..)?.to_string();
    let removed_segment = TextSegment {
        text: removed_text,
        start: cluster.start,
        end: cluster.end,
        properties: segment.properties.clone(),
        revision: segment.revision.clone(),
        hyperlink: segment.hyperlink.clone(),
        leading_width: 0.0,
    };

    segment.text = retained_text;
    segment.start = cluster.end;
    if segment.text.is_empty() {
        line.segments.remove(idx);
    }
    line.clusters.remove(0);
    Some((cluster, removed_segment))
}

/// Appends a cluster+segment pair to `line`, merging with tail when possible.
fn append_cluster_to_line_end(
    line: &mut WrappedLine,
    mut cluster: InlineCluster,
    mut segment: TextSegment,
) {
    cluster.forbid_line_start = false;
    cluster.forbid_line_end = false;
    line.clusters.push(cluster);
    segment.leading_width = 0.0;

    if let Some(last) = line.segments.last_mut() {
        if last.properties == segment.properties
            && last.revision == segment.revision
            && last.hyperlink == segment.hyperlink
            && last.end == segment.start
        {
            last.text.push_str(&segment.text);
            last.end = segment.end;
            return;
        }
    }
    line.segments.push(segment);
}

/// Classifies a grapheme into the font-slot categories used by Word layout.
///
/// Unicode `Script` alone is insufficient for Japanese typography: kana-block
/// punctuation and the prolonged sound mark can report `Common`, while Word
/// selects the East-Asian font slot for them in Japanese runs. Classifying the
/// complete Hiragana/Katakana blocks here keeps measurement, auto-spacing, and
/// drawing on the same slot. `classify_script_fast_path_matches_slow_path` and
/// the Japanese Common-script regression guard this behavior.
pub fn classify_script(grapheme: &str) -> ScriptCategory {
    let bytes = grapheme.as_bytes();
    if !bytes.is_empty() {
        let first_byte = bytes[0];
        if first_byte < 0x80 {
            return ScriptCategory::Latin;
        }
    }

    // Fast path for the most common East Asian letters (CJK ideographs + kana),
    // which dominate Japanese/Chinese text. These ranges are unambiguously
    // Han/Hiragana/Katakana per Unicode, so they always classify as East Asian
    // and we can skip the general script-property lookup. Punctuation, prolonged
    // sound marks and iteration marks live in the kana blocks but have the
    // Unicode Script value `Common`. Word nevertheless selects the East Asian
    // font slot for them in Japanese text, so classify the complete Hiragana
    // and Katakana blocks here rather than losing their Script_Extensions
    // context in the slow `char::script()` fallback.
    if let Some(ch) = grapheme.chars().next() {
        let cp = ch as u32;
        if (0x4E00..=0x9FFF).contains(&cp)   // CJK Unified Ideographs
            || (0x3040..=0x309F).contains(&cp)  // Hiragana block
            || (0x30A0..=0x30FF).contains(&cp)  // Katakana block
            || (0x3400..=0x4DBF).contains(&cp)
        // CJK Extension A
        {
            return ScriptCategory::EastAsia;
        }
    }

    classify_script_slow(grapheme)
}

/// Slow fallback that inspects each Unicode scalar for script hints/punctuation.
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

/// Returns true when the character is considered CJK punctuation for breaking logic.
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

#[cfg(test)]
mod float_exclusion_tests {
    use super::*;

    fn excl(top: f64, bottom: f64, left: f64, right: f64) -> FloatExclusion {
        FloatExclusion {
            top,
            bottom,
            left,
            right,
            wrap_text: WrapTextSide::BothSides,
            polygon: None,
        }
    }

    fn excl_side(
        top: f64,
        bottom: f64,
        left: f64,
        right: f64,
        wrap_text: WrapTextSide,
    ) -> FloatExclusion {
        FloatExclusion {
            top,
            bottom,
            left,
            right,
            wrap_text,
            polygon: None,
        }
    }

    #[test]
    fn no_exclusions_yields_full_content_box() {
        let (start, width) = line_available_segment(0.0, 20.0, 100.0, 500.0, &[]);
        assert_eq!(start, 100.0);
        assert_eq!(width, 400.0);
    }

    #[test]
    fn exclusion_outside_line_band_is_ignored() {
        // Float sits well below this line band → no effect.
        let floats = [excl(200.0, 300.0, 100.0, 250.0)];
        let (start, width) = line_available_segment(0.0, 20.0, 100.0, 500.0, &floats);
        assert_eq!(start, 100.0);
        assert_eq!(width, 400.0);
    }

    #[test]
    fn touching_edges_do_not_count_as_overlap() {
        // Float bottom exactly at line top → no overlap (half-open band).
        let floats = [excl(0.0, 10.0, 100.0, 250.0)];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 100.0);
        assert_eq!(width, 400.0);
    }

    #[test]
    fn left_anchored_float_shifts_text_to_the_right() {
        // Image hugging the left margin → text starts to its right.
        let floats = [excl(0.0, 100.0, 100.0, 250.0)];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 250.0);
        assert_eq!(width, 250.0);
    }

    #[test]
    fn right_anchored_float_shrinks_text_width() {
        // Image at the right edge → text keeps its start but loses width.
        let floats = [excl(0.0, 100.0, 350.0, 500.0)];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 100.0);
        assert_eq!(width, 250.0);
    }

    #[test]
    fn full_width_float_leaves_no_room() {
        let floats = [excl(0.0, 100.0, 50.0, 600.0)];
        let (_start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(width, 0.0);
    }

    #[test]
    fn center_float_picks_the_wider_side() {
        // Float occupies [220, 300]; left gap = 120, right gap = 200 → right wins.
        let floats = [excl(0.0, 100.0, 220.0, 300.0)];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 300.0);
        assert_eq!(width, 200.0);
    }

    #[test]
    fn left_only_wrap_closes_the_larger_right_interval() {
        let floats = [excl_side(0.0, 40.0, 250.0, 300.0, WrapTextSide::Left)];
        let segment = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(segment, (100.0, 150.0));
    }

    #[test]
    fn right_only_wrap_closes_the_larger_left_interval() {
        let floats = [excl_side(0.0, 40.0, 300.0, 350.0, WrapTextSide::Right)];
        let segment = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(segment, (350.0, 150.0));
    }

    #[test]
    fn tight_polygon_narrows_the_exclusion_toward_its_apex() {
        let mut float = excl(0.0, 100.0, 200.0, 400.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 300.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: false,
        });

        assert_eq!(
            line_available_segment(0.0, 20.0, 100.0, 500.0, &[float.clone()]),
            (100.0, 180.0)
        );
        assert_eq!(
            line_available_segment(80.0, 100.0, 100.0, 500.0, &[float]),
            (100.0, 100.0)
        );
    }

    #[test]
    fn tight_polygon_applies_horizontal_and_vertical_clearance() {
        let mut float = excl(-10.0, 110.0, 190.0, 410.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 300.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 10.0,
            distance_bottom: 10.0,
            distance_left: 10.0,
            distance_right: 10.0,
            allow_internal_gaps: false,
        });

        // The line above the original apex still intersects the expanded
        // contour; horizontal distances widen the sampled span.
        assert_eq!(
            line_available_segment(-10.0, 0.0, 100.0, 500.0, &[float]),
            (100.0, 180.0)
        );
    }

    #[test]
    fn invalid_polygon_falls_back_to_rectangular_exclusion() {
        let mut float = excl(0.0, 100.0, 200.0, 400.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 300.0, y: 0.0 },
                FloatPoint { x: 300.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: false,
        });
        assert_eq!(
            line_available_segment(0.0, 20.0, 100.0, 500.0, &[float]),
            (100.0, 100.0)
        );
    }

    #[test]
    fn through_polygon_keeps_concave_opening_as_a_free_interval() {
        let mut float = excl(0.0, 100.0, 200.0, 400.0);
        float.polygon = Some(FloatWrapPolygon {
            // A downward-opening U: the lower line band intersects two narrow
            // legs with a wider opening between them.
            points: vec![
                FloatPoint { x: 200.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 370.0, y: 100.0 },
                FloatPoint { x: 370.0, y: 30.0 },
                FloatPoint { x: 230.0, y: 30.0 },
                FloatPoint { x: 230.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: true,
        });

        assert_eq!(
            line_available_segment(60.0, 80.0, 100.0, 500.0, &[float.clone()]),
            (230.0, 140.0)
        );
        // A band crossing the U's shoulder is conservatively blocked across
        // the full outer span because the top bar intersects that text line.
        assert_eq!(
            line_available_segment(20.0, 40.0, 100.0, 500.0, &[float.clone()]),
            (100.0, 100.0)
        );

        // `largest` means the larger exterior side, even when an internal
        // opening is wider than either exterior interval.
        let mut largest = float.clone();
        largest.wrap_text = WrapTextSide::Largest;
        assert_eq!(
            line_available_segment(60.0, 80.0, 100.0, 500.0, &[largest]),
            (100.0, 100.0)
        );

        // `wrapTight` deliberately keeps the old outer-span semantics for the
        // same contour; only `wrapThrough` opens internal gaps.
        float.polygon.as_mut().unwrap().allow_internal_gaps = false;
        assert_eq!(
            line_available_segment(60.0, 80.0, 100.0, 500.0, &[float]),
            (100.0, 100.0)
        );
    }

    #[test]
    fn overlapping_floats_are_merged() {
        // Two overlapping left floats merge into [100, 280]; right gap = 220.
        let floats = [
            excl(0.0, 100.0, 100.0, 250.0),
            excl(0.0, 100.0, 200.0, 280.0),
        ];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 280.0);
        assert_eq!(width, 220.0);
    }

    #[test]
    fn float_clipped_to_content_box() {
        // Float extends left of the content box; only the in-box part counts.
        let floats = [excl(0.0, 100.0, 0.0, 180.0)];
        let (start, width) = line_available_segment(10.0, 30.0, 100.0, 500.0, &floats);
        assert_eq!(start, 180.0);
        assert_eq!(width, 320.0);
    }

    #[test]
    fn planner_returns_full_box_for_every_line_without_floats() {
        let plan = plan_line_segments_around_floats(0.0, 20.0, 3, 100.0, 500.0, &[]);
        assert_eq!(plan, vec![(100.0, 400.0); 3]);
    }

    #[test]
    fn planner_shifts_lines_inside_left_float_and_reverts_below() {
        // Left float occupies y[0, 45) → lines 0 and 1 (bands [0,20) and [20,40))
        // are shifted right; line 2 (band [40,60)) only partially overlaps the
        // float's [0,45) band so it is still shifted, line 3 reverts to full box.
        let floats = [excl(0.0, 45.0, 100.0, 250.0)];
        let plan = plan_line_segments_around_floats(0.0, 20.0, 4, 100.0, 500.0, &floats);
        assert_eq!(plan[0], (250.0, 250.0));
        assert_eq!(plan[1], (250.0, 250.0));
        assert_eq!(plan[2], (250.0, 250.0)); // band [40,60) ∩ [0,45) = [40,45)
        assert_eq!(plan[3], (100.0, 400.0)); // band [60,80) is clear
    }

    #[test]
    fn planner_reports_zero_width_when_float_covers_a_line() {
        // Float covers the whole content box for the first line only.
        let floats = [excl(0.0, 20.0, 50.0, 600.0)];
        let plan = plan_line_segments_around_floats(0.0, 20.0, 2, 100.0, 500.0, &floats);
        assert_eq!(plan[0].1, 0.0);
        assert_eq!(plan[1], (100.0, 400.0));
    }

    #[test]
    fn contour_font_box_excludes_leading_and_preserves_band_advance() {
        let mut float = excl(0.0, 100.0, 200.0, 400.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 300.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: false,
        });
        let plan = plan_usable_line_segments_with_contour_font_box(
            0.0,
            20.0,
            Some(10.0),
            2,
            100.0,
            500.0,
            &[float.clone()],
            false,
        );
        assert_eq!(
            plan.iter()
                .map(|s| (s.start_x, s.width))
                .collect::<Vec<_>>(),
            vec![
                (100.0, 190.0),
                (310.0, 190.0),
                (100.0, 170.0),
                (330.0, 170.0)
            ]
        );
        assert!(plan[1].shares_band_with_previous);
        assert!(!plan[2].shares_band_with_previous);
        let full =
            plan_usable_line_segments_around_floats(0.0, 20.0, 2, 100.0, 500.0, &[float.clone()]);
        for bad in [None, Some(0.0), Some(f64::NAN)] {
            assert_eq!(
                plan_usable_line_segments_with_contour_font_box(
                    0.0,
                    20.0,
                    bad,
                    2,
                    100.0,
                    500.0,
                    &[float.clone()],
                    false
                ),
                full
            );
        }
        float.polygon = None;
        assert_eq!(
            plan_usable_line_segments_with_contour_font_box(
                0.0,
                20.0,
                Some(10.0),
                2,
                100.0,
                500.0,
                &[float.clone()],
                false
            ),
            plan_usable_line_segments_around_floats(0.0, 20.0, 2, 100.0, 500.0, &[float])
        );
    }

    #[test]
    fn both_sides_tight_contour_keeps_both_free_segments_in_one_band() {
        let mut float = excl(0.0, 100.0, 200.0, 400.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 300.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: false,
        });
        assert_eq!(
            line_available_segments(0.0, 20.0, 100.0, 500.0, &[float.clone()]).as_slice(),
            &[(100.0, 180.0), (320.0, 180.0)]
        );
        let plan = plan_usable_line_segments_around_floats(0.0, 20.0, 2, 100.0, 500.0, &[float]);
        assert_eq!(plan.len(), 4);
        assert!(plan[1].shares_band_with_previous);
        assert!(plan[3].shares_band_with_previous);
        assert_eq!(
            ParagraphStyle {
                float_line_segments: plan,
                ..Default::default()
            }
            .float_band_starts(4),
            vec![0, 2]
        );
    }

    #[test]
    fn both_sides_through_contour_keeps_center_and_outer_segments() {
        let mut float = excl(0.0, 100.0, 150.0, 450.0);
        float.polygon = Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 150.0, y: 0.0 },
                FloatPoint { x: 450.0, y: 0.0 },
                FloatPoint { x: 450.0, y: 100.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 400.0, y: 30.0 },
                FloatPoint { x: 200.0, y: 30.0 },
                FloatPoint { x: 200.0, y: 100.0 },
                FloatPoint { x: 150.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: true,
        });
        assert_eq!(
            line_available_segments(40.0, 60.0, 0.0, 600.0, &[float.clone()]).as_slice(),
            &[(0.0, 150.0), (200.0, 200.0), (450.0, 150.0)]
        );
        float.polygon.as_mut().unwrap().allow_internal_gaps = false;
        assert_eq!(
            line_available_segments(40.0, 60.0, 0.0, 600.0, &[float]).as_slice(),
            &[(0.0, 150.0), (450.0, 150.0)]
        );
    }

    #[test]
    fn numbered_first_line_offsets_keep_direct_suffixes_and_tab_floors() {
        for suffix in [Some("space"), Some("nothing")] {
            assert_eq!(numbering_first_line_offset(100.0, 80.0, 0.0, suffix), -20.0);
            assert_eq!(numbering_first_line_offset(100.0, 120.0, 0.0, suffix), 20.0);
        }
        for suffix in [None, Some("tab"), Some("unknown")] {
            assert_eq!(numbering_first_line_offset(100.0, 80.0, 3.0, suffix), 3.0);
            assert_eq!(numbering_first_line_offset(100.0, 120.0, 3.0, suffix), 20.0);
            assert_eq!(numbering_first_line_offset(100.0, 101.0, 3.0, suffix), 3.0);
        }
    }

    #[test]
    fn both_sides_square_wrap_fills_both_gaps_of_a_band() {
        // A centred square float leaves 150px on each side of a 100..500 box.
        let floats = [excl(0.0, 40.0, 250.0, 350.0)];
        let segments = line_available_segments(0.0, 20.0, 100.0, 500.0, &floats);
        assert_eq!(segments.as_slice(), &[(100.0, 150.0), (350.0, 150.0)]);

        let plan = plan_usable_line_segments_around_floats(0.0, 20.0, 3, 100.0, 500.0, &floats);
        let shape = plan
            .iter()
            .map(|segment| {
                (
                    segment.start_x,
                    segment.width,
                    segment.shares_band_with_previous,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            vec![
                (100.0, 150.0, false),
                (350.0, 150.0, true),
                (100.0, 150.0, false),
                (350.0, 150.0, true),
                (100.0, 400.0, false),
            ]
        );
        let style = ParagraphStyle {
            float_line_segments: plan,
            ..ParagraphStyle::default()
        };
        assert!(!style.line_shares_float_band(0));
        assert!(style.line_shares_float_band(1));
        assert!(!style.line_shares_float_band(4));
        // Five wrapped lines occupy three physical bands.
        assert_eq!(style.float_band_starts(5), vec![0, 2, 4]);
        // Lines past the plan are ordinary bands.
        assert_eq!(style.float_band_starts(7), vec![0, 2, 4, 5, 6]);
        assert_eq!(
            ParagraphStyle::default().float_band_starts(3),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn both_sides_keeps_one_side_for_slivers_and_other_wrap_modes() {
        // The left gap is narrower than the minimum: widest side only.
        let floats = [excl(0.0, 20.0, 120.0, 300.0)];
        let segments = line_available_segments(0.0, 20.0, 100.0, 500.0, &floats);
        assert_eq!(segments.as_slice(), &[(300.0, 200.0)]);
        // `largest` and one-sided wraps keep a single interval.
        for side in [
            WrapTextSide::Largest,
            WrapTextSide::Left,
            WrapTextSide::Right,
        ] {
            let floats = [FloatExclusion {
                wrap_text: side,
                ..excl(0.0, 20.0, 250.0, 350.0)
            }];
            assert_eq!(
                line_available_segments(0.0, 20.0, 100.0, 500.0, &floats).len(),
                1,
                "{side:?}"
            );
        }
        // Top-and-bottom covers the whole band: nothing to fill.
        let floats = [excl(0.0, 20.0, f64::NEG_INFINITY, f64::INFINITY)];
        assert_eq!(
            line_available_segments(0.0, 20.0, 100.0, 500.0, &floats).as_slice(),
            &[(100.0, 0.0)]
        );
    }

    #[test]
    fn usable_planner_skips_fully_covered_bands_without_creating_text_lines() {
        let floats = [excl(0.0, 40.0, 50.0, 600.0)];
        let plan = plan_usable_line_segments_around_floats(0.0, 20.0, 4, 100.0, 500.0, &floats);
        assert_eq!(
            plan,
            vec![
                FloatLineSegment {
                    start_x: 100.0,
                    width: 400.0,
                    skipped_lines_before: 2,
                    exact_clearance_height: 0.0,
                    shares_band_with_previous: false,
                },
                FloatLineSegment {
                    start_x: 100.0,
                    width: 400.0,
                    skipped_lines_before: 0,
                    exact_clearance_height: 0.0,
                    shares_band_with_previous: false,
                },
            ]
        );
    }

    #[test]
    fn nongrid_float_clearance_stops_at_the_exclusion_bottom() {
        let floats = [excl(5.0, 41.5, f64::NEG_INFINITY, f64::INFINITY)];
        let plan = plan_usable_line_segments_around_floats_with_exact_clearance(
            0.0, 20.0, 6, 100.0, 500.0, &floats,
        );
        let style = ParagraphStyle {
            float_line_segments: plan,
            ..ParagraphStyle::default()
        };
        assert_eq!(style.skipped_float_height_before(0, 20.0), 41.5);
        assert_eq!(style.skipped_float_height_before(0, 30.0), 41.5);
        assert_eq!(style.skipped_float_height_before(1, 20.0), 0.0);
        assert_eq!(style.line_base_width(400.0, 0), 400.0);
        let grid = ParagraphStyle {
            float_line_segments: plan_usable_line_segments_around_floats(
                0.0, 20.0, 6, 100.0, 500.0, &floats,
            ),
            ..ParagraphStyle::default()
        };
        assert_eq!(grid.skipped_float_height_before(0, 20.0), 60.0);
    }

    #[test]
    fn exact_float_clearance_rechecks_stacked_objects_and_both_sides() {
        let floats = [
            excl(0.0, 41.5, f64::NEG_INFINITY, f64::INFINITY),
            excl(40.0, 73.25, f64::NEG_INFINITY, f64::INFINITY),
            excl(73.25, 120.0, 250.0, 350.0),
        ];
        let plan = plan_usable_line_segments_around_floats_with_exact_clearance(
            0.0, 20.0, 8, 100.0, 500.0, &floats,
        );
        let style = ParagraphStyle {
            float_line_segments: plan,
            ..ParagraphStyle::default()
        };
        assert_eq!(style.skipped_float_height_before(0, 20.0), 73.25);
        assert_eq!(style.line_base_width(400.0, 0), 150.0);
        assert_eq!(style.line_base_width(400.0, 1), 150.0);
        assert!(style.line_shares_float_band(1));
        assert_eq!(style.skipped_float_height_before(1, 20.0), 0.0);
        assert!(!style.line_shares_float_band(2));
        assert_eq!(style.skipped_float_height_before(2, 20.0), 0.0);
    }

    #[test]
    fn usable_planner_matches_full_plan_with_mixed_float_coverage() {
        let floats = [
            excl(0.0, 40.0, 50.0, 600.0),
            excl(60.0, 100.0, 100.0, 250.0),
        ];
        let full = plan_line_segments_around_floats(0.0, 20.0, 6, 100.0, 500.0, &floats);
        let usable = plan_usable_line_segments_around_floats(0.0, 20.0, 6, 100.0, 500.0, &floats);
        let mut skipped = 0;
        let expected: Vec<_> = full
            .into_iter()
            .filter_map(|(start_x, width)| {
                if width <= f64::EPSILON {
                    skipped += 1;
                    None
                } else {
                    Some(FloatLineSegment {
                        start_x,
                        width,
                        skipped_lines_before: std::mem::take(&mut skipped),
                        exact_clearance_height: 0.0,
                        shares_band_with_previous: false,
                    })
                }
            })
            .collect();
        assert_eq!(usable, expected);
    }

    #[test]
    fn planner_handles_zero_line_count_and_nonpositive_height() {
        assert!(plan_line_segments_around_floats(0.0, 20.0, 0, 100.0, 500.0, &[]).is_empty());
        // Non-positive / non-finite height collapses every band to zero thickness
        // anchored at first_line_top, so all lines share the same band [30,30).
        // That degenerate band still sits inside the float's [0,45) span, so each
        // line is shifted identically (no Y advance between lines).
        let floats = [excl(0.0, 45.0, 100.0, 250.0)];
        let plan = plan_line_segments_around_floats(30.0, 0.0, 2, 100.0, 500.0, &floats);
        assert_eq!(plan, vec![(250.0, 250.0); 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ECMA-376 §17.15.1.58-59: a Japanese custom set replaces the default
    /// table; a custom set for another language only adds to it (jlsi/jade#467).
    #[test]
    fn japanese_custom_kinsoku_sets_replace_the_default_tables() {
        let style = |replaces_default: bool| ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                no_line_break_before: vec!["、".into(), "々".into()],
                no_line_break_after: vec!["「".into()],
                no_line_break_before_replaces_default: replaces_default,
                no_line_break_after_replaces_default: replaces_default,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let replacing = style(true);
        assert!(replacing.forbids_line_start("、"));
        assert!(replacing.forbids_line_start("々"));
        assert!(!replacing.forbids_line_start("。"));
        assert!(!replacing.forbids_line_start("ッ"));
        assert!(replacing.forbids_line_end("「"));
        assert!(!replacing.forbids_line_end("（"));

        let extending = style(false);
        assert!(extending.forbids_line_start("、"));
        assert!(extending.forbids_line_start("。"));
        assert!(extending.forbids_line_start("）"));
        assert!(extending.forbids_line_end("（"));
    }

    #[test]
    fn indent_char_units_supersede_twips_values() {
        // ECMA-376 §17.3.1.12: `w:leftChars` supersedes `w:left` when both are
        // present. Japanese Word writes both, with the chars value authoritative.
        let indent = Indent {
            left: Some(36.0),
            left_chars: Some(200),
            right: Some(72.0),
            right_chars: Some(100),
            ..Indent::default()
        };
        // 200 chars = 2 characters; at 12pt char unit → 24pt.
        assert_eq!(resolve_indent_left_pt(&indent, false, 12.0), Some(24.0));
        assert_eq!(resolve_indent_right_pt(&indent, false, 12.0), Some(12.0));

        // An explicit 0 chars re-enables the twips value.
        let neutralized = Indent {
            left: Some(36.0),
            left_chars: Some(0),
            ..Indent::default()
        };
        assert_eq!(
            resolve_indent_left_pt(&neutralized, false, 12.0),
            Some(36.0),
            "leftChars=0 must fall back to w:left"
        );

        // Logical spellings stay bidi-aware: on an RTL paragraph the leading
        // edge comes from `w:endChars`/`w:end`.
        let logical = Indent {
            start: Some(36.0),
            end: Some(18.0),
            start_chars: Some(200),
            end_chars: Some(100),
            ..Indent::default()
        };
        assert_eq!(resolve_indent_left_pt(&logical, false, 12.0), Some(24.0));
        assert_eq!(resolve_indent_left_pt(&logical, true, 12.0), Some(12.0));
    }

    #[test]
    fn hanging_supersedes_first_line_when_both_present() {
        // ECMA-376 §17.3.1.12: when both `w:firstLine` and `w:hanging` are
        // present, `hanging` wins.
        let indent = Indent {
            first_line: Some(24.0),
            hanging: Some(48.0),
            ..Indent::default()
        };
        assert_eq!(
            resolve_indent_first_line_pt(&indent, 10.5, true),
            Some(-48.0)
        );

        // hangingChars supersedes hanging; firstLineChars supersedes firstLine.
        let chars = Indent {
            first_line: Some(24.0),
            first_line_chars: Some(100),
            hanging: Some(48.0),
            hanging_chars: Some(200),
            ..Indent::default()
        };
        assert_eq!(
            resolve_indent_first_line_pt(&chars, 10.5, true),
            Some(-21.0),
            "hangingChars=200 at 10.5pt char unit"
        );
        assert_eq!(
            resolve_indent_first_line_pt(&chars, 10.5, false),
            Some(10.5),
            "numbered path skips hanging and honors firstLineChars"
        );
    }

    #[test]
    fn apply_paragraph_indent_scales_char_units_with_char_unit_pt() {
        let indent = Indent {
            left_chars: Some(100),
            first_line_chars: Some(100),
            ..Indent::default()
        };
        let mut style = ParagraphStyle::default();
        let mut x = 0.0;
        apply_paragraph_indent_with_char_unit(&mut x, &mut style, &indent, false, 12.0);
        assert!((x - points_to_px(12.0)).abs() < 1e-6);
        assert!((style.first_line_indent - points_to_px(12.0)).abs() < 1e-6);

        // Default entry point keeps the 10.5pt Japanese body default.
        let mut style_default = ParagraphStyle::default();
        let mut x_default = 0.0;
        apply_paragraph_indent(&mut x_default, &mut style_default, &indent, false);
        assert!((x_default - points_to_px(10.5)).abs() < 1e-6);
    }

    /// Word for Mac 16.114 indent probe: a character-unit left indent places
    /// the first line there and hangs the continuation lines past it by the
    /// hanging amount; a point left indent subtracts the hanging from the
    /// first line instead.
    #[test]
    fn character_left_indent_hangs_continuation_lines_past_it() {
        let cases = [
            // leftChars=202 left=565 hangingChars=67 hanging=141
            (Some(202), Some(28.25), Some(67), Some(7.05), 21.21, 28.245),
            // leftChars=202 hangingChars=67, no twips values
            (Some(202), None, Some(67), None, 21.21, 28.245),
            // a twips hanging also hangs past a character left indent
            (Some(202), Some(28.25), None, Some(5.65), 21.21, 26.86),
            // leftChars=202 left=565 without any hanging
            (Some(202), Some(28.25), None, None, 21.21, 21.21),
            // point indents: the first line starts left - hanging
            (None, Some(28.25), None, Some(7.05), 21.2, 28.25),
        ];
        for (left_chars, left, hanging_chars, hanging, first_pt, continuation_pt) in cases {
            let indent = Indent {
                left_chars,
                left: left.map(|value: f64| value as f32),
                hanging_chars,
                hanging: hanging.map(|value: f64| value as f32),
                ..Indent::default()
            };
            let mut style = ParagraphStyle::default();
            let mut x = 0.0;
            apply_paragraph_indent_with_char_unit(&mut x, &mut style, &indent, false, 10.5);
            let case = format!("{indent:?}");
            assert!(
                (x - points_to_px(continuation_pt as f32)).abs() < 1e-3,
                "{case}: continuation {x}"
            );
            assert!(
                (x + style.first_line_indent - points_to_px(first_pt as f32)).abs() < 1e-3,
                "{case}: first line {}",
                x + style.first_line_indent
            );
        }
    }

    /// Word for Mac 16.114 indent probe over a numbering level with
    /// `w:left="533" w:hanging="113"`: where the marker and continuation
    /// lines start for each paragraph indent.
    #[test]
    fn numbered_paragraph_origin_follows_word_indent_precedence() {
        let cases: [(Indent, f64, f64); 6] = [
            // w:left="420" w:firstLine="4": firstLine replaces the hanging
            (
                Indent {
                    left: Some(21.0),
                    first_line: Some(0.2),
                    ..Indent::default()
                },
                21.2,
                21.0,
            ),
            // leftChars=202 left=565 hangingChars=67 hanging=141
            (
                Indent {
                    left_chars: Some(202),
                    left: Some(28.25),
                    hanging_chars: Some(67),
                    hanging: Some(7.05),
                    ..Indent::default()
                },
                21.21,
                28.245,
            ),
            // left=565 hanging=141
            (
                Indent {
                    left: Some(28.25),
                    hanging: Some(7.05),
                    ..Indent::default()
                },
                21.2,
                28.25,
            ),
            // leftChars=202 left=565: the level's hanging hangs past it
            (
                Indent {
                    left_chars: Some(202),
                    left: Some(28.25),
                    ..Indent::default()
                },
                21.21,
                26.86,
            ),
            // leftChars=202 hangingChars=67 only: no level left on top
            (
                Indent {
                    left_chars: Some(202),
                    hanging_chars: Some(67),
                    ..Indent::default()
                },
                21.21,
                28.245,
            ),
            // no paragraph indent: the level's
            (Indent::default(), 21.0, 26.65),
        ];
        for (paragraph, marker_pt, continuation_pt) in cases {
            let mut style = ParagraphStyle::default();
            let mut x = 0.0;
            apply_paragraph_indent_with_char_unit(&mut x, &mut style, &paragraph, true, 10.5);
            let origin = numbered_paragraph_origin(
                x,
                style.first_line_indent,
                Some(&paragraph),
                Some(26.65),
                Some(5.65),
                false,
                10.5,
            );
            let case = format!("{paragraph:?}");
            assert!(
                (origin.number_x - points_to_px(marker_pt as f32)).abs() < 1e-2,
                "{case}: marker {}",
                origin.number_x
            );
            assert!(
                (origin.x - points_to_px(continuation_pt as f32)).abs() < 1e-2,
                "{case}: continuation {}",
                origin.x
            );
            assert!(
                (origin.body_start - (origin.x + style.first_line_indent)).abs() < 1e-9,
                "{case}: body start"
            );
        }
    }

    /// Unambiguous script values handled by the fast path must agree with the
    /// general script-property lookup it bypasses.
    #[test]
    fn classify_script_fast_path_matches_slow_path() {
        let samples = [
            "本", "契", "約", "甲", "乙", // CJK ideographs
            "あ", "い", "は", "の", "を", // Hiragana letters
            "ア", "カ", "ン", "ヲ", // Katakana letters
            "、", "。", "「", "」", // punctuation (slow path)
            "（", "）", "Ａ", "1", "a", " ", // fullwidth/ASCII/space
        ];
        for s in samples {
            assert_eq!(
                classify_script(s),
                classify_script_slow(s),
                "fast and slow classification diverged for {s:?}"
            );
        }
    }

    #[test]
    fn classifies_kana_common_characters_in_the_east_asian_font_slot() {
        // U+30FC and the iteration marks have Script=Common plus Japanese
        // Script_Extensions. Word measures them with the East Asian run font,
        // not as Latin text with autoSpaceDE on both sides.
        for grapheme in ["ー", "ヽ", "ヾ", "ゝ", "ゞ"] {
            assert_eq!(classify_script(grapheme), ScriptCategory::EastAsia);
        }
    }

    use crate::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};
    use crate::line_metrics::resolve_line_height_points;
    use crate::units::points_to_px;
    use futures::executor;
    use jade_docx_reader::{
        Body, BodyElement, Document, DocumentParser, DocumentSettings, FontHint, OpenXmlPackage,
        PageSettings, Paragraph, ParagraphProperties, Run, RunFitText, RunFonts, RunProperties,
        StylesPart, TabStop,
    };
    use proptest::prelude::*;
    use std::cell::RefCell;
    use std::fs;
    use std::hint::black_box;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Instant;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn word_distribution_tokens_are_justified_alignments() {
        for alignment in [
            "both",
            "justify",
            "distribute",
            "thaiDistribute",
            "lowKashida",
            "mediumKashida",
            "highKashida",
        ] {
            assert!(
                is_justified_alignment(alignment),
                "{alignment} must distribute a soft line"
            );
        }
    }

    #[test]
    fn soft_alignment_uses_visible_extent_and_preserves_explicit_separators() {
        let glyphs = [(" ", 23.0, 4.0), (" ", 20.0, 3.0), ("B", 10.0, 10.0)];
        assert_eq!(
            soft_line_alignment_width_px(27.0, LineBreakKind::Soft, glyphs),
            20.0
        );
        for kind in [
            LineBreakKind::Hard,
            LineBreakKind::Page,
            LineBreakKind::ParagraphEnd,
        ] {
            assert_eq!(soft_line_alignment_width_px(27.0, kind, glyphs), 27.0);
        }
        for tail in ["\t", "\u{a0}", "\u{3000}", ""] {
            assert_eq!(
                soft_line_alignment_width_px(
                    27.0,
                    LineBreakKind::Soft,
                    [(tail, 23.0, 4.0), ("B", 10.0, 10.0)]
                ),
                27.0
            );
        }
        assert_eq!(
            soft_line_alignment_width_px(
                27.0,
                LineBreakKind::Soft,
                [(" ", 23.0, 4.0), ("B", f64::NAN, 10.0)]
            ),
            27.0
        );
        assert_eq!(
            soft_line_alignment_width_px(
                7.0,
                LineBreakKind::Soft,
                [(" ", 3.0, 4.0), (" ", 0.0, 3.0)]
            ),
            0.0
        );
    }

    #[test]
    fn soft_alignment_visits_only_the_trailing_separator_and_final_visible_glyph() {
        let mut visited = 0;
        let glyphs = [
            (" ", 30.0, 3.0),
            ("C", 20.0, 10.0),
            ("B", 10.0, 10.0),
            ("A", 0.0, 10.0),
        ];
        let width = soft_line_alignment_width_px(
            33.0,
            LineBreakKind::Soft,
            glyphs.into_iter().inspect(|_| visited += 1),
        );
        assert_eq!(width, 30.0);
        assert_eq!(visited, 2);
    }

    #[test]
    fn logical_start_and_end_follow_paragraph_direction() {
        assert_eq!(
            resolve_physical_paragraph_alignment("start", false),
            PhysicalParagraphAlignment::Left,
        );
        assert_eq!(
            resolve_physical_paragraph_alignment("start", true),
            PhysicalParagraphAlignment::Right,
        );
        assert_eq!(
            resolve_physical_paragraph_alignment("end", false),
            PhysicalParagraphAlignment::Right,
        );
        assert_eq!(
            resolve_physical_paragraph_alignment("end", true),
            PhysicalParagraphAlignment::Left,
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        #[test]
        fn word_justification_tokens_ignore_ascii_case_and_surrounding_space(
            token in prop::sample::select(vec![
                "both",
                "justify",
                "distribute",
                "thaiDistribute",
                "lowKashida",
                "mediumKashida",
                "highKashida",
            ]),
            uppercase in any::<bool>(),
            padding in 0_usize..=4,
        ) {
            let token = if uppercase {
                token.to_ascii_uppercase()
            } else {
                token.to_string()
            };
            let padded = format!("{}{}{}", " ".repeat(padding), token, " ".repeat(padding));
            prop_assert!(is_justified_alignment(&padded));
        }

        #[test]
        fn logical_alignment_resolution_matches_direction_for_arbitrary_case(
            token in prop::sample::select(vec!["start", "end", "left", "right"]),
            bidi in any::<bool>(),
            uppercase in any::<bool>(),
        ) {
            let value = if uppercase {
                token.to_ascii_uppercase()
            } else {
                token.to_string()
            };
            let expected = match token {
                "start" if bidi => PhysicalParagraphAlignment::Right,
                "start" => PhysicalParagraphAlignment::Left,
                "end" if bidi => PhysicalParagraphAlignment::Left,
                "end" => PhysicalParagraphAlignment::Right,
                "right" => PhysicalParagraphAlignment::Right,
                _ => PhysicalParagraphAlignment::Left,
            };
            prop_assert_eq!(resolve_physical_paragraph_alignment(&value, bidi), expected);
        }
    }

    struct TestContext {
        style: ResolvedRunStyle,
        width_per_grapheme: f64,
        vertical_width_per_grapheme: Option<f64>,
        measurements: Rc<RefCell<Vec<String>>>,
    }

    impl TestContext {
        fn new(width_per_grapheme: f64) -> Self {
            let style = ResolvedRunStyle::new(
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                0.0,
                1.0,
                false,
                None,
            );
            Self {
                style,
                width_per_grapheme,
                vertical_width_per_grapheme: None,
                measurements: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn with_vertical_width(mut self, width_per_grapheme: f64) -> Self {
            self.vertical_width_per_grapheme = Some(width_per_grapheme);
            self
        }

        fn recorded_measurements(&self) -> Vec<String> {
            self.measurements.borrow().clone()
        }
    }

    impl LineWrapContext for TestContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            self.measurements.borrow_mut().push(text.to_string());
            let grapheme_count = text.graphemes(true).count() as f64;
            Ok(grapheme_count * self.width_per_grapheme)
        }

        fn measure_text_for_vertical_stack(
            &self,
            text: &str,
            _font_descriptor: &str,
        ) -> Result<f64, Self::Error> {
            self.measurements.borrow_mut().push(text.to_string());
            let grapheme_count = text.graphemes(true).count() as f64;
            Ok(grapheme_count
                * self
                    .vertical_width_per_grapheme
                    .unwrap_or(self.width_per_grapheme))
        }
    }

    #[test]
    fn text_advance_uses_resolved_font_ids_when_available() {
        struct FontIdContext {
            seen: RefCell<Vec<Option<u32>>>,
        }

        impl LineWrapContext for FontIdContext {
            type Error = ();

            fn resolve_run_style(
                &self,
                _properties: Option<&RunProperties>,
            ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
                unreachable!("walk_text_advance_steps receives its resolved style directly")
            }

            fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
                Ok(text.graphemes(true).count() as f64)
            }

            fn measure_text_with_font_key(
                &self,
                text: &str,
                _font_descriptor: &str,
                _font_descriptor_hash: u64,
                font_descriptor_id: Option<u32>,
            ) -> Result<f64, Self::Error> {
                self.seen.borrow_mut().push(font_descriptor_id);
                Ok(text.graphemes(true).count() as f64)
            }
        }

        let style = ResolvedRunStyle::new(
            "ascii",
            "hansi",
            "east-asia",
            "complex",
            0.0,
            1.0,
            false,
            None,
        )
        .with_font_descriptor_ids(11, 22, 33, 44);
        let context = FontIdContext {
            seen: RefCell::new(Vec::new()),
        };
        let mut snap_cache = FxHashMap::default();
        walk_text_advance_steps(
            &context,
            "A漢",
            &style,
            None,
            &TextAdvanceState::default(),
            None,
            &mut snap_cache,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(*context.seen.borrow(), vec![Some(11), Some(33)]);
    }

    struct VariableWidthContext {
        style: ResolvedRunStyle,
        ascii_width: f64,
        cjk_width: f64,
    }

    struct ResolvingFixedWidthContext {
        width_per_grapheme: f64,
    }

    impl LineWrapContext for ResolvingFixedWidthContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(resolve_run_style_for_wrap(properties, None))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            Ok(text.graphemes(true).count() as f64 * self.width_per_grapheme)
        }
    }

    impl VariableWidthContext {
        fn new(ascii_width: f64, cjk_width: f64) -> Self {
            let style = ResolvedRunStyle::new(
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                Rc::<str>::from("12pt Test"),
                0.0,
                1.0,
                false,
                None,
            );
            Self {
                style,
                ascii_width,
                cjk_width,
            }
        }
    }

    impl LineWrapContext for VariableWidthContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            let mut width = 0.0;
            for grapheme in text.graphemes(true) {
                if grapheme.is_ascii() && !grapheme.trim().is_empty() {
                    width += self.ascii_width;
                } else {
                    width += self.cjk_width;
                }
            }
            Ok(width)
        }
    }

    struct FontAwareVariableWidthContext {
        ascii_width_at_12pt: f64,
        cjk_width_at_12pt: f64,
    }

    impl FontAwareVariableWidthContext {
        fn new(ascii_width_at_12pt: f64, cjk_width_at_12pt: f64) -> Self {
            Self {
                ascii_width_at_12pt,
                cjk_width_at_12pt,
            }
        }
    }

    impl LineWrapContext for FontAwareVariableWidthContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(resolve_run_style_for_wrap(properties, None))
        }

        fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
            fn font_size_from_descriptor(descriptor: &str) -> Option<f64> {
                descriptor
                    .split_whitespace()
                    .find_map(|part| part.strip_suffix("pt"))
                    .and_then(|value| value.parse::<f64>().ok())
            }

            let scale = font_size_from_descriptor(font_descriptor).unwrap_or(12.0) / 12.0;
            let mut width = 0.0;
            for grapheme in text.graphemes(true) {
                if grapheme.is_ascii() && !grapheme.trim().is_empty() {
                    width += self.ascii_width_at_12pt * scale;
                } else {
                    width += self.cjk_width_at_12pt * scale;
                }
            }
            Ok(width)
        }
    }

    struct KerningAwareContext {
        style: ResolvedRunStyle,
    }

    impl KerningAwareContext {
        fn new(enable_pair_kerning: bool) -> Self {
            Self {
                style: ResolvedRunStyle::new(
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    0.0,
                    1.0,
                    enable_pair_kerning,
                    None,
                ),
            }
        }
    }

    impl LineWrapContext for KerningAwareContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            let width = match text {
                "A" | "V" | "X" => 6.0,
                "AV" => 10.0,
                _ => text.graphemes(true).count() as f64 * 6.0,
            };
            Ok(width)
        }

        fn measure_font_vertical_metrics(
            &self,
            _font_descriptor: &str,
        ) -> Result<Option<(f64, f64)>, Self::Error> {
            Ok(Some((8.0, 2.0)))
        }
    }

    struct CountingKerningContext {
        style: ResolvedRunStyle,
        pair_measurements: Rc<RefCell<usize>>,
        pair_adjustment: f64,
        glyph_width: f64,
    }

    impl CountingKerningContext {
        fn new(enable_pair_kerning: bool) -> Self {
            Self {
                style: ResolvedRunStyle::new(
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    0.0,
                    1.0,
                    enable_pair_kerning,
                    None,
                ),
                pair_measurements: Rc::new(RefCell::new(0)),
                pair_adjustment: 0.0,
                glyph_width: 6.0,
            }
        }
    }

    impl LineWrapContext for CountingKerningContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            let pair = text.graphemes(true).count() == 2;
            if pair {
                *self.pair_measurements.borrow_mut() += 1;
            }
            Ok(text.graphemes(true).count() as f64 * self.glyph_width
                + if pair { self.pair_adjustment } else { 0.0 })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn legacy_compute_text_advance_step<C: LineWrapContext>(
        ctx: &C,
        grapheme: &str,
        style: &ResolvedRunStyle,
        paragraph_style: Option<&ParagraphStyle>,
        state: &TextAdvanceState,
        next_grapheme: Option<&str>,
        is_line_start: bool,
        snap_cell_width: Option<f64>,
    ) -> Result<OwnedTextAdvanceStep, C::Error> {
        let script = classify_script(grapheme);
        let (font_descriptor, font_descriptor_hash, font_descriptor_id) =
            style.descriptor_key_for_grapheme(grapheme, script);
        let font_descriptor = font_descriptor.to_string();
        let measured_glyph_width = ctx.measure_text_with_font_key(
            grapheme,
            font_descriptor.as_str(),
            font_descriptor_hash,
            font_descriptor_id,
        )?;
        let extra_spacing =
            if style.letter_spacing_px.abs() > f64::EPSILON && next_grapheme.is_some() {
                style.letter_spacing_px
            } else {
                0.0
            };
        let kerning_adjustment = if style.enable_pair_kerning
            && state.prev_grapheme.is_some()
            && state.prev_font_descriptor_hash == Some(font_descriptor_hash)
            && state.prev_raw_glyph_width.is_some()
        {
            let prev_grapheme = state.prev_grapheme.as_deref().unwrap_or_default();
            let prev_width = state.prev_raw_glyph_width.unwrap_or_default();
            let mut pair = String::with_capacity(prev_grapheme.len() + grapheme.len());
            pair.push_str(prev_grapheme);
            pair.push_str(grapheme);
            let pair_width = ctx.measure_text(&pair, font_descriptor.as_str())?;
            let raw_adjustment = pair_width - prev_width - measured_glyph_width;
            if raw_adjustment.is_finite() {
                raw_adjustment * style.text_scale_x.max(0.01)
            } else {
                0.0
            }
        } else {
            0.0
        };
        let advance = compute_grapheme_advance(GraphemeAdvanceContext {
            paragraph_style,
            grapheme,
            script,
            measured_glyph_width,
            measured_width_is_oracle: false,
            text_scale_x: style.text_scale_x.max(0.01),
            extra_spacing,
            kerning_adjustment,
            is_line_start,
            prev_prev_grapheme: state.prev_prev_grapheme.as_deref(),
            prev_grapheme: state.prev_grapheme.as_deref(),
            next_grapheme,
            prev_glyph_width: state.prev_adjusted_glyph_width,
            snap_cell_width,
            font_descriptor: None,
            prev_auto_space: None,
            run_letter_spacing: 0.0,
        });
        Ok(OwnedTextAdvanceStep {
            rel_offset: 0,
            grapheme: grapheme.into(),
            script,
            font_descriptor: font_descriptor.into(),
            font_descriptor_hash,
            measured_glyph_width,
            advance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn legacy_walk_text_advance_steps<C, F>(
        ctx: &C,
        text: &str,
        style: &ResolvedRunStyle,
        paragraph_style: Option<&ParagraphStyle>,
        initial_state: &TextAdvanceState,
        next_grapheme_after_text: Option<&str>,
        snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
        mut visitor: F,
    ) -> Result<TextAdvanceState, C::Error>
    where
        C: LineWrapContext,
        F: FnMut(TextAdvanceStep<'_>) -> Result<(), C::Error>,
    {
        let graphemes: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
        let snap_cell_width = if let Some(paragraph_style) = paragraph_style {
            resolve_snap_to_chars_cell_width(
                ctx,
                paragraph_style,
                &style.font_descriptor_east_asia,
                style.font_descriptor_east_asia_hash,
                style.text_scale_x.max(0.01),
                snap_to_chars_cell_width_cache,
            )?
        } else {
            None
        };
        let mut state = initial_state.clone();

        for (index, (rel_offset, grapheme)) in graphemes.iter().enumerate() {
            let next_grapheme = graphemes
                .get(index + 1)
                .map(|(_, value)| *value)
                .or_else(|| {
                    (index + 1 == graphemes.len())
                        .then_some(next_grapheme_after_text)
                        .flatten()
                });
            let mut computed = legacy_compute_text_advance_step(
                ctx,
                grapheme,
                style,
                paragraph_style,
                &state,
                next_grapheme,
                index == 0 && initial_state.prev_grapheme.is_none(),
                snap_cell_width,
            )?;
            computed.rel_offset = *rel_offset;
            visitor(TextAdvanceStep {
                rel_offset: *rel_offset,
                grapheme,
                script: computed.script,
                font_descriptor: &computed.font_descriptor,
                font_descriptor_hash: computed.font_descriptor_hash,
                measured_glyph_width: computed.measured_glyph_width,
                advance: computed.advance,
            })?;
            state.prev_prev_grapheme = state.prev_grapheme.take();
            state.prev_grapheme = Some(CompactString::new(grapheme));
            state.prev_adjusted_glyph_width = Some(computed.advance.adjusted_glyph_width);
            state.prev_raw_glyph_width = Some(computed.measured_glyph_width);
            state.prev_font_descriptor_hash = Some(
                style
                    .descriptor_and_hash_for_grapheme(grapheme, computed.script)
                    .1,
            );
        }
        Ok(state)
    }

    fn legacy_collect_text_advance_steps<C>(
        ctx: &C,
        text: &str,
        style: &ResolvedRunStyle,
        paragraph_style: Option<&ParagraphStyle>,
        initial_state: &TextAdvanceState,
        next_grapheme_after_text: Option<&str>,
        snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
    ) -> Result<(Vec<OwnedTextAdvanceStep>, TextAdvanceState), C::Error>
    where
        C: LineWrapContext,
    {
        let mut steps = Vec::new();
        let end_state = legacy_walk_text_advance_steps(
            ctx,
            text,
            style,
            paragraph_style,
            initial_state,
            next_grapheme_after_text,
            snap_to_chars_cell_width_cache,
            |step| {
                steps.push(OwnedTextAdvanceStep {
                    rel_offset: step.rel_offset,
                    grapheme: Rc::from(step.grapheme),
                    script: step.script,
                    font_descriptor: Rc::from(step.font_descriptor),
                    font_descriptor_hash: step.font_descriptor_hash,
                    measured_glyph_width: step.measured_glyph_width,
                    advance: step.advance,
                });
                Ok(())
            },
        )?;
        Ok((steps, end_state))
    }

    #[test]
    fn yu_mincho_pa_walk_uses_requested_kerning_without_host_pair_measurement() {
        use crate::line_metrics::WordLayoutPlatform;
        let ctx = CountingKerningContext::new(true);
        let descriptor = "4.50pt 'Yu Mincho', serif";
        let style = ResolvedRunStyle::new(
            descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
        );
        let paragraph = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut kern = 0.0;
        walk_text_advance_steps(
            &ctx,
            "Pa",
            &style,
            Some(&paragraph),
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |step| {
                kern += step.advance.kerning_adjustment;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(kern, -92.0 * 6.0 / 2048.0);
        assert_eq!(*ctx.pair_measurements.borrow(), 0);
    }

    #[test]
    fn fixed_cjk_pair_gate_accepts_standin_subpixel_rounding() {
        use crate::line_metrics::WordLayoutPlatform;
        for (descriptor, delta, expected_kern, expected_pairs) in [
            ("4.50pt 'Yu Mincho', serif", -1.0 / 16384.0, 0.0, 0),
            ("4.50pt 'Yu Mincho', serif", 1.0 / 16384.0, 0.0, 0),
            ("4.50pt 'Yu Mincho', serif", -1.0 / 64.0, -1.0, 1),
            ("4.50pt 'MS PMincho', serif", -1.0 / 16384.0, -1.0, 1),
            ("4.50pt 'Test', serif", -1.0 / 16384.0, -1.0, 1),
        ] {
            let mut ctx = CountingKerningContext::new(true);
            ctx.glyph_width += delta;
            ctx.pair_adjustment = -1.0;
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
            );
            let paragraph = ParagraphStyle {
                word_platform: WordLayoutPlatform::Mac,
                ..Default::default()
            };
            let mut cache = FxHashMap::default();
            let mut adjustment = 0.0;
            walk_text_advance_steps(
                &ctx,
                "す。",
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    adjustment += step.advance.kerning_adjustment;
                    Ok(())
                },
            )
            .unwrap();
            assert!(
                (adjustment - expected_kern).abs() < 1e-9,
                "{descriptor} delta={delta}: {adjustment}"
            );
            assert_eq!(*ctx.pair_measurements.borrow(), expected_pairs);
        }
    }

    #[test]
    fn mac_fixed_punctuation_pairs_do_not_depend_on_standin_kerning() {
        use crate::line_metrics::WordLayoutPlatform;
        let kern = |text: &str, control: Option<&str>, kerning: bool, host_adjustment: f64| {
            let mut ctx = CountingKerningContext::new(kerning);
            ctx.pair_adjustment = host_adjustment;
            let descriptor = "4.50pt 'MS Mincho', serif";
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, kerning, None,
            );
            let paragraph = ParagraphStyle {
                word_platform: WordLayoutPlatform::Mac,
                east_asian_rules: EastAsianLineBreakingRules {
                    character_spacing_control: control.map(str::to_string),
                    ..EastAsianLineBreakingRules::default()
                },
                ..ParagraphStyle::default()
            };
            let mut cache = FxHashMap::default();
            let mut kern = 0.0;
            walk_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    kern += step.advance.kerning_adjustment;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(*ctx.pair_measurements.borrow(), 0, "{text}");
            kern
        };
        // A kerned pair of punctuation shares the half-em blank on its
        // touching sides, whatever the stand-in face's pair kerning says;
        // an unkerned pair keeps full cells (Word for Mac 16.114 probe
        // pairs-kerned: 5.25pt apart with `w:kern`, 10.5pt without).
        for text in [
            "、「", "）」", "）（", "。）", "）。", "）、", "。「", "（「", "「（",
        ] {
            for host_adjustment in [0.0, -1.0, -3.0] {
                for kerning in [true, false] {
                    assert_eq!(
                        kern(text, Some("compressPunctuation"), kerning, host_adjustment),
                        if kerning { -3.0 } else { 0.0 },
                        "{text} kerning={kerning}"
                    );
                }
            }
        }
        // Not between an opening bracket and a character with no blank.
        for text in ["「あ", "あ「"] {
            assert_eq!(
                kern(text, Some("compressPunctuation"), true, -1.0),
                0.0,
                "{text}"
            );
        }
        // So do doNotCompress documents (Word for Mac 16.114 probe
        // pairs-kerned and its doNotCompress twin are identical).
        for control in [Some("doNotCompress"), None] {
            assert_eq!(kern("、「", control, true, -1.0), -3.0, "{control:?}");
        }
    }

    #[test]
    fn mac_kerned_adjacent_punctuation_shares_one_half_em_in_every_pairing() {
        // Word for Mac 16.114 sets each of these pairs 5.25pt apart at 10.5pt
        // MS Mincho when the run is kerned, whatever characterSpacingControl
        // says, and a full 10.5pt apart when it is not: closing→closing
        // (`。）`), closing→opening (`。「`) and opening→opening (`（「`) alike.
        use crate::line_metrics::WordLayoutPlatform;
        let descriptor = "4.50pt 'MS Mincho', serif";
        let paragraph = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        for kerning in [true, false] {
            for pair in [
                "。）", "）。", "）、", "、「", "」（", "（「", "。「", "）」",
            ] {
                let ctx = CountingKerningContext::new(kerning);
                let style = ResolvedRunStyle::new(
                    descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, kerning, None,
                );
                let mut cache = FxHashMap::default();
                let mut kern = 0.0;
                walk_text_advance_steps(
                    &ctx,
                    pair,
                    &style,
                    Some(&paragraph),
                    &TextAdvanceState::default(),
                    None,
                    &mut cache,
                    |step| {
                        kern += step.advance.kerning_adjustment;
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    kern,
                    if kerning { -3.0 } else { 0.0 },
                    "{pair} kern={kerning}"
                );
            }
        }
    }

    #[test]
    fn mac_fixed_fullwidth_cjk_does_not_take_host_pair_kerning() {
        use crate::line_metrics::WordLayoutPlatform;
        let mut ctx = CountingKerningContext::new(true);
        ctx.pair_adjustment = -1.0;
        for (descriptor, text, platform, expected, pairs) in [
            (
                "4.50pt 'MS Mincho', serif",
                "す。",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                "4.50pt 'Yu Mincho', serif",
                "て、",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                // Fixed-pitch faces have no kerning on either platform
                // (jlsi/jade#469): the stand-in's pair kerning is not Word's.
                "4.50pt 'MS Mincho', serif",
                "す。",
                WordLayoutPlatform::Windows,
                0.0,
                0,
            ),
            (
                "4.50pt 'Test', serif",
                "す。",
                WordLayoutPlatform::Mac,
                -1.0,
                1,
            ),
            (
                "4.50pt 'MS PMincho', serif",
                "す。",
                WordLayoutPlatform::Mac,
                -1.0,
                1,
            ),
            (
                "4.50pt 'MS Mincho', serif",
                "す\u{3099}。",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                "4.50pt 'MS Mincho', serif",
                "To",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                // A kerned punctuation pair shares its half-em blank without
                // compressPunctuation too (see
                // mac_fixed_punctuation_pairs_do_not_depend_on_standin_kerning).
                "4.50pt 'MS Mincho', serif",
                "、「",
                WordLayoutPlatform::Mac,
                -3.0,
                0,
            ),
            // The cell, not a narrower stand-in glyph, sets the advance.
            (
                "10.50pt 'MS Mincho', serif",
                "す。",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                "4.50pt 'MS Mincho', serif",
                "ｶ､",
                WordLayoutPlatform::Mac,
                0.0,
                0,
            ),
            (
                "4.50pt 'MS Mincho', serif",
                "אב",
                WordLayoutPlatform::Mac,
                -1.0,
                1,
            ),
        ] {
            *ctx.pair_measurements.borrow_mut() = 0;
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
            );
            let paragraph = ParagraphStyle {
                word_platform: platform,
                ..ParagraphStyle::default()
            };
            let mut cache = FxHashMap::default();
            let mut kern = 0.0;
            walk_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    kern += step.advance.kerning_adjustment;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(kern, expected, "{descriptor} {text} {platform:?}");
            assert_eq!(*ctx.pair_measurements.borrow(), pairs);
        }
    }

    #[test]
    fn fixed_cjk_metrics_follow_public_descriptor_replacement() {
        let descriptor = "10.50pt 'MS Mincho', serif";
        let mut style = ResolvedRunStyle::new(
            descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
        );
        assert_eq!(style.fixed_east_asia_em_px(), Some(14.0));
        style.font_descriptor_east_asia = Rc::from("10.50pt 'MS PMincho', serif");
        assert_eq!(style.fixed_east_asia_em_px(), None);
        style.font_descriptor_east_asia = Rc::from("12.00pt 'Yu Mincho', serif");
        assert_eq!(style.fixed_east_asia_em_px(), Some(16.0));
        eprintln!(
            "ResolvedRunStyle_bytes={}",
            std::mem::size_of::<ResolvedRunStyle>()
        );
    }

    #[test]
    fn yu_mincho_wo_kerning_uses_word_font_metrics_across_host_substitutes() {
        use crate::line_metrics::WordLayoutPlatform;
        // Word's 2col PDF places W/o 9.690pt apart at 10.5pt. Its W
        // hmtx advance is 2013/2048 em; the pair contracts by 123/2048 em.
        let descriptor = "10.50pt 'Yu Mincho', serif";
        for (platform, enabled, expected_pair_measurements) in [
            (WordLayoutPlatform::Mac, true, 0),
            (WordLayoutPlatform::Mac, false, 0),
            (WordLayoutPlatform::Windows, true, 1),
        ] {
            for host_adjustment in [0.0, -0.5, -2.0] {
                let mut ctx = CountingKerningContext::new(enabled);
                ctx.pair_adjustment = host_adjustment;
                let style = ResolvedRunStyle::new(
                    descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, enabled, None,
                );
                let paragraph = ParagraphStyle {
                    word_platform: platform,
                    ..ParagraphStyle::default()
                };
                let mut cache = FxHashMap::default();
                let mut steps = Vec::new();
                walk_text_advance_steps(
                    &ctx,
                    "Wo",
                    &style,
                    Some(&paragraph),
                    &TextAdvanceState::default(),
                    None,
                    &mut cache,
                    |step| {
                        steps.push(step.advance.kerning_adjustment);
                        Ok(())
                    },
                )
                .unwrap();
                let expected = if !enabled {
                    0.0
                } else if platform == WordLayoutPlatform::Mac {
                    -14.0 * 123.0 / 2048.0
                } else {
                    host_adjustment
                };
                assert!(
                    (steps[1] - expected).abs() < 1e-9,
                    "{platform:?}: host pair {host_adjustment}, actual {} expected {expected}",
                    steps[1]
                );
                assert_eq!(*ctx.pair_measurements.borrow(), expected_pair_measurements);
            }
        }
    }

    #[test]
    fn yu_mincho_observed_unadjusted_pairs_ignore_substitute_kerning() {
        use crate::line_metrics::WordLayoutPlatform;
        let descriptor = "10.50pt 'Yu Mincho', serif";
        // The committed 2col PDF repeats these origins on all 36 rows.
        // Non-space pitches agree with hmtx, while the space expansion is
        // applied by justification rather than by font-pair kerning.
        for pair in [
            "He", "el", "ll", "lo", "o ", " W", "or", "rl", "ld", "d ", " H",
        ] {
            for host_adjustment in [-0.5, -2.0, 0.0] {
                let mut ctx = CountingKerningContext::new(true);
                ctx.pair_adjustment = host_adjustment;
                let style = ResolvedRunStyle::new(
                    descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
                );
                let paragraph = ParagraphStyle {
                    word_platform: WordLayoutPlatform::Mac,
                    ..ParagraphStyle::default()
                };
                let mut cache = FxHashMap::default();
                let mut adjustment = None;
                walk_text_advance_steps(
                    &ctx,
                    pair,
                    &style,
                    Some(&paragraph),
                    &TextAdvanceState::default(),
                    None,
                    &mut cache,
                    |step| {
                        adjustment = Some(step.advance.kerning_adjustment);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    adjustment,
                    Some(0.0),
                    "pair {pair:?}, host {host_adjustment}"
                );
                assert_eq!(*ctx.pair_measurements.borrow(), 0, "pair {pair:?}");
            }
        }
    }

    #[test]
    fn mac_synthetic_bold_leaves_the_ideographic_space_unwidened() {
        // scroll-offset-repro's bold 14pt `第６章　賃金`: Word for Mac widens
        // each emboldened glyph by 1/50 em but sets the ideographic space at
        // its plain 14pt.
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let ctx = CountingKerningContext::new(false);
        let paragraph = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let glyphs = |descriptor: &str| {
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, false, None,
            );
            let mut cache = FxHashMap::default();
            let mut glyphs = Vec::new();
            walk_text_advance_steps(
                &ctx,
                "A\u{3000}A",
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    glyphs.push((step.grapheme.to_string(), step.advance.glyph_width));
                    Ok(())
                },
            )
            .unwrap();
            glyphs
        };
        let bold = glyphs("bold 10.50pt 'MS Mincho', serif");
        let regular = glyphs("10.50pt 'MS Mincho', serif");
        set_word_layout_platform(WordLayoutPlatform::Windows);
        for ((grapheme, bold), (_, regular)) in bold.into_iter().zip(regular) {
            let extra = if grapheme.trim().is_empty() {
                0.0
            } else {
                0.28
            };
            assert!(
                (bold - regular - extra).abs() < 1e-9,
                "{grapheme:?}: {regular} -> {bold}"
            );
        }
    }

    #[test]
    fn mac_bold_ascii_walk_keeps_synthetic_advance_out_of_pair_measurement() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let ctx = CountingKerningContext::new(true);
        let descriptor = "bold 10.50pt 'MS Mincho', serif";
        let style = ResolvedRunStyle::new(
            descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
        );
        let paragraph = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut advances = Vec::new();
        walk_text_advance_steps(
            &ctx,
            "AAA",
            &style,
            Some(&paragraph),
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |step| {
                advances.push((
                    step.advance.glyph_width,
                    step.advance.kerning_adjustment,
                    step.advance.total_advance,
                ));
                Ok(())
            },
        )
        .unwrap();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        for (glyph, kern, advance) in advances {
            assert!((glyph - 7.28).abs() < 1e-9);
            assert!(
                (kern - 0.0).abs() < 1e-9,
                "synthetic emboldening is not negative pair kerning: {kern}"
            );
            assert!((advance - 7.28).abs() < 1e-9);
        }
        // A fixed-pitch face takes no host pair kerning at all.
        assert_eq!(*ctx.pair_measurements.borrow(), 0);
    }

    #[test]
    fn mac_ascii_walk_resolves_fixed_font_metrics_once_per_run() {
        let ctx = CountingKerningContext::new(false);
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        for (descriptor, expected) in [
            ("10.50pt 'MS Mincho', serif", 7.0),
            ("10.50pt 'MS Gothic', serif", 7.0),
            ("10.50pt 'Yu Mincho', serif", 14.0 * 1448.0 / 2048.0),
            ("11.00pt 'Times New Roman', serif", 6.0),
        ] {
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, false, None,
            );
            crate::line_metrics::WORD_FIXED_ASCII_DESCRIPTOR_LOOKUPS.with(|count| count.set(0));
            let mut cache = FxHashMap::default();
            walk_text_advance_steps(
                &ctx,
                &"A".repeat(1_000),
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    assert_eq!(step.advance.glyph_width, expected);
                    Ok(())
                },
            )
            .expect("walked ASCII run");
            assert!(
                crate::line_metrics::WORD_FIXED_ASCII_DESCRIPTOR_LOOKUPS.with(std::cell::Cell::get)
                    <= 1,
                "a resolved run must not look its font metrics up per glyph"
            );
        }
    }

    #[test]
    fn walk_text_advance_steps_reuses_kerning_pair_measurements() {
        let ctx = CountingKerningContext::new(true);
        let style = ctx.style.clone();
        let mut cache = FxHashMap::default();

        let state = walk_text_advance_steps(
            &ctx,
            "AVAVAVAV",
            &style,
            None,
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |_| Ok(()),
        )
        .expect("walked steps");

        assert_eq!(state.prev_grapheme.as_deref(), Some("V"));
        assert_eq!(*ctx.pair_measurements.borrow(), 2);
    }

    #[test]
    fn wrap_text_populates_inline_cluster_layout_metadata() {
        let ctx = KerningAwareContext::new(true);
        let paragraph = ParagraphStyle::default();
        let text = "AVA";
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        };

        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 400.0)
            .expect("wrapped lines");
        let first_line = lines.first().expect("first line");

        for cluster in &first_line.clusters {
            if cluster.kind == InlineClusterKind::Grapheme {
                let layout = cluster.layout.as_ref().expect("cluster layout");
                assert!(layout.glyph_box_width >= 0.0);
                assert!(layout.measured_glyph_width >= 0.0);
                assert_eq!(layout.font_ascent, Some(8.0));
                assert_eq!(layout.font_descent, Some(2.0));
            }
        }
    }

    #[test]
    fn inline_note_separators_support_multiple_utf8_boundaries() {
        let ctx = TestContext::new(3.0);
        let text = "界1 A ii B C";
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            inline_note_separator_offsets: vec![4, 9],
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 400.0).unwrap();
        let widths: Vec<_> = lines[0]
            .clusters
            .iter()
            .filter(|c| c.text.as_str() == " ")
            .map(|c| c.width)
            .collect();
        assert_eq!(widths.len(), 4);
        assert_eq!(widths, vec![8.0, 3.0, 8.0, 3.0]);
        assert_eq!(
            lines[0]
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn inline_note_separator_respects_slice_offsets_platform_and_nonspaces() {
        let ctx = TestContext::new(3.0);
        for (platform, offsets, expected) in [
            (crate::line_metrics::WordLayoutPlatform::Mac, vec![4], 8.0),
            (
                crate::line_metrics::WordLayoutPlatform::Windows,
                vec![4],
                3.0,
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                vec![3, 5],
                3.0,
            ),
        ] {
            let paragraph = ParagraphStyle {
                word_platform: platform,
                inline_note_separator_offsets: offsets,
                ..Default::default()
            };
            let lines =
                wrap_text_into_lines(&ctx, "1 A", &[make_run("1 A")], 0.0, &paragraph, 3, 400.0)
                    .unwrap();
            let widths: Vec<_> = lines[0].clusters.iter().map(|c| c.width).collect();
            assert_eq!(widths, vec![3.0, expected, 3.0]);
        }
    }

    #[test]
    fn automatic_note_separator_uses_half_em_without_changing_other_spaces_or_offsets() {
        let mut ctx = TestContext::new(3.0);
        ctx.style.enable_pair_kerning = true;
        for marker in ["1", "iii", "※"] {
            let text = format!("{marker} A B");
            let paragraph = ParagraphStyle {
                word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
                automatic_note_separator_offset: Some(marker.len()),
                ..ParagraphStyle::default()
            };
            let runs = [RunStyle {
                start: 0,
                end: text.len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            }];
            let lines =
                wrap_text_into_lines(&ctx, &text, &runs, 0.0, &paragraph, 0, 100.0).unwrap();
            let first_space = lines[0]
                .clusters
                .iter()
                .find(|cluster| cluster.start == marker.len())
                .unwrap();
            assert_eq!(first_space.layout.as_ref().unwrap().glyph_box_width, 8.0);
            let last_space = lines[0]
                .clusters
                .iter()
                .find(|cluster| cluster.start == marker.len() + 2)
                .unwrap();
            assert_eq!(last_space.layout.as_ref().unwrap().glyph_box_width, 3.0);
            assert_eq!(
                lines[0]
                    .segments
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<String>(),
                text
            );
            let mut advances = Vec::new();
            walk_text_advance_steps_at_offset(
                &ctx,
                " A B",
                &ctx.style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                Some(marker.len()),
                &mut FxHashMap::default(),
                |step| {
                    assert_eq!(step.measured_glyph_width, 3.0);
                    advances.push(step.advance.total_advance);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(advances, [8.0, 3.0, 3.0, 3.0]);
        }
    }

    #[test]
    fn automatic_note_separator_keeps_oracle_windows_and_unmarked_space_advances() {
        let ctx = TestContext::new(3.0);
        let runs = [RunStyle {
            start: 0,
            end: 3,
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];
        for (platform, offset, oracle, expected) in [
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                None,
                None,
                3.0,
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Windows,
                Some(1),
                None,
                3.0,
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                Some(1),
                Some(4.0),
                4.0,
            ),
        ] {
            let paragraph = ParagraphStyle {
                word_platform: platform,
                automatic_note_separator_offset: offset,
                oracle_advance_px_by_text_offset: oracle
                    .map(|width| Rc::new(FxHashMap::from_iter([(1, width)]))),
                ..ParagraphStyle::default()
            };
            let lines =
                wrap_text_into_lines(&ctx, "1 A", &runs, 0.0, &paragraph, 0, 100.0).unwrap();
            assert_eq!(
                lines[0].clusters[1]
                    .layout
                    .as_ref()
                    .unwrap()
                    .glyph_box_width,
                expected
            );
        }
    }

    #[test]
    fn consecutive_manual_breaks_use_their_run_font_box_in_a_tall_paragraph() {
        use crate::line_metrics::{
            set_word_layout_platform, WordLayoutPlatform, WordProportionalLineSizing,
        };
        use jade_docx_reader::document::RunFonts;
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            word_proportional_line_sizing: WordProportionalLineSizing::new(None, false, true),
            ..ParagraphStyle::default()
        };
        let properties = RunProperties {
            font_size: Some(12.0),
            fonts: Some(RunFonts {
                ascii: Some("Times New Roman".into()),
                east_asia: Some("Arial Unicode MS".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        let run = RunStyle {
            start: 0,
            end: 2,
            properties: Some(Rc::new(properties.clone())),
            revision: None,
            hyperlink: None,
            inline_object: None,
        };
        let lines = wrap_text_into_lines(
            &TestContext::new(10.0),
            "\n\n",
            &[run],
            0.0,
            &paragraph,
            0,
            100.0,
        )
        .unwrap();
        let tall = points_to_px(20.90625);
        let latin = points_to_px(14_130.0 / 1024.0);
        for line in &lines[..2] {
            assert!(line.clusters.is_empty());
            assert!((line.line_height_px(&paragraph, tall) - latin).abs() < 1e-9);
        }
        let mut unknown = lines[0].clone();
        let mut properties = properties.clone();
        properties.fonts.as_mut().unwrap().ascii = Some("Unregistered face".into());
        unknown.break_properties = Some(Rc::new(properties));
        assert_eq!(unknown.line_height_px(&paragraph, tall), tall);
        unknown.break_properties = None;
        assert_eq!(unknown.line_height_px(&paragraph, tall), tall);
        unknown.break_properties = Some(Rc::new(RunProperties::default()));
        assert_eq!(unknown.line_height_px(&paragraph, tall), tall);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(lines[0].line_height_px(&paragraph, tall), tall);
    }

    #[test]
    fn wrap_text_keeps_inline_object_with_adjacent_text_when_it_fits() {
        let ctx = TestContext::new(10.0);
        let paragraph = ParagraphStyle::default();
        let text = "Hello\u{fffc}World";
        let image_start = "Hello".len();
        let image_end = image_start + "\u{fffc}".len();
        let runs = [
            RunStyle {
                start: 0,
                end: image_start,
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: image_start,
                end: image_end,
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: Some(InlineObject {
                    width: 100.0,
                    height: 80.0,
                }),
            },
            RunStyle {
                start: image_end,
                end: text.len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
        ];

        let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 260.0)
            .expect("wrap inline object");
        let infos = collect_line_infos(&lines);

        assert_eq!(infos.len(), 1, "inline image should not force its own line");
        assert_eq!(infos[0].text, "HelloWorld");
        assert_eq!(
            lines[0]
                .clusters
                .iter()
                .filter(|cluster| cluster.kind == InlineClusterKind::InlineObject)
                .count(),
            1
        );
    }

    #[test]
    fn numbering_ascent_covers_both_segments_of_first_float_band_only() {
        use crate::line_metrics::{
            set_word_layout_platform, WordGridLineSizing, WordLayoutPlatform,
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let grid = jade_docx_reader::DocGrid {
            grid_type: Some("lines".into()),
            line_pitch: Some(18.0),
            char_space: None,
        };
        let mut style = ParagraphStyle {
            word_grid_line_sizing: WordGridLineSizing::new(None, true, Some(&grid), None, 10.5),
            word_platform: WordLayoutPlatform::Mac,
            ..Default::default()
        };
        let text = "あ\nい\nう";
        let runs = [RunStyle {
            start: 0,
            end: text.len(),
            properties: Some(Rc::new(RunProperties {
                font_family: Some("MS Mincho".into()),
                font_size: Some(10.5),
                ..Default::default()
            })),
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];
        let mut lines =
            wrap_text_into_lines(&TestContext::new(10.0), text, &runs, 0.0, &style, 0, 100.0)
                .unwrap();
        assert_eq!(lines.len(), 3);
        for cluster in lines.iter_mut().flat_map(|line| &mut line.clusters) {
            let metrics = cluster.layout.as_mut().unwrap();
            metrics.font_ascent = Some(points_to_px(10.5 * 258.0 / 256.0));
            metrics.font_descent = Some(points_to_px(10.5 * 74.0 / 256.0));
        }
        assert!(lines[0].font_box_px().is_some());
        style.float_line_segments = (0..3)
            .map(|index| FloatLineSegment {
                start_x: 0.0,
                width: 100.0,
                skipped_lines_before: 0,
                exact_clearance_height: 0.0,
                shares_band_with_previous: index == 1,
            })
            .collect();
        let heights = WrappedLine::line_heights_px_with_numbering_ascent(
            &lines,
            &style,
            24.0,
            None,
            Some(points_to_px(40.0)),
        )
        .unwrap();
        assert_eq!(heights, vec![72.0, 72.0, 24.0]);
        assert!(
            WrappedLine::line_heights_px_with_numbering_ascent(
                &lines,
                &ParagraphStyle::default(),
                4.0,
                None,
                Some(0.0),
            )
            .is_none(),
            "a label below the body must not change its existing fixed-height plan"
        );
        assert!(
            WrappedLine::line_heights_px_with_numbering_ascent(
                &lines,
                &ParagraphStyle::default(),
                16.0,
                None,
                Some(points_to_px(40.0)),
            )
            .is_none(),
            "a tall label must retain the paragraph's resolved constant height"
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn inline_object_line_occupies_whole_word_grid_steps() {
        use crate::line_metrics::{
            set_word_layout_platform, WordGridLineSizing, WordLayoutPlatform,
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let grid = jade_docx_reader::DocGrid {
            grid_type: Some("lines".into()),
            line_pitch: Some(18.0),
            char_space: None,
        };
        let paragraph = ParagraphStyle {
            word_grid_line_sizing: WordGridLineSizing::new(None, true, Some(&grid), None, 10.5),
            ..ParagraphStyle::default()
        };
        let text = "\u{fffc}";
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: Some(InlineObject {
                width: points_to_px(124.1),
                height: points_to_px(25.8),
            }),
        };
        let lines = wrap_text_into_lines(
            &TestContext::new(10.0),
            text,
            &[run],
            0.0,
            &paragraph,
            0,
            500.0,
        )
        .unwrap();
        let height = lines[0].line_height_px(&paragraph, points_to_px(18.0));
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(
            (height - points_to_px(36.0)).abs() < 1e-6,
            "height={height}"
        );
        assert_eq!(
            lines[0].font_box_px(),
            None,
            "non-grid estimate is unchanged"
        );
        assert_eq!(
            lines[0].line_height_px(&ParagraphStyle::default(), 24.0),
            24.0
        );

        // An object just shorter than two steps still needs three when a
        // neighboring text run descends below its baseline.
        let mut mixed = lines[0].clone();
        mixed.clusters[0].inline_object.as_mut().unwrap().height = points_to_px(35.6);
        let mut text_cluster = InlineCluster::grapheme(3, 4, 10.0, "A", false, false, false);
        text_cluster.layout = Some(InlineClusterLayout {
            character_border_edges: 0,
            script: ScriptCategory::Latin,
            font_slot: ResolvedFontSlot::LatinAscii,
            leading_advance: 0.0,
            glyph_box_width: 10.0,
            measured_glyph_width: 10.0,
            adjusted_glyph_width: 10.0,
            font_descriptor_hash: 0,
            font_ascent: Some(points_to_px(10.5)),
            font_descent: Some(points_to_px(3.235)),
        });
        mixed.clusters.push(text_cluster);
        assert!((mixed.line_height_px(&paragraph, 24.0) - points_to_px(54.0)).abs() < 1e-5);
        mixed.clusters[1].layout = None;
        assert_eq!(
            mixed.line_height_px(&paragraph, 24.0),
            24.0,
            "unknown text metrics keep the fallback"
        );
    }

    #[test]
    fn single_auto_line_uses_resolved_content_when_the_estimate_is_a_blank_mark() {
        use crate::line_metrics::{
            set_word_layout_platform, WordLayoutPlatform, WordProportionalLineSizing,
        };
        use jade_docx_reader::document::{RunFonts, Spacing};
        struct KnownFaceContext;
        impl LineWrapContext for KnownFaceContext {
            type Error = ();
            fn resolve_run_style(
                &self,
                props: Option<&RunProperties>,
            ) -> Result<Rc<ResolvedRunStyle>, ()> {
                Ok(crate::font_resolver::resolve_run_style_for_wrap(
                    props, None,
                ))
            }
            fn measure_text(&self, text: &str, _: &str) -> Result<f64, ()> {
                Ok(text.chars().count() as f64 * 10.0)
            }
            fn measure_font_vertical_metrics(
                &self,
                descriptor: &str,
            ) -> Result<Option<(f64, f64)>, ()> {
                Ok(crate::line_metrics::word_font_vertical_metrics_px(
                    descriptor,
                ))
            }
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let spacing = Spacing {
            line: Some(278.0 / 240.0),
            line_rule: Some("auto".into()),
            ..Spacing::default()
        };
        let style = ParagraphStyle {
            word_proportional_line_sizing: WordProportionalLineSizing::new(
                Some(&spacing),
                false,
                true,
            ),
            ..ParagraphStyle::default()
        };
        let text = "資料文献が指定されていません。";
        let runs = [RunStyle {
            start: 0,
            end: text.len(),
            properties: Some(Rc::new(RunProperties {
                font_size: Some(12.0),
                fonts: Some(RunFonts {
                    ascii: Some("Aptos".into()),
                    east_asia: Some("Yu Mincho".into()),
                    ..RunFonts::default()
                }),
                ..RunProperties::default()
            })),
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];
        let lines =
            wrap_text_into_lines(&KnownFaceContext, text, &runs, 0.0, &style, 0, 800.0).unwrap();
        let expected = lines[0].line_height_px(&style, 1.0);
        let fallback = points_to_px(12.0 * 2500.0 / 2048.0 * 278.0 / 240.0);
        let heights = WrappedLine::line_heights_px(&lines, &style, fallback);
        let same = WrappedLine::line_heights_px(&lines, &style, expected);
        let unknown = WrappedLine::line_heights_px(&lines, &ParagraphStyle::default(), fallback);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(lines.len(), 1);
        assert!(expected > fallback + 3.0);
        assert_eq!(heights, Some(vec![expected]));
        assert!(
            same.is_none(),
            "constant scalar height keeps its no-plan path"
        );
        assert!(
            unknown.is_none(),
            "unconfigured sizing retains its fallback"
        );
    }

    #[test]
    fn object_only_paragraph_end_includes_its_mark_font_box() {
        use crate::line_metrics::{
            set_word_layout_platform, WordGridLineSizing, WordLayoutPlatform,
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let object = InlineCluster::inline_object(
            0,
            3,
            InlineObject {
                width: 100.0,
                height: 118.8,
            },
        );
        let mut line = WrappedLine {
            segments: Vec::new(),
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: vec![object],
        };
        let mark = crate::line_metrics::word_font_vertical_metrics_px("10.50pt Arial").unwrap();
        assert_eq!(
            line.content_box_px_with_paragraph_mark(Some(mark)),
            Some((118.8, mark.1))
        );
        let heights = WrappedLine::line_heights_px_with_paragraph_mark(
            std::slice::from_ref(&line),
            &ParagraphStyle::default(),
            20.0,
            Some(mark),
        )
        .unwrap();
        assert_eq!(heights, vec![118.8 + mark.1]);
        assert_eq!(
            line.content_box_px_with_paragraph_mark(None),
            Some((118.8, 0.0))
        );
        assert_eq!(
            line.clusters[0].inline_object_bounds_px(),
            Some((-118.8, 0.0))
        );
        line.break_kind = LineBreakKind::Soft;
        assert_eq!(
            line.content_box_px_with_paragraph_mark(Some(mark)),
            Some((118.8, 0.0))
        );
        line.break_kind = LineBreakKind::ParagraphEnd;
        line.clusters[0].inline_object.as_mut().unwrap().height = points_to_px(35.6);
        let grid = jade_docx_reader::DocGrid {
            grid_type: Some("lines".into()),
            line_pitch: Some(18.0),
            char_space: None,
        };
        let paragraph = ParagraphStyle {
            word_grid_line_sizing: WordGridLineSizing::new(None, true, Some(&grid), None, 10.5),
            ..ParagraphStyle::default()
        };
        assert_eq!(
            WrappedLine::line_heights_px_with_paragraph_mark(
                std::slice::from_ref(&line),
                &paragraph,
                24.0,
                Some(mark),
            ),
            Some(vec![72.0])
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            line.content_box_px_with_paragraph_mark(Some(mark)),
            Some((points_to_px(35.6), 0.0))
        );
    }

    #[test]
    fn inline_object_keeps_authored_lowered_baseline_in_shared_layout() {
        let text = "\u{fffc}";
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: Some(Rc::new(RunProperties {
                text_position: Some(-9.0),
                ..RunProperties::default()
            })),
            revision: None,
            hyperlink: None,
            inline_object: Some(InlineObject {
                width: points_to_px(124.1),
                height: points_to_px(25.8),
            }),
        };
        let lines = wrap_text_into_lines(
            &TestContext::new(10.0),
            text,
            &[run],
            0.0,
            &ParagraphStyle::default(),
            0,
            500.0,
        )
        .unwrap();
        let layout = lines[0].clusters[0]
            .layout
            .as_ref()
            .expect("positioned object bounds");
        assert!((layout.font_ascent.unwrap() - points_to_px(16.8)).abs() < 1e-5);
        assert!((layout.font_descent.unwrap() - points_to_px(9.0)).abs() < 1e-6);
        assert_eq!(
            wrapped_line_max_font_ascent(&lines[0]),
            None,
            "object bounds are not font metrics"
        );
    }

    #[test]
    fn inline_object_position_preserves_bounds_above_and_below_baseline() {
        for (position, top, bottom) in [(12.0, -22.0, -12.0), (-12.0, 2.0, 12.0)] {
            let cluster = InlineCluster::inline_object(
                0,
                3,
                InlineObject {
                    width: 20.0,
                    height: points_to_px(10.0),
                },
            )
            .with_object_position(Some(position));
            let bounds = cluster.inline_object_bounds_px().unwrap();
            assert!((bounds.0 - points_to_px(top)).abs() < 1e-6);
            assert!((bounds.1 - points_to_px(bottom)).abs() < 1e-6);
        }
        let cluster = InlineCluster::inline_object(
            0,
            3,
            InlineObject {
                width: 20.0,
                height: 30.0,
            },
        )
        .with_object_position(Some(f32::NAN));
        assert!(cluster.layout.is_none());
        assert_eq!(cluster.inline_object_bounds_px(), Some((-30.0, 0.0)));
    }

    #[test]
    #[ignore]
    fn benchmark_kerning_pair_cache_hot_path() {
        const ITERS: usize = 20_000;
        let ctx = KerningAwareContext::new(true);
        let style = ctx.style.clone();
        let text = "AVAVAVAVAVAVAVAVAVAVAVAVAVAVAVAV";

        let legacy_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let state = legacy_walk_text_advance_steps(
                &ctx,
                text,
                &style,
                None,
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |_| Ok(()),
            )
            .expect("legacy kerning benchmark");
            black_box((state, cache));
        }
        let legacy_elapsed = legacy_start.elapsed();

        let current_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let state = walk_text_advance_steps(
                &ctx,
                text,
                &style,
                None,
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |_| Ok(()),
            )
            .expect("current kerning benchmark");
            black_box((state, cache));
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "kerning pair cache benchmark: legacy={}us current={}us iterations={} graphemes={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
            text.graphemes(true).count(),
        );
    }

    fn legacy_resolve_snap_to_chars_cell_width<C: LineWrapContext>(
        ctx: &C,
        paragraph_style: &ParagraphStyle,
        east_asia_font_descriptor: &str,
        text_scale_x: f64,
        cache: &mut FxHashMap<String, f64>,
    ) -> Result<Option<f64>, C::Error> {
        if !paragraph_style.doc_grid_snap_to_chars {
            return Ok(None);
        }

        let char_space_px = paragraph_style.doc_grid_char_space_px.unwrap_or(0.0);
        let cache_key = format!("{east_asia_font_descriptor}|{text_scale_x:.6}|{char_space_px:.6}");
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(Some(*cached));
        }

        let measured = ctx.measure_text("あ", east_asia_font_descriptor)?;
        let width = (measured * EAST_ASIA_ADVANCE_SCALE * text_scale_x + char_space_px).max(1.0);
        cache.insert(cache_key, width);
        Ok(Some(width))
    }

    /// Word treats spaces after a tab as ordinary spaces: only the segment
    /// directly following the tab starts at the tab stop. A missing
    /// `pending_leading_width` reset used to re-apply the tab gap at every
    /// whitespace transition, spreading each word to a successive stop.
    #[test]
    fn tab_gap_applies_only_to_first_segment_after_tab() {
        let ctx = FontAwareVariableWidthContext::new(6.0, 12.0);
        let text = "1\ta b c";
        let runs = vec![make_run(text)];
        let lines =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &ParagraphStyle::default(), 0, 400.0)
                .expect("wrap");
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        let tab_end = line
            .clusters
            .iter()
            .position(|cluster| cluster.kind == InlineClusterKind::Tab)
            .map(|index| line.clusters[index].end)
            .expect("tab cluster");
        let leads: Vec<(usize, f64)> = line
            .segments
            .iter()
            .map(|segment| (segment.start, segment.leading_width))
            .collect();
        for (start, lead) in &leads {
            if *start == tab_end {
                assert!(
                    *lead > 0.0,
                    "segment at the tab stop must carry the tab gap: {leads:?}"
                );
            } else {
                assert!(
                    lead.abs() < f64::EPSILON,
                    "only the segment after the tab may carry the gap: {leads:?}"
                );
            }
        }
    }

    fn make_run(text: &str) -> RunStyle {
        RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }
    }

    #[test]
    fn wrapper_fills_both_side_segments_in_reading_order() {
        let ctx = TestContext::new(10.0);
        let text = "aaaaaa bbbbbb cccccc dddddd";
        let style = ParagraphStyle {
            float_line_segments: plan_usable_line_segments_around_floats(
                0.0,
                20.0,
                1,
                0.0,
                300.0,
                &[FloatExclusion {
                    top: 0.0,
                    bottom: 20.0,
                    left: 100.0,
                    right: 200.0,
                    wrap_text: WrapTextSide::BothSides,
                    polygon: None,
                }],
            ),
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 300.0)
            .expect("wrap both sides");
        let infos = collect_line_infos(&lines);
        // Left of the float, then past it on the same band, then full width.
        assert_eq!(infos[0].text.trim_end(), "aaaaaa");
        assert_eq!(infos[1].text.trim_end(), "bbbbbb");
        assert_eq!(infos[2].text.trim_end(), "cccccc dddddd");
    }

    #[test]
    fn wrapper_consumes_per_line_float_widths() {
        let ctx = TestContext::new(10.0);
        let text = "abcdefghij";
        let style = ParagraphStyle {
            float_line_segments: vec![
                FloatLineSegment {
                    start_x: 60.0,
                    width: 25.0,
                    skipped_lines_before: 0,
                    exact_clearance_height: 0.0,
                    shares_band_with_previous: false,
                },
                FloatLineSegment {
                    start_x: 0.0,
                    width: 100.0,
                    skipped_lines_before: 0,
                    exact_clearance_height: 0.0,
                    shares_band_with_previous: false,
                },
            ],
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 100.0)
            .expect("wrap around float plan");
        let info = collect_line_infos(&lines);
        assert_eq!(info[0].text, "ab");
        assert_eq!(info[1].text, "cdefghij");
    }

    #[test]
    fn consecutive_manual_page_break_runs_remain_two_page_break_lines() {
        let ctx = TestContext::new(10.0);
        let text = "He\u{000c}\u{000c}llo";
        let runs = [
            RunStyle {
                start: 0,
                end: 2,
                ..make_run("")
            },
            RunStyle {
                start: 2,
                end: 3,
                ..make_run("")
            },
            RunStyle {
                start: 3,
                end: 4,
                ..make_run("")
            },
            RunStyle {
                start: 4,
                end: text.len(),
                ..make_run("")
            },
        ];
        let lines =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &ParagraphStyle::default(), 0, 400.0)
                .expect("wrap consecutive page breaks");
        assert_eq!(
            lines.iter().map(|line| line.break_kind).collect::<Vec<_>>(),
            vec![
                LineBreakKind::Page,
                LineBreakKind::Page,
                LineBreakKind::ParagraphEnd,
            ]
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn prepared_drop_caps_preserve_unicode_text_and_reserve_exact_line_bands(
            first in prop::sample::select(vec!["A", "界", "🙂", "é", "ש", "क"]),
            suffix_chars in proptest::collection::vec(
                prop::sample::select(vec!['a', ' ', '界', '🙂', 'é']), 4..40,
            ),
            lines in 1_u8..=10,
            margin in any::<bool>(),
        ) {
            let ctx = TestContext::new(10.0);
            let suffix = suffix_chars.into_iter().collect::<String>();
            let text = format!("{first}{suffix}");
            let mut runs = vec![RunStyle {
                start: 0,
                end: text.len(),
                ..make_run("")
            }];
            let kind = if margin { DropCapKind::Margin } else { DropCapKind::Drop };
            let drop_cap = prepare_drop_cap_layout(
                &ctx,
                &text,
                &mut runs,
                DropCap { kind, lines },
                16.0,
            )
            .expect("prepare drop cap")
            .expect("visible first grapheme");

            prop_assert_eq!(drop_cap.kind, kind);
            prop_assert_eq!(drop_cap.lines, lines as usize);
            prop_assert_eq!(&text[drop_cap.grapheme_start..drop_cap.grapheme_end], first);
            prop_assert!(drop_cap.width.is_finite() && drop_cap.width > 0.0);
            prop_assert_eq!(runs.first().map(|run| run.start), Some(0));
            prop_assert_eq!(runs.last().map(|run| run.end), Some(text.len()));
            prop_assert!(runs.windows(2).all(|pair| pair[0].end == pair[1].start));

            let mut style = ParagraphStyle {
                drop_cap: Some(drop_cap),
                ..ParagraphStyle::default()
            };
            if margin {
                prop_assert!((style.line_indent(0) + drop_cap.width).abs() < 1e-6);
                prop_assert!(style.line_indent(1).abs() < 1e-6);
            } else {
                prop_assert!(style.line_indent(0).abs() < 1e-6);
                for line_index in 1..lines as usize {
                    prop_assert!((style.line_indent(line_index) - drop_cap.width).abs() < 1e-6);
                }
                prop_assert!(style.line_indent(lines as usize).abs() < 1e-6);
            }

            let wrapped = wrap_text_into_lines(&ctx, &text, &runs, 0.0, &style, 0, 120.0)
                .expect("wrap prepared drop cap");
            let wrapped_text = wrapped
                .iter()
                .flat_map(|line| line.segments.iter())
                .map(|segment| segment.text.as_str())
                .collect::<String>();
            prop_assert_eq!(wrapped_text, text);

            // Changing only placement must not mutate the synthetic run split.
            style.drop_cap = Some(DropCapLayout {
                kind: if margin { DropCapKind::Drop } else { DropCapKind::Margin },
                ..drop_cap
            });
            prop_assert!(style.line_indent(0).is_finite());
        }
    }

    #[test]
    fn vertical_layout_groups_lines_into_columns() {
        let ctx = TestContext::new(10.0);
        let text = "あいう";

        let layout =
            layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, Some(25.0), 10.0)
                .expect("vertical layout");

        assert_eq!(layout.lines.len(), 3);
        assert_eq!(layout.column_ranges, vec![0..2, 2..3]);
        assert!((layout.max_line_width - 10.0).abs() < f64::EPSILON);
        assert_eq!(collect_line_infos(&layout.lines)[0].text, "あ");
        assert_eq!(collect_line_infos(&layout.lines)[2].text, "う");
    }

    #[test]
    fn vertical_layout_uses_vertical_measurement_hook() {
        let ctx = TestContext::new(100.0).with_vertical_width(10.0);
        let text = "あいう";

        let layout =
            layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, Some(25.0), 10.0)
                .expect("vertical layout");

        assert_eq!(layout.column_ranges, vec![0..2, 2..3]);
        assert!((layout.max_line_width - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn section_vertical_metrics_preserve_measured_columns_and_source_ranges() {
        fn context() -> TestContext {
            let mut ctx = TestContext::new(14.0);
            ctx.style = ResolvedRunStyle::new(
                "10.50pt 'Yu Mincho'",
                "10.50pt 'Yu Mincho'",
                "10.50pt 'Yu Mincho'",
                "10.50pt 'Yu Mincho'",
                0.0,
                1.0,
                false,
                None,
            );
            ctx
        }
        let text = "あ\nいう";
        let legacy_ctx = context();
        let section_ctx = context();
        let legacy =
            layout_text_vertically(&legacy_ctx, text, &[make_run(text)], false, 7, None, 10.0)
                .unwrap();
        let section = layout_section_text_vertically(
            &section_ctx,
            text,
            &[make_run(text)],
            false,
            7,
            None,
            10.0,
        )
        .unwrap();
        assert_eq!(legacy.column_ranges, section.column_ranges);
        assert_eq!(legacy.column_height, section.column_height);
        assert_eq!(
            legacy_ctx.recorded_measurements(),
            section_ctx.recorded_measurements()
        );
        assert_eq!(section_ctx.recorded_measurements().len(), 3);
        let bounds = |layout: &VerticalWrappedLayout| {
            layout
                .lines
                .iter()
                .flat_map(|line| &line.clusters)
                .map(|cluster| (cluster.start, cluster.end, cluster.width))
                .collect::<Vec<_>>()
        };
        assert_eq!(bounds(&legacy), bounds(&section));
        assert!(legacy
            .lines
            .iter()
            .flat_map(|line| &line.clusters)
            .all(|cluster| cluster.layout.is_none()));
        assert!(section
            .lines
            .iter()
            .flat_map(|line| &line.clusters)
            .all(|cluster| cluster.layout.as_ref().unwrap().font_ascent.is_some()));
    }

    #[test]
    fn vertical_columns_fit_the_advances_used_by_painting() {
        let ctx = TestContext::new(10.0).with_vertical_width(6.0);
        let text = "あいうえ";
        let layout =
            layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, Some(20.0), 10.0)
                .expect("vertical layout");
        assert_eq!(layout.column_ranges, vec![0..3, 3..4]);

        let ctx = TestContext::new(10.0).with_vertical_width(12.0);
        let layout =
            layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, Some(20.0), 10.0)
                .expect("vertical layout");
        assert_eq!(layout.column_ranges, vec![0..1, 1..2, 2..3, 3..4]);
    }

    #[test]
    fn vertical_hard_break_starts_a_new_column() {
        let ctx = TestContext::new(10.0);
        let text = "あ\nいう";
        let layout =
            layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, Some(40.0), 10.0)
                .expect("vertical layout");
        assert_eq!(layout.column_ranges, vec![0..2, 2..4]);
    }

    #[test]
    fn vertical_layout_applies_run_text_scale_to_glyph_advance() {
        let mut ctx = TestContext::new(10.0);
        ctx.style.text_scale_x = 0.5;
        let text = "あいう";

        let layout = layout_text_vertically(&ctx, text, &[make_run(text)], false, 0, None, 10.0)
            .expect("vertical layout");

        assert!(
            layout
                .lines
                .iter()
                .flat_map(|line| &line.clusters)
                .all(|cluster| (cluster.width - 5.0).abs() < f64::EPSILON),
            "vertical tcFitText must scale the same advances consumed by layout and painting"
        );
        assert!((layout.max_line_width - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn turned_cell_reuses_wrap_indents_hard_breaks_and_source_ranges() {
        let ctx = TestContext::new(10.0);
        let text = "あいう\nえお";
        let style = ParagraphStyle {
            first_line_indent: 10.0,
            right_indent: 5.0,
            ..Default::default()
        };
        let result = layout_cell_text_turned(
            &ctx,
            text,
            &[make_run(text)],
            9,
            Some(40.0),
            10.0,
            5.0,
            &style,
        )
        .unwrap();
        assert_eq!(result.layout.column_ranges, vec![0..2, 2..3, 3..5]);
        assert_eq!(result.column_start_offsets, vec![15.0, 5.0, 5.0]);
        assert_eq!(result.layout.lines[0].clusters[0].start, 9);
        assert_eq!(result.layout.lines[4].clusters[0].end, 9 + text.len());
        assert_eq!(result.layout.lines[2].break_kind, LineBreakKind::Hard);
        assert_eq!(result.layout.lines[2].break_range, Some((18, 19)));
        let normal =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 9, 35.0).unwrap();
        let a = result
            .layout
            .lines
            .iter()
            .flat_map(|l| l.clusters.iter())
            .map(|c| (c.start, c.end, c.width))
            .collect::<Vec<_>>();
        let b = normal
            .iter()
            .flat_map(|l| l.clusters.iter())
            .map(|c| (c.start, c.end, c.width))
            .collect::<Vec<_>>();
        assert_eq!(a, b);
    }

    #[test]
    fn turned_baseline_centres_the_resolved_font_box_in_both_axes() {
        let mut cluster = InlineCluster::grapheme(0, 1, 10.0, "A", false, false, false);
        cluster.layout = Some(InlineClusterLayout {
            character_border_edges: 0,
            script: ScriptCategory::Latin,
            font_slot: crate::font_resolver::ResolvedFontSlot::LatinAscii,
            leading_advance: 0.0,
            glyph_box_width: 10.0,
            measured_glyph_width: 10.0,
            adjusted_glyph_width: 10.0,
            font_descriptor_hash: 0,
            font_ascent: Some(12.0),
            font_descent: Some(4.0),
        });
        let line = WrappedLine {
            segments: vec![],
            clusters: vec![cluster],
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: None,
            break_properties: None,
            leaders: vec![],
            tab_stops: vec![],
        };
        assert_eq!(
            wrapped_line_turned_baseline_offset(&line, 24.0, true),
            Some(8.0)
        );
        assert_eq!(
            wrapped_line_turned_baseline_offset(&line, 24.0, false),
            Some(16.0)
        );
        assert_eq!(
            wrapped_line_turned_baseline_offset(&line, 8.0, true),
            Some(4.0)
        );
        assert_eq!(
            wrapped_line_turned_baseline_offset(&line, 8.0, false),
            Some(12.0)
        );
    }

    #[test]
    fn vertical_cell_reserves_indents_and_justifies_only_a_soft_column() {
        let ctx = TestContext::new(14.0);
        let text = "こんにちは";
        let style = ParagraphStyle {
            alignment: "both".into(),
            right_indent: 113.0 / 20.0 * 4.0 / 3.0,
            ..Default::default()
        };
        let start = style.right_indent;
        let result = layout_cell_text_vertically(
            &ctx,
            text,
            &[make_run(text)],
            7,
            Some(1134.0 / 20.0 * 4.0 / 3.0),
            24.0,
            start,
            &style,
        )
        .unwrap();
        assert_eq!(result.layout.column_ranges, vec![0..4, 4..5]);
        assert_eq!(result.column_start_offsets, vec![start, start]);
        assert_eq!(result.layout.lines[0].clusters[0].start, 7);
        assert_eq!(result.layout.lines[4].clusters[0].end, 7 + text.len());
        let advances: Vec<f64> = result
            .layout
            .lines
            .iter()
            .map(|line| line.clusters.iter().map(|c| c.width).sum())
            .collect();
        assert!((advances[..4].iter().sum::<f64>() - (75.6 - start * 2.0)).abs() < 1e-9);
        assert!((advances[3] - 14.0).abs() < 1e-9);
        assert!((advances[4] - 14.0).abs() < 1e-9);
        assert_eq!(ctx.recorded_measurements().len(), 5);
    }

    #[test]
    fn vertical_cell_first_indent_and_hard_break_keep_source_geometry() {
        let ctx = TestContext::new(10.0);
        let text = "あいう\nえお";
        let style = ParagraphStyle {
            alignment: "both".into(),
            first_line_indent: 10.0,
            right_indent: 5.0,
            ..Default::default()
        };
        let result = layout_cell_text_vertically(
            &ctx,
            text,
            &[make_run(text)],
            9,
            Some(40.0),
            10.0,
            5.0,
            &style,
        )
        .unwrap();
        assert_eq!(result.layout.column_ranges, vec![0..2, 2..4, 4..6]);
        assert_eq!(result.column_start_offsets, vec![15.0, 5.0, 5.0]);
        assert_eq!(result.layout.lines[3].break_kind, LineBreakKind::Hard);
        assert_eq!(result.layout.lines[3].break_range, Some((18, 19)));
        assert_eq!(result.layout.lines[2].clusters[0].width, 10.0);
        assert_eq!(result.layout.lines[4].clusters[0].width, 10.0);
    }

    #[test]
    fn vertical_layout_batches_horizontal_latin_measurement_when_requested() {
        let ctx = TestContext::new(10.0);
        let text = "ABCD";

        let layout = layout_text_vertically(&ctx, text, &[make_run(text)], true, 0, None, 14.0)
            .expect("vertical layout");

        assert_eq!(layout.lines.len(), 1);
        let lines = collect_line_infos(&layout.lines);
        assert_eq!(lines[0].text, "ABCD");
        assert_eq!(ctx.recorded_measurements(), vec!["ABCD".to_string()]);
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    fn merge_run_props_override(target: &mut RunProperties, source: &RunProperties) {
        if source.font_size.is_some() {
            target.font_size = source.font_size;
        }
        if source.font_family.is_some() {
            target.font_family = source.font_family.clone();
        }
        if source.authored_east_asia_font_hint.is_some() {
            target.authored_east_asia_font_hint = source.authored_east_asia_font_hint;
        }
        if source.authored_east_asia_font_name.is_some() {
            target.authored_east_asia_font_name = source.authored_east_asia_font_name;
        }
        match (&mut target.fonts, source.fonts.as_ref()) {
            (Some(target_fonts), Some(source_fonts)) => {
                target_fonts.merge_from(source_fonts);
            }
            (None, Some(source_fonts)) => {
                target.fonts = Some(source_fonts.clone());
            }
            _ => {}
        }
    }

    fn fill_missing_run_fonts(target: &mut RunFonts, fallback: &RunFonts) {
        if target.ascii.is_none() {
            target.ascii = fallback.ascii.clone();
        }
        if target.h_ansi.is_none() {
            target.h_ansi = fallback.h_ansi.clone();
        }
        if target.east_asia.is_none() {
            target.east_asia = fallback.east_asia.clone();
        }
        if target.complex.is_none() {
            target.complex = fallback.complex.clone();
        }
        if target.ascii_theme.is_none() {
            target.ascii_theme = fallback.ascii_theme.clone();
        }
        if target.h_ansi_theme.is_none() {
            target.h_ansi_theme = fallback.h_ansi_theme.clone();
        }
        if target.east_asia_theme.is_none() {
            target.east_asia_theme = fallback.east_asia_theme.clone();
        }
        if target.complex_theme.is_none() {
            target.complex_theme = fallback.complex_theme.clone();
        }
        if target.hint.is_none() {
            target.hint = fallback.hint.clone();
        }
    }

    fn fill_missing_font_props(target: &mut RunProperties, fallback: &RunProperties) {
        if target.font_size.is_none() {
            target.font_size = fallback.font_size;
        }
        if target.font_family.is_none() {
            target.font_family = fallback.font_family.clone();
        }
        if target.authored_east_asia_font_hint.is_none() {
            target.authored_east_asia_font_hint = fallback.authored_east_asia_font_hint;
        }
        if target.authored_east_asia_font_name.is_none() {
            target.authored_east_asia_font_name = fallback.authored_east_asia_font_name;
        }
        match (&mut target.fonts, fallback.fonts.as_ref()) {
            (Some(target_fonts), Some(fallback_fonts)) => {
                fill_missing_run_fonts(target_fonts, fallback_fonts);
            }
            (None, Some(fallback_fonts)) => {
                target.fonts = Some(fallback_fonts.clone());
            }
            _ => {}
        }
    }

    fn clone_font_fallback_props(fallback: &RunProperties) -> Option<RunProperties> {
        let has_fonts = fallback
            .fonts
            .as_ref()
            .is_some_and(|fonts| !fonts.is_empty());
        if fallback.font_size.is_none()
            && fallback.font_family.is_none()
            && fallback.authored_east_asia_font_hint.is_none()
            && fallback.authored_east_asia_font_name.is_none()
            && !has_fonts
        {
            return None;
        }
        Some(RunProperties {
            font_size: fallback.font_size,
            font_family: fallback.font_family.clone(),
            fonts: fallback.fonts.clone(),
            authored_east_asia_font_hint: fallback.authored_east_asia_font_hint,
            authored_east_asia_font_name: fallback.authored_east_asia_font_name,
            ..RunProperties::default()
        })
    }

    fn apply_style_run_font_defaults_to_paragraph(paragraph: &mut Paragraph, styles: &StylesPart) {
        let paragraph_style_id = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref())
            .or_else(|| styles.default_paragraph_style_id());
        let style_run_props =
            paragraph_style_id.and_then(|style_id| styles.resolve_run_properties(style_id));
        let doc_default_run_props = styles.doc_default_run_properties().cloned();

        let mut fallback_props = RunProperties::default();
        let mut has_fallback = false;
        if let Some(defaults) = doc_default_run_props.as_ref() {
            merge_run_props_override(&mut fallback_props, defaults);
            has_fallback = true;
        }
        if let Some(style_props) = style_run_props.as_ref() {
            merge_run_props_override(&mut fallback_props, style_props);
            has_fallback = true;
        }
        if !has_fallback {
            return;
        }

        for run in &mut paragraph.runs {
            let mut run_fallback_props = fallback_props.clone();
            if let Some(style_id) = run
                .properties
                .as_ref()
                .and_then(|props| props.style.as_deref())
            {
                if let Some(character_style_props) =
                    styles.resolve_character_run_properties(style_id)
                {
                    merge_run_props_override(&mut run_fallback_props, &character_style_props);
                }
            }

            let Some(fallback_font_props) = clone_font_fallback_props(&run_fallback_props) else {
                continue;
            };

            if let Some(props) = run.properties.as_mut() {
                fill_missing_font_props(props, &fallback_font_props);
            } else {
                run.properties = Some(fallback_font_props);
            }
        }
    }

    fn apply_style_run_font_defaults_to_document(document: &mut Document, styles: &StylesPart) {
        let Some(body) = document.body.as_mut() else {
            return;
        };
        for element in &mut body.elements {
            match element {
                BodyElement::Paragraph(paragraph) => {
                    apply_style_run_font_defaults_to_paragraph(paragraph, styles);
                }
                BodyElement::Table(table) => {
                    for row in &mut table.rows {
                        for cell in &mut row.cells {
                            for block in &mut cell.blocks {
                                if let BodyElement::Paragraph(paragraph) = block {
                                    apply_style_run_font_defaults_to_paragraph(paragraph, styles);
                                }
                            }
                        }
                    }
                }
                BodyElement::Math(_) => {}
            }
        }
    }

    fn load_document_with_settings_and_style_defaults(
        fixture: &str,
    ) -> Result<(Document, DocumentSettings), Box<dyn std::error::Error>> {
        let data = fs::read(fixture_path(fixture))?;
        let package = executor::block_on(OpenXmlPackage::load(&data))?;
        let xml = package
            .get("word/document.xml")
            .ok_or("word/document.xml missing")?;
        let xml_text = String::from_utf8_lossy(xml);
        let mut document = DocumentParser::new().parse_document_xml(xml_text.as_ref())?;

        if let Some(styles_xml) = package.get("word/styles.xml") {
            let styles = StylesPart::from_xml(&String::from_utf8_lossy(styles_xml))?;
            apply_style_run_font_defaults_to_document(&mut document, &styles);
        }

        let settings = if let Some(settings_xml) = package.get("word/settings.xml") {
            DocumentSettings::from_xml(&String::from_utf8_lossy(settings_xml))?
        } else {
            DocumentSettings::default()
        };

        Ok((document, settings))
    }

    fn load_document_part_with_settings(
        fixture: &str,
        part_path: &str,
    ) -> Result<(Document, DocumentSettings), Box<dyn std::error::Error>> {
        let data = fs::read(fixture_path(fixture))?;
        let package = executor::block_on(OpenXmlPackage::load(&data))?;
        let xml = package.get(part_path).ok_or("document part missing")?;
        let document = DocumentParser::new().parse_document_xml(&String::from_utf8_lossy(xml))?;
        let settings = if let Some(settings_xml) = package.get("word/settings.xml") {
            DocumentSettings::from_xml(&String::from_utf8_lossy(settings_xml))?
        } else {
            DocumentSettings::default()
        };
        Ok((document, settings))
    }

    fn synthetic_numbered_two_line_document() -> Document {
        Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![Run {
                            text: "abc".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: None,
                        }],
                        properties: Some(ParagraphProperties {
                            widow_control: Some(true),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![Run {
                            text: "(1)abcdefg".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: None,
                        }],
                        properties: Some(ParagraphProperties {
                            widow_control: Some(true),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(40.0),
                height: Some(48.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                margin_top: Some(0.0),
                margin_bottom: Some(0.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        }
    }

    fn find_paragraph_by_prefix<'a>(document: &'a Document, prefix: &str) -> Option<&'a Paragraph> {
        document
            .body
            .as_ref()?
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .starts_with(prefix)
            })
    }

    fn layout_lines_with_widow_control<C: LineWrapContext<Error = ()>>(
        document: &Document,
        settings: &DocumentSettings,
        ctx: &C,
    ) -> Result<Vec<(usize, String)>, Box<dyn std::error::Error>> {
        let page_settings = document.page_settings.as_ref();
        let page_width_px = points_to_px(page_settings.and_then(|p| p.width).unwrap_or(612.0));
        let page_height_px = points_to_px(page_settings.and_then(|p| p.height).unwrap_or(792.0));
        let margin_left_px =
            points_to_px(page_settings.and_then(|p| p.margin_left).unwrap_or(72.0));
        let margin_right_px =
            points_to_px(page_settings.and_then(|p| p.margin_right).unwrap_or(72.0));
        let margin_top_px = points_to_px(page_settings.and_then(|p| p.margin_top).unwrap_or(72.0));
        let margin_bottom_px =
            points_to_px(page_settings.and_then(|p| p.margin_bottom).unwrap_or(72.0));
        let bottom_limit = page_height_px - margin_bottom_px;
        let base_x = margin_left_px;
        let column_right = page_width_px - margin_right_px;
        let mut current_page = 1usize;
        let mut current_y = margin_top_px;
        let mut page_lines: Vec<(usize, String)> = Vec::new();

        let ensure_space = |needed: f64, page: &mut usize, y: &mut f64| {
            if *y + needed > bottom_limit {
                *page += 1;
                *y = margin_top_px;
            }
        };

        let body = match document.body.as_ref() {
            Some(body) => body,
            None => return Ok(Vec::new()),
        };

        for element in &body.elements {
            let paragraph = match element {
                BodyElement::Paragraph(paragraph) => paragraph,
                _ => continue,
            };

            let mut paragraph_x = base_x;
            let mut paragraph_style = ParagraphStyle {
                east_asian_rules: EastAsianLineBreakingRules::from_settings(
                    &settings.to_line_breaking_settings(),
                ),
                ..ParagraphStyle::default()
            };

            if let Some(props) = paragraph.properties.as_ref() {
                if let Some(alignment) = props.alignment.as_ref() {
                    paragraph_style.alignment = alignment.to_string();
                }
                if let Some(indent) = props.indent.as_ref() {
                    apply_paragraph_indent(
                        &mut paragraph_x,
                        &mut paragraph_style,
                        indent,
                        props.numbering.is_some(),
                    );
                }
                if !props.tabs.is_empty() {
                    paragraph_style.tab_stops = resolve_tab_stops(&props.tabs);
                }
            }

            let mut text = String::new();
            let mut run_styles: Vec<RunStyle> = Vec::new();
            for run in &paragraph.runs {
                if run.text.is_empty() {
                    continue;
                }
                let start = text.len();
                text.push_str(&run.text);
                let end = text.len();
                run_styles.push(RunStyle {
                    start,
                    end,
                    properties: run.properties.clone().map(Rc::new),
                    revision: run.revision.clone(),
                    hyperlink: run.hyperlink.clone(),
                    inline_object: None,
                });
            }
            if text.is_empty() {
                continue;
            }

            let spacing_ref = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.spacing.as_ref());
            let line_height_points = resolve_line_height_points(
                estimate_paragraph_font_size(paragraph),
                spacing_ref,
                None,
                false,
                None,
                None,
            );
            let line_height_px = points_to_px(line_height_points);

            if let Some(spacing) = spacing_ref {
                if let Some(before) = spacing.before {
                    let before_px = points_to_px(before);
                    ensure_space(before_px, &mut current_page, &mut current_y);
                    current_y += before_px;
                } else if let Some(before_lines) = spacing.before_lines {
                    let before_px = points_to_px(line_height_points * before_lines);
                    ensure_space(before_px, &mut current_page, &mut current_y);
                    current_y += before_px;
                }
            }

            let lines = wrap_text_into_lines(
                ctx,
                &text,
                &run_styles,
                paragraph_x,
                &paragraph_style,
                0,
                column_right,
            )
            .map_err(|_| std::io::Error::other("failed to wrap text"))?;
            let infos = collect_line_infos(&lines);
            if infos.is_empty() {
                continue;
            }

            let widow_control_enabled = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.widow_control)
                .unwrap_or(true);
            let mut force_break_before_line: Option<usize> = None;
            if widow_control_enabled && infos.len() > 1 {
                let remaining = (bottom_limit - current_y).max(0.0);
                let lines_fit = ((remaining + 1e-6) / line_height_px).floor() as usize;
                match resolve_widow_orphan_decision(infos.len(), lines_fit) {
                    WidowOrphanDecision::MoveWholeParagraph => {
                        ensure_space(
                            (infos.len() as f64) * line_height_px,
                            &mut current_page,
                            &mut current_y,
                        );
                    }
                    WidowOrphanDecision::BreakBeforeLine(line_index) => {
                        force_break_before_line = Some(line_index);
                    }
                    WidowOrphanDecision::FlowNormally => {}
                }
            }

            let total = infos.len();
            for (line_index, info) in infos.into_iter().enumerate() {
                if force_break_before_line == Some(line_index) {
                    let remaining_lines = (total - line_index) as f64;
                    ensure_space(
                        remaining_lines * line_height_px,
                        &mut current_page,
                        &mut current_y,
                    );
                }
                ensure_space(line_height_px, &mut current_page, &mut current_y);
                current_y += line_height_px;
                page_lines.push((current_page, info.text.trim().to_string()));
            }

            if let Some(spacing) = spacing_ref {
                if let Some(after) = spacing.after {
                    let after_px = points_to_px(after);
                    ensure_space(after_px, &mut current_page, &mut current_y);
                    current_y += after_px;
                } else if let Some(after_lines) = spacing.after_lines {
                    let after_px = points_to_px(line_height_points * after_lines);
                    ensure_space(after_px, &mut current_page, &mut current_y);
                    current_y += after_px;
                }
            }
        }

        Ok(page_lines)
    }

    fn wrap_fixture_paragraph_lines<C: LineWrapContext<Error = ()>>(
        document: &Document,
        settings: &DocumentSettings,
        paragraph: &Paragraph,
        ctx: &C,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let page_settings = document.page_settings.as_ref();
        let page_width_px = points_to_px(page_settings.and_then(|p| p.width).unwrap_or(612.0));
        let margin_left_px =
            points_to_px(page_settings.and_then(|p| p.margin_left).unwrap_or(72.0));
        let margin_right_px =
            points_to_px(page_settings.and_then(|p| p.margin_right).unwrap_or(72.0));
        let column_right = page_width_px - margin_right_px;

        let mut paragraph_x = margin_left_px;
        let defaults =
            EastAsianLineBreakingRules::from_settings(&settings.to_line_breaking_settings());
        let mut paragraph_style = ParagraphStyle {
            east_asian_rules: defaults,
            ..ParagraphStyle::default()
        };
        if let Some(props) = paragraph.properties.as_ref() {
            if let Some(alignment) = props.alignment.as_ref() {
                paragraph_style.alignment = alignment.to_string();
            }
            if let Some(indent) = props.indent.as_ref() {
                apply_paragraph_indent(
                    &mut paragraph_x,
                    &mut paragraph_style,
                    indent,
                    props.numbering.is_some(),
                );
            }
            if !props.tabs.is_empty() {
                paragraph_style.tab_stops = resolve_tab_stops(&props.tabs);
            }
            paragraph_style.east_asian_rules = merge_east_asian_rules(
                &paragraph_style.east_asian_rules,
                props.east_asian_settings.as_ref(),
            );
        }
        paragraph_style.enable_line_break_rules =
            paragraph_style.east_asian_rules.use_east_asian_layout;
        let doc_grid = document
            .default_section
            .as_ref()
            .and_then(|section| section.doc_grid.as_ref());
        let snap_to_grid = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.snap_to_grid)
            .unwrap_or(document.snap_to_grid_default);
        paragraph_style.apply_doc_grid_character_settings(doc_grid, snap_to_grid);

        let mut text = String::new();
        let mut run_styles: Vec<RunStyle> = Vec::new();
        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let start = text.len();
            text.push_str(&run.text);
            let end = text.len();
            run_styles.push(RunStyle {
                start,
                end,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let lines = wrap_text_into_lines(
            ctx,
            &text,
            &run_styles,
            paragraph_x,
            &paragraph_style,
            0,
            column_right,
        )
        .map_err(|_| std::io::Error::other("wrap failed"))?;
        Ok(collect_line_infos(&lines)
            .into_iter()
            .map(|info| info.text.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    fn estimate_paragraph_font_size(paragraph: &Paragraph) -> f32 {
        paragraph
            .runs
            .iter()
            .filter_map(|run| run.properties.as_ref().and_then(|props| props.font_size))
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(12.0)
    }

    fn page_three_lines_for_fixture(
        ascii_width: f64,
        cjk_width: f64,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let data = fs::read(fixture_path("yokohama_page6.docx"))?;
        let package = executor::block_on(OpenXmlPackage::load(&data))?;
        let xml = package
            .get("word/document.xml")
            .ok_or("word/document.xml missing")?;
        let xml_text = String::from_utf8_lossy(xml);
        let document = DocumentParser::new().parse_document_xml(xml_text.as_ref())?;

        let page_settings = document.page_settings.as_ref();
        let page_width_px = points_to_px(page_settings.and_then(|p| p.width).unwrap_or(612.0));
        let page_height_px = points_to_px(page_settings.and_then(|p| p.height).unwrap_or(792.0));
        let margin_left_px =
            points_to_px(page_settings.and_then(|p| p.margin_left).unwrap_or(72.0));
        let margin_right_px =
            points_to_px(page_settings.and_then(|p| p.margin_right).unwrap_or(72.0));
        let margin_top_px = points_to_px(page_settings.and_then(|p| p.margin_top).unwrap_or(72.0));
        let margin_bottom_px =
            points_to_px(page_settings.and_then(|p| p.margin_bottom).unwrap_or(72.0));
        let bottom_limit = page_height_px - margin_bottom_px;
        let base_x = margin_left_px;
        let column_right = page_width_px - margin_right_px;
        let mut current_page = 1usize;
        let mut current_y = margin_top_px;
        let mut page_lines: Vec<(usize, String)> = Vec::new();
        let ctx = VariableWidthContext::new(ascii_width, cjk_width);

        let ensure_space = |needed: f64, page: &mut usize, y: &mut f64| {
            if *y + needed > bottom_limit {
                *page += 1;
                *y = margin_top_px;
            }
        };

        let body = match document.body {
            Some(body) => body,
            None => return Ok(Vec::new()),
        };

        for element in body.elements {
            let paragraph = match element {
                BodyElement::Paragraph(paragraph) => paragraph,
                _ => continue,
            };

            let mut paragraph_x = base_x;
            let mut paragraph_style = ParagraphStyle {
                east_asian_rules: EastAsianLineBreakingRules {
                    use_east_asian_layout: true,
                    balance_single_byte_double_byte_width: true,
                    overflow_punctuation: Some(true),
                    ..EastAsianLineBreakingRules::default()
                },
                enable_line_break_rules: true,
                ..ParagraphStyle::default()
            };

            if let Some(props) = paragraph.properties.as_ref() {
                if let Some(alignment) = props.alignment.as_ref() {
                    paragraph_style.alignment = alignment.to_string();
                }
                if let Some(indent) = props.indent.as_ref() {
                    apply_paragraph_indent(
                        &mut paragraph_x,
                        &mut paragraph_style,
                        indent,
                        props.numbering.is_some(),
                    );
                }
                if !props.tabs.is_empty() {
                    paragraph_style.tab_stops = resolve_tab_stops(&props.tabs);
                }
            }

            let mut text = String::new();
            let mut run_styles: Vec<RunStyle> = Vec::new();
            for run in &paragraph.runs {
                if run.text.is_empty() {
                    continue;
                }
                let start = text.len();
                text.push_str(&run.text);
                let end = text.len();
                run_styles.push(RunStyle {
                    start,
                    end,
                    properties: run.properties.clone().map(Rc::new),
                    revision: run.revision.clone(),
                    hyperlink: run.hyperlink.clone(),
                    inline_object: None,
                });
            }
            if text.is_empty() {
                continue;
            }

            let spacing_ref = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.spacing.as_ref());
            let line_height_points = resolve_line_height_points(
                estimate_paragraph_font_size(&paragraph),
                spacing_ref,
                None,
                false,
                None,
                None,
            );
            let line_height_px = points_to_px(line_height_points);

            if let Some(spacing) = spacing_ref {
                if let Some(before) = spacing.before {
                    let before_px = points_to_px(before);
                    ensure_space(before_px, &mut current_page, &mut current_y);
                    current_y += before_px;
                } else if let Some(before_lines) = spacing.before_lines {
                    let before_px = points_to_px(line_height_points * before_lines);
                    ensure_space(before_px, &mut current_page, &mut current_y);
                    current_y += before_px;
                }
            }

            let lines = wrap_text_into_lines(
                &ctx,
                &text,
                &run_styles,
                paragraph_x,
                &paragraph_style,
                0,
                column_right,
            )
            .map_err(|_| std::io::Error::other("failed to wrap text"))?;
            let infos = collect_line_infos(&lines);
            if infos.is_empty() {
                continue;
            }

            let widow_control_enabled = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.widow_control)
                .unwrap_or(true);
            let mut force_break_before_line: Option<usize> = None;
            if widow_control_enabled && infos.len() > 1 {
                let remaining = (bottom_limit - current_y).max(0.0);
                let lines_fit = ((remaining + 1e-6) / line_height_px).floor() as usize;
                match resolve_widow_orphan_decision(infos.len(), lines_fit) {
                    WidowOrphanDecision::MoveWholeParagraph => {
                        ensure_space(
                            (infos.len() as f64) * line_height_px,
                            &mut current_page,
                            &mut current_y,
                        );
                    }
                    WidowOrphanDecision::BreakBeforeLine(line_index) => {
                        force_break_before_line = Some(line_index);
                    }
                    WidowOrphanDecision::FlowNormally => {}
                }
            }

            let total = infos.len();
            for (line_index, info) in infos.into_iter().enumerate() {
                if force_break_before_line == Some(line_index) {
                    let remaining_lines = (total - line_index) as f64;
                    ensure_space(
                        remaining_lines * line_height_px,
                        &mut current_page,
                        &mut current_y,
                    );
                }
                ensure_space(line_height_px, &mut current_page, &mut current_y);
                current_y += line_height_px;
                page_lines.push((current_page, info.text.trim().to_string()));
            }

            if let Some(spacing) = spacing_ref {
                if let Some(after) = spacing.after {
                    let after_px = points_to_px(after);
                    ensure_space(after_px, &mut current_page, &mut current_y);
                    current_y += after_px;
                } else if let Some(after_lines) = spacing.after_lines {
                    let after_px = points_to_px(line_height_points * after_lines);
                    ensure_space(after_px, &mut current_page, &mut current_y);
                    current_y += after_px;
                }
            }
        }

        let lines = page_lines
            .into_iter()
            .filter(|(page, text)| *page == 3 && !text.is_empty())
            .map(|(_, text)| text)
            .collect();
        Ok(lines)
    }

    #[test]
    fn descriptor_for_grapheme_uses_ascii_and_hansi() {
        let style = ResolvedRunStyle::new(
            "ASCII".to_string(),
            "HANSI".to_string(),
            "EA".to_string(),
            "CS".to_string(),
            0.0,
            1.0,
            false,
            None,
        );

        assert_eq!(
            style.descriptor_for_grapheme("A", ScriptCategory::Latin),
            "ASCII"
        );
        assert_eq!(
            style.descriptor_for_grapheme("é", ScriptCategory::Latin),
            "HANSI"
        );
    }

    #[test]
    fn widow_control_keeps_two_line_numbered_paragraph_on_current_page_when_it_fits() {
        let context = TestContext::new(10.0);
        let document = synthetic_numbered_two_line_document();
        let settings = DocumentSettings::default();
        let paragraph = document
            .body
            .as_ref()
            .expect("body")
            .elements
            .get(1)
            .and_then(BodyElement::as_paragraph)
            .expect("numbered paragraph");

        let lines =
            wrap_fixture_paragraph_lines(&document, &settings, paragraph, &context).expect("wrap");
        assert_eq!(lines, vec!["(1)ab".to_string(), "cdefg".to_string()]);

        let laid_out =
            layout_lines_with_widow_control(&document, &settings, &context).expect("layout");
        assert_eq!(
            laid_out,
            vec![
                (1, "abc".to_string()),
                (1, "(1)ab".to_string()),
                (1, "cdefg".to_string()),
            ]
        );
    }

    #[test]
    fn descriptor_for_grapheme_applies_hint() {
        let style = ResolvedRunStyle::new(
            "ASCII".to_string(),
            "HANSI".to_string(),
            "EA".to_string(),
            "CS".to_string(),
            0.0,
            1.0,
            false,
            Some(FontHint::EastAsia),
        );

        assert_eq!(
            style.descriptor_for_grapheme("\u{201C}", ScriptCategory::Latin),
            "EA"
        );
        // Basic Latin keeps the ascii font whatever the hint says.
        assert_eq!(
            style.descriptor_for_grapheme("A", ScriptCategory::Latin),
            "ASCII"
        );
    }

    #[test]
    fn wrap_text_respects_measured_width() {
        let ctx = TestContext::new(5.0);
        let text = "abcdefgh";
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 16.0).unwrap();

        let segments: Vec<String> = lines
            .iter()
            .map(|line| {
                line.segments
                    .first()
                    .map(|segment| segment.text.clone())
                    .unwrap_or_default()
            })
            .collect();

        assert_eq!(segments, vec!["abc", "def", "gh"]);
        assert_eq!(
            lines.last().unwrap().break_kind,
            LineBreakKind::ParagraphEnd
        );
        assert!(ctx.recorded_measurements().len() >= 3);
    }

    #[test]
    fn trailing_ascii_space_can_hang_past_right_edge_like_word() {
        let ctx = TestContext::new(10.0);
        let text = "abc d";
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 30.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("abc "));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("d"));
    }

    #[test]
    fn latin_word_after_east_asian_text_moves_as_one_word() {
        // trust-beneficiary-sale-keep-next page 8: Word ends a line at `新規`
        // and starts the next with `MLPM`; the boundary with East Asian text
        // is the word's break opportunity, not a split after its first `M`.
        let ctx = TestContext::new(10.0);
        let text = "あああMLPMいい";
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 65.0)
            .expect("wrap");
        assert_eq!(
            collect_line_infos(&lines)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["あああ", "MLPMいい"]
        );
    }

    #[test]
    fn number_after_east_asian_text_moves_as_one_word() {
        // trust-beneficiary-sale-keep-next page 9: Word ends a line at
        // `売買代金の` and starts the next with `20％`, not `2` | `0％`.
        let ctx = TestContext::new(10.0);
        let text = "あああ20％いい";
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 45.0)
            .expect("wrap");
        assert_eq!(
            collect_line_infos(&lines)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["あああ", "20％いい"]
        );
        // A unit that may not start a line, after a number squeezed onto the
        // line, takes the whole number along.
        let lines = wrap_text_into_lines(
            &ctx,
            "あああ、20％いい",
            &[make_run("あああ、20％いい")],
            0.0,
            &ParagraphStyle {
                alignment: "both".to_string(),
                word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
                east_asian_rules: EastAsianLineBreakingRules {
                    character_spacing_control: Some("compressPunctuation".into()),
                    ..paragraph.east_asian_rules.clone()
                },
                ..paragraph.clone()
            },
            0,
            60.0,
        )
        .expect("wrap");
        let texts: Vec<_> = collect_line_infos(&lines)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(
            texts.last().map(String::as_str),
            Some("20％いい"),
            "{texts:?}"
        );
        // A justified Mac line that could squeeze the `2` in still moves the
        // number whole.
        let justified = ParagraphStyle {
            alignment: "both".to_string(),
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..paragraph
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &justified, 0, 45.0)
            .expect("wrap");
        assert_eq!(
            collect_line_infos(&lines)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["あああ", "20％いい"]
        );
    }

    #[test]
    fn wrapped_line_starting_with_latin_drops_the_auto_space_before_it() {
        // trust-beneficiary-sale-keep-next pages 5, 8 and 9: Word starts a
        // wrapped line's `MLPM`, `V` or `20％` at the line's edge; the auto
        // spacing belongs to the East Asian character left on the line above.
        let ctx = TestContext::new(10.0);
        let text = "あああMLPMいい";
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 65.0)
            .expect("wrap");
        assert_eq!(
            collect_line_infos(&lines)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["あああ", "MLPMいい"]
        );
        let first = &lines[1].clusters[0];
        assert_eq!(first.text, "M");
        assert_eq!(first.auto_space_leading, 0.0);
        assert!(
            first.layout.as_ref().unwrap().leading_advance.abs() < 1e-9,
            "{:?}",
            first.layout
        );
    }

    #[test]
    fn mac_justification_leaves_the_ideographic_space_unstretched() {
        use crate::line_metrics::WordLayoutPlatform;
        assert!(is_justification_space(" ", WordLayoutPlatform::Mac));
        assert!(!is_justification_space("\u{3000}", WordLayoutPlatform::Mac));
        assert!(is_justification_space(
            "\u{3000}",
            WordLayoutPlatform::Windows
        ));
        assert!(!is_justification_space("あ", WordLayoutPlatform::Mac));
        assert!(!is_justification_space("", WordLayoutPlatform::Mac));
    }

    #[test]
    fn slash_joined_latin_word_moves_as_one_word() {
        // word-compat-comprehensive: Word sets `header/footer` on the next
        // line whole rather than splitting it after the slash or a letter.
        let ctx = TestContext::new(10.0);
        let text = "and header/footer run";
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &ParagraphStyle::default(),
            0,
            135.0,
        )
        .expect("wrap");
        let infos = collect_line_infos(&lines);
        assert_eq!(
            infos
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["and ", "header/footer ", "run"]
        );
    }

    #[test]
    fn justified_trailing_space_overflow_moves_whole_latin_word() {
        let ctx = TestContext::new(10.0);
        let text = "Hello World Hello World ";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            alignment: "both".to_string(),
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 165.0).expect("wrap");
        let infos = collect_line_infos(&lines);

        // `tests/word-oracle/2col/word-trace.json` keeps justified Latin lines
        // to the Word column width. A trailing separator may hang, but if the
        // preceding Latin word is also overfull, move the word as a unit so the
        // browser does not split it as `Worl`/`d`.
        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("Hello World ")
        );
        assert_eq!(
            infos.get(1).map(|line| line.text.as_str()),
            Some("Hello World ")
        );
    }

    #[test]
    fn justified_soft_line_excludes_trailing_separator_from_content_edge() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let paragraph = ParagraphStyle {
            alignment: "both".into(),
            ..ParagraphStyle::default()
        };
        let text = "aa aa aa";
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 23.0).unwrap();
        assert_eq!(collect_line_infos(&lines)[0].text, "aa aa ");
        let natural: f64 = lines[0].clusters.iter().map(|cluster| cluster.width).sum();
        assert_eq!(natural, 24.0);
        assert_eq!(
            lines[0].justification_extra_width_px(23.0, natural, 0.0),
            1.0
        );
        assert_eq!(
            lines[0].justification_extra_width_px(20.0, natural, 0.0),
            0.0
        );
        // A terminal line has no hanging separator; punctuation remains an
        // explicit caller-provided overhang, as in the existing CJK path.
        assert_eq!(lines[1].justification_extra_width_px(12.0, 10.0, 0.0), 2.0);
        assert_eq!(lines[1].justification_extra_width_px(10.0, 12.0, 3.0), 1.0);
    }

    #[test]
    fn distributed_final_line_expands_character_gaps_across_runs() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let text = "ABCD";
        let runs = [
            make_run("AB"),
            RunStyle {
                start: 2,
                end: 4,
                ..make_run("CD")
            },
        ];
        for alignment in ["distribute", " DISTRIBUTE "] {
            let paragraph = ParagraphStyle {
                alignment: alignment.into(),
                ..ParagraphStyle::default()
            };
            let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 100.0)
                .expect("distributed line");
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].break_kind, LineBreakKind::ParagraphEnd);
            assert_eq!(collect_line_infos(&lines)[0].text, text);
            let clusters = &lines[0].clusters;
            assert_eq!(
                clusters
                    .iter()
                    .map(|c| (c.start, c.end))
                    .collect::<Vec<_>>(),
                vec![(0, 1), (1, 2), (2, 3), (3, 4)]
            );
            let width: f64 = clusters.iter().map(|c| c.width).sum();
            assert!((width - 100.0).abs() < 1e-6, "distributed width={width}");
            assert!((clusters.last().unwrap().width - 5.0).abs() < 1e-6);
        }
        for alignment in ["both", "left", "thaiDistribute"] {
            let paragraph = ParagraphStyle {
                alignment: alignment.into(),
                ..ParagraphStyle::default()
            };
            let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 100.0).unwrap();
            assert_eq!(lines[0].clusters.iter().map(|c| c.width).sum::<f64>(), 20.0);
        }
    }

    #[test]
    fn distributed_line_of_one_character_centres_it() {
        // Word for Mac 16.114 (squeeze probe sq4, `w:jc="distribute"`) sets a
        // last line holding only `も` midway across the line: 177.96pt in a
        // 196.5pt line from 84.96pt. A trailing space does not count.
        let ctx = TestContext::new(10.5);
        let paragraph = ParagraphStyle {
            alignment: "distribute".into(),
            ..ParagraphStyle::default()
        };
        for text in ["も", "も\u{3000}"] {
            let lines =
                wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 196.5)
                    .unwrap();
            let cluster = &lines[0].clusters[0];
            let leading = cluster.layout.as_ref().unwrap().leading_advance;
            assert!(
                (leading - (196.5 - 10.5) / 2.0).abs() < 1e-9,
                "{text:?} {leading}"
            );
            assert!((cluster.width - (196.5 + 10.5) / 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn distributed_rtl_line_expands_in_visual_order() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let text = "אבגד";
        let paragraph = ParagraphStyle {
            alignment: "distribute".into(),
            bidi: true,
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 100.0).unwrap();
        let clusters = &lines[0].clusters;
        assert_eq!(
            clusters.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            vec!["ד", "ג", "ב", "א"]
        );
        assert_eq!(
            clusters.iter().map(|c| c.start).collect::<Vec<_>>(),
            vec![6, 4, 2, 0]
        );
        assert!((clusters.iter().map(|c| c.width).sum::<f64>() - 100.0).abs() < 1e-6);
        assert!((clusters.last().unwrap().width - 2.0).abs() < 1e-6);
    }

    #[test]
    fn distributed_lines_keep_tab_stops_and_unexpandable_content() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let paragraph = ParagraphStyle {
            alignment: "distribute".into(),
            ..ParagraphStyle::default()
        };
        // A lone character is centred instead (Word for Mac 16.114 distribute
        // probe: `A`, `も`, and `あ` before a trailing space all sit midway).
        for (text, expected) in [("", 0.0), ("A", (100.0 + 5.0) / 2.0), ("ABCD", 20.0)] {
            let limit = if text == "ABCD" { 10.0 } else { 100.0 };
            let lines =
                wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, limit)
                    .unwrap();
            let width: f64 = lines
                .iter()
                .flat_map(|line| &line.clusters)
                .map(|c| c.width)
                .sum();
            assert_eq!(width, expected, "{text:?}");
        }
        let tabbed = "A\tBC";
        let plain = ParagraphStyle {
            alignment: "left".into(),
            ..paragraph.clone()
        };
        let distributed =
            wrap_text_into_lines(&ctx, tabbed, &[make_run(tabbed)], 0.0, &paragraph, 0, 100.0)
                .unwrap();
        let left =
            wrap_text_into_lines(&ctx, tabbed, &[make_run(tabbed)], 0.0, &plain, 0, 100.0).unwrap();
        assert!(!distributed[0].tab_stops.is_empty());
        assert_eq!(
            distributed[0]
                .clusters
                .iter()
                .map(|c| c.width)
                .collect::<Vec<_>>(),
            left[0].clusters.iter().map(|c| c.width).collect::<Vec<_>>()
        );
    }

    fn finished_punctuation_test_line(text: &str) -> WrappedLine {
        let ctx = TestContext::new(14.0);
        wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &ParagraphStyle::default(),
            0,
            1000.0,
        )
        .unwrap()
        .remove(0)
    }

    fn mac_punctuation_test_style() -> ParagraphStyle {
        ParagraphStyle {
            alignment: "both".into(),
            enable_line_break_rules: true,
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            east_asian_rules: EastAsianLineBreakingRules {
                character_spacing_control: Some("compressPunctuation".into()),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        }
    }

    #[test]
    fn completed_mac_punctuation_allocation_keeps_boundaries_and_quarter_sides() {
        let mut line = finished_punctuation_test_line("甲・乙、丙・丁");
        line.break_kind = LineBreakKind::Soft;
        let ranges = line
            .clusters
            .iter()
            .map(|c| (c.start, c.end, c.text.clone()))
            .collect::<Vec<_>>();
        compress_mac_punctuation_on_completed_lines(
            std::slice::from_mut(&mut line),
            &mac_punctuation_test_style(),
            84.0,
        );
        assert!((line.clusters.iter().map(|c| c.width).sum::<f64>() - 84.0).abs() < 1e-9);
        assert_eq!(
            ranges,
            line.clusters
                .iter()
                .map(|c| (c.start, c.end, c.text.clone()))
                .collect::<Vec<_>>()
        );
        let middle = line.clusters[2].layout.as_ref().unwrap().leading_advance;
        let comma = line.clusters[4].layout.as_ref().unwrap().leading_advance;
        assert!((comma - 2.0 * middle).abs() < 1e-9);
        assert!(line
            .clusters
            .iter()
            .all(|c| c.layout.as_ref().unwrap().glyph_box_width == 14.0));
        assert_eq!(line.break_kind, LineBreakKind::Soft);
    }

    #[test]
    fn completed_mac_punctuation_terminal_blank_reduces_only_when_internal_space_is_insufficient() {
        let mut line = finished_punctuation_test_line("甲、乙。");
        line.clusters.last_mut().unwrap().can_hang_line_end = true;
        compress_mac_punctuation_on_completed_lines(
            std::slice::from_mut(&mut line),
            &mac_punctuation_test_style(),
            48.0,
        );
        assert!((line.clusters.iter().map(|c| c.width).sum::<f64>() - 48.0).abs() < 1e-9);
        assert_eq!(
            line.clusters
                .last()
                .unwrap()
                .layout
                .as_ref()
                .unwrap()
                .glyph_box_width,
            7.0
        );
        assert_eq!(
            line.clusters[2].layout.as_ref().unwrap().leading_advance,
            -1.0
        );
        let mut inside = finished_punctuation_test_line("甲、乙、丙。");
        inside.clusters.last_mut().unwrap().can_hang_line_end = true;
        compress_mac_punctuation_on_completed_lines(
            std::slice::from_mut(&mut inside),
            &mac_punctuation_test_style(),
            76.0,
        );
        assert_eq!(
            inside
                .clusters
                .last()
                .unwrap()
                .layout
                .as_ref()
                .unwrap()
                .glyph_box_width,
            14.0
        );
        assert!((inside.clusters.iter().map(|c| c.width).sum::<f64>() - 76.0).abs() < 1e-9);
    }

    #[test]
    fn paragraph_end_mark_hangs_its_whole_blank_half_like_a_soft_line() {
        // scroll-offset-repro page 10: `…退職とする。` ends its paragraph 1.2pt
        // past the edge. Word for Mac hangs the whole blank half of `。` and
        // justifies the line to the edge, so the paragraph's last line is set
        // like any other.
        for break_kind in [LineBreakKind::Soft, LineBreakKind::ParagraphEnd] {
            let mut line = finished_punctuation_test_line("甲乙。");
            line.break_kind = break_kind;
            line.clusters.last_mut().unwrap().can_hang_line_end = true;
            compress_mac_punctuation_on_completed_lines(
                std::slice::from_mut(&mut line),
                &mac_punctuation_test_style(),
                40.0,
            );
            let last = line.clusters.last().unwrap();
            assert_eq!(last.width, 7.0, "{break_kind:?}");
            assert_eq!(last.layout.as_ref().unwrap().glyph_box_width, 7.0);
        }
    }

    #[test]
    fn do_not_compress_line_squeezes_auto_spacing_before_hanging_its_final_mark() {
        // trust-beneficiary-sale-keep-next page 14 (doNotCompress): the line
        // `…（別紙10の様式別紙Aに…含む。）` would push its final `）` past the
        // edge. Word for Mac narrows the auto spacing around `10` and `A`
        // instead, and the bracket stays inside.
        // Natural 88px, 80px available: an 8px overflow past half the
        // bracket (7px), within the 9px the auto spacing can give.
        let mut line = finished_punctuation_test_line("甲1乙。）");
        for index in [1, 2] {
            let cluster = &mut line.clusters[index];
            cluster.auto_space_leading = 9.0;
            cluster.width += 9.0;
            cluster.layout.as_mut().unwrap().leading_advance += 9.0;
        }
        line.clusters.last_mut().unwrap().can_hang_line_end = true;
        let mut style = mac_punctuation_test_style();
        style.east_asian_rules.character_spacing_control = Some("doNotCompress".into());
        compress_mac_punctuation_on_completed_lines(std::slice::from_mut(&mut line), &style, 80.0);
        let last = line.clusters.last().unwrap();
        assert_eq!(last.width, 14.0, "the bracket keeps its whole cell");
        assert!((line.clusters.iter().map(|c| c.width).sum::<f64>() - 80.0).abs() < 1e-9);
    }

    #[test]
    fn final_comma_overflow_narrows_the_tab_before_it_by_the_comma() {
        // Word for Mac 16.114 probe tabshrink-comma (doNotCompress): a
        // justified line `1．<tab>…、` whose comma would end past the edge
        // narrows the tab by the comma's whole advance and keeps the comma
        // inside; a tab narrower than that closes and the comma hangs the
        // rest (trust page 22).
        // (default tab interval, available width, tab before, tab after)
        for (interval, available, tab_before, tab_after) in
            [(56.0, 100.0, 42.0, 28.0), (20.0, 70.0, 6.0, 0.0)]
        {
            let mut style = ParagraphStyle {
                default_tab_width: interval,
                ..mac_punctuation_test_style()
            };
            style.east_asian_rules.character_spacing_control = Some("doNotCompress".into());
            let ctx = TestContext::new(14.0);
            let text = "1\t甲乙丙、";
            let mut line =
                wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 1000.0)
                    .unwrap()
                    .remove(0);
            line.break_kind = LineBreakKind::Soft;
            line.clusters.last_mut().unwrap().can_hang_line_end = true;
            let tab = line
                .clusters
                .iter()
                .position(|c| c.kind == InlineClusterKind::Tab)
                .unwrap();
            assert_eq!(line.clusters[tab].width, tab_before);
            let natural: f64 = line.clusters.iter().map(|c| c.width).sum();
            shrink_tab_for_final_mark(&mut line, &style, available, natural);
            assert_eq!(line.clusters[tab].width, tab_after, "available {available}");
            assert_eq!(
                line.tab_stops[0].end_x - line.tab_stops[0].start_x,
                tab_after
            );
            let after_tab = line
                .segments
                .iter()
                .find(|s| s.start > line.clusters[tab].start)
                .unwrap();
            assert_eq!(after_tab.leading_width, tab_after);
            assert_eq!(line.clusters.last().unwrap().width, 14.0);
        }
    }

    #[test]
    fn completed_mac_punctuation_controls_keep_original_advances() {
        let original = finished_punctuation_test_line("甲。）乙、丙");
        let mut styles = Vec::new();
        let mut style = mac_punctuation_test_style();
        style.word_platform = crate::line_metrics::WordLayoutPlatform::Windows;
        styles.push(style);
        let mut style = mac_punctuation_test_style();
        style.east_asian_rules.character_spacing_control = Some("doNotCompress".into());
        styles.push(style);
        let mut style = mac_punctuation_test_style();
        style.east_asian_rules.character_spacing_control = None;
        styles.push(style);
        let mut style = mac_punctuation_test_style();
        style.alignment = "left".into();
        styles.push(style);
        let mut style = mac_punctuation_test_style();
        style.oracle_advance_px_by_text_offset = Some(Rc::new(FxHashMap::default()));
        styles.push(style);
        for style in styles {
            let mut line = original.clone();
            compress_mac_punctuation_on_completed_lines(
                std::slice::from_mut(&mut line),
                &style,
                76.0,
            );
            assert_eq!(
                line.clusters.iter().map(|c| c.width).collect::<Vec<_>>(),
                original
                    .clusters
                    .iter()
                    .map(|c| c.width)
                    .collect::<Vec<_>>()
            );
        }
        // Adjacent punctuation is outside the adjustable space: even when
        // the line is compressed, no extra reduction occurs at 。|）.
        let mut line = original;
        compress_mac_punctuation_on_completed_lines(
            std::slice::from_mut(&mut line),
            &mac_punctuation_test_style(),
            76.0,
        );
        assert_eq!(
            line.clusters[2].layout.as_ref().unwrap().leading_advance,
            0.0
        );
        let mut insufficient = finished_punctuation_test_line("甲、乙");
        let old = insufficient
            .clusters
            .iter()
            .map(|c| c.width)
            .collect::<Vec<_>>();
        compress_mac_punctuation_on_completed_lines(
            std::slice::from_mut(&mut insufficient),
            &mac_punctuation_test_style(),
            20.0,
        );
        assert_eq!(
            insufficient
                .clusters
                .iter()
                .map(|c| c.width)
                .collect::<Vec<_>>(),
            old
        );
    }

    /// Characters on the first line when the k-th character needs `squeeze`
    /// points of reduction to fit: 10.5-unit glyphs, like 10.5pt MS Mincho.
    fn mac_squeeze_first_line(text: &str, k: usize, squeeze: f64, style: &ParagraphStyle) -> usize {
        let ctx = TestContext::new(10.5);
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            style,
            0,
            k as f64 * 10.5 - squeeze,
        )
        .unwrap();
        lines[0]
            .clusters
            .iter()
            .filter(|c| !c.is_whitespace)
            .count()
    }

    /// `k`-th character preceded by commas at the 1-based `commas` positions.
    fn mac_squeeze_text(k: usize, commas: &[usize]) -> String {
        let kana = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん";
        let mut kana = kana.chars().cycle();
        (1..k + 15)
            .map(|position| {
                if commas.contains(&position) {
                    '、'
                } else {
                    kana.next().unwrap()
                }
            })
            .collect()
    }

    // Word for Mac 16.114 squeeze probes (justified MS Mincho 10.5pt,
    // compressPunctuation): the k-th character is admitted by narrowing the
    // commas only while the reduction per comma stays within 32 times the gap
    // each character would gain if it were pushed to the next line. Each case
    // is the last admitted and first refused reduction Word showed.
    #[test]
    fn mac_squeeze_admits_a_character_while_punctuation_reduction_beats_expansion() {
        let style = mac_punctuation_test_style();
        for (k, commas, admitted, refused) in [
            (19, &[4, 9, 14][..], 8.75, 9.0),
            (5, &[2, 4][..], 10.0, 10.05),
            (6, &[2, 4][..], 9.85, 9.9),
            (8, &[2, 4, 6][..], 9.85, 9.9),
            (12, &[3, 9][..], 9.05, 9.1),
            (40, &[10, 30][..], 6.55, 6.6),
            (40, &[8, 20, 32][..], 7.5, 7.55),
            (25, &[4, 10, 16, 22][..], 8.9, 8.95),
        ] {
            let text = mac_squeeze_text(k, commas);
            assert_eq!(
                mac_squeeze_first_line(&text, k, admitted, &style),
                k,
                "k={k} commas={commas:?} admits at {admitted}"
            );
            assert_eq!(
                mac_squeeze_first_line(&text, k, refused, &style),
                k - 1,
                "k={k} commas={commas:?} refuses at {refused}"
            );
        }
    }

    #[test]
    fn characters_that_exactly_fill_an_indented_line_stay_on_it() {
        // Indents travel as f32 points, so 19 em of room set as a twips right
        // indent arrives a few 1e-5 px short. Word works in whole twips and
        // keeps all 19 characters (squeeze probe, paragraph at D = 0).
        let style = mac_punctuation_test_style();
        let text = mac_squeeze_text(19, &[]);
        // A4 width 11906tw and 1701tw margins, also carried as f32 points.
        let page_pt = f64::from(11906.0_f32 / 20.0);
        let margin_pt = f64::from(1701.0_f32 / 20.0);
        let right_indent_pt = f64::from((8504.0_f32 - 19.0 * 210.0) / 20.0);
        let available = (page_pt - 2.0 * margin_pt - right_indent_pt) * 96.0 / 72.0;
        let ctx = TestContext::new(14.0);
        let lines =
            wrap_text_into_lines(&ctx, &text, &[make_run(&text)], 0.0, &style, 0, available)
                .unwrap();
        assert!(
            available < 19.0 * 14.0,
            "the fixture reproduces the f32 shortfall"
        );
        assert_eq!(lines[0].clusters.len(), 19);
    }

    #[test]
    fn mac_squeeze_never_narrows_a_comma_past_its_blank_half() {
        // One comma can give at most its 5.25pt blank: Word admits at 5.25 and
        // refuses at 5.5 however long the line is.
        let style = mac_punctuation_test_style();
        for k in [10, 19, 30] {
            let text = mac_squeeze_text(k, &[k / 2]);
            assert_eq!(mac_squeeze_first_line(&text, k, 5.25, &style), k);
            assert_eq!(mac_squeeze_first_line(&text, k, 5.5, &style), k - 1);
        }
        // Without punctuation there is nothing to narrow.
        let plain = mac_squeeze_text(19, &[]);
        assert_eq!(mac_squeeze_first_line(&plain, 19, 0.25, &style), 18);
    }

    #[test]
    fn mac_squeeze_counts_the_blank_before_fullwidth_digits() {
        // scroll-offset-repro page 6: Word narrows both commas by 2.65pt to
        // keep `１０` on the line, the second comma's blank sitting before the
        // fullwidth `１`.
        let text = "採用日から６か月間継続勤務し、所定労働日の８割以上出勤した社員に対しては、１０日の年次有給休暇を与える。";
        let ctx = TestContext::new(10.5);
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &mac_punctuation_test_style(),
            0,
            404.2,
        )
        .unwrap();
        let first: String = lines[0].clusters.iter().map(|c| c.text.as_str()).collect();
        assert!(first.ends_with("１０"), "{first}");
    }

    #[test]
    fn mac_squeeze_reaches_past_fullwidth_spaces() {
        // scroll-offset-repro page 5: `…この場合、　　　前日…`. Word keeps
        // `前` by narrowing four commas 3.95pt each, the last before the
        // spaces; pushed out, the spaces would hang and the line would end at
        // the comma with room to spare.
        let text = "を得ない事情により、これらを繰り上げ、又は繰り下げることがある。この場合、\u{3000}\u{3000}\u{3000}前日までに社員に通知する。";
        let ctx = TestContext::new(10.5);
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &mac_punctuation_test_style(),
            0,
            414.7,
        )
        .unwrap();
        let first: String = lines[0].clusters.iter().map(|c| c.text.as_str()).collect();
        assert!(first.ends_with("前"), "{first}");
        let total: f64 = lines[0].clusters.iter().map(|c| c.width).sum();
        assert!((total - 414.7).abs() < 1e-6, "{total}");
    }

    /// 10.5pt MS Mincho: kana 10.5pt, ASCII half width.
    struct MinchoContext {
        style: ResolvedRunStyle,
    }

    impl MinchoContext {
        fn new() -> Self {
            let descriptor = "10.50pt 'MS Mincho', serif";
            Self {
                style: ResolvedRunStyle::new(
                    descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, false, None,
                ),
            }
        }
    }

    impl LineWrapContext for MinchoContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            Ok(text
                .chars()
                .map(|ch| if ch.is_ascii() { 7.0 } else { 14.0 })
                .sum())
        }
    }

    // Word for Mac 16.114 auto-space probes (justified 10.5pt MS Mincho,
    // compressPunctuation and doNotCompress alike): an overflowing kana is
    // kept by narrowing the auto spacing around `AB` runs. Each case is the
    // last kept and the first refused overflow, in points.
    #[test]
    fn mac_squeeze_narrows_auto_spacing_like_word() {
        let px = |pt: f64| pt * 96.0 / 72.0;
        let kana = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん";
        for (k, runs, kept, refused) in [
            (30, 1, 2.2, 2.3),
            (10, 2, 3.3, 3.4),
            (20, 3, 3.2, 3.3),
            (20, 1, 2.6, 2.7),
        ] {
            let positions: Vec<usize> = (0..runs)
                .map(|i| ((i + 1) * (k - 1) + runs / 2) / (runs + 1))
                .collect();
            let mut letters = kana.chars().cycle();
            let mut text = String::new();
            for unit in 1..k {
                if positions.contains(&unit) {
                    text.push_str("AB");
                } else {
                    text.push(letters.next().unwrap());
                }
            }
            let head_chars = text.chars().count();
            let natural_pt = text
                .chars()
                .map(|ch| if ch.is_ascii() { 5.25 } else { 10.5 })
                .sum::<f64>()
                + 10.5
                + runs as f64 * 2.0 * 2.625;
            text.extend(letters.by_ref().take(15));
            for compress in [true, false] {
                let mut style = mac_punctuation_test_style();
                style.east_asian_rules.character_spacing_control = Some(
                    if compress {
                        "compressPunctuation"
                    } else {
                        "doNotCompress"
                    }
                    .into(),
                );
                let first = |overflow: f64| {
                    wrap_text_into_lines(
                        &MinchoContext::new(),
                        &text,
                        &[make_run(&text)],
                        0.0,
                        &style,
                        0,
                        px(natural_pt - overflow),
                    )
                    .unwrap()[0]
                        .clusters
                        .iter()
                        .filter(|c| !c.is_whitespace)
                        .count()
                };
                let case = format!("k={k} runs={runs} compress={compress}");
                assert_eq!(first(kept), head_chars + 1, "{case} keeps at {kept}");
                assert_eq!(first(refused), head_chars, "{case} refuses at {refused}");
            }
        }
    }

    #[test]
    fn mac_squeeze_narrows_punctuation_before_auto_spacing() {
        // math.docx: Word narrows `、` and `。` 4.58pt each to fit the line and
        // leaves the auto spacing around `8pt` alone.
        let text = "後、ホームタブ。下の数式は8ptに変更した場合";
        let ctx = MinchoContext::new();
        let style = mac_punctuation_test_style();
        let natural = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 2000.0)
            .unwrap()[0]
            .clusters
            .iter()
            .map(|c| c.width)
            .sum::<f64>();
        let squeeze = 9.16 * 96.0 / 72.0;
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &style,
            0,
            natural - squeeze,
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        let digit = lines[0].clusters.iter().find(|c| c.text == "8").unwrap();
        let after = lines[0].clusters.iter().find(|c| c.text == "に").unwrap();
        assert!(digit.auto_space_leading > 0.0);
        assert!(
            (digit.layout.as_ref().unwrap().leading_advance - digit.auto_space_leading).abs()
                < 1e-9,
            "auto spacing before `8` stays"
        );
        assert!(
            after.layout.as_ref().unwrap().leading_advance >= -1e-9,
            "and after `t`"
        );
    }

    #[test]
    fn mac_squeeze_admission_needs_justified_compress_punctuation() {
        // Word pushes the character out at a 3pt reduction for left/centered
        // paragraphs and for doNotCompress documents; kerning does not matter.
        let text = mac_squeeze_text(19, &[4, 9, 14]);
        let style = mac_punctuation_test_style();
        assert_eq!(mac_squeeze_first_line(&text, 19, 3.0, &style), 19);
        let distribute = ParagraphStyle {
            alignment: "distribute".into(),
            ..style.clone()
        };
        assert_eq!(mac_squeeze_first_line(&text, 19, 3.0, &distribute), 19);
        for alignment in ["left", "center"] {
            let other = ParagraphStyle {
                alignment: alignment.into(),
                ..style.clone()
            };
            assert_eq!(
                mac_squeeze_first_line(&text, 19, 3.0, &other),
                18,
                "{alignment}"
            );
        }
        let mut no_compress = style.clone();
        no_compress.east_asian_rules.character_spacing_control = Some("doNotCompress".into());
        assert_eq!(mac_squeeze_first_line(&text, 19, 3.0, &no_compress), 18);
    }

    /// The first line of `text` (Japanese, so `、` and `。` may hang) at the
    /// given right edge, as `(text, width)` per cluster.
    fn mac_kinsoku_first_line(text: &str, right_edge: f64) -> Vec<(String, f64)> {
        let ctx = TestContext::new(10.5);
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[run],
            0.0,
            &mac_punctuation_test_style(),
            0,
            right_edge,
        )
        .unwrap();
        lines[0]
            .clusters
            .iter()
            .map(|c| (c.text.to_string(), c.width))
            .collect()
    }

    #[test]
    fn mac_line_end_comma_hangs_after_a_closing_paren() {
        // scroll-offset-repro page 11 `…ごとに１回）、`: Word for Mac keeps
        // the closing paren and hangs the comma after it; the paren's own
        // line-start ban does not stop the comma from hanging. Here the paren
        // already overflows by 3pt (squeezed in), and the comma then hangs
        // half and squeezes the rest.
        let text = "あ、ああああ（あ）、う";
        let line = mac_kinsoku_first_line(text, 9.0 * 10.5 - 3.0);
        let texts: Vec<_> = line.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts.last(), Some(&"、"), "{texts:?}");
        assert_eq!(texts.len(), 10, "{texts:?}");
    }

    // Word for Mac 16.114 squeeze probe with a comma as the 19th character
    // (it may not start a line): Word always keeps it, hanging its blank half
    // while that covers the overflow, otherwise narrowing the three earlier
    // commas, and hanging the half once those are exhausted.
    #[test]
    fn mac_line_end_comma_hangs_or_squeezes_like_word() {
        let text = mac_squeeze_text(19, &[4, 9, 14, 19]);
        for (overflow, hang, squeeze_each) in [
            // f32 page geometry leaves an exact fit ~1e-5 px over: no hang,
            // and the half-em and full-capacity boundaries stay where Word
            // puts them.
            (1.5e-5, 0.0, 0.0),
            (5.25 + 1.5e-5, 5.25, 0.0),
            (15.75 + 1.5e-5, 0.0, 5.25),
            (0.25, 5.25, 0.0),
            (5.25, 5.25, 0.0),
            (5.5, 0.0, 5.5 / 3.0),
            (10.0, 0.0, 10.0 / 3.0),
            (15.75, 0.0, 5.25),
            (16.0, 5.25, (16.0 - 5.25) / 3.0),
            (17.0, 5.25, (17.0 - 5.25) / 3.0),
        ] {
            let line = mac_kinsoku_first_line(&text, 19.0 * 10.5 - overflow);
            assert_eq!(line.len(), 19, "overflow {overflow}");
            assert_eq!(line[18].0, "、");
            assert!(
                (line[18].1 - (10.5 - hang)).abs() < 1e-6,
                "overflow {overflow}: line-end comma {}",
                line[18].1
            );
            for index in [3, 8, 13] {
                let narrowed = line[index].1 + line[index + 1].1 - 21.0;
                assert!(
                    (narrowed + squeeze_each).abs() < 1e-6,
                    "overflow {overflow}: comma {index} narrowed {narrowed}"
                );
            }
        }
    }

    #[test]
    fn mac_paragraph_end_period_hangs_whole_when_half_is_not_enough() {
        // scroll-offset-repro page 3 `…ことがある。`: 15.8pt over with two
        // commas (10.5pt) to give; Word sets the period wholly past the margin
        // and narrows each comma by 2.65pt.
        let text = "ああ、いいいい、うう。";
        let natural = 11.0 * 10.5;
        let line = mac_kinsoku_first_line(text, natural - 15.8);
        assert_eq!(line.len(), 11);
        assert!(line[10].1.abs() < 1e-6, "{}", line[10].1);
        for index in [2, 7] {
            let narrowed = line[index].1 + line[index + 1].1 - 21.0;
            assert!((narrowed + 2.65).abs() < 1e-6, "{narrowed}");
        }
    }

    #[test]
    fn mac_squeezed_line_narrows_every_comma_equally() {
        // At 8.75pt Word draws each of the three commas 7.58pt wide.
        let text = mac_squeeze_text(19, &[4, 9, 14]);
        let ctx = TestContext::new(10.5);
        let lines = wrap_text_into_lines(
            &ctx,
            &text,
            &[make_run(&text)],
            0.0,
            &mac_punctuation_test_style(),
            0,
            19.0 * 10.5 - 8.75,
        )
        .unwrap();
        let widths = lines[0]
            .clusters
            .iter()
            .filter(|c| c.text.as_str() == "、")
            .map(|c| c.width)
            .collect::<Vec<_>>();
        let commas: Vec<f64> = lines[0]
            .clusters
            .windows(2)
            .filter(|pair| pair[0].text.as_str() == "、")
            .map(|pair| pair[0].width + pair[1].width - 10.5)
            .collect();
        assert_eq!(commas.len(), 3, "{widths:?}");
        for comma in commas {
            assert!((comma - (10.5 - 8.75 / 3.0)).abs() < 1e-6, "{comma}");
        }
        let total: f64 = lines[0].clusters.iter().map(|c| c.width).sum();
        assert!((total - (19.0 * 10.5 - 8.75)).abs() < 1e-6, "{total}");
    }

    #[test]
    fn completed_mac_space_pass_keeps_word_space_and_punctuation_reductions_separate() {
        let style = mac_punctuation_test_style();
        let context = TestContext::new(14.0);
        let original = finished_punctuation_test_line("甲 乙、丙、丁。");
        for (available, expected_space, expected_comma_leading) in
            [(109.2, 11.2, 0.0), (102.0, 14.0, -5.0)]
        {
            let mut line = original.clone();
            let ranges = line
                .clusters
                .iter()
                .map(|c| (c.start, c.end))
                .collect::<Vec<_>>();
            compress_justified_word_spaces_to_available_width(
                &context,
                std::slice::from_mut(&mut line),
                0.0,
                &style,
                available,
                false,
                &mut FxHashMap::default(),
            )
            .unwrap();
            assert!((line.clusters.iter().map(|c| c.width).sum::<f64>() - available).abs() < 1e-9);
            assert!(
                (line.clusters[1].layout.as_ref().unwrap().glyph_box_width - expected_space).abs()
                    < 1e-9
            );
            assert!(
                (line.clusters[4].layout.as_ref().unwrap().leading_advance
                    - expected_comma_leading)
                    .abs()
                    < 1e-9
            );
            assert_eq!(
                ranges,
                line.clusters
                    .iter()
                    .map(|c| (c.start, c.end))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn complete_mac_paragraph_candidate_uses_original_word_terminal_glyphs() {
        let ctx = TestContext::new(14.0);
        let text =
            "２　前項について、会社が特に認めたときは、試用期間を短縮し、又は設けないことがある。";
        let mut run = make_run(text);
        run.properties = Some(Rc::new(RunProperties {
            font_size: Some(10.5),
            font_family: Some(Rc::from("MS Mincho")),
            east_asia_language: Some("ja-JP".into()),
            ..Default::default()
        }));
        let origin_pt = 85.05000305175781;
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[run],
            origin_pt * 4.0 / 3.0,
            &mac_punctuation_test_style(),
            0,
            510.25 * 4.0 / 3.0,
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].break_kind, LineBreakKind::ParagraphEnd);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/word-oracle/mac-glyph-corpus-20261010/scroll-offset-repro.json");
        let reference: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let row = reference["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|page| page["rows"].as_array().unwrap())
            .find(|row| row["text"].as_str().unwrap().starts_with("２前項について"))
            .unwrap();
        let mut cursor = origin_pt * 4.0 / 3.0;
        let mut actual = Vec::new();
        for cluster in &lines[0].clusters {
            if !cluster.is_whitespace {
                actual.push((
                    cluster.text.as_str(),
                    (cursor + cluster.layout.as_ref().unwrap().leading_advance) * 0.75,
                ));
            }
            cursor += cluster.width;
        }
        let expected = row["characters"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for ((text, x), expected) in actual.iter().zip(expected) {
            assert_eq!(*text, expected["text"].as_str().unwrap());
            assert!(
                (x - expected["x_pt"].as_f64().unwrap()).abs() <= 0.375,
                "{text}: {x}, {expected}"
            );
        }
        assert_eq!(
            lines[0]
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn mac_compressed_punctuation_uses_word_stop_gap_on_existing_soft_line() {
        let ctx = TestContext::new(14.0);
        let text = "２　休職期間中に休職事由が消滅したときは、原則として元の職務に復帰させる。ただし、元の職務に復帰させることが困難又は不適当な場合には、他の職務に就かせることがある。";
        let paragraph = ParagraphStyle {
            alignment: "both".into(),
            enable_line_break_rules: true,
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            east_asian_rules: EastAsianLineBreakingRules {
                character_spacing_control: Some("compressPunctuation".into()),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let mut run = make_run(text);
        run.properties = Some(Rc::new(RunProperties {
            east_asia_language: Some("ja-JP".into()),
            ..RunProperties::default()
        }));
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 425.2 * 4.0 / 3.0)
            .unwrap();
        assert_eq!(
            lines[0]
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            "２　休職期間中に休職事由が消滅したときは、原則として元の職務に復帰させる。ただし、"
        );
        let index = lines[0]
            .clusters
            .windows(2)
            .position(|pair| pair[0].text.as_str() == "。" && pair[1].text.as_str() == "た")
            .unwrap();
        let gap = lines[0].clusters[index]
            .layout
            .as_ref()
            .unwrap()
            .glyph_box_width
            + lines[0].clusters[index + 1]
                .layout
                .as_ref()
                .unwrap()
                .leading_advance;
        assert!((gap * 0.75 - 7.849).abs() <= 0.375, "stop gap {gap}px");
    }

    #[test]
    fn justified_mac_paragraph_end_compresses_spaces_without_expanding_short_lines() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let text = "ab cd ef gh ij kl";
        let paragraph = ParagraphStyle {
            alignment: "both".into(),
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        // 60px of letters plus 10px of spaces: the required 1.8px reduction
        // is more than one sixth, but below one fifth of the word spaces.
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 68.2).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].break_kind, LineBreakKind::ParagraphEnd);
        assert!((lines[0].clusters.iter().map(|c| c.width).sum::<f64>() - 68.2).abs() < 1e-9);
        assert!(lines[0]
            .clusters
            .iter()
            .filter(|c| !c.is_whitespace)
            .all(|c| c.width == 5.0));
        let roomy =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 100.0).unwrap();
        assert!((roomy[0].clusters.iter().map(|c| c.width).sum::<f64>() - 70.0).abs() < 1e-9);
        let repeated = format!("{text} {text}");
        let repeated_lines = wrap_text_into_lines(
            &ctx,
            &repeated,
            &[make_run(&repeated)],
            0.0,
            &paragraph,
            0,
            68.2,
        )
        .unwrap();
        assert_eq!(repeated_lines.len(), 2);
        assert_eq!(repeated_lines[1].break_kind, LineBreakKind::ParagraphEnd);
        for line in &repeated_lines {
            let extent = line.clusters.iter().map(|c| c.width).sum::<f64>()
                - trailing_whitespace_width(&line.clusters);
            assert!((extent - 68.2).abs() < 1e-9);
        }
        for (width, platform) in [
            (67.9, crate::line_metrics::WordLayoutPlatform::Mac),
            (68.2, crate::line_metrics::WordLayoutPlatform::Windows),
        ] {
            let bounded = wrap_text_into_lines(
                &ctx,
                text,
                &[make_run(text)],
                0.0,
                &ParagraphStyle {
                    word_platform: platform,
                    ..paragraph.clone()
                },
                0,
                width,
            )
            .unwrap();
            assert_eq!(bounded.len(), 2);
        }
        let left = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &ParagraphStyle {
                alignment: "left".into(),
                ..paragraph
            },
            0,
            68.2,
        )
        .unwrap();
        assert_eq!(left.len(), 2);
    }

    #[test]
    fn justified_latin_wrap_uses_word_space_compression_capacity() {
        let ctx = VariableWidthContext::new(5.0, 2.0);
        let text = "World Hello World Hello World Hello World Hello";
        let paragraph = ParagraphStyle {
            alignment: "both".to_string(),
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 185.0)
            .expect("wrap");
        let infos = collect_line_infos(&lines);

        // Word 16.112 fits this seven-word continuation in the 2col fixture by
        // shrinking its six ordinary word spaces. Its natural width is 187px,
        // while reducing each 2px space by one sixth makes it fit 185px.
        assert_eq!(
            infos.first().map(|line| line.text.trim_end()),
            Some("World Hello World Hello World Hello World")
        );
        let first_width: f64 = lines[0]
            .clusters
            .iter()
            .map(|cluster| cluster.width)
            .sum::<f64>()
            - trailing_whitespace_width(&lines[0].clusters);
        assert!(
            (first_width - 185.0).abs() < 1e-6,
            "compressed first-line width={first_width}"
        );

        let left_aligned = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &ParagraphStyle::default(),
            0,
            185.0,
        )
        .expect("wrap left aligned");
        assert_eq!(
            collect_line_infos(&left_aligned)
                .first()
                .map(|line| line.text.as_str()),
            Some("World Hello World Hello World Hello ")
        );
    }

    #[test]
    fn residual_hanging_first_line_keeps_unscaled_fe_content_capacity() {
        let paragraph = ParagraphStyle {
            first_line_indent: -16.0,
            tab_origin_offset: -32.0,
            east_asian_advance_scale: crate::constants::EAST_ASIA_FE_ADVANCE_SCALE,
            ..ParagraphStyle::default()
        };

        assert!((paragraph.available_width_for_line(400.0, 0) - 420.16).abs() < 1e-6);
        assert_eq!(paragraph.available_width_for_line(400.0, 1), 392.0);
    }

    #[test]
    fn word_oracle_two_column_latin_wrap_keeps_word_fit_boundary() {
        let ctx = TestContext::new(6.0);
        let text = "Hello World Hello World Hello World Hello World";
        let run = make_run(text);
        let word_advances_pt = [
            9.0, 4.5, 3.0, 3.0, 5.25, 3.0, 10.5, 5.25, 4.5, 2.25, 6.0, 3.0, 8.25, 5.25, 3.0, 3.0,
            5.25, 2.25, 10.5, 6.0, 3.75, 3.0, 6.0, 2.25, 9.0, 5.25, 2.25, 3.0, 6.0, 2.25, 10.5,
            5.25, 4.5, 3.0, 5.25, 3.0, 8.25, 5.25, 3.0, 3.0, 5.25, 5.25,
        ];
        let oracle = word_advances_pt
            .iter()
            .enumerate()
            .map(|(offset, width_pt)| (offset, points_to_px(*width_pt)))
            .collect();
        let paragraph = ParagraphStyle {
            alignment: "both".to_string(),
            oracle_advance_px_by_text_offset: Some(Rc::new(oracle)),
            first_line_available_adjustment:
                -crate::text_extract::ORACLE_MULTI_COLUMN_WRAP_BONUS_PX,
            continuation_available_adjustment:
                crate::text_extract::ORACLE_MULTI_COLUMN_WRAP_BONUS_PX,
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, points_to_px(201.75))
                .expect("wrap");
        let infos = collect_line_infos(&lines);

        // TRACE evidence: `tests/word-oracle/2col/word-trace.json` records the
        // first justified two-column line with `Hello` still on the current
        // line. This protects the generic Latin word-boundary repair from
        // moving a whole fitting word when the next Word line begins after an
        // omitted source-space separator.
        assert_eq!(
            infos.first().map(|line| line.text.trim_end()),
            Some("Hello World Hello World Hello World Hello")
        );
    }

    #[test]
    fn trailing_fullwidth_space_can_hang_past_right_edge_like_word() {
        let ctx = TestContext::new(10.0);
        let text = "abc　d";
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 30.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("abc　"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("d"));
    }

    #[test]
    fn wrap_text_applies_text_scale_x() {
        let mut ctx = TestContext::new(5.0);
        ctx.style.text_scale_x = 2.0;
        let text = "abcd";
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 16.0).unwrap();

        let segments: Vec<String> = lines
            .iter()
            .map(|line| {
                line.segments
                    .first()
                    .map(|segment| segment.text.clone())
                    .unwrap_or_default()
            })
            .collect();

        assert_eq!(segments, vec!["a", "b", "c", "d"]);
    }

    /// Measures every grapheme one em wide at the descriptor's size, with
    /// MS Mincho's 0.859/0.141 em ascent and descent.
    struct EmWidthContext;

    impl EmWidthContext {
        fn size_px(font_descriptor: &str) -> f64 {
            font_descriptor
                .split_whitespace()
                .find_map(|token| token.strip_suffix("pt")?.parse::<f64>().ok())
                .map_or(16.0, |pt| pt * 96.0 / 72.0)
        }
    }

    impl LineWrapContext for EmWidthContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(resolve_run_style_for_wrap(properties, None))
        }

        fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
            Ok(text.graphemes(true).count() as f64 * Self::size_px(font_descriptor))
        }

        fn measure_font_vertical_metrics(
            &self,
            font_descriptor: &str,
        ) -> Result<Option<(f64, f64)>, Self::Error> {
            let size = Self::size_px(font_descriptor);
            Ok(Some((size * 0.859, size * 0.141)))
        }
    }

    fn ruby_runs(
        text: &str,
        base: std::ops::Range<usize>,
        guide: &str,
        align: &str,
    ) -> Vec<RunStyle> {
        let plain = Rc::new(RunProperties {
            font_size: Some(10.5),
            ..RunProperties::default()
        });
        let annotated = Rc::new(RunProperties {
            font_size: Some(10.5),
            ruby: Some(Box::new(jade_docx_reader::document::RubyAnnotation {
                text: guide.to_string(),
                size_pt: Some(5.5),
                align: Some(align.to_string()),
                raise_half_points: Some(21),
                ..Default::default()
            })),
            ..RunProperties::default()
        });
        [
            (0, base.start, &plain),
            (base.start, base.end, &annotated),
            (base.end, text.len(), &plain),
        ]
        .into_iter()
        .map(|(start, end, properties)| RunStyle {
            start,
            end,
            properties: Some(Rc::clone(properties)),
            revision: None,
            hyperlink: None,
            inline_object: None,
        })
        .collect()
    }

    #[test]
    fn ruby_widens_its_base_and_raises_the_line_ascent() {
        // Word for Mac (ruby2 probe): 漢字 under the 33pt guide かんじかんじ
        // takes the guide's width; distributeSpace gives each base character
        // 6pt, half of it before the first.
        let text = "か漢字さ";
        let runs = ruby_runs(text, 3..9, "かんじかんじ", "distributespace");
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&EmWidthContext, text, &runs, 0.0, &paragraph, 0, 400.0)
            .expect("wrap");
        assert_eq!(lines.len(), 1);
        let pt = |px: f64| px * 72.0 / 96.0;
        let clusters = &lines[0].clusters;
        let widths: Vec<f64> = clusters.iter().map(|c| pt(c.width)).collect();
        let expected = [10.5, 3.0 + 10.5 + 6.0, 10.5 + 3.0, 10.5];
        for (width, expected) in widths.iter().zip(expected) {
            assert!((width - expected).abs() < 1e-6, "{widths:?}");
        }
        let layout = |index: usize| clusters[index].layout.as_ref().unwrap();
        assert!((pt(layout(1).leading_advance) - 3.0).abs() < 1e-6);
        assert!(pt(layout(2).leading_advance).abs() < 1e-6);
        // The guide baseline is 10pt up (hpsRaise 21) and its 5.5pt face
        // reaches 0.859 em above that.
        let ruby_ascent = 10.0 + 5.5 * 0.859;
        for index in [1, 2] {
            let ascent = pt(layout(index).font_ascent.unwrap());
            assert!((ascent - ruby_ascent).abs() < 1e-6, "{ascent}");
        }
        assert!((pt(layout(0).font_ascent.unwrap()) - 10.5 * 0.859).abs() < 1e-6);
    }

    #[test]
    fn ruby_base_spreads_across_formatting_runs() {
        // 漢 and 字 are separate runs of one base: the guide's width spreads
        // over both, as if they were one run.
        let text = "か漢字さ";
        let mut runs = ruby_runs(text, 3..9, "かんじかんじ", "distributespace");
        let mut first = runs[1].properties.as_deref().unwrap().clone();
        first.bold = Some(true);
        let mut second = runs[1].properties.as_deref().unwrap().clone();
        second.ruby.as_mut().unwrap().continuation = true;
        runs[1].end = 6;
        runs[1].properties = Some(Rc::new(first));
        runs.insert(
            2,
            RunStyle {
                start: 6,
                end: 9,
                properties: Some(Rc::new(second)),
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
        );
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&EmWidthContext, text, &runs, 0.0, &paragraph, 0, 400.0)
            .expect("wrap");
        let widths: Vec<f64> = lines[0]
            .clusters
            .iter()
            .map(|c| c.width * 72.0 / 96.0)
            .collect();
        let expected = [10.5, 3.0 + 10.5 + 6.0, 10.5 + 3.0, 10.5];
        for (width, expected) in widths.iter().zip(expected) {
            assert!((width - expected).abs() < 1e-6, "{widths:?}");
        }
    }

    #[test]
    fn ruby_narrower_than_its_base_leaves_the_base_alone() {
        let text = "か東京都特許さ";
        let runs = ruby_runs(text, 3..18, "とう", "distributespace");
        let lines = wrap_text_into_lines(
            &EmWidthContext,
            text,
            &runs,
            0.0,
            &ParagraphStyle::default(),
            0,
            400.0,
        )
        .expect("wrap");
        for cluster in &lines[0].clusters {
            assert!((cluster.width * 72.0 / 96.0 - 10.5).abs() < 1e-6);
            assert!(cluster.layout.as_ref().unwrap().leading_advance.abs() < 1e-9);
        }
    }

    #[test]
    fn wrap_text_applies_fit_text_as_effective_text_scale() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        let text = "abcd";
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: Some(Rc::new(RunProperties {
                fit_text: Some(RunFitText {
                    val: 300,
                    id: Some(50),
                }),
                ..RunProperties::default()
            })),
            revision: None,
            hyperlink: None,
            inline_object: None,
        };
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 25.0).expect("wrap");

        assert_eq!(lines.len(), 1);
        let segment = lines[0].segments.first().expect("segment");
        assert_eq!(segment.text, "abcd");
        let text_scale = segment
            .properties
            .as_ref()
            .and_then(|props| props.text_scale)
            .expect("effective text scale");
        assert!((text_scale - 50.0).abs() < 1e-6);
        assert_eq!(
            segment
                .properties
                .as_ref()
                .and_then(|props| props.fit_text.as_ref())
                .and_then(|fit| fit.id),
            Some(50)
        );
    }

    #[test]
    fn fit_text_id_shares_one_target_width_across_formatting_runs() {
        for id in [50, -931340544] {
            let ctx = ResolvingFixedWidthContext {
                width_per_grapheme: 10.0,
            };
            let make_fitted_run = |start, end, bold| RunStyle {
                start,
                end,
                properties: Some(Rc::new(RunProperties {
                    fit_text: Some(RunFitText {
                        val: 300,
                        id: Some(id),
                    }),
                    bold: Some(bold),
                    ..RunProperties::default()
                })),
                revision: None,
                hyperlink: None,
                inline_object: None,
            };
            let runs = [make_fitted_run(0, 2, false), make_fitted_run(2, 4, true)];
            let lines = wrap_text_into_lines(
                &ctx,
                "abcd",
                &runs,
                0.0,
                &ParagraphStyle::default(),
                0,
                25.0,
            )
            .expect("wrap");

            assert_eq!(lines.len(), 1, "the complete 20px fitText group must fit");
            assert_eq!(lines[0].segments.len(), 2);
            for (index, segment) in lines[0].segments.iter().enumerate() {
                let properties = segment.properties.as_ref().expect("properties");
                assert_eq!(properties.text_scale, Some(50.0));
                assert_eq!(properties.bold, Some(index == 1));
            }
        }
    }

    #[test]
    fn mac_fitted_expansion_preserves_natural_glyphs_and_stops_at_group_end() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        for tracking in [-1.5, 3.0] {
            for suffix in ["", "Z", "\tZ"] {
                let text = format!("abcd{suffix}");
                let props = Rc::new(RunProperties {
                    fit_text: Some(RunFitText {
                        val: 900,
                        id: Some(-50),
                    }),
                    char_spacing: Some(tracking),
                    bold: Some(true),
                    ..RunProperties::default()
                });
                let mut runs = vec![
                    RunStyle {
                        start: 0,
                        end: 2,
                        properties: Some(props.clone()),
                        ..make_run("ab")
                    },
                    RunStyle {
                        start: 2,
                        end: 4,
                        properties: Some(props.clone()),
                        ..make_run("cd")
                    },
                ];
                if !suffix.is_empty() {
                    runs.push(RunStyle {
                        start: 4,
                        end: text.len(),
                        ..make_run(suffix)
                    });
                }
                let lines =
                    wrap_text_into_lines(&ctx, &text, &runs, 0.0, &paragraph, 17, 200.0).unwrap();
                assert_eq!(lines.len(), 1);
                let fitted = &lines[0].clusters[..4];
                assert!((fitted.iter().map(|c| c.width).sum::<f64>() - 60.0).abs() < 1e-5);
                for c in fitted {
                    assert!((c.layout.as_ref().unwrap().adjusted_glyph_width - 10.0).abs() < 1e-5,
                        "expansion preserves the natural glyph, tracking={tracking}, suffix={suffix:?}");
                }
                assert!((fitted[3].width - 10.0).abs() < 1e-5, "no terminal gap");
                assert_eq!(fitted[0].start, 17);
                assert_eq!(fitted[3].end, 21);
                assert_eq!(props.char_spacing, Some(tracking));
                if suffix == "Z" {
                    assert!(
                        (lines[0].clusters.iter().map(|c| c.width).sum::<f64>() - 70.0).abs()
                            < 1e-5
                    );
                }
                assert_eq!(collect_line_infos(&lines)[0].text, text.replace('\t', ""));
                if suffix.starts_with('\t') {
                    let tab = &lines[0].clusters[4];
                    assert_eq!(tab.kind, InlineClusterKind::Tab);
                    assert_eq!((tab.start, tab.end), (21, 22));
                }
            }
        }
    }

    #[test]
    fn mac_fitted_condensation_ignores_tracking_and_retains_effective_styles() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        for tracking in [-1.5, 3.0] {
            let text = "ab cd";
            let props = Rc::new(RunProperties {
                fit_text: Some(RunFitText {
                    val: 300,
                    id: Some(-50),
                }),
                char_spacing: Some(tracking),
                italic: Some(true),
                ..RunProperties::default()
            });
            let run = RunStyle {
                properties: Some(props.clone()),
                ..make_run(text)
            };
            let lines =
                wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 100.0).unwrap();
            assert_eq!(lines.len(), 1);
            assert!((lines[0].clusters.iter().map(|c| c.width).sum::<f64>() - 20.0).abs() < 1e-5);
            for segment in &lines[0].segments {
                let p = segment.properties.as_ref().unwrap();
                assert_eq!(p.char_spacing, Some(0.0));
                assert_eq!(p.italic, Some(true));
                assert!((p.text_scale.unwrap() - 40.0).abs() < 1e-5);
            }
            assert_eq!(props.char_spacing, Some(tracking));
        }
    }

    #[test]
    fn mac_fitted_natural_cjk_expansion_matches_original_gap_geometry() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 14.0,
        };
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let fit = RunFitText {
            val: 840,
            id: Some(-931340544),
        };
        let text = "床面積";
        let runs = [
            RunStyle {
                start: 0,
                end: 6,
                properties: Some(Rc::new(RunProperties {
                    fit_text: Some(fit.clone()),
                    char_spacing: Some(5.25),
                    // The original is 10.5pt: a 14px ＭＳ 明朝 cell.
                    font_size: Some(10.5),
                    ..RunProperties::default()
                })),
                ..make_run("床面")
            },
            RunStyle {
                start: 6,
                end: 9,
                properties: Some(Rc::new(RunProperties {
                    fit_text: Some(fit),
                    font_size: Some(10.5),
                    ..RunProperties::default()
                })),
                ..make_run("積")
            },
        ];
        let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 17, 100.0).unwrap();
        assert_eq!(lines.len(), 1);
        let c = &lines[0].clusters;
        assert_eq!(
            c.iter().map(|v| (v.start, v.end)).collect::<Vec<_>>(),
            vec![(17, 20), (20, 23), (23, 26)]
        );
        assert_eq!(
            c.iter().map(|v| v.width).collect::<Vec<_>>(),
            vec![21.0, 21.0, 14.0]
        );
        assert!(c
            .iter()
            .all(|v| v.layout.as_ref().unwrap().adjusted_glyph_width == 14.0));
        assert_eq!(
            runs[0].properties.as_ref().unwrap().char_spacing,
            Some(5.25)
        );
    }

    #[test]
    fn mac_fitted_rtl_expansion_keeps_gaps_between_visual_glyphs() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        let text = "אבגד";
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            bidi: true,
            ..ParagraphStyle::default()
        };
        let props = Rc::new(RunProperties {
            fit_text: Some(RunFitText {
                val: 900,
                id: Some(50),
            }),
            ..RunProperties::default()
        });
        let run = RunStyle {
            properties: Some(props),
            ..make_run(text)
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 100.0).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(collect_line_infos(&lines)[0].text, "דגבא");
        assert_eq!(
            (
                collect_line_infos(&lines)[0].start,
                collect_line_infos(&lines)[0].end
            ),
            (0, text.len())
        );
        let c = &lines[0].clusters;
        for glyph in &c[..3] {
            assert!(
                (glyph.width - (50.0 / 3.0)).abs() < 1e-5,
                "gap follows each visual interior glyph"
            );
        }
        assert!(
            (c[3].width - 10.0).abs() < 1e-5,
            "no gap after visual terminal glyph"
        );
        assert!((c.iter().map(|v| v.width).sum::<f64>() - 60.0).abs() < 1e-5);
        assert!(c
            .iter()
            .all(|v| v.layout.as_ref().unwrap().adjusted_glyph_width == 10.0));
    }

    #[test]
    fn mac_fitted_rtl_groups_keep_separate_targets_in_visual_order() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        let text = "אבגד הו";
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            bidi: true,
            ..ParagraphStyle::default()
        };
        let fitted = |start, end, val| RunStyle {
            start,
            end,
            properties: Some(Rc::new(RunProperties {
                fit_text: Some(RunFitText { val, id: Some(50) }),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let runs = [
            fitted(0, 8, 900),
            RunStyle {
                start: 8,
                end: 9,
                ..make_run(" ")
            },
            fitted(9, 13, 600),
        ];
        let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 17, 200.0).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(collect_line_infos(&lines)[0].text, "וה דגבא");
        let c = &lines[0].clusters;
        assert!((c[0].width - 30.0).abs() < 1e-5);
        assert!((c[1].width - 10.0).abs() < 1e-5);
        assert!((c[2].width - 10.0).abs() < 1e-5);
        for glyph in &c[3..6] {
            assert!((glyph.width - 50.0 / 3.0).abs() < 1e-5);
        }
        assert!((c[6].width - 10.0).abs() < 1e-5);
        assert!((c.iter().map(|v| v.width).sum::<f64>() - 110.0).abs() < 1e-5);
    }

    #[test]
    fn fitted_platform_and_ineligible_controls_preserve_preceding_properties() {
        let ctx = ResolvingFixedWidthContext {
            width_per_grapheme: 10.0,
        };
        for (platform, text, fit) in [
            (
                crate::line_metrics::WordLayoutPlatform::Windows,
                "abcd",
                Some(RunFitText {
                    val: 300,
                    id: Some(50),
                }),
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                "ab\ncd",
                Some(RunFitText {
                    val: 300,
                    id: Some(50),
                }),
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                "ab\rcd",
                Some(RunFitText {
                    val: 300,
                    id: Some(50),
                }),
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                "abcd",
                Some(RunFitText {
                    val: 0,
                    id: Some(50),
                }),
            ),
            (crate::line_metrics::WordLayoutPlatform::Mac, "abcd", None),
        ] {
            let props = Rc::new(RunProperties {
                fit_text: fit,
                char_spacing: Some(3.0),
                ..RunProperties::default()
            });
            let run = RunStyle {
                properties: Some(props.clone()),
                ..make_run(text)
            };
            let paragraph = ParagraphStyle {
                word_platform: platform,
                ..ParagraphStyle::default()
            };
            let lines =
                wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 100.0).unwrap();
            assert!(lines.iter().flat_map(|l| &l.segments).all(|s| s
                .properties
                .as_ref()
                .unwrap()
                .char_spacing
                == Some(3.0)));
            assert_eq!(props.char_spacing, Some(3.0));
        }
    }

    #[test]
    fn fit_text_control_scan_handles_utf8_and_all_ineligible_controls() {
        assert!(!contains_fit_text_control("横浜ab"));
        assert!(contains_fit_text_control("横浜\tab"));
        assert!(contains_fit_text_control("横浜\nab"));
        assert!(contains_fit_text_control("横浜\u{000c}ab"));
    }

    #[test]
    fn wrap_text_applies_pair_kerning_when_enabled() {
        let text = "AVX";
        let paragraph = ParagraphStyle::default();

        let no_kern_ctx = KerningAwareContext::new(false);
        let no_kern_lines = wrap_text_into_lines(
            &no_kern_ctx,
            text,
            &[make_run(text)],
            0.0,
            &paragraph,
            0,
            11.0,
        )
        .unwrap();
        assert_eq!(
            no_kern_lines[0]
                .segments
                .first()
                .map(|segment| segment.text.as_str()),
            Some("A")
        );

        let kern_ctx = KerningAwareContext::new(true);
        let kern_lines =
            wrap_text_into_lines(&kern_ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 11.0)
                .unwrap();
        assert_eq!(
            kern_lines[0]
                .segments
                .first()
                .map(|segment| segment.text.as_str()),
            Some("AV")
        );
    }

    #[test]
    fn character_border_inline_extent_keeps_right_and_center_tab_anchors() {
        use jade_docx_reader::document::BorderEdge;
        let text = "\t甲乙";
        let properties = Rc::new(RunProperties {
            border: Some(BorderEdge {
                val: Some("single".into()),
                size_eighths: Some(4),
                space: Some(0),
                color: None,
            }),
            ..RunProperties::default()
        });
        let runs = vec![
            RunStyle {
                end: 1,
                ..make_run(text)
            },
            RunStyle {
                start: 1,
                properties: Some(properties),
                ..make_run(text)
            },
        ];
        for kind in ["right", "center"] {
            let paragraph = ParagraphStyle {
                tab_stops: resolve_tab_stops(&[TabStop {
                    kind: Some(kind.into()),
                    position: Some(144.0),
                    leader: None,
                }]),
                ..ParagraphStyle::default()
            };
            let lines = wrap_text_into_lines(
                &TestContext::new(10.0),
                text,
                &runs,
                0.0,
                &paragraph,
                0,
                500.0,
            )
            .unwrap();
            let line = &lines[0];
            let text_width: f64 = line.clusters.iter().skip(1).map(|c| c.width).sum();
            let anchor =
                line.clusters[0].width + text_width * if kind == "right" { 1.0 } else { 0.5 };
            assert!(
                (anchor - points_to_px(144.0)).abs() < 1e-9,
                "{kind}: {anchor}"
            );
        }
    }

    #[test]
    fn character_border_inline_flag_preserves_cluster_allocation_size() {
        #[allow(dead_code)]
        struct PreviousInlineClusterLayout {
            script: ScriptCategory,
            font_slot: ResolvedFontSlot,
            leading_advance: f64,
            glyph_box_width: f64,
            measured_glyph_width: f64,
            adjusted_glyph_width: f64,
            font_descriptor_hash: u64,
            font_ascent: Option<f64>,
            font_descent: Option<f64>,
        }
        assert_eq!(
            std::mem::size_of::<InlineClusterLayout>(),
            std::mem::size_of::<PreviousInlineClusterLayout>()
        );
        assert_eq!(
            std::mem::size_of::<Option<InlineClusterLayout>>(),
            std::mem::size_of::<Option<PreviousInlineClusterLayout>>()
        );
    }

    #[test]
    fn character_border_inline_extent_merges_equal_runs_and_reserves_wrap_width() {
        use jade_docx_reader::document::BorderEdge;
        let border = Rc::new(RunProperties {
            border: Some(BorderEdge {
                val: Some("single".into()),
                size_eighths: Some(4),
                space: Some(0),
                color: Some("808080".into()),
            }),
            ..RunProperties::default()
        });
        let text = "甲乙丙丁";
        let runs = vec![
            RunStyle {
                end: 3,
                properties: Some(Rc::clone(&border)),
                ..make_run(text)
            },
            RunStyle {
                start: 3,
                properties: Some(border),
                ..make_run(text)
            },
        ];
        let ctx = TestContext::new(10.0);
        let paragraph = ParagraphStyle::default();
        let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 500.0).unwrap();
        assert_eq!(lines.len(), 1);
        let extent = 0.5 * 96.0 / 72.0;
        let width: f64 = lines[0].clusters.iter().map(|c| c.width).sum();
        assert!((width - (40.0 + 2.0 * extent)).abs() < 1e-9, "{width}");
        assert!(
            (lines[0].clusters[0]
                .layout
                .as_ref()
                .unwrap()
                .leading_advance
                - extent)
                .abs()
                < 1e-9
        );
        // A source-run boundary within the same border is not another box.
        assert_eq!(
            lines[0].clusters[1]
                .layout
                .as_ref()
                .unwrap()
                .leading_advance,
            0.0
        );
        let lines = wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 20.5).unwrap();
        assert_eq!(lines.len(), 4);
        for line in lines {
            assert_eq!(line.clusters.len(), 1);
            let width: f64 = line.clusters.iter().map(|c| c.width).sum();
            assert!((width - (10.0 + 2.0 * extent)).abs() < 1e-9, "{width}");
        }
    }

    #[test]
    fn character_border_inline_extent_uses_space_and_distinct_groups_but_ignores_nil() {
        use jade_docx_reader::document::BorderEdge;
        let text = "甲乙";
        for val in ["single", "nil", "none"] {
            let runs: Vec<_> = ["FF0000", "0000FF"]
                .into_iter()
                .enumerate()
                .map(|(i, color)| RunStyle {
                    start: i * 3,
                    end: (i + 1) * 3,
                    properties: Some(Rc::new(RunProperties {
                        border: Some(BorderEdge {
                            val: Some(val.into()),
                            size_eighths: Some(12),
                            space: Some(2),
                            color: Some(color.into()),
                        }),
                        ..RunProperties::default()
                    })),
                    ..make_run(text)
                })
                .collect();
            let lines = wrap_text_into_lines(
                &TestContext::new(10.0),
                text,
                &runs,
                0.0,
                &ParagraphStyle::default(),
                0,
                500.0,
            )
            .unwrap();
            let width: f64 = lines[0].clusters.iter().map(|c| c.width).sum();
            let expected = if val == "single" {
                20.0 + 4.0 * 3.5 * 96.0 / 72.0
            } else {
                20.0
            };
            assert!((width - expected).abs() < 1e-9, "{val}: {width}");
        }
    }

    #[test]
    fn tab_alignment_uses_word_pair_metrics_across_run_boundaries() {
        use crate::line_metrics::WordLayoutPlatform;
        // The unchanged Word 2col PDF supplies Yu Mincho W/o hmtx advances
        // (2013, 1096) and pair contraction (123), all in 2048 units/em.
        let descriptor = "10.50pt 'Yu Mincho', serif";
        let text = "\tWo";
        let expected_width = 14.0 * (2013.0 + 1096.0 - 123.0) / 2048.0;
        for split_runs in [false, true] {
            let runs = if split_runs {
                vec![
                    RunStyle {
                        end: 2,
                        ..make_run(text)
                    },
                    RunStyle {
                        start: 2,
                        ..make_run(text)
                    },
                ]
            } else {
                vec![make_run(text)]
            };
            for kind in ["right", "center"] {
                for host_adjustment in [0.0, -0.5, -2.0] {
                    let mut ctx = CountingKerningContext::new(true);
                    ctx.style = ResolvedRunStyle::new(
                        descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
                    );
                    ctx.pair_adjustment = host_adjustment;
                    let paragraph = ParagraphStyle {
                        word_platform: WordLayoutPlatform::Mac,
                        tab_stops: resolve_tab_stops(&[TabStop {
                            kind: Some(kind.to_string()),
                            position: Some(144.0),
                            leader: None,
                        }]),
                        ..ParagraphStyle::default()
                    };
                    let mut cache = FxHashMap::default();
                    let width =
                        measure_tab_segment(&ctx, text, &runs, 0, 1, 0, &paragraph, &mut cache)
                            .unwrap();
                    assert!(
                        (width - expected_width).abs() < 1e-9,
                        "{kind}, split={split_runs}, host={host_adjustment}: {width}"
                    );
                    let lines =
                        wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, 500.0).unwrap();
                    assert_eq!(lines.len(), 1);
                    let expected_start = points_to_px(144.0)
                        - expected_width * if kind == "right" { 1.0 } else { 0.5 };
                    assert!((lines[0].segments[0].leading_width - expected_start).abs() < 1e-9);
                    // Neither lookahead nor actual placement may measure a host
                    // pair that is already covered by requested Word metrics.
                    assert_eq!(*ctx.pair_measurements.borrow(), 0);
                }
            }
        }
    }

    #[test]
    fn tab_lookahead_keeps_cross_run_tracking_and_oracle_advances() {
        let text = "\tAV";
        let runs = vec![
            RunStyle {
                end: 2,
                ..make_run(text)
            },
            RunStyle {
                start: 2,
                ..make_run(text)
            },
        ];
        let mut ctx = CountingKerningContext::new(true);
        ctx.pair_adjustment = -2.0;
        ctx.style.letter_spacing_px = 0.75;
        let mut cache = FxHashMap::default();
        // Tracking applies across a run boundary, and tabs reset pair context.
        let width = measure_tab_segment(
            &ctx,
            text,
            &runs,
            0,
            1,
            0,
            &ParagraphStyle::default(),
            &mut cache,
        )
        .unwrap();
        assert!((width - 10.75).abs() < 1e-9, "{width}");
        assert_eq!(*ctx.pair_measurements.borrow(), 1);
        let paragraph = ParagraphStyle {
            oracle_advance_px_by_text_offset: Some(Rc::new(FxHashMap::from_iter([
                (101, 3.0),
                (102, 5.0),
            ]))),
            ..ParagraphStyle::default()
        };
        let width =
            measure_tab_segment(&ctx, text, &runs, 0, 101, 100, &paragraph, &mut cache).unwrap();
        assert_eq!(width, 8.0);
        assert_eq!(*ctx.pair_measurements.borrow(), 1);
    }

    #[test]
    fn measure_tab_segment_stops_at_line_break() {
        let ctx = TestContext::new(2.0);
        let text = "\tabc\nxyz";
        let runs = vec![make_run(text)];
        let paragraph = ParagraphStyle::default();
        let mut snap_cache = FxHashMap::default();
        let width =
            measure_tab_segment(&ctx, text, &runs, 0, 1, 0, &paragraph, &mut snap_cache).unwrap();
        assert_eq!(width, 3.0 * 2.0);
    }

    #[test]
    fn tab_gap_becomes_next_segment_leading_width() {
        let ctx = TestContext::new(10.0);
        let text = "左\t中央\t右";
        let paragraph = ParagraphStyle {
            tab_stops: resolve_tab_stops(&[
                TabStop {
                    kind: Some("left".to_string()),
                    position: Some(36.0),
                    leader: None,
                },
                TabStop {
                    kind: Some("center".to_string()),
                    position: Some(144.0),
                    leader: None,
                },
            ]),
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 500.0)
            .expect("wrap");
        let line = &lines[0];

        assert_eq!(line.segments.len(), 3);
        assert_eq!(line.segments[0].text, "左");
        assert_eq!(line.segments[1].text, "中央");
        assert_eq!(line.segments[2].text, "右");

        let left_tab_x = points_to_px(36.0);
        let center_tab_x = points_to_px(144.0);
        assert!((line.segments[1].leading_width - (left_tab_x - 10.0)).abs() < 1e-6);
        assert!((line.segments[2].leading_width - (center_tab_x - 5.0 - 68.0)).abs() < 1e-6);

        let mut cursor = 0.0;
        cursor += line.segments[0].leading_width;
        assert!((cursor - 0.0).abs() < 1e-6);
        cursor += 10.0 + line.segments[1].leading_width;
        assert!((cursor - left_tab_x).abs() < 1e-6);
        cursor += 20.0 + line.segments[2].leading_width;
        assert!((cursor - (center_tab_x - 5.0)).abs() < 1e-6);
    }

    #[test]
    fn header_tabbed_segment_reflows_together_at_the_declared_stop() {
        for fixture in [
            "header-tab-overflow-long.docx",
            "header-tab-overflow-short.docx",
        ] {
            let (document, settings) =
                load_document_part_with_settings(fixture, "word/header2.xml")
                    .expect("load header fixture");
            let paragraph = document
                .body
                .as_ref()
                .and_then(|body| body.elements.iter().find_map(BodyElement::as_paragraph))
                .expect("header paragraph");
            let mut text = String::new();
            let mut runs = Vec::new();
            for run in &paragraph.runs {
                let start = text.len();
                text.push_str(&run.text);
                runs.push(RunStyle {
                    start,
                    end: text.len(),
                    properties: run.properties.clone().map(Rc::new),
                    revision: run.revision.clone(),
                    hyperlink: run.hyperlink.clone(),
                    inline_object: None,
                });
            }
            let east_asian_rules =
                EastAsianLineBreakingRules::from_settings(&settings.to_line_breaking_settings());
            let paragraph_style = ParagraphStyle {
                tab_stops: paragraph
                    .properties
                    .as_ref()
                    .map(|props| resolve_tab_stops(&props.tabs))
                    .unwrap_or_default(),
                default_tab_width: points_to_px(settings.default_tab_stop.expect("default tab")),
                enable_line_break_rules: east_asian_rules.use_east_asian_layout,
                east_asian_rules,
                ..ParagraphStyle::default()
            };

            let lines = wrap_text_into_lines(
                // Linux CI falls back from Yu Mincho to a narrower font. Use
                // that narrow advance with the fixture's real 8504-twip text
                // width so the regression cannot depend on host fonts.
                &VariableWidthContext::new(9.0, 14.0),
                &text,
                &runs,
                0.0,
                &paragraph_style,
                0,
                points_to_px(425.2),
            )
            .expect("wrap header paragraph");
            let line_texts = lines
                .iter()
                .map(|line| {
                    line.segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            let expected_first_line = text.split_once('\t').expect("fixture tab").0;

            assert_eq!(
                line_texts,
                vec![
                    expected_first_line.to_string(),
                    "BBBBBBBBBBBBBBBBBB".to_string()
                ],
                "tabbed text must move as one segment in {fixture}"
            );
            assert_eq!(lines[0].tab_stops.len(), 0, "tab stays off line 1");
            assert_eq!(lines[1].tab_stops.len(), 1, "tab moves to line 2");
            assert!(
                (lines[1].segments[0].leading_width - points_to_px(169.5)).abs() < 1e-6,
                "line 2 text must retain the 3390-twip tab position in {fixture}: actual={}",
                lines[1].segments[0].leading_width,
            );
        }
    }

    #[test]
    fn hanging_indent_creates_implicit_tab_stop_before_distant_custom_stop() {
        let ctx = TestContext::new(10.0);
        let text = "1．\t本文";
        let paragraph = ParagraphStyle {
            first_line_indent: -points_to_px(31.45),
            tab_stops: resolve_tab_stops(&[TabStop {
                kind: Some("left".to_string()),
                position: Some(453.5),
                leader: None,
            }]),
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            points_to_px(31.5),
            &paragraph,
            0,
            700.0,
        )
        .expect("wrap hanging-indent paragraph");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].segments.len(), 2);
        assert_eq!(lines[0].segments[0].text, "1．");
        assert_eq!(lines[0].segments[1].text, "本文");
        assert!((lines[0].segments[1].leading_width - (points_to_px(31.45) - 20.0)).abs() < 1e-6);
    }

    #[test]
    fn custom_tab_stop_before_the_hanging_indent_wins() {
        // trust-beneficiary-sale-keep-next `2．<TAB>信託不動産…`: w:ind left 630
        // hanging 629 with a left tab at 600; Word starts the text at the
        // 600 stop, 1.5pt before the indent's implicit stop.
        let ctx = TestContext::new(10.0);
        let text = "1．\t本文";
        let paragraph = ParagraphStyle {
            first_line_indent: -points_to_px(31.45),
            tab_origin_offset: -points_to_px(31.5),
            tab_stops: resolve_tab_stops(&[TabStop {
                kind: Some("left".to_string()),
                position: Some(30.0),
                leader: None,
            }]),
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            points_to_px(31.5),
            &paragraph,
            0,
            700.0,
        )
        .expect("wrap hanging-indent paragraph");
        // The line starts 0.05pt from the margin; the stop is 30pt from it.
        assert!(
            (lines[0].segments[1].leading_width - (points_to_px(30.0 - 0.05) - 20.0)).abs() < 1e-3,
            "{}",
            lines[0].segments[1].leading_width
        );
    }

    #[test]
    fn tabs_fixture_preserves_declared_tab_positions_in_layout_segments() {
        let (document, _) =
            load_document_with_settings_and_style_defaults("tabs.docx").expect("fixture");
        let body = document.body.as_ref().expect("body");
        let paragraphs = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        let ctx = TestContext::new(10.0);

        let wrap_fixture_paragraph = |paragraph: &Paragraph| {
            let mut text = String::new();
            let mut runs = Vec::new();
            for run in &paragraph.runs {
                if run.text.is_empty() {
                    continue;
                }
                let start = text.len();
                text.push_str(&run.text);
                let end = text.len();
                runs.push(RunStyle {
                    start,
                    end,
                    properties: run.properties.clone().map(Rc::new),
                    revision: run.revision.clone(),
                    hyperlink: run.hyperlink.clone(),
                    inline_object: None,
                });
            }
            let paragraph_style = ParagraphStyle {
                tab_stops: paragraph
                    .properties
                    .as_ref()
                    .map(|props| resolve_tab_stops(&props.tabs))
                    .unwrap_or_default(),
                ..ParagraphStyle::default()
            };
            wrap_text_into_lines(&ctx, &text, &runs, 0.0, &paragraph_style, 0, 700.0)
                .expect("wrap fixture paragraph")
        };

        let first = &wrap_fixture_paragraph(paragraphs[0])[0];
        assert_eq!(
            first
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["左", "中央", "右"]
        );
        assert!((first.segments[1].leading_width - (points_to_px(36.0) - 10.0)).abs() < 1e-6);
        assert!(
            (first.segments[2].leading_width - (points_to_px(144.0) - 5.0 - 68.0)).abs() < 1e-6
        );

        let second = &wrap_fixture_paragraph(paragraphs[1])[0];
        assert_eq!(
            second
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["項目1", "10,000"]
        );
        assert_eq!(second.leaders.len(), 1);
        assert_eq!(second.leaders[0].leader, TabLeader::Dot);
        let value_start_x = points_to_px(400.0) - 60.0;
        assert!((second.segments[1].leading_width - (value_start_x - 30.0)).abs() < 1e-6);
        assert!((second.leaders[0].end_x - value_start_x).abs() < 1e-6);
        let leader = &second.leaders[0];
        let advance = ctx.measure_text(".", "12pt serif").unwrap();
        assert_eq!(leader.glyph_positions[0], leader.start_x);
        assert!(leader.glyph_positions.len() > 1);
        for pair in leader.glyph_positions.windows(2) {
            assert!((pair[1] - pair[0] - advance).abs() < 1e-6);
        }
        let last = *leader.glyph_positions.last().unwrap();
        assert!(last + advance <= leader.end_x + 0.5);
        assert!(last + 2.0 * advance > leader.end_x + 0.5);
    }

    #[test]
    fn mac_dot_leader_grid_keeps_original_word_counts_and_origins() {
        let aptos = 12.0 * 96.0 / 72.0 * 585.0 / 2048.0;
        for (start_pt, first_index, count) in [(92.68359375, 28, 127), (92.197265625, 27, 128)] {
            let positions = tab_leader_positions_with_grid(
                start_pt * 4.0 / 3.0,
                533.08984375 * 4.0 / 3.0,
                aptos,
                true,
            );
            assert_eq!(positions.len(), count);
            for (index, x) in positions.iter().enumerate() {
                assert!((x - (first_index + index) as f64 * aptos).abs() < 1e-9);
            }
        }
        let yu_mincho = 10.5 * 96.0 / 72.0 * 559.0 / 2048.0;
        let positions = tab_leader_positions_with_grid(
            101.316 * 4.0 / 3.0,
            440.679 * 4.0 / 3.0,
            yu_mincho,
            true,
        );
        assert_eq!(positions.len(), 117);
        assert!((positions[0] * 0.75 - 103.175).abs() < 0.001);
        assert!((positions.last().unwrap() * 0.75 - 435.628).abs() < 0.002);
        assert_eq!(
            &*tab_leader_positions_with_grid(11.5, 30.0, 4.0, false),
            &[11.5, 15.5, 19.5, 23.5]
        );
        for (start, end, advance) in [
            (f64::NAN, 30.0, 4.0),
            (0.0, f64::INFINITY, 4.0),
            (0.0, 30.0, 0.0),
        ] {
            assert!(tab_leader_positions_with_grid(start, end, advance, true).is_empty());
        }
    }

    #[test]
    fn tab_fixture_uses_document_default_tab_stop() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("tab.docx").expect("fixture");
        assert_eq!(settings.default_tab_stop, Some(42.0));
        let paragraph = document
            .body
            .as_ref()
            .and_then(|body| body.elements.iter().find_map(BodyElement::as_paragraph))
            .expect("paragraph");
        let mut text = String::new();
        let mut runs = Vec::new();
        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let start = text.len();
            text.push_str(&run.text);
            let end = text.len();
            runs.push(RunStyle {
                start,
                end,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }

        let paragraph_style = ParagraphStyle {
            default_tab_width: points_to_px(settings.default_tab_stop.unwrap()),
            ..ParagraphStyle::default()
        };
        let first = &wrap_text_into_lines(
            &TestContext::new(10.0),
            &text,
            &runs,
            0.0,
            &paragraph_style,
            0,
            700.0,
        )
        .expect("wrap tab fixture")[0];

        assert_eq!(
            first
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Hello", "World"]
        );
        assert!((first.segments[1].leading_width - (points_to_px(42.0) - 50.0)).abs() < 1e-6);
    }

    #[test]
    fn trust_true_sale_default_tab_grid_starts_at_text_margin_not_paragraph_indent() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("trust-beneficiary-sale-keep-next.docx")
                .expect("fixture");
        assert_eq!(settings.default_tab_stop, Some(42.0));
        let paragraph =
            find_paragraph_by_prefix(&document, "(10)\t真正譲渡").expect("find true-sale heading");
        let props = paragraph.properties.as_ref().expect("paragraph properties");
        let indent = props.indent.as_ref().expect("paragraph indent");
        assert_eq!(indent.left, Some(31.45));
        assert_eq!(indent.hanging, Some(21.0));
        assert!(props.tabs.is_empty());

        let mut text = String::new();
        let mut runs = Vec::new();
        for run in &paragraph.runs {
            let start = text.len();
            text.push_str(&run.text);
            runs.push(RunStyle {
                start,
                end: text.len(),
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }
        let mut base_x = 0.0;
        let mut paragraph_style = ParagraphStyle {
            default_tab_width: points_to_px(settings.default_tab_stop.unwrap()),
            ..ParagraphStyle::default()
        };
        apply_paragraph_indent(&mut base_x, &mut paragraph_style, indent, false);

        let lines = wrap_text_into_lines(
            &TestContext::new(10.0),
            &text,
            &runs,
            base_x,
            &paragraph_style,
            0,
            700.0,
        )
        .expect("wrap true-sale heading");
        let tab = lines[0].tab_stops.first().expect("tab layout");

        // With no custom w:tabs entry, Word advances to the first 840-twip
        // default stop measured from the text margin. The 629-twip paragraph
        // indent must not shift the entire default tab grid.
        assert!((tab.end_x - points_to_px(42.0)).abs() < 1e-6);
    }

    #[allow(clippy::too_many_arguments)]
    fn legacy_measure_tab_segment<C: LineWrapContext>(
        ctx: &C,
        text: &str,
        run_styles: &[RunStyle],
        start_run_index: usize,
        start_abs_offset: usize,
        base_offset: usize,
        paragraph_style: &ParagraphStyle,
        snap_to_chars_cell_width_cache: &mut FxHashMap<SnapToCharsCacheKey, f64>,
    ) -> Result<f64, C::Error> {
        let mut width = 0.0;
        let mut run_index = start_run_index;
        let mut cursor_rel = start_abs_offset.saturating_sub(base_offset);
        let mut prev_adjusted_glyph_width_across_runs: Option<f64> = None;
        let mut pair_cache = PairMeasureCache::default();

        while run_index < run_styles.len() {
            let run = &run_styles[run_index];
            if cursor_rel >= run.end {
                run_index += 1;
                continue;
            }

            let run_start = run.start;
            if cursor_rel < run_start {
                cursor_rel = run_start;
            }
            if cursor_rel >= run.end {
                run_index += 1;
                continue;
            }

            let run_start = run_start.min(text.len());
            let mut run_end = run.end.min(text.len());
            while run_end > run_start && !text.is_char_boundary(run_end) {
                run_end = run_end.saturating_sub(1);
            }
            let mut run_start_aligned = run_start;
            while run_start_aligned > 0 && !text.is_char_boundary(run_start_aligned) {
                run_start_aligned = run_start_aligned.saturating_sub(1);
            }
            let run_text = text.get(run_start_aligned..run_end).unwrap_or("");
            let initial_offset = cursor_rel - run_start_aligned;
            let style = ctx.resolve_run_style(run.properties.as_deref())?;
            let letter_spacing = style.letter_spacing_px;
            let text_scale_x = style.text_scale_x.max(0.01);
            let initial_offset = initial_offset.min(run_text.len());
            let mut initial_offset_aligned = initial_offset;
            while initial_offset_aligned > 0 && !run_text.is_char_boundary(initial_offset_aligned) {
                initial_offset_aligned = initial_offset_aligned.saturating_sub(1);
            }
            let prefix = run_text.get(..initial_offset_aligned).unwrap_or("");
            let mut char_index_in_run = prefix.graphemes(true).count();
            let mut prev_run_grapheme = prefix.graphemes(true).next_back();
            let mut prev_run_font_desc: Option<&str> = None;
            let mut prev_run_measured_width: Option<f64> = None;
            if let Some(prev) = prev_run_grapheme {
                let prev_script = classify_script(prev);
                let (prev_font_desc, prev_font_desc_hash, prev_font_desc_id) =
                    style.descriptor_key_for_grapheme(prev, prev_script);
                prev_run_font_desc = Some(prev_font_desc);
                prev_run_measured_width = Some(ctx.measure_text_with_font_key(
                    prev,
                    prev_font_desc,
                    prev_font_desc_hash,
                    prev_font_desc_id,
                )?);
                prev_adjusted_glyph_width_across_runs = prev_run_measured_width
                    .map(|value| adjusted_glyph_advance_width(prev_script, value, text_scale_x));
            }
            let mut active_script = ScriptCategory::Latin;

            let mut tab_grapheme_iter = crate::graphemes::grapheme_indices(run_text).peekable();
            while let Some((rel_offset, grapheme)) = tab_grapheme_iter.next() {
                if rel_offset < initial_offset {
                    continue;
                }
                if grapheme == "\r" {
                    continue;
                }
                if grapheme == "\n" || grapheme == PAGE_BREAK_STR || grapheme == "\t" {
                    return Ok(width);
                }
                let grapheme_start_abs = base_offset + run_start + rel_offset;
                let grapheme_end_abs = base_offset + run_start + rel_offset + grapheme.len();
                let prev_prev_grapheme = previous_previous_grapheme_at_absolute_offset(
                    text,
                    base_offset,
                    grapheme_start_abs,
                );
                let prev_grapheme =
                    previous_grapheme_at_absolute_offset(text, base_offset, grapheme_start_abs);
                let next_grapheme = match tab_grapheme_iter.peek() {
                    Some(&(_, next_g)) => Some(next_g),
                    None => next_grapheme_at_absolute_offset(text, base_offset, grapheme_end_abs),
                };
                let script = classify_script(grapheme);
                if script != active_script {
                    active_script = script;
                }
                let (font_desc, font_desc_hash, font_desc_id) =
                    style.descriptor_key_for_grapheme(grapheme, active_script);
                let measured_glyph_width = ctx.measure_text_with_font_key(
                    grapheme,
                    font_desc,
                    font_desc_hash,
                    font_desc_id,
                )?;
                let kerning_adjustment = if style.enable_pair_kerning
                    && char_index_in_run > 0
                    && prev_run_grapheme.is_some()
                    && prev_run_font_desc == Some(font_desc)
                {
                    let prev_grapheme = prev_run_grapheme.unwrap_or_default();
                    let prev_width =
                        prev_run_measured_width.unwrap_or(ctx.measure_text_with_font_key(
                            prev_grapheme,
                            font_desc,
                            font_desc_hash,
                            font_desc_id,
                        )?);
                    let pair_width = measure_text_pair(
                        ctx,
                        prev_grapheme,
                        grapheme,
                        font_desc,
                        font_desc_hash,
                        font_desc_id,
                        &mut pair_cache,
                    )?;
                    let raw_adjustment = pair_width - prev_width - measured_glyph_width;
                    if raw_adjustment.is_finite() {
                        raw_adjustment * text_scale_x
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let has_following_grapheme_in_run = run_text
                    .get(rel_offset + grapheme.len()..)
                    .and_then(|suffix| suffix.graphemes(true).next())
                    .is_some();
                let extra_spacing =
                    if letter_spacing.abs() > f64::EPSILON && has_following_grapheme_in_run {
                        letter_spacing
                    } else {
                        0.0
                    };
                let snap_cell_width = resolve_snap_to_chars_cell_width(
                    ctx,
                    paragraph_style,
                    &style.font_descriptor_east_asia,
                    style.font_descriptor_east_asia_hash,
                    text_scale_x,
                    snap_to_chars_cell_width_cache,
                )?;
                let advance = compute_grapheme_advance(GraphemeAdvanceContext {
                    paragraph_style: Some(paragraph_style),
                    grapheme,
                    script,
                    measured_glyph_width,
                    measured_width_is_oracle: false,
                    text_scale_x,
                    extra_spacing,
                    kerning_adjustment,
                    is_line_start: width <= f64::EPSILON,
                    prev_prev_grapheme,
                    prev_grapheme,
                    next_grapheme,
                    prev_glyph_width: prev_adjusted_glyph_width_across_runs,
                    snap_cell_width,
                    font_descriptor: None,
                    prev_auto_space: None,
                    run_letter_spacing: 0.0,
                });
                width += advance.total_advance;
                char_index_in_run += 1;
                prev_run_grapheme = Some(grapheme);
                prev_run_font_desc = Some(font_desc);
                prev_run_measured_width = Some(measured_glyph_width);
                prev_adjusted_glyph_width_across_runs = Some(advance.adjusted_glyph_width);
            }

            cursor_rel = run.end;
            run_index += 1;
        }

        Ok(width)
    }

    #[test]
    fn measure_tab_segment_matches_legacy_context_width() {
        let ctx = KerningAwareContext::new(true);
        let text = "\tAVAVAVAV日本語123,456終わり";
        let runs = vec![make_run(text)];
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                auto_space_de: Some(true),
                auto_space_dn: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let mut legacy_snap_cache = FxHashMap::default();
        let legacy = legacy_measure_tab_segment(
            &ctx,
            text,
            &runs,
            0,
            1,
            0,
            &paragraph,
            &mut legacy_snap_cache,
        )
        .expect("legacy tab segment width");
        let mut snap_cache = FxHashMap::default();
        let current = measure_tab_segment(&ctx, text, &runs, 0, 1, 0, &paragraph, &mut snap_cache)
            .expect("current tab segment width");

        assert!(
            (legacy - current).abs() < 1e-9,
            "legacy={legacy} current={current}"
        );
    }

    #[test]
    #[ignore]
    fn benchmark_measure_tab_segment_hot_path() {
        const ITERS: usize = 20_000;
        let ctx = KerningAwareContext::new(true);
        let body = "AVAVAVAV日本語123,456終わり".repeat(4);
        let text = format!("\t{body}");
        let runs = vec![make_run(&text)];
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                auto_space_de: Some(true),
                auto_space_dn: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let legacy_start = Instant::now();
        for _ in 0..ITERS {
            let mut snap_cache = FxHashMap::default();
            let width = legacy_measure_tab_segment(
                &ctx,
                &text,
                &runs,
                0,
                1,
                0,
                &paragraph,
                &mut snap_cache,
            )
            .expect("legacy tab benchmark");
            black_box((width, snap_cache));
        }
        let legacy_elapsed = legacy_start.elapsed();

        let current_start = Instant::now();
        for _ in 0..ITERS {
            let mut snap_cache = FxHashMap::default();
            let width =
                measure_tab_segment(&ctx, &text, &runs, 0, 1, 0, &paragraph, &mut snap_cache)
                    .expect("current tab benchmark");
            black_box((width, snap_cache));
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "measure_tab_segment benchmark: legacy={}us current={}us iterations={} graphemes={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
            body.graphemes(true).count(),
        );
    }

    #[test]
    fn small_kana_and_choon_start_a_line_unless_kinsoku_is_strict() {
        // Word's standard set (標準) leaves them out: Word for Mac starts
        // trust-beneficiary-sale-keep-next's page 12 line with `ーシング`.
        // `w:strictFirstAndLastChars` adds them.
        let mut paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        assert!(!paragraph.forbids_line_start("ぁ"));
        assert!(!paragraph.forbids_line_start("ッ"));
        assert!(!paragraph.forbids_line_start("ー"));
        assert!(paragraph.forbids_line_start("］"));
        assert!(!paragraph.forbids_line_start("あ"));
        paragraph.east_asian_rules.strict_first_and_last_chars = true;
        assert!(paragraph.forbids_line_start("ぁ"));
        assert!(paragraph.forbids_line_start("ッ"));
        assert!(paragraph.forbids_line_start("ー"));
        assert!(!paragraph.forbids_line_start("あ"));
    }

    #[test]
    fn implicit_numeric_gap_applies_for_cjk_adjacent_digits() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "4",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("第"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_side = (10.0 * ratio).max(min_px);
        assert!((extra - expected_side).abs() < 1e-9);
    }

    #[test]
    fn implicit_numeric_gap_uses_word_quarter_em_floor_for_narrow_digits() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        for auto_space_dn in [None, Some(true)] {
            let paragraph = ParagraphStyle {
                enable_line_break_rules: true,
                east_asian_rules: EastAsianLineBreakingRules {
                    auto_space_dn,
                    ..EastAsianLineBreakingRules::default()
                },
                ..ParagraphStyle::default()
            };
            let extra = implicit_numeric_east_asian_gap_width(
                &paragraph,
                ImplicitNumericGapContext {
                    grapheme: "4",
                    script: ScriptCategory::Latin,
                    glyph_width: 7.0,
                    prev_prev_grapheme: None,
                    prev_grapheme: Some("第"),
                    prev_glyph_width: None,
                    font_descriptor: None,
                    prev_auto_space: None,
                },
            );

            assert!((extra - 3.5).abs() < 1e-9);
        }
    }

    #[test]
    fn implicit_latin_gap_defaults_to_enabled_when_auto_space_de_missing() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "A",
            ScriptCategory::Latin,
            6.0,
            Some("第"),
            Some("項"),
            None,
        );
        assert!((leading - 3.5).abs() < 1e-9);
        assert!((trailing - 3.5 * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE).abs() < 1e-9);
    }

    #[test]
    fn implicit_latin_gap_respects_auto_space_de_disabled() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                auto_space_de: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "A",
            ScriptCategory::Latin,
            6.0,
            Some("第"),
            Some("項"),
            None,
        );
        assert_eq!(leading, 0.0);
        assert_eq!(trailing, 0.0);
    }

    #[test]
    fn east_asian_justification_widens_auto_space_before_characters() {
        assert_eq!(
            east_asian_justification_gap("前", "1"),
            Some(EastAsianJustificationGap::AutoSpace { em_share: 0.25 })
        );
        assert_eq!(east_asian_justification_gap("1", "2"), None);
        assert_eq!(
            east_asian_justification_gap("る", "。"),
            Some(EastAsianJustificationGap::InterCharacter)
        );
        // `…直前12か月間…` justified by 7.6pt at a 14px (10.5pt) em: the two
        // auto-space boundaries double, the rest goes between characters.
        let em = 14.0;
        let mut gaps = vec![(east_asian_justification_gap("前", "1"), em)];
        gaps.push((east_asian_justification_gap("1", "2"), em));
        gaps.push((east_asian_justification_gap("2", "か"), em));
        gaps.extend((0..40).map(|_| (Some(EastAsianJustificationGap::InterCharacter), em)));
        let extra = 7.6 * 96.0 / 72.0;
        let shares = distribute_east_asian_justification(&gaps, extra);
        assert!((shares[0] - 3.5).abs() < 1e-9);
        assert_eq!(shares[1], 0.0);
        assert!((shares[2] - 2.8).abs() < 1e-9);
        let per_char = (extra - 6.3) / 40.0;
        assert!((shares[3] - per_char).abs() < 1e-9);
        assert!((shares.iter().sum::<f64>() - extra).abs() < 1e-9);
        // A small stretch only widens the auto-space boundaries, in proportion.
        let shares = distribute_east_asian_justification(&gaps, 0.63);
        assert!((shares[0] - 0.35).abs() < 1e-9);
        assert_eq!(shares[3], 0.0);
    }

    #[test]
    fn word_for_windows_places_auto_space_on_whole_pixels() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        // 10.5pt (14px em): 1/4 em = 3.5px → 4px (3.0pt), 1/5 em = 2.8px → 3px
        // (2.25pt), as in the `yokohama_page1` Word COM trace.
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "A",
            ScriptCategory::Latin,
            6.0,
            Some("第"),
            Some("項"),
            None,
        );
        assert_eq!((leading, trailing), (4.0, 3.0));
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "A",
            ScriptCategory::Latin,
            6.0,
            Some("第"),
            Some("項"),
            None,
        );
        assert!((leading - 3.5).abs() < 1e-9 && (trailing - 2.8).abs() < 1e-9);
    }

    #[test]
    fn no_auto_space_after_east_asian_punctuation() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        // `本契約の有効期間は、2026年…`: Word leaves no gap after `、`.
        let after_comma = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "2",
                script: ScriptCategory::Latin,
                glyph_width: 7.0,
                prev_prev_grapheme: Some("は"),
                prev_grapheme: Some("、"),
                prev_glyph_width: Some(14.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert_eq!(after_comma, 0.0);
        let (leading, _) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "M",
            ScriptCategory::Latin,
            9.0,
            Some("「"),
            None,
            None,
        );
        assert_eq!(leading, 0.0);
        // `々` is an ideograph even though it lives in the punctuation block.
        let after_iteration_mark = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "2",
                script: ScriptCategory::Latin,
                glyph_width: 7.0,
                prev_prev_grapheme: Some("人"),
                prev_grapheme: Some("々"),
                prev_glyph_width: Some(14.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert!(after_iteration_mark > 0.0);
    }

    #[test]
    fn implicit_latin_gap_trailing_only_when_next_is_east_asia() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        // Latin letter with prev=Latin, next=EastAsia:
        // leading should be 0, trailing should be non-zero.
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "d",
            ScriptCategory::Latin,
            6.0,
            Some("c"),
            Some("に"),
            None,
        );
        let expected_side = (6.0 * constants::IMPLICIT_LATIN_GAP_SIDE_RATIO)
            .max(constants::IMPLICIT_LATIN_GAP_SIDE_MIN_PX)
            * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE;
        assert_eq!(leading, 0.0);
        assert!((trailing - expected_side).abs() < 1e-9);
    }

    #[test]
    fn implicit_latin_gap_leading_only_when_prev_is_east_asia() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        // Latin letter with prev=EastAsia, next=Latin:
        // leading should be non-zero, trailing should be 0.
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "a",
            ScriptCategory::Latin,
            6.0,
            Some("い"),
            Some("b"),
            None,
        );
        let expected_side = (6.0 * constants::IMPLICIT_LATIN_GAP_SIDE_RATIO)
            .max(constants::IMPLICIT_LATIN_GAP_SIDE_MIN_PX);
        assert!((leading - expected_side).abs() < 1e-9);
        assert_eq!(trailing, 0.0);
    }

    #[test]
    fn balanced_unknown_latin_face_keeps_word_quarter_em_at_the_cjk_boundary() {
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                balance_single_byte_double_byte_width: true,
                ..Default::default()
            },
            ..Default::default()
        };
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let (leading, trailing) = implicit_latin_east_asian_gap_width(
            &paragraph,
            "t",
            ScriptCategory::Latin,
            14.0 * 778.0 / 2048.0,
            None,
            Some("数"),
            Some("10.50pt 'Bookman Old Style', serif"),
        );
        assert_eq!(leading, 0.0);
        assert!((trailing - 3.5).abs() < 1e-9, "{trailing}");
        crate::line_metrics::set_word_layout_platform(
            crate::line_metrics::WordLayoutPlatform::Windows,
        );
    }

    #[test]
    fn balanced_bookman_digit_spacing_survives_a_run_boundary() {
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let ctx = TestContext::new(14.0);
        let style = ResolvedRunStyle::new(
            "10.50pt 'Bookman Old Style', serif",
            "10.50pt 'Bookman Old Style', serif",
            "10.50pt 'HG丸ｺﾞｼｯｸM-PRO', sans-serif",
            "10.50pt 'Bookman Old Style', serif",
            0.0,
            1.0,
            false,
            None,
        );
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                balance_single_byte_double_byte_width: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut cache = FxHashMap::default();
        let state = walk_text_advance_steps(
            &ctx,
            "0",
            &style,
            Some(&paragraph),
            &TextAdvanceState::default(),
            Some("の"),
            &mut cache,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(state.prev_auto_space.unwrap().trailing_gap_px(true), 3.5);
        walk_text_advance_steps(
            &ctx,
            "の",
            &style,
            Some(&paragraph),
            &state,
            None,
            &mut cache,
            |step| {
                assert_eq!(step.advance.implicit_numeric_gap, 3.5);
                assert_eq!(step.advance.total_advance, 17.5);
                Ok(())
            },
        )
        .unwrap();
        crate::line_metrics::set_word_layout_platform(
            crate::line_metrics::WordLayoutPlatform::Windows,
        );
    }

    #[test]
    fn punctuation_compression_adjustment_applies_for_compress_punctuation_setting() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                character_spacing_control: Some("compressPunctuation".to_string()),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let adjustment =
            punctuation_compression_adjustment(&paragraph, "、", ScriptCategory::EastAsia, 10.0);
        assert!((adjustment + 1.8).abs() < 1e-9);
    }

    #[test]
    fn punctuation_compression_adjustment_is_disabled_without_compress_punctuation() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                character_spacing_control: Some("doNotCompress".to_string()),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let adjustment =
            punctuation_compression_adjustment(&paragraph, "、", ScriptCategory::EastAsia, 10.0);
        assert_eq!(adjustment, 0.0);
    }

    #[test]
    fn line_initial_number_before_east_asian_text_is_not_a_marker() {
        let paragraph = ParagraphStyle {
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let clusters = vec![
            InlineCluster::grapheme(0, 1, 0.0, "2", false, false, false),
            InlineCluster::grapheme(1, 2, 0.0, "0", false, false, false),
            InlineCluster::grapheme(2, 3, 0.0, "2", false, false, false),
            InlineCluster::grapheme(3, 4, 0.0, "6", false, false, false),
        ];
        let ratio = marker_spacing_gap_ratio(&paragraph, &clusters, "年", ScriptCategory::EastAsia);
        assert_eq!(ratio, 0.0);
    }

    #[test]
    fn mac_sets_no_marker_gap_after_a_literal_parenthesized_number() {
        // trust-beneficiary-sale-keep-next page 5: Word for Mac 16.113 sets
        // `(8) 売主` with the ASCII space alone between `)` and `売`.
        let paragraph = ParagraphStyle {
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let mut clusters = vec![
            InlineCluster::grapheme(0, 1, 0.0, "(", false, false, false),
            InlineCluster::grapheme(1, 2, 0.0, "8", false, false, false),
            InlineCluster::grapheme(2, 3, 0.0, ")", false, false, false),
        ];
        assert_eq!(
            marker_spacing_gap_ratio(&paragraph, &clusters, "売", ScriptCategory::EastAsia),
            0.0
        );
        clusters.push(InlineCluster::grapheme(3, 4, 0.0, " ", true, false, false));
        assert_eq!(
            marker_spacing_gap_ratio(&paragraph, &clusters, "売", ScriptCategory::EastAsia),
            0.0
        );
    }

    #[test]
    fn marker_spacing_gap_is_small_after_fullwidth_space() {
        let paragraph = ParagraphStyle {
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let clusters = vec![
            InlineCluster::grapheme(0, 1, 0.0, "(", false, false, false),
            InlineCluster::grapheme(1, 2, 0.0, "1", false, false, false),
            InlineCluster::grapheme(2, 3, 0.0, ")", false, false, false),
            InlineCluster::grapheme(3, 6, 0.0, "　", true, false, false),
        ];
        let ratio = marker_spacing_gap_ratio(&paragraph, &clusters, "法", ScriptCategory::EastAsia);
        assert!(ratio.abs() < 1e-9);
    }

    #[test]
    fn implicit_numeric_gap_keeps_leading_boundary_for_counter_and_group_start() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let counter_extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "3",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("が"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let grouped_start_extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "1",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("が"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let grouped_extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "0",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some(","),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_side = (10.0 * ratio).max(min_px);
        assert!((counter_extra - expected_side).abs() < 1e-9);
        assert!((grouped_start_extra - expected_side).abs() < 1e-9);
        assert_eq!(grouped_extra, 0.0);
    }

    #[test]
    fn implicit_numeric_gap_applies_before_east_asian_after_digit() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "項",
                script: ScriptCategory::EastAsia,
                glyph_width: 12.0,
                prev_prev_grapheme: Some("第"),
                prev_grapheme: Some("3"),
                prev_glyph_width: Some(10.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_side =
            (10.0 * ratio).max(min_px) * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE;
        assert!((extra - expected_side).abs() < 1e-9);
    }

    #[test]
    fn mac_century_ascii_walk_keeps_word_widths_with_host_substitution() {
        use crate::line_metrics::WordLayoutPlatform;
        // Original Word PDFs use 1139/2048 em digits and 569/2048 em
        // periods, regardless of the host's substitute for Century.
        for (platform, descriptor, scale, requested_metrics) in [
            (WordLayoutPlatform::Mac, "10.50pt Century, serif", 1.0, true),
            (
                WordLayoutPlatform::Mac,
                "10.50pt 'Century', serif",
                0.8,
                true,
            ),
            (
                WordLayoutPlatform::Mac,
                "bold 10.50pt Century, serif",
                1.0,
                false,
            ),
            (
                WordLayoutPlatform::Mac,
                "italic 10.50pt Century, serif",
                1.0,
                false,
            ),
            (
                WordLayoutPlatform::Windows,
                "10.50pt Century, serif",
                1.0,
                false,
            ),
        ] {
            let ctx = CountingKerningContext::new(false);
            let style = ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, scale, false, None,
            );
            let paragraph = ParagraphStyle {
                word_platform: platform,
                ..ParagraphStyle::default()
            };
            let mut cache = FxHashMap::default();
            walk_text_advance_steps(
                &ctx,
                "2024.10q",
                &style,
                Some(&paragraph),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    let expected = if requested_metrics && step.grapheme != "q" {
                        14.0 * if step.grapheme == "." { 569.0 } else { 1139.0 } / 2048.0
                    } else {
                        6.0
                    } * scale;
                    assert!(
                        (step.advance.glyph_width - expected).abs() < 1e-9,
                        "{platform:?} {descriptor} {}: {} vs {expected}",
                        step.grapheme,
                        step.advance.glyph_width
                    );
                    Ok(())
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn mac_yu_mincho_ascii_advance_uses_proportional_requested_metrics() {
        use crate::line_metrics::WordLayoutPlatform;
        // Embedded hmtx facts agree across 38 unchanged Word PDF fixtures.
        // Deliberately wrong host widths exercise the font-substitution path.
        for (platform, oracle, descriptor, scale, use_word_metrics) in [
            (
                WordLayoutPlatform::Mac,
                false,
                "10.50pt 'Yu Mincho', serif",
                1.0,
                true,
            ),
            (
                WordLayoutPlatform::Mac,
                false,
                "bold 10.50pt '游明朝', serif",
                0.8,
                true,
            ),
            (
                WordLayoutPlatform::Windows,
                false,
                "10.50pt 'Yu Mincho', serif",
                1.0,
                false,
            ),
            (
                WordLayoutPlatform::Mac,
                true,
                "10.50pt 'Yu Mincho', serif",
                1.0,
                false,
            ),
            (
                WordLayoutPlatform::Mac,
                false,
                "10.50pt 'YuMincho-Demibold', serif",
                1.0,
                false,
            ),
        ] {
            let paragraph = ParagraphStyle {
                word_platform: platform,
                ..ParagraphStyle::default()
            };
            for (grapheme, font_units) in [
                ("H", 1686.0),
                ("e", 1004.0),
                ("l", 549.0),
                ("o", 1096.0),
                ("W", 2013.0),
                (" ", 532.0),
                ("1", 1110.0),
                (".", 559.0),
                ("q", 0.0),
            ] {
                let advance = compute_grapheme_advance(GraphemeAdvanceContext {
                    paragraph_style: Some(&paragraph),
                    grapheme,
                    script: ScriptCategory::Latin,
                    measured_glyph_width: 8.25,
                    measured_width_is_oracle: oracle,
                    text_scale_x: scale,
                    extra_spacing: 2.0,
                    kerning_adjustment: 0.0,
                    is_line_start: false,
                    prev_prev_grapheme: None,
                    prev_grapheme: None,
                    next_grapheme: None,
                    prev_glyph_width: None,
                    snap_cell_width: None,
                    font_descriptor: Some(descriptor),
                    prev_auto_space: None,
                    run_letter_spacing: 0.0,
                });
                let expected = if use_word_metrics && font_units > 0.0 {
                    14.0 * (font_units / 2048.0
                        + if descriptor.starts_with("bold ") {
                            0.02
                        } else {
                            0.0
                        })
                } else {
                    8.25
                } * scale;
                assert!(
                    (advance.glyph_width - expected).abs() < 1e-9,
                    "{platform:?} {descriptor} {grapheme}: {} vs {expected}",
                    advance.glyph_width
                );
                assert!((advance.total_advance - expected - 2.0).abs() < 1e-9);
            }
        }
    }

    /// ＭＳ 明朝 / ＭＳ ゴシック advances come from their full/half-em cells,
    /// never from the host's stand-in face (jlsi/jade#469).
    #[test]
    fn ms_fixed_pitch_advances_follow_their_cells_without_host_measurement() {
        let text = "日本ab ｱ。é";
        let wrap = |family: &str| {
            let descriptor: Rc<str> = Rc::from(format!("10.50pt '{family}', serif").as_str());
            let mut ctx = TestContext::new(9.0);
            ctx.style = ResolvedRunStyle::new(
                descriptor.clone(),
                descriptor.clone(),
                descriptor.clone(),
                descriptor,
                0.0,
                1.0,
                false,
                None,
            );
            let lines = wrap_text_into_lines(
                &ctx,
                text,
                &[make_run(text)],
                0.0,
                &ParagraphStyle::default(),
                0,
                1000.0,
            )
            .unwrap();
            assert_eq!(lines.len(), 1);
            let widths: Vec<f64> = lines[0].clusters.iter().map(|c| c.width).collect();
            (widths, ctx.recorded_measurements())
        };
        for family in ["MS Mincho", "ＭＳ ゴシック", "ＭＳ明朝", "MS-Gothic"] {
            let (widths, measured) = wrap(family);
            // 10.5pt = 14px: full cells 14, half cells 7; é is not a known
            // cell and keeps the stand-in's 9px.
            assert_eq!(
                widths,
                vec![14.0, 14.0, 7.0, 7.0, 7.0, 7.0, 14.0, 9.0],
                "{family}"
            );
            for grapheme in ["日", "本", "a", "b", " ", "ｱ", "。"] {
                assert!(
                    !measured.iter().any(|text| text == grapheme),
                    "{family}: {grapheme:?} was measured by the host: {measured:?}"
                );
            }
        }
        // The proportional siblings keep the host's widths.
        let (widths, _) = wrap("MS PMincho");
        assert_eq!(widths, vec![9.0; 8]);
    }

    #[test]
    fn mac_ms_mincho_ascii_advance_uses_requested_half_em_with_fallback_fonts() {
        use crate::line_metrics::WordLayoutPlatform;
        for (platform, oracle, descriptor, scale, expected) in [
            (
                WordLayoutPlatform::Mac,
                false,
                "10.50pt 'MS Mincho', serif",
                1.0,
                7.0,
            ),
            (
                WordLayoutPlatform::Mac,
                false,
                "bold italic 10.50pt 'ＭＳ 明朝', serif",
                0.8,
                5.824,
            ),
            (
                WordLayoutPlatform::Mac,
                false,
                "10.50pt 'MS PMincho', serif",
                1.0,
                8.25,
            ),
            (
                WordLayoutPlatform::Windows,
                false,
                "10.50pt 'MS Mincho', serif",
                1.0,
                8.25,
            ),
            (
                WordLayoutPlatform::Mac,
                true,
                "10.50pt 'MS Mincho', serif",
                1.0,
                8.25,
            ),
        ] {
            let paragraph = ParagraphStyle {
                word_platform: platform,
                ..ParagraphStyle::default()
            };
            for grapheme in ["1", "0", "(", ")", ",", "A", "i", " "] {
                let advance = compute_grapheme_advance(GraphemeAdvanceContext {
                    paragraph_style: Some(&paragraph),
                    grapheme,
                    script: ScriptCategory::Latin,
                    measured_glyph_width: 8.25,
                    measured_width_is_oracle: oracle,
                    text_scale_x: scale,
                    extra_spacing: 2.0,
                    kerning_adjustment: 0.0,
                    is_line_start: false,
                    prev_prev_grapheme: None,
                    prev_grapheme: None,
                    next_grapheme: None,
                    prev_glyph_width: None,
                    snap_cell_width: None,
                    font_descriptor: Some(descriptor),
                    prev_auto_space: None,
                    run_letter_spacing: 0.0,
                });
                assert!(
                    (advance.glyph_width - expected).abs() < 1e-9,
                    "{platform:?} {descriptor} {grapheme}"
                );
                assert!((advance.total_advance - expected - 2.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn mac_character_grid_widens_auto_spacing_by_a_quarter_of_its_extra_space() {
        // indent-with-grid (linesAndChars, 12pt MS Mincho on 20pt cells):
        // Word for Mac 16.113 sets `げ2` 25pt apart and `2文` 15pt, a quarter
        // of the 20pt cell (5pt) at each boundary instead of 3pt.
        let px = |pt: f64| pt * 96.0 / 72.0;
        let advance = |grid: Option<f64>, tracking: f64, grapheme: &str, prev: &str, next: &str| {
            let paragraph = ParagraphStyle {
                enable_line_break_rules: true,
                word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
                doc_grid_char_space_px: grid,
                ..ParagraphStyle::default()
            };
            let script = classify_script(grapheme);
            compute_grapheme_advance(GraphemeAdvanceContext {
                paragraph_style: Some(&paragraph),
                grapheme,
                script,
                measured_glyph_width: if script == ScriptCategory::EastAsia {
                    px(12.0)
                } else {
                    px(6.0)
                },
                measured_width_is_oracle: false,
                text_scale_x: 1.0,
                extra_spacing: 0.0,
                kerning_adjustment: 0.0,
                is_line_start: false,
                prev_prev_grapheme: None,
                prev_grapheme: Some(prev),
                next_grapheme: Some(next),
                prev_glyph_width: Some(px(12.0)),
                snap_cell_width: None,
                font_descriptor: Some("12.00pt 'MS Mincho'"),
                prev_auto_space: None,
                run_letter_spacing: tracking,
            })
        };
        let gap =
            |advance: GraphemeAdvance| advance.implicit_latin_gap + advance.implicit_numeric_gap;
        for (grapheme, prev, next) in [("2", "げ", "文"), ("文", "2", "字")] {
            let plain = gap(advance(None, 0.0, grapheme, prev, next));
            let grid = gap(advance(Some(px(8.0)), 0.0, grapheme, prev, next));
            assert!(plain > 0.0, "{grapheme}");
            assert!(
                (grid - plain - px(2.0)).abs() < 1e-6,
                "{grapheme}: {plain} -> {grid}"
            );
            // 3pt of grid space with −1pt tracking: (3 − 2) / 4.
            let tracked = gap(advance(Some(px(3.0)), px(-1.0), grapheme, prev, next));
            let untracked = gap(advance(None, px(-1.0), grapheme, prev, next));
            assert!(
                (tracked - untracked - px(0.25)).abs() < 1e-6,
                "{grapheme}: {untracked} -> {tracked}"
            );
        }
    }

    #[test]
    fn compute_grapheme_advance_includes_numeric_gap_after_digit() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let advance = compute_grapheme_advance(GraphemeAdvanceContext {
            paragraph_style: Some(&paragraph),
            grapheme: "項",
            script: ScriptCategory::EastAsia,
            measured_glyph_width: 10.0,
            measured_width_is_oracle: false,
            text_scale_x: 1.0,
            extra_spacing: 0.0,
            kerning_adjustment: 0.0,
            is_line_start: false,
            prev_prev_grapheme: Some("第"),
            prev_grapheme: Some("3"),
            next_grapheme: None,
            prev_glyph_width: Some(8.0),
            snap_cell_width: None,
            font_descriptor: None,
            prev_auto_space: None,
            run_letter_spacing: 0.0,
        });
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_gap =
            (8.0 * ratio).max(min_px) * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE;
        assert!((advance.implicit_numeric_gap - expected_gap).abs() < 1e-9);
        assert!(
            (advance.total_advance - (advance.implicit_numeric_gap + advance.glyph_box_width))
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn compute_grapheme_advance_applies_top_line_punctuation_adjustment() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                top_line_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let advance = compute_grapheme_advance(GraphemeAdvanceContext {
            paragraph_style: Some(&paragraph),
            grapheme: "。",
            script: ScriptCategory::EastAsia,
            measured_glyph_width: 10.0,
            measured_width_is_oracle: false,
            text_scale_x: 1.0,
            extra_spacing: 0.0,
            kerning_adjustment: 0.0,
            is_line_start: true,
            prev_prev_grapheme: None,
            prev_grapheme: None,
            next_grapheme: Some("法"),
            prev_glyph_width: None,
            snap_cell_width: None,
            font_descriptor: None,
            prev_auto_space: None,
            run_letter_spacing: 0.0,
        });
        let expected_glyph_width =
            adjusted_glyph_advance_width(ScriptCategory::EastAsia, 10.0, 1.0);
        assert!((advance.glyph_width - expected_glyph_width).abs() < 1e-9);
        assert!((advance.adjusted_glyph_width - expected_glyph_width * 0.5).abs() < 1e-9);
        assert!((advance.glyph_box_width - expected_glyph_width * 0.5).abs() < 1e-9);
    }

    #[test]
    fn compute_grapheme_advance_does_not_reapply_spacing_to_oracle_width() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                auto_space_de: Some(true),
                auto_space_dn: Some(true),
                character_spacing_control: Some("compressPunctuation".to_string()),
                top_line_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let advance = compute_grapheme_advance(GraphemeAdvanceContext {
            paragraph_style: Some(&paragraph),
            grapheme: "1",
            script: ScriptCategory::Latin,
            measured_glyph_width: 5.25,
            measured_width_is_oracle: true,
            text_scale_x: 1.23,
            extra_spacing: 0.0,
            kerning_adjustment: 0.0,
            is_line_start: false,
            prev_prev_grapheme: Some("A"),
            prev_grapheme: Some("A"),
            next_grapheme: Some("A"),
            prev_glyph_width: Some(13.5),
            snap_cell_width: None,
            font_descriptor: None,
            prev_auto_space: None,
            run_letter_spacing: 0.0,
        });

        // TRACE evidence: COM glyph advances in `tests/word-oracle/tabs`
        // already include Word's final spacing. Browser oracle replay must not
        // apply jade-docx-layout auto-spacing, balance, or scale corrections again,
        // otherwise right-tab glyph X/WIDTH double-count those adjustments.
        assert_eq!(advance.glyph_width, 5.25);
        assert_eq!(advance.adjusted_glyph_width, 5.25);
        assert_eq!(advance.implicit_latin_gap, 0.0);
        assert_eq!(advance.implicit_numeric_gap, 0.0);
        assert_eq!(advance.balance_extra, 0.0);
        assert_eq!(advance.glyph_box_width, 5.25);
        assert_eq!(advance.total_advance, 5.25);

        let punctuation = compute_grapheme_advance(GraphemeAdvanceContext {
            paragraph_style: Some(&paragraph),
            grapheme: "、",
            script: ScriptCategory::EastAsia,
            measured_glyph_width: 10.5,
            measured_width_is_oracle: true,
            text_scale_x: 1.0,
            extra_spacing: 0.0,
            kerning_adjustment: 0.0,
            is_line_start: true,
            prev_prev_grapheme: None,
            prev_grapheme: None,
            next_grapheme: Some("地"),
            prev_glyph_width: None,
            snap_cell_width: None,
            font_descriptor: None,
            prev_auto_space: None,
            run_letter_spacing: 0.0,
        });

        // TRACE evidence: `tests/word-oracle/yokohama_mini/word-trace.json`
        // records Japanese punctuation as final Word advances. Browser oracle
        // replay must not compress those values a second time.
        assert_eq!(punctuation.glyph_width, 10.5);
        assert_eq!(punctuation.adjusted_glyph_width, 10.5);
        assert_eq!(punctuation.glyph_box_width, 10.5);
        assert_eq!(punctuation.total_advance, 10.5);
    }

    #[test]
    fn text_advance_step_does_not_reapply_letter_spacing_to_oracle_width() {
        let ctx = KerningAwareContext::new(true);
        let style = ResolvedRunStyle::new(
            Rc::<str>::from("12pt Test"),
            Rc::<str>::from("12pt Test"),
            Rc::<str>::from("12pt Test"),
            Rc::<str>::from("12pt Test"),
            30.0,
            1.0,
            true,
            None,
        );
        let paragraph = ParagraphStyle {
            oracle_advance_px_by_text_offset: Some(Rc::new(FxHashMap::from_iter([(0, 39.0)]))),
            ..ParagraphStyle::default()
        };
        let mut pair_cache = PairMeasureCache::default();
        let state = TextAdvanceState {
            prev_grapheme: Some(CompactString::new("A")),
            prev_font_descriptor_hash: Some(style.font_descriptor_latin_ascii_hash),
            prev_raw_glyph_width: Some(6.0),
            ..TextAdvanceState::default()
        };

        let step = compute_text_advance_step_with_descriptor(
            &ctx,
            "H",
            &style,
            Some(&paragraph),
            &state,
            Some("e"),
            false,
            None,
            &mut pair_cache,
            ScriptCategory::Latin,
            &style.font_descriptor_latin_ascii,
            style.font_descriptor_latin_ascii_hash,
            style.font_descriptor_latin_ascii_id,
            Some(0),
        )
        .unwrap();

        // TRACE evidence: `tests/word-oracle/char-spacing/word-trace.json`
        // records null COM advances but reliable per-character X positions.
        // The browser oracle reconstructs those advances from X deltas, which
        // already include Word's explicit character spacing and kerning result.
        assert_eq!(step.measured_glyph_width, 39.0);
        assert_eq!(step.advance.glyph_box_width, 39.0);
        assert_eq!(step.advance.kerning_adjustment, 0.0);
        assert_eq!(step.advance.total_advance, 39.0);
    }

    #[test]
    fn wrap_keeps_zero_width_oracle_glyph_at_line_end() {
        let ctx = TestContext::new(10.0);
        let text = "ab";
        let runs = vec![RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];
        let paragraph_style = ParagraphStyle {
            oracle_advance_px_by_text_offset: Some(Rc::new(FxHashMap::from_iter([
                (0, 10.0),
                (1, 0.0),
            ]))),
            ..ParagraphStyle::default()
        };

        let lines =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph_style, 0, 9.25).expect("wrap");
        let first_line_text: String = lines[0]
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();

        // TRACE evidence: `tests/word-oracle/yokohama_mini/word-trace.json`
        // has a terminal glyph at the line end with no positive COM advance.
        // Browser oracle replay should keep that zero-width glyph on the line
        // instead of applying the normal wrap safety margin to it.
        assert_eq!(first_line_text, "ab");
    }

    #[test]
    fn malformed_run_offsets_use_the_aligned_slice_origin() {
        let ctx = TestContext::new(10.0);
        let text = "あb";
        let runs = vec![RunStyle {
            start: 1,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];

        let lines =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &ParagraphStyle::default(), 0, 100.0)
                .expect("wrap malformed offsets defensively");
        let segment = lines[0].segments.first().expect("wrapped segment");

        assert_eq!(segment.text, "あb");
        assert_eq!(segment.start, 0);
        assert_eq!(segment.end, text.len());
    }

    #[test]
    fn overflow_refit_does_not_remeasure_the_whole_line_per_moved_cluster() {
        let ctx = TestContext::new(1.0);
        let text = "a".repeat(100);
        let first = WrappedLine {
            segments: vec![TextSegment {
                text: text.clone(),
                start: 0,
                end: text.len(),
                properties: None,
                revision: None,
                hyperlink: None,
                leading_width: 0.0,
            }],
            break_kind: LineBreakKind::Soft,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: (0..text.len())
                .map(|index| {
                    InlineCluster::grapheme(index, index + 1, 1.0, "a", false, false, false)
                })
                .collect(),
        };
        let second = WrappedLine {
            segments: Vec::new(),
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: Vec::new(),
        };
        let mut lines = vec![first, second];
        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &ParagraphStyle::default(),
            10.0,
            &mut FxHashMap::default(),
        )
        .expect("refit overfull line");

        assert!(
            ctx.recorded_measurements().len() < 500,
            "refit should measure each line once, not once per moved cluster"
        );
    }

    #[test]
    fn implicit_numeric_gap_applies_same_gap_for_counter_suffix_boundary() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let plain_extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "項",
                script: ScriptCategory::EastAsia,
                glyph_width: 12.0,
                prev_prev_grapheme: Some("が"),
                prev_grapheme: Some("3"),
                prev_glyph_width: Some(10.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let counter_extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "月",
                script: ScriptCategory::EastAsia,
                glyph_width: 12.0,
                prev_prev_grapheme: Some("が"),
                prev_grapheme: Some("3"),
                prev_glyph_width: Some(10.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_side =
            (10.0 * ratio).max(min_px) * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE;
        assert!((plain_extra - expected_side).abs() < 1e-9);
        assert!((counter_extra - expected_side).abs() < 1e-9);
        assert!(counter_extra > 0.0);
        assert!((counter_extra - plain_extra).abs() < 1e-9);
    }

    #[test]
    fn implicit_numeric_gap_applies_after_decimal_fraction_before_east_asian_text() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "パ",
                script: ScriptCategory::EastAsia,
                glyph_width: 14.0,
                prev_prev_grapheme: Some("."),
                prev_grapheme: Some("6"),
                prev_glyph_width: Some(7.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(paragraph.east_asian_rules.auto_space_dn);
        let expected_gap =
            (7.0 * ratio).max(min_px) * constants::AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE;
        assert!((extra - expected_gap).abs() < 1e-9);
    }

    #[test]
    fn implicit_numeric_gap_is_zero_without_previous_digit_width_context() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "号",
                script: ScriptCategory::EastAsia,
                glyph_width: 12.0,
                prev_prev_grapheme: Some("項"),
                prev_grapheme: Some("4"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert_eq!(extra, 0.0);
    }

    #[test]
    fn implicit_numeric_gap_is_zero_between_adjacent_digits() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "6",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("1"),
                prev_glyph_width: Some(10.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert_eq!(extra, 0.0);
    }

    #[test]
    fn implicit_numeric_gap_ignores_fullwidth_space_boundary() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "　",
                script: ScriptCategory::EastAsia,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("4"),
                prev_glyph_width: Some(10.0),
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert_eq!(extra, 0.0);
    }

    #[test]
    fn implicit_numeric_gap_respects_auto_space_dn_disabled() {
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                auto_space_dn: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let extra = implicit_numeric_east_asian_gap_width(
            &paragraph,
            ImplicitNumericGapContext {
                grapheme: "2",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("第"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        assert_eq!(extra, 0.0);
    }

    #[test]
    fn implicit_numeric_gap_defaults_to_enabled_when_auto_space_dn_missing() {
        // Exact gap arithmetic (Word for Mac); Windows rounds it to pixels.
        crate::line_metrics::set_word_layout_platform(crate::line_metrics::WordLayoutPlatform::Mac);
        let missing = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let enabled = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                auto_space_dn: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let missing_extra = implicit_numeric_east_asian_gap_width(
            &missing,
            ImplicitNumericGapContext {
                grapheme: "2",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("第"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let enabled_extra = implicit_numeric_east_asian_gap_width(
            &enabled,
            ImplicitNumericGapContext {
                grapheme: "2",
                script: ScriptCategory::Latin,
                glyph_width: 10.0,
                prev_prev_grapheme: None,
                prev_grapheme: Some("第"),
                prev_glyph_width: None,
                font_descriptor: None,
                prev_auto_space: None,
            },
        );
        let (ratio, min_px) = numeric_auto_spacing_params(missing.east_asian_rules.auto_space_dn);
        let expected_side = (10.0 * ratio).max(min_px);
        assert!((missing_extra - expected_side).abs() < 1e-9);
        assert!((enabled_extra - expected_side).abs() < 1e-9);
        assert!((enabled_extra - missing_extra).abs() < 1e-9);
    }

    #[test]
    fn effective_default_font_hint_keeps_ansi_digits_out_of_numeric_auto_spacing() {
        let ctx = TestContext::new(10.0);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let properties = RunProperties {
            fonts: Some(RunFonts {
                hint: Some(FontHint::Default),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        let style = resolve_run_style_for_wrap(Some(&properties), None);
        let mut numeric_gap = 0.0;
        let mut cache = FxHashMap::default();

        walk_text_advance_steps(
            &ctx,
            "第562条",
            &style,
            Some(&paragraph),
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |step| {
                numeric_gap += step.advance.implicit_numeric_gap;
                Ok(())
            },
        )
        .expect("walk default-hint number run");

        assert_eq!(numeric_gap, 0.0);
    }

    #[test]
    fn word_for_mac_keeps_numeric_auto_spacing_for_default_font_hint() {
        let ctx = TestContext::new(10.0);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            word_platform: crate::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let properties = RunProperties {
            fonts: Some(RunFonts {
                hint: Some(FontHint::Default),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        let style = resolve_run_style_for_wrap(Some(&properties), None);
        let mut numeric_gap = 0.0;
        let mut cache = FxHashMap::default();

        walk_text_advance_steps(
            &ctx,
            "別紙7の",
            &style,
            Some(&paragraph),
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |step| {
                numeric_gap += step.advance.implicit_numeric_gap;
                Ok(())
            },
        )
        .expect("walk default-hint number run");

        assert!(numeric_gap > 0.0);
    }

    #[test]
    fn does_not_force_digit_and_month_together_after_wrap() {
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "期間が3".to_string(),
                    start: 0,
                    end: 4,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(0, 3, 0.0, "期", false, false, false),
                    InlineCluster::grapheme(3, 6, 0.0, "間", false, false, false),
                    InlineCluster::grapheme(6, 9, 0.0, "が", false, false, false),
                    InlineCluster::grapheme(9, 10, 0.0, "3", false, false, false),
                ],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "月を超える".to_string(),
                    start: 10,
                    end: 22,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(10, 13, 0.0, "月", false, false, false),
                    InlineCluster::grapheme(13, 16, 0.0, "を", false, false, false),
                ],
            },
        ];

        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        enforce_line_start_rules(&paragraph, &mut lines);

        let infos = collect_line_infos(&lines);
        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("期間が3")
        );
        assert_eq!(
            infos.get(1).map(|line| line.text.as_str()),
            Some("月を超える")
        );
    }

    #[test]
    fn hanging_punctuation_does_not_overflow_multiple_characters() {
        let ctx = TestContext::new(10.0);
        let text = "あああ）」";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 32.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("ああ"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("あ）」"));
    }

    #[test]
    fn hanging_punctuation_keeps_a_closing_run_with_its_body_character() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。）。";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 40.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("ああ"));
        assert_eq!(
            infos.get(1).map(|line| line.text.as_str()),
            Some("あ。）。")
        );
    }

    #[test]
    fn overflow_punctuation_off_keeps_forbidden_start_on_next_line() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("ああ"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("あ。"));
    }

    #[test]
    fn hanging_punctuation_keeps_kuten_on_same_line() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。い";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("あああ。")
        );
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("い"));
    }

    #[test]
    fn hanging_punctuation_leaves_the_next_line_full() {
        // The hung `。` takes no room from the next line: Word for Mac starts
        // it at `い` and fills it as it would any other line. The Windows
        // layout keeps the kinsoku repair it was calibrated with
        // (trust-beneficiary-sale-pagination).
        for (platform, expected) in [
            (
                crate::line_metrics::WordLayoutPlatform::Mac,
                ["あああ。", "いいい", "いう"],
            ),
            (
                crate::line_metrics::WordLayoutPlatform::Windows,
                WINDOWS_HANGING_LINES,
            ),
        ] {
            assert_eq!(
                hanging_punctuation_lines(platform),
                expected,
                "{platform:?}"
            );
        }
    }

    const WINDOWS_HANGING_LINES: [&str; 3] = ["あああ。", "いい", "いいう"];

    fn hanging_punctuation_lines(platform: crate::line_metrics::WordLayoutPlatform) -> Vec<String> {
        let ctx = TestContext::new(10.0);
        let text = "あああ。いいいいう";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            word_platform: platform,
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();
        collect_line_infos(&lines)
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn overflow_punctuation_language_tags_are_ascii_case_insensitive() {
        for (language, punctuation) in [
            ("JA-jp", "｡"),
            ("KO-kr", "℃"),
            ("ZH-tW", "。"),
            ("zh-HK", "。"),
            ("zh-HaNt", "。"),
            ("ZH-cn", ">"),
        ] {
            let properties = RunProperties {
                east_asia_language: Some(Rc::from(language)),
                ..RunProperties::default()
            };
            assert!(
                word_overflow_punctuation_set(Some(&properties))
                    .is_some_and(|set| set.contains(punctuation)),
                "expected {punctuation:?} to overflow for {language}"
            );
        }
    }

    #[test]
    fn char_set_contains_matches_str_contains_for_every_overflow_table() {
        for set in [
            WORD_OVERFLOW_PUNCT_JAPANESE,
            WORD_OVERFLOW_PUNCT_CHINESE_SIMPLIFIED,
            WORD_OVERFLOW_PUNCT_CHINESE_TRADITIONAL,
            WORD_OVERFLOW_PUNCT_KOREAN,
        ] {
            for character in set
                .chars()
                .chain(['a', 'あ', '。', '\u{3000}', '😀', '\u{0}'])
            {
                assert_eq!(
                    char_set_contains(set, character),
                    set.contains(character),
                    "{character:?} in {set:?}"
                );
            }
        }
    }

    #[test]
    fn omitted_overflow_punctuation_defaults_to_hanging() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。い";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: None,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("あああ。")
        );
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("い"));
    }

    #[test]
    fn omitted_language_does_not_guess_an_overflow_punctuation_table() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。い";
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: None,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &paragraph, 0, 35.0)
            .expect("wrap language-neutral run");

        assert_eq!(
            collect_line_infos(&lines)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>(),
            ["ああ".to_string(), "あ。い".to_string()]
        );
    }

    #[test]
    fn paragraph_terminal_punctuation_does_not_hang_past_the_text_extent() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                fonts: Some(RunFonts {
                    hint: Some(FontHint::EastAsia),
                    ..RunFonts::default()
                }),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: None,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();

        assert_eq!(
            collect_line_infos(&lines)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>(),
            ["ああ".to_string(), "あ。".to_string()]
        );
    }

    #[test]
    fn explicit_default_font_hint_does_not_disable_japanese_overflow_punctuation() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。い";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                fonts: Some(RunFonts {
                    hint: Some(FontHint::Default),
                    ..RunFonts::default()
                }),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: None,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();

        assert_eq!(
            collect_line_infos(&lines)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>(),
            ["あああ。".to_string(), "い".to_string()]
        );
    }

    #[test]
    fn japanese_ascii_closing_parenthesis_is_kinsoku_but_not_hanging_punctuation() {
        let ctx = TestContext::new(10.0);
        let text = "ああ(a)";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 45.0).unwrap();
        let infos = collect_line_infos(&lines);

        // MS-OE376 2.1.56 lists ASCII ')' for Chinese and Korean, but not
        // Japanese. Word therefore moves the complete parenthesized token to
        // the next line instead of hanging ')' past the Japanese text extent.
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("ああ"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("(a)"));
    }

    #[test]
    fn japanese_internal_kuten_uses_the_parent_run_language_table() {
        let ctx = TestContext::new(10.0);
        let text = "あああ。い";
        let run = RunStyle {
            properties: Some(Rc::new(RunProperties {
                east_asia_language: Some(Rc::from("ja-JP")),
                ..RunProperties::default()
            })),
            ..make_run(text)
        };
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let lines = wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 35.0).unwrap();

        assert_eq!(
            collect_line_infos(&lines)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>(),
            ["あああ。".to_string(), "い".to_string()]
        );
    }

    #[test]
    fn kinsoku_off_does_not_move_forbidden_start_character() {
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "あああ".to_string(),
                    start: 0,
                    end: 9,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![InlineCluster::grapheme(
                    0, 3, 0.0, "あ", false, false, false,
                )],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "。".to_string(),
                    start: 9,
                    end: 12,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![InlineCluster::grapheme(
                    9, 12, 0.0, "。", false, true, false,
                )],
            },
        ];
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                kinsoku: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        enforce_line_start_rules(&paragraph, &mut lines);
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("あああ"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("。"));
    }

    #[test]
    fn word_wrap_off_avoids_breaking_inside_ascii_word() {
        let ctx = TestContext::new(10.0);
        let text = "法ABCDEF";
        let run = make_run(text);

        let normal = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                word_wrap: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let normal_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &normal,
            0,
            45.0,
        )
        .expect("normal wrap");
        let normal_infos = collect_line_infos(&normal_lines);
        assert_eq!(
            normal_infos.first().map(|line| line.text.as_str()),
            Some("法ABC")
        );

        let keep_word = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                word_wrap: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let keep_word_lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &keep_word, 0, 45.0).expect("wrap");
        let keep_word_infos = collect_line_infos(&keep_word_lines);
        assert_eq!(
            keep_word_infos.first().map(|line| line.text.as_str()),
            Some("法")
        );
        assert_eq!(
            keep_word_infos.get(1).map(|line| line.text.as_str()),
            Some("ABCDEF")
        );
    }

    #[test]
    fn word_wrap_missing_defaults_to_allow_mid_word_break_for_east_asian_layout() {
        let ctx = TestContext::new(10.0);
        let text = "法ABCDEF";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 45.0).expect("wrap");
        let infos = collect_line_infos(&lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("法ABC"));
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("DEF"));
    }

    #[test]
    fn word_wrap_missing_still_prefers_spaces_for_latin_phrase_in_fe_layout() {
        let ctx = TestContext::new(10.0);
        let text = "Hello World Hello";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 145.0).expect("wrap");
        let infos = collect_line_infos(&lines);

        // `tests/word-oracle/2col/word-trace.json` is a FE-layout document
        // (`w:useFELayout`) whose Latin "Hello World" paragraph wraps at spaces,
        // not inside `World`/`Hello`. Keep the missing-`wordWrap` CJK behavior
        // above, but repair Latin phrases to the available word boundary.
        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("Hello World ")
        );
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("Hello"));
    }

    #[test]
    fn latin_hyphen_opportunity_keeps_initial_numeric_and_nonbreaking_contexts() {
        let ctx = TestContext::new(10.0);
        for text in ["-bravo", "12-bravo", "alpha\u{2011}bravo"] {
            let lines = wrap_text_into_lines(
                &ctx,
                text,
                &[make_run(text)],
                0.0,
                &ParagraphStyle::default(),
                0,
                45.0,
            )
            .unwrap();
            // These delimiters were not a letter-letter internal hyphen.
            assert_eq!(
                collect_line_infos(&lines)[0].text,
                text.chars().take(4).collect::<String>()
            );
        }
        let mut lines = wrap_text_into_lines(
            &ctx,
            "alpha-bravo",
            &[make_run("alpha-bravo")],
            0.0,
            &ParagraphStyle::default(),
            0,
            85.0,
        )
        .unwrap();
        // Recreate the raw candidate split, including its trailing subword.
        lines[0].clusters.push(InlineCluster::grapheme(
            6, 7, 10.0, "b", false, false, false,
        ));
        lines[0].clusters.push(InlineCluster::grapheme(
            7, 8, 10.0, "r", false, false, false,
        ));
        assert!(has_preceding_latin_break_opportunity(&lines[0]));
        let hyphen = lines[0]
            .clusters
            .iter_mut()
            .find(|c| c.text == "-")
            .unwrap();
        hyphen.forbid_line_end = true;
        assert!(!has_preceding_latin_break_opportunity(&lines[0]));
    }

    #[test]
    fn latin_compound_word_prefers_existing_internal_hyphen_opportunity() {
        let ctx = TestContext::new(10.0);
        let text = "alpha-bravo";
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &ParagraphStyle::default(),
            0,
            85.0,
        )
        .unwrap();
        let infos = collect_line_infos(&lines);
        assert_eq!(infos[0].text, "alpha-");
        assert_eq!(infos[1].text, "bravo");
        let mut left = make_run(text);
        left.end = 6;
        let mut right = make_run(text);
        right.start = 6;
        right.properties = Some(Rc::new(RunProperties {
            color: Some(Rc::from("#FF0000")),
            ..RunProperties::default()
        }));
        let styled = wrap_text_into_lines(
            &ctx,
            text,
            &[left, right],
            0.0,
            &ParagraphStyle::default(),
            0,
            85.0,
        )
        .unwrap();
        assert_eq!(collect_line_infos(&styled)[0].text, "alpha-");
        assert_eq!(collect_line_infos(&styled)[1].text, "bravo");
        assert!(styled[1].segments.iter().all(|segment| segment
            .properties
            .as_ref()
            .and_then(|p| p.color.as_deref())
            == Some("#FF0000")));
    }

    #[test]
    fn normal_latin_paragraph_prefers_space_boundary_over_mid_word_break() {
        let ctx = TestContext::new(10.0);
        let text = "Hello World Hello";
        let run = make_run(text);
        let paragraph = ParagraphStyle::default();
        let lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &paragraph, 0, 115.0).expect("wrap");
        let infos = collect_line_infos(&lines);

        assert_eq!(
            infos.first().map(|line| line.text.as_str()),
            Some("Hello World ")
        );
        assert_eq!(infos.get(1).map(|line| line.text.as_str()), Some("Hello"));
    }

    #[test]
    fn latin_space_boundary_includes_existing_opening_punctuation_context() {
        let ctx = TestContext::new(10.0);
        for text in [
            "Alpha (Bracket) Omega",
            "Alpha [Bracket] Omega",
            "Alpha ((Bracket)) Omega",
        ] {
            let style = ParagraphStyle {
                enable_line_break_rules: true,
                east_asian_rules: EastAsianLineBreakingRules {
                    use_east_asian_layout: true,
                    no_line_break_after: if text.contains('[') {
                        vec!["[".into()]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                ..ParagraphStyle::default()
            };
            let lines =
                wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 115.0).unwrap();
            let infos = collect_line_infos(&lines);
            assert_eq!(infos[0].text, "Alpha ", "{text}");
            assert_eq!(
                infos
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                text
            );
            assert!(lines
                .iter()
                .flat_map(|line| &line.clusters)
                .filter(|cluster| cluster.text == "(" || cluster.text == "[")
                .all(|cluster| cluster.forbid_line_end));
            assert!(lines
                .iter()
                .all(|line| line.break_kind != LineBreakKind::Soft
                    || !line
                        .clusters
                        .last()
                        .is_some_and(|cluster| cluster.forbid_line_end)));
        }
    }

    #[test]
    fn opening_word_context_preserves_styled_ranges_and_long_word_progress() {
        let ctx = TestContext::new(10.0);
        let text = "Alpha (Bracket) Omega";
        let mut first = make_run(text);
        first.end = 7;
        first.properties = Some(Rc::new(RunProperties {
            color: Some("112233".into()),
            ..Default::default()
        }));
        let mut second = make_run(text);
        second.start = 7;
        second.properties = Some(Rc::new(RunProperties {
            color: Some("445566".into()),
            ..Default::default()
        }));
        let style = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..Default::default()
            },
            ..ParagraphStyle::default()
        };
        let lines =
            wrap_text_into_lines(&ctx, text, &[first, second], 0.0, &style, 17, 115.0).unwrap();
        assert_eq!(collect_line_infos(&lines)[0].text, "Alpha ");
        let clusters = lines
            .iter()
            .flat_map(|line| &line.clusters)
            .collect::<Vec<_>>();
        assert_eq!(clusters.first().unwrap().start, 17);
        assert_eq!(clusters.last().unwrap().end, 17 + text.len());
        assert!(clusters.windows(2).all(|pair| pair[0].end == pair[1].start));
        let opening = lines
            .iter()
            .flat_map(|line| &line.segments)
            .find(|segment| segment.text == "(")
            .unwrap();
        assert_eq!(
            opening.properties.as_ref().unwrap().color.as_deref(),
            Some("112233")
        );
        let long = "Alpha (Extraordinarilylongtoken) Omega";
        let wrapped =
            wrap_text_into_lines(&ctx, long, &[make_run(long)], 0.0, &style, 0, 115.0).unwrap();
        let reconstructed = collect_line_infos(&wrapped)
            .into_iter()
            .map(|line| line.text)
            .collect::<String>();
        assert_eq!(reconstructed, long);
        assert!(wrapped.iter().all(|line| !line.clusters.is_empty()));
    }

    #[test]
    fn top_line_punctuation_half_width_allows_more_glyphs_on_line_start() {
        let ctx = TestContext::new(10.0);
        let text = "、あああ";
        let run = make_run(text);

        let normal = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                top_line_punctuation: Some(false),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let normal_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &normal,
            0,
            29.0,
        )
        .expect("wrap");
        let normal_infos = collect_line_infos(&normal_lines);
        assert_eq!(
            normal_infos.first().map(|line| line.text.as_str()),
            Some("、あ")
        );

        let half = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                top_line_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let half_lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &half, 0, 29.0).expect("wrap");
        let infos = collect_line_infos(&half_lines);
        assert_eq!(infos.first().map(|line| line.text.as_str()), Some("、ああ"));
    }

    #[test]
    fn balance_single_byte_double_byte_width_wraps_earlier() {
        let ctx = TestContext::new(10.0);
        let text = "123456あ";
        let run = make_run(text);

        let normal = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let normal_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &normal,
            0,
            61.0,
        )
        .unwrap();
        assert_eq!(
            normal_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("123456")
        );

        let balanced = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let balanced_lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &balanced, 0, 61.0).unwrap();
        assert_eq!(
            balanced_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("1234")
        );
    }

    #[test]
    fn balance_single_byte_double_byte_width_handles_narrow_fallback_ascii() {
        let ctx = VariableWidthContext::new(6.66, 14.0);
        let text = "法(25226)とい";
        let run = make_run(text);

        let normal = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let normal_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &normal,
            0,
            90.95,
        )
        .unwrap();
        assert_eq!(
            normal_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("法(25226)とい")
        );

        let balanced = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let balanced_lines =
            wrap_text_into_lines(&ctx, text, &[run], 0.0, &balanced, 0, 90.95).unwrap();
        assert_eq!(
            balanced_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("法(25226)と")
        );
    }

    #[test]
    fn compat_use_fe_layout_and_balance_flags_change_wrap_result() {
        let ctx = TestContext::new(10.0);
        let text = "123456あ";
        let run = make_run(text);

        let style_from_settings = |settings: &DocumentSettings| {
            let mut style = ParagraphStyle::default();
            style.east_asian_rules =
                EastAsianLineBreakingRules::from_settings(&settings.to_line_breaking_settings());
            style.enable_line_break_rules = style.east_asian_rules.use_east_asian_layout;
            style
        };

        let compat_enabled_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat>
            <w:useFELayout/>
            <w:balanceSingleByteDoubleByteWidth/>
          </w:compat>
        </w:settings>
        "#;
        let compat_enabled =
            DocumentSettings::from_xml(compat_enabled_xml).expect("parse settings");
        let enabled_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &style_from_settings(&compat_enabled),
            0,
            61.0,
        )
        .unwrap();
        assert_eq!(
            enabled_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("1234")
        );

        let balance_off_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat>
            <w:useFELayout/>
            <w:balanceSingleByteDoubleByteWidth w:val="0"/>
          </w:compat>
        </w:settings>
        "#;
        let balance_off = DocumentSettings::from_xml(balance_off_xml).expect("parse settings");
        let balance_off_lines = wrap_text_into_lines(
            &ctx,
            text,
            std::slice::from_ref(&run),
            0.0,
            &style_from_settings(&balance_off),
            0,
            61.0,
        )
        .unwrap();
        assert_eq!(
            balance_off_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("123456")
        );

        let fe_layout_off_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat>
            <w:useFELayout w:val="false"/>
            <w:balanceSingleByteDoubleByteWidth/>
          </w:compat>
        </w:settings>
        "#;
        let fe_layout_off = DocumentSettings::from_xml(fe_layout_off_xml).expect("parse settings");
        let fe_layout_off_lines = wrap_text_into_lines(
            &ctx,
            text,
            &[run],
            0.0,
            &style_from_settings(&fe_layout_off),
            0,
            61.0,
        )
        .unwrap();
        assert_eq!(
            fe_layout_off_lines
                .first()
                .and_then(|line| line.segments.first())
                .map(|segment| segment.text.as_str()),
            Some("123456")
        );
    }

    #[test]
    fn line_info_wraps_first_article_at_ho_to_for_balanced_layout() {
        let ctx = VariableWidthContext::new(6.66, 14.0);
        let text = "第1条　市税及びその賦課徴収については、地方税法(昭和25年法律第226号。以下「法」という。)";
        let run = make_run(text);
        let paragraph = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };

        let mut matched: Option<Vec<LineInfo>> = None;
        for step in 4000..=9200 {
            let column_right_edge = step as f64 / 10.0;
            let lines = wrap_text_into_lines(
                &ctx,
                text,
                std::slice::from_ref(&run),
                0.0,
                &paragraph,
                0,
                column_right_edge,
            )
            .expect("wrap text");
            let infos = collect_line_infos(&lines);
            if infos.first().is_some_and(|first| {
                first
                    .text
                    .starts_with("第1条　市税及びその賦課徴収については、地方税法(")
                    && first.text.ends_with("以下「法」と")
            }) {
                matched = Some(infos);
                break;
            }
        }

        let infos = matched.expect("expected wrap boundary not found");
        let first = infos.first().expect("first line exists");
        assert!(first.text.ends_with("以下「法」と"));
        assert!(!first.text.ends_with("以下「法」とい"));
        assert!(first.width.is_finite());
        assert!(first.width > 0.0);
        assert_eq!(first.index, 0);
        assert!(first.end > first.start);
    }

    #[test]
    fn yokohama_page6_page3_last_line_matches_word_expectation() {
        let page_three_lines = page_three_lines_for_fixture(6.66, 11.2).expect("layout fixture");
        assert!(
            page_three_lines
                .iter()
                .all(|line| !line.contains("15条第3項又は第5項")),
            "unexpected page3 line includes paragraph that should spill to next page"
        );
    }

    #[test]
    fn yokohama_page6_page3_marker_line_wraps_at_jijitsuni() {
        let data = fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let package = executor::block_on(OpenXmlPackage::load(&data)).expect("load package");
        let xml = package.get("word/document.xml").expect("document.xml");
        let xml_text = String::from_utf8_lossy(xml);
        let document = DocumentParser::new()
            .parse_document_xml(xml_text.as_ref())
            .expect("parse document");
        let body = document.body.expect("body");
        let paragraph = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                let text = paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>();
                text.contains("該当する事実があること及びその該当する事実")
            })
            .expect("target paragraph");

        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let mut run_styles = Vec::new();
        let mut cursor = 0usize;
        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let start = cursor;
            cursor += run.text.len();
            run_styles.push(RunStyle {
                start,
                end: cursor,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }

        let mut base_x = 0.0;
        let mut paragraph_style = ParagraphStyle {
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        if let Some(props) = paragraph.properties.as_ref() {
            if let Some(indent) = props.indent.as_ref() {
                apply_paragraph_indent(
                    &mut base_x,
                    &mut paragraph_style,
                    indent,
                    props.numbering.is_some(),
                );
            }
        }

        let ctx = VariableWidthContext::new(6.66, 11.2);
        let mut matched = None;
        let mut sampled = Vec::new();
        for right in 300..=900 {
            let lines = wrap_text_into_lines(
                &ctx,
                &text,
                &run_styles,
                base_x,
                &paragraph_style,
                0,
                right as f64,
            )
            .expect("wrap");
            let infos = collect_line_infos(&lines);
            if let Some(first) = infos.first() {
                if sampled.len() < 14 && right % 40 == 0 {
                    sampled.push(format!("{right}: {}", first.text));
                }
                if first.text.starts_with("(1)　法第15条第1項各号")
                    && first.text.ends_with("該当する事実に")
                {
                    matched = Some(first.text.clone());
                    break;
                }
            }
        }

        let first_line = matched.unwrap_or_else(|| {
            panic!(
                "failed to find marker line wrapped at '事実に'; samples={}",
                sampled.join(" | ")
            )
        });
        assert!(!first_line.ends_with("該当する事実に基づ"));
    }

    #[test]
    fn yokohama_page6_page3_numbered_line_keeps_month_with_digit() {
        let data = fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let package = executor::block_on(OpenXmlPackage::load(&data)).expect("load package");
        let xml = package.get("word/document.xml").expect("document.xml");
        let xml_text = String::from_utf8_lossy(xml);
        let document = DocumentParser::new()
            .parse_document_xml(xml_text.as_ref())
            .expect("parse document");
        let body = document.body.expect("body");
        let paragraph = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                let text = paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>();
                text.starts_with("(6)　当該徴収の猶予を受けようとする金額")
            })
            .expect("target paragraph");

        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let mut run_styles = Vec::new();
        let mut cursor = 0usize;
        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let start = cursor;
            cursor += run.text.len();
            run_styles.push(RunStyle {
                start,
                end: cursor,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }

        let mut base_x = 0.0;
        let mut paragraph_style = ParagraphStyle {
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        if let Some(props) = paragraph.properties.as_ref() {
            if let Some(indent) = props.indent.as_ref() {
                apply_paragraph_indent(
                    &mut base_x,
                    &mut paragraph_style,
                    indent,
                    props.numbering.is_some(),
                );
            }
        }

        let ctx = VariableWidthContext::new(6.66, 11.2);
        let mut matched = None;
        for right in 300..=900 {
            let lines = wrap_text_into_lines(
                &ctx,
                &text,
                &run_styles,
                base_x,
                &paragraph_style,
                0,
                right as f64,
            )
            .expect("wrap");
            let infos = collect_line_infos(&lines);
            if let Some(first) = infos.first() {
                if first.text.contains("その期間が3月") && !first.text.contains("その期間が3月を")
                {
                    matched = Some(first.text.clone());
                    break;
                }
            }
        }
        assert!(matched.is_some(), "failed to keep 3月 together on (6) line");
    }

    #[test]
    fn yokohama_page6_page3_numbered_line4_keeps_month_with_digit() {
        let data = fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let package = executor::block_on(OpenXmlPackage::load(&data)).expect("load package");
        let xml = package.get("word/document.xml").expect("document.xml");
        let xml_text = String::from_utf8_lossy(xml);
        let document = DocumentParser::new()
            .parse_document_xml(xml_text.as_ref())
            .expect("parse document");
        let body = document.body.expect("body");
        let paragraph = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                let text = paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>();
                text.starts_with("(4)　当該徴収の猶予を受けようとする金額")
            })
            .expect("target paragraph");

        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let mut run_styles = Vec::new();
        let mut cursor = 0usize;
        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let start = cursor;
            cursor += run.text.len();
            run_styles.push(RunStyle {
                start,
                end: cursor,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: run.hyperlink.clone(),
                inline_object: None,
            });
        }

        let mut base_x = 0.0;
        let mut paragraph_style = ParagraphStyle {
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                balance_single_byte_double_byte_width: true,
                ..EastAsianLineBreakingRules::default()
            },
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        if let Some(props) = paragraph.properties.as_ref() {
            if let Some(indent) = props.indent.as_ref() {
                apply_paragraph_indent(
                    &mut base_x,
                    &mut paragraph_style,
                    indent,
                    props.numbering.is_some(),
                );
            }
        }

        let ctx = VariableWidthContext::new(6.66, 11.2);
        let mut matched = None;
        for right in 300..=900 {
            let lines = wrap_text_into_lines(
                &ctx,
                &text,
                &run_styles,
                base_x,
                &paragraph_style,
                0,
                right as f64,
            )
            .expect("wrap");
            let infos = collect_line_infos(&lines);
            if let Some(first) = infos.first() {
                if first.text.contains("その期間が3月") && !first.text.contains("その期間が3月を")
                {
                    matched = Some(first.text.clone());
                    break;
                }
            }
        }
        assert!(matched.is_some(), "failed to keep 3月 together on (4) line");
    }

    #[test]
    fn yokohama_page6_page3_line_breaks_match_word_for_collateral_and_reason_lines() {
        let data = fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let package = executor::block_on(OpenXmlPackage::load(&data)).expect("load package");
        let xml = package.get("word/document.xml").expect("document.xml");
        let xml_text = String::from_utf8_lossy(xml);
        let document = DocumentParser::new()
            .parse_document_xml(xml_text.as_ref())
            .expect("parse document");
        let body = document.body.expect("body");

        let collateral_paragraph = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .starts_with("(6)　当該徴収の猶予を受けようとする金額")
            })
            .expect("collateral paragraph");
        let reason_paragraph = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .find(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .starts_with("(1)　当該徴収の猶予に係る徴収金を一時に納付し、又は納入することができない事情の詳細")
            })
            .expect("reason paragraph");

        let build_styles = |paragraph: &Paragraph| {
            let mut text = String::new();
            let mut run_styles = Vec::new();
            for run in &paragraph.runs {
                if run.text.is_empty() {
                    continue;
                }
                let start = text.len();
                text.push_str(&run.text);
                let end = text.len();
                run_styles.push(RunStyle {
                    start,
                    end,
                    properties: run.properties.clone().map(Rc::new),
                    revision: run.revision.clone(),
                    hyperlink: run.hyperlink.clone(),
                    inline_object: None,
                });
            }
            let mut base_x = 0.0;
            let mut paragraph_style = ParagraphStyle {
                east_asian_rules: EastAsianLineBreakingRules {
                    use_east_asian_layout: true,
                    balance_single_byte_double_byte_width: true,
                    ..EastAsianLineBreakingRules::default()
                },
                enable_line_break_rules: true,
                ..ParagraphStyle::default()
            };
            if let Some(props) = paragraph.properties.as_ref() {
                if let Some(indent) = props.indent.as_ref() {
                    apply_paragraph_indent(
                        &mut base_x,
                        &mut paragraph_style,
                        indent,
                        props.numbering.is_some(),
                    );
                }
            }
            (text, run_styles, base_x, paragraph_style)
        };

        let (collateral_text, collateral_run_styles, collateral_base_x, collateral_style) =
            build_styles(collateral_paragraph);
        let (reason_text, reason_run_styles, reason_base_x, reason_style) =
            build_styles(reason_paragraph);

        let ctx = VariableWidthContext::new(6.66, 11.2);
        let mut collateral_matched = false;
        let mut reason_matched = false;
        for right in 300..=900 {
            let collateral_lines = wrap_text_into_lines(
                &ctx,
                &collateral_text,
                &collateral_run_styles,
                collateral_base_x,
                &collateral_style,
                0,
                right as f64,
            )
            .expect("wrap collateral");
            let collateral_infos = collect_line_infos(&collateral_lines);
            if collateral_infos
                .iter()
                .any(|line| line.text.ends_with("氏名及び住所又は居所)"))
                && !collateral_infos
                    .iter()
                    .any(|line| line.text.ends_with("居所)そ"))
            {
                collateral_matched = true;
            }

            let reason_lines = wrap_text_into_lines(
                &ctx,
                &reason_text,
                &reason_run_styles,
                reason_base_x,
                &reason_style,
                0,
                right as f64,
            )
            .expect("wrap reason");
            let reason_infos = collect_line_infos(&reason_lines);
            if reason_infos
                .iter()
                .any(|line| line.text.ends_with("ことができない事情の"))
                && reason_infos
                    .get(1)
                    .is_some_and(|line| line.text.starts_with("詳細"))
            {
                reason_matched = true;
            }

            if collateral_matched && reason_matched {
                break;
            }
        }

        assert!(
            collateral_matched,
            "failed to reproduce collateral break ending with '居所)'"
        );
        assert!(
            reason_matched,
            "failed to reproduce reason break ending with '事情の'"
        );
    }

    #[test]
    fn yokohama_page3_only_ordinance_line_wraps_before_madeni() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("yokohama_page3_only.docx")
                .expect("load fixture");
        let paragraph = find_paragraph_by_prefix(
            &document,
            "4　法第15条の2第2項及び第3項に規定する条例で定める書類は、第2項第2号から第4号",
        )
        .expect("find ordinance paragraph");
        let mut matched = false;
        let mut samples = Vec::new();
        for ascii_step in 50..=90 {
            let ascii_width = ascii_step as f64 / 10.0;
            for step in 90..=260 {
                let cjk_width = step as f64 / 10.0;
                let ctx = FontAwareVariableWidthContext::new(ascii_width, cjk_width);
                let lines = wrap_fixture_paragraph_lines(&document, &settings, paragraph, &ctx)
                    .expect("wrap ordinance line");
                if samples.len() < 8 && step % 10 == 0 && ascii_step % 10 == 0 {
                    samples.push(format!(
                        "ascii={ascii_width:.2},cjk={cjk_width:.2}: {}",
                        lines.join(" | ")
                    ));
                }
                if lines.windows(2).any(|pair| {
                    pair[0].ends_with("第2項第2号から第4号")
                        && pair[1].starts_with("までに掲げる書類")
                }) {
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
        }
        assert!(
            matched,
            "failed to wrap ordinance line at 第4号/までに boundary; samples={}",
            samples.join(" || ")
        );
    }

    /// The ordinance paragraph is set in ＭＳ 明朝, whose advances follow from
    /// the characters alone (jlsi/jade#469): whatever widths the host's
    /// stand-in face reports, the lines do not move. (This simplified wrap
    /// has no auto-space, so it does not reach Word's `第4号 | までに` break;
    /// the production path is checked against the Word corpus elsewhere.)
    #[test]
    fn yokohama_page6_ordinance_line_does_not_depend_on_host_widths() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("yokohama_page6.docx")
                .expect("load fixture");
        let paragraph = find_paragraph_by_prefix(
            &document,
            "4　法第15条の2第2項及び第3項に規定する条例で定める書類は、",
        )
        .expect("find ordinance paragraph");
        let wrap = |ascii_width: f64, cjk_width: f64| {
            let ctx = FontAwareVariableWidthContext::new(ascii_width, cjk_width);
            wrap_fixture_paragraph_lines(&document, &settings, paragraph, &ctx)
                .expect("wrap ordinance line")
        };
        let reference = wrap(6.0, 12.0);
        assert!(reference.len() > 1, "{reference:?}");
        for (ascii_width, cjk_width) in [(5.0, 9.0), (9.0, 26.0), (7.3, 15.6), (8.25, 11.0)] {
            assert_eq!(
                wrap(ascii_width, cjk_width),
                reference,
                "ascii={ascii_width}, cjk={cjk_width}"
            );
        }
    }

    #[test]
    fn wrap_is_run_boundary_invariant_for_numeric_east_asian_spacing() {
        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let text = "第2項第2号から第4号までに掲げる書類";
        let merged_runs = vec![make_run(text)];
        let split_runs = vec![
            RunStyle {
                start: 0,
                end: "第".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第".len(),
                end: "第2".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第2".len(),
                end: "第2項第".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第2項第".len(),
                end: "第2項第2".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第2項第2".len(),
                end: "第2項第2号から第".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第2項第2号から第".len(),
                end: "第2項第2号から第4".len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: "第2項第2号から第4".len(),
                end: text.len(),
                properties: None,
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
        ];
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };

        for column_right in (120..=220).map(|value| value as f64) {
            let merged = wrap_text_into_lines(
                &ctx,
                text,
                &merged_runs,
                0.0,
                &paragraph_style,
                0,
                column_right,
            )
            .expect("wrap merged");
            let split = wrap_text_into_lines(
                &ctx,
                text,
                &split_runs,
                0.0,
                &paragraph_style,
                0,
                column_right,
            )
            .expect("wrap split");
            assert_eq!(
                collect_line_infos(&merged)
                    .into_iter()
                    .map(|line| line.text)
                    .collect::<Vec<_>>(),
                collect_line_infos(&split)
                    .into_iter()
                    .map(|line| line.text)
                    .collect::<Vec<_>>(),
                "run-boundary-sensitive wrap at column_right={column_right}"
            );
        }
    }

    #[test]
    fn walk_text_advance_steps_is_segment_boundary_invariant() {
        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let style = resolve_run_style_for_wrap(None, None);
        let mut merged_width = 0.0;
        let mut merged_cache = FxHashMap::default();
        let merged_state = walk_text_advance_steps(
            &ctx,
            "第2項",
            &style,
            Some(&paragraph_style),
            &TextAdvanceState::default(),
            None,
            &mut merged_cache,
            |step| {
                merged_width += step.advance.total_advance;
                Ok(())
            },
        )
        .expect("merged steps");

        let mut split_width = 0.0;
        let mut split_cache = FxHashMap::default();
        let state_after_first = walk_text_advance_steps(
            &ctx,
            "第2",
            &style,
            Some(&paragraph_style),
            &TextAdvanceState::default(),
            Some("項"),
            &mut split_cache,
            |step| {
                split_width += step.advance.total_advance;
                Ok(())
            },
        )
        .expect("first segment");
        let split_state = walk_text_advance_steps(
            &ctx,
            "項",
            &style,
            Some(&paragraph_style),
            &state_after_first,
            None,
            &mut split_cache,
            |step| {
                split_width += step.advance.total_advance;
                Ok(())
            },
        )
        .expect("second segment");

        assert!((merged_width - split_width).abs() < 1e-9);
        assert_eq!(merged_state, split_state);
    }

    #[test]
    fn collect_text_advance_steps_matches_walked_steps() {
        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let style = resolve_run_style_for_wrap(None, None);
        let mut cache = FxHashMap::default();
        let mut walked = Vec::new();
        let walked_state = walk_text_advance_steps(
            &ctx,
            "第2項",
            &style,
            Some(&paragraph_style),
            &TextAdvanceState::default(),
            None,
            &mut cache,
            |step| {
                walked.push(OwnedTextAdvanceStep {
                    rel_offset: step.rel_offset,
                    grapheme: Rc::from(step.grapheme),
                    script: step.script,
                    font_descriptor: Rc::from(step.font_descriptor),
                    font_descriptor_hash: step.font_descriptor_hash,
                    measured_glyph_width: step.measured_glyph_width,
                    advance: step.advance,
                });
                Ok(())
            },
        )
        .expect("walked steps");

        let mut collect_cache = FxHashMap::default();
        let (collected, collected_state) = collect_text_advance_steps(
            &ctx,
            "第2項",
            &style,
            Some(&paragraph_style),
            &TextAdvanceState::default(),
            None,
            &mut collect_cache,
        )
        .expect("collected steps");

        assert_eq!(walked, collected);
        assert_eq!(walked_state, collected_state);
    }

    #[test]
    #[ignore]
    fn benchmark_text_advance_steps_hot_path() {
        const ITERS: usize = 2_000;
        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            ..ParagraphStyle::default()
        };
        let style = resolve_run_style_for_wrap(None, None);
        let text = "第2項第2号から第4号までに掲げる書類ABC123第2項第2号から第4号までに掲げる書類";

        let legacy_walk_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let mut total = 0.0;
            let state = legacy_walk_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    total += step.advance.total_advance;
                    Ok(())
                },
            )
            .expect("legacy walk benchmark");
            black_box((total, state, cache));
        }
        let legacy_walk_elapsed = legacy_walk_start.elapsed();

        let walk_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let mut total = 0.0;
            let state = walk_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    total += step.advance.total_advance;
                    Ok(())
                },
            )
            .expect("walk benchmark");
            black_box((total, state, cache));
        }
        let walk_elapsed = walk_start.elapsed();

        let legacy_collect_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let result = legacy_collect_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
            )
            .expect("legacy collect benchmark");
            black_box((result, cache));
        }
        let legacy_collect_elapsed = legacy_collect_start.elapsed();

        let visit_owned_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let mut total = 0.0;
            let state = visit_owned_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    total += step.advance.total_advance;
                    black_box(step);
                    Ok(())
                },
            )
            .expect("visit owned benchmark");
            black_box((total, state, cache));
        }
        let visit_owned_elapsed = visit_owned_start.elapsed();

        let visit_borrowed_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let latin_ascii_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_latin_ascii);
            let latin_hansi_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_latin_hansi);
            let east_asia_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_east_asia);
            let complex_descriptor: Rc<str> = Rc::clone(&style.font_descriptor_complex);
            let mut total = 0.0;
            let state = walk_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
                |step| {
                    let font_descriptor =
                        if step.font_descriptor == &*style.font_descriptor_latin_ascii {
                            latin_ascii_descriptor.clone()
                        } else if step.font_descriptor == &*style.font_descriptor_latin_hansi {
                            latin_hansi_descriptor.clone()
                        } else if step.font_descriptor == &*style.font_descriptor_east_asia {
                            east_asia_descriptor.clone()
                        } else if step.font_descriptor == &*style.font_descriptor_complex {
                            complex_descriptor.clone()
                        } else {
                            Rc::from(step.font_descriptor)
                        };
                    let grapheme: Rc<str> = Rc::from(step.grapheme);
                    total += step.advance.total_advance;
                    black_box((font_descriptor, grapheme, step.rel_offset));
                    Ok(())
                },
            )
            .expect("visit borrowed benchmark");
            black_box((total, state, cache));
        }
        let visit_borrowed_elapsed = visit_borrowed_start.elapsed();

        let collect_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let result = collect_text_advance_steps(
                &ctx,
                text,
                &style,
                Some(&paragraph_style),
                &TextAdvanceState::default(),
                None,
                &mut cache,
            )
            .expect("collect benchmark");
            black_box((result, cache));
        }
        let collect_elapsed = collect_start.elapsed();

        println!(
            "text advance benchmark: legacy_walk={}us walk={}us legacy_collect={}us visit_owned={}us visit_borrowed={}us collect={}us iterations={} graphemes={}",
            legacy_walk_elapsed.as_micros(),
            walk_elapsed.as_micros(),
            legacy_collect_elapsed.as_micros(),
            visit_owned_elapsed.as_micros(),
            visit_borrowed_elapsed.as_micros(),
            collect_elapsed.as_micros(),
            ITERS,
            text.graphemes(true).count(),
        );
    }

    #[test]
    #[ignore]
    fn benchmark_snap_to_chars_cache_hot_path() {
        struct SnapContext;

        impl LineWrapContext for SnapContext {
            type Error = ();

            fn resolve_run_style(
                &self,
                _properties: Option<&RunProperties>,
            ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
                Ok(Rc::new(ResolvedRunStyle::new(
                    "latin".to_string(),
                    "latin".to_string(),
                    "east".to_string(),
                    "complex".to_string(),
                    0.0,
                    1.25,
                    false,
                    None,
                )))
            }

            fn measure_text(
                &self,
                _text: &str,
                _font_descriptor: &str,
            ) -> Result<f64, Self::Error> {
                Ok(10.0)
            }
        }

        const ITERS: usize = 200_000;
        let ctx = SnapContext;
        let paragraph_style = ParagraphStyle {
            doc_grid_snap_to_chars: true,
            doc_grid_char_space_px: Some(1.5),
            ..ParagraphStyle::default()
        };
        let style = ctx.resolve_run_style(None).expect("style");

        let legacy_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let width = legacy_resolve_snap_to_chars_cell_width(
                &ctx,
                &paragraph_style,
                &style.font_descriptor_east_asia,
                style.text_scale_x,
                &mut cache,
            )
            .expect("legacy snap");
            for _ in 0..8 {
                black_box(
                    legacy_resolve_snap_to_chars_cell_width(
                        &ctx,
                        &paragraph_style,
                        &style.font_descriptor_east_asia,
                        style.text_scale_x,
                        &mut cache,
                    )
                    .expect("legacy cached"),
                );
            }
            black_box((width, cache));
        }
        let legacy_elapsed = legacy_start.elapsed();

        let current_start = Instant::now();
        for _ in 0..ITERS {
            let mut cache = FxHashMap::default();
            let width = resolve_snap_to_chars_cell_width(
                &ctx,
                &paragraph_style,
                &style.font_descriptor_east_asia,
                style.font_descriptor_east_asia_hash,
                style.text_scale_x,
                &mut cache,
            )
            .expect("current snap");
            for _ in 0..8 {
                black_box(
                    resolve_snap_to_chars_cell_width(
                        &ctx,
                        &paragraph_style,
                        &style.font_descriptor_east_asia,
                        style.font_descriptor_east_asia_hash,
                        style.text_scale_x,
                        &mut cache,
                    )
                    .expect("current cached"),
                );
            }
            black_box((width, cache));
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "snap-to-chars benchmark: legacy={}us current={}us iterations={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
        );
    }

    #[test]
    fn trailing_overhang_can_trigger_earlier_wrap() {
        struct OverhangContext;

        impl LineWrapContext for OverhangContext {
            type Error = ();

            fn resolve_run_style(
                &self,
                _properties: Option<&RunProperties>,
            ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
                Ok(Rc::new(ResolvedRunStyle::new(
                    "latin".to_string(),
                    "latin".to_string(),
                    "east".to_string(),
                    "complex".to_string(),
                    0.0,
                    1.0,
                    false,
                    None,
                )))
            }

            fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
                Ok(match text {
                    "AB" => 20.0,
                    _ => 10.0,
                })
            }

            fn measure_trailing_overhang(
                &self,
                text: &str,
                _font_descriptor: &str,
            ) -> Result<f64, Self::Error> {
                Ok(if text == "B" { 2.0 } else { 0.0 })
            }
        }

        let ctx = OverhangContext;
        let lines = wrap_text_into_lines(
            &ctx,
            "AB",
            &[make_run("AB")],
            0.0,
            &ParagraphStyle::default(),
            0,
            20.0,
        )
        .expect("wrap");

        let infos = collect_line_infos(&lines);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].text, "A");
        assert_eq!(infos[1].text, "B");
    }

    #[test]
    fn yokohama_page3_only_wraps_match_word_boundaries() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("yokohama_page3_only.docx")
                .expect("load fixture");
        let paragraph_1 = find_paragraph_by_prefix(
            &document,
            "(1)　法第15条第1項各号のいずれかに該当する事実があること及びその該当する事実",
        )
        .expect("find (1) paragraph")
        .clone();
        let paragraph_5 = find_paragraph_by_prefix(
            &document,
            "(5)　分割納付又は分割納入の方法により納付し、又は納入するかどうか",
        )
        .expect("find (5) paragraph")
        .clone();
        let paragraph_4_ordinance = find_paragraph_by_prefix(
            &document,
            "4　法第15条の2第2項及び第3項に規定する条例で定める書類は、第2項第2号から第4号",
        )
        .expect("find ordinance paragraph")
        .clone();
        let paragraph_6 = find_paragraph_by_prefix(
            &document,
            "(6)　当該徴収の猶予を受けようとする金額が1,000,000円を超え、かつ、その期間が3月",
        )
        .expect("find (6) paragraph")
        .clone();

        let mut matched_1_head = false;
        let mut matched_1_detail = false;
        let mut matched_4_ordinance = false;
        let mut matched_5 = false;
        let mut matched_6 = false;
        let mut matched_6_residence = false;
        let mut matched_6_reason = false;
        let mut samples = Vec::new();
        for ascii_step in 50..=90 {
            let ascii_width = ascii_step as f64 / 10.0;
            for step in 90..=260 {
                let cjk_width = step as f64 / 10.0;
                let ctx = FontAwareVariableWidthContext::new(ascii_width, cjk_width);

                let lines_1 =
                    wrap_fixture_paragraph_lines(&document, &settings, &paragraph_1, &ctx)
                        .expect("wrap (1) paragraph");
                let lines_5 =
                    wrap_fixture_paragraph_lines(&document, &settings, &paragraph_5, &ctx)
                        .expect("wrap (5) paragraph");
                let lines_4_ordinance = wrap_fixture_paragraph_lines(
                    &document,
                    &settings,
                    &paragraph_4_ordinance,
                    &ctx,
                )
                .expect("wrap ordinance paragraph");
                let lines_6 =
                    wrap_fixture_paragraph_lines(&document, &settings, &paragraph_6, &ctx)
                        .expect("wrap (6) paragraph");

                if samples.len() < 8 && step % 10 == 0 && ascii_step % 10 == 0 {
                    samples.push(format!(
                        "w12(ascii={ascii_width:.2},cjk={cjk_width:.2}) :: (1)[{}|{}] (4)[{}|{}] (5)[{}|{}] (6)[{}|{}]",
                        lines_1.first().cloned().unwrap_or_default(),
                        lines_1.get(1).cloned().unwrap_or_default(),
                        lines_4_ordinance.first().cloned().unwrap_or_default(),
                        lines_4_ordinance.get(1).cloned().unwrap_or_default(),
                        lines_5.first().cloned().unwrap_or_default(),
                        lines_5.get(1).cloned().unwrap_or_default(),
                        lines_6.first().cloned().unwrap_or_default(),
                        lines_6.get(1).cloned().unwrap_or_default(),
                    ));
                }

                if !matched_1_head
                    && lines_1.windows(2).any(|pair| {
                        pair[0].ends_with("その該当する事実に")
                            && pair[1].starts_with("基づき当該徴収の猶予に係る徴収金")
                    })
                {
                    matched_1_head = true;
                }
                if !matched_1_detail
                    && lines_1.windows(2).any(|pair| {
                        (pair[0].ends_with("又は納入することができない事情")
                            && pair[1].starts_with("の詳細"))
                            || (pair[0].ends_with("又は納入することができない事情の")
                                && pair[1].starts_with("詳細"))
                    })
                {
                    matched_1_detail = true;
                }
                if !matched_4_ordinance
                    && lines_4_ordinance.windows(2).any(|pair| {
                        pair[0].ends_with("第2項第2号から第4号")
                            && pair[1].starts_with("までに掲げる書類")
                    })
                {
                    matched_4_ordinance = true;
                }
                if !matched_5
                    && lines_5.windows(2).any(|pair| {
                        pair[0].ends_with("(分割納付又は分")
                            && pair[1].starts_with("割納入の方法により納付し、又は納入する場合")
                    })
                {
                    matched_5 = true;
                }
                if !matched_6
                    && lines_6.windows(2).any(|pair| {
                        pair[0].ends_with("かつ、その期間が3月")
                            && pair[1].starts_with("を超える場合には、提供しようとする法第16条")
                    })
                {
                    matched_6 = true;
                }
                if !matched_6_residence
                    && lines_6.windows(2).any(|pair| {
                        pair[0].ends_with("保証人の氏名及び住所又は居所)")
                            && pair[1].starts_with("その他担保に関し参考となるべき事項")
                    })
                {
                    matched_6_residence = true;
                }
                if !matched_6_reason
                    && lines_6.windows(2).any(|pair| {
                        pair[0].ends_with("担保を提供することができない特別の事情がある")
                            && pair[1].starts_with("ときは、その事情")
                    })
                {
                    matched_6_reason = true;
                }

                if matched_1_head
                    && matched_1_detail
                    && matched_4_ordinance
                    && matched_5
                    && matched_6
                    && matched_6_residence
                    && matched_6_reason
                {
                    break;
                }
            }
            if matched_1_head
                && matched_1_detail
                && matched_4_ordinance
                && matched_5
                && matched_6
                && matched_6_residence
                && matched_6_reason
            {
                break;
            }
        }

        assert!(
            matched_1_head
                && matched_1_detail
                && matched_4_ordinance
                && matched_5
                && matched_6
                && matched_6_residence
                && matched_6_reason,
            "failed to match Word wrap boundaries for yokohama_page3_only ((1-head)/(1-detail)/(4-ordinance)/(5)/(6)/(6-residence)/(6-reason)); matched_1_head={matched_1_head} matched_1_detail={matched_1_detail} matched_4_ordinance={matched_4_ordinance} matched_5={matched_5} matched_6={matched_6} matched_6_residence={matched_6_residence} matched_6_reason={matched_6_reason}; samples={}",
            samples.join(" || ")
        );
    }

    #[test]
    fn yokohama_page6_page3_reason_line_keeps_no_before_detail() {
        let mut matched = false;
        for step in 90..=260 {
            let cjk_width = step as f64 / 10.0;
            let page_three_lines =
                page_three_lines_for_fixture(6.66, cjk_width).expect("layout fixture");
            for pair in page_three_lines.windows(2) {
                if pair[0].starts_with(
                    "(1)　当該徴収の猶予に係る徴収金を一時に納付し、又は納入することができない事情",
                ) && pair[0].ends_with("ことができない事情の")
                    && pair[1].starts_with("詳細")
                {
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
        }
        assert!(matched, "failed to keep '事情の' + '詳細' boundary");
    }

    fn with_numbering_marker_prefix(paragraph: &Paragraph, marker: &str) -> Paragraph {
        let mut prefixed = paragraph.clone();
        if let Some(base_run) = prefixed.runs.first().cloned() {
            let mut marker_run = base_run;
            marker_run.text = marker.to_string();
            marker_run.image = None;
            marker_run.text_box = None;
            marker_run.math = None;
            marker_run.field = None;
            marker_run.hyperlink = None;
            marker_run.comment_reference_ids.clear();
            marker_run.footnote_reference_ids.clear();
            marker_run.endnote_reference_ids.clear();
            marker_run.has_footnote_ref = false;
            marker_run.has_endnote_ref = false;
            prefixed.runs.insert(0, marker_run);
        }
        prefixed
    }

    #[test]
    fn contract_first_paragraph_wraps_like_word_at_reference_widths() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let first = find_paragraph_by_prefix(
            &document,
            "秘密保持契約書は営業上の情報、本契約の存在および内容そのasdasd他一切の情報を言う。",
        )
        .expect("find first paragraph");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, first, &ctx)
            .expect("wrap first paragraph");
        let line_two = lines
            .get(1)
            .map(String::as_str)
            .expect("first paragraph second line");
        assert!(
            line_two.ends_with("あｓだｓだｓ") || line_two.ends_with("あｓだｓだｓだ"),
            "unexpected first paragraph second line: {line_two}"
        );
        let line_three = lines
            .get(2)
            .map(String::as_str)
            .expect("first paragraph third line");
        assert!(
            line_three.starts_with("だｓasdas") || line_three.starts_with("ｓasdas"),
            "unexpected first paragraph third line: {line_three}"
        );
    }

    #[test]
    fn trust_article_14_2_item_1_keeps_non_east_asian_run_punctuation_on_next_line() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("trust-beneficiary-sale-keep-next.docx")
                .expect("load fixture");
        let article_thirteen = find_paragraph_by_prefix(
            &document,
            "1．\t売主は、信託不動産及び受益権の種類、品質若しくは数量",
        )
        .expect("find article 13 item 1");
        assert_eq!(
            article_thirteen
                .runs
                .last()
                .and_then(|run| run.properties.as_ref())
                .and_then(|props| props.fonts.as_ref())
                .and_then(|fonts| fonts.hint.clone()),
            Some(FontHint::EastAsia),
            "article 13's terminal run explicitly selects the East-Asian font slot"
        );
        let paragraph = find_paragraph_by_prefix(&document, "1.\t売主は、信託建物について20")
            .expect("find article 14-2 item 1");
        let terminal_run = paragraph.runs.last().expect("terminal run");
        assert_eq!(
            terminal_run
                .properties
                .as_ref()
                .and_then(|props| props.fonts.as_ref())
                .and_then(|fonts| fonts.hint.clone()),
            None,
            "the terminal OOXML run intentionally omits w:rFonts/@w:hint"
        );
        let ctx = TestContext::new(points_to_px(10.5));
        let lines = wrap_fixture_paragraph_lines(&document, &settings, paragraph, &ctx)
            .expect("wrap article 14-2 item 1");
        assert_eq!(lines.len(), 10, "Word lays this paragraph out in ten lines");
        assert!(
            lines
                .get(1)
                .is_some_and(|line| line.starts_with("(a)本請負契約附属")),
            "the non-hanging ASCII ')' must move the complete '(a)' token: {lines:?}"
        );
    }

    #[test]
    fn trust_article_14_2_paragraph_terminal_period_does_not_hang() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("trust-beneficiary-sale-keep-next.docx")
                .expect("load fixture");
        let paragraph = find_paragraph_by_prefix(&document, "1.\t売主は、信託建物について20")
            .expect("find article 14-2 item 1");
        let terminal_index = paragraph.runs.len() - 1;
        let previous = &paragraph.runs[terminal_index - 1];
        let terminal = &paragraph.runs[terminal_index];
        let previous_tail = "又は拒絶してはならない。）";
        assert!(previous.text.ends_with(previous_tail));
        let text = format!("{previous_tail}{}", terminal.text);
        let split = previous_tail.len();
        let run_styles = [
            RunStyle {
                start: 0,
                end: split,
                properties: previous.properties.clone().map(Rc::new),
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
            RunStyle {
                start: split,
                end: text.len(),
                properties: terminal.properties.clone().map(Rc::new),
                revision: None,
                hyperlink: None,
                inline_object: None,
            },
        ];
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules::from_settings(
                &settings.to_line_breaking_settings(),
            ),
            ..ParagraphStyle::default()
        };
        let ctx = TestContext::new(points_to_px(10.5));
        let lines = wrap_text_into_lines(
            &ctx,
            &text,
            &run_styles,
            0.0,
            &paragraph_style,
            0,
            points_to_px(420.0),
        )
        .expect("wrap terminal runs");
        let lines = collect_line_infos(&lines)
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>();

        assert_eq!(
            lines,
            [
                "又は拒絶してはならない。）した場合は、信託建物の本竣工期限を変更できるものとす"
                    .to_string(),
                "る。".to_string(),
            ]
        );
    }

    #[test]
    fn original_trust_notice_word_2013_left_lines_keep_punctuation_inside() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let (document, settings) =
            load_document_with_settings_and_style_defaults("trust-beneficiary-sale-keep-next.docx")
                .unwrap();
        let paragraph = document
            .body
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .nth(362)
            .unwrap();
        let mut text = String::new();
        let mut runs = Vec::new();
        for run in &paragraph.runs {
            let start = text.len();
            text.push_str(&run.text);
            if start < text.len() {
                runs.push(RunStyle {
                    start,
                    end: text.len(),
                    properties: Some(Rc::new(RunProperties {
                        language: Some(Rc::from("en-US")),
                        east_asia_language: Some(Rc::from("ja-JP")),
                        ..run.properties.clone().unwrap_or_default()
                    })),
                    revision: None,
                    hyperlink: None,
                    inline_object: None,
                });
            }
        }
        let mut style = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules::from_settings(
                &settings.to_line_breaking_settings(),
            ),
            tab_stops: resolve_tab_stops(&paragraph.properties.as_ref().unwrap().tabs),
            ..Default::default()
        };
        let mut x = points_to_px(68.25);
        apply_paragraph_indent_with_char_unit(
            &mut x,
            &mut style,
            paragraph
                .properties
                .as_ref()
                .unwrap()
                .indent
                .as_ref()
                .unwrap(),
            false,
            10.5,
        );
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let lines = wrap_text_into_lines(
            &FontAwareVariableWidthContext::new(8.0, 16.0),
            &text,
            &runs,
            x,
            &style,
            0,
            points_to_px(524.4),
        )
        .unwrap();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let actual = lines
            .iter()
            .map(|line| {
                line.segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, [
            "1．本契約に別段の定めのある場合を除き、本契約に基づく通知その他の連絡は、全て書面に",
            "よらなければならず、別紙6に記載する住所、ファクシミリ番号（ただし、本契約締結",
            "後、当事者がその住所又はファクシミリ番号を変更し、これを本条に規定する方法により",
            "他の当事者に通知した場合、かかる変更後の住所又はファクシミリ番号とする。）又は別",
            "途指定する電子メールアドレス宛に、郵便、ファクシミリ又は電子メールにてこれを行",
            "う。",
        ]);
    }

    #[test]
    fn justified_paragraph_terminal_period_hangs_under_word_2013_layout() {
        let style = |alignment: &str, compatibility_mode: u32| ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                compatibility_mode: Some(compatibility_mode),
                ..EastAsianLineBreakingRules::default()
            },
            alignment: alignment.to_string(),
            ..ParagraphStyle::default()
        };
        assert!(!style("left", 15).allows_hanging_punctuation_at(LineBreakKind::ParagraphEnd));
        assert!(style("both", 15).allows_hanging_punctuation_at(LineBreakKind::ParagraphEnd));
        assert!(style("left", 14).allows_hanging_punctuation_at(LineBreakKind::ParagraphEnd));
        assert!(style("left", 15).allows_hanging_punctuation_at(LineBreakKind::Soft));
        let mut mac = style("left", 15);
        mac.word_platform = crate::line_metrics::WordLayoutPlatform::Mac;
        assert!(!mac.allows_hanging_punctuation_at(LineBreakKind::Soft));
        mac.alignment = "both".into();
        assert!(mac.allows_hanging_punctuation_at(LineBreakKind::Soft));
        mac.alignment = "left".into();
        mac.east_asian_rules.compatibility_mode = Some(12);
        assert!(mac.allows_hanging_punctuation_at(LineBreakKind::Soft));
    }

    #[test]
    fn contract_item2_marker_line_keeps_joho_together_at_reference_widths() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let item2 = find_paragraph_by_prefix(
            &document,
            "開示を受けた後、秘密保持義務を負うことなく第三者から正当に入手した情報",
        )
        .expect("find item2 paragraph");
        let paragraph = with_numbering_marker_prefix(item2, "(2)　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap item2 paragraph");
        assert_eq!(lines.len(), 1, "item2 should stay on one line");
        assert!(
            lines[0].ends_with("正当に入手した情報"),
            "unexpected item2 line: {}",
            lines[0]
        );
    }

    #[test]
    fn contract_item3_marker_line_keeps_shutoku_together_at_reference_widths() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let item3 = find_paragraph_by_prefix(
            &document,
            "開示を受けた後、相手方から開asdasdsa示を受けた情報に関係なく独自に取得",
        )
        .expect("find item3 paragraph");
        let paragraph = with_numbering_marker_prefix(item3, "(3)　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap item3 paragraph");
        let first_line = lines.first().map(String::as_str).unwrap_or_default();
        assert!(
            first_line.contains("独自に取得"),
            "unexpected item3 first line: {first_line}"
        );
        assert!(
            !first_line.ends_with("独自に取"),
            "item3 should not split between '取' and '得': {first_line}"
        );
    }

    #[test]
    fn contract_item5_marker_line_keeps_shimei_together_at_reference_widths() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let item5 = find_paragraph_by_prefix(
            &document,
            "秘密情報の管理について、取扱責任者を定め、書面をもって取扱責任者の氏名および連絡先",
        )
        .expect("find item5 paragraph");
        let paragraph = with_numbering_marker_prefix(item5, "(5)　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap item5 paragraph");
        let first_line = lines.first().map(String::as_str).unwrap_or_default();
        let second_line = lines.get(1).map(String::as_str).unwrap_or_default();
        assert!(
            first_line.ends_with("取扱責任者の氏名"),
            "item5 first line should end with '取扱責任者の氏名': {first_line}"
        );
        assert!(
            second_line.starts_with("および連絡先"),
            "item5 second line should start with 'および連絡先': {second_line}"
        );
    }

    #[test]
    fn contract_article1_item1_does_not_split_between_kaku_and_gou() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let article1_item1 = find_paragraph_by_prefix(
            &document,
            "甲および乙は、暴力団等反社会的勢力との関係ないし関与の事実のないことを表明し、保証する。",
        )
        .expect("find article1 item1 paragraph");
        let paragraph = with_numbering_marker_prefix(article1_item1, "1.　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap article1 item1 paragraph");
        let split_between_kaku_and_gou = lines.windows(2).any(|pair| {
            let prev = pair[0].as_str();
            let next = pair[1].as_str();
            prev.ends_with('各') && next.starts_with('号')
        });
        assert!(
            !split_between_kaku_and_gou,
            "article1 item1 split between '各' and '号': {lines:?}"
        );
    }

    #[test]
    fn contract_article1_item2_does_not_split_between_kai_and_jo() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let article1_item2 = find_paragraph_by_prefix(
            &document,
            "契約締結後に、甲又は乙が前項に違反してasdasdasdいることが判明した場合又は違反することになった場合には、相手方は本契約を何らの通知、催告なしに直ちに解除",
        )
        .expect("find article1 item2 paragraph");
        let paragraph = with_numbering_marker_prefix(article1_item2, "2.　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap article1 item2 paragraph");
        let split_between_kai_and_jo = lines.windows(2).any(|pair| {
            let prev = pair[0].as_str();
            let next = pair[1].as_str();
            prev.ends_with('解') && next.starts_with('除')
        });
        assert!(
            !split_between_kai_and_jo,
            "article1 item2 split between '解' and '除': {lines:?}"
        );
    }

    #[test]
    fn contract_copy_clause_without_run_language_does_not_guess_hanging_set() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let copy_clause = find_paragraph_by_prefix(
            &document,
            "秘密情報等を複製する場合には、本取引の目的の範囲内に限って行うものとし、その複製物は、",
        )
        .expect("find copy clause paragraph");
        let paragraph = with_numbering_marker_prefix(copy_clause, "(3)　");

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap copy clause paragraph");
        assert!(
            lines
                .first()
                .is_some_and(|line| line.ends_with("行うものと")),
            "language-neutral copy clause must keep punctuation inside the text extent: {lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|line| line.starts_with("し、その複製物は、")),
            "language-neutral copy clause must move the body character with its punctuation: {lines:?}"
        );
    }

    #[test]
    fn contract_copy_clause_explicit_overflow_still_requires_run_language() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let copy_clause = find_paragraph_by_prefix(
            &document,
            "秘密情報等を複製する場合には、本取引の目的の範囲内に限って行うものとし、その複製物は、",
        )
        .expect("find copy clause paragraph");
        let mut paragraph = with_numbering_marker_prefix(copy_clause, "(3)　");
        let paragraph_props = paragraph.properties.get_or_insert_with(Default::default);
        let mut east_asian = paragraph_props
            .east_asian_settings
            .clone()
            .unwrap_or_default();
        east_asian.use_east_asian_layout = true;
        east_asian.overflow_punctuation = Some(true);
        paragraph_props.east_asian_settings = Some(east_asian);

        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines = wrap_fixture_paragraph_lines(&document, &settings, &paragraph, &ctx)
            .expect("wrap copy clause paragraph (overflow on)");
        assert!(
            lines
                .first()
                .is_some_and(|line| line.ends_with("行うものと")),
            "overflowPunct cannot choose a punctuation table without a run language: {lines:?}"
        );
    }

    #[test]
    fn contract_copy_clause_explicit_overflow_punctuation_off_keeps_kuten_next_line() {
        let (document, settings) =
            load_document_with_settings_and_style_defaults("contract.docx").expect("load fixture");
        let copy_clause = find_paragraph_by_prefix(
            &document,
            "秘密情報等を複製する場合には、本取引の目的の範囲内に限って行うものとし、その複製物は、",
        )
        .expect("find copy clause paragraph");
        let mut overflow_off_paragraph = with_numbering_marker_prefix(copy_clause, "(3)　");
        let paragraph_props = overflow_off_paragraph
            .properties
            .get_or_insert_with(Default::default);
        let mut east_asian = paragraph_props
            .east_asian_settings
            .clone()
            .unwrap_or_default();
        east_asian.use_east_asian_layout = true;
        east_asian.overflow_punctuation = Some(false);
        paragraph_props.east_asian_settings = Some(east_asian);
        let ctx = FontAwareVariableWidthContext::new(5.0, 15.6);
        let lines =
            wrap_fixture_paragraph_lines(&document, &settings, &overflow_off_paragraph, &ctx)
                .expect("wrap copy clause paragraph (overflow off)");
        assert!(
            lines
                .first()
                .is_some_and(|line| line.ends_with("行うものと")),
            "copy clause first line should end with '行うものと' when overflowPunct is off: {lines:?}"
        );
    }

    #[test]
    fn observed_mac_hansi_ellipsis_keeps_full_grid_cell_and_font_slot() {
        use crate::line_metrics::WordLayoutPlatform;
        assert_eq!(classify_script("…"), ScriptCategory::Latin);
        let style = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            doc_grid_char_space_px: Some(points_to_px(3.0)),
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let advance = |paragraph: &ParagraphStyle, descriptor, oracle, scale, tracking| {
            compute_grapheme_advance(GraphemeAdvanceContext {
                paragraph_style: Some(paragraph),
                grapheme: "…",
                script: ScriptCategory::Latin,
                measured_glyph_width: points_to_px(8.0),
                measured_width_is_oracle: oracle,
                text_scale_x: scale,
                extra_spacing: points_to_px(tracking),
                kerning_adjustment: 0.0,
                is_line_start: false,
                prev_prev_grapheme: None,
                prev_grapheme: Some("g"),
                next_grapheme: Some("s"),
                prev_glyph_width: None,
                snap_cell_width: None,
                font_descriptor: Some(descriptor),
                prev_auto_space: None,
                run_letter_spacing: 0.0,
            })
        };
        for descriptor in [
            "11pt 'ＭＳ 明朝'",
            "11pt 'MS Mincho'",
            "11pt 'MS-Mincho'",
            "11pt '游ゴシック Light'",
            "11pt 'Yu Gothic Light'",
            "11pt 'YuGothic-Light'",
        ] {
            let result = advance(&style, descriptor, false, 1.0, -1.0);
            assert!(
                (result.glyph_width * 0.75 - 11.0).abs() < 1e-9,
                "{descriptor}"
            );
            assert!(
                (result.total_advance * 0.75 - 12.0).abs() < 1e-9,
                "{descriptor}"
            );
            // Source terminal glyphs already pass zero following tracking.
            assert!(
                (advance(&style, descriptor, false, 1.0, 0.0).total_advance * 0.75 - 14.0).abs()
                    < 1e-9
            );
            assert!(
                (advance(&style, descriptor, false, 0.5, -1.0).total_advance * 0.75 - 6.5).abs()
                    < 1e-9
            );
            let no_grid = ParagraphStyle {
                doc_grid_char_space_px: None,
                ..style.clone()
            };
            assert!(
                (advance(&no_grid, descriptor, false, 1.0, -1.0).total_advance * 0.75 - 10.0).abs()
                    < 1e-9
            );
        }
        for descriptor in [
            "11pt 'Unknown'",
            "11pt 'MS Gothic'",
            "11pt 'Yu Gothic'",
            "bold 11pt 'MS Mincho'",
            "italic 11pt 'Yu Gothic Light'",
        ] {
            assert!(
                (advance(&style, descriptor, false, 1.0, -1.0).total_advance * 0.75 - 8.5).abs()
                    < 1e-9
            );
        }
        let windows = ParagraphStyle {
            word_platform: WordLayoutPlatform::Windows,
            ..style.clone()
        };
        assert!(
            (advance(&windows, "11pt 'MS Mincho'", false, 1.0, -1.0).total_advance * 0.75 - 8.5)
                .abs()
                < 1e-9
        );
        assert!(
            (advance(&style, "11pt 'MS Mincho'", true, 1.0, -1.0).total_advance * 0.75 - 7.0).abs()
                < 1e-9
        );
    }

    #[test]
    fn mac_east_asian_loose_grid_keeps_word_condensed_cell_pitch() {
        use crate::line_metrics::WordLayoutPlatform;
        // numbering-restart's Normal is 11pt, spacing=-20 twips and its
        // linesAndChars grid adds 3pt. Word's Japanese glyph origins are
        // 12pt apart (the ASCII origins are 6pt apart).
        let mut ctx = VariableWidthContext::new(points_to_px(5.5), points_to_px(11.0));
        ctx.style.letter_spacing_px = points_to_px(-1.0);
        let style = ParagraphStyle {
            word_platform: WordLayoutPlatform::Mac,
            doc_grid_char_space_px: Some(points_to_px(3.0)),
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let text = "漢漢";
        let lines = wrap_text_into_lines(
            &ctx,
            text,
            &[make_run(text)],
            0.0,
            &style,
            0,
            points_to_px(27.0),
        )
        .unwrap();
        assert_eq!(lines.len(), 1, "two condensed grid cells fit in 27pt");
        assert!((lines[0].clusters[0].width * 0.75 - 12.0).abs() < 0.01);
        // The terminal character has no following tracking boundary.
        assert!((lines[0].clusters[1].width * 0.75 - 14.0).abs() < 0.01);
        let latin =
            wrap_text_into_lines(&ctx, "AB", &[make_run("AB")], 0.0, &style, 0, 400.0).unwrap();
        assert!((latin[0].clusters[0].width * 0.75 - 6.0).abs() < 0.01);

        for control in [
            ParagraphStyle {
                word_platform: WordLayoutPlatform::Windows,
                ..style.clone()
            },
            ParagraphStyle {
                east_asian_rules: EastAsianLineBreakingRules::default(),
                ..style.clone()
            },
            ParagraphStyle {
                doc_grid_char_space_px: None,
                ..style.clone()
            },
        ] {
            let lines =
                wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &control, 0, 400.0)
                    .unwrap();
            let expected = 11.0 * EAST_ASIA_ADVANCE_SCALE - 1.0
                + control.doc_grid_char_space_px.unwrap_or(0.0) * 0.75;
            assert!((lines[0].clusters[0].width * 0.75 - expected).abs() < 0.01);
        }
    }

    #[test]
    fn lines_and_chars_keeps_proportional_latin_advances_with_half_grid_spacing() {
        use crate::line_metrics::WordLayoutPlatform;
        for platform in [WordLayoutPlatform::Mac, WordLayoutPlatform::Windows] {
            let mut ctx = VariableWidthContext::new(6.0, 10.0);
            ctx.style.letter_spacing_px = -0.5;
            let text = "Aaああ";
            let style = ParagraphStyle {
                word_platform: platform,
                doc_grid_char_space_px: Some(2.0),
                ..ParagraphStyle::default()
            };
            let lines = wrap_text_into_lines(&ctx, text, &[make_run(text)], 0.0, &style, 0, 400.0)
                .expect("wrap grid text");
            let clusters = &lines[0].clusters;
            assert_eq!(clusters.len(), 4);
            assert!((clusters[0].width - 6.5).abs() < 0.01);
            assert!((clusters[1].width - 6.5).abs() < 0.01);
            assert!(
                (clusters[2].width - (10.0 * EAST_ASIA_ADVANCE_SCALE + 1.5)).abs() < 0.01,
                "{platform:?}: clusters={clusters:?}"
            );
            // Explicit tracking applies between glyphs; the terminal grid
            // character still receives the section's full character pitch.
            assert!((clusters[3].width - (10.0 * EAST_ASIA_ADVANCE_SCALE + 2.0)).abs() < 0.01);
        }
    }

    #[test]
    fn snap_to_chars_spans_latin_and_snaps_east_asian_clusters() {
        let ctx = VariableWidthContext::new(6.0, 10.0);
        let text = "Aあ";
        let runs = vec![make_run(text)];
        let paragraph_style = ParagraphStyle {
            doc_grid_snap_to_chars: true,
            ..ParagraphStyle::default()
        };

        let wrapped =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph_style, 0, 400.0).expect("wrap");
        let clusters = &wrapped[0].clusters;
        assert_eq!(clusters.len(), 2);

        let expected_cell = 10.0 * EAST_ASIA_ADVANCE_SCALE;
        assert!(
            (clusters[0].width - expected_cell).abs() < 0.01,
            "latin cluster should span one grid cell: got={} expected={}",
            clusters[0].width,
            expected_cell
        );
        assert!(
            (clusters[1].width - expected_cell).abs() < 0.01,
            "east asian cluster should snap to one grid cell: got={} expected={}",
            clusters[1].width,
            expected_cell
        );
    }

    #[test]
    fn snap_to_chars_applies_doc_grid_char_space_to_cell_width() {
        let ctx = VariableWidthContext::new(6.0, 10.0);
        let text = "Aa";
        let runs = vec![make_run(text)];
        let paragraph_style = ParagraphStyle {
            doc_grid_snap_to_chars: true,
            doc_grid_char_space_px: Some(2.0),
            ..ParagraphStyle::default()
        };

        let wrapped =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph_style, 0, 400.0).expect("wrap");
        let clusters = &wrapped[0].clusters;
        assert_eq!(clusters.len(), 2);

        let expected_cell = 10.0 * EAST_ASIA_ADVANCE_SCALE + 2.0;
        assert!(
            (clusters[0].width - expected_cell).abs() < 0.01,
            "latin cluster should use charSpace-adjusted cell width: got={} expected={}",
            clusters[0].width,
            expected_cell
        );
    }

    #[test]
    fn lines_mode_keeps_default_cluster_widths_without_snap_to_chars() {
        let ctx = VariableWidthContext::new(6.0, 10.0);
        let text = "Aあ";
        let runs = vec![make_run(text)];
        let paragraph_style = ParagraphStyle::default();

        let wrapped =
            wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph_style, 0, 400.0).expect("wrap");
        let clusters = &wrapped[0].clusters;
        assert_eq!(clusters.len(), 2);
        assert!((clusters[0].width - 6.0).abs() < 0.01);
        assert!((clusters[1].width - (10.0 * EAST_ASIA_ADVANCE_SCALE)).abs() < 0.01);
    }

    #[test]
    fn doc_grid_modes_lines_lines_and_chars_and_snap_to_chars_diverge() {
        let text = "Aあ";
        let runs = vec![make_run(text)];
        let lines_style = ParagraphStyle::default();

        let lines_ctx = VariableWidthContext::new(6.0, 10.0);
        let lines = wrap_text_into_lines(&lines_ctx, text, &runs, 0.0, &lines_style, 0, 400.0)
            .expect("wrap lines");
        let lines_width: f64 = lines[0].clusters.iter().map(|cluster| cluster.width).sum();

        let mut lines_and_chars_ctx = VariableWidthContext::new(6.0, 10.0);
        lines_and_chars_ctx.style.letter_spacing_px = 2.0;
        let lines_and_chars = wrap_text_into_lines(
            &lines_and_chars_ctx,
            text,
            &runs,
            0.0,
            &lines_style,
            0,
            400.0,
        )
        .expect("wrap linesAndChars");
        let lines_and_chars_width: f64 = lines_and_chars[0]
            .clusters
            .iter()
            .map(|cluster| cluster.width)
            .sum();

        let snap_style = ParagraphStyle {
            doc_grid_snap_to_chars: true,
            ..ParagraphStyle::default()
        };
        let snap_ctx = VariableWidthContext::new(6.0, 10.0);
        let snap = wrap_text_into_lines(&snap_ctx, text, &runs, 0.0, &snap_style, 0, 400.0)
            .expect("wrap snapToChars");
        let snap_width: f64 = snap[0].clusters.iter().map(|cluster| cluster.width).sum();

        assert!(
            lines_width < lines_and_chars_width,
            "linesAndChars should widen advance vs lines: lines={} linesAndChars={}",
            lines_width,
            lines_and_chars_width
        );
        assert!(
            lines_and_chars_width < snap_width,
            "snapToChars should occupy wider snapped cells than linesAndChars: linesAndChars={} snap={}",
            lines_and_chars_width,
            snap_width
        );
    }

    #[test]
    fn final_width_normalization_moves_overflowing_cluster_to_next_line() {
        let ctx = TestContext::new(1.0);
        let paragraph_style = ParagraphStyle::default();
        let mut cache = FxHashMap::default();
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "abcX".into(),
                    start: 0,
                    end: 4,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(0, 1, 1.0, "a", false, false, false),
                    InlineCluster::grapheme(1, 2, 1.0, "b", false, false, false),
                    InlineCluster::grapheme(2, 3, 1.0, "c", false, false, false),
                    InlineCluster::grapheme(3, 4, 1.0, "X", false, false, false),
                ],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "def".into(),
                    start: 4,
                    end: 7,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((7, 7)),
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(4, 5, 1.0, "d", false, false, false),
                    InlineCluster::grapheme(5, 6, 1.0, "e", false, false, false),
                    InlineCluster::grapheme(6, 7, 1.0, "f", false, false, false),
                ],
            },
        ];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            3.5,
            &mut cache,
        )
        .expect("normalize widths");

        let infos = collect_line_infos(&lines);
        assert_eq!(infos[0].text, "abc");
        assert_eq!(infos[1].text, "Xdef");
    }

    #[test]
    fn final_width_normalization_reapplies_word_boundaries_to_breakable_continuation() {
        let ctx = TestContext::new(1.0);
        let paragraph_style = ParagraphStyle {
            alignment: "both".to_string(),
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let first = WrappedLine {
            segments: vec![TextSegment {
                text: "x".into(),
                start: 0,
                end: 1,
                properties: None,
                revision: None,
                hyperlink: None,
                leading_width: 0.0,
            }],
            break_kind: LineBreakKind::Soft,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: vec![InlineCluster::grapheme(0, 1, 1.0, "x", false, false, false)],
        };
        let final_text = "ab cd";
        let second = WrappedLine {
            segments: vec![TextSegment {
                text: final_text.into(),
                start: 1,
                end: 1 + final_text.len(),
                properties: None,
                revision: None,
                hyperlink: None,
                leading_width: 0.0,
            }],
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((1 + final_text.len(), 1 + final_text.len())),
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters: final_text
                .char_indices()
                .map(|(offset, character)| {
                    let start = 1 + offset;
                    let end = start + character.len_utf8();
                    InlineCluster::grapheme(
                        start,
                        end,
                        1.0,
                        &character.to_string(),
                        character.is_whitespace(),
                        false,
                        false,
                    )
                })
                .collect(),
        };
        let mut lines = vec![first, second];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            4.5,
            &mut cache,
        )
        .expect("split breakable final continuation");
        let infos = collect_line_infos(&lines);
        assert_eq!(
            infos
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "ab ", "cd"]
        );
        assert!(infos.iter().all(|line| line.width <= 3.75 + 1e-9));
    }

    #[test]
    fn final_width_normalization_reuses_absolute_word_oracle_offsets() {
        // The fallback font reports 10px per glyph, while the Word trace says
        // the same glyphs advance by only 3/4/4px. Initial wrapping and final
        // normalization must consume the same oracle map; otherwise the final
        // line is falsely declared overfull and split a second time.
        let ctx = TestContext::new(10.0);
        let paragraph_style = ParagraphStyle {
            oracle_advance_px_by_text_offset: Some(Rc::new(FxHashMap::from_iter([
                (0, 3.0),
                (1, 4.0),
                (2, 4.0),
            ]))),
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "a".into(),
                    start: 0,
                    end: 1,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![InlineCluster::grapheme(0, 1, 3.0, "a", false, false, false)],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "bc".into(),
                    start: 1,
                    end: 3,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((3, 3)),
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(1, 2, 4.0, "b", false, false, false),
                    InlineCluster::grapheme(2, 3, 4.0, "c", false, false, false),
                ],
            },
        ];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            9.5,
            &mut cache,
        )
        .expect("normalize oracle widths");

        let infos = collect_line_infos(&lines);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].text, "a");
        assert_eq!(infos[1].text, "bc");
    }

    #[test]
    fn final_width_normalization_still_runs_with_overflow_punctuation_for_non_punct() {
        let ctx = TestContext::new(1.0);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "abcX".into(),
                    start: 0,
                    end: 4,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(0, 1, 1.0, "a", false, false, false),
                    InlineCluster::grapheme(1, 2, 1.0, "b", false, false, false),
                    InlineCluster::grapheme(2, 3, 1.0, "c", false, false, false),
                    InlineCluster::grapheme(3, 4, 1.0, "X", false, false, false),
                ],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "def".into(),
                    start: 4,
                    end: 7,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((7, 7)),
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(4, 5, 1.0, "d", false, false, false),
                    InlineCluster::grapheme(5, 6, 1.0, "e", false, false, false),
                    InlineCluster::grapheme(6, 7, 1.0, "f", false, false, false),
                ],
            },
        ];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            3.5,
            &mut cache,
        )
        .expect("normalize widths");

        let infos = collect_line_infos(&lines);
        assert_eq!(infos[0].text, "abc");
        assert_eq!(infos[1].text, "Xdef");
    }

    #[test]
    fn final_width_normalization_keeps_hanging_punctuation_overflow() {
        let ctx = TestContext::new(1.0);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: Some(true),
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "abc。".into(),
                    start: 0,
                    end: 6,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(0, 1, 1.0, "a", false, false, false),
                    InlineCluster::grapheme(1, 2, 1.0, "b", false, false, false),
                    InlineCluster::grapheme(2, 3, 1.0, "c", false, false, false),
                    InlineCluster {
                        can_hang_line_end: true,
                        ..InlineCluster::grapheme(3, 6, 1.0, "。", false, true, false)
                    },
                ],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "def".into(),
                    start: 6,
                    end: 9,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((9, 9)),
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(6, 7, 1.0, "d", false, false, false),
                    InlineCluster::grapheme(7, 8, 1.0, "e", false, false, false),
                    InlineCluster::grapheme(8, 9, 1.0, "f", false, false, false),
                ],
            },
        ];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            3.5,
            &mut cache,
        )
        .expect("normalize widths");

        let infos = collect_line_infos(&lines);
        assert_eq!(infos[0].text, "abc。");
        assert_eq!(infos[1].text, "def");
    }

    #[test]
    fn final_width_normalization_creates_a_continuation_for_left_aligned_text() {
        let ctx = TestContext::new(1.0);
        let paragraph_style = ParagraphStyle {
            enable_line_break_rules: true,
            east_asian_rules: EastAsianLineBreakingRules {
                use_east_asian_layout: true,
                overflow_punctuation: None,
                ..EastAsianLineBreakingRules::default()
            },
            ..ParagraphStyle::default()
        };
        let mut cache = FxHashMap::default();
        let mut lines = vec![
            WrappedLine {
                segments: vec![TextSegment {
                    text: "a".into(),
                    start: 0,
                    end: 1,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::Soft,
                break_range: None,
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![InlineCluster::grapheme(0, 1, 1.0, "a", false, false, false)],
            },
            WrappedLine {
                segments: vec![TextSegment {
                    text: "bc字".into(),
                    start: 1,
                    end: 6,
                    properties: None,
                    revision: None,
                    hyperlink: None,
                    leading_width: 0.0,
                }],
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((6, 6)),
                break_properties: None,
                leaders: Vec::new(),
                tab_stops: Vec::new(),
                clusters: vec![
                    InlineCluster::grapheme(1, 2, 1.0, "b", false, false, false),
                    InlineCluster::grapheme(2, 3, 1.0, "c", false, false, false),
                    InlineCluster::grapheme(3, 6, 1.0, "字", false, false, false),
                ],
            },
        ];

        ensure_lines_fit_available_width(
            &ctx,
            false,
            &mut lines,
            0.0,
            &paragraph_style,
            3.0,
            &mut cache,
        )
        .expect("normalize widths");
        enforce_line_start_rules(&paragraph_style, &mut lines);

        assert_eq!(
            collect_line_infos(&lines)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>(),
            [
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "字".to_string(),
            ]
        );
        assert_eq!(
            lines.last().map(|line| line.break_kind),
            Some(LineBreakKind::ParagraphEnd)
        );
    }

    fn bidi_test_line(text: &str) -> WrappedLine {
        let graphemes = text.grapheme_indices(true).collect::<Vec<_>>();
        let clusters = graphemes
            .iter()
            .map(|(start, grapheme)| {
                InlineCluster::grapheme(
                    *start,
                    *start + grapheme.len(),
                    1.0,
                    grapheme,
                    grapheme.chars().all(char::is_whitespace),
                    false,
                    false,
                )
            })
            .collect();
        // Split every grapheme into its own differently formatted run. This is
        // deliberately harsher than normal imported documents and proves the
        // visual reorder preserves formatting across arbitrary run boundaries.
        let segments = graphemes
            .iter()
            .enumerate()
            .map(|(index, (start, grapheme))| TextSegment {
                text: grapheme.to_string(),
                start: *start,
                end: *start + grapheme.len(),
                properties: Some(Rc::new(RunProperties {
                    bold: Some(index.is_multiple_of(2)),
                    ..RunProperties::default()
                })),
                revision: None,
                hyperlink: None,
                leading_width: 0.0,
            })
            .collect();
        WrappedLine {
            segments,
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((text.len(), text.len())),
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            clusters,
        }
    }

    fn expected_bold_by_range(text: &str) -> FxHashMap<(usize, usize), bool> {
        text.grapheme_indices(true)
            .enumerate()
            .map(|(index, (start, grapheme))| {
                ((start, start + grapheme.len()), index.is_multiple_of(2))
            })
            .collect()
    }

    #[test]
    fn bidi_fast_path_skips_plain_ltr_and_cjk_but_keeps_directional_inputs() {
        assert!(!requires_bidi_resolution(
            &bidi_test_line("横浜ABC 123"),
            false
        ));
        assert!(requires_bidi_resolution(&bidi_test_line("ABC אבג"), false));
        assert!(requires_bidi_resolution(
            &bidi_test_line("ABC \u{202e}123"),
            false
        ));
        assert!(requires_bidi_resolution(
            &bidi_test_line("横浜ABC 123"),
            true
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn bidi_visual_order_matches_uax9_and_preserves_logical_ranges(
            chars in prop::collection::vec(
                prop_oneof![
                    Just('א'), Just('ב'), Just('ג'), Just('ش'), Just('م'), Just('ر'),
                    Just('A'), Just('b'), Just('Z'), Just('1'), Just('7'), Just(' '),
                    Just('-'), Just('.'), Just(','),
                ],
                1..64,
            ),
            base_rtl in any::<bool>(),
        ) {
            let text = chars.into_iter().collect::<String>();
            let mut line = bidi_test_line(&text);
            let expected_bold = expected_bold_by_range(&text);
            let logical_ranges = line
                .clusters
                .iter()
                .map(|cluster| (cluster.start, cluster.end))
                .collect::<Vec<_>>();
            let total_width = line.clusters.iter().map(|cluster| cluster.width).sum::<f64>();
            apply_bidi_visual_order(&mut line, base_rtl);

            let base = if base_rtl { Level::rtl() } else { Level::ltr() };
            let bidi = BidiInfo::new(&text, Some(base));
            let expected = bidi.reorder_line(&bidi.paragraphs[0], 0..text.len());
            let actual = line
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>();
            prop_assert_eq!(actual, expected.as_ref());

            let mut actual_ranges = line
                .clusters
                .iter()
                .map(|cluster| (cluster.start, cluster.end))
                .collect::<Vec<_>>();
            actual_ranges.sort_unstable();
            prop_assert_eq!(actual_ranges, logical_ranges);
            prop_assert_eq!(line.clusters.iter().map(|cluster| cluster.width).sum::<f64>(), total_width);
            prop_assert_eq!(line.segments.len(), line.clusters.len());
            for segment in &line.segments {
                let bold = segment
                    .properties
                    .as_ref()
                    .and_then(|properties| properties.bold);
                prop_assert_eq!(bold, expected_bold.get(&(segment.start, segment.end)).copied());
            }
        }
    }
}
