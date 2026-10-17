//! Charts as charts: columns, bars, lines and pies laid out from the cached
//! series into placed shapes, so the renderer needs nothing chart-specific.
//!
//! This is the shape of a chart, not PowerPoint's chart engine: one value
//! axis scaled to the data, categories evenly spaced, series in the theme's
//! accent colours. It replaces a blank frame, and the numbers it draws are
//! the deck's own.

use jade_pptx_reader::{Chart, Color, TextAlign, TextBody, TextParagraph, TextRun};

use crate::style::text_context_for_table;
use crate::{
    resolve_color, text, Context, PlacedGeometry, PlacedOrigin, PlacedOutline, PlacedShape, Rect,
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
    };
    let accent = |index: usize| {
        resolve_color(
            &Color::Scheme(format!("accent{}", index % 6 + 1)),
            context.theme,
        )
    };

    let mut out = Vec::new();
    match kind {
        "barChart" | "bar3DChart" | "lineChart" | "line3DChart" | "areaChart" => {}
        "pieChart" | "pie3DChart" | "doughnutChart" | "ofPieChart" => {
            plot_pie(chart, plot, &tag, &make, &accent, &mut out);
            return out;
        }
        _ => return out,
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
    let values = chart
        .series
        .iter()
        .flat_map(|series| series.values.iter().flatten().copied())
        .filter(|value| value.is_finite());
    let (low, mut high) = values.fold((0.0f64, 0.0f64), |(low, high), value| {
        (low.min(value), high.max(value))
    });
    if high - low <= 0.0 {
        high = low + 1.0;
    }
    let range = high - low;
    let horizontal = kind.starts_with("bar") && chart.horizontal;
    let is_line = kind.starts_with("line") || kind.starts_with("area");

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
    let zero = at(0.0);

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
        });
        axis
    });

    let series_count = chart.series.len();
    for (s, series) in chart.series.iter().enumerate() {
        let series_name = series
            .name
            .clone()
            .unwrap_or_else(|| format!("series {}", s + 1));
        let color = accent(s);
        let mut previous: Option<(f64, f64)> = None;
        for c in 0..categories {
            let category = series
                .categories
                .get(c)
                .cloned()
                .unwrap_or_else(|| (c + 1).to_string());
            let value = series.values.get(c).copied().flatten();
            if is_line {
                let Some(value) = value.filter(|v| v.is_finite()) else {
                    previous = None;
                    continue;
                };
                let point = if horizontal {
                    (at(value), plot.y + (c as f64 + 0.5) * slot)
                } else {
                    (plot.x + (c as f64 + 0.5) * slot, at(value))
                };
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
                    segment.flip_v = point.1 < start.1;
                    segment.outline = Some(PlacedOutline {
                        color: color.clone(),
                        width_px: LINE_WIDTH_PX,
                        dash_px: Vec::new(),
                    });
                    out.push(segment);
                }
                previous = Some(point);
                continue;
            }
            let Some(value) = value.filter(|v| v.is_finite()) else {
                continue;
            };
            let group = slot * BAR_GROUP_FRACTION;
            let thickness = group / series_count as f64;
            let along = (c as f64) * slot + (slot - group) / 2.0 + s as f64 * thickness;
            let end = at(value);
            let rect = if horizontal {
                Rect {
                    x: zero.min(end),
                    y: plot.y + along,
                    width: (end - zero).abs(),
                    height: thickness,
                }
            } else {
                Rect {
                    x: plot.x + along,
                    y: zero.min(end),
                    width: thickness,
                    height: (end - zero).abs(),
                }
            };
            let mut bar = make(
                tag(format!("{series_name}/{category}")),
                rect,
                PlacedGeometry::Rect,
            );
            bar.fill = Some(color.clone());
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
    let text_context = text_context_for_table(context);
    for (c, category) in label_categories.iter().enumerate() {
        if category.trim().is_empty() {
            continue;
        }
        let rect = if horizontal {
            Rect {
                x: frame.x,
                y: plot.y + c as f64 * slot,
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
        let body = TextBody {
            insets: jade_pptx_reader::TextInsets {
                left_emu: Some(0),
                top_emu: Some(0),
                right_emu: Some(0),
                bottom_emu: Some(0),
            },
            anchor: Some(jade_pptx_reader::TextAnchor::Center),
            paragraphs: vec![TextParagraph {
                align: Some(if horizontal {
                    TextAlign::Right
                } else {
                    TextAlign::Center
                }),
                runs: vec![TextRun {
                    text: category.clone(),
                    size_pt: Some(LABEL_SIZE_PT),
                    ..TextRun::default()
                }],
                ..TextParagraph::default()
            }],
            ..TextBody::default()
        };
        let (lines, _) = text::layout_text(&body, rect.width, rect.height, &text_context);
        let mut label = make(tag(format!("label {category}")), rect, PlacedGeometry::Rect);
        label.lines = lines;
        out.push(label);
    }
    out
}

fn plot_pie(
    chart: &Chart,
    plot: Rect,
    tag: &dyn Fn(String) -> String,
    make: &dyn Fn(String, Rect, PlacedGeometry) -> PlacedShape,
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
    // Sectors start at twelve o'clock and run clockwise, as PowerPoint's.
    let mut start_deg = -90.0;
    for (index, value) in shares {
        let sweep_deg = value / total * 360.0;
        let steps = ((sweep_deg / PIE_STEP_DEG).ceil() as usize).max(1);
        let mut points = vec![(0.5, 0.5)];
        for step in 0..=steps {
            let angle = (start_deg + sweep_deg * step as f64 / steps as f64).to_radians();
            points.push((0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin()));
        }
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
        sector.fill = Some(accent(index));
        out.push(sector);
        start_deg += sweep_deg;
    }
}
