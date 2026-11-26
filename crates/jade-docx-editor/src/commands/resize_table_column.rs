use super::table_structure::TableStructureBackup;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{TableColumn, TableLayoutMode};
use crate::word_document::{whole_twips, WordDocument, MIN_TABLE_COLUMN_WIDTH_PT};
use std::rc::Rc;

/// Measured widths of a nested table within its parent's AutoFit grid.
#[derive(Debug)]
pub struct NestedColumnResizeGeometry {
    pub(crate) displayed_widths: Vec<f32>,
    pub(crate) minimum_widths: Vec<f64>,
    pub(crate) frame_width: f32,
}

pub(crate) fn fit_nested_columns_to_content(
    widths: &mut [f64],
    preferred: &[f64],
    floors: &[f64],
    twips_per_unit: f64,
) {
    let measured_budget: f64 = widths.iter().sum();
    let preferred_total: f64 = preferred.iter().sum();
    let nearest_twip = |value: f64| (value * twips_per_unit).round() / twips_per_unit;
    let budget = nearest_twip(measured_budget);
    let grown_total: f64 = preferred
        .iter()
        .zip(floors)
        .map(|(preferred, floor)| preferred.max(*floor))
        .sum();
    if budget >= grown_total - 1e-8 && grown_total > preferred_total + 1e-8 {
        // A column whose word exceeds its cell preference grows from that
        // minimum when the enclosing cell still has spare width.
        for ((width, preferred), floor) in widths.iter_mut().zip(preferred).zip(floors) {
            *width = preferred.max(*floor) * measured_budget / grown_total;
        }
    }
    if budget >= preferred_total - 1e-8
        && widths
            .iter()
            .zip(floors)
            .all(|(width, floor)| width + 1e-8 >= *floor)
    {
        // Snap each boundary to the nearest twip. For an exact half, the
        // trailing column receives the remainder.
        let mut ideal_boundary = 0.0;
        let mut previous_boundary = 0.0;
        let last = widths.len() - 1;
        // A stored percentage can describe a fractional twip even when
        // Word's resolved grid has the same width as its cell preferences.
        let scale = budget / measured_budget;
        for (index, width) in widths.iter_mut().enumerate() {
            ideal_boundary += *width * scale;
            let boundary = if index == last {
                budget
            } else {
                ((ideal_boundary * twips_per_unit - 0.5 - 1e-8).ceil() / twips_per_unit)
                    .max(previous_boundary + floors[index])
            };
            *width = boundary - previous_boundary;
            previous_boundary = boundary;
        }
        return;
    }
    let floor_total: f64 = floors.iter().sum();
    if budget <= floor_total {
        widths.copy_from_slice(floors);
        return;
    }
    // Each column takes the same fraction of the distance from its
    // unbreakable text width to its authored preference.
    let slack: f64 = preferred
        .iter()
        .zip(floors)
        .map(|(preferred, floor)| (preferred - floor).max(0.0))
        .sum();
    let mut ideal_boundary = 0.0;
    let mut previous_boundary = 0.0;
    let last = widths.len() - 1;
    for (index, width) in widths.iter_mut().enumerate() {
        ideal_boundary += floors[index]
            + (budget - floor_total)
                * if slack > f64::EPSILON {
                    (preferred[index] - floors[index]).max(0.0) / slack
                } else {
                    preferred[index] / preferred_total
                };
        let boundary = if index == last {
            budget
        } else {
            nearest_twip(ideal_boundary)
        };
        *width = boundary - previous_boundary;
        previous_boundary = boundary;
    }
}

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
    previous_rendered_width: Option<f32>,
    nested_geometry: Option<Rc<NestedColumnResizeGeometry>>,
    nested_backup: Option<TableStructureBackup>,
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
            previous_rendered_width: None,
            nested_geometry: None,
            nested_backup: None,
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
            previous_rendered_width: None,
            nested_geometry: None,
            nested_backup: None,
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
            previous_rendered_width: None,
            nested_geometry: None,
            nested_backup: None,
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

    pub fn with_previous_rendered_width(mut self, width_points: f32) -> Self {
        self.previous_rendered_width = Some(width_points);
        self
    }

    pub fn with_nested_geometry(mut self, geometry: Rc<NestedColumnResizeGeometry>) -> Self {
        self.nested_geometry = Some(geometry);
        self
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

        let order = self.table_order_index;
        let geometry = self.nested_geometry.as_ref().filter(|geometry| {
            ctx.document.table_by_order(order).is_some_and(|table| {
                WordDocument::resolved_table_column_count(table) == geometry.displayed_widths.len()
            })
        });
        let sizing = geometry
            .map(|geometry| (geometry.displayed_widths.clone(), geometry.frame_width))
            .or_else(|| ctx.document.nested_resize_column_grid(order));
        let backup = sizing
            .as_ref()
            .map(|_| TableStructureBackup::capture(ctx.document, order))
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

            if backup.is_none() {
                self.previous_columns = Some(table.columns.clone());
                self.previous_layout = Some(table.table_layout);
            }

            if let Some(widths) = normalized_replacement {
                table.columns = widths
                    .into_iter()
                    .map(|width| TableColumn { width: Some(width) })
                    .collect();
            } else {
                if let Some((widths, _)) = &sizing {
                    table.columns = widths
                        .iter()
                        .map(|width| TableColumn {
                            width: Some(whole_twips(*width)),
                        })
                        .collect();
                }
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
            // Word preserves the layout mode when the enclosing cell gives
            // this nested grid a resolved frame. The other resize paths rely
            // on fixed layout to prevent content-based redistribution.
            if sizing.is_none() {
                table.table_layout = Some(TableLayoutMode::Fixed);
            }

            (table.node_id, table.columns.clone())
        };

        // Word refreshes a stale saved grid even for a zero-distance resize,
        // but keeps the independent cell and table width preferences.
        let same_displayed_widths = sizing.as_ref().is_some_and(|(previous, _)| {
            previous.len() == columns_snapshot.len()
                && columns_snapshot
                    .iter()
                    .zip(previous)
                    .all(|(column, width)| column.width == Some(whole_twips(*width)))
        });
        let result = (|| {
            ctx.document
                .sync_table_grid(table_id, &columns_snapshot)
                .map_err(|err| CommandError::Custom(err.to_string()))?;
            if !same_displayed_widths {
                ctx.document
                    .sync_table_cell_widths(order)
                    .map_err(CommandError::Custom)?;
                if let Some((_, frame)) = sizing {
                    let previous_rendered_width =
                        self.previous_rendered_width.filter(|_| geometry.is_none());
                    ctx.document
                        .sync_resized_nested_table_width(order, frame, previous_rendered_width)
                        .map_err(CommandError::Custom)?;
                    if let Some(geometry) = geometry {
                        let preferred: Vec<_> = columns_snapshot
                            .iter()
                            .map(|column| {
                                (f64::from(column.width.unwrap_or(0.0)) * 20.0).round() / 20.0
                            })
                            .collect();
                        let mut actual = preferred.clone();
                        fit_nested_columns_to_content(
                            &mut actual,
                            &preferred,
                            &geometry.minimum_widths,
                            20.0,
                        );
                        let columns: Vec<_> = actual
                            .into_iter()
                            .map(|width| TableColumn {
                                width: Some(whole_twips(width as f32)),
                            })
                            .collect();
                        ctx.document
                            .sync_table_grid(table_id, &columns)
                            .map_err(|err| CommandError::Custom(err.to_string()))?;
                        ctx.document
                            .table_mut_by_order(order)
                            .ok_or_else(|| CommandError::Custom("target table not found".into()))?
                            .columns = columns;
                    }
                } else {
                    ctx.document
                        .sync_table_layout(table_id, Some(TableLayoutMode::Fixed))
                        .map_err(|err| CommandError::Custom(err.to_string()))?;
                }
            }
            Ok(())
        })();
        if result.is_ok() {
            self.nested_backup = backup;
        } else if let Some(backup) = backup {
            backup.restore(ctx.document, order)?;
        }
        result
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(backup) = self.nested_backup.take() {
            return backup.restore(ctx.document, self.table_order_index);
        }
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
