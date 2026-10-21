use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod run_format;
pub use run_format::{HeadlessEastAsianLayout, HeadlessRunFormat, HeadlessThemeColor};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeadlessEditRequest {
    #[serde(default)]
    pub commands: Vec<HeadlessCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeadlessQueryRequest {
    #[serde(default)]
    pub queries: Vec<HeadlessQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeadlessApplyAndQueryRequest {
    #[serde(default)]
    pub commands: Vec<HeadlessCommand>,
    #[serde(default)]
    pub queries: Vec<HeadlessQuery>,
    #[serde(default)]
    pub include_diff: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessQueryResponse {
    #[serde(default)]
    pub results: Vec<HeadlessQueryResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessRunStyle {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub baseline: Option<String>,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessRunSnapshot {
    pub text: String,
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_char_offset: usize,
    pub end_char_offset: usize,
    #[serde(default)]
    pub style: HeadlessRunStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessParagraphIndent {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub first_line: Option<f32>,
    pub hanging: Option<f32>,
}

/// Direction of a ribbon-style indent step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessIndentDirection {
    Increase,
    Decrease,
}

/// Which edge a ribbon-style indent step moves.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessIndentTarget {
    /// The logical start edge (Word's Ctrl+M; list level on list items).
    #[default]
    Start,
    /// Every line but the first (Word's Ctrl+T): the first line stays put and
    /// the difference becomes the hanging indent.
    Hanging,
}

/// Direction of a ribbon-style grow/shrink font step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessFontSizeDirection {
    Grow,
    Shrink,
}

/// How far a grow/shrink font step moves.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessFontSizeStep {
    /// Word's ribbon size list (Ctrl+Shift+> / <).
    #[default]
    Ladder,
    /// Exactly one point (Word's Ctrl+] / Ctrl+[).
    Point,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessParagraphSnapshot {
    pub paragraph: usize,
    pub text: String,
    #[serde(default)]
    pub numbering_text: Option<String>,
    pub alignment: Option<String>,
    #[serde(default)]
    pub indent: Option<HeadlessParagraphIndent>,
    /// The paragraph's `w:pStyle`, so a caller can see which style produced the
    /// formatting it is reading — and pass the same id back to
    /// `set_paragraph_style`.
    #[serde(default)]
    pub style_id: Option<String>,
    /// Line spacing and space before/after (`w:spacing`), so an adjustment made
    /// with `set_paragraph_line_spacing` can be read back and a document's
    /// current spacing can be inspected before changing it.
    #[serde(default)]
    pub spacing: Option<HeadlessParagraphSpacing>,
    #[serde(default)]
    pub runs: Vec<HeadlessRunSnapshot>,
}

/// One border edge as a command describes it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeadlessBorderEdge {
    /// `single`, `double`, `dashed`, `dotted`, `thick`, `none`, … — `none`
    /// removes the line.
    #[serde(default)]
    pub style: Option<String>,
    /// Thickness in points.
    #[serde(default)]
    pub width: Option<f32>,
    /// `#RRGGBB`, or `auto` for Word's automatic colour.
    #[serde(default)]
    pub color: Option<String>,
}

/// Editable subset of the package-wide DrawingML document theme.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeadlessDocumentTheme {
    pub name: String,
    pub major_font: String,
    pub minor_font: String,
    pub east_asia_font: String,
    pub dark2: String,
    pub light2: String,
    pub accent1: String,
    pub accent2: String,
    pub accent3: String,
    pub accent4: String,
    pub accent5: String,
    pub accent6: String,
}

/// `w:spacing`: the vertical rhythm of one paragraph, in points except `line`,
/// whose meaning depends on `line_rule`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessParagraphSpacing {
    /// Space above the paragraph, in points.
    pub before: Option<f32>,
    /// Space below the paragraph, in points.
    pub after: Option<f32>,
    /// Line spacing. With `line_rule` `auto` this is a multiple of single
    /// spacing (1.5 is one-and-a-half); with `exact` or `at_least` it is points.
    pub line: Option<f32>,
    /// `auto`, `exact` or `at_least` — how `line` is read.
    pub line_rule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessCharacterStyleSnapshot {
    pub paragraph: usize,
    pub offset: usize,
    pub char_offset: usize,
    pub run_start_offset: usize,
    pub run_end_offset: usize,
    pub run_start_char_offset: usize,
    pub run_end_char_offset: usize,
    pub run_text: String,
    #[serde(default)]
    pub style: HeadlessRunStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessParagraphText {
    pub paragraph: usize,
    pub text: String,
    #[serde(default)]
    pub numbering_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessSearchMatch {
    pub paragraph: usize,
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_char_offset: usize,
    pub end_char_offset: usize,
    pub matched_text: String,
    pub context_before: String,
    pub context_after: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessTableCellSelection {
    pub row: usize,
    pub column: usize,
    pub column_span: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessTableCellDeleteDirection {
    ShiftLeft,
    ShiftUp,
}

/// Where Word's セルの挿入 moves the existing cells.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessTableCellInsertDirection {
    /// The new cell takes the target's place and the rest of the row moves
    /// right, so the row ends one grid column further out.
    ShiftRight,
    /// The new cell takes the target's place and the cells below it in that
    /// column move down; a row is appended to hold the last one.
    ShiftDown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessTableFitMode {
    Contents,
    Window,
    Fixed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessTableTextWrapping {
    None,
    Around,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessTableColumnSnapshot {
    pub column: usize,
    pub width: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessTableCellSnapshot {
    pub row: usize,
    pub column: usize,
    pub span: Option<u32>,
    pub vertical_merge: Option<String>,
    pub text_direction: Option<String>,
    pub text: String,
    #[serde(default)]
    pub style: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessTableRowSnapshot {
    pub row: usize,
    pub is_header: bool,
    pub height: Option<f32>,
    pub height_rule: Option<String>,
    pub grid_before: Option<u32>,
    pub grid_after: Option<u32>,
    #[serde(default)]
    pub cells: Vec<HeadlessTableCellSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessTableSnapshot {
    pub table_order_index: usize,
    pub table_width: Option<String>,
    pub alignment: Option<String>,
    #[serde(default)]
    pub style: BTreeMap<String, String>,
    #[serde(default)]
    pub cell_style: BTreeMap<String, String>,
    #[serde(default)]
    pub columns: Vec<HeadlessTableColumnSnapshot>,
    #[serde(default)]
    pub rows: Vec<HeadlessTableRowSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessTableColumnDetailSnapshot {
    pub column: usize,
    pub width: Option<f32>,
    #[serde(default)]
    pub cells: Vec<HeadlessTableCellSnapshot>,
}

/// One style definition from `word/styles.xml`.
///
/// `set_paragraph_style` takes a `style_id`, so a caller that cannot list the
/// document's styles cannot use it: this snapshot is what makes the styling
/// commands addressable.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessStyleSnapshot {
    pub style_id: String,
    /// `w:name`, the label Word shows in its style gallery.
    pub name: Option<String>,
    pub based_on: Option<String>,
    /// Whether Word applies this style to paragraphs that name none.
    pub is_default: bool,
    /// `paragraph` or `character`.
    pub kind: String,
}

/// Page geometry of the document's default section, in points — the read side
/// of `set_page_size` / `set_page_orientation` / `set_page_margins` /
/// `set_page_columns` / `set_page_background_color`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessPageSetupSnapshot {
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// `portrait` or `landscape` when the section states it.
    pub orientation: Option<String>,
    pub margin_top: Option<f32>,
    pub margin_bottom: Option<f32>,
    pub margin_left: Option<f32>,
    pub margin_right: Option<f32>,
    /// Distance from the top page edge to the header story.
    pub header_distance: Option<f32>,
    /// Distance from the bottom page edge to the footer story.
    pub footer_distance: Option<f32>,
    pub column_count: Option<u32>,
    pub column_space: Option<f32>,
    pub column_equal_width: Option<bool>,
    pub column_separator: Option<bool>,
    /// Page background color as `#RRGGBB`, when one is set.
    pub background_color: Option<String>,
    /// Text stored in the document's Word-compatible VML watermark.
    pub watermark_text: Option<String>,
}

/// One line of a header or footer story, split into the three tab-separated
/// zones Word lays out. `text` is the same tab-joined form
/// `set_header_paragraph_text` / `set_footer_paragraph_text` accept, so a
/// caller can read a line, edit it and write it straight back.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessHeaderFooterLine {
    /// Index to pass back as `paragraph` when rewriting this line.
    pub paragraph: usize,
    pub left: Option<String>,
    pub center: Option<String>,
    pub right: Option<String>,
    pub text: String,
}

/// One header or footer story present in the document.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessHeaderFooterSnapshot {
    /// `header` or `footer`.
    pub role: String,
    /// `default`, `first` (title page) or `even` (odd/even headers).
    pub variant: String,
    pub lines: Vec<HeadlessHeaderFooterLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessCommentAnchor {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    pub marker_position: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessCommentSnapshot {
    pub id: String,
    pub author: Option<String>,
    pub initials: Option<String>,
    pub date: Option<String>,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub reply_to_comment_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub anchors: Vec<HeadlessCommentAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessCommentThreadSnapshot {
    pub comment: HeadlessCommentSnapshot,
    #[serde(default)]
    pub replies: Vec<HeadlessCommentThreadSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessRevisionSnapshot {
    pub id: Option<String>,
    pub kind: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub date_utc: Option<String>,
    pub paragraph: usize,
    pub start_offset: usize,
    pub end_offset: usize,
    pub text: String,
    #[serde(default)]
    pub paragraph_mark: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessDocumentStats {
    pub paragraph_count: usize,
    pub table_count: usize,
    pub comment_count: usize,
    pub revision_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessDocumentDiff {
    pub before: HeadlessDocumentStats,
    pub after: HeadlessDocumentStats,
    #[serde(default)]
    pub changed_paragraphs: Vec<usize>,
    #[serde(default)]
    pub changed_runs: Vec<HeadlessRunDiff>,
    #[serde(default)]
    pub changed_tables: Vec<HeadlessTableDiff>,
    #[serde(default)]
    pub section_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessRunDiff {
    pub paragraph: usize,
    pub before_run_count: usize,
    pub after_run_count: usize,
    #[serde(default)]
    pub changed_run_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessTableDiff {
    pub table_order_index: usize,
    pub before_rows: usize,
    pub after_rows: usize,
    pub before_columns: usize,
    pub after_columns: usize,
    #[serde(default)]
    pub changed_cells: Vec<HeadlessTableCellDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessTableCellDiff {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessOutlineItem {
    pub paragraph: usize,
    pub level: u8,
    pub style: Option<String>,
    pub text: String,
    /// Numbering label rendered in front of `text` (`第１条　`, `1.` …). Legal
    /// documents usually carry the article number here rather than in the run
    /// text, so heading detection and `insert_*_heading` matching need it.
    #[serde(default)]
    pub numbering_text: Option<String>,
    /// The article's own title (`秘密保持`), whether it is written inline
    /// (`第１条　秘密保持`) or on the parenthesised line above the article
    /// (`（秘密保持）` then `第１条　…`). It is what a reader navigates and
    /// cites by, and in the second form it is not part of this paragraph's
    /// text at all — so it is reported separately.
    #[serde(default)]
    pub title: Option<String>,
    /// Where [`Self::title`] came from: `inline` or `preceding_line`.
    #[serde(default)]
    pub title_source: Option<String>,
}

/// One unit of a chunked document: an article (`第１条` up to the next
/// article), the matter before the first article, a `章` / `節` heading, or a
/// table. Chunks are the granularity an agent reads a long contract in — small
/// enough to fit a prompt, and aligned with how the document is cited.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessChunkSnapshot {
    /// Zero-based position of this chunk in document order.
    pub index: usize,
    /// `article`, `section` (編/章/節/款/目), `preamble` or `table`.
    pub kind: String,
    pub start_paragraph: usize,
    /// Inclusive.
    pub end_paragraph: usize,
    /// `w14:paraId` of the chunk's first paragraph — a stable anchor that
    /// survives the edits which shift paragraph indices.
    #[serde(default)]
    pub para_id: Option<String>,
    /// Rendered article/section number (`第１条`, `第2条の2`, `第3章`).
    #[serde(default)]
    pub number: Option<String>,
    /// The same number normalised for comparison and sorting: `5`, or `2-2`
    /// for `第2条の2`.
    #[serde(default)]
    pub number_key: Option<String>,
    /// The chunk's title (`秘密保持`).
    #[serde(default)]
    pub title: Option<String>,
    /// `inline` or `preceding_line`; absent when the chunk has no title.
    #[serde(default)]
    pub title_source: Option<String>,
    /// Paragraph the title was read from — the article line itself, or the
    /// parenthesised line above it.
    #[serde(default)]
    pub title_paragraph: Option<usize>,
    /// The chunk's text, paragraphs joined by newlines. Empty when the query
    /// asked for structure only.
    pub text: String,
    /// Character count of the chunk's text, reported even when the text itself
    /// was not.
    pub char_count: usize,
}

/// One cross-reference found in the body text (`第５条第２項`, `前条`,
/// `第３条から第７条まで`), together with the article it points at.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessReferenceSnapshot {
    /// Paragraph the reference is written in.
    pub paragraph: usize,
    /// UTF-8 byte offsets of the reference within that paragraph, ready to
    /// hand to `replace_range` or `add_comment`.
    pub start_offset: usize,
    pub end_offset: usize,
    /// The reference exactly as written.
    pub text: String,
    /// `article` (`第５条`) or `relative` (`前条` / `次条` / `本条`).
    pub kind: String,
    /// Normalised number the reference names (`5`, `2-2`). Absent for a
    /// relative reference that has no article to resolve against.
    #[serde(default)]
    pub target_number_key: Option<String>,
    /// Paragraph index of the article it resolves to, when it resolves.
    #[serde(default)]
    pub target_paragraph: Option<usize>,
    /// `w14:paraId` of that paragraph — the anchor to keep once indices move.
    #[serde(default)]
    pub target_para_id: Option<String>,
    /// Rendered number of the target article (`第５条`).
    #[serde(default)]
    pub target_number: Option<String>,
    /// Title of the target article (`秘密保持`), so a citation can be read
    /// without jumping to it.
    #[serde(default)]
    pub target_title: Option<String>,
    /// End of a range reference (`第３条から第７条まで`); absent otherwise.
    #[serde(default)]
    pub target_end_paragraph: Option<usize>,
    #[serde(default)]
    pub target_end_number: Option<String>,
    /// Sub-references written after the article (`第２項`, `第３号`), as
    /// written. They narrow the citation without changing the paragraph it
    /// resolves to.
    #[serde(default)]
    pub sub_references: Vec<String>,
    /// False when no article in this document carries that number — a dangling
    /// citation, which is what renumbering breaks first.
    pub resolved: bool,
}

/// One tracked change, addressed the way the `revisions` query reports it:
/// a run span `[start_offset, end_offset)` in `paragraph`, or that
/// paragraph's mark when `paragraph_mark` is set, plus its `w:id` (if any).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeadlessRevisionTarget {
    pub paragraph: usize,
    #[serde(default)]
    pub start_offset: usize,
    #[serde(default)]
    pub end_offset: usize,
    #[serde(default)]
    pub paragraph_mark: bool,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessRevisionKindFilter {
    Inserted,
    Deleted,
}

/// Inline Word fields exposed by the non-AI Insert ribbon. The serialized
/// names match the public JavaScript API while `HeadlessCommand` itself keeps
/// its existing snake_case tagged representation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessFieldKind {
    Date,
    Time,
    Page,
    NumPages,
    FileName,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessEquationPlacement {
    Inline,
    Block,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessDropCapKind {
    Drop,
    Margin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessMultilevelListPreset {
    DecimalHierarchy,
    ChineseOfficial,
    BulletHierarchy,
}

/// One level of a custom Word multilevel-list definition.
///
/// Indentation values use OOXML twips so browser integrations can round-trip
/// the values shown by Word's list-definition dialog without unit drift.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessCustomNumberingLevel {
    pub num_fmt: String,
    pub lvl_text: String,
    pub indent_left: i32,
    #[serde(default)]
    pub hanging: Option<i32>,
    #[serde(default)]
    pub start: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessChangeCaseMode {
    Upper,
    Lower,
    Title,
    Sentence,
    /// 大文字と小文字を入れ替える.
    Toggle,
    /// 半角: full-width ASCII, katakana and spaces to their half-width forms.
    HalfWidth,
    /// 全角: ASCII, half-width katakana and spaces to their full-width forms.
    FullWidth,
    /// カタカナ: hiragana to katakana.
    Katakana,
    /// ひらがな: katakana to hiragana.
    Hiragana,
}

/// DrawingML preset geometries that Jade can currently render and round-trip
/// without flattening them to a rectangle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HeadlessShapeKind {
    Rectangle,
    Line,
    LineArrow,
    LineArrowDouble,
    LineBent,
    LineCurved,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Triangle,
    RightTriangle,
    Parallelogram,
    Trapezoid,
    Pentagon,
    Hexagon,
    Octagon,
    Plus,
    MathPlus,
    LeftBrace,
    RightBrace,
    BlockArc,
    Pie,
    Chord,
    Teardrop,
    Frame,
    HalfFrame,
    Corner,
    #[serde(rename = "diagStripe")]
    DiagonalStripe,
    Donut,
    NoSmoking,
    FoldedCorner,
    Bevel,
    Cube,
    Can,
    LightningBolt,
    Heart,
    Sun,
    Moon,
    Cloud,
    Arc,
    Plaque,
    SmileyFace,
    LeftBracket,
    RightBracket,
    FourPointStar,
    FivePointStar,
    SixPointStar,
    SevenPointStar,
    EightPointStar,
    TenPointStar,
    TwelvePointStar,
    SixteenPointStar,
    TwentyFourPointStar,
    ThirtyTwoPointStar,
    IrregularSealOne,
    IrregularSealTwo,
    Ribbon,
    #[serde(rename = "ribbon2", alias = "ribbonTwo")]
    RibbonTwo,
    Wave,
    DoubleWave,
    FlowChartProcess,
    FlowChartAlternateProcess,
    FlowChartDecision,
    FlowChartPredefinedProcess,
    FlowChartInternalStorage,
    FlowChartDocument,
    FlowChartMultidocument,
    FlowChartTerminator,
    FlowChartPreparation,
    FlowChartManualInput,
    FlowChartManualOperation,
    FlowChartConnector,
    FlowChartOffpageConnector,
    FlowChartMagneticDisk,
    FlowChartMagneticDrum,
    FlowChartDisplay,
    FlowChartDelay,
    FlowChartOr,
    FlowChartCollate,
    FlowChartSort,
    FlowChartExtract,
    FlowChartMerge,
    FlowChartSummingJunction,
    FlowChartPunchedTape,
    #[serde(rename = "wedgeRectCallout")]
    WedgeRectangleCallout,
    #[serde(rename = "wedgeRoundRectCallout")]
    WedgeRoundedRectangleCallout,
    WedgeEllipseCallout,
    CloudCallout,
    RightArrow,
    LeftArrow,
    UpArrow,
    DownArrow,
    LeftRightArrow,
    UpDownArrow,
    NotchedRightArrow,
    Chevron,
    HomePlate,
    QuadArrow,
    BentArrow,
    UturnArrow,
    CurvedRightArrow,
    StripedRightArrow,
    SnipOneRectangle,
    SnipTwoSameRectangle,
    SnipTwoDiagonalRectangle,
    SnipRoundRectangle,
    RoundOneRectangle,
    RoundTwoSameRectangle,
    RoundTwoDiagonalRectangle,
}

const fn default_drop_cap_lines() -> u8 {
    3
}

fn default_bibliography_heading() -> String {
    "Bibliography".to_string()
}

/// One styled paragraph in a cover page inserted at the start of a document.
/// The trailing page-break paragraph is authored by Jade and is not part of
/// this list, preventing callers from accidentally leaving body content on the
/// cover page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct HeadlessCoverParagraph {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub color: Option<String>,
    /// Font size in points.
    #[serde(default)]
    pub font_size: Option<f32>,
    /// `left`, `center`, or `right`.
    #[serde(default)]
    pub alignment: Option<String>,
    /// Paragraph spacing in points.
    #[serde(default)]
    pub space_before: Option<f32>,
    #[serde(default)]
    pub space_after: Option<f32>,
    #[serde(default)]
    pub shading_color: Option<String>,
    #[serde(default)]
    pub border_top: bool,
    #[serde(default)]
    pub border_bottom: bool,
    #[serde(default)]
    pub border_left: bool,
    #[serde(default)]
    pub border_right: bool,
    /// Paragraph indents in points.
    #[serde(default)]
    pub indent_left: Option<f32>,
    #[serde(default)]
    pub indent_right: Option<f32>,
}

/// One source in Word's bibliography `customXml` catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeadlessCitationSource {
    pub tag: String,
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub author: String,
    pub title: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// `w:documentProtection` credentials. `None` in the command removes the
/// restriction; the supported authored mode is enforced `readOnly`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeadlessDocumentProtection {
    pub edit: String,
    pub enforced: bool,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub salt: Option<String>,
    #[serde(default)]
    pub spin_count: Option<u32>,
    #[serde(default)]
    pub algorithm_sid: Option<u32>,
}

// テストで組み立てた値と比較できるようにする（デシリアライズ結果の検証に要る）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
// 誤ったフィールド名を黙って既定値で埋めると、呼び出し側は「別の場所が返った」ことに
// 気付けない。名指しでエラーにする。
#[serde(deny_unknown_fields)]
pub enum HeadlessCommand {
    InsertText {
        paragraph: usize,
        offset: usize,
        text: String,
    },
    ReplaceRange {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        text: String,
    },
    ReplacePlaceholder {
        placeholder: String,
        text: String,
        #[serde(default)]
        max_replacements: Option<usize>,
    },
    DeleteText {
        paragraph: usize,
        offset: usize,
        #[serde(default)]
        direction: HeadlessDeleteDirection,
        #[serde(default = "default_delete_count")]
        count: usize,
    },
    InsertParagraphBreak {
        paragraph: usize,
        offset: usize,
        #[serde(default = "default_break_count")]
        count: usize,
    },
    /// Split a top-level body paragraph at this snapshot coordinate and begin
    /// the following section with the requested Word break type.
    InsertSectionBreak {
        paragraph: usize,
        offset: usize,
        break_type: HeadlessSectionBreakType,
    },
    /// Inserts `text` at a rendered offset where zero-width runs — text
    /// boxes and note references — sit, after the first `after_anchors` of
    /// them (`insert_text` types before them all, or at a paragraph's start
    /// after them). The text takes a run of its own, formatted like the
    /// text before it.
    InsertTextAmongAnchors {
        paragraph: usize,
        offset: usize,
        after_anchors: usize,
        text: String,
    },
    /// Inserts a Word endnote reference at the exact current-state body
    /// coordinate and creates the matching package entry.
    InsertEndnote {
        paragraph: usize,
        offset: usize,
        #[serde(default)]
        text: String,
    },
    /// Replaces the plain-text body of an existing substantive endnote.
    UpdateEndnote {
        endnote_id: String,
        text: String,
    },
    /// Removes an endnote entry and every body reference to its id.
    DeleteEndnote {
        endnote_id: String,
    },
    /// Inserts a Word footnote reference at the exact current-state body
    /// coordinate and creates the matching `word/footnotes.xml` entry.
    InsertFootnote {
        paragraph: usize,
        offset: usize,
        #[serde(default)]
        text: String,
    },
    /// Replaces the plain-text body of an existing substantive footnote.
    UpdateFootnote {
        footnote_id: String,
        text: String,
    },
    /// Removes a footnote entry and every body reference to its id.
    DeleteFootnote {
        footnote_id: String,
    },
    /// Moves a footnote, retaining its formatted body and every reference.
    ConvertFootnoteToEndnote {
        footnote_id: String,
    },
    /// Moves an endnote, retaining its formatted body and every reference.
    ConvertEndnoteToFootnote {
        endnote_id: String,
    },
    /// Adds a source to Word's bibliography catalog without inserting text.
    AddCitationSource {
        source: HeadlessCitationSource,
    },
    /// Replaces the exact current selection with the source's display citation.
    InsertCitation {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        tag: String,
    },
    /// Inserts a heading and one bibliography paragraph per source.
    InsertBibliography {
        after_paragraph: usize,
        #[serde(default = "default_bibliography_heading")]
        heading: String,
    },
    /// Sets/removes Word's editing restriction in `word/settings.xml`.
    SetDocumentProtection {
        protection: Option<HeadlessDocumentProtection>,
    },
    ApplyBold {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// Enables or explicitly disables bold formatting.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    ApplyItalic {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// Enables or explicitly disables italic formatting.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    ApplyUnderline {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// OOXML `w:u@w:val`; omitted for a plain single underline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// Enables or explicitly disables underline formatting.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    ApplyStrike {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// Emits Word's `w:dstrike` instead of `w:strike` when true.
        #[serde(default)]
        double: bool,
        /// Enables or explicitly disables both single and double strike.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    ApplySuperscript {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// Enables or removes Word's superscript baseline formatting.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    ApplySubscript {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        /// Enables or removes Word's subscript baseline formatting.
        #[serde(default = "default_true")]
        enabled: bool,
    },
    SetFontSize {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        font_size: f32,
    },
    SetFontFamily {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        font_family: String,
    },
    ClearFormatting {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
    },
    ChangeCase {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        mode: HeadlessChangeCaseMode,
    },
    SetBold {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        value: bool,
    },
    SetParagraphAlignment {
        start_paragraph: usize,
        end_paragraph: usize,
        alignment: String,
    },
    /// Sets the paragraph base direction. `bidi: true` authors RTL; false
    /// explicitly overrides inherited RTL with LTR.
    SetParagraphDirection {
        start_paragraph: usize,
        end_paragraph: usize,
        bidi: bool,
    },
    SetParagraphStyle {
        start_paragraph: usize,
        end_paragraph: usize,
        #[serde(default)]
        style_id: Option<String>,
    },
    /// Replaces direct paragraph shading with a solid `#RRGGBB` fill, or
    /// removes it when `color` is null.
    SetParagraphShading {
        start_paragraph: usize,
        end_paragraph: usize,
        #[serde(default)]
        color: Option<String>,
    },
    /// Replaces the selected paragraphs' four direct border sides. Omitted
    /// sides are removed; an all-null command clears the border box.
    SetParagraphBorders {
        start_paragraph: usize,
        end_paragraph: usize,
        #[serde(default)]
        top: Option<HeadlessBorderEdge>,
        #[serde(default)]
        bottom: Option<HeadlessBorderEdge>,
        #[serde(default)]
        left: Option<HeadlessBorderEdge>,
        #[serde(default)]
        right: Option<HeadlessBorderEdge>,
    },
    /// Applies or removes a first-letter drop cap on exact current-state body
    /// paragraph indices. `kind: null` removes it; Word's default is 3 lines.
    SetDropCap {
        start_paragraph: usize,
        end_paragraph: usize,
        #[serde(default)]
        kind: Option<HeadlessDropCapKind>,
        #[serde(default = "default_drop_cap_lines")]
        lines: u8,
    },
    /// Applies (or with `style_id: None` removes) a character style on a range.
    /// Offsets are the same rendered byte offsets the other range commands take.
    SetCharacterStyle {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        #[serde(default)]
        style_id: Option<String>,
    },
    SetParagraphIndent {
        start_paragraph: usize,
        end_paragraph: usize,
        left: Option<f32>,
        right: Option<f32>,
        first_line: Option<f32>,
        hanging: Option<f32>,
        #[serde(default)]
        left_chars: Option<i32>,
        #[serde(default)]
        right_chars: Option<i32>,
        /// Word's paragraph dialog 特殊インデント「なし」: remove any
        /// first-line or hanging indent (character twins included) while
        /// leaving the left/right edges alone.
        #[serde(default)]
        clear_special: bool,
    },
    /// Ribbon-style increase/decrease indent. Each paragraph in the range
    /// steps relative to its own current start-edge indent: character-unit
    /// indents move by one character, point indents snap on the document's
    /// default tab grid.
    AdjustParagraphIndent {
        start_paragraph: usize,
        end_paragraph: usize,
        direction: HeadlessIndentDirection,
        /// `start` (default) steps the start edge like Ctrl+M; `hanging`
        /// steps the hanging indent like Ctrl+T, keeping the first line where
        /// it is and never touching list levels.
        #[serde(default)]
        target: HeadlessIndentTarget,
    },
    /// Word's Ctrl+0: add 12pt of space before each paragraph in the range,
    /// or remove it when every paragraph already has exactly 12pt.
    ToggleParagraphSpaceBefore {
        start_paragraph: usize,
        end_paragraph: usize,
    },
    /// Word's Ctrl+Q: strip direct paragraph formatting (alignment, indents,
    /// spacing, shading, borders, tab stops, frames) while keeping the
    /// paragraph style, its numbering, and run formatting.
    ResetParagraphFormatting {
        start_paragraph: usize,
        end_paragraph: usize,
    },
    /// Ribbon-style grow/shrink font. Each run in the range steps relative to
    /// its own effective size through Word's ribbon size list.
    AdjustFontSize {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        direction: HeadlessFontSizeDirection,
        /// `ladder` (default) steps through Word's ribbon size list; `point`
        /// moves exactly one point like Word's Ctrl+] / Ctrl+[.
        #[serde(default)]
        step: HeadlessFontSizeStep,
    },
    /// Applies the paragraph dialog's indent, special indent and before/after
    /// spacing values as one atomic edit and one undo-history entry. Omitted
    /// fields stay as they are, like Word's dialog writes only what changed.
    SetParagraphMetrics {
        start_paragraph: usize,
        end_paragraph: usize,
        #[serde(default)]
        left: Option<f32>,
        #[serde(default)]
        right: Option<f32>,
        #[serde(default)]
        before: Option<f32>,
        #[serde(default)]
        after: Option<f32>,
        /// 特殊インデント「字下げ」 in points (exclusive with `hanging`).
        #[serde(default)]
        first_line: Option<f32>,
        /// 特殊インデント「ぶら下げ」 in points.
        #[serde(default)]
        hanging: Option<f32>,
        /// 特殊インデント「なし」: remove first-line and hanging indents.
        #[serde(default)]
        clear_special: bool,
    },
    SetNumbering {
        start_paragraph: usize,
        end_paragraph: usize,
        num_id: Option<String>,
        level: Option<u32>,
    },
    /// Applies a stable nine-level list gallery and creates the required
    /// `numbering.xml` package objects when they do not exist yet.
    ApplyMultilevelList {
        start_paragraph: usize,
        end_paragraph: usize,
        preset: HeadlessMultilevelListPreset,
        level: u32,
    },
    /// Creates and applies an arbitrary one-to-nine-level Word list
    /// definition. Each invocation receives its own numbering instance.
    ApplyCustomMultilevelList {
        start_paragraph: usize,
        end_paragraph: usize,
        levels: Vec<HeadlessCustomNumberingLevel>,
        level: u32,
    },
    SetFontColor {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        color: String,
    },
    SetHighlightColor {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        color: String,
    },
    /// Applies Word's フォント dialog to a range: every field `format` sets,
    /// and nothing else, as one step.
    SetRunFormat {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        format: Box<HeadlessRunFormat>,
    },
    StartChangeTracking {
        author: Option<String>,
        initials: Option<String>,
        date: Option<String>,
        date_utc: Option<String>,
    },
    StopChangeTracking,
    /// Accept tracked changes. Every filter is optional and they combine;
    /// with none, all revisions are accepted.
    AcceptRevisions {
        kind: Option<HeadlessRevisionKindFilter>,
        start_paragraph: Option<usize>,
        end_paragraph: Option<usize>,
        /// Accept only the revision carrying this `w:id`, as reported by the
        /// `revisions` query — how a reviewer takes one specific change
        /// instead of a whole range.
        #[serde(default)]
        revision_id: Option<String>,
        /// Accept only revisions whose `w:id` is in this list — the changes a
        /// selection touches, or the ones a kind/author filter shows. An empty
        /// list decides nothing (and records no undo step).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision_ids: Option<Vec<String>>,
        /// Decide exactly these revisions, addressed by position as the
        /// `revisions` query reports them. Unlike `revision_ids` this stays
        /// exact when a file reuses ids and reaches unidentified revisions.
        /// An empty list decides nothing (and records no undo step).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision_targets: Option<Vec<HeadlessRevisionTarget>>,
    },
    /// Reject tracked changes. Every filter is optional and they combine;
    /// with none, all revisions are rejected.
    RejectRevisions {
        kind: Option<HeadlessRevisionKindFilter>,
        start_paragraph: Option<usize>,
        end_paragraph: Option<usize>,
        /// Reject only the revision carrying this `w:id`, as reported by the
        /// `revisions` query.
        #[serde(default)]
        revision_id: Option<String>,
        /// Reject only revisions whose `w:id` is in this list; see
        /// `AcceptRevisions::revision_ids`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision_ids: Option<Vec<String>>,
        /// Decide exactly these revisions, addressed by position as the
        /// `revisions` query reports them. Unlike `revision_ids` this stays
        /// exact when a file reuses ids and reaches unidentified revisions.
        /// An empty list decides nothing (and records no undo step).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision_targets: Option<Vec<HeadlessRevisionTarget>>,
    },
    /// Anchor a comment to a character range.
    ///
    /// `start_offset` / `end_offset` may be omitted to comment on the whole
    /// paragraph range: the start defaults to the beginning of
    /// `start_paragraph` and the end to the end of `end_paragraph`. Callers
    /// that only want "comment on this paragraph" therefore do not have to
    /// compute offsets.
    AddComment {
        start_paragraph: usize,
        #[serde(default)]
        start_offset: Option<usize>,
        end_paragraph: usize,
        #[serde(default)]
        end_offset: Option<usize>,
        text: String,
        author: Option<String>,
        initials: Option<String>,
        date: Option<String>,
    },
    /// Anchor a comment to the first (or `occurrence`-th, 0-based) match of
    /// `query`.
    ///
    /// This is the shape callers usually want: "comment on this phrase". It
    /// removes the manual `search_text` -> offset -> `add_comment` round trip,
    /// which is easy to get wrong because offsets are byte offsets.
    AddCommentByText {
        query: String,
        #[serde(default)]
        occurrence: Option<usize>,
        text: String,
        author: Option<String>,
        initials: Option<String>,
        date: Option<String>,
    },
    /// Like [`HeadlessCommand::AddComment`], but the target paragraph is
    /// addressed by its stable `w14:paraId` instead of a positional index. The
    /// comment spans `start_offset..end_offset` within that paragraph.
    AddCommentByParaId {
        para_id: String,
        start_offset: usize,
        end_offset: usize,
        text: String,
        author: Option<String>,
        initials: Option<String>,
        date: Option<String>,
    },
    /// Reply to an existing comment, continuing its thread.
    ///
    /// Word models a reply as another comment anchored to the same range as the
    /// one it answers, tied to it by `w15:paraIdParent` in
    /// `word/commentsExtended.xml`. Review is a conversation — a counterparty's
    /// point and the answer to it have to stay attached to each other — so the
    /// reply is its own command rather than a second top-level comment on the
    /// same words.
    ReplyToComment {
        comment_id: String,
        text: String,
        author: Option<String>,
        initials: Option<String>,
        date: Option<String>,
    },
    UpdateComment {
        comment_id: String,
        text: String,
    },
    DeleteComment {
        comment_id: String,
    },
    ResolveComment {
        comment_id: String,
        resolved: bool,
    },
    SetHeaderParagraphText {
        paragraph: usize,
        text: String,
    },
    SetFooterParagraphText {
        paragraph: usize,
        text: String,
    },
    /// Toggles a section's first-page header/footer stories (`w:titlePg`) and
    /// the document-wide odd/even stories (`w:evenAndOddHeaders`). Enabling
    /// materializes editable blank variants when the package does not define
    /// them yet. `section` is an index into the document's sections; omitted,
    /// it targets the final section, the way Word's ribbon targets the
    /// section holding the caret.
    SetHeaderFooterVariants {
        different_first_page: bool,
        different_odd_even_pages: bool,
        #[serde(default)]
        section: Option<usize>,
    },
    InsertTable {
        after_paragraph: usize,
        rows: usize,
        columns: usize,
        #[serde(default)]
        column_widths: Vec<f32>,
        /// When given, `after_paragraph` names the caret paragraph and the
        /// table lands where Word puts it for a caret at this byte offset:
        /// above the paragraph at offset 0 (and above an empty paragraph),
        /// between the two halves of a paragraph split in the middle, after
        /// the paragraph at its end. Omitted, the table simply follows
        /// `after_paragraph`.
        #[serde(default)]
        caret_offset: Option<usize>,
    },
    /// Inserts a generated table of contents after `after_paragraph`, collecting
    /// headings in the inclusive outline-level range `[outline_min, outline_max]`
    /// (Word's `TOC \o "MIN-MAX"`). Mirrors References ▸ Table of Contents.
    InsertTableOfContents {
        after_paragraph: usize,
        #[serde(default = "default_toc_outline_min")]
        outline_min: u8,
        #[serde(default = "default_toc_outline_max")]
        outline_max: u8,
    },
    /// Inserts a centered Word caption paragraph after `after_paragraph`.
    /// The visible number is backed by a live `SEQ` field so Word can update it.
    InsertCaption {
        after_paragraph: usize,
        label: String,
        #[serde(default)]
        description: String,
    },
    /// Replaces the exact current-state selection with a dirty inline Word
    /// field and its caller-provided cached display result.
    InsertField {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
        kind: HeadlessFieldKind,
        cached_text: String,
    },
    /// Inserts native Office Math Markup Language at an exact current-state
    /// UTF-8 boundary. Inline accepts an `m:oMath` root; block accepts an
    /// `m:oMathPara` root and splits the target paragraph around it.
    InsertEquation {
        paragraph: usize,
        offset: usize,
        omml: String,
        placement: HeadlessEquationPlacement,
    },
    /// Replaces the exact current-state selection with a blank Word page.
    /// When document content follows the selection, two page breaks surround
    /// the new page and the caret remains between them. At the document end a
    /// single break creates the trailing blank page.
    InsertBlankPage {
        start_paragraph: usize,
        start_offset: usize,
        end_paragraph: usize,
        end_offset: usize,
    },
    /// Inserts styled cover paragraphs at the absolute document start and a
    /// trailing `pageBreakBefore` paragraph, pushing all existing body content
    /// to page 2. The complete block is one atomic history operation.
    InsertCoverPage {
        paragraphs: Vec<HeadlessCoverParagraph>,
    },
    /// Deletes the `text_box`-th text box (in run order) anchored in
    /// paragraph `paragraph`: the anchoring run's drawing and everything in
    /// the box. One atomic history operation.
    DeleteTextBox {
        paragraph: usize,
        text_box: usize,
    },
    /// Inserts a floating WordprocessingShape at an exact current-state UTF-8
    /// boundary. The shape contains one editable text paragraph and is one
    /// atomic history operation.
    InsertShape {
        paragraph: usize,
        offset: usize,
        shape: HeadlessShapeKind,
        width_px: f64,
        height_px: f64,
        #[serde(default)]
        fill_color: Option<String>,
        #[serde(default)]
        border_color: Option<String>,
        border_width_px: f64,
        #[serde(default)]
        text: String,
    },
    /// Inserts an invisible XE index-entry marker at a UTF-8 text boundary.
    MarkIndexEntry {
        paragraph: usize,
        offset: usize,
        term: String,
    },
    /// Inserts a generated dirty INDEX field after a top-level body paragraph,
    /// collecting every XE marker in the current document state.
    InsertIndex {
        after_paragraph: usize,
    },
    /// Regenerates the first existing table of contents from the current
    /// headings (Word's References ▸ Update Table). Mirrors right-click ▸ field
    /// update on a TOC.
    UpdateTableOfContents,
    SetTableWidth {
        table_order_index: usize,
        width: String,
    },
    /// Word's AutoFit choices: content-driven layout, page-window width, or
    /// fixed column geometry.
    SetTableFit {
        table_order_index: usize,
        mode: HeadlessTableFitMode,
    },
    /// Inline table placement or Word's floating-table "Around" wrapping.
    SetTableTextWrapping {
        table_order_index: usize,
        wrapping: HeadlessTableTextWrapping,
    },
    /// Sets one width per logical grid column and switches the table to fixed
    /// layout. Widths below the editor's 9pt visual minimum are clamped.
    SetTableColumnWidths {
        table_order_index: usize,
        widths: Vec<f32>,
    },
    /// Word's 表の分割 (Ctrl+Shift+Enter): the rows from `row_index` on become
    /// a second table, with an empty paragraph between the two. `row_index: 0`
    /// puts the empty paragraph in front of the table instead, as Word does.
    SplitTable {
        table_order_index: usize,
        row_index: usize,
    },
    /// Word's セルの分割: the cell at (`row`, `column`) becomes `columns` × `rows`
    /// cells. Extra columns are new grid columns that the other rows absorb
    /// into the cell covering them; extra rows are new rows whose other cells
    /// are vertically merged with the row above.
    SplitTableCell {
        table_order_index: usize,
        row: usize,
        column: usize,
        columns: usize,
        rows: usize,
    },
    /// Word's 行の高さを揃える: every row gets the same height with the
    /// `atLeast` rule. `height` (points) is the height to use; when omitted the
    /// rows must all carry an explicit height and their average is used.
    DistributeTableRows {
        table_order_index: usize,
        #[serde(default)]
        height: Option<f32>,
    },
    /// Word's 列の幅を揃える: every grid column gets the same width and the
    /// table switches to fixed layout. `total_width` (points) is the width to
    /// share out; when omitted the grid must carry explicit widths and their
    /// sum is used.
    DistributeTableColumns {
        table_order_index: usize,
        #[serde(default)]
        total_width: Option<f32>,
    },
    /// How the table sits between the margins (`w:jc`) — a 別表 is usually
    /// centred while the body text around it is not. Unrelated to the alignment
    /// of the paragraphs inside the cells.
    SetTableAlignment {
        table_order_index: usize,
        alignment: String,
    },
    /// Fills a cell's background (`w:shd`), the way a header row is shaded.
    /// `color: null` clears it.
    SetTableCellShading {
        table_order_index: usize,
        row: usize,
        column: usize,
        #[serde(default)]
        color: Option<String>,
    },
    /// Applies one of Jade's deterministic direct-formatting table presets.
    /// Unlike a package-local `w:tblStyle` id, these presets render the same in
    /// Jade and Word because every border and cell fill is persisted directly.
    ApplyTableStylePreset {
        table_order_index: usize,
        preset: String,
    },
    /// Applies a package-local `w:style w:type="table"` by id. `style_id:
    /// null` removes the named style while preserving direct formatting.
    ApplyTableNamedStyle {
        table_order_index: usize,
        #[serde(default)]
        style_id: Option<String>,
    },
    /// Conditional regions controlled by `w:tblLook`.
    SetTableLook {
        table_order_index: usize,
        #[serde(default)]
        first_row: Option<bool>,
        #[serde(default)]
        banded_rows: Option<bool>,
    },
    /// Where a cell's content sits in its box (`w:vAlign`): `top`, `center` or
    /// `bottom`.
    SetTableCellVerticalAlignment {
        table_order_index: usize,
        row: usize,
        column: usize,
        alignment: String,
    },
    /// One border edge: the line style (`single`, `double`, `dashed`, `none`,
    /// …), its thickness in points, and its colour. Absent parts fall back to a
    /// single line, Word's own default thickness, and `auto`.
    /// Table borders (`w:tblBorders`): the four outer edges plus the interior
    /// grid lines. Only the edges named are touched — switching the inside
    /// lines off leaves the frame alone.
    SetTableBorders {
        table_order_index: usize,
        #[serde(default)]
        top: Option<HeadlessBorderEdge>,
        #[serde(default)]
        bottom: Option<HeadlessBorderEdge>,
        #[serde(default)]
        left: Option<HeadlessBorderEdge>,
        #[serde(default)]
        right: Option<HeadlessBorderEdge>,
        /// The horizontal lines between rows.
        #[serde(default)]
        inside_h: Option<HeadlessBorderEdge>,
        /// The vertical lines between columns.
        #[serde(default)]
        inside_v: Option<HeadlessBorderEdge>,
    },
    /// One cell's own borders (`w:tcBorders`), which win over the table's
    /// defaults. A cell has only its four edges.
    SetTableCellBorders {
        table_order_index: usize,
        row: usize,
        column: usize,
        #[serde(default)]
        top: Option<HeadlessBorderEdge>,
        #[serde(default)]
        bottom: Option<HeadlessBorderEdge>,
        #[serde(default)]
        left: Option<HeadlessBorderEdge>,
        #[serde(default)]
        right: Option<HeadlessBorderEdge>,
    },
    /// Row properties for a table that runs over a page break: repeat this row
    /// as the header at the top of every page (`w:tblHeader`), and keep the row
    /// from splitting across pages (`w:cantSplit`).
    SetTableRowProperties {
        table_order_index: usize,
        row: usize,
        #[serde(default)]
        repeat_header: Option<bool>,
        #[serde(default)]
        cant_split: Option<bool>,
    },
    /// Restore row/table padding by removing all direct cell margins.
    ResetTableCellMargins {
        table_order_index: usize,
        row: usize,
        column: usize,
    },
    /// Authored row height in points. auto removes the constraint; atLeast and
    /// exact require a positive height. Unlike drag resizing, excludes padding.
    SetTableRowHeightProperties {
        table_order_index: usize,
        row: usize,
        mode: String,
        #[serde(default)]
        height: Option<f32>,
    },
    /// Inner padding of a cell (`w:tcMar`), or the table's default for cells
    /// that set none of their own (`w:tblCellMar`) when `row` and `column` are
    /// omitted. Each edge is optional and in points.
    SetTableCellMargins {
        table_order_index: usize,
        #[serde(default)]
        row: Option<usize>,
        #[serde(default)]
        column: Option<usize>,
        #[serde(default)]
        top: Option<f32>,
        #[serde(default)]
        bottom: Option<f32>,
        #[serde(default)]
        left: Option<f32>,
        #[serde(default)]
        right: Option<f32>,
    },
    SetTableCellText {
        table_order_index: usize,
        row: usize,
        column: usize,
        text: String,
    },
    FillTableRow {
        table_order_index: usize,
        row: usize,
        values: Vec<String>,
    },
    /// Inserts an empty row before `row_index`. `template_row` names the row
    /// whose formatting the new row takes (Word copies the caret row's shading,
    /// height, margins and paragraph formatting); `None` inserts a plain row.
    InsertTableRow {
        table_order_index: usize,
        row_index: usize,
        #[serde(default)]
        template_row: Option<usize>,
    },
    CloneTableRow {
        table_order_index: usize,
        source_row: usize,
        target_row: usize,
        #[serde(default = "default_clone_table_row_clear_text")]
        clear_text: bool,
    },
    DeleteTableRow {
        table_order_index: usize,
        row_index: usize,
    },
    /// Inserts a grid column before `column_index`. `template_column` names
    /// the column whose cell formatting the new cells take (Word copies the
    /// caret column's); `None` inserts plain cells.
    InsertTableColumn {
        table_order_index: usize,
        column_index: usize,
        width: Option<f32>,
        #[serde(default)]
        template_column: Option<usize>,
    },
    DeleteTableColumn {
        table_order_index: usize,
        column_index: usize,
    },
    /// Removes one logical cell while keeping the table grid. The remaining
    /// cells in its row move left, or cells below it move up.
    DeleteTableCell {
        table_order_index: usize,
        row: usize,
        column: usize,
        direction: HeadlessTableCellDeleteDirection,
    },
    /// Inserts one empty cell (formatted like the target) before the target
    /// cell, shifting the target and the cells after it right or down.
    InsertTableCell {
        table_order_index: usize,
        row: usize,
        column: usize,
        direction: HeadlessTableCellInsertDirection,
    },
    /// Remove an entire top-level table from the document body. Mirrors
    /// Word's 表の削除; also the cascade target when the last remaining
    /// row/column of a table is deleted.
    DeleteTable {
        table_order_index: usize,
    },
    ResizeTableColumnBetween {
        table_order_index: usize,
        left_column_index: usize,
        new_left_width: f32,
        new_right_width: f32,
    },
    ResizeTableColumnRightEdge {
        table_order_index: usize,
        column_index: usize,
        new_width: f32,
    },
    ResizeTableRowBetween {
        table_order_index: usize,
        row_index: usize,
        new_top_height: f32,
        new_bottom_height: f32,
    },
    ResizeTableRowBottomEdge {
        table_order_index: usize,
        row_index: usize,
        new_height: f32,
    },
    MergeTableCells {
        table_order_index: usize,
        selected_cells: Vec<HeadlessTableCellSelection>,
    },
    /// Reverse of `MergeTableCells`: clear `w:gridSpan` / `w:vMerge` on every
    /// target cell so each returns to its own grid column. The caller passes
    /// the logical-grid positions currently occupied by merged master cells.
    UnmergeTableCells {
        table_order_index: usize,
        target_cells: Vec<HeadlessTableCellSelection>,
    },
    /// Toggles a boolean on/off element directly under `<w:settings>` in
    /// `word/settings.xml` (e.g. `w:evenAndOddHeaders`, `w:autoHyphenation`,
    /// `w:mirrorMargins`, `w:trackChanges`). `name` is the fully qualified
    /// element name. `enabled = true` inserts a bare element if absent;
    /// `enabled = false` removes it.
    SetDocumentSetting {
        name: String,
        enabled: bool,
    },
    /// Sets the default section's page orientation (Word の「印刷の向き」,
    /// ECMA-376 Part 1 §17.6.13 `pgSz`). `landscape = true` makes the page
    /// wider than tall; `false` restores portrait.
    SetPageOrientation {
        landscape: bool,
    },
    /// Sets the default section's page size, given in portrait points
    /// (height ≥ width). The current orientation is preserved.
    SetPageSize {
        portrait_width: f32,
        portrait_height: f32,
    },
    /// Sets the default section's page margins, in points.
    /// Page margins. Every field is optional and `None` leaves that edge as it
    /// is, so the binding gutter can be widened without restating the rest.
    /// `header` / `footer` are the distances from the page edge to the header
    /// and footer stories; `gutter` is the extra space on the bound edge.
    SetPageMargins {
        #[serde(default)]
        top: Option<f32>,
        #[serde(default)]
        bottom: Option<f32>,
        #[serde(default)]
        left: Option<f32>,
        #[serde(default)]
        right: Option<f32>,
        #[serde(default)]
        header: Option<f32>,
        #[serde(default)]
        footer: Option<f32>,
        #[serde(default)]
        gutter: Option<f32>,
    },
    /// Sets equal-width columns on the default section. `space` is the gutter
    /// between columns in points; `separator` draws a rule in each gutter.
    SetPageColumns {
        count: u32,
        space: f32,
        separator: bool,
    },
    /// Replaces or clears the default section's page frame (`w:pgBorders`).
    /// `clear` is explicit so an omitted/misspelled edge cannot accidentally
    /// erase a frame. When clearing, all edge and offset fields must be absent.
    SetPageBorders {
        clear: bool,
        #[serde(default)]
        top: Option<HeadlessBorderEdge>,
        #[serde(default)]
        bottom: Option<HeadlessBorderEdge>,
        #[serde(default)]
        left: Option<HeadlessBorderEdge>,
        #[serde(default)]
        right: Option<HeadlessBorderEdge>,
        /// `page` measures from the paper edge; `text` from the body box.
        #[serde(default)]
        offset_from: Option<String>,
    },
    /// Replaces or clears the Word-compatible text watermark authored into
    /// every referenced header variant. `clear` is explicit: clearing forbids
    /// `text`, while setting requires it.
    SetWatermark {
        clear: bool,
        #[serde(default)]
        text: Option<String>,
    },
    /// Replaces the package-wide Word theme fonts and color scheme while
    /// preserving unmodeled theme XML.
    SetDocumentTheme {
        theme: HeadlessDocumentTheme,
    },
    /// Sets the document page background color (`#RRGGBB`); `None` clears it.
    SetPageBackgroundColor {
        color: Option<String>,
    },
    /// Sets the paragraph line-spacing multiplier (1.0/1.5/2.0; `None` clears).
    /// Paragraph spacing (`w:spacing`): the line height and the space above and
    /// below. `line` is a multiple of single spacing under the default `auto`
    /// rule, and points under `exact` / `at_least`. Each field is optional, so
    /// the space after a clause can be set without touching its line height.
    SetParagraphLineSpacing {
        start_paragraph: usize,
        end_paragraph: usize,
        /// Absent leaves the line height alone, `null` clears it, a number sets
        /// it — so a call that only adjusts `after` does not reset the leading.
        #[serde(default)]
        line: Option<Option<f32>>,
        /// `auto` (default), `exact` or `atLeast`.
        #[serde(default)]
        line_rule: Option<String>,
        /// Space above the paragraph, in points.
        #[serde(default)]
        before: Option<f32>,
        /// Space below the paragraph, in points.
        #[serde(default)]
        after: Option<f32>,
    },
    InsertAfterHeading {
        heading: String,
        text: String,
        #[serde(default)]
        level: Option<u8>,
        #[serde(default)]
        occurrence: usize,
    },
    InsertBeforeHeading {
        heading: String,
        text: String,
        #[serde(default)]
        level: Option<u8>,
        #[serde(default)]
        occurrence: usize,
    },
}

fn default_clone_table_row_clear_text() -> bool {
    true
}

impl HeadlessCommand {
    pub fn name(&self) -> &'static str {
        match self {
            HeadlessCommand::InsertText { .. } => "insert_text",
            HeadlessCommand::ReplaceRange { .. } => "replace_range",
            HeadlessCommand::ReplacePlaceholder { .. } => "replace_placeholder",
            HeadlessCommand::DeleteText { .. } => "delete_text",
            HeadlessCommand::InsertParagraphBreak { .. } => "insert_paragraph_break",
            HeadlessCommand::InsertSectionBreak { .. } => "insert_section_break",
            HeadlessCommand::InsertEndnote { .. } => "insert_endnote",
            HeadlessCommand::UpdateEndnote { .. } => "update_endnote",
            HeadlessCommand::DeleteEndnote { .. } => "delete_endnote",
            HeadlessCommand::InsertFootnote { .. } => "insert_footnote",
            HeadlessCommand::UpdateFootnote { .. } => "update_footnote",
            HeadlessCommand::DeleteFootnote { .. } => "delete_footnote",
            HeadlessCommand::ConvertFootnoteToEndnote { .. } => "convert_footnote_to_endnote",
            HeadlessCommand::ConvertEndnoteToFootnote { .. } => "convert_endnote_to_footnote",
            HeadlessCommand::AddCitationSource { .. } => "add_citation_source",
            HeadlessCommand::InsertCitation { .. } => "insert_citation",
            HeadlessCommand::InsertBibliography { .. } => "insert_bibliography",
            HeadlessCommand::SetDocumentProtection { .. } => "set_document_protection",
            HeadlessCommand::ApplyBold { .. } => "apply_bold",
            HeadlessCommand::ApplyItalic { .. } => "apply_italic",
            HeadlessCommand::ApplyUnderline { .. } => "apply_underline",
            HeadlessCommand::ApplyStrike { .. } => "apply_strike",
            HeadlessCommand::ApplySuperscript { .. } => "apply_superscript",
            HeadlessCommand::ApplySubscript { .. } => "apply_subscript",
            HeadlessCommand::SetFontSize { .. } => "set_font_size",
            HeadlessCommand::SetFontFamily { .. } => "set_font_family",
            HeadlessCommand::ClearFormatting { .. } => "clear_formatting",
            HeadlessCommand::ChangeCase { .. } => "change_case",
            HeadlessCommand::SetBold { .. } => "set_bold",
            HeadlessCommand::SetParagraphAlignment { .. } => "set_paragraph_alignment",
            HeadlessCommand::SetParagraphDirection { .. } => "set_paragraph_direction",
            HeadlessCommand::SetCharacterStyle { .. } => "set_character_style",
            HeadlessCommand::SetParagraphStyle { .. } => "set_paragraph_style",
            HeadlessCommand::SetParagraphShading { .. } => "set_paragraph_shading",
            HeadlessCommand::SetParagraphBorders { .. } => "set_paragraph_borders",
            HeadlessCommand::SetDropCap { .. } => "set_drop_cap",
            HeadlessCommand::SetParagraphIndent { .. } => "set_paragraph_indent",
            HeadlessCommand::AdjustParagraphIndent { .. } => "adjust_paragraph_indent",
            HeadlessCommand::ToggleParagraphSpaceBefore { .. } => "toggle_paragraph_space_before",
            HeadlessCommand::ResetParagraphFormatting { .. } => "reset_paragraph_formatting",
            HeadlessCommand::AdjustFontSize { .. } => "adjust_font_size",
            HeadlessCommand::SetParagraphMetrics { .. } => "set_paragraph_metrics",
            HeadlessCommand::StartChangeTracking { .. } => "start_change_tracking",
            HeadlessCommand::StopChangeTracking => "stop_change_tracking",
            HeadlessCommand::AcceptRevisions { .. } => "accept_revisions",
            HeadlessCommand::RejectRevisions { .. } => "reject_revisions",
            HeadlessCommand::AddComment { .. } => "add_comment",
            HeadlessCommand::AddCommentByText { .. } => "add_comment_by_text",
            HeadlessCommand::AddCommentByParaId { .. } => "add_comment_by_para_id",
            HeadlessCommand::ReplyToComment { .. } => "reply_to_comment",
            HeadlessCommand::UpdateComment { .. } => "update_comment",
            HeadlessCommand::DeleteComment { .. } => "delete_comment",
            HeadlessCommand::ResolveComment { .. } => "resolve_comment",
            HeadlessCommand::SetHeaderParagraphText { .. } => "set_header_paragraph_text",
            HeadlessCommand::SetFooterParagraphText { .. } => "set_footer_paragraph_text",
            HeadlessCommand::SetHeaderFooterVariants { .. } => "set_header_footer_variants",
            HeadlessCommand::InsertTable { .. } => "insert_table",
            HeadlessCommand::InsertTableOfContents { .. } => "insert_table_of_contents",
            HeadlessCommand::InsertCaption { .. } => "insert_caption",
            HeadlessCommand::InsertField { .. } => "insert_field",
            HeadlessCommand::InsertEquation { .. } => "insert_equation",
            HeadlessCommand::InsertBlankPage { .. } => "insert_blank_page",
            HeadlessCommand::InsertCoverPage { .. } => "insert_cover_page",
            HeadlessCommand::InsertShape { .. } => "insert_shape",
            HeadlessCommand::InsertTextAmongAnchors { .. } => "insert_text_among_anchors",
            HeadlessCommand::DeleteTextBox { .. } => "delete_text_box",
            HeadlessCommand::MarkIndexEntry { .. } => "mark_index_entry",
            HeadlessCommand::InsertIndex { .. } => "insert_index",
            HeadlessCommand::UpdateTableOfContents => "update_table_of_contents",
            HeadlessCommand::SetTableWidth { .. } => "set_table_width",
            HeadlessCommand::SetTableFit { .. } => "set_table_fit",
            HeadlessCommand::SetTableTextWrapping { .. } => "set_table_text_wrapping",
            HeadlessCommand::SetTableColumnWidths { .. } => "set_table_column_widths",
            HeadlessCommand::SplitTable { .. } => "split_table",
            HeadlessCommand::SplitTableCell { .. } => "split_table_cell",
            HeadlessCommand::DistributeTableRows { .. } => "distribute_table_rows",
            HeadlessCommand::DistributeTableColumns { .. } => "distribute_table_columns",
            HeadlessCommand::SetTableAlignment { .. } => "set_table_alignment",
            HeadlessCommand::SetTableCellShading { .. } => "set_table_cell_shading",
            HeadlessCommand::ApplyTableStylePreset { .. } => "apply_table_style_preset",
            HeadlessCommand::ApplyTableNamedStyle { .. } => "apply_table_named_style",
            HeadlessCommand::SetTableLook { .. } => "set_table_look",
            HeadlessCommand::SetTableCellVerticalAlignment { .. } => {
                "set_table_cell_vertical_alignment"
            }
            HeadlessCommand::ResetTableCellMargins { .. } => "reset_table_cell_margins",
            HeadlessCommand::SetTableRowHeightProperties { .. } => {
                "set_table_row_height_properties"
            }
            HeadlessCommand::SetTableCellMargins { .. } => "set_table_cell_margins",
            HeadlessCommand::SetTableBorders { .. } => "set_table_borders",
            HeadlessCommand::SetTableCellBorders { .. } => "set_table_cell_borders",
            HeadlessCommand::SetTableRowProperties { .. } => "set_table_row_properties",
            HeadlessCommand::SetTableCellText { .. } => "set_table_cell_text",
            HeadlessCommand::FillTableRow { .. } => "fill_table_row",
            HeadlessCommand::InsertTableRow { .. } => "insert_table_row",
            HeadlessCommand::CloneTableRow { .. } => "clone_table_row",
            HeadlessCommand::DeleteTableRow { .. } => "delete_table_row",
            HeadlessCommand::InsertTableColumn { .. } => "insert_table_column",
            HeadlessCommand::DeleteTableColumn { .. } => "delete_table_column",
            HeadlessCommand::DeleteTableCell { .. } => "delete_table_cell",
            HeadlessCommand::InsertTableCell { .. } => "insert_table_cell",
            HeadlessCommand::DeleteTable { .. } => "delete_table",
            HeadlessCommand::ResizeTableColumnBetween { .. } => "resize_table_column_between",
            HeadlessCommand::ResizeTableColumnRightEdge { .. } => "resize_table_column_right_edge",
            HeadlessCommand::ResizeTableRowBetween { .. } => "resize_table_row_between",
            HeadlessCommand::ResizeTableRowBottomEdge { .. } => "resize_table_row_bottom_edge",
            HeadlessCommand::MergeTableCells { .. } => "merge_table_cells",
            HeadlessCommand::UnmergeTableCells { .. } => "unmerge_table_cells",
            HeadlessCommand::SetNumbering { .. } => "set_numbering",
            HeadlessCommand::ApplyMultilevelList { .. } => "apply_multilevel_list",
            HeadlessCommand::ApplyCustomMultilevelList { .. } => "apply_custom_multilevel_list",
            HeadlessCommand::SetFontColor { .. } => "set_font_color",
            HeadlessCommand::SetHighlightColor { .. } => "set_highlight_color",
            HeadlessCommand::SetRunFormat { .. } => "set_run_format",
            HeadlessCommand::SetDocumentSetting { .. } => "set_document_setting",
            HeadlessCommand::SetPageOrientation { .. } => "set_page_orientation",
            HeadlessCommand::SetPageSize { .. } => "set_page_size",
            HeadlessCommand::SetPageMargins { .. } => "set_page_margins",
            HeadlessCommand::SetPageColumns { .. } => "set_page_columns",
            HeadlessCommand::SetPageBorders { .. } => "set_page_borders",
            HeadlessCommand::SetWatermark { .. } => "set_watermark",
            HeadlessCommand::SetDocumentTheme { .. } => "set_document_theme",
            HeadlessCommand::SetPageBackgroundColor { .. } => "set_page_background_color",
            HeadlessCommand::SetParagraphLineSpacing { .. } => "set_paragraph_line_spacing",
            HeadlessCommand::InsertAfterHeading { .. } => "insert_after_heading",
            HeadlessCommand::InsertBeforeHeading { .. } => "insert_before_heading",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
// 誤ったフィールド名を黙って既定値で埋めると、呼び出し側は「別の場所が返った」ことに
// 気付けない。名指しでエラーにする。
#[serde(deny_unknown_fields)]
pub enum HeadlessQuery {
    DocumentStats,
    DocumentOutline {
        #[serde(default)]
        start_heading: usize,
        #[serde(default = "default_outline_slice_limit")]
        max_headings: usize,
        #[serde(default)]
        max_level: Option<u8>,
    },
    ParagraphCount,
    ParagraphText {
        paragraph: usize,
    },
    Paragraph {
        paragraph: usize,
    },
    /// Look a paragraph up by its stable `w14:paraId` (`getElementById`-style),
    /// returning its current positional index and snapshot, or `None`.
    ParagraphByParaId {
        para_id: String,
    },
    ParagraphSlice {
        #[serde(default)]
        start_paragraph: usize,
        #[serde(default = "default_paragraph_slice_limit")]
        max_paragraphs: usize,
    },
    SearchText {
        query: String,
        #[serde(default)]
        start_paragraph: usize,
        #[serde(default = "default_search_max_results")]
        max_results: usize,
        #[serde(default = "default_context_before_chars")]
        context_before_chars: usize,
        #[serde(default = "default_context_after_chars")]
        context_after_chars: usize,
        /// Match without regard to ASCII case, so a defined term is found
        /// however the draft capitalises it. Offsets stay byte offsets into
        /// the original text.
        #[serde(default)]
        case_insensitive: bool,
    },
    CharacterStyleAt {
        paragraph: usize,
        offset: usize,
    },
    TableCount,
    Table {
        table_order_index: usize,
    },
    TableRow {
        table_order_index: usize,
        row: usize,
    },
    TableColumn {
        table_order_index: usize,
        column: usize,
    },
    TableCell {
        table_order_index: usize,
        row: usize,
        column: usize,
    },
    Revisions {
        #[serde(default)]
        start_revision: usize,
        #[serde(default = "default_revision_slice_limit")]
        max_revisions: usize,
        #[serde(default)]
        kind: Option<HeadlessRevisionKindFilter>,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        date_from: Option<String>,
        #[serde(default)]
        date_to: Option<String>,
    },
    Comments {
        #[serde(default)]
        start_comment: usize,
        #[serde(default = "default_comment_slice_limit")]
        max_comments: usize,
        /// Only comments by this author (exact match).
        #[serde(default)]
        author: Option<String>,
        /// Only comments in this resolved state — `false` is the review
        /// backlog: what still needs an answer.
        #[serde(default)]
        resolved: Option<bool>,
    },
    CommentThreads {
        #[serde(default)]
        start_thread: usize,
        #[serde(default = "default_comment_slice_limit")]
        max_threads: usize,
    },
    /// The document's style catalogue, so `set_paragraph_style` can be given a
    /// `style_id` that exists.
    Styles {
        /// Only styles of this kind; omit for both.
        #[serde(default)]
        kind: Option<HeadlessStyleKindFilter>,
    },
    /// Page geometry of the default section: size, orientation, margins and
    /// background.
    PageSetup,
    /// Text of every header and footer story in the document.
    HeaderFooterText,
    /// Package metadata (`docProps/core.xml` and friends): title, creator,
    /// last modified by, timestamps, and any custom properties.
    DocumentProperties,
    /// Word bibliography sources stored in the package's `customXml` catalog.
    CitationSources,
    /// Word editing-restriction metadata from `word/settings.xml`.
    DocumentProtection,
    /// Split the document into article-sized chunks and read them.
    ///
    /// Contracts and regulations are written, cited and reviewed one article at
    /// a time; paging by paragraph cuts across that structure, and reading the
    /// whole body at once does not fit. A chunk carries its own number and
    /// title, so the agent knows what it is reading.
    Chunks {
        #[serde(default)]
        start_chunk: usize,
        #[serde(default = "default_chunk_slice_limit")]
        max_chunks: usize,
        /// Let a top-level table stay inside the article it appears in instead
        /// of becoming a chunk of its own.
        #[serde(default)]
        disable_table_splitting: bool,
        /// Give `第N条のM` its own chunk instead of keeping it with `第N条`.
        #[serde(default)]
        enable_branch_splitting: bool,
        /// Return the chunk texts. Turn it off to page through a long document
        /// by structure alone — number, title and paragraph range — which is
        /// far cheaper when the agent is only looking for the right article.
        #[serde(default = "default_true")]
        include_text: bool,
    },
    /// Cross-references in the body text, resolved to the articles they name.
    ///
    /// `第５条第２項`, `前条`, `第３条から第７条まで` — the citations that go
    /// stale as soon as articles are inserted, renumbered or deleted.
    References {
        #[serde(default)]
        start_reference: usize,
        #[serde(default = "default_reference_slice_limit")]
        max_references: usize,
        /// Only references that resolve to nothing: the dangling citations.
        #[serde(default)]
        unresolved_only: bool,
        /// Only references written in this paragraph range.
        #[serde(default)]
        start_paragraph: Option<usize>,
        #[serde(default)]
        end_paragraph: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessStyleKindFilter {
    Paragraph,
    Character,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HeadlessQueryResult {
    DocumentStats {
        stats: HeadlessDocumentStats,
    },
    DocumentOutline {
        start_heading: usize,
        headings: Vec<HeadlessOutlineItem>,
        has_more: bool,
        total_headings: usize,
    },
    ParagraphCount {
        paragraph_count: usize,
    },
    ParagraphText {
        paragraph: usize,
        text: Option<String>,
        #[serde(default)]
        numbering_text: Option<String>,
    },
    Paragraph {
        paragraph: usize,
        value: Option<HeadlessParagraphSnapshot>,
    },
    ParagraphByParaId {
        para_id: String,
        /// The paragraph's current positional index, or `None` if no paragraph
        /// carries this `para_id`.
        paragraph: Option<usize>,
        value: Option<HeadlessParagraphSnapshot>,
    },
    ParagraphSlice {
        start_paragraph: usize,
        paragraphs: Vec<HeadlessParagraphText>,
        has_more: bool,
    },
    SearchText {
        query: String,
        matches: Vec<HeadlessSearchMatch>,
        truncated: bool,
    },
    CharacterStyleAt {
        paragraph: usize,
        offset: usize,
        value: Option<HeadlessCharacterStyleSnapshot>,
    },
    TableCount {
        table_count: usize,
    },
    Table {
        table_order_index: usize,
        value: Option<HeadlessTableSnapshot>,
    },
    TableRow {
        table_order_index: usize,
        row: usize,
        value: Option<HeadlessTableRowSnapshot>,
    },
    TableColumn {
        table_order_index: usize,
        column: usize,
        value: Option<HeadlessTableColumnDetailSnapshot>,
    },
    TableCell {
        table_order_index: usize,
        row: usize,
        column: usize,
        value: Option<HeadlessTableCellSnapshot>,
    },
    Revisions {
        start_revision: usize,
        revisions: Vec<HeadlessRevisionSnapshot>,
        has_more: bool,
        total_revisions: usize,
    },
    Comments {
        start_comment: usize,
        comments: Vec<HeadlessCommentSnapshot>,
        has_more: bool,
        total_comments: usize,
    },
    CommentThreads {
        start_thread: usize,
        threads: Vec<HeadlessCommentThreadSnapshot>,
        has_more: bool,
        total_threads: usize,
    },
    Styles {
        styles: Vec<HeadlessStyleSnapshot>,
    },
    PageSetup {
        page_setup: HeadlessPageSetupSnapshot,
    },
    HeaderFooterText {
        stories: Vec<HeadlessHeaderFooterSnapshot>,
    },
    DocumentProperties {
        /// Property name → value, sorted by name. Names are the ones the
        /// package uses (`title`, `creator`, `lastModifiedBy`, `created`, …).
        properties: BTreeMap<String, String>,
    },
    CitationSources {
        sources: Vec<HeadlessCitationSource>,
    },
    DocumentProtection {
        protection: Option<HeadlessDocumentProtection>,
    },
    Chunks {
        start_chunk: usize,
        chunks: Vec<HeadlessChunkSnapshot>,
        has_more: bool,
        total_chunks: usize,
    },
    References {
        start_reference: usize,
        references: Vec<HeadlessReferenceSnapshot>,
        has_more: bool,
        total_references: usize,
    },
}

fn default_delete_count() -> usize {
    1
}

fn default_break_count() -> usize {
    1
}

fn default_paragraph_slice_limit() -> usize {
    20
}

fn default_search_max_results() -> usize {
    20
}

fn default_context_before_chars() -> usize {
    24
}

fn default_context_after_chars() -> usize {
    24
}

fn default_comment_slice_limit() -> usize {
    50
}

fn default_revision_slice_limit() -> usize {
    100
}

fn default_outline_slice_limit() -> usize {
    100
}

fn default_chunk_slice_limit() -> usize {
    20
}

fn default_reference_slice_limit() -> usize {
    100
}

fn default_true() -> bool {
    true
}

fn default_toc_outline_min() -> u8 {
    1
}

fn default_toc_outline_max() -> u8 {
    3
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessDeleteDirection {
    #[default]
    Backward,
    Forward,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessSectionBreakType {
    NextPage,
    Continuous,
    EvenPage,
    OddPage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_defaults_high_level_commands_and_dry_run_contract() {
        let request: HeadlessApplyAndQueryRequest = serde_json::from_str(
            r##"{
              "include_diff": true,
              "commands": [
                {"type":"clone_table_row","table_order_index":0,"source_row":1,"target_row":2},
                {"type":"insert_after_heading","heading":"請求明細","text":"body"},
                {"type":"replace_placeholder","placeholder":"${name}","text":"Alice"},
                {"type":"set_header_footer_variants","different_first_page":true,"different_odd_even_pages":false},
                {"type":"set_page_borders","clear":false,"top":{"style":"single","width":0.75,"color":"#336699"},"offset_from":"text"},
                {"type":"set_watermark","clear":false,"text":"社外秘"}
              ],
              "queries": []
            }"##,
        )
        .expect("request should deserialize");

        assert!(request.include_diff);
        assert!(!request.dry_run, "dry_run defaults to false");
        match &request.commands[0] {
            HeadlessCommand::CloneTableRow { clear_text, .. } => assert!(clear_text),
            other => panic!("unexpected command: {other:?}"),
        }
        match &request.commands[1] {
            HeadlessCommand::InsertAfterHeading {
                level, occurrence, ..
            } => {
                assert_eq!(level, &None);
                assert_eq!(occurrence, &0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        match &request.commands[2] {
            HeadlessCommand::ReplacePlaceholder {
                max_replacements, ..
            } => assert_eq!(max_replacements, &None),
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            &request.commands[3],
            HeadlessCommand::SetHeaderFooterVariants {
                different_first_page: true,
                different_odd_even_pages: false,
                section: None,
            }
        ));
        assert!(matches!(
            &request.commands[4],
            HeadlessCommand::SetPageBorders {
                clear: false,
                top: Some(HeadlessBorderEdge { .. }),
                offset_from: Some(value),
                ..
            } if value == "text"
        ));
        assert!(matches!(
            &request.commands[5],
            HeadlessCommand::SetWatermark {
                clear: false,
                text: Some(value),
            } if value == "社外秘"
        ));

        assert!(
            serde_json::from_str::<HeadlessCommand>(
                r#"{"type":"set_page_borders","top":{"style":"single"}}"#
            )
            .is_err(),
            "clear is required at the wire boundary"
        );
        assert!(
            serde_json::from_str::<HeadlessCommand>(
                r#"{"type":"set_page_borders","clear":true,"offsetFrom":"text"}"#
            )
            .is_err(),
            "camelCase typo must not be ignored"
        );
        assert!(
            serde_json::from_str::<HeadlessCommand>(
                r#"{"type":"set_page_borders","clear":false,"top":{"widht":1}}"#
            )
            .is_err(),
            "nested edge typos must not fall back to border defaults"
        );
        assert!(
            serde_json::from_str::<HeadlessCommand>(r#"{"type":"set_watermark","text":"draft"}"#)
                .is_err(),
            "watermark clear intent is required at the wire boundary"
        );
        assert!(
            serde_json::from_str::<HeadlessCommand>(
                r#"{"type":"set_watermark","clear":false,"watermak_text":"draft"}"#
            )
            .is_err(),
            "watermark field typos must not be ignored"
        );

        let diff = HeadlessDocumentDiff {
            before: HeadlessDocumentStats::default(),
            after: HeadlessDocumentStats::default(),
            changed_paragraphs: vec![0],
            changed_runs: vec![HeadlessRunDiff {
                paragraph: 0,
                before_run_count: 3,
                after_run_count: 2,
                changed_run_indices: vec![1],
            }],
            changed_tables: vec![HeadlessTableDiff {
                table_order_index: 0,
                before_rows: 2,
                after_rows: 3,
                before_columns: 2,
                after_columns: 2,
                changed_cells: vec![HeadlessTableCellDiff { row: 2, column: 1 }],
            }],
            section_changed: true,
        };
        let roundtrip: HeadlessDocumentDiff =
            serde_json::from_str(&serde_json::to_string(&diff).expect("serialize diff"))
                .expect("deserialize diff");
        assert_eq!(roundtrip, diff);
    }

    #[test]
    fn cover_page_wire_schema_requires_a_strict_paragraph_list() {
        let command = serde_json::from_str::<HeadlessCommand>(
            r##"{
                "type":"insert_cover_page",
                "paragraphs":[{
                    "text":"文書タイトル",
                    "bold":true,
                    "color":"#1F3864",
                    "font_size":36,
                    "alignment":"center",
                    "space_before":180,
                    "space_after":12,
                    "shading_color":"#FFFFFF",
                    "border_top":true,
                    "indent_left":36
                }]
            }"##,
        )
        .expect("parse cover page");
        assert!(matches!(
            command,
            HeadlessCommand::InsertCoverPage { paragraphs }
                if paragraphs.len() == 1
                    && paragraphs[0].text == "文書タイトル"
                    && paragraphs[0].bold
                    && paragraphs[0].font_size == Some(36.0)
                    && paragraphs[0].border_top
        ));

        for invalid in [
            r#"{"type":"insert_cover_page"}"#,
            r#"{"type":"insert_cover_page","paragraphs":{},"unexpected":true}"#,
            r#"{"type":"insert_cover_page","paragraphs":[{"text":"title","fontSize":36}]}"#,
            r#"{"type":"insert_cover_page","paragraphs":[{"text":"title","border_top":true,"border_typo":true}]}"#,
        ] {
            assert!(
                serde_json::from_str::<HeadlessCommand>(invalid).is_err(),
                "invalid cover wire input was accepted: {invalid}"
            );
        }
    }
}

#[cfg(test)]
mod ai_friendly_tests {
    use super::*;

    /// 誤ったフィールド名を黙って受け入れないこと。
    ///
    /// `paragraph_slice` の引数は `start_paragraph` / `max_paragraphs` だが、
    /// 直感的に `start` / `end` と書いてしまう。`#[serde(default)]` だけだと
    /// **エラーにならず既定値（先頭から）で返る**ため、呼び出し側は
    /// 「別の場所が返ってきた」ことに気付けない。実際に MCP 経由で踏んだ。
    #[test]
    fn paragraph_slice_rejects_unknown_fields() {
        let err = serde_json::from_str::<HeadlessQuery>(
            r#"{"type":"paragraph_slice","start":32,"end":35}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("start"),
            "unknown field を名指しで報告すること: {err}"
        );
    }

    #[test]
    fn paragraph_slice_accepts_documented_fields() {
        let q: HeadlessQuery = serde_json::from_str(
            r#"{"type":"paragraph_slice","start_paragraph":32,"max_paragraphs":3}"#,
        )
        .unwrap();
        assert_eq!(
            q,
            HeadlessQuery::ParagraphSlice {
                start_paragraph: 32,
                max_paragraphs: 3
            }
        );
    }

    /// 段落全体にコメントしたいだけのとき、オフセットまで求めない。
    ///
    /// 現状は start/end の paragraph と offset を 4 つとも必須にしており、
    /// 「この段落にコメント」を表現できない。しかもエラーが 1 つずつしか出ないため、
    /// 順に踏むと 3 回失敗してようやく通る。
    #[test]
    fn add_comment_offsets_are_optional() {
        let c: HeadlessCommand = serde_json::from_str(
            r#"{"type":"add_comment","start_paragraph":33,"end_paragraph":33,"text":"note"}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            HeadlessCommand::AddComment {
                start_paragraph: 33,
                start_offset: None,
                end_paragraph: 33,
                end_offset: None,
                text: "note".into(),
                author: None,
                initials: None,
                date: None,
            }
        );
    }

    /// 検索した文字列にそのままコメントを付けられること。
    ///
    /// AI が実際にやりたいのは「"満５年間" にコメント」であって、
    /// オフセットの計算ではない。search_text と add_comment を人手でつなぐ必要をなくす。
    #[test]
    fn add_comment_by_text_parses() {
        let c: HeadlessCommand = serde_json::from_str(
            r#"{"type":"add_comment_by_text","query":"満５年間","text":"確認してください"}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            HeadlessCommand::AddCommentByText {
                query: "満５年間".into(),
                occurrence: None,
                text: "確認してください".into(),
                author: None,
                initials: None,
                date: None,
            }
        );
    }

    /// 相手方の指摘に返信できること。
    ///
    /// レビューは往復なので、指摘とその答えは同じスレッドに残らないと読めない。
    #[test]
    fn reply_to_comment_parses() {
        let c: HeadlessCommand = serde_json::from_str(
            r#"{"type":"reply_to_comment","comment_id":"3","text":"承知しました","author":"法務"}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            HeadlessCommand::ReplyToComment {
                comment_id: "3".into(),
                text: "承知しました".into(),
                author: Some("法務".into()),
                initials: None,
                date: None,
            }
        );
    }

    #[test]
    fn ribbon_two_uses_the_drawingml_public_name() {
        let command: HeadlessCommand = serde_json::from_str(
            r#"{"type":"insert_shape","paragraph":0,"offset":0,"shape":"ribbon2","width_px":200,"height_px":120,"border_width_px":1}"#,
        )
        .unwrap();
        assert!(matches!(
            command,
            HeadlessCommand::InsertShape {
                shape: HeadlessShapeKind::RibbonTwo,
                ..
            }
        ));
        assert_eq!(
            serde_json::to_string(&HeadlessShapeKind::RibbonTwo).unwrap(),
            r#""ribbon2""#
        );

        let legacy: HeadlessShapeKind = serde_json::from_str(r#""ribbonTwo""#).unwrap();
        assert_eq!(legacy, HeadlessShapeKind::RibbonTwo);
    }

    #[test]
    fn line_and_callout_kinds_use_genoffice_public_names() {
        for (json, expected) in [
            (r#""lineArrowDouble""#, HeadlessShapeKind::LineArrowDouble),
            (
                r#""wedgeRectCallout""#,
                HeadlessShapeKind::WedgeRectangleCallout,
            ),
            (
                r#""wedgeRoundRectCallout""#,
                HeadlessShapeKind::WedgeRoundedRectangleCallout,
            ),
            (r#""cloudCallout""#, HeadlessShapeKind::CloudCallout),
        ] {
            let parsed: HeadlessShapeKind = serde_json::from_str(json).expect("public shape");
            assert_eq!(parsed, expected);
            assert_eq!(
                serde_json::to_string(&parsed).expect("serialize shape"),
                json
            );
        }
    }
}
