//! Excel tables when the grid changes shape, and when a header cell is
//! edited.
//!
//! Excel validates a table against the sheet when it opens the file: the
//! `tableColumn` count has to equal the width of `ref`, and every header
//! cell has to hold the name of its column. So a column inserted inside a
//! table needs a `tableColumn` of its own — with a name Excel would give it,
//! `Column1`, and that name written into the header cell — a deleted column
//! takes its `tableColumn` with it, a deleted header row makes the row
//! below the header (its cells rewritten with the column names), and a
//! table whose every row or column went is removed with its part,
//! relationship and content type. Typing into a header cell renames the
//! column, the way it does in Excel.

use quick_xml::events::Event;
use quick_xml::Reader;

/// Normalizes edited table headers and updates the table parts in the same
/// transaction. A paste and a direct cell edit follow the same naming rules.
pub fn follow_table_headers(
    original: &[u8],
    workbook: &jade_xlsx_reader::Workbook,
    sheets: &mut [crate::SheetEdits],
) -> crate::Result<Vec<(String, String)>> {
    use crate::NewValue;
    use jade_xlsx_reader::CellValue;
    use std::collections::BTreeMap;
    let package = jade_opc::OpcPackage::open(original)?;
    let mut parts: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for edits in sheets {
        let sheet = workbook
            .sheets
            .iter()
            .find(|sheet| sheet.part_name == edits.part_name)
            .ok_or_else(|| crate::WriteError::MissingPart(edits.part_name.clone()))?;
        for edit in &mut edits.cells {
            if matches!(edit.value, NewValue::Unchanged) {
                continue;
            }
            let Some(table) = sheet.table_at(edit.reference) else {
                continue;
            };
            if table.header_row() != Some(edit.reference.row) {
                continue;
            }
            let position = (edit.reference.column - table.reference.start.column) as usize;
            if position >= table.columns.len() {
                continue;
            }
            if !parts.contains_key(&table.part_name) {
                let xml = package
                    .part_str(&table.part_name)?
                    .ok_or_else(|| crate::WriteError::MissingPart(table.part_name.clone()))?;
                parts.insert(table.part_name.clone(), (xml, table.columns.clone()));
            }
            let (xml, names) = parts.get_mut(&table.part_name).expect("table inserted");
            let wanted = match &edit.value {
                NewValue::Text(text) | NewValue::Error(text) => Some(text.clone()),
                NewValue::Number(value) => Some(
                    jade_xlsx_layout::number_format::format_value(
                        &CellValue::Number(*value),
                        Some("General"),
                        workbook.date_system,
                    )
                    .text,
                ),
                NewValue::Bool(value) => Some(if *value { "TRUE" } else { "FALSE" }.into()),
                NewValue::Formula(formula) | NewValue::FormulaCached { formula, .. } => {
                    Some(format!("={formula}"))
                }
                NewValue::Blank | NewValue::Remove | NewValue::Unchanged => None,
            };
            let name = match wanted {
                Some(wanted) if !wanted.trim().is_empty() => {
                    unique_column_name(&wanted, names, position)
                }
                _ => fresh_column_name(names, &mut 0),
            };
            *xml = rename_table_column(xml, position, &name)?;
            names[position] = name.clone();
            edit.value = NewValue::Text(name);
        }
    }
    Ok(parts
        .into_iter()
        .map(|(part, (xml, _))| (part, xml))
        .collect())
}

use jade_xlsx_reader::{shift_formula, Axis, CellRange, CellRef, StructureChange};

use crate::patch::{apply, escape, Splice};
use crate::shift::range_text;
use crate::{CellEdit, NewValue, Result};

/// A table part after a structural change.
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftedTable {
    /// The new part text, or `None` when the table was deleted.
    pub xml: Option<String>,
    /// Header cells the sheet has to write so they match the column names:
    /// the header of every new column, or the whole header when the row it
    /// was on went away.
    pub header_cells: Vec<CellEdit>,
}

/// What the first pass learns about a table part.
#[derive(Debug, Default, Clone)]
struct TableShape {
    reference: Option<CellRange>,
    header_row_count: u32,
    totals_row_count: u32,
    names: Vec<String>,
    ids: Vec<u32>,
}

fn read_shape(xml: &str) -> Result<TableShape> {
    Ok(match jade_xlsx_reader::table::parse_table("", xml)? {
        Some(t) => TableShape {
            reference: Some(t.reference),
            header_row_count: t.header_row_count,
            totals_row_count: t.totals_row_count,
            names: t.columns,
            ids: t.column_properties.into_iter().map(|c| c.id).collect(),
        },
        None => TableShape::default(),
    })
}

/// A name no column of the table has, the way Excel picks one: `Column1`,
/// then `Column2`, counting past every name in use.
pub fn fresh_column_name(taken: &[String], from: &mut u32) -> String {
    loop {
        *from += 1;
        let candidate = format!("Column{from}");
        if !taken
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
}

/// `wanted` made unique among `taken` (ignoring the entry at `own`, the
/// column being renamed), the way Excel does when a header is typed over
/// an existing name: `Amount` becomes `Amount2`, then `Amount3`.
pub fn unique_column_name(wanted: &str, taken: &[String], own: usize) -> String {
    let clashes = |candidate: &str| {
        taken
            .iter()
            .enumerate()
            .any(|(index, name)| index != own && name.to_lowercase() == candidate.to_lowercase())
    };
    if !clashes(wanted) {
        return wanted.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{wanted}{suffix}");
        if !clashes(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// The table part with `change` applied. `sheet_name` is the sheet the
/// table sits on, which is what a bare reference in a column formula means.
pub fn shift_table(xml: &str, sheet_name: &str, change: &StructureChange) -> Result<ShiftedTable> {
    shift_table_impl(xml, sheet_name, change, true)
}

pub(crate) fn shift_table_geometry(
    xml: &str,
    sheet_name: &str,
    change: &StructureChange,
) -> Result<ShiftedTable> {
    shift_table_impl(xml, sheet_name, change, false)
}

fn shift_table_impl(
    xml: &str,
    sheet_name: &str,
    change: &StructureChange,
    formulas: bool,
) -> Result<ShiftedTable> {
    let shape = read_shape(xml)?;
    let Some(reference) = shape.reference else {
        return Ok(ShiftedTable {
            xml: Some(xml.to_string()),
            header_cells: Vec::new(),
        });
    };
    let Some(mut mapped) = change.map_range(reference) else {
        return Ok(ShiftedTable {
            xml: None,
            header_cells: Vec::new(),
        });
    };

    let has_header = shape.header_row_count > 0;
    let mut totals_row_count = shape.totals_row_count;
    let mut header_cells = Vec::new();
    // The header row went with a deletion: the row that took its place is
    // the header now, and has to say so.
    let mut header_row_replaced = false;
    // New columns: (position among the existing columns, name).
    let mut new_columns: Vec<(usize, String)> = Vec::new();
    // Existing columns to drop, by position.
    let mut dropped: Vec<usize> = Vec::new();
    let mut names = shape.names.clone();

    match change.axis {
        Axis::Rows if !change.inserted => {
            let deleted_end = change.at.saturating_add(change.count);
            let deleted = |row: u32| row >= change.at && row < deleted_end;
            if has_header && deleted(reference.start.row) {
                header_row_replaced = true;
            }
            if totals_row_count > 0 && deleted(reference.end.row) {
                // Deleting the totals row turns totals off, as Excel does.
                totals_row_count = 0;
            }
            // A table keeps at least one data row: when the deletion left
            // only the header and the totals, the totals row becomes that
            // data row (totals off), and a header alone gains the empty
            // row below it.
            let rows = mapped.end.row - mapped.start.row + 1;
            if rows <= u32::from(has_header) + totals_row_count {
                totals_row_count = 0;
            }
            if rows <= u32::from(has_header)
                && mapped.end.row + 1 < jade_xlsx_reader::model::MAX_ROW_COUNT
            {
                mapped.end.row += 1;
            }
        }
        Axis::Columns if change.inserted => {
            // Columns inserted inside the table (not at its first column,
            // which moves the whole table) need tableColumn entries.
            if change.at > reference.start.column && change.at <= reference.end.column {
                let position = (change.at - reference.start.column) as usize;
                let mut counter = 0u32;
                for offset in 0..change.count {
                    let name = fresh_column_name(&names, &mut counter);
                    names.push(name.clone());
                    new_columns.push((position, name.clone()));
                    if has_header {
                        header_cells.push(CellEdit {
                            reference: CellRef::new(change.at + offset, reference.start.row),
                            value: NewValue::Text(name),
                            style_index: None,
                        });
                    }
                }
            }
        }
        Axis::Columns => {
            let deleted_end = change.at.saturating_add(change.count);
            for (position, _) in shape.names.iter().enumerate() {
                let column = reference.start.column + position as u32;
                if column >= change.at && column < deleted_end {
                    dropped.push(position);
                }
            }
        }
        Axis::Rows => {}
    }

    if header_row_replaced {
        let kept: Vec<&String> = names
            .iter()
            .enumerate()
            .filter(|(position, _)| !dropped.contains(position))
            .map(|(_, name)| name)
            .collect();
        for (offset, name) in kept.into_iter().enumerate() {
            header_cells.push(CellEdit {
                reference: CellRef::new(mapped.start.column + offset as u32, mapped.start.row),
                value: NewValue::Text(name.clone()),
                style_index: None,
            });
        }
    }

    let filter_range = CellRange {
        start: mapped.start,
        end: CellRef::new(
            mapped.end.column,
            mapped.end.row.saturating_sub(totals_row_count),
        ),
    };
    let column_count = shape.names.len() + new_columns.len() - dropped.len();
    let next_id = shape
        .ids
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| {
            n.checked_add(new_columns.len().saturating_sub(1) as u32)
                .is_some()
        })
        .unwrap_or_else(|| {
            (1..)
                .find(|first| {
                    (0..new_columns.len() as u32)
                        .all(|offset| !shape.ids.contains(&(first + offset)))
                })
                .unwrap()
        });

    let mut updates = vec![("ref", Some(range_text(mapped)))];
    if totals_row_count != shape.totals_row_count {
        updates.push(("totalsRowCount", None));
        updates.push(("totalsRowShown", Some("0".into())));
    }
    let mut xml = crate::xml_edit::root_attributes(xml, &updates)?;
    xml = crate::cell_shift::children_map(&xml, b"autoFilter", |f| {
        crate::filter::shift_auto_filter(f, change, filter_range)
    })?;
    xml = crate::cell_shift::children_map(&xml, b"sortState", |f| {
        Ok(crate::filter::shift_sort_state(f, change)?.unwrap_or_default())
    })?;
    xml = crate::cell_shift::children_map(&xml, b"tableColumns", |container| {
        let mut splices = Vec::new();
        let entries = crate::xml_edit_ns::children(container, b"tableColumn")?;
        let mut inserted = false;
        for (position, (start, end, entry)) in entries.iter().enumerate() {
            let prefix = root_prefix(container)?;
            if !inserted && new_columns.first().is_some_and(|(at, _)| *at == position) {
                splices.push(Splice {
                    start: *start,
                    end: *start,
                    text: new_column_xml(&new_columns, next_id, &prefix),
                });
                inserted = true;
            }
            if dropped.contains(&position) {
                splices.push(Splice {
                    start: *start,
                    end: *end,
                    text: String::new(),
                });
            } else if formulas {
                let mut changed = entry.text.clone();
                for name in [b"calculatedColumnFormula".as_slice(), b"totalsRowFormula"] {
                    changed = crate::cell_shift::children_map(&changed, name, |f| {
                        let formula = crate::cell_shift::leaf_text(f)?;
                        let shifted = shift_formula(&formula, sheet_name, change);
                        if shifted == formula {
                            Ok(f.into())
                        } else {
                            crate::cell_shift::text_content(f, &shifted)
                        }
                    })?;
                }
                if changed != entry.text {
                    splices.push(Splice {
                        start: *start,
                        end: *end,
                        text: entry.finish(&changed)?,
                    });
                }
            }
        }
        let mut moved = apply(container, splices);
        if !inserted && !new_columns.is_empty() {
            moved = crate::xml_edit::append_child(
                &moved,
                &new_column_xml(&new_columns, next_id, &root_prefix(container)?),
            )?;
        }
        crate::xml_edit::root_attributes(&moved, &[("count", Some(column_count.to_string()))])
    })?;
    Ok(ShiftedTable {
        xml: Some(xml),
        header_cells,
    })
}

fn root_prefix(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                return Ok(crate::patch::namespace_prefix(e.name().as_ref()))
            }
            Event::Eof => return Err(crate::WriteError::NoRoot("table")),
            _ => {}
        }
    }
}

fn new_column_xml(columns: &[(usize, String)], first_id: u32, prefix: &str) -> String {
    let mut text = String::new();
    for (offset, (_, name)) in columns.iter().enumerate() {
        text.push_str(&format!(
            "<{prefix}tableColumn id=\"{}\" name=\"{}\"/>",
            first_id + offset as u32,
            escape(name)
        ));
    }
    text
}

/// The table part with the column at `position` (0-based, left to right)
/// renamed to `name`. The caller has made the name unique.
pub fn rename_table_column(xml: &str, position: usize, name: &str) -> Result<String> {
    if name.encode_utf16().count() > 255 {
        return Err(crate::WriteError::Table(
            "a table header exceeds 255 characters".into(),
        ));
    }
    let container = crate::xml_edit_ns::children(xml, b"tableColumns")?
        .into_iter()
        .next()
        .ok_or_else(|| crate::WriteError::Table("missing table columns".into()))?
        .2;
    let (start, end, entry) = crate::xml_edit_ns::children(&container.text, b"tableColumn")?
        .into_iter()
        .nth(position)
        .ok_or_else(|| crate::WriteError::Table("column is outside the table".into()))?;
    let name = jade_xlsx_reader::xstring::encode_xstring(name)
        .replace('\n', "_x000A_")
        .replace('\t', "_x0009_");
    let updated = crate::xml_edit::root_attributes(&entry.text, &[("name", Some(name))])?;
    let updated = apply(
        &container.text,
        vec![Splice {
            start,
            end,
            text: entry.finish(&updated)?,
        }],
    );
    let updated = container.finish(&updated)?;
    crate::xml_edit_ns::set_child(xml, b"tableColumns", |_| updated, &[])
}

/// The sheet part without the `<tablePart>` entries whose `r:id` is in
/// `ids`, its `<tableParts count>` kept true — and dropped when nothing
/// is left in it.
pub fn remove_table_parts(xml: &str, ids: &[String]) -> Result<String> {
    if ids.is_empty() {
        return Ok(xml.to_string());
    }
    crate::cell_shift::children_map(xml, b"tableParts", |container| {
        let entries = crate::xml_edit_ns::children(container, b"tablePart")?;
        let mut kept = entries.len();
        let mut edits = Vec::new();
        for (start, end, entry) in entries {
            if jade_xlsx_reader::table::table_part_id(&entry.text)?
                .is_some_and(|id| ids.contains(&id))
            {
                kept -= 1;
                edits.push(Splice {
                    start,
                    end,
                    text: String::new(),
                });
            }
        }
        if edits.is_empty() {
            return Ok(container.into());
        }
        if kept == 0 {
            return Ok(String::new());
        }
        crate::xml_edit::root_attributes(
            &apply(container, edits),
            &[("count", Some(kept.to_string()))],
        )
    })
}
