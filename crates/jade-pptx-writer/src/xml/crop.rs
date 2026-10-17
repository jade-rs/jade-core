use super::{
    apply_patches, child, edit_tag, objects, parse_nodes, shape_nodes, transform_info,
    transform_xml, Node,
};
use crate::{Result, WriteError};
use jade_pptx_reader::{SourceRect, Transform};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureCropInfo {
    pub crop: SourceRect,
    pub can_edit: bool,
    pub can_preserve_scale: bool,
    pub picture: bool,
}
pub(super) fn fill<'a>(nodes: &'a [Node], shape: &Node) -> Option<&'a Node> {
    if shape.is(1, "pic") {
        child(nodes, shape, 1, "blipFill")
    } else if shape.is(1, "sp") {
        child(nodes, shape, 1, "spPr").and_then(|pr| child(nodes, pr, 2, "blipFill"))
    } else {
        None
    }
}
fn source(nodes: &[Node], fill: &Node) -> SourceRect {
    child(nodes, fill, 2, "srcRect")
        .map(|rect| {
            SourceRect::from_attributes(
                rect.attr("l"),
                rect.attr("t"),
                rect.attr("r"),
                rect.attr("b"),
            )
        })
        .unwrap_or_default()
}
fn crop_locked(nodes: &[Node], shape: &Node) -> bool {
    child(nodes, shape, 1, "nvPicPr")
        .and_then(|nv| child(nodes, nv, 1, "cNvPicPr"))
        .and_then(|nv| child(nodes, nv, 2, "picLocks"))
        .and_then(|locks| locks.attr("noCrop"))
        .is_some_and(|value| matches!(value, "1" | "true"))
}
pub(super) fn selected(nodes: &[Node], id: u32) -> Result<&Node> {
    let found: Vec<_> = shape_nodes(nodes)
        .into_iter()
        .filter(|shape| transform_info(nodes, shape).is_some_and(|info| info.id == id))
        .collect();
    if found.len() != 1 {
        return Err(WriteError::NoShape(id));
    }
    Ok(found[0])
}
pub(super) fn branches<'a>(nodes: &'a [Node], shape: &Node, id: u32) -> Vec<&'a Node> {
    let Some(owner) = objects::owner(nodes, shape) else {
        return Vec::new();
    };
    objects::descendants(nodes, owner)
        .iter()
        .filter(|node| transform_info(nodes, node).is_some_and(|info| info.id == id))
        .collect()
}
fn state(xml: &str, id: u32) -> Result<Option<PictureCropInfo>> {
    let nodes = parse_nodes(xml)?;
    let shape = selected(&nodes, id)?;
    let Some(current) = fill(&nodes, shape) else {
        return Ok(None);
    };
    let crop = source(&nodes, current);
    let mut info = transform_info(&nodes, shape).ok_or(WriteError::NoShape(id))?;
    objects::capabilities(&nodes, &objects::References::new(&nodes), shape, &mut info);
    let branches = branches(&nodes, shape, id);
    let owner = objects::owner(&nodes, shape).ok_or(WriteError::NoShape(id))?;
    let all_image_branches = objects::descendants(&nodes, owner)
        .iter()
        .filter(|n| {
            n.ns == 1
                && matches!(
                    n.name.as_str(),
                    "sp" | "pic" | "cxnSp" | "graphicFrame" | "grpSp" | "contentPart"
                )
        })
        .all(|n| branches.iter().any(|branch| std::ptr::eq(*branch, n)));
    let supported = all_image_branches
        && !branches.is_empty()
        && branches.iter().all(|shape| {
            fill(&nodes, shape).is_some_and(|fill| {
                child(&nodes, fill, 2, "blip").is_some()
                    && child(&nodes, fill, 2, "tile").is_none()
                    && source(&nodes, fill) == crop
            }) && !crop_locked(&nodes, shape)
        });
    let can_edit = supported && info.can_arrange;
    let can_preserve_scale = can_edit
        && shape.is(1, "pic")
        && valid(crop)
        && branches.iter().all(|shape| {
            transform_info(&nodes, shape).is_some_and(|branch| {
                branch.can_move && branch.can_resize && branch.transform == info.transform
            })
        });
    Ok(Some(PictureCropInfo {
        crop,
        can_edit,
        can_preserve_scale,
        picture: shape.is(1, "pic"),
    }))
}
fn valid(crop: SourceRect) -> bool {
    crop.left < 100_000
        && crop.right < 100_000
        && crop.top < 100_000
        && crop.bottom < 100_000
        && i64::from(crop.left) + i64::from(crop.right) < 100_000
        && i64::from(crop.top) + i64::from(crop.bottom) < 100_000
}
pub fn picture_crop(bytes: &[u8], slide: usize, id: u32) -> Result<Option<PictureCropInfo>> {
    let (_, _, xml) = crate::slide_xml(bytes, slide)?;
    state(&xml, id)
}
fn plan(
    bytes: &[u8],
    slide: usize,
    id: u32,
    crop: SourceRect,
    keep_frame: bool,
) -> Result<Option<Transform>> {
    if !valid(crop) {
        return Err(WriteError::InvalidPictureCrop);
    }
    let (_, _, xml) = crate::slide_xml(bytes, slide)?;
    let state = state(&xml, id)?.ok_or(WriteError::NoShape(id))?;
    if !state.can_edit {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    if keep_frame || !state.picture || crop == state.crop {
        return Ok(None);
    }
    if !state.can_preserve_scale {
        return Err(WriteError::LockedShape(id));
    }
    let deck = jade_pptx_reader::read_deck(bytes)?;
    let layout =
        jade_pptx_layout::layout_deck_slide(&deck, slide, &jade_pptx_layout::HeuristicMeasure)
            .ok_or(WriteError::NoSlide(slide))?;
    let placed = layout
        .shapes
        .iter()
        .find(|shape| {
            shape.origin == jade_pptx_layout::PlacedOrigin::Slide && shape.source_id == Some(id)
        })
        .ok_or(WriteError::NoShape(id))?;
    let after = jade_pptx_layout::crop_transform(placed.into(), state.crop, crop)
        .ok_or(WriteError::InvalidPictureCrop)?;
    let transform = Transform {
        offset_emu: (
            (after.rect.x * 9525.0).round() as i64,
            (after.rect.y * 9525.0).round() as i64,
        ),
        extent_emu: (
            (after.rect.width * 9525.0).round() as i64,
            (after.rect.height * 9525.0).round() as i64,
        ),
        rotation_60000ths: (after.rotation_degrees * 60000.0).round() as i32,
        flip_h: after.flip_h,
        flip_v: after.flip_v,
    };
    crate::validate_transform(transform)?;
    Ok(Some(transform))
}
pub fn validate_picture_crop_edit(
    bytes: &[u8],
    slide: usize,
    id: u32,
    crop: SourceRect,
    keep_frame: bool,
) -> Result<()> {
    plan(bytes, slide, id, crop, keep_frame).map(|_| ())
}

pub fn set_picture_crop(
    bytes: &[u8],
    slide: usize,
    id: u32,
    crop: SourceRect,
    keep_frame: bool,
) -> Result<Vec<u8>> {
    let transform = plan(bytes, slide, id, crop, keep_frame)?;
    let (mut package, part, xml) = crate::slide_xml(bytes, slide)?;
    let nodes = parse_nodes(&xml)?;
    let shape = selected(&nodes, id)?;
    let targets = branches(&nodes, shape, id);
    let mut patches = Vec::new();
    for shape in targets {
        let fill = fill(&nodes, shape).ok_or(WriteError::NoShape(id))?;
        let before = source(&nodes, fill);
        if before != crop {
            let values: Vec<_> = [
                ("l", before.left, crop.left),
                ("t", before.top, crop.top),
                ("r", before.right, crop.right),
                ("b", before.bottom, crop.bottom),
            ]
            .into_iter()
            .filter(|(_, old, new)| old != new)
            .map(|(key, _, value)| (key, value.to_string()))
            .collect();
            if let Some(rect) = child(&nodes, fill, 2, "srcRect") {
                patches.push((
                    rect.opening.clone(),
                    edit_tag(&xml[rect.opening.clone()], &values)?,
                ));
            } else {
                let blip = child(&nodes, fill, 2, "blip").ok_or(WriteError::NoShape(id))?;
                patches.push((blip.outer.end..blip.outer.end,format!("<cropDraw:srcRect xmlns:cropDraw=\"{}\" l=\"{}\" t=\"{}\" r=\"{}\" b=\"{}\"/>",super::drawing_ns(shape.strict),crop.left,crop.top,crop.right,crop.bottom)));
            }
        }
        if let Some(transform) = transform {
            let pr = child(&nodes, shape, 1, "spPr").ok_or(WriteError::NoShape(id))?;
            let xfrm = child(&nodes, pr, 2, "xfrm");
            let fragment = transform_xml(&xml, &nodes, xfrm, transform, shape.strict)?;
            if let Some(xfrm) = xfrm {
                patches.push((xfrm.outer.clone(), fragment));
            } else if pr.empty {
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
        }
    }
    let changed = apply_patches(&xml, patches);
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}
