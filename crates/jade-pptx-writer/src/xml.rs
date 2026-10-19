use crate::{EditableRun, Result, RunAddress, ShapeTransformInfo, WriteError};
use jade_pptx_reader::Transform;
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::ops::Range;

pub(crate) mod crop;
mod formatting;
pub(crate) mod grouping;
mod objects;
pub(crate) mod pictures;
pub(crate) mod shape_insertion;
pub(crate) mod shape_style;
pub(crate) use objects::{
    arrange as arrange_shape, delete as delete_shape, duplicate as duplicate_shape, insert_text_box,
};
mod paragraphs;
pub(crate) use paragraphs::format_paragraphs;
pub(crate) mod layouts;
mod notes;
mod slides;
pub(crate) use notes::{notes_snapshot, replace_notes_range, set_notes};
mod text;
pub(crate) use formatting::format_text_range;
pub(crate) use slides::{edit_slides, SlideEdit};
pub(crate) use text::{replace_text_range, text_snapshot};

pub(crate) struct RunAnchor {
    pub value: EditableRun,
    pub range: Range<usize>,
    pub empty_name: Option<String>,
    pub empty_open: Option<String>,
}

struct Node {
    ns: u8,
    name: String,
    qualified: String,
    attrs: Vec<(String, String)>,
    children: Vec<usize>,
    range: Range<usize>,
    empty: bool,
    empty_open: Option<String>,
    text: String,
    opening: Range<usize>,
    outer: Range<usize>,
    strict: bool,
    parent: Option<usize>,
}

impl Node {
    fn is(&self, ns: u8, name: &str) -> bool {
        self.ns == ns && self.name == name
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn child<'a>(nodes: &'a [Node], node: &Node, ns: u8, name: &str) -> Option<&'a Node> {
    node.children
        .iter()
        .map(|&i| &nodes[i])
        .find(|n| n.is(ns, name))
}

fn xml_error(error: impl std::fmt::Display) -> WriteError {
    WriteError::Xml(error.to_string())
}

fn parse_nodes(xml: &str) -> Result<Vec<Node>> {
    let nodes = parse_document_nodes(xml)?;
    if !nodes
        .first()
        .is_some_and(|root| root.ns == 1 && matches!(root.name.as_str(), "sld" | "notes"))
    {
        return Err(xml_error("expected a slide or notes root"));
    }
    Ok(nodes)
}

fn parse_document_nodes(xml: &str) -> Result<Vec<Node>> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut buffer = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let (namespace, event) = reader
            .read_resolved_event_into(&mut buffer)
            .map_err(xml_error)?;
        let strict = matches!(&namespace, ResolveResult::Bound(ns) if ns.as_ref().starts_with(b"http://purl.oclc.org/"));
        let ns = match namespace {
            ResolveResult::Bound(ns) => match ns.as_ref() {
                b"http://schemas.openxmlformats.org/presentationml/2006/main"
                | b"http://purl.oclc.org/ooxml/presentationml/main" => 1,
                b"http://schemas.openxmlformats.org/drawingml/2006/main"
                | b"http://purl.oclc.org/ooxml/drawingml/main" => 2,
                b"http://schemas.openxmlformats.org/markup-compatibility/2006" => 3,
                b"http://schemas.openxmlformats.org/package/2006/relationships" => 4,
                b"http://schemas.openxmlformats.org/package/2006/content-types" => 5,
                b"http://schemas.microsoft.com/office/powerpoint/2010/main" => 6,
                b"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" => 7,
                b"http://schemas.microsoft.com/office/drawing/2014/main" => 8,
                _ => 0,
            },
            _ => 0,
        };
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let qualified = std::str::from_utf8(element.name().as_ref())
                    .map_err(xml_error)?
                    .to_owned();
                let name = std::str::from_utf8(element.local_name().as_ref())
                    .map_err(xml_error)?
                    .to_owned();
                let attrs = element
                    .attributes()
                    .map(|a| {
                        let a = a.map_err(xml_error)?;
                        Ok((
                            std::str::from_utf8(a.key.as_ref())
                                .map_err(xml_error)?
                                .to_owned(),
                            a.unescape_value().map_err(xml_error)?.into_owned(),
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let empty = matches!(event, Event::Empty(_));
                let index = nodes.len();
                if let Some(&parent) = stack.last() {
                    nodes[parent].children.push(index);
                }
                nodes.push(Node {
                    ns,
                    name,
                    qualified,
                    attrs,
                    children: Vec::new(),
                    range: if empty { start..end } else { end..end },
                    empty,
                    empty_open: empty.then(|| format!("{}>", &xml[start..end - 2])),
                    text: String::new(),
                    opening: start..end,
                    outer: start..end,
                    strict,
                    parent: stack.last().copied(),
                });
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                let index = stack.pop().ok_or_else(|| xml_error("unbalanced XML"))?;
                nodes[index].range.end = start;
                nodes[index].outer.end = end;
            }
            Event::Text(text) => {
                if let Some(&index) = stack.last() {
                    nodes[index]
                        .text
                        .push_str(&text.xml_content().map_err(xml_error)?);
                }
            }
            Event::CData(text) => {
                if let Some(&index) = stack.last() {
                    nodes[index]
                        .text
                        .push_str(&text.decode().map_err(xml_error)?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(&index) = stack.last() {
                    let value = reference.decode().map_err(xml_error)?;
                    nodes[index].text.push_str(
                        &quick_xml::escape::unescape(&format!("&{value};")).map_err(xml_error)?,
                    );
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !stack.is_empty() {
        return Err(xml_error("unclosed XML"));
    }
    Ok(nodes)
}

fn direct_shapes<'a>(nodes: &'a [Node], parent: &Node, out: &mut Vec<&'a Node>) {
    for &index in &parent.children {
        let node = &nodes[index];
        if node.is(1, "sp")
            || node.is(1, "pic")
            || node.is(1, "cxnSp")
            || node.is(1, "grpSp")
            || node.is(1, "graphicFrame")
        {
            out.push(node);
        } else if node.is(3, "AlternateContent") {
            if let Some(fallback) = child(nodes, node, 3, "Fallback") {
                direct_shapes(nodes, fallback, out);
            }
        }
    }
}

fn transform_info(nodes: &[Node], shape: &Node) -> Option<ShapeTransformInfo> {
    let (nv_name, locks_parent, locks_name) = match shape.name.as_str() {
        "sp" => ("nvSpPr", "cNvSpPr", "spLocks"),
        "pic" => ("nvPicPr", "cNvPicPr", "picLocks"),
        "cxnSp" => ("nvCxnSpPr", "cNvCxnSpPr", "cxnSpLocks"),
        "grpSp" => ("nvGrpSpPr", "cNvGrpSpPr", "grpSpLocks"),
        "graphicFrame" => ("nvGraphicFramePr", "cNvGraphicFramePr", "graphicFrameLocks"),
        _ => return None,
    };
    // A table, chart or diagram: its frame carries its own `p:xfrm`.
    let frame = shape.name == "graphicFrame";
    let nv = child(nodes, shape, 1, nv_name)?;
    let identity = child(nodes, nv, 1, "cNvPr")?;
    let id = identity.attr("id")?.parse().ok()?;
    let locks =
        child(nodes, nv, 1, locks_parent).and_then(|parent| child(nodes, parent, 2, locks_name));
    let locked = |name| {
        locks
            .and_then(|locks| locks.attr(name))
            .is_some_and(|value| matches!(value, "1" | "true"))
    };
    let xfrm = if frame {
        child(nodes, shape, 1, "xfrm")
    } else {
        child(
            nodes,
            shape,
            1,
            if shape.name == "grpSp" {
                "grpSpPr"
            } else {
                "spPr"
            },
        )
        .and_then(|pr| child(nodes, pr, 2, "xfrm"))
    };
    let transform = xfrm.map(|xfrm| {
        let off = child(nodes, xfrm, 2, "off");
        let ext = child(nodes, xfrm, 2, "ext");
        let coord = |node: Option<&Node>, name| {
            node.and_then(|n| n.attr(name))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0)
        };
        Transform {
            offset_emu: (coord(off, "x"), coord(off, "y")),
            extent_emu: (coord(ext, "cx"), coord(ext, "cy")),
            rotation_60000ths: xfrm.attr("rot").and_then(|v| v.parse().ok()).unwrap_or(0),
            flip_h: xfrm
                .attr("flipH")
                .is_some_and(|v| matches!(v, "1" | "true")),
            flip_v: xfrm
                .attr("flipV")
                .is_some_and(|v| matches!(v, "1" | "true")),
        }
    });
    Some(ShapeTransformInfo {
        id,
        name: identity.attr("name").unwrap_or_default().into(),
        kind: if shape.name == "grpSp" {
            "group"
        } else if shape.name == "pic" {
            "picture"
        } else if frame {
            frame_kind(nodes, shape)
        } else {
            "shape"
        }
        .into(),
        transform,
        can_move: !locked("noMove"),
        // A frame moves; resizing a table would need its column widths to
        // follow, and a diagram its drawing, so a frame keeps its size, and
        // PowerPoint does not rotate one.
        can_resize: !frame && !locked("noResize"),
        can_rotate: !frame && !locked("noRot"),
        can_select: !locked("noSelect"),
        lock_aspect_ratio: locked("noChangeAspect"),
        can_duplicate: !locked("noSelect"),
        can_delete: !locked("noSelect"),
        can_arrange: !locked("noSelect"),
        can_group: !frame
            && !locked("noSelect")
            && !locked("noGrp")
            && child(nodes, nv, 1, "nvPr")
                .and_then(|pr| child(nodes, pr, 1, "ph"))
                .is_none(),
    })
}

/// What a graphic frame holds, by its `graphicData` URI.
fn frame_kind(nodes: &[Node], frame: &Node) -> &'static str {
    let uri = child(nodes, frame, 2, "graphic")
        .and_then(|graphic| child(nodes, graphic, 2, "graphicData"))
        .and_then(|data| data.attr("uri"))
        .unwrap_or_default();
    if uri.ends_with("/table") {
        "table"
    } else if uri.ends_with("/chart") {
        "chart"
    } else if uri.ends_with("/diagram") {
        "diagram"
    } else {
        "frame"
    }
}

fn shape_nodes(nodes: &[Node]) -> Vec<&Node> {
    let mut shapes = Vec::new();
    if let Some(tree) =
        child(nodes, &nodes[0], 1, "cSld").and_then(|cs| child(nodes, cs, 1, "spTree"))
    {
        direct_shapes(nodes, tree, &mut shapes);
    }
    shapes
}

pub(crate) fn shapes(xml: &str) -> Result<Vec<ShapeTransformInfo>> {
    let nodes = parse_nodes(xml)?;
    let refs = objects::References::new(&nodes);
    let infos: Vec<_> = shape_nodes(&nodes)
        .into_iter()
        .filter_map(|shape| {
            let mut info = transform_info(&nodes, shape)?;
            objects::capabilities(&nodes, &refs, shape, &mut info);
            Some(info)
        })
        .collect();
    let mut counts = std::collections::HashMap::new();
    for info in &infos {
        *counts.entry(info.id).or_insert(0) += 1;
    }
    Ok(infos
        .into_iter()
        .filter(|info| counts[&info.id] == 1)
        .collect())
}

/// Edits attributes by byte range, retaining all unrelated spelling and values.
fn edit_tag(tag: &str, values: &[(&str, String)]) -> Result<String> {
    edit_tag_attributes(tag, values, &[])
}
fn remove_tag_attributes(tag: &str, names: &[&str]) -> Result<String> {
    edit_tag_attributes(tag, &[], names)
}
fn edit_tag_attributes(tag: &str, values: &[(&str, String)], remove: &[&str]) -> Result<String> {
    let bytes = tag.as_bytes();
    let mut i = 1;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'/' | b'>') {
        i += 1;
    }
    let mut attributes = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || matches!(bytes[i], b'/' | b'>') {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = &tag[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            return Err(xml_error("attribute without equals"));
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *bytes
            .get(i)
            .ok_or_else(|| xml_error("attribute without value"))?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(xml_error("unquoted attribute"));
        }
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i == bytes.len() {
            return Err(xml_error("unclosed attribute"));
        }
        attributes.push((name, value_start..i, start..i + 1));
        i += 1;
    }
    let mut patches = Vec::new();
    let mut appended = String::new();
    for (name, _, range) in &attributes {
        if remove.contains(name) {
            patches.push((range.clone(), String::new()));
        }
    }
    for (name, value) in values {
        if let Some((_, range, _)) = attributes.iter().find(|(key, _, _)| key == name) {
            let old = quick_xml::escape::unescape(&tag[range.clone()]).map_err(xml_error)?;
            let equal = old.as_ref() == value
                || if matches!(*name, "flipH" | "flipV" | "b" | "i") {
                    matches!(old.as_ref(), "true" | "1") == matches!(value.as_str(), "true" | "1")
                } else if matches!(
                    *name,
                    "x" | "y"
                        | "cx"
                        | "cy"
                        | "rot"
                        | "sz"
                        | "val"
                        | "lvl"
                        | "marL"
                        | "indent"
                        | "startAt"
                ) {
                    old.parse::<i64>()
                        .ok()
                        .zip(value.parse::<i64>().ok())
                        .is_some_and(|(a, b)| a == b)
                } else {
                    false
                };
            if equal {
                continue;
            }
            patches.push((range.clone(), quick_xml::escape::escape(value).into_owned()));
        } else {
            appended.push_str(&format!(" {name}=\"{}\"", quick_xml::escape::escape(value)));
        }
    }
    if !appended.is_empty() {
        let end = if tag.ends_with("/>") {
            tag.len() - 2
        } else {
            tag.len() - 1
        };
        patches.push((end..end, appended));
    }
    Ok(apply_patches(tag, patches))
}

fn apply_patches(xml: &str, mut patches: Vec<(Range<usize>, String)>) -> String {
    patches.sort_by_key(|(range, _)| std::cmp::Reverse((range.start, range.end)));
    let mut result = xml.to_owned();
    for (range, replacement) in patches {
        result.replace_range(range, &replacement);
    }
    result
}

fn drawing_ns(strict: bool) -> &'static str {
    if strict {
        "http://purl.oclc.org/ooxml/drawingml/main"
    } else {
        "http://schemas.openxmlformats.org/drawingml/2006/main"
    }
}

fn coordinate_tag(name: &str, values: &[(&str, i64)], strict: bool) -> String {
    let attrs: String = values
        .iter()
        .map(|(key, value)| format!(" {key}=\"{value}\""))
        .collect();
    format!(
        "<jadeDraw:{name} xmlns:jadeDraw=\"{}\"{attrs}/>",
        drawing_ns(strict)
    )
}

fn transform_xml(
    xml: &str,
    nodes: &[Node],
    node: Option<&Node>,
    transform: Transform,
    strict: bool,
) -> Result<String> {
    let (x, y) = transform.offset_emu;
    let (cx, cy) = transform.extent_emu;
    let off = coordinate_tag("off", &[("x", x), ("y", y)], strict);
    let ext = coordinate_tag("ext", &[("cx", cx), ("cy", cy)], strict);
    let attrs = [
        ("rot", transform.rotation_60000ths.to_string()),
        ("flipH", u8::from(transform.flip_h).to_string()),
        ("flipV", u8::from(transform.flip_v).to_string()),
    ];
    let Some(node) = node else {
        return Ok(format!("<jadeDraw:xfrm xmlns:jadeDraw=\"{}\" rot=\"{}\" flipH=\"{}\" flipV=\"{}\">{off}{ext}</jadeDraw:xfrm>", drawing_ns(strict), attrs[0].1, attrs[1].1, attrs[2].1));
    };
    let mut updates = Vec::new();
    for (key, value) in attrs {
        if node.attr(key).is_some() || value != "0" {
            updates.push((key, value));
        }
    }
    let opening = edit_tag(&xml[node.opening.clone()], &updates)?;
    if node.empty {
        return Ok(format!(
            "{}>{off}{ext}</{}>",
            &opening[..opening.len() - 2],
            node.qualified
        ));
    }
    let base = node.outer.start;
    let local_range = |range: &Range<usize>| range.start - base..range.end - base;
    let mut patches = vec![(local_range(&node.opening), opening)];
    let old_off = child(nodes, node, 2, "off");
    let old_ext = child(nodes, node, 2, "ext");
    for (existing, values) in [
        (old_off, vec![("x", x.to_string()), ("y", y.to_string())]),
        (
            old_ext,
            vec![("cx", cx.to_string()), ("cy", cy.to_string())],
        ),
    ] {
        if let Some(existing) = existing {
            patches.push((
                local_range(&existing.opening),
                edit_tag(&xml[existing.opening.clone()], &values)?,
            ));
        }
    }
    match (old_off, old_ext) {
        (None, None) => patches.push((
            node.range.start - base..node.range.start - base,
            format!("{off}{ext}"),
        )),
        (None, Some(_)) => patches.push((node.range.start - base..node.range.start - base, off)),
        (Some(old), None) => patches.push((old.outer.end - base..old.outer.end - base, ext)),
        _ => {}
    }
    Ok(apply_patches(&xml[node.outer.clone()], patches))
}

pub(crate) fn set_transform(
    xml: &str,
    id: u32,
    transform: Transform,
    baseline: Option<Transform>,
) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let matches: Vec<_> = shape_nodes(&nodes)
        .into_iter()
        .filter_map(|node| {
            transform_info(&nodes, node)
                .filter(|info| info.id == id)
                .map(|info| (node, info))
        })
        .collect();
    if matches.len() != 1 {
        return Err(WriteError::NoShape(id));
    }
    let (shape, info) = &matches[0];
    let mut info = info.clone();
    let selectable = info.can_select;
    objects::capabilities(&nodes, &objects::References::new(&nodes), shape, &mut info);
    if selectable && !info.can_select {
        return Err(WriteError::NoShape(id));
    }
    info.transform = info.transform.or(baseline);
    crate::validate_shape_transform(&info, transform)?;
    if info.transform == Some(transform) {
        return Ok(xml.to_owned());
    }
    if shape.is(1, "grpSp") {
        let branches = objects::group_branches(&nodes, shape, id)
            .ok_or(WriteError::UnsupportedObjectOperation(id))?;
        let mut patches = Vec::new();
        for branch in branches {
            let info = transform_info(&nodes, branch).ok_or(WriteError::NoShape(id))?;
            crate::validate_shape_transform(&info, transform)?;
            let xfrm = child(&nodes, branch, 1, "grpSpPr")
                .and_then(|pr| child(&nodes, pr, 2, "xfrm"))
                .ok_or(WriteError::NoShape(id))?;
            patches.push((
                xfrm.outer.clone(),
                transform_xml(xml, &nodes, Some(xfrm), transform, xfrm.strict)?,
            ));
        }
        return Ok(apply_patches(xml, patches));
    }
    if shape.is(1, "graphicFrame") {
        let xfrm = child(&nodes, shape, 1, "xfrm").ok_or(WriteError::NoShape(id))?;
        let fragment = transform_xml(xml, &nodes, Some(xfrm), transform, xfrm.strict)?;
        return Ok(apply_patches(xml, vec![(xfrm.outer.clone(), fragment)]));
    }
    let sp_pr = child(&nodes, shape, 1, "spPr");
    let xfrm = sp_pr.and_then(|pr| child(&nodes, pr, 2, "xfrm"));
    let fragment = transform_xml(
        xml,
        &nodes,
        xfrm,
        transform,
        xfrm.map(|n| n.strict).unwrap_or(shape.strict),
    )?;
    let (range, fragment) = if let Some(xfrm) = xfrm {
        (xfrm.outer.clone(), fragment)
    } else if let Some(pr) = sp_pr {
        if pr.empty {
            (
                pr.outer.clone(),
                format!(
                    "{}{fragment}</{}>",
                    pr.empty_open.as_deref().unwrap_or_default(),
                    pr.qualified
                ),
            )
        } else {
            (pr.range.start..pr.range.start, fragment)
        }
    } else {
        let identity = shape
            .children
            .iter()
            .map(|&i| &nodes[i])
            .find(|n| n.ns == 1 && matches!(n.name.as_str(), "nvSpPr" | "nvPicPr" | "nvCxnSpPr"))
            .ok_or(WriteError::NoShape(id))?;
        let name = shape
            .qualified
            .rsplit_once(':')
            .map(|(prefix, _)| format!("{prefix}:spPr"))
            .unwrap_or_else(|| "spPr".into());
        (
            identity.outer.end..identity.outer.end,
            format!("<{name}>{fragment}</{name}>"),
        )
    };
    Ok(apply_patches(xml, vec![(range, fragment)]))
}

pub(crate) fn runs(xml: &str) -> Result<Vec<RunAnchor>> {
    let nodes = parse_nodes(xml)?;
    let root = &nodes[0];
    let mut result = Vec::new();
    if let Some(tree) = child(&nodes, root, 1, "cSld").and_then(|cs| child(&nodes, cs, 1, "spTree"))
    {
        shape_runs(&nodes, tree, &mut result);
    }
    Ok(result)
}

fn shape_runs(nodes: &[Node], parent: &Node, result: &mut Vec<RunAnchor>) {
    for &index in &parent.children {
        let shape = &nodes[index];
        if shape.is(1, "grpSp") || shape.is(3, "Fallback") {
            shape_runs(nodes, shape, result);
        } else if shape.is(3, "AlternateContent") {
            if let Some(fallback) = child(nodes, shape, 3, "Fallback") {
                shape_runs(nodes, fallback, result);
            }
        } else if shape.is(1, "sp") || shape.is(1, "cxnSp") {
            let nv =
                child(nodes, shape, 1, "nvSpPr").or_else(|| child(nodes, shape, 1, "nvCxnSpPr"));
            let Some(identity) = nv.and_then(|nv| child(nodes, nv, 1, "cNvPr")) else {
                continue;
            };
            let Some(shape_id) = identity.attr("id").and_then(|id| id.parse().ok()) else {
                continue;
            };
            let Some(body) = child(nodes, shape, 1, "txBody") else {
                continue;
            };
            for (paragraph, p) in body
                .children
                .iter()
                .map(|&i| &nodes[i])
                .filter(|p| p.is(2, "p"))
                .enumerate()
            {
                for (run, r) in p
                    .children
                    .iter()
                    .map(|&i| &nodes[i])
                    .filter(|r| r.is(2, "r"))
                    .enumerate()
                {
                    let texts: Vec<_> = r
                        .children
                        .iter()
                        .map(|&i| &nodes[i])
                        .filter(|t| t.is(2, "t"))
                        .collect();
                    if texts.len() != 1 {
                        continue;
                    }
                    let t = texts[0];
                    result.push(RunAnchor {
                        value: EditableRun {
                            address: RunAddress {
                                shape_id,
                                paragraph,
                                run,
                            },
                            shape_name: identity.attr("name").unwrap_or_default().to_owned(),
                            text: t.text.clone(),
                            locked: text::shape_locked(nodes, shape),
                        },
                        range: t.range.clone(),
                        empty_name: t.empty.then(|| t.qualified.clone()),
                        empty_open: t.empty.then(|| t.empty_open.clone().unwrap_or_default()),
                    });
                }
            }
        }
    }
}
