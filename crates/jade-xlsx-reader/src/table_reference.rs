//! Structured table selectors independent of the calculation backend.
use crate::{CellRef, Table};
pub fn column_selectors(selector: &str) -> Vec<(usize, usize, String)> {
    let bytes = selector.as_bytes();
    let mut result = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 2;
                continue;
            }
            b'[' => starts.push((i, false)),
            b']' => {
                if let Some((start, nested)) = starts.pop() {
                    if !nested {
                        let raw = &selector[start + 1..i];
                        let raw = raw.strip_prefix('@').unwrap_or(raw);
                        let mut value = String::new();
                        let mut chars = raw.chars();
                        while let Some(c) = chars.next() {
                            if c == '\'' {
                                if let Some(c) = chars.next() {
                                    value.push(c);
                                }
                            } else {
                                value.push(c);
                            }
                        }
                        if !raw.is_empty() && !raw.starts_with('#') {
                            result.push((start + 1, i, value));
                        }
                    }
                    if let Some((_, nested)) = starts.last_mut() {
                        *nested = true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    result
}
pub fn to_a1(table: &Table, sheet: &str, selector: Option<&str>, at: Option<CellRef>) -> String {
    let selector = selector.unwrap_or("[#Data]");
    let lower = selector.to_ascii_lowercase();
    let flags = ["#all", "#data", "#headers", "#totals", "#this row"]
        .into_iter()
        .filter(|flag| lower.contains(&format!("[{flag}]")))
        .collect::<Vec<_>>();
    let this_row = lower.contains("[@") || flags.contains(&"#this row");
    let mut rows = Vec::new();
    if this_row {
        let Some(at) = at else {
            return "#VALUE!".into();
        };
        if at.row < table.reference.start.row + table.header_row_count
            || at.row
                > table
                    .reference
                    .end
                    .row
                    .saturating_sub(table.totals_row_count)
        {
            return "#VALUE!".into();
        }
        rows.push((at.row, at.row));
    } else if flags.contains(&"#all") {
        rows.push((table.reference.start.row, table.reference.end.row));
    } else {
        if flags.is_empty() || flags.contains(&"#data") {
            let start = table.reference.start.row + table.header_row_count;
            let end = table
                .reference
                .end
                .row
                .saturating_sub(table.totals_row_count);
            if start <= end {
                rows.push((start, end));
            }
        }
        if flags.contains(&"#headers") {
            if let Some(row) = table.header_row() {
                rows.push((row, row));
            } else {
                return "#REF!".into();
            }
        }
        if flags.contains(&"#totals") {
            if let Some(row) = table.totals_row() {
                rows.push((row, row));
            } else {
                return "#REF!".into();
            }
        }
    }
    rows.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (start, end) in rows {
        if let Some(last) = merged.last_mut() {
            if start <= last.1.saturating_add(1) {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    let names = column_selectors(selector);
    let mut columns = Vec::new();
    for (_, _, name) in &names {
        let Some(index) = table
            .columns
            .iter()
            .position(|n| n.to_lowercase() == name.to_lowercase())
        else {
            return "#REF!".into();
        };
        columns.push(table.reference.start.column + index as u32);
    }
    let column_areas = if columns.is_empty() {
        vec![(table.reference.start.column, table.reference.end.column)]
    } else if columns.len() > 1 && selector.chars().any(|c| c == ':') {
        vec![(
            *columns.iter().min().unwrap(),
            *columns.iter().max().unwrap(),
        )]
    } else {
        columns.into_iter().map(|c| (c, c)).collect()
    };
    let corner = |column: u32, row: u32| {
        format!(
            "${}{}{}",
            column_label(column),
            if this_row { "" } else { "$" },
            row + 1
        )
    };
    let mut refs = Vec::new();
    for (first, last) in merged {
        for (left, right) in &column_areas {
            let first = corner(*left, first);
            let last = corner(*right, last);
            let range = if first == last {
                first
            } else {
                format!("{first}:{last}")
            };
            refs.push(format!("{}!{range}", crate::quote_sheet_name(sheet)));
        }
    }
    match refs.len() {
        0 => "#REF!".into(),
        1 => refs.remove(0),
        _ => format!("({})", refs.join(",")),
    }
}
fn column_label(column: u32) -> String {
    let mut n = column + 1;
    let mut bytes = Vec::new();
    while n > 0 {
        bytes.push(b'A' + ((n - 1) % 26) as u8);
        n = (n - 1) / 26;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap()
}
