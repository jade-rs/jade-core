//! Chart data resolution and paint geometry shared by every XLSX renderer.
use crate::{number_format, LayoutOptions, Rect, ResolvedFont};
use jade_xlsx_reader::{
    chart::{
        resolve_chart_reference, Chart, ChartAxis, ChartColor, ChartData, ChartPointState,
        ChartSeries, DataLabels, ErrorBars, Trendline,
    },
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
        /// Dash pattern for the stroke; solid when empty.
        #[serde(default)]
        dash: Vec<f64>,
    },
    Text {
        text: String,
        x: f64,
        y: f64,
        font: ResolvedFont,
        color: String,
        align: String,
        /// Radians clockwise about (`x`, `y`): a value axis title reads
        /// upwards at -π/2.
        #[serde(default)]
        rotation: f64,
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
fn rgba(rgb: [u8; 3], alpha: f64) -> String {
    format!("rgba({}, {}, {}, {alpha})", rgb[0], rgb[1], rgb[2])
}
fn resolved_color(chart: &Chart, color: &ChartColor) -> Option<[u8; 3]> {
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
/// Excel's automatic series colours: the six accents, then the same accents
/// darkened, then lightened.
fn automatic(chart: &Chart, index: usize) -> [u8; 3] {
    let base = chart.palette[index % 6];
    let shade = |factor: f64| base.map(|c| (f64::from(c) * factor).round().min(255.0) as u8);
    let tint = |factor: f64| {
        base.map(|c| {
            (f64::from(c) + (255.0 - f64::from(c)) * factor)
                .round()
                .min(255.0) as u8
        })
    };
    match (index / 6) % 4 {
        0 => base,
        1 => shade(0.6),
        2 => tint(0.4),
        _ => shade(0.8),
    }
}
fn series_rgb(chart: &Chart, series: &ChartSeries, palette_index: usize) -> [u8; 3] {
    resolved_color(chart, &series.color).unwrap_or_else(|| automatic(chart, palette_index))
}
fn point_rgb(
    chart: &Chart,
    series: &ChartSeries,
    palette_index: usize,
    point: usize,
    vary: bool,
) -> [u8; 3] {
    series
        .point_colors
        .get(&point)
        .and_then(|c| resolved_color(chart, c))
        .unwrap_or_else(|| {
            if vary {
                automatic(chart, point)
            } else {
                series_rgb(chart, series, palette_index)
            }
        })
}
fn point_color(chart: &Chart, series: usize, index: usize, vary: bool) -> String {
    rgb(point_rgb(chart, &chart.series[series], series, index, vary))
}
#[derive(Clone, Copy)]
struct Scale {
    min: f64,
    max: f64,
    step: f64,
    reverse: bool,
    /// A logarithmic axis's base.
    log: Option<f64>,
}
impl Scale {
    fn fraction(self, value: f64) -> f64 {
        let f = match self.log {
            Some(base) => {
                let value = value.max(self.min * 1e-6);
                (value.log(base) - self.min.log(base)) / (self.max.log(base) - self.min.log(base))
            }
            None => (value - self.min) / (self.max - self.min),
        };
        if self.reverse {
            1.0 - f
        } else {
            f
        }
    }
    fn ticks(self) -> Vec<f64> {
        if let Some(base) = self.log {
            let mut ticks = Vec::new();
            let mut tick = self.min;
            while tick <= self.max * (1.0 + 1e-9) && ticks.len() < 200 {
                ticks.push(tick);
                tick *= base;
            }
            return ticks;
        }
        let first = (self.min / self.step).ceil() * self.step;
        let count = ((self.max - first) / self.step + 1e-9).floor().max(0.0) as usize;
        (0..=count.min(200))
            .map(|i| first + i as f64 * self.step)
            .collect()
    }
    /// Where bars and areas grow from: zero, or the axis edge nearer to it.
    fn base(self) -> f64 {
        if self.log.is_some() {
            self.min
        } else {
            0.0_f64.clamp(self.min, self.max)
        }
    }
}
fn parse_number(value: &Option<String>) -> Option<f64> {
    value
        .as_ref()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}
fn scale(
    values: impl Iterator<Item = f64>,
    axis: &ChartAxis,
    zero: bool,
    issues: &mut Vec<String>,
) -> Scale {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let log = parse_number(&axis.log_base).filter(|base| *base >= 2.0 && *base <= 1000.0);
    let mut values: Vec<f64> = values.collect();
    if log.is_some() {
        values.retain(|value| *value > 0.0);
    }
    for v in &values {
        min = min.min(*v);
        max = max.max(*v);
    }
    if let Some(base) = log {
        if !min.is_finite() {
            min = 1.0;
            max = base;
        }
        let low = base.powf(min.log(base).floor());
        let mut high = base.powf(max.log(base).ceil());
        if high <= low {
            high = low * base;
        }
        let min = parse_number(&axis.minimum)
            .filter(|v| *v > 0.0)
            .unwrap_or(low);
        let max = parse_number(&axis.maximum)
            .filter(|v| *v > min)
            .unwrap_or(high.max(min * base));
        return Scale {
            min,
            max,
            step: base,
            reverse: axis.reverse,
            log: Some(base),
        };
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
    let min = parse_number(&axis.minimum).unwrap_or((min / step).floor() * step);
    let max = parse_number(&axis.maximum).unwrap_or((max / step).ceil() * step);
    let mut step = parse_number(&axis.major_unit).unwrap_or(step);
    if !(max > min && step > 0.0 && (max - min).is_finite()) {
        issues.push("invalid axis scale".into());
        return Scale {
            min: 0.0,
            max: 1.0,
            step: 0.2,
            reverse: axis.reverse,
            log: None,
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
        log: None,
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
fn chart_font(options: &LayoutOptions, size: f32, bold: bool) -> ResolvedFont {
    ResolvedFont {
        size_points: size,
        bold,
        ..options.default_font.clone()
    }
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
    let mut font = chart_font(options, if bold { 12.0 } else { 9.0 }, bold);
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
        rotation: 0.0,
    });
}
/// Text turned to read upwards, centred on (`x`, `y`).
fn upright_text(ops: &mut Vec<ChartOp>, text: String, x: f64, y: f64, options: &LayoutOptions) {
    let font = chart_font(options, 10.0, false);
    let advance = options.measure.measure(&text, &font);
    ops.push(ChartOp::Text {
        text,
        x,
        y: y + advance / 2.0,
        font,
        color: "#595959".into(),
        align: "center".into(),
        rotation: -std::f64::consts::FRAC_PI_2,
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
            dash: Vec::new(),
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
        _ => circle(x, y, r, 20),
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
fn circle(x: f64, y: f64, r: f64, steps: usize) -> Vec<(f64, f64)> {
    (0..steps)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / steps as f64;
            (x + r * a.cos(), y + r * a.sin())
        })
        .collect()
}
/// A curve through `points` as Excel smooths a line: a Catmull-Rom spline,
/// sampled.
fn smooth(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        for step in 1..=10 {
            let t = step as f64 / 10.0;
            let (t2, t3) = (t * t, t * t * t);
            let at = |a: f64, b: f64, c: f64, d: f64| {
                0.5 * (2.0 * b
                    + (-a + c) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push((at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out
}

/// One plot's series with their data resolved.
struct PlotView<'a> {
    kind: &'a str,
    horizontal: bool,
    vary_colors: bool,
    grouping: &'a str,
    gap_width: u32,
    overlap: i32,
    scatter_style: &'a str,
    data_labels: Option<&'a DataLabels>,
    secondary: bool,
    series: Vec<SeriesView<'a>>,
}
struct SeriesView<'a> {
    spec: &'a ChartSeries,
    /// Index for automatic colours: the series' position in the chart.
    palette: usize,
    name: String,
    categories: Vec<Option<String>>,
    /// The categories as numbers, for a date axis.
    category_numbers: Vec<Option<f64>>,
    values: Vec<Option<f64>>,
    xs: Vec<Option<f64>>,
    sizes: Vec<Option<f64>>,
    hidden: Vec<bool>,
}
impl SeriesView<'_> {
    fn is_hidden(&self, point: usize) -> bool {
        self.hidden.get(point) == Some(&true)
    }
    fn value(&self, point: usize) -> Option<f64> {
        self.values
            .get(point)
            .copied()
            .flatten()
            .filter(|_| !self.is_hidden(point))
    }
}

/// Resolves every series of a plot against the workbook.
#[allow(clippy::too_many_arguments)]
fn view_series<'a>(
    chart: &Chart,
    series: &'a [ChartSeries],
    first_palette: usize,
    sheet: &Sheet,
    workbook: Option<&Workbook>,
    options: &LayoutOptions,
    styles: &StyleTable,
    states: &mut Vec<String>,
) -> Vec<SeriesView<'a>> {
    let mut views = Vec::new();
    for (offset, spec) in series.iter().enumerate() {
        let resolve = |data: &ChartData, visible: bool, format: bool| {
            source(
                data,
                sheet,
                workbook,
                visible && chart.visible_only,
                styles,
                options.date_system,
                format,
            )
        };
        let name = resolve(&spec.name, false, true);
        let cat = resolve(&spec.categories, true, true);
        let raw_cat = resolve(&spec.categories, true, false);
        let val = resolve(&spec.values, true, false);
        let x = resolve(&spec.x_values, true, false);
        let sizes = resolve(&spec.bubble_sizes, true, false);
        let palette = first_palette + offset;
        states.extend([
            name.state.clone(),
            cat.state.clone(),
            val.state.clone(),
            x.state.clone(),
        ]);
        let count = val.points.len().max(cat.points.len()).max(x.points.len());
        views.push(SeriesView {
            spec,
            palette,
            name: name
                .points
                .first()
                .and_then(|v| v.clone())
                .unwrap_or_else(|| format!("Series {}", palette + 1)),
            hidden: (0..count)
                .map(|i| {
                    cat.hidden.get(i) == Some(&true)
                        || val.hidden.get(i) == Some(&true)
                        || x.hidden.get(i) == Some(&true)
                })
                .collect(),
            categories: cat.points,
            category_numbers: raw_cat.points.iter().map(|v| number(v.as_ref())).collect(),
            values: val
                .points
                .iter()
                .map(|v| {
                    number(v.as_ref())
                        .or_else(|| (v.is_none() && chart.blank_mode == "zero").then_some(0.0))
                })
                .collect(),
            xs: x.points.iter().map(|v| number(v.as_ref())).collect(),
            sizes: sizes.points.iter().map(|v| number(v.as_ref())).collect(),
        });
    }
    views
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
    let drawable = |kind: &str| {
        matches!(
            kind,
            "barChart"
                | "lineChart"
                | "areaChart"
                | "pieChart"
                | "doughnutChart"
                | "scatterChart"
                | "radarChart"
                | "bubbleChart"
        )
    };
    if !drawable(&chart.kind)
        || chart.extra_plots.iter().any(|plot| !drawable(&plot.kind))
        || chart
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
    let mut states = Vec::new();
    let mut plots = vec![PlotView {
        kind: &chart.kind,
        horizontal: chart.horizontal,
        vary_colors: chart.vary_colors,
        grouping: &chart.grouping,
        gap_width: chart.gap_width,
        overlap: chart.overlap,
        scatter_style: if chart.kind == "radarChart" {
            &chart.radar_style
        } else {
            &chart.scatter_style
        },
        data_labels: chart.data_labels.as_ref(),
        secondary: false,
        series: view_series(
            chart,
            &chart.series,
            0,
            sheet,
            workbook,
            options,
            styles,
            &mut states,
        ),
    }];
    let mut palette = chart.series.len();
    for extra in &chart.extra_plots {
        plots.push(PlotView {
            kind: &extra.kind,
            horizontal: extra.horizontal,
            vary_colors: extra.vary_colors,
            grouping: &extra.grouping,
            gap_width: extra.gap_width,
            overlap: extra.overlap,
            scatter_style: &extra.scatter_style,
            data_labels: extra.data_labels.as_ref(),
            secondary: extra.secondary,
            series: view_series(
                chart,
                &extra.series,
                palette,
                sheet,
                workbook,
                options,
                styles,
                &mut states,
            ),
        });
        palette += extra.series.len();
    }
    result.source_states = states;
    if chart.title_source.formula.is_some() {
        result.source_states.push(title_data.state);
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
    let count = plots
        .iter()
        .flat_map(|plot| plot.series.iter())
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0);
    if count == 0 || chart.series.is_empty() {
        result.unsupported_properties.push("no chart values".into());
        return result;
    }
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
    let circular = matches!(chart.kind.as_str(), "pieChart" | "doughnutChart");
    let legend: Vec<(String, String)> = if circular {
        let series = &plots[0].series[0];
        (0..count)
            .map(|i| {
                (
                    series
                        .categories
                        .get(i)
                        .and_then(|v| v.clone())
                        .unwrap_or_else(|| (i + 1).to_string()),
                    point_color(chart, 0, i, chart.vary_colors),
                )
            })
            .collect()
    } else {
        plots
            .iter()
            .flat_map(|plot| plot.series.iter())
            .map(|series| {
                (
                    series.name.clone(),
                    rgb(series_rgb(chart, series.spec, series.palette)),
                )
            })
            .collect()
    };
    if let Some(position) = &chart.legend {
        place_legend(ops, &legend, position, frame, &mut plot, options);
    }
    result.plot = plot;
    if circular {
        layout_pie(ops, chart, &plots[0], plot, options);
        return finish_chart(result);
    }
    if chart.kind == "radarChart" {
        let mut issues = Vec::new();
        layout_radar(ops, chart, &plots[0], plot, options, &mut issues);
        result.unsupported_properties.extend(issues);
        return finish_chart(result);
    }
    let mut issues = Vec::new();
    let plot = layout_cartesian(ops, chart, &plots, plot, workbook, options, &mut issues);
    result.plot = plot;
    result.unsupported_properties.extend(issues);
    finish_chart(result)
}

fn place_legend(
    ops: &mut Vec<ChartOp>,
    entries: &[(String, String)],
    position: &str,
    frame: Rect,
    plot: &mut Rect,
    options: &LayoutOptions,
) {
    let font = chart_font(options, 9.0, false);
    let right = position == "r" || position == "l" || position == "tr";
    let legend_width = entries
        .iter()
        .map(|(name, _)| options.measure.measure(name, &font) + 24.0)
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
    for (name, color) in entries {
        ops.push(ChartOp::Rectangle {
            rect: Rect {
                x: lx,
                y: ly - 8.0,
                width: 10.0,
                height: 10.0,
            },
            fill: Some(color.clone()),
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
            lx += options.measure.measure(name, &font) + 30.0;
        }
    }
}

/// The labels a point shows, joined as Excel joins them.
fn label_text(
    labels: &DataLabels,
    series: &SeriesView<'_>,
    point: usize,
    value: f64,
    percent: Option<f64>,
    dates: DateSystem,
) -> Option<String> {
    if !labels.shows_anything() || labels.deleted.contains(&point) {
        return None;
    }
    let mut parts = Vec::new();
    if labels.show_series {
        parts.push(series.name.clone());
    }
    if labels.show_category {
        if let Some(category) = series.categories.get(point).cloned().flatten() {
            parts.push(category);
        } else if !series.xs.is_empty() {
            if let Some(x) = series.xs.get(point).copied().flatten() {
                parts.push(label(x, series.spec.x_values.format_code.as_deref(), dates));
            }
        } else {
            parts.push((point + 1).to_string());
        }
    }
    if labels.show_value {
        let format = labels
            .number_format
            .as_deref()
            .or(series.spec.values.format_code.as_deref());
        parts.push(label(value, format, dates));
    }
    if labels.show_percent {
        if let Some(percent) = percent {
            parts.push(label(percent, Some("0%"), dates));
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(labels.separator.as_deref().unwrap_or(", ")))
}
fn labels_for<'a>(plot: &'a PlotView<'_>, series: &'a SeriesView<'_>) -> Option<&'a DataLabels> {
    series.spec.data_labels.as_ref().or(plot.data_labels)
}
fn data_label(
    ops: &mut Vec<ChartOp>,
    text: String,
    x: f64,
    y: f64,
    align: &str,
    options: &LayoutOptions,
) {
    let font = chart_font(options, 9.0, false);
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
        rotation: 0.0,
    });
}

fn layout_pie(
    ops: &mut Vec<ChartOp>,
    chart: &Chart,
    view: &PlotView<'_>,
    plot: Rect,
    options: &LayoutOptions,
) {
    ops.push(ChartOp::Clip { rect: plot });
    let series = &view.series[0];
    let total: f64 = (0..series.values.len())
        .filter_map(|i| series.value(i))
        .map(f64::abs)
        .sum();
    let mut labels = Vec::new();
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
        for i in 0..series.values.len() {
            let Some(value) = series.value(i) else {
                continue;
            };
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
            if let Some(text) = labels_for(view, series).and_then(|labels| {
                label_text(
                    labels,
                    series,
                    i,
                    value,
                    Some(value.abs() / total),
                    options.date_system,
                )
            }) {
                let position = labels_for(view, series)
                    .and_then(|labels| labels.position.as_deref())
                    .unwrap_or("bestFit");
                let middle = angle + span / 2.0;
                let distance = match position {
                    "outEnd" => radius * 1.12,
                    "inEnd" => radius * 0.82,
                    "ctr" if inner > 0.0 => (radius + inner) / 2.0,
                    "ctr" => radius * 0.5,
                    _ if inner > 0.0 => (radius + inner) / 2.0,
                    _ => radius * 0.68,
                };
                labels.push((
                    text,
                    cx + distance * middle.cos(),
                    cy + distance * middle.sin() + 3.0,
                ));
            }
            angle = end;
        }
    }
    ops.push(ChartOp::Restore);
    for (text, x, y) in labels {
        data_label(ops, text, x, y, "center", options);
    }
}

fn layout_radar(
    ops: &mut Vec<ChartOp>,
    chart: &Chart,
    view: &PlotView<'_>,
    plot: Rect,
    options: &LayoutOptions,
    issues: &mut Vec<String>,
) {
    let count = view
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0);
    if count == 0 {
        return;
    }
    let scale = scale(
        view.series
            .iter()
            .flat_map(|series| (0..series.values.len()).filter_map(|i| series.value(i))),
        &chart.value_axis,
        true,
        issues,
    );
    let cx = plot.x + plot.width / 2.0;
    let cy = plot.y + plot.height / 2.0;
    let radius = (plot.width.min(plot.height) / 2.0 - 14.0).max(4.0);
    let angle =
        |i: usize| -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / count as f64;
    let at = |i: usize, value: f64| {
        let r = radius * scale.fraction(value).clamp(0.0, 1.0);
        (cx + r * angle(i).cos(), cy + r * angle(i).sin())
    };
    let dates = options.date_system;
    // The web: one ring per tick, a spoke per category.
    for tick in scale.ticks() {
        let ring: Vec<(f64, f64)> = (0..count).map(|i| at(i, tick)).collect();
        path(ops, ring, true, None, Some("#d9d9d9".into()), 1.0);
        if !chart.value_axis.hidden {
            let (x, y) = at(0, tick);
            text(
                ops,
                label(
                    tick,
                    chart
                        .value_axis
                        .number_format
                        .as_deref()
                        .or(chart.series[0].values.format_code.as_deref()),
                    dates,
                ),
                (x - 4.0, y + 3.0),
                "right",
                45.0,
                false,
                options,
            );
        }
    }
    for i in 0..count {
        line(ops, (cx, cy), at(i, scale.max), "#d9d9d9");
        if !chart.category_axis.hidden {
            let category = view
                .series
                .iter()
                .find_map(|series| series.categories.get(i).cloned().flatten())
                .unwrap_or_else(|| (i + 1).to_string());
            let a = angle(i);
            let (x, y) = (
                cx + (radius + 8.0) * a.cos(),
                cy + (radius + 12.0) * a.sin() + 3.0,
            );
            let align = if a.cos() > 0.2 {
                "left"
            } else if a.cos() < -0.2 {
                "right"
            } else {
                "center"
            };
            text(ops, category, (x, y), align, 80.0, false, options);
        }
    }
    let filled = view.scatter_style == "filled";
    let mut labels = Vec::new();
    for series in &view.series {
        let color = series_rgb(chart, series.spec, series.palette);
        let points: Vec<(f64, f64)> = (0..count)
            .map(|i| at(i, series.value(i).unwrap_or(scale.min.max(0.0))))
            .collect();
        if filled {
            path(
                ops,
                points.clone(),
                true,
                Some(rgba(color, 0.85)),
                Some(rgb(color)),
                1.0,
            );
        } else {
            path(ops, points.clone(), true, None, Some(rgb(color)), 2.0);
        }
        if !filled && view.scatter_style == "marker" {
            for (i, (x, y)) in points.iter().enumerate() {
                if series.value(i).is_some() {
                    marker(
                        ops,
                        *x,
                        *y,
                        series.spec.marker_size as f64,
                        if series.spec.marker == "none" {
                            "circle"
                        } else {
                            &series.spec.marker
                        },
                        &rgb(color),
                    );
                }
            }
        }
        if let Some(labels_spec) = labels_for(view, series) {
            for (i, (x, y)) in points.iter().enumerate() {
                if let Some(value) = series.value(i) {
                    if let Some(text) = label_text(labels_spec, series, i, value, None, dates) {
                        labels.push((text, *x, *y - 6.0));
                    }
                }
            }
        }
    }
    for (text, x, y) in labels {
        data_label(ops, text, x, y, "center", options);
    }
}

/// Where categories sit along the category axis.
struct Categories {
    /// For each point index, its slot (fractional for a date axis).
    slots: Vec<Option<f64>>,
    /// How many slots the axis spans.
    count: f64,
    /// The labels to draw: slot and text.
    labels: Vec<(f64, String)>,
    /// Points sit on the slots' edges rather than their middles.
    between: bool,
}

fn date_categories(
    view: &[&SeriesView<'_>],
    axis: &ChartAxis,
    format: Option<&str>,
    count: usize,
    dates: DateSystem,
) -> Option<Categories> {
    let series = view
        .iter()
        .find(|series| series.category_numbers.iter().any(|value| value.is_some()))?;
    let numbers: Vec<Option<f64>> = (0..count)
        .map(|i| series.category_numbers.get(i).copied().flatten())
        .collect();
    let present: Vec<f64> = numbers.iter().flatten().copied().collect();
    let min = present.iter().copied().fold(f64::INFINITY, f64::min);
    let max = present.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || max - min > 1e6 {
        return None;
    }
    let ymd = |serial: f64| {
        let civil = number_format::serial_to_civil(serial.floor(), dates)?;
        Some(civil)
    };
    let months_unit = match axis.base_time_unit.as_deref() {
        Some("months") => true,
        Some("years") => false,
        Some(_) => false,
        // Monthly data: every date the first of its month.
        None => present.len() > 1 && present.iter().all(|v| ymd(*v).is_some_and(|d| d.2 == 1)),
    };
    let years_unit = axis.base_time_unit.as_deref() == Some("years");
    let index = |serial: f64| -> Option<f64> {
        if years_unit || months_unit {
            let (y, m, _) = ymd(serial)?;
            let (y0, m0, _) = ymd(min)?;
            Some(if years_unit {
                f64::from(y - y0)
            } else {
                f64::from((y - y0) * 12 + (m as i32 - m0 as i32))
            })
        } else {
            Some((serial.floor() - min.floor()).max(0.0))
        }
    };
    let slots: Vec<Option<f64>> = numbers.iter().map(|v| v.and_then(index)).collect();
    let total = slots.iter().flatten().copied().fold(0.0, f64::max) + 1.0;
    let major = parse_number(&axis.major_unit)
        .filter(|v| *v >= 1.0)
        .unwrap_or_else(|| (total / 8.0).ceil().max(1.0));
    let format = axis
        .number_format
        .as_deref()
        .or(format)
        .filter(|code| !code.eq_ignore_ascii_case("General"))
        .unwrap_or(if months_unit { "mmm-yy" } else { "m/d/yyyy" });
    let mut labels = Vec::new();
    let mut slot = 0.0;
    while slot < total && labels.len() < 200 {
        let serial = if months_unit || years_unit {
            let (y0, m0, _) = ymd(min)?;
            let months = slot as i32 * if years_unit { 12 } else { 1 };
            let total_months = y0 * 12 + (m0 as i32 - 1) + months;
            number_format::civil_to_serial(
                total_months / 12,
                (total_months % 12 + 1) as u32,
                1,
                dates,
            )?
        } else {
            min.floor() + slot
        };
        labels.push((slot, label(serial, Some(format), dates)));
        slot += major;
    }
    Some(Categories {
        slots,
        count: total,
        labels,
        between: false,
    })
}

/// The fit a trendline draws, as y for x.
enum Fit {
    Linear { slope: f64, intercept: f64 },
    Exponential { scale: f64, rate: f64 },
    Logarithmic { slope: f64, intercept: f64 },
    Power { scale: f64, power: f64 },
    Polynomial(Vec<f64>),
}
impl Fit {
    fn at(&self, x: f64) -> Option<f64> {
        let y = match self {
            Fit::Linear { slope, intercept } => slope * x + intercept,
            Fit::Exponential { scale, rate } => scale * (rate * x).exp(),
            Fit::Logarithmic { slope, intercept } => {
                if x <= 0.0 {
                    return None;
                }
                slope * x.ln() + intercept
            }
            Fit::Power { scale, power } => {
                if x <= 0.0 {
                    return None;
                }
                scale * x.powf(*power)
            }
            Fit::Polynomial(coefficients) => coefficients
                .iter()
                .enumerate()
                .map(|(power, c)| c * x.powi(power as i32))
                .sum(),
        };
        y.is_finite().then_some(y)
    }
    fn equation(&self) -> String {
        let n = |v: f64| {
            let text = format!("{:.4}", v);
            let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
            if text == "-0" {
                "0".into()
            } else {
                text
            }
        };
        let signed = |v: f64| {
            if v < 0.0 {
                format!(" - {}", n(-v))
            } else {
                format!(" + {}", n(v))
            }
        };
        match self {
            Fit::Linear { slope, intercept } => format!("y = {}x{}", n(*slope), signed(*intercept)),
            Fit::Exponential { scale, rate } => format!("y = {}e^{}x", n(*scale), n(*rate)),
            Fit::Logarithmic { slope, intercept } => {
                format!("y = {}ln(x){}", n(*slope), signed(*intercept))
            }
            Fit::Power { scale, power } => format!("y = {}x^{}", n(*scale), n(*power)),
            Fit::Polynomial(c) => {
                let mut text = String::from("y =");
                for (power, coefficient) in c.iter().enumerate().rev() {
                    let term = match power {
                        0 => String::new(),
                        1 => "x".into(),
                        p => format!("x^{p}"),
                    };
                    if power == c.len() - 1 {
                        text.push_str(&format!(" {}{term}", n(*coefficient)));
                    } else {
                        text.push_str(&format!("{}{term}", signed(*coefficient)));
                    }
                }
                text
            }
        }
    }
}
#[allow(clippy::needless_range_loop)]
fn least_squares(points: &[(f64, f64)], order: usize) -> Option<Vec<f64>> {
    let size = order + 1;
    if points.len() < size {
        return None;
    }
    let mut matrix = vec![vec![0.0; size + 1]; size];
    for (x, y) in points {
        for row in 0..size {
            for column in 0..size {
                matrix[row][column] += x.powi((row + column) as i32);
            }
            matrix[row][size] += y * x.powi(row as i32);
        }
    }
    for column in 0..size {
        let pivot = (column..size).max_by(|a, b| {
            matrix[*a][column]
                .abs()
                .total_cmp(&matrix[*b][column].abs())
        })?;
        matrix.swap(column, pivot);
        let lead = matrix[column][column];
        if lead.abs() < 1e-12 {
            return None;
        }
        for row in 0..size {
            if row != column {
                let factor = matrix[row][column] / lead;
                for k in column..=size {
                    matrix[row][k] -= factor * matrix[column][k];
                }
            }
        }
    }
    Some(
        (0..size)
            .map(|row| matrix[row][size] / matrix[row][row])
            .collect(),
    )
}
fn fit(trendline: &Trendline, points: &[(f64, f64)]) -> Option<Fit> {
    let transformed = |fx: &dyn Fn(f64) -> Option<f64>, fy: &dyn Fn(f64) -> Option<f64>| {
        points
            .iter()
            .filter_map(|(x, y)| Some((fx(*x)?, fy(*y)?)))
            .collect::<Vec<_>>()
    };
    let positive = |v: f64| (v > 0.0).then(|| v.ln());
    let same = |v: f64| Some(v);
    match trendline.kind.as_str() {
        "linear" => {
            let c = least_squares(points, 1)?;
            Some(Fit::Linear {
                slope: c[1],
                intercept: c[0],
            })
        }
        "exp" => {
            let c = least_squares(&transformed(&same, &positive), 1)?;
            Some(Fit::Exponential {
                scale: c[0].exp(),
                rate: c[1],
            })
        }
        "log" => {
            let c = least_squares(&transformed(&positive, &same), 1)?;
            Some(Fit::Logarithmic {
                slope: c[1],
                intercept: c[0],
            })
        }
        "power" => {
            let c = least_squares(&transformed(&positive, &positive), 1)?;
            Some(Fit::Power {
                scale: c[0].exp(),
                power: c[1],
            })
        }
        "poly" => least_squares(points, trendline.order as usize).map(Fit::Polynomial),
        _ => None,
    }
}
fn r_squared(fit: &Fit, points: &[(f64, f64)]) -> Option<f64> {
    let mean = points.iter().map(|(_, y)| y).sum::<f64>() / points.len() as f64;
    let total: f64 = points.iter().map(|(_, y)| (y - mean).powi(2)).sum();
    let residual: f64 = points
        .iter()
        .map(|(x, y)| Some((y - fit.at(*x)?).powi(2)))
        .sum::<Option<f64>>()?;
    (total > 0.0).then(|| 1.0 - residual / total)
}

/// How far an error bar reaches each way from a point's value.
fn error_amounts(
    bars: &ErrorBars,
    values: &[Option<f64>],
    point: usize,
    plus_data: &[Option<f64>],
    minus_data: &[Option<f64>],
) -> Option<(f64, f64, Option<f64>)> {
    let value = values.get(point).copied().flatten()?;
    let fixed = parse_number(&bars.value).unwrap_or(1.0);
    let present: Vec<f64> = values.iter().flatten().copied().collect();
    let deviation = || {
        let n = present.len() as f64;
        if n < 2.0 {
            return 0.0;
        }
        let mean = present.iter().sum::<f64>() / n;
        (present.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    };
    let (plus, minus, center) = match bars.value_type.as_str() {
        "percentage" => {
            let amount = value.abs() * fixed / 100.0;
            (amount, amount, None)
        }
        // Standard deviation bars stand about the series' mean, not the point.
        "stdDev" => {
            let amount = deviation() * fixed;
            let mean = present.iter().sum::<f64>() / present.len().max(1) as f64;
            (amount, amount, Some(mean))
        }
        "stdErr" => {
            let amount = deviation() / (present.len().max(1) as f64).sqrt();
            (amount, amount, None)
        }
        "cust" => (
            plus_data.get(point).copied().flatten().unwrap_or(0.0),
            minus_data.get(point).copied().flatten().unwrap_or(0.0),
            None,
        ),
        _ => (fixed, fixed, None),
    };
    let (plus, minus) = match bars.bar_type.as_str() {
        "plus" => (plus, 0.0),
        "minus" => (0.0, minus),
        _ => (plus, minus),
    };
    Some((plus, minus, center))
}

#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn layout_cartesian(
    ops: &mut Vec<ChartOp>,
    chart: &Chart,
    plots: &[PlotView<'_>],
    mut plot: Rect,
    workbook: Option<&Workbook>,
    options: &LayoutOptions,
    issues: &mut Vec<String>,
) -> Rect {
    let dates = workbook
        .map(|w| w.date_system)
        .unwrap_or(options.date_system);
    let horizontal = plots[0].horizontal && plots[0].kind == "barChart";
    let xy = matches!(plots[0].kind, "scatterChart" | "bubbleChart");
    let secondary_axis = plots
        .iter()
        .any(|view| view.secondary)
        .then(|| chart.secondary_value_axis.clone().unwrap_or_default());
    // Axis titles take room from the plot.
    let value_title = chart
        .value_axis
        .title
        .clone()
        .filter(|_| !chart.value_axis.hidden);
    let category_title = chart
        .category_axis
        .title
        .clone()
        .filter(|_| !chart.category_axis.hidden);
    let (left_title, bottom_title) = if horizontal {
        (category_title.clone(), value_title.clone())
    } else {
        (value_title.clone(), category_title.clone())
    };
    if left_title.is_some() {
        plot.x += 18.0;
        plot.width = (plot.width - 18.0).max(1.0);
    }
    if bottom_title.is_some() {
        plot.height = (plot.height - 18.0).max(1.0);
    }
    let secondary_title = secondary_axis
        .as_ref()
        .and_then(|axis| axis.title.clone().filter(|_| !axis.hidden));
    if secondary_axis.as_ref().is_some_and(|axis| !axis.hidden) {
        plot.width = (plot.width - 40.0).max(1.0);
    }
    if secondary_title.is_some() {
        plot.width = (plot.width - 18.0).max(1.0);
    }
    if let Some(title) = left_title {
        upright_text(
            ops,
            title,
            plot.x - 52.0,
            plot.y + plot.height / 2.0,
            options,
        );
    }
    if let Some(title) = bottom_title {
        let font = chart_font(options, 10.0, false);
        let advance = options.measure.measure(&title, &font);
        ops.push(ChartOp::Text {
            text: title,
            x: plot.x + plot.width / 2.0 - advance / 2.0,
            y: plot.bottom() + 34.0,
            font,
            color: "#595959".into(),
            align: "center".into(),
            rotation: 0.0,
        });
    }
    if let Some(title) = secondary_title {
        upright_text(
            ops,
            title,
            plot.right() + 58.0,
            plot.y + plot.height / 2.0,
            options,
        );
    }

    // Stacks per plot, and the values each axis must show.
    struct Stacked {
        lower: Vec<Vec<f64>>,
        upper: Vec<Vec<Option<f64>>>,
    }
    let count = plots
        .iter()
        .flat_map(|view| view.series.iter())
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0);
    let stacks: Vec<Stacked> = plots
        .iter()
        .map(|view| {
            let stacked = matches!(view.grouping, "stacked" | "percentStacked");
            let percent = view.grouping == "percentStacked";
            let mut lower = vec![vec![0.0; count]; view.series.len()];
            let mut upper: Vec<Vec<Option<f64>>> = view
                .series
                .iter()
                .map(|series| (0..count).map(|i| series.value(i)).collect())
                .collect();
            if stacked && !matches!(view.kind, "scatterChart" | "bubbleChart") {
                for i in 0..count {
                    let total = view
                        .series
                        .iter()
                        .filter_map(|series| series.value(i))
                        .map(f64::abs)
                        .sum::<f64>();
                    let mut pos = 0.0;
                    let mut neg = 0.0;
                    for (s, series) in view.series.iter().enumerate() {
                        upper[s][i] = None;
                        if let Some(value) = series.value(i) {
                            let value = if percent && total > 0.0 {
                                value / total
                            } else {
                                value
                            };
                            let base = if value >= 0.0 { &mut pos } else { &mut neg };
                            lower[s][i] = *base;
                            *base += value;
                            upper[s][i] = Some(*base);
                        }
                    }
                }
            }
            Stacked { lower, upper }
        })
        .collect();
    let axis_for = |secondary: bool| {
        let mut axis = if secondary {
            secondary_axis.clone().unwrap_or_default()
        } else {
            chart.value_axis.clone()
        };
        let percent = plots
            .iter()
            .any(|view| view.secondary == secondary && view.grouping == "percentStacked");
        if percent {
            let all = || {
                plots
                    .iter()
                    .filter(|view| view.secondary == secondary)
                    .flat_map(|view| view.series.iter())
                    .flat_map(|series| series.values.iter().flatten())
            };
            axis.minimum
                .get_or_insert(if all().any(|v| *v < 0.0) { "-1" } else { "0" }.into());
            axis.maximum
                .get_or_insert(if all().any(|v| *v > 0.0) { "1" } else { "0" }.into());
            axis.number_format.get_or_insert("0%".into());
        }
        axis
    };
    // A bubble reaches past its centre: up to an eighth of the plot.
    let bubble_reach = |series: &SeriesView<'_>, i: usize| -> f64 {
        let largest = series.sizes.iter().flatten().copied().fold(0.0, f64::max);
        let size = series.sizes.get(i).copied().flatten().unwrap_or(0.0);
        if largest <= 0.0 || size <= 0.0 {
            return 0.0;
        }
        let share = if chart.bubble_size_width {
            size / largest
        } else {
            (size / largest).sqrt()
        };
        share * 0.125 * chart.bubble_scale as f64 / 100.0
    };
    let spread = |values: &[f64]| {
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if high > low {
            high - low
        } else {
            low.abs().max(1.0)
        }
    };
    let axis_values = |secondary: bool| -> Vec<f64> {
        let mut values = Vec::new();
        for (view, stack) in plots.iter().zip(&stacks) {
            if view.secondary != secondary {
                continue;
            }
            for (s, series) in view.series.iter().enumerate() {
                for i in 0..count {
                    if let Some(v) = stack.upper[s].get(i).copied().flatten() {
                        values.push(v);
                    }
                    // Error bars reach beyond their points.
                    for bars in &series.spec.error_bars {
                        if bars.direction == "x" {
                            continue;
                        }
                        if let Some((plus, minus, center)) =
                            error_amounts(bars, &series.values, i, &[], &[])
                        {
                            if bars.value_type == "cust" {
                                continue;
                            }
                            let at = center.unwrap_or_else(|| series.values[i].unwrap_or(0.0));
                            values.extend([at + plus, at - minus]);
                        }
                    }
                }
            }
        }
        if plots
            .iter()
            .any(|view| view.kind == "bubbleChart" && view.secondary == secondary)
        {
            let range = spread(&values);
            for view in plots.iter().filter(|view| view.kind == "bubbleChart") {
                for series in &view.series {
                    for i in 0..series.values.len() {
                        if let Some(v) = series.value(i) {
                            let reach = bubble_reach(series, i) * range * 1.6;
                            values.extend([v - reach, v + reach]);
                        }
                    }
                }
            }
        }
        values
    };
    let primary_axis = axis_for(false);
    let sy = scale(axis_values(false).into_iter(), &primary_axis, true, issues);
    let secondary_scale = secondary_axis.as_ref().map(|_| {
        let axis = axis_for(true);
        (
            scale(axis_values(true).into_iter(), &axis, true, issues),
            axis,
        )
    });
    let to_axis = |scale: Scale, v: f64| {
        if horizontal {
            plot.x + scale.fraction(v) * plot.width
        } else {
            plot.bottom() - scale.fraction(v) * plot.height
        }
    };
    let y = |v: f64| to_axis(sy, v);
    // Value axis ticks, gridlines and labels.
    let value_format = primary_axis
        .number_format
        .clone()
        .or_else(|| chart.series[0].values.format_code.clone());
    for tick in sy.ticks() {
        let coordinate = y(tick);
        if primary_axis.hidden {
            continue;
        }
        if primary_axis.gridlines {
            if horizontal {
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
        let text_value = label(tick, value_format.as_deref(), dates);
        if horizontal {
            text(
                ops,
                text_value,
                (coordinate, plot.bottom() + 18.0),
                "center",
                45.0,
                false,
                options,
            )
        } else {
            text(
                ops,
                text_value,
                (plot.x - 7.0, coordinate + 3.0),
                "right",
                45.0,
                false,
                options,
            )
        }
    }
    if let Some((scale2, axis2)) = &secondary_scale {
        if !axis2.hidden {
            let format = axis2.number_format.clone().or_else(|| {
                plots
                    .iter()
                    .find(|view| view.secondary)
                    .and_then(|view| view.series.first())
                    .and_then(|series| series.spec.values.format_code.clone())
            });
            for tick in scale2.ticks() {
                let coordinate = to_axis(*scale2, tick);
                text(
                    ops,
                    label(tick, format.as_deref(), dates),
                    (plot.right() + 7.0, coordinate + 3.0),
                    "left",
                    40.0,
                    false,
                    options,
                );
            }
            line(
                ops,
                (plot.right(), plot.y),
                (plot.right(), plot.bottom()),
                "#808080",
            );
        }
    }
    let zero = y(sy.base());
    if !primary_axis.hidden {
        if horizontal {
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

    // The horizontal axis of an XY chart is a value axis too.
    let sx = xy.then(|| {
        let x_of = |series: &SeriesView<'_>, i: usize| {
            if series.is_hidden(i) {
                return None;
            }
            if series.xs.is_empty() {
                Some(i as f64 + 1.0)
            } else {
                series.xs.get(i).copied().flatten()
            }
        };
        let mut values: Vec<f64> = plots
            .iter()
            .flat_map(|view| view.series.iter())
            .flat_map(|series| (0..series.values.len()).filter_map(move |i| x_of(series, i)))
            .collect();
        let range = spread(&values);
        for view in plots.iter().filter(|view| view.kind == "bubbleChart") {
            for series in &view.series {
                for i in 0..series.values.len() {
                    if let Some(x) = x_of(series, i) {
                        let reach = bubble_reach(series, i) * range * 1.6;
                        values.extend([x - reach, x + reach]);
                    }
                }
            }
        }
        scale(values.into_iter(), &chart.category_axis, false, issues)
    });
    let x_of_value = |x: f64| plot.x + sx.map_or(0.0, |sx| sx.fraction(x)) * plot.width;
    if let Some(sx) = sx {
        for tick in sx.ticks() {
            if chart.category_axis.hidden {
                break;
            }
            let x = x_of_value(tick);
            if chart.category_axis.gridlines {
                line(ops, (x, plot.y), (x, plot.bottom()), "#d9d9d9");
            }
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
        if !chart.category_axis.hidden {
            line(ops, (plot.x, zero), (plot.right(), zero), "#808080");
        }
    }

    // Category positions, shared by every plot on the category axis.
    let all_series: Vec<&SeriesView<'_>> =
        plots.iter().flat_map(|view| view.series.iter()).collect();
    let categories = if xy {
        None
    } else {
        let date = if chart.category_axis.date {
            date_categories(
                &all_series,
                &chart.category_axis,
                all_series
                    .first()
                    .and_then(|series| series.spec.categories.format_code.as_deref()),
                count,
                dates,
            )
        } else {
            None
        };
        Some(date.unwrap_or_else(|| {
            let active: Vec<usize> = (0..count)
                .filter(|i| !all_series.iter().all(|series| series.is_hidden(*i)))
                .collect();
            let mut slots = vec![None; count];
            for (position, &i) in active.iter().enumerate() {
                slots[i] = Some(position as f64);
            }
            let labels = active
                .iter()
                .enumerate()
                .map(|(position, &i)| {
                    (
                        position as f64,
                        all_series
                            .iter()
                            .find_map(|series| series.categories.get(i).and_then(|v| v.clone()))
                            .unwrap_or_else(|| (i + 1).to_string()),
                    )
                })
                .collect();
            Categories {
                slots,
                count: active.len().max(1) as f64,
                labels,
                between: primary_axis.cross_between.as_deref() == Some("midCat")
                    && !plots.iter().any(|view| view.kind == "barChart")
                    && active.len() > 1,
            }
        }))
    };
    let slot = categories.as_ref().map_or(1.0, |c| {
        if horizontal {
            plot.height / c.count
        } else {
            plot.width / c.count
        }
    });
    let category_at = |slot_index: f64| -> f64 {
        let Some(c) = categories.as_ref() else {
            return x_of_value(slot_index + 1.0);
        };
        let reversed = chart.category_axis.reverse ^ horizontal;
        let pos = if reversed {
            c.count - 1.0 - slot_index
        } else {
            slot_index
        };
        if horizontal {
            plot.y + (pos + 0.5) * slot
        } else if c.between {
            plot.x + pos * plot.width / (c.count - 1.0)
        } else {
            plot.x + (pos + 0.5) * slot
        }
    };
    if let Some(c) = categories.as_ref() {
        if !chart.category_axis.hidden {
            let width = if chart.category_axis.date && c.labels.len() > 1 {
                (c.labels[1].0 - c.labels[0].0) * slot
            } else {
                slot
            };
            for (slot_index, text_value) in &c.labels {
                if horizontal {
                    text(
                        ops,
                        text_value.clone(),
                        (zero - 7.0, category_at(*slot_index) + 3.0),
                        "right",
                        45.0,
                        false,
                        options,
                    )
                } else {
                    text(
                        ops,
                        text_value.clone(),
                        (category_at(*slot_index), zero + 18.0),
                        "center",
                        (width - 4.0).max(1.0),
                        false,
                        options,
                    )
                }
            }
            if horizontal {
                line(ops, (zero, plot.y), (zero, plot.bottom()), "#808080");
            } else {
                line(ops, (plot.x, zero), (plot.right(), zero), "#808080");
            }
        }
    }

    // Where each point of a series sits across the plot.
    let point_x = |series: &SeriesView<'_>, i: usize| -> Option<f64> {
        if xy {
            let x = if series.xs.is_empty() {
                Some(i as f64 + 1.0)
            } else {
                series.xs.get(i).copied().flatten()
            }?;
            return Some(x_of_value(x));
        }
        let slot_index = categories.as_ref()?.slots.get(i).copied().flatten()?;
        Some(category_at(slot_index))
    };

    // Draw areas, then bars, then lines and points over them, as Excel does.
    let rank = |kind: &str| match kind {
        "areaChart" => 0,
        "barChart" => 1,
        "bubbleChart" => 2,
        _ => 3,
    };
    let mut order: Vec<usize> = (0..plots.len()).collect();
    order.sort_by_key(|&index| rank(plots[index].kind));
    let mut labels: Vec<(String, f64, f64, &'static str)> = Vec::new();
    let mut trend_labels: Vec<(String, f64, f64)> = Vec::new();
    ops.push(ChartOp::Clip { rect: plot });
    for index in order {
        let view = &plots[index];
        let stack = &stacks[index];
        let scale = if view.secondary {
            secondary_scale
                .as_ref()
                .map(|(scale, _)| *scale)
                .unwrap_or(sy)
        } else {
            sy
        };
        let value_at = |v: f64| to_axis(scale, v);
        let base = value_at(scale.base());
        let stacked = matches!(view.grouping, "stacked" | "percentStacked");
        match view.kind {
            "barChart" => {
                let overlap = view.overlap as f64 / 100.0;
                let n = if stacked {
                    1.0
                } else {
                    view.series.len() as f64
                };
                let bar = slot / (view.gap_width as f64 / 100.0 + n - (n - 1.0) * overlap);
                let group = bar * (n - (n - 1.0) * overlap);
                for (s, series) in view.series.iter().enumerate() {
                    for i in 0..count {
                        let Some(value) = stack.upper[s].get(i).copied().flatten() else {
                            continue;
                        };
                        let Some(center) = point_x(series, i) else {
                            continue;
                        };
                        let c = center - group / 2.0
                            + if stacked {
                                0.0
                            } else {
                                s as f64 * bar * (1.0 - overlap)
                            };
                        let a = if stacked {
                            value_at(stack.lower[s][i])
                        } else {
                            base
                        };
                        let b = value_at(value);
                        let rect = if horizontal {
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
                            fill: Some(rgb(point_rgb(
                                chart,
                                series.spec,
                                series.palette,
                                i,
                                view.vary_colors && view.series.len() == 1,
                            ))),
                            stroke: None,
                        });
                        if let Some(text) = labels_for(view, series).and_then(|spec| {
                            label_text(spec, series, i, series.values[i]?, None, dates)
                        }) {
                            let position = labels_for(view, series)
                                .and_then(|spec| spec.position.as_deref())
                                .unwrap_or(if stacked { "ctr" } else { "outEnd" });
                            let negative = b > a && !horizontal || b < a && horizontal;
                            let (along, across) = (c + bar / 2.0, (a, b));
                            let (start, end) = across;
                            let point = match position {
                                "ctr" => (start + end) / 2.0,
                                "inBase" => start + (end - start).signum() * 8.0,
                                "inEnd" => end - (end - start).signum() * 8.0,
                                // outEnd: past the bar's end.
                                _ => {
                                    end + (end - start).signum()
                                        * if horizontal { 4.0 } else { 6.0 }
                                        + if (end - start).abs() < 1e-9 {
                                            -6.0
                                        } else {
                                            0.0
                                        }
                                }
                            };
                            if horizontal {
                                let align = if position == "outEnd"
                                    || !matches!(position, "ctr" | "inBase" | "inEnd")
                                {
                                    if negative {
                                        "right"
                                    } else {
                                        "left"
                                    }
                                } else {
                                    "center"
                                };
                                labels.push((text, point, along + 3.0, align));
                            } else {
                                let baseline = if position == "outEnd"
                                    || !matches!(position, "ctr" | "inBase" | "inEnd")
                                {
                                    if negative {
                                        point + 8.0
                                    } else {
                                        point
                                    }
                                } else {
                                    point + 3.0
                                };
                                labels.push((text, along, baseline, "center"));
                            }
                        }
                    }
                }
            }
            "bubbleChart" => {
                let sizes: Vec<f64> = view
                    .series
                    .iter()
                    .flat_map(|series| series.sizes.iter().flatten().copied())
                    .filter(|size| *size > 0.0)
                    .collect();
                let largest = sizes.iter().copied().fold(0.0, f64::max);
                let max_radius =
                    plot.width.min(plot.height) * 0.125 * chart.bubble_scale as f64 / 100.0;
                for series in &view.series {
                    let color = series_rgb(chart, series.spec, series.palette);
                    for i in 0..series.values.len() {
                        let (Some(value), Some(x)) = (series.value(i), point_x(series, i)) else {
                            continue;
                        };
                        let size = series.sizes.get(i).copied().flatten().unwrap_or(0.0);
                        if size <= 0.0 || largest <= 0.0 {
                            continue;
                        }
                        let r = if chart.bubble_size_width {
                            max_radius * size / largest
                        } else {
                            max_radius * (size / largest).sqrt()
                        };
                        let y = value_at(value);
                        path(
                            ops,
                            circle(x, y, r.max(1.0), 48),
                            true,
                            Some(rgba(color, 0.75)),
                            Some(rgb(color)),
                            1.0,
                        );
                        if let Some(text) = labels_for(view, series)
                            .and_then(|spec| label_text(spec, series, i, value, None, dates))
                        {
                            labels.push((text, x, y + 3.0, "center"));
                        }
                    }
                }
            }
            _ => {
                let area = view.kind == "areaChart";
                let lines = match view.kind {
                    "scatterChart" => !matches!(view.scatter_style, "marker"),
                    _ => true,
                };
                for (s, series) in view.series.iter().enumerate() {
                    let color = rgb(series_rgb(chart, series.spec, series.palette));
                    let smooth_line = series.spec.smooth
                        || (view.kind == "scatterChart"
                            && view.scatter_style.starts_with("smooth"));
                    let mut segment: Vec<(f64, f64)> = Vec::new();
                    let mut bases: Vec<(f64, f64)> = Vec::new();
                    let flush = |segment: &mut Vec<(f64, f64)>,
                                 bases: &mut Vec<(f64, f64)>,
                                 ops: &mut Vec<ChartOp>| {
                        if area {
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
                            let points = std::mem::take(segment);
                            if lines {
                                let points = if smooth_line { smooth(&points) } else { points };
                                path(ops, points, false, None, Some(color.clone()), 2.0);
                            }
                            bases.clear();
                        }
                    };
                    for i in 0..count.max(series.values.len()) {
                        let value = stack.upper[s].get(i).copied().flatten();
                        let at = value.zip(point_x(series, i));
                        if let Some((value, x)) = at {
                            let p = (x, value_at(value));
                            segment.push(p);
                            bases.push((
                                x,
                                if stacked {
                                    value_at(stack.lower[s][i])
                                } else {
                                    base
                                },
                            ));
                            marker(
                                ops,
                                p.0,
                                p.1,
                                series.spec.marker_size as f64,
                                &series.spec.marker,
                                &color,
                            );
                            if let Some(text) = labels_for(view, series).and_then(|spec| {
                                label_text(spec, series, i, series.values[i]?, None, dates)
                            }) {
                                let position = labels_for(view, series)
                                    .and_then(|spec| spec.position.as_deref())
                                    .unwrap_or(if area { "ctr" } else { "r" });
                                let (x, y, align) = match position {
                                    "t" => (p.0, p.1 - 6.0, "center"),
                                    "b" => (p.0, p.1 + 14.0, "center"),
                                    "l" => (p.0 - 6.0, p.1 + 3.0, "right"),
                                    "ctr" if area => (
                                        p.0,
                                        (p.1 + bases.last().map_or(p.1, |b| b.1)) / 2.0 + 3.0,
                                        "center",
                                    ),
                                    "ctr" => (p.0, p.1 + 3.0, "center"),
                                    _ => (p.0 + 6.0, p.1 + 3.0, "left"),
                                };
                                labels.push((text, x, y, align));
                            }
                        } else if chart.blank_mode != "span" {
                            flush(&mut segment, &mut bases, ops)
                        }
                    }
                    flush(&mut segment, &mut bases, ops);
                }
            }
        }
        // Trendlines and error bars over each series.
        for series in &view.series {
            let color = series_rgb(chart, series.spec, series.palette);
            let point_xs: Vec<Option<f64>> = (0..series.values.len())
                .map(|i| {
                    if xy {
                        if series.xs.is_empty() {
                            Some(i as f64 + 1.0)
                        } else {
                            series.xs.get(i).copied().flatten()
                        }
                    } else {
                        categories
                            .as_ref()
                            .and_then(|c| c.slots.get(i).copied().flatten())
                            .map(|slot| slot + 1.0)
                    }
                })
                .collect();
            let to_x = |x: f64| {
                if xy {
                    x_of_value(x)
                } else {
                    category_at(x - 1.0)
                }
            };
            for trendline in &series.spec.trendlines {
                let points: Vec<(f64, f64)> = (0..series.values.len())
                    .filter_map(|i| Some((point_xs[i]?, series.value(i)?)))
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                let stroke = rgb(resolved_color(chart, &trendline.color).unwrap_or(color));
                let screen: Vec<(f64, f64)>;
                let mut fitted = None;
                if trendline.kind == "movingAvg" {
                    let period = trendline.period as usize;
                    screen = points
                        .windows(period)
                        .map(|window| {
                            let (x, _) = window[period - 1];
                            let mean = window.iter().map(|(_, y)| y).sum::<f64>() / period as f64;
                            (to_x(x), value_at(mean))
                        })
                        .collect();
                } else {
                    let Some(found) = fit(trendline, &points) else {
                        continue;
                    };
                    let low = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min)
                        - parse_number(&trendline.backward).unwrap_or(0.0);
                    let high = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max)
                        + parse_number(&trendline.forward).unwrap_or(0.0);
                    let steps = if trendline.kind == "linear" { 1 } else { 60 };
                    screen = (0..=steps)
                        .filter_map(|step| {
                            let x = low + (high - low) * step as f64 / steps as f64;
                            Some((to_x(x), value_at(found.at(x)?)))
                        })
                        .filter(|(x, y)| x.is_finite() && y.is_finite())
                        .collect();
                    fitted = Some(found);
                }
                if let Some(&(x, y)) = screen.last() {
                    if let Some(found) = &fitted {
                        let mut lines = Vec::new();
                        if trendline.display_equation {
                            lines.push(found.equation());
                        }
                        if trendline.display_r_squared {
                            if let Some(r2) = r_squared(found, &points) {
                                lines.push(format!("R² = {:.4}", r2));
                            }
                        }
                        let above = lines.len();
                        for (k, text_line) in lines.into_iter().enumerate() {
                            let y = y - 8.0 - 12.0 * (above - 1 - k) as f64;
                            trend_labels.push((text_line, x - 4.0, y));
                        }
                    }
                }
                if screen.len() >= 2 {
                    ops.push(ChartOp::Path {
                        points: screen,
                        closed: false,
                        fill: None,
                        stroke: Some(stroke),
                        width: 1.5,
                        dash: vec![2.0, 2.0],
                    });
                }
            }
            for bars in &series.spec.error_bars {
                let resolve = |data: &ChartData| -> Vec<Option<f64>> {
                    data.points.iter().map(|v| number(v.as_ref())).collect()
                };
                let plus = resolve(&bars.plus);
                let minus = resolve(&bars.minus);
                let stroke = rgb(resolved_color(chart, &bars.color).unwrap_or([89, 89, 89]));
                let along_x = bars.direction == "x";
                for i in 0..series.values.len() {
                    if series.is_hidden(i) {
                        continue;
                    }
                    let Some((up, down, center)) =
                        error_amounts(bars, &series.values, i, &plus, &minus)
                    else {
                        continue;
                    };
                    let Some(x) = point_xs[i] else { continue };
                    let Some(v) = series.values[i] else { continue };
                    let cap = 3.0;
                    let mut segment = |a: (f64, f64), b: (f64, f64), vertical: bool| {
                        line(ops, a, b, &stroke);
                        if !bars.no_end_cap {
                            for end in [a, b] {
                                if (end.0 - a.0).abs() + (end.1 - a.1).abs() < 1e-9
                                    && (a.0 - b.0).abs() + (a.1 - b.1).abs() < 1e-9
                                {
                                    continue;
                                }
                                if vertical {
                                    line(ops, (end.0 - cap, end.1), (end.0 + cap, end.1), &stroke);
                                } else {
                                    line(ops, (end.0, end.1 - cap), (end.0, end.1 + cap), &stroke);
                                }
                            }
                        }
                    };
                    if along_x && xy {
                        let py = value_at(v);
                        let center = center.unwrap_or(x);
                        segment((to_x(center - down), py), (to_x(center + up), py), false);
                    } else {
                        let px = to_x(x);
                        let center = center.unwrap_or(v);
                        let (a, b) = (value_at(center - down), value_at(center + up));
                        if horizontal {
                            segment((a, px), (b, px), false);
                        } else {
                            segment((px, a), (px, b), true);
                        }
                    }
                }
            }
        }
    }
    ops.push(ChartOp::Restore);
    for (text, x, y, align) in labels {
        data_label(ops, text, x, y, align, options);
    }
    for (text, x, y) in trend_labels {
        data_label(ops, text, x, y, "right", options);
    }
    plot
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
