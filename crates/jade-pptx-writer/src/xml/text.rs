use super::{apply_patches, child, drawing_ns, parse_nodes, shape_nodes, transform_info, Node};
use crate::{Result, TextSnapshot, WriteError};
use std::collections::BTreeMap;

pub(super) fn body(nodes: &[Node], id: u32) -> Result<(&Node, &Node)> {
    let mut candidates = shape_nodes(nodes);
    fn groups<'a>(nodes: &'a [Node], parent: &Node, result: &mut Vec<&'a Node>) {
        for &index in &parent.children {
            let n = &nodes[index];
            if n.is(1, "grpSp") {
                result.extend(
                    n.children
                        .iter()
                        .map(|&i| &nodes[i])
                        .filter(|n| n.is(1, "sp") || n.is(1, "cxnSp")),
                );
                groups(nodes, n, result);
            } else if n.is(3, "AlternateContent") {
                if let Some(fallback) = child(nodes, n, 3, "Fallback") {
                    groups(nodes, fallback, result);
                }
            }
        }
    }
    if let Some(tree) =
        child(nodes, &nodes[0], 1, "cSld").and_then(|cs| child(nodes, cs, 1, "spTree"))
    {
        groups(nodes, tree, &mut candidates);
    }
    let shapes: Vec<_> = candidates
        .into_iter()
        .filter(|shape| transform_info(nodes, shape).is_some_and(|info| info.id == id))
        .collect();
    if shapes.len() != 1 {
        return Err(WriteError::NoShape(id));
    }
    let shape = shapes[0];
    Ok((
        shape,
        child(nodes, shape, 1, "txBody").ok_or(WriteError::NoTextBody(id))?,
    ))
}

pub(super) fn paragraphs<'a>(nodes: &'a [Node], body: &Node) -> Vec<&'a Node> {
    body.children
        .iter()
        .map(|&i| &nodes[i])
        .filter(|n| n.is(2, "p"))
        .collect()
}

pub(super) fn paragraph_text(nodes: &[Node], p: &Node) -> String {
    p.children
        .iter()
        .map(|&i| &nodes[i])
        .filter_map(|node| {
            if node.is(2, "r") || node.is(2, "fld") {
                Some(
                    child(nodes, node, 2, "t")
                        .map(|t| t.text.as_str())
                        .unwrap_or_default(),
                )
            } else if node.is(2, "br") {
                Some("\n")
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn shape_locked(nodes: &[Node], shape: &Node) -> bool {
    let mut node = Some(shape);
    while let Some(current) = node {
        let names = match (current.ns, current.name.as_str()) {
            (1, "sp") => Some(("nvSpPr", "cNvSpPr", "spLocks")),
            (1, "cxnSp") => Some(("nvCxnSpPr", "cNvCxnSpPr", "cxnSpLocks")),
            (1, "grpSp") => Some(("nvGrpSpPr", "cNvGrpSpPr", "grpSpLocks")),
            _ => None,
        };
        if let Some((nv, pr, locks)) = names {
            if let Some(locks) = child(nodes, current, 1, nv)
                .and_then(|n| child(nodes, n, 1, pr))
                .and_then(|n| child(nodes, n, 2, locks))
            {
                if ["noTextEdit", "noSelect"]
                    .iter()
                    .any(|name| locks.attr(name).is_some_and(|v| matches!(v, "1" | "true")))
                {
                    return true;
                }
            }
        }
        node = current.parent.map(|index| &nodes[index]);
    }
    false
}

pub(super) fn snapshot(nodes: &[Node], shape: &Node, body: &Node) -> TextSnapshot {
    let locked = shape_locked(nodes, shape);
    let paragraphs = paragraphs(nodes, body);
    let unsupported = paragraphs.iter().any(|p| {
        p.children
            .iter()
            .map(|&i| &nodes[i])
            .any(|n| n.ns != 2 || !matches!(n.name.as_str(), "pPr" | "endParaRPr" | "r" | "br"))
    });
    TextSnapshot {
        text: paragraphs
            .iter()
            .map(|p| paragraph_text(nodes, p))
            .collect::<Vec<_>>()
            .join("\n"),
        can_edit: !locked && !unsupported,
        locked,
    }
}

pub(crate) fn text_snapshot(xml: &str, id: u32) -> Result<TextSnapshot> {
    let nodes = parse_nodes(xml)?;
    let (shape, body) = body(&nodes, id)?;
    Ok(snapshot(&nodes, shape, body))
}

pub(super) fn byte_offset(text: &str, utf16: usize) -> Result<usize> {
    let mut offset = 0;
    for (byte, ch) in text.char_indices() {
        if offset == utf16 {
            return Ok(byte);
        }
        offset += ch.len_utf16();
        if offset > utf16 {
            return Err(WriteError::InvalidTextRange);
        }
    }
    if offset == utf16 {
        Ok(text.len())
    } else {
        Err(WriteError::InvalidTextRange)
    }
}

/// Moving a node between paragraphs must retain its original namespace scope,
/// including aliases declared on the paragraph or shape that held it.
pub(super) fn scoped(_xml: &str, nodes: &[Node], node: &Node, fragment: String) -> String {
    let mut scope = BTreeMap::new();
    scope.insert("xmlns".to_string(), String::new());
    let mut ancestors = Vec::new();
    let mut parent = node.parent;
    while let Some(index) = parent {
        ancestors.push(index);
        parent = nodes[index].parent;
    }
    for &index in ancestors.iter().rev() {
        for (name, value) in &nodes[index].attrs {
            if name == "xmlns" || name.starts_with("xmlns:") {
                scope.insert(name.clone(), value.clone());
            }
        }
    }
    for (name, _) in &node.attrs {
        scope.remove(name);
    }
    let declared: String = scope
        .into_iter()
        .map(|(key, value)| format!(" {key}=\"{}\"", quick_xml::escape::escape(&value)))
        .collect();
    // The opening tag itself can contain > in a quoted attribute.
    let mut reader = quick_xml::Reader::from_str(&fragment);
    let _ = reader.read_event();
    let length = reader.buffer_position() as usize;
    let end = if fragment[..length].ends_with("/>") {
        length - 2
    } else {
        length - 1
    };
    let mut fragment = fragment;
    fragment.insert_str(end, &declared);
    fragment
}

pub(super) fn partial_run(
    xml: &str,
    nodes: &[Node],
    run: &Node,
    text: &str,
    rehome: bool,
) -> Result<String> {
    let (range, replacement) = run_text_patch(nodes, run, text)?;
    let range = range.start - run.outer.start..range.end - run.outer.start;
    let result = apply_patches(&xml[run.outer.clone()], vec![(range, replacement)]);
    Ok(if rehome {
        scoped(xml, nodes, run, result)
    } else {
        result
    })
}

pub(super) fn run_text_patch(
    nodes: &[Node],
    run: &Node,
    text: &str,
) -> Result<(std::ops::Range<usize>, String)> {
    let t = child(nodes, run, 2, "t").ok_or_else(|| WriteError::Xml("run without text".into()))?;
    let escaped = quick_xml::escape::escape(text);
    let (range, replacement) = if t.empty {
        (
            t.outer.clone(),
            format!(
                "{}{escaped}</{}>",
                t.empty_open.as_deref().unwrap_or_default(),
                t.qualified
            ),
        )
    } else {
        (t.range.clone(), escaped.into_owned())
    };
    Ok((range, replacement))
}

struct Split {
    prefix: String,
    suffix: String,
    donor: Option<usize>,
}

fn split(xml: &str, nodes: &[Node], p: &Node, offset: usize, rehome: bool) -> Result<Split> {
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut position = 0;
    let mut found = false;
    let mut donor = None;
    for &index in &p.children {
        let node = &nodes[index];
        if node.is(2, "pPr") || node.is(2, "endParaRPr") {
            continue;
        }
        let text = if node.is(2, "r") {
            child(nodes, node, 2, "t")
                .map(|t| t.text.as_str())
                .unwrap_or_default()
        } else {
            "\n"
        };
        let length = text.encode_utf16().count();
        let fragment = xml[node.outer.clone()].to_string();
        let fragment = if rehome {
            scoped(xml, nodes, node, fragment)
        } else {
            fragment
        };
        if !found && offset >= position && offset <= position + length {
            let local = offset - position;
            if node.is(2, "r") {
                let byte = byte_offset(text, local)?;
                prefix.push_str(&partial_run(xml, nodes, node, &text[..byte], rehome)?);
                suffix.push_str(&partial_run(xml, nodes, node, &text[byte..], rehome)?);
                donor = Some(index);
            } else if local == 0 {
                suffix.push_str(&fragment);
                if child(nodes, node, 2, "rPr").is_some() {
                    donor = Some(index);
                }
            } else {
                prefix.push_str(&fragment);
                if child(nodes, node, 2, "rPr").is_some() {
                    donor = Some(index);
                }
            }
            found = true;
        } else if !found {
            prefix.push_str(&fragment);
            if node.is(2, "r") || child(nodes, node, 2, "rPr").is_some() {
                donor = Some(index);
            }
        } else {
            suffix.push_str(&fragment);
        }
        position += length;
    }
    if offset > position {
        return Err(WriteError::InvalidTextRange);
    }
    Ok(Split {
        prefix,
        suffix,
        donor,
    })
}

fn property(xml: &str, nodes: &[Node], p: &Node, name: &str, rehome: bool) -> String {
    let Some(node) = child(nodes, p, 2, name) else {
        return String::new();
    };
    let value = xml[node.outer.clone()].to_string();
    if rehome {
        scoped(xml, nodes, node, value)
    } else {
        value
    }
}

fn continuing_properties(xml: &str, nodes: &[Node], p: &Node) -> Result<String> {
    let Some(pr) = child(nodes, p, 2, "pPr") else {
        return Ok(String::new());
    };
    let mut fragment = xml[pr.outer.clone()].to_owned();
    if let Some(number) = child(nodes, pr, 2, "buAutoNum") {
        let opening = super::remove_tag_attributes(&xml[number.opening.clone()], &["startAt"])?;
        fragment = apply_patches(
            &fragment,
            vec![(
                number.opening.start - pr.outer.start..number.opening.end - pr.outer.start,
                opening,
            )],
        );
    }
    Ok(scoped(xml, nodes, pr, fragment))
}

fn insertion_props(xml: &str, nodes: &[Node], p: &Node, donor: Option<usize>) -> String {
    let props = donor
        .and_then(|i| child(nodes, &nodes[i], 2, "rPr"))
        .or_else(|| child(nodes, p, 2, "endParaRPr"));
    props
        .map(|node| {
            let mut raw = scoped(xml, nodes, node, xml[node.outer.clone()].to_string());
            if node.name == "endParaRPr" {
                let renamed = node.qualified.replace("endParaRPr", "rPr");
                if let Some(close) = raw.rfind(&format!("</{}", node.qualified)) {
                    raw.replace_range(close + 2..close + 2 + node.qualified.len(), &renamed);
                }
                raw.replace_range(1..1 + node.qualified.len(), &renamed);
                raw
            } else {
                raw
            }
        })
        .unwrap_or_default()
}

fn inserted_run(xml: &str, nodes: &[Node], p: &Node, donor: Option<usize>, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let props = insertion_props(xml, nodes, p, donor);
    format!(
        "<jadeDraw:r xmlns:jadeDraw=\"{}\">{props}<jadeDraw:t>{}</jadeDraw:t></jadeDraw:r>",
        drawing_ns(p.strict),
        quick_xml::escape::escape(text)
    )
}

pub(crate) fn replace_text_range(
    xml: &str,
    id: u32,
    start: usize,
    end: usize,
    text: &str,
    soft_break: bool,
) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (shape, body) = body(&nodes, id)?;
    let state = snapshot(&nodes, shape, body);
    if state.locked {
        return Err(WriteError::TextLocked(id));
    }
    if !state.can_edit {
        return Err(WriteError::UnsupportedTextContent(id));
    }
    if start > end {
        return Err(WriteError::InvalidTextRange);
    }
    let start_byte = byte_offset(&state.text, start)?;
    let end_byte = byte_offset(&state.text, end)?;
    if !text.contains('\n') && &state.text[start_byte..end_byte] == text {
        return Ok(xml.to_owned());
    }
    if start == end && text.is_empty() {
        return Ok(xml.to_owned());
    }
    let paragraphs = paragraphs(&nodes, body);
    if paragraphs.is_empty() {
        return Err(WriteError::NoTextBody(id));
    }
    let mut bases = Vec::new();
    let mut length = 0;
    for p in &paragraphs {
        bases.push(length);
        length += paragraph_text(&nodes, p).encode_utf16().count() + 1;
    }
    let locate = |offset| {
        bases
            .iter()
            .enumerate()
            .rev()
            .find(|(_, base)| **base <= offset)
            .map(|(index, base)| (index, offset - base))
            .ok_or(WriteError::InvalidTextRange)
    };
    let (first, first_offset) = locate(start)?;
    let (last, last_offset) = locate(end)?;
    let first_p = paragraphs[first];
    let last_p = paragraphs[last];
    let left = split(xml, &nodes, first_p, first_offset, false)?;
    // A replacement inherits its first selected character, rather than a
    // differently styled run ending at the same insertion boundary.
    let mut donor = left.donor;
    if start < end {
        let mut at = 0;
        for &index in &first_p.children {
            let node = &nodes[index];
            let length = if node.is(2, "r") {
                child(&nodes, node, 2, "t")
                    .map(|t| t.text.encode_utf16().count())
                    .unwrap_or(0)
            } else if node.is(2, "br") {
                1
            } else {
                continue;
            };
            if at <= first_offset && first_offset < at + length {
                donor = Some(index);
                break;
            }
            at += length;
        }
    }
    let right = split(xml, &nodes, last_p, last_offset, true)?;
    let chunks: Vec<_> = if soft_break {
        vec![text]
    } else {
        text.split('\n').collect()
    };
    let mut result = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let first = index == 0;
        let last = index + 1 == chunks.len();
        if first {
            let opening = &xml[first_p.opening.clone()];
            result.push_str(if first_p.empty {
                &opening[..opening.len() - 2]
            } else {
                opening
            });
            if first_p.empty {
                result.push('>');
            }
            result.push_str(&property(xml, &nodes, first_p, "pPr", false));
            result.push_str(&left.prefix);
        } else {
            result.push_str(&format!(
                "<jadeDraw:p xmlns:jadeDraw=\"{}\">",
                drawing_ns(first_p.strict)
            ));
            result.push_str(&continuing_properties(xml, &nodes, first_p)?);
        }
        if soft_break {
            for (j, segment) in chunk.split('\n').enumerate() {
                if j > 0 {
                    let props = insertion_props(xml, &nodes, first_p, donor);
                    result.push_str(&format!(
                        "<jadeDraw:br xmlns:jadeDraw=\"{}\">{props}</jadeDraw:br>",
                        drawing_ns(first_p.strict)
                    ));
                }
                result.push_str(&inserted_run(xml, &nodes, first_p, donor, segment));
            }
        } else {
            result.push_str(&inserted_run(xml, &nodes, first_p, donor, chunk));
        }
        if last {
            result.push_str(&right.suffix);
            result.push_str(&property(xml, &nodes, last_p, "endParaRPr", true));
        } else {
            result.push_str(&property(xml, &nodes, first_p, "endParaRPr", !first));
        }
        result.push_str(&format!(
            "</{}>",
            if first {
                first_p.qualified.as_str()
            } else {
                "jadeDraw:p"
            }
        ));
    }
    Ok(apply_patches(
        xml,
        vec![(first_p.outer.start..last_p.outer.end, result)],
    ))
}
