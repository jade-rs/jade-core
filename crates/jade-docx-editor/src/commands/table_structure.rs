use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Table;
use crate::docx_xml::{XmlElement, XmlNodeId};
use jade_docx_layout::selection::SelectionRange;

/// Structural table grid edit performed by [`TableStructureCommand`].
#[derive(Debug, Clone, Copy)]
pub enum TableStructureOp {
    /// Insert an empty row before `row_index`; `rows.len()` appends below the
    /// last row, while larger indices are rejected.
    InsertRow {
        row_index: usize,
        /// Row whose formatting the new row copies (Word's 行の挿入).
        template_row: Option<usize>,
    },
    /// Delete the row at `row_index`, removing the table with its last row.
    DeleteRow { row_index: usize },
    /// Insert a grid column before `column_index`; the resolved column count
    /// appends after the last column, while larger indices are rejected.
    /// `width` is in points; `None` copies a neighbouring width when possible.
    InsertColumn {
        column_index: usize,
        width: Option<f32>,
        /// Column whose cell formatting the new cells copy.
        template_column: Option<usize>,
    },
    /// Delete the grid column at `column_index`, removing the table with its
    /// last column. Column edits are untracked even during a revision session.
    DeleteColumn { column_index: usize },
    /// Split one cell into `columns` × `rows` cells (Word's セルの分割).
    SplitCell {
        row: usize,
        column: usize,
        columns: usize,
        rows: usize,
    },
    /// Give every row the same height (Word's 行の高さを揃える). `None` averages
    /// the rows' explicit heights.
    DistributeRows { height: Option<f32> },
    /// Give every grid column the same width (Word's 列の幅を揃える). `None`
    /// shares out the grid's explicit widths.
    DistributeColumns { total_width: Option<f32> },
}

/// Word-style table layout commands (行の挿入/削除・列の挿入/削除) with undo
/// support.
///
/// The grid mutations are hard to invert edit-by-edit — vertical merges may
/// be re-anchored, `gridSpan`s widened or narrowed, and `gridBefore` shifted —
/// so undo restores full model and raw-XML snapshots taken before the
/// mutation. Keeping the raw subtree is required for cell content that is not
/// represented by the table model, such as OMML and nested tables.
pub struct TableStructureCommand {
    table_order_index: usize,
    op: TableStructureOp,
    backup: Option<TableStructureBackup>,
    whole_table_backup: Option<crate::word_document::WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

struct TableStructureBackup {
    next_revision_id: u32,
    table: Table,
    table_node_id: Option<XmlNodeId>,
    raw_table_element: Option<XmlElement>,
}

impl TableStructureCommand {
    pub fn new(table_order_index: usize, op: TableStructureOp) -> Self {
        Self {
            table_order_index,
            op,
            backup: None,
            whole_table_backup: None,
            previous_selection: None,
        }
    }

    fn capture_backup(
        ctx: &CommandContext<'_>,
        table_order_index: usize,
    ) -> CommandResult<TableStructureBackup> {
        let table = ctx
            .document
            .table_by_order(table_order_index)
            .ok_or_else(|| {
                CommandError::Custom(format!("table {table_order_index} was not found"))
            })?;
        let table_node_id = table.node_id;
        let raw_table_element = match (ctx.document.raw_document.as_ref(), table_node_id) {
            (Some(raw), Some(table_id)) => raw.element(table_id).cloned(),
            _ => None,
        };

        Ok(TableStructureBackup {
            next_revision_id: ctx.document.revision_id_counter(),
            table: table.clone(),
            table_node_id,
            raw_table_element,
        })
    }

    fn remove_whole_table(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let selection = *ctx.selection;
        if let Err(error) = super::delete_table::DeleteTableCommand::new(self.table_order_index)
            .apply_untracked(ctx)
        {
            ctx.document.restore_snapshot(snapshot);
            return Err(error);
        }
        self.whole_table_backup = Some(snapshot);
        self.previous_selection = selection;
        Ok(())
    }
}

impl Command for TableStructureCommand {
    fn name(&self) -> &'static str {
        match self.op {
            TableStructureOp::InsertRow { .. } => "insert_table_row",
            TableStructureOp::DeleteRow { .. } => "delete_table_row",
            TableStructureOp::InsertColumn { .. } => "insert_table_column",
            TableStructureOp::DeleteColumn { .. } => "delete_table_column",
            TableStructureOp::SplitCell { .. } => "split_table_cell",
            TableStructureOp::DistributeRows { .. } => "distribute_table_rows",
            TableStructureOp::DistributeColumns { .. } => "distribute_table_columns",
        }
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = Self::capture_backup(ctx, self.table_order_index)?;

        match self.op {
            TableStructureOp::InsertRow {
                row_index,
                template_row,
            } => ctx
                .document
                .insert_table_row(self.table_order_index, row_index, template_row)
                .and_then(|()| {
                    // With changes tracked the new row is an insertion.
                    match ctx
                        .document
                        .new_row_revision(jade_docx_reader::RevisionKind::Inserted)
                    {
                        Some(mark) => ctx.document.set_table_row_revision(
                            self.table_order_index,
                            row_index,
                            Some(mark),
                        ),
                        None => Ok(()),
                    }
                }),
            TableStructureOp::DeleteRow { row_index } => {
                let tracking = ctx.document.revision_session().is_some();
                let remove = !tracking
                    || ctx
                        .document
                        .track_table_row_deletion(self.table_order_index, row_index)
                        .map_err(CommandError::Custom)?;
                if remove
                    && row_index == 0
                    && ctx
                        .document
                        .table_by_order(self.table_order_index)
                        .is_some_and(|table| table.rows.len() == 1)
                {
                    return self.remove_whole_table(ctx);
                }
                if remove {
                    ctx.document
                        .delete_table_row(self.table_order_index, row_index)
                } else {
                    Ok(())
                }
            }
            TableStructureOp::InsertColumn {
                column_index,
                width,
                template_column,
            } => ctx.document.insert_table_column(
                self.table_order_index,
                column_index,
                width,
                template_column,
            ),
            TableStructureOp::DeleteColumn { column_index } => {
                if column_index == 0
                    && ctx
                        .document
                        .table_dimensions(self.table_order_index)
                        .is_some_and(|(_, columns)| columns == 1)
                {
                    return self.remove_whole_table(ctx);
                }
                ctx.document
                    .delete_table_column(self.table_order_index, column_index)
            }
            TableStructureOp::SplitCell {
                row,
                column,
                columns,
                rows,
            } => ctx
                .document
                .split_table_cell(self.table_order_index, row, column, columns, rows),
            TableStructureOp::DistributeRows { height } => ctx
                .document
                .distribute_table_rows(self.table_order_index, height),
            TableStructureOp::DistributeColumns { total_width } => ctx
                .document
                .distribute_table_columns(self.table_order_index, total_width),
        }
        .map_err(CommandError::Custom)?;

        self.backup = Some(snapshot);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(snapshot) = self.whole_table_backup.take() {
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        let snapshot = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("table structure backup is missing".into()))?;

        // A structural edit rewrites `<w:tbl>` and gives its descendants new
        // node ids. Restore the saved children into the live root so the model
        // ids in the backup again match the raw subtree.
        let current_node_id = ctx
            .document
            .table_by_order(self.table_order_index)
            .and_then(|table| table.node_id);
        {
            let table = ctx
                .document
                .table_mut_by_order(self.table_order_index)
                .ok_or_else(|| {
                    CommandError::Custom("table to restore was not found".to_string())
                })?;
            *table = snapshot.table;
            table.node_id = current_node_id.or(snapshot.table_node_id);
        }

        let restore_node_id = current_node_id.or(snapshot.table_node_id);
        let restored_raw = if let (Some(raw), Some(table_id), Some(raw_table)) = (
            ctx.document.raw_document.as_mut(),
            restore_node_id,
            snapshot.raw_table_element.as_ref(),
        ) {
            let table_element = raw.element_mut(table_id).ok_or_else(|| {
                CommandError::Custom("table XML could not be restored".to_string())
            })?;
            table_element.attributes = raw_table.attributes.clone();
            table_element.children = raw_table.children.clone();
            raw.rebuild_index_for_subtree(table_id);
            true
        } else {
            false
        };
        if !restored_raw {
            ctx.document
                .rewrite_table_xml_from_model(self.table_order_index)
                .map_err(CommandError::Custom)?;
        }
        ctx.document.rebuild_paragraph_surfaces();
        ctx.document
            .restore_revision_id_counter(snapshot.next_revision_id);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, StyleMap, Table, TableCell, TableColumn,
        TableRow,
    };
    use crate::word_document::WordDocument;
    use proptest::prelude::*;

    fn make_paragraph(text: &str) -> BodyElement {
        BodyElement::from_paragraph(Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        })
    }

    fn make_cell(text: &str) -> TableCell {
        TableCell {
            blocks: vec![make_paragraph(text)],
            style: StyleMap::default(),
            class_name: None,
            span: None,
            vertical_merge: None,
            text_direction: None,
            margins: None,
            vertical_align: None,
            node_id: None,
        }
    }

    fn make_row(texts: &[&str]) -> TableRow {
        TableRow {
            revision: None,
            cells: texts.iter().map(|text| make_cell(text)).collect(),
            style: StyleMap::default(),
            class_name: None,
            is_header: false,
            cant_split: false,
            grid_before: None,
            grid_after: None,
            width_before: None,
            width_after: None,
            cell_spacing: None,
            cell_spacing_exception: None,
            cell_margins: None,
            borders: None,
            height: None,
            height_rule: None,
            node_id: None,
        }
    }

    fn two_by_two_document() -> WordDocument {
        let table = Table {
            rows: vec![make_row(&["A1", "A2"]), make_row(&["B1", "B2"])],
            columns: Vec::new(),
            style: StyleMap::default(),
            cell_style: StyleMap::default(),
            style_name: None,
            class_name: None,
            col_band_size: None,
            row_band_size: None,
            look: Default::default(),
            resolved_style: None,
            borders: None,
            cell_margins: None,
            cell_spacing: None,
            table_layout: None,
            bidi_visual: false,
            node_id: None,
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::Table(Box::new(table))],
            }),
            ..Document::default()
        })
    }

    fn cell_text(doc: &WordDocument, row: usize, cell: usize) -> String {
        let table = doc.table_by_order(0).expect("table exists");
        table.rows[row].cells[cell]
            .blocks
            .iter()
            .filter_map(|block| match block {
                BodyElement::Paragraph(paragraph) => Some(
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()
    }

    fn vertical_merge_snapshot(doc: &WordDocument) -> Vec<Vec<Option<String>>> {
        doc.table_by_order(0)
            .expect("table exists")
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.vertical_merge.clone())
                    .collect()
            })
            .collect()
    }

    fn assert_flat_vertical_merges_are_valid(doc: &WordDocument) {
        let table = doc.table_by_order(0).expect("table exists");
        for (row_index, row) in table.rows.iter().enumerate() {
            assert_eq!(row.cells.len(), table.columns.len());
            for (column, cell) in row.cells.iter().enumerate() {
                match cell.vertical_merge.as_deref() {
                    Some("restart") => assert!(
                        table.rows.get(row_index + 1).is_some_and(|next| {
                            next.cells.get(column).is_some_and(|cell| {
                                cell.vertical_merge.as_deref() == Some("continue")
                            })
                        }),
                        "orphan restart at row {row_index}, column {column}",
                    ),
                    Some("continue") => assert!(
                        row_index.checked_sub(1).is_some_and(|previous| {
                            table.rows[previous].cells.get(column).is_some_and(|cell| {
                                matches!(
                                    cell.vertical_merge.as_deref(),
                                    Some("restart" | "continue")
                                )
                            })
                        }),
                        "orphan continuation at row {row_index}, column {column}",
                    ),
                    None => {}
                    Some(other) => panic!("unexpected vertical merge marker {other}"),
                }
            }
        }
    }

    #[test]
    fn splitting_a_cell_adds_grid_columns_and_merged_rows_like_word() {
        let mut doc = two_by_two_document();
        {
            let table = doc.table_mut_by_order(0).expect("table");
            table.columns = vec![
                crate::document::TableColumn { width: Some(90.0) },
                crate::document::TableColumn { width: Some(90.0) },
            ];
            table.rows[0].cells[0]
                .style
                .insert("background-color".to_string(), "#FFCC00".to_string());
        }
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::SplitCell {
                            row: 0,
                            column: 0,
                            columns: 3,
                            rows: 2,
                        },
                    )),
                    &mut ctx,
                )
                .expect("split cell");
        }
        let table = doc.table_by_order(0).expect("table");
        // Two new grid columns share the split cell's 90pt.
        assert_eq!(
            table
                .columns
                .iter()
                .map(|column| column.width)
                .collect::<Vec<_>>(),
            vec![Some(30.0), Some(30.0), Some(30.0), Some(90.0)]
        );
        assert_eq!(table.rows.len(), 3, "one row added below the split row");
        let split_row = &table.rows[0];
        assert_eq!(split_row.cells.len(), 4);
        assert_eq!(
            split_row.cells[0].blocks.len(),
            1,
            "the original cell keeps its content"
        );
        for cell in &split_row.cells[..3] {
            assert_eq!(cell.span, None);
            assert_eq!(
                cell.style.get("background-color").map(String::as_str),
                Some("#FFCC00"),
                "the pieces keep the cell's shading"
            );
        }
        assert_eq!(
            split_row.cells[3].vertical_merge.as_deref(),
            Some("restart"),
            "the neighbour spans the new row"
        );
        let new_row = &table.rows[1];
        assert_eq!(new_row.cells.len(), 4);
        assert!(new_row.cells[..3]
            .iter()
            .all(|cell| cell.vertical_merge.is_none()));
        assert_eq!(new_row.cells[3].vertical_merge.as_deref(), Some("continue"));
        // The other row's cell widens over the new grid columns.
        assert_eq!(table.rows[2].cells[0].span, Some(3));
        assert_eq!(table.rows[2].cells[1].span, None);

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo split");
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);
    }

    #[test]
    fn splitting_a_spanned_cell_evenly_keeps_the_grid() {
        let mut doc = two_by_two_document();
        {
            let table = doc.table_mut_by_order(0).expect("table");
            table.columns = vec![
                crate::document::TableColumn { width: Some(90.0) },
                crate::document::TableColumn { width: Some(90.0) },
            ];
            table.rows[0].cells.truncate(1);
            table.rows[0].cells[0].span = Some(2);
        }
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        TableStructureCommand::new(
            0,
            TableStructureOp::SplitCell {
                row: 0,
                column: 0,
                columns: 2,
                rows: 1,
            },
        )
        .apply(&mut ctx)
        .expect("split spanned cell");
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.columns.len(), 2, "the grid already has the columns");
        assert_eq!(table.rows[0].cells.len(), 2);
        assert!(table.rows[0].cells.iter().all(|cell| cell.span.is_none()));
    }

    #[test]
    fn distributing_rows_and_columns_evens_them_out_like_word() {
        let mut doc = two_by_two_document();
        {
            let table = doc.table_mut_by_order(0).expect("table");
            table.columns = vec![
                crate::document::TableColumn { width: Some(150.0) },
                crate::document::TableColumn { width: Some(50.0) },
            ];
            table.rows[0].height = Some(20.0);
            table.rows[0].height_rule = Some(crate::document::TableRowHeightRule::Exact);
            table.rows[1].height = Some(60.0);
        }
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::DistributeColumns { total_width: None },
                    )),
                    &mut ctx,
                )
                .expect("distribute columns");
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::DistributeRows { height: None },
                    )),
                    &mut ctx,
                )
                .expect("distribute rows");
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(
            table
                .columns
                .iter()
                .map(|column| column.width)
                .collect::<Vec<_>>(),
            vec![Some(100.0), Some(100.0)],
            "the total width is shared evenly"
        );
        for row in &table.rows {
            assert_eq!(row.height, Some(40.0), "the average height is used");
            assert_eq!(
                row.height_rule,
                Some(crate::document::TableRowHeightRule::AtLeast),
                "rows stay free to grow with their content"
            );
        }
        // The grid and `w:trHeight` serialisation is covered by the
        // `set_table_column_widths` / `sync_table_row_heights` tests.
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo rows");
            executor.undo(&mut ctx).expect("undo columns");
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.columns[0].width, Some(150.0));
        assert_eq!(table.rows[1].height, Some(60.0));

        let mut doc = two_by_two_document();
        {
            let table = doc.table_mut_by_order(0).expect("table");
            table.columns = vec![
                crate::document::TableColumn { width: None },
                crate::document::TableColumn { width: Some(50.0) },
            ];
            table.rows[0].height = None;
        }
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(
            TableStructureCommand::new(
                0,
                TableStructureOp::DistributeColumns { total_width: None }
            )
            .apply(&mut ctx)
            .is_err(),
            "unknown widths need an explicit total"
        );
        assert!(
            TableStructureCommand::new(0, TableStructureOp::DistributeRows { height: None })
                .apply(&mut ctx)
                .is_err(),
            "unknown heights need an explicit height"
        );
    }

    #[test]
    fn inserted_rows_and_columns_copy_the_template_formatting() {
        let mut doc = two_by_two_document();
        {
            let table = doc.table_mut_by_order(0).expect("table");
            let row = &mut table.rows[0];
            row.height = Some(40.0);
            row.height_rule = Some(crate::document::TableRowHeightRule::Exact);
            for cell in &mut row.cells {
                cell.style
                    .insert("background-color".to_string(), "#FFCC00".to_string());
                cell.vertical_align = Some("center".to_string());
            }
            if let Some(BodyElement::Paragraph(paragraph)) = row.cells[0].blocks.first_mut() {
                paragraph.properties = Some(crate::document::ParagraphProperties {
                    alignment: Some(std::rc::Rc::<str>::from("center")),
                    ..Default::default()
                });
            }
        }
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::InsertRow {
                            row_index: 1,
                            template_row: Some(0),
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert row below");
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.rows.len(), 3);
        let inserted = &table.rows[1];
        assert_eq!(inserted.height, Some(40.0));
        assert_eq!(
            inserted.height_rule,
            Some(crate::document::TableRowHeightRule::Exact)
        );
        assert_eq!(
            inserted.cells[0]
                .style
                .get("background-color")
                .map(String::as_str),
            Some("#FFCC00")
        );
        assert_eq!(inserted.cells[1].vertical_align.as_deref(), Some("center"));
        assert!(matches!(
            inserted.cells[0].blocks.first(),
            Some(BodyElement::Paragraph(paragraph))
                if paragraph.properties.as_ref().and_then(|props| props.alignment.as_deref()) == Some("center")
                    && paragraph.runs.is_empty()
        ));
        assert_eq!(cell_text(&doc, 1, 0), "", "the copied row is empty");

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::InsertColumn {
                            column_index: 1,
                            width: None,
                            template_column: Some(0),
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert column right of column 0");
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.rows[0].cells.len(), 3);
        assert_eq!(
            table.rows[0].cells[1]
                .style
                .get("background-color")
                .map(String::as_str),
            Some("#FFCC00"),
            "the new cell copies the caret column's shading"
        );
        assert!(!table.rows[0].cells[1].style.contains_key("width"));
        assert!(
            !table.rows[2].cells[1]
                .style
                .contains_key("background-color"),
            "rows without shading stay plain"
        );

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo column"));
            assert!(executor.undo(&mut ctx).expect("undo row"));
        }
        let table = doc.table_by_order(0).expect("table");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);
    }

    #[test]
    fn tracked_row_redo_reuses_revision_ids() {
        for op in [
            TableStructureOp::InsertRow {
                row_index: 1,
                template_row: None,
            },
            TableStructureOp::DeleteRow { row_index: 0 },
        ] {
            let mut document = two_by_two_document();
            document.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-10-25T00:00:00Z".into()),
                date_utc: None,
            }));
            let counter = document.revision_id_counter();
            let mut selection = None;
            let mut command = TableStructureCommand::new(0, op);
            command
                .apply(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            let after = document.table_by_order(0).unwrap().rows
                [usize::from(matches!(op, TableStructureOp::InsertRow { .. }))]
            .revision
            .clone();
            assert!(after.is_some());
            for _ in 0..2 {
                command
                    .undo(&mut CommandContext::new(&mut document, &mut selection))
                    .unwrap();
                assert_eq!(document.revision_id_counter(), counter);
                command
                    .apply(&mut CommandContext::new(&mut document, &mut selection))
                    .unwrap();
                assert_eq!(
                    document.table_by_order(0).unwrap().rows
                        [usize::from(matches!(op, TableStructureOp::InsertRow { .. }))]
                    .revision,
                    after
                );
            }
        }
    }

    #[test]
    fn insert_row_supports_undo_and_redo() {
        let mut word_doc = two_by_two_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::InsertRow {
                            row_index: 1,
                            template_row: None,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert row");
        }
        assert_eq!(word_doc.table_dimensions(0), Some((3, 2)));
        // The inserted middle row is empty; the original rows keep their text.
        assert_eq!(cell_text(&word_doc, 0, 0), "A1");
        assert_eq!(cell_text(&word_doc, 1, 0), "");
        assert_eq!(cell_text(&word_doc, 2, 0), "B1");

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo insert row"));
        }
        assert_eq!(word_doc.table_dimensions(0), Some((2, 2)));
        assert_eq!(cell_text(&word_doc, 1, 0), "B1");

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo insert row"));
        }
        assert_eq!(word_doc.table_dimensions(0), Some((3, 2)));
    }

    #[test]
    fn delete_column_undo_restores_cell_contents() {
        let mut word_doc = two_by_two_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::DeleteColumn { column_index: 0 },
                    )),
                    &mut ctx,
                )
                .expect("delete column");
        }
        assert_eq!(word_doc.table_dimensions(0), Some((2, 1)));
        assert_eq!(cell_text(&word_doc, 0, 0), "A2");
        assert_eq!(cell_text(&word_doc, 1, 0), "B2");

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo delete column"));
        }
        assert_eq!(word_doc.table_dimensions(0), Some((2, 2)));
        assert_eq!(cell_text(&word_doc, 0, 0), "A1");
        assert_eq!(cell_text(&word_doc, 0, 1), "A2");
        assert_eq!(cell_text(&word_doc, 1, 0), "B1");
        assert_eq!(cell_text(&word_doc, 1, 1), "B2");
    }

    #[test]
    fn insert_column_with_copied_width_preserves_reference_width() {
        let mut word_doc = two_by_two_document();
        {
            let table = word_doc.table_mut_by_order(0).expect("table exists");
            table.columns = vec![
                TableColumn { width: Some(100.0) },
                TableColumn { width: Some(200.0) },
            ];
        }
        // The caret-column width is what the wasm layer copies into the
        // insert command (Word's 左/右に列を挿入 semantics).
        assert_eq!(word_doc.column_insert_width(0, 0), Some(100.0));
        assert_eq!(word_doc.column_insert_width(0, 1), Some(200.0));

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TableStructureCommand::new(
                        0,
                        TableStructureOp::InsertColumn {
                            column_index: 1,
                            width: Some(100.0),
                            template_column: None,
                        },
                    )),
                    &mut ctx,
                )
                .expect("insert column with copied width");
        }
        let grid_widths = |doc: &WordDocument| -> Vec<Option<f32>> {
            doc.table_by_order(0)
                .expect("table exists")
                .columns
                .iter()
                .map(|column| column.width)
                .collect()
        };
        assert_eq!(
            grid_widths(&word_doc),
            vec![Some(100.0), Some(100.0), Some(200.0)]
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo insert column"));
        }
        assert_eq!(grid_widths(&word_doc), vec![Some(100.0), Some(200.0)]);
    }

    #[test]
    fn delete_last_remaining_row_removes_table_and_supports_undo() {
        let table = Table {
            rows: vec![make_row(&["only"])],
            columns: Vec::new(),
            style: StyleMap::default(),
            cell_style: StyleMap::default(),
            style_name: None,
            class_name: None,
            col_band_size: None,
            row_band_size: None,
            look: Default::default(),
            resolved_style: None,
            borders: None,
            cell_margins: None,
            cell_spacing: None,
            table_layout: None,
            bidi_visual: false,
            node_id: None,
        };
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::Table(Box::new(table))],
            }),
            ..Document::default()
        });
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            let result = executor.execute(
                Box::new(TableStructureCommand::new(
                    0,
                    TableStructureOp::DeleteRow { row_index: 0 },
                )),
                &mut ctx,
            );
            result.expect("deleting the only row removes the table");
        }
        assert_eq!(word_doc.table_dimensions(0), None);
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo table removal"));
        }
        assert_eq!(word_doc.table_dimensions(0), Some((1, 1)));
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo table removal"));
        }
        assert_eq!(word_doc.table_dimensions(0), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// Insert/delete at every position around an arbitrary vertical merge.
        /// The edit must not leave an orphan marker, and undo/redo must preserve
        /// both the chain and the explicit grid widths.
        #[test]
        fn row_edits_preserve_vertical_merge_chains_and_undo(
            (rows, merge_start, merge_len, insert, at) in
                (2usize..8, any::<bool>())
                    .prop_flat_map(|(rows, insert)| (Just(rows), 0usize..rows - 1, Just(insert)))
                    .prop_flat_map(|(rows, merge_start, insert)| (
                        Just(rows), Just(merge_start), 2usize..=rows - merge_start,
                        Just(insert), 0usize..rows + usize::from(insert),
                    )),
        ) {

            let row_values: Vec<TableRow> = (0..rows)
                .map(|row| make_row(&[&format!("{row}:0"), &format!("{row}:1")]))
                .collect();
            let mut word_doc = two_by_two_document();
            {
                let table = word_doc.table_mut_by_order(0).expect("table exists");
                table.rows = row_values;
                table.columns = vec![
                    TableColumn { width: Some(37.0) },
                    TableColumn { width: Some(83.0) },
                ];
                table.rows[merge_start].cells[0].vertical_merge = Some("restart".into());
                for row in merge_start + 1..merge_start + merge_len {
                    table.rows[row].cells[0].vertical_merge = Some("continue".into());
                }
            }
            let before_markers = vertical_merge_snapshot(&word_doc);
            let before_widths: Vec<Option<f32>> = word_doc
                .table_by_order(0)
                .expect("table exists")
                .columns
                .iter()
                .map(|column| column.width)
                .collect();

            let op = if insert {
                TableStructureOp::InsertRow { row_index: at, template_row: None }
            } else {
                TableStructureOp::DeleteRow { row_index: at }
            };
            let mut selection = None;
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(Box::new(TableStructureCommand::new(0, op)), &mut ctx)
                    .expect("row structure edit");
            }
            assert_flat_vertical_merges_are_valid(&word_doc);
            prop_assert_eq!(
                word_doc.table_by_order(0).expect("table exists").columns.iter()
                    .map(|column| column.width).collect::<Vec<_>>(),
                before_widths.clone(),
            );
            let after_markers = vertical_merge_snapshot(&word_doc);

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo row edit"));
            }
            prop_assert_eq!(vertical_merge_snapshot(&word_doc), before_markers);
            prop_assert_eq!(
                word_doc.table_by_order(0).expect("table exists").columns.iter()
                    .map(|column| column.width).collect::<Vec<_>>(),
                before_widths,
            );

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo row edit"));
            }
            assert_flat_vertical_merges_are_valid(&word_doc);
            prop_assert_eq!(vertical_merge_snapshot(&word_doc), after_markers);
        }
    }
}
