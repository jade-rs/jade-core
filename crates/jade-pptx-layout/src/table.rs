//! Breaks a table into the cells and borders the renderer draws.
//!
//! A cell becomes a rectangle with the cell's fill and text; each stated
//! border becomes a line along that edge. Rows are at least as tall as
//! `a:tr h` says and grow to fit their tallest cell, as PowerPoint grows them.
//! Borders draw after every cell so no fill covers them.

use jade_pptx_reader::{Table, TableCell, TextAnchor};

use crate::style::text_context_for_table;
use crate::table_style::{cell_style, resolve_table_style, CellPosition, CellStyle};
use crate::text::{anchor_lines, flow_text, PlacedLine, TextFrame};
use crate::{
    emu_to_px_f, placed_rect, resolve_fill, resolve_outline, Context, GroupSpace, PlacedGeometry,
    PlacedOrigin, PlacedShape, Rect,
};

/// A cell that draws: its grid position, span, and text laid out from the
/// top (anchoring waits for the row height).
struct FlowedCell<'a> {
    cell: &'a TableCell,
    row: usize,
    column: usize,
    lines: Vec<PlacedLine>,
    /// The last line's bottom from the cell's top; the cell needs this plus
    /// the bottom inset to show all its text.
    content_bottom: f64,
    frame: TextFrame,
}

pub(crate) fn place_table(
    table: &Table,
    space: &GroupSpace,
    context: &Context<'_>,
    origin: PlacedOrigin,
    out: &mut Vec<PlacedShape>,
) {
    let Some(transform) = table.transform.as_ref() else {
        return;
    };
    let frame_rect = placed_rect(transform, space);
    let column_widths: Vec<f64> = table
        .column_widths_emu
        .iter()
        .map(|width| emu_to_px_f((*width).max(0) as f64 * space.scale.0))
        .collect();
    if column_widths.is_empty() {
        return;
    }
    let mut text_context = text_context_for_table(context);
    let table_style = resolve_table_style(&table.style, context.inherited.table_styles);
    let styled = |row: usize, column: usize, cell: &TableCell| -> CellStyle {
        table_style
            .as_ref()
            .map(|style| {
                cell_style(
                    style,
                    &table.style,
                    CellPosition {
                        row,
                        column,
                        row_span: (cell.row_span.max(1) as usize).min(table.rows.len() - row),
                        column_span: (cell.grid_span.max(1) as usize)
                            .min(column_widths.len() - column),
                        rows: table.rows.len(),
                        columns: column_widths.len(),
                    },
                )
            })
            .unwrap_or_default()
    };

    // Pass one: flow every drawn cell's text from the top so row heights
    // can grow to fit.
    let mut flowed: Vec<FlowedCell<'_>> = Vec::new();
    let mut row_heights: Vec<f64> = table
        .rows
        .iter()
        .map(|row| emu_to_px_f(row.height_emu.max(0) as f64 * space.scale.1))
        .collect();
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut column = 0usize;
        for cell in &row.cells {
            if column >= column_widths.len() {
                break;
            }
            let span = (cell.grid_span.max(1) as usize).min(column_widths.len() - column);
            if !(cell.h_merge || cell.v_merge) {
                let width: f64 = column_widths[column..column + span].iter().sum();
                text_context.cell_defaults = styled(row_index, column, cell).text;
                let (lines, content_bottom, frame) = match &cell.text {
                    Some(text) => {
                        let frame = TextFrame::resolve(
                            &cell.insets,
                            cell.anchor.or(text.anchor),
                            &text.insets,
                            None,
                        );
                        let (lines, bottom) = flow_text(text, width, &frame, &text_context);
                        (lines, bottom, frame)
                    }
                    None => (
                        Vec::new(),
                        0.0,
                        TextFrame::resolve(
                            &cell.insets,
                            cell.anchor.or(Some(TextAnchor::Top)),
                            &Default::default(),
                            None,
                        ),
                    ),
                };
                // Only a single-row cell asks its row to grow; a spanning
                // cell has the rows below to draw into.
                if cell.row_span <= 1 && !lines.is_empty() {
                    let needed = content_bottom + frame.inset_bottom;
                    if needed.is_finite() && needed > row_heights[row_index] {
                        row_heights[row_index] = needed;
                    }
                }
                flowed.push(FlowedCell {
                    cell,
                    row: row_index,
                    column,
                    lines,
                    content_bottom,
                    frame,
                });
            }
            column += span;
        }
    }

    let mut column_lefts = Vec::with_capacity(column_widths.len() + 1);
    let mut x = frame_rect.x;
    for width in &column_widths {
        column_lefts.push(x);
        x += width;
    }
    column_lefts.push(x);
    let mut row_tops = Vec::with_capacity(row_heights.len() + 1);
    let mut y = frame_rect.y;
    for height in &row_heights {
        row_tops.push(y);
        y += height;
    }
    row_tops.push(y);

    // Pass two: cells, then borders.
    let mut borders = Vec::new();
    for mut placed in flowed {
        let span = (placed.cell.grid_span.max(1) as usize).min(column_widths.len() - placed.column);
        let row_span = (placed.cell.row_span.max(1) as usize).min(row_heights.len() - placed.row);
        let rect = Rect {
            x: column_lefts[placed.column],
            y: row_tops[placed.row],
            width: column_lefts[placed.column + span] - column_lefts[placed.column],
            height: row_tops[placed.row + row_span] - row_tops[placed.row],
        };
        anchor_lines(
            &mut placed.lines,
            &placed.frame,
            rect.height,
            placed.content_bottom,
        );
        let name = format!(
            "{}!{}{}",
            table.name,
            column_letters(placed.column),
            placed.row + 1
        );
        let from_style = styled(placed.row, placed.column, placed.cell);
        for (side, outline) in [
            (
                "left",
                placed
                    .cell
                    .borders
                    .left
                    .as_ref()
                    .or(from_style.left.as_ref()),
            ),
            (
                "right",
                placed
                    .cell
                    .borders
                    .right
                    .as_ref()
                    .or(from_style.right.as_ref()),
            ),
            (
                "top",
                placed.cell.borders.top.as_ref().or(from_style.top.as_ref()),
            ),
            (
                "bottom",
                placed
                    .cell
                    .borders
                    .bottom
                    .as_ref()
                    .or(from_style.bottom.as_ref()),
            ),
            ("diagonal_down", placed.cell.borders.diagonal_down.as_ref()),
            ("diagonal_up", placed.cell.borders.diagonal_up.as_ref()),
        ] {
            let Some(resolved) = resolve_outline(
                outline,
                None,
                context.theme,
                context.inherited.theme_line_styles,
            ) else {
                continue;
            };
            let edge = match side {
                "left" => Rect { width: 0.0, ..rect },
                "right" => Rect {
                    x: rect.x + rect.width,
                    width: 0.0,
                    ..rect
                },
                "top" => Rect {
                    height: 0.0,
                    ..rect
                },
                // A diagonal spans the whole cell; the upward one flips.
                "diagonal_down" | "diagonal_up" => rect,
                _ => Rect {
                    y: rect.y + rect.height,
                    height: 0.0,
                    ..rect
                },
            };
            borders.push(PlacedShape {
                text_region: None,
                local_rect: None,
                group_transforms: Vec::new(),
                group_ids: Vec::new(),
                name: format!("{name}:{side}"),
                rect: edge,
                rotation_rad: 0.0,
                flip_h: false,
                flip_v: side == "diagonal_up",
                geometry: PlacedGeometry::Line,
                fill: None,
                gradient: None,
                outline: Some(resolved),
                image_part: None,
                source_rect: None,
                lines: Vec::new(),
                origin,
                source_id: table.id,
                text_rotation_deg: 0.0,
                shadow: None,
                image_alpha: None,
                pattern: None,
                effects: Default::default(),
            });
        }
        out.push(PlacedShape {
            text_region: None,
            local_rect: None,
            group_transforms: Vec::new(),
            group_ids: Vec::new(),
            name,
            rect,
            rotation_rad: 0.0,
            flip_h: false,
            flip_v: false,
            geometry: PlacedGeometry::Rect,
            fill: placed
                .cell
                .fill
                .as_ref()
                .or(from_style.fill.as_ref())
                .and_then(|fill| resolve_fill(fill, context.theme)),
            gradient: placed
                .cell
                .fill
                .as_ref()
                .or(from_style.fill.as_ref())
                .and_then(|fill| crate::resolve_gradient(fill, context.theme)),
            outline: None,
            image_part: None,
            source_rect: None,
            lines: placed.lines,
            origin,
            source_id: table.id,
            text_rotation_deg: 0.0,
            shadow: None,
            image_alpha: None,
            pattern: None,
            effects: Default::default(),
        });
    }
    out.extend(borders);
}

/// Spreadsheet-style column letters: 0 → A, 25 → Z, 26 → AA.
fn column_letters(index: usize) -> String {
    let mut letters = Vec::new();
    let mut n = index + 1;
    while n > 0 {
        let rem = (n - 1) % 26;
        letters.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect()
}
