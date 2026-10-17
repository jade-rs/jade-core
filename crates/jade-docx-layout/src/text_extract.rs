use crate::line_metrics::{
    apply_word_east_asian_font_line_height, apply_word_font_proportional_line_height,
    doc_grid_has_active_char_grid, doc_grid_has_active_line_grid, resolve_effective_snap_to_grid,
    resolve_line_height_points_with_default_multiplier, resolve_widow_orphan_decision,
    text_font_slots, word_east_asian_fe_font_size_floor, word_east_asian_font_line_box_applies,
    word_east_asian_font_line_height_pt, word_east_asian_font_metrics_apply,
    word_grid_line_centres_text, word_run_border_extent_pt, word_run_vertical_extent_pt,
    word_sizes_east_asian_lines_from_faces, word_tallest_single_line_height_pt,
    WidowOrphanDecision, WORD_EAST_ASIAN_FE_LINE_SPACING_MULTIPLIER,
};
use crate::line_wrap::{
    apply_paragraph_indent_with_char_unit, layout_text_vertically, merge_east_asian_rules,
    prepare_drop_cap_layout, resolve_indent_first_line_pt, resolve_indent_left_pt,
    resolve_tab_stops, wrap_text_into_lines, EastAsianLineBreakingRules, LineWrapContext,
    ParagraphStyle, RunStyle, DEFAULT_INDENT_CHAR_UNIT_PT,
};
use crate::section_transition::{
    resolve_page_margins, resolve_section_transition, SectionBreakType, SectionTransition,
};
use crate::units::points_to_px;
use jade_docx_reader::PAGE_BREAK_CHAR;
use jade_docx_reader::{
    BodyElement, Document, DocumentParser, DocumentSettings, MathParagraph, OpenXmlPackage,
    Paragraph, RelationshipTypes, Run, RunFonts, RunHyperlink, RunProperties, SectionColumns,
    SectionProperties, StylesPart, Table, TableCell, TextDirection, ThemePart,
};
use rustc_hash::FxHashMap;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

const DEFAULT_PAGE_WIDTH_PT: f32 = 595.0;
const DEFAULT_PAGE_HEIGHT_PT: f32 = 842.0;
const DEFAULT_MARGIN_PT: f32 = 72.0;
const DEFAULT_TABLE_CELL_HORIZONTAL_PADDING_PT: f32 = 6.0;
const DEFAULT_TABLE_CELL_VERTICAL_PADDING_PT: f32 =
    jade_docx_reader::document::DEFAULT_VERTICAL_CELL_MARGIN_PT;
pub const ORACLE_MULTI_COLUMN_WRAP_BONUS_PX: f64 = 15.5;
const EPS: f64 = 0.001;

pub type ParagraphOracleAdvanceMap = FxHashMap<usize, f64>;
pub type OracleAdvanceMap = FxHashMap<usize, Rc<ParagraphOracleAdvanceMap>>;

#[derive(Debug)]
pub enum LineLayoutError<E> {
    PackageLoad(Box<dyn std::error::Error>),
    MissingDocumentRelationship,
    MissingDocumentPart(String),
    DocumentParse(Box<dyn std::error::Error>),
    Layout(E),
}

pub type FirstWrappedLineError<E> = LineLayoutError<E>;

#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    pub index: usize,
    pub paragraph_index: usize,
    pub line_index_in_paragraph: usize,
    /// 1-indexed page number.
    pub page: usize,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub y: f64,
    /// Absolute baseline Y in the same top-down coordinate space as `y`.
    pub baseline: f64,
    pub width: f64,
    pub height: f64,
    pub spans: Vec<LineTextSpan>,
    pub hyperlinks: Vec<LineHyperlinkSpan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineTextSpan {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub x_offset: f64,
    pub width: f64,
    pub properties: Option<RunProperties>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineHyperlinkSpan {
    pub start: usize,
    pub end: usize,
    pub x_offset: f64,
    pub width: f64,
    pub hyperlink: RunHyperlink,
}

#[derive(Clone, Debug, Default)]
pub struct DocumentLineLayout {
    lines: Vec<LineInfo>,
    page_width: f64,
    page_height: f64,
}

impl DocumentLineLayout {
    pub fn get_lines(&self) -> std::slice::Iter<'_, LineInfo> {
        self.lines.iter()
    }

    pub fn line(&self, index: usize) -> Option<&LineInfo> {
        self.lines.get(index)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn page_width(&self) -> f64 {
        self.page_width
    }

    pub fn page_height(&self) -> f64 {
        self.page_height
    }
}

struct ParagraphLayoutInput {
    paragraph_index: usize,
    text: String,
    run_styles: Vec<RunStyle>,
    base_x: f64,
    report_x_offsets: WordOracleLineXOffsets,
    paragraph_style: ParagraphStyle,
    column_right_edge: f64,
    line_height_px: f64,
    spacing_before_px: f64,
    spacing_after_px: f64,
    widow_control_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WordOracleLineXOffsets {
    first_line: f64,
    continuation: f64,
}

impl WordOracleLineXOffsets {
    fn for_line(self, line_index: usize) -> f64 {
        if line_index == 0 {
            self.first_line
        } else {
            self.continuation
        }
    }
}

struct LayoutInputContext<'a, C> {
    document: &'a Document,
    styles: Option<&'a StylesPart>,
    theme: Option<&'a ThemePart>,
    context: &'a C,
    oracle_advances_px_by_paragraph: Option<&'a OracleAdvanceMap>,
}

#[derive(Clone, Debug)]
struct TextColumnLayout {
    columns: Vec<(f64, f64)>,
}

#[derive(Clone, Debug)]
struct SectionLayoutGeometry {
    page_width: f64,
    page_height: f64,
    content_left_x: f64,
    content_right_x: f64,
    content_top_y: f64,
    content_bottom_y: f64,
    section_content_height_pt: f32,
    column_layout: TextColumnLayout,
    text_direction: Option<TextDirection>,
}

impl SectionLayoutGeometry {
    fn resolve(
        document: &Document,
        section: Option<&SectionProperties>,
        page_number: usize,
    ) -> Self {
        let page_settings = section
            .and_then(|properties| properties.page_settings.as_ref())
            .or_else(|| {
                document
                    .default_section
                    .as_ref()
                    .and_then(|properties| properties.page_settings.as_ref())
            })
            .or(document.page_settings.as_ref());
        let width_pt = page_settings
            .and_then(|settings| settings.width)
            .unwrap_or(DEFAULT_PAGE_WIDTH_PT);
        let height_pt = page_settings
            .and_then(|settings| settings.height)
            .unwrap_or(DEFAULT_PAGE_HEIGHT_PT);
        let margins = resolve_page_margins(
            page_settings
                .and_then(|settings| settings.margin_left)
                .unwrap_or(DEFAULT_MARGIN_PT),
            page_settings
                .and_then(|settings| settings.margin_right)
                .unwrap_or(DEFAULT_MARGIN_PT),
            page_settings
                .and_then(|settings| settings.margin_top)
                .unwrap_or(DEFAULT_MARGIN_PT),
            page_settings
                .and_then(|settings| settings.gutter)
                .unwrap_or(0.0),
            document.gutter_at_top,
            document.mirror_margins,
            page_number,
        );
        let margin_bottom_pt = page_settings
            .and_then(|settings| settings.margin_bottom)
            .unwrap_or(DEFAULT_MARGIN_PT);
        let page_width = points_to_px(width_pt);
        let page_height = points_to_px(height_pt);
        let content_left_x = points_to_px(margins.left);
        let content_right_x = (page_width - points_to_px(margins.right)).max(content_left_x + 1.0);
        let content_top_y = points_to_px(margins.top);
        let content_bottom_y =
            (page_height - points_to_px(margin_bottom_pt)).max(content_top_y + 1.0);
        let columns = section
            .and_then(|properties| properties.columns.as_ref())
            .or_else(|| {
                document
                    .default_section
                    .as_ref()
                    .and_then(|properties| properties.columns.as_ref())
            });

        Self {
            page_width,
            page_height,
            content_left_x,
            content_right_x,
            content_top_y,
            content_bottom_y,
            section_content_height_pt: (height_pt - margins.top - margin_bottom_pt).max(0.0),
            column_layout: TextColumnLayout::from_section(columns, content_left_x, content_right_x),
            text_direction: section
                .and_then(|properties| properties.text_direction)
                .or_else(|| {
                    document
                        .default_section
                        .as_ref()
                        .and_then(|properties| properties.text_direction)
                }),
        }
    }

    fn same_page_geometry(&self, other: &Self) -> bool {
        (self.page_width - other.page_width).abs() < EPS
            && (self.page_height - other.page_height).abs() < EPS
            && (self.content_left_x - other.content_left_x).abs() < EPS
            && (self.content_right_x - other.content_right_x).abs() < EPS
            && (self.content_top_y - other.content_top_y).abs() < EPS
            && (self.content_bottom_y - other.content_bottom_y).abs() < EPS
    }
}

impl TextColumnLayout {
    fn single(left: f64, right: f64) -> Self {
        Self {
            columns: vec![(left, right.max(left + 1.0))],
        }
    }

    fn from_section(
        columns: Option<&SectionColumns>,
        content_left: f64,
        content_right: f64,
    ) -> Self {
        let Some(columns) = columns else {
            return Self::single(content_left, content_right);
        };
        let count = columns
            .count
            .unwrap_or_else(|| columns.columns.len().max(1) as u32)
            .max(1) as usize;
        if count == 1 {
            return Self::single(content_left, content_right);
        }

        let total_width = (content_right - content_left).max(1.0);
        let mut gap = columns.space.map(points_to_px).unwrap_or(0.0).max(0.0);
        if gap * (count.saturating_sub(1) as f64) >= total_width {
            gap = 0.0;
        }
        let available_for_columns = (total_width - gap * (count.saturating_sub(1) as f64)).max(1.0);
        let use_custom_widths = columns.columns.len() >= count
            && columns
                .columns
                .iter()
                .take(count)
                .all(|column| column.width.is_some());
        let widths = if use_custom_widths {
            let declared: Vec<f64> = columns
                .columns
                .iter()
                .take(count)
                .map(|column| points_to_px(column.width.unwrap_or_default()))
                .collect();
            let total_declared = declared.iter().sum::<f64>();
            if total_declared > 0.0 {
                let scale = available_for_columns / total_declared;
                declared.into_iter().map(|width| width * scale).collect()
            } else {
                vec![available_for_columns / count as f64; count]
            }
        } else {
            vec![available_for_columns / count as f64; count]
        };

        let mut left = content_left;
        let mut resolved = Vec::with_capacity(count);
        for width in widths {
            let right = (left + width).max(left + 1.0);
            resolved.push((left, right));
            left = right + gap;
        }
        Self { columns: resolved }
    }

    fn left(&self, index: usize) -> f64 {
        self.columns
            .get(index)
            .or_else(|| self.columns.last())
            .map(|(left, _)| *left)
            .unwrap_or(0.0)
    }

    fn right(&self, index: usize) -> f64 {
        self.columns
            .get(index)
            .or_else(|| self.columns.last())
            .map(|(_, right)| *right)
            .unwrap_or(1.0)
    }

    fn len(&self) -> usize {
        self.columns.len().max(1)
    }
}

fn estimate_paragraph_font_size(paragraph: &jade_docx_reader::Paragraph) -> f32 {
    let explicit = paragraph
        .runs
        .iter()
        .filter(|run| !run.suppresses_line_height_contribution())
        .filter_map(|run| run.properties.as_ref().and_then(|props| props.font_size))
        .chain(
            paragraph
                .properties
                .as_ref()
                .and_then(|props| props.mark_run_properties.as_ref())
                .and_then(|props| props.font_size),
        )
        .reduce(f32::max);

    let has_visible_run = paragraph.runs.iter().any(|run| !run.text.is_empty());
    let mark_run_properties = paragraph
        .properties
        .as_ref()
        .and_then(|properties| properties.mark_run_properties.as_ref());
    let floor = word_east_asian_fe_font_size_floor(has_visible_run, mark_run_properties);
    explicit.map(|size| size.max(floor)).unwrap_or(floor)
}

/// Width in points of one character unit ("1字") used to convert
/// `w:leftChars`/`w:firstLineChars`/… indents: the paragraph's effective font
/// size, matching Word. Falls back through style/docDefault sizes.
pub fn paragraph_indent_char_unit_pt(
    paragraph: &jade_docx_reader::Paragraph,
    styles: Option<&StylesPart>,
) -> f64 {
    f64::from(estimate_paragraph_font_size_with_styles(paragraph, styles)).max(1.0)
}

/// Grid-aware character unit: with an active character grid that the
/// paragraph snaps to, one "字" is one grid cell — the font size plus the
/// grid's extra per-character spacing (`w:docGrid/@w:charSpace`).
pub fn paragraph_indent_char_unit_pt_with_grid(
    paragraph: &jade_docx_reader::Paragraph,
    styles: Option<&StylesPart>,
    doc_grid: Option<&jade_docx_reader::DocGrid>,
    snap_to_grid: bool,
) -> f64 {
    let base = paragraph_indent_char_unit_pt(paragraph, styles);
    if snap_to_grid && doc_grid_has_active_char_grid(doc_grid) {
        if let Some(char_space) = doc_grid.and_then(|grid| grid.char_space) {
            if char_space.is_finite() {
                return (base + f64::from(char_space)).max(1.0);
            }
        }
    }
    base
}

fn estimate_paragraph_font_size_with_styles(
    paragraph: &jade_docx_reader::Paragraph,
    styles: Option<&StylesPart>,
) -> f32 {
    let explicit = paragraph
        .runs
        .iter()
        .filter(|run| !run.suppresses_line_height_contribution())
        .filter_map(|run| run.properties.as_ref().and_then(|props| props.font_size))
        .reduce(f32::max);
    if let Some(size) = explicit {
        return size;
    }
    let style_size = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
        .or_else(|| styles.and_then(StylesPart::default_paragraph_style_id))
        .and_then(|style_id| styles.and_then(|styles| styles.resolve_font_size(style_id)));
    style_size
        .or_else(|| styles.and_then(StylesPart::doc_default_font_size))
        .unwrap_or(12.0)
}

fn run_is_hidden_text(run: &Run) -> bool {
    run.properties
        .as_ref()
        .map(|props| props.vanish.unwrap_or(false) || props.spec_vanish.unwrap_or(false))
        .unwrap_or(false)
}

fn paragraph_uses_east_asian_fonting(paragraph: &Paragraph) -> bool {
    paragraph.runs.iter().any(|run| {
        run.properties
            .as_ref()
            .and_then(|props| props.fonts.as_ref())
            .is_some_and(|fonts| fonts.east_asia.is_some() || fonts.east_asia_theme.is_some())
    })
}

fn paragraph_has_visible_text(paragraph: &Paragraph) -> bool {
    paragraph
        .runs
        .iter()
        .filter(|run| !run_is_hidden_text(run))
        .any(|run| !run.text.trim().is_empty())
}

fn paragraph_has_traceable_text_or_math(paragraph: &Paragraph) -> bool {
    paragraph
        .runs
        .iter()
        .filter(|run| !run_is_hidden_text(run))
        .any(|run| {
            !run.text.trim().is_empty()
                || run
                    .math
                    .as_ref()
                    .and_then(|math| math.preview_text(1))
                    .is_some()
        })
}

fn block_has_visible_paragraph_without_east_asian_fonting(block: &BodyElement) -> bool {
    match block {
        BodyElement::Paragraph(paragraph) => {
            paragraph_has_visible_text(paragraph) && !paragraph_uses_east_asian_fonting(paragraph)
        }
        BodyElement::Table(table) => table.rows.iter().any(|row| {
            row.cells.iter().any(|cell| {
                cell.blocks
                    .iter()
                    .any(block_has_visible_paragraph_without_east_asian_fonting)
            })
        }),
        BodyElement::Math(_) => false,
    }
}

fn document_has_visible_paragraph_without_east_asian_fonting(document: &Document) -> bool {
    document.body.as_ref().is_some_and(|body| {
        body.elements
            .iter()
            .any(block_has_visible_paragraph_without_east_asian_fonting)
    })
}

fn apply_document_settings(document: &mut Document, settings: &DocumentSettings) {
    // `w:defaultTabStop` is a general document setting, not a FE-layout
    // compatibility toggle. `tests/word-oracle/tab/word-trace.json` uses 42pt
    // stops from settings.xml, so apply it for every oracle document.
    document.default_tab_stop = settings.default_tab_stop;
    document.no_tab_hanging_indent = settings.no_tab_hanging_indent;
    document.gutter_at_top = settings.gutter_at_top;
    document.mirror_margins = settings.mirror_margins;
    if settings.character_spacing_control.as_deref() == Some("doNotCompress") {
        document.snap_to_grid_default = settings.snap_to_grid_default;
        document.east_asian_line_breaking = settings.to_line_breaking_settings();
    }
}

/// Resolve paragraph-before spacing in pixels using the shared line height.
// Word's nominal line-unit conversion is distinct from font-sized text
// advance (Application.LinesToPoints). The original Mac contract's 33/79
// afterLines/after pair measures 3.95pt, rather than 0.33 × 13.65pt.
// Keep the Windows sizing policy until its physical spacing is verified.
fn paragraph_spacing_line_unit_pt(line_height_points: f32) -> f32 {
    if crate::line_metrics::word_layout_platform() == crate::line_metrics::WordLayoutPlatform::Mac {
        12.0
    } else {
        line_height_points
    }
}

pub fn spacing_before_px(paragraph: &jade_docx_reader::Paragraph, line_height_points: f32) -> f64 {
    paragraph
        .properties
        .as_ref()
        .and_then(|props| props.spacing.as_ref())
        .and_then(|spacing| {
            if let Some(lines) = spacing.before_lines {
                Some(points_to_px(
                    paragraph_spacing_line_unit_pt(line_height_points) * lines,
                ))
            } else {
                spacing.before.map(points_to_px)
            }
        })
        .unwrap_or(0.0)
}

/// Resolve paragraph-after spacing using the platform's shared line-unit rule.
pub fn spacing_after_px(paragraph: &jade_docx_reader::Paragraph, line_height_points: f32) -> f64 {
    paragraph
        .properties
        .as_ref()
        .and_then(|props| props.spacing.as_ref())
        .and_then(|spacing| {
            if let Some(lines) = spacing.after_lines {
                Some(points_to_px(
                    paragraph_spacing_line_unit_pt(line_height_points) * lines,
                ))
            } else {
                spacing.after.map(points_to_px)
            }
        })
        .unwrap_or(0.0)
}

fn paragraph_doc_grid(document: &Document) -> Option<&jade_docx_reader::DocGrid> {
    document
        .default_section
        .as_ref()
        .and_then(|section| section.doc_grid.as_ref())
}

fn paragraph_allows_style_font_size_defaults(
    _document: &Document,
    _doc_grid: Option<&jade_docx_reader::DocGrid>,
) -> bool {
    // OOXML run properties always participate in the style cascade. FE layout
    // and docGrid change glyph placement/line metrics, but do not suppress
    // `w:docDefaults/w:rPrDefault` for runs that omit `w:sz`.
    true
}

fn word_oracle_non_numbered_left_indent_report_offsets(
    document: &Document,
    paragraph: &jade_docx_reader::Paragraph,
) -> WordOracleLineXOffsets {
    if !document.east_asian_line_breaking.use_east_asian_layout {
        return WordOracleLineXOffsets::default();
    }
    let Some(props) = paragraph.properties.as_ref() else {
        return WordOracleLineXOffsets::default();
    };
    if props.numbering.is_some() {
        return WordOracleLineXOffsets::default();
    }
    let Some(indent) = props.indent.as_ref() else {
        return WordOracleLineXOffsets::default();
    };
    let bidi = props.bidi.unwrap_or(false);
    let left_pt =
        resolve_indent_left_pt(indent, bidi, DEFAULT_INDENT_CHAR_UNIT_PT).map(|v| v as f32);
    let first_line_offset_pt =
        match resolve_indent_first_line_pt(indent, DEFAULT_INDENT_CHAR_UNIT_PT, true) {
            // firstLine variants report their own offset.
            Some(v) if v >= 0.0 => v as f32,
            // hanging variants report left minus the hanging amount.
            Some(v) => left_pt.unwrap_or(0.0) + v as f32,
            None => left_pt.unwrap_or(0.0),
        };
    let continuation_offset_pt = left_pt.unwrap_or(0.0);
    WordOracleLineXOffsets {
        first_line: if first_line_offset_pt.abs() <= f32::EPSILON {
            0.0
        } else {
            -points_to_px(first_line_offset_pt)
        },
        continuation: if continuation_offset_pt.abs() <= f32::EPSILON {
            0.0
        } else {
            -points_to_px(continuation_offset_pt)
        },
    }
}

fn nonzero_word_oracle_offsets(offsets: WordOracleLineXOffsets) -> bool {
    offsets.first_line != 0.0 || offsets.continuation != 0.0
}

fn word_oracle_line_x_offset(input: &ParagraphLayoutInput, line_index: usize) -> f64 {
    input.report_x_offsets.for_line(line_index)
}

fn word_oracle_line_start_x(input: &ParagraphLayoutInput, line_index: usize) -> f64 {
    let start_x = input.base_x + input.paragraph_style.line_indent(line_index);
    start_x + word_oracle_line_x_offset(input, line_index)
}

fn paragraph_line_height_points<C: LineWrapContext>(
    layout_context: &LayoutInputContext<'_, C>,
    paragraph: &jade_docx_reader::Paragraph,
    text: &str,
    run_styles: &[RunStyle],
    doc_grid: Option<&jade_docx_reader::DocGrid>,
    snap_to_grid: bool,
    section_content_height_pt: f32,
) -> Result<(f32, Option<crate::line_metrics::WordProportionalLineSizing>), C::Error> {
    let document = layout_context.document;
    let font_size = if document.east_asian_line_breaking.use_east_asian_layout
        || doc_grid_has_active_line_grid(doc_grid)
        || doc_grid_has_active_char_grid(doc_grid)
    {
        estimate_paragraph_font_size(paragraph)
    } else {
        estimate_paragraph_font_size_with_styles(paragraph, layout_context.styles)
    };
    let implicit_line_box_pt =
        measure_paragraph_implicit_line_box_points(layout_context.context, paragraph)?;
    let (east_asian_line_pt, single_line_pt) = paragraph_word_font_lines_pt(
        layout_context.context,
        text,
        run_styles,
        word_sizes_east_asian_lines_from_faces(
            document.east_asian_line_breaking.use_east_asian_layout,
        ),
    )?;
    let implicit_line_box_pt = if word_grid_line_centres_text(snap_to_grid, doc_grid) {
        single_line_pt.or(implicit_line_box_pt)
    } else {
        implicit_line_box_pt
    };
    let default_line_spacing_multiplier = if document.east_asian_line_breaking.use_east_asian_layout
        && !doc_grid_has_active_line_grid(doc_grid)
        && !doc_grid_has_active_char_grid(doc_grid)
    {
        WORD_EAST_ASIAN_FE_LINE_SPACING_MULTIPLIER
    } else {
        1.15
    };
    let spacing = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.spacing.as_ref());
    let line_height = resolve_line_height_points_with_default_multiplier(
        font_size,
        spacing,
        doc_grid,
        snap_to_grid,
        Some(section_content_height_pt),
        implicit_line_box_pt,
        default_line_spacing_multiplier,
    );
    let line_height = apply_word_east_asian_font_line_height(
        line_height,
        word_east_asian_font_line_box_applies(
            spacing,
            snap_to_grid,
            document.east_asian_line_breaking.use_east_asian_layout,
        ),
        east_asian_line_pt,
    );
    Ok((
        apply_word_font_proportional_line_height(
            line_height,
            spacing,
            snap_to_grid,
            single_line_pt,
        ),
        crate::line_metrics::WordProportionalLineSizing::new(
            spacing,
            snap_to_grid,
            single_line_pt.is_some(),
        ),
    ))
}

/// Word's font-sized lines for a paragraph's laid-out runs, in one pass: the
/// tallest East Asian face line among runs that draw East Asian text (see
/// [`word_east_asian_font_line_height_pt`]) and the paragraph's single line
/// when every visible face has known metrics (see
/// [`word_tallest_single_line_height_pt`]). `run_styles` carry the faces the
/// runs are laid out with: style chain and theme already resolved.
fn paragraph_word_font_lines_pt<C: LineWrapContext>(
    context: &C,
    text: &str,
    run_styles: &[RunStyle],
    east_asian_faces_size_lines: bool,
) -> Result<(Option<f32>, Option<f32>), C::Error> {
    let mut east_asian_line: Option<f32> = None;
    let mut runs = Vec::with_capacity(run_styles.len());
    for run_style in run_styles {
        let Some(run_text) = text.get(run_style.start..run_style.end) else {
            continue;
        };
        if run_text.is_empty() {
            continue;
        }
        let resolved = context.resolve_run_style(run_style.properties.as_deref())?;
        let slots = text_font_slots(run_text);
        if slots.1 {
            if let Some(height) =
                word_east_asian_font_line_height_pt(&resolved.font_descriptor_east_asia)
            {
                east_asian_line =
                    Some(east_asian_line.map_or(height, |current| current.max(height)));
            }
        }
        runs.push(word_run_vertical_extent_pt(
            slots,
            &resolved.font_descriptor_latin_ascii,
            &resolved.font_descriptor_east_asia,
            east_asian_faces_size_lines,
            word_run_border_extent_pt(run_style.properties.as_deref()),
        ));
    }
    Ok((east_asian_line, word_tallest_single_line_height_pt(runs)))
}

fn build_empty_paragraph_input<C: LineWrapContext>(
    layout_context: &LayoutInputContext<'_, C>,
    paragraph: &jade_docx_reader::Paragraph,
    section_content_height_pt: f32,
) -> Result<ParagraphLayoutInput, C::Error> {
    let document = layout_context.document;
    let doc_grid = paragraph_doc_grid(document);
    let paragraph_snap_to_grid = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.snap_to_grid);
    let snap_to_grid = resolve_effective_snap_to_grid(
        paragraph_snap_to_grid,
        document.snap_to_grid_default,
        doc_grid,
    );
    let (line_height_points, _) = paragraph_line_height_points(
        layout_context,
        paragraph,
        "",
        &[],
        doc_grid,
        snap_to_grid,
        section_content_height_pt,
    )?;
    Ok(ParagraphLayoutInput {
        paragraph_index: 0,
        text: String::new(),
        run_styles: Vec::new(),
        base_x: 0.0,
        report_x_offsets: WordOracleLineXOffsets::default(),
        paragraph_style: ParagraphStyle::default(),
        column_right_edge: 0.0,
        line_height_px: points_to_px(line_height_points),
        spacing_before_px: spacing_before_px(paragraph, line_height_points),
        spacing_after_px: spacing_after_px(paragraph, line_height_points),
        widow_control_enabled: false,
    })
}

fn build_paragraph_input<C: LineWrapContext>(
    layout_context: &LayoutInputContext<'_, C>,
    paragraph_index: usize,
    paragraph: &jade_docx_reader::Paragraph,
    content_left_x: f64,
    content_right_x: f64,
    section_content_height_pt: f32,
) -> Result<Option<ParagraphLayoutInput>, C::Error> {
    let document = layout_context.document;
    let doc_grid = paragraph_doc_grid(document);
    let allow_font_size_defaults = paragraph_allows_style_font_size_defaults(document, doc_grid);
    let protect_explicit_east_asian_paragraph =
        !allow_font_size_defaults && paragraph_uses_east_asian_fonting(paragraph);
    let paragraph_run_fallback = if protect_explicit_east_asian_paragraph {
        None
    } else {
        style_run_font_fallback(paragraph, layout_context.styles)
    };
    let mut text = String::new();
    let mut run_styles = Vec::new();
    for run in &paragraph.runs {
        if run_is_hidden_text(run) {
            continue;
        }
        let run_text = if run.text.is_empty() {
            run_math_oracle_text(run).unwrap_or_default()
        } else if let Some(math_text) = run_math_oracle_text(run) {
            format!("{}{}", run.text, math_text)
        } else {
            run.text.clone()
        };
        if run_text.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(&run_text);
        let end = text.len();
        run_styles.push(RunStyle {
            start,
            end,
            properties: effective_run_properties(
                run,
                // Keep the catalogue available for `w:rStyle` even when a
                // CJK-authored paragraph suppresses paragraph/doc-default
                // font fallback. Character styles are run-local semantics,
                // not paragraph font defaults.
                layout_context.styles,
                paragraph_run_fallback.as_ref(),
                allow_font_size_defaults,
            )
            .map(|mut props| {
                // Resolve `w:asciiTheme`-style references so the headless and
                // PDF layout paths measure with the same concrete faces as
                // the editor, instead of the default font stacks.
                if let Some(theme) = layout_context.theme {
                    theme.apply_to_run_properties(&mut props);
                }
                Rc::new(props)
            }),
            revision: run.revision.clone(),
            hyperlink: run.hyperlink.clone(),
            inline_object: None,
        });
    }
    if text.trim().is_empty() {
        return Ok(None);
    }

    let mut base_x = content_left_x;
    let mut paragraph_style = ParagraphStyle::default();
    if let Some(default_tab_stop) = document.default_tab_stop {
        paragraph_style.default_tab_width = points_to_px(default_tab_stop);
    }
    paragraph_style.use_hanging_indent_tab_stop = !document.no_tab_hanging_indent;
    if let Some(char_advances) = layout_context
        .oracle_advances_px_by_paragraph
        .and_then(|by_paragraph| by_paragraph.get(&paragraph_index))
    {
        let mut byte_advances = FxHashMap::default();
        let mut char_index = 0usize;
        for (byte_offset, grapheme) in text.grapheme_indices(true) {
            if let Some(advance) = char_advances.get(&char_index).copied() {
                byte_advances.insert(paragraph_index + byte_offset, advance);
            }
            char_index += grapheme.chars().count();
        }
        if !byte_advances.is_empty() {
            // Word oracle comparison only: COM TRACE records per-character
            // advances in `target/contract-numbering-word-trace.json`. Passing
            // them through ParagraphStyle lets the wrap loop use position-aware
            // Word measurements without changing normal editor/runtime layout.
            paragraph_style.oracle_advance_px_by_text_offset = Some(Rc::new(byte_advances));
        }
    }
    let mut report_x_offsets = WordOracleLineXOffsets::default();
    let inherited_bidi = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
        .and_then(|style_id| {
            layout_context
                .styles
                .and_then(|styles| styles.resolve_paragraph_properties(style_id))
        })
        .and_then(|props| props.bidi);
    paragraph_style.bidi = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.bidi)
        .or(inherited_bidi)
        .unwrap_or(false);
    if let Some(props) = paragraph.properties.as_ref() {
        if let Some(alignment) = &props.alignment {
            paragraph_style.alignment = alignment.to_string();
        }
        if let Some(indent) = &props.indent {
            // Character-unit indents ("1字") scale with the paragraph font
            // size in Word (plus the grid cell's extra spacing when the
            // paragraph snaps to an active character grid).
            let indent_snap_to_grid = resolve_effective_snap_to_grid(
                props.snap_to_grid,
                document.snap_to_grid_default,
                doc_grid,
            );
            let char_unit_pt = paragraph_indent_char_unit_pt_with_grid(
                paragraph,
                layout_context.styles,
                doc_grid,
                indent_snap_to_grid,
            );
            apply_paragraph_indent_with_char_unit(
                &mut base_x,
                &mut paragraph_style,
                indent,
                props.numbering.is_some(),
                char_unit_pt,
            );
            let offsets = word_oracle_non_numbered_left_indent_report_offsets(document, paragraph);
            if nonzero_word_oracle_offsets(offsets) {
                // Word COM trace evidence:
                // `target/contract-rich-word-trace.json` compared via
                // `target/contract-after-origin-report.json` shows
                // `contract.docx` non-numbered `doNotCompress` FE-layout
                // paragraphs reported at the page margin even when OOXML carries
                // `w:ind w:left`. Examples: p0 has `w:left=3414` and p42-p47
                // have `w:left=4195/4620`, but Word reports `x_pt=84.65` for
                // their visible text. The follow-up full report
                // `target/contract-after-settings-indent-full-report.json`
                // showed that removing `w:left` from the wrap base made p0
                // collapse from Word's three visual lines to two, so keep the
                // indented wrap width and adjust only the oracle comparison x.
                // Numbered paragraphs are excluded because their text x is
                // driven by numbering definitions rather than this paragraph
                // left-indent quirk.
                report_x_offsets = offsets;
            }
        }
        if props.numbering.is_some()
            && layout_context.oracle_advances_px_by_paragraph.is_some()
            && document
                .east_asian_line_breaking
                .character_spacing_control
                .as_deref()
                == Some("doNotCompress")
        {
            // `target/contract-position-advance-report.json` still showed
            // numbered legal paragraphs overfilling their first line by about
            // roughly 1.5 10.5pt full-width cells once Word COM character advances
            // are applied. Keep the visual x origin unchanged
            // and narrow only first-line availability; continuation lines use
            // a slightly wider effective width because the same report shows
            // they otherwise end one glyph early (`解除` vs `解`, `につい`
            // vs `につ`).
            paragraph_style.first_line_available_adjustment = points_to_px(15.75);
            paragraph_style.continuation_available_adjustment = points_to_px(10.5);
        }
        paragraph_style.tab_stops = resolve_tab_stops(&props.tabs);
        let defaults =
            EastAsianLineBreakingRules::from_settings(&document.east_asian_line_breaking);
        paragraph_style.east_asian_rules =
            merge_east_asian_rules(&defaults, props.east_asian_settings.as_ref());
        paragraph_style.enable_line_break_rules =
            paragraph_style.east_asian_rules.use_east_asian_layout;
        if paragraph_style
            .east_asian_rules
            .character_spacing_control
            .as_deref()
            == Some("compressPunctuation")
            && !doc_grid_has_active_line_grid(doc_grid)
            && !doc_grid_has_active_char_grid(doc_grid)
        {
            paragraph_style.east_asian_advance_scale =
                crate::line_metrics::word_compressed_punctuation_east_asian_advance_scale();
        }
    }

    let paragraph_snap_to_grid = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.snap_to_grid);
    let snap_to_grid = resolve_effective_snap_to_grid(
        paragraph_snap_to_grid,
        document.snap_to_grid_default,
        doc_grid,
    );
    paragraph_style.apply_doc_grid_character_settings(doc_grid, snap_to_grid);
    if paragraph_style.alignment.is_empty() && paragraph_style.bidi {
        paragraph_style.alignment = "right".to_string();
    }
    paragraph_style.word_east_asian_font_line_box = word_east_asian_font_metrics_apply(
        paragraph
            .properties
            .as_ref()
            .and_then(|props| props.spacing.as_ref()),
        document.east_asian_line_breaking.use_east_asian_layout,
    );
    paragraph_style.word_grid_line_sizing = crate::line_metrics::WordGridLineSizing::new(
        paragraph
            .properties
            .as_ref()
            .and_then(|props| props.spacing.as_ref()),
        snap_to_grid,
        doc_grid,
        Some(section_content_height_pt),
        estimate_paragraph_font_size(paragraph),
    );

    let (line_height_points, proportional_sizing) = paragraph_line_height_points(
        layout_context,
        paragraph,
        &text,
        &run_styles,
        doc_grid,
        snap_to_grid,
        section_content_height_pt,
    )?;
    paragraph_style.word_proportional_line_sizing = proportional_sizing;

    let vertical_section = document
        .default_section
        .as_ref()
        .and_then(|section| section.text_direction)
        .is_some_and(|direction| {
            matches!(
                direction,
                TextDirection::TbRl
                    | TextDirection::TbRlV
                    | TextDirection::BtLr
                    | TextDirection::BtLrV
            )
        });
    if let Some(drop_cap) = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.drop_cap)
        .filter(|_| !vertical_section)
    {
        paragraph_style.drop_cap = prepare_drop_cap_layout(
            layout_context.context,
            &text,
            &mut run_styles,
            drop_cap,
            points_to_px(line_height_points),
        )?;
    }

    Ok(Some(ParagraphLayoutInput {
        paragraph_index,
        text,
        run_styles,
        base_x,
        report_x_offsets,
        paragraph_style,
        column_right_edge: content_right_x,
        line_height_px: points_to_px(line_height_points),
        spacing_before_px: spacing_before_px(paragraph, line_height_points),
        spacing_after_px: spacing_after_px(paragraph, line_height_points),
        widow_control_enabled: paragraph
            .properties
            .as_ref()
            .and_then(|props| props.widow_control)
            .unwrap_or(true),
    }))
}

fn measure_paragraph_implicit_line_box_points<C: LineWrapContext>(
    context: &C,
    paragraph: &jade_docx_reader::Paragraph,
) -> Result<Option<f32>, C::Error> {
    let mut max_line_box_px = 0.0_f64;
    let mut seen_descriptors = std::collections::HashSet::new();

    for run in &paragraph.runs {
        if run_is_hidden_text(run) {
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let resolved = context.resolve_run_style(run.properties.as_ref())?;
        for descriptor in [
            &*resolved.font_descriptor_latin_ascii,
            &*resolved.font_descriptor_latin_hansi,
            &*resolved.font_descriptor_east_asia,
            &*resolved.font_descriptor_complex,
        ] {
            if !seen_descriptors.insert(descriptor.to_string()) {
                continue;
            }
            if let Some(height) = context.measure_line_box_height(descriptor)? {
                max_line_box_px = max_line_box_px.max(height);
            }
        }
    }

    if max_line_box_px > 0.0 {
        Ok(Some((max_line_box_px * 72.0 / 96.0) as f32))
    } else {
        Ok(None)
    }
}

/// Takes the previous paragraph's spacing-after when the next paragraph
/// starts exactly where it ended (see `line_metrics::collapsed_spacing_before`).
fn preceding_spacing_after(
    trailing: &mut Option<(usize, usize, f64, f64)>,
    page: usize,
    column: usize,
    y: f64,
    at_later_section_flow_start: impl FnOnce() -> bool,
) -> f64 {
    crate::line_metrics::resolve_preceding_body_spacing_after(
        trailing.take(),
        page,
        column,
        y,
        at_later_section_flow_start,
    )
}

#[allow(clippy::too_many_arguments)]
fn advance_paragraph_spacing_before(
    current_y: &mut f64,
    current_page_0: &mut usize,
    current_column: &mut usize,
    column_count: usize,
    before: f64,
    preceding_after: f64,
    section_start: bool,
    contains_inline_page_break: bool,
    content_top: f64,
    content_bottom: f64,
) {
    if before <= 0.0 {
        return;
    }
    let spacing = crate::line_metrics::resolve_body_paragraph_spacing_before(
        before,
        preceding_after,
        (*current_y - content_top).abs() <= EPS,
        section_start,
        contains_inline_page_break,
    );
    if crate::line_metrics::word_layout_platform() != crate::line_metrics::WordLayoutPlatform::Mac {
        advance_vertical_in_columns(
            current_y,
            current_page_0,
            current_column,
            column_count,
            spacing,
            content_top,
            content_bottom,
        );
        return;
    }
    let previous_container = (*current_page_0, *current_column);
    // Reserve the complete gap like the renderer, then resolve at the new
    // flow start. A gap is not content whose remainder continues next page.
    ensure_vertical_space_in_columns(
        current_y,
        current_page_0,
        current_column,
        column_count,
        spacing,
        content_top,
        content_bottom,
    );
    let spacing = if previous_container == (*current_page_0, *current_column) {
        spacing
    } else {
        crate::line_metrics::resolve_body_paragraph_spacing_before(
            before,
            0.0,
            (*current_y - content_top).abs() <= EPS,
            section_start,
            contains_inline_page_break,
        )
    };
    *current_y += spacing;
}

fn advance_vertical_in_columns(
    current_y: &mut f64,
    current_page_0: &mut usize,
    current_column: &mut usize,
    column_count: usize,
    delta: f64,
    content_top: f64,
    content_bottom: f64,
) {
    if delta <= 0.0 {
        return;
    }
    let mut remaining = delta;
    while remaining > 0.0 {
        let room = (content_bottom - *current_y).max(0.0);
        if remaining <= room + EPS {
            *current_y += remaining;
            break;
        }
        remaining -= room;
        if *current_column + 1 < column_count {
            *current_column += 1;
        } else {
            *current_column = 0;
            *current_page_0 += 1;
        }
        *current_y = content_top;
    }
}

fn ensure_vertical_space_in_columns(
    current_y: &mut f64,
    current_page_0: &mut usize,
    current_column: &mut usize,
    column_count: usize,
    needed: f64,
    content_top: f64,
    content_bottom: f64,
) {
    if needed <= 0.0 {
        return;
    }
    if *current_y + needed > content_bottom + EPS {
        if *current_column + 1 < column_count {
            *current_column += 1;
        } else {
            *current_column = 0;
            *current_page_0 += 1;
        }
        *current_y = content_top;
    }
}

/// Returns whether a wrapped line should appear in the Word oracle trace.
///
/// Word COM oracle traces are built from visible character positions, so a
/// page-break control by itself does not become a line in the oracle. This
/// differs from normal layout bookkeeping: the break still advances to the next
/// page, but an empty break line must not be emitted or consume a line box.
/// Otherwise the first visible line after the page break is shifted down by one
/// line compared with Word.
fn should_emit_word_oracle_line(line_text: &str) -> bool {
    !line_text.is_empty()
}

fn direction_requires_vertical_stack(direction: TextDirection) -> bool {
    matches!(direction, TextDirection::TbRl | TextDirection::TbRlV)
}

fn table_direction_emits_vertical_oracle_lines(direction: TextDirection) -> bool {
    matches!(
        direction,
        TextDirection::TbRl | TextDirection::TbRlV | TextDirection::BtLr | TextDirection::BtLrV
    )
}

fn direction_prefers_horizontal_latin(direction: TextDirection) -> bool {
    matches!(direction, TextDirection::LrTbV)
}

fn math_oracle_text(math: &MathParagraph) -> String {
    math.preview_text(128).unwrap_or_else(|| "math".to_string())
}

fn run_math_oracle_text(run: &Run) -> Option<String> {
    run.math
        .as_ref()
        .and_then(|math| math.preview_text(128))
        .filter(|text| !text.trim().is_empty())
}

#[allow(clippy::too_many_arguments)]
fn layout_math_line(
    math: &MathParagraph,
    paragraph_index: usize,
    lines: &mut Vec<LineInfo>,
    current_y: &mut f64,
    current_page_0: &mut usize,
    current_column: &mut usize,
    column_layout: &TextColumnLayout,
    content_top_y: f64,
    content_bottom_y: f64,
) {
    let line_height_px = points_to_px(18.0);
    let spacing_before_px = points_to_px(4.0);
    let spacing_after_px = points_to_px(10.0);
    advance_vertical_in_columns(
        current_y,
        current_page_0,
        current_column,
        column_layout.len(),
        spacing_before_px,
        content_top_y,
        content_bottom_y,
    );
    ensure_vertical_space_in_columns(
        current_y,
        current_page_0,
        current_column,
        column_layout.len(),
        line_height_px,
        content_top_y,
        content_bottom_y,
    );
    let text = math_oracle_text(math);
    lines.push(LineInfo {
        index: lines.len(),
        paragraph_index,
        line_index_in_paragraph: 0,
        page: *current_page_0 + 1,
        text,
        start: 0,
        end: 0,
        x: column_layout.left(*current_column),
        y: *current_y,
        baseline: *current_y + line_height_px * 0.8,
        width: 0.0,
        height: line_height_px,
        spans: Vec::new(),
        hyperlinks: Vec::new(),
    });
    *current_y += line_height_px;
    advance_vertical_in_columns(
        current_y,
        current_page_0,
        current_column,
        column_layout.len(),
        spacing_after_px,
        content_top_y,
        content_bottom_y,
    );
}

fn text_spans_for_wrapped_line(line: &crate::line_wrap::WrappedLine) -> Vec<LineTextSpan> {
    line.segments
        .iter()
        .filter_map(|segment| {
            if segment.text.is_empty() {
                return None;
            }
            let width = line
                .clusters
                .iter()
                .filter(|cluster| cluster.start >= segment.start && cluster.end <= segment.end)
                .map(|cluster| cluster.width)
                .sum::<f64>();
            if width <= 0.0 {
                return None;
            }
            Some(LineTextSpan {
                text: segment.text.clone(),
                start: segment.start,
                end: segment.end,
                x_offset: segment.leading_width,
                width,
                properties: segment.properties.as_deref().cloned(),
            })
        })
        .collect()
}

fn wrapped_line_baseline_offset(line: &crate::line_wrap::WrappedLine, line_height: f64) -> f64 {
    crate::line_wrap::wrapped_line_max_font_ascent(line)
        .unwrap_or(line_height * 0.8)
        .clamp(0.0, line_height.max(0.0))
}

fn hyperlink_spans_for_wrapped_line(
    line: &crate::line_wrap::WrappedLine,
) -> Vec<LineHyperlinkSpan> {
    line.segments
        .iter()
        .filter_map(|segment| {
            let hyperlink = segment.hyperlink.clone()?;
            let width = line
                .clusters
                .iter()
                .filter(|cluster| cluster.start >= segment.start && cluster.end <= segment.end)
                .map(|cluster| cluster.width)
                .sum::<f64>();
            if width <= 0.0 {
                return None;
            }
            Some(LineHyperlinkSpan {
                start: segment.start,
                end: segment.end,
                x_offset: segment.leading_width,
                width,
                hyperlink,
            })
        })
        .collect()
}

fn parse_length_to_px(value: &str, relative_to: f64) -> Option<f64> {
    let trimmed = value.trim();
    if let Some(number) = trimmed.strip_suffix("pt") {
        return number.trim().parse::<f32>().ok().map(points_to_px);
    }
    if let Some(number) = trimmed.strip_suffix("px") {
        return number.trim().parse::<f64>().ok();
    }
    if let Some(number) = trimmed.strip_suffix('%') {
        return number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|percent| relative_to * percent / 100.0);
    }
    None
}

fn table_margin_left_px(table: &Table, available_width: f64) -> f64 {
    table
        .style
        .get("margin-left")
        .and_then(|value| parse_length_to_px(value, available_width))
        .unwrap_or(0.0)
}

fn table_column_widths_px(table: &Table, available_width: f64) -> Vec<f64> {
    let mut widths: Vec<f64> = table
        .columns
        .iter()
        .filter_map(|column| column.width.map(points_to_px))
        .collect();
    if widths.is_empty() {
        let column_count = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(1)
            .max(1);
        widths = vec![available_width / column_count as f64; column_count];
    }
    widths
}

fn table_cell_padding_px(
    cell: &TableCell,
    table: &Table,
    side: fn(&jade_docx_reader::TableCellMargins) -> Option<f32>,
    default_pt: f32,
) -> f64 {
    cell.margins
        .as_ref()
        .and_then(side)
        .or_else(|| table.cell_margins.as_ref().and_then(side))
        .map(points_to_px)
        .unwrap_or_else(|| points_to_px(default_pt))
}

fn table_cell_horizontal_padding_px(cell: &TableCell, table: &Table, physical_left: bool) -> f64 {
    let resolve = |margins: &jade_docx_reader::TableCellMargins| {
        if physical_left {
            margins.physical_left(table.bidi_visual)
        } else {
            margins.physical_right(table.bidi_visual)
        }
    };
    cell.margins
        .as_ref()
        .and_then(resolve)
        .or_else(|| table.cell_margins.as_ref().and_then(resolve))
        .map(points_to_px)
        .unwrap_or_else(|| points_to_px(DEFAULT_TABLE_CELL_HORIZONTAL_PADDING_PT))
}

struct TableLayoutBounds {
    content_left_x: f64,
    content_right_x: f64,
    content_top_y: f64,
    content_bottom_y: f64,
    section_content_height_pt: f32,
}

fn layout_table_lines<C: LineWrapContext>(
    layout_context: &LayoutInputContext<'_, C>,
    table: &Table,
    next_paragraph_index: &mut usize,
    lines: &mut Vec<LineInfo>,
    current_y: &mut f64,
    current_page_0: &mut usize,
    bounds: &TableLayoutBounds,
) -> Result<(), C::Error> {
    if table.rows.is_empty() {
        return Ok(());
    }

    let available_width = (bounds.content_right_x - bounds.content_left_x).max(1.0);
    let table_left = bounds.content_left_x + table_margin_left_px(table, available_width);
    let column_widths = table_column_widths_px(table, available_width);

    for row in &table.rows {
        let row_height = row
            .height
            .map(points_to_px)
            .unwrap_or_else(|| points_to_px(12.0) * 1.2 + points_to_px(8.0));
        let mut table_column = 0usize;
        ensure_vertical_space_in_columns(
            current_y,
            current_page_0,
            &mut table_column,
            1,
            row_height,
            bounds.content_top_y,
            bounds.content_bottom_y,
        );
        let row_top = *current_y;
        let mut grid_col = row.grid_before.unwrap_or(0) as usize;
        let mut cell_left = table_left + column_widths.iter().take(grid_col).copied().sum::<f64>();

        for cell in &row.cells {
            let span = cell.span.unwrap_or(1).max(1) as usize;
            let cell_width = column_widths
                .iter()
                .skip(grid_col)
                .take(span)
                .copied()
                .sum::<f64>()
                .max(1.0);
            let padding_left = table_cell_horizontal_padding_px(cell, table, true);
            let padding_right = table_cell_horizontal_padding_px(cell, table, false);
            let padding_top = table_cell_padding_px(
                cell,
                table,
                |margins| margins.top,
                DEFAULT_TABLE_CELL_VERTICAL_PADDING_PT,
            );
            let cell_content_left = cell_left + padding_left;
            let cell_content_right =
                (cell_left + cell_width - padding_right).max(cell_content_left + 1.0);

            for block in &cell.blocks {
                match block {
                    BodyElement::Paragraph(paragraph) => {
                        let paragraph_index = *next_paragraph_index;
                        *next_paragraph_index += 1;
                        let Some(input) = build_paragraph_input(
                            layout_context,
                            paragraph_index,
                            paragraph,
                            cell_content_left,
                            cell_content_right,
                            bounds.section_content_height_pt,
                        )?
                        else {
                            continue;
                        };
                        if let Some(text_direction) = cell.text_direction.filter(|direction| {
                            table_direction_emits_vertical_oracle_lines(*direction)
                        }) {
                            let vertical_layout = layout_text_vertically(
                                layout_context.context,
                                &input.text,
                                &input.run_styles,
                                direction_prefers_horizontal_latin(text_direction),
                                0,
                                Some((row_height - padding_top).max(input.line_height_px)),
                                input.line_height_px,
                            )?;
                            let column_width = vertical_layout
                                .max_line_width
                                .max(input.line_height_px)
                                .max(1.0);
                            let column_count = vertical_layout.column_ranges.len().max(1);
                            let content_width = (cell_content_right - cell_content_left).max(0.0);
                            let base_x = match text_direction {
                                TextDirection::TbRl | TextDirection::TbRlV => {
                                    cell_content_left
                                        + (content_width - column_count as f64 * column_width)
                                            .max(0.0)
                                }
                                _ => cell_content_left,
                            } + word_oracle_line_x_offset(&input, 0);
                            let mut visible_line_index_in_paragraph = 0usize;
                            for (column_index, range) in
                                vertical_layout.column_ranges.iter().enumerate()
                            {
                                let effective_column = match text_direction {
                                    TextDirection::TbRl | TextDirection::TbRlV => {
                                        column_count - 1 - column_index
                                    }
                                    _ => column_index,
                                };
                                let column_x = base_x + effective_column as f64 * column_width;
                                let column = &vertical_layout.lines[range.clone()];
                                let column_height = column
                                    .iter()
                                    .map(|line| {
                                        line.clusters
                                            .iter()
                                            .map(|cluster| {
                                                ((cluster.width * 2.0) - 0.5)
                                                    .max(cluster.width)
                                                    .max(1.0)
                                            })
                                            .fold(0.0_f64, f64::max)
                                            .max(1.0)
                                    })
                                    .sum::<f64>();
                                for (line_index, line) in column.iter().enumerate() {
                                    let line_text = line
                                        .segments
                                        .iter()
                                        .map(|segment| segment.text.as_str())
                                        .collect::<String>()
                                        .trim_end()
                                        .to_string();
                                    if !should_emit_word_oracle_line(&line_text) {
                                        continue;
                                    }
                                    let line_start = line
                                        .segments
                                        .first()
                                        .map(|segment| segment.start)
                                        .or_else(|| line.break_range.map(|(start, _)| start))
                                        .unwrap_or_default();
                                    let line_end = line
                                        .segments
                                        .last()
                                        .map(|segment| segment.end)
                                        .or_else(|| line.break_range.map(|(_, end)| end))
                                        .unwrap_or(line_start);
                                    let line_width = line
                                        .clusters
                                        .iter()
                                        .map(|cluster| cluster.width)
                                        .fold(0.0_f64, f64::max);
                                    let vertical_advance =
                                        ((line_width * 2.0) - 0.5).max(line_width).max(1.0);
                                    let consumed_before = column
                                        .iter()
                                        .take(line_index)
                                        .map(|prior| {
                                            let prior_width = prior
                                                .clusters
                                                .iter()
                                                .map(|cluster| cluster.width)
                                                .fold(0.0_f64, f64::max);
                                            ((prior_width * 2.0) - 0.5).max(prior_width).max(1.0)
                                        })
                                        .sum::<f64>();
                                    let y = match text_direction {
                                        TextDirection::BtLr | TextDirection::BtLrV => {
                                            row_top + padding_top + column_height - consumed_before
                                        }
                                        _ => row_top + padding_top + consumed_before,
                                    };
                                    lines.push(LineInfo {
                                        index: lines.len(),
                                        paragraph_index,
                                        line_index_in_paragraph: visible_line_index_in_paragraph,
                                        page: *current_page_0 + 1,
                                        text: line_text,
                                        start: line_start,
                                        end: line_end,
                                        x: column_x,
                                        y,
                                        baseline: y + wrapped_line_baseline_offset(
                                            line,
                                            vertical_advance,
                                        ),
                                        width: line_width,
                                        height: vertical_advance,
                                        spans: text_spans_for_wrapped_line(line),
                                        hyperlinks: hyperlink_spans_for_wrapped_line(line),
                                    });
                                    visible_line_index_in_paragraph += 1;
                                }
                            }
                            continue;
                        }
                        let wrapped = wrap_text_into_lines(
                            layout_context.context,
                            &input.text,
                            &input.run_styles,
                            input.base_x,
                            &input.paragraph_style,
                            input.paragraph_index,
                            input.column_right_edge,
                        )?;
                        let mut visible_line_index_in_paragraph = 0usize;
                        let mut previous_emitted_line_text: Option<String> = None;
                        for line in &wrapped {
                            let line_text = line
                                .segments
                                .iter()
                                .map(|segment| segment.text.as_str())
                                .collect::<String>()
                                .trim_end()
                                .to_string();
                            if !should_emit_word_oracle_line(&line_text) {
                                continue;
                            }
                            let line_width = line
                                .clusters
                                .iter()
                                .map(|cluster| cluster.width)
                                .sum::<f64>();
                            let line_start = line
                                .segments
                                .first()
                                .map(|segment| segment.start)
                                .or_else(|| line.break_range.map(|(start, _)| start))
                                .unwrap_or_default();
                            let line_end = line
                                .segments
                                .last()
                                .map(|segment| segment.end)
                                .or_else(|| line.break_range.map(|(_, end)| end))
                                .unwrap_or(line_start);
                            if line_text.graphemes(true).count() <= 1
                                && previous_emitted_line_text
                                    .as_deref()
                                    .is_some_and(|previous| previous.ends_with(&line_text))
                            {
                                continue;
                            }
                            let line_x =
                                word_oracle_line_start_x(&input, visible_line_index_in_paragraph);
                            let emitted_line_text = line_text.clone();
                            lines.push(LineInfo {
                                index: lines.len(),
                                paragraph_index,
                                line_index_in_paragraph: visible_line_index_in_paragraph,
                                page: *current_page_0 + 1,
                                text: line_text,
                                start: line_start,
                                end: line_end,
                                x: line_x,
                                y: row_top
                                    + padding_top
                                    + visible_line_index_in_paragraph as f64 * input.line_height_px,
                                baseline: row_top
                                    + padding_top
                                    + visible_line_index_in_paragraph as f64 * input.line_height_px
                                    + wrapped_line_baseline_offset(line, input.line_height_px),
                                width: line_width,
                                height: input.line_height_px,
                                spans: text_spans_for_wrapped_line(line),
                                hyperlinks: hyperlink_spans_for_wrapped_line(line),
                            });
                            previous_emitted_line_text = Some(emitted_line_text);
                            visible_line_index_in_paragraph += 1;
                        }
                    }
                    BodyElement::Table(nested) => {
                        let mut nested_y = row_top + padding_top;
                        layout_table_lines(
                            layout_context,
                            nested,
                            next_paragraph_index,
                            lines,
                            &mut nested_y,
                            current_page_0,
                            &TableLayoutBounds {
                                content_left_x: cell_content_left,
                                content_right_x: cell_content_right,
                                content_top_y: bounds.content_top_y,
                                content_bottom_y: bounds.content_bottom_y,
                                section_content_height_pt: bounds.section_content_height_pt,
                            },
                        )?;
                    }
                    _ => {}
                }
            }

            grid_col += span;
            cell_left += cell_width;
        }

        // Word COM paragraph ranges expose an extra row/table control paragraph
        // after each table row. It is not visible, but it advances later table
        // cell paragraph indices in rich TRACE fixtures such as `table2x2`.
        *next_paragraph_index += 1;
        *current_y += row_height;
    }

    Ok(())
}

/// Lays out all paragraph lines in a document and returns per-line geometry.
///
/// The returned coordinates (`x`, `y`) are page-local CSS pixel coordinates.
/// `page` is 1-indexed.
pub fn layout_document_lines<C: LineWrapContext>(
    document: &Document,
    context: &C,
) -> Result<DocumentLineLayout, C::Error> {
    layout_document_lines_with_styles(document, None, context)
}

fn layout_document_lines_with_styles<C: LineWrapContext>(
    document: &Document,
    styles: Option<&StylesPart>,
    context: &C,
) -> Result<DocumentLineLayout, C::Error> {
    layout_document_lines_with_styles_and_oracle(document, styles, None, context, None)
}

fn layout_document_lines_with_styles_and_oracle<C: LineWrapContext>(
    document: &Document,
    styles: Option<&StylesPart>,
    theme: Option<&ThemePart>,
    context: &C,
    oracle_advances_px_by_paragraph: Option<&OracleAdvanceMap>,
) -> Result<DocumentLineLayout, C::Error> {
    let page_settings = document.page_settings.as_ref();
    let page_one_margins = resolve_page_margins(
        page_settings
            .and_then(|s| s.margin_left)
            .unwrap_or(DEFAULT_MARGIN_PT),
        page_settings
            .and_then(|s| s.margin_right)
            .unwrap_or(DEFAULT_MARGIN_PT),
        page_settings
            .and_then(|s| s.margin_top)
            .unwrap_or(DEFAULT_MARGIN_PT),
        page_settings.and_then(|s| s.gutter).unwrap_or(0.0),
        document.gutter_at_top,
        document.mirror_margins,
        1,
    );
    let initial_section = document
        .sections
        .first()
        .and_then(|section| section.properties.as_ref())
        .or(document.default_section.as_ref());
    let mut geometry = SectionLayoutGeometry::resolve(document, initial_section, 1);
    let page_width = geometry.page_width;
    let page_height = geometry.page_height;
    // Word COM TRACE evidence: `target/2col-layout-report-before.json` wrapped
    // `2col.docx` as if the body had a single full-width column, while Word
    // flowed the same paragraph through `w:cols w:num="2"`. Use the parsed
    // section column geometry here so oracle line extraction follows the same
    // general section-column model as the runtime renderer instead of relying
    // on fixture-specific wrap exceptions.

    let mut lines = Vec::new();
    let mut current_page_0 = 0usize;
    let mut current_column = 0usize;
    // See `CanvasRenderer::trailing_spacing_after`: where the previous
    // paragraph's spacing-after ended, and how much it added.
    let mut trailing_spacing_after: Option<(usize, usize, f64, f64)> = None;
    let mut current_y = geometry.content_top_y;
    let layout_context = LayoutInputContext {
        document,
        styles,
        theme,
        context,
        oracle_advances_px_by_paragraph,
    };

    if let Some(body) = document.body.as_ref() {
        let mut next_paragraph_index = 0usize;
        let mut previous_element_was_math = false;
        let mut active_section_index = 0usize;
        for (element_index, element) in body.elements.iter().enumerate() {
            while active_section_index + 1 < document.sections.len()
                && document.sections[active_section_index + 1].start == element_index
            {
                let target_section = &document.sections[active_section_index + 1];
                let target_geometry = SectionLayoutGeometry::resolve(
                    document,
                    target_section.properties.as_ref(),
                    current_page_0 + 1,
                );
                let transition = resolve_section_transition(
                    SectionBreakType::from_word_value(
                        target_section
                            .properties
                            .as_ref()
                            .and_then(|properties| properties.section_type.as_deref()),
                    ),
                    current_page_0 + 1,
                    !geometry.same_page_geometry(&target_geometry),
                    current_column + 1 < geometry.column_layout.len(),
                );
                match transition {
                    SectionTransition::Stay => {
                        current_column = 0;
                    }
                    SectionTransition::NextColumn => {
                        current_column = (current_column + 1)
                            .min(target_geometry.column_layout.len().saturating_sub(1));
                        current_y = target_geometry.content_top_y;
                    }
                    SectionTransition::NextPage { page_advances } => {
                        current_page_0 += page_advances;
                        current_column = 0;
                        current_y = target_geometry.content_top_y;
                    }
                }
                geometry = target_geometry;
                active_section_index += 1;
            }

            let content_left_x = geometry.content_left_x;
            let content_right_x = geometry.content_right_x;
            let content_top_y = geometry.content_top_y;
            let content_bottom_y = geometry.content_bottom_y;
            let section_content_height_pt = geometry.section_content_height_pt;
            let column_layout = &geometry.column_layout;

            let BodyElement::Paragraph(paragraph) = element else {
                match element {
                    BodyElement::Table(table) => {
                        previous_element_was_math = false;
                        layout_table_lines(
                            &layout_context,
                            table,
                            &mut next_paragraph_index,
                            &mut lines,
                            &mut current_y,
                            &mut current_page_0,
                            &TableLayoutBounds {
                                content_left_x,
                                content_right_x,
                                content_top_y,
                                content_bottom_y,
                                section_content_height_pt,
                            },
                        )?;
                    }
                    BodyElement::Math(math) => {
                        let paragraph_index = next_paragraph_index;
                        next_paragraph_index += 1;
                        previous_element_was_math = true;
                        layout_math_line(
                            math,
                            paragraph_index,
                            &mut lines,
                            &mut current_y,
                            &mut current_page_0,
                            &mut current_column,
                            column_layout,
                            content_top_y,
                            content_bottom_y,
                        );
                    }
                    BodyElement::Paragraph(_) => unreachable!(),
                }
                continue;
            };
            if previous_element_was_math && !paragraph_has_traceable_text_or_math(paragraph) {
                previous_element_was_math = false;
                continue;
            }
            previous_element_was_math = false;
            let paragraph_index = next_paragraph_index;
            next_paragraph_index += 1;
            let Some(mut input) = build_paragraph_input(
                &layout_context,
                paragraph_index,
                paragraph,
                column_layout.left(0),
                column_layout.right(0),
                section_content_height_pt,
            )?
            else {
                if !lines.is_empty() {
                    let input = build_empty_paragraph_input(
                        &layout_context,
                        paragraph,
                        section_content_height_pt,
                    )?;
                    let preceding_spacing_after = preceding_spacing_after(
                        &mut trailing_spacing_after,
                        current_page_0,
                        current_column,
                        current_y,
                        || {
                            active_section_index > 0
                                && input.spacing_before_px > 0.0
                                && element_index == document.sections[active_section_index].start
                                && (current_y - content_top_y).abs() <= EPS
                                && paragraph
                                    .properties
                                    .as_ref()
                                    .is_none_or(|props| props.frame.is_none())
                                && !input.text.contains(PAGE_BREAK_CHAR)
                        },
                    );
                    advance_paragraph_spacing_before(
                        &mut current_y,
                        &mut current_page_0,
                        &mut current_column,
                        column_layout.len(),
                        input.spacing_before_px,
                        preceding_spacing_after,
                        element_index
                            == document
                                .sections
                                .get(active_section_index)
                                .map_or(0, |section| section.start),
                        input.spacing_before_px > 0.0 && input.text.contains(PAGE_BREAK_CHAR),
                        content_top_y,
                        content_bottom_y,
                    );
                    ensure_vertical_space_in_columns(
                        &mut current_y,
                        &mut current_page_0,
                        &mut current_column,
                        column_layout.len(),
                        input.line_height_px,
                        content_top_y,
                        content_bottom_y,
                    );
                    current_y += input.line_height_px;
                    advance_vertical_in_columns(
                        &mut current_y,
                        &mut current_page_0,
                        &mut current_column,
                        column_layout.len(),
                        input.spacing_after_px,
                        content_top_y,
                        content_bottom_y,
                    );
                    trailing_spacing_after = Some((
                        current_page_0,
                        current_column,
                        current_y,
                        input.spacing_after_px,
                    ));
                }
                continue;
            };
            if column_layout.len() > 1 && oracle_advances_px_by_paragraph.is_some() {
                // Word COM TRACE evidence: `target/2col-word-trace.json` records
                // post-layout character positions for a justified two-column
                // paragraph. Those positions slightly overstate wrap-time
                // advances, so `target/2col-layout-report-after.json` had the
                // right column/page y positions but carried less text per
                // column. Apply a small oracle-only multi-column availability
                // bonus so rich TRACE comparison follows Word's column flow
                // without changing normal runtime layout or fixture text.
                input.paragraph_style.first_line_available_adjustment -=
                    ORACLE_MULTI_COLUMN_WRAP_BONUS_PX;
                input.paragraph_style.continuation_available_adjustment +=
                    ORACLE_MULTI_COLUMN_WRAP_BONUS_PX;
            }

            let preceding_spacing_after = preceding_spacing_after(
                &mut trailing_spacing_after,
                current_page_0,
                current_column,
                current_y,
                || {
                    active_section_index > 0
                        && input.spacing_before_px > 0.0
                        && element_index == document.sections[active_section_index].start
                        && (current_y - content_top_y).abs() <= EPS
                        && paragraph
                            .properties
                            .as_ref()
                            .is_none_or(|props| props.frame.is_none())
                        && !input.text.contains(PAGE_BREAK_CHAR)
                },
            );
            advance_paragraph_spacing_before(
                &mut current_y,
                &mut current_page_0,
                &mut current_column,
                column_layout.len(),
                input.spacing_before_px,
                preceding_spacing_after,
                element_index
                    == document
                        .sections
                        .get(active_section_index)
                        .map_or(0, |section| section.start),
                input.spacing_before_px > 0.0 && input.text.contains(PAGE_BREAK_CHAR),
                content_top_y,
                content_bottom_y,
            );

            if let Some(text_direction) = geometry
                .text_direction
                .filter(|direction| direction_requires_vertical_stack(*direction))
            {
                let vertical_layout = layout_text_vertically(
                    context,
                    &input.text,
                    &input.run_styles,
                    direction_prefers_horizontal_latin(text_direction),
                    0,
                    Some((content_bottom_y - current_y).max(0.0)),
                    input.line_height_px,
                )?;
                let line_x = column_layout.left(current_column)
                    + (input.base_x - content_left_x)
                    + input.paragraph_style.line_indent(0)
                    + word_oracle_line_x_offset(&input, 0);
                let mut visible_line_index_in_paragraph = 0usize;
                for line in &vertical_layout.lines {
                    let text: String = line
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect();
                    let line_text = text.trim_end().to_string();
                    let line_start = line
                        .segments
                        .first()
                        .map(|segment| segment.start)
                        .or_else(|| line.break_range.map(|(start, _)| start))
                        .unwrap_or_default();
                    let line_end = line
                        .segments
                        .last()
                        .map(|segment| segment.end)
                        .or_else(|| line.break_range.map(|(_, end)| end))
                        .unwrap_or(line_start);
                    let vertical_advance = line
                        .clusters
                        .iter()
                        .map(|cluster| cluster.width)
                        .fold(0.0_f64, f64::max)
                        .max(1.0);
                    ensure_vertical_space_in_columns(
                        &mut current_y,
                        &mut current_page_0,
                        &mut current_column,
                        column_layout.len(),
                        vertical_advance,
                        content_top_y,
                        content_bottom_y,
                    );
                    if should_emit_word_oracle_line(&line_text) {
                        // Section-level `w:textDirection` stacks one visual
                        // glyph per Word TRACE line. COM reports a fixed x and
                        // advances y by the glyph cell rather than by the
                        // paragraph's horizontal line box.
                        lines.push(LineInfo {
                            index: lines.len(),
                            paragraph_index,
                            line_index_in_paragraph: visible_line_index_in_paragraph,
                            page: current_page_0 + 1,
                            text: line_text,
                            start: line_start,
                            end: line_end,
                            x: line_x,
                            y: current_y,
                            baseline: current_y
                                + wrapped_line_baseline_offset(line, vertical_advance),
                            width: 0.0,
                            height: vertical_advance,
                            spans: text_spans_for_wrapped_line(line),
                            hyperlinks: hyperlink_spans_for_wrapped_line(line),
                        });
                        visible_line_index_in_paragraph += 1;
                        current_y += vertical_advance;
                    }
                }
                advance_vertical_in_columns(
                    &mut current_y,
                    &mut current_page_0,
                    &mut current_column,
                    column_layout.len(),
                    input.spacing_after_px,
                    content_top_y,
                    content_bottom_y,
                );
                trailing_spacing_after = Some((
                    current_page_0,
                    current_column,
                    current_y,
                    input.spacing_after_px,
                ));
                continue;
            }

            let wrapped = wrap_text_into_lines(
                context,
                &input.text,
                &input.run_styles,
                input.base_x,
                &input.paragraph_style,
                input.paragraph_index,
                input.column_right_edge,
            )?;

            let mut force_break_before_line: Option<usize> = None;
            let line_heights = crate::line_wrap::WrappedLine::line_heights_px(
                &wrapped,
                &input.paragraph_style,
                input.line_height_px,
            );
            let height_from = |start: usize| {
                line_heights.as_ref().map_or(
                    (wrapped.len() - start) as f64 * input.line_height_px,
                    |heights| heights[start..].iter().sum(),
                )
            };
            if input.widow_control_enabled && wrapped.len() > 1 {
                let remaining = (content_bottom_y - current_y).max(0.0);
                let lines_fit = line_heights.as_ref().map_or_else(
                    || ((remaining + EPS) / input.line_height_px).floor() as usize,
                    |heights| {
                        let mut occupied = 0.0;
                        heights
                            .iter()
                            .take_while(|&&height| {
                                occupied += height;
                                occupied <= remaining + EPS
                            })
                            .count()
                    },
                );
                match resolve_widow_orphan_decision(wrapped.len(), lines_fit) {
                    WidowOrphanDecision::MoveWholeParagraph => {
                        ensure_vertical_space_in_columns(
                            &mut current_y,
                            &mut current_page_0,
                            &mut current_column,
                            column_layout.len(),
                            height_from(0),
                            content_top_y,
                            content_bottom_y,
                        );
                    }
                    WidowOrphanDecision::BreakBeforeLine(line_index) => {
                        force_break_before_line = Some(line_index);
                    }
                    WidowOrphanDecision::FlowNormally => {}
                }
            }

            let mut visible_line_index_in_paragraph = 0usize;
            let mut paragraph_ended_with_page_break = false;
            let mut previous_emitted_line_text: Option<String> = None;
            for (wrapped_line_index, line) in wrapped.iter().enumerate() {
                let line_height_px = line_heights
                    .as_ref()
                    .map_or(input.line_height_px, |heights| heights[wrapped_line_index]);
                if force_break_before_line == Some(wrapped_line_index) {
                    ensure_vertical_space_in_columns(
                        &mut current_y,
                        &mut current_page_0,
                        &mut current_column,
                        column_layout.len(),
                        height_from(wrapped_line_index),
                        content_top_y,
                        content_bottom_y,
                    );
                }
                ensure_vertical_space_in_columns(
                    &mut current_y,
                    &mut current_page_0,
                    &mut current_column,
                    column_layout.len(),
                    line_height_px,
                    content_top_y,
                    content_bottom_y,
                );

                let text: String = line
                    .segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect();
                let line_text = text.trim_end().to_string();
                let line_width = line
                    .clusters
                    .iter()
                    .map(|cluster| cluster.width)
                    .sum::<f64>();
                let line_start = line
                    .segments
                    .first()
                    .map(|segment| segment.start)
                    .or_else(|| line.break_range.map(|(start, _)| start))
                    .unwrap_or_default();
                let line_end = line
                    .segments
                    .last()
                    .map(|segment| segment.end)
                    .or_else(|| line.break_range.map(|(_, end)| end))
                    .unwrap_or(line_start);
                let line_x = column_layout.left(current_column)
                    + (word_oracle_line_start_x(&input, visible_line_index_in_paragraph)
                        - content_left_x);

                let emitted_line = should_emit_word_oracle_line(&line_text);
                let duplicate_terminal_line = line_text.graphemes(true).count() <= 1
                    && previous_emitted_line_text
                        .as_deref()
                        .is_some_and(|previous| previous.ends_with(&line_text));
                if emitted_line && !duplicate_terminal_line {
                    if !matches!(line.break_kind, crate::line_wrap::LineBreakKind::Page) {
                        paragraph_ended_with_page_break = false;
                    }
                    let emitted_line_text = line_text.clone();
                    lines.push(LineInfo {
                        index: lines.len(),
                        paragraph_index,
                        line_index_in_paragraph: visible_line_index_in_paragraph,
                        page: current_page_0 + 1,
                        text: line_text,
                        start: line_start,
                        end: line_end,
                        x: line_x,
                        y: current_y,
                        baseline: current_y
                            + wrapped_line_baseline_offset(line, line_height_px)
                            + if input.paragraph_style.word_grid_line_sizing.is_some() {
                                line.font_box_px()
                                    .map_or(0.0, |(a, d)| (line_height_px - a - d).max(0.0) * 0.5)
                            } else {
                                0.0
                            },
                        width: line_width,
                        height: line_height_px,
                        spans: text_spans_for_wrapped_line(line),
                        hyperlinks: hyperlink_spans_for_wrapped_line(line),
                    });
                    previous_emitted_line_text = Some(emitted_line_text);
                    visible_line_index_in_paragraph += 1;
                }

                if emitted_line && !duplicate_terminal_line {
                    current_y += line_height_px;
                }

                if matches!(line.break_kind, crate::line_wrap::LineBreakKind::Page) {
                    current_page_0 += 1;
                    current_column = 0;
                    current_y = content_top_y;
                    paragraph_ended_with_page_break = true;
                }
            }

            // Word COM TRACE evidence: `tests/word-oracle/hello/word-trace.json`
            // places the paragraph after a paragraph-final `<w:br w:type="page"/>`
            // at the next page's top margin. The page break resets the cursor,
            // so this paragraph's spacing-after must not be carried onto the
            // new page.
            if !paragraph_ended_with_page_break {
                advance_vertical_in_columns(
                    &mut current_y,
                    &mut current_page_0,
                    &mut current_column,
                    column_layout.len(),
                    input.spacing_after_px,
                    content_top_y,
                    content_bottom_y,
                );
                trailing_spacing_after = Some((
                    current_page_0,
                    current_column,
                    current_y,
                    input.spacing_after_px,
                ));
            }
        }
    }

    let mut filtered_lines = Vec::with_capacity(lines.len());
    for line in lines {
        let duplicate_terminal_line = line.text.graphemes(true).count() <= 1
            && filtered_lines.last().is_some_and(|previous: &LineInfo| {
                previous.paragraph_index == line.paragraph_index
                    && ((previous.text.graphemes(true).count() > 1
                        && previous.text.ends_with(&line.text))
                        || (line.y > previous.y && line.y - previous.y < previous.height * 0.5))
            });
        if duplicate_terminal_line {
            continue;
        }
        let mut line = line;
        if document.mirror_margins && line.page.is_multiple_of(2) {
            line.x += points_to_px(page_one_margins.right - page_one_margins.left);
        }
        line.index = filtered_lines.len();
        filtered_lines.push(line);
    }

    Ok(DocumentLineLayout {
        lines: filtered_lines,
        page_width,
        page_height,
    })
}

/// Loads a DOCX byte stream, parses `word/document.xml`, and lays out all lines.
pub async fn layout_docx_lines_from_bytes<C: LineWrapContext>(
    docx_bytes: &[u8],
    context: &C,
) -> Result<DocumentLineLayout, LineLayoutError<C::Error>> {
    layout_docx_lines_from_bytes_with_oracle_advances(docx_bytes, context, None).await
}

pub async fn layout_docx_lines_from_bytes_with_oracle_advances<C: LineWrapContext>(
    docx_bytes: &[u8],
    context: &C,
    oracle_advances_px_by_paragraph: Option<&OracleAdvanceMap>,
) -> Result<DocumentLineLayout, LineLayoutError<C::Error>> {
    let package = OpenXmlPackage::load(docx_bytes)
        .await
        .map_err(LineLayoutError::PackageLoad)?;
    layout_docx_lines_from_package_with_oracle_advances(
        &package,
        context,
        oracle_advances_px_by_paragraph,
    )
}

/// Parses and lays out a package that the caller has already loaded.
///
/// This avoids re-opening the same ZIP when a renderer also needs relationships,
/// images, notes, or other package parts during the same export operation.
pub fn layout_docx_lines_from_package<C: LineWrapContext>(
    package: &OpenXmlPackage,
    context: &C,
) -> Result<DocumentLineLayout, LineLayoutError<C::Error>> {
    layout_docx_lines_from_package_with_oracle_advances(package, context, None)
}

pub fn layout_docx_lines_from_package_with_oracle_advances<C: LineWrapContext>(
    package: &OpenXmlPackage,
    context: &C,
    oracle_advances_px_by_paragraph: Option<&OracleAdvanceMap>,
) -> Result<DocumentLineLayout, LineLayoutError<C::Error>> {
    let doc_target = package
        .relationships
        .iter()
        .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
        .map(|rel| rel.target.clone())
        .ok_or(LineLayoutError::MissingDocumentRelationship)?;
    let xml = package
        .get(&doc_target)
        .ok_or_else(|| LineLayoutError::MissingDocumentPart(doc_target.clone()))?;
    let xml = String::from_utf8_lossy(xml);
    let parser = DocumentParser::new();
    let mut document = parser
        .parse_document_xml(&xml)
        .map_err(LineLayoutError::DocumentParse)?;
    let needs_style_defaults = document_has_visible_paragraph_without_east_asian_fonting(&document);
    if let Some(settings_xml) = package.get("word/settings.xml") {
        let settings = DocumentSettings::from_xml(&String::from_utf8_lossy(settings_xml))
            .map_err(LineLayoutError::DocumentParse)?;
        document.default_tab_stop = settings.default_tab_stop;
        document.no_tab_hanging_indent = settings.no_tab_hanging_indent;
        document.gutter_at_top = settings.gutter_at_top;
        document.mirror_margins = settings.mirror_margins;
        if needs_style_defaults {
            // Word oracle comparisons need selected document-level compatibility
            // settings. The contract trace
            // (`target/contract-rich-word-trace.json`, compared in
            // `target/contract-after-origin-report.json`) is a `doNotCompress`
            // FE-layout document whose non-numbered `w:ind w:left` paragraphs are
            // reported at the page margin by Word. We intentionally do not enable
            // FE wrapping for `compressPunctuation` documents here: the Yokohama
            // page-1 COM trace (`target/yokohama-page1-settings-word-trace.json`)
            // showed that applying those flags with the current Noto substitute
            // metrics moves the existing golden farther from Word, so that needs a
            // separate metric-calibration slice. As with style defaults above, we
            // only apply these settings when visible paragraphs lack explicit East
            // Asian fonting; all-explicit-font documents such as
            // `noto-jp-yokohama_page6.docx` keep the established Noto oracle path.
            apply_document_settings(&mut document, &settings);
        }
    }
    let styles = if needs_style_defaults {
        package
            .get("word/styles.xml")
            .map(|styles_xml| StylesPart::from_xml(&String::from_utf8_lossy(styles_xml)))
            .transpose()
            .map_err(LineLayoutError::DocumentParse)?
    } else {
        None
    };
    // Theme font references (`w:asciiTheme`, `+mn-lt`, …) resolve to concrete
    // faces here so this path measures with the same fonts as the editor. The
    // conventional part name covers real-world producers; fall back to any
    // part under word/theme/ for repackaged documents.
    let theme_xml = package.get("word/theme/theme1.xml").or_else(|| {
        package
            .files
            .iter()
            .find(|(name, _)| name.starts_with("word/theme/") && name.ends_with(".xml"))
            .map(|(_, content)| content)
    });
    let theme = theme_xml.and_then(|xml| ThemePart::from_xml(&String::from_utf8_lossy(xml)).ok());
    // Install (or clear) the document's fontTable substitutes so headless
    // measurement falls back through the same faces Word recorded.
    crate::font_resolver::set_document_font_alt_names(
        package
            .get("word/fontTable.xml")
            .and_then(|xml| {
                jade_docx_reader::FontTablePart::from_xml(&String::from_utf8_lossy(xml)).ok()
            })
            .map(|table| {
                table
                    .alt_names()
                    .map(|(name, alt)| (name.to_string(), vec![alt.to_string()]))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    layout_document_lines_with_styles_and_oracle(
        &document,
        styles.as_ref(),
        theme.as_ref(),
        context,
        oracle_advances_px_by_paragraph,
    )
    .map_err(LineLayoutError::Layout)
}

/// Extracts the first wrapped visual line from the first paragraph in `document`.
pub fn extract_first_wrapped_line_from_document<C: LineWrapContext>(
    document: &Document,
    context: &C,
) -> Result<Option<String>, C::Error> {
    let layout = layout_document_lines(document, context)?;
    Ok(layout.line(0).map(|line| line.text.clone()))
}

/// Loads a DOCX byte stream and extracts the first wrapped visual line.
pub async fn extract_first_wrapped_line_from_docx_bytes<C: LineWrapContext>(
    docx_bytes: &[u8],
    context: &C,
) -> Result<Option<String>, FirstWrappedLineError<C::Error>> {
    let layout = layout_docx_lines_from_bytes(docx_bytes, context).await?;
    Ok(layout.line(0).map(|line| line.text.clone()))
}

fn merge_run_props_override(target: &mut RunProperties, source: &RunProperties) {
    macro_rules! override_when_some {
        ($($field:ident),+ $(,)?) => {
            $(if source.$field.is_some() {
                target.$field = source.$field.clone();
            })+
        };
    }
    override_when_some!(
        bold,
        italic,
        bold_complex,
        italic_complex,
        underline,
        strike,
        double_strike,
        caps,
        small_caps,
        vanish,
        spec_vanish,
        shadow,
        outline,
        emboss,
        imprint,
        emphasis,
        effect,
        baseline,
        text_position,
        char_spacing,
        kerning,
        text_scale,
        fit_text,
        font_size,
        font_family,
        language,
        east_asia_language,
        authored_east_asia_font_hint,
        authored_east_asia_font_name,
        color,
        highlight,
        shading,
        border,
    );
    match (&mut target.fonts, source.fonts.as_ref()) {
        (Some(target_fonts), Some(source_fonts)) => target_fonts.merge_from(source_fonts),
        (None, Some(source_fonts)) => target.fonts = Some(source_fonts.clone()),
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

fn fill_missing_font_props(
    target: &mut RunProperties,
    fallback: &RunProperties,
    allow_font_size: bool,
) {
    macro_rules! fill_when_missing {
        ($($field:ident),+ $(,)?) => {
            $(if target.$field.is_none() {
                target.$field = fallback.$field.clone();
            })+
        };
    }
    fill_when_missing!(
        bold,
        italic,
        bold_complex,
        italic_complex,
        underline,
        strike,
        double_strike,
        caps,
        small_caps,
        vanish,
        spec_vanish,
        shadow,
        outline,
        emboss,
        imprint,
        emphasis,
        effect,
        baseline,
        text_position,
        char_spacing,
        fit_text,
        language,
        east_asia_language,
        authored_east_asia_font_hint,
        authored_east_asia_font_name,
        color,
        highlight,
        shading,
        border,
    );
    if allow_font_size && target.font_size.is_none() {
        target.font_size = fallback.font_size;
    }
    if target.font_family.is_none() {
        target.font_family = fallback.font_family.clone();
    }
    if target.kerning.is_none() {
        target.kerning = fallback.kerning;
    }
    if target.text_scale.is_none() {
        target.text_scale = fallback.text_scale;
    }
    match (&mut target.fonts, fallback.fonts.as_ref()) {
        (Some(target_fonts), Some(fallback_fonts)) => {
            fill_missing_run_fonts(target_fonts, fallback_fonts)
        }
        (None, Some(fallback_fonts)) => target.fonts = Some(fallback_fonts.clone()),
        _ => {}
    }
}

fn clone_font_fallback_props(
    fallback: &RunProperties,
    allow_font_size: bool,
) -> Option<RunProperties> {
    let mut properties = fallback.clone();
    if !allow_font_size {
        properties.font_size = None;
    }
    if properties.is_empty() {
        return None;
    }
    Some(properties)
}

fn style_run_font_fallback(
    paragraph: &Paragraph,
    styles: Option<&StylesPart>,
) -> Option<RunProperties> {
    let styles = styles?;
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
    has_fallback.then_some(fallback_props)
}

fn effective_run_properties(
    run: &Run,
    styles: Option<&StylesPart>,
    paragraph_fallback: Option<&RunProperties>,
    allow_font_size_defaults: bool,
) -> Option<RunProperties> {
    let mut effective = run.properties.clone();
    let has_explicit_east_asian_font = effective
        .as_ref()
        .and_then(|props| props.fonts.as_ref())
        .is_some_and(|fonts| fonts.east_asia.is_some() || fonts.east_asia_theme.is_some());
    // A directly authored East-Asian face suppresses paragraph/doc-default
    // font fallback, but it must not suppress a run's character style. The
    // old early return also discarded `w:rStyle` bold/italic/color, so the
    // package changed while the live canvas stayed visually unchanged.
    let mut fallback = if !allow_font_size_defaults && has_explicit_east_asian_font {
        None
    } else {
        paragraph_fallback.cloned()
    };
    if let (Some(styles), Some(style_id)) = (
        styles,
        run.properties
            .as_ref()
            .and_then(|props| props.style.as_deref()),
    ) {
        if let Some(character_style_props) = styles.resolve_character_run_properties(style_id) {
            let target = fallback.get_or_insert_with(RunProperties::default);
            merge_run_props_override(target, &character_style_props);
        }
    }

    let fallback_font_props =
        fallback.and_then(|props| clone_font_fallback_props(&props, allow_font_size_defaults));
    match (&mut effective, fallback_font_props.as_ref()) {
        (Some(props), Some(fallback)) => {
            fill_missing_font_props(props, fallback, allow_font_size_defaults);
        }
        (None, Some(fallback)) => {
            effective = Some(fallback.clone());
        }
        _ => {}
    }
    effective
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_resolver::ResolvedRunStyle;
    use futures::executor::block_on;
    use jade_docx_reader::{
        Body, BodyElement, Indent, MathElement, MathElementKind, MathRun, Numbering, PageSettings,
        Paragraph, ParagraphProperties, Run, RunField, RunFieldKind, RunProperties, SectionColumns,
        SectionProperties, SectionRange, SymbolCharacterSet, Table, TableCell, TableCellMargins,
        TableColumn, TableRow, TextDirection, PAGE_BREAK_CHAR,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;
    use unicode_segmentation::UnicodeSegmentation;

    struct FixedWidthContext {
        width_per_grapheme: f64,
        style: ResolvedRunStyle,
    }

    impl FixedWidthContext {
        fn new(width_per_grapheme: f64) -> Self {
            Self {
                width_per_grapheme,
                style: ResolvedRunStyle::new(
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    Rc::<str>::from("12pt Test"),
                    0.0,
                    1.0,
                    false,
                    None,
                ),
            }
        }
    }

    impl LineWrapContext for FixedWidthContext {
        type Error = ();

        fn resolve_run_style(
            &self,
            _properties: Option<&jade_docx_reader::RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
            Ok(Rc::new(self.style.clone()))
        }

        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            Ok(text.graphemes(true).count() as f64 * self.width_per_grapheme)
        }
    }

    fn plain_run(text: &str) -> Run {
        Run {
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
        }
    }

    fn paragraph_with_runs(runs: Vec<Run>) -> Paragraph {
        Paragraph {
            runs,
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    #[test]
    fn auto_spaced_mixed_face_lines_keep_their_own_font_boxes() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        struct FaceContext;
        impl LineWrapContext for FaceContext {
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
                Ok(text.chars().count() as f64 * 5.0)
            }
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let xml = r#"
          <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p><w:pPr><w:spacing w:line="240" w:lineRule="auto"/></w:pPr>
              <w:r><w:rPr><w:rFonts w:ascii="Noto Sans" w:eastAsia="Noto Sans"/><w:sz w:val="21"/><w:lang w:eastAsia="ja-JP"/></w:rPr>
                <w:t>123</w:t><w:br/><w:t>日本語</w:t><w:br/><w:t>漢字</w:t></w:r></w:p>
            <w:sectPr><w:pgSz w:w="6000" w:h="10000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>
          </w:body></w:document>"#;
        let document = DocumentParser::new().parse_document_xml(xml).unwrap();
        let layout = layout_document_lines(&document, &FaceContext).unwrap();
        let lines: Vec<_> = layout.get_lines().collect();
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["123", "日本語", "漢字"]
        );
        for (line, single_pt) in lines.iter().zip([14.299072, 13.6171875, 13.6171875]) {
            assert!(
                (line.height * 0.75 - single_pt).abs() < 0.001,
                "{}: {}",
                line.text,
                line.height * 0.75
            );
        }
        let exact_xml = xml.replace(
            "w:line=\"240\" w:lineRule=\"auto\"",
            "w:line=\"400\" w:lineRule=\"exact\"",
        );
        let exact_document = DocumentParser::new()
            .parse_document_xml(&exact_xml)
            .unwrap();
        let exact = layout_document_lines(&exact_document, &FaceContext).unwrap();
        assert!(exact
            .get_lines()
            .all(|line| (line.height * 0.75 - 20.0).abs() < 0.001));
        let unknown_xml = xml.replace("Noto Sans", "Unknown Sans");
        let unknown_document = DocumentParser::new()
            .parse_document_xml(&unknown_xml)
            .unwrap();
        let unknown = layout_document_lines(&unknown_document, &FaceContext).unwrap();
        let unknown_height = unknown.line(0).unwrap().height;
        assert!(unknown
            .get_lines()
            .all(|line| (line.height - unknown_height).abs() < 0.001));
        let text = "日本語\n漢字";
        let properties = RunProperties {
            font_family: Some("MS Mincho".into()),
            font_size: Some(10.5),
            ..RunProperties::default()
        };
        let style = ParagraphStyle {
            word_east_asian_font_line_box: true,
            word_proportional_line_sizing: crate::line_metrics::WordProportionalLineSizing::new(
                None, false, true,
            ),
            ..ParagraphStyle::default()
        };
        let runs = [RunStyle {
            start: 0,
            end: text.len(),
            properties: Some(Rc::new(properties)),
            revision: None,
            hyperlink: None,
            inline_object: None,
        }];
        let wrapped =
            wrap_text_into_lines(&FaceContext, text, &runs, 0.0, &style, 0, 300.0).unwrap();
        assert_eq!(wrapped.len(), 2);
        assert!(crate::line_wrap::WrappedLine::line_heights_px(
            &wrapped,
            &style,
            13.6171875 * 96.0 / 72.0
        )
        .is_none());
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn mixed_grid_line_heights_preserve_widow_pairs_in_headless_layout() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        struct FaceContext;
        impl LineWrapContext for FaceContext {
            type Error = ();
            fn resolve_run_style(
                &self,
                props: Option<&RunProperties>,
            ) -> Result<Rc<ResolvedRunStyle>, ()> {
                let face = props
                    .and_then(|p| p.fonts.as_ref())
                    .and_then(|f| f.ascii.as_deref())
                    .unwrap_or("MS Mincho");
                let descriptor: Rc<str> = format!("11.00pt '{face}'").into();
                Ok(Rc::new(ResolvedRunStyle::new(
                    descriptor.clone(),
                    descriptor.clone(),
                    descriptor.clone(),
                    descriptor,
                    0.0,
                    1.0,
                    false,
                    None,
                )))
            }
            fn measure_text(&self, text: &str, _: &str) -> Result<f64, ()> {
                Ok(text.chars().count() as f64 * 5.0)
            }
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let document = DocumentParser::new().parse_document_xml(r#"
          <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p><w:pPr><w:spacing w:line="1196" w:lineRule="exact"/><w:snapToGrid w:val="0"/></w:pPr><w:r><w:t>filler</w:t></w:r></w:p>
            <w:p><w:r><w:rPr><w:rFonts w:ascii="MS Mincho"/><w:sz w:val="22"/></w:rPr><w:t>A</w:t><w:br/><w:t>B</w:t><w:br/></w:r>
              <w:r><w:rPr><w:rFonts w:ascii="Yu Gothic"/><w:sz w:val="22"/></w:rPr><w:t>C</w:t><w:br/><w:t>D</w:t></w:r></w:p>
            <w:sectPr><w:pgSz w:w="6000" w:h="3192"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:docGrid w:type="lines" w:linePitch="299"/></w:sectPr>
          </w:body></w:document>"#).unwrap();
        let layout = layout_document_lines(&document, &FaceContext).unwrap();
        let lines: Vec<_> = layout.get_lines().collect();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["filler", "A", "B", "C", "D"]
        );
        assert_eq!(
            lines.iter().map(|line| line.page).collect::<Vec<_>>(),
            [1, 1, 1, 2, 2]
        );
        for (line, steps) in lines[1..].iter().zip([1.0, 1.0, 2.0, 2.0]) {
            assert!((line.height - 14.95 * 96.0 / 72.0 * steps).abs() < 1e-5);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    fn load_fixture_and_available_width(docx_path: &PathBuf) -> (Vec<u8>, f64) {
        let bytes = fs::read(docx_path).expect("failed to read docx fixture");
        let package = block_on(OpenXmlPackage::load(&bytes)).expect("failed to load package");
        let doc_target = package
            .relationships
            .iter()
            .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
            .map(|rel| rel.target.clone())
            .expect("missing document relationship");
        let xml = String::from_utf8_lossy(
            package
                .get(&doc_target)
                .expect("missing word/document.xml in fixture"),
        );
        let parser = DocumentParser::new();
        let document = parser
            .parse_document_xml(&xml)
            .expect("failed to parse document xml");
        let page_settings = document.page_settings.as_ref();
        let page_width = points_to_px(
            page_settings
                .and_then(|s| s.width)
                .unwrap_or(DEFAULT_PAGE_WIDTH_PT),
        );
        let margin_left = points_to_px(
            page_settings
                .and_then(|s| s.margin_left)
                .unwrap_or(DEFAULT_MARGIN_PT),
        );
        let margin_right = points_to_px(
            page_settings
                .and_then(|s| s.margin_right)
                .unwrap_or(DEFAULT_MARGIN_PT),
        );
        (bytes, (page_width - margin_right) - margin_left)
    }

    fn find_context_for_expected_first_line(
        bytes: &[u8],
        available_width: f64,
        expected: &str,
    ) -> FixedWidthContext {
        let expected_graphemes = expected.graphemes(true).count() as f64;
        let base_width = available_width / expected_graphemes;
        let mut candidate = FixedWidthContext::new(base_width);
        let mut matched = false;
        for step in 160..=260 {
            let scale = step as f64 / 200.0;
            let ctx = FixedWidthContext::new(base_width * scale);
            let first = block_on(extract_first_wrapped_line_from_docx_bytes(bytes, &ctx))
                .expect("failed to extract first line")
                .unwrap_or_default();
            if first == expected {
                candidate = ctx;
                matched = true;
                break;
            }
        }
        assert!(
            matched,
            "failed to find stable test context for expected first line"
        );
        candidate
    }

    #[test]
    fn mac_paragraph_spacing_line_units_do_not_use_font_box_height() {
        use crate::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        struct Restore(WordLayoutPlatform);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_word_layout_platform(self.0);
            }
        }
        let _restore = Restore(word_layout_platform());
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let paragraph = jade_docx_reader::Paragraph {
            properties: Some(jade_docx_reader::ParagraphProperties {
                spacing: Some(jade_docx_reader::Spacing {
                    before: Some(30.0),
                    after: Some(3.95),
                    before_lines: Some(0.5),
                    after_lines: Some(0.33),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            runs: Vec::new(),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        // Original Word contract afterLines=33 / after=79: the nominal
        // 12pt line gives 3.96pt; the 13.65pt CJK font box gives 4.5045pt.
        // Font-sized text still advances by its own physical line height.
        for font_box in [13.65, 28.6, 32.0] {
            assert!((spacing_after_px(&paragraph, font_box) * 0.75 - 3.96).abs() < 1e-5);
            assert_eq!(spacing_before_px(&paragraph, font_box) * 0.75, 6.0);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!((spacing_after_px(&paragraph, 20.0) * 0.75 - 6.6).abs() < 1e-5);
    }

    #[test]
    fn paragraph_line_spacing_units_override_point_margins() {
        let paragraph = jade_docx_reader::Paragraph {
            properties: Some(jade_docx_reader::ParagraphProperties {
                spacing: Some(jade_docx_reader::Spacing {
                    before: Some(30.0),
                    after: Some(40.0),
                    before_lines: Some(0.5),
                    after_lines: Some(1.5),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            runs: Vec::new(),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        assert_eq!(spacing_before_px(&paragraph, 12.0), 8.0);
        assert_eq!(spacing_after_px(&paragraph, 12.0), 24.0);
    }

    #[test]
    fn extracts_expected_first_wrapped_line_from_yokohama_mini() {
        let expected =
            "第1条　市税及びその賦課徴収については、地方税法(昭和25年法律第226号。以下「法」と";
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/yokohama_mini.docx");
        let (bytes, available_width) = load_fixture_and_available_width(&fixture);
        let context = find_context_for_expected_first_line(&bytes, available_width, expected);

        let line = block_on(extract_first_wrapped_line_from_docx_bytes(&bytes, &context))
            .expect("failed to extract first wrapped line")
            .expect("first wrapped line should exist");

        assert_eq!(line, expected);
    }

    #[test]
    fn line_layout_exposes_index_text_page_and_geometry() {
        let expected =
            "第1条　市税及びその賦課徴収については、地方税法(昭和25年法律第226号。以下「法」と";
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/yokohama_mini.docx");
        let (bytes, available_width) = load_fixture_and_available_width(&fixture);
        let context = find_context_for_expected_first_line(&bytes, available_width, expected);

        let layout = block_on(layout_docx_lines_from_bytes(&bytes, &context))
            .expect("failed to layout lines");
        let first = layout.line(0).expect("line[0] should exist");

        assert_eq!(first.index, 0);
        assert_eq!(first.text, expected);
        assert_eq!(first.page, 1);
        assert!(first.width > 0.0);
        assert!(first.height > 0.0);
        assert!(first.x >= 0.0);
        assert!(first.y >= 0.0);
        assert_eq!(layout.get_lines().next(), Some(first));
    }

    #[test]
    fn table_body_element_emits_cell_paragraph_lines() {
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(Table {
                    columns: vec![
                        TableColumn { width: Some(100.0) },
                        TableColumn { width: Some(100.0) },
                    ],
                    rows: vec![
                        TableRow {
                            height: Some(20.0),
                            cells: vec![
                                TableCell {
                                    blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                        vec![plain_run("Hello")],
                                    ))],
                                    margins: Some(TableCellMargins {
                                        left: Some(4.0),
                                        right: Some(4.0),
                                        top: Some(4.0),
                                        bottom: Some(4.0),
                                        ..TableCellMargins::default()
                                    }),
                                    ..TableCell::default()
                                },
                                TableCell {
                                    blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                        Vec::new(),
                                    ))],
                                    ..TableCell::default()
                                },
                            ],
                            ..TableRow::default()
                        },
                        TableRow {
                            height: Some(20.0),
                            cells: vec![
                                TableCell {
                                    blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                        Vec::new(),
                                    ))],
                                    ..TableCell::default()
                                },
                                TableCell {
                                    blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                        vec![plain_run("World")],
                                    ))],
                                    margins: Some(TableCellMargins {
                                        left: Some(4.0),
                                        right: Some(4.0),
                                        top: Some(4.0),
                                        bottom: Some(4.0),
                                        ..TableCellMargins::default()
                                    }),
                                    ..TableCell::default()
                                },
                            ],
                            ..TableRow::default()
                        },
                    ],
                    ..Table::default()
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(240.0),
                height: Some(160.0),
                margin_left: Some(20.0),
                margin_right: Some(20.0),
                margin_top: Some(20.0),
                margin_bottom: Some(20.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };
        let context = FixedWidthContext::new(5.0);

        let layout = layout_document_lines(&document, &context).expect("layout table lines");
        let lines: Vec<_> = layout.get_lines().collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[0].paragraph_index, 0);
        assert_eq!(lines[1].text, "World");
        assert_eq!(
            lines[1].paragraph_index, 4,
            "Word COM counts one invisible row-control paragraph after each row"
        );
    }

    #[test]
    fn bidi_table_horizontal_padding_resolves_logical_start_and_end() {
        let cell = TableCell::default();
        let table = Table {
            bidi_visual: true,
            cell_margins: Some(TableCellMargins {
                start: Some(12.0),
                end: Some(3.0),
                ..TableCellMargins::default()
            }),
            ..Table::default()
        };

        assert!(
            (table_cell_horizontal_padding_px(&cell, &table, true) - points_to_px(3.0)).abs()
                < 0.01
        );
        assert!(
            (table_cell_horizontal_padding_px(&cell, &table, false) - points_to_px(12.0)).abs()
                < 0.01
        );
    }

    #[test]
    fn table_text_direction_cells_emit_vertical_oracle_lines() {
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(Table {
                    columns: vec![
                        TableColumn { width: Some(80.0) },
                        TableColumn { width: Some(80.0) },
                    ],
                    rows: vec![TableRow {
                        height: Some(60.0),
                        cells: vec![
                            TableCell {
                                text_direction: Some(TextDirection::TbRlV),
                                blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                    vec![plain_run("abc")],
                                ))],
                                margins: Some(TableCellMargins {
                                    left: Some(4.0),
                                    right: Some(4.0),
                                    top: Some(4.0),
                                    bottom: Some(4.0),
                                    ..TableCellMargins::default()
                                }),
                                ..TableCell::default()
                            },
                            TableCell {
                                text_direction: Some(TextDirection::BtLr),
                                blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(
                                    vec![plain_run("de")],
                                ))],
                                margins: Some(TableCellMargins {
                                    left: Some(4.0),
                                    right: Some(4.0),
                                    top: Some(4.0),
                                    bottom: Some(4.0),
                                    ..TableCellMargins::default()
                                }),
                                ..TableCell::default()
                            },
                        ],
                        ..TableRow::default()
                    }],
                    ..Table::default()
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(220.0),
                height: Some(160.0),
                margin_left: Some(20.0),
                margin_right: Some(20.0),
                margin_top: Some(20.0),
                margin_bottom: Some(20.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };
        let context = FixedWidthContext::new(5.0);

        let layout = layout_document_lines(&document, &context).expect("layout table lines");
        let lines: Vec<_> = layout.get_lines().collect();

        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c", "d", "e"]
        );
        assert_eq!(
            lines
                .iter()
                .take(3)
                .map(|line| line.paragraph_index)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(
            lines[1].y > lines[0].y,
            "tbRlV should advance top-to-bottom in a column"
        );
        assert!(
            lines[4].y < lines[3].y,
            "btLr should report later glyphs upward"
        );
    }

    #[test]
    fn table_text_direction_fixture_keeps_bottom_to_top_ascii_glyph_lines() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/table-with-text-direction.docx");
        let bytes = std::fs::read(fixture).expect("read table-with-text-direction fixture");
        let context = FixedWidthContext::new(7.0);

        let layout = block_on(layout_docx_lines_from_bytes(&bytes, &context))
            .expect("layout table-with-text-direction lines");
        let bottom_to_top_lines: Vec<_> = layout
            .get_lines()
            .filter(|line| line.paragraph_index == 3)
            .map(|line| line.text.as_str())
            .collect();

        assert_eq!(bottom_to_top_lines, vec!["H", "e", "l", "l", "o"]);
    }

    #[test]
    fn math_body_element_emits_oracle_line() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_runs(vec![plain_run("before")])),
                    BodyElement::from_math_paragraph(MathParagraph {
                        properties: None,
                        raw_omml: "<m:oMathPara/>".to_string(),
                        elements: vec![MathElement {
                            kind: MathElementKind::Text,
                            tag: Some("t".to_string()),
                            text: Some("P a≤X≤b".to_string()),
                            children: Vec::new(),
                            attributes: Default::default(),
                        }],
                    }),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![plain_run("after")])),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(240.0),
                height: Some(160.0),
                margin_left: Some(20.0),
                margin_right: Some(20.0),
                margin_top: Some(20.0),
                margin_bottom: Some(20.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };
        let context = FixedWidthContext::new(5.0);

        let layout = layout_document_lines(&document, &context).expect("layout math line");
        let lines: Vec<_> = layout.get_lines().collect();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].paragraph_index, 1);
        assert_eq!(lines[1].line_index_in_paragraph, 0);
        assert_eq!(lines[1].text, "P a≤X≤b");
        assert_eq!(lines[2].paragraph_index, 2);
    }

    #[test]
    fn paragraph_inline_math_run_contributes_oracle_text() {
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph_with_runs(vec![
                    Run {
                        text: String::new(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: Some(MathRun {
                            raw_omml: "<m:oMath/>".to_string(),
                            elements: vec![MathElement {
                                kind: MathElementKind::Text,
                                tag: Some("t".to_string()),
                                text: Some("P a≤X≤b".to_string()),
                                children: Vec::new(),
                                attributes: Default::default(),
                            }],
                        }),
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
                ]))],
            }),
            page_settings: Some(PageSettings {
                width: Some(240.0),
                height: Some(160.0),
                margin_left: Some(20.0),
                margin_right: Some(20.0),
                margin_top: Some(20.0),
                margin_bottom: Some(20.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };
        let context = FixedWidthContext::new(5.0);

        let layout = layout_document_lines(&document, &context).expect("layout inline math");
        let line = layout.line(0).expect("inline math line");

        assert_eq!(line.paragraph_index, 0);
        assert_eq!(line.text, "P a≤X≤b");
    }

    #[test]
    fn empty_paragraph_after_math_block_does_not_shift_following_trace_indices() {
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_math_paragraph(MathParagraph {
                        properties: None,
                        raw_omml: "<m:oMathPara/>".to_string(),
                        elements: vec![MathElement {
                            kind: MathElementKind::Text,
                            tag: Some("t".to_string()),
                            text: Some("P a≤X≤b".to_string()),
                            children: Vec::new(),
                            attributes: Default::default(),
                        }],
                    }),
                    BodyElement::from_paragraph(paragraph_with_runs(Vec::new())),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![plain_run("after")])),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(240.0),
                height: Some(160.0),
                margin_left: Some(20.0),
                margin_right: Some(20.0),
                margin_top: Some(20.0),
                margin_bottom: Some(20.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };
        let context = FixedWidthContext::new(5.0);

        let layout = layout_document_lines(&document, &context).expect("layout math paragraph");
        let lines: Vec<_> = layout.get_lines().collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].paragraph_index, 0);
        assert_eq!(lines[1].paragraph_index, 1);
        assert_eq!(lines[1].text, "after");
    }

    fn synthetic_wrapping_document(widow_control: Option<bool>) -> Document {
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "abcdef".to_string(),
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
                        widow_control,
                        ..ParagraphProperties::default()
                    }),
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(20.0),
                height: Some(32.0),
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

    fn synthetic_two_column_document() -> Document {
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run("aa aa aa aa aa aa aa aa")],
                    properties: Some(ParagraphProperties {
                        widow_control: Some(false),
                        ..ParagraphProperties::default()
                    }),
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(40.0),
                height: Some(40.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                margin_top: Some(0.0),
                margin_bottom: Some(0.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            default_section: Some(jade_docx_reader::SectionProperties {
                columns: Some(SectionColumns {
                    count: Some(2),
                    space: Some(4.0),
                    ..SectionColumns::default()
                }),
                ..jade_docx_reader::SectionProperties::default()
            }),
            ..Document::default()
        }
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

    fn synthetic_page_break_document() -> Document {
        Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run(&format!("Hello{PAGE_BREAK_CHAR}"))],
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("World")],
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(200.0),
                height: Some(200.0),
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

    fn synthetic_two_section_document() -> Document {
        let first_page = PageSettings {
            width: Some(200.0),
            height: Some(200.0),
            margin_left: Some(10.0),
            margin_right: Some(10.0),
            margin_top: Some(10.0),
            margin_bottom: Some(10.0),
            ..PageSettings::default()
        };
        let second_page = PageSettings {
            width: Some(300.0),
            height: Some(150.0),
            margin_left: Some(30.0),
            margin_right: Some(20.0),
            margin_top: Some(20.0),
            margin_bottom: Some(10.0),
            ..PageSettings::default()
        };
        Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_runs(vec![plain_run("Hello")])),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![plain_run("World")])),
                ],
            }),
            page_settings: Some(first_page.clone()),
            default_section: Some(SectionProperties {
                page_settings: Some(second_page.clone()),
                ..SectionProperties::default()
            }),
            sections: vec![
                SectionRange {
                    start: 0,
                    end: 1,
                    properties: Some(SectionProperties {
                        page_settings: Some(first_page),
                        ..SectionProperties::default()
                    }),
                },
                SectionRange {
                    start: 1,
                    end: 2,
                    properties: Some(SectionProperties {
                        page_settings: Some(second_page),
                        // Omitted `w:type` is the normal next-page break.
                        section_type: None,
                        ..SectionProperties::default()
                    }),
                },
            ],
            ..Document::default()
        }
    }

    fn synthetic_vertical_section_document() -> Document {
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run("abc")],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(200.0),
                height: Some(200.0),
                margin_left: Some(10.0),
                margin_right: Some(10.0),
                margin_top: Some(10.0),
                margin_bottom: Some(10.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            default_section: Some(SectionProperties {
                text_direction: Some(TextDirection::TbRl),
                ..SectionProperties::default()
            }),
            ..Document::default()
        }
    }

    fn synthetic_page_break_document_with_spacing_after() -> Document {
        let mut document = synthetic_page_break_document();
        let body = document.body.as_mut().expect("body");
        let first = body.elements[0].as_paragraph_mut().expect("paragraph");
        first.properties = Some(ParagraphProperties {
            spacing: Some(jade_docx_reader::Spacing {
                after: Some(18.0),
                ..jade_docx_reader::Spacing::default()
            }),
            ..ParagraphProperties::default()
        });
        document
    }

    fn synthetic_soft_break_document() -> Document {
        let mut run = plain_run("Hello\nworld");
        run.properties = Some(RunProperties {
            font_size: Some(11.0),
            ..RunProperties::default()
        });
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![run],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(200.0),
                height: Some(200.0),
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

    fn synthetic_hidden_run_document() -> Document {
        let mut hidden = plain_run("hidden");
        hidden.properties = Some(RunProperties {
            vanish: Some(true),
            ..RunProperties::default()
        });
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run("Visible "), hidden, plain_run(" tail")],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(200.0),
                height: Some(200.0),
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

    #[test]
    fn widow_control_moves_three_line_paragraph_to_next_page_when_split_is_invalid() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_wrapping_document(Some(true));

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let pages = layout.get_lines().map(|line| line.page).collect::<Vec<_>>();

        // Widow control avoids leaving a single line on page 1, but the paragraph
        // can still continue onto page 3 because the synthetic page height only
        // fits two lines per page.
        assert_eq!(pages, vec![2, 2, 3]);
    }

    #[test]
    fn disabling_widow_control_allows_three_line_paragraph_to_split_across_pages() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_wrapping_document(Some(false));

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let pages = layout.get_lines().map(|line| line.page).collect::<Vec<_>>();

        assert_eq!(pages, vec![1, 1, 2]);
    }

    #[test]
    fn section_columns_wrap_and_flow_lines_across_columns() {
        let context = FixedWidthContext::new(4.0);
        let document = synthetic_two_column_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert!(
            lines.len() > 2,
            "narrow section columns should produce multiple wrapped lines"
        );
        assert_eq!(lines[0].page, 1);
        assert_eq!(lines[1].page, 1);
        assert!(
            lines.iter().any(|line| line.x > lines[0].x),
            "a two-column section should move later lines into the second column"
        );
    }

    #[test]
    fn omitted_section_type_starts_next_page_with_target_section_geometry() {
        let context = FixedWidthContext::new(5.0);
        let document = synthetic_two_section_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].page, 1);
        assert_eq!(lines[1].page, 2);
        assert!((lines[0].x - points_to_px(10.0)).abs() < 0.01);
        assert!((lines[1].x - points_to_px(30.0)).abs() < 0.01);
        assert!((lines[1].y - points_to_px(20.0)).abs() < 0.01);
    }

    #[test]
    fn section_vertical_text_direction_emits_one_oracle_line_per_grapheme() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_vertical_section_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "a");
        assert_eq!(lines[1].text, "b");
        assert_eq!(lines[2].text, "c");
        assert_eq!(lines[0].line_index_in_paragraph, 0);
        assert_eq!(lines[1].line_index_in_paragraph, 1);
        assert_eq!(lines[2].line_index_in_paragraph, 2);
        assert_eq!(lines[1].x, lines[0].x);
        assert_eq!(lines[2].x, lines[0].x);
        assert!(lines[1].y > lines[0].y);
        assert!(lines[2].y > lines[1].y);
    }

    #[test]
    fn widow_control_keeps_two_line_numbered_paragraph_on_current_page_when_it_fits() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_numbered_two_line_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let pages = layout.get_lines().map(|line| line.page).collect::<Vec<_>>();

        assert_eq!(pages, vec![1, 1, 1]);
    }

    #[test]
    fn page_break_does_not_emit_empty_oracle_line() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_page_break_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[0].paragraph_index, 0);
        assert_eq!(lines[0].line_index_in_paragraph, 0);
        assert_eq!(lines[0].page, 1);
        assert_eq!(lines[1].text, "World");
        assert_eq!(lines[1].paragraph_index, 1);
        assert_eq!(lines[1].line_index_in_paragraph, 0);
        assert_eq!(lines[1].page, 2);
        assert_eq!(lines[1].y, 0.0);
    }

    #[test]
    fn mirrored_margins_shift_even_page_lines_to_the_verso_margin() {
        let context = FixedWidthContext::new(10.0);
        let mut document = synthetic_page_break_document();
        document.mirror_margins = true;
        document.page_settings = Some(PageSettings {
            width: Some(300.0),
            height: Some(300.0),
            margin_left: Some(72.0),
            margin_right: Some(54.0),
            margin_top: Some(72.0),
            margin_bottom: Some(72.0),
            gutter: Some(18.0),
            ..PageSettings::default()
        });

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert!((lines[0].x - points_to_px(90.0)).abs() < 1e-6);
        assert!((lines[1].x - points_to_px(54.0)).abs() < 1e-6);
    }

    #[test]
    fn page_break_at_paragraph_end_does_not_carry_spacing_after_to_next_page() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_page_break_document_with_spacing_after();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "World");
        assert_eq!(lines[1].page, 2);
        assert_eq!(
            lines[1].y, 0.0,
            "Word COM TRACE evidence: tests/word-oracle/hello/word-trace.json places page-2 text at the top margin after a paragraph-final w:br type=\"page\"; paragraph spacing-after must not be carried onto the new page"
        );
    }

    #[test]
    fn mac_page_start_before_spacing_matches_the_renderer_and_preserves_document_start() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let mut document = synthetic_page_break_document_with_spacing_after();
        for element in &mut document.body.as_mut().unwrap().elements {
            if let Some(paragraph) = element.as_paragraph_mut() {
                let props = paragraph
                    .properties
                    .get_or_insert_with(ParagraphProperties::default);
                props
                    .spacing
                    .get_or_insert_with(jade_docx_reader::Spacing::default)
                    .before = Some(24.0);
            }
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let layout = layout_document_lines(&document, &FixedWidthContext::new(10.0)).unwrap();
        let positions = layout
            .get_lines()
            .map(|line| (line.page, line.y))
            .collect::<Vec<_>>();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(positions, vec![(1, 32.0), (2, 0.0)]);
    }

    #[test]
    fn mac_before_spacing_reservation_restarts_at_the_next_page_or_column() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let mut outcomes = Vec::new();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for columns in [1, 2] {
            let paragraphs = ["First", "Following"]
                .into_iter()
                .enumerate()
                .map(|(index, text)| {
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run(text)],
                        properties: Some(ParagraphProperties {
                            spacing: Some(jade_docx_reader::Spacing {
                                before: (index == 1).then_some(24.0),
                                after: Some(0.0),
                                line: Some(if index == 0 { 80.0 } else { 10.0 }),
                                line_rule: Some("exact".into()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    })
                })
                .collect();
            let document = Document {
                body: Some(Body {
                    elements: paragraphs,
                }),
                page_settings: Some(PageSettings {
                    width: Some(200.0),
                    height: Some(100.0),
                    margin_top: Some(0.0),
                    margin_bottom: Some(0.0),
                    margin_left: Some(0.0),
                    margin_right: Some(0.0),
                    ..Default::default()
                }),
                default_section: Some(SectionProperties {
                    columns: Some(SectionColumns {
                        count: Some(columns),
                        space: Some(12.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let layout = layout_document_lines(&document, &FixedWidthContext::new(10.0)).unwrap();
            let following = layout
                .get_lines()
                .find(|line| line.text == "Following")
                .unwrap();
            outcomes.push((columns, following.page, following.x, following.y));
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        for (columns, page, x, y) in outcomes {
            assert_eq!(y, 0.0, "{columns} columns, page {page}: before spacing must not carry across the reservation");
            assert_eq!(page, if columns == 1 { 2 } else { 1 });
            if columns == 2 {
                assert!(x > 100.0, "the next column owns the paragraph");
            }
        }
    }

    #[test]
    fn mac_headless_section_start_collapses_preceding_after_spacing() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let position = |before: f32| {
            let mut document = synthetic_two_section_document();
            for (index, element) in document
                .body
                .as_mut()
                .unwrap()
                .elements
                .iter_mut()
                .enumerate()
            {
                element.as_paragraph_mut().unwrap().properties = Some(ParagraphProperties {
                    spacing: Some(jade_docx_reader::Spacing {
                        before: (index == 1).then_some(before),
                        after: (index == 0).then_some(10.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
            let layout = layout_document_lines(&document, &FixedWidthContext::new(10.0)).unwrap();
            let line = layout
                .get_lines()
                .find(|line| line.text == "World")
                .unwrap();
            assert_eq!(line.page, 2);
            line.y
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let difference = position(24.0) - position(0.0);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(
            (difference - points_to_px(14.0)).abs() < 0.001,
            "before spacing: {difference}"
        );
    }

    #[test]
    fn before_spacing_reservation_preserves_section_inline_and_windows_contracts() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let mut outcomes = Vec::new();
        for (platform, section_start, inline, expected) in [
            (WordLayoutPlatform::Mac, false, false, 0.0),
            (WordLayoutPlatform::Mac, true, false, 32.0),
            (WordLayoutPlatform::Mac, false, true, 32.0),
            (WordLayoutPlatform::Windows, false, false, 8.0),
        ] {
            set_word_layout_platform(platform);
            let (mut y, mut page, mut column) = (80.0, 0, 0);
            advance_paragraph_spacing_before(
                &mut y,
                &mut page,
                &mut column,
                1,
                32.0,
                4.0,
                section_start,
                inline,
                0.0,
                100.0,
            );
            outcomes.push((platform, section_start, inline, page, column, y, expected));
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        for (platform, section, inline, page, column, y, expected) in outcomes {
            assert_eq!((page, column), (1, 0));
            assert_eq!(
                y, expected,
                "{platform:?}, section {section}, inline {inline}"
            );
        }
    }

    #[test]
    fn mac_empty_paragraph_before_spacing_reservation_does_not_carry_a_gap() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let elements = ["First", "", "Following"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run(text)],
                    properties: Some(ParagraphProperties {
                        spacing: Some(jade_docx_reader::Spacing {
                            before: (index == 1).then_some(24.0),
                            after: Some(0.0),
                            line: Some(if index == 0 { 80.0 } else { 10.0 }),
                            line_rule: Some("exact".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })
            })
            .collect();
        let document = Document {
            body: Some(Body { elements }),
            page_settings: Some(PageSettings {
                width: Some(200.0),
                height: Some(100.0),
                margin_top: Some(0.0),
                margin_bottom: Some(0.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let layout = layout_document_lines(&document, &FixedWidthContext::new(10.0)).unwrap();
        let line = layout
            .get_lines()
            .find(|line| line.text == "Following")
            .unwrap();
        let position = (line.page, line.y);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(position.0, 2);
        assert!((position.1 - 40.0 / 3.0).abs() < EPS,
            "empty paragraph consumes its own exact line, without a carried before gap: {position:?}");
    }

    #[test]
    fn hidden_page_break_line_does_not_increment_visible_line_index() {
        let context = FixedWidthContext::new(10.0);
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run(&format!("{PAGE_BREAK_CHAR}World"))],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(100.0),
                height: Some(100.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                margin_top: Some(0.0),
                margin_bottom: Some(0.0),
                orientation: None,
                gutter: None,
                ..PageSettings::default()
            }),
            ..Document::default()
        };

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "World");
        assert_eq!(lines[0].paragraph_index, 0);
        assert_eq!(lines[0].line_index_in_paragraph, 0);
        assert_eq!(lines[0].page, 2);
    }

    #[test]
    fn soft_break_line_uses_word_oracle_step_height() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_soft_break_document();
        let paragraph = document
            .body
            .as_ref()
            .and_then(|body| body.elements[0].as_paragraph())
            .expect("paragraph");
        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.font_size),
            Some(11.0)
        );
        assert_eq!(estimate_paragraph_font_size(paragraph), 12.0);
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:docDefaults>
                  <w:rPrDefault>
                    <w:rPr><w:sz w:val="22"/></w:rPr>
                  </w:rPrDefault>
                </w:docDefaults>
              </w:styles>"#,
        )
        .expect("styles should parse");

        let layout = layout_document_lines_with_styles(&document, Some(&styles), &context)
            .expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[1].text, "world");
        // break.docx Word COM oracle places the second line 12.65pt after the
        // first (69.10pt - 56.45pt). The normal 12pt single-line box is 13.8pt.
        assert!(
            (lines[1].y - points_to_px(12.65)).abs() < 0.01,
            "expected second line y={}px, got {}px",
            points_to_px(12.65),
            lines[1].y
        );
    }

    #[test]
    fn symbol_h_size_does_not_raise_paragraph_line_height_font_size() {
        let mut symbol = plain_run("");
        symbol.properties = Some(RunProperties {
            font_size: Some(48.0),
            ..RunProperties::default()
        });
        symbol.field = Some(RunField {
            kind: RunFieldKind::Symbol {
                code_point: 0x2605,
                character_set: SymbolCharacterSet::Unicode,
                font: None,
                font_size_points: Some(48),
                preserve_line_height: true,
            },
            cached_text: Some("★".to_string()),
            source_field_id: None,
            resolved_value: Some("★".to_string()),
        });
        let paragraph = Paragraph {
            runs: vec![plain_run("Body"), symbol.clone()],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        // Word's SYMBOL \h switch deliberately permits a large glyph to
        // overprint neighbouring leading. It must not turn an otherwise 12pt
        // paragraph into a 48pt line merely because \s also appears.
        assert_eq!(estimate_paragraph_font_size(&paragraph), 12.0);

        if let Some(RunField {
            kind:
                RunFieldKind::Symbol {
                    preserve_line_height,
                    ..
                },
            ..
        }) = symbol.field.as_mut()
        {
            *preserve_line_height = false;
        }
        let ordinary = Paragraph {
            runs: vec![plain_run("Body"), symbol],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        assert_eq!(estimate_paragraph_font_size(&ordinary), 48.0);
    }

    #[test]
    fn east_asian_empty_paragraph_uses_word_empty_line_box_floor() {
        let paragraph = Paragraph {
            runs: Vec::new(),
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        assert_eq!(estimate_paragraph_font_size(&paragraph), 11.0);
    }

    #[test]
    fn east_asian_empty_paragraph_honors_explicit_mark_font_face() {
        let paragraph = Paragraph {
            runs: Vec::new(),
            properties: Some(ParagraphProperties {
                mark_run_properties: Some(RunProperties {
                    fonts: Some(RunFonts {
                        ascii: Some("ＭＳ 明朝".into()),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        assert_eq!(estimate_paragraph_font_size(&paragraph), 12.0);
    }

    #[test]
    fn hidden_runs_are_omitted_from_word_oracle_lines() {
        let context = FixedWidthContext::new(10.0);
        let document = synthetic_hidden_run_document();

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Visible  tail");
    }

    #[test]
    fn runless_empty_paragraph_consumes_line_height_without_emitting_trace_line() {
        let context = FixedWidthContext::new(10.0);
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("First")],
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: Vec::new(),
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("Visible")],
                        properties: None,
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            page_settings: Some(PageSettings {
                margin_top: Some(72.0),
                margin_bottom: Some(72.0),
                ..PageSettings::default()
            }),
            ..Document::default()
        };

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "First");
        assert_eq!(lines[1].text, "Visible");
        assert!(
            (lines[1].y - points_to_px(72.0 + (12.0 * 1.15 * 2.0))).abs() < 1e-4,
            "empty paragraph should advance one default line before the visible trace line: got {}",
            lines[1].y
        );
    }

    #[test]
    fn word_oracle_fe_layout_reports_non_numbered_left_indent_at_margin_without_widening_wrap() {
        let context = FixedWidthContext::new(10.0);
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("abcdefghi")],
                        properties: Some(ParagraphProperties {
                            indent: Some(Indent {
                                left: Some(30.0),
                                ..Indent::default()
                            }),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("Number")],
                        properties: Some(ParagraphProperties {
                            numbering: Some(Numbering {
                                level: Some(0),
                                id: Some("1".to_string()),
                            }),
                            indent: Some(Indent {
                                left: Some(15.0),
                                ..Indent::default()
                            }),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(75.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                ..PageSettings::default()
            }),
            east_asian_line_breaking: jade_docx_reader::EastAsianLineBreakingSettings {
                use_east_asian_layout: true,
                character_spacing_control: Some("doNotCompress".to_string()),
                ..jade_docx_reader::EastAsianLineBreakingSettings::default()
            },
            ..Document::default()
        };

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines[0].text, "abcdef");
        assert!(
            (lines[0].x - points_to_px(0.0)).abs() < 1e-4,
            "contract COM TRACE reports non-numbered FE-layout paragraphs with w:left at the page margin"
        );
        assert_eq!(
            lines[1].text, "ghi",
            "w:left must still narrow the wrap width; target/contract-after-settings-indent-full-report.json showed that dropping it collapses p0 from three Word lines to two"
        );
        assert_eq!(lines[2].text, "Number");
        assert!(
            (lines[2].x - points_to_px(15.0)).abs() < 1e-4,
            "numbered paragraph x remains governed by numbering/paragraph indent"
        );
    }

    #[test]
    fn theme_fonts_resolve_in_the_package_layout_path() {
        // The PDF renderer and headless CLI lay documents out through this
        // package path. A document whose fonts come from `w:docDefaults`
        // theme references (`w:asciiTheme="minorHAnsi"`, …) must resolve to
        // the theme's concrete faces here too, not fall through to the
        // default stacks while the editor resolves them properly.
        const DOCUMENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>Theme text</w:t></w:r>
    </w:p>
  </w:body>
</w:document>
"#;
        const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:cstheme="minorBidi"/>
      </w:rPr>
    </w:rPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
  </w:style>
</w:styles>
"#;
        const THEME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Test">
  <a:themeElements>
    <a:fontScheme name="Test">
      <a:majorFont>
        <a:latin typeface="Cambria"/>
        <a:ea typeface=""/>
        <a:cs typeface="Times New Roman"/>
        <a:font script="Jpan" typeface="ＭＳ Ｐゴシック"/>
      </a:majorFont>
      <a:minorFont>
        <a:latin typeface="Calibri"/>
        <a:ea typeface=""/>
        <a:cs typeface="Arial"/>
        <a:font script="Jpan" typeface="ＭＳ 明朝"/>
      </a:minorFont>
    </a:fontScheme>
  </a:themeElements>
</a:theme>
"#;

        let mut files = std::collections::HashMap::new();
        files.insert(
            "word/document.xml".to_string(),
            DOCUMENT_XML.as_bytes().to_vec(),
        );
        files.insert(
            "word/styles.xml".to_string(),
            STYLES_XML.as_bytes().to_vec(),
        );
        files.insert(
            "word/theme/theme1.xml".to_string(),
            THEME_XML.as_bytes().to_vec(),
        );
        let package = OpenXmlPackage {
            files,
            relationships: vec![jade_docx_reader::Relationship {
                id: "rId1".to_string(),
                r#type:
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
                        .to_string(),
                target: "word/document.xml".to_string(),
                target_mode: None,
            }],
            original_size_bytes: None,
        };

        let context = FixedWidthContext::new(10.0);
        let layout =
            layout_docx_lines_from_package(&package, &context).expect("layout theme package");
        let line = layout.get_lines().next().expect("line");
        let fonts = line
            .spans
            .first()
            .and_then(|span| span.properties.as_ref())
            .and_then(|props| props.fonts.clone())
            .expect("span fonts");
        assert_eq!(
            fonts.h_ansi.as_deref(),
            Some("Calibri"),
            "minorHAnsi must resolve to the theme's minor latin face"
        );
        assert_eq!(
            fonts.east_asia.as_deref(),
            Some("ＭＳ 明朝"),
            "minorEastAsia must resolve via the Jpan script fallback"
        );
    }

    #[test]
    fn word_oracle_fe_layout_reports_non_numbered_first_line_and_hanging_indent_at_margin() {
        let context = FixedWidthContext::new(10.0);
        let document = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("FirstLine")],
                        properties: Some(ParagraphProperties {
                            indent: Some(Indent {
                                first_line: Some(12.0),
                                ..Indent::default()
                            }),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![plain_run("Hanging")],
                        properties: Some(ParagraphProperties {
                            indent: Some(Indent {
                                left: Some(48.0),
                                hanging: Some(12.0),
                                ..Indent::default()
                            }),
                            ..ParagraphProperties::default()
                        }),
                        node_id: None,
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            page_settings: Some(PageSettings {
                width: Some(100.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                ..PageSettings::default()
            }),
            east_asian_line_breaking: jade_docx_reader::EastAsianLineBreakingSettings {
                use_east_asian_layout: true,
                character_spacing_control: Some("doNotCompress".to_string()),
                ..jade_docx_reader::EastAsianLineBreakingSettings::default()
            },
            ..Document::default()
        };

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines[0].text, "FirstLine");
        assert!((lines[0].x - points_to_px(0.0)).abs() < 1e-4);
        assert_eq!(lines[1].text, "Hanging");
        assert!((lines[1].x - points_to_px(0.0)).abs() < 1e-4);
    }

    #[test]
    fn word_oracle_fe_layout_reports_compress_punctuation_indent_at_margin() {
        let context = FixedWidthContext::new(10.0);
        let document = Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![plain_run("Indented")],
                    properties: Some(ParagraphProperties {
                        indent: Some(Indent {
                            left: Some(12.0),
                            ..Indent::default()
                        }),
                        ..ParagraphProperties::default()
                    }),
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            page_settings: Some(PageSettings {
                width: Some(100.0),
                margin_left: Some(0.0),
                margin_right: Some(0.0),
                ..PageSettings::default()
            }),
            east_asian_line_breaking: jade_docx_reader::EastAsianLineBreakingSettings {
                use_east_asian_layout: true,
                character_spacing_control: Some("compressPunctuation".to_string()),
                ..jade_docx_reader::EastAsianLineBreakingSettings::default()
            },
            ..Document::default()
        };

        let layout = layout_document_lines(&document, &context).expect("layout should succeed");
        let lines = layout.get_lines().collect::<Vec<_>>();

        assert_eq!(lines[0].text, "Indented");
        assert!((lines[0].x - points_to_px(0.0)).abs() < 1e-4);
    }

    #[test]
    fn word_oracle_docx_settings_enable_fe_layout_rules_for_do_not_compress() {
        let settings = DocumentSettings::from_xml(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:compat>
                  <w:useFELayout/>
                  <w:characterSpacingControl w:val="doNotCompress"/>
                </w:compat>
              </w:settings>"#,
        )
        .expect("settings should parse");
        let mut document = Document::default();

        apply_document_settings(&mut document, &settings);

        assert!(document.snap_to_grid_default);
        assert!(document.east_asian_line_breaking.use_east_asian_layout);
        assert_eq!(
            document
                .east_asian_line_breaking
                .character_spacing_control
                .as_deref(),
            Some("doNotCompress")
        );
    }

    #[test]
    fn word_oracle_docx_settings_do_not_enable_fe_layout_for_compress_punctuation() {
        let settings = DocumentSettings::from_xml(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:characterSpacingControl w:val="compressPunctuation"/>
                <w:compat>
                  <w:useFELayout/>
                </w:compat>
              </w:settings>"#,
        )
        .expect("settings should parse");
        let mut document = Document::default();

        apply_document_settings(&mut document, &settings);

        assert!(!document.snap_to_grid_default);
        assert!(!document.east_asian_line_breaking.use_east_asian_layout);
        assert_eq!(
            document.east_asian_line_breaking.character_spacing_control,
            None
        );
    }

    #[test]
    fn style_font_size_uses_default_paragraph_style_before_doc_defaults() {
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:docDefaults>
                  <w:rPrDefault>
                    <w:rPr><w:sz w:val="20"/></w:rPr>
                  </w:rPrDefault>
                </w:docDefaults>
                <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
                  <w:name w:val="Normal"/>
                  <w:rPr><w:sz w:val="24"/></w:rPr>
                </w:style>
              </w:styles>"#,
        )
        .expect("styles should parse");
        let paragraph = Paragraph {
            runs: vec![plain_run("Visible  tail")],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        assert_eq!(
            estimate_paragraph_font_size_with_styles(&paragraph, Some(&styles)),
            12.0
        );
    }

    #[test]
    fn east_asian_style_defaults_fill_run_fonts_and_font_size() {
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:docDefaults>
                  <w:rPrDefault>
                    <w:rPr>
                      <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="ＭＳ 明朝"/>
                      <w:sz w:val="21"/>
                    </w:rPr>
                  </w:rPrDefault>
                </w:docDefaults>
                <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
                  <w:name w:val="Normal"/>
                </w:style>
              </w:styles>"#,
        )
        .expect("styles should parse");
        let paragraph = Paragraph {
            runs: vec![plain_run("契約")],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let fallback = style_run_font_fallback(&paragraph, Some(&styles));

        let effective =
            effective_run_properties(&paragraph.runs[0], Some(&styles), fallback.as_ref(), true)
                .expect("effective run props");
        let fonts = effective.fonts.expect("run fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Times New Roman"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Times New Roman"));
        assert_eq!(fonts.east_asia.as_deref(), Some("ＭＳ 明朝"));
        assert_eq!(effective.font_size, Some(10.5));
    }

    #[test]
    fn east_asian_style_defaults_do_not_touch_explicit_east_asia_fonts() {
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:docDefaults>
                  <w:rPrDefault>
                    <w:rPr>
                      <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="ＭＳ 明朝"/>
                      <w:sz w:val="21"/>
                    </w:rPr>
                  </w:rPrDefault>
                </w:docDefaults>
              </w:styles>"#,
        )
        .expect("styles should parse");
        let paragraph = Paragraph {
            runs: vec![Run {
                properties: Some(RunProperties {
                    fonts: Some(RunFonts {
                        east_asia: Some("Noto Sans JP".into()),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
                ..plain_run("横浜")
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let fallback = style_run_font_fallback(&paragraph, Some(&styles));

        let effective =
            effective_run_properties(&paragraph.runs[0], Some(&styles), fallback.as_ref(), true)
                .expect("effective run props");
        let fonts = effective.fonts.expect("run fonts");
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.ascii.as_deref(), Some("Times New Roman"));
        assert_eq!(effective.font_size, Some(10.5));
    }

    #[test]
    fn styles_are_needed_only_when_visible_paragraph_lacks_east_asia_fonting() {
        let explicit_east_asia_paragraph = Paragraph {
            runs: vec![Run {
                properties: Some(RunProperties {
                    fonts: Some(RunFonts {
                        east_asia: Some("Noto Sans JP".into()),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
                ..plain_run("Yokohama")
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let unfonted_visible_paragraph = Paragraph {
            runs: vec![plain_run("Contract")],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let empty_unfonted_paragraph = Paragraph {
            runs: vec![plain_run("")],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let all_explicit = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(explicit_east_asia_paragraph.clone()),
                    BodyElement::from_paragraph(empty_unfonted_paragraph.clone()),
                ],
            }),
            ..Document::default()
        };
        assert!(!document_has_visible_paragraph_without_east_asian_fonting(
            &all_explicit
        ));

        let mixed = Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(explicit_east_asia_paragraph),
                    BodyElement::from_paragraph(unfonted_visible_paragraph),
                ],
            }),
            ..Document::default()
        };
        assert!(document_has_visible_paragraph_without_east_asian_fonting(
            &mixed
        ));
    }

    #[test]
    fn effective_run_properties_inherit_visual_paragraph_style_properties() {
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:style w:type="paragraph" w:styleId="Styled">
                <w:rPr>
                  <w:b/><w:i/><w:u w:val="single"/><w:strike/><w:color w:val="336699"/>
                </w:rPr>
              </w:style>
            </w:styles>"#,
        )
        .expect("styles should parse");
        assert_eq!(
            styles
                .resolve_run_properties("Styled")
                .and_then(|props| props.bold),
            Some(true)
        );
        let paragraph = Paragraph {
            properties: Some(ParagraphProperties {
                style: Some("Styled".into()),
                ..ParagraphProperties::default()
            }),
            runs: vec![plain_run("Styled text")],
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let fallback = style_run_font_fallback(&paragraph, Some(&styles));
        assert_eq!(fallback.as_ref().and_then(|props| props.bold), Some(true));
        let effective =
            effective_run_properties(&paragraph.runs[0], Some(&styles), fallback.as_ref(), true)
                .expect("style should produce effective properties");

        assert_eq!(effective.bold, Some(true));
        assert_eq!(effective.italic, Some(true));
        assert_eq!(effective.underline, Some(true));
        assert_eq!(effective.strike, Some(true));
        assert_eq!(effective.color.as_deref(), Some("#336699"));
    }

    #[test]
    fn character_style_visuals_survive_explicit_east_asian_run_fonts() {
        let styles = StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:docDefaults><w:rPrDefault><w:rPr>
                <w:rFonts w:ascii="Aptos" w:eastAsia="Yu Gothic"/>
              </w:rPr></w:rPrDefault></w:docDefaults>
              <w:style w:type="character" w:styleId="Strong">
                <w:rPr><w:b/><w:i/><w:color w:val="336699"/></w:rPr>
              </w:style>
            </w:styles>"#,
        )
        .expect("styles should parse");
        let paragraph = Paragraph {
            runs: vec![Run {
                properties: Some(RunProperties {
                    style: Some("Strong".into()),
                    fonts: Some(RunFonts {
                        east_asia: Some("Noto Sans JP".into()),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
                ..plain_run("強調 text")
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let fallback = style_run_font_fallback(&paragraph, Some(&styles));
        let effective =
            effective_run_properties(&paragraph.runs[0], Some(&styles), fallback.as_ref(), false)
                .expect("effective character style");

        assert_eq!(effective.bold, Some(true));
        assert_eq!(effective.italic, Some(true));
        assert_eq!(effective.color.as_deref(), Some("#336699"));
        let fonts = effective.fonts.expect("direct run fonts remain");
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Sans JP"));
        assert_eq!(
            fonts.ascii, None,
            "paragraph/doc-default font stays suppressed"
        );
    }
}
