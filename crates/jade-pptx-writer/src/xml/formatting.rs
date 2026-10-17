use super::{apply_patches, child, drawing_ns, edit_tag, parse_nodes, text, Node};
use crate::{Result, TextFormat, TextFormatEdit, WriteError};
use std::ops::Range;

fn values(format: &TextFormat) -> Vec<(&str, String)> {
    let mut values = Vec::new();
    for (name, value) in [("b", format.bold), ("i", format.italic)] {
        if let Some(value) = value {
            values.push((name, u8::from(value).to_string()));
        }
    }
    if let Some(value) = format.underline {
        values.push(("u", if value { "sng" } else { "none" }.into()));
    }
    if let Some(value) = format.size_pt {
        values.push(("sz", ((value * 100.0).round() as u32).to_string()));
    }
    values
}

// CT_TextCharacterProperties child sequence. Unknown children are retained verbatim.
fn rank(node: &Node) -> Option<u8> {
    if node.ns != 2 {
        return None;
    }
    Some(match node.name.as_str() {
        "ln" => 0,
        "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill" => 1,
        "effectLst" | "effectDag" => 2,
        "highlight" => 3,
        "uLnTx" | "uLn" => 4,
        "uFillTx" | "uFill" => 5,
        "latin" => 6,
        "ea" => 7,
        "cs" => 8,
        "sym" => 9,
        "hlinkClick" => 10,
        "hlinkMouseOver" => 11,
        "rtl" => 12,
        "extLst" => 13,
        _ => return None,
    })
}

fn property_patch(
    xml: &str,
    nodes: &[Node],
    parent: &Node,
    name: &str,
    format: &TextFormat,
) -> Result<(Range<usize>, String)> {
    let values = values(format);
    let pr = child(nodes, parent, 2, name);
    let ns = drawing_ns(parent.strict);
    let mut additions = std::collections::BTreeMap::<usize, String>::new();
    let mut patches = Vec::new();
    let mut append = |order: u8, fragment: String| {
        let at = pr
            .map(|pr| {
                pr.children
                    .iter()
                    .map(|i| &nodes[*i])
                    .find(|n| rank(n).is_some_and(|r| r > order))
                    .map_or(pr.range.end, |n| n.outer.start)
            })
            .unwrap_or(0);
        additions.entry(at).or_default().push_str(&fragment);
    };
    if let Some(color) = &format.color {
        let fills: Vec<_> = pr
            .into_iter()
            .flat_map(|pr| pr.children.iter())
            .map(|i| &nodes[*i])
            .filter(|n| rank(n) == Some(1))
            .collect();
        let rgb = color[1..].to_ascii_uppercase();
        let same = fills.len() == 1
            && fills[0].is(2, "solidFill")
            && fills[0].children.len() == 1
            && child(nodes, fills[0], 2, "srgbClr").is_some_and(|c| {
                c.children.is_empty() && c.attr("val").is_some_and(|v| v.eq_ignore_ascii_case(&rgb))
            });
        if !same {
            let fragment = format!("<jadeFmt:solidFill xmlns:jadeFmt=\"{ns}\"><jadeFmt:srgbClr val=\"{rgb}\"/></jadeFmt:solidFill>");
            if let Some(first) = fills.first() {
                patches.push((first.outer.clone(), fragment));
                for extra in fills.iter().skip(1) {
                    patches.push((extra.outer.clone(), String::new()));
                }
            } else {
                append(1, fragment);
            }
        }
    }
    for (name, face, order) in [
        ("latin", &format.font, 6),
        ("ea", &format.east_asian_font, 7),
    ] {
        if let Some(face) = face {
            if let Some(font) = pr.and_then(|pr| child(nodes, pr, 2, name)) {
                patches.push((
                    font.opening.clone(),
                    edit_tag(&xml[font.opening.clone()], &[("typeface", face.clone())])?,
                ));
            } else {
                append(
                    order,
                    format!(
                        "<jadeFmt:{name} xmlns:jadeFmt=\"{ns}\" typeface=\"{}\"/>",
                        quick_xml::escape::escape(face)
                    ),
                );
            }
        }
    }
    if let Some(pr) = pr {
        let opening = edit_tag(&xml[pr.opening.clone()], &values)?;
        if pr.empty && !additions.is_empty() {
            let open = opening.strip_suffix("/>").expect("empty property");
            return Ok((
                pr.outer.clone(),
                format!(
                    "{open}>{}</{}>",
                    additions.into_values().collect::<String>(),
                    pr.qualified
                ),
            ));
        }
        patches.push((pr.opening.clone(), opening));
        patches.extend(
            additions
                .into_iter()
                .map(|(at, fragment)| (at..at, fragment)),
        );
        let patches = patches
            .into_iter()
            .map(|(r, s)| (r.start - pr.outer.start..r.end - pr.outer.start, s))
            .collect();
        return Ok((
            pr.outer.clone(),
            apply_patches(&xml[pr.outer.clone()], patches),
        ));
    }
    let attrs: String = values
        .iter()
        .map(|(name, value)| format!(" {name}=\"{value}\""))
        .collect();
    let children = additions.into_values().collect::<String>();
    let fragment = if children.is_empty() {
        format!("<jadeFmt:{name} xmlns:jadeFmt=\"{ns}\"{attrs}/>")
    } else {
        format!("<jadeFmt:{name} xmlns:jadeFmt=\"{ns}\"{attrs}>{children}</jadeFmt:{name}>")
    };
    if parent.empty {
        return Ok((
            parent.outer.clone(),
            format!(
                "{}{fragment}</{}>",
                parent.empty_open.as_deref().unwrap_or_default(),
                parent.qualified
            ),
        ));
    }
    let at = if name == "endParaRPr" {
        parent.range.end
    } else {
        parent.range.start
    };
    Ok((at..at, fragment))
}

fn formatted_node(xml: &str, nodes: &[Node], node: &Node, format: &TextFormat) -> Result<String> {
    let (range, fragment) = property_patch(xml, nodes, node, "rPr", format)?;
    Ok(apply_patches(
        &xml[node.outer.clone()],
        vec![(
            range.start - node.outer.start..range.end - node.outer.start,
            fragment,
        )],
    ))
}

pub(crate) fn format_text_range(xml: &str, edit: &TextFormatEdit) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (shape, body) = text::body(&nodes, edit.shape_id)?;
    let state = text::snapshot(&nodes, shape, body);
    if state.locked {
        return Err(WriteError::TextLocked(edit.shape_id));
    }
    if !state.can_edit {
        return Err(WriteError::UnsupportedTextContent(edit.shape_id));
    }
    if edit.start > edit.end {
        return Err(WriteError::InvalidTextRange);
    }
    text::byte_offset(&state.text, edit.start)?;
    text::byte_offset(&state.text, edit.end)?;
    if edit.format.is_empty() {
        return Ok(xml.to_owned());
    }
    let paragraphs = text::paragraphs(&nodes, body);
    let mut base = 0;
    let mut patches = Vec::new();
    for (p_index, p) in paragraphs.iter().enumerate() {
        let length = text::paragraph_text(&nodes, p).encode_utf16().count();
        let mut position = base;
        for &index in &p.children {
            let run = &nodes[index];
            let plain = if run.is(2, "r") {
                child(&nodes, run, 2, "t")
                    .map(|t| t.text.as_str())
                    .unwrap_or_default()
            } else if run.is(2, "br") {
                "\n"
            } else {
                continue;
            };
            let end = position + plain.encode_utf16().count();
            if edit.start < edit.end && edit.start < end && edit.end > position {
                let updated = formatted_node(xml, &nodes, run, &edit.format)?;
                // Reapplying an explicit property should not split a run or dirty the archive.
                if updated != xml[run.outer.clone()] {
                    if run.is(2, "r") {
                        let from = text::byte_offset(plain, edit.start.saturating_sub(position))?;
                        let to =
                            text::byte_offset(plain, (edit.end - position).min(end - position))?;
                        let mut fragment = String::new();
                        if from > 0 {
                            fragment.push_str(&text::partial_run(
                                xml,
                                &nodes,
                                run,
                                &plain[..from],
                                false,
                            )?);
                        }
                        let run_patches = [
                            text::run_text_patch(&nodes, run, &plain[from..to])?,
                            property_patch(xml, &nodes, run, "rPr", &edit.format)?,
                        ]
                        .into_iter()
                        .map(|(range, value)| {
                            (
                                range.start - run.outer.start..range.end - run.outer.start,
                                value,
                            )
                        })
                        .collect();
                        fragment.push_str(&apply_patches(&xml[run.outer.clone()], run_patches));
                        if to < plain.len() {
                            fragment.push_str(&text::partial_run(
                                xml,
                                &nodes,
                                run,
                                &plain[to..],
                                false,
                            )?);
                        }
                        patches.push((run.outer.clone(), fragment));
                    } else {
                        patches.push((run.outer.clone(), updated));
                    }
                }
            }
            position = end;
        }
        let mark = base + length;
        let selected_mark = p_index + 1 < paragraphs.len() && edit.start <= mark && edit.end > mark;
        let selected_empty = length == 0 && edit.start <= base && edit.end >= base;
        if selected_mark || selected_empty {
            patches.push(property_patch(xml, &nodes, p, "endParaRPr", &edit.format)?);
        }
        base = mark + 1;
    }
    Ok(apply_patches(xml, patches))
}
