//! Preserving editable-range metadata and structural coverage reconciliation.
use crate::{
    patch::{apply, escape, namespace_prefix, Splice},
    xml_edit, xml_edit_ns, Result, WriteError,
};
use jade_opc::{protection::PasswordVerifier, OpcPackage};
use jade_xlsx_reader::{
    protected_ranges::{sources, RangeRecord, XM},
    CellRange,
};
use quick_xml::{events::Event, Reader};
use std::collections::BTreeMap;
fn bad(s: &str) -> WriteError {
    WriteError::ProtectedRange(s.into())
}
fn prefix(xml: &str) -> Result<String> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => return Ok(namespace_prefix(e.name().as_ref())),
            Event::Eof => return Err(bad("missing container root")),
            _ => {}
        }
    }
}
fn name_key(name: &str) -> String {
    name.to_lowercase()
}
pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first = chars
        .next()
        .ok_or_else(|| bad("a range name is required"))?;
    if name.chars().count() > 255
        || !(first.is_alphabetic() || matches!(first, '_' | '\\'))
        || !chars.all(|c| {
            c.is_alphabetic()
                || c.is_numeric()
                || c.is_whitespace()
                || matches!(c, '_' | '\\' | '.' | '?')
        })
        || name
            .chars()
            .any(|c| c.is_control() || !crate::patch::is_xml_10_character(c))
    {
        return Err(bad("invalid range name"));
    }
    Ok(())
}
fn references(ranges: &[CellRange]) -> Result<String> {
    if ranges.is_empty() {
        return Err(bad("at least one range is required"));
    }
    for r in ranges {
        if r.start.column > r.end.column
            || r.start.row > r.end.row
            || r.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
            || r.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        {
            return Err(bad("invalid range coverage"));
        }
    }
    Ok(ranges
        .iter()
        .map(|r| crate::shift::range_text(*r))
        .collect::<Vec<_>>()
        .join(" "))
}
fn password_updates(
    password: &str,
    salt: &[u8; 16],
) -> Result<Vec<(&'static str, Option<String>)>> {
    let p = if password.is_empty() {
        PasswordVerifier::default()
    } else {
        PasswordVerifier::new(password, salt)?
    };
    Ok(vec![
        ("password", None),
        ("algorithmName", p.algorithm),
        ("hashValue", p.hash),
        ("saltValue", p.salt),
        ("spinCount", p.spin_count),
    ])
}
pub(crate) fn moved_record(xml: &str, extended: bool, ranges: &[CellRange]) -> Result<String> {
    let text = references(ranges)?;
    if !extended {
        return xml_edit::root_attributes(xml, &[("sqref", Some(text))]);
    }
    let leaves = xml_edit_ns::children_ns(xml, b"sqref", XM)?;
    if leaves.len() != 1 {
        return Err(bad("an x14 range needs exactly one direct xm:sqref"));
    }
    let (start, end, c) = &leaves[0];
    let moved = crate::cell_shift::text_content(&c.text, &text)?;
    Ok(apply(
        xml,
        vec![Splice {
            start: *start,
            end: *end,
            text: c.finish(&moved)?,
        }],
    ))
}
fn editable(original: &[u8], part: &str) -> Result<(OpcPackage, String)> {
    if crate::sheet_protection::sheet_protection(original, part)?.is_some_and(|p| p.protected) {
        return Err(bad("unprotect the sheet before changing editable ranges"));
    }
    let p = OpcPackage::open(original)?;
    let xml = p.required_part_str(part)?;
    if sources(&xml)?.unsupported_mce {
        return Err(bad(
            "editable ranges inside AlternateContent cannot be edited",
        ));
    }
    Ok((p, xml))
}
const FOLLOWING: &[&[u8]] = &[
    b"scenarios",
    b"autoFilter",
    b"sortState",
    b"dataConsolidate",
    b"customSheetViews",
    b"mergeCells",
    b"phoneticPr",
    b"conditionalFormatting",
    b"dataValidations",
    b"hyperlinks",
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
/// Create/update/rename one logical range. None preserves existing credentials;
/// Some(("",salt)) explicitly removes them. Sheet protection must first be off.
pub fn set_editable_range(
    original: &[u8],
    part: &str,
    old_name: Option<&str>,
    name: &str,
    ranges: &[CellRange],
    password: Option<(&str, &[u8; 16])>,
) -> Result<Vec<u8>> {
    validate_name(name)?;
    let reference = references(ranges)?;
    let (mut p, xml) = editable(original, part)?;
    let source = sources(&xml)?;
    let old_key = old_name.map(name_key);
    let new_key = name_key(name);
    if source
        .records
        .iter()
        .any(|r| name_key(&r.value.name) == new_key && old_key.as_deref() != Some(new_key.as_str()))
    {
        return Err(bad("another editable range has this name"));
    }
    let targets = source
        .records
        .iter()
        .filter(|r| old_key.as_deref() == Some(name_key(&r.value.name).as_str()))
        .collect::<Vec<_>>();
    if old_name.is_some() && targets.is_empty() {
        return Err(bad("no editable range has this name"));
    }
    if targets.iter().any(|r| r.value.ambiguous) {
        return Err(bad("malformed coverage must be deleted before replacement"));
    }
    if password.is_none()
        && targets.first().is_some_and(|r| r.value.name == name)
        && targets
            .iter()
            .all(|r| jade_xlsx_reader::protected_ranges::same_coverage(&r.value.ranges, ranges))
    {
        return Ok(original.to_vec());
    }
    let name = jade_xlsx_reader::xstring::encode_xstring(name);
    let mut updates = vec![("name", Some(name))];
    if let Some((password, salt)) = password {
        updates.extend(password_updates(password, salt)?);
    }
    let updated = if old_name.is_none() {
        let standard = source.containers.iter().filter(|c| !c.extended).count();
        if standard > 1 {
            return Err(bad("multiple standard editable-range containers"));
        }
        let mut attrs = String::new();
        for (key, value) in &updates {
            if let Some(value) = value {
                attrs.push_str(&format!(" {key}=\"{}\"", escape(value)));
            }
        }
        xml_edit_ns::edit_child(&xml, b"protectedRanges", FOLLOWING, |container| {
            let prefix = prefix(container)?;
            xml_edit::append_child(
                container,
                &format!(
                    "<{prefix}protectedRange{attrs} sqref=\"{}\"/>",
                    escape(&reference)
                ),
            )
        })?
    } else {
        let mut edits = Vec::new();
        for record in targets {
            if record.value.ambiguous {
                return Err(bad("malformed coverage must be deleted before replacement"));
            }
            let c = xml_edit_ns::fragment(&xml, record.span.clone())?;
            let moved = moved_record(
                &c.text,
                source.containers[record.container].extended,
                ranges,
            )?;
            let moved = xml_edit::root_attributes(&moved, &updates)?;
            edits.push(Splice {
                start: record.span.start,
                end: record.span.end,
                text: c.finish(&moved)?,
            });
        }
        apply(&xml, edits)
    };
    if updated == xml {
        return Ok(original.to_vec());
    }
    p.replace(part, updated.into_bytes());
    Ok(p.write()?)
}
/// Delete all owned standard/x14 representations of the selected sheet-local name.
pub fn delete_editable_range(original: &[u8], part: &str, name: &str) -> Result<Vec<u8>> {
    let (mut p, xml) = editable(original, part)?;
    let source = sources(&xml)?;
    let key = name_key(name);
    let edits = source
        .records
        .iter()
        .filter(|r| name_key(&r.value.name) == key)
        .map(|r| Splice {
            start: r.span.start,
            end: r.span.end,
            text: String::new(),
        })
        .collect::<Vec<_>>();
    if edits.is_empty() {
        return Err(bad("no editable range has this name"));
    }
    let updated = cleanup(&apply(&xml, edits))?;
    p.replace(part, updated.into_bytes());
    Ok(p.write()?)
}
/// Rewrite only interpreted containers. The whole-axis walker consumes every
/// protectedRanges subtree, so opaque records never enter its local-name pass.
pub(crate) fn structure_containers(
    xml: &str,
    changed: bool,
    map: &impl Fn(CellRange) -> Vec<CellRange>,
) -> Result<BTreeMap<usize, String>> {
    let source = sources(xml)?;
    if changed && source.unsupported_mce {
        return Err(bad("cannot shift editable ranges inside AlternateContent"));
    }
    let mut out = BTreeMap::new();
    if !changed {
        return Ok(out);
    }
    let mut groups: BTreeMap<usize, Vec<&RangeRecord>> = BTreeMap::new();
    for record in &source.records {
        groups.entry(record.container).or_default().push(record);
    }
    for (index, records) in groups {
        let container = &source.containers[index];
        let mut edits = Vec::new();
        for record in records {
            if record.value.ambiguous {
                continue;
            }
            let ranges = record
                .value
                .ranges
                .iter()
                .flat_map(|r| map(*r))
                .collect::<Vec<_>>();
            if ranges == record.value.ranges {
                continue;
            }
            let replacement = if ranges.is_empty() {
                String::new()
            } else {
                let c = xml_edit_ns::fragment(xml, record.span.clone())?;
                c.finish(&moved_record(&c.text, container.extended, &ranges)?)?
            };
            edits.push(Splice {
                start: record.span.start - container.span.start,
                end: record.span.end - container.span.start,
                text: replacement,
            });
        }
        let updated = apply(&xml[container.span.clone()], edits);
        out.insert(
            container.span.start,
            if plain_empty(&updated, &[])? {
                String::new()
            } else {
                updated
            },
        );
    }
    Ok(out)
}

fn plain_empty(xml: &str, allowed: &[&[u8]]) -> Result<bool> {
    let mut r = Reader::from_str(xml);
    let mut depth = 0;
    loop {
        match r.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if depth != 0 {
                    return Ok(false);
                }
                for a in e.attributes() {
                    let a = a?;
                    if a.key.as_ref() != b"xmlns"
                        && !a.key.as_ref().starts_with(b"xmlns:")
                        && !allowed.contains(&a.key.as_ref())
                    {
                        return Ok(false);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth -= 1,
            Event::Text(t) if t.decode()?.trim().is_empty() => {}
            Event::Eof => return Ok(true),
            _ => return Ok(false),
        }
    }
}
/// Remove only empty owned containers/wrappers with no opaque data to preserve.
pub(crate) fn cleanup(xml: &str) -> Result<String> {
    let source = sources(xml)?;
    let mut edits = Vec::new();
    for container in source.containers {
        if plain_empty(&xml[container.span.clone()], &[])? {
            edits.push(Splice {
                start: container.span.start,
                end: container.span.end,
                text: String::new(),
            });
        }
    }
    let mut xml = apply(xml, edits);
    let mut edits = Vec::new();
    for (start, end, list) in xml_edit_ns::children(&xml, b"extLst")? {
        let mut children = Vec::new();
        for (a, b, extension) in xml_edit_ns::children(&list.text, b"ext")? {
            let mut r = Reader::from_str(&extension.text);
            let mut known = false;
            loop {
                match r.read_event()? {
                    Event::Start(e) | Event::Empty(e) => {
                        for attr in e.attributes() {
                            let attr = attr?;
                            if attr.key.as_ref() == b"uri"
                                && attr.unescape_value()?.eq_ignore_ascii_case(
                                    jade_xlsx_reader::protected_ranges::EXT_URI,
                                )
                            {
                                known = true;
                            }
                        }
                        break;
                    }
                    Event::Eof => break,
                    _ => {}
                }
            }
            if known && plain_empty(&extension.text, &[b"uri"])? {
                children.push(Splice {
                    start: a,
                    end: b,
                    text: String::new(),
                });
            }
        }
        if children.is_empty() {
            continue;
        }
        let moved = apply(&list.text, children);
        let moved = if plain_empty(&moved, &[])? {
            String::new()
        } else {
            list.finish(&moved)?
        };
        if moved != xml[start..end] {
            edits.push(Splice {
                start,
                end,
                text: moved,
            });
        }
    }
    xml = apply(&xml, edits);
    Ok(xml)
}
pub(crate) fn map_sheet(xml: &str, map: &impl Fn(CellRange) -> Vec<CellRange>) -> Result<String> {
    let replacements = structure_containers(xml, true, map)?;
    let source = sources(xml)?;
    let mut edits = Vec::new();
    for container in source.containers {
        if let Some(replacement) = replacements.get(&container.span.start) {
            edits.push(Splice {
                start: container.span.start,
                end: container.span.end,
                text: replacement.clone(),
            });
        }
    }
    cleanup(&apply(xml, edits))
}
