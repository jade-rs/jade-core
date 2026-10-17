use super::{
    apply_patches, child, drawing_ns, edit_tag, parse_nodes, remove_tag_attributes, text, Node,
};
use crate::{ParagraphAlignment, ParagraphBullet, ParagraphFormatEdit, Result, WriteError};
use std::collections::BTreeMap;

fn rank(node: &Node) -> Option<u8> {
    if node.ns != 2 {
        return None;
    }
    Some(match node.name.as_str() {
        "lnSpc" => 0,
        "spcBef" => 1,
        "spcAft" => 2,
        "buClr" | "buClrTx" => 3,
        "buSzPct" | "buSzPts" | "buSzTx" => 4,
        "buFont" | "buFontTx" => 5,
        "buNone" | "buAutoNum" | "buChar" | "buBlip" => 6,
        "tabLst" => 7,
        "defRPr" => 8,
        "extLst" => 9,
        _ => return None,
    })
}

fn format_paragraph(
    xml: &str,
    nodes: &[Node],
    p: &Node,
    edit: &ParagraphFormatEdit,
    first: bool,
) -> Result<(std::ops::Range<usize>, String)> {
    let format = &edit.format;
    let pr = child(nodes, p, 2, "pPr");
    let ns = drawing_ns(p.strict);
    let mut values = Vec::new();
    if let Some(align) = format.alignment {
        values.push((
            "algn",
            match align {
                ParagraphAlignment::Left => "l",
                ParagraphAlignment::Center => "ctr",
                ParagraphAlignment::Right => "r",
                ParagraphAlignment::Justify => "just",
                ParagraphAlignment::Distributed => "dist",
            }
            .into(),
        ));
    }
    if let Some(level) = format.level {
        values.push(("lvl", level.to_string()));
    }
    for (name, value) in [
        ("marL", format.margin_left_pt),
        ("marR", format.margin_right_pt),
        ("indent", format.indent_pt),
    ] {
        if let Some(value) = value {
            values.push((name, (value * 12700.0).round().to_string()));
        }
    }
    let mut patches = Vec::new();
    let mut additions = BTreeMap::<usize, String>::new();
    let mut add = |order, fragment: String| {
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
    for (name, spacing, order) in [
        ("lnSpc", format.line_spacing, 0),
        ("spcBef", format.space_before, 1),
        ("spcAft", format.space_after, 2),
    ] {
        if let Some(spacing) = spacing {
            let (kind, value) = spacing.units();
            let fragment = format!("<jadePara:{kind} xmlns:jadePara=\"{ns}\" val=\"{value}\"/>");
            if let Some(container) = pr.and_then(|pr| child(nodes, pr, 2, name)) {
                if let Some(old) = container
                    .children
                    .iter()
                    .map(|i| &nodes[*i])
                    .find(|n| n.is(2, "spcPct") || n.is(2, "spcPts"))
                {
                    if old.is(2, kind) {
                        patches.push((
                            old.opening.clone(),
                            edit_tag(&xml[old.opening.clone()], &[("val", value.to_string())])?,
                        ));
                    } else {
                        patches.push((old.outer.clone(), fragment));
                    }
                } else if container.empty {
                    patches.push((
                        container.outer.clone(),
                        format!(
                            "{}{fragment}</{}>",
                            container.empty_open.as_deref().unwrap_or_default(),
                            container.qualified
                        ),
                    ));
                } else {
                    patches.push((container.range.start..container.range.start, fragment));
                }
            } else {
                add(
                    order,
                    format!(
                        "<jadePara:{name} xmlns:jadePara=\"{ns}\">{fragment}</jadePara:{name}>"
                    ),
                );
            }
        }
    }
    if let Some(bullet) = &format.bullet {
        // Changing list kind retains bullet color/size while explicitly choosing its font.
        let (name, attrs, face) = match bullet {
            ParagraphBullet::None => ("buNone", Vec::new(), None),
            ParagraphBullet::Character { character, font } => (
                "buChar",
                vec![("char", character.clone())],
                Some(font.as_deref()),
            ),
            ParagraphBullet::Numbered { scheme, start_at } => {
                let mut attrs = vec![("type", scheme.clone())];
                if first {
                    if let Some(start) = start_at {
                        attrs.push(("startAt", start.to_string()));
                    }
                }
                ("buAutoNum", attrs, Some(None))
            }
        };
        let mut groups = vec![(6, name, attrs)];
        if let Some(face) = face {
            groups.insert(
                0,
                if let Some(face) = face {
                    (5, "buFont", vec![("typeface", face.to_owned())])
                } else {
                    (5, "buFontTx", Vec::new())
                },
            );
        }
        for (order, name, attrs) in groups {
            let old: Vec<_> = pr
                .into_iter()
                .flat_map(|pr| &pr.children)
                .map(|i| &nodes[*i])
                .filter(|n| rank(n) == Some(order))
                .collect();
            let fragment = edit_tag(
                &format!("<jadePara:{name} xmlns:jadePara=\"{ns}\"/>"),
                &attrs,
            )?;
            if let Some(node) = old.first() {
                if node.is(2, name) {
                    let mut opening = edit_tag(&xml[node.opening.clone()], &attrs)?;
                    if name == "buAutoNum" && !attrs.iter().any(|(k, _)| *k == "startAt") {
                        opening = remove_tag_attributes(&opening, &["startAt"])?;
                    }
                    patches.push((node.opening.clone(), opening));
                } else {
                    patches.push((node.outer.clone(), fragment));
                }
                for extra in old.iter().skip(1) {
                    patches.push((extra.outer.clone(), String::new()));
                }
            } else {
                add(order, fragment);
            }
        }
    }
    if let Some(pr) = pr {
        let opening = edit_tag(&xml[pr.opening.clone()], &values)?;
        if pr.empty && !additions.is_empty() {
            return Ok((
                pr.outer.clone(),
                format!(
                    "{}>{}</{}>",
                    opening.strip_suffix("/>").expect("empty properties"),
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
    let opening = edit_tag(&format!("<jadePara:pPr xmlns:jadePara=\"{ns}\"/>"), &values)?;
    let children = additions.into_values().collect::<String>();
    let fragment = if children.is_empty() {
        opening
    } else {
        format!(
            "{}>{children}</jadePara:pPr>",
            opening.strip_suffix("/>").expect("new properties")
        )
    };
    if p.empty {
        return Ok((
            p.outer.clone(),
            format!(
                "{}{fragment}</{}>",
                p.empty_open.as_deref().unwrap_or_default(),
                p.qualified
            ),
        ));
    }
    Ok((p.range.start..p.range.start, fragment))
}

pub(crate) fn format_paragraphs(xml: &str, edit: &ParagraphFormatEdit) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (shape, body) = text::body(&nodes, edit.shape_id)?;
    let state = text::snapshot(&nodes, shape, body);
    if state.locked {
        return Err(WriteError::TextLocked(edit.shape_id));
    }
    if !state.can_edit {
        return Err(WriteError::UnsupportedTextContent(edit.shape_id));
    }
    text::byte_offset(&state.text, edit.start)?;
    text::byte_offset(&state.text, edit.end)?;
    let paragraphs = text::paragraphs(&nodes, body);
    let lengths: Vec<_> = paragraphs
        .iter()
        .map(|p| text::paragraph_text(&nodes, p).encode_utf16().count())
        .collect();
    let indices = jade_pptx_layout::paragraph_indices_for_selection(&lengths, edit.start, edit.end)
        .ok_or(WriteError::InvalidTextRange)?;
    if edit.format.is_empty() {
        return Ok(xml.to_owned());
    }
    let mut patches = Vec::new();
    for (at, index) in indices.into_iter().enumerate() {
        patches.push(format_paragraph(
            xml,
            &nodes,
            paragraphs[index],
            edit,
            at == 0,
        )?);
    }
    Ok(apply_patches(xml, patches))
}
