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
use quick_xml::name::QName;
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

use crate::patch::{apply, attribute, escape, local, Splice};
use crate::shift::{range_text, rewrite, rewrite_head_text};
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
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut shape = TableShape::default();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                match local(element.name().as_ref()) {
                    b"table" => {
                        shape.reference = attribute(element, b"ref")?
                            .and_then(|value| CellRange::parse(&value).ok());
                        shape.header_row_count = attribute(element, b"headerRowCount")?
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1);
                        shape.totals_row_count = attribute(element, b"totalsRowCount")?
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0);
                    }
                    b"tableColumn" => {
                        shape
                            .names
                            .push(attribute(element, b"name")?.unwrap_or_default());
                        shape.ids.push(
                            attribute(element, b"id")?
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(0),
                        );
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(shape)
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
            .any(|(index, name)| index != own && name.eq_ignore_ascii_case(candidate))
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
    let next_id = shape.ids.iter().copied().max().unwrap_or(0) + 1;

    // Second pass: splice.
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut column_position = 0usize;
    let mut formula_content: Option<usize> = None;
    let mut columns_head_end: Option<usize> = None;
    let mut column_prefix = String::new();
    // Where each new column's element goes: before the existing column at
    // its position. All new columns share one position.
    let mut inserted_new = false;

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let head_end = reader.buffer_position() as usize;
                match local(&name) {
                    b"table" => {
                        let mut updates = vec![("ref", Some(range_text(mapped)))];
                        if totals_row_count != shape.totals_row_count {
                            updates.push(("totalsRowCount", None));
                            updates.push(("totalsRowShown", Some("0".to_string())));
                        }
                        splices.push(Splice {
                            start,
                            end: head_end,
                            text: rewrite(element, &updates, empty)?,
                        });
                    }
                    b"autoFilter" => {
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        splices.push(Splice {
                            start,
                            end,
                            text: crate::filter::shift_auto_filter(
                                &xml[start..end],
                                change,
                                filter_range,
                            )?,
                        });
                    }
                    b"sortState" => {
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        splices.push(Splice {
                            start,
                            end,
                            text: crate::filter::shift_sort_state(&xml[start..end], change)?
                                .unwrap_or_default(),
                        });
                    }
                    b"tableColumns" => {
                        splices.push(Splice {
                            start,
                            end: head_end,
                            text: rewrite(
                                element,
                                &[("count", Some(column_count.to_string()))],
                                empty,
                            )?,
                        });
                        columns_head_end = Some(head_end);
                        column_prefix = crate::patch::namespace_prefix(&name);
                    }
                    b"tableColumn" => {
                        column_prefix = crate::patch::namespace_prefix(&name);
                        if !inserted_new {
                            if let Some((position, _)) = new_columns.first() {
                                if *position == column_position {
                                    splices.push(Splice {
                                        start,
                                        end: start,
                                        text: new_column_xml(&new_columns, next_id, &column_prefix),
                                    });
                                    inserted_new = true;
                                }
                            }
                        }
                        if dropped.contains(&column_position) {
                            if !empty {
                                reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                            }
                            let end = reader.buffer_position() as usize;
                            splices.push(Splice {
                                start,
                                end,
                                text: String::new(),
                            });
                        }
                        column_position += 1;
                    }
                    b"calculatedColumnFormula" | b"totalsRowFormula" if !empty => {
                        formula_content = Some(head_end);
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                b"calculatedColumnFormula" | b"totalsRowFormula" => {
                    if let Some(content_start) = formula_content.take() {
                        let raw = &xml[content_start..start];
                        let formula =
                            quick_xml::escape::unescape(raw).map_err(quick_xml::Error::from)?;
                        let shifted = shift_formula(&formula, sheet_name, change);
                        if shifted != formula {
                            splices.push(Splice {
                                start: content_start,
                                end: start,
                                text: escape(&shifted),
                            });
                        }
                    }
                }
                b"tableColumns" => {
                    if !inserted_new && !new_columns.is_empty() {
                        splices.push(Splice {
                            start,
                            end: start,
                            text: new_column_xml(&new_columns, next_id, &column_prefix),
                        });
                        inserted_new = true;
                    }
                    let _ = columns_head_end.take();
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(ShiftedTable {
        xml: Some(apply(xml, splices)),
        header_cells,
    })
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
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut column_position = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"tableColumn" =>
            {
                let empty = matches!(event, Event::Empty(_));
                if column_position == position {
                    let head_end = reader.buffer_position() as usize;
                    splices.push(Splice {
                        start,
                        end: head_end,
                        text: rewrite(element, &[("name", Some(name.to_string()))], empty)?,
                    });
                }
                column_position += 1;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The sheet part without the `<tablePart>` entries whose `r:id` is in
/// `ids`, its `<tableParts count>` kept true — and dropped when nothing
/// is left in it.
pub fn remove_table_parts(xml: &str, ids: &[String]) -> Result<String> {
    if ids.is_empty() {
        return Ok(xml.to_string());
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut list: Option<(usize, usize, usize)> = None; // start, head end, kept
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                match local(element.name().as_ref()) {
                    b"tableParts" if !empty => {
                        list = Some((start, reader.buffer_position() as usize, 0));
                    }
                    b"tablePart" => {
                        let id = attribute(element, b"id")?.unwrap_or_default();
                        if ids.contains(&id) {
                            if !empty {
                                let name = element.name().as_ref().to_vec();
                                reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                            }
                            let end = reader.buffer_position() as usize;
                            splices.push(Splice {
                                start,
                                end,
                                text: String::new(),
                            });
                        } else if let Some(list) = list.as_mut() {
                            list.2 += 1;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"tableParts" => {
                if let Some((list_start, head_end, kept)) = list.take() {
                    let end = reader.buffer_position() as usize;
                    if kept == 0 {
                        splices.push(Splice {
                            start: list_start,
                            end,
                            text: String::new(),
                        });
                    } else {
                        let head = &xml[list_start..head_end];
                        if head.contains("count=") {
                            splices.push(Splice {
                                start: list_start,
                                end: head_end,
                                text: rewrite_head_text(head, "count", &kept.to_string())?,
                            });
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}
