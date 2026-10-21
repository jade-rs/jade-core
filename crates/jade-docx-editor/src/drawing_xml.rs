//! Editing a drawing's XML (`w:drawing`) in place.
//!
//! A picture, shape or group carries much more than the model reads: effects,
//! line formats, theme references, extension lists, a SmartArt's relationship
//! ids. Rebuilding its drawing from the model on every move or wrap change
//! threw all of that away. These helpers patch what an edit changed into the
//! drawing the package already has, and leave the rest as it was.

use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode};

/// EMU per CSS pixel (914400 per inch, 96 pixels per inch).
pub const EMU_PER_PX: f64 = 9525.0;

pub fn px_to_emu(value: f64) -> i64 {
    (value * EMU_PER_PX).round() as i64
}

pub fn emu_to_px(value: f64) -> f64 {
    value / EMU_PER_PX
}

pub fn attr(name: &str, value: impl ToString) -> XmlAttribute {
    XmlAttribute {
        name: name.to_string(),
        value: value.to_string(),
    }
}

/// The value of attribute `name`.
pub fn get_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

/// Sets attribute `name`, or removes it with `None`.
pub fn set_attr(element: &mut XmlElement, name: &str, value: Option<String>) {
    match value {
        Some(value) => match element
            .attributes
            .iter_mut()
            .find(|attribute| attribute.name == name)
        {
            Some(attribute) => attribute.value = value,
            None => element.attributes.push(attr(name, value)),
        },
        None => element.attributes.retain(|attribute| attribute.name != name),
    }
}

pub fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XmlNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

pub fn child_mut<'a>(element: &'a mut XmlElement, name: &str) -> Option<&'a mut XmlElement> {
    element.children.iter_mut().find_map(|node| match node {
        XmlNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// The first child whose name is one of `names`.
pub fn child_any<'a>(element: &'a XmlElement, names: &[&str]) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XmlNode::Element(child) if names.contains(&child.name.as_str()) => Some(child),
        _ => None,
    })
}

pub fn child_index(element: &XmlElement, names: &[&str]) -> Option<usize> {
    element.children.iter().position(
        |node| matches!(node, XmlNode::Element(child) if names.contains(&child.name.as_str())),
    )
}

/// Every element child, in order.
pub fn elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|node| match node {
        XmlNode::Element(child) => Some(child),
        _ => None,
    })
}

/// The first descendant named `name`, depth first.
pub fn descendant<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    for node in &element.children {
        if let XmlNode::Element(child) = node {
            if child.name == name {
                return Some(child);
            }
            if let Some(found) = descendant(child, name) {
                return Some(found);
            }
        }
    }
    None
}

/// The first descendant named `name`, depth first, mutably.
pub fn descendant_mut<'a>(element: &'a mut XmlElement, name: &str) -> Option<&'a mut XmlElement> {
    let index = element.children.iter().position(|node| match node {
        XmlNode::Element(child) => child.name == name || descendant(child, name).is_some(),
        _ => false,
    })?;
    let XmlNode::Element(child) = &mut element.children[index] else {
        return None;
    };
    if child.name == name {
        Some(child)
    } else {
        descendant_mut(child, name)
    }
}

/// Removes every child named one of `names`.
pub fn remove_children(element: &mut XmlElement, names: &[&str]) {
    element.children.retain(
        |node| !matches!(node, XmlNode::Element(child) if names.contains(&child.name.as_str())),
    );
}

/// Puts `new` where the first child named one of `replaces` is, removing the
/// others of those names; or, when there is none, before the first child named
/// one of `before` (at the end when there is none of those either).
pub fn put_child(element: &mut XmlElement, new: XmlElement, replaces: &[&str], before: &[&str]) {
    if let Some(index) = child_index(element, replaces) {
        element.children[index] = XmlNode::Element(new);
        let mut seen = false;
        element.children.retain(|node| match node {
            XmlNode::Element(child) if replaces.contains(&child.name.as_str()) => {
                if seen {
                    false
                } else {
                    seen = true;
                    true
                }
            }
            _ => true,
        });
        // The kept one is the replacement: it sits first among its kind
        // because it took the first one's place.
        return;
    }
    let at = child_index(element, before).unwrap_or(element.children.len());
    element.children.insert(at, XmlNode::Element(new));
}

/// A detached element with fresh ids, its children included.
pub fn element(
    doc: &mut DocxXmlDocument,
    name: &str,
    attributes: Vec<XmlAttribute>,
    children: Vec<XmlElement>,
) -> XmlElement {
    doc.create_element(
        name,
        attributes,
        children.into_iter().map(XmlNode::Element).collect(),
    )
}

/// Gives every element in `element`'s subtree a fresh id from `doc`, for a
/// subtree built outside it (a parsed fragment).
pub fn adopt(doc: &mut DocxXmlDocument, element: &mut XmlElement) {
    element.id = doc.allocate_node_id();
    for node in &mut element.children {
        if let XmlNode::Element(child) = node {
            adopt(doc, child);
        }
    }
}

/// The anchor (`wp:anchor`) or inline (`wp:inline`) container of a drawing.
pub fn container(drawing: &XmlElement) -> Option<&XmlElement> {
    child_any(drawing, &["wp:anchor", "wp:inline"])
}

pub fn container_mut(drawing: &mut XmlElement) -> Option<&mut XmlElement> {
    let index = child_index(drawing, &["wp:anchor", "wp:inline"])?;
    match &mut drawing.children[index] {
        XmlNode::Element(element) => Some(element),
        _ => None,
    }
}

/// The shape-properties element of the graphic's object: `pic:spPr`,
/// `wps:spPr`, `wpg:grpSpPr`, whichever the drawing holds.
pub fn shape_properties_mut(drawing: &mut XmlElement) -> Option<&mut XmlElement> {
    let graphic_data = descendant_mut(drawing, "a:graphicData")?;
    let object_index = child_index(graphic_data, &["pic:pic", "wps:wsp", "wpg:wgp", "wpc:wpc"])?;
    let XmlNode::Element(object) = &mut graphic_data.children[object_index] else {
        return None;
    };
    let index = child_index(object, &["pic:spPr", "wps:spPr", "wpg:grpSpPr"])?;
    match &mut object.children[index] {
        XmlNode::Element(element) => Some(element),
        _ => None,
    }
}

pub fn shape_properties(drawing: &XmlElement) -> Option<&XmlElement> {
    let graphic_data = descendant(drawing, "a:graphicData")?;
    let object = child_any(graphic_data, &["pic:pic", "wps:wsp", "wpg:wgp", "wpc:wpc"])?;
    child_any(object, &["pic:spPr", "wps:spPr", "wpg:grpSpPr"])
}

/// The children of `wp:anchor`, in the schema's order, by name.
pub const ANCHOR_ORDER: &[&str] = &[
    "wp:simplePos",
    "wp:positionH",
    "wp:positionV",
    "wp:extent",
    "wp:effectExtent",
    "wp:wrapNone",
    "wp:wrapSquare",
    "wp:wrapTight",
    "wp:wrapThrough",
    "wp:wrapTopAndBottom",
    "wp:docPr",
    "wp:cNvGraphicFramePr",
    "a:graphic",
];

/// The children of `a:spPr`-like shape properties, in the schema's order.
pub const SHAPE_PROPERTIES_ORDER: &[&str] = &[
    "a:xfrm",
    "a:custGeom",
    "a:prstGeom",
    "a:noFill",
    "a:solidFill",
    "a:gradFill",
    "a:blipFill",
    "a:pattFill",
    "a:grpFill",
    "a:ln",
    "a:effectLst",
    "a:effectDag",
    "a:scene3d",
    "a:sp3d",
    "a:extLst",
];

/// The names in `order` after `name`, to insert `name` before the first of
/// them a parent already has.
pub fn after<'a>(order: &'a [&'a str], name: &str) -> &'a [&'a str] {
    match order.iter().position(|entry| *entry == name) {
        Some(index) => &order[index + 1..],
        None => &[],
    }
}

/// Puts `new` into shape properties `sp_pr` in the schema's place, in place of
/// any of `replaces`.
pub fn put_shape_property(sp_pr: &mut XmlElement, new: XmlElement, replaces: &[&str]) {
    let name = new.name.clone();
    let before = after(SHAPE_PROPERTIES_ORDER, replaces.last().copied().unwrap_or(&name));
    put_child(sp_pr, new, replaces, before);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> XmlElement {
        DocxXmlDocument::parse(xml).unwrap().root().clone()
    }

    #[test]
    fn a_child_takes_its_schema_place() {
        let mut doc = DocxXmlDocument::parse("<r/>").unwrap();
        let mut sp_pr = parse("<pic:spPr><a:xfrm/><a:prstGeom/><a:effectLst/></pic:spPr>");
        let line = element(&mut doc, "a:ln", Vec::new(), Vec::new());
        put_shape_property(&mut sp_pr, line, &["a:ln"]);
        let names: Vec<_> = elements(&sp_pr).map(|child| child.name.clone()).collect();
        assert_eq!(names, ["a:xfrm", "a:prstGeom", "a:ln", "a:effectLst"]);
        let fill = element(&mut doc, "a:solidFill", Vec::new(), Vec::new());
        put_shape_property(&mut sp_pr, fill, &["a:noFill", "a:solidFill", "a:gradFill"]);
        let names: Vec<_> = elements(&sp_pr).map(|child| child.name.clone()).collect();
        assert_eq!(
            names,
            ["a:xfrm", "a:prstGeom", "a:solidFill", "a:ln", "a:effectLst"]
        );
    }

    #[test]
    fn descendants_are_found_depth_first() {
        let mut drawing = parse(
            "<w:drawing><wp:anchor><a:graphic><a:graphicData><pic:pic><pic:spPr><a:xfrm rot=\"1\"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing>",
        );
        let sp_pr = shape_properties_mut(&mut drawing).unwrap();
        assert_eq!(sp_pr.name, "pic:spPr");
        let xfrm = descendant_mut(&mut drawing, "a:xfrm").unwrap();
        set_attr(xfrm, "rot", Some("60000".into()));
        assert_eq!(
            get_attr(descendant(&drawing, "a:xfrm").unwrap(), "rot"),
            Some("60000")
        );
    }
}
