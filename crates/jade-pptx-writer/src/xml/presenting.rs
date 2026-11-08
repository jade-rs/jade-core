//! Writes how a deck is presented and viewed: スライド ショーの設定
//! (`presProps.xml`'s `<p:showPr>` and `<p:prnPr>`), 目的別スライド ショー
//! (`<p:custShowLst>`), rehearsed advance times (`<p:transition advTm>`) and
//! the slide view's grid and guides (`viewProps.xml`).
//!
//! Each edit rewrites one element and leaves everything around it — other
//! properties, extensions PowerPoint added — byte for byte.

use super::slides::{content_types, p_ns, r_ns};
use super::{apply_patches, child, edit_tag, parse_document_nodes, remove_tag_attributes};
use super::{xml_error, Node};
use crate::{Result, WriteError};
use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_reader::{CustomShow, ShowRange, ShowSettings, ShowType, ViewProps};
use std::collections::{BTreeMap, BTreeSet};

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const LASER_EXT: &str = "{EC167BDD-8182-4AB7-AECC-EB403E3ABB37}";
const PRES_PROPS_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml";
const VIEW_PROPS_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml";

/// `CT_PresentationProperties`' children, in schema order.
const PRES_PROPS_ORDER: &[&str] = &["htmlPubPr", "webPr", "prnPr", "showPr", "clrMru", "extLst"];
/// `CT_Presentation`'s children, in schema order.
pub(super) const PRESENTATION_ORDER: &[&str] = &[
    "sldMasterIdLst",
    "notesMasterIdLst",
    "handoutMasterIdLst",
    "sldIdLst",
    "sldSz",
    "notesSz",
    "smartTags",
    "embeddedFontLst",
    "custShowLst",
    "photoAlbum",
    "custDataLst",
    "kinsoku",
    "defaultTextStyle",
    "modifyVerifier",
    "extLst",
];
/// `CT_ViewProperties`' children, in schema order.
const VIEW_PROPS_ORDER: &[&str] = &[
    "normalViewPr",
    "slideViewPr",
    "outlineViewPr",
    "notesTextViewPr",
    "sorterViewPr",
    "notesViewPr",
    "gridSpacing",
    "extLst",
];

pub(super) fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

/// `p:` (or whatever prefix the part's root uses for PresentationML).
pub(super) fn prefix(node: &Node) -> String {
    node.qualified
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default()
}

pub(super) fn presentation_part(package: &OpcPackage) -> Result<String> {
    Ok(package
        .part_str("_rels/.rels")?
        .and_then(|xml| parse_relationships(&xml).ok())
        .and_then(|rels| {
            rels.target_by_type_suffix("/officeDocument")
                .map(|target| resolve_target("", target))
        })
        .unwrap_or_else(|| "ppt/presentation.xml".into()))
}

/// The part `owner` relates to with a relationship of type `…/suffix`.
pub(super) fn related(package: &OpcPackage, owner: &str, suffix: &str) -> Result<Option<String>> {
    let Some(xml) = package.part_str(&rels_part_name(owner))? else {
        return Ok(None);
    };
    Ok(parse_relationships(&xml)?
        .by_type_suffix(&format!("/{suffix}"))
        .into_iter()
        .find(|r| !r.external)
        .map(|r| resolve_target(owner, &r.target)))
}

/// Adds a relationship of type `kind` from `owner` to `target`, returning
/// its id.
pub(super) fn add_relationship(
    package: &mut OpcPackage,
    owner: &str,
    kind: &str,
    target: &str,
    strict: bool,
) -> Result<String> {
    let name = rels_part_name(owner);
    let xml = package.part_str(&name)?.unwrap_or_else(|| {
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
            .into()
    });
    let rels = parse_relationships(&xml)?;
    let id = (1..)
        .map(|i| format!("rId{i}"))
        .find(|id| rels.relationship(id).is_none())
        .expect("a free relationship id");
    let nodes = parse_document_nodes(&xml)?;
    let tag = nodes[0]
        .qualified
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:Relationship"))
        .unwrap_or_else(|| "Relationship".into());
    let relationship = format!(
        "<{tag} Id=\"{id}\" Type=\"{}/{kind}\" Target=\"/{}\"/>",
        r_ns(strict),
        escape(target)
    );
    package.replace(
        &name,
        super::slides::append_child(&xml, &nodes[0], &relationship).into_bytes(),
    );
    Ok(id)
}

/// Registers `part`'s content type.
pub(super) fn add_content_type(package: &mut OpcPackage, part: &str, mime: &str) -> Result<()> {
    content_types(
        package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((part, mime)),
    )
}

/// The part `presentation` relates to as `kind`, made from `initial` (at
/// `default_name`, or beside it when that name is taken) when there is none.
fn ensure_part(
    package: &mut OpcPackage,
    presentation: &str,
    kind: &str,
    default_name: &str,
    mime: &str,
    initial: impl FnOnce(bool) -> String,
    strict: bool,
) -> Result<String> {
    if let Some(part) = related(package, presentation, kind)? {
        if package.part(&part).is_some() {
            return Ok(part);
        }
    }
    let part = if package.part(default_name).is_none() {
        default_name.to_string()
    } else {
        super::slides::fresh_part(package, default_name)
    };
    package.replace(&part, initial(strict).into_bytes());
    add_relationship(package, presentation, kind, &part, strict)?;
    add_content_type(package, &part, mime)?;
    Ok(part)
}

/// Inserts `fragment` as `parent`'s child named `name`, before the first
/// existing child that `order` puts after it.
pub(super) fn insert_ordered(
    xml: &str,
    nodes: &[Node],
    parent: &Node,
    name: &str,
    fragment: &str,
    order: &[&str],
) -> String {
    let rank = |n: &str| order.iter().position(|o| *o == n).unwrap_or(order.len());
    let own = rank(name);
    let before = parent
        .children
        .iter()
        .map(|&i| &nodes[i])
        .find(|n| rank(&n.name) > own && n.name != "AlternateContent");
    match before {
        Some(next) => apply_patches(
            xml,
            vec![(next.outer.start..next.outer.start, fragment.into())],
        ),
        None => super::slides::append_child(xml, parent, fragment),
    }
}

/// Replaces `parent`'s child `name` with `fragment` (removing it when
/// `fragment` is empty), or inserts it in schema order.
pub(super) fn put_child(
    xml: &str,
    nodes: &[Node],
    parent: &Node,
    name: &str,
    fragment: &str,
    order: &[&str],
) -> String {
    match parent
        .children
        .iter()
        .map(|&i| &nodes[i])
        .find(|n| n.name == name && (n.ns == 1 || n.ns == 0))
    {
        Some(existing) => apply_patches(xml, vec![(existing.outer.clone(), fragment.into())]),
        None if fragment.is_empty() => xml.to_string(),
        None => insert_ordered(xml, nodes, parent, name, fragment, order),
    }
}

fn root_strict(package: &OpcPackage, part: &str) -> Result<bool> {
    let xml = package.required_part_str(part)?;
    Ok(parse_document_nodes(&xml)?
        .first()
        .is_some_and(|root| root.strict))
}

fn color_xml(hex: &str) -> String {
    format!("<a:srgbClr xmlns:a=\"{A}\" val=\"{}\"/>", escape(hex))
}

fn valid_hex(hex: &str) -> bool {
    hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<p:showPr>` for `settings`, keeping `extension` (its old `extLst`)
/// apart from the laser colour it states.
fn show_pr(p: &str, settings: &ShowSettings, extension: Option<(&str, &[Node], &Node)>) -> String {
    let mut attrs = String::new();
    if settings.loop_until_esc || matches!(settings.show_type, ShowType::Kiosk { .. }) {
        attrs.push_str(" loop=\"1\"");
    }
    if !settings.without_narration {
        attrs.push_str(" showNarration=\"1\"");
    }
    if settings.without_animation {
        attrs.push_str(" showAnimation=\"0\"");
    }
    if !settings.use_timings {
        attrs.push_str(" useTimings=\"0\"");
    }
    let mut body = match settings.show_type {
        ShowType::Speaker => format!("<{p}present/>"),
        ShowType::Individual { scrollbar: true } => format!("<{p}browse/>"),
        ShowType::Individual { scrollbar: false } => {
            format!("<{p}browse showScrollbar=\"0\"/>")
        }
        ShowType::Kiosk {
            restart_ms: 300_000,
        } => format!("<{p}kiosk/>"),
        ShowType::Kiosk { restart_ms } => format!("<{p}kiosk restart=\"{restart_ms}\"/>"),
    };
    body.push_str(&match &settings.range {
        ShowRange::All => format!("<{p}sldAll/>"),
        ShowRange::Slides { from, to } => {
            format!("<{p}sldRg st=\"{from}\" end=\"{}\"/>", to.max(from))
        }
        ShowRange::Custom(id) => format!("<{p}custShow id=\"{id}\"/>"),
    });
    if let Some(pen) = settings.pen_color.as_deref().filter(|c| valid_hex(c)) {
        body.push_str(&format!("<{p}penClr>{}</{p}penClr>", color_xml(pen)));
    }
    let laser = settings
        .laser_color
        .as_deref()
        .filter(|c| valid_hex(c))
        .map(|color| {
            format!(
                "<{p}ext uri=\"{LASER_EXT}\"><p14:laserClr xmlns:p14=\"{P14}\">{}</p14:laserClr></{p}ext>",
                color_xml(color)
            )
        });
    match extension {
        Some((xml, nodes, list)) => {
            let laser_ext = list
                .children
                .iter()
                .map(|&i| &nodes[i])
                .find(|n| n.name == "ext" && n.attr("uri") == Some(LASER_EXT));
            let kept = match (laser_ext, laser) {
                (Some(old), Some(new)) => {
                    let mut kept = xml[list.outer.clone()].to_string();
                    let start = old.outer.start - list.outer.start;
                    let end = old.outer.end - list.outer.start;
                    kept.replace_range(start..end, &new);
                    kept
                }
                (Some(_), None) => xml[list.outer.clone()].to_string(),
                (None, Some(new)) if list.empty => format!("<{p}extLst>{new}</{p}extLst>"),
                (None, Some(new)) => {
                    let mut kept = xml[list.outer.clone()].to_string();
                    let at = list.range.end - list.outer.start;
                    kept.insert_str(at, &new);
                    kept
                }
                (None, None) => xml[list.outer.clone()].to_string(),
            };
            body.push_str(&kept);
        }
        None => {
            if let Some(laser) = laser {
                body.push_str(&format!("<{p}extLst>{laser}</{p}extLst>"));
            }
        }
    }
    format!("<{p}showPr{attrs}>{body}</{p}showPr>")
}

fn empty_pres_props(strict: bool) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentationPr xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\"/>",
        super::drawing_ns(strict),
        r_ns(strict),
        p_ns(strict)
    )
}

/// Opens `presProps.xml` (making it when the deck has none) for an edit of
/// its root.
fn edit_pres_props(
    bytes: &[u8],
    edit: impl FnOnce(&str, &[Node]) -> Result<String>,
) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = presentation_part(&package)?;
    let strict = root_strict(&package, &presentation)?;
    let existed = related(&package, &presentation, "presProps")?
        .is_some_and(|part| package.part(&part).is_some());
    let part = ensure_part(
        &mut package,
        &presentation,
        "presProps",
        "ppt/presProps.xml",
        PRES_PROPS_MIME,
        empty_pres_props,
        strict,
    )?;
    let xml = package.required_part_str(&part)?;
    let nodes = parse_document_nodes(&xml)?;
    if !nodes.first().is_some_and(|n| n.is(1, "presentationPr")) {
        return Err(xml_error("expected presentation properties"));
    }
    let updated = edit(&xml, &nodes)?;
    if updated == xml && existed {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, updated.into_bytes());
    Ok(package.write()?)
}

/// Rewrites `<p:showPr>` to `settings`.
pub(crate) fn set_show_settings(bytes: &[u8], settings: &ShowSettings) -> Result<Vec<u8>> {
    if let ShowRange::Slides { from, to } = settings.range {
        if from == 0 || to < from {
            return Err(WriteError::InvalidSlideOrder);
        }
    }
    edit_pres_props(bytes, |xml, nodes| {
        let root = &nodes[0];
        let p = prefix(root);
        let existing = child(nodes, root, 1, "showPr");
        let extension = existing
            .and_then(|show| child(nodes, show, 1, "extLst").map(|list| (xml, nodes, list)));
        let fragment = show_pr(&p, settings, extension);
        Ok(put_child(
            xml,
            nodes,
            root,
            "showPr",
            &fragment,
            PRES_PROPS_ORDER,
        ))
    })
}

/// `<p:prnPr prnWhat>`: what 印刷 prints — `slides`, `handouts1`…`handouts9`,
/// `notes` or `outline`.
pub(crate) fn set_print_what(bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    const VALID: &[&str] = &[
        "slides",
        "handouts1",
        "handouts2",
        "handouts3",
        "handouts4",
        "handouts6",
        "handouts9",
        "notes",
        "outline",
    ];
    if !VALID.contains(&what) {
        return Err(xml_error(format!("unknown print layout {what}")));
    }
    edit_pres_props(bytes, |xml, nodes| {
        let root = &nodes[0];
        let p = prefix(root);
        Ok(match child(nodes, root, 1, "prnPr") {
            Some(print) => {
                let opening = edit_tag(&xml[print.opening.clone()], &[("prnWhat", what.into())])?;
                apply_patches(xml, vec![(print.opening.clone(), opening)])
            }
            None => insert_ordered(
                xml,
                nodes,
                root,
                "prnPr",
                &format!("<{p}prnPr prnWhat=\"{what}\"/>"),
                PRES_PROPS_ORDER,
            ),
        })
    })
}

/// Reads `prnWhat`, `slides` when the deck states none.
pub fn print_what(bytes: &[u8]) -> Result<String> {
    let package = OpcPackage::open(bytes)?;
    let presentation = presentation_part(&package)?;
    let Some(part) = related(&package, &presentation, "presProps")? else {
        return Ok("slides".into());
    };
    let Some(xml) = package.part_str(&part)? else {
        return Ok("slides".into());
    };
    let nodes = parse_document_nodes(&xml)?;
    Ok(nodes
        .iter()
        .find(|n| n.is(1, "prnPr"))
        .and_then(|n| n.attr("prnWhat"))
        .unwrap_or("slides")
        .to_string())
}

/// Each slide's relationship id, in presentation order.
fn slide_relationship_ids(nodes: &[Node]) -> Vec<String> {
    let Some(list) = child(nodes, &nodes[0], 1, "sldIdLst") else {
        return Vec::new();
    };
    list.children
        .iter()
        .map(|&i| &nodes[i])
        .filter(|n| n.is(1, "sldId"))
        .filter_map(|n| super::slides::rel_attr(nodes, n).map(|(_, id)| id.to_string()))
        .collect()
}

/// Rewrites `<p:custShowLst>` to `shows` (removing it when there are none).
pub(crate) fn set_custom_shows(bytes: &[u8], shows: &[CustomShow]) -> Result<Vec<u8>> {
    let mut ids = BTreeSet::new();
    for show in shows {
        if show.name.trim().is_empty() || !ids.insert(show.id) {
            return Err(xml_error("custom shows need distinct ids and a name"));
        }
    }
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = presentation_part(&package)?;
    let xml = package.required_part_str(&presentation)?;
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(1, "presentation"))
        .ok_or_else(|| xml_error("expected a presentation"))?;
    let slide_ids = slide_relationship_ids(&nodes);
    let p = prefix(root);
    let strict = root.strict;
    let fragment = if shows.is_empty() {
        String::new()
    } else {
        let mut list = format!("<{p}custShowLst xmlns:r=\"{}\">", r_ns(strict));
        for show in shows {
            list.push_str(&format!(
                "<{p}custShow name=\"{}\" id=\"{}\"><{p}sldLst>",
                escape(&show.name),
                show.id
            ));
            for &slide in &show.slides {
                let id = slide_ids.get(slide).ok_or(WriteError::NoSlide(slide))?;
                list.push_str(&format!("<{p}sld r:id=\"{}\"/>", escape(id)));
            }
            list.push_str(&format!("</{p}sldLst></{p}custShow>"));
        }
        list.push_str(&format!("</{p}custShowLst>"));
        list
    };
    let updated = put_child(
        &xml,
        &nodes,
        root,
        "custShowLst",
        &fragment,
        PRESENTATION_ORDER,
    );
    if updated == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&presentation, updated.into_bytes());
    Ok(package.write()?)
}

/// Sets (or, with `None`, removes) each listed slide's 自動的に切り替え time.
pub(crate) fn set_slide_advances(
    bytes: &[u8],
    advances: &[(usize, Option<u32>)],
) -> Result<Vec<u8>> {
    let slides = jade_pptx_reader::read_pptx(bytes)?.slides;
    let mut package = OpcPackage::open_preserving(bytes)?;
    let mut changed = false;
    for &(index, after) in advances {
        let part = &slides
            .get(index)
            .ok_or(WriteError::NoSlide(index))?
            .part_name;
        let xml = package.required_part_str(part)?;
        let nodes = parse_document_nodes(&xml)?;
        let root = nodes
            .first()
            .filter(|n| n.is(1, "sld"))
            .ok_or_else(|| xml_error("expected a slide"))?;
        let transitions: Vec<&Node> = nodes.iter().filter(|n| n.is(1, "transition")).collect();
        let updated = if transitions.is_empty() {
            match after {
                None => xml.clone(),
                Some(ms) => {
                    let p = prefix(root);
                    let fragment = format!("<{p}transition advTm=\"{ms}\"/>");
                    let after_node = child(&nodes, root, 1, "clrMapOvr")
                        .or_else(|| child(&nodes, root, 1, "cSld"))
                        .ok_or_else(|| xml_error("slide without cSld"))?;
                    apply_patches(
                        &xml,
                        vec![(after_node.outer.end..after_node.outer.end, fragment)],
                    )
                }
            }
        } else {
            let mut patches = Vec::new();
            for node in transitions {
                let tag = &xml[node.opening.clone()];
                let opening = match after {
                    Some(ms) => edit_tag(tag, &[("advTm", ms.to_string())])?,
                    None => remove_tag_attributes(tag, &["advTm"])?,
                };
                patches.push((node.opening.clone(), opening));
            }
            apply_patches(&xml, patches)
        };
        if updated != xml {
            changed = true;
            package.replace(part, updated.into_bytes());
        }
    }
    if !changed {
        return Ok(bytes.to_vec());
    }
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

fn empty_view_props(strict: bool) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:viewPr xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\"><p:gridSpacing cx=\"72000\" cy=\"72000\"/></p:viewPr>",
        super::drawing_ns(strict),
        r_ns(strict),
        p_ns(strict)
    )
}

fn guide_list(p: &str, props: &ViewProps) -> String {
    if props.guides.is_empty() {
        return format!("<{p}guideLst/>");
    }
    let mut list = format!("<{p}guideLst>");
    for guide in &props.guides {
        let position = (guide.position_emu as f64
            / jade_pptx_reader::view_props::GUIDE_UNITS_PER_EMU)
            .round() as i64;
        if guide.horizontal {
            list.push_str(&format!("<{p}guide orient=\"horz\" pos=\"{position}\"/>"));
        } else {
            list.push_str(&format!("<{p}guide pos=\"{position}\"/>"));
        }
    }
    list.push_str(&format!("</{p}guideLst>"));
    list
}

fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

/// Rewrites `viewProps.xml`'s grid, guides and last view to `props`,
/// keeping its other views' settings.
pub(crate) fn set_view_props(bytes: &[u8], props: &ViewProps) -> Result<Vec<u8>> {
    let (cx, cy) = props.grid_spacing_emu;
    if cx <= 0 || cy <= 0 {
        return Err(xml_error("grid spacing must be positive"));
    }
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = presentation_part(&package)?;
    let strict = root_strict(&package, &presentation)?;
    let existed = related(&package, &presentation, "viewProps")?
        .is_some_and(|part| package.part(&part).is_some());
    let part = ensure_part(
        &mut package,
        &presentation,
        "viewProps",
        "ppt/viewProps.xml",
        VIEW_PROPS_MIME,
        empty_view_props,
        strict,
    )?;
    let mut xml = package.required_part_str(&part)?;
    let original = xml.clone();

    // lastView on the root; the default (sldView) is left unstated.
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(1, "viewPr"))
        .ok_or_else(|| xml_error("expected view properties"))?;
    let p = prefix(root);
    let tag = &xml[root.opening.clone()];
    let opening = if props.last_view == jade_pptx_reader::LastView::Normal {
        remove_tag_attributes(tag, &["lastView"])?
    } else {
        edit_tag(tag, &[("lastView", props.last_view.attr().into())])?
    };
    xml = apply_patches(&xml, vec![(root.opening.clone(), opening)]);

    // The grid's spacing.
    let nodes = parse_document_nodes(&xml)?;
    xml = put_child(
        &xml,
        &nodes,
        &nodes[0],
        "gridSpacing",
        &format!("<{p}gridSpacing cx=\"{cx}\" cy=\"{cy}\"/>"),
        VIEW_PROPS_ORDER,
    );

    // The slide view's snapping, guides shown and guides.
    let nodes = parse_document_nodes(&xml)?;
    let attrs = [
        ("snapToGrid", flag(props.snap_to_grid)),
        ("snapToObjects", flag(props.snap_to_objects)),
        ("showGuides", flag(props.show_guides)),
    ];
    let common = child(&nodes, &nodes[0], 1, "slideViewPr")
        .and_then(|view| child(&nodes, view, 1, "cSldViewPr"));
    xml = match common {
        Some(common) => {
            let mut patches = vec![(
                common.opening.clone(),
                edit_tag(&xml[common.opening.clone()], &attrs)?,
            )];
            let list = guide_list(&p, props);
            match child(&nodes, common, 1, "guideLst") {
                Some(old) => patches.push((old.outer.clone(), list)),
                None => {
                    let at = child(&nodes, common, 1, "cViewPr")
                        .map(|n| n.outer.end)
                        .unwrap_or(common.range.end);
                    if common.empty {
                        // `<p:cSldViewPr/>` has no room for children: rebuild it.
                        let opening = edit_tag(&xml[common.opening.clone()], &attrs)?;
                        let opening = opening.trim_end_matches("/>").trim_end().to_string() + ">";
                        patches = vec![(
                            common.outer.clone(),
                            format!("{opening}{}{list}</{p}cSldViewPr>", c_view_pr(&p)),
                        )];
                    } else {
                        patches.push((at..at, list));
                    }
                }
            }
            apply_patches(&xml, patches)
        }
        None => {
            let fragment = format!(
                "<{p}slideViewPr><{p}cSldViewPr snapToGrid=\"{}\" snapToObjects=\"{}\" showGuides=\"{}\">{}{}</{p}cSldViewPr></{p}slideViewPr>",
                attrs[0].1,
                attrs[1].1,
                attrs[2].1,
                c_view_pr(&p),
                guide_list(&p, props)
            );
            put_child(
                &xml,
                &nodes,
                &nodes[0],
                "slideViewPr",
                &fragment,
                VIEW_PROPS_ORDER,
            )
        }
    };
    if xml == original && existed {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, xml.into_bytes());
    Ok(package.write()?)
}

/// The `cViewPr` a slide view must carry: 100 %, at the origin.
fn c_view_pr(p: &str) -> String {
    format!(
        "<{p}cViewPr varScale=\"1\"><{p}scale><a:sx xmlns:a=\"{A}\" n=\"100\" d=\"100\"/><a:sy xmlns:a=\"{A}\" n=\"100\" d=\"100\"/></{p}scale><{p}origin x=\"0\" y=\"0\"/></{p}cViewPr>"
    )
}
