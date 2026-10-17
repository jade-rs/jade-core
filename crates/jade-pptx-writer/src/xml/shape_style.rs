use super::{apply_patches, child, crop, drawing_ns, edit_tag, objects, parse_nodes, Node};
use crate::{Result, WriteError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

/// Omitted properties keep the original fill, outline, effects and theme references.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeStyleEdit {
    pub fill: Option<String>,
    pub outline: Option<String>,
    pub outline_width_pt: Option<f64>,
}
impl ShapeStyleEdit {
    pub fn validate(&self) -> Result<()> {
        for value in [&self.fill, &self.outline].into_iter().flatten() {
            if value != "none"
                && !(value.len() == 7
                    && value.starts_with('#')
                    && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
            {
                return Err(WriteError::InvalidShapeStyle);
            }
        }
        if self
            .outline_width_pt
            .is_some_and(|w| !w.is_finite() || !(0.0..=1584.0).contains(&w))
        {
            return Err(WriteError::InvalidShapeStyle);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShapePaintInfo {
    Inherited,
    None,
    Solid { color: String },
    Other,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeStyleInfo {
    pub fill: ShapePaintInfo,
    pub outline: ShapePaintInfo,
    pub outline_width_pt: Option<f64>,
    pub can_edit: bool,
}
fn is_fill(n: &Node) -> bool {
    n.ns == 2
        && matches!(
            n.name.as_str(),
            "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill"
        )
}
fn paint(nodes: &[Node], pr: Option<&Node>) -> ShapePaintInfo {
    let fills: Vec<_> = pr
        .into_iter()
        .flat_map(|n| &n.children)
        .map(|i| &nodes[*i])
        .filter(|n| is_fill(n))
        .collect();
    match fills.as_slice() {
        [] => ShapePaintInfo::Inherited,
        [n] if n.is(2, "noFill") => ShapePaintInfo::None,
        [n] if n.is(2, "solidFill") && n.children.len() == 1 => {
            if let Some(rgb) = child(nodes, n, 2, "srgbClr")
                .filter(|c| c.children.is_empty())
                .and_then(|c| c.attr("val"))
                .filter(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                ShapePaintInfo::Solid {
                    color: format!("#{}", rgb.to_ascii_uppercase()),
                }
            } else {
                ShapePaintInfo::Other
            }
        }
        _ => ShapePaintInfo::Other,
    }
}
fn supported(nodes: &[Node], id: u32) -> Result<(Vec<&Node>, bool)> {
    let shape = crop::selected(nodes, id)?;
    if !matches!(shape.name.as_str(), "sp" | "cxnSp") {
        return Ok((Vec::new(), false));
    }
    let branches = crop::branches(nodes, shape, id);
    let mut info = super::transform_info(nodes, shape).ok_or(WriteError::NoShape(id))?;
    objects::capabilities(nodes, &objects::References::new(nodes), shape, &mut info);
    let supported = info.can_arrange
        && !branches.is_empty()
        && branches.iter().all(|n| {
            matches!(n.name.as_str(), "sp" | "cxnSp") && child(nodes, n, 1, "spPr").is_some()
        });
    Ok((branches, supported))
}
pub fn shape_style(bytes: &[u8], slide: usize, id: u32) -> Result<Option<ShapeStyleInfo>> {
    let (_, _, xml) = crate::slide_xml(bytes, slide)?;
    let nodes = parse_nodes(&xml)?;
    let (branches, can_edit) = supported(&nodes, id)?;
    let Some(shape) = branches.first() else {
        return Ok(None);
    };
    let pr = child(&nodes, shape, 1, "spPr");
    let ln = pr.and_then(|pr| child(&nodes, pr, 2, "ln"));
    Ok(Some(ShapeStyleInfo {
        fill: paint(&nodes, pr),
        outline: paint(&nodes, ln),
        outline_width_pt: ln
            .and_then(|ln| ln.attr("w"))
            .and_then(|w| w.parse::<u32>().ok())
            .map(|w| f64::from(w) / 12700.0),
        can_edit,
    }))
}
fn fragment(value: &str, strict: bool) -> String {
    let ns = drawing_ns(strict);
    if value == "none" {
        format!("<jadeStyle:noFill xmlns:jadeStyle=\"{ns}\"/>")
    } else {
        format!("<jadeStyle:solidFill xmlns:jadeStyle=\"{ns}\"><jadeStyle:srgbClr val=\"{}\"/></jadeStyle:solidFill>", value[1..].to_ascii_uppercase())
    }
}
fn rank(n: &Node, line: bool) -> Option<u8> {
    if n.ns != 2 {
        return None;
    }
    if is_fill(n) {
        return Some(1);
    }
    Some(if line {
        match n.name.as_str() {
            "prstDash" | "custDash" => 2,
            "round" | "bevel" | "miter" => 3,
            "headEnd" => 4,
            "tailEnd" => 5,
            "extLst" => 6,
            _ => return None,
        }
    } else {
        match n.name.as_str() {
            "xfrm" | "prstGeom" | "custGeom" => 0,
            "ln" => 2,
            "effectLst" | "effectDag" => 3,
            "scene3d" => 4,
            "sp3d" => 5,
            "extLst" => 6,
            _ => return None,
        }
    })
}
fn properties(
    xml: &str,
    nodes: &[Node],
    pr: &Node,
    fill: Option<&str>,
    outline: Option<String>,
    width: Option<f64>,
    line: bool,
) -> Result<String> {
    let mut patches: Vec<(Range<usize>, String)> = Vec::new();
    let mut additions = BTreeMap::<usize, String>::new();
    let mut append = |order: u8, value: String| {
        let at = pr
            .children
            .iter()
            .map(|i| &nodes[*i])
            .find(|n| rank(n, line).is_some_and(|r| r > order))
            .map_or(if pr.empty { pr.outer.end } else { pr.range.end }, |n| {
                n.outer.start
            });
        additions.entry(at).or_default().push_str(&value);
    };
    if let Some(fill) = fill {
        let current = paint(nodes, Some(pr));
        let same = if fill == "none" {
            current == ShapePaintInfo::None
        } else {
            current
                == ShapePaintInfo::Solid {
                    color: fill.to_ascii_uppercase(),
                }
        };
        if !same {
            let fills: Vec<_> = pr
                .children
                .iter()
                .map(|i| &nodes[*i])
                .filter(|n| is_fill(n))
                .collect();
            let value = fragment(fill, pr.strict);
            if let Some(first) = fills.first() {
                patches.push((first.outer.clone(), value));
                patches.extend(
                    fills
                        .iter()
                        .skip(1)
                        .map(|n| (n.outer.clone(), String::new())),
                );
            } else {
                append(1, value);
            }
        }
    }
    if let Some(outline) = outline {
        if let Some(ln) = child(nodes, pr, 2, "ln") {
            patches.push((ln.outer.clone(), outline));
        } else {
            append(2, outline);
        }
    }
    let values = width
        .map(|w| vec![("w", ((w * 12700.0).round() as u32).to_string())])
        .unwrap_or_default();
    let opening = edit_tag(&xml[pr.opening.clone()], &values)?;
    if pr.empty && !additions.is_empty() {
        return Ok(format!(
            "{}>{}</{}>",
            opening
                .strip_suffix("/>")
                .ok_or_else(|| WriteError::Xml("empty shape properties".into()))?,
            additions.into_values().collect::<String>(),
            pr.qualified
        ));
    }
    patches.push((pr.opening.clone(), opening));
    patches.extend(additions.into_iter().map(|(at, value)| (at..at, value)));
    let patches = patches
        .into_iter()
        .map(|(r, v)| (r.start - pr.outer.start..r.end - pr.outer.start, v))
        .collect();
    Ok(apply_patches(&xml[pr.outer.clone()], patches))
}
fn edit(xml: &str, id: u32, value: &ShapeStyleEdit) -> Result<String> {
    value.validate()?;
    let nodes = parse_nodes(xml)?;
    let (branches, can_edit) = supported(&nodes, id)?;
    if !can_edit {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let mut patches = Vec::new();
    for shape in branches {
        let pr = child(&nodes, shape, 1, "spPr").ok_or(WriteError::NoShape(id))?;
        let outline = if value.outline.is_some() || value.outline_width_pt.is_some() {
            if let Some(ln) = child(&nodes, pr, 2, "ln") {
                Some(properties(
                    xml,
                    &nodes,
                    ln,
                    value.outline.as_deref(),
                    None,
                    value.outline_width_pt,
                    true,
                )?)
            } else {
                let ns = drawing_ns(pr.strict);
                let width = value
                    .outline_width_pt
                    .map(|w| format!(" w=\"{}\"", (w * 12700.0).round() as u32))
                    .unwrap_or_default();
                let fill = value
                    .outline
                    .as_deref()
                    .map(|v| fragment(v, pr.strict))
                    .unwrap_or_default();
                Some(format!(
                    "<jadeStyle:ln xmlns:jadeStyle=\"{ns}\"{width}>{fill}</jadeStyle:ln>"
                ))
            }
        } else {
            None
        };
        patches.push((
            pr.outer.clone(),
            properties(xml, &nodes, pr, value.fill.as_deref(), outline, None, false)?,
        ));
    }
    Ok(apply_patches(xml, patches))
}
pub fn validate_shape_style(
    bytes: &[u8],
    slide: usize,
    id: u32,
    value: &ShapeStyleEdit,
) -> Result<()> {
    let (_, _, xml) = crate::slide_xml(bytes, slide)?;
    edit(&xml, id, value).map(|_| ())
}
pub fn set_shape_style(
    bytes: &[u8],
    slide: usize,
    id: u32,
    value: &ShapeStyleEdit,
) -> Result<Vec<u8>> {
    crate::shape_operation(bytes, slide, |xml| edit(xml, id, value))
}
