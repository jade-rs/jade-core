//! Tables on a slide (`a:tbl` in a `p:graphicFrame`): their cells' text, and
//! their grid scaled to a resized frame.

use std::ops::Range;

use super::{apply_patches, child, edit_tag, parse_nodes, shape_nodes, Node};
use crate::{Result, TableCell, WriteError};

/// The `a:tbl` of the frame with `id`.
fn table(nodes: &[Node], id: u32) -> Option<&Node> {
    shape_nodes(nodes)
        .into_iter()
        .filter(|node| node.is(1, "graphicFrame"))
        .find(|frame| {
            child(nodes, frame, 1, "nvGraphicFramePr")
                .and_then(|nv| child(nodes, nv, 1, "cNvPr"))
                .and_then(|identity| identity.attr("id"))
                .and_then(|value| value.parse::<u32>().ok())
                == Some(id)
        })
        .and_then(|frame| child(nodes, frame, 2, "graphic"))
        .and_then(|graphic| child(nodes, graphic, 2, "graphicData"))
        .and_then(|data| child(nodes, data, 2, "tbl"))
}

fn children<'a>(nodes: &'a [Node], parent: &Node, name: &str) -> Vec<&'a Node> {
    parent
        .children
        .iter()
        .map(|&index| &nodes[index])
        .filter(|node| node.is(2, name))
        .collect()
}

/// Whether a cell belongs to a merge another cell starts.
fn merged(cell: &Node) -> bool {
    ["hMerge", "vMerge"].iter().any(|key| {
        cell.attr(key)
            .is_some_and(|value| matches!(value, "1" | "true"))
    })
}

/// A paragraph's text: its runs and fields, a line break as `\n`.
fn paragraph_text(nodes: &[Node], paragraph: &Node) -> String {
    let mut text = String::new();
    for node in paragraph.children.iter().map(|&index| &nodes[index]) {
        if node.is(2, "r") || node.is(2, "fld") {
            if let Some(t) = child(nodes, node, 2, "t") {
                text.push_str(&t.text);
            }
        } else if node.is(2, "br") {
            text.push('\n');
        }
    }
    text
}

fn cell_text(nodes: &[Node], cell: &Node) -> String {
    child(nodes, cell, 2, "txBody")
        .map(|body| {
            children(nodes, body, "p")
                .into_iter()
                .map(|paragraph| paragraph_text(nodes, paragraph))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The table's cells, row by row.
pub(crate) fn cells(xml: &str, id: u32) -> Result<Vec<Vec<TableCell>>> {
    let nodes = parse_nodes(xml)?;
    let table = table(&nodes, id).ok_or(WriteError::NoShape(id))?;
    Ok(children(&nodes, table, "tr")
        .into_iter()
        .map(|row| {
            children(&nodes, row, "tc")
                .into_iter()
                .map(|cell| TableCell {
                    text: cell_text(&nodes, cell),
                    merged: merged(cell),
                })
                .collect()
        })
        .collect())
}

/// `xml` with cell (`row`, `column`) of the table `id` holding `text`, one
/// paragraph per line, each keeping the cell's first paragraph and run
/// properties. A cell inside a merge another cell starts is not editable,
/// as in PowerPoint.
pub(crate) fn set_cell_text(
    xml: &str,
    id: u32,
    row: usize,
    column: usize,
    text: &str,
) -> Result<String> {
    if let Some(bad) = text
        .chars()
        .find(|&c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n'))
    {
        return Err(WriteError::InvalidCharacter(bad as u32));
    }
    let nodes = parse_nodes(xml)?;
    let table = table(&nodes, id).ok_or(WriteError::NoShape(id))?;
    let cell = children(&nodes, table, "tr")
        .get(row)
        .and_then(|tr| children(&nodes, tr, "tc").get(column).copied())
        .ok_or(WriteError::NoShape(id))?;
    if merged(cell) {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let prefix = cell
        .qualified
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default();
    let body = child(&nodes, cell, 2, "txBody");
    let paragraphs = body
        .map(|body| children(&nodes, body, "p"))
        .unwrap_or_default();
    let first = paragraphs.first().copied();
    let outer = |node: Option<&Node>| node.map(|node| xml[node.outer.clone()].to_string());
    let paragraph_properties = outer(first.and_then(|p| child(&nodes, p, 2, "pPr")));
    let run_properties = first
        .and_then(|p| children(&nodes, p, "r").first().copied())
        .and_then(|run| child(&nodes, run, 2, "rPr"))
        .map(|rpr| xml[rpr.outer.clone()].to_string())
        .or_else(|| {
            outer(first.and_then(|p| child(&nodes, p, 2, "endParaRPr")))
                .map(|end| end.replace("endParaRPr", "rPr"))
        });
    let end_properties = outer(first.and_then(|p| child(&nodes, p, 2, "endParaRPr")));
    let mut written = String::new();
    for line in text.split('\n') {
        written.push_str(&format!("<{prefix}p>"));
        written.push_str(paragraph_properties.as_deref().unwrap_or_default());
        if line.is_empty() {
            written.push_str(end_properties.as_deref().unwrap_or_default());
        } else {
            written.push_str(&format!(
                "<{prefix}r>{}<{prefix}t>{}</{prefix}t></{prefix}r>",
                run_properties.as_deref().unwrap_or_default(),
                quick_xml::escape::escape(line)
            ));
        }
        written.push_str(&format!("</{prefix}p>"));
    }
    let patch: (Range<usize>, String) = match (body, paragraphs.first(), paragraphs.last()) {
        (Some(_), Some(first), Some(last)) => (first.outer.start..last.outer.end, written),
        (Some(body), _, _) => (body.range.end..body.range.end, written),
        (None, _, _) => (
            cell.range.start..cell.range.start,
            format!(
                "<{prefix}txBody><{prefix}bodyPr/><{prefix}lstStyle/>{written}</{prefix}txBody>"
            ),
        ),
    };
    Ok(apply_patches(xml, vec![patch]))
}

/// The patches that scale the table's column widths to `width` and its row
/// heights to `height`, in proportion, the last of each taking the rounding.
pub(super) fn scale_grid(
    xml: &str,
    nodes: &[Node],
    frame: &Node,
    (width, height): (i64, i64),
) -> Result<Vec<(Range<usize>, String)>> {
    let Some(table) = child(nodes, frame, 2, "graphic")
        .and_then(|graphic| child(nodes, graphic, 2, "graphicData"))
        .and_then(|data| child(nodes, data, 2, "tbl"))
    else {
        return Ok(Vec::new());
    };
    let columns = child(nodes, table, 2, "tblGrid")
        .map(|grid| children(nodes, grid, "gridCol"))
        .unwrap_or_default();
    let rows = children(nodes, table, "tr");
    let mut patches = scale(xml, &columns, "w", width)?;
    patches.extend(scale(xml, &rows, "h", height)?);
    Ok(patches)
}

fn scale(xml: &str, nodes: &[&Node], key: &str, total: i64) -> Result<Vec<(Range<usize>, String)>> {
    let sizes: Vec<i64> = nodes
        .iter()
        .map(|node| node.attr(key).and_then(|v| v.parse().ok()).unwrap_or(0))
        .collect();
    let sum: i64 = sizes.iter().sum();
    if sum <= 0 || sum == total {
        return Ok(Vec::new());
    }
    let mut given = 0;
    let mut patches = Vec::new();
    for (index, (node, size)) in nodes.iter().zip(&sizes).enumerate() {
        let scaled = if index + 1 == nodes.len() {
            total - given
        } else {
            (i128::from(*size) * i128::from(total) / i128::from(sum)) as i64
        };
        given += scaled;
        patches.push((
            node.opening.clone(),
            edit_tag(&xml[node.opening.clone()], &[(key, scaled.to_string())])?,
        ));
    }
    Ok(patches)
}
