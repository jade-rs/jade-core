//! Worksheet link authoring. Range subtraction preserves the links outside the
//! selection; relationship edits never overwrite shared or foreign records.
use crate::{
    patch::{apply, escape, namespace_prefix, Splice},
    xml_edit, xml_edit_ns, Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{CellRange, Hyperlink};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};
use std::collections::HashSet;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT_REL: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const FOLLOWING: &[&[u8]] = &[
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];
fn bad(s: &str) -> WriteError {
    WriteError::Hyperlink(s.into())
}
pub(crate) fn validate_range(r: CellRange) -> Result<()> {
    if r.start.column > r.end.column
        || r.start.row > r.end.row
        || r.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
        || r.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
    {
        return Err(bad("invalid cell range"));
    }
    Ok(())
}
fn attr(xml: &str, key: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                for a in e.attributes() {
                    let a = a?;
                    if a.key.as_ref() == key {
                        return Ok(Some(a.unescape_value()?.into_owned()));
                    }
                }
                return Ok(None);
            }
            Event::Eof => return Err(WriteError::NoRoot("hyperlink")),
            _ => {}
        }
    }
}
fn root_prefix(xml: &str) -> Result<String> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => return Ok(namespace_prefix(e.name().as_ref())),
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}
fn string(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|s| {
        jade_xlsx_reader::xstring::encode_xstring(s)
            .replace('\n', "_x000A_")
            .replace('\t', "_x0009_")
    })
}
fn relationship_ids(xml: &str) -> Result<HashSet<String>> {
    let mut reader = NsReader::from_str(xml);
    let mut ids = HashSet::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                for a in e.attributes() {
                    let a = a?;
                    let (ns, name) = reader.resolver().resolve_attribute(a.key);
                    if name.as_ref() == b"id"
                        && (matches!(&ns,ResolveResult::Bound(n) if n.as_ref()==REL.as_bytes() || n.as_ref()==STRICT_REL.as_bytes())
                            || (a.key.as_ref() == b"r:id"
                                && matches!(&ns, ResolveResult::Unknown(_))))
                    {
                        ids.insert(a.unescape_value()?.into_owned());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ids)
}
fn clear(xml: &str, ranges: &[CellRange]) -> Result<(String, HashSet<String>)> {
    let Some((_, _, container)) = xml_edit_ns::children(xml, b"hyperlinks")?
        .into_iter()
        .next()
    else {
        return Ok((xml.into(), HashSet::new()));
    };
    let mut splices = Vec::new();
    let mut removed = HashSet::new();
    for (start, end, context) in xml_edit_ns::children(&container.text, b"hyperlink")? {
        let Some(reference) = attr(&context.text, b"ref")? else {
            continue;
        };
        let range =
            CellRange::parse(&reference).map_err(|_| bad("invalid existing hyperlink range"))?;
        let mut remaining = vec![range];
        for cut in ranges {
            remaining = remaining
                .into_iter()
                .flat_map(|r| crate::conditional_formats::subtract(r, *cut))
                .collect();
        }
        if remaining == [range] {
            continue;
        }
        removed.extend(relationship_ids(&context.text)?);
        let mut text = String::new();
        for r in remaining {
            let updated = xml_edit::root_attributes(
                &context.text,
                &[("ref", Some(crate::shift::range_text(r)))],
            )?;
            text.push_str(&context.finish(&updated)?);
        }
        splices.push(Splice { start, end, text });
    }
    if splices.is_empty() {
        return Ok((xml.into(), removed));
    }
    let updated = apply(&container.text, splices);
    let updated = container.finish(&updated)?;
    let updated = xml_edit_ns::set_child(xml, b"hyperlinks", |_| updated, FOLLOWING)?;
    Ok((updated, removed))
}
/// Apply a link over its whole range, replacing overlapping links only there.
/// Cell values and formatting are deliberately left to the caller.
pub fn set_hyperlink(package: &mut OpcPackage, sheet_part: &str, link: &Hyperlink) -> Result<bool> {
    validate_range(link.reference)?;
    if link.target.as_deref().is_none_or(|s| s.is_empty())
        && link.location.as_deref().is_none_or(|s| s.is_empty())
    {
        return Err(bad("a target or location is required"));
    }
    if let Some(target) = &link.target {
        if target.is_empty()
            || target
                .chars()
                .any(|c| !crate::patch::is_xml_10_character(c))
        {
            return Err(bad("invalid external target"));
        }
    }
    let original = package.required_part_str(sheet_part)?;
    let strict = {
        let mut reader = NsReader::from_str(&original);
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    break matches!(reader.resolver().resolve_element(e.name()).0, ResolveResult::Bound(ns) if ns.as_ref() == b"http://purl.oclc.org/ooxml/spreadsheetml/main")
                }
                Event::Eof => return Err(WriteError::NoRoot("worksheet")),
                _ => {}
            }
        }
    };
    let rel_ns = if strict { STRICT_REL } else { REL };
    let rels_part = jade_opc::rels_part_name(sheet_part);
    let old_rels = package.part_str(&rels_part)?;
    let mut rels = old_rels.clone();
    let mut id = None;
    if let Some(target) = &link.target {
        let parsed = old_rels
            .as_deref()
            .map(jade_opc::parse_relationships)
            .transpose()?
            .unwrap_or_default();
        id = parsed
            .all()
            .into_iter()
            .find(|r| {
                r.relationship_type == format!("{rel_ns}/hyperlink")
                    && r.target == *target
                    && r.external
            })
            .map(|r| r.id.clone());
        if id.is_none() {
            let xml=old_rels.as_deref().unwrap_or("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>");
            let key = (1u64..)
                .map(|i| format!("rId{i}"))
                .find(|id| parsed.relationship(id).is_none())
                .unwrap();
            let prefix = root_prefix(xml)?;
            let entry=format!("<{prefix}Relationship Id=\"{key}\" Type=\"{rel_ns}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",escape(target));
            rels = Some(xml_edit::append_child(xml, &entry)?);
            id = Some(key);
        }
    }
    // Reuse an exact-range element to retain unknown attributes and children.
    let mut existing = None;
    if let Some((_, _, container)) = xml_edit_ns::children(&original, b"hyperlinks")?
        .into_iter()
        .next()
    {
        for (_, _, entry) in xml_edit_ns::children(&container.text, b"hyperlink")? {
            if attr(&entry.text, b"ref")?.and_then(|r| CellRange::parse(&r).ok())
                == Some(link.reference)
            {
                existing = Some(entry);
                break;
            }
        }
    }
    let (cleared, removed) = clear(&original, &[link.reference])?;
    let updated = xml_edit_ns::edit_child(&cleared, b"hyperlinks", FOLLOWING, |container| {
        let prefix = root_prefix(container)?;
        let mut entry = existing
            .as_ref()
            .map(|x| x.text.clone())
            .unwrap_or_else(|| format!("<{prefix}hyperlink/>"));
        let mut reader = NsReader::from_str(&entry);
        let mut keys = Vec::new();
        let mut reuse_key = None;
        let mut prefixes = HashSet::new();
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    for a in e.attributes() {
                        let a = a?;
                        let (ns, name) = reader.resolver().resolve_attribute(a.key);
                        if name.as_ref() == b"id"
                            && (matches!(&ns,ResolveResult::Bound(n) if n.as_ref()==REL.as_bytes() || n.as_ref()==STRICT_REL.as_bytes())
                                || (a.key.as_ref() == b"r:id"
                                    && matches!(&ns, ResolveResult::Unknown(_))))
                        {
                            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                            if matches!(&ns, ResolveResult::Bound(n) if n.as_ref() == rel_ns.as_bytes())
                            {
                                reuse_key = Some(key.clone());
                            }
                            keys.push(key);
                        }
                    }
                    prefixes.extend(reader.resolver().bindings().filter_map(|(p, _)| match p {
                        quick_xml::name::PrefixDeclaration::Named(p) => {
                            Some(String::from_utf8_lossy(p).into_owned())
                        }
                        _ => None,
                    }));
                    break;
                }
                Event::Eof => break,
                _ => {}
            }
        }
        let prefix = (0u64..)
            .map(|i| format!("jadeLink{i}"))
            .find(|p| !prefixes.contains(p))
            .unwrap();
        let mut attrs = vec![
            (
                "ref".to_string(),
                Some(crate::shift::range_text(link.reference)),
            ),
            ("location".into(), string(&link.location)),
            ("display".into(), string(&link.display)),
            ("tooltip".into(), string(&link.tooltip)),
        ];
        attrs.extend(keys.into_iter().map(|key| (key, None)));
        if let Some(id) = id {
            if let Some(key) = reuse_key {
                attrs.retain(|(old, _)| old != &key);
                attrs.push((key, Some(id)));
            } else {
                attrs.push((format!("xmlns:{prefix}"), Some(rel_ns.into())));
                attrs.push((format!("{prefix}:id"), Some(id)));
            }
        }
        entry = xml_edit::root_attributes(
            &entry,
            &attrs
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )?;
        if let Some(existing) = existing {
            entry = existing.finish(&entry)?;
        }
        xml_edit::append_child(container, &entry)
    })?;
    let rels = prune(rels, &updated, &removed)?;
    if updated == original && rels == old_rels {
        return Ok(false);
    }
    package.replace(sheet_part, updated.into_bytes());
    if rels != old_rels {
        if let Some(xml) = rels {
            package.replace(&rels_part, xml.into_bytes());
        }
    }
    Ok(true)
}
/// Remove only links in the selected ranges. Text and formats are retained.
pub fn clear_hyperlinks(
    package: &mut OpcPackage,
    sheet_part: &str,
    ranges: &[CellRange],
) -> Result<bool> {
    for r in ranges {
        validate_range(*r)?;
    }
    let original = package.required_part_str(sheet_part)?;
    let (updated, removed) = clear(&original, ranges)?;
    if updated == original {
        return Ok(false);
    }
    let part = jade_opc::rels_part_name(sheet_part);
    let old_rels = package.part_str(&part)?;
    let rels = prune(old_rels.clone(), &updated, &removed)?;
    package.replace(sheet_part, updated.into_bytes());
    if rels != old_rels {
        if let Some(xml) = rels {
            package.replace(&part, xml.into_bytes());
        }
    }
    Ok(true)
}
fn prune(rels: Option<String>, sheet: &str, removed: &HashSet<String>) -> Result<Option<String>> {
    let Some(xml) = rels else {
        return Ok(None);
    };
    let retained = relationship_ids(sheet)?;
    let mut splices = Vec::new();
    for (start, end, context) in xml_edit_ns::children(&xml, b"Relationship")? {
        if let (Some(id), Some(kind)) = (attr(&context.text, b"Id")?, attr(&context.text, b"Type")?)
        {
            if removed.contains(&id) && !retained.contains(&id) && kind.ends_with("/hyperlink") {
                splices.push(Splice {
                    start,
                    end,
                    text: String::new(),
                });
            }
        }
    }
    Ok(Some(apply(&xml, splices)))
}
