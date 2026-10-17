use super::slides::{append_child, content_types, ensure_note_backlink, fresh_part, p_ns, r_ns};
use super::{child, parse_document_nodes, shape_nodes, text, xml_error, Node};
use crate::{Result, TextSnapshot, WriteError};
use jade_opc::{
    parse_relationships, rels_part_name, resolve_target, OpcPackage, PreservingOpcPackage,
};
use std::collections::{BTreeMap, BTreeSet};
const MIME: &str = "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
fn locate(
    bytes: &[u8],
    index: usize,
) -> Result<(PreservingOpcPackage<'_>, String, Option<String>)> {
    let slides = jade_pptx_reader::read_pptx(bytes)?;
    let slide = slides
        .slides
        .get(index)
        .ok_or(WriteError::NoSlide(index))?
        .part_name
        .clone();
    let package = OpcPackage::open_preserving(bytes)?;
    let part = match package.part_str(&rels_part_name(&slide))? {
        Some(xml) => {
            let rels = parse_relationships(&xml)?;
            let targets = rels.by_type_suffix("/notesSlide");
            if targets.len() > 1 || targets.iter().any(|r| r.external) {
                return Err(xml_error("ambiguous notes relationship"));
            }
            targets.first().map(|r| resolve_target(&slide, &r.target))
        }
        None => None,
    };
    Ok((package, slide, part))
}
fn body_id(nodes: &[Node]) -> Result<Option<u32>> {
    let ids: Vec<_> = shape_nodes(nodes)
        .into_iter()
        .filter_map(|shape| {
            let nv = child(nodes, shape, 1, "nvSpPr")?;
            let ph = child(nodes, nv, 1, "nvPr").and_then(|n| child(nodes, n, 1, "ph"))?;
            if ph.attr("type") != Some("body") {
                return None;
            }
            Some(
                child(nodes, nv, 1, "cNvPr")
                    .and_then(|n| n.attr("id"))
                    .and_then(|s| s.parse::<u32>().ok()),
            )
        })
        .collect();
    if ids.len() > 1 || ids.iter().any(Option::is_none) {
        return Err(xml_error(
            "notes must have a uniquely identified body placeholder",
        ));
    }
    Ok(ids.first().copied().flatten())
}
fn shape_for_id(nodes: &[Node], id: u32) -> Option<&Node> {
    shape_nodes(nodes).into_iter().find(|shape| {
        child(nodes, shape, 1, "nvSpPr")
            .and_then(|n| child(nodes, n, 1, "cNvPr"))
            .and_then(|n| n.attr("id"))
            .and_then(|s| s.parse::<u32>().ok())
            == Some(id)
    })
}
pub(crate) fn notes_snapshot(bytes: &[u8], index: usize) -> Result<TextSnapshot> {
    let (package, _, part) = locate(bytes, index)?;
    let Some(part) = part else {
        return Ok(TextSnapshot {
            text: String::new(),
            can_edit: true,
            locked: false,
        });
    };
    let xml = package.required_part_str(&part)?;
    let nodes = parse_document_nodes(&xml)?;
    if !nodes.first().is_some_and(|n| n.is(1, "notes")) {
        return Err(xml_error("expected notes root"));
    }
    match body_id(&nodes)? {
        Some(id) => match text::text_snapshot(&xml, id) {
            Err(WriteError::NoTextBody(_)) => {
                let shape = shape_for_id(&nodes, id).ok_or(WriteError::NoShape(id))?;
                let locked = text::shape_locked(&nodes, shape);
                Ok(TextSnapshot {
                    text: String::new(),
                    can_edit: !locked,
                    locked,
                })
            }
            result => result,
        },
        None => Ok(TextSnapshot {
            text: String::new(),
            can_edit: true,
            locked: false,
        }),
    }
}
fn body(id: u32, strict: bool) -> String {
    format!("<n:sp xmlns:n=\"{}\" xmlns:d=\"{}\"><n:nvSpPr><n:cNvPr id=\"{id}\" name=\"Notes body\"/><n:cNvSpPr/><n:nvPr><n:ph type=\"body\" idx=\"1\"/></n:nvPr></n:nvSpPr><n:spPr/><n:txBody><d:bodyPr/><d:lstStyle/><d:p/></n:txBody></n:sp>",p_ns(strict),super::drawing_ns(strict))
}
fn add_relation(
    package: &mut OpcPackage,
    owner: &str,
    kind: &str,
    target: &str,
    strict: bool,
) -> Result<()> {
    let name = rels_part_name(owner);
    let xml=package.part_str(&name)?.unwrap_or_else(||"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let rels = parse_relationships(&xml)?;
    let id = (1..)
        .map(|i| format!("rId{i}"))
        .find(|id| rels.relationship(id).is_none())
        .expect("notes relationship id");
    let nodes = parse_document_nodes(&xml)?;
    let target = quick_xml::escape::escape(target);
    let rel=format!("<n:Relationship xmlns:n=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{id}\" Type=\"{}/{kind}\" Target=\"/{target}\"/>",r_ns(strict));
    package.replace(&name, append_child(&xml, &nodes[0], &rel).into_bytes());
    Ok(())
}
pub(crate) fn set_notes(
    bytes: &[u8],
    index: usize,
    value: &str,
    soft_break: bool,
) -> Result<Vec<u8>> {
    update_notes(bytes, index, value, soft_break, None)
}
pub(crate) fn replace_notes_range(
    bytes: &[u8],
    index: usize,
    start: usize,
    end: usize,
    replacement: &str,
    soft_break: bool,
) -> Result<Vec<u8>> {
    let before = notes_snapshot(bytes, index)?;
    if start > end {
        return Err(WriteError::InvalidTextRange);
    }
    let from = text::byte_offset(&before.text, start)?;
    let to = text::byte_offset(&before.text, end)?;
    let value = format!(
        "{}{}{}",
        &before.text[..from],
        replacement,
        &before.text[to..]
    );
    update_notes(
        bytes,
        index,
        &value,
        soft_break,
        Some((start, end, replacement)),
    )
}
fn update_notes(
    bytes: &[u8],
    index: usize,
    value: &str,
    soft_break: bool,
    explicit: Option<(usize, usize, &str)>,
) -> Result<Vec<u8>> {
    let before = notes_snapshot(bytes, index)?;
    if before.locked {
        return Err(WriteError::TextLocked(0));
    }
    if !before.can_edit {
        return Err(WriteError::UnsupportedTextContent(0));
    }
    if before.text == value {
        return Ok(bytes.to_vec());
    }
    let (mut package, slide, existing) = locate(bytes, index)?;
    let slide_xml = package.required_part_str(&slide)?;
    let slide_nodes = parse_document_nodes(&slide_xml)?;
    let strict = slide_nodes[0].strict;
    let part = if let Some(part) = existing {
        part
    } else {
        let directory = slide
            .rsplit_once('/')
            .map(|(dir, _)| {
                dir.rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .unwrap_or(dir)
            })
            .unwrap_or("");
        let part = fresh_part(
            &package,
            format!("{directory}/notesSlides/notesSlide.xml").trim_start_matches('/'),
        );
        let xml=format!("<n:notes xmlns:n=\"{}\"><n:cSld><n:spTree><n:nvGrpSpPr><n:cNvPr id=\"1\" name=\"\"/><n:cNvGrpSpPr/><n:nvPr/></n:nvGrpSpPr><n:grpSpPr/>{}</n:spTree></n:cSld></n:notes>",p_ns(strict),body(2,strict));
        package.replace(&part, xml.into_bytes());
        add_relation(&mut package, &slide, "notesSlide", &part, strict)?;
        content_types(
            &mut package,
            &BTreeMap::new(),
            &BTreeSet::new(),
            Some((&part, MIME)),
        )?;
        // Reuse an available notes master without regenerating its appearance.
        let masters: Vec<_> = package
            .part_names()
            .iter()
            .filter(|p| p.ends_with(".rels"))
            .cloned()
            .collect();
        for name in masters {
            let Some(xml) = package.part_str(&name)? else {
                continue;
            };
            let rels = parse_relationships(&xml)?;
            if let Some(rel) = rels
                .by_type_suffix("/notesMaster")
                .into_iter()
                .find(|r| !r.external)
            {
                let owner = if name == "_rels/.rels" {
                    String::new()
                } else {
                    name.replace("/_rels/", "/")
                        .trim_end_matches(".rels")
                        .to_string()
                };
                let master = resolve_target(&owner, &rel.target);
                if package.part(&master).is_some() {
                    add_relation(&mut package, &part, "notesMaster", &master, strict)?;
                    break;
                }
            }
        }
        part
    };
    let mut xml = package.required_part_str(&part)?;
    let nodes = parse_document_nodes(&xml)?;
    let id = if let Some(id) = body_id(&nodes)? {
        let shape = shape_for_id(&nodes, id).ok_or(WriteError::NoShape(id))?;
        if child(&nodes, shape, 1, "txBody").is_none() {
            let at = child(&nodes, shape, 1, "extLst")
                .map(|n| n.outer.start)
                .unwrap_or(shape.range.end);
            let fragment = format!(
                "<n:txBody xmlns:n=\"{}\" xmlns:d=\"{}\"><d:bodyPr/><d:lstStyle/><d:p/></n:txBody>",
                p_ns(nodes[0].strict),
                super::drawing_ns(nodes[0].strict)
            );
            xml = super::apply_patches(&xml, vec![(at..at, fragment)]);
        }
        id
    } else {
        let id = nodes
            .iter()
            .filter(|n| n.is(1, "cNvPr"))
            .filter_map(|n| n.attr("id").and_then(|s| s.parse::<u32>().ok()))
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| xml_error("notes shape ids exhausted"))?;
        let tree = child(&nodes, &nodes[0], 1, "cSld")
            .and_then(|n| child(&nodes, n, 1, "spTree"))
            .ok_or_else(|| xml_error("notes have no shape tree"))?;
        xml = append_child(&xml, tree, &body(id, nodes[0].strict));
        id
    };
    let edit = if let Some((start, end, text)) = explicit {
        crate::TextRangeEdit {
            shape_id: id,
            start,
            end,
            text: text.into(),
            soft_break,
        }
    } else {
        crate::text_edit_between(id, &before.text, value, soft_break)
    };
    xml = text::replace_text_range(&xml, id, edit.start, edit.end, &edit.text, soft_break)?;
    package.replace(&part, xml.into_bytes());
    ensure_note_backlink(&mut package, &part, &slide, strict)?;
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}
