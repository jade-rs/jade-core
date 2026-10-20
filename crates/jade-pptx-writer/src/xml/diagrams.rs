//! SmartArt frames (`p:graphicFrame` holding a `dgm:relIds`): the drawing
//! PowerPoint caches for one, scaled to a resized frame.

use super::{apply_patches, child, edit_tag, parse_document_nodes, parse_nodes, tables};
use crate::Result;

/// The `r:dm` relationship of the SmartArt frame `id`: its data part, which
/// names the drawing.
pub(crate) fn data_relationship(xml: &str, id: u32) -> Result<Option<String>> {
    let nodes = parse_nodes(xml)?;
    Ok(tables::frame(&nodes, id)
        .and_then(|frame| child(&nodes, frame, 2, "graphic"))
        .and_then(|graphic| child(&nodes, graphic, 2, "graphicData"))
        .and_then(|data| {
            data.children
                .iter()
                .map(|&index| &nodes[index])
                .find(|node| node.name == "relIds")
        })
        .and_then(|rel_ids| {
            rel_ids
                .attrs
                .iter()
                .find(|(key, _)| key.rsplit(':').next() == Some("dm"))
                .map(|(_, value)| value.clone())
        }))
}

/// `xml`, a cached SmartArt drawing (`dsp:drawing`), with every box scaled
/// from a frame of `from` to one of `to` (EMU, width and height): each
/// shape's and text box's offset and extent, and a group's child offset and
/// extent — so the drawing fills the resized frame as it filled the old.
pub(crate) fn scale_drawing(xml: &str, from: (i64, i64), to: (i64, i64)) -> Result<String> {
    let nodes = parse_document_nodes(xml)?;
    let scale = |value: i64, to: i64, from: i64| {
        if from == 0 {
            value
        } else {
            (i128::from(value) * i128::from(to) / i128::from(from)) as i64
        }
    };
    let mut patches = Vec::new();
    for node in nodes.iter().filter(|node| node.ns == 2) {
        let keys = match node.name.as_str() {
            "off" | "chOff" => ["x", "y"],
            "ext" | "chExt" => ["cx", "cy"],
            _ => continue,
        };
        let values: Vec<(&str, String)> = keys
            .iter()
            .zip([(to.0, from.0), (to.1, from.1)])
            .filter_map(|(key, (to, from))| {
                let value = node.attr(key)?.parse::<i64>().ok()?;
                Some((*key, scale(value, to, from).to_string()))
            })
            .collect();
        // An `a:ext` of an extension list has no box.
        if values.is_empty() {
            continue;
        }
        patches.push((
            node.opening.clone(),
            edit_tag(&xml[node.opening.clone()], &values)?,
        ));
    }
    Ok(apply_patches(xml, patches))
}
