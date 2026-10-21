//! Charts as charts: columns, bars, lines and pies laid out from the cached
//! series into placed shapes, so the renderer needs nothing chart-specific.
//!
//! This is the shape of a chart, not PowerPoint's chart engine: one value
//! axis scaled to the data, categories evenly spaced, series in the theme's
//! accent colours. It replaces a blank frame, and the numbers it draws are
//! the deck's own.

use jade_pptx_reader::{Chart, ChartSeries, Color, TextAlign, TextBody, TextParagraph, TextRun};

use crate::style::text_context_for_table;
use crate::{
    resolve_color, resolve_fill, text, Context, PlacedGeometry, PlacedOrigin, PlacedOutline,
    PlacedShape, Rect,
};

/// Room under the plot for category labels, in px.
const LABEL_HEIGHT_PX: f64 = 20.0;
/// Room above the plot, more when a title sits there.
const TOP_INSET_PX: f64 = 12.0;
const TOP_INSET_WITH_TITLE_PX: f64 = 28.0;
/// Horizontal insets as fractions of the frame width.
const LEFT_INSET: f64 = 0.08;
const RIGHT_INSET: f64 = 0.04;
/// How much of a category slot the bars fill.
const BAR_GROUP_FRACTION: f64 = 0.8;
const AXIS_COLOR: &str = "#808080";
const GRID_COLOR: &str = "#D9D9D9";
/// A scatter point's marker diameter, in px.
const MARKER_PX: f64 = 7.0;
const LABEL_SIZE_PT: f32 = 9.0;
const LINE_WIDTH_PX: f64 = 2.0;
/// Degrees per sampled point along a pie sector's arc.
const PIE_STEP_DEG: f64 = 5.0;

/// The plot for `chart` inside `frame`, in paint order.
pub(crate) fn plot_chart(
    chart: &Chart,
    frame: Rect,
    has_title: bool,
    context: &Context<'_>,
    origin: PlacedOrigin,
) -> Vec<PlacedShape> {
    let kind = chart.kind.as_deref().unwrap_or("");
    let top = if has_title {
        TOP_INSET_WITH_TITLE_PX
    } else {
        TOP_INSET_PX
    };
    let plot = Rect {
        x: frame.x + frame.width * LEFT_INSET,
        y: frame.y + top,
        width: (frame.width * (1.0 - LEFT_INSET - RIGHT_INSET)).max(1.0),
        height: (frame.height - top - LABEL_HEIGHT_PX).max(1.0),
    };
    let tag = |suffix: String| format!("{}!{}", chart.name, suffix);
    let make = |name: String, rect: Rect, geometry: PlacedGeometry| PlacedShape {
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
        origin,
        source_id: chart.id,
        text_rotation_deg: 0.0,
        shadow: None,
        image_alpha: None,
        pattern: None,
        effects: Default::default(),
    };
    let accent = |index: usize| {
        resolve_color(
            &Color::Scheme(format!("accent{}", index % 6 + 1)),
            context.theme,
        )
    };

    let style = &chart.style;
    let theme = context.theme;
    let text_context = text_context_for_table(context);
    let label = |name: String, rect: Rect, text: &str, align: TextAlign| -> PlacedShape {
        let body = TextBody {
            insets: jade_pptx_reader::TextInsets {
                left_emu: Some(0),
                top_emu: Some(0),
                right_emu: Some(0),
                bottom_emu: Some(0),
            },
            anchor: Some(jade_pptx_reader::TextAnchor::Center),
            paragraphs: vec![TextParagraph {
                align: Some(align),
                runs: vec![TextRun {
                    text: text.to_string(),
                    size_pt: Some(LABEL_SIZE_PT),
                    ..TextRun::default()
                }],
                ..TextParagraph::default()
            }],
            ..TextBody::default()
        };
        let (lines, _) = text::layout_text(&body, rect.width, rect.height, &text_context);
        let mut shape = make(name, rect, PlacedGeometry::Rect);
        shape.lines = lines;
        shape
    };
    let is_pie = matches!(
        kind,
        "pieChart" | "pie3DChart" | "doughnutChart" | "ofPieChart"
    );
    // A series' colour: its own fill (or, for a line, its stroke), else the
    // theme accent at its index.
    let series_color = |index: usize, series: &ChartSeries, prefer_line: bool| -> String {
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
        .unwrap_or_else(|| accent(index))
    };

    let mut out = Vec::new();

    // The legend takes its side of the frame before the plot is placed.
    let mut plot = plot;
    if let Some(position) = style.legend.as_deref() {
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
                                point_color(series, i, theme, &accent),
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
                    let line = series.plot.as_deref().is_some_and(is_line_kind);
                    (
                        series
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("Series {}", i + 1)),
                        series_color(i, series, line),
                    )
                })
                .collect()
        };
        let row = LABEL_HEIGHT_PX;
        let swatch = 8.0;
        let place_entry = |index: usize, x: f64, y: f64, width: f64, out: &mut Vec<PlacedShape>| {
            let (name, color) = &entries[index];
            let mut key = make(
                tag(format!("legend key {name}")),
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
            out.push(label(
                tag(format!("legend {name}")),
                Rect {
                    x: x + swatch + 4.0,
                    y,
                    width: (width - swatch - 4.0).max(1.0),
                    height: row,
                },
                name,
                TextAlign::Left,
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
                let width = (frame.width * 0.9 / entries.len().max(1) as f64).min(120.0);
                let start = frame.x + (frame.width - width * entries.len() as f64) / 2.0;
                for i in 0..entries.len() {
                    place_entry(i, start + i as f64 * width, y, width, &mut out);
                }
            }
            _ => {
                let width = (frame.width * 0.22).min(160.0);
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
                    place_entry(i, x, top + i as f64 * row, width - 8.0, &mut out);
                }
            }
        }
    }

    match kind {
        "barChart" | "bar3DChart" | "lineChart" | "line3DChart" | "areaChart" | "area3DChart"
        | "scatterChart" | "bubbleChart" | "stockChart" => {}
        _ if is_pie => {
            plot_pie(chart, plot, &tag, &make, theme, &accent, &mut out);
            return out;
        }
        _ => return out,
    }
    if kind == "scatterChart" || kind == "bubbleChart" {
        plot_scatter(chart, plot, &tag, &make, &series_color, &mut out);
        return out;
    }

    let categories = chart
        .series
        .iter()
        .map(|series| series.categories.len().max(series.values.len()))
        .max()
        .unwrap_or(0);
    if categories == 0 || chart.series.is_empty() {
        return out;
    }
    let grouping = style.grouping.as_deref().unwrap_or("clustered");
    let stacked = matches!(grouping, "stacked" | "percentStacked");
    let percent = grouping == "percentStacked";
    let value_at = |s: usize, c: usize| -> Option<f64> {
        chart.series[s]
            .values
            .get(c)
            .copied()
            .flatten()
            .filter(|v| v.is_finite())
    };
    // Each category's positive and negative totals, for stacking.
    let totals: Vec<(f64, f64)> = (0..categories)
        .map(|c| {
            (0..chart.series.len()).fold((0.0, 0.0), |(pos, neg), s| match value_at(s, c) {
                Some(v) if v >= 0.0 => (pos + v, neg),
                Some(v) => (pos, neg + v),
                None => (pos, neg),
            })
        })
        .collect();
    let scaled = |c: usize, v: f64| -> f64 {
        if percent {
            let total = totals[c].0 - totals[c].1;
            if total > 0.0 {
                v / total * 100.0
            } else {
                0.0
            }
        } else {
            v
        }
    };
    let (mut low, mut high) = if stacked {
        (0..categories).fold((0.0f64, 0.0f64), |(low, high), c| {
            (
                low.min(scaled(c, totals[c].1)),
                high.max(scaled(c, totals[c].0)),
            )
        })
    } else {
        chart
            .series
            .iter()
            .flat_map(|series| series.values.iter().flatten().copied())
            .filter(|value| value.is_finite())
            .fold((0.0f64, 0.0f64), |(low, high), value| {
                (low.min(value), high.max(value))
            })
    };
    if high - low <= 0.0 {
        high = low + 1.0;
    }
    // With a labelled value axis, the scale snaps to round ticks the way
    // PowerPoint's automatic axis does.
    let ticks = style.value_labels || style.gridlines;
    let step = nice_step((high - low) / 5.0);
    if ticks {
        low = (low / step).floor() * step;
        high = (high / step).ceil() * step;
        if style.value_labels {
            let room = (frame.width * 0.08).max(40.0);
            if plot.x - frame.x < room {
                let shift = room - (plot.x - frame.x);
                plot.x += shift;
                plot.width = (plot.width - shift).max(1.0);
            }
        }
    }
    let range = high - low;
    let horizontal = kind.starts_with("bar") && chart.horizontal;

    // Value → px along the value axis; category index → the slot it owns.
    let slot = if horizontal {
        plot.height / categories as f64
    } else {
        plot.width / categories as f64
    };
    let at = |value: f64| -> f64 {
        let fraction = (value - low) / range;
        if horizontal {
            plot.x + fraction * plot.width
        } else {
            plot.y + plot.height - fraction * plot.height
        }
    };
    let zero = at(0.0_f64.clamp(low, high));
    // A bar chart lists its first category at the bottom, as PowerPoint
    // does; columns run left to right.
    let row = |c: usize| -> f64 {
        if horizontal {
            (categories - 1 - c) as f64
        } else {
            c as f64
        }
    };

    if ticks {
        let count = (range / step).round() as usize;
        for i in 0..=count.min(50) {
            let value = low + step * i as f64;
            let p = at(value);
            if style.gridlines {
                let mut grid = make(
                    tag(format!("gridline {i}")),
                    if horizontal {
                        Rect {
                            x: p,
                            y: plot.y,
                            width: 0.0,
                            height: plot.height,
                        }
                    } else {
                        Rect {
                            x: plot.x,
                            y: p,
                            width: plot.width,
                            height: 0.0,
                        }
                    },
                    PlacedGeometry::Line,
                );
                grid.outline = Some(PlacedOutline {
                    color: GRID_COLOR.to_string(),
                    width_px: 1.0,
                    dash_px: Vec::new(),
                    head_end: None,
                    tail_end: None,
                });
                out.push(grid);
            }
            if style.value_labels {
                let text = format_tick(value, step, percent);
                let rect = if horizontal {
                    Rect {
                        x: p - 30.0,
                        y: plot.y + plot.height,
                        width: 60.0,
                        height: LABEL_HEIGHT_PX,
                    }
                } else {
                    Rect {
                        x: frame.x,
                        y: p - LABEL_HEIGHT_PX / 2.0,
                        width: plot.x - frame.x - 4.0,
                        height: LABEL_HEIGHT_PX,
                    }
                };
                let align = if horizontal {
                    TextAlign::Center
                } else {
                    TextAlign::Right
                };
                out.push(label(tag(format!("value {text}")), rect, &text, align));
            }
        }
    }

    // The axis at zero.
    out.push({
        let mut axis = make(
            tag("axis".to_string()),
            if horizontal {
                Rect {
                    x: zero,
                    y: plot.y,
                    width: 0.0,
                    height: plot.height,
                }
            } else {
                Rect {
                    x: plot.x,
                    y: zero,
                    width: plot.width,
                    height: 0.0,
                }
            },
            PlacedGeometry::Line,
        );
        axis.outline = Some(PlacedOutline {
            color: AXIS_COLOR.to_string(),
            width_px: 1.0,
            dash_px: Vec::new(),
            head_end: None,
            tail_end: None,
        });
        axis
    });

    // Bars share each category's slot between the bar series only.
    let bar_series: Vec<usize> = (0..chart.series.len())
        .filter(|&s| {
            let plot_kind = chart.series[s].plot.as_deref().unwrap_or(kind);
            plot_kind.starts_with("bar")
        })
        .collect();
    let mut stack_pos = vec![0.0f64; categories];
    let mut stack_neg = vec![0.0f64; categories];
    // An area's previous top edge, for stacked areas.
    let mut area_base: Vec<f64> = vec![0.0; categories];
    for (s, series) in chart.series.iter().enumerate() {
        let series_name = series
            .name
            .clone()
            .unwrap_or_else(|| format!("series {}", s + 1));
        let plot_kind = series.plot.as_deref().unwrap_or(kind);
        let is_line = is_line_kind(plot_kind);
        let is_area = plot_kind.starts_with("area");
        let color = series_color(s, series, is_line);
        let center = |c: usize, value: f64| -> (f64, f64) {
            if horizontal {
                (at(value), plot.y + (row(c) + 0.5) * slot)
            } else {
                (plot.x + (row(c) + 0.5) * slot, at(value))
            }
        };
        if is_area {
            // One polygon from the lower edge up to the values and back.
            let mut top = Vec::new();
            let mut bottom = Vec::new();
            #[allow(clippy::needless_range_loop)]
            for c in 0..categories {
                let v = scaled(c, value_at(s, c).unwrap_or(0.0));
                let base = if stacked { area_base[c] } else { 0.0 };
                let upper = base + v;
                top.push(center(c, upper));
                bottom.push(center(c, base));
                if stacked {
                    area_base[c] = upper;
                }
            }
            let mut points: Vec<(f64, f64)> = top;
            points.extend(bottom.into_iter().rev());
            let normalized = points
                .iter()
                .map(|(x, y)| ((x - plot.x) / plot.width, (y - plot.y) / plot.height))
                .collect();
            let mut area = make(
                tag(format!("{series_name} area")),
                plot,
                PlacedGeometry::Polygon(normalized),
            );
            area.fill = Some(color.clone());
            out.push(area);
            continue;
        }
        let mut previous: Option<(f64, f64)> = None;
        for c in 0..categories {
            let category = series
                .categories
                .get(c)
                .cloned()
                .unwrap_or_else(|| (c + 1).to_string());
            let value = value_at(s, c);
            if is_line {
                let Some(value) = value else {
                    previous = None;
                    continue;
                };
                let value = scaled(c, value);
                let value = if stacked {
                    let base = if value >= 0.0 {
                        stack_pos[c]
                    } else {
                        stack_neg[c]
                    };
                    if value >= 0.0 {
                        stack_pos[c] += value;
                    } else {
                        stack_neg[c] += value;
                    }
                    base + value
                } else {
                    value
                };
                let point = center(c, value);
                if let Some(start) = previous {
                    let rect = Rect {
                        x: start.0.min(point.0),
                        y: start.1.min(point.1),
                        width: (point.0 - start.0).abs(),
                        height: (point.1 - start.1).abs(),
                    };
                    let mut segment = make(
                        tag(format!("{series_name} {c}")),
                        rect,
                        PlacedGeometry::Line,
                    );
                    // The line geometry runs top-left to bottom-right; a
                    // rising segment is that, flipped vertically.
                    segment.flip_v = (point.1 < start.1) != (point.0 < start.0);
                    segment.outline = Some(PlacedOutline {
                        color: color.clone(),
                        width_px: series
                            .line
                            .as_ref()
                            .and_then(|line| line.width_emu)
                            .map_or(LINE_WIDTH_PX, jade_pptx_reader::emu_to_px),
                        dash_px: Vec::new(),
                        head_end: None,
                        tail_end: None,
                    });
                    out.push(segment);
                }
                previous = Some(point);
                continue;
            }
            let Some(value) = value else {
                continue;
            };
            let value = scaled(c, value);
            let group = slot * BAR_GROUP_FRACTION;
            let (along, thickness, start, end) = if stacked {
                let base = if value >= 0.0 {
                    stack_pos[c]
                } else {
                    stack_neg[c]
                };
                if value >= 0.0 {
                    stack_pos[c] += value;
                } else {
                    stack_neg[c] += value;
                }
                (
                    row(c) * slot + (slot - group) / 2.0,
                    group,
                    at(base),
                    at(base + value),
                )
            } else {
                let position = bar_series.iter().position(|&b| b == s).unwrap_or(0);
                // Sideways bars stack their series bottom-up too.
                let position = if horizontal {
                    bar_series.len() - 1 - position
                } else {
                    position
                };
                let thickness = group / bar_series.len().max(1) as f64;
                (
                    row(c) * slot + (slot - group) / 2.0 + position as f64 * thickness,
                    thickness,
                    zero,
                    at(value),
                )
            };
            let rect = if horizontal {
                Rect {
                    x: start.min(end),
                    y: plot.y + along,
                    width: (end - start).abs(),
                    height: thickness,
                }
            } else {
                Rect {
                    x: plot.x + along,
                    y: start.min(end),
                    width: thickness,
                    height: (end - start).abs(),
                }
            };
            let mut bar = make(
                tag(format!("{series_name}/{category}")),
                rect,
                PlacedGeometry::Rect,
            );
            bar.fill = Some(
                series
                    .point_fills
                    .iter()
                    .find(|(index, _)| *index == c)
                    .and_then(|(_, fill)| resolve_fill(fill, theme))
                    .unwrap_or_else(|| color.clone()),
            );
            out.push(bar);
        }
    }

    // Category labels along the category axis.
    let label_categories = chart
        .series
        .iter()
        .max_by_key(|series| series.categories.len())
        .map(|series| series.categories.clone())
        .unwrap_or_default();
    for (c, category) in label_categories.iter().enumerate() {
        if category.trim().is_empty() {
            continue;
        }
        let rect = if horizontal {
            Rect {
                x: frame.x,
                y: plot.y + row(c) * slot,
                width: plot.x - frame.x,
                height: slot,
            }
        } else {
            Rect {
                x: plot.x + c as f64 * slot,
                y: plot.y + plot.height,
                width: slot,
                height: LABEL_HEIGHT_PX,
            }
        };
        let align = if horizontal {
            TextAlign::Right
        } else {
            TextAlign::Center
        };
        out.push(label(
            tag(format!("label {category}")),
            rect,
            category,
            align,
        ));
    }
    out
}

fn is_line_kind(kind: &str) -> bool {
    kind.starts_with("line") || kind.starts_with("radar") || kind == "stockChart"
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

/// A pie slice's colour: its data point's own fill, else the accent cycle.
fn point_color(
    series: &ChartSeries,
    index: usize,
    theme: &jade_pptx_reader::ThemeColors,
    accent: &dyn Fn(usize) -> String,
) -> String {
    series
        .point_fills
        .iter()
        .find(|(i, _)| *i == index)
        .and_then(|(_, fill)| resolve_fill(fill, theme))
        .unwrap_or_else(|| accent(index))
}

fn plot_scatter(
    chart: &Chart,
    plot: Rect,
    tag: &dyn Fn(String) -> String,
    make: &dyn Fn(String, Rect, PlacedGeometry) -> PlacedShape,
    series_color: &dyn Fn(usize, &ChartSeries, bool) -> String,
    out: &mut Vec<PlacedShape>,
) {
    let points: Vec<Vec<(f64, f64)>> = chart
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
                    Some((x, (*y)?)).filter(|(x, y)| x.is_finite() && y.is_finite())
                })
                .collect()
        })
        .collect();
    let all = points.iter().flatten();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, 0.0f64, f64::MIN);
    for (x, y) in all {
        x0 = x0.min(*x);
        x1 = x1.max(*x);
        y0 = y0.min(*y);
        y1 = y1.max(*y);
    }
    if x0 > x1 {
        return;
    }
    if x1 - x0 <= 0.0 {
        x1 = x0 + 1.0;
    }
    if y1 - y0 <= 0.0 {
        y1 = y0 + 1.0;
    }
    let map = |(x, y): (f64, f64)| {
        (
            plot.x + (x - x0) / (x1 - x0) * plot.width,
            plot.y + plot.height - (y - y0) / (y1 - y0) * plot.height,
        )
    };
    for (s, series) in chart.series.iter().enumerate() {
        let color = series_color(s, series, true);
        let name = series
            .name
            .clone()
            .unwrap_or_else(|| format!("series {}", s + 1));
        let draws_line = series
            .line
            .as_ref()
            .is_some_and(|line| !line.no_fill && line.color.is_some());
        let mut previous: Option<(f64, f64)> = None;
        for (i, point) in points[s].iter().enumerate() {
            let (px, py) = map(*point);
            if let (true, Some(start)) = (draws_line, previous) {
                let mut segment = make(
                    tag(format!("{name} {i}")),
                    Rect {
                        x: start.0.min(px),
                        y: start.1.min(py),
                        width: (px - start.0).abs(),
                        height: (py - start.1).abs(),
                    },
                    PlacedGeometry::Line,
                );
                segment.flip_v = (py < start.1) != (px < start.0);
                segment.outline = Some(PlacedOutline {
                    color: color.clone(),
                    width_px: LINE_WIDTH_PX,
                    dash_px: Vec::new(),
                    head_end: None,
                    tail_end: None,
                });
                out.push(segment);
            }
            previous = Some((px, py));
            let r = MARKER_PX / 2.0;
            let mut marker = make(
                tag(format!("{name} point {i}")),
                Rect {
                    x: px - r,
                    y: py - r,
                    width: MARKER_PX,
                    height: MARKER_PX,
                },
                PlacedGeometry::Ellipse,
            );
            marker.fill = Some(color.clone());
            out.push(marker);
        }
    }
}

fn plot_pie(
    chart: &Chart,
    plot: Rect,
    tag: &dyn Fn(String) -> String,
    make: &dyn Fn(String, Rect, PlacedGeometry) -> PlacedShape,
    theme: &jade_pptx_reader::ThemeColors,
    accent: &dyn Fn(usize) -> String,
    out: &mut Vec<PlacedShape>,
) {
    let Some(series) = chart.series.first() else {
        return;
    };
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
    let radius = plot.width.min(plot.height) / 2.0;
    let square = Rect {
        x: plot.x + plot.width / 2.0 - radius,
        y: plot.y + plot.height / 2.0 - radius,
        width: radius * 2.0,
        height: radius * 2.0,
    };
    let hole = if chart.kind.as_deref() == Some("doughnutChart") {
        chart.style.hole.unwrap_or(0.5)
    } else {
        0.0
    };
    // Sectors start at twelve o'clock and run clockwise, as PowerPoint's.
    let mut start_deg = -90.0;
    for (index, value) in shares {
        let sweep_deg = value / total * 360.0;
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
        let mut sector = make(
            tag(format!("{series_name}/{category}")),
            square,
            PlacedGeometry::Polygon(points),
        );
        sector.fill = Some(point_color(series, index, theme, accent));
        sector.outline = Some(PlacedOutline {
            color: "#FFFFFF".to_string(),
            width_px: 1.0,
            dash_px: Vec::new(),
            head_end: None,
            tail_end: None,
        });
        out.push(sector);
        start_deg += sweep_deg;
    }
}
