//! Word ink: the InkML parts (W3C InkML 2011) a `w14:contentPart` points at.
//!
//! Word keeps each ink drawing as a drawing whose graphic is a
//! `w14:contentPart` relating to an `application/inkml+xml` part, inside an
//! `mc:AlternateContent` whose fallback is a picture of the strokes. This
//! reads such a part into strokes in the drawing's own pixel box, and writes
//! strokes back as InkML Word reads.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pixels in a centimetre at 96 dpi.
const PX_PER_CM: f64 = 96.0 / 2.54;
/// The resolution strokes are written at: 1/1000 cm.
const WRITE_RESOLUTION_PER_CM: f64 = 1000.0;

/// The relationship type Word relates an ink part with.
pub const INK_RELATIONSHIP_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
/// The ink part's content type.
pub const INK_CONTENT_TYPE: &str = "application/inkml+xml";

/// The pen a stroke was drawn with: 描画's ペン, 鉛筆 and 蛍光ペン.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InkTool {
    #[default]
    Pen,
    Pencil,
    Highlighter,
}

/// How a stroke is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkBrush {
    pub tool: InkTool,
    /// `#RRGGBB`.
    pub color: String,
    /// The tip's width and height in px.
    pub width_px: f64,
    pub height_px: f64,
    /// Office's ink effect (`inkEffects`): `rainbow`, `galaxy`, `gold`,
    /// `silver`, `bronze`, `rosegold`, `lava`, `ocean`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// InkML transparency, 0 (opaque) to 255.
    #[serde(default)]
    pub transparency: u8,
}

impl Default for InkBrush {
    fn default() -> Self {
        Self {
            tool: InkTool::Pen,
            color: "#000000".to_string(),
            width_px: 1.89,
            height_px: 1.89,
            effect: None,
            transparency: 0,
        }
    }
}

/// One stroke: its points, in px within the drawing's box, and its brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkTrace {
    pub points: Vec<(f64, f64)>,
    pub brush: InkBrush,
}

/// An ink drawing's content: the part it came from and its strokes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkContent {
    /// The `w14:contentPart`'s relationship, once the drawing is in a package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_id: Option<String>,
    pub traces: Vec<InkTrace>,
}

impl InkContent {
    /// The strokes' extent with their brushes, in px: `(left, top, right,
    /// bottom)`, or `None` with no points.
    pub fn bounds(traces: &[InkTrace]) -> Option<(f64, f64, f64, f64)> {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for trace in traces {
            let half = trace.brush.width_px.max(trace.brush.height_px) / 2.0;
            for &(x, y) in &trace.points {
                let (l, t, r, b) = bounds.unwrap_or((x - half, y - half, x + half, y + half));
                bounds = Some((
                    l.min(x - half),
                    t.min(y - half),
                    r.max(x + half),
                    b.max(y + half),
                ));
            }
        }
        bounds
    }
}

/// A length in an InkML unit, in px.
fn unit_to_px(value: f64, units: &str) -> f64 {
    value
        * match units.trim().to_ascii_lowercase().as_str() {
            "mm" => PX_PER_CM / 10.0,
            "in" => 96.0,
            "pt" => 96.0 / 72.0,
            "px" | "dev" => 1.0,
            "himetric" => PX_PER_CM / 1000.0,
            _ => PX_PER_CM,
        }
}

/// Channel values per px: an InkML `resolution` (in `1/<unit>`).
fn resolution_per_px(value: f64, units: &str) -> f64 {
    let unit = units.trim().trim_start_matches("1/");
    if value <= 0.0 {
        return 1.0;
    }
    value / unit_to_px(1.0, unit)
}

#[derive(Clone, Copy)]
enum Mode {
    Explicit,
    First,
    Second,
}

/// Decodes a trace's text into points, one value per channel.
fn decode_trace(text: &str, channels: usize) -> Vec<Vec<f64>> {
    let channels = channels.max(1);
    let mut modes = vec![Mode::Explicit; channels];
    let mut last = vec![0.0_f64; channels];
    let mut velocity = vec![0.0_f64; channels];
    let mut points = Vec::new();
    for point in text.split(',') {
        let mut values: Vec<(Option<char>, Option<f64>)> = Vec::new();
        let mut prefix: Option<char> = None;
        let chars: Vec<char> = point.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            if ch.is_whitespace() {
                index += 1;
            } else if matches!(ch, '!' | '\'' | '"') {
                prefix = Some(ch);
                index += 1;
            } else if matches!(ch, '?' | '*') {
                values.push((prefix.take(), None));
                index += 1;
            } else if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.') {
                let start = index;
                index += 1;
                while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.')
                {
                    index += 1;
                }
                let token: String = chars[start..index].iter().collect();
                values.push((prefix.take(), token.parse::<f64>().ok()));
            } else {
                index += 1;
            }
        }
        if values.is_empty() {
            continue;
        }
        let mut decoded = vec![0.0; channels];
        for (channel, slot) in decoded.iter_mut().enumerate() {
            let (prefix, value) = values.get(channel).copied().unwrap_or((None, None));
            match prefix {
                Some('!') => modes[channel] = Mode::Explicit,
                Some('\'') => modes[channel] = Mode::First,
                Some('"') => modes[channel] = Mode::Second,
                _ => {}
            }
            let first_point = points.is_empty();
            let v = match (value, modes[channel]) {
                (None, _) => last[channel],
                (Some(n), Mode::Explicit) => n,
                (Some(n), Mode::First) => last[channel] + n,
                (Some(n), Mode::Second) => last[channel] + velocity[channel] + n,
            };
            velocity[channel] = if first_point { 0.0 } else { v - last[channel] };
            if let (Some(n), Mode::First) = (value, modes[channel]) {
                velocity[channel] = n;
            }
            last[channel] = v;
            *slot = v;
        }
        points.push(decoded);
    }
    points
}

#[derive(Clone, Default)]
struct Context {
    channels: Vec<String>,
    /// Channel values per px, by channel name.
    resolution: HashMap<String, f64>,
}

/// Parses an InkML part: its strokes, in px from the ink's own origin.
pub fn parse_inkml(xml: &str) -> Vec<InkTrace> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut brushes: HashMap<String, InkBrush> = HashMap::new();
    let mut contexts: HashMap<String, Context> = HashMap::new();
    let mut current_brush: Option<(String, InkBrush)> = None;
    let mut current_context: Option<(String, Context)> = None;
    let mut group_brush: Vec<Option<String>> = Vec::new();
    let mut trace: Option<(Option<String>, Option<String>, String)> = None;
    let mut traces = Vec::new();
    let local = |name: &[u8]| -> String {
        let name = String::from_utf8_lossy(name).to_string();
        name.rsplit(':').next().unwrap_or("").to_string()
    };
    let attrs = |e: &quick_xml::events::BytesStart<'_>| -> HashMap<String, String> {
        e.attributes()
            .flatten()
            .map(|attribute| {
                let key = String::from_utf8_lossy(attribute.key.as_ref()).to_string();
                let key = key.rsplit(':').next().unwrap_or("").to_string();
                let value = attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                (key, value)
            })
            .collect()
    };
    loop {
        let event = reader.read_event_into(&mut buf);
        let (element, empty) = match &event {
            Ok(Event::Start(e)) => (Some(e.clone().into_owned()), false),
            Ok(Event::Empty(e)) => (Some(e.clone().into_owned()), true),
            _ => (None, false),
        };
        if let Some(e) = element {
            let name = local(e.name().as_ref());
            let a = attrs(&e);
            match name.as_str() {
                "brush" => {
                    let id = a.get("id").cloned().unwrap_or_default();
                    current_brush = Some((id.clone(), InkBrush::default()));
                    if empty {
                        brushes.insert(id, InkBrush::default());
                        current_brush = None;
                    }
                }
                "brushProperty" => {
                    if let Some((_, brush)) = current_brush.as_mut() {
                        let value = a.get("value").cloned().unwrap_or_default();
                        let units = a.get("units").cloned().unwrap_or_else(|| "cm".into());
                        match a.get("name").map(String::as_str) {
                            Some("width") => {
                                brush.width_px = unit_to_px(value.parse().unwrap_or(0.05), &units)
                            }
                            Some("height") => {
                                brush.height_px = unit_to_px(value.parse().unwrap_or(0.05), &units)
                            }
                            Some("color") => brush.color = value.to_ascii_uppercase(),
                            Some("tip") if value.eq_ignore_ascii_case("rectangle") => {
                                brush.tool = InkTool::Highlighter
                            }
                            Some("rasterOp") if value.eq_ignore_ascii_case("maskPen") => {
                                brush.tool = InkTool::Highlighter
                            }
                            Some("transparency") => {
                                brush.transparency = value.parse::<f64>().unwrap_or(0.0) as u8
                            }
                            Some("inkEffects") => {
                                if value.eq_ignore_ascii_case("pencil") {
                                    brush.tool = InkTool::Pencil;
                                } else if !value.is_empty() {
                                    brush.effect = Some(value.to_ascii_lowercase());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "context" => {
                    let id = a.get("id").cloned().unwrap_or_default();
                    current_context = Some((id, Context::default()));
                }
                "channel" => {
                    if let Some((_, context)) = current_context.as_mut() {
                        context
                            .channels
                            .push(a.get("name").cloned().unwrap_or_default());
                    }
                }
                "channelProperty" => {
                    if let Some((_, context)) = current_context.as_mut() {
                        if a.get("name").map(String::as_str) == Some("resolution") {
                            let value = a
                                .get("value")
                                .and_then(|value| value.parse::<f64>().ok())
                                .unwrap_or(1000.0);
                            let units = a.get("units").cloned().unwrap_or_else(|| "1/cm".into());
                            context.resolution.insert(
                                a.get("channel").cloned().unwrap_or_default(),
                                resolution_per_px(value, &units),
                            );
                        }
                    }
                }
                "traceGroup" => {
                    group_brush.push(a.get("brushRef").cloned());
                }
                "trace" => {
                    let brush = a
                        .get("brushRef")
                        .cloned()
                        .or_else(|| group_brush.iter().rev().flatten().next().cloned());
                    trace = Some((a.get("contextRef").cloned(), brush, String::new()));
                    if empty {
                        trace = None;
                    }
                }
                _ => {}
            }
        }
        match event {
            Ok(Event::Text(text)) => {
                if let Some((_, _, body)) = trace.as_mut() {
                    body.push_str(&String::from_utf8_lossy(text.as_ref()));
                }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                "brush" => {
                    if let Some((id, brush)) = current_brush.take() {
                        brushes.insert(id, brush);
                    }
                }
                "context" => {
                    if let Some((id, context)) = current_context.take() {
                        contexts.insert(id, context);
                    }
                }
                "traceGroup" => {
                    group_brush.pop();
                }
                "trace" => {
                    if let Some((context, brush, body)) = trace.take() {
                        let reference = |reference: Option<String>| {
                            reference.map(|reference| reference.trim_start_matches('#').to_string())
                        };
                        let context = reference(context)
                            .and_then(|id| contexts.get(&id).cloned())
                            .or_else(|| contexts.values().next().cloned())
                            .unwrap_or_default();
                        let brush = reference(brush)
                            .and_then(|id| brushes.get(&id).cloned())
                            .or_else(|| brushes.values().next().cloned())
                            .unwrap_or_default();
                        let x_channel = context
                            .channels
                            .iter()
                            .position(|channel| channel == "X")
                            .unwrap_or(0);
                        let y_channel = context
                            .channels
                            .iter()
                            .position(|channel| channel == "Y")
                            .unwrap_or(1);
                        let per_px = |channel: &str| {
                            context
                                .resolution
                                .get(channel)
                                .copied()
                                .unwrap_or(WRITE_RESOLUTION_PER_CM / PX_PER_CM)
                        };
                        let (x_scale, y_scale) = (per_px("X"), per_px("Y"));
                        let points: Vec<(f64, f64)> =
                            decode_trace(&body, context.channels.len().max(2))
                                .into_iter()
                                .map(|values| {
                                    (
                                        values.get(x_channel).copied().unwrap_or(0.0) / x_scale,
                                        values.get(y_channel).copied().unwrap_or(0.0) / y_scale,
                                    )
                                })
                                .collect();
                        if !points.is_empty() {
                            traces.push(InkTrace { points, brush });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    traces
}

/// Fits strokes read from InkML onto a drawing's box: their extent with the
/// brushes fills the box, as Word scales a resized ink drawing.
pub fn fit_traces_to_box(traces: Vec<InkTrace>, width_px: f64, height_px: f64) -> Vec<InkTrace> {
    let Some((left, top, right, bottom)) = InkContent::bounds(&traces) else {
        return traces;
    };
    let (natural_w, natural_h) = ((right - left).max(1e-6), (bottom - top).max(1e-6));
    let sx = if width_px > 0.0 {
        width_px / natural_w
    } else {
        1.0
    };
    let sy = if height_px > 0.0 {
        height_px / natural_h
    } else {
        1.0
    };
    traces
        .into_iter()
        .map(|mut trace| {
            for point in &mut trace.points {
                *point = ((point.0 - left) * sx, (point.1 - top) * sy);
            }
            trace
        })
        .collect()
}

/// Writes strokes as an InkML part Word reads: coordinates in 1/1000 cm.
pub fn write_inkml(traces: &[InkTrace]) -> String {
    let cm = |px: f64| px / PX_PER_CM;
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat><inkml:channel name="X" type="integer" max="32767" units="cm"/><inkml:channel name="Y" type="integer" max="32767" units="cm"/></inkml:traceFormat><inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="1000" units="1/cm"/><inkml:channelProperty channel="Y" name="resolution" value="1000" units="1/cm"/></inkml:channelProperties></inkml:inkSource></inkml:context>"#,
    );
    // One brush per distinct look.
    let mut brushes: Vec<&InkBrush> = Vec::new();
    for trace in traces {
        if !brushes.contains(&&trace.brush) {
            brushes.push(&trace.brush);
        }
    }
    for (index, brush) in brushes.iter().enumerate() {
        out.push_str(&format!(r#"<inkml:brush xml:id="br{index}">"#));
        out.push_str(&format!(
            r#"<inkml:brushProperty name="width" value="{:.4}" units="cm"/><inkml:brushProperty name="height" value="{:.4}" units="cm"/><inkml:brushProperty name="color" value="{}"/>"#,
            cm(brush.width_px),
            cm(brush.height_px),
            escape(&brush.color),
        ));
        match brush.tool {
            InkTool::Highlighter => out.push_str(
                r#"<inkml:brushProperty name="tip" value="rectangle"/><inkml:brushProperty name="rasterOp" value="maskPen"/>"#,
            ),
            InkTool::Pencil => {
                out.push_str(r#"<inkml:brushProperty name="inkEffects" value="pencil"/>"#)
            }
            InkTool::Pen => {}
        }
        if let Some(effect) = brush
            .effect
            .as_deref()
            .filter(|_| brush.tool == InkTool::Pen)
        {
            out.push_str(&format!(
                r#"<inkml:brushProperty name="inkEffects" value="{}"/>"#,
                escape(effect)
            ));
        }
        if brush.transparency > 0 {
            out.push_str(&format!(
                r#"<inkml:brushProperty name="transparency" value="{}"/>"#,
                brush.transparency
            ));
        }
        out.push_str("</inkml:brush>");
    }
    out.push_str("</inkml:definitions>");
    for trace in traces {
        let brush = brushes
            .iter()
            .position(|brush| **brush == trace.brush)
            .unwrap_or(0);
        out.push_str(&format!(
            r##"<inkml:trace contextRef="#ctx0" brushRef="#br{brush}">"##
        ));
        let points: Vec<String> = trace
            .points
            .iter()
            .map(|&(x, y)| {
                format!(
                    "{} {}",
                    (cm(x) * WRITE_RESOLUTION_PER_CM).round() as i64,
                    (cm(y) * WRITE_RESOLUTION_PER_CM).round() as i64
                )
            })
            .collect();
        out.push_str(&points.join(", "));
        out.push_str("</inkml:trace>");
    }
    out.push_str("</inkml:ink>");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differential_traces_decode() {
        // Explicit, then first differences, then second differences.
        let points = decode_trace("10 20, '1 '2, '1 '2, \"0 \"1", 2);
        assert_eq!(
            points,
            vec![
                vec![10.0, 20.0],
                vec![11.0, 22.0],
                vec![12.0, 24.0],
                vec![13.0, 27.0]
            ]
        );
        // Packed values: a sign starts the next one.
        assert_eq!(decode_trace("5-3", 2), vec![vec![5.0, -3.0]]);
    }

    #[test]
    fn strokes_round_trip_through_inkml() {
        let traces = vec![
            InkTrace {
                points: vec![(1.0, 1.0), (50.0, 20.0), (80.0, 5.0)],
                brush: InkBrush {
                    color: "#E71224".into(),
                    width_px: 3.78,
                    height_px: 3.78,
                    ..Default::default()
                },
            },
            InkTrace {
                points: vec![(10.0, 40.0), (90.0, 40.0)],
                brush: InkBrush {
                    tool: InkTool::Highlighter,
                    color: "#FFFF00".into(),
                    width_px: 3.78,
                    height_px: 22.68,
                    ..Default::default()
                },
            },
            InkTrace {
                points: vec![(0.0, 0.0), (5.0, 5.0)],
                brush: InkBrush {
                    tool: InkTool::Pencil,
                    ..Default::default()
                },
            },
            InkTrace {
                points: vec![(3.0, 3.0), (7.0, 9.0)],
                brush: InkBrush {
                    effect: Some("rainbow".into()),
                    ..Default::default()
                },
            },
        ];
        let read = parse_inkml(&write_inkml(&traces));
        assert_eq!(read.len(), 4);
        for (read, written) in read.iter().zip(&traces) {
            assert_eq!(read.brush.tool, written.brush.tool);
            assert_eq!(read.brush.color, written.brush.color);
            assert_eq!(read.brush.effect, written.brush.effect);
            assert!((read.brush.width_px - written.brush.width_px).abs() < 0.01);
            for (a, b) in read.points.iter().zip(&written.points) {
                assert!(
                    (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05,
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn word_ink_with_differences_and_a_resolution_is_read() {
        let xml = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat><inkml:channel name="X" type="integer" max="32767" units="in"/><inkml:channel name="Y" type="integer" max="32767" units="in"/><inkml:channel name="F" name2="F" type="integer" max="4095" units="dev"/></inkml:traceFormat><inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="2540" units="1/in"/><inkml:channelProperty channel="Y" name="resolution" value="2540" units="1/in"/><inkml:channelProperty channel="F" name="resolution" value="0" units="1/dev"/></inkml:channelProperties></inkml:inkSource></inkml:context><inkml:brush xml:id="br0"><inkml:brushProperty name="width" value="0.05" units="cm"/><inkml:brushProperty name="height" value="0.05" units="cm"/><inkml:brushProperty name="color" value="#0070c0"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#ctx0" brushRef="#br0">2540 2540 100,'2540'0'0</inkml:trace></inkml:ink>"##;
        let traces = parse_inkml(xml);
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].brush.color, "#0070C0");
        assert_eq!(traces[0].points, vec![(96.0, 96.0), (192.0, 96.0)]);
        let fitted = fit_traces_to_box(traces, 100.0, 10.0);
        let half = 0.05 * PX_PER_CM / 2.0;
        assert!((fitted[0].points[0].0 - half * 100.0 / (96.0 + 2.0 * half)).abs() < 1e-6);
    }
}
