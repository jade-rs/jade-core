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
    /// Which of [`ChartStyle::plots`] the series is drawn by.
    #[serde(default)]
    pub plot_index: usize,
    /// `c:dLbls` of the series, over its plot's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<DataLabels>,
    /// `c:marker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<ChartMarker>,
    /// `c:smooth val="1"`: the line runs as a curve through its points.
    #[serde(default)]
    pub smooth: bool,
    /// `c:bubbleSize` of a bubble series, per point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bubble_sizes: Vec<Option<f64>>,
    /// The number format the values were cached with (`c:numCache/
    /// c:formatCode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_code: Option<String>,
    /// `c:explosion`: how far a pie's slices stand off, as a fraction of
    /// the radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explosion: Option<f64>,
}

/// `c:dLbls`/`c:dLbl`: what each point's label says and where it sits.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DataLabels {
    #[serde(default)]
    pub show_value: bool,
    #[serde(default)]
    pub show_category: bool,
    #[serde(default)]
    pub show_series: bool,
    #[serde(default)]
    pub show_percent: bool,
    #[serde(default)]
    pub show_bubble_size: bool,
    /// `c:dLblPos`: `outEnd`, `inEnd`, `ctr`, `inBase`, `t`, `b`, `l`, `r`,
    /// `bestFit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    /// `c:numFmt formatCode` when not linked to the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<crate::RunProps>,
    /// Points whose own `c:dLbl` deletes or hides the label.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_points: Vec<usize>,
    /// `c:delete val="1"`: no labels at all.
    #[serde(default)]
    pub deleted: bool,
}

impl DataLabels {
    pub fn shows_anything(&self) -> bool {
        !self.deleted
            && (self.show_value
                || self.show_category
                || self.show_series
                || self.show_percent
                || self.show_bubble_size)
    }
}

/// `c:marker`: the symbol drawn at each point of a line, scatter or radar.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartMarker {
    /// `circle`, `dash`, `diamond`, `dot`, `none`, `picture`, `plus`,
    /// `square`, `star`, `triangle`, `x` or `auto`.
    pub symbol: String,
    /// Points, 2–72; PowerPoint's default is 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Outline>,
}

/// One plot element of the plot area (`c:barChart`, `c:lineChart`, …): a
/// combo chart has several, each on its own pair of axes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartPlot {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
    /// `c:barDir val="bar"`.
    #[serde(default)]
    pub horizontal: bool,
    /// `c:gapWidth`, percent of a bar's width; 150 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_width: Option<f64>,
    /// `c:overlap`, percent, -100..100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap: Option<f64>,
    /// `c:axId`s, category (or x) axis first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axis_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<DataLabels>,
    /// A line chart's `c:marker val`: points carry markers unless a series
    /// says otherwise.
    #[serde(default)]
    pub markers: bool,
    /// `c:radarStyle`: `standard`, `marker` or `filled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_style: Option<String>,
    /// `c:scatterStyle`: `lineMarker`, `line`, `marker`, `smooth`, ….
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scatter_style: Option<String>,
    /// `c:bubbleScale`, percent; 100 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble_scale: Option<f64>,
    /// `c:varyColors val="1"`: each point takes the next accent.
    #[serde(default)]
    pub vary_colors: bool,
    /// `c:firstSliceAng`, degrees clockwise from twelve o'clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_slice_angle: Option<f64>,
}

/// One axis of the plot area (`c:catAx`, `c:valAx`, `c:dateAx`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartAxis {
    pub id: u32,
    /// The element: `catAx`, `valAx`, `dateAx` or `serAx`.
    pub kind: String,
    /// `c:axPos`: `l`, `r`, `t` or `b`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    /// `c:delete val="1"`: the axis is neither drawn nor labelled.
    #[serde(default)]
    pub deleted: bool,
    /// `c:scaling/c:orientation val="maxMin"`.
    #[serde(default)]
    pub reversed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub major_unit: Option<f64>,
    /// `c:numFmt formatCode`, and whether it follows the source data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    #[serde(default)]
    pub source_linked: bool,
    /// `c:title` text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_axis: Option<u32>,
    /// `c:crosses`: `autoZero`, `min` or `max`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosses: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosses_at: Option<f64>,
    #[serde(default)]
    pub gridlines: bool,
    /// `c:tickLblPos`: `nextTo`, `low`, `high` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_label_position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<crate::RunProps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Outline>,
}

impl ChartAxis {
    /// Whether the axis shows tick labels.
    pub fn labelled(&self) -> bool {
        !self.deleted && self.tick_label_position.as_deref() != Some("none")
    }
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
    /// Every plot element, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plots: Vec<ChartPlot>,
    /// Every axis, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<ChartAxis>,
    /// `c:chartSpace/c:txPr`: the text properties every label inherits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<crate::RunProps>,
    /// The title's own run properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_text: Option<crate::RunProps>,
}

impl ChartStyle {
    pub fn axis(&self, id: u32) -> Option<&ChartAxis> {
        self.axes.iter().find(|axis| axis.id == id)
    }
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
    let plot = chart.child("plotArea");
    let plots: Vec<&XmlElement> = plot
        .map(|area| {
            area.children
                .iter()
                .filter(|child| child.name.ends_with("Chart"))
                .collect()
        })
        .unwrap_or_default();
    let series: Vec<ChartSeries> = plots
        .iter()
        .enumerate()
        .flat_map(|(index, plot)| {
            plot.children
                .iter()
                .filter(|child| child.name == "ser")
                .map(move |ser| {
                    let mut series = parse_series(ser);
                    series.plot = Some(plot.name.clone());
                    series.plot_index = index;
                    series
                })
        })
        .collect();
    let title_element = chart.child("title");
    let auto_title_deleted = flag(chart.child("autoTitleDeleted")).unwrap_or(false);
    let title = title_element
        .and_then(|title| title.child("tx"))
        .and_then(|tx| tx.child("rich"))
        .map(rich_text)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            // A title element without text is the automatic title: the
            // series' name when there is just one, as PowerPoint shows it.
            title_element
                .filter(|_| !auto_title_deleted && series.len() == 1)
                .and(series[0].name.clone())
        });
    let axes: Vec<ChartAxis> = plot
        .map(|area| {
            area.children
                .iter()
                .filter(|child| {
                    matches!(child.name.as_str(), "catAx" | "valAx" | "dateAx" | "serAx")
                })
                .filter_map(parse_axis)
                .collect()
        })
        .unwrap_or_default();
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
        gridlines: axes
            .iter()
            .filter(|axis| axis.kind == "valAx")
            .any(|axis| axis.gridlines),
        value_labels: axes
            .iter()
            .filter(|axis| axis.kind == "valAx")
            .any(ChartAxis::labelled),
        plots: plots.iter().map(|plot| parse_plot(plot)).collect(),
        axes,
        text: root.child("txPr").and_then(text_properties),
        title_text: title_element
            .and_then(|title| {
                title
                    .child("tx")
                    .and_then(|tx| tx.child("rich"))
                    .or_else(|| title.child("txPr"))
            })
            .and_then(text_properties),
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

/// `val="1"`/`"true"` of a boolean element; an element without `val` is
/// true, as the schema defaults it.
fn flag(element: Option<&XmlElement>) -> Option<bool> {
    let element = element?;
    Some(!matches!(element.attr("val"), Some("0") | Some("false")))
}

fn number(element: Option<&XmlElement>) -> Option<f64> {
    element?
        .attr("val")?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// The run properties of a `c:txPr` or `c:rich`: its first paragraph's
/// `a:defRPr`, overlaid by its first run's `a:rPr`.
fn text_properties(body: &XmlElement) -> Option<crate::RunProps> {
    let paragraph = body.child("p")?;
    let defaults = paragraph
        .child("pPr")
        .and_then(|p| p.child("defRPr"))
        .map(crate::slide::parse_run_props);
    let run = paragraph
        .child("r")
        .and_then(|r| r.child("rPr"))
        .map(crate::slide::parse_run_props);
    match (defaults, run) {
        (None, None) => None,
        (Some(props), None) | (None, Some(props)) => Some(props),
        (Some(base), Some(over)) => Some(crate::RunProps {
            size_pt: over.size_pt.or(base.size_pt),
            bold: over.bold.or(base.bold),
            italic: over.italic.or(base.italic),
            underline: over.underline.or(base.underline),
            color: over.color.or(base.color),
            font: over.font.or(base.font),
            east_asian_font: over.east_asian_font.or(base.east_asian_font),
        }),
    }
}

fn parse_plot(plot: &XmlElement) -> ChartPlot {
    let value = |name: &str| {
        plot.child(name)
            .and_then(|e| e.attr("val"))
            .map(str::to_string)
    };
    ChartPlot {
        kind: plot.name.clone(),
        grouping: value("grouping"),
        horizontal: value("barDir").as_deref() == Some("bar"),
        gap_width: number(plot.child("gapWidth")),
        overlap: number(plot.child("overlap")),
        axis_ids: plot
            .children
            .iter()
            .filter(|child| child.name == "axId")
            .filter_map(|id| id.attr("val")?.parse::<i64>().ok())
            .map(|id| id as u32)
            .collect(),
        labels: plot.child("dLbls").map(parse_labels),
        markers: flag(plot.child("marker")).unwrap_or(false),
        radar_style: value("radarStyle"),
        scatter_style: value("scatterStyle"),
        bubble_scale: number(plot.child("bubbleScale")),
        vary_colors: flag(plot.child("varyColors")).unwrap_or(false),
        first_slice_angle: number(plot.child("firstSliceAng")),
    }
}

fn parse_axis(axis: &XmlElement) -> Option<ChartAxis> {
    let id = axis.child("axId")?.attr("val")?.parse::<i64>().ok()? as u32;
    let scaling = axis.child("scaling");
    let number_format = axis.child("numFmt");
    Some(ChartAxis {
        id,
        kind: axis.name.clone(),
        position: axis
            .child("axPos")
            .and_then(|p| p.attr("val"))
            .map(str::to_string),
        deleted: flag(axis.child("delete")).unwrap_or(false),
        reversed: scaling
            .and_then(|s| s.child("orientation"))
            .and_then(|o| o.attr("val"))
            == Some("maxMin"),
        min: number(scaling.and_then(|s| s.child("min"))),
        max: number(scaling.and_then(|s| s.child("max"))),
        major_unit: number(axis.child("majorUnit")).filter(|unit| *unit > 0.0),
        number_format: number_format
            .and_then(|f| f.attr("formatCode"))
            .map(str::to_string),
        source_linked: number_format
            .and_then(|f| f.attr("sourceLinked"))
            .is_some_and(|linked| linked == "1" || linked == "true"),
        title: axis
            .child("title")
            .and_then(|title| title.child("tx"))
            .and_then(|tx| tx.child("rich"))
            .map(rich_text)
            .filter(|text| !text.trim().is_empty()),
        cross_axis: axis
            .child("crossAx")
            .and_then(|c| c.attr("val"))
            .and_then(|v| v.parse::<i64>().ok())
            .map(|id| id as u32),
        crosses: axis
            .child("crosses")
            .and_then(|c| c.attr("val"))
            .map(str::to_string),
        crosses_at: number(axis.child("crossesAt")),
        gridlines: axis.child("majorGridlines").is_some(),
        tick_label_position: axis
            .child("tickLblPos")
            .and_then(|p| p.attr("val"))
            .map(str::to_string),
        text: axis.child("txPr").and_then(text_properties),
        line: axis
            .child("spPr")
            .and_then(|pr| pr.child("ln"))
            .map(parse_outline),
    })
}

/// A `c:dLbls` (or a single `c:dLbl`): the flags, position and format, with
/// the points whose own label is deleted.
fn parse_labels(labels: &XmlElement) -> DataLabels {
    let shows = |name: &str| flag(labels.child(name)).unwrap_or(false);
    let number_format = labels
        .child("numFmt")
        .filter(|f| !matches!(f.attr("sourceLinked"), Some("1") | Some("true")))
        .and_then(|f| f.attr("formatCode"))
        .map(str::to_string);
    DataLabels {
        show_value: shows("showVal"),
        show_category: shows("showCatName"),
        show_series: shows("showSerName"),
        show_percent: shows("showPercent"),
        show_bubble_size: shows("showBubbleSize"),
        position: labels
            .child("dLblPos")
            .and_then(|p| p.attr("val"))
            .map(str::to_string),
        number_format,
        separator: labels.child("separator").map(|s| s.text.clone()),
        text: labels.child("txPr").and_then(text_properties),
        hidden_points: labels
            .children
            .iter()
            .filter(|child| child.name == "dLbl")
            .filter(|label| {
                flag(label.child("delete")).unwrap_or(false)
                    || !parse_labels(label).shows_anything()
            })
            .filter_map(|label| label.child("idx")?.attr("val")?.parse().ok())
            .collect(),
        deleted: flag(labels.child("delete")).unwrap_or(false),
    }
}

fn parse_marker(marker: &XmlElement) -> ChartMarker {
    ChartMarker {
        symbol: marker
            .child("symbol")
            .and_then(|s| s.attr("val"))
            .unwrap_or("auto")
            .to_string(),
        size: number(marker.child("size")),
        fill: marker.child("spPr").and_then(parse_fill),
        line: marker
            .child("spPr")
            .and_then(|pr| pr.child("ln"))
            .map(parse_outline),
    }
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

fn numbers(container: &XmlElement) -> Vec<Option<f64>> {
    cached_points(container)
        .into_iter()
        .map(|point| point.and_then(|v| v.trim().parse::<f64>().ok()))
        .collect()
}

fn parse_series(ser: &XmlElement) -> ChartSeries {
    let values = ser.child("val").or_else(|| ser.child("yVal"));
    ChartSeries {
        name: ser.child("tx").and_then(text_of_reference),
        categories: ser
            .child("cat")
            .map(|cat| cached_points(cat).into_iter().flatten().collect())
            .unwrap_or_default(),
        values: values.map(numbers).unwrap_or_default(),
        x_values: ser.child("xVal").map(numbers).unwrap_or_default(),
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
        plot_index: 0,
        labels: ser.child("dLbls").map(parse_labels),
        marker: ser.child("marker").map(parse_marker),
        smooth: flag(ser.child("smooth")).unwrap_or(false),
        bubble_sizes: ser.child("bubbleSize").map(numbers).unwrap_or_default(),
        format_code: values
            .and_then(|v| v.child("numRef"))
            .and_then(|r| r.child("numCache"))
            .and_then(|c| c.child("formatCode"))
            .map(|code| code.text.clone())
            .filter(|code| !code.trim().is_empty()),
        explosion: number(ser.child("explosion")).map(|percent| percent / 100.0),
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
