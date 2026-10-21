//! SmartArt from its data: the text and structure of a diagram
//! (`word/diagrams/dataN.xml`, a `dgm:dataModel`) laid out as Word's common
//! layouts lay it out — 基本ブロック リスト, 縦方向箇条書きリスト, 基本ステップ,
//! 基本の循環, 階層 and 組織図 — and written as the cached drawing
//! (`dsp:drawing`) Word keeps beside it.
//!
//! A diagram Word saved carries that drawing and is drawn from it; one with
//! data only (made by another program, or edited here) is drawn from this
//! layout, and a diagram put in or edited by jade gets this drawing written,
//! so Word shows it as jade did until it lays it out again itself.

use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};

/// One point of a diagram's text: a bullet in Word's text pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagramNode {
    pub text: String,
    pub children: Vec<DiagramNode>,
}

impl DiagramNode {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            children: Vec::new(),
        }
    }

    pub fn with(text: impl Into<String>, children: Vec<DiagramNode>) -> Self {
        Self {
            text: text.into(),
            children,
        }
    }
}

/// How a layout arranges its points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Boxes in rows: 基本ブロック リスト.
    BlockList,
    /// One band per point, its sub-points under it: 縦方向箇条書きリスト.
    VerticalList,
    /// Boxes left to right joined by arrows: 基本ステップ.
    Process,
    /// Circles round a ring with arrows between: 基本の循環.
    Cycle,
    /// A tree, top down: 階層, 組織図.
    Hierarchy,
}

/// The layouts jade lays out, as Word identifies them (`layoutDef@uniqueId`),
/// with their gallery names.
pub const LAYOUTS: [(&str, &str, &str, Family); 6] = [
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/default",
        "基本ブロック リスト",
        "Basic Block List",
        Family::BlockList,
    ),
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/vList2",
        "縦方向箇条書きリスト",
        "Vertical Bullet List",
        Family::VerticalList,
    ),
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/process1",
        "基本ステップ",
        "Basic Process",
        Family::Process,
    ),
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/cycle2",
        "基本の循環",
        "Basic Cycle",
        Family::Cycle,
    ),
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/hierarchy1",
        "階層",
        "Hierarchy",
        Family::Hierarchy,
    ),
    (
        "urn:microsoft.com/office/officeart/2005/8/layout/orgChart1",
        "組織図",
        "Organization Chart",
        Family::Hierarchy,
    ),
];

/// The family a layout id belongs to; an unknown one is laid out as a list,
/// by the name's word for its category.
pub fn family(layout_id: &str) -> Family {
    if let Some((_, _, _, family)) = LAYOUTS.iter().find(|(id, ..)| *id == layout_id) {
        return *family;
    }
    let name = layout_id
        .rsplit('/')
        .next()
        .unwrap_or(layout_id)
        .to_ascii_lowercase();
    if name.contains("cycle") || name.contains("radial") {
        Family::Cycle
    } else if name.contains("hierarchy") || name.contains("org") {
        Family::Hierarchy
    } else if name.contains("process") || name.contains("chevron") || name.contains("arrow") {
        Family::Process
    } else if name.contains("vlist") || name.contains("bullet") {
        Family::VerticalList
    } else {
        Family::BlockList
    }
}

/// A diagram's text and its layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagram {
    pub layout: String,
    pub nodes: Vec<DiagramNode>,
}

fn attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name || attribute.name.ends_with(&format!(":{name}")))
        .map(|attribute| attribute.value.as_str())
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn children<'a>(element: &'a XmlElement, name: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    element.children.iter().filter_map(move |node| match node {
        XmlNode::Element(child) if local(&child.name) == name => Some(child),
        _ => None,
    })
}

fn text_of(element: &XmlElement) -> String {
    fn walk(element: &XmlElement, out: &mut String, paragraph: &mut bool) {
        if local(&element.name) == "p" {
            if *paragraph {
                out.push('\n');
            }
            *paragraph = true;
        }
        for child in &element.children {
            match child {
                XmlNode::Element(child) if local(&child.name) == "t" => {
                    for node in &child.children {
                        if let XmlNode::Text(text) = node {
                            out.push_str(text);
                        }
                    }
                }
                XmlNode::Element(child) => walk(child, out, paragraph),
                _ => {}
            }
        }
    }
    let mut out = String::new();
    walk(element, &mut out, &mut false);
    out
}

/// Reads a `dgm:dataModel`: the document point's children, in order, down
/// the `parOf` connections. `layout_id` is the layout part's id when known.
pub fn parse_data(xml: &str, layout_id: Option<&str>) -> Option<Diagram> {
    let document = DocxXmlDocument::parse(xml).ok()?;
    let root = document.root();
    let points = children(root, "ptLst").next()?;
    let connections = children(root, "cxnLst").next();
    let mut texts = std::collections::HashMap::new();
    let mut document_id = None;
    let mut layout = layout_id.map(str::to_string);
    for point in children(points, "pt") {
        let id = attr(point, "modelId")?.to_string();
        match attr(point, "type").unwrap_or("node") {
            "doc" => {
                document_id = Some(id);
                if layout.is_none() {
                    layout = children(point, "prSet")
                        .next()
                        .and_then(|set| attr(set, "loTypeId"))
                        .map(str::to_string);
                }
            }
            "node" | "asst" => {
                let text = children(point, "t").next().map(text_of).unwrap_or_default();
                texts.insert(id, text);
            }
            _ => {}
        }
    }
    let document_id = document_id?;
    let mut edges: Vec<(String, String, usize)> = connections
        .map(|connections| {
            children(connections, "cxn")
                .filter(|cxn| matches!(attr(cxn, "type"), None | Some("parOf")))
                .filter_map(|cxn| {
                    Some((
                        attr(cxn, "srcId")?.to_string(),
                        attr(cxn, "destId")?.to_string(),
                        attr(cxn, "srcOrd")
                            .and_then(|o| o.parse().ok())
                            .unwrap_or(0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    edges.sort_by_key(|(_, _, order)| *order);
    fn build(
        id: &str,
        edges: &[(String, String, usize)],
        texts: &std::collections::HashMap<String, String>,
        depth: usize,
    ) -> Vec<DiagramNode> {
        if depth > 16 {
            return Vec::new();
        }
        edges
            .iter()
            .filter(|(source, destination, _)| source == id && texts.contains_key(destination))
            .map(|(_, destination, _)| DiagramNode {
                text: texts[destination].clone(),
                children: build(destination, edges, texts, depth + 1),
            })
            .collect()
    }
    Some(Diagram {
        layout: layout.unwrap_or_else(|| LAYOUTS[0].0.to_string()),
        nodes: build(&document_id, &edges, &texts, 0),
    })
}

/// A shape of the drawing, in EMU.
struct Shape {
    preset: &'static str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rotation: f64,
    fill: Option<&'static str>,
    /// `shade` or `tint` on the fill's accent, thousandths of a percent.
    fill_tint: Option<i64>,
    line: Option<&'static str>,
    text: Vec<(String, usize)>,
    text_color: &'static str,
    bullets: bool,
}

impl Shape {
    fn boxed(
        preset: &'static str,
        (x, y, width, height): (f64, f64, f64, f64),
        node: &DiagramNode,
        with_children: bool,
    ) -> Self {
        let mut text = vec![(node.text.clone(), 0)];
        if with_children {
            for child in &node.children {
                text.push((child.text.clone(), 1));
            }
        }
        Shape {
            preset,
            x,
            y,
            width,
            height,
            rotation: 0.0,
            fill: Some("accent1"),
            fill_tint: None,
            line: Some("lt1"),
            text,
            text_color: "lt1",
            bullets: false,
        }
    }

    fn arrow(
        preset: &'static str,
        (x, y, width, height): (f64, f64, f64, f64),
        rotation: f64,
    ) -> Self {
        Shape {
            preset,
            x,
            y,
            width,
            height,
            rotation,
            fill: Some("accent1"),
            fill_tint: Some(60_000),
            line: None,
            text: Vec::new(),
            text_color: "lt1",
            bullets: false,
        }
    }

    fn line((x, y, width, height): (f64, f64, f64, f64)) -> Self {
        Shape {
            preset: "rect",
            x,
            y,
            width: width.max(12_700.0),
            height: height.max(12_700.0),
            rotation: 0.0,
            fill: Some("accent1"),
            fill_tint: Some(-60_000),
            line: None,
            text: Vec::new(),
            text_color: "lt1",
            bullets: false,
        }
    }
}

/// Lays `diagram` out in a `width` × `height` EMU frame.
fn layout(diagram: &Diagram, width: f64, height: f64) -> Vec<Shape> {
    let nodes = &diagram.nodes;
    let count = nodes.len().max(1);
    let mut shapes = Vec::new();
    match family(&diagram.layout) {
        Family::BlockList => {
            // As many columns as keep the boxes largest at Word's 3:2.
            let mut best = (1, 0.0);
            for columns in 1..=count {
                let rows = count.div_ceil(columns);
                let box_width = (width / (columns as f64 + (columns - 1) as f64 * 0.1))
                    .min(height / (rows as f64 + (rows - 1) as f64 * 0.1) * 1.5);
                if box_width > best.1 {
                    best = (columns, box_width);
                }
            }
            let (columns, box_width) = best;
            let box_height = box_width / 1.5;
            let rows = count.div_ceil(columns);
            let used_width = box_width * (columns as f64 + (columns - 1) as f64 * 0.1);
            let used_height = box_height * (rows as f64 + (rows - 1) as f64 * 0.1);
            let (left, top) = ((width - used_width) / 2.0, (height - used_height) / 2.0);
            for (index, node) in nodes.iter().enumerate() {
                let (row, column) = (index / columns, index % columns);
                // The last row is centred when it is short.
                let in_row = if row == rows - 1 {
                    count - row * columns
                } else {
                    columns
                };
                let row_left = left
                    + (used_width - box_width * (in_row as f64 + (in_row - 1) as f64 * 0.1)) / 2.0;
                shapes.push(Shape::boxed(
                    "rect",
                    (
                        row_left + column as f64 * box_width * 1.1,
                        top + row as f64 * box_height * 1.1,
                        box_width,
                        box_height,
                    ),
                    node,
                    true,
                ));
            }
        }
        Family::VerticalList => {
            // A band per point, and a lighter box of its sub-points.
            let units: f64 = nodes
                .iter()
                .map(|node| {
                    1.0 + if node.children.is_empty() {
                        0.0
                    } else {
                        0.25 + 0.5 * node.children.len() as f64
                    }
                })
                .sum::<f64>()
                + 0.1 * (count - 1) as f64;
            let unit = (height / units.max(1.0)).min(width / 3.0);
            let mut y = (height - unit * units).max(0.0) / 2.0;
            for node in nodes {
                shapes.push(Shape::boxed(
                    "roundRect",
                    (0.0, y, width, unit),
                    node,
                    false,
                ));
                y += unit;
                if !node.children.is_empty() {
                    let box_height = unit * (0.25 + 0.5 * node.children.len() as f64);
                    shapes.push(Shape {
                        preset: "rect",
                        x: 0.0,
                        y,
                        width,
                        height: box_height,
                        rotation: 0.0,
                        fill: Some("accent1"),
                        fill_tint: Some(40_000),
                        line: None,
                        text: node
                            .children
                            .iter()
                            .map(|child| (child.text.clone(), 1))
                            .collect(),
                        text_color: "tx1",
                        bullets: true,
                    });
                    y += box_height;
                }
                y += unit * 0.1;
            }
        }
        Family::Process => {
            let arrow_share = 0.4;
            let box_width = width / (count as f64 + (count - 1) as f64 * arrow_share);
            let box_height = (box_width * 0.6).min(height);
            let top = (height - box_height) / 2.0;
            for (index, node) in nodes.iter().enumerate() {
                let x = index as f64 * box_width * (1.0 + arrow_share);
                shapes.push(Shape::boxed(
                    "roundRect",
                    (x, top, box_width, box_height),
                    node,
                    true,
                ));
                if index + 1 < count {
                    let arrow_width = box_width * arrow_share * 0.6;
                    let arrow_height = box_height * 0.3;
                    shapes.push(Shape::arrow(
                        "rightArrow",
                        (
                            x + box_width + box_width * arrow_share * 0.2,
                            top + (box_height - arrow_height) / 2.0,
                            arrow_width,
                            arrow_height,
                        ),
                        0.0,
                    ));
                }
            }
        }
        Family::Cycle => {
            let size = width.min(height);
            let (center_x, center_y) = (width / 2.0, height / 2.0);
            let node_size = size
                / (2.0 + 2.0 * (std::f64::consts::PI / count as f64).sin().recip().min(3.0))
                * 1.6;
            let node_size = node_size.min(size / 2.5);
            let radius = (size - node_size) / 2.0;
            for (index, node) in nodes.iter().enumerate() {
                let angle = -std::f64::consts::FRAC_PI_2
                    + std::f64::consts::TAU * index as f64 / count as f64;
                let (x, y) = (
                    center_x + radius * angle.cos(),
                    center_y + radius * angle.sin(),
                );
                shapes.push(Shape::boxed(
                    "ellipse",
                    (
                        x - node_size / 2.0,
                        y - node_size / 2.0,
                        node_size,
                        node_size,
                    ),
                    node,
                    true,
                ));
                if count > 1 {
                    // An arrow on the ring halfway to the next, along it.
                    let between = angle + std::f64::consts::PI / count as f64;
                    let (ax, ay) = (
                        center_x + radius * between.cos(),
                        center_y + radius * between.sin(),
                    );
                    let (arrow_width, arrow_height) = (node_size * 0.3, node_size * 0.25);
                    shapes.push(Shape::arrow(
                        "rightArrow",
                        (
                            ax - arrow_width / 2.0,
                            ay - arrow_height / 2.0,
                            arrow_width,
                            arrow_height,
                        ),
                        (between.to_degrees() + 90.0).rem_euclid(360.0),
                    ));
                }
            }
        }
        Family::Hierarchy => {
            // Leaves get equal columns; a parent sits over its leaves.
            fn leaves(node: &DiagramNode) -> usize {
                node.children.iter().map(leaves).sum::<usize>().max(1)
            }
            fn depth(node: &DiagramNode) -> usize {
                1 + node.children.iter().map(depth).max().unwrap_or(0)
            }
            let total_leaves: usize = nodes.iter().map(leaves).sum::<usize>().max(1);
            let levels = nodes.iter().map(depth).max().unwrap_or(1);
            let column = width / total_leaves as f64;
            let level_height = height / (levels as f64 + (levels - 1) as f64 * 0.5);
            let box_width = (column * 0.85).min(level_height * 2.2);
            let box_height = (box_width / 1.6).min(level_height);
            #[allow(clippy::too_many_arguments)]
            fn place(
                node: &DiagramNode,
                first_column: usize,
                level: usize,
                column: f64,
                level_height: f64,
                (box_width, box_height): (f64, f64),
                shapes: &mut Vec<Shape>,
            ) -> (f64, f64) {
                let span = leaves(node);
                let center_x = column * (first_column as f64 + span as f64 / 2.0);
                let top = level as f64 * level_height * 1.5;
                shapes.push(Shape::boxed(
                    "roundRect",
                    (center_x - box_width / 2.0, top, box_width, box_height),
                    node,
                    false,
                ));
                let mut next = first_column;
                let mut child_centers = Vec::new();
                for child in &node.children {
                    let (child_x, child_top) = place(
                        child,
                        next,
                        level + 1,
                        column,
                        level_height,
                        (box_width, box_height),
                        shapes,
                    );
                    child_centers.push((child_x, child_top));
                    next += leaves(child);
                }
                if let (Some(first), Some(last)) = (child_centers.first(), child_centers.last()) {
                    // Elbow lines: down from the parent, across, down to each.
                    let bus = top + box_height + (first.1 - top - box_height) / 2.0;
                    shapes.push(Shape::line((
                        center_x,
                        top + box_height,
                        0.0,
                        bus - top - box_height,
                    )));
                    shapes.push(Shape::line((first.0, bus, last.0 - first.0, 0.0)));
                    for (child_x, child_top) in &child_centers {
                        shapes.push(Shape::line((*child_x, bus, 0.0, child_top - bus)));
                    }
                }
                (center_x, top)
            }
            let mut next = 0;
            for node in nodes {
                place(
                    node,
                    next,
                    0,
                    column,
                    level_height,
                    (box_width, box_height),
                    &mut shapes,
                );
                next += leaves(node);
            }
        }
    }
    shapes
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A font size (hundredths of a point) the text fits a box with.
fn font_size(shape: &Shape) -> i64 {
    let lines: Vec<&(String, usize)> = shape
        .text
        .iter()
        .filter(|(text, _)| !text.is_empty())
        .collect();
    let line_count = lines.len().max(1) as f64;
    let longest = lines
        .iter()
        .map(|(text, _)| {
            text.chars()
                .map(|ch| if ch.is_ascii() { 0.55 } else { 1.0 })
                .sum::<f64>()
        })
        .fold(1.0, f64::max);
    let by_height = shape.height * 0.75 / line_count / 1.2;
    let by_width = shape.width * 0.85 / longest;
    let emu = by_height.min(by_width);
    let points = (emu / 12_700.0).clamp(7.0, 65.0);
    (points * 100.0).round() as i64
}

/// The cached drawing Word keeps for a diagram (`dsp:drawing`), laid out in
/// a `width` × `height` EMU frame.
pub fn drawing_xml(diagram: &Diagram, width: f64, height: f64) -> String {
    let mut shapes = String::new();
    for (index, shape) in layout(diagram, width, height).iter().enumerate() {
        let fill = match (shape.fill, shape.fill_tint) {
            (Some(slot), Some(tint)) if tint > 0 => format!(
                r#"<a:solidFill><a:schemeClr val="{slot}"><a:tint val="{tint}"/></a:schemeClr></a:solidFill>"#
            ),
            (Some(slot), Some(shade)) => format!(
                r#"<a:solidFill><a:schemeClr val="{slot}"><a:shade val="{}"/></a:schemeClr></a:solidFill>"#,
                -shade
            ),
            (Some(slot), None) => {
                format!(r#"<a:solidFill><a:schemeClr val="{slot}"/></a:solidFill>"#)
            }
            (None, _) => "<a:noFill/>".to_string(),
        };
        let line = match shape.line {
            Some(slot) => format!(
                r#"<a:ln w="12700" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="{slot}"/></a:solidFill><a:prstDash val="solid"/><a:miter lim="800000"/></a:ln>"#
            ),
            None => "<a:ln><a:noFill/></a:ln>".to_string(),
        };
        let rotation = if shape.rotation.abs() > 0.01 {
            format!(r#" rot="{}""#, (shape.rotation * 60_000.0).round() as i64)
        } else {
            String::new()
        };
        let text = if shape.text.is_empty() {
            String::new()
        } else {
            let size = font_size(shape);
            let paragraphs: String = shape
                .text
                .iter()
                .map(|(text, level)| {
                    let (align, bullet) = if shape.bullets || *level > 0 {
                        ("l", r#"<a:buChar char="•"/>"#)
                    } else {
                        ("ctr", "<a:buNone/>")
                    };
                    let size = if *level > 0 { size * 3 / 4 } else { size };
                    format!(
                        r#"<a:p><a:pPr marL="{}" lvl="{level}" indent="{}" algn="{align}">{bullet}</a:pPr><a:r><a:rPr lang="ja-JP" altLang="en-US" sz="{size}" kern="1200"><a:solidFill><a:schemeClr val="{}"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p>"#,
                        if *level > 0 { 228_600 } else { 0 },
                        if *level > 0 { -228_600 } else { 0 },
                        shape.text_color,
                        escape(text)
                    )
                })
                .collect();
            format!(
                r#"<dsp:txBody><a:bodyPr spcFirstLastPara="0" vert="horz" wrap="square" lIns="76200" tIns="76200" rIns="76200" bIns="76200" numCol="1" spcCol="1270" anchor="ctr" anchorCtr="0"><a:noAutofit/></a:bodyPr><a:lstStyle/>{paragraphs}</dsp:txBody><dsp:txXfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></dsp:txXfrm>"#,
                x = shape.x.round() as i64,
                y = shape.y.round() as i64,
                w = shape.width.round() as i64,
                h = shape.height.round() as i64,
            )
        };
        shapes.push_str(&format!(
            r#"<dsp:sp modelId="{{{index:08X}-0000-4000-8000-00000000D5A0}}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm{rotation}><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm><a:prstGeom prst="{preset}"><a:avLst/></a:prstGeom>{fill}{line}</dsp:spPr><dsp:style><a:lnRef idx="2"><a:scrgbClr r="0" g="0" b="0"/></a:lnRef><a:fillRef idx="1"><a:scrgbClr r="0" g="0" b="0"/></a:fillRef><a:effectRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="{text_color}"/></a:fontRef></dsp:style>{text}</dsp:sp>"#,
            x = shape.x.round() as i64,
            y = shape.y.round() as i64,
            w = shape.width.max(1.0).round() as i64,
            h = shape.height.max(1.0).round() as i64,
            preset = shape.preset,
            text_color = shape.text_color,
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dsp:drawing xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>{shapes}</dsp:spTree></dsp:drawing>"#
    )
}

/// A diagram's data model (`dgm:dataModel`): a document point, a point per
/// bullet with its parent connection, and the extension naming the cached
/// drawing's relationship.
pub fn data_xml(diagram: &Diagram, drawing_relationship: Option<&str>) -> String {
    let mut points = format!(
        r#"<dgm:pt modelId="{{00000000-0000-4000-8000-000000000000}}" type="doc"><dgm:prSet loTypeId="{}" loCatId="list" qsTypeId="urn:microsoft.com/office/officeart/2005/8/quickstyle/simple1" qsCatId="simple" csTypeId="urn:microsoft.com/office/officeart/2005/8/colors/accent1_2" csCatId="accent1" phldr="1"/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="ja-JP" altLang="en-US"/></a:p></dgm:t></dgm:pt>"#,
        escape(&diagram.layout)
    );
    let mut connections = String::new();
    let mut counter = 0u32;
    fn walk(
        nodes: &[DiagramNode],
        parent: &str,
        counter: &mut u32,
        points: &mut String,
        connections: &mut String,
    ) {
        for (order, node) in nodes.iter().enumerate() {
            *counter += 1;
            let id = format!("{{{:08X}-0000-4000-8000-000000000001}}", *counter);
            let parent_transition = format!("{{{:08X}-0000-4000-8000-000000000002}}", *counter);
            let sibling_transition = format!("{{{:08X}-0000-4000-8000-000000000003}}", *counter);
            let connection = format!("{{{:08X}-0000-4000-8000-000000000004}}", *counter);
            let text = if node.text.is_empty() {
                r#"<a:p><a:endParaRPr lang="ja-JP" altLang="en-US"/></a:p>"#.to_string()
            } else {
                format!(
                    r#"<a:p><a:r><a:rPr lang="ja-JP" altLang="en-US"/><a:t>{}</a:t></a:r></a:p>"#,
                    escape(&node.text)
                )
            };
            points.push_str(&format!(
                r#"<dgm:pt modelId="{id}"><dgm:prSet phldrT="[テキスト]"/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/>{text}</dgm:t></dgm:pt><dgm:pt modelId="{parent_transition}" type="parTrans" cxnId="{connection}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="ja-JP" altLang="en-US"/></a:p></dgm:t></dgm:pt><dgm:pt modelId="{sibling_transition}" type="sibTrans" cxnId="{connection}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="ja-JP" altLang="en-US"/></a:p></dgm:t></dgm:pt>"#
            ));
            connections.push_str(&format!(
                r#"<dgm:cxn modelId="{connection}" srcId="{parent}" destId="{id}" srcOrd="{order}" destOrd="0" parTransId="{parent_transition}" sibTransId="{sibling_transition}"/>"#
            ));
            walk(&node.children, &id, counter, points, connections);
        }
    }
    walk(
        &diagram.nodes,
        "{00000000-0000-4000-8000-000000000000}",
        &mut counter,
        &mut points,
        &mut connections,
    );
    let extension = drawing_relationship
        .map(|id| {
            format!(
                r#"<dgm:extLst><a:ext uri="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:dataModelExt xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" relId="{id}" minVer="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:ext></dgm:extLst>"#
            )
        })
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dgm:ptLst>{points}</dgm:ptLst><dgm:cxnLst>{connections}</dgm:cxnLst><dgm:bg/><dgm:whole/>{extension}</dgm:dataModel>"#
    )
}

/// The layout part: the layout's id, which Word lays the data out with.
pub fn layout_xml(layout_id: &str) -> String {
    let (name_ja, _, _) = LAYOUTS
        .iter()
        .find(|(id, ..)| *id == layout_id)
        .map(|(_, ja, en, family)| (*ja, *en, *family))
        .unwrap_or(("", "", Family::BlockList));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dgm:layoutDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" uniqueId="{}"><dgm:title val="{}"/><dgm:desc val=""/><dgm:catLst><dgm:cat type="list" pri="400"/></dgm:catLst><dgm:layoutNode name="diagram"><dgm:alg type="snake"/><dgm:shape xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:blip=""><dgm:adjLst/></dgm:shape><dgm:presOf/><dgm:forEach name="Name0" axis="ch" ptType="node"><dgm:layoutNode name="node"><dgm:shape type="rect" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:blip=""><dgm:adjLst/></dgm:shape><dgm:presOf axis="desOrSelf" ptType="node"/></dgm:layoutNode></dgm:forEach></dgm:layoutNode></dgm:layoutDef>"#,
        escape(layout_id),
        escape(name_ja)
    )
}

/// The quick style part (シンプル).
pub fn quick_style_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dgm:styleDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" uniqueId="urn:microsoft.com/office/officeart/2005/8/quickstyle/simple1"><dgm:title val=""/><dgm:desc val=""/><dgm:catLst><dgm:cat type="simple" pri="10100"/></dgm:catLst><dgm:scene3d><a:camera prst="orthographicFront"/><a:lightRig rig="threePt" dir="t"/></dgm:scene3d><dgm:styleLbl name="node0"><dgm:scene3d><a:camera prst="orthographicFront"/><a:lightRig rig="threePt" dir="t"/></dgm:scene3d><dgm:sp3d/><dgm:txPr/><dgm:style><a:lnRef idx="2"><a:scrgbClr r="0" g="0" b="0"/></a:lnRef><a:fillRef idx="1"><a:scrgbClr r="0" g="0" b="0"/></a:fillRef><a:effectRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></dgm:style></dgm:styleLbl></dgm:styleDef>"#
}

/// The colours part (カラフル - アクセント 1).
pub fn colors_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dgm:colorsDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" uniqueId="urn:microsoft.com/office/officeart/2005/8/colors/accent1_2"><dgm:title val=""/><dgm:desc val=""/><dgm:catLst><dgm:cat type="accent1" pri="11200"/></dgm:catLst><dgm:styleLbl name="node0"><dgm:fillClrLst meth="repeat"><a:schemeClr val="accent1"/></dgm:fillClrLst><dgm:linClrLst meth="repeat"><a:schemeClr val="lt1"/></dgm:linClrLst><dgm:effectClrLst/><dgm:txLinClrLst/><dgm:txFillClrLst meth="repeat"><a:schemeClr val="lt1"/></dgm:txFillClrLst><dgm:txEffectClrLst/></dgm:styleLbl></dgm:colorsDef>"#
}

/// The text pane's lines for `nodes`: each point's text, indented a tab per
/// level, as Word's text pane shows them.
pub fn outline(nodes: &[DiagramNode]) -> Vec<(usize, String)> {
    fn walk(nodes: &[DiagramNode], level: usize, out: &mut Vec<(usize, String)>) {
        for node in nodes {
            out.push((level, node.text.clone()));
            walk(&node.children, level + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(nodes, 0, &mut out);
    out
}

/// The points the text pane's lines make: a line's level puts it under the
/// last line one level up (a jump of more than one level is one level).
pub fn from_outline(lines: &[(usize, String)]) -> Vec<DiagramNode> {
    fn insert(nodes: &mut Vec<DiagramNode>, level: usize, text: String) {
        if level == 0 || nodes.is_empty() {
            nodes.push(DiagramNode::new(text));
        } else {
            let last = nodes.last_mut().expect("not empty");
            insert(&mut last.children, level - 1, text);
        }
    }
    let mut nodes = Vec::new();
    for (level, text) in lines {
        insert(&mut nodes, *level, text.clone());
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(layout: usize) -> Diagram {
        Diagram {
            layout: LAYOUTS[layout].0.to_string(),
            nodes: vec![
                DiagramNode::with(
                    "社長",
                    vec![DiagramNode::new("営業"), DiagramNode::new("開発")],
                ),
                DiagramNode::new("監査"),
            ],
        }
    }

    #[test]
    fn data_round_trips_through_its_xml() {
        let diagram = sample(4);
        let xml = data_xml(&diagram, Some("rId9"));
        assert!(xml.contains(r#"relId="rId9""#));
        let read = parse_data(&xml, None).unwrap();
        assert_eq!(read, diagram);
    }

    #[test]
    fn every_layout_draws_a_shape_per_point_with_its_text() {
        for (index, (id, ..)) in LAYOUTS.iter().enumerate() {
            let diagram = sample(index);
            let shapes = layout(&diagram, 5_486_400.0, 3_200_400.0);
            let texts: Vec<_> = shapes
                .iter()
                .flat_map(|shape| shape.text.iter().map(|(text, _)| text.as_str()))
                .collect();
            for word in ["社長", "営業", "開発", "監査"] {
                assert!(texts.contains(&word), "{id} lost {word}: {texts:?}");
            }
            for shape in &shapes {
                assert!(shape.x >= -1.0 && shape.y >= -1.0, "{id}: {}", shape.x);
                assert!(shape.x + shape.width <= 5_486_401.0, "{id} overflows");
                assert!(shape.y + shape.height <= 3_200_401.0, "{id} overflows down");
            }
            let drawing = drawing_xml(&diagram, 5_486_400.0, 3_200_400.0);
            assert!(DocxXmlDocument::parse(&drawing).is_ok(), "{drawing}");
        }
    }

    #[test]
    fn the_text_pane_outline_round_trips() {
        let nodes = sample(0).nodes;
        let lines = outline(&nodes);
        assert_eq!(lines[1], (1, "営業".to_string()));
        assert_eq!(from_outline(&lines), nodes);
        // A deeper jump is one level down.
        let nodes = from_outline(&[(0, "a".into()), (3, "b".into())]);
        assert_eq!(nodes[0].children[0].text, "b");
    }
}
