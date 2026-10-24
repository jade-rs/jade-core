//! Drawing objects — charts, pictures, shapes — put somewhere else or taken
//! away, as Excel does when one is dragged by its border or a handle, or
//! selected and deleted.
//!
//! A placed object is written with a two-cell anchor over the cells it was
//! put over. What it did when the grid changes stays what it was: a
//! one-cell anchor becomes `editAs="oneCell"` (moves with its cells, keeps
//! its size) and an absolute one `editAs="absolute"`, which is how Excel
//! writes those after a move.

use serde::{Deserialize, Serialize};

use jade_xlsx_reader::CellRef;

use crate::patch::{apply, Splice};
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
    let Some(found) = jade_xlsx_reader::object_protection::drawing_source(xml, id)? else {
        return Ok(None);
    };
    let prefix = found
        .qualified
        .rsplit_once(':')
        .map_or(String::new(), |(p, _)| format!("{p}:"));
    let qualified = format!("{prefix}twoCellAnchor");
    let mut head = format!(
        "<{qualified}{}",
        &xml[found.span.start + 1 + found.qualified.len()..found.head_end]
    );
    if found.kind != "twoCellAnchor" {
        let mode = if found.kind == "oneCellAnchor" {
            "oneCell"
        } else {
            "absolute"
        };
        head = crate::xml_edit::root_attributes(&head, &[("editAs", Some(mode.into()))])?;
    }
    let mut splices = vec![
        Splice {
            start: found.span.start,
            end: found.head_end,
            text: head,
        },
        Splice {
            start: found.end_start,
            end: found.span.end,
            text: format!("</{qualified}>"),
        },
        Splice {
            start: found.object_span.start,
            end: found.object_span.start,
            text: format!(
                "{}{}",
                marker(&prefix, "from", anchor.from, anchor.from_offset),
                marker(&prefix, "to", anchor.to, anchor.to_offset)
            ),
        },
    ];
    splices.extend(found.positions.into_iter().map(|r| Splice {
        start: r.start,
        end: r.end,
        text: String::new(),
    }));
    Ok(Some(apply(xml, splices)))
}

/// Remove only the unique owned anchor; foreign and opaque payloads stay intact.
pub fn delete_shape(xml: &str, id: u32) -> Result<Option<String>> {
    let Some(found) = jade_xlsx_reader::object_protection::drawing_source(xml, id)? else {
        return Ok(None);
    };
    Ok(Some(apply(
        xml,
        vec![Splice {
            start: found.span.start,
            end: found.span.end,
            text: String::new(),
        }],
    )))
}
