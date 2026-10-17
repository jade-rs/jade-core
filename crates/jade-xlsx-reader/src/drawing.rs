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
use crate::model::{CellRef, Picture, Shape, ShapeKind};
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

    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                        anchor = Some(Anchor::default());
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
