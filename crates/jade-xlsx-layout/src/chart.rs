//! Chart data resolution and paint geometry shared by every XLSX renderer.
use crate::{number_format, LayoutOptions, Rect, ResolvedFont};
use jade_xlsx_reader::{
    chart::{resolve_chart_reference, Chart, ChartAxis, ChartData, ChartPointState},
    CellValue, DateSystem, Sheet, StyleTable, Workbook,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChartOp {
    Clip {
        rect: Rect,
    },
    Restore,
    Rectangle {
        rect: Rect,
        fill: Option<String>,
        stroke: Option<String>,
    },
    Path {
        points: Vec<(f64, f64)>,
        closed: bool,
        fill: Option<String>,
        stroke: Option<String>,
        width: f64,
    },
    Text {
        text: String,
        x: f64,
        y: f64,
        font: ResolvedFont,
        color: String,
        align: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedChart {
    pub plot: Rect,
    pub ops: Vec<ChartOp>,
    pub unsupported_properties: Vec<String>,
    pub source_states: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartStatus {
    pub sheet: String,
    pub sheet_index: usize,
    pub name: String,
    pub part: Option<String>,
    pub kind: String,
    pub rendered: bool,
    pub unsupported_properties: Vec<String>,
    pub source_states: Vec<String>,
}
#[derive(Clone)]
struct Source {
    points: Vec<Option<String>>,
    hidden: Vec<bool>,
    state: String,
}
fn source(
    data: &ChartData,
    sheet: &Sheet,
    workbook: Option<&Workbook>,
    visible_only: bool,
    styles: &StyleTable,
    dates: DateSystem,
    format_labels: bool,
) -> Source {
    let cache = || Source {
        points: data
            .points
            .iter()
            .map(|point| {
                point.as_ref().map(|text| {
                    if data.numeric && format_labels {
                        if let Ok(value) = text.parse::<f64>() {
                            return number_format::format_value(
                                &CellValue::Number(value),
                                data.format_code.as_deref(),
                                dates,
                            )
                            .text;
                        }
                    }
                    text.clone()
                })
            })
            .collect(),
        hidden: vec![false; data.points.len()],
        state: if data.formula.is_some() {
            "cached"
        } else {
            "literal"
        }
        .into(),
    };
    let Some(formula) = data.formula.as_deref() else {
        return cache();
    };
    let Some(reference) = resolve_chart_reference(formula, sheet, workbook) else {
        return cache();
    };
    let mut points = Vec::with_capacity(reference.points.len());
    let mut hidden = Vec::with_capacity(reference.points.len());
    let mut state = ChartPointState::Live;
    let styles = workbook.map(|w| &w.styles).unwrap_or(styles);
    let conditional = crate::conditional_format::ConditionalStyles::new(reference.sheet, styles);
    for point in &reference.points {
        if point.state == ChartPointState::Uncalculated
            || (point.state == ChartPointState::Stale && state != ChartPointState::Uncalculated)
        {
            state = point.state;
        }
        points.push(point.cell.and_then(|cell| {
            if matches!(cell.value, CellValue::Empty) {
                return None;
            }
            if !format_labels {
                return match &cell.value {
                    CellValue::Number(v) => v.is_finite().then(|| v.to_string()),
                    CellValue::Bool(v) => Some(if *v { "1" } else { "0" }.into()),
                    CellValue::Text(v) => Some(v.clone()),
                    CellValue::Error(v) => Some(v.clone()),
                    _ => None,
                };
            }
            let format = styles.cell_format(
                reference
                    .sheet
                    .effective_style_index(point.reference, cell.style_index),
            );
            let conditional_style = conditional.for_cell(point.reference, &cell.value);
            let code = conditional_style
                .number_format_code
                .as_deref()
                .or_else(|| format.and_then(|f| styles.number_format_code(f)));
            Some(
                number_format::format_value(
                    &cell.value,
                    code,
                    workbook.map(|w| w.date_system).unwrap_or(dates),
                )
                .text,
            )
        }));
        hidden.push(visible_only && point.hidden);
    }

    Source {
        points,
        hidden,
        state: state.as_str().into(),
    }
}
fn number(value: Option<&String>) -> Option<f64> {
    value
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}
fn rgb(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}
fn series_color(chart: &Chart, index: usize) -> String {
    rgb(resolved_color(chart, &chart.series[index].color).unwrap_or(chart.palette[index % 6]))
}
fn resolved_color(chart: &Chart, color: &jade_xlsx_reader::chart::ChartColor) -> Option<[u8; 3]> {
    color.rgb.or_else(|| match color.scheme.as_deref()? {
        "dk1" | "tx1" => Some([0, 0, 0]),
        "lt1" | "bg1" => Some([255, 255, 255]),
        "dk2" | "tx2" => Some([68, 84, 106]),
        "lt2" | "bg2" => Some([231, 230, 230]),
        name if name.starts_with("accent") => name[6..]
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=6).contains(n))
            .map(|n| chart.palette[n - 1]),
        _ => None,
    })
}
fn point_color(chart: &Chart, series: usize, index: usize, vary: bool) -> String {
    rgb(chart.series[series]
        .point_colors
        .get(&index)
        .and_then(|c| resolved_color(chart, c))
        .unwrap_or(if vary {
            chart.palette[index % 6]
        } else {
            resolved_color(chart, &chart.series[series].color).unwrap_or(chart.palette[series % 6])
        }))
}
#[derive(Clone, Copy)]
struct Scale {
    min: f64,
    max: f64,
    step: f64,
    reverse: bool,
}
impl Scale {
    fn fraction(self, value: f64) -> f64 {
        let f = (value - self.min) / (self.max - self.min);
        if self.reverse {
            1.0 - f
        } else {
            f
        }
    }
    fn ticks(self) -> Vec<f64> {
        let first = (self.min / self.step).ceil() * self.step;
        let count = ((self.max - first) / self.step).floor().max(0.0) as usize;
        (0..=count.min(200))
            .map(|i| first + i as f64 * self.step)
            .collect()
    }
}
fn scale(
    values: impl Iterator<Item = f64>,
    axis: &ChartAxis,
    zero: bool,
    issues: &mut Vec<String>,
) -> Scale {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values {
        min = min.min(v);
        max = max.max(v);
    }
    if !min.is_finite() {
        min = 0.0;
        max = 1.0;
    }
    if zero {
        min = min.min(0.0);
        max = max.max(0.0);
    }
    if min == max {
        if min == 0.0 {
            max = 1.0
        } else {
            let delta = min.abs() * 0.1;
            min -= delta;
            max += delta;
        }
    }
    let raw = (max - min) / 5.0;
    let base = 10.0_f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|v| v * base)
        .find(|v| *v >= raw)
        .unwrap_or(raw);
    let parse = |s: &Option<String>| {
        s.as_ref()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let min = parse(&axis.minimum).unwrap_or((min / step).floor() * step);
    let max = parse(&axis.maximum).unwrap_or((max / step).ceil() * step);
    let mut step = parse(&axis.major_unit).unwrap_or(step);
    if !(max > min && step > 0.0 && (max - min).is_finite()) {
        issues.push("invalid axis scale".into());
        return Scale {
            min: 0.0,
            max: 1.0,
            step: 0.2,
            reverse: axis.reverse,
        };
    }
    if (max - min) / step > 200.0 {
        issues.push("axis tick limit".into());
        step = (max - min) / 5.0;
    }
    Scale {
        min,
        max,
        step,
        reverse: axis.reverse,
    }
}
fn label(value: f64, format: Option<&str>, dates: DateSystem) -> String {
    number_format::format_value(
        &CellValue::Number(if value.abs() < 1e-12 { 0.0 } else { value }),
        format,
        dates,
    )
    .text
}
fn text(
    ops: &mut Vec<ChartOp>,
    text: String,
    position: (f64, f64),
    align: &str,
    width: f64,
    bold: bool,
    options: &LayoutOptions,
) {
    let (x, y) = position;
    let mut font = ResolvedFont {
        size_points: if bold { 12.0 } else { 9.0 },
        bold,
        ..options.default_font.clone()
    };
    let measured = options.measure.measure(&text, &font);
    if measured > width && measured > 0.0 {
        font.size_points = (font.size_points as f64 * width / measured).max(6.0) as f32;
    }
    let advance = options.measure.measure(&text, &font);
    let x = match align {
        "center" => x - advance / 2.0,
        "right" => x - advance,
        _ => x,
    };
    ops.push(ChartOp::Text {
        text,
        x,
        y,
        font,
        color: "#404040".into(),
        align: align.into(),
    });
}
fn path(
    ops: &mut Vec<ChartOp>,
    points: Vec<(f64, f64)>,
    closed: bool,
    fill: Option<String>,
    stroke: Option<String>,
    width: f64,
) {
    if !points.is_empty() {
        ops.push(ChartOp::Path {
            points,
            closed,
            fill,
            stroke,
            width,
        })
    }
}
fn line(ops: &mut Vec<ChartOp>, a: (f64, f64), b: (f64, f64), color: &str) {
    path(ops, vec![a, b], false, None, Some(color.into()), 1.0)
}

fn finish_chart(mut chart: PlacedChart) -> PlacedChart {
    // Axis labels at a zero crossing must remain above negative bars.
    let mut labels = Vec::new();
    chart.ops = std::mem::take(&mut chart.ops)
        .into_iter()
        .filter_map(|op| {
            if matches!(op, ChartOp::Text { .. }) {
                labels.push(op);
                None
            } else {
                Some(op)
            }
        })
        .collect();
    chart.ops.extend(labels);
    chart
}
fn marker(ops: &mut Vec<ChartOp>, x: f64, y: f64, size: f64, symbol: &str, color: &str) {
    let r = size / 2.0;
    if matches!(symbol, "plus" | "x" | "star" | "dash") {
        if matches!(symbol, "plus" | "star" | "dash") {
            line(ops, (x - r, y), (x + r, y), color);
        }
        if matches!(symbol, "plus" | "star") {
            line(ops, (x, y - r), (x, y + r), color);
        }
        if matches!(symbol, "x" | "star") {
            line(ops, (x - r, y - r), (x + r, y + r), color);
            line(ops, (x - r, y + r), (x + r, y - r), color);
        }
        return;
    }
    let points = match symbol {
        "none" => return,
        "square" => vec![
            (x - r, y - r),
            (x + r, y - r),
            (x + r, y + r),
            (x - r, y + r),
        ],
        "diamond" => vec![(x, y - r), (x + r, y), (x, y + r), (x - r, y)],
        "triangle" => vec![(x, y - r), (x + r, y + r), (x - r, y + r)],
        _ => (0..20)
            .map(|i| {
                let a = i as f64 * std::f64::consts::TAU / 20.0;
                (x + r * a.cos(), y + r * a.sin())
            })
            .collect(),
    };
    path(
        ops,
        points,
        true,
        Some(color.into()),
        Some(color.into()),
        1.0,
    );
}

pub fn layout_chart(
    chart: &Chart,
    frame: Rect,
    sheet: &Sheet,
    workbook: Option<&Workbook>,
    options: &LayoutOptions,
    styles: &StyleTable,
) -> PlacedChart {
    let mut result = PlacedChart {
        plot: frame,
        ops: Vec::new(),
        unsupported_properties: chart.unsupported_properties.clone(),
        source_states: Vec::new(),
    };
    if !matches!(
        chart.kind.as_str(),
        "barChart" | "lineChart" | "areaChart" | "pieChart" | "doughnutChart" | "scatterChart"
    ) || chart
        .unsupported_properties
        .iter()
        .any(|v| v == "combined or missing plot")
    {
        return result;
    }
    let title_data = source(
        &chart.title_source,
        sheet,
        workbook,
        false,
        styles,
        options.date_system,
        true,
    );
    if chart.title_source.formula.is_some() {
        result.source_states.push(title_data.state.clone());
    }
    let title = if !chart.title.is_empty() {
        chart.title.clone()
    } else {
        title_data
            .points
            .first()
            .and_then(|v| v.clone())
            .unwrap_or_default()
    };
    let mut names = Vec::new();
    let mut categories = Vec::new();
    let mut values = Vec::new();
    let mut xs = Vec::new();
    let mut hidden = Vec::new();
    for series in &chart.series {
        let name = source(
            &series.name,
            sheet,
            workbook,
            false,
            styles,
            options.date_system,
            true,
        );
        let cat = source(
            &series.categories,
            sheet,
            workbook,
            chart.visible_only,
            styles,
            options.date_system,
            true,
        );
        let val = source(
            &series.values,
            sheet,
            workbook,
            chart.visible_only,
            styles,
            options.date_system,
            false,
        );
        let x = source(
            &series.x_values,
            sheet,
            workbook,
            chart.visible_only,
            styles,
            options.date_system,
            false,
        );
        names.push(
            name.points
                .first()
                .and_then(|v| v.clone())
                .unwrap_or_else(|| format!("Series {}", names.len() + 1)),
        );
        result
            .source_states
            .extend([name.state, cat.state, val.state, x.state]);
        let count = val.points.len().max(cat.points.len()).max(x.points.len());
        hidden.push(
            (0..count)
                .map(|i| {
                    cat.hidden.get(i) == Some(&true)
                        || val.hidden.get(i) == Some(&true)
                        || x.hidden.get(i) == Some(&true)
                })
                .collect::<Vec<_>>(),
        );
        categories.push(cat.points);
        values.push(
            val.points
                .iter()
                .map(|v| {
                    number(v.as_ref())
                        .or_else(|| (v.is_none() && chart.blank_mode == "zero").then_some(0.0))
                })
                .collect::<Vec<_>>(),
        );
        xs.push(
            x.points
                .iter()
                .map(|v| number(v.as_ref()))
                .collect::<Vec<_>>(),
        );
    }
    result.source_states.sort();
    result.source_states.dedup();
    if result.source_states.iter().any(|s| s == "cached") {
        result
            .unsupported_properties
            .push("unresolved chart formula; saved cache used".into());
    }
    if result
        .source_states
        .iter()
        .any(|s| s == "stale" || s == "uncalculated")
    {
        result
            .unsupported_properties
            .push("chart source formula requires recalculation".into());
    }
    let count = values.iter().map(Vec::len).max().unwrap_or(0);
    if count == 0 || chart.series.is_empty() {
        result.unsupported_properties.push("no chart values".into());
        return result;
    }
    let active: Vec<usize> = (0..count)
        .filter(|i| !hidden.iter().all(|v| v.get(*i) == Some(&true)))
        .collect();
    let ops = &mut result.ops;
    ops.push(ChartOp::Rectangle {
        rect: frame,
        fill: Some("#ffffff".into()),
        stroke: Some("#d9d9d9".into()),
    });
    let mut plot = Rect {
        x: frame.x + 52.0,
        y: frame.y + if title.is_empty() { 16.0 } else { 36.0 },
        width: (frame.width - 70.0).max(1.0),
        height: (frame.height - if title.is_empty() { 52.0 } else { 72.0 }).max(1.0),
    };
    if !title.is_empty() {
        text(
            ops,
            title,
            (frame.x + frame.width / 2.0, frame.y + 22.0),
            "center",
            (frame.width - 20.0).max(1.0),
            true,
            options,
        );
    }
    let pie = chart.kind == "pieChart" || chart.kind == "doughnutChart";
    let legend_names = if pie {
        (0..count)
            .map(|i| {
                categories[0]
                    .get(i)
                    .and_then(|v| v.clone())
                    .unwrap_or_else(|| (i + 1).to_string())
            })
            .collect::<Vec<_>>()
    } else {
        names.clone()
    };
    if let Some(position) = &chart.legend {
        let right = position == "r" || position == "l" || position == "tr";
        let legend_width = legend_names
            .iter()
            .map(|name| {
                options.measure.measure(
                    name,
                    &ResolvedFont {
                        size_points: 9.0,
                        ..options.default_font.clone()
                    },
                ) + 24.0
            })
            .fold(0.0, f64::max)
            .min(frame.width * 0.4);
        let (x, y) = if right {
            if position == "l" {
                plot.x += legend_width;
                plot.width = (plot.width - legend_width).max(1.0);
                (frame.x + 10.0, plot.y + 10.0)
            } else {
                plot.width = (plot.width - legend_width).max(1.0);
                (plot.right() + 10.0, plot.y + 10.0)
            }
        } else {
            plot.height = (plot.height - 24.0).max(1.0);
            (
                frame.x + 12.0,
                if position == "t" {
                    plot.y += 24.0;
                    plot.y - 14.0
                } else {
                    frame.bottom() - 12.0
                },
            )
        };
        let mut lx = x;
        let mut ly = y;
        for (i, name) in legend_names.iter().enumerate() {
            let color = if pie {
                point_color(chart, 0, i, chart.vary_colors)
            } else {
                series_color(chart, i)
            };
            ops.push(ChartOp::Rectangle {
                rect: Rect {
                    x: lx,
                    y: ly - 8.0,
                    width: 10.0,
                    height: 10.0,
                },
                fill: Some(color),
                stroke: None,
            });
            text(
                ops,
                name.clone(),
                (lx + 15.0, ly),
                "left",
                legend_width.max(40.0),
                false,
                options,
            );
            if right {
                ly += 18.0
            } else {
                lx += options.measure.measure(
                    name,
                    &ResolvedFont {
                        size_points: 9.0,
                        ..options.default_font.clone()
                    },
                ) + 30.0;
            }
        }
    }
    result.plot = plot;
    if pie {
        ops.push(ChartOp::Clip { rect: plot });
        let total: f64 = values[0]
            .iter()
            .enumerate()
            .filter(|(i, _)| hidden[0].get(*i) != Some(&true))
            .filter_map(|(_, v)| *v)
            .map(f64::abs)
            .sum();
        if total > 0.0 {
            let cx = plot.x + plot.width / 2.0;
            let cy = plot.y + plot.height / 2.0;
            let radius = plot.width.min(plot.height) / 2.0;
            let inner = if chart.kind == "doughnutChart" {
                radius * chart.hole_size as f64 / 100.0
            } else {
                0.0
            };
            let mut angle = (chart.first_slice_angle as f64 - 90.0).to_radians();
            for (i, value) in values[0].iter().enumerate() {
                if hidden[0].get(i) == Some(&true) {
                    continue;
                }
                let Some(value) = value else { continue };
                let span = value.abs() / total * std::f64::consts::TAU;
                let end = angle + span;
                let steps = (span.to_degrees() / 3.0).ceil().max(1.0) as usize;
                let mut points: Vec<(f64, f64)> = (0..=steps)
                    .map(|s| {
                        let a = angle + (end - angle) * s as f64 / steps as f64;
                        (cx + radius * a.cos(), cy + radius * a.sin())
                    })
                    .collect();
                if inner > 0.0 {
                    points.extend((0..=steps).rev().map(|s| {
                        let a = angle + (end - angle) * s as f64 / steps as f64;
                        (cx + inner * a.cos(), cy + inner * a.sin())
                    }));
                } else {
                    points.push((cx, cy));
                }
                path(
                    ops,
                    points,
                    true,
                    Some(point_color(chart, 0, i, chart.vary_colors)),
                    Some("#ffffff".into()),
                    1.0,
                );
                angle = end;
            }
        }
        ops.push(ChartOp::Restore);
        return finish_chart(result);
    }
    let stacked = chart.grouping == "stacked" || chart.grouping == "percentStacked";
    let percent = chart.grouping == "percentStacked";
    let mut lower = vec![vec![0.0; count]; values.len()];
    let mut upper = values.clone();
    if stacked {
        for i in 0..count {
            let total = values
                .iter()
                .enumerate()
                .filter(|(s, _)| hidden[*s].get(i) != Some(&true))
                .filter_map(|(_, v)| v.get(i).and_then(|v| *v))
                .map(f64::abs)
                .sum::<f64>();
            let mut pos = 0.0;
            let mut neg = 0.0;
            for s in 0..values.len() {
                if hidden[s].get(i) == Some(&true) {
                    if i < upper[s].len() {
                        upper[s][i] = None
                    }
                    continue;
                }
                if let Some(value) = values[s].get(i).and_then(|v| *v) {
                    let value = if percent && total > 0.0 {
                        value / total
                    } else {
                        value
                    };
                    let base = if value >= 0.0 { &mut pos } else { &mut neg };
                    lower[s][i] = *base;
                    *base += value;
                    if i < upper[s].len() {
                        upper[s][i] = Some(*base)
                    }
                }
            }
        }
    }
    let mut axis = chart.value_axis.clone();
    if percent {
        axis.minimum.get_or_insert(
            if values.iter().flatten().flatten().any(|v| *v < 0.0) {
                "-1"
            } else {
                "0"
            }
            .into(),
        );
        axis.maximum.get_or_insert(
            if values.iter().flatten().flatten().any(|v| *v > 0.0) {
                "1"
            } else {
                "0"
            }
            .into(),
        );
        axis.number_format.get_or_insert("0%".into());
    }
    let sy = scale(
        upper.iter().enumerate().flat_map(|(s, v)| {
            let hidden = &hidden;
            v.iter()
                .enumerate()
                .filter(move |(i, _)| hidden[s].get(*i) != Some(&true))
                .filter_map(|(_, v)| *v)
        }),
        &axis,
        true,
        &mut result.unsupported_properties,
    );
    let dates = workbook
        .map(|w| w.date_system)
        .unwrap_or(options.date_system);
    let y = |v: f64| {
        if chart.horizontal {
            plot.x + sy.fraction(v) * plot.width
        } else {
            plot.bottom() - sy.fraction(v) * plot.height
        }
    };
    for tick in sy.ticks() {
        let coordinate = y(tick);
        if !axis.hidden {
            if axis.gridlines {
                if chart.horizontal {
                    line(
                        ops,
                        (coordinate, plot.y),
                        (coordinate, plot.bottom()),
                        "#d9d9d9",
                    )
                } else {
                    line(
                        ops,
                        (plot.x, coordinate),
                        (plot.right(), coordinate),
                        "#d9d9d9",
                    )
                }
            }
            let format = axis
                .number_format
                .as_deref()
                .or(chart.series[0].values.format_code.as_deref());
            if chart.horizontal {
                text(
                    ops,
                    label(tick, format, dates),
                    (coordinate, plot.bottom() + 18.0),
                    "center",
                    45.0,
                    false,
                    options,
                )
            } else {
                text(
                    ops,
                    label(tick, format, dates),
                    (plot.x - 7.0, coordinate + 3.0),
                    "right",
                    45.0,
                    false,
                    options,
                )
            }
        }
    }
    let zero = y(0.0_f64.clamp(sy.min, sy.max));
    if !chart.category_axis.hidden {
        if chart.horizontal {
            line(ops, (zero, plot.y), (zero, plot.bottom()), "#808080");
        } else {
            line(ops, (plot.x, zero), (plot.right(), zero), "#808080");
        }
    }
    if !axis.hidden {
        if chart.horizontal {
            line(
                ops,
                (plot.x, plot.bottom()),
                (plot.right(), plot.bottom()),
                "#808080",
            );
        } else {
            line(ops, (plot.x, plot.y), (plot.x, plot.bottom()), "#808080");
        }
    }
    if chart.kind == "scatterChart" {
        let sx = scale(
            xs.iter().flatten().filter_map(|v| *v),
            &chart.category_axis,
            false,
            &mut result.unsupported_properties,
        );
        for tick in sx.ticks() {
            let x = plot.x + sx.fraction(tick) * plot.width;
            text(
                ops,
                label(tick, chart.category_axis.number_format.as_deref(), dates),
                (x, plot.bottom() + 18.0),
                "center",
                40.0,
                false,
                options,
            );
        }
        ops.push(ChartOp::Clip { rect: plot });
        for s in 0..values.len() {
            let color = series_color(chart, s);
            let mut segment = Vec::new();
            for (i, value) in values[s].iter().enumerate() {
                let point = (*value)
                    .zip(xs[s].get(i).and_then(|v| *v))
                    .filter(|_| hidden[s].get(i) != Some(&true));
                if let Some((value, x)) = point {
                    let p = (plot.x + sx.fraction(x) * plot.width, y(value));
                    segment.push(p);
                    marker(
                        ops,
                        p.0,
                        p.1,
                        chart.series[s].marker_size as f64,
                        &chart.series[s].marker,
                        &color,
                    );
                } else if chart.blank_mode != "span" {
                    if chart.scatter_style != "marker" {
                        path(
                            ops,
                            std::mem::take(&mut segment),
                            false,
                            None,
                            Some(color.clone()),
                            2.0,
                        )
                    } else {
                        segment.clear()
                    }
                }
            }
            if chart.scatter_style != "marker" {
                path(ops, segment, false, None, Some(color), 2.0)
            }
        }
        ops.push(ChartOp::Restore);
        return finish_chart(result);
    }
    let slots = active.len().max(1) as f64;
    let slot = if chart.horizontal {
        plot.height / slots
    } else {
        plot.width / slots
    };
    let category = |position: usize| {
        let pos = if chart.category_axis.reverse ^ chart.horizontal {
            active.len() - 1 - position
        } else {
            position
        };
        if chart.horizontal {
            plot.y + (pos as f64 + 0.5) * slot
        } else if axis.cross_between.as_deref() == Some("midCat")
            && chart.kind != "barChart"
            && active.len() > 1
        {
            plot.x + pos as f64 * plot.width / (active.len() - 1) as f64
        } else {
            plot.x + (pos as f64 + 0.5) * slot
        }
    };
    for (position, &i) in active.iter().enumerate() {
        if !chart.category_axis.hidden {
            let text_value = categories
                .iter()
                .find_map(|cat| cat.get(i).and_then(|v| v.clone()))
                .unwrap_or_else(|| (i + 1).to_string());
            if chart.horizontal {
                text(
                    ops,
                    text_value,
                    (zero - 7.0, category(position) + 3.0),
                    "right",
                    45.0,
                    false,
                    options,
                )
            } else {
                text(
                    ops,
                    text_value,
                    (category(position), zero + 18.0),
                    "center",
                    (slot - 4.0).max(1.0),
                    false,
                    options,
                )
            }
        }
    }
    ops.push(ChartOp::Clip { rect: plot });
    if chart.kind == "barChart" {
        let overlap = chart.overlap as f64 / 100.0;
        let n = if stacked { 1.0 } else { values.len() as f64 };
        let bar = slot / (chart.gap_width as f64 / 100.0 + n - (n - 1.0) * overlap);
        let group = bar * (n - (n - 1.0) * overlap);
        for s in 0..values.len() {
            for (position, &i) in active.iter().enumerate() {
                if hidden[s].get(i) == Some(&true) {
                    continue;
                }
                if let Some(value) = upper[s].get(i).and_then(|v| *v) {
                    let c = category(position) - group / 2.0
                        + if stacked {
                            0.0
                        } else {
                            s as f64 * bar * (1.0 - overlap)
                        };
                    let a = if stacked { y(lower[s][i]) } else { zero };
                    let b = y(value);
                    let rect = if chart.horizontal {
                        Rect {
                            x: a.min(b),
                            y: c,
                            width: (b - a).abs(),
                            height: bar,
                        }
                    } else {
                        Rect {
                            x: c,
                            y: a.min(b),
                            width: bar,
                            height: (b - a).abs(),
                        }
                    };
                    ops.push(ChartOp::Rectangle {
                        rect,
                        fill: Some(point_color(
                            chart,
                            s,
                            i,
                            chart.vary_colors && values.len() == 1,
                        )),
                        stroke: None,
                    });
                }
            }
        }
    } else {
        for s in 0..values.len() {
            let color = series_color(chart, s);
            let mut segment = Vec::new();
            let mut bases = Vec::new();
            let flush = |segment: &mut Vec<(f64, f64)>,
                         bases: &mut Vec<(f64, f64)>,
                         ops: &mut Vec<ChartOp>| {
                if chart.kind == "areaChart" {
                    let mut points = std::mem::take(segment);
                    points.extend(std::mem::take(bases).into_iter().rev());
                    path(
                        ops,
                        points,
                        true,
                        Some(color.clone()),
                        Some(color.clone()),
                        1.0,
                    )
                } else {
                    path(
                        ops,
                        std::mem::take(segment),
                        false,
                        None,
                        Some(color.clone()),
                        2.0,
                    );
                    bases.clear();
                }
            };
            for (position, &i) in active.iter().enumerate() {
                let value = upper[s]
                    .get(i)
                    .and_then(|v| *v)
                    .filter(|_| hidden[s].get(i) != Some(&true));
                if let Some(value) = value {
                    let p = (category(position), y(value));
                    segment.push(p);
                    bases.push((p.0, if stacked { y(lower[s][i]) } else { zero }));
                    marker(
                        ops,
                        p.0,
                        p.1,
                        chart.series[s].marker_size as f64,
                        &chart.series[s].marker,
                        &color,
                    );
                } else if chart.blank_mode != "span" {
                    flush(&mut segment, &mut bases, ops)
                }
            }
            flush(&mut segment, &mut bases, ops);
        }
    }
    ops.push(ChartOp::Restore);
    finish_chart(result)
}
pub fn chart_status(workbook: &Workbook) -> Vec<ChartStatus> {
    workbook
        .sheets
        .iter()
        .enumerate()
        .flat_map(|(sheet_index, sheet)| {
            sheet.shapes.iter().filter_map(move |shape| {
                shape.chart.as_ref().map(|chart| {
                    let placed = layout_chart(
                        chart,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 400.0,
                            height: 300.0,
                        },
                        sheet,
                        Some(workbook),
                        &LayoutOptions::default(),
                        &workbook.styles,
                    );
                    ChartStatus {
                        sheet: sheet.name.clone(),
                        sheet_index,
                        name: shape.name.clone(),
                        part: chart.part_name.clone(),
                        kind: chart.kind.clone(),
                        rendered: !placed.ops.is_empty(),
                        unsupported_properties: placed.unsupported_properties,
                        source_states: placed.source_states,
                    }
                })
            })
        })
        .collect()
}
