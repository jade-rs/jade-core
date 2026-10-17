//! Reset only placeholder presentation properties; content and identities survive.
use super::{child, match_slot, placeholders};
use crate::xml::{apply_patches, edit_tag_attributes, objects, parse_document_nodes, text, Node};
use crate::Result;
use std::{collections::HashSet, ops::Range};

const BODY_ATTRS: &[&str] = &[
    "rot",
    "spcFirstLastPara",
    "vertOverflow",
    "horzOverflow",
    "vert",
    "wrap",
    "lIns",
    "tIns",
    "rIns",
    "bIns",
    "numCol",
    "spcCol",
    "rtlCol",
    "fromWordArt",
    "anchor",
    "anchorCtr",
    "forceAA",
    "upright",
    "compatLnSpc",
];
const PARA_ATTRS: &[&str] = &[
    "marL",
    "marR",
    "indent",
    "algn",
    "defTabSz",
    "rtl",
    "eaLnBrk",
    "fontAlgn",
    "latinLnBrk",
    "hangingPunct",
];
const RUN_ATTRS: &[&str] = &[
    "kumimoji",
    "sz",
    "b",
    "i",
    "u",
    "strike",
    "kern",
    "cap",
    "spc",
    "normalizeH",
    "baseline",
    "noProof",
];
const RUN_CHILDREN: &[&str] = &[
    "ln",
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
    "effectLst",
    "effectDag",
    "highlight",
    "uLnTx",
    "uLn",
    "uFillTx",
    "uFill",
    "latin",
    "ea",
    "cs",
    "sym",
];
const PARA_CHILDREN: &[&str] = &[
    "lnSpc",
    "spcBef",
    "spcAft",
    "buClrTx",
    "buClr",
    "buSzTx",
    "buSzPct",
    "buSzPts",
    "buFontTx",
    "buFont",
    "buNone",
    "buAutoNum",
    "buChar",
    "buBlip",
    "tabLst",
];
const BODY_CHILDREN: &[&str] = &[
    "prstTxWarp",
    "noAutofit",
    "normAutofit",
    "spAutoFit",
    "scene3d",
    "sp3d",
    "flatTx",
];
const SHAPE_CHILDREN: &[&str] = &[
    "xfrm",
    "prstGeom",
    "custGeom",
    "noFill",
    "solidFill",
    "gradFill",
    "pattFill",
    "grpFill",
    "ln",
    "effectLst",
    "effectDag",
    "scene3d",
    "sp3d",
];
const LIST_CHILDREN: &[&str] = &[
    "defPPr", "lvl1pPr", "lvl2pPr", "lvl3pPr", "lvl4pPr", "lvl5pPr", "lvl6pPr", "lvl7pPr",
    "lvl8pPr", "lvl9pPr",
];

fn safe_property(nodes: &[Node], property: &Node, copied: bool) -> Result<()> {
    for n in objects::descendants(nodes, property) {
        if n.ns != 2
            || n.name == "extLst"
            || n.attrs
                .iter()
                .any(|(name, _)| name.contains(':') && !name.starts_with("xmlns:"))
            || (copied && !crate::xml::slides::relationship_attrs(nodes, n).is_empty())
        {
            return Err(crate::xml::xml_error(
                "placeholder formatting has unsupported extensions or part-relative references",
            ));
        }
    }
    Ok(())
}
fn scoped(xml: &str, nodes: &[Node], n: &Node) -> Result<String> {
    safe_property(nodes, n, true)?;
    Ok(text::scoped(xml, nodes, n, xml[n.outer.clone()].into()))
}
fn property_rank(parent: &Node, name: &str) -> usize {
    let groups: &[&[&str]] = match parent.name.as_str() {
        "spPr" => &[
            &["xfrm"],
            &["prstGeom", "custGeom"],
            &[
                "noFill",
                "solidFill",
                "gradFill",
                "blipFill",
                "pattFill",
                "grpFill",
            ],
            &["ln"],
            &["effectLst", "effectDag"],
            &["scene3d"],
            &["sp3d"],
        ],
        "bodyPr" => &[
            &["prstTxWarp"],
            &["noAutofit", "normAutofit", "spAutoFit"],
            &["scene3d"],
            &["sp3d", "flatTx"],
        ],
        "style" => &[&["lnRef"], &["fillRef"], &["effectRef"], &["fontRef"]],
        _ => {
            return LIST_CHILDREN
                .iter()
                .position(|n| *n == name)
                .unwrap_or(usize::MAX)
        }
    };
    groups
        .iter()
        .position(|g| g.contains(&name))
        .unwrap_or(usize::MAX)
}
// Replace known formatting only. Preserve the source's unknown attributes/children.
fn property(
    xml: &str,
    nodes: &[Node],
    n: &Node,
    template: Option<(&str, &[Node], &Node)>,
    attrs: &[&str],
    children: &[&str],
) -> Result<(Range<usize>, String)> {
    let mut patches = Vec::new();
    let values: Vec<_> = template
        .into_iter()
        .flat_map(|(_, _, t)| {
            attrs
                .iter()
                .filter_map(move |a| t.attr(a).map(|v| (*a, v.to_owned())))
        })
        .collect();
    let absent: Vec<_> = attrs
        .iter()
        .copied()
        .filter(|name| !values.iter().any(|(key, _)| key == name))
        .collect();
    let opening = edit_tag_attributes(&xml[n.opening.clone()], &values, &absent)?;
    patches.push((n.opening.clone(), opening.clone()));
    for old in n
        .children
        .iter()
        .map(|i| &nodes[*i])
        .filter(|c| c.ns == 2 && children.contains(&c.name.as_str()))
    {
        safe_property(nodes, old, false)?;
        patches.push((old.outer.clone(), String::new()));
    }
    let mut additions = std::collections::BTreeMap::<usize, String>::new();
    if let Some((s, ns, t)) = template {
        for c in t.children.iter().map(|i| &ns[*i]).filter(|c| {
            c.ns == 2
                && children.contains(&c.name.as_str())
                && !(n.is(1, "spPr") && c.is(2, "xfrm"))
        }) {
            let rank = property_rank(n, &c.name);
            let at = n
                .children
                .iter()
                .map(|i| &nodes[*i])
                .find(|old| property_rank(n, &old.name) > rank)
                .map_or(n.range.end, |old| old.outer.start);
            additions
                .entry(at)
                .or_default()
                .push_str(&scoped(s, ns, c)?);
        }
    }
    if n.empty && !additions.is_empty() {
        return Ok((
            n.outer.clone(),
            format!(
                "{}>{}</{}>",
                opening.strip_suffix("/>").expect("empty node"),
                additions.into_values().collect::<String>(),
                n.qualified
            ),
        ));
    }
    patches.extend(additions.into_iter().map(|(at, value)| (at..at, value)));
    if n.is(2, "pPr") {
        if let Some(d) = child(nodes, n, 2, "defRPr") {
            patches.push(run(xml, nodes, d)?);
        }
    }
    let local = patches
        .into_iter()
        .map(|(r, s)| (r.start - n.outer.start..r.end - n.outer.start, s))
        .collect();
    Ok((n.outer.clone(), apply_patches(&xml[n.outer.clone()], local)))
}
fn run(xml: &str, nodes: &[Node], n: &Node) -> Result<(Range<usize>, String)> {
    property(xml, nodes, n, None, RUN_ATTRS, RUN_CHILDREN)
}

pub(super) fn reset(xml: &str, target: &str) -> Result<String> {
    let nodes = parse_document_nodes(xml)?;
    let templates = parse_document_nodes(target)?;
    let sources = placeholders(&nodes, true)?;
    let targets = placeholders(&templates, true)?;
    let mut used = HashSet::new();
    let mut patches = Vec::new();
    for ph in sources {
        let Some(i) = match_slot(&ph, &targets, &used)? else {
            continue;
        };
        used.insert(i);
        if text::shape_locked(&nodes, ph.shape) {
            return Err(crate::xml::xml_error("placeholder is locked"));
        }
        let template = targets[i].shape;
        let pr = child(&nodes, ph.shape, 1, "spPr")
            .ok_or_else(|| crate::xml::xml_error("placeholder has no shape properties"))?;
        let target_pr = child(&templates, template, 1, "spPr");
        // The inherited native transform remains authoritative after local xfrm removal.
        let visual: Vec<_> = SHAPE_CHILDREN
            .iter()
            .copied()
            .filter(|c| *c != "xfrm")
            .collect();
        let has_image = child(&nodes, pr, 2, "blipFill").is_some();
        let visual: Vec<_> = visual
            .into_iter()
            .filter(|c| {
                !has_image
                    || !matches!(
                        *c,
                        "noFill" | "solidFill" | "gradFill" | "pattFill" | "grpFill"
                    )
            })
            .collect();
        let mut names = visual.clone();
        names.push("xfrm");
        // Reset geometry/style to the layout, but leave image payload and crop untouched.
        patches.push(property(
            xml,
            &nodes,
            pr,
            target_pr.map(|t| (target, templates.as_slice(), t)),
            &["bwMode"],
            &names,
        )?);
        let style = child(&nodes, ph.shape, 1, "style");
        let target_style = child(&templates, template, 1, "style");
        match (style, target_style) {
            (Some(s), None) => {
                if s.attrs.iter().any(|(name, _)| !name.starts_with("xmlns")) {
                    return Err(crate::xml::xml_error(
                        "placeholder style has unsupported attributes",
                    ));
                }
                for c in s.children.iter().map(|i| &nodes[*i]) {
                    safe_property(&nodes, c, false)?;
                }
                patches.push((s.outer.clone(), String::new()));
            }
            (Some(s), t) => patches.push(property(
                xml,
                &nodes,
                s,
                t.map(|t| (target, templates.as_slice(), t)),
                &[],
                &["lnRef", "fillRef", "effectRef", "fontRef"],
            )?),
            (None, Some(t)) => {
                let at = ph
                    .shape
                    .children
                    .iter()
                    .map(|i| &nodes[*i])
                    .find(|n| n.is(1, "txBody") || n.is(1, "extLst"))
                    .map_or(ph.shape.range.end, |n| n.outer.start);
                let value = text::scoped(target, &templates, t, target[t.outer.clone()].into());
                for c in t.children.iter().map(|i| &templates[*i]) {
                    safe_property(&templates, c, true)?;
                }
                patches.push((at..at, value));
            }
            _ => {}
        }
        let Some(body) = child(&nodes, ph.shape, 1, "txBody") else {
            continue;
        };
        let target_body = child(&templates, template, 1, "txBody");
        for (name, attrs, children) in [
            ("bodyPr", BODY_ATTRS, BODY_CHILDREN),
            ("lstStyle", &[][..], LIST_CHILDREN),
        ] {
            if let Some(n) = child(&nodes, body, 2, name) {
                let t = target_body.and_then(|b| child(&templates, b, 2, name));
                patches.push(property(
                    xml,
                    &nodes,
                    n,
                    t.map(|t| (target, templates.as_slice(), t)),
                    attrs,
                    children,
                )?);
            }
        }
        for p in body
            .children
            .iter()
            .map(|i| &nodes[*i])
            .filter(|p| p.is(2, "p"))
        {
            if let Some(pr) = child(&nodes, p, 2, "pPr") {
                // Outline levels and hyperlinks describe content and survive reset.
                patches.push(property(xml, &nodes, pr, None, PARA_ATTRS, PARA_CHILDREN)?);
            }
            for n in p.children.iter().map(|i| &nodes[*i]) {
                if n.is(2, "endParaRPr") {
                    patches.push(run(xml, &nodes, n)?);
                } else if n.is(2, "r") || n.is(2, "br") || n.is(2, "fld") {
                    if let Some(r) = child(&nodes, n, 2, "rPr") {
                        patches.push(run(xml, &nodes, r)?);
                    }
                }
            }
        }
    }
    Ok(apply_patches(xml, patches))
}
