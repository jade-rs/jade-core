//! Charts made and changed as Excel's Insert ▸ Chart and Chart Design do.
//!
//! A new chart is a chart part written the way Excel 2016 writes its
//! defaults, a frame for it in the sheet's drawing (made, with its
//! relationship and content type, when the sheet has none), and caches the
//! export fills. A change to a chart already in the file edits its part in
//! place: the title and the legend are replaced where they are, and a new
//! type or new data rebuilds the plot area's chart and axes while keeping
//! its layout and fill.

use jade_opc::OpcPackage;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::notes::{add_relationship, ensure_override, free_part_name, relative_target};
use crate::objects::ShapeAnchor;
use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::{Result, WriteError};

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const RELATIONSHIPS_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SPREADSHEET_DRAWING_NS: &str =
    "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const DRAWING_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const CHART_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
const DRAWING_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
const CHART_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

/// The chart types Insert ▸ Chart offers: each of Excel's 2-D sub-types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartKind {
    Column,
    StackedColumn,
    PercentStackedColumn,
    Bar,
    StackedBar,
    PercentStackedBar,
    Line,
    StackedLine,
    PercentStackedLine,
    LineWithMarkers,
    StackedLineWithMarkers,
    PercentStackedLineWithMarkers,
    Area,
    StackedArea,
    PercentStackedArea,
    Pie,
    Doughnut,
    /// Markers only.
    Scatter,
    ScatterSmoothMarkers,
    ScatterSmooth,
    ScatterLinesMarkers,
    ScatterLines,
    Bubble,
    Radar,
    RadarWithMarkers,
    FilledRadar,
    /// A combination: each series drawn as its own
    /// [`SeriesReferences::kind`], a clustered column when it has none.
    Combo,
}

impl ChartKind {
    /// Pies and doughnuts: one series, a slice per category, no axes.
    pub fn is_round(self) -> bool {
        matches!(self, ChartKind::Pie | ChartKind::Doughnut)
    }

    /// Scatter and bubble charts: numbers along both axes.
    pub fn is_xy(self) -> bool {
        matches!(
            self,
            ChartKind::Scatter
                | ChartKind::ScatterSmoothMarkers
                | ChartKind::ScatterSmooth
                | ChartKind::ScatterLinesMarkers
                | ChartKind::ScatterLines
                | ChartKind::Bubble
        )
    }

    /// Bars that run across, with the categories up the side.
    pub fn is_horizontal(self) -> bool {
        matches!(
            self,
            ChartKind::Bar | ChartKind::StackedBar | ChartKind::PercentStackedBar
        )
    }

    /// The plot's `c:grouping`: `clustered` or `standard`, `stacked`,
    /// `percentStacked`.
    pub fn grouping(self) -> &'static str {
        match self {
            ChartKind::StackedColumn
            | ChartKind::StackedBar
            | ChartKind::StackedLine
            | ChartKind::StackedLineWithMarkers
            | ChartKind::StackedArea => "stacked",
            ChartKind::PercentStackedColumn
            | ChartKind::PercentStackedBar
            | ChartKind::PercentStackedLine
            | ChartKind::PercentStackedLineWithMarkers
            | ChartKind::PercentStackedArea => "percentStacked",
            ChartKind::Column | ChartKind::Bar | ChartKind::Combo => "clustered",
            _ => "standard",
        }
    }

    /// The chart element that draws it.
    pub fn element(self) -> &'static str {
        match self {
            ChartKind::Column
            | ChartKind::StackedColumn
            | ChartKind::PercentStackedColumn
            | ChartKind::Bar
            | ChartKind::StackedBar
            | ChartKind::PercentStackedBar
            | ChartKind::Combo => "barChart",
            ChartKind::Line
            | ChartKind::StackedLine
            | ChartKind::PercentStackedLine
            | ChartKind::LineWithMarkers
            | ChartKind::StackedLineWithMarkers
            | ChartKind::PercentStackedLineWithMarkers => "lineChart",
            ChartKind::Area | ChartKind::StackedArea | ChartKind::PercentStackedArea => "areaChart",
            ChartKind::Pie => "pieChart",
            ChartKind::Doughnut => "doughnutChart",
            ChartKind::Scatter
            | ChartKind::ScatterSmoothMarkers
            | ChartKind::ScatterSmooth
            | ChartKind::ScatterLinesMarkers
            | ChartKind::ScatterLines => "scatterChart",
            ChartKind::Bubble => "bubbleChart",
            ChartKind::Radar | ChartKind::RadarWithMarkers | ChartKind::FilledRadar => "radarChart",
        }
    }

    /// Whether its points carry markers.
    fn markers(self) -> bool {
        matches!(
            self,
            ChartKind::LineWithMarkers
                | ChartKind::StackedLineWithMarkers
                | ChartKind::PercentStackedLineWithMarkers
                | ChartKind::Scatter
                | ChartKind::ScatterSmoothMarkers
                | ChartKind::ScatterLinesMarkers
                | ChartKind::RadarWithMarkers
        )
    }
}

/// Where the legend sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegendPosition {
    Right,
    Left,
    Top,
    Bottom,
}

impl LegendPosition {
    fn code(self) -> &'static str {
        match self {
            LegendPosition::Right => "r",
            LegendPosition::Left => "l",
            LegendPosition::Top => "t",
            LegendPosition::Bottom => "b",
        }
    }
}

/// One series by its references, sheet-qualified and absolute, as a chart
/// part writes them (`Operations!$B$2:$B$4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesReferences {
    /// A reference to the cell naming it, or its name as text in quotes
    /// (`"Sales"`).
    pub name: Option<String>,
    /// The categories, or a scatter's or bubble's X values.
    pub categories: Option<String>,
    /// Whether the categories are text, written as a string reference.
    pub text_categories: bool,
    pub values: String,
    /// A bubble's sizes.
    #[serde(default)]
    pub bubble_sizes: Option<String>,
    /// In a combination, the type this series is drawn as.
    #[serde(default)]
    pub kind: Option<ChartKind>,
    /// Drawn against the secondary axis.
    #[serde(default)]
    pub secondary: bool,
}

fn reference(prefix: &str, element: &str, numeric: bool, formula: &str) -> String {
    let kind = if numeric { "numRef" } else { "strRef" };
    format!(
        "<{prefix}{element}><{prefix}{kind}><{prefix}f>{}</{prefix}f></{prefix}{kind}></{prefix}{element}>",
        escape(formula)
    )
}

/// A name typed as text, `"Sales"`, as Excel's `="Sales"` is: its text.
fn literal(name: &str) -> Option<String> {
    let inner = name.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\"\"", "\""))
}

fn series_xml(prefix: &str, kind: ChartKind, index: usize, series: &SeriesReferences) -> String {
    let p = prefix;
    let mut xml = format!("<{p}ser><{p}idx val=\"{index}\"/><{p}order val=\"{index}\"/>");
    if let Some(name) = &series.name {
        match literal(name) {
            Some(text) => xml.push_str(&format!("<{p}tx><{p}v>{}</{p}v></{p}tx>", escape(&text))),
            None => xml.push_str(&reference(p, "tx", false, name)),
        }
    }
    let categories = |element: &str| {
        series
            .categories
            .as_deref()
            .map(|formula| reference(p, element, !series.text_categories, formula))
            .unwrap_or_default()
    };
    let marker = || {
        if kind.markers() {
            format!("<{p}marker><{p}symbol val=\"circle\"/><{p}size val=\"5\"/></{p}marker>")
        } else {
            format!("<{p}marker><{p}symbol val=\"none\"/></{p}marker>")
        }
    };
    match kind.element() {
        "barChart" => {
            xml.push_str(&format!("<{p}invertIfNegative val=\"0\"/>"));
            xml.push_str(&categories("cat"));
            xml.push_str(&reference(p, "val", true, &series.values));
        }
        "lineChart" => {
            xml.push_str(&marker());
            xml.push_str(&categories("cat"));
            xml.push_str(&reference(p, "val", true, &series.values));
            xml.push_str(&format!("<{p}smooth val=\"0\"/>"));
        }
        "radarChart" => {
            if kind != ChartKind::FilledRadar {
                xml.push_str(&marker());
            }
            xml.push_str(&categories("cat"));
            xml.push_str(&reference(p, "val", true, &series.values));
        }
        "scatterChart" => {
            if kind == ChartKind::Scatter {
                // Markers only, as Excel's first scatter is.
                xml.push_str(&format!(
                    "<{p}spPr><a:ln w=\"19050\" xmlns:a=\"{DRAWINGML_NS}\"><a:noFill/></a:ln></{p}spPr>"
                ));
            }
            xml.push_str(&marker());
            xml.push_str(&categories("xVal"));
            xml.push_str(&reference(p, "yVal", true, &series.values));
            let smooth = matches!(
                kind,
                ChartKind::ScatterSmooth | ChartKind::ScatterSmoothMarkers
            );
            xml.push_str(&format!("<{p}smooth val=\"{}\"/>", u8::from(smooth)));
        }
        "bubbleChart" => {
            xml.push_str(&format!("<{p}invertIfNegative val=\"0\"/>"));
            xml.push_str(&categories("xVal"));
            xml.push_str(&reference(p, "yVal", true, &series.values));
            if let Some(sizes) = &series.bubble_sizes {
                xml.push_str(&reference(p, "bubbleSize", true, sizes));
            }
            xml.push_str(&format!("<{p}bubble3D val=\"0\"/>"));
        }
        // Areas, pies and doughnuts.
        _ => {
            xml.push_str(&categories("cat"));
            xml.push_str(&reference(p, "val", true, &series.values));
        }
    }
    xml.push_str(&format!("</{p}ser>"));
    xml
}

const CATEGORY_AXIS: u32 = 500_000_001;
const VALUE_AXIS: u32 = 500_000_002;
const SECONDARY_CATEGORY_AXIS: u32 = 500_000_003;
const SECONDARY_VALUE_AXIS: u32 = 500_000_004;

/// A pair of axes: (category or X axis, value axis) ids.
type AxisPair = (u32, u32);
const PRIMARY: AxisPair = (CATEGORY_AXIS, VALUE_AXIS);
const SECONDARY: AxisPair = (SECONDARY_CATEGORY_AXIS, SECONDARY_VALUE_AXIS);

/// The axes a plot of `kind` is drawn against, `ids`; the secondary pair
/// hides its category axis and puts its values on the right, crossing at
/// the maximum, as Excel's secondary axis does.
fn axes(prefix: &str, kind: ChartKind, ids: AxisPair, secondary: bool) -> String {
    let p = prefix;
    let (category_id, value_id) = ids;
    let horizontal = kind.is_horizontal();
    let (category_position, value_position) = match (horizontal, secondary) {
        (false, false) => ("b", "l"),
        (true, false) => ("l", "b"),
        (false, true) => ("b", "r"),
        (true, true) => ("l", "t"),
    };
    let common = |id: u32, position: &str, deleted: bool| {
        format!(
            "<{p}axId val=\"{id}\"/><{p}scaling><{p}orientation val=\"minMax\"/></{p}scaling><{p}delete val=\"{}\"/><{p}axPos val=\"{position}\"/>",
            u8::from(deleted)
        )
    };
    let format = if kind.grouping() == "percentStacked" {
        "0%"
    } else {
        "General"
    };
    let ticks = |format: &str| {
        format!(
            "<{p}numFmt formatCode=\"{format}\" sourceLinked=\"1\"/><{p}majorTickMark val=\"{}\"/><{p}minorTickMark val=\"none\"/><{p}tickLblPos val=\"nextTo\"/>",
            if kind.element() == "radarChart" { "cross" } else { "none" }
        )
    };
    let crosses = if secondary { "max" } else { "autoZero" };
    let value_axis = |id: u32,
                      position: &str,
                      cross: u32,
                      gridlines: bool,
                      between: &str,
                      crosses: &str,
                      deleted: bool| {
        format!(
            "<{p}valAx>{}{}{}<{p}crossAx val=\"{cross}\"/><{p}crosses val=\"{crosses}\"/><{p}crossBetween val=\"{between}\"/></{p}valAx>",
            common(id, position, deleted),
            if gridlines {
                format!("<{p}majorGridlines/>")
            } else {
                String::new()
            },
            ticks(format),
        )
    };
    if kind.is_round() {
        return String::new();
    }
    if kind.is_xy() {
        return value_axis(
            category_id,
            "b",
            value_id,
            false,
            "midCat",
            "autoZero",
            secondary,
        ) + &value_axis(
            value_id,
            if secondary { "r" } else { "l" },
            category_id,
            !secondary,
            "midCat",
            crosses,
            false,
        );
    }
    let radar = kind.element() == "radarChart";
    format!(
        "<{p}catAx>{}{}{}<{p}crossAx val=\"{value_id}\"/><{p}crosses val=\"autoZero\"/><{p}auto val=\"1\"/><{p}lblAlgn val=\"ctr\"/><{p}lblOffset val=\"100\"/><{p}noMultiLvlLbl val=\"0\"/></{p}catAx>",
        common(category_id, category_position, secondary),
        if radar {
            format!("<{p}majorGridlines/>")
        } else {
            String::new()
        },
        ticks("General"),
    ) + &value_axis(
        value_id,
        value_position,
        category_id,
        !secondary,
        "between",
        crosses,
        false,
    )
}

/// One chart-type element over `series` (each with its index in the
/// chart), against the axes `ids`.
fn group_xml(
    prefix: &str,
    kind: ChartKind,
    series: &[(usize, &SeriesReferences)],
    ids: AxisPair,
) -> String {
    let p = prefix;
    let series: String = series
        .iter()
        .map(|(index, series)| series_xml(p, kind, *index, series))
        .collect();
    let axis_ids = format!("<{p}axId val=\"{}\"/><{p}axId val=\"{}\"/>", ids.0, ids.1);
    let grouping = kind.grouping();
    match kind.element() {
        "barChart" => {
            let direction = if kind.is_horizontal() { "bar" } else { "col" };
            let (gap, overlap) = if grouping == "clustered" {
                (219, -27)
            } else {
                (150, 100)
            };
            format!(
                "<{p}barChart><{p}barDir val=\"{direction}\"/><{p}grouping val=\"{grouping}\"/><{p}varyColors val=\"0\"/>{series}<{p}gapWidth val=\"{gap}\"/><{p}overlap val=\"{overlap}\"/>{axis_ids}</{p}barChart>"
            )
        }
        "lineChart" => format!(
            "<{p}lineChart><{p}grouping val=\"{grouping}\"/><{p}varyColors val=\"0\"/>{series}<{p}marker val=\"1\"/>{axis_ids}</{p}lineChart>"
        ),
        "areaChart" => format!(
            "<{p}areaChart><{p}grouping val=\"{grouping}\"/><{p}varyColors val=\"0\"/>{series}{axis_ids}</{p}areaChart>"
        ),
        "pieChart" => format!(
            "<{p}pieChart><{p}varyColors val=\"1\"/>{series}<{p}firstSliceAng val=\"0\"/></{p}pieChart>"
        ),
        "doughnutChart" => format!(
            "<{p}doughnutChart><{p}varyColors val=\"1\"/>{series}<{p}firstSliceAng val=\"0\"/><{p}holeSize val=\"75\"/></{p}doughnutChart>"
        ),
        "scatterChart" => {
            let style = if matches!(
                kind,
                ChartKind::ScatterSmooth | ChartKind::ScatterSmoothMarkers
            ) {
                "smoothMarker"
            } else {
                "lineMarker"
            };
            format!(
                "<{p}scatterChart><{p}scatterStyle val=\"{style}\"/><{p}varyColors val=\"0\"/>{series}{axis_ids}</{p}scatterChart>"
            )
        }
        "bubbleChart" => format!(
            "<{p}bubbleChart><{p}varyColors val=\"0\"/>{series}<{p}bubbleScale val=\"100\"/><{p}showNegBubbles val=\"0\"/>{axis_ids}</{p}bubbleChart>"
        ),
        _ => {
            let style = if kind == ChartKind::FilledRadar {
                "filled"
            } else {
                "marker"
            };
            format!(
                "<{p}radarChart><{p}radarStyle val=\"{style}\"/><{p}varyColors val=\"0\"/>{series}{axis_ids}</{p}radarChart>"
            )
        }
    }
}

/// What series of different types can share a chart element: the same
/// element, direction and grouping. Lines with and without markers share
/// one, each series keeping its own.
fn same_plot(a: ChartKind, b: ChartKind) -> bool {
    a.element() == b.element()
        && a.grouping() == b.grouping()
        && a.is_horizontal() == b.is_horizontal()
}

/// The chart-type elements and axes, for `series`: one element for a
/// chart of one type; for a combination, one for each type the series
/// are drawn as, on the primary or the secondary axes.
fn plot_xml(prefix: &str, kind: ChartKind, series: &[SeriesReferences]) -> String {
    let p = prefix;
    let own = |series: &SeriesReferences| -> ChartKind {
        match (kind, series.kind) {
            (ChartKind::Combo, Some(ChartKind::Combo) | None) => ChartKind::Column,
            (ChartKind::Combo, Some(kind)) => kind,
            _ => kind,
        }
    };
    // Each type's series, in the order the types first come.
    type Group<'a> = (ChartKind, bool, Vec<(usize, &'a SeriesReferences)>);
    let mut groups: Vec<Group> = Vec::new();
    let combination = kind == ChartKind::Combo;
    for (index, one) in series.iter().enumerate() {
        let mut of = own(one);
        // A pie or doughnut takes no part in a combination.
        if combination && of.is_round() {
            of = ChartKind::Column;
        }
        let secondary = combination && one.secondary;
        match groups
            .iter_mut()
            .find(|(kind, on, _)| *on == secondary && same_plot(*kind, of))
        {
            Some((_, _, members)) => members.push((index, one)),
            None => groups.push((of, secondary, vec![(index, one)])),
        }
    }
    if groups.is_empty() {
        groups.push((own_or_column(kind), false, Vec::new()));
    }
    // Something has to be on the primary axes.
    if groups.iter().all(|(_, secondary, _)| *secondary) {
        for group in &mut groups {
            group.1 = false;
        }
    }
    let primary = groups
        .iter()
        .find(|(_, secondary, _)| !secondary)
        .map(|(kind, _, _)| *kind)
        .unwrap_or(ChartKind::Column);
    let secondary = groups
        .iter()
        .find(|(_, secondary, _)| *secondary)
        .map(|(kind, _, _)| *kind);
    let mut xml: String = groups
        .iter()
        .map(|(kind, on_secondary, members)| {
            group_xml(
                p,
                *kind,
                members,
                if *on_secondary { SECONDARY } else { PRIMARY },
            )
        })
        .collect();
    xml.push_str(&axes(p, primary, PRIMARY, false));
    if let Some(kind) = secondary {
        xml.push_str(&axes(p, kind, SECONDARY, true));
    }
    xml
}

fn own_or_column(kind: ChartKind) -> ChartKind {
    if kind == ChartKind::Combo {
        ChartKind::Column
    } else {
        kind
    }
}

fn title_xml(prefix: &str, text: &str) -> String {
    let p = prefix;
    let paragraphs: String = text
        .split('\n')
        .map(|line| format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", escape(line)))
        .collect();
    format!(
        "<{p}title><{p}tx><{p}rich xmlns:a=\"{DRAWINGML_NS}\"><a:bodyPr/><a:lstStyle/>{paragraphs}</{p}rich></{p}tx><{p}overlay val=\"0\"/></{p}title>"
    )
}

fn legend_xml(prefix: &str, position: LegendPosition) -> String {
    let p = prefix;
    format!(
        "<{p}legend><{p}legendPos val=\"{}\"/><{p}overlay val=\"0\"/></{p}legend>",
        position.code()
    )
}

/// A whole chart part for a new chart.
pub fn new_chart_part(
    kind: ChartKind,
    title: Option<&str>,
    legend: Option<LegendPosition>,
    series: &[SeriesReferences],
) -> String {
    let p = "c:";
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace xmlns:c=\"{CHART_NS}\" xmlns:a=\"{DRAWINGML_NS}\" xmlns:r=\"{RELATIONSHIPS_NS}\"><c:roundedCorners val=\"0\"/><c:chart>{}<c:autoTitleDeleted val=\"{}\"/><c:plotArea><c:layout/>{}</c:plotArea>{}<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart></c:chartSpace>",
        title.map(|text| title_xml(p, text)).unwrap_or_default(),
        if title.is_some() { 0 } else { 1 },
        plot_xml(p, kind, series),
        legend.map(|position| legend_xml(p, position)).unwrap_or_default(),
    )
}

/// What a change does to a chart part.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartEdit {
    /// The chart type and series to rebuild the plot area with.
    pub plot: Option<(ChartKind, Vec<SeriesReferences>)>,
    /// A new title, or none.
    pub title: Option<Option<String>>,
    /// A legend somewhere, or none.
    pub legend: Option<Option<LegendPosition>>,
}

/// The `<c:chart>` element's children as the part has them.
struct ChartLayout {
    prefix: String,
    /// Each child's local name and span.
    children: Vec<(Vec<u8>, (usize, usize))>,
    /// Where `<c:chart>` closes.
    close: usize,
    /// The plot area's children: local name and span.
    plot_children: Vec<(Vec<u8>, (usize, usize))>,
    plot_close: usize,
}

fn chart_layout(xml: &str) -> Result<ChartLayout> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut stack: Vec<(Vec<u8>, usize)> = Vec::new();
    let mut prefix = String::new();
    let mut layout = ChartLayout {
        prefix: String::new(),
        children: Vec::new(),
        close: 0,
        plot_children: Vec::new(),
        plot_close: 0,
    };
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        let (name, closed_at) = match &event {
            Event::Start(element) => {
                let qualified = element.name().as_ref().to_vec();
                if stack.len() == 1 && local(&qualified) == b"chart" {
                    prefix = namespace_prefix(&qualified);
                }
                stack.push((local(&qualified).to_vec(), start));
                buf.clear();
                continue;
            }
            Event::Empty(element) => (local(element.name().as_ref()).to_vec(), Some((start, end))),
            Event::End(_) => {
                let (name, begin) = stack.pop().unwrap_or_default();
                if stack.len() == 1 && name == b"chart" {
                    layout.close = start;
                }
                if stack.len() == 2 && name == b"plotArea" {
                    layout.plot_close = start;
                }
                (name, Some((begin, end)))
            }
            Event::Eof => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        if let Some(span) = closed_at {
            let path: Vec<&[u8]> = stack.iter().map(|(name, _)| name.as_slice()).collect();
            if path.len() == 2 && path[1] == b"chart" {
                layout.children.push((name.clone(), span));
            }
            if path.len() == 3 && path[1] == b"chart" && path[2] == b"plotArea" {
                layout.plot_children.push((name, span));
            }
        }
        buf.clear();
    }
    if layout.close == 0 {
        return Err(WriteError::MissingPart("c:chart".into()));
    }
    layout.prefix = prefix;
    Ok(layout)
}

/// Children of `c:chart` in schema order.
const CHART_CHILDREN: &[&[u8]] = &[
    b"title",
    b"autoTitleDeleted",
    b"pivotFmts",
    b"view3D",
    b"floor",
    b"sideWall",
    b"backWall",
    b"plotArea",
    b"legend",
    b"plotVisOnly",
    b"dispBlanksAs",
    b"showDLblsOverMax",
    b"extLst",
];

/// A splice putting `text` where the chart's `name` child is, or — when it
/// has none — where the schema puts it.
fn place(layout: &ChartLayout, name: &[u8], text: String) -> Splice {
    if let Some((_, span)) = layout.children.iter().find(|(child, _)| child == name) {
        return Splice {
            start: span.0,
            end: span.1,
            text,
        };
    }
    let rank = CHART_CHILDREN
        .iter()
        .position(|child| *child == name)
        .unwrap_or(0);
    let at = layout
        .children
        .iter()
        .find(|(child, _)| {
            CHART_CHILDREN
                .iter()
                .position(|known| known == child)
                .is_some_and(|other| other > rank)
        })
        .map(|(_, span)| span.0)
        .unwrap_or(layout.close);
    Splice {
        start: at,
        end: at,
        text,
    }
}

/// The chart part with `edit` made.
pub fn edit_chart_part(xml: &str, edit: &ChartEdit) -> Result<String> {
    let layout = chart_layout(xml)?;
    let p = layout.prefix.clone();
    let mut splices = Vec::new();
    if let Some(title) = &edit.title {
        splices.push(place(
            &layout,
            b"title",
            title
                .as_deref()
                .map(|text| title_xml(&p, text))
                .unwrap_or_default(),
        ));
        splices.push(place(
            &layout,
            b"autoTitleDeleted",
            format!(
                "<{p}autoTitleDeleted val=\"{}\"/>",
                if title.is_some() { 0 } else { 1 }
            ),
        ));
    }
    if let Some(legend) = edit.legend {
        splices.push(place(
            &layout,
            b"legend",
            legend
                .map(|position| legend_xml(&p, position))
                .unwrap_or_default(),
        ));
    }
    if let Some((kind, series)) = &edit.plot {
        // The chart types and axes go; the layout, data table and fill stay.
        let kept: &[&[u8]] = &[b"layout", b"dTable", b"spPr", b"extLst"];
        let mut first_removed: Option<usize> = None;
        for (name, span) in &layout.plot_children {
            if kept.contains(&name.as_slice()) {
                continue;
            }
            first_removed.get_or_insert(span.0);
            splices.push(Splice {
                start: span.0,
                end: span.1,
                text: String::new(),
            });
        }
        // After the layout, before the data table or fill.
        let at = layout
            .plot_children
            .iter()
            .find(|(name, _)| matches!(name.as_slice(), b"dTable" | b"spPr" | b"extLst"))
            .map(|(_, span)| span.0)
            .or(first_removed)
            .unwrap_or(layout.plot_close);
        let at = first_removed.map_or(at, |first| first.min(at));
        splices.push(Splice {
            start: at,
            end: at,
            text: plot_xml(&p, *kind, series),
        });
    }
    Ok(apply(xml, splices))
}

/// Every `cNvPr/@id` in a drawing.
fn drawing_ids(xml: &str) -> Result<Vec<u32>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut ids = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"cNvPr" =>
            {
                if let Some(id) = attribute(element, b"id")?.and_then(|id| id.parse().ok()) {
                    ids.push(id);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(ids)
}

/// The sheet's drawing part, made — with its relationship, content type
/// and `<drawing>` element — when it has none.
pub(crate) fn drawing_for(package: &mut OpcPackage, sheet_part: &str) -> Result<String> {
    let rels_name = jade_opc::rels_part_name(sheet_part);
    let rels_xml = package.part_str(&rels_name)?;
    if let Some(target) = rels_xml
        .as_deref()
        .map(jade_opc::parse_relationships)
        .transpose()?
        .and_then(|rels| rels.target_by_type_suffix("/drawing").map(str::to_string))
    {
        return Ok(jade_opc::resolve_target(sheet_part, &target));
    }
    let part = free_part_name(package, "xl/drawings/drawing", ".xml");
    package.replace(
        &part,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<xdr:wsDr xmlns:xdr=\"{SPREADSHEET_DRAWING_NS}\" xmlns:a=\"{DRAWINGML_NS}\"></xdr:wsDr>"
        )
        .into_bytes(),
    );
    let (rels, id) = add_relationship(
        rels_xml.as_deref(),
        DRAWING_TYPE,
        &relative_target(sheet_part, &part),
    )?;
    package.replace(&rels_name, rels.into_bytes());
    ensure_override(package, &part, DRAWING_CONTENT_TYPE)?;
    let sheet_xml = package
        .part_str(sheet_part)?
        .ok_or_else(|| WriteError::MissingPart(sheet_part.to_string()))?;
    let sheet_xml = crate::header_footer::set_child(
        &sheet_xml,
        b"drawing",
        |prefix| format!("<{prefix}drawing xmlns:r=\"{RELATIONSHIPS_NS}\" r:id=\"{id}\"/>"),
        &[
            b"legacyDrawing",
            b"legacyDrawingHF",
            b"drawingHF",
            b"picture",
            b"oleObjects",
            b"controls",
            b"webPublishItems",
            b"tableParts",
            b"extLst",
        ],
    )?;
    package.replace(sheet_part, sheet_xml.into_bytes());
    Ok(part)
}

/// Puts `chart` (a whole chart part) on the sheet over `anchor`, as a new
/// object in its drawing; returns the object's id.
pub fn insert_chart(
    package: &mut OpcPackage,
    sheet_part: &str,
    chart: &str,
    anchor: &ShapeAnchor,
) -> Result<u32> {
    let drawing = drawing_for(package, sheet_part)?;
    let chart_part = free_part_name(package, "xl/charts/chart", ".xml");
    package.replace(&chart_part, chart.as_bytes().to_vec());
    ensure_override(package, &chart_part, CHART_CONTENT_TYPE)?;
    let drawing_rels = jade_opc::rels_part_name(&drawing);
    let (rels, relationship) = add_relationship(
        package.part_str(&drawing_rels)?.as_deref(),
        CHART_TYPE,
        &relative_target(&drawing, &chart_part),
    )?;
    package.replace(&drawing_rels, rels.into_bytes());
    append_to_drawing(package, &drawing, |p, id| {
        let marker = |name: &str, cell: jade_xlsx_reader::CellRef, offset: (i64, i64)| {
            format!(
                "<{p}{name}><{p}col>{}</{p}col><{p}colOff>{}</{p}colOff><{p}row>{}</{p}row><{p}rowOff>{}</{p}rowOff></{p}{name}>",
                cell.column, offset.0, cell.row, offset.1
            )
        };
        format!(
            "<{p}twoCellAnchor>{}{}<{p}graphicFrame macro=\"\" xmlns:a=\"{DRAWINGML_NS}\"><{p}nvGraphicFramePr><{p}cNvPr id=\"{id}\" name=\"Chart {id}\"/><{p}cNvGraphicFramePr/></{p}nvGraphicFramePr><{p}xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></{p}xfrm><a:graphic><a:graphicData uri=\"{CHART_NS}\"><c:chart xmlns:c=\"{CHART_NS}\" xmlns:r=\"{RELATIONSHIPS_NS}\" r:id=\"{relationship}\"/></a:graphicData></a:graphic></{p}graphicFrame><{p}clientData/></{p}twoCellAnchor>",
            marker("from", anchor.from, anchor.from_offset),
            marker("to", anchor.to, anchor.to_offset),
        )
    })
}

/// A new object at the end of `drawing`, written by `frame` from the
/// drawing's namespace prefix and the object's new id; returns the id.
pub(crate) fn append_to_drawing(
    package: &mut OpcPackage,
    drawing: &str,
    frame: impl FnOnce(&str, u32) -> String,
) -> Result<u32> {
    let xml = package
        .part_str(drawing)?
        .ok_or_else(|| WriteError::MissingPart(drawing.to_string()))?;
    let id = drawing_ids(&xml)?.into_iter().max().unwrap_or(0) + 1;
    // The root's prefix, and where it closes.
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut prefix = None;
    let mut close = None;
    let mut empty_root = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) => {
                if depth == 0 {
                    prefix = Some(namespace_prefix(element.name().as_ref()));
                }
                depth += 1;
            }
            Event::Empty(ref element) if depth == 0 => {
                prefix = Some(namespace_prefix(element.name().as_ref()));
                empty_root = Some((start, end, element.name().as_ref().to_vec()));
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    close = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let frame = frame(&prefix.unwrap_or_default(), id);
    let splice = match (close, empty_root) {
        (Some(at), _) => Splice {
            start: at,
            end: at,
            text: frame,
        },
        (None, Some((start, end, name))) => {
            let head = &xml[start..end];
            let opened = head.trim_end_matches("/>").trim_end().to_string() + ">";
            Splice {
                start,
                end,
                text: format!("{opened}{frame}</{}>", String::from_utf8_lossy(&name)),
            }
        }
        (None, None) => return Err(WriteError::MissingPart(drawing.to_string())),
    };
    package.replace(drawing, apply(&xml, vec![splice]).into_bytes());
    Ok(id)
}
