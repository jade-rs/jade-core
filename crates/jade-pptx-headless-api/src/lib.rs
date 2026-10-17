//! The shapes a caller with no UI sees when it asks about a deck.
//!
//! Two views of a presentation live here. What the deck *says* — slide
//! order, titles, shape names, their text, table cells, speaker notes — and
//! what it would *draw*: the px geometry the layout engine resolves, so an
//! agent can tell whether a title overflows its box without rendering
//! anything. Deliberately a separate crate, as on the DOCX and XLSX sides:
//! these types travel across a process boundary (a CLI's stdout, an MCP
//! response), and a consumer should be able to depend on them without
//! pulling in a presentation engine.

use serde::{Deserialize, Serialize};

/// What the deck holds, without reading any slide deeply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeckSummary {
    pub slide_count: usize,
    /// Slide dimensions at 96 dpi.
    pub width_px: f64,
    pub height_px: f64,
    pub slides: Vec<SlideSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideSummary {
    /// 1-based, the number a presenter would say.
    pub index: usize,
    pub part_name: String,
    /// The title placeholder's text, when the slide has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Top-level shapes; grouped children count through their group.
    pub shape_count: usize,
    /// Whether the slide carries speaker notes.
    #[serde(default)]
    pub has_notes: bool,
    /// `show="0"`: skipped in a slide show.
    #[serde(default)]
    pub hidden: bool,
}

/// One slide's shapes, in z-order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideView {
    pub index: usize,
    pub shapes: Vec<ShapeView>,
    /// Speaker notes, paragraphs joined with newlines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Shape,
    Picture,
    Group,
    Table,
    Chart,
}

/// What a chart plots, as cached in its part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartView {
    /// `barChart`, `lineChart`, `pieChart`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub series: Vec<ChartSeriesView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSeriesView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub categories: Vec<String>,
    /// One per category; `null` where the cache has no point.
    pub values: Vec<Option<f64>>,
}

/// A rectangle in slide px at 96 dpi, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RectPx {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeView {
    pub kind: ShapeKind,
    /// The `cNvPr id`, unique within the slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    /// The placeholder this shape fills, as `kind` or `kind#idx`
    /// (`title`, `body#1`, `#1`), when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// The preset geometry name, e.g. `rect`, when the shape declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<String>,
    /// Where the shape lands in slide px, including geometry a placeholder
    /// inherits from its layout or master. `None` when nothing places it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<RectPx>,
    /// The shape's text, paragraphs joined with newlines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A table's cells, row by row; merged-away cells are empty strings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<String>>,
    /// The package part a picture draws, e.g. `ppt/media/image1.png`; pass
    /// it to the media query to get the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_part: Option<String>,
    /// A chart's plot kind, title and series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<ChartView>,
    /// Every hyperlink target in the shape's text, in order, deduplicated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hyperlinks: Vec<String>,
    /// A group's children, in z-order; empty for everything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ShapeView>,
}

/// One shape whose text contains a searched-for needle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextMatch {
    /// 1-based slide number.
    pub slide: usize,
    /// The matching shape's name; a table cell is `Table!B2`, speaker notes
    /// are `Notes`.
    pub shape: String,
    /// The shape's full text.
    pub text: String,
    /// The match is in the slide's speaker notes rather than on the slide.
    #[serde(default)]
    pub in_notes: bool,
}

/// What the layout engine would draw for one slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideLayoutView {
    pub index: usize,
    pub width_px: f64,
    pub height_px: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// In paint order: master shapes, layout shapes, then the slide's own.
    pub shapes: Vec<PlacedShapeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedShapeView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// `slide`, `layout` or `master`.
    pub origin: String,
    pub rect: RectPx,
    /// Geometry text bounds in local text-frame pixels, before body insets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_region: Option<RectPx>,
    /// Clockwise degrees.
    #[serde(default)]
    pub rotation_deg: f64,
    /// `rect`, `round_rect`, `ellipse`, `line` or `polygon`.
    pub geometry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<OutlineView>,
    /// The text as laid out, one entry per line, bullets included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// How far the text runs past the text region's bottom edge; 0 when it fits.
    #[serde(default)]
    pub text_overflow_px: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineView {
    pub color: String,
    pub width_px: f64,
    #[serde(default)]
    pub dashed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// The text needs more height than its shape has.
    TextOverflow,
    /// The shape lies wholly outside the slide.
    OutsideSlide,
    /// Unwrapped text runs past the shape's right edge.
    TextTooWide,
}

/// Something a presenter would notice on a slide: text spilling out of its
/// box, a shape off the slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutIssue {
    /// 1-based slide number.
    pub slide: usize,
    pub shape: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub kind: IssueKind,
    /// Readable, e.g. "text runs 12.3 px past the bottom of the shape".
    pub detail: String,
    /// The overflow or the distance off-slide, in px.
    pub amount_px: f64,
}
