use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{TableColumn, TableLayoutMode};
use crate::word_document::{whole_twips, WordDocument, MIN_TABLE_COLUMN_WIDTH_PT};

pub struct ResizeTableColumnCommand {
    table_order_index: usize,
    left_column_index: usize,
    new_left_width: f32,
    right_column_index: Option<usize>,
    new_right_width: Option<f32>,
    replacement_widths: Option<Vec<f32>>,
    preserve_rendered_widths: bool,
    previous_columns: Option<Vec<TableColumn>>,
    previous_layout: Option<Option<TableLayoutMode>>,
}

impl ResizeTableColumnCommand {
    pub fn between_columns(
        table_order_index: usize,
        left_column_index: usize,
        new_left_width: f32,
        new_right_width: f32,
    ) -> Self {
        Self {
            table_order_index,
            left_column_index,
            new_left_width,
            right_column_index: Some(left_column_index + 1),
            new_right_width: Some(new_right_width),
            replacement_widths: None,
            preserve_rendered_widths: false,
            previous_columns: None,
            previous_layout: None,
        }
    }

    pub fn table_right_edge(table_order_index: usize, column_index: usize, new_width: f32) -> Self {
        Self {
            table_order_index,
            left_column_index: column_index,
            new_left_width: new_width,
            right_column_index: None,
            new_right_width: None,
            replacement_widths: None,
            preserve_rendered_widths: false,
            previous_columns: None,
            previous_layout: None,
        }
    }

    /// Replaces the complete grid, enforcing the explicit width input minimum.
    pub fn all_columns(table_order_index: usize, widths: Vec<f32>) -> Self {
        Self {
            table_order_index,
            left_column_index: 0,
            new_left_width: 0.0,
            right_column_index: None,
            new_right_width: None,
            replacement_widths: Some(widths),
            preserve_rendered_widths: false,
            previous_columns: None,
            previous_layout: None,
        }
    }

    /// Commits geometry already constrained by the pointer resize operation.
    /// Preserve reconstructed/autofit columns, including untouched columns
    /// below the input minimum and a pair whose total is below two minima.
    pub fn rendered_columns(table_order_index: usize, widths: Vec<f32>) -> Self {
        let mut command = Self::all_columns(table_order_index, widths);
        command.preserve_rendered_widths = true;
        command
    }

    fn normalize_width(width: f32) -> CommandResult<f32> {
        if !width.is_finite() {
            return Err(CommandError::Custom(
                "table column width must be finite".into(),
            ));
        }
        Ok(whole_twips(width.max(MIN_TABLE_COLUMN_WIDTH_PT)))
    }
}

impl Command for ResizeTableColumnCommand {
    fn name(&self) -> &'static str {
        "resize_table_column"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let normalized_replacement = self
            .replacement_widths
            .as_ref()
            .map(|widths| {
                if widths.is_empty() {
                    return Err(CommandError::Custom(
                        "set_table_column_widths requires at least one width".into(),
                    ));
                }
                if widths
                    .iter()
                    .any(|width| !width.is_finite() || *width < 0.0)
                {
                    return Err(CommandError::Custom(
                        "widths must be finite non-negative numbers".into(),
                    ));
                }
                // A drag's pixel widths are rarely whole twips, which is all
                // the saved grid can hold.
                if self.preserve_rendered_widths {
                    return Ok(widths.iter().copied().map(whole_twips).collect());
                }
                widths
                    .iter()
                    .map(|width| Self::normalize_width(*width))
                    .collect::<CommandResult<Vec<_>>>()
            })
            .transpose()?;
        let normalized_left = self
            .replacement_widths
            .is_none()
            .then(|| Self::normalize_width(self.new_left_width))
            .transpose()?;
        let normalized_right = self
            .new_right_width
            .map(Self::normalize_width)
            .transpose()?;

        let (table_id, columns_snapshot) = {
            let table = ctx
                .document
                .table_mut_by_order(self.table_order_index)
                .ok_or_else(|| CommandError::Custom("target table not found".into()))?;

            let column_count = WordDocument::resolved_table_column_count(table);
            if let Some(widths) = normalized_replacement.as_ref() {
                if widths.len() != column_count {
                    return Err(CommandError::Custom(format!(
                        "table {} has {column_count} columns but {} widths were given",
                        self.table_order_index,
                        widths.len()
                    )));
                }
            } else {
                if self.left_column_index >= column_count {
                    return Err(CommandError::Custom(format!(
                        "column {} is out of range; the table has {column_count} columns",
                        self.left_column_index
                    )));
                }
                if let Some(right_index) = self.right_column_index {
                    if right_index >= column_count {
                        return Err(CommandError::Custom(format!(
                            "column {right_index} is out of range; the table has {column_count} columns"
                        )));
                    }
                }
            }

            self.previous_columns = Some(table.columns.clone());
            self.previous_layout = Some(table.table_layout);

            if let Some(widths) = normalized_replacement {
                table.columns = widths
                    .into_iter()
                    .map(|width| TableColumn { width: Some(width) })
                    .collect();
            } else {
                if table.columns.len() < column_count {
                    table.columns.resize(column_count, TableColumn::default());
                }

                let new_left_width = normalized_left.expect("non-replacement width was normalized");

                if let Some(column) = table.columns.get_mut(self.left_column_index) {
                    column.width = Some(new_left_width);
                }
                if let (Some(right_index), Some(new_right)) =
                    (self.right_column_index, normalized_right)
                {
                    if let Some(column) = table.columns.get_mut(right_index) {
                        column.width = Some(new_right);
                    }
                }
            }
            // A pointer drag is an explicit grid decision. Keeping autofit
            // enabled would immediately run content-based redistribution over
            // these widths during the same mutation's repaint.
            table.table_layout = Some(TableLayoutMode::Fixed);

            (table.node_id, table.columns.clone())
        };

        ctx.document
            .sync_table_grid(table_id, &columns_snapshot)
            .map_err(|err| CommandError::Custom(err.to_string()))?;
        ctx.document
            .sync_table_cell_widths(self.table_order_index)
            .map_err(CommandError::Custom)?;
        ctx.document
            .sync_table_layout(table_id, Some(TableLayoutMode::Fixed))
            .map_err(|err| CommandError::Custom(err.to_string()))?;

        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (table_id, restored_columns, restored_layout) = {
            let table = ctx
                .document
                .table_mut_by_order(self.table_order_index)
                .ok_or_else(|| CommandError::Custom("target table not found".into()))?;

            let previous = self
                .previous_columns
                .take()
                .ok_or_else(|| CommandError::Custom("missing previous table state".into()))?;

            table.columns = previous.clone();
            let previous_layout = self
                .previous_layout
                .take()
                .ok_or_else(|| CommandError::Custom("missing previous table layout".into()))?;
            table.table_layout = previous_layout;

            (table.node_id, previous, previous_layout)
        };

        ctx.document
            .sync_table_grid(table_id, &restored_columns)
            .map_err(|err| CommandError::Custom(err.to_string()))?;
        ctx.document
            .sync_table_cell_widths(self.table_order_index)
            .map_err(CommandError::Custom)?;
        ctx.document
            .sync_table_layout(table_id, restored_layout)
            .map_err(|err| CommandError::Custom(err.to_string()))?;

        self.previous_columns = Some(restored_columns);
        self.previous_layout = Some(restored_layout);

        Ok(())
    }
}
