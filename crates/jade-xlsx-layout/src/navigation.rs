//! Keyboard movement uses the same visible axes as drawing and hit testing.

use crate::{AxisEntry, SheetLayout};
use jade_xlsx_reader::{CellRef, Sheet};

/// Steps across visible cells, leaving a merge at its outer edge and landing
/// on a destination merge's anchor. At a grid boundary the selection stays put.
pub fn move_cell(
    sheet: &Sheet,
    layout: &SheetLayout,
    from: CellRef,
    d_column: i32,
    d_row: i32,
) -> CellRef {
    let merge = sheet.merge_at(from);
    let start = merge.map(|range| range.start).unwrap_or(from);
    let end = merge.map(|range| range.end).unwrap_or(from);
    let column_from = if d_column > 0 {
        end.column
    } else {
        start.column
    };
    let row_from = if d_row > 0 { end.row } else { start.row };
    let landed = CellRef::new(
        step_axis(&layout.columns, column_from, d_column),
        step_axis(&layout.rows, row_from, d_row),
    );
    sheet
        .merge_at(landed)
        .map(|range| range.start)
        .unwrap_or(landed)
}

fn step_axis(entries: &[AxisEntry], from: u32, delta: i32) -> u32 {
    let mut current = from;
    let direction = delta.signum();
    for _ in 0..delta.unsigned_abs() {
        let mut next = i64::from(current) + i64::from(direction);
        while let Some(entry) = usize::try_from(next)
            .ok()
            .and_then(|index| entries.get(index))
        {
            if !entry.hidden && entry.size > 0.0 {
                current = entry.index;
                break;
            }
            next += i64::from(direction);
        }
        if next < 0 || next >= entries.len() as i64 {
            break;
        }
    }
    current
}
