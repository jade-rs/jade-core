use crate::line_geometry::{logical_source_range, LineHorizontalMetrics};
use jade_docx_reader::document::PAGE_BREAK_STR;
pub use jade_selection::{SelectionRange, TextPosition};
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default)]
pub struct GlyphBox {
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub width: f64,
}

#[derive(Clone, Debug, Default)]
pub struct FragmentLayout {
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub width: f64,
    /// Painted baseline relative to the line baseline (super/subscript and w:position).
    pub baseline_shift_px: f64,
    pub glyphs: Rc<[GlyphBox]>,
}

impl FragmentLayout {
    pub fn relative_x_for_start(&self, offset: usize) -> f64 {
        if self.glyphs.is_empty() || offset <= self.start {
            return 0.0;
        }

        if offset >= self.end {
            if let Some(last) = self.glyphs.last() {
                return last.x + last.width;
            }
            return 0.0;
        }

        for glyph in self.glyphs.iter() {
            if offset <= glyph.start || (offset >= glyph.start && offset < glyph.end) {
                return glyph.x;
            }
        }

        0.0
    }

    pub fn relative_x_for_end(&self, offset: usize) -> f64 {
        if self.glyphs.is_empty() {
            return 0.0;
        }

        if offset <= self.start {
            return 0.0;
        }

        if offset >= self.end {
            if let Some(last) = self.glyphs.last() {
                return last.x + last.width;
            }
            return 0.0;
        }

        for (index, glyph) in self.glyphs.iter().enumerate() {
            if offset <= glyph.start {
                return glyph.x;
            }
            if offset <= glyph.end {
                let mut x = glyph.x + glyph.width;
                for following in self.glyphs.iter().skip(index + 1) {
                    if following.start == offset && following.end == offset {
                        x = following.x + following.width;
                        continue;
                    }
                    break;
                }
                return x;
            }
        }

        if let Some(last) = self.glyphs.last() {
            last.x + last.width
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TabLeaderLayout {
    pub start_x: f64,
    pub end_x: f64,
    pub leader: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TabStopLayout {
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_x: f64,
    pub end_x: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LineLayout {
    /// Zero-based section column containing this line.
    pub column_index: usize,
    pub top: f64,
    pub absolute_top: f64,
    pub height: f64,
    pub baseline: f64,
    pub start_x: f64,
    pub cursor_end_x: f64,
    /// Maximum visual overhang intentionally retained by the shared wrapper
    /// for the final cluster (a trailing space or eligible punctuation).
    pub allowed_trailing_overflow_width: f64,
    pub content_top: f64,
    pub absolute_content_top: f64,
    pub content_height: f64,
    pub fragments: Rc<[FragmentLayout]>,
    pub break_range: Option<(usize, usize)>,
    pub tab_leaders: Rc<[TabLeaderLayout]>,
    pub tab_stops: Rc<[TabStopLayout]>,
    /// True when this line is part of vertically-stacked text (OOXML
    /// `w:textDirection` = tbRl/tbRlV inside a table cell). Each such line
    /// holds a single grapheme and the lines are stacked along the Y axis, so
    /// caret/hit-test resolution within the line must walk Y instead of X.
    pub vertical: bool,
    /// Source offsets advance upward when a vertical line is bottom-to-top.
    pub vertical_reversed: bool,
}

#[cfg(target_arch = "wasm32")]
const _: () = assert!(std::mem::size_of::<LineLayout>() == 128);

#[derive(Clone, Debug, Serialize)]
pub struct SelectedLineDebug {
    pub paragraph: usize,
    pub top: f64,
    pub absolute_top: f64,
    pub height: f64,
    pub start_x: f64,
    pub end_x: f64,
    pub cursor_end_x: f64,
    pub visual_end_x: f64,
    pub start: usize,
    pub end: usize,
    pub has_fragments: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tab_leaders: Vec<TabLeaderLayout>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tab_stops: Vec<TabStopLayout>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayoutGlyphDebug {
    pub start: usize,
    pub end: usize,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub text: String,
    pub x: f64,
    pub width: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayoutFragmentDebug {
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub width: f64,
    /// Page-local baseline of the painted fragment, including its vertical shift.
    pub baseline: f64,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub glyphs: Vec<LayoutGlyphDebug>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayoutLineDebug {
    pub paragraph: usize,
    pub line: usize,
    pub page: usize,
    pub column_index: usize,
    /// True for a vertically stacked table-cell line.
    pub vertical: bool,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub top: f64,
    pub absolute_top: f64,
    pub height: f64,
    /// Where the line's glyphs are drawn, in the same space as `top` and
    /// `absolute_top`. Lets parity checks compare the painted baseline with
    /// Word's instead of the ink box, which depends on the face drawing it.
    pub baseline: f64,
    pub absolute_baseline: f64,
    /// Report-only shift already added to `start_x`, `end_x`, each fragment's
    /// `x` and each glyph's `x` so Word COM traces compare
    /// (`word_oracle_report_x_offsets`). A glyph is drawn at
    /// `fragment.x + glyph.x - 2 * report_x_offset`.
    pub report_x_offset: f64,
    pub start_x: f64,
    pub end_x: f64,
    pub cursor_end_x: f64,
    pub visual_end_x: f64,
    pub allowed_trailing_overflow_width: f64,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fragments: Vec<LayoutFragmentDebug>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tab_leaders: Vec<TabLeaderLayout>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tab_stops: Vec<TabStopLayout>,
}

#[derive(Clone, Debug, Default)]
pub struct ParagraphLayout {
    pub text: String,
    pub lines: Vec<LineLayout>,
    /// Cached bounding box for fast hit testing (min_y, max_y, min_x, max_x)
    /// Uses Cell for interior mutability so hit_test can use &self
    bounding_box: Cell<Option<(f64, f64, f64, f64)>>,
    /// The render pass that last registered this paragraph (see
    /// `SelectionManager::render_pass`).
    render_pass: u64,
}

impl LineLayout {
    /// Logical source extent; fragment order describes physical placement.
    #[inline]
    pub fn logical_source_range(&self) -> Option<(usize, usize)> {
        logical_source_range(self.fragments.iter().map(|f| (f.start, f.end))).or_else(|| {
            self.break_range
                .map(|(start, end)| (start.min(end), start.max(end)))
        })
    }

    fn horizontal_metrics(&self) -> LineHorizontalMetrics {
        LineHorizontalMetrics {
            start_x: self.start_x,
            cursor_end_x: self.cursor_end_x,
        }
    }

    pub fn cursor_end_x(&self) -> f64 {
        self.horizontal_metrics().cursor_end_x()
    }

    pub fn cursor_width(&self) -> f64 {
        self.horizontal_metrics().cursor_width()
    }

    pub fn visual_end_x(&self) -> f64 {
        // Vertical fragments carry inline advances along Y. The column's
        // recorded X bounds already describe its physical horizontal extent.
        if self.vertical {
            return self.cursor_end_x();
        }
        self.horizontal_metrics().visual_end_x(
            self.tab_stops
                .iter()
                .map(|tab_stop| (tab_stop.start_x, tab_stop.end_x))
                .chain(
                    self.fragments
                        .iter()
                        .map(|fragment| (fragment.x, fragment.x + fragment.width)),
                ),
        )
    }

    fn interactive_max_x(&self) -> f64 {
        if self.fragments.is_empty() {
            self.cursor_end_x()
        } else {
            self.visual_end_x().max(self.start_x)
        }
    }
}

impl ParagraphLayout {
    /// Returns the bounding box (min_y, max_y, min_x, max_x) of this paragraph.
    /// Computes and caches the result on first call.
    /// Uses interior mutability to allow caching with &self.
    pub fn bounding_box(&self) -> Option<(f64, f64, f64, f64)> {
        if let Some(bbox) = self.bounding_box.get() {
            return Some(bbox);
        }

        if self.lines.is_empty() {
            return None;
        }

        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;

        for line in &self.lines {
            let top = line.absolute_top;
            let bottom = top + line.height.max(1.0);
            min_y = min_y.min(top);
            max_y = max_y.max(bottom);
            min_x = min_x.min(line.start_x);
            max_x = max_x.max(line.visual_end_x());
        }

        let bbox = (min_y, max_y, min_x, max_x);
        self.bounding_box.set(Some(bbox));
        Some(bbox)
    }

    /// Invalidates the cached bounding box. Call this when lines are modified.
    pub fn invalidate_bounding_box(&mut self) {
        self.bounding_box.set(None);
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CellLayout {
    pub paragraphs: Vec<usize>,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub table_id: usize,
    pub row: usize,
    pub column: usize,
    pub column_span: usize,
    pub order: usize,
}

const SOFT_BREAK_MIN_HIT_WIDTH: f64 = 8.0;
const CARET_THICKNESS: f64 = 1.5;
const CELL_SELECTION_GUTTER_WIDTH: f64 = 8.0;

#[derive(Clone, Debug, Default)]
struct CellSelectionState {
    anchor_cell: usize,
    focus_cell: usize,
}

/// Which line a caret on a soft wrap is drawn on: the offset that ends one
/// line also starts the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaretAffinity {
    /// At the end of the line above — a click past a line's end, or End.
    #[default]
    Upstream,
    /// At the start of the line below — a click at a line's start, Home.
    Downstream,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionManager {
    paragraphs: Vec<ParagraphLayout>,
    selection: Option<SelectionRange>,
    /// The line a collapsed caret on a soft wrap is drawn on.
    caret_affinity: CaretAffinity,
    /// Per-line caret origins that differ from the interactive line bounds.
    /// Empty table cells use the full cell for hit testing, while insertion
    /// starts after the resolved cell padding/alignment.
    caret_x_overrides: HashMap<(usize, usize), f64>,
    cells: Vec<CellLayout>,
    paragraph_to_cell: HashMap<usize, usize>,
    cell_selection: Option<CellSelectionState>,
    non_selectable_paragraphs: HashSet<usize>,
    /// When true, all mutation methods (begin_render, ensure_paragraph,
    /// record_line, etc.) are no-ops. This is used during page-bitmap
    /// re-renders to prevent selection data from being cleared/corrupted.
    frozen: bool,
    paragraph_bounds_index: RefCell<Option<Vec<(usize, f64, f64)>>>,
    /// How many leading paragraphs the current render pass kept from the
    /// previous one (`begin_render_incremental`). Cached replays of those
    /// paragraphs must not register their text and lines a second time.
    preserved_prefix_len: usize,
    /// Counts `begin_render*` calls. A paragraph kept from an earlier pass
    /// and then registered again in the current one (a replayed prefix, a
    /// table whose other cells are re-laid out, a footnote drawn at the page
    /// end) starts from empty on its first touch instead of appending: the
    /// text and lines it carried describe the same paragraph, and appending
    /// grew the registry with every edit and made selected text repeat.
    render_pass: u64,
    #[cfg(test)]
    paragraph_bounds_build_count: Cell<usize>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CaretMetrics {
    pub x: f64,
    pub top: f64,
    pub absolute_top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Direction of caret movement triggered by keyboard navigation.
pub enum CaretMovement {
    Left,
    Right,
    Up,
    Down,
    /// Jump the focus to the very first byte offset of the current paragraph.
    /// Used by Emacs-style `Ctrl+A` (configurable via `Options::set_shortcuts`).
    ParagraphStart,
    /// Jump the focus to the end-of-text byte offset of the current paragraph.
    /// Used by Emacs-style `Ctrl+E` (configurable via `Options::set_shortcuts`).
    ParagraphEnd,
    /// Word's Home: the first offset of the caret's laid-out line.
    LineStart,
    /// Word's End: the offset after the last character of the caret's line
    /// (before a soft wrap, the caret sits at the break).
    LineEnd,
    /// Word's Ctrl+Left: the start of the previous word.
    WordLeft,
    /// Word's Ctrl+Right: the start of the next word.
    WordRight,
    /// Word for Mac's Cmd+Up: the start of the current paragraph, or of the
    /// previous paragraph when the caret is already at a paragraph start.
    ParagraphPrevious,
    /// Word for Mac's Cmd+Down: the start of the next paragraph (the end of
    /// the last one).
    ParagraphNext,
    /// Word's Ctrl+Home: the very start of the document.
    DocumentStart,
    /// Word's Ctrl+End: the very end of the document.
    DocumentEnd,
}

impl SelectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Freeze the selection manager. While frozen, all render-time mutation
    /// methods (begin_render, ensure_paragraph, record_line, etc.) are no-ops.
    /// This is used during page-bitmap re-renders to prevent selection data
    /// from being cleared or corrupted.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Unfreeze the selection manager, re-enabling render-time mutations.
    pub fn unfreeze(&mut self) {
        self.frozen = false;
    }

    /// Whether render-time mutations are currently ignored. A nested measuring
    /// pass restores this instead of unconditionally unfreezing, so it cannot
    /// thaw an enclosing frozen re-render.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn begin_render(&mut self) {
        if self.frozen {
            return;
        }
        let selection = self.selection;
        self.render_pass = self.render_pass.wrapping_add(1);
        self.paragraphs.clear();
        self.caret_x_overrides.clear();
        *self.paragraph_bounds_index.get_mut() = None;
        self.selection = selection;
        *self.paragraph_bounds_index.get_mut() = None;
        self.cells.clear();
        self.paragraph_to_cell.clear();
        self.non_selectable_paragraphs.clear();
        self.preserved_prefix_len = 0;
    }

    /// The number of leading paragraphs the current pass preserved from the
    /// previous render (0 after a full `begin_render`).
    pub fn preserved_prefix_len(&self) -> usize {
        self.preserved_prefix_len
    }

    /// Move the untouched tail out before an edit render that may stop after
    /// its changed paragraph. The caller restores it only after proving that
    /// the downstream layout is unchanged.
    pub fn take_paragraph_suffix(&mut self, from: usize) -> Vec<ParagraphLayout> {
        if from >= self.paragraphs.len() {
            return Vec::new();
        }
        *self.paragraph_bounds_index.get_mut() = None;
        self.paragraphs.split_off(from)
    }

    pub fn restore_paragraph_suffix(&mut self, from: usize, suffix: Vec<ParagraphLayout>) {
        if suffix.is_empty() {
            return;
        }
        // Nested stories can re-record layouts beyond the edited body
        // paragraph before the stable-edit exit. The saved suffix is the
        // authoritative unchanged tail, so replace any such overlap.
        self.paragraphs.truncate(from);
        *self.paragraph_bounds_index.get_mut() = None;
        self.paragraphs.extend(suffix);
    }

    /// Begins an incremental render pass, preserving paragraphs before `dirty_from`.
    ///
    /// If `dirty_from` is `None`, behaves like `begin_render()` (full clear).
    /// If `dirty_from` is `Some(index)`, paragraphs 0..index are preserved
    /// (for prefix replay optimization), and paragraphs index.. are cleared.
    pub fn begin_render_incremental(&mut self, dirty_from: Option<usize>) {
        if self.frozen {
            return;
        }
        let selection = self.selection;
        self.render_pass = self.render_pass.wrapping_add(1);

        match dirty_from {
            Some(from) if from > 0 && from <= self.paragraphs.len() => {
                // Keep paragraphs 0..from, clear from..
                self.preserved_prefix_len = from;
                self.paragraphs.truncate(from);
                self.caret_x_overrides
                    .retain(|(paragraph_index, _), _| *paragraph_index < from);
                // Clear cells and paragraph_to_cell for affected range
                // For simplicity, we clear all cell data since cells may span paragraphs
                // TODO: smarter cell invalidation if needed
                self.cells.clear();
                self.paragraph_to_cell.clear();
                self.non_selectable_paragraphs
                    .retain(|paragraph_index| *paragraph_index < from);
            }
            _ => {
                // Full clear
                self.preserved_prefix_len = 0;
                self.paragraphs.clear();
                self.caret_x_overrides.clear();
                self.cells.clear();
                self.paragraph_to_cell.clear();
                self.non_selectable_paragraphs.clear();
            }
        }

        self.selection = selection;
    }

    /// Applies a Y-offset to all paragraphs from `from_index` onward.
    ///
    /// This is used when a paragraph break shifts downstream content without
    /// changing its layout. Instead of recalculating selection data, we just
    /// shift the Y coordinates.
    pub fn apply_y_offset_from(&mut self, from_index: usize, y_offset: f64) {
        const EPS: f64 = 0.001;
        if y_offset.abs() < EPS {
            return;
        }
        *self.paragraph_bounds_index.get_mut() = None;

        for paragraph in self.paragraphs.iter_mut().skip(from_index) {
            for line in &mut paragraph.lines {
                line.top += y_offset;
                line.absolute_top += y_offset;
                line.baseline += y_offset;
                line.content_top += y_offset;
                line.absolute_content_top += y_offset;
            }
            // Update bounding box if present (Cell interior mutability)
            if let Some((min_y, max_y, min_x, max_x)) = paragraph.bounding_box.get() {
                paragraph.bounding_box.set(Some((
                    min_y + y_offset,
                    max_y + y_offset,
                    min_x,
                    max_x,
                )));
            }
        }
    }

    /// Inserts a gap at the given index for a new paragraph.
    ///
    /// Used when a paragraph break creates a new paragraph. The new paragraph
    /// slot is initialized empty, and downstream paragraphs are shifted.
    pub fn insert_paragraph_gap(&mut self, index: usize) {
        *self.paragraph_bounds_index.get_mut() = None;
        if index > self.paragraphs.len() {
            self.paragraphs
                .resize_with(index + 1, ParagraphLayout::default);
        } else {
            self.paragraphs.insert(index, ParagraphLayout::default());
        }

        self.caret_x_overrides = std::mem::take(&mut self.caret_x_overrides)
            .into_iter()
            .map(|((paragraph_index, line_index), x)| {
                let shifted = if paragraph_index >= index {
                    paragraph_index + 1
                } else {
                    paragraph_index
                };
                ((shifted, line_index), x)
            })
            .collect();

        // Update paragraph_to_cell mapping for shifted indices
        let shifted: Vec<(usize, usize)> = self
            .paragraph_to_cell
            .iter()
            .filter(|(&p, _)| p >= index)
            .map(|(&p, &c)| (p, c))
            .collect();
        for (old_idx, cell_id) in shifted {
            self.paragraph_to_cell.remove(&old_idx);
            self.paragraph_to_cell.insert(old_idx + 1, cell_id);
        }
    }

    /// Removes a paragraph at the given index (for paragraph merge).
    ///
    /// Downstream paragraphs are shifted down to fill the gap.
    pub fn remove_paragraph(&mut self, index: usize) {
        *self.paragraph_bounds_index.get_mut() = None;
        if index < self.paragraphs.len() {
            self.paragraphs.remove(index);
        }

        self.caret_x_overrides = std::mem::take(&mut self.caret_x_overrides)
            .into_iter()
            .filter_map(|((paragraph_index, line_index), x)| {
                if paragraph_index == index {
                    return None;
                }
                let shifted = if paragraph_index > index {
                    paragraph_index - 1
                } else {
                    paragraph_index
                };
                Some(((shifted, line_index), x))
            })
            .collect();

        // Update paragraph_to_cell mapping for shifted indices
        self.paragraph_to_cell.remove(&index);
        let shifted: Vec<(usize, usize)> = self
            .paragraph_to_cell
            .iter()
            .filter(|(&p, _)| p > index)
            .map(|(&p, &c)| (p, c))
            .collect();
        for (old_idx, cell_id) in shifted {
            self.paragraph_to_cell.remove(&old_idx);
            self.paragraph_to_cell.insert(old_idx - 1, cell_id);
        }
    }

    fn ensure_paragraph_internal(&mut self, index: usize) -> &mut ParagraphLayout {
        *self.paragraph_bounds_index.get_mut() = None;
        if self.paragraphs.len() <= index {
            let render_pass = self.render_pass;
            self.paragraphs.resize_with(index + 1, || ParagraphLayout {
                render_pass,
                ..ParagraphLayout::default()
            });
        }
        let paragraph = &mut self.paragraphs[index];
        if paragraph.render_pass != self.render_pass {
            // First touch in this pass of a paragraph kept from an earlier
            // one: it is being registered afresh, so drop the old copy.
            paragraph.text.clear();
            paragraph.lines.clear();
            paragraph.invalidate_bounding_box();
            paragraph.render_pass = self.render_pass;
        }
        paragraph
    }

    pub fn ensure_paragraph(&mut self, index: usize) {
        if self.frozen {
            return;
        }
        let _ = self.ensure_paragraph_internal(index);
    }

    /// Start a complete logical paragraph registration.
    ///
    /// The renderer can revisit a paragraph within one render pass (for
    /// example while placing page footnotes). Each visit describes the whole
    /// paragraph, so discard the previous visit before recording its text and
    /// page-by-page lines again.
    pub fn reset_paragraph_registration(&mut self, index: usize) {
        if self.frozen {
            return;
        }
        let paragraph = self.ensure_paragraph_internal(index);
        paragraph.text.clear();
        paragraph.lines.clear();
        paragraph.invalidate_bounding_box();
    }

    /// Continue a logical paragraph on another page without dropping its
    /// earlier lines. Re-entering the same tail replaces only that tail.
    pub fn continue_paragraph_registration(&mut self, index: usize, text: &str, start: usize) {
        if self.frozen {
            return;
        }
        let paragraph = self.ensure_paragraph_internal(index);
        paragraph.text.clear();
        paragraph.text.push_str(text);
        paragraph.lines.retain(|line| {
            let first = line
                .logical_source_range()
                .map(|range| range.0)
                .unwrap_or(0);
            first < start
        });
        let keep = paragraph.lines.len();
        paragraph.invalidate_bounding_box();
        self.caret_x_overrides
            .retain(|&(paragraph, line), _| paragraph != index || line < keep);
    }

    pub fn set_paragraph_selectable(&mut self, index: usize, selectable: bool) {
        if self.frozen {
            return;
        }
        let _ = self.ensure_paragraph_internal(index);
        if selectable {
            self.non_selectable_paragraphs.remove(&index);
        } else {
            self.non_selectable_paragraphs.insert(index);
        }
    }

    fn is_paragraph_selectable(&self, index: usize) -> bool {
        index < self.paragraphs.len() && !self.non_selectable_paragraphs.contains(&index)
    }

    fn nearest_selectable_paragraph(&self, index: usize) -> Option<usize> {
        if self.paragraphs.is_empty() {
            return None;
        }
        let clamped = index.min(self.paragraphs.len().saturating_sub(1));
        if self.is_paragraph_selectable(clamped) {
            return Some(clamped);
        }
        for candidate in (0..clamped).rev() {
            if self.is_paragraph_selectable(candidate) {
                return Some(candidate);
            }
        }
        (clamped + 1..self.paragraphs.len())
            .find(|&candidate| self.is_paragraph_selectable(candidate))
    }

    pub fn append_paragraph_text(&mut self, index: usize, text: &str) {
        if self.frozen {
            return;
        }
        let paragraph = self.ensure_paragraph_internal(index);
        if !text.is_empty() {
            paragraph.text.push_str(text);
        }
    }

    pub fn record_table_cell(&mut self, cell: CellLayout) {
        self.record_table_cell_impl(cell, false);
    }

    /// Register one physical fragment of a cell that can span pages.
    /// Distinct bounds must survive even when their logical paragraphs match.
    pub fn record_table_cell_fragment(&mut self, cell: CellLayout) {
        self.record_table_cell_impl(cell, true);
    }

    fn record_table_cell_impl(&mut self, mut cell: CellLayout, fragment: bool) {
        if self.frozen {
            return;
        }
        if cell.paragraphs.is_empty() {
            return;
        }

        let (min_x, max_x) = if cell.left <= cell.right {
            (cell.left, cell.right)
        } else {
            (cell.right, cell.left)
        };
        let (min_y, max_y) = if cell.top <= cell.bottom {
            (cell.top, cell.bottom)
        } else {
            (cell.bottom, cell.top)
        };
        cell.left = min_x;
        cell.right = max_x;
        cell.top = min_y;
        cell.bottom = max_y;
        if cell.column_span == 0 {
            cell.column_span = 1;
        }

        cell.paragraphs.sort_unstable();
        cell.paragraphs.dedup();
        let paragraph_indices = cell.paragraphs.clone();
        // The same cell laid out again in one render pass (a lazy-page
        // re-entry) replaces its earlier record rather than adding a twin.
        let existing = paragraph_indices
            .first()
            .and_then(|index| self.paragraph_to_cell.get(index).copied())
            .filter(|id| {
                self.cells.get(*id).is_some_and(|recorded| {
                    recorded.paragraphs == paragraph_indices
                        && (!fragment
                            || (recorded.table_id == cell.table_id
                                && recorded.row == cell.row
                                && recorded.column == cell.column
                                && recorded.column_span == cell.column_span))
                })
            });
        if let Some(existing) = existing {
            let recorded = &self.cells[existing];
            if fragment {
                // Physical paint order advances on every page fragment. Keep
                // the source cell's order so keyboard navigation cannot walk
                // back into another fragment of that same cell.
                cell.order = recorded.order;
            }
            if !fragment
                || (recorded.left == cell.left
                    && recorded.right == cell.right
                    && recorded.top == cell.top
                    && recorded.bottom == cell.bottom)
            {
                self.cells[existing] = cell;
                return;
            }
        }
        let cell_id = self.cells.len();
        self.cells.push(cell);
        for index in paragraph_indices {
            self.paragraph_to_cell.insert(index, cell_id);
        }
    }

    /// Add another physical fragment of the same logical table cell. Its
    /// paragraphs/order stay shared, while hit testing retains both bounds.
    pub fn record_table_cell_continuation(&mut self, mut cell: CellLayout) {
        if self.frozen || cell.paragraphs.is_empty() {
            return;
        }
        // Paragraph ids already identify source cells in document order. The
        // common snapshot replay path must not scan all preceding cells.
        let existing = cell
            .paragraphs
            .first()
            .and_then(|paragraph| self.paragraph_to_cell.get(paragraph).copied())
            .filter(|id| {
                self.cells.get(*id).is_some_and(|recorded| {
                    recorded.table_id == cell.table_id
                        && recorded.row == cell.row
                        && recorded.column == cell.column
                        && recorded.column_span == cell.column_span
                })
            })
            .or_else(|| {
                // Nested-table rows assign paragraph ids while painting. A
                // later segment can therefore start with a new paragraph id.
                self.cells.iter().position(|recorded| {
                    recorded.table_id == cell.table_id
                        && recorded.row == cell.row
                        && recorded.column == cell.column
                        && recorded.column_span == cell.column_span
                })
            });
        let Some(existing) = existing else {
            self.record_table_cell(cell);
            return;
        };
        let (left, right) = (cell.left.min(cell.right), cell.left.max(cell.right));
        let (top, bottom) = (cell.top.min(cell.bottom), cell.top.max(cell.bottom));
        cell.left = left;
        cell.right = right;
        cell.top = top;
        cell.bottom = bottom;
        cell.order = self.cells[existing].order;
        let mut paragraphs = self.cells[existing].paragraphs.clone();
        paragraphs.extend(cell.paragraphs.iter().copied());
        paragraphs.sort_unstable();
        paragraphs.dedup();
        cell.paragraphs = paragraphs.clone();
        for recorded in &mut self.cells {
            if recorded.table_id == cell.table_id
                && recorded.row == cell.row
                && recorded.column == cell.column
                && recorded.column_span == cell.column_span
            {
                recorded.paragraphs = paragraphs.clone();
            }
        }
        for paragraph in paragraphs {
            self.paragraph_to_cell.insert(paragraph, existing);
        }
        if let Some(same_bounds) = self.cells.iter().position(|recorded| {
            recorded.table_id == cell.table_id
                && recorded.row == cell.row
                && recorded.column == cell.column
                && recorded.column_span == cell.column_span
                && recorded.top == cell.top
                && recorded.bottom == cell.bottom
                && recorded.left == cell.left
                && recorded.right == cell.right
        }) {
            self.cells[same_bounds] = cell;
        } else {
            self.cells.push(cell);
        }
    }

    pub fn cell_id_for_paragraph(&self, paragraph_index: usize) -> Option<usize> {
        self.paragraph_to_cell.get(&paragraph_index).copied()
    }

    pub fn cell_layout(&self, cell_id: usize) -> Option<&CellLayout> {
        self.cells.get(cell_id)
    }

    pub fn is_cell_fully_selected(&self, cell_id: usize, range: &SelectionRange) -> bool {
        let Some(cell_start) = self.cell_start_position(cell_id) else {
            return false;
        };
        let Some(cell_end) = self.cell_end_position(cell_id) else {
            return false;
        };
        let (start, end) = range.normalized();

        start <= cell_start && end >= cell_end
    }

    fn cell_selection_bounds(&self) -> Option<(usize, usize)> {
        self.cell_selection.as_ref().map(|state| {
            if state.anchor_cell <= state.focus_cell {
                (state.anchor_cell, state.focus_cell)
            } else {
                (state.focus_cell, state.anchor_cell)
            }
        })
    }

    pub fn cell_selection_span(&self) -> Option<(usize, usize)> {
        self.cell_selection_bounds()
    }

    /// Computes the set of cells inside the rectangular (row × column) region
    /// spanned by the current `cell_selection` anchor/focus, restricted to the
    /// anchor's `table_id`. Used for Word-like mouse-drag selections where the
    /// user expects only the middle 2 columns to be highlighted when dragging
    /// across the middle of a multi-column table, rather than every cell that
    /// falls between anchor and focus in `order`.
    pub fn selected_cell_ids_rect(&self) -> Option<Vec<usize>> {
        let state = self.cell_selection.as_ref()?;
        let anchor = self.cells.get(state.anchor_cell)?;
        let focus = self.cells.get(state.focus_cell)?;
        if anchor.table_id != focus.table_id {
            return None;
        }
        let table_id = anchor.table_id;

        let cell_col_range = |cell: &CellLayout| {
            let span = cell.column_span.max(1);
            (cell.column, cell.column + span - 1)
        };
        let (a_c0, a_c1) = cell_col_range(anchor);
        let (f_c0, f_c1) = cell_col_range(focus);

        let r0 = anchor.row.min(focus.row);
        let r1 = anchor.row.max(focus.row);
        let c0 = a_c0.min(f_c0);
        let c1 = a_c1.max(f_c1);

        let mut ids: Vec<(usize, usize)> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| {
                if cell.table_id != table_id {
                    return false;
                }
                if cell.row < r0 || cell.row > r1 {
                    return false;
                }
                let (cs, ce) = cell_col_range(cell);
                cs <= c1 && ce >= c0
            })
            .map(|(id, cell)| (id, cell.order))
            .collect();
        ids.sort_by_key(|(_, order)| *order);
        if ids.is_empty() {
            None
        } else {
            Some(ids.into_iter().map(|(id, _)| id).collect())
        }
    }

    pub fn selected_cell_ids(&self) -> Option<Vec<usize>> {
        if let Some(ids) = self.selected_cell_ids_rect() {
            return Some(ids);
        }

        let range = self.selection?;
        let cells = self.infer_selected_cells_from_text(&range)?;
        if cells.is_empty() {
            None
        } else {
            Some(cells)
        }
    }

    pub fn selected_cell_layouts(&self) -> Option<Vec<CellLayout>> {
        let ids = self.selected_cell_ids()?;
        let mut layouts = Vec::new();
        for id in ids {
            if let Some(layout) = self.cells.get(id) {
                layouts.push(layout.clone());
            }
        }
        if layouts.is_empty() {
            None
        } else {
            Some(layouts)
        }
    }

    /// One layout per logical source cell for editing commands. Painting uses
    /// `selected_cell_layouts` so every physical page rectangle stays visible.
    pub fn selected_logical_cell_layouts(&self) -> Option<Vec<CellLayout>> {
        let mut cells = self.selected_cell_layouts()?;
        cells.sort_by_key(|cell| {
            (
                cell.table_id,
                cell.order,
                cell.row,
                cell.column,
                cell.column_span,
            )
        });
        cells.dedup_by_key(|cell| (cell.table_id, cell.row, cell.column, cell.column_span));
        Some(cells)
    }

    pub fn cell_layouts_debug(&self) -> Vec<CellLayout> {
        self.cells.clone()
    }

    /// Every recorded cell, indexed by the cell id
    /// [`Self::cell_id_for_paragraph`] returns.
    pub fn cell_layouts(&self) -> &[CellLayout] {
        &self.cells
    }

    fn cell_text(&self, cell: &CellLayout) -> String {
        let mut text = String::new();
        for (idx, paragraph_index) in cell.paragraphs.iter().enumerate() {
            if let Some(paragraph) = self.paragraphs.get(*paragraph_index) {
                if idx > 0 {
                    text.push('\n');
                }
                text.push_str(&paragraph.text);
            }
        }
        text
    }

    fn infer_selected_cells_from_text(&self, range: &SelectionRange) -> Option<Vec<usize>> {
        let (start, end) = range.normalized();
        let anchor_cell = self.cell_id_for_paragraph(start.paragraph)?;
        let anchor_layout = self.cells.get(anchor_cell)?;
        let table_id = anchor_layout.table_id;
        let focus_cell = self.cell_id_for_paragraph(end.paragraph)?;
        if self.cells.get(focus_cell)?.table_id != table_id {
            return None;
        }
        let selection_text = self.selection_text()?.trim_end_matches('\n').to_string();
        if selection_text.is_empty() {
            return None;
        }
        let forward = start <= end;

        let mut candidates: Vec<(usize, &CellLayout)> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.table_id == table_id)
            .collect();
        candidates.sort_by_key(|(_, cell)| cell.order);
        // Continued page rectangles carry the same full logical text. Count
        // each source cell once when inferring a selection from that text.
        candidates.dedup_by_key(|(_, cell)| (cell.row, cell.column, cell.column_span));

        if forward {
            let start_order = anchor_layout.order;
            let mut buffer = String::new();
            let mut selected = Vec::new();
            for (cell_id, cell) in candidates
                .into_iter()
                .filter(|(_, cell)| cell.order >= start_order)
            {
                let cell_text = self.cell_text(cell);
                if cell_text.is_empty() && buffer.is_empty() {
                    continue;
                }
                if !buffer.is_empty() {
                    buffer.push('\n');
                }
                buffer.push_str(&cell_text);
                selected.push(cell_id);
                let normalized = buffer.trim_matches('\n');
                if normalized.len() >= selection_text.len() {
                    return Some(selected);
                }
            }
        } else {
            let start_order = anchor_layout.order;
            let mut buffer = String::new();
            let mut selected = Vec::new();
            for (cell_id, cell) in candidates
                .into_iter()
                .filter(|(_, cell)| cell.order <= start_order)
                .rev()
            {
                let cell_text = self.cell_text(cell);
                if cell_text.is_empty() && buffer.is_empty() {
                    continue;
                }
                if !buffer.is_empty() {
                    buffer.insert(0, '\n');
                }
                buffer.insert_str(0, &cell_text);
                selected.push(cell_id);
                let normalized = buffer.trim_matches('\n');
                if normalized.len() >= selection_text.len() {
                    selected.reverse();
                    return Some(selected);
                }
            }
        }

        None
    }

    pub fn paragraph_in_selected_cell(&self, paragraph_index: usize) -> bool {
        if let Some(ids) = self.selected_cell_ids_rect() {
            let set: HashSet<usize> = ids.into_iter().collect();
            return self
                .paragraph_to_cell
                .get(&paragraph_index)
                .is_some_and(|cell| set.contains(cell));
        }

        let Some(range) = self.selection else {
            return false;
        };

        if let Some(cell_id) = self.paragraph_to_cell.get(&paragraph_index) {
            if self.is_cell_fully_selected(*cell_id, &range) {
                return true;
            }
        }

        let Some(cells) = self.infer_selected_cells_from_text(&range) else {
            return false;
        };

        let mut set = HashSet::new();
        for cell_id in cells {
            set.insert(cell_id);
        }

        self.paragraph_to_cell
            .get(&paragraph_index)
            .is_some_and(|cell| set.contains(cell))
    }

    pub fn clear_cell_selection(&mut self) {
        self.cell_selection = None;
    }

    fn start_cell_selection(&mut self, anchor_cell: usize, focus_cell: usize) {
        self.cell_selection = Some(CellSelectionState {
            anchor_cell,
            focus_cell,
        });
    }

    /// Public entry for starting or restarting a cell-range selection from the
    /// pointer path. `anchor_cell` and `focus_cell` are physical rectangle
    /// indices into `self.cells`; `CellLayout.order` identifies logical order.
    pub fn set_cell_selection(&mut self, anchor_cell: usize, focus_cell: usize) {
        self.start_cell_selection(anchor_cell, focus_cell);
    }

    /// Update the focus end of the active cell selection. No-op if no cell
    /// selection is currently active. The anchor side is preserved.
    pub fn update_cell_selection_focus(&mut self, focus_cell: usize) -> bool {
        if let Some(state) = self.cell_selection.as_mut() {
            state.focus_cell = focus_cell;
            true
        } else {
            false
        }
    }

    /// Resolve the physical cell rectangle whose geometric bounds contain
    /// the given point. Used by pointer drag to map canvas coordinates to the
    /// cell currently under the cursor.
    pub fn cell_at_point(&self, x: f64, y: f64) -> Option<usize> {
        self.cells.iter().enumerate().find_map(|(idx, cell)| {
            if x >= cell.left && x <= cell.right && y >= cell.top && y <= cell.bottom {
                Some(idx)
            } else {
                None
            }
        })
    }

    /// Resolve a Word-like cell-selection hotspot at the leading edge of a
    /// cell. Normal clicks farther inside the cell remain text/caret clicks.
    pub fn cell_selection_target_at_point(&self, x: f64, y: f64) -> Option<usize> {
        self.cells.iter().enumerate().find_map(|(idx, cell)| {
            let gutter_right = (cell.left + CELL_SELECTION_GUTTER_WIDTH).min(cell.right);
            if x >= cell.left && x <= gutter_right && y >= cell.top && y <= cell.bottom {
                Some(idx)
            } else {
                None
            }
        })
    }

    /// Resolve the cell inside `table_id` whose bounding box is closest to
    /// the given point. When the drag cursor leaves the table geometry we
    /// still want the rectangular cell selection to grow — e.g., dragging
    /// down past the bottom row should extend to the last row in the
    /// anchor's column rather than collapsing back to a linear text range
    /// that paints whole rows of body paragraphs beneath.
    ///
    /// The squared distance is computed from the clamped point: if `(x, y)`
    /// lies inside the cell rect the distance is zero, otherwise it is the
    /// distance from `(x, y)` to the nearest point on the rect. Ties are
    /// broken by `CellLayout.order` to give deterministic results.
    pub fn nearest_cell_in_table(&self, table_id: usize, x: f64, y: f64) -> Option<usize> {
        let mut best: Option<(f64, usize, usize)> = None;
        for (idx, cell) in self.cells.iter().enumerate() {
            if cell.table_id != table_id {
                continue;
            }
            let clamped_x = x.clamp(cell.left.min(cell.right), cell.left.max(cell.right));
            let clamped_y = y.clamp(cell.top.min(cell.bottom), cell.top.max(cell.bottom));
            let dx = x - clamped_x;
            let dy = y - clamped_y;
            let dist_sq = dx * dx + dy * dy;
            let replace = match best {
                None => true,
                Some((best_dist, _, best_order)) => {
                    dist_sq < best_dist || (dist_sq == best_dist && cell.order < best_order)
                }
            };
            if replace {
                best = Some((dist_sq, idx, cell.order));
            }
        }
        best.map(|(_, idx, _)| idx)
    }

    /// Return the `table_id` the given cell belongs to, if any.
    pub fn cell_table_id(&self, cell_id: usize) -> Option<usize> {
        self.cells.get(cell_id).map(|cell| cell.table_id)
    }

    /// Expand the current cell-range selection to cover every cell in
    /// `table_id`. Used when a pointer drag leaves the table geometry —
    /// Word's behavior is to convert the in-progress rectangular selection
    /// to "entire table selected" and continue the drag into the body
    /// paragraphs before/after the table.
    ///
    /// Returns `true` if a cell selection was established, `false` if the
    /// table has no cells.
    pub fn select_entire_table(&mut self, table_id: usize) -> bool {
        let mut anchor: Option<(usize, usize, usize)> = None; // (row, col_start, cell_id)
        let mut focus: Option<(usize, usize, usize)> = None; // (row, col_end, cell_id)
        for (idx, cell) in self.cells.iter().enumerate() {
            if cell.table_id != table_id {
                continue;
            }
            let col_start = cell.column;
            let col_end = cell.column + cell.column_span.max(1) - 1;
            let min_key = (cell.row, col_start);
            if anchor.is_none_or(|(r, c, _)| min_key < (r, c)) {
                anchor = Some((cell.row, col_start, idx));
            }
            let max_key = (cell.row, col_end);
            if focus.is_none_or(|(r, c, _)| max_key > (r, c)) {
                focus = Some((cell.row, col_end, idx));
            }
        }
        match (anchor, focus) {
            (Some((_, _, a)), Some((_, _, f))) => {
                self.cell_selection = Some(CellSelectionState {
                    anchor_cell: a,
                    focus_cell: f,
                });
                true
            }
            _ => false,
        }
    }

    fn cell_start_position(&self, cell_id: usize) -> Option<TextPosition> {
        let cell = self.cells.get(cell_id)?;
        let first = *cell.paragraphs.first()?;
        Some(TextPosition::new(first, 0))
    }

    fn cell_end_position(&self, cell_id: usize) -> Option<TextPosition> {
        let cell = self.cells.get(cell_id)?;
        let last = *cell.paragraphs.last()?;
        let len = self
            .paragraphs
            .get(last)
            .map(|paragraph| paragraph.text.len())
            .unwrap_or(0);
        Some(TextPosition::new(last, len))
    }

    /// Return a linear text range that fully covers the anchor and focus
    /// cells. Rectangular painting is still driven by `cell_selection`.
    pub fn cell_selection_range(
        &self,
        anchor_cell: usize,
        focus_cell: usize,
    ) -> Option<SelectionRange> {
        let anchor = self.cells.get(anchor_cell)?;
        let focus = self.cells.get(focus_cell)?;
        if anchor.table_id != focus.table_id {
            return None;
        }

        if anchor.order <= focus.order {
            Some(SelectionRange::new(
                self.cell_start_position(anchor_cell)?,
                self.cell_end_position(focus_cell)?,
            ))
        } else {
            Some(SelectionRange::new(
                self.cell_end_position(anchor_cell)?,
                self.cell_start_position(focus_cell)?,
            ))
        }
    }

    fn is_at_cell_start(&self, position: TextPosition, cell_id: usize) -> bool {
        let Some(cell) = self.cells.get(cell_id) else {
            return false;
        };
        let Some(&first_paragraph) = cell.paragraphs.first() else {
            return true;
        };
        position.paragraph == first_paragraph && position.offset == 0
    }

    fn is_at_cell_end(&self, position: TextPosition, cell_id: usize) -> bool {
        let Some(cell) = self.cells.get(cell_id) else {
            return false;
        };
        let mut target_paragraph = None;
        for &idx in cell.paragraphs.iter().rev() {
            if let Some(paragraph) = self.paragraphs.get(idx) {
                if !paragraph.text.is_empty() {
                    target_paragraph = Some(idx);
                    break;
                }
            }
        }
        let paragraph_index = if let Some(idx) = target_paragraph {
            idx
        } else if let Some(&last) = cell.paragraphs.last() {
            last
        } else {
            return true;
        };
        if position.paragraph != paragraph_index {
            return false;
        }
        let len = self
            .paragraphs
            .get(paragraph_index)
            .map(|paragraph| paragraph.text.len())
            .unwrap_or(0);
        position.offset >= len
    }

    fn next_cell_in_same_table(&self, cell_id: usize) -> Option<usize> {
        let current = self.cells.get(cell_id)?;
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.table_id == current.table_id && cell.order > current.order)
            .min_by_key(|(_, cell)| cell.order)
            .map(|(id, _)| id)
    }

    fn previous_cell_in_same_table(&self, cell_id: usize) -> Option<usize> {
        let current = self.cells.get(cell_id)?;
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.table_id == current.table_id && cell.order < current.order)
            .max_by_key(|(_, cell)| cell.order)
            .map(|(id, _)| id)
    }

    fn cell_focus_boundary_forward(&self, focus_cell: usize) -> Option<TextPosition> {
        self.next_cell_in_same_table(focus_cell)
            .and_then(|after| self.cell_start_position(after))
            .or_else(|| self.cell_end_position(focus_cell))
    }

    fn navigate_cell_selection_forward(
        &mut self,
        anchor: TextPosition,
        focus: TextPosition,
    ) -> Option<SelectionRange> {
        if let Some((anchor_cell, focus_cell)) = self
            .cell_selection
            .as_ref()
            .map(|state| (state.anchor_cell, state.focus_cell))
        {
            if self.cells.get(focus_cell)?.order < self.cells.get(anchor_cell)?.order {
                return None;
            }
            if let Some(next_cell) = self.next_cell_in_same_table(focus_cell) {
                if let Some(state_mut) = self.cell_selection.as_mut() {
                    state_mut.focus_cell = next_cell;
                }
                let anchor_pos = self.cell_start_position(anchor_cell).unwrap_or(anchor);
                let focus_pos = self.cell_focus_boundary_forward(next_cell).unwrap_or(focus);
                return Some(SelectionRange::new(anchor_pos, focus_pos));
            } else {
                let anchor_pos = self.cell_start_position(anchor_cell).unwrap_or(anchor);
                let focus_pos = self.cell_end_position(focus_cell).unwrap_or(focus);
                return Some(SelectionRange::new(anchor_pos, focus_pos));
            }
        }

        let current_cell = self.cell_id_for_paragraph(focus.paragraph)?;
        if !self.is_at_cell_end(focus, current_cell) {
            return None;
        }
        let anchor_cell = self
            .cell_id_for_paragraph(anchor.paragraph)
            .unwrap_or(current_cell);
        self.start_cell_selection(anchor_cell, current_cell);
        let anchor_pos = self.cell_start_position(anchor_cell).unwrap_or(anchor);
        let focus_pos = self
            .cell_focus_boundary_forward(current_cell)
            .unwrap_or(focus);
        Some(SelectionRange::new(anchor_pos, focus_pos))
    }

    fn navigate_cell_selection_backward(
        &mut self,
        anchor: TextPosition,
        focus: TextPosition,
    ) -> Option<SelectionRange> {
        if let Some((anchor_cell, focus_cell)) = self
            .cell_selection
            .as_ref()
            .map(|state| (state.anchor_cell, state.focus_cell))
        {
            let focus_order = self.cells.get(focus_cell)?.order;
            let anchor_order = self.cells.get(anchor_cell)?.order;
            if focus_order >= anchor_order {
                // Forward selection shrinking backwards
                if focus_order == anchor_order {
                    self.clear_cell_selection();
                    return Some(SelectionRange::new(
                        self.cell_start_position(anchor_cell).unwrap_or(anchor),
                        self.cell_start_position(anchor_cell).unwrap_or(focus),
                    ));
                }
                let previous = self.previous_cell_in_same_table(focus_cell)?;
                if let Some(state_mut) = self.cell_selection.as_mut() {
                    state_mut.focus_cell = previous;
                }
                let anchor_pos = self.cell_start_position(anchor_cell).unwrap_or(anchor);
                let focus_pos = self.cell_focus_boundary_forward(previous).unwrap_or(focus);
                return Some(SelectionRange::new(anchor_pos, focus_pos));
            } else {
                // Backward selection extending further left
                let previous = self.previous_cell_in_same_table(focus_cell)?;
                if let Some(state_mut) = self.cell_selection.as_mut() {
                    state_mut.focus_cell = previous;
                }
                let anchor_pos = self.cell_end_position(anchor_cell).unwrap_or(anchor);
                let focus_pos = self.cell_start_position(previous).unwrap_or(focus);
                return Some(SelectionRange::new(anchor_pos, focus_pos));
            }
        }

        let current_cell = self.cell_id_for_paragraph(focus.paragraph)?;
        if !self.is_at_cell_start(focus, current_cell) {
            return None;
        }
        let anchor_cell = self
            .cell_id_for_paragraph(anchor.paragraph)
            .unwrap_or(current_cell);
        self.start_cell_selection(anchor_cell, current_cell);
        let anchor_pos = self.cell_end_position(anchor_cell).unwrap_or(anchor);
        let focus_pos = self.cell_start_position(current_cell).unwrap_or(focus);
        Some(SelectionRange::new(anchor_pos, focus_pos))
    }

    pub fn record_line(&mut self, index: usize, line: LineLayout) {
        if self.frozen {
            return;
        }
        let paragraph = self.ensure_paragraph_internal(index);
        // Lines changed: cached paragraph bounds are no longer valid.
        paragraph.invalidate_bounding_box();
        paragraph.lines.push(line);
    }

    pub fn record_line_with_caret_start(
        &mut self,
        index: usize,
        line: LineLayout,
        caret_start_x: f64,
    ) {
        if self.frozen {
            return;
        }
        let line_index = self
            .paragraphs
            .get(index)
            .map(|paragraph| paragraph.lines.len())
            .unwrap_or(0);
        self.caret_x_overrides
            .insert((index, line_index), caret_start_x);
        self.record_line(index, line);
    }

    pub fn append_lines_with_y_offset(&mut self, index: usize, lines: &[LineLayout], offset: f64) {
        if self.frozen || lines.is_empty() {
            return;
        }
        let paragraph = self.ensure_paragraph_internal(index);
        paragraph.invalidate_bounding_box();
        paragraph.lines.reserve(lines.len());

        if offset.abs() < f64::EPSILON {
            paragraph.lines.extend_from_slice(lines);
            return;
        }

        for line in lines {
            // Clone-then-mutate so any non-positional field (e.g. `vertical`)
            // is carried over automatically.
            let mut shifted = line.clone();
            shifted.top += offset;
            shifted.absolute_top += offset;
            shifted.baseline += offset;
            shifted.content_top += offset;
            shifted.absolute_content_top += offset;
            paragraph.lines.push(shifted);
        }
    }

    pub fn append_paragraph_text_and_lines_with_y_offset(
        &mut self,
        index: usize,
        text: &str,
        lines: &[LineLayout],
        offset: f64,
    ) {
        if self.frozen {
            return;
        }

        let paragraph = self.ensure_paragraph_internal(index);
        if !text.is_empty() {
            paragraph.text.push_str(text);
        }
        if lines.is_empty() {
            return;
        }

        paragraph.invalidate_bounding_box();
        paragraph.lines.reserve(lines.len());

        if offset.abs() < f64::EPSILON {
            paragraph.lines.extend_from_slice(lines);
            return;
        }

        for line in lines {
            // Clone-then-mutate so any non-positional field (e.g. `vertical`)
            // is carried over automatically.
            let mut shifted = line.clone();
            shifted.top += offset;
            shifted.absolute_top += offset;
            shifted.baseline += offset;
            shifted.content_top += offset;
            shifted.absolute_content_top += offset;
            paragraph.lines.push(shifted);
        }
    }

    pub fn record_empty_line(
        &mut self,
        index: usize,
        top: f64,
        absolute_top: f64,
        height: f64,
        baseline: f64,
        start_x: f64,
    ) {
        let line = LineLayout {
            column_index: 0,
            top,
            absolute_top,
            height,
            baseline,
            start_x,
            cursor_end_x: start_x,
            allowed_trailing_overflow_width: 0.0,
            content_top: top,
            absolute_content_top: absolute_top,
            content_height: height,
            fragments: Rc::from([]),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };
        self.record_line(index, line);
    }

    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.selection
    }

    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> bool {
        let clamped = range.map(|r| self.clamp_range(r));
        if self.selection == clamped {
            return false;
        }
        self.selection = clamped;
        if let Some(range) = self.selection {
            if let Some(state) = self.cell_selection.as_ref() {
                if !self.is_cell_fully_selected(state.anchor_cell, &range)
                    || !self.is_cell_fully_selected(state.focus_cell, &range)
                {
                    self.cell_selection = None;
                }
            }
        } else {
            self.cell_selection = None;
        }
        true
    }

    pub fn clear_selection(&mut self) -> bool {
        if self.selection.is_some() {
            self.selection = None;
            self.cell_selection = None;
            true
        } else {
            false
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        let range = self.selection?;
        if range.is_collapsed() {
            return None;
        }
        let (start, end) = range.normalized();
        if self.paragraphs.is_empty() {
            return None;
        }

        let mut result = String::new();
        let mut paragraph_index = start.paragraph;
        while paragraph_index <= end.paragraph {
            let paragraph = match self.paragraphs.get(paragraph_index) {
                Some(p) => p,
                None => break,
            };
            let text_len = paragraph.text.len();
            let from = if paragraph_index == start.paragraph {
                start.offset.min(text_len)
            } else {
                0
            };
            let mut to = if paragraph_index == end.paragraph {
                end.offset.min(text_len)
            } else {
                text_len
            };
            if to < from {
                to = from;
            }
            if to > from {
                if !result.is_empty() {
                    result.push('\n');
                }
                // Defensive: offsets are expected to be byte indices aligned to UTF-8 boundaries,
                // but some documents/paths can produce non-boundary indices. Avoid panicking.
                let from = from.min(paragraph.text.len());
                let mut to = to.min(paragraph.text.len());
                while to > from && !paragraph.text.is_char_boundary(to) {
                    to = to.saturating_sub(1);
                }
                let mut from_aligned = from;
                while from_aligned > 0 && !paragraph.text.is_char_boundary(from_aligned) {
                    from_aligned = from_aligned.saturating_sub(1);
                }
                if let Some(slice) = paragraph.text.get(from_aligned..to) {
                    result.push_str(slice);
                }
            } else if paragraph_index > start.paragraph
                && paragraph_index <= end.paragraph
                && !result.is_empty()
            {
                result.push('\n');
            }
            if paragraph_index == end.paragraph {
                break;
            }
            paragraph_index += 1;
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    pub fn paragraph_debug_texts(&self) -> Vec<String> {
        self.paragraphs
            .iter()
            .map(|paragraph| paragraph.text.clone())
            .collect()
    }

    /// Finds candidate paragraph indices for a given Y coordinate using bounding boxes.
    ///
    /// Returns a small set of paragraph indices that could potentially contain the Y coordinate.
    /// Uses binary search on paragraph bounding boxes for O(log n) performance instead of O(n).
    fn find_candidate_paragraphs_for_y(&self, y: f64) -> Vec<usize> {
        if self.paragraphs.is_empty() {
            return Vec::new();
        }

        // For small documents, just return all indices (binary search overhead not worth it)
        if self.paragraphs.len() <= 10 {
            return (0..self.paragraphs.len()).collect();
        }

        if self.paragraph_bounds_index.borrow().is_none() {
            #[cfg(test)]
            self.paragraph_bounds_build_count
                .set(self.paragraph_bounds_build_count.get() + 1);

            let mut bounds = Vec::with_capacity(self.paragraphs.len());
            for (idx, paragraph) in self.paragraphs.iter().enumerate() {
                if let Some((min_y, max_y, _, _)) = paragraph.bounding_box() {
                    bounds.push((idx, min_y, max_y));
                }
            }
            if !bounds.is_sorted_by(|a, b| a.1 <= b.1) {
                bounds.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
            }
            *self.paragraph_bounds_index.borrow_mut() = Some(bounds);
        }
        let bounds_index = self.paragraph_bounds_index.borrow();
        let bounds = bounds_index.as_deref().unwrap_or_default();
        if bounds.is_empty() {
            return (0..self.paragraphs.len()).collect();
        }

        // `max_y` is not monotonic when columns or merged table cells overlap.
        // Search the monotonic `min_y` key and inspect all preceding bounds that
        // may still contain the point.
        let start_idx = bounds.partition_point(|&(_, min_y, _)| min_y <= y);

        // Collect candidates: paragraphs that contain y or are closest to it.
        //
        // IMPORTANT: paragraphs whose Y bounds *contain* the click are always
        // included, regardless of how many we've already collected. The
        // MAX_CANDIDATES cap only governs the "near but outside" pool — these
        // are speculative candidates included so a click in a small vertical
        // gap can still snap to the nearest line. Capping `contain` candidates
        // breaks tables where many cells share the same Y range: with 8 cells
        // in a row, only the first ~5 would survive the cap and clicks on the
        // remaining cells would collapse to the closest *included* cell. See
        // CLAUDE.md changelog and `hit_test_resolves_each_cell_in_wide_table_row`.
        let mut candidates = Vec::new();
        let mut near_count: usize = 0;
        const MAX_NEAR_CANDIDATES: usize = 5;
        const Y_TOLERANCE: f64 = 50.0; // Include paragraphs within 50px

        // Look backwards for paragraphs that might contain y
        if start_idx > 0 {
            for &(idx, min_y, max_y) in bounds[..start_idx].iter().rev() {
                let contains = min_y <= y && y <= max_y;
                let near = !contains && (y - max_y).abs() < Y_TOLERANCE;
                if contains {
                    candidates.push(idx);
                } else if near {
                    candidates.push(idx);
                    near_count += 1;
                }
            }
        }

        // Look forwards for paragraphs that contain y or are closest below
        for &(idx, min_y, max_y) in bounds.iter().skip(start_idx) {
            let contains = min_y <= y && y <= max_y;
            let near =
                !contains && ((min_y - y).abs() < Y_TOLERANCE || (y - max_y).abs() < Y_TOLERANCE);

            if contains {
                candidates.push(idx);
            } else if near {
                if near_count >= MAX_NEAR_CANDIDATES {
                    // Stop looking for more *near* candidates, but keep
                    // scanning in case later entries actually contain y
                    // (rare but possible for overlapping lines on the same
                    // page — break only when we're well past the tolerance).
                    if min_y > y + Y_TOLERANCE {
                        break;
                    }
                    continue;
                }
                candidates.push(idx);
                near_count += 1;
            } else if min_y > y + Y_TOLERANCE {
                // Past the tolerance zone, no need to check more
                break;
            }
        }

        // If no candidates found (edge case), fall back to closest paragraph
        if candidates.is_empty() {
            // Find the paragraph with the closest Y range
            let closest = bounds
                .iter()
                .min_by(|a, b| {
                    let dist_a = if y < a.1 {
                        a.1 - y
                    } else if y > a.2 {
                        y - a.2
                    } else {
                        0.0
                    };
                    let dist_b = if y < b.1 {
                        b.1 - y
                    } else if y > b.2 {
                        y - b.2
                    } else {
                        0.0
                    };
                    dist_a
                        .partial_cmp(&dist_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|&(idx, _, _)| idx);

            if let Some(idx) = closest {
                candidates.push(idx);
            }
        }

        // Sort candidates by index for predictable iteration order
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    #[cfg(test)]
    fn paragraph_bounds_build_count(&self) -> usize {
        self.paragraph_bounds_build_count.get()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> Option<TextPosition> {
        self.hit_test_matching(x, y, |_| true)
    }

    /// Resolves a text position while restricting candidates to paragraphs
    /// accepted by `include`. The layout manager remains the source of truth
    /// for line geometry; callers only provide document-surface eligibility.
    pub fn hit_test_matching<F>(&self, x: f64, y: f64, mut include: F) -> Option<TextPosition>
    where
        F: FnMut(usize) -> bool,
    {
        if self.paragraphs.is_empty() {
            return None;
        }

        fn horizontal_distance(line: &LineLayout, x: f64) -> f64 {
            let min_x = line.start_x;
            let max_x = line.interactive_max_x();
            if x < min_x {
                min_x - x
            } else if x > max_x {
                x - max_x
            } else {
                0.0
            }
        }

        fn center_distance(line: &LineLayout, x: f64) -> f64 {
            let min_x = line.start_x;
            let max_x = line.interactive_max_x();
            let mid = min_x + (max_x - min_x) / 2.0;
            (x - mid).abs()
        }

        const DIST_EPSILON: f64 = 1e-6;

        // Optimization: Use paragraph bounds to narrow candidates.
        // If candidate bounds appear inconsistent, fall back to scanning all paragraphs
        // to prioritize hit-test correctness.
        let mut candidate_indices = self.find_candidate_paragraphs_for_y(y);
        let has_covering_candidate = candidate_indices.iter().any(|&idx| {
            include(idx)
                && self.is_paragraph_selectable(idx)
                && self
                    .paragraphs
                    .get(idx)
                    .and_then(|p| p.bounding_box())
                    .map(|(min_y, max_y, _, _)| y >= min_y && y <= max_y)
                    .unwrap_or(false)
        });
        if !has_covering_candidate {
            candidate_indices = (0..self.paragraphs.len()).collect();
        }

        let mut best: Option<(usize, usize, f64, f64, f64, f64)> = None;

        for &p_idx in &candidate_indices {
            if !include(p_idx) || !self.is_paragraph_selectable(p_idx) {
                continue;
            }
            let Some(paragraph) = self.paragraphs.get(p_idx) else {
                continue;
            };
            for (line_idx, line) in paragraph.lines.iter().enumerate() {
                let top = line.absolute_top;
                let bottom = top + line.height.max(1.0);
                let distance = if y < top {
                    top - y
                } else if y > bottom {
                    y - bottom
                } else {
                    0.0
                };

                let horizontal = horizontal_distance(line, x);
                let center = center_distance(line, x);
                let line_top = line.absolute_top;
                let update = match best {
                    Some((
                        best_paragraph,
                        _,
                        best_distance,
                        best_horizontal,
                        best_center,
                        best_top,
                    )) => {
                        if distance + DIST_EPSILON < best_distance {
                            true
                        } else if (distance - best_distance).abs() <= DIST_EPSILON {
                            if horizontal + DIST_EPSILON < best_horizontal {
                                true
                            } else if (horizontal - best_horizontal).abs() <= DIST_EPSILON {
                                if center + DIST_EPSILON < best_center {
                                    true
                                } else if (center - best_center).abs() <= DIST_EPSILON {
                                    if line_top > best_top + DIST_EPSILON {
                                        true
                                    } else if (line_top - best_top).abs() <= DIST_EPSILON {
                                        p_idx > best_paragraph
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    None => true,
                };

                if update {
                    best = Some((p_idx, line_idx, distance, horizontal, center, line_top));
                }
            }
        }

        let (paragraph_index, line_index, _, _, _, _) = best?;
        let paragraph = self.paragraphs.get(paragraph_index)?;
        let line = paragraph.lines.get(line_index)?;

        if line.vertical {
            if let Some((start, end)) = line.break_range.or_else(|| line.logical_source_range()) {
                let mid = line.absolute_top + line.height.max(1.0) / 2.0;
                let before = if line.vertical_reversed {
                    y >= mid
                } else {
                    y <= mid
                };
                return Some(TextPosition::new(
                    paragraph_index,
                    if before { start } else { end },
                ));
            }
            return Some(TextPosition::new(paragraph_index, 0));
        }

        if line.fragments.is_empty() {
            for tab in line.tab_stops.iter() {
                let tab_start = tab.start_x.min(tab.end_x);
                let tab_end = tab.end_x.max(tab.start_x);
                if x < tab_start {
                    break;
                }
                if x <= tab_end {
                    let mid = tab_start + (tab_end - tab_start) / 2.0;
                    let offset = if x <= mid {
                        tab.start_offset
                    } else {
                        tab.end_offset
                    };
                    return Some(TextPosition::new(paragraph_index, offset));
                }
            }

            if let Some((start, end)) = line.break_range {
                let min_x = line.start_x;
                let width = line.cursor_width().max(SOFT_BREAK_MIN_HIT_WIDTH);
                let max_x = min_x + width;
                let mid = min_x + width / 2.0;
                let clamped_x = x.clamp(min_x, max_x);
                let offset = if clamped_x <= mid { start } else { end };
                return Some(TextPosition::new(paragraph_index, offset));
            }
            return Some(TextPosition::new(paragraph_index, 0));
        }

        let mut last_position =
            TextPosition::new(paragraph_index, line.fragments.last().unwrap().end);

        for tab in line.tab_stops.iter() {
            let tab_start = tab.start_x.min(tab.end_x);
            let tab_end = tab.end_x.max(tab.start_x);
            if x < tab_start {
                break;
            }
            if x <= tab_end {
                let mid = tab_start + (tab_end - tab_start) / 2.0;
                let offset = if x <= mid {
                    tab.start_offset
                } else {
                    tab.end_offset
                };
                return Some(TextPosition::new(paragraph_index, offset));
            }
        }

        for fragment in line.fragments.iter() {
            let frag_start_x = fragment.x;
            let frag_end_x = fragment.x + fragment.width;

            if x < frag_start_x {
                return Some(TextPosition::new(paragraph_index, fragment.start));
            }

            if x <= frag_end_x {
                return Some(TextPosition::new(
                    paragraph_index,
                    self.resolve_position_in_fragment(fragment, x),
                ));
            }

            last_position = TextPosition::new(paragraph_index, fragment.end);
        }

        if let Some((start, end)) = line.break_range {
            // Word places a click after the visible text before its page break.
            // The end offset belongs to the next page, whose placeholder line
            // can be hit independently. Do not move input to an invisible page.
            let offset = if paragraph.text.get(start..end) == Some(PAGE_BREAK_STR) {
                start
            } else {
                end
            };
            return Some(TextPosition::new(paragraph_index, offset));
        }

        Some(last_position)
    }

    /// Resolves the paragraph that should own a dragged floating-object anchor.
    ///
    /// Word advances the anchor as the object's top edge crosses later visual
    /// lines. This differs subtly from caret hit-testing: when two paragraph
    /// line boxes overlap vertically, the line with the later top edge wins
    /// before horizontal proximity is considered. Horizontal distance still
    /// disambiguates cells or columns whose lines start at the same Y.
    pub fn hit_test_anchor_paragraph_matching<F>(
        &self,
        x: f64,
        y: f64,
        mut include: F,
    ) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
    {
        if self.paragraphs.is_empty() {
            return None;
        }

        fn horizontal_distance(line: &LineLayout, x: f64) -> f64 {
            let min_x = line.start_x;
            let max_x = line.interactive_max_x();
            if x < min_x {
                min_x - x
            } else if x > max_x {
                x - max_x
            } else {
                0.0
            }
        }

        const DIST_EPSILON: f64 = 1e-6;
        let mut best: Option<(usize, f64, f64, f64)> = None;
        // Floating-anchor previews are far less frequent than caret hit-tests,
        // and page-break paragraphs can leave broad/stale bounds in the lookup
        // index while pagination is being previewed. Scan the canonical line
        // layouts so an eligible destination paragraph cannot be dropped by a
        // cached candidate set.
        for paragraph_index in 0..self.paragraphs.len() {
            if !include(paragraph_index) || !self.is_paragraph_selectable(paragraph_index) {
                continue;
            }
            let Some(paragraph) = self.paragraphs.get(paragraph_index) else {
                continue;
            };
            for line in &paragraph.lines {
                let top = line.absolute_top;
                let bottom = top + line.height.max(1.0);
                let vertical = if y < top {
                    top - y
                } else if y > bottom {
                    y - bottom
                } else {
                    0.0
                };
                let horizontal = horizontal_distance(line, x);
                let update = match best {
                    Some((best_paragraph, best_vertical, best_top, best_horizontal)) => {
                        vertical + DIST_EPSILON < best_vertical
                            || ((vertical - best_vertical).abs() <= DIST_EPSILON
                                && (top > best_top + DIST_EPSILON
                                    || ((top - best_top).abs() <= DIST_EPSILON
                                        && (horizontal + DIST_EPSILON < best_horizontal
                                            || ((horizontal - best_horizontal).abs()
                                                <= DIST_EPSILON
                                                && paragraph_index > best_paragraph)))))
                    }
                    None => true,
                };
                if update {
                    best = Some((paragraph_index, vertical, top, horizontal));
                }
            }
        }

        best.map(|(paragraph_index, _, _, _)| paragraph_index)
    }

    fn resolve_position_in_fragment(&self, fragment: &FragmentLayout, x: f64) -> usize {
        if fragment.glyphs.is_empty() {
            return fragment.start;
        }

        let relative_x = (x - fragment.x).max(0.0);
        let mut last = fragment.start;

        for glyph in fragment.glyphs.iter() {
            let glyph_start = glyph.x;
            let glyph_end = glyph.x + glyph.width;
            let mid = glyph_start + (glyph.width / 2.0);

            if relative_x < glyph_start {
                return last;
            }

            if relative_x <= glyph_end {
                return if relative_x < mid {
                    glyph.start
                } else {
                    glyph.end
                };
            }

            last = glyph.end;
        }

        last
    }

    pub fn move_left(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }

        let Some(mut paragraph_index) = self.nearest_selectable_paragraph(position.paragraph)
        else {
            return TextPosition::default();
        };

        if position.offset > 0 {
            let paragraph = &self.paragraphs[paragraph_index];
            let offset = position.offset.min(paragraph.text.len());
            let new_offset = previous_grapheme_boundary(&paragraph.text, offset);
            return TextPosition::new(paragraph_index, new_offset);
        }

        while paragraph_index > 0 {
            paragraph_index -= 1;
            if !self.is_paragraph_selectable(paragraph_index) {
                continue;
            }
            let text_len = self.paragraphs[paragraph_index].text.len();
            return TextPosition::new(paragraph_index, text_len);
        }

        TextPosition::new(0, 0)
    }

    pub fn move_right(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }

        let Some(mut paragraph_index) = self.nearest_selectable_paragraph(position.paragraph)
        else {
            return TextPosition::default();
        };
        let mut offset = position.offset;

        loop {
            let paragraph = &self.paragraphs[paragraph_index];
            let text_len = paragraph.text.len();
            if offset < text_len {
                let new_offset = next_grapheme_boundary(&paragraph.text, offset);
                return TextPosition::new(paragraph_index, new_offset);
            }

            if paragraph_index + 1 >= self.paragraphs.len() {
                return TextPosition::new(paragraph_index, text_len);
            }

            paragraph_index += 1;
            if !self.is_paragraph_selectable(paragraph_index) {
                offset = 0;
                continue;
            }
            return TextPosition::new(paragraph_index, 0);
        }
    }

    /// Returns a position pointing at byte offset 0 of the paragraph that
    /// `position` resolves to. Empty document collapses to the default
    /// position.
    pub fn move_paragraph_start(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        TextPosition::new(clamped.paragraph, 0)
    }

    /// Returns a position pointing at the end-of-text byte offset of the
    /// paragraph that `position` resolves to.
    pub fn move_paragraph_end(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        let text_len = self
            .paragraphs
            .get(clamped.paragraph)
            .map(|p| p.text.len())
            .unwrap_or(0);
        TextPosition::new(clamped.paragraph, text_len)
    }

    /// Word for Mac's Cmd+Up target: the current paragraph's start, or the
    /// previous paragraph's start from offset zero.
    pub fn move_previous_paragraph(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        if clamped.offset > 0 {
            return self.move_paragraph_start(clamped);
        }
        self.move_paragraph_start(self.move_left(clamped))
    }

    /// Word for Mac's Cmd+Down target: the next paragraph's start, or the end
    /// of the last paragraph.
    pub fn move_next_paragraph(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        self.move_right(self.move_paragraph_end(position))
    }

    /// Word's Ctrl+Left: the start of the word before `position`, or the end
    /// of the previous paragraph from a paragraph start.
    pub fn move_word_left(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        if clamped.offset == 0 {
            return self.move_left(clamped);
        }
        let text = &self.paragraphs[clamped.paragraph].text;
        TextPosition::new(
            clamped.paragraph,
            previous_word_boundary(text, clamped.offset),
        )
    }

    /// Word's Ctrl+Right: the start of the word after `position` (skipping
    /// the spaces behind the current word), or the next paragraph's start
    /// from a paragraph end.
    pub fn move_word_right(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        let text = &self.paragraphs[clamped.paragraph].text;
        if clamped.offset >= text.len() {
            return self.move_right(clamped);
        }
        TextPosition::new(clamped.paragraph, next_word_boundary(text, clamped.offset))
    }

    /// The word under `position` for a double-click: Word selects the word
    /// plus the spaces that follow it; on whitespace it selects that run of
    /// spaces. Returns the paragraph's byte range.
    pub fn word_range_at(&self, position: TextPosition) -> Option<(usize, usize)> {
        if self.paragraphs.is_empty() {
            return None;
        }
        let clamped = self.clamp_position(position);
        let text = &self.paragraphs[clamped.paragraph].text;
        if text.is_empty() {
            return None;
        }
        Some(word_range_in(text, clamped.offset))
    }

    /// The first offset of the line `position` sits on. Without layout the
    /// paragraph start is the best answer.
    pub fn move_line_start(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        let Some(line_index) = self.find_line_index_for_position(clamped.paragraph, clamped.offset)
        else {
            return TextPosition::new(clamped.paragraph, 0);
        };
        let start = self
            .paragraphs
            .get(clamped.paragraph)
            .and_then(|paragraph| paragraph.lines.get(line_index))
            .and_then(|line| line.logical_source_range().map(|range| range.0))
            .unwrap_or(0);
        TextPosition::new(clamped.paragraph, start)
    }

    /// The offset after the last character of the line `position` sits on.
    /// On a wrapped line that is the wrap point itself, as in Word; the last
    /// line ends at the paragraph's end of text.
    pub fn move_line_end(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let clamped = self.clamp_position(position);
        let paragraph_end = self.move_paragraph_end(clamped);
        let Some(paragraph) = self.paragraphs.get(clamped.paragraph) else {
            return paragraph_end;
        };
        let Some(line_index) = self.find_line_index_for_position(clamped.paragraph, clamped.offset)
        else {
            return paragraph_end;
        };
        if line_index + 1 >= paragraph.lines.len() {
            return paragraph_end;
        }
        let end = paragraph
            .lines
            .get(line_index)
            .and_then(|line| line.logical_source_range().map(|range| range.1))
            .unwrap_or(paragraph_end.offset)
            .min(paragraph_end.offset);
        TextPosition::new(clamped.paragraph, end)
    }

    /// The end of the last paragraph.
    pub fn move_document_end(&self) -> TextPosition {
        let last = self.paragraphs.len().saturating_sub(1);
        self.move_paragraph_end(TextPosition::new(last, usize::MAX))
    }

    pub fn move_up(&self, position: TextPosition, preferred_x: Option<f64>) -> (TextPosition, f64) {
        self.move_vertical(position, preferred_x, VerticalDirection::Up)
    }

    pub fn move_down(
        &self,
        position: TextPosition,
        preferred_x: Option<f64>,
    ) -> (TextPosition, f64) {
        self.move_vertical(position, preferred_x, VerticalDirection::Down)
    }

    pub fn caret_x_for_position(&self, position: TextPosition) -> Option<f64> {
        if self.paragraphs.is_empty() {
            return None;
        }
        let clamped = self.clamp_position(position);
        let line_index = self.find_line_index_for_position(clamped.paragraph, clamped.offset)?;
        self.caret_x_on_line(clamped.paragraph, line_index, clamped.offset)
    }

    /// Computes the next selection range for caret navigation.
    ///
    /// 水平方向で `extend = true` の場合は anchor を固定したまま、focus を 1
    /// グラフェム境界分だけ移動させる（`move_left` / `move_right`）ことで、
    /// Shift+矢印操作が 1 文字ずつ拡張されるようにしている。
    pub fn navigate(
        &mut self,
        current_range: Option<SelectionRange>,
        movement: CaretMovement,
        extend: bool,
        preferred_x: Option<f64>,
    ) -> (SelectionRange, Option<f64>) {
        let fallback = self.clamp_position(TextPosition::default());
        let current_focus = current_range.map(|range| range.focus).unwrap_or(fallback);

        let anchor = if extend {
            current_range
                .map(|range| range.anchor)
                .unwrap_or(current_focus)
        } else {
            current_focus
        };

        if extend {
            match movement {
                CaretMovement::Right => {
                    if let Some(range) = self.navigate_cell_selection_forward(anchor, current_focus)
                    {
                        return (range, None);
                    }
                }
                CaretMovement::Left => {
                    if let Some(range) =
                        self.navigate_cell_selection_backward(anchor, current_focus)
                    {
                        return (range, None);
                    }
                }
                _ => {
                    self.clear_cell_selection();
                }
            }
        } else {
            self.clear_cell_selection();
        }

        if !extend {
            if let Some(range) = current_range.filter(|range| !range.is_collapsed()) {
                let (start, end) = range.normalized();
                let edge = match movement {
                    CaretMovement::Left => Some(start),
                    CaretMovement::Right => Some(end),
                    _ => None,
                };
                if let Some(edge) = edge {
                    return (SelectionRange::collapsed(self.clamp_position(edge)), None);
                }
            }
        }

        let (new_focus, next_goal) = match movement {
            CaretMovement::Left => (self.move_left(current_focus), None),
            CaretMovement::Right => (self.move_right(current_focus), None),
            CaretMovement::Up | CaretMovement::Down => {
                let desired = preferred_x
                    .or_else(|| self.caret_x_for_position(current_focus))
                    .unwrap_or(0.0);
                let (position, _) = match movement {
                    CaretMovement::Up => self.move_up(current_focus, Some(desired)),
                    CaretMovement::Down => self.move_down(current_focus, Some(desired)),
                    _ => unreachable!(),
                };
                (position, Some(desired))
            }
            CaretMovement::ParagraphStart => (self.move_paragraph_start(current_focus), None),
            CaretMovement::ParagraphEnd => (self.move_paragraph_end(current_focus), None),
            CaretMovement::ParagraphPrevious => (self.move_previous_paragraph(current_focus), None),
            CaretMovement::ParagraphNext => (self.move_next_paragraph(current_focus), None),
            CaretMovement::WordLeft => (self.move_word_left(current_focus), None),
            CaretMovement::WordRight => (self.move_word_right(current_focus), None),
            CaretMovement::LineStart => (self.move_line_start(current_focus), None),
            CaretMovement::LineEnd => (self.move_line_end(current_focus), None),
            CaretMovement::DocumentStart => (TextPosition::default(), None),
            CaretMovement::DocumentEnd => (self.move_document_end(), None),
        };

        let selection = if extend {
            SelectionRange::new(anchor, new_focus)
        } else {
            SelectionRange::collapsed(new_focus)
        };

        (selection, next_goal)
    }

    fn move_vertical(
        &self,
        position: TextPosition,
        preferred_x: Option<f64>,
        direction: VerticalDirection,
    ) -> (TextPosition, f64) {
        if self.paragraphs.is_empty() {
            return (TextPosition::default(), preferred_x.unwrap_or(0.0));
        }

        let clamped = self.clamp_position(position);
        if self.paragraphs.get(clamped.paragraph).is_none() {
            return (clamped, preferred_x.unwrap_or(0.0));
        }

        let current_line_idx = self.find_line_index_for_position(clamped.paragraph, clamped.offset);

        let desired_x = preferred_x.unwrap_or_else(|| {
            current_line_idx
                .and_then(|line_index| {
                    self.caret_x_on_line(clamped.paragraph, line_index, clamped.offset)
                })
                .unwrap_or(0.0)
        });

        let document_target_line = match direction {
            VerticalDirection::Up => self.previous_line(clamped.paragraph, current_line_idx),
            VerticalDirection::Down => self.next_line(clamped.paragraph, current_line_idx),
        };

        let target_line = self
            .vertical_line_in_adjacent_table_cell(
                clamped.paragraph,
                document_target_line,
                desired_x,
                direction,
            )
            .or(document_target_line);

        let Some((target_paragraph_index, target_line_index)) = target_line else {
            let fallback_x = current_line_idx
                .and_then(|line_index| {
                    self.caret_x_on_line(clamped.paragraph, line_index, clamped.offset)
                })
                .unwrap_or(desired_x);
            return (clamped, fallback_x);
        };

        let target_paragraph = match self.paragraphs.get(target_paragraph_index) {
            Some(paragraph) => paragraph,
            None => return (clamped, desired_x),
        };

        if target_paragraph.lines.is_empty() {
            return (clamped, desired_x);
        }

        let target_line = &target_paragraph.lines[target_line_index];
        let mut new_offset = self.offset_for_x_in_line(target_line, desired_x);
        let paragraph_len = target_paragraph.text.len();
        if new_offset > paragraph_len {
            new_offset = paragraph_len;
        }

        let new_position = TextPosition::new(target_paragraph_index, new_offset);
        if new_position == clamped {
            let actual_x = self
                .caret_x_on_line(
                    clamped.paragraph,
                    current_line_idx.unwrap_or(0),
                    clamped.offset,
                )
                .unwrap_or(desired_x);
            return (new_position, actual_x);
        }

        let actual_x = self
            .caret_x_on_line(target_paragraph_index, target_line_index, new_offset)
            .unwrap_or(desired_x);

        (new_position, actual_x)
    }

    fn vertical_line_in_adjacent_table_cell(
        &self,
        paragraph_index: usize,
        document_target_line: Option<(usize, usize)>,
        desired_x: f64,
        direction: VerticalDirection,
    ) -> Option<(usize, usize)> {
        let current_cell_id = self.cell_id_for_paragraph(paragraph_index)?;
        if document_target_line
            .and_then(|(target_paragraph, _)| self.cell_id_for_paragraph(target_paragraph))
            == Some(current_cell_id)
        {
            return None;
        }

        let current_cell = self.cells.get(current_cell_id)?;
        let target_row = self
            .cells
            .iter()
            .filter(|cell| {
                cell.table_id == current_cell.table_id
                    && match direction {
                        VerticalDirection::Up => cell.row < current_cell.row,
                        VerticalDirection::Down => cell.row > current_cell.row,
                    }
            })
            .map(|cell| cell.row)
            .reduce(|best, row| match direction {
                VerticalDirection::Up => best.max(row),
                VerticalDirection::Down => best.min(row),
            })?;

        let target_cell = self
            .cells
            .iter()
            .filter(|cell| cell.table_id == current_cell.table_id && cell.row == target_row)
            .min_by(|left, right| {
                let horizontal_distance = |cell: &CellLayout| {
                    if desired_x < cell.left {
                        cell.left - desired_x
                    } else if desired_x > cell.right {
                        desired_x - cell.right
                    } else {
                        0.0
                    }
                };
                horizontal_distance(left)
                    .total_cmp(&horizontal_distance(right))
                    .then_with(|| {
                        let left_center = (left.left + left.right) / 2.0;
                        let right_center = (right.left + right.right) / 2.0;
                        (desired_x - left_center)
                            .abs()
                            .total_cmp(&(desired_x - right_center).abs())
                    })
                    .then_with(|| left.order.cmp(&right.order))
            })?;

        target_cell
            .paragraphs
            .iter()
            .filter(|&&paragraph| self.is_paragraph_selectable(paragraph))
            .flat_map(|&paragraph| {
                self.paragraphs
                    .get(paragraph)
                    .into_iter()
                    .flat_map(move |layout| {
                        layout
                            .lines
                            .iter()
                            .enumerate()
                            .map(move |(line, layout)| (paragraph, line, layout.absolute_top))
                    })
            })
            .reduce(|best, candidate| match direction {
                VerticalDirection::Up => {
                    if candidate.2 > best.2 {
                        candidate
                    } else {
                        best
                    }
                }
                VerticalDirection::Down => {
                    if candidate.2 < best.2 {
                        candidate
                    } else {
                        best
                    }
                }
            })
            .map(|(paragraph, line, _)| (paragraph, line))
    }

    fn find_line_index_for_position(&self, paragraph_index: usize, offset: usize) -> Option<usize> {
        let paragraph = self.paragraphs.get(paragraph_index)?;
        if paragraph.lines.is_empty() {
            return None;
        }

        if let Some((idx, _)) = paragraph.lines.iter().enumerate().find(|(_, line)| {
            line.fragments.is_empty()
                && line
                    .break_range
                    .map(|(start, end)| offset >= start && offset < end)
                    .unwrap_or(false)
        }) {
            return Some(idx);
        }

        let mut chosen_idx = paragraph.lines.len().saturating_sub(1);
        for (idx, line) in paragraph.lines.iter().enumerate() {
            if let Some((start, end)) = line.break_range {
                if offset >= start && offset < end {
                    return Some(idx);
                }
            }

            if line.fragments.is_empty() {
                chosen_idx = idx;
                continue;
            }

            if line
                .logical_source_range()
                .is_some_and(|(_, end)| offset <= end)
            {
                return Some(idx);
            }

            chosen_idx = idx;
        }

        Some(chosen_idx)
    }

    fn caret_x_on_line(
        &self,
        paragraph_index: usize,
        line_index: usize,
        offset: usize,
    ) -> Option<f64> {
        let paragraph = self.paragraphs.get(paragraph_index)?;
        let line = paragraph.lines.get(line_index)?;
        Some(
            self.caret_x_overrides
                .get(&(paragraph_index, line_index))
                .copied()
                .unwrap_or_else(|| caret_x_for_line(line, offset)),
        )
    }

    fn offset_for_x_in_line(&self, line: &LineLayout, target_x: f64) -> usize {
        if line.fragments.is_empty() {
            for tab in line.tab_stops.iter() {
                let tab_start = tab.start_x.min(tab.end_x);
                let tab_end = tab.end_x.max(tab.start_x);
                if target_x < tab_start {
                    break;
                }
                if target_x <= tab_end {
                    let mid = tab_start + (tab_end - tab_start) / 2.0;
                    return if target_x <= mid {
                        tab.start_offset
                    } else {
                        tab.end_offset
                    };
                }
            }

            if let Some((start, end)) = line.break_range {
                let min_x = line.start_x;
                let width = line.cursor_width().max(SOFT_BREAK_MIN_HIT_WIDTH);
                let max_x = min_x + width;
                let clamped_x = target_x.clamp(min_x, max_x);
                let mid = min_x + width / 2.0;
                if clamped_x <= mid {
                    start
                } else {
                    end
                }
            } else {
                0
            }
        } else {
            for tab in line.tab_stops.iter() {
                let tab_start = tab.start_x.min(tab.end_x);
                let tab_end = tab.end_x.max(tab.start_x);
                if target_x < tab_start {
                    break;
                }
                if target_x <= tab_end {
                    let mid = tab_start + (tab_end - tab_start) / 2.0;
                    return if target_x <= mid {
                        tab.start_offset
                    } else {
                        tab.end_offset
                    };
                }
            }

            let mut last_offset = line
                .fragments
                .last()
                .map(|fragment| fragment.end)
                .unwrap_or(0);
            for fragment in line.fragments.iter() {
                let frag_start_x = fragment.x;
                let frag_end_x = fragment.x + fragment.width;

                if target_x <= frag_start_x {
                    return fragment.start;
                }

                if target_x <= frag_end_x {
                    return self.resolve_position_in_fragment(fragment, target_x);
                }

                last_offset = fragment.end;
            }

            if let Some((_, end)) = line.break_range {
                end
            } else {
                last_offset
            }
        }
    }

    fn previous_line(
        &self,
        paragraph_index: usize,
        current_line_index: Option<usize>,
    ) -> Option<(usize, usize)> {
        if let Some(line_index) = current_line_index {
            if line_index > 0 {
                return Some((paragraph_index, line_index - 1));
            }
        }

        let mut paragraph_idx = paragraph_index;
        while paragraph_idx > 0 {
            paragraph_idx -= 1;
            let paragraph = &self.paragraphs[paragraph_idx];
            if paragraph.lines.is_empty() {
                continue;
            }
            return Some((paragraph_idx, paragraph.lines.len() - 1));
        }

        None
    }

    fn next_line(
        &self,
        paragraph_index: usize,
        current_line_index: Option<usize>,
    ) -> Option<(usize, usize)> {
        if let Some(line_index) = current_line_index {
            if let Some(paragraph) = self.paragraphs.get(paragraph_index) {
                if line_index + 1 < paragraph.lines.len() {
                    return Some((paragraph_index, line_index + 1));
                }
            }
        }

        let mut paragraph_idx = paragraph_index + 1;
        while paragraph_idx < self.paragraphs.len() {
            let paragraph = &self.paragraphs[paragraph_idx];
            if paragraph.lines.is_empty() {
                paragraph_idx += 1;
                continue;
            }
            return Some((paragraph_idx, 0));
        }

        None
    }

    /// Sets which line a caret on a soft wrap is drawn on; see
    /// [`CaretAffinity`].
    pub fn set_caret_affinity(&mut self, affinity: CaretAffinity) {
        self.caret_affinity = affinity;
    }

    pub fn caret_affinity(&self) -> CaretAffinity {
        self.caret_affinity
    }

    pub fn caret_metrics(&self) -> Option<CaretMetrics> {
        let range = self.selection?;
        if !range.is_collapsed() {
            return None;
        }

        self.caret_metrics_at_position(range.focus)
    }

    /// Reads layout geometry without changing the active selection.
    pub fn caret_metrics_at_position(&self, position: TextPosition) -> Option<CaretMetrics> {
        let paragraph = self.paragraphs.get(position.paragraph)?;
        if paragraph.lines.is_empty() {
            return None;
        }

        let mut chosen_line_index = paragraph.lines.len().saturating_sub(1);

        if let Some((line_index, _)) = paragraph.lines.iter().enumerate().find(|(_, line)| {
            line.fragments.is_empty()
                && line
                    .break_range
                    .map(|(start, end)| position.offset >= start && position.offset < end)
                    .unwrap_or(false)
        }) {
            chosen_line_index = line_index;
        } else {
            for (line_index, line) in paragraph.lines.iter().enumerate() {
                if let Some((start, end)) = line.break_range {
                    if position.offset >= start && position.offset < end {
                        chosen_line_index = line_index;
                        if line.fragments.is_empty() {
                            break;
                        }
                    }
                }

                if line.fragments.is_empty() {
                    chosen_line_index = line_index;
                    continue;
                }

                if line
                    .logical_source_range()
                    .is_some_and(|(_, end)| position.offset <= end)
                {
                    chosen_line_index = line_index;
                    break;
                }

                chosen_line_index = line_index;
            }
        }
        // On a soft wrap — the offset that ends this line starts the next,
        // with no break character between — downstream means the next line.
        if self.caret_affinity == CaretAffinity::Downstream {
            let chosen = &paragraph.lines[chosen_line_index];
            let wraps_here = !chosen.fragments.is_empty()
                && chosen.break_range.is_none()
                && chosen
                    .logical_source_range()
                    .is_some_and(|(_, end)| end == position.offset);
            if wraps_here {
                if let Some(next) = paragraph.lines.get(chosen_line_index + 1) {
                    if !next.fragments.is_empty()
                        && next
                            .logical_source_range()
                            .is_some_and(|(start, _)| start == position.offset)
                    {
                        chosen_line_index += 1;
                    }
                }
            }
        }
        let chosen_line = &paragraph.lines[chosen_line_index];

        // Keep the caret on the source edge of each vertical grapheme,
        // including the reversed start/end edges of bottom-to-top cells.
        if chosen_line.vertical {
            let past_end = chosen_line
                .break_range
                .or_else(|| chosen_line.logical_source_range())
                .map(|(_, end)| position.offset >= end)
                .unwrap_or(false);
            let after_top = past_end != chosen_line.vertical_reversed;
            let top = if after_top {
                chosen_line.content_top + chosen_line.content_height
            } else {
                chosen_line.content_top
            };
            let absolute_top = if after_top {
                chosen_line.absolute_content_top + chosen_line.content_height
            } else {
                chosen_line.absolute_content_top
            };
            return Some(CaretMetrics {
                x: chosen_line.start_x,
                top,
                absolute_top,
                width: (chosen_line.cursor_end_x - chosen_line.start_x)
                    .abs()
                    .max(1.0),
                height: CARET_THICKNESS,
            });
        }

        Some(CaretMetrics {
            x: self
                .caret_x_overrides
                .get(&(position.paragraph, chosen_line_index))
                .copied()
                .unwrap_or_else(|| caret_x_for_line(chosen_line, position.offset)),
            top: chosen_line.content_top,
            absolute_top: chosen_line.absolute_content_top,
            width: CARET_THICKNESS,
            height: chosen_line.content_height.max(1.0),
        })
    }

    pub fn paragraph_text_len(&self, index: usize) -> usize {
        self.paragraphs
            .get(index)
            .map(|p| p.text.len())
            .unwrap_or(0)
    }

    fn clamp_range(&self, range: SelectionRange) -> SelectionRange {
        SelectionRange {
            anchor: self.clamp_position(range.anchor),
            focus: self.clamp_position(range.focus),
        }
    }

    pub fn clamp_position(&self, position: TextPosition) -> TextPosition {
        if self.paragraphs.is_empty() {
            return TextPosition::default();
        }
        let Some(paragraph_index) = self.nearest_selectable_paragraph(position.paragraph) else {
            return TextPosition::default();
        };
        let paragraph = &self.paragraphs[paragraph_index];
        let offset = position.offset.min(paragraph.text.len());
        TextPosition::new(paragraph_index, offset)
    }

    pub fn paragraphs(&self) -> &[ParagraphLayout] {
        &self.paragraphs
    }

    pub fn selected_lines(&self) -> Vec<SelectedLineDebug> {
        let Some(range) = self.selection else {
            return Vec::new();
        };

        if range.is_collapsed() {
            return Vec::new();
        }

        // When a rectangular cell-range selection is active the underlying
        // text range still runs linearly from anchor to focus, which would
        // cause the JS overlay to paint entire intermediate rows. Restrict
        // the emitted lines to paragraphs that belong to the rectangular
        // cell set so the overlay matches the rectangle.
        let cell_rect_filter: Option<HashSet<usize>> = self
            .selected_cell_ids_rect()
            .map(|ids| ids.into_iter().collect());

        let (start, end) = range.normalized();
        let mut lines = Vec::new();

        for (paragraph_index, paragraph) in self.paragraphs.iter().enumerate() {
            if paragraph_index < start.paragraph || paragraph_index > end.paragraph {
                continue;
            }
            if let Some(filter) = cell_rect_filter.as_ref() {
                match self.paragraph_to_cell.get(&paragraph_index) {
                    Some(cell_id) if filter.contains(cell_id) => {}
                    _ => continue,
                }
            }

            // Inside a rectangular cell-range selection the full content of
            // every included paragraph is considered selected, regardless of
            // where the caret was pressed / released within the anchor and
            // focus cells.
            let force_full = cell_rect_filter.is_some();
            let range_start = if !force_full && paragraph_index == start.paragraph {
                start.offset.min(paragraph.text.len())
            } else {
                0
            };
            let range_end = if !force_full && paragraph_index == end.paragraph {
                end.offset.min(paragraph.text.len())
            } else {
                paragraph.text.len()
            };

            for line in &paragraph.lines {
                if line.fragments.is_empty() {
                    let paragraph_len = paragraph.text.len();
                    if line.break_range.is_none() && range_end < paragraph_len {
                        continue;
                    }
                    let (start, end) = line.break_range.unwrap_or((paragraph_len, paragraph_len));
                    let overlaps = if start == end {
                        range_start <= start && range_end >= end
                    } else {
                        range_start < end && range_end > start
                    };
                    if !overlaps {
                        continue;
                    }
                    lines.push(SelectedLineDebug {
                        paragraph: paragraph_index,
                        top: line.content_top,
                        absolute_top: line.absolute_content_top,
                        height: line.content_height,
                        start_x: line.start_x,
                        end_x: line.visual_end_x(),
                        cursor_end_x: line.cursor_end_x,
                        visual_end_x: line.visual_end_x(),
                        start,
                        end,
                        has_fragments: false,
                        tab_leaders: line.tab_leaders.to_vec(),
                        tab_stops: line.tab_stops.to_vec(),
                    });
                    continue;
                }

                let mut overlaps = false;
                let bounds = logical_source_range(line.fragments.iter().map(|fragment| {
                    overlaps |= range_start < fragment.end && range_end > fragment.start;
                    (fragment.start, fragment.end)
                }));

                if overlaps {
                    let (start, end) = bounds.unwrap_or((0, 0));
                    lines.push(SelectedLineDebug {
                        paragraph: paragraph_index,
                        top: line.content_top,
                        absolute_top: line.absolute_content_top,
                        height: line.content_height,
                        start_x: line.start_x,
                        end_x: line.visual_end_x(),
                        cursor_end_x: line.cursor_end_x,
                        visual_end_x: line.visual_end_x(),
                        start,
                        end,
                        has_fragments: true,
                        tab_leaders: line.tab_leaders.to_vec(),
                        tab_stops: line.tab_stops.to_vec(),
                    });
                }
            }
        }

        lines
    }
}

fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    if text.is_empty() || offset == 0 {
        return 0;
    }

    let mut prev = 0usize;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= offset {
            break;
        }
        prev = idx;
    }

    prev
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    if text.is_empty() {
        return 0;
    }

    for (idx, _) in text.grapheme_indices(true) {
        if idx > offset {
            return idx;
        }
    }

    text.len()
}

/// A word for caret purposes: letters, digits and marks of one script run.
/// Contiguous CJK ideographs and kana form runs, while punctuation and
/// whitespace are boundaries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Space,
    Punctuation,
    Alphanumeric,
    Ideograph,
    Kana,
}

fn word_class(grapheme: &str) -> WordClass {
    let Some(ch) = grapheme.chars().next() else {
        return WordClass::Space;
    };
    if ch.is_whitespace() {
        WordClass::Space
    } else if matches!(ch as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F) {
        WordClass::Kana
    } else if matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F)
    {
        WordClass::Ideograph
    } else if ch.is_alphanumeric() || ch == '_' {
        WordClass::Alphanumeric
    } else {
        WordClass::Punctuation
    }
}

/// Splits `text` into runs of the same word class: `(start, end, class)`.
fn word_runs(text: &str) -> Vec<(usize, usize, WordClass)> {
    let mut runs: Vec<(usize, usize, WordClass)> = Vec::new();
    for (idx, grapheme) in text.grapheme_indices(true) {
        let class = word_class(grapheme);
        let end = idx + grapheme.len();
        match runs.last_mut() {
            Some((_, last_end, last_class)) if *last_class == class => {
                *last_end = end;
            }
            _ => runs.push((idx, end, class)),
        }
    }
    runs
}

/// Word's Ctrl+Left target: the start of the word run that contains the
/// character before `offset`, skipping spaces on the way back.
fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let runs = word_runs(text);
    let mut candidate = 0usize;
    for (start, _, class) in runs.iter().rev() {
        if *start >= offset {
            continue;
        }
        if *class == WordClass::Space {
            continue;
        }
        candidate = *start;
        break;
    }
    candidate
}

/// Word's Ctrl+Right target: the start of the next word run, so the spaces
/// after the current word are skipped too.
fn next_word_boundary(text: &str, offset: usize) -> usize {
    let runs = word_runs(text);
    let Some(current) = runs
        .iter()
        .position(|(start, end, _)| offset >= *start && offset < *end)
    else {
        return text.len();
    };
    let mut index = current + 1;
    while let Some((start, _, class)) = runs.get(index) {
        if *class != WordClass::Space {
            return *start;
        }
        index += 1;
    }
    text.len()
}

/// Word's double-click range: the word run at `offset` plus the spaces that
/// follow it. At the paragraph end the last run is used.
fn word_range_in(text: &str, offset: usize) -> (usize, usize) {
    let runs = word_runs(text);
    let index = runs
        .iter()
        .position(|(start, end, _)| offset >= *start && offset < *end)
        .unwrap_or(runs.len().saturating_sub(1));
    let (start, mut end, class) = runs[index];
    if class != WordClass::Space {
        if let Some((_, space_end, WordClass::Space)) = runs.get(index + 1) {
            end = *space_end;
        }
    }
    (start, end)
}

fn caret_x_for_line(line: &LineLayout, offset: usize) -> f64 {
    let mut caret_x = line.start_x;
    let min_x = line.start_x;
    let mut max_x = line.interactive_max_x();

    let mut handled_tab = false;
    for tab_stop in line.tab_stops.iter() {
        if offset == tab_stop.start_offset {
            caret_x = tab_stop.start_x;
            handled_tab = true;
            break;
        }
        if offset == tab_stop.end_offset {
            caret_x = tab_stop.end_x;
            handled_tab = true;
            break;
        }
    }

    if !handled_tab && !line.fragments.is_empty() {
        for fragment in line.fragments.iter() {
            if offset == fragment.start && fragment.start == fragment.end {
                caret_x = fragment.x + fragment.width;
                continue;
            }

            if offset <= fragment.start {
                caret_x = fragment.x;
                break;
            }

            if offset < fragment.end {
                let rel = fragment.relative_x_for_end(offset);
                caret_x = fragment.x + rel;
                break;
            }

            caret_x = fragment.x + fragment.width;
        }
    }

    if handled_tab {
        return caret_x.clamp(min_x, max_x);
    }

    if line.fragments.is_empty() {
        if let Some((start, end)) = line.break_range {
            let width = line.cursor_width().max(SOFT_BREAK_MIN_HIT_WIDTH);
            max_x = min_x + width;
            caret_x = min_x;
            if start != end && offset >= end {
                caret_x = max_x;
            }
        }
    } else if let Some((start, end)) = line.break_range {
        if offset >= end {
            if start == end {
                if let Some(last) = line.fragments.last() {
                    caret_x = last.x + last.width;
                    max_x = caret_x.max(min_x);
                } else {
                    caret_x = min_x;
                }
            } else {
                let width = line.cursor_width().max(SOFT_BREAK_MIN_HIT_WIDTH);
                max_x = min_x + width;
                caret_x = max_x;
            }
        }
    }

    caret_x.clamp(min_x, max_x)
}

#[derive(Clone, Copy)]
enum VerticalDirection {
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_reader::document::PAGE_BREAK_STR;
    use std::hint::black_box;
    use std::time::Instant;

    fn make_line(text: &str, base_x: f64, top: f64, absolute_top: f64) -> LineLayout {
        let mut glyphs = Vec::new();
        let mut fragment = FragmentLayout {
            baseline_shift_px: 0.0,
            start: 0,
            end: text.len(),
            x: base_x,
            width: 0.0,
            glyphs: Rc::from([]),
        };

        let mut cursor = 0.0f64;
        let mut offset = 0usize;
        for grapheme in text.graphemes(true) {
            let width = 10.0;
            glyphs.push(GlyphBox {
                start: offset,
                end: offset + grapheme.len(),
                x: cursor,
                width,
            });
            cursor += width;
            offset += grapheme.len();
        }

        fragment.glyphs = glyphs.into();
        fragment.width = cursor;

        LineLayout {
            column_index: 0,
            top,
            absolute_top,
            height: 20.0,
            baseline: 15.0,
            start_x: base_x,
            cursor_end_x: base_x + cursor,
            allowed_trailing_overflow_width: 0.0,
            content_top: top,
            absolute_content_top: absolute_top,
            content_height: 20.0,
            fragments: vec![fragment].into(),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        }
    }

    fn make_line_with_offset(
        start: usize,
        text: &str,
        base_x: f64,
        top: f64,
        absolute_top: f64,
    ) -> LineLayout {
        let mut glyphs = Vec::new();
        let mut fragment = FragmentLayout {
            baseline_shift_px: 0.0,
            start,
            end: start + text.len(),
            x: base_x,
            width: 0.0,
            glyphs: Rc::from([]),
        };

        let mut cursor = 0.0f64;
        let mut offset = start;
        for grapheme in text.graphemes(true) {
            let width = 10.0;
            glyphs.push(GlyphBox {
                start: offset,
                end: offset + grapheme.len(),
                x: cursor,
                width,
            });
            cursor += width;
            offset += grapheme.len();
        }

        fragment.glyphs = glyphs.into();
        fragment.width = cursor;

        LineLayout {
            column_index: 0,
            top,
            absolute_top,
            height: 20.0,
            baseline: top + 15.0,
            start_x: base_x,
            cursor_end_x: base_x + cursor,
            allowed_trailing_overflow_width: 0.0,
            content_top: top,
            absolute_content_top: absolute_top,
            content_height: 20.0,
            fragments: vec![fragment].into(),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        }
    }

    /// Builds one line of a vertically-stacked (tbRl) paragraph: a single
    /// grapheme occupying the Y band [top, top + height) within a column whose
    /// left edge is `column_x` and width is `column_width`.
    fn make_vertical_line(
        start: usize,
        grapheme: &str,
        column_x: f64,
        column_width: f64,
        top: f64,
        height: f64,
    ) -> LineLayout {
        LineLayout {
            column_index: 0,
            top,
            absolute_top: top,
            height,
            baseline: top + height,
            start_x: column_x,
            cursor_end_x: column_x + column_width,
            allowed_trailing_overflow_width: 0.0,
            content_top: top,
            absolute_content_top: top,
            content_height: height,
            fragments: Rc::from([]),
            break_range: Some((start, start + grapheme.len())),
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: true,
            vertical_reversed: false,
        }
    }

    fn build_vertical_paragraph() -> SelectionManager {
        // "あいう": each grapheme is 3 bytes (offsets 0/3/6, end 9), stacked in
        // a single column 100..120, each line 20px tall at top 0/20/40.
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "あいう");
        manager.record_line(0, make_vertical_line(0, "あ", 100.0, 20.0, 0.0, 20.0));
        manager.record_line(0, make_vertical_line(3, "い", 100.0, 20.0, 20.0, 20.0));
        manager.record_line(0, make_vertical_line(6, "う", 100.0, 20.0, 40.0, 20.0));
        manager
    }

    #[test]
    fn hit_test_vertical_resolves_offset_by_click_y() {
        let manager = build_vertical_paragraph();

        // Upper half of the first grapheme -> caret before it (offset 0).
        assert_eq!(
            manager.hit_test(110.0, 5.0).unwrap(),
            TextPosition::new(0, 0)
        );
        // Lower half of the first grapheme -> caret after it (offset 3).
        assert_eq!(
            manager.hit_test(110.0, 15.0).unwrap(),
            TextPosition::new(0, 3)
        );
        // Upper half of the second grapheme -> offset 3.
        assert_eq!(
            manager.hit_test(110.0, 25.0).unwrap(),
            TextPosition::new(0, 3)
        );
        // Upper half of the third grapheme -> offset 6.
        assert_eq!(
            manager.hit_test(110.0, 45.0).unwrap(),
            TextPosition::new(0, 6)
        );
        // Lower half of the third grapheme -> end of paragraph (offset 9).
        assert_eq!(
            manager.hit_test(110.0, 55.0).unwrap(),
            TextPosition::new(0, 9)
        );
    }

    #[test]
    fn vertical_hit_test_uses_y_with_painted_fragments() {
        for explicit_break_range in [false, true] {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.append_paragraph_text(0, "あ");
            let mut line = make_vertical_line(0, "あ", 100.0, 20.0, 20.0, 20.0);
            line.fragments = Rc::from([FragmentLayout {
                start: 0,
                end: 3,
                x: 100.0,
                width: 20.0,
                baseline_shift_px: 0.0,
                glyphs: Rc::from([GlyphBox {
                    start: 0,
                    end: 3,
                    x: 0.0,
                    width: 20.0,
                }]),
            }]);
            if !explicit_break_range {
                line.break_range = None;
            }
            manager.record_line(0, line);
            assert_eq!(manager.hit_test(101.0, 25.0), Some(TextPosition::new(0, 0)));
            assert_eq!(manager.hit_test(101.0, 35.0), Some(TextPosition::new(0, 3)));
            manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 3))));
            assert_eq!(manager.caret_metrics().unwrap().top, 40.0);
        }
    }

    #[test]
    fn bottom_to_top_hit_testing_and_caret_follow_source_order() {
        for painted_fragments in [false, true] {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.append_paragraph_text(0, "あいう");
            for (index, text) in ["あ", "い", "う"].iter().enumerate() {
                let start = index * 3;
                let mut line =
                    make_vertical_line(start, text, 100.0, 20.0, 40.0 - index as f64 * 20.0, 20.0);
                line.vertical_reversed = true;
                if painted_fragments {
                    line.fragments = Rc::from([FragmentLayout {
                        start,
                        end: start + 3,
                        x: 100.0,
                        width: 20.0,
                        baseline_shift_px: 0.0,
                        glyphs: Rc::from([GlyphBox {
                            start,
                            end: start + 3,
                            x: 0.0,
                            width: 20.0,
                        }]),
                    }]);
                }
                manager.record_line(0, line);
            }
            assert_eq!(manager.hit_test(101.0, 55.0), Some(TextPosition::new(0, 0)));
            assert_eq!(manager.hit_test(101.0, 50.0), Some(TextPosition::new(0, 0)));
            assert_eq!(manager.hit_test(101.0, 45.0), Some(TextPosition::new(0, 3)));
            assert_eq!(manager.hit_test(101.0, 5.0), Some(TextPosition::new(0, 9)));
            for (offset, top) in [(0, 60.0), (3, 40.0), (6, 20.0), (9, 0.0)] {
                manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(
                    0, offset,
                ))));
                let caret = manager.caret_metrics().unwrap();
                assert_eq!(caret.x, 100.0);
                assert_eq!(caret.top, top);
                assert_eq!(caret.absolute_top, top);
                assert_eq!(caret.width, 20.0);
            }
        }
    }

    #[test]
    fn selection_line_direction_metadata_fits_existing_memory_budget() {
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<LineLayout>(), 168);
    }

    #[test]
    fn caret_metrics_vertical_advances_down_the_column() {
        let mut manager = build_vertical_paragraph();

        let caret_for = |manager: &mut SelectionManager, offset: usize| {
            manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(
                0, offset,
            ))));
            manager.caret_metrics().expect("caret")
        };

        // Caret x stays pinned to the column's left edge for every offset.
        let c0 = caret_for(&mut manager, 0);
        assert!((c0.x - 100.0).abs() < f64::EPSILON);
        assert!((c0.top - 0.0).abs() < f64::EPSILON);

        let c3 = caret_for(&mut manager, 3);
        assert!((c3.x - 100.0).abs() < f64::EPSILON);
        assert!((c3.top - 20.0).abs() < f64::EPSILON);

        let c6 = caret_for(&mut manager, 6);
        assert!((c6.top - 40.0).abs() < f64::EPSILON);

        // End of paragraph: caret sits just below the final grapheme.
        let c9 = caret_for(&mut manager, 9);
        assert!((c9.x - 100.0).abs() < f64::EPSILON);
        assert!((c9.top - 60.0).abs() < f64::EPSILON);
        assert!((c9.width - 20.0).abs() < f64::EPSILON);
        assert!((c9.height - CARET_THICKNESS).abs() < f64::EPSILON);
    }

    fn build_two_column_vertical_paragraph() -> SelectionManager {
        // "あいうえ" wrapped into two tbRl columns. tbRl flows right-to-left, so
        // the first (earlier-offset) column is on the RIGHT (x 100..120) and the
        // second column is to its LEFT (x 70..90). Both columns share the same
        // Y bands [0,20) and [20,40), so the click X must disambiguate columns.
        //   right column: あ(0..3) top 0..20, い(3..6) top 20..40
        //   left  column: う(6..9) top 0..20, え(9..12) top 20..40, end 12
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "あいうえ");
        manager.record_line(0, make_vertical_line(0, "あ", 100.0, 20.0, 0.0, 20.0));
        manager.record_line(0, make_vertical_line(3, "い", 100.0, 20.0, 20.0, 20.0));
        manager.record_line(0, make_vertical_line(6, "う", 70.0, 20.0, 0.0, 20.0));
        manager.record_line(0, make_vertical_line(9, "え", 70.0, 20.0, 20.0, 20.0));
        manager
    }

    #[test]
    fn hit_test_vertical_disambiguates_columns_by_click_x() {
        let manager = build_two_column_vertical_paragraph();

        // Right column (x ~110): resolves to the early-offset graphemes.
        assert_eq!(
            manager.hit_test(110.0, 5.0).unwrap(),
            TextPosition::new(0, 0)
        );
        assert_eq!(
            manager.hit_test(110.0, 15.0).unwrap(),
            TextPosition::new(0, 3)
        );
        assert_eq!(
            manager.hit_test(110.0, 25.0).unwrap(),
            TextPosition::new(0, 3)
        );
        assert_eq!(
            manager.hit_test(110.0, 35.0).unwrap(),
            TextPosition::new(0, 6)
        );

        // Left column (x ~80) at the SAME Y bands must pick the left column's
        // graphemes (offsets 6/9), not the right column's (0/3).
        assert_eq!(
            manager.hit_test(80.0, 5.0).unwrap(),
            TextPosition::new(0, 6)
        );
        assert_eq!(
            manager.hit_test(80.0, 15.0).unwrap(),
            TextPosition::new(0, 9)
        );
        assert_eq!(
            manager.hit_test(80.0, 25.0).unwrap(),
            TextPosition::new(0, 9)
        );
        assert_eq!(
            manager.hit_test(80.0, 35.0).unwrap(),
            TextPosition::new(0, 12)
        );
    }

    #[test]
    fn caret_metrics_vertical_lands_in_correct_column() {
        let mut manager = build_two_column_vertical_paragraph();

        let caret_for = |manager: &mut SelectionManager, offset: usize| {
            manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(
                0, offset,
            ))));
            manager.caret_metrics().expect("caret")
        };

        // Right column offsets pin x to 100 and walk down its Y bands.
        let c0 = caret_for(&mut manager, 0);
        assert!((c0.x - 100.0).abs() < f64::EPSILON);
        assert!((c0.top - 0.0).abs() < f64::EPSILON);
        let c3 = caret_for(&mut manager, 3);
        assert!((c3.x - 100.0).abs() < f64::EPSILON);
        assert!((c3.top - 20.0).abs() < f64::EPSILON);

        // Left column start (offset 6) must jump to the LEFT column's top
        // (x 70, top 0), not continue below the right column.
        let c6 = caret_for(&mut manager, 6);
        assert!((c6.x - 70.0).abs() < f64::EPSILON);
        assert!((c6.top - 0.0).abs() < f64::EPSILON);
        let c9 = caret_for(&mut manager, 9);
        assert!((c9.x - 70.0).abs() < f64::EPSILON);
        assert!((c9.top - 20.0).abs() < f64::EPSILON);

        // Paragraph end (offset 12) sits just below the final left-column glyph.
        let c12 = caret_for(&mut manager, 12);
        assert!((c12.x - 70.0).abs() < f64::EPSILON);
        assert!((c12.top - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn hit_test_returns_expected_offsets() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcde");
        manager.record_line(0, make_line("abcde", 5.0, 0.0, 0.0));

        let pos_start = manager.hit_test(5.0, 10.0).unwrap();
        assert_eq!(pos_start.paragraph, 0);
        assert_eq!(pos_start.offset, 0);

        let pos_middle = manager.hit_test(25.0, 10.0).unwrap();
        assert_eq!(pos_middle.paragraph, 0);
        assert_eq!(pos_middle.offset, "ab".len());

        let pos_end = manager.hit_test(200.0, 10.0).unwrap();
        assert_eq!(pos_end.offset, "abcde".len());
    }

    #[test]
    fn hit_test_matching_skips_ineligible_paragraph_surfaces() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for (paragraph, top) in [(0, 0.0), (1, 40.0), (2, 80.0)] {
            manager.append_paragraph_text(paragraph, "anchor");
            manager.record_line(paragraph, make_line("anchor", 5.0, top, top));
        }

        assert_eq!(manager.hit_test(10.0, 50.0).unwrap().paragraph, 1);
        let body_hit = manager
            .hit_test_matching(10.0, 50.0, |paragraph| paragraph != 1)
            .expect("nearest eligible paragraph");
        assert_eq!(body_hit.paragraph, 2);
    }

    #[test]
    fn anchor_hit_test_prefers_the_later_line_top_before_horizontal_proximity() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "wide source paragraph");
        manager.record_line(0, make_line("wide source paragraph", 0.0, 150.0, 150.0));
        manager.append_paragraph_text(1, "destination");
        manager.record_line(1, make_line("destination", 200.0, 168.0, 168.0));

        // Both line boxes contain y=168. Caret hit-testing prefers the source
        // because x=5 is closer, while a Word floating anchor follows the line
        // whose top edge was crossed most recently.
        assert_eq!(manager.hit_test(5.0, 168.0).unwrap().paragraph, 0);
        assert_eq!(
            manager.hit_test_anchor_paragraph_matching(5.0, 168.0, |_| true),
            Some(1)
        );

        manager.append_paragraph_text(2, "left cell");
        manager.record_line(2, make_line("left cell", 0.0, 220.0, 220.0));
        manager.append_paragraph_text(3, "right cell");
        manager.record_line(3, make_line("right cell", 200.0, 220.0, 220.0));
        assert_eq!(
            manager.hit_test_anchor_paragraph_matching(210.0, 225.0, |_| true),
            Some(3),
            "same-row table cells remain horizontally disambiguated"
        );
    }

    #[test]
    fn apply_y_offset_from_moves_caret_metrics_fields() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc");
        manager.record_line(0, make_line("abc", 0.0, 10.0, 110.0));
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 1))));

        manager.apply_y_offset_from(0, 24.0);

        let metrics = manager.caret_metrics().expect("caret metrics");
        assert_eq!(metrics.top, 34.0);
        assert_eq!(metrics.absolute_top, 134.0);
    }

    #[test]
    fn hit_test_resolves_when_paragraph_bounds_are_not_y_sorted() {
        // More than 10 paragraphs forces the binary-search candidate path, and
        // recording them with non-monotonic `absolute_top` (as happens across
        // columns/pages) exercises the fallback sort branch. The click must
        // still land on the paragraph whose Y band contains it.
        let mut manager = SelectionManager::new();
        manager.begin_render();
        let count = 14usize;
        // Interleave Y positions: even indices low, odd indices high, so the
        // index order is deliberately not ascending in `absolute_top`.
        for i in 0..count {
            let abs_top = if i % 2 == 0 {
                i as f64 * 10.0
            } else {
                500.0 + i as f64 * 10.0
            };
            manager.append_paragraph_text(i, "abcde");
            manager.record_line(i, make_line("abcde", 5.0, abs_top, abs_top));
        }

        // Paragraph 6 sits at absolute_top = 60 (height 20 -> band 60..80).
        let hit = manager
            .hit_test(10.0, 70.0)
            .expect("hit inside paragraph 6");
        assert_eq!(hit.paragraph, 6);

        // Paragraph 7 sits at absolute_top = 570 (band 570..590).
        let hit_high = manager
            .hit_test(10.0, 580.0)
            .expect("hit inside paragraph 7");
        assert_eq!(hit_high.paragraph, 7);
    }

    #[test]
    fn repeated_hit_tests_reuse_paragraph_bounds_index() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for index in 0..20 {
            let top = index as f64 * 20.0;
            manager.append_paragraph_text(index, "text");
            manager.record_line(index, make_line("text", 0.0, top, top));
        }

        assert_eq!(manager.hit_test(5.0, 45.0).unwrap().paragraph, 2);
        assert_eq!(manager.hit_test(5.0, 65.0).unwrap().paragraph, 3);
        assert_eq!(
            manager.paragraph_bounds_build_count(),
            1,
            "pointer moves over unchanged layout must share one bounds index"
        );
    }

    #[test]
    fn paragraph_bounds_index_rebuilds_after_geometry_shift() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for index in 0..20 {
            let top = index as f64 * 20.0;
            manager.append_paragraph_text(index, "text");
            manager.record_line(index, make_line("text", 0.0, top, top));
        }
        assert_eq!(manager.hit_test(5.0, 45.0).unwrap().paragraph, 2);

        manager.apply_y_offset_from(2, 100.0);

        assert_eq!(manager.hit_test(5.0, 145.0).unwrap().paragraph, 2);
        assert_eq!(manager.paragraph_bounds_build_count(), 2);
    }

    /// Lays out `count` single-line paragraphs stacked vertically (band
    /// `i*20 .. i*20+20`), as a full layout render would. Used to model the
    /// bitmap "pixel fetch" hazard the cheap-prefix skip exposes.
    fn build_tall_single_column_document(count: usize) -> SelectionManager {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for i in 0..count {
            let abs_top = i as f64 * 20.0;
            manager.append_paragraph_text(i, "hello");
            manager.record_line(i, make_line("hello", 5.0, abs_top, abs_top));
        }
        manager
    }

    #[test]
    fn unfrozen_narrow_rerender_drops_out_of_window_selection() {
        // Regression contract: a bitmap pixel fetch whose visible window is
        // NARROWER than what is on screen only re-records that window. With the
        // cheap-prefix skip, the from-0 walk no longer re-records the whole
        // prefix, so rebuilding selection from such a narrow render drops the
        // line layouts of on-screen paragraphs outside the window — and clicking
        // them resolves to the wrong paragraph. This pins WHY the fix is needed.
        let mut manager = build_tall_single_column_document(60);
        assert_eq!(manager.hit_test(10.0, 205.0).expect("hit").paragraph, 10);

        // Pixel fetch for a narrow window [48..52] WITHOUT freezing.
        manager.begin_render();
        for i in 48..52 {
            let abs_top = i as f64 * 20.0;
            manager.append_paragraph_text(i, "hello");
            manager.record_line(i, make_line("hello", 5.0, abs_top, abs_top));
        }

        // Paragraph 10's band (200..220) no longer resolves to paragraph 10.
        let resolved = manager.hit_test(10.0, 205.0).map(|p| p.paragraph);
        assert_ne!(
            resolved,
            Some(10),
            "unfrozen narrow re-render must drop paragraph 10's selection (it did not)"
        );
    }

    #[test]
    fn frozen_pixel_rerender_preserves_offscreen_window_selection_for_hit_test() {
        // Fix contract: a bitmap pixel fetch FREEZES selection first, so
        // `begin_render` and the narrow re-record become no-ops and the prior
        // complete selection survives. (The render auto-unfreezes at the end;
        // mirrored here.) Hit-testing stays correct for on-screen paragraphs
        // outside the fetch window.
        let mut manager = build_tall_single_column_document(60);
        assert_eq!(manager.hit_test(10.0, 205.0).expect("hit").paragraph, 10);

        manager.freeze();
        manager.begin_render();
        for i in 48..52 {
            let abs_top = i as f64 * 20.0;
            manager.append_paragraph_text(i, "hello");
            manager.record_line(i, make_line("hello", 5.0, abs_top, abs_top));
        }
        manager.unfreeze();

        // Paragraph 10 (band 200..220) and a deep paragraph 55 (band 1100..1120)
        // both still resolve correctly after the frozen pixel fetch.
        assert_eq!(manager.hit_test(10.0, 205.0).expect("hit").paragraph, 10);
        assert_eq!(manager.hit_test(10.0, 1105.0).expect("hit").paragraph, 55);
    }

    #[test]
    fn hit_test_handles_break_only_line() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "\n");
        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 0.0,
                absolute_top: 0.0,
                height: 20.0,
                baseline: 15.0,
                start_x: 10.0,
                cursor_end_x: 10.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: 0.0,
                absolute_content_top: 0.0,
                content_height: 20.0,
                fragments: Rc::from([]),
                break_range: Some((0, 1)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        let before = manager.hit_test(5.0, 10.0).expect("position");
        assert_eq!(before, TextPosition::new(0, 0));

        let after = manager.hit_test(25.0, 10.0).expect("position");
        assert_eq!(after, TextPosition::new(0, 1));
    }

    #[test]
    fn caret_x_for_paragraph_end_stays_inside_mark() {
        let mut line = make_line("abcd", 10.0, 0.0, 0.0);
        let text_end_x = line
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(line.start_x);
        line.cursor_end_x = text_end_x + 12.0;
        line.break_range = Some((4, 4));

        let caret = caret_x_for_line(&line, 4);
        assert!((caret - text_end_x).abs() < f64::EPSILON);
    }

    #[test]
    fn caret_x_after_text_following_zero_length_inline_object_uses_document_offsets() {
        let fragment = FragmentLayout {
            baseline_shift_px: 0.0,
            start: 0,
            end: 10,
            x: 10.0,
            width: 200.0,
            glyphs: vec![
                GlyphBox {
                    start: 0,
                    end: 5,
                    x: 0.0,
                    width: 50.0,
                },
                GlyphBox {
                    start: 5,
                    end: 6,
                    x: 50.0,
                    width: 10.0,
                },
                GlyphBox {
                    start: 6,
                    end: 6,
                    x: 60.0,
                    width: 100.0,
                },
                GlyphBox {
                    start: 6,
                    end: 10,
                    x: 160.0,
                    width: 40.0,
                },
            ]
            .into(),
        };
        let line = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 0.0,
            height: 120.0,
            baseline: 100.0,
            start_x: 10.0,
            cursor_end_x: 210.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 0.0,
            content_height: 120.0,
            fragments: vec![fragment].into(),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };

        let before_following_text = caret_x_for_line(&line, 6);
        assert!((before_following_text - 170.0).abs() < f64::EPSILON);

        let after_following_text = caret_x_for_line(&line, 10);
        assert!((after_following_text - 210.0).abs() < f64::EPSILON);
    }

    #[test]
    fn caret_x_at_zero_length_inline_object_fragment_uses_object_trailing_edge() {
        let text_fragment = FragmentLayout {
            baseline_shift_px: 0.0,
            start: 0,
            end: 6,
            x: 10.0,
            width: 60.0,
            glyphs: vec![GlyphBox {
                start: 0,
                end: 6,
                x: 0.0,
                width: 60.0,
            }]
            .into(),
        };
        let image_fragment = FragmentLayout {
            baseline_shift_px: 0.0,
            start: 6,
            end: 6,
            x: 70.0,
            width: 100.0,
            glyphs: vec![GlyphBox {
                start: 6,
                end: 6,
                x: 0.0,
                width: 100.0,
            }]
            .into(),
        };
        let line = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 0.0,
            height: 120.0,
            baseline: 100.0,
            start_x: 10.0,
            cursor_end_x: 170.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 0.0,
            content_height: 120.0,
            fragments: vec![text_fragment, image_fragment].into(),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };

        let caret = caret_x_for_line(&line, 6);
        assert!((caret - 170.0).abs() < f64::EPSILON);
    }

    #[test]
    fn caret_x_for_empty_paragraph_break_prefers_inner_position() {
        let line = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 0.0,
            height: 20.0,
            baseline: 15.0,
            start_x: 32.0,
            cursor_end_x: 48.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 0.0,
            content_height: 20.0,
            fragments: Rc::from([]),
            break_range: Some((0, 0)),
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };

        let caret = caret_x_for_line(&line, 0);
        assert!((caret - 32.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_table_line_keeps_hit_bounds_but_uses_insertion_origin_for_caret() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.ensure_paragraph(0);
        manager.record_line_with_caret_start(
            0,
            LineLayout {
                column_index: 0,
                top: 10.0,
                absolute_top: 10.0,
                height: 20.0,
                baseline: 25.0,
                start_x: 100.0,
                cursor_end_x: 200.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: 10.0,
                absolute_content_top: 10.0,
                content_height: 20.0,
                fragments: Rc::from([]),
                break_range: Some((0, 0)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
            112.0,
        );
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 0))));

        let caret = manager.caret_metrics().expect("caret metrics");
        assert!((caret.x - 112.0).abs() < f64::EPSILON);
        assert_eq!(manager.hit_test(101.0, 20.0), Some(TextPosition::new(0, 0)));
    }

    #[test]
    fn caret_metrics_for_paragraph_end_uses_inner_position() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "hello");

        let mut line = make_line("hello", 8.0, 0.0, 0.0);
        let text_end = line
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(8.0);
        line.cursor_end_x = text_end + 10.0;
        line.break_range = Some(("hello".len(), "hello".len()));
        manager.record_line(0, line);

        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(
            0,
            "hello".len(),
        ))));

        let metrics = manager.caret_metrics().expect("caret metrics");
        assert!(
            (metrics.x - text_end).abs() < f64::EPSILON,
            "expected caret at {text_end}, got {}",
            metrics.x
        );
    }

    #[test]
    fn a_caret_on_a_soft_wrap_is_drawn_on_the_line_its_affinity_names() {
        // "abc" wraps onto "def": offset 3 ends the first line and starts
        // the second.
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.record_line(0, make_line_with_offset(3, "def", 0.0, 20.0, 20.0));
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 3))));

        // Upstream, the default: the end of the first line, as before.
        let upstream = manager.caret_metrics().expect("caret");
        assert_eq!((upstream.x, upstream.absolute_top), (30.0, 0.0));
        // Downstream: the start of the second.
        manager.set_caret_affinity(CaretAffinity::Downstream);
        let downstream = manager.caret_metrics().expect("caret");
        assert_eq!((downstream.x, downstream.absolute_top), (0.0, 20.0));
        // Away from a wrap, affinity changes nothing.
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 1))));
        assert_eq!(manager.caret_metrics().expect("caret").absolute_top, 0.0);
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 6))));
        assert_eq!(manager.caret_metrics().expect("caret").absolute_top, 20.0);
    }

    #[test]
    fn a_hard_break_is_not_a_soft_wrap() {
        // "abc", a line break (offset 3..4), then "def" from offset 4.
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc\ndef");
        let mut first = make_line_with_offset(0, "abc", 0.0, 0.0, 0.0);
        first.break_range = Some((3, 4));
        manager.record_line(0, first);
        manager.record_line(0, make_line_with_offset(4, "def", 0.0, 20.0, 20.0));
        manager.set_caret_affinity(CaretAffinity::Downstream);
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 3))));
        assert_eq!(manager.caret_metrics().expect("caret").absolute_top, 0.0);
    }

    #[test]
    fn visual_end_x_prefers_fragment_extent_over_cursor_end_x() {
        let mut line = make_line("hello", 8.0, 0.0, 0.0);
        let text_end = line
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(8.0);
        line.cursor_end_x = text_end - 3.0;

        assert!((line.visual_end_x() - text_end).abs() < f64::EPSILON);
    }

    #[test]
    fn vertical_visual_end_x_keeps_column_bounds_without_y_advances() {
        let mut line = make_line("H", 8.0, 0.0, 0.0);
        line.vertical = true;
        line.cursor_end_x = 26.0;
        let fragment = &mut Rc::make_mut(&mut line.fragments)[0];
        fragment.x = 20.0;
        fragment.width = 48.0;
        for reversed in [false, true] {
            line.vertical_reversed = reversed;
            assert_eq!(line.cursor_width(), 18.0);
            assert_eq!(line.visual_end_x(), 26.0);
        }
    }

    #[test]
    fn caret_metrics_prefers_blank_break_line() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc\n\n");

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 0.0,
                absolute_top: 0.0,
                height: 20.0,
                baseline: 20.0,
                start_x: 10.0,
                cursor_end_x: 40.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: 0.0,
                absolute_content_top: 0.0,
                content_height: 20.0,
                fragments: vec![FragmentLayout {
                    baseline_shift_px: 0.0,
                    start: 0,
                    end: 3,
                    x: 10.0,
                    width: 30.0,
                    glyphs: vec![
                        GlyphBox {
                            start: 0,
                            end: 1,
                            x: 0.0,
                            width: 10.0,
                        },
                        GlyphBox {
                            start: 1,
                            end: 2,
                            x: 10.0,
                            width: 10.0,
                        },
                        GlyphBox {
                            start: 2,
                            end: 3,
                            x: 20.0,
                            width: 10.0,
                        },
                    ]
                    .into(),
                }]
                .into(),
                break_range: Some((3, 4)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 20.0,
                absolute_top: 20.0,
                height: 20.0,
                baseline: 40.0,
                start_x: 10.0,
                cursor_end_x: 10.0 + SOFT_BREAK_MIN_HIT_WIDTH,
                allowed_trailing_overflow_width: 0.0,
                content_top: 20.0,
                absolute_content_top: 20.0,
                content_height: 20.0,
                fragments: Rc::from([]),
                break_range: Some((4, 5)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 4))));

        let metrics = manager.caret_metrics().expect("caret metrics");
        assert!(
            (metrics.absolute_top - 20.0).abs() < f64::EPSILON,
            "expected caret on blank line, got absolute_top={} (expected 20.0)",
            metrics.absolute_top
        );
    }

    #[test]
    fn caret_metrics_prefers_deep_blank_break_line() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "hello\n\n\n");

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 0.0,
                absolute_top: 0.0,
                height: 16.0,
                baseline: 16.0,
                start_x: 10.0,
                cursor_end_x: 50.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: 0.0,
                absolute_content_top: 0.0,
                content_height: 16.0,
                fragments: vec![FragmentLayout {
                    baseline_shift_px: 0.0,
                    start: 0,
                    end: 5,
                    x: 10.0,
                    width: 40.0,
                    glyphs: Rc::from([]),
                }]
                .into(),
                break_range: Some((5, 6)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 16.0,
                absolute_top: 16.0,
                height: 16.0,
                baseline: 32.0,
                start_x: 10.0,
                cursor_end_x: 10.0 + SOFT_BREAK_MIN_HIT_WIDTH,
                allowed_trailing_overflow_width: 0.0,
                content_top: 16.0,
                absolute_content_top: 16.0,
                content_height: 16.0,
                fragments: Rc::from([]),
                break_range: Some((6, 7)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 32.0,
                absolute_top: 32.0,
                height: 16.0,
                baseline: 48.0,
                start_x: 10.0,
                cursor_end_x: 10.0 + SOFT_BREAK_MIN_HIT_WIDTH,
                allowed_trailing_overflow_width: 0.0,
                content_top: 32.0,
                absolute_content_top: 32.0,
                content_height: 16.0,
                fragments: Rc::from([]),
                break_range: Some((7, 8)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 7))));

        let metrics = manager.caret_metrics().expect("caret metrics");
        assert!(
            (metrics.absolute_top - 32.0).abs() < f64::EPSILON,
            "expected caret on deepest blank line, got absolute_top={} (expected 32.0)",
            metrics.absolute_top
        );
    }

    #[test]
    fn caret_metrics_use_content_box_for_visual_alignment() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc");

        manager.record_line(
            0,
            LineLayout {
                column_index: 0,
                top: 96.0,
                absolute_top: 4096.0,
                height: 22.0,
                baseline: 112.0,
                start_x: 24.0,
                cursor_end_x: 54.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: 101.0,
                absolute_content_top: 4101.0,
                content_height: 14.0,
                fragments: vec![FragmentLayout {
                    baseline_shift_px: 0.0,
                    start: 0,
                    end: 3,
                    x: 24.0,
                    width: 30.0,
                    glyphs: Rc::from([]),
                }]
                .into(),
                break_range: None,
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );

        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 1))));

        let metrics = manager.caret_metrics().expect("caret metrics");
        assert!(
            (metrics.top - 101.0).abs() < f64::EPSILON,
            "expected caret top to match content_top, got {}",
            metrics.top
        );
        assert!(
            (metrics.absolute_top - 4101.0).abs() < f64::EPSILON,
            "expected caret absolute_top to match absolute_content_top, got {}",
            metrics.absolute_top
        );
        assert!(
            (metrics.height - 14.0).abs() < f64::EPSILON,
            "expected caret height to match content_height, got {}",
            metrics.height
        );
    }

    #[test]
    fn caret_and_hit_test_respect_tab_stop_layout() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello\tWorld");

        let line = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 0.0,
            height: 20.0,
            baseline: 15.0,
            start_x: 10.0,
            cursor_end_x: 220.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 0.0,
            content_height: 20.0,
            fragments: vec![
                FragmentLayout {
                    baseline_shift_px: 0.0,
                    start: 0,
                    end: 5,
                    x: 10.0,
                    width: 50.0,
                    glyphs: Rc::from([]),
                },
                FragmentLayout {
                    baseline_shift_px: 0.0,
                    start: 6,
                    end: 11,
                    x: 160.0,
                    width: 60.0,
                    glyphs: Rc::from([]),
                },
            ]
            .into(),
            break_range: None,
            tab_leaders: Rc::from([]),
            tab_stops: vec![TabStopLayout {
                start_offset: 5,
                end_offset: 6,
                start_x: 60.0,
                end_x: 150.0,
            }]
            .into(),
            vertical: false,
            vertical_reversed: false,
        };
        manager.record_line(0, line.clone());
        assert_eq!(
            manager.paragraphs[0].lines[0].tab_stops.len(),
            1,
            "tab stop should be recorded"
        );

        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 5))));
        let metrics = manager.caret_metrics().expect("caret metrics");
        assert!(
            (metrics.x - 60.0).abs() < f64::EPSILON,
            "caret should align with tab stop"
        );

        let tab_click = manager.hit_test(80.0, 10.0).expect("tab hit");
        assert_eq!(
            tab_click,
            TextPosition::new(0, 5),
            "click within tab gap should map to tab offset"
        );
    }

    #[test]
    fn move_across_paragraph_boundaries() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc");
        manager.record_line(0, make_line("abc", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "de");
        manager.record_line(1, make_line("de", 0.0, 0.0, 2000.0));

        let right = manager.move_right(TextPosition::new(0, "abc".len()));
        assert_eq!(right, TextPosition::new(1, 0));

        let left = manager.move_left(TextPosition::new(1, 0));
        assert_eq!(left, TextPosition::new(0, "abc".len()));
    }

    #[test]
    fn move_across_empty_paragraphs_one_at_a_time() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for index in 0..3 {
            manager.append_paragraph_text(index, "");
            manager.record_line(index, make_line("", 0.0, 0.0, index as f64 * 1000.0));
        }

        assert_eq!(
            manager.move_left(TextPosition::new(2, 0)),
            TextPosition::new(1, 0)
        );
        assert_eq!(
            manager.move_left(TextPosition::new(1, 0)),
            TextPosition::new(0, 0)
        );
        assert_eq!(
            manager.move_right(TextPosition::new(0, 0)),
            TextPosition::new(1, 0)
        );
        assert_eq!(
            manager.move_right(TextPosition::new(1, 0)),
            TextPosition::new(2, 0)
        );

        manager.set_paragraph_selectable(1, false);
        assert_eq!(
            manager.move_left(TextPosition::new(2, 0)),
            TextPosition::new(0, 0)
        );
        assert_eq!(
            manager.move_right(TextPosition::new(0, 0)),
            TextPosition::new(2, 0)
        );
    }

    #[test]
    fn selection_text_spans_multiple_paragraphs() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "ef");
        manager.record_line(1, make_line("ef", 0.0, 0.0, 1000.0));

        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, "ab".len()),
            TextPosition::new(1, "e".len()),
        )));

        assert_eq!(manager.selection_text().as_deref(), Some("cd\ne"));
    }

    #[test]
    fn vertical_move_preserves_column_within_paragraph() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.record_line(0, make_line_with_offset(3, "def", 0.0, 20.0, 20.0));

        let (down_pos, _) = manager.move_down(TextPosition::new(0, 1), None);
        assert_eq!(down_pos, TextPosition::new(0, 4));

        let (up_pos, _) = manager.move_up(down_pos, None);
        assert_eq!(up_pos, TextPosition::new(0, 1));
    }

    #[test]
    fn vertical_move_crosses_paragraphs() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abc");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "d");
        manager.record_line(1, make_line_with_offset(0, "d", 0.0, 20.0, 200.0));

        let goal_x = manager
            .caret_x_for_position(TextPosition::new(0, "abc".len()))
            .unwrap();

        let (down_pos, _) = manager.move_down(TextPosition::new(0, "abc".len()), Some(goal_x));
        assert_eq!(down_pos, TextPosition::new(1, "d".len()));

        let (up_pos, _) = manager.move_up(down_pos, Some(goal_x));
        assert_eq!(up_pos, TextPosition::new(0, "abc".len()));
    }

    #[test]
    fn vertical_move_clamps_to_line_end_when_goal_exceeds_width() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.record_line(0, make_line_with_offset(3, "def", 0.0, 20.0, 20.0));

        let goal_x = 120.0;
        let (down_pos, actual_x) = manager.move_down(TextPosition::new(0, 2), Some(goal_x));
        assert_eq!(down_pos, TextPosition::new(0, "abcdef".len()));
        assert!(
            (actual_x - 30.0).abs() < f64::EPSILON,
            "expected caret at line end, got {actual_x}"
        );
    }

    #[test]
    fn word_movement_and_double_click_ranges_follow_word() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "alpha  beta, gamma");
        manager.record_line(0, make_line("alpha  beta, gamma", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "日本語のテキスト");
        manager.record_line(1, make_line("日本語のテキスト", 0.0, 20.0, 20.0));

        let text = "alpha  beta, gamma";
        // Ctrl+Left from inside "gamma" goes to its start, then to "beta".
        let gamma = text.find("gamma").unwrap();
        assert_eq!(
            manager.move_word_left(TextPosition::new(0, gamma + 2)),
            TextPosition::new(0, gamma)
        );
        // Punctuation is a word of its own for the caret, as in Word.
        let comma = text.find(',').unwrap();
        assert_eq!(
            manager.move_word_left(TextPosition::new(0, gamma)),
            TextPosition::new(0, comma)
        );
        assert_eq!(
            manager.move_word_left(TextPosition::new(0, comma)),
            TextPosition::new(0, text.find("beta").unwrap())
        );
        // Ctrl+Right skips the trailing spaces to the next word start.
        assert_eq!(
            manager.move_word_right(TextPosition::new(0, 1)),
            TextPosition::new(0, text.find("beta").unwrap())
        );
        assert_eq!(
            manager.move_word_right(TextPosition::new(0, gamma + 1)),
            TextPosition::new(0, text.len())
        );
        // Across paragraphs the word steps fall back to the character steps.
        assert_eq!(
            manager.move_word_right(TextPosition::new(0, text.len())),
            TextPosition::new(1, 0)
        );
        assert_eq!(
            manager.move_word_left(TextPosition::new(1, 0)),
            TextPosition::new(0, text.len())
        );
        // Double-click on "alpha" takes the spaces after it.
        assert_eq!(
            manager.word_range_at(TextPosition::new(0, 2)),
            Some((0, text.find("beta").unwrap()))
        );
        // A comma is a word of its own; on whitespace the spaces are selected.
        assert_eq!(
            manager.word_range_at(TextPosition::new(0, comma)),
            Some((comma, comma + 2))
        );
        // Word groups contiguous ideographs and kana for caret movement and
        // double-click selection.
        let kanji = "日本語のテキスト";
        assert_eq!(
            manager.move_word_right(TextPosition::new(1, 0)),
            TextPosition::new(1, "日本語".len())
        );
        assert_eq!(
            manager.word_range_at(TextPosition::new(1, "日本".len())),
            Some((0, "日本語".len()))
        );
        assert_eq!(
            manager.word_range_at(TextPosition::new(1, "日本語のテ".len())),
            Some(("日本語".len(), kanji.len()))
        );
    }

    fn visual_source_line(parts: &[(usize, &str)], top: f64) -> LineLayout {
        let mut line = make_line("", 0.0, top, top);
        let mut fragments = Vec::new();
        let mut x = 0.0;
        for &(offset, text) in parts {
            let mut part = make_line_with_offset(offset, text, x, top, top);
            let fragment = Rc::make_mut(&mut part.fragments)[0].clone();
            x += fragment.width;
            fragments.push(fragment);
        }
        line.fragments = fragments.into();
        line.cursor_end_x = x;
        line
    }

    #[test]
    fn visual_rtl_source_bounds_drive_home_end_and_caret_line_affinity() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "אבגדאבגד");
        let first = visual_source_line(&[(6, "ד"), (4, "ג"), (2, "ב"), (0, "א")], 0.0);
        let second = visual_source_line(&[(14, "ד"), (12, "ג"), (10, "ב"), (8, "א")], 20.0);
        manager.record_line(0, first);
        manager.record_line(0, second);
        assert_eq!(
            manager.move_line_start(TextPosition::new(0, 4)),
            TextPosition::new(0, 0)
        );
        assert_eq!(
            manager.move_line_end(TextPosition::new(0, 4)),
            TextPosition::new(0, 8)
        );
        assert_eq!(
            manager.move_line_start(TextPosition::new(0, 12)),
            TextPosition::new(0, 8)
        );
        assert_eq!(
            manager.move_line_end(TextPosition::new(0, 12)),
            TextPosition::new(0, 16)
        );
        assert_eq!(
            manager
                .caret_metrics_at_position(TextPosition::new(0, 8))
                .unwrap()
                .top,
            0.0,
            "preserve existing previous-line affinity at a wrap boundary"
        );
        manager.set_selection(Some(SelectionRange {
            anchor: TextPosition::new(0, 0),
            focus: TextPosition::new(0, 8),
        }));
        let selected = manager.selected_lines();
        assert_eq!(selected.len(), 1);
        assert_eq!((selected[0].start, selected[0].end), (0, 8));
        manager.continue_paragraph_registration(0, "אבגדאבגד", 12);
        assert_eq!(
            manager.paragraphs()[0].lines.len(),
            2,
            "registration uses logical starts"
        );
    }

    #[test]
    fn mixed_visual_source_bounds_cover_interior_extrema() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "אבגדabהו");
        let line = visual_source_line(
            &[
                (6, "ד"),
                (4, "ג"),
                (2, "ב"),
                (0, "א"),
                (8, "a"),
                (9, "b"),
                (12, "ו"),
                (10, "ה"),
            ],
            0.0,
        );
        manager.record_line(0, line);
        assert_eq!(
            manager.move_line_start(TextPosition::new(0, 9)),
            TextPosition::new(0, 0)
        );
        manager.set_selection(Some(SelectionRange {
            anchor: TextPosition::new(0, 0),
            focus: TextPosition::new(0, 14),
        }));
        let selected = manager.selected_lines();
        assert_eq!((selected[0].start, selected[0].end), (0, 14));
    }

    #[test]
    fn home_and_end_move_within_the_caret_line_and_ctrl_variants_to_the_document_ends() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.record_line(0, make_line_with_offset(3, "def", 0.0, 20.0, 20.0));
        manager.append_paragraph_text(1, "gh");
        manager.record_line(1, make_line_with_offset(0, "gh", 0.0, 40.0, 40.0));

        // Home on the second wrapped line goes to that line's first offset.
        let (home, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(0, 5))),
            CaretMovement::LineStart,
            false,
            None,
        );
        assert_eq!(home.focus, TextPosition::new(0, 3));
        // End on the first wrapped line stops at the wrap point.
        let (end, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(0, 1))),
            CaretMovement::LineEnd,
            true,
            None,
        );
        assert_eq!(end.anchor, TextPosition::new(0, 1));
        assert_eq!(end.focus, TextPosition::new(0, 3));
        // End on the last line goes to the paragraph's end of text.
        let (last_end, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(0, 4))),
            CaretMovement::LineEnd,
            false,
            None,
        );
        assert_eq!(last_end.focus, TextPosition::new(0, 6));
        // Ctrl+Home / Ctrl+End reach the document's ends.
        let (doc_start, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(1, 1))),
            CaretMovement::DocumentStart,
            false,
            None,
        );
        assert_eq!(doc_start.focus, TextPosition::new(0, 0));
        let (doc_end, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
            CaretMovement::DocumentEnd,
            false,
            None,
        );
        assert_eq!(doc_end.focus, TextPosition::new(1, 2));
    }

    #[test]
    fn navigate_paragraph_start_collapses_focus_to_offset_zero() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 3));
        let (selection, goal) =
            manager.navigate(Some(collapsed), CaretMovement::ParagraphStart, false, None);
        assert_eq!(selection.anchor, TextPosition::new(0, 0));
        assert_eq!(selection.focus, TextPosition::new(0, 0));
        assert!(goal.is_none());
    }

    fn three_paragraph_manager() -> SelectionManager {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for (index, text) in ["ab", "cde", "fg"].into_iter().enumerate() {
            manager.append_paragraph_text(index, text);
            manager.record_line(index, make_line(text, 0.0, index as f64 * 20.0, 0.0));
        }
        manager
    }

    /// Word for Mac's Cmd+Up: to the start of the current paragraph, or of
    /// the previous one when the caret already sits at a paragraph start.
    #[test]
    fn navigate_previous_paragraph_matches_word_for_mac() {
        let mut manager = three_paragraph_manager();
        let mut step = |from: TextPosition| {
            manager
                .navigate(
                    Some(SelectionRange::collapsed(from)),
                    CaretMovement::ParagraphPrevious,
                    false,
                    None,
                )
                .0
                .focus
        };
        assert_eq!(step(TextPosition::new(1, 2)), TextPosition::new(1, 0));
        assert_eq!(step(TextPosition::new(1, 0)), TextPosition::new(0, 0));
        assert_eq!(step(TextPosition::new(0, 0)), TextPosition::new(0, 0));
    }

    /// Word for Mac's Cmd+Down: to the start of the next paragraph; the last
    /// paragraph goes to its end.
    #[test]
    fn navigate_next_paragraph_matches_word_for_mac() {
        let mut manager = three_paragraph_manager();
        let (selection, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(0, 1))),
            CaretMovement::ParagraphNext,
            true,
            None,
        );
        assert_eq!(selection.anchor, TextPosition::new(0, 1));
        assert_eq!(selection.focus, TextPosition::new(1, 0));
        let (last, _) = manager.navigate(
            Some(SelectionRange::collapsed(TextPosition::new(2, 0))),
            CaretMovement::ParagraphNext,
            false,
            None,
        );
        assert_eq!(last.focus, TextPosition::new(2, 2));
    }

    #[test]
    fn navigate_paragraph_end_extends_selection_to_text_length() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 1));
        let (selection, _) =
            manager.navigate(Some(collapsed), CaretMovement::ParagraphEnd, true, None);
        assert_eq!(selection.anchor, TextPosition::new(0, 1));
        assert_eq!(selection.focus, TextPosition::new(0, "abcd".len()));
    }

    #[test]
    fn navigate_extends_selection_right() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 1));
        let (selection, goal) = manager.navigate(Some(collapsed), CaretMovement::Right, true, None);
        assert_eq!(selection.anchor, TextPosition::new(0, 1));
        assert_eq!(selection.focus, TextPosition::new(0, 2));
        assert!(goal.is_none());
    }

    #[test]
    fn navigate_extends_selection_right_multiple_steps() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let mut range = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));

        let (first, _) = manager.navigate(range, CaretMovement::Right, true, None);
        assert_eq!(first.anchor, TextPosition::new(0, 0));
        assert_eq!(first.focus, TextPosition::new(0, 1));

        range = Some(first);
        let (second, _) = manager.navigate(range, CaretMovement::Right, true, None);
        assert_eq!(second.anchor, TextPosition::new(0, 0));
        assert_eq!(second.focus, TextPosition::new(0, 2));
    }

    #[test]
    fn navigate_extends_selection_left_multiple_steps() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let mut range = Some(SelectionRange::collapsed(TextPosition::new(
            0,
            "abcd".len(),
        )));

        let (first, _) = manager.navigate(range, CaretMovement::Left, true, None);
        assert_eq!(first.anchor, TextPosition::new(0, "abcd".len()));
        assert_eq!(first.focus, TextPosition::new(0, "abc".len()));

        range = Some(first);
        let (second, _) = manager.navigate(range, CaretMovement::Left, true, None);
        assert_eq!(second.anchor, TextPosition::new(0, "abcd".len()));
        assert_eq!(second.focus, TextPosition::new(0, "ab".len()));
    }

    #[test]
    fn navigate_extends_selection_respects_grapheme_clusters() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        let text = "a😊b";
        manager.append_paragraph_text(0, text);
        manager.record_line(0, make_line(text, 0.0, 0.0, 0.0));

        let mut range = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));

        let (first, _) = manager.navigate(range, CaretMovement::Right, true, None);
        assert_eq!(first.anchor, TextPosition::new(0, 0));
        assert_eq!(first.focus, TextPosition::new(0, "a".len()));

        range = Some(first);
        let (second, _) = manager.navigate(range, CaretMovement::Right, true, None);
        assert_eq!(second.anchor, TextPosition::new(0, 0));
        assert_eq!(second.focus, TextPosition::new(0, "a😊".len()));
    }

    #[test]
    fn navigate_extends_selection_left_and_shrinks() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        let range = SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(0, 3));
        let (selection, goal) = manager.navigate(Some(range), CaretMovement::Left, true, None);
        assert_eq!(selection.anchor, TextPosition::new(0, 1));
        assert_eq!(selection.focus, TextPosition::new(0, 2));
        assert!(goal.is_none());
    }

    #[test]
    fn navigate_horizontal_without_extend_collapses_to_selection_edge() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcd");
        manager.record_line(0, make_line("abcd", 0.0, 0.0, 0.0));

        for range in [
            SelectionRange::new(TextPosition::new(0, 1), TextPosition::new(0, 3)),
            SelectionRange::new(TextPosition::new(0, 3), TextPosition::new(0, 1)),
        ] {
            let (left, left_goal) = manager.navigate(Some(range), CaretMovement::Left, false, None);
            assert_eq!(left, SelectionRange::collapsed(TextPosition::new(0, 1)));
            assert!(left_goal.is_none());

            let (right, right_goal) =
                manager.navigate(Some(range), CaretMovement::Right, false, None);
            assert_eq!(right, SelectionRange::collapsed(TextPosition::new(0, 3)));
            assert!(right_goal.is_none());
        }
    }

    #[test]
    fn navigate_extends_selection_down_across_lines() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef");
        manager.record_line(0, make_line_with_offset(0, "abc", 0.0, 0.0, 0.0));
        manager.record_line(0, make_line_with_offset(3, "def", 0.0, 20.0, 20.0));

        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 1));
        let expected_goal = manager
            .caret_x_for_position(TextPosition::new(0, 1))
            .unwrap();
        let (selection, goal) = manager.navigate(Some(collapsed), CaretMovement::Down, true, None);
        assert_eq!(selection.anchor, TextPosition::new(0, 1));
        assert_eq!(selection.focus, TextPosition::new(0, 4));
        match goal {
            Some(value) => assert!(
                (value - expected_goal).abs() < f64::EPSILON,
                "expected goal {expected_goal}, got {value}"
            ),
            None => panic!("expected goal value"),
        }
    }

    #[test]
    fn vertical_navigation_between_table_cells_stays_in_the_same_column() {
        let mut manager = SelectionManager::new();
        manager.begin_render();

        for row in 0..3 {
            for column in 0..3 {
                let paragraph = row * 3 + column;
                let x = column as f64 * 100.0;
                let y = row as f64 * 20.0;
                manager.append_paragraph_text(paragraph, "x");
                manager.record_line(paragraph, make_line("x", x, y, y));
                manager.record_table_cell(CellLayout {
                    paragraphs: vec![paragraph],
                    left: x,
                    right: x + 100.0,
                    top: y,
                    bottom: y + 20.0,
                    table_id: 0,
                    row,
                    column,
                    column_span: 1,
                    order: paragraph,
                });
            }
        }

        let (up, _) = manager.move_up(TextPosition::new(6, 1), None);
        assert_eq!(up, TextPosition::new(3, 1));

        let (down, _) = manager.move_down(TextPosition::new(0, 1), None);
        assert_eq!(down, TextPosition::new(3, 1));
    }

    #[test]
    fn hit_test_prefers_line_with_closest_absolute_top() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "page1");
        manager.record_line(0, make_line("page1", 0.0, 100.0, 100.0));
        manager.append_paragraph_text(1, "page2");
        manager.record_line(1, make_line("page2", 0.0, 100.0, 1500.0));

        let pos = manager.hit_test(10.0, 120.0).unwrap();
        assert_eq!(pos.paragraph, 0);

        let pos_page2 = manager.hit_test(10.0, 1600.0).unwrap();
        assert_eq!(pos_page2.paragraph, 1);
    }

    #[test]
    fn hit_test_prefers_later_paragraph_when_overlap_is_identical() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "same");
        manager.record_line(0, make_line("same", 0.0, 100.0, 100.0));
        manager.append_paragraph_text(1, "same");
        manager.record_line(1, make_line("same", 0.0, 100.0, 100.0));

        let pos = manager.hit_test(10.0, 110.0).unwrap();
        assert_eq!(pos.paragraph, 1);
    }

    #[test]
    fn non_selectable_paragraphs_are_skipped_by_hit_test_and_clamp() {
        let mut manager = SelectionManager::new();
        manager.append_paragraph_text(0, "Body");
        manager.record_line(0, make_line("Body", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "Endnote");
        manager.record_line(1, make_line("Endnote", 0.0, 100.0, 100.0));
        manager.set_paragraph_selectable(1, false);

        let hit = manager.hit_test(5.0, 110.0).expect("hit position");
        assert_eq!(hit.paragraph, 0);

        let clamped = manager.clamp_position(TextPosition::new(1, 4));
        assert_eq!(clamped.paragraph, 0);
    }

    /// Regression: clicking inside any cell of a wide table row (>4 columns
    /// when the document has >10 paragraphs) must focus the paragraph in the
    /// cell that geometrically contains the click — not snap back to one of
    /// the leftmost cells. Previously `find_candidate_paragraphs_for_y` capped
    /// the candidate set at MAX_CANDIDATES=5 *before* checking which
    /// paragraphs actually contained the click Y, so a row of 8 same-Y cells
    /// only ever surfaced the first 4 (after a backward body candidate),
    /// collapsing clicks on cells 5-8 to cell 4. See CLAUDE.md changelog.
    #[test]
    fn hit_test_resolves_each_cell_in_wide_table_row() {
        let mut manager = SelectionManager::new();
        manager.begin_render();

        // Body paragraph just above the table — placed close enough to the
        // first row that the backward-candidate scan picks it up (this is the
        // realistic layout that triggered the regression).
        manager.append_paragraph_text(0, "body");
        manager.record_line(0, make_line("body", 0.0, 0.0, 0.0));

        const ROW_TOP_R0: f64 = 30.0;
        const ROW_TOP_R1: f64 = 60.0;
        const CELL_WIDTH: f64 = 50.0;
        const COLUMNS: usize = 8;

        // Two rows × 8 cells = 16 cell paragraphs (+1 body = 17 total),
        // exceeding the `paragraphs.len() <= 10` shortcut so the bounded-
        // candidate path is exercised.
        for row in 0..2usize {
            let row_top = if row == 0 { ROW_TOP_R0 } else { ROW_TOP_R1 };
            for col in 0..COLUMNS {
                let paragraph_index = 1 + row * COLUMNS + col;
                let cell_x = (col as f64) * CELL_WIDTH;
                manager.ensure_paragraph(paragraph_index);
                manager.record_line(
                    paragraph_index,
                    LineLayout {
                        column_index: 0,
                        top: row_top,
                        absolute_top: row_top,
                        height: 20.0,
                        baseline: row_top + 15.0,
                        start_x: cell_x,
                        cursor_end_x: cell_x + CELL_WIDTH,
                        allowed_trailing_overflow_width: 0.0,
                        content_top: row_top,
                        absolute_content_top: row_top,
                        content_height: 20.0,
                        fragments: Rc::from([]),
                        break_range: Some((0, 0)),
                        tab_leaders: Rc::from([]),
                        tab_stops: Rc::from([]),
                        vertical: false,
                        vertical_reversed: false,
                    },
                );
            }
        }

        for col in 0..COLUMNS {
            let click_x = (col as f64) * CELL_WIDTH + CELL_WIDTH / 2.0;
            let click_y = ROW_TOP_R0 + 10.0;
            let pos = manager
                .hit_test(click_x, click_y)
                .unwrap_or_else(|| panic!("hit_test returned None for col {col}"));
            let expected = 1 + col;
            assert_eq!(
                pos.paragraph, expected,
                "row 0 col {col} click @ x={click_x} expected paragraph {expected}, got {}",
                pos.paragraph
            );
        }

        for col in 0..COLUMNS {
            let click_x = (col as f64) * CELL_WIDTH + CELL_WIDTH / 2.0;
            let click_y = ROW_TOP_R1 + 10.0;
            let pos = manager
                .hit_test(click_x, click_y)
                .unwrap_or_else(|| panic!("hit_test returned None for row 1 col {col}"));
            let expected = 1 + COLUMNS + col;
            assert_eq!(
                pos.paragraph, expected,
                "row 1 col {col} click @ x={click_x} expected paragraph {expected}, got {}",
                pos.paragraph
            );
        }
    }

    /// Tests hit_test on a multi-page document (6 pages) to ensure correct
    /// paragraph detection when clicking on later pages.
    #[test]
    fn hit_test_multipage_document_later_pages() {
        // Simulate a 6-page document with PAGE_HEIGHT = 800.0 and PAGE_GAP = 16.0
        // Each page starts at: 0, 816, 1632, 2448, 3264, 4080
        const PAGE_HEIGHT: f64 = 800.0;
        const PAGE_GAP: f64 = 16.0;

        let mut manager = SelectionManager::new();
        manager.begin_render();

        // Create paragraphs for each page
        for page in 0..6 {
            let paragraph_index = page;
            let text = format!("page{}", page + 1);
            manager.append_paragraph_text(paragraph_index, &text);

            let page_start_y = (page as f64) * (PAGE_HEIGHT + PAGE_GAP);
            // Place line at 100px from top of page
            let absolute_top = page_start_y + 100.0;
            manager.record_line(paragraph_index, make_line(&text, 50.0, 100.0, absolute_top));
        }

        // Click on page 1 (first page)
        let pos_page1 = manager.hit_test(60.0, 110.0).unwrap();
        assert_eq!(pos_page1.paragraph, 0, "Should hit paragraph 0 on page 1");

        // Click on page 4 (index 3), y = 3 * 816 + 110 = 2558
        let page4_y = 3.0 * (PAGE_HEIGHT + PAGE_GAP) + 110.0;
        let pos_page4 = manager.hit_test(60.0, page4_y).unwrap();
        assert_eq!(pos_page4.paragraph, 3, "Should hit paragraph 3 on page 4");

        // Click on page 5 (index 4), y = 4 * 816 + 110 = 3374
        let page5_y = 4.0 * (PAGE_HEIGHT + PAGE_GAP) + 110.0;
        let pos_page5 = manager.hit_test(60.0, page5_y).unwrap();
        assert_eq!(pos_page5.paragraph, 4, "Should hit paragraph 4 on page 5");

        // Click on page 6 (index 5), y = 5 * 816 + 110 = 4190
        let page6_y = 5.0 * (PAGE_HEIGHT + PAGE_GAP) + 110.0;
        let pos_page6 = manager.hit_test(60.0, page6_y).unwrap();
        assert_eq!(pos_page6.paragraph, 5, "Should hit paragraph 5 on page 6");
    }

    /// Tests that hit_test returns correct offset within a line on a later page.
    #[test]
    fn hit_test_returns_correct_offset_on_later_page() {
        const PAGE_HEIGHT: f64 = 800.0;
        const PAGE_GAP: f64 = 16.0;

        let mut manager = SelectionManager::new();
        manager.begin_render();

        // Add paragraph on page 4 (index 3)
        manager.append_paragraph_text(3, "hello");
        let page4_start = 3.0 * (PAGE_HEIGHT + PAGE_GAP);
        let absolute_top = page4_start + 100.0;
        // Line starts at x=50, each glyph is 10px wide
        manager.record_line(3, make_line("hello", 50.0, 100.0, absolute_top));

        // Click at x=50 should give offset 0 (start of 'h')
        let pos_start = manager.hit_test(50.0, absolute_top + 10.0).unwrap();
        assert_eq!(pos_start.paragraph, 3);
        assert_eq!(pos_start.offset, 0);

        // Click at x=75 should give offset 3 (end of 'l' at position 2)
        // With glyph width 10, x=75 is in 3rd glyph (50+20=70 to 50+30=80)
        // Since 75 is at midpoint (25 relative), it returns glyph.end = 3
        let pos_middle = manager.hit_test(75.0, absolute_top + 10.0).unwrap();
        assert_eq!(pos_middle.paragraph, 3);
        assert_eq!(pos_middle.offset, 3); // at midpoint, returns end of glyph

        // Click at x=100 should give offset 5 (end of 'hello')
        let pos_end = manager.hit_test(100.0, absolute_top + 10.0).unwrap();
        assert_eq!(pos_end.paragraph, 3);
        assert_eq!(pos_end.offset, 5);
    }

    #[test]
    fn hit_test_before_page_break_stays_on_clicked_page() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, &format!("Hello{PAGE_BREAK_STR}"));
        let mut line = make_line_with_offset(0, "Hello", 0.0, 0.0, 100.0);
        line.break_range = Some((5, 6));
        manager.record_line(0, line);
        let placeholder = LineLayout {
            column_index: 0,
            absolute_top: 1200.0,
            height: 20.0,
            break_range: Some((6, 6)),
            ..Default::default()
        };
        manager.record_line(0, placeholder);
        assert_eq!(
            manager.hit_test(100.0, 110.0),
            Some(TextPosition::new(0, 5))
        );
        assert_eq!(
            manager.hit_test(100.0, 1210.0),
            Some(TextPosition::new(0, 6))
        );
    }

    #[test]
    fn selected_lines_skip_page_break_placeholder_when_outside_range() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, &format!("Hello{PAGE_BREAK_STR}"));

        let mut line = make_line_with_offset(0, "Hello", 0.0, 0.0, 100.0);
        line.break_range = Some((5, 6));
        manager.record_line(0, line);

        let placeholder = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 1200.0,
            height: 20.0,
            baseline: 20.0,
            start_x: 0.0,
            cursor_end_x: 0.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 1200.0,
            content_height: 20.0,
            fragments: Rc::from([]),
            break_range: Some((6, 6)),
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };
        manager.record_line(0, placeholder);

        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        )));

        let lines = manager.selected_lines();
        assert_eq!(lines.len(), 1);
        assert!((lines[0].absolute_top - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn selected_lines_include_page_break_placeholder_when_range_extends_to_end() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, &format!("Hello{PAGE_BREAK_STR}"));

        let mut line = make_line_with_offset(0, "Hello", 0.0, 0.0, 100.0);
        line.break_range = Some((5, 6));
        manager.record_line(0, line);

        let placeholder = LineLayout {
            column_index: 0,
            top: 0.0,
            absolute_top: 1200.0,
            height: 20.0,
            baseline: 20.0,
            start_x: 0.0,
            cursor_end_x: 0.0,
            allowed_trailing_overflow_width: 0.0,
            content_top: 0.0,
            absolute_content_top: 1200.0,
            content_height: 20.0,
            fragments: Rc::from([]),
            break_range: Some((6, 6)),
            tab_leaders: Rc::from([]),
            tab_stops: Rc::from([]),
            vertical: false,
            vertical_reversed: false,
        };
        manager.record_line(0, placeholder);

        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 6),
        )));

        let lines = manager.selected_lines();
        assert_eq!(lines.len(), 2);
        assert!(
            lines
                .iter()
                .any(|line| (line.absolute_top - 1200.0).abs() < f64::EPSILON),
            "expected placeholder line to be included"
        );
    }

    #[test]
    fn selected_lines_preserve_tab_leader_segments() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "A\tB");

        let mut line = make_line("AB", 0.0, 0.0, 0.0);
        line.start_x = 0.0;
        line.cursor_end_x = 200.0;
        line.tab_leaders = vec![TabLeaderLayout {
            start_x: 40.0,
            end_x: 160.0,
            leader: "dot".to_string(),
        }]
        .into();
        manager.record_line(0, line);

        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 2),
        )));

        let lines = manager.selected_lines();
        assert_eq!(lines.len(), 1);
        let leaders = &lines[0].tab_leaders;
        assert_eq!(leaders.len(), 1);
        assert!((leaders[0].start_x - 40.0).abs() < f64::EPSILON);
        assert!((leaders[0].end_x - 160.0).abs() < f64::EPSILON);
        assert_eq!(leaders[0].leader, "dot");
    }

    #[test]
    fn selected_lines_report_visual_end_x() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "hello");

        let mut line = make_line("hello", 8.0, 0.0, 0.0);
        let text_end = line
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(8.0);
        line.cursor_end_x = text_end - 4.0;
        manager.record_line(0, line);

        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, "hello".len()),
        )));

        let lines = manager.selected_lines();
        assert_eq!(lines.len(), 1);
        assert!((lines[0].visual_end_x - text_end).abs() < f64::EPSILON);
        assert!((lines[0].end_x - text_end).abs() < f64::EPSILON);
        assert!((lines[0].cursor_end_x - (text_end - 4.0)).abs() < f64::EPSILON);
    }

    /// A paragraph kept across an incremental pass and then registered again
    /// (cached replay, a re-laid-out table, a footnote drawn at the page end)
    /// is rebuilt rather than appended to.
    #[test]
    fn re_registering_a_preserved_paragraph_replaces_its_text_and_lines() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        manager.record_line(0, make_line("Hello", 8.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "World");
        manager.record_line(1, make_line("World", 8.0, 20.0, 20.0));

        // Edit paragraph 1: paragraph 0 is preserved, then replayed.
        manager.begin_render_incremental(Some(1));
        assert_eq!(manager.preserved_prefix_len(), 1);
        manager.append_paragraph_text_and_lines_with_y_offset(
            0,
            "Hello",
            &[make_line("Hello", 8.0, 0.0, 0.0)],
            0.0,
        );
        manager.append_paragraph_text(1, "World!");
        manager.record_line(1, make_line("World!", 8.0, 20.0, 20.0));

        assert_eq!(manager.paragraph_debug_texts(), vec!["Hello", "World!"]);
        assert_eq!(manager.paragraphs[0].lines.len(), 1);
        assert_eq!(manager.paragraphs[1].lines.len(), 1);

        // Within one pass, registration still accumulates (a paragraph split
        // across pages records its lines page by page).
        manager.record_line(0, make_line("Hello", 8.0, 40.0, 40.0));
        assert_eq!(manager.paragraphs[0].lines.len(), 2);

        // A full pass starts every paragraph from scratch.
        manager.begin_render();
        assert_eq!(manager.preserved_prefix_len(), 0);
        manager.append_paragraph_text(0, "Hello");
        assert_eq!(manager.paragraph_debug_texts(), vec!["Hello"]);
    }

    #[test]
    fn stable_edit_suffix_replaces_overlapping_nested_story_layouts() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        for (index, text) in ["before", "unchanged", "tail"].iter().enumerate() {
            manager.append_paragraph_text(index, text);
        }

        let suffix = manager.take_paragraph_suffix(1);
        manager.begin_render_incremental(Some(0));
        manager.append_paragraph_text(0, "edited");
        // A nested story may register part of the unchanged tail before the
        // body render takes its stable-edit exit.
        manager.append_paragraph_text(1, "partial replay");
        manager.restore_paragraph_suffix(1, suffix);

        assert_eq!(
            manager.paragraph_debug_texts(),
            vec!["edited", "unchanged", "tail"]
        );
    }

    #[test]
    fn resetting_same_pass_paragraph_registration_replaces_text_and_lines() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Footnote");
        manager.record_line(0, make_line("Footnote", 8.0, 20.0, 20.0));

        manager.reset_paragraph_registration(0);
        manager.append_paragraph_text(0, "Footnote");
        manager.record_line(0, make_line("Footnote", 8.0, 20.0, 20.0));

        assert_eq!(manager.paragraph_debug_texts(), vec!["Footnote"]);
        assert_eq!(manager.paragraphs[0].lines.len(), 1);
    }

    #[test]
    fn continued_table_paragraph_retains_head_lines_and_replaces_a_revisited_tail() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "abcdef10");
        manager.record_line(0, make_line("abc", 8.0, 0.0, 0.0));
        let mut tail = make_line("def1", 8.0, 100.0, 100.0);
        for fragment in Rc::make_mut(&mut tail.fragments) {
            fragment.start += 3;
            fragment.end += 3;
            for glyph in Rc::make_mut(&mut fragment.glyphs) {
                glyph.start += 3;
                glyph.end += 3;
            }
        }
        for _ in 0..2 {
            manager.continue_paragraph_registration(0, "abcdef1", 3);
            manager.record_line(0, tail.clone());
        }
        assert_eq!(manager.paragraph_debug_texts(), vec!["abcdef1"]);
        assert_eq!(manager.paragraphs[0].lines.len(), 2);
        assert_eq!(manager.hit_test(4.0, 10.0).unwrap().paragraph, 0);
        assert_eq!(manager.hit_test(4.0, 110.0).unwrap().paragraph, 0);
    }

    #[test]
    fn rendered_cell_fragments_keep_one_order_when_physical_orders_advance() {
        let cell = |column, top, order| CellLayout {
            paragraphs: vec![column],
            left: column as f64 * 100.0,
            right: column as f64 * 100.0 + 100.0,
            top,
            bottom: top + 20.0,
            table_id: 0,
            row: 0,
            column,
            column_span: 1,
            order,
        };
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "left");
        manager.append_paragraph_text(1, "right");
        manager.append_paragraph_text(2, "following");
        manager.record_table_cell_fragment(cell(0, 0.0, 0));
        manager.record_table_cell_fragment(cell(1, 0.0, 1));
        manager.record_table_cell_fragment(cell(0, 100.0, 2));
        manager.record_table_cell_fragment(cell(1, 100.0, 3));
        assert_eq!(
            manager
                .cells
                .iter()
                .map(|cell| cell.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 0, 1]
        );
        assert_eq!(manager.next_cell_in_same_table(0), Some(1));
        assert_eq!(manager.next_cell_in_same_table(2), Some(1));
        manager.set_selection(manager.cell_selection_range(0, 0));
        manager.set_cell_selection(0, 0);
        let (range, _) =
            manager.navigate(manager.selection_range(), CaretMovement::Right, true, None);
        manager.set_selection(Some(range));
        assert!(!range.is_collapsed());
        assert_eq!(manager.selected_logical_cell_layouts().unwrap().len(), 2);
        assert_eq!(manager.selected_cell_layouts().unwrap().len(), 4);
        manager.record_table_cell_fragment(cell(0, 100.0, 4));
        assert_eq!(manager.cells.len(), 4);
        assert_eq!(manager.cells[2].order, 0);
        let mut unrelated = cell(0, 200.0, 9);
        unrelated.table_id = 1;
        manager.record_table_cell_fragment(unrelated);
        assert_eq!(manager.cells[4].order, 9);
        assert_eq!(manager.next_cell_in_same_table(2), Some(1));
    }

    #[test]
    fn continued_cell_keeps_both_page_rects_and_one_navigation_order() {
        let cell = |column, top| CellLayout {
            paragraphs: vec![column],
            left: column as f64 * 100.0,
            right: column as f64 * 100.0 + 100.0,
            top,
            bottom: top + 20.0,
            table_id: 0,
            row: 0,
            column,
            column_span: 1,
            order: column,
        };
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "left");
        manager.append_paragraph_text(1, "right");
        manager.record_table_cell(cell(0, 0.0));
        manager.record_table_cell(cell(1, 0.0));
        manager.record_table_cell_continuation(cell(0, 100.0));
        manager.record_table_cell_continuation(cell(0, 100.0));
        assert_eq!(manager.cells.len(), 3);
        assert_eq!(manager.cell_at_point(50.0, 10.0), Some(0));
        assert_eq!(manager.cell_at_point(50.0, 110.0), Some(2));
        assert_eq!(manager.cell_id_for_paragraph(0), Some(0));
        assert_eq!(manager.next_cell_in_same_table(0), Some(1));
        assert_eq!(manager.next_cell_in_same_table(2), Some(1));
        assert_eq!(manager.previous_cell_in_same_table(1), Some(2));
        manager.set_cell_selection(2, 2);
        assert_eq!(manager.selected_cell_layouts().unwrap().len(), 2);
        assert_eq!(manager.selected_logical_cell_layouts().unwrap().len(), 1);
        manager.set_cell_selection(2, 1);
        assert_eq!(manager.selected_cell_layouts().unwrap().len(), 3);
        assert_eq!(manager.selected_logical_cell_layouts().unwrap().len(), 2);
        assert_eq!(manager.cell_selection_span(), Some((1, 2)));
        assert!(manager
            .navigate_cell_selection_forward(TextPosition::new(0, 0), TextPosition::new(1, 5))
            .is_some());
        assert!(manager
            .navigate_cell_selection_backward(TextPosition::new(0, 0), TextPosition::new(1, 5))
            .is_some());
        assert!(manager
            .navigate_cell_selection_backward(TextPosition::new(0, 0), TextPosition::new(1, 0))
            .is_some());
        assert!(manager.cell_selection.is_none());
        manager.clear_cell_selection();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 5));
        manager.set_selection(Some(range));
        assert_eq!(manager.selection_text().as_deref(), Some("left\nright"));
        assert_eq!(
            manager.infer_selected_cells_from_text(&range),
            Some(vec![0, 1])
        );
    }

    /// A table row laid out again in the same render pass (lazy-page
    /// re-entry) records its cells again; they replace the earlier records
    /// instead of leaving twins behind for cell selection to walk (#294).
    #[test]
    fn continued_cell_unifies_paragraphs_assigned_lazily_on_different_pages() {
        let cell = |paragraph, top| CellLayout {
            paragraphs: vec![paragraph],
            left: 0.0,
            right: 100.0,
            top,
            bottom: top + 20.0,
            table_id: 0,
            row: 0,
            column: 0,
            column_span: 1,
            order: paragraph,
        };
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "head");
        manager.append_paragraph_text(1, "tail");
        manager.record_table_cell(cell(0, 0.0));
        manager.record_table_cell_continuation(cell(1, 100.0));
        assert_eq!(manager.cell_id_for_paragraph(1), Some(0));
        assert_eq!(manager.cells[0].paragraphs, vec![0, 1]);
        assert_eq!(manager.cells[1].paragraphs, vec![0, 1]);
        assert_eq!(manager.cells[1].order, 0);
        manager.set_cell_selection(1, 1);
        assert_eq!(manager.selected_cell_layouts().unwrap().len(), 2);
    }

    #[test]
    fn recording_the_same_cell_twice_in_one_pass_replaces_it() {
        let cell = |paragraphs: Vec<usize>, top: f64, column: usize| CellLayout {
            paragraphs,
            left: column as f64 * 100.0,
            right: column as f64 * 100.0 + 100.0,
            top,
            bottom: top + 20.0,
            table_id: 0,
            row: 0,
            column,
            column_span: 1,
            order: column,
        };
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.record_table_cell(cell(vec![0], 0.0, 0));
        manager.record_table_cell(cell(vec![1], 0.0, 1));
        manager.record_table_cell(cell(vec![0], 40.0, 0));
        manager.record_table_cell(cell(vec![1], 40.0, 1));

        assert_eq!(manager.cells.len(), 2);
        assert_eq!(manager.cell_id_for_paragraph(0), Some(0));
        assert_eq!(manager.cell_id_for_paragraph(1), Some(1));
        assert_eq!(manager.cell_layout(0).map(|cell| cell.top), Some(40.0));

        // A different paragraph set (a split row's tail) is still its own cell.
        manager.record_table_cell(cell(vec![0, 2], 80.0, 0));
        assert_eq!(manager.cells.len(), 3);
    }

    #[test]
    fn cell_fragments_with_identical_paragraphs_keep_each_page_bounds() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        let cell = |top| CellLayout {
            paragraphs: vec![0],
            left: 0.0,
            right: 100.0,
            top,
            bottom: top + 20.0,
            table_id: 0,
            row: 0,
            column: 0,
            column_span: 1,
            order: 0,
        };
        manager.record_table_cell_fragment(cell(0.0));
        manager.record_table_cell_fragment(cell(1000.0));
        manager.record_table_cell_fragment(cell(1000.0));
        assert_eq!(manager.cells.len(), 2);
        assert_eq!(manager.cells[0].top, 0.0);
        assert_eq!(manager.cells[1].top, 1000.0);
        assert_eq!(manager.cell_id_for_paragraph(0), Some(1));
    }

    #[test]
    fn paragraph_bounding_box_uses_visual_end_x() {
        let mut paragraph = ParagraphLayout {
            text: "hello".to_string(),
            lines: vec![make_line("hello", 8.0, 0.0, 0.0)],
            bounding_box: Cell::new(None),
            render_pass: 0,
        };
        let text_end = paragraph.lines[0]
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(8.0);
        paragraph.lines[0].cursor_end_x = text_end - 4.0;

        let bbox = paragraph.bounding_box().expect("bounding box");
        assert!((bbox.3 - text_end).abs() < f64::EPSILON);
    }

    #[test]
    fn hit_test_uses_visual_extent_for_fragment_lines() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "hello");

        let mut line = make_line("hello", 8.0, 0.0, 0.0);
        let text_end = line
            .fragments
            .last()
            .map(|fragment| fragment.x + fragment.width)
            .unwrap_or(8.0);
        line.cursor_end_x = text_end - 4.0;
        manager.record_line(0, line);

        let pos = manager
            .hit_test(text_end - 1.0, 10.0)
            .expect("hit position");
        assert_eq!(pos.paragraph, 0);
        assert_eq!(pos.offset, "hello".len());
    }

    #[test]
    fn navigate_extends_selection_across_cells() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        manager.record_line(0, make_line("Hello", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "World");
        manager.record_line(1, make_line("World", 0.0, 0.0, 20.0));

        // Setup cells
        // Cell 1: Para 0
        manager.record_table_cell(CellLayout {
            paragraphs: vec![0],
            left: 0.0,
            right: 100.0,
            top: 0.0,
            bottom: 20.0,
            table_id: 0,
            row: 0,
            column: 0,
            column_span: 1,
            order: 0,
        });
        // Cell 2: Para 1
        manager.record_table_cell(CellLayout {
            paragraphs: vec![1],
            left: 100.0,
            right: 200.0,
            top: 0.0,
            bottom: 20.0,
            table_id: 0,
            row: 0,
            column: 1,
            column_span: 1,
            order: 1,
        });

        // Start at end of Cell 1
        let start_pos = TextPosition::new(0, 5); // End of "Hello"
        let mut range = Some(SelectionRange::collapsed(start_pos));

        // 1. Shift+Right -> Select Cell 1
        let (first, _) = manager.navigate(range, CaretMovement::Right, true, None);
        // Expectation: [0:0, 1:0] (Start of Cell 1 to Start of Cell 2)
        assert_eq!(first.anchor, TextPosition::new(0, 0));
        assert_eq!(first.focus, TextPosition::new(1, 0));
        assert!(manager.cell_selection.is_some());

        // 2. Shift+Right -> Select Cell 1 and Cell 2
        range = Some(first);
        let (second, _) = manager.navigate(range, CaretMovement::Right, true, None);

        // Expectation: [0:0, 1:5] (Start of Cell 1 to End of Cell 2)
        assert_eq!(second.anchor, TextPosition::new(0, 0));
        assert_eq!(second.focus, TextPosition::new(1, 5));
        assert!(manager.cell_selection.is_some());
    }

    #[test]
    fn navigate_extends_selection_across_cells_with_rerender() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        manager.record_line(0, make_line("Hello", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "World");
        manager.record_line(1, make_line("World", 0.0, 0.0, 20.0));

        // Setup cells helper
        let setup_cells = |mgr: &mut SelectionManager| {
            mgr.record_table_cell(CellLayout {
                paragraphs: vec![0],
                left: 0.0,
                right: 100.0,
                top: 0.0,
                bottom: 20.0,
                table_id: 0,
                row: 0,
                column: 0,
                column_span: 1,
                order: 0,
            });
            mgr.record_table_cell(CellLayout {
                paragraphs: vec![1],
                left: 100.0,
                right: 200.0,
                top: 0.0,
                bottom: 20.0,
                table_id: 0,
                row: 0,
                column: 1,
                column_span: 1,
                order: 1,
            });
        };
        setup_cells(&mut manager);

        // Start at end of Cell 1
        let start_pos = TextPosition::new(0, 5);
        let mut range = Some(SelectionRange::collapsed(start_pos));

        // 1. Shift+Right -> Select Cell 1
        let (first, _) = manager.navigate(range, CaretMovement::Right, true, None);
        manager.set_selection(Some(first));

        // SIMULATE RERENDER
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        manager.record_line(0, make_line("Hello", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "World");
        manager.record_line(1, make_line("World", 0.0, 0.0, 20.0));
        setup_cells(&mut manager);

        // 2. Shift+Right -> Select Cell 1 and Cell 2
        range = Some(first);
        let (second, _) = manager.navigate(range, CaretMovement::Right, true, None);

        assert_eq!(second.anchor, TextPosition::new(0, 0));
        assert_eq!(second.focus, TextPosition::new(1, 5));
        assert!(manager.cell_selection.is_some());
    }

    #[test]
    fn record_table_cell_normalizes_bounds_and_paragraph_indices() {
        let mut manager = SelectionManager::new();
        manager.begin_render();

        manager.record_table_cell(CellLayout {
            paragraphs: vec![2, 0, 2],
            left: 150.0,
            right: 50.0,
            top: 80.0,
            bottom: 20.0,
            table_id: 3,
            row: 1,
            column: 2,
            column_span: 0,
            order: 5,
        });

        let cell = manager.cell_layout(0).unwrap();
        assert_eq!(cell.left, 50.0);
        assert_eq!(cell.right, 150.0);
        assert_eq!(cell.top, 20.0);
        assert_eq!(cell.bottom, 80.0);
        assert_eq!(cell.column_span, 1);
        assert_eq!(cell.paragraphs, vec![0, 2]);
        assert_eq!(manager.cell_id_for_paragraph(0), Some(0));
        assert_eq!(manager.cell_id_for_paragraph(2), Some(0));
    }

    #[test]
    fn set_cell_selection_produces_rectangular_middle_columns() {
        // 3 rows × 4 columns. Dragging from (row 0, col 1) to (row 2, col 2)
        // should select a 3×2 rectangle (the middle two columns), not the
        // full linear range [r0c1 .. r2c2] that would have dragged along
        // the row order and also included r0c2, r0c3, r1c0..r1c3, r2c0, r2c1,
        // r2c2 (i.e. almost the whole table).
        let mut manager = SelectionManager::new();
        manager.begin_render();

        let mut order = 0;
        let mut paragraph_index = 0;
        let mut cell_ids = [[0usize; 4]; 3];
        #[allow(clippy::needless_range_loop)]
        for row in 0..3 {
            #[allow(clippy::needless_range_loop)]
            for col in 0..4 {
                manager.append_paragraph_text(paragraph_index, "x");
                manager.record_line(
                    paragraph_index,
                    make_line("x", 0.0, 0.0, (paragraph_index * 20) as f64),
                );
                manager.record_table_cell(CellLayout {
                    paragraphs: vec![paragraph_index],
                    left: (col * 100) as f64,
                    right: ((col + 1) * 100) as f64,
                    top: (row * 20) as f64,
                    bottom: ((row + 1) * 20) as f64,
                    table_id: 0,
                    row,
                    column: col,
                    column_span: 1,
                    order,
                });
                cell_ids[row][col] = order;
                order += 1;
                paragraph_index += 1;
            }
        }

        // Anchor = (row 0, col 1), focus = (row 2, col 2).
        manager.set_cell_selection(cell_ids[0][1], cell_ids[2][2]);

        let ids = manager.selected_cell_ids().unwrap();
        let mut expected = vec![
            cell_ids[0][1],
            cell_ids[0][2],
            cell_ids[1][1],
            cell_ids[1][2],
            cell_ids[2][1],
            cell_ids[2][2],
        ];
        expected.sort();
        let mut got = ids.clone();
        got.sort();
        assert_eq!(got, expected);

        // Paragraphs in col 0 / col 3 must NOT be considered selected.
        assert!(!manager.paragraph_in_selected_cell(0)); // r0c0
        assert!(!manager.paragraph_in_selected_cell(3)); // r0c3
        assert!(manager.paragraph_in_selected_cell(1)); // r0c1
        assert!(manager.paragraph_in_selected_cell(2)); // r0c2
        assert!(manager.paragraph_in_selected_cell(9)); // r2c1
        assert!(manager.paragraph_in_selected_cell(10)); // r2c2
        assert!(!manager.paragraph_in_selected_cell(8)); // r2c0
        assert!(!manager.paragraph_in_selected_cell(11)); // r2c3
    }

    #[test]
    fn cell_selection_gutter_only_targets_the_leading_edge() {
        let mut manager = SelectionManager::new();
        manager.begin_render();

        for (column, (left, right)) in [(0, (100.0, 200.0)), (1, (200.0, 300.0))] {
            manager.append_paragraph_text(column, "cell");
            manager.record_line(column, make_line("cell", left + 8.0, 0.0, 20.0));
            manager.record_table_cell(CellLayout {
                paragraphs: vec![column],
                left,
                right,
                top: 20.0,
                bottom: 60.0,
                table_id: 0,
                row: 0,
                column,
                column_span: 1,
                order: column,
            });
        }

        assert_eq!(manager.cell_selection_target_at_point(105.0, 40.0), Some(0));
        assert_eq!(manager.cell_selection_target_at_point(205.0, 40.0), Some(1));
        assert_eq!(manager.cell_selection_target_at_point(120.0, 40.0), None);
        assert_eq!(manager.cell_selection_target_at_point(105.0, 70.0), None);
    }

    #[test]
    fn nearest_cell_in_table_snaps_to_bottom_row_when_dragging_below_table() {
        // 2 cols × 3 rows. Simulate the user dragging from row 0 col 0
        // straight down *past* the bottom row. `cell_at_point` returns
        // `None` (point is below all cells) so `pointer_move` would
        // previously fall back to a linear text highlight. The nearest-cell
        // fallback should land on row 2 col 0 (same column, closest row).
        let mut manager = SelectionManager::new();
        manager.begin_render();

        let mut order = 0;
        let mut paragraph_index = 0;
        let mut cell_ids = [[0usize; 2]; 3];
        #[allow(clippy::needless_range_loop)]
        for row in 0..3 {
            #[allow(clippy::needless_range_loop)]
            for col in 0..2 {
                manager.append_paragraph_text(paragraph_index, "x");
                manager.record_line(
                    paragraph_index,
                    make_line("x", 0.0, 0.0, (paragraph_index * 20) as f64),
                );
                manager.record_table_cell(CellLayout {
                    paragraphs: vec![paragraph_index],
                    left: (col * 100) as f64,
                    right: ((col + 1) * 100) as f64,
                    top: (row * 20) as f64,
                    bottom: ((row + 1) * 20) as f64,
                    table_id: 0,
                    row,
                    column: col,
                    column_span: 1,
                    order,
                });
                cell_ids[row][col] = order;
                order += 1;
                paragraph_index += 1;
            }
        }

        // Point below the table, aligned with column 0.
        let nearest = manager.nearest_cell_in_table(0, 30.0, 500.0);
        assert_eq!(nearest, Some(cell_ids[2][0]));

        // Point below the table, aligned with column 1.
        let nearest = manager.nearest_cell_in_table(0, 180.0, 500.0);
        assert_eq!(nearest, Some(cell_ids[2][1]));

        // Point above the table, aligned with column 0 — snap to row 0.
        let nearest = manager.nearest_cell_in_table(0, 30.0, -100.0);
        assert_eq!(nearest, Some(cell_ids[0][0]));

        // Point inside a cell — return that cell.
        let nearest = manager.nearest_cell_in_table(0, 30.0, 30.0);
        assert_eq!(nearest, Some(cell_ids[1][0]));

        // Different table id — no cells match.
        assert_eq!(manager.nearest_cell_in_table(99, 30.0, 30.0), None);
    }

    #[test]
    fn nearest_cell_fallback_yields_full_column_selection_when_drag_exits_below() {
        // End-to-end at the SelectionManager level: anchor in row 0 col 0,
        // fallback focus from `nearest_cell_in_table` should produce a
        // rectangular selection spanning the entire column (3 cells), which
        // is what the user expects when dragging a column down past the
        // bottom of the table.
        let mut manager = SelectionManager::new();
        manager.begin_render();

        let mut order = 0;
        let mut paragraph_index = 0;
        let mut cell_ids = [[0usize; 2]; 3];
        #[allow(clippy::needless_range_loop)]
        for row in 0..3 {
            #[allow(clippy::needless_range_loop)]
            for col in 0..2 {
                manager.append_paragraph_text(paragraph_index, "x");
                manager.record_line(
                    paragraph_index,
                    make_line("x", 0.0, 0.0, (paragraph_index * 20) as f64),
                );
                manager.record_table_cell(CellLayout {
                    paragraphs: vec![paragraph_index],
                    left: (col * 100) as f64,
                    right: ((col + 1) * 100) as f64,
                    top: (row * 20) as f64,
                    bottom: ((row + 1) * 20) as f64,
                    table_id: 0,
                    row,
                    column: col,
                    column_span: 1,
                    order,
                });
                cell_ids[row][col] = order;
                order += 1;
                paragraph_index += 1;
            }
        }

        let anchor_cell = cell_ids[0][0];
        let fallback_focus = manager
            .nearest_cell_in_table(
                manager.cell_table_id(anchor_cell).unwrap(),
                30.0,  // still aligned with column 0
                500.0, // far below the table
            )
            .unwrap();
        assert_eq!(fallback_focus, cell_ids[2][0]);

        manager.set_cell_selection(anchor_cell, fallback_focus);
        let ids = manager.selected_cell_ids().unwrap();
        let mut expected = vec![cell_ids[0][0], cell_ids[1][0], cell_ids[2][0]];
        expected.sort();
        let mut got = ids;
        got.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn select_entire_table_marks_every_cell_as_selected() {
        // 3 rows × 2 columns. When a drag leaves the table the pointer
        // handler converts the rectangular selection to "whole table
        // selected" by calling `select_entire_table`. Every cell in the
        // table must show up in `selected_cell_ids`, and no cell in a
        // different table must be included.
        let mut manager = SelectionManager::new();
        manager.begin_render();

        let mut order = 0;
        let mut paragraph_index = 0;
        let mut table_a_ids = Vec::new();
        let mut table_b_ids = Vec::new();
        for (table_id, bucket) in [(0, &mut table_a_ids), (1, &mut table_b_ids)] {
            #[allow(clippy::needless_range_loop)]
            for row in 0..3 {
                #[allow(clippy::needless_range_loop)]
                for col in 0..2 {
                    manager.append_paragraph_text(paragraph_index, "x");
                    manager.record_line(
                        paragraph_index,
                        make_line("x", 0.0, 0.0, (paragraph_index * 20) as f64),
                    );
                    manager.record_table_cell(CellLayout {
                        paragraphs: vec![paragraph_index],
                        left: (col * 100) as f64,
                        right: ((col + 1) * 100) as f64,
                        top: (row * 20) as f64,
                        bottom: ((row + 1) * 20) as f64,
                        table_id,
                        row,
                        column: col,
                        column_span: 1,
                        order,
                    });
                    bucket.push(order);
                    order += 1;
                    paragraph_index += 1;
                }
            }
        }

        assert!(manager.select_entire_table(0));
        let ids = manager.selected_cell_ids().unwrap();
        let mut got = ids;
        got.sort();
        let mut expected = table_a_ids.clone();
        expected.sort();
        assert_eq!(got, expected);

        // Paragraphs in table B must NOT be considered selected.
        for cell_id in &table_b_ids {
            let cell = manager.cell_layout(*cell_id).unwrap();
            for paragraph in &cell.paragraphs {
                assert!(!manager.paragraph_in_selected_cell(*paragraph));
            }
        }

        // Unknown table id: returns false, leaves selection state
        // belonging to table A alone.
        assert!(!manager.select_entire_table(99));
    }

    #[test]
    fn selected_lines_restricts_to_rectangular_cell_selection() {
        // 2 rows × 3 columns. Drag-select the middle column (anchor = r0c1,
        // focus = r1c1). The underlying text range spans paragraphs 1..=4
        // linearly (r0c1 → r0c2 → r1c0 → r1c1), but `selected_lines` must
        // only emit lines for the middle-column paragraphs (1 and 4).
        let mut manager = SelectionManager::new();
        manager.begin_render();

        let mut order = 0;
        let mut paragraph_index = 0;
        let mut cell_ids = [[0usize; 3]; 2];
        #[allow(clippy::needless_range_loop)]
        for row in 0..2 {
            #[allow(clippy::needless_range_loop)]
            for col in 0..3 {
                manager.append_paragraph_text(paragraph_index, "x");
                manager.record_line(
                    paragraph_index,
                    make_line("x", 0.0, 0.0, (paragraph_index * 20) as f64),
                );
                manager.record_table_cell(CellLayout {
                    paragraphs: vec![paragraph_index],
                    left: (col * 100) as f64,
                    right: ((col + 1) * 100) as f64,
                    top: (row * 20) as f64,
                    bottom: ((row + 1) * 20) as f64,
                    table_id: 0,
                    row,
                    column: col,
                    column_span: 1,
                    order,
                });
                cell_ids[row][col] = order;
                order += 1;
                paragraph_index += 1;
            }
        }

        // Simulate pointer_move_async: set both the text range and the
        // cell selection.
        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(1, 0),
            TextPosition::new(4, 1),
        )));
        manager.set_cell_selection(cell_ids[0][1], cell_ids[1][1]);

        let lines = manager.selected_lines();
        let mut paragraphs: Vec<usize> = lines.iter().map(|l| l.paragraph).collect();
        paragraphs.sort();
        assert_eq!(paragraphs, vec![1, 4]);
    }

    #[test]
    fn selected_cell_ids_infers_cells_from_selection_text() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "cell-one");
        manager.record_line(0, make_line("cell-one", 0.0, 0.0, 0.0));
        manager.append_paragraph_text(1, "cell-two");
        manager.record_line(1, make_line("cell-two", 0.0, 20.0, 20.0));

        manager.record_table_cell(CellLayout {
            paragraphs: vec![0],
            left: 0.0,
            right: 50.0,
            top: 0.0,
            bottom: 20.0,
            table_id: 1,
            row: 0,
            column: 0,
            column_span: 1,
            order: 0,
        });
        manager.record_table_cell(CellLayout {
            paragraphs: vec![1],
            left: 50.0,
            right: 100.0,
            top: 0.0,
            bottom: 20.0,
            table_id: 1,
            row: 0,
            column: 1,
            column_span: 1,
            order: 1,
        });

        let end_offset = "cell-two".len();
        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(1, end_offset),
        )));

        let ids = manager.selected_cell_ids().unwrap();
        assert_eq!(ids, vec![0, 1]);

        let layouts = manager.selected_cell_layouts().unwrap();
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].table_id, 1);
        assert_eq!(layouts[1].paragraphs, vec![1]);
    }

    /// Tests that set_selection only updates selection state without requiring re-render.
    /// This is important for performance optimization - selection changes should not
    /// trigger full document re-rendering.
    #[test]
    fn set_selection_returns_true_when_selection_changes() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello World");
        manager.record_line(0, make_line("Hello World", 0.0, 0.0, 0.0));

        // First selection change should return true
        let changed =
            manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 5))));
        assert!(
            changed,
            "set_selection should return true when selection changes"
        );

        // Same selection should return false (no change)
        let changed =
            manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 5))));
        assert!(
            !changed,
            "set_selection should return false when selection is unchanged"
        );

        // Different selection should return true
        let changed = manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        )));
        assert!(
            changed,
            "set_selection should return true when selection changes"
        );
    }

    /// Tests that selection changes don't modify the underlying paragraph layout data.
    /// This verifies that selection is a pure overlay and doesn't affect rendered content.
    #[test]
    fn set_selection_does_not_modify_paragraph_layout() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        let line = make_line("Hello", 10.0, 0.0, 0.0);
        manager.record_line(0, line.clone());

        // Capture layout state before selection
        let line_count_before = manager.paragraphs[0].lines.len();
        let text_before = manager.paragraphs[0].text.clone();

        // Change selection multiple times
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 0))));
        manager.set_selection(Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        )));
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 3))));

        // Verify layout data is unchanged
        assert_eq!(manager.paragraphs[0].lines.len(), line_count_before);
        assert_eq!(manager.paragraphs[0].text, text_before);
    }

    /// Tests that clear_selection returns correct change indicator.
    #[test]
    fn clear_selection_returns_change_indicator() {
        let mut manager = SelectionManager::new();
        manager.begin_render();
        manager.append_paragraph_text(0, "Hello");
        manager.record_line(0, make_line("Hello", 0.0, 0.0, 0.0));

        // Set a selection first
        manager.set_selection(Some(SelectionRange::collapsed(TextPosition::new(0, 0))));

        // Clear should return true (selection exists)
        let changed = manager.clear_selection();
        assert!(
            changed,
            "clear_selection should return true when selection existed"
        );

        // Clear again should return false (no selection)
        let changed = manager.clear_selection();
        assert!(
            !changed,
            "clear_selection should return false when no selection"
        );
    }

    #[test]
    #[ignore]
    fn benchmark_append_lines_with_y_offset_hot_path() {
        const ITERS: usize = 100_000;
        let lines: Vec<LineLayout> = (0..8)
            .map(|i| {
                make_line(
                    "Hello World",
                    10.0 + i as f64,
                    i as f64 * 20.0,
                    i as f64 * 20.0,
                )
            })
            .collect();

        let legacy_start = Instant::now();
        for _ in 0..ITERS {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.ensure_paragraph(0);
            for line in &lines {
                manager.record_line(
                    0,
                    LineLayout {
                        column_index: 0,
                        top: line.top + 12.0,
                        absolute_top: line.absolute_top + 12.0,
                        height: line.height,
                        baseline: line.baseline + 12.0,
                        start_x: line.start_x,
                        cursor_end_x: line.cursor_end_x,
                        allowed_trailing_overflow_width: line.allowed_trailing_overflow_width,
                        content_top: line.content_top + 12.0,
                        absolute_content_top: line.absolute_content_top + 12.0,
                        content_height: line.content_height,
                        fragments: line.fragments.clone(),
                        break_range: line.break_range,
                        tab_leaders: line.tab_leaders.clone(),
                        tab_stops: line.tab_stops.clone(),
                        vertical: false,
                        vertical_reversed: false,
                    },
                );
            }
            black_box(manager);
        }
        let legacy_elapsed = legacy_start.elapsed();

        let current_start = Instant::now();
        for _ in 0..ITERS {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.ensure_paragraph(0);
            manager.append_lines_with_y_offset(0, &lines, 12.0);
            black_box(manager);
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "selection append lines benchmark: legacy={}us current={}us iterations={} lines={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
            lines.len(),
        );
    }

    #[test]
    #[ignore]
    fn benchmark_append_cached_paragraph_hot_path() {
        const ITERS: usize = 100_000;
        let text = "Hello World";
        let lines: Vec<LineLayout> = (0..8)
            .map(|i| make_line(text, 10.0 + i as f64, i as f64 * 20.0, i as f64 * 20.0))
            .collect();

        let legacy_start = Instant::now();
        for _ in 0..ITERS {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.ensure_paragraph(0);
            manager.append_paragraph_text(0, text);
            manager.append_lines_with_y_offset(0, &lines, 12.0);
            black_box(manager);
        }
        let legacy_elapsed = legacy_start.elapsed();

        let current_start = Instant::now();
        for _ in 0..ITERS {
            let mut manager = SelectionManager::new();
            manager.begin_render();
            manager.append_paragraph_text_and_lines_with_y_offset(0, text, &lines, 12.0);
            black_box(manager);
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "selection append cached paragraph benchmark: legacy={}us current={}us iterations={} lines={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
            lines.len(),
        );
    }
}
