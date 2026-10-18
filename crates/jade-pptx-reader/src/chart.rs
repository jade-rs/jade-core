//! Charts: the `c:chartSpace` part a graphic frame points at, read down to
//! the cached points of each series — what a caller with no UI wants from a
//! chart is its numbers, and PowerPoint caches them in the part.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::slide::{parse_fill, parse_outline};
use crate::xml_tree::{parse_tree, XmlElement};
use crate::{Fill, Outline};

/// One series: its name, category labels and values, as cached in the part.
/// A point missing from the cache is `None` at its index so the values line
/// up with the categories.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: Option<String>,
    pub categories: Vec<String>,
    pub values: Vec<Option<f64>>,
    /// `c:xVal` of a scatter series: each point's x.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub x_values: Vec<Option<f64>>,
    /// The plot element this series belongs to (`barChart`, `lineChart`, …),
    /// which differs between the series of a combo chart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot: Option<String>,
    /// `c:spPr` fill of the series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    /// `c:spPr/a:ln` of the series: a line chart's stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Outline>,
    /// `c:dPt`: per-point fills (a pie's slices), by point index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub point_fills: Vec<(usize, Fill)>,
}

/// How a chart arranges and labels its series.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartStyle {
    /// `c:grouping`: `clustered`, `stacked`, `percentStacked` or `standard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
    /// `c:legend/c:legendPos`: `r`, `l`, `t`, `b` or `tr`; `None` without a
    /// legend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<String>,
    /// `c:holeSize` of a doughnut, as a fraction of the radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole: Option<f64>,
    /// `c:majorGridlines` on the value axis.
    #[serde(default)]
    pub gridlines: bool,
    /// A value axis that shows its tick labels (present, not `c:delete`d).
    #[serde(default)]
    pub value_labels: bool,
}

/// What the chart part says: the plot kind, the title, the series.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartContent {
    /// The plot element under `c:plotArea`, e.g. `barChart`, `lineChart`,
    /// `pieChart`; `None` when the plot area is empty.
    pub kind: Option<String>,
    /// `<c:barDir val="bar"/>`: bars run sideways rather than up.
    pub horizontal: bool,
    pub title: Option<String>,
    pub series: Vec<ChartSeries>,
    #[serde(default)]
    pub style: ChartStyle,
}

/// Parses a `c:chartSpace` part.
pub fn parse_chart_xml(xml: &str) -> Result<ChartContent> {
    let root = parse_tree(xml)?;
    let chart = root.child("chart").unwrap_or(&root);
    let title = chart
        .child("title")
        .and_then(|title| title.child("tx"))
        .and_then(|tx| tx.child("rich"))
        .map(rich_text)
        .filter(|text| !text.trim().is_empty());
    let plot = chart.child("plotArea");
    let plots: Vec<&XmlElement> = plot
        .map(|area| {
            area.children
                .iter()
                .filter(|child| child.name.ends_with("Chart"))
                .collect()
        })
        .unwrap_or_default();
    let series = plots
        .iter()
        .flat_map(|plot| {
            plot.children
                .iter()
                .filter(|child| child.name == "ser")
                .map(|ser| {
                    let mut series = parse_series(ser);
                    series.plot = Some(plot.name.clone());
                    series
                })
        })
        .collect();
    let first = plots.first();
    let style = ChartStyle {
        grouping: first
            .and_then(|plot| plot.child("grouping"))
            .and_then(|g| g.attr("val"))
            .map(str::to_string),
        legend: chart.child("legend").map(|legend| {
            legend
                .child("legendPos")
                .and_then(|pos| pos.attr("val"))
                .unwrap_or("r")
                .to_string()
        }),
        hole: first
            .and_then(|plot| plot.child("holeSize"))
            .and_then(|h| h.attr("val"))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| (v / 100.0).clamp(0.0, 0.9)),
        gridlines: plot
            .map(|area| {
                area.children
                    .iter()
                    .filter(|axis| axis.name == "valAx")
                    .any(|axis| axis.child("majorGridlines").is_some())
            })
            .unwrap_or(false),
        value_labels: plot
            .map(|area| {
                area.children
                    .iter()
                    .filter(|axis| axis.name == "valAx")
                    .any(|axis| {
                        !matches!(
                            axis.child("delete").and_then(|d| d.attr("val")),
                            Some("1") | Some("true")
                        ) && axis.child("tickLblPos").and_then(|p| p.attr("val")) != Some("none")
                    })
            })
            .unwrap_or(false),
    };
    Ok(ChartContent {
        style,
        kind: plots.first().map(|plot| plot.name.clone()),
        horizontal: plots
            .first()
            .and_then(|plot| plot.child("barDir"))
            .and_then(|dir| dir.attr("val"))
            == Some("bar"),
        title,
        series,
    })
}

/// Every paragraph's runs concatenated, paragraphs joined with newlines.
fn rich_text(rich: &XmlElement) -> String {
    rich.children
        .iter()
        .filter(|child| child.name == "p")
        .map(|paragraph| {
            paragraph
                .children
                .iter()
                .filter(|child| child.name == "r" || child.name == "fld")
                .filter_map(|run| run.child("t"))
                .map(|t| t.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_series(ser: &XmlElement) -> ChartSeries {
    ChartSeries {
        name: ser.child("tx").and_then(text_of_reference),
        categories: ser
            .child("cat")
            .map(|cat| cached_points(cat).into_iter().flatten().collect())
            .unwrap_or_default(),
        values: ser
            .child("val")
            .or_else(|| ser.child("yVal"))
            .map(|val| {
                cached_points(val)
                    .into_iter()
                    .map(|point| point.and_then(|v| v.trim().parse::<f64>().ok()))
                    .collect()
            })
            .unwrap_or_default(),
        x_values: ser
            .child("xVal")
            .map(|val| {
                cached_points(val)
                    .into_iter()
                    .map(|point| point.and_then(|v| v.trim().parse::<f64>().ok()))
                    .collect()
            })
            .unwrap_or_default(),
        plot: None,
        fill: ser.child("spPr").and_then(parse_fill),
        line: ser
            .child("spPr")
            .and_then(|pr| pr.child("ln"))
            .map(parse_outline),
        point_fills: ser
            .children
            .iter()
            .filter(|child| child.name == "dPt")
            .filter_map(|point| {
                let index = point.child("idx")?.attr("val")?.parse().ok()?;
                Some((index, point.child("spPr").and_then(parse_fill)?))
            })
            .collect(),
    }
}

/// A `c:tx`: a literal `c:v`, or the first cached point of a `c:strRef`.
fn text_of_reference(tx: &XmlElement) -> Option<String> {
    if let Some(literal) = tx.child("v") {
        return Some(literal.text.clone());
    }
    cached_points(tx).into_iter().flatten().next()
}

/// The cached points under a `c:cat`/`c:val`/`c:tx`, in index order, with
/// `None` at indices the cache skipped. Reads `strRef`/`numRef` caches and
/// `strLit`/`numLit` literals alike.
fn cached_points(container: &XmlElement) -> Vec<Option<String>> {
    let mut points: Vec<Option<String>> = Vec::new();
    let mut visit = |cache: &XmlElement| {
        for pt in cache.children.iter().filter(|child| child.name == "pt") {
            let Some(index) = pt.attr("idx").and_then(|idx| idx.parse::<usize>().ok()) else {
                continue;
            };
            // A runaway idx would allocate without bound; caches are small.
            if index > 100_000 {
                continue;
            }
            if points.len() <= index {
                points.resize(index + 1, None);
            }
            points[index] = pt.child("v").map(|v| v.text.clone());
        }
    };
    for child in &container.children {
        match child.name.as_str() {
            "strRef" | "numRef" => {
                for cache in &child.children {
                    if cache.name == "strCache" || cache.name == "numCache" {
                        visit(cache);
                    }
                }
            }
            "strLit" | "numLit" => visit(child),
            _ => {}
        }
    }
    points
}
