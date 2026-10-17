use super::{
    apply_patches, child, edit_tag, parse_nodes, shape_nodes, text, transform_info, transform_xml,
    Node,
};
use crate::{
    ObjectArrangement, Result, ShapeTransformInfo, TextFormat, TextFormatEdit, WriteError,
};
use jade_pptx_reader::Transform;
use std::collections::{HashMap, HashSet};

pub(super) struct References(HashMap<u32, Vec<usize>>, HashMap<u32, Vec<usize>>);
impl References {
    pub(super) fn new(nodes: &[Node]) -> Self {
        let mut refs = HashMap::<u32, Vec<usize>>::new();
        let mut identities = HashMap::<u32, Vec<usize>>::new();
        for (i, n) in nodes.iter().enumerate() {
            if n.is(1, "cNvPr") {
                if let Some(id) = n.attr("id").and_then(|v| v.parse().ok()) {
                    identities.entry(id).or_default().push(i);
                }
            }
            for (key, value) in &n.attrs {
                if matches!(
                    key.rsplit(':').next().unwrap_or(key),
                    "spid" | "spId" | "shapeId"
                ) || (n.ns == 2 && matches!(n.name.as_str(), "stCxn" | "endCxn") && key == "id")
                {
                    if let Ok(id) = value.parse() {
                        refs.entry(id).or_default().push(i);
                    }
                }
            }
        }
        Self(refs, identities)
    }
    fn for_id(&self, id: u32) -> impl Iterator<Item = usize> + '_ {
        self.0.get(&id).into_iter().flatten().copied()
    }
}
pub(super) fn descendants<'a>(nodes: &'a [Node], owner: &Node) -> &'a [Node] {
    let start = nodes.partition_point(|n| n.outer.start < owner.outer.start);
    let end = nodes.partition_point(|n| n.outer.start < owner.outer.end);
    &nodes[start..end]
}
fn within(node: &Node, owner: &Node) -> bool {
    node.outer.start >= owner.outer.start && node.outer.end <= owner.outer.end
}
fn tree(nodes: &[Node]) -> Result<&Node> {
    child(nodes, &nodes[0], 1, "cSld")
        .and_then(|cs| child(nodes, cs, 1, "spTree"))
        .ok_or_else(|| WriteError::Xml("slide has no shape tree".into()))
}
fn drawable(node: &Node) -> bool {
    (node.ns == 1
        && matches!(
            node.name.as_str(),
            "sp" | "pic" | "cxnSp" | "grpSp" | "graphicFrame" | "contentPart"
        ))
        || node.is(3, "AlternateContent")
}
pub(super) fn owner<'a>(nodes: &'a [Node], shape: &'a Node) -> Option<&'a Node> {
    let tree = tree(nodes).ok()?;
    let mut node = shape;
    while let Some(parent) = node.parent.map(|i| &nodes[i]) {
        if std::ptr::eq(parent, tree) {
            return Some(node);
        }
        node = parent;
    }
    None
}
fn branch_infos(nodes: &[Node], owner: &Node, id: u32) -> Option<Vec<ShapeTransformInfo>> {
    let identities: HashSet<_> = descendants(nodes, owner)
        .iter()
        .filter(|n| n.is(1, "cNvPr"))
        .filter_map(|n| n.attr("id").and_then(|v| v.parse::<u32>().ok()))
        .collect();
    if identities != HashSet::from([id]) {
        return None;
    }
    let infos: Vec<_> = descendants(nodes, owner)
        .iter()
        .filter_map(|n| transform_info(nodes, n))
        .collect();
    (!infos.is_empty() && infos.iter().all(|i| i.id == id && i.can_select)).then_some(infos)
}
fn detachable(node: &Node) -> bool {
    node.ns == 2 && matches!(node.name.as_str(), "stCxn" | "endCxn")
}
fn known_reference(node: &Node, id: u32) -> bool {
    let key = if detachable(node) {
        "id"
    } else if build(node) {
        "spid"
    } else {
        return false;
    };
    node.attr(key).and_then(|v| v.parse().ok()) == Some(id)
        && node
            .attrs
            .iter()
            .filter(|(k, v)| {
                matches!(
                    k.rsplit(':').next().unwrap_or(k),
                    "spid" | "spId" | "shapeId"
                ) && v.parse::<u32>().ok() == Some(id)
            })
            .all(|(k, _)| k == key)
}
fn build(node: &Node) -> bool {
    node.ns == 1
        && matches!(
            node.name.as_str(),
            "bldP" | "bldDgm" | "bldOleChart" | "bldGraphic"
        )
}

pub(super) fn capabilities(
    nodes: &[Node],
    refs: &References,
    shape: &Node,
    info: &mut ShapeTransformInfo,
) {
    let Some(owner) = owner(nodes, shape) else {
        info.can_duplicate = false;
        info.can_delete = false;
        info.can_arrange = false;
        info.can_group = false;
        return;
    };
    let unique = refs
        .1
        .get(&info.id)
        .is_some_and(|ids| ids.iter().all(|i| within(&nodes[*i], owner)));
    info.can_select &= unique;
    if shape.is(1, "grpSp") {
        let branches = group_branches(nodes, shape, info.id);
        info.can_select &= branches.is_some();
        info.can_arrange = info.can_select;
        info.can_duplicate = false;
        info.can_delete = false;
        if let Some(branches) = branches {
            for branch in branches {
                if let Some(branch) = transform_info(nodes, branch) {
                    info.can_move &= branch.can_move;
                    info.can_resize &= branch.can_resize;
                    info.can_rotate &= branch.can_rotate;
                    info.can_select &= branch.can_select;
                    info.lock_aspect_ratio |= branch.lock_aspect_ratio;
                }
            }
        }
        info.can_arrange &= info.can_select;
        info.can_group &= info.can_arrange
            && group_branches(nodes, shape, info.id).is_some_and(|branches| {
                branches
                    .iter()
                    .all(|branch| transform_info(nodes, branch).is_some_and(|info| info.can_group))
            });
        if let Some(ids) = subtree_ids(nodes, refs, owner) {
            let copyable_contents = descendants(nodes, owner).iter().all(|node| {
                if node.is(1, "contentPart") || node.is(1, "oleObj") || node.is(1, "control") {
                    return false;
                }
                if !node.is(1, "graphicFrame") {
                    return true;
                }
                child(nodes, node, 2, "graphic")
                    .and_then(|g| child(nodes, g, 2, "graphicData"))
                    .is_some_and(|data| {
                        child(nodes, data, 2, "tbl").is_some()
                            && data.children.iter().all(|i| nodes[*i].is(2, "tbl"))
                    })
            });
            info.can_duplicate = info.can_arrange
                && copyable_contents
                && ids
                    .iter()
                    .all(|id| refs.for_id(*id).all(|i| known_reference(&nodes[i], *id)));
            info.can_delete = info.can_arrange
                && ids.iter().all(|id| {
                    refs.for_id(*id)
                        .all(|i| within(&nodes[i], owner) || known_reference(&nodes[i], *id))
                });
        }
        return;
    }
    let supported = unique && drawable(owner) && branch_infos(nodes, owner, info.id).is_some();
    info.can_arrange = supported && info.can_select;
    info.can_group &= info.can_arrange
        && branch_infos(nodes, owner, info.id)
            .is_some_and(|branches| branches.iter().all(|info| info.can_group));
    info.can_duplicate = info.can_arrange
        && !refs
            .for_id(info.id)
            .any(|i| !known_reference(&nodes[i], info.id));
    info.can_delete = info.can_arrange
        && !refs
            .for_id(info.id)
            .any(|i| !within(&nodes[i], owner) && !known_reference(&nodes[i], info.id));
}

fn exclusive_branches(nodes: &[Node], a: &Node, b: &Node) -> bool {
    let ancestry = |node: &Node| {
        let mut branches = HashMap::new();
        let mut current = node;
        while let Some(parent) = current.parent {
            if matches!(current.name.as_str(), "Choice" | "Fallback")
                && current.ns == 3
                && nodes[parent].is(3, "AlternateContent")
            {
                branches.insert(parent, current.outer.start);
            }
            current = &nodes[parent];
        }
        branches
    };
    let a = ancestry(a);
    let b = ancestry(b);
    a.iter()
        .any(|(parent, branch)| b.get(parent).is_some_and(|other| other != branch))
}
fn subtree_ids(nodes: &[Node], refs: &References, owner: &Node) -> Option<HashSet<u32>> {
    let ids = descendants(nodes, owner)
        .iter()
        .filter(|n| n.is(1, "cNvPr"))
        .map(|n| n.attr("id")?.parse::<u32>().ok())
        .collect::<Option<HashSet<_>>>()?;
    for id in &ids {
        let identities = refs.1.get(id)?;
        if identities.iter().any(|i| !within(&nodes[*i], owner)) {
            return None;
        }
        for (i, a) in identities.iter().enumerate() {
            if identities[i + 1..]
                .iter()
                .any(|b| !exclusive_branches(nodes, &nodes[*a], &nodes[*b]))
            {
                return None;
            }
        }
    }
    Some(ids)
}
pub(super) fn group_branches<'a>(
    nodes: &'a [Node],
    shape: &'a Node,
    id: u32,
) -> Option<Vec<&'a Node>> {
    let owner = owner(nodes, shape)?;
    let groups: Vec<_> = descendants(nodes, owner)
        .iter()
        .filter(|n| n.is(1, "grpSp") && transform_info(nodes, n).is_some_and(|i| i.id == id))
        .collect();
    let first = *groups.first()?;
    let first_xfrm =
        child(nodes, first, 1, "grpSpPr").and_then(|pr| child(nodes, pr, 2, "xfrm"))?;
    let first_info = transform_info(nodes, first)?;
    let child_coords = |xfrm: &Node| {
        let off = child(nodes, xfrm, 2, "chOff")?;
        let ext = child(nodes, xfrm, 2, "chExt")?;
        let val = |n: &Node, key: &str| n.attr(key)?.parse::<i64>().ok();
        Some((
            val(off, "x")?,
            val(off, "y")?,
            val(ext, "cx")?,
            val(ext, "cy")?,
        ))
    };
    let coords = child_coords(first_xfrm)?;
    if coords.2 <= 0
        || coords.3 <= 0
        || first_info
            .transform
            .is_none_or(|t| t.extent_emu.0 <= 0 || t.extent_emu.1 <= 0)
    {
        return None;
    }
    if groups.iter().any(|group| {
        transform_info(nodes, group).is_none_or(|info| info.transform != first_info.transform)
            || child(nodes, group, 1, "grpSpPr")
                .and_then(|pr| child(nodes, pr, 2, "xfrm"))
                .and_then(child_coords)
                != Some(coords)
    }) {
        return None;
    }
    if !owner.is(1, "grpSp") && !owner.is(3, "AlternateContent") {
        return None;
    }
    // Every root of an alternate branch must be the same logical group.
    if descendants(nodes, owner).iter().any(|n| {
        drawable(n) && !n.is(3, "AlternateContent") && !groups.iter().any(|g| within(n, g))
    }) {
        return None;
    }
    Some(groups)
}
pub(super) fn target(nodes: &[Node], id: u32) -> Result<(&Node, ShapeTransformInfo)> {
    let mut found = shape_nodes(nodes).into_iter().filter_map(|n| {
        transform_info(nodes, n)
            .filter(|info| info.id == id)
            .map(|i| (n, i))
    });
    let (shape, mut info) = found.next().ok_or(WriteError::NoShape(id))?;
    if found.next().is_some() {
        return Err(WriteError::NoShape(id));
    }
    capabilities(nodes, &References::new(nodes), shape, &mut info);
    if !info.can_select {
        return Err(WriteError::LockedShape(id));
    }
    Ok((owner(nodes, shape).ok_or(WriteError::NoShape(id))?, info))
}
fn fresh_id(used: &HashSet<u32>) -> Result<u32> {
    if let Some(id) = used.iter().max().copied().unwrap_or(0).checked_add(1) {
        return Ok(id);
    }
    (1..=u32::MAX)
        .find(|id| !used.contains(id))
        .ok_or_else(|| WriteError::Xml("no unused shape id".into()))
}

pub(super) fn insertion_identity<'a>(
    nodes: &'a [Node],
    stem: &str,
) -> Result<(&'a Node, u32, String)> {
    let tree = tree(nodes)?;
    if tree.empty
        || child(nodes, tree, 1, "nvGrpSpPr").is_none()
        || child(nodes, tree, 1, "grpSpPr").is_none()
    {
        return Err(WriteError::Xml(
            "shape tree lacks required group headers".into(),
        ));
    }
    let refs = References::new(nodes);
    let used: HashSet<u32> = refs.0.keys().chain(refs.1.keys()).copied().collect();
    let id = used
        .iter()
        .max()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .or_else(|| (1..=u32::MAX).find(|id| !used.contains(id)))
        .ok_or_else(|| WriteError::Xml("no unused shape id".into()))?;
    let names: HashSet<_> = nodes
        .iter()
        .filter(|n| n.is(1, "cNvPr"))
        .filter_map(|n| n.attr("name"))
        .collect();
    let name = (1..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !names.contains(name.as_str()))
        .unwrap();
    Ok((tree, id, name))
}

pub(crate) fn insert_picture_shape(
    xml: &str,
    transform: Transform,
    rid: &str,
) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let (tree, id, name) = insertion_identity(&nodes, "Picture")?;
    let ns = super::slides::p_ns(tree.strict);
    let draw = super::drawing_ns(tree.strict);
    let rel = super::slides::r_ns(tree.strict);
    let geometry = transform_xml(xml, &nodes, None, transform, tree.strict)?;
    let fragment = format!(
        r#"<jadeSp:pic xmlns:jadeSp="{ns}" xmlns:jadeDraw="{draw}" xmlns:jadeRel="{rel}"><jadeSp:nvPicPr><jadeSp:cNvPr id="{id}" name="{name}"/><jadeSp:cNvPicPr><jadeDraw:picLocks noChangeAspect="1"/></jadeSp:cNvPicPr><jadeSp:nvPr/></jadeSp:nvPicPr><jadeSp:blipFill><jadeDraw:blip jadeRel:embed="{rid}"/><jadeDraw:stretch><jadeDraw:fillRect/></jadeDraw:stretch></jadeSp:blipFill><jadeSp:spPr>{geometry}<jadeDraw:prstGeom prst="rect"><jadeDraw:avLst/></jadeDraw:prstGeom></jadeSp:spPr></jadeSp:pic>"#
    );
    let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start);
    Ok((apply_patches(xml, vec![(at..at, fragment)]), id))
}

pub(super) fn text_box_paragraphs(text: &str, centered: bool, color: &str) -> String {
    let properties = if centered {
        "<jadeDraw:pPr algn=\"ctr\"/>"
    } else {
        ""
    };
    text.split('\n').map(|paragraph| {
        let run = if paragraph.is_empty() { String::new() } else {
            format!("<jadeDraw:r><jadeDraw:rPr sz=\"1800\"><jadeDraw:solidFill><jadeDraw:schemeClr val=\"{color}\"/></jadeDraw:solidFill></jadeDraw:rPr><jadeDraw:t>{}</jadeDraw:t></jadeDraw:r>", quick_xml::escape::escape(paragraph))
        };
        format!("<jadeDraw:p>{properties}{run}<jadeDraw:endParaRPr sz=\"1800\"><jadeDraw:solidFill><jadeDraw:schemeClr val=\"{color}\"/></jadeDraw:solidFill></jadeDraw:endParaRPr></jadeDraw:p>")
    }).collect()
}

pub(crate) fn insert_text_box(
    xml: &str,
    transform: Transform,
    text: &str,
    format: &TextFormat,
) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let (tree, id, name) = insertion_identity(&nodes, "TextBox")?;
    let ns = if tree.strict {
        "http://purl.oclc.org/ooxml/presentationml/main"
    } else {
        "http://schemas.openxmlformats.org/presentationml/2006/main"
    };
    let draw = super::drawing_ns(tree.strict);
    let geometry = transform_xml(xml, &nodes, None, transform, tree.strict)?;
    let paragraphs = text_box_paragraphs(text, false, "tx1");
    let fragment = format!(
        r#"<jadeSp:sp xmlns:jadeSp="{ns}" xmlns:jadeDraw="{draw}"><jadeSp:nvSpPr><jadeSp:cNvPr id="{id}" name="{name}"/><jadeSp:cNvSpPr txBox="1"/><jadeSp:nvPr/></jadeSp:nvSpPr><jadeSp:spPr>{geometry}<jadeDraw:prstGeom prst="rect"><jadeDraw:avLst/></jadeDraw:prstGeom><jadeDraw:noFill/><jadeDraw:ln><jadeDraw:noFill/></jadeDraw:ln></jadeSp:spPr><jadeSp:txBody><jadeDraw:bodyPr wrap="square" lIns="91440" rIns="91440" tIns="45720" bIns="45720" anchor="t"><jadeDraw:noAutofit/></jadeDraw:bodyPr><jadeDraw:lstStyle/>{paragraphs}</jadeSp:txBody></jadeSp:sp>"#
    );
    let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start);
    let inserted = apply_patches(xml, vec![(at..at, fragment)]);
    let changed = super::format_text_range(
        &inserted,
        &TextFormatEdit {
            shape_id: id,
            start: 0,
            end: text.encode_utf16().count(),
            format: format.clone(),
        },
    )?;
    Ok((changed, id))
}
fn fresh_name(nodes: &[Node], name: &str) -> String {
    let used: HashSet<&str> = nodes
        .iter()
        .filter(|n| n.is(1, "cNvPr"))
        .filter_map(|n| n.attr("name"))
        .collect();
    let stem = if name.is_empty() { "Object" } else { name };
    for index in 1.. {
        let name = if index == 1 {
            format!("{stem} (copy)")
        } else {
            format!("{stem} (copy {index})")
        };
        if !used.contains(name.as_str()) {
            return name;
        }
    }
    unreachable!()
}

pub(crate) fn duplicate(xml: &str, id: u32, baseline: Option<Transform>) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let (source, info) = target(&nodes, id)?;
    if !info.can_duplicate {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let refs = References::new(&nodes);
    let mut reserved: HashSet<_> = refs.0.keys().chain(refs.1.keys()).copied().collect();
    let mut old_ids: Vec<_> = if info.kind == "group" {
        subtree_ids(&nodes, &refs, source)
            .ok_or(WriteError::UnsupportedObjectOperation(id))?
            .into_iter()
            .collect()
    } else {
        vec![id]
    };
    old_ids.sort_unstable();
    old_ids.retain(|old| *old != id);
    old_ids.insert(0, id);
    let mut id_map = HashMap::new();
    for old in old_ids {
        let fresh = fresh_id(&reserved)?;
        reserved.insert(fresh);
        id_map.insert(old, fresh);
    }
    let new_id = id_map[&id];
    let name = fresh_name(&nodes, &info.name);
    let can_offset = if info.kind == "group" {
        info.can_move
    } else {
        branch_infos(&nodes, source, id).is_some_and(|branches| branches.iter().all(|b| b.can_move))
    };
    let mut creation_ids = HashMap::<String, String>::new();
    let mut patches = Vec::new();
    for n in descendants(&nodes, source) {
        if n.is(1, "cNvPr") {
            let old = n
                .attr("id")
                .and_then(|v| v.parse().ok())
                .ok_or(WriteError::UnsupportedObjectOperation(id))?;
            let fresh = id_map
                .get(&old)
                .ok_or(WriteError::UnsupportedObjectOperation(id))?;
            let values = if old == id {
                vec![("id", fresh.to_string()), ("name", name.clone())]
            } else {
                vec![("id", fresh.to_string())]
            };
            patches.push((
                n.opening.clone(),
                edit_tag(&xml[n.opening.clone()], &values)?,
            ));
        } else if detachable(n) || build(n) {
            let key = if detachable(n) { "id" } else { "spid" };
            if let Some(fresh) = n
                .attr(key)
                .and_then(|v| v.parse::<u32>().ok())
                .and_then(|old| id_map.get(&old))
            {
                patches.push((
                    n.opening.clone(),
                    edit_tag(&xml[n.opening.clone()], &[(key, fresh.to_string())])?,
                ));
            }
        }
        if n.is(8, "creationId") {
            if let Some(old) = n.attr("id") {
                let fresh = creation_ids.entry(old.to_owned()).or_insert_with(|| {
                    format!(
                        "{{{}}}",
                        uuid::Uuid::new_v4().to_string().to_ascii_uppercase()
                    )
                });
                patches.push((
                    n.opening.clone(),
                    edit_tag(&xml[n.opening.clone()], &[("id", fresh.clone())])?,
                ));
            }
        }
        if let Some(branch) = transform_info(&nodes, n).filter(|branch| branch.id == id) {
            if let Some(mut moved) = branch.transform.or(baseline).filter(|_| can_offset) {
                // A 10 pt visual offset is only an insertion convenience. Keep
                // the original position when a boundary prevents the offset.
                let original = moved;
                moved.offset_emu = (
                    moved.offset_emu.0.saturating_add(127000),
                    moved.offset_emu.1.saturating_add(127000),
                );
                if crate::validate_transform(moved).is_err() {
                    moved = original;
                }
                if moved != original {
                    let pr = child(
                        &nodes,
                        n,
                        1,
                        if info.kind == "group" {
                            "grpSpPr"
                        } else {
                            "spPr"
                        },
                    );
                    let xfrm = pr.and_then(|pr| child(&nodes, pr, 2, "xfrm"));
                    let fragment = transform_xml(xml, &nodes, xfrm, moved, n.strict)?;
                    if let Some(xfrm) = xfrm {
                        patches.push((xfrm.outer.clone(), fragment));
                    } else if let Some(pr) = pr {
                        if pr.empty {
                            patches.push((
                                pr.outer.clone(),
                                format!(
                                    "{}{fragment}</{}>",
                                    pr.empty_open.as_deref().unwrap_or_default(),
                                    pr.qualified
                                ),
                            ));
                        } else {
                            patches.push((pr.range.start..pr.range.start, fragment));
                        }
                    } else {
                        let nv = n
                            .children
                            .iter()
                            .map(|i| &nodes[*i])
                            .find(|c| {
                                c.ns == 1
                                    && matches!(c.name.as_str(), "nvSpPr" | "nvPicPr" | "nvCxnSpPr")
                            })
                            .ok_or(WriteError::UnsupportedObjectOperation(id))?;
                        let tag = n.qualified.rsplit_once(':').map_or_else(
                            || "spPr".to_owned(),
                            |(prefix, _)| format!("{prefix}:spPr"),
                        );
                        patches.push((
                            nv.outer.end..nv.outer.end,
                            format!("<{tag}>{fragment}</{tag}>"),
                        ));
                    }
                }
            }
        }
    }
    let local = patches
        .into_iter()
        .map(|(r, s)| (r.start - source.outer.start..r.end - source.outer.start, s))
        .collect();
    let fragment = text::scoped(
        xml,
        &nodes,
        source,
        apply_patches(&xml[source.outer.clone()], local),
    );
    let tree = tree(&nodes)?;
    let at = tree
        .children
        .iter()
        .map(|i| &nodes[*i])
        .rfind(|n| drawable(n))
        .map_or(tree.range.start, |n| n.outer.end);
    let mut patches = vec![(at..at, fragment)];
    for old_id in id_map.keys() {
        for i in refs.for_id(*old_id) {
            let node = &nodes[i];
            if !within(node, source) && build(node) {
                let value = apply_patches(
                    &xml[node.outer.clone()],
                    vec![(
                        0..node.opening.len(),
                        edit_tag(
                            &xml[node.opening.clone()],
                            &[("spid", id_map[old_id].to_string())],
                        )?,
                    )],
                );
                patches.push((node.outer.end..node.outer.end, value));
            }
        }
    }
    Ok((apply_patches(xml, patches), new_id))
}

pub(crate) fn delete(xml: &str, id: u32) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (source, info) = target(&nodes, id)?;
    if !info.can_delete {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let mut removed = vec![source.outer.clone()];
    let refs = References::new(&nodes);
    let ids = if info.kind == "group" {
        subtree_ids(&nodes, &refs, source).ok_or(WriteError::UnsupportedObjectOperation(id))?
    } else {
        HashSet::from([id])
    };
    for old_id in ids {
        for i in refs.for_id(old_id) {
            let n = &nodes[i];
            if !within(n, source) && (detachable(n) || build(n)) {
                removed.push(n.outer.clone());
            }
        }
    }
    // bldLst is optional but requires at least one child when retained.
    for n in &nodes {
        if n.is(1, "bldLst")
            && !n.children.is_empty()
            && n.children.iter().all(|i| {
                removed
                    .iter()
                    .any(|r| r.start <= nodes[*i].outer.start && r.end >= nodes[*i].outer.end)
            })
        {
            removed.push(n.outer.clone());
        }
    }
    removed.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut outer = Vec::<std::ops::Range<usize>>::new();
    for range in removed {
        if outer.last().is_none_or(|r| r.end < range.end) {
            outer.push(range);
        }
    }
    Ok(apply_patches(
        xml,
        outer.into_iter().map(|r| (r, String::new())).collect(),
    ))
}

pub(crate) fn arrange(xml: &str, id: u32, direction: ObjectArrangement) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (source, info) = target(&nodes, id)?;
    if !info.can_arrange {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let tree = tree(&nodes)?;
    let slots: Vec<_> = tree
        .children
        .iter()
        .map(|i| &nodes[*i])
        .filter(|n| drawable(n))
        .collect();
    let from = slots
        .iter()
        .position(|n| std::ptr::eq(*n, source))
        .ok_or(WriteError::NoShape(id))?;
    let to = match direction {
        ObjectArrangement::Front => slots.len() - 1,
        ObjectArrangement::Back => 0,
        ObjectArrangement::Forward => (from + 1).min(slots.len() - 1),
        ObjectArrangement::Backward => from.saturating_sub(1),
    };
    if from == to {
        return Ok(xml.to_owned());
    }
    let mut reordered = slots.clone();
    let object = reordered.remove(from);
    reordered.insert(to, object);
    Ok(apply_patches(
        xml,
        slots
            .iter()
            .zip(&reordered)
            .filter(|(a, b)| !std::ptr::eq(**a, **b))
            .map(|(a, b)| (a.outer.clone(), xml[b.outer.clone()].to_owned()))
            .collect(),
    ))
}
