//! Charts as charts: columns, bars, lines, areas, pies, radars, scatters and
//! bubbles laid out from the cached series into placed shapes, so the
//! renderer needs nothing chart-specific.
//!
//! The plot follows what the part states — each plot element on its own
//! axes (a combo chart's line on a secondary value axis), the axes' scaling
//! (`c:min`, `c:max`, `c:majorUnit`, `c:orientation`) and number formats,
//! axis titles, gap width and overlap, markers, smoothed lines and data
//! labels — and PowerPoint's automatic choices where it states nothing.

use jade_pptx_reader::{
    Chart, ChartAxis, ChartMarker, ChartPlot, ChartSeries, Color, DataLabels, RunProps,
    TextAlign, TextBody, TextParagraph, TextRun,
};

use crate::number_format::format_chart_number;
use crate::style::text_context_for_table;
use crate::{
    resolve_color, resolve_fill, resolve_outline, text, Context, PathFill, PlacedGeometry,
    PlacedOrigin, PlacedOutline, PlacedPath, PlacedShape, PlacedSubpath, Rect, RunStyle,
    PX_PER_PT,
};

/// Room under the plot for category labels, in px.
const LABEL_HEIGHT_PX: f64 = 20.0;
/// Room above the plot, more when a title sits there.
const TOP_INSET_PX: f64 = 12.0;
const TOP_INSET_WITH_TITLE_PX: f64 = 28.0;
/// Horizontal insets as fractions of the frame width.
const LEFT_INSET: f64 = 0.08;
const RIGHT_INSET: f64 = 0.04;
const AXIS_COLOR: &str = "#808080";
const GRID_COLOR: &str = "#D9D9D9";
/// PowerPoint's chart text: dark grey, 10pt unless the part says.
const TEXT_COLOR: &str = "595959";
const DATA_LABEL_COLOR: &str = "404040";
const DEFAULT_TEXT_PT: f32 = 10.0;
/// A label's size when the chart states none, kept for the tick labels of
/// charts that carry no text properties at all.
const LABEL_SIZE_PT: f32 = 9.0;
const LINE_WIDTH_PX: f64 = 2.0;
/// `c:gapWidth`'s default, percent of a bar's width.
const DEFAULT_GAP: f64 = 150.0;
/// A marker's size when its series states none, in points.
const DEFAULT_MARKER_PT: f64 = 5.0;
/// The symbols `auto` markers cycle through, series by series.
const AUTO_MARKERS: [&str; 9] = [
    "diamond", "square", "triangle", "x", "star", "circle", "plus", "dash", "dot",
];
/// Degrees per sampled point along a pie sector's arc.
const PIE_STEP_DEG: f64 = 5.0;
/// Samples per segment of a smoothed line.
const CURVE_STEPS: usize = 12;
/// The largest bubble's diameter at `c:bubbleScale` 100, as a fraction of
/// the plot's shorter side.
const BUBBLE_FRACTION: f64 = 0.25;

/// How a piece of chart text draws.
#[derive(Clone)]
struct TextStyle {
    size_pt: f32,
    bold: bool,
    color: Color,
}

impl TextStyle {
    /// `props` over `self`.
    fn with(&self, props: Option<&RunProps>) -> TextStyle {
        let Some(props) = props else {
            return self.clone();
        };
        TextStyle {
            size_pt: props.size_pt.unwrap_or(self.size_pt),
            bold: props.bold.unwrap_or(self.bold),
            color: props.color.clone().unwrap_or_else(|| self.color.clone()),
        }
    }

    fn line_px(&self) -> f64 {
        self.size_pt as f64 * PX_PER_PT * 1.25
    }
}

/// What every part of the plot is built with.
struct Kit<'a> {
    chart: &'a Chart,
    origin: PlacedOrigin,
    context: &'a Context<'a>,
}

impl Kit<'_> {
    fn tag(&self, suffix: &str) -> String {
        format!("{}!{}", self.chart.name, suffix)
    }

    fn shape(&self, name: String, rect: Rect, geometry: PlacedGeometry) -> PlacedShape {
        PlacedShape {
            text_region: None,
            local_rect: None,
            group_transforms: Vec::new(),
            group_ids: Vec::new(),
            name,
            rect,
            rotation_rad: 0.0,
            flip_h: false,
            flip_v: false,
            geometry,
            fill: None,
            gradient: None,
            outline: None,
            image_part: None,
            source_rect: None,
            lines: Vec::new(),
            origin: self.origin,
            source_id: self.chart.id,
            text_rotation_deg: 0.0,
            shadow: None,
            image_alpha: None,
            pattern: None,
            effects: Default::default(),
        }
    }

    fn color(&self, color: &Color) -> String {
        resolve_color(color, self.context.theme)
    }

    fn accent(&self, index: usize) -> String {
        self.color(&Color::Scheme(format!("accent{}", index % 6 + 1)))
    }

    /// A series' colour: its own fill (or, for a line, its stroke), else
    /// the theme accent at its index.
    fn series_color(&self, index: usize, series: &ChartSeries, prefer_line: bool) -> String {
        let theme = self.context.theme;
        let line = series
            .line
            .as_ref()
            .filter(|line| !line.no_fill)
            .and_then(|line| line.color.as_ref())
            .map(|color| resolve_color(color, theme));
        let fill = series
            .fill
            .as_ref()
            .and_then(|fill| resolve_fill(fill, theme));
        if prefer_line {
            line.or(fill)
        } else {
            fill.or(line)
        }
        .unwrap_or_else(|| self.accent(index))
    }

    fn measure(&self, text: &str, style: &TextStyle) -> f64 {
        text.lines()
            .map(|line| {
                self.context.measure.measure(
                    line,
                    &RunStyle {
                        size_pt: style.size_pt,
                        bold: style.bold,
                        italic: false,
                        font: None,
                        east_asian_font: None,
                    },
                )
            })
            .fold(0.0, f64::max)
    }

    /// A text box holding `text`, unwrapped, aligned in `rect`.
    fn label(
        &self,
        name: String,
        rect: Rect,
        text: &str,
        align: TextAlign,
        style: &TextStyle,
    ) -> PlacedShape {
        self.label_turned(name, rect, text, align, style, 0.0)
    }

    /// [`Self::label`] with its text turned `degrees` (270 reads upwards).
    fn label_turned(
        &self,
        name: String,
        rect: Rect,
        text: &str,
        align: TextAlign,
        style: &TextStyle,
        degrees: f64,
    ) -> PlacedShape {
        let body = TextBody {
            insets: jade_pptx_reader::TextInsets {
                left_emu: Some(0),
                top_emu: Some(0),
                right_emu: Some(0),
                bottom_emu: Some(0),
            },
            anchor: Some(jade_pptx_reader::TextAnchor::Center),
            paragraphs: text
                .split('\n')
                .map(|line| TextParagraph {
                    align: Some(align),
                    runs: vec![TextRun {
                        text: line.to_string(),
                        size_pt: Some(style.size_pt),
                        bold: style.bold,
                        bold_stated: Some(style.bold),
                        color: Some(style.color.clone()),
                        ..TextRun::default()
                    }],
                    ..TextParagraph::default()
                })
                .collect(),
            ..TextBody::default()
        };
        let text_context = text_context_for_table(self.context);
        let (width, height) = if degrees == 0.0 {
            (rect.width, rect.height)
        } else {
            (rect.height, rect.width)
        };
        let (lines, _) = text::layout_text(&body, width, height, &text_context);
        let mut shape = self.shape(name, rect, PlacedGeometry::Rect);
        shape.lines = lines;
        shape.text_rotation_deg = degrees;
        shape
    }

    fn stroke(&self, color: &str, width_px: f64) -> PlacedOutline {
        PlacedOutline {
            color: color.to_string(),
            width_px,
            dash_px: Vec::new(),
            head_end: None,
            tail_end: None,
        }
    }

    /// A straight segment from `a` to `b` as a line shape.
    fn segment(&self, name: String, a: (f64, f64), b: (f64, f64), outline: PlacedOutline) -> PlacedShape {
        let mut segment = self.shape(
            name,
            Rect {
                x: a.0.min(b.0),
                y: a.1.min(b.1),
                width: (b.0 - a.0).abs(),
                height: (b.1 - a.1).abs(),
            },
            PlacedGeometry::Line,
        );
        // The line geometry runs top-left to bottom-right; a rising
        // segment is that, flipped vertically.
        segment.flip_v = (b.1 < a.1) != (b.0 < a.0);
        segment.outline = Some(outline);
        segment
    }

    /// An open polyline through `points` (px) as one stroked path.
    fn polyline(&self, name: String, points: &[(f64, f64)], outline: PlacedOutline, closed: bool) -> PlacedShape {
        let bounds = bounds_of(points);
        let normalized = normalize(points, bounds);
        let mut shape = self.shape(
            name,
            bounds,
            PlacedGeometry::Paths(vec![PlacedPath {
                subpaths: vec![PlacedSubpath {
                    points: normalized,
                    closed,
                }],
                fill: PathFill::None,
                stroked: true,
            }]),
        );
        shape.outline = Some(outline);
        shape
    }
}

/// The smallest rect around `points`, never zero-sized so normalizing
/// stays finite.
fn bounds_of(points: &[(f64, f64)]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in points {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    if x0 > x1 {
        return Rect::default();
    }
    Rect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(1e-6),
        height: (y1 - y0).max(1e-6),
    }
}

fn normalize(points: &[(f64, f64)], rect: Rect) -> Vec<(f64, f64)> {
    points
        .iter()
        .map(|(x, y)| ((x - rect.x) / rect.width, (y - rect.y) / rect.height))
        .collect()
}

/// A Catmull-Rom curve through `points`, as Office smooths a series'
/// line, flattened to short segments.
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
        // Bézier controls a sixth of the neighbours' span along the tangent.
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        for step in 1..=CURVE_STEPS {
            let t = step as f64 / CURVE_STEPS as f64;
            let u = 1.0 - t;
            let b = |a: f64, b: f64, c: f64, d: f64| {
                u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
            };
            out.push((b(p1.0, c1.0, c2.0, p2.0), b(p1.1, c1.1, c2.1, p2.1)));
        }
    }
    out
}

/// A round axis step near `rough`: 1, 2 or 5 times a power of ten.
fn nice_step(rough: f64) -> f64 {
    if !(rough.is_finite() && rough > 0.0) {
        return 1.0;
    }
    let power = 10f64.powf(rough.log10().floor());
    let fraction = rough / power;
    let nice = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * power
}

/// A tick's text when no number format applies: as many decimals as the
/// step needs.
fn format_tick(value: f64, step: f64, percent: bool) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        (-step.log10().floor()) as usize
    };
    let text = format!("{value:.decimals$}");
    let text = if text == "-0" { "0".to_string() } else { text };
    if percent {
        format!("{text}%")
    } else {
        text
    }
}

/// A value axis' scale: its ends, its step, and whether ticks are drawn.
#[derive(Clone, Copy)]
struct Scale {
    low: f64,
    high: f64,
    step: f64,
    reversed: bool,
}

impl Scale {
    /// Fits the data range `[low, high]` the way PowerPoint's automatic
    /// axis does, then lets the axis' own `c:min`/`c:max`/`c:majorUnit`
    /// win. `snap` rounds the ends to whole steps.
    fn fit(low: f64, high: f64, axis: Option<&ChartAxis>, snap: bool) -> Scale {
        let (mut low, mut high) = (low.min(high), low.max(high));
        if !low.is_finite() || !high.is_finite() {
            low = 0.0;
            high = 1.0;
        }
        // A range far from zero starts near its data, as Office's automatic
        // minimum does when the data's spread is under a sixth of its top.
        if low > 0.0 && (high - low) < high / 6.0 {
            low -= (high - low) / 2.0;
        } else if high < 0.0 && (high - low) < -low / 6.0 {
            high += (high - low) / 2.0;
        } else {
            low = low.min(0.0);
            high = high.max(0.0);
        }
        let min = axis.and_then(|axis| axis.min);
        let max = axis.and_then(|axis| axis.max);
        if let Some(min) = min {
            low = min;
        }
        if let Some(max) = max {
            high = max;
        }
        if high - low <= 0.0 {
            high = low + 1.0;
        }
        let step = axis
            .and_then(|axis| axis.major_unit)
            .unwrap_or_else(|| nice_step((high - low) / 5.0));
        if snap {
            if min.is_none() {
                low = (low / step).floor() * step;
            }
            if max.is_none() {
                high = (high / step).ceil() * step;
            }
        }
        Scale {
            low,
            high,
            step,
            reversed: axis.is_some_and(|axis| axis.reversed),
        }
    }

    /// 0 at the axis' start, 1 at its end.
    fn fraction(&self, value: f64) -> f64 {
        let f = (value - self.low) / (self.high - self.low);
        if self.reversed {
            1.0 - f
        } else {
            f
        }
    }

    fn ticks(&self) -> Vec<f64> {
        let count = ((self.high - self.low) / self.step + 1e-9).floor() as usize;
        (0..=count.min(100))
            .map(|i| self.low + self.step * i as f64)
            .collect()
    }
}

/// The plot for `chart` inside `frame`, in paint order.
pub(crate) fn plot_chart(
    chart: &Chart,
    frame: Rect,
    has_title: bool,
    context: &Context<'_>,
    origin: PlacedOrigin,
) -> Vec<PlacedShape> {
    let kit = Kit {
        chart,
        origin,
        context,
    };
    let kind = chart.kind.as_deref().unwrap_or("");
    let style = &chart.style;
    let base = TextStyle {
        size_pt: style
            .text
            .as_ref()
            .and_then(|text| text.size_pt)
            .unwrap_or(if style.plots.is_empty() {
                LABEL_SIZE_PT
            } else {
                DEFAULT_TEXT_PT
            }),
        bold: false,
        color: Color::Srgb(TEXT_COLOR.to_string()),
    }
    .with(style.text.as_ref());
    let top = if has_title {
        TOP_INSET_WITH_TITLE_PX.max(chart_title_height(chart) + 8.0)
    } else {
        TOP_INSET_PX
    };
    let line = base.line_px().max(LABEL_HEIGHT_PX);
    let mut plot = Rect {
        x: frame.x + frame.width * LEFT_INSET,
        y: frame.y + top,
        width: (frame.width * (1.0 - LEFT_INSET - RIGHT_INSET)).max(1.0),
        height: (frame.height - top - line).max(1.0),
    };
    let plots = effective_plots(chart);
    let is_pie = matches!(
        kind,
        "pieChart" | "pie3DChart" | "doughnutChart" | "ofPieChart"
    );
    let mut out = Vec::new();
    if style.legend.is_some() {
        place_legend(&kit, frame, &mut plot, is_pie, &plots, &base, &mut out);
    }
    match kind {
        _ if is_pie => {
            plot_pie(&kit, plot, &plots, &base, &mut out);
        }
        "radarChart" => plot_radar(&kit, plot, &plots, &base, &mut out),
        "scatterChart" | "bubbleChart" => plot_xy(&kit, frame, plot, &plots, &base, &mut out),
        "barChart" | "bar3DChart" | "lineChart" | "line3DChart" | "areaChart" | "area3DChart"
        | "stockChart" => plot_categories(&kit, frame, plot, &plots, &base, &mut out),
        _ => {}
    }
    out
}

/// The chart title's size and weight: its own run properties, else 1.2×
/// the chart's text (PowerPoint's title step), else 14pt bold.
pub(crate) fn title_style(chart: &Chart) -> (f32, bool, Option<Color>) {
    let style = &chart.style;
    let base = style.text.as_ref().and_then(|text| text.size_pt);
    let own = style.title_text.as_ref();
    let size = own
        .and_then(|text| text.size_pt)
        .or(base.map(|size| (size * 1.2 * 2.0).round() / 2.0))
        .unwrap_or(14.0);
    let bold = own
        .and_then(|text| text.bold)
        .or_else(|| style.text.as_ref().and_then(|text| text.bold))
        .unwrap_or(style.plots.is_empty());
    let color = own
        .and_then(|text| text.color.clone())
        .or_else(|| style.text.as_ref().and_then(|text| text.color.clone()))
        .or_else(|| (!style.plots.is_empty()).then(|| Color::Srgb(TEXT_COLOR.to_string())));
    (size, bold, color)
}

fn chart_title_height(chart: &Chart) -> f64 {
    let lines = chart.title.as_deref().map_or(1, |t| t.lines().count().max(1));
    title_style(chart).0 as f64 * PX_PER_PT * 1.2 * lines as f64
}

/// The plot elements to draw: the part's own, or, for a chart built in
/// code with none, one made from its kind and grouping.
fn effective_plots(chart: &Chart) -> Vec<ChartPlot> {
    if !chart.style.plots.is_empty() {
        return chart.style.plots.clone();
    }
    vec![ChartPlot {
        kind: chart.kind.clone().unwrap_or_default(),
        grouping: chart.style.grouping.clone(),
        horizontal: chart.horizontal,
        gap_width: Some(25.0),
        ..ChartPlot::default()
    }]
}

/// The series' kind: its own plot element's.
fn series_kind<'a>(series: &'a ChartSeries, plots: &'a [ChartPlot]) -> &'a str {
    plots
        .get(series.plot_index)
        .map(|plot| plot.kind.as_str())
        .filter(|kind| !kind.is_empty())
        .or(series.plot.as_deref())
        .unwrap_or("")
}

fn is_line_kind(kind: &str) -> bool {
    kind.starts_with("line") || kind.starts_with("radar") || kind == "stockChart"
}

#[allow(clippy::too_many_arguments)]
fn place_legend(
    kit: &Kit<'_>,
    frame: Rect,
    plot: &mut Rect,
    is_pie: bool,
    plots: &[ChartPlot],
    base: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    let chart = kit.chart;
    let position = chart.style.legend.as_deref().unwrap_or("r");
    let entries: Vec<(String, String)> = if is_pie {
        chart
            .series
            .first()
            .map(|series| {
                (0..series.values.len())
                    .map(|i| {
                        (
                            series
                                .categories
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| (i + 1).to_string()),
                            point_color(kit, series, i),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        chart
            .series
            .iter()
            .enumerate()
            .map(|(i, series)| {
                let line = is_line_kind(series_kind(series, plots));
                (
                    series
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Series {}", i + 1)),
                    kit.series_color(i, series, line),
                )
            })
            .collect()
    };
    let row = base.line_px().max(LABEL_HEIGHT_PX);
    let swatch = (base.size_pt as f64 * PX_PER_PT * 0.6).max(6.0);
    let place_entry = |index: usize, x: f64, y: f64, width: f64, out: &mut Vec<PlacedShape>| {
        let (name, color) = &entries[index];
        let mut key = kit.shape(
            kit.tag(&format!("legend key {name}")),
            Rect {
                x,
                y: y + (row - swatch) / 2.0,
                width: swatch,
                height: swatch,
            },
            PlacedGeometry::Rect,
        );
        key.fill = Some(color.clone());
        out.push(key);
        out.push(kit.label(
            kit.tag(&format!("legend {name}")),
            Rect {
                x: x + swatch + 4.0,
                y,
                width: (width - swatch - 4.0).max(1.0),
                height: row,
            },
            name,
            TextAlign::Left,
            base,
        ));
    };
    match position {
        "b" | "t" => {
            let band = row + 4.0;
            let y = if position == "b" {
                plot.height = (plot.height - band).max(1.0);
                frame.y + frame.height - row - 2.0
            } else {
                plot.y += band;
                plot.height = (plot.height - band).max(1.0);
                plot.y - band
            };
            // Entries as wide as their names, centred as a row.
            let widths: Vec<f64> = entries
                .iter()
                .map(|(name, _)| kit.measure(name, base) + swatch + 16.0)
                .collect();
            let total: f64 = widths.iter().sum();
            let scale = if total > frame.width * 0.95 {
                frame.width * 0.95 / total
            } else {
                1.0
            };
            let mut x = frame.x + (frame.width - total * scale) / 2.0;
            for (i, width) in widths.iter().enumerate() {
                place_entry(i, x, y, width * scale, out);
                x += width * scale;
            }
        }
        _ => {
            let widest = entries
                .iter()
                .map(|(name, _)| kit.measure(name, base))
                .fold(0.0, f64::max);
            let width = (widest + swatch + 16.0).clamp(40.0, frame.width * 0.35);
            let x = if position == "l" {
                plot.x += width;
                plot.width = (plot.width - width).max(1.0);
                frame.x + 4.0
            } else {
                plot.width = (plot.width - width).max(1.0);
                frame.x + frame.width - width
            };
            let top = if position == "tr" {
                plot.y
            } else {
                plot.y + (plot.height - row * entries.len() as f64).max(0.0) / 2.0
            };
            for i in 0..entries.len() {
                place_entry(i, x, top + i as f64 * row, width - 8.0, out);
            }
        }
    }
}

/// The labels a series shows: its own `c:dLbls`, else its plot's.
fn labels_for<'a>(series: &'a ChartSeries, plot: Option<&'a ChartPlot>) -> Option<&'a DataLabels> {
    series
        .labels
        .as_ref()
        .or_else(|| plot.and_then(|plot| plot.labels.as_ref()))
        .filter(|labels| labels.shows_anything())
}

/// One point's label text, in PowerPoint's order: series, category, value,
/// percentage, bubble size.
fn label_text(
    labels: &DataLabels,
    series: &ChartSeries,
    category: &str,
    value: f64,
    percent: Option<f64>,
    bubble: Option<f64>,
) -> String {
    let mut parts = Vec::new();
    if labels.show_series {
        if let Some(name) = &series.name {
            parts.push(name.clone());
        }
    }
    if labels.show_category {
        parts.push(category.to_string());
    }
    if labels.show_value {
        let code = labels
            .number_format
            .as_deref()
            .or(series.format_code.as_deref());
        parts.push(format_chart_number(value, code));
    }
    if labels.show_percent {
        if let Some(percent) = percent {
            let code = labels
                .number_format
                .as_deref()
                .filter(|code| code.contains('%'))
                .unwrap_or("0%");
            parts.push(format_chart_number(percent, Some(code)));
        }
    }
    if labels.show_bubble_size {
        if let Some(bubble) = bubble {
            parts.push(format_chart_number(bubble, None));
        }
    }
    let separator = labels.separator.as_deref().unwrap_or(", ");
    parts.join(separator)
}

/// Places a label box for `text` so its `anchor` side touches `at`:
/// `(0.5, 0.5)` centres it there, `(0.5, 1.0)` sets it above.
#[allow(clippy::too_many_arguments)]
fn place_label(
    kit: &Kit<'_>,
    name: String,
    text: &str,
    at: (f64, f64),
    anchor: (f64, f64),
    style: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    if text.is_empty() {
        return;
    }
    let width = kit.measure(text, style) + 4.0;
    let height = style.line_px() * text.lines().count().max(1) as f64;
    let rect = Rect {
        x: at.0 - anchor.0 * width,
        y: at.1 - anchor.1 * height,
        width,
        height,
    };
    out.push(kit.label(name, rect, text, TextAlign::Center, style));
}

fn label_style(labels: &DataLabels, base: &TextStyle) -> TextStyle {
    TextStyle {
        color: Color::Srgb(DATA_LABEL_COLOR.to_string()),
        ..base.clone()
    }
    .with(labels.text.as_ref())
}

/// A slice's or bar's colour: its data point's own fill, else the accent
/// cycle.
fn point_color(kit: &Kit<'_>, series: &ChartSeries, index: usize) -> String {
    series
        .point_fills
        .iter()
        .find(|(i, _)| *i == index)
        .and_then(|(_, fill)| resolve_fill(fill, kit.context.theme))
        .unwrap_or_else(|| kit.accent(index))
}

/// The marker a series draws at each point, if any: its own `c:marker`,
/// else automatic when its plot asks for markers.
fn marker_for(series: &ChartSeries, index: usize, plot_wants: bool) -> Option<ChartMarker> {
    let marker = match &series.marker {
        Some(marker) => marker.clone(),
        None if plot_wants => ChartMarker {
            symbol: "auto".to_string(),
            ..ChartMarker::default()
        },
        None => return None,
    };
    match marker.symbol.as_str() {
        "none" => None,
        "auto" => Some(ChartMarker {
            symbol: AUTO_MARKERS[index % AUTO_MARKERS.len()].to_string(),
            ..marker
        }),
        _ => Some(marker),
    }
}

/// One marker centred on `at`.
fn place_marker(
    kit: &Kit<'_>,
    name: String,
    marker: &ChartMarker,
    at: (f64, f64),
    color: &str,
    out: &mut Vec<PlacedShape>,
) {
    let size = marker.size.unwrap_or(DEFAULT_MARKER_PT).clamp(2.0, 72.0) * PX_PER_PT;
    let r = size / 2.0;
    let rect = Rect {
        x: at.0 - r,
        y: at.1 - r,
        width: size,
        height: size,
    };
    let theme = kit.context.theme;
    let fill = marker
        .fill
        .as_ref()
        .map(|fill| resolve_fill(fill, theme))
        .unwrap_or_else(|| Some(color.to_string()));
    let outline = match &marker.line {
        Some(line) => resolve_outline(Some(line), None, theme, None),
        None => Some(kit.stroke(color, 0.75 * PX_PER_PT)),
    };
    let strokes = |segments: &[[(f64, f64); 2]]| {
        PlacedGeometry::Paths(vec![PlacedPath {
            subpaths: segments
                .iter()
                .map(|[a, b]| PlacedSubpath {
                    points: vec![*a, *b],
                    closed: false,
                })
                .collect(),
            fill: PathFill::None,
            stroked: true,
        }])
    };
    let (geometry, filled) = match marker.symbol.as_str() {
        "circle" => (PlacedGeometry::Ellipse, true),
        "square" => (PlacedGeometry::Rect, true),
        "diamond" => (
            PlacedGeometry::Polygon(vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
            true,
        ),
        "triangle" => (
            PlacedGeometry::Polygon(vec![(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            true,
        ),
        "dash" => (
            PlacedGeometry::Polygon(vec![(0.0, 0.4), (1.0, 0.4), (1.0, 0.6), (0.0, 0.6)]),
            true,
        ),
        "dot" => (
            PlacedGeometry::Polygon(vec![(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]),
            true,
        ),
        "x" => (strokes(&[[(0.0, 0.0), (1.0, 1.0)], [(1.0, 0.0), (0.0, 1.0)]]), false),
        "plus" => (strokes(&[[(0.5, 0.0), (0.5, 1.0)], [(0.0, 0.5), (1.0, 0.5)]]), false),
        "star" => (
            strokes(&[
                [(0.0, 0.0), (1.0, 1.0)],
                [(1.0, 0.0), (0.0, 1.0)],
                [(0.5, 0.0), (0.5, 1.0)],
            ]),
            false,
        ),
        _ => (PlacedGeometry::Ellipse, true),
    };
    let mut shape = kit.shape(name, rect, geometry);
    if filled {
        shape.fill = fill;
    }
    shape.outline = outline.or_else(|| (!filled).then(|| kit.stroke(color, 1.0)));
    out.push(shape);
}

/// The series' stroke: its `a:ln` resolved, else a line in its colour;
/// `None` when the line is `noFill`.
fn series_stroke(kit: &Kit<'_>, series: &ChartSeries, color: &str) -> Option<PlacedOutline> {
    if series.line.as_ref().is_some_and(|line| line.no_fill) {
        return None;
    }
    let resolved = series
        .line
        .as_ref()
        .and_then(|line| resolve_outline(Some(line), None, kit.context.theme, None));
    Some(match resolved {
        Some(mut outline) => {
            if series.line.as_ref().and_then(|line| line.color.as_ref()).is_none() {
                outline.color = color.to_string();
            }
            outline
        }
        None => kit.stroke(color, LINE_WIDTH_PX),
    })
}

/// Columns, bars, lines and areas over a category axis, each plot element
/// on its own value axis.
fn plot_categories(
    kit: &Kit<'_>,
    frame: Rect,
    mut plot: Rect,
    plots: &[ChartPlot],
    base: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    let chart = kit.chart;
    let style = &chart.style;
    let categories = chart
        .series
        .iter()
        .map(|series| series.categories.len().max(series.values.len()))
        .max()
        .unwrap_or(0);
    if categories == 0 || chart.series.is_empty() {
        return;
    }
    let horizontal = plots.iter().any(|plot| plot.kind.starts_with("bar") && plot.horizontal);
    // The axes each plot uses: its category axis and its value axis. A
    // chart built without axes has one labelled the way its style says.
    let synthetic_value = ChartAxis {
        id: u32::MAX,
        kind: "valAx".into(),
        gridlines: style.gridlines,
        tick_label_position: (!style.value_labels).then(|| "none".to_string()),
        ..ChartAxis::default()
    };
    let axis_of = |plot: &ChartPlot, kinds: &[&str]| -> Option<&ChartAxis> {
        plot.axis_ids
            .iter()
            .filter_map(|id| style.axis(*id))
            .find(|axis| kinds.contains(&axis.kind.as_str()))
    };
    let value_axis = |plot: &ChartPlot| -> &ChartAxis {
        axis_of(plot, &["valAx"]).unwrap_or(&synthetic_value)
    };
    let category_axis = plots
        .first()
        .and_then(|plot| axis_of(plot, &["catAx", "dateAx"]));
    let primary_id = plots.first().map(|plot| value_axis(plot).id).unwrap_or(u32::MAX);
    // Value axes in the order first used: primary, then secondary.
    let mut axis_ids: Vec<u32> = Vec::new();
    for plot in plots {
        let id = value_axis(plot).id;
        if !axis_ids.contains(&id) {
            axis_ids.push(id);
        }
    }
    let value_at = |s: usize, c: usize| -> Option<f64> {
        chart.series[s]
            .values
            .get(c)
            .copied()
            .flatten()
            .filter(|v| v.is_finite())
    };
    let grouping_of = |plot: &ChartPlot| plot.grouping.as_deref().unwrap_or("clustered").to_string();
    // Each plot's per-category totals, for stacking and percentages.
    let plot_totals: Vec<Vec<(f64, f64)>> = plots
        .iter()
        .enumerate()
        .map(|(p, _)| {
            (0..categories)
                .map(|c| {
                    (0..chart.series.len())
                        .filter(|&s| chart.series[s].plot_index == p)
                        .fold((0.0, 0.0), |(pos, neg), s| match value_at(s, c) {
                            Some(v) if v >= 0.0 => (pos + v, neg),
                            Some(v) => (pos, neg + v),
                            None => (pos, neg),
                        })
                })
                .collect()
        })
        .collect();
    let percent_of = |p: usize, c: usize, v: f64| -> f64 {
        let (pos, neg) = plot_totals[p][c];
        let total = pos - neg;
        if total > 0.0 {
            v / total * 100.0
        } else {
            0.0
        }
    };
    // The data range each value axis has to show.
    let range_of = |id: u32| -> (f64, f64) {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for (p, plot) in plots.iter().enumerate() {
            if value_axis(plot).id != id {
                continue;
            }
            let grouping = grouping_of(plot);
            let stacked = matches!(grouping.as_str(), "stacked" | "percentStacked");
            if stacked {
                for c in 0..categories {
                    let (pos, neg) = plot_totals[p][c];
                    let (pos, neg) = if grouping == "percentStacked" {
                        (percent_of(p, c, pos), percent_of(p, c, neg))
                    } else {
                        (pos, neg)
                    };
                    low = low.min(neg);
                    high = high.max(pos);
                }
            } else {
                for (s, series) in chart.series.iter().enumerate() {
                    if series.plot_index != p {
                        continue;
                    }
                    for c in 0..categories {
                        if let Some(v) = value_at(s, c) {
                            low = low.min(v);
                            high = high.max(v);
                        }
                    }
                }
            }
        }
        if low > high {
            (0.0, 1.0)
        } else {
            (low, high)
        }
    };
    let axis_by_id = |id: u32| -> &ChartAxis {
        if id == u32::MAX {
            &synthetic_value
        } else {
            style.axis(id).unwrap_or(&synthetic_value)
        }
    };
    let scales: Vec<(u32, Scale)> = axis_ids
        .iter()
        .map(|&id| {
            let axis = axis_by_id(id);
            let (low, high) = range_of(id);
            let snap = axis.labelled() || axis.gridlines;
            (id, Scale::fit(low, high, Some(axis), snap))
        })
        .collect();
    let scale_of = |id: u32| -> Scale {
        scales
            .iter()
            .find(|(axis, _)| *axis == id)
            .map(|(_, scale)| *scale)
            .unwrap_or_else(|| Scale::fit(0.0, 1.0, None, false))
    };
    let percent_axis = |id: u32| {
        plots
            .iter()
            .any(|plot| value_axis(plot).id == id && plot.grouping.as_deref() == Some("percentStacked"))
    };
    // A value axis' tick text: its own format, the source's when linked,
    // else digits for the step.
    let tick_text = |id: u32, value: f64, scale: &Scale| -> String {
        let axis = axis_by_id(id);
        let source = chart
            .series
            .iter()
            .find(|series| plots.get(series.plot_index).is_some_and(|plot| value_axis(plot).id == id))
            .and_then(|series| series.format_code.as_deref());
        let code = match (&axis.number_format, axis.source_linked) {
            (Some(code), false) => Some(code.as_str()),
            (Some(code), true) => source.or(Some(code.as_str())),
            (None, _) => None,
        }
        .filter(|code| !code.eq_ignore_ascii_case("general"));
        if percent_axis(id) {
            return format_tick(value, scale.step, true);
        }
        match code {
            Some(code) => format_chart_number(value, Some(code)),
            None => format_tick(value, scale.step, false),
        }
    };
    let value_text_style = |id: u32| base.with(axis_by_id(id).text.as_ref());
    let category_style = base.with(category_axis.and_then(|axis| axis.text.as_ref()));
    let category_labels: Vec<String> = chart
        .series
        .iter()
        .max_by_key(|series| series.categories.len())
        .map(|series| series.categories.clone())
        .unwrap_or_default();
    let categories_labelled = category_axis.is_none_or(|axis| axis.labelled());

    // Room for tick labels and axis titles, measured from what they say.
    let side_room = |id: u32| -> f64 {
        let axis = axis_by_id(id);
        if !axis.labelled() {
            return 0.0;
        }
        let scale = scale_of(id);
        let text_style = value_text_style(id);
        scale
            .ticks()
            .iter()
            .map(|v| kit.measure(&tick_text(id, *v, &scale), &text_style))
            .fold(0.0, f64::max)
            + 8.0
    };
    let title_room = |axis: Option<&ChartAxis>, style: &TextStyle| -> f64 {
        axis.and_then(|axis| axis.title.as_ref())
            .map_or(0.0, |_| style.line_px() * 1.2 + 4.0)
    };
    let primary = axis_by_id(primary_id);
    let secondary_id = axis_ids.get(1).copied();
    let title_bold = TextStyle { bold: true, ..base.clone() };
    if !horizontal {
        let mut left = side_room(primary_id) + title_room(Some(primary), &title_bold);
        if primary_id == u32::MAX && style.value_labels {
            left = left.max((frame.width * 0.08).max(40.0));
        }
        let left_edge = frame.x + if style.plots.is_empty() {
            frame.width * LEFT_INSET
        } else {
            8.0
        };
        let x = (frame.x + left + 4.0).max(left_edge);
        plot.width = (plot.width - (x - plot.x)).max(1.0);
        plot.x = x;
        if let Some(id) = secondary_id {
            let right = side_room(id) + title_room(Some(axis_by_id(id)), &title_bold);
            plot.width = (plot.width - right).max(1.0);
        }
        let bottom_title = title_room(category_axis, &title_bold);
        plot.height = (plot.height - bottom_title).max(1.0);
    } else {
        let left = if categories_labelled {
            category_labels
                .iter()
                .map(|label| kit.measure(label, &category_style))
                .fold(0.0, f64::max)
                + 8.0
        } else {
            0.0
        } + title_room(category_axis, &title_bold);
        let x = (frame.x + left).max(plot.x).min(frame.x + frame.width * 0.45);
        plot.width = (plot.width - (x - plot.x)).max(1.0);
        plot.x = x;
        let bottom_title = title_room(Some(primary), &title_bold);
        plot.height = (plot.height - bottom_title).max(1.0);
        if primary.labelled() && primary.position.as_deref() == Some("t") {
            let band = value_text_style(primary_id).line_px().max(LABEL_HEIGHT_PX);
            plot.y += band;
            plot.height = (plot.height - band).max(1.0);
        }
    }

    // Value → px along a value axis; category → the slot it owns.
    let slot = if horizontal {
        plot.height / categories as f64
    } else {
        plot.width / categories as f64
    };
    let at = |scale: &Scale, value: f64| -> f64 {
        let fraction = scale.fraction(value);
        if horizontal {
            plot.x + fraction * plot.width
        } else {
            plot.y + plot.height - fraction * plot.height
        }
    };
    let category_reversed = category_axis.is_some_and(|axis| axis.reversed);
    // Bars list their first category at the bottom, as PowerPoint does;
    // columns run left to right. A reversed axis turns either round.
    let row = |c: usize| -> f64 {
        let from_start = if horizontal { categories - 1 - c } else { c };
        if category_reversed {
            (categories - 1 - from_start) as f64
        } else {
            from_start as f64
        }
    };

    // Gridlines and tick labels of each value axis.
    for (index, &id) in axis_ids.iter().enumerate() {
        let axis = axis_by_id(id);
        let scale = scale_of(id);
        let secondary = index > 0;
        let text_style = value_text_style(id);
        let labelled = axis.labelled();
        if !(labelled || axis.gridlines) {
            continue;
        }
        for (i, value) in scale.ticks().into_iter().enumerate() {
            let p = at(&scale, value);
            if axis.gridlines && !secondary {
                let (a, b) = if horizontal {
                    ((p, plot.y), (p, plot.y + plot.height))
                } else {
                    ((plot.x, p), (plot.x + plot.width, p))
                };
                let mut grid = kit.segment(
                    kit.tag(&format!("gridline {i}")),
                    a,
                    b,
                    kit.stroke(GRID_COLOR, 1.0),
                );
                grid.flip_v = false;
                out.push(grid);
            }
            if labelled {
                let text = tick_text(id, value, &scale);
                let height = text_style.line_px().max(LABEL_HEIGHT_PX);
                let (rect, align) = if horizontal {
                    let width = kit.measure(&text, &text_style) + 8.0;
                    let y = if secondary || axis.position.as_deref() == Some("t") {
                        plot.y - height
                    } else {
                        plot.y + plot.height
                    };
                    (
                        Rect {
                            x: p - width / 2.0,
                            y,
                            width,
                            height,
                        },
                        TextAlign::Center,
                    )
                } else if secondary {
                    (
                        Rect {
                            x: plot.x + plot.width + 4.0,
                            y: p - height / 2.0,
                            width: side_room(id).max(1.0),
                            height,
                        },
                        TextAlign::Left,
                    )
                } else {
                    let width = side_room(id).max(plot.x - frame.x - 4.0).max(1.0);
                    (
                        Rect {
                            x: plot.x - 4.0 - width,
                            y: p - height / 2.0,
                            width,
                            height,
                        },
                        TextAlign::Right,
                    )
                };
                let name = if secondary {
                    format!("secondary value {text}")
                } else {
                    format!("value {text}")
                };
                out.push(kit.label(kit.tag(&name), rect, &text, align, &text_style));
            }
        }
    }

    // Areas first, then bars, then lines and their markers, as PowerPoint
    // stacks chart types.
    let mut layers: [Vec<PlacedShape>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut labels_out: Vec<PlacedShape> = Vec::new();
    for (p, plot_element) in plots.iter().enumerate() {
        let id = value_axis(plot_element).id;
        let scale = scale_of(id);
        let grouping = grouping_of(plot_element);
        let stacked = matches!(grouping.as_str(), "stacked" | "percentStacked");
        let percent = grouping == "percentStacked";
        let zero = at(&scale, 0.0_f64.clamp(scale.low.min(scale.high), scale.high.max(scale.low)));
        let members: Vec<usize> = (0..chart.series.len())
            .filter(|&s| chart.series[s].plot_index == p)
            .collect();
        let scaled = |c: usize, v: f64| if percent { percent_of(p, c, v) } else { v };
        let mut stack_pos = vec![0.0f64; categories];
        let mut stack_neg = vec![0.0f64; categories];
        let kind = plot_element.kind.as_str();
        // Bar geometry: gap width and overlap as PowerPoint applies them.
        let bars = members.len().max(1) as f64;
        let gap = plot_element.gap_width.unwrap_or(DEFAULT_GAP).max(0.0) / 100.0;
        let overlap = if stacked {
            1.0
        } else {
            plot_element.overlap.unwrap_or(0.0).clamp(-100.0, 100.0) / 100.0
        };
        let lanes = if stacked { 1.0 } else { bars - (bars - 1.0) * overlap };
        let bar_width = slot / (lanes + gap);
        for (position, &s) in members.iter().enumerate() {
            let series = &chart.series[s];
            let series_name = series
                .name
                .clone()
                .unwrap_or_else(|| format!("series {}", s + 1));
            let series_kind = series_kind(series, plots);
            let kind = if series_kind.is_empty() { kind } else { series_kind };
            let is_line = is_line_kind(kind);
            let is_area = kind.starts_with("area");
            let color = kit.series_color(s, series, is_line);
            let labels = labels_for(series, Some(plot_element));
            let text_style = labels.map(|labels| label_style(labels, base));
            let category = |c: usize| {
                series
                    .categories
                    .get(c)
                    .or(category_labels.get(c))
                    .cloned()
                    .unwrap_or_else(|| (c + 1).to_string())
            };
            let center = |c: usize, value: f64| -> (f64, f64) {
                if horizontal {
                    (at(&scale, value), plot.y + (row(c) + 0.5) * slot)
                } else {
                    (plot.x + (row(c) + 0.5) * slot, at(&scale, value))
                }
            };
            if is_area {
                // One polygon from the lower edge up to the values and back.
                let mut top = Vec::new();
                let mut bottom = Vec::new();
                for c in 0..categories {
                    let v = scaled(c, value_at(s, c).unwrap_or(0.0));
                    let base_value = if stacked { stack_pos[c] } else { 0.0 };
                    let upper = base_value + v;
                    top.push(center(c, upper));
                    bottom.push(center(c, base_value));
                    if stacked {
                        stack_pos[c] = upper;
                    }
                    if let (Some(labels), Some(text_style), Some(raw)) =
                        (labels, text_style.as_ref(), value_at(s, c))
                    {
                        if !labels.hidden_points.contains(&c) {
                            let text = label_text(labels, series, &category(c), raw, None, None);
                            let mid = center(c, base_value + v / 2.0);
                            place_label(kit, kit.tag(&format!("{series_name} label {c}")), &text, mid, (0.5, 0.5), text_style, &mut labels_out);
                        }
                    }
                }
                let mut points: Vec<(f64, f64)> = top;
                points.extend(bottom.into_iter().rev());
                let normalized = points
                    .iter()
                    .map(|(x, y)| ((x - plot.x) / plot.width, (y - plot.y) / plot.height))
                    .collect();
                let mut area = kit.shape(
                    kit.tag(&format!("{series_name} area")),
                    plot,
                    PlacedGeometry::Polygon(normalized),
                );
                area.fill = Some(color.clone());
                layers[0].push(area);
                continue;
            }
            if is_line {
                let stroke = series_stroke(kit, series, &color);
                let marker = marker_for(series, s, plot_element.markers);
                let mut run: Vec<(usize, (f64, f64))> = Vec::new();
                let mut runs: Vec<Vec<(usize, (f64, f64))>> = Vec::new();
                for c in 0..categories {
                    let Some(raw) = value_at(s, c) else {
                        if !run.is_empty() {
                            runs.push(std::mem::take(&mut run));
                        }
                        continue;
                    };
                    let value = scaled(c, raw);
                    let value = if stacked {
                        let base_value = if value >= 0.0 { stack_pos[c] } else { stack_neg[c] };
                        if value >= 0.0 {
                            stack_pos[c] += value;
                        } else {
                            stack_neg[c] += value;
                        }
                        base_value + value
                    } else {
                        value
                    };
                    let point = center(c, value);
                    run.push((c, point));
                    if let (Some(labels), Some(text_style)) = (labels, text_style.as_ref()) {
                        if !labels.hidden_points.contains(&c) {
                            let text = label_text(labels, series, &category(c), raw, None, None);
                            let gap = 4.0 + marker.as_ref().map_or(0.0, |m| m.size.unwrap_or(DEFAULT_MARKER_PT) * PX_PER_PT / 2.0);
                            let (offset, anchor) = match labels.position.as_deref().unwrap_or("r") {
                                "t" => ((0.0, -gap), (0.5, 1.0)),
                                "b" => ((0.0, gap), (0.5, 0.0)),
                                "l" => ((-gap, 0.0), (1.0, 0.5)),
                                "ctr" => ((0.0, 0.0), (0.5, 0.5)),
                                _ => ((gap, 0.0), (0.0, 0.5)),
                            };
                            place_label(
                                kit,
                                kit.tag(&format!("{series_name} label {c}")),
                                &text,
                                (point.0 + offset.0, point.1 + offset.1),
                                anchor,
                                text_style,
                                &mut labels_out,
                            );
                        }
                    }
                }
                if !run.is_empty() {
                    runs.push(run);
                }
                if let Some(stroke) = stroke {
                    for run in &runs {
                        if series.smooth && run.len() > 2 {
                            let points: Vec<(f64, f64)> = run.iter().map(|(_, p)| *p).collect();
                            layers[2].push(kit.polyline(
                                kit.tag(&format!("{series_name} line")),
                                &smooth(&points),
                                stroke.clone(),
                                false,
                            ));
                        } else {
                            for pair in run.windows(2) {
                                let (_, start) = pair[0];
                                let (c, end) = pair[1];
                                layers[2].push(kit.segment(
                                    kit.tag(&format!("{series_name} {c}")),
                                    start,
                                    end,
                                    stroke.clone(),
                                ));
                            }
                        }
                    }
                }
                if let Some(marker) = &marker {
                    for (c, point) in runs.iter().flatten() {
                        place_marker(
                            kit,
                            kit.tag(&format!("{series_name} marker {c}")),
                            marker,
                            *point,
                            &color,
                            &mut layers[2],
                        );
                    }
                }
                continue;
            }
            // Bars.
            let horizontal_bars = horizontal;
            for c in 0..categories {
                let Some(raw) = value_at(s, c) else {
                    continue;
                };
                let value = scaled(c, raw);
                let (offset, start, end) = if stacked {
                    let base_value = if value >= 0.0 { stack_pos[c] } else { stack_neg[c] };
                    if value >= 0.0 {
                        stack_pos[c] += value;
                    } else {
                        stack_neg[c] += value;
                    }
                    (
                        gap * bar_width / 2.0,
                        at(&scale, base_value),
                        at(&scale, base_value + value),
                    )
                } else {
                    // Sideways bars stack their series bottom-up too.
                    let lane = if horizontal_bars != category_reversed {
                        members.len() - 1 - position
                    } else {
                        position
                    };
                    (
                        gap * bar_width / 2.0 + lane as f64 * bar_width * (1.0 - overlap),
                        zero,
                        at(&scale, value),
                    )
                };
                let along = row(c) * slot + offset;
                let rect = if horizontal_bars {
                    Rect {
                        x: start.min(end),
                        y: plot.y + along,
                        width: (end - start).abs(),
                        height: bar_width,
                    }
                } else {
                    Rect {
                        x: plot.x + along,
                        y: start.min(end),
                        width: bar_width,
                        height: (end - start).abs(),
                    }
                };
                let category_name = category(c);
                let mut bar = kit.shape(
                    kit.tag(&format!("{series_name}/{category_name}")),
                    rect,
                    PlacedGeometry::Rect,
                );
                bar.fill = Some(
                    series
                        .point_fills
                        .iter()
                        .find(|(index, _)| *index == c)
                        .and_then(|(_, fill)| resolve_fill(fill, kit.context.theme))
                        .unwrap_or_else(|| {
                            if plot_element.vary_colors && chart.series.len() == 1 {
                                kit.accent(c)
                            } else {
                                color.clone()
                            }
                        }),
                );
                bar.outline = series
                    .line
                    .as_ref()
                    .filter(|line| !line.no_fill && line.color.is_some())
                    .and_then(|line| resolve_outline(Some(line), None, kit.context.theme, None));
                layers[1].push(bar);
                if let (Some(labels), Some(text_style)) = (labels, text_style.as_ref()) {
                    if labels.hidden_points.contains(&c) {
                        continue;
                    }
                    let text = label_text(labels, series, &category_name, raw, None, None);
                    let default = if stacked { "ctr" } else { "outEnd" };
                    let position = labels.position.as_deref().unwrap_or(default);
                    // Along the value axis: where on the bar the label
                    // sits, and which way it hangs from there.
                    let grows = if horizontal_bars {
                        (end >= start) as i32 as f64 * 2.0 - 1.0
                    } else {
                        (end <= start) as i32 as f64 * 2.0 - 1.0
                    };
                    let (along_value, hang) = match position {
                        "ctr" => ((start + end) / 2.0, 0.0),
                        "inEnd" => (end, -1.0),
                        "inBase" => (start, 1.0),
                        _ => (end, 1.0),
                    };
                    let pad = 3.0;
                    let mid_lane = along + bar_width / 2.0;
                    let (at_point, anchor) = if horizontal_bars {
                        let direction = grows * hang;
                        (
                            (along_value + direction * pad, plot.y + mid_lane),
                            (if direction > 0.0 { 0.0 } else if direction < 0.0 { 1.0 } else { 0.5 }, 0.5),
                        )
                    } else {
                        // Up the screen is negative y.
                        let direction = -grows * hang;
                        (
                            (plot.x + mid_lane, along_value + direction * pad),
                            (0.5, if direction < 0.0 { 1.0 } else if direction > 0.0 { 0.0 } else { 0.5 }),
                        )
                    };
                    place_label(
                        kit,
                        kit.tag(&format!("{series_name} label {c}")),
                        &text,
                        at_point,
                        anchor,
                        text_style,
                        &mut labels_out,
                    );
                }
            }
        }
    }

    // The category axis line, where the primary value axis crosses zero
    // (or its stated crossing).
    let scale = scale_of(primary_id);
    let crossing = category_axis
        .and_then(|axis| axis.crosses_at)
        .or_else(|| match primary.crosses.as_deref() {
            Some("max") => Some(scale.high),
            Some("min") => Some(scale.low),
            _ => None,
        })
        .unwrap_or(0.0)
        .clamp(scale.low.min(scale.high), scale.high.max(scale.low));
    let crossing = if scale.low > 0.0 && category_axis.and_then(|a| a.crosses_at).is_none() {
        scale.low
    } else {
        crossing
    };
    let zero = at(&scale, crossing);
    let axis_line = {
        let (a, b) = if horizontal {
            ((zero, plot.y), (zero, plot.y + plot.height))
        } else {
            ((plot.x, zero), (plot.x + plot.width, zero))
        };
        let mut axis = kit.shape(
            kit.tag("axis"),
            Rect {
                x: a.0,
                y: a.1,
                width: b.0 - a.0,
                height: b.1 - a.1,
            },
            PlacedGeometry::Line,
        );
        axis.outline = Some(
            category_axis
                .and_then(|axis| axis.line.as_ref())
                .and_then(|line| resolve_outline(Some(line), None, kit.context.theme, None))
                .unwrap_or_else(|| kit.stroke(AXIS_COLOR, 1.0)),
        );
        axis
    };
    for layer in layers {
        out.extend(layer);
    }
    if !category_axis.is_some_and(|axis| axis.deleted) {
        out.push(axis_line);
    }

    // Category labels along the category axis, under the plot (or left of
    // a bar chart's).
    if categories_labelled {
        for (c, label) in category_labels.iter().enumerate() {
            if label.trim().is_empty() {
                continue;
            }
            let height = category_style.line_px().max(LABEL_HEIGHT_PX);
            let rect = if horizontal {
                Rect {
                    x: frame.x,
                    y: plot.y + row(c) * slot,
                    width: (plot.x - frame.x - 4.0).max(1.0),
                    height: slot,
                }
            } else {
                Rect {
                    x: plot.x + row(c) * slot,
                    y: plot.y + plot.height,
                    width: slot,
                    height,
                }
            };
            let align = if horizontal {
                TextAlign::Right
            } else {
                TextAlign::Center
            };
            out.push(kit.label(kit.tag(&format!("label {label}")), rect, label, align, &category_style));
        }
    }

    // Axis titles: the value axis' turned to read upwards beside its
    // labels, the category axis' under its labels.
    let place_title = |axis: &ChartAxis, side: &str, out: &mut Vec<PlacedShape>| {
        let Some(title) = axis.title.as_deref() else {
            return;
        };
        let style = title_bold.with(axis.text.as_ref());
        let thick = style.line_px() * 1.2;
        match side {
            "l" | "r" => {
                let x = if side == "l" {
                    frame.x + 2.0
                } else {
                    frame.x + frame.width - thick - 2.0
                };
                let degrees = 270.0;
                out.push(kit.label_turned(
                    kit.tag(&format!("axis title {title}")),
                    Rect {
                        x,
                        y: plot.y,
                        width: thick,
                        height: plot.height,
                    },
                    title,
                    TextAlign::Center,
                    &style,
                    degrees,
                ));
            }
            _ => {
                let labels = category_style.line_px().max(LABEL_HEIGHT_PX);
                out.push(kit.label(
                    kit.tag(&format!("axis title {title}")),
                    Rect {
                        x: plot.x,
                        y: plot.y + plot.height + labels + 2.0,
                        width: plot.width,
                        height: thick,
                    },
                    title,
                    TextAlign::Center,
                    &style,
                ));
            }
        }
    };
    if horizontal {
        if let Some(axis) = category_axis {
            place_title(axis, "l", out);
        }
        place_title(primary, "b", out);
    } else {
        place_title(primary, "l", out);
        if let Some(id) = secondary_id {
            place_title(axis_by_id(id), "r", out);
        }
        if let Some(axis) = category_axis {
            place_title(axis, "b", out);
        }
    }
    out.extend(labels_out);
}

/// Scatter and bubble charts: two value axes, points at their x and y,
/// bubbles sized by area.
fn plot_xy(
    kit: &Kit<'_>,
    frame: Rect,
    mut plot: Rect,
    plots: &[ChartPlot],
    base: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    let chart = kit.chart;
    let style = &chart.style;
    let plot_element = plots.first();
    let bubble = chart.kind.as_deref() == Some("bubbleChart");
    let axes: Vec<&ChartAxis> = plot_element
        .map(|plot| plot.axis_ids.iter().filter_map(|id| style.axis(*id)).collect())
        .unwrap_or_default();
    let x_axis = axes
        .iter()
        .copied()
        .find(|axis| matches!(axis.position.as_deref(), Some("b") | Some("t")))
        .or(axes.first().copied());
    let y_axis = axes
        .iter()
        .copied()
        .find(|axis| matches!(axis.position.as_deref(), Some("l") | Some("r")))
        .or(axes.get(1).copied());
    let points: Vec<Vec<(usize, f64, f64, f64)>> = chart
        .series
        .iter()
        .map(|series| {
            series
                .values
                .iter()
                .enumerate()
                .filter_map(|(i, y)| {
                    let x = series
                        .x_values
                        .get(i)
                        .copied()
                        .flatten()
                        .unwrap_or(i as f64 + 1.0);
                    let size = series.bubble_sizes.get(i).copied().flatten().unwrap_or(1.0);
                    Some((i, x, (*y)?, size)).filter(|(_, x, y, _)| x.is_finite() && y.is_finite())
                })
                .collect()
        })
        .collect();
    let all: Vec<&(usize, f64, f64, f64)> = points.iter().flatten().collect();
    if all.is_empty() {
        return;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (_, x, y, _) in &all {
        x0 = x0.min(*x);
        x1 = x1.max(*x);
        y0 = y0.min(*y);
        y1 = y1.max(*y);
    }
    let labelled = |axis: Option<&ChartAxis>| axis.is_some_and(|axis| axis.labelled());
    let gridded = |axis: Option<&ChartAxis>| axis.is_some_and(|axis| axis.gridlines);
    let x_scale = if x_axis.is_none() {
        Scale {
            low: x0,
            high: if x1 > x0 { x1 } else { x0 + 1.0 },
            step: 1.0,
            reversed: false,
        }
    } else {
        Scale::fit(x0, x1, x_axis, labelled(x_axis) || gridded(x_axis))
    };
    let y_scale = if y_axis.is_none() {
        Scale {
            low: y0.min(0.0),
            high: if y1 > y0.min(0.0) { y1 } else { y0.min(0.0) + 1.0 },
            step: 1.0,
            reversed: false,
        }
    } else {
        Scale::fit(y0, y1, y_axis, labelled(y_axis) || gridded(y_axis))
    };
    let tick_text = |axis: Option<&ChartAxis>, value: f64, scale: &Scale, source: Option<&str>| {
        let code = axis
            .and_then(|axis| match (&axis.number_format, axis.source_linked) {
                (Some(code), false) => Some(code.as_str()),
                (Some(code), true) => source.or(Some(code.as_str())),
                (None, _) => None,
            })
            .filter(|code| !code.eq_ignore_ascii_case("general"));
        match code {
            Some(code) => format_chart_number(value, Some(code)),
            None => format_tick(value, scale.step, false),
        }
    };
    let y_source = chart.series.first().and_then(|s| s.format_code.as_deref());
    let y_style = base.with(y_axis.and_then(|axis| axis.text.as_ref()));
    let x_style = base.with(x_axis.and_then(|axis| axis.text.as_ref()));
    let title_bold = TextStyle { bold: true, ..base.clone() };
    if labelled(y_axis) {
        let width = y_scale
            .ticks()
            .iter()
            .map(|v| kit.measure(&tick_text(y_axis, *v, &y_scale, y_source), &y_style))
            .fold(0.0, f64::max)
            + 8.0;
        let title = y_axis.and_then(|a| a.title.as_ref()).map_or(0.0, |_| title_bold.line_px() * 1.2 + 4.0);
        let x = (frame.x + width + title + 4.0).max(frame.x + 8.0);
        plot.width = (plot.width - (x - plot.x)).max(1.0);
        plot.x = x;
    }
    if x_axis.and_then(|a| a.title.as_ref()).is_some() {
        plot.height = (plot.height - title_bold.line_px() * 1.2 - 4.0).max(1.0);
    }
    let map = |x: f64, y: f64| {
        (
            plot.x + x_scale.fraction(x) * plot.width,
            plot.y + plot.height - y_scale.fraction(y) * plot.height,
        )
    };
    // Grid and ticks.
    for (axis, scale, vertical) in [(y_axis, y_scale, true), (x_axis, x_scale, false)] {
        if axis.is_none() {
            continue;
        }
        for (i, value) in scale.ticks().into_iter().enumerate() {
            let (px, py) = if vertical { map(x_scale.low, value) } else { map(value, y_scale.low) };
            if gridded(axis) {
                let (a, b) = if vertical {
                    ((plot.x, py), (plot.x + plot.width, py))
                } else {
                    ((px, plot.y), (px, plot.y + plot.height))
                };
                let name = if vertical { format!("gridline {i}") } else { format!("x gridline {i}") };
                out.push(kit.segment(kit.tag(&name), a, b, kit.stroke(GRID_COLOR, 1.0)));
            }
            if labelled(axis) {
                let text = tick_text(axis, value, &scale, if vertical { y_source } else { None });
                let (text_style, rect, align) = if vertical {
                    let height = y_style.line_px().max(LABEL_HEIGHT_PX);
                    (
                        &y_style,
                        Rect {
                            x: frame.x,
                            y: py - height / 2.0,
                            width: (plot.x - frame.x - 4.0).max(1.0),
                            height,
                        },
                        TextAlign::Right,
                    )
                } else {
                    let width = kit.measure(&text, &x_style) + 8.0;
                    (
                        &x_style,
                        Rect {
                            x: px - width / 2.0,
                            y: plot.y + plot.height,
                            width,
                            height: x_style.line_px().max(LABEL_HEIGHT_PX),
                        },
                        TextAlign::Center,
                    )
                };
                let name = if vertical { format!("value {text}") } else { format!("x value {text}") };
                out.push(kit.label(kit.tag(&name), rect, &text, align, text_style));
            }
        }
    }
    // Axis lines at the other axis' zero (or its low end).
    let y_cross = 0.0_f64.clamp(y_scale.low.min(y_scale.high), y_scale.high.max(y_scale.low));
    let x_cross = 0.0_f64.clamp(x_scale.low.min(x_scale.high), x_scale.high.max(x_scale.low));
    if x_axis.is_some_and(|axis| !axis.deleted) {
        let (a, b) = (map(x_scale.low, y_cross), map(x_scale.high, y_cross));
        out.push(kit.segment(kit.tag("axis"), a, b, kit.stroke(AXIS_COLOR, 1.0)));
    }
    if y_axis.is_some_and(|axis| !axis.deleted) {
        let (a, b) = (map(x_cross, y_scale.low), map(x_cross, y_scale.high));
        out.push(kit.segment(kit.tag("value axis"), a, b, kit.stroke(AXIS_COLOR, 1.0)));
    }

    let scatter_style = plot_element
        .and_then(|plot| plot.scatter_style.as_deref())
        .unwrap_or("marker");
    let max_size = all.iter().map(|(_, _, _, size)| size.abs()).fold(0.0, f64::max);
    let biggest = plot.width.min(plot.height)
        * BUBBLE_FRACTION
        * plot_element.and_then(|plot| plot.bubble_scale).unwrap_or(100.0)
        / 100.0;
    let mut labels_out = Vec::new();
    for (s, series) in chart.series.iter().enumerate() {
        let color = kit.series_color(s, series, !bubble);
        let name = series
            .name
            .clone()
            .unwrap_or_else(|| format!("series {}", s + 1));
        let labels = labels_for(series, plot_element);
        let text_style = labels.map(|labels| label_style(labels, base));
        if bubble {
            for (i, x, y, size) in &points[s] {
                if *size <= 0.0 || max_size <= 0.0 {
                    continue;
                }
                let diameter = biggest * (size / max_size).sqrt();
                let (px, py) = map(*x, *y);
                let mut disc = kit.shape(
                    kit.tag(&format!("{name} bubble {i}")),
                    Rect {
                        x: px - diameter / 2.0,
                        y: py - diameter / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    PlacedGeometry::Ellipse,
                );
                disc.fill = Some(
                    series
                        .point_fills
                        .iter()
                        .find(|(index, _)| index == i)
                        .and_then(|(_, fill)| resolve_fill(fill, kit.context.theme))
                        .unwrap_or_else(|| color.clone()),
                );
                disc.outline = series
                    .line
                    .as_ref()
                    .filter(|line| !line.no_fill)
                    .and_then(|line| resolve_outline(Some(line), None, kit.context.theme, None));
                out.push(disc);
                if let (Some(labels), Some(text_style)) = (labels, text_style.as_ref()) {
                    if !labels.hidden_points.contains(i) {
                        let category = format_chart_number(*x, None);
                        let text = label_text(labels, series, &category, *y, None, Some(*size));
                        place_label(kit, kit.tag(&format!("{name} label {i}")), &text, (px, py), (0.5, 0.5), text_style, &mut labels_out);
                    }
                }
            }
            continue;
        }
        let pixels: Vec<(usize, (f64, f64))> = points[s]
            .iter()
            .map(|(i, x, y, _)| (*i, map(*x, *y)))
            .collect();
        let line_wanted = !matches!(scatter_style, "marker" | "none")
            || series.line.as_ref().is_some_and(|line| !line.no_fill && line.color.is_some());
        if let Some(stroke) = series_stroke(kit, series, &color).filter(|_| line_wanted) {
            let path: Vec<(f64, f64)> = pixels.iter().map(|(_, p)| *p).collect();
            let smooth_line = series.smooth || scatter_style.starts_with("smooth");
            if smooth_line && path.len() > 2 {
                out.push(kit.polyline(kit.tag(&format!("{name} line")), &smooth(&path), stroke, false));
            } else {
                for pair in pixels.windows(2) {
                    let (i, end) = pair[1];
                    out.push(kit.segment(kit.tag(&format!("{name} {i}")), pair[0].1, end, stroke.clone()));
                }
            }
        }
        let wants_markers = matches!(scatter_style, "marker" | "lineMarker" | "smoothMarker")
            || series.marker.is_some();
        let marker = marker_for(series, s, wants_markers).or_else(|| {
            // A chart built in code with no markers named still shows its
            // points as dots.
            (series.marker.is_none() && plots.iter().all(|p| p.axis_ids.is_empty())).then(|| ChartMarker {
                symbol: "circle".into(),
                size: Some(5.25),
                ..ChartMarker::default()
            })
        });
        for (i, point) in &pixels {
            if let Some(marker) = &marker {
                place_marker(kit, kit.tag(&format!("{name} point {i}")), marker, *point, &color, out);
            }
            if let (Some(labels), Some(text_style)) = (labels, text_style.as_ref()) {
                if !labels.hidden_points.contains(i) {
                    let (_, x, y, _) = points[s].iter().find(|p| p.0 == *i).copied().unwrap_or((0, 0.0, 0.0, 0.0));
                    let text = label_text(labels, series, &format_chart_number(x, None), y, None, None);
                    place_label(kit, kit.tag(&format!("{name} label {i}")), &text, (point.0 + 5.0, point.1), (0.0, 0.5), text_style, &mut labels_out);
                }
            }
        }
    }
    // Axis titles.
    if let Some(title) = y_axis.and_then(|axis| axis.title.as_deref()) {
        let style = title_bold.with(y_axis.and_then(|a| a.text.as_ref()));
        let thick = style.line_px() * 1.2;
        out.push(kit.label_turned(
            kit.tag(&format!("axis title {title}")),
            Rect { x: frame.x + 2.0, y: plot.y, width: thick, height: plot.height },
            title,
            TextAlign::Center,
            &style,
            270.0,
        ));
    }
    if let Some(title) = x_axis.and_then(|axis| axis.title.as_deref()) {
        let style = title_bold.with(x_axis.and_then(|a| a.text.as_ref()));
        out.push(kit.label(
            kit.tag(&format!("axis title {title}")),
            Rect {
                x: plot.x,
                y: plot.y + plot.height + x_style.line_px().max(LABEL_HEIGHT_PX) + 2.0,
                width: plot.width,
                height: style.line_px() * 1.2,
            },
            title,
            TextAlign::Center,
            &style,
        ));
    }
    out.extend(labels_out);
}

/// A radar: one spoke per category from the centre, value rings, and each
/// series as a closed outline (filled for `radarStyle="filled"`).
fn plot_radar(
    kit: &Kit<'_>,
    plot: Rect,
    plots: &[ChartPlot],
    base: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    let chart = kit.chart;
    let style = &chart.style;
    let categories = chart
        .series
        .iter()
        .map(|series| series.categories.len().max(series.values.len()))
        .max()
        .unwrap_or(0);
    if categories < 3 {
        return;
    }
    let plot_element = plots.first();
    let value_axis = plot_element.and_then(|plot| {
        plot.axis_ids
            .iter()
            .filter_map(|id| style.axis(*id))
            .find(|axis| axis.kind == "valAx")
    });
    let category_axis = plot_element.and_then(|plot| {
        plot.axis_ids
            .iter()
            .filter_map(|id| style.axis(*id))
            .find(|axis| axis.kind == "catAx")
    });
    let values = chart.series.iter().flat_map(|s| s.values.iter().flatten().copied());
    let (low, high) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| (l.min(v), h.max(v)));
    if !low.is_finite() {
        return;
    }
    let scale = Scale::fit(low, high, value_axis, true);
    let name_style = base.with(category_axis.and_then(|axis| axis.text.as_ref()));
    let room = name_style.line_px() * 1.5;
    let radius = ((plot.width.min(plot.height)) / 2.0 - room).max(4.0);
    let center = (plot.x + plot.width / 2.0, plot.y + plot.height / 2.0);
    let angle = |c: usize| (-90.0 + 360.0 * c as f64 / categories as f64).to_radians();
    let point = |c: usize, value: f64| {
        let r = radius * scale.fraction(value).clamp(0.0, 1.5);
        (center.0 + r * angle(c).cos(), center.1 + r * angle(c).sin())
    };
    // The web: a ring per tick, a spoke per category.
    if value_axis.is_none_or(|axis| axis.gridlines) {
        for (i, value) in scale.ticks().into_iter().enumerate() {
            if scale.fraction(value) <= 0.0 {
                continue;
            }
            let ring: Vec<(f64, f64)> = (0..categories).map(|c| point(c, value)).collect();
            out.push(kit.polyline(kit.tag(&format!("ring {i}")), &ring, kit.stroke(GRID_COLOR, 1.0), true));
        }
    }
    for c in 0..categories {
        out.push(kit.segment(
            kit.tag(&format!("spoke {c}")),
            center,
            point(c, scale.high),
            kit.stroke(GRID_COLOR, 1.0),
        ));
    }
    let radar_style = plot_element
        .and_then(|plot| plot.radar_style.as_deref())
        .unwrap_or("standard");
    let mut labels_out = Vec::new();
    for (s, series) in chart.series.iter().enumerate() {
        let color = kit.series_color(s, series, radar_style != "filled");
        let name = series
            .name
            .clone()
            .unwrap_or_else(|| format!("series {}", s + 1));
        let ring: Vec<(usize, (f64, f64))> = (0..categories)
            .filter_map(|c| {
                let value = series.values.get(c).copied().flatten()?;
                Some((c, point(c, value)))
            })
            .collect();
        let points: Vec<(f64, f64)> = ring.iter().map(|(_, p)| *p).collect();
        if points.len() >= 2 {
            if radar_style == "filled" {
                let bounds = bounds_of(&points);
                let mut area = kit.shape(
                    kit.tag(&format!("{name} area")),
                    bounds,
                    PlacedGeometry::Polygon(normalize(&points, bounds)),
                );
                area.fill = Some(color.clone());
                out.push(area);
            } else if let Some(stroke) = series_stroke(kit, series, &color) {
                out.push(kit.polyline(kit.tag(&format!("{name} line")), &points, stroke, points.len() == categories));
            }
        }
        if let Some(marker) = marker_for(series, s, radar_style == "marker") {
            for (c, p) in &ring {
                place_marker(kit, kit.tag(&format!("{name} marker {c}")), &marker, *p, &color, out);
            }
        }
        if let Some(labels) = labels_for(series, plot_element) {
            let text_style = label_style(labels, base);
            for (c, p) in &ring {
                if labels.hidden_points.contains(c) {
                    continue;
                }
                let value = series.values.get(*c).copied().flatten().unwrap_or(0.0);
                let category = series.categories.get(*c).cloned().unwrap_or_default();
                let text = label_text(labels, series, &category, value, None, None);
                place_label(kit, kit.tag(&format!("{name} label {c}")), &text, *p, (0.5, 1.1), &text_style, &mut labels_out);
            }
        }
    }
    // Category names just outside the web, value labels up the first spoke.
    let names: Vec<String> = chart
        .series
        .iter()
        .max_by_key(|series| series.categories.len())
        .map(|series| series.categories.clone())
        .unwrap_or_default();
    for (c, name) in names.iter().enumerate().take(categories) {
        let (cos, sin) = (angle(c).cos(), angle(c).sin());
        let at = (
            center.0 + (radius + 6.0) * cos,
            center.1 + (radius + 6.0) * sin,
        );
        let anchor = (0.5 - cos * 0.5, 0.5 - sin * 0.5);
        place_label(kit, kit.tag(&format!("label {name}")), name, at, anchor, &name_style, out);
    }
    if value_axis.is_some_and(|axis| axis.labelled()) {
        let value_style = base.with(value_axis.and_then(|axis| axis.text.as_ref()));
        for value in scale.ticks() {
            let text = match value_axis.and_then(|axis| axis.number_format.as_deref()) {
                Some(code) if !code.eq_ignore_ascii_case("general") => format_chart_number(value, Some(code)),
                _ => format_tick(value, scale.step, false),
            };
            let p = point(0, value);
            place_label(kit, kit.tag(&format!("value {text}")), &text, (p.0 - 4.0, p.1), (1.0, 0.5), &value_style, out);
        }
    }
    out.extend(labels_out);
}

fn plot_pie(
    kit: &Kit<'_>,
    plot: Rect,
    plots: &[ChartPlot],
    base: &TextStyle,
    out: &mut Vec<PlacedShape>,
) {
    let chart = kit.chart;
    let Some(series) = chart.series.first() else {
        return;
    };
    let plot_element = plots.first();
    let series_name = series
        .name
        .clone()
        .unwrap_or_else(|| "series 1".to_string());
    let shares: Vec<(usize, f64)> = series
        .values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            value
                .filter(|v| v.is_finite() && *v > 0.0)
                .map(|v| (index, v))
        })
        .collect();
    let total: f64 = shares.iter().map(|(_, v)| v).sum();
    if total <= 0.0 {
        return;
    }
    let labels = labels_for(series, plot_element);
    let text_style = labels.map(|labels| label_style(labels, base));
    // Labels outside the pie need room around it.
    let outside = labels.is_some_and(|labels| labels.position.as_deref() == Some("outEnd"));
    let margin = if outside {
        text_style.as_ref().map_or(0.0, |style| style.line_px() * 1.5)
    } else {
        0.0
    };
    let explosion = series.explosion.unwrap_or(0.0).max(0.0);
    let radius = ((plot.width.min(plot.height) / 2.0 - margin) / (1.0 + explosion)).max(1.0);
    let center = (plot.x + plot.width / 2.0, plot.y + plot.height / 2.0);
    let hole = if chart.kind.as_deref() == Some("doughnutChart") {
        chart.style.hole.unwrap_or(0.5)
    } else {
        0.0
    };
    // Sectors start at twelve o'clock (turned by `c:firstSliceAng`) and
    // run clockwise, as PowerPoint's.
    let mut start_deg = -90.0
        + plot_element
            .and_then(|plot| plot.first_slice_angle)
            .unwrap_or(0.0);
    let mut labels_out = Vec::new();
    for (index, value) in shares {
        let sweep_deg = value / total * 360.0;
        let middle = (start_deg + sweep_deg / 2.0).to_radians();
        let (dx, dy) = (
            explosion * radius * middle.cos(),
            explosion * radius * middle.sin(),
        );
        let square = Rect {
            x: center.0 - radius + dx,
            y: center.1 - radius + dy,
            width: radius * 2.0,
            height: radius * 2.0,
        };
        let steps = ((sweep_deg / PIE_STEP_DEG).ceil() as usize).max(1);
        let arc = |r: f64| -> Vec<(f64, f64)> {
            (0..=steps)
                .map(|step| {
                    let angle = (start_deg + sweep_deg * step as f64 / steps as f64).to_radians();
                    (0.5 + r * angle.cos(), 0.5 + r * angle.sin())
                })
                .collect()
        };
        let points = if hole > 0.0 {
            let mut ring = arc(0.5);
            ring.extend(arc(0.5 * hole).into_iter().rev());
            ring
        } else {
            let mut points = vec![(0.5, 0.5)];
            points.extend(arc(0.5));
            points
        };
        let category = series
            .categories
            .get(index)
            .cloned()
            .unwrap_or_else(|| (index + 1).to_string());
        let mut sector = kit.shape(
            kit.tag(&format!("{series_name}/{category}")),
            square,
            PlacedGeometry::Polygon(points),
        );
        sector.fill = Some(point_color(kit, series, index));
        sector.outline = Some(kit.stroke("#FFFFFF", 1.0));
        out.push(sector);
        if let (Some(labels), Some(text_style)) = (labels, text_style.as_ref()) {
            if !labels.hidden_points.contains(&index) {
                let text = label_text(labels, series, &category, value, Some(value / total), None);
                let (reach, anchor) = match labels.position.as_deref() {
                    Some("ctr") => ((0.5 + hole * 0.5) / 1.0 * 0.5 + 0.25 * hole, (0.5, 0.5)),
                    Some("inEnd") => (0.8, (0.5, 0.5)),
                    Some("outEnd") => (1.0, (0.5 - middle.cos() * 0.5, 0.5 - middle.sin() * 0.5)),
                    // Best fit: inside, two thirds out (mid-ring for a
                    // doughnut).
                    _ => (if hole > 0.0 { (1.0 + hole) / 2.0 } else { 0.65 }, (0.5, 0.5)),
                };
                let gap = if anchor == (0.5, 0.5) { 0.0 } else { 4.0 };
                let at = (
                    center.0 + dx + (radius * reach + gap) * middle.cos(),
                    center.1 + dy + (radius * reach + gap) * middle.sin(),
                );
                place_label(
                    kit,
                    kit.tag(&format!("{series_name} label {index}")),
                    &text,
                    at,
                    anchor,
                    text_style,
                    &mut labels_out,
                );
            }
        }
        start_deg += sweep_deg;
    }
    out.extend(labels_out);
}
