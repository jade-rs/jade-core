//! Formatting complete axes without creating every cell in the worksheet.
use crate::{
    add_cell_format,
    patch::{apply, attribute, element_head_with_attribute, local, namespace_prefix, Splice},
    CellEdit, FormatChange, NewValue, Result, WriteError,
};
use jade_xlsx_reader::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    CellRange, CellRef, Workbook,
};
use quick_xml::{events::Event, name::QName, Reader};
use std::collections::BTreeMap;

pub const AXIS_FORMAT_LIMIT: u64 = 1_000_000;

pub struct AxisFormatResult {
    pub bytes: Vec<u8>,
    /// Stored cells and intersections whose individual styles were patched.
    pub cells: Vec<CellRef>,
}

/// The range must contain complete rows, complete columns or the whole sheet.
/// Defaults carry the change to future cells; explicit styles and intersections
/// retain every property the change does not name.
pub fn format_axes(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    range: CellRange,
    change: &FormatChange,
) -> Result<AxisFormatResult> {
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(sheet_name))
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let full_columns = range.start.column == 0 && range.end.column == MAX_COLUMN_COUNT - 1;
    let full_rows = range.start.row == 0 && range.end.row == MAX_ROW_COUNT - 1;
    if (!full_columns && !full_rows)
        || range.end.column >= MAX_COLUMN_COUNT
        || range.end.row >= MAX_ROW_COUNT
        || range.start.column > range.end.column
        || range.start.row > range.end.row
    {
        return Err(WriteError::CellOutOfBounds {
            column: range.end.column,
            row: range.end.row,
        });
    }
    let whole_sheet = full_columns && full_rows;
    let row_defaults = full_columns && !whole_sheet;
    let row_count = u64::from(range.end.row - range.start.row + 1);
    let column_count = u64::from(range.end.column - range.start.column + 1);
    let styled_columns: Vec<u32> = (0..MAX_COLUMN_COUNT)
        .filter(|column| {
            sheet
                .column_span(*column)
                .and_then(|span| span.style_index)
                .is_some_and(|style| style != 0)
        })
        .collect();
    let custom_rows: Vec<_> = sheet
        .rows
        .iter()
        .filter(|row| row.custom_format && row.style_index.is_some())
        .collect();
    let intersections = if row_defaults {
        row_count * styled_columns.len() as u64
    } else if !whole_sheet {
        column_count * custom_rows.len() as u64
    } else {
        0
    };
    let stored = sheet
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| range.contains(cell.reference))
        .count() as u64;
    let metadata = if row_defaults {
        row_count
    } else {
        column_count + custom_rows.len() as u64
    };
    if stored + metadata + intersections > AXIS_FORMAT_LIMIT {
        return Err(WriteError::AxisFormatLimit {
            limit: AXIS_FORMAT_LIMIT,
        });
    }
    let (styles_part, mut styles) = crate::styles_part(original)?;
    let mut derived = BTreeMap::new();
    let mut derive = |base: u32| -> Result<u32> {
        if let Some(index) = derived.get(&base) {
            return Ok(*index);
        }
        let (xml, index) = add_cell_format(&styles, Some(base), change)?;
        styles = xml;
        derived.insert(base, index);
        Ok(index)
    };
    let mut edits = BTreeMap::new();
    for row in &sheet.rows {
        for cell in row
            .cells
            .iter()
            .filter(|cell| range.contains(cell.reference))
        {
            let base = sheet
                .effective_style_index_in_row(cell.reference, cell.style_index, Some(row))
                .unwrap_or(0);
            edits.insert(cell.reference, derive(base)?);
        }
    }
    let mut rows = BTreeMap::new();
    let mut columns = BTreeMap::new();
    if row_defaults {
        let original_rows: BTreeMap<_, _> = sheet.rows.iter().map(|row| (row.index, row)).collect();
        for row_index in range.start.row..=range.end.row {
            let row = original_rows.get(&row_index).copied();
            let explicit = row
                .filter(|row| row.custom_format)
                .and_then(|row| row.style_index);
            rows.insert(row_index, derive(explicit.unwrap_or(0))?);
            if explicit.is_none() {
                for column in &styled_columns {
                    let at = CellRef::new(*column, row_index);
                    if let std::collections::btree_map::Entry::Vacant(entry) = edits.entry(at) {
                        entry.insert(derive(
                            sheet
                                .column_span(*column)
                                .and_then(|span| span.style_index)
                                .unwrap_or(0),
                        )?);
                    }
                }
            }
        }
    } else {
        for column in range.start.column..=range.end.column {
            columns.insert(
                column,
                derive(
                    sheet
                        .column_span(column)
                        .and_then(|span| span.style_index)
                        .unwrap_or(0),
                )?,
            );
        }
        for row in custom_rows {
            let style = derive(row.style_index.unwrap_or(0))?;
            if whole_sheet {
                rows.insert(row.index, style);
            } else {
                for column in range.start.column..=range.end.column {
                    edits
                        .entry(CellRef::new(column, row.index))
                        .or_insert(style);
                }
            }
        }
    }
    let cells: Vec<_> = edits.keys().copied().collect();
    let edits: Vec<_> = edits
        .into_iter()
        .map(|(reference, style_index)| CellEdit {
            reference,
            value: NewValue::Unchanged,
            style_index: Some(style_index),
        })
        .collect();
    let package = jade_opc::OpcPackage::open(original)?;
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let xml = crate::patch_sheet(&xml, &edits)?;
    let xml = patch_defaults(&xml, rows, columns)?;
    let bytes = crate::write_xlsx_with_parts(
        original,
        Some((&styles_part, &styles)),
        &[],
        &[(sheet.part_name.clone(), xml)],
    )?;
    Ok(AxisFormatResult { bytes, cells })
}

fn set_attributes(head: &str, values: &[(&[u8], String)], empty: bool) -> Result<String> {
    let mut head = head.to_string();
    for (key, value) in values {
        let mut reader = Reader::from_str(&head);
        if let Event::Start(element) | Event::Empty(element) = reader.read_event()? {
            head = element_head_with_attribute(&element, key, value, empty)?;
        }
    }
    Ok(head)
}

fn new_rows(rows: &mut BTreeMap<u32, u32>, before: u32, prefix: &str) -> String {
    let mut out = String::new();
    while rows.first_key_value().is_some_and(|(row, _)| *row < before) {
        let (row, style) = rows.pop_first().expect("entry exists");
        out.push_str(&format!(
            "<{prefix}row r=\"{}\" s=\"{style}\" customFormat=\"1\"/>",
            row + 1
        ));
    }
    out
}

pub(crate) fn patch_defaults(
    xml: &str,
    mut rows: BTreeMap<u32, u32>,
    columns: BTreeMap<u32, u32>,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut splices = Vec::new();
    let mut prefix = String::new();
    let mut next_row = 0;
    let mut in_data = false;
    let mut saw_columns = false;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let end = reader.buffer_position() as usize;
                match local(&name) {
                    b"cols" if !columns.is_empty() => {
                        saw_columns = true;
                        if !empty {
                            reader.read_to_end(QName(&name))?;
                        }
                        let finish = reader.buffer_position() as usize;
                        splices.push(Splice {
                            start,
                            end: finish,
                            text: patch_columns(&xml[start..finish], &columns)?,
                        });
                    }
                    b"sheetData" => {
                        prefix = namespace_prefix(&name);
                        if !columns.is_empty() && !saw_columns {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: format!(
                                    "<{prefix}cols>{}</{prefix}cols>",
                                    column_entries(&columns, &prefix)
                                ),
                            });
                        }
                        if empty && !rows.is_empty() {
                            splices.push(Splice {
                                start,
                                end,
                                text: format!(
                                    "<{prefix}sheetData>{}</{prefix}sheetData>",
                                    new_rows(&mut rows, MAX_ROW_COUNT, &prefix)
                                ),
                            });
                        } else {
                            in_data = !empty;
                        }
                    }
                    b"row" if in_data => {
                        let row = attribute(element, b"r")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .and_then(|value| value.checked_sub(1))
                            .unwrap_or(next_row);
                        next_row = row + 1;
                        let inserted = new_rows(&mut rows, row, &prefix);
                        if !inserted.is_empty() {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: inserted,
                            });
                        }
                        if let Some(style) = rows.remove(&row) {
                            splices.push(Splice {
                                start,
                                end,
                                text: set_attributes(
                                    &xml[start..end],
                                    &[(b"s", style.to_string()), (b"customFormat", "1".into())],
                                    empty,
                                )?,
                            });
                        }
                        if !empty {
                            reader.read_to_end(QName(&name))?;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                in_data = false;
                let inserted = new_rows(&mut rows, MAX_ROW_COUNT, &prefix);
                if !inserted.is_empty() {
                    splices.push(Splice {
                        start,
                        end: start,
                        text: inserted,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, splices))
}

fn column_entries(columns: &BTreeMap<u32, u32>, prefix: &str) -> String {
    let mut out = String::new();
    let mut entries = columns.iter().peekable();
    while let Some((&first, &style)) = entries.next() {
        let mut last = first;
        while entries
            .peek()
            .is_some_and(|(column, next_style)| **column == last + 1 && **next_style == style)
        {
            last += 1;
            entries.next();
        }
        out.push_str(&format!(
            "<{prefix}col min=\"{}\" max=\"{}\" style=\"{style}\"/>",
            first + 1,
            last + 1
        ));
    }
    out
}

fn patch_columns(xml: &str, columns: &BTreeMap<u32, u32>) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut remaining = columns.clone();
    let mut entries: Vec<(u32, String)> = Vec::new();
    let mut prefix = String::new();
    let mut root_head = String::new();
    let mut suffix = String::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let name = element.name().as_ref().to_vec();
                let empty = matches!(event, Event::Empty(_));
                let end = reader.buffer_position() as usize;
                if local(&name) == b"cols" {
                    prefix = namespace_prefix(&name);
                    root_head = xml[start..end]
                        .trim_end()
                        .trim_end_matches("/>")
                        .trim_end_matches('>')
                        .to_string();
                } else if local(&name) == b"col" {
                    let first = attribute(element, b"min")?
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(1)
                        .saturating_sub(1);
                    let last = attribute(element, b"max")?
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(first + 1)
                        .saturating_sub(1)
                        .min(MAX_COLUMN_COUNT - 1);
                    if !empty {
                        reader.read_to_end(QName(&name))?;
                    }
                    let finish = reader.buffer_position() as usize;
                    let mut at = first;
                    while at <= last {
                        let style = columns.get(&at).copied();
                        let mut to = at;
                        while to < last && columns.get(&(to + 1)).copied() == style {
                            to += 1;
                        }
                        let mut attrs = vec![
                            (b"min".as_slice(), (at + 1).to_string()),
                            (b"max".as_slice(), (to + 1).to_string()),
                        ];
                        if let Some(style) = style {
                            attrs.push((b"style", style.to_string()));
                        }
                        let head = set_attributes(&xml[start..end], &attrs, empty)?;
                        entries.push((at, format!("{head}{}", &xml[end..finish])));
                        for column in at..=to {
                            remaining.remove(&column);
                        }
                        at = to + 1;
                    }
                } else {
                    if !empty {
                        reader.read_to_end(QName(&name))?;
                    }
                    suffix.push_str(&xml[start..reader.buffer_position() as usize]);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut gaps = remaining.iter().peekable();
    while let Some((&first, &style)) = gaps.next() {
        let mut last = first;
        while gaps
            .peek()
            .is_some_and(|(column, next_style)| **column == last + 1 && **next_style == style)
        {
            last += 1;
            gaps.next();
        }
        entries.push((
            first,
            format!(
                "<{prefix}col min=\"{}\" max=\"{}\" style=\"{style}\"/>",
                first + 1,
                last + 1
            ),
        ));
    }
    entries.sort_by_key(|(first, _)| *first);
    Ok(format!(
        "{root_head}>{}{suffix}</{prefix}cols>",
        entries
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<String>()
    ))
}
