use super::*;
use crate::document::{
    resolve_cell_margin_edge_points, TableStyleCellFormat, TableStyleCellPosition, TextDirection,
    DEFAULT_VERTICAL_CELL_MARGIN_PT,
};
use jade_docx_layout::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};
use jade_docx_layout::layout_cache::{TextMeasureCache, TextMeasurer};
use jade_docx_layout::line_geometry::LineHorizontalMetrics;
use jade_docx_layout::line_wrap::{
    classify_script, LineWrapContext, ParagraphStyle, RunStyle, ScriptCategory, WrappedLine,
};
use jade_painter::Painter;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::f64::consts::FRAC_PI_2;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// Default font descriptor used while rendering table text onto the canvas.
const TABLE_FONT: &str = "12pt 'Times New Roman', serif";
thread_local! {
    static DEFAULT_TABLE_RUN_STYLE: Rc<ResolvedRunStyle> = Rc::new(ResolvedRunStyle::new(
        TABLE_FONT.to_string(), TABLE_FONT.to_string(), TABLE_FONT.to_string(), TABLE_FONT.to_string(),
        0.0, 1.0, false, None,
    ));
}

fn default_table_run_style() -> Rc<ResolvedRunStyle> {
    DEFAULT_TABLE_RUN_STYLE.with(Rc::clone)
}
/// セルの 1 行をどこに置くか。
///
/// `start_x` / `top` / `height` をまとめて渡すためのもの。ばらばらに引数で
/// 並べると、描画の関数が引数だらけになって順番を取り違えやすい。
///
/// 表の描画は `textBaseline` を `"top"` にしているので、`top` は行の上端
/// (`fill_text` へ渡す y) を指す。本文側は `"alphabetic"` で、あちらの y は
/// ベースラインになる。
#[derive(Clone, Copy)]
struct CellLinePlacement {
    start_x: f64,
    top: f64,
    height: f64,
}

/// Fallback text color when neither the cell nor the table specifies one.
const DEFAULT_TEXT_COLOR: &str = "#000000";
/// Stroke color for table borders.
const BORDER_COLOR: &str = "#000000";
/// Theme-aware replacement for default black borders on the dark preview page.
///
/// Keep this slightly quieter than body text while retaining strong contrast
/// against the runtime's default dark page (`#1a1f2c`). Low-contrast authored
/// colors are blended toward this value for preview only; the document color
/// itself is preserved for export and light mode.
const DARK_MODE_BORDER_COLOR: &str = "#cbd5e1";
/// Runtime dark page used by `DEFAULT_THEME_COLORS` in `@jlsi/jade-docx-editor`.
const DARK_MODE_PAGE_COLOR: (u8, u8, u8) = (0x1a, 0x1f, 0x2c);
/// WCAG contrast target used for thin graphical boundaries. We intentionally
/// use the stronger text threshold: Word table hairlines are often only one
/// device pixel wide, so the 3:1 non-text threshold still looks faint.
const DARK_MODE_BORDER_MIN_CONTRAST: f64 = 4.5;
/// Stroke color used for Word-like guide lines when a border is explicitly hidden.
const HIDDEN_BORDER_COLOR: &str = "#9A9A9A";
/// Dark-preview counterpart for hidden-border editing guides. Keep it dimmer
/// than a real table border, but bright enough that an empty table remains
/// discoverable on the dark page.
const DARK_MODE_HIDDEN_BORDER_COLOR: &str = "#94A3B8";
/// Horizontal padding (in points) applied on both sides of table cell content.
const CELL_HORIZONTAL_PADDING_PT: f32 = 6.0;
/// Vertical padding (in points) applied above and below table cell content.
const CELL_VERTICAL_PADDING_PT: f32 = DEFAULT_VERTICAL_CELL_MARGIN_PT;
const VERTICAL_TABLE_TEXT_TOP_INSET_FACTOR: f64 = 0.25;
const VERTICAL_TABLE_ROTATED_BASELINE_INSET_FACTOR: f64 = 0.8;
const VERTICAL_TABLE_ROTATED_RIGHT_INSET_FACTOR: f64 = 0.15;
/// Word leaves a tiny visual gap between a table border and following paragraph.
/// Apply this only when no explicit table margin-bottom is provided.
const DEFAULT_TABLE_AFTER_GAP_PX: f64 = 1.25;
const ROW_SPLIT_EPSILON: f64 = 0.001;
/// Keep a fitted paragraph just inside the wrap boundary. Canvas/browser text
/// measurement can accumulate a few millionths of a pixel across mixed runs;
/// without this tolerance an exactly fitted final word may wrap by itself.
const CELL_FIT_TEXT_EPSILON_PX: f64 = 0.01;
const TABLE_AUTOFIT_CACHE_CAPACITY: usize = 64;
const TABLE_ASCII_MEASURE_MEMO_CAPACITY: usize = 4096;

fn resolve_table_border_stroke_color(requested: &str, dark_mode: bool) -> Cow<'_, str> {
    if !dark_mode {
        return Cow::Borrowed(requested);
    }
    if should_use_dark_mode_default_text_color(requested)
        || requested.trim().eq_ignore_ascii_case("auto")
    {
        return Cow::Borrowed(DARK_MODE_BORDER_COLOR);
    }

    let Some(source) = parse_table_border_rgb(requested) else {
        return Cow::Borrowed(requested);
    };
    if color_contrast_ratio(source, DARK_MODE_PAGE_COLOR) >= DARK_MODE_BORDER_MIN_CONTRAST {
        return Cow::Borrowed(requested);
    }

    let target = parse_table_border_rgb(DARK_MODE_BORDER_COLOR)
        .expect("DARK_MODE_BORDER_COLOR must be a six-digit hex color");
    // Blend only the preview stroke toward the theme border color. The OOXML
    // model and exported `w:color` remain untouched.
    for step in 1..=255_u16 {
        let mixed = (
            blend_color_channel(source.0, target.0, step),
            blend_color_channel(source.1, target.1, step),
            blend_color_channel(source.2, target.2, step),
        );
        if color_contrast_ratio(mixed, DARK_MODE_PAGE_COLOR) >= DARK_MODE_BORDER_MIN_CONTRAST {
            return Cow::Owned(format!("#{:02X}{:02X}{:02X}", mixed.0, mixed.1, mixed.2));
        }
    }

    Cow::Borrowed(DARK_MODE_BORDER_COLOR)
}

fn resolve_hidden_table_border_stroke_color(dark_mode: bool) -> &'static str {
    if dark_mode {
        DARK_MODE_HIDDEN_BORDER_COLOR
    } else {
        HIDDEN_BORDER_COLOR
    }
}

fn resolve_table_border_stroke_width(requested: f64, _dark_mode: bool) -> f64 {
    // Keep Word hairlines at one complete CSS pixel. A 4-eighth-point table
    // grid resolves to 0.667 px; centered on a half-pixel boundary that leaves
    // every device pixel partially transparent and makes page-break rules look
    // like hidden-border guides. This changes canvas presentation only; the
    // authored OOXML width is retained for export.
    requested.max(1.0)
}

fn parse_table_border_rgb(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.trim().strip_prefix('#').unwrap_or(color.trim());
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

fn blend_color_channel(source: u8, target: u8, step: u16) -> u8 {
    let inverse = 255_u16.saturating_sub(step);
    ((u16::from(source) * inverse + u16::from(target) * step + 127) / 255) as u8
}

fn color_contrast_ratio(first: (u8, u8, u8), second: (u8, u8, u8)) -> f64 {
    let first = color_relative_luminance(first);
    let second = color_relative_luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn color_relative_luminance((red, green, blue): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

/// A top-level floating table frame retained for the current render pass.
///
/// Word's `w:tblOverlap` contract is pairwise: if either intersecting table
/// says `never`, the later frame must be adjusted. Keeping both the rectangle
/// and the earlier table's policy lets the later table honor that rule without
/// introducing paragraph-wrap calculations in the renderer.
#[derive(Clone, Copy, Debug)]
pub(super) struct FloatingTableFrame {
    pub(super) page_number: usize,
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) top: f64,
    pub(super) bottom: f64,
    pub(super) prevents_overlap: bool,
}

/// Clips to a rectangle for as long as it is alive.
///
/// Borrows the painter rather than cloning the drawing context: `Painter` is
/// the only thing a guard needs, and a backend's surface is not necessarily
/// cheap (or possible) to clone the way a canvas handle is.
struct ClipGuard<'a, P: Painter> {
    painter: &'a P,
}

impl<'a, P: Painter> ClipGuard<'a, P> {
    fn new(painter: &'a P, x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        painter.save();
        painter.begin_path();
        painter.rect(x, y, width, height);
        painter.clip();
        Some(Self { painter })
    }
}

impl<P: Painter> Drop for ClipGuard<'_, P> {
    fn drop(&mut self) {
        self.painter.restore();
    }
}

/// Preserves canvas state while table rendering temporarily switches baseline/font settings.
/// Saves painter state on construction and restores it on drop.
struct StateGuard<'a, P: Painter> {
    painter: &'a P,
}

impl<'a, P: Painter> StateGuard<'a, P> {
    fn new(painter: &'a P) -> Self {
        painter.save();
        Self { painter }
    }
}

impl<P: Painter> Drop for StateGuard<'_, P> {
    fn drop(&mut self) {
        self.painter.restore();
    }
}

/// Represents a fully laid out line of text ready to be painted on the canvas.
#[derive(Clone, Debug)]
struct LineLayout {
    text: String,
    visual_width: f64,
    left_inset: f64,
    right_inset: f64,
    start: usize,
    end: usize,
    glyphs: Vec<GlyphBox>,
    break_kind: LineBreakKind,
    break_range: Option<(usize, usize)>,
    break_properties: Option<Rc<RunProperties>>,
    /// Baseline below the line top when the line takes Word's East Asian
    /// font line box (`ParagraphStyle::word_east_asian_font_line_box`). Then
    /// the text is drawn on that baseline, as in body paragraphs, instead of
    /// hanging from the line top in whatever face draws it.
    word_baseline_offset: Option<f64>,
    /// The height a line holding an inline picture needs at least: the
    /// picture stands on the baseline and the text's descent hangs below it.
    /// Zero for a line of text only.
    min_advance: f64,
}

impl LineLayout {
    fn aligned_start_x(&self, cell_start: f64, content_width: f64, align: TextAlign) -> f64 {
        let alignment_width = if !matches!(align, TextAlign::Left)
            && jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        {
            jade_docx_layout::line_wrap::soft_line_alignment_width_px(
                self.visual_width,
                self.break_kind,
                self.glyphs.iter().rev().map(|glyph| {
                    let text = glyph
                        .start
                        .checked_sub(self.start)
                        .zip(glyph.end.checked_sub(self.start))
                        .and_then(|(start, end)| self.text.get(start..end))
                        .unwrap_or("");
                    (text, glyph.x, glyph.width)
                }),
            )
        } else {
            self.visual_width
        };
        let remaining =
            (content_width - self.left_inset - self.right_inset - alignment_width).max(0.0);
        cell_start
            + self.left_inset
            + match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => remaining / 2.0,
                TextAlign::Right => remaining,
            }
    }

    fn horizontal_metrics(&self, start_x: f64) -> LineHorizontalMetrics {
        LineHorizontalMetrics {
            start_x,
            cursor_end_x: start_x + self.visual_width,
        }
    }

    fn visual_width(&self) -> f64 {
        self.visual_width
    }

    fn visual_end_x(&self, start_x: f64) -> f64 {
        self.horizontal_metrics(start_x)
            .visual_end_x(std::iter::empty())
    }
}

/// Text range backed by a single DOCX run in a table paragraph.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct StyledRunRange {
    start: usize,
    end: usize,
    properties: Option<RunProperties>,
    /// この run に付いている変更履歴 (挿入 / 削除 / 移動)。
    ///
    /// 以前はここで捨てていたので、表の中の変更履歴に色が付かなかった。
    revision: Option<RevisionMark>,
    /// An inline picture this run stands for (one `U+FFFC` in the cell text).
    picture: Option<CellPicture>,
}

/// An inline picture in a table cell paragraph: the run that holds it and
/// its model. Laid out as one inline object; the model text has no
/// character for it, as in body paragraphs.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CellPicture {
    run_index: usize,
    image: Rc<RunImage>,
}

impl CellPicture {
    /// The picture's model size as the line wrapper's inline object.
    fn inline_object(&self) -> jade_docx_layout::line_wrap::InlineObject {
        jade_docx_layout::line_wrap::InlineObject {
            width: self.image.width_px.unwrap_or(1.0).max(1.0),
            height: self.image.height_px.unwrap_or(1.0).max(1.0),
        }
    }
}

/// The picture an inline-image run lays out in a table cell, if any: a
/// floating picture is not part of the cell's lines, and one without a size
/// has nothing to reserve.
fn cell_inline_picture(run: &Run, run_index: usize) -> Option<CellPicture> {
    let image = run.image.as_ref()?;
    (matches!(image.layout.kind, RunImageKind::Inline)
        && image.width_px.is_some_and(|width| width > 0.0)
        && image.height_px.is_some_and(|height| height > 0.0))
    .then(|| CellPicture {
        run_index,
        image: Rc::new(image.clone()),
    })
}

/// Paragraph representation within a table cell along with its line layouts.
#[derive(Clone, Debug, Default)]
pub(super) struct CellParagraph {
    text: String,
    /// Page fragments retain full text/style offsets but register model text once.
    is_continuation: bool,
    /// Spacing belongs only to the first/last page fragment of a paragraph.
    spacing_before: f64,
    spacing_after: f64,
    /// Resolved paragraph formatting takes precedence over the cell fallback.
    align: Option<TextAlign>,
    lines: Rc<Vec<LineLayout>>,
    styled_runs: Vec<StyledRunRange>,
    max_line_width: f64,
    height: f64,
    /// The line height the paragraph was laid out with: its own resolved
    /// line (`CanvasRenderer::resolve_paragraph_line_height_points`), which
    /// drawing has to advance by too. The table-wide empty-paragraph line is
    /// only the fallback for cells without a paragraph.
    line_height: f64,
    vertical_layout: Option<VerticalTextLayout>,
    /// Byte boundary already painted by an earlier page of this paragraph.
    segment_start: usize,
    keep_lines: bool,
    widow_control: bool,
    note_anchors: Vec<TableNoteAnchor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableNoteAnchor {
    id: String,
    footnote: bool,
    offset: usize,
    /// Document-order ordinal for continuous/section numbering. Each-page
    /// numbering is assigned only when the reference's segment is placed.
    ordinal: Option<usize>,
}

impl CellParagraph {
    /// How far `line` advances: the paragraph's line, or more for a line
    /// holding an inline picture.
    fn line_advance(&self, line: &LineLayout) -> f64 {
        self.line_height.max(line.min_advance)
    }

    /// The height of `lines` laid out in this paragraph.
    fn lines_height(&self, lines: &[LineLayout]) -> f64 {
        lines.iter().map(|line| self.line_advance(line)).sum()
    }

    /// Layout offsets of this paragraph's inline pictures, in order.
    fn picture_offsets(&self) -> impl Iterator<Item = usize> + '_ {
        self.styled_runs
            .iter()
            .filter(|run| run.picture.is_some())
            .map(|run| run.start)
    }

    /// The model offset of layout offset `offset`: an inline picture has a
    /// character in the layout text and none in the model.
    fn model_offset(&self, offset: usize) -> usize {
        let pictures = self
            .picture_offsets()
            .filter(|start| *start < offset)
            .count();
        offset.saturating_sub(pictures * CELL_PICTURE_CHAR.len_utf8())
    }

    /// The paragraph's model text: its layout text without its pictures.
    fn model_text(&self) -> std::borrow::Cow<'_, str> {
        let mut pictures = self.picture_offsets().peekable();
        if pictures.peek().is_none() {
            return std::borrow::Cow::Borrowed(&self.text);
        }
        let mut text = String::with_capacity(self.text.len());
        let mut cursor = 0;
        for start in pictures {
            let start = start.min(self.text.len());
            text.push_str(&self.text[cursor.min(start)..start]);
            cursor = start + CELL_PICTURE_CHAR.len_utf8();
        }
        text.push_str(&self.text[cursor.min(self.text.len())..]);
        std::borrow::Cow::Owned(text)
    }

    /// Moves selection fragments from layout to model offsets.
    fn remap_selection_fragments(&self, fragments: &mut [SelectionFragmentLayout]) {
        if self.picture_offsets().next().is_none() {
            return;
        }
        for fragment in fragments {
            fragment.start = self.model_offset(fragment.start);
            fragment.end = self.model_offset(fragment.end);
            let mut glyphs = fragment.glyphs.to_vec();
            for glyph in &mut glyphs {
                glyph.start = self.model_offset(glyph.start);
                glyph.end = self.model_offset(glyph.end);
            }
            fragment.glyphs = glyphs.into();
        }
    }

    fn fragment(&self, range: Range<usize>, segment_start: usize) -> Self {
        let lines = self.lines[range].to_vec();
        let height = self.lines_height(&lines);
        Self {
            text: self.text.clone(),
            is_continuation: self.is_continuation || segment_start > self.segment_start,
            spacing_before: if segment_start == self.segment_start {
                self.spacing_before
            } else {
                0.0
            },
            spacing_after: if lines.last().map(|line| line.end)
                == self.lines.last().map(|line| line.end)
            {
                self.spacing_after
            } else {
                0.0
            },
            align: self.align,
            lines: Rc::new(lines),
            styled_runs: self.styled_runs.clone(),
            max_line_width: self.max_line_width,
            height,
            line_height: self.line_height,
            vertical_layout: self.vertical_layout.clone(),
            segment_start,
            keep_lines: self.keep_lines,
            widow_control: self.widow_control,
            note_anchors: self.note_anchors.clone(),
        }
    }

    pub(super) fn height(&self) -> f64 {
        self.height
    }

    #[cfg(all(debug_assertions, target_arch = "wasm32"))]
    pub(super) fn line_count(&self) -> usize {
        self.lines.len()
    }

    #[cfg(any(test, all(debug_assertions, target_arch = "wasm32")))]
    pub(super) fn column_count(&self) -> usize {
        self.vertical_layout
            .as_ref()
            .map(|layout| layout.column_ranges.len())
            .unwrap_or(0)
    }

    #[cfg(any(test, all(debug_assertions, target_arch = "wasm32")))]
    pub(super) fn vertical_line_count(&self) -> usize {
        self.vertical_layout
            .as_ref()
            .map(|layout| {
                layout
                    .column_ranges
                    .iter()
                    .map(|range| range.end.saturating_sub(range.start))
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
struct VerticalTextLayout {
    column_ranges: Vec<Range<usize>>,
    column_height: f64,
}

/// Supported horizontal alignments for table cell text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextAlign {
    Left,
    Center,
    Right,
}

// A grid column inside a horizontal merge has no visible boundary to drag.
// Keeping a full-height handle there steals clicks from text and nested tables.
fn column_boundary_crosses_merged_cell(cells: &[PreparedCell], boundary: usize) -> bool {
    cells.iter().any(|cell| {
        cell.column_index < boundary && boundary < cell.column_index.saturating_add(cell.span)
    })
}

fn paragraph_text_align_for_direction(
    paragraph: &Paragraph,
    direction: Option<TextDirection>,
) -> Option<TextAlign> {
    // Paragraph jc describes the inline axis. TextAlign here positions the
    // column block on X; applying a vertical paragraph's jc would move the
    // columns from the cell's right edge to its left edge.
    if direction.is_some_and(direction_requires_vertical_stack) {
        None
    } else {
        paragraph_text_align(paragraph)
    }
}

fn paragraph_text_align(paragraph: &Paragraph) -> Option<TextAlign> {
    use jade_docx_layout::line_wrap::{
        resolve_physical_paragraph_alignment, PhysicalParagraphAlignment,
    };
    let properties = paragraph.properties.as_ref()?;
    let bidi = properties.bidi.unwrap_or(false);
    let alignment = properties
        .alignment
        .as_deref()
        .or_else(|| bidi.then_some("start"))?;
    Some(
        match resolve_physical_paragraph_alignment(alignment, bidi) {
            PhysicalParagraphAlignment::Center => TextAlign::Center,
            PhysicalParagraphAlignment::Right => TextAlign::Right,
            PhysicalParagraphAlignment::Left | PhysicalParagraphAlignment::Justify => {
                TextAlign::Left
            }
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnFlow {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerticalAxis {
    TopToBottom,
    BottomToTop,
}

/// Visual kind of a table cell border as expressed by OOXML `w:val`.
///
/// The renderer paints each kind differently (single stroke, two parallel
/// strokes for `Double`, dashed pattern for `Dashed`, etc.). Unknown OOXML
/// style tokens fall back to [`BorderKind::Single`] so the border is still
/// drawn instead of disappearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BorderKind {
    /// Explicitly hidden (`w:val="nil"`/`"none"`). Rendered as a pale guide
    /// line so the table grid remains visible while editing.
    None,
    Single,
    Thick,
    Double,
    Triple,
    Dashed,
    DashSmallGap,
    Dotted,
    DotDash,
    DotDotDash,
    /// `wave` / `doubleWave` — approximated with a dashed pattern since the
    /// Canvas API has no built-in wavy-stroke primitive.
    Wave,
}

/// Fully resolved drawing parameters for one side of a table cell border.
///
/// Captures the [`BorderKind`] together with the stroke thickness (derived
/// from OOXML `w:sz`, which is given in eighths of a point) and the stroke
/// color (from `w:color`, falling back to the default table border color).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CellBorderSpec {
    kind: BorderKind,
    /// Compact forms of Word's border number and equal-weight style rank.
    /// Keeping these beside the drawable kind avoids retaining another heap
    /// string for every edge of every prepared table cell.
    conflict_number: u8,
    conflict_precedence: u8,
    width_px: f64,
    color: String,
}

impl CellBorderSpec {
    fn hidden() -> Self {
        Self {
            kind: BorderKind::None,
            conflict_number: 0,
            conflict_precedence: 0,
            width_px: 1.0,
            color: HIDDEN_BORDER_COLOR.to_string(),
        }
    }

    #[cfg(test)]
    fn single_default() -> Self {
        Self {
            kind: BorderKind::Single,
            conflict_number: 1,
            conflict_precedence: 1,
            width_px: 1.0,
            color: BORDER_COLOR.to_string(),
        }
    }
}

/// One physical portion of a cell's top or bottom border. Segments are needed
/// when a spanning cell faces multiple cells in the adjacent row and Word's
/// conflict procedure selects a different winner for each overlap.
#[derive(Clone, Debug, PartialEq)]
struct HorizontalBorderSegment {
    start_px: f64,
    end_px: f64,
    border: CellBorderSpec,
}

/// Pre-computed rendering data for an individual table cell.
#[derive(Clone, Debug)]
struct PreparedCell {
    column_index: usize,
    span: usize,
    vertical_merge: Option<String>,
    border_top: CellBorderSpec,
    /// Boxed slices keep the common empty case compact while still allowing a
    /// spanning edge to retain multiple immutable draw segments after prepare.
    border_top_segments: Box<[HorizontalBorderSegment]>,
    border_right: CellBorderSpec,
    border_bottom: CellBorderSpec,
    border_bottom_segments: Box<[HorizontalBorderSegment]>,
    border_left: CellBorderSpec,
    /// Optional physical cell diagonals (`w:tl2br` / `w:tr2bl`). Unlike the
    /// four grid edges, absence means no editable guide line should be drawn.
    border_tl2br: Option<CellBorderSpec>,
    border_tr2bl: Option<CellBorderSpec>,
    padding: CellPadding,
    /// Effective row/table `w:tblCellSpacing`, converted to pixels. Kept on
    /// each prepared cell so page splits and repeated headers reuse identical
    /// geometry without consulting mutable document state.
    cell_spacing: f64,
    blocks: Vec<CellBlock>,
    height: f64,
    background: Option<String>,
    text_color: Option<String>,
    align: TextAlign,
    text_direction: Option<TextDirection>,
    /// Fraction of the leftover vertical space placed above the cell content
    /// (`w:vAlign`): 0.0 = top (Word default), 0.5 = center, 1.0 = bottom.
    vertical_align_share: f64,
    skip_render: bool,
    logical_paragraph_count: usize,
    /// Model paragraphs suppressed only from visual layout by `w:hideMark`.
    /// They still reserve selection indices so later cells stay aligned with
    /// `WordDocument::paragraph_surfaces`.
    hidden_end_marker_paragraph_count: usize,
    page_top_inset: f64,
    selection_paragraph_indices: Vec<usize>,
}

impl PreparedCell {
    fn top_border_extent(&self) -> f64 {
        horizontal_border_extent(&self.border_top, &self.border_top_segments)
    }

    fn bottom_border_extent(&self) -> f64 {
        horizontal_border_extent(&self.border_bottom, &self.border_bottom_segments)
    }

    /// A page fragment copies cell geometry, not discarded paragraph glyphs.
    fn fragment(&self, blocks: Vec<CellBlock>) -> Self {
        Self {
            column_index: self.column_index,
            span: self.span,
            vertical_merge: self.vertical_merge.clone(),
            border_top: self.border_top.clone(),
            border_top_segments: self.border_top_segments.clone(),
            border_right: self.border_right.clone(),
            border_bottom: self.border_bottom.clone(),
            border_bottom_segments: self.border_bottom_segments.clone(),
            border_left: self.border_left.clone(),
            border_tl2br: self.border_tl2br.clone(),
            border_tr2bl: self.border_tr2bl.clone(),
            padding: self.padding,
            cell_spacing: self.cell_spacing,
            blocks,
            height: 0.0,
            background: self.background.clone(),
            text_color: self.text_color.clone(),
            align: self.align,
            text_direction: self.text_direction,
            vertical_align_share: self.vertical_align_share,
            skip_render: self.skip_render,
            logical_paragraph_count: self.logical_paragraph_count,
            hidden_end_marker_paragraph_count: self.hidden_end_marker_paragraph_count,
            page_top_inset: self.page_top_inset,
            selection_paragraph_indices: self.selection_paragraph_indices.clone(),
        }
    }
}

/// Nonuniform horizontal rules occupy one shared band, aligned at its top.
/// Stack-only paint/content inputs; prepared cells retain their cached geometry.
#[derive(Clone, Copy, Default)]
struct HorizontalBorderBands {
    top: f64,
    bottom: f64,
    content_bottom: f64,
}

impl HorizontalBorderBands {
    fn for_row(cells: &[PreparedCell], reserve_bottom: bool) -> Self {
        // The widest rule along the edge, and whether its cells disagree.
        let extent = |top: bool| {
            let mut minimum = f64::INFINITY;
            let mut maximum = 0.0_f64;
            let mut replaced = false;
            for cell in cells.iter().filter(|cell| !cell.skip_render) {
                if cell.cell_spacing > f64::EPSILON {
                    return (0.0, false);
                }
                let (fallback, segments) = if top {
                    (&cell.border_top, &cell.border_top_segments)
                } else {
                    (&cell.border_bottom, &cell.border_bottom_segments)
                };
                let scalar = visible_border_width(fallback);
                if segments.is_empty() {
                    minimum = minimum.min(scalar);
                    maximum = maximum.max(scalar);
                } else {
                    for segment in segments
                        .iter()
                        .filter(|segment| horizontal_segment_has_extent(segment))
                    {
                        let width = visible_border_width(&segment.border);
                        minimum = minimum.min(width);
                        maximum = maximum.max(width);
                        replaced |= (width - scalar).abs() > f64::EPSILON;
                    }
                }
            }
            (maximum, replaced || maximum - minimum > f64::EPSILON)
        };
        let band = |(width, nonuniform): (f64, bool)| if nonuniform { width } else { 0.0 };
        let top = band(extent(true));
        let (bottom_width, bottom_nonuniform) = extent(false);
        let bottom = band((bottom_width, bottom_nonuniform));
        // Word for Mac aligns a cell's content within the row above the
        // border the row reserves below it, even when every cell draws the
        // same rule there: a centred line does not sink by half of a 0.5pt
        // rule (`scroll-offset-repro`).
        let content_bottom = if !reserve_bottom {
            0.0
        } else if jade_docx_layout::line_metrics::word_layout_platform()
            == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        {
            bottom_width
        } else {
            bottom
        };
        Self {
            top,
            bottom,
            content_bottom,
        }
    }
}

/// Shared physical grid inputs for visual-only header redraws. Keeping the
/// mirroring bit beside the offsets prevents repeated headers from drifting
/// away from the body-table coordinate system.
#[derive(Clone, Copy)]
struct TableVisualGrid<'a> {
    table_x: f64,
    column_positions: &'a [f64],
    column_widths: &'a [f64],
    bidi_visual: bool,
}

/// Maps a normalized `w:vAlign` token to the fraction of leftover vertical
/// space placed above the cell content. `top` (and absent/unknown) keep content
/// at the top (Word's default), `center` splits the slack evenly, and `bottom`
/// pushes content to the cell floor. See ECMA-376 Part 1 §17.4.84.
pub(super) fn cell_vertical_align_share(token: Option<&str>) -> f64 {
    match token {
        Some("center") => 0.5,
        Some("bottom") => 1.0,
        _ => 0.0,
    }
}

/// A single block-level element that can appear inside a table cell. Keeps
/// track of its layout metrics and the paragraph indices associated with the
/// rendered selection.
#[derive(Clone, Debug)]
struct CellBlock {
    kind: CellBlockKind,
    height: f64,
    paragraph_indices: Vec<usize>,
    source_index: usize,
}

/// Supported kinds of block-level content inside a table cell.
#[derive(Clone, Debug)]
enum CellBlockKind {
    /// A paragraph margin continued across a page without a text/caret surface.
    Spacing,
    Paragraph(CellParagraph),
    Table(NestedTableLayout),
}

/// Cached layout for a nested table so that it can be rendered within a parent
/// cell without recomputing widths and heights.
#[derive(Clone, Debug)]
struct NestedTableLayout {
    table: Box<Table>,
    /// Preorder model identity, independent of the order page fragments paint.
    order: Option<usize>,
    column_widths: Vec<f64>,
    column_positions: Vec<f64>,
    rows: Vec<NestedRowLayout>,
    width: f64,
    height: f64,
    margin_top: f64,
    margin_bottom: f64,
    line_height: f64,
}

/// Prepared data for a single nested table row.
#[derive(Clone, Debug)]
struct NestedRowLayout {
    /// Row in the source table, retained across page fragments.
    source_index: usize,
    cells: Vec<PreparedCell>,
    height: f64,
    hidden: bool,
}

#[derive(Clone, Debug)]
struct SplitPreparedRow {
    head_cells: Vec<PreparedCell>,
    head_height: f64,
    tail_cells: Vec<PreparedCell>,
    tail_height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VerticalMergeContinuationBorderSides {
    top: bool,
    bottom: bool,
}

#[derive(Clone, Copy, Debug)]
struct CellBorderRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// A separated row pays one complete gap at its leading edge and between
/// cells. The row's slots have already been reduced to reserve the last gap.
/// Vertically each row reserves the declared spacing above and below content.
fn spaced_cell_rect(x: f64, y: f64, width: f64, height: f64, spacing: f64) -> CellBorderRect {
    let horizontal_inset = (2.0 * spacing.max(0.0)).min(width.max(0.0));
    let vertical_inset = spacing.max(0.0).min(height.max(0.0) / 2.0);
    CellBorderRect {
        x: x + horizontal_inset,
        y: y + vertical_inset,
        width: (width - horizontal_inset).max(0.0),
        height: (height - 2.0 * vertical_inset).max(0.0),
    }
}

fn spaced_cell_border_rect(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    cell: &PreparedCell,
) -> CellBorderRect {
    if cell.cell_spacing <= f64::EPSILON {
        return CellBorderRect {
            x,
            y,
            width,
            height,
        };
    }
    let rect = spaced_cell_rect(x, y, width, height, cell.cell_spacing);
    let left = visible_border_width(&cell.border_left) * 0.5;
    let right = visible_border_width(&cell.border_right) * 0.5;
    CellBorderRect {
        x: rect.x - left,
        y,
        width: (rect.width - left - right).max(0.0),
        height,
    }
}

fn spaced_cell_shading_rect(rect: CellBorderRect, cell: &PreparedCell) -> CellBorderRect {
    if cell.cell_spacing <= f64::EPSILON {
        return rect;
    }
    let left = visible_border_width(&cell.border_left) * 0.5;
    let right = visible_border_width(&cell.border_right) * 0.5;
    let vertical = cell.cell_spacing.min(rect.height.max(0.0) * 0.5);
    let initial_gap = (cell.page_top_inset - cell.top_border_extent()).max(0.0);
    CellBorderRect {
        x: rect.x + left,
        y: rect.y + vertical + initial_gap,
        width: (rect.width - left - right).max(0.0),
        height: (rect.height - 2.0 * vertical - initial_gap).max(0.0),
    }
}

/// Cell borders occupy the spacing budget as well as the outer table frame.
/// Preserve the authored table width and charge the two perimeter differences
/// before distributing the remaining slots in proportion to the grid.
fn spacing_slot_scale(
    width: f64,
    spacing: f64,
    cell_leading: f64,
    cell_trailing: f64,
    frame_leading: f64,
    frame_trailing: f64,
) -> f64 {
    if spacing <= f64::EPSILON || width <= f64::EPSILON {
        return 1.0;
    }
    ((width - 2.0 * spacing + cell_leading + cell_trailing - frame_leading - frame_trailing)
        .max(0.0)
        / width)
        .max(0.0)
}

fn spacing_frame_width(row: &TableRow, table: &Table, side: TableBorderSide) -> f64 {
    spaced_table_frame_edge(row, table, side)
        .map(|edge| visible_border_width(&border_edge_to_spec(edge)))
        .unwrap_or(0.0)
}

fn prepared_spacing_slot_scale(
    row: &TableRow,
    table: &Table,
    cells: &[PreparedCell],
    width: f64,
) -> f64 {
    let Some(first) = cells
        .first()
        .filter(|cell| cell.cell_spacing > f64::EPSILON)
    else {
        return 1.0;
    };
    let Some(last) = cells.last() else {
        return 1.0;
    };
    let (leading, trailing) = if table.bidi_visual {
        (&first.border_right, &last.border_left)
    } else {
        (&first.border_left, &last.border_right)
    };
    spacing_slot_scale(
        width,
        first.cell_spacing,
        visible_border_width(leading),
        visible_border_width(trailing),
        spacing_frame_width(row, table, TableBorderSide::Left),
        spacing_frame_width(row, table, TableBorderSide::Right),
    )
}

/// The last gap is outside the reduced row slots. It remains part of the
/// unchanged table frame, without covering any cell interior.
fn paint_spacing_tail<P: Painter>(
    context: &P,
    background: Option<&str>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
) {
    if let Some(color) = background.filter(|_| scale < 1.0 && width > 0.0 && height > 0.0) {
        context.set_fill_style_str(color);
        context.fill_rect(x + width * scale, y, width * (1.0 - scale), height);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PageClippedCellSegment {
    height: f64,
    clipped_at_bottom: bool,
}

/// Common measurements derived from the current table rendering configuration.
struct LayoutConfig {
    padding_x: f64,
    padding_y: f64,
    /// Line height (px) of an empty paragraph in the current section: the
    /// fallback for cells without any paragraph and for synthetic blocks.
    line_height: f64,
    /// Per-paragraph line heights resolved by the renderer's shared body
    /// path, keyed by paragraph address (see
    /// `CanvasRenderer::collect_table_paragraph_line_heights`).
    paragraph_line_heights: FxHashMap<usize, f64>,
    /// Per-paragraph East Asian wrapping rules (kinsoku, autoSpaceDE/DN)
    /// resolved by the renderer's body path, keyed like
    /// `paragraph_line_heights`, so `2026年5月29日` in a cell gets the same
    /// digit/kanji spacing as in the body.
    paragraph_east_asian_rules: FxHashMap<usize, EastAsianLineBreakingRules>,
    available_width: f64,
    render_changes: bool,
    render_original: bool,
    /// Reference marks for the notes referenced from this table's cells,
    /// provisional until each row registers its notes at placement (see
    /// `CanvasRenderer::register_table_row_notes`).
    note_markers: NoteMarkers,
    doc_grid: Option<DocGrid>,
    styles_part: Option<Rc<StylesPart>>,
    document_snap_to_grid_default: bool,
    /// The document's `w:useFELayout`.
    document_uses_east_asian_layout: bool,
    word_modern_border_insets: bool,
}

/// Rendered reference marks ("1", "ⅰ", …) keyed by note id, one map per note
/// kind because footnote and endnote ids are separate spaces.
#[derive(Clone, Debug, Default)]
pub(super) struct NoteMarkers {
    footnotes: FxHashMap<String, String>,
    endnotes: FxHashMap<String, String>,
    footnote_ordinals: FxHashMap<String, usize>,
    endnote_ordinals: FxHashMap<String, usize>,
    /// Only the table's provisional plan needs source-order ordinals. Actual
    /// registrations need text marks; each-page ordinals are never retained.
    retain_ordinals: bool,
}

/// Note numbering state a trial registration can roll back.
#[derive(Clone)]
struct NoteRegistrationState {
    footnote_display_map: HashMap<String, usize>,
    footnote_order: Vec<String>,
    footnote_counter: usize,
    footnote_numbering_scope: super::NoteNumberingScope,
    footnote_section_map: HashMap<String, usize>,
    endnote_display_map: HashMap<String, usize>,
    endnote_order: Vec<String>,
    endnote_counter: usize,
    endnote_numbering_scope: super::NoteNumberingScope,
    endnote_section_map: HashMap<String, usize>,
    footnote_reserve_px: f64,
}

impl NoteMarkers {
    /// Marker text for every note `run` references, in run order. Empty when
    /// the run references nothing or note rendering is off.
    fn marker_text_for_run(&self, run: &Run) -> String {
        let mut text = String::new();
        for id in &run.footnote_reference_ids {
            if let Some(marker) = self.footnotes.get(id) {
                text.push_str(marker);
            }
        }
        for id in &run.endnote_reference_ids {
            if let Some(marker) = self.endnotes.get(id) {
                text.push_str(marker);
            }
        }
        text
    }
}

impl LayoutConfig {
    /// The line height a cell paragraph gets from the shared resolution, or
    /// the empty-paragraph line when the prepass did not see it.
    fn line_height_for(&self, paragraph: &Paragraph) -> f64 {
        let key = paragraph as *const Paragraph as usize;
        self.paragraph_line_heights
            .get(&key)
            .copied()
            .unwrap_or(self.line_height)
    }

    fn paragraph_style(&self, paragraph: &Paragraph) -> (f64, ParagraphStyle) {
        let authored = paragraph
            .properties
            .as_ref()
            .and_then(|properties| properties.snap_to_grid);
        let enabled = jade_docx_layout::line_metrics::resolve_effective_snap_to_grid(
            authored,
            self.document_snap_to_grid_default,
            self.doc_grid.as_ref(),
        );
        let mut style = ParagraphStyle::default();
        if let Some(alignment) = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.alignment.as_ref())
        {
            style.alignment = alignment.to_string();
        }
        if let Some(rules) = self
            .paragraph_east_asian_rules
            .get(&(paragraph as *const Paragraph as usize))
        {
            style.east_asian_rules = rules.clone();
            style.enable_line_break_rules = rules.use_east_asian_layout;
        }
        style.word_east_asian_font_line_box =
            jade_docx_layout::line_metrics::word_east_asian_font_metrics_apply_on_grid(
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.spacing.as_ref()),
                self.document_uses_east_asian_layout,
                jade_docx_layout::line_metrics::word_grid_line_centres_text(
                    enabled,
                    self.doc_grid.as_ref(),
                ),
            );
        let text_alignment = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.text_alignment.as_deref());
        if matches!(text_alignment, None | Some("auto" | "baseline"))
            && jade_docx_layout::line_metrics::word_grid_line_centres_text(
                enabled,
                self.doc_grid.as_ref(),
            )
        {
            let base_size = paragraph
                .runs
                .iter()
                .filter(|run| !run.suppresses_line_height_contribution())
                .filter_map(|run| run.properties.as_ref().and_then(|props| props.font_size))
                .reduce(f32::max)
                .or_else(|| {
                    self.styles_part
                        .as_ref()
                        .and_then(|styles| styles.doc_default_font_size())
                })
                .unwrap_or(12.0);
            style.word_grid_line_sizing = jade_docx_layout::line_metrics::WordGridLineSizing::new(
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.spacing.as_ref()),
                enabled,
                self.doc_grid.as_ref(),
                None,
                base_size,
            );
        }
        if enabled
            && jade_docx_layout::line_metrics::doc_grid_has_active_char_grid(self.doc_grid.as_ref())
        {
            style.doc_grid_snap_to_chars = true;
            style.doc_grid_char_space_px = self
                .doc_grid
                .as_ref()
                .and_then(|grid| grid.char_space)
                .map(points_to_px);
        }
        let mut left_inset = 0.0;
        if let Some(properties) = &paragraph.properties {
            style.bidi = properties.bidi.unwrap_or(false);
            if let Some(indent) = &properties.indent {
                let char_unit =
                    jade_docx_layout::text_extract::paragraph_indent_char_unit_pt_with_grid(
                        paragraph,
                        self.styles_part.as_deref(),
                        self.doc_grid.as_ref(),
                        enabled,
                    );
                apply_paragraph_indent_with_char_unit(
                    &mut left_inset,
                    &mut style,
                    indent,
                    properties
                        .numbering
                        .as_ref()
                        .and_then(Numbering::list_id)
                        .is_some(),
                    char_unit,
                );
            }
        }
        (left_inset, style)
    }
}

struct TableLineWrapContext<'a, P: Painter> {
    context: &'a P,
    style: Rc<ResolvedRunStyle>,
    cache: Rc<RefCell<TextMeasureCache>>,
    /// Memoizes trailing ink overhang (keyed by a hash of font descriptor + text)
    /// so repeated graphemes in a table cell avoid redundant `measureText`
    /// boundary calls during wrapping.
    trailing_overhang_cache: Rc<RefCell<FxHashMap<u64, f64>>>,
    /// One render's short ASCII widths. Repeated wrap passes otherwise touch
    /// the shared LRU for every letter and pair, even when all widths are warm.
    /// Dropping the context between renders also drops these entries, so font
    /// load invalidation still reaches the persistent measurement cache.
    ascii_measure_memo: RefCell<FxHashMap<(u32, u32), f64>>,
    wrap_cache: Option<Rc<RefCell<ParagraphWrapCache>>>,
}

impl<'a, P: Painter> TableLineWrapContext<'a, P> {
    /// A context with caches of its own, for one-off cell measurement.
    fn new(context: &'a P) -> Self {
        Self::with_caches(
            context,
            Rc::new(RefCell::new(TextMeasureCache::default())),
            Rc::new(RefCell::new(FxHashMap::default())),
        )
    }

    /// A context measuring into caches the caller keeps between tables.
    fn with_caches(
        context: &'a P,
        cache: Rc<RefCell<TextMeasureCache>>,
        trailing_overhang_cache: Rc<RefCell<FxHashMap<u64, f64>>>,
    ) -> Self {
        let mut result = Self {
            context,
            style: default_table_run_style(),
            cache,
            trailing_overhang_cache,
            ascii_measure_memo: RefCell::new(FxHashMap::default()),
            wrap_cache: None,
        };
        result.style = Rc::new(result.attach_font_ids((*result.style).clone()));
        result
    }

    fn attach_font_ids(&self, style: ResolvedRunStyle) -> ResolvedRunStyle {
        let interner = self.cache.borrow().interner();
        let mut interner = interner.borrow_mut();
        let latin_ascii = interner.intern(&style.font_descriptor_latin_ascii);
        let latin_hansi = interner.intern(&style.font_descriptor_latin_hansi);
        let east_asia = interner.intern(&style.font_descriptor_east_asia);
        let complex = interner.intern(&style.font_descriptor_complex);
        style.with_font_descriptor_ids(latin_ascii, latin_hansi, east_asia, complex)
    }

    fn set_font(&self, font_descriptor: &str) {
        self.context.set_font(font_descriptor);
    }
}

// Hash serialized inputs directly into the key without building a JSON buffer.
struct TableWrapHashWriter(FxHasher);

impl std::io::Write for TableWrapHashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<P: Painter<Error: PainterError>> TableLineWrapContext<'_, P> {
    fn wrap_horizontal(
        &self,
        text: &str,
        run_styles: &[RunStyle],
        paragraph_style: &ParagraphStyle,
        base_offset: usize,
        max_width: f64,
    ) -> Result<Rc<[WrappedLine]>, JsValue> {
        // Offset-specific oracle widths retain the uncached path. Table
        // placement coordinates do not enter this content/width cache.
        let cache = self
            .wrap_cache
            .as_ref()
            .filter(|_| paragraph_style.oracle_advance_px_by_text_offset.is_none());
        let key = cache.and_then(|_| {
            let mut writer = TableWrapHashWriter(FxHasher::default());
            text.hash(&mut writer.0);
            for run in run_styles {
                if serde_json::to_writer(
                    &mut writer,
                    &(
                        run.start,
                        run.end,
                        run.properties.as_deref(),
                        run.revision.as_ref(),
                        run.hyperlink.as_ref(),
                    ),
                )
                .is_err()
                {
                    return None;
                }
                run.inline_object
                    .map(|object| (object.width.to_bits(), object.height.to_bits()))
                    .hash(&mut writer.0);
            }
            let mut style_hasher = FxHasher::default();
            CanvasRenderer::<P>::compute_wrap_style_hash(paragraph_style).hash(&mut style_hasher);
            // Preserve exact widths/indents instead of the body's centipixel
            // key quantization; even tiny changes can cross a wrap boundary.
            for value in [
                max_width.max(1.0),
                paragraph_style.first_line_indent,
                paragraph_style.first_line_available_adjustment,
                paragraph_style.continuation_available_adjustment,
                paragraph_style.right_indent,
                paragraph_style.default_tab_width,
                paragraph_style.tab_origin_offset,
                paragraph_style.east_asian_advance_scale,
            ] {
                value.to_bits().hash(&mut style_hasher);
            }
            paragraph_style
                .use_hanging_indent_tab_stop
                .hash(&mut style_hasher);
            for stop in &paragraph_style.tab_stops {
                stop.position_pt.to_bits().hash(&mut style_hasher);
            }
            if let Some(sizing) = &paragraph_style.word_grid_line_sizing {
                format!("{sizing:?}").hash(&mut style_hasher);
            }
            if let Some(sizing) = &paragraph_style.word_proportional_line_sizing {
                format!("{sizing:?}").hash(&mut style_hasher);
            }
            let mut key = ParagraphWrapCacheKey::new(
                writer.0.finish(),
                0.0,
                max_width.max(1.0),
                paragraph_style.first_line_indent,
                paragraph_style.right_indent,
                style_hasher.finish(),
            );
            key.table_context = true;
            key.base_offset = base_offset;
            Some(key)
        });
        if let (Some(cache), Some(key)) = (cache, key.as_ref()) {
            if let Some(lines) = cache.borrow_mut().get(key) {
                return Ok(lines);
            }
        }
        let mut lines = jade_docx_layout::line_wrap::wrap_text_into_lines(
            self,
            text,
            run_styles,
            0.0,
            paragraph_style,
            base_offset,
            max_width.max(1.0),
        )?;
        justify_table_wrapped_lines(&mut lines, max_width, paragraph_style);
        let lines: Rc<[WrappedLine]> = lines.into();
        if let (Some(cache), Some(key)) = (cache, key) {
            cache.borrow_mut().insert(key, Rc::clone(&lines));
        }
        Ok(lines)
    }
}

fn justify_table_wrapped_lines(lines: &mut [WrappedLine], max_width: f64, style: &ParagraphStyle) {
    if jade_docx_layout::line_wrap::is_justified_alignment(&style.alignment) {
        for (index, line) in lines.iter_mut().enumerate() {
            line.justify_word_spaces_to_width(
                (max_width - style.line_indent(index) - style.right_indent).max(1.0),
            );
        }
    }
}

fn layout_cached_horizontal_table_paragraph<P: Painter<Error: PainterError>>(
    context: &TableLineWrapContext<'_, P>,
    text: &str,
    max_width: f64,
    line_height: f64,
    run_styles: &[RunStyle],
    style: &ParagraphStyle,
) -> Result<CellParagraph, JsValue> {
    let lines = context.wrap_horizontal(text, run_styles, style, 0, max_width)?;
    Ok(horizontal_cell_paragraph_from_wrapped_lines(
        text,
        line_height,
        style,
        0,
        &lines,
    ))
}

// The renderer reports `JsValue` throughout, so the wrap contexts do too
// rather than leaking the backend's error type outward: with the painter now a
// type parameter, `P::Error` here would make every caller generic over an error
// they cannot construct. `PainterError` converts at the boundary instead.
impl<P: Painter<Error: PainterError>> TextMeasurer for TableLineWrapContext<'_, P>
where
    P::Error: PainterError,
{
    type Error = JsValue;

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        self.set_font(font_descriptor);
        self.context
            .measure_text_width(text)
            .map_err(PainterError::into_js_value)
    }
}

impl<P: Painter<Error: PainterError>> LineWrapContext for TableLineWrapContext<'_, P>
where
    P::Error: PainterError,
{
    type Error = JsValue;

    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        if properties.is_some() {
            Ok(Rc::new(self.attach_font_ids(
                (*resolve_run_style_for_wrap(properties, None)).clone(),
            )))
        } else {
            Ok(self.style.clone())
        }
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let mut cache = self.cache.borrow_mut();
        cache.measure_with(self, text, font_descriptor)
    }

    #[inline]
    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        let mut cache = self.cache.borrow_mut();
        cache.measure_with_hash(self, text, font_descriptor, font_descriptor_hash)
    }

    #[inline]
    fn measure_text_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        let memo_key = font_descriptor_id.and_then(|font_id| {
            let bytes = text.as_bytes();
            (text.is_ascii() && (1..=2).contains(&bytes.len())).then(|| {
                let text_key = ((bytes.len() as u32) << 16)
                    | ((bytes[0] as u32) << 8)
                    | u32::from(*bytes.get(1).unwrap_or(&0));
                (font_id, text_key)
            })
        });
        if let Some(key) = memo_key {
            if let Some(width) = self.ascii_measure_memo.borrow().get(&key).copied() {
                return Ok(width);
            }
        }
        let mut cache = self.cache.borrow_mut();
        let width = if let Some(font_id) = font_descriptor_id {
            cache.measure_with_id(self, text, font_descriptor, font_id)
        } else {
            cache.measure_with_hash(self, text, font_descriptor, font_descriptor_hash)
        }?;
        if let Some(key) = memo_key {
            let mut memo = self.ascii_measure_memo.borrow_mut();
            if memo.len() < TABLE_ASCII_MEASURE_MEMO_CAPACITY {
                memo.insert(key, width);
            }
        }
        Ok(width)
    }

    #[inline]
    fn measure_text_for_vertical_stack_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        self.measure_text_with_font_key(
            text,
            font_descriptor,
            font_descriptor_hash,
            font_descriptor_id,
        )
    }

    fn measure_trailing_overhang(
        &self,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        let mut hasher = FxHasher::default();
        font_descriptor.hash(&mut hasher);
        text.hash(&mut hasher);
        let key = hasher.finish();
        if let Some(overhang) = self.trailing_overhang_cache.borrow().get(&key).copied() {
            return Ok(overhang);
        }

        self.set_font(font_descriptor);
        let metrics = self
            .context
            .measure_text_metrics(text)
            .map_err(PainterError::into_js_value)?;
        let advance = {
            let mut cache = self.cache.borrow_mut();
            cache.measure_with(self, text, font_descriptor)?
        };
        let overhang = metrics.actual_bounding_box_right - advance;
        let overhang = if overhang.is_finite() && overhang > 0.0 {
            overhang
        } else {
            0.0
        };
        self.trailing_overhang_cache
            .borrow_mut()
            .insert(key, overhang);
        Ok(overhang)
    }
}

impl<HostPainter> CanvasRenderer<HostPainter>
where
    HostPainter: Painter,
    HostPainter::Error: PainterError,
{
    /// Renders a DOCX table onto the current canvas, carrying rows across pages when needed.
    ///
    /// The routine mirrors the HTML renderer’s layout logic: column widths are derived from
    /// explicit `tblGrid` information when present, or fall back to an even distribution. Each
    /// row is measured before drawing; if a row cannot fit in the remaining space, the renderer
    /// tries to split the row at cell block boundaries and continues on the next page, matching
    /// Word's behavior for splittable rows.
    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    /// Registers the notes referenced from `elements` (nested tables
    /// included) in document order, at the current page and section, exactly
    /// as a body paragraph registers its references when it is laid out, and
    /// records the rendered marks in `markers`.
    pub(super) fn register_block_note_markers(
        &mut self,
        elements: &[BodyElement],
        options: &Options,
        markers: &mut NoteMarkers,
    ) {
        for element in elements {
            match element {
                BodyElement::Paragraph(paragraph) => {
                    for run in &paragraph.runs {
                        if options.render_footnotes {
                            for id in &run.footnote_reference_ids {
                                let text = self.register_note_reference(true, id);
                                if markers.retain_ordinals
                                    && self
                                        .note_restart_for_section(true, self.active_section_index)
                                        != NoteRestart::EachPage
                                {
                                    if let Some(number) = self.footnote_display_map.get(id) {
                                        markers.footnote_ordinals.insert(id.clone(), *number);
                                    }
                                }
                                markers.footnotes.insert(id.clone(), text);
                            }
                        }
                        if options.render_endnotes {
                            for id in &run.endnote_reference_ids {
                                let text = self.register_note_reference(false, id);
                                if markers.retain_ordinals
                                    && self
                                        .note_restart_for_section(false, self.active_section_index)
                                        != NoteRestart::EachPage
                                {
                                    if let Some(number) = self.endnote_display_map.get(id) {
                                        markers.endnote_ordinals.insert(id.clone(), *number);
                                    }
                                }
                                markers.endnotes.insert(id.clone(), text);
                            }
                        }
                    }
                }
                BodyElement::Table(nested) => {
                    for row in &nested.rows {
                        for cell in &row.cells {
                            self.register_block_note_markers(&cell.blocks, options, markers);
                        }
                    }
                }
                BodyElement::Math(_) => {}
            }
        }
    }

    /// Marks for every note referenced from the table, numbered as if the
    /// whole table were laid out on the page where it starts. Rows are
    /// prepared before any of them is placed, so this is only a provisional
    /// layout input: nothing is registered (the note state is restored), and
    /// each row registers its notes for real once its page is known (see
    /// [`Self::register_table_row_notes`]).
    fn provisional_table_note_markers(&mut self, table: &Table, options: &Options) -> NoteMarkers {
        let mut markers = NoteMarkers {
            retain_ordinals: true,
            ..NoteMarkers::default()
        };
        if (!options.render_footnotes && !options.render_endnotes)
            || !table.rows.iter().any(table_row_has_note_references)
        {
            return markers;
        }
        let saved = self.note_registration_state();
        for row in &table.rows {
            for cell in &row.cells {
                self.register_block_note_markers(&cell.blocks, options, &mut markers);
            }
        }
        self.restore_note_registration_state(saved);
        markers
    }

    fn note_registration_state(&self) -> NoteRegistrationState {
        NoteRegistrationState {
            footnote_display_map: self.footnote_display_map.clone(),
            footnote_order: self.footnote_order.clone(),
            footnote_counter: self.footnote_counter,
            footnote_numbering_scope: self.footnote_numbering_scope,
            footnote_section_map: self.footnote_section_map.clone(),
            endnote_display_map: self.endnote_display_map.clone(),
            endnote_order: self.endnote_order.clone(),
            endnote_counter: self.endnote_counter,
            endnote_numbering_scope: self.endnote_numbering_scope,
            endnote_section_map: self.endnote_section_map.clone(),
            footnote_reserve_px: self.footnote_reserve_px,
        }
    }

    fn restore_note_registration_state(&mut self, state: NoteRegistrationState) {
        self.footnote_display_map = state.footnote_display_map;
        self.footnote_order = state.footnote_order;
        self.footnote_counter = state.footnote_counter;
        self.footnote_numbering_scope = state.footnote_numbering_scope;
        self.footnote_section_map = state.footnote_section_map;
        self.endnote_display_map = state.endnote_display_map;
        self.endnote_order = state.endnote_order;
        self.endnote_counter = state.endnote_counter;
        self.endnote_numbering_scope = state.endnote_numbering_scope;
        self.endnote_section_map = state.endnote_section_map;
        self.footnote_reserve_px = state.footnote_reserve_px;
    }

    /// Whether a row that fits the page as laid out must still move to the
    /// next page because the start of its footnotes would not fit below it
    /// (Word keeps a reference and its note on one page). Decided by
    /// registering the row's notes on trial and undoing that when it moves.
    fn table_row_moves_for_its_notes(
        &mut self,
        row: &TableRow,
        options: &Options,
        row_height: f64,
    ) -> Result<bool, JsValue> {
        if !self.tracks_footnote_placement() || self.at_page_body_top() {
            return Ok(false);
        }
        let saved = self.note_registration_state();
        let mut trial = NoteMarkers::default();
        self.register_table_row_notes(row, options, &mut trial);
        let decision = self.reserve_footnote_area(row_height)?;
        self.restore_note_registration_state(saved);
        Ok(decision == NoteAreaReserve::MoveReference)
    }

    /// Registers the notes referenced from one table row on the page the row
    /// is placed on (#294). Word numbers notes by reference position, so a
    /// row continuing on a later page takes that page's `eachPage` restart
    /// and its note bodies belong at that page's bottom, not at the bottom of
    /// the page where the table started. Returns whether any mark differs
    /// from the provisional one the row was prepared with.
    fn register_table_row_notes(
        &mut self,
        row: &TableRow,
        options: &Options,
        markers: &mut NoteMarkers,
    ) -> bool {
        let mut registered = NoteMarkers::default();
        for cell in &row.cells {
            self.register_block_note_markers(&cell.blocks, options, &mut registered);
        }
        let mut changed = false;
        for (id, text) in registered.footnotes {
            changed |= markers.footnotes.get(&id) != Some(&text);
            markers.footnotes.insert(id, text);
        }
        for (id, text) in registered.endnotes {
            changed |= markers.endnotes.get(&id) != Some(&text);
            markers.endnotes.insert(id, text);
        }
        changed
    }

    fn register_prepared_row_notes(
        &mut self,
        cells: &[PreparedCell],
        markers: &mut NoteMarkers,
    ) -> bool {
        let mut changed = false;
        for anchor in prepared_row_note_anchors(cells) {
            let restart = self.note_restart_for_section(anchor.footnote, self.active_section_index);
            let (counter, scope) = if anchor.footnote {
                (self.footnote_counter, self.footnote_numbering_scope)
            } else {
                (self.endnote_counter, self.endnote_numbering_scope)
            };
            let mut mark = self.register_note_reference(anchor.footnote, &anchor.id);
            if restart != NoteRestart::EachPage {
                if let Some(number) = anchor.ordinal {
                    let (map, current_counter, current_scope) = if anchor.footnote {
                        (
                            &mut self.footnote_display_map,
                            &mut self.footnote_counter,
                            self.footnote_numbering_scope,
                        )
                    } else {
                        (
                            &mut self.endnote_display_map,
                            &mut self.endnote_counter,
                            self.endnote_numbering_scope,
                        )
                    };
                    map.insert(anchor.id.clone(), number);
                    let previous_counter = match restart {
                        NoteRestart::Continuous => counter,
                        NoteRestart::EachSection if current_scope.section == scope.section => {
                            counter
                        }
                        _ => 0,
                    };
                    *current_counter = previous_counter.max(number);
                    mark = self.format_note_number(number, anchor.footnote);
                }
            }
            let target = if anchor.footnote {
                &mut markers.footnotes
            } else {
                &mut markers.endnotes
            };
            changed |= target.get(&anchor.id) != Some(&mark);
            target.insert(anchor.id.clone(), mark);
        }
        changed
    }

    pub(super) fn render_table_with_numbering(
        &mut self,
        table: &Table,
        options: &Options,
        numbering_part: Option<&NumberingPart>,
        numbering_counter: &mut NumberingCounter,
        _deleted_numbering_offsets: &mut HashMap<String, Vec<u32>>,
    ) -> Result<(), JsValue> {
        // 表の描画は本文と経路が分かれていて Options を持ち回っていない。
        // 描く関数から見えるように、ここで置いておく。
        self.table_render_changes.set(options.render_changes);
        self.table_show_formatting_marks
            .set(options.show_formatting_marks);
        if table.rows.is_empty() {
            return Ok(());
        }

        // Tables do not participate in the paragraph render cache, so the normal
        // cache-miss path that promotes pages to dirty never fires for them.
        // Explicitly promote the current page so incremental rendering after
        // table-resize commands correctly repaints the table content.
        if !self.pageless_mode {
            self.promote_page_to_dirty_for_current_render(self.current_page_number)?;
        }

        // Cell paragraphs resolve their line height through the same path as
        // body paragraphs (font size, spacing, section grid, FE multiplier);
        // a fixed table line would make an empty 10.5pt row taller than the
        // body line Word draws it as. Resolve before the table painter state
        // is configured, since the prepass may activate fonts on the host.
        // Without `w:adjustLineHeightInTable` the section's line grid pitch
        // does not reach cell lines (ECMA-376 §17.15.3.1), so the prepass and
        // the cell layout below both see the grid without its line pitch.
        let table_doc_grid = jade_docx_layout::line_metrics::doc_grid_for_table_cells(
            self.current_doc_grid.as_ref(),
            self.document_adjust_line_height_in_table,
        );
        let section_doc_grid =
            std::mem::replace(&mut self.current_doc_grid, table_doc_grid.clone());
        let prepass = self
            .resolve_empty_paragraph_line_height_points()
            .and_then(|empty| {
                self.collect_table_paragraph_line_heights(table)
                    .map(|heights| (points_to_px(empty), heights))
            });
        self.current_doc_grid = section_doc_grid;
        let (empty_paragraph_line_height, paragraph_line_heights) = prepass?;
        let paragraph_east_asian_rules = self.collect_table_paragraph_east_asian_rules(table);
        let line_height = empty_paragraph_line_height;

        // An owning painter, because table rendering needs `&mut self` for
        // layout bookkeeping while the painter and the wrap context are alive.
        let painter = self.host.painter();
        let _table_state_guard = StateGuard::new(&painter);
        self.configure_table_context();
        // Autofit and row layout measure many of the same glyph/font pairs.
        // Keep one cache/interner alive for the complete table, including its
        // nested tables, instead of rebuilding them for every paragraph. With
        // the autofit cache enabled the measurements also outlive this table,
        // under the same invalidation; disabling it restores per-table caches.
        // The wrap context measures through the same painter the table paints
        // on, so metrics and ink come from one backend.
        let mut table_wrap_context = if self.table_autofit_cache_enabled {
            TableLineWrapContext::with_caches(
                &painter,
                Rc::clone(&self.table_text_measure_cache),
                Rc::clone(&self.table_trailing_overhang_cache),
            )
        } else {
            TableLineWrapContext::new(&painter)
        };
        if self.table_autofit_cache_enabled {
            table_wrap_context.wrap_cache = Some(Rc::clone(&self.paragraph_wrap_cache));
        }

        let padding_x = table_default_horizontal_padding_px(
            table,
            jade_docx_layout::line_metrics::word_layout_platform(),
        );
        let padding_y = points_to_px(CELL_VERTICAL_PADDING_PT);
        let available_width = (self.page_width - self.margin_left - self.margin_right).max(1.0);
        // Notes referenced from cells get their marks here, before the cells
        // are laid out, so the mark takes part in wrapping and the note body
        // is drawn at the page bottom like one referenced from a body
        // paragraph. Without this a footnote inserted in a cell was invisible
        // and its reserved body paragraph had nowhere to render.
        let note_markers = self.provisional_table_note_markers(table, options);
        let mut layout_config = LayoutConfig {
            padding_x,
            padding_y,
            line_height: empty_paragraph_line_height,
            paragraph_line_heights,
            paragraph_east_asian_rules,
            available_width,
            render_changes: options.render_changes,
            render_original: options.render_original,
            note_markers,
            doc_grid: table_doc_grid,
            styles_part: self.styles_part.clone(),
            document_snap_to_grid_default: self.document_snap_to_grid_default,
            document_uses_east_asian_layout: self.document_east_asian_rules.use_east_asian_layout,
            word_modern_border_insets: word_modern_nested_border_frame_enabled(
                self.document_east_asian_rules.compatibility_mode,
            ),
        };

        let column_count = compute_column_count(table);
        let sizing_width = word_pre_2013_table_sizing_width_px(
            table,
            available_width,
            self.document_east_asian_rules.compatibility_mode,
        );
        let mut column_widths = compute_column_widths(table, column_count, sizing_width);
        let (table_width, mut natural_paragraph_layouts) =
            apply_table_layout_width_for_table_with_content(
                &table_wrap_context,
                table,
                &mut column_widths,
                word_autofit_width_limit_px(table, sizing_width),
                padding_x,
                &|paragraph| layout_config.line_height_for(paragraph),
                (self.table_autofit_cache_enabled && table_autofit_cache_supported(table))
                    .then_some(&self.table_autofit_cache),
                &|field| self.resolve_run_field(field),
            )?;

        let order_index = self.table_counter;
        self.table_counter += 1;
        let table_index = self.table_layouts.len();
        self.table_layouts.push(TableLayoutInfo {
            index: table_index,
            order: order_index,
            node_id: table.node_id.map(|id| id.raw()),
            column_widths: column_widths.clone(),
            row_heights: Vec::with_capacity(table.rows.len()),
        });

        let mut col_positions = Vec::with_capacity(column_count + 1);
        let mut position = 0.0;
        col_positions.push(position);
        for width in &column_widths {
            position += *width;
            col_positions.push(position);
        }

        let margin_top = table
            .style
            .get("margin-top")
            .and_then(|v| parse_length_to_px(v));
        if let Some(mt) = margin_top {
            self.ensure_space(mt)?;
            self.current_y += mt;
        }

        let table_x = compute_floating_table_x(
            self.margin_left,
            available_width,
            self.page_width,
            table_width,
            table,
            self.current_page_number,
        )
        .unwrap_or_else(|| {
            compute_table_x(
                self.margin_left,
                available_width,
                table_width,
                table,
                pre_2013_table_text_inset_px(
                    table,
                    self.document_east_asian_rules.compatibility_mode,
                ) - modern_leading_border_half_px(table, layout_config.word_modern_border_insets),
            )
        });
        let left_edge = table_x;
        let right_edge = table_x + table_width;

        let margin_bottom = table
            .style
            .get("margin-bottom")
            .and_then(|v| parse_length_to_px(v));

        let mut last_row_bottom: Option<f64> = None;

        let mut cell_order = 0usize;

        let mut prepared_rows: Vec<Vec<PreparedCell>> = Vec::with_capacity(table.rows.len());
        let mut row_heights: Vec<f64> = Vec::with_capacity(table.rows.len());
        let hidden_rows: Vec<bool> = table.rows.iter().map(table_row_is_hidden).collect();
        let total_row_count = hidden_rows.iter().filter(|hidden| !**hidden).count();
        let mut visible_row_index = 0usize;
        // `(height hint, visible row index)` each row was prepared with, so a
        // row whose note marks change at placement can be laid out again.
        let mut row_prepare_inputs: Vec<(Option<f64>, usize)> =
            Vec::with_capacity(table.rows.len());
        for (row, &hidden) in table.rows.iter().zip(&hidden_rows) {
            let row_height_hint = row.height.map(points_to_px);
            row_prepare_inputs.push((row_height_hint, visible_row_index));
            let row_layout = {
                prepare_row_layout(
                    &table_wrap_context,
                    row,
                    table,
                    &column_widths,
                    &layout_config,
                    row_height_hint,
                    visible_row_index,
                    total_row_count,
                    &mut natural_paragraph_layouts,
                    None,
                    &|field| self.resolve_run_field(field),
                )?
            };

            let content_height = row_layout
                .iter()
                .filter(|cell| !cell.skip_render)
                .map(|cell| cell.height)
                .fold(0.0_f64, f64::max);
            let mut row_height = resolve_row_height(
                content_height,
                layout_config.line_height + 2.0 * padding_y,
                row,
                &row_layout,
            );
            if hidden {
                row_height = 0.0;
            } else {
                visible_row_index += 1;
            }
            prepared_rows.push(row_layout);
            row_heights.push(row_height);
        }
        // The heights with every owner sizing its first row: a span that
        // crosses the page foot falls back to them below, so the owner's
        // text splits with that row instead of running past the page.
        let contained_row_heights = row_heights.clone();
        let has_merge_owners;
        // A `w:vMerge` owner spanning rows does not size its first row; the
        // rows take their other cells' heights and only what the owner needs
        // beyond all of them goes to the last row it spans.
        {
            let row_refs: Vec<&[PreparedCell]> = prepared_rows.iter().map(Vec::as_slice).collect();
            let spans = compute_vertical_merge_row_spans(&row_refs);
            let owners = merged_cell_owners(&row_refs, &spans);
            has_merge_owners = !owners.is_empty();
            if has_merge_owners {
                for (row_index, cells) in prepared_rows.iter().enumerate() {
                    if hidden_rows[row_index]
                        || !(0..cells.len()).any(|cell| spans_merged_owner(&spans[row_index], cell))
                    {
                        continue;
                    }
                    let content_height = cells
                        .iter()
                        .enumerate()
                        .filter(|(cell_index, cell)| {
                            !cell.skip_render && !spans_merged_owner(&spans[row_index], *cell_index)
                        })
                        .map(|(_, cell)| cell.height)
                        .fold(0.0_f64, f64::max);
                    row_heights[row_index] = resolve_row_height(
                        content_height,
                        layout_config.line_height + 2.0 * padding_y,
                        &table.rows[row_index],
                        cells,
                    );
                }
                grow_rows_for_vertical_merges(&mut row_heights, &owners);
            }
        }
        for cells in &mut prepared_rows {
            assign_nested_table_orders(cells, &mut self.table_counter);
        }
        resolve_table_shared_horizontal_border_conflicts(
            &mut prepared_rows,
            &hidden_rows,
            &column_widths,
            table.bidi_visual,
        );
        let contained_row_heights = has_merge_owners.then(|| {
            let mut rows = prepared_rows.clone();
            let mut heights = contained_row_heights;
            add_border_extents(table, &mut rows, &hidden_rows, &mut heights, false);
            heights
        });
        let last_row_bottom_border = add_border_extents(
            table,
            &mut prepared_rows,
            &hidden_rows,
            &mut row_heights,
            true,
        );
        let row_refs: Vec<&[PreparedCell]> = prepared_rows.iter().map(Vec::as_slice).collect();
        let row_spans = compute_vertical_merge_row_spans(&row_refs);

        // `w:tblHeader` (ECMA-376 Part 1 §17.4.50): the contiguous block of
        // header rows at the very top of the table is repeated at the top of
        // every continuation page. Snapshot the prepared header cells + heights
        // *before* the render loop mutates anything, so the repeat draws from a
        // pristine copy without disturbing selection / paragraph indices.
        let header_row_count = leading_header_row_count(&table.rows);
        let header_rows_snapshot: Vec<Vec<PreparedCell>> =
            prepared_rows[..header_row_count.min(prepared_rows.len())].to_vec();
        let header_heights_snapshot: Vec<f64> =
            row_heights[..header_row_count.min(row_heights.len())].to_vec();

        // Floating tables retain a logical position in the body but paint at
        // their tblpPr frame. Until paragraph wrapping around that frame is
        // available, keep a flow floor at the logical anchor so positioning a
        // table upward cannot pull following body content over earlier text.
        //
        // `w:tblOverlap` is pairwise: either table can forbid an intersection.
        // Resolve that rectangle collision here, after exact row heights are
        // known, without deriving any independent paragraph wrap boundaries.
        let is_floating_table = !self.pageless_mode && table_floats(table);
        let floating_table_frame = if is_floating_table {
            let table_height = row_heights.iter().sum::<f64>();
            let margin_height = (self.page_height - self.margin_top - self.margin_bottom).max(0.0);
            let logical_y = self.current_y;
            let requested_y = compute_floating_table_y(
                self.margin_top,
                margin_height,
                self.page_height,
                logical_y,
                table_height,
                table,
            )
            .unwrap_or(logical_y);
            let prevents_overlap = table
                .style
                .get("table-overlap")
                .is_some_and(|value| value.eq_ignore_ascii_case("never"));
            let requested_frame = FloatingTableFrame {
                page_number: self.current_page_number,
                left: table_x,
                right: table_x + table_width,
                top: requested_y,
                bottom: requested_y + table_height,
                prevents_overlap,
            };
            let resolved_y = resolve_floating_table_overlap_y(
                requested_frame,
                logical_y,
                &self.floating_table_frames,
            );
            self.current_y = resolved_y;
            Some((
                FloatingTableFrame {
                    page_number: self.current_page_number,
                    left: table_x,
                    right: table_x + table_width,
                    top: resolved_y,
                    bottom: resolved_y + table_height,
                    prevents_overlap,
                },
                logical_y,
            ))
        } else {
            None
        };

        let first_visible_row = hidden_rows.iter().position(|hidden| !hidden);
        let last_visible_row = hidden_rows.iter().rposition(|hidden| !hidden);
        let mut last_visible_row_index = None;
        for (row_index, prepared_row) in prepared_rows.iter().enumerate() {
            let row = &table.rows[row_index];
            if let Some(contained) = contained_row_heights
                .as_ref()
                .filter(|_| !self.pageless_mode)
            {
                let span = row_spans[row_index].iter().copied().max().unwrap_or(1);
                let end = (row_index + span).min(row_heights.len());
                if span > 1
                    && self.current_y + row_heights[row_index..end].iter().sum::<f64>()
                        > self.table_row_bottom_limit()
                {
                    row_heights[row_index..end].copy_from_slice(&contained[row_index..end]);
                }
            }
            let original_row_height = row_heights[row_index];
            if let Some(info) = self.table_layouts.get_mut(table_index) {
                info.row_heights.push(original_row_height);
            }
            if hidden_rows[row_index] {
                // Hidden rows still occupy document/model positions even
                // though Word gives them no visual box. Reserve their nested
                // paragraph indices so following visible content stays mapped
                // to the same editing surfaces.
                let hidden_paragraphs = row
                    .cells
                    .iter()
                    .map(|cell| count_body_paragraphs(&cell.blocks))
                    .sum::<usize>();
                self.paragraph_counter = self
                    .advance_counter_with_note_surfaces(self.paragraph_counter, hidden_paragraphs);
                continue;
            }
            last_visible_row_index = Some(row_index);

            let mut pending_cells = prepared_row.clone();
            let mut pending_height = original_row_height;
            let mut row_start = true;
            let mut row_notes_registered = (!options.render_footnotes && !options.render_endnotes)
                || !table_row_has_note_references(row);

            // Reserve the whole recursive row in document order before any
            // cell is split into page fragments. Painting order is not model order.
            let indices_preassigned = true;
            self.paragraph_counter = assign_row_paragraph_indices(
                &mut pending_cells,
                self.paragraph_counter,
                &self.footnote_surface_extras,
            );

            loop {
                // A row that fits but whose notes would not start below it
                // moves on with them (one page; it is then placed there).
                let move_for_notes = !row_notes_registered
                    && !self.pageless_mode
                    && self.current_y + pending_height <= self.table_row_bottom_limit()
                    && if !row_start && indices_preassigned {
                        let saved = self.note_registration_state();
                        let mut trial = layout_config.note_markers.clone();
                        self.register_prepared_row_notes(&pending_cells, &mut trial);
                        let decision = self.reserve_footnote_area(pending_height)?;
                        self.restore_note_registration_state(saved);
                        decision == NoteAreaReserve::MoveReference
                    } else {
                        self.table_row_moves_for_its_notes(row, options, pending_height)?
                    };
                if !self.pageless_mode {
                    // Rows stop above the page's footnote area (#294).
                    let bottom_limit = self.table_row_bottom_limit();
                    let mut available_height = (bottom_limit - self.current_y).max(0.0);
                    if move_for_notes {
                        // The reference-bearing whole row cannot be placed here.
                        // Try a shorter segment before moving all its content;
                        // the head's own references are then reserved below.
                        available_height =
                            available_height.min(pending_height - 2.0 * ROW_SPLIT_EPSILON);
                    }
                    let full_page_content_height =
                        (self.page_height - self.margin_top - self.margin_bottom).max(0.0);
                    let header_height =
                        repeated_header_height(&header_heights_snapshot, header_row_count);
                    let full_page_content_height =
                        if should_repeat_header_before_row(header_row_count, row_index, true)
                            && header_height <= full_page_content_height
                        {
                            full_page_content_height - header_height
                        } else {
                            full_page_content_height
                        };
                    // A merge owner can itself exceed a page. Its page
                    // fragments retain restart/span metadata; continuation
                    // cells remain non-rendering and must not block content.
                    let disallow_split = row_height_blocks_content_split(row, &pending_cells)
                        || cant_split_blocks_row_split(
                            row.cant_split,
                            pending_height,
                            full_page_content_height,
                        );

                    if pending_height > available_height
                        && !disallow_split
                        && available_height > MIN_ROW_HEIGHT_PX
                    {
                        let tail_has_header =
                            should_repeat_header_before_row(header_row_count, row_index, true)
                                && repeated_header_height(
                                    &header_heights_snapshot,
                                    header_row_count,
                                ) <= full_page_content_height;
                        let mut split = split_prepared_row_for_available_height(
                            &pending_cells,
                            available_height,
                            line_height,
                            full_page_content_height,
                            tail_has_header,
                        );
                        if split.as_ref().is_some_and(|split| {
                            !prepared_row_note_anchors(&split.head_cells).is_empty()
                        }) && !row_notes_registered
                            && indices_preassigned
                        {
                            let saved = self.note_registration_state();
                            let mut budget = available_height;
                            let mut tried = HashSet::new();
                            loop {
                                self.restore_note_registration_state(saved.clone());
                                let Some(candidate) = split.as_ref() else {
                                    break;
                                };
                                let references = prepared_row_note_anchors(&candidate.head_cells)
                                    .into_iter()
                                    .map(|anchor| (anchor.footnote, anchor.id.clone()))
                                    .collect::<Vec<_>>();
                                if !tried.insert((references.clone(), budget.to_bits())) {
                                    // No stable placement for this reference set and budget:
                                    // retain the whole-row fallback rather than register
                                    // a note on a page where its reference is absent.
                                    split = None;
                                    break;
                                }
                                let changed = self.register_prepared_row_notes(
                                    &candidate.head_cells,
                                    &mut layout_config.note_markers,
                                );
                                let decision = if references.iter().any(|reference| reference.0)
                                    && self.tracks_footnote_placement()
                                {
                                    self.reserve_footnote_area(candidate.head_height)?
                                } else {
                                    NoteAreaReserve::Fits(self.footnote_reserve_px)
                                };
                                if changed {
                                    let (hint, visible_index) = row_prepare_inputs[row_index];
                                    pending_height = refresh_prepared_row_text(
                                        &table_wrap_context,
                                        row,
                                        table,
                                        &column_widths,
                                        &layout_config,
                                        hint,
                                        visible_index,
                                        total_row_count,
                                        &mut pending_cells,
                                        &|field| self.resolve_run_field(field),
                                    )?;
                                }
                                if decision == NoteAreaReserve::MoveReference {
                                    budget =
                                        budget.min(candidate.head_height - 2.0 * ROW_SPLIT_EPSILON);
                                } else {
                                    budget = budget.min(
                                        (self.table_row_bottom_limit() - self.current_y).max(0.0),
                                    );
                                }
                                let next = split_prepared_row_for_available_height(
                                    &pending_cells,
                                    budget,
                                    line_height,
                                    full_page_content_height,
                                    tail_has_header,
                                );
                                let next_references = next.as_ref().map(|next| {
                                    prepared_row_note_anchors(&next.head_cells)
                                        .into_iter()
                                        .map(|anchor| (anchor.footnote, anchor.id.clone()))
                                        .collect::<Vec<_>>()
                                });
                                if decision != NoteAreaReserve::MoveReference
                                    && next_references.as_ref() == Some(&references)
                                {
                                    split = next;
                                    break;
                                }
                                split = next;
                            }
                            if split.is_none() {
                                self.restore_note_registration_state(saved);
                            }
                        }
                        if let Some(split) = split {
                            if !row_notes_registered && !indices_preassigned {
                                row_notes_registered = true;
                                let changed = self.register_table_row_notes(
                                    row,
                                    options,
                                    &mut layout_config.note_markers,
                                );
                                self.reserve_footnote_area_for_placed_content(line_height)?;
                                if changed {
                                    let (hint, visible_index) = row_prepare_inputs[row_index];
                                    pending_height = refresh_prepared_row_text(
                                        &table_wrap_context,
                                        row,
                                        table,
                                        &column_widths,
                                        &layout_config,
                                        hint,
                                        visible_index,
                                        total_row_count,
                                        &mut pending_cells,
                                        &|field| self.resolve_run_field(field),
                                    )?;
                                }
                                continue;
                            }
                            let mut head_cells = split.head_cells;
                            let slot_scale =
                                prepared_spacing_slot_scale(row, table, &head_cells, table_width);
                            self.render_prepared_table_row_segment(
                                &mut head_cells,
                                &prepared_rows,
                                row_index,
                                split.head_height,
                                row_start,
                                !self.pageless_mode
                                    && (self.current_y - self.margin_top).abs()
                                        <= ROW_SPLIT_EPSILON,
                                table_index,
                                table_x,
                                left_edge,
                                right_edge,
                                column_count,
                                &col_positions,
                                &column_widths,
                                &row_spans,
                                &row_heights,
                                line_height,
                                numbering_part,
                                numbering_counter,
                                &mut cell_order,
                                indices_preassigned,
                                table.bidi_visual,
                                table_spacing_background(row, table),
                                slot_scale,
                                row_grows_for_borders(row),
                            )?;
                            self.draw_spaced_table_frame_segment(
                                table,
                                row,
                                SpacedTableFrameSegment {
                                    x: left_edge,
                                    y: self.current_y,
                                    width: right_edge - left_edge,
                                    height: split.head_height,
                                    draw_top: row_start && first_visible_row == Some(row_index),
                                    draw_bottom: false,
                                },
                            )?;
                            self.current_y += split.head_height;
                            if self.force_table_page_break()? {
                                self.configure_table_context();
                                // The row was split across a page boundary: redraw
                                // the header block above the continued (tail)
                                // segment, just like the wholesale-advance path.
                                let full_page_content_height =
                                    (self.page_height - self.margin_top - self.margin_bottom)
                                        .max(0.0);
                                if should_repeat_header_before_row(
                                    header_row_count,
                                    row_index,
                                    true,
                                ) && repeated_header_height(
                                    &header_heights_snapshot,
                                    header_row_count,
                                ) <= full_page_content_height
                                {
                                    self.draw_repeated_header_rows(
                                        &header_rows_snapshot,
                                        &header_heights_snapshot,
                                        table,
                                        TableVisualGrid {
                                            table_x,
                                            column_positions: &col_positions,
                                            column_widths: &column_widths,
                                            bidi_visual: table.bidi_visual,
                                        },
                                        line_height,
                                    )?;
                                }
                            }
                            pending_cells = split.tail_cells;
                            pending_height = split.tail_height;
                            row_start = false;
                            continue;
                        }
                    }
                }

                let required_height = if move_for_notes {
                    f64::INFINITY
                } else {
                    pending_height
                };
                let mut segment_starts_page = !self.pageless_mode
                    && (self.current_y - self.margin_top).abs() <= ROW_SPLIT_EPSILON;
                if self.ensure_table_row_space(required_height)? {
                    self.configure_table_context();
                    segment_starts_page = true;
                    // A page boundary was crossed mid-table: redraw the header
                    // block at the top of the new page before this body row,
                    // provided the header actually fits a fresh page.
                    let full_page_content_height =
                        (self.page_height - self.margin_top - self.margin_bottom).max(0.0);
                    if should_repeat_header_before_row(header_row_count, row_index, true)
                        && repeated_header_height(&header_heights_snapshot, header_row_count)
                            <= full_page_content_height
                    {
                        self.draw_repeated_header_rows(
                            &header_rows_snapshot,
                            &header_heights_snapshot,
                            table,
                            TableVisualGrid {
                                table_x,
                                column_positions: &col_positions,
                                column_widths: &column_widths,
                                bidi_visual: table.bidi_visual,
                            },
                            line_height,
                        )?;
                        segment_starts_page = !self.pageless_mode
                            && (self.current_y - self.margin_top).abs() <= ROW_SPLIT_EPSILON;
                    } else if row_start && first_visible_row != Some(row_index) {
                        // A fresh page has no preceding row to reserve its
                        // opening rule. Repeated headers already provide it.
                        self.current_y += pending_cells
                            .iter()
                            .filter(|cell| !cell.skip_render)
                            .map(|cell| cell.top_border_extent())
                            .fold(0.0_f64, f64::max);
                    }
                }
                // The whole row lands on the current page (after any advance
                // above): its notes are numbered and drawn here.
                if !row_notes_registered {
                    let changed = if !row_start && indices_preassigned {
                        self.register_prepared_row_notes(
                            &pending_cells,
                            &mut layout_config.note_markers,
                        )
                    } else {
                        self.register_table_row_notes(row, options, &mut layout_config.note_markers)
                    };
                    self.reserve_footnote_area_for_placed_content(pending_height)?;
                    if changed {
                        let (hint, visible_index) = row_prepare_inputs[row_index];
                        pending_height = refresh_prepared_row_text(
                            &table_wrap_context,
                            row,
                            table,
                            &column_widths,
                            &layout_config,
                            hint,
                            visible_index,
                            total_row_count,
                            &mut pending_cells,
                            &|field| self.resolve_run_field(field),
                        )?;
                    }
                }
                let mut render_cells = pending_cells;
                let slot_scale =
                    prepared_spacing_slot_scale(row, table, &render_cells, table_width);
                let absolute_bottom = self.render_prepared_table_row_segment(
                    &mut render_cells,
                    &prepared_rows,
                    row_index,
                    pending_height,
                    row_start,
                    segment_starts_page,
                    table_index,
                    table_x,
                    left_edge,
                    right_edge,
                    column_count,
                    &col_positions,
                    &column_widths,
                    &row_spans,
                    &row_heights,
                    line_height,
                    numbering_part,
                    numbering_counter,
                    &mut cell_order,
                    indices_preassigned,
                    table.bidi_visual,
                    table_spacing_background(row, table),
                    slot_scale,
                    row_grows_for_borders(row),
                )?;
                self.draw_spaced_table_frame_segment(
                    table,
                    row,
                    SpacedTableFrameSegment {
                        x: left_edge,
                        y: self.current_y,
                        width: right_edge - left_edge,
                        height: pending_height,
                        draw_top: row_start && first_visible_row == Some(row_index),
                        draw_bottom: last_visible_row == Some(row_index),
                    },
                )?;
                self.current_y += pending_height;
                last_row_bottom = Some(absolute_bottom);
                break;
            }
        }

        if let Some(info) = self.table_layouts.get_mut(table_index) {
            if info.row_heights.len() < table.rows.len() {
                info.row_heights.resize(table.rows.len(), MIN_ROW_HEIGHT_PX);
            }
        }

        if let (Some(bottom), Some(row_index)) = (last_row_bottom, last_visible_row_index) {
            if !table.rows.is_empty() {
                self.table_row_handles.push(TableRowResizeHandle {
                    table_index,
                    row_index,
                    y: bottom,
                    left: table_x,
                    right: table_x + table_width,
                    kind: TableRowResizeHandleKind::TableBottomEdge,
                });
            }
        }

        let table_end_y = self.current_y;
        match margin_bottom {
            Some(mb) => {
                self.current_y += mb;
            }
            // The last row already ends below its bottom border, unless its
            // height is exact: Word for Mac hangs that border below an exact
            // row (`table-diagonal-borders`: text resumes 0.5pt below two
            // 60pt exact rows).
            None if last_row_bottom_border > 0.0 => {
                if jade_docx_layout::line_metrics::word_layout_platform()
                    == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
                    && last_visible_row_index
                        .and_then(|index| table.rows.get(index))
                        .is_some_and(|row| !row_grows_for_borders(row))
                {
                    self.current_y += last_row_bottom_border;
                }
            }
            // An authored grid does not invent an editing gap beneath an
            // omitted bottom edge. Following text starts at the actual end.
            None if last_visible_row_index
                .and_then(|index| table.rows.get(index))
                .is_some_and(|row| has_document_table_border_defaults(row, table)) => {}
            None => {
                self.current_y += DEFAULT_TABLE_AFTER_GAP_PX;
            }
        }

        if let Some((frame, logical_floor)) = floating_table_frame {
            if self.current_page_number == frame.page_number {
                // A floating table paints at its absolute frame but does not
                // consume the body cursor. Following paragraphs use the shared
                // jade-docx-layout exclusion plan to flow beside/below it.
                self.current_y = logical_floor;
                self.active_float_exclusions.push(PageFloatExclusion {
                    page_number: frame.page_number,
                    bounds: floating_table_text_exclusion(table, frame),
                });
                self.floating_table_frames.push(frame);
            } else if self.current_page_number > frame.page_number {
                // The rows broke across pages, so the body cursor already
                // follows the table's last row. Register what the table really
                // occupies on every page so a later `w:tblOverlap` check sees
                // the continuation, not an extent running off the first page.
                self.floating_table_frames
                    .extend(floating_table_page_frames(
                        frame,
                        self.current_page_number,
                        table_end_y,
                        self.margin_top,
                        self.page_height - self.margin_bottom,
                    ));
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn render_prepared_table_row_segment(
        &mut self,
        row_layout: &mut [PreparedCell],
        all_rows: &[Vec<PreparedCell>],
        row_index: usize,
        row_height: f64,
        row_start: bool,
        segment_starts_page: bool,
        table_index: usize,
        table_x: f64,
        left_edge: f64,
        right_edge: f64,
        column_count: usize,
        col_positions: &[f64],
        column_widths: &[f64],
        row_spans: &[Vec<usize>],
        row_heights: &[f64],
        line_height: f64,
        numbering_part: Option<&NumberingPart>,
        numbering_counter: &mut NumberingCounter,
        cell_order: &mut usize,
        indices_preassigned: bool,
        bidi_visual: bool,
        spacing_background: Option<&str>,
        slot_scale: f64,
        reserve_bottom: bool,
    ) -> Result<f64, JsValue> {
        let bands = HorizontalBorderBands::for_row(row_layout, reserve_bottom);
        let y_top = self.current_y;
        let absolute_top = self.page_offset_y + y_top;
        let absolute_bottom = absolute_top + row_height;

        // A hidden predecessor has no physical boundary to drag. The next
        // visible row starts immediately after the preceding visible box, but
        // the current handle contract assumes adjacent model indices; omit the
        // ambiguous handle rather than resizing a hidden row by accident.
        if row_start
            && row_index > 0
            && row_heights
                .get(row_index - 1)
                .is_some_and(|height| *height > ROW_SPLIT_EPSILON)
        {
            self.table_row_handles.push(TableRowResizeHandle {
                table_index,
                row_index: row_index - 1,
                y: absolute_top,
                left: left_edge,
                right: right_edge,
                kind: TableRowResizeHandleKind::BetweenRows,
            });
        }

        if column_count > 1 {
            for boundary in 1..column_count {
                if col_positions.get(boundary).is_some()
                    && !column_boundary_crosses_merged_cell(row_layout, boundary)
                {
                    self.table_handles.push(TableResizeHandle {
                        table_index,
                        column_index: boundary - 1,
                        x: visual_column_boundary_x(table_x, col_positions, boundary, bidi_visual),
                        top: absolute_top,
                        bottom: absolute_bottom,
                        kind: TableResizeHandleKind::BetweenColumns,
                        reverse_delta: bidi_visual,
                    });
                }
            }
        }
        if column_count > 0 {
            if let Some(last_offset) = col_positions.last() {
                self.table_handles.push(TableResizeHandle {
                    table_index,
                    column_index: visual_right_edge_column_index(column_count, bidi_visual),
                    x: table_x + *last_offset,
                    top: absolute_top,
                    bottom: absolute_bottom,
                    kind: TableResizeHandleKind::TableRightEdge,
                    reverse_delta: false,
                });
            }
        }

        // When the row pre-assigned its paragraph indices in document order
        // (`assign_row_paragraph_indices`), keep them: clearing here would force
        // the lazy per-segment renumbering that splitting interleaves out of
        // order. The legacy nested-table path still clears so it can lazily
        // assign during render.
        if !indices_preassigned {
            for cell in row_layout.iter_mut() {
                if cell.skip_render {
                    continue;
                }
                for block in &mut cell.blocks {
                    block.paragraph_indices.clear();
                }
            }
        }

        #[cfg(test)]
        {
            for cell in row_layout.iter() {
                println!(
                    "table_debug: col {} span {} blocks {} skip {}",
                    cell.column_index,
                    cell.span,
                    cell.blocks.len(),
                    cell.skip_render
                );
            }
        }

        if self.should_draw_current_page_for_render() {
            let painter = self.host.painter();
            paint_spacing_tail(
                &painter,
                spacing_background,
                table_x,
                y_top,
                right_edge - left_edge,
                row_height,
                slot_scale,
            );
        }

        for (cell_index, cell) in row_layout.iter_mut().enumerate() {
            let column_start = cell.column_index.min(column_widths.len());
            let column_end = column_start
                .saturating_add(cell.span.max(1))
                .min(column_widths.len());
            let x_start = visual_cell_x(
                table_x,
                col_positions,
                column_start,
                column_end,
                bidi_visual,
            );
            let x_start = table_x + (x_start - table_x) * slot_scale;
            let cell_width =
                column_widths[column_start..column_end].iter().sum::<f64>() * slot_scale;
            let row_span = row_spans
                .get(row_index)
                .and_then(|row| row.get(cell_index))
                .copied()
                .unwrap_or(1)
                .max(1);
            let row_span_end = (row_index + row_span).min(row_heights.len());
            let mut merged_row_heights = row_heights[row_index..row_span_end].to_vec();
            // The source row height includes all pages of a split owner.
            // Geometry on this page starts with the current fragment only.
            if let Some(first) = merged_row_heights.first_mut() {
                *first = row_height;
            }
            let merged_height = if row_span == 1 {
                row_height
            } else {
                merged_row_heights.iter().sum::<f64>()
            };
            let bottom_limit = if self.pageless_mode {
                None
            } else {
                Some(self.page_height - self.margin_bottom)
            };
            let draw_segment = if row_span == 1 {
                page_clipped_cell_segment(y_top, merged_height, bottom_limit)
            } else {
                page_clipped_vertical_merge_segment(y_top, &merged_row_heights, bottom_limit)
            };
            let cell_rect =
                spaced_cell_border_rect(x_start, y_top, cell_width, merged_height, cell);
            let draw_rect =
                spaced_cell_border_rect(x_start, y_top, cell_width, draw_segment.height, cell);

            if cell.skip_render {
                if self.should_draw_current_page_for_render()
                    && matches!(cell.vertical_merge.as_deref(), Some("continue"))
                {
                    let painter = self.host.painter();
                    let context = &painter;
                    // Opening rules may inset the first row on a fresh page.
                    // Preserve the physical page boundary decided before that
                    // inset so vertically merged cells still reopen their grid.
                    let page_segment_top = segment_starts_page;
                    let page_segment_bottom = bottom_limit.is_some_and(|limit| {
                        row_heights.get(row_index + 1).is_some_and(|next_height| {
                            y_top + row_height + next_height > limit + ROW_SPLIT_EPSILON
                        })
                    });
                    let border_sides = vertical_merge_continuation_border_sides(
                        all_rows,
                        row_index,
                        cell_index,
                        page_segment_top,
                        page_segment_bottom,
                    );
                    self.draw_vertical_merge_continuation_borders(
                        context,
                        CellBorderRect {
                            x: draw_rect.x,
                            y: draw_rect.y,
                            width: draw_rect.width,
                            height: draw_rect.height.min(row_height),
                        },
                        cell,
                        border_sides,
                    )?;
                }
                // In pre-assigned mode `assign_row_paragraph_indices` already
                // reserved this continuation cell's paragraphs, so advancing the
                // counter again here would double-count them.
                if !indices_preassigned {
                    self.paragraph_counter = self.advance_counter_with_note_surfaces(
                        self.paragraph_counter,
                        cell.logical_paragraph_count,
                    );
                }
                continue;
            }

            if self.should_draw_current_page_for_render() {
                let painter = self.host.painter();
                let context = &painter;
                let shading_rect = spaced_cell_shading_rect(draw_rect, cell);
                paint_cell_spacing_background(
                    context,
                    spacing_background,
                    CellBorderRect {
                        x: x_start,
                        y: y_top,
                        width: cell_width,
                        height: draw_segment.height,
                    },
                    shading_rect,
                );
                if let Some(bg) = &cell.background {
                    #[allow(deprecated)]
                    {
                        context.set_fill_style_str(bg);
                    }
                    context.fill_rect(
                        shading_rect.x,
                        shading_rect.y,
                        shading_rect.width,
                        shading_rect.height,
                    );
                }

                #[allow(deprecated)]
                {
                    context.set_stroke_style_str(BORDER_COLOR);
                }
                if draw_segment.clipped_at_bottom {
                    // Word closes every visible page fragment of a vertically
                    // merged cell. The continuation reopens with its top
                    // border on the next page, so the page-break boundary has
                    // a complete horizontal rule on both pages.
                    self.draw_vertical_merge_continuation_borders(
                        context,
                        CellBorderRect {
                            x: draw_rect.x,
                            y: draw_rect.y,
                            width: draw_rect.width,
                            height: draw_rect.height,
                        },
                        cell,
                        VerticalMergeContinuationBorderSides {
                            top: true,
                            bottom: true,
                        },
                    )?;
                } else {
                    self.draw_cell_border_rect(
                        context,
                        draw_rect.x,
                        draw_rect.y,
                        draw_rect.width,
                        draw_rect.height,
                        cell,
                        bands,
                    )?;
                }
            }

            let start_paragraph = self.paragraph_counter;
            self.render_cell_blocks(
                cell,
                cell_rect.x,
                cell_rect.y,
                cell_rect.width,
                cell_rect.height,
                line_height,
                numbering_part,
                numbering_counter,
                None,
                bands.content_bottom,
            )?;
            if !indices_preassigned {
                self.paragraph_counter = self
                    .paragraph_counter
                    .saturating_add(cell.hidden_end_marker_paragraph_count);
            }

            // Both index modes derive the cell's paragraphs from its own
            // paragraph blocks: in pre-assigned mode those carry the pinned
            // indices, and in the lazy mode `render_cell_blocks` pushes the
            // index it takes from the counter onto each block. The lazy path
            // used to claim the whole counter delta instead, which also swept
            // in the paragraphs of a nested table rendered inside the cell.
            // Recording them here overwrote the nested cell's own registration
            // (the outer cell is recorded last), so a caret inside a nested
            // table reported the outer cell and 表の削除 removed the outer
            // table.
            let paragraph_indices = if !cell.selection_paragraph_indices.is_empty() {
                cell.selection_paragraph_indices.clone()
            } else {
                collect_cell_paragraph_indices(cell)
            };
            debug_assert!(
                indices_preassigned
                    || paragraph_indices
                        .iter()
                        .all(|index| (start_paragraph..self.paragraph_counter).contains(index)),
                "lazy cell paragraph indices must come from this cell's render pass"
            );
            if !paragraph_indices.is_empty() {
                let layout = CellLayout {
                    paragraphs: paragraph_indices,
                    left: cell_rect.x,
                    right: cell_rect.x + cell_rect.width,
                    // Selection coordinates share the document-space origin
                    // of the recorded text lines, including this page's offset.
                    top: absolute_top + draw_rect.y - y_top,
                    bottom: absolute_top + draw_rect.y - y_top + draw_rect.height,
                    table_id: table_index,
                    row: row_index,
                    column: cell.column_index,
                    column_span: cell.span,
                    order: *cell_order,
                };
                self.selection.record_table_cell_fragment(layout);
            }
            *cell_order += 1;
        }

        Ok(absolute_bottom)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_cell_blocks(
        &mut self,
        cell: &mut PreparedCell,
        x_start: f64,
        y_top: f64,
        cell_width: f64,
        row_height: f64,
        line_height: f64,
        numbering_part: Option<&NumberingPart>,
        numbering_counter: &mut NumberingCounter,
        ancestor_clip: Option<(f64, f64)>,
        bottom_band: f64,
    ) -> Result<f64, JsValue> {
        let padding = cell.padding;
        let pad_left = padding.left;
        let pad_horizontal = padding.horizontal();
        let draw_enabled = self.should_draw_current_page_for_render();
        let projection_clip =
            if self.auxiliary_story_capture_mode == AuxiliaryStoryCaptureMode::Full {
                let local = (cell_width > 0.0
                    && row_height > 0.0
                    && y_top.is_finite()
                    && row_height.is_finite())
                .then_some((y_top, y_top + row_height));
                intersect_vertical_clips(local, ancestor_clip)
            } else {
                None
            };
        let line_projection_clip =
            projection_clip.filter(|_| cell_ink_projection_is_horizontal(cell.text_direction));
        // Named so the painter outlives the guard that borrows it, and owning so
        // the borrow is not of `self`.
        let clip_painter = self.host.painter();
        let _clip_guard = if draw_enabled {
            ClipGuard::new(&clip_painter, x_start, y_top, cell_width, row_height)
        } else {
            None
        };
        // Offset the content downward for `w:vAlign="center"/"bottom"`. The
        // leftover space is the cell's inner height minus the total content
        // height; `vertical_align_share` decides how much of it sits above the
        // content (top=0.0, center=0.5, bottom=1.0). See ECMA-376 §17.4.84.
        let content_height: f64 = cell.blocks.iter().map(|block| block.height).sum();
        let (text_top_border, text_bottom_border) =
            if !cell_ink_projection_is_horizontal(cell.text_direction)
                && cell.cell_spacing <= f64::EPSILON
            {
                (
                    cell.top_border_extent() * 0.5,
                    cell.bottom_border_extent() * 0.5,
                )
            } else {
                (0.0, 0.0)
            };
        let inner_height = (row_height
            - padding.top
            - padding.bottom
            - 2.0 * cell.cell_spacing
            - bottom_band
            - text_top_border
            - text_bottom_border)
            .max(0.0);
        let vertical_align_offset =
            ((inner_height - content_height) * cell.vertical_align_share).max(0.0);
        let mut block_top =
            y_top + cell.cell_spacing + padding.top + text_top_border + vertical_align_offset;
        let mut consumed_height = 0.0;

        for block in &mut cell.blocks {
            match &mut block.kind {
                CellBlockKind::Spacing => {
                    block_top += block.height;
                    consumed_height += block.height;
                }
                CellBlockKind::Paragraph(paragraph) => {
                    // Lines advance by the paragraph's own line, as laid out.
                    let line_height = paragraph.line_height;
                    let align = paragraph.align.unwrap_or(cell.align);
                    let text_color = cell.text_color.as_deref().unwrap_or(DEFAULT_TEXT_COLOR);
                    let paragraph_index = match block.paragraph_indices.first() {
                        Some(index) => *index,
                        None => {
                            let index = self.paragraph_counter;
                            self.paragraph_counter =
                                self.advance_counter_with_note_surfaces(index, 1);
                            block.paragraph_indices.push(index);
                            index
                        }
                    };

                    if !paragraph.is_continuation {
                        self.selection.reset_paragraph_registration(paragraph_index);
                        self.selection
                            .append_paragraph_text(paragraph_index, &paragraph.model_text());
                    } else {
                        self.selection.continue_paragraph_registration(
                            paragraph_index,
                            &paragraph.model_text(),
                            paragraph.model_offset(paragraph.segment_start),
                        );
                    }

                    // プラグインの装飾。本文の段落と同じ手順で引く。
                    // 表の中も文書の一部なので、ここを飛ばすと「表の中の変更
                    // だけ塗られない」ことになる。
                    let mut decorations =
                        self.decorations_for_paragraph_text(&paragraph.text, paragraph_index);
                    decorations.extend(
                        self.spelling_decorations(
                            &paragraph.text,
                            paragraph
                                .styled_runs
                                .iter()
                                .map(|r| (r.start, r.end, r.properties.as_ref())),
                            paragraph_index,
                        ),
                    );

                    if let Some(direction) = cell.text_direction {
                        // The cell's fill set the painter's colour; vertical
                        // text paints with the current fill, so it would come
                        // out in the shading colour and vanish into it.
                        if draw_enabled {
                            let first_run_color = paragraph
                                .styled_runs
                                .first()
                                .and_then(|run| run.properties.as_ref())
                                .and_then(|properties| properties.color.clone());
                            let color = self.resolve_text_fill_color(
                                first_run_color.as_deref().unwrap_or(text_color),
                            );
                            #[allow(deprecated)]
                            clip_painter.set_fill_style_str(color.as_ref());
                        }
                        if direction_requires_rotated_layout(direction) {
                            let mut base_y = block_top + paragraph.spacing_before;
                            if matches!(direction, TextDirection::BtLr | TextDirection::BtLrV) {
                                let inner_top = y_top + padding.top;
                                let inner_bottom = y_top + row_height - padding.bottom;
                                let available_height = (inner_bottom - inner_top).max(0.0);
                                let free_space = (available_height - paragraph.height).max(0.0);
                                base_y = match align {
                                    TextAlign::Left => inner_bottom,
                                    TextAlign::Center => {
                                        inner_top + free_space / 2.0 + paragraph.height
                                    }
                                    TextAlign::Right => inner_top + paragraph.height,
                                }
                                .min(inner_bottom)
                                .max(inner_top + paragraph.height.min(available_height));
                            }
                            let consumed = self.render_rotated_paragraph(
                                paragraph,
                                paragraph_index,
                                direction,
                                align,
                                x_start,
                                base_y,
                                cell_width,
                                pad_left,
                                line_height,
                                0,
                            )?;
                            let consumed = consumed.max(block.height);
                            block_top += consumed;
                            consumed_height += consumed;
                            continue;
                        }
                        if direction_requires_vertical_stack(direction) {
                            let consumed = self.render_vertical_paragraph(
                                paragraph,
                                paragraph_index,
                                direction,
                                align,
                                x_start,
                                block_top + paragraph.spacing_before,
                                cell_width,
                                pad_left,
                                line_height,
                                0,
                                &decorations,
                                false,
                            )?;
                            let consumed = consumed.max(block.height);
                            block_top += consumed;
                            consumed_height += consumed;
                            continue;
                        }
                    }

                    let line_height = paragraph.line_height;
                    let zero_flow_marker =
                        block.height <= f64::EPSILON && paragraph.text.is_empty();
                    let mut text_y = if zero_flow_marker {
                        (y_top + row_height - padding.bottom - line_height).max(y_top + padding.top)
                    } else {
                        block_top + paragraph.spacing_before
                    };
                    if paragraph.lines.is_empty() {
                        let line_top = text_y;
                        let baseline = line_top + line_height;
                        let absolute_top = self.page_offset_y + line_top;
                        let content_width = (cell_width - pad_horizontal).max(0.0);
                        let caret_start_x = match align {
                            TextAlign::Left => x_start + pad_left,
                            TextAlign::Center => x_start + pad_left + content_width / 2.0,
                            TextAlign::Right => x_start + pad_left + content_width,
                        };
                        let line_layout = SelectionLineLayout {
                            top: line_top,
                            absolute_top,
                            height: line_height,
                            baseline,
                            start_x: x_start,
                            cursor_end_x: (x_start + cell_width).max(x_start + 1.0),
                            allowed_trailing_overflow_width: 0.0,
                            content_top: line_top,
                            absolute_content_top: absolute_top,
                            content_height: line_height,
                            fragments: Rc::from([]),
                            break_range: Some((0, 0)),
                            tab_leaders: Rc::from([]),
                            tab_stops: Rc::from([]),
                            column_index: self.column_index,
                            vertical: false,
                            vertical_reversed: false,
                        };
                        self.draw_selection_highlight(paragraph_index, &line_layout)?;
                        self.selection.record_line_with_caret_start(
                            paragraph_index,
                            line_layout,
                            caret_start_x,
                        );
                        self.last_line_baseline = baseline;
                        text_y += line_height;
                    } else {
                        for line in paragraph.lines.iter() {
                            let line_height = paragraph.line_advance(line);
                            let line_top = text_y;
                            let text_x = line.aligned_start_x(
                                x_start + pad_left,
                                cell_width - pad_horizontal,
                                align,
                            );

                            if !line.text.is_empty() {
                                if matches!(cell.text_direction, Some(TextDirection::LrTbV)) {
                                    self.draw_lr_tbv_line(
                                        paragraph,
                                        line,
                                        CellLinePlacement {
                                            start_x: text_x,
                                            top: line_top,
                                            height: line_height,
                                        },
                                        text_color,
                                        &decorations,
                                    )?;
                                } else if draw_enabled {
                                    self.draw_horizontal_line_with_run_styles(
                                        paragraph,
                                        line,
                                        CellLinePlacement {
                                            start_x: text_x,
                                            top: line_top,
                                            height: line_height,
                                        },
                                        text_color,
                                        &decorations,
                                    )?;
                                }
                            }
                            if draw_enabled
                                && !matches!(cell.text_direction, Some(TextDirection::LrTbV))
                            {
                                self.draw_cell_line_pictures(
                                    paragraph,
                                    line,
                                    text_x,
                                    line_top,
                                    line_height,
                                    paragraph_index,
                                )?;
                            }

                            if draw_enabled {
                                self.draw_table_formatting_mark(
                                    line,
                                    line.visual_end_x(text_x),
                                    line_top,
                                    line_height,
                                )?;
                            }

                            let baseline =
                                line_top + line.word_baseline_offset.unwrap_or(line_height);
                            let absolute_top = self.page_offset_y + line_top;
                            let mut fragments = Vec::new();
                            let mut selection_line = SelectionLineLayout {
                                top: line_top,
                                absolute_top,
                                height: line_height,
                                baseline,
                                start_x: text_x,
                                cursor_end_x: line.visual_end_x(text_x),
                                allowed_trailing_overflow_width: 0.0,
                                content_top: line_top,
                                absolute_content_top: absolute_top,
                                content_height: line_height,
                                fragments: Rc::from([]),
                                break_range: line.break_range,
                                tab_leaders: Rc::from([]),
                                tab_stops: Rc::from([]),
                                column_index: self.column_index,
                                vertical: false,
                                vertical_reversed: false,
                            };

                            if line.text.is_empty() {
                                selection_line.start_x = x_start;
                                selection_line.cursor_end_x =
                                    (x_start + cell_width).max(selection_line.start_x + 1.0);
                            } else {
                                fragments =
                                    self.table_line_selection_fragments(paragraph, line, text_x);
                                selection_line.start_x = x_start;
                                selection_line.cursor_end_x =
                                    (x_start + cell_width).max(selection_line.start_x + 1.0);
                                if selection_line.cursor_end_x
                                    < text_x + line.visual_width().max(1.0)
                                {
                                    selection_line.cursor_end_x =
                                        text_x + line.visual_width().max(1.0);
                                }
                            }
                            paragraph.remap_selection_fragments(&mut fragments);
                            selection_line.fragments = fragments.into();
                            selection_line.break_range = line.break_range.map(|(start, end)| {
                                (paragraph.model_offset(start), paragraph.model_offset(end))
                            });

                            if self.auxiliary_story_capture_mode == AuxiliaryStoryCaptureMode::Full
                                && cell.text_direction == Some(TextDirection::LrTbV)
                            {
                                self.capture_lr_tbv_paint_line(
                                    paragraph_index,
                                    paragraph,
                                    line,
                                    &selection_line,
                                    text_x,
                                    projection_clip,
                                )?;
                            } else if let Some(clip) = line_projection_clip {
                                self.capture_clipped_table_line(
                                    paragraph_index,
                                    paragraph,
                                    line,
                                    &selection_line,
                                    clip,
                                )?;
                            }
                            self.draw_selection_highlight(paragraph_index, &selection_line)?;
                            if line.text.is_empty() && line.start == line.end {
                                self.selection.record_line_with_caret_start(
                                    paragraph_index,
                                    selection_line,
                                    text_x,
                                );
                            } else {
                                self.selection.record_line(paragraph_index, selection_line);
                            }
                            self.last_line_baseline = baseline;

                            text_y += line_height;
                        }
                    }

                    let consumed = if zero_flow_marker {
                        0.0
                    } else {
                        (text_y - block_top).max(block.height.max(line_height))
                    };
                    block_top += consumed;
                    consumed_height += consumed;
                }
                CellBlockKind::Table(nested_table) => {
                    let [border_left_inset, border_right_inset] = nested_border_frame_insets(
                        word_modern_nested_border_frame_enabled(
                            self.document_east_asian_rules.compatibility_mode,
                        ),
                        &cell.border_left,
                        &cell.border_right,
                    );
                    let content_width =
                        cell_width - pad_horizontal - border_left_inset - border_right_inset;
                    if content_width <= 0.0 {
                        let consumed = block.height.max(line_height);
                        block_top += consumed;
                        consumed_height += consumed;
                        continue;
                    }

                    let nested_height = self.render_nested_table(
                        nested_table,
                        numbering_part,
                        numbering_counter,
                        x_start + pad_left + border_left_inset,
                        block_top,
                        content_width,
                        projection_clip,
                    )?;

                    let consumed = nested_height.max(block.height.max(line_height));
                    block_top += consumed;
                    consumed_height += consumed;
                }
            }
        }

        Ok(consumed_height)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_vertical_paragraph(
        &mut self,
        paragraph: &CellParagraph,
        paragraph_index: usize,
        direction: TextDirection,
        align: TextAlign,
        x_start: f64,
        block_top: f64,
        cell_width: f64,
        padding_x: f64,
        line_height: f64,
        base_offset: usize,
        decorations: &[crate::plugins::DecoratedRange],
        section: bool,
    ) -> Result<f64, JsValue> {
        self.draw_vertical_paragraph(
            paragraph,
            Some((paragraph_index, base_offset)),
            direction,
            align,
            x_start,
            block_top,
            cell_width,
            padding_x,
            line_height,
            decorations,
            section,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_vertical_paragraph(
        &mut self,
        paragraph: &CellParagraph,
        selection_context: Option<(usize, usize)>,
        direction: TextDirection,
        align: TextAlign,
        x_start: f64,
        block_top: f64,
        cell_width: f64,
        padding_x: f64,
        line_height: f64,
        decorations: &[crate::plugins::DecoratedRange],
        section: bool,
    ) -> Result<f64, JsValue> {
        let base_offset = selection_context.map_or(0, |(_, offset)| offset);
        let painter = self.host.painter();
        let context = &painter;
        let _canvas_state_guard = StateGuard::new(context);
        context.set_text_baseline("top");
        let draw_enabled = self.should_draw_current_page_for_render();
        let column_flow = direction_column_flow(direction);
        let axis = direction_vertical_axis(direction);

        let mut fallback_layout_holder: Option<VerticalTextLayout> = None;
        let layout = if let Some(layout) = paragraph.vertical_layout.as_ref() {
            layout
        } else {
            fallback_layout_holder = Some(VerticalTextLayout {
                column_ranges: std::iter::once(0..paragraph.lines.len()).collect(),
                column_height: paragraph.height.max(line_height),
            });
            fallback_layout_holder.as_ref().unwrap()
        };
        let _ = &fallback_layout_holder;

        let column_width = paragraph.max_line_width.max(line_height).max(1.0);
        let column_count = layout.column_ranges.len().max(1);
        let total_width = column_count as f64 * column_width;
        let content_left = x_start + padding_x;
        let content_right = (x_start + cell_width - padding_x).max(content_left);
        let available_width = (content_right - content_left).max(0.0);
        let horizontal_free = (available_width - total_width).max(0.0);
        let mut base_x = content_left;
        match align {
            TextAlign::Left => {}
            TextAlign::Center => base_x += horizontal_free / 2.0,
            TextAlign::Right => base_x += horizontal_free,
        }

        let column_height = layout.column_height.max(line_height);
        let draw_rotated_visual = direction_draws_rotated_table_visual(direction, section);
        let turned_angle = if section {
            None
        } else {
            match direction {
                TextDirection::TbRl => Some(FRAC_PI_2),
                TextDirection::BtLr => Some(-FRAC_PI_2),
                _ => None,
            }
        };
        if draw_enabled && draw_rotated_visual && !paragraph.text.is_empty() {
            context.save();
            let font = paragraph_primary_font_descriptor(paragraph);
            context.set_font(font.as_deref().unwrap_or(TABLE_FONT));
            let text_scale = paragraph_primary_text_scale(paragraph);
            let rotated_x = rotated_visual_anchor_x(direction, base_x, content_right, line_height);
            let rotated_baseline_y =
                rotated_visual_baseline_y(direction, block_top, column_height, line_height);
            let _ = context.translate(rotated_x, rotated_baseline_y);
            let _ = context.rotate(direction_rotation_angle(direction));
            if (text_scale - 1.0).abs() > f64::EPSILON {
                let _ = context.scale(text_scale, 1.0);
            }
            context
                .fill_text(&paragraph.text, 0.0, 0.0)
                .map_err(PainterError::into_js_value)?;
            context.restore();
        }
        if draw_enabled && draw_rotated_visual && !decorations.is_empty() {
            let _guard = StateGuard::new(context);
            context
                .translate(
                    rotated_visual_anchor_x(direction, base_x, content_right, line_height),
                    rotated_visual_baseline_y(direction, block_top, column_height, line_height),
                )
                .map_err(PainterError::into_js_value)?;
            context
                .rotate(direction_rotation_angle(direction))
                .map_err(PainterError::into_js_value)?;
            // The rotated visual paints the full string; concatenate the shared
            // grapheme advances in that same order, including multibyte prefixes.
            let mut advance = 0.0;
            for line in paragraph.lines.iter() {
                self.draw_table_line_squiggles(
                    line,
                    CellLinePlacement {
                        start_x: advance,
                        top: 0.0,
                        height: line_height,
                    },
                    decorations,
                )?;
                advance += line.visual_width();
            }
        }
        let oracle_advances = selection_context.and_then(|(paragraph_index, _)| {
            self.word_oracle_paragraph_advances
                .borrow()
                .get(&paragraph_index)
                .cloned()
        });
        let oracle_line_width = |line: &LineLayout| {
            oracle_advances
                .as_ref()
                .and_then(|advances| advances.get(&line.start.saturating_add(base_offset)))
                .copied()
                .unwrap_or_else(|| line.visual_width())
        };

        for (col_idx, column_range) in layout.column_ranges.iter().enumerate() {
            let selection_column_start = selection_context.and_then(|(index, _)| {
                self.selection
                    .paragraphs()
                    .get(index)
                    .map(|p| p.lines.len())
            });
            let effective_index = match column_flow {
                ColumnFlow::LeftToRight => col_idx,
                ColumnFlow::RightToLeft => column_count - 1 - col_idx,
            };
            let column_x = base_x + (effective_index as f64) * column_width;
            // Turn the column once; per-glyph origins and font baselines stay
            // local to that transform, avoiding a save/rotate/restore per glyph.
            let _turned_column_guard = if let Some(angle) = turned_angle.filter(|_| draw_enabled) {
                let guard = StateGuard::new(context);
                let origin_y = match axis {
                    VerticalAxis::TopToBottom => block_top,
                    VerticalAxis::BottomToTop => block_top + column_height,
                };
                context
                    .translate(column_x, origin_y)
                    .map_err(PainterError::into_js_value)?;
                context.rotate(angle).map_err(PainterError::into_js_value)?;
                context.set_text_baseline("alphabetic");
                Some(guard)
            } else {
                None
            };
            let column = &paragraph.lines[column_range.clone()];
            let mut consumed_before = column.first().map_or(0.0, |line| line.left_inset);

            for line in column {
                // A vertical glyph advances by the same measured cell width
                // used by layout. Painting, selection, hit testing, and the
                // caret must walk the identical Y geometry; applying separate
                // visual/selection multipliers leaves the caret detached from
                // the final glyph.
                let vertical_cell_advance = |line: &LineLayout| {
                    if line.text.is_empty() {
                        line_height.max(1.0)
                    } else {
                        oracle_line_width(line).max(1.0)
                    }
                };
                let visual_width = oracle_line_width(line);
                let line_advance = vertical_cell_advance(line);
                let center_glyphs = !draw_rotated_visual
                    && turned_angle.is_none()
                    && line.word_baseline_offset.is_some();
                let visual_consumed_before = consumed_before;
                consumed_before += line_advance;
                let (line_top, line_bottom, mut baseline) = match axis {
                    VerticalAxis::TopToBottom => {
                        let top = block_top + visual_consumed_before;
                        let bottom = top + line_advance;
                        (top, bottom, bottom)
                    }
                    VerticalAxis::BottomToTop => {
                        let bottom = block_top + column_height - visual_consumed_before;
                        let top = bottom - line_advance;
                        (top, bottom, bottom)
                    }
                };
                if center_glyphs {
                    baseline = line_top + line.word_baseline_offset.unwrap_or(line_advance);
                }

                let visual_text_top = match axis {
                    VerticalAxis::TopToBottom => block_top + visual_consumed_before,
                    VerticalAxis::BottomToTop => {
                        block_top + column_height - visual_consumed_before - line_advance
                    }
                };
                // Alignment positions the column group once. Upright section
                // glyphs are centered within that pitch; painting and selection
                // use the same origin and shared glyph baseline.
                let draw_text_x = if turned_angle.is_some() {
                    column_x + line.word_baseline_offset.unwrap_or(column_width * 0.5)
                } else if center_glyphs {
                    column_x
                        + (column_width
                            - line
                                .glyphs
                                .iter()
                                .map(|glyph| glyph.x + glyph.width)
                                .fold(0.0_f64, f64::max))
                        .max(0.0)
                            * 0.5
                } else {
                    column_x
                };
                let paint_y = if turned_angle.is_some() {
                    let leading = line.glyphs.first().map_or(0.0, |glyph| glyph.x);
                    match axis {
                        VerticalAxis::TopToBottom => line_top + leading,
                        VerticalAxis::BottomToTop => line_bottom - leading,
                    }
                } else if center_glyphs {
                    baseline
                } else {
                    visual_text_top
                };

                // 縦中横: the group upright and across the column, centred,
                // and with 行の幅に合わせる squeezed to the column's width.
                let tate_chu_yoko = turned_angle
                    .is_none()
                    .then(|| paragraph_line_tate_chu_yoko(paragraph, line))
                    .flatten();
                if let Some((compress, font)) = tate_chu_yoko.as_ref() {
                    if !line.text.is_empty() && draw_enabled && !draw_rotated_visual {
                        context.set_font(font.as_deref().unwrap_or(TABLE_FONT));
                        context.set_text_baseline(if center_glyphs { "alphabetic" } else { "top" });
                        let width = context
                            .measure_text_width(&line.text)
                            .map_err(PainterError::into_js_value)?
                            .max(0.01);
                        // One character's width: the cell the group takes.
                        let fit = line_advance.max(1.0);
                        let squeeze = if *compress && width > fit {
                            fit / width
                        } else {
                            1.0
                        };
                        let drawn = width * squeeze;
                        let _guard = StateGuard::new(context);
                        context
                            .translate(column_x + (column_width - drawn) * 0.5, paint_y)
                            .map_err(PainterError::into_js_value)?;
                        context
                            .scale(squeeze, 1.0)
                            .map_err(PainterError::into_js_value)?;
                        context
                            .fill_text(&line.text, 0.0, 0.0)
                            .map_err(PainterError::into_js_value)?;
                    }
                } else if !line.text.is_empty() && draw_enabled && !draw_rotated_visual {
                    let font = paragraph_line_font_descriptor(paragraph, line);
                    context.set_font(font.as_deref().unwrap_or(TABLE_FONT));
                    if turned_angle.is_none() {
                        context.set_text_baseline(if center_glyphs { "alphabetic" } else { "top" });
                    }
                    let text_scale = paragraph_line_text_scale(paragraph, line);
                    if turned_angle.is_some() {
                        let local_x = match axis {
                            VerticalAxis::TopToBottom => paint_y - block_top,
                            VerticalAxis::BottomToTop => block_top + column_height - paint_y,
                        };
                        let local_y = match axis {
                            VerticalAxis::TopToBottom => column_x - draw_text_x,
                            VerticalAxis::BottomToTop => draw_text_x - column_x,
                        };
                        if (text_scale - 1.0).abs() > f64::EPSILON {
                            let _guard = StateGuard::new(context);
                            context
                                .translate(local_x, local_y)
                                .map_err(PainterError::into_js_value)?;
                            context
                                .scale(text_scale, 1.0)
                                .map_err(PainterError::into_js_value)?;
                            context
                                .fill_text(&line.text, 0.0, 0.0)
                                .map_err(PainterError::into_js_value)?;
                        } else {
                            context
                                .fill_text(&line.text, local_x, local_y)
                                .map_err(PainterError::into_js_value)?;
                        }
                    } else if (text_scale - 1.0).abs() > f64::EPSILON {
                        context.save();
                        context
                            .translate(draw_text_x, paint_y)
                            .map_err(PainterError::into_js_value)?;
                        context
                            .scale(1.0, text_scale)
                            .map_err(PainterError::into_js_value)?;
                        context
                            .fill_text(&line.text, 0.0, 0.0)
                            .map_err(PainterError::into_js_value)?;
                        context.restore();
                    } else {
                        context
                            .fill_text(&line.text, draw_text_x, paint_y)
                            .map_err(PainterError::into_js_value)?;
                    }
                }

                if draw_enabled && !draw_rotated_visual && !decorations.is_empty() {
                    let _guard = StateGuard::new(context);
                    if turned_angle.is_some() {
                        let local_y = match axis {
                            VerticalAxis::TopToBottom => column_x - draw_text_x,
                            VerticalAxis::BottomToTop => draw_text_x - column_x,
                        };
                        context
                            .translate(visual_consumed_before, local_y)
                            .map_err(PainterError::into_js_value)?;
                    } else {
                        context
                            .translate(
                                column_x + column_width + points_to_px(12.0) * 0.20,
                                line_top,
                            )
                            .map_err(PainterError::into_js_value)?;
                        context
                            .rotate(FRAC_PI_2)
                            .map_err(PainterError::into_js_value)?;
                    }
                    self.draw_table_line_squiggles(
                        line,
                        CellLinePlacement {
                            start_x: 0.0,
                            top: 0.0,
                            height: 0.0,
                        },
                        decorations,
                    )?;
                }

                // Repeated headers share painting geometry without creating
                // another editable copy of the original paragraph.
                let Some((paragraph_index, _)) = selection_context else {
                    continue;
                };
                let mut selection_line = SelectionLineLayout {
                    top: line_top,
                    absolute_top: self.page_offset_y + line_top,
                    height: line_advance,
                    baseline: if turned_angle.is_some() {
                        paint_y
                    } else {
                        baseline
                    },
                    start_x: if turned_angle.is_some() {
                        column_x
                    } else {
                        draw_text_x
                    },
                    cursor_end_x: if turned_angle.is_some() {
                        column_x + column_width
                    } else {
                        draw_text_x + visual_width
                    },
                    allowed_trailing_overflow_width: 0.0,
                    content_top: line_top,
                    absolute_content_top: self.page_offset_y + line_top,
                    content_height: line_advance,
                    fragments: Rc::from([SelectionFragmentLayout {
                        baseline_shift_px: 0.0,
                        start: line.start.saturating_add(base_offset),
                        end: line.end.saturating_add(base_offset),
                        x: draw_text_x,
                        width: visual_width,
                        glyphs: Rc::from(
                            line.glyphs
                                .iter()
                                .enumerate()
                                .map(|(glyph_index, glyph)| GlyphBox {
                                    start: glyph.start.saturating_add(base_offset),
                                    end: glyph.end.saturating_add(base_offset),
                                    x: if turned_angle.is_some() { 0.0 } else { glyph.x },
                                    width: if line.glyphs.len() == 1 && glyph_index == 0 {
                                        visual_width
                                    } else {
                                        glyph.width
                                    },
                                })
                                .collect::<Vec<_>>(),
                        ),
                    }]),
                    break_range: Some((
                        line.start.saturating_add(base_offset),
                        line.end.saturating_add(base_offset),
                    )),
                    tab_leaders: Rc::from([]),
                    tab_stops: Rc::from([]),
                    // Vertically-stacked text: each line holds a single grapheme
                    // and the lines are stacked along Y, so caret/hit-test must
                    // resolve within the line by the click's Y position.
                    column_index: self.column_index,
                    vertical: true,
                    vertical_reversed: matches!(axis, VerticalAxis::BottomToTop),
                };
                if line.text.is_empty() {
                    selection_line.fragments = Rc::from([]);
                    selection_line.break_range = Some((
                        line.start.saturating_add(base_offset),
                        line.end.saturating_add(base_offset),
                    ));
                }

                if turned_angle.is_none() {
                    self.draw_selection_highlight(paragraph_index, &selection_line)?;
                }
                self.selection.record_line(paragraph_index, selection_line);
                self.last_line_baseline = line_bottom;
            }
            drop(_turned_column_guard);
            // Selection rectangles are in page coordinates. Paint them after
            // restoring the column transform, reusing the recorded lines.
            if turned_angle.is_some() {
                if let (Some((index, _)), Some(start)) = (selection_context, selection_column_start)
                {
                    if let Some(paragraph) = self.selection.paragraphs().get(index) {
                        for line in &paragraph.lines[start..] {
                            self.draw_selection_highlight(index, line)?;
                        }
                    }
                }
            }
        }

        Ok(column_height)
    }

    /// Redraws the table's leading header rows (`w:tblHeader`, ECMA-376 Part 1
    /// §17.4.50) at the current Y — the top of a continuation page — as a purely
    /// visual repeat. Unlike [`render_prepared_table_row_segment`] this touches
    /// **no** selection / paragraph-counter / cell-order / resize-handle state:
    /// the repeated header is decoration, not selectable content (matching
    /// Word, where the original header rows remain the editable cells). Advances
    /// `current_y` past every redrawn row regardless of whether the current page
    /// is actually being painted, so layout stays consistent across the dirty /
    /// incremental render paths.
    fn draw_repeated_header_rows(
        &mut self,
        header_rows: &[Vec<PreparedCell>],
        header_row_heights: &[f64],
        table: &Table,
        grid: TableVisualGrid<'_>,
        line_height: f64,
    ) -> Result<(), JsValue> {
        for (row_index, (row_cells, &row_height)) in header_rows
            .iter()
            .zip(header_row_heights.iter())
            .enumerate()
        {
            let reserve_bottom = table.rows.get(row_index).is_some_and(row_grows_for_borders);
            let bands = HorizontalBorderBands::for_row(row_cells, reserve_bottom);
            let spacing_background = table
                .rows
                .get(row_index)
                .and_then(|row| table_spacing_background(row, table));
            let slot_scale = table
                .rows
                .get(row_index)
                .map(|source| {
                    prepared_spacing_slot_scale(
                        source,
                        table,
                        row_cells,
                        *grid.column_positions.last().unwrap_or(&0.0),
                    )
                })
                .unwrap_or(1.0);
            let y_top = self.current_y;
            if self.should_draw_current_page_for_render() {
                let painter = self.host.painter();
                paint_spacing_tail(
                    &painter,
                    spacing_background,
                    grid.table_x,
                    y_top,
                    *grid.column_positions.last().unwrap_or(&0.0),
                    row_height,
                    slot_scale,
                );
                for cell in row_cells {
                    if cell.skip_render {
                        continue;
                    }
                    let end_column = (cell.column_index + cell.span).min(grid.column_widths.len());
                    let cell_width = grid.column_widths[cell.column_index..end_column]
                        .iter()
                        .sum::<f64>()
                        * slot_scale;
                    let x_start = visual_cell_x(
                        grid.table_x,
                        grid.column_positions,
                        cell.column_index,
                        end_column,
                        grid.bidi_visual,
                    );
                    let x_start = grid.table_x + (x_start - grid.table_x) * slot_scale;
                    let rect =
                        spaced_cell_border_rect(x_start, y_top, cell_width, row_height, cell);

                    let painter = self.host.painter();
                    let context = &painter;
                    let shading_rect = spaced_cell_shading_rect(rect, cell);
                    paint_cell_spacing_background(
                        context,
                        spacing_background,
                        CellBorderRect {
                            x: x_start,
                            y: y_top,
                            width: cell_width,
                            height: row_height,
                        },
                        shading_rect,
                    );
                    if let Some(bg) = &cell.background {
                        #[allow(deprecated)]
                        {
                            context.set_fill_style_str(bg);
                        }
                        context.fill_rect(
                            shading_rect.x,
                            shading_rect.y,
                            shading_rect.width,
                            shading_rect.height,
                        );
                    }
                    #[allow(deprecated)]
                    {
                        context.set_stroke_style_str(BORDER_COLOR);
                    }
                    self.draw_cell_border_rect(
                        context,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        cell,
                        bands,
                    )?;

                    self.draw_cell_text_visual_only(
                        cell,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        line_height,
                        bands.content_bottom,
                    )?;
                }
            }
            self.current_y += row_height;
        }
        Ok(())
    }

    /// Paints a repeated header cell with the ordinary glyph primitives and
    /// prepared layout, without recording another editable copy of the text.
    #[allow(clippy::too_many_arguments)]
    fn draw_cell_text_visual_only(
        &mut self,
        cell: &PreparedCell,
        x_start: f64,
        y_top: f64,
        cell_width: f64,
        row_height: f64,
        line_height: f64,
        bottom_band: f64,
    ) -> Result<(), JsValue> {
        let padding = cell.padding;
        let pad_left = padding.left;
        let pad_horizontal = padding.horizontal();
        let clip_painter = self.host.painter();
        let _clip_guard = ClipGuard::new(&clip_painter, x_start, y_top, cell_width, row_height);
        let text_color = cell.text_color.as_deref().unwrap_or(DEFAULT_TEXT_COLOR);
        #[allow(deprecated)]
        clip_painter.set_fill_style_str(text_color);

        let content_height: f64 = cell.blocks.iter().map(|block| block.height).sum();
        let inner_height =
            (row_height - padding.top - padding.bottom - 2.0 * cell.cell_spacing - bottom_band)
                .max(0.0);
        let vertical_align_offset =
            ((inner_height - content_height) * cell.vertical_align_share).max(0.0);
        let mut block_top = y_top + cell.cell_spacing + padding.top + vertical_align_offset;

        for block in &cell.blocks {
            if let CellBlockKind::Table(nested) = &block.kind {
                // Word repeats the whole header row, including a table nested
                // in one of its cells.
                let [border_left_inset, border_right_inset] = nested_border_frame_insets(
                    word_modern_nested_border_frame_enabled(
                        self.document_east_asian_rules.compatibility_mode,
                    ),
                    &cell.border_left,
                    &cell.border_right,
                );
                let content_width =
                    cell_width - pad_horizontal - border_left_inset - border_right_inset;
                if content_width > 0.0 {
                    self.draw_nested_table_visual_only(
                        nested,
                        x_start + pad_left + border_left_inset,
                        block_top,
                        content_width,
                    )?;
                }
                block_top += block.height.max(line_height);
                #[allow(deprecated)]
                clip_painter.set_fill_style_str(text_color);
                continue;
            }
            if let CellBlockKind::Paragraph(paragraph) = &block.kind {
                let line_height = paragraph.line_height;
                let align = paragraph.align.unwrap_or(cell.align);
                // 繰り返しの見出し行にも装飾を乗せる。段落番号はここでは
                // 割り当てられないので、キャッシュへの記録用に 0 を渡す。
                // 引くのは本文テキストなので、それで正しく当たる。
                let mut repeat_decorations =
                    self.decorations_for_paragraph_text(&paragraph.text, 0);
                repeat_decorations.extend(
                    self.spelling_decorations(
                        &paragraph.text,
                        paragraph
                            .styled_runs
                            .iter()
                            .map(|r| (r.start, r.end, r.properties.as_ref())),
                        0,
                    ),
                );
                if let Some(direction) = cell
                    .text_direction
                    .filter(|direction| direction_requires_vertical_stack(*direction))
                {
                    self.draw_vertical_paragraph(
                        paragraph,
                        None,
                        direction,
                        align,
                        x_start,
                        block_top + paragraph.spacing_before,
                        cell_width,
                        pad_left,
                        line_height,
                        &repeat_decorations,
                        false,
                    )?;
                    block_top += block.height;
                    continue;
                }
                let mut text_y = block_top + paragraph.spacing_before;
                // Repeated copies have a capture-local identity, never a body
                // selection index. The template remains the editable paragraph.
                let capture_paragraph = self.auxiliary_story_debug_lines.len();
                for (line_index, line) in paragraph.lines.iter().enumerate() {
                    let line_height = paragraph.line_advance(line);
                    let line_top = text_y;
                    let text_x = line.aligned_start_x(
                        x_start + pad_left,
                        cell_width - pad_horizontal,
                        align,
                    );
                    if !line.text.is_empty() {
                        if matches!(cell.text_direction, Some(TextDirection::LrTbV)) {
                            let _ = self.draw_lr_tbv_line(
                                paragraph,
                                line,
                                CellLinePlacement {
                                    start_x: text_x,
                                    top: line_top,
                                    height: line_height,
                                },
                                text_color,
                                &repeat_decorations,
                            )?;
                        } else {
                            self.draw_horizontal_line_with_run_styles(
                                paragraph,
                                line,
                                CellLinePlacement {
                                    start_x: text_x,
                                    top: line_top,
                                    height: line_height,
                                },
                                text_color,
                                &repeat_decorations,
                            )?;
                        }
                    }
                    if !matches!(cell.text_direction, Some(TextDirection::LrTbV)) {
                        self.draw_cell_line_pictures(
                            paragraph,
                            line,
                            text_x,
                            line_top,
                            line_height,
                            capture_paragraph,
                        )?;
                    }
                    if self.auxiliary_story_capture_mode.enabled()
                        && !line.text.is_empty()
                        && !matches!(cell.text_direction, Some(TextDirection::LrTbV))
                    {
                        let page = self.current_page_number.saturating_sub(1);
                        let baseline = line_top + line.word_baseline_offset.unwrap_or(line_height);
                        let fragments =
                            self.table_line_selection_fragments(paragraph, line, text_x);
                        self.auxiliary_story_debug_lines
                            .push(AuxiliaryStoryLineDebug {
                                story: "table_header",
                                page,
                                line: LayoutLineDebug {
                                    paragraph: capture_paragraph,
                                    line: line_index,
                                    page,
                                    column_index: self.column_index,
                                    vertical: false,
                                    start: line.start,
                                    end: line.end,
                                    text: line.text.clone(),
                                    top: line_top,
                                    absolute_top: self.page_offset_y + line_top,
                                    height: line_height,
                                    baseline,
                                    absolute_baseline: self.page_offset_y + baseline,
                                    report_x_offset: 0.0,
                                    start_x: text_x,
                                    end_x: line.visual_end_x(text_x),
                                    cursor_end_x: line.visual_end_x(text_x),
                                    visual_end_x: line.visual_end_x(text_x),
                                    allowed_trailing_overflow_width: 0.0,
                                    fragments: fragments
                                        .iter()
                                        .map(|fragment| LayoutFragmentDebug {
                                            start: fragment.start,
                                            end: fragment.end,
                                            x: fragment.x,
                                            width: fragment.width,
                                            baseline: baseline + fragment.baseline_shift_px,
                                            glyphs: fragment
                                                .glyphs
                                                .iter()
                                                .map(|glyph| LayoutGlyphDebug {
                                                    start: glyph.start,
                                                    end: glyph.end,
                                                    text: paragraph
                                                        .text
                                                        .get(glyph.start..glyph.end)
                                                        .unwrap_or("")
                                                        .to_string(),
                                                    x: glyph.x,
                                                    width: glyph.width,
                                                })
                                                .collect(),
                                        })
                                        .collect(),
                                    tab_leaders: Vec::new(),
                                    tab_stops: Vec::new(),
                                },
                            });
                    }
                    text_y += line_height;
                }
            }
            block_top += block.height;
        }
        Ok(())
    }

    /// Paints a nested table inside a repeated header cell from its prepared
    /// layout: shading, borders and text only. Like the repeated header itself
    /// it records no table layout, selection, paragraph index or resize handle,
    /// so the original nested table stays the only editable copy.
    fn draw_nested_table_visual_only(
        &mut self,
        layout: &NestedTableLayout,
        x_start: f64,
        y_start: f64,
        available_width: f64,
    ) -> Result<(), JsValue> {
        if layout.rows.is_empty() {
            return Ok(());
        }
        let is_floating_table = table_floats(&layout.table);
        let page_margin_width = (self.page_width - self.margin_left - self.margin_right).max(1.0);
        let mut table_x = compute_floating_table_x_in_frames(
            self.margin_left,
            page_margin_width,
            self.page_width,
            x_start,
            available_width,
            layout.width,
            &layout.table,
            self.current_page_number,
        )
        .unwrap_or_else(|| {
            compute_table_x(
                x_start,
                available_width,
                layout.width,
                &layout.table,
                // The parent cell already supplies its content-frame origin.
                0.0,
            )
        });
        if !is_floating_table && table_x + layout.width > x_start + available_width {
            table_x = (x_start + available_width - layout.width).max(x_start);
        }

        let row_refs: Vec<&[PreparedCell]> =
            layout.rows.iter().map(|row| row.cells.as_slice()).collect();
        let row_spans = compute_vertical_merge_row_spans(&row_refs);
        let row_heights: Vec<f64> = layout.rows.iter().map(|row| row.height).collect();
        let column_count = layout.column_widths.len();
        let mut y_top = y_start + layout.margin_top;
        for (row_index, row) in layout.rows.iter().enumerate() {
            if row.hidden {
                continue;
            }
            let reserve_bottom = layout
                .table
                .rows
                .get(row.source_index)
                .is_some_and(row_grows_for_borders);
            let bands = HorizontalBorderBands::for_row(&row.cells, reserve_bottom);
            let spacing_background = layout
                .table
                .rows
                .get(row.source_index)
                .and_then(|source| table_spacing_background(source, &layout.table));
            let slot_scale = layout
                .table
                .rows
                .get(row.source_index)
                .map(|source| {
                    prepared_spacing_slot_scale(source, &layout.table, &row.cells, layout.width)
                })
                .unwrap_or(1.0);
            if self.should_draw_current_page_for_render() {
                let painter = self.host.painter();
                paint_spacing_tail(
                    &painter,
                    spacing_background,
                    table_x,
                    y_top,
                    layout.width,
                    row.height,
                    slot_scale,
                );
            }
            for (cell_index, cell) in row.cells.iter().enumerate() {
                if cell.skip_render {
                    continue;
                }
                let column_start = cell.column_index.min(column_count);
                let column_end = column_start
                    .saturating_add(cell.span.max(1))
                    .min(column_count);
                let cell_x = visual_cell_x(
                    table_x,
                    &layout.column_positions,
                    column_start,
                    column_end,
                    layout.table.bidi_visual,
                );
                let cell_x = table_x + (cell_x - table_x) * slot_scale;
                let cell_width = layout.column_widths[column_start..column_end]
                    .iter()
                    .sum::<f64>()
                    * slot_scale;
                let span = row_spans
                    .get(row_index)
                    .and_then(|spans| spans.get(cell_index))
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                let height = row_heights[row_index..(row_index + span).min(row_heights.len())]
                    .iter()
                    .sum::<f64>();
                let rect = spaced_cell_border_rect(cell_x, y_top, cell_width, height, cell);
                {
                    let painter = self.host.painter();
                    let context = &painter;
                    let shading_rect = spaced_cell_shading_rect(rect, cell);
                    paint_cell_spacing_background(
                        context,
                        spacing_background,
                        CellBorderRect {
                            x: cell_x,
                            y: y_top,
                            width: cell_width,
                            height,
                        },
                        shading_rect,
                    );
                    if let Some(bg) = &cell.background {
                        #[allow(deprecated)]
                        context.set_fill_style_str(bg);
                        context.fill_rect(
                            shading_rect.x,
                            shading_rect.y,
                            shading_rect.width,
                            shading_rect.height,
                        );
                    }
                    #[allow(deprecated)]
                    context.set_stroke_style_str(BORDER_COLOR);
                    self.draw_cell_border_rect(
                        context,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        cell,
                        bands,
                    )?;
                }
                self.draw_cell_text_visual_only(
                    cell,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    layout.line_height,
                    bands.content_bottom,
                )?;
            }
            y_top += row.height;
        }
        Ok(())
    }

    fn draw_table_line_squiggles(
        &self,
        line: &LineLayout,
        placement: CellLinePlacement,
        decorations: &[crate::plugins::DecoratedRange],
    ) -> Result<(), JsValue> {
        let CellLinePlacement {
            start_x,
            top: line_top,
            height: line_height,
        } = placement;
        let painter = self.host.painter();
        let context = &painter;
        // The shared cell layout supplies every advance; painting does not measure spelling ranges.
        for decoration in decorations {
            let crate::plugins::DecorationStyle::Squiggly { color } = &decoration.style else {
                continue;
            };
            let mut spans: Vec<(f64, f64)> = Vec::new();
            for glyph in &line.glyphs {
                if glyph.end <= decoration.start || glyph.start >= decoration.end {
                    continue;
                }
                let x0 = start_x + glyph.x;
                let x1 = x0 + glyph.width;
                if let Some(last) = spans.last_mut() {
                    if (last.1 - x0).abs() < 1e-6 {
                        last.1 = x1;
                        continue;
                    }
                }
                spans.push((x0, x1));
            }
            for (x0, x1) in spans {
                self.draw_colored_squiggly(
                    context,
                    x0,
                    line_top + line_height * 0.8,
                    x1 - x0,
                    12.0,
                    color,
                )
                .map_err(PainterError::into_js_value)?;
            }
        }

        Ok(())
    }

    /// いま描こうとしている文字の下に、プラグインの装飾を敷く。
    ///
    /// 本文側 (`render_text_line`) と描画経路が分かれているため、同じことを
    /// ここでもやる必要がある。
    ///
    /// 位置はクラスタの座標ではなく、**文字を描くのに使う実測値**から出す。
    /// 以前はクラスタの `x` / `width` を使っていたが、あちらは文字の描画が
    /// 使う幅 (実測へ落ちることがある) と必ずしも一致せず、
    /// devicePixelRatio が 1 でない画面で帯が縮んだり横へずれたりしていた。
    ///
    /// `span_text` はこの一続きで描く文字、`span_start` はその段落内での
    /// 開始バイト位置、`x` / `width` は実際に描く場所と幅。
    #[allow(clippy::too_many_arguments)]
    fn draw_span_decorations<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        span_text: &str,
        span_start: usize,
        x: f64,
        width: f64,
        line_top: f64,
        line_height: f64,
        decorations: &[crate::plugins::DecoratedRange],
    ) {
        if decorations.is_empty() || span_text.is_empty() || width <= 0.0 {
            return;
        }
        let span_end = span_start + span_text.len();
        // 部分範囲の位置は前置きぶんを測って出す。測った合計が実際に描く幅と
        // ずれることがあるので、最後に width へ合わせて伸縮させる。
        let measured_total = context.measure_text_width(span_text).ok().unwrap_or(0.0);
        let ratio = if measured_total > f64::EPSILON {
            width / measured_total
        } else {
            0.0
        };
        let prefix_width = |upto: usize| -> Option<f64> {
            let prefix = span_text.get(..upto)?;
            if prefix.is_empty() {
                return Some(0.0);
            }
            context
                .measure_text_width(prefix)
                .ok()
                .map(|width| width * ratio)
        };

        // 表の描画は textBaseline を "top" にしている (この関数の呼び出し元が
        // fill_text へ渡す y は行の上端)。なので帯の上端はそのまま使える。
        // 本文側は "alphabetic" で、あちらは y がベースラインになる。
        for decoration in decorations {
            let crate::plugins::DecorationStyle::Highlight { color } = &decoration.style else {
                continue;
            };
            let range_start = decoration.start.max(span_start);
            let range_end = decoration.end.min(span_end);
            if range_end <= range_start {
                continue;
            }
            let (Some(x0), Some(x1)) = (
                prefix_width(range_start - span_start),
                prefix_width(range_end - span_start),
            ) else {
                continue;
            };
            if x1 <= x0 {
                continue;
            }
            context.save();
            context.set_fill_style_str(color);
            context.fill_rect(x + x0, line_top, x1 - x0, line_height);
            context.restore();
        }
    }

    /// Project only a release capture's painted glyphs. Selection keeps all
    /// source glyphs, including text hidden by an exact-height cell's clip.
    fn capture_clipped_table_line(
        &mut self,
        paragraph_index: usize,
        paragraph: &CellParagraph,
        line: &LineLayout,
        selection_line: &SelectionLineLayout,
        (clip_top, clip_bottom): (f64, f64),
    ) -> Result<(), JsValue> {
        let empty_clip = clip_bottom <= clip_top;
        if (!empty_clip && line.word_baseline_offset.is_none())
            || line.text.is_empty()
            || (!empty_clip
                && selection_line.top >= clip_top
                && selection_line.top + selection_line.height <= clip_bottom)
        {
            return Ok(());
        }
        let spans = if empty_clip {
            Vec::new()
        } else {
            line_style_spans(line.start, line.end, &paragraph.styled_runs)
        };
        // Effects can extend ink beyond measureText's box. Keep those glyphs
        // conservatively until their full effect bounds are available.
        if spans.iter().any(|span| {
            span.run_index
                .and_then(|index| paragraph.styled_runs.get(index))
                .and_then(|run| run.properties.as_ref())
                .is_some_and(|p| {
                    p.outline == Some(true)
                        || p.shadow == Some(true)
                        || p.emboss == Some(true)
                        || p.imprint == Some(true)
                })
        }) {
            return Ok(());
        }
        let painter = self.host.painter();
        let _state = StateGuard::new(&painter);
        painter.set_text_baseline("alphabetic");
        let mut span_index = 0;
        let mut active_span = None;
        let mut ink_cache = ClippedGlyphInkCache::default();
        let mut fragments = Vec::with_capacity(selection_line.fragments.len());
        for fragment in selection_line.fragments.iter() {
            let baseline = selection_line.baseline + fragment.baseline_shift_px;
            let mut glyphs = Vec::with_capacity(fragment.glyphs.len());
            for glyph in fragment.glyphs.iter() {
                if empty_clip {
                    continue;
                }
                while spans
                    .get(span_index)
                    .is_some_and(|span| span.end <= glyph.start)
                {
                    span_index += 1;
                }
                let text = paragraph.text.get(glyph.start..glyph.end).unwrap_or("");
                if active_span != Some(span_index) {
                    let span = spans.get(span_index);
                    let props = span
                        .and_then(|span| span.run_index)
                        .and_then(|index| paragraph.styled_runs.get(index))
                        .and_then(|run| run.properties.as_ref());
                    let span_text = span
                        .and_then(|span| paragraph.text.get(span.start..span.end))
                        .unwrap_or(text);
                    let font = table_font_descriptor_for_text(props, span_text);
                    painter.set_font(font.as_deref().unwrap_or(TABLE_FONT));
                    active_span = Some(span_index);
                    ink_cache.clear();
                }
                let (ascent, descent) = ink_cache.measure(text, || {
                    let metrics = painter
                        .measure_text_metrics(text)
                        .map_err(PainterError::into_js_value)?;
                    Ok::<_, JsValue>((
                        metrics.actual_bounding_box_ascent,
                        metrics.actual_bounding_box_descent,
                    ))
                })?;
                if glyph_ink_intersects_vertical_clip(
                    baseline,
                    ascent,
                    descent,
                    clip_top,
                    clip_bottom,
                ) {
                    glyphs.push(LayoutGlyphDebug {
                        start: glyph.start,
                        end: glyph.end,
                        text: String::new(),
                        x: 0.0,
                        width: 0.0,
                    });
                }
            }
            fragments.push(LayoutFragmentDebug {
                start: 0,
                end: 0,
                x: 0.0,
                width: 0.0,
                baseline: 0.0,
                glyphs,
            });
        }
        self.record_table_line_projection(
            paragraph_index,
            selection_line,
            "table_clip_projection",
            fragments,
        );
        Ok(())
    }

    fn capture_lr_tbv_paint_line(
        &mut self,
        paragraph_index: usize,
        paragraph: &CellParagraph,
        line: &LineLayout,
        selection_line: &SelectionLineLayout,
        text_x: f64,
        clip: Option<(f64, f64)>,
    ) -> Result<(), JsValue> {
        let painter = self.host.painter();
        let _state = StateGuard::new(&painter);
        let top = selection_line.content_top;
        let height = selection_line.content_height;
        let mut fragments = Vec::with_capacity(line.glyphs.len());
        for glyph in &line.glyphs {
            let text = paragraph.text.get(glyph.start..glyph.end).unwrap_or("");
            let font = paragraph_range_font_descriptor(paragraph, glyph.start, glyph.end, text);
            let descriptor = font.as_deref().unwrap_or(TABLE_FONT);
            painter.set_font(descriptor);
            let turned = glyph_has_east_asian(text);
            painter.set_text_baseline(if turned || line.word_baseline_offset.is_some() {
                "alphabetic"
            } else {
                "top"
            });
            let (x, y) = if turned {
                let (ascent, _) =
                    match jade_docx_layout::line_metrics::word_vertical_glyph_metrics_px(descriptor)
                    {
                        Some(metrics) => metrics,
                        None => self.font_vertical_metrics_cached(descriptor, height, 0.0)?,
                    };
                turned_em_origin(text_x + glyph.x, top, height, glyph.width.max(1.0), ascent)
            } else {
                (
                    text_x + glyph.x,
                    top + line.word_baseline_offset.unwrap_or(0.0),
                )
            };
            if let Some((clip_top, clip_bottom)) = clip {
                let metrics = painter
                    .measure_text_metrics(text)
                    .map_err(PainterError::into_js_value)?;
                if clip_bottom <= clip_top {
                    continue;
                }
                if turned {
                    // No left-ink bound is available from Painter. Keep a
                    // conservative upper-edge case; a wholly lower glyph is
                    // proven hidden by its measured right extent.
                    if y - metrics.actual_bounding_box_right.max(metrics.width) >= clip_bottom {
                        continue;
                    }
                } else if !glyph_ink_intersects_vertical_clip(
                    y,
                    metrics.actual_bounding_box_ascent,
                    metrics.actual_bounding_box_descent,
                    clip_top,
                    clip_bottom,
                ) {
                    continue;
                }
            }
            fragments.push(LayoutFragmentDebug {
                start: glyph.start,
                end: glyph.end,
                x,
                width: glyph.width,
                baseline: y,
                glyphs: vec![LayoutGlyphDebug {
                    start: glyph.start,
                    end: glyph.end,
                    text: text.into(),
                    x: 0.0,
                    width: glyph.width,
                }],
            });
        }
        self.record_table_line_projection(
            paragraph_index,
            selection_line,
            "table_paint_projection",
            fragments,
        );
        Ok(())
    }

    fn record_table_line_projection(
        &mut self,
        paragraph_index: usize,
        selection_line: &SelectionLineLayout,
        story: &'static str,
        fragments: Vec<LayoutFragmentDebug>,
    ) {
        let page_with_gap = self.page_height + PAGE_GAP_PX;
        let reported_page = (selection_line.absolute_content_top / page_with_gap).floor();
        let page = if self.pageless_mode || !reported_page.is_finite() || reported_page < 0.0 {
            0
        } else {
            reported_page as usize
        };
        let line_index = self
            .selection
            .paragraphs()
            .get(paragraph_index)
            .map_or(0, |paragraph| paragraph.lines.len());
        self.auxiliary_story_debug_lines
            .push(AuxiliaryStoryLineDebug {
                // Internal records are consumed before serialization: clip
                // masks retain source ranges; paint projections retain origins.
                story,
                page,
                line: LayoutLineDebug {
                    paragraph: paragraph_index,
                    line: line_index,
                    page,
                    column_index: 0,
                    vertical: false,
                    start: 0,
                    end: 0,
                    text: String::new(),
                    top: 0.0,
                    absolute_top: 0.0,
                    height: 0.0,
                    baseline: 0.0,
                    absolute_baseline: 0.0,
                    report_x_offset: 0.0,
                    start_x: 0.0,
                    end_x: 0.0,
                    cursor_end_x: 0.0,
                    visual_end_x: 0.0,
                    allowed_trailing_overflow_width: 0.0,
                    fragments,
                    tab_leaders: Vec::new(),
                    tab_stops: Vec::new(),
                },
            });
    }

    fn draw_horizontal_line_with_run_styles(
        &self,
        paragraph: &CellParagraph,
        line: &LineLayout,
        placement: CellLinePlacement,
        default_color: &str,
        decorations: &[crate::plugins::DecoratedRange],
    ) -> Result<(), JsValue> {
        if line.text.is_empty() {
            return Ok(());
        }
        let line_top = placement.top;
        self.draw_table_line_squiggles(line, placement, decorations)?;
        let painter = self.host.painter();
        let context = &painter;
        // Table text otherwise hangs from the line top (`textBaseline` "top").
        let text_y = match line.word_baseline_offset {
            Some(offset) => {
                context.save();
                context.set_text_baseline("alphabetic");
                line_top + offset
            }
            None => line_top,
        };
        let result = self.draw_horizontal_line_text(
            context,
            paragraph,
            line,
            placement,
            text_y,
            default_color,
            decorations,
        );
        if line.word_baseline_offset.is_some() {
            context.restore();
        }
        result
    }

    fn table_run_baseline_shift(
        &self,
        line: &LineLayout,
        properties: Option<&RunProperties>,
    ) -> f64 {
        if line.word_baseline_offset.is_none()
            || properties
                .is_none_or(|props| props.baseline.is_none() && props.text_position.is_none())
        {
            return 0.0;
        }
        self.resolve_applied_run_style_cached(properties)
            .baseline_shift_px
    }

    /// Paints the inline pictures on `line`, standing on its baseline at
    /// their layout positions.
    fn draw_cell_line_pictures(
        &mut self,
        paragraph: &CellParagraph,
        line: &LineLayout,
        text_x: f64,
        line_top: f64,
        line_height: f64,
        paragraph_index: usize,
    ) -> Result<(), JsValue> {
        if line.min_advance <= 0.0 {
            return Ok(());
        }
        let baseline = line_top + line.word_baseline_offset.unwrap_or(line_height);
        for run in &paragraph.styled_runs {
            let Some(picture) = run.picture.as_ref() else {
                continue;
            };
            let Some(glyph) = line.glyphs.iter().find(|glyph| glyph.start == run.start) else {
                continue;
            };
            let object = picture.inline_object();
            let image = picture.image.as_ref();
            let draw_spec = ImageDrawSpec {
                x: text_x + glyph.x,
                y: baseline - object.height,
                width: object.width,
                height: object.height,
                rotation_deg: image.layout.rotation_deg.unwrap_or(0.0),
                flip_horizontal: image.layout.flip_horizontal,
                flip_vertical: image.layout.flip_vertical,
                crop: image.crop,
                paragraph_index,
                run_index: picture.run_index,
            };
            if image.data_uri.is_some() {
                self.draw_image_element(image, draw_spec)?;
            } else if let Some(group) = image.shape_group.as_ref() {
                self.draw_shape_group(group, draw_spec)?;
            } else if let Some(label) = image.placeholder_label.as_deref() {
                self.draw_image_placeholder(
                    label,
                    draw_spec.x,
                    draw_spec.y,
                    draw_spec.width,
                    draw_spec.height,
                    draw_spec.rotation_deg,
                )?;
            }
        }
        Ok(())
    }

    fn table_line_selection_fragments(
        &self,
        paragraph: &CellParagraph,
        line: &LineLayout,
        text_x: f64,
    ) -> Vec<SelectionFragmentLayout> {
        if line.word_baseline_offset.is_none()
            || !paragraph.styled_runs.iter().any(|run| {
                run.start < line.end
                    && run.end > line.start
                    && run.properties.as_ref().is_some_and(|props| {
                        props.baseline.is_some() || props.text_position.is_some()
                    })
            })
        {
            return vec![SelectionFragmentLayout {
                baseline_shift_px: 0.0,
                start: line.start,
                end: line.end,
                x: text_x,
                width: line.visual_width(),
                glyphs: line.glyphs.clone().into(),
            }];
        }
        line_style_spans(line.start, line.end, &paragraph.styled_runs)
            .into_iter()
            .map(|span| {
                let properties = span
                    .run_index
                    .and_then(|index| paragraph.styled_runs.get(index))
                    .and_then(|run| run.properties.as_ref());
                let glyphs: Vec<_> = line
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.start >= span.start && glyph.end <= span.end)
                    .cloned()
                    .collect();
                let origin = glyphs.first().map_or(0.0, |glyph| glyph.x);
                SelectionFragmentLayout {
                    baseline_shift_px: self.table_run_baseline_shift(line, properties),
                    start: span.start,
                    end: span.end,
                    x: text_x + origin,
                    width: line_range_width(line, span.start, span.end),
                    glyphs: glyphs
                        .into_iter()
                        .map(|mut glyph| {
                            glyph.x -= origin;
                            glyph
                        })
                        .collect::<Vec<_>>()
                        .into(),
                }
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_horizontal_line_text<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        paragraph: &CellParagraph,
        line: &LineLayout,
        placement: CellLinePlacement,
        text_y: f64,
        default_color: &str,
        decorations: &[crate::plugins::DecoratedRange],
    ) -> Result<(), JsValue> {
        let CellLinePlacement {
            start_x,
            top: line_top,
            height: line_height,
        } = placement;
        let spans = line_style_spans(line.start, line.end, &paragraph.styled_runs);
        let mut pen_x = start_x;
        for span in spans {
            if span.end <= span.start {
                continue;
            }
            let range = span
                .run_index
                .and_then(|index| paragraph.styled_runs.get(index));
            // `draw_cell_line_pictures` paints a picture's run.
            if range.is_some_and(|range| range.picture.is_some()) {
                continue;
            }
            let run_props = range.and_then(|range| range.properties.as_ref());
            let revision = range.and_then(|range| range.revision.as_ref());
            // Draw at the layout's glyph positions: auto-spacing puts gaps
            // between glyphs that a single `fillText` of the span would close.
            let span_glyphs: Vec<&GlyphBox> = line
                .glyphs
                .iter()
                .filter(|glyph| glyph.start >= span.start && glyph.end <= span.end)
                .collect();
            let x = span_glyphs.first().map_or(pen_x, |glyph| start_x + glyph.x);
            pen_x = x + self.draw_line_text_span(
                context,
                paragraph,
                line,
                span.start,
                span.end,
                &span_glyphs,
                run_props,
                revision,
                default_color,
                x,
                line_top,
                text_y,
                line_height,
                decorations,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line_text_span<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        paragraph: &CellParagraph,
        line: &LineLayout,
        start: usize,
        end: usize,
        glyphs: &[&GlyphBox],
        run_props: Option<&RunProperties>,
        revision: Option<&RevisionMark>,
        default_color: &str,
        x: f64,
        y: f64,
        text_y: f64,
        line_height: f64,
        decorations: &[crate::plugins::DecoratedRange],
    ) -> Result<f64, JsValue> {
        if end <= start {
            return Ok(0.0);
        }
        let Some(text) = paragraph.text.get(start..end) else {
            return Ok(0.0);
        };
        if text.is_empty() {
            return Ok(0.0);
        }

        let text_y = text_y + self.table_run_baseline_shift(line, run_props);

        let font = table_font_descriptor_for_text(run_props, text);
        context.set_font(font.as_deref().unwrap_or(TABLE_FONT));
        let mut width = line_range_width(line, start, end);
        if width <= f64::EPSILON {
            width = context.measure_text_width(text).ok().unwrap_or(0.0);
        }

        let requested = run_props
            .and_then(|props| props.color.as_deref())
            .unwrap_or(default_color);
        let resolved = self.resolve_text_fill_color(requested);

        // 変更履歴の帯。本文側と同じ色で、文字の下に敷く。
        //
        // 表のセルは描画の経路が本文と分かれていて、ここが抜けていた。XML には
        // `w:ins` / `w:del` が入っているのに、表の中だけ色が付かなかった。
        if self.table_render_changes.get() && width > 0.0 {
            if let Some(mark) = revision {
                let fill = match mark.kind {
                    RevisionKind::Inserted => REVISION_INSERTED_FILL,
                    RevisionKind::Deleted => REVISION_DELETED_FILL,
                    RevisionKind::MovedFrom | RevisionKind::MovedTo => REVISION_MOVED_FILL,
                };
                context.save();
                context.set_fill_style_str(fill);
                context.fill_rect(x, y, width, line_height);
                context.restore();
                self.reset_fill_style_state();
            }
        }

        // 装飾は文字の下。ここまでで「この span をどこに、どれだけの幅で描くか」が
        // 決まっているので、その値をそのまま使う。
        self.draw_span_decorations(context, text, start, x, width, y, line_height, decorations);
        context.set_fill_style_str(resolved.as_ref());
        let text_scale_x = run_props
            .and_then(|props| props.text_scale)
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .map(|scale| (scale as f64 / 100.0).clamp(0.01, 10.0))
            .unwrap_or(1.0);
        let resolved_style = run_props.map_or_else(default_table_run_style, |props| {
            resolve_run_style_for_wrap(Some(props), None)
        });
        // Joining scripts need the neighboring letters in the painter's
        // shaping context. Preserve the span path for those scripts; Latin
        // and East Asian text can use each authoritative glyph origin.
        let contextual_shaping = glyphs.iter().any(|glyph| {
            classify_script(&paragraph.text[glyph.start..glyph.end]) == ScriptCategory::Complex
        });
        let origin = glyphs.first().map_or(0.0, |glyph| glyph.x);
        let steps = glyphs
            .iter()
            .filter(|_| !contextual_shaping)
            .map(|glyph| {
                let glyph_text = &paragraph.text[glyph.start..glyph.end];
                PlannedGlyphLayoutStep {
                    text_start: (glyph.start - start) as u32,
                    text_end: (glyph.end - start) as u32,
                    font_slot: resolved_style
                        .font_slot_for_grapheme(glyph_text, classify_script(glyph_text)),
                    x_offset: glyph.x - origin,
                }
            })
            .collect::<Vec<_>>();
        self.reset_font_state();
        if steps.is_empty() {
            if (text_scale_x - 1.0).abs() > f64::EPSILON {
                context.save();
                context
                    .translate(x, text_y)
                    .map_err(PainterError::into_js_value)?;
                context
                    .scale(text_scale_x, 1.0)
                    .map_err(PainterError::into_js_value)?;
                context
                    .fill_text(text, 0.0, 0.0)
                    .map_err(PainterError::into_js_value)?;
                context.restore();
            } else {
                context
                    .fill_text(text, x, text_y)
                    .map_err(PainterError::into_js_value)?;
            }
        } else if (text_scale_x - 1.0).abs() > f64::EPSILON {
            // Match the body renderer: layout already advances by the scaled
            // width, while Canvas must receive an explicit horizontal
            // transform to paint the glyphs into that same geometry.
            context.save();
            context
                .translate(x, text_y)
                .map_err(PainterError::into_js_value)?;
            context
                .scale(text_scale_x, 1.0)
                .map_err(PainterError::into_js_value)?;
            for step in &steps {
                self.set_font_if_changed_rc(
                    context,
                    resolved_style.descriptor_for_slot(step.font_slot),
                );
                context
                    .fill_text(step.text(text), step.x_offset / text_scale_x, 0.0)
                    .map_err(PainterError::into_js_value)?;
            }
            context.restore();
        } else if !self
            .try_draw_planned_text_batch(context, x, text_y, text, &resolved_style, &steps)
            .map_err(PainterError::into_js_value)?
        {
            for step in &steps {
                self.set_font_if_changed_rc(
                    context,
                    resolved_style.descriptor_for_slot(step.font_slot),
                );
                context
                    .fill_text(step.text(text), x + step.x_offset, text_y)
                    .map_err(PainterError::into_js_value)?;
            }
        }
        self.reset_font_state();

        let deleted = self.table_render_changes.get()
            && matches!(revision.map(|mark| &mark.kind), Some(RevisionKind::Deleted));
        if deleted || run_props.and_then(|props| props.strike).unwrap_or(false) {
            self.draw_table_strike_line(context, x, y, width, line_height, resolved.as_ref());
        }
        if run_props.and_then(|props| props.underline).unwrap_or(false) {
            self.draw_table_underline_line(context, x, y, width, line_height, resolved.as_ref());
        }

        Ok(width)
    }

    fn draw_table_strike_line<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        x: f64,
        line_top: f64,
        width: f64,
        line_height: f64,
        color: &str,
    ) {
        if width <= 0.0 {
            return;
        }
        let stroke_width = (line_height * 0.05).clamp(1.0, 2.0);
        let strike_y = line_top + line_height * 0.55;
        context.save();
        context.set_stroke_style_str(color);
        context.set_line_width(stroke_width);
        context.begin_path();
        context.move_to(x, strike_y);
        context.line_to(x + width, strike_y);
        context.stroke();
        context.restore();
    }

    fn draw_table_underline_line<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        x: f64,
        line_top: f64,
        width: f64,
        line_height: f64,
        color: &str,
    ) {
        if width <= 0.0 {
            return;
        }
        let stroke_width = (line_height * 0.05).clamp(1.0, 2.0);
        let underline_y = line_top + line_height * 0.9;
        context.save();
        context.set_stroke_style_str(color);
        context.set_line_width(stroke_width);
        context.begin_path();
        context.move_to(x, underline_y);
        context.line_to(x + width, underline_y);
        context.stroke();
        context.restore();
    }

    fn draw_table_formatting_mark(
        &self,
        line: &LineLayout,
        x: f64,
        line_top: f64,
        line_height: f64,
    ) -> Result<f64, JsValue> {
        let Some(symbol) =
            table_formatting_mark_symbol(self.table_show_formatting_marks.get(), line.break_kind)
        else {
            return Ok(0.0);
        };

        let style = self.apply_run_style(line.break_properties.as_deref())?;
        let painter = self.host.painter();
        let context = &painter;
        context.save();
        context.set_text_baseline("alphabetic");
        let width = self.draw_control_mark(context, symbol, x, line_top + line_height, &style)?;
        context.restore();
        Ok(width)
    }

    fn draw_lr_tbv_line(
        &self,
        paragraph: &CellParagraph,
        line: &LineLayout,
        placement: CellLinePlacement,
        default_color: &str,
        decorations: &[crate::plugins::DecoratedRange],
    ) -> Result<f64, JsValue> {
        let CellLinePlacement {
            start_x,
            top: line_top,
            height: line_height,
        } = placement;
        if !self.should_draw_current_page_for_render() {
            return Ok(line.visual_end_x(start_x));
        }
        let painter = self.host.painter();
        let context = &painter;
        let _state = StateGuard::new(context);
        let text_y = if let Some(offset) = line.word_baseline_offset {
            context.set_text_baseline("alphabetic");
            line_top + offset
        } else {
            context.set_text_baseline("top");
            line_top
        };
        if line.glyphs.is_empty() {
            context.set_fill_style_str(self.resolve_text_fill_color(default_color).as_ref());
            context
                .fill_text(&line.text, start_x, text_y)
                .map_err(PainterError::into_js_value)?;
            return Ok(line.visual_end_x(start_x));
        }

        for glyph in &line.glyphs {
            let glyph_text = &paragraph.text[glyph.start..glyph.end];
            let width = glyph.width.max(1.0);
            let is_east_asian = glyph_has_east_asian(glyph_text);
            let glyph_x = start_x + glyph.x;
            let font =
                paragraph_range_font_descriptor(paragraph, glyph.start, glyph.end, glyph_text);
            let descriptor = font.as_deref().unwrap_or(TABLE_FONT);
            context.set_font(descriptor);
            let requested = paragraph
                .styled_runs
                .iter()
                .find(|run| run.start <= glyph.start && glyph.start < run.end)
                .and_then(|run| run.properties.as_ref())
                .and_then(|properties| properties.color.as_deref())
                .unwrap_or(default_color);
            context.set_fill_style_str(self.resolve_text_fill_color(requested).as_ref());
            if is_east_asian {
                context.save();
                context.set_text_baseline("alphabetic");
                let (ascent, _) =
                    match jade_docx_layout::line_metrics::word_vertical_glyph_metrics_px(descriptor)
                    {
                        Some(metrics) => metrics,
                        None => self.font_vertical_metrics_cached(descriptor, line_height, 0.0)?,
                    };
                // A turned em retains its shared horizontal advance. Centre it
                // in the line box; its font ascent projects along X.
                let (x, y) = turned_em_origin(glyph_x, line_top, line_height, width, ascent);
                let _ = context.translate(x, y);
                let _ = context.rotate(-FRAC_PI_2);
                context
                    .fill_text(glyph_text, 0.0, 0.0)
                    .map_err(PainterError::into_js_value)?;
                context.restore();
            } else {
                context
                    .fill_text(glyph_text, glyph_x, text_y)
                    .map_err(PainterError::into_js_value)?;
            }
        }
        self.draw_table_line_squiggles(line, placement, decorations)?;
        Ok(line.visual_end_x(start_x))
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_rotated_paragraph(
        &mut self,
        paragraph: &CellParagraph,
        paragraph_index: usize,
        direction: TextDirection,
        align: TextAlign,
        x_start: f64,
        block_top: f64,
        cell_width: f64,
        padding_x: f64,
        line_height: f64,
        base_offset: usize,
    ) -> Result<f64, JsValue> {
        let painter = self.host.painter();
        let context = &painter;
        let draw_enabled = self.should_draw_current_page_for_render();
        let lines = if paragraph.lines.is_empty() {
            vec![LineLayout {
                min_advance: 0.0,
                left_inset: 0.0,
                right_inset: 0.0,
                text: String::new(),
                visual_width: 0.0,
                start: 0,
                end: 0,
                glyphs: Vec::new(),
                break_kind: LineBreakKind::ParagraphEnd,
                break_range: Some((0, 0)),
                break_properties: None,
                word_baseline_offset: None,
            }]
        } else {
            paragraph.lines.as_ref().clone()
        };

        let column_width = line_height.max(1.0);
        let inner_left = x_start + padding_x;
        let inner_right = x_start + cell_width - padding_x;
        let rotation = direction_rotation_angle(direction);
        let column_flow = direction_column_flow(direction);
        let block_height = paragraph.height.max(line_height);

        for (idx, line) in lines.iter().enumerate() {
            let column_index = idx as f64;
            let anchor_x = match column_flow {
                ColumnFlow::LeftToRight => inner_left + column_index * column_width,
                ColumnFlow::RightToLeft => inner_right - column_index * column_width,
            };
            let (column_start, column_end) = match column_flow {
                ColumnFlow::LeftToRight => (anchor_x, anchor_x + column_width),
                ColumnFlow::RightToLeft => (anchor_x - column_width, anchor_x),
            };

            let vertical_free = (block_height - line.visual_width).max(0.0);
            let offset = match align {
                TextAlign::Center => vertical_free / 2.0,
                TextAlign::Right => vertical_free,
                TextAlign::Left => 0.0,
            };
            let base_y = block_top + offset;

            if draw_enabled {
                context.save();
                let _ = context.translate(anchor_x, base_y);
                let _ = context.rotate(rotation);
                if !line.text.is_empty() {
                    context
                        .fill_text(&line.text, 0.0, 0.0)
                        .map_err(PainterError::into_js_value)?;
                }
                context.restore();
            }

            let line_top = base_y;
            let line_bottom = base_y + line.visual_width.max(line_height);
            let selection_line = SelectionLineLayout {
                top: line_top,
                absolute_top: self.page_offset_y + line_top,
                height: (line_bottom - line_top).max(1.0),
                baseline: line_bottom,
                start_x: column_start.min(column_end),
                cursor_end_x: column_start.max(column_end),
                allowed_trailing_overflow_width: 0.0,
                content_top: line_top,
                absolute_content_top: self.page_offset_y + line_top,
                content_height: (line_bottom - line_top).max(1.0),
                fragments: Rc::from([]),
                break_range: Some((
                    line.start.saturating_add(base_offset),
                    line.end.saturating_add(base_offset),
                )),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                column_index: self.column_index,
                vertical: false,
                vertical_reversed: false,
            };
            self.draw_selection_highlight(paragraph_index, &selection_line)?;
            self.selection.record_line(paragraph_index, selection_line);
            self.last_line_baseline = line_bottom;
        }

        Ok(block_height)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_nested_table(
        &mut self,
        layout: &mut NestedTableLayout,
        numbering_part: Option<&NumberingPart>,
        numbering_counter: &mut NumberingCounter,
        x_start: f64,
        y_start: f64,
        available_width: f64,
        ancestor_clip: Option<(f64, f64)>,
    ) -> Result<f64, JsValue> {
        let mut total_height = layout.margin_top + layout.margin_bottom;
        if layout.rows.is_empty() {
            return Ok(total_height.max(layout.line_height));
        }

        let painter = self.host.painter();
        let _table_state_guard = StateGuard::new(&painter);
        self.configure_table_context();

        let is_floating_table = table_floats(&layout.table);
        let page_margin_width = (self.page_width - self.margin_left - self.margin_right).max(1.0);
        let mut table_x = compute_floating_table_x_in_frames(
            self.margin_left,
            page_margin_width,
            self.page_width,
            x_start,
            available_width,
            layout.width,
            &layout.table,
            self.current_page_number,
        )
        .unwrap_or_else(|| {
            compute_table_x(
                x_start,
                available_width,
                layout.width,
                &layout.table,
                // The parent cell already supplies its content-frame origin.
                0.0,
            )
        });

        if !is_floating_table && table_x + layout.width > x_start + available_width {
            table_x = (x_start + available_width - layout.width).max(x_start);
        }

        let order = layout.order.unwrap_or_else(|| {
            let order = self.table_counter;
            self.table_counter += 1;
            order
        });
        let table_index = self
            .table_layouts
            .iter()
            .position(|info| info.order == order)
            .unwrap_or_else(|| {
                let index = self.table_layouts.len();
                self.table_layouts.push(TableLayoutInfo {
                    index,
                    order,
                    node_id: layout.table.node_id.map(|id| id.raw()),
                    column_widths: layout.column_widths.clone(),
                    row_heights: vec![0.0; layout.table.rows.len()],
                });
                index
            });
        let nested_column_count = layout.column_widths.len();

        let logical_y = y_start + layout.margin_top;
        let mut current_y = logical_y;
        let floating_table_frame = if is_floating_table {
            let margin_height = (self.page_height - self.margin_top - self.margin_bottom).max(0.0);
            let table_height = layout
                .rows
                .iter()
                .map(|row| row.height)
                .sum::<f64>()
                .max(layout.line_height);
            let requested_y = compute_floating_table_y(
                self.margin_top,
                margin_height,
                self.page_height,
                logical_y,
                table_height,
                &layout.table,
            )
            .unwrap_or(logical_y);
            let prevents_overlap = layout
                .table
                .style
                .get("table-overlap")
                .is_some_and(|value| value.eq_ignore_ascii_case("never"));
            let requested_frame = FloatingTableFrame {
                page_number: self.current_page_number,
                left: table_x,
                right: table_x + layout.width,
                top: requested_y,
                bottom: requested_y + table_height,
                prevents_overlap,
            };
            let resolved_y = resolve_floating_table_overlap_y(
                requested_frame,
                logical_y,
                &self.floating_table_frames,
            );
            current_y = resolved_y;
            Some(FloatingTableFrame {
                top: resolved_y,
                bottom: resolved_y + table_height,
                ..requested_frame
            })
        } else {
            None
        };
        total_height = current_y - y_start;

        let mut last_row_bottom: Option<f64> = None;
        let mut last_visible_row_index = None;

        let mut cell_order = 0usize;
        let row_heights: Vec<f64> = layout.rows.iter().map(|row| row.height).collect();
        let row_refs: Vec<&[PreparedCell]> =
            layout.rows.iter().map(|row| row.cells.as_slice()).collect();
        let row_spans = compute_vertical_merge_row_spans(&row_refs);

        for (row_index, row) in layout.rows.iter_mut().enumerate() {
            let row_height = row.height;

            if row.hidden {
                if let Some(info) = self.table_layouts.get_mut(table_index) {
                    info.row_heights[row.source_index] = 0.0;
                }
                continue;
            }
            last_visible_row_index = Some(row.source_index);

            let y_top = current_y;
            let absolute_top = self.page_offset_y + y_top;
            let absolute_bottom = absolute_top + row_height;
            if let Some(info) = self.table_layouts.get_mut(table_index) {
                info.row_heights[row.source_index] += row_height;
            }
            if row_index > 0
                && row_heights
                    .get(row_index - 1)
                    .is_some_and(|height| *height > ROW_SPLIT_EPSILON)
            {
                self.table_row_handles.push(TableRowResizeHandle {
                    table_index,
                    row_index: row.source_index.saturating_sub(1),
                    y: absolute_top,
                    left: table_x,
                    right: table_x + layout.width,
                    kind: TableRowResizeHandleKind::BetweenRows,
                });
            }
            if nested_column_count > 1 {
                for boundary in 1..nested_column_count {
                    if layout.column_positions.get(boundary).is_some()
                        && !column_boundary_crosses_merged_cell(&row.cells, boundary)
                    {
                        self.table_handles.push(TableResizeHandle {
                            table_index,
                            column_index: boundary - 1,
                            x: visual_column_boundary_x(
                                table_x,
                                &layout.column_positions,
                                boundary,
                                layout.table.bidi_visual,
                            ),
                            top: absolute_top,
                            bottom: absolute_bottom,
                            kind: TableResizeHandleKind::BetweenColumns,
                            reverse_delta: layout.table.bidi_visual,
                        });
                    }
                }
            }
            if nested_column_count > 0 {
                if let Some(last_offset) = layout.column_positions.last() {
                    self.table_handles.push(TableResizeHandle {
                        table_index,
                        column_index: visual_right_edge_column_index(
                            nested_column_count,
                            layout.table.bidi_visual,
                        ),
                        x: table_x + *last_offset,
                        top: absolute_top,
                        bottom: absolute_bottom,
                        kind: TableResizeHandleKind::TableRightEdge,
                        reverse_delta: false,
                    });
                }
            }

            let reserve_bottom = layout
                .table
                .rows
                .get(row.source_index)
                .is_some_and(row_grows_for_borders);
            let bands = HorizontalBorderBands::for_row(&row.cells, reserve_bottom);
            let spacing_background = layout
                .table
                .rows
                .get(row.source_index)
                .and_then(|source| table_spacing_background(source, &layout.table));
            let slot_scale = layout
                .table
                .rows
                .get(row.source_index)
                .map(|source| {
                    prepared_spacing_slot_scale(source, &layout.table, &row.cells, layout.width)
                })
                .unwrap_or(1.0);
            if self.should_draw_current_page_for_render() {
                let painter = self.host.painter();
                paint_spacing_tail(
                    &painter,
                    spacing_background,
                    table_x,
                    y_top,
                    layout.width,
                    row_height,
                    slot_scale,
                );
            }
            for (cell_index, cell) in row.cells.iter_mut().enumerate() {
                let column_index = cell.column_index;
                let span = cell.span;
                let end_index = (column_index + span).min(layout.column_widths.len());
                let width_slice = if column_index < end_index {
                    &layout.column_widths[column_index..end_index]
                } else {
                    layout
                        .column_widths
                        .last()
                        .map(std::slice::from_ref)
                        .unwrap_or(&[])
                };
                let mut cell_width = width_slice.iter().sum::<f64>() * slot_scale;
                if cell_width <= 0.0 {
                    cell_width = layout.width / layout.column_widths.len().max(1) as f64;
                }
                let cell_x = visual_cell_x(
                    table_x,
                    &layout.column_positions,
                    column_index,
                    end_index,
                    layout.table.bidi_visual,
                );
                let cell_x = table_x + (cell_x - table_x) * slot_scale;
                let row_span = row_spans
                    .get(row_index)
                    .and_then(|row| row.get(cell_index))
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                let row_span_end = (row_index + row_span).min(row_heights.len());
                let merged_height = row_heights[row_index..row_span_end].iter().sum::<f64>();
                let cell_rect =
                    spaced_cell_border_rect(cell_x, y_top, cell_width, merged_height, cell);

                if cell.skip_render {
                    continue;
                }

                if self.should_draw_current_page_for_render() {
                    let painter = self.host.painter();
                    let context = &painter;
                    let shading_rect = spaced_cell_shading_rect(cell_rect, cell);
                    paint_cell_spacing_background(
                        context,
                        spacing_background,
                        CellBorderRect {
                            x: cell_x,
                            y: y_top,
                            width: cell_width,
                            height: merged_height,
                        },
                        shading_rect,
                    );
                    if let Some(bg) = &cell.background {
                        #[allow(deprecated)]
                        {
                            context.set_fill_style_str(bg);
                        }
                        context.fill_rect(
                            shading_rect.x,
                            shading_rect.y,
                            shading_rect.width,
                            shading_rect.height,
                        );
                    }

                    #[allow(deprecated)]
                    {
                        context.set_stroke_style_str(BORDER_COLOR);
                    }
                    self.draw_cell_border_rect(
                        context,
                        cell_rect.x,
                        cell_rect.y,
                        cell_rect.width,
                        cell_rect.height,
                        cell,
                        bands,
                    )?;
                }

                self.render_cell_blocks(
                    cell,
                    cell_rect.x,
                    cell_rect.y,
                    cell_rect.width,
                    cell_rect.height,
                    layout.line_height,
                    numbering_part,
                    numbering_counter,
                    ancestor_clip,
                    bands.content_bottom,
                )?;

                let mut paragraph_indices: Vec<usize> = Vec::new();
                for block in &cell.blocks {
                    paragraph_indices.extend(block.paragraph_indices.iter().copied());
                }
                if !paragraph_indices.is_empty() {
                    self.selection.record_table_cell_fragment(CellLayout {
                        paragraphs: paragraph_indices,
                        left: cell_rect.x,
                        right: cell_rect.x + cell_rect.width,
                        top: absolute_top + cell_rect.y - y_top,
                        bottom: absolute_top + cell_rect.y - y_top + cell_rect.height,
                        table_id: table_index,
                        row: row.source_index,
                        column: column_index,
                        column_span: span,
                        order: cell_order,
                    });
                }
                cell_order += 1;
            }

            current_y += row_height;
            total_height += row_height;
            last_row_bottom = Some(absolute_bottom);
        }

        if let Some(info) = self.table_layouts.get_mut(table_index) {
            if info.row_heights.len() < layout.rows.len() {
                info.row_heights
                    .resize(layout.rows.len(), MIN_ROW_HEIGHT_PX);
            }
        }

        if let (Some(bottom), Some(row_index)) = (last_row_bottom, last_visible_row_index) {
            if !layout.rows.is_empty() {
                self.table_row_handles.push(TableRowResizeHandle {
                    table_index,
                    row_index,
                    y: bottom,
                    left: table_x,
                    right: table_x + layout.width,
                    kind: TableRowResizeHandleKind::TableBottomEdge,
                });
            }
        }

        total_height += layout.margin_bottom;
        if let Some(frame) = floating_table_frame {
            self.floating_table_frames.push(frame);
        }
        Ok(total_height.max(layout.line_height))
    }

    /// Lowest point a table row may reach. With a footnote area on the page
    /// it is the area's top, taken from the same footer-aware body bottom the
    /// area is reserved and drawn against (#294); without one, the margin
    /// edge rows have always used.
    fn table_row_bottom_limit(&self) -> f64 {
        if self.footnote_reserve_px > 0.0 {
            self.effective_body_bottom()
        } else {
            self.page_height - self.margin_bottom
        }
    }

    /// Ensures that the current page has enough space for a table row; starts a new page when
    /// the remaining height is insufficient. Returns `true` when a page break occurred.
    fn ensure_table_row_space(&mut self, height: f64) -> Result<bool, JsValue> {
        if self.rendering_header_footer_story {
            return Ok(false);
        }
        if self.pageless_mode {
            let required_height = self.current_y + height;
            self.host
                .ensure_pageless_height(required_height, self.margin_bottom)?;
            self.invalidate_context_state_tracking();
            return Ok(false);
        }

        // Rows stop above the page's footnote area (#294).
        let bottom_limit = self.table_row_bottom_limit();
        if self.current_y + height > bottom_limit {
            self.force_table_page_break()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn force_table_page_break(&mut self) -> Result<bool, JsValue> {
        if self.rendering_header_footer_story {
            return Ok(false);
        }
        if self.pageless_mode {
            return Ok(false);
        }
        self.render_footer_simple()?;
        let previous_page_height = self.page_height;
        self.advance_page_offset(previous_page_height);
        // `switch_host_page` releases a band-redraw clip left on the page we
        // are leaving. Calling the host directly here once left the caret
        // page clipped for every later repaint.
        let (page_width, page_height, margin_top) =
            (self.page_width, self.page_height, self.margin_top);
        self.switch_host_page(|host| host.start_new_page(page_width, page_height, margin_top))?;
        self.invalidate_context_state_tracking();
        let next_page = self.current_page_number + 1;
        self.clear_current_page_for_render(next_page)?;
        self.draw_content_crop_marks_for_page(next_page);
        self.current_page_number += 1;
        self.current_page_started_this_pass = true;
        // Ensure the new page is marked dirty so table content that continues
        // after the page break is actually drawn during incremental rendering.
        self.promote_page_to_dirty_for_current_render(self.current_page_number)?;
        self.deferred_cached_draw_page = self.current_page_number;
        self.deferred_cached_draw_commands.clear();
        self.current_y = self.margin_top;
        self.last_line_baseline = self.current_y;
        self.header_drawn = false;
        self.footer_drawn = false;
        self.render_header_simple()?;
        self.column_index = 0;
        Ok(true)
    }

    /// Applies the common drawing state (font, baseline, default border color) used by table
    /// rendering. Invoked both before the first row and after each page break.
    fn configure_table_context(&self) {
        let painter = self.host.painter();
        let context = &painter;
        context.set_font(TABLE_FONT);
        context.set_text_baseline("top");
        context.set_line_width(1.0);
        #[allow(deprecated)]
        {
            context.set_stroke_style_str(BORDER_COLOR);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cell_border_rect<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        cell: &PreparedCell,
        bands: HorizontalBorderBands,
    ) -> Result<(), JsValue> {
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        context.save();
        self.draw_horizontal_cell_border(
            context,
            x,
            y,
            width,
            &cell.border_top,
            &cell.border_top_segments,
            (bands.top > 0.0).then_some(if cell.page_top_inset > 0.0 {
                y
            } else {
                y - bands.top
            }),
        )?;
        self.draw_cell_border_side(
            context,
            x + width,
            y,
            x + width,
            y + height,
            &cell.border_right,
        )?;
        self.draw_horizontal_cell_border(
            context,
            x,
            y + height,
            width,
            &cell.border_bottom,
            &cell.border_bottom_segments,
            (bands.bottom > 0.0).then_some(y + height - bands.bottom),
        )?;
        self.draw_cell_border_side(context, x, y, x, y + height, &cell.border_left)?;
        if let Some(border) = cell.border_tl2br.as_ref() {
            self.draw_cell_border_side(context, x, y, x + width, y + height, border)?;
        }
        if let Some(border) = cell.border_tr2bl.as_ref() {
            self.draw_cell_border_side(context, x + width, y, x, y + height, border)?;
        }
        context.restore();

        Ok(())
    }

    /// Draws the table's own outer border around one painted row segment of a
    /// table with nonzero cell spacing.
    ///
    /// With cell spacing there is no border conflict: every cell keeps its own
    /// box and the outer table border is displayed as well, taken from the
    /// row's `w:tblPrEx` exception when it has one (ECMA-376 §17.4.38/§17.4.39,
    /// MS-OI29500 §17.4.66(a)). Rows without spacing keep the collapsed model
    /// where perimeter cells already draw the outer edges.
    fn draw_spaced_table_frame_segment(
        &self,
        table: &Table,
        row: &TableRow,
        frame: SpacedTableFrameSegment,
    ) -> Result<(), JsValue> {
        if resolve_cell_spacing_px(row, table) <= f64::EPSILON
            || frame.width <= 0.0
            || frame.height <= 0.0
            || !self.should_draw_current_page_for_render()
        {
            return Ok(());
        }
        let edge = |side| {
            spaced_table_frame_edge(row, table, side)
                .map(border_edge_to_spec)
                .filter(|spec| !matches!(spec.kind, BorderKind::None))
        };
        let painter = self.host.painter();
        let context = &painter;
        let (left, top, right, bottom) = (
            frame.x,
            frame.y,
            frame.x + frame.width,
            frame.y + frame.height,
        );
        context.save();
        if frame.draw_top {
            if let Some(spec) = edge(TableBorderSide::Top) {
                self.draw_cell_border_side(context, left, top, right, top, &spec)?;
            }
        }
        if let Some(spec) = edge(TableBorderSide::Left) {
            self.draw_cell_border_side(context, left, top, left, bottom, &spec)?;
        }
        if let Some(spec) = edge(TableBorderSide::Right) {
            self.draw_cell_border_side(context, right, top, right, bottom, &spec)?;
        }
        if frame.draw_bottom {
            if let Some(spec) = edge(TableBorderSide::Bottom) {
                self.draw_cell_border_side(context, left, bottom, right, bottom, &spec)?;
            }
        }
        context.restore();
        Ok(())
    }

    fn draw_vertical_merge_continuation_borders<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        rect: CellBorderRect,
        cell: &PreparedCell,
        sides: VerticalMergeContinuationBorderSides,
    ) -> Result<(), JsValue> {
        let CellBorderRect {
            x,
            y,
            width,
            height,
        } = rect;
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        context.save();
        if sides.top {
            self.draw_horizontal_cell_border(
                context,
                x,
                y,
                width,
                &cell.border_top,
                &cell.border_top_segments,
                None,
            )?;
        }
        self.draw_cell_border_side(
            context,
            x + width,
            y,
            x + width,
            y + height,
            &cell.border_right,
        )?;
        if sides.bottom {
            self.draw_horizontal_cell_border(
                context,
                x,
                y + height,
                width,
                &cell.border_bottom,
                &cell.border_bottom_segments,
                None,
            )?;
        }
        self.draw_cell_border_side(context, x, y + height, x, y, &cell.border_left)?;
        context.restore();

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_horizontal_cell_border<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        x: f64,
        y: f64,
        width: f64,
        fallback: &CellBorderSpec,
        segments: &[HorizontalBorderSegment],
        band_top: Option<f64>,
    ) -> Result<(), JsValue> {
        if segments.is_empty() {
            let y = band_top.map_or(y, |top| top + visible_border_width(fallback) * 0.5);
            return self.draw_cell_border_side(context, x, y, x + width, y, fallback);
        }
        for segment in segments {
            let y = band_top.map_or(y, |top| top + visible_border_width(&segment.border) * 0.5);
            let start = segment.start_px.clamp(0.0, width);
            let end = segment.end_px.clamp(start, width);
            if end - start > f64::EPSILON {
                self.draw_cell_border_side(context, x + start, y, x + end, y, &segment.border)?;
            }
        }
        Ok(())
    }

    /// Renders a single side of a cell border, dispatching on
    /// [`BorderKind`] so each OOXML border style maps to a distinct visual
    /// (dashed pattern, stacked parallel strokes for `Double`/`Triple`, etc.).
    /// The `spec` carries the resolved stroke width (in CSS pixels) and color,
    /// so callers don't need to re-apply canvas state beyond what this helper
    /// sets.
    pub(super) fn draw_cell_border_side<P: Painter<Error: PainterError>>(
        &self,
        context: &P,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        spec: &CellBorderSpec,
    ) -> Result<(), JsValue> {
        if matches!(spec.kind, BorderKind::None) {
            // Word-like guide line: thin dashed stroke in pale gray so an
            // explicitly hidden border remains visible while editing, but does
            // not get mistaken for a real border in screenshots.
            context.set_stroke_style_str(resolve_hidden_table_border_stroke_color(
                self.dark_mode_text,
            ));
            context.set_line_width(1.0);
            Self::apply_line_dash(context, &[1.2, 1.8]).map_err(PainterError::into_js_value)?;
            Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            return Ok(());
        }

        let stroke_color =
            resolve_table_border_stroke_color(spec.color.as_str(), self.dark_mode_text);
        #[allow(deprecated)]
        {
            context.set_stroke_style_str(stroke_color.as_ref());
        }

        // Guard against hairlines vanishing when OOXML specifies a very small
        // `w:sz`: use one full CSS pixel so Word hairlines remain opaque and
        // reliable on both light and dark preview pages.
        let base_width = resolve_table_border_stroke_width(spec.width_px, self.dark_mode_text);

        match spec.kind {
            BorderKind::None => unreachable!("handled above"),
            BorderKind::Single => {
                context.set_line_width(base_width);
                Self::apply_line_dash(context, &[]).map_err(PainterError::into_js_value)?;
                Self::stroke_pixel_snapped_segment(
                    context, start_x, start_y, end_x, end_y, base_width,
                );
            }
            BorderKind::Thick => {
                // `thick` is explicitly heavier than a `single` of the same
                // size; promote to a 2px floor so it reads as thicker even
                // when no `w:sz` was supplied.
                context.set_line_width(base_width.max(2.0));
                Self::apply_line_dash(context, &[]).map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::Double => {
                use jade_docx_layout::line_metrics::{word_layout_platform, WordLayoutPlatform};
                let platform = word_layout_platform();
                // A double's complete Mac band contains two authored rules
                // and their gap. Keep thin rules at their authored size so
                // this presentation uses the same geometry as pagination.
                let width = if platform == WordLayoutPlatform::Mac {
                    spec.width_px
                } else {
                    base_width
                };
                let metrics =
                    jade_docx_layout::border_geometry::double_border_metrics(width, platform);
                context.set_line_width(metrics.stripe_width);
                Self::apply_line_dash(context, &[]).map_err(PainterError::into_js_value)?;
                Self::stroke_parallel(
                    context,
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    metrics.stripe_offset,
                );
                Self::stroke_parallel(
                    context,
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    -metrics.stripe_offset,
                );
            }
            BorderKind::Triple => {
                let stripe_width = (base_width / 5.0).max(0.5);
                let offset = (base_width * 2.0 / 5.0).max(stripe_width);
                context.set_line_width(stripe_width);
                Self::apply_line_dash(context, &[]).map_err(PainterError::into_js_value)?;
                Self::stroke_parallel(context, start_x, start_y, end_x, end_y, offset);
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
                Self::stroke_parallel(context, start_x, start_y, end_x, end_y, -offset);
            }
            BorderKind::Dashed => {
                context.set_line_width(base_width);
                Self::apply_line_dash(context, &[base_width * 4.0, base_width * 2.0])
                    .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::DashSmallGap => {
                context.set_line_width(base_width);
                Self::apply_line_dash(context, &[base_width * 3.0, base_width])
                    .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::Dotted => {
                context.set_line_width(base_width);
                Self::apply_line_dash(context, &[base_width, base_width])
                    .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::DotDash => {
                context.set_line_width(base_width);
                Self::apply_line_dash(
                    context,
                    &[
                        base_width * 4.0,
                        base_width * 2.0,
                        base_width,
                        base_width * 2.0,
                    ],
                )
                .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::DotDotDash => {
                context.set_line_width(base_width);
                Self::apply_line_dash(
                    context,
                    &[
                        base_width * 4.0,
                        base_width * 2.0,
                        base_width,
                        base_width * 2.0,
                        base_width,
                        base_width * 2.0,
                    ],
                )
                .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
            BorderKind::Wave => {
                // Canvas has no wave primitive. Approximate with a short
                // alternating dash pattern so `wave`/`doubleWave` still
                // renders visually distinct from `single`.
                context.set_line_width(base_width);
                Self::apply_line_dash(context, &[base_width * 2.0, base_width * 2.0])
                    .map_err(PainterError::into_js_value)?;
                Self::stroke_segment(context, start_x, start_y, end_x, end_y);
            }
        }

        Ok(())
    }

    /// Strokes a straight segment between two points using the canvas's
    /// currently configured style. Extracted so the per-kind branches in
    /// [`Self::draw_cell_border_side`] stay terse.
    fn stroke_segment<P: Painter<Error: PainterError>>(
        context: &P,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
    ) {
        context.begin_path();
        context.move_to(start_x, start_y);
        context.line_to(end_x, end_y);
        context.stroke();
    }

    fn stroke_pixel_snapped_segment<P: Painter<Error: PainterError>>(
        context: &P,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        width: f64,
    ) {
        let is_hairline = width <= 1.0;
        let is_horizontal = (start_y - end_y).abs() <= f64::EPSILON;
        let is_vertical = (start_x - end_x).abs() <= f64::EPSILON;
        if is_hairline && is_horizontal {
            let y = start_y.round() + 0.5;
            Self::stroke_segment(context, start_x.round(), y, end_x.round(), y);
        } else if is_hairline && is_vertical {
            let x = start_x.round() + 0.5;
            Self::stroke_segment(context, x, start_y.round(), x, end_y.round());
        } else {
            Self::stroke_segment(context, start_x, start_y, end_x, end_y);
        }
    }

    /// Strokes a segment that runs parallel to the line from `(start_x,
    /// start_y)` to `(end_x, end_y)` but shifted by `offset` along the
    /// perpendicular direction. Used for the inner stripes of `Double` and
    /// `Triple` borders so they stay aligned along each cell edge regardless
    /// of orientation (horizontal top/bottom vs vertical left/right).
    fn stroke_parallel<P: Painter<Error: PainterError>>(
        context: &P,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        offset: f64,
    ) {
        let dx = end_x - start_x;
        let dy = end_y - start_y;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            return;
        }
        // Perpendicular unit vector (rotate tangent 90° clockwise).
        let nx = -dy / length;
        let ny = dx / length;
        context.begin_path();
        context.move_to(start_x + nx * offset, start_y + ny * offset);
        context.line_to(end_x + nx * offset, end_y + ny * offset);
        context.stroke();
    }
}

/// A resolved package style is the Mac table's padding default; absent
/// horizontal margins do not add the SDK's bare-table editing padding.
fn table_default_horizontal_padding_px(
    table: &Table,
    platform: jade_docx_layout::line_metrics::WordLayoutPlatform,
) -> f64 {
    if platform == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        && table.resolved_style.is_some()
    {
        0.0
    } else {
        points_to_px(CELL_HORIZONTAL_PADDING_PT)
    }
}

/// How far a leading-aligned table moves toward the leading margin so its
/// first cell's text, not the cell edge, sits at `w:tblInd`: Word 2007/2010
/// layout (`compatibilityMode` below 15) measures the indent to the text. Word
/// for Mac prints `system-dev-contract.docx` (mode 12, `tblInd` 0.5pt, cell
/// margin 0.5pt) with the cell text 0.5pt inside the page margin.
/// The width a top-level table is sized in. A pre-2013 table hangs its cell
/// margins outside the text extent on both sides, and Word for Mac resolves
/// a percentage `w:tblW` and autofit against that wider span:
/// `word-mac-probe-partial-grid` prints 50% as 253.8pt and 100% as 507.6pt
/// on a 496.8pt text width with 5.4pt cell margins.
fn word_pre_2013_table_sizing_width_px(
    table: &Table,
    available_width: f64,
    compatibility_mode: Option<u32>,
) -> f64 {
    if jade_docx_layout::line_metrics::word_layout_platform()
        != jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
    {
        return available_width;
    }
    let leading = pre_2013_table_text_inset_px(table, compatibility_mode);
    if leading <= 0.0 {
        return available_width;
    }
    available_width + leading + pre_2013_table_trailing_text_inset_px(table)
}

/// The cell margin a pre-2013 table hangs past its trailing text edge: the
/// last cell's trailing padding in the first row.
fn pre_2013_table_trailing_text_inset_px(table: &Table) -> f64 {
    table
        .rows
        .first()
        .and_then(|row| {
            let cell = if table.bidi_visual {
                row.cells.first()
            } else {
                row.cells.last()
            }?;
            Some(resolve_cell_padding(
                cell.margins.as_ref(),
                row.cell_margins.as_ref(),
                table.effective_cell_margins(),
                table.bidi_visual,
                table_default_horizontal_padding_px(
                    table,
                    jade_docx_layout::line_metrics::word_layout_platform(),
                ),
                0.0,
            ))
        })
        .map_or(0.0, |padding| {
            if table.bidi_visual {
                padding.left
            } else {
                padding.right
            }
        })
}

fn pre_2013_table_text_inset_px(table: &Table, compatibility_mode: Option<u32>) -> f64 {
    if compatibility_mode.is_none_or(|mode| mode >= 15) {
        return 0.0;
    }
    // Word ignores automatic/percentage indentation at the table border;
    // it does not align that ignored indent to the legacy cell text edge.
    if table
        .style
        .get("table-indent-width-type")
        .is_some_and(|kind| matches!(kind.as_str(), "auto" | "pct"))
    {
        return 0.0;
    }
    let Some(row) = table.rows.first() else {
        return 0.0;
    };
    let Some(cell) = row.cells.first() else {
        return 0.0;
    };
    let padding = resolve_cell_padding(
        cell.margins.as_ref(),
        row.cell_margins.as_ref(),
        table.effective_cell_margins(),
        table.bidi_visual,
        table_default_horizontal_padding_px(
            table,
            jade_docx_layout::line_metrics::word_layout_platform(),
        ),
        0.0,
    );
    let outer_spacing = table
        .cell_spacing
        .map(points_to_px)
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
        .max(0.0)
        * 2.0;
    outer_spacing
        + if table.bidi_visual {
            padding.right
        } else {
            padding.left
        }
}

/// Word for Mac in compatibility mode 15 measures `w:tblInd` to the outer
/// edge of the table's leading border, so the cell edge (the border's centre
/// line) and its text sit half that stroke further in. Default-style tables
/// with 0.5pt borders print their cell text 0.25pt right of the cell margin
/// (`footnote-table-each-page`, `table-with-text-direction`). Cell spacing
/// already carves the stroke out of the cell, and older modes align the text
/// edge instead (`pre_2013_table_text_inset_px`).
fn modern_leading_border_half_px(table: &Table, enabled: bool) -> f64 {
    if !enabled {
        return 0.0;
    }
    let visible_row_count = table
        .rows
        .iter()
        .filter(|row| !table_row_is_hidden(row))
        .count();
    let Some(row) = table.rows.iter().find(|row| !table_row_is_hidden(row)) else {
        return 0.0;
    };
    let cell_spacing = resolve_cell_spacing_px(row, table);
    if row.grid_before.unwrap_or(0) > 0 || cell_spacing > f64::EPSILON {
        return 0.0;
    }
    let Some(cell) = row.cells.first() else {
        return 0.0;
    };
    let total_columns = compute_column_count(table);
    let spans_row = cell.span.unwrap_or(1).max(1) as usize >= total_columns;
    let format =
        table.table_style_cell_format(table_style_cell_position(row, 0, visible_row_count, 0));
    let (name, side) = if table.bidi_visual {
        ("right", TableBorderSide::Right)
    } else {
        ("left", TableBorderSide::Left)
    };
    let (left_col, right_col) = if table.bidi_visual {
        (spans_row, true)
    } else {
        (true, spans_row)
    };
    let border = resolve_cell_border_side(
        cell,
        row,
        table,
        &format,
        name,
        side,
        [true, visible_row_count == 1, left_col, right_col],
        cell_spacing,
    );
    visible_border_width(&border) * 0.5
}

fn compute_table_x(
    base_left: f64,
    available_width: f64,
    table_width: f64,
    table: &Table,
    leading_text_inset: f64,
) -> f64 {
    let indent = table
        .style
        .get("margin-left")
        .and_then(|value| parse_length_to_px(value))
        .unwrap_or(0.0);

    if table
        .style
        .get("margin-left")
        .is_some_and(|left| left.trim().eq_ignore_ascii_case("auto"))
    {
        let centered = base_left + (available_width - table_width) / 2.0;
        return centered.max(base_left);
    }

    let remaining_width = available_width - table_width;
    let align_offset = match table.style.get("text-align").map(|value| value.trim()) {
        Some(value)
            if value.eq_ignore_ascii_case("center") || value.eq_ignore_ascii_case("centre") =>
        {
            remaining_width / 2.0
        }
        Some(value) if value.eq_ignore_ascii_case("right") || value.eq_ignore_ascii_case("end") => {
            if table.bidi_visual {
                -leading_text_inset
            } else if leading_text_inset > 0.0
                && jade_docx_layout::line_metrics::word_layout_platform()
                    == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
            {
                // A pre-2013 right-aligned table puts its last cell's text,
                // not its border, at the right margin, as a left-aligned one
                // does on the left: Word for Mac prints
                // `table-floating-anchor-defaults` (mode 14) 5.4pt further
                // right than the margin-aligned border.
                remaining_width + pre_2013_table_trailing_text_inset_px(table)
            } else {
                remaining_width
            }
        }
        // OOXML reverses table justification when `w:bidiVisual` is active:
        // the default/left value is relative to the physical right text margin.
        // Cell model order stays logical; only the table box is repositioned.
        _ => {
            if table.bidi_visual {
                remaining_width + leading_text_inset
            } else {
                -leading_text_inset
            }
        }
    };

    // `w:tblInd` is measured from the leading margin. That is physical left in
    // an ordinary table and physical right in a bidiVisual table, so the same
    // positive OOXML value moves in opposite canvas directions.
    let directed_indent = if table.bidi_visual { -indent } else { indent };
    base_left + directed_indent + align_offset
}

/// Resolves the horizontal component of `w:tblpPr` floating-table placement.
///
/// Relative alignment uses the complete anchor frame, while an absolute
/// `tblpX` positions the table's top-left corner from that frame's origin.
/// An omitted horizontal anchor defaults to the page, as required by OOXML.
/// `inside`/`outside` swap on even pages for mirrored book-page semantics.
/// This helper owns only X resolution. Vertical placement and pairwise overlap
/// are resolved after row heights are known; paragraph wrapping remains in
/// `jade-docx-layout` rather than being duplicated here.
fn compute_floating_table_x(
    margin_left: f64,
    margin_width: f64,
    page_width: f64,
    table_width: f64,
    table: &Table,
    page_number: usize,
) -> Option<f64> {
    compute_floating_table_x_in_frames(
        margin_left,
        margin_width,
        page_width,
        margin_left,
        margin_width,
        table_width,
        table,
        page_number,
    )
}

/// Resolves floating X placement when the containing text frame differs from
/// the page margin frame, as it does for a table nested inside a cell.
#[allow(clippy::too_many_arguments)]
fn compute_floating_table_x_in_frames(
    margin_left: f64,
    margin_width: f64,
    page_width: f64,
    text_left: f64,
    text_width: f64,
    table_width: f64,
    table: &Table,
    page_number: usize,
) -> Option<f64> {
    if !table_floats(table) {
        return None;
    }

    let (anchor_left, anchor_width) = match table
        .style
        .get("table-position-horizontal-anchor")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("page") => (0.0, page_width),
        // Word's `text` anchor follows the text frame containing the logical
        // anchor paragraph. Body tables use the current margin text extent.
        Some("text") => (text_left, text_width),
        Some("margin") => (margin_left, margin_width),
        // ECMA-376 defines `page` as the omitted horzAnchor default.
        None => (0.0, page_width),
        Some(_) => return None,
    };

    if let Some(alignment) = table.style.get("table-position-x-align") {
        let trailing = anchor_left + anchor_width - table_width;
        let centered = anchor_left + (anchor_width - table_width) / 2.0;
        let odd_page = page_number % 2 == 1;
        return match alignment.trim().to_ascii_lowercase().as_str() {
            "left" => Some(anchor_left),
            "center" => Some(centered),
            "right" => Some(trailing),
            "inside" => Some(if odd_page { anchor_left } else { trailing }),
            "outside" => Some(if odd_page { trailing } else { anchor_left }),
            _ => None,
        };
    }

    table
        .style
        .get("table-position-x")
        .and_then(|value| parse_length_to_px(value))
        .map(|offset| anchor_left + offset)
        // Presence of tblpPr makes the table floating even if the producer
        // omitted both X forms. Use the anchor's leading edge as Word's stable
        // default instead of falling back to inline jc/tblInd properties.
        .or(Some(anchor_left))
}

/// Resolves the vertical component of `w:tblpPr` within the current page.
///
/// Page and margin anchors use their complete frames; omission defaults to the
/// page. A text anchor starts at the table's logical body position and ignores
/// relative tblpYSpec in favor of absolute tblpY. The caller retains the
/// logical position as a flow floor because paragraph wrapping around the
/// floating frame is not implemented yet.
fn compute_floating_table_y(
    margin_top: f64,
    margin_height: f64,
    page_height: f64,
    logical_anchor_y: f64,
    table_height: f64,
    table: &Table,
) -> Option<f64> {
    if !table_floats(table) {
        return None;
    }

    let vertical_anchor = table
        .style
        .get("table-position-vertical-anchor")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "page".to_string());
    let (anchor_top, anchor_height) = match vertical_anchor.as_str() {
        "page" => (0.0, page_height),
        "margin" => (margin_top, margin_height),
        "text" => (
            logical_anchor_y,
            (margin_top + margin_height - logical_anchor_y).max(0.0),
        ),
        _ => return None,
    };

    // Relative vertical positioning is invalid for a text anchor. In that
    // case Word ignores tblpYSpec and uses the absolute tblpY offset instead.
    // For page/margin anchors, `inline` is not an alias for `top`: ST_YAlign
    // defines it as remaining in line with the surrounding text, so retain
    // the body-flow position supplied by the caller.
    if vertical_anchor != "text" {
        if let Some(alignment) = table.style.get("table-position-y-align") {
            return match alignment.trim().to_ascii_lowercase().as_str() {
                "top" => Some(anchor_top),
                "inline" => Some(logical_anchor_y),
                "center" => Some(anchor_top + (anchor_height - table_height) / 2.0),
                "bottom" => Some(anchor_top + anchor_height - table_height),
                "inside" => Some(anchor_top),
                "outside" => Some(anchor_top + anchor_height - table_height),
                _ => None,
            };
        }
    }

    let offset = table
        .style
        .get("table-position-y")
        .and_then(|value| parse_length_to_px(value))
        // Omitted tblpY is an absolute zero offset from the anchor.
        .unwrap_or(0.0);
    Some(anchor_top + offset)
}

fn floating_table_text_exclusion(table: &Table, frame: FloatingTableFrame) -> FloatExclusion {
    let distance = |key: &str| {
        table
            .style
            .get(key)
            .and_then(|value| parse_length_to_px(value))
            .unwrap_or(0.0)
    };
    FloatExclusion {
        top: frame.top - distance("table-distance-top"),
        bottom: frame.bottom + distance("table-distance-bottom"),
        left: frame.left - distance("table-distance-left"),
        right: frame.right + distance("table-distance-right"),
        wrap_text: WrapTextSide::BothSides,
        polygon: None,
    }
}

/// Splits a floating table that broke across pages into the frame it occupies
/// on each page: from its resolved top to the body bottom on the first page,
/// the full body height on intermediate pages, and from the body top to the
/// last row on the final page.
fn floating_table_page_frames(
    first: FloatingTableFrame,
    last_page: usize,
    last_bottom: f64,
    body_top: f64,
    body_bottom: f64,
) -> Vec<FloatingTableFrame> {
    if last_page <= first.page_number {
        return vec![first];
    }
    (first.page_number..=last_page)
        .map(|page_number| FloatingTableFrame {
            page_number,
            top: if page_number == first.page_number {
                first.top
            } else {
                body_top
            },
            bottom: if page_number == last_page {
                last_bottom
            } else {
                body_bottom
            },
            ..first
        })
        .collect()
}

/// Moves a requested floating-table Y below every conflicting frame.
///
/// Only tables whose horizontal and vertical extents both intersect can
/// collide. Touching edges are allowed. Repeating until stable handles a table
/// moved below one frame into another frame farther down the page.
fn resolve_floating_table_overlap_y(
    requested: FloatingTableFrame,
    logical_floor: f64,
    existing: &[FloatingTableFrame],
) -> f64 {
    let table_height = requested.bottom - requested.top;
    // Word can wrap already-laid-out anchor text around a frame positioned
    // above that anchor. Jade cannot do that yet, so never paint the table over
    // preceding logical content while retaining the requested position when it
    // is naturally below the anchor.
    let mut resolved_y = requested.top.max(logical_floor);

    loop {
        let mut next_y = resolved_y;
        for frame in existing {
            if frame.page_number != requested.page_number
                || !(requested.prevents_overlap || frame.prevents_overlap)
                || requested.right <= frame.left
                || requested.left >= frame.right
                || resolved_y + table_height <= frame.top
                || resolved_y >= frame.bottom
            {
                continue;
            }
            next_y = next_y.max(frame.bottom);
        }
        if next_y <= resolved_y {
            return resolved_y;
        }
        resolved_y = next_y;
    }
}

/// Converts a logical grid boundary into a physical canvas X coordinate. A
/// bidiVisual table keeps model/editing column indices in document order but
/// mirrors their positions around the table's right edge.
fn visual_column_boundary_x(
    table_x: f64,
    column_positions: &[f64],
    boundary: usize,
    bidi_visual: bool,
) -> f64 {
    let total_width = column_positions.last().copied().unwrap_or_default();
    let logical_offset = column_positions
        .get(boundary)
        .copied()
        .unwrap_or(total_width);
    if bidi_visual {
        table_x + total_width - logical_offset
    } else {
        table_x + logical_offset
    }
}

/// Returns the physical left edge of a cell spanning logical columns
/// `[column_start, column_end)`. Mirroring uses the logical end boundary, which
/// preserves unequal grid widths and merged-cell spans exactly.
fn visual_cell_x(
    table_x: f64,
    column_positions: &[f64],
    column_start: usize,
    column_end: usize,
    bidi_visual: bool,
) -> f64 {
    if bidi_visual {
        visual_column_boundary_x(table_x, column_positions, column_end, true)
    } else {
        visual_column_boundary_x(table_x, column_positions, column_start, false)
    }
}

/// Returns the logical column occupying the physical right table edge.
fn visual_right_edge_column_index(column_count: usize, bidi_visual: bool) -> usize {
    if bidi_visual {
        0
    } else {
        column_count.saturating_sub(1)
    }
}

/// Determines the number of columns that participate in rendering the table, taking into account
/// explicit `tblGrid` definitions as well as cell spans in each row.
fn compute_column_count(table: &Table) -> usize {
    let mut max_cols = table.columns.len();
    for row in &table.rows {
        let mut count = row.grid_before.unwrap_or(0) as usize;
        for cell in &row.cells {
            count += cell.span.unwrap_or(1) as usize;
        }
        count += row.grid_after.unwrap_or(0) as usize;
        if count > max_cols {
            max_cols = count;
        }
    }
    max_cols.max(1)
}

/// Parses the preferred-width values retained by the reader. The caller
/// supplies the correct percentage reference box because `tcW` uses the table
/// width while `wBefore`/`wAfter` use the page text extent. Absolute values use
/// the same CSS-like representation as the rest of the reader/renderer bridge.
fn preferred_width_px(value: &str, percentage_reference: f64) -> Option<f64> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|percent| percent.is_finite() && *percent >= 0.0)
            .map(|percent| percentage_reference * percent / 100.0);
    }
    parse_length_to_px(value).filter(|width| width.is_finite() && *width >= 0.0)
}

/// Seeds columns missing from `tblGrid` with the preferred widths carried by
/// `tcW`, `wBefore`, and `wAfter`. Word treats these as constraints in its table
/// layout algorithm. This is intentionally limited to undefined grid columns:
/// an explicit `gridCol` remains authoritative in the renderer's current
/// simplified autofit model.
fn apply_preferred_column_widths(
    table: &Table,
    widths: &mut [f64],
    grid_defined: &[bool],
    available_width: f64,
    table_width_reference: f64,
) {
    let mut constraints = Vec::new();
    for row in &table.rows {
        // wBefore/wAfter percentages are relative to the page text extent,
        // unlike tcW percentages below. Their grid counts identify the logical
        // leading/trailing column ranges even when this row has no cells there.
        let before_count = row.grid_before.unwrap_or(0) as usize;
        if before_count > 0 {
            if let Some(preferred_width) = row
                .width_before
                .as_deref()
                .and_then(|value| preferred_width_px(value, available_width))
            {
                constraints.push((0, before_count.min(widths.len()), preferred_width));
            }
        }
        let after_count = row.grid_after.unwrap_or(0) as usize;
        if after_count > 0 {
            if let Some(preferred_width) = row
                .width_after
                .as_deref()
                .and_then(|value| preferred_width_px(value, available_width))
            {
                let end = widths.len();
                constraints.push((end.saturating_sub(after_count), end, preferred_width));
            }
        }

        let mut column_index = row.grid_before.unwrap_or(0) as usize;
        for cell in &row.cells {
            let span = cell.span.unwrap_or(1).max(1) as usize;
            let start = column_index.min(widths.len());
            let end = (start + span).min(widths.len());
            column_index = end;
            if start >= end {
                continue;
            }

            let Some(preferred_width) = cell
                .style
                .get("width")
                .and_then(|value| preferred_width_px(value, table_width_reference))
            else {
                continue;
            };
            constraints.push((start, end, preferred_width));
        }
    }

    // Resolve narrow constraints before spanning constraints. A two-column
    // tcW normally equals the sum of the same cells' individual preferences;
    // applying the span first would split it evenly and leave a later wider
    // single-cell preference to inflate the complete table.
    constraints.sort_by_key(|(start, end, _)| end - start);
    for (start, end, preferred_width) in constraints {
        let flexible: Vec<usize> = (start..end)
            .filter(|index| !grid_defined.get(*index).copied().unwrap_or(false))
            .collect();
        if flexible.is_empty() {
            continue;
        }

        // A spanning cell's tcW describes the whole span. Preserve widths
        // already required by narrower constraints and distribute only the
        // deficit. This satisfies the span without counting its width once per
        // grid column or enlarging a span that already has the requested sum.
        let current_width: f64 = widths[start..end].iter().sum();
        let deficit = (preferred_width - current_width).max(0.0);
        let share = deficit / flexible.len() as f64;
        for index in flexible {
            widths[index] += share;
        }
    }
}

/// Builds the width (in pixels) for each column. Explicit grid definitions win;
/// missing columns first use cell `tcW` preferences, then divide any remaining
/// page width evenly when no preferred width exists.
fn compute_column_widths(table: &Table, column_count: usize, available_width: f64) -> Vec<f64> {
    let mut widths = vec![0.0_f64; column_count];
    let mut grid_defined = vec![false; column_count];
    let grid = if word_mac_sizes_without_grid(table, column_count) {
        &[][..]
    } else {
        &table.columns[..]
    };
    for (idx, column) in grid.iter().enumerate().take(column_count) {
        if let Some(width) = column.width {
            widths[idx] = points_to_px(width);
            grid_defined[idx] = width > 0.0;
        }
    }
    if let Some(cell_widths) = word_mac_stale_grid_cell_widths(table, column_count) {
        for (idx, width) in cell_widths.into_iter().enumerate() {
            widths[idx] = width;
            grid_defined[idx] = true;
        }
    }

    // A percentage tcW is relative to the table, unlike percentage tblW which
    // is relative to the page text extent. Use tblW as the provisional table
    // width when supplied; otherwise the available layout extent is the best
    // initial bound before content-driven autofit runs.
    let cell_width_reference = table
        .style
        .get("width")
        .and_then(|width| preferred_width_px(width, available_width))
        .unwrap_or(available_width);
    let table_width_budget = if cell_width_reference.is_finite() && cell_width_reference > 0.0 {
        cell_width_reference
    } else {
        available_width
    };
    apply_preferred_column_widths(
        table,
        &mut widths,
        &grid_defined,
        available_width,
        cell_width_reference,
    );

    let defined_total: f64 = widths.iter().sum();
    let mut undefined_indices: Vec<usize> = widths
        .iter()
        .enumerate()
        .filter(|(_, width)| **width <= 0.0)
        .map(|(idx, _)| idx)
        .collect();

    // tblW is a preferred total, not a hard cap: explicit grid/cell/row
    // constraints above are never shrunk here. It does, however, bound how
    // much unclaimed width is available to columns with no preference at all.
    let remaining = (table_width_budget - defined_total).max(0.0);
    let fallback = if undefined_indices.is_empty() {
        0.0
    } else {
        remaining / undefined_indices.len() as f64
    };

    for idx in undefined_indices.drain(..) {
        widths[idx] = if fallback > 0.0 {
            fallback
        } else {
            table_width_budget / column_count as f64
        };
    }

    widths
}

/// Ensures the table has a measurable width when no column widths are defined.
///
/// This intentionally does not scale down an oversized table; callers layer the
/// table-layout policy on top via [`apply_table_layout_width_for_table`].
/// The width an autofit table may not exceed. Word for Mac keeps an absolute
/// `w:tblW` wider than the text extent and lets the table run past the right
/// margin: `table-nowrap-autofit` prints its 12000-twip tables with two 300pt
/// columns on a 496.8pt text width.
fn word_autofit_width_limit_px(table: &Table, available_width: f64) -> f64 {
    if jade_docx_layout::line_metrics::word_layout_platform()
        != jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
    {
        return available_width;
    }
    table
        .style
        .get("width")
        .filter(|width| !width.trim().ends_with('%'))
        .and_then(|width| preferred_width_px(width, available_width))
        .map_or(available_width, |width| width.max(available_width))
}

fn normalize_column_widths_for_table(column_widths: &mut [f64], available_width: f64) -> f64 {
    let mut table_width: f64 = column_widths.iter().sum();
    if table_width <= 0.0 {
        let fallback = available_width / column_widths.len().max(1) as f64;
        column_widths.fill(fallback);
        table_width = available_width;
    }
    table_width
}

/// Resolves the effective table layout algorithm, defaulting absent
/// `w:tblLayout` to autofit (ECMA-376 Part 1 §17.4.52).
fn resolve_table_layout_mode(table: &Table) -> TableLayoutMode {
    table.table_layout.unwrap_or_default()
}

/// Finalizes the column widths according to the table-layout algorithm and
/// returns the total table width.
///
/// Missing widths are always filled from the leftover content width (shared
/// with [`normalize_column_widths_for_table`]). In [`TableLayoutMode::Autofit`]
/// — Word's default when `w:tblLayout` is absent — a grid wider than the
/// available content is scaled down proportionally so the whole table fits the
/// page. In [`TableLayoutMode::Fixed`] the grid is preserved verbatim and a wide
/// table overflows on the right, matching Word's fixed-width behaviour.
#[cfg(test)]
fn apply_table_layout_width(
    column_widths: &mut [f64],
    available_width: f64,
    mode: TableLayoutMode,
) -> f64 {
    apply_table_layout_width_with_minimums(column_widths, available_width, mode, &[])
}

/// Applies autofit after measuring the natural width of horizontal cell
/// contents.
///
/// Absolute `tcW` values already provide a useful minimum without canvas
/// access. Percentage and automatic widths do not: Word treats their no-wrap
/// text as one unbreakable unit, so the shared jade-docx-layout measurement is
/// folded into the minimum-width vector. Ordinary cells contribute a separate
/// preference used to redistribute, rather than grow, the saved table width.
#[allow(clippy::too_many_arguments)]
fn apply_table_layout_width_for_table_with_content<C: LineWrapContext>(
    wrap_context: &C,
    table: &Table,
    column_widths: &mut [f64],
    available_width: f64,
    default_padding_x: f64,
    line_height_for: &dyn Fn(&Paragraph) -> f64,
    cache: Option<&RefCell<TableAutofitCache>>,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Result<(f64, NaturalParagraphLayoutCache), C::Error> {
    let mode = resolve_table_layout_mode(table);
    if matches!(mode, TableLayoutMode::Fixed) {
        // Fixed layout deliberately ignores cell contents. Avoid a full-table
        // measurement pass that cannot affect the declared grid.
        return Ok((
            apply_table_layout_width_with_content_constraints(
                column_widths,
                available_width,
                mode,
                &[],
                &[],
            ),
            NaturalParagraphLayoutCache::default(),
        ));
    }
    let cache_signature = cache.and_then(|_| {
        table_autofit_cache_signature(
            table,
            column_widths,
            available_width,
            default_padding_x,
            line_height_for,
            field_value_for,
        )
    });
    let cached = cache
        .zip(cache_signature.as_ref())
        .and_then(|(cache, signature)| cache.borrow_mut().get(signature));
    let content_widths = if let Some(cached) = cached {
        cached
    } else {
        let measured = autofit_content_widths(
            wrap_context,
            table,
            column_widths,
            default_padding_x,
            line_height_for,
            field_value_for,
        )?;
        if let (Some(cache), Some(signature)) = (cache, cache_signature) {
            cache.borrow_mut().insert(signature, measured.clone());
        }
        measured
    };
    if word_mac_sizes_without_grid(table, column_widths.len()) {
        let preferred_total: f64 = content_widths.preferred.iter().sum();
        let table_width: f64 = column_widths.iter().sum();
        if content_widths
            .preferred
            .iter()
            .all(|width| *width > f64::EPSILON)
            && preferred_total <= table_width
            && table_width <= available_width + f64::EPSILON
        {
            for (width, preferred) in column_widths.iter_mut().zip(&content_widths.preferred) {
                *width = preferred + (table_width - preferred_total) * preferred / preferred_total;
            }
            return Ok((table_width, content_widths.natural_paragraph_layouts));
        }
        // Narrower than its content: each column keeps its widest word and
        // gives up width in proportion to how much more its text would take
        // (`table-cell-preferred-widths`: `FIXED 72PT` wraps at 65.5pt).
        let floors: Vec<f64> = content_widths
            .unbreakable
            .iter()
            .zip(&content_widths.preferred)
            .map(|(floor, preferred)| floor.min(*preferred))
            .collect();
        let floor_total: f64 = floors.iter().sum();
        if content_widths
            .preferred
            .iter()
            .all(|width| *width > f64::EPSILON)
            && floor_total <= table_width
            && table_width < preferred_total
            && table_width <= available_width + f64::EPSILON
        {
            let share = (table_width - floor_total) / (preferred_total - floor_total);
            for ((width, preferred), floor) in column_widths
                .iter_mut()
                .zip(&content_widths.preferred)
                .zip(&floors)
            {
                *width = floor + (preferred - floor) * share;
            }
            return Ok((table_width, content_widths.natural_paragraph_layouts));
        }
    }
    if let Some(table_width) = word_mac_shrink_autofit_to_words(
        column_widths,
        available_width,
        mode,
        &content_widths.minimums,
        &content_widths.unbreakable,
    ) {
        return Ok((table_width, content_widths.natural_paragraph_layouts));
    }
    let table_width = apply_table_layout_width_with_content_constraints(
        column_widths,
        available_width,
        mode,
        &content_widths.minimums,
        &content_widths.preferred,
    );
    Ok((table_width, content_widths.natural_paragraph_layouts))
}

/// Word for Mac fits an autofit table wider than its space below its cells'
/// absolute widths: each column keeps its widest word and gives up width in
/// proportion to how much wider than that word it is. A 2 × 165.6pt nested
/// table in a 320.4pt cell gives `A` 159.1pt and `long wrapping value`
/// 161.3pt (table-object-edge-cases; captures with `wrapping`,
/// `longwrappingvalue` and `Alpha beta` follow the same share). jade shrank
/// every column by the same share of its width. `None` where the words
/// cannot give the width.
fn word_mac_shrink_autofit_to_words(
    column_widths: &mut [f64],
    available_width: f64,
    mode: TableLayoutMode,
    minimum_widths: &[f64],
    unbreakable_widths: &[f64],
) -> Option<f64> {
    if jade_docx_layout::line_metrics::word_layout_platform()
        != jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        || !matches!(mode, TableLayoutMode::Autofit)
        || available_width <= 0.0
        || unbreakable_widths.len() != column_widths.len()
    {
        return None;
    }
    let table_width: f64 = column_widths.iter().sum();
    let deficit = table_width - available_width;
    if deficit <= 0.01 {
        return None;
    }
    let floors: Vec<f64> = column_widths
        .iter()
        .zip(unbreakable_widths)
        .enumerate()
        .map(|(index, (width, word))| {
            let minimum = minimum_widths.get(index).copied().unwrap_or(0.0);
            word.max(minimum).max(0.0).min(*width)
        })
        .collect();
    let capacity: f64 = column_widths
        .iter()
        .zip(&floors)
        .map(|(width, floor)| width - floor)
        .sum();
    if capacity + f64::EPSILON < deficit {
        return None;
    }
    for (width, floor) in column_widths.iter_mut().zip(&floors) {
        *width -= deficit * (*width - floor) / capacity;
    }
    Some(column_widths.iter().sum())
}

/// Whether Word for Mac sizes an autofit table from its content alone: its
/// `w:tblGrid` lists fewer columns than the cells use and no cell gives an
/// absolute `w:tcW`. Word then discards the grid and gives each column its
/// widest content plus a share of the remaining table width in proportion
/// to that content (`word-mac-probe-partial-grid`: `AAAA AAAA`, `B`, `C` in
/// 253.8pt take 166.8, 43.7 and 43.3pt; one, two or a 2880-twip grid column
/// for three `A` cells all give three equal columns).
/// Word for Mac sizes an autofit table from its cells' preferred widths and
/// saves the result as the grid, so a grid Word wrote matches the cells'
/// widths but for its own fitting (scroll-offset-repro: 3217 and 4469 twips
/// for cells of 3260 and 4536). A grid that disagrees with every row's cell
/// widths by more than that was not computed from them, and Word lays the
/// table out from the cells: word-compat-comprehensive prints a 3312-twip
/// grid as its cells' 2160, 3744 and 3744 twips. Returns those widths in px.
fn word_mac_stale_grid_cell_widths(table: &Table, column_count: usize) -> Option<Vec<f64>> {
    if jade_docx_layout::line_metrics::word_layout_platform()
        != jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        || !matches!(resolve_table_layout_mode(table), TableLayoutMode::Autofit)
        || table.columns.len() != column_count
        || column_count == 0
    {
        return None;
    }
    let mut cell_widths: Vec<Option<f64>> = vec![None; column_count];
    for row in &table.rows {
        if row.grid_before.unwrap_or(0) > 0 || row.grid_after.unwrap_or(0) > 0 {
            return None;
        }
        if row.cells.len() != column_count {
            return None;
        }
        for (column, cell) in row.cells.iter().enumerate() {
            if cell.span.unwrap_or(1) != 1 || !cell_has_absolute_preferred_width(cell) {
                return None;
            }
            let width = cell
                .style
                .get("width")
                .and_then(|value| parse_length_to_px(value))
                .filter(|width| *width > 0.0)?;
            match cell_widths[column] {
                Some(previous) if (previous - width).abs() > 0.5 => return None,
                _ => cell_widths[column] = Some(width),
            }
        }
    }
    let cell_widths = cell_widths.into_iter().collect::<Option<Vec<f64>>>()?;
    let stale = table
        .columns
        .iter()
        .zip(&cell_widths)
        .any(|(column, cell)| {
            column
                .width
                .map(points_to_px)
                .is_none_or(|grid| (grid - cell).abs() > cell * 0.05)
        });
    stale.then_some(cell_widths)
}

fn word_mac_sizes_without_grid(table: &Table, column_count: usize) -> bool {
    jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        && matches!(resolve_table_layout_mode(table), TableLayoutMode::Autofit)
        && table.columns.len() < column_count
        && table.rows.iter().all(|row| {
            row.cells
                .iter()
                .all(|cell| !cell_has_absolute_preferred_width(cell))
        })
}

fn no_wrap_absolute_minimum_widths(table: &Table, column_widths: &[f64]) -> Vec<f64> {
    let mut minimums = vec![0.0_f64; column_widths.len()];
    for row in &table.rows {
        let mut column_index = row.grid_before.unwrap_or(0) as usize;
        for cell in &row.cells {
            let span = cell.span.unwrap_or(1).max(1) as usize;
            let start = column_index.min(column_widths.len());
            let end = (start + span).min(column_widths.len());
            column_index = end;
            if start >= end
                || !cell
                    .style
                    .get("no-wrap")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                continue;
            }

            // The reader serializes dxa tcW values as points and pct values
            // with a percent suffix. Only the absolute case defines the fixed
            // minimum described by noWrap without measuring cell contents.
            let Some(width_value) = cell.style.get("width") else {
                continue;
            };
            if !width_value.trim().to_ascii_lowercase().ends_with("pt") {
                continue;
            }
            let Some(preferred_width) = parse_length_to_px(width_value) else {
                continue;
            };

            let current_span_width: f64 = column_widths[start..end].iter().sum();
            for index in start..end {
                let share = if current_span_width > f64::EPSILON {
                    preferred_width * column_widths[index] / current_span_width
                } else {
                    preferred_width / (end - start) as f64
                };
                minimums[index] = minimums[index].max(share.min(column_widths[index]));
            }
        }
    }
    minimums
}

#[derive(Clone, Debug)]
struct AutofitContentWidths {
    /// Hard shrink floors. Only no-wrap content belongs here because ordinary
    /// text may wrap when the page is narrower than its preferred width.
    minimums: Vec<f64>,
    /// Natural non-wrapped widths used to redistribute the saved table width.
    preferred: Vec<f64>,
    /// Widest unbreakable piece of each column's content, with the cell
    /// chrome: the narrowest the column gets without breaking a word.
    unbreakable: Vec<f64>,
    /// Full natural layouts produced while measuring preferred widths. Row
    /// preparation can reuse these in the final table when the resolved cell
    /// width still fits them, avoiding an identical second wrap pass. Values
    /// are shared across cache hits; the final row layout clones only the
    /// paragraphs it can reuse.
    natural_paragraph_layouts: NaturalParagraphLayoutCache,
}

#[derive(Clone, Debug)]
struct NaturalParagraphLayout {
    layout: CellParagraph,
    required_width: f64,
    line_height: f64,
    styled_runs: Vec<StyledRunRange>,
}

type NaturalParagraphLayoutCache = Rc<Vec<NaturalParagraphLayout>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableAutofitCacheSignature {
    hash: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TableAutofitCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub invalidations: u64,
    pub saved_paragraph_measures: u64,
    pub size: usize,
    pub capacity: usize,
}

#[derive(Clone, Debug)]
struct TableAutofitCacheEntry {
    signature: TableAutofitCacheSignature,
    widths: AutofitContentWidths,
    paragraph_measure_count: u64,
}

#[derive(Debug)]
pub(super) struct TableAutofitCache {
    entries: FxHashMap<u64, TableAutofitCacheEntry>,
    lru: VecDeque<u64>,
    stats: TableAutofitCacheStats,
}

impl Default for TableAutofitCache {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
            lru: VecDeque::new(),
            stats: TableAutofitCacheStats {
                capacity: TABLE_AUTOFIT_CACHE_CAPACITY,
                ..TableAutofitCacheStats::default()
            },
        }
    }
}

impl TableAutofitCache {
    fn get(&mut self, signature: &TableAutofitCacheSignature) -> Option<AutofitContentWidths> {
        let Some(entry) = self.entries.get(&signature.hash) else {
            self.stats.misses += 1;
            return None;
        };
        if entry.signature != *signature {
            self.stats.misses += 1;
            return None;
        }
        let widths = entry.widths.clone();
        let saved = entry.paragraph_measure_count;
        self.stats.hits += 1;
        self.stats.saved_paragraph_measures += saved;
        self.lru.retain(|key| *key != signature.hash);
        self.lru.push_back(signature.hash);
        Some(widths)
    }

    fn insert(&mut self, signature: TableAutofitCacheSignature, widths: AutofitContentWidths) {
        let paragraph_measure_count = widths.natural_paragraph_layouts.len() as u64;
        self.lru.retain(|key| *key != signature.hash);
        self.lru.push_back(signature.hash);
        self.entries.insert(
            signature.hash,
            TableAutofitCacheEntry {
                signature,
                widths,
                paragraph_measure_count,
            },
        );
        while self.entries.len() > TABLE_AUTOFIT_CACHE_CAPACITY {
            if let Some(key) = self.lru.pop_front() {
                self.entries.remove(&key);
            }
        }
        self.stats.size = self.entries.len();
    }

    pub(super) fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.stats.invalidations += self.entries.len() as u64;
        }
        self.entries.clear();
        self.lru.clear();
        self.stats.size = 0;
    }

    pub(super) fn stats(&self) -> TableAutofitCacheStats {
        self.stats
    }
}

fn table_autofit_cache_signature(
    table: &Table,
    column_widths: &[f64],
    available_width: f64,
    default_padding_x: f64,
    line_height_for: &dyn Fn(&Paragraph) -> f64,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Option<TableAutofitCacheSignature> {
    let mut bytes = serde_json::to_vec(table).ok()?;
    // `resolved_style` is intentionally skipped by the document serializer but
    // affects table and first-row typography, so retain it in the cache key.
    bytes.extend_from_slice(format!("{:?}", table.resolved_style).as_bytes());
    bytes.extend_from_slice(&available_width.to_bits().to_le_bytes());
    bytes.extend_from_slice(&default_padding_x.to_bits().to_le_bytes());
    for width in column_widths {
        bytes.extend_from_slice(&width.to_bits().to_le_bytes());
    }
    for row in &table.rows {
        for cell in &row.cells {
            for block in &cell.blocks {
                if let BodyElement::Paragraph(paragraph) = block {
                    bytes.extend_from_slice(&line_height_for(paragraph).to_bits().to_le_bytes());
                    for run in &paragraph.runs {
                        if let Some(field) = &run.field {
                            let value = field_value_for(field);
                            bytes.push(u8::from(value.is_some()));
                            if let Some(value) = value {
                                bytes.extend_from_slice(&value.len().to_le_bytes());
                                bytes.extend_from_slice(value.as_bytes());
                            }
                        }
                    }
                }
            }
        }
    }
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    Some(TableAutofitCacheSignature {
        hash: hasher.finish(),
        bytes,
    })
}

fn table_autofit_cache_supported(table: &Table) -> bool {
    table.rows.iter().all(|row| {
        row.cells.iter().all(|cell| {
            cell.text_direction.is_none()
                && cell
                    .blocks
                    .iter()
                    .all(|block| matches!(block, BodyElement::Paragraph(_)))
        })
    })
}

fn take_fitting_natural_paragraph_layout(
    layouts: &mut NaturalParagraphLayoutCache,
    available_width: f64,
    line_height: f64,
    text: &str,
    styled_runs: &[StyledRunRange],
) -> Option<CellParagraph> {
    layouts
        .iter()
        .find(|cached| {
            cached.required_width <= available_width + f64::EPSILON
                && cached.line_height.to_bits() == line_height.to_bits()
                && cached.layout.text == text
                && cached.styled_runs == styled_runs
        })
        .map(|cached| cached.layout.clone())
}

/// Measures natural horizontal cell extents through jade-docx-layout and maps each
/// cell-wide constraint back to its grid columns. Spanning cells distribute
/// their width in the same proportions as the provisional grid, preserving
/// explicit and preferred column relationships.
///
/// Ordinary cells contribute only a preferred width. `w:noWrap` cells also
/// contribute a minimum so page fitting cannot wrap them while a sibling still
/// has shrink capacity.
/// `line_height_for` supplies each paragraph's resolved line height; the
/// natural layouts measured here are reused by row layout, so their heights
/// must already be the per-paragraph ones.
/// `field_value_for` supplies the same displayed text used by final row layout;
/// stale saved page fields must not determine the natural width.
fn autofit_content_widths<C: LineWrapContext>(
    context: &C,
    table: &Table,
    column_widths: &[f64],
    default_padding_x: f64,
    line_height_for: &dyn Fn(&Paragraph) -> f64,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Result<AutofitContentWidths, C::Error> {
    let mut minimums = no_wrap_absolute_minimum_widths(table, column_widths);
    let mut preferred = vec![0.0_f64; column_widths.len()];
    let mut explicit_width_columns = vec![false; column_widths.len()];
    // Widest piece of content that cannot wrap, per column.
    let mut unbreakable_widths = vec![0.0_f64; column_widths.len()];
    let mut natural_paragraph_layouts = Vec::new();
    for row in &table.rows {
        let mut column_index = row.grid_before.unwrap_or(0) as usize;
        for cell in &row.cells {
            let span = cell.span.unwrap_or(1).max(1) as usize;
            let start = column_index.min(column_widths.len());
            let end = (start + span).min(column_widths.len());
            column_index = end;
            if start >= end
                || cell.text_direction.is_some_and(|direction| {
                    direction_requires_vertical_stack(direction)
                        || direction_requires_rotated_layout(direction)
                })
            {
                continue;
            }
            let no_wrap = cell
                .style
                .get("no-wrap")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            if cell_has_absolute_preferred_width(cell) {
                explicit_width_columns[start..end].fill(true);
            }

            let mut natural_content_width = 0.0_f64;
            let mut unbreakable_content_width = 0.0_f64;
            for block in &cell.blocks {
                let BodyElement::Paragraph(paragraph) = block else {
                    continue;
                };
                let (text, styled_runs, _) = collect_paragraph_text(
                    paragraph,
                    true,
                    false,
                    &NoteMarkers::default(),
                    field_value_for,
                );
                if text.is_empty() {
                    continue;
                }
                let run_styles = styled_run_styles(&text, &styled_runs);
                let natural = layout_horizontal_paragraph_with_run_styles_context(
                    context,
                    &text,
                    1_000_000_000.0,
                    line_height_for(paragraph),
                    &run_styles,
                    &ParagraphStyle::default(),
                )?;
                let overhang = paragraph_trailing_overhang(context, &text, &styled_runs)?;
                // `max_line_width` never drops below the line height, which
                // would widen a one-letter column. Word for Mac shares an
                // autofit table by the text's own width.
                let content_width = if jade_docx_layout::line_metrics::word_layout_platform()
                    == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
                {
                    natural
                        .lines
                        .iter()
                        .map(|line| line.visual_width)
                        .fold(0.0, f64::max)
                } else {
                    natural.max_line_width
                };
                let required_width = content_width + overhang;
                natural_content_width = natural_content_width.max(required_width);
                unbreakable_content_width =
                    unbreakable_content_width.max(longest_unbreakable_width(&natural, &text));
                natural_paragraph_layouts.push(NaturalParagraphLayout {
                    layout: natural,
                    required_width,
                    line_height: line_height_for(paragraph),
                    styled_runs,
                });
            }
            if natural_content_width <= f64::EPSILON {
                continue;
            }

            let padding = resolve_cell_padding(
                cell.margins.as_ref(),
                row.cell_margins.as_ref(),
                table.effective_cell_margins(),
                false,
                default_padding_x,
                0.0,
            );
            let chrome = padding.horizontal() + resolve_cell_spacing_px(row, table);
            let required_width = natural_content_width + chrome;
            let unbreakable_width = unbreakable_content_width + chrome;
            let current_span_width: f64 = column_widths[start..end].iter().sum();
            for index in start..end {
                let span_share = if current_span_width > f64::EPSILON {
                    column_widths[index] / current_span_width
                } else {
                    1.0 / (end - start) as f64
                };
                let share = required_width * span_share;
                preferred[index] = preferred[index].max(share);
                unbreakable_widths[index] =
                    unbreakable_widths[index].max(unbreakable_width * span_share);
                if no_wrap {
                    minimums[index] = minimums[index].max(share);
                }
            }
        }
    }
    // Word wraps the content of a column whose cells give an absolute
    // `w:tcW` instead of widening it to the content's natural width: Word for
    // Mac 16.113 keeps two 4800-dxa cells of a 9600-dxa autofit table 240pt
    // wide each around a long sentence, and a 2000-dxa one 100pt wide, while
    // cells without `w:tcW` (or with `w:type="auto"`) share the width by
    // natural content width. Only content that cannot wrap widens such a
    // column (a 40-letter word takes a 4500-dxa column to 291pt, from its
    // neighbour), so its preferred width is its widest unbreakable piece.
    for (index, explicit) in explicit_width_columns.iter().enumerate() {
        if *explicit {
            preferred[index] = unbreakable_widths[index];
        }
    }
    Ok(AutofitContentWidths {
        minimums,
        unbreakable: unbreakable_widths.clone(),
        preferred,
        natural_paragraph_layouts: Rc::new(natural_paragraph_layouts),
    })
}

/// The widest run of glyphs in `paragraph` that has no break opportunity: a
/// word of Latin letters or digits. East Asian characters and whitespace may
/// break on either side, so they end a run.
fn longest_unbreakable_width(paragraph: &CellParagraph, text: &str) -> f64 {
    let mut widest = 0.0_f64;
    for line in paragraph.lines.iter() {
        let mut run = 0.0_f64;
        for glyph in &line.glyphs {
            let grapheme = text.get(glyph.start..glyph.end).unwrap_or("");
            let breakable = grapheme.is_empty()
                || grapheme.chars().all(char::is_whitespace)
                || classify_script(grapheme) == ScriptCategory::EastAsia;
            if breakable {
                widest = widest.max(run);
                run = 0.0;
                if !grapheme.chars().all(char::is_whitespace) {
                    widest = widest.max(glyph.width);
                }
            } else {
                run += glyph.width;
            }
        }
        widest = widest.max(run);
    }
    widest
}

/// Whether `cell` gives an absolute preferred width (`w:tcW w:type="dxa"`,
/// serialized by the reader in points).
fn cell_has_absolute_preferred_width(cell: &TableCell) -> bool {
    cell.style
        .get("width")
        .is_some_and(|value| value.trim().to_ascii_lowercase().ends_with("pt"))
}

#[cfg(test)]
fn apply_table_layout_width_with_minimums(
    column_widths: &mut [f64],
    available_width: f64,
    mode: TableLayoutMode,
    minimum_widths: &[f64],
) -> f64 {
    apply_table_layout_width_with_content_constraints(
        column_widths,
        available_width,
        mode,
        minimum_widths,
        &[],
    )
}

fn apply_table_layout_width_with_content_constraints(
    column_widths: &mut [f64],
    available_width: f64,
    mode: TableLayoutMode,
    minimum_widths: &[f64],
    preferred_widths: &[f64],
) -> f64 {
    let mut table_width = normalize_column_widths_for_table(column_widths, available_width);

    // Content-derived minima may be wider than a provisional percentage/auto
    // column. Expand those columns first, then let the ordinary autofit shrink
    // capacity come from their siblings. Fixed layout intentionally ignores
    // content minima and preserves its declared grid verbatim.
    if matches!(mode, TableLayoutMode::Autofit) {
        for (index, width) in column_widths.iter_mut().enumerate() {
            if let Some(minimum) = minimum_widths.get(index).copied() {
                *width = (*width).max(minimum);
            }
        }
        table_width = column_widths.iter().sum();

        // tblGrid is an initial grid, not the final width under autofit. Move
        // width from columns already wider than their natural contents to
        // columns whose unwrapped contents exceed the provisional width. This
        // preserves the table's saved total width; ordinary content remains a
        // preference (not a shrink floor) so it can still wrap on narrow pages.
        let receiver_demands: Vec<f64> = column_widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let preferred = preferred_widths
                    .get(index)
                    .copied()
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0)
                    .max(0.0);
                (preferred - *width).max(0.0)
            })
            .collect();
        let donor_capacities: Vec<f64> = column_widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let minimum = minimum_widths
                    .get(index)
                    .copied()
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0)
                    .max(0.0);
                let preferred = preferred_widths
                    .get(index)
                    .copied()
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0)
                    .max(0.0);
                let donor_floor = minimum.max(preferred.min(*width));
                (*width - donor_floor).max(0.0)
            })
            .collect();
        let total_demand: f64 = receiver_demands.iter().sum();
        let total_capacity: f64 = donor_capacities.iter().sum();
        let transfer = total_demand.min(total_capacity);
        if transfer > f64::EPSILON {
            for (index, width) in column_widths.iter_mut().enumerate() {
                let received = transfer * receiver_demands[index] / total_demand;
                let donated = transfer * donor_capacities[index] / total_capacity;
                *width += received - donated;
            }
            table_width = column_widths.iter().sum();
        }
    }

    if matches!(mode, TableLayoutMode::Autofit)
        && available_width > 0.0
        && table_width > available_width
    {
        let deficit = table_width - available_width;
        let shrink_capacity: f64 = column_widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let minimum = minimum_widths.get(index).copied().unwrap_or(0.0);
                (width - minimum.min(*width)).max(0.0)
            })
            .sum();

        if shrink_capacity + f64::EPSILON >= deficit {
            for (index, width) in column_widths.iter_mut().enumerate() {
                let minimum = minimum_widths.get(index).copied().unwrap_or(0.0);
                let capacity = (*width - minimum.min(*width)).max(0.0);
                *width -= deficit * capacity / shrink_capacity.max(f64::EPSILON);
            }
        } else {
            // The absolute minima themselves exceed the page. Once every
            // other column is exhausted, proportionally fit the unavoidable
            // remainder so the existing autofit guarantee still holds.
            for (index, width) in column_widths.iter_mut().enumerate() {
                *width = minimum_widths
                    .get(index)
                    .copied()
                    .unwrap_or(0.0)
                    .min(*width);
            }
            let minimum_total: f64 = column_widths.iter().sum();
            if minimum_total > available_width && minimum_total > f64::EPSILON {
                let scale = available_width / minimum_total;
                for width in column_widths.iter_mut() {
                    *width *= scale;
                }
            }
        }
        table_width = column_widths.iter().sum();
    }

    table_width
}

/// Computes row spans for vertically merged cells (`w:vMerge`). Restart cells
/// return a span length, continuation cells return `0`, and normal cells return `1`.
fn compute_vertical_merge_row_spans(rows: &[&[PreparedCell]]) -> Vec<Vec<usize>> {
    let mut spans: Vec<Vec<usize>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| if cell.skip_render { 0 } else { 1 })
                .collect()
        })
        .collect();

    for row_index in 0..rows.len() {
        for cell_index in 0..rows[row_index].len() {
            let cell = &rows[row_index][cell_index];
            if cell.skip_render || !matches!(cell.vertical_merge.as_deref(), Some("restart")) {
                continue;
            }

            let mut span = 1usize;
            for next_row_index in (row_index + 1)..rows.len() {
                let next_row = rows[next_row_index];
                // Continue cells are matched by both column start and horizontal span so that
                // vertical merges stay on the same logical grid region even when row structures
                // differ (e.g. neighboring cells use different spans).
                let Some(next_cell_index) = next_row.iter().position(|candidate| {
                    candidate.column_index == cell.column_index
                        && candidate.span == cell.span
                        && matches!(candidate.vertical_merge.as_deref(), Some("continue"))
                }) else {
                    break;
                };
                span += 1;
                spans[next_row_index][next_cell_index] = 0;
            }

            spans[row_index][cell_index] = span;
        }
    }

    spans
}

/// `(first row, rows spanned, content height)` of every `w:vMerge` owner
/// spanning more than one row.
fn merged_cell_owners(rows: &[&[PreparedCell]], spans: &[Vec<usize>]) -> Vec<(usize, usize, f64)> {
    let mut owners = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (cell_index, cell) in row.iter().enumerate() {
            if spans_merged_owner(&spans[row_index], cell_index) {
                owners.push((row_index, spans[row_index][cell_index], cell.height));
            }
        }
    }
    owners
}

/// Whether cell `cell_index` of a row owns a vertical merge over more rows.
fn spans_merged_owner(row_spans: &[usize], cell_index: usize) -> bool {
    row_spans.get(cell_index).is_some_and(|&span| span > 1)
}

/// Word sizes the rows a `w:vMerge` owner spans from their other cells and
/// adds only the part of the owner taller than all of them to the last row.
fn grow_rows_for_vertical_merges(row_heights: &mut [f64], owners: &[(usize, usize, f64)]) {
    for &(row, span, height) in owners {
        if span < 2 || row + span > row_heights.len() {
            continue;
        }
        let spanned: f64 = row_heights[row..row + span].iter().sum();
        if height > spanned {
            row_heights[row + span - 1] += height - spanned;
        }
    }
}

/// Chooses the horizontal borders drawn for a `w:vMerge` continuation cell.
/// Continuations keep left/right borders, resume a top border when the merged
/// cell appears at the top of a new page segment, and suppress inner bottoms
/// until the final continuation row.
fn vertical_merge_continuation_border_sides(
    rows: &[Vec<PreparedCell>],
    row_index: usize,
    cell_index: usize,
    page_segment_top: bool,
    page_segment_bottom: bool,
) -> VerticalMergeContinuationBorderSides {
    let Some(cell) = rows.get(row_index).and_then(|row| row.get(cell_index)) else {
        return VerticalMergeContinuationBorderSides {
            top: false,
            bottom: true,
        };
    };
    if !matches!(cell.vertical_merge.as_deref(), Some("continue")) {
        return VerticalMergeContinuationBorderSides {
            top: false,
            bottom: true,
        };
    }

    let continues_below = rows.get(row_index + 1).is_some_and(|next_row| {
        next_row.iter().any(|candidate| {
            candidate.column_index == cell.column_index
                && candidate.span == cell.span
                && matches!(candidate.vertical_merge.as_deref(), Some("continue"))
        })
    });

    VerticalMergeContinuationBorderSides {
        top: page_segment_top,
        bottom: page_segment_bottom || !continues_below,
    }
}

fn page_clipped_cell_segment(
    y_top: f64,
    height: f64,
    bottom_limit: Option<f64>,
) -> PageClippedCellSegment {
    let Some(bottom_limit) = bottom_limit else {
        return PageClippedCellSegment {
            height,
            clipped_at_bottom: false,
        };
    };
    let available = (bottom_limit - y_top).max(0.0);
    if height > available + ROW_SPLIT_EPSILON {
        PageClippedCellSegment {
            height: available,
            clipped_at_bottom: true,
        }
    } else {
        PageClippedCellSegment {
            height,
            clipped_at_bottom: false,
        }
    }
}

/// Clips a vertically merged cell at the last complete row that stays on the
/// current page. Table pagination advances whole `w:vMerge` continuation rows,
/// so Word closes the visible merge fragment at that row boundary instead of
/// extending its side borders through the unused bottom-margin space.
fn page_clipped_vertical_merge_segment(
    y_top: f64,
    row_heights: &[f64],
    bottom_limit: Option<f64>,
) -> PageClippedCellSegment {
    let total_height = row_heights.iter().sum::<f64>();
    let Some(bottom_limit) = bottom_limit else {
        return PageClippedCellSegment {
            height: total_height,
            clipped_at_bottom: false,
        };
    };

    let available = (bottom_limit - y_top).max(0.0);
    let mut complete_rows_height = 0.0;
    for row_height in row_heights {
        if complete_rows_height + row_height > available + ROW_SPLIT_EPSILON {
            return PageClippedCellSegment {
                height: if complete_rows_height > ROW_SPLIT_EPSILON {
                    complete_rows_height
                } else {
                    available
                },
                clipped_at_bottom: true,
            };
        }
        complete_rows_height += row_height;
    }

    PageClippedCellSegment {
        height: total_height,
        clipped_at_bottom: false,
    }
}

/// Decide whether a `w:cantSplit` row must be kept together rather than split
/// across a page boundary. A non-splittable row is only protected when it could
/// actually fit on a fresh, empty page.
fn cant_split_blocks_row_split(
    cant_split: bool,
    row_height: f64,
    full_page_content_height: f64,
) -> bool {
    cant_split && row_height <= full_page_content_height
}

fn row_height_blocks_content_split(row: &TableRow, cells: &[PreparedCell]) -> bool {
    let Some(height) = row.height else {
        return false;
    };
    match row.height_rule.unwrap_or(TableRowHeightRule::Auto) {
        TableRowHeightRule::Exact => true,
        // A minimum is not a keep-together restriction when content determines
        // the height. Keep minimum-driven blank space intact: the content
        // splitter only distributes actual blocks, not declared empty space.
        TableRowHeightRule::AtLeast => {
            let content_height = cells.iter().map(|cell| cell.height).fold(0.0, f64::max);
            points_to_px(height) > content_height + ROW_SPLIT_EPSILON
        }
        TableRowHeightRule::Auto => false,
    }
}

/// Applies Microsoft Word's application-specific `w:hRule="exact"` height.
///
/// ISO/IEC 29500 describes the declared value as the complete row height, but
/// MS-OE376 Part 4 §2.4.77(f) documents that Word adds the largest bottom cell
/// padding in the row. Use the already-resolved physical cell padding so table,
/// row-exception, and direct cell margin precedence stays centralized.
fn exact_row_height_with_bottom_padding(explicit_height: f64, cells: &[PreparedCell]) -> f64 {
    let largest_bottom_padding = cells
        .iter()
        .map(|cell| cell.padding.bottom)
        .fold(0.0_f64, f64::max);
    explicit_height + largest_bottom_padding
}

/// Word persists `hRule="exact"` rows down to one twip; the auto-row floor
/// exists only to keep rows grabbable, so it must not inflate them.
const MIN_EXACT_ROW_HEIGHT_PT: f32 = 1.0 / 20.0;

/// Resolves one row's painted height from its laid-out cells and `w:trHeight`.
///
/// `content_height` is the tallest painted cell (content plus the resolved cell
/// padding); `fallback_height` stands in when nothing in the row paints, e.g.
/// a row made only of vertical-merge continuations. Auto and `atLeast` rows keep
/// the renderer's floor, but an `exact` row is exactly what Word persists (plus
/// the largest bottom cell margin, see [`exact_row_height_with_bottom_padding`]):
/// Word draws a 100-twip exact row 5pt tall, well below the floor.
fn resolve_row_height(
    content_height: f64,
    fallback_height: f64,
    row: &TableRow,
    cells: &[PreparedCell],
) -> f64 {
    let mut row_height = if content_height > 0.0 {
        content_height
    } else {
        fallback_height
    };
    match (
        row.height,
        row.height_rule.unwrap_or(TableRowHeightRule::Auto),
    ) {
        (Some(explicit_height), TableRowHeightRule::Exact) => {
            return exact_row_height_with_bottom_padding(points_to_px(explicit_height), cells)
                .max(points_to_px(MIN_EXACT_ROW_HEIGHT_PT));
        }
        (Some(explicit_height), TableRowHeightRule::AtLeast) => {
            // Horizontal rows retain vertical padding around the minimum.
            // All directions retain the shared horizontal border band: half
            // of each border, or on Mac the bottom border below the padding
            // (a 14.25pt minimum under a 1.5pt bottom is 23.75pt with 4pt
            // padding; the first row's top border is added with the others).
            let mac = jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac;
            let minimum = cells
                .iter()
                .filter(|cell| !cell.skip_render)
                .map(|cell| {
                    let padding = if cell_ink_projection_is_horizontal(cell.text_direction) {
                        cell.padding.vertical()
                    } else {
                        0.0
                    };
                    let border_band = if mac {
                        cell.bottom_border_extent()
                    } else {
                        (cell.top_border_extent() + cell.bottom_border_extent()) * 0.5
                    };
                    points_to_px(explicit_height) + padding + border_band
                })
                .fold(points_to_px(explicit_height), f64::max);
            row_height = row_height.max(minimum);
        }
        _ => {}
    }

    let visible_cells = cells.iter().filter(|cell| !cell.skip_render);
    let only_hidden_end_markers = visible_cells.clone().next().is_some()
        && visible_cells
            .clone()
            .all(|cell| cell.blocks.is_empty() && cell.hidden_end_marker_paragraph_count > 0);
    if row.height.is_none() && only_hidden_end_markers {
        // `w:hideMark` explicitly removes Word's mandatory end-of-cell marker
        // from auto-height measurement. Do not put the generic grabbable-row
        // floor back after `prepare_row_layout` removed that line. The painted
        // band still needs enough extent for its insets and horizontal borders.
        return visible_cells
            .map(|cell| {
                cell.height + (cell.border_top.width_px + cell.border_bottom.width_px) / 2.0
            })
            .fold(0.0_f64, f64::max);
    }
    row_height.max(MIN_ROW_HEIGHT_PX)
}

fn cell_ink_projection_is_horizontal(direction: Option<TextDirection>) -> bool {
    direction.is_none_or(|direction| direction == TextDirection::LrTb)
}

fn intersect_vertical_clips(
    local: Option<(f64, f64)>,
    ancestor: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    match (local, ancestor) {
        (Some((lt, lb)), Some((at, ab))) => Some((lt.max(at), lb.min(ab))),
        (clip, None) | (None, clip) => clip,
    }
}

/// Capture-local, bounded borrowed keys; no heap or persistent font cache.
#[derive(Default)]
struct ClippedGlyphInkCache<'a> {
    slots: [Option<(&'a str, (f64, f64))>; 16],
    next: usize,
}

impl<'a> ClippedGlyphInkCache<'a> {
    fn clear(&mut self) {
        self.slots.fill(None);
        self.next = 0;
    }

    fn measure<E>(
        &mut self,
        text: &'a str,
        measure: impl FnOnce() -> Result<(f64, f64), E>,
    ) -> Result<(f64, f64), E> {
        if let Some((_, ink)) = self.slots.iter().flatten().find(|(key, _)| *key == text) {
            return Ok(*ink);
        }
        let ink = measure()?;
        self.slots[self.next] = Some((text, ink));
        self.next = (self.next + 1) % self.slots.len();
        Ok(ink)
    }
}

/// Unknown ink metrics retain source glyphs; a font-wide box would turn
/// completely clipped x-height letters into apparently visible glyphs.
fn glyph_ink_intersects_vertical_clip(
    baseline: f64,
    ascent: f64,
    descent: f64,
    top: f64,
    bottom: f64,
) -> bool {
    if top.is_finite() && bottom.is_finite() && bottom <= top {
        return false;
    }
    if [baseline, ascent, descent, top, bottom]
        .iter()
        .any(|v| !v.is_finite())
        || ascent + descent <= 0.0
    {
        return true;
    }
    baseline - ascent < bottom && baseline + descent > top
}

fn visible_border_width(border: &CellBorderSpec) -> f64 {
    if matches!(border.kind, BorderKind::None) {
        0.0
    } else if border.kind == BorderKind::Double {
        jade_docx_layout::border_geometry::double_border_metrics(
            border.width_px,
            jade_docx_layout::line_metrics::word_layout_platform(),
        )
        .layout_width
    } else {
        border.width_px
    }
}

/// Segments replace the fallback rule when painted. Reserve their widest
/// visible winner; aligned edges keep the allocation-free scalar path.
fn horizontal_segment_has_extent(segment: &HorizontalBorderSegment) -> bool {
    segment.start_px.is_finite()
        && segment.end_px.is_finite()
        && segment.end_px > 0.0
        && segment.end_px - segment.start_px > f64::EPSILON
}

fn horizontal_border_extent(
    fallback: &CellBorderSpec,
    segments: &[HorizontalBorderSegment],
) -> f64 {
    if segments.is_empty() {
        return visible_border_width(fallback);
    }
    segments
        .iter()
        .filter(|segment| horizontal_segment_has_extent(segment))
        .map(|segment| visible_border_width(&segment.border))
        .fold(0.0_f64, f64::max)
}

fn word_modern_nested_border_frame_enabled(compatibility_mode: Option<u32>) -> bool {
    jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
        && compatibility_mode.is_some_and(|mode| mode >= 15)
}

/// Word's saved modern nested grids reserve the parent's two half-strokes
/// outside its cell margins. Ordinary paragraph text keeps its text extent.
fn nested_border_frame_insets(
    enabled: bool,
    left: &CellBorderSpec,
    right: &CellBorderSpec,
) -> [f64; 2] {
    if enabled {
        [
            visible_border_width(left) * 0.5,
            visible_border_width(right) * 0.5,
        ]
    } else {
        [0.0, 0.0]
    }
}

/// Grows auto/at-least rows by their drawn horizontal borders, as Word does:
/// a row's height is its content plus the border below it, and the first row
/// also holds the table's top border above its text. Word for Mac prints
/// `system-dev-contract.docx` (0.5pt borders, no vertical cell margins) with
/// the header text 0.5pt lower than an unbordered row, 15.6pt per 15.07pt text
/// row, and the paragraph after the table 0.5pt below the last row. Returns
/// the last visible row's border width, which then stands in for the default
/// gap after a table.
fn add_border_extents(
    table: &Table,
    prepared_rows: &mut [Vec<PreparedCell>],
    hidden_rows: &[bool],
    row_heights: &mut [f64],
    exclude_merge_owners: bool,
) -> f64 {
    let mut last_visible = 0.0;
    let mut first_visible = true;
    let mut previous_row_grows = false;
    let spans = {
        let row_refs: Vec<&[PreparedCell]> = prepared_rows.iter().map(Vec::as_slice).collect();
        compute_vertical_merge_row_spans(&row_refs)
    };
    let merge_edges = (exclude_merge_owners
        && jade_docx_layout::line_metrics::word_layout_platform()
            == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac)
        .then(|| vertical_merge_edges(prepared_rows, &spans));
    let mac = jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac;
    // The last visible row, the bottom border band it added and whether it grows.
    let mut previous: Option<(usize, f64, bool)> = None;
    for (index, (row, cells)) in table.rows.iter().zip(prepared_rows.iter_mut()).enumerate() {
        if hidden_rows.get(index).copied().unwrap_or(false) {
            continue;
        }
        // Word for Mac draws an exact row exactly as tall as declared, and
        // the borders between it and a growing row inside the growing row:
        // the border above it then belongs to the exact row and the one below
        // to the growing row after it (`table-row-height-defaults`,
        // `table-hidden-rows`: 24.48pt, 50.16pt and 24.48pt between the
        // borders of two-line rows around a 50pt exact one).
        let exact_below_growing =
            mac && !row_grows_for_borders(row) && previous.is_some_and(|(_, _, grows)| grows);
        if mac && row_grows_for_borders(row) && previous.is_some_and(|(_, _, grows)| !grows) {
            first_visible = true;
        }
        if exact_below_growing {
            if let Some((previous_index, previous_bottom, _)) = previous {
                if let Some(previous_height) = row_heights.get_mut(previous_index) {
                    *previous_height -= previous_bottom;
                }
            }
            previous_row_grows = false;
        }
        let Some(height) = row_heights.get_mut(index) else {
            continue;
        };
        last_visible = add_row_border_extents(
            row,
            cells,
            height,
            &mut first_visible,
            &mut previous_row_grows,
            spans
                .get(index)
                .map(Vec::as_slice)
                .filter(|_| exclude_merge_owners),
            merge_edges
                .as_ref()
                .and_then(|rows| rows.get(index))
                .map(Vec::as_slice),
        );
        previous = Some((index, last_visible, row_grows_for_borders(row)));
    }
    last_visible
}

/// Whether `table` is laid out as a floating table (`w:tblpPr`). Word for
/// Mac lays a table out inline when its `w:tblpPr` comes before
/// `w:tblStyle`, against the schema's order (the five table-floating
/// fixtures; the same table with the elements in order floats).
pub(super) fn table_floats(table: &Table) -> bool {
    table
        .style
        .get("table-position")
        .is_some_and(|value| value.eq_ignore_ascii_case("floating"))
        && !(table.style.contains_key("table-position-before-style")
            && jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac)
}

/// For each cell of a `w:vMerge` region, whether the region goes on below
/// its row (`Some(true)`: the owner of a span over more rows and every
/// continuation but the last) or ends there (`Some(false)`: the last
/// continuation); `None` for cells outside a merge.
fn vertical_merge_edges(
    rows: &[Vec<PreparedCell>],
    spans: &[Vec<usize>],
) -> Vec<Vec<Option<bool>>> {
    let mut edges: Vec<Vec<Option<bool>>> = rows.iter().map(|row| vec![None; row.len()]).collect();
    for (row_index, row) in rows.iter().enumerate() {
        for (cell_index, owner) in row.iter().enumerate() {
            let span = spans[row_index].get(cell_index).copied().unwrap_or(1);
            if span < 2 {
                continue;
            }
            edges[row_index][cell_index] = Some(true);
            let last = (row_index + span - 1).min(rows.len() - 1);
            for below in (row_index + 1)..=last {
                if let Some(position) = rows[below].iter().position(|cell| {
                    cell.skip_render
                        && cell.column_index == owner.column_index
                        && cell.span == owner.span
                }) {
                    edges[below][position] = Some(below < last);
                }
            }
        }
    }
    edges
}

/// Word for Mac's height of a row holding part of a `w:vMerge` region. The
/// row ends where its tallest cell's bottom margin and bottom border end,
/// less the widest bottom border in the row, and the next row's top border
/// follows. The merged cells count with their own bottom margin and declared
/// bottom border, also where the region goes on below and nothing is drawn:
/// in yokohama_table a 1.5pt merged bottom border under continuations
/// without `w:tcMar` makes each row it passes 1.25pt shorter, and 8pt of
/// continuation bottom margin makes each row 4pt taller.
/// Returns the bordered row height and the bottom border drawn under it.
fn mac_row_height_with_vertical_merges(
    cells: &[PreparedCell],
    edges: &[Option<bool>],
    content: f64,
    content_cells: &[usize],
) -> Option<(f64, f64)> {
    let edge = |index: usize| edges.get(index).copied().flatten();
    if !(0..cells.len()).any(|index| edge(index).is_some()) {
        return None;
    }
    let drawn_bottom = cells
        .iter()
        .enumerate()
        .filter(|(index, cell)| match edge(*index) {
            Some(continues) => !continues,
            None => !cell.skip_render,
        })
        .map(|(_, cell)| cell.bottom_border_extent())
        .fold(0.0_f64, f64::max);
    let merged = cells
        .iter()
        .enumerate()
        .filter(|(index, _)| edge(*index).is_some())
        .map(|(_, cell)| cell);
    let widest_bottom = merged
        .clone()
        .map(PreparedCell::bottom_border_extent)
        .fold(drawn_bottom, f64::max);
    let text = content_cells
        .iter()
        .map(|&index| cells[index].height - cells[index].padding.bottom)
        .fold(0.0_f64, f64::max);
    let ends = content_cells
        .iter()
        .map(|&index| cells[index].height + cells[index].bottom_border_extent())
        .chain(merged.map(|cell| text + cell.padding.bottom + cell.bottom_border_extent()))
        .fold(content, f64::max);
    Some((ends - widest_bottom + drawn_bottom, drawn_bottom))
}

fn row_grows_for_borders(row: &TableRow) -> bool {
    !matches!(
        row.height_rule.unwrap_or(TableRowHeightRule::Auto),
        TableRowHeightRule::Exact
    ) || row.height.is_none()
}

/// `row_spans` (from [`compute_vertical_merge_row_spans`]) keeps a
/// `w:vMerge` owner spanning more rows from sizing this one.
fn add_row_border_extents(
    row: &TableRow,
    cells: &mut [PreparedCell],
    height: &mut f64,
    first_visible: &mut bool,
    previous_row_grows: &mut bool,
    row_spans: Option<&[usize]>,
    merge_edges: Option<&[Option<bool>]>,
) -> f64 {
    let grows = row_grows_for_borders(row);
    let mut bottom = cells
        .iter()
        .filter(|cell| !cell.skip_render)
        .map(|cell| cell.bottom_border_extent())
        .fold(0.0_f64, f64::max);
    let top = if *first_visible {
        cells
            .iter()
            .filter(|cell| !cell.skip_render)
            .map(|cell| cell.top_border_extent() + cell.cell_spacing)
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };
    if grows {
        // The declared minimum already budgets the shared border band;
        // only larger measured content grows it here.
        // Vertically stacked text fills whatever height the row has,
        // so those cells do not size it.
        let content_cells: Vec<usize> = cells
            .iter()
            .enumerate()
            .filter(|(cell_index, cell)| {
                !cell.skip_render
                    && !cell
                        .text_direction
                        .is_some_and(direction_requires_vertical_stack)
                    && !row_spans.is_some_and(|spans| spans_merged_owner(spans, *cell_index))
            })
            .map(|(cell_index, _)| cell_index)
            .collect();
        let content = content_cells
            .iter()
            .map(|&index| cells[index].height)
            .fold(0.0_f64, f64::max);
        match merge_edges.and_then(|edges| {
            mac_row_height_with_vertical_merges(cells, edges, content, &content_cells)
        }) {
            Some((bordered_content, drawn_bottom)) => {
                // The row was sized from its content: a shorter Word row
                // takes the difference off that height, a taller one grows it.
                let shortfall = (content + bottom - bordered_content).max(0.0);
                *height = (*height + top).max(content + top + bottom) - shortfall;
                *height = height.max(bordered_content + top);
                bottom = drawn_bottom;
            }
            // On Mac a declared minimum stays below the first row's top
            // border, as content does.
            None if jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac =>
            {
                *height = (*height + top).max(content + top + bottom);
            }
            None => *height = height.max(content + top + bottom),
        }
        if top > 0.0 {
            for cell in cells.iter_mut() {
                cell.padding.top += top;
                cell.height += top;
                cell.page_top_inset = top;
            }
        }
    } else if !*previous_row_grows {
        // Auto rows already reserve the shared border below their content.
        // Exact rows keep their height and reserve it inside the next row.
        for cell in cells.iter_mut() {
            let inset = cell.top_border_extent();
            cell.padding.top += inset;
            cell.height += inset;
            cell.page_top_inset = inset;
        }
    }
    *first_visible = false;
    *previous_row_grows = grows;
    bottom
}

/// `w:hidden` hides the row-end marker, not visible cell content.
/// A row collapses only when its content and paragraph markers are hidden too.
fn table_row_is_hidden(row: &TableRow) -> bool {
    row.style
        .get("hidden")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        && row.cells.iter().all(|cell| {
            cell.blocks.iter().all(|block| match block {
                BodyElement::Paragraph(paragraph) => {
                    let hides_content = |properties: Option<&RunProperties>| {
                        properties.is_some_and(|props| {
                            props.vanish == Some(true) || props.spec_vanish == Some(true)
                        })
                    };
                    hides_content(
                        paragraph
                            .properties
                            .as_ref()
                            .and_then(|props| props.mark_run_properties.as_ref()),
                    ) && paragraph.runs.iter().all(|run| {
                        hides_content(run.properties.as_ref())
                            || (run.text.is_empty()
                                && run.image.is_none()
                                && run.text_box.is_none()
                                && run.math.is_none()
                                && run.field.is_none()
                                && run.note_separator.is_none()
                                && run.footnote_reference_ids.is_empty()
                                && run.endnote_reference_ids.is_empty()
                                && !run.has_footnote_ref
                                && !run.has_endnote_ref)
                    })
                }
                BodyElement::Table(table) => table.rows.iter().all(table_row_is_hidden),
                BodyElement::Math(_) => false,
            })
        })
}

/// Counts the leading header rows of a table (`w:tblHeader`, ECMA-376 Part 1
/// §17.4.41 / §17.4.50). Word only repeats the *contiguous block of header
/// rows at the very top of the table* across page breaks, so a non-header row
/// terminates the block even if a later row is flagged again. The returned
/// count is the number of rows that should be redrawn at the top of every
/// continuation page when the table spans more than one page.
///
/// Pure decision core for the header-repetition feature. The streaming-render
/// loop calls this once per table and feeds the result to
/// [`should_repeat_header_before_row`] / [`Self::draw_repeated_header_rows`].
fn leading_header_row_count(rows: &[TableRow]) -> usize {
    rows.iter()
        .take_while(|row| row.is_header && !table_row_is_hidden(row))
        .count()
}

/// Total vertical space (px) occupied by the repeated header block — the sum of
/// the first `header_row_count` row heights. Returns `0.0` when there are no
/// header rows (so callers can treat "no repeat" as a zero-height reservation).
/// `header_row_count` is clamped to the available `row_heights` length so a
/// malformed table cannot index out of bounds.
fn repeated_header_height(row_heights: &[f64], header_row_count: usize) -> f64 {
    let count = header_row_count.min(row_heights.len());
    row_heights[..count].iter().copied().sum()
}

/// Decides whether the repeated header block must be redrawn *before* a body row
/// that has just started a new page. The header is repeated only when (a) the
/// table actually has at least one header row, (b) the row about to be drawn is
/// itself a body row (its index is at or beyond the header block, so we never
/// repeat the header in front of the original header rows), and (c) a page
/// boundary was just crossed mid-table (`page_advanced`). The very first page's
/// header is the table's own leading rows, so no repeat happens there.
fn should_repeat_header_before_row(
    header_row_count: usize,
    row_index: usize,
    page_advanced: bool,
) -> bool {
    header_row_count > 0 && row_index >= header_row_count && page_advanced
}

fn prepared_row_note_anchors(cells: &[PreparedCell]) -> Vec<&TableNoteAnchor> {
    let mut anchors = Vec::new();
    for cell in cells.iter().filter(|cell| !cell.skip_render) {
        for block in &cell.blocks {
            if let CellBlockKind::Paragraph(paragraph) = &block.kind {
                anchors.extend(paragraph.note_anchors.iter().filter(|anchor| {
                    paragraph.lines.iter().any(|line| {
                        anchor.offset >= line.start
                            && (anchor.offset < line.end.max(line.start.saturating_add(1))
                                || (anchor.offset == line.end
                                    && matches!(line.break_kind, LineBreakKind::ParagraphEnd)))
                    })
                }));
            }
        }
    }
    anchors
}

fn split_cell_paragraph(
    block: &CellBlock,
    available: f64,
    fresh_page_height: f64,
) -> Option<(CellBlock, CellBlock)> {
    let CellBlockKind::Paragraph(paragraph) = &block.kind else {
        return None;
    };
    if paragraph.vertical_layout.is_some()
        || paragraph.lines.len() < 2
        || (paragraph.keep_lines && block.height <= fresh_page_height + ROW_SPLIT_EPSILON)
        || paragraph.line_height <= 0.0
    {
        return None;
    }
    let room = (available - paragraph.spacing_before).max(0.0) + ROW_SPLIT_EPSILON;
    let mut used = 0.0;
    let mut fits = paragraph
        .lines
        .iter()
        .take_while(|line| {
            used += paragraph.line_advance(line);
            used <= room
        })
        .count();
    if paragraph.widow_control {
        use jade_docx_layout::line_metrics::{resolve_widow_orphan_decision, WidowOrphanDecision};
        match resolve_widow_orphan_decision(paragraph.lines.len(), fits) {
            WidowOrphanDecision::MoveWholeParagraph => return None,
            WidowOrphanDecision::BreakBeforeLine(line) => fits = line,
            WidowOrphanDecision::FlowNormally => {}
        }
    }
    if fits == 0 || fits >= paragraph.lines.len() {
        return None;
    }
    let start = paragraph.lines[fits].start;
    let head_paragraph = paragraph.fragment(0..fits, paragraph.segment_start);
    let tail_paragraph = paragraph.fragment(fits..paragraph.lines.len(), start);
    let head = CellBlock {
        height: head_paragraph.height
            + head_paragraph.spacing_before
            + head_paragraph.spacing_after,
        kind: CellBlockKind::Paragraph(head_paragraph),
        paragraph_indices: block.paragraph_indices.clone(),
        source_index: block.source_index,
    };
    let tail = CellBlock {
        height: tail_paragraph.height
            + tail_paragraph.spacing_before
            + tail_paragraph.spacing_after,
        kind: CellBlockKind::Paragraph(tail_paragraph),
        paragraph_indices: block.paragraph_indices.clone(),
        source_index: block.source_index,
    };
    Some((head, tail))
}

fn split_prepared_row_for_available_height(
    row_cells: &[PreparedCell],
    available_height: f64,
    line_height: f64,
    full_page_content_height: f64,
    tail_has_header: bool,
) -> Option<SplitPreparedRow> {
    if available_height <= ROW_SPLIT_EPSILON {
        return None;
    }

    let mut head_cells = Vec::with_capacity(row_cells.len());
    let mut tail_cells = Vec::with_capacity(row_cells.len());
    let mut has_tail = false;
    let mut has_head_content = false;

    for cell in row_cells {
        if cell.skip_render {
            head_cells.push(cell.clone());
            tail_cells.push(cell.clone());
            continue;
        }

        if cell.blocks.is_empty() {
            let mut head_cell = cell.clone();
            head_cell.height = compute_split_cell_height(&head_cell, line_height);
            head_cells.push(head_cell.clone());

            let mut tail_cell = cell.clone();
            tail_cell.blocks.clear();
            // The hidden end-marker paragraph belongs to the head segment. A
            // tall sibling can create an otherwise empty tail clone; reserving
            // it again there would shift all later paragraph indices.
            tail_cell.hidden_end_marker_paragraph_count = 0;
            tail_cell.height = compute_split_cell_height(&tail_cell, line_height);
            tail_cells.push(tail_cell);
            continue;
        }

        // `w:tcMar` / `w:tblCellMar` are resolved per cell during row
        // preparation. Page splitting must use those effective insets too;
        // subtracting the renderer default here would reserve phantom space
        // for table styles that explicitly set top/bottom margins to zero.
        let content_limit = (available_height
            - cell.padding.vertical()
            - 2.0 * cell.cell_spacing
            - cell.bottom_border_extent())
        .max(0.0);
        let mut head_blocks = Vec::new();
        let mut tail_blocks = Vec::new();
        let mut consumed = 0.0;
        for (index, block) in cell.blocks.iter().enumerate() {
            if consumed + block.height <= content_limit + ROW_SPLIT_EPSILON {
                consumed += block.height;
                head_blocks.push(block.clone());
                continue;
            }
            if matches!(block.kind, CellBlockKind::Spacing) {
                let remaining = (content_limit - consumed).max(0.0);
                if remaining > ROW_SPLIT_EPSILON {
                    let mut head = block.clone();
                    head.height = remaining;
                    head_blocks.push(head);
                    let mut tail = block.clone();
                    tail.height -= remaining;
                    tail_blocks.push(tail);
                    tail_blocks.extend_from_slice(&cell.blocks[index + 1..]);
                    break;
                }
            }
            if cell
                .text_direction
                .is_none_or(|direction| !direction_requires_vertical_stack(direction))
            {
                if let CellBlockKind::Paragraph(paragraph) = &block.kind {
                    // Consume the shared layout's existing lines without
                    // rewrapping or rebasing glyph/run/selection offsets.
                    let line_count = paragraph.lines.len();
                    let advance = paragraph.height / line_count.max(1) as f64;
                    let head_count = if advance > ROW_SPLIT_EPSILON {
                        ((content_limit - consumed - paragraph.spacing_before + ROW_SPLIT_EPSILON)
                            / advance)
                            .max(0.0) as usize
                    } else {
                        0
                    };
                    if line_count > 0 && head_count >= line_count {
                        // All text fits. Continue only the overflowing margin,
                        // rather than moving this paragraph's text to the next page.
                        let remaining = (content_limit
                            - consumed
                            - paragraph.spacing_before
                            - paragraph.height)
                            .max(0.0);
                        let mut head = paragraph.clone();
                        head.spacing_after = remaining;
                        head_blocks.push(CellBlock {
                            height: head.height + head.spacing_before + remaining,
                            kind: CellBlockKind::Paragraph(head),
                            paragraph_indices: block.paragraph_indices.clone(),
                            source_index: block.source_index,
                        });
                        tail_blocks.push(CellBlock {
                            height: paragraph.spacing_after - remaining,
                            kind: CellBlockKind::Spacing,
                            paragraph_indices: Vec::new(),
                            source_index: block.source_index,
                        });
                        tail_blocks.extend_from_slice(&cell.blocks[index + 1..]);
                        break;
                    }
                    if let Some((head, tail)) = split_cell_paragraph(
                        block,
                        (content_limit - consumed).max(0.0),
                        (full_page_content_height
                            - cell.padding.vertical()
                            - 2.0 * cell.cell_spacing)
                            .max(0.0),
                    ) {
                        head_blocks.push(head);
                        tail_blocks.push(tail);
                        tail_blocks.extend_from_slice(&cell.blocks[index + 1..]);
                        break;
                    }
                }
            }
            if let CellBlockKind::Table(table) = &block.kind {
                if let Some((head, tail)) = split_nested_table_for_available_height(
                    table,
                    content_limit - consumed,
                    (full_page_content_height - cell.padding.vertical() - 2.0 * cell.cell_spacing)
                        .max(0.0),
                ) {
                    head_blocks.push(CellBlock {
                        height: head.height,
                        kind: CellBlockKind::Table(head),
                        paragraph_indices: Vec::new(),
                        source_index: block.source_index,
                    });
                    tail_blocks.push(CellBlock {
                        height: tail.height,
                        kind: CellBlockKind::Table(tail),
                        paragraph_indices: Vec::new(),
                        source_index: block.source_index,
                    });
                    tail_blocks.extend_from_slice(&cell.blocks[index + 1..]);
                    break;
                }
            }
            tail_blocks.extend_from_slice(&cell.blocks[index..]);
            break;
        }
        has_head_content |= !head_blocks.is_empty();
        let cell_has_tail = !tail_blocks.is_empty();
        let mut head_cell = cell.fragment(head_blocks);
        if cell_has_tail {
            head_cell.hidden_end_marker_paragraph_count = 0;
        }
        head_cell.height = compute_split_cell_height(&head_cell, line_height);

        let mut tail_cell = cell.fragment(tail_blocks);
        if cell.selection_paragraph_indices.is_empty() {
            head_cell.selection_paragraph_indices = collect_cell_paragraph_indices(cell);
            tail_cell.selection_paragraph_indices = head_cell.selection_paragraph_indices.clone();
        }
        tail_cell.padding.top -= tail_cell.page_top_inset;
        tail_cell.page_top_inset = if tail_has_header {
            0.0
        } else {
            tail_cell.top_border_extent()
        };
        tail_cell.padding.top += tail_cell.page_top_inset;
        if !cell_has_tail {
            tail_cell.hidden_end_marker_paragraph_count = 0;
        }
        tail_cell.height = compute_split_cell_height(&tail_cell, line_height);
        head_cells.push(head_cell);
        tail_cells.push(tail_cell);
        has_tail |= cell_has_tail;
    }

    if !has_tail || !has_head_content {
        return None;
    }

    let fragment_height = |cells: &[PreparedCell]| {
        cells
            .iter()
            .filter(|cell| !cell.skip_render)
            .map(|cell| cell.height + cell.bottom_border_extent())
            .fold(MIN_ROW_HEIGHT_PX, f64::max)
    };
    let head_height = fragment_height(&head_cells);
    if head_height > available_height + ROW_SPLIT_EPSILON {
        return None;
    }

    let tail_height = fragment_height(&tail_cells);
    if tail_height <= ROW_SPLIT_EPSILON {
        return None;
    }

    Some(SplitPreparedRow {
        head_cells,
        head_height,
        tail_cells,
        tail_height,
    })
}

/// Partition cached shared-layout blocks, recursively, before painting. A nested
/// table fragment never changes the host page while an enclosing cell is clipped.
fn split_nested_table_for_available_height(
    layout: &NestedTableLayout,
    available_height: f64,
    full_page_content_height: f64,
) -> Option<(NestedTableLayout, NestedTableLayout)> {
    // Word ignores repeated-header flags in nested tables. Only ancestor
    // insets reduce their full-page budget; outer headers are reserved by callers.
    let body_page_height =
        (full_page_content_height - layout.margin_top - layout.margin_bottom).max(0.0);
    let mut consumed = layout.margin_top;
    let mut head_rows = Vec::new();
    let mut tail_rows = Vec::new();
    for (index, row) in layout.rows.iter().enumerate() {
        if consumed + row.height <= available_height + ROW_SPLIT_EPSILON {
            consumed += row.height;
            head_rows.push(row.clone());
            continue;
        }
        let source = layout.table.rows.get(row.source_index)?;
        let can_split =
            !cant_split_blocks_row_split(source.cant_split, row.height, body_page_height)
                && !row_height_blocks_content_split(source, &row.cells);
        if can_split {
            if let Some(split) = split_prepared_row_for_available_height(
                &row.cells,
                available_height - consumed,
                layout.line_height,
                body_page_height,
                false,
            ) {
                head_rows.push(NestedRowLayout {
                    cells: split.head_cells,
                    height: split.head_height,
                    ..row.clone()
                });
                tail_rows.push(NestedRowLayout {
                    cells: split.tail_cells,
                    height: split.tail_height,
                    ..row.clone()
                });
                tail_rows.extend_from_slice(&layout.rows[index + 1..]);
                break;
            }
        }
        tail_rows.extend_from_slice(&layout.rows[index..]);
        break;
    }
    // Do not strand the original headers without any body content.
    let has_body = head_rows.iter().any(|row| {
        layout
            .table
            .rows
            .get(row.source_index)
            .is_some_and(|source| !source.is_header)
    });
    if head_rows.is_empty() || tail_rows.is_empty() || !has_body {
        return None;
    }
    let mut head = layout.clone();
    head.rows = head_rows;
    head.margin_bottom = 0.0;
    head.height = head.margin_top + head.rows.iter().map(|r| r.height).sum::<f64>();
    // A vertical merge crossing this boundary needs a blank owner on the
    // continuation page. Preserve its visual properties without duplicating
    // the original owner's text or editable paragraph surfaces.
    if let Some(first_tail) = tail_rows.first_mut() {
        for cell in &mut first_tail.cells {
            if cell.skip_render && cell.vertical_merge.as_deref() == Some("continue") {
                if let Some(owner) = layout
                    .rows
                    .iter()
                    .rev()
                    .filter(|row| row.source_index < first_tail.source_index)
                    .filter_map(|row| {
                        row.cells
                            .iter()
                            .find(|candidate| candidate.column_index == cell.column_index)
                    })
                    .find(|candidate| !candidate.skip_render)
                {
                    let mut continuation = owner.clone();
                    continuation.blocks.clear();
                    continuation.logical_paragraph_count = 0;
                    continuation.hidden_end_marker_paragraph_count = 0;
                    continuation.vertical_merge = Some("restart".to_string());
                    *cell = continuation;
                }
            }
        }
    }
    let mut tail = layout.clone();
    tail.rows = tail_rows;
    tail.margin_top = 0.0;
    tail.height = tail.margin_bottom + tail.rows.iter().map(|r| r.height).sum::<f64>();
    Some((head, tail))
}

fn compute_split_cell_height(cell: &PreparedCell, line_height: f64) -> f64 {
    if cell.skip_render {
        return 0.0;
    }

    let vertical_padding = cell.padding.vertical() + 2.0 * cell.cell_spacing;
    if cell.blocks.is_empty() {
        return vertical_padding;
    }

    let content_height = cell.blocks.iter().map(|block| block.height).sum::<f64>();
    if content_height <= ROW_SPLIT_EPSILON {
        line_height + vertical_padding
    } else {
        content_height + vertical_padding
    }
}

pub(super) fn count_body_paragraphs(elements: &[BodyElement]) -> usize {
    elements
        .iter()
        .map(|element| match element {
            BodyElement::Paragraph(_) => 1,
            BodyElement::Table(table) => table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| count_body_paragraphs(&cell.blocks))
                        .sum::<usize>()
                })
                .sum(),
            BodyElement::Math(_) => 0,
        })
        .sum()
}

fn assign_nested_table_orders(cells: &mut [PreparedCell], next: &mut usize) {
    for cell in cells {
        for block in &mut cell.blocks {
            if let CellBlockKind::Table(table) = &mut block.kind {
                table.order = Some(*next);
                *next += 1;
                for row in &mut table.rows {
                    assign_nested_table_orders(&mut row.cells, next);
                }
            }
        }
    }
}

/// Assigns document-order paragraph indices to every paragraph block of a row,
/// starting at `base` and walking cells left-to-right then blocks top-to-bottom
/// — the exact order [`count_body_paragraphs`] uses for the editor's model.
///
/// This must run on the *whole* row before a tall row is divided into head/tail
/// page segments. Splitting a row renders all of its cells' heads on one page
/// and then all of their tails on the next; assigning indices lazily during that
/// two-pass render interleaves the global paragraph counter out of model order
/// (e.g. a short right-hand cell that fits entirely in the head would be numbered
/// *before* the left cell's continuation paragraphs instead of after them). The
/// model's paragraph numbering drives caret placement, selection hit-testing and
/// dirty-region invalidation, so any divergence makes typing land in the wrong
/// cell and leaves stale or duplicated content on continuation pages.
///
/// Pre-assigning here pins each block's index, and the indices survive the later
/// split clones unchanged as [`split_prepared_row_for_available_height`]
/// partitions block lists and nested rows. Returns the next free paragraph index.
/// `skip_render` continuation cells reserve `logical_paragraph_count` indices
/// without storing per-block indices, mirroring the render counter's skip path.
fn assign_row_paragraph_indices(
    cells: &mut [PreparedCell],
    base: usize,
    footnote_surface_extras: &HashMap<usize, usize>,
) -> usize {
    // After each assigned index, skip any footnote surfaces reserved behind it
    // so a footnote referenced from a table cell keeps index parity with the
    // editable surface enumeration.
    let advance = |next: &mut usize, count: usize| {
        for _ in 0..count {
            let index = *next;
            *next = next.saturating_add(1);
            if let Some(extra) = footnote_surface_extras.get(&index) {
                *next = next.saturating_add(*extra);
            }
        }
    };
    let mut next = base;
    for cell in cells.iter_mut() {
        if cell.skip_render {
            advance(&mut next, cell.logical_paragraph_count);
            continue;
        }
        for block in &mut cell.blocks {
            match &mut block.kind {
                CellBlockKind::Spacing => {}
                CellBlockKind::Paragraph(_) => {
                    block.paragraph_indices.clear();
                    block.paragraph_indices.push(next);
                    advance(&mut next, 1);
                }
                CellBlockKind::Table(table) => {
                    for row in &mut table.rows {
                        if row.hidden {
                            advance(
                                &mut next,
                                row.cells.iter().map(|c| c.logical_paragraph_count).sum(),
                            );
                        } else {
                            next = assign_row_paragraph_indices(
                                &mut row.cells,
                                next,
                                footnote_surface_extras,
                            );
                        }
                    }
                }
            }
        }
        // `w:hideMark` suppresses an otherwise real empty paragraph from the
        // visual block list. Zero-flow nested terminal markers keep their own
        // paragraph block, so they need no additional counter reservation.
        next = next.saturating_add(cell.hidden_end_marker_paragraph_count);
    }
    next
}

/// Flattens the pre-assigned paragraph indices of a cell's blocks in order, used
/// to record the cell's selection layout when the row used up-front index
/// assignment. Mirrors the `start..end` counter range used by the legacy lazy
/// path, but stays correct for split segments where the global counter does not
/// move during the segment render.
fn collect_cell_paragraph_indices(cell: &PreparedCell) -> Vec<usize> {
    cell.blocks
        .iter()
        .flat_map(|block| block.paragraph_indices.iter().copied())
        .collect()
}

/// Maps an OOXML border `w:val` token onto the renderer's [`BorderKind`].
///
/// Unknown tokens are treated as `single` so a visible stroke is always
/// drawn — silently dropping them caused every non-`single` border to look
/// identical in earlier revisions.
pub(super) fn parse_border_kind(style: Option<&String>) -> BorderKind {
    match normalize_border_style(style).as_str() {
        "none" | "nil" | "hidden" => BorderKind::None,
        // "solid" is the CSS-ish token emitted by older revisions of this
        // parser; keep it working so we don't regress on cached documents.
        "single" | "solid" => BorderKind::Single,
        "thick" => BorderKind::Thick,
        "double" => BorderKind::Double,
        "triple" => BorderKind::Triple,
        "dashed" | "dashlarge" | "dashlonglarge" | "dashlong" => BorderKind::Dashed,
        "dashsmallgap" => BorderKind::DashSmallGap,
        "dotted" | "dottedlarge" => BorderKind::Dotted,
        "dotdash" | "dashdotstroked" => BorderKind::DotDash,
        "dotdotdash" => BorderKind::DotDotDash,
        "wave" | "doublewave" => BorderKind::Wave,
        _ => BorderKind::Single,
    }
}

fn normalize_border_style(style: Option<&String>) -> String {
    style
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "single".to_string())
}

/// Resolves the three `border-<side>-{style,width,color}` keys populated by
/// the DOCX parser into a renderer-ready [`CellBorderSpec`], applying
/// sensible defaults when any of them are absent.
fn resolve_border_spec(
    style: Option<&String>,
    width: Option<&String>,
    color: Option<&String>,
) -> CellBorderSpec {
    let conflict_style = normalize_border_style(style);
    let (conflict_number, conflict_precedence) = border_conflict_rank(&conflict_style);
    let kind = parse_border_kind(style);
    if matches!(kind, BorderKind::None) {
        return CellBorderSpec::hidden();
    }
    let width_px = width
        .and_then(|value| parse_length_to_px(value))
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let color = color.cloned().unwrap_or_else(|| BORDER_COLOR.to_string());
    CellBorderSpec {
        kind,
        conflict_number,
        conflict_precedence,
        width_px,
        color,
    }
}

/// Converts a table-level `w:tblBorders` edge into a drawable [`CellBorderSpec`].
///
/// Mirrors [`resolve_border_spec`] but reads the already-parsed
/// [`crate::document::BorderEdge`] (val lowercased, `w:sz` in eighths of a
/// point, color normalized to `#RRGGBB`) instead of the CSS-style string keys
/// used for cell-level `w:tcBorders`.
pub(super) fn border_edge_to_spec(edge: &crate::document::BorderEdge) -> CellBorderSpec {
    let conflict_style = normalize_border_style(edge.val.as_ref());
    let (conflict_number, conflict_precedence) = border_conflict_rank(&conflict_style);
    let kind = match edge.val.as_deref() {
        Some(val) => parse_border_kind(Some(&val.to_string())),
        None => BorderKind::Single,
    };
    if matches!(kind, BorderKind::None) {
        return CellBorderSpec::hidden();
    }
    let width_px = edge
        .size_eighths
        .map(|eighths| points_to_px(eighths as f32 / 8.0))
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let color = edge
        .color
        .clone()
        .unwrap_or_else(|| BORDER_COLOR.to_string());
    CellBorderSpec {
        kind,
        conflict_number,
        conflict_precedence,
        width_px,
        color,
    }
}

/// Effective per-side inner padding of a table cell, in pixels.
///
/// Word resolves cell content insets from `w:tcMar` (cell level) with a
/// fallback to the table-wide `w:tblCellMar`, and finally the application
/// default. Unlike the renderer's historical uniform padding, each side is
/// tracked independently so asymmetric margins are honored.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CellPadding {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

impl CellPadding {
    fn horizontal(&self) -> f64 {
        self.left + self.right
    }

    fn vertical(&self) -> f64 {
        self.top + self.bottom
    }
}

/// Resolves the effective cell padding (px) for a table cell.
///
/// Resolution order per side: cell `w:tcMar` (precedence) → row-scoped
/// `w:tblPrEx/w:tblCellMar` → table `w:tblCellMar` → the renderer defaults
/// (`default_x` for left/right, `default_y` for top/bottom). Margin values are
/// parsed as points by the DOCX reader (DXA ÷ 20), so they are converted to px
/// here; the defaults are already px. See ECMA-376 Part 1 §17.4.43
/// (`tblCellMar`) and §17.4.42 (`tcMar`).
fn resolve_cell_padding(
    cell_margins: Option<&TableCellMargins>,
    row_margins: Option<&TableCellMargins>,
    table_margins: Option<&TableCellMargins>,
    bidi_visual: bool,
    default_x: f64,
    default_y: f64,
) -> CellPadding {
    let pick = |side: fn(&TableCellMargins) -> Option<f32>, default: f64| -> f64 {
        resolve_cell_margin_edge_points(cell_margins, row_margins, table_margins, side)
            .map(points_to_px)
            .unwrap_or(default)
    };
    let horizontal = |margins: &TableCellMargins, physical_left: bool| {
        if physical_left {
            margins.physical_left(bidi_visual)
        } else {
            margins.physical_right(bidi_visual)
        }
    };
    let pick_horizontal = |physical_left: bool| {
        cell_margins
            .and_then(|margins| horizontal(margins, physical_left))
            .or_else(|| row_margins.and_then(|margins| horizontal(margins, physical_left)))
            .or_else(|| table_margins.and_then(|margins| horizontal(margins, physical_left)))
            .map(points_to_px)
            .unwrap_or(default_x)
    };
    CellPadding {
        left: pick_horizontal(true),
        right: pick_horizontal(false),
        top: pick(|m| m.top, default_y),
        bottom: pick(|m| m.bottom, default_y),
    }
}

/// Resolves row-level spacing before the table default and converts the
/// reader's point value to canvas pixels. Invalid/negative values are clamped
/// so malformed documents cannot invert cell rectangles.
fn resolve_cell_spacing_px(row: &TableRow, table: &Table) -> f64 {
    row.cell_spacing
        .or(row.cell_spacing_exception)
        .or(table.cell_spacing)
        .map(points_to_px)
        .filter(|spacing| spacing.is_finite())
        .unwrap_or(0.0)
        .max(0.0)
}

/// Fill from the table style: the conditional formats that apply to the cell
/// (already layered in Word's order) over the style's whole-table shading.
fn table_style_background<'a>(
    table: &'a Table,
    style_format: &'a TableStyleCellFormat,
) -> Option<&'a String> {
    style_format.fill.as_ref().or_else(|| {
        table
            .resolved_style
            .as_ref()
            .and_then(|style| style.fill.as_ref())
    })
}

/// Direct table/exception shading belongs to the gaps, independently of a
/// cell's own shading. Explicit zero spacing leaves no area to paint.
fn table_spacing_background<'a>(row: &'a TableRow, table: &'a Table) -> Option<&'a str> {
    if resolve_cell_spacing_px(row, table) <= f64::EPSILON {
        return None;
    }
    row.style
        .get("background-color")
        .or_else(|| table.cell_style.get("background-color"))
        .map(String::as_str)
}

/// Paint only the gap inside this grid slot. Keeping the cell interior
/// untouched preserves its style/direct shading and any parent background.
fn paint_cell_spacing_background<P: Painter>(
    context: &P,
    background: Option<&str>,
    slot: CellBorderRect,
    cell: CellBorderRect,
) {
    let Some(background) = background else {
        return;
    };
    let left = cell.x - slot.x;
    let top = cell.y - slot.y;
    let right = slot.x + slot.width - cell.x - cell.width;
    let bottom = slot.y + slot.height - cell.y - cell.height;
    if left <= 0.0 && top <= 0.0 && right <= 0.0 && bottom <= 0.0 {
        return;
    }
    context.set_fill_style_str(background);
    for (x, y, width, height) in [
        (slot.x, slot.y, slot.width, top),
        (slot.x, cell.y + cell.height, slot.width, bottom),
        (slot.x, cell.y, left, cell.height),
        (cell.x + cell.width, cell.y, right, cell.height),
    ] {
        if width > 0.0 && height > 0.0 {
            context.fill_rect(x, y, width, height);
        }
    }
}

/// Applies the table style's conditional run formatting (`w:tblStylePr/w:rPr`)
/// to runs that do not specify the property themselves.
/// Gives cell runs the character formatting of their paragraph's style —
/// Heading 1's weight and colour — where the runs set none themselves.
///
/// Loading resolves only a style's fonts and sizes into the runs; body text
/// takes the rest from the style as it draws. Word applies a paragraph style
/// other than the default one over the table style's formatting, so it goes in
/// before the conditional table formatting. The default style stays below the
/// table style.
fn apply_paragraph_style_runs(
    styled_runs: &mut [StyledRunRange],
    paragraph: &Paragraph,
    styles: Option<&StylesPart>,
) {
    let Some(styles) = styles else {
        return;
    };
    let Some(style_id) = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
        .filter(|style_id| Some(*style_id) != styles.default_paragraph_style_id())
    else {
        return;
    };
    let Some(style) = styles.resolve_run_properties(style_id) else {
        return;
    };
    for run in styled_runs {
        let properties = run.properties.get_or_insert_with(RunProperties::default);
        macro_rules! fill {
            ($($field:ident),* $(,)?) => {
                $(if properties.$field.is_none() {
                    properties.$field = style.$field.clone();
                })*
            };
        }
        fill!(
            bold,
            bold_complex,
            italic,
            italic_complex,
            strike,
            double_strike,
            caps,
            small_caps,
            color,
            char_spacing,
            baseline,
        );
        if properties.underline.is_none() {
            properties.underline = style.underline;
            properties.underline_kind = style.underline_kind.clone();
            properties.underline_color = style.underline_color.clone();
        }
    }
}

fn apply_table_style_conditional_runs(
    styled_runs: &mut [StyledRunRange],
    style_format: &TableStyleCellFormat,
) {
    if style_format.bold.is_none() && style_format.italic.is_none() {
        return;
    }
    for run in styled_runs {
        let properties = run.properties.get_or_insert_with(RunProperties::default);
        if properties.bold.is_none() {
            properties.bold = style_format.bold;
        }
        if properties.italic.is_none() {
            properties.italic = style_format.italic;
        }
    }
}

/// Where one cell sits for the table style's conditional regions. Rows are
/// counted among visible rows; columns follow the cell order of the row.
fn table_style_cell_position(
    row: &TableRow,
    row_index: usize,
    row_count: usize,
    cell_index: usize,
) -> TableStyleCellPosition {
    TableStyleCellPosition {
        row: row_index,
        row_count,
        cell: cell_index,
        cell_count: row.cells.len(),
    }
}

/// Word resolves omitted cell shading from the table style, not from direct
/// table shading or row exceptions (MS-OI29500 §17.4.32(b)). Those properties
/// paint only cell spacing (§17.4.30(a), §17.4.31(a)).
fn resolve_cell_background(
    cell: &TableCell,
    table: &Table,
    style_format: &TableStyleCellFormat,
) -> Option<String> {
    cell.style
        .get("background-color")
        .or_else(|| table_style_background(table, style_format))
        .cloned()
}

/// Picks the `w:tblBorders` edge that applies to one side of a cell, honoring
/// the OOXML distinction between outer edges (`top`/`bottom`/`left`/`right`)
/// and interior grid lines (`insideH`/`insideV`). See ECMA-376 Part 1 §17.4.39
/// (<https://ooxml.info/docs/11/Part1/17.4.39>).
fn table_border_edge_for_side(
    borders: &crate::document::TableBorders,
    side: TableBorderSide,
    bidi_visual: bool,
    is_top_row: bool,
    is_bottom_row: bool,
    is_left_col: bool,
    is_right_col: bool,
) -> Option<&crate::document::BorderEdge> {
    match side {
        TableBorderSide::Top => {
            if is_top_row {
                borders.top.as_ref()
            } else {
                borders.inside_h.as_ref()
            }
        }
        TableBorderSide::Bottom => {
            if is_bottom_row {
                borders.bottom.as_ref()
            } else {
                borders.inside_h.as_ref()
            }
        }
        TableBorderSide::Left => {
            if is_left_col {
                borders.physical_left(bidi_visual)
            } else {
                borders.inside_v.as_ref()
            }
        }
        TableBorderSide::Right => {
            if is_right_col {
                borders.physical_right(bidi_visual)
            } else {
                borders.inside_v.as_ref()
            }
        }
    }
}

/// One painted row segment of a spaced table's outer frame.
#[derive(Clone, Copy, Debug)]
struct SpacedTableFrameSegment {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    draw_top: bool,
    draw_bottom: bool,
}

/// The outer table border for one side of a row: the row's `w:tblPrEx`
/// border set when present, otherwise the table's (or its style's) borders.
fn spaced_table_frame_edge<'a>(
    row: &'a TableRow,
    table: &'a Table,
    side: TableBorderSide,
) -> Option<&'a crate::document::BorderEdge> {
    effective_table_border_edge_for_side(row, table, side, true, true, true, true)
}

/// The physical edge a table style's conditional formats give one cell side.
fn table_style_border_for_side(
    style_format: &TableStyleCellFormat,
    side: TableBorderSide,
) -> Option<&crate::document::BorderEdge> {
    let borders = &style_format.borders;
    match side {
        TableBorderSide::Top => borders.top.as_ref(),
        TableBorderSide::Bottom => borders.bottom.as_ref(),
        TableBorderSide::Left => borders.left.as_ref(),
        TableBorderSide::Right => borders.right.as_ref(),
    }
}

/// A row exception's interior edge (`w:tblPrEx/w:tblBorders/w:insideH` or
/// `w:insideV`) for a cell side that is not on the table's perimeter.
fn row_exception_inside_edge(
    row: &TableRow,
    side: TableBorderSide,
    is_top_row: bool,
    is_bottom_row: bool,
    is_left_col: bool,
    is_right_col: bool,
) -> Option<&crate::document::BorderEdge> {
    let borders = row.borders.as_deref()?;
    match side {
        TableBorderSide::Top if !is_top_row => borders.inside_h.as_ref(),
        TableBorderSide::Bottom if !is_bottom_row => borders.inside_h.as_ref(),
        TableBorderSide::Left if !is_left_col => borders.inside_v.as_ref(),
        TableBorderSide::Right if !is_right_col => borders.inside_v.as_ref(),
        _ => None,
    }
}

fn has_document_table_border_defaults(row: &TableRow, table: &Table) -> bool {
    // A resolved package style supplies the border default even when its
    // edges are absent. Do not replace that absence with the bare SDK grid.
    row.borders.is_some() || table.borders.is_some() || table.resolved_style.is_some()
}

/// Resolve each edge through row exception, direct table and whole-table style.
/// Omitted edges inherit independently; a present nil edge remains authoritative.
#[allow(clippy::too_many_arguments)]
fn effective_table_border_edge_for_side<'a>(
    row: &'a TableRow,
    table: &'a Table,
    side: TableBorderSide,
    is_top_row: bool,
    is_bottom_row: bool,
    is_left_col: bool,
    is_right_col: bool,
) -> Option<&'a crate::document::BorderEdge> {
    let pick = |borders: &'a crate::document::TableBorders| {
        table_border_edge_for_side(
            borders,
            side,
            table.bidi_visual,
            is_top_row,
            is_bottom_row,
            is_left_col,
            is_right_col,
        )
    };
    row.borders
        .as_deref()
        .and_then(pick)
        .or_else(|| table.borders.as_ref().and_then(pick))
        .or_else(|| {
            table
                .resolved_style
                .as_ref()
                .and_then(|style| style.borders.as_ref())
                .and_then(pick)
        })
}

/// Looks up a cell border property after mapping Office 2010+ logical
/// `start`/`end` edges to the physical canvas side. Logical edges take
/// precedence over legacy `left`/`right` within each formatting level.
fn cell_border_property<'a>(
    cell: &'a TableCell,
    table: &'a Table,
    physical_side: &str,
    property: &str,
) -> Option<&'a String> {
    let logical_side = match (physical_side, table.bidi_visual) {
        ("left", false) | ("right", true) => Some("start"),
        ("right", false) | ("left", true) => Some("end"),
        _ => None,
    };
    let lookup = |style: &'a crate::document::StyleMap| {
        logical_side
            .and_then(|side| border_style_entry(style, side, property))
            .or_else(|| border_style_entry(style, physical_side, property))
    };
    lookup(&cell.style).or_else(|| lookup(&table.cell_style))
}

/// Looks up `border-{side}-{property}` in a cell style map.
///
/// Every cell asks for up to four keys per side and property on each table
/// layout, which made formatting the key the bulk of border resolution on
/// table-heavy pages. The key is assembled on the stack instead.
fn border_style_entry<'a>(
    style: &'a crate::document::StyleMap,
    side: &str,
    property: &str,
) -> Option<&'a String> {
    const PREFIX: &[u8] = b"border-";
    let len = PREFIX.len() + side.len() + 1 + property.len();
    let mut key = [0u8; 48];
    if len > key.len() {
        return style.get(&format!("border-{side}-{property}"));
    }
    let (prefix, rest) = key.split_at_mut(PREFIX.len());
    prefix.copy_from_slice(PREFIX);
    let (side_bytes, rest) = rest.split_at_mut(side.len());
    side_bytes.copy_from_slice(side.as_bytes());
    rest[0] = b'-';
    rest[1..=property.len()].copy_from_slice(property.as_bytes());
    // Joining UTF-8 strings with ASCII separators is valid UTF-8.
    style.get(std::str::from_utf8(&key[..len]).ok()?)
}

/// Resolves an explicitly supplied cell-only border such as a diagonal. There
/// is no table-level outer/interior fallback for these edges, and a completely
/// absent diagonal must remain `None` so the editor does not paint a guide line
/// across every ordinary cell.
fn optional_cell_border_spec(
    cell: &TableCell,
    table: &Table,
    side: &str,
) -> Option<CellBorderSpec> {
    cell_border_property(cell, table, side, "style").map(|style| {
        resolve_border_spec(
            Some(style),
            cell_border_property(cell, table, side, "width"),
            cell_border_property(cell, table, side, "color"),
        )
    })
}

/// Border number and equal-weight precedence from MS-OE376 §2.4.63.
///
/// The first tuple component participates in `width × number`; the second is
/// compared only when those weights tie. Unknown/CSS compatibility tokens use
/// the single-border rank so they remain deterministic without outranking a
/// recognized Word style.
fn border_conflict_rank(style: &str) -> (u8, u8) {
    match style {
        "single" | "solid" => (1, 1),
        "thick" => (2, 2),
        "double" => (3, 3),
        // Word fixes these two styles at weight 1 regardless of width/number.
        "dotted" | "dottedlarge" => (0, 4),
        "dashed" | "dashlarge" | "dashlonglarge" | "dashlong" => (0, 5),
        "dotdash" => (8, 6),
        "dotdotdash" => (9, 7),
        "triple" => (10, 8),
        "thinthicksmallgap" => (11, 9),
        "thickthinsmallgap" => (12, 10),
        "thinthickthinsmallgap" => (13, 11),
        "thinthickmediumgap" => (14, 12),
        "thickthinmediumgap" => (15, 13),
        "thinthickthinmediumgap" => (16, 14),
        "thinthicklargegap" => (17, 15),
        "thickthinlargegap" => (18, 16),
        "thinthickthinlargegap" => (19, 17),
        "wave" => (20, 18),
        "doublewave" => (21, 19),
        "dashsmallgap" => (22, 20),
        "dashdotstroked" => (23, 21),
        "threedemboss" => (24, 22),
        "threedengrave" => (25, 23),
        "outset" => (26, 24),
        "inset" => (27, 25),
        _ => (1, 1),
    }
}

fn border_conflict_weight(spec: &CellBorderSpec) -> u32 {
    if spec.conflict_number == 0 {
        return 1;
    }
    // 1pt = 4/3 CSS px and OOXML measures `w:sz` in eighth-points, so one
    // Canvas pixel equals six eighth-points. Round back to the source integer
    // before multiplying to avoid floating conversion noise on equal weights.
    let width_eighths = (spec.width_px * 6.0).round().max(0.0) as u32;
    width_eighths.saturating_mul(u32::from(spec.conflict_number))
}

/// Word compares identical-style colors in three increasingly specific
/// brightness passes: R+B+2G, then B+2G, then G. Lower values win.
fn border_color_brightness(color: &str) -> Option<(u16, u16, u16)> {
    let hex = color.trim().strip_prefix('#').unwrap_or(color.trim());
    if hex.len() != 6 {
        return None;
    }
    let red = u16::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u16::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u16::from_str_radix(&hex[4..6], 16).ok()?;
    Some((red + blue + 2 * green, blue + 2 * green, green))
}

/// Resolves Word's shared-edge conflict procedure from MS-OE376 §2.4.63.
///
/// Canvas paints cells sequentially, so resolving first makes the result
/// independent of draw order. The arguments remain in logical reading order;
/// exact ties therefore retain `first`, as Word specifies.
fn resolve_shared_border_conflict(
    first: &CellBorderSpec,
    second: &CellBorderSpec,
) -> CellBorderSpec {
    match (
        matches!(first.kind, BorderKind::None),
        matches!(second.kind, BorderKind::None),
    ) {
        (false, true) => return first.clone(),
        (true, false) => return second.clone(),
        (true, true) => return first.clone(),
        (false, false) => {}
    }

    let first_weight = border_conflict_weight(first);
    let second_weight = border_conflict_weight(second);
    if first_weight != second_weight {
        return if first_weight > second_weight {
            first.clone()
        } else {
            second.clone()
        };
    }

    let first_rank = first.conflict_precedence;
    let second_rank = second.conflict_precedence;
    if first_rank != second_rank {
        return if first_rank > second_rank {
            first.clone()
        } else {
            second.clone()
        };
    }

    if first.conflict_precedence == second.conflict_precedence {
        if let (Some(first_brightness), Some(second_brightness)) = (
            border_color_brightness(&first.color),
            border_color_brightness(&second.color),
        ) {
            if first_brightness != second_brightness {
                return if first_brightness < second_brightness {
                    first.clone()
                } else {
                    second.clone()
                };
            }
        }
    }

    first.clone()
}

/// Gives both sides of every zero-spacing vertical grid edge the same winning
/// border. Assigning the resolved spec to both cells keeps existing per-cell
/// painting and page clipping simple while preventing draw order from changing
/// Word's conflict result.
fn resolve_row_shared_vertical_border_conflicts(
    cells: &mut [PreparedCell],
    bidi_visual: bool,
    cell_spacing: f64,
) {
    if cell_spacing > f64::EPSILON {
        return;
    }

    for index in 0..cells.len().saturating_sub(1) {
        let (before, after) = cells.split_at_mut(index + 1);
        let first = &mut before[index];
        let second = &mut after[0];
        if first.skip_render
            || second.skip_render
            || first.column_index.saturating_add(first.span) != second.column_index
        {
            continue;
        }

        if bidi_visual {
            let resolved = resolve_shared_border_conflict(&first.border_left, &second.border_right);
            first.border_left = resolved.clone();
            second.border_right = resolved;
        } else {
            let resolved = resolve_shared_border_conflict(&first.border_right, &second.border_left);
            first.border_right = resolved.clone();
            second.border_left = resolved;
        }
    }
}

fn horizontal_segment_offsets(
    cell: &PreparedCell,
    overlap_start: usize,
    overlap_end: usize,
    column_widths: &[f64],
    bidi_visual: bool,
) -> (f64, f64) {
    let cell_start = cell.column_index.min(column_widths.len());
    let cell_end = cell_start
        .saturating_add(cell.span)
        .min(column_widths.len());
    let overlap_start = overlap_start.clamp(cell_start, cell_end);
    let overlap_end = overlap_end.clamp(overlap_start, cell_end);
    if bidi_visual {
        (
            column_widths[overlap_end..cell_end].iter().sum(),
            column_widths[overlap_start..cell_end].iter().sum(),
        )
    } else {
        (
            column_widths[cell_start..overlap_start].iter().sum(),
            column_widths[cell_start..overlap_end].iter().sum(),
        )
    }
}

/// Partitions one horizontal cell edge at every adjacent-cell boundary, then
/// resolves Word's winner independently for each physical overlap.
fn resolved_horizontal_segments_for_cell(
    cell: &PreparedCell,
    opponents: &[PreparedCell],
    cell_is_upper: bool,
    column_widths: &[f64],
    bidi_visual: bool,
) -> Vec<HorizontalBorderSegment> {
    if cell.skip_render || cell.cell_spacing > f64::EPSILON {
        return Vec::new();
    }
    let cell_start = cell.column_index.min(column_widths.len());
    let cell_end = cell_start
        .saturating_add(cell.span)
        .min(column_widths.len());
    if cell_start >= cell_end {
        return Vec::new();
    }
    let eligible_opponents: Vec<&PreparedCell> = opponents
        .iter()
        .filter(|opponent| {
            if opponent.skip_render || opponent.cell_spacing > f64::EPSILON {
                return false;
            }
            let opponent_start = opponent.column_index.min(column_widths.len());
            let opponent_end = opponent_start
                .saturating_add(opponent.span)
                .min(column_widths.len());
            cell_start < opponent_end && opponent_start < cell_end
        })
        .collect();
    if eligible_opponents.is_empty() {
        return Vec::new();
    }
    let mut cuts = vec![cell_start, cell_end];
    for opponent in &eligible_opponents {
        let opponent_start = opponent.column_index.min(column_widths.len());
        let opponent_end = opponent_start
            .saturating_add(opponent.span)
            .min(column_widths.len());
        cuts.push(cell_start.max(opponent_start));
        cuts.push(cell_end.min(opponent_end));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let base_border = if cell_is_upper {
        &cell.border_bottom
    } else {
        &cell.border_top
    };
    let mut segments: Vec<HorizontalBorderSegment> = Vec::new();
    for interval in cuts.windows(2) {
        let logical_start = interval[0];
        let logical_end = interval[1];
        if logical_start >= logical_end {
            continue;
        }
        let border = eligible_opponents
            .iter()
            .find(|opponent| {
                let opponent_start = opponent.column_index.min(column_widths.len());
                let opponent_end = opponent_start
                    .saturating_add(opponent.span)
                    .min(column_widths.len());
                opponent_start <= logical_start && logical_end <= opponent_end
            })
            .map(|opponent| {
                if cell_is_upper {
                    resolve_shared_border_conflict(base_border, &opponent.border_top)
                } else {
                    resolve_shared_border_conflict(&opponent.border_bottom, base_border)
                }
            })
            .unwrap_or_else(|| base_border.clone());
        let (start_px, end_px) = horizontal_segment_offsets(
            cell,
            logical_start,
            logical_end,
            column_widths,
            bidi_visual,
        );
        segments.push(HorizontalBorderSegment {
            start_px,
            end_px,
            border,
        });
    }
    segments.sort_by(|first, second| first.start_px.total_cmp(&second.start_px));
    // Adjacent intervals with the same winner need only one draw command.
    let mut merged: Vec<HorizontalBorderSegment> = Vec::with_capacity(segments.len());
    for segment in segments {
        if let Some(previous) = merged.last_mut() {
            if previous.border == segment.border
                && (previous.end_px - segment.start_px).abs() <= f64::EPSILON
            {
                previous.end_px = segment.end_px;
                continue;
            }
        }
        merged.push(segment);
    }
    merged
}

fn apply_horizontal_border_segments(
    border: &mut CellBorderSpec,
    target: &mut Box<[HorizontalBorderSegment]>,
    segments: Vec<HorizontalBorderSegment>,
    cell_width: f64,
) {
    if segments.is_empty() {
        return;
    }
    if segments.len() == 1
        && segments[0].start_px.abs() <= f64::EPSILON
        && (segments[0].end_px - cell_width).abs() <= f64::EPSILON
    {
        *border = segments[0].border.clone();
        *target = Box::default();
    } else {
        *target = segments.into_boxed_slice();
    }
}

/// Gives both sides of every zero-spacing row boundary the same Word conflict
/// winner. Differently spanned rows retain separate pixel segments so one
/// spanning edge can display multiple winners without paint-order dependence.
fn resolve_shared_horizontal_border_conflicts_between_rows(
    upper: &mut [PreparedCell],
    lower: &mut [PreparedCell],
    column_widths: &[f64],
    bidi_visual: bool,
) {
    // Keep the common aligned-grid path allocation-free; only mismatched row
    // topologies need the segment partitioner below.
    let aligned = upper.len() == lower.len()
        && upper.iter().zip(lower.iter()).all(|(above, below)| {
            above.column_index == below.column_index && above.span == below.span
        });
    if aligned {
        for (above, below) in upper.iter_mut().zip(lower.iter_mut()) {
            if above.skip_render
                || below.skip_render
                || above.cell_spacing > f64::EPSILON
                || below.cell_spacing > f64::EPSILON
            {
                continue;
            }
            let resolved = resolve_shared_border_conflict(&above.border_bottom, &below.border_top);
            above.border_bottom = resolved.clone();
            above.border_bottom_segments = Box::default();
            below.border_top = resolved;
            below.border_top_segments = Box::default();
        }
        return;
    }

    let upper_segments: Vec<_> = upper
        .iter()
        .map(|cell| {
            resolved_horizontal_segments_for_cell(cell, lower, true, column_widths, bidi_visual)
        })
        .collect();
    let lower_segments: Vec<_> = lower
        .iter()
        .map(|cell| {
            resolved_horizontal_segments_for_cell(cell, upper, false, column_widths, bidi_visual)
        })
        .collect();
    for (cell, segments) in upper.iter_mut().zip(upper_segments) {
        let cell_start = cell.column_index.min(column_widths.len());
        let cell_end = cell_start
            .saturating_add(cell.span)
            .min(column_widths.len());
        let cell_width = column_widths[cell_start..cell_end].iter().sum();
        apply_horizontal_border_segments(
            &mut cell.border_bottom,
            &mut cell.border_bottom_segments,
            segments,
            cell_width,
        );
    }
    for (cell, segments) in lower.iter_mut().zip(lower_segments) {
        let cell_start = cell.column_index.min(column_widths.len());
        let cell_end = cell_start
            .saturating_add(cell.span)
            .min(column_widths.len());
        let cell_width = column_widths[cell_start..cell_end].iter().sum();
        apply_horizontal_border_segments(
            &mut cell.border_top,
            &mut cell.border_top_segments,
            segments,
            cell_width,
        );
    }
}

/// Resolves shared horizontal edges between adjacent visible rows before
/// per-cell painting. Hidden rows are kept out of this pass because they have
/// no visual box and their border participation needs a separate Word oracle.
fn resolve_table_shared_horizontal_border_conflicts(
    rows: &mut [Vec<PreparedCell>],
    hidden_rows: &[bool],
    column_widths: &[f64],
    bidi_visual: bool,
) {
    for boundary in 0..rows.len().saturating_sub(1) {
        if hidden_rows.get(boundary).copied().unwrap_or(false)
            || hidden_rows.get(boundary + 1).copied().unwrap_or(false)
        {
            continue;
        }
        let (before, after) = rows.split_at_mut(boundary + 1);
        resolve_shared_horizontal_border_conflicts_between_rows(
            &mut before[boundary],
            &mut after[0],
            column_widths,
            bidi_visual,
        );
    }
}

#[derive(Clone, Copy)]
enum TableBorderSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// Whether any paragraph of the row (nested tables included) references a
/// footnote or an endnote.
fn table_row_has_note_references(row: &TableRow) -> bool {
    row.cells
        .iter()
        .any(|cell| blocks_have_note_references(&cell.blocks))
}

/// Whether any paragraph in `blocks` (nested tables included) references a
/// footnote or an endnote.
pub(super) fn blocks_have_note_references(blocks: &[BodyElement]) -> bool {
    blocks.iter().any(|block| match block {
        BodyElement::Paragraph(paragraph) => paragraph.runs.iter().any(|run| {
            !run.footnote_reference_ids.is_empty()
                || !run.endnote_reference_ids.is_empty()
                || run
                    .text_box
                    .as_ref()
                    .is_some_and(|text_box| blocks_have_note_references(&text_box.blocks))
        }),
        BodyElement::Table(table) => table.rows.iter().any(|row| {
            row.cells
                .iter()
                .any(|cell| blocks_have_note_references(&cell.blocks))
        }),
        BodyElement::Math(_) => false,
    })
}

/// Lays the row's text out again after its note marks changed at placement,
/// keeping everything resolved across rows (borders, paragraph indices) and
/// replacing only the cell blocks and heights. Returns the new row height.
#[allow(clippy::too_many_arguments)]
fn refresh_prepared_row_text<P: Painter<Error: PainterError>>(
    wrap_context: &TableLineWrapContext<'_, P>,
    row: &TableRow,
    table: &Table,
    column_widths: &[f64],
    config: &LayoutConfig,
    row_height_hint: Option<f64>,
    visible_row_index: usize,
    row_count: usize,
    cells: &mut [PreparedCell],
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Result<f64, JsValue> {
    // Only cells that reference notes can change: lay out just those again.
    // Keep the original paragraphs: the shared metrics and East Asian rules
    // are keyed by their addresses, so a cloned row would lose both inputs.
    let referencing: Vec<bool> = row
        .cells
        .iter()
        .map(|cell| blocks_have_note_references(&cell.blocks))
        .collect();
    let fresh = prepare_row_layout(
        wrap_context,
        row,
        table,
        column_widths,
        config,
        row_height_hint,
        visible_row_index,
        row_count,
        &mut NaturalParagraphLayoutCache::default(),
        Some(&referencing),
        field_value_for,
    )?;
    // Prepared cells follow `row.cells` one to one; when they do not, fall
    // back to laying the whole row out again.
    let aligned = fresh.len() == row.cells.len();
    let fresh = if aligned {
        fresh
    } else {
        prepare_row_layout(
            wrap_context,
            row,
            table,
            column_widths,
            config,
            row_height_hint,
            visible_row_index,
            row_count,
            &mut NaturalParagraphLayoutCache::default(),
            None,
            field_value_for,
        )?
    };
    for (index, (cell, fresh_cell)) in cells.iter_mut().zip(fresh).enumerate() {
        if (aligned && !referencing[index]) || cell.skip_render {
            continue;
        }
        let whole_cell = cell.blocks.len() == fresh_cell.blocks.len()
            && cell.blocks.iter().enumerate().all(|(index, block)| block.source_index == index
                && !matches!(&block.kind, CellBlockKind::Paragraph(paragraph) if paragraph.segment_start > 0));
        if whole_cell {
            // The common unsplit path owns the freshly laid out blocks. Move
            // them into place instead of copying their text, lines and glyphs.
            let mut blocks = fresh_cell.blocks;
            for (block, previous) in blocks.iter_mut().zip(&mut cell.blocks) {
                block.paragraph_indices = std::mem::take(&mut previous.paragraph_indices);
            }
            cell.blocks = blocks;
            cell.height =
                fresh_cell.height + cell.padding.vertical() - fresh_cell.padding.vertical();
            continue;
        }
        for block in &mut cell.blocks {
            let Some(fresh_block) = fresh_cell.blocks.get(block.source_index) else {
                continue;
            };
            let start = match &block.kind {
                CellBlockKind::Paragraph(paragraph) => paragraph.segment_start,
                _ => 0,
            };
            let indices = block.paragraph_indices.clone();
            *block = fresh_block.clone();
            block.paragraph_indices = indices;
            if let CellBlockKind::Paragraph(paragraph) = &mut block.kind {
                if start > 0 {
                    Rc::make_mut(&mut paragraph.lines).retain(|line| line.start >= start);
                    if paragraph
                        .lines
                        .first()
                        .is_none_or(|line| line.start != start)
                    {
                        let source = row
                            .cells
                            .get(index)
                            .and_then(|cell| cell.blocks.get(block.source_index));
                        let (left_inset, mut style) = match source {
                            Some(BodyElement::Paragraph(paragraph)) => {
                                config.paragraph_style(paragraph)
                            }
                            _ => (0.0, ParagraphStyle::default()),
                        };
                        style.first_line_indent = 0.0;
                        let text = paragraph.text.get(start..).unwrap_or("");
                        let styles = paragraph
                            .styled_runs
                            .iter()
                            .filter(|run| run.end > start)
                            .map(|run| StyledRunRange {
                                picture: None,
                                start: run.start.saturating_sub(start),
                                end: run.end - start,
                                properties: run.properties.clone(),
                                revision: run.revision.clone(),
                            })
                            .collect::<Vec<_>>();
                        let width = column_widths
                            .iter()
                            .skip(cell.column_index)
                            .take(cell.span)
                            .sum::<f64>()
                            - cell.padding.horizontal()
                            - 2.0 * cell.cell_spacing;
                        paragraph.lines =
                            layout_horizontal_paragraph_with_run_styles_context_offset(
                                wrap_context,
                                text,
                                (width - left_inset).max(1.0),
                                paragraph.line_height,
                                &styled_run_styles(text, &styles),
                                &style,
                                start,
                            )?
                            .lines;
                        for line in Rc::make_mut(&mut paragraph.lines) {
                            line.left_inset += left_inset;
                        }
                    }
                    paragraph.segment_start = start;
                    paragraph.is_continuation = true;
                    paragraph.spacing_before = 0.0;
                    paragraph.height = paragraph.lines_height(&paragraph.lines);
                    block.height = paragraph.height + paragraph.spacing_after;
                }
            }
        }
        cell.height = compute_split_cell_height(cell, config.line_height);
    }
    let content_height = cells
        .iter()
        .filter(|cell| !cell.skip_render)
        .map(|cell| cell.height)
        .fold(0.0_f64, f64::max);
    let bordered_content = cells
        .iter()
        .filter(|cell| {
            !cell.skip_render
                && !cell
                    .text_direction
                    .is_some_and(direction_requires_vertical_stack)
        })
        .map(|cell| cell.height + cell.bottom_border_extent())
        .fold(0.0_f64, f64::max);
    let height = resolve_row_height(
        content_height,
        config.line_height + 2.0 * config.padding_y,
        row,
        cells,
    );
    if matches!(row.height_rule, Some(TableRowHeightRule::Exact)) && row.height.is_some() {
        return Ok(height);
    }
    Ok(height.max(bordered_content))
}

#[allow(clippy::too_many_arguments)]
fn resolve_cell_border_side(
    cell: &TableCell,
    row: &TableRow,
    table: &Table,
    style_format: &TableStyleCellFormat,
    side_name: &str,
    side: TableBorderSide,
    [is_top_row, is_bottom_row, is_left_col, is_right_col]: [bool; 4],
    cell_spacing: f64,
) -> CellBorderSpec {
    // In the separated model each cell has its own perimeter. The table's
    // outside edges are painted separately by draw_spaced_table_frame_segment.
    let separated = cell_spacing > f64::EPSILON;
    let [is_top_row, is_bottom_row, is_left_col, is_right_col] = if separated {
        [false; 4]
    } else {
        [is_top_row, is_bottom_row, is_left_col, is_right_col]
    };
    let cell_spec = if cell_border_property(cell, table, side_name, "style").is_some() {
        Some(resolve_border_spec(
            cell_border_property(cell, table, side_name, "style"),
            cell_border_property(cell, table, side_name, "width"),
            cell_border_property(cell, table, side_name, "color"),
        ))
    } else {
        table_style_border_for_side(style_format, side).map(border_edge_to_spec)
    };
    if let Some(cell_spec) = cell_spec {
        // With nonzero cell spacing there is normally no border
        // conflict, but Word still resolves a row exception's insideH /
        // insideV against the cell border (MS-OI29500 §17.4.39).
        if cell_spacing > f64::EPSILON {
            if let Some(exception) = row_exception_inside_edge(
                row,
                side,
                is_top_row,
                is_bottom_row,
                is_left_col,
                is_right_col,
            ) {
                return resolve_shared_border_conflict(&cell_spec, &border_edge_to_spec(exception));
            }
        }
        return cell_spec;
    }
    if let Some(edge) = effective_table_border_edge_for_side(
        row,
        table,
        side,
        is_top_row,
        is_bottom_row,
        is_left_col,
        is_right_col,
    ) {
        return border_edge_to_spec(edge);
    }
    if separated || has_document_table_border_defaults(row, table) {
        CellBorderSpec::hidden()
    } else {
        resolve_border_spec(None, None, None)
    }
}

/// Transforms a DOCX table row into drawable cells, measuring wrapped text and resolving styles.
#[allow(clippy::too_many_arguments)]
fn prepare_row_layout<P: Painter<Error: PainterError>>(
    wrap_context: &TableLineWrapContext<'_, P>,
    row: &TableRow,
    table: &Table,
    column_widths: &[f64],
    config: &LayoutConfig,
    row_height_hint: Option<f64>,
    row_index: usize,
    row_count: usize,
    natural_paragraph_layouts: &mut NaturalParagraphLayoutCache,
    cell_filter: Option<&[bool]>,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Result<Vec<PreparedCell>, JsValue> {
    let mut layouts: Vec<PreparedCell> = Vec::new();
    let mut prefetched_border: Option<(TableStyleCellFormat, CellBorderSpec)> = None;
    let content_height_hint =
        row_height_hint.map(|hint| (hint - 2.0 * config.padding_y).max(config.line_height));

    let mut column_index = row.grid_before.unwrap_or(0) as usize;
    let cell_spacing = resolve_cell_spacing_px(row, table);
    let total_columns = column_widths.len();
    let slot_scale = if cell_spacing > f64::EPSILON {
        let edge_width = |index: usize, name, side| {
            row.cells
                .get(index)
                .map(|cell| {
                    let format = table.table_style_cell_format(table_style_cell_position(
                        row, row_index, row_count, index,
                    ));
                    visible_border_width(&resolve_cell_border_side(
                        cell,
                        row,
                        table,
                        &format,
                        name,
                        side,
                        [false; 4],
                        cell_spacing,
                    ))
                })
                .unwrap_or(0.0)
        };
        let (leading, trailing) = if table.bidi_visual {
            (
                edge_width(0, "right", TableBorderSide::Right),
                edge_width(
                    row.cells.len().saturating_sub(1),
                    "left",
                    TableBorderSide::Left,
                ),
            )
        } else {
            (
                edge_width(0, "left", TableBorderSide::Left),
                edge_width(
                    row.cells.len().saturating_sub(1),
                    "right",
                    TableBorderSide::Right,
                ),
            )
        };
        spacing_slot_scale(
            column_widths.iter().sum(),
            cell_spacing,
            leading,
            trailing,
            spacing_frame_width(row, table, TableBorderSide::Left),
            spacing_frame_width(row, table, TableBorderSide::Right),
        )
    } else {
        1.0
    };
    if column_index > total_columns {
        column_index = total_columns;
    }

    for (cell_index, cell) in row.cells.iter().enumerate() {
        let logical_paragraph_count = count_body_paragraphs(&cell.blocks);
        // Table style conditional formatting (w:tblStylePr gated by
        // w:tblLook), layered once per cell and reused for runs, shading,
        // text color and borders.
        let (style_format, mut prefetched_leading) = match prefetched_border.take() {
            Some((format, leading)) => (format, Some(leading)),
            None => (
                table.table_style_cell_format(table_style_cell_position(
                    row, row_index, row_count, cell_index,
                )),
                None,
            ),
        };
        let span = cell.span.unwrap_or(1).max(1) as usize;
        let start = column_index.min(total_columns);
        let end = (start + span).min(total_columns);
        let slice = if start < end {
            &column_widths[start..end]
        } else {
            column_widths
                .last()
                .map(std::slice::from_ref)
                .unwrap_or(&[])
        };
        let mut cell_width = slice.iter().sum::<f64>() * slot_scale;
        if cell_width <= 0.0 {
            cell_width = config.available_width / total_columns.max(1) as f64;
        }

        let skip_render = matches!(cell.vertical_merge.as_deref(), Some("continue"));
        let is_top_row = row_index == 0;
        let is_bottom_row = row_count == 0 || row_index + 1 == row_count;
        let physical_edges = |start: usize, end: usize| {
            [
                is_top_row,
                is_bottom_row,
                if table.bidi_visual {
                    end >= total_columns
                } else {
                    start == 0
                },
                if table.bidi_visual {
                    start == 0
                } else {
                    end >= total_columns
                },
            ]
        };
        let resolve_side = |name, side| {
            resolve_cell_border_side(
                cell,
                row,
                table,
                &style_format,
                name,
                side,
                physical_edges(start, end),
                cell_spacing,
            )
        };
        let border_top = resolve_side("top", TableBorderSide::Top);
        let mut border_right = if table.bidi_visual {
            prefetched_leading
                .take()
                .unwrap_or_else(|| resolve_side("right", TableBorderSide::Right))
        } else {
            resolve_side("right", TableBorderSide::Right)
        };
        let border_bottom = resolve_side("bottom", TableBorderSide::Bottom);
        let mut border_left = if table.bidi_visual {
            resolve_side("left", TableBorderSide::Left)
        } else {
            prefetched_leading
                .take()
                .unwrap_or_else(|| resolve_side("left", TableBorderSide::Left))
        };
        let border_tl2br = optional_cell_border_spec(cell, table, "tl2br");
        let border_tr2bl = optional_cell_border_spec(cell, table, "tr2bl");
        // Resolve each adjacent edge before the content width is handed to
        // jade-docx-layout. Keep only the next style/leading edge on the stack.
        if config.word_modern_border_insets && cell_spacing <= f64::EPSILON && !skip_render {
            if let Some(previous) = layouts.last().filter(|previous| {
                !previous.skip_render
                    && previous.column_index.saturating_add(previous.span) == start
            }) {
                if table.bidi_visual {
                    border_right =
                        resolve_shared_border_conflict(&previous.border_left, &border_right);
                } else {
                    border_left =
                        resolve_shared_border_conflict(&previous.border_right, &border_left);
                }
            }
            if let Some(next) = row.cells.get(cell_index + 1).filter(|next| {
                !matches!(next.vertical_merge.as_deref(), Some("continue"))
                    && start.saturating_add(span.min(total_columns.saturating_sub(start)).max(1))
                        == end
            }) {
                let format = table.table_style_cell_format(table_style_cell_position(
                    row,
                    row_index,
                    row_count,
                    cell_index + 1,
                ));
                let next_end = (end + next.span.unwrap_or(1).max(1) as usize).min(total_columns);
                let leading = resolve_cell_border_side(
                    next,
                    row,
                    table,
                    &format,
                    if table.bidi_visual { "right" } else { "left" },
                    if table.bidi_visual {
                        TableBorderSide::Right
                    } else {
                        TableBorderSide::Left
                    },
                    physical_edges(end, next_end),
                    cell_spacing,
                );
                if table.bidi_visual {
                    border_left = resolve_shared_border_conflict(&border_left, &leading);
                } else {
                    border_right = resolve_shared_border_conflict(&border_right, &leading);
                }
                prefetched_border = Some((format, leading));
            }
        }

        // Content margins remain in the cell's text frame when bidiVisual
        // mirrors the grid and borders. Word's LTR contents retain start/end
        // padding even though their cells change physical position.
        let mut padding = resolve_cell_padding(
            cell.margins.as_ref(),
            row.cell_margins.as_ref(),
            table.effective_cell_margins(),
            false,
            config.padding_x,
            config.padding_y,
        );

        let border_inset = if cell_spacing > f64::EPSILON {
            let left = visible_border_width(&border_left) * 0.5;
            let right = visible_border_width(&border_right) * 0.5;
            padding.left += left;
            padding.right += right;
            left + right
        } else {
            0.0
        };

        // Word normally reserves one line for the non-printing end-of-cell
        // marker. `w:hideMark` removes only that marker from row-height
        // measurement; actual text, drawings, fields, and nested blocks still
        // participate as usual.
        let contains_only_hidden_end_marker = cell_contains_only_hidden_end_marker(cell);
        let terminal_marker = terminal_cell_marker_after_nested_table(cell);
        let hidden_end_marker_paragraph_count = if contains_only_hidden_end_marker {
            logical_paragraph_count
        } else {
            0
        };
        let mut blocks: Vec<CellBlock> = Vec::new();
        if !skip_render
            && cell_filter.is_none_or(|filter| filter.get(cell_index).copied().unwrap_or(true))
        {
            // Cell spacing is carved out of the grid slot, not added to the
            // table width. Feed the resulting inner box to jade-docx-layout so text
            // wrapping matches the same rectangle that will be painted.
            let available_width =
                (cell_width - 2.0 * cell_spacing - border_inset - padding.horizontal()).max(1.0);
            let text_direction_available_height = if cell
                .text_direction
                .is_some_and(direction_requires_vertical_stack)
            {
                row_height_hint
            } else {
                content_height_hint
            };
            let (suppress_before, suppress_after) =
                super::compute_contextual_spacing_suppression(&cell.blocks);
            let mut previous_after = 0.0_f64;
            for (block_index, block) in cell.blocks.iter().enumerate() {
                if contains_only_hidden_end_marker && matches!(block, BodyElement::Paragraph(_)) {
                    continue;
                }
                match block {
                    BodyElement::Paragraph(paragraph) => {
                        let paragraph_line_height = config.line_height_for(paragraph);
                        let (left_inset, mut paragraph_style) = config.paragraph_style(paragraph);
                        let (text, mut styled_runs, note_anchors) = collect_paragraph_text(
                            paragraph,
                            config.render_changes,
                            config.render_original,
                            &config.note_markers,
                            field_value_for,
                        );
                        paragraph_style.inline_note_separator_offsets = note_anchors
                            .iter()
                            .filter_map(|anchor| {
                                let markers = if anchor.footnote {
                                    &config.note_markers.footnotes
                                } else {
                                    &config.note_markers.endnotes
                                };
                                let end = anchor.offset + markers.get(&anchor.id)?.len();
                                (text.as_bytes().get(end) == Some(&b' ')).then_some(end)
                            })
                            .collect();
                        apply_paragraph_style_runs(
                            &mut styled_runs,
                            paragraph,
                            config.styles_part.as_deref(),
                        );
                        apply_table_style_conditional_runs(&mut styled_runs, &style_format);
                        let fit_text = cell_fit_text_enabled(cell);
                        let vertical_fit_target = if fit_text
                            && cell
                                .text_direction
                                .is_some_and(direction_requires_vertical_stack)
                        {
                            row_height_hint.map(|height| {
                                (height - 2.0 * cell_spacing - padding.vertical()).max(1.0)
                            })
                        } else {
                            None
                        };
                        if fit_text
                            && cell.text_direction.is_none_or(|direction| {
                                !direction_requires_vertical_stack(direction)
                            })
                            && !text.is_empty()
                        {
                            // Measure the paragraph unwrapped through jade-docx-layout, then
                            // feed the resulting common ratio back as the existing run
                            // text-scale property. This keeps one source of truth for
                            // glyph advances and preserves mixed run formatting.
                            let natural = layout_paragraph_with_table_context(
                                wrap_context,
                                &text,
                                1_000_000_000.0,
                                paragraph_line_height,
                                cell.text_direction,
                                text_direction_available_height,
                                &styled_runs,
                                &paragraph_style,
                                left_inset,
                            )?;
                            let target_width =
                                (available_width - left_inset - paragraph_style.right_indent)
                                    .max(1.0);
                            styled_runs = if paragraph_style.word_platform
                                == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac
                            {
                                let border_width = if cell_spacing > f64::EPSILON {
                                    0.0
                                } else {
                                    visible_border_width(&border_left)
                                        + visible_border_width(&border_right)
                                };
                                mac_cell_fit_text_run_ranges(
                                    &text,
                                    &styled_runs,
                                    natural.max_line_width,
                                    (target_width - border_width).max(1.0),
                                )
                            } else {
                                let trailing_overhang =
                                    paragraph_trailing_overhang(wrap_context, &text, &styled_runs)?;
                                fit_text_run_ranges(
                                    &text,
                                    &styled_runs,
                                    natural.max_line_width + trailing_overhang,
                                    target_width,
                                )
                            };
                        } else if let (Some(direction), Some(target_height)) =
                            (cell.text_direction, vertical_fit_target)
                        {
                            // A vertical cell fits along its writing axis. Measure a
                            // single unbounded column first, then apply the same
                            // cell-wide run ratio consumed by jade-docx-layout.
                            let natural = layout_paragraph_with_table_context(
                                wrap_context,
                                &text,
                                available_width,
                                paragraph_line_height,
                                Some(direction),
                                None,
                                &styled_runs,
                                &paragraph_style,
                                left_inset,
                            )?;
                            let natural_advance = natural
                                .lines
                                .iter()
                                .map(|line| {
                                    if line.text.is_empty() {
                                        paragraph_line_height
                                    } else {
                                        line.visual_width.max(1.0)
                                    }
                                })
                                .sum::<f64>();
                            styled_runs = fit_text_run_ranges(
                                &text,
                                &styled_runs,
                                natural_advance,
                                target_height,
                            );
                        }
                        // Autofit measures with a plain `ParagraphStyle`; its
                        // layout is only reusable while the cell paragraph
                        // adds no East Asian rules (auto-spacing, kinsoku) or
                        // Word font line box of its own.
                        let reusable_natural_layout = (!fit_text
                            && cell.text_direction.is_none()
                            && !paragraph_style.doc_grid_snap_to_chars
                            && !paragraph_style.enable_line_break_rules
                            && !paragraph_style.word_east_asian_font_line_box
                            && left_inset == 0.0
                            && paragraph_style.first_line_indent == 0.0
                            && paragraph_style.right_indent == 0.0)
                            .then(|| {
                                take_fitting_natural_paragraph_layout(
                                    natural_paragraph_layouts,
                                    available_width,
                                    paragraph_line_height,
                                    &text,
                                    &styled_runs,
                                )
                            })
                            .flatten();
                        let mut cell_paragraph = if let Some(layout) = reusable_natural_layout {
                            layout
                        } else {
                            layout_paragraph_with_table_context(
                                wrap_context,
                                &text,
                                (available_width - left_inset).max(1.0),
                                paragraph_line_height,
                                cell.text_direction,
                                if vertical_fit_target.is_some() {
                                    None
                                } else {
                                    text_direction_available_height
                                },
                                &styled_runs,
                                &paragraph_style,
                                left_inset,
                            )?
                        };
                        if let Some(target_height) = vertical_fit_target {
                            cell_paragraph.height = target_height;
                            if let Some(layout) = cell_paragraph.vertical_layout.as_mut() {
                                layout.column_height = target_height;
                            }
                        }
                        if cell
                            .text_direction
                            .is_none_or(|direction| matches!(direction, TextDirection::LrTbV))
                        {
                            for line in Rc::make_mut(&mut cell_paragraph.lines) {
                                line.left_inset += left_inset;
                            }
                        }
                        cell_paragraph.styled_runs = styled_runs;
                        cell_paragraph.note_anchors = note_anchors;
                        cell_paragraph.keep_lines = paragraph
                            .properties
                            .as_ref()
                            .and_then(|properties| properties.keep_lines)
                            .unwrap_or(false);
                        cell_paragraph.widow_control = paragraph
                            .properties
                            .as_ref()
                            .and_then(|properties| properties.widow_control)
                            .unwrap_or(true);
                        cell_paragraph.align =
                            paragraph_text_align_for_direction(paragraph, cell.text_direction);
                        let line_height_points = (paragraph_line_height * 0.75) as f32;
                        let horizontal = cell
                            .text_direction
                            .is_none_or(|direction| matches!(direction, TextDirection::LrTbV));
                        let before = if !horizontal || suppress_before.contains(&block_index) {
                            0.0
                        } else {
                            jade_docx_layout::text_extract::spacing_before_px(
                                paragraph,
                                line_height_points,
                            )
                        };
                        let after = if !horizontal || suppress_after.contains(&block_index) {
                            0.0
                        } else {
                            jade_docx_layout::text_extract::spacing_after_px(
                                paragraph,
                                line_height_points,
                            )
                        };
                        // Adjacent paragraph margins collapse to their maximum;
                        // the preceding block has already reserved its after space.
                        cell_paragraph.spacing_before = (before - previous_after).max(0.0);
                        cell_paragraph.spacing_after = after;
                        previous_after = after;
                        // Preserve the shared empty-line geometry and formatting
                        // for caret/selection, but do not grow the cell for its
                        // mandatory marker after a nested table.
                        let height = if terminal_marker == Some(block_index) {
                            0.0
                        } else {
                            cell_paragraph.height.max(paragraph_line_height)
                                + cell_paragraph.spacing_before
                                + cell_paragraph.spacing_after
                        };
                        blocks.push(CellBlock {
                            source_index: 0,
                            kind: CellBlockKind::Paragraph(cell_paragraph),
                            height,
                            paragraph_indices: Vec::new(),
                        });
                    }
                    BodyElement::Table(table) => {
                        previous_after = 0.0;
                        let [border_left_inset, border_right_inset] = nested_border_frame_insets(
                            config.word_modern_border_insets && cell_spacing <= f64::EPSILON,
                            &border_left,
                            &border_right,
                        );
                        let nested_width =
                            (available_width - border_left_inset - border_right_inset).max(1.0);
                        let nested_layout = layout_nested_table(
                            wrap_context,
                            table.as_ref(),
                            nested_width,
                            config,
                            field_value_for,
                        )?;
                        let height = nested_layout.height.max(config.line_height); // ensure measurable height
                        blocks.push(CellBlock {
                            source_index: 0,
                            kind: CellBlockKind::Table(nested_layout),
                            height,
                            paragraph_indices: Vec::new(),
                        });
                    }
                    BodyElement::Math(math) => {
                        previous_after = 0.0;
                        let label = <CanvasRenderer>::math_placeholder_label(math);
                        let cell_paragraph = layout_paragraph_with_table_context(
                            wrap_context,
                            &label,
                            available_width,
                            config.line_height,
                            cell.text_direction,
                            text_direction_available_height,
                            &[],
                            &ParagraphStyle::default(),
                            0.0,
                        )?;
                        let height = cell_paragraph.height.max(config.line_height);
                        blocks.push(CellBlock {
                            source_index: 0,
                            kind: CellBlockKind::Paragraph(cell_paragraph),
                            height,
                            paragraph_indices: Vec::new(),
                        });
                    }
                }
            }

            if blocks.is_empty() && !contains_only_hidden_end_marker {
                let paragraph = empty_cell_paragraph(config.line_height);
                let height = paragraph.height.max(config.line_height);
                blocks.push(CellBlock {
                    source_index: 0,
                    kind: CellBlockKind::Paragraph(paragraph),
                    height,
                    paragraph_indices: Vec::new(),
                });
            }
        }

        for (index, block) in blocks.iter_mut().enumerate() {
            block.source_index = index;
        }
        let mut content_height = if skip_render {
            0.0
        } else {
            blocks.iter().map(|block| block.height).sum::<f64>()
        };
        if content_height <= 0.0 && !skip_render && !contains_only_hidden_end_marker {
            content_height = config.line_height;
        }
        // Auto-height rows reserve spacing above and below their content.
        // add_row_border_extents reserves the table's additional leading gap.
        // Exact row heights remain authoritative later.
        let cell_height = content_height + padding.vertical() + 2.0 * cell_spacing;

        let background = resolve_cell_background(cell, table, &style_format);

        let text_color = cell
            .style
            .get("color")
            .cloned()
            .or_else(|| table.cell_style.get("color").cloned())
            .or_else(|| style_format.color.clone());

        let default_align = default_align_for_direction(cell.text_direction);
        let align = cell
            .style
            .get("text-align")
            .or_else(|| table.cell_style.get("text-align"))
            .map(|value| match value.trim().to_ascii_lowercase().as_str() {
                "center" => TextAlign::Center,
                "right" | "end" => TextAlign::Right,
                _ => TextAlign::Left,
            })
            .unwrap_or(default_align);

        layouts.push(PreparedCell {
            column_index: start,
            span: span.min(total_columns.saturating_sub(start)).max(1),
            vertical_merge: cell.vertical_merge.clone(),
            border_top,
            border_top_segments: Box::default(),
            border_right,
            border_bottom,
            border_bottom_segments: Box::default(),
            border_left,
            border_tl2br,
            border_tr2bl,
            padding,
            cell_spacing,
            blocks,
            height: cell_height,
            background,
            text_color,
            align,
            text_direction: cell.text_direction,
            vertical_align_share: cell_vertical_align_share(cell.vertical_align.as_deref()),
            skip_render,
            logical_paragraph_count,
            hidden_end_marker_paragraph_count,
            page_top_inset: 0.0,
            selection_paragraph_indices: Vec::new(),
        });

        column_index = (start + span).min(total_columns);
    }

    if !config.word_modern_border_insets {
        resolve_row_shared_vertical_border_conflicts(&mut layouts, table.bidi_visual, cell_spacing);
    }
    Ok(layouts)
}

fn layout_nested_table<P: Painter<Error: PainterError>>(
    wrap_context: &TableLineWrapContext<'_, P>,
    table: &Table,
    available_width: f64,
    config: &LayoutConfig,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> Result<NestedTableLayout, JsValue> {
    let column_count = compute_column_count(table).max(1);
    let mut column_widths = compute_column_widths(table, column_count, available_width);
    let (_, mut natural_paragraph_layouts) = apply_table_layout_width_for_table_with_content(
        wrap_context,
        table,
        &mut column_widths,
        available_width,
        config.padding_x,
        &|paragraph| config.line_height_for(paragraph),
        None,
        field_value_for,
    )?;

    let mut column_positions = Vec::with_capacity(column_count + 1);
    let mut position = 0.0;
    column_positions.push(position);
    for width in &column_widths {
        position += *width;
        column_positions.push(position);
    }

    let margin_top = table
        .style
        .get("margin-top")
        .and_then(|v| parse_length_to_px(v))
        .unwrap_or(0.0);
    let margin_bottom = table
        .style
        .get("margin-bottom")
        .and_then(|v| parse_length_to_px(v))
        .unwrap_or(0.0);

    let mut rows = Vec::with_capacity(table.rows.len());
    let mut total_height = margin_top;

    let total_row_count = table
        .rows
        .iter()
        .filter(|row| !table_row_is_hidden(row))
        .count();
    let mut visible_row_index = 0usize;
    for row in &table.rows {
        let hidden = table_row_is_hidden(row);
        let row_height_hint = row.height.map(points_to_px);
        let prepared = prepare_row_layout(
            wrap_context,
            row,
            table,
            &column_widths,
            config,
            row_height_hint,
            visible_row_index,
            total_row_count,
            &mut natural_paragraph_layouts,
            None,
            field_value_for,
        )?;
        let content_height = prepared
            .iter()
            .filter(|cell| !cell.skip_render)
            .map(|cell| cell.height)
            .fold(0.0_f64, f64::max);
        let mut row_height = resolve_row_height(
            content_height,
            config.line_height + 2.0 * config.padding_y,
            row,
            &prepared,
        );
        if hidden {
            row_height = 0.0;
        } else {
            visible_row_index += 1;
        }
        rows.push(NestedRowLayout {
            source_index: rows.len(),
            cells: prepared,
            height: row_height,
            hidden,
        });
        total_height += row_height;
    }

    let mut first_visible = true;
    let mut previous_row_grows = false;
    for boundary in 0..rows.len().saturating_sub(1) {
        let (before, after) = rows.split_at_mut(boundary + 1);
        let upper = &mut before[boundary];
        let lower = &mut after[0];
        if !upper.hidden && !lower.hidden {
            resolve_shared_horizontal_border_conflicts_between_rows(
                &mut upper.cells,
                &mut lower.cells,
                &column_widths,
                table.bidi_visual,
            );
        }
        if !upper.hidden {
            let previous_height = upper.height;
            add_row_border_extents(
                &table.rows[boundary],
                &mut upper.cells,
                &mut upper.height,
                &mut first_visible,
                &mut previous_row_grows,
                None,
                None,
            );
            total_height += upper.height - previous_height;
        }
    }
    if let Some(last) = rows.last_mut().filter(|row| !row.hidden) {
        let previous_height = last.height;
        add_row_border_extents(
            &table.rows[last.source_index],
            &mut last.cells,
            &mut last.height,
            &mut first_visible,
            &mut previous_row_grows,
            None,
            None,
        );
        total_height += last.height - previous_height;
    }

    total_height += margin_bottom;
    if rows.is_empty() && total_height <= 0.0 {
        total_height = config.line_height;
    }

    let table_width = column_widths.iter().sum::<f64>().max(1.0);

    Ok(NestedTableLayout {
        table: Box::new(table.clone()),
        order: None,
        column_widths,
        column_positions,
        rows,
        width: table_width,
        height: total_height.max(config.line_height),
        margin_top,
        margin_bottom,
        line_height: config.line_height,
    })
}

fn collect_paragraph_text(
    paragraph: &Paragraph,
    render_changes: bool,
    render_original: bool,
    note_markers: &NoteMarkers,
    field_value_for: &dyn Fn(&RunField) -> Option<String>,
) -> (String, Vec<StyledRunRange>, Vec<TableNoteAnchor>) {
    let mut note_anchors = Vec::new();
    let mut text = String::new();
    let mut styled_runs = Vec::new();
    for (run_index, run) in paragraph.runs.iter().enumerate() {
        if !render_changes {
            let hidden = match run.revision.as_ref().map(|mark| &mark.kind) {
                Some(RevisionKind::Inserted | RevisionKind::MovedTo) => render_original,
                Some(RevisionKind::Deleted | RevisionKind::MovedFrom) => !render_original,
                None => false,
            };
            if hidden {
                continue;
            }
        }
        if let Some(picture) = cell_inline_picture(run, run_index) {
            let start = text.len();
            text.push(CELL_PICTURE_CHAR);
            styled_runs.push(StyledRunRange {
                start,
                end: text.len(),
                properties: run.properties.clone(),
                revision: run.revision.clone(),
                picture: Some(picture),
            });
            continue;
        }
        let marker = note_markers.marker_text_for_run(run);
        let field_text = run.field.as_ref().and_then(field_value_for);
        if run.text.is_empty()
            && field_text.as_deref().is_none_or(str::is_empty)
            && marker.is_empty()
        {
            continue;
        }
        if !run.text.is_empty() || field_text.as_deref().is_some_and(|text| !text.is_empty()) {
            let start = text.len();
            // As in body paragraphs: a field's result lives in `run.text`
            // once authored or brought up to date, and a resolved value
            // replaces it — appended, a field in a cell drew twice.
            match field_text.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => text.push_str(value),
                None => text.push_str(&run.text),
            }
            let end = text.len();
            styled_runs.push(StyledRunRange {
                picture: None,
                start,
                end,
                properties: run.properties.clone(),
                revision: run.revision.clone(),
            });
        }
        if !marker.is_empty() {
            // Word draws note references with the FootnoteReference character
            // style, i.e. superscript, as its own chunk after the run text (the
            // same shape the body paragraph path emits). An explicit run
            // baseline still wins.
            let mut properties = run.properties.clone().unwrap_or_default();
            if properties.baseline.is_none() {
                properties.baseline = Some("superscript".into());
            }
            let start = text.len();
            let mut marker_offset = start;
            for (footnote, ids, marks, ordinals) in [
                (
                    true,
                    &run.footnote_reference_ids,
                    &note_markers.footnotes,
                    &note_markers.footnote_ordinals,
                ),
                (
                    false,
                    &run.endnote_reference_ids,
                    &note_markers.endnotes,
                    &note_markers.endnote_ordinals,
                ),
            ] {
                for id in ids {
                    if let Some(mark) = marks.get(id) {
                        note_anchors.push(TableNoteAnchor {
                            id: id.clone(),
                            footnote,
                            offset: marker_offset,
                            ordinal: ordinals.get(id).copied(),
                        });
                        marker_offset += mark.len();
                    }
                }
            }
            text.push_str(&marker);
            let end = text.len();
            styled_runs.push(StyledRunRange {
                picture: None,
                start,
                end,
                properties: Some(properties),
                revision: run.revision.clone(),
            });
        }
    }
    (text, styled_runs, note_anchors)
}

/// The character an inline picture takes in a table cell's layout text, as
/// in body paragraphs. The model text has none.
const CELL_PICTURE_CHAR: char = '\u{fffc}';

fn cell_fit_text_enabled(cell: &TableCell) -> bool {
    cell.style
        .get("fit-text")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

/// Returns the ink overhang of the paragraph's final grapheme. The shared line
/// wrapper includes this value in its wrap decision but not in line advance
/// width, so `tcFitText` must add it to the target-ratio measurement or an
/// italic final glyph can push an otherwise exact fit onto a second line.
fn paragraph_trailing_overhang<C: LineWrapContext>(
    context: &C,
    text: &str,
    styled_runs: &[StyledRunRange],
) -> Result<f64, C::Error> {
    let Some((offset, grapheme)) = text.grapheme_indices(true).next_back() else {
        return Ok(0.0);
    };
    let run_styles = styled_run_styles(text, styled_runs);
    let properties = run_styles
        .iter()
        .find(|run| offset >= run.start && offset < run.end)
        .and_then(|run| run.properties.as_deref());
    let style = context.resolve_run_style(properties)?;
    let descriptor = style.descriptor_for_grapheme(grapheme, classify_script(grapheme));
    // A fixed-pitch ＭＳ 明朝 / ＭＳ ゴシック glyph stays inside its cell,
    // whatever face the host substitutes for it.
    if jade_docx_layout::line_metrics::word_monospace_advance_px(descriptor, grapheme).is_some() {
        return Ok(0.0);
    }
    Ok(context.measure_trailing_overhang(grapheme, descriptor)? * style.text_scale_x.max(0.01))
}

/// Applies the cell-wide `w:tcFitText` ratio to every run in a paragraph.
///
/// Word defines this as the cell equivalent of a run-level `w:fitText` whose
/// target is the cell text extent. Multiplying each run's existing `w:w`
/// percentage by one shared ratio preserves relative scaling and all unrelated
/// run properties. An unstyled paragraph receives a synthetic full-range style
/// solely so jade-docx-layout and the draw pass consume the same effective scale.
fn fit_text_run_ranges(
    text: &str,
    styled_runs: &[StyledRunRange],
    measured_width: f64,
    target_width: f64,
) -> Vec<StyledRunRange> {
    if text.is_empty()
        || !measured_width.is_finite()
        || measured_width <= f64::EPSILON
        || !target_width.is_finite()
        || target_width <= f64::EPSILON
    {
        return styled_runs.to_vec();
    }

    let fitted_target = (target_width - CELL_FIT_TEXT_EPSILON_PX).max(1.0);
    let ratio = fitted_target / measured_width;
    let source = if styled_runs.is_empty() {
        vec![StyledRunRange {
            picture: None,
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
        }]
    } else {
        styled_runs.to_vec()
    };

    source
        .into_iter()
        .map(|mut run| {
            let mut properties = run.properties.unwrap_or_default();
            let current_percent = properties.text_scale.unwrap_or(100.0) as f64;
            properties.text_scale = Some((current_percent * ratio).clamp(1.0, 1000.0) as f32);
            run.properties = Some(properties);
            run
        })
        .collect()
}

/// Word for Mac fits a `w:tcFitText` cell as one run-level `w:fitText` group
/// over the cell's text width less its side borders: a wider cell spreads
/// the characters (the final one keeps its advance), a narrower one condenses
/// by a whole percentage, rounded down. `table-cell-fit-text` prints `WIDE`
/// with 26.9pt gaps in a 120pt cell with 0.5pt borders, and its 179.2pt mixed
/// run at 60%, short of the cell's edge.
fn mac_cell_fit_text_run_ranges(
    text: &str,
    styled_runs: &[StyledRunRange],
    measured_width: f64,
    target_width: f64,
) -> Vec<StyledRunRange> {
    if text.is_empty()
        || !measured_width.is_finite()
        || measured_width <= f64::EPSILON
        || !target_width.is_finite()
        || target_width <= f64::EPSILON
    {
        return styled_runs.to_vec();
    }
    let fitted_width = if target_width < measured_width {
        let percent = (100.0 * target_width / measured_width + 1e-9)
            .floor()
            .max(1.0);
        measured_width * percent / 100.0
    } else {
        target_width
    };
    let fit = crate::document::RunFitText {
        // Twentieths of a point; 96 dpi px * 15.
        val: (fitted_width * 15.0).round().max(1.0) as u32,
        id: Some(MAC_CELL_FIT_TEXT_GROUP_ID),
    };
    let source = if styled_runs.is_empty() {
        vec![StyledRunRange {
            picture: None,
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
        }]
    } else {
        styled_runs.to_vec()
    };
    source
        .into_iter()
        .map(|mut run| {
            let mut properties = run.properties.unwrap_or_default();
            properties.fit_text = Some(fit.clone());
            run.properties = Some(properties);
            run
        })
        .collect()
}

/// Group id shared by every run of a Mac `w:tcFitText` paragraph so the layout
/// fits them as one group whatever their own formatting.
const MAC_CELL_FIT_TEXT_GROUP_ID: i32 = i32::MIN;

fn cell_contains_only_hidden_end_marker(cell: &TableCell) -> bool {
    let hide_mark = cell
        .style
        .get("hide-mark")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if !hide_mark || cell.blocks.len() != 1 {
        return false;
    }

    let BodyElement::Paragraph(paragraph) = &cell.blocks[0] else {
        return false;
    };

    !paragraph_has_cell_marker_content(paragraph)
}

/// A nested table's mandatory terminal paragraph carries the cell marker,
/// rather than an additional text line. Explicit breaks and content keep it
/// on the ordinary paragraph path; its model index is always retained.
fn terminal_cell_marker_after_nested_table(cell: &TableCell) -> Option<usize> {
    let [.., BodyElement::Table(_), BodyElement::Paragraph(paragraph)] = cell.blocks.as_slice()
    else {
        return None;
    };
    // Zero-flow caret registration uses the horizontal cell path. Rotated or
    // stacked cells keep their normal line until that path supports the marker.
    if cell.text_direction.is_some_and(|direction| {
        direction_requires_rotated_layout(direction) || direction_requires_vertical_stack(direction)
    }) {
        return None;
    }
    (!paragraph_has_cell_marker_content(paragraph)).then_some(cell.blocks.len() - 1)
}

fn paragraph_has_cell_marker_content(paragraph: &Paragraph) -> bool {
    !paragraph.comment_ranges.is_empty()
        || !paragraph.comment_markers.is_empty()
        || paragraph.properties.as_ref().is_some_and(|props| {
            props.numbering.is_some()
                || props.frame.is_some()
                || props.borders.is_some()
                || props.shading.is_some()
        })
        || paragraph.runs.iter().any(|run| {
            !run.text.is_empty()
                || run.image.is_some()
                || run.text_box.is_some()
                || run.math.is_some()
                || run.field.is_some()
                || run.note_separator.is_some()
                || !run.comment_reference_ids.is_empty()
                || !run.footnote_reference_ids.is_empty()
                || !run.endnote_reference_ids.is_empty()
                || run.has_footnote_ref
                || run.has_endnote_ref
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LineStyleSpan {
    start: usize,
    end: usize,
    run_index: Option<usize>,
}

fn line_style_spans(
    line_start: usize,
    line_end: usize,
    styled_runs: &[StyledRunRange],
) -> Vec<LineStyleSpan> {
    if line_end <= line_start {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut cursor = line_start;

    for (index, run) in styled_runs.iter().enumerate() {
        if run.end <= cursor || run.start >= line_end {
            continue;
        }

        let overlap_start = run.start.max(line_start);
        let overlap_end = run.end.min(line_end);
        if overlap_end <= overlap_start {
            continue;
        }

        if cursor < overlap_start {
            spans.push(LineStyleSpan {
                start: cursor,
                end: overlap_start,
                run_index: None,
            });
        }

        spans.push(LineStyleSpan {
            start: overlap_start,
            end: overlap_end,
            run_index: Some(index),
        });
        cursor = overlap_end;
    }

    if cursor < line_end {
        spans.push(LineStyleSpan {
            start: cursor,
            end: line_end,
            run_index: None,
        });
    }

    if spans.is_empty() {
        spans.push(LineStyleSpan {
            start: line_start,
            end: line_end,
            run_index: None,
        });
    }

    spans
}

fn table_font_descriptor_for_text(
    run_props: Option<&RunProperties>,
    text: &str,
) -> Option<Rc<str>> {
    let properties_ref = run_props?;
    let resolved = resolve_run_style_for_wrap(Some(properties_ref), None);
    if !text.is_ascii() {
        Some(Rc::clone(&resolved.font_descriptor_east_asia))
    } else {
        Some(Rc::clone(&resolved.font_descriptor_latin_hansi))
    }
}

fn paragraph_primary_font_descriptor(paragraph: &CellParagraph) -> Option<Rc<str>> {
    let run = paragraph
        .styled_runs
        .iter()
        .find(|run| run.end > run.start && run.properties.is_some())?;
    let text = paragraph.text.get(run.start..run.end).unwrap_or("");
    table_font_descriptor_for_text(run.properties.as_ref(), text)
}

fn run_text_scale(properties: Option<&RunProperties>) -> f64 {
    properties
        .and_then(|properties| properties.text_scale)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .map(|scale| (scale as f64 / 100.0).clamp(0.01, 10.0))
        .unwrap_or(1.0)
}

fn paragraph_primary_text_scale(paragraph: &CellParagraph) -> f64 {
    paragraph
        .styled_runs
        .iter()
        .find(|run| run.end > run.start)
        .map(|run| run_text_scale(run.properties.as_ref()))
        .unwrap_or(1.0)
}

fn paragraph_line_text_scale(paragraph: &CellParagraph, line: &LineLayout) -> f64 {
    paragraph
        .styled_runs
        .iter()
        .find(|run| run.start <= line.start && line.end <= run.end)
        .map(|run| run_text_scale(run.properties.as_ref()))
        .unwrap_or(1.0)
}

/// Whether a vertical line is a 縦中横 group (`w:eastAsianLayout w:vert`),
/// with whether it is fitted to the line (`w:vertCompress`) and the font of
/// the run it starts in.
fn paragraph_line_tate_chu_yoko(
    paragraph: &CellParagraph,
    line: &LineLayout,
) -> Option<(bool, Option<Rc<str>>)> {
    let run = paragraph
        .styled_runs
        .iter()
        .find(|run| run.start <= line.start && line.start < run.end)?;
    let layout = run.properties.as_ref()?.east_asian_layout.as_deref()?;
    layout.vert.then(|| {
        (
            layout.vert_compress,
            table_font_descriptor_for_text(run.properties.as_ref(), &line.text),
        )
    })
}

fn paragraph_line_font_descriptor(paragraph: &CellParagraph, line: &LineLayout) -> Option<Rc<str>> {
    let run = paragraph
        .styled_runs
        .iter()
        .find(|run| run.properties.is_some() && run.start <= line.start && line.end <= run.end)?;
    table_font_descriptor_for_text(run.properties.as_ref(), &line.text)
}

fn paragraph_range_font_descriptor(
    paragraph: &CellParagraph,
    start: usize,
    end: usize,
    text: &str,
) -> Option<Rc<str>> {
    let run = paragraph
        .styled_runs
        .iter()
        .find(|run| run.properties.is_some() && run.start <= start && end <= run.end)?;
    table_font_descriptor_for_text(run.properties.as_ref(), text)
}

fn line_range_width(line: &LineLayout, start: usize, end: usize) -> f64 {
    if end <= start || line.glyphs.is_empty() {
        return 0.0;
    }

    line.glyphs
        .iter()
        .filter_map(|glyph| {
            if glyph.end <= start || glyph.start >= end {
                return None;
            }

            let overlap_start = start.max(glyph.start);
            let overlap_end = end.min(glyph.end);
            if overlap_end <= overlap_start {
                return None;
            }

            let glyph_span = glyph.end.saturating_sub(glyph.start);
            if glyph_span == 0 {
                return Some(0.0);
            }

            let overlap = overlap_end.saturating_sub(overlap_start);
            let ratio = overlap as f64 / glyph_span as f64;
            Some(glyph.width * ratio)
        })
        .sum()
}

fn wrapped_line_range(line: &WrappedLine) -> (usize, usize) {
    if let (Some(first), Some(last)) = (line.clusters.first(), line.clusters.last()) {
        return (first.start, last.end);
    }
    if let (Some(first), Some(last)) = (line.segments.first(), line.segments.last()) {
        return (first.start, last.end);
    }
    line.break_range.unwrap_or((0, 0))
}

fn build_line_layout_from_wrapped_line(line: &WrappedLine) -> LineLayout {
    let (start, end) = wrapped_line_range(line);
    let mut text = String::new();
    let mut glyphs = Vec::with_capacity(line.clusters.len());
    let mut cursor = 0.0;

    for segment in &line.segments {
        text.push_str(&segment.text);
    }

    for cluster in &line.clusters {
        let width = cluster.width.max(0.0);
        // A cluster's advance starts with its auto-spacing/kerning gap; the
        // glyph itself sits after it, as in body paragraphs.
        let leading = cluster
            .layout
            .as_ref()
            .map(|layout| layout.leading_advance)
            .filter(|leading| leading.is_finite())
            .unwrap_or(0.0)
            // Negative kerning moves the glyph before the advance's start.
            // Keep it, or the cursor stays correct but this glyph is shifted.
            .min(width);
        glyphs.push(GlyphBox {
            start: cluster.start,
            end: cluster.end,
            x: cursor + leading,
            width: width - leading,
        });
        cursor += width;
    }

    // An inline picture stands on the baseline: the line's baseline sits
    // the picture's height (or the text's ascent, when taller) below its top
    // and the text's descent hangs below. Word for Mac draws a cell line of
    // only an 89.1pt picture exactly 89.1pt tall (`table-object-edge-cases`).
    let object_height = line
        .clusters
        .iter()
        .filter_map(|cluster| cluster.inline_object)
        .map(|object| object.height)
        .fold(0.0_f64, f64::max);
    let (word_baseline_offset, min_advance) = if object_height > 0.0 {
        let (text_ascent, text_descent) = line
            .clusters
            .iter()
            .filter(|cluster| cluster.inline_object.is_none() && !cluster.is_whitespace)
            .filter_map(|cluster| cluster.layout.as_ref())
            .fold((0.0_f64, 0.0_f64), |(ascent, descent), layout| {
                (
                    ascent.max(layout.font_ascent.unwrap_or(0.0)),
                    descent.max(layout.font_descent.unwrap_or(0.0)),
                )
            });
        let baseline = object_height.max(text_ascent);
        (Some(baseline), baseline + text_descent)
    } else {
        (None, 0.0)
    };

    LineLayout {
        left_inset: 0.0,
        right_inset: 0.0,
        text,
        visual_width: cursor,
        start,
        end,
        glyphs,
        break_kind: line.break_kind,
        break_range: line.break_range,
        break_properties: line.break_properties.clone(),
        word_baseline_offset,
        min_advance,
    }
}

fn table_formatting_mark_symbol(
    show_formatting_marks: bool,
    break_kind: LineBreakKind,
) -> Option<&'static str> {
    if !show_formatting_marks || matches!(break_kind, LineBreakKind::Soft) {
        return None;
    }
    formatting_mark_symbol(break_kind)
}

fn plain_text_run_styles(text: &str) -> Vec<RunStyle> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }]
    }
}

fn styled_run_styles(text: &str, styled_runs: &[StyledRunRange]) -> Vec<RunStyle> {
    let mut run_styles = styled_runs
        .iter()
        .filter_map(|run| {
            let start = run.start.min(text.len());
            let end = run.end.min(text.len());
            if end <= start {
                return None;
            }
            Some(RunStyle {
                start,
                end,
                properties: run.properties.clone().map(Rc::new),
                revision: run.revision.clone(),
                hyperlink: None,
                inline_object: run.picture.as_ref().map(CellPicture::inline_object),
            })
        })
        .collect::<Vec<_>>();

    if run_styles.is_empty() {
        run_styles = plain_text_run_styles(text);
    }
    run_styles
}

pub(super) fn layout_paragraph<P: Painter<Error: PainterError>>(
    context: &P,
    text: &str,
    max_width: f64,
    line_height: f64,
    direction: Option<TextDirection>,
    available_height: Option<f64>,
    styled_runs: &[StyledRunRange],
) -> Result<CellParagraph, JsValue> {
    let wrap_context = TableLineWrapContext::new(context);
    layout_paragraph_with_table_context(
        &wrap_context,
        text,
        max_width,
        line_height,
        direction,
        available_height,
        styled_runs,
        &ParagraphStyle::default(),
        0.0,
    )
}

#[allow(clippy::too_many_arguments)]
fn layout_paragraph_with_table_context<P: Painter<Error: PainterError>>(
    context: &TableLineWrapContext<'_, P>,
    text: &str,
    max_width: f64,
    line_height: f64,
    direction: Option<TextDirection>,
    available_height: Option<f64>,
    styled_runs: &[StyledRunRange],
    paragraph_style: &ParagraphStyle,
    writing_axis_start: f64,
) -> Result<CellParagraph, JsValue> {
    let run_styles = styled_run_styles(text, styled_runs);
    if let Some(dir) = direction {
        if matches!(
            dir,
            TextDirection::TbRlV | TextDirection::TbRl | TextDirection::BtLr
        ) {
            let result = if dir == TextDirection::TbRlV {
                jade_docx_layout::line_wrap::layout_cell_text_vertically(
                    context,
                    text,
                    &run_styles,
                    0,
                    available_height,
                    line_height,
                    writing_axis_start,
                    paragraph_style,
                )?
            } else {
                let start = if writing_axis_start.is_finite() {
                    writing_axis_start
                } else {
                    0.0
                };
                let capacity = (available_height.unwrap_or(1_000_000_000.0) - start).max(1.0);
                let columns =
                    context.wrap_horizontal(text, &run_styles, paragraph_style, 0, capacity)?;
                jade_docx_layout::line_wrap::project_cell_text_turned(
                    context,
                    columns.to_vec(),
                    available_height,
                    line_height,
                    start,
                    paragraph_style,
                )?
            };
            let mut lines = result
                .layout
                .lines
                .iter()
                .map(|wrapped| {
                    let mut line = build_line_layout_from_wrapped_line(wrapped);
                    if dir == TextDirection::TbRlV {
                        line.word_baseline_offset =
                            jade_docx_layout::line_wrap::wrapped_line_max_font_ascent(wrapped);
                        for (glyph, cluster) in line.glyphs.iter_mut().zip(&wrapped.clusters) {
                            if let Some(metrics) = &cluster.layout {
                                glyph.width = metrics.glyph_box_width;
                            }
                        }
                    } else {
                        line.word_baseline_offset =
                            jade_docx_layout::line_wrap::wrapped_line_turned_baseline_offset(
                                wrapped,
                                line_height,
                                dir == TextDirection::TbRl,
                            );
                    }
                    line
                })
                .collect::<Vec<_>>();
            for (range, offset) in result
                .layout
                .column_ranges
                .iter()
                .zip(result.column_start_offsets)
            {
                lines[range.start].left_inset = offset;
            }
            return Ok(CellParagraph {
                text: text.to_string(),
                lines: Rc::new(lines),
                max_line_width: result.layout.max_line_width,
                height: result.layout.column_height,
                line_height,
                vertical_layout: Some(VerticalTextLayout {
                    column_ranges: result.layout.column_ranges,
                    column_height: result.layout.column_height,
                }),
                ..CellParagraph::default()
            });
        }
        if direction_requires_rotated_layout(dir) {
            return layout_rotated_paragraph(context.context, text, max_width, line_height);
        }
        if direction_requires_vertical_stack(dir) {
            let capacity_height = available_height.map(|height| height + line_height);
            let mut paragraph = layout_vertical_paragraph_with_run_styles_context(
                context,
                text,
                dir,
                capacity_height,
                line_height,
                &run_styles,
                false,
            )?;
            if let Some(height) = available_height {
                let visible_height = height.max(line_height);
                paragraph.height = visible_height;
                if let Some(layout) = paragraph.vertical_layout.as_mut() {
                    layout.column_height = visible_height;
                }
            }
            return Ok(paragraph);
        }
    }

    layout_cached_horizontal_table_paragraph(
        context,
        text,
        max_width,
        line_height,
        &run_styles,
        paragraph_style,
    )
}

pub(super) fn layout_vertical_paragraph_with_wrap_context<C: LineWrapContext>(
    context: &C,
    text: &str,
    run_styles: &[RunStyle],
    direction: TextDirection,
    available_height: Option<f64>,
    line_height: f64,
) -> Result<CellParagraph, C::Error> {
    let mut paragraph = layout_vertical_paragraph_with_run_styles_context(
        context,
        text,
        direction,
        available_height,
        line_height,
        run_styles,
        true,
    )?;
    // Section text uses this table-like layout adapter too. Keep its resolved
    // run properties for painting, instead of falling back to TABLE_FONT.
    paragraph.styled_runs = run_styles
        .iter()
        .map(|run| StyledRunRange {
            picture: None,
            start: run.start,
            end: run.end,
            properties: run.properties.as_deref().cloned(),
            revision: run.revision.clone(),
        })
        .collect();
    Ok(paragraph)
}

#[cfg(test)]
fn layout_vertical_paragraph_with_context<C: LineWrapContext>(
    context: &C,
    text: &str,
    direction: TextDirection,
    available_height: Option<f64>,
    line_height: f64,
) -> Result<CellParagraph, C::Error> {
    let run_styles = plain_text_run_styles(text);
    layout_vertical_paragraph_with_run_styles_context(
        context,
        text,
        direction,
        available_height,
        line_height,
        &run_styles,
        false,
    )
}

fn layout_vertical_paragraph_with_run_styles_context<C: LineWrapContext>(
    context: &C,
    text: &str,
    direction: TextDirection,
    available_height: Option<f64>,
    line_height: f64,
    run_styles: &[RunStyle],
    upright_section: bool,
) -> Result<CellParagraph, C::Error> {
    let layout = if upright_section {
        jade_docx_layout::line_wrap::layout_section_text_vertically
    } else {
        jade_docx_layout::line_wrap::layout_text_vertically
    };
    let wrapped = layout(
        context,
        text,
        run_styles,
        direction_prefers_horizontal_latin(direction),
        0,
        available_height,
        line_height,
    )?;
    let lines = wrapped
        .lines
        .iter()
        .map(|wrapped_line| {
            let mut line = build_line_layout_from_wrapped_line(wrapped_line);
            if upright_section {
                line.word_baseline_offset =
                    jade_docx_layout::line_wrap::wrapped_line_max_font_ascent(wrapped_line);
            }
            line
        })
        .collect::<Vec<_>>();
    let vertical_layout = VerticalTextLayout {
        column_ranges: wrapped.column_ranges,
        column_height: wrapped.column_height,
    };

    Ok(CellParagraph {
        text: text.to_string(),
        is_continuation: false,
        spacing_before: 0.0,
        spacing_after: 0.0,
        align: None,
        lines: Rc::new(lines),
        styled_runs: Vec::new(),
        max_line_width: wrapped.max_line_width,
        height: wrapped.column_height,
        line_height,
        vertical_layout: Some(vertical_layout),
        ..CellParagraph::default()
    })
}

fn layout_horizontal_paragraph<P: Painter<Error: PainterError>>(
    context: &P,
    text: &str,
    max_width: f64,
    line_height: f64,
) -> Result<CellParagraph, JsValue> {
    let wrap_context = TableLineWrapContext::new(context);
    layout_horizontal_paragraph_with_context(&wrap_context, text, max_width, line_height)
}

fn layout_horizontal_paragraph_with_context<C: LineWrapContext>(
    context: &C,
    text: &str,
    max_width: f64,
    line_height: f64,
) -> Result<CellParagraph, C::Error> {
    let run_styles = plain_text_run_styles(text);
    layout_horizontal_paragraph_with_run_styles_context(
        context,
        text,
        max_width,
        line_height,
        &run_styles,
        &ParagraphStyle::default(),
    )
}

fn layout_horizontal_paragraph_with_run_styles_context<C: LineWrapContext>(
    context: &C,
    text: &str,
    max_width: f64,
    line_height: f64,
    run_styles: &[RunStyle],
    paragraph_style: &ParagraphStyle,
) -> Result<CellParagraph, C::Error> {
    layout_horizontal_paragraph_with_run_styles_context_offset(
        context,
        text,
        max_width,
        line_height,
        run_styles,
        paragraph_style,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn layout_horizontal_paragraph_with_run_styles_context_offset<C: LineWrapContext>(
    context: &C,
    text: &str,
    max_width: f64,
    line_height: f64,
    run_styles: &[RunStyle],
    paragraph_style: &ParagraphStyle,
    base_offset: usize,
) -> Result<CellParagraph, C::Error> {
    let mut wrapped = jade_docx_layout::line_wrap::wrap_text_into_lines(
        context,
        text,
        run_styles,
        0.0,
        paragraph_style,
        base_offset,
        max_width.max(1.0),
    )?;
    justify_table_wrapped_lines(&mut wrapped, max_width, paragraph_style);
    Ok(horizontal_cell_paragraph_from_wrapped_lines(
        text,
        line_height,
        paragraph_style,
        base_offset,
        &wrapped,
    ))
}

fn horizontal_cell_paragraph_from_wrapped_lines(
    text: &str,
    line_height: f64,
    paragraph_style: &ParagraphStyle,
    base_offset: usize,
    wrapped: &[WrappedLine],
) -> CellParagraph {
    let mut lines = wrapped
        .iter()
        .enumerate()
        .map(|(index, wrapped)| {
            let mut line = build_line_layout_from_wrapped_line(wrapped);
            line.left_inset = paragraph_style.line_indent(index);
            line.right_inset = paragraph_style.right_indent;
            if paragraph_style.word_east_asian_font_line_box && line.min_advance <= 0.0 {
                line.word_baseline_offset =
                    jade_docx_layout::line_wrap::wrapped_line_grid_baseline_offset(
                        wrapped,
                        paragraph_style,
                        line_height,
                    );
            }
            line
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        lines.push(LineLayout {
            min_advance: 0.0,
            left_inset: paragraph_style.line_indent(0),
            right_inset: paragraph_style.right_indent,
            text: String::new(),
            visual_width: 0.0,
            start: base_offset,
            end: base_offset,
            glyphs: Vec::new(),
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((base_offset, base_offset)),
            break_properties: None,
            word_baseline_offset: None,
        });
    }

    let mut max_line_width: f64 = 0.0;
    for line in &mut lines {
        if line.glyphs.is_empty() {
            line.glyphs.push(GlyphBox {
                start: line.start,
                end: line.end,
                x: 0.0,
                width: 0.0,
            });
        }
        max_line_width = max_line_width.max(line.visual_width);
    }

    let height = lines
        .iter()
        .map(|line| line_height.max(line.min_advance))
        .sum::<f64>()
        .max(line_height);

    CellParagraph {
        text: text.to_string(),
        is_continuation: false,
        spacing_before: 0.0,
        spacing_after: 0.0,
        align: None,
        lines: Rc::new(lines),
        styled_runs: Vec::new(),
        max_line_width: max_line_width.max(line_height),
        height,
        line_height,
        vertical_layout: None,
        ..CellParagraph::default()
    }
}

fn layout_rotated_paragraph<P: Painter<Error: PainterError>>(
    context: &P,
    text: &str,
    max_width: f64,
    line_height: f64,
) -> Result<CellParagraph, JsValue> {
    let mut paragraph = layout_horizontal_paragraph(context, text, max_width, line_height)?;
    paragraph.height = paragraph.max_line_width.max(line_height);
    Ok(paragraph)
}

fn empty_cell_paragraph(line_height: f64) -> CellParagraph {
    CellParagraph {
        text: String::new(),
        is_continuation: false,
        spacing_before: 0.0,
        spacing_after: 0.0,
        align: None,
        lines: Rc::new(vec![LineLayout {
            min_advance: 0.0,
            left_inset: 0.0,
            right_inset: 0.0,
            text: String::new(),
            visual_width: 0.0,
            start: 0,
            end: 0,
            glyphs: vec![GlyphBox {
                start: 0,
                end: 0,
                x: 0.0,
                width: 0.0,
            }],
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((0, 0)),
            break_properties: None,
            word_baseline_offset: None,
        }]),
        styled_runs: Vec::new(),
        max_line_width: 0.0,
        height: line_height,
        line_height,
        vertical_layout: None,
        ..CellParagraph::default()
    }
}

fn direction_requires_rotated_layout(_direction: TextDirection) -> bool {
    false
}

fn direction_requires_vertical_stack(direction: TextDirection) -> bool {
    matches!(
        direction,
        TextDirection::TbRl | TextDirection::TbRlV | TextDirection::BtLr | TextDirection::BtLrV
    )
}

fn direction_draws_rotated_table_visual(direction: TextDirection, section: bool) -> bool {
    matches!(direction, TextDirection::BtLrV) || section && direction == TextDirection::TbRlV
}

fn direction_column_flow(direction: TextDirection) -> ColumnFlow {
    match direction {
        TextDirection::TbRl | TextDirection::TbRlV => ColumnFlow::RightToLeft,
        _ => ColumnFlow::LeftToRight,
    }
}

fn direction_vertical_axis(direction: TextDirection) -> VerticalAxis {
    match direction {
        TextDirection::BtLr | TextDirection::BtLrV => VerticalAxis::BottomToTop,
        _ => VerticalAxis::TopToBottom,
    }
}

fn direction_prefers_horizontal_latin(direction: TextDirection) -> bool {
    matches!(direction, TextDirection::LrTbV)
}

fn direction_rotation_angle(direction: TextDirection) -> f64 {
    match direction {
        TextDirection::TbRlV => FRAC_PI_2,
        TextDirection::BtLrV => -FRAC_PI_2,
        _ => 0.0,
    }
}

fn rotated_visual_anchor_x(
    direction: TextDirection,
    base_x: f64,
    content_right: f64,
    line_height: f64,
) -> f64 {
    match direction {
        TextDirection::TbRlV => {
            content_right - line_height * VERTICAL_TABLE_ROTATED_RIGHT_INSET_FACTOR
        }
        _ => base_x,
    }
}

fn rotated_visual_baseline_y(
    direction: TextDirection,
    block_top: f64,
    column_height: f64,
    line_height: f64,
) -> f64 {
    match direction {
        TextDirection::TbRlV => block_top + line_height * VERTICAL_TABLE_TEXT_TOP_INSET_FACTOR,
        _ => block_top + column_height - line_height * VERTICAL_TABLE_ROTATED_BASELINE_INSET_FACTOR,
    }
}

fn default_align_for_direction(direction: Option<TextDirection>) -> TextAlign {
    match direction {
        Some(TextDirection::TbRl) | Some(TextDirection::TbRlV) => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

fn glyph_has_east_asian(text: &str) -> bool {
    classify_script(text) == ScriptCategory::EastAsia
}

fn turned_em_origin(x: f64, top: f64, pitch: f64, advance: f64, ascent: f64) -> (f64, f64) {
    (x + ascent, top + (pitch + advance) * 0.5)
}

/// Parses a CSS-like length string and converts it into device pixels. Supports `pt`, `px`,
/// absolute numbers (assumed to be pixels), and returns `None` for unknown formats.
fn parse_length_to_px(value: &str) -> Option<f64> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.ends_with("pt") {
        let number = trimmed.trim_end_matches("pt").trim();
        if let Ok(v) = number.parse::<f32>() {
            return Some(points_to_px(v));
        }
    } else if trimmed.ends_with("px") {
        let number = trimmed.trim_end_matches("px").trim();
        if let Ok(v) = number.parse::<f64>() {
            return Some(v);
        }
    } else if let Ok(v) = trimmed.parse::<f64>() {
        return Some(v);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn vertical_geometry_keeps_the_cached_paragraph_memory_budget() {
        // Frozen column-layout parent: native 168 bytes, wasm32 112 bytes.
        let budget = if cfg!(target_pointer_width = "32") {
            112
        } else {
            168
        };
        assert!(std::mem::size_of::<super::CellParagraph>() <= budget);
    }

    use super::super::points_to_px;
    use super::*;
    use jade_painter::recording::{Op, RecordingPainter};

    use crate::document::{
        Paragraph, Run, RunProperties, Table, TableCell, TableColumn, TableLayoutMode, TableRow,
    };
    use jade_docx_layout::font_resolver::ResolvedRunStyle;
    use proptest::prelude::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn paragraph_with_field(kind: RunFieldKind, saved: &str) -> Paragraph {
        let mut run: Run = serde_json::from_value(serde_json::json!({ "text": "" })).unwrap();
        run.field = Some(RunField {
            word_mac_body_result: None,
            kind,
            source_field_id: None,
            cached_text: Some(saved.into()),
            resolved_value: None,
        });
        run.properties = Some(RunProperties {
            bold: Some(true),
            font_size: Some(14.0),
            ..Default::default()
        });
        Paragraph {
            runs: vec![run],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    #[test]
    fn table_field_text_uses_renderer_page_section_lock_and_editable_contracts() {
        let mut renderer = super::super::tests::test_renderer();
        renderer.current_page_number = 2;
        renderer.active_section_index = 1;
        for (kind, saved, expected) in [
            (RunFieldKind::PageNumber, "stale", "2"),
            (RunFieldKind::SectionNumber, "stale", "2"),
            (RunFieldKind::NumPages.with_lock(true), "4", "4"),
            (RunFieldKind::NumPages.with_lock(true), "", ""),
            (
                RunFieldKind::NonDisplay {
                    instruction: "PRIVATE".into(),
                },
                "hidden",
                "",
            ),
        ] {
            let paragraph = paragraph_with_field(kind, saved);
            let (text, ranges, _) = collect_paragraph_text(
                &paragraph,
                true,
                false,
                &NoteMarkers::default(),
                &|field| renderer.resolve_run_field(field),
            );
            assert_eq!(text, expected);
            if !expected.is_empty() {
                assert_eq!(ranges.len(), 1);
                assert_eq!((ranges[0].start, ranges[0].end), (0, expected.len()));
                assert_eq!(ranges[0].properties, paragraph.runs[0].properties);
            } else {
                assert!(ranges.is_empty());
            }
        }
        let mut paragraph = paragraph_with_field(
            RunFieldKind::MergeField {
                name: "Client".into(),
            },
            "cached",
        );
        paragraph.runs[0].text = "editable".into();
        let (text, _, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|field| {
                renderer.resolve_run_field(field)
            });
        assert_eq!(text, "editable");
        // An authored field keeps its result in the run as well as in the
        // field; a cell draws it once, as a body paragraph does.
        let mut paragraph = paragraph_with_field(
            RunFieldKind::Unsupported {
                instruction: "=SUM(ABOVE)".into(),
            },
            "8",
        );
        paragraph.runs[0].text = "8".into();
        let (text, _, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|field| {
                renderer.resolve_run_field(field)
            });
        assert_eq!(text, "8");
    }

    #[test]
    fn table_autofit_cache_tracks_live_field_display_values() {
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph_with_field(
                        RunFieldKind::PageNumber,
                        "stale",
                    ))],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = FixedWidthWrapContext::new();
        let cache = RefCell::new(TableAutofitCache::default());
        let mut renderer = super::super::tests::test_renderer();
        for (page, expected, hits, misses) in [(1, "1", 0, 1), (1, "1", 1, 1), (12, "12", 1, 2)] {
            renderer.current_page_number = page;
            let (_, layouts) = apply_table_layout_width_for_table_with_content(
                &context,
                &table,
                &mut [240.0],
                240.0,
                0.0,
                &|_| 16.0,
                Some(&cache),
                &|field| renderer.resolve_run_field(field),
            )
            .unwrap();
            assert_eq!(layouts[0].layout.text, expected);
            assert_eq!(cache.borrow().stats.hits, hits);
            assert_eq!(cache.borrow().stats.misses, misses);
        }
        let BodyElement::Paragraph(paragraph) = &table.rows[0].cells[0].blocks[0] else {
            panic!("original paragraph");
        };
        assert_eq!(
            paragraph.runs[0]
                .field
                .as_ref()
                .unwrap()
                .cached_text
                .as_deref(),
            Some("stale")
        );
    }

    #[test]
    fn ordinary_table_text_does_not_call_the_field_resolver() {
        let mut paragraph = paragraph_with_field(RunFieldKind::PageNumber, "stale");
        paragraph.runs[0].field = None;
        paragraph.runs[0].text = "Ordinary cell".into();
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph)],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (_, layouts) = apply_table_layout_width_for_table_with_content(
            &FixedWidthWrapContext::new(),
            &table,
            &mut [240.0],
            240.0,
            0.0,
            &|_| 16.0,
            Some(&RefCell::new(TableAutofitCache::default())),
            &|_| panic!("ordinary cell must retain the plain text path"),
        )
        .unwrap();
        assert_eq!(layouts[0].layout.text, "Ordinary cell");
    }

    #[test]
    fn turned_cells_reuse_shared_table_wrap_cache_without_changing_geometry() {
        let painter = RecordingPainter::with_width_per_char(10.0);
        let cache = Rc::new(RefCell::new(ParagraphWrapCache::new(16)));
        let mut context = TableLineWrapContext::new(&painter);
        context.wrap_cache = Some(Rc::clone(&cache));
        let style = ParagraphStyle {
            first_line_indent: 5.0,
            right_indent: 3.0,
            ..Default::default()
        };
        let layout = |height| {
            layout_paragraph_with_table_context(
                &context,
                "ABあいCD",
                100.0,
                24.0,
                Some(TextDirection::TbRl),
                Some(height),
                &[],
                &style,
                7.0,
            )
            .unwrap()
        };
        let first = layout(60.0);
        let second = layout(60.0);
        let stats = cache.borrow().stats();
        assert_eq!(stats.table_misses, 1);
        assert_eq!(stats.table_hits, 1);
        let signature = |paragraph: &CellParagraph| {
            paragraph
                .lines
                .iter()
                .map(|line| {
                    (
                        line.text.clone(),
                        line.start,
                        line.end,
                        line.visual_width,
                        line.left_inset,
                        line.word_baseline_offset,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&first), signature(&second));
        let _ = layout(60.001);
        assert_eq!(cache.borrow().stats().table_misses, 2);
    }

    #[test]
    fn table_wrap_cache_reuses_final_lines_and_keys_exact_width_offsets_and_properties() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let mut context = TableLineWrapContext::new(&painter);
        let cache = Rc::new(RefCell::new(ParagraphWrapCache::new(16)));
        context.wrap_cache = Some(Rc::clone(&cache));
        let text = "One two three four five";
        let runs = plain_text_run_styles(text);
        let style = ParagraphStyle {
            alignment: "both".into(),
            ..Default::default()
        };
        let first = context
            .wrap_horizontal(text, &runs, &style, 0, 60.0)
            .unwrap();
        let repeated = context
            .wrap_horizontal(text, &runs, &style, 0, 60.0)
            .unwrap();
        assert!(
            Rc::ptr_eq(&first, &repeated),
            "justification is not repeated on a hit"
        );
        assert_eq!(cache.borrow().misses, 1);
        assert_eq!(cache.borrow().hits, 1);
        let uncached = TableLineWrapContext::new(&painter)
            .wrap_horizontal(text, &runs, &style, 0, 60.0)
            .unwrap();
        let geometry = |lines: &[WrappedLine]| {
            horizontal_cell_paragraph_from_wrapped_lines(text, 16.0, &style, 0, lines)
                .lines
                .iter()
                .map(|line| {
                    (
                        line.text.clone(),
                        line.start,
                        line.end,
                        line.visual_width,
                        line.left_inset,
                        line.right_inset,
                        line.word_baseline_offset,
                        line.glyphs
                            .iter()
                            .map(|glyph| (glyph.start, glyph.end, glyph.x, glyph.width))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(geometry(&first), geometry(&uncached));
        let narrow = context
            .wrap_horizontal(text, &runs, &style, 0, 60.001)
            .unwrap();
        assert!(
            !Rc::ptr_eq(&first, &narrow),
            "sub-centipixel widths have distinct keys"
        );
        let shifted = context
            .wrap_horizontal(text, &runs, &style, 9, 60.0)
            .unwrap();
        assert_eq!(shifted[0].segments[0].start, 9);
        let mut changed = runs.clone();
        changed[0].properties = Some(Rc::new(RunProperties {
            color: Some("123456".into()),
            ..Default::default()
        }));
        let recolored = context
            .wrap_horizontal(text, &changed, &style, 0, 60.0)
            .unwrap();
        assert_eq!(
            recolored[0].segments[0]
                .properties
                .as_ref()
                .unwrap()
                .color
                .as_deref(),
            Some("123456")
        );
        context
            .wrap_horizontal(
                "10 reference",
                &plain_text_run_styles("10 reference"),
                &style,
                0,
                60.0,
            )
            .unwrap();
        assert_eq!(cache.borrow().misses, 5);
        let body_key = ParagraphWrapCacheKey::new(1, 0.0, 60.0, 0.0, 0.0, 1);
        let mut table_key = body_key.clone();
        table_key.table_context = true;
        assert_ne!(body_key, table_key);
    }

    #[test]
    fn table_wrap_cache_is_bounded_and_metric_clear_invalidates_retained_lines() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let mut context = TableLineWrapContext::new(&painter);
        let cache = Rc::new(RefCell::new(ParagraphWrapCache::new(2)));
        context.wrap_cache = Some(Rc::clone(&cache));
        for text in ["one", "two", "three"] {
            context
                .wrap_horizontal(
                    text,
                    &plain_text_run_styles(text),
                    &ParagraphStyle::default(),
                    0,
                    80.0,
                )
                .unwrap();
        }
        assert_eq!(cache.borrow().len, 2);
        let mut renderer = super::super::recording_host::recording_renderer();
        renderer.paragraph_wrap_cache = Rc::clone(&cache);
        let misses_before = cache.borrow().misses;
        renderer.clear_text_measure_cache();
        assert_eq!(cache.borrow().len, 0);
        context
            .wrap_horizontal(
                "three",
                &plain_text_run_styles("three"),
                &ParagraphStyle::default(),
                0,
                80.0,
            )
            .unwrap();
        assert_eq!(cache.borrow().misses, misses_before + 1);
    }

    #[test]
    fn repeated_table_ascii_measurements_lookup_shared_cache_once_per_font_and_text() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let context = TableLineWrapContext::new(&painter);
        for descriptor in [TABLE_FONT, "14pt Arial"] {
            let font_id = context
                .cache
                .borrow()
                .interner()
                .borrow_mut()
                .intern(descriptor);
            for _ in 0..1000 {
                for text in ["A", "AB"] {
                    let width = context
                        .measure_text_with_font_key(text, descriptor, 0, Some(font_id))
                        .expect("ASCII measurement");
                    assert_eq!(width, painter.measure_text_width(text).unwrap());
                }
            }
        }
        let stats = context.cache.borrow().stats();
        assert_eq!(stats.misses, 4, "font identity remains part of each lookup");
        assert_eq!(
            stats.hits, 0,
            "repeated per-pass measurements avoid shared LRU work"
        );
    }

    #[test]
    fn table_ascii_memo_does_not_outlive_font_measurement_invalidation() {
        let cache = Rc::new(RefCell::new(TextMeasureCache::default()));
        let overhang = Rc::new(RefCell::new(FxHashMap::default()));
        for width in [7.0, 11.0] {
            cache.borrow_mut().clear();
            let painter = RecordingPainter::with_width_per_char(width);
            let context = TableLineWrapContext::with_caches(
                &painter,
                Rc::clone(&cache),
                Rc::clone(&overhang),
            );
            let id = context.style.font_descriptor_latin_ascii_id;
            for _ in 0..2 {
                assert_eq!(
                    context
                        .measure_text_with_font_key("AB", TABLE_FONT, 0, id)
                        .unwrap(),
                    width * 2.0,
                );
            }
            assert_eq!(painter.count(|op| matches!(op, Op::MeasureText(_))), 1);
        }
    }

    struct FixedWidthWrapContext {
        style: Rc<ResolvedRunStyle>,
        measure_calls: Cell<u64>,
    }

    impl FixedWidthWrapContext {
        fn new() -> Self {
            Self {
                style: Rc::new(ResolvedRunStyle::new(
                    TABLE_FONT.to_string(),
                    TABLE_FONT.to_string(),
                    TABLE_FONT.to_string(),
                    TABLE_FONT.to_string(),
                    0.0,
                    1.0,
                    false,
                    None,
                )),
                measure_calls: Cell::new(0),
            }
        }

        fn measure_calls(&self) -> u64 {
            self.measure_calls.get()
        }
    }

    impl LineWrapContext for FixedWidthWrapContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            let mut style = self.style.as_ref().clone();
            if let Some(scale) = properties.and_then(|props| props.text_scale) {
                style.text_scale_x = scale as f64 / 100.0;
            }
            Ok(Rc::new(style))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            self.measure_calls.set(self.measure_calls.get() + 1);
            let width = text
                .graphemes(true)
                .map(|grapheme| {
                    if grapheme.trim().is_empty() {
                        5.0
                    } else {
                        10.0
                    }
                })
                .sum();
            Ok(width)
        }
    }

    #[test]
    fn table_glyph_origins_keep_negative_shared_pair_spacing() {
        let descriptor = "10.50pt 'Yu Mincho'";
        let context = FixedWidthWrapContext {
            style: Rc::new(ResolvedRunStyle::new(
                descriptor, descriptor, descriptor, descriptor, 0.0, 1.0, true, None,
            )),
            measure_calls: Cell::new(0),
        };
        let style = ParagraphStyle {
            word_platform: jade_docx_layout::line_metrics::WordLayoutPlatform::Mac,
            ..ParagraphStyle::default()
        };
        let paragraph = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            "Wo",
            100.0,
            20.0,
            &plain_text_run_styles("Wo"),
            &style,
        )
        .unwrap();
        let line = &paragraph.lines[0];
        // The same observed Word pair used by body layout: the second glyph
        // starts after W's 2013-unit advance minus 123 units of kerning.
        assert!((line.glyphs[1].x - 14.0 * (2013.0 - 123.0) / 2048.0).abs() < 1e-6);
        assert!((line.glyphs[1].width - 14.0 * 1096.0 / 2048.0).abs() < 1e-6);
        assert!((line.visual_width - 14.0 * (2013.0 - 123.0 + 1096.0) / 2048.0).abs() < 1e-6);
    }

    #[test]
    fn renumbered_table_notes_keep_source_metrics_surface_indices_and_borders() {
        for footnote in [true, false] {
            let paragraph = |references_notes: bool| Paragraph {
                runs: vec![Run {
                    text: "年1月".into(),
                    properties: Some(RunProperties {
                        font_family: Some("Refresh Test Face".into()),
                        font_size: Some(12.0),
                        ..RunProperties::default()
                    }),
                    image: None,
                    text_box: None,
                    math: None,
                    field: None,
                    hyperlink: None,
                    node_id: None,
                    text_node_id: None,
                    revision_node_id: None,
                    comment_reference_ids: Vec::new(),
                    footnote_reference_ids: if references_notes && footnote {
                        vec!["1".into()]
                    } else {
                        Vec::new()
                    },
                    endnote_reference_ids: if references_notes && !footnote {
                        vec!["1".into()]
                    } else {
                        Vec::new()
                    },
                    note_separator: None,
                    has_footnote_ref: false,
                    has_endnote_ref: false,
                    revision: None,
                }],
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            };
            let table = Table {
                rows: vec![TableRow {
                    cells: vec![
                        TableCell {
                            blocks: vec![BodyElement::from_paragraph(paragraph(true))],
                            ..Default::default()
                        },
                        TableCell {
                            blocks: vec![BodyElement::from_paragraph(paragraph(false))],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let row = &table.rows[0];
            let mut heights = FxHashMap::default();
            let mut rules = FxHashMap::default();
            for (index, cell) in row.cells.iter().enumerate() {
                let BodyElement::Paragraph(paragraph) = &cell.blocks[0] else {
                    panic!("paragraph cell");
                };
                let key = paragraph.as_ref() as *const Paragraph as usize;
                heights.insert(key, 32.0 + index as f64);
                rules.insert(
                    key,
                    EastAsianLineBreakingRules {
                        use_east_asian_layout: true,
                        auto_space_dn: Some(true),
                        ..EastAsianLineBreakingRules::default()
                    },
                );
            }
            let mut config = LayoutConfig {
                padding_x: 0.0,
                padding_y: 0.0,
                line_height: 12.0,
                paragraph_line_heights: heights,
                paragraph_east_asian_rules: rules,
                available_width: 400.0,
                render_changes: false,
                render_original: false,
                note_markers: NoteMarkers::default(),
                doc_grid: None,
                document_snap_to_grid_default: false,
                document_uses_east_asian_layout: false,
                word_modern_border_insets: false,
                styles_part: None,
            };
            let marks = if footnote {
                &mut config.note_markers.footnotes
            } else {
                &mut config.note_markers.endnotes
            };
            marks.insert("1".into(), "10".into());
            let painter = RecordingPainter::with_width_per_char(7.0);
            let context = TableLineWrapContext::new(&painter);
            let mut cells = prepare_row_layout(
                &context,
                row,
                &table,
                &[200.0, 200.0],
                &config,
                None,
                0,
                1,
                &mut NaturalParagraphLayoutCache::default(),
                None,
                &|_| None,
            )
            .unwrap();
            for cell in &mut cells {
                cell.border_top = CellBorderSpec::single_default();
                cell.border_bottom = CellBorderSpec::single_default();
                cell.border_top.width_px = points_to_px(0.5);
                cell.border_bottom.width_px = points_to_px(0.5);
            }
            let mut prepared = vec![cells];
            let mut row_heights = vec![33.0];
            add_border_extents(&table, &mut prepared, &[false], &mut row_heights, true);
            let mut cells = prepared.remove(0);
            assign_row_paragraph_indices(&mut cells, 70, &HashMap::new());
            let original = cells.clone();
            let marks = if footnote {
                &mut config.note_markers.footnotes
            } else {
                &mut config.note_markers.endnotes
            };
            marks.insert("1".into(), "1".into());
            let refreshed_height = refresh_prepared_row_text(
                &context,
                row,
                &table,
                &[200.0, 200.0],
                &config,
                None,
                0,
                1,
                &mut cells,
                &|_| None,
            )
            .unwrap();
            assert!(
                (refreshed_height - row_heights[0]).abs() < 1e-6,
                "renumbering preserves top/bottom border reservation: {refreshed_height} vs {}",
                row_heights[0]
            );
            for (index, cell) in cells.iter().enumerate() {
                assert_eq!(
                    cell.blocks[0].paragraph_indices,
                    original[index].blocks[0].paragraph_indices
                );
                let CellBlockKind::Paragraph(paragraph) = &cell.blocks[0].kind else {
                    panic!("prepared paragraph");
                };
                let CellBlockKind::Paragraph(previous) = &original[index].blocks[0].kind else {
                    panic!("original paragraph");
                };
                assert_eq!(
                    paragraph.line_height, previous.line_height,
                    "cell {index}, footnote {footnote}"
                );
                assert_eq!(paragraph.text, if index == 0 { "年1月1" } else { "年1月" });
                for (glyph, previous) in paragraph.lines[0]
                    .glyphs
                    .iter()
                    .zip(&previous.lines[0].glyphs)
                    .take(3)
                {
                    assert_eq!(
                        glyph.x, previous.x,
                        "East Asian spacing survives renumbering"
                    );
                }
            }
        }
    }

    #[test]
    fn table_cell_wrap_preserves_paragraph_alignment_for_word_space_shrink() {
        let config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 69.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            styles_part: None,
        };
        let context = FixedWidthWrapContext::new();
        let text = "ab cd ef gh";
        for (alignment, expected_first_line) in [("both", "ab cd ef "), ("left", "ab cd ")] {
            let paragraph = Paragraph {
                properties: Some(ParagraphProperties {
                    alignment: Some(alignment.into()),
                    ..ParagraphProperties::default()
                }),
                runs: Vec::new(),
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            };
            let layout = layout_horizontal_paragraph_with_run_styles_context(
                &context,
                text,
                69.0,
                20.0,
                &plain_text_run_styles(text),
                &config.paragraph_style(&paragraph).1,
            )
            .unwrap();
            assert_eq!(layout.lines[0].text, expected_first_line, "{alignment}");
        }
    }

    #[test]
    fn table_cell_layout_expands_only_soft_line_interior_word_spaces() {
        let context = FixedWidthWrapContext::new();
        let text = "ab cd ef";
        let style = ParagraphStyle {
            alignment: "both".into(),
            ..ParagraphStyle::default()
        };
        let layout = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            text,
            55.0,
            20.0,
            &plain_text_run_styles(text),
            &style,
        )
        .unwrap();
        assert_eq!(layout.lines[0].text, "ab cd ");
        let first = &layout.lines[0];
        let content_edge = first
            .glyphs
            .iter()
            .filter(|glyph| !text[glyph.start..glyph.end].trim().is_empty())
            .map(|glyph| glyph.x + glyph.width)
            .reduce(f64::max)
            .unwrap();
        assert!(
            (content_edge - 55.0).abs() < 0.001,
            "content edge {content_edge}"
        );
        assert!((first.glyphs.last().unwrap().width - 5.0).abs() < 0.001);
        assert_eq!(layout.lines[1].text, "ef");
        assert!((layout.lines[1].visual_width - 20.0).abs() < 0.001);
    }

    #[test]
    fn table_cell_justification_respects_paragraph_indents() {
        let context = FixedWidthWrapContext::new();
        let text = "ab cd ef";
        let style = ParagraphStyle {
            alignment: "both".into(),
            first_line_indent: 10.0,
            right_indent: 15.0,
            ..ParagraphStyle::default()
        };
        let layout = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            text,
            80.0,
            20.0,
            &plain_text_run_styles(text),
            &style,
        )
        .unwrap();
        let first = &layout.lines[0];
        assert_eq!(first.text, "ab cd ");
        assert_eq!(first.left_inset, 10.0);
        assert_eq!(first.right_inset, 15.0);
        let content_edge = first
            .glyphs
            .iter()
            .filter(|glyph| !text[glyph.start..glyph.end].trim().is_empty())
            .map(|glyph| glyph.x + glyph.width)
            .reduce(f64::max)
            .unwrap();
        assert!(
            (content_edge - 55.0).abs() < 0.001,
            "content edge {content_edge}"
        );
    }

    #[test]
    fn table_cell_horizontal_layout_applies_snap_to_chars_grid_width() {
        let context = FixedWidthWrapContext::new();
        let style = ParagraphStyle {
            doc_grid_snap_to_chars: true,
            doc_grid_char_space_px: Some(4.0),
            ..ParagraphStyle::default()
        };
        let layout = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            "A",
            100.0,
            20.0,
            &plain_text_run_styles("A"),
            &style,
        )
        .expect("layout snapped table text");

        let expected = 10.0 * jade_docx_layout::line_wrap::EAST_ASIA_ADVANCE_SCALE + 4.0;
        assert!((layout.lines[0].visual_width - expected).abs() < 0.01);
    }

    #[test]
    fn column_count_accounts_for_spans_and_grids() {
        let table = Table {
            columns: vec![TableColumn { width: Some(40.0) }],
            rows: vec![TableRow {
                grid_before: Some(1),
                cells: vec![TableCell {
                    span: Some(2),
                    ..Default::default()
                }],
                grid_after: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(compute_column_count(&table), 4);
    }

    #[test]
    fn column_widths_distribute_remaining_space() {
        let table = Table {
            columns: vec![
                TableColumn { width: Some(40.0) },
                TableColumn::default(),
                TableColumn::default(),
            ],
            ..Default::default()
        };

        let available_width = 300.0;
        let widths = compute_column_widths(&table, 3, available_width);
        assert_eq!(widths.len(), 3);
        assert!((widths.iter().copied().sum::<f64>() - available_width).abs() < 1.0);
        assert!((widths[0] - points_to_px(40.0)).abs() < 0.1);
        assert!((widths[1] - widths[2]).abs() < 0.1);
        assert!(widths[1] > widths[0]);
    }

    #[test]
    fn column_widths_use_cell_preferences_when_table_grid_is_missing() {
        // Word permits tblGrid to be omitted or incomplete. In that case tcW
        // still carries the preferred width used by the table layout
        // algorithm; treating every column as unspecified incorrectly turns
        // this deliberately 1:2 table into equal-width columns.
        let table = Table {
            rows: vec![
                TableRow {
                    cells: vec![
                        TableCell {
                            style: [("width".to_string(), "72.00pt".to_string())]
                                .into_iter()
                                .collect(),
                            ..Default::default()
                        },
                        TableCell {
                            style: [("width".to_string(), "144.00pt".to_string())]
                                .into_iter()
                                .collect(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                TableRow {
                    cells: vec![TableCell {
                        span: Some(2),
                        style: [("width".to_string(), "216.00pt".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 2, 600.0);
        assert!((widths[0] - points_to_px(72.0)).abs() < 0.1);
        assert!((widths[1] - points_to_px(144.0)).abs() < 0.1);
    }

    #[test]
    fn spanning_cell_preference_is_shared_across_missing_grid_columns() {
        // A tcW belongs to the whole gridSpan, not to each participating
        // column. Sharing it prevents a 150pt two-column cell from becoming a
        // 300pt region while still giving following cells their own width.
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        span: Some(2),
                        style: [("width".to_string(), "150.00pt".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                    TableCell {
                        style: [("width".to_string(), "75.00pt".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 3, 600.0);
        assert!((widths[0] - points_to_px(75.0)).abs() < 0.1);
        assert!((widths[1] - points_to_px(75.0)).abs() < 0.1);
        assert!((widths[2] - points_to_px(75.0)).abs() < 0.1);
    }

    #[test]
    fn percentage_cell_preferences_use_preferred_table_width() {
        // tcW percentages are relative to the overall table width, while tblW
        // percentages are relative to the page text extent. A 50%-wide table
        // on a 600px text area therefore gives these 25%/75% cells 75/225px.
        let table = Table {
            style: [("width".to_string(), "50.00%".to_string())]
                .into_iter()
                .collect(),
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        style: [("width".to_string(), "25.00%".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                    TableCell {
                        style: [("width".to_string(), "75.00%".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 2, 600.0);
        assert!((widths[0] - 75.0).abs() < 0.1);
        assert!((widths[1] - 225.0).abs() < 0.1);
    }

    #[test]
    fn omitted_row_regions_use_before_and_after_preferred_widths() {
        // The row has one omitted column before its only cell and two omitted
        // columns after it. wAfter is the total width of both trailing columns,
        // so it must be shared rather than copied to each one.
        let table = Table {
            rows: vec![TableRow {
                grid_before: Some(1),
                width_before: Some("72.00pt".to_string()),
                cells: vec![TableCell {
                    style: [("width".to_string(), "100.00pt".to_string())]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                }],
                grid_after: Some(2),
                width_after: Some("144.00pt".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 4, 600.0);
        assert!((widths[0] - points_to_px(72.0)).abs() < 0.1);
        assert!((widths[1] - points_to_px(100.0)).abs() < 0.1);
        assert!((widths[2] - points_to_px(72.0)).abs() < 0.1);
        assert!((widths[3] - points_to_px(72.0)).abs() < 0.1);
    }

    #[test]
    fn row_and_cell_percentage_widths_use_their_distinct_reference_boxes() {
        // wBefore percentages use the 600px page text extent; tcW percentages
        // use the table's 50% tblW (300px). The same 50% token therefore means
        // 300px for the omitted region but 150px for the cell.
        let table = Table {
            style: [("width".to_string(), "50.00%".to_string())]
                .into_iter()
                .collect(),
            rows: vec![TableRow {
                grid_before: Some(1),
                width_before: Some("50.00%".to_string()),
                cells: vec![TableCell {
                    style: [("width".to_string(), "50.00%".to_string())]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 2, 600.0);
        assert!((widths[0] - 300.0).abs() < 0.1);
        assert!((widths[1] - 150.0).abs() < 0.1);
    }

    #[test]
    fn preferred_table_width_bounds_columns_without_grid_or_cell_widths() {
        // tblW percentages are relative to the page text extent. With no
        // gridCol or tcW constraints, a 50%-wide three-column table on a 600px
        // text area should distribute 300px, not expand to the full page.
        let table = Table {
            style: [("width".to_string(), "50.00%".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 3, 600.0);
        assert_eq!(widths, vec![100.0, 100.0, 100.0]);
    }

    #[test]
    fn preferred_table_width_fills_only_the_missing_grid_budget() {
        // The explicit 72pt/96px first grid column remains authoritative. The
        // two unspecified columns share the 204px left in a 50%-of-600px tblW.
        let table = Table {
            style: [("width".to_string(), "50.00%".to_string())]
                .into_iter()
                .collect(),
            columns: vec![
                TableColumn { width: Some(72.0) },
                TableColumn::default(),
                TableColumn::default(),
            ],
            ..Default::default()
        };

        let widths = compute_column_widths(&table, 3, 600.0);
        assert!((widths[0] - 96.0).abs() < 0.1);
        assert!((widths[1] - 102.0).abs() < 0.1);
        assert!((widths[2] - 102.0).abs() < 0.1);
    }

    #[test]
    fn normalize_column_widths_preserves_overflowing_table_width() {
        let mut widths = vec![240.0, 240.0, 240.0];
        let total = normalize_column_widths_for_table(&mut widths, 400.0);

        assert!((total - 720.0).abs() < 0.01);
        assert_eq!(widths, vec![240.0, 240.0, 240.0]);
    }

    #[test]
    fn normalize_column_widths_fills_missing_widths_from_available_space() {
        let mut widths = vec![0.0, 0.0, 0.0];
        let total = normalize_column_widths_for_table(&mut widths, 300.0);

        assert!((total - 300.0).abs() < 0.01);
        assert_eq!(widths, vec![100.0, 100.0, 100.0]);
    }

    #[test]
    fn apply_table_layout_width_scales_autofit_grid_to_fit_page() {
        // An over-wide grid (720 > 400) is scaled proportionally to the
        // available content width under autofit (Word's default).
        let mut widths = vec![240.0, 240.0, 240.0];
        let total = apply_table_layout_width(&mut widths, 400.0, TableLayoutMode::Autofit);

        assert!((total - 400.0).abs() < 0.01);
        for width in &widths {
            assert!((width - 400.0 / 3.0).abs() < 0.01, "column scaled evenly");
        }
    }

    #[test]
    fn apply_table_layout_width_preserves_fixed_overflow() {
        // Fixed layout keeps the grid verbatim and lets the table overflow.
        let mut widths = vec![240.0, 240.0, 240.0];
        let total = apply_table_layout_width(&mut widths, 400.0, TableLayoutMode::Fixed);

        assert!((total - 720.0).abs() < 0.01);
        assert_eq!(widths, vec![240.0, 240.0, 240.0]);
    }

    #[test]
    fn apply_table_layout_width_does_not_expand_narrow_autofit_grid() {
        // A grid narrower than the page is left untouched (no up-scaling).
        let mut widths = vec![100.0, 100.0];
        let total = apply_table_layout_width(&mut widths, 400.0, TableLayoutMode::Autofit);

        assert!((total - 200.0).abs() < 0.01);
        assert_eq!(widths, vec![100.0, 100.0]);
    }

    #[test]
    fn no_wrap_fixed_cell_keeps_its_preferred_width_during_autofit() {
        // ECMA-376 §17.4.29: a noWrap cell with an absolute preferred width
        // must not be collapsed while another cell can still shrink. The first
        // 225pt/300px column therefore stays fixed and the second absorbs the
        // complete 200px deficit.
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        style: [
                            ("width".to_string(), "225pt".to_string()),
                            ("no-wrap".to_string(), "true".to_string()),
                        ]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    },
                    TableCell {
                        style: [("width".to_string(), "225pt".to_string())]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut widths = vec![300.0, 300.0];

        let minimums = no_wrap_absolute_minimum_widths(&table, &widths);
        let total = apply_table_layout_width_with_minimums(
            &mut widths,
            400.0,
            resolve_table_layout_mode(&table),
            &minimums,
        );

        assert!((total - 400.0).abs() < 0.01);
        assert!((widths[0] - 300.0).abs() < 0.01);
        assert!((widths[1] - 100.0).abs() < 0.01);
    }

    #[test]
    fn no_wrap_percentage_cell_uses_measured_content_during_autofit() {
        // For pct/auto tcW, noWrap makes the contents one unbreakable string.
        // Word therefore allocates enough width for the measured text before
        // taking the remaining autofit deficit from an ordinary sibling.
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX".to_string(),
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
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        blocks: vec![BodyElement::from_paragraph(paragraph)],
                        style: [
                            ("width".to_string(), "50%".to_string()),
                            ("no-wrap".to_string(), "true".to_string()),
                        ]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    },
                    TableCell::default(),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut widths = vec![300.0, 300.0];
        let context = FixedWidthWrapContext::new();
        let minimums = autofit_content_widths(&context, &table, &widths, 0.0, &|_| 16.0, &|_| None)
            .expect("measure noWrap contents")
            .minimums;

        let total = apply_table_layout_width_with_minimums(
            &mut widths,
            600.0,
            TableLayoutMode::Autofit,
            &minimums,
        );

        assert!((minimums[0] - 400.0).abs() < 0.01);
        assert!((total - 600.0).abs() < 0.01);
        assert!((widths[0] - 400.0).abs() < 0.01);
        assert!((widths[1] - 200.0).abs() < 0.01);
    }

    #[test]
    fn regular_cell_content_redistributes_autofit_columns_without_growing_table() {
        // Word's autofit grid is only an initial width. A long ordinary cell
        // can borrow width from a sibling whose content is much shorter while
        // the table's saved total width remains unchanged. Unlike noWrap, the
        // measured preference is not a hard minimum: both cells may still wrap
        // later when the page itself is narrower than the table.
        let paragraph_with = |text: &str| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
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
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        blocks: vec![BodyElement::from_paragraph(paragraph_with(
                            "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
                        ))],
                        ..Default::default()
                    },
                    TableCell {
                        blocks: vec![BodyElement::from_paragraph(paragraph_with("XX"))],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = FixedWidthWrapContext::new();
        let mut widths = vec![300.0, 300.0];
        let constraints =
            autofit_content_widths(&context, &table, &widths, 0.0, &|_| 16.0, &|_| None)
                .expect("measure ordinary cell contents");

        let total = apply_table_layout_width_with_content_constraints(
            &mut widths,
            600.0,
            TableLayoutMode::Autofit,
            &constraints.minimums,
            &constraints.preferred,
        );

        assert_eq!(constraints.minimums, vec![0.0, 0.0]);
        assert_eq!(constraints.preferred, vec![400.0, 20.0]);
        assert!((total - 600.0).abs() < 0.01);
        assert_eq!(widths, vec![400.0, 200.0]);
    }

    #[test]
    fn natural_table_paragraph_layout_is_reused_only_when_it_fits() {
        let mut layouts = Rc::new(vec![NaturalParagraphLayout {
            layout: empty_cell_paragraph(16.0),
            required_width: 120.0,
            line_height: 16.0,
            styled_runs: Vec::new(),
        }]);

        assert!(
            take_fitting_natural_paragraph_layout(&mut layouts, 119.0, 16.0, "", &[],).is_none()
        );
        assert_eq!(layouts.len(), 1, "a non-fitting layout remains available");
        assert!(
            take_fitting_natural_paragraph_layout(&mut layouts, 120.0, 16.0, "different", &[],)
                .is_none(),
            "layout text must match the final revision view"
        );

        let reused = take_fitting_natural_paragraph_layout(&mut layouts, 120.0, 16.0, "", &[])
            .expect("an exactly fitting natural layout should be reused");
        assert_eq!(reused.lines.len(), 1);
        assert_eq!(layouts.len(), 1, "reused layout remains available to twins");
    }

    #[test]
    fn table_autofit_cache_hits_unchanged_content_and_misses_table_edits() {
        let paragraph_with = |text: &str| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
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
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let mut table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph_with("alpha"))],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let widths = vec![240.0];
        let signature =
            table_autofit_cache_signature(&table, &widths, 240.0, 8.0, &|_| 16.0, &|_| None)
                .expect("serializable table cache key");
        let measured = AutofitContentWidths {
            minimums: vec![0.0],
            preferred: vec![50.0],
            unbreakable: vec![0.0],
            natural_paragraph_layouts: Rc::new(vec![NaturalParagraphLayout {
                layout: empty_cell_paragraph(16.0),
                required_width: 50.0,
                line_height: 16.0,
                styled_runs: Vec::new(),
            }]),
        };
        let mut cache = TableAutofitCache::default();
        cache.insert(signature.clone(), measured);

        let hit = cache.get(&signature).expect("unchanged table should hit");
        assert_eq!(hit.preferred, vec![50.0]);
        let next_hit = cache.get(&signature).expect("same table should hit again");
        assert!(Rc::ptr_eq(
            &hit.natural_paragraph_layouts,
            &next_hit.natural_paragraph_layouts,
        ));
        assert_eq!(cache.stats().saved_paragraph_measures, 2);

        table.rows[0].cells[0].blocks = vec![BodyElement::from_paragraph(paragraph_with("edited"))];
        let edited =
            table_autofit_cache_signature(&table, &widths, 240.0, 8.0, &|_| 16.0, &|_| None)
                .expect("edited table cache key");
        assert!(cache.get(&edited).is_none());
        assert_eq!(cache.stats().hits, 2);
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn consecutive_table_layouts_skip_repeated_intrinsic_measurement() {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "cache me".to_string(),
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
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph)],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = FixedWidthWrapContext::new();
        let cache = RefCell::new(TableAutofitCache::default());

        let mut first_widths = vec![240.0];
        let first = apply_table_layout_width_for_table_with_content(
            &context,
            &table,
            &mut first_widths,
            240.0,
            8.0,
            &|_| 16.0,
            Some(&cache),
            &|_| None,
        )
        .expect("first layout");
        let first_measure_calls = context.measure_calls();
        assert!(first_measure_calls > 0);

        let mut second_widths = vec![240.0];
        let second = apply_table_layout_width_for_table_with_content(
            &context,
            &table,
            &mut second_widths,
            240.0,
            8.0,
            &|_| 16.0,
            Some(&cache),
            &|_| None,
        )
        .expect("second layout");

        assert_eq!(context.measure_calls(), first_measure_calls);
        assert_eq!(first.0, second.0);
        assert_eq!(first_widths, second_widths);
        assert_eq!(cache.borrow().stats().hits, 1);
        assert_eq!(cache.borrow().stats().misses, 1);
    }

    fn text_cell(text: &str) -> TableCell {
        let run: Run = serde_json::from_value(serde_json::json!({ "text": text })).unwrap();
        TableCell {
            blocks: vec![BodyElement::from_paragraph(Paragraph {
                runs: vec![run],
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            })],
            ..Default::default()
        }
    }

    #[test]
    fn mac_autofit_shrinks_each_column_toward_its_widest_word() {
        // table-object-edge-cases' nested table: 2 × 165.6pt in 320.4pt, with
        // `A` and `long wrapping value`, whose widest words take 17.8 and
        // 53.4pt with the cell margins. Word gives up 10.8pt as 6.4 and 4.4pt.
        use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let pt = |points: f64| points * 96.0 / 72.0;
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mut widths = vec![pt(165.6), pt(165.6)];
        let total = word_mac_shrink_autofit_to_words(
            &mut widths,
            pt(320.4),
            TableLayoutMode::Autofit,
            &[0.0, 0.0],
            &[pt(17.8), pt(53.4)],
        )
        .expect("shrinks");
        assert!((total - pt(320.4)).abs() < 1e-9);
        let first = widths[0] * 72.0 / 96.0;
        assert!(
            (first - (165.6 - 10.8 * 147.8 / 260.0)).abs() < 1e-6,
            "{first}"
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let mut widths = vec![pt(165.6), pt(165.6)];
        assert!(word_mac_shrink_autofit_to_words(
            &mut widths,
            pt(320.4),
            TableLayoutMode::Autofit,
            &[0.0, 0.0],
            &[pt(17.8), pt(53.4)],
        )
        .is_none());
    }

    #[test]
    fn mac_autofit_lays_out_a_stale_grid_from_its_cell_widths() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        let table = |grid: &[f32], cells: &[&str]| {
            let row = TableRow {
                cells: cells
                    .iter()
                    .map(|width| {
                        let mut cell = text_cell("A");
                        cell.style.insert("width".into(), (*width).into());
                        cell
                    })
                    .collect(),
                ..Default::default()
            };
            Table {
                columns: grid
                    .iter()
                    .map(|width| TableColumn {
                        width: Some(*width),
                    })
                    .collect(),
                rows: vec![row.clone(), row],
                ..Default::default()
            }
        };
        let pt = |widths: Vec<f64>| {
            widths
                .iter()
                .map(|w| (w * 0.75 * 10.0).round() / 10.0)
                .collect::<Vec<_>>()
        };
        // word-compat-comprehensive: an even 3312-twip grid under cells of
        // 2160, 3744 and 3744 twips.
        let stale = table(
            &[165.6, 165.6, 165.6],
            &["108.00pt", "187.20pt", "187.20pt"],
        );
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            pt(compute_column_widths(&stale, 3, 662.4)),
            vec![108.0, 187.2, 187.2]
        );
        // scroll-offset-repro: Word's own grid for cells of 3260 and 4536 twips.
        let fitted = table(&[160.85, 223.45], &["163.00pt", "226.80pt"]);
        assert_eq!(
            pt(compute_column_widths(&fitted, 2, 662.4)),
            vec![160.9, 223.4]
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            pt(compute_column_widths(&stale, 3, 662.4)),
            vec![165.6, 165.6, 165.6]
        );
        set_word_layout_platform(previous);
    }

    #[test]
    fn mac_autofit_without_a_usable_grid_shares_width_by_content() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mut table = Table {
            columns: vec![TableColumn { width: Some(72.0) }],
            rows: vec![TableRow {
                cells: vec![text_cell("AAAA AAAA"), text_cell("B"), text_cell("C")],
                ..Default::default()
            }],
            ..Default::default()
        };
        table.style.insert("width".into(), "50%".into());
        let context = FixedWidthWrapContext::new();
        let mut widths = compute_column_widths(&table, 3, 600.0);
        assert_eq!(widths, vec![100.0; 3], "the one grid column is ignored");
        apply_table_layout_width_for_table_with_content(
            &context,
            &table,
            &mut widths,
            600.0,
            8.0,
            &|_| 16.0,
            None,
            &|_| None,
        )
        .unwrap();
        // Ten per glyph and five per space, plus 8px padding on each side.
        let preferred = [85.0 + 16.0, 26.0, 26.0];
        let total: f64 = preferred.iter().sum();
        for (width, preferred) in widths.iter().zip(preferred) {
            assert!(
                (width - preferred * 300.0 / total).abs() < 1e-6,
                "{widths:?}"
            );
        }
        set_word_layout_platform(previous);
    }

    #[test]
    fn table_autofit_cache_bypasses_unsupported_nested_tables() {
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::Table(Box::default())],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!table_autofit_cache_supported(&table));
    }

    #[test]
    fn resolve_table_layout_mode_defaults_absent_to_autofit() {
        let mut table = Table::default();
        assert_eq!(resolve_table_layout_mode(&table), TableLayoutMode::Autofit);

        table.table_layout = Some(TableLayoutMode::Fixed);
        assert_eq!(resolve_table_layout_mode(&table), TableLayoutMode::Fixed);
    }

    /// 表のセルでも、run に付いた変更履歴を落とさない。
    ///
    /// ここで落としていたので、XML に `w:ins` / `w:del` が入っていても表の中
    /// だけ色が付かなかった。描く側はこの値を見て帯を敷く。
    fn picture_run(width: f64, height: f64) -> Run {
        Run {
            text: String::new(),
            properties: None,
            image: Some(RunImage {
                width_px: Some(width),
                height_px: Some(height),
                layout: RunImageLayout {
                    kind: RunImageKind::Inline,
                    ..RunImageLayout::default()
                },
                ..RunImage::default()
            }),
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
        }
    }

    /// Lays out `runs` as one table cell paragraph 500px wide on 16px lines.
    fn cell_paragraph_with_runs(runs: Vec<Run>) -> CellParagraph {
        let paragraph = Paragraph {
            runs,
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let (text, styled_runs, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|_| None);
        let painter = RecordingPainter::with_width_per_char(7.0);
        let context = TableLineWrapContext::new(&painter);
        let style = ParagraphStyle::default();
        let lines = context
            .wrap_horizontal(
                &text,
                &styled_run_styles(&text, &styled_runs),
                &style,
                0,
                500.0,
            )
            .unwrap();
        let mut paragraph =
            horizontal_cell_paragraph_from_wrapped_lines(&text, 16.0, &style, 0, &lines);
        paragraph.styled_runs = styled_runs;
        paragraph
    }

    #[test]
    fn a_cell_picture_stands_on_the_baseline_of_a_line_as_tall_as_it() {
        // table-object-edge-cases: Word for Mac draws a cell line holding only
        // an 89.1pt picture 89.1pt tall, the picture's foot on the baseline.
        let paragraph = cell_paragraph_with_runs(vec![picture_run(118.8, 118.8)]);
        assert_eq!(paragraph.text, "\u{fffc}");
        let line = &paragraph.lines[0];
        assert_eq!(line.min_advance, 118.8);
        assert_eq!(line.word_baseline_offset, Some(118.8));
        assert_eq!(paragraph.line_advance(line), 118.8);
        assert_eq!(paragraph.height, 118.8);
        assert!((line.visual_width - 118.8).abs() < 1e-9);
        // A short picture keeps the paragraph's own line.
        let short = cell_paragraph_with_runs(vec![picture_run(10.0, 8.0)]);
        assert_eq!(short.height, 16.0);
    }

    #[test]
    fn a_cell_picture_has_no_model_character() {
        let mut text = picture_run(0.0, 0.0);
        text.image = None;
        text.text = "AB".to_string();
        let paragraph = cell_paragraph_with_runs(vec![picture_run(40.0, 30.0), text]);
        assert_eq!(paragraph.text, "\u{fffc}AB");
        assert_eq!(paragraph.model_text(), "AB");
        assert_eq!(paragraph.model_offset(0), 0);
        assert_eq!(paragraph.model_offset(3), 0);
        assert_eq!(paragraph.model_offset(5), 2);
        // The text's descent hangs below the picture.
        let line = &paragraph.lines[0];
        assert_eq!(
            line.word_baseline_offset.map(|baseline| baseline >= 30.0),
            Some(true)
        );
        assert!(line.min_advance >= 30.0);
        let mut fragments = vec![SelectionFragmentLayout {
            baseline_shift_px: 0.0,
            start: 3,
            end: 5,
            x: 0.0,
            width: 14.0,
            glyphs: Rc::from([]),
        }];
        paragraph.remap_selection_fragments(&mut fragments);
        assert_eq!((fragments[0].start, fragments[0].end), (0, 2));
    }

    #[test]
    fn collect_paragraph_text_appends_superscript_note_markers() {
        let mut reference = Run {
            text: String::new(),
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
            footnote_reference_ids: vec!["7".to_string()],
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        };
        let mut body = reference.clone();
        body.text = "Cell".to_string();
        body.footnote_reference_ids.clear();
        reference.text = String::new();
        let paragraph = Paragraph {
            runs: vec![body, reference],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        // Without an assigned mark the empty reference run contributes nothing.
        let (plain, plain_runs, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|_| None);
        assert_eq!(plain, "Cell");
        assert_eq!(plain_runs.len(), 1);

        let mut markers = NoteMarkers::default();
        markers.footnotes.insert("7".to_string(), "3".to_string());
        let (text, runs, _) = collect_paragraph_text(&paragraph, true, false, &markers, &|_| None);
        assert_eq!(text, "Cell3");
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[1].start, runs[1].end), (4, 5));
        assert_eq!(
            runs[1]
                .properties
                .as_ref()
                .and_then(|props| props.baseline.as_deref()),
            Some("superscript")
        );
    }

    #[test]
    fn collect_paragraph_text_keeps_revision_marks() {
        let mut run = Run {
            text: "消えた".to_string(),
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
        };
        run.revision = Some(RevisionMark {
            kind: RevisionKind::Deleted,
            id: Some("1".into()),
            author: Some("ellisii".into()),
            date: None,
            date_utc: None,
            insertion: None,
        });
        let paragraph = Paragraph {
            runs: vec![run],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let (text, styled_runs, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|_| None);
        assert_eq!(text, "消えた");
        assert_eq!(styled_runs.len(), 1);
        assert!(
            matches!(
                styled_runs[0].revision.as_ref().map(|mark| &mark.kind),
                Some(RevisionKind::Deleted)
            ),
            "変更履歴が落ちている: {:?}",
            styled_runs[0].revision
        );
    }

    #[test]
    fn collect_paragraph_text_keeps_run_ranges_and_properties() {
        let paragraph = Paragraph {
            runs: vec![
                Run {
                    text: "abc".to_string(),
                    properties: Some(RunProperties {
                        color: Some("#008000".into()),
                        ..Default::default()
                    }),
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
                },
                Run {
                    text: "def".to_string(),
                    properties: Some(RunProperties {
                        strike: Some(true),
                        ..Default::default()
                    }),
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
                },
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let (text, styled_runs, _) =
            collect_paragraph_text(&paragraph, true, false, &NoteMarkers::default(), &|_| None);
        assert_eq!(text, "abcdef");
        assert_eq!(styled_runs.len(), 2);
        assert_eq!((styled_runs[0].start, styled_runs[0].end), (0, 3));
        assert_eq!(
            styled_runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.color.as_deref()),
            Some("#008000")
        );
        assert_eq!((styled_runs[1].start, styled_runs[1].end), (3, 6));
        assert_eq!(
            styled_runs[1]
                .properties
                .as_ref()
                .and_then(|props| props.strike),
            Some(true)
        );
    }

    #[test]
    fn table_grid_baseline_centres_the_shared_font_box() {
        use jade_docx_layout::font_resolver::ResolvedFontSlot;
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, WordGridLineSizing, WordLayoutPlatform,
        };
        use jade_docx_layout::line_wrap::{InlineCluster, InlineClusterLayout, LineBreakKind};
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let grid = DocGrid {
            grid_type: Some("lines".into()),
            line_pitch: Some(18.0),
            char_space: None,
        };
        let mut cluster = InlineCluster::grapheme(0, 1, 10.0, "A", false, false, false);
        cluster.layout = Some(InlineClusterLayout {
            character_border_edges: 0,
            script: ScriptCategory::Latin,
            font_slot: ResolvedFontSlot::LatinAscii,
            leading_advance: 0.0,
            glyph_box_width: 10.0,
            measured_glyph_width: 10.0,
            adjusted_glyph_width: 10.0,
            font_descriptor_hash: 0,
            font_ascent: Some(12.0),
            font_descent: Some(4.0),
        });
        let mut wrapped = WrappedLine {
            segments: Vec::new(),
            clusters: vec![cluster],
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: None,
            break_properties: None,
            leaders: Vec::new(),
            tab_stops: Vec::new(),
            hyphen: None,
        };
        let mut style = ParagraphStyle {
            word_east_asian_font_line_box: true,
            word_grid_line_sizing: WordGridLineSizing::new(None, true, Some(&grid), None, 10.5),
            ..Default::default()
        };
        let layout = |line: &WrappedLine, style: &ParagraphStyle| {
            horizontal_cell_paragraph_from_wrapped_lines(
                "A",
                24.0,
                style,
                0,
                std::slice::from_ref(line),
            )
            .lines[0]
                .word_baseline_offset
        };
        assert_eq!(layout(&wrapped, &style), Some(16.0));
        let mut space = InlineCluster::grapheme(1, 2, 10.0, " ", true, false, false);
        space.layout = wrapped.clusters[0].layout.clone();
        space.layout.as_mut().unwrap().font_ascent = Some(80.0);
        space.layout.as_mut().unwrap().font_descent = Some(20.0);
        wrapped.clusters.push(space);
        assert_eq!(
            layout(&wrapped, &style),
            Some(16.0),
            "blank runs do not size visible text"
        );
        wrapped.clusters.pop();
        wrapped.clusters[0].layout.as_mut().unwrap().font_ascent = Some(22.0);
        wrapped.clusters[0].layout.as_mut().unwrap().font_descent = Some(8.0);
        assert_eq!(
            layout(&wrapped, &style),
            Some(22.0),
            "an oversized box gets no negative leading"
        );
        wrapped.clusters[0].layout.as_mut().unwrap().font_ascent = Some(12.0);
        wrapped.clusters[0].layout.as_mut().unwrap().font_descent = Some(4.0);
        style.word_grid_line_sizing = None;
        assert_eq!(layout(&wrapped, &style), Some(12.0));
        style.word_grid_line_sizing = WordGridLineSizing::new(None, true, Some(&grid), None, 10.5);
        wrapped.clusters[0].layout.as_mut().unwrap().font_descent = None;
        assert_eq!(
            layout(&wrapped, &style),
            Some(12.0),
            "unknown metrics keep the preceding ascent"
        );
    }

    #[test]
    fn terminal_marker_after_nested_table_keeps_model_indices_without_a_line_box() {
        let painter = RecordingPainter::new();
        let context = TableLineWrapContext::new(&painter);
        let config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 200.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            styles_part: None,
        };
        let paragraph = |text: &str| -> Paragraph {
            serde_json::from_value(serde_json::json!({"runs": [{"text": text}]})).unwrap()
        };
        let nested = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph("nested"))],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        blocks: vec![
                            BodyElement::from_table(nested),
                            BodyElement::from_paragraph(paragraph("")),
                        ],
                        ..Default::default()
                    },
                    TableCell {
                        blocks: vec![BodyElement::from_paragraph(paragraph("next"))],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut cells = prepare_row_layout(
            &context,
            &table.rows[0],
            &table,
            &[100.0, 100.0],
            &config,
            None,
            0,
            1,
            &mut NaturalParagraphLayoutCache::default(),
            None,
            &|_| None,
        )
        .unwrap();
        let CellBlockKind::Table(nested) = &cells[0].blocks[0].kind else {
            panic!()
        };
        assert!(
            (cells[0].height - nested.height).abs() < 1e-9,
            "the empty terminal marker must not add a paragraph line after the nested table"
        );
        assert_eq!(cells[0].hidden_end_marker_paragraph_count, 0);
        assert_eq!(cells[0].blocks[1].height, 0.0);
        assert_eq!(
            assign_row_paragraph_indices(&mut cells, 7, &HashMap::new()),
            10
        );
        assert_eq!(collect_cell_paragraph_indices(&cells[0]), vec![8]);
        let CellBlockKind::Table(nested) = &cells[0].blocks[0].kind else {
            panic!()
        };
        assert_eq!(
            collect_cell_paragraph_indices(&nested.rows[0].cells[0]),
            vec![7]
        );
        assert_eq!(collect_cell_paragraph_indices(&cells[1]), vec![9]);
    }

    #[test]
    fn terminal_nested_marker_keeps_breaks_fields_and_numbered_empty_paragraphs() {
        let empty: Paragraph = serde_json::from_value(serde_json::json!({"runs": []})).unwrap();
        let cell = |paragraph| TableCell {
            blocks: vec![
                BodyElement::from_table(Table::default()),
                BodyElement::from_paragraph(paragraph),
            ],
            ..Default::default()
        };
        assert_eq!(
            terminal_cell_marker_after_nested_table(&cell(empty.clone())),
            Some(1)
        );
        for text in ["text", "\n", "\t", " "] {
            let paragraph =
                serde_json::from_value(serde_json::json!({"runs": [{"text": text}]})).unwrap();
            assert_eq!(
                terminal_cell_marker_after_nested_table(&cell(paragraph)),
                None
            );
        }
        assert_eq!(
            terminal_cell_marker_after_nested_table(&cell(paragraph_with_field(
                RunFieldKind::PageNumber,
                ""
            ))),
            None
        );
        let mut numbered = empty.clone();
        numbered.properties = Some(ParagraphProperties {
            numbering: Some(crate::document::Numbering {
                id: Some("1".into()),
                level: Some(0),
            }),
            ..Default::default()
        });
        assert_eq!(
            terminal_cell_marker_after_nested_table(&cell(numbered)),
            None
        );
        let mut note = empty.clone();
        let mut run: Run = serde_json::from_value(serde_json::json!({"text":""})).unwrap();
        run.has_footnote_ref = true;
        note.runs.push(run);
        assert_eq!(terminal_cell_marker_after_nested_table(&cell(note)), None);
        let mut vertical = cell(empty.clone());
        vertical.text_direction = Some(TextDirection::TbRl);
        assert_eq!(terminal_cell_marker_after_nested_table(&vertical), None);
        let mut separated = cell(empty.clone());
        separated.blocks.push(BodyElement::from_paragraph(empty));
        assert_eq!(terminal_cell_marker_after_nested_table(&separated), None);
    }

    #[test]
    fn hide_mark_suppresses_only_an_empty_cell_end_marker() {
        let empty_paragraph = BodyElement::from_paragraph(Paragraph {
            runs: Vec::new(),
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        });
        let hidden_empty = TableCell {
            blocks: vec![empty_paragraph.clone()],
            style: [("hide-mark".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let ordinary_empty = TableCell {
            blocks: vec![empty_paragraph],
            ..Default::default()
        };
        let hidden_nested_table = TableCell {
            blocks: vec![BodyElement::from_table(Table::default())],
            style: [("hide-mark".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        assert!(cell_contains_only_hidden_end_marker(&hidden_empty));
        assert!(!cell_contains_only_hidden_end_marker(&ordinary_empty));
        assert!(!cell_contains_only_hidden_end_marker(&hidden_nested_table));
    }

    #[test]
    fn table_cell_fit_text_scales_all_runs_without_losing_formatting() {
        // `w:tcFitText` applies one cell-wide ratio to the paragraph. Preserve
        // each run's existing relative width and unrelated formatting instead
        // of flattening the cell into a synthetic plain-text run.
        let styled_runs = vec![
            StyledRunRange {
                picture: None,
                start: 0,
                end: 2,
                properties: Some(RunProperties {
                    bold: Some(true),
                    ..RunProperties::default()
                }),
                revision: None,
            },
            StyledRunRange {
                picture: None,
                start: 2,
                end: 4,
                properties: Some(RunProperties {
                    italic: Some(true),
                    text_scale: Some(80.0),
                    ..RunProperties::default()
                }),
                revision: None,
            },
        ];

        let fitted = fit_text_run_ranges("abcd", &styled_runs, 200.0, 100.0);

        assert_eq!(fitted.len(), 2);
        let first = fitted[0].properties.as_ref().expect("first properties");
        let second = fitted[1].properties.as_ref().expect("second properties");
        assert_eq!(first.bold, Some(true));
        assert_eq!(second.italic, Some(true));
        assert!((first.text_scale.expect("first scale") - 49.995).abs() < 0.001);
        assert!((second.text_scale.expect("second scale") - 39.996).abs() < 0.001);
    }

    #[test]
    fn mac_cell_fit_text_groups_runs_and_condenses_by_whole_percent() {
        let styled_runs = vec![
            StyledRunRange {
                picture: None,
                start: 0,
                end: 2,
                properties: Some(RunProperties {
                    bold: Some(true),
                    ..RunProperties::default()
                }),
                revision: None,
            },
            StyledRunRange {
                picture: None,
                start: 2,
                end: 4,
                properties: None,
                revision: None,
            },
        ];
        // 109.2 / 179.17 = 60.9%: Word prints the run at 60%.
        let condensed = mac_cell_fit_text_run_ranges("abcd", &styled_runs, 179.17, 109.2);
        let fits: Vec<_> = condensed
            .iter()
            .map(|run| run.properties.as_ref().unwrap().fit_text.clone().unwrap())
            .collect();
        assert_eq!(fits[0], fits[1], "one shared group");
        assert_eq!(fits[0].val, (179.17 * 0.6 * 15.0_f64).round() as u32);
        assert_eq!(condensed[0].properties.as_ref().unwrap().bold, Some(true));

        let expanded = mac_cell_fit_text_run_ranges("WIDE", &[], 36.5, 144.0);
        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0]
                .properties
                .as_ref()
                .unwrap()
                .fit_text
                .as_ref()
                .unwrap()
                .val,
            2160
        );
    }

    #[test]
    fn table_cell_fit_text_expands_an_unstyled_run_to_the_cell_width() {
        // The on/off property is bidirectional: short text expands as well as
        // long text condenses. An unstyled paragraph needs a synthetic range
        // so the layout and Canvas draw pass both receive the same scale.
        let fitted = fit_text_run_ranges("WIDE", &[], 40.0, 160.0);

        assert_eq!(fitted.len(), 1);
        assert_eq!((fitted[0].start, fitted[0].end), (0, 4));
        let scale = fitted[0]
            .properties
            .as_ref()
            .and_then(|properties| properties.text_scale)
            .expect("synthetic text scale");
        assert!((scale - 399.975).abs() < 0.001);
    }

    #[test]
    fn table_cell_fit_text_keeps_mixed_runs_on_one_visual_line() {
        let context = FixedWidthWrapContext::new();
        let text = "MIXED BOLD ITALIC CELL FIT TEXT";
        let styled_runs = vec![
            StyledRunRange {
                picture: None,
                start: 0,
                end: 11,
                properties: Some(RunProperties {
                    bold: Some(true),
                    ..RunProperties::default()
                }),
                revision: None,
            },
            StyledRunRange {
                picture: None,
                start: 11,
                end: text.len(),
                properties: Some(RunProperties {
                    italic: Some(true),
                    ..RunProperties::default()
                }),
                revision: None,
            },
        ];
        let natural = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            text,
            1_000_000_000.0,
            16.0,
            &styled_run_styles(text, &styled_runs),
            &ParagraphStyle::default(),
        )
        .expect("natural layout");
        let fitted = fit_text_run_ranges(text, &styled_runs, natural.max_line_width, 150.0);
        let layout = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            text,
            150.0,
            16.0,
            &styled_run_styles(text, &fitted),
            &ParagraphStyle::default(),
        )
        .expect("fitted layout");

        assert_eq!(
            layout.lines.len(),
            1,
            "natural={} fitted={:?} lines={:?}",
            natural.max_line_width,
            fitted
                .iter()
                .map(|run| run.properties.as_ref().and_then(|props| props.text_scale))
                .collect::<Vec<_>>(),
            layout
                .lines
                .iter()
                .map(|line| (&line.text, line.visual_width))
                .collect::<Vec<_>>()
        );
        assert!(layout.max_line_width <= 150.0);
    }

    #[test]
    fn table_cell_fit_text_scales_vertical_advances_to_row_content_height() {
        let context = FixedWidthWrapContext::new();
        let text = "あいうえ";
        let natural = layout_vertical_paragraph_with_run_styles_context(
            &context,
            text,
            TextDirection::TbRl,
            None,
            10.0,
            &plain_text_run_styles(text),
            false,
        )
        .expect("natural vertical layout");
        let natural_advance = natural
            .lines
            .iter()
            .map(|line| line.visual_width)
            .sum::<f64>();
        let fitted = fit_text_run_ranges(text, &[], natural_advance, 20.0);
        let layout = layout_vertical_paragraph_with_run_styles_context(
            &context,
            text,
            TextDirection::TbRl,
            None,
            10.0,
            &styled_run_styles(text, &fitted),
            false,
        )
        .expect("fitted vertical layout");
        let fitted_advance = layout
            .lines
            .iter()
            .map(|line| line.visual_width)
            .sum::<f64>();

        let column_ranges = &layout
            .vertical_layout
            .as_ref()
            .expect("vertical metadata")
            .column_ranges;
        assert_eq!(column_ranges.len(), 1);
        assert_eq!(column_ranges[0], 0..4);
        assert!(fitted_advance <= 20.0);
        assert!(fitted_advance > 19.0);
    }

    #[test]
    fn line_style_spans_preserves_unstyled_gaps() {
        let styled_runs = vec![
            StyledRunRange {
                picture: None,
                start: 0,
                end: 2,
                properties: None,
                revision: None,
            },
            StyledRunRange {
                picture: None,
                start: 4,
                end: 6,
                properties: None,
                revision: None,
            },
        ];

        let spans = line_style_spans(0, 6, &styled_runs);
        assert_eq!(spans.len(), 3);
        assert_eq!(
            (spans[0].start, spans[0].end, spans[0].run_index),
            (0, 2, Some(0))
        );
        assert_eq!(
            (spans[1].start, spans[1].end, spans[1].run_index),
            (2, 4, None)
        );
        assert_eq!(
            (spans[2].start, spans[2].end, spans[2].run_index),
            (4, 6, Some(1))
        );
    }

    #[test]
    fn line_range_width_handles_partial_overlap() {
        let line = LineLayout {
            min_advance: 0.0,
            left_inset: 0.0,
            right_inset: 0.0,
            text: "ab".to_string(),
            visual_width: 10.0,
            start: 0,
            end: 2,
            glyphs: vec![
                GlyphBox {
                    start: 0,
                    end: 1,
                    x: 0.0,
                    width: 4.0,
                },
                GlyphBox {
                    start: 1,
                    end: 2,
                    x: 4.0,
                    width: 6.0,
                },
            ],
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((2, 2)),
            break_properties: None,
            word_baseline_offset: None,
        };

        assert!((line_range_width(&line, 0, 1) - 4.0).abs() < f64::EPSILON);
        assert!((line_range_width(&line, 1, 2) - 6.0).abs() < f64::EPSILON);
        assert!((line_range_width(&line, 0, 2) - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mac_soft_cell_line_aligns_content_without_losing_space_caret_advance() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let line = LineLayout {
            min_advance: 0.0,
            text: "AB ".into(),
            visual_width: 23.0,
            left_inset: 0.0,
            right_inset: 0.0,
            start: 17,
            end: 20,
            glyphs: vec![
                GlyphBox {
                    start: 17,
                    end: 18,
                    x: 0.0,
                    width: 10.0,
                },
                GlyphBox {
                    start: 18,
                    end: 19,
                    x: 10.0,
                    width: 10.0,
                },
                GlyphBox {
                    start: 19,
                    end: 20,
                    x: 20.0,
                    width: 3.0,
                },
            ],
            break_kind: LineBreakKind::Soft,
            break_range: None,
            break_properties: None,
            word_baseline_offset: None,
        };
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let center = line.aligned_start_x(5.0, 100.0, TextAlign::Center);
        let right = line.aligned_start_x(5.0, 100.0, TextAlign::Right);
        let left = line.aligned_start_x(5.0, 100.0, TextAlign::Left);
        let caret = line.horizontal_metrics(center).cursor_end_x;
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let windows_center = line.aligned_start_x(5.0, 100.0, TextAlign::Center);
        set_word_layout_platform(previous);
        assert_eq!(center, 45.0);
        assert_eq!(right, 85.0);
        assert_eq!(left, 5.0);
        assert_eq!(caret, 68.0);
        assert_eq!(windows_center, 43.5);
        assert_eq!(line.visual_width, 23.0);
        assert_eq!(line.glyphs[2].width, 3.0);
    }

    #[test]
    fn line_layout_visual_end_x_uses_visual_width() {
        let line = LineLayout {
            min_advance: 0.0,
            left_inset: 0.0,
            right_inset: 0.0,
            text: "ab".to_string(),
            visual_width: 10.0,
            start: 0,
            end: 2,
            glyphs: Vec::new(),
            break_kind: LineBreakKind::ParagraphEnd,
            break_range: Some((2, 2)),
            break_properties: None,
            word_baseline_offset: None,
        };

        assert!((line.visual_end_x(24.0) - 34.0).abs() < f64::EPSILON);
    }

    #[test]
    fn table_horizontal_layout_uses_jade_docx_layout_wraps_on_whitespace() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph = layout_horizontal_paragraph_with_context(&ctx, "Hello World", 60.0, 14.0)
            .expect("layout paragraph");
        assert_eq!(paragraph.lines.len(), 2);
        assert_eq!(paragraph.lines[0].text, "Hello ");
        assert_eq!(paragraph.lines[0].start, 0);
        assert_eq!(paragraph.lines[0].end, 6);
        assert_eq!(paragraph.lines[1].text, "World");
        assert_eq!(paragraph.lines[1].start, 6);
    }

    #[test]
    fn table_horizontal_layout_uses_jade_docx_layout_newlines() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph = layout_horizontal_paragraph_with_context(&ctx, "Line1\nLine2", 100.0, 14.0)
            .expect("layout paragraph");
        assert_eq!(paragraph.lines.len(), 2);
        assert_eq!(paragraph.lines[0].text, "Line1");
        assert_eq!(paragraph.lines[1].text, "Line2");
        assert_eq!(paragraph.lines[1].start, 6);
        assert_eq!(paragraph.lines[0].break_kind, LineBreakKind::Hard);
        assert_eq!(paragraph.lines[1].break_kind, LineBreakKind::ParagraphEnd);
        assert_eq!(
            table_formatting_mark_symbol(true, paragraph.lines[0].break_kind),
            formatting_mark_symbol(LineBreakKind::Hard)
        );
        assert_eq!(
            table_formatting_mark_symbol(true, paragraph.lines[1].break_kind),
            formatting_mark_symbol(LineBreakKind::ParagraphEnd)
        );
        assert_eq!(
            table_formatting_mark_symbol(false, paragraph.lines[0].break_kind),
            None
        );
    }

    #[test]
    fn table_horizontal_layout_uses_jade_docx_layout_for_cjk_wraps() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph = layout_horizontal_paragraph_with_context(&ctx, "あいうえお", 20.0, 14.0)
            .expect("layout paragraph");
        assert_eq!(paragraph.lines.len(), 3);
        assert_eq!(paragraph.lines[0].text, "あい");
        assert_eq!(paragraph.lines[1].text, "うえ");
        assert_eq!(paragraph.lines[2].text, "お");
        for line in paragraph.lines.iter() {
            assert!(line.visual_width <= 20.0 + f64::EPSILON);
        }
    }

    #[test]
    fn table_vertical_layout_uses_jade_docx_layout_column_ranges() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph = layout_vertical_paragraph_with_context(
            &ctx,
            "あいう",
            TextDirection::TbRl,
            Some(25.0),
            10.0,
        )
        .expect("layout paragraph");

        assert_eq!(paragraph.lines.len(), 3);
        assert_eq!(paragraph.column_count(), 2);
        assert_eq!(paragraph.vertical_line_count(), 3);
        assert_eq!(paragraph.lines[0].text, "あ");
        assert_eq!(paragraph.lines[2].text, "う");
    }

    #[test]
    fn table_vertical_layout_uses_jade_docx_layout_newline_breaks() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph =
            layout_vertical_paragraph_with_context(&ctx, "あ\nい", TextDirection::TbRl, None, 10.0)
                .expect("layout paragraph");

        assert_eq!(paragraph.lines.len(), 3);
        assert_eq!(paragraph.lines[0].text, "あ");
        assert_eq!(paragraph.lines[1].text, "");
        assert_eq!(paragraph.lines[1].start, 3);
        assert_eq!(paragraph.lines[2].text, "い");
    }

    #[test]
    fn table_bottom_to_top_layout_keeps_ascii_as_glyph_lines() {
        let ctx = FixedWidthWrapContext::new();
        let paragraph =
            layout_vertical_paragraph_with_context(&ctx, "Hello", TextDirection::BtLr, None, 10.0)
                .expect("layout paragraph");

        assert_eq!(
            paragraph
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["H", "e", "l", "l", "o"]
        );
        assert_eq!(paragraph.column_count(), 1);
    }

    #[test]
    fn rotated_table_text_baseline_follows_text_direction_axis() {
        let block_top = 20.0;
        let column_height = 80.0;
        let line_height = 10.0;
        let base_x = 40.0;
        let content_right = 100.0;

        assert!(
            rotated_visual_baseline_y(TextDirection::TbRlV, block_top, column_height, line_height)
                < block_top + line_height
        );
        assert!(
            rotated_visual_baseline_y(TextDirection::BtLr, block_top, column_height, line_height)
                > block_top + column_height - line_height
        );
        assert!(
            rotated_visual_anchor_x(TextDirection::TbRlV, base_x, content_right, line_height)
                > base_x
        );
        assert_eq!(
            rotated_visual_anchor_x(TextDirection::BtLr, base_x, content_right, line_height),
            base_x
        );
    }

    /// A shaded cell set in 縦書き draws its text in the text colour: the
    /// shading's fill used to stay on the painter, and the text vanished
    /// into it.
    #[test]
    fn vertical_text_in_a_shaded_cell_is_drawn_in_the_text_colour() {
        let painter = RecordingPainter::with_width_per_char(10.0);
        let mut renderer = CanvasRenderer::with_host(
            Box::new(super::super::recording_host::RecordingHost {
                painter: painter.clone(),
            }),
            false,
        )
        .unwrap();
        let paragraph = |text: &str| {
            BodyElement::from_paragraph(crate::document::Paragraph {
                runs: vec![serde_json::from_value(serde_json::json!({ "text": text })).unwrap()],
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            })
        };
        let mut cell = TableCell {
            blocks: vec![paragraph("縦書き")],
            text_direction: Some(TextDirection::TbRlV),
            ..Default::default()
        };
        cell.style
            .insert("background-color".into(), "#FFF2CC".into());
        let table = Table {
            rows: vec![TableRow {
                cells: vec![cell],
                ..Default::default()
            }],
            columns: vec![crate::document::TableColumn { width: Some(100.0) }],
            ..Default::default()
        };
        let document = crate::document::Document {
            body: Some(crate::document::Body {
                elements: vec![BodyElement::from_table(table), paragraph("")],
            }),
            ..Default::default()
        };
        renderer
            .render(&document, &crate::Options::default())
            .expect("render");
        let mut fill = String::new();
        let mut saved = Vec::new();
        let mut drawn = Vec::new();
        for op in painter.ops() {
            match op {
                Op::Save => saved.push(fill.clone()),
                Op::Restore => fill = saved.pop().unwrap_or_default(),
                Op::SetFillStyle(value) => fill = value,
                Op::FillText(text, _, _)
                    if "縦書き".contains(text.as_str()) && !text.is_empty() =>
                {
                    drawn.push(fill.clone())
                }
                _ => {}
            }
        }
        assert!(!drawn.is_empty(), "the vertical text was drawn");
        assert!(
            drawn
                .iter()
                .all(|fill| !fill.eq_ignore_ascii_case("#FFF2CC")),
            "drawn in the shading colour: {drawn:?}"
        );
    }

    #[test]
    fn upright_vertical_cell_paint_and_selection_share_indented_columns() {
        use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        use jade_painter::recording::Op;
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let painter = jade_painter::recording::RecordingPainter::with_width_per_char(10.0);
        let mut renderer = CanvasRenderer::with_host(
            Box::new(super::super::recording_host::RecordingHost {
                painter: painter.clone(),
            }),
            false,
        )
        .unwrap();
        let text = "こんにちは";
        let runs = vec![StyledRunRange {
            picture: None,
            start: 0,
            end: text.len(),
            properties: Some(RunProperties {
                font_family: Some("MS Mincho".into()),
                font_size: Some(10.5),
                ..Default::default()
            }),
            revision: None,
        }];
        let style = ParagraphStyle {
            right_indent: 8.0,
            ..Default::default()
        };
        let mut paragraph = {
            let context = TableLineWrapContext::new(&painter);
            layout_paragraph_with_table_context(
                &context,
                text,
                100.0,
                24.0,
                Some(TextDirection::TbRlV),
                Some(48.0),
                &runs,
                &style,
                8.0,
            )
            .unwrap()
        };
        paragraph.styled_runs = runs;
        assert!(paragraph.column_count() >= 2);
        renderer.selection.append_paragraph_text(0, text);
        renderer
            .render_vertical_paragraph(
                &paragraph,
                0,
                TextDirection::TbRlV,
                TextAlign::Right,
                20.0,
                30.0,
                100.0,
                0.0,
                24.0,
                0,
                &[],
                false,
            )
            .unwrap();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let lines = renderer.layout_debug_lines();
        assert_eq!(lines.len(), 5);
        assert!((lines[0].top - 38.0).abs() < 1e-9);
        let ops = painter.ops();
        assert!(!ops.iter().any(|op| matches!(op, Op::Rotate(_))));
        let paints: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                Op::FillText(text, x, y) => Some((text, x, y)),
                _ => None,
            })
            .collect();
        assert_eq!(paints.len(), lines.len());
        for ((text, x, y), line) in paints.iter().zip(lines) {
            assert_eq!(text.as_str(), line.text);
            assert!((**x - line.start_x).abs() < 1e-9);
            assert!((**y - line.baseline).abs() < 1e-9);
        }
    }

    #[test]
    fn upright_table_cell_direction_does_not_change_section_rotation() {
        assert!(!direction_draws_rotated_table_visual(
            TextDirection::TbRl,
            true
        ));
        assert!(direction_draws_rotated_table_visual(
            TextDirection::TbRlV,
            true
        ));
        assert!(!direction_draws_rotated_table_visual(
            TextDirection::TbRlV,
            false
        ));
        assert_eq!(direction_rotation_angle(TextDirection::TbRl), 0.0);
        assert_eq!(direction_rotation_angle(TextDirection::TbRlV), FRAC_PI_2);
    }

    #[test]
    fn turned_cell_runs_consume_shared_horizontal_advances() {
        use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let painter = RecordingPainter::with_width_per_char(10.0);
        let text = "Hello";
        let runs = vec![StyledRunRange {
            picture: None,
            start: 0,
            end: text.len(),
            revision: None,
            properties: Some(RunProperties {
                font_family: Some("Yu Mincho".into()),
                font_size: Some(10.5),
                ..Default::default()
            }),
        }];
        let horizontal = layout_paragraph(&painter, text, 1000.0, 24.0, None, None, &runs).unwrap();
        for direction in [TextDirection::TbRl, TextDirection::BtLr] {
            let turned =
                layout_paragraph(&painter, text, 1000.0, 24.0, Some(direction), None, &runs)
                    .unwrap();
            let actual = turned
                .lines
                .iter()
                .flat_map(|line| line.glyphs.iter())
                .map(|g| (g.start, g.end, g.width))
                .collect::<Vec<_>>();
            let expected = horizontal
                .lines
                .iter()
                .flat_map(|line| line.glyphs.iter())
                .map(|g| (g.start, g.end, g.width))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{direction:?}");
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn turned_cell_glyph_paint_follows_each_source_axis() {
        use jade_docx_layout::selection::{SelectionRange, TextPosition};
        use jade_painter::recording::Op;
        for selected in [false, true] {
            for (direction, angle) in [
                (TextDirection::TbRl, FRAC_PI_2),
                (TextDirection::BtLr, -FRAC_PI_2),
            ] {
                let painter = RecordingPainter::with_width_per_char(10.0);
                let mut renderer = CanvasRenderer::with_host(
                    Box::new(super::super::recording_host::RecordingHost {
                        painter: painter.clone(),
                    }),
                    false,
                )
                .unwrap();
                let text = "AB";
                let mut paragraph = layout_paragraph(
                    &painter,
                    text,
                    1000.0,
                    24.0,
                    Some(direction),
                    Some(80.0),
                    &[],
                )
                .unwrap();
                paragraph.styled_runs = vec![];
                renderer.set_use_selection_overlay(false);
                renderer.selection.append_paragraph_text(0, text);
                if selected {
                    renderer.selection.set_selection(Some(SelectionRange {
                        anchor: TextPosition {
                            paragraph: 0,
                            offset: 0,
                        },
                        focus: TextPosition {
                            paragraph: 0,
                            offset: 1,
                        },
                    }));
                }
                painter.clear();
                renderer
                    .render_vertical_paragraph(
                        &paragraph,
                        0,
                        direction,
                        TextAlign::Left,
                        40.0,
                        30.0,
                        100.0,
                        0.0,
                        24.0,
                        0,
                        &[],
                        false,
                    )
                    .unwrap();
                let ops = painter.ops();
                assert!(
                    selected || ops.iter().filter(|op| matches!(op, Op::Save)).count() <= 2,
                    "default-scale column must not save state per glyph"
                );
                assert_eq!(
                    ops.iter()
                        .filter(|op| matches!(op, Op::Rotate(a) if (*a-angle).abs()<1e-9))
                        .count(),
                    1,
                    "{direction:?}"
                );
                assert!(ops.contains(&Op::SetTextBaseline("alphabetic".into())));
                let lines = renderer.layout_debug_lines();
                assert_eq!(lines.len(), 2);
                assert!(lines.iter().all(|line| line.vertical));
                if selected {
                    let mut matrix = [1.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0];
                    let mut stack = Vec::new();
                    let mut rectangles = 0;
                    for op in &ops {
                        match op {
                            Op::Save => stack.push(matrix),
                            Op::Restore => matrix = stack.pop().unwrap(),
                            Op::Translate(x, y) => {
                                matrix[4] += matrix[0] * x + matrix[2] * y;
                                matrix[5] += matrix[1] * x + matrix[3] * y;
                            }
                            Op::Rotate(angle) => {
                                let [a, b, c, d, e, f] = matrix;
                                let (sin, cos) = angle.sin_cos();
                                matrix = [
                                    a * cos + c * sin,
                                    b * cos + d * sin,
                                    c * cos - a * sin,
                                    d * cos - b * sin,
                                    e,
                                    f,
                                ];
                            }
                            Op::FillRect(x, y, width, height) => {
                                assert_eq!(
                                    matrix,
                                    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                                    "highlight must use page coordinates"
                                );
                                assert_eq!(*x, 40.0);
                                assert_eq!(*y, lines[0].top);
                                assert_eq!(*width, 24.0);
                                assert_eq!(*height, lines[0].height);
                                rectangles += 1;
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(rectangles, 1);
                }
            }
        }
    }

    #[test]
    fn lr_tbv_retains_shared_wrap_and_glyph_advances() {
        let painter = RecordingPainter::with_width_per_char(10.0);
        for text in ["Hello世界", "éあ🙂", "あHelloい", "あいうえおか"] {
            for width in [40.0, 1000.0] {
                let horizontal =
                    layout_paragraph(&painter, text, width, 24.0, None, None, &[]).unwrap();
                let turned = layout_paragraph(
                    &painter,
                    text,
                    width,
                    24.0,
                    Some(TextDirection::LrTbV),
                    None,
                    &[],
                )
                .unwrap();
                assert_eq!(turned.lines.len(), horizontal.lines.len());
                for (actual, expected) in turned.lines.iter().zip(horizontal.lines.iter()) {
                    assert_eq!(actual.text, expected.text);
                    let advances = |line: &LineLayout| {
                        line.glyphs
                            .iter()
                            .map(|g| (g.start, g.end, g.x, g.width))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        advances(actual),
                        advances(expected),
                        "{text}, width {width}"
                    );
                    assert_eq!(actual.visual_width, expected.visual_width);
                }
                assert_eq!(turned.max_line_width, horizontal.max_line_width);
                assert_eq!(turned.height, horizontal.height);
            }
        }
    }

    #[test]
    fn lr_tbv_paints_the_shared_baseline_and_centres_a_turned_em() {
        use jade_painter::recording::Op;
        let painter = RecordingPainter::with_width_per_char(10.0);
        let renderer = CanvasRenderer::with_host(
            Box::new(super::super::recording_host::RecordingHost {
                painter: painter.clone(),
            }),
            false,
        )
        .unwrap();
        let text = "Aあ";
        let runs = vec![StyledRunRange {
            picture: None,
            start: 0,
            end: text.len(),
            revision: None,
            properties: Some(RunProperties {
                font_family: Some("Yu Mincho".into()),
                font_size: Some(10.5),
                ..Default::default()
            }),
        }];
        let mut paragraph =
            layout_paragraph(&painter, text, 1000.0, 24.0, None, None, &runs).unwrap();
        paragraph.styled_runs = runs;
        Rc::make_mut(&mut paragraph.lines)[0].word_baseline_offset = Some(16.0);
        let line = &paragraph.lines[0];
        let glyph = &line.glyphs[1];
        let descriptor =
            paragraph_range_font_descriptor(&paragraph, glyph.start, glyph.end, "あ").unwrap();
        let (ascent, _) =
            jade_docx_layout::line_metrics::word_vertical_glyph_metrics_px(&descriptor).unwrap();
        renderer
            .draw_lr_tbv_line(
                &paragraph,
                line,
                CellLinePlacement {
                    start_x: 20.0,
                    top: 30.0,
                    height: 24.0,
                },
                "#000000",
                &[],
            )
            .unwrap();
        let ops = painter.ops();
        assert!(ops.contains(&Op::FillText("A".into(), 20.0, 46.0)));
        assert!(ops.iter().any(|op| matches!(op, Op::Translate(x,y)
            if (*x - (20.0 + glyph.x + ascent)).abs() < 1e-6
                && (*y - (30.0 + (24.0 + glyph.width) * 0.5)).abs() < 1e-6)));
        assert!(ops.contains(&Op::SetTextBaseline("alphabetic".into())));
        assert!(ops.contains(&Op::Rotate(-FRAC_PI_2)));
    }

    #[test]
    fn nested_rows_reserve_horizontal_borders_without_growing_exact_heights() {
        let painter = RecordingPainter::new();
        let context = TableLineWrapContext::new(&painter);
        let config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 100.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            styles_part: None,
        };
        for exact_middle in [false, true] {
            let table = Table {
                rows: (0..3)
                    .map(|index| TableRow {
                        cells: vec![TableCell {
                            style: [
                                ("border-top-style".into(), "single".into()),
                                ("border-top-width".into(), "1px".into()),
                                ("border-bottom-style".into(), "single".into()),
                                ("border-bottom-width".into(), "1px".into()),
                            ]
                            .into_iter()
                            .collect(),
                            ..Default::default()
                        }],
                        height: (exact_middle && index == 1).then_some(15.0),
                        height_rule: (exact_middle && index == 1)
                            .then_some(TableRowHeightRule::Exact),
                        ..Default::default()
                    })
                    .collect(),
                table_layout: Some(TableLayoutMode::Fixed),
                ..Default::default()
            };
            let nested = layout_nested_table(&context, &table, 100.0, &config, &|_| None).unwrap();
            let expected = if exact_middle {
                [22.0, 20.0, 21.0]
            } else {
                [22.0, 21.0, 21.0]
            };
            for (row, height) in nested.rows.iter().zip(expected) {
                assert!(
                    (row.height - height).abs() < 1e-9,
                    "nested row height {} must reserve its 1px border in {height}",
                    row.height
                );
            }
            assert!((nested.height - expected.iter().sum::<f64>()).abs() < 1e-9);
            assert_eq!(nested.rows[0].cells[0].page_top_inset, 1.0);
            assert_eq!(nested.rows[1].cells[0].page_top_inset, 0.0);
        }
    }

    #[test]
    fn exact_rows_inset_text_by_the_resolved_top_border_without_growing() {
        let table = Table {
            rows: (0..2)
                .map(|_| TableRow {
                    height: Some(18.0),
                    height_rule: Some(TableRowHeightRule::Exact),
                    ..TableRow::default()
                })
                .collect(),
            ..Table::default()
        };
        let mut rows = vec![vec![make_prepared_cell(0, 1, None, false)]; 2];
        for row in &mut rows {
            row[0].padding.top = 0.0;
            row[0].border_top.width_px = points_to_px(0.5);
        }
        let mut heights = vec![24.0, 24.0];
        add_border_extents(&table, &mut rows, &[false, false], &mut heights, true);
        assert_eq!(heights, vec![24.0, 24.0]);
        for row in &rows {
            assert!((row[0].padding.top - points_to_px(0.5)).abs() < 1e-9);
        }
    }

    #[test]
    fn exact_row_after_auto_row_does_not_reserve_the_shared_border_twice() {
        let table = Table {
            rows: vec![
                TableRow::default(),
                TableRow {
                    height: Some(18.0),
                    height_rule: Some(TableRowHeightRule::Exact),
                    ..TableRow::default()
                },
            ],
            ..Table::default()
        };
        let mut rows = vec![vec![make_prepared_cell(0, 1, None, false)]; 2];
        let padding = rows[1][0].padding.top;
        let mut heights = vec![24.0, 24.0];
        add_border_extents(&table, &mut rows, &[false, false], &mut heights, true);
        assert_eq!(rows[1][0].padding.top, padding);
        assert_eq!(heights[1], 24.0);
    }

    #[test]
    fn segmented_shared_rule_reserves_the_painted_winner_in_row_height() {
        let mut rows = vec![
            vec![make_prepared_cell(0, 2, None, false)],
            vec![
                make_prepared_cell(0, 1, None, false),
                make_prepared_cell(1, 1, None, false),
            ],
        ];
        for cell in rows.iter_mut().flatten() {
            cell.height = 20.0;
            cell.border_top = CellBorderSpec::hidden();
            cell.border_bottom = CellBorderSpec::hidden();
        }
        let border = |width| CellBorderSpec {
            width_px: width,
            ..CellBorderSpec::single_default()
        };
        rows[0][0].border_bottom = border(1.0);
        rows[1][0].border_top = border(4.0);
        rows[1][1].border_top = border(2.0);
        resolve_table_shared_horizontal_border_conflicts(
            &mut rows,
            &[false, false],
            &[40.0, 60.0],
            false,
        );
        assert_eq!(rows[0][0].border_bottom_segments.len(), 2);
        let mut heights = vec![20.0, 20.0];
        let table = Table {
            rows: vec![TableRow::default(), TableRow::default()],
            ..Table::default()
        };
        add_border_extents(&table, &mut rows, &[false, false], &mut heights, true);
        assert_eq!(heights, [24.0, 20.0]);
        assert_eq!(rows[1][0].page_top_inset, 0.0);
    }

    #[test]
    fn segmented_rules_replace_fallback_and_use_shared_double_geometry() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let fallback = CellBorderSpec {
            width_px: 50.0,
            ..CellBorderSpec::single_default()
        };
        let single = CellBorderSpec {
            width_px: 4.0,
            ..CellBorderSpec::single_default()
        };
        let double = CellBorderSpec {
            kind: BorderKind::Double,
            width_px: 2.0,
            ..CellBorderSpec::single_default()
        };
        let segments = [
            HorizontalBorderSegment {
                start_px: 0.0,
                end_px: 40.0,
                border: single,
            },
            HorizontalBorderSegment {
                start_px: 40.0,
                end_px: 100.0,
                border: double,
            },
        ];
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let extent = horizontal_border_extent(&fallback, &segments);
        set_word_layout_platform(previous);
        assert_eq!(extent, 6.0);
        assert_eq!(horizontal_border_extent(&fallback, &[]), 50.0);
        let hidden = [HorizontalBorderSegment {
            start_px: 0.0,
            end_px: 100.0,
            border: CellBorderSpec::hidden(),
        }];
        assert_eq!(horizontal_border_extent(&fallback, &hidden), 0.0);
        for (start, end) in [(0.0, 0.0), (10.0, 5.0), (-10.0, -5.0), (f64::NAN, 100.0)] {
            let invalid = [HorizontalBorderSegment {
                start_px: start,
                end_px: end,
                border: fallback.clone(),
            }];
            assert_eq!(horizontal_border_extent(&fallback, &invalid), 0.0);
        }
    }

    #[test]
    fn segmented_exact_top_and_split_bottom_share_visible_rule_extents() {
        let mut cell = make_prepared_cell(0, 2, None, false);
        cell.padding = CellPadding {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };
        cell.blocks = vec![make_cell_block(20.0), make_cell_block(20.0)];
        cell.height = 40.0;
        let segment = HorizontalBorderSegment {
            start_px: 0.0,
            end_px: 100.0,
            border: CellBorderSpec {
                width_px: 4.0,
                ..CellBorderSpec::single_default()
            },
        };
        cell.border_top_segments = vec![segment.clone()].into_boxed_slice();
        cell.border_bottom_segments = vec![segment].into_boxed_slice();
        let mut exact_cells = vec![cell.clone()];
        let exact = TableRow {
            height: Some(30.0),
            height_rule: Some(TableRowHeightRule::Exact),
            ..TableRow::default()
        };
        let mut height = 40.0;
        add_row_border_extents(
            &exact,
            &mut exact_cells,
            &mut height,
            &mut true,
            &mut false,
            None,
            None,
        );
        assert_eq!(height, 40.0);
        assert_eq!(exact_cells[0].page_top_inset, 4.0);
        assert!(split_prepared_row_for_available_height(
            std::slice::from_ref(&cell),
            23.0,
            10.0,
            100.0,
            false
        )
        .is_none());
        for has_header in [false, true] {
            let split = split_prepared_row_for_available_height(
                &[cell.clone()],
                24.0,
                10.0,
                100.0,
                has_header,
            )
            .unwrap();
            assert_eq!(split.head_height, 24.0);
            assert_eq!(split.tail_height, if has_header { 24.0 } else { 28.0 });
            assert_eq!(
                split.tail_cells[0].page_top_inset,
                if has_header { 0.0 } else { 4.0 }
            );
        }
    }

    #[test]
    fn unequal_shared_rules_use_one_band_without_changing_uniform_row_storage() {
        let mut upper = make_prepared_cell(0, 2, None, false);
        upper.border_bottom_segments = vec![
            HorizontalBorderSegment {
                start_px: 0.0,
                end_px: 40.0,
                border: CellBorderSpec {
                    width_px: 4.0,
                    ..CellBorderSpec::single_default()
                },
            },
            HorizontalBorderSegment {
                start_px: 40.0,
                end_px: 100.0,
                border: CellBorderSpec {
                    width_px: 3.0,
                    ..CellBorderSpec::single_default()
                },
            },
        ]
        .into_boxed_slice();
        let mut lower = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        lower[0].border_top.width_px = 4.0;
        lower[1].border_top.width_px = 3.0;
        let above = HorizontalBorderBands::for_row(std::slice::from_ref(&upper), true);
        let below = HorizontalBorderBands::for_row(&lower, true);
        assert_eq!(
            (above.top, above.bottom, above.content_bottom),
            (0.0, 4.0, 4.0)
        );
        assert_eq!(
            (below.top, below.bottom, below.content_bottom),
            (4.0, 0.0, 0.0)
        );
        let exact = HorizontalBorderBands::for_row(std::slice::from_ref(&upper), false);
        assert_eq!(exact.content_bottom, 0.0);
        let uniform =
            HorizontalBorderBands::for_row(&[make_prepared_cell(0, 1, None, false)], true);
        assert_eq!(
            (uniform.top, uniform.bottom, uniform.content_bottom),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(std::mem::size_of::<HorizontalBorderBands>(), 24);
    }

    #[test]
    fn mac_aligns_cell_content_above_a_uniform_bottom_rule() {
        // scroll-offset-repro's centred rows under 0.5pt rules: Word for Mac
        // keeps the reserved bottom rule out of the alignment slack.
        use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let mut cells = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        for cell in &mut cells {
            cell.border_bottom.width_px = 2.0 / 3.0;
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = HorizontalBorderBands::for_row(&cells, true);
        let exact = HorizontalBorderBands::for_row(&cells, false);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let windows = HorizontalBorderBands::for_row(&cells, true);
        assert_eq!((mac.bottom, mac.content_bottom), (0.0, 2.0 / 3.0));
        assert_eq!(exact.content_bottom, 0.0);
        assert_eq!(windows.content_bottom, 0.0);
    }

    #[test]
    fn separated_cell_rules_do_not_form_a_shared_horizontal_band() {
        let mut cells = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        for cell in &mut cells {
            cell.cell_spacing = 2.0;
        }
        cells[0].border_top.width_px = 4.0;
        cells[0].border_bottom.width_px = 4.0;
        cells[1].border_top.width_px = 3.0;
        cells[1].border_bottom.width_px = 3.0;
        let bands = HorizontalBorderBands::for_row(&cells, true);
        assert_eq!(
            (bands.top, bands.bottom, bands.content_bottom),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn exact_row_height_adds_largest_cell_bottom_padding() {
        let mut first = make_prepared_cell(0, 1, None, false);
        first.padding.bottom = 5.0;
        let mut second = make_prepared_cell(1, 1, None, false);
        second.padding.bottom = 11.0;

        let resolved = exact_row_height_with_bottom_padding(80.0, &[first, second]);

        assert!((resolved - 91.0).abs() < 0.01);
    }

    fn make_prepared_cell(
        column_index: usize,
        span: usize,
        vertical_merge: Option<&str>,
        skip_render: bool,
    ) -> PreparedCell {
        PreparedCell {
            column_index,
            span,
            vertical_merge: vertical_merge.map(str::to_string),
            border_top: CellBorderSpec::single_default(),
            border_top_segments: Box::default(),
            border_right: CellBorderSpec::single_default(),
            border_bottom: CellBorderSpec::single_default(),
            border_bottom_segments: Box::default(),
            border_left: CellBorderSpec::single_default(),
            border_tl2br: None,
            border_tr2bl: None,
            padding: CellPadding {
                left: 6.0,
                right: 6.0,
                top: 4.0,
                bottom: 4.0,
            },
            cell_spacing: 0.0,
            blocks: Vec::new(),
            height: 0.0,
            background: None,
            text_color: None,
            align: TextAlign::Left,
            text_direction: None,
            vertical_align_share: 0.0,
            skip_render,
            logical_paragraph_count: 0,
            hidden_end_marker_paragraph_count: 0,
            page_top_inset: 0.0,
            selection_paragraph_indices: Vec::new(),
        }
    }

    fn make_cell_block(height: f64) -> CellBlock {
        CellBlock {
            source_index: 0,
            kind: CellBlockKind::Paragraph(empty_cell_paragraph(height)),
            height,
            paragraph_indices: Vec::new(),
        }
    }

    fn make_empty_nested_table_layout() -> NestedTableLayout {
        NestedTableLayout {
            table: Box::new(Table::default()),
            order: None,
            column_widths: Vec::new(),
            column_positions: Vec::new(),
            rows: Vec::new(),
            width: 0.0,
            height: 0.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            line_height: 12.0,
        }
    }

    #[test]
    fn computes_vertical_merge_row_spans_from_restart_and_continue_cells() {
        let rows = [
            vec![
                make_prepared_cell(0, 1, Some("restart"), false),
                make_prepared_cell(1, 1, None, false),
            ],
            vec![
                make_prepared_cell(0, 1, Some("continue"), true),
                make_prepared_cell(1, 1, None, false),
            ],
            vec![
                make_prepared_cell(0, 1, Some("continue"), true),
                make_prepared_cell(1, 1, None, false),
            ],
        ];
        let row_refs: Vec<&[PreparedCell]> = rows.iter().map(Vec::as_slice).collect();
        let spans = compute_vertical_merge_row_spans(&row_refs);

        assert_eq!(spans[0], vec![3, 1]);
        assert_eq!(spans[1], vec![0, 1]);
        assert_eq!(spans[2], vec![0, 1]);
    }

    #[test]
    fn vertical_merge_owner_grows_only_the_last_row_it_spans() {
        // scroll-offset-repro page 6: a double-spaced owner spanning two
        // 17.25pt/15.75pt rows fits them, so Word grows neither; only the
        // part of an owner taller than all its rows lands on the last one.
        let mut heights = vec![20.0, 23.0, 21.0, 30.0];
        grow_rows_for_vertical_merges(&mut heights, &[(1, 2, 36.0)]);
        assert_eq!(heights, vec![20.0, 23.0, 21.0, 30.0]);
        grow_rows_for_vertical_merges(&mut heights, &[(1, 2, 54.0)]);
        assert_eq!(heights, vec![20.0, 23.0, 31.0, 30.0]);
        grow_rows_for_vertical_merges(&mut heights, &[(0, 1, 90.0)]);
        assert_eq!(
            heights,
            vec![20.0, 23.0, 31.0, 30.0],
            "a single-row owner sized its row"
        );
    }

    #[test]
    fn vertical_merge_owner_height_is_left_out_of_its_first_row() {
        let rows = vec![
            vec![
                make_prepared_cell(0, 1, None, false),
                make_prepared_cell(1, 1, Some("restart"), false),
            ],
            vec![
                make_prepared_cell(0, 1, None, false),
                make_prepared_cell(1, 1, Some("continue"), true),
            ],
        ];
        let refs: Vec<&[PreparedCell]> = rows.iter().map(Vec::as_slice).collect();
        let spans = compute_vertical_merge_row_spans(&refs);
        let owners = merged_cell_owners(&refs, &spans);
        assert_eq!(owners, vec![(0, 2, rows[0][1].height)]);
        assert!(spans_merged_owner(&spans[0], 1));
        assert!(!spans_merged_owner(&spans[0], 0));
        // The border pass does not size the first row from the owner either.
        let mut rows = rows;
        rows[0][0].height = 20.0;
        rows[1][0].height = 20.0;
        rows[0][1].height = 60.0;
        let table = Table {
            rows: vec![TableRow::default(), TableRow::default()],
            ..Table::default()
        };
        let owner_height = rows[0][1].height;
        let mut heights = vec![rows[0][0].height, rows[1][0].height];
        add_border_extents(&table, &mut rows, &[false, false], &mut heights, true);
        assert!(
            heights[0] < owner_height,
            "{heights:?} owner {owner_height}"
        );
    }

    #[test]
    fn vertical_merge_does_not_extend_when_next_row_has_mismatched_span() {
        let rows = [
            vec![make_prepared_cell(0, 2, Some("restart"), false)],
            vec![make_prepared_cell(0, 1, Some("continue"), true)],
        ];
        let row_refs: Vec<&[PreparedCell]> = rows.iter().map(Vec::as_slice).collect();
        let spans = compute_vertical_merge_row_spans(&row_refs);

        assert_eq!(spans[0], vec![1]);
        assert_eq!(spans[1], vec![0]);
    }

    #[test]
    fn vertical_merge_requires_contiguous_continue_rows() {
        let rows = [
            vec![make_prepared_cell(0, 1, Some("restart"), false)],
            vec![make_prepared_cell(0, 1, None, false)],
            vec![make_prepared_cell(0, 1, Some("continue"), true)],
        ];
        let row_refs: Vec<&[PreparedCell]> = rows.iter().map(Vec::as_slice).collect();
        let spans = compute_vertical_merge_row_spans(&row_refs);

        assert_eq!(spans[0], vec![1]);
        assert_eq!(spans[1], vec![1]);
        assert_eq!(spans[2], vec![0]);
    }

    #[test]
    fn vertical_merge_continuation_borders_resume_at_page_top_without_inner_bottom() {
        let rows = vec![
            vec![make_prepared_cell(0, 1, Some("restart"), false)],
            vec![make_prepared_cell(0, 1, Some("continue"), true)],
            vec![make_prepared_cell(0, 1, Some("continue"), true)],
        ];

        let first_continuation = vertical_merge_continuation_border_sides(&rows, 1, 0, true, false);
        assert_eq!(
            first_continuation,
            VerticalMergeContinuationBorderSides {
                top: true,
                bottom: false,
            }
        );

        let final_continuation =
            vertical_merge_continuation_border_sides(&rows, 2, 0, false, false);
        assert_eq!(
            final_continuation,
            VerticalMergeContinuationBorderSides {
                top: false,
                bottom: true,
            }
        );

        let page_bottom_continuation =
            vertical_merge_continuation_border_sides(&rows, 1, 0, false, true);
        assert_eq!(
            page_bottom_continuation,
            VerticalMergeContinuationBorderSides {
                top: false,
                bottom: true,
            }
        );
    }

    #[test]
    fn page_clipped_cell_segment_limits_visible_height_to_page_bottom() {
        assert_eq!(
            page_clipped_cell_segment(90.0, 40.0, Some(120.0)),
            PageClippedCellSegment {
                height: 30.0,
                clipped_at_bottom: true,
            }
        );
        assert_eq!(
            page_clipped_cell_segment(60.0, 40.0, Some(120.0)),
            PageClippedCellSegment {
                height: 40.0,
                clipped_at_bottom: false,
            }
        );
        assert_eq!(
            page_clipped_cell_segment(90.0, 40.0, None),
            PageClippedCellSegment {
                height: 40.0,
                clipped_at_bottom: false,
            }
        );
    }

    #[test]
    fn page_clipped_vertical_merge_segment_stops_at_last_complete_row() {
        assert_eq!(
            page_clipped_vertical_merge_segment(90.0, &[20.0, 20.0], Some(125.0)),
            PageClippedCellSegment {
                height: 20.0,
                clipped_at_bottom: true,
            }
        );
        assert_eq!(
            page_clipped_vertical_merge_segment(60.0, &[20.0, 20.0], Some(120.0)),
            PageClippedCellSegment {
                height: 40.0,
                clipped_at_bottom: false,
            }
        );
        assert_eq!(
            page_clipped_vertical_merge_segment(90.0, &[40.0], Some(120.0)),
            PageClippedCellSegment {
                height: 30.0,
                clipped_at_bottom: true,
            }
        );
    }

    #[test]
    fn parse_border_kind_recognizes_ooxml_styles() {
        let cases: &[(&str, BorderKind)] = &[
            ("none", BorderKind::None),
            ("nil", BorderKind::None),
            ("hidden", BorderKind::None),
            ("single", BorderKind::Single),
            // Older revisions of the parser wrote "solid" instead of the
            // OOXML token; make sure we still accept it.
            ("solid", BorderKind::Single),
            ("thick", BorderKind::Thick),
            ("double", BorderKind::Double),
            ("triple", BorderKind::Triple),
            ("dashed", BorderKind::Dashed),
            ("dashSmallGap", BorderKind::DashSmallGap),
            ("dotted", BorderKind::Dotted),
            ("dotDash", BorderKind::DotDash),
            ("dotDotDash", BorderKind::DotDotDash),
            ("wave", BorderKind::Wave),
            ("doubleWave", BorderKind::Wave),
            ("unknown-token", BorderKind::Single),
        ];
        for (input, expected) in cases {
            let value = String::from(*input);
            assert_eq!(
                parse_border_kind(Some(&value)),
                *expected,
                "unexpected kind for `{input}`"
            );
        }
        assert_eq!(parse_border_kind(None), BorderKind::Single);
    }

    #[test]
    fn resolve_border_spec_applies_width_color_and_hidden_fallback() {
        // None-style short-circuits to the pale guide color regardless of the
        // width/color keys — hidden borders must never paint in the real color.
        let none_style = String::from("none");
        let width = String::from("4pt");
        let red = String::from("#FF0000");
        let hidden = resolve_border_spec(Some(&none_style), Some(&width), Some(&red));
        assert_eq!(hidden.kind, BorderKind::None);
        assert_eq!(hidden.color, HIDDEN_BORDER_COLOR);

        // Width is parsed via `parse_length_to_px`; "3pt" → 4px at 96 DPI.
        let single_style = String::from("single");
        let three_pt = String::from("3pt");
        let spec = resolve_border_spec(Some(&single_style), Some(&three_pt), Some(&red));
        assert_eq!(spec.kind, BorderKind::Single);
        assert_eq!(spec.color, "#FF0000");
        assert!((spec.width_px - points_to_px(3.0)).abs() < 0.01);

        // Missing keys fall back to 1 px and the default border color.
        let fallback = resolve_border_spec(None, None, None);
        assert_eq!(fallback.kind, BorderKind::Single);
        assert_eq!(fallback.color, BORDER_COLOR);
        assert!((fallback.width_px - 1.0).abs() < f64::EPSILON);

        // Zero or non-finite widths are rejected so we never call
        // `set_line_width(0)` which would silently draw nothing on Canvas.
        let zero = String::from("0pt");
        let zero_spec = resolve_border_spec(Some(&single_style), Some(&zero), None);
        assert!((zero_spec.width_px - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn dark_mode_remaps_default_black_table_borders() {
        for color in [
            BORDER_COLOR,
            "000000",
            "black",
            " Black ",
            "auto",
            " AUTO ",
            "rgb(0,0,0)",
            "rgb(0, 0, 0)",
        ] {
            assert_eq!(
                resolve_table_border_stroke_color(color, true).as_ref(),
                DARK_MODE_BORDER_COLOR,
                "default border variant `{color}` should remain visible on a dark page"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        /// Light preview is document-faithful, while every parseable dark-mode
        /// border gets enough contrast for a one-pixel table grid. This is a
        /// presentation-only transform; the source model is not mutated.
        #[test]
        fn table_border_theme_preserves_light_color_and_enforces_dark_contrast(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let color = format!("#{red:02x}{green:02x}{blue:02x}");
            let light = resolve_table_border_stroke_color(&color, false);
            let dark = resolve_table_border_stroke_color(&color, true);

            prop_assert_eq!(light.as_ref(), color.as_str());
            let rendered = parse_table_border_rgb(dark.as_ref()).expect("rendered hex color");
            prop_assert!(
                color_contrast_ratio(rendered, DARK_MODE_PAGE_COLOR)
                    >= DARK_MODE_BORDER_MIN_CONTRAST,
                "source={color}, rendered={dark}"
            );
        }

        #[test]
        fn dark_table_hairlines_keep_a_visible_css_pixel(
            authored_width in 0.0_f64..8.0,
        ) {
            let light = resolve_table_border_stroke_width(authored_width, false);
            let dark = resolve_table_border_stroke_width(authored_width, true);

            prop_assert!((light - authored_width.max(1.0)).abs() < f64::EPSILON);
            prop_assert!((dark - authored_width.max(1.0)).abs() < f64::EPSILON);
            prop_assert_eq!(dark, light);
        }
    }

    #[test]
    fn hidden_border_editing_guides_are_theme_aware() {
        assert_eq!(
            resolve_hidden_table_border_stroke_color(false),
            HIDDEN_BORDER_COLOR
        );
        assert_eq!(
            resolve_hidden_table_border_stroke_color(true),
            DARK_MODE_HIDDEN_BORDER_COLOR
        );
        let dark = parse_table_border_rgb(DARK_MODE_HIDDEN_BORDER_COLOR)
            .expect("dark hidden-border guide must be a six-digit hex color");
        assert!(color_contrast_ratio(dark, DARK_MODE_PAGE_COLOR) >= DARK_MODE_BORDER_MIN_CONTRAST);
    }

    #[test]
    fn dark_mode_lightens_low_contrast_authored_borders_without_touching_light_mode() {
        for color in ["#111827", "#262626", "#3b3f8c", "4F50A5", "#595959"] {
            assert_eq!(resolve_table_border_stroke_color(color, false), color);
            let rendered = resolve_table_border_stroke_color(color, true);
            assert_ne!(rendered.as_ref(), color);
            assert!(
                color_contrast_ratio(
                    parse_table_border_rgb(rendered.as_ref()).expect("rendered hex color"),
                    DARK_MODE_PAGE_COLOR,
                ) >= DARK_MODE_BORDER_MIN_CONTRAST,
                "source={color}, rendered={rendered}"
            );
        }
    }

    #[test]
    fn dark_mode_keeps_high_contrast_and_unknown_css_colors_unchanged() {
        for color in ["#ffffff", "#cbd5e1", "rebeccapurple"] {
            assert_eq!(resolve_table_border_stroke_color(color, true), color);
        }
    }

    #[test]
    fn shared_border_conflict_keeps_visible_edge_against_hidden_neighbor() {
        // MS-OE376 §2.4.63 differs from simply painting cells in order: when
        // either side of one shared edge is nil/none, Word displays the
        // opposing border. A hidden left edge from the later-painted cell must
        // therefore not erase the visible right edge of its neighbor.
        let visible = CellBorderSpec {
            kind: BorderKind::Double,
            conflict_number: 3,
            conflict_precedence: 3,
            width_px: 3.0,
            color: "#C00000".to_string(),
        };
        let hidden = CellBorderSpec::hidden();

        assert_eq!(resolve_shared_border_conflict(&visible, &hidden), visible);
        assert_eq!(resolve_shared_border_conflict(&hidden, &visible), visible);

        let mut cells = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        cells[0].border_right = visible.clone();
        cells[1].border_left = hidden;

        resolve_row_shared_vertical_border_conflicts(&mut cells, false, 0.0);

        assert_eq!(cells[0].border_right, visible);
        assert_eq!(cells[1].border_left, visible);

        // bidiVisual mirrors the two logical cells, so their physical shared
        // edge is first.left against second.right. Conflict resolution must
        // follow that physical mapping without reordering the document model.
        let mut bidi_cells = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        bidi_cells[0].border_left = visible.clone();
        bidi_cells[1].border_right = CellBorderSpec::hidden();
        resolve_row_shared_vertical_border_conflicts(&mut bidi_cells, true, 0.0);
        assert_eq!(bidi_cells[0].border_left, visible);
        assert_eq!(bidi_cells[1].border_right, visible);

        // Nonzero cell spacing gives the two borders separate physical edges;
        // they should both remain available for painting instead of collapsing
        // into one shared winner.
        let mut spaced_cells = vec![
            make_prepared_cell(0, 1, None, false),
            make_prepared_cell(1, 1, None, false),
        ];
        spaced_cells[0].border_right = visible;
        spaced_cells[1].border_left = CellBorderSpec::hidden();
        resolve_row_shared_vertical_border_conflicts(&mut spaced_cells, false, 2.0);
        assert_eq!(spaced_cells[1].border_left.kind, BorderKind::None);
    }

    #[test]
    fn shared_border_conflict_uses_word_weight_style_and_color_order() {
        // Weight is `w:sz` (eighths of a point) × the style number. A 4pt
        // single therefore beats a 1pt double (32×1 > 8×3), regardless of
        // which neighboring cell is painted second.
        let single = resolve_border_spec(
            Some(&"single".to_string()),
            Some(&"4pt".to_string()),
            Some(&"#000000".to_string()),
        );
        let double = resolve_border_spec(
            Some(&"double".to_string()),
            Some(&"1pt".to_string()),
            Some(&"#000000".to_string()),
        );
        assert_eq!(resolve_shared_border_conflict(&single, &double), single);
        assert_eq!(resolve_shared_border_conflict(&double, &single), single);

        // Equal weight falls through to Word's style precedence. Here 3pt
        // single and 1pt double both weigh 24; double ranks later and wins.
        let equal_single = resolve_border_spec(
            Some(&"single".to_string()),
            Some(&"3pt".to_string()),
            Some(&"#000000".to_string()),
        );
        assert_eq!(
            resolve_shared_border_conflict(&equal_single, &double),
            double
        );
        assert_eq!(
            resolve_shared_border_conflict(&double, &equal_single),
            double
        );

        // Dotted and dashed borders have fixed weight 1 in Word, even when
        // their declared width is very large.
        let dashed = resolve_border_spec(
            Some(&"dashed".to_string()),
            Some(&"20pt".to_string()),
            Some(&"#000000".to_string()),
        );
        let thin_single = resolve_border_spec(
            Some(&"single".to_string()),
            Some(&"0.5pt".to_string()),
            Some(&"#000000".to_string()),
        );
        assert_eq!(
            resolve_shared_border_conflict(&dashed, &thin_single),
            thin_single
        );
        assert_eq!(
            resolve_shared_border_conflict(&thin_single, &dashed),
            thin_single
        );

        // Identical style/width uses Word's three-stage color brightness
        // comparison. Red and blue tie at R+B+2G, then red wins at B+2G.
        let red = resolve_border_spec(
            Some(&"single".to_string()),
            Some(&"1pt".to_string()),
            Some(&"#FF0000".to_string()),
        );
        let blue = resolve_border_spec(
            Some(&"single".to_string()),
            Some(&"1pt".to_string()),
            Some(&"#0000FF".to_string()),
        );
        assert_eq!(resolve_shared_border_conflict(&red, &blue), red);
        assert_eq!(resolve_shared_border_conflict(&blue, &red), red);
    }

    #[test]
    fn horizontal_shared_border_conflict_handles_aligned_and_segmented_rows() {
        // A row boundary is painted twice today: the upper cell's bottom edge,
        // followed by the lower cell's top edge. Word instead selects one
        // winner with the same conflict procedure used for vertical neighbors.
        let visible = resolve_border_spec(
            Some(&"double".to_string()),
            Some(&"1pt".to_string()),
            Some(&"#C00000".to_string()),
        );
        let hidden = CellBorderSpec::hidden();
        let mut rows = vec![
            vec![make_prepared_cell(0, 1, None, false)],
            vec![make_prepared_cell(0, 1, None, false)],
        ];
        rows[0][0].border_bottom = visible.clone();
        rows[1][0].border_top = hidden;

        resolve_table_shared_horizontal_border_conflicts(
            &mut rows,
            &[false, false],
            &[100.0],
            false,
        );

        assert_eq!(rows[0][0].border_bottom, visible);
        assert_eq!(rows[1][0].border_top, visible);

        // Nonzero cell spacing creates distinct physical edges, so those
        // borders must remain independent.
        let mut spaced = rows.clone();
        spaced[1][0].cell_spacing = 2.0;
        spaced[1][0].border_top = CellBorderSpec::hidden();
        resolve_table_shared_horizontal_border_conflicts(
            &mut spaced,
            &[false, false],
            &[100.0],
            false,
        );
        assert_eq!(spaced[1][0].border_top.kind, BorderKind::None);

        let border = |style: &str, width: &str, color: &str| {
            resolve_border_spec(
                Some(&style.to_string()),
                Some(&width.to_string()),
                Some(&color.to_string()),
            )
        };
        // A spanning upper cell can face different winners in the two lower
        // cells. Use unequal column widths to prove offsets follow grid geometry.
        let mut mismatched = vec![
            vec![make_prepared_cell(0, 2, None, false)],
            vec![
                make_prepared_cell(0, 1, None, false),
                make_prepared_cell(1, 1, None, false),
            ],
        ];
        mismatched[0][0].border_bottom = border("single", "1pt", "#4472C4");
        mismatched[1][0].border_top = border("single", "4pt", "#00B050");
        mismatched[1][1].border_top = border("double", "1pt", "#7030A0");
        resolve_table_shared_horizontal_border_conflicts(
            &mut mismatched,
            &[false, false],
            &[40.0, 60.0],
            false,
        );
        let segments = &mismatched[0][0].border_bottom_segments;
        assert_eq!(segments.len(), 2);
        assert_eq!((segments[0].start_px, segments[0].end_px), (0.0, 40.0));
        assert_eq!(segments[0].border.color, "#00B050");
        assert_eq!((segments[1].start_px, segments[1].end_px), (40.0, 100.0));
        assert_eq!(segments[1].border.color, "#7030A0");

        // bidiVisual mirrors physical segment order inside the spanning cell.
        let mut bidi = mismatched.clone();
        bidi[0][0].border_bottom_segments = Box::default();
        resolve_table_shared_horizontal_border_conflicts(
            &mut bidi,
            &[false, false],
            &[40.0, 60.0],
            true,
        );
        let bidi_segments = &bidi[0][0].border_bottom_segments;
        assert_eq!(
            (bidi_segments[0].start_px, bidi_segments[0].end_px),
            (0.0, 60.0)
        );
        assert_eq!(bidi_segments[0].border.color, "#7030A0");
        assert_eq!(
            (bidi_segments[1].start_px, bidi_segments[1].end_px),
            (60.0, 100.0)
        );
        assert_eq!(bidi_segments[1].border.color, "#00B050");

        // The inverse topology stores both winners on the spanning lower cell.
        let mut inverse = vec![
            vec![
                make_prepared_cell(0, 1, None, false),
                make_prepared_cell(1, 1, None, false),
            ],
            vec![make_prepared_cell(0, 2, None, false)],
        ];
        inverse[0][0].border_bottom = border("single", "4pt", "#00B050");
        inverse[0][1].border_bottom = border("double", "1pt", "#7030A0");
        inverse[1][0].border_top = border("single", "1pt", "#4472C4");
        resolve_table_shared_horizontal_border_conflicts(
            &mut inverse,
            &[false, false],
            &[40.0, 60.0],
            false,
        );
        let inverse_segments = &inverse[1][0].border_top_segments;
        assert_eq!(inverse_segments.len(), 2);
        assert_eq!(inverse_segments[0].border.color, "#00B050");
        assert_eq!(inverse_segments[1].border.color, "#7030A0");
    }

    #[test]
    fn border_edge_to_spec_maps_table_level_edges() {
        use crate::document::BorderEdge;

        // `w:sz` is in eighths of a point: 24 → 3pt. Color is taken verbatim.
        let visible = BorderEdge {
            val: Some("double".to_string()),
            size_eighths: Some(24),
            space: None,
            color: Some("#FF0000".into()),
        };
        let spec = border_edge_to_spec(&visible);
        assert_eq!(spec.kind, BorderKind::Double);
        assert_eq!(spec.color, "#FF0000");
        assert!((spec.width_px - points_to_px(3.0)).abs() < 0.01);

        // `nil` collapses to the hidden guide spec regardless of size/color.
        let hidden = BorderEdge {
            val: Some("nil".to_string()),
            size_eighths: Some(24),
            space: None,
            color: Some("#FF0000".into()),
        };
        let hidden_spec = border_edge_to_spec(&hidden);
        assert_eq!(hidden_spec.kind, BorderKind::None);
        assert_eq!(hidden_spec.color, HIDDEN_BORDER_COLOR);

        // A bare edge (no size/color) defaults to a 1px single stroke.
        let bare = BorderEdge::default();
        let bare_spec = border_edge_to_spec(&bare);
        assert_eq!(bare_spec.kind, BorderKind::Single);
        assert_eq!(bare_spec.color, BORDER_COLOR);
        assert!((bare_spec.width_px - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn table_border_edge_for_side_picks_outer_vs_interior() {
        use crate::document::{BorderEdge, TableBorders};

        let edge = |val: &str| BorderEdge {
            val: Some(val.to_string()),
            size_eighths: None,
            space: None,
            color: None,
        };
        let borders = TableBorders {
            top: Some(edge("top")),
            bottom: Some(edge("bottom")),
            left: Some(edge("left")),
            right: Some(edge("right")),
            start: None,
            end: None,
            inside_h: Some(edge("insideH")),
            inside_v: Some(edge("insideV")),
        };

        // A top-left corner cell in a single-cell table uses every outer edge.
        let val = |side| {
            table_border_edge_for_side(&borders, side, false, true, true, true, true)
                .and_then(|e| e.val.clone())
        };
        assert_eq!(val(TableBorderSide::Top).as_deref(), Some("top"));
        assert_eq!(val(TableBorderSide::Bottom).as_deref(), Some("bottom"));
        assert_eq!(val(TableBorderSide::Left).as_deref(), Some("left"));
        assert_eq!(val(TableBorderSide::Right).as_deref(), Some("right"));

        // An interior cell (not on any boundary) uses the grid lines instead.
        let inner = |side| {
            table_border_edge_for_side(&borders, side, false, false, false, false, false)
                .and_then(|e| e.val.clone())
        };
        assert_eq!(inner(TableBorderSide::Top).as_deref(), Some("insideH"));
        assert_eq!(inner(TableBorderSide::Bottom).as_deref(), Some("insideH"));
        assert_eq!(inner(TableBorderSide::Left).as_deref(), Some("insideV"));
        assert_eq!(inner(TableBorderSide::Right).as_deref(), Some("insideV"));
    }

    #[test]
    fn logical_table_and_cell_borders_follow_bidi_visual_direction() {
        use crate::document::{BorderEdge, TableBorders};

        let edge = |color: &str| BorderEdge {
            val: Some("single".to_string()),
            size_eighths: Some(8),
            space: None,
            color: Some(color.to_string()),
        };
        let borders = TableBorders {
            left: Some(edge("#LEGACY-L")),
            right: Some(edge("#LEGACY-R")),
            start: Some(edge("#START")),
            end: Some(edge("#END")),
            ..TableBorders::default()
        };
        assert_eq!(
            borders
                .physical_left(false)
                .and_then(|edge| edge.color.as_deref()),
            Some("#START")
        );
        assert_eq!(
            borders
                .physical_right(false)
                .and_then(|edge| edge.color.as_deref()),
            Some("#END")
        );
        assert_eq!(
            borders
                .physical_left(true)
                .and_then(|edge| edge.color.as_deref()),
            Some("#END")
        );
        assert_eq!(
            borders
                .physical_right(true)
                .and_then(|edge| edge.color.as_deref()),
            Some("#START")
        );

        let mut cell = TableCell::default();
        cell.style
            .insert("border-start-color".to_string(), "#CELL-START".to_string());
        cell.style
            .insert("border-end-color".to_string(), "#CELL-END".to_string());
        let ltr = Table::default();
        assert_eq!(
            cell_border_property(&cell, &ltr, "left", "color").map(String::as_str),
            Some("#CELL-START")
        );
        let rtl = Table {
            bidi_visual: true,
            ..Table::default()
        };
        assert_eq!(
            cell_border_property(&cell, &rtl, "left", "color").map(String::as_str),
            Some("#CELL-END")
        );
    }

    #[test]
    fn border_style_entry_reads_the_same_keys_as_formatting_them() {
        let long_side = "x".repeat(60);
        let mut style = crate::document::StyleMap::new();
        style.insert("border-top-style".to_string(), "single".to_string());
        style.insert("border-tl2br-width".to_string(), "8".to_string());
        style.insert(format!("border-{long_side}-color"), "#ABCDEF".to_string());

        for (side, property) in [
            ("top", "style"),
            ("tl2br", "width"),
            (long_side.as_str(), "color"),
            ("top", "color"),
            ("to", "p-style"),
        ] {
            assert_eq!(
                border_style_entry(&style, side, property),
                style.get(&format!("border-{side}-{property}")),
                "border-{side}-{property}"
            );
        }
        assert_eq!(
            border_style_entry(&style, &long_side, "color").map(String::as_str),
            Some("#ABCDEF")
        );
    }

    #[test]
    fn diagonal_cell_borders_are_optional_and_keep_their_stroke_spec() {
        let table = Table::default();
        let empty = TableCell::default();
        assert!(optional_cell_border_spec(&empty, &table, "tl2br").is_none());

        let mut cell = TableCell::default();
        cell.style
            .insert("border-tl2br-style".to_string(), "double".to_string());
        cell.style
            .insert("border-tl2br-width".to_string(), "2pt".to_string());
        cell.style
            .insert("border-tl2br-color".to_string(), "#C00000".to_string());
        cell.style
            .insert("border-tr2bl-style".to_string(), "none".to_string());

        let descending = optional_cell_border_spec(&cell, &table, "tl2br")
            .expect("explicit descending diagonal");
        assert_eq!(descending.kind, BorderKind::Double);
        assert!((descending.width_px - points_to_px(2.0)).abs() < 0.01);
        assert_eq!(descending.color, "#C00000");

        let hidden = optional_cell_border_spec(&cell, &table, "tr2bl")
            .expect("explicitly hidden ascending diagonal");
        assert_eq!(hidden.kind, BorderKind::None);
    }

    #[test]
    fn row_table_border_exception_replaces_table_border_set() {
        use crate::document::{BorderEdge, TableBorders};

        let inside_v = |color: &str| TableBorders {
            inside_v: Some(BorderEdge {
                val: Some("single".to_string()),
                size_eighths: Some(8),
                space: None,
                color: Some(color.to_string()),
            }),
            ..TableBorders::default()
        };
        let table = Table {
            borders: Some(inside_v("#4472C4")),
            ..Table::default()
        };
        let inherited = TableRow::default();
        let excepted = TableRow {
            borders: Some(Box::new(inside_v("#C00000"))),
            ..TableRow::default()
        };

        let edge = |row: &TableRow| {
            effective_table_border_edge_for_side(
                row,
                &table,
                TableBorderSide::Left,
                false,
                false,
                false,
                false,
            )
            .and_then(|edge| edge.color.clone())
        };
        assert_eq!(edge(&inherited).as_deref(), Some("#4472C4"));
        assert_eq!(edge(&excepted).as_deref(), Some("#C00000"));
    }

    #[test]
    fn cant_split_blocks_row_split_only_when_row_fits_a_page() {
        // Splittable rows are never blocked.
        assert!(!cant_split_blocks_row_split(false, 100.0, 800.0));

        // A cantSplit row shorter than a full page is protected (kept whole).
        assert!(cant_split_blocks_row_split(true, 100.0, 800.0));

        // Exactly page-tall is still allowed to stay whole.
        assert!(cant_split_blocks_row_split(true, 800.0, 800.0));

        // A cantSplit row taller than a full page cannot be honored: it must
        // still be split/clipped rather than pushed onto endless blank pages.
        assert!(!cant_split_blocks_row_split(true, 900.0, 800.0));
    }

    fn header_row(is_header: bool) -> TableRow {
        TableRow {
            is_header,
            ..Default::default()
        }
    }

    #[test]
    fn leading_header_row_count_counts_only_the_contiguous_top_block() {
        // No header rows at all.
        assert_eq!(
            leading_header_row_count(&[header_row(false), header_row(false)]),
            0
        );

        // A single leading header row.
        assert_eq!(
            leading_header_row_count(&[header_row(true), header_row(false)]),
            1
        );

        // Multiple contiguous leading header rows.
        assert_eq!(
            leading_header_row_count(&[header_row(true), header_row(true), header_row(false)]),
            2
        );

        // A header flag that appears *after* a body row does not extend the
        // block — Word only repeats the contiguous top block.
        assert_eq!(
            leading_header_row_count(&[header_row(true), header_row(false), header_row(true)]),
            1
        );

        // Empty table → no header rows.
        assert_eq!(leading_header_row_count(&[]), 0);

        // A hidden first row has no visual header box to repeat, even if the
        // row also carries tblHeader. A later header cannot restart the block.
        let hidden_header = TableRow {
            is_header: true,
            style: [("hidden".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            leading_header_row_count(&[hidden_header, header_row(true)]),
            0
        );
    }

    #[test]
    fn hidden_row_marker_preserves_visible_content_and_paragraph_marks() {
        let paragraph: Paragraph = serde_json::from_value(serde_json::json!({
            "runs": [{"text": "visible"}],
            "properties": {"mark_run_properties": {"vanish": true}}
        }))
        .unwrap();
        let mut row = TableRow {
            style: [("hidden".into(), "true".into())].into_iter().collect(),
            cells: vec![TableCell {
                blocks: vec![BodyElement::from_paragraph(paragraph)],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            !table_row_is_hidden(&row),
            "the marker cannot hide visible text"
        );
        row.is_header = true;
        assert_eq!(leading_header_row_count(&[row.clone()]), 1);
        let paragraph = row.cells[0].blocks[0].as_paragraph_mut().unwrap();
        paragraph.runs[0].properties = Some(RunProperties {
            vanish: Some(true),
            ..Default::default()
        });
        assert!(table_row_is_hidden(&row), "all text and markers are hidden");
        assert_eq!(leading_header_row_count(&[row.clone()]), 0);
        row.cells[0].blocks[0]
            .as_paragraph_mut()
            .unwrap()
            .properties = None;
        assert!(
            !table_row_is_hidden(&row),
            "a visible paragraph mark keeps the row"
        );
    }

    #[test]
    fn hidden_row_marker_preserves_nested_visible_content_and_fields() {
        let mut paragraph = paragraph_with_field(RunFieldKind::PageNumber, "");
        paragraph.properties = Some(ParagraphProperties {
            mark_run_properties: Some(RunProperties {
                vanish: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut row = TableRow {
            style: [("hidden".into(), "true".into())].into_iter().collect(),
            cells: vec![TableCell {
                blocks: vec![BodyElement::from_paragraph(paragraph)],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            !table_row_is_hidden(&row),
            "a dynamic field remains visible"
        );
        row.cells[0].blocks[0].as_paragraph_mut().unwrap().runs[0]
            .properties
            .as_mut()
            .unwrap()
            .spec_vanish = Some(true);
        assert!(table_row_is_hidden(&row));
        let mut parent = row.clone();
        parent.cells[0].blocks = vec![BodyElement::from_table(Table {
            rows: vec![row],
            ..Default::default()
        })];
        assert!(table_row_is_hidden(&parent));
        parent.cells[0].blocks[0].as_table_mut().unwrap().rows[0]
            .style
            .clear();
        assert!(
            !table_row_is_hidden(&parent),
            "nested visible rows keep their parent"
        );
    }

    #[test]
    fn hidden_table_row_marker_honors_explicit_false() {
        let hidden = TableRow {
            style: [("hidden".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let visible = TableRow {
            style: [("hidden".to_string(), "false".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        assert!(table_row_is_hidden(&hidden));
        assert!(!table_row_is_hidden(&visible));
        assert!(!table_row_is_hidden(&TableRow::default()));
    }

    #[test]
    fn repeated_header_height_sums_the_leading_rows_and_clamps() {
        let heights = [20.0, 30.0, 40.0];

        // No header rows reserve no space.
        assert_eq!(repeated_header_height(&heights, 0), 0.0);

        // One/two header rows sum their heights.
        assert_eq!(repeated_header_height(&heights, 1), 20.0);
        assert_eq!(repeated_header_height(&heights, 2), 50.0);

        // A count larger than the row list is clamped (no out-of-bounds panic).
        assert_eq!(repeated_header_height(&heights, 9), 90.0);

        // Empty heights → zero.
        assert_eq!(repeated_header_height(&[], 3), 0.0);
    }

    #[test]
    fn should_repeat_header_only_for_body_rows_after_a_page_advance() {
        // Tables without header rows never repeat anything.
        assert!(!should_repeat_header_before_row(0, 5, true));

        // The leading header rows themselves are never preceded by a repeat,
        // even if a page advance happens to land on them.
        assert!(!should_repeat_header_before_row(2, 0, true));
        assert!(!should_repeat_header_before_row(2, 1, true));

        // A body row (index >= header count) that starts a new page repeats.
        assert!(should_repeat_header_before_row(2, 2, true));
        assert!(should_repeat_header_before_row(2, 7, true));

        // Same body row without a page boundary does not repeat.
        assert!(!should_repeat_header_before_row(2, 7, false));
    }

    #[test]
    fn bidi_and_partial_grid_keeps_content_margins_independent_of_visual_order() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let context = TableLineWrapContext::new(&painter);
        let config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 200.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            styles_part: None,
        };
        let row = TableRow {
            cells: vec![TableCell::default()],
            ..Default::default()
        };
        for bidi_visual in [false, true] {
            let table = Table {
                bidi_visual,
                cell_margins: Some(TableCellMargins {
                    start: Some(18.0),
                    end: Some(3.6),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let cells = prepare_row_layout(
                &context,
                &row,
                &table,
                &[200.0],
                &config,
                None,
                0,
                1,
                &mut NaturalParagraphLayoutCache::default(),
                None,
                &|_| None,
            )
            .unwrap();
            assert_eq!(cells[0].padding.left, points_to_px(18.0));
            assert!((cells[0].padding.right - points_to_px(3.6)).abs() < 1e-6);
        }
    }

    #[test]
    fn bidi_and_partial_grid_inherits_each_missing_edge_without_overriding_nil() {
        let edge = |val: &str, size| crate::document::BorderEdge {
            val: Some(val.into()),
            size_eighths: Some(size),
            ..Default::default()
        };
        let mut table = Table {
            borders: Some(crate::document::TableBorders {
                inside_v: Some(edge("single", 8)),
                ..Default::default()
            }),
            resolved_style: Some(crate::document::TableStyleDisplay {
                borders: Some(crate::document::TableBorders {
                    top: Some(edge("single", 4)),
                    bottom: Some(edge("single", 4)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let row = TableRow {
            borders: Some(Box::new(crate::document::TableBorders {
                right: Some(edge("nil", 0)),
                ..Default::default()
            })),
            ..Default::default()
        };
        let pick = |table: &Table, side, outer| {
            effective_table_border_edge_for_side(&row, table, side, outer, outer, outer, outer)
                .map(|e| e.size_eighths)
        };
        assert_eq!(pick(&table, TableBorderSide::Top, true), Some(Some(4)));
        assert_eq!(pick(&table, TableBorderSide::Right, false), Some(Some(8)));
        assert_eq!(pick(&table, TableBorderSide::Right, true), Some(Some(0)));
        table.borders.as_mut().unwrap().top = Some(edge("nil", 0));
        assert_eq!(pick(&table, TableBorderSide::Top, true), Some(Some(0)));
    }

    #[test]
    fn resolve_cell_padding_prefers_cell_then_row_then_table_then_default() {
        use crate::document::TableCellMargins;

        let default_x = points_to_px(CELL_HORIZONTAL_PADDING_PT);
        let default_y = points_to_px(CELL_VERTICAL_PADDING_PT);

        // No margins anywhere → renderer defaults on every side.
        let plain = resolve_cell_padding(None, None, None, false, default_x, default_y);
        assert_eq!(
            plain,
            CellPadding {
                left: default_x,
                right: default_x,
                top: default_y,
                bottom: default_y,
            }
        );

        // Table-level margins apply when the cell defines none.
        let table_margins = TableCellMargins {
            top: Some(2.0),
            bottom: Some(2.0),
            left: Some(5.0),
            right: Some(5.0),
            ..TableCellMargins::default()
        };
        let from_table = resolve_cell_padding(
            None,
            None,
            Some(&table_margins),
            false,
            default_x,
            default_y,
        );
        assert!((from_table.left - points_to_px(5.0)).abs() < 0.01);
        assert!((from_table.top - points_to_px(2.0)).abs() < 0.01);

        // A tblPrEx margin applies only to its row and overrides the table
        // default per edge. Unspecified edges continue to inherit normally.
        let row_margins = TableCellMargins {
            top: Some(7.0),
            start: Some(9.0),
            ..TableCellMargins::default()
        };
        let from_row = resolve_cell_padding(
            None,
            Some(&row_margins),
            Some(&table_margins),
            false,
            default_x,
            default_y,
        );
        assert!((from_row.left - points_to_px(9.0)).abs() < 0.01);
        assert!((from_row.right - points_to_px(5.0)).abs() < 0.01);
        assert!((from_row.top - points_to_px(7.0)).abs() < 0.01);

        // Cell-level margins win per side; unspecified sides fall back to the
        // table value (here `bottom`), and missing-everywhere uses the default.
        let cell_margins = TableCellMargins {
            top: Some(10.0),
            bottom: None,
            left: Some(0.0),
            right: None,
            ..TableCellMargins::default()
        };
        let resolved = resolve_cell_padding(
            Some(&cell_margins),
            Some(&row_margins),
            Some(&table_margins),
            false,
            default_x,
            default_y,
        );
        assert!((resolved.top - points_to_px(10.0)).abs() < 0.01);
        assert!((resolved.left - points_to_px(0.0)).abs() < 0.01);
        // bottom unspecified on the cell and row → table value (2pt).
        assert!((resolved.bottom - points_to_px(2.0)).abs() < 0.01);
        // right unspecified on cell/row → table value (5pt).
        assert!((resolved.right - points_to_px(5.0)).abs() < 0.01);
        assert!((resolved.horizontal() - (resolved.left + resolved.right)).abs() < f64::EPSILON);
        assert!((resolved.vertical() - (resolved.top + resolved.bottom)).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_cell_padding_maps_logical_edges_through_bidi_visual() {
        let margins = TableCellMargins {
            start: Some(12.0),
            end: Some(3.0),
            ..TableCellMargins::default()
        };

        let ltr = resolve_cell_padding(Some(&margins), None, None, false, 0.0, 0.0);
        assert!((ltr.left - points_to_px(12.0)).abs() < 0.01);
        assert!((ltr.right - points_to_px(3.0)).abs() < 0.01);

        let rtl = resolve_cell_padding(Some(&margins), None, None, true, 0.0, 0.0);
        assert!((rtl.left - points_to_px(3.0)).abs() < 0.01);
        assert!((rtl.right - points_to_px(12.0)).abs() < 0.01);

        // Precedence remains per physical side after direction resolution: a
        // cell-level logical end overrides only the mirrored left, while the
        // right continues to inherit table-level logical start.
        let cell = TableCellMargins {
            end: Some(1.0),
            ..TableCellMargins::default()
        };
        let inherited = resolve_cell_padding(Some(&cell), None, Some(&margins), true, 0.0, 0.0);
        assert!((inherited.left - points_to_px(1.0)).abs() < 0.01);
        assert!((inherited.right - points_to_px(12.0)).abs() < 0.01);
    }

    #[test]
    fn spacing_background_prefers_row_exception_and_respects_zero_spacing() {
        let mut table = Table {
            cell_spacing: Some(7.2),
            ..Default::default()
        };
        table
            .cell_style
            .insert("background-color".into(), "#DDEBF7".into());
        let mut row = TableRow::default();
        row.style
            .insert("background-color".into(), "#FFF2CC".into());
        assert_eq!(table_spacing_background(&row, &table), Some("#FFF2CC"));
        row.cell_spacing = Some(0.0);
        assert_eq!(table_spacing_background(&row, &table), None);
        row.cell_spacing = None;
        row.style.clear();
        assert_eq!(table_spacing_background(&row, &table), Some("#DDEBF7"));
        table.cell_spacing = None;
        assert_eq!(table_spacing_background(&row, &table), None);
    }

    #[test]
    fn spacing_background_never_paints_the_cell_interior() {
        let painter = RecordingPainter::new();
        let slot = CellBorderRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 60.0,
        };
        for spacing in [0.0, 12.0, 200.0] {
            painter.clear();
            let cell = spaced_cell_rect(slot.x, slot.y, slot.width, slot.height, spacing);
            paint_cell_spacing_background(&painter, Some("#GAP"), slot, cell);
            let mut painted_area = 0.0;
            for op in painter.ops() {
                if let Op::FillRect(x, y, width, height) = op {
                    assert!(x >= slot.x && y >= slot.y);
                    assert!(x + width <= slot.x + slot.width && y + height <= slot.y + slot.height);
                    let overlap_width = (x + width).min(cell.x + cell.width) - x.max(cell.x);
                    let overlap_height = (y + height).min(cell.y + cell.height) - y.max(cell.y);
                    assert!(overlap_width <= 0.0 || overlap_height <= 0.0);
                    painted_area += width * height;
                }
            }
            assert_eq!(
                painted_area,
                slot.width * slot.height - cell.width * cell.height
            );
            if spacing == 0.0 {
                assert!(painter.ops().is_empty());
            }
        }
        painter.clear();
        let cell = spaced_cell_rect(slot.x, slot.y, slot.width, slot.height, 12.0);
        paint_cell_spacing_background(&painter, None, slot, cell);
        assert!(painter.ops().is_empty());
    }

    fn conditional_fill(fill: &str) -> crate::document::TableStyleConditionalFormat {
        crate::document::TableStyleConditionalFormat {
            fill: Some(fill.to_string()),
            ..Default::default()
        }
    }

    /// Style format of the single cell in a one-column row.
    fn row_style_format(table: &Table, row: usize, row_count: usize) -> TableStyleCellFormat {
        table.table_style_cell_format(TableStyleCellPosition {
            row,
            row_count,
            cell: 0,
            cell_count: 1,
        })
    }

    #[test]
    fn table_and_row_shading_do_not_replace_the_cell_style() {
        let mut table = Table::default();
        table
            .cell_style
            .insert("background-color".into(), "#DDEBF7".into());
        let mut row = TableRow::default();
        row.style
            .insert("background-color".into(), "#FFF2CC".into());
        let mut cell = TableCell::default();
        let format = TableStyleCellFormat::default();
        for spacing in [None, Some(0.0), Some(6.0)] {
            row.cell_spacing = spacing;
            assert_eq!(resolve_cell_background(&cell, &table, &format), None);
        }
        let format = TableStyleCellFormat {
            fill: Some("#STYLE".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_cell_background(&cell, &table, &format).as_deref(),
            Some("#STYLE")
        );
        cell.style.insert("background-color".into(), "#CELL".into());
        assert_eq!(
            resolve_cell_background(&cell, &table, &format).as_deref(),
            Some("#CELL")
        );
    }

    #[test]
    fn document_table_style_respects_first_row_bands_and_direct_precedence() {
        use crate::document::{TableLook, TableStyleCondition, TableStyleDisplay};

        let mut table = Table {
            row_band_size: Some(2),
            look: TableLook::from_mask(TableLook::FIRST_ROW),
            resolved_style: Some(TableStyleDisplay {
                fill: Some("#BASE".to_string()),
                conditional: [
                    (TableStyleCondition::FirstRow, conditional_fill("#HEAD")),
                    (TableStyleCondition::Band1Horz, conditional_fill("#BAND1")),
                    (TableStyleCondition::Band2Horz, conditional_fill("#BAND2")),
                ]
                .into_iter()
                .collect(),
                ..TableStyleDisplay::default()
            }),
            ..Table::default()
        };
        let mut row = TableRow::default();
        let mut cell = TableCell::default();
        let background = |table: &Table, _row: &TableRow, cell: &TableCell, index: usize| {
            resolve_cell_background(cell, table, &row_style_format(table, index, 6))
        };

        assert_eq!(background(&table, &row, &cell, 0).as_deref(), Some("#HEAD"));
        assert_eq!(
            background(&table, &row, &cell, 1).as_deref(),
            Some("#BAND1")
        );
        assert_eq!(
            background(&table, &row, &cell, 2).as_deref(),
            Some("#BAND1")
        );
        assert_eq!(
            background(&table, &row, &cell, 3).as_deref(),
            Some("#BAND2")
        );

        row.style
            .insert("background-color".to_string(), "#ROW".to_string());
        assert_eq!(
            background(&table, &row, &cell, 3).as_deref(),
            Some("#BAND2")
        );
        cell.style
            .insert("background-color".to_string(), "#CELL".to_string());
        assert_eq!(background(&table, &row, &cell, 3).as_deref(), Some("#CELL"));

        table.look = TableLook::from_mask(TableLook::NO_H_BAND);
        row.style.clear();
        cell.style.clear();
        assert_eq!(background(&table, &row, &cell, 0).as_deref(), Some("#BASE"));
        assert_eq!(background(&table, &row, &cell, 1).as_deref(), Some("#BASE"));

        // Word's omitted tblStyleRowBandSize is 0, which disables banding
        // (MS-OI29500 2.1.251); the style's own size applies otherwise.
        table.look = TableLook::from_mask(TableLook::FIRST_ROW);
        table.row_band_size = None;
        assert_eq!(background(&table, &row, &cell, 2).as_deref(), Some("#BASE"));
        table.resolved_style.as_mut().expect("style").row_band_size = Some(1);
        assert_eq!(
            background(&table, &row, &cell, 2).as_deref(),
            Some("#BAND2")
        );
    }

    #[test]
    fn table_style_conditions_layer_in_word_order_for_corner_cells() {
        use crate::document::{TableLook, TableStyleCondition, TableStyleDisplay};

        // Word applies row banding, column banding, first/last column,
        // first/last row, then corners (MS-OI29500 §17.7.6.6(a)): a header
        // row wins over the first column, and nwCell wins over both.
        let mut conditional: std::collections::BTreeMap<_, _> = [
            (TableStyleCondition::FirstRow, conditional_fill("#ROW")),
            (TableStyleCondition::FirstCol, conditional_fill("#COL")),
            (TableStyleCondition::Band1Vert, conditional_fill("#VBAND")),
            (TableStyleCondition::Band1Horz, conditional_fill("#HBAND")),
        ]
        .into_iter()
        .collect();
        let table_with = |conditional, mask| Table {
            look: TableLook::from_mask(mask),
            resolved_style: Some(TableStyleDisplay {
                conditional,
                row_band_size: Some(1),
                col_band_size: Some(1),
                ..TableStyleDisplay::default()
            }),
            ..Table::default()
        };
        let at = |table: &Table, row, cell| {
            table
                .table_style_cell_format(TableStyleCellPosition {
                    row,
                    row_count: 4,
                    cell,
                    cell_count: 3,
                })
                .fill
        };
        let all = TableLook::FIRST_ROW | TableLook::FIRST_COLUMN;
        let table = table_with(conditional.clone(), all);
        assert_eq!(at(&table, 0, 0).as_deref(), Some("#ROW"));
        assert_eq!(at(&table, 2, 0).as_deref(), Some("#COL"));
        // Column banding is applied after row banding.
        assert_eq!(at(&table, 1, 1).as_deref(), Some("#VBAND"));

        conditional.insert(TableStyleCondition::NwCell, conditional_fill("#NW"));
        let table = table_with(conditional.clone(), all);
        assert_eq!(at(&table, 0, 0).as_deref(), Some("#NW"));
        // The corner needs both the header row and the first column.
        let header_only = table_with(conditional, TableLook::FIRST_ROW);
        assert_eq!(at(&header_only, 0, 0).as_deref(), Some("#ROW"));
    }

    #[test]
    fn document_table_style_is_fallback_for_margins_borders_and_header_bold() {
        use crate::document::{
            BorderEdge, TableBorders, TableCellMargins, TableStyleCondition,
            TableStyleConditionalFormat, TableStyleDisplay,
        };

        let style_margins = TableCellMargins {
            top: Some(3.0),
            start: Some(7.0),
            ..TableCellMargins::default()
        };
        let style_borders = TableBorders {
            top: Some(BorderEdge {
                val: Some("double".to_string()),
                size_eighths: Some(12),
                space: None,
                color: Some("#123456".to_string()),
            }),
            ..TableBorders::default()
        };
        let table = Table {
            resolved_style: Some(TableStyleDisplay {
                conditional: [(
                    TableStyleCondition::FirstRow,
                    TableStyleConditionalFormat {
                        bold: Some(true),
                        ..TableStyleConditionalFormat::default()
                    },
                )]
                .into_iter()
                .collect(),
                borders: Some(style_borders),
                cell_margins: Some(style_margins.clone()),
                ..TableStyleDisplay::default()
            }),
            ..Table::default()
        };
        assert_eq!(table.effective_cell_margins(), Some(&style_margins));
        assert_eq!(
            effective_table_border_edge_for_side(
                &TableRow::default(),
                &table,
                TableBorderSide::Top,
                true,
                false,
                true,
                false,
            )
            .and_then(|edge| edge.val.as_deref()),
            Some("double")
        );

        let mut runs = vec![
            StyledRunRange {
                picture: None,
                start: 0,
                end: 1,
                properties: None,
                revision: None,
            },
            StyledRunRange {
                picture: None,
                start: 1,
                end: 2,
                properties: Some(RunProperties {
                    bold: Some(false),
                    ..RunProperties::default()
                }),
                revision: None,
            },
        ];
        apply_table_style_conditional_runs(&mut runs, &row_style_format(&table, 0, 3));
        assert_eq!(
            runs[0].properties.as_ref().and_then(|props| props.bold),
            Some(true)
        );
        assert_eq!(
            runs[1].properties.as_ref().and_then(|props| props.bold),
            Some(false)
        );
        let mut body_runs = vec![StyledRunRange {
            picture: None,
            start: 0,
            end: 1,
            properties: None,
            revision: None,
        }];
        apply_table_style_conditional_runs(&mut body_runs, &row_style_format(&table, 1, 3));
        assert_eq!(
            body_runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            None
        );
    }

    #[test]
    fn spacing_slots_pay_perimeter_rules_and_preserve_authored_width() {
        for (spacing, cell_edge, table_edge, expected_slot) in [
            (7.2, 1.0, 0.0, 155.8),
            (3.0, 1.0, 0.0, 160.0),
            (6.0, 6.0, 0.0, 162.0),
        ] {
            let scale =
                spacing_slot_scale(324.0, spacing, cell_edge, cell_edge, table_edge, table_edge);
            assert!((162.0 * scale - expected_slot).abs() < 1e-9);
            assert!(
                (324.0 * scale + 2.0 * spacing - 2.0 * cell_edge + 2.0 * table_edge - 324.0).abs()
                    < 1e-9
            );
        }
        // A separate three-column Word probe uses equal 1pt cell/outline rules
        // and 40pt row spacing: the remaining outer cell box is 13 1/3pt.
        let scale = spacing_slot_scale(360.0, 40.0, 1.0, 1.0, 1.0, 1.0);
        assert!((120.0 * scale - 80.0 - 40.0 / 3.0).abs() < 1e-9);
        let widths = [60.0, 100.0, 200.0];
        assert!((widths.iter().map(|w| w * scale).sum::<f64>() + 80.0 - 360.0).abs() < 1e-9);
        assert_eq!(spacing_slot_scale(360.0, 0.0, 8.0, 4.0, 1.0, 1.0), 1.0);
        assert_eq!(spacing_slot_scale(10.0, 40.0, 0.0, 0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn spacing_reserves_two_horizontal_sides_and_both_vertical_sides() {
        let rect = spaced_cell_rect(10.0, 20.0, 100.0, 60.0, 12.0);
        assert_eq!(rect.x, 34.0);
        assert_eq!(rect.y, 32.0);
        assert_eq!(rect.width, 76.0);
        assert_eq!(rect.height, 36.0);
    }

    #[test]
    fn omitted_cell_spacing_preserves_the_collapsed_grid_slot() {
        // Zero spacing is exactly the legacy geometry. This matters because
        // omitted tblCellSpacing must not perturb existing table fixtures.
        let unchanged = spaced_cell_rect(10.0, 20.0, 100.0, 60.0, 0.0);
        assert_eq!(unchanged.x, 10.0);
        assert_eq!(unchanged.y, 20.0);
        assert_eq!(unchanged.width, 100.0);
        assert_eq!(unchanged.height, 60.0);
    }

    #[test]
    fn clipped_ink_projection_declines_rotated_and_vertical_directions() {
        assert!(cell_ink_projection_is_horizontal(None));
        assert!(cell_ink_projection_is_horizontal(Some(TextDirection::LrTb)));
        for direction in [
            TextDirection::LrTbV,
            TextDirection::TbRl,
            TextDirection::TbRlV,
            TextDirection::BtLr,
            TextDirection::BtLrV,
        ] {
            assert!(!cell_ink_projection_is_horizontal(Some(direction)));
        }
    }

    #[test]
    fn ancestor_vertical_clip_intersection_can_hide_a_whole_nested_row() {
        assert_eq!(
            intersect_vertical_clips(Some((20.0, 100.0)), Some((10.0, 26.0))),
            Some((20.0, 26.0))
        );
        assert!(!glyph_ink_intersects_vertical_clip(
            40.0, 11.0, 1.0, 20.0, 26.0
        ));
        let empty = intersect_vertical_clips(Some((30.0, 100.0)), Some((10.0, 26.0))).unwrap();
        assert!(!glyph_ink_intersects_vertical_clip(
            40.0, 20.0, 20.0, empty.0, empty.1
        ));
        assert!(!glyph_ink_intersects_vertical_clip(
            40.0, 0.0, 0.0, empty.0, empty.1
        ));
        assert_eq!(
            intersect_vertical_clips(None, Some((10.0, 26.0))),
            Some((10.0, 26.0))
        );
    }

    #[test]
    fn clipped_ink_cache_reuses_glyphs_resets_fonts_and_retries_errors() {
        let mut cache = ClippedGlyphInkCache::default();
        assert_eq!(
            cache.measure("a", || Ok::<_, ()>((10.0, 1.0))),
            Ok((10.0, 1.0))
        );
        assert_eq!(
            cache.measure("a", || -> Result<_, ()> { panic!("duplicate measure") }),
            Ok((10.0, 1.0))
        );
        cache.clear();
        assert_eq!(
            cache.measure("a", || Ok::<_, ()>((12.0, 2.0))),
            Ok((12.0, 2.0))
        );
        assert_eq!(cache.measure("b", || Err::<(f64, f64), _>(())), Err(()));
        assert_eq!(
            cache.measure("b", || Ok::<_, ()>((7.0, 0.0))),
            Ok((7.0, 0.0))
        );
        let text = "abcdefghijklmnopq";
        cache.clear();
        for i in 0..text.len() {
            cache
                .measure(&text[i..i + 1], || Ok::<_, ()>((1.0, 0.0)))
                .unwrap();
        }
        assert_eq!(
            cache.measure("a", || Ok::<_, ()>((3.0, 0.0))),
            Ok((3.0, 0.0))
        );
        eprintln!(
            "native_capture_ink_cache_bytes={}",
            std::mem::size_of::<ClippedGlyphInkCache<'_>>()
        );
    }

    #[test]
    fn clipped_glyph_projection_uses_ink_and_preserves_unknown_metrics() {
        assert!(glyph_ink_intersects_vertical_clip(
            22.0, 10.0, 0.0, 0.0, 14.0
        ));
        assert!(!glyph_ink_intersects_vertical_clip(
            22.0, 7.0, 0.0, 0.0, 14.0
        ));
        assert!(!glyph_ink_intersects_vertical_clip(
            22.0, 8.0, 0.0, 0.0, 14.0
        ));
        assert!(!glyph_ink_intersects_vertical_clip(
            0.0, 3.0, -1.0, 0.0, 14.0
        ));
        assert!(glyph_ink_intersects_vertical_clip(3.0, 3.0, 1.0, 0.0, 14.0));
        assert!(glyph_ink_intersects_vertical_clip(
            22.0, 0.0, 0.0, 0.0, 14.0
        ));
        assert!(glyph_ink_intersects_vertical_clip(
            22.0,
            f64::NAN,
            0.0,
            0.0,
            14.0
        ));
    }

    #[test]
    fn package_borderless_table_style_does_not_invent_a_grid() {
        let table = Table {
            resolved_style: Some(crate::document::TableStyleDisplay::default()),
            ..Table::default()
        };
        let cell = TableCell::default();
        let row = TableRow::default();
        let format = TableStyleCellFormat::default();
        for (name, side) in [
            ("top", TableBorderSide::Top),
            ("bottom", TableBorderSide::Bottom),
            ("left", TableBorderSide::Left),
            ("right", TableBorderSide::Right),
        ] {
            let resolved =
                resolve_cell_border_side(&cell, &row, &table, &format, name, side, [true; 4], 0.0);
            assert_eq!(resolved.kind, BorderKind::None, "{name}");
        }
    }

    #[test]
    fn mac_package_table_padding_defaults_do_not_override_declared_margins() {
        use jade_docx_layout::line_metrics::WordLayoutPlatform;
        let bare = Table::default();
        let package = Table {
            resolved_style: Some(crate::document::TableStyleDisplay::default()),
            ..Table::default()
        };
        let fallback = table_default_horizontal_padding_px(&package, WordLayoutPlatform::Mac);
        let padding = resolve_cell_padding(None, None, None, false, fallback, 0.0);
        assert_eq!(padding.horizontal(), 0.0);
        let margins = TableCellMargins {
            left: Some(5.4),
            right: Some(7.2),
            ..Default::default()
        };
        let declared = resolve_cell_padding(None, None, Some(&margins), false, fallback, 0.0);
        assert_eq!(declared.left, points_to_px(5.4));
        assert_eq!(declared.right, points_to_px(7.2));
        assert_eq!(
            table_default_horizontal_padding_px(&bare, WordLayoutPlatform::Mac),
            points_to_px(CELL_HORIZONTAL_PADDING_PT)
        );
        assert_eq!(
            table_default_horizontal_padding_px(&package, WordLayoutPlatform::Windows),
            points_to_px(CELL_HORIZONTAL_PADDING_PT)
        );
    }

    #[test]
    fn authored_table_grid_does_not_invent_omitted_collapsed_rules() {
        let table = Table {
            borders: Some(crate::document::TableBorders {
                inside_v: Some(crate::document::BorderEdge {
                    val: Some("single".into()),
                    size_eighths: Some(8),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let row = TableRow::default();
        let cell = TableCell::default();
        let format = TableStyleCellFormat::default();
        for (name, side) in [
            ("top", TableBorderSide::Top),
            ("bottom", TableBorderSide::Bottom),
            ("left", TableBorderSide::Left),
        ] {
            let border =
                resolve_cell_border_side(&cell, &row, &table, &format, name, side, [true; 4], 0.0);
            assert_eq!(border.kind, BorderKind::None);
        }
        let inside = resolve_cell_border_side(
            &cell,
            &row,
            &table,
            &format,
            "right",
            TableBorderSide::Right,
            [false; 4],
            0.0,
        );
        assert_eq!(inside.kind, BorderKind::Single);
    }

    #[test]
    fn separated_cells_use_interior_rules_without_unauthored_outer_rules() {
        use crate::document::{BorderEdge, TableBorders};
        let edge = |val: &str, size, color: &str| BorderEdge {
            val: Some(val.into()),
            size_eighths: Some(size),
            color: Some(color.into()),
            ..Default::default()
        };
        let mut table = Table {
            borders: Some(TableBorders {
                inside_v: Some(edge("single", 8, "#4472C4")),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut row = TableRow::default();
        let cell = TableCell::default();
        let format = TableStyleCellFormat::default();
        let resolve = |table: &Table, row: &TableRow, name, side, spacing| {
            resolve_cell_border_side(
                &cell,
                row,
                table,
                &format,
                name,
                side,
                [true, true, true, true],
                spacing,
            )
        };
        for (name, side) in [
            ("left", TableBorderSide::Left),
            ("right", TableBorderSide::Right),
        ] {
            let border = resolve(&table, &row, name, side, points_to_px(7.2));
            assert_eq!(border.color, "#4472C4");
            assert_eq!(border.kind, BorderKind::Single);
        }
        for (name, side) in [
            ("top", TableBorderSide::Top),
            ("bottom", TableBorderSide::Bottom),
        ] {
            assert_eq!(
                resolve(&table, &row, name, side, points_to_px(7.2)).kind,
                BorderKind::None
            );
        }
        row.borders = Some(Box::new(TableBorders {
            inside_v: Some(edge("double", 16, "#C00000")),
            ..Default::default()
        }));
        for (name, side) in [
            ("left", TableBorderSide::Left),
            ("right", TableBorderSide::Right),
        ] {
            let border = resolve(&table, &row, name, side, points_to_px(6.0));
            assert_eq!(border.color, "#C00000");
            assert_eq!(border.kind, BorderKind::Double);
        }
        row.borders = None;
        table.borders.as_mut().unwrap().left = Some(edge("single", 16, "#00B050"));
        assert_eq!(
            resolve(&table, &row, "left", TableBorderSide::Left, 0.0).color,
            "#00B050"
        );
    }

    #[test]
    fn row_cell_spacing_overrides_table_default() {
        let table = Table {
            cell_spacing: Some(7.2),
            ..Table::default()
        };
        let inherited = TableRow::default();
        let overridden = TableRow {
            cell_spacing: Some(3.0),
            ..TableRow::default()
        };

        assert!((resolve_cell_spacing_px(&inherited, &table) - points_to_px(7.2)).abs() < 0.01);
        assert!((resolve_cell_spacing_px(&overridden, &table) - points_to_px(3.0)).abs() < 0.01);
    }

    #[test]
    fn row_spacing_exception_keeps_its_scope_and_direct_zero_wins() {
        let table = Table {
            cell_spacing: Some(7.2),
            ..Default::default()
        };
        let mut row = TableRow {
            cell_spacing_exception: Some(6.0),
            ..Default::default()
        };
        assert_eq!(row.cell_spacing, None);
        assert_eq!(resolve_cell_spacing_px(&row, &table), points_to_px(6.0));
        row.cell_spacing = Some(3.0);
        assert_eq!(resolve_cell_spacing_px(&row, &table), points_to_px(3.0));
        row.cell_spacing = Some(0.0);
        assert_eq!(resolve_cell_spacing_px(&row, &table), 0.0);
        assert_eq!(row.cell_spacing_exception, Some(6.0));
        row.cell_spacing = None;
        row.cell_spacing_exception = None;
        assert_eq!(resolve_cell_spacing_px(&row, &table), points_to_px(7.2));
    }

    #[test]
    fn mac_double_border_reserves_two_authored_rules_and_their_gap() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let double =
            resolve_border_spec(Some(&"double".to_string()), Some(&"1pt".to_string()), None);
        let single =
            resolve_border_spec(Some(&"single".to_string()), Some(&"1pt".to_string()), None);
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let double_width = visible_border_width(&double);
        let single_width = visible_border_width(&single);
        set_word_layout_platform(previous);
        assert!((double_width - points_to_px(3.0)).abs() < 1e-9);
        assert!((single_width - points_to_px(1.0)).abs() < 1e-9);
        // Conflict weighting continues to use the authored size, not the
        // complete painted band; a 4pt single beats a 1pt double.
        let heavier =
            resolve_border_spec(Some(&"single".to_string()), Some(&"4pt".to_string()), None);
        assert_eq!(resolve_shared_border_conflict(&double, &heavier), heavier);
    }

    #[test]
    fn cell_vertical_align_share_maps_tokens_to_offset_fraction() {
        // Word's default keeps content at the top.
        assert_eq!(cell_vertical_align_share(None), 0.0);
        assert_eq!(cell_vertical_align_share(Some("top")), 0.0);
        // Center splits the leftover slack evenly; bottom pushes to the floor.
        assert_eq!(cell_vertical_align_share(Some("center")), 0.5);
        assert_eq!(cell_vertical_align_share(Some("bottom")), 1.0);
        // Unknown tokens degrade to the top default rather than panicking.
        assert_eq!(cell_vertical_align_share(Some("both")), 0.0);
    }

    #[test]
    fn parse_length_to_px_supports_various_units() {
        let pt = parse_length_to_px("36pt").unwrap();
        assert!((pt - points_to_px(36.0)).abs() < 0.01);

        let px = parse_length_to_px("144px").unwrap();
        assert!((px - 144.0).abs() < 0.01);

        let raw = parse_length_to_px("72").unwrap();
        assert!((raw - 72.0).abs() < 0.01);

        assert!(parse_length_to_px("1em").is_none());
    }

    #[test]
    fn modern_border_content_width_uses_the_neighbouring_winner_before_wrap() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let context = TableLineWrapContext::new(&painter);
        let mut config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 80.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: true,
            styles_part: None,
        };
        for bidi in [false, true] {
            let mut row = TableRow::default();
            for index in 0..2 {
                let mut cell = TableCell {
                    blocks: vec![BodyElement::from_paragraph(
                        serde_json::from_value(serde_json::json!({
                            "runs": [{"text": "AAAAA"}]
                        }))
                        .unwrap(),
                    )],
                    ..Default::default()
                };
                let nested = Table {
                    table_layout: Some(TableLayoutMode::Fixed),
                    rows: vec![TableRow {
                        cells: vec![TableCell {
                            blocks: cell.blocks.clone(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                cell.blocks.push(BodyElement::from_table(nested));
                for side in ["top", "right", "bottom", "left"] {
                    cell.style
                        .insert(format!("border-{side}-style"), "nil".into());
                }
                if index == 0 {
                    let side = if bidi { "left" } else { "right" };
                    cell.style
                        .insert(format!("border-{side}-style"), "single".into());
                    cell.style
                        .insert(format!("border-{side}-width"), "12px".into());
                }
                row.cells.push(cell);
            }
            let table = Table {
                bidi_visual: bidi,
                rows: vec![row.clone()],
                ..Default::default()
            };
            let prepare = |config: &LayoutConfig, row: &TableRow| {
                prepare_row_layout(
                    &context,
                    row,
                    &table,
                    &[40.0, 40.0],
                    config,
                    None,
                    0,
                    1,
                    &mut NaturalParagraphLayoutCache::default(),
                    None,
                    &|_| None,
                )
                .unwrap()
            };
            config.word_modern_border_insets = true;
            let cells = prepare(&config, &row);
            let inset = |cell: &PreparedCell, first: bool| {
                if first != bidi {
                    cell.padding.right
                } else {
                    cell.padding.left
                }
            };
            assert_eq!(inset(&cells[0], true), 0.0);
            assert_eq!(inset(&cells[1], false), 0.0);
            let line_count = |cell: &PreparedCell| match &cell.blocks[0].kind {
                CellBlockKind::Paragraph(paragraph) => paragraph.lines.len(),
                _ => panic!("paragraph"),
            };
            assert_eq!(line_count(&cells[0]), 1);
            assert_eq!(line_count(&cells[1]), 1);
            let nested = |cell: &PreparedCell| match &cell.blocks[1].kind {
                CellBlockKind::Table(table) => (table.width, line_count(&table.rows[0].cells[0])),
                _ => panic!("nested table"),
            };
            assert_eq!(nested(&cells[0]), (34.0, 2));
            assert_eq!(nested(&cells[1]), (34.0, 2));
            config.word_modern_border_insets = false;
            let legacy = prepare(&config, &row);
            assert_eq!(inset(&legacy[0], true), 0.0);
            assert_eq!(inset(&legacy[1], false), 0.0);
            assert_eq!(line_count(&legacy[0]), 1);
            assert_eq!(line_count(&legacy[1]), 1);
            assert_eq!(nested(&legacy[0]), (40.0, 1));
            assert_eq!(nested(&legacy[1]), (40.0, 1));
            config.word_modern_border_insets = true;
            row.cell_spacing = Some(1.0);
            let spaced = prepare(&config, &row);
            assert_eq!(inset(&spaced[1], false), 0.0);
            assert!((nested(&spaced[1]).0 - (40.0 - 3.0 * points_to_px(1.0))).abs() < 1e-6);
        }
    }

    #[test]
    fn repeated_nested_table_paints_at_the_editable_content_frame_origin() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        let context = TableLineWrapContext::new(&painter);
        let config = LayoutConfig {
            padding_x: 7.2,
            padding_y: 0.0,
            line_height: 20.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            available_width: 200.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            document_snap_to_grid_default: false,
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            styles_part: None,
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(
                        serde_json::from_value(serde_json::json!({
                            "runs": [{"text": "Nested"}]
                        }))
                        .unwrap(),
                    )],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let layout = layout_nested_table(&context, &table, 200.0, &config, &|_| None).unwrap();
        let text_positions = |ops: Vec<Op>| {
            ops.into_iter()
                .filter_map(|op| match op {
                    Op::FillText(text, x, y) => Some((text, x, y)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        for mode in [Some(12), Some(14), Some(15), None] {
            let ordinary_painter = RecordingPainter::with_width_per_char(7.0);
            let mut ordinary =
                super::super::tests::recording_table_renderer(ordinary_painter.clone());
            ordinary.document_east_asian_rules.compatibility_mode = mode;
            ordinary
                .render_nested_table(
                    &mut layout.clone(),
                    None,
                    &mut NumberingCounter::default(),
                    100.0,
                    40.0,
                    200.0,
                    None,
                )
                .unwrap();
            let repeated_painter = RecordingPainter::with_width_per_char(7.0);
            let mut repeated =
                super::super::tests::recording_table_renderer(repeated_painter.clone());
            repeated.document_east_asian_rules.compatibility_mode = mode;
            repeated
                .draw_nested_table_visual_only(&layout, 100.0, 40.0, 200.0)
                .unwrap();
            let ordinary_text = text_positions(ordinary_painter.ops());
            let repeated_text = text_positions(repeated_painter.ops());
            assert!(!ordinary_text.is_empty());
            assert_eq!(ordinary.table_layouts.len(), 1);
            assert!(repeated.table_layouts.is_empty());
            assert_eq!(repeated.paragraph_counter, 0);
            assert_eq!(repeated_text, ordinary_text, "compatibility mode {mode:?}");
        }
        let mut spaced_table = table.clone();
        spaced_table.cell_spacing = Some(6.0);
        spaced_table
            .cell_style
            .insert("background-color".into(), "#GAP".into());
        let spaced =
            layout_nested_table(&context, &spaced_table, 200.0, &config, &|_| None).unwrap();
        let colored_rects = |ops: Vec<Op>| {
            let mut color = String::new();
            let mut rectangles = Vec::new();
            for op in ops {
                match op {
                    Op::SetFillStyle(value) => color = value,
                    Op::FillRect(x, y, width, height) if color == "#GAP" => {
                        rectangles.push((x, y, width, height))
                    }
                    _ => {}
                }
            }
            rectangles
        };
        let ordinary_painter = RecordingPainter::with_width_per_char(7.0);
        let mut ordinary = super::super::tests::recording_table_renderer(ordinary_painter.clone());
        ordinary
            .render_nested_table(
                &mut spaced.clone(),
                None,
                &mut NumberingCounter::default(),
                100.0,
                40.0,
                200.0,
                None,
            )
            .unwrap();
        let repeated_painter = RecordingPainter::with_width_per_char(7.0);
        let mut repeated = super::super::tests::recording_table_renderer(repeated_painter.clone());
        repeated
            .draw_nested_table_visual_only(&spaced, 100.0, 40.0, 200.0)
            .unwrap();
        let expected = colored_rects(ordinary_painter.ops());
        assert_eq!(expected.len(), 4);
        assert_eq!(colored_rects(repeated_painter.ops()), expected);
        let header_painter = RecordingPainter::with_width_per_char(7.0);
        let mut header = super::super::tests::recording_table_renderer(header_painter.clone());
        header.current_y = 40.0;
        header
            .draw_repeated_header_rows(
                &[spaced.rows[0].cells.clone()],
                &[spaced.rows[0].height],
                &spaced_table,
                TableVisualGrid {
                    table_x: 100.0,
                    column_positions: &spaced.column_positions,
                    column_widths: &spaced.column_widths,
                    bidi_visual: false,
                },
                20.0,
            )
            .unwrap();
        assert_eq!(colored_rects(header_painter.ops()), expected);
    }

    #[test]
    fn pre_2013_tables_align_first_cell_text_with_the_indent() {
        let mut table = Table {
            cell_margins: Some(TableCellMargins {
                left: Some(0.5),
                right: Some(0.5),
                ..TableCellMargins::default()
            }),
            rows: vec![TableRow {
                cells: vec![TableCell::default()],
                ..TableRow::default()
            }],
            ..Table::default()
        };
        let inset = pre_2013_table_text_inset_px(&table, Some(12));
        assert!((inset - points_to_px(0.5)).abs() < 1e-9);
        assert_eq!(pre_2013_table_text_inset_px(&table, Some(15)), 0.0);
        assert_eq!(pre_2013_table_text_inset_px(&table, None), 0.0);
        table
            .style
            .insert("margin-left".to_string(), "0.5pt".to_string());
        // Word for Mac: `tblInd` 0.5pt puts the first cell's text 0.5pt in.
        let x = compute_table_x(100.0, 400.0, 240.0, &table, inset);
        assert!((x + inset - (100.0 + points_to_px(0.5))).abs() < 0.01);
    }

    #[test]
    fn mac_autofit_keeps_an_absolute_table_width_past_the_margin() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        let mut table = Table::default();
        table.style.insert("width".into(), "600pt".into());
        let text_width = points_to_px(496.8);

        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = word_autofit_width_limit_px(&table, text_width);
        table.style.insert("width".into(), "50%".into());
        let mac_percent = word_autofit_width_limit_px(&table, text_width);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        table.style.insert("width".into(), "600pt".into());
        let windows = word_autofit_width_limit_px(&table, text_width);
        set_word_layout_platform(previous);

        assert!((mac - points_to_px(600.0)).abs() < 1e-9);
        assert_eq!(mac_percent, text_width);
        assert_eq!(windows, text_width);
    }

    #[test]
    fn modern_mac_tables_put_the_leading_border_outside_the_indent() {
        use crate::document::{BorderEdge, TableBorders};
        let edge = BorderEdge {
            val: Some("single".into()),
            size_eighths: Some(4),
            color: Some("000000".into()),
            ..Default::default()
        };
        let mut table = Table {
            borders: Some(TableBorders {
                left: Some(edge.clone()),
                right: Some(edge),
                ..Default::default()
            }),
            rows: vec![TableRow {
                cells: vec![TableCell::default(), TableCell::default()],
                ..TableRow::default()
            }],
            ..Table::default()
        };
        // `footnote-table-textbox-order`: 0.5pt borders, text 0.25pt in.
        let half = modern_leading_border_half_px(&table, true);
        assert!((half - points_to_px(0.25)).abs() < 1e-9, "half={half}");
        assert_eq!(modern_leading_border_half_px(&table, false), 0.0);
        let x = compute_table_x(100.0, 400.0, 240.0, &table, -half);
        assert!((x - (100.0 + points_to_px(0.25))).abs() < 1e-9);

        table.cell_spacing = Some(2.0);
        assert_eq!(modern_leading_border_half_px(&table, true), 0.0);
        table.cell_spacing = None;
        table.borders = Some(TableBorders {
            left: Some(BorderEdge {
                val: Some("nil".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(modern_leading_border_half_px(&table, true), 0.0);
    }

    #[test]
    fn compute_table_x_applies_indent_and_alignment() {
        let mut table = Table::default();
        table
            .style
            .insert("margin-left".to_string(), "24pt".to_string());
        let left = compute_table_x(100.0, 400.0, 240.0, &table, 0.0);
        assert!((left - (100.0 + points_to_px(24.0))).abs() < 0.01);

        table
            .style
            .insert("text-align".to_string(), "right".to_string());
        let right = compute_table_x(100.0, 400.0, 240.0, &table, 0.0);
        assert!((right - (100.0 + points_to_px(24.0) + 160.0)).abs() < 0.01);
    }

    #[test]
    fn legacy_bidi_physical_left_alignment_uses_the_text_edge() {
        let table = Table {
            bidi_visual: true,
            style: [("text-align".into(), "right".into())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(compute_table_x(100.0, 400.0, 240.0, &table, 8.0), 92.0);
    }

    #[test]
    fn mac_legacy_right_alignment_uses_the_trailing_text_edge() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell::default()],
                ..Default::default()
            }],
            style: [("text-align".into(), "right".into())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let trailing = pre_2013_table_trailing_text_inset_px(&table);
        assert!(trailing > 0.0);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            compute_table_x(100.0, 400.0, 240.0, &table, 8.0),
            260.0 + trailing
        );
        // Mode 15 has no legacy text inset and keeps the border at the margin.
        assert_eq!(compute_table_x(100.0, 400.0, 240.0, &table, 0.0), 260.0);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(compute_table_x(100.0, 400.0, 240.0, &table, 8.0), 260.0);
        set_word_layout_platform(previous);
    }

    #[test]
    fn ignored_table_indentation_does_not_add_a_legacy_text_inset() {
        let mut table = Table {
            bidi_visual: true,
            rows: vec![TableRow {
                cells: vec![TableCell::default()],
                ..Default::default()
            }],
            ..Default::default()
        };
        for kind in ["auto", "pct"] {
            table
                .style
                .insert("table-indent-width-type".into(), kind.into());
            assert_eq!(pre_2013_table_text_inset_px(&table, Some(14)), 0.0);
        }
    }

    #[test]
    fn compute_table_x_reverses_alignment_and_leading_indent_for_bidi_visual() {
        let mut table = Table {
            bidi_visual: true,
            ..Table::default()
        };

        // `w:jc="left"` is interpreted from the opposite physical edge for a
        // bidiVisual table. With 160px of spare width, logical left therefore
        // places the table against the physical right margin.
        table
            .style
            .insert("text-align".to_string(), "left".to_string());
        assert!((compute_table_x(100.0, 400.0, 240.0, &table, 0.0) - 260.0).abs() < 0.01);

        // The converse alignment lands on the physical left. Center remains
        // centered because reflecting it around the text extents is a no-op.
        table
            .style
            .insert("text-align".to_string(), "right".to_string());
        assert!((compute_table_x(100.0, 400.0, 240.0, &table, 0.0) - 100.0).abs() < 0.01);
        table
            .style
            .insert("text-align".to_string(), "center".to_string());
        assert!((compute_table_x(100.0, 400.0, 240.0, &table, 0.0) - 180.0).abs() < 0.01);

        // tblInd is measured from the leading margin: right-to-left tables
        // move leftward into the text area instead of adding a physical-left
        // offset. 24pt = 32px at the renderer's 96dpi coordinate system.
        table
            .style
            .insert("text-align".to_string(), "left".to_string());
        table
            .style
            .insert("margin-left".to_string(), "24pt".to_string());
        assert!((compute_table_x(100.0, 400.0, 240.0, &table, 0.0) - 228.0).abs() < 0.01);
    }

    #[test]
    fn floating_table_horizontal_position_overrides_inline_alignment() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-horizontal-anchor".into(), "margin".into());
        table
            .style
            .insert("table-position-x-align".into(), "right".into());
        table.style.insert("text-align".into(), "left".into());
        table.style.insert("margin-left".into(), "36pt".into());

        let right = compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 1)
            .expect("floating position");
        assert!((right - 260.0).abs() < 0.01);

        table.style.remove("table-position-x-align");
        table
            .style
            .insert("table-position-horizontal-anchor".into(), "page".into());
        table.style.insert("table-position-x".into(), "72pt".into());
        let absolute = compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 1)
            .expect("absolute page position");
        assert!((absolute - 96.0).abs() < 0.01);
    }

    #[test]
    fn nested_floating_table_text_anchor_uses_owning_cell_text_frame() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-horizontal-anchor".into(), "text".into());
        table
            .style
            .insert("table-position-x-align".into(), "right".into());

        let x =
            compute_floating_table_x_in_frames(100.0, 400.0, 600.0, 220.0, 180.0, 80.0, &table, 1)
                .expect("nested floating position");

        assert!((x - 320.0).abs() < 0.01);
    }

    #[test]
    fn floating_table_missing_anchors_defaults_to_page() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table.style.insert("table-position-x".into(), "72pt".into());
        table.style.insert("table-position-y".into(), "72pt".into());

        let x = compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 1)
            .expect("default page X anchor");
        let y = compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table)
            .expect("default page Y anchor");
        assert!((x - 96.0).abs() < 0.01);
        assert!((y - 96.0).abs() < 0.01);
    }

    #[test]
    fn floating_inside_and_outside_positions_follow_page_parity() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-horizontal-anchor".into(), "margin".into());
        table
            .style
            .insert("table-position-x-align".into(), "inside".into());

        let odd_inside =
            compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 1).expect("odd inside");
        let even_inside =
            compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 2).expect("even inside");
        assert!((odd_inside - 100.0).abs() < 0.01);
        assert!((even_inside - 260.0).abs() < 0.01);

        table
            .style
            .insert("table-position-x-align".into(), "outside".into());
        let odd_outside =
            compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 1).expect("odd outside");
        let even_outside =
            compute_floating_table_x(100.0, 400.0, 600.0, 240.0, &table, 2).expect("even outside");
        assert!((odd_outside - 260.0).abs() < 0.01);
        assert!((even_outside - 100.0).abs() < 0.01);
    }

    #[test]
    fn floating_table_vertical_position_resolves_absolute_and_relative_frames() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-vertical-anchor".into(), "page".into());
        table.style.insert("table-position-y".into(), "72pt".into());

        let absolute = compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table)
            .expect("absolute page Y");
        assert!((absolute - 96.0).abs() < 0.01);

        table.style.remove("table-position-y");
        table
            .style
            .insert("table-position-vertical-anchor".into(), "margin".into());
        table
            .style
            .insert("table-position-y-align".into(), "center".into());
        let centered = compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table)
            .expect("centered margin Y");
        assert!((centered - 350.0).abs() < 0.01);

        table
            .style
            .insert("table-position-vertical-anchor".into(), "text".into());
        table
            .style
            .insert("table-position-y-align".into(), "top".into());
        table.style.insert("table-position-y".into(), "72pt".into());
        let text_top =
            compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table).expect("text top Y");
        assert!(
            (text_top - 336.0).abs() < 0.01,
            "text anchors ignore tblpYSpec and use absolute tblpY"
        );
    }

    #[test]
    fn floating_table_inline_vertical_alignment_stays_in_document_flow() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-vertical-anchor".into(), "page".into());
        table
            .style
            .insert("table-position-y-align".into(), "inline".into());
        // Keep an absolute offset in the model to prove that tblpYSpec wins.
        // Word's `inline` ST_YAlign value is special: unlike `top`, it keeps
        // the table at the surrounding content's logical position instead of
        // moving it to the selected page/margin anchor edge.
        table.style.insert("table-position-y".into(), "72pt".into());

        let inline = compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table)
            .expect("inline floating table Y");

        assert!(
            (inline - 240.0).abs() < 0.01,
            "inline alignment must preserve the table's logical body position"
        );
    }

    #[test]
    fn floating_table_vertical_inside_and_outside_use_anchor_extents() {
        let mut table = Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        table
            .style
            .insert("table-position-vertical-anchor".into(), "margin".into());
        table
            .style
            .insert("table-position-y-align".into(), "inside".into());
        assert_eq!(
            compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table),
            Some(80.0)
        );

        table
            .style
            .insert("table-position-y-align".into(), "outside".into());
        assert_eq!(
            compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table),
            Some(620.0)
        );
    }

    #[test]
    fn floating_table_without_vertical_position_defaults_to_page_origin() {
        let table = Table {
            style: [("table-position".into(), "floating".into())]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        assert_eq!(
            compute_floating_table_y(80.0, 640.0, 800.0, 240.0, 100.0, &table),
            Some(0.0)
        );
    }

    #[test]
    fn floating_table_text_exclusion_includes_tblppr_distances() {
        let mut table = Table::default();
        table
            .style
            .insert("table-distance-top".into(), "3pt".into());
        table
            .style
            .insert("table-distance-bottom".into(), "6pt".into());
        table
            .style
            .insert("table-distance-left".into(), "9pt".into());
        table
            .style
            .insert("table-distance-right".into(), "12pt".into());
        let bounds = floating_table_text_exclusion(
            &table,
            floating_frame(1, 100.0, 200.0, 120.0, 80.0, false),
        );
        assert_eq!(bounds.top, 116.0);
        assert_eq!(bounds.bottom, 208.0);
        assert_eq!(bounds.left, 88.0);
        assert_eq!(bounds.right, 316.0);
    }

    fn floating_frame(
        page_number: usize,
        left: f64,
        width: f64,
        top: f64,
        height: f64,
        prevents_overlap: bool,
    ) -> FloatingTableFrame {
        FloatingTableFrame {
            page_number,
            left,
            right: left + width,
            top,
            bottom: top + height,
            prevents_overlap,
        }
    }

    #[test]
    fn floating_table_never_overlap_moves_below_intersecting_frames() {
        let existing = vec![FloatingTableFrame {
            page_number: 1,
            left: 100.0,
            right: 300.0,
            top: 120.0,
            bottom: 180.0,
            prevents_overlap: false,
        }];

        // The new table forbids overlap, so an intersecting requested frame is
        // moved immediately below the existing table. Horizontal separation
        // leaves the requested Y untouched.
        assert_eq!(
            resolve_floating_table_overlap_y(
                floating_frame(1, 150.0, 200.0, 140.0, 50.0, true),
                0.0,
                &existing,
            ),
            180.0
        );
        assert_eq!(
            resolve_floating_table_overlap_y(
                floating_frame(1, 320.0, 100.0, 140.0, 50.0, true),
                0.0,
                &existing,
            ),
            140.0
        );
    }

    #[test]
    fn page_spanning_floating_table_registers_its_continuation_frames() {
        let first = floating_frame(3, 100.0, 200.0, 700.0, 900.0, true);
        let frames = floating_table_page_frames(first, 5, 260.0, 96.0, 1000.0);
        assert_eq!(
            frames
                .iter()
                .map(|frame| (frame.page_number, frame.top, frame.bottom))
                .collect::<Vec<_>>(),
            vec![(3, 700.0, 1000.0), (4, 96.0, 1000.0), (5, 96.0, 260.0)]
        );
        assert!(frames
            .iter()
            .all(|frame| frame.prevents_overlap && frame.left == 100.0 && frame.right == 300.0));

        // A later table on the continuation page is moved below the continued
        // rows, because the earlier table forbids overlap.
        assert_eq!(
            resolve_floating_table_overlap_y(
                floating_frame(5, 150.0, 100.0, 120.0, 40.0, false),
                0.0,
                &frames,
            ),
            260.0
        );
        // A single-page table keeps its exact frame.
        let single = floating_table_page_frames(first, 3, 0.0, 96.0, 1000.0);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].bottom, first.bottom);
    }

    #[test]
    fn earlier_never_overlap_policy_also_moves_later_table() {
        let existing = vec![FloatingTableFrame {
            page_number: 1,
            left: 100.0,
            right: 300.0,
            top: 120.0,
            bottom: 180.0,
            prevents_overlap: true,
        }];

        assert_eq!(
            resolve_floating_table_overlap_y(
                floating_frame(1, 150.0, 200.0, 140.0, 50.0, false),
                0.0,
                &existing,
            ),
            180.0
        );
    }

    #[test]
    fn floating_table_fallback_does_not_cover_earlier_logical_content() {
        // Word wraps anchor text around an upward-positioned floating table.
        // Until that wrapping exists, the logical anchor is the safe lower
        // bound: painting above it would cover content already rendered.
        assert_eq!(
            resolve_floating_table_overlap_y(
                floating_frame(1, 100.0, 200.0, 120.0, 50.0, false),
                240.0,
                &[],
            ),
            240.0
        );
    }

    #[test]
    fn bidi_visual_mirrors_cells_with_unequal_column_widths() {
        // The grid offsets describe logical widths 50, 100, and 150. Word
        // places logical column zero at the visual right edge without
        // reassigning its model index, so its physical left edge is 250.
        let positions = [0.0, 50.0, 150.0, 300.0];
        assert_eq!(visual_cell_x(10.0, &positions, 0, 1, true), 260.0);
        assert_eq!(visual_cell_x(10.0, &positions, 1, 2, true), 160.0);
        assert_eq!(visual_cell_x(10.0, &positions, 2, 3, true), 10.0);

        // A normal table continues to use the logical start boundary.
        assert_eq!(visual_cell_x(10.0, &positions, 1, 2, false), 60.0);
    }

    #[test]
    fn bidi_visual_uses_merged_cell_end_boundary_and_mirrors_resize_handles() {
        let positions = [0.0, 50.0, 150.0, 300.0];

        // Logical columns zero and one merge into a 150px cell. Its left edge
        // is derived from logical boundary two, keeping the merged region on
        // the visual right with its exact combined width.
        assert_eq!(visual_cell_x(10.0, &positions, 0, 2, true), 160.0);

        // The handle after logical column zero is the same grid boundary as in
        // the model, but reflected across the table's physical right edge.
        assert_eq!(visual_column_boundary_x(10.0, &positions, 1, true), 260.0);
        assert_eq!(visual_column_boundary_x(10.0, &positions, 1, false), 60.0);
        assert_eq!(visual_right_edge_column_index(3, true), 0);
        assert_eq!(visual_right_edge_column_index(3, false), 2);
    }

    #[test]
    fn split_note_registration_preserves_source_ordinals_without_each_page_metadata() {
        let context = FixedWidthWrapContext::new();
        let source_cell = |id: &str, ordinal, footnote| {
            let mut paragraph =
                layout_horizontal_paragraph_with_context(&context, "x", 20.0, 12.0).unwrap();
            paragraph.note_anchors.push(TableNoteAnchor {
                id: id.to_string(),
                footnote,
                offset: 0,
                ordinal: Some(ordinal),
            });
            let mut cell = make_prepared_cell(0, 1, None, false);
            cell.blocks = vec![CellBlock {
                source_index: 0,
                height: paragraph.height,
                kind: CellBlockKind::Paragraph(paragraph),
                paragraph_indices: vec![0],
            }];
            cell
        };
        for (footnote, restart) in [
            (true, NoteRestart::Continuous),
            (false, NoteRestart::Continuous),
            (true, NoteRestart::EachSection),
            (false, NoteRestart::EachSection),
        ] {
            let mut renderer = super::super::tests::test_renderer();
            renderer.active_section_index = 2;
            if footnote {
                renderer.footnote_number_restart = restart;
            } else {
                renderer.endnote_number_restart = restart;
            }
            let mut markers = NoteMarkers::default();
            // The short right cell's second reference paints before the long
            // left cell's first reference on the following page.
            renderer.register_prepared_row_notes(&[source_cell("2", 2, footnote)], &mut markers);
            renderer.current_page_number = 2;
            renderer.register_prepared_row_notes(&[source_cell("1", 1, footnote)], &mut markers);
            renderer.register_note_reference(footnote, "3");
            let map = if footnote {
                &renderer.footnote_display_map
            } else {
                &renderer.endnote_display_map
            };
            assert_eq!(map.get("1"), Some(&1));
            assert_eq!(map.get("2"), Some(&2));
            assert_eq!(map.get("3"), Some(&3));
        }
        let paragraph = Paragraph {
            runs: vec![Run {
                text: String::new(),
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
                footnote_reference_ids: vec!["1".into()],
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let table = Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks: vec![BodyElement::from_paragraph(paragraph.clone())],
                    ..TableCell::default()
                }],
                ..TableRow::default()
            }],
            ..Table::default()
        };
        let options = Options {
            render_footnotes: true,
            ..Options::default()
        };
        let mut renderer = super::super::tests::test_renderer();
        let provisional = renderer.provisional_table_note_markers(&table, &options);
        assert_eq!(provisional.footnote_ordinals.get("1"), Some(&1));
        let mut actual = NoteMarkers::default();
        renderer.register_block_note_markers(&table.rows[0].cells[0].blocks, &options, &mut actual);
        assert!(actual.footnote_ordinals.is_empty());
        let mut renderer = super::super::tests::test_renderer();
        renderer.footnote_number_restart = NoteRestart::EachPage;
        let provisional = renderer.provisional_table_note_markers(&table, &options);
        assert_eq!(
            provisional.footnotes.get("1").map(String::as_str),
            Some("1")
        );
        assert!(provisional.footnote_ordinals.is_empty());
    }

    #[test]
    fn split_table_paragraph_keeps_shared_lines_and_reference_on_its_tail() {
        let context = FixedWidthWrapContext::new();
        let mut paragraph =
            layout_horizontal_paragraph_with_context(&context, "あいうえおかきく", 20.0, 12.0)
                .unwrap();
        assert_eq!(paragraph.lines.len(), 4);
        paragraph.note_anchors.push(TableNoteAnchor {
            id: "7".into(),
            footnote: true,
            offset: 21,
            ordinal: Some(7),
        });
        let original = paragraph
            .lines
            .iter()
            .map(|line| (line.start, line.end, line.visual_width))
            .collect::<Vec<_>>();
        let measured = context.measure_calls();
        // A textbox anchored after the visible text contributes notes at
        // the source paragraph end, without a marker in the parent's text.
        paragraph.note_anchors.push(TableNoteAnchor {
            id: "8".to_string(),
            footnote: true,
            offset: paragraph.text.len(),
            ordinal: Some(8),
        });
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.padding.top = 0.0;
        cell.padding.bottom = 0.0;
        cell.blocks = vec![CellBlock {
            source_index: 0,
            height: paragraph.height,
            kind: CellBlockKind::Paragraph(paragraph),
            paragraph_indices: vec![42],
        }];
        let split =
            split_prepared_row_for_available_height(&[cell], 26.0, 12.0, 100.0, false).unwrap();
        let ranges = |cells: &[PreparedCell]| {
            let CellBlockKind::Paragraph(paragraph) = &cells[0].blocks[0].kind else {
                panic!("paragraph")
            };
            paragraph
                .lines
                .iter()
                .map(|line| (line.start, line.end, line.visual_width))
                .collect::<Vec<_>>()
        };
        assert_eq!(ranges(&split.head_cells), original[..2]);
        assert_eq!(ranges(&split.tail_cells), original[2..]);
        assert!(prepared_row_note_anchors(&split.head_cells).is_empty());
        assert_eq!(
            prepared_row_note_anchors(&split.tail_cells)
                .iter()
                .map(|anchor| anchor.id.as_str())
                .collect::<Vec<_>>(),
            vec!["7", "8"]
        );
        assert_eq!(block_indices(&split.head_cells[0]), vec![vec![42]]);
        assert_eq!(block_indices(&split.tail_cells[0]), vec![vec![42]]);
        assert_eq!(
            context.measure_calls(),
            measured,
            "splitting reuses shared glyph metrics"
        );
        assert_eq!(split.head_height, 25.0);
        assert_eq!(split.tail_height, 26.0);
    }

    #[test]
    fn split_table_paragraph_obeys_keep_lines_and_shared_widow_policy() {
        let context = FixedWidthWrapContext::new();
        let mut paragraph =
            layout_horizontal_paragraph_with_context(&context, "あいうえおかきく", 20.0, 12.0)
                .unwrap();
        paragraph.widow_control = true;
        let make = |paragraph: CellParagraph| CellBlock {
            source_index: 0,
            height: paragraph.height,
            kind: CellBlockKind::Paragraph(paragraph),
            paragraph_indices: vec![0],
        };
        let (head, tail) = split_cell_paragraph(&make(paragraph.clone()), 36.0, 100.0).unwrap();
        assert_eq!(head.height, 24.0);
        assert_eq!(tail.height, 24.0);
        assert!(split_cell_paragraph(&make(paragraph.clone()), 12.0, 100.0).is_none());
        paragraph.widow_control = false;
        assert_eq!(
            split_cell_paragraph(&make(paragraph.clone()), 36.0, 100.0)
                .unwrap()
                .0
                .height,
            36.0
        );
        paragraph.keep_lines = true;
        assert!(split_cell_paragraph(&make(paragraph.clone()), 36.0, 100.0).is_none());
        assert!(
            split_cell_paragraph(&make(paragraph), 36.0, 36.0).is_some(),
            "an oversized paragraph can continue"
        );
    }

    #[test]
    fn split_prepared_row_splits_at_block_boundaries() {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.blocks = vec![
            make_cell_block(8.0),
            make_cell_block(8.0),
            make_cell_block(8.0),
        ];
        cell.height = 32.0;
        cell.logical_paragraph_count = 3;
        let row = vec![cell];

        let split = split_prepared_row_for_available_height(&row, 28.0, 12.0, f64::INFINITY, false)
            .expect("row should split");

        assert_eq!(split.head_cells.len(), 1);
        assert_eq!(split.tail_cells.len(), 1);
        assert_eq!(split.head_cells[0].blocks.len(), 2);
        assert_eq!(split.tail_cells[0].blocks.len(), 1);
        assert!((split.head_height - 25.0).abs() < 0.01);
        assert!((split.tail_height - 18.0).abs() < 0.01);
    }

    #[test]
    fn split_prepared_row_uses_resolved_zero_cell_padding() {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.padding.top = 0.0;
        cell.padding.bottom = 0.0;
        cell.blocks = vec![
            make_cell_block(19.2),
            make_cell_block(19.2),
            make_cell_block(19.2),
            make_cell_block(19.2),
        ];
        cell.height = 76.8;
        let row = vec![cell];

        let split = split_prepared_row_for_available_height(&row, 64.0, 19.2, f64::INFINITY, false)
            .expect("row should split after the third zero-margin paragraph");

        assert_eq!(split.head_cells[0].blocks.len(), 3);
        assert_eq!(split.tail_cells[0].blocks.len(), 1);
        assert!((split.head_height - 58.6).abs() < 0.01);
    }

    #[test]
    fn split_prepared_row_returns_none_when_first_block_cannot_fit() {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.blocks = vec![make_cell_block(24.0), make_cell_block(6.0)];
        cell.height = 38.0;
        let row = vec![cell];

        let split = split_prepared_row_for_available_height(&row, 28.0, 12.0, f64::INFINITY, false);
        assert!(split.is_none());
    }

    #[test]
    fn column_resize_boundaries_exclude_horizontal_merge_interiors() {
        let cells = vec![
            make_prepared_cell(0, 2, None, false),
            make_prepared_cell(2, 1, None, false),
            make_prepared_cell(3, 3, None, true),
        ];
        for boundary in 0..=6 {
            assert_eq!(
                column_boundary_crosses_merged_cell(&cells, boundary),
                matches!(boundary, 1 | 4 | 5),
                "boundary {boundary}"
            );
        }
    }

    #[test]
    fn table_paragraph_indents_use_shared_wrap_and_line_origins() {
        let config = LayoutConfig {
            padding_x: 0.0,
            padding_y: 0.0,
            line_height: 16.0,
            paragraph_line_heights: FxHashMap::default(),
            paragraph_east_asian_rules: FxHashMap::default(),
            document_uses_east_asian_layout: false,
            word_modern_border_insets: false,
            available_width: 120.0,
            render_changes: false,
            render_original: false,
            note_markers: NoteMarkers::default(),
            doc_grid: None,
            styles_part: None,
            document_snap_to_grid_default: false,
        };
        let paragraph = Paragraph {
            runs: Vec::new(),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
            properties: Some(ParagraphProperties {
                indent: Some(jade_docx_reader::Indent {
                    left: Some(12.0),
                    right: Some(6.0),
                    hanging: Some(6.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        let (left, style) = config.paragraph_style(&paragraph);
        assert_eq!(left, 16.0);
        assert_eq!(style.first_line_indent, -8.0);
        assert_eq!(style.right_indent, 8.0);
        let context = FixedWidthWrapContext::new();
        let text = "あいうえおかきくけこさしすせそたちつてとなにぬねの";
        let mut layout = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            text,
            120.0 - left,
            16.0,
            &styled_run_styles(text, &[]),
            &style,
        )
        .unwrap();
        let empty = layout_horizontal_paragraph_with_run_styles_context(
            &context,
            "",
            120.0 - left,
            16.0,
            &[],
            &style,
        )
        .unwrap();
        assert_eq!(empty.lines[0].left_inset, -8.0);
        assert_eq!(empty.lines[0].right_inset, 8.0);
        assert!(layout.lines.len() > 2);
        for (index, line) in Rc::make_mut(&mut layout.lines).iter_mut().enumerate() {
            line.left_inset += left;
            assert_eq!(line.left_inset, if index == 0 { 8.0 } else { 16.0 });
            assert_eq!(line.right_inset, 8.0);
            assert!(line.visual_width <= 120.0 - line.left_inset - line.right_inset);
            assert_eq!(
                line.aligned_start_x(10.0, 120.0, TextAlign::Left),
                10.0 + line.left_inset
            );
            assert!(
                (line.aligned_start_x(10.0, 120.0, TextAlign::Right) + line.visual_width - 122.0)
                    .abs()
                    < 0.001
            );
            let center =
                line.aligned_start_x(10.0, 120.0, TextAlign::Center) + line.visual_width / 2.0;
            assert!((center - (70.0 + (line.left_inset - line.right_inset) / 2.0)).abs() < 0.001);
        }
    }

    #[test]
    fn vertical_paragraph_inline_alignment_does_not_move_the_column_block_left() {
        let paragraph = Paragraph {
            properties: Some(crate::document::ParagraphProperties {
                alignment: Some("left".into()),
                ..Default::default()
            }),
            runs: Vec::new(),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        for direction in [
            TextDirection::TbRl,
            TextDirection::TbRlV,
            TextDirection::BtLr,
        ] {
            // The original vertical fixture's inherited left/start paragraph
            // alignment is along the inline Y axis. Its columns still start
            // at the cell's right block edge, as the Word PDF demonstrates.
            assert_eq!(
                paragraph_text_align_for_direction(&paragraph, Some(direction)),
                None
            );
        }
        assert_eq!(
            paragraph_text_align_for_direction(&paragraph, None),
            Some(TextAlign::Left)
        );
        assert_eq!(
            paragraph_text_align_for_direction(&paragraph, Some(TextDirection::LrTbV)),
            Some(TextAlign::Left)
        );
    }

    #[test]
    fn paragraph_alignment_overrides_cell_defaults_and_resolves_logical_direction() {
        let mut paragraph = Paragraph {
            runs: Vec::new(),
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        assert_eq!(paragraph_text_align(&paragraph), None);
        for (token, bidi, expected) in [
            ("left", false, TextAlign::Left),
            ("center", false, TextAlign::Center),
            ("right", false, TextAlign::Right),
            ("start", false, TextAlign::Left),
            ("end", false, TextAlign::Right),
            ("start", true, TextAlign::Right),
            ("end", true, TextAlign::Left),
            ("right", true, TextAlign::Right),
        ] {
            paragraph.properties = Some(ParagraphProperties {
                alignment: Some(token.into()),
                bidi: Some(bidi),
                ..Default::default()
            });
            assert_eq!(
                paragraph_text_align(&paragraph),
                Some(expected),
                "{token}/{bidi}"
            );
        }
        paragraph.properties.as_mut().unwrap().alignment = None;
        assert_eq!(paragraph_text_align(&paragraph), Some(TextAlign::Right));
    }

    #[test]
    fn split_row_reserves_visible_bottom_stroke_separately_from_text_spacing() {
        for (stroke, expected_head_lines) in [(0.0, 3), (1.0, 2)] {
            let mut paragraph = empty_cell_paragraph(12.0);
            paragraph.text = "abcdef".into();
            paragraph.lines = Rc::new(
                (0..6)
                    .map(|index| {
                        let mut line = paragraph.lines[0].clone();
                        line.text = paragraph.text[index..index + 1].into();
                        line.start = index;
                        line.end = index + 1;
                        line
                    })
                    .collect(),
            );
            paragraph.height = 72.0;
            let mut cell = make_prepared_cell(0, 1, None, false);
            cell.border_top = CellBorderSpec::hidden();
            cell.border_bottom = if stroke == 0.0 {
                CellBorderSpec::hidden()
            } else {
                let mut border = CellBorderSpec::single_default();
                border.width_px = stroke;
                border
            };
            cell.blocks = vec![CellBlock {
                source_index: 0,
                height: 72.0,
                kind: CellBlockKind::Paragraph(paragraph),
                paragraph_indices: vec![17],
            }];
            cell.height = 80.0;
            let split =
                split_prepared_row_for_available_height(&[cell], 44.0, 12.0, 800.0, false).unwrap();
            let CellBlockKind::Paragraph(head) = &split.head_cells[0].blocks[0].kind else {
                panic!()
            };
            let CellBlockKind::Paragraph(tail) = &split.tail_cells[0].blocks[0].kind else {
                panic!()
            };
            assert_eq!(head.lines.len(), expected_head_lines);
            assert_eq!(head.lines.len() + tail.lines.len(), 6);
            assert_eq!(
                head.lines
                    .iter()
                    .chain(tail.lines.iter())
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "abcdef"
            );
            assert!(split.head_height <= 44.0);
            assert_eq!(split.head_cells[0].blocks[0].paragraph_indices, [17]);
            assert_eq!(split.tail_cells[0].blocks[0].paragraph_indices, [17]);
        }
    }

    #[test]
    fn split_row_partitions_shared_paragraph_lines_and_preserves_model_offsets() {
        for direction in [None, Some(TextDirection::LrTbV)] {
            let mut paragraph = empty_cell_paragraph(12.0);
            paragraph.align = Some(TextAlign::Right);
            paragraph.text = "abcdef".into();
            paragraph.lines = (0..6)
                .map(|index| {
                    let mut line = paragraph.lines[0].clone();
                    line.text = paragraph.text[index..index + 1].into();
                    line.left_inset = if index == 0 { 8.0 } else { 16.0 };
                    line.right_inset = 8.0;
                    line.start = index;
                    line.end = index + 1;
                    line
                })
                .collect::<Vec<_>>()
                .into();
            paragraph.height = 72.0;
            let mut cell = make_prepared_cell(0, 1, None, false);
            // This test isolates text/margin budgets; visible borders have
            // a separate reservation test below.
            cell.border_top = CellBorderSpec::hidden();
            cell.border_bottom = CellBorderSpec::hidden();
            cell.text_direction = direction;
            cell.blocks = vec![CellBlock {
                source_index: 0,
                kind: CellBlockKind::Paragraph(paragraph),
                height: 72.0,
                paragraph_indices: vec![17],
            }];
            cell.height = 80.0;
            let split = split_prepared_row_for_available_height(&[cell], 44.0, 12.0, 800.0, false)
                .expect("a paragraph taller than the remainder splits at a shared line");
            let head = &split.head_cells[0].blocks[0];
            let tail = &split.tail_cells[0].blocks[0];
            assert_eq!(head.paragraph_indices, [17]);
            assert_eq!(tail.paragraph_indices, [17]);
            let CellBlockKind::Paragraph(head) = &head.kind else {
                panic!()
            };
            let CellBlockKind::Paragraph(tail) = &tail.kind else {
                panic!()
            };
            assert_eq!(
                head.lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "abc"
            );
            assert_eq!(
                tail.lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "def"
            );
            assert_eq!(tail.lines[0].start, 3);
            assert_eq!(head.lines[0].left_inset, 8.0);
            assert_eq!(tail.lines[0].left_inset, 16.0);
            assert_eq!(tail.lines[0].right_inset, 8.0);
            assert_eq!(head.text, "abcdef");
            assert_eq!(tail.text, "abcdef");
            assert!(!head.is_continuation);
            assert!(tail.is_continuation);
            assert_eq!(head.align, Some(TextAlign::Right));
            assert_eq!(tail.align, Some(TextAlign::Right));
            assert_eq!(split.head_height, 44.0);
            assert_eq!(split.tail_height, 44.0);
            let again = split_prepared_row_for_available_height(
                &split.tail_cells,
                32.0,
                12.0,
                800.0,
                false,
            )
            .expect("a continuation can split again");
            let CellBlockKind::Paragraph(last) = &again.tail_cells[0].blocks[0].kind else {
                panic!()
            };
            assert_eq!(last.lines[0].start, 5);
            assert!(last.is_continuation);
            assert_eq!(last.align, Some(TextAlign::Right));
            assert_eq!(again.tail_cells[0].blocks[0].paragraph_indices, [17]);
        }
    }

    #[test]
    fn split_row_keeps_spacing_only_on_first_and_last_fragments() {
        for direction in [None, Some(TextDirection::LrTbV)] {
            let mut paragraph = empty_cell_paragraph(12.0);
            paragraph.align = Some(TextAlign::Right);
            paragraph.text = "abcdef".into();
            paragraph.lines = (0..6)
                .map(|index| {
                    let mut line = paragraph.lines[0].clone();
                    line.text = paragraph.text[index..index + 1].into();
                    line.left_inset = if index == 0 { 8.0 } else { 16.0 };
                    line.right_inset = 8.0;
                    line.start = index;
                    line.end = index + 1;
                    line
                })
                .collect::<Vec<_>>()
                .into();
            paragraph.height = 72.0;
            paragraph.spacing_before = 8.0;
            paragraph.spacing_after = 16.0;
            let mut cell = make_prepared_cell(0, 1, None, false);
            // This test isolates text/margin budgets; visible borders have
            // a separate reservation test below.
            cell.border_top = CellBorderSpec::hidden();
            cell.border_bottom = CellBorderSpec::hidden();
            cell.text_direction = direction;
            cell.blocks = vec![CellBlock {
                source_index: 0,
                kind: CellBlockKind::Paragraph(paragraph),
                height: 96.0,
                paragraph_indices: vec![17],
            }];
            cell.height = 104.0;
            let split = split_prepared_row_for_available_height(&[cell], 44.0, 12.0, 800.0, false)
                .expect("a paragraph taller than the remainder splits at a shared line");
            let head = &split.head_cells[0].blocks[0];
            let tail = &split.tail_cells[0].blocks[0];
            assert_eq!(head.paragraph_indices, [17]);
            assert_eq!(tail.paragraph_indices, [17]);
            let CellBlockKind::Paragraph(head) = &head.kind else {
                panic!()
            };
            let CellBlockKind::Paragraph(tail) = &tail.kind else {
                panic!()
            };
            assert_eq!(
                head.lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "ab"
            );
            assert_eq!(
                tail.lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "cdef"
            );
            assert_eq!(tail.lines[0].start, 2);
            assert_eq!(head.lines[0].left_inset, 8.0);
            assert_eq!(tail.lines[0].left_inset, 16.0);
            assert_eq!(tail.lines[0].right_inset, 8.0);
            assert_eq!(head.spacing_before, 8.0);
            assert_eq!(head.spacing_after, 0.0);
            assert_eq!(tail.spacing_before, 0.0);
            assert_eq!(tail.spacing_after, 16.0);
            assert_eq!(head.text, "abcdef");
            assert_eq!(tail.text, "abcdef");
            assert!(!head.is_continuation);
            assert!(tail.is_continuation);
            assert_eq!(head.align, Some(TextAlign::Right));
            assert_eq!(tail.align, Some(TextAlign::Right));
            assert_eq!(split.head_height, 40.0);
            assert_eq!(split.tail_height, 72.0);
            let again = split_prepared_row_for_available_height(
                &split.tail_cells,
                32.0,
                12.0,
                800.0,
                false,
            )
            .expect("a continuation can split again");
            let CellBlockKind::Paragraph(last) = &again.tail_cells[0].blocks[0].kind else {
                panic!()
            };
            assert_eq!(last.lines[0].start, 4);
            assert!(last.is_continuation);
            assert_eq!(last.align, Some(TextAlign::Right));
            assert_eq!(again.tail_cells[0].blocks[0].paragraph_indices, [17]);
        }
    }

    #[test]
    fn split_row_keeps_fitting_text_when_only_after_spacing_overflows() {
        for direction in [None, Some(TextDirection::LrTbV)] {
            let mut paragraph = empty_cell_paragraph(12.0);
            paragraph.text = "text".into();
            paragraph.spacing_before = 8.0;
            paragraph.spacing_after = 60.0;
            let mut cell = make_prepared_cell(0, 1, None, false);
            // This test isolates text/margin budgets; visible borders have
            // a separate reservation test below.
            cell.border_top = CellBorderSpec::hidden();
            cell.border_bottom = CellBorderSpec::hidden();
            cell.text_direction = direction;
            cell.blocks = vec![CellBlock {
                source_index: 0,
                kind: CellBlockKind::Paragraph(paragraph),
                height: 80.0,
                paragraph_indices: vec![17],
            }];
            cell.height = 88.0;
            let split = split_prepared_row_for_available_height(&[cell], 36.0, 12.0, 800.0, false)
                .expect("only the margin must continue");
            let head = &split.head_cells[0].blocks[0];
            let tail = &split.tail_cells[0].blocks[0];
            let CellBlockKind::Paragraph(paragraph) = &head.kind else {
                panic!()
            };
            assert_eq!(paragraph.text, "text");
            assert_eq!(paragraph.lines.len(), 1);
            assert_eq!(paragraph.spacing_after, 8.0);
            assert_eq!(head.paragraph_indices, [17]);
            assert!(matches!(tail.kind, CellBlockKind::Spacing));
            assert!(tail.paragraph_indices.is_empty());
            assert_eq!(head.height + tail.height, 80.0);
            assert_eq!(split.head_height, 36.0);
            let again = split_prepared_row_for_available_height(
                &split.tail_cells,
                32.0,
                12.0,
                800.0,
                false,
            )
            .expect("a large margin can continue again without creating text");
            assert!(matches!(
                again.head_cells[0].blocks[0].kind,
                CellBlockKind::Spacing
            ));
            assert!(matches!(
                again.tail_cells[0].blocks[0].kind,
                CellBlockKind::Spacing
            ));
            assert_eq!(
                again.head_cells[0].blocks[0].height + again.tail_cells[0].blocks[0].height,
                tail.height
            );
        }
    }

    #[test]
    fn explicit_row_height_only_blocks_content_split_when_height_requires_it() {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.height = 80.0;
        let cells = [cell];
        for (rule, height, blocked) in [
            (TableRowHeightRule::AtLeast, 12.0, false),
            (TableRowHeightRule::AtLeast, 60.0, false),
            (TableRowHeightRule::AtLeast, 61.0, true),
            (TableRowHeightRule::Exact, 12.0, true),
            (TableRowHeightRule::Auto, 120.0, false),
        ] {
            let row = TableRow {
                height: Some(height),
                height_rule: Some(rule),
                ..TableRow::default()
            };
            assert_eq!(row_height_blocks_content_split(&row, &cells), blocked);
        }
    }

    #[test]
    fn oversized_nested_vertical_merge_owner_splits_without_losing_paragraphs() {
        let mut nested = make_empty_nested_table_layout();
        let mut owner = make_prepared_cell(0, 2, Some("restart"), false);
        owner.blocks = (0..10).map(|_| make_cell_block(8.0)).collect();
        owner.logical_paragraph_count = 10;
        owner.height = 88.0;
        nested.rows.push(NestedRowLayout {
            source_index: 0,
            height: 88.0,
            cells: vec![owner],
            hidden: false,
        });
        nested.rows.push(NestedRowLayout {
            source_index: 1,
            height: 16.0,
            cells: vec![make_prepared_cell(0, 2, Some("continue"), true)],
            hidden: false,
        });
        nested.table.rows = vec![TableRow::default(); 2];
        nested.height = 104.0;
        assign_row_paragraph_indices(&mut nested.rows[0].cells, 30, &HashMap::new());
        let (head, tail) = split_nested_table_for_available_height(&nested, 40.0, 100.0)
            .expect("vertical merge owner must split when its content exceeds available space");
        let (second, third) = split_nested_table_for_available_height(&tail, 40.0, 100.0)
            .expect("vertical merge tail must support another page boundary");
        let mut indices = Vec::new();
        for fragment in [&head, &second, &third] {
            let cell = &fragment.rows[0].cells[0];
            assert_eq!(cell.vertical_merge.as_deref(), Some("restart"));
            assert_eq!(cell.span, 2);
            indices.extend(collect_cell_paragraph_indices(cell));
        }
        assert_eq!(indices, (30..40).collect::<Vec<_>>());
        assert!(third.rows[1].cells[0].skip_render);
    }

    #[test]
    fn nested_minimum_height_row_can_split_without_losing_source_identity() {
        let mut nested = make_empty_nested_table_layout();
        let mut cell = make_prepared_cell(0, 1, None, false);
        // This test isolates text/margin budgets; visible borders have
        // a separate reservation test below.
        cell.border_top = CellBorderSpec::hidden();
        cell.border_bottom = CellBorderSpec::hidden();
        cell.blocks = (0..10).map(|_| make_cell_block(8.0)).collect();
        cell.logical_paragraph_count = 10;
        cell.height = 88.0;
        nested.rows.push(NestedRowLayout {
            source_index: 0,
            height: 88.0,
            cells: vec![cell],
            hidden: false,
        });
        nested.table.rows.push(TableRow {
            height: Some(12.0),
            height_rule: Some(TableRowHeightRule::AtLeast),
            ..TableRow::default()
        });
        nested.height = 88.0;
        assign_row_paragraph_indices(&mut nested.rows[0].cells, 30, &HashMap::new());
        let (head, tail) = split_nested_table_for_available_height(&nested, 40.0, 100.0)
            .expect("content-driven minimum row should split");
        assert_eq!(head.rows[0].cells[0].blocks.len(), 4);
        assert_eq!(tail.rows[0].cells[0].blocks.len(), 6);
        assert_eq!(tail.rows[0].source_index, 0);
        assert_eq!(
            collect_cell_paragraph_indices(&tail.rows[0].cells[0]),
            vec![34, 35, 36, 37, 38, 39]
        );
        nested.table.rows[0].height_rule = Some(TableRowHeightRule::Exact);
        assert!(split_nested_table_for_available_height(&nested, 40.0, 100.0).is_none());
    }

    #[test]
    fn nested_cant_split_uses_full_page_body_budget_including_ancestor_insets() {
        for (block_count, should_split) in [(4, false), (9, false), (10, true)] {
            let mut nested = make_empty_nested_table_layout();
            for (source_index, count) in [1, block_count].into_iter().enumerate() {
                let mut cell = make_prepared_cell(0, 1, None, false);
                // This test isolates text/margin budgets; visible borders have
                // a separate reservation test below.
                cell.border_top = CellBorderSpec::hidden();
                cell.border_bottom = CellBorderSpec::hidden();
                cell.blocks = (0..count).map(|_| make_cell_block(8.0)).collect();
                cell.logical_paragraph_count = count;
                cell.height = count as f64 * 8.0 + 8.0;
                nested.rows.push(NestedRowLayout {
                    source_index,
                    height: cell.height,
                    cells: vec![cell],
                    hidden: false,
                });
                nested.table.rows.push(TableRow {
                    is_header: source_index == 0,
                    cant_split: source_index == 1,
                    ..TableRow::default()
                });
            }
            nested.height = nested.rows.iter().map(|row| row.height).sum();
            let mut outer = make_prepared_cell(0, 1, None, false);
            outer.border_top = CellBorderSpec::hidden();
            outer.border_bottom = CellBorderSpec::hidden();
            outer.blocks = vec![CellBlock {
                source_index: 0,
                height: nested.height,
                kind: CellBlockKind::Table(nested),
                paragraph_indices: Vec::new(),
            }];
            outer.height = outer.blocks[0].height + 8.0;
            let mut cells = vec![outer];
            assign_row_paragraph_indices(&mut cells, 20, &HashMap::new());
            // Full page 88 - outer padding 8 = body 80; nested headers do not repeat.
            // Current remainder 48 leaves only 24px for body content.
            let split = split_prepared_row_for_available_height(&cells, 48.0, 12.0, 88.0, false);
            assert_eq!(split.is_some(), should_split, "{block_count} blocks");
            if let Some(split) = split {
                let CellBlockKind::Table(head) = &split.head_cells[0].blocks[0].kind else {
                    panic!()
                };
                let CellBlockKind::Table(tail) = &split.tail_cells[0].blocks[0].kind else {
                    panic!()
                };
                assert_eq!(head.rows[1].cells[0].blocks.len(), 2);
                assert_eq!(tail.rows[0].cells[0].blocks.len(), 8);
                assert_eq!(tail.rows[0].source_index, 1);
                assert_eq!(
                    collect_cell_paragraph_indices(&tail.rows[0].cells[0]),
                    vec![23, 24, 25, 26, 27, 28, 29, 30]
                );
            }
        }
    }

    fn block_indices(cell: &PreparedCell) -> Vec<Vec<usize>> {
        cell.blocks
            .iter()
            .map(|block| block.paragraph_indices.clone())
            .collect()
    }

    #[test]
    fn recursive_row_split_preserves_nested_indices_headers_and_source_rows() {
        let mut nested = make_empty_nested_table_layout();
        for index in 0..5 {
            let mut cell = make_prepared_cell(0, 1, None, false);
            cell.blocks = vec![make_cell_block(8.0)];
            cell.height = 16.0;
            cell.logical_paragraph_count = 1;
            nested.rows.push(NestedRowLayout {
                source_index: index,
                cells: vec![cell],
                height: 16.0,
                hidden: false,
            });
            nested.table.rows.push(TableRow {
                is_header: index == 0,
                ..TableRow::default()
            });
        }
        nested.height = 80.0;
        let mut left = make_prepared_cell(0, 1, None, false);
        left.blocks = vec![CellBlock {
            source_index: 0,
            kind: CellBlockKind::Table(nested),
            height: 80.0,
            paragraph_indices: Vec::new(),
        }];
        left.height = 88.0;
        let mut right = make_prepared_cell(1, 1, None, false);
        right.blocks = vec![make_cell_block(8.0)];
        right.height = 16.0;
        let mut cells = vec![left, right];
        assert_eq!(
            assign_row_paragraph_indices(&mut cells, 10, &HashMap::new()),
            16
        );
        let split =
            split_prepared_row_for_available_height(&cells, 44.0, 12.0, 800.0, false).unwrap();
        assert_eq!(
            collect_cell_paragraph_indices(&split.head_cells[1]),
            vec![15]
        );
        let CellBlockKind::Table(head) = &split.head_cells[0].blocks[0].kind else {
            panic!()
        };
        let CellBlockKind::Table(tail) = &split.tail_cells[0].blocks[0].kind else {
            panic!()
        };
        assert_eq!(head.rows.len(), 2);
        assert_eq!(
            tail.rows.iter().map(|r| r.source_index).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(
            collect_cell_paragraph_indices(&tail.rows[0].cells[0]),
            vec![12]
        );
        assert_eq!(tail.height, 48.0);
        let (next_head, next_tail) =
            split_nested_table_for_available_height(tail, 33.0, 800.0).unwrap();
        assert_eq!(next_head.rows[0].source_index, 2);
        assert_eq!(next_tail.rows[0].source_index, 4);
        assert_eq!(
            collect_cell_paragraph_indices(&next_tail.rows[0].cells[0]),
            vec![14]
        );
    }

    #[test]
    fn nested_table_orders_follow_model_preorder_before_page_splitting() {
        let block = |layout: NestedTableLayout| CellBlock {
            source_index: 0,
            height: layout.height,
            kind: CellBlockKind::Table(layout),
            paragraph_indices: Vec::new(),
        };
        let mut inner_cell = make_prepared_cell(0, 1, None, false);
        inner_cell.blocks = vec![block(make_empty_nested_table_layout())];
        let mut left_table = make_empty_nested_table_layout();
        left_table.rows.push(NestedRowLayout {
            source_index: 0,
            cells: vec![inner_cell],
            height: 16.0,
            hidden: false,
        });
        let mut left = make_prepared_cell(0, 1, None, false);
        left.blocks = vec![block(left_table)];
        let mut right = make_prepared_cell(1, 1, None, false);
        right.blocks = vec![block(make_empty_nested_table_layout())];
        let mut cells = vec![left, right];
        let mut next = 1;
        assign_nested_table_orders(&mut cells, &mut next);
        let CellBlockKind::Table(left) = &cells[0].blocks[0].kind else {
            panic!()
        };
        let CellBlockKind::Table(deep) = &left.rows[0].cells[0].blocks[0].kind else {
            panic!()
        };
        let CellBlockKind::Table(right) = &cells[1].blocks[0].kind else {
            panic!()
        };
        assert_eq!(
            (left.order, deep.order, right.order, next),
            (Some(1), Some(2), Some(3), 4)
        );
        assert_eq!(left.clone().order, Some(1));
    }

    #[test]
    fn nested_vertical_merge_continuation_keeps_owner_style_without_text() {
        let mut layout = make_empty_nested_table_layout();
        for source_index in 0..5 {
            let mut cell = make_prepared_cell(
                0,
                1,
                Some(if source_index == 0 {
                    "restart"
                } else {
                    "continue"
                }),
                source_index != 0,
            );
            if source_index == 0 {
                cell.blocks = vec![make_cell_block(8.0)];
                cell.blocks[0].paragraph_indices = vec![7];
                cell.logical_paragraph_count = 1;
                cell.padding.left = 13.0;
            }
            layout.rows.push(NestedRowLayout {
                source_index,
                cells: vec![cell],
                height: 16.0,
                hidden: false,
            });
            layout.table.rows.push(TableRow::default());
        }
        layout.height = 80.0;
        let (head, tail) = split_nested_table_for_available_height(&layout, 33.0, 800.0).unwrap();
        assert_eq!(head.rows.len(), 2);
        let owner = &tail.rows[0].cells[0];
        assert!(!owner.skip_render);
        assert_eq!(owner.vertical_merge.as_deref(), Some("restart"));
        assert_eq!(owner.padding.left, 13.0);
        assert!(owner.blocks.is_empty());
        assert_eq!(owner.logical_paragraph_count, 0);
        assert!(collect_cell_paragraph_indices(owner).is_empty());
        let (second, third) = split_nested_table_for_available_height(&tail, 17.0, 800.0).unwrap();
        assert_eq!(second.rows[0].source_index, 2);
        assert_eq!(third.rows[0].source_index, 3);
        assert!(!third.rows[0].cells[0].skip_render);
        assert!(third.rows[0].cells[0].blocks.is_empty());
    }

    #[test]
    fn assign_row_paragraph_indices_numbers_cells_in_document_order() {
        let mut left = make_prepared_cell(0, 1, None, false);
        left.blocks = vec![
            make_cell_block(8.0),
            make_cell_block(8.0),
            make_cell_block(8.0),
            make_cell_block(8.0),
        ];
        left.logical_paragraph_count = 4;
        let mut right = make_prepared_cell(1, 1, None, false);
        right.blocks = vec![make_cell_block(8.0)];
        right.logical_paragraph_count = 1;

        let mut row = vec![left, right];
        let next = assign_row_paragraph_indices(&mut row, 10, &HashMap::new());

        assert_eq!(next, 15);
        assert_eq!(
            block_indices(&row[0]),
            vec![vec![10], vec![11], vec![12], vec![13]]
        );
        // The right-hand cell is numbered *after* the entire left cell, matching
        // `count_body_paragraphs`' cell-by-cell walk.
        assert_eq!(block_indices(&row[1]), vec![vec![14]]);
    }

    #[test]
    fn assign_row_paragraph_indices_reserves_skip_render_cells() {
        let mut merged = make_prepared_cell(0, 1, Some("continue"), true);
        merged.logical_paragraph_count = 2;
        let mut body = make_prepared_cell(1, 1, None, false);
        body.blocks = vec![make_cell_block(8.0)];
        body.logical_paragraph_count = 1;

        let mut row = vec![merged, body];
        let next = assign_row_paragraph_indices(&mut row, 0, &HashMap::new());

        // The skip_render cell reserves its two paragraphs without storing
        // per-block indices, so the body cell that follows is numbered 2.
        assert_eq!(next, 3);
        assert!(row[0].blocks.is_empty());
        assert_eq!(block_indices(&row[1]), vec![vec![2]]);
    }

    #[test]
    fn assign_row_paragraph_indices_reserves_hidden_end_marker_paragraphs() {
        let mut hidden = make_prepared_cell(0, 1, None, false);
        hidden.logical_paragraph_count = 2;
        hidden.hidden_end_marker_paragraph_count = 2;
        let mut body = make_prepared_cell(1, 1, None, false);
        body.blocks = vec![make_cell_block(8.0)];
        body.logical_paragraph_count = 1;

        let mut row = vec![hidden, body];
        let next = assign_row_paragraph_indices(&mut row, 0, &HashMap::new());

        assert_eq!(next, 3);
        assert!(row[0].blocks.is_empty());
        assert_eq!(block_indices(&row[1]), vec![vec![2]]);
    }

    #[test]
    fn collect_cell_paragraph_indices_flattens_blocks_in_order() {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.blocks = vec![make_cell_block(8.0), make_cell_block(8.0)];
        assign_row_paragraph_indices(std::slice::from_mut(&mut cell), 7, &HashMap::new());
        assert_eq!(collect_cell_paragraph_indices(&cell), vec![7, 8]);
    }

    #[test]
    fn split_row_preserves_document_order_paragraph_indices() {
        // Reproduces the page-spanning bug: a two-column row whose left cell has
        // four paragraphs (tall enough to overflow the page) and whose right
        // cell holds a single short paragraph. Pre-assigning indices in document
        // order *before* the split keeps the right cell numbered after the left
        // cell's continuation paragraphs, even though the right cell renders in
        // the head segment alongside the left cell's first two paragraphs.
        let mut left = make_prepared_cell(0, 1, None, false);
        left.blocks = vec![
            make_cell_block(8.0),
            make_cell_block(8.0),
            make_cell_block(8.0),
            make_cell_block(8.0),
        ];
        left.height = 40.0;
        left.logical_paragraph_count = 4;
        let mut right = make_prepared_cell(1, 1, None, false);
        right.blocks = vec![make_cell_block(8.0)];
        right.height = 16.0;
        right.logical_paragraph_count = 1;

        let mut row = vec![left, right];
        let next = assign_row_paragraph_indices(&mut row, 0, &HashMap::new());
        assert_eq!(next, 5);

        // available_height 28 with 4px padding on each side leaves a 20px content
        // budget: the left cell fits two 8px blocks (16px) but not three (24px).
        let split = split_prepared_row_for_available_height(&row, 28.0, 12.0, f64::INFINITY, false)
            .expect("row should split");

        // Head: left cell's first two paragraphs (0, 1) plus the right cell (4).
        assert_eq!(block_indices(&split.head_cells[0]), vec![vec![0], vec![1]]);
        assert_eq!(block_indices(&split.head_cells[1]), vec![vec![4]]);

        // Tail: left cell's continuation paragraphs (2, 3); right cell is empty.
        assert_eq!(block_indices(&split.tail_cells[0]), vec![vec![2], vec![3]]);
        assert!(split.tail_cells[1].blocks.is_empty());

        // The killer assertion: the right cell keeps index 4. The pre-fix lazy
        // path would have numbered it 2 (rendered in the head before the left
        // cell's tail), desyncing every later caret/dirty mapping.
        assert_eq!(
            collect_cell_paragraph_indices(&split.head_cells[1]),
            vec![4]
        );
    }

    /// Table borders are the part of a table that a non-canvas backend has to
    /// reproduce exactly. Going through `Painter` means their geometry is now
    /// assertable without a browser.
    #[test]
    fn hairline_borders_snap_to_the_pixel_grid() {
        let painter = RecordingPainter::new();

        // A 1px line drawn on an integer coordinate straddles two pixel rows and
        // renders as a 2px blur, so hairlines are snapped to a half-pixel centre.
        <CanvasRenderer>::stroke_pixel_snapped_segment(&painter, 10.2, 20.4, 90.7, 20.4, 1.0);

        let ops = painter.ops();
        assert!(ops.contains(&Op::MoveTo(10.0, 20.5)), "{ops:?}");
        assert!(ops.contains(&Op::LineTo(91.0, 20.5)), "{ops:?}");
    }

    #[test]
    fn thick_borders_keep_their_exact_coordinates() {
        let painter = RecordingPainter::new();

        // Snapping only helps a hairline. A thick border covers whole pixels
        // either way, and moving it would misalign it with the cell edge.
        <CanvasRenderer>::stroke_pixel_snapped_segment(&painter, 10.2, 20.4, 90.7, 20.4, 3.0);

        let ops = painter.ops();
        assert!(ops.contains(&Op::MoveTo(10.2, 20.4)), "{ops:?}");
        assert!(ops.contains(&Op::LineTo(90.7, 20.4)), "{ops:?}");
    }

    #[test]
    fn a_parallel_stroke_is_offset_along_the_normal() {
        let painter = RecordingPainter::new();

        // The second line of a double border runs parallel at a fixed distance,
        // measured perpendicular to the first rather than along an axis.
        <CanvasRenderer>::stroke_parallel(&painter, 0.0, 0.0, 100.0, 0.0, 4.0);

        let ops = painter.ops();
        assert!(ops.contains(&Op::MoveTo(0.0, 4.0)), "{ops:?}");
        assert!(ops.contains(&Op::LineTo(100.0, 4.0)), "{ops:?}");
    }

    #[test]
    fn a_degenerate_parallel_stroke_paints_nothing() {
        let painter = RecordingPainter::new();

        // Zero-length borders occur on collapsed cells; the normal is undefined
        // there, so nothing should be drawn rather than a NaN path.
        <CanvasRenderer>::stroke_parallel(&painter, 7.0, 7.0, 7.0, 7.0, 4.0);

        assert!(painter.ops().is_empty(), "{:?}", painter.ops());
    }

    #[test]
    fn the_clip_guard_restores_the_painter_when_it_drops() {
        let painter = RecordingPainter::new();

        {
            let _guard = ClipGuard::new(&painter, 10.0, 20.0, 100.0, 50.0).expect("guard");
            painter.fill_rect(0.0, 0.0, 1.0, 1.0);
        }

        assert_eq!(
            painter.ops(),
            vec![
                Op::Save,
                Op::BeginPath,
                Op::Rect(10.0, 20.0, 100.0, 50.0),
                Op::Clip,
                Op::FillRect(0.0, 0.0, 1.0, 1.0),
                Op::Restore,
            ]
        );
    }

    #[test]
    fn an_empty_clip_rect_produces_no_guard_and_no_clip() {
        let painter = RecordingPainter::new();

        // A zero-width cell must not clip everything away; the guard declines.
        assert!(ClipGuard::new(&painter, 10.0, 20.0, 0.0, 50.0).is_none());

        assert!(painter.ops().is_empty(), "{:?}", painter.ops());
    }

    fn zero_padding_cell() -> PreparedCell {
        let mut cell = make_prepared_cell(0, 1, None, false);
        cell.padding = CellPadding {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };
        cell
    }

    /// Word keeps `hRule="exact"` rows at their declared height even below the
    /// renderer's auto-row floor: the yokohama fixture's 100-twip row measures
    /// 5pt in Word, not the 9pt floor.
    #[test]
    fn exact_row_below_auto_minimum_keeps_declared_height() {
        let row = TableRow {
            height: Some(5.0),
            height_rule: Some(TableRowHeightRule::Exact),
            ..TableRow::default()
        };
        let cells = [zero_padding_cell()];

        let resolved = resolve_row_height(30.0, 20.0, &row, &cells);

        assert!((resolved - points_to_px(5.0)).abs() < 0.01);
        assert!(resolved < MIN_ROW_HEIGHT_PX);
    }

    #[test]
    fn auto_row_uses_content_height_and_falls_back_to_the_empty_line() {
        let row = TableRow::default();
        let cells = [zero_padding_cell()];

        assert!((resolve_row_height(30.0, 20.0, &row, &cells) - 30.0).abs() < 0.01);
        assert!((resolve_row_height(0.0, 20.0, &row, &cells) - 20.0).abs() < 0.01);
        assert!((resolve_row_height(0.0, 4.0, &row, &cells) - MIN_ROW_HEIGHT_PX).abs() < 0.01);
    }

    #[test]
    fn hide_mark_auto_row_keeps_only_insets_and_borders() {
        let row = TableRow::default();
        let mut cell = zero_padding_cell();
        cell.padding.top = 2.0;
        cell.padding.bottom = 3.0;
        cell.height = 5.0;
        cell.hidden_end_marker_paragraph_count = 1;
        cell.border_top.width_px = 2.0;
        cell.border_bottom.width_px = 4.0;

        let resolved = resolve_row_height(5.0, 20.0, &row, &[cell]);

        assert!((resolved - 8.0).abs() < 0.01);
        assert!(resolved < MIN_ROW_HEIGHT_PX);
    }

    #[test]
    fn at_least_row_takes_the_larger_of_content_and_declared_height() {
        let row = TableRow {
            height: Some(24.0),
            height_rule: Some(TableRowHeightRule::AtLeast),
            ..TableRow::default()
        };
        let mut cell = zero_padding_cell();
        cell.border_top.width_px = 0.0;
        cell.border_bottom.width_px = 0.0;
        let cells = [cell];

        assert!((resolve_row_height(10.0, 20.0, &row, &cells) - points_to_px(24.0)).abs() < 0.01);
        assert!((resolve_row_height(60.0, 20.0, &row, &cells) - 60.0).abs() < 0.01);
    }

    #[test]
    fn horizontal_at_least_floor_retains_cell_insets_in_original_word_rows() {
        for (minimum, top_border, bottom_border, observed_word_height) in
            [(47.4, 1.5, 0.25, 56.28), (16.25, 0.25, 0.75, 24.72)]
        {
            let row = TableRow {
                height: Some(minimum),
                height_rule: Some(TableRowHeightRule::AtLeast),
                ..Default::default()
            };
            let mut cell = zero_padding_cell();
            cell.padding.top = points_to_px(4.0);
            cell.padding.bottom = points_to_px(4.0);
            cell.border_top.width_px = points_to_px(top_border);
            cell.border_bottom.width_px = points_to_px(bottom_border);
            let resolved_pt = resolve_row_height(points_to_px(10.0), 20.0, &row, &[cell]) * 0.75;
            assert!(
                (resolved_pt - observed_word_height).abs() <= 0.375,
                "minimum={minimum}, resolved={resolved_pt}, Word={observed_word_height}"
            );
        }
    }

    #[test]
    fn mac_at_least_floor_keeps_the_bottom_border_below_the_padding() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        // Probe rows with 4pt padding: a 14.25pt minimum over a 1.5pt
        // bottom border is 23.75pt; yokohama_table's 47.4pt minimum under a
        // 1.5pt top and over a 0.25pt bottom border is 55.65pt.
        for (minimum, top_border, bottom_border, observed_word_height) in
            [(14.25, 0.0, 1.5, 23.75), (47.4, 1.5, 0.25, 55.65)]
        {
            let row = TableRow {
                height: Some(minimum),
                height_rule: Some(TableRowHeightRule::AtLeast),
                ..Default::default()
            };
            let mut cell = zero_padding_cell();
            cell.padding.top = points_to_px(4.0);
            cell.padding.bottom = points_to_px(4.0);
            cell.border_top.width_px = points_to_px(top_border);
            cell.border_bottom.width_px = points_to_px(bottom_border);
            let resolved_pt = resolve_row_height(points_to_px(10.0), 20.0, &row, &[cell]) * 0.75;
            assert!(
                (resolved_pt - observed_word_height).abs() < 0.01,
                "minimum={minimum}, resolved={resolved_pt}, Word={observed_word_height}"
            );
        }
        set_word_layout_platform(previous);
    }

    /// Three rows beside a cell merged over all of them: 4pt padding and a
    /// 13.62pt line in the second column, a 0.25pt border between its rows
    /// and 1.5pt under the table, and the merged cell's continuations
    /// without `w:tcMar` (yokohama_table rows 3-5).
    fn rows_beside_a_merge(continuation_bottom_padding_pt: f32) -> Vec<Vec<PreparedCell>> {
        (0..3)
            .map(|row_index| {
                let mut merged = make_prepared_cell(
                    0,
                    1,
                    Some(if row_index == 0 {
                        "restart"
                    } else {
                        "continue"
                    }),
                    row_index > 0,
                );
                merged.border_bottom.width_px = points_to_px(1.5);
                merged.border_top.width_px = points_to_px(0.25);
                if row_index == 0 {
                    merged.padding.top = points_to_px(4.0);
                    merged.padding.bottom = points_to_px(4.0);
                    merged.height = points_to_px(21.62);
                } else {
                    merged.padding.top = 0.0;
                    merged.padding.bottom = points_to_px(continuation_bottom_padding_pt);
                }
                let mut text = make_prepared_cell(1, 1, None, false);
                text.padding.top = points_to_px(4.0);
                text.padding.bottom = points_to_px(4.0);
                text.height = points_to_px(21.62);
                text.border_top.width_px = points_to_px(0.25);
                text.border_bottom.width_px = points_to_px(if row_index == 2 { 1.5 } else { 0.25 });
                vec![merged, text]
            })
            .collect()
    }

    fn bordered_heights_beside_a_merge(continuation_bottom_padding_pt: f32) -> Vec<f64> {
        let mut rows = rows_beside_a_merge(continuation_bottom_padding_pt);
        let table = Table {
            rows: vec![TableRow::default(); 3],
            ..Default::default()
        };
        let mut heights: Vec<f64> = vec![points_to_px(21.62); 3];
        add_border_extents(&table, &mut rows, &[false; 3], &mut heights, true);
        heights
            .iter()
            .map(|height| ((height * 0.75) * 100.0).round() / 100.0)
            .collect()
    }

    #[test]
    fn mac_rows_beside_a_continuing_merge_count_its_bottom_border_and_margin() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        // Word for Mac draws the borders between these rows 21.84, 20.64 and
        // 21.84pt apart, then the 1.5pt border; with 8pt of bottom margin on
        // the continuations, 21.84, 25.92 and 25.92pt (each border's top).
        set_word_layout_platform(WordLayoutPlatform::Mac);
        // The first row also holds the table's 0.25pt top border.
        assert_eq!(
            bordered_heights_beside_a_merge(0.0),
            vec![22.12, 20.62, 23.12]
        );
        assert_eq!(
            bordered_heights_beside_a_merge(8.0),
            vec![22.12, 25.87, 27.12]
        );
        // A 3pt merged border over the last row's 1.5pt ones: Word draws the
        // last row 20.64pt below the one above, then the 3pt border.
        let mut rows = rows_beside_a_merge(0.0);
        for row in &mut rows {
            row[0].border_bottom.width_px = points_to_px(3.0);
        }
        let table = Table {
            rows: vec![TableRow::default(); 3],
            ..Default::default()
        };
        let mut heights = vec![points_to_px(21.62); 3];
        add_border_extents(&table, &mut rows, &[false; 3], &mut heights, true);
        assert!(
            (heights[2] * 0.75 - 23.12).abs() < 0.01,
            "{}",
            heights[2] * 0.75
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            bordered_heights_beside_a_merge(0.0),
            vec![23.37, 21.87, 23.12]
        );
        set_word_layout_platform(previous);
    }

    #[test]
    fn mac_exact_row_keeps_the_border_above_it_inside_its_height() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        // `table-row-height-defaults`: a two-line row over a 24pt exact row
        // with 12pt bottom padding, 0.5pt borders everywhere.
        let heights = |platform| {
            set_word_layout_platform(platform);
            let table = Table {
                rows: vec![
                    TableRow::default(),
                    TableRow {
                        height: Some(24.0),
                        height_rule: Some(TableRowHeightRule::Exact),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let mut rows: Vec<Vec<PreparedCell>> = (0..2)
                .map(|_| {
                    let mut cell = zero_padding_cell();
                    cell.border_top.width_px = points_to_px(0.5);
                    cell.border_bottom.width_px = points_to_px(0.5);
                    cell.height = points_to_px(24.14);
                    vec![cell]
                })
                .collect();
            let mut heights = vec![points_to_px(24.14), points_to_px(36.0)];
            add_border_extents(&table, &mut rows, &[false; 2], &mut heights, true);
            heights
                .iter()
                .map(|height| ((height * 0.75) * 100.0).round() / 100.0)
                .collect::<Vec<_>>()
        };
        // Word for Mac draws the borders 24.48pt and 36pt apart.
        assert_eq!(heights(WordLayoutPlatform::Mac), vec![24.64, 36.0]);
        assert_eq!(heights(WordLayoutPlatform::Windows), vec![25.14, 36.0]);
        // A growing row after an exact one holds the border between them.
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let table = Table {
            rows: vec![
                TableRow {
                    height: Some(50.0),
                    height_rule: Some(TableRowHeightRule::Exact),
                    ..Default::default()
                },
                TableRow::default(),
            ],
            ..Default::default()
        };
        let mut rows: Vec<Vec<PreparedCell>> = (0..2)
            .map(|_| {
                let mut cell = zero_padding_cell();
                cell.border_top.width_px = points_to_px(0.5);
                cell.border_bottom.width_px = points_to_px(0.5);
                cell.height = points_to_px(24.14);
                vec![cell]
            })
            .collect();
        let mut heights = vec![points_to_px(50.0), points_to_px(24.14)];
        add_border_extents(&table, &mut rows, &[false; 2], &mut heights, true);
        assert!(
            (heights[1] * 0.75 - 25.14).abs() < 0.01,
            "{}",
            heights[1] * 0.75
        );
        set_word_layout_platform(previous);
    }

    #[test]
    fn vertical_minimum_row_retains_its_shared_border_band() {
        let row = TableRow {
            height: Some(56.7),
            height_rule: Some(TableRowHeightRule::AtLeast),
            ..Default::default()
        };
        for direction in [
            TextDirection::TbRlV,
            TextDirection::TbRl,
            TextDirection::BtLr,
            TextDirection::BtLrV,
        ] {
            let mut cell = zero_padding_cell();
            cell.text_direction = Some(direction);
            cell.border_top.width_px = points_to_px(0.5);
            cell.border_bottom.width_px = points_to_px(0.5);
            let resolved = resolve_row_height(10.0, 20.0, &row, &[cell]) * 0.75;
            assert!(
                (resolved - 57.119995).abs() <= 0.375,
                "{direction:?}: {resolved}"
            );
        }
        let row = TableRow {
            height: Some(12.0),
            height_rule: Some(TableRowHeightRule::AtLeast),
            ..Default::default()
        };
        let mut cell = zero_padding_cell();
        cell.padding.top = 4.0;
        cell.padding.bottom = 6.0;
        assert_eq!(resolve_row_height(80.0, 20.0, &row, &[cell]), 80.0);
    }

    #[test]
    fn exact_row_still_adds_the_largest_bottom_cell_padding() {
        let row = TableRow {
            height: Some(24.0),
            height_rule: Some(TableRowHeightRule::Exact),
            ..TableRow::default()
        };
        let mut cell = zero_padding_cell();
        cell.padding.bottom = 3.0;

        let resolved = resolve_row_height(60.0, 20.0, &row, &[cell]);

        assert!((resolved - (points_to_px(24.0) + 3.0)).abs() < 0.01);
    }
}
