//! Drawing objects — charts, pictures, shapes — put somewhere else or taken
//! away, as Excel does when one is dragged by its border or a handle, or
//! selected and deleted.
//!
//! A placed object is written with a two-cell anchor over the cells it was
//! put over. What it did when the grid changes stays what it was: a
//! one-cell anchor becomes `editAs="oneCell"` (moves with its cells, keeps
//! its size) and an absolute one `editAs="absolute"`, which is how Excel
//! writes those after a move.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use jade_xlsx_reader::CellRef;

use crate::patch::{apply, attribute, local, namespace_prefix, Splice};
use crate::Result;

/// Where an object goes: the cells its corners sit in, and how far into
/// each, as (x, y) in EMUs — 9,525 to a pixel at 96 dpi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeAnchor {
    pub from: CellRef,
    pub from_offset: (i64, i64),
    pub to: CellRef,
    pub to_offset: (i64, i64),
}

/// One anchor of the drawing, as the file has it.
struct AnchorSpan {
    span: (usize, usize),
    /// Its local name: `twoCellAnchor`, `oneCellAnchor`, `absoluteAnchor`.
    kind: Vec<u8>,
    prefix: String,
    edit_as: Option<String>,
    /// Where its object starts (after the position elements) and where its
    /// closing tag starts.
    content: (usize, usize),
    /// The object's `cNvPr/@id`.
    id: Option<u32>,
}

const OBJECTS: &[&[u8]] = &[
    b"sp",
    b"pic",
    b"graphicFrame",
    b"grpSp",
    b"cxnSp",
    b"contentPart",
    b"AlternateContent",
];

fn anchors(xml: &str) -> Result<Vec<AnchorSpan>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut found = Vec::new();
    let mut open: Option<AnchorSpan> = None;
    // How deep into the anchor's objects: its own is 1.
    let mut object_depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name().as_ref().to_vec();
                let name = local(&qualified);
                if depth == 1
                    && matches!(
                        name,
                        b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor"
                    )
                {
                    open = Some(AnchorSpan {
                        span: (start, end),
                        kind: name.to_vec(),
                        prefix: namespace_prefix(&qualified),
                        edit_as: attribute(element, b"editAs")?,
                        content: (end, end),
                        id: None,
                    });
                } else if let Some(anchor) = open.as_mut() {
                    if depth == 2 && OBJECTS.contains(&name) && object_depth == 0 {
                        anchor.content.0 = start;
                    }
                    if OBJECTS.contains(&name) && !empty {
                        object_depth += 1;
                    }
                    if name == b"cNvPr" && anchor.id.is_none() && object_depth <= 2 {
                        anchor.id = attribute(element, b"id")?.and_then(|value| value.parse().ok());
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                let name = local(element.name().as_ref()).to_vec();
                if depth == 1 {
                    if let Some(mut anchor) = open.take() {
                        anchor.span.1 = end;
                        anchor.content.1 = start;
                        found.push(anchor);
                    }
                    object_depth = 0;
                } else if open.is_some() && OBJECTS.contains(&name.as_slice()) {
                    object_depth = object_depth.saturating_sub(1);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(found)
}

fn marker(prefix: &str, name: &str, cell: CellRef, offset: (i64, i64)) -> String {
    format!(
        "<{prefix}{name}><{prefix}col>{}</{prefix}col><{prefix}colOff>{}</{prefix}colOff><{prefix}row>{}</{prefix}row><{prefix}rowOff>{}</{prefix}rowOff></{prefix}{name}>",
        cell.column,
        offset.0.max(0),
        cell.row,
        offset.1.max(0)
    )
}

/// The drawing with the object whose `cNvPr/@id` is `id` over `anchor`;
/// `None` when the drawing has no such object.
pub fn set_shape_anchor(xml: &str, id: u32, anchor: &ShapeAnchor) -> Result<Option<String>> {
    let Some(found) = anchors(xml)?.into_iter().find(|found| found.id == Some(id)) else {
        return Ok(None);
    };
    let prefix = &found.prefix;
    let edit_as = match (found.kind.as_slice(), found.edit_as.as_deref()) {
        (b"oneCellAnchor", _) => Some("oneCell"),
        (b"absoluteAnchor", _) => Some("absolute"),
        (_, edit_as) => edit_as,
    };
    let edit_as = edit_as
        .map(|value| format!(" editAs=\"{value}\""))
        .unwrap_or_default();
    let text = format!(
        "<{prefix}twoCellAnchor{edit_as}>{}{}{}</{prefix}twoCellAnchor>",
        marker(prefix, "from", anchor.from, anchor.from_offset),
        marker(prefix, "to", anchor.to, anchor.to_offset),
        &xml[found.content.0..found.content.1],
    );
    Ok(Some(apply(
        xml,
        vec![Splice {
            start: found.span.0,
            end: found.span.1,
            text,
        }],
    )))
}

/// The drawing without the object whose `cNvPr/@id` is `id`; `None` when
/// it has no such object.
pub fn delete_shape(xml: &str, id: u32) -> Result<Option<String>> {
    let Some(found) = anchors(xml)?.into_iter().find(|found| found.id == Some(id)) else {
        return Ok(None);
    };
    Ok(Some(apply(
        xml,
        vec![Splice {
            start: found.span.0,
            end: found.span.1,
            text: String::new(),
        }],
    )))
}
