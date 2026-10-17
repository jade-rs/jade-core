//! Verified SDK row-hide provenance. Standard row flags remain authoritative.
use crate::{Result, Sheet};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const URI: &str = "{061B6165-59F5-49A9-A710-2C1745C088F8}";
pub const NAMESPACE: &str = "urn:jlsi:jade:xlsx:filter-row-state:v1";
pub const ROW_LIMIT: usize = 100_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRowState {
    pub manual: HashSet<u32>,
    pub filtered: HashSet<u32>,
    pub invalid: bool,
}
pub fn body_ranges(sheet: &Sheet) -> Vec<(u32, u32)> {
    let mut ranges = vec![];
    if let Some(range) = sheet
        .auto_filter
        .as_ref()
        .and_then(|filter| filter.reference)
    {
        ranges.push((range.start.row.saturating_add(1), range.end.row));
    }
    for table in &sheet.tables {
        if let Some(filter) = &table.filter {
            let range = filter.reference.unwrap_or(table.reference);
            let end = if let Some(total) = table.totals_row() {
                let Some(end) = total.checked_sub(1) else {
                    continue;
                };
                range.end.row.min(end)
            } else {
                range.end.row
            };
            ranges.push((range.start.row.saturating_add(table.header_row_count), end));
        }
    }
    ranges.retain(|(first, end)| first <= end);
    ranges
}
pub fn signature(sheet: &Sheet, manual: &HashSet<u32>, filtered: &HashSet<u32>) -> String {
    let mut tables: Vec<_> = sheet
        .tables
        .iter()
        .map(|table| {
            (
                &table.name,
                table.reference,
                table.header_row_count,
                table.totals_row_count,
                &table.filter,
            )
        })
        .collect();
    tables.sort_by(|a, b| a.0.cmp(b.0));
    let ranges = body_ranges(sheet);
    let rows: Vec<_> = sheet
        .rows
        .iter()
        .filter(|row| {
            row.hidden != sheet.default_rows_hidden
                && ranges
                    .iter()
                    .any(|(first, end)| row.index >= *first && row.index <= *end)
        })
        .map(|row| (row.index, row.hidden))
        .collect();
    let mut manual: Vec<_> = manual.iter().copied().collect();
    manual.sort_unstable();
    let mut filtered: Vec<_> = filtered.iter().copied().collect();
    filtered.sort_unstable();
    let bytes = serde_json::to_vec(&(
        sheet.filter_mode,
        sheet.default_rows_hidden,
        &sheet.auto_filter,
        tables,
        rows,
        manual,
        filtered,
    ))
    .expect("filter metadata contains only serializable strings and integers");
    format!("{:x}", Sha256::digest(bytes))
}
fn attribute(element: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attr in element.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == key {
            return Ok(Some(attr.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
pub fn read(xml: &str, sheet: &Sheet) -> Result<Option<FilterRowState>> {
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0;
    let mut root_ns = None;
    let mut ext_list = false;
    let mut owned = false;
    let mut count = 0;
    let mut payloads = 0;
    let mut records = 0;
    let mut seen = HashSet::new();
    let ranges = body_ranges(sheet);
    let mut in_payload = false;
    let mut stamp = None;
    let mut result = FilterRowState::default();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = match ns {
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    ResolveResult::Unbound => Some(vec![]),
                    _ => None,
                };
                if depth == 0
                    && local.as_ref() == b"worksheet"
                    && ns.as_ref().is_some_and(|ns| {
                        ns.is_empty()
                            || matches!(
                                ns.as_slice(),
                                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                            )
                    })
                {
                    root_ns = ns.clone();
                }
                if depth == 1 && root_ns.is_some() && ns == root_ns && local.as_ref() == b"extLst" {
                    ext_list = !empty;
                }
                if ext_list
                    && depth == 2
                    && ns == root_ns
                    && local.as_ref() == b"ext"
                    && attribute(element, b"uri")?.as_deref() == Some(URI)
                {
                    count += 1;
                    owned = !empty;
                }
                if owned
                    && depth == 3
                    && ns.as_deref() == Some(NAMESPACE.as_bytes())
                    && local.as_ref() == b"filterRows"
                {
                    in_payload = !empty;
                    payloads += 1;
                    stamp = attribute(element, b"signature")?;
                    if attribute(element, b"version")?.as_deref() != Some("1") {
                        result.invalid = true;
                    }
                }
                if in_payload
                    && depth == 4
                    && ns.as_deref() == Some(NAMESPACE.as_bytes())
                    && local.as_ref() == b"record"
                {
                    records += 1;
                    let row =
                        attribute(element, b"index")?.and_then(|value| value.parse::<u32>().ok());
                    let manual = attribute(element, b"manual")?;
                    let filtered = attribute(element, b"filtered")?;
                    let Some(row) = row.filter(|row| *row < crate::model::MAX_ROW_COUNT) else {
                        result.invalid = true;
                        depth += usize::from(!empty);
                        continue;
                    };
                    let hidden = sheet
                        .rows
                        .binary_search_by_key(&row, |row| row.index)
                        .ok()
                        .map(|index| sheet.rows[index].hidden)
                        .unwrap_or(sheet.default_rows_hidden);
                    if records > ROW_LIMIT
                        || !ranges
                            .iter()
                            .any(|(first, end)| row >= *first && row <= *end)
                        || !seen.insert(row)
                        || !hidden
                        || !matches!(manual.as_deref(), Some("0" | "1"))
                        || !matches!(filtered.as_deref(), Some("0" | "1"))
                        || result.manual.len() + result.filtered.len() >= ROW_LIMIT * 2
                    {
                        result.invalid = true;
                    } else {
                        if manual.as_deref() == Some("1") {
                            result.manual.insert(row);
                        }
                        if filtered.as_deref() == Some("1") {
                            result.filtered.insert(row);
                        }
                    }
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 3 {
                    in_payload = false;
                }
                if depth == 2 {
                    owned = false;
                }
                if depth == 1 {
                    ext_list = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if count == 0 {
        return Ok(None);
    }
    if count != 1
        || payloads != 1
        || stamp.as_deref() != Some(signature(sheet, &result.manual, &result.filtered).as_str())
    {
        result.invalid = true;
    }
    if result.invalid {
        result.manual.clear();
        result.filtered.clear();
    }
    Ok(Some(result))
}
