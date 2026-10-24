//! Geometry for inserting/deleting a rectangle without moving entire axes.
use crate::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    Axis, CellRange, CellRef, StructureChange,
};
use serde::{Deserialize, Serialize};

/// Insert shifts cells down/right; delete shifts cells up/left. The rectangle
/// gives both the insertion/deletion size and the perpendicular strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellShift {
    pub sheet: String,
    pub range: CellRange,
    pub axis: Axis,
    pub inserted: bool,
}

impl CellShift {
    pub fn is_valid(&self) -> bool {
        let r = self.range;
        r.start.column <= r.end.column
            && r.start.row <= r.end.row
            && r.end.column < MAX_COLUMN_COUNT
            && r.end.row < MAX_ROW_COUNT
    }

    pub fn axis_change(&self) -> StructureChange {
        let (at, end) = match self.axis {
            Axis::Rows => (self.range.start.row, self.range.end.row),
            Axis::Columns => (self.range.start.column, self.range.end.column),
        };
        StructureChange {
            sheet: self.sheet.clone(),
            axis: self.axis,
            at,
            count: end.saturating_sub(at).saturating_add(1),
            inserted: self.inserted,
        }
    }

    pub fn strip(&self) -> (u32, u32) {
        match self.axis {
            Axis::Rows => (self.range.start.column, self.range.end.column),
            Axis::Columns => (self.range.start.row, self.range.end.row),
        }
    }

    pub fn perpendicular(&self, at: CellRef) -> u32 {
        match self.axis {
            Axis::Rows => at.column,
            Axis::Columns => at.row,
        }
    }

    pub fn along(&self, at: CellRef) -> u32 {
        match self.axis {
            Axis::Rows => at.row,
            Axis::Columns => at.column,
        }
    }

    pub fn covers(&self, range: CellRange) -> bool {
        let (first, last) = self.strip();
        first <= self.perpendicular(range.start) && self.perpendicular(range.end) <= last
    }

    pub fn affects(&self, range: CellRange) -> bool {
        let (first, last) = self.strip();
        self.perpendicular(range.start) <= last
            && self.perpendicular(range.end) >= first
            && self.along(range.end) >= self.axis_change().at
    }

    pub fn map_cell(&self, at: CellRef) -> Option<CellRef> {
        let (first, last) = self.strip();
        if (first..=last).contains(&self.perpendicular(at)) {
            self.axis_change().map_cell(at)
        } else {
            Some(at)
        }
    }

    /// A rectangular formula reference follows only when its perpendicular
    /// span is inside the strip. A crossing reference keeps its boundaries.
    pub fn map_reference(&self, range: CellRange) -> Option<CellRange> {
        if self.covers(range) {
            self.axis_change().map_range(range)
        } else {
            Some(range)
        }
    }

    /// Coverage metadata may become disconnected. Split along strip edges
    /// before mapping, retaining the parts on either side. Unlike a single
    /// cell reference, coverage reaching the grid edge is clipped there.
    pub fn map_regions(&self, range: CellRange) -> Vec<CellRange> {
        let (first, last) = self.strip();
        let (a, b) = (
            self.perpendicular(range.start),
            self.perpendicular(range.end),
        );
        if b < first || a > last {
            return vec![range];
        }
        let with_span = |lo, hi| match self.axis {
            Axis::Rows => CellRange {
                start: CellRef::new(lo, range.start.row),
                end: CellRef::new(hi, range.end.row),
            },
            Axis::Columns => CellRange {
                start: CellRef::new(range.start.column, lo),
                end: CellRef::new(range.end.column, hi),
            },
        };
        let mut result = Vec::with_capacity(3);
        if a < first {
            result.push(with_span(a, first - 1));
        }
        let middle = with_span(a.max(first), b.min(last));
        let change = self.axis_change();
        let mapped = if change.inserted {
            let limit = match self.axis {
                Axis::Rows => MAX_ROW_COUNT,
                Axis::Columns => MAX_COLUMN_COUNT,
            };
            change.map_cell(middle.start).map(|start| {
                let mut end = middle.end;
                let index = self.along(end);
                let moved = if index >= change.at {
                    index.saturating_add(change.count).min(limit - 1)
                } else {
                    index
                };
                match self.axis {
                    Axis::Rows => end.row = moved,
                    Axis::Columns => end.column = moved,
                }
                CellRange { start, end }
            })
        } else {
            change.map_range(middle)
        };
        if let Some(mapped) = mapped {
            result.push(mapped);
        }
        if b > last {
            result.push(with_span(last + 1, b));
        }
        result
    }
}
