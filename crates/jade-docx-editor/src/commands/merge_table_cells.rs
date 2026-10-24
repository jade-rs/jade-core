use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{BodyElement, Paragraph, Table, TableCell, TableRow};
use crate::docx_xml::{XmlAttribute, XmlElement, XmlNode, XmlNodeId};
use crate::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::cmp::max;
use std::collections::{HashMap, HashSet};
use std::ptr;

/// Describes a single table cell selected by the UI.
///
/// * `row`/`column` are zero-based indices that refer to the logical grid position
///   computed during layout (before accounting for spans).
/// * `column_span` is the number of columns covered by the cell in the grid.
#[derive(Clone, Debug)]
pub struct TableCellSelection {
    pub row: usize,
    pub column: usize,
    pub column_span: usize,
}

pub struct MergeTableCellsCommand {
    table_order_index: usize,
    selected_cells: Vec<TableCellSelection>,
    backup: Option<MergeTableBackup>,
    previous_selection: Option<SelectionRange>,
}

impl MergeTableCellsCommand {
    pub fn new(table_order_index: usize, selected_cells: Vec<TableCellSelection>) -> Self {
        Self {
            table_order_index,
            selected_cells,
            backup: None,
            previous_selection: None,
        }
    }

    fn capture_backup(
        document: &WordDocument,
        table_order_index: usize,
    ) -> CommandResult<MergeTableBackup> {
        let table = document
            .table_by_order(table_order_index)
            .ok_or_else(|| CommandError::Custom("指定されたテーブルが見つかりません".into()))?;
        let table_node_id = table.node_id;
        let raw_table_element = match (document.raw_document.as_ref(), table_node_id) {
            (Some(raw), Some(table_id)) => raw.element(table_id).cloned(),
            _ => None,
        };

        Ok(MergeTableBackup {
            table_order_index,
            table: table.clone(),
            table_node_id,
            raw_table_element,
        })
    }

    fn restore_backup(document: &mut WordDocument, backup: &MergeTableBackup) -> CommandResult<()> {
        // A later structural table edit rewrites `<w:tbl>` and gives its root
        // a new node id. Undo that edit first and the merge backup still holds
        // the older id, so restore into the live root while bringing the
        // backup's children (and their model ids) back.
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

    pub fn preview_plan(
        word_doc: &WordDocument,
        table_order_index: usize,
        selected_cells: &[TableCellSelection],
    ) -> CommandResult<()> {
        Self::build_plan(word_doc, table_order_index, selected_cells).map(|_| ())
    }

    fn build_plan(
        word_doc: &WordDocument,
        table_order_index: usize,
        selected_cells: &[TableCellSelection],
    ) -> CommandResult<MergePlan> {
        if selected_cells.len() < 2 {
            return Err(CommandError::Custom(
                "セルをマージするには複数のセルを選択してください".into(),
            ));
        }

        let table = word_doc
            .table_by_order(table_order_index)
            .ok_or_else(|| CommandError::Custom("指定されたテーブルが見つかりません".into()))?;

        let mut selections = selected_cells.to_vec();
        selections.sort_by_key(|sel| (sel.row, sel.column));

        let mut planned_cells = Vec::new();
        let mut row_set = HashSet::new();
        let mut column_min = usize::MAX;
        let mut column_max = 0usize;

        for selection in selections.iter() {
            let row = table
                .rows
                .get(selection.row)
                .ok_or_else(|| CommandError::Custom("選択されたセルの行が存在しません".into()))?;
            let boundary = find_cell_boundary(row, selection.column).ok_or_else(|| {
                CommandError::Custom("選択されたセルの位置がテーブルと一致しません".into())
            })?;

            let cell = row
                .cells
                .get(boundary.cell_index)
                .ok_or_else(|| CommandError::Custom("内部エラー: セルが見つかりません".into()))?;
            let column_end = boundary.start + boundary.span;
            column_min = column_min.min(boundary.start);
            column_max = max(column_max, column_end);
            row_set.insert(selection.row);

            planned_cells.push(SelectionCellPlan {
                row_index: selection.row,
                cell_index: boundary.cell_index,
                column_start: boundary.start,
                column_span: boundary.span,
                cell_id: cell.node_id,
                vertical_merge: cell.vertical_merge.clone(),
            });
        }

        let row_start = *row_set
            .iter()
            .min()
            .ok_or_else(|| CommandError::Custom("選択範囲が正しく取得できませんでした".into()))?;
        let row_end = *row_set
            .iter()
            .max()
            .ok_or_else(|| CommandError::Custom("選択範囲が正しく取得できませんでした".into()))?;

        if column_min >= column_max {
            return Err(CommandError::Custom(
                "セルの矩形領域を特定できませんでした".into(),
            ));
        }

        let mut cells_by_row: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, cell) in planned_cells.iter().enumerate() {
            cells_by_row.entry(cell.row_index).or_default().push(index);
        }

        for row in row_start..=row_end {
            let indices = cells_by_row
                .get(&row)
                .ok_or_else(|| CommandError::Custom("選択範囲が矩形になっていません".into()))?;
            let mut coverage = column_min;
            let mut sorted_indices = indices.clone();
            sorted_indices.sort_by_key(|idx| planned_cells[*idx].column_start);

            let boundaries = row_cell_boundaries(
                table
                    .rows
                    .get(row)
                    .ok_or_else(|| CommandError::Custom("行が存在しません".into()))?,
            );
            let mut expected_cells: HashSet<usize> = boundaries
                .iter()
                .filter(|boundary| !(boundary.start >= column_max || boundary.end <= column_min))
                .map(|boundary| boundary.cell_index)
                .collect();

            for idx in sorted_indices {
                let cell = &planned_cells[idx];
                if cell.column_start != coverage {
                    return Err(CommandError::Custom(
                        "選択されたセルが連続していません".into(),
                    ));
                }
                coverage = cell.column_start + cell.column_span;
                expected_cells.remove(&cell.cell_index);
            }

            if coverage != column_max || !expected_cells.is_empty() {
                return Err(CommandError::Custom(
                    "セルの選択が矩形領域を完全に覆っていません".into(),
                ));
            }
        }

        validate_selected_vertical_merge_chains(table, &planned_cells)?;

        let master_index = planned_cells
            .iter()
            .position(|cell| cell.row_index == row_start && cell.column_start == column_min)
            .ok_or_else(|| CommandError::Custom("左上のセルを特定できませんでした".into()))?;

        if planned_cells[master_index].vertical_merge.as_deref() == Some("continue") {
            return Err(CommandError::Custom(
                "既存の結合セルの途中からはマージできません".into(),
            ));
        }

        let col_span = column_max - column_min;
        let row_span = row_end - row_start + 1;

        Ok(MergePlan {
            table_node_id: table.node_id,
            table_order_index,
            column_start: column_min,
            row_start,
            row_end,
            column_span: col_span,
            row_span,
            master_index,
            cells: planned_cells,
        })
    }

    fn apply_plan(
        document: &mut WordDocument,
        plan: &MergePlan,
    ) -> CommandResult<Option<SelectionRange>> {
        // Each follower (a covered cell below the master, in its column)
        // keeps one empty paragraph — `<w:tc>` must hold a `<w:p>` — and the
        // model's paragraph and the XML's share one node id.
        let followers: Vec<(usize, usize)> = plan
            .cells
            .iter()
            .filter(|cell| {
                cell.row_index != plan.row_start && cell.column_start == plan.column_start
            })
            .map(|cell| (cell.row_index, cell.cell_index))
            .collect();
        let follower_paragraph_ids: HashMap<usize, Option<XmlNodeId>> = followers
            .iter()
            .map(|(row, _)| {
                let id = match (document.raw_document.as_mut(), plan.table_node_id) {
                    (Some(raw), Some(_)) => Some(raw.allocate_node_id()),
                    _ => None,
                };
                (*row, id)
            })
            .collect();
        let paragraph_ptr = {
            let table = document
                .table_mut_by_order(plan.table_order_index)
                .ok_or_else(|| CommandError::Custom("テーブルが存在しません".into()))?;

            let mut combined_blocks: Vec<BodyElement> = Vec::new();
            for cell in plan.cells.iter() {
                let row = table
                    .rows
                    .get_mut(cell.row_index)
                    .ok_or_else(|| CommandError::Custom("行が存在しません".into()))?;
                let table_cell = row
                    .cells
                    .get_mut(cell.cell_index)
                    .ok_or_else(|| CommandError::Custom("セルが存在しません".into()))?;
                combined_blocks.extend(std::mem::take(&mut table_cell.blocks));
            }

            let master_ptr = {
                let master_cell_plan = &plan.cells[plan.master_index];
                let master_row = table
                    .rows
                    .get_mut(master_cell_plan.row_index)
                    .ok_or_else(|| CommandError::Custom("行が存在しません".into()))?;
                let master_cell = master_row
                    .cells
                    .get_mut(master_cell_plan.cell_index)
                    .ok_or_else(|| CommandError::Custom("セルが存在しません".into()))?;
                master_cell.blocks = combined_blocks;
                if plan.column_span > 1 {
                    master_cell.span = Some(plan.column_span as u32);
                } else {
                    master_cell.span = None;
                }
                master_cell.vertical_merge = if plan.row_span > 1 {
                    Some("restart".to_string())
                } else {
                    None
                };
                locate_first_paragraph(master_cell)
            };

            for row_index in plan.row_start..=plan.row_end {
                let keep_index = plan
                    .cells
                    .iter()
                    .find(|cell| {
                        cell.row_index == row_index && cell.column_start == plan.column_start
                    })
                    .map(|cell| cell.cell_index)
                    .ok_or_else(|| {
                        CommandError::Custom("結合するセルを特定できませんでした".into())
                    })?;

                let row = table
                    .rows
                    .get_mut(row_index)
                    .ok_or_else(|| CommandError::Custom("行が存在しません".into()))?;

                if row_index != plan.row_start {
                    if let Some(cell) = row.cells.get_mut(keep_index) {
                        if plan.column_span > 1 {
                            cell.span = Some(plan.column_span as u32);
                        } else {
                            cell.span = None;
                        }
                        cell.vertical_merge = Some("continue".to_string());
                        cell.blocks = vec![BodyElement::from_paragraph(Paragraph {
                            runs: Vec::new(),
                            properties: None,
                            node_id: follower_paragraph_ids.get(&row_index).copied().flatten(),
                            comment_ranges: Vec::new(),
                            comment_markers: Vec::new(),
                        })];
                    }
                }

                let remove_set: HashSet<usize> = plan
                    .cells
                    .iter()
                    .filter(|cell| cell.row_index == row_index && cell.cell_index != keep_index)
                    .map(|cell| cell.cell_index)
                    .collect();

                let mut idx = 0usize;
                row.cells.retain(|_| {
                    let keep = idx == keep_index || !remove_set.contains(&idx);
                    idx += 1;
                    keep
                });
            }
            master_ptr
        };

        if let Some(raw) = document.raw_document.as_mut() {
            apply_plan_to_raw(raw, plan, &follower_paragraph_ids)?;
        }
        document
            .sync_table_cell_widths(plan.table_order_index)
            .map_err(CommandError::Custom)?;

        document.rebuild_paragraph_surfaces();

        let selection = paragraph_ptr
            .and_then(|ptr| locate_paragraph_index(document, ptr))
            .map(|index| SelectionRange::collapsed(TextPosition::new(index, 0)));

        Ok(selection)
    }
}

impl Command for MergeTableCellsCommand {
    fn name(&self) -> &'static str {
        "merge_table_cells"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.selected_cells.len() < 2 {
            return Err(CommandError::Custom(
                "セルをマージするには複数のセルを選択してください".into(),
            ));
        }

        let plan = Self::build_plan(ctx.document, self.table_order_index, &self.selected_cells)?;
        let backup = Self::capture_backup(ctx.document, self.table_order_index)?;
        let previous_selection = *ctx.selection;

        match Self::apply_plan(ctx.document, &plan) {
            Ok(new_selection) => {
                self.backup = Some(backup);
                self.previous_selection = previous_selection;
                *ctx.selection = new_selection;
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

#[derive(Clone)]
struct MergeTableBackup {
    table_order_index: usize,
    table: Table,
    table_node_id: Option<XmlNodeId>,
    raw_table_element: Option<XmlElement>,
}

struct MergePlan {
    table_node_id: Option<XmlNodeId>,
    table_order_index: usize,
    column_start: usize,
    row_start: usize,
    row_end: usize,
    column_span: usize,
    row_span: usize,
    master_index: usize,
    cells: Vec<SelectionCellPlan>,
}

#[derive(Clone, Debug)]
struct SelectionCellPlan {
    row_index: usize,
    cell_index: usize,
    column_start: usize,
    column_span: usize,
    cell_id: Option<XmlNodeId>,
    vertical_merge: Option<String>,
}

pub(crate) struct CellBoundary {
    pub cell_index: usize,
    pub start: usize,
    pub span: usize,
    pub end: usize,
}

pub(crate) fn find_cell_boundary(row: &TableRow, column: usize) -> Option<CellBoundary> {
    let mut start = row.grid_before.unwrap_or(0) as usize;
    for (idx, cell) in row.cells.iter().enumerate() {
        let span = cell.span.unwrap_or(1).max(1) as usize;
        let end = start + span;
        if column >= start && column < end {
            return Some(CellBoundary {
                cell_index: idx,
                start,
                span,
                end,
            });
        }
        start = end;
    }
    None
}

fn row_cell_boundaries(row: &TableRow) -> Vec<CellBoundary> {
    let mut start = row.grid_before.unwrap_or(0) as usize;
    let mut result = Vec::new();
    for (idx, cell) in row.cells.iter().enumerate() {
        let span = cell.span.unwrap_or(1).max(1) as usize;
        let end = start + span;
        result.push(CellBoundary {
            cell_index: idx,
            start,
            span,
            end,
        });
        start = end;
    }
    result
}

fn exact_cell_at(row: &TableRow, start: usize, end: usize) -> Option<&TableCell> {
    let boundary = find_cell_boundary(row, start)?;
    if boundary.start == start && boundary.end == end {
        row.cells.get(boundary.cell_index)
    } else {
        None
    }
}

fn validate_selected_vertical_merge_chains(
    table: &Table,
    selected_cells: &[SelectionCellPlan],
) -> CommandResult<()> {
    let selected: HashSet<(usize, usize, usize)> = selected_cells
        .iter()
        .map(|cell| {
            (
                cell.row_index,
                cell.column_start,
                cell.column_start + cell.column_span,
            )
        })
        .collect();

    for selected_cell in selected_cells
        .iter()
        .filter(|cell| cell.vertical_merge.is_some())
    {
        let start = selected_cell.column_start;
        let end = start + selected_cell.column_span;
        let mut chain_start = selected_cell.row_index;

        while exact_cell_at(&table.rows[chain_start], start, end)
            .and_then(|cell| cell.vertical_merge.as_deref())
            == Some("continue")
        {
            chain_start = chain_start.checked_sub(1).ok_or_else(|| {
                CommandError::Custom("既存の縦方向の結合セルが完全に含まれていません".into())
            })?;
        }
        if exact_cell_at(&table.rows[chain_start], start, end)
            .and_then(|cell| cell.vertical_merge.as_deref())
            != Some("restart")
        {
            return Err(CommandError::Custom(
                "既存の縦方向の結合セルが完全に含まれていません".into(),
            ));
        }

        let mut chain_end = chain_start;
        while table.rows.get(chain_end + 1).is_some_and(|row| {
            exact_cell_at(row, start, end).and_then(|cell| cell.vertical_merge.as_deref())
                == Some("continue")
        }) {
            chain_end += 1;
        }
        if chain_end == chain_start
            || (chain_start..=chain_end).any(|row| !selected.contains(&(row, start, end)))
        {
            return Err(CommandError::Custom(
                "既存の縦方向の結合セルが完全に含まれていません".into(),
            ));
        }
    }
    Ok(())
}

fn locate_first_paragraph(cell: &TableCell) -> Option<*const Paragraph> {
    cell.blocks.iter().find_map(|block| match block {
        BodyElement::Paragraph(paragraph) => Some(paragraph.as_ref() as *const Paragraph),
        _ => None,
    })
}

fn locate_paragraph_index(document: &WordDocument, target: *const Paragraph) -> Option<usize> {
    for index in 0..document.paragraph_count() {
        if let Some(paragraph) = document.paragraph(index) {
            if ptr::eq(paragraph, unsafe { &*target }) {
                return Some(index);
            }
        }
    }
    None
}

pub(crate) fn find_row_element_mut(
    table_element: &mut XmlElement,
    row_index: usize,
) -> Option<&mut XmlElement> {
    let mut current = 0usize;
    for node in table_element.children.iter_mut() {
        if let XmlNode::Element(element) = node {
            if element.name == "w:tr" {
                if current == row_index {
                    return Some(element);
                }
                current += 1;
            }
        }
    }
    None
}

pub(crate) fn find_cell_element_mut(
    row_element: &mut XmlElement,
    cell_id: XmlNodeId,
) -> Option<&mut XmlElement> {
    for node in row_element.children.iter_mut() {
        if let XmlNode::Element(element) = node {
            if element.name == "w:tc" && element.id == cell_id {
                return Some(element);
            }
        }
    }
    None
}

fn remove_cell_by_id(row_element: &mut XmlElement, cell_id: XmlNodeId) {
    row_element.children.retain(|node| {
        if let XmlNode::Element(element) = node {
            if element.name == "w:tc" && element.id == cell_id {
                return false;
            }
        }
        true
    });
}

fn ensure_tc_pr<'a>(
    cell_element: &'a mut XmlElement,
    reserved_ids: &mut impl Iterator<Item = XmlNodeId>,
) -> &'a mut XmlElement {
    if let Some(position) = cell_element
        .children
        .iter()
        .position(|node| matches!(node, XmlNode::Element(element) if element.name == "w:tcPr"))
    {
        match cell_element.children.get_mut(position) {
            Some(XmlNode::Element(element)) => element,
            _ => unreachable!(),
        }
    } else {
        let tc_pr = XmlElement {
            id: reserved_ids.next().expect("reserved tcPr node id"),
            name: "w:tcPr".to_string(),
            attributes: Vec::new(),
            children: Vec::new(),
        };
        cell_element.children.insert(0, XmlNode::Element(tc_pr));
        match cell_element.children.first_mut() {
            Some(XmlNode::Element(element)) => element,
            _ => unreachable!(),
        }
    }
}

pub(crate) fn set_grid_span(
    cell_element: &mut XmlElement,
    span: usize,
    reserved_ids: &mut impl Iterator<Item = XmlNodeId>,
) {
    let tc_pr = ensure_tc_pr(cell_element, reserved_ids);
    tc_pr.children.retain(|node| {
        if let XmlNode::Element(element) = node {
            element.name != "w:gridSpan"
        } else {
            true
        }
    });
    if span > 1 {
        let grid_span = XmlElement {
            id: reserved_ids.next().expect("reserved gridSpan node id"),
            name: "w:gridSpan".to_string(),
            attributes: vec![XmlAttribute {
                name: "w:val".to_string(),
                value: span.to_string(),
            }],
            children: Vec::new(),
        };
        tc_pr.children.push(XmlNode::Element(grid_span));
    }
}

pub(crate) fn set_vertical_merge(
    cell_element: &mut XmlElement,
    value: Option<&str>,
    reserved_ids: &mut impl Iterator<Item = XmlNodeId>,
) {
    let tc_pr = ensure_tc_pr(cell_element, reserved_ids);
    tc_pr.children.retain(|node| {
        if let XmlNode::Element(element) = node {
            element.name != "w:vMerge"
        } else {
            true
        }
    });
    if let Some(val) = value {
        let merge = XmlElement {
            id: reserved_ids.next().expect("reserved vMerge node id"),
            name: "w:vMerge".to_string(),
            attributes: vec![XmlAttribute {
                name: "w:val".to_string(),
                value: val.to_string(),
            }],
            children: Vec::new(),
        };
        tc_pr.children.push(XmlNode::Element(merge));
    }
}

fn apply_plan_to_raw(
    raw: &mut crate::docx_xml::DocxXmlDocument,
    plan: &MergePlan,
    follower_paragraph_ids: &HashMap<usize, Option<XmlNodeId>>,
) -> CommandResult<()> {
    let table_id = match plan.table_node_id {
        Some(id) => id,
        None => return Ok(()),
    };
    let mut reserved_ids = (0..plan.cells.len().saturating_mul(4))
        .map(|_| raw.allocate_node_id())
        .collect::<Vec<_>>()
        .into_iter();
    let table_element = raw
        .element_mut(table_id)
        .ok_or_else(|| CommandError::Custom("テーブルのXMLを取得できませんでした".into()))?;

    let mut cell_children: HashMap<XmlNodeId, Vec<XmlNode>> = HashMap::new();
    for cell in plan.cells.iter().filter(|cell| cell.cell_id.is_some()) {
        let cell_id = cell.cell_id.unwrap();
        if let Some(row_element) = find_row_element_mut(table_element, cell.row_index) {
            if let Some(cell_element) = find_cell_element_mut(row_element, cell_id) {
                let children = std::mem::take(&mut cell_element.children);
                cell_children.insert(cell_id, children);
            }
        }
    }

    for row_index in plan.row_start..=plan.row_end {
        if let Some(row_element) = find_row_element_mut(table_element, row_index) {
            for remove_id in plan
                .cells
                .iter()
                .filter(|cell| {
                    cell.row_index == row_index && cell.column_start != plan.column_start
                })
                .filter_map(|cell| cell.cell_id)
            {
                remove_cell_by_id(row_element, remove_id);
            }
        }
    }

    let mut follower_properties: HashMap<XmlNodeId, XmlNode> = HashMap::new();
    if let Some(master_id) = plan.cells[plan.master_index].cell_id {
        if let Some(row_element) =
            find_row_element_mut(table_element, plan.cells[plan.master_index].row_index)
        {
            if let Some(cell_element) = find_cell_element_mut(row_element, master_id) {
                let mut combined = Vec::new();
                let mut master_properties = None;
                for cell in plan.cells.iter() {
                    if let Some(id) = cell.cell_id {
                        if let Some(nodes) = cell_children.remove(&id) {
                            for node in nodes {
                                if matches!(&node, XmlNode::Element(element) if element.name == "w:tcPr")
                                {
                                    if id == master_id && master_properties.is_none() {
                                        master_properties = Some(node);
                                    } else if id != master_id {
                                        follower_properties.insert(id, node);
                                    }
                                } else {
                                    combined.push(node);
                                }
                            }
                        }
                    }
                }
                if let Some(properties) = master_properties {
                    combined.insert(0, properties);
                }
                cell_element.children = combined;
                set_grid_span(cell_element, plan.column_span, &mut reserved_ids);
                set_vertical_merge(
                    cell_element,
                    if plan.row_span > 1 {
                        Some("restart")
                    } else {
                        None
                    },
                    &mut reserved_ids,
                );
            }
        }
    }

    for cell in plan
        .cells
        .iter()
        .filter(|cell| cell.row_index != plan.row_start)
    {
        if let Some(cell_id) = cell.cell_id {
            if let Some(row_element) = find_row_element_mut(table_element, cell.row_index) {
                if let Some(cell_element) = find_cell_element_mut(row_element, cell_id) {
                    cell_element.children.clear();
                    if let Some(properties) = follower_properties.remove(&cell_id) {
                        cell_element.children.push(properties);
                    }
                    set_grid_span(cell_element, plan.column_span, &mut reserved_ids);
                    set_vertical_merge(cell_element, Some("continue"), &mut reserved_ids);
                    let paragraph_id = follower_paragraph_ids
                        .get(&cell.row_index)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| {
                            reserved_ids
                                .next()
                                .expect("reserved continuation paragraph id")
                        });
                    cell_element.children.push(XmlNode::Element(XmlElement {
                        id: paragraph_id,
                        name: "w:p".to_string(),
                        attributes: Vec::new(),
                        children: Vec::new(),
                    }));
                }
            }
        }
    }

    raw.rebuild_index();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::TableCellSelection;
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, StyleMap, Table, TableCell, TableRow,
    };
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use std::io::Read;
    use std::path::PathBuf;

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
                    revision: None,
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
                    revision: None,
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

    fn table_with_rows(rows: Vec<Vec<TableCell>>) -> WordDocument {
        let table = Table {
            rows: rows
                .into_iter()
                .map(|cells| TableRow {
                    revision: None,
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
                })
                .collect(),
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

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn merge_simple_rectangle() {
        let mut word_doc = simple_table_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let selections = vec![
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
                    Box::new(MergeTableCellsCommand::new(0, selections.clone())),
                    &mut ctx,
                )
                .expect("merge cells");

            assert!((*ctx.selection).is_some());
        }

        if let Some(sel) = selection.as_ref() {
            assert!(word_doc.paragraph(sel.anchor.paragraph).is_some());
        }

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[1].cells.len(), 1);

        let master = &table.rows[0].cells[0];
        assert_eq!(master.span, Some(2));
        assert_eq!(master.vertical_merge.as_deref(), Some("restart"));
        let merged_text: Vec<String> = master
            .blocks
            .iter()
            .filter_map(|block| {
                if let BodyElement::Paragraph(paragraph) = block {
                    Some(
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>(),
                    )
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(merged_text, vec!["A1", "A2", "B1", "B2"]);

        let second = &table.rows[1].cells[0];
        assert_eq!(second.span, Some(2));
        assert_eq!(second.vertical_merge.as_deref(), Some("continue"));
        assert_eq!(
            second.blocks.len(),
            1,
            "a follower keeps one empty paragraph"
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo merge");
        }

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[1].cells.len(), 2);
    }

    #[test]
    fn merge_requires_rectangle() {
        let word_doc = simple_table_document();
        let selections = vec![
            TableCellSelection {
                row: 0,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 1,
                column: 1,
                column_span: 1,
            },
        ];
        let result = MergeTableCellsCommand::preview_plan(&word_doc, 0, &selections);
        assert!(result.is_err());
    }

    #[test]
    fn merge_rejects_a_partial_existing_vertical_merge_chain() {
        let mut word_doc = simple_table_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(MergeTableCellsCommand::new(
                        0,
                        vec![
                            TableCellSelection {
                                row: 0,
                                column: 0,
                                column_span: 1,
                            },
                            TableCellSelection {
                                row: 1,
                                column: 0,
                                column_span: 1,
                            },
                        ],
                    )),
                    &mut ctx,
                )
                .expect("vertical merge");
        }
        let before = word_doc.table_by_order(0).expect("table exists").clone();

        let result = MergeTableCellsCommand::preview_plan(
            &word_doc,
            0,
            &[
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
            ],
        );
        assert!(result
            .expect_err("a partial vertical chain must be rejected")
            .to_string()
            .contains("完全に含まれていません"));

        let after = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(after.rows.len(), before.rows.len());
        assert!(after.rows.iter().zip(&before.rows).all(|(after, before)| {
            after.cells.len() == before.cells.len()
                && after
                    .cells
                    .iter()
                    .zip(&before.cells)
                    .all(|(after, before)| {
                        after.span == before.span && after.vertical_merge == before.vertical_merge
                    })
        }));
    }

    #[test]
    fn merge_horizontal_cells_single_row() {
        let mut word_doc = table_with_rows(vec![vec![make_cell("L"), make_cell("R")]]);
        let mut selection = None;
        {
            let mut executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(MergeTableCellsCommand::new(
                        0,
                        vec![
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
                        ],
                    )),
                    &mut ctx,
                )
                .expect("merge horizontal cells");
        }
        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows[0].cells.len(), 1);
        let merged = &table.rows[0].cells[0];
        assert_eq!(merged.span, Some(2));
        assert!(merged.vertical_merge.is_none());
        let merged_text: Vec<String> = merged
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
            .collect();
        assert_eq!(merged_text, vec!["L", "R"]);
    }

    #[test]
    fn merge_three_rows_rectangle() {
        let mut word_doc = table_with_rows(vec![
            vec![make_cell("A"), make_cell("B")],
            vec![make_cell("C"), make_cell("D")],
            vec![make_cell("E"), make_cell("F")],
        ]);

        let selections = vec![
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
            TableCellSelection {
                row: 2,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 2,
                column: 1,
                column_span: 1,
            },
        ];

        let mut selection = None;
        {
            let mut executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(MergeTableCellsCommand::new(0, selections)),
                    &mut ctx,
                )
                .expect("merge three rows");
        }

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[1].cells.len(), 1);
        assert_eq!(table.rows[2].cells.len(), 1);

        let master = &table.rows[0].cells[0];
        assert_eq!(master.span, Some(2));
        assert_eq!(master.vertical_merge.as_deref(), Some("restart"));
        let texts: Vec<String> = master
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
            .collect();
        assert_eq!(texts, vec!["A", "B", "C", "D", "E", "F"]);

        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
        assert_eq!(table.rows[1].cells[0].blocks.len(), 1);
        assert_eq!(
            table.rows[2].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
        assert_eq!(table.rows[2].cells[0].blocks.len(), 1);
    }

    #[test]
    fn redo_reapplies_merge_after_undo() {
        let mut word_doc = simple_table_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let selections = vec![
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
                    Box::new(MergeTableCellsCommand::new(0, selections)),
                    &mut ctx,
                )
                .expect("merge cells");
        }
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo merge"));
            assert!(executor.redo(&mut ctx).expect("redo merge"));
        }

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[1].cells.len(), 1);
        assert_eq!(
            table.rows[0].cells[0].vertical_merge.as_deref(),
            Some("restart")
        );
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
        assert!(
            selection.is_some(),
            "redo should restore collapsed selection"
        );
    }

    #[test]
    fn undo_redo_roundtrip_loaded_docx_table() {
        let data = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let selections = vec![
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
                    Box::new(MergeTableCellsCommand::new(0, selections)),
                    &mut ctx,
                )
                .expect("merge cells");
        }
        {
            let table = word_doc.table_by_order(0).expect("table exists");
            assert_eq!(table.rows[0].cells.len(), 1);
            assert_eq!(table.rows[1].cells.len(), 1);
        }

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo merge"));
        }
        {
            let table = word_doc.table_by_order(0).expect("table exists");
            assert_eq!(table.rows[0].cells.len(), 2);
            assert_eq!(table.rows[1].cells.len(), 2);
        }

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo merge"));
        }
        {
            let table = word_doc.table_by_order(0).expect("table exists");
            assert_eq!(table.rows[0].cells.len(), 1);
            assert_eq!(table.rows[1].cells.len(), 1);
            assert_eq!(
                table.rows[0].cells[0].vertical_merge.as_deref(),
                Some("restart")
            );
            assert_eq!(
                table.rows[1].cells[0].vertical_merge.as_deref(),
                Some("continue")
            );
        }

        word_doc.export_docx().expect("export merged docx");
    }

    #[test]
    fn vertical_merge_continuation_cells_export_with_required_block_content() {
        let data = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let selections = (0..2)
            .flat_map(|row| {
                (0..2).map(move |column| TableCellSelection {
                    row,
                    column,
                    column_span: 1,
                })
            })
            .collect();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(MergeTableCellsCommand::new(0, selections)),
                    &mut ctx,
                )
                .expect("merge cells");
        }

        let exported = word_doc.export_docx().expect("export merged docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document xml")
            .read_to_string(&mut xml)
            .expect("read document xml");
        let continuation_cells = xml
            .match_indices("<w:vMerge w:val=\"continue\"")
            .map(|(merge_position, _)| {
                let cell_start = xml[..merge_position]
                    .rfind("<w:tc>")
                    .expect("continuation cell start");
                let relative_end = xml[merge_position..]
                    .find("</w:tc>")
                    .expect("continuation cell end");
                let cell = &xml[cell_start..merge_position + relative_end];
                assert!(
                    cell.contains("<w:p") || cell.contains("<w:tbl"),
                    "continuation w:tc must contain w:p or w:tbl: {cell}"
                );
            })
            .count();
        assert!(continuation_cells > 0, "expected a continuation cell");

        for cell in xml
            .split("<w:tc>")
            .skip(1)
            .filter_map(|tail| tail.find("</w:tc>").map(|end| &tail[..end]))
        {
            assert!(
                cell.matches("<w:tcPr").count() <= 1,
                "merged w:tc must not contain multiple w:tcPr elements: {cell}"
            );
        }
    }

    /// A vertical merge leaves its follower cells with one empty paragraph —
    /// in the model as in the XML (`<w:tc>` must hold a `<w:p>`) — so the
    /// document has the same paragraphs before and after a save, and a
    /// follower freed by an unmerge takes text that survives the save.
    #[test]
    fn followers_keep_a_paragraph_in_memory_as_in_the_xml() {
        use crate::headless_api::apply_headless_commands_to_document;
        use jade_docx_headless_api::{
            HeadlessCommand, HeadlessEditRequest, HeadlessTableCellSelection,
        };
        let run = |doc: &mut WordDocument, command: HeadlessCommand| {
            apply_headless_commands_to_document(
                doc,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .expect("command applies");
        };
        let bytes =
            std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hello.docx"))
                .expect("hello.docx");
        let mut doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("loads");
        run(
            &mut doc,
            HeadlessCommand::InsertTable {
                after_paragraph: 0,
                rows: 3,
                columns: 3,
                column_widths: Vec::new(),
                caret_offset: None,
            },
        );
        let cell = |row, column| HeadlessTableCellSelection {
            row,
            column,
            column_span: 1,
        };
        run(
            &mut doc,
            HeadlessCommand::MergeTableCells {
                table_order_index: 0,
                selected_cells: vec![cell(1, 1), cell(1, 2), cell(2, 1), cell(2, 2)],
            },
        );
        let follower = &doc.table_by_order(0).expect("table").rows[2].cells[1];
        assert_eq!(follower.vertical_merge.as_deref(), Some("continue"));
        assert_eq!(follower.blocks.len(), 1, "one empty paragraph");
        let reopen = |doc: &WordDocument| {
            executor::block_on(WordDocument::load(
                &doc.export_docx().expect("exports"),
                &Options::default(),
            ))
            .expect("reloads")
        };
        assert_eq!(reopen(&doc).paragraph_count(), doc.paragraph_count());

        run(
            &mut doc,
            HeadlessCommand::UnmergeTableCells {
                table_order_index: 0,
                target_cells: vec![HeadlessTableCellSelection {
                    row: 1,
                    column: 1,
                    column_span: 2,
                }],
            },
        );
        assert_eq!(reopen(&doc).paragraph_count(), doc.paragraph_count());
        run(
            &mut doc,
            HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row: 2,
                column: 1,
                text: "freed".into(),
            },
        );
        let reopened = reopen(&doc);
        let text: String =
            match &reopened.table_by_order(0).expect("table").rows[2].cells[1].blocks[0] {
                BodyElement::Paragraph(paragraph) => {
                    paragraph.runs.iter().map(|r| r.text.as_str()).collect()
                }
                _ => String::new(),
            };
        assert_eq!(text, "freed");
    }
}
