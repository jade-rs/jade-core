use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{
    resolve_cell_margin_edge_points, Table, TableRowHeightRule, DEFAULT_VERTICAL_CELL_MARGIN_PT,
};
use crate::word_document::whole_twips;

const MIN_TABLE_ROW_HEIGHT_PT: f32 = 9.0;
const MIN_EXACT_ROW_HEIGHT_PT: f32 = 1.0 / 20.0;

fn largest_effective_bottom_margin(table: &Table, row_index: usize) -> Option<f32> {
    let row = table.rows.get(row_index)?;
    Some(row.cells.iter().fold(0.0_f32, |largest, cell| {
        let bottom = resolve_cell_margin_edge_points(
            cell.margins.as_ref(),
            row.cell_margins.as_ref(),
            table.effective_cell_margins(),
            |margins| margins.bottom,
        )
        .unwrap_or(DEFAULT_VERTICAL_CELL_MARGIN_PT);
        largest.max(bottom)
    }))
}

/// Converts a persisted row height back to the border-to-border value used by
/// the resize UI. This is the inverse of [`exact_height_for_resize`] for exact
/// rows, including Word's effective bottom cell margin.
// This command module is also path-included by jade-docx-document-core, whose API
// does not expose caret context and therefore does not call this read helper.
#[allow(dead_code)]
pub(crate) fn visual_height_for_resize(table: &Table, row_index: usize) -> Option<f32> {
    let row = table.rows.get(row_index)?;
    let height = row.height?;
    if row.height_rule == Some(TableRowHeightRule::Exact) {
        Some(height + largest_effective_bottom_margin(table, row_index)?)
    } else {
        Some(height)
    }
}

/// Converts the border-to-border height reported by the resize UI into the
/// exact `w:trHeight` value Word persists. Word adds the largest effective
/// bottom cell margin after applying an exact height, so retaining the visual
/// value verbatim would make the row grow by that margin on the next layout.
fn exact_height_for_resize(table: &Table, row_index: usize, visual_height: f32) -> Option<f32> {
    let largest_bottom_margin = largest_effective_bottom_margin(table, row_index)?;
    Some((visual_height - largest_bottom_margin).max(MIN_EXACT_ROW_HEIGHT_PT))
}

enum ResizeTableRowKind {
    BetweenRows,
    TableBottomEdge,
}

pub struct ResizeTableRowCommand {
    table_order_index: usize,
    row_index: usize,
    kind: ResizeTableRowKind,
    new_top_height: f32,
    new_bottom_height: Option<f32>,
    previous_top_height: Option<f32>,
    previous_top_rule: Option<TableRowHeightRule>,
    previous_bottom_height: Option<f32>,
    previous_bottom_rule: Option<TableRowHeightRule>,
}

impl ResizeTableRowCommand {
    pub fn between_rows(
        table_order_index: usize,
        row_index: usize,
        new_top_height: f32,
        new_bottom_height: f32,
    ) -> Self {
        Self {
            table_order_index,
            row_index,
            kind: ResizeTableRowKind::BetweenRows,
            new_top_height,
            new_bottom_height: Some(new_bottom_height),
            previous_top_height: None,
            previous_top_rule: None,
            previous_bottom_height: None,
            previous_bottom_rule: None,
        }
    }

    pub fn table_bottom_edge(table_order_index: usize, row_index: usize, new_height: f32) -> Self {
        Self {
            table_order_index,
            row_index,
            kind: ResizeTableRowKind::TableBottomEdge,
            new_top_height: new_height,
            new_bottom_height: None,
            previous_top_height: None,
            previous_top_rule: None,
            previous_bottom_height: None,
            previous_bottom_rule: None,
        }
    }

    fn normalize_height(height: f32) -> CommandResult<f32> {
        if !height.is_finite() {
            return Err(CommandError::Custom(
                "table row height must be finite".into(),
            ));
        }
        Ok(height.max(MIN_TABLE_ROW_HEIGHT_PT))
    }
}

impl Command for ResizeTableRowCommand {
    fn name(&self) -> &'static str {
        "resize_table_row"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let new_top_height = Self::normalize_height(self.new_top_height)?;
        let new_bottom_height = match self.new_bottom_height {
            Some(height) => Some(Self::normalize_height(height)?),
            None => None,
        };

        let (table_node_id, snapshot) = {
            let table = ctx
                .document
                .table_mut_by_order(self.table_order_index)
                .ok_or_else(|| CommandError::Custom("target table not found".into()))?;

            // `w:trHeight` holds whole twips; a dragged height rarely is.
            let new_top_height = exact_height_for_resize(table, self.row_index, new_top_height)
                .map(whole_twips)
                .ok_or_else(|| CommandError::Custom("target row not found".into()))?;
            let new_bottom_height = match new_bottom_height {
                Some(height) => Some(
                    exact_height_for_resize(table, self.row_index + 1, height)
                        .map(whole_twips)
                        .ok_or_else(|| CommandError::Custom("adjacent row missing".into()))?,
                ),
                None => None,
            };

            let top_row = table
                .rows
                .get_mut(self.row_index)
                .ok_or_else(|| CommandError::Custom("target row not found".into()))?;
            self.previous_top_height = top_row.height;
            self.previous_top_rule = top_row.height_rule;
            top_row.height = Some(new_top_height);
            top_row.height_rule = Some(TableRowHeightRule::Exact);

            self.previous_bottom_height = None;
            self.previous_bottom_rule = None;
            if let (ResizeTableRowKind::BetweenRows, Some(new_bottom)) =
                (&self.kind, new_bottom_height)
            {
                let next_row = table
                    .rows
                    .get_mut(self.row_index + 1)
                    .ok_or_else(|| CommandError::Custom("adjacent row missing".into()))?;
                self.previous_bottom_height = next_row.height;
                self.previous_bottom_rule = next_row.height_rule;
                next_row.height = Some(new_bottom);
                next_row.height_rule = Some(TableRowHeightRule::Exact);
            }

            (table.node_id, table.rows.clone())
        };

        ctx.document
            .sync_table_row_heights(table_node_id, &snapshot)
            .map_err(|err| CommandError::Custom(err.to_string()))?;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (table_node_id, snapshot) = {
            let table = ctx
                .document
                .table_mut_by_order(self.table_order_index)
                .ok_or_else(|| CommandError::Custom("target table not found".into()))?;
            let node_id = table.node_id;

            if let Some(row) = table.rows.get_mut(self.row_index) {
                row.height = self.previous_top_height;
                row.height_rule = self.previous_top_rule;
            }

            if let ResizeTableRowKind::BetweenRows = self.kind {
                if let Some(row) = table.rows.get_mut(self.row_index + 1) {
                    row.height = self.previous_bottom_height;
                    row.height_rule = self.previous_bottom_rule;
                }
            }

            (node_id, table.rows.clone())
        };

        ctx.document
            .sync_table_row_heights(table_node_id, &snapshot)
            .map_err(|err| CommandError::Custom(err.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Table, TableCell, TableCellMargins, TableRow, TableStyleDisplay};
    use proptest::prelude::*;

    #[test]
    fn exact_height_for_resize_subtracts_largest_resolved_bottom_margin() {
        // Word treats an exact `w:trHeight` as the row content-box height and
        // then adds the largest effective bottom cell margin. The resize UI,
        // however, reports the visual border-to-border height. Persisting that
        // visual value unchanged would therefore move the border a second time
        // when the document is rendered again.
        let table = Table {
            rows: vec![TableRow {
                cells: vec![
                    TableCell {
                        margins: Some(TableCellMargins {
                            bottom: Some(6.0),
                            ..TableCellMargins::default()
                        }),
                        ..TableCell::default()
                    },
                    TableCell {
                        margins: Some(TableCellMargins {
                            bottom: Some(12.0),
                            ..TableCellMargins::default()
                        }),
                        ..TableCell::default()
                    },
                ],
                ..TableRow::default()
            }],
            ..Table::default()
        };

        assert_eq!(exact_height_for_resize(&table, 0, 36.0), Some(24.0));
    }

    #[test]
    fn visual_height_for_resize_adds_exact_rows_bottom_margin_only_once() {
        let table = Table {
            cell_margins: Some(TableCellMargins {
                bottom: Some(7.0),
                ..TableCellMargins::default()
            }),
            rows: vec![
                TableRow {
                    cells: vec![TableCell::default()],
                    height: Some(29.0),
                    height_rule: Some(TableRowHeightRule::Exact),
                    ..TableRow::default()
                },
                TableRow {
                    cells: vec![TableCell::default()],
                    height: Some(29.0),
                    height_rule: Some(TableRowHeightRule::AtLeast),
                    ..TableRow::default()
                },
            ],
            ..Table::default()
        };

        assert_eq!(visual_height_for_resize(&table, 0), Some(36.0));
        assert_eq!(visual_height_for_resize(&table, 1), Some(29.0));
    }

    proptest! {
        #[test]
        fn styled_margin_resize_roundtrip_preserves_visual_height(
            bottom_margin in 0.0_f32..24.0,
            visual_height in 24.0_f32..240.0,
        ) {
            let table = Table {
                resolved_style: Some(TableStyleDisplay {
                    cell_margins: Some(TableCellMargins {
                        bottom: Some(bottom_margin),
                        ..TableCellMargins::default()
                    }),
                    ..TableStyleDisplay::default()
                }),
                rows: vec![TableRow {
                    cells: vec![TableCell::default()],
                    ..TableRow::default()
                }],
                ..Table::default()
            };

            let persisted = exact_height_for_resize(&table, 0, visual_height)
                .expect("row exists");
            let expected = (visual_height - bottom_margin).max(MIN_EXACT_ROW_HEIGHT_PT);
            prop_assert!((persisted - expected).abs() < 0.001);

            if visual_height - bottom_margin >= MIN_EXACT_ROW_HEIGHT_PT {
                let mut exact_table = table;
                exact_table.rows[0].height = Some(persisted);
                exact_table.rows[0].height_rule = Some(TableRowHeightRule::Exact);
                let restored = visual_height_for_resize(&exact_table, 0)
                    .expect("exact row has a visual height");
                prop_assert!((restored - visual_height).abs() < 0.001);
            }
        }
    }
}
