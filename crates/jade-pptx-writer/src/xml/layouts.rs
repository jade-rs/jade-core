//! Preserving layout assignment and placeholder reconciliation.
use super::{
    apply_patches, child, edit_tag, edit_tag_attributes, objects, parse_document_nodes,
    parse_nodes, text, transform_xml, Node,
};
use crate::{Result, WriteError};
use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use serde::Serialize;
use std::collections::HashSet;
#[path = "reset.rs"]
mod reset;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideLayoutInfo {
    pub part_name: String,
    pub master_part_name: String,
    pub master_name: String,
    pub name: String,
    pub kind: String,
    pub current: bool,
}
fn relation(package: &OpcPackage, owner: &str, kind: &str) -> Result<Option<String>> {
    let Some(xml) = package.part_str(&rels_part_name(owner))? else {
        return Ok(None);
    };
    Ok(parse_relationships(&xml)?
        .by_type_suffix(kind)
        .into_iter()
        .find(|r| !r.external)
        .map(|r| resolve_target(owner, &r.target)))
}
fn catalog(package: &OpcPackage) -> Result<Vec<SlideLayoutInfo>> {
    let presentation =
        relation(package, "", "/officeDocument")?.unwrap_or_else(|| "ppt/presentation.xml".into());
    let xml = package.required_part_str(&presentation)?;
    let nodes = parse_document_nodes(&xml)?;
    let rels = parse_relationships(&package.required_part_str(&rels_part_name(&presentation))?)?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for (master_index, entry) in nodes.iter().filter(|n| n.is(1, "sldMasterId")).enumerate() {
        let Some((_, id)) = super::slides::rel_attr(&nodes, entry) else {
            continue;
        };
        let Some(rel) = rels
            .relationship(id)
            .filter(|r| !r.external && r.relationship_type.ends_with("/slideMaster"))
        else {
            continue;
        };
        let master = resolve_target(&presentation, &rel.target);
        let Some(xml) = package.part_str(&master)? else {
            continue;
        };
        let master_nodes = parse_document_nodes(&xml)?;
        let Some(root) = master_nodes.first().filter(|n| n.is(1, "sldMaster")) else {
            continue;
        };
        let master_name = child(&master_nodes, root, 1, "cSld")
            .and_then(|n| n.attr("name"))
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Master {}", master_index + 1));
        let Some(list) = child(&master_nodes, root, 1, "sldLayoutIdLst") else {
            continue;
        };
        let Some(xml_rels) = package.part_str(&rels_part_name(&master))? else {
            continue;
        };
        let master_rels = parse_relationships(&xml_rels)?;
        for entry in list
            .children
            .iter()
            .map(|i| &master_nodes[*i])
            .filter(|n| n.is(1, "sldLayoutId"))
        {
            let Some((_, id)) = super::slides::rel_attr(&master_nodes, entry) else {
                continue;
            };
            let Some(rel) = master_rels
                .relationship(id)
                .filter(|r| !r.external && r.relationship_type.ends_with("/slideLayout"))
            else {
                continue;
            };
            let part = resolve_target(&master, &rel.target);
            if !seen.insert(part.clone())
                || relation(package, &part, "/slideMaster")?.as_deref() != Some(master.as_str())
            {
                continue;
            }
            let Some(xml) = package.part_str(&part)? else {
                continue;
            };
            let layout_nodes = parse_document_nodes(&xml)?;
            let Some(root) = layout_nodes.first().filter(|n| n.is(1, "sldLayout")) else {
                continue;
            };
            result.push(SlideLayoutInfo {
                part_name: part.clone(),
                master_part_name: master.clone(),
                master_name: master_name.clone(),
                name: child(&layout_nodes, root, 1, "cSld")
                    .and_then(|n| n.attr("name"))
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Layout {}", result.len() + 1)),
                kind: root.attr("type").unwrap_or("cust").into(),
                current: false,
            });
        }
    }
    Ok(result)
}
pub fn slide_layouts(bytes: &[u8], slide: usize) -> Result<Vec<SlideLayoutInfo>> {
    let (package, part, _) = crate::slide_xml(bytes, slide)?;
    let current = relation(&package, &part, "/slideLayout")?;
    let mut result = catalog(&package)?;
    for layout in &mut result {
        layout.current = current.as_deref() == Some(&layout.part_name)
    }
    Ok(result)
}
struct Placeholder<'a> {
    shape: &'a Node,
    ph: &'a Node,
    id: u32,
    index: u32,
    kind: &'a str,
}
fn placeholders(nodes: &[Node], include_footer: bool) -> Result<Vec<Placeholder<'_>>> {
    let tree = child(nodes, &nodes[0], 1, "cSld")
        .and_then(|n| child(nodes, n, 1, "spTree"))
        .ok_or_else(|| super::xml_error("no shape tree"))?;
    let mut result = Vec::new();
    for shape in tree.children.iter().map(|i| &nodes[*i]) {
        let nv = match shape.name.as_str() {
            "sp" => "nvSpPr",
            "pic" => "nvPicPr",
            "graphicFrame" => "nvGraphicFramePr",
            _ => continue,
        };
        let Some(nv) = child(nodes, shape, 1, nv) else {
            continue;
        };
        let Some(ph) = child(nodes, nv, 1, "nvPr").and_then(|n| child(nodes, n, 1, "ph")) else {
            continue;
        };
        let kind = ph.attr("type").unwrap_or("obj");
        if !include_footer && matches!(kind, "dt" | "ftr" | "hdr" | "sldNum") {
            continue;
        }
        if !shape.is(1, "sp") && !shape.is(1, "pic") {
            return Err(super::xml_error("unsupported layout placeholder frame"));
        }
        let id = child(nodes, nv, 1, "cNvPr")
            .and_then(|n| n.attr("id"))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| super::xml_error("placeholder has no shape id"))?;
        let index = ph
            .attr("idx")
            .unwrap_or("0")
            .parse()
            .map_err(super::xml_error)?;
        if result
            .iter()
            .any(|p: &Placeholder| p.id == id || (p.index == index && p.kind == kind))
        {
            return Err(super::xml_error("ambiguous placeholders"));
        }
        result.push(Placeholder {
            shape,
            ph,
            id,
            index,
            kind,
        });
    }
    // A fallback/extension placeholder must not be silently duplicated or retargeted.
    for ph in nodes.iter().filter(|n| n.is(1, "ph")) {
        if (include_footer || !matches!(ph.attr("type"), Some("dt" | "ftr" | "hdr" | "sldNum")))
            && !result.iter().any(|p| std::ptr::eq(p.ph, ph))
        {
            return Err(super::xml_error(
                "nested or alternate placeholder cannot be reconciled",
            ));
        }
    }
    Ok(result)
}
fn title(kind: &str) -> bool {
    matches!(kind, "title" | "ctrTitle")
}
fn match_slot(
    source: &Placeholder,
    target: &[Placeholder],
    used: &HashSet<usize>,
) -> Result<Option<usize>> {
    let mut candidates: Vec<_> = target
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            !used.contains(i) && title(p.kind) == title(source.kind) && p.index == source.index
        })
        .map(|(i, _)| i)
        .collect();
    if candidates.len() > 1 {
        candidates.retain(|i| target[*i].kind == source.kind)
    }
    if candidates.is_empty() {
        candidates = target
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                !used.contains(i) && (p.kind == source.kind || title(p.kind) && title(source.kind))
            })
            .map(|(i, _)| i)
            .collect()
    }
    match candidates.as_slice() {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        _ => Err(super::xml_error("ambiguous placeholder mapping")),
    }
}
fn freeze(
    xml: &str,
    nodes: &[Node],
    shape: &Node,
    transform: jade_pptx_reader::Transform,
) -> Result<Option<(std::ops::Range<usize>, String)>> {
    let pr = child(nodes, shape, 1, "spPr")
        .ok_or_else(|| super::xml_error("placeholder has no shape properties"))?;
    if child(nodes, pr, 2, "xfrm").is_some() {
        return Ok(None);
    }
    let value = transform_xml(xml, nodes, None, transform, shape.strict)?;
    Ok(Some(if pr.empty {
        (
            pr.outer.clone(),
            format!(
                "{}{value}</{}>",
                pr.empty_open.as_deref().unwrap_or_default(),
                pr.qualified
            ),
        )
    } else {
        (pr.range.start..pr.range.start, value)
    }))
}
fn reconcile(
    source: &str,
    target: &str,
    same: bool,
    infos: &[crate::ShapeTransformInfo],
) -> Result<String> {
    let nodes = parse_nodes(source)?;
    let target_nodes = parse_document_nodes(target)?;
    let sources = placeholders(&nodes, false)?;
    let targets = placeholders(&target_nodes, false)?;
    let mut used = HashSet::new();
    let mut patches = Vec::new();
    for ph in sources {
        let slot = match_slot(&ph, &targets, &used)?;
        if let Some(i) = slot {
            used.insert(i);
            if !same {
                let target = targets[i].ph;
                let names = ["type", "idx", "orient", "sz"];
                let values: Vec<_> = names
                    .iter()
                    .filter_map(|name| target.attr(name).map(|value| (*name, value.into())))
                    .collect();
                let absent: Vec<_> = names
                    .iter()
                    .filter(|name| target.attr(name).is_none())
                    .copied()
                    .collect();
                patches.push((
                    ph.ph.opening.clone(),
                    edit_tag_attributes(&source[ph.ph.opening.clone()], &values, &absent)?,
                ));
            }
        }
        if !same {
            let info = infos
                .iter()
                .find(|p| p.id == ph.id)
                .ok_or(WriteError::NoShape(ph.id))?;
            if slot.is_none() || !info.can_move || !info.can_resize || info.lock_aspect_ratio {
                let t = info.transform.ok_or(WriteError::InvalidTransform)?;
                if let Some(patch) = freeze(source, &nodes, ph.shape, t)? {
                    patches.push(patch)
                }
            }
        }
    }
    let mut changed = apply_patches(source, patches);
    for (i, placeholder) in targets
        .iter()
        .enumerate()
        .filter(|(i, _)| !used.contains(i))
    {
        let _ = i;
        let nodes = parse_nodes(&changed)?;
        let (tree, id, name) = objects::insertion_identity(&nodes, "Placeholder")?;
        if objects::descendants(&target_nodes, placeholder.ph)
            .iter()
            .any(|node| !super::slides::relationship_attrs(&target_nodes, node).is_empty())
        {
            return Err(super::xml_error(
                "layout placeholder has part-relative extension references",
            ));
        }
        let ph = text::scoped(
            target,
            &target_nodes,
            placeholder.ph,
            target[placeholder.ph.outer.clone()].to_owned(),
        );
        let p = super::slides::p_ns(tree.strict);
        let a = super::drawing_ns(tree.strict);
        let fragment=format!("<jadeLayout:sp xmlns:jadeLayout=\"{p}\" xmlns:jadeDraw=\"{a}\"><jadeLayout:nvSpPr><jadeLayout:cNvPr id=\"{id}\" name=\"{name}\"/><jadeLayout:cNvSpPr/><jadeLayout:nvPr>{ph}</jadeLayout:nvPr></jadeLayout:nvSpPr><jadeLayout:spPr/><jadeLayout:txBody><jadeDraw:bodyPr/><jadeDraw:lstStyle/><jadeDraw:p/></jadeLayout:txBody></jadeLayout:sp>");
        let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |n| n.outer.start);
        changed = apply_patches(&changed, vec![(at..at, fragment)]);
    }
    Ok(changed)
}
fn plan(
    package: &OpcPackage,
    deck: &jade_pptx_reader::Deck,
    slide: usize,
    layout: &str,
    layouts: &[SlideLayoutInfo],
) -> Result<(String, String, String, String)> {
    let part = deck
        .slides
        .get(slide)
        .ok_or(WriteError::NoSlide(slide))?
        .part_name
        .clone();
    let xml = package.required_part_str(&part)?;
    if !layouts.iter().any(|l| l.part_name == layout) {
        return Err(super::xml_error(
            "layout is not registered in this presentation",
        ));
    }
    let layout_xml = package.required_part_str(layout)?;
    if parse_document_nodes(&layout_xml)?[0].strict != parse_nodes(&xml)?[0].strict {
        return Err(super::xml_error(
            "layout namespace convention differs from the slide",
        ));
    }
    let rel_part = rels_part_name(&part);
    let rel_xml=package.part_str(&rel_part)?.unwrap_or_else(||"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let rel_nodes = parse_document_nodes(&rel_xml)?;
    if !rel_nodes.first().is_some_and(|n| n.is(4, "Relationships")) {
        return Err(super::xml_error("invalid relationship root"));
    }
    let relationships = parse_relationships(&rel_xml)?;
    let existing = relationships.by_type_suffix("/slideLayout");
    if existing.len() > 1 || existing.first().is_some_and(|r| r.external) {
        return Err(super::xml_error("ambiguous slide layout relationship"));
    }
    let same = existing
        .first()
        .is_some_and(|r| resolve_target(&part, &r.target) == layout);
    let infos = if same {
        Vec::new()
    } else {
        crate::shape_infos_in_deck(&xml, deck, slide)?
    };
    let changed = reconcile(&xml, &layout_xml, same, &infos)?;
    let references = if same {
        rel_xml.clone()
    } else if let Some(rel) = existing.first() {
        let node = rel_nodes
            .iter()
            .find(|n| n.is(4, "Relationship") && n.attr("Id") == Some(rel.id.as_str()))
            .ok_or_else(|| super::xml_error("missing relationship"))?;
        apply_patches(
            &rel_xml,
            vec![(
                node.opening.clone(),
                edit_tag(
                    &rel_xml[node.opening.clone()],
                    &[("Target", format!("/{layout}"))],
                )?,
            )],
        )
    } else {
        let ids: HashSet<_> = rel_nodes.iter().filter_map(|n| n.attr("Id")).collect();
        let id = (1..)
            .map(|i| format!("rId{i}"))
            .find(|id| !ids.contains(id.as_str()))
            .expect("relationship id");
        let ns = super::slides::r_ns(parse_nodes(&xml)?[0].strict);
        let fragment=format!("<Relationship xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{id}\" Type=\"{ns}/slideLayout\" Target=\"{}\"/>",quick_xml::escape::escape(format!("/{layout}")));
        super::slides::append_child(&rel_xml, &rel_nodes[0], &fragment)
    };
    Ok((part, changed, rel_part, references))
}
enum BatchEdit<'a> {
    Layout(&'a str),
    Reset,
}
fn batch(bytes: &[u8], indices: &[usize], edit: BatchEdit<'_>) -> Result<Vec<u8>> {
    if indices.is_empty() || indices.iter().copied().collect::<HashSet<_>>().len() != indices.len()
    {
        return Err(super::xml_error(
            "slide selection is empty or contains duplicate indexes",
        ));
    }
    let deck = jade_pptx_reader::read_deck(bytes)?;
    for &slide in indices {
        if slide >= deck.slides.len() {
            return Err(WriteError::NoSlide(slide));
        }
    }
    let mut package = OpcPackage::open_preserving(bytes)?;
    let layouts = catalog(&package)?;
    let mut planned = Vec::new();
    let mut old_infos = Vec::new();
    // Plan every target against one source snapshot before changing the package.
    for &slide in indices {
        let current = if matches!(edit, BatchEdit::Reset) {
            relation(&package, &deck.slides[slide].part_name, "/slideLayout")?
                .ok_or_else(|| super::xml_error("slide has no registered layout"))?
        } else {
            String::new()
        };
        let target = match edit {
            BatchEdit::Layout(part) => part,
            BatchEdit::Reset => &current,
        };
        let (part, mut xml, rel_part, rels) = plan(&package, &deck, slide, target, &layouts)?;
        if matches!(edit, BatchEdit::Reset) {
            old_infos.push((
                slide,
                crate::shape_infos_in_deck(&package.required_part_str(&part)?, &deck, slide)?,
            ));
            xml = reset::reset(&xml, &package.required_part_str(target)?)?;
        }
        planned.push((part, xml, rel_part, rels));
    }
    let mut changed = false;
    for (part, xml, rel_part, rels) in &planned {
        if package.part_str(part)?.as_deref() != Some(xml) {
            package.replace(part, xml.as_bytes().to_vec());
            changed = true;
        }
        if matches!(edit, BatchEdit::Layout(_))
            && package.part_str(rel_part)?.as_deref() != Some(rels)
        {
            package.replace(rel_part, rels.as_bytes().to_vec());
            changed = true;
        }
    }
    if !changed {
        return Ok(bytes.to_vec());
    }
    let saved = package.write()?;
    let new_deck = jade_pptx_reader::read_deck(&saved)?;
    for (slide, old) in old_infos {
        let part = &new_deck.slides[slide].part_name;
        let xml = planned
            .iter()
            .find(|(p, _, _, _)| p == part)
            .map(|(_, xml, _, _)| xml.as_str())
            .expect("planned slide");
        for new in crate::shape_infos_in_deck(xml, &new_deck, slide)? {
            if let Some(before) = old.iter().find(|o| o.id == new.id) {
                if before.transform.is_some() && new.transform.is_none() {
                    return Err(WriteError::InvalidTransform);
                }
                if let Some(t) = new.transform {
                    crate::validate_shape_transform(before, t)?;
                }
            }
        }
    }
    Ok(saved)
}
pub fn validate_slide_layout(bytes: &[u8], slide: usize, layout: &str) -> Result<()> {
    validate_slide_layouts(bytes, &[slide], layout)
}
pub fn set_slide_layout(bytes: &[u8], slide: usize, layout: &str) -> Result<Vec<u8>> {
    set_slide_layouts(bytes, &[slide], layout)
}
pub fn validate_slide_layouts(bytes: &[u8], slides: &[usize], layout: &str) -> Result<()> {
    set_slide_layouts(bytes, slides, layout).map(|_| ())
}
pub fn set_slide_layouts(bytes: &[u8], slides: &[usize], layout: &str) -> Result<Vec<u8>> {
    batch(bytes, slides, BatchEdit::Layout(layout))
}
pub fn validate_slide_reset(bytes: &[u8], slide: usize) -> Result<()> {
    validate_slide_resets(bytes, &[slide])
}
pub fn reset_slide(bytes: &[u8], slide: usize) -> Result<Vec<u8>> {
    reset_slides(bytes, &[slide])
}
pub fn validate_slide_resets(bytes: &[u8], slides: &[usize]) -> Result<()> {
    reset_slides(bytes, slides).map(|_| ())
}
pub fn reset_slides(bytes: &[u8], slides: &[usize]) -> Result<Vec<u8>> {
    batch(bytes, slides, BatchEdit::Reset)
}
