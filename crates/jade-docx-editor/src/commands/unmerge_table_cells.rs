use super::merge_table_cells::{find_cell_boundary, TableCellSelection};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::docx_xml::XmlElement;
use crate::word_document::WordDocument;
use jade_docx_layout::selection::SelectionRange;
use std::collections::{BTreeMap, HashSet};

/// Reverses a previous merge: for every target cell position, clears the
/// `w:gridSpan` and `w:vMerge` markers on the master cell and every
/// `vMerge="continue"` follower it spans downward. The follower cells remain
/// in the table as independent empty cells (matching Word's behaviour).
pub struct UnmergeTableCellsCommand {
    table_order_index: usize,
    target_cells: Vec<TableCellSelection>,
    backup: Option<UnmergeBackup>,
    previous_selection: Option<SelectionRange>,
}

struct UnmergeBackup {
    table_order_index: usize,
    table: crate::document::Table,
    table_node_id: Option<crate::docx_xml::XmlNodeId>,
    raw_table_element: Option<XmlElement>,
}

impl UnmergeTableCellsCommand {
    pub fn new(table_order_index: usize, target_cells: Vec<TableCellSelection>) -> Self {
        Self {
            table_order_index,
            target_cells,
            backup: None,
            previous_selection: None,
        }
    }

    fn capture_backup(
        document: &WordDocument,
        table_order_index: usize,
    ) -> CommandResult<UnmergeBackup> {
        let table = document
            .table_by_order(table_order_index)
            .ok_or_else(|| CommandError::Custom("指定されたテーブルが見つかりません".into()))?;
        let table_node_id = table.node_id;
        let raw_table_element = match (document.raw_document.as_ref(), table_node_id) {
            (Some(raw), Some(table_id)) => raw.element(table_id).cloned(),
            _ => None,
        };
        Ok(UnmergeBackup {
            table_order_index,
            table: table.clone(),
            table_node_id,
            raw_table_element,
        })
    }

    fn restore_backup(document: &mut WordDocument, backup: &UnmergeBackup) -> CommandResult<()> {
        let live_table_node_id = document
            .table_by_order(backup.table_order_index)
            .and_then(|table| table.node_id);
        {
            let table = document
                .table_mut_by_order(backup.table_order_index)
                .ok_or_else(|| CommandError::Custom("テーブルが存在しません".into()))?;
            *table = backup.table.clone();
            table.node_id = live_table_node_id.or(backup.table_node_id);
        }

        if let (Some(raw), Some(table_id), Some(raw_table)) = (
            document.raw_document.as_mut(),
            live_table_node_id.or(backup.table_node_id),
            backup.raw_table_element.as_ref(),
        ) {
            let table_element = raw.element_mut(table_id).ok_or_else(|| {
                CommandError::Custom("テーブルのXMLを復元できませんでした".into())
            })?;
            table_element.attributes = raw_table.attributes.clone();
            table_element.children = raw_table.children.clone();
            raw.rebuild_index_for_subtree(table_id);
        }

        document.rebuild_paragraph_surfaces();
        Ok(())
    }

    /// Expand each requested target into the full set of (row, cell_index)
    /// positions that must have their merge markers cleared: the master cell
    /// itself plus every downstream `vMerge="continue"` follower sharing the
    /// same column range.
    fn collect_targets(
        document: &WordDocument,
        table_order_index: usize,
        target_cells: &[TableCellSelection],
    ) -> CommandResult<Vec<(usize, usize)>> {
        if target_cells.is_empty() {
            return Err(CommandError::Custom(
                "分割対象のセルが指定されていません".into(),
            ));
        }
        let table = document
            .table_by_order(table_order_index)
            .ok_or_else(|| CommandError::Custom("指定されたテーブルが見つかりません".into()))?;

        let mut collected: HashSet<(usize, usize)> = HashSet::new();
        let mut any_split = false;

        for target in target_cells {
            let row = table.rows.get(target.row).ok_or_else(|| {
                CommandError::Custom("対象セルの行がテーブルに存在しません".into())
            })?;
            let boundary = find_cell_boundary(row, target.column).ok_or_else(|| {
                CommandError::Custom("対象セルの位置がテーブルと一致しません".into())
            })?;
            let cell = row
                .cells
                .get(boundary.cell_index)
                .ok_or_else(|| CommandError::Custom("内部エラー: セルが見つかりません".into()))?;

            let has_gridspan = cell.span.is_some_and(|s| s > 1);
            let is_vmerge_master = cell.vertical_merge.as_deref() == Some("restart");
            let is_vmerge_follower = cell.vertical_merge.as_deref() == Some("continue");

            if !has_gridspan && !is_vmerge_master && !is_vmerge_follower {
                // Already unmerged — nothing to do for this target.
                continue;
            }
            any_split = true;

            // For a continue follower, walk up to find the master first.
            let (master_row, master_cell_index, master_column_start, master_column_span) =
                if is_vmerge_follower {
                    let mut found = None;
                    for up in (0..target.row).rev() {
                        if let Some(up_row) = table.rows.get(up) {
                            if let Some(up_boundary) = find_cell_boundary(up_row, boundary.start) {
                                if let Some(up_cell) = up_row.cells.get(up_boundary.cell_index) {
                                    match up_cell.vertical_merge.as_deref() {
                                        Some("continue") => continue,
                                        Some("restart") | None => {
                                            found = Some((
                                                up,
                                                up_boundary.cell_index,
                                                up_boundary.start,
                                                up_boundary.span,
                                            ));
                                            break;
                                        }
                                        _ => break,
                                    }
                                }
                            }
                        }
                        break;
                    }
                    found.unwrap_or((
                        target.row,
                        boundary.cell_index,
                        boundary.start,
                        boundary.span,
                    ))
                } else {
                    (
                        target.row,
                        boundary.cell_index,
                        boundary.start,
                        boundary.span,
                    )
                };

            collected.insert((master_row, master_cell_index));

            // Walk downwards for every continue follower sharing this column.
            for below in (master_row + 1)..table.rows.len() {
                let Some(below_row) = table.rows.get(below) else {
                    break;
                };
                let Some(below_boundary) = find_cell_boundary(below_row, master_column_start)
                else {
                    break;
                };
                if below_boundary.start != master_column_start
                    || below_boundary.span != master_column_span
                {
                    break;
                }
                let Some(below_cell) = below_row.cells.get(below_boundary.cell_index) else {
                    break;
                };
                if below_cell.vertical_merge.as_deref() != Some("continue") {
                    break;
                }
                collected.insert((below, below_boundary.cell_index));
            }
        }

        if !any_split {
            return Err(CommandError::Custom(
                "選択セルには分割可能な結合が含まれていません".into(),
            ));
        }

        let mut result: Vec<(usize, usize)> = collected.into_iter().collect();
        result.sort();
        Ok(result)
    }

    fn apply_clear(
        document: &mut WordDocument,
        table_order_index: usize,
        targets: &[(usize, usize)],
    ) -> CommandResult<()> {
        // Group targets per row so cell-index shifts caused by inserting
        // replacement cells stay isolated to a single row.
        let mut per_row: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (row_idx, cell_idx) in targets.iter() {
            per_row.entry(*row_idx).or_default().push(*cell_idx);
        }

        let has_raw = document.raw_document.is_some();
        {
            let table = document
                .table_mut_by_order(table_order_index)
                .ok_or_else(|| CommandError::Custom("テーブルが存在しません".into()))?;

            for (row_idx, mut cell_indices) in per_row.into_iter() {
                cell_indices.sort_unstable();
                cell_indices.dedup();

                let row = table.rows.get_mut(row_idx).ok_or_else(|| {
                    CommandError::Custom("対象セルの行がテーブルに存在しません".into())
                })?;

                // As we insert replacement cells after each target, later
                // cell indices in the same row need to shift by the number of
                // cells we've already inserted upstream.
                let mut offset: usize = 0;
                for original_cell_idx in cell_indices {
                    let cell_idx = original_cell_idx + offset;
                    let span = {
                        let cell = row.cells.get_mut(cell_idx).ok_or_else(|| {
                            CommandError::Custom("内部エラー: セルが見つかりません".into())
                        })?;
                        let span = cell.span.unwrap_or(1).max(1) as usize;
                        cell.span = None;
                        cell.vertical_merge = None;
                        span
                    };

                    // Horizontal merges collapsed `span - 1` siblings into the
                    // master at merge time. Restore them as independent empty
                    // cells so every row keeps covering the same column count.
                    for i in 1..span {
                        row.cells
                            .insert(cell_idx + i, WordDocument::new_empty_table_cell());
                    }
                    offset += span.saturating_sub(1);
                }
            }
        }

        if has_raw {
            document
                .rewrite_table_xml_from_model(table_order_index)
                .map_err(CommandError::Custom)?;
        }
        document
            .sync_table_cell_widths(table_order_index)
            .map_err(CommandError::Custom)?;

        document.rebuild_paragraph_surfaces();
        Ok(())
    }

    pub fn preview_plan(
        document: &WordDocument,
        table_order_index: usize,
        target_cells: &[TableCellSelection],
    ) -> CommandResult<()> {
        Self::collect_targets(document, table_order_index, target_cells).map(|_| ())
    }
}

impl Command for UnmergeTableCellsCommand {
    fn name(&self) -> &'static str {
        "unmerge_table_cells"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let targets =
            Self::collect_targets(ctx.document, self.table_order_index, &self.target_cells)?;
        let backup = Self::capture_backup(ctx.document, self.table_order_index)?;
        let previous_selection = *ctx.selection;

        match Self::apply_clear(ctx.document, self.table_order_index, &targets) {
            Ok(()) => {
                self.backup = Some(backup);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(err) => match Self::restore_backup(ctx.document, &backup) {
                Ok(()) => Err(err),
                Err(restore_err) => Err(restore_err),
            },
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("元の状態を復元できません".into()))?;
        Self::restore_backup(ctx.document, &backup)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::merge_table_cells::MergeTableCellsCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, StyleMap, Table, TableCell, TableColumn,
        TableRow,
    };
    use crate::word_document::WordDocument;

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

    fn simple_table_document() -> WordDocument {
        let table = Table {
            rows: vec![
                TableRow {
                    cells: vec![make_cell("A1"), make_cell("A2")],
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
                },
                TableRow {
                    cells: vec![make_cell("B1"), make_cell("B2")],
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
                },
            ],
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

    #[test]
    fn unmerge_restores_independent_cells() {
        let mut word_doc = simple_table_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        // First, merge the 2x2 block.
        let merge_selections = vec![
            TableCellSelection {
                row: 0,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 0,
                column: 1,
                column_span: 1,
            },
            TableCellSelection {
                row: 1,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 1,
                column: 1,
                column_span: 1,
            },
        ];
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(MergeTableCellsCommand::new(0, merge_selections)),
                    &mut ctx,
                )
                .expect("merge");
        }
        assert_eq!(
            word_doc.table_by_order(0).unwrap().rows[0].cells[0]
                .span
                .unwrap(),
            2
        );

        // Now unmerge by selecting just the master cell.
        let unmerge_targets = vec![TableCellSelection {
            row: 0,
            column: 0,
            column_span: 2,
        }];
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(UnmergeTableCellsCommand::new(0, unmerge_targets)),
                    &mut ctx,
                )
                .expect("unmerge");
        }

        let table = word_doc.table_by_order(0).expect("table");
        assert!(table.rows[0].cells[0].span.is_none());
        assert!(table.rows[0].cells[0].vertical_merge.is_none());
        // The continue row should also be cleared.
        assert!(table.rows[1].cells[0].span.is_none());
        assert!(table.rows[1].cells[0].vertical_merge.is_none());

        // Undo should restore the merged state.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo unmerge"));
        }
        let table = word_doc.table_by_order(0).expect("table");
        assert_eq!(table.rows[0].cells[0].span, Some(2));
        assert_eq!(
            table.rows[0].cells[0].vertical_merge.as_deref(),
            Some("restart")
        );
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
    }

    #[test]
    fn unmerge_errors_when_no_merged_cells_selected() {
        let word_doc = simple_table_document();
        let targets = vec![TableCellSelection {
            row: 0,
            column: 0,
            column_span: 1,
        }];
        let result = UnmergeTableCellsCommand::preview_plan(&word_doc, 0, &targets);
        assert!(result.is_err());
    }

    /// Builds a plain N×M table with every cell distinct and no spans.
    fn grid_table_document(rows: usize, cols: usize) -> WordDocument {
        let mut table_rows = Vec::with_capacity(rows);
        for r in 0..rows {
            let mut cells = Vec::with_capacity(cols);
            for c in 0..cols {
                cells.push(make_cell(&format!("r{r}c{c}")));
            }
            table_rows.push(TableRow {
                cells,
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
            });
        }

        let table = Table {
            rows: table_rows,
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

    /// Total logical columns a row covers, accounting for gridSpan and
    /// grid_before/grid_after. In a well-formed table every row shares the
    /// same total — merges and unmerges must never break that invariant.
    fn row_column_coverage(row: &TableRow) -> u32 {
        let before = row.grid_before.unwrap_or(0);
        let after = row.grid_after.unwrap_or(0);
        let cells: u32 = row.cells.iter().map(|c| c.span.unwrap_or(1).max(1)).sum();
        before + cells + after
    }

    fn assert_table_is_rectangular(word_doc: &WordDocument, expected_cols: u32) {
        let table = word_doc.table_by_order(0).expect("table");
        for (idx, row) in table.rows.iter().enumerate() {
            assert_eq!(
                row_column_coverage(row),
                expected_cols,
                "row {idx} should span {expected_cols} columns but has {} \
                 cells with spans {:?}",
                row.cells.len(),
                row.cells.iter().map(|c| c.span).collect::<Vec<_>>(),
            );
        }
    }

    fn do_merge(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        selection: &mut Option<SelectionRange>,
        r0: usize,
        c0: usize,
        r1: usize,
        c1: usize,
    ) {
        let mut selections = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                selections.push(TableCellSelection {
                    row: r,
                    column: c,
                    column_span: 1,
                });
            }
        }
        let mut ctx = CommandContext::new(word_doc, selection);
        executor
            .execute(
                Box::new(MergeTableCellsCommand::new(0, selections)),
                &mut ctx,
            )
            .expect("merge");
    }

    fn do_unmerge_master(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        selection: &mut Option<SelectionRange>,
        row: usize,
        column: usize,
    ) {
        let column_span = {
            let table = word_doc.table_by_order(0).expect("table");
            let row_ref = &table.rows[row];
            find_cell_boundary(row_ref, column)
                .map(|b| b.span)
                .unwrap_or(1)
        };
        let targets = vec![TableCellSelection {
            row,
            column,
            column_span,
        }];
        let mut ctx = CommandContext::new(word_doc, selection);
        executor
            .execute(
                Box::new(UnmergeTableCellsCommand::new(0, targets)),
                &mut ctx,
            )
            .expect("unmerge");
    }

    /// Regression: pure horizontal merge followed by unmerge must leave every
    /// row covering the same column count as before. Prior to the fix, unmerge
    /// only cleared span/vMerge flags without reinserting the cells that
    /// merge had collapsed, leaving a short row visually misaligned with the
    /// rest of the table (the failure shown in the user's screenshot).
    #[test]
    fn unmerge_of_horizontal_merge_restores_row_cell_count() {
        let mut word_doc = grid_table_document(3, 4);
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        // Merge columns 1..=2 in row 0 only (horizontal-only merge).
        do_merge(&mut word_doc, &mut executor, &mut selection, 0, 1, 0, 2);
        assert_eq!(
            word_doc.table_by_order(0).unwrap().rows[0].cells[1]
                .span
                .unwrap(),
            2
        );

        // Unmerge the same master cell.
        do_unmerge_master(&mut word_doc, &mut executor, &mut selection, 0, 1);

        assert_table_is_rectangular(&word_doc, 4);
        let table = word_doc.table_by_order(0).expect("table");
        assert_eq!(table.rows[0].cells.len(), 4);
        // The restored sibling must exist as an empty cell next to the master.
        assert!(table.rows[0].cells[1].span.is_none());
        assert!(table.rows[0].cells[2].span.is_none());
    }

    /// Regression: 2×2 merge unmerged must yield 2 independent cells per
    /// affected row, not 1. The existing `unmerge_restores_independent_cells`
    /// test only checked span/vMerge flags on the master, which is why the
    /// screenshot bug slipped through.
    #[test]
    fn unmerge_of_rectangular_merge_restores_grid_coverage() {
        let mut word_doc = grid_table_document(3, 3);
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        do_merge(&mut word_doc, &mut executor, &mut selection, 0, 0, 1, 1);
        do_unmerge_master(&mut word_doc, &mut executor, &mut selection, 0, 0);

        assert_table_is_rectangular(&word_doc, 3);
        let table = word_doc.table_by_order(0).expect("table");
        assert_eq!(table.rows[0].cells.len(), 3);
        assert_eq!(table.rows[1].cells.len(), 3);
        assert_eq!(table.rows[2].cells.len(), 3);
    }

    /// Undo after unmerge must restore the merged grid exactly, including the
    /// reduced cell counts on affected rows.
    #[test]
    fn undo_of_unmerge_restores_merged_cell_counts() {
        let mut word_doc = grid_table_document(3, 3);
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        do_merge(&mut word_doc, &mut executor, &mut selection, 0, 0, 1, 1);
        let merged_row0_len = word_doc.table_by_order(0).unwrap().rows[0].cells.len();
        let merged_row1_len = word_doc.table_by_order(0).unwrap().rows[1].cells.len();

        do_unmerge_master(&mut word_doc, &mut executor, &mut selection, 0, 0);
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo unmerge"));
        }

        let table = word_doc.table_by_order(0).expect("table");
        assert_eq!(table.rows[0].cells.len(), merged_row0_len);
        assert_eq!(table.rows[1].cells.len(), merged_row1_len);
        assert_eq!(table.rows[0].cells[0].span, Some(2));
        assert_eq!(
            table.rows[0].cells[0].vertical_merge.as_deref(),
            Some("restart")
        );
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
        // The grid invariant must hold through the roundtrip.
        assert_table_is_rectangular(&word_doc, 3);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// A merge rectangle expressed in logical grid coordinates, pre-resolution.
        /// `shrunk` merges that collapse to 1×1 are filtered out at generation time.
        #[derive(Clone, Debug)]
        struct MergeRect {
            row0: usize,
            col0: usize,
            row1: usize,
            col1: usize,
        }

        fn merge_rect_strategy(rows: usize, cols: usize) -> impl Strategy<Value = MergeRect> {
            (0..rows, 0..rows, 0..cols, 0..cols).prop_map(move |(a, b, c, d)| MergeRect {
                row0: a.min(b),
                row1: a.max(b),
                col0: c.min(d),
                col1: c.max(d),
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 64,
                max_shrink_iters: 4096,
                .. ProptestConfig::default()
            })]

            /// Invariant: after any single merge→unmerge roundtrip on a plain
            /// N×M grid, every row must still cover exactly M columns. This
            /// is the contract violated by the screenshot bug.
            #[test]
            fn merge_then_unmerge_preserves_grid_coverage(
                rows in 2usize..=5,
                cols in 2usize..=5,
                rect in merge_rect_strategy(5, 5),
                widths in prop::collection::vec(9f32..240f32, 5),
            ) {
                // Clamp the generated rectangle to the current table size.
                let r0 = rect.row0.min(rows - 1);
                let r1 = rect.row1.min(rows - 1);
                let c0 = rect.col0.min(cols - 1);
                let c1 = rect.col1.min(cols - 1);
                // Skip degenerate 1x1 merges — merge requires >= 2 cells.
                prop_assume!(!(r0 == r1 && c0 == c1));

                let mut word_doc = grid_table_document(rows, cols);
                let expected_widths: Vec<f32> = widths.into_iter().take(cols).collect();
                word_doc.table_mut_by_order(0).expect("table").columns = expected_widths
                    .iter()
                    .map(|width| TableColumn { width: Some(*width) })
                    .collect();
                let mut selection = None;
                let mut executor = CommandExecutor::new();

                do_merge(&mut word_doc, &mut executor, &mut selection, r0, c0, r1, c1);
                do_unmerge_master(&mut word_doc, &mut executor, &mut selection, r0, c0);

                let table = word_doc.table_by_order(0).expect("table");
                let actual_widths: Vec<f32> = table.columns
                    .iter()
                    .map(|column| column.width.expect("generated width"))
                    .collect();
                prop_assert_eq!(actual_widths, expected_widths);
                for (idx, row) in table.rows.iter().enumerate() {
                    prop_assert_eq!(
                        row_column_coverage(row),
                        cols as u32,
                        "row {} lost grid coverage after merge→unmerge", idx
                    );
                    // No stray merge markers should survive the unmerge.
                    for cell in row.cells.iter() {
                        prop_assert!(cell.vertical_merge.is_none());
                        prop_assert!(cell.span.is_none() || cell.span == Some(1));
                    }
                }
            }

            /// Invariant: undo after merge→unmerge must restore the exact
            /// post-merge state. Redo of the unmerge must reproduce the
            /// post-unmerge grid.
            #[test]
            fn merge_unmerge_undo_redo_is_faithful(
                rows in 2usize..=4,
                cols in 2usize..=4,
                rect in merge_rect_strategy(4, 4),
            ) {
                let r0 = rect.row0.min(rows - 1);
                let r1 = rect.row1.min(rows - 1);
                let c0 = rect.col0.min(cols - 1);
                let c1 = rect.col1.min(cols - 1);
                prop_assume!(!(r0 == r1 && c0 == c1));

                let mut word_doc = grid_table_document(rows, cols);
                let mut selection = None;
                let mut executor = CommandExecutor::new();

                do_merge(&mut word_doc, &mut executor, &mut selection, r0, c0, r1, c1);
                type RowSnapshot = (Vec<Option<u32>>, Vec<Option<String>>);
                let post_merge: Vec<RowSnapshot> = word_doc
                    .table_by_order(0).unwrap().rows.iter()
                    .map(|row| (
                        row.cells.iter().map(|c| c.span).collect(),
                        row.cells.iter().map(|c| c.vertical_merge.clone()).collect(),
                    ))
                    .collect();

                do_unmerge_master(&mut word_doc, &mut executor, &mut selection, r0, c0);
                let post_unmerge: Vec<usize> = word_doc
                    .table_by_order(0).unwrap().rows.iter()
                    .map(|row| row.cells.len())
                    .collect();

                // Undo the unmerge — table must match post-merge snapshot.
                {
                    let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                    prop_assert!(executor.undo(&mut ctx).expect("undo"));
                }
                let table = word_doc.table_by_order(0).expect("table");
                for (i, row) in table.rows.iter().enumerate() {
                    let spans: Vec<Option<u32>> = row.cells.iter().map(|c| c.span).collect();
                    let merges: Vec<Option<String>> = row.cells.iter()
                        .map(|c| c.vertical_merge.clone()).collect();
                    prop_assert_eq!(spans, post_merge[i].0.clone());
                    prop_assert_eq!(merges, post_merge[i].1.clone());
                }

                // Redo the unmerge — cell counts must match post-unmerge.
                {
                    let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                    prop_assert!(executor.redo(&mut ctx).expect("redo"));
                }
                let table = word_doc.table_by_order(0).expect("table");
                let counts: Vec<usize> = table.rows.iter().map(|r| r.cells.len()).collect();
                prop_assert_eq!(counts, post_unmerge);
                for row in table.rows.iter() {
                    prop_assert_eq!(row_column_coverage(row), cols as u32);
                }
            }
        }
    }
}
