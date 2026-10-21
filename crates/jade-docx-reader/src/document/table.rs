use super::table_conditional::{
    table_style_condition_matches, TableLook, TableStyleBandSizes, TableStyleCellFormat,
    TableStyleCellPosition, TableStyleCondition, TableStyleConditionalFormat,
};
use super::{BodyElement, BorderEdge};
use crate::docx_xml::XmlNodeId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, str::FromStr};

/// Map of CSS-style properties collected while parsing. The keys follow the
/// naming used by the original docxjs implementation to stay compatible with
/// its rendering behaviour.
pub type StyleMap = BTreeMap<String, String>;

/// Conditional first-row formatting contributed by a package-local table
/// style. These values are display-only; the source remains in styles.xml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableStyleFirstRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Resolved display properties for a `w:style w:type="table"`, including its
/// `w:basedOn` chain. Direct table/cell formatting remains authoritative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableStyleDisplay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_row: Option<TableStyleFirstRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band1_fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band2_fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<TableBorders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_margins: Option<TableCellMargins>,
    /// Every applied `w:tblStylePr` (except `wholeTable`, which Word ignores),
    /// after `w:basedOn` inheritance. The renderer layers them per cell in
    /// Word's order; see [`Table::table_style_cell_format`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub conditional: BTreeMap<TableStyleCondition, TableStyleConditionalFormat>,
    /// The style's `w:tblStyleRowBandSize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_band_size: Option<u32>,
    /// The style's `w:tblStyleColBandSize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_band_size: Option<u32>,
}

/// Represents a table at the block level. The structure intentionally mirrors
/// the data extracted by docxjs so that style and layout handling can be ported
/// feature-by-feature.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Table {
    pub rows: Vec<TableRow>,
    pub columns: Vec<TableColumn>,
    pub style: StyleMap,
    pub cell_style: StyleMap,
    pub style_name: Option<String>,
    pub class_name: Option<String>,
    pub col_band_size: Option<u32>,
    pub row_band_size: Option<u32>,
    /// `w:tblLook` switches; unspecified ones follow Word's `0x04A0` default.
    #[serde(default, skip_serializing_if = "TableLook::is_unspecified")]
    pub look: TableLook,
    /// Display-only package style resolved after styles.xml is available.
    #[serde(skip)]
    pub resolved_style: Option<TableStyleDisplay>,
    /// Table-level default borders (`w:tblBorders`). Applied to cells that do
    /// not define their own `w:tcBorders`, with outer edges used at the table
    /// boundary and `inside_h`/`inside_v` between cells. See ECMA-376 Part 1
    /// §17.4.39 (<https://ooxml.info/docs/11/Part1/17.4.39>).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borders: Option<TableBorders>,
    /// Table-level default cell margins (`w:tblCellMar`). Applied to cells that
    /// do not define their own `w:tcMar`. Values are in points. See ECMA-376
    /// Part 1 §17.4.43 (<https://ooxml.info/docs/11/Part1/17.4.43>).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_margins: Option<TableCellMargins>,
    /// Default distance between adjacent cells and between cells and the table
    /// edges (`w:tblCellSpacing`), in points. It is laid out inside the table
    /// extents and therefore must not increase the overall grid width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_spacing: Option<f32>,
    /// Layout algorithm (`w:tblLayout`). `None`/absent is treated as
    /// [`TableLayoutMode::Autofit`] per ECMA-376 Part 1 §17.4.52.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_layout: Option<TableLayoutMode>,
    /// `w:bidiVisual` mirrors the table grid so the first logical cell is drawn
    /// at the visual right edge. It does not by itself change paragraph bidi or
    /// the logical row/cell order used by editing and selection APIs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bidi_visual: bool,
    #[serde(skip_serializing, skip_deserializing)]
    pub node_id: Option<XmlNodeId>,
}

impl Table {
    /// Band sizes for conditional formatting: a direct table value wins over
    /// the style, and an omitted size is Word's `0` (no banding).
    pub fn table_style_band_sizes(&self) -> TableStyleBandSizes {
        let style = self.resolved_style.as_ref();
        TableStyleBandSizes {
            rows: self
                .row_band_size
                .or_else(|| style.and_then(|style| style.row_band_size))
                .unwrap_or(0),
            columns: self
                .col_band_size
                .or_else(|| style.and_then(|style| style.col_band_size))
                .unwrap_or(0),
        }
    }

    /// The table style's conditional formatting for one cell, layered in
    /// Word's order and gated by `w:tblLook`. Direct cell, row-exception and
    /// table formatting remain the caller's responsibility to apply on top.
    pub fn table_style_cell_format(
        &self,
        position: TableStyleCellPosition,
    ) -> TableStyleCellFormat {
        let Some(style) = self.resolved_style.as_ref() else {
            return TableStyleCellFormat::default();
        };
        if style.conditional.is_empty() {
            return TableStyleCellFormat::default();
        }
        let matches =
            table_style_condition_matches(&self.look, self.table_style_band_sizes(), position);
        TableStyleCellFormat::resolve(
            matches.into_iter().filter_map(|matched| {
                style
                    .conditional
                    .get(&matched.condition)
                    .map(|format| (format, matched.edges))
            }),
            self.bidi_visual,
        )
    }

    /// Returns the effective table-level cell margins after applying the
    /// direct-formatting-over-style precedence shared by layout and editing.
    pub fn effective_cell_margins(&self) -> Option<&TableCellMargins> {
        self.cell_margins.as_ref().or_else(|| {
            self.resolved_style
                .as_ref()
                .and_then(|style| style.cell_margins.as_ref())
        })
    }
}

/// Cell margins (`w:tblCellMar` at the table level, `w:tcMar` at the cell
/// level). Each edge is an inner padding measured in points. Absent edges fall
/// back through a row-scoped `w:tblPrEx` exception, the table default, and then
/// the renderer's built-in padding. See ECMA-376 Part 1 §17.4.43 / §17.4.42.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableCellMargins {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<f32>,
    /// Logical leading edge (`w:start`). Resolved to left for ordinary tables
    /// and right when `w:bidiVisual` mirrors the table grid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f32>,
    /// Logical trailing edge (`w:end`), the counterpart of `start`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f32>,
}

/// Word-compatible fallback used when no `w:tcMar`, row `w:tblPrEx`, table
/// `w:tblCellMar`, or default table style supplies a vertical cell edge.
///
/// Word's built-in "Normal Table" style declares `w:tblCellMar` as
/// 0 / 108 / 0 / 108 dxa, so a cell without any margin source has no vertical
/// padding: an empty 10.5pt row in a Word-inserted table is one paragraph line
/// tall, and an `hRule="exact"` row is exactly its declared `w:trHeight`.
pub const DEFAULT_VERTICAL_CELL_MARGIN_PT: f32 = 0.0;

/// Resolves one cell-margin edge without applying a renderer-specific unit.
///
/// Keeping the OOXML precedence with the document model lets layout and
/// editing agree about the effective padding: a cell edge wins over the row
/// exception, which wins over the table default. The caller remains
/// responsible for its fallback value.
pub fn resolve_cell_margin_edge_points(
    cell_margins: Option<&TableCellMargins>,
    row_margins: Option<&TableCellMargins>,
    table_margins: Option<&TableCellMargins>,
    edge: impl Fn(&TableCellMargins) -> Option<f32>,
) -> Option<f32> {
    cell_margins
        .and_then(&edge)
        .or_else(|| row_margins.and_then(&edge))
        .or_else(|| table_margins.and_then(edge))
}

impl TableCellMargins {
    /// Returns true when no edge carries any information.
    pub fn is_empty(&self) -> bool {
        self.top.is_none()
            && self.bottom.is_none()
            && self.left.is_none()
            && self.right.is_none()
            && self.start.is_none()
            && self.end.is_none()
    }

    /// Resolves the physical left edge after applying the table's visual
    /// direction. Logical values take precedence over legacy physical names.
    pub fn physical_left(&self, bidi_visual: bool) -> Option<f32> {
        if bidi_visual {
            self.end.or(self.left)
        } else {
            self.start.or(self.left)
        }
    }

    /// Resolves the physical right edge after applying the table's visual
    /// direction. See [`Self::physical_left`].
    pub fn physical_right(&self, bidi_visual: bool) -> Option<f32> {
        if bidi_visual {
            self.start.or(self.right)
        } else {
            self.end.or(self.right)
        }
    }

    /// Sets the physical left (`left == true`) or right padding. See
    /// [`TableBorders::set_physical`] for why logical names are kept.
    pub fn set_physical(&mut self, left: bool, bidi_visual: bool, value: f32) {
        let logical = self.start.is_some() || self.end.is_some();
        // The physical left is the leading edge unless the table is mirrored.
        let leading = left != bidi_visual;
        match (logical, leading) {
            (true, true) => self.start = Some(value),
            (true, false) => self.end = Some(value),
            (false, _) if left => self.left = Some(value),
            (false, _) => self.right = Some(value),
        }
        if logical {
            self.left = None;
            self.right = None;
        }
    }
}

/// Table-level default borders (`w:tblBorders`). The four outer edges plus the
/// interior horizontal (`w:insideH`) and vertical (`w:insideV`) grid lines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<BorderEdge>,
    /// Logical leading outer edge (`w:start`, Office 2010+). When present it
    /// takes precedence over the legacy physical left/right edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<BorderEdge>,
    /// Logical trailing outer edge (`w:end`, Office 2010+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_h: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_v: Option<BorderEdge>,
}

impl TableBorders {
    /// Returns true when no edge carries any information.
    pub fn is_empty(&self) -> bool {
        self.top.is_none()
            && self.bottom.is_none()
            && self.left.is_none()
            && self.right.is_none()
            && self.start.is_none()
            && self.end.is_none()
            && self.inside_h.is_none()
            && self.inside_v.is_none()
    }

    pub fn physical_left(&self, bidi_visual: bool) -> Option<&BorderEdge> {
        if bidi_visual {
            self.end.as_ref().or(self.left.as_ref())
        } else {
            self.start.as_ref().or(self.left.as_ref())
        }
    }

    pub fn physical_right(&self, bidi_visual: bool) -> Option<&BorderEdge> {
        if bidi_visual {
            self.start.as_ref().or(self.right.as_ref())
        } else {
            self.end.as_ref().or(self.right.as_ref())
        }
    }

    /// Sets the physical left (`left == true`) or right outer edge. A table
    /// that names its sides logically (`w:start`/`w:end`) keeps doing so: the
    /// schema pairs `w:start` with `w:left` and `w:end` with `w:right` as
    /// alternatives, so adding `w:left` beside an existing `w:start` made Word
    /// repair the file. The physical side maps to the logical one through
    /// `w:bidiVisual`.
    pub fn set_physical(&mut self, left: bool, bidi_visual: bool, edge: BorderEdge) {
        let logical = self.start.is_some() || self.end.is_some();
        // The physical left is the leading edge unless the table is mirrored.
        let leading = left != bidi_visual;
        match (logical, leading) {
            (true, true) => self.start = Some(edge),
            (true, false) => self.end = Some(edge),
            (false, _) if left => self.left = Some(edge),
            (false, _) => self.right = Some(edge),
        }
        if logical {
            self.left = None;
            self.right = None;
        }
    }
}

/// Column information extracted from the optional `tblGrid` definition.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableColumn {
    pub width: Option<f32>,
}

/// Table layout algorithm (`w:tblLayout` `w:type`, ECMA-376 Part 1 §17.4.52
/// ST_TblLayoutType, <https://ooxml.info/docs/11/Part1/17.4.52>). `Fixed` makes
/// the `w:tblGrid` column widths authoritative and lets a table wider than the
/// page overflow on the right (Word's behaviour for fixed-width tables).
/// `Autofit` — the algorithm Word assumes when the element is absent — scales an
/// over-wide grid down proportionally so the table fits the available content
/// width.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TableLayoutMode {
    /// Fit the table to the available width (default when `w:tblLayout` is
    /// absent).
    #[default]
    Autofit,
    /// Honor the grid widths verbatim and allow page overflow.
    Fixed,
}

impl TableLayoutMode {
    /// Resolves the `w:type` token. Any value other than `fixed` (including the
    /// explicit `autofit`) maps to [`TableLayoutMode::Autofit`].
    pub fn from_type_token(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "fixed" => Self::Fixed,
            _ => Self::Autofit,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TableRowHeightRule {
    #[default]
    Auto,
    AtLeast,
    Exact,
}

/// A single table row along with its styling metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    pub style: StyleMap,
    pub class_name: Option<String>,
    pub is_header: bool,
    /// `w:cantSplit` (ECMA-376 Part 1 §17.4.6): when true the row must stay
    /// together on a single page rather than splitting across a page boundary.
    pub cant_split: bool,
    pub grid_before: Option<u32>,
    pub grid_after: Option<u32>,
    /// Preferred total width of the grid columns omitted before this row's
    /// first cell (`w:wBefore`). Stored in the same CSS-like representation as
    /// `tcW` so absolute and percentage constraints share renderer parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_before: Option<String>,
    /// Preferred total width of the `grid_after` columns following this row's
    /// last cell (`w:wAfter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_after: Option<String>,
    /// Direct `w:trPr/w:tblCellSpacing`, in points. It takes precedence over
    /// the table-property exception and table default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_spacing: Option<f32>,
    /// `w:tblPrEx/w:tblCellSpacing`, in points. Unlike direct row spacing,
    /// its spacing belongs outside the text margins (§17.4.44).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_spacing_exception: Option<f32>,
    /// Row-scoped cell margin exception (`w:tblPrEx/w:tblCellMar`). Each edge
    /// overrides the table default for cells in this row, while a cell's own
    /// `w:tcMar` remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_margins: Option<TableCellMargins>,
    /// Row-scoped table border exception (`w:tblPrEx/w:tblBorders`). For cells
    /// in this row it replaces the table-wide border set, while an individual
    /// cell's `w:tcBorders` remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borders: Option<Box<TableBorders>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_rule: Option<TableRowHeightRule>,
    #[serde(skip_serializing, skip_deserializing)]
    pub node_id: Option<XmlNodeId>,
}

/// A single table cell. Cells can contain arbitrary block elements, allowing
/// nested tables.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextDirection {
    LrTb,
    LrTbV,
    TbRl,
    TbRlV,
    BtLr,
    BtLrV,
    /// Vertical text with its lines running left to right (`tbLrV`).
    TbLrV,
}

impl TextDirection {
    pub fn try_from_str(value: &str) -> Option<Self> {
        match value {
            "lrTb" => Some(TextDirection::LrTb),
            "lrTbV" => Some(TextDirection::LrTbV),
            "tbRl" => Some(TextDirection::TbRl),
            "tbRlV" => Some(TextDirection::TbRlV),
            "btLr" => Some(TextDirection::BtLr),
            "btLrV" => Some(TextDirection::BtLrV),
            "tbLrV" => Some(TextDirection::TbLrV),
            _ => None,
        }
    }
}

impl FromStr for TextDirection {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        TextDirection::try_from_str(value).ok_or(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableCell {
    pub blocks: Vec<BodyElement>,
    pub style: StyleMap,
    pub class_name: Option<String>,
    pub span: Option<u32>,
    pub vertical_merge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_direction: Option<TextDirection>,
    /// Cell-level margin overrides (`w:tcMar`). When present, each edge takes
    /// precedence over the table-level `w:tblCellMar`. Values are in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margins: Option<TableCellMargins>,
    /// Vertical alignment of the cell's content within its cell box
    /// (`w:vAlign`, ECMA-376 Part 1 §17.4.84). Holds the normalized lowercase
    /// token (`top`/`center`/`bottom`). Absent means `top` (Word's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub node_id: Option<XmlNodeId>,
}
