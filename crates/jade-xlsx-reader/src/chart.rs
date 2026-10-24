//! Classic DrawingML charts. Sparse caches retain their point indices and
//! formulas so layout can prefer current workbook cells over saved values.
mod pivot;
use crate::error::Result;
pub use pivot::{PivotSource, PivotSourceName};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_CHART_POINTS: usize = 100_000;
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartData {
    pub formula: Option<String>,
    pub points: Vec<Option<String>>,
    pub format_code: Option<String>,
    #[serde(default)]
    pub numeric: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartColor {
    pub rgb: Option<[u8; 3]>,
    pub scheme: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartSeries {
    pub order: u32,
    pub name: ChartData,
    pub categories: ChartData,
    pub values: ChartData,
    pub x_values: ChartData,
    pub color: ChartColor,
    pub point_colors: BTreeMap<usize, ChartColor>,
    pub marker: String,
    pub marker_size: u32,
    /// `c:dLbls` on the series, over the plot's own.
    #[serde(default)]
    pub data_labels: Option<DataLabels>,
    #[serde(default)]
    pub trendlines: Vec<Trendline>,
    #[serde(default)]
    pub error_bars: Vec<ErrorBars>,
    /// A bubble chart's `c:bubbleSize`.
    #[serde(default)]
    pub bubble_sizes: ChartData,
    /// `c:smooth`: the line is drawn as a curve through its points.
    #[serde(default)]
    pub smooth: bool,
}
/// What a data label shows and where (`c:dLbls`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataLabels {
    pub show_value: bool,
    pub show_category: bool,
    pub show_series: bool,
    pub show_percent: bool,
    /// `dLblPos`: `outEnd`, `inEnd`, `ctr`, `inBase`, `t`, `b`, `l`, `r`,
    /// `bestFit`; the chart type's default when absent.
    pub position: Option<String>,
    pub number_format: Option<String>,
    pub separator: Option<String>,
    /// Points whose label is deleted (`c:dLbl` with `c:delete`).
    pub deleted: Vec<usize>,
}
impl DataLabels {
    pub fn shows_anything(&self) -> bool {
        self.show_value || self.show_category || self.show_series || self.show_percent
    }
}
/// `c:trendline`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Trendline {
    /// `linear`, `exp`, `log`, `poly`, `power` or `movingAvg`.
    pub kind: String,
    pub order: u32,
    pub period: u32,
    pub forward: Option<String>,
    pub backward: Option<String>,
    pub color: ChartColor,
    pub display_equation: bool,
    pub display_r_squared: bool,
}
/// `c:errBars`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErrorBars {
    /// `x` or `y`; `y` when absent.
    pub direction: String,
    /// `both`, `plus` or `minus`.
    pub bar_type: String,
    /// `fixedVal`, `percentage`, `stdDev`, `stdErr` or `cust`.
    pub value_type: String,
    pub value: Option<String>,
    pub no_end_cap: bool,
    pub plus: ChartData,
    pub minus: ChartData,
    pub color: ChartColor,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartAxis {
    pub minimum: Option<String>,
    pub maximum: Option<String>,
    pub major_unit: Option<String>,
    pub number_format: Option<String>,
    pub reverse: bool,
    pub hidden: bool,
    pub gridlines: bool,
    pub cross_between: Option<String>,
    /// The axis title's text.
    #[serde(default)]
    pub title: Option<String>,
    /// `scaling/logBase`: a logarithmic value axis.
    #[serde(default)]
    pub log_base: Option<String>,
    /// A `c:dateAx`: categories are dates placed by their value.
    #[serde(default)]
    pub date: bool,
    /// `baseTimeUnit` of a date axis: `days`, `months` or `years`.
    #[serde(default)]
    pub base_time_unit: Option<String>,
    /// `majorTimeUnit` of a date axis.
    #[serde(default)]
    pub major_time_unit: Option<String>,
    /// `axPos`: `l`, `r`, `t` or `b`.
    #[serde(default)]
    pub position: Option<String>,
    /// `crosses`: `autoZero`, `min` or `max`.
    #[serde(default)]
    pub crosses: Option<String>,
}
/// One plot of a chart, after the first: a combination chart has several,
/// a line over columns, often on a secondary axis.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartPlot {
    pub kind: String,
    pub horizontal: bool,
    pub vary_colors: bool,
    pub grouping: String,
    pub gap_width: u32,
    pub overlap: i32,
    pub scatter_style: String,
    pub series: Vec<ChartSeries>,
    pub data_labels: Option<DataLabels>,
    /// Drawn against the secondary axes.
    pub secondary: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_source: Option<PivotSource>,
    pub relationship_id: Option<String>,
    pub part_name: Option<String>,
    pub kind: String,
    pub title: String,
    pub title_source: ChartData,
    pub horizontal: bool,
    pub vary_colors: bool,
    pub grouping: String,
    pub blank_mode: String,
    pub visible_only: bool,
    pub legend: Option<String>,
    pub series: Vec<ChartSeries>,
    pub value_axis: ChartAxis,
    pub category_axis: ChartAxis,
    pub gap_width: u32,
    pub overlap: i32,
    pub hole_size: u32,
    pub first_slice_angle: u32,
    pub scatter_style: String,
    pub palette: [[u8; 3]; 6],
    pub unsupported_properties: Vec<String>,
    /// The first plot's `c:dLbls`.
    #[serde(default)]
    pub data_labels: Option<DataLabels>,
    /// `radarStyle`: `marker`, `standard` or `filled`.
    #[serde(default)]
    pub radar_style: String,
    /// `bubbleScale`, percent of the default bubble size.
    #[serde(default)]
    pub bubble_scale: u32,
    /// `sizeRepresents="w"`: bubble width, not area, follows the size.
    #[serde(default)]
    pub bubble_size_width: bool,
    /// The plots after the first, in the file's order.
    #[serde(default)]
    pub extra_plots: Vec<ChartPlot>,
    #[serde(default)]
    pub secondary_value_axis: Option<ChartAxis>,
    #[serde(default)]
    pub secondary_category_axis: Option<ChartAxis>,
}
impl Default for Chart {
    fn default() -> Self {
        Self {
            pivot_source: None,
            relationship_id: None,
            part_name: None,
            kind: String::new(),
            title: String::new(),
            title_source: ChartData::default(),
            horizontal: false,
            vary_colors: false,
            grouping: "standard".into(),
            blank_mode: "gap".into(),
            visible_only: true,
            legend: None,
            series: Vec::new(),
            value_axis: ChartAxis::default(),
            category_axis: ChartAxis::default(),
            gap_width: 150,
            overlap: 0,
            hole_size: 50,
            first_slice_angle: 0,
            scatter_style: "marker".into(),
            palette: [
                [68, 114, 196],
                [237, 125, 49],
                [165, 165, 165],
                [255, 192, 0],
                [91, 155, 213],
                [112, 173, 71],
            ],
            unsupported_properties: Vec::new(),
            data_labels: None,
            radar_style: "marker".into(),
            bubble_scale: 100,
            bubble_size_width: false,
            extra_plots: Vec::new(),
            secondary_value_axis: None,
            secondary_category_axis: None,
        }
    }
}

#[derive(Default)]
struct Node {
    name: String,
    main: bool,
    drawing: bool,
    attrs: BTreeMap<String, String>,
    text: String,
    children: Vec<Node>,
}
impl Node {
    fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.main && n.name == name)
    }
    fn dchild(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.drawing && n.name == name)
    }
    fn val(&self, name: &str) -> Option<&str> {
        self.child(name)?.attrs.get("val").map(String::as_str)
    }
    fn enabled(&self, name: &str, default: bool) -> bool {
        match self.val(name) {
            Some("1" | "true") => true,
            Some("0" | "false") => false,
            _ => default,
        }
    }
    fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Self>) {
        for n in &self.children {
            if n.main && n.name == name {
                out.push(n)
            }
            n.descendants(name, out)
        }
    }
    fn texts(&self) -> String {
        if self.drawing && self.name == "t" {
            return self.text.clone();
        }
        self.children.iter().map(Self::texts).collect()
    }
}
fn parse_tree(xml: &str) -> Result<Node> {
    let mut reader = NsReader::from_str(xml);
    let mut stack = vec![Node::default()];
    let mut buf = Vec::new();
    loop {
        let (ns, event) = reader.read_resolved_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let main = matches!(&ns,ResolveResult::Bound(n) if matches!(n.as_ref(),b"http://schemas.openxmlformats.org/drawingml/2006/chart"|b"http://purl.oclc.org/ooxml/drawingml/chart"));
                let drawing = matches!(&ns,ResolveResult::Bound(n) if matches!(n.as_ref(),b"http://schemas.openxmlformats.org/drawingml/2006/main"|b"http://purl.oclc.org/ooxml/drawingml/main"));
                let mut node = Node {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    main,
                    drawing,
                    ..Default::default()
                };
                for attr in e.attributes() {
                    let attr = attr?;
                    if !attr.key.as_ref().contains(&b':') {
                        node.attrs.insert(
                            String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                            attr.unescape_value()?.into_owned(),
                        );
                    }
                }
                if matches!(event, Event::Empty(_)) {
                    stack.last_mut().unwrap().children.push(node)
                } else {
                    stack.push(node)
                }
            }
            Event::Text(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::CData(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::GeneralRef(e) => {
                if let Some(ch) = e.resolve_char_ref()? {
                    stack.last_mut().unwrap().text.push(ch)
                } else if let Some(value) =
                    quick_xml::escape::resolve_predefined_entity(&e.decode()?)
                {
                    stack.last_mut().unwrap().text.push_str(value)
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(node)
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(stack.remove(0))
}
fn data(node: Option<&Node>, issues: &mut Vec<String>) -> ChartData {
    let Some(node) = node else {
        return ChartData::default();
    };
    if let Some(v) = node.child("v") {
        return ChartData {
            points: vec![Some(crate::xstring::decode_xstring(&v.text))],
            ..Default::default()
        };
    }
    if node.child("multiLvlStrRef").is_some() {
        issues.push("multilevel categories".into());
    }
    let source = node
        .children
        .iter()
        .find(|n| n.main && matches!(n.name.as_str(), "numRef" | "strRef" | "numLit" | "strLit"));
    let Some(source) = source else {
        return ChartData::default();
    };
    let cache = source
        .child("numCache")
        .or_else(|| source.child("strCache"))
        .unwrap_or(source);
    let mut points = Vec::new();
    if let Some(count) = cache.val("ptCount").and_then(|n| n.parse::<usize>().ok()) {
        if count <= MAX_CHART_POINTS {
            points.resize(count, None)
        } else {
            issues.push("chart point limit".into())
        }
    }
    for pt in cache.children.iter().filter(|n| n.main && n.name == "pt") {
        if let Some(index) = pt.attrs.get("idx").and_then(|n| n.parse::<usize>().ok()) {
            if index < MAX_CHART_POINTS {
                if points.len() <= index {
                    points.resize(index + 1, None)
                }
                points[index] = pt
                    .child("v")
                    .map(|n| crate::xstring::decode_xstring(&n.text));
            } else {
                issues.push("chart point limit".into());
            }
        }
    }
    ChartData {
        formula: source.child("f").map(|f| f.text.trim().to_string()),
        points,
        format_code: cache
            .child("formatCode")
            .map(|f| crate::xstring::decode_xstring(&f.text)),
        numeric: source.name.starts_with("num"),
    }
}
fn color(properties: Option<&Node>, issues: &mut Vec<String>) -> ChartColor {
    let Some(properties) = properties else {
        return ChartColor::default();
    };
    if properties.dchild("gradFill").is_some() || properties.dchild("pattFill").is_some() {
        issues.push("chart gradient/pattern fill".into());
    }
    let fill = properties
        .dchild("solidFill")
        .or_else(|| properties.dchild("ln").and_then(|n| n.dchild("solidFill")));
    let Some(node) = fill.and_then(|fill| fill.children.iter().find(|n| n.drawing)) else {
        return ChartColor::default();
    };
    if !node.children.is_empty() {
        issues.push("chart color transforms".into());
    }
    let val = node.attrs.get("val");
    let rgb = if node.name == "srgbClr" {
        val.and_then(|v| {
            if v.len() != 6 || !v.is_ascii() {
                return None;
            }
            Some([
                u8::from_str_radix(&v[0..2], 16).ok()?,
                u8::from_str_radix(&v[2..4], 16).ok()?,
                u8::from_str_radix(&v[4..6], 16).ok()?,
            ])
        })
    } else {
        None
    };
    ChartColor {
        rgb,
        scheme: (node.name == "schemeClr").then(|| val.cloned()).flatten(),
    }
}
fn axis(node: Option<&Node>) -> ChartAxis {
    let Some(node) = node else {
        return ChartAxis::default();
    };
    let scaling = node.child("scaling");
    ChartAxis {
        minimum: scaling.and_then(|n| n.val("min")).map(str::to_string),
        maximum: scaling.and_then(|n| n.val("max")).map(str::to_string),
        major_unit: node.val("majorUnit").map(str::to_string),
        number_format: node
            .child("numFmt")
            .and_then(|n| n.attrs.get("formatCode"))
            .cloned(),
        reverse: scaling.and_then(|n| n.val("orientation")) == Some("maxMin"),
        hidden: node.enabled("delete", false),
        gridlines: node.child("majorGridlines").is_some(),
        cross_between: node.val("crossBetween").map(str::to_string),
        title: node
            .child("title")
            .map(|title| {
                let text = title.texts();
                if text.is_empty() {
                    // A title with no text of its own reads "Axis Title".
                    "Axis Title".to_string()
                } else {
                    text
                }
            })
            .filter(|_| !node.enabled("delete", false)),
        log_base: scaling.and_then(|n| n.val("logBase")).map(str::to_string),
        date: node.name == "dateAx",
        base_time_unit: node.val("baseTimeUnit").map(str::to_string),
        major_time_unit: node.val("majorTimeUnit").map(str::to_string),
        position: node.val("axPos").map(str::to_string),
        crosses: node.val("crosses").map(str::to_string),
    }
}
fn data_labels(node: Option<&Node>) -> Option<DataLabels> {
    let node = node?;
    if node.enabled("delete", false) {
        return Some(DataLabels::default());
    }
    Some(DataLabels {
        show_value: node.enabled("showVal", false),
        show_category: node.enabled("showCatName", false),
        show_series: node.enabled("showSerName", false),
        show_percent: node.enabled("showPercent", false),
        position: node.val("dLblPos").map(str::to_string),
        number_format: node
            .child("numFmt")
            .filter(|n| n.attrs.get("sourceLinked").is_none_or(|v| v != "1"))
            .and_then(|n| n.attrs.get("formatCode"))
            .cloned(),
        separator: node.child("separator").map(|n| n.text.clone()),
        deleted: node
            .children
            .iter()
            .filter(|n| n.main && n.name == "dLbl" && n.enabled("delete", false))
            .filter_map(|n| n.val("idx")?.parse().ok())
            .collect(),
    })
}
fn trendline(node: &Node, issues: &mut Vec<String>) -> Trendline {
    Trendline {
        kind: node.val("trendlineType").unwrap_or("linear").to_string(),
        order: node
            .val("order")
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .clamp(2, 6),
        period: node
            .val("period")
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .max(2),
        forward: node.val("forward").map(str::to_string),
        backward: node.val("backward").map(str::to_string),
        color: color(node.child("spPr"), issues),
        display_equation: node.enabled("dispEq", false),
        display_r_squared: node.enabled("dispRSqr", false),
    }
}
fn error_bars(node: &Node, issues: &mut Vec<String>) -> ErrorBars {
    ErrorBars {
        direction: node.val("errDir").unwrap_or("y").to_string(),
        bar_type: node.val("errBarType").unwrap_or("both").to_string(),
        value_type: node.val("errValType").unwrap_or("fixedVal").to_string(),
        value: node.val("val").map(str::to_string),
        no_end_cap: node.enabled("noEndCap", false),
        plus: data(node.child("plus"), issues),
        minus: data(node.child("minus"), issues),
        color: color(node.child("spPr"), issues),
    }
}
const PLOT_KINDS: [&str; 8] = [
    "barChart",
    "lineChart",
    "areaChart",
    "pieChart",
    "doughnutChart",
    "scatterChart",
    "radarChart",
    "bubbleChart",
];
/// Plots that can share a chart with others, on value and category axes.
const CARTESIAN_KINDS: [&str; 4] = ["barChart", "lineChart", "areaChart", "scatterChart"];

fn parse_series(
    ser: &Node,
    kind: &str,
    scatter_style: &str,
    index: usize,
    issues: &mut Vec<String>,
) -> ChartSeries {
    let mut series = ChartSeries {
        order: ser
            .val("order")
            .and_then(|v| v.parse().ok())
            .unwrap_or(index as u32),
        name: data(ser.child("tx"), issues),
        categories: data(ser.child("cat"), issues),
        values: data(ser.child("val").or_else(|| ser.child("yVal")), issues),
        x_values: data(ser.child("xVal"), issues),
        color: color(ser.child("spPr"), issues),
        point_colors: BTreeMap::new(),
        marker: ser
            .child("marker")
            .and_then(|n| n.val("symbol"))
            .unwrap_or(
                if (kind == "scatterChart"
                    && (scatter_style.contains("Marker") || scatter_style == "marker"))
                    || kind == "radarChart" && scatter_style == "marker"
                {
                    "circle"
                } else {
                    "none"
                },
            )
            .into(),
        marker_size: ser
            .child("marker")
            .and_then(|n| n.val("size"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(5)
            .clamp(2, 72),
        data_labels: data_labels(ser.child("dLbls")),
        trendlines: Vec::new(),
        error_bars: Vec::new(),
        bubble_sizes: data(ser.child("bubbleSize"), issues),
        smooth: ser
            .child("smooth")
            .is_some_and(|n| n.attrs.get("val").is_none_or(|v| v != "0" && v != "false")),
    };
    for point in ser.children.iter().filter(|n| n.main && n.name == "dPt") {
        if let Some(index) = point.val("idx").and_then(|v| v.parse::<usize>().ok()) {
            series
                .point_colors
                .insert(index, color(point.child("spPr"), issues));
        }
    }
    for node in ser.children.iter().filter(|n| n.main) {
        match node.name.as_str() {
            "trendline" => series.trendlines.push(trendline(node, issues)),
            "errBars" => series.error_bars.push(error_bars(node, issues)),
            _ => {}
        }
    }
    series
}

fn parse_plot(kind: &Node, issues: &mut Vec<String>) -> ChartPlot {
    let name = kind.name.clone();
    let grouping: String = kind
        .val("grouping")
        .unwrap_or(if name == "barChart" {
            "clustered"
        } else {
            "standard"
        })
        .into();
    let scatter_style: String = if name == "radarChart" {
        kind.val("radarStyle").unwrap_or("marker").into()
    } else {
        kind.val("scatterStyle").unwrap_or("marker").into()
    };
    let mut series: Vec<ChartSeries> = kind
        .children
        .iter()
        .filter(|n| n.main && n.name == "ser")
        .enumerate()
        .map(|(index, ser)| parse_series(ser, &name, &scatter_style, index, issues))
        .collect();
    series.sort_by_key(|s| s.order);
    ChartPlot {
        horizontal: kind.val("barDir") == Some("bar"),
        vary_colors: kind
            .val("varyColors")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(matches!(name.as_str(), "pieChart" | "doughnutChart")),
        gap_width: kind
            .val("gapWidth")
            .and_then(|v| v.trim().strip_suffix('%').unwrap_or(v.trim()).parse().ok())
            .unwrap_or(150)
            .min(500),
        overlap: kind
            .val("overlap")
            .and_then(|v| v.trim().strip_suffix('%').unwrap_or(v.trim()).parse().ok())
            .unwrap_or(if grouping.ends_with("Stacked") || grouping == "stacked" {
                100
            } else {
                0
            })
            .clamp(-100, 100),
        grouping,
        scatter_style,
        series,
        data_labels: data_labels(kind.child("dLbls")),
        secondary: false,
        kind: name,
    }
}

pub fn parse_chart(xml: &str) -> Result<Chart> {
    let tree = parse_tree(xml)?;
    let space = tree
        .children
        .iter()
        .find(|n| n.main && n.name == "chartSpace");
    let Some(root) = space.and_then(|n| n.child("chart")) else {
        return Ok(Chart {
            unsupported_properties: vec!["missing classic chart".into()],
            ..Default::default()
        });
    };
    let mut chart = Chart::default();
    chart.pivot_source = pivot::parse(space.unwrap(), &mut chart.unsupported_properties);
    if space.unwrap().child("externalData").is_some() {
        chart
            .unsupported_properties
            .push("external chart data".into());
    }

    let plot = root.child("plotArea");
    let plots: Vec<&Node> = plot
        .into_iter()
        .flat_map(|n| n.children.iter())
        .filter(|n| n.main && n.name.ends_with("Chart"))
        .collect();
    let combinable = plots
        .iter()
        .all(|n| CARTESIAN_KINDS.contains(&n.name.as_str()));
    if plots.is_empty() || (plots.len() > 1 && !combinable) {
        chart
            .unsupported_properties
            .push("combined or missing plot".into());
    }
    let Some(kind) = plots.first() else {
        return Ok(chart);
    };
    chart.kind = kind.name.clone();
    if !PLOT_KINDS.contains(&chart.kind.as_str()) {
        chart
            .unsupported_properties
            .push(format!("chart kind: {}", chart.kind));
    }
    chart.title = root.child("title").map(Node::texts).unwrap_or_default();
    chart.title_source = data(
        root.child("title").and_then(|n| n.child("tx")),
        &mut chart.unsupported_properties,
    );
    chart.legend = root
        .child("legend")
        .map(|n| n.val("legendPos").unwrap_or("r").to_string());
    chart.blank_mode = root.val("dispBlanksAs").unwrap_or("gap").into();
    chart.visible_only = root.enabled("plotVisOnly", true);
    let first = parse_plot(kind, &mut chart.unsupported_properties);
    chart.horizontal = first.horizontal;
    chart.vary_colors = first.vary_colors;
    chart.grouping = first.grouping.clone();
    chart.gap_width = first.gap_width;
    chart.overlap = first.overlap;
    chart.data_labels = first.data_labels.clone();
    chart.series = first.series;
    if chart.kind == "radarChart" {
        chart.radar_style = first.scatter_style;
    } else {
        chart.scatter_style = first.scatter_style;
    }
    chart.hole_size = kind
        .val("holeSize")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .clamp(10, 90);
    chart.first_slice_angle = kind
        .val("firstSliceAng")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        % 360;
    chart.bubble_scale = kind
        .val("bubbleScale")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .min(300);
    chart.bubble_size_width = kind.val("sizeRepresents") == Some("w");

    let axes: Vec<&Node> = plot
        .into_iter()
        .flat_map(|n| n.children.iter())
        .filter(|n| n.main && matches!(n.name.as_str(), "catAx" | "valAx" | "dateAx" | "serAx"))
        .collect();
    let axis_ids = |plot: &Node| -> Vec<String> {
        plot.children
            .iter()
            .filter(|n| n.main && n.name == "axId")
            .filter_map(|n| n.attrs.get("val").cloned())
            .collect()
    };
    let by_id = |id: &str| axes.iter().find(|n| n.val("axId") == Some(id)).copied();
    let primary_ids = axis_ids(kind);
    let scatter_like = matches!(chart.kind.as_str(), "scatterChart" | "bubbleChart");
    // A plot's axes in its own `axId` order: category (or X) first, then value.
    let pick = |ids: &[String], scatter: bool| -> (Option<&Node>, Option<&Node>) {
        let found: Vec<&Node> = ids.iter().filter_map(|id| by_id(id)).collect();
        if found.is_empty() {
            return (None, None);
        }
        if scatter {
            let x = found
                .iter()
                .find(|n| matches!(n.val("axPos"), Some("b" | "t")))
                .or(found.first())
                .copied();
            let y = found
                .iter()
                .find(|n| !x.is_some_and(|x| std::ptr::eq(**n, x)))
                .copied();
            (x, y)
        } else {
            (
                found
                    .iter()
                    .find(|n| matches!(n.name.as_str(), "catAx" | "dateAx"))
                    .copied(),
                found.iter().find(|n| n.name == "valAx").copied(),
            )
        }
    };
    let (category, value) = if primary_ids.is_empty() {
        // Without axis ids, take the axes by kind as before.
        (
            axes.iter()
                .find(|n| {
                    matches!(n.name.as_str(), "catAx" | "dateAx")
                        || (scatter_like && n.val("axPos") == Some("b"))
                })
                .copied(),
            axes.iter()
                .find(|n| n.name == "valAx" && n.val("axPos") != Some("b"))
                .copied()
                .or_else(|| axes.iter().find(|n| n.name == "valAx").copied()),
        )
    } else {
        pick(&primary_ids, scatter_like)
    };
    chart.value_axis = axis(value);
    chart.category_axis = axis(category);
    for extra in plots.iter().skip(1) {
        let mut parsed = parse_plot(extra, &mut chart.unsupported_properties);
        let ids = axis_ids(extra);
        if !ids.is_empty() && ids != primary_ids {
            parsed.secondary = true;
            let (category, value) = pick(&ids, extra.name == "scatterChart");
            if chart.secondary_value_axis.is_none() {
                chart.secondary_value_axis = value.map(|n| axis(Some(n)));
                chart.secondary_category_axis = category.map(|n| axis(Some(n)));
            }
        }
        chart.extra_plots.push(parsed);
    }
    if axes.len() > 4 {
        chart.unsupported_properties.push("secondary axes".into());
    }
    if matches!(chart.kind.as_str(), "pieChart" | "doughnutChart") && chart.series.len() > 1 {
        chart
            .unsupported_properties
            .push("multiple circular series".into());
    }
    let all_series: Vec<&ChartSeries> = chart
        .series
        .iter()
        .chain(chart.extra_plots.iter().flat_map(|plot| plot.series.iter()))
        .collect();
    let mut issues = Vec::new();
    for series in &all_series {
        if !matches!(
            series.marker.as_str(),
            "none"
                | "circle"
                | "square"
                | "diamond"
                | "triangle"
                | "plus"
                | "x"
                | "star"
                | "dash"
                | "dot"
        ) {
            issues.push(format!("marker: {}", series.marker));
        }
        for trendline in &series.trendlines {
            if !matches!(
                trendline.kind.as_str(),
                "linear" | "exp" | "log" | "poly" | "power" | "movingAvg"
            ) {
                issues.push(format!("trendline: {}", trendline.kind));
            }
        }
    }
    chart.unsupported_properties.extend(issues);
    for (name, label) in [
        ("manualLayout", "manual chart layout"),
        ("pivotSource", "misplaced pivot chart source"),
        ("extLst", "chart extensions"),
        ("explosion", "exploded pie"),
        ("noFill", "chart no-fill styling"),
    ] {
        let mut found = Vec::new();
        root.descendants(name, &mut found);
        if !found.is_empty() {
            chart.unsupported_properties.push(label.into());
        }
    }
    chart.unsupported_properties.sort();
    chart.unsupported_properties.dedup();
    Ok(chart)
}
pub(crate) fn apply_theme(chart: &mut Chart, theme: &crate::theme::ThemePalette) {
    for (index, color) in chart.palette.iter_mut().enumerate() {
        if let Some(rgb) = theme.color(index as u32 + 4) {
            *color = rgb
        }
    }
    let apply = |color: &mut ChartColor| {
        if let Some(scheme) = color.scheme.as_deref() {
            let index = match scheme {
                "dk1" | "tx1" => Some(1),
                "lt1" | "bg1" => Some(0),
                "dk2" | "tx2" => Some(3),
                "lt2" | "bg2" => Some(2),
                s if s.starts_with("accent") => s[6..].parse::<u32>().ok().map(|n| n + 3),
                _ => None,
            };
            if let Some(rgb) = index.and_then(|i| theme.color(i)) {
                color.rgb = Some(rgb)
            }
        }
    };
    let extra = chart
        .extra_plots
        .iter_mut()
        .flat_map(|plot| plot.series.iter_mut());
    for series in chart.series.iter_mut().chain(extra) {
        apply(&mut series.color);
        for color in series.point_colors.values_mut() {
            apply(color)
        }
        for trendline in &mut series.trendlines {
            apply(&mut trendline.color)
        }
        for bars in &mut series.error_bars {
            apply(&mut bars.color)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartPointState {
    Live,
    Stale,
    Uncalculated,
}
impl ChartPointState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Stale => "stale",
            Self::Uncalculated => "uncalculated",
        }
    }
}
pub struct ChartReferencePoint<'a> {
    pub reference: crate::CellRef,
    pub cell: Option<&'a crate::Cell>,
    pub hidden: bool,
    pub state: ChartPointState,
}
pub struct ChartReference<'a> {
    pub sheet: &'a crate::Sheet,
    pub points: Vec<ChartReferencePoint<'a>>,
}

/// Direct, single-axis ranges and named ranges. Unresolved/external formulas
/// return None, letting callers retain a cache without claiming live data.
pub fn resolve_chart_reference<'a>(
    formula: &str,
    sheet: &'a crate::Sheet,
    workbook: Option<&'a crate::Workbook>,
) -> Option<ChartReference<'a>> {
    fn range<'a>(
        formula: &str,
        sheet: &'a crate::Sheet,
        workbook: Option<&'a crate::Workbook>,
        seen: &mut std::collections::HashSet<(String, String)>,
    ) -> Option<(&'a crate::Sheet, crate::CellRange)> {
        let formula = formula.trim().trim_start_matches('=');
        let (name, reference) = formula
            .rsplit_once('!')
            .map(|(s, r)| (Some(s), r))
            .unwrap_or((None, formula));
        let name = name.map(|s| {
            if s.starts_with('\'') && s.ends_with('\'') {
                s[1..s.len() - 1].replace("''", "'")
            } else {
                s.to_string()
            }
        });
        if name
            .as_ref()
            .is_some_and(|n| n.contains('[') || n.contains(']'))
        {
            return None;
        }
        let current = match name.as_deref() {
            None => sheet,
            Some(name) if name.eq_ignore_ascii_case(&sheet.name) => sheet,
            Some(name) => workbook?
                .sheets
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))?,
        };
        if let Ok(range) = crate::CellRange::parse(reference) {
            return Some((current, range));
        }
        if seen.len() >= 32
            || !seen.insert((
                current.name.to_ascii_lowercase(),
                reference.to_ascii_lowercase(),
            ))
        {
            return None;
        }
        let book = workbook?;
        let index = book
            .sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(&current.name));
        let definition = book
            .defined_names
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(reference) && n.local_sheet_index == index)
            .or_else(|| {
                book.defined_names.iter().find(|n| {
                    n.name.eq_ignore_ascii_case(reference) && n.local_sheet_index.is_none()
                })
            })?;
        range(&definition.formula, current, workbook, seen)
    }
    let (sheet, range) = range(
        formula,
        sheet,
        workbook,
        &mut std::collections::HashSet::new(),
    )?;
    if range.end.column < range.start.column
        || range.end.row < range.start.row
        || (range.start.column != range.end.column && range.start.row != range.end.row)
    {
        return None;
    }
    let count = (range.end.column - range.start.column + 1) as usize
        * (range.end.row - range.start.row + 1) as usize;
    if count > MAX_CHART_POINTS {
        return None;
    }
    let mut points = Vec::with_capacity(count);
    for row_index in range.start.row..=range.end.row {
        let row = sheet
            .rows
            .binary_search_by_key(&row_index, |r| r.index)
            .ok()
            .map(|i| &sheet.rows[i]);
        for column in range.start.column..=range.end.column {
            let cell = row.and_then(|r| {
                r.cells
                    .binary_search_by_key(&column, |c| c.reference.column)
                    .ok()
                    .map(|i| &r.cells[i])
            });
            let state = if cell.is_some_and(|c| c.formula.is_some()) {
                if cell.is_some_and(|c| c.value.is_empty()) {
                    ChartPointState::Uncalculated
                } else if workbook.is_some_and(|w| w.full_calculation_on_load) {
                    ChartPointState::Stale
                } else {
                    ChartPointState::Live
                }
            } else {
                ChartPointState::Live
            };
            points.push(ChartReferencePoint {
                reference: crate::CellRef::new(column, row_index),
                cell,
                hidden: row.is_some_and(|r| r.hidden)
                    || sheet.column_span(column).is_some_and(|c| c.hidden),
                state,
            });
        }
    }
    Some(ChartReference { sheet, points })
}
