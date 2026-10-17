//! Logical selection of cells or complete worksheet axes, independent of
//! how much of the sheet currently has geometry or stored values.
use crate::SheetLayout;
use jade_xlsx_reader::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    CellRange, CellRef, Sheet,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionKind {
    #[default]
    Cells,
    Rows,
    Columns,
    Sheet,
}
impl SelectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cells => "cells",
            Self::Rows => "rows",
            Self::Columns => "columns",
            Self::Sheet => "sheet",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionArea {
    pub kind: SelectionKind,
    pub anchor: CellRef,
    pub focus: CellRef,
}
impl SelectionArea {
    pub fn range(self) -> CellRange {
        let mut range = CellRange {
            start: CellRef::new(
                self.anchor.column.min(self.focus.column),
                self.anchor.row.min(self.focus.row),
            ),
            end: CellRef::new(
                self.anchor.column.max(self.focus.column),
                self.anchor.row.max(self.focus.row),
            ),
        };
        if matches!(self.kind, SelectionKind::Rows | SelectionKind::Sheet) {
            range.start.column = 0;
            range.end.column = MAX_COLUMN_COUNT - 1;
        }
        if matches!(self.kind, SelectionKind::Columns | SelectionKind::Sheet) {
            range.start.row = 0;
            range.end.row = MAX_ROW_COUNT - 1;
        }
        range
    }
    /// Clipboard contents only need the stored extent along an unselected
    /// axis. The selected row/column span remains intact, including blanks.
    pub fn clipboard_range(self, sheet: &Sheet) -> CellRange {
        let mut range = self.range();
        if matches!(self.kind, SelectionKind::Rows | SelectionKind::Sheet) {
            range.end.column = sheet.used_column_count().saturating_sub(1);
        }
        if matches!(self.kind, SelectionKind::Columns | SelectionKind::Sheet) {
            range.end.row = sheet.used_row_count().saturating_sub(1);
        }
        range
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderHit {
    pub kind: SelectionKind,
    pub reference: CellRef,
}
/// Header coordinates use the same frozen-pane mapping as body hit testing.
pub fn header_hit(
    layout: &SheetLayout,
    x: f64,
    y: f64,
    scroll_x: f64,
    scroll_y: f64,
) -> Option<HeaderHit> {
    let metrics = layout.header_metrics();
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let row = x < metrics.row_width;
    let column = y < metrics.column_height;
    if !row && !column {
        return None;
    }
    let kind = match (row, column) {
        (true, true) => SelectionKind::Sheet,
        (true, false) => SelectionKind::Rows,
        _ => SelectionKind::Columns,
    };
    let (sx, sy) = layout.sheet_point_in_viewport(
        if row { 0.0 } else { x - metrics.row_width },
        if column {
            0.0
        } else {
            y - metrics.column_height
        },
        scroll_x,
        scroll_y,
    );
    Some(HeaderHit {
        kind,
        reference: CellRef::new(
            crate::axis_at(&layout.columns, sx)?,
            crate::axis_at(&layout.rows, sy)?,
        ),
    })
}
/// Existing valued/formula cells in a range; clearing an axis must not
/// materialize millions of absent cells or replace untouched empty styles.
pub fn occupied_cells(sheet: &Sheet, range: CellRange) -> Vec<CellRef> {
    sheet
        .rows
        .iter()
        .filter(|row| (range.start.row..=range.end.row).contains(&row.index))
        .flat_map(|row| row.cells.iter())
        .filter(|cell| {
            range.contains(cell.reference) && (!cell.value.is_empty() || cell.formula.is_some())
        })
        .map(|cell| cell.reference)
        .collect()
}
