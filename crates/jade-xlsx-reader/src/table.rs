//! SpreadsheetML table definitions and column calculation/total metadata.
use crate::{
    error::Result,
    model::{CellRange, Table, TableColumn},
    xstring::decode_xstring,
};
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
fn attr(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        if a.key.as_ref() == key {
            return Ok(Some(a.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
fn flag(e: &BytesStart<'_>, key: &[u8]) -> Result<bool> {
    Ok(attr(e, key)?.is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true")))
}
pub fn parse_table(part_name: &str, xml: &str) -> Result<Option<Table>> {
    let (filter, _) = crate::auto_filter::parse(xml, b"table")?;
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut root_ns = None;
    let mut table: Option<Table> = None;
    let mut columns = false;
    let mut column: Option<TableColumn> = None;
    let mut formula: Option<(Vec<u8>, String)> = None;
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let ns = match ns {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
                if depth == 0 && local.as_ref() == b"table" {
                    root_ns = ns.clone().filter(|n| {
                        matches!(
                            n.as_slice(),
                            b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                        )
                    });
                }
                let own = root_ns.is_some() && ns == root_ns;
                if own {
                    match (depth, local.as_ref()) {
                        (0, b"table") => {
                            let Some(reference) = attr(e, b"ref")? else {
                                return Ok(None);
                            };
                            let name = attr(e, b"name")?.unwrap_or_default();
                            let display_name =
                                attr(e, b"displayName")?.unwrap_or_else(|| name.clone());
                            table = Some(Table {
                                part_name: part_name.into(),
                                name,
                                display_name,
                                reference: CellRange::parse(&reference)?,
                                header_row_count: attr(e, b"headerRowCount")?
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(1),
                                totals_row_count: attr(e, b"totalsRowCount")?
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0),
                                columns: Vec::new(),
                                column_properties: Vec::new(),
                                style_name: None,
                                show_first_column: false,
                                show_last_column: false,
                                show_row_stripes: false,
                                show_column_stripes: false,
                                auto_filter: false,
                                filter: None,
                            });
                        }
                        (1, b"tableColumns") => columns = !empty,
                        (2, b"tableColumn") if columns => {
                            if let Some(t) = table.as_mut() {
                                t.columns
                                    .push(decode_xstring(&attr(e, b"name")?.unwrap_or_default()));
                                column = Some(TableColumn {
                                    id: attr(e, b"id")?.and_then(|s| s.parse().ok()).unwrap_or(0),
                                    totals_function: attr(e, b"totalsRowFunction")?,
                                    totals_label: attr(e, b"totalsRowLabel")?
                                        .map(|s| decode_xstring(&s)),
                                    ..Default::default()
                                });
                                if empty {
                                    t.column_properties.push(column.take().unwrap());
                                }
                            }
                        }
                        (3, b"calculatedColumnFormula" | b"totalsRowFormula")
                            if column.is_some() =>
                        {
                            formula = Some((local.as_ref().to_vec(), String::new()));
                            if empty {
                                finish_formula(&mut column, &mut formula);
                            }
                        }
                        (1, b"tableStyleInfo") => {
                            if let Some(t) = table.as_mut() {
                                t.style_name = attr(e, b"name")?;
                                t.show_first_column = flag(e, b"showFirstColumn")?;
                                t.show_last_column = flag(e, b"showLastColumn")?;
                                t.show_row_stripes = flag(e, b"showRowStripes")?;
                                t.show_column_stripes = flag(e, b"showColumnStripes")?;
                            }
                        }
                        _ => {}
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::Text(e) if depth == 4 => {
                if let Some((_, text)) = formula.as_mut() {
                    text.push_str(&e.decode()?);
                }
            }
            Event::CData(e) if depth == 4 => {
                if let Some((_, text)) = formula.as_mut() {
                    text.push_str(&e.decode()?);
                }
            }
            Event::GeneralRef(e) if depth == 4 => {
                if let Some((_, text)) = formula.as_mut() {
                    let name = e.decode()?;
                    if let Ok(value) = quick_xml::escape::unescape(&format!("&{name};")) {
                        text.push_str(&value);
                    }
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if depth == 3
                    && formula
                        .as_ref()
                        .is_some_and(|(name, _)| *name == e.local_name().as_ref())
                {
                    finish_formula(&mut column, &mut formula);
                }
                if depth == 2 && e.local_name().as_ref() == b"tableColumn" {
                    if let (Some(t), Some(c)) = (table.as_mut(), column.take()) {
                        t.column_properties.push(c);
                    }
                }
                if depth == 1 && e.local_name().as_ref() == b"tableColumns" {
                    columns = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if let Some(t) = table.as_mut() {
        t.auto_filter = filter.is_some();
        t.filter = filter;
    }
    Ok(table)
}
fn finish_formula(column: &mut Option<TableColumn>, formula: &mut Option<(Vec<u8>, String)>) {
    if let (Some(c), Some((kind, text))) = (column.as_mut(), formula.take()) {
        if kind == b"calculatedColumnFormula" {
            c.calculated_formula = Some(text);
        } else {
            c.totals_formula = Some(text);
        }
    }
}

/// Relationship identity of a detached tablePart with in-scope bindings.
/// Retain the conventional undeclared r:id tolerance used by older files.
pub fn table_part_id(xml: &str) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => return relationship_id(&reader, &e),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}
fn relationship_id(reader: &NsReader<&[u8]>, e: &BytesStart<'_>) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        let (ns, name) = reader.resolver().resolve_attribute(a.key);
        let recognized = matches!(&ns,ResolveResult::Bound(n) if matches!(n.as_ref(),
            b"http://schemas.openxmlformats.org/officeDocument/2006/relationships" |
            b"http://purl.oclc.org/ooxml/officeDocument/relationships"))
            || (a.key.as_ref() == b"r:id" && matches!(ns, ResolveResult::Unknown(_)));
        if recognized && name.as_ref() == b"id" {
            return Ok(Some(a.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
/// Tables actually referenced by a worksheet's direct tableParts collection,
/// in document order. Relationship-only, external, foreign and duplicate
/// targets do not introduce extra owned tables.
pub fn referenced_table_parts(
    xml: &str,
    rels: &jade_opc::Relationships,
    source: &str,
) -> Result<Vec<String>> {
    if !xml.contains("tablePart") {
        return Ok(Vec::new());
    }
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut root = None;
    let mut container = false;
    let mut parts = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    loop {
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match &event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(n) => Some(n.as_ref().to_vec()),
                    _ => None,
                };
                if depth == 0 && e.local_name().as_ref() == b"worksheet" {
                    root = ns.clone().filter(|n| {
                        matches!(
                            n.as_slice(),
                            b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                        )
                    });
                }
                let own = root.is_some() && root == ns;
                if depth == 1 && own && e.local_name().as_ref() == b"tableParts" {
                    container = !empty;
                }
                if depth == 2 && container && own && e.local_name().as_ref() == b"tablePart" {
                    if let Some(rel) =
                        relationship_id(&reader, e)?.and_then(|id| rels.relationship(&id))
                    {
                        if !rel.external && matches!(rel.relationship_type.as_str(),
                        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" |
                        "http://purl.oclc.org/ooxml/officeDocument/relationships/table") {
                        let part=jade_opc::resolve_target(source,&rel.target);
                        if seen.insert(part.clone()) {parts.push(part);}
                    }
                    }
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    container = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parts)
}
