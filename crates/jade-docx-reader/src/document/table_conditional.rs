//! Table style conditional formatting (`w:tblStylePr`, ECMA-376 Part 1
//! §17.7.6) and the `w:tblLook` switches that gate it (§17.4.55).
//!
//! Word deviates from the standard in several documented ways, and those
//! deviations are what Jade follows (MS-OI29500):
//!
//! - §17.4.55(a): an omitted `w:tblLook` means mask `0x04A0` (header row, first
//!   column, no vertical banding), not `0x0000`.
//! - §17.4.55(c): the hexadecimal `w:val` is read only when none of the
//!   explicit on/off attributes are present.
//! - §17.7.6.6(a): conditional formats apply in the order row banding, column
//!   banding, first/last column, first/last row, corner cells. Later formats
//!   override earlier ones.
//! - §17.7.6.7 (2.1.251): an omitted `w:tblStyleRowBandSize` /
//!   `w:tblStyleColBandSize` is `0`, and a band size of `0` disables banding.
//! - §17.18.89(a): a `wholeTable` `w:tblStylePr` is ignored.
//! - §17.4.55(b): corner cells (`nwCell`, ...) apply only when both the row
//!   and column region that meet at the corner are enabled.
//!
//! The model is display-only. The source of truth for export stays in
//! `styles.xml` and the raw `w:tblLook` element.

use super::{BorderEdge, TableBorders};
use serde::{Deserialize, Serialize};

/// A `w:tblStylePr/@w:type` that Word applies. `wholeTable` is intentionally
/// absent: Word discards it (MS-OI29500 §17.18.89(a)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableStyleCondition {
    Band1Horz,
    Band2Horz,
    Band1Vert,
    Band2Vert,
    FirstCol,
    LastCol,
    FirstRow,
    LastRow,
    NwCell,
    NeCell,
    SwCell,
    SeCell,
}

impl TableStyleCondition {
    /// Word's application order (MS-OI29500 §17.7.6.6(a)). A format later in
    /// this list overrides the properties of an earlier one.
    pub const OFFICE_ORDER: [Self; 12] = [
        Self::Band1Horz,
        Self::Band2Horz,
        Self::Band1Vert,
        Self::Band2Vert,
        Self::FirstCol,
        Self::LastCol,
        Self::FirstRow,
        Self::LastRow,
        Self::NwCell,
        Self::NeCell,
        Self::SwCell,
        Self::SeCell,
    ];

    /// Parses `ST_TblStyleOverrideType`. Returns `None` for `wholeTable` and
    /// unknown tokens, which Word does not apply.
    pub fn from_type_token(value: &str) -> Option<Self> {
        Some(match value {
            "band1Horz" => Self::Band1Horz,
            "band2Horz" => Self::Band2Horz,
            "band1Vert" => Self::Band1Vert,
            "band2Vert" => Self::Band2Vert,
            "firstCol" => Self::FirstCol,
            "lastCol" => Self::LastCol,
            "firstRow" => Self::FirstRow,
            "lastRow" => Self::LastRow,
            "nwCell" => Self::NwCell,
            "neCell" => Self::NeCell,
            "swCell" => Self::SwCell,
            "seCell" => Self::SeCell,
            _ => return None,
        })
    }
}

/// The display properties one `w:tblStylePr` contributes to a cell.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableStyleConditionalFormat {
    /// `w:tcPr/w:shd/@w:fill`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// `w:rPr/w:b`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// `w:rPr/w:i`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// `w:rPr/w:color`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// `w:tcPr/w:tcBorders`. Outer edges describe the boundary of the
    /// conditional region (a row, a band, a column, or a corner cell) and
    /// `insideH`/`insideV` the edges between cells inside that region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<TableBorders>,
}

impl TableStyleConditionalFormat {
    pub fn is_empty(&self) -> bool {
        self.fill.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.color.is_none()
            && self.borders.as_ref().is_none_or(TableBorders::is_empty)
    }

    /// Resolves `w:basedOn` inheritance: the derived style's values win.
    pub fn inherit(parent: Self, child: Self) -> Self {
        Self {
            fill: child.fill.or(parent.fill),
            bold: child.bold.or(parent.bold),
            italic: child.italic.or(parent.italic),
            color: child.color.or(parent.color),
            borders: match (parent.borders, child.borders) {
                (Some(parent), Some(child)) => Some(TableBorders::inherit(parent, child)),
                (parent, child) => child.or(parent),
            },
        }
    }
}

impl TableBorders {
    /// Per-edge inheritance: every edge the child defines wins.
    pub fn inherit(parent: TableBorders, child: TableBorders) -> TableBorders {
        TableBorders {
            top: child.top.or(parent.top),
            bottom: child.bottom.or(parent.bottom),
            left: child.left.or(parent.left),
            right: child.right.or(parent.right),
            start: child.start.or(parent.start),
            end: child.end.or(parent.end),
            inside_h: child.inside_h.or(parent.inside_h),
            inside_v: child.inside_v.or(parent.inside_v),
        }
    }
}

/// `w:tblLook`: which conditional regions of the table style apply. `None`
/// for a switch means the element did not specify it, which resolves through
/// Word's `0x04A0` default (MS-OI29500 §17.4.55(a)).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableLook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_row: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_row: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_column: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_column: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_h_band: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_v_band: Option<bool>,
}

impl TableLook {
    pub const FIRST_ROW: u16 = 0x0020;
    pub const LAST_ROW: u16 = 0x0040;
    pub const FIRST_COLUMN: u16 = 0x0080;
    pub const LAST_COLUMN: u16 = 0x0100;
    pub const NO_H_BAND: u16 = 0x0200;
    pub const NO_V_BAND: u16 = 0x0400;
    /// Word's mask when `w:tblLook` is omitted (MS-OI29500 §17.4.55(a)).
    pub const WORD_DEFAULT_MASK: u16 = 0x04A0;

    /// Every switch taken from a hexadecimal `w:val` mask.
    pub fn from_mask(mask: u16) -> Self {
        Self {
            first_row: Some(mask & Self::FIRST_ROW != 0),
            last_row: Some(mask & Self::LAST_ROW != 0),
            first_column: Some(mask & Self::FIRST_COLUMN != 0),
            last_column: Some(mask & Self::LAST_COLUMN != 0),
            no_h_band: Some(mask & Self::NO_H_BAND != 0),
            no_v_band: Some(mask & Self::NO_V_BAND != 0),
        }
    }

    /// The effective mask after applying Word's default to unspecified
    /// switches.
    pub fn effective_mask(&self) -> u16 {
        let mut mask = 0;
        for (enabled, bit) in [
            (self.first_row_enabled(), Self::FIRST_ROW),
            (self.last_row_enabled(), Self::LAST_ROW),
            (self.first_column_enabled(), Self::FIRST_COLUMN),
            (self.last_column_enabled(), Self::LAST_COLUMN),
            (!self.horizontal_banding_enabled(), Self::NO_H_BAND),
            (!self.vertical_banding_enabled(), Self::NO_V_BAND),
        ] {
            if enabled {
                mask |= bit;
            }
        }
        mask
    }

    fn default_bit(bit: u16) -> bool {
        Self::WORD_DEFAULT_MASK & bit != 0
    }

    pub fn is_unspecified(&self) -> bool {
        *self == Self::default()
    }

    pub fn first_row_enabled(&self) -> bool {
        self.first_row
            .unwrap_or_else(|| Self::default_bit(Self::FIRST_ROW))
    }

    pub fn last_row_enabled(&self) -> bool {
        self.last_row
            .unwrap_or_else(|| Self::default_bit(Self::LAST_ROW))
    }

    pub fn first_column_enabled(&self) -> bool {
        self.first_column
            .unwrap_or_else(|| Self::default_bit(Self::FIRST_COLUMN))
    }

    pub fn last_column_enabled(&self) -> bool {
        self.last_column
            .unwrap_or_else(|| Self::default_bit(Self::LAST_COLUMN))
    }

    pub fn horizontal_banding_enabled(&self) -> bool {
        !self
            .no_h_band
            .unwrap_or_else(|| Self::default_bit(Self::NO_H_BAND))
    }

    pub fn vertical_banding_enabled(&self) -> bool {
        !self
            .no_v_band
            .unwrap_or_else(|| Self::default_bit(Self::NO_V_BAND))
    }
}

/// Where a cell sits among the rendered rows and the cells of its row.
///
/// Conditional column formatting follows the cell order inside a row (Word's
/// table model), so a row with `w:gridBefore` still has a first column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableStyleCellPosition {
    pub row: usize,
    pub row_count: usize,
    pub cell: usize,
    pub cell_count: usize,
}

/// Which edges of a cell lie on the boundary of a conditional region. The
/// horizontal flags are logical (reading order), not physical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableStyleRegionEdges {
    pub top: bool,
    pub bottom: bool,
    pub start: bool,
    pub end: bool,
}

/// One applicable conditional format and the cell's place in its region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableStyleConditionMatch {
    pub condition: TableStyleCondition,
    pub edges: TableStyleRegionEdges,
}

/// Band sizes after Word's defaults: an omitted size is `0`, which disables
/// banding (MS-OI29500 2.1.251).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableStyleBandSizes {
    pub rows: u32,
    pub columns: u32,
}

/// The applicable conditional formats for one cell in Word's application
/// order, each with the edges of its region that this cell touches.
pub fn table_style_condition_matches(
    look: &TableLook,
    bands: TableStyleBandSizes,
    position: TableStyleCellPosition,
) -> Vec<TableStyleConditionMatch> {
    let TableStyleCellPosition {
        row,
        row_count,
        cell,
        cell_count,
    } = position;
    let row_count = row_count.max(row + 1);
    let cell_count = cell_count.max(cell + 1);
    let is_first_row = row == 0;
    let is_last_row = row + 1 == row_count;
    let is_first_cell = cell == 0;
    let is_last_cell = cell + 1 == cell_count;
    let first_row = look.first_row_enabled() && is_first_row;
    let last_row = look.last_row_enabled() && is_last_row;
    let first_column = look.first_column_enabled() && is_first_cell;
    let last_column = look.last_column_enabled() && is_last_cell;

    // A whole row (or column) region spans the row (or column) from edge to
    // edge; a single-row region has both outer horizontal edges.
    let row_region = |top: bool, bottom: bool| TableStyleRegionEdges {
        top,
        bottom,
        start: is_first_cell,
        end: is_last_cell,
    };
    let column_region = |start: bool, end: bool| TableStyleRegionEdges {
        top: is_first_row,
        bottom: is_last_row,
        start,
        end,
    };
    let single_cell = TableStyleRegionEdges {
        top: true,
        bottom: true,
        start: true,
        end: true,
    };

    let mut matches = Vec::new();
    let mut push = |condition, edges| {
        matches.push(TableStyleConditionMatch { condition, edges });
    };

    // Header and total rows/columns are not part of the banded body: the
    // first body row after an enabled header row is always band 1.
    if look.horizontal_banding_enabled() && bands.rows > 0 && !first_row && !last_row {
        let header = usize::from(look.first_row_enabled());
        let body_row = row.saturating_sub(header);
        let body_end = row_count - usize::from(look.last_row_enabled());
        let size = bands.rows as usize;
        let offset = body_row % size;
        let condition = if (body_row / size).is_multiple_of(2) {
            TableStyleCondition::Band1Horz
        } else {
            TableStyleCondition::Band2Horz
        };
        push(
            condition,
            row_region(offset == 0, offset + 1 == size || row + 1 >= body_end),
        );
    }
    if look.vertical_banding_enabled() && bands.columns > 0 && !first_column && !last_column {
        let header = usize::from(look.first_column_enabled());
        let body_cell = cell.saturating_sub(header);
        let body_end = cell_count - usize::from(look.last_column_enabled());
        let size = bands.columns as usize;
        let offset = body_cell % size;
        let condition = if (body_cell / size).is_multiple_of(2) {
            TableStyleCondition::Band1Vert
        } else {
            TableStyleCondition::Band2Vert
        };
        push(
            condition,
            column_region(offset == 0, offset + 1 == size || cell + 1 >= body_end),
        );
    }
    if first_column {
        push(TableStyleCondition::FirstCol, column_region(true, true));
    }
    if last_column {
        push(TableStyleCondition::LastCol, column_region(true, true));
    }
    if first_row {
        push(TableStyleCondition::FirstRow, row_region(true, true));
    }
    if last_row {
        push(TableStyleCondition::LastRow, row_region(true, true));
    }
    // Corner cells need both regions that meet there (MS-OI29500 §17.4.55(b)).
    if first_row && first_column {
        push(TableStyleCondition::NwCell, single_cell);
    }
    if first_row && last_column {
        push(TableStyleCondition::NeCell, single_cell);
    }
    if last_row && first_column {
        push(TableStyleCondition::SwCell, single_cell);
    }
    if last_row && last_column {
        push(TableStyleCondition::SeCell, single_cell);
    }
    matches
}

/// Physical cell edges contributed by the table style's conditional formats.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyleCellBorders {
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
}

/// The table style's conditional formatting resolved for one cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyleCellFormat {
    pub fill: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub color: Option<String>,
    pub borders: TableStyleCellBorders,
}

impl TableStyleCellFormat {
    /// Layers the formats in the given order; each later format overrides
    /// every property (and every border edge) it defines.
    pub fn resolve<'a>(
        formats: impl IntoIterator<Item = (&'a TableStyleConditionalFormat, TableStyleRegionEdges)>,
        bidi_visual: bool,
    ) -> Self {
        let mut resolved = Self::default();
        for (format, edges) in formats {
            if let Some(fill) = &format.fill {
                resolved.fill = Some(fill.clone());
            }
            if let Some(bold) = format.bold {
                resolved.bold = Some(bold);
            }
            if let Some(italic) = format.italic {
                resolved.italic = Some(italic);
            }
            if let Some(color) = &format.color {
                resolved.color = Some(color.clone());
            }
            let Some(borders) = &format.borders else {
                continue;
            };
            let pick =
                |outer: bool, outer_edge: Option<&BorderEdge>, inner: &Option<BorderEdge>| {
                    if outer {
                        outer_edge.cloned()
                    } else {
                        inner.clone()
                    }
                };
            let (left_outer, right_outer) = if bidi_visual {
                (edges.end, edges.start)
            } else {
                (edges.start, edges.end)
            };
            let sides = [
                (
                    &mut resolved.borders.top,
                    pick(edges.top, borders.top.as_ref(), &borders.inside_h),
                ),
                (
                    &mut resolved.borders.bottom,
                    pick(edges.bottom, borders.bottom.as_ref(), &borders.inside_h),
                ),
                (
                    &mut resolved.borders.left,
                    pick(
                        left_outer,
                        borders.physical_left(bidi_visual),
                        &borders.inside_v,
                    ),
                ),
                (
                    &mut resolved.borders.right,
                    pick(
                        right_outer,
                        borders.physical_right(bidi_visual),
                        &borders.inside_v,
                    ),
                ),
            ];
            for (slot, edge) in sides {
                if edge.is_some() {
                    *slot = edge;
                }
            }
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(
        row: usize,
        row_count: usize,
        cell: usize,
        cell_count: usize,
    ) -> TableStyleCellPosition {
        TableStyleCellPosition {
            row,
            row_count,
            cell,
            cell_count,
        }
    }

    fn conditions(
        look: TableLook,
        bands: (u32, u32),
        pos: TableStyleCellPosition,
    ) -> Vec<TableStyleCondition> {
        table_style_condition_matches(
            &look,
            TableStyleBandSizes {
                rows: bands.0,
                columns: bands.1,
            },
            pos,
        )
        .into_iter()
        .map(|matched| matched.condition)
        .collect()
    }

    #[test]
    fn omitted_table_look_uses_word_04a0_default() {
        let look = TableLook::default();
        assert!(look.first_row_enabled());
        assert!(!look.last_row_enabled());
        assert!(look.first_column_enabled());
        assert!(!look.last_column_enabled());
        assert!(look.horizontal_banding_enabled());
        assert!(!look.vertical_banding_enabled());
        assert_eq!(look.effective_mask(), TableLook::WORD_DEFAULT_MASK);
        assert_eq!(TableLook::from_mask(0x04A0).effective_mask(), 0x04A0);
    }

    #[test]
    fn whole_table_and_unknown_conditions_are_not_applied() {
        assert_eq!(TableStyleCondition::from_type_token("wholeTable"), None);
        assert_eq!(TableStyleCondition::from_type_token("bogus"), None);
        assert_eq!(
            TableStyleCondition::from_type_token("nwCell"),
            Some(TableStyleCondition::NwCell)
        );
    }

    #[test]
    fn conditions_follow_office_order_and_need_both_corner_regions() {
        let all = TableLook::from_mask(0x01E0);
        // Top-left: banding is suppressed by the header row/first column, and
        // the corner applies because both meeting regions are on.
        assert_eq!(
            conditions(all, (1, 1), position(0, 4, 0, 3)),
            vec![
                TableStyleCondition::FirstCol,
                TableStyleCondition::FirstRow,
                TableStyleCondition::NwCell,
            ]
        );
        assert_eq!(
            conditions(all, (1, 1), position(3, 4, 2, 3)),
            vec![
                TableStyleCondition::LastCol,
                TableStyleCondition::LastRow,
                TableStyleCondition::SeCell,
            ]
        );
        // Without the first column switch the top-left cell is only a
        // header-row cell, so `nwCell` must not apply.
        let header_only = TableLook::from_mask(TableLook::FIRST_ROW | TableLook::NO_V_BAND);
        assert_eq!(
            conditions(header_only, (1, 1), position(0, 4, 0, 3)),
            vec![TableStyleCondition::FirstRow]
        );
        // Body cell with both banding directions: rows apply before columns.
        let banded = TableLook::from_mask(TableLook::FIRST_ROW | TableLook::FIRST_COLUMN);
        assert_eq!(
            conditions(banded, (1, 1), position(2, 4, 2, 3)),
            vec![
                TableStyleCondition::Band2Horz,
                TableStyleCondition::Band2Vert
            ]
        );
    }

    #[test]
    fn banding_skips_header_rows_and_honors_band_size_zero() {
        let look = TableLook::from_mask(TableLook::FIRST_ROW | TableLook::NO_V_BAND);
        let row_condition = |row, size| {
            conditions(look, (size, 0), position(row, 6, 1, 3))
                .first()
                .copied()
        };
        assert_eq!(row_condition(1, 1), Some(TableStyleCondition::Band1Horz));
        assert_eq!(row_condition(2, 1), Some(TableStyleCondition::Band2Horz));
        assert_eq!(row_condition(1, 2), Some(TableStyleCondition::Band1Horz));
        assert_eq!(row_condition(2, 2), Some(TableStyleCondition::Band1Horz));
        assert_eq!(row_condition(3, 2), Some(TableStyleCondition::Band2Horz));
        // Word's default band size is 0, which disables banding.
        assert_eq!(row_condition(1, 0), None);
    }

    #[test]
    fn band_regions_report_group_edges() {
        let look = TableLook::from_mask(TableLook::FIRST_ROW | TableLook::NO_V_BAND);
        let edges = |row| {
            table_style_condition_matches(
                &look,
                TableStyleBandSizes {
                    rows: 2,
                    columns: 0,
                },
                position(row, 6, 1, 3),
            )[0]
            .edges
        };
        assert!(edges(1).top && !edges(1).bottom);
        assert!(!edges(2).top && edges(2).bottom);
        // The last row is the end of its (shortened) band.
        assert!(edges(5).top && edges(5).bottom);
        assert!(!edges(1).start && !edges(1).end);
    }

    fn edge(color: &str) -> BorderEdge {
        BorderEdge {
            val: Some("single".into()),
            size_eighths: Some(4),
            space: None,
            color: Some(color.into()),
        }
    }

    #[test]
    fn later_formats_override_fill_runs_and_individual_border_edges() {
        let band = TableStyleConditionalFormat {
            fill: Some("#BAND".into()),
            italic: Some(true),
            borders: Some(TableBorders {
                top: Some(edge("#B0B0B0")),
                inside_v: Some(edge("#B1B1B1")),
                ..TableBorders::default()
            }),
            ..TableStyleConditionalFormat::default()
        };
        let first_column = TableStyleConditionalFormat {
            fill: Some("#COL".into()),
            bold: Some(true),
            borders: Some(TableBorders {
                left: Some(edge("#C0C0C0")),
                ..TableBorders::default()
            }),
            ..TableStyleConditionalFormat::default()
        };
        let row_edges = TableStyleRegionEdges {
            top: true,
            bottom: false,
            start: true,
            end: false,
        };
        let resolved =
            TableStyleCellFormat::resolve([(&band, row_edges), (&first_column, row_edges)], false);
        assert_eq!(resolved.fill.as_deref(), Some("#COL"));
        assert_eq!(resolved.bold, Some(true));
        assert_eq!(resolved.italic, Some(true));
        assert_eq!(resolved.borders.top, Some(edge("#B0B0B0")));
        assert_eq!(resolved.borders.left, Some(edge("#C0C0C0")));
        // The right side is interior to the band, so it takes insideV.
        assert_eq!(resolved.borders.right, Some(edge("#B1B1B1")));
        assert_eq!(resolved.borders.bottom, None);

        // In a bidiVisual table the logical start edge is physically right.
        let mirrored = TableStyleCellFormat::resolve([(&first_column, row_edges)], true);
        assert_eq!(mirrored.borders.left, None);
        assert_eq!(mirrored.borders.right, None, "legacy left stays physical");
        let logical = TableStyleConditionalFormat {
            borders: Some(TableBorders {
                start: Some(edge("#5A5A5A")),
                ..TableBorders::default()
            }),
            ..TableStyleConditionalFormat::default()
        };
        let mirrored = TableStyleCellFormat::resolve([(&logical, row_edges)], true);
        assert_eq!(mirrored.borders.right, Some(edge("#5A5A5A")));
    }
}
