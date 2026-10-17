use super::{apply_patches, child, edit_tag, parse_document_nodes, xml_error, Node};
use crate::{Result, WriteError};
use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) enum SlideEdit {
    Duplicate(Vec<usize>),
    Delete(Vec<usize>),
    Insert(usize),
    Move { indices: Vec<usize>, to: usize },
    Hidden { indices: Vec<usize>, hidden: bool },
}
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SLIDE_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
pub(super) fn p_ns(strict: bool) -> &'static str {
    if strict {
        "http://purl.oclc.org/ooxml/presentationml/main"
    } else {
        P
    }
}
pub(super) fn r_ns(strict: bool) -> &'static str {
    if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        R
    }
}
fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}
pub(super) fn relationship_attrs<'a>(nodes: &'a [Node], node: &'a Node) -> Vec<(&'a str, &'a str)> {
    node.attrs
        .iter()
        .filter_map(|(key, value)| {
            let (prefix, local) = key.split_once(':')?;
            if !matches!(local, "id" | "embed" | "link") {
                return None;
            }
            let binding = format!("xmlns:{prefix}");
            let mut at = Some(node);
            while let Some(n) = at {
                if let Some(uri) = n.attr(&binding) {
                    return (uri == R || uri == r_ns(true))
                        .then_some((key.as_str(), value.as_str()));
                }
                at = n.parent.map(|i| &nodes[i]);
            }
            None
        })
        .collect()
}
pub(super) fn rel_attr<'a>(nodes: &'a [Node], node: &'a Node) -> Option<(&'a str, &'a str)> {
    relationship_attrs(nodes, node)
        .into_iter()
        .find(|(key, _)| key.ends_with(":id"))
}
fn patch_node_attr(xml: &str, node: &Node, key: &str, value: &str) -> Result<String> {
    let opening = edit_tag(&xml[node.opening.clone()], &[(key, value.to_owned())])?;
    Ok(apply_patches(
        &xml[node.outer.clone()],
        vec![(0..node.opening.len(), opening)],
    ))
}
pub(super) fn append_child(xml: &str, parent: &Node, fragment: &str) -> String {
    if parent.empty {
        apply_patches(
            xml,
            vec![(
                parent.outer.clone(),
                format!(
                    "{}{fragment}</{}>",
                    parent.empty_open.as_deref().unwrap_or_default(),
                    parent.qualified
                ),
            )],
        )
    } else {
        apply_patches(
            xml,
            vec![(parent.range.end..parent.range.end, fragment.into())],
        )
    }
}
pub(super) fn fresh_part(package: &OpcPackage, original: &str) -> String {
    let (stem, extension) = original.rsplit_once('.').unwrap_or((original, "xml"));
    for id in 1.. {
        let name = format!("{stem}-copy{id}.{extension}");
        if package.part(&name).is_none() && package.part(&rels_part_name(&name)).is_none() {
            return name;
        }
    }
    unreachable!()
}
fn shared(kind: &str) -> bool {
    matches!(
        kind.rsplit('/').next().unwrap_or_default(),
        "slide"
            | "slideLayout"
            | "slideMaster"
            | "notesMaster"
            | "handoutMaster"
            | "theme"
            | "image"
            | "audio"
            | "video"
            | "media"
            | "font"
            | "commentAuthors"
            | "person"
    )
}
fn clone_part(
    package: &mut OpcPackage,
    old: &str,
    mapping: &mut BTreeMap<String, String>,
) -> Result<String> {
    if let Some(new) = mapping.get(old) {
        return Ok(new.clone());
    }
    let new = fresh_part(package, old);
    mapping.insert(old.into(), new.clone());
    package.replace(
        &new,
        package
            .part(old)
            .ok_or_else(|| xml_error(format!("missing slide dependency {old}")))?
            .to_vec(),
    );
    if let Some(xml) = package.part_str(&rels_part_name(old))? {
        let nodes = parse_document_nodes(&xml)?;
        let mut patches = Vec::new();
        for relation in nodes.iter().filter(|n| n.is(4, "Relationship")) {
            if relation.attr("TargetMode") == Some("External") {
                continue;
            }
            let target = relation
                .attr("Target")
                .ok_or_else(|| xml_error("relationship without target"))?;
            let resolved = resolve_target(old, target);
            let copy = if let Some(copy) = mapping.get(&resolved) {
                Some(copy.clone())
            } else if !shared(relation.attr("Type").unwrap_or_default()) {
                Some(clone_part(package, &resolved, mapping)?)
            } else {
                None
            };
            if let Some(copy) = copy {
                if relation
                    .attr("Type")
                    .is_some_and(|v| v.ends_with("/notesSlide"))
                {
                    ensure_note_backlink(
                        package,
                        &copy,
                        &new,
                        relation
                            .attr("Type")
                            .unwrap_or_default()
                            .starts_with("http://purl.oclc.org/"),
                    )?;
                }
                patches.push((
                    relation.opening.clone(),
                    edit_tag(
                        &xml[relation.opening.clone()],
                        &[("Target", format!("/{copy}"))],
                    )?,
                ));
            }
        }
        package.replace(
            &rels_part_name(&new),
            apply_patches(&xml, patches).into_bytes(),
        );
    }
    Ok(new)
}

pub(super) fn ensure_note_backlink(
    package: &mut OpcPackage,
    note: &str,
    slide: &str,
    strict: bool,
) -> Result<()> {
    let name = rels_part_name(note);
    let xml = package.part_str(&name)?.unwrap_or_else(|| "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let rels = parse_relationships(&xml)?;
    if rels.all().iter().any(|r| {
        r.relationship_type.ends_with("/slide")
            && !r.external
            && resolve_target(note, &r.target) == slide
    }) {
        return Ok(());
    }
    let id = (1..)
        .map(|i| format!("rId{i}"))
        .find(|id| rels.relationship(id).is_none())
        .expect("notes relationship id");
    let fragment = format!("<jadePkg:Relationship xmlns:jadePkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{id}\" Type=\"{}/slide\" Target=\"/{}\"/>",r_ns(strict),escape(slide));
    let nodes = parse_document_nodes(&xml)?;
    package.replace(&name, append_child(&xml, &nodes[0], &fragment).into_bytes());
    Ok(())
}
pub(super) fn content_types(
    package: &mut OpcPackage,
    copies: &BTreeMap<String, String>,
    removed: &BTreeSet<String>,
    inserted: Option<(&str, &str)>,
) -> Result<()> {
    let xml = package.required_part_str("[Content_Types].xml")?;
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(5, "Types"))
        .ok_or_else(|| xml_error("missing content types root"))?;
    let mut patches = Vec::new();
    let mut additions = String::new();
    for node in nodes.iter().filter(|n| n.is(5, "Override")) {
        let part = node
            .attr("PartName")
            .unwrap_or_default()
            .trim_start_matches('/');
        if removed.contains(part) {
            patches.push((node.outer.clone(), String::new()));
        }
        if let Some(copy) = copies.get(part) {
            additions.push_str(&patch_node_attr(
                &xml,
                node,
                "PartName",
                &format!("/{copy}"),
            )?);
        }
    }
    if let Some((part, mime)) = inserted {
        let tag = root
            .qualified
            .rsplit_once(':')
            .map(|(prefix, _)| format!("{prefix}:Override"))
            .unwrap_or_else(|| "Override".into());
        additions.push_str(&format!(
            "<{tag} PartName=\"/{}\" ContentType=\"{mime}\"/>",
            escape(part)
        ));
    }
    let updated = apply_patches(&xml, patches);
    let updated_nodes = parse_document_nodes(&updated)?;
    let result = if additions.is_empty() {
        updated
    } else {
        append_child(&updated, &updated_nodes[0], &additions)
    };
    package.replace("[Content_Types].xml", result.into_bytes());
    Ok(())
}
fn owned_parts(package: &OpcPackage, part: &str, found: &mut BTreeSet<String>) -> Result<()> {
    if !found.insert(part.into()) {
        return Ok(());
    }
    if let Some(xml) = package.part_str(&rels_part_name(part))? {
        for rel in parse_relationships(&xml)?.all() {
            if !rel.external && !shared(&rel.relationship_type) {
                owned_parts(package, &resolve_target(part, &rel.target), found)?;
            }
        }
    }
    Ok(())
}
fn remove_unreferenced(
    package: &mut OpcPackage,
    candidates: BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    // Protect references from all remaining parts, including preexisting orphan parts.
    let mut live: BTreeSet<_> = package
        .part_names()
        .iter()
        .filter(|p| !p.ends_with(".rels") && !candidates.contains(*p))
        .cloned()
        .collect();
    let mut pending: Vec<_> = live.iter().cloned().collect();
    pending.push(String::new());
    while let Some(part) = pending.pop() {
        let rels = if part.is_empty() {
            "_rels/.rels".into()
        } else {
            rels_part_name(&part)
        };
        if let Some(xml) = package.part_str(&rels)? {
            for rel in parse_relationships(&xml)?.all() {
                if !rel.external {
                    let target = resolve_target(&part, &rel.target);
                    if live.insert(target.clone()) {
                        pending.push(target);
                    }
                }
            }
        }
    }
    let mut removed = BTreeSet::new();
    for part in candidates.difference(&live) {
        package.remove(part);
        package.remove(&rels_part_name(part));
        removed.insert(part.clone());
    }
    Ok(removed)
}
fn remove_links(
    package: &mut OpcPackage,
    deleted: &BTreeSet<String>,
    presentation: &str,
) -> Result<()> {
    let parts: Vec<_> = package
        .part_names()
        .iter()
        .filter(|p| !p.ends_with(".rels") && !deleted.contains(*p) && p.as_str() != presentation)
        .cloned()
        .collect();
    for part in parts {
        let Some(xml) = package.part_str(&rels_part_name(&part))? else {
            continue;
        };
        let nodes = parse_document_nodes(&xml)?;
        let removed: Vec<_> = nodes
            .iter()
            .filter(|n| {
                n.is(4, "Relationship")
                    && n.attr("TargetMode") != Some("External")
                    && n.attr("Type").is_some_and(|v| v.ends_with("/slide"))
                    && n.attr("Target")
                        .is_some_and(|t| deleted.contains(&resolve_target(&part, t)))
            })
            .collect();
        if removed.is_empty() {
            continue;
        }
        let source = package.required_part_str(&part)?;
        let source_nodes = parse_document_nodes(&source)?;
        let mut source_patches = Vec::new();
        for node in &source_nodes {
            if rel_attr(&source_nodes, node)
                .is_some_and(|(_, id)| removed.iter().any(|r| r.attr("Id") == Some(id)))
            {
                if node.is(2, "hlinkClick") || node.is(2, "hlinkMouseOver") {
                    source_patches.push((node.outer.clone(), String::new()));
                } else {
                    return Err(xml_error("unsupported inbound slide reference"));
                }
            }
        }
        package.replace(&part, apply_patches(&source, source_patches).into_bytes());
        package.replace(
            &rels_part_name(&part),
            apply_patches(
                &xml,
                removed
                    .iter()
                    .map(|n| (n.outer.clone(), String::new()))
                    .collect(),
            )
            .into_bytes(),
        );
    }
    Ok(())
}
fn update_counts(package: &mut OpcPackage, count: usize, hidden: usize) -> Result<()> {
    let Some(xml) = package.part_str("docProps/app.xml")? else {
        return Ok(());
    };
    let nodes = parse_document_nodes(&xml)?;
    let patches = nodes
        .iter()
        .filter_map(|n| {
            if n.ns != 7 || n.parent != Some(0) {
                return None;
            }
            let value = match n.name.as_str() {
                "Slides" => count,
                "HiddenSlides" => hidden,
                _ => return None,
            };
            (!n.empty).then(|| (n.range.clone(), value.to_string()))
        })
        .collect();
    package.replace(
        "docProps/app.xml",
        apply_patches(&xml, patches).into_bytes(),
    );
    Ok(())
}

pub(crate) fn edit_slides(bytes: &[u8], edit: SlideEdit) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = package
        .part_str("_rels/.rels")?
        .and_then(|x| parse_relationships(&x).ok())
        .and_then(|r| {
            r.target_by_type_suffix("/officeDocument")
                .map(|t| resolve_target("", t))
        })
        .unwrap_or_else(|| "ppt/presentation.xml".into());
    let xml = package.required_part_str(&presentation)?;
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(1, "presentation"))
        .ok_or_else(|| xml_error("expected a presentation"))?;
    let list = child(&nodes, root, 1, "sldIdLst");
    let entries: Vec<_> = list
        .map(|l| {
            l.children
                .iter()
                .map(|&i| &nodes[i])
                .filter(|n| n.is(1, "sldId"))
                .collect()
        })
        .unwrap_or_default();
    let mut order: Vec<_> = entries
        .iter()
        .map(|n| xml[n.outer.clone()].to_string())
        .collect();
    let rel_part = rels_part_name(&presentation);
    let rel_xml = package.required_part_str(&rel_part)?;
    let rel_nodes = parse_document_nodes(&rel_xml)?;
    let relationships = parse_relationships(&rel_xml)?;
    let part_at = |index: usize| -> Result<String> {
        let entry = entries.get(index).ok_or(WriteError::NoSlide(index))?;
        let (_, rid) =
            rel_attr(&nodes, entry).ok_or_else(|| xml_error("slide without relationship id"))?;
        let rel = relationships
            .relationship(rid)
            .filter(|r| !r.external && r.relationship_type.ends_with("/slide"))
            .ok_or_else(|| xml_error("slide without internal relationship"))?;
        Ok(resolve_target(&presentation, &rel.target))
    };
    let mut presentation_patches = Vec::new();
    let mut copies = BTreeMap::new();
    let mut removed = BTreeSet::new();
    let mut inserted = None;
    let mut new_members = Vec::new();
    let mut moved_members = Vec::new();
    let mut deleted_ids = BTreeSet::new();
    match edit {
        SlideEdit::Hidden {
            ref indices,
            hidden,
        } => {
            let distinct: BTreeSet<_> = indices.iter().copied().collect();
            if indices.is_empty() || distinct.len() != indices.len() {
                return Err(WriteError::InvalidSlideOrder);
            }
            let parts = indices
                .iter()
                .map(|&i| part_at(i))
                .collect::<Result<Vec<_>>>()?;
            let mut planned = Vec::new();
            for part in parts {
                let body = package.required_part_str(&part)?;
                let body_nodes = parse_document_nodes(&body)?;
                let root = body_nodes
                    .first()
                    .filter(|n| n.is(1, "sld"))
                    .ok_or_else(|| xml_error("expected a slide"))?;
                let before = matches!(root.attr("show"), Some("0" | "false"));
                if before != hidden {
                    let updated =
                        patch_node_attr(&body, root, "show", if hidden { "0" } else { "1" })?;
                    planned.push((part, updated));
                }
            }
            if planned.is_empty() {
                return Ok(bytes.to_vec());
            }
            for (part, body) in planned {
                package.replace(&part, body.into_bytes());
            }
        }
        SlideEdit::Move { ref indices, to } => {
            let distinct: BTreeSet<_> = indices.iter().copied().collect();
            if indices.is_empty()
                || distinct.len() != indices.len()
                || distinct.last().is_some_and(|&i| i >= entries.len())
                || to > entries.len() - distinct.len()
            {
                return Err(WriteError::InvalidSlideOrder);
            }
            for &index in indices {
                part_at(index)?;
            }
            // Keep presentation order even when the caller supplies reversed indexes.
            let moved: Vec<_> = distinct.iter().map(|&i| order[i].clone()).collect();
            let mut remaining: Vec<_> = order
                .iter()
                .enumerate()
                .filter_map(|(i, item)| (!distinct.contains(&i)).then_some(item.clone()))
                .collect();
            remaining.splice(to..to, moved.iter().cloned());
            if remaining == order {
                return Ok(bytes.to_vec());
            }
            order = remaining;
            let neighbor = if to + moved.len() < order.len() {
                &order[to + moved.len()]
            } else {
                &order[to - 1]
            };
            let neighbor_id = entry_id(neighbor)?;
            for &index in &distinct {
                let id = entries[index]
                    .attr("id")
                    .ok_or_else(|| xml_error("slide without id"))?;
                moved_members.push((id.to_owned(), neighbor_id.clone()));
            }
        }
        SlideEdit::Delete(ref indices) => {
            let distinct: BTreeSet<_> = indices.iter().copied().collect();
            if indices.is_empty() || distinct.len() != indices.len() {
                return Err(WriteError::InvalidSlideOrder);
            }
            // Resolve every target against the original manifest before changing parts.
            let parts = indices
                .iter()
                .map(|&i| part_at(i))
                .collect::<Result<BTreeSet<_>>>()?;
            let mut rel_patches = Vec::new();
            let mut owned = BTreeSet::new();
            for &index in indices {
                let entry = entries[index];
                let (_, rid) = rel_attr(&nodes, entry)
                    .ok_or_else(|| xml_error("missing slide relationship"))?;
                if let Some(id) = entry.attr("id") {
                    deleted_ids.insert(id.to_owned());
                }
                for n in nodes.iter().filter(|n| {
                    n.is(1, "sld") && rel_attr(&nodes, n).is_some_and(|(_, id)| id == rid)
                }) {
                    presentation_patches.push((n.outer.clone(), String::new()));
                }
                let rel = rel_nodes
                    .iter()
                    .find(|n| n.is(4, "Relationship") && n.attr("Id") == Some(rid))
                    .ok_or_else(|| xml_error("missing slide relationship"))?;
                rel_patches.push((rel.outer.clone(), String::new()));
            }
            for part in &parts {
                owned_parts(&package, part, &mut owned)?;
            }
            order = order
                .into_iter()
                .enumerate()
                .filter_map(|(i, item)| (!distinct.contains(&i)).then_some(item))
                .collect();
            package.replace(&rel_part, apply_patches(&rel_xml, rel_patches).into_bytes());
            remove_links(&mut package, &parts, &presentation)?;
            removed = remove_unreferenced(&mut package, owned)?;
        }
        SlideEdit::Duplicate(ref indices) => {
            let distinct: BTreeSet<_> = indices.iter().copied().collect();
            if indices.is_empty()
                || distinct.len() != indices.len()
                || distinct.last().is_some_and(|&i| i >= entries.len())
            {
                return Err(WriteError::InvalidSlideOrder);
            }
            let sources = distinct
                .iter()
                .map(|&i| part_at(i))
                .collect::<Result<Vec<_>>>()?;
            let mut used: BTreeSet<_> = entries
                .iter()
                .filter_map(|n| n.attr("id").and_then(|id| id.parse::<u32>().ok()))
                .collect();
            let mut used_rels: BTreeSet<String> = rel_nodes
                .iter()
                .filter_map(|n| n.attr("Id").map(str::to_owned))
                .collect();
            let last = *distinct.last().expect("nonempty selection");
            let neighbor = entries[last]
                .attr("id")
                .ok_or_else(|| xml_error("slide without id"))?
                .to_owned();
            let mut fragments = Vec::new();
            let mut relations = String::new();
            for source in sources {
                let id = (256..=i32::MAX as u32)
                    .find(|id| used.insert(*id))
                    .ok_or(WriteError::InvalidSlideOrder)?;
                let rid = (1..)
                    .map(|i| format!("rId{i}"))
                    .find(|id| used_rels.insert(id.clone()))
                    .expect("relationship id");
                // Each copied slide owns an independent graph, even if sources share owned parts.
                let mut mapping = BTreeMap::new();
                let part = clone_part(&mut package, &source, &mut mapping)?;
                content_types(&mut package, &mapping, &BTreeSet::new(), None)?;
                relations.push_str(&format!("<jadePkg:Relationship xmlns:jadePkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{rid}\" Type=\"{}/slide\" Target=\"/{}\"/>",r_ns(root.strict),escape(&part)));
                fragments.push(format!("<jadeSlides:sldId xmlns:jadeSlides=\"{}\" xmlns:jadeRel=\"{}\" id=\"{id}\" jadeRel:id=\"{rid}\"/>",p_ns(root.strict),r_ns(root.strict)));
                new_members.push((id.to_string(), neighbor.clone()));
            }
            order.splice(last + 1..last + 1, fragments);
            package.replace(
                &rel_part,
                append_child(&rel_xml, &rel_nodes[0], &relations).into_bytes(),
            );
        }
        SlideEdit::Insert(index) => {
            if index > entries.len() {
                return Err(WriteError::InvalidSlideOrder);
            }
            let mut used: BTreeSet<_> = entries
                .iter()
                .filter_map(|n| n.attr("id").and_then(|s| s.parse::<u32>().ok()))
                .collect();
            let id = (256..=i32::MAX as u32)
                .find(|i| used.insert(*i))
                .ok_or(WriteError::InvalidSlideOrder)?;
            let used_rels: BTreeSet<_> = rel_nodes.iter().filter_map(|n| n.attr("Id")).collect();
            let rid = (1..)
                .map(|i| format!("rId{i}"))
                .find(|i| !used_rels.contains(i.as_str()))
                .expect("relationship id");
            let part = {
                let dir = presentation.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let part = fresh_part(&package, &format!("{dir}/slides/slide.xml"));
                let reference = if entries.is_empty() {
                    None
                } else {
                    Some(part_at(index.min(entries.len() - 1))?)
                };
                insert_body(
                    &mut package,
                    &part,
                    reference.as_deref(),
                    root.strict,
                    &mut copies,
                )?;
                inserted = Some(part.clone());
                part
            };
            let rel = format!("<jadePkg:Relationship xmlns:jadePkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{rid}\" Type=\"{}/slide\" Target=\"/{}\"/>", r_ns(root.strict), escape(&part));
            package.replace(
                &rel_part,
                append_child(&rel_xml, &rel_nodes[0], &rel).into_bytes(),
            );
            let entry = format!("<jadeSlides:sldId xmlns:jadeSlides=\"{}\" xmlns:jadeRel=\"{}\" id=\"{id}\" jadeRel:id=\"{rid}\"/>", p_ns(root.strict), r_ns(root.strict));
            order.insert(index, entry);
            if !entries.is_empty() {
                new_members.push((
                    id.to_string(),
                    entries[index.min(entries.len() - 1)]
                        .attr("id")
                        .unwrap_or_default()
                        .to_string(),
                ));
            }
        }
    }
    if !matches!(edit, SlideEdit::Hidden { .. }) {
        if let Some(list) = list {
            let updated = if list.empty {
                format!(
                    "{}{}</{}>",
                    list.empty_open.as_deref().unwrap_or_default(),
                    order.concat(),
                    list.qualified
                )
            } else {
                apply_patches(
                    &xml[list.outer.clone()],
                    entries
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            (
                                n.outer.start - list.outer.start..n.outer.end - list.outer.start,
                                order.get(i).cloned().unwrap_or_default(),
                            )
                        })
                        .chain((order.len() > entries.len()).then(|| {
                            (
                                list.range.end - list.outer.start
                                    ..list.range.end - list.outer.start,
                                order[entries.len()..].concat(),
                            )
                        }))
                        .collect(),
                )
            };
            presentation_patches.push((list.outer.clone(), updated));
        } else if !order.is_empty() {
            let at = root
                .children
                .iter()
                .map(|&i| &nodes[i])
                .find(|n| {
                    !matches!(
                        n.name.as_str(),
                        "sldMasterIdLst" | "notesMasterIdLst" | "handoutMasterIdLst"
                    )
                })
                .map(|n| n.outer.start)
                .unwrap_or(root.range.end);
            presentation_patches.push((
                at..at,
                format!(
                    "<jadeSlides:sldIdLst xmlns:jadeSlides=\"{}\">{}</jadeSlides:sldIdLst>",
                    p_ns(root.strict),
                    order.concat()
                ),
            ));
        }
        section_patches(
            &xml,
            &nodes,
            &order,
            &new_members,
            &moved_members,
            &deleted_ids,
            &mut presentation_patches,
        )?;
        package.replace(
            &presentation,
            apply_patches(&xml, presentation_patches).into_bytes(),
        );
        content_types(
            &mut package,
            &copies,
            &removed,
            inserted.as_deref().map(|part| (part, SLIDE_TYPE)),
        )?;
    }
    let result = package.write()?;
    let deck = jade_pptx_reader::read_deck(&result)?;
    update_counts(
        &mut package,
        deck.slides.len(),
        deck.slides.iter().filter(|s| s.content.hidden).count(),
    )?;
    Ok(package.write()?)
}
fn entry_id(fragment: &str) -> Result<String> {
    let xml = format!("<p:presentation xmlns:p=\"{P}\">{fragment}</p:presentation>");
    let nodes = parse_document_nodes(&xml)?;
    Ok(nodes
        .iter()
        .find(|n| n.name == "sldId")
        .and_then(|n| n.attr("id"))
        .unwrap_or_default()
        .to_string())
}
fn section_patches(
    xml: &str,
    nodes: &[Node],
    order: &[String],
    new_members: &[(String, String)],
    moved: &[(String, String)],
    deleted: &BTreeSet<String>,
    patches: &mut Vec<(std::ops::Range<usize>, String)>,
) -> Result<()> {
    let lists: Vec<_> = nodes.iter().filter(|n| n.is(6, "sldIdLst")).collect();
    let mut membership = BTreeMap::new();
    let mut fragments = BTreeMap::new();
    for (i, list) in lists.iter().enumerate() {
        for &c in &list.children {
            let node = &nodes[c];
            if node.is(6, "sldId") {
                let id = node.attr("id").unwrap_or_default().to_string();
                membership.insert(id.clone(), i);
                fragments.insert(
                    id,
                    super::text::scoped(xml, nodes, node, xml[node.outer.clone()].to_string()),
                );
            }
        }
    }
    for (id, neighbor) in new_members.iter().chain(moved.iter()) {
        if let Some(&section) = membership.get(neighbor) {
            membership.insert(id.clone(), section);
        }
    }
    for deleted in deleted {
        membership.remove(deleted);
    }
    let ids = order
        .iter()
        .map(|e| entry_id(e))
        .collect::<Result<Vec<_>>>()?;
    for (i, list) in lists.iter().enumerate() {
        let mut content = String::new();
        for id in &ids {
            if membership.get(id) == Some(&i) {
                if let Some(raw) = fragments.get(id) {
                    content.push_str(raw);
                } else {
                    let prefix = list
                        .qualified
                        .rsplit_once(':')
                        .map(|(p, _)| format!("{p}:"))
                        .unwrap_or_default();
                    content.push_str(&format!("<{prefix}sldId id=\"{id}\"/>"));
                }
            }
        }
        let old_ids: Vec<_> = list
            .children
            .iter()
            .map(|&c| &nodes[c])
            .filter(|n| n.is(6, "sldId"))
            .collect();
        let mut edits: Vec<_> = old_ids
            .iter()
            .map(|n| {
                (
                    n.outer.start - list.outer.start..n.outer.end - list.outer.start,
                    String::new(),
                )
            })
            .collect();
        let at = old_ids
            .first()
            .map(|n| n.outer.start)
            .unwrap_or(list.range.end)
            - list.outer.start;
        edits.push((at..at, content));
        let updated = if list.empty {
            format!(
                "{}{}</{}>",
                list.empty_open.as_deref().unwrap_or_default(),
                edits.last().map(|(_, s)| s.as_str()).unwrap_or_default(),
                list.qualified
            )
        } else {
            apply_patches(&xml[list.outer.clone()], edits)
        };
        patches.push((list.outer.clone(), updated));
    }
    Ok(())
}
fn insert_body(
    package: &mut OpcPackage,
    part: &str,
    reference: Option<&str>,
    strict: bool,
    copies: &mut BTreeMap<String, String>,
) -> Result<()> {
    let layout = reference.and_then(|p| {
        package
            .part_str(&rels_part_name(p))
            .ok()
            .flatten()
            .and_then(|s| parse_relationships(&s).ok())
            .and_then(|r| {
                r.target_by_type_suffix("/slideLayout")
                    .map(|t| resolve_target(p, t))
            })
    });
    let mut shapes = String::new();
    if let Some(layout) = layout {
        let xml = package.required_part_str(&layout)?;
        let nodes = parse_document_nodes(&xml)?;
        for node in nodes.iter().filter(|n| n.is(1, "sp")) {
            let placeholder = child(&nodes, node, 1, "nvSpPr")
                .and_then(|nv| child(&nodes, nv, 1, "nvPr"))
                .and_then(|nv| child(&nodes, nv, 1, "ph"));
            if placeholder
                .is_none_or(|ph| matches!(ph.attr("type"), Some("dt" | "ftr" | "hdr" | "sldNum")))
            {
                continue;
            }
            let Some(body) = child(&nodes, node, 1, "txBody") else {
                continue;
            };
            // Scope insertion changes opening length; edit the body on the original fragment first.
            let defaults: String = body
                .children
                .iter()
                .map(|&i| &nodes[i])
                .filter(|n| n.is(2, "bodyPr") || n.is(2, "lstStyle"))
                .map(|n| super::text::scoped(&xml, &nodes, n, xml[n.outer.clone()].to_string()))
                .collect();
            let replacement = format!("<jadeSlides:txBody xmlns:jadeSlides=\"{}\" xmlns:jadeDraw=\"{}\">{defaults}<jadeDraw:p/></jadeSlides:txBody>", p_ns(strict), super::drawing_ns(strict));
            let original = apply_patches(
                &xml[node.outer.clone()],
                vec![(
                    body.outer.start - node.outer.start..body.outer.end - node.outer.start,
                    replacement,
                )],
            );
            let raw = super::text::scoped(&xml, &nodes, node, original);
            shapes.push_str(&raw);
        }
        let body = format!("<p:sld xmlns:p=\"{}\">{shapes}</p:sld>", p_ns(strict));
        let shape_nodes = parse_document_nodes(&body)?;
        let needed: BTreeSet<_> = shape_nodes
            .iter()
            .flat_map(|n| relationship_attrs(&shape_nodes, n))
            .map(|(_, v)| v.to_string())
            .collect();
        let mut references = String::new();
        if let Some(rel_xml) = package.part_str(&rels_part_name(&layout))? {
            let rel_nodes = parse_document_nodes(&rel_xml)?;
            for rel in rel_nodes.iter().filter(|n| {
                n.is(4, "Relationship") && n.attr("Id").is_some_and(|id| needed.contains(id))
            }) {
                let mut raw = rel_xml[rel.outer.clone()].to_string();
                if rel.attr("TargetMode") != Some("External") {
                    let target = resolve_target(
                        &layout,
                        rel.attr("Target")
                            .ok_or_else(|| xml_error("layout dependency without target"))?,
                    );
                    let target = if shared(rel.attr("Type").unwrap_or_default()) {
                        target
                    } else {
                        clone_part(package, &target, copies)?
                    };
                    raw = patch_node_attr(&rel_xml, rel, "Target", &format!("/{target}"))?;
                }
                references.push_str(&super::text::scoped(&rel_xml, &rel_nodes, rel, raw));
            }
        }
        let id = (1..)
            .map(|i| format!("rId{i}"))
            .find(|id| !needed.contains(id))
            .expect("layout relationship id");
        let rels = format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{references}<Relationship Id=\"{id}\" Type=\"{}/slideLayout\" Target=\"/{}\"/></Relationships>",r_ns(strict),escape(&layout));
        package.replace(&rels_part_name(part), rels.into_bytes());
    } else {
        let size = jade_pptx_reader::read_pptx(&package.write()?)?.slide_size;
        for (id, name, kind, y, height) in [
            (2, "Title 1", "title", 0.08, 0.18),
            (3, "Content 2", "body", 0.32, 0.55),
        ] {
            shapes.push_str(&format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr><p:ph type=\"{kind}\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp>",(size.width_emu as f64*0.1).round() as i64,(size.height_emu as f64*y).round() as i64,(size.width_emu as f64*0.8).round() as i64,(size.height_emu as f64*height).round() as i64));
        }
    }
    package.replace(part, format!("<p:sld xmlns:p=\"{}\" xmlns:a=\"{}\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",p_ns(strict),super::drawing_ns(strict)).into_bytes());
    Ok(())
}
