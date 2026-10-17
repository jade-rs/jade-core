use super::{apply_patches, objects, parse_nodes, text, Node};
use crate::{Result, WriteError};
use jade_pptx_reader::Transform;
use std::collections::HashSet;

fn selection<'a>(nodes: &'a [Node], ids: &[u32]) -> Result<Vec<&'a Node>> {
    if ids.len() < 2 || ids.iter().copied().collect::<HashSet<_>>().len() != ids.len() {
        return Err(WriteError::InvalidGroupSelection);
    }
    let mut selected = Vec::new();
    for id in ids {
        let (owner, info) = objects::target(nodes, *id)?;
        if !info.can_group {
            return Err(WriteError::UnsupportedObjectOperation(*id));
        }
        if selected.iter().any(|n: &&Node| std::ptr::eq(*n, owner)) {
            return Err(WriteError::InvalidGroupSelection);
        }
        selected.push(owner);
    }
    selected.sort_by_key(|n| n.outer.start);
    Ok(selected)
}
pub fn validate_selection(xml: &str, ids: &[u32]) -> Result<()> {
    selection(&parse_nodes(xml)?, ids).map(|_| ())
}
pub fn group(xml: &str, ids: &[u32], bounds: Transform) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let selected = selection(&nodes, ids)?;
    let (tree, id, name) = objects::insertion_identity(&nodes, "Group")?;
    let ns = if tree.strict {
        "http://purl.oclc.org/ooxml/presentationml/main"
    } else {
        "http://schemas.openxmlformats.org/presentationml/2006/main"
    };
    let draw = super::drawing_ns(tree.strict);
    let (x, y) = bounds.offset_emu;
    let (w, h) = bounds.extent_emu;
    let children = selected
        .iter()
        .map(|node| text::scoped(xml, &nodes, node, xml[node.outer.clone()].to_owned()))
        .collect::<String>();
    let fragment=format!("<jadeGroup:grpSp xmlns:jadeGroup=\"{ns}\" xmlns:jadeDraw=\"{draw}\"><jadeGroup:nvGrpSpPr><jadeGroup:cNvPr id=\"{id}\" name=\"{name}\"/><jadeGroup:cNvGrpSpPr/><jadeGroup:nvPr/></jadeGroup:nvGrpSpPr><jadeGroup:grpSpPr><jadeDraw:xfrm><jadeDraw:off x=\"{x}\" y=\"{y}\"/><jadeDraw:ext cx=\"{w}\" cy=\"{h}\"/><jadeDraw:chOff x=\"{x}\" y=\"{y}\"/><jadeDraw:chExt cx=\"{w}\" cy=\"{h}\"/></jadeDraw:xfrm></jadeGroup:grpSpPr>{children}</jadeGroup:grpSp>");
    let last = selected.last().ok_or(WriteError::InvalidGroupSelection)?;
    let patches = selected
        .iter()
        .map(|node| {
            (
                node.outer.clone(),
                if std::ptr::eq(*node, *last) {
                    fragment.clone()
                } else {
                    String::new()
                },
            )
        })
        .collect();
    Ok((apply_patches(xml, patches), id))
}
