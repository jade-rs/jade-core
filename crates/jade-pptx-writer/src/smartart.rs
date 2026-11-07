//! SmartArt as PowerPoint's SmartArt のデザイン and 書式 tabs edit it:
//! inserting a diagram, its bullets (the text pane, adding, removing,
//! promoting and demoting shapes, assistants), its layout, colours and
//! style, one shape's own formatting and picture, turning a bulleted list
//! into a diagram and a diagram into plain shapes.
//!
//! jade writes a diagram as its data and its drawing: the data is what
//! PowerPoint reads, the drawing what every other reader shows. A layout,
//! colour scheme or style jade sets leaves the frame's part for it empty,
//! and PowerPoint takes its built-in definition for the id the data names.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_reader::Transform;
use jade_smartart::{DataModel, Node, ShapeFormat};
use serde::{Deserialize, Serialize};

use crate::xml::smartart_frames::{self, RelIds};
use crate::{Result, WriteError};

const DIAGRAM_DATA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml";
const DIAGRAM_DRAWING_TYPE: &str = "application/vnd.ms-office.drawingml.diagramDrawing+xml";
const DRAWING_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// A diagram as the SmartArt tabs show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartArtInfo {
    pub layout: String,
    pub colors: String,
    pub style: String,
    /// The bullets of the text pane, as a tree.
    #[serde(skip)]
    pub nodes: Vec<Node>,
    /// The drawn shapes, back to front.
    pub shapes: Vec<SmartArtShape>,
    /// Whether jade lays this layout out itself (a structure edit draws it
    /// anew) or only brings its text up to date.
    pub laid_out_by_jade: bool,
}

/// One shape of a diagram's drawing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartArtShape {
    /// Its `modelId`.
    pub model: String,
    /// The bullets it shows: the first is the one selecting it selects.
    pub nodes: Vec<String>,
    /// Its box in the frame, EMU: x, y, width, height.
    pub rect: (i64, i64, i64, i64),
}

/// One change to a diagram.
#[derive(Clone, Debug, PartialEq)]
pub enum SmartArtEdit {
    /// The bullets, as the text pane leaves them: ids kept for bullets that
    /// stay, empty for new ones.
    Nodes(Vec<Node>),
    /// レイアウトの変更.
    Layout(String),
    /// 色の変更.
    Colors(String),
    /// SmartArt のスタイル.
    Style(String),
    /// One bullet's shape: fill, outline, text colour, shape.
    Format { node: String, format: ShapeFormat },
    /// グラフィックのリセット: every shape's own formatting cleared.
    ResetGraphic,
}

fn add_relationship(
    package: &mut OpcPackage,
    owner: &str,
    kind: &str,
    target: &str,
    avoid: &HashSet<String>,
) -> Result<String> {
    add_relationship_named(package, owner, kind, target, avoid, "rIdJadeDiagram")
}

/// Adds a relationship from `owner` to `target`, its id `stem` and the
/// first free number.
pub(crate) fn add_relationship_named(
    package: &mut OpcPackage,
    owner: &str,
    kind: &str,
    target: &str,
    avoid: &HashSet<String>,
    stem: &str,
) -> Result<String> {
    let rel_part = rels_part_name(owner);
    let xml = package
        .part_str(&rel_part)?
        .unwrap_or_else(|| format!("<Relationships xmlns=\"{RELS_NS}\"></Relationships>"));
    let existing = parse_relationships(&xml)?;
    let taken: HashSet<String> = existing.all().iter().map(|r| r.id.clone()).collect();
    let rid = (1..)
        .map(|n| format!("{stem}{n}"))
        .find(|rid| !taken.contains(rid) && !avoid.contains(rid))
        .expect("unbounded");
    let relation = format!(
        "<Relationship xmlns=\"{RELS_NS}\" Id=\"{rid}\" Type=\"{kind}\" Target=\"/{target}\"/>"
    );
    let updated = match xml.rfind("</") {
        Some(at) if !xml.trim_end().ends_with("/>") => {
            format!("{}{relation}{}", &xml[..at], &xml[at..])
        }
        _ => {
            // `<Relationships/>`: open it up.
            let close = xml.rfind("/>").unwrap_or(xml.len());
            format!("{}>{relation}</Relationships>", &xml[..close])
        }
    };
    package.replace(&rel_part, updated.into_bytes());
    Ok(rid)
}

pub(crate) fn remove_relationship(
    package: &mut OpcPackage,
    owner: &str,
    rid: &str,
) -> Result<Option<String>> {
    let rel_part = rels_part_name(owner);
    let Some(xml) = package.part_str(&rel_part)? else {
        return Ok(None);
    };
    let target = parse_relationships(&xml)?
        .relationship(rid)
        .filter(|r| !r.external)
        .map(|r| resolve_target(owner, &r.target));
    let mut root = jade_smartart::xml::parse(&xml).map_err(WriteError::Xml)?;
    root.children.retain(
        |node| !matches!(node, jade_smartart::xml::Node::Element(e) if e.attr("Id") == Some(rid)),
    );
    package.replace(&rel_part, root.to_document().into_bytes());
    Ok(target)
}

/// Removes `part` (and what only it refers to) when no relationship of the
/// package points at it any more.
pub(crate) fn drop_if_unreferenced(package: &mut OpcPackage, part: &str) -> Result<()> {
    let names: Vec<String> = package.part_names().to_vec();
    for name in names.iter().filter(|name| name.ends_with(".rels")) {
        let owner = name
            .strip_prefix("_rels/")
            .map(|rest| rest.trim_end_matches(".rels").to_string())
            .unwrap_or_else(|| {
                name.replace("/_rels/", "/")
                    .trim_end_matches(".rels")
                    .to_string()
            });
        let rels = parse_relationships(&package.required_part_str(name)?)?;
        if rels
            .all()
            .iter()
            .any(|r| !r.external && resolve_target(&owner, &r.target) == part)
        {
            return Ok(());
        }
    }
    let owned_rels = rels_part_name(part);
    let mut removed = BTreeSet::new();
    if package.remove(part) {
        removed.insert(part.to_string());
    }
    package.remove(&owned_rels);
    crate::xml::slides_content_types(package, &BTreeMap::new(), &removed, None)?;
    Ok(())
}

fn fresh_part(package: &OpcPackage, stem: &str) -> String {
    (1..)
        .map(|n| format!("ppt/diagrams/{stem}{n}.xml"))
        .find(|name| package.part(name).is_none())
        .expect("unbounded")
}

struct Located {
    slide_part: String,
    slide_xml: String,
    rel_ids: RelIds,
    data_part: String,
    data: DataModel,
    drawing_part: Option<String>,
    frame: Transform,
}

fn locate(package: &OpcPackage, slide_part: &str, slide_xml: &str, id: u32) -> Result<Located> {
    let rel_ids = smartart_frames::rel_ids(slide_xml, id)?.ok_or(WriteError::NoShape(id))?;
    let rels = parse_relationships(
        &package
            .part_str(&rels_part_name(slide_part))?
            .unwrap_or_default(),
    )?;
    let data_part = rels
        .target(&rel_ids.data)
        .map(|target| resolve_target(slide_part, target))
        .ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let data =
        DataModel::parse(&package.required_part_str(&data_part)?).map_err(WriteError::Xml)?;
    let drawing_part = data
        .drawing_rel_id()
        .and_then(|rid| rels.target(&rid).map(str::to_string))
        .map(|target| resolve_target(slide_part, &target));
    let frame = crate::xml::shapes(slide_xml)?
        .into_iter()
        .find(|shape| shape.id == id)
        .and_then(|shape| shape.transform)
        .ok_or(WriteError::NoShape(id))?;
    Ok(Located {
        slide_part: slide_part.to_string(),
        slide_xml: slide_xml.to_string(),
        rel_ids,
        data_part,
        data,
        drawing_part,
        frame,
    })
}

fn part_of(package: &OpcPackage, slide_part: &str, rid: &str) -> Result<Option<String>> {
    if rid.is_empty() {
        return Ok(None);
    }
    let rels = parse_relationships(
        &package
            .part_str(&rels_part_name(slide_part))?
            .unwrap_or_default(),
    )?;
    Ok(match rels.target(rid) {
        Some(target) => package.part_str(&resolve_target(slide_part, target))?,
        None => None,
    })
}

/// The diagram in frame `id` on `slide`, or `None` when it is no diagram.
pub fn smartart(bytes: &[u8], slide: usize, id: u32) -> Result<Option<SmartArtInfo>> {
    let (package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    if smartart_frames::rel_ids(&slide_xml, id)?.is_none() {
        return Ok(None);
    }
    let found = locate(&package, &slide_part, &slide_xml, id)?;
    let nodes = found.data.nodes();
    let shown = found.data.shown_in();
    let mut shapes: Vec<SmartArtShape> = Vec::new();
    if let Some(drawing) = found
        .drawing_part
        .as_deref()
        .map(|part| package.part_str(part))
        .transpose()?
        .flatten()
    {
        let root = jade_smartart::xml::parse(&drawing).map_err(WriteError::Xml)?;
        let mut sps = Vec::new();
        root.descendants("sp", &mut sps);
        let known: HashSet<String> = jade_smartart::data::outline(&nodes)
            .into_iter()
            .map(|(_, node)| node.id.clone())
            .collect();
        for sp in sps {
            let Some(model) = sp.attr("modelId") else {
                continue;
            };
            let shows = match shown.get(model) {
                Some(ids) => ids
                    .iter()
                    .filter(|id| known.contains(*id))
                    .cloned()
                    .collect(),
                None if known.contains(model) => vec![model.to_string()],
                None => Vec::new(),
            };
            let xfrm = sp.child("spPr").and_then(|pr| pr.child("xfrm"));
            let number = |name: &str, attr: &str| {
                xfrm.and_then(|x| x.child(name))
                    .and_then(|e| e.attr(attr))
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0)
            };
            shapes.push(SmartArtShape {
                model: model.to_string(),
                nodes: shows,
                rect: (
                    number("off", "x"),
                    number("off", "y"),
                    number("ext", "cx"),
                    number("ext", "cy"),
                ),
            });
        }
    }
    let layout = found.data.layout_id().unwrap_or_default();
    Ok(Some(SmartArtInfo {
        laid_out_by_jade: jade_smartart::drawing::lays_out(&layout),
        layout,
        colors: found.data.colors_id().unwrap_or_default(),
        style: found.data.style_id().unwrap_or_default(),
        nodes,
        shapes,
    }))
}

fn palette(bytes: &[u8]) -> Result<jade_smartart::colors::ThemePalette> {
    let deck = jade_pptx_reader::read_deck(bytes)?;
    Ok(jade_pptx_reader::theme_palette(&deck.theme_colors))
}

/// Writes `found`'s data and a drawing for it: laid out anew when jade lays
/// the layout out or `relayout` asks, else the old drawing with its text
/// brought up to date.
fn store(
    package: &mut OpcPackage,
    found: &mut Located,
    palette: &jade_smartart::colors::ThemePalette,
    relayout: bool,
) -> Result<()> {
    let layout = found.data.layout_id().unwrap_or_default();
    let old_drawing = match &found.drawing_part {
        Some(part) => package.part_str(part)?,
        None => None,
    };
    let parts_xml = (
        part_of(package, &found.slide_part, &found.rel_ids.layout)?,
        part_of(package, &found.slide_part, &found.rel_ids.colors)?,
        part_of(package, &found.slide_part, &found.rel_ids.style)?,
    );
    let drawing = match old_drawing {
        Some(old) if !relayout && !jade_smartart::drawing::lays_out(&layout) => {
            jade_smartart::update_text(&old, &found.data).map_err(WriteError::Xml)?
        }
        _ => jade_smartart::render_model(
            &found.data,
            jade_smartart::Parts {
                layout: parts_xml.0.as_deref(),
                colors: parts_xml.1.as_deref(),
                style: parts_xml.2.as_deref(),
            },
            Some(palette),
            found.frame.extent_emu.0,
            found.frame.extent_emu.1,
        ),
    };
    let drawing_part = match &found.drawing_part {
        Some(part) => part.clone(),
        None => {
            let part = fresh_part(package, "drawing");
            let avoid = HashSet::new();
            let rid = add_relationship(package, &found.slide_part, DRAWING_REL, &part, &avoid)?;
            found.data.set_drawing_rel_id(&rid);
            crate::xml::slides_content_types(
                package,
                &BTreeMap::new(),
                &BTreeSet::new(),
                Some((&part, DIAGRAM_DRAWING_TYPE)),
            )?;
            found.drawing_part = Some(part.clone());
            part
        }
    };
    // Pictures in the shapes: the drawing names them by the data's
    // relationship ids, so it gets the same relationships.
    mirror_picture_relationships(package, &found.data_part, &drawing_part)?;
    package.replace(&drawing_part, drawing.into_bytes());
    package.replace(&found.data_part, found.data.to_xml().into_bytes());
    package.replace(&found.slide_part, found.slide_xml.clone().into_bytes());
    Ok(())
}

fn mirror_picture_relationships(
    package: &mut OpcPackage,
    data_part: &str,
    drawing_part: &str,
) -> Result<()> {
    let Some(data_rels) = package.part_str(&rels_part_name(data_part))? else {
        return Ok(());
    };
    let data_rels = parse_relationships(&data_rels)?;
    let drawing_rels_xml = package
        .part_str(&rels_part_name(drawing_part))?
        .unwrap_or_else(|| format!("<Relationships xmlns=\"{RELS_NS}\"></Relationships>"));
    let drawing_rels = parse_relationships(&drawing_rels_xml)?;
    let mut additions = String::new();
    for rel in data_rels.all() {
        if rel.relationship_type.ends_with("/image") && drawing_rels.relationship(&rel.id).is_none()
        {
            let target = resolve_target(data_part, &rel.target);
            additions.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"/{}\"/>",
                jade_smartart::xml::escape(&rel.id),
                jade_smartart::xml::escape(&rel.relationship_type),
                jade_smartart::xml::escape(&target)
            ));
        }
    }
    if additions.is_empty() {
        return Ok(());
    }
    let mut root = jade_smartart::xml::parse(&drawing_rels_xml).map_err(WriteError::Xml)?;
    root.children.push(jade_smartart::xml::Node::Raw(additions));
    package.replace(
        &rels_part_name(drawing_part),
        root.to_document().into_bytes(),
    );
    Ok(())
}

/// Inserts a diagram of layout `layout` showing `nodes` at `transform` on
/// `slide`: its frame id.
pub fn insert_smartart(
    bytes: &[u8],
    slide: usize,
    transform: Transform,
    layout: &str,
    nodes: &[Node],
) -> Result<(Vec<u8>, u32)> {
    crate::validate_transform(transform)?;
    if transform.extent_emu.0 <= 0 || transform.extent_emu.1 <= 0 {
        return Err(WriteError::InvalidTransform);
    }
    let palette = palette(bytes)?;
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let (_, _, r_ns) = smartart_frames::namespaces(&slide_xml)?;
    let data_part = fresh_part(&package, "data");
    let avoid = HashSet::new();
    let data_rid = add_relationship(
        &mut package,
        &slide_part,
        &format!("{r_ns}/diagramData"),
        &data_part,
        &avoid,
    )?;
    crate::xml::slides_content_types(
        &mut package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((&data_part, DIAGRAM_DATA_TYPE)),
    )?;
    let mut data = DataModel::new(layout, "accent1_2", "simple1");
    data.set_nodes(nodes);
    package.replace(&data_part, data.to_xml().into_bytes());
    let (slide_xml, id) = smartart_frames::insert_frame(&slide_xml, transform, &data_rid)?;
    let mut found = Located {
        slide_part: slide_part.clone(),
        slide_xml,
        rel_ids: RelIds {
            data: data_rid,
            ..RelIds::default()
        },
        data_part,
        data,
        drawing_part: None,
        frame: transform,
    };
    store(&mut package, &mut found, &palette, true)?;
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}

/// Applies `edit` to the diagram in frame `id`.
pub fn edit_smartart(bytes: &[u8], slide: usize, id: u32, edit: &SmartArtEdit) -> Result<Vec<u8>> {
    let palette = palette(bytes)?;
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let mut found = locate(&package, &slide_part, &slide_xml, id)?;
    let mut drop = Vec::new();
    let relayout = match edit {
        SmartArtEdit::Nodes(nodes) => {
            let before = shape_of(&found.data.nodes());
            found.data.set_nodes(nodes);
            before != shape_of(&found.data.nodes())
        }
        SmartArtEdit::Layout(layout) => {
            found
                .data
                .set_layout_id(&jade_smartart::catalog::layout_id(layout));
            drop.push(("lo", found.rel_ids.layout.clone()));
            found.rel_ids.layout.clear();
            true
        }
        SmartArtEdit::Colors(colors) => {
            found
                .data
                .set_colors_id(&jade_smartart::catalog::colors_id(colors));
            drop.push(("cs", found.rel_ids.colors.clone()));
            found.rel_ids.colors.clear();
            true
        }
        SmartArtEdit::Style(style) => {
            found
                .data
                .set_style_id(&jade_smartart::catalog::style_id(style));
            drop.push(("qs", found.rel_ids.style.clone()));
            found.rel_ids.style.clear();
            true
        }
        SmartArtEdit::Format { node, format } => {
            found.data.set_format(node, format);
            true
        }
        SmartArtEdit::ResetGraphic => {
            for node in found.data.nodes_flat() {
                found.data.set_format(&node.id, &ShapeFormat::default());
            }
            true
        }
    };
    if !drop.is_empty() {
        let names: Vec<&str> = drop.iter().map(|(name, _)| *name).collect();
        found.slide_xml = smartart_frames::blank_rel_ids(&found.slide_xml, id, &names)?;
    }
    // Write the slide first so the old parts' relationships go with it.
    package.replace(&slide_part, found.slide_xml.clone().into_bytes());
    for (_, rid) in drop.iter().filter(|(_, rid)| !rid.is_empty()) {
        // Another frame may share the part; only this frame's link goes.
        let shared = smartart_frames_using(&found.slide_xml, rid);
        if !shared {
            if let Some(part) = remove_relationship(&mut package, &slide_part, rid)? {
                drop_if_unreferenced(&mut package, &part)?;
            }
        }
    }
    store(&mut package, &mut found, &palette, relayout)?;
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

fn smartart_frames_using(slide_xml: &str, rid: &str) -> bool {
    slide_xml.contains(&format!(":lo=\"{rid}\""))
        || slide_xml.contains(&format!(":qs=\"{rid}\""))
        || slide_xml.contains(&format!(":cs=\"{rid}\""))
}

/// The nesting of bullets without their text: what decides whether a
/// layout jade does not lay out needs drawing anew.
fn shape_of(nodes: &[Node]) -> Vec<(usize, String, jade_smartart::NodeKind)> {
    jade_smartart::data::outline(nodes)
        .into_iter()
        .map(|(level, node)| (level, node.id.clone(), node.kind))
        .collect()
}

/// Fills the picture shape of bullet `node` with `image` (PNG or JPEG).
pub fn set_smartart_picture(
    bytes: &[u8],
    slide: usize,
    id: u32,
    node: &str,
    image: &[u8],
) -> Result<Vec<u8>> {
    let info = crate::xml::pictures::picture_info(image)?;
    let palette = palette(bytes)?;
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let mut found = locate(&package, &slide_part, &slide_xml, id)?;
    let media = crate::xml::pictures::media_part(&package, image, info.mime_type)?;
    package.replace(&media, image.to_vec());
    crate::xml::slides_content_types(
        &mut package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((&media, info.mime_type)),
    )?;
    // One id free in both the data's and the drawing's relationships.
    let mut avoid = HashSet::new();
    if let Some(drawing) = &found.drawing_part {
        if let Some(xml) = package.part_str(&rels_part_name(drawing))? {
            avoid.extend(
                parse_relationships(&xml)?
                    .all()
                    .iter()
                    .map(|r| r.id.clone()),
            );
        }
    }
    let (_, _, r_ns) = smartart_frames::namespaces(&slide_xml)?;
    let rid = add_relationship(
        &mut package,
        &found.data_part,
        &format!("{r_ns}/image"),
        &media,
        &avoid,
    )?;
    let mut format = found
        .data
        .nodes_flat()
        .into_iter()
        .find(|n| n.id == node)
        .map(|n| n.format)
        .ok_or(WriteError::NoShape(id))?;
    format.picture = Some(rid);
    found.data.set_format(node, &format);
    store(&mut package, &mut found, &palette, true)?;
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// 図形に変換: the diagram in frame `id` becomes a group of the shapes its
/// drawing shows, which no longer change with its bullets. The group's id.
pub fn convert_smartart_to_shapes(bytes: &[u8], slide: usize, id: u32) -> Result<(Vec<u8>, u32)> {
    let (mut package, slide_part, slide_xml) = crate::slide_xml(bytes, slide)?;
    let found = locate(&package, &slide_part, &slide_xml, id)?;
    // A diagram without a drawing is drawn first, as the reader draws it.
    let drawing = match &found.drawing_part {
        Some(part) => package.required_part_str(part)?,
        None => jade_smartart::render_model(
            &found.data,
            jade_smartart::Parts::default(),
            Some(&palette(bytes)?),
            found.frame.extent_emu.0,
            found.frame.extent_emu.1,
        ),
    };
    let (p_ns, a_ns, r_ns) = smartart_frames::namespaces(&slide_xml)?;
    let (group_id, _) = smartart_frames::fresh_identity(&slide_xml, "Group")?;
    let name = crate::xml::shapes(&slide_xml)?
        .into_iter()
        .find(|shape| shape.id == id)
        .map(|shape| shape.name)
        .unwrap_or_else(|| format!("Group {group_id}"));
    // Pictures: the slide gets relationships to the drawing's images.
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    if let Some(drawing_part) = &found.drawing_part {
        if let Some(xml) = package.part_str(&rels_part_name(drawing_part))? {
            for rel in parse_relationships(&xml)?.all() {
                if rel.external || !rel.relationship_type.ends_with("/image") {
                    continue;
                }
                let target = resolve_target(drawing_part, &rel.target);
                let rid = add_relationship(
                    &mut package,
                    &slide_part,
                    &rel.relationship_type,
                    &target,
                    &HashSet::new(),
                )?;
                renamed.insert(rel.id.clone(), rid);
            }
        }
    }
    let root = jade_smartart::xml::parse(&drawing).map_err(WriteError::Xml)?;
    let mut sps = Vec::new();
    root.descendants("sp", &mut sps);
    let (fx, fy) = found.frame.offset_emu;
    let (fw, fh) = found.frame.extent_emu;
    let mut out = format!(
        "<jadeSp:grpSp xmlns:jadeSp=\"{p_ns}\" xmlns:a=\"{a_ns}\" xmlns:r=\"{r_ns}\"><jadeSp:nvGrpSpPr><jadeSp:cNvPr id=\"{group_id}\" name=\"{}\"/><jadeSp:cNvGrpSpPr/><jadeSp:nvPr/></jadeSp:nvGrpSpPr><jadeSp:grpSpPr><a:xfrm><a:off x=\"{fx}\" y=\"{fy}\"/><a:ext cx=\"{fw}\" cy=\"{fh}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{fw}\" cy=\"{fh}\"/></a:xfrm></jadeSp:grpSpPr>",
        jade_smartart::xml::escape(&name)
    );
    for (index, sp) in sps.iter().enumerate() {
        let shape_id = group_id + 1 + index as u32;
        out.push_str(&format!(
            "<jadeSp:sp><jadeSp:nvSpPr><jadeSp:cNvPr id=\"{shape_id}\" name=\"Shape {shape_id}\"/><jadeSp:cNvSpPr/><jadeSp:nvPr/></jadeSp:nvSpPr>"
        ));
        for (local, required) in [("spPr", true), ("style", false), ("txBody", false)] {
            match sp.child(local) {
                Some(element) => {
                    let mut element = element.clone();
                    element.name = format!("jadeSp:{local}");
                    rename_embeds(&mut element, &renamed);
                    out.push_str(&element.to_xml());
                }
                None if required => out.push_str("<jadeSp:spPr/>"),
                None => {}
            }
        }
        out.push_str("</jadeSp:sp>");
    }
    out.push_str("</jadeSp:grpSp>");
    let slide_xml = smartart_frames::replace_frame(&slide_xml, id, &out)?;
    package.replace(&slide_part, slide_xml.clone().into_bytes());
    // The diagram's parts go with their relationships.
    let rel_ids = found.rel_ids.clone();
    let mut dropped = vec![rel_ids.data, rel_ids.layout, rel_ids.style, rel_ids.colors];
    if let Some(rid) = found.data.drawing_rel_id() {
        dropped.push(rid);
    }
    for rid in dropped.into_iter().filter(|rid| !rid.is_empty()) {
        if slide_xml.contains(&format!("\"{rid}\"")) {
            continue;
        }
        if let Some(part) = remove_relationship(&mut package, &slide_part, &rid)? {
            drop_if_unreferenced(&mut package, &part)?;
        }
    }
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, group_id))
}

fn rename_embeds(element: &mut jade_smartart::xml::Element, renamed: &BTreeMap<String, String>) {
    for (key, value) in element.attrs.iter_mut() {
        if matches!(key.rsplit(':').next(), Some("embed" | "link")) {
            if let Some(new) = renamed.get(value.as_str()) {
                *value = new.clone();
            }
        }
    }
    for child in element.elements_mut() {
        rename_embeds(child, renamed);
    }
}

/// SmartArt グラフィックに変換: the bulleted text of shape `id` becomes a
/// diagram of `layout` in its place, one bullet per paragraph, nested by
/// indent level. The diagram's frame id.
pub fn convert_text_to_smartart(
    bytes: &[u8],
    slide: usize,
    id: u32,
    layout: &str,
) -> Result<(Vec<u8>, u32)> {
    let deck = jade_pptx_reader::read_deck(bytes)?;
    let shape = deck
        .slides
        .get(slide)
        .ok_or(WriteError::NoSlide(slide))?
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            jade_pptx_reader::ShapeItem::Shape(shape) if shape.id == Some(id) => Some(shape),
            _ => None,
        })
        .ok_or(WriteError::NoShape(id))?;
    let transform = crate::editable_shapes(bytes, slide)?
        .into_iter()
        .find(|info| info.id == id)
        .and_then(|info| info.transform)
        .or(shape.transform)
        .ok_or(WriteError::NoShape(id))?;
    let body = shape.text.as_ref().ok_or(WriteError::NoTextBody(id))?;
    let lines: Vec<(usize, Node)> = body
        .paragraphs
        .iter()
        .map(|p| {
            (
                usize::from(p.level),
                Node::new(p.runs.iter().map(|r| r.text.as_str()).collect::<String>()),
            )
        })
        .filter(|(_, node)| !node.text.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Err(WriteError::NoTextBody(id));
    }
    let base = lines.iter().map(|(level, _)| *level).min().unwrap_or(0);
    let nodes = jade_smartart::data::from_outline(
        lines
            .into_iter()
            .map(|(level, node)| (level - base, node))
            .collect(),
    );
    let without = crate::delete_shape(bytes, slide, id)?;
    insert_smartart(&without, slide, transform, layout, &nodes)
}
