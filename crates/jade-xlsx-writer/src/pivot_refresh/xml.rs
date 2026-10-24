use super::{bad, Result};
use crate::{
    patch::{apply, attribute, escape, namespace_prefix, Splice},
    xml_edit, xml_edit_ns,
};
use quick_xml::{events::Event, Reader};
pub(super) fn prefix(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => return Ok(namespace_prefix(e.name().as_ref())),
            Event::Eof => return Err(bad("empty pivot XML")),
            _ => {}
        }
    }
}
pub(super) fn strict(xml: &str) -> Result<bool> {
    let mut reader = quick_xml::NsReader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                return match reader.resolver().resolve_element(e.name()).0 {
                    quick_xml::name::ResolveResult::Unbound => Ok(false),
                    quick_xml::name::ResolveResult::Bound(ns)
                        if ns.as_ref()
                            == b"http://schemas.openxmlformats.org/spreadsheetml/2006/main" =>
                    {
                        Ok(false)
                    }
                    quick_xml::name::ResolveResult::Bound(ns)
                        if ns.as_ref() == b"http://purl.oclc.org/ooxml/spreadsheetml/main" =>
                    {
                        Ok(true)
                    }
                    _ => Err(bad("unrecognized pivot root namespace")),
                }
            }
            Event::Eof => return Err(bad("empty pivot XML")),
            _ => {}
        }
    }
}
pub(super) fn attr(xml: &str, name: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => return attribute(&e, name.as_bytes()),
            Event::Eof => return Err(bad("empty pivot XML")),
            _ => {}
        }
    }
}
pub(super) fn child(xml: &str, name: &str) -> Result<Option<xml_edit_ns::ContextXml>> {
    let mut all = xml_edit_ns::children(xml, name.as_bytes())?;
    if all.len() > 1 {
        return Err(bad(format!("duplicate owned {name}")));
    }
    Ok(all.pop().map(|(_, _, c)| c))
}
pub(super) fn list(
    xml: &str,
    container: &str,
    item: &str,
    rows: impl FnOnce(&str) -> Result<Vec<String>>,
) -> Result<String> {
    let existing = child(xml, container)?;
    let contextual = existing
        .as_ref()
        .map(|c| c.text.clone())
        .unwrap_or_else(|| format!("<{}{container}/>", prefix(xml).unwrap_or_default()));
    let p = prefix(&contextual)?;
    let new = rows(&p)?;
    if new.is_empty() {
        if existing.is_none() {
            return Ok(xml.to_string());
        }
        // These optional containers require at least one entry in OOXML.
        // Removing the last owned entry removes its container; unknown
        // container metadata cannot silently disappear with it.
        let remaining = replace_entries(&contextual, item, "")?;
        let mut reader = Reader::from_str(&remaining);
        let mut depth = 0usize;
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    if depth != 0 {
                        return Err(bad("uninterpreted empty pivot list metadata"));
                    }
                    for a in e.attributes() {
                        let a = a?;
                        let k = a.key.as_ref();
                        if k != b"count" && k != b"xmlns" && !k.starts_with(b"xmlns:") {
                            return Err(bad("uninterpreted empty pivot list attributes"));
                        }
                    }
                    depth += 1;
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Text(t) if !t.decode()?.trim().is_empty() => {
                    return Err(bad("uninterpreted empty pivot list text"))
                }
                Event::CData(t) if !t.decode()?.trim().is_empty() => {
                    return Err(bad("uninterpreted empty pivot list text"))
                }
                Event::GeneralRef(_) => return Err(bad("uninterpreted empty pivot list text")),
                Event::Eof => break,
                _ => {}
            }
        }
        return xml_edit_ns::set_child(xml, container.as_bytes(), |_| String::new(), &[]);
    }
    let mut out = replace_entries(&contextual, item, &new.join(""))?;
    out = xml_edit::root_attributes(&out, &[("count", Some(new.len().to_string()))])?;
    let out = if let Some(c) = existing {
        c.finish(&out)?
    } else {
        out
    };
    // Pivot children are schema-ordered. Insert a missing list immediately
    // before the first known following child rather than after extLst.
    let order = [
        "location",
        "pivotFields",
        "rowFields",
        "rowItems",
        "colFields",
        "colItems",
        "pageFields",
        "dataFields",
        "formats",
        "conditionalFormats",
        "chartFormats",
        "pivotHierarchies",
        "pivotTableStyleInfo",
        "filters",
        "rowHierarchiesUsage",
        "colHierarchiesUsage",
        "extLst",
    ];
    let following = order
        .iter()
        .position(|n| *n == container)
        .map(|i| {
            order[i + 1..]
                .iter()
                .map(|s| s.as_bytes())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    xml_edit_ns::set_child(xml, container.as_bytes(), |_| out, &following)
}
pub(super) fn replace_entries(xml: &str, item: &str, new: &str) -> Result<String> {
    let old = xml_edit_ns::children(xml, item.as_bytes())?;
    if old.is_empty() {
        return xml_edit::append_child(xml, new);
    }
    let mut edits = Vec::new();
    for (i, (start, end, _)) in old.into_iter().enumerate() {
        edits.push(Splice {
            start,
            end,
            text: if i == 0 { new.into() } else { String::new() },
        });
    }
    Ok(apply(xml, edits))
}
pub(super) fn xstring_attribute(value: &str) -> String {
    jade_xlsx_reader::xstring::encode_xstring(value)
        .replace('\n', "_x000A_")
        .replace('\t', "_x0009_")
}
pub(super) fn text_attr(value: &str) -> String {
    escape(&xstring_attribute(value))
}

/// These entries are rebuilt, so unknown per-entry metadata cannot safely be
/// attached to a different output row. Refuse it rather than silently drop it.
pub(super) fn flat_entries(
    xml: &str,
    entry: &str,
    attributes: &[&str],
    leaves: &[&str],
    leaf_attributes: &[&str],
) -> Result<()> {
    for (_, _, fragment) in xml_edit_ns::children(xml, entry.as_bytes())? {
        let mut reader = quick_xml::NsReader::from_str(&fragment.text);
        let mut depth = 0usize;
        let mut root_namespace = None;
        loop {
            let event = reader.read_event()?;
            let empty = matches!(event, Event::Empty(_));
            match event {
                Event::Start(e) | Event::Empty(e) => {
                    let ns = match reader.resolver().resolve_element(e.name()).0 {
                        quick_xml::name::ResolveResult::Unbound => Some(Vec::new()),
                        quick_xml::name::ResolveResult::Bound(n) => Some(n.as_ref().to_vec()),
                        _ => None,
                    };
                    if depth == 0 {
                        root_namespace = ns.clone();
                    }
                    if depth > 1
                        || ns != root_namespace
                        || depth == 1
                            && !leaves
                                .iter()
                                .any(|name| name.as_bytes() == e.local_name().as_ref())
                    {
                        return Err(bad("uninterpreted pivot entry children cannot be rebuilt"));
                    }
                    let permitted = if depth == 0 {
                        attributes
                    } else {
                        leaf_attributes
                    };
                    for a in e.attributes() {
                        let a = a?;
                        let key = a.key.as_ref();
                        if key == b"xmlns" || key.starts_with(b"xmlns:") {
                            continue;
                        }
                        if !permitted.iter().any(|name| name.as_bytes() == key) {
                            return Err(bad(
                                "uninterpreted pivot entry attributes cannot be rebuilt",
                            ));
                        }
                    }
                    if !empty {
                        depth += 1;
                    }
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                }
                Event::Text(t) if !t.decode()?.trim().is_empty() => {
                    return Err(bad("unexpected pivot entry text"))
                }
                Event::CData(t) if !t.decode()?.trim().is_empty() => {
                    return Err(bad("unexpected pivot entry CDATA"))
                }
                Event::GeneralRef(_) => return Err(bad("unexpected pivot entry entity")),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(())
}
