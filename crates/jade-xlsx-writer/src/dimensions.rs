//! Worksheet axis sizes, preserving cell data and unrelated metadata.
use crate::{
    patch::{apply, attribute, escape, local, namespace_prefix, Splice},
    Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    Axis, Workbook,
};
use quick_xml::{
    events::{BytesStart, Event},
    name::{QName, ResolveResult},
    NsReader,
};
use std::collections::BTreeMap;
pub const MAX_COLUMN_WIDTH: f32 = 255.0;
pub const MAX_ROW_HEIGHT: f32 = 409.0;
pub const AXIS_SIZE_LIMIT: usize = 1_000_000;

/// Uniformly sizes all worksheet rows without materializing implicit rows.
pub fn set_all_row_sizes(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    size: f32,
) -> Result<Vec<u8>> {
    set_row_sizes_with_default(original, workbook, sheet, size, &BTreeMap::new(), true)
}

/// Sizes implicit rows through a default and retains only explicit exceptions.
/// Automatic fitting sets `custom_height` false so wrapped content can grow.
pub fn set_row_sizes_with_default(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    size: f32,
    overrides: &BTreeMap<u32, f32>,
    custom_height: bool,
) -> Result<Vec<u8>> {
    validate_row_sizes(overrides)?;
    if !size.is_finite() || !(0.0..=MAX_ROW_HEIGHT).contains(&size) {
        return Err(WriteError::InvalidAxisSize {
            value: size,
            maximum: MAX_ROW_HEIGHT,
        });
    }
    let sheet = workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    let default_height = if size > 0.0 {
        size
    } else {
        sheet
            .default_row_height_points
            .filter(|height| *height > 0.0 && height.is_finite())
            .unwrap_or(15.0)
    };
    let mut package = OpcPackage::open(original)?;
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let mut reader = NsReader::from_str(&xml);
    let mut splices = Vec::new();
    let mut format_found = false;
    let mut insert = None;
    let mut prefix = String::new();
    let mut in_data = false;
    let mut next_row = 0;
    let mut pending = overrides.clone();
    let values = [
        ("defaultRowHeight", default_height.to_string()),
        ("customHeight", u8::from(custom_height).to_string()),
        ("zeroHeight", u8::from(size == 0.0).to_string()),
    ];
    loop {
        let start = reader.buffer_position() as usize;
        let (namespace, event) = reader.read_resolved_event()?;
        let is_main = main(namespace);
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if is_main => {
                let name = element.name().as_ref().to_vec();
                let end = reader.buffer_position() as usize;
                match local(&name) {
                    b"worksheet" => prefix = namespace_prefix(&name),
                    b"sheetFormatPr" => {
                        format_found = true;
                        splices.push(Splice {
                            start,
                            end,
                            text: head(element, &values, empty)?,
                        });
                    }
                    b"cols" | b"sheetData" => {
                        insert.get_or_insert(start);
                        if local(&name) == b"sheetData" {
                            in_data = !empty;
                            if empty && !pending.is_empty() {
                                splices.push(Splice {
                                    start,
                                    end,
                                    text: format!(
                                        "<{prefix}sheetData>{}</{prefix}sheetData>",
                                        pending_rows(
                                            &mut pending,
                                            MAX_ROW_COUNT,
                                            &prefix,
                                            custom_height
                                        )
                                    ),
                                });
                            }
                        }
                    }
                    b"row" if in_data => {
                        let row = attribute(element, b"r")?
                            .and_then(|row| row.parse::<u32>().ok())
                            .and_then(|row| row.checked_sub(1))
                            .unwrap_or(next_row);
                        next_row = row + 1;
                        let inserted = pending_rows(&mut pending, row, &prefix, custom_height);
                        if !inserted.is_empty() {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: inserted,
                            });
                        }
                        let row_size = pending.remove(&row).unwrap_or(size);
                        splices.push(Splice {
                            start,
                            end,
                            text: head(
                                element,
                                &row_size_attributes(row_size, custom_height, false),
                                empty,
                            )?,
                        });
                        if !empty {
                            reader.read_to_end(QName(&name))?;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(element) if is_main && local(element.name().as_ref()) == b"sheetData" => {
                in_data = false;
                let inserted = pending_rows(&mut pending, MAX_ROW_COUNT, &prefix, custom_height);
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
    if !format_found {
        let at = insert.ok_or_else(|| WriteError::MissingPart("sheetData".into()))?;
        splices.push(Splice { start: at, end: at, text: format!(
            "<{prefix}sheetFormatPr defaultRowHeight=\"{default_height}\" customHeight=\"{}\" zeroHeight=\"{}\"/>",
            u8::from(custom_height), u8::from(size == 0.0)
        ) });
    }
    if !pending.is_empty() {
        return Err(WriteError::MissingPart("sheetData".into()));
    }
    package.replace(&sheet.part_name, apply(&xml, splices).into_bytes());
    Ok(package.write()?)
}

/// Zero size hides the axis; a positive size also reveals it. Column sizes are
/// stored in SpreadsheetML encoded digit units and row sizes in points.
pub fn set_axis_sizes(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    axis: Axis,
    sizes: &BTreeMap<u32, f32>,
) -> Result<Vec<u8>> {
    write_axis_sizes(original, workbook, sheet, axis, sizes, true, false)
}

pub fn set_fitted_row_sizes(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    sizes: &BTreeMap<u32, f32>,
) -> Result<Vec<u8>> {
    write_axis_sizes(original, workbook, sheet, Axis::Rows, sizes, false, false)
}
/// Refreshes automatic height caches without changing row visibility.
pub fn refresh_automatic_row_sizes(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    sizes: &BTreeMap<u32, f32>,
) -> Result<Vec<u8>> {
    write_axis_sizes(original, workbook, sheet, Axis::Rows, sizes, false, true)
}
fn write_axis_sizes(
    original: &[u8],
    workbook: &Workbook,
    sheet: &str,
    axis: Axis,
    sizes: &BTreeMap<u32, f32>,
    custom_height: bool,
    preserve_visibility: bool,
) -> Result<Vec<u8>> {
    let limit = if axis == Axis::Rows {
        MAX_ROW_COUNT
    } else {
        MAX_COLUMN_COUNT
    };
    let maximum = if axis == Axis::Rows {
        MAX_ROW_HEIGHT
    } else {
        MAX_COLUMN_WIDTH
    };
    if sizes.len() > AXIS_SIZE_LIMIT {
        return Err(WriteError::AxisFormatLimit {
            limit: AXIS_SIZE_LIMIT as u64,
        });
    }
    for (index, size) in sizes {
        if *index >= limit {
            return Err(WriteError::StructureOutOfBounds {
                at: *index,
                count: 1,
            });
        }
        if !size.is_finite() || *size < 0.0 || *size > maximum {
            return Err(WriteError::InvalidAxisSize {
                value: *size,
                maximum,
            });
        }
    }
    let sheet = workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    if sizes.is_empty() {
        return Ok(original.to_vec());
    }
    let mut package = OpcPackage::open(original)?;
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let patched = if axis == Axis::Rows {
        patch_rows(&xml, sizes, custom_height, preserve_visibility)?
    } else {
        patch_columns(&xml, sizes)?
    };
    package.replace(&sheet.part_name, patched.into_bytes());
    Ok(package.write()?)
}
fn main(namespace: ResolveResult<'_>) -> bool {
    match namespace {
        ResolveResult::Unbound => true,
        ResolveResult::Bound(namespace) => matches!(
            namespace.as_ref(),
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
        ),
        ResolveResult::Unknown(_) => false,
    }
}
fn head(element: &BytesStart<'_>, values: &[(&str, String)], empty: bool) -> Result<String> {
    let mut out = format!("<{}", String::from_utf8_lossy(element.name().as_ref()));
    for attr in element.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref());
        if values.iter().any(|(name, _)| *name == key) {
            continue;
        }
        out.push_str(&format!(" {key}=\"{}\"", escape(&attr.unescape_value()?)));
    }
    for (key, value) in values {
        out.push_str(&format!(" {key}=\"{}\"", escape(value)));
    }
    out.push_str(if empty { "/>" } else { ">" });
    Ok(out)
}
fn size_attributes(size: f32, row: bool) -> Vec<(&'static str, String)> {
    vec![
        (
            if row { "ht" } else { "width" },
            if row {
                size.to_string()
            } else {
                f64::from(size).to_string()
            },
        ),
        (if row { "customHeight" } else { "customWidth" }, "1".into()),
        ("hidden", u8::from(size == 0.0).to_string()),
    ]
}
fn validate_row_sizes(sizes: &BTreeMap<u32, f32>) -> Result<()> {
    if sizes.len() > AXIS_SIZE_LIMIT {
        return Err(WriteError::AxisFormatLimit {
            limit: AXIS_SIZE_LIMIT as u64,
        });
    }
    for (&index, &size) in sizes {
        if index >= MAX_ROW_COUNT {
            return Err(WriteError::StructureOutOfBounds {
                at: index,
                count: 1,
            });
        }
        if !size.is_finite() || !(0.0..=MAX_ROW_HEIGHT).contains(&size) {
            return Err(WriteError::InvalidAxisSize {
                value: size,
                maximum: MAX_ROW_HEIGHT,
            });
        }
    }
    Ok(())
}
fn row_size_attributes(
    size: f32,
    custom: bool,
    preserve_visibility: bool,
) -> Vec<(&'static str, String)> {
    let mut values = vec![
        ("ht", size.to_string()),
        ("customHeight", u8::from(custom).to_string()),
    ];
    if !preserve_visibility {
        values.push(("hidden", u8::from(size == 0.0).to_string()));
    }
    values
}
fn pending_rows(
    pending: &mut BTreeMap<u32, f32>,
    before: u32,
    prefix: &str,
    custom_height: bool,
) -> String {
    let mut out = String::new();
    while pending
        .first_key_value()
        .is_some_and(|(row, _)| *row < before)
    {
        let (row, size) = pending.pop_first().expect("entry exists");
        out.push_str(&format!(
            "<{prefix}row r=\"{}\" ht=\"{size}\" customHeight=\"{}\" hidden=\"{}\"/>",
            row + 1,
            u8::from(custom_height),
            u8::from(size == 0.0)
        ));
    }
    out
}
fn patch_rows(
    xml: &str,
    sizes: &BTreeMap<u32, f32>,
    custom_height: bool,
    preserve_visibility: bool,
) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut pending = sizes.clone();
    let mut splices = Vec::new();
    let mut prefix = String::new();
    let mut in_data = false;
    let mut next_row = 0;
    loop {
        let start = reader.buffer_position() as usize;
        let (namespace, event) = reader.read_resolved_event()?;
        let is_main = main(namespace);
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if is_main => {
                let name = element.name().as_ref().to_vec();
                let end = reader.buffer_position() as usize;
                match local(&name) {
                    b"sheetData" => {
                        prefix = namespace_prefix(&name);
                        if empty {
                            splices.push(Splice {
                                start,
                                end,
                                text: format!(
                                    "<{prefix}sheetData>{}</{prefix}sheetData>",
                                    pending_rows(
                                        &mut pending,
                                        MAX_ROW_COUNT,
                                        &prefix,
                                        custom_height
                                    )
                                ),
                            });
                        } else {
                            in_data = true;
                        }
                    }
                    b"row" if in_data => {
                        let row = attribute(element, b"r")?
                            .and_then(|row| row.parse::<u32>().ok())
                            .and_then(|row| row.checked_sub(1))
                            .unwrap_or(next_row);
                        next_row = row + 1;
                        let inserted = pending_rows(&mut pending, row, &prefix, custom_height);
                        if !inserted.is_empty() {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: inserted,
                            });
                        }
                        if let Some(size) = pending.remove(&row) {
                            splices.push(Splice {
                                start,
                                end,
                                text: head(
                                    element,
                                    &row_size_attributes(size, custom_height, preserve_visibility),
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
            Event::End(element) if is_main && local(element.name().as_ref()) == b"sheetData" => {
                in_data = false;
                let inserted = pending_rows(&mut pending, MAX_ROW_COUNT, &prefix, custom_height);
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
    if !pending.is_empty() {
        return Err(WriteError::MissingPart("sheetData".into()));
    }
    Ok(apply(xml, splices))
}
fn columns_xml(sizes: &BTreeMap<u32, f32>, prefix: &str) -> String {
    let mut out = String::new();
    let mut entries = sizes.iter().peekable();
    while let Some((&first, &size)) = entries.next() {
        let mut last = first;
        while entries
            .peek()
            .is_some_and(|(column, next)| **column == last + 1 && **next == size)
        {
            last += 1;
            entries.next();
        }
        let stored_size = f64::from(size);
        out.push_str(&format!(
            "<{prefix}col min=\"{}\" max=\"{}\" width=\"{stored_size}\" customWidth=\"1\" hidden=\"{}\"/>",
            first + 1,
            last + 1,
            u8::from(size == 0.0)
        ));
    }
    out
}
fn patch_columns(xml: &str, sizes: &BTreeMap<u32, f32>) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut remaining = sizes.clone();
    let mut prefix = String::new();
    let mut columns = None;
    let mut entries = Vec::new();
    let mut insert = None;
    let mut in_columns = false;
    loop {
        let start = reader.buffer_position() as usize;
        let (namespace, event) = reader.read_resolved_event()?;
        let is_main = main(namespace);
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if is_main => {
                let name = element.name().as_ref().to_vec();
                let end = reader.buffer_position() as usize;
                match local(&name) {
                    b"worksheet" => prefix = namespace_prefix(&name),
                    b"cols" => {
                        columns = Some((start, end, name.clone(), element.to_owned()));
                        in_columns = !empty;
                    }
                    b"col" if in_columns => {
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
                        if sizes.range(first..=last).next().is_none() {
                            entries.push((first, xml[start..finish].to_string()));
                            continue;
                        }
                        let mut at = first;
                        while at <= last {
                            let size = sizes.get(&at);
                            let mut to = at;
                            while to < last && sizes.get(&(to + 1)) == size {
                                to += 1;
                            }
                            let mut values =
                                vec![("min", (at + 1).to_string()), ("max", (to + 1).to_string())];
                            if let Some(size) = size {
                                values.extend(size_attributes(*size, false));
                            }
                            entries.push((
                                at,
                                format!("{}{}", head(element, &values, empty)?, &xml[end..finish]),
                            ));
                            for column in at..=to {
                                remaining.remove(&column);
                            }
                            at = to + 1;
                        }
                    }
                    b"sheetData" => {
                        insert = Some(start);
                        break;
                    }
                    _ => {}
                }
            }
            Event::End(element) if is_main && local(element.name().as_ref()) == b"cols" => {
                in_columns = false;
                if let Some((_, end, _, _)) = columns.as_mut() {
                    *end = reader.buffer_position() as usize;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // New gaps must be sorted among existing spans.
    let gaps = columns_xml(&remaining, &prefix);
    let mut gaps_reader = NsReader::from_str(&gaps);
    loop {
        match gaps_reader.read_event()? {
            Event::Empty(element) => {
                let first = attribute(&element, b"min")?
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(1)
                    - 1;
                entries.push((first, head(&element, &[], true)?));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    entries.sort_by_key(|(first, _)| *first);
    let content: String = entries.into_iter().map(|(_, xml)| xml).collect();
    let splice = if let Some((start, end, name, element)) = columns {
        Splice {
            start,
            end,
            text: format!(
                "{}{content}</{}>",
                head(&element, &[], false)?,
                String::from_utf8_lossy(&name)
            ),
        }
    } else {
        let at = insert.ok_or_else(|| WriteError::MissingPart("sheetData".into()))?;
        Splice {
            start: at,
            end: at,
            text: format!("<{prefix}cols>{content}</{prefix}cols>"),
        }
    };
    Ok(apply(xml, vec![splice]))
}
