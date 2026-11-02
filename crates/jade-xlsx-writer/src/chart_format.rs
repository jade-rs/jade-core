//! Chart Elements and the Format panes, as edits of a chart part already in
//! the file: axis titles, scales and gridlines; data labels; the data
//! table; a series' fill, line, markers, trendline and error bars; the gap
//! between bars; how empty and hidden cells are drawn; the chart and plot
//! area's fill; the chart style.
//!
//! The part is read into a tree that keeps everything it does not change
//! as it was, and what is added goes where the schema puts it.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::patch::escape;
use crate::{Result, WriteError};

const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

#[derive(Debug, Clone)]
enum Item {
    Element(Element),
    /// Text, comments and the rest, as the file has them.
    Raw(String),
}

#[derive(Debug, Clone)]
struct Element {
    /// The qualified name, `c:valAx`.
    name: String,
    /// Attributes as the file writes them, values still escaped.
    attributes: Vec<(String, String)>,
    children: Vec<Item>,
}

impl Element {
    fn new(name: String) -> Self {
        Self {
            name,
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    fn with(mut self, key: &str, value: &str) -> Self {
        self.attributes.push((key.to_string(), escape(value)));
        self
    }

    fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    fn prefix(&self) -> &str {
        match self.name.rsplit_once(':') {
            Some((prefix, _)) => prefix,
            None => "",
        }
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|item| match item {
            Item::Element(element) => Some(element),
            Item::Raw(_) => None,
        })
    }

    fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|item| match item {
            Item::Element(element) => Some(element),
            Item::Raw(_) => None,
        })
    }

    fn child(&self, local: &str) -> Option<&Element> {
        self.elements().find(|element| element.local() == local)
    }

    fn child_mut(&mut self, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|element| element.local() == local)
    }

    fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn set_attribute(&mut self, key: &str, value: Option<&str>) {
        self.attributes.retain(|(name, _)| name != key);
        if let Some(value) = value {
            self.attributes.push((key.to_string(), escape(value)));
        }
    }

    fn val(&self, local: &str) -> Option<&str> {
        self.child(local)?.attribute("val")
    }

    fn remove(&mut self, local: &str) {
        self.children.retain(|item| match item {
            Item::Element(element) => element.local() != local,
            Item::Raw(_) => true,
        });
    }

    /// Puts `element` in place of its namesake, or — when there is none —
    /// before the first child that `order` puts after it.
    fn put(&mut self, element: Element, order: &[&str]) {
        let local = element.local().to_string();
        if let Some(at) = self
            .children
            .iter()
            .position(|item| matches!(item, Item::Element(child) if child.local() == local))
        {
            self.children[at] = Item::Element(element);
            return;
        }
        let rank = order.iter().position(|name| *name == local);
        let at = self
            .children
            .iter()
            .position(|item| match item {
                Item::Element(child) => {
                    let other = order.iter().position(|name| *name == child.local());
                    match (rank, other) {
                        (Some(rank), Some(other)) => other > rank,
                        _ => false,
                    }
                }
                Item::Raw(_) => false,
            })
            .unwrap_or_else(|| {
                // Before an extension list, which comes last everywhere.
                self.children
                    .iter()
                    .position(
                        |item| matches!(item, Item::Element(child) if child.local() == "extLst"),
                    )
                    .unwrap_or(self.children.len())
            });
        self.children.insert(at, Item::Element(element));
    }

    /// The child `local`, made empty where `order` puts it when missing.
    fn ensure(&mut self, local: &str, order: &[&str]) -> &mut Element {
        if self.child(local).is_none() {
            let name = qualified(self.prefix(), local);
            self.put(Element::new(name), order);
        }
        self.child_mut(local).expect("just put")
    }

    /// `<local val="value"/>` where `order` puts it; none for `None`.
    fn set_val(&mut self, local: &str, value: Option<&str>, order: &[&str]) {
        match value {
            None => self.remove(local),
            Some(value) => {
                let name = qualified(self.prefix(), local);
                self.put(Element::new(name).with("val", value), order);
            }
        }
    }

    fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attributes {
            out.push_str(&format!(" {key}=\"{value}\""));
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for item in &self.children {
            match item {
                Item::Element(element) => element.write(out),
                Item::Raw(text) => out.push_str(text),
            }
        }
        out.push_str(&format!("</{}>", self.name));
    }
}

fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

/// The part's prolog and its root.
fn parse(xml: &str) -> Result<(String, Element)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = vec![Element::new(String::new())];
    let mut prolog = String::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let end = reader.buffer_position() as usize;
        let element = |head: &quick_xml::events::BytesStart<'_>| -> Result<Element> {
            let mut element =
                Element::new(String::from_utf8_lossy(head.name().as_ref()).into_owned());
            for attribute in head.attributes() {
                let attribute = attribute?;
                element.attributes.push((
                    String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                    String::from_utf8_lossy(&attribute.value).into_owned(),
                ));
            }
            Ok(element)
        };
        match event {
            Event::Start(ref head) => stack.push(element(head)?),
            Event::Empty(ref head) => {
                let made = element(head)?;
                stack
                    .last_mut()
                    .expect("root")
                    .children
                    .push(Item::Element(made));
            }
            Event::End(_) => {
                let done = stack.pop().expect("open element");
                stack
                    .last_mut()
                    .ok_or_else(|| WriteError::MissingPart("chart root".into()))?
                    .children
                    .push(Item::Element(done));
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) if stack.len() == 1 => {
                prolog.push_str(&xml[start..end]);
            }
            _ => {
                let raw = xml[start..end].to_string();
                if stack.len() == 1 {
                    prolog.push_str(&raw);
                } else {
                    stack
                        .last_mut()
                        .expect("open element")
                        .children
                        .push(Item::Raw(raw));
                }
            }
        }
    }
    let document = stack
        .pop()
        .ok_or_else(|| WriteError::MissingPart("chart root".into()))?;
    let root = document
        .children
        .into_iter()
        .find_map(|item| match item {
            Item::Element(element) => Some(element),
            Item::Raw(_) => None,
        })
        .ok_or_else(|| WriteError::MissingPart("chart root".into()))?;
    Ok((prolog, root))
}

/// The children of each element, in the schema's order.
const CHART_SPACE: &[&str] = &[
    "date1904",
    "lang",
    "roundedCorners",
    "AlternateContent",
    "style",
    "clrMapOvr",
    "pivotSource",
    "protection",
    "chart",
    "spPr",
    "txPr",
    "externalData",
    "printSettings",
    "userShapes",
    "extLst",
];
const CHART: &[&str] = &[
    "title",
    "autoTitleDeleted",
    "pivotFmts",
    "view3D",
    "floor",
    "sideWall",
    "backWall",
    "plotArea",
    "legend",
    "plotVisOnly",
    "dispBlanksAs",
    "showDLblsOverMax",
    "extLst",
];
const PLOT_AREA_TAIL: &[&str] = &["layout", "dTable", "spPr", "extLst"];
const AXIS: &[&str] = &[
    "axId",
    "scaling",
    "delete",
    "axPos",
    "majorGridlines",
    "minorGridlines",
    "title",
    "numFmt",
    "majorTickMark",
    "minorTickMark",
    "tickLblPos",
    "spPr",
    "txPr",
    "crossAx",
    "crosses",
    "crossesAt",
    "crossBetween",
    "auto",
    "lblAlgn",
    "lblOffset",
    "baseTimeUnit",
    "majorUnit",
    "majorTimeUnit",
    "minorUnit",
    "minorTimeUnit",
    "tickLblSkip",
    "tickMarkSkip",
    "noMultiLvlLbl",
    "dispUnits",
    "extLst",
];
const SCALING: &[&str] = &["logBase", "orientation", "max", "min", "extLst"];
const SERIES: &[&str] = &[
    "idx",
    "order",
    "tx",
    "spPr",
    "invertIfNegative",
    "pictureOptions",
    "marker",
    "dPt",
    "dLbls",
    "trendline",
    "errBars",
    "cat",
    "val",
    "xVal",
    "yVal",
    "smooth",
    "bubbleSize",
    "bubble3D",
    "shape",
    "extLst",
];
const PLOT: &[&str] = &[
    "barDir",
    "radarStyle",
    "scatterStyle",
    "ofPieType",
    "wireframe",
    "grouping",
    "varyColors",
    "ser",
    "dLbls",
    "dropLines",
    "firstSliceAng",
    "holeSize",
    "gapWidth",
    "overlap",
    "serLines",
    "hiLowLines",
    "upDownBars",
    "marker",
    "smooth",
    "bubble3D",
    "bubbleScale",
    "showNegBubbles",
    "sizeRepresents",
    "shape",
    "axId",
    "extLst",
];
const TRENDLINE: &[&str] = &[
    "name",
    "spPr",
    "trendlineType",
    "order",
    "period",
    "forward",
    "backward",
    "intercept",
    "dispRSqr",
    "dispEq",
    "trendlineLbl",
    "extLst",
];
const MARKER: &[&str] = &["symbol", "size", "spPr", "extLst"];
const SHAPE: &[&str] = &[
    "xfrm",
    "custGeom",
    "prstGeom",
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
    "ln",
    "effectLst",
    "effectDag",
    "scene3d",
    "sp3d",
    "extLst",
];
const LINE: &[&str] = &[
    "noFill",
    "solidFill",
    "gradFill",
    "pattFill",
    "prstDash",
    "custDash",
    "round",
    "bevel",
    "miter",
    "headEnd",
    "tailEnd",
    "extLst",
];
const FILLS: &[&str] = &[
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
];

/// Which axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisId {
    /// The category axis, or a scatter's X axis.
    Category,
    Value,
    SecondaryCategory,
    SecondaryValue,
}

/// Where an axis crosses the other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Crosses {
    /// Automatic: at zero.
    AutoZero,
    /// At the other axis' maximum.
    Maximum,
    /// At a value of the other axis (a category number on a category axis).
    At(f64),
}

/// A category axis' kind, as Format Axis ▸ Axis Type has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisKind {
    Automatic,
    Text,
    Date,
}

/// A fill: none, one colour (`RRGGBB`), a gradient between two colours at
/// an angle in degrees, or a pattern (`pct50`, `ltHorz`, …) of a colour
/// on another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    None,
    Solid(String),
    Gradient(String, String, u32),
    Pattern(String, String, String),
}

/// What Format Axis changes; what is left `None` stays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AxisFormat {
    /// A title, or (`Some(None)`) none.
    pub title: Option<Option<String>>,
    /// Bounds and units: a number, or (`Some(None)`) automatic.
    pub minimum: Option<Option<f64>>,
    pub maximum: Option<Option<f64>>,
    pub major_unit: Option<Option<f64>>,
    pub minor_unit: Option<Option<f64>>,
    /// A logarithmic scale of a base, or (`Some(None)`) none.
    pub log_base: Option<Option<f64>>,
    pub reverse: Option<bool>,
    pub crosses: Option<Crosses>,
    pub kind: Option<AxisKind>,
    /// Shown, or deleted.
    pub visible: Option<bool>,
    pub major_gridlines: Option<bool>,
    pub minor_gridlines: Option<bool>,
    /// A number format, or (`Some(None)`) linked to the source.
    pub number_format: Option<Option<String>>,
}

/// What a data label shows and where.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelFormat {
    pub value: bool,
    pub category: bool,
    pub series: bool,
    pub percent: bool,
    /// `ctr`, `inEnd`, `inBase`, `outEnd`, `t`, `b`, `l`, `r`, `bestFit`;
    /// the type's default when `None`.
    pub position: Option<String>,
    pub separator: Option<String>,
}

/// The data table under the plot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableFormat {
    /// Legend keys beside the series' names.
    pub keys: bool,
}

/// A trendline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendlineFormat {
    /// `linear`, `exp`, `log`, `poly`, `power` or `movingAvg`.
    pub kind: String,
    /// A polynomial's order.
    pub order: u32,
    /// A moving average's period.
    pub period: u32,
    pub forward: Option<f64>,
    pub backward: Option<f64>,
    pub display_equation: bool,
    pub display_r_squared: bool,
}

/// Error bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBarFormat {
    /// `x` or `y`.
    pub direction: String,
    /// `both`, `plus` or `minus`.
    pub bar_type: String,
    /// `fixedVal`, `percentage`, `stdDev` or `stdErr`.
    pub value_type: String,
    pub value: f64,
    pub end_cap: bool,
}

/// What Format Data Series changes; what is left `None` stays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeriesFormat {
    /// A fill, or (`Some(None)`) automatic.
    pub fill: Option<Option<Fill>>,
    /// The line's colour (`RRGGBB`), none (`Some(Some(None))`), or
    /// automatic (`Some(None)`).
    pub line_color: Option<Option<Option<String>>>,
    /// Width in points.
    pub line_width: Option<f64>,
    /// `solid`, `dash`, `sysDot`…
    pub line_dash: Option<String>,
    /// A marker's symbol (`none`, `circle`…) and size.
    pub marker: Option<(String, u32)>,
    pub smooth: Option<bool>,
    /// The plot's gap between bars and their overlap, in percent.
    pub gap_width: Option<u32>,
    pub overlap: Option<i32>,
    /// Data labels on this series, or (`Some(None)`) the plot's.
    pub labels: Option<Option<LabelFormat>>,
    /// Its trendline, or (`Some(None)`) none.
    pub trendline: Option<Option<TrendlineFormat>>,
    /// Its error bars, or (`Some(None)`) none.
    pub error_bars: Option<Option<ErrorBarFormat>>,
}

/// One change to a chart part.
// A handful are made at a time: the size of the largest is no matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChartFormat {
    Axis(AxisId, AxisFormat),
    /// Chart Elements ▸ Data Labels on every series, or none.
    Labels(Option<LabelFormat>),
    /// Chart Elements ▸ Data Table, or none.
    Table(Option<TableFormat>),
    /// Format Data Series, on the series at an index in the chart's order.
    Series(usize, SeriesFormat),
    /// Hidden and Empty Cells: `gap`, `zero` or `span`; data in hidden
    /// rows and columns shown.
    Blanks(String, bool),
    /// The chart area's fill (`false`) or the plot area's (`true`); `None`
    /// for automatic.
    AreaFill(bool, Option<Fill>),
    /// Chart Styles: `c:style`, 1 to 48.
    Style(u32),
}

/// The chart part with `changes` made.
pub fn format_chart_part(xml: &str, changes: &[ChartFormat]) -> Result<String> {
    let (prolog, mut root) = parse(xml)?;
    let c = root.prefix().to_string();
    // The prefix DrawingML is written with, declared on the root when the
    // part has none.
    let a = root
        .attributes
        .iter()
        .find(|(key, value)| key.starts_with("xmlns:") && value == DRAWINGML_NS)
        .map(|(key, _)| key["xmlns:".len()..].to_string())
        .unwrap_or_else(|| {
            root.attributes
                .push(("xmlns:a".to_string(), DRAWINGML_NS.to_string()));
            "a".to_string()
        });
    let names = Names { c, a };
    for change in changes {
        apply(&mut root, &names, change)?;
    }
    let mut out = prolog;
    root.write(&mut out);
    Ok(out)
}

struct Names {
    c: String,
    a: String,
}

impl Names {
    fn c(&self, local: &str) -> Element {
        Element::new(qualified(&self.c, local))
    }
    fn a(&self, local: &str) -> Element {
        Element::new(qualified(&self.a, local))
    }
    fn val(&self, local: &str, value: &str) -> Element {
        self.c(local).with("val", value)
    }
}

fn missing(what: &str) -> WriteError {
    WriteError::MissingPart(what.to_string())
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// The chart-type elements of a plot area.
fn is_plot(element: &Element) -> bool {
    element.local().ends_with("Chart")
}

fn is_axis(element: &Element) -> bool {
    matches!(element.local(), "catAx" | "valAx" | "dateAx" | "serAx")
}

/// The axis ids each plot is drawn against, the first plot's first.
fn axis_ids(plot_area: &Element) -> (Vec<String>, Option<Vec<String>>) {
    let ids = |plot: &Element| -> Vec<String> {
        plot.elements()
            .filter(|e| e.local() == "axId")
            .filter_map(|e| e.attribute("val").map(str::to_string))
            .collect()
    };
    let plots: Vec<&Element> = plot_area.elements().filter(|e| is_plot(e)).collect();
    let primary = plots.first().map(|plot| ids(plot)).unwrap_or_default();
    let secondary = plots
        .iter()
        .skip(1)
        .map(|plot| ids(plot))
        .find(|other| !other.is_empty() && *other != primary);
    (primary, secondary)
}

/// The axis element `id` names, by its index among the plot area's children.
fn find_axis(plot_area: &Element, id: AxisId) -> Option<usize> {
    let (primary, secondary) = axis_ids(plot_area);
    let ids = match id {
        AxisId::Category | AxisId::Value => primary,
        AxisId::SecondaryCategory | AxisId::SecondaryValue => secondary?,
    };
    let category = matches!(id, AxisId::Category | AxisId::SecondaryCategory);
    let axes: Vec<(usize, &Element)> = plot_area
        .children
        .iter()
        .enumerate()
        .filter_map(|(at, item)| match item {
            Item::Element(e)
                if is_axis(e) && e.val("axId").is_some_and(|v| ids.iter().any(|id| id == v)) =>
            {
                Some((at, e))
            }
            _ => None,
        })
        .collect();
    let all_values = axes.iter().all(|(_, e)| e.local() == "valAx");
    let found = if all_values {
        // A scatter: X along the bottom or top.
        let x = axes
            .iter()
            .find(|(_, e)| matches!(e.val("axPos"), Some("b" | "t")))
            .or(axes.first());
        if category {
            x
        } else {
            axes.iter()
                .find(|(at, _)| Some(*at) != x.map(|(at, _)| *at))
        }
    } else if category {
        axes.iter()
            .find(|(_, e)| matches!(e.local(), "catAx" | "dateAx"))
    } else {
        axes.iter().find(|(_, e)| e.local() == "valAx")
    };
    found.map(|(at, _)| *at)
}

fn plot_area_mut(root: &mut Element) -> Result<&mut Element> {
    root.child_mut("chart")
        .and_then(|chart| chart.child_mut("plotArea"))
        .ok_or_else(|| missing("c:plotArea"))
}

/// A rich-text title of `text`, as a chart part writes one.
fn title(names: &Names, text: &str) -> Element {
    let a = &names.a;
    let paragraphs: String = text
        .split('\n')
        .map(|line| {
            format!(
                "<{a}:p><{a}:r><{a}:t>{}</{a}:t></{a}:r></{a}:p>",
                escape(line)
            )
        })
        .collect();
    let mut rich = names.c("rich");
    rich.children.push(Item::Raw(format!(
        "<{a}:bodyPr/><{a}:lstStyle/>{paragraphs}"
    )));
    let mut tx = names.c("tx");
    tx.children.push(Item::Element(rich));
    let mut title = names.c("title");
    title.children.push(Item::Element(tx));
    title
        .children
        .push(Item::Element(names.val("overlay", "0")));
    title
}

fn color(names: &Names, rgb: &str) -> Element {
    names.a("srgbClr").with("val", rgb.trim_start_matches('#'))
}

fn fill_element(names: &Names, fill: &Fill) -> Element {
    match fill {
        Fill::None => names.a("noFill"),
        Fill::Solid(rgb) => {
            let mut solid = names.a("solidFill");
            solid.children.push(Item::Element(color(names, rgb)));
            solid
        }
        Fill::Gradient(from, to, angle) => {
            let stop = |position: &str, rgb: &str| {
                let mut stop = names.a("gs").with("pos", position);
                stop.children.push(Item::Element(color(names, rgb)));
                Item::Element(stop)
            };
            let mut list = names.a("gsLst");
            list.children.push(stop("0", from));
            list.children.push(stop("100000", to));
            let mut gradient = names.a("gradFill").with("rotWithShape", "1");
            gradient.children.push(Item::Element(list));
            gradient.children.push(Item::Element(
                names
                    .a("lin")
                    .with("ang", &(u64::from(*angle % 360) * 60_000).to_string())
                    .with("scaled", "0"),
            ));
            gradient
        }
        Fill::Pattern(preset, foreground, background) => {
            let mut pattern = names.a("pattFill").with("prst", preset);
            let mut fg = names.a("fgClr");
            fg.children.push(Item::Element(color(names, foreground)));
            let mut bg = names.a("bgClr");
            bg.children.push(Item::Element(color(names, background)));
            pattern.children.push(Item::Element(fg));
            pattern.children.push(Item::Element(bg));
            pattern
        }
    }
}

/// `spPr`'s fill set to `fill`, or (`None`) automatic.
fn set_fill(properties: &mut Element, names: &Names, fill: Option<&Fill>) {
    for name in FILLS {
        properties.remove(name);
    }
    if let Some(fill) = fill {
        properties.put(fill_element(names, fill), SHAPE);
    }
}

/// The element's `c:spPr`, made when missing where `order` puts it.
fn shape_properties<'a>(element: &'a mut Element, order: &[&str]) -> &'a mut Element {
    element.ensure("spPr", order)
}

fn labels_element(names: &Names, labels: &LabelFormat) -> Element {
    let mut element = names.c("dLbls");
    let flag = |on: bool| if on { "1" } else { "0" };
    if let Some(position) = &labels.position {
        element
            .children
            .push(Item::Element(names.val("dLblPos", position)));
    }
    for (local, on) in [
        ("showLegendKey", false),
        ("showVal", labels.value),
        ("showCatName", labels.category),
        ("showSerName", labels.series),
        ("showPercent", labels.percent),
        ("showBubbleSize", false),
    ] {
        element
            .children
            .push(Item::Element(names.val(local, flag(on))));
    }
    if let Some(separator) = &labels.separator {
        let mut sep = names.c("separator");
        sep.children.push(Item::Raw(escape(separator)));
        element.children.push(Item::Element(sep));
    }
    element
}

/// The chart's series, each with its plot's index among the plot area's
/// children and its own among the plot's, in the chart's order.
fn series_at(plot_area: &Element) -> Vec<(usize, usize)> {
    let mut all: Vec<(u32, usize, usize)> = Vec::new();
    for (plot_at, item) in plot_area.children.iter().enumerate() {
        let Item::Element(plot) = item else { continue };
        if !is_plot(plot) {
            continue;
        }
        for (series_at, item) in plot.children.iter().enumerate() {
            let Item::Element(series) = item else {
                continue;
            };
            if series.local() == "ser" {
                let order = series
                    .val("order")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(all.len() as u32);
                all.push((order, plot_at, series_at));
            }
        }
    }
    all.sort_by_key(|(order, _, _)| *order);
    all.into_iter()
        .map(|(_, plot, series)| (plot, series))
        .collect()
}

fn element_at(element: &mut Element, at: usize) -> &mut Element {
    match &mut element.children[at] {
        Item::Element(element) => element,
        Item::Raw(_) => unreachable!("an element's index"),
    }
}

fn apply(root: &mut Element, names: &Names, change: &ChartFormat) -> Result<()> {
    match change {
        ChartFormat::Axis(id, format) => {
            let plot_area = plot_area_mut(root)?;
            let at = find_axis(plot_area, *id).ok_or_else(|| missing("c:valAx"))?;
            let axis = element_at(plot_area, at);
            format_axis(axis, names, format);
        }
        ChartFormat::Labels(labels) => {
            let plot_area = plot_area_mut(root)?;
            for plot in plot_area.elements_mut().filter(|e| is_plot(e)) {
                // The series' own labels go: the plot's speak for them all.
                for series in plot.elements_mut().filter(|e| e.local() == "ser") {
                    series.remove("dLbls");
                }
                match labels {
                    Some(labels) => plot.put(labels_element(names, labels), PLOT),
                    None => plot.remove("dLbls"),
                }
            }
        }
        ChartFormat::Table(table) => {
            let plot_area = plot_area_mut(root)?;
            match table {
                None => plot_area.remove("dTable"),
                Some(table) => {
                    let mut element = names.c("dTable");
                    for (local, on) in [
                        ("showHorzBorder", true),
                        ("showVertBorder", true),
                        ("showOutline", true),
                        ("showKeys", table.keys),
                    ] {
                        element
                            .children
                            .push(Item::Element(names.val(local, if on { "1" } else { "0" })));
                    }
                    plot_area.put(element, PLOT_AREA_TAIL);
                }
            }
        }
        ChartFormat::Series(index, format) => {
            let plot_area = plot_area_mut(root)?;
            let (plot_at, series_at) = *series_at(plot_area)
                .get(*index)
                .ok_or_else(|| missing("c:ser"))?;
            let plot = element_at(plot_area, plot_at);
            format_plot(plot, names, format);
            let series = element_at(plot, series_at);
            format_series(series, names, format);
        }
        ChartFormat::Blanks(mode, hidden) => {
            let chart = root.child_mut("chart").ok_or_else(|| missing("c:chart"))?;
            chart.set_val("plotVisOnly", Some(if *hidden { "0" } else { "1" }), CHART);
            chart.set_val("dispBlanksAs", Some(mode), CHART);
        }
        ChartFormat::AreaFill(plot, fill) => {
            let target = if *plot { plot_area_mut(root)? } else { root };
            let order: &[&str] = if *plot { PLOT_AREA_TAIL } else { CHART_SPACE };
            let properties = shape_properties(target, order);
            set_fill(properties, names, fill.as_ref());
        }
        ChartFormat::Style(style) => {
            // An Excel 2010 `mc:AlternateContent` holding the style goes:
            // the plain one says it for every version.
            root.remove("AlternateContent");
            root.set_val("style", Some(&style.to_string()), CHART_SPACE);
        }
    }
    Ok(())
}

fn format_axis(axis: &mut Element, names: &Names, format: &AxisFormat) {
    if let Some(title_text) = &format.title {
        match title_text {
            Some(text) => axis.put(title(names, text), AXIS),
            None => axis.remove("title"),
        }
    }
    if let Some(on) = format.major_gridlines {
        if on {
            if axis.child("majorGridlines").is_none() {
                axis.put(names.c("majorGridlines"), AXIS);
            }
        } else {
            axis.remove("majorGridlines");
        }
    }
    if let Some(on) = format.minor_gridlines {
        if on {
            if axis.child("minorGridlines").is_none() {
                axis.put(names.c("minorGridlines"), AXIS);
            }
        } else {
            axis.remove("minorGridlines");
        }
    }
    {
        let scaling = axis.ensure("scaling", AXIS);
        if scaling.child("orientation").is_none() {
            scaling.set_val("orientation", Some("minMax"), SCALING);
        }
        if let Some(reverse) = format.reverse {
            scaling.set_val(
                "orientation",
                Some(if reverse { "maxMin" } else { "minMax" }),
                SCALING,
            );
        }
        if let Some(minimum) = format.minimum {
            scaling.set_val("min", minimum.map(number).as_deref(), SCALING);
        }
        if let Some(maximum) = format.maximum {
            scaling.set_val("max", maximum.map(number).as_deref(), SCALING);
        }
        if let Some(base) = format.log_base {
            scaling.set_val("logBase", base.map(number).as_deref(), SCALING);
        }
    }
    // Units are a value or date axis's; a category axis has none.
    if matches!(axis.local(), "valAx" | "dateAx") {
        if let Some(unit) = format.major_unit {
            axis.set_val("majorUnit", unit.map(number).as_deref(), AXIS);
        }
        if let Some(unit) = format.minor_unit {
            axis.set_val("minorUnit", unit.map(number).as_deref(), AXIS);
        }
    }
    if let Some(visible) = format.visible {
        axis.set_val("delete", Some(if visible { "0" } else { "1" }), AXIS);
    }
    if let Some(crosses) = &format.crosses {
        axis.remove("crosses");
        axis.remove("crossesAt");
        match crosses {
            Crosses::AutoZero => axis.set_val("crosses", Some("autoZero"), AXIS),
            Crosses::Maximum => axis.set_val("crosses", Some("max"), AXIS),
            Crosses::At(value) => axis.set_val("crossesAt", Some(&number(*value)), AXIS),
        }
    }
    if let Some(code) = &format.number_format {
        match code {
            Some(code) => axis.put(
                names
                    .c("numFmt")
                    .with("formatCode", code)
                    .with("sourceLinked", "0"),
                AXIS,
            ),
            None => axis.put(
                names
                    .c("numFmt")
                    .with("formatCode", "General")
                    .with("sourceLinked", "1"),
                AXIS,
            ),
        }
    }
    if let Some(kind) = format.kind {
        if matches!(axis.local(), "catAx" | "dateAx") {
            let date = kind == AxisKind::Date;
            axis.name = qualified(&names.c, if date { "dateAx" } else { "catAx" });
            axis.set_val(
                "auto",
                Some(if kind == AxisKind::Automatic {
                    "1"
                } else {
                    "0"
                }),
                AXIS,
            );
            if date {
                axis.remove("lblAlgn");
                axis.remove("noMultiLvlLbl");
                axis.remove("tickLblSkip");
                axis.remove("tickMarkSkip");
                if axis.child("lblOffset").is_none() {
                    axis.set_val("lblOffset", Some("100"), AXIS);
                }
                if axis.child("baseTimeUnit").is_none() {
                    axis.set_val("baseTimeUnit", Some("days"), AXIS);
                }
            } else {
                for local in ["baseTimeUnit", "majorTimeUnit", "minorTimeUnit"] {
                    axis.remove(local);
                }
                if axis.child("lblAlgn").is_none() {
                    axis.set_val("lblAlgn", Some("ctr"), AXIS);
                }
                if axis.child("lblOffset").is_none() {
                    axis.set_val("lblOffset", Some("100"), AXIS);
                }
                if axis.child("noMultiLvlLbl").is_none() {
                    axis.set_val("noMultiLvlLbl", Some("0"), AXIS);
                }
            }
        }
    }
}

/// What of Format Data Series is the plot's: the gap and overlap of its
/// bars.
fn format_plot(plot: &mut Element, names: &Names, format: &SeriesFormat) {
    let _ = names;
    if plot.local() != "barChart" {
        return;
    }
    if let Some(gap) = format.gap_width {
        plot.set_val("gapWidth", Some(&gap.min(500).to_string()), PLOT);
    }
    if let Some(overlap) = format.overlap {
        plot.set_val("overlap", Some(&overlap.clamp(-100, 100).to_string()), PLOT);
    }
}

fn format_series(series: &mut Element, names: &Names, format: &SeriesFormat) {
    if let Some(fill) = &format.fill {
        let properties = shape_properties(series, SERIES);
        set_fill(properties, names, fill.as_ref());
    }
    let line_changed =
        format.line_color.is_some() || format.line_width.is_some() || format.line_dash.is_some();
    if line_changed {
        let properties = shape_properties(series, SERIES);
        // `a:ln`, DrawingML's, in the chart's `c:spPr`.
        if properties.child("ln").is_none() {
            properties.put(names.a("ln"), SHAPE);
        }
        let line = properties.child_mut("ln").expect("just put");
        if let Some(width) = format.line_width {
            line.set_attribute("w", Some(&((width * 12_700.0).round() as i64).to_string()));
        }
        if let Some(color) = &format.line_color {
            for name in ["noFill", "solidFill", "gradFill", "pattFill"] {
                line.remove(name);
            }
            match color {
                None => {}
                Some(None) => line.put(names.a("noFill"), LINE),
                Some(Some(rgb)) => line.put(fill_element(names, &Fill::Solid(rgb.clone())), LINE),
            }
        }
        if let Some(dash) = &format.line_dash {
            line.put(names.a("prstDash").with("val", dash), LINE);
        }
    }
    if let Some((symbol, size)) = &format.marker {
        let marker = series.ensure("marker", SERIES);
        marker.set_val("symbol", Some(symbol), MARKER);
        if symbol == "none" {
            marker.remove("size");
        } else {
            marker.set_val("size", Some(&(*size).clamp(2, 72).to_string()), MARKER);
        }
    }
    if let Some(smooth) = format.smooth {
        if series.child("smooth").is_some() || smooth {
            series.set_val("smooth", Some(if smooth { "1" } else { "0" }), SERIES);
        }
    }
    if let Some(labels) = &format.labels {
        match labels {
            Some(labels) => series.put(labels_element(names, labels), SERIES),
            None => series.remove("dLbls"),
        }
    }
    if let Some(trendline) = &format.trendline {
        series.remove("trendline");
        if let Some(trendline) = trendline {
            let mut element = names.c("trendline");
            let mut put = |e: Element| element.put(e, TRENDLINE);
            put(names.val("trendlineType", &trendline.kind));
            if trendline.kind == "poly" {
                put(names.val("order", &trendline.order.clamp(2, 6).to_string()));
            }
            if trendline.kind == "movingAvg" {
                put(names.val("period", &trendline.period.max(2).to_string()));
            }
            if let Some(forward) = trendline.forward {
                put(names.val("forward", &number(forward)));
            }
            if let Some(backward) = trendline.backward {
                put(names.val("backward", &number(backward)));
            }
            put(names.val(
                "dispRSqr",
                if trendline.display_r_squared {
                    "1"
                } else {
                    "0"
                },
            ));
            put(names.val("dispEq", if trendline.display_equation { "1" } else { "0" }));
            series.put(element, SERIES);
        }
    }
    if let Some(bars) = &format.error_bars {
        series.remove("errBars");
        if let Some(bars) = bars {
            let mut element = names.c("errBars");
            // `errDir` only where the series has both directions.
            if series.child("xVal").is_some() {
                element
                    .children
                    .push(Item::Element(names.val("errDir", &bars.direction)));
            }
            element
                .children
                .push(Item::Element(names.val("errBarType", &bars.bar_type)));
            element
                .children
                .push(Item::Element(names.val("errValType", &bars.value_type)));
            element.children.push(Item::Element(
                names.val("noEndCap", if bars.end_cap { "0" } else { "1" }),
            ));
            if matches!(
                bars.value_type.as_str(),
                "fixedVal" | "percentage" | "stdDev"
            ) {
                element
                    .children
                    .push(Item::Element(names.val("val", &number(bars.value))));
            }
            series.put(element, SERIES);
        }
    }
}
