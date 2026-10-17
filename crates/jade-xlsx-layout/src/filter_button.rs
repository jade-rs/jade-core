//! Saved filter controls, with geometry shared by painters and input hosts.
use crate::{AxisEntry, Rect, SheetLayout};
use jade_xlsx_reader::{auto_filter::AutoFilter, CellRange, CellRef, Sheet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilterButtonTarget {
    Worksheet,
    Table { name: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterButton {
    pub target: FilterButtonTarget,
    /// Zero-based column within the owner's range.
    pub column: u32,
    pub reference: CellRef,
    pub range: CellRange,
    pub active: bool,
    pub rect: Rect,
}
#[derive(Clone)]
pub(crate) struct Definition {
    target: FilterButtonTarget,
    column: u32,
    pub(crate) reference: CellRef,
    range: CellRange,
    active: bool,
}
pub(crate) type Definitions = BTreeMap<(u32, u32), Definition>;
pub(crate) fn definitions(sheet: &Sheet) -> Definitions {
    let mut buttons = BTreeMap::new();
    let mut add = |filter: &AutoFilter, target: FilterButtonTarget, inferred: Option<CellRange>| {
        if filter
            .issues
            .iter()
            .any(|issue| issue != "missing filter range" || inferred.is_none())
        {
            return;
        }
        let Some(range) = filter.reference.or(inferred) else {
            return;
        };
        let saved_columns: BTreeMap<_, _> = filter
            .columns
            .iter()
            .map(|entry| (entry.column_id, entry))
            .collect();
        for column in range.start.column..=range.end.column {
            let offset = column - range.start.column;
            let saved = saved_columns.get(&offset);
            buttons.remove(&(range.start.row, column));
            if saved.is_some_and(|entry| entry.hidden_button || !entry.show_button) {
                continue;
            }
            let reference = CellRef::new(column, range.start.row);
            buttons.insert(
                (reference.row, reference.column),
                Definition {
                    target: target.clone(),
                    column: offset,
                    reference,
                    range,
                    active: sheet.filter_mode != Some(false)
                        && saved.is_some_and(|entry| entry.criterion.is_some()),
                },
            );
        }
    };
    if let Some(filter) = &sheet.auto_filter {
        add(filter, FilterButtonTarget::Worksheet, None);
    }
    // A table is the selected owner when saved ranges overlap.
    for table in &sheet.tables {
        if table.header_row_count == 1 {
            if let Some(filter) = &table.filter {
                add(
                    filter,
                    FilterButtonTarget::Table {
                        name: table.name.clone(),
                    },
                    Some(table.reference),
                );
            }
        }
    }
    buttons
}
fn width(column: &AxisEntry) -> f64 {
    (column.size - 2.0).clamp(0.0, 16.0)
}
pub(crate) fn text_reserve(
    definitions: &Definitions,
    reference: CellRef,
    merge: Option<CellRange>,
    columns: &[AxisEntry],
) -> f64 {
    let end = merge
        .map(|range| range.end.column)
        .unwrap_or(reference.column);
    let Some(((_, at), _)) = definitions
        .range((reference.row, reference.column)..=(reference.row, end))
        .next()
    else {
        return 0.0;
    };
    let Some(column) = columns
        .get(*at as usize)
        .filter(|column| !column.hidden && column.size >= 8.0)
    else {
        return 0.0;
    };
    let Some(last) = columns.get(end as usize) else {
        return 0.0;
    };
    (last.offset + last.size - (column.offset + column.size - width(column) - 1.0)).max(0.0)
}
pub(crate) fn place(
    definitions: Definitions,
    columns: &[AxisEntry],
    rows: &[AxisEntry],
) -> Vec<FilterButton> {
    definitions
        .into_values()
        .filter_map(|definition| {
            let column = columns.get(definition.reference.column as usize)?;
            let row = rows.get(definition.reference.row as usize)?;
            if column.hidden || row.hidden || column.size < 8.0 || row.size < 8.0 {
                return None;
            }
            let height = (row.size - 2.0).min(16.0);
            Some(FilterButton {
                target: definition.target,
                column: definition.column,
                reference: definition.reference,
                range: definition.range,
                active: definition.active,
                rect: Rect {
                    x: column.offset + column.size - width(column) - 1.0,
                    y: row.offset + (row.size - height) / 2.0,
                    width: width(column),
                    height,
                },
            })
        })
        .collect()
}
impl SheetLayout {
    pub fn filter_button_at(&self, x: f64, y: f64) -> Option<&FilterButton> {
        if self.shape_at(x, y).is_some() {
            return None;
        }
        self.filter_buttons.iter().find(|button| {
            x >= button.rect.x
                && x < button.rect.right()
                && y >= button.rect.y
                && y < button.rect.bottom()
        })
    }
    pub fn filter_button_for_cell(&self, reference: CellRef) -> Option<&FilterButton> {
        self.filter_buttons
            .iter()
            .filter(|button| {
                button.reference.column == reference.column && button.range.contains(reference)
            })
            .max_by_key(|button| matches!(button.target, FilterButtonTarget::Table { .. }))
    }
}
