//! The parts anchored to cells when the grid changes shape: the drawing
//! (shapes, text boxes, pictures, charts), the legacy comments and the VML
//! that positions them, and the charts whose series point at the sheet.
//!
//! Excel moves an object with the cells it is anchored to: rows inserted
//! above it push it down, rows deleted above it pull it up, rows deleted
//! under it shrink it to the boundary, and an object whose every anchor
//! cell went is removed. A comment goes with its cell.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_xlsx_reader::{shift_formula, Axis, CellRef, StructureChange};

use crate::patch::{apply, attribute, escape, local, Splice};
use crate::shift::{reference_text, rewrite};
use crate::Result;

/// Where an anchor marker on the changed axis lands.
///
/// A marker inside deleted rows or columns is pulled to the boundary (`at`)
/// with a zero offset, rather than dropped: the object shrinks, the way
/// Excel shrinks a shape when the rows under it go. `inside` says the
/// marker was in the deleted range, so a caller can tell whether the whole
/// object was.
fn map_marker(change: &StructureChange, index: u32) -> (u32, bool) {
    let limit = match change.axis {
        Axis::Rows => jade_xlsx_reader::model::MAX_ROW_COUNT,
        Axis::Columns => jade_xlsx_reader::model::MAX_COLUMN_COUNT,
    };
    if change.inserted {
        if index >= change.at {
            ((index + change.count).min(limit - 1), false)
        } else {
            (index, false)
        }
    } else if index < change.at {
        (index, false)
    } else if index < change.at.saturating_add(change.count) {
        (change.at.min(limit - 1), true)
    } else {
        (index - change.count, false)
    }
}

/// The drawing part with every anchor moved. Anchors of shapes `editAs`
/// absolute stay where they are, as Excel leaves them.
pub fn shift_drawing(xml: &str, change: &StructureChange) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();

    // The anchor being walked: its start, whether it moves, and whether its
    // markers were inside a deletion (from, to).
    let mut anchor: Option<(usize, bool, bool, Option<bool>)> = None;
    let mut in_marker = false;
    let mut axis_content: Option<usize> = None;
    // The offset after a marker pulled to the boundary is zeroed.
    let mut zero_offset = false;
    let mut offset_content: Option<usize> = None;
    let axis_name: &[u8] = match change.axis {
        Axis::Rows => b"row",
        Axis::Columns => b"col",
    };
    let offset_name: &[u8] = match change.axis {
        Axis::Rows => b"rowOff",
        Axis::Columns => b"colOff",
    };

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                match local(element.name().as_ref()) {
                    b"twoCellAnchor" | b"oneCellAnchor" if !empty => {
                        let moves = attribute(element, b"editAs")?.as_deref() != Some("absolute");
                        anchor = Some((start, moves, false, None));
                    }
                    b"from" | b"to" if anchor.is_some() && !empty => {
                        in_marker = true;
                        if local(element.name().as_ref()) == b"to" {
                            if let Some(anchor) = anchor.as_mut() {
                                anchor.3 = Some(false);
                            }
                        }
                    }
                    name if in_marker && name == axis_name && !empty => {
                        axis_content = Some(head_end);
                    }
                    name if in_marker && name == offset_name && !empty && zero_offset => {
                        offset_content = Some(head_end);
                        zero_offset = false;
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                name if name == axis_name => {
                    if let (Some(content_start), Some(anchor)) =
                        (axis_content.take(), anchor.as_mut())
                    {
                        if anchor.1 {
                            if let Ok(index) = xml[content_start..start].trim().parse::<u32>() {
                                let (mapped, inside) = map_marker(change, index);
                                if mapped != index {
                                    splices.push(Splice {
                                        start: content_start,
                                        end: start,
                                        text: mapped.to_string(),
                                    });
                                }
                                if inside {
                                    zero_offset = true;
                                    match anchor.3 {
                                        Some(_) => anchor.3 = Some(true),
                                        None => anchor.2 = true,
                                    }
                                }
                            }
                        }
                    }
                }
                name if name == offset_name => {
                    if let Some(content_start) = offset_content.take() {
                        splices.push(Splice {
                            start: content_start,
                            end: start,
                            text: "0".to_string(),
                        });
                    }
                }
                b"from" | b"to" => {
                    in_marker = false;
                    zero_offset = false;
                }
                b"twoCellAnchor" | b"oneCellAnchor" => {
                    if let Some((anchor_start, moves, from_inside, to_inside)) = anchor.take() {
                        // Every anchor cell went: so does the object. A
                        // one-cell anchor (move but don't size) only moves.
                        if moves && from_inside && to_inside == Some(true) {
                            let end = reader.buffer_position() as usize;
                            splices.push(Splice {
                                start: anchor_start,
                                end,
                                text: String::new(),
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The comments part with every comment moved to where its cell went;
/// `None` when no comment is left, so the caller can drop the part.
pub fn shift_comments(xml: &str, change: &StructureChange) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut kept = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"comment" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                let Some(at) =
                    attribute(element, b"ref")?.and_then(|value| CellRef::parse(&value).ok())
                else {
                    kept += 1;
                    continue;
                };
                match change.map_cell(at) {
                    Some(mapped) => {
                        kept += 1;
                        if mapped != at {
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(
                                    element,
                                    &[("ref", Some(reference_text(mapped)))],
                                    empty,
                                )?,
                            });
                        }
                    }
                    None => {
                        if !empty {
                            let name = element.name().as_ref().to_vec();
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        splices.push(Splice {
                            start,
                            end,
                            text: String::new(),
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if kept == 0 {
        return Ok(None);
    }
    Ok(Some(apply(xml, splices)))
}

/// The legacy drawing (VML) with every note moved to its cell's new place
/// and every anchor shifted. A note whose cell went is removed. VML that
/// does not parse as XML is returned as it was: better a note drawn in the
/// old place than a part Excel refuses.
pub fn shift_vml(xml: &str, change: &StructureChange) -> String {
    shift_vml_impl(xml, change).unwrap_or_else(|_| xml.to_string())
}

fn shift_vml_impl(xml: &str, change: &StructureChange) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    // The shape being walked: its start, and whether it is a note whose cell
    // was deleted.
    let mut shape: Option<(usize, bool)> = None;
    let mut is_note = false;
    let mut content: Option<(usize, &[u8])> = None;
    let cell_name: &[u8] = match change.axis {
        Axis::Rows => b"Row",
        Axis::Columns => b"Column",
    };
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                match local(element.name().as_ref()) {
                    b"shape" if !empty => {
                        shape = Some((start, false));
                        is_note = false;
                    }
                    b"ClientData" if shape.is_some() => {
                        is_note = attribute(element, b"ObjectType")?.as_deref() == Some("Note");
                    }
                    b"Anchor" if shape.is_some() && !empty => content = Some((head_end, b"Anchor")),
                    name if shape.is_some() && name == cell_name && !empty => {
                        content = Some((head_end, cell_name));
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                b"Anchor" => {
                    if let Some((content_start, b"Anchor")) = content.take() {
                        let text = &xml[content_start..start];
                        let mut parts: Vec<String> = text
                            .split(',')
                            .map(|part| part.trim().to_string())
                            .collect();
                        // LeftColumn, LeftOffset, TopRow, TopOffset,
                        // RightColumn, RightOffset, BottomRow, BottomOffset.
                        let (first, second) = match change.axis {
                            Axis::Rows => (2, 6),
                            Axis::Columns => (0, 4),
                        };
                        if parts.len() == 8 {
                            for slot in [first, second] {
                                if let Ok(index) = parts[slot].parse::<u32>() {
                                    let (mapped, inside) = map_marker(change, index);
                                    parts[slot] = mapped.to_string();
                                    if inside {
                                        parts[slot + 1] = "0".to_string();
                                    }
                                }
                            }
                            let joined = parts.join(", ");
                            if joined != text {
                                splices.push(Splice {
                                    start: content_start,
                                    end: start,
                                    text: joined,
                                });
                            }
                        }
                    }
                }
                name if name == cell_name => {
                    if let Some((content_start, kind)) = content.take() {
                        if kind == cell_name {
                            if let Ok(index) = xml[content_start..start].trim().parse::<u32>() {
                                match change.map_index(index) {
                                    Some(mapped) => {
                                        if mapped != index {
                                            splices.push(Splice {
                                                start: content_start,
                                                end: start,
                                                text: mapped.to_string(),
                                            });
                                        }
                                    }
                                    None => {
                                        if is_note {
                                            if let Some(shape) = shape.as_mut() {
                                                shape.1 = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                b"shape" => {
                    if let Some((shape_start, doomed)) = shape.take() {
                        if doomed {
                            let end = reader.buffer_position() as usize;
                            splices.push(Splice {
                                start: shape_start,
                                end,
                                text: String::new(),
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// A chart part with every series reference (`<c:f>`) into the changed
/// sheet moved. Chart references are always sheet-qualified, so a chart on
/// another sheet follows too.
pub fn shift_chart(xml: &str, change: &StructureChange) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut content: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) if local(element.name().as_ref()) == b"f" => {
                content = Some(reader.buffer_position() as usize);
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"f" => {
                if let Some(content_start) = content.take() {
                    let raw = &xml[content_start..start];
                    let formula =
                        quick_xml::escape::unescape(raw).map_err(quick_xml::Error::from)?;
                    let shifted = shift_formula(&formula, "", change);
                    if shifted != formula {
                        splices.push(Splice {
                            start: content_start,
                            end: start,
                            text: escape(&shifted),
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The drawing part with the text of the shape whose `cNvPr/@id` is `id`
/// replaced by `text`, one paragraph per line. The first paragraph's
/// properties and the first run's character properties are kept, so the
/// new text wears the font the old one did. `None` when the drawing has no
/// such shape, or the shape has no text body to write into.
pub fn set_shape_text(xml: &str, id: u32, text: &str) -> Result<Option<String>> {
    let Some(source) = jade_xlsx_reader::object_protection::drawing_source(xml, id)? else {
        return Ok(None);
    };
    let Some(body) = source.text else {
        return Ok(None);
    };
    let mut prefix = "jadeText".to_string();
    while xml.contains(&prefix) {
        prefix.push('_');
    }
    let mut replacement = String::new();
    for line in text.split('\n') {
        replacement.push_str(&format!(
            "<{prefix}:p xmlns:{prefix}=\"{}\"{}>{}",
            escape(&body.namespace),
            body.declarations,
            body.paragraph_properties
        ));
        if line.is_empty() {
            replacement.push_str(&body.end_properties);
        } else {
            replacement.push_str(&format!(
                "<{prefix}:r>{}<{prefix}:t xml:space=\"preserve\">{}</{prefix}:t></{prefix}:r>",
                body.run_properties,
                escape(line)
            ));
        }
        replacement.push_str(&format!("</{prefix}:p>"));
    }
    let mut splices: Vec<_> = body
        .paragraphs
        .into_iter()
        .map(|r| Splice {
            start: r.start,
            end: r.end,
            text: String::new(),
        })
        .collect();
    if let Some(first) = splices.first_mut() {
        first.text = replacement;
    } else {
        splices.push(Splice {
            start: body.insertion,
            end: body.insertion,
            text: replacement,
        });
    }
    Ok(Some(apply(xml, splices)))
}
