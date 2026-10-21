//! Word's ink: pen strokes stored as an InkML part (`word/ink/inkN.xml`)
//! that a drawing's `w14:contentPart` points at. Read into strokes the
//! renderer draws, and written from strokes drawn in the app.

use crate::document::{InkDrawing, InkStroke};
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};

/// CSS px per centimetre.
const PX_PER_CM: f64 = 96.0 / 2.54;

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| local(&attribute.name) == name)
        .map(|attribute| attribute.value.as_str())
}

fn walk<'a>(element: &'a XmlElement, out: &mut Vec<&'a XmlElement>) {
    out.push(element);
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            walk(child, out);
        }
    }
}

fn text(element: &XmlElement) -> String {
    element
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Reads an InkML part: its traces as strokes in centimetres (the units
/// Word writes, at its channels' resolution), each with its brush.
pub fn parse_inkml(xml: &str) -> Option<InkDrawing> {
    let document = DocxXmlDocument::parse(xml).ok()?;
    let mut all = Vec::new();
    walk(document.root(), &mut all);
    // Each channel's resolution (units per cm); Word writes 1000/cm.
    let mut resolution = (1000.0, 1000.0);
    for element in &all {
        if local(&element.name) == "channelProperty" && attr(element, "name") == Some("resolution")
        {
            let value = attr(element, "value").and_then(|v| v.parse::<f64>().ok());
            let units = attr(element, "units").unwrap_or("1/cm");
            let per_cm = value.map(|value| match units {
                "1/in" => value / 2.54,
                "1/mm" => value * 10.0,
                _ => value,
            });
            match (attr(element, "channel"), per_cm) {
                (Some("X"), Some(value)) => resolution.0 = value,
                (Some("Y"), Some(value)) => resolution.1 = value,
                _ => {}
            }
        }
    }
    let mut brushes = std::collections::HashMap::new();
    for element in &all {
        if local(&element.name) != "brush" {
            continue;
        }
        let id = attr(element, "id").unwrap_or("").to_string();
        let mut color = "#000000".to_string();
        let mut width_cm = 0.05;
        let mut highlighter = false;
        for property in element.children.iter().filter_map(|node| match node {
            XmlNode::Element(child) if local(&child.name) == "brushProperty" => Some(child),
            _ => None,
        }) {
            let value = attr(property, "value").unwrap_or("");
            match attr(property, "name") {
                Some("color") => color = value.to_string(),
                Some("width") => {
                    let number = value.parse::<f64>().unwrap_or(0.05);
                    width_cm = match attr(property, "units") {
                        Some("mm") => number / 10.0,
                        Some("in") => number * 2.54,
                        Some("himetric") => number / 1000.0,
                        _ => number,
                    };
                }
                Some("transparency") => highlighter = value.parse::<f64>().unwrap_or(0.0) > 0.0,
                Some("tip") => highlighter |= value == "rectangle",
                _ => {}
            }
        }
        brushes.insert(id, (color, width_cm, highlighter));
    }
    let mut strokes = Vec::new();
    for element in &all {
        if local(&element.name) != "trace" {
            continue;
        }
        let brush = attr(element, "brushRef")
            .map(|reference| reference.trim_start_matches('#'))
            .and_then(|id| brushes.get(id))
            .cloned()
            .unwrap_or(("#000000".to_string(), 0.05, false));
        // InkML's sample modes (§4.2.4): `!` explicit values, `'` first
        // differences, `"` second differences; a mode lasts until the next
        // marker.
        #[derive(Clone, Copy, PartialEq)]
        enum Mode {
            Explicit,
            Velocity,
            Acceleration,
        }
        let mut mode = Mode::Explicit;
        let mut points = Vec::new();
        let mut position = (0.0, 0.0);
        let mut velocity = (0.0, 0.0);
        for sample in text(element).split(',') {
            let mut sample = sample.trim();
            if sample.is_empty() {
                continue;
            }
            for (marker, next) in [
                ('!', Mode::Explicit),
                ('\'', Mode::Velocity),
                ('"', Mode::Acceleration),
            ] {
                if let Some(rest) = sample.strip_prefix(marker) {
                    mode = next;
                    sample = rest;
                }
            }
            let values: Vec<f64> = sample
                .split(|ch: char| ch.is_whitespace() || matches!(ch, '!' | '\'' | '"'))
                .filter_map(|value| value.parse().ok())
                .collect();
            if values.len() < 2 {
                continue;
            }
            let value = (values[0], values[1]);
            match mode {
                Mode::Explicit => {
                    velocity = (value.0 - position.0, value.1 - position.1);
                    position = value;
                }
                Mode::Velocity => {
                    velocity = value;
                    position = (position.0 + value.0, position.1 + value.1);
                }
                Mode::Acceleration => {
                    velocity = (velocity.0 + value.0, velocity.1 + value.1);
                    position = (position.0 + velocity.0, position.1 + velocity.1);
                }
            }
            points.push((position.0 / resolution.0, position.1 / resolution.1));
        }
        if !points.is_empty() {
            strokes.push(InkStroke {
                points,
                color: brush.0,
                width: brush.1,
                highlighter: brush.2,
            });
        }
    }
    (!strokes.is_empty()).then_some(InkDrawing { strokes })
}

/// The bounding box of the strokes, in their units.
pub fn bounds(ink: &InkDrawing) -> (f64, f64, f64, f64) {
    let points = ink.strokes.iter().flat_map(|stroke| stroke.points.iter());
    let (mut left, mut top, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in points {
        left = left.min(*x);
        top = top.min(*y);
        right = right.max(*x);
        bottom = bottom.max(*y);
    }
    if !left.is_finite() {
        return (0.0, 0.0, 1.0, 1.0);
    }
    (
        left,
        top,
        (right - left).max(1e-6),
        (bottom - top).max(1e-6),
    )
}

/// Writes strokes drawn on the page (CSS px, with widths in px) as an
/// InkML part, in centimetres at 1000 per cm, and returns it with the
/// strokes' box (x, y, width, height in px, widened by the pen).
pub fn write_inkml(strokes: &[InkStroke]) -> (String, (f64, f64, f64, f64)) {
    let pen = strokes
        .iter()
        .map(|stroke| stroke.width)
        .fold(0.0, f64::max);
    let (left, top, width, height) = bounds(&InkDrawing {
        strokes: strokes.to_vec(),
    });
    let mut brushes = String::new();
    let mut traces = String::new();
    for (index, stroke) in strokes.iter().enumerate() {
        brushes.push_str(&format!(
            r##"<inkml:brush xml:id="br{index}"><inkml:brushProperty name="width" value="{:.4}" units="cm"/><inkml:brushProperty name="height" value="{:.4}" units="cm"/><inkml:brushProperty name="color" value="{}"/>{}</inkml:brush>"##,
            stroke.width / PX_PER_CM,
            stroke.width / PX_PER_CM,
            stroke.color,
            if stroke.highlighter {
                r#"<inkml:brushProperty name="tip" value="rectangle"/><inkml:brushProperty name="rasterOp" value="maskPen"/><inkml:brushProperty name="transparency" value="128"/>"#
            } else {
                ""
            }
        ));
        let samples: Vec<String> = stroke
            .points
            .iter()
            .map(|(x, y)| {
                format!(
                    "{} {}",
                    ((x - left) / PX_PER_CM * 1000.0).round() as i64,
                    ((y - top) / PX_PER_CM * 1000.0).round() as i64
                )
            })
            .collect();
        traces.push_str(&format!(
            r##"<inkml:trace contextRef="#ctx0" brushRef="#br{index}">{}</inkml:trace>"##,
            samples.join(", ")
        ));
    }
    let xml = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?><inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat><inkml:channel name="X" type="integer" max="32767" units="cm"/><inkml:channel name="Y" type="integer" max="32767" units="cm"/></inkml:traceFormat><inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="1000" units="1/cm"/><inkml:channelProperty channel="Y" name="resolution" value="1000" units="1/cm"/></inkml:channelProperties></inkml:inkSource></inkml:context>{brushes}</inkml:definitions>{traces}</inkml:ink>"##
    );
    (
        xml,
        (left - pen / 2.0, top - pen / 2.0, width + pen, height + pen),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_round_trip_through_inkml() {
        let strokes = vec![
            InkStroke {
                points: vec![(100.0, 100.0), (150.0, 120.0), (200.0, 100.0)],
                color: "#E71224".into(),
                width: 2.0,
                highlighter: false,
            },
            InkStroke {
                points: vec![(100.0, 140.0), (200.0, 140.0)],
                color: "#FFFF00".into(),
                width: 8.0,
                highlighter: true,
            },
        ];
        let (xml, frame) = write_inkml(&strokes);
        assert!(
            (frame.0 - 96.0).abs() < 1e-9 && (frame.2 - 108.0).abs() < 1e-9,
            "{frame:?}"
        );
        let ink = parse_inkml(&xml).unwrap();
        assert_eq!(ink.strokes.len(), 2);
        assert_eq!(ink.strokes[0].color, "#E71224");
        assert!(ink.strokes[1].highlighter);
        // In centimetres, from the strokes' corner.
        let (x, y) = ink.strokes[0].points[1];
        assert!((x - 50.0 / PX_PER_CM).abs() < 0.001 && (y - 20.0 / PX_PER_CM).abs() < 0.001);
        assert!((ink.strokes[0].width - 2.0 / PX_PER_CM).abs() < 0.001);
    }

    #[test]
    fn words_compressed_traces_read_as_differences() {
        let xml = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:trace>1000 1000, '10 0, 10 5, "0 0</inkml:trace></inkml:ink>"##;
        let ink = parse_inkml(xml).unwrap();
        let points: Vec<(i64, i64)> = ink.strokes[0]
            .points
            .iter()
            .map(|(x, y)| ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64))
            .collect();
        assert_eq!(
            points,
            vec![(1000, 1000), (1010, 1000), (1020, 1005), (1030, 1010)]
        );
    }
}
