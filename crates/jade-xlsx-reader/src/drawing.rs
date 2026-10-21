//! `xl/drawings/drawingN.xml` — the shapes, text boxes, pictures and charts
//! anchored on a sheet, by the cells they are anchored to and the text they
//! carry.
//!
//! Anchors and picture transforms feed the shared layout. Shape text is
//! available to headless callers; writers patch the drawing without replacing
//! unrelated geometry, media or formatting.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;
use crate::model::{CellRef, DrawingColor, Picture, Shape, ShapeKind, ShapePaint, ShapeStyle};
use jade_opc::local_name;

/// Parses a drawing part into its anchored objects, in document order.
pub fn parse_drawing(xml: &str) -> Result<Vec<Shape>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut shapes = Vec::new();
    let mut buf = Vec::new();

    // The anchor being walked.
    let mut anchor: Option<Anchor> = None;
    // Inside `from` / `to`: which marker, and the number being read.
    let mut marker: Option<Marker> = None;
    let mut marker_axis: Option<MarkerAxis> = None;
    let mut number = String::new();
    // Nesting inside an anchor: only the outermost object is reported (a
    // group's members are its own affair), so `sp` at depth 1 is the object
    // and `sp` at depth 2 is a member.
    let mut object_depth = 0usize;
    let mut in_text_body = false;
    let mut in_text = false;
    let mut paragraph: Option<String> = None;
    let mut paragraphs: Vec<String> = Vec::new();
    let mut styles = shape_styles(xml)?.into_iter();

    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                        anchor = Some(Anchor {
                            style: styles.next().flatten(),
                            ..Anchor::default()
                        });
                        object_depth = 0;
                        paragraphs.clear();
                    }
                    b"from" | b"to" if anchor.is_some() && !empty => {
                        marker = Some(if name == b"from" {
                            Marker::From
                        } else {
                            Marker::To
                        });
                    }
                    b"col" | b"row" | b"colOff" | b"rowOff" if marker.is_some() && !empty => {
                        marker_axis = Some(match name {
                            b"col" => MarkerAxis::Column,
                            b"row" => MarkerAxis::Row,
                            b"colOff" => MarkerAxis::ColumnOffset,
                            _ => MarkerAxis::RowOffset,
                        });
                        number.clear();
                    }
                    // A one-cell anchor's size; `a:ext` inside a shape's
                    // transform is at object depth 1 and not this.
                    b"ext" if anchor.is_some() && object_depth == 0 => {
                        let cx =
                            attribute_value(element, b"cx")?.and_then(|value| value.parse().ok());
                        let cy =
                            attribute_value(element, b"cy")?.and_then(|value| value.parse().ok());
                        if let (Some(cx), Some(cy), Some(anchor)) = (cx, cy, anchor.as_mut()) {
                            anchor.extent = Some((cx, cy));
                        }
                    }
                    b"pos" if anchor.is_some() && object_depth == 0 => {
                        let x =
                            attribute_value(element, b"x")?.and_then(|value| value.parse().ok());
                        let y =
                            attribute_value(element, b"y")?.and_then(|value| value.parse().ok());
                        if let (Some(x), Some(y), Some(anchor)) = (x, y, anchor.as_mut()) {
                            anchor.from = Some(CellRef::new(0, 0));
                            anchor.from_offset = (x, y);
                        }
                    }
                    b"sp" | b"pic" | b"graphicFrame" | b"grpSp" | b"cxnSp" if anchor.is_some() => {
                        object_depth += 1;
                        if object_depth == 1 {
                            if let Some(anchor) = anchor.as_mut() {
                                anchor.kind = Some(match name {
                                    b"sp" => ShapeKind::Shape,
                                    b"pic" => ShapeKind::Picture,
                                    b"graphicFrame" => ShapeKind::GraphicFrame,
                                    b"grpSp" => ShapeKind::Group,
                                    _ => ShapeKind::Connector,
                                });
                            }
                            if name == b"pic" {
                                anchor.as_mut().unwrap().picture = Some(Picture::default());
                            }
                        }
                        if empty {
                            object_depth -= 1;
                        }
                    }
                    b"cNvPr" if object_depth == 1 => {
                        if let Some(anchor) = anchor.as_mut() {
                            if anchor.id.is_none() {
                                anchor.id = attribute_value(element, b"id")?
                                    .and_then(|value| value.parse().ok());
                                anchor.name = attribute_value(element, b"name")?;
                            }
                        }
                    }
                    b"cNvSpPr" if object_depth == 1 => {
                        let text_box = attribute_value(element, b"txBox")?
                            .is_some_and(|value| value != "0" && value != "false");
                        if text_box {
                            if let Some(anchor) = anchor.as_mut() {
                                anchor.kind = Some(ShapeKind::TextBox);
                            }
                        }
                    }
                    b"chart" if object_depth == 1 => {
                        if let Some(anchor) = anchor.as_mut() {
                            if anchor.kind == Some(ShapeKind::GraphicFrame) {
                                anchor.kind = Some(ShapeKind::Chart);
                                anchor.chart = Some(crate::chart::Chart {
                                    relationship_id: attribute_value(element, b"id")?,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    b"blip" if object_depth == 1 => {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            picture.relationship_id = attribute_value(element, b"embed")?
                                .or(attribute_value(element, b"link")?);
                        }
                    }
                    b"srcRect" | b"fillRect" if object_depth == 1 => {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            let values = if name == b"srcRect" {
                                &mut picture.crop
                            } else {
                                &mut picture.fill
                            };
                            for (slot, key) in values.iter_mut().zip([b"l", b"t", b"r", b"b"]) {
                                *slot = attribute_value(element, key)?
                                    .and_then(|value| value.parse::<i32>().ok())
                                    .unwrap_or(0);
                            }
                        }
                    }
                    b"tile" | b"outerShdw" | b"innerShdw" | b"glow" | b"softEdge"
                    | b"reflection" | b"fillOverlay" | b"duotone" | b"grayscl" | b"lum"
                    | b"tint" | b"blur" | b"clrChange" | b"clrRepl" | b"biLevel"
                    | b"alphaBiLevel" | b"alphaCeiling" | b"alphaFloor" | b"alphaInv"
                    | b"alphaMod" | b"alphaRepl" | b"custGeom"
                        if object_depth == 1 =>
                    {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            picture
                                .unsupported_properties
                                .push(String::from_utf8_lossy(name).into_owned());
                        }
                    }
                    b"prstGeom" if object_depth == 1 => {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            if let Some(geometry) = attribute_value(element, b"prst")? {
                                if geometry != "rect" {
                                    picture
                                        .unsupported_properties
                                        .push(format!("geometry: {geometry}"));
                                }
                            }
                        }
                    }
                    b"xfrm" if object_depth == 1 => {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            picture.rotation = attribute_value(element, b"rot")?
                                .and_then(|value| value.parse::<i32>().ok())
                                .unwrap_or(0);
                            picture.flip_h = attribute_value(element, b"flipH")?
                                .is_some_and(|value| value == "1" || value == "true");
                            picture.flip_v = attribute_value(element, b"flipV")?
                                .is_some_and(|value| value == "1" || value == "true");
                        }
                    }
                    b"alphaModFix" if object_depth == 1 => {
                        if let Some(picture) =
                            anchor.as_mut().and_then(|anchor| anchor.picture.as_mut())
                        {
                            picture.opacity = attribute_value(element, b"amt")?
                                .and_then(|value| value.parse::<u32>().ok())
                                .unwrap_or(100_000);
                        }
                    }
                    b"txBody" if object_depth == 1 && !empty => {
                        in_text_body = true;
                        paragraphs.clear();
                    }
                    b"p" if in_text_body => {
                        if empty {
                            paragraphs.push(String::new());
                        } else {
                            paragraph = Some(String::new());
                        }
                    }
                    b"t" if paragraph.is_some() && !empty => in_text = true,
                    // A line break inside a paragraph.
                    b"br" if paragraph.is_some() => {
                        if let Some(paragraph) = paragraph.as_mut() {
                            paragraph.push('\n');
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref text) => {
                let decoded = text.decode()?;
                if marker_axis.is_some() {
                    number.push_str(&decoded);
                } else if in_text {
                    if let Some(paragraph) = paragraph.as_mut() {
                        paragraph.push_str(&decoded);
                    }
                }
            }
            Event::GeneralRef(ref reference) => {
                if in_text {
                    if let Some(decoded) = decoded_reference(reference) {
                        if let Some(paragraph) = paragraph.as_mut() {
                            paragraph.push_str(&decoded);
                        }
                    }
                }
            }
            Event::End(ref element) => {
                let qualified = element.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"col" | b"row" | b"colOff" | b"rowOff" => {
                        if let (Some(axis), Some(which), Some(anchor)) =
                            (marker_axis.take(), marker, anchor.as_mut())
                        {
                            let (cell, offset) = match which {
                                Marker::From => (&mut anchor.from, &mut anchor.from_offset),
                                Marker::To => (&mut anchor.to, &mut anchor.to_offset),
                            };
                            match axis {
                                MarkerAxis::Column | MarkerAxis::Row => {
                                    if let Ok(value) = number.trim().parse::<u32>() {
                                        let cell = cell.get_or_insert(CellRef::new(0, 0));
                                        if axis == MarkerAxis::Column {
                                            cell.column = value;
                                        } else {
                                            cell.row = value;
                                        }
                                    }
                                }
                                MarkerAxis::ColumnOffset | MarkerAxis::RowOffset => {
                                    if let Ok(value) = number.trim().parse::<i64>() {
                                        if axis == MarkerAxis::ColumnOffset {
                                            offset.0 = value;
                                        } else {
                                            offset.1 = value;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"from" | b"to" => marker = None,
                    b"t" => in_text = false,
                    b"p" if in_text_body => {
                        if let Some(paragraph) = paragraph.take() {
                            paragraphs.push(paragraph);
                        }
                    }
                    b"txBody" => {
                        in_text_body = false;
                        if let Some(anchor) = anchor.as_mut() {
                            if object_depth == 1 && anchor.text.is_none() {
                                anchor.text = Some(paragraphs.join("\n"));
                            }
                        }
                    }
                    b"sp" | b"pic" | b"graphicFrame" | b"grpSp" | b"cxnSp" if anchor.is_some() => {
                        object_depth = object_depth.saturating_sub(1);
                    }
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                        if let Some(anchor) = anchor.take() {
                            if let (Some(kind), Some(from)) = (anchor.kind, anchor.from) {
                                shapes.push(Shape {
                                    id: anchor.id.unwrap_or(0),
                                    name: anchor.name.unwrap_or_default(),
                                    kind,
                                    from,
                                    from_offset: anchor.from_offset,
                                    to: anchor.to,
                                    to_offset: anchor.to_offset,
                                    extent: anchor.extent,
                                    text: anchor.text,
                                    picture: anchor.picture,
                                    chart: anchor.chart,
                                    style: anchor
                                        .style
                                        .filter(|_| {
                                            matches!(
                                                kind,
                                                ShapeKind::Shape
                                                    | ShapeKind::TextBox
                                                    | ShapeKind::Connector
                                            )
                                        })
                                        .map(Box::new),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(shapes)
}

#[derive(Default)]
struct Anchor {
    id: Option<u32>,
    name: Option<String>,
    kind: Option<ShapeKind>,
    from: Option<CellRef>,
    from_offset: (i64, i64),
    to: Option<CellRef>,
    to_offset: (i64, i64),
    extent: Option<(i64, i64)>,
    text: Option<String>,
    picture: Option<Picture>,
    chart: Option<crate::chart::Chart>,
    style: Option<ShapeStyle>,
}

/// An element of a drawing, kept whole to read a shape's look from.
#[derive(Default)]
struct Node {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == name)
    }
    fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find_map(|node| {
            if node.name == name {
                Some(node)
            } else {
                node.find(name)
            }
        })
    }
}

/// Each anchor's shape look, in document order (`None` for an anchor whose
/// object is not a shape or connector).
fn shape_styles(xml: &str) -> Result<Vec<Option<ShapeStyle>>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let mut node = Node {
                    name: String::from_utf8_lossy(local_name(element.name().as_ref())).into_owned(),
                    ..Node::default()
                };
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    node.attributes.push((
                        String::from_utf8_lossy(local_name(attribute.key.as_ref())).into_owned(),
                        attribute.unescape_value()?.into_owned(),
                    ));
                }
                if matches!(event, Event::Empty(_)) {
                    stack.last_mut().unwrap().children.push(node);
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(node);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let root = stack.remove(0);
    let mut anchors = Vec::new();
    collect_anchors(&root, &mut anchors);
    Ok(anchors
        .into_iter()
        .map(|anchor| {
            let object = anchor
                .children
                .iter()
                .find(|node| matches!(node.name.as_str(), "sp" | "cxnSp"))?;
            Some(shape_style(object))
        })
        .collect())
}

fn collect_anchors<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    for child in &node.children {
        if matches!(
            child.name.as_str(),
            "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor"
        ) {
            out.push(child);
        } else {
            collect_anchors(child, out);
        }
    }
}

fn drawing_color(node: &Node) -> Option<DrawingColor> {
    let color = node.children.iter().find(|child| {
        matches!(
            child.name.as_str(),
            "srgbClr" | "schemeClr" | "sysClr" | "prstClr" | "scrgbClr"
        )
    })?;
    let hex = |value: &str| -> Option<[u8; 3]> {
        if value.len() != 6 {
            return None;
        }
        Some([
            u8::from_str_radix(&value[0..2], 16).ok()?,
            u8::from_str_radix(&value[2..4], 16).ok()?,
            u8::from_str_radix(&value[4..6], 16).ok()?,
        ])
    };
    let rgb =
        match color.name.as_str() {
            "srgbClr" => color.attr("val").and_then(hex),
            "sysClr" => color.attr("lastClr").and_then(hex).or(Some(
                if color.attr("val") == Some("window") {
                    [255, 255, 255]
                } else {
                    [0, 0, 0]
                },
            )),
            "scrgbClr" => {
                let channel = |name: &str| {
                    let linear = color.attr(name)?.parse::<f64>().ok()? / 100_000.0;
                    let srgb = if linear <= 0.003_130_8 {
                        linear * 12.92
                    } else {
                        1.055 * linear.powf(1.0 / 2.4) - 0.055
                    };
                    Some((srgb.clamp(0.0, 1.0) * 255.0).round() as u8)
                };
                Some([channel("r")?, channel("g")?, channel("b")?])
            }
            "prstClr" => Some(match color.attr("val")? {
                "black" => [0, 0, 0],
                "white" => [255, 255, 255],
                "red" => [255, 0, 0],
                "green" => [0, 128, 0],
                "blue" => [0, 0, 255],
                "yellow" => [255, 255, 0],
                "gray" | "grey" => [128, 128, 128],
                "orange" => [255, 165, 0],
                _ => [0, 0, 0],
            }),
            _ => None,
        };
    Some(DrawingColor {
        rgb,
        scheme: (color.name == "schemeClr")
            .then(|| color.attr("val").map(str::to_string))
            .flatten(),
        transforms: color
            .children
            .iter()
            .filter_map(|transform| {
                Some((
                    transform.name.clone(),
                    transform.attr("val")?.parse::<i32>().ok()?,
                ))
            })
            .collect(),
    })
}

fn paint(node: &Node) -> ShapePaint {
    for child in &node.children {
        match child.name.as_str() {
            "noFill" => return ShapePaint::None,
            "solidFill" => {
                return drawing_color(child).map_or(ShapePaint::Unset, ShapePaint::Solid);
            }
            "gradFill" => {
                let stops: Vec<DrawingColor> = child
                    .child("gsLst")
                    .map(|list| list.children.iter().filter_map(drawing_color).collect())
                    .unwrap_or_default();
                return if stops.is_empty() {
                    ShapePaint::Unset
                } else {
                    ShapePaint::Gradient(stops)
                };
            }
            "pattFill" => {
                return child
                    .child("fgClr")
                    .and_then(drawing_color)
                    .map_or(ShapePaint::Unset, ShapePaint::Solid);
            }
            _ => {}
        }
    }
    ShapePaint::Unset
}

fn shape_style(object: &Node) -> ShapeStyle {
    let mut style = ShapeStyle::default();
    if let Some(properties) = object.child("spPr") {
        if let Some(geometry) = properties.child("prstGeom") {
            style.geometry = geometry.attr("prst").map(str::to_string);
            if let Some(list) = geometry.child("avLst") {
                style.adjustments = list
                    .children
                    .iter()
                    .filter_map(|guide| {
                        let formula = guide.attr("fmla")?;
                        let value = formula.strip_prefix("val ")?.trim().parse().ok()?;
                        Some((guide.attr("name")?.to_string(), value))
                    })
                    .collect();
            }
        } else if properties.child("custGeom").is_some() {
            style.geometry = Some("rect".into());
        }
        if let Some(transform) = properties.child("xfrm") {
            style.rotation = transform
                .attr("rot")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let flag = |name: &str| {
                transform
                    .attr(name)
                    .is_some_and(|v| v == "1" || v == "true")
            };
            style.flip_h = flag("flipH");
            style.flip_v = flag("flipV");
        }
        style.fill = paint(properties);
        if let Some(line) = properties.child("ln") {
            style.line = paint(line);
            style.line_width = line.attr("w").and_then(|value| value.parse().ok());
            style.line_dash = line
                .child("prstDash")
                .and_then(|dash| dash.attr("val"))
                .map(str::to_string);
            style.head_end = line
                .child("headEnd")
                .and_then(|end| end.attr("type"))
                .filter(|kind| *kind != "none")
                .map(str::to_string);
            style.tail_end = line
                .child("tailEnd")
                .and_then(|end| end.attr("type"))
                .filter(|kind| *kind != "none")
                .map(str::to_string);
        }
    }
    if let Some(references) = object.child("style") {
        let reference = |name: &str| {
            let node = references.child(name)?;
            let index = node
                .attr("idx")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            Some((index, drawing_color(node).unwrap_or_default()))
        };
        style.style_fill = reference("fillRef");
        style.style_line = reference("lnRef");
        style.style_font = references.child("fontRef").and_then(drawing_color);
    }
    if let Some(body) = object.child("txBody") {
        let properties = body.child("bodyPr");
        style.text_anchor = properties
            .and_then(|properties| properties.attr("anchor"))
            .map(str::to_string);
        for (slot, key) in style
            .text_insets
            .iter_mut()
            .zip(["lIns", "tIns", "rIns", "bIns"])
        {
            *slot = properties
                .and_then(|properties| properties.attr(key))
                .and_then(|value| value.parse().ok());
        }
        if let Some(paragraph) = body.child("p") {
            style.text_align = paragraph
                .child("pPr")
                .and_then(|properties| properties.attr("algn"))
                .map(str::to_string);
            let run = paragraph
                .children
                .iter()
                .find(|node| node.name == "r")
                .and_then(|run| run.child("rPr"))
                .or_else(|| paragraph.child("endParaRPr"))
                .or_else(|| body.find("defRPr"));
            if let Some(run) = run {
                style.text_size = run.attr("sz").and_then(|value| value.parse().ok());
                style.text_bold = run.attr("b").is_some_and(|v| v == "1" || v == "true");
                style.text_italic = run.attr("i").is_some_and(|v| v == "1" || v == "true");
                style.text_color = run.child("solidFill").and_then(drawing_color);
            }
        }
    }
    style
}

#[derive(Clone, Copy)]
enum Marker {
    From,
    To,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkerAxis {
    Column,
    Row,
    ColumnOffset,
    RowOffset,
}

fn attribute_value(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}
