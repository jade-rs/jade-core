//! Classic DrawingML charts. Sparse caches retain their point indices and
//! formulas so layout can prefer current workbook cells over saved values.
use crate::error::Result;
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
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chart {
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
}
impl Default for Chart {
    fn default() -> Self {
        Self {
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
    let plot = root.child("plotArea");
    let plots: Vec<&Node> = plot
        .into_iter()
        .flat_map(|n| n.children.iter())
        .filter(|n| n.main && n.name.ends_with("Chart"))
        .collect();
    if plots.len() != 1 {
        chart
            .unsupported_properties
            .push("combined or missing plot".into());
    }
    let Some(kind) = plots.first() else {
        return Ok(chart);
    };
    chart.kind = kind.name.clone();
    if !matches!(
        chart.kind.as_str(),
        "barChart" | "lineChart" | "areaChart" | "pieChart" | "doughnutChart" | "scatterChart"
    ) {
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
    chart.horizontal = kind.val("barDir") == Some("bar");
    chart.vary_colors = kind
        .val("varyColors")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(matches!(chart.kind.as_str(), "pieChart" | "doughnutChart"));
    chart.grouping = kind
        .val("grouping")
        .unwrap_or(if chart.kind == "barChart" {
            "clustered"
        } else {
            "standard"
        })
        .into();
    chart.gap_width = kind
        .val("gapWidth")
        .and_then(|v| v.parse().ok())
        .unwrap_or(150)
        .min(500);
    chart.overlap = kind
        .val("overlap")
        .and_then(|v| v.parse().ok())
        .unwrap_or(
            if chart.grouping.ends_with("Stacked") || chart.grouping == "stacked" {
                100
            } else {
                0
            },
        )
        .clamp(-100, 100);
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
    chart.scatter_style = kind.val("scatterStyle").unwrap_or("marker").into();
    let axes: Vec<&Node> = plot
        .into_iter()
        .flat_map(|n| n.children.iter())
        .filter(|n| n.main && matches!(n.name.as_str(), "catAx" | "valAx" | "dateAx"))
        .collect();
    if axes.len() > 2 {
        chart.unsupported_properties.push("secondary axes".into());
    }
    chart.value_axis = axis(
        axes.iter()
            .find(|n| n.name == "valAx" && n.val("axPos") != Some("b"))
            .copied()
            .or_else(|| axes.iter().find(|n| n.name == "valAx").copied()),
    );
    chart.category_axis = axis(
        axes.iter()
            .find(|n| {
                n.name == "catAx" || (chart.kind == "scatterChart" && n.val("axPos") == Some("b"))
            })
            .copied(),
    );
    for axis in &axes {
        if axis.name == "dateAx" {
            chart.unsupported_properties.push("date axis".into())
        }
        if axis
            .child("scaling")
            .and_then(|n| n.child("logBase"))
            .is_some()
        {
            chart.unsupported_properties.push("logarithmic axis".into())
        }
    }
    for ser in kind.children.iter().filter(|n| n.main && n.name == "ser") {
        let issues = &mut chart.unsupported_properties;
        let mut series = ChartSeries {
            order: ser
                .val("order")
                .and_then(|v| v.parse().ok())
                .unwrap_or(chart.series.len() as u32),
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
                    if chart.kind == "scatterChart"
                        && (chart.scatter_style.contains("Marker")
                            || chart.scatter_style == "marker")
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
        };
        for point in ser.children.iter().filter(|n| n.main && n.name == "dPt") {
            if let Some(index) = point.val("idx").and_then(|v| v.parse::<usize>().ok()) {
                series
                    .point_colors
                    .insert(index, color(point.child("spPr"), issues));
            }
        }
        chart.series.push(series);
    }
    chart.series.sort_by_key(|s| s.order);
    if matches!(chart.kind.as_str(), "pieChart" | "doughnutChart") && chart.series.len() > 1 {
        chart
            .unsupported_properties
            .push("multiple circular series".into());
    }
    for series in &chart.series {
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
            chart
                .unsupported_properties
                .push(format!("marker: {}", series.marker));
        }
    }
    for (name, label) in [
        ("dLbls", "data labels"),
        ("trendline", "trendlines"),
        ("errBars", "error bars"),
        ("manualLayout", "manual chart layout"),
        ("pivotSource", "pivot chart"),
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
    let mut smooth = Vec::new();
    root.descendants("smooth", &mut smooth);
    if smooth
        .iter()
        .any(|n| n.attrs.get("val").is_none_or(|v| v != "0" && v != "false"))
    {
        chart.unsupported_properties.push("smoothed lines".into());
    }
    if chart.scatter_style.starts_with("smooth") {
        chart.unsupported_properties.push("smoothed scatter".into());
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
    for series in &mut chart.series {
        apply(&mut series.color);
        for color in series.point_colors.values_mut() {
            apply(color)
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
