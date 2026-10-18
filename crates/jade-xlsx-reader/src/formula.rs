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
/// instead, quoted as `to` needs. This is what Excel does to every formula
/// and defined name when a sheet is renamed. Sheet names compare without
/// regard to case; text in string literals (`INDIRECT("Sheet1!A1")`),
/// error literals (`#REF!`), structured references and references into
/// other workbooks (`[1]Sheet1!A1`) are left alone.
pub fn rename_sheet_in_formula(formula: &str, from: &str, to: &str) -> String {
    rewrite_sheet_qualifiers(
        formula,
        |first, last| {
            let renamed = |name: &str| {
                if same_sheet_name(name, from) {
                    to.to_string()
                } else {
                    name.to_string()
                }
            };
            if !same_sheet_name(first, from)
                && !last.is_some_and(|last| same_sheet_name(last, from))
            {
                return None;
            }
            Some(qualifier(&renamed(first), last.map(renamed).as_deref()))
        },
        |_| false,
    )
}

/// `formula` with every reference into the deleted sheet written as Excel
/// writes it: `Sheet2!A1` becomes `#REF!A1`, and a structured reference
/// into one of the sheet's tables (`deleted_tables`, compared without
/// case) becomes `#REF!`. A 3-D reference keeps the sheets that remain
/// between its ends — `Jan:Mar!A1` without `Jan` is `Feb:Mar!A1` — which
/// is why `sheet_order`, the tab order before the deletion, is needed; a
/// 3-D reference whose ends are not in it is left as written.
pub fn delete_sheet_in_formula(
    formula: &str,
    deleted: &str,
    sheet_order: &[&str],
    deleted_tables: &[String],
) -> String {
    rewrite_sheet_qualifiers(
        formula,
        |first, last| {
            let Some(last) = last else {
                return same_sheet_name(first, deleted).then(|| "#REF!".to_string());
            };
            if !same_sheet_name(first, deleted) && !same_sheet_name(last, deleted) {
                return None;
            }
            let position = |name: &str| {
                sheet_order
                    .iter()
                    .position(|sheet| same_sheet_name(sheet, name))
            };
            let (Some(a), Some(b)) = (position(first), position(last)) else {
                return Some("#REF!".to_string());
            };
            let remaining: Vec<&str> = sheet_order[a.min(b)..=a.max(b)]
                .iter()
                .copied()
                .filter(|sheet| !same_sheet_name(sheet, deleted))
                .collect();
            match remaining.as_slice() {
                [] => Some("#REF!".to_string()),
                [only] => Some(qualifier(only, None)),
                [start, .., end] => Some(qualifier(start, Some(end))),
            }
        },
        |table| {
            deleted_tables
                .iter()
                .any(|deleted| same_sheet_name(deleted, table))
        },
    )
}

/// Whether two sheet names name the same sheet: Excel compares them
/// without regard to case.
pub fn same_sheet_name(a: &str, b: &str) -> bool {
    a == b || a.to_uppercase() == b.to_uppercase()
}

/// `name` as a formula writes it before `!`: bare when Excel would write it
/// bare — ASCII letters, digits, `_` and `.`, not starting with a digit or
/// a period, and not readable as a cell reference, an R1C1 reference or a
/// boolean — and in apostrophes otherwise, with its own apostrophes
/// doubled. Quoting a name that did not need it is still valid.
pub fn quote_sheet_name(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
        && !name.as_bytes()[0].is_ascii_digit()
        && name.as_bytes()[0] != b'.'
        && !reads_as_reference(name);
    if bare {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// `Sheet!` or `First:Last!`, quoted as the names need.
fn qualifier(first: &str, last: Option<&str>) -> String {
    match last {
        None => format!("{}!", quote_sheet_name(first)),
        Some(last) => {
            let first_quoted = quote_sheet_name(first);
            let last_quoted = quote_sheet_name(last);
            if first_quoted == first && last_quoted == last {
                format!("{first}:{last}!")
            } else {
                format!(
                    "'{}:{}'!",
                    first.replace('\'', "''"),
                    last.replace('\'', "''")
                )
            }
        }
    }
}

/// Whether a bare name would be read as something other than a sheet: a
/// cell (`A1`, `XFD1048576`), an R1C1 reference (`R`, `C`, `R1C1`, `RC2`)
/// or a boolean.
fn reads_as_reference(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper == "TRUE" || upper == "FALSE" {
        return true;
    }
    let letters = upper.bytes().take_while(u8::is_ascii_alphabetic).count();
    let digits = &upper[letters..];
    if (1..=3).contains(&letters)
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return true;
    }
    let r1c1 = |text: &str| {
        let rest = text.strip_prefix('R').map_or(Some(text), |rest| {
            Some(rest.trim_start_matches(|c: char| c.is_ascii_digit()))
        });
        match rest {
            Some("") => text.starts_with('R'),
            Some(rest) => rest
                .strip_prefix('C')
                .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit())),
            None => false,
        }
    };
    r1c1(&upper)
}

/// Walks `formula`, offering each sheet qualifier — `Sheet!`, `'Sheet'!`,
/// `First:Last!` — to `on_qualifier`, which returns the text to write in
/// its place (its `!` included) or `None` to keep it, and each structured
/// reference `Name[...]` to `drop_table`, which says whether it becomes
/// `#REF!`. Strings, error literals and other workbooks' references pass
/// through untouched.
fn rewrite_sheet_qualifiers(
    formula: &str,
    mut on_qualifier: impl FnMut(&str, Option<&str>) -> Option<String>,
    mut drop_table: impl FnMut(&str) -> bool,
) -> String {
    let bytes = formula.as_bytes();
    let mut output = String::with_capacity(formula.len());
    let mut index = 0usize;
    // A `[1]` workbook index makes the qualifier after it another file's.
    let mut external = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            let end = string_end(formula, index);
            output.push_str(&formula[index..end]);
            index = end;
            external = false;
            continue;
        }
        if byte == b'\'' {
            let end = quoted_end(formula, index);
            if bytes.get(end) == Some(&b'!') && end >= index + 2 {
                let inner = formula[index + 1..end - 1].replace("''", "'");
                let replacement = if external || inner.starts_with('[') {
                    None
                } else {
                    match inner.split_once(':') {
                        Some((first, last)) => on_qualifier(first, Some(last)),
                        None => on_qualifier(&inner, None),
                    }
                };
                match replacement {
                    Some(text) => output.push_str(&text),
                    None => output.push_str(&formula[index..=end]),
                }
                index = end + 1;
            } else {
                output.push_str(&formula[index..end]);
                index = end;
            }
            external = false;
            continue;
        }
        if byte == b'[' {
            // A workbook index, or a structured reference relative to the
            // formula's own table: neither names a sheet.
            let end = bracket_end(formula, index);
            output.push_str(&formula[index..end]);
            index = end;
            external = true;
            continue;
        }
        let previous = index.checked_sub(1).map(|at| bytes[at]);
        if is_name_byte(byte) && !previous.is_some_and(is_name_byte) {
            let mut end = index;
            while bytes.get(end).is_some_and(|byte| is_name_byte(*byte)) {
                end += 1;
            }
            let name = &formula[index..end];
            let error_literal = previous == Some(b'#');
            match bytes.get(end) {
                Some(b'!') if !error_literal => {
                    let replacement = if external {
                        None
                    } else {
                        on_qualifier(name, None)
                    };
                    match replacement {
                        Some(text) => output.push_str(&text),
                        None => output.push_str(&formula[index..=end]),
                    }
                    index = end + 1;
                    external = false;
                    continue;
                }
                Some(b':') if !error_literal => {
                    // `First:Last!`, a 3-D reference — or a range such as
                    // `A1:B2`, which has no `!` after its second end.
                    let mut last_end = end + 1;
                    while bytes.get(last_end).is_some_and(|byte| is_name_byte(*byte)) {
                        last_end += 1;
                    }
                    if last_end > end + 1 && bytes.get(last_end) == Some(&b'!') {
                        let last = &formula[end + 1..last_end];
                        let replacement = if external {
                            None
                        } else {
                            on_qualifier(name, Some(last))
                        };
                        match replacement {
                            Some(text) => output.push_str(&text),
                            None => output.push_str(&formula[index..=last_end]),
                        }
                        index = last_end + 1;
                        external = false;
                        continue;
                    }
                }
                Some(b'[') if !error_literal => {
                    let close = bracket_end(formula, end);
                    if drop_table(name) {
                        output.push_str("#REF!");
                    } else {
                        output.push_str(&formula[index..close]);
                    }
                    index = close;
                    external = false;
                    continue;
                }
                _ => {}
            }
            output.push_str(name);
            index = end;
            external = false;
            continue;
        }
        let character = formula[index..].chars().next().expect("a character");
        output.push(character);
        index += character.len_utf8();
        external = false;
    }
    output
}

/// One past the `]` that closes the bracket opened at `start`, counting
/// nested brackets and skipping characters escaped with `'`.
fn bracket_end(formula: &str, start: usize) -> usize {
    let bytes = formula.as_bytes();
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => {
                index += 1;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}
