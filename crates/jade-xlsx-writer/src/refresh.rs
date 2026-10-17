//! Refreshing the cached `<v>` of formula cells in place.
//!
//! Recalculation changes what a formula's cached result is, not what the cell
//! is — so unlike [`crate::patch_sheet`], which replaces whole cells, this
//! touches at most the cell's `t` attribute and its `<v>` element. The
//! formula itself — a shared master's `ref`, a follower's `si` — and any
//! other child the cell carries survive byte for byte.

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::patch::{
    apply, attribute, escape as xml_escape, local, number_text, qualified_name, Splice,
};
use crate::{Result, WriteError};
use jade_xlsx_reader::CellRef;

/// A freshly computed result for a formula cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CachedValue {
    Number(f64),
    Text(String),
    Bool(bool),
    /// `#DIV/0!` and friends, verbatim.
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedValueEdit {
    pub reference: CellRef,
    pub value: CachedValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetCachedValues {
    /// The part name inside the archive, e.g. `xl/worksheets/sheet1.xml`.
    pub part_name: String,
    pub cells: Vec<CachedValueEdit>,
}

/// The `t` attribute a cached value needs on its `<c>`, `None` for numeric.
fn type_attribute(value: &CachedValue) -> Option<&'static str> {
    match value {
        CachedValue::Number(_) => None,
        CachedValue::Text(_) => Some("str"),
        CachedValue::Bool(_) => Some("b"),
        CachedValue::Error(_) => Some("e"),
    }
}

fn value_text(value: &CachedValue) -> String {
    match value {
        CachedValue::Number(number) => number_text(*number),
        CachedValue::Text(text) => xml_escape(&jade_xlsx_reader::xstring::encode_xstring(text)),
        CachedValue::Bool(flag) => u8::from(*flag).to_string(),
        CachedValue::Error(error) => xml_escape(error),
    }
}

/// Refreshes cached values in one worksheet part.
///
/// A reference that names a cell without a formula (or a cell the part does
/// not write) is ignored: there is no cached result there to refresh, and
/// replacing a stored value is [`crate::patch_sheet`]'s job.
pub fn refresh_sheet(xml: &str, edits: &[CachedValueEdit]) -> Result<String> {
    for edit in edits {
        if matches!(edit.value, CachedValue::Number(number) if !number.is_finite()) {
            return Err(WriteError::NonFiniteNumber {
                column: edit.reference.column,
                row: edit.reference.row,
            });
        }
    }
    let mut pending: HashMap<CellRef, &CachedValue> = HashMap::new();
    for edit in edits {
        pending.insert(edit.reference, &edit.value);
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut splices: Vec<Splice> = Vec::new();
    let mut buf = Vec::new();

    let mut in_sheet_data = false;
    let mut next_row_index = 0u32;
    let mut row_index = 0u32;
    let mut next_column = 0u32;

    /// The parts of the target cell found so far.
    struct OpenCell {
        value: CachedValue,
        head_start: usize,
        head_end: usize,
        head: String,
        prefix: String,
        type_matches: bool,
        formula_end: Option<usize>,
        formula_depth: usize,
        value_range: Option<(usize, usize)>,
    }
    let mut open: Option<OpenCell> = None;

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                in_sheet_data = true;
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if in_sheet_data && open.is_none() && local(element.name().as_ref()) == b"row" =>
            {
                row_index = attribute(element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value > 0)
                    .map(|value| value - 1)
                    .unwrap_or(next_row_index);
                next_row_index = row_index.saturating_add(1);
                next_column = 0;
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if in_sheet_data && open.is_none() && local(element.name().as_ref()) == b"c" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let reference = attribute(element, b"r")?
                    .and_then(|value| CellRef::parse(&value).ok())
                    .unwrap_or(CellRef::new(next_column, row_index));
                next_column = reference.column.saturating_add(1);
                // An empty `<c/>` holds no formula, so there is nothing to
                // refresh in it.
                if empty {
                    continue;
                }
                let Some(value) = pending.remove(&reference) else {
                    continue;
                };

                let head_end = reader.buffer_position() as usize;
                let wanted_type = type_attribute(value);
                let current_type = attribute(element, b"t")?;
                let type_matches = current_type.as_deref() == wanted_type;

                // The head with every attribute but `t`, which the new value
                // decides.
                let mut head = String::from("<");
                head.push_str(&qualified_name(element.name().as_ref()));
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    if local(attribute.key.as_ref()) == b"t" {
                        continue;
                    }
                    head.push(' ');
                    head.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
                    head.push_str("=\"");
                    head.push_str(&String::from_utf8_lossy(&attribute.value));
                    head.push('"');
                }
                if let Some(wanted) = wanted_type {
                    head.push_str(&format!(" t=\"{wanted}\""));
                }
                head.push('>');

                let name = element.name();
                let prefix = name
                    .prefix()
                    .map(|prefix| format!("{}:", String::from_utf8_lossy(prefix.as_ref())))
                    .unwrap_or_default();
                open = Some(OpenCell {
                    value: value.clone(),
                    head_start: start,
                    head_end,
                    head,
                    prefix,
                    type_matches,
                    formula_end: None,
                    formula_depth: 0,
                    value_range: None,
                });
            }
            Event::Start(ref element) if open.is_some() => {
                let cell = open.as_mut().expect("an open cell");
                match local(element.name().as_ref()) {
                    b"f" => cell.formula_depth += 1,
                    b"v" if cell.formula_depth == 0 && cell.value_range.is_none() => {
                        cell.value_range = Some((start, usize::MAX));
                    }
                    _ => {}
                }
            }
            Event::Empty(ref element) if open.is_some() => {
                let cell = open.as_mut().expect("an open cell");
                let end = reader.buffer_position() as usize;
                match local(element.name().as_ref()) {
                    b"f" if cell.formula_depth == 0 => cell.formula_end = Some(end),
                    b"v" if cell.formula_depth == 0 && cell.value_range.is_none() => {
                        cell.value_range = Some((start, end));
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if open.is_some() => {
                let end = reader.buffer_position() as usize;
                let cell = open.as_mut().expect("an open cell");
                match local(element.name().as_ref()) {
                    b"f" => {
                        cell.formula_depth = cell.formula_depth.saturating_sub(1);
                        if cell.formula_depth == 0 {
                            cell.formula_end = Some(end);
                        }
                    }
                    b"v" => {
                        if let Some((value_start, usize::MAX)) = cell.value_range {
                            cell.value_range = Some((value_start, end));
                        }
                    }
                    b"c" => {
                        let cell = open.take().expect("an open cell");
                        // A cell without a formula holds a stored value, not a
                        // cached result; leave it alone.
                        if let Some(formula_end) = cell.formula_end {
                            if !cell.type_matches {
                                splices.push(Splice {
                                    start: cell.head_start,
                                    end: cell.head_end,
                                    text: cell.head,
                                });
                            }
                            let value = format!(
                                "<{prefix}v>{}</{prefix}v>",
                                value_text(&cell.value),
                                prefix = cell.prefix
                            );
                            match cell.value_range {
                                Some((value_start, value_end)) => splices.push(Splice {
                                    start: value_start,
                                    end: value_end,
                                    text: value,
                                }),
                                None => splices.push(Splice {
                                    start: formula_end,
                                    end: formula_end,
                                    text: value,
                                }),
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                in_sheet_data = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(apply(xml, splices))
}
