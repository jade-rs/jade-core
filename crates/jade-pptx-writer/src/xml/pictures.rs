use crate::{xml, Result, WriteError};
use image::{metadata::Orientation, ImageDecoder, ImageFormat, ImageReader, Limits};
use jade_opc::{rels_part_name, resolve_target, OpcPackage};
use jade_pptx_reader::Transform;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io::Cursor,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureInfo {
    pub width: u32,
    pub height: u32,
    pub mime_type: &'static str,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureInsertResult {
    pub id: u32,
    pub part: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureReplacementInfo {
    pub can_replace: bool,
    pub fingerprint: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureReplaceResult {
    pub part: String,
    pub changed: bool,
}

/// Validate decoded pixels as well as the header before modifying a document.
pub fn picture_info(bytes: &[u8]) -> Result<PictureInfo> {
    let invalid = |e: image::ImageError| WriteError::InvalidPicture(e.to_string());
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(WriteError::InvalidPicture(
            "encoded image exceeds 64 MiB".into(),
        ));
    }
    let format = image::guess_format(bytes).map_err(invalid)?;
    let mime_type = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        _ => {
            return Err(WriteError::InvalidPicture(
                "only PNG and JPEG are supported".into(),
            ))
        }
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(invalid)?;
    let orientation = decoder.orientation().map_err(invalid)?;
    let (mut width, mut height) = decoder.dimensions();
    if width == 0
        || height == 0
        || decoder.total_bytes() > 256 * 1024 * 1024
        || u64::from(width) * u64::from(height) > 64 * 1024 * 1024
    {
        return Err(WriteError::InvalidPicture(
            "decoded image exceeds pixel limits".into(),
        ));
    }
    image::DynamicImage::from_decoder(decoder).map_err(invalid)?;
    if format == ImageFormat::Png {
        // Image decoding can stop before IEND; validate the remaining chunks too.
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_limits(png::Limits {
            bytes: 256 * 1024 * 1024,
        });
        decoder
            .read_info()
            .and_then(|mut reader| reader.finish())
            .map_err(|e| WriteError::InvalidPicture(e.to_string()))?;
    }
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    Ok(PictureInfo {
        width,
        height,
        mime_type,
    })
}

fn media_part(package: &OpcPackage, bytes: &[u8], mime: &str) -> Result<String> {
    let mut reserved: HashSet<String> = package.part_names().iter().cloned().collect();
    let types = package.required_part_str("[Content_Types].xml")?;
    for node in xml::parse_document_nodes(&types)? {
        if node.is(5, "Override") {
            if let Some(part) = node.attr("PartName") {
                reserved.insert(part.trim_start_matches('/').into());
            }
        }
    }
    for name in package
        .part_names()
        .iter()
        .filter(|name| name.ends_with(".rels"))
    {
        let rels = package.required_part_str(name)?;
        let owner = if name == "_rels/.rels" {
            String::new()
        } else {
            name.strip_prefix("_rels/")
                .unwrap_or(name)
                .replace("/_rels/", "/")
                .trim_end_matches(".rels")
                .to_owned()
        };
        for node in xml::parse_document_nodes(&rels)? {
            if node.is(4, "Relationship") && node.attr("TargetMode") != Some("External") {
                if let Some(target) = node.attr("Target") {
                    reserved.insert(resolve_target(&owner, target));
                }
            }
        }
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    let extension = if mime == "image/png" { "png" } else { "jpeg" };
    for suffix in 0.. {
        let tail = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let part = format!("ppt/media/jade-picture-{digest}{tail}.{extension}");
        if !reserved.contains(&part) {
            return Ok(part);
        }
    }
    unreachable!()
}

pub fn insert_picture(
    bytes: &[u8],
    slide: usize,
    transform: Transform,
    image: &[u8],
) -> Result<(Vec<u8>, PictureInsertResult)> {
    crate::validate_transform(transform)?;
    if transform.extent_emu.0 <= 0 || transform.extent_emu.1 <= 0 {
        return Err(WriteError::InvalidTransform);
    }
    let info = picture_info(image)?;
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let nodes = xml::parse_nodes(&slide_xml)?;
    let (part, rid) = install_image(&mut package, &slide_part, &nodes, image, &info)?;
    let (changed, id) = xml::objects::insert_picture_shape(&slide_xml, transform, &rid)?;
    package.replace(&slide_part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, PictureInsertResult { id, part }))
}

fn install_image(
    package: &mut OpcPackage,
    slide_part: &str,
    nodes: &[xml::Node],
    image: &[u8],
    info: &PictureInfo,
) -> Result<(String, String)> {
    let part = media_part(package, image, info.mime_type)?;
    let rel_part = rels_part_name(slide_part);
    let old_rels = package.part_str(&rel_part)?.unwrap_or_else(|| "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let rel_nodes = xml::parse_document_nodes(&old_rels)?;
    let root = rel_nodes
        .first()
        .filter(|node| node.is(4, "Relationships"))
        .ok_or_else(|| WriteError::Xml("missing relationships root".into()))?;
    let mut ids: HashSet<_> = rel_nodes
        .iter()
        .filter_map(|node| node.attr("Id").map(str::to_owned))
        .collect();
    for node in nodes {
        for (key, value) in &node.attrs {
            if matches!(key.rsplit(':').next(), Some("id" | "embed" | "link")) {
                ids.insert(value.clone());
            }
        }
    }
    let rid = (1..)
        .map(|n| format!("rIdJadePicture{n}"))
        .find(|rid| !ids.contains(rid))
        .unwrap();
    let rel_type = xml::slides::r_ns(nodes[0].strict);
    let relation = format!("<jadePkg:Relationship xmlns:jadePkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{rid}\" Type=\"{rel_type}/image\" Target=\"/{part}\"/>");
    package.replace(
        &rel_part,
        xml::slides::append_child(&old_rels, root, &relation).into_bytes(),
    );
    package.replace(&part, image.to_vec());
    xml::slides::content_types(
        package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((&part, info.mime_type)),
    )?;
    Ok((part, rid))
}

fn embed(blip: &xml::Node) -> Option<(&str, &str)> {
    let mut attrs = blip
        .attrs
        .iter()
        .filter(|(key, _)| key.rsplit(':').next() == Some("embed"));
    let (key, value) = attrs.next()?;
    (attrs.next().is_none() && key.contains(':')).then_some((key.as_str(), value.as_str()))
}
fn replacement_state(
    package: &OpcPackage,
    slide_part: &str,
    slide_xml: &str,
    id: u32,
) -> Result<Option<PictureReplacementInfo>> {
    let nodes = xml::parse_nodes(slide_xml)?;
    let shape = xml::crop::selected(&nodes, id)?;
    if xml::crop::fill(&nodes, shape).is_none() {
        return Ok(None);
    }
    let mut info = xml::transform_info(&nodes, shape).ok_or(WriteError::NoShape(id))?;
    xml::objects::capabilities(
        &nodes,
        &xml::objects::References::new(&nodes),
        shape,
        &mut info,
    );
    let owner = xml::objects::owner(&nodes, shape).ok_or(WriteError::NoShape(id))?;
    let branches = xml::crop::branches(&nodes, shape, id);
    let mut can_replace = info.can_arrange
        && !branches.is_empty()
        && xml::objects::descendants(&nodes, owner)
            .iter()
            .filter(|n| {
                n.ns == 1
                    && matches!(
                        n.name.as_str(),
                        "sp" | "pic" | "cxnSp" | "graphicFrame" | "grpSp" | "contentPart"
                    )
            })
            .all(|n| branches.iter().any(|b| std::ptr::eq(*b, n)));
    let rels = package
        .part_str(&rels_part_name(slide_part))?
        .unwrap_or_default();
    let rel_nodes = xml::parse_document_nodes(&rels)?;
    let mut digest = Sha256::new();
    for branch in branches {
        let blip = xml::crop::fill(&nodes, branch).and_then(|f| xml::child(&nodes, f, 2, "blip"));
        let Some(blip) = blip else {
            can_replace = false;
            continue;
        };
        digest.update(&slide_xml[blip.outer.clone()]);
        can_replace &= !blip
            .attrs
            .iter()
            .any(|(key, _)| key.rsplit(':').next() == Some("link"))
            && xml::child(&nodes, blip, 2, "extLst").is_none();
        let Some((_, rid)) = embed(blip) else {
            can_replace = false;
            continue;
        };
        let matches: Vec<_> = rel_nodes
            .iter()
            .filter(|r| r.is(4, "Relationship") && r.attr("Id") == Some(rid))
            .collect();
        for relation in &matches {
            digest.update(&rels[relation.outer.clone()]);
        }
        can_replace &= matches.len() == 1
            && matches.iter().all(|r| {
                r.attr("TargetMode") != Some("External")
                    && r.attr("Type")
                        .is_some_and(|t| t == format!("{}/image", xml::slides::r_ns(branch.strict)))
                    && r.attr("Target")
                        .is_some_and(|t| package.part(&resolve_target(slide_part, t)).is_some())
            });
    }
    Ok(Some(PictureReplacementInfo {
        can_replace,
        fingerprint: format!("{:x}", digest.finalize()),
    }))
}
pub fn picture_replacement(
    bytes: &[u8],
    slide: usize,
    id: u32,
) -> Result<Option<PictureReplacementInfo>> {
    let (package, part, xml) = crate::slide_xml(bytes, slide)?;
    replacement_state(&package, &part, &xml, id)
}
pub fn replace_picture(
    bytes: &[u8],
    slide: usize,
    id: u32,
    image: &[u8],
    expected_fingerprint: &str,
) -> Result<(Vec<u8>, PictureReplaceResult)> {
    let info = picture_info(image)?;
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let state =
        replacement_state(&package, &slide_part, &slide_xml, id)?.ok_or(WriteError::NoShape(id))?;
    if !state.can_replace || state.fingerprint != expected_fingerprint {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let nodes = xml::parse_nodes(&slide_xml)?;
    let shape = xml::crop::selected(&nodes, id)?;
    let targets = xml::crop::branches(&nodes, shape, id);
    let rels =
        xml::parse_document_nodes(&package.required_part_str(&rels_part_name(&slide_part))?)?;
    let old_parts: Vec<_> = targets
        .iter()
        .map(|shape| {
            let blip =
                xml::child(&nodes, xml::crop::fill(&nodes, shape).unwrap(), 2, "blip").unwrap();
            let rid = embed(blip).unwrap().1;
            let rel = rels
                .iter()
                .find(|r| r.is(4, "Relationship") && r.attr("Id") == Some(rid))
                .unwrap();
            resolve_target(&slide_part, rel.attr("Target").unwrap())
        })
        .collect();
    if old_parts.iter().all(|p| package.part(p) == Some(image)) {
        return Ok((
            bytes.to_vec(),
            PictureReplaceResult {
                part: old_parts[0].clone(),
                changed: false,
            },
        ));
    }
    let (part, rid) = install_image(&mut package, &slide_part, &nodes, image, &info)?;
    let patches = targets
        .iter()
        .map(|shape| {
            let blip =
                xml::child(&nodes, xml::crop::fill(&nodes, shape).unwrap(), 2, "blip").unwrap();
            let key = embed(blip).unwrap().0;
            Ok((
                blip.opening.clone(),
                xml::edit_tag(&slide_xml[blip.opening.clone()], &[(key, rid.clone())])?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    package.replace(
        &slide_part,
        xml::apply_patches(&slide_xml, patches).into_bytes(),
    );
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((
        result,
        PictureReplaceResult {
            part,
            changed: true,
        },
    ))
}
