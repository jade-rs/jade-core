//! Tables on a slide (`a:tbl` in a `p:graphicFrame`): their cells' text,
//! rows and columns added and removed, and their grid scaled to a resized
//! frame.

use std::ops::Range;

use super::{apply_patches, child, edit_tag, edit_tag_attributes, parse_nodes, shape_nodes, Node};
use crate::{Result, TableCell, TableChange, WriteError};

/// The `a:tbl` of the frame with `id`.
fn table(nodes: &[Node], id: u32) -> Option<&Node> {
    frame(nodes, id)
        .and_then(|frame| child(nodes, frame, 2, "graphic"))
        .and_then(|graphic| child(nodes, graphic, 2, "graphicData"))
        .and_then(|data| child(nodes, data, 2, "tbl"))
}

/// The `p:graphicFrame` with `id`.
fn frame(nodes: &[Node], id: u32) -> Option<&Node> {
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
                    row_span: span(cell, "rowSpan"),
                    column_span: span(cell, "gridSpan"),
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

fn flag(cell: &Node, key: &str) -> bool {
    cell.attr(key)
        .is_some_and(|value| matches!(value, "1" | "true"))
}

fn span(cell: &Node, key: &str) -> usize {
    cell.attr(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn size(node: &Node, key: &str) -> i64 {
    node.attr(key).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// A copy of `cell` with no text — one empty paragraph keeping the first
/// paragraph's properties — and without the merge attributes in `drop`.
fn emptied_cell(xml: &str, nodes: &[Node], cell: &Node, drop: &[&str]) -> Result<String> {
    let prefix = prefix_of(cell);
    let mut out = edit_tag_attributes(&xml[cell.opening.clone()], &[], drop)?;
    if cell.empty {
        return Ok(out);
    }
    if let Some(body) = child(nodes, cell, 2, "txBody") {
        let outer = |node: Option<&Node>| {
            node.map(|node| xml[node.outer.clone()].to_string())
                .unwrap_or_default()
        };
        let first = children(nodes, body, "p").first().copied();
        let end = first
            .and_then(|p| child(nodes, p, 2, "endParaRPr"))
            .map(|end| xml[end.outer.clone()].to_string())
            .or_else(|| {
                first
                    .and_then(|p| children(nodes, p, "r").first().copied())
                    .and_then(|run| child(nodes, run, 2, "rPr"))
                    .map(|rpr| xml[rpr.outer.clone()].replace("rPr", "endParaRPr"))
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<{prefix}txBody>{}{}<{prefix}p>{}{end}</{prefix}p></{prefix}txBody>",
            outer(child(nodes, body, 2, "bodyPr")),
            outer(child(nodes, body, 2, "lstStyle")),
            outer(first.and_then(|p| child(nodes, p, 2, "pPr"))),
        ));
    }
    if let Some(properties) = child(nodes, cell, 2, "tcPr") {
        out.push_str(&xml[properties.outer.clone()]);
    }
    out.push_str(&format!("</{}>", cell.qualified));
    Ok(out)
}

fn prefix_of(node: &Node) -> String {
    node.qualified
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default()
}

/// `xml` with a row or column of table `id` added or removed, as PowerPoint
/// does: a new one is empty, formatted (and as tall or wide) as the one it
/// pushes over — the last, when added at the end — and the frame grows or
/// shrinks by it. A change that would cut a merged cell is refused, as is
/// removing the last row or column.
pub(crate) fn change(xml: &str, id: u32, change: TableChange) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let refused = || WriteError::UnsupportedObjectOperation(id);
    let frame = frame(&nodes, id).ok_or(WriteError::NoShape(id))?;
    let table = table(&nodes, id).ok_or(WriteError::NoShape(id))?;
    let extent = child(&nodes, frame, 1, "xfrm")
        .and_then(|xfrm| child(&nodes, xfrm, 2, "ext"))
        .ok_or(WriteError::NoShape(id))?;
    let grid = child(&nodes, table, 2, "tblGrid").ok_or(refused())?;
    let columns = children(&nodes, grid, "gridCol");
    let rows = children(&nodes, table, "tr");
    let cells: Vec<Vec<&Node>> = rows.iter().map(|row| children(&nodes, row, "tc")).collect();
    if cells.iter().any(|row| row.len() != columns.len()) {
        return Err(refused());
    }
    let resize = |key: &str, by: i64| -> Result<(Range<usize>, String)> {
        let total = (size(extent, key) + by).max(0);
        Ok((
            extent.opening.clone(),
            edit_tag(&xml[extent.opening.clone()], &[(key, total.to_string())])?,
        ))
    };
    let mut patches = Vec::new();
    match change {
        TableChange::InsertRow(at) => {
            if at > rows.len() || rows.is_empty() {
                return Err(refused());
            }
            // A vertical merge running across the new row's place.
            if at < rows.len() && at > 0 && cells[at].iter().any(|cell| flag(cell, "vMerge")) {
                return Err(refused());
            }
            let template = rows[at.min(rows.len() - 1)];
            // Its height and cells: the row's id (`extLst`) stays its own.
            let mut row = xml[template.opening.clone()].to_string();
            for cell in &cells[at.min(rows.len() - 1)] {
                row.push_str(&emptied_cell(xml, &nodes, cell, &["rowSpan", "vMerge"])?);
            }
            row.push_str(&format!("</{}>", template.qualified));
            let place = if at < rows.len() {
                rows[at].outer.start
            } else {
                rows[rows.len() - 1].outer.end
            };
            patches.push((place..place, row));
            patches.push(resize("cy", size(template, "h"))?);
        }
        TableChange::DeleteRow(at) => {
            if at >= rows.len() || rows.len() == 1 {
                return Err(refused());
            }
            if cells[at]
                .iter()
                .any(|cell| flag(cell, "vMerge") || span(cell, "rowSpan") > 1)
            {
                return Err(refused());
            }
            patches.push((rows[at].outer.clone(), String::new()));
            patches.push(resize("cy", -size(rows[at], "h"))?);
        }
        TableChange::InsertColumn(at) => {
            if at > columns.len() || columns.is_empty() {
                return Err(refused());
            }
            if at < columns.len() && at > 0 && cells.iter().any(|row| flag(row[at], "hMerge")) {
                return Err(refused());
            }
            let from = at.min(columns.len() - 1);
            let template = columns[from];
            // The column's width alone: its id (`extLst`) stays its own.
            let column = if template.empty {
                xml[template.outer.clone()].to_string()
            } else {
                format!(
                    "{}</{}>",
                    &xml[template.opening.clone()],
                    template.qualified
                )
            };
            let before = |nodes: &[&Node]| {
                if at < nodes.len() {
                    nodes[at].outer.start
                } else {
                    nodes[nodes.len() - 1].outer.end
                }
            };
            let place = before(&columns);
            patches.push((place..place, column));
            for row in &cells {
                let place = before(row);
                patches.push((
                    place..place,
                    emptied_cell(xml, &nodes, row[from], &["gridSpan", "hMerge"])?,
                ));
            }
            patches.push(resize("cx", size(template, "w"))?);
        }
        TableChange::MergeCells {
            row,
            column,
            rows,
            columns,
        } => {
            patches.extend(merge_cells(
                xml,
                &nodes,
                &cells,
                (row, column),
                (rows, columns),
            )?);
        }
        TableChange::SplitCell { row, column } => {
            let anchor = cells
                .get(row)
                .and_then(|cells| cells.get(column))
                .ok_or_else(refused)?;
            let (rows, columns) = (span(anchor, "rowSpan"), span(anchor, "gridSpan"));
            if merged(anchor) || (rows <= 1 && columns <= 1) {
                return Err(refused());
            }
            for (r, line) in cells.iter().enumerate().skip(row).take(rows) {
                for (c, cell) in line.iter().enumerate().skip(column).take(columns) {
                    let drop: &[&str] = if (r, c) == (row, column) {
                        &["gridSpan", "rowSpan"]
                    } else {
                        &["hMerge", "vMerge"]
                    };
                    patches.push((
                        cell.opening.clone(),
                        edit_tag_attributes(&xml[cell.opening.clone()], &[], drop)?,
                    ));
                }
            }
        }
        TableChange::DeleteColumn(at) => {
            if at >= columns.len() || columns.len() == 1 {
                return Err(refused());
            }
            if cells
                .iter()
                .any(|row| flag(row[at], "hMerge") || span(row[at], "gridSpan") > 1)
            {
                return Err(refused());
            }
            patches.push((columns[at].outer.clone(), String::new()));
            for row in &cells {
                patches.push((row[at].outer.clone(), String::new()));
            }
            patches.push(resize("cx", -size(columns[at], "w"))?);
        }
    }
    Ok(apply_patches(xml, patches))
}

/// The patches that merge the `rows` × `columns` cells from `at` into the
/// first. Every merge the range touches must lie inside it: a cell
/// continuing a merge from outside, or a merge reaching out of it, is
/// refused, as PowerPoint will not cut one.
fn merge_cells(
    xml: &str,
    nodes: &[Node],
    cells: &[Vec<&Node>],
    (row, column): (usize, usize),
    (rows, columns): (usize, usize),
) -> Result<Vec<(Range<usize>, String)>> {
    let refused = || WriteError::UnsupportedObjectOperation(0);
    let (last_row, last_column) = (row + rows, column + columns);
    if rows * columns < 2
        || last_row > cells.len()
        || cells.first().is_none_or(|line| last_column > line.len())
    {
        return Err(refused());
    }
    for (r, line) in cells.iter().enumerate().take(last_row).skip(row) {
        for (c, cell) in line.iter().enumerate().take(last_column).skip(column) {
            let continues_from_outside =
                (c == column && flag(cell, "hMerge")) || (r == row && flag(cell, "vMerge"));
            let reaches_out =
                c + span(cell, "gridSpan") > last_column || r + span(cell, "rowSpan") > last_row;
            if continues_from_outside || reaches_out {
                return Err(refused());
            }
        }
    }
    let mut patches = Vec::new();
    let mut moved = String::new();
    for (r, line) in cells.iter().enumerate().take(last_row).skip(row) {
        for (c, cell) in line.iter().enumerate().take(last_column).skip(column) {
            let first = (r, c) == (row, column);
            let tag = &xml[cell.opening.clone()];
            let opening = if first {
                let mut spans = Vec::new();
                if columns > 1 {
                    spans.push(("gridSpan", columns.to_string()));
                }
                if rows > 1 {
                    spans.push(("rowSpan", rows.to_string()));
                }
                edit_tag_attributes(tag, &spans, &["hMerge", "vMerge"])?
            } else {
                let mut merges = Vec::new();
                let mut drop = vec!["gridSpan", "rowSpan"];
                if c > column {
                    merges.push(("hMerge", "1".to_string()));
                } else {
                    drop.push("hMerge");
                }
                if r > row {
                    merges.push(("vMerge", "1".to_string()));
                } else {
                    drop.push("vMerge");
                }
                edit_tag_attributes(tag, &merges, &drop)?
            };
            patches.push((cell.opening.clone(), opening));
            // The rest give the first their text, paragraph by paragraph.
            if first || cell_text(nodes, cell).is_empty() {
                continue;
            }
            let paragraphs = child(nodes, cell, 2, "txBody")
                .map(|body| children(nodes, body, "p"))
                .unwrap_or_default();
            if let (Some(start), Some(end)) = (paragraphs.first(), paragraphs.last()) {
                moved.push_str(&xml[start.outer.start..end.outer.end]);
                let prefix = prefix_of(cell);
                let properties = child(nodes, start, 2, "pPr")
                    .map(|pr| xml[pr.outer.clone()].to_string())
                    .unwrap_or_default();
                patches.push((
                    start.outer.start..end.outer.end,
                    format!("<{prefix}p>{properties}</{prefix}p>"),
                ));
            }
        }
    }
    if !moved.is_empty() {
        let anchor = cells[row][column];
        let paragraphs = child(nodes, anchor, 2, "txBody")
            .map(|body| children(nodes, body, "p"))
            .unwrap_or_default();
        let place = match (
            cell_text(nodes, anchor).is_empty(),
            paragraphs.first(),
            paragraphs.last(),
        ) {
            // An empty first cell gives way to the text.
            (true, Some(start), Some(end)) => start.outer.start..end.outer.end,
            (false, _, Some(end)) => end.outer.end..end.outer.end,
            _ => return Err(refused()),
        };
        patches.push((place, moved));
    }
    Ok(patches)
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
