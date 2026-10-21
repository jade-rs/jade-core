use crate::document::{
    BorderEdge, DropCap, DropCapKind, Indent, ParagraphBorders, Shading, Spacing,
};
use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode, XmlNodeId};
use crate::word_document::WordDocument;

pub(crate) fn sync_paragraph_alignment_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    alignment: Option<&str>,
) {
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        set_paragraph_alignment_xml(raw, paragraph_properties, alignment);
    });
}

/// Update only the alignment property, preserving other paragraph XML.
pub(crate) fn set_paragraph_alignment_xml(
    raw: &mut DocxXmlDocument,
    properties: &mut XmlElement,
    alignment: Option<&str>,
) {
    match alignment.and_then(normalize_alignment) {
        Some(value) => upsert_paragraph_property_child_with_value(raw, properties, "w:jc", &value),
        None => remove_paragraph_property_child(properties, "w:jc"),
    }
}

pub(crate) fn sync_paragraph_bidi_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    bidi: Option<bool>,
) {
    sync_paragraph_properties_xml(
        word_doc,
        paragraph_index,
        |raw, paragraph_properties| match bidi {
            Some(value) => upsert_paragraph_property_child_with_value(
                raw,
                paragraph_properties,
                "w:bidi",
                if value { "1" } else { "0" },
            ),
            None => remove_paragraph_property_child(paragraph_properties, "w:bidi"),
        },
    );
}

/// `w:snapToGrid`: absent (on, the default) or `w:val="0"` (off).
pub(crate) fn sync_paragraph_snap_to_grid_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    snap: bool,
) {
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        if snap {
            remove_paragraph_property_child(paragraph_properties, "w:snapToGrid")
        } else {
            upsert_paragraph_property_child_with_value(
                raw,
                paragraph_properties,
                "w:snapToGrid",
                "0",
            )
        }
    });
}

pub(crate) fn sync_paragraph_style_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    style_id: Option<&str>,
) {
    let style_id = style_id.map(|value| value.to_string());
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        match style_id.as_deref() {
            Some(value) if !value.is_empty() => {
                upsert_paragraph_property_child_with_value(
                    raw,
                    paragraph_properties,
                    "w:pStyle",
                    value,
                );
            }
            _ => {
                remove_paragraph_property_child(paragraph_properties, "w:pStyle");
            }
        }
    });
}

pub(crate) fn sync_paragraph_drop_cap_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    drop_cap: Option<DropCap>,
) {
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        remove_paragraph_property_child(paragraph_properties, "w:framePr");
        let Some(drop_cap) = drop_cap else {
            return;
        };
        let kind = match drop_cap.kind {
            DropCapKind::Drop => "drop",
            DropCapKind::Margin => "margin",
        };
        let child = raw.create_element(
            "w:framePr",
            vec![
                XmlAttribute {
                    name: "w:dropCap".to_string(),
                    value: kind.to_string(),
                },
                XmlAttribute {
                    name: "w:lines".to_string(),
                    value: drop_cap.lines.clamp(1, 10).to_string(),
                },
                XmlAttribute {
                    name: "w:wrap".to_string(),
                    value: "around".to_string(),
                },
                XmlAttribute {
                    name: "w:vAnchor".to_string(),
                    value: "text".to_string(),
                },
                XmlAttribute {
                    name: "w:hAnchor".to_string(),
                    value: "text".to_string(),
                },
            ],
            Vec::new(),
        );
        paragraph_properties
            .children
            .insert(0, XmlNode::Element(child));
    });
}

pub(crate) fn sync_paragraph_indent_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    indent: Option<&Indent>,
    replace_logical_start: bool,
    replace_logical_end: bool,
) {
    let indent = indent.cloned();
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        set_paragraph_indent_xml(
            raw,
            paragraph_properties,
            indent.as_ref(),
            replace_logical_start,
            replace_logical_end,
        );
    });
}

/// Word's Ctrl+Q: removes every managed direct-formatting child of `w:pPr`
/// while leaving `w:pStyle`, `w:numPr`, `w:rPr`, and section properties in
/// place.
pub(crate) fn sync_reset_paragraph_formatting_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
) {
    sync_paragraph_properties_xml(word_doc, paragraph_index, |_raw, paragraph_properties| {
        reset_paragraph_formatting_xml(paragraph_properties);
    });
}

pub(crate) fn reset_paragraph_formatting_xml(paragraph_properties: &mut XmlElement) {
    for tag in [
        "w:jc",
        "w:ind",
        "w:spacing",
        "w:shd",
        "w:pBdr",
        "w:tabs",
        "w:mirrorIndents",
        "w:textAlignment",
        "w:framePr",
        "w:contextualSpacing",
    ] {
        remove_paragraph_property_child(paragraph_properties, tag);
    }
}

pub(crate) fn sync_paragraph_shading_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    shading: Option<&Shading>,
) {
    let shading = shading.cloned();
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        remove_paragraph_property_child(paragraph_properties, "w:shd");
        let Some(shading) = shading.as_ref() else {
            return;
        };
        let mut attributes = Vec::new();
        if let Some(value) = shading.val.as_deref() {
            attributes.push(XmlAttribute {
                name: "w:val".to_string(),
                value: value.to_string(),
            });
        }
        if let Some(color) = shading.color.as_deref() {
            attributes.push(XmlAttribute {
                name: "w:color".to_string(),
                value: color.trim_start_matches('#').to_string(),
            });
        }
        if let Some(fill) = shading.fill.as_deref() {
            attributes.push(XmlAttribute {
                name: "w:fill".to_string(),
                value: fill.trim_start_matches('#').to_string(),
            });
        }
        if attributes.is_empty() {
            return;
        }
        let child = raw.create_element("w:shd", attributes, Vec::new());
        insert_paragraph_property_child(paragraph_properties, child);
    });
}

pub(crate) fn sync_paragraph_borders_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    borders: Option<&ParagraphBorders>,
) {
    let borders = borders.cloned();
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        remove_paragraph_property_child(paragraph_properties, "w:pBdr");
        let Some(borders) = borders.as_ref() else {
            return;
        };
        let mut children = Vec::new();
        for (name, edge) in [
            ("w:top", borders.top.as_ref()),
            ("w:left", borders.left.as_ref()),
            ("w:bottom", borders.bottom.as_ref()),
            ("w:right", borders.right.as_ref()),
            ("w:between", borders.between.as_ref()),
            ("w:bar", borders.bar.as_ref()),
        ] {
            let Some(edge) = edge else { continue };
            children.push(XmlNode::Element(create_border_edge(raw, name, edge)));
        }
        if children.is_empty() {
            return;
        }
        let child = raw.create_element_preserving_child_ids("w:pBdr", Vec::new(), children);
        insert_paragraph_property_child(paragraph_properties, child);
    });
}

/// The paragraph's `<w:pPr>`, so an undo can put it back exactly. It is read
/// where the edit writes it (see `sync_paragraph_properties_xml`): a note
/// paragraph's part, or a text box paragraph's `w:p` through its host run.
/// Reading only a body paragraph's own node left a note or text box
/// paragraph's edit in the package after it was undone on screen.
pub(crate) fn snapshot_paragraph_properties_xml(
    word_doc: &WordDocument,
    paragraph_index: usize,
) -> Option<XmlElement> {
    let properties = |raw: &DocxXmlDocument, paragraph_id: XmlNodeId| {
        raw.element(paragraph_id)?
            .children
            .iter()
            .find_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:pPr" => Some(element.clone()),
                _ => None,
            })
    };
    if word_doc.is_note_paragraph_surface(paragraph_index) {
        let (_, raw, paragraph_id) = word_doc.note_paragraph_xml(paragraph_index)?;
        return properties(&raw, paragraph_id);
    }
    let paragraph_id = match word_doc.paragraph(paragraph_index)?.node_id {
        Some(paragraph_id) => paragraph_id,
        None => *word_doc
            .text_box_paragraph_element_ids(paragraph_index)
            .first()?,
    };
    properties(word_doc.raw_document.as_ref()?, paragraph_id)
}

/// Restores a previously captured raw `<w:pPr>` subtree exactly, wherever
/// [`snapshot_paragraph_properties_xml`] read it. `None` restores the absence
/// of paragraph properties.
pub(crate) fn restore_paragraph_properties_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    snapshot: Option<&XmlElement>,
) {
    let restore = |raw: &mut DocxXmlDocument, paragraph_id: XmlNodeId| {
        let Some(paragraph) = raw.element_mut(paragraph_id) else {
            return;
        };
        paragraph
            .children
            .retain(|child| !matches!(child, XmlNode::Element(element) if element.name == "w:pPr"));
        if let Some(properties) = snapshot {
            paragraph
                .children
                .insert(0, XmlNode::Element(properties.clone()));
        }
        raw.rebuild_index_for_subtree(paragraph_id);
    };
    if word_doc.is_note_paragraph_surface(paragraph_index) {
        let _ = word_doc.update_note_paragraph_xml(paragraph_index, restore);
        return;
    }
    let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
        return;
    };
    let targets = match paragraph.node_id {
        Some(paragraph_id) => vec![paragraph_id],
        None => word_doc.text_box_paragraph_element_ids(paragraph_index),
    };
    let Some(raw) = word_doc.raw_document.as_mut() else {
        return;
    };
    for paragraph_id in targets {
        restore(raw, paragraph_id);
    }
}

fn insert_paragraph_property_child(paragraph_properties: &mut XmlElement, child: XmlElement) {
    let target_rank = paragraph_property_rank(&child.name);
    let index = paragraph_properties
        .children
        .iter()
        .position(|existing| match existing {
            XmlNode::Element(element) => paragraph_property_rank(&element.name) > target_rank,
            _ => false,
        })
        .unwrap_or(paragraph_properties.children.len());
    paragraph_properties
        .children
        .insert(index, XmlNode::Element(child));
}

/// Relative order from the `CT_PPrBase` sequence. Only tags relevant to the
/// insertion boundary need distinct ranks; unknown extension nodes stay in
/// their original relative order.
fn paragraph_property_rank(name: &str) -> u8 {
    match name {
        "w:pStyle" => 0,
        "w:keepNext" | "w:keepLines" | "w:pageBreakBefore" | "w:framePr" | "w:widowControl" => 1,
        "w:numPr" | "w:suppressLineNumbers" => 2,
        "w:pBdr" => 3,
        "w:shd" => 4,
        "w:tabs"
        | "w:suppressAutoHyphens"
        | "w:kinsoku"
        | "w:wordWrap"
        | "w:overflowPunct"
        | "w:topLinePunct"
        | "w:autoSpaceDE"
        | "w:autoSpaceDN"
        | "w:bidi"
        | "w:adjustRightInd"
        | "w:snapToGrid" => 5,
        "w:spacing" => 6,
        "w:ind" => 7,
        "w:contextualSpacing" | "w:mirrorIndents" | "w:suppressOverlap" => 8,
        "w:jc" | "w:textDirection" | "w:textAlignment" | "w:textboxTightWrap" | "w:outlineLvl"
        | "w:divId" | "w:cnfStyle" | "w:rPr" | "w:sectPr" | "w:pPrChange" => 9,
        _ => 10,
    }
}

fn create_border_edge(raw: &mut DocxXmlDocument, name: &str, edge: &BorderEdge) -> XmlElement {
    let mut attributes = Vec::new();
    if let Some(value) = edge.val.as_deref() {
        attributes.push(XmlAttribute {
            name: "w:val".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = edge.size_eighths {
        attributes.push(XmlAttribute {
            name: "w:sz".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = edge.space {
        attributes.push(XmlAttribute {
            name: "w:space".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = edge.color.as_deref() {
        attributes.push(XmlAttribute {
            name: "w:color".to_string(),
            value: value.trim_start_matches('#').to_string(),
        });
    }
    raw.create_element(name, attributes, Vec::new())
}

fn sync_paragraph_properties_xml<F>(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    mut update_properties: F,
) where
    F: FnMut(&mut DocxXmlDocument, &mut XmlElement),
{
    if word_doc.is_note_paragraph_surface(paragraph_index) {
        // Note node IDs belong to their own part, not word/document.xml.
        let _ = word_doc.update_note_paragraph_xml(paragraph_index, |raw, paragraph_id| {
            sync_paragraph_properties_element_xml(raw, paragraph_id, &mut update_properties);
        });
        return;
    }
    let Some(paragraph_id) = word_doc
        .paragraph(paragraph_index)
        .map(|paragraph| paragraph.node_id)
    else {
        return;
    };
    // Text box paragraphs are unbound; their `w:p` elements (DrawingML box and
    // VML fallback) are reached through the host run instead.
    let targets = match paragraph_id {
        Some(paragraph_id) => vec![paragraph_id],
        None => word_doc.text_box_paragraph_element_ids(paragraph_index),
    };
    let Some(raw) = word_doc.raw_document.as_mut() else {
        return;
    };

    for paragraph_id in targets {
        sync_paragraph_properties_element_xml(raw, paragraph_id, &mut update_properties);
    }
}

fn sync_paragraph_properties_element_xml<F>(
    raw: &mut DocxXmlDocument,
    paragraph_id: crate::docx_xml::XmlNodeId,
    update_properties: &mut F,
) where
    F: FnMut(&mut DocxXmlDocument, &mut XmlElement),
{
    let old_children = {
        let Some(paragraph_element) = raw.element_mut(paragraph_id) else {
            return;
        };
        std::mem::take(&mut paragraph_element.children)
    };

    let mut existing_props = None;
    let mut body_children = Vec::with_capacity(old_children.len());
    for child in old_children {
        match child {
            XmlNode::Element(element) if element.name == "w:pPr" => {
                if existing_props.is_none() {
                    existing_props = Some(element);
                }
            }
            other => body_children.push(other),
        }
    }

    let mut paragraph_properties =
        existing_props.unwrap_or_else(|| raw.create_element("w:pPr", Vec::new(), Vec::new()));
    update_properties(raw, &mut paragraph_properties);

    if !paragraph_properties.children.is_empty() || !paragraph_properties.attributes.is_empty() {
        body_children.insert(0, XmlNode::Element(paragraph_properties));
    }

    if let Some(paragraph_element) = raw.element_mut(paragraph_id) {
        paragraph_element.children = body_children;
    }

    raw.rebuild_index_for_subtree(paragraph_id);
}

fn normalize_alignment(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let normalized = match lowered.as_str() {
        "start" => "start",
        "left" => "left",
        "center" => "center",
        "end" => "end",
        "right" => "right",
        // The editor model uses CSS-like `justify`; Word's ST_Jc token is
        // `both` (§17.18.44). Never emit the non-standard `justify` token.
        "both" | "justify" => "both",
        _ => trimmed,
    };

    Some(normalized.to_string())
}

fn upsert_paragraph_property_child_with_value(
    raw: &mut DocxXmlDocument,
    paragraph_properties: &mut XmlElement,
    tag_name: &str,
    value: &str,
) {
    for child in &mut paragraph_properties.children {
        if let XmlNode::Element(child_element) = child {
            if child_element.name == tag_name {
                if let Some(attr) = child_element
                    .attributes
                    .iter_mut()
                    .find(|attr| attr.name == "w:val")
                {
                    attr.value = value.to_string();
                } else {
                    child_element.attributes.push(XmlAttribute {
                        name: "w:val".to_string(),
                        value: value.to_string(),
                    });
                }
                return;
            }
        }
    }

    let child = raw.create_element(
        tag_name,
        vec![XmlAttribute {
            name: "w:val".to_string(),
            value: value.to_string(),
        }],
        Vec::new(),
    );
    paragraph_properties
        .children
        .insert(0, XmlNode::Element(child));
}

pub(crate) fn set_paragraph_indent_xml(
    raw: &mut DocxXmlDocument,
    paragraph_properties: &mut XmlElement,
    indent: Option<&Indent>,
    _replace_logical_start: bool,
    _replace_logical_end: bool,
) {
    let Some(indent) = indent else {
        remove_paragraph_property_child(paragraph_properties, "w:ind");
        return;
    };

    let mut attrs = Vec::new();
    if let Some(value) = indent.left.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:left".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.right.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:right".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.start.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:start".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.end.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:end".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.first_line.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:firstLine".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.hanging.filter(|value| value.is_finite()) {
        attrs.push(XmlAttribute {
            name: "w:hanging".to_string(),
            value: ((value * 20.0).round() as i32).to_string(),
        });
    }
    if let Some(value) = indent.left_chars {
        attrs.push(XmlAttribute {
            name: "w:leftChars".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = indent.right_chars {
        attrs.push(XmlAttribute {
            name: "w:rightChars".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = indent.start_chars {
        attrs.push(XmlAttribute {
            name: "w:startChars".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = indent.end_chars {
        attrs.push(XmlAttribute {
            name: "w:endChars".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = indent.first_line_chars {
        attrs.push(XmlAttribute {
            name: "w:firstLineChars".to_string(),
            value: value.to_string(),
        });
    }
    if let Some(value) = indent.hanging_chars {
        attrs.push(XmlAttribute {
            name: "w:hangingChars".to_string(),
            value: value.to_string(),
        });
    }

    if attrs.is_empty() {
        remove_paragraph_property_child(paragraph_properties, "w:ind");
        return;
    }

    for child in &mut paragraph_properties.children {
        if let XmlNode::Element(child_element) = child {
            if child_element.name == "w:ind" {
                child_element.attributes.retain(|attr| {
                    attr.name != "w:left"
                        && attr.name != "w:right"
                        && attr.name != "w:start"
                        && attr.name != "w:end"
                        && attr.name != "w:firstLine"
                        && attr.name != "w:hanging"
                        && attr.name != "w:leftChars"
                        && attr.name != "w:rightChars"
                        && attr.name != "w:startChars"
                        && attr.name != "w:endChars"
                        && attr.name != "w:firstLineChars"
                        && attr.name != "w:hangingChars"
                });
                child_element.attributes.extend(attrs);
                return;
            }
        }
    }

    let child = raw.create_element("w:ind", attrs, Vec::new());
    paragraph_properties
        .children
        .insert(0, XmlNode::Element(child));
}

/// Sets the parts of `<w:spacing>` this editor manages: the line height and
/// the space above and below the paragraph. Point and line-based before/after
/// values are retained together for faithful Undo roundtrips. Setting a
/// point value also removes Word's competing automatic-spacing attribute.
/// `None` means "remove the attribute I manage" rather than "leave it".
///
/// `before` / `after` are points; `line` is the multiplier under the `auto`
/// rule and points under `exact` / `atLeast`, which is how Word stores it.
pub(crate) fn sync_paragraph_spacing_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    spacing: Option<&Spacing>,
) {
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        set_paragraph_spacing_xml(raw, paragraph_properties, spacing, true, true, true);
    });
}

pub(crate) fn set_paragraph_spacing_xml(
    raw: &mut DocxXmlDocument,
    paragraph_properties: &mut XmlElement,
    spacing: Option<&Spacing>,
    update_line: bool,
    update_before: bool,
    update_after: bool,
) {
    let line = spacing.and_then(|value| value.line);
    let line_rule = spacing.and_then(|value| value.line_rule.as_deref());
    let before = spacing.and_then(|value| value.before);
    let after = spacing.and_then(|value| value.after);
    let before_lines = spacing.and_then(|value| value.before_lines);
    let after_lines = spacing.and_then(|value| value.after_lines);
    let mut managed: Vec<XmlAttribute> = Vec::new();
    let points_to_twips = |value: f32| (value * 20.0).round().max(0.0) as i32;
    if let Some(value) = before.filter(|v| update_before && v.is_finite() && *v >= 0.0) {
        managed.push(XmlAttribute {
            name: "w:before".to_string(),
            value: points_to_twips(value).to_string(),
        });
    }
    if let Some(value) = after.filter(|v| update_after && v.is_finite() && *v >= 0.0) {
        managed.push(XmlAttribute {
            name: "w:after".to_string(),
            value: points_to_twips(value).to_string(),
        });
    }
    if update_before {
        if let Some(value) = before_lines.filter(|v| v.is_finite() && *v >= 0.0) {
            managed.push(XmlAttribute {
                name: "w:beforeLines".to_string(),
                value: (value * 100.0).round().to_string(),
            });
        }
    }
    if update_after {
        if let Some(value) = after_lines.filter(|v| v.is_finite() && *v >= 0.0) {
            managed.push(XmlAttribute {
                name: "w:afterLines".to_string(),
                value: (value * 100.0).round().to_string(),
            });
        }
    }
    if let Some(value) = line.filter(|v| update_line && v.is_finite() && *v > 0.0) {
        let normalized_rule = match line_rule.unwrap_or("auto") {
            "exactly" => "exact",
            rule => rule,
        };
        let line_twips = if matches!(normalized_rule, "exact" | "atLeast") {
            value * 20.0
        } else {
            value * 240.0
        };
        managed.push(XmlAttribute {
            name: "w:line".to_string(),
            value: (line_twips.round() as i32).to_string(),
        });
        managed.push(XmlAttribute {
            name: "w:lineRule".to_string(),
            value: normalized_rule.to_string(),
        });
    }

    let existing = paragraph_properties
        .children
        .iter()
        .position(|c| matches!(c, XmlNode::Element(e) if e.name == "w:spacing"));
    if let Some(idx) = existing {
        if let XmlNode::Element(el) = &mut paragraph_properties.children[idx] {
            el.attributes.retain(|a| {
                let managed = (update_line && matches!(a.name.as_str(), "w:line" | "w:lineRule"))
                    || (update_before && matches!(a.name.as_str(), "w:before" | "w:beforeLines"))
                    || (update_after && matches!(a.name.as_str(), "w:after" | "w:afterLines"));
                let competing_auto_spacing =
                    (update_before && before.is_some() && a.name == "w:beforeAutospacing")
                        || (update_after && after.is_some() && a.name == "w:afterAutospacing");
                !managed && !competing_auto_spacing
            });
            el.attributes.extend(managed);
            if el.attributes.is_empty() {
                paragraph_properties.children.remove(idx);
            }
        }
        return;
    }
    if !managed.is_empty() {
        let child = raw.create_element("w:spacing", managed, Vec::new());
        paragraph_properties
            .children
            .insert(0, XmlNode::Element(child));
    }
}

pub(crate) fn sync_paragraph_numbering_xml(
    word_doc: &mut WordDocument,
    paragraph_index: usize,
    num_id: Option<&str>,
    level: Option<u32>,
) {
    let mut direct_indent = false;
    let mut synced = false;
    sync_paragraph_properties_xml(word_doc, paragraph_index, |raw, paragraph_properties| {
        synced = true;
        direct_indent |= paragraph_properties
            .children
            .iter()
            .any(|child| matches!(child, XmlNode::Element(element) if element.name == "w:ind"));
        match num_id {
            Some(id) => {
                upsert_paragraph_numbering_child(raw, paragraph_properties, id, level);
            }
            None => {
                remove_paragraph_property_child(paragraph_properties, "w:numPr");
            }
        }
    });
    if synced {
        // The model carries the style's indent filled in at load; Word lets
        // the numbering level override it unless the paragraph has its own.
        word_doc.apply_numbering_indent_precedence(
            paragraph_index,
            num_id.is_some_and(|id| id != "0"),
            direct_indent,
        );
    }
}

fn upsert_paragraph_numbering_child(
    raw: &mut DocxXmlDocument,
    paragraph_properties: &mut XmlElement,
    num_id: &str,
    level: Option<u32>,
) {
    // Remove existing w:numPr if present.
    remove_paragraph_property_child(paragraph_properties, "w:numPr");

    // Build children of w:numPr.
    let ilvl_element = raw.create_element(
        "w:ilvl",
        vec![XmlAttribute {
            name: "w:val".to_string(),
            value: level.unwrap_or(0).to_string(),
        }],
        Vec::new(),
    );
    let num_id_element = raw.create_element(
        "w:numId",
        vec![XmlAttribute {
            name: "w:val".to_string(),
            value: num_id.to_string(),
        }],
        Vec::new(),
    );
    let num_pr = raw.create_element(
        "w:numPr",
        Vec::new(),
        vec![
            XmlNode::Element(ilvl_element),
            XmlNode::Element(num_id_element),
        ],
    );
    // CT_PPr is a sequence: w:numPr follows w:pStyle, w:keepNext,
    // w:keepLines, w:pageBreakBefore, w:framePr and w:widowControl.
    let at = paragraph_properties
        .children
        .iter()
        .rposition(|child| {
            matches!(child, XmlNode::Element(element) if matches!(
                element.name.as_str(),
                "w:pStyle" | "w:keepNext" | "w:keepLines" | "w:pageBreakBefore" | "w:framePr" | "w:widowControl"
            ))
        })
        .map_or(0, |index| index + 1);
    paragraph_properties
        .children
        .insert(at, XmlNode::Element(num_pr));
}

fn remove_paragraph_property_child(paragraph_properties: &mut XmlElement, tag_name: &str) {
    if let Some(index) = paragraph_properties.children.iter().position(
        |child| matches!(child, XmlNode::Element(child_element) if child_element.name == tag_name),
    ) {
        paragraph_properties.children.remove(index);
    }
}
