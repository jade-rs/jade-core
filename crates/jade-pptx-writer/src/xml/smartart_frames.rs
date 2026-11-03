//! The slide side of a SmartArt diagram: the `p:graphicFrame` holding its
//! `dgm:relIds`, inserted, its relationship ids read and blanked, and the
//! frame swapped for a group of plain shapes.

use super::{apply_patches, child, edit_tag_attributes, objects, parse_nodes, slides, tables};
use crate::Result;
use jade_pptx_reader::Transform;

const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";

/// The relationship ids a SmartArt frame names: data, layout, quick style
/// and colours. An empty one is a part left to PowerPoint's built-ins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RelIds {
    pub data: String,
    pub layout: String,
    pub style: String,
    pub colors: String,
}

fn rel_ids_node(nodes: &[super::Node], id: u32) -> Option<&super::Node> {
    let frame = tables::frame(nodes, id)?;
    let graphic = child(nodes, frame, 2, "graphic")?;
    let data = child(nodes, graphic, 2, "graphicData")?;
    data.children
        .iter()
        .map(|&index| &nodes[index])
        .find(|node| node.name == "relIds")
}

fn local_attr<'a>(node: &'a super::Node, local: &str) -> Option<(&'a str, &'a str)> {
    node.attrs
        .iter()
        .find(|(key, _)| key.rsplit(':').next() == Some(local) && key.contains(':'))
        .map(|(key, value)| (key.as_str(), value.as_str()))
}

/// The relationship ids of SmartArt frame `id`, when it is one.
pub(crate) fn rel_ids(xml: &str, id: u32) -> Result<Option<RelIds>> {
    let nodes = parse_nodes(xml)?;
    Ok(rel_ids_node(&nodes, id).map(|node| {
        let get = |name| {
            local_attr(node, name)
                .map(|(_, v)| v.to_string())
                .unwrap_or_default()
        };
        RelIds {
            data: get("dm"),
            layout: get("lo"),
            style: get("qs"),
            colors: get("cs"),
        }
    }))
}

/// Blanks the frame's layout (`lo`), style (`qs`) and colours (`cs`)
/// relationship ids named in `which`, so PowerPoint takes its built-in
/// definitions for the ids the data names.
pub(crate) fn blank_rel_ids(xml: &str, id: u32, which: &[&str]) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let Some(node) = rel_ids_node(&nodes, id) else {
        return Ok(xml.to_string());
    };
    let values: Vec<(&str, String)> = which
        .iter()
        .filter_map(|name| local_attr(node, name).map(|(key, _)| (key, String::new())))
        .collect();
    if values.is_empty() {
        return Ok(xml.to_string());
    }
    let tag = edit_tag_attributes(&xml[node.opening.clone()], &values, &[])?;
    Ok(apply_patches(xml, vec![(node.opening.clone(), tag)]))
}

/// Adds a SmartArt frame at `transform` whose data is relationship
/// `data_rid`, with no layout, style or colour part: its id.
pub(crate) fn insert_frame(
    xml: &str,
    transform: Transform,
    data_rid: &str,
) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let (tree, id, name) = objects::insertion_identity(&nodes, "Diagram")?;
    let ns = slides::p_ns(tree.strict);
    let draw = super::drawing_ns(tree.strict);
    let rel = slides::r_ns(tree.strict);
    let (x, y) = transform.offset_emu;
    let (cx, cy) = transform.extent_emu;
    let fragment = format!(
        r#"<jadeSp:graphicFrame xmlns:jadeSp="{ns}" xmlns:jadeDraw="{draw}" xmlns:jadeRel="{rel}" xmlns:jadeDgm="{DGM}"><jadeSp:nvGraphicFramePr><jadeSp:cNvPr id="{id}" name="{name}"/><jadeSp:cNvGraphicFramePr/><jadeSp:nvPr/></jadeSp:nvGraphicFramePr><jadeSp:xfrm><jadeDraw:off x="{x}" y="{y}"/><jadeDraw:ext cx="{cx}" cy="{cy}"/></jadeSp:xfrm><jadeDraw:graphic><jadeDraw:graphicData uri="{DGM}"><jadeDgm:relIds jadeRel:dm="{data_rid}" jadeRel:lo="" jadeRel:qs="" jadeRel:cs=""/></jadeDraw:graphicData></jadeDraw:graphic></jadeSp:graphicFrame>"#
    );
    let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start);
    Ok((apply_patches(xml, vec![(at..at, fragment)]), id))
}

/// Swaps SmartArt frame `id` for `group` (a `p:grpSp` fragment).
pub(crate) fn replace_frame(xml: &str, id: u32, group: &str) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let frame = tables::frame(&nodes, id).ok_or(crate::WriteError::NoShape(id))?;
    Ok(apply_patches(
        xml,
        vec![(frame.outer.clone(), group.to_string())],
    ))
}

/// The id and name a new object would get on the slide: for a group made
/// from a diagram.
pub(crate) fn fresh_identity(xml: &str, stem: &str) -> Result<(u32, String)> {
    let nodes = parse_nodes(xml)?;
    let (_, id, name) = objects::insertion_identity(&nodes, stem)?;
    Ok((id, name))
}

/// The slide's namespace URIs: (presentation, drawing, relationships).
pub(crate) fn namespaces(xml: &str) -> Result<(&'static str, &'static str, &'static str)> {
    let nodes = parse_nodes(xml)?;
    let strict = nodes[0].strict;
    Ok((
        slides::p_ns(strict),
        super::drawing_ns(strict),
        slides::r_ns(strict),
    ))
}
