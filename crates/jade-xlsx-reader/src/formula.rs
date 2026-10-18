//! Rewriting the references in a formula when rows or columns come and go.
//!
//! Inserting rows above a reference moves it down; deleting the rows it
//! sits on turns it into `#REF!`; a range that straddles the change grows
//! or shrinks. Excel moves absolute and relative references alike here —
//! `$` is about copying, not about the grid changing shape — and only
//! references into the changed sheet: a formula on another sheet moves its
//! `Sheet1!B4` but not its own `B4`.

use serde::{Deserialize, Serialize};

use crate::model::{CellRange, CellRef};

/// Which way a structural change runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Rows,
    Columns,
}

/// Rows or columns inserted before index `at` (0-based), or deleted from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureChange {
    /// The sheet whose grid changes shape.
    pub sheet: String,
    pub axis: Axis,
    pub at: u32,
    pub count: u32,
    /// `true` to insert, `false` to delete.
    pub inserted: bool,
}

impl StructureChange {
    /// Where an index on the axis lands; `None` when it was deleted or
    /// pushed off the grid.
    pub fn map_index(&self, index: u32) -> Option<u32> {
        let limit = match self.axis {
            Axis::Rows => crate::model::MAX_ROW_COUNT,
            Axis::Columns => crate::model::MAX_COLUMN_COUNT,
        };
        if self.inserted {
            if index >= self.at {
                index.checked_add(self.count).filter(|moved| *moved < limit)
            } else {
                Some(index)
            }
        } else if index < self.at {
            Some(index)
        } else if index < self.at.saturating_add(self.count) {
            None
        } else {
            Some(index - self.count)
        }
    }

    /// Where an inclusive span on the axis lands; `None` when every index
    /// in it was deleted. A deletion that overlaps one end shortens the
    /// span; an insertion inside it lengthens it.
    pub fn map_span(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        if self.inserted {
            return Some((self.map_index(start)?, self.map_index(end)?));
        }
        let deleted_end = self.at.saturating_add(self.count);
        if start >= self.at && end < deleted_end {
            return None;
        }
        let start = if start >= self.at && start < deleted_end {
            self.at
        } else {
            self.map_index(start)?
        };
        let end = if end >= self.at && end < deleted_end {
            self.at.checked_sub(1)?
        } else {
            self.map_index(end)?
        };
        (start <= end).then_some((start, end))
    }

    pub fn map_cell(&self, cell: CellRef) -> Option<CellRef> {
        Some(match self.axis {
            Axis::Rows => CellRef::new(cell.column, self.map_index(cell.row)?),
            Axis::Columns => CellRef::new(self.map_index(cell.column)?, cell.row),
        })
    }

    pub fn map_range(&self, range: CellRange) -> Option<CellRange> {
        let (start, end) = (range.start, range.end);
        Some(match self.axis {
            Axis::Rows => {
                let (first, last) = self.map_span(start.row, end.row)?;
                CellRange {
                    start: CellRef::new(start.column, first),
                    end: CellRef::new(end.column, last),
                }
            }
            Axis::Columns => {
                let (first, last) = self.map_span(start.column, end.column)?;
                CellRange {
                    start: CellRef::new(first, start.row),
                    end: CellRef::new(last, end.row),
                }
            }
        })
    }

    fn applies_to(&self, sheet: &str) -> bool {
        self.sheet.eq_ignore_ascii_case(sheet)
    }
}

/// One end of a reference as written: a column, a row, or both, each with
/// its `$`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Endpoint {
    column: Option<(bool, u32)>,
    row: Option<(bool, u32)>,
}

impl Endpoint {
    fn write(&self, out: &mut String) {
        if let Some((absolute, column)) = self.column {
            if absolute {
                out.push('$');
            }
            out.push_str(&column_label(column));
        }
        if let Some((absolute, row)) = self.row {
            if absolute {
                out.push('$');
            }
            out.push_str(&(row + 1).to_string());
        }
    }
}

/// `formula` — as written in the cell, without its `=` — with every
/// reference into `change.sheet` moved. `formula_sheet` is the sheet the
/// formula sits on, which is what an unqualified reference means.
pub fn shift_formula(formula: &str, formula_sheet: &str, change: &StructureChange) -> String {
    let bytes = formula.as_bytes();
    let mut output = String::with_capacity(formula.len());
    let mut index = 0usize;
    // The sheet the next reference belongs to, once a `Name!` has gone by.
    let mut qualified: Option<String> = None;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            let end = string_end(formula, index);
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        if byte == b'\'' {
            let end = quoted_end(formula, index);
            output.push_str(&formula[index..end]);
            if bytes.get(end) == Some(&b'!') {
                output.push('!');
                qualified = Some(formula[index + 1..end - 1].replace("''", "'"));
                index = end + 1;
            } else {
                index = end;
            }
            continue;
        }

        let previous = index.checked_sub(1).and_then(|at| bytes.get(at)).copied();
        let starts_token = !previous.is_some_and(is_name_byte) && previous != Some(b'[');
        if starts_token {
            if let Some((end, reference)) = parse_reference(formula, index) {
                let sheet = qualified.take();
                let applies = change.applies_to(sheet.as_deref().unwrap_or(formula_sheet));
                if applies {
                    match shift_reference(&reference, change) {
                        Some(shifted) => output.push_str(&shifted),
                        None => output.push_str("#REF!"),
                    }
                } else {
                    output.push_str(&formula[index..end]);
                }
                index = end;
                continue;
            }
            // A bare sheet name before `!`.
            let mut end = index;
            while bytes.get(end).is_some_and(|byte| is_name_byte(*byte)) {
                end += 1;
            }
            if end > index && bytes.get(end) == Some(&b'!') {
                qualified = Some(formula[index..end].to_string());
                output.push_str(&formula[index..=end]);
                index = end + 1;
                continue;
            }
        }

        let character = formula[index..].chars().next().expect("a character");
        output.push(character);
        index += character.len_utf8();
        if !is_name_byte(byte) && byte != b'!' {
            qualified = None;
        }
    }
    output
}

/// Relocates references fully contained in a cut range, preserving absolute
/// flags. A moved formula qualifies its unmoved local references when its sheet
/// changes. Partially overlapping ranges retain their original boundaries.
pub fn move_formula(
    formula: &str,
    old_context: &str,
    new_context: &str,
    source_sheet: &str,
    source: CellRange,
    target_sheet: &str,
    target: CellRef,
) -> String {
    let bytes = formula.as_bytes();
    let mut out = String::with_capacity(formula.len());
    let mut index = 0;
    let mut qualified: Option<(String, usize, bool)> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            let end = string_end(formula, index);
            out.push_str(&formula[index..end]);
            index = end;
            qualified = None;
            continue;
        }
        if byte == b'\'' {
            let end = quoted_end(formula, index);
            let begin = out.len();
            out.push_str(&formula[index..end]);
            if bytes.get(end) == Some(&b'!') {
                let sheet = formula[index + 1..end - 1].replace("''", "'");
                let external = sheet.contains('[');
                qualified = Some((sheet, begin, external));
                out.push('!');
                index = end + 1;
            } else {
                index = end;
            }
            continue;
        }
        let previous = index.checked_sub(1).and_then(|at| bytes.get(at)).copied();
        if !previous.is_some_and(is_name_byte) && previous != Some(b'[') {
            if let Some((end, mut reference)) = parse_reference(formula, index) {
                let qualifier = qualified.take();
                let sheet = qualifier
                    .as_ref()
                    .map(|(name, _, _)| name.as_str())
                    .unwrap_or(old_context);
                let external = qualifier.as_ref().is_some_and(|(_, _, external)| *external);
                let last = reference.end.unwrap_or(reference.start);
                let first_cell = CellRef::new(
                    reference.start.column.map_or(0, |(_, column)| column),
                    reference.start.row.map_or(0, |(_, row)| row),
                );
                let last_cell = CellRef::new(
                    last.column
                        .map_or(crate::model::MAX_COLUMN_COUNT - 1, |(_, column)| column),
                    last.row
                        .map_or(crate::model::MAX_ROW_COUNT - 1, |(_, row)| row),
                );
                let moved = !external
                    && sheet.eq_ignore_ascii_case(source_sheet)
                    && source.contains(first_cell)
                    && source.contains(last_cell);
                if moved {
                    let shift = |endpoint: &mut Endpoint| {
                        if let Some((_, column)) = endpoint.column.as_mut() {
                            *column = target.column + *column - source.start.column;
                        }
                        if let Some((_, row)) = endpoint.row.as_mut() {
                            *row = target.row + *row - source.start.row;
                        }
                    };
                    shift(&mut reference.start);
                    if let Some(last) = reference.end.as_mut() {
                        shift(last);
                    }
                    if let Some((_, begin, _)) = qualifier.as_ref() {
                        out.truncate(*begin);
                    }
                    if qualifier.is_some() || !target_sheet.eq_ignore_ascii_case(new_context) {
                        out.push_str(&format!("'{}'!", target_sheet.replace('\'', "''")));
                    }
                    out.push_str(&render(&reference));
                } else {
                    if qualifier.is_none() && !old_context.eq_ignore_ascii_case(new_context) {
                        out.push_str(&format!("'{}'!", old_context.replace('\'', "''")));
                    }
                    out.push_str(&formula[index..end]);
                }
                index = end;
                continue;
            }
            let mut end = index;
            while bytes.get(end).is_some_and(|byte| is_name_byte(*byte)) {
                end += 1;
            }
            if end > index && bytes.get(end) == Some(&b'!') {
                qualified = Some((
                    formula[index..end].into(),
                    out.len(),
                    previous == Some(b']'),
                ));
                out.push_str(&formula[index..=end]);
                index = end + 1;
                continue;
            }
        }
        let character = formula[index..].chars().next().expect("character exists");
        out.push(character);
        index += character.len_utf8();
        if !is_name_byte(byte) && byte != b'!' {
            qualified = None;
        }
    }
    out
}

/// A reference as written: one endpoint, or two around a colon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reference {
    start: Endpoint,
    end: Option<Endpoint>,
}

fn shift_reference(reference: &Reference, change: &StructureChange) -> Option<String> {
    let start = reference.start;
    let end = reference.end.unwrap_or(start);
    let axis_of = |endpoint: Endpoint| match change.axis {
        Axis::Rows => endpoint.row,
        Axis::Columns => endpoint.column,
    };
    let (first, last) = match (axis_of(start), axis_of(end)) {
        // Neither end names the changed axis (`A:A` under a row change):
        // nothing moves.
        (None, None) => return Some(render(reference)),
        (Some((_, first)), Some((_, last))) => (first, last),
        // `A1:B` is not a reference Excel writes; leave it alone.
        _ => return Some(render(reference)),
    };
    // A range written back to front (`B1:A2`) still covers the same cells;
    // map it the right way round and keep the order it was written in.
    let reversed = first > last;
    let (first, last) = if reference.end.is_some() {
        let (low, high) = change.map_span(first.min(last), first.max(last))?;
        if reversed {
            (high, low)
        } else {
            (low, high)
        }
    } else {
        let moved = change.map_index(first)?;
        (moved, moved)
    };
    let replace = |endpoint: Endpoint, value: u32| match change.axis {
        Axis::Rows => Endpoint {
            row: endpoint.row.map(|(absolute, _)| (absolute, value)),
            ..endpoint
        },
        Axis::Columns => Endpoint {
            column: endpoint.column.map(|(absolute, _)| (absolute, value)),
            ..endpoint
        },
    };
    Some(render(&Reference {
        start: replace(start, first),
        end: reference.end.map(|end| replace(end, last)),
    }))
}

fn render(reference: &Reference) -> String {
    let mut out = String::new();
    reference.start.write(&mut out);
    if let Some(end) = reference.end {
        out.push(':');
        end.write(&mut out);
    }
    out
}

/// A reference starting at `start`: `A1`, `$A$1`, `A1:B2`, `A:C`, `3:5`.
fn parse_reference(formula: &str, start: usize) -> Option<(usize, Reference)> {
    let (end, first) = parse_endpoint(formula, start)?;
    let bytes = formula.as_bytes();
    // A lone column or row is only a reference as one end of a range.
    let whole_axis = first.column.is_none() || first.row.is_none();
    if bytes.get(end) == Some(&b':') {
        if let Some((range_end, second)) = parse_endpoint(formula, end + 1) {
            let compatible = (first.column.is_some() == second.column.is_some())
                && (first.row.is_some() == second.row.is_some());
            if compatible && !followed_by_name(formula, range_end) {
                return Some((
                    range_end,
                    Reference {
                        start: first,
                        end: Some(second),
                    },
                ));
            }
        }
    }
    if whole_axis || followed_by_name(formula, end) {
        return None;
    }
    Some((
        end,
        Reference {
            start: first,
            end: None,
        },
    ))
}

fn followed_by_name(formula: &str, index: usize) -> bool {
    let following = formula.as_bytes().get(index).copied();
    following.is_some_and(is_name_byte)
        || matches!(following, Some(b'!') | Some(b'[') | Some(b']') | Some(b'('))
}

fn parse_endpoint(formula: &str, start: usize) -> Option<(usize, Endpoint)> {
    let bytes = formula.as_bytes();
    let mut index = start;
    let column_absolute = bytes.get(index) == Some(&b'$');
    index += usize::from(column_absolute);
    let column_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
        index += 1;
    }
    let column = if index > column_start {
        let number = column_number(&formula[column_start..index])?;
        (number < crate::model::MAX_COLUMN_COUNT).then_some((column_absolute, number))
    } else {
        if column_absolute {
            // `$` then digits: a row-only endpoint such as `$3`.
            index = start;
        }
        None
    };
    let column = match (index > column_start, column) {
        (true, None) => return None,
        (_, column) => column,
    };

    let row_absolute = bytes.get(index) == Some(&b'$');
    let row_start = index + usize::from(row_absolute);
    let mut row_end = row_start;
    while bytes.get(row_end).is_some_and(u8::is_ascii_digit) {
        row_end += 1;
    }
    let row = if row_end > row_start {
        let number = formula[row_start..row_end]
            .parse::<u32>()
            .ok()?
            .checked_sub(1)?;
        if number >= crate::model::MAX_ROW_COUNT {
            return None;
        }
        index = row_end;
        Some((row_absolute, number))
    } else {
        if row_absolute && column.is_some() {
            // `A$` with no digits: not a reference.
            return None;
        }
        None
    };
    if column.is_none() && row.is_none() {
        return None;
    }
    // A number with a decimal point or an exponent is a number.
    if column.is_none() && matches!(bytes.get(index), Some(b'.') | Some(b'E') | Some(b'e')) {
        return None;
    }
    Some((index, Endpoint { column, row }))
}

fn string_end(formula: &str, start: usize) -> usize {
    let bytes = formula.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if bytes.get(index + 1) == Some(&b'"') {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn quoted_end(formula: &str, start: usize) -> usize {
    let bytes = formula.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn is_name_byte(byte: u8) -> bool {
    byte >= 0x80 || byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
}

fn column_number(letters: &str) -> Option<u32> {
    let mut value = 0u32;
    for byte in letters.bytes() {
        let digit = u32::from(byte.to_ascii_uppercase().checked_sub(b'A')?) + 1;
        if digit > 26 {
            return None;
        }
        value = value.checked_mul(26)?.checked_add(digit)?;
    }
    value.checked_sub(1)
}

fn column_label(index: u32) -> String {
    let mut label = String::new();
    let mut value = index + 1;
    while value > 0 {
        let remainder = (value - 1) % 26;
        label.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    label
}

/// `formula` with every reference qualified by the sheet `from` — `from!A1`,
/// `'from'!A1:B2`, either end of a 3-D `from:Other!A1` — naming `to`
/// instead, quoted as `to` needs. Sheet names compare without regard to
/// case; text in string literals and references into other workbooks are
/// left alone.
pub fn rename_sheet_in_formula(formula: &str, from: &str, to: &str) -> String {
    let _ = (from, to);
    todo!("rename_sheet_in_formula {formula}")
}

/// `formula` with every reference into the deleted sheet `deleted` written
/// as `#REF!`, as Excel writes it.
pub fn delete_sheet_in_formula(
    formula: &str,
    deleted: &str,
    sheet_order: &[&str],
    deleted_tables: &[String],
) -> String {
    let _ = (deleted, sheet_order, deleted_tables);
    todo!("delete_sheet_in_formula {formula}")
}

/// `name` as a formula writes it before `!`.
pub fn quote_sheet_name(name: &str) -> String {
    todo!("quote_sheet_name {name}")
}
