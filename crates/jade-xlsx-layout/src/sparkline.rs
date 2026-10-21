//! Sparklines placed in their cells, as chart paint operations.

use crate::chart::ChartOp;
use crate::{AxisEntry, Rect};
use jade_xlsx_reader::{
    chart::resolve_chart_reference, sparkline::SparklineGroup, CellRef, CellValue, Color, Sheet,
    Workbook,
};
use serde::{Deserialize, Serialize};

/// A sparkline drawn in its cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedSparkline {
    pub reference: CellRef,
    /// The cell's rectangle.
    pub rect: Rect,
    pub ops: Vec<ChartOp>,
}

fn css(color: Option<Color>, fallback: [u8; 4]) -> String {
    let [r, g, b, a] = color.and_then(crate::color::rgba).unwrap_or(fallback);
    format!("rgba({r}, {g}, {b}, {})", f64::from(a) / 255.0)
}

/// The values a sparkline plots, `None` for an empty or hidden point.
fn values(
    formula: &str,
    sheet: &Sheet,
    workbook: Option<&Workbook>,
    group: &SparklineGroup,
) -> Vec<Option<f64>> {
    let Some(reference) = resolve_chart_reference(formula, sheet, workbook) else {
        return Vec::new();
    };
    reference
        .points
        .iter()
        .filter(|point| group.display_hidden || !point.hidden)
        .map(|point| match point.cell.map(|cell| &cell.value) {
            Some(CellValue::Number(value)) if value.is_finite() => Some(*value),
            _ if group.empty_cells == "zero" => Some(0.0),
            _ => None,
        })
        .collect()
}

pub fn place(
    sheet: &Sheet,
    workbook: Option<&Workbook>,
    columns: &[AxisEntry],
    rows: &[AxisEntry],
) -> Vec<PlacedSparkline> {
    let mut placed = Vec::new();
    for group in &sheet.sparkline_groups {
        let series: Vec<Vec<Option<f64>>> = group
            .sparklines
            .iter()
            .map(|sparkline| values(&sparkline.formula, sheet, workbook, group))
            .collect();
        let all = || series.iter().flatten().flatten().copied();
        let group_min = all().fold(f64::INFINITY, f64::min);
        let group_max = all().fold(f64::NEG_INFINITY, f64::max);
        for (sparkline, values) in group.sparklines.iter().zip(&series) {
            let (Some(column), Some(row)) = (
                columns.get(sparkline.cell.column as usize),
                rows.get(sparkline.cell.row as usize),
            ) else {
                continue;
            };
            if column.size <= 0.0 || row.size <= 0.0 {
                continue;
            }
            let rect = Rect {
                x: column.offset,
                y: row.offset,
                width: column.size,
                height: row.size,
            };
            let own_min = values
                .iter()
                .flatten()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let own_max = values
                .iter()
                .flatten()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            if !own_min.is_finite() {
                continue;
            }
            let min = match group.min_axis.as_str() {
                "group" => group_min,
                "custom" => group.manual_min.unwrap_or(own_min),
                _ => own_min,
            };
            let max = match group.max_axis.as_str() {
                "group" => group_max,
                "custom" => group.manual_max.unwrap_or(own_max),
                _ => own_max,
            };
            let ops = draw(group, values, rect, min, max);
            placed.push(PlacedSparkline {
                reference: sparkline.cell,
                rect,
                ops,
            });
        }
    }
    placed
}

fn draw(
    group: &SparklineGroup,
    values: &[Option<f64>],
    cell: Rect,
    min: f64,
    max: f64,
) -> Vec<ChartOp> {
    let mut ops = Vec::new();
    let count = values.len();
    if count == 0 {
        return ops;
    }
    let inset_x = 3.0_f64.min(cell.width / 4.0);
    let inset_y = 3.0_f64.min(cell.height / 4.0);
    let area = Rect {
        x: cell.x + inset_x,
        y: cell.y + inset_y,
        width: (cell.width - 2.0 * inset_x).max(1.0),
        height: (cell.height - 2.0 * inset_y).max(1.0),
    };
    let order = |index: usize| {
        if group.right_to_left {
            count - 1 - index
        } else {
            index
        }
    };
    let span = if max > min { max - min } else { 1.0 };
    let y_of = |value: f64| {
        let fraction = if max > min {
            ((value - min) / span).clamp(0.0, 1.0)
        } else {
            0.5
        };
        area.bottom() - fraction * area.height
    };
    let present: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.map(|value| (index, value)))
        .collect();
    let high = present
        .iter()
        .map(|(_, v)| *v)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = present
        .iter()
        .map(|(_, v)| *v)
        .fold(f64::INFINITY, f64::min);
    let first = present.first().map(|(index, _)| *index);
    let last = present.last().map(|(index, _)| *index);
    let series = css(group.color_series, [55, 96, 146, 255]);
    // The colour a point takes for what it is: highest, lowest, first,
    // last or negative, when the group marks that.
    let special = |index: usize, value: f64| -> Option<String> {
        if group.high && value == high {
            return Some(css(group.color_high, [208, 0, 0, 255]));
        }
        if group.low && value == low {
            return Some(css(group.color_low, [208, 0, 0, 255]));
        }
        if group.first && Some(index) == first {
            return Some(css(group.color_first, [208, 0, 0, 255]));
        }
        if group.last && Some(index) == last {
            return Some(css(group.color_last, [208, 0, 0, 255]));
        }
        if group.negative && value < 0.0 {
            return Some(css(group.color_negative, [208, 0, 0, 255]));
        }
        None
    };
    let axis = |ops: &mut Vec<ChartOp>, y: f64| {
        ops.push(ChartOp::Path {
            points: vec![(area.x, y), (area.right(), y)],
            closed: false,
            fill: None,
            stroke: Some(css(group.color_axis, [0, 0, 0, 255])),
            width: 0.75,
            dash: Vec::new(),
        });
    };
    match group.kind.as_str() {
        "column" | "stacked" => {
            let slot = area.width / count as f64;
            let bar = (slot * 0.8).max(1.0);
            let win_loss = group.kind == "stacked";
            let middle = area.y + area.height / 2.0;
            let base = if win_loss {
                middle
            } else if min < 0.0 && max > 0.0 {
                y_of(0.0)
            } else if max <= 0.0 {
                area.y
            } else {
                area.bottom()
            };
            for (index, value) in &present {
                let x = area.x + order(*index) as f64 * slot + (slot - bar) / 2.0;
                let top = if win_loss {
                    if *value > 0.0 {
                        area.y
                    } else if *value < 0.0 {
                        area.bottom()
                    } else {
                        continue;
                    }
                } else {
                    y_of(*value)
                };
                let (y0, y1) = (top.min(base), top.max(base));
                // The smallest column still shows.
                let (y0, y1) = if y1 - y0 < 1.0 {
                    (y1 - 1.0, y1)
                } else {
                    (y0, y1)
                };
                ops.push(ChartOp::Rectangle {
                    rect: Rect {
                        x,
                        y: y0,
                        width: bar,
                        height: y1 - y0,
                    },
                    fill: Some(special(*index, *value).unwrap_or_else(|| series.clone())),
                    stroke: None,
                });
            }
            if group.display_axis && (win_loss || (min < 0.0 && max > 0.0)) {
                axis(&mut ops, base);
            }
        }
        _ => {
            if group.display_axis && min < 0.0 && max > 0.0 {
                axis(&mut ops, y_of(0.0));
            }
            let x_of = |index: usize| {
                if count == 1 {
                    area.x + area.width / 2.0
                } else {
                    area.x + order(index) as f64 * area.width / (count - 1) as f64
                }
            };
            let width = (group.line_weight_points * 96.0 / 72.0).max(0.5);
            let mut segment: Vec<(f64, f64)> = Vec::new();
            let flush = |segment: &mut Vec<(f64, f64)>, ops: &mut Vec<ChartOp>| {
                if segment.len() > 1 {
                    ops.push(ChartOp::Path {
                        points: std::mem::take(segment),
                        closed: false,
                        fill: None,
                        stroke: Some(series.clone()),
                        width,
                        dash: Vec::new(),
                    });
                } else {
                    segment.clear();
                }
            };
            for (index, value) in values.iter().enumerate() {
                match value {
                    Some(value) => segment.push((x_of(index), y_of(*value))),
                    None if group.empty_cells == "span" => {}
                    None => flush(&mut segment, &mut ops),
                }
            }
            flush(&mut segment, &mut ops);
            let radius = (width * 1.5).max(1.5);
            for (index, value) in &present {
                let color = special(*index, *value).or_else(|| {
                    group
                        .markers
                        .then(|| css(group.color_markers, [55, 96, 146, 255]))
                });
                if let Some(color) = color {
                    let (x, y) = (x_of(*index), y_of(*value));
                    ops.push(ChartOp::Path {
                        points: vec![
                            (x - radius, y - radius),
                            (x + radius, y - radius),
                            (x + radius, y + radius),
                            (x - radius, y + radius),
                        ],
                        closed: true,
                        fill: Some(color.clone()),
                        stroke: Some(color),
                        width: 0.5,
                        dash: Vec::new(),
                    });
                }
            }
        }
    }
    ops
}
