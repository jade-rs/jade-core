//! Excel's outline gutter: the strip left of the row numbers (and above the
//! column letters) that a grouped sheet grows, with a numbered button per
//! level, a bracket beside each group and a box on its summary row — `−`
//! while the group shows, `+` once its detail is hidden.
//!
//! The geometry lives here so that the painter draws what a click hits.

use jade_xlsx_reader::Axis;

use crate::SheetLayout;

/// Width of one level's lane in the gutter, in px.
pub const LEVEL_STEP: f64 = 16.0;
/// Side of a level button and of a group's box, in px.
pub const BUTTON: f64 = 11.0;

/// The gutter a sheet grouped `levels` deep grows: a lane per level and one
/// more for the button that shows everything.
pub fn gutter_size(levels: u8) -> f64 {
    if levels == 0 {
        0.0
    } else {
        (f64::from(levels) + 1.0) * LEVEL_STEP
    }
}

/// The middle of level `level`'s lane (1-based), across the gutter.
pub fn lane_center(level: u8) -> f64 {
    (f64::from(level) - 0.5) * LEVEL_STEP
}

/// What a click in the gutter means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineTarget {
    /// A level button: show the outline down to this level.
    Level { axis: Axis, level: u8 },
    /// A group's box on its summary row or column.
    Toggle {
        axis: Axis,
        summary: u32,
        collapsed: bool,
    },
}

impl SheetLayout {
    /// How deep rows (or columns) are grouped; 0 when they are not.
    pub fn outline_levels(&self, axis: Axis) -> u8 {
        let levels = match axis {
            Axis::Rows => &self.outline.row_levels,
            Axis::Columns => &self.outline.column_levels,
        };
        levels.values().copied().max().unwrap_or(0)
    }

    /// A row's (or column's) outline level.
    pub fn outline_level(&self, axis: Axis, index: u32) -> u8 {
        match axis {
            Axis::Rows => self.outline.row_level(index),
            Axis::Columns => self.outline.column_level(index),
        }
    }

    /// The levels whose groups `index` is the summary of: the row after a
    /// group (or before it, with summaries above), shallower than it.
    pub fn outline_summary_levels(&self, axis: Axis, index: u32) -> Vec<u8> {
        let after = match axis {
            Axis::Rows => self.outline.summary_below,
            Axis::Columns => self.outline.summary_right,
        };
        let detail = if after {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        let Some(detail) = detail else {
            return Vec::new();
        };
        let own = self.outline_level(axis, index);
        (own + 1..=self.outline_level(axis, detail)).collect()
    }

    /// Whether the group `index` summarises is collapsed: so marked, or its
    /// neighbouring detail hidden.
    pub fn outline_collapsed(&self, axis: Axis, index: u32) -> bool {
        let (collapsed, after) = match axis {
            Axis::Rows => (&self.outline.collapsed_rows, self.outline.summary_below),
            Axis::Columns => (&self.outline.collapsed_columns, self.outline.summary_right),
        };
        if collapsed.contains(&index) {
            return true;
        }
        let detail = if after {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        let entry = detail.and_then(|detail| match axis {
            Axis::Rows => self.row(detail),
            Axis::Columns => self.column(detail),
        });
        entry.is_some_and(|entry| entry.hidden || entry.size <= 0.0)
    }

    /// What the gutter has at viewport point `(x, y)` (headers included) with
    /// the sheet scrolled by `scroll`, if anything.
    pub fn outline_hit(&self, x: f64, y: f64, scroll: (f64, f64)) -> Option<OutlineTarget> {
        let metrics = self.header_metrics();
        if x < metrics.outline_width {
            let levels = self.outline_levels(Axis::Rows);
            if y >= metrics.outline_height && y < metrics.column_height {
                let level = (x / LEVEL_STEP).floor() as u8 + 1;
                return (level <= levels + 1).then_some(OutlineTarget::Level {
                    axis: Axis::Rows,
                    level,
                });
            }
            if y >= metrics.column_height {
                let (_, sheet_y) = self.sheet_point_in_viewport(
                    0.0,
                    y - metrics.column_height,
                    scroll.0,
                    scroll.1,
                );
                let row = self.row_at(sheet_y)?;
                return self.toggle_at(Axis::Rows, row, x);
            }
            return None;
        }
        if y < metrics.outline_height {
            let levels = self.outline_levels(Axis::Columns);
            if x >= metrics.outline_width && x < metrics.row_width {
                let level = (y / LEVEL_STEP).floor() as u8 + 1;
                return (level <= levels + 1).then_some(OutlineTarget::Level {
                    axis: Axis::Columns,
                    level,
                });
            }
            if x >= metrics.row_width {
                let (sheet_x, _) =
                    self.sheet_point_in_viewport(x - metrics.row_width, 0.0, scroll.0, scroll.1);
                let column = self.column_at(sheet_x)?;
                return self.toggle_at(Axis::Columns, column, y);
            }
        }
        None
    }

    /// The box of a group `index` summarises, under `across` in the gutter.
    fn toggle_at(&self, axis: Axis, index: u32, across: f64) -> Option<OutlineTarget> {
        self.outline_summary_levels(axis, index)
            .into_iter()
            .find(|level| (across - lane_center(*level)).abs() <= LEVEL_STEP / 2.0)
            .map(|_| OutlineTarget::Toggle {
                axis,
                summary: index,
                collapsed: self.outline_collapsed(axis, index),
            })
    }
}
