//! Splicing cells into a worksheet part.
//!
//! The part is edited as text at byte ranges rather than reparsed and
//! reserialised, so everything the edits do not touch — attribute order,
//! whitespace, elements this crate has never heard of — comes back exactly as
//! it was.

use std::collections::{BTreeMap, HashMap, HashSet};

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use crate::{CellEdit, NewValue, Result, WriteError};
use jade_xlsx_reader::{CellRange, CellRef};

/// A change to make to the original text: replace `[start, end)` with `text`.
/// A zero-width range is an insertion.
pub(crate) struct Splice {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) text: String,
}

/// Applies `edits` to a worksheet part.
pub fn patch_sheet(xml: &str, edits: &[CellEdit]) -> Result<String> {
    for edit in edits {
        if edit.reference.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
            || edit.reference.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        {
            return Err(WriteError::CellOutOfBounds {
                column: edit.reference.column,
                row: edit.reference.row,
            });
        }
        let number = match &edit.value {
            NewValue::Number(number)
            | NewValue::FormulaCached {
                cached: jade_xlsx_reader::CellValue::Number(number),
                ..
            } => Some(number),
            _ => None,
        };
        if number.is_some_and(|number| !number.is_finite()) {
            return Err(WriteError::NonFiniteNumber {
                column: edit.reference.column,
                row: edit.reference.row,
            });
        }
        if let NewValue::Formula(formula) | NewValue::FormulaCached { formula, .. } = &edit.value {
            if formula.trim().is_empty() {
                return Err(WriteError::EmptyFormula {
                    column: edit.reference.column,
                    row: edit.reference.row,
                });
            }
        }
        // Text goes through the `_xHHHH_` escape below, so any character can
        // be written; a formula has no such escape, so XML's own limits apply.
        if let NewValue::Formula(text) | NewValue::FormulaCached { formula: text, .. } = &edit.value
        {
            if let Some(character) = text
                .chars()
                .find(|character| !is_xml_10_character(*character))
            {
                return Err(WriteError::InvalidXmlCharacter {
                    column: edit.reference.column,
                    row: edit.reference.row,
                    codepoint: character as u32,
                });
            }
        }
    }

    // Replacing one member of a shared formula leaves the other members
    // pointing at a group that no longer describes them. Expand only groups
    // touched by the edit, keeping each cell's cached <v> unchanged.
    let expanded = detach_edited_shared_formulas(xml, edits)?;
    // A sheet whose rows or cells are already out of order — some writers
    // leave them so — is put in order first: Excel drops such a sheet's
    // cells when it repairs the file, and the splices below assume order.
    let ordered = order_sheet_data(&expanded)?;
    let xml = ordered.as_deref().unwrap_or(expanded.as_str());

    // Rows and columns in order, because cells and rows have to stay sorted:
    // Excel reads a sheet whose rows are out of order as corrupt.
    let mut pending: BTreeMap<u32, BTreeMap<u32, &CellEdit>> = BTreeMap::new();
    for edit in edits {
        pending
            .entry(edit.reference.row)
            .or_default()
            .insert(edit.reference.column, edit);
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut splices: Vec<Splice> = Vec::new();
    let mut current_row: Option<(u32, BTreeMap<u32, &CellEdit>, String, u32)> = None;
    let mut in_sheet_data = false;
    let mut saw_sheet_data = false;
    let mut sheet_data_prefix = String::new();
    let mut next_row_index = 0u32;
    let mut buf = Vec::new();

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"dimension" =>
            {
                let empty = matches!(event, Event::Empty(_));
                if let Some(original) =
                    attribute(element, b"ref")?.and_then(|value| CellRange::parse(&value).ok())
                {
                    let expanded = expand_range(original, edits);
                    if expanded != original {
                        splices.push(Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: element_with_ref(element, expanded, empty)?,
                        });
                    }
                }
            }
            Event::Start(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                in_sheet_data = true;
                saw_sheet_data = true;
                sheet_data_prefix = namespace_prefix(element.name().as_ref());
            }
            Event::Empty(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                // An empty sheet: everything goes inside a `<sheetData>` that
                // has to be opened first.
                saw_sheet_data = true;
                let end = reader.buffer_position() as usize;
                let prefix = namespace_prefix(element.name().as_ref());
                let mut text = element_head(element)?;
                for (row, cells) in std::mem::take(&mut pending) {
                    text.push_str(&row_xml(row, &cells, &prefix));
                }
                text.push_str("</");
                text.push_str(&qualified_name(element.name().as_ref()));
                text.push('>');
                splices.push(Splice { start, end, text });
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if in_sheet_data && local(element.name().as_ref()) == b"row" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let row_prefix = namespace_prefix(element.name().as_ref());
                let index = attribute(element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value > 0)
                    .map(|value| value - 1)
                    .unwrap_or(next_row_index);
                next_row_index = index.saturating_add(1);

                // Rows the file does not have, that sort before this one.
                let mut text = String::new();
                while let Some(first) = pending.keys().next().copied() {
                    if first >= index {
                        break;
                    }
                    let cells = pending.remove(&first).expect("a row");
                    text.push_str(&row_xml(first, &cells, &row_prefix));
                }
                if !text.is_empty() {
                    splices.push(Splice {
                        start,
                        end: start,
                        text,
                    });
                }

                let cells = pending.remove(&index).unwrap_or_default();
                if empty {
                    // `<row r="3"/>` with cells to add: reopen it.
                    if !cells.is_empty() {
                        let end = reader.buffer_position() as usize;
                        let mut text = element_head(element)?;
                        for (column, edit) in &cells {
                            text.push_str(&cell_xml(
                                CellRef::new(*column, index),
                                edit,
                                None,
                                &row_prefix,
                            ));
                        }
                        text.push_str("</");
                        text.push_str(&qualified_name(element.name().as_ref()));
                        text.push('>');
                        splices.push(Splice { start, end, text });
                    }
                } else {
                    current_row = Some((index, cells, row_prefix, 0));
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if current_row.is_some() && local(element.name().as_ref()) == b"c" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let name_bytes = element.name().as_ref().to_vec();
                let cell_prefix = namespace_prefix(&name_bytes);
                let (row, cells, row_prefix, next_column) = current_row.as_mut().expect("a row");
                let row = *row;
                let reference = match attribute(element, b"r")? {
                    Some(value) => {
                        CellRef::parse(&value).unwrap_or_else(|_| CellRef::new(*next_column, row))
                    }
                    None => CellRef::new(*next_column, row),
                };
                *next_column = reference.column.saturating_add(1);

                // Cells that sort before this one.
                let mut text = String::new();
                while let Some(first) = cells.keys().next().copied() {
                    if first >= reference.column {
                        break;
                    }
                    let edit = cells.remove(&first).expect("a cell");
                    text.push_str(&cell_xml(CellRef::new(first, row), edit, None, row_prefix));
                }
                if !text.is_empty() {
                    splices.push(Splice {
                        start,
                        end: start,
                        text,
                    });
                }

                if let Some(edit) = cells.remove(&reference.column) {
                    if matches!(edit.value, NewValue::Unchanged) {
                        // Only the opening tag changes; the content is left
                        // to the reader, which walks through it untouched.
                        if let Some(style) = edit.style_index {
                            splices.push(Splice {
                                start,
                                end: reader.buffer_position() as usize,
                                text: element_head_with_style(element, &style.to_string(), empty)?,
                            });
                        }
                    } else {
                        // The style index moves to the replacement, so a
                        // value written into a formatted cell keeps its
                        // format unless the edit brings its own.
                        let style = attribute(element, b"s")?;
                        if !empty {
                            reader.read_to_end_into(QName(&name_bytes), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        splices.push(Splice {
                            start,
                            end,
                            text: cell_xml(reference, edit, style.as_deref(), &cell_prefix),
                        });
                    }
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"row" => {
                if let Some((row, cells, prefix, _)) = current_row.take() {
                    let mut text = String::new();
                    for (column, edit) in &cells {
                        text.push_str(&cell_xml(CellRef::new(*column, row), edit, None, &prefix));
                    }
                    if !text.is_empty() {
                        splices.push(Splice {
                            start,
                            end: start,
                            text,
                        });
                    }
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                in_sheet_data = false;
                let mut text = String::new();
                for (row, cells) in std::mem::take(&mut pending) {
                    text.push_str(&row_xml(row, &cells, &sheet_data_prefix));
                }
                if !text.is_empty() {
                    splices.push(Splice {
                        start,
                        end: start,
                        text,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !saw_sheet_data {
        return Err(WriteError::NoSheetData("the worksheet".to_string()));
    }

    Ok(apply(xml, splices))
}

struct SharedFormulaTag {
    reference: CellRef,
    index: u32,
    start: usize,
    end: usize,
    prefix: String,
}

/// Whether `keys` rise strictly.
fn ascending(keys: impl Iterator<Item = Option<u32>>) -> bool {
    let mut last = None;
    for key in keys {
        if last.is_some() && key <= last {
            return false;
        }
        last = key;
    }
    true
}

/// An element of `<sheetData>` or a `<row>`: where it is in the text, and
/// the row or column its `r` names.
struct Span {
    start: usize,
    end: usize,
    key: Option<u32>,
}

/// The sheet with `<sheetData>`'s rows sorted by row and each row's cells by
/// column, or `None` when they already are (or cannot be sorted safely: an
/// element without an `r`, or anything else among them).
pub(crate) fn order_sheet_data(xml: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth_in_data: Option<usize> = None;
    let mut data_span: Option<(usize, usize)> = None;
    let mut rows: Vec<(Span, Vec<Span>, usize, usize)> = Vec::new();
    let mut row_inner_start = 0;
    let mut cell_start = 0;
    let mut cell_key = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match (&event, depth_in_data) {
            (Event::Eof, _) => break,
            (Event::Start(element), None) if local(element.name().as_ref()) == b"sheetData" => {
                depth_in_data = Some(0);
                data_span = Some((end, end));
            }
            (Event::End(element), Some(0)) if local(element.name().as_ref()) == b"sheetData" => {
                if let Some((inner, _)) = data_span {
                    data_span = Some((inner, start));
                }
                depth_in_data = None;
            }
            (Event::Start(element) | Event::Empty(element), Some(0)) => {
                if local(element.name().as_ref()) != b"row" {
                    return Ok(None);
                }
                let key = attribute(element, b"r")?.and_then(|value| value.parse().ok());
                if key.is_none() {
                    return Ok(None);
                }
                let span = Span { start, end, key };
                if matches!(event, Event::Start(_)) {
                    depth_in_data = Some(1);
                    row_inner_start = end;
                }
                rows.push((span, Vec::new(), end, end));
            }
            (Event::End(element), Some(1)) if local(element.name().as_ref()) == b"row" => {
                if let Some(row) = rows.last_mut() {
                    row.0.end = end;
                    row.2 = row_inner_start;
                    row.3 = start;
                }
                depth_in_data = Some(0);
            }
            (Event::Start(element) | Event::Empty(element), Some(1)) => {
                if local(element.name().as_ref()) != b"c" {
                    return Ok(None);
                }
                let key = attribute(element, b"r")?
                    .and_then(|value| CellRange::parse(&value).ok())
                    .map(|range| range.start.column);
                if key.is_none() {
                    return Ok(None);
                }
                if matches!(event, Event::Empty(_)) {
                    if let Some(row) = rows.last_mut() {
                        row.1.push(Span { start, end, key });
                    }
                } else {
                    cell_start = start;
                    cell_key = key;
                    depth_in_data = Some(2);
                }
            }
            (Event::End(element), Some(2)) if local(element.name().as_ref()) == b"c" => {
                if let Some(row) = rows.last_mut() {
                    row.1.push(Span {
                        start: cell_start,
                        end,
                        key: cell_key,
                    });
                }
                depth_in_data = Some(1);
            }
            (Event::Start(_), Some(depth)) if depth >= 2 => depth_in_data = Some(depth + 1),
            (Event::End(_), Some(depth)) if depth >= 3 => depth_in_data = Some(depth - 1),
            _ => {}
        }
        buf.clear();
    }
    let Some((inner_start, inner_end)) = data_span else {
        return Ok(None);
    };
    let rows_sorted = ascending(rows.iter().map(|row| row.0.key));
    let cells_sorted = rows
        .iter()
        .all(|row| ascending(row.1.iter().map(|cell| cell.key)));
    if rows_sorted && cells_sorted {
        return Ok(None);
    }
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&index| rows[index].0.key);
    if order
        .windows(2)
        .any(|pair| rows[pair[0]].0.key == rows[pair[1]].0.key)
    {
        // Two rows with one number: which to keep is not this pass's call.
        return Ok(None);
    }
    let mut text = String::with_capacity(xml.len());
    text.push_str(&xml[..inner_start]);
    for index in order {
        let (row, cells, inner, inner_close) = &rows[index];
        let mut cells: Vec<&Span> = cells.iter().collect();
        if ascending(cells.iter().map(|cell| cell.key)) {
            text.push_str(&xml[row.start..row.end]);
            continue;
        }
        cells.sort_by_key(|cell| cell.key);
        text.push_str(&xml[row.start..*inner]);
        for cell in cells {
            text.push_str(&xml[cell.start..cell.end]);
        }
        text.push_str(&xml[*inner_close..row.end]);
    }
    text.push_str(&xml[inner_end..]);
    Ok(Some(text))
}

pub(crate) fn detach_edited_shared_formulas(xml: &str, edits: &[CellEdit]) -> Result<String> {
    if edits.is_empty() || !xml.contains("shared") {
        return Ok(xml.to_string());
    }

    let edited: HashSet<CellRef> = edits
        .iter()
        .filter(|edit| !matches!(edit.value, NewValue::Unchanged))
        .map(|edit| edit.reference)
        .collect();
    if edited.is_empty() {
        return Ok(xml.to_string());
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut tags = Vec::new();
    let mut pending_formula: Option<SharedFormulaTag> = None;
    let mut next_row = 0u32;
    let mut current_row = None;
    let mut next_column = 0u32;
    let mut current_cell = None;
    let mut buf = Vec::new();

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"row" =>
            {
                let row = attribute(element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value > 0)
                    .map(|value| value - 1)
                    .unwrap_or(next_row);
                next_row = row.saturating_add(1);
                current_row = Some(row);
                next_column = 0;
                if matches!(event, Event::Empty(_)) {
                    current_row = None;
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if current_row.is_some() && local(element.name().as_ref()) == b"c" =>
            {
                let row = current_row.expect("inside row");
                let reference = attribute(element, b"r")?
                    .and_then(|value| CellRef::parse(&value).ok())
                    .unwrap_or_else(|| CellRef::new(next_column, row));
                next_column = reference.column.saturating_add(1);
                current_cell = Some(reference);
                if matches!(event, Event::Empty(_)) {
                    current_cell = None;
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if current_cell.is_some() && local(element.name().as_ref()) == b"f" =>
            {
                if attribute(element, b"t")?.as_deref() == Some("shared") {
                    if let Some(index) =
                        attribute(element, b"si")?.and_then(|value| value.parse::<u32>().ok())
                    {
                        let tag = SharedFormulaTag {
                            reference: current_cell.expect("inside cell"),
                            index,
                            start,
                            end: reader.buffer_position() as usize,
                            prefix: namespace_prefix(element.name().as_ref()),
                        };
                        if matches!(event, Event::Empty(_)) {
                            tags.push(tag);
                        } else {
                            pending_formula = Some(tag);
                        }
                    }
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"f" => {
                if let Some(mut tag) = pending_formula.take() {
                    tag.end = reader.buffer_position() as usize;
                    tags.push(tag);
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"c" => {
                current_cell = None;
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"row" => {
                current_row = None;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let affected: HashSet<u32> = tags
        .iter()
        .filter(|tag| edited.contains(&tag.reference))
        .map(|tag| tag.index)
        .collect();
    if affected.is_empty() {
        return Ok(xml.to_string());
    }

    let sheet = jade_xlsx_reader::sheet::parse_sheet("", "", xml, &[])?;
    let formulas: HashMap<CellRef, &str> = sheet
        .rows
        .iter()
        .flat_map(|row| row.cells.iter())
        .filter_map(|cell| {
            cell.formula
                .as_deref()
                .map(|formula| (cell.reference, formula))
        })
        .collect();
    let splices = tags
        .into_iter()
        .filter(|tag| affected.contains(&tag.index))
        .filter_map(|tag| {
            let formula = formulas.get(&tag.reference)?;
            Some(Splice {
                start: tag.start,
                end: tag.end,
                text: format!("<{0}f>{1}</{0}f>", tag.prefix, escape(formula)),
            })
        })
        .collect();
    Ok(apply(xml, splices))
}

fn expand_range(range: CellRange, edits: &[CellEdit]) -> CellRange {
    let mut start = CellRef::new(
        range.start.column.min(range.end.column),
        range.start.row.min(range.end.row),
    );
    let mut end = CellRef::new(
        range.start.column.max(range.end.column),
        range.start.row.max(range.end.row),
    );
    for edit in edits {
        start.column = start.column.min(edit.reference.column);
        start.row = start.row.min(edit.reference.row);
        end.column = end.column.max(edit.reference.column);
        end.row = end.row.max(edit.reference.row);
    }
    CellRange { start, end }
}

fn element_with_ref(
    element: &quick_xml::events::BytesStart<'_>,
    range: CellRange,
    empty: bool,
) -> Result<String> {
    let mut out = String::from("<");
    out.push_str(&String::from_utf8_lossy(element.name().as_ref()));
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local(attribute.key.as_ref()) == b"ref" {
            continue;
        }
        out.push(' ');
        out.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
        out.push_str("=\"");
        out.push_str(&escape(&attribute.unescape_value()?));
        out.push('"');
    }
    out.push_str(" ref=\"");
    out.push_str(&format_dimension(range));
    out.push_str(if empty { "\"/>" } else { "\">" });
    Ok(out)
}

fn format_dimension(range: CellRange) -> String {
    let start = format!(
        "{}{}",
        column_label(range.start.column),
        range.start.row + 1
    );
    if range.start == range.end {
        return start;
    }
    format!(
        "{start}:{}{}",
        column_label(range.end.column),
        range.end.row + 1
    )
}

fn is_xml_10_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}'
            | '\u{20}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
    )
}

pub(crate) fn apply(xml: &str, mut splices: Vec<Splice>) -> String {
    splices.sort_by_key(|splice| (splice.start, splice.end));
    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0usize;
    for splice in splices {
        if splice.start < cursor {
            continue;
        }
        out.push_str(&xml[cursor..splice.start]);
        out.push_str(&splice.text);
        cursor = splice.end.max(splice.start);
    }
    out.push_str(&xml[cursor..]);
    out
}

/// `<row r="3">…</row>` for a row the file does not have.
fn row_xml(row: u32, cells: &BTreeMap<u32, &CellEdit>, prefix: &str) -> String {
    let mut out = format!("<{prefix}row r=\"{}\">", row + 1);
    for (column, edit) in cells {
        out.push_str(&cell_xml(CellRef::new(*column, row), edit, None, prefix));
    }
    out.push_str(&format!("</{prefix}row>"));
    out
}

/// A cell element for `edit`. `existing_style` is the `s` of the cell being
/// replaced, which the edit's own style index overrides.
fn cell_xml(
    reference: CellRef,
    edit: &CellEdit,
    existing_style: Option<&str>,
    prefix: &str,
) -> String {
    let reference = format!("{}{}", column_label(reference.column), reference.row + 1);
    let style = match edit.style_index {
        Some(index) => format!(" s=\"{index}\""),
        None => existing_style
            .map(|style| format!(" s=\"{style}\""))
            .unwrap_or_default(),
    };
    match &edit.value {
        NewValue::Remove => String::new(),
        NewValue::Error(value) => format!("<{prefix}c r=\"{reference}\"{style} t=\"e\"><{prefix}v>{}</{prefix}v></{prefix}c>", escape(value)),
        NewValue::FormulaCached { formula, cached } => {
            use jade_xlsx_reader::CellValue;
            let (kind, value) = match cached {
                CellValue::Empty => ("", String::new()),
                CellValue::Number(value) => ("", format!("<{prefix}v>{}</{prefix}v>", number_text(*value))),
                CellValue::Text(value) => (" t=\"str\"", format!("<{prefix}v>{}</{prefix}v>", escape(&jade_xlsx_reader::xstring::encode_xstring(value)))),
                CellValue::Bool(value) => (" t=\"b\"", format!("<{prefix}v>{}</{prefix}v>", u8::from(*value))),
                CellValue::Error(value) => (" t=\"e\"", format!("<{prefix}v>{}</{prefix}v>", escape(value))),
            };
            format!("<{prefix}c r=\"{reference}\"{style}{kind}><{prefix}f>{}</{prefix}f>{value}</{prefix}c>", escape(formula))
        }
        NewValue::Blank | NewValue::Unchanged => format!("<{prefix}c r=\"{reference}\"{style}/>"),
        NewValue::Number(number) => format!(
            "<{prefix}c r=\"{reference}\"{style}><{prefix}v>{}</{prefix}v></{prefix}c>",
            number_text(*number)
        ),
        NewValue::Bool(flag) => format!(
            "<{prefix}c r=\"{reference}\"{style} t=\"b\"><{prefix}v>{}</{prefix}v></{prefix}c>",
            u8::from(*flag)
        ),
        NewValue::Formula(formula) => format!(
            "<{prefix}c r=\"{reference}\"{style}><{prefix}f>{}</{prefix}f></{prefix}c>",
            escape(formula)
        ),
        // Characters XML cannot carry — a carriage return above all — go
        // through SpreadsheetML's `_xHHHH_` escape first, then XML's own.
        NewValue::Text(text) => format!(
            "<{prefix}c r=\"{reference}\"{style} t=\"inlineStr\"><{prefix}is><{prefix}t xml:space=\"preserve\">{}</{prefix}t></{prefix}is></{prefix}c>",
            escape(&jade_xlsx_reader::xstring::encode_xstring(text))
        ),
    }
}

/// A number the way a sheet part writes one: no exponent for ordinary
/// magnitudes, and no trailing `.0`.
pub(crate) fn number_text(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let mut text = format!("{value}");
    if text.contains('e') || text.contains('E') {
        text = format!("{value:.10}");
        let trimmed = text.trim_end_matches('0').trim_end_matches('.');
        return trimmed.to_string();
    }
    text
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// The opening tag of an element with its `s` attribute set to `style` —
/// replaced where there is one, appended otherwise — self-closing when the
/// element was.
fn element_head_with_style(
    element: &quick_xml::events::BytesStart<'_>,
    style: &str,
    empty: bool,
) -> Result<String> {
    element_head_with_attribute(element, b"s", style, empty)
}

/// The opening tag of an element with one attribute set to `value` —
/// replaced where there is one, appended otherwise — self-closing when the
/// element was.
pub(crate) fn element_head_with_attribute(
    element: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
    value: &str,
    empty: bool,
) -> Result<String> {
    let mut head = String::from("<");
    head.push_str(&String::from_utf8_lossy(element.name().as_ref()));
    let mut replaced = false;
    for attribute in element.attributes() {
        let attribute = attribute?;
        head.push(' ');
        head.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
        head.push_str("=\"");
        if local(attribute.key.as_ref()) == key {
            head.push_str(&escape(value));
            replaced = true;
        } else {
            head.push_str(&escape(&attribute.unescape_value()?));
        }
        head.push('"');
    }
    if !replaced {
        head.push(' ');
        head.push_str(&String::from_utf8_lossy(key));
        head.push_str("=\"");
        head.push_str(&escape(value));
        head.push('"');
    }
    head.push_str(if empty { "/>" } else { ">" });
    Ok(head)
}

/// The opening tag of an element, as written, turned from `<row .../>` into
/// `<row ...>`.
fn element_head(element: &quick_xml::events::BytesStart<'_>) -> Result<String> {
    let mut head = String::from("<");
    head.push_str(&String::from_utf8_lossy(element.name().as_ref()));
    for attribute in element.attributes() {
        let attribute = attribute?;
        head.push(' ');
        head.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
        head.push_str("=\"");
        head.push_str(&escape(&attribute.unescape_value()?));
        head.push('"');
    }
    head.push('>');
    Ok(head)
}

pub(crate) fn attribute(
    element: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

pub(crate) fn local(name: &[u8]) -> &[u8] {
    match name.iter().position(|byte| *byte == b':') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

pub(crate) fn namespace_prefix(name: &[u8]) -> String {
    match name.iter().position(|byte| *byte == b':') {
        Some(index) => String::from_utf8_lossy(&name[..=index]).into_owned(),
        None => String::new(),
    }
}

pub(crate) fn qualified_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// `A`, `Z`, `AA` — the same base-26 naming the layout crate uses, duplicated
/// here rather than depending on layout from a writer.
fn column_label(index: u32) -> String {
    let mut label = Vec::new();
    let mut remaining = u64::from(index) + 1;
    while remaining > 0 {
        label.push(b'A' + ((remaining - 1) % 26) as u8);
        remaining = (remaining - 1) / 26;
    }
    label.reverse();
    String::from_utf8(label).expect("ASCII")
}
