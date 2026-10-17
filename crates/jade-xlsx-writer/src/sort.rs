//! Permute a rectangular row range while preserving the original cell XML.
use crate::{
    auto_filter::{children, head_attribute},
    patch::{apply, attribute, detach_edited_shared_formulas, escape, namespace_prefix, Splice},
    shift::{range_text, reference_text},
    CellEdit, NewValue, Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{CellRange, CellRef, Sheet};
use quick_xml::{
    events::Event,
    name::{QName, ResolveResult},
    NsReader,
};
use std::collections::{BTreeMap, BTreeSet};

pub const SORT_ROW_LIMIT: u32 = 100_000;
pub const SORT_CELL_LIMIT: u64 = 1_000_000;
fn error(message: impl Into<String>) -> WriteError {
    WriteError::Sort(message.into())
}
pub fn intersects(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.column <= b.end.column
        && b.start.column <= a.end.column
}
fn namespace(ns: ResolveResult<'_>) -> Option<Vec<u8>> {
    match ns {
        ResolveResult::Unbound => Some(vec![]),
        ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
        _ => None,
    }
}
#[derive(Clone)]
struct RawCell {
    at: CellRef,
    start: usize,
    end: usize,
    xml: String,
}
fn cells(xml: &str, range: CellRange) -> Result<Vec<RawCell>> {
    let mut reader = NsReader::from_str(xml);
    let mut stack = Vec::<(String, Option<Vec<u8>>, BTreeMap<String, String>)>::new();
    let mut root_ns = None;
    let mut row = 0;
    let mut next_row = 0;
    let mut next_column = 0;
    let mut cells = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name();
                let (ns, local) = reader.resolver().resolve_element(name);
                let ns = namespace(ns);
                let local = String::from_utf8_lossy(local.as_ref()).into_owned();
                let mut declarations = stack
                    .last()
                    .map(|(_, _, attrs)| attrs.clone())
                    .unwrap_or_default();
                for attr in element.attributes() {
                    let attr = attr?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        declarations.insert(key, attr.unescape_value()?.into_owned());
                    }
                }
                if stack.is_empty() && local == "worksheet" {
                    root_ns = ns.clone();
                }
                let owned = ns == root_ns && root_ns.is_some();
                if matches!(local.as_str(), "row" | "c")
                    && !owned
                    && stack.iter().any(|(local, _, _)| local == "sheetData")
                {
                    return Err(error("foreign cell/row elements cannot yet be sorted"));
                }
                if local == "row"
                    && owned
                    && attribute(element, b"outlineLevel")?.is_some_and(|v| v != "0")
                {
                    return Err(error("outlined sheets cannot yet be sorted"));
                }

                if stack.len() == 2 && stack[1].0 == "sheetData" && owned && local == "row" {
                    row = attribute(element, b"r")?
                        .and_then(|v| v.parse::<u32>().ok())
                        .and_then(|v| v.checked_sub(1))
                        .unwrap_or(next_row);
                    next_row = row + 1;
                    next_column = 0;
                }
                if stack.len() == 3
                    && stack[1].0 == "sheetData"
                    && stack[2].0 == "row"
                    && owned
                    && local == "c"
                {
                    let at = attribute(element, b"r")?
                        .and_then(|v| CellRef::parse(&v).ok())
                        .unwrap_or(CellRef::new(next_column, row));
                    next_column = at.column + 1;
                    let head = reader.buffer_position() as usize;
                    let mut cell_head = xml[start..head].to_string();
                    // A row can supply aliases absent at the destination row.
                    for (key, value) in &declarations {
                        cell_head = head_attribute(&cell_head, key, value)?;
                    }
                    if !empty {
                        reader.read_to_end(QName(name.as_ref()))?;
                    }
                    let end = reader.buffer_position() as usize;
                    if range.contains(at) {
                        cells.push(RawCell {
                            at,
                            start,
                            end,
                            xml: format!("{cell_head}{}", &xml[head..end]),
                        });
                    }
                    continue;
                }
                if !empty {
                    stack.push((local, ns, declarations));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(cells)
}
fn relocate_cell(raw: &str, to: CellRef, style: u32, formula: Option<&str>) -> Result<String> {
    let mut reader = NsReader::from_str(raw);
    let mut root_ns = None;
    let mut depth = 0;
    let mut splices = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name();
                let (ns, local) = reader.resolver().resolve_element(name);
                let ns = namespace(ns);
                if depth == 0 {
                    root_ns = ns;
                    let head = reader.buffer_position() as usize;
                    let rewritten = head_attribute(
                        &head_attribute(&raw[start..head], "r", &reference_text(to))?,
                        "s",
                        &style.to_string(),
                    )?;
                    splices.push(Splice {
                        start,
                        end: head,
                        text: rewritten,
                    });
                } else if depth == 1 && ns == root_ns && local.as_ref() == b"f" {
                    if attribute(element, b"t")?.is_some_and(|kind| kind != "normal") {
                        return Err(error("array/data-table formulas cannot yet be sorted"));
                    }
                    let prefix = namespace_prefix(name.as_ref());
                    if !empty {
                        reader.read_to_end(QName(name.as_ref()))?;
                    }
                    let end = reader.buffer_position() as usize;
                    if let Some(formula) = formula {
                        splices.push(Splice {
                            start,
                            end,
                            text: format!("<{prefix}f>{}</{prefix}f>", escape(formula)),
                        });
                    }
                    continue;
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth -= 1;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(raw, splices))
}

fn check_formula_regions(xml: &str, range: CellRange) -> Result<()> {
    let mut reader = NsReader::from_str(xml);
    let mut root_ns = None;
    let mut stack = Vec::<(String, Option<Vec<u8>>)>::new();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = namespace(ns);
                let local = String::from_utf8_lossy(local.as_ref()).into_owned();
                if stack.is_empty() {
                    root_ns = ns.clone();
                }
                if local == "f"
                    && ns == root_ns
                    && stack.len() == 4
                    && stack[1].0 == "sheetData"
                    && stack[2].0 == "row"
                    && stack[3].0 == "c"
                    && stack[1..].iter().all(|(_, ns)| ns == &root_ns)
                    && attribute(element, b"t")?
                        .is_some_and(|kind| matches!(kind.as_str(), "array" | "dataTable"))
                    && attribute(element, b"ref")?
                        .and_then(|v| CellRange::parse(&v).ok())
                        .is_some_and(|r| intersects(r, range))
                {
                    return Err(error(
                        "array/data-table formula regions cannot yet be sorted",
                    ));
                }
                if !empty {
                    stack.push((local, ns));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

/// `order[destination - range.start.row]` contains the original source row.
/// Row sizes and row defaults stay at their worksheet positions; cell formats
/// become explicit where necessary so inherited source formatting moves too.
pub fn permute_rows(
    original: &[u8],
    sheet: &Sheet,
    range: CellRange,
    order: &[u32],
) -> Result<Vec<u8>> {
    let height = range
        .end
        .row
        .checked_sub(range.start.row)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("invalid sort range"))?;
    let width = range
        .end
        .column
        .checked_sub(range.start.column)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("invalid sort range"))?;
    if height > SORT_ROW_LIMIT || u64::from(height) * u64::from(width) > SORT_CELL_LIMIT {
        return Err(error("sort row/cell limit exceeded"));
    }
    if order.len() != height as usize
        || order.iter().copied().collect::<BTreeSet<_>>()
            != (range.start.row..=range.end.row).collect()
    {
        return Err(error("invalid row permutation"));
    }
    if sheet.merges.iter().any(|merge| intersects(*merge, range)) {
        return Err(error("merged sort ranges are not supported yet"));
    }
    if sheet
        .data_validations
        .iter()
        .any(|rule| rule.ranges.iter().any(|r| intersects(*r, range)))
    {
        return Err(error(
            "sorting ranges with data validation is not supported yet",
        ));
    }
    if sheet.shapes.iter().any(|shape| {
        intersects(
            CellRange {
                start: shape.from,
                end: shape.to.unwrap_or(shape.from),
            },
            range,
        )
    }) {
        return Err(error("sorting anchored drawings is not supported yet"));
    }
    let mut package = OpcPackage::open(original)?;
    let (styles_part, styles) = crate::styles_part(original)?;
    if package.part(&styles_part).is_none() {
        crate::styles::install_styles_relationship(&mut package, &styles_part)?;
        package.replace(&styles_part, styles.into_bytes());
    }
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let (_, nodes) = children(&xml, "worksheet")?;
    check_formula_regions(&xml, range)?;
    if nodes.iter().any(|node| {
        matches!(
            node.local.as_str(),
            "sheetProtection" | "protectedRanges" | "pivotTableParts"
        )
    }) {
        return Err(error("protected/pivot sheets cannot yet be sorted"));
    }
    let raw = cells(&xml, range)?;
    let touched: Vec<_> = raw
        .iter()
        .filter(|cell| range.contains(cell.at))
        .map(|cell| CellEdit {
            reference: cell.at,
            value: NewValue::Blank,
            style_index: None,
        })
        .collect();
    let expanded = detach_edited_shared_formulas(&xml, &touched)?;
    let raw: BTreeMap<_, _> = cells(&expanded, range)?
        .into_iter()
        .filter(|cell| range.contains(cell.at))
        .map(|cell| (cell.at, cell.xml))
        .collect();
    let model: BTreeMap<_, _> = sheet
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| range.contains(cell.reference))
        .map(|cell| (cell.reference, cell))
        .collect();
    let mut edits = vec![];
    let mut moved = BTreeMap::new();
    for (offset, source_row) in order.iter().enumerate() {
        let destination_row = range.start.row + offset as u32;
        for column in range.start.column..=range.end.column {
            let from = CellRef::new(column, *source_row);
            let to = CellRef::new(column, destination_row);
            let cell = model.get(&from);
            let style = sheet
                .effective_style_index(from, cell.and_then(|cell| cell.style_index))
                .unwrap_or(0);
            if let Some(raw) = raw.get(&from) {
                let formula = cell
                    .and_then(|cell| cell.formula.as_ref())
                    .map(|formula| jade_xlsx_reader::translate_formula(formula, from, to));
                moved.insert(to, relocate_cell(raw, to, style, formula.as_deref())?);
                edits.push(CellEdit {
                    reference: to,
                    value: NewValue::Blank,
                    style_index: Some(style),
                });
            } else {
                let default = sheet.effective_style_index(to, None).unwrap_or(0);
                edits.push(CellEdit {
                    reference: to,
                    value: if style == default {
                        NewValue::Remove
                    } else {
                        NewValue::Blank
                    },
                    style_index: Some(style),
                });
            }
        }
    }
    let skeleton = crate::patch_sheet(&expanded, &edits)?;
    let splices = cells(&skeleton, range)?
        .into_iter()
        .filter_map(|cell| {
            moved.remove(&cell.at).map(|text| Splice {
                start: cell.start,
                end: cell.end,
                text,
            })
        })
        .collect();
    let written = apply(&skeleton, splices);
    let inverse: BTreeMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(offset, row)| (*row, range.start.row + offset as u32))
        .collect();
    let written = relocate_hyperlinks(&written, range, &inverse)?;
    package.replace(&sheet.part_name, written.into_bytes());
    let rels = package
        .part_str(&jade_opc::rels_part_name(&sheet.part_name))?
        .map(|xml| jade_opc::parse_relationships(&xml))
        .transpose()?
        .unwrap_or_default();
    if rels.target_by_type_suffix("/threadedComment").is_some()
        || rels.target_by_type_suffix("/threadedComments").is_some()
    {
        return Err(error("threaded comments cannot yet be sorted"));
    }
    if let Some(target) = rels.target_by_type_suffix("/comments") {
        let part = jade_opc::resolve_target(&sheet.part_name, target);
        let xml = package
            .part_str(&part)?
            .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
        package.replace(
            &part,
            relocate_comments(&xml, range, &inverse)?.into_bytes(),
        );
    }
    if let Some(target) = rels.target_by_type_suffix("/vmlDrawing") {
        let part = jade_opc::resolve_target(&sheet.part_name, target);
        let xml = package
            .part_str(&part)?
            .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
        package.replace(&part, relocate_vml(&xml, range, &inverse)?.into_bytes());
    }
    crate::mark_for_recalculation(&package.write()?)
}
fn relocate_comments(xml: &str, range: CellRange, order: &BTreeMap<u32, u32>) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut root_ns = None;
    let mut depth = 0;
    let mut in_list = false;
    let mut splices = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = namespace(ns);
                if depth == 0 {
                    root_ns = ns.clone();
                }
                if depth == 1 && ns == root_ns && local.as_ref() == b"commentList" {
                    in_list = !empty;
                }
                if depth == 2 && in_list && ns == root_ns && local.as_ref() == b"comment" {
                    if let Some(at) = attribute(element, b"ref")?
                        .and_then(|v| CellRef::parse(&v).ok())
                        .filter(|at| range.contains(*at))
                    {
                        let to = CellRef::new(at.column, order[&at.row]);
                        splices.push(Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: head_attribute(
                                &xml[start..reader.buffer_position() as usize],
                                "ref",
                                &reference_text(to),
                            )?,
                        });
                    }
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 1 {
                    in_list = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, splices))
}
fn relocate_hyperlinks(xml: &str, range: CellRange, order: &BTreeMap<u32, u32>) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut root_ns = None;
    let mut depth = 0;
    let mut inside = false;
    let mut splices = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = namespace(ns);
                if depth == 0 {
                    root_ns = ns.clone();
                }
                if depth == 1 && ns == root_ns && local.as_ref() == b"hyperlinks" {
                    inside = !empty;
                }
                if depth == 2 && inside && ns == root_ns && local.as_ref() == b"hyperlink" {
                    if let Some(at) = attribute(element, b"ref")?
                        .and_then(|v| CellRange::parse(&v).ok())
                        .filter(|at| intersects(*at, range))
                    {
                        if at.start != at.end {
                            return Err(error("range hyperlinks cannot yet be sorted"));
                        }
                        let to = CellRef::new(at.start.column, order[&at.start.row]);
                        splices.push(Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: head_attribute(
                                &xml[start..reader.buffer_position() as usize],
                                "ref",
                                &reference_text(to),
                            )?,
                        });
                    }
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 1 {
                    inside = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, splices))
}
fn relocate_vml(xml: &str, range: CellRange, order: &BTreeMap<u32, u32>) -> Result<String> {
    const EXCEL_NS: &[u8] = b"urn:schemas-microsoft-com:office:excel";
    let mut reader = NsReader::from_str(xml);
    let mut declarations = Vec::<BTreeMap<String, String>>::new();
    let mut splices = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let excel = matches!(ns,ResolveResult::Bound(ns) if ns.as_ref()==EXCEL_NS);
                let mut scope = declarations.last().cloned().unwrap_or_default();
                for attr in element.attributes() {
                    let attr = attr?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        scope.insert(key, attr.unescape_value()?.into_owned());
                    }
                }
                if !empty
                    && excel
                    && local.as_ref() == b"ClientData"
                    && attribute(element, b"ObjectType")?.as_deref() == Some("Note")
                {
                    let head = reader.buffer_position() as usize;
                    let name = element.name().as_ref().to_vec();
                    reader.read_to_end(QName(&name))?;
                    let end = reader.buffer_position() as usize;
                    let close = xml[..end].rfind("</").unwrap_or(head);
                    let raw = &xml[head..close];
                    let mut bound_head = xml[start..head].to_string();
                    for (key, value) in scope {
                        bound_head = head_attribute(&bound_head, &key, &value)?;
                    }
                    let shift = bound_head.len();
                    let bound = format!("{bound_head}{}", &xml[head..end]);
                    let mut child = NsReader::from_str(&bound);
                    let mut depth = 0;
                    let mut fields = vec![];
                    loop {
                        let event = child.read_event()?;
                        match event {
                            Event::Start(ref field) | Event::Empty(ref field) => {
                                let empty = matches!(event, Event::Empty(_));
                                let (ns, local) = child.resolver().resolve_element(field.name());
                                if !empty
                                    && depth == 1
                                    && matches!(ns,ResolveResult::Bound(ns) if ns.as_ref()==EXCEL_NS)
                                    && matches!(local.as_ref(), b"Row" | b"Column" | b"Anchor")
                                {
                                    let key = String::from_utf8_lossy(local.as_ref()).into_owned();
                                    let a = child.buffer_position() as usize;
                                    child.read_to_end(field.name())?;
                                    let end = child.buffer_position() as usize;
                                    let b = bound[..end].rfind("</").unwrap_or(a);
                                    fields.push((key, a - shift, b - shift));
                                    continue;
                                }
                                depth += usize::from(!empty);
                            }
                            Event::End(_) => {
                                depth -= 1;
                            }
                            Event::Eof => break,
                            _ => {}
                        }
                    }
                    let value = |key: &str| {
                        fields
                            .iter()
                            .find(|(k, _, _)| k == key)
                            .and_then(|(_, a, b)| raw[*a..*b].trim().parse::<u32>().ok())
                    };
                    if let (Some(row), Some(column)) = (value("Row"), value("Column")) {
                        if range.contains(CellRef::new(column, row)) {
                            let to = order[&row];
                            let delta = i64::from(to) - i64::from(row);
                            let mut changes = vec![];
                            for (key, a, b) in fields {
                                if key == "Row" {
                                    changes.push(Splice {
                                        start: a,
                                        end: b,
                                        text: to.to_string(),
                                    });
                                } else if key == "Anchor" {
                                    let mut parts: Vec<_> = raw[a..b]
                                        .split(',')
                                        .map(|v| v.trim().to_string())
                                        .collect();
                                    if parts.len() == 8 {
                                        for index in [2, 6] {
                                            if let Ok(value) = parts[index].parse::<i64>() {
                                                parts[index] =
                                                    (value + delta).clamp(0, 1_048_575).to_string();
                                            }
                                        }
                                        changes.push(Splice {
                                            start: a,
                                            end: b,
                                            text: parts.join(", "),
                                        });
                                    }
                                }
                            }
                            splices.push(Splice {
                                start: head,
                                end: close,
                                text: apply(raw, changes),
                            });
                        }
                    }
                    continue;
                }
                if !empty {
                    declarations.push(scope);
                }
            }
            Event::End(_) => {
                declarations.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, splices))
}

struct SavedSort {
    start: usize,
    end: usize,
    direct: bool,
    range: Option<CellRange>,
    xml: String,
}
fn saved_sorts(xml: &str) -> Result<Vec<SavedSort>> {
    let mut reader = NsReader::from_str(xml);
    let mut root_ns = None;
    let mut stack = Vec::<(String, BTreeMap<String, String>, bool)>::new();
    let mut sorts = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name();
                let (ns, local) = reader.resolver().resolve_element(name);
                let ns = namespace(ns);
                if stack.is_empty() {
                    root_ns = ns.clone();
                }
                let mut declarations = stack
                    .last()
                    .map(|(_, attrs, _)| attrs.clone())
                    .unwrap_or_default();
                for attr in element.attributes() {
                    let attr = attr?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        declarations.insert(key, attr.unescape_value()?.into_owned());
                    }
                }
                let local = String::from_utf8_lossy(local.as_ref()).into_owned();
                if ns == root_ns
                    && local == "sortState"
                    && (stack.len() == 1
                        || stack.len() == 2 && stack[1].0 == "autoFilter" && stack[1].2)
                {
                    let head = reader.buffer_position() as usize;
                    let mut text = xml[start..head].to_string();
                    for (key, value) in declarations {
                        text = head_attribute(&text, &key, &value)?;
                    }
                    let range = attribute(element, b"ref")?.and_then(|v| CellRange::parse(&v).ok());
                    if !empty {
                        reader.read_to_end(QName(name.as_ref()))?;
                    }
                    let end = reader.buffer_position() as usize;
                    text.push_str(&xml[head..end]);
                    sorts.push(SavedSort {
                        start,
                        end,
                        direct: stack.len() == 1,
                        range,
                        xml: text,
                    });
                    continue;
                }
                if !empty {
                    let owned = ns == root_ns;
                    stack.push((local, declarations, owned));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sorts)
}
/// Record standard value-sort keys without serializing unrelated metadata.
pub fn write_sort_state(
    original: &[u8],
    sheet: &Sheet,
    table: Option<&str>,
    range: CellRange,
    keys: &[(u32, bool)],
    case_sensitive: bool,
) -> Result<Vec<u8>> {
    if range.start.row > range.end.row
        || range.start.column > range.end.column
        || keys.is_empty()
        || keys.len() > 64
        || keys
            .iter()
            .any(|(column, _)| *column < range.start.column || *column > range.end.column)
    {
        return Err(error("invalid sort state"));
    }
    let mut package = OpcPackage::open(original)?;
    let table = table
        .map(|name| {
            sheet
                .tables
                .iter()
                .find(|t| t.name == name)
                .ok_or_else(|| error("table is unavailable"))
        })
        .transpose()?;
    let part = table
        .map(|table| table.part_name.as_str())
        .unwrap_or(&sheet.part_name);
    let root_name = if table.is_some() {
        "table"
    } else {
        "worksheet"
    };
    let xml = package
        .part_str(part)?
        .ok_or_else(|| WriteError::MissingPart(part.into()))?;
    let (root, nodes) = children(&xml, root_name)?;
    let saved = saved_sorts(&xml)?;
    let chosen = saved.iter().find(|sort| sort.direct).or_else(|| {
        saved
            .iter()
            .find(|sort| sort.range.is_some_and(|r| intersects(r, range)))
    });
    let prefix = chosen
        .map(|sort| {
            children(&sort.xml, "sortState").map(|(node, _)| namespace_prefix(node.name.as_bytes()))
        })
        .transpose()?
        .unwrap_or_else(|| namespace_prefix(root.name.as_bytes()));
    let mut conditions = String::new();
    for (column, descending) in keys {
        let key = CellRange {
            start: CellRef::new(*column, range.start.row),
            end: CellRef::new(*column, range.end.row),
        };
        conditions.push_str(&format!(
            "<{prefix}sortCondition ref=\"{}\" descending=\"{}\"/>",
            range_text(key),
            u8::from(*descending)
        ));
    }
    let text = if let Some(old) = chosen {
        let (node, children) = children(&old.xml, "sortState")?;
        let head = head_attribute(
            &head_attribute(
                &head_attribute(&old.xml[node.start..node.head], "ref", &range_text(range))?,
                "caseSensitive",
                &u8::from(case_sensitive).to_string(),
            )?,
            "columnSort",
            "0",
        )?;
        if node.empty {
            format!(
                "{}>{conditions}</{}>",
                head.trim_end_matches("/>"),
                node.name
            )
        } else {
            let insertion = children
                .iter()
                .find(|child| matches!(child.local.as_str(), "sortCondition" | "extLst"))
                .map(|child| child.start)
                .unwrap_or(node.close);
            let mut changes = vec![
                Splice {
                    start: node.start,
                    end: node.head,
                    text: head,
                },
                Splice {
                    start: insertion,
                    end: insertion,
                    text: conditions,
                },
            ];
            for child in children
                .iter()
                .filter(|child| child.local == "sortCondition")
            {
                changes.push(Splice {
                    start: child.start,
                    end: child.end,
                    text: String::new(),
                });
            }
            apply(&old.xml, changes)
        }
    } else {
        format!(
            "<{prefix}sortState ref=\"{}\" caseSensitive=\"{}\">{conditions}</{prefix}sortState>",
            range_text(range),
            u8::from(case_sensitive)
        )
    };
    let insertion = chosen.map(|sort| sort.start).unwrap_or_else(|| {
        nodes
            .iter()
            .find(|node| {
                if root_name == "table" {
                    matches!(
                        node.local.as_str(),
                        "tableColumns" | "tableStyleInfo" | "extLst"
                    )
                } else {
                    matches!(
                        node.local.as_str(),
                        "dataConsolidate"
                            | "customSheetViews"
                            | "mergeCells"
                            | "phoneticPr"
                            | "conditionalFormatting"
                            | "dataValidations"
                            | "hyperlinks"
                            | "printOptions"
                            | "pageMargins"
                            | "pageSetup"
                            | "headerFooter"
                            | "rowBreaks"
                            | "colBreaks"
                            | "customProperties"
                            | "cellWatches"
                            | "ignoredErrors"
                            | "smartTags"
                            | "drawing"
                            | "legacyDrawing"
                            | "legacyDrawingHF"
                            | "picture"
                            | "oleObjects"
                            | "controls"
                            | "webPublishItems"
                            | "tableParts"
                            | "extLst"
                    )
                }
            })
            .map(|node| node.start)
            .unwrap_or(root.close)
    });
    let mut splices = vec![Splice {
        start: insertion,
        end: insertion,
        text,
    }];
    for sort in saved
        .iter()
        .filter(|sort| sort.direct || sort.range.is_some_and(|r| intersects(r, range)))
    {
        splices.push(Splice {
            start: sort.start,
            end: sort.end,
            text: String::new(),
        });
    }
    package.replace(part, apply(&xml, splices).into_bytes());
    package.write().map_err(Into::into)
}
