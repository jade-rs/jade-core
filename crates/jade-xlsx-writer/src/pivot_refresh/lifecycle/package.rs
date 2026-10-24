//! Package ownership for pivot creation/removal. Never match a foreign child
//! just because its local name or relationship URI suffix looks familiar.
use super::super::{bad, xml, Result};
use crate::{
    patch::{apply, Splice},
    xml_edit, xml_edit_ns,
};
use jade_opc::{OpcPackage, Relationship};
use std::collections::BTreeSet;
pub(super) const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub(super) const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub(super) fn root(xml: &str, name: &str, namespace: &str) -> Result<()> {
    let mut r = quick_xml::NsReader::from_str(xml);
    loop {
        match r.read_event()? {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                if e.local_name().as_ref() != name.as_bytes()
                    || !matches!(r.resolver().resolve_element(e.name()).0,quick_xml::name::ResolveResult::Bound(n) if n.as_ref()==namespace.as_bytes())
                {
                    return Err(bad("unrecognized pivot package metadata root"));
                }
                return Ok(());
            }
            quick_xml::events::Event::Eof => return Err(bad("empty pivot package metadata")),
            _ => {}
        }
    }
}
pub(super) fn rels(
    package: &OpcPackage,
    owner: &str,
) -> Result<(String, String, Vec<Relationship>)> {
    let path = jade_opc::rels_part_name(owner);
    let body = package
        .part_str(&path)?
        .unwrap_or_else(|| format!("<Relationships xmlns=\"{PACKAGE}\"/>"));
    root(&body, "Relationships", PACKAGE)?;
    let mut used = BTreeSet::new();
    let mut records = Vec::new();
    for (_, _, node) in xml_edit_ns::children(&body, b"Relationship")? {
        let id = xml::attr(&node.text, "Id")?.ok_or_else(|| bad("missing relationship ID"))?;
        if !used.insert(id.clone()) {
            return Err(bad("duplicate pivot package relationship ID"));
        }
        let mode = xml::attr(&node.text, "TargetMode")?;
        if mode
            .as_deref()
            .is_some_and(|s| s != "Internal" && s != "External")
        {
            return Err(bad("invalid pivot relationship target mode"));
        }
        records.push(Relationship {
            id,
            relationship_type: xml::attr(&node.text, "Type")?
                .ok_or_else(|| bad("missing relationship type"))?,
            target: xml::attr(&node.text, "Target")?
                .ok_or_else(|| bad("missing relationship target"))?,
            external: mode.as_deref() == Some("External"),
        });
    }
    Ok((path, body, records))
}
pub(super) fn add_relationship(
    package: &mut OpcPackage,
    owner: &str,
    kind: &str,
    target: &str,
    strict: bool,
) -> Result<String> {
    let (path, body, records) = rels(package, owner)?;
    let id = (1u32..)
        .map(|n| format!("rIdPivot{n}"))
        .find(|id| !records.iter().any(|r| r.id == *id))
        .ok_or_else(|| bad("relationship ID overflow"))?;
    let namespace = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    let entry = xml_edit::root_attributes(
        &format!("<{}Relationship/>", xml::prefix(&body)?),
        &[
            ("Id", Some(id.clone())),
            ("Type", Some(format!("{namespace}/{kind}"))),
            ("Target", Some(crate::notes::relative_target(owner, target))),
        ],
    )?;
    package.replace(&path, xml_edit::append_child(&body, &entry)?.into_bytes());
    Ok(id)
}
pub(super) fn remove_relationship(package: &mut OpcPackage, owner: &str, id: &str) -> Result<()> {
    let (path, body, records) = rels(package, owner)?;
    if records.iter().filter(|r| r.id == id).count() != 1 {
        return Err(bad("missing relationship to remove"));
    }
    let mut edits = Vec::new();
    for (start, end, node) in xml_edit_ns::children(&body, b"Relationship")? {
        if xml::attr(&node.text, "Id")?.as_deref() == Some(id) {
            edits.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
    }
    package.replace(&path, apply(&body, edits).into_bytes());
    Ok(())
}
pub(super) fn free_part(package: &OpcPackage, prefix: &str) -> Result<String> {
    let types = package.required_part_str("[Content_Types].xml")?;
    root(&types, "Types", TYPES)?;
    let mut used = package
        .part_names()
        .iter()
        .map(|p| p.to_lowercase())
        .collect::<BTreeSet<_>>();
    for (_, _, n) in xml_edit_ns::children(&types, b"Override")? {
        if let Some(p) = xml::attr(&n.text, "PartName")? {
            used.insert(p.trim_start_matches('/').to_lowercase());
        }
    }
    (1u32..)
        .map(|n| format!("{prefix}{n}.xml"))
        .find(|p| {
            !used.contains(&p.to_lowercase())
                && !used.contains(&jade_opc::rels_part_name(p).to_lowercase())
        })
        .ok_or_else(|| bad("pivot part identity overflow"))
}
pub(super) fn add_type(package: &mut OpcPackage, part: &str, kind: &str) -> Result<()> {
    let body = package.required_part_str("[Content_Types].xml")?;
    root(&body, "Types", TYPES)?;
    for (_, _, n) in xml_edit_ns::children(&body, b"Override")? {
        if xml::attr(&n.text, "PartName")?
            .is_some_and(|p| p.trim_start_matches('/').eq_ignore_ascii_case(part))
        {
            return Err(bad("duplicate pivot content type ownership"));
        }
    }
    let entry = xml_edit::root_attributes(
        &format!("<{}Override/>", xml::prefix(&body)?),
        &[
            ("PartName", Some(format!("/{part}"))),
            (
                "ContentType",
                Some(format!(
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.{kind}+xml"
                )),
            ),
        ],
    )?;
    package.replace(
        "[Content_Types].xml",
        xml_edit::append_child(&body, &entry)?.into_bytes(),
    );
    Ok(())
}
pub(super) fn drop_part(package: &mut OpcPackage, part: &str) -> Result<()> {
    let body = package.required_part_str("[Content_Types].xml")?;
    root(&body, "Types", TYPES)?;
    let mut edits = Vec::new();
    for (start, end, node) in xml_edit_ns::children(&body, b"Override")? {
        if xml::attr(&node.text, "PartName")?
            .is_some_and(|p| p.trim_start_matches('/').eq_ignore_ascii_case(part))
        {
            edits.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
    }
    package.remove(part);
    package.remove(&jade_opc::rels_part_name(part));
    package.replace("[Content_Types].xml", apply(&body, edits).into_bytes());
    Ok(())
}
pub(super) fn add_declaration(
    package: &mut OpcPackage,
    workbook: &str,
    id: u32,
    relationship: &str,
    rel_namespace: &str,
) -> Result<()> {
    let body = package.required_part_str(workbook)?;
    let updated = xml_edit_ns::edit_child(
        &body,
        b"pivotCaches",
        &[
            b"smartTagPr",
            b"smartTagTypes",
            b"webPublishing",
            b"fileRecoveryPr",
            b"webPublishObjects",
            b"extLst",
        ],
        |node| {
            let entry = xml_edit::root_attributes(
                &format!(
                    "<{}pivotCache xmlns:jadePivot=\"{rel_namespace}\"/>",
                    xml::prefix(node)?
                ),
                &[
                    ("cacheId", Some(id.to_string())),
                    ("jadePivot:id", Some(relationship.into())),
                ],
            )?;
            xml_edit::append_child(node, &entry)
        },
    )?;
    package.replace(workbook, updated.into_bytes());
    Ok(())
}
pub(super) fn relationship_id(node: &str) -> Result<Option<String>> {
    let mut r = quick_xml::NsReader::from_str(node);
    loop {
        match r.read_event()? {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                let mut result = None;
                for a in e.attributes() {
                    let a = a?;
                    let (namespace, local) = r.resolver().resolve_attribute(a.key);
                    if local.as_ref() == b"id"
                        && matches!(namespace,quick_xml::name::ResolveResult::Bound(n) if n.as_ref()==b"http://schemas.openxmlformats.org/officeDocument/2006/relationships"||n.as_ref()==b"http://purl.oclc.org/ooxml/officeDocument/relationships")
                    {
                        if result.is_some() {
                            return Err(bad("ambiguous pivot relationship attribute"));
                        }
                        result = Some(a.unescape_value()?.into_owned());
                    }
                }
                return Ok(result);
            }
            quick_xml::events::Event::Eof => return Err(bad("empty pivot declaration")),
            _ => {}
        }
    }
}
pub(super) fn owner_of_rels(path: &str) -> Option<String> {
    if path == "_rels/.rels" {
        return Some(String::new());
    }
    if let Some(file) = path.strip_prefix("_rels/") {
        return file.strip_suffix(".rels").map(str::to_string);
    }
    let (dir, file) = path.rsplit_once("/_rels/")?;
    Some(format!("{dir}/{}", file.strip_suffix(".rels")?))
}
/// Unknown or malformed relationship ownership prevents collecting a cache;
/// keeping an unused definition is preferable to deleting a live dependency.
pub(super) fn has_other_incoming(
    package: &OpcPackage,
    targets: &BTreeSet<String>,
    allowed: &BTreeSet<(String, String)>,
) -> Result<bool> {
    // Collection must retain even a potential URI alias used by an opaque
    // dependency. This is conservative ownership, not promotion into a binding.
    fn decoded_uri(target: &str) -> String {
        let bytes = target.split(['#', '?']).next().unwrap_or(target).as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%' && at + 2 < bytes.len() {
                if let (Some(a), Some(b)) = (
                    (bytes[at + 1] as char).to_digit(16),
                    (bytes[at + 2] as char).to_digit(16),
                ) {
                    decoded.push((a * 16 + b) as u8);
                    at += 3;
                    continue;
                }
            }
            decoded.push(bytes[at]);
            at += 1;
        }
        String::from_utf8_lossy(&decoded).into_owned()
    }
    fn potential_target(owner: &str, target: &str) -> String {
        jade_opc::resolve_target(&decoded_uri(owner), &decoded_uri(target)).to_lowercase()
    }
    let targets = targets
        .iter()
        .map(|p| potential_target("", p))
        .collect::<BTreeSet<_>>();
    for path in package.part_names().iter().filter(|p| p.ends_with(".rels")) {
        let Some(owner) = owner_of_rels(path) else {
            return Ok(true);
        };
        let Ok((_, _, records)) = rels(package, &owner) else {
            return Ok(true);
        };
        for link in records {
            if !link.external
                && targets.contains(&potential_target(&owner, &link.target))
                && !allowed.contains(&(owner.clone(), link.id))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
