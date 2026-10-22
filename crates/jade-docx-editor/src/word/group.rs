//! グループ化 and グループ解除 over any [`WordSession`]: several floating
//! shapes and pictures become one WordprocessingGroup (`wpg:wgp`) anchored
//! where the first was, each keeping its own look inside; a group comes
//! apart into floating objects where its members were on the page.

use crate::commands::delete_object::DeleteObjectCommand;
use crate::commands::edit_drawing::replace_drawing_command;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::Command;
use crate::document::TextBoxShape;
use crate::docx_xml::{XmlElement, XmlNode};
use crate::drawing_xml::{self, child, child_mut, descendant, elements, get_attr, set_attr};
use crate::word_document::objects::{element_xml, parse_element};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, ShapeInsertSpec};

use super::objects::{fragment, Sequence, WordObjects};
use super::picture::{PlacedObject, Wrap};
use super::shapes::{InsertThen, Inserted};
use super::WordSession;

const EMU: f64 = 9525.0;

fn emu(px: f64) -> i64 {
    (px * EMU).round() as i64
}

/// The object inside a drawing's graphic (`wps:wsp`, `pic:pic`, `wpg:wgp`).
fn graphic_object(drawing: &XmlElement) -> Option<&XmlElement> {
    let data = descendant(drawing, "a:graphicData")?;
    elements(data)
        .find(|element| matches!(element.name.as_str(), "wps:wsp" | "pic:pic" | "wpg:wgp"))
}

/// A member of a group, as it is written inside `wpg:wgp`: a shape keeps
/// `wps:wsp` but names itself with `wps:cNvPr`, a group becomes `wpg:grpSp`.
fn as_member(
    object: &XmlElement,
    doc_pr: Option<&XmlElement>,
    (x, y, width, height): (i64, i64, i64, i64),
) -> Result<XmlElement, String> {
    let mut member = object.clone();
    match member.name.as_str() {
        "wps:wsp" => {
            if child(&member, "wps:cNvPr").is_none() {
                let id = doc_pr.and_then(|pr| get_attr(pr, "id")).unwrap_or("1");
                let name = doc_pr
                    .and_then(|pr| get_attr(pr, "name"))
                    .unwrap_or("Shape");
                member.children.insert(
                    0,
                    XmlNode::Element(fragment(&format!(
                        r#"<wps:cNvPr id="{id}" name="{}"/>"#,
                        name.replace('"', "&quot;")
                    ))?),
                );
            }
        }
        "wpg:wgp" => member.name = "wpg:grpSp".to_string(),
        _ => {}
    }
    let sp_pr_name = match member.name.as_str() {
        "pic:pic" => "pic:spPr",
        "wpg:grpSp" => "wpg:grpSpPr",
        _ => "wps:spPr",
    };
    let sp_pr = child_mut(&mut member, sp_pr_name).ok_or("the object has no shape properties")?;
    if child(sp_pr, "a:xfrm").is_none() {
        sp_pr
            .children
            .insert(0, XmlNode::Element(fragment("<a:xfrm/>")?));
    }
    let xfrm = child_mut(sp_pr, "a:xfrm").expect("added above");
    xfrm.children.retain(|node| {
        !matches!(node, XmlNode::Element(element) if element.name == "a:off" || element.name == "a:ext")
    });
    xfrm.children.insert(
        0,
        XmlNode::Element(fragment(&format!(r#"<a:off x="{x}" y="{y}"/>"#))?),
    );
    xfrm.children.insert(
        1,
        XmlNode::Element(fragment(&format!(
            r#"<a:ext cx="{width}" cy="{height}"/>"#
        ))?),
    );
    Ok(member)
}

/// The groups' tools, for every session.
pub trait WordGroups: WordSession + WordObjects {
    /// グループ化: `objects` (two or more floating ones, with their boxes on
    /// the page) become one group in place of the first.
    fn group_objects(&mut self, objects: &[PlacedObject]) -> Result<(), String> {
        let mut members: Vec<PlacedObject> = objects
            .iter()
            .copied()
            .filter(|object| {
                self.object_format(object.target)
                    .is_some_and(|format| format.wrap != Wrap::Inline)
            })
            .collect();
        if members.len() < 2 {
            return Err("select two or more floating objects to group".into());
        }
        if members.iter().any(|object| object.page != members[0].page) {
            return Err("objects on different pages cannot be grouped".into());
        }
        members.sort_by_key(|object| (object.target.paragraph_index, object.target.run_index));
        let left = members.iter().map(|o| o.x).fold(f64::INFINITY, f64::min);
        let top = members.iter().map(|o| o.y).fold(f64::INFINITY, f64::min);
        let right = members
            .iter()
            .map(|o| o.x + o.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = members
            .iter()
            .map(|o| o.y + o.height)
            .fold(f64::NEG_INFINITY, f64::max);
        let (width, height) = (emu(right - left), emu(bottom - top));
        let first = members[0];
        let others: Vec<PlacedObject> = members[1..].to_vec();
        // Each member's drawing as the package has it, read before any of
        // them goes: removing a text box renumbers the paragraphs after it.
        let mut drawings = Vec::new();
        for member in &members {
            drawings.push(
                package_drawing(self.word_document(), member.target)
                    .ok_or("an object's drawing is not available")?,
            );
        }
        let all: Vec<(PlacedObject, XmlElement)> = members.iter().copied().zip(drawings).collect();
        let group = replace_drawing_command(
            "group_objects",
            first.target,
            move |document, drawing| {
                let mut children = Vec::new();
                for (member, own) in &all {
                    let member = *member;
                    let object = graphic_object(own).ok_or("an object cannot be grouped")?;
                    let doc_pr = descendant(own, "wp:docPr");
                    children.push(as_member(
                        object,
                        doc_pr,
                        (
                            emu(member.x - left),
                            emu(member.y - top),
                            emu(member.width),
                            emu(member.height),
                        ),
                    )?);
                }
                let id = document.new_drawing_object_id();
                let members_xml: String = children.iter().map(element_xml).collect();
                let group = parse_element(&format!(
                    r#"<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{width}" cy="{height}"/><a:chOff x="0" y="0"/><a:chExt cx="{width}" cy="{height}"/></a:xfrm></wpg:grpSpPr>{members_xml}</wpg:wgp></a:graphicData></a:graphic>"#
                ))?;
                let frame = drawing_xml::container_mut(drawing).ok_or("the object has no frame")?;
                frame.children.retain(|node| {
                !matches!(node, XmlNode::Element(element) if element.name == "a:graphic")
            });
                frame.children.push(XmlNode::Element(group));
                for (name, value) in [("cx", width), ("cy", height)] {
                    if let Some(extent) = child_mut(frame, "wp:extent") {
                        set_attr(extent, name, Some(value.to_string()));
                    }
                }
                // The group sits where its members' box is on the page.
                let position_h = fragment(&format!(
                    r#"<wp:positionH relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionH>"#,
                    emu(left)
                ))?;
                let position_v = fragment(&format!(
                    r#"<wp:positionV relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionV>"#,
                    emu(top)
                ))?;
                drawing_xml::put_child(
                    frame,
                    position_h,
                    &["wp:positionH"],
                    &["wp:positionV", "wp:extent"],
                );
                drawing_xml::put_child(frame, position_v, &["wp:positionV"], &["wp:extent"]);
                if let Some(doc_pr) = child_mut(frame, "wp:docPr") {
                    set_attr(doc_pr, "id", Some(id.to_string()));
                    set_attr(doc_pr, "name", Some(format!("グループ化 {id}")));
                    set_attr(doc_pr, "descr", None);
                }
                frame.children.retain(|node| {
                !matches!(node, XmlNode::Element(element) if element.name == "wp:cNvGraphicFramePr")
            });
                Ok(())
            },
        );
        // The others go first, last first so the earlier ones keep their
        // places; the first, earliest in the document, keeps its own.
        let mut sequence: Vec<Box<dyn Command>> = Vec::new();
        let mut removals: Vec<_> = others.iter().map(|object| object.target).collect();
        removals.sort_by_key(|target| (target.paragraph_index, target.run_index));
        for target in removals.into_iter().rev() {
            sequence.push(Box::new(DeleteObjectCommand::new(target)));
        }
        sequence.push(Box::new(group));
        self.execute_command(Box::new(Sequence::new("group_objects", sequence)), true)
    }

    /// グループ解除: the group's members become floating objects where they
    /// are on the page, anchored at the group's paragraph.
    fn ungroup_object(&mut self, group: PlacedObject) -> Result<(), String> {
        if group.target.kind != FloatingObjectKind::Image {
            return Err("the object is not a group".into());
        }
        let drawing = package_drawing(self.word_document(), group.target)
            .ok_or("the group's drawing is not available")?;
        let wgp = descendant(&drawing, "wpg:wgp").ok_or("the object is not a group")?;
        let xfrm = child(wgp, "wpg:grpSpPr")
            .and_then(|properties| child(properties, "a:xfrm"))
            .ok_or("the group has no frame")?;
        let number = |element: Option<&XmlElement>, name: &str| {
            element
                .and_then(|element| get_attr(element, name))
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        let child_offset = (
            number(child(xfrm, "a:chOff"), "x"),
            number(child(xfrm, "a:chOff"), "y"),
        );
        let child_extent = (
            number(child(xfrm, "a:chExt"), "cx").max(1.0),
            number(child(xfrm, "a:chExt"), "cy").max(1.0),
        );
        let (scale_x, scale_y) = (
            group.width / (child_extent.0 / EMU),
            group.height / (child_extent.1 / EMU),
        );
        let frame = drawing_xml::container(&drawing).ok_or("the group has no frame")?;
        let mut new_drawings = Vec::new();
        for member in elements(wgp) {
            let properties_name = match member.name.as_str() {
                "wps:wsp" => "wps:spPr",
                "pic:pic" => "pic:spPr",
                "wpg:grpSp" => "wpg:grpSpPr",
                _ => continue,
            };
            let member_xfrm = child(member, properties_name)
                .and_then(|properties| child(properties, "a:xfrm"))
                .ok_or("a member has no frame")?;
            let x = group.x
                + (number(child(member_xfrm, "a:off"), "x") - child_offset.0) / EMU * scale_x;
            let y = group.y
                + (number(child(member_xfrm, "a:off"), "y") - child_offset.1) / EMU * scale_y;
            let width = number(child(member_xfrm, "a:ext"), "cx") / EMU * scale_x;
            let height = number(child(member_xfrm, "a:ext"), "cy") / EMU * scale_y;
            let mut object = member.clone();
            let (uri, name, id) = match object.name.as_str() {
                "pic:pic" => (
                    "http://schemas.openxmlformats.org/drawingml/2006/picture",
                    descendant(member, "pic:cNvPr")
                        .and_then(|pr| get_attr(pr, "name"))
                        .unwrap_or("図")
                        .to_string(),
                    descendant(member, "pic:cNvPr")
                        .and_then(|pr| get_attr(pr, "id"))
                        .unwrap_or("0")
                        .to_string(),
                ),
                "wpg:grpSp" => {
                    object.name = "wpg:wgp".into();
                    (
                        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
                        "グループ化".to_string(),
                        "0".to_string(),
                    )
                }
                _ => {
                    let name = child(member, "wps:cNvPr")
                        .and_then(|pr| get_attr(pr, "name"))
                        .unwrap_or("図形")
                        .to_string();
                    let id = child(member, "wps:cNvPr")
                        .and_then(|pr| get_attr(pr, "id"))
                        .unwrap_or("0")
                        .to_string();
                    object.children.retain(|node| {
                        !matches!(node, XmlNode::Element(element) if element.name == "wps:cNvPr")
                    });
                    (
                        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
                        name,
                        id,
                    )
                }
            };
            // The member's frame becomes its own extent at its new place.
            if let Some(properties) = child_mut(&mut object, properties_name) {
                if let Some(xfrm) = child_mut(properties, "a:xfrm") {
                    if let Some(off) = child_mut(xfrm, "a:off") {
                        set_attr(off, "x", Some("0".into()));
                        set_attr(off, "y", Some("0".into()));
                    }
                    if let Some(ext) = child_mut(xfrm, "a:ext") {
                        set_attr(ext, "cx", Some(emu(width).max(1).to_string()));
                        set_attr(ext, "cy", Some(emu(height).max(1).to_string()));
                    }
                }
            }
            let attributes: String = frame
                .attributes
                .iter()
                .map(|attribute| {
                    format!(
                        r#" {}="{}""#,
                        attribute.name,
                        attribute.value.replace('"', "&quot;")
                    )
                })
                .collect();
            let xml = format!(
                r#"<w:drawing><wp:anchor{attributes}><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx="{}" cy="{}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>{}<wp:docPr id="{id}" name="{}"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="{uri}">{}</a:graphicData></a:graphic></wp:anchor></w:drawing>"#,
                emu(x),
                emu(y),
                emu(width).max(1),
                emu(height).max(1),
                drawing_xml::child_any(
                    frame,
                    &[
                        "wp:wrapNone",
                        "wp:wrapSquare",
                        "wp:wrapTight",
                        "wp:wrapThrough",
                        "wp:wrapTopAndBottom"
                    ]
                )
                .map(element_xml)
                .unwrap_or_else(|| "<wp:wrapNone/>".into()),
                name.replace('"', "&quot;"),
                element_xml(&object),
            );
            new_drawings.push(parse_element(&xml)?);
        }
        if new_drawings.is_empty() {
            return Err("the group is empty".into());
        }
        // Each member goes in as a placeholder shape at the group's anchor,
        // then takes its own drawing; then the group goes.
        let anchor = group.target;
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        for drawing in new_drawings {
            commands.push(Box::new(InsertThen::new(
                Inserted::Shape(InsertShapeCommand::new(
                    anchor.paragraph_index,
                    0,
                    ShapeInsertSpec {
                        shape: TextBoxShape::Rectangle,
                        width_px: 12.0,
                        height_px: 12.0,
                        fill_color: None,
                        border_color: None,
                        border_width_px: 0.0,
                        text: String::new(),
                        text_color: None,
                        text_font_size_pt: None,
                        text_bold: None,
                        text_italic: None,
                        text_alignment: None,
                    },
                )),
                move |target| {
                    let mut drawing = Some(drawing.clone());
                    vec![Box::new(replace_drawing_command(
                        "ungroup_member",
                        target,
                        move |_, current| {
                            if let Some(drawing) = drawing.take() {
                                *current = drawing;
                            }
                            Ok(())
                        },
                    )) as Box<dyn Command>]
                },
            )));
        }
        // The group has moved along by one run per member inserted before it.
        let shifted = FloatingObjectTarget {
            run_index: anchor.run_index + commands.len(),
            ..anchor
        };
        commands.push(Box::new(DeleteObjectCommand::new(shifted)));
        self.execute_command(Box::new(Sequence::new("ungroup_object", commands)), false)
    }
}

impl<T: WordSession + ?Sized> WordGroups for T {}

/// The `w:drawing` of the object at `target` as the package has it.
fn package_drawing(
    document: &crate::word_document::WordDocument,
    target: FloatingObjectTarget,
) -> Option<XmlElement> {
    let run = document
        .paragraph(target.paragraph_index)?
        .runs
        .get(target.run_index)?;
    let element = document.raw_document.as_ref()?.element(run.node_id?)?;
    descendant(element, "w:drawing").cloned()
}

/// Members' frames as fragments, for tests.
pub fn member_frames(drawing: &XmlElement) -> Vec<(i64, i64, i64, i64)> {
    let Some(wgp) = descendant(drawing, "wpg:wgp") else {
        return Vec::new();
    };
    elements(wgp)
        .filter_map(|member| {
            let xfrm = descendant(member, "a:xfrm")?;
            let number = |element: Option<&XmlElement>, name: &str| {
                element
                    .and_then(|element| get_attr(element, name))
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0)
            };
            Some((
                number(child(xfrm, "a:off"), "x"),
                number(child(xfrm, "a:off"), "y"),
                number(child(xfrm, "a:ext"), "cx"),
                number(child(xfrm, "a:ext"), "cy"),
            ))
        })
        .collect()
}
