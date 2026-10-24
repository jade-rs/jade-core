//! OOXML property revisions are separate from insertion/deletion wrappers.
use super::*;
use jade_docx_headless_api::HeadlessRevisionSnapshot;

fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == name => Some(element),
        _ => None,
    })
}
fn attr(element: &XmlElement, name: &str) -> Option<String> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.clone())
}
fn without_changes(mut element: XmlElement) -> XmlElement {
    element.children.retain(|node| !matches!(node, XmlNode::Element(e) if e.name == "w:rPrChange" || e.name == "w:pPrChange"));
    // pPrChange contains CT_PPrGeneral, which excludes paragraph-mark run
    // properties and section properties. Those have their own revisions.
    if element.name == "w:pPr" {
        element.children.retain(|node| !matches!(node, XmlNode::Element(e) if e.name == "w:rPr" || e.name == "w:sectPr"));
    }
    element
}
fn signature(element: Option<&XmlElement>) -> String {
    fn visit(element: &XmlElement, out: &mut String) {
        out.push_str(&element.name);
        for attribute in &element.attributes {
            out.push_str(&attribute.name);
            out.push('=');
            out.push_str(&attribute.value);
            out.push(';');
        }
        for node in &element.children {
            match node {
                XmlNode::Element(e)
                    if e.name == "w:rPrChange"
                        || e.name == "w:pPrChange"
                        || element.name == "w:pPr"
                            && matches!(e.name.as_str(), "w:rPr" | "w:sectPr") => {}
                XmlNode::Element(e) => {
                    out.push('[');
                    visit(e, out);
                    out.push(']');
                }
                other => out.push_str(&format!("{other:?}")),
            }
        }
    }
    let mut result = String::new();
    if let Some(element) = element {
        visit(element, &mut result);
    }
    result
}

impl WordDocument {
    pub(crate) fn record_formatting_changes(
        &mut self,
        before: &[Paragraph],
        old_raw: &DocxXmlDocument,
    ) {
        let Some(session) = self.revision_session.clone() else {
            return;
        };
        let Some(raw) = self.raw_document.as_ref() else {
            return;
        };
        let mut pending = Vec::new();
        for (index, old) in before.iter().enumerate() {
            let Some(current) = self.paragraph(index) else {
                continue;
            };
            let Some(parent) = current.node_id.and_then(|id| raw.element(id)) else {
                continue;
            };
            let old_ppr = old
                .node_id
                .and_then(|id| old_raw.element(id))
                .and_then(|element| child(element, "w:pPr"));
            if signature(old_ppr) != signature(child(parent, "w:pPr")) {
                pending.push((parent.id, "w:pPr", "w:pPrChange", old_ppr.cloned()));
            }
            let mut offset = 0usize;
            for run in &current.runs {
                let start = offset;
                offset += run.text.len();
                let mut old_offset = 0usize;
                let source = old.runs.iter().find(|source| {
                    let from = old_offset;
                    old_offset += source.text.len();
                    from <= start && (start < old_offset || source.text.is_empty() && from == start)
                });
                let Some(source) = source else {
                    continue;
                };
                // Paragraph style inheritance is represented by pPrChange alone.
                if source.properties == run.properties {
                    continue;
                }
                let Some(parent) = run.node_id.and_then(|id| raw.element(id)) else {
                    continue;
                };
                let old_rpr = source
                    .node_id
                    .and_then(|id| old_raw.element(id))
                    .and_then(|element| child(element, "w:rPr"));
                if signature(old_rpr) != signature(child(parent, "w:rPr")) {
                    pending.push((parent.id, "w:rPr", "w:rPrChange", old_rpr.cloned()));
                }
            }
        }
        for (parent_id, property_name, change_name, previous) in pending {
            let revision_id = self.allocate_revision_id().to_string();
            let Some(raw) = self.raw_document.as_mut() else {
                continue;
            };
            // Further edits to an unreviewed property retain its original value.
            let existing = raw
                .element(parent_id)
                .and_then(|parent| child(parent, property_name))
                .and_then(|props| child(props, change_name))
                .cloned()
                .or_else(|| {
                    previous
                        .as_ref()
                        .and_then(|props| child(props, change_name))
                        .cloned()
                });
            let change = if let Some(existing) = existing {
                raw.create_element(change_name, existing.attributes, existing.children)
            } else {
                let mut original = previous
                    .map(without_changes)
                    .unwrap_or_else(|| raw.create_element(property_name, Vec::new(), Vec::new()));
                // CT_PPrBase excludes paragraph-mark and section properties.
                // Their current values stay outside the paragraph-format revision.
                if property_name == "w:pPr" {
                    original.children.retain(|node| {
                        !matches!(node, XmlNode::Element(e)
                        if e.name == "w:rPr" || e.name == "w:sectPr")
                    });
                }
                let mut attributes = vec![XmlAttribute {
                    name: "w:id".into(),
                    value: revision_id,
                }];
                for (name, value) in [
                    ("w:author", &session.author),
                    ("w:date", &session.date),
                    ("w16du:dateUtc", &session.date_utc),
                ] {
                    if let Some(value) = value {
                        attributes.push(XmlAttribute {
                            name: name.into(),
                            value: value.clone(),
                        });
                    }
                }
                raw.create_element(change_name, attributes, vec![XmlNode::Element(original)])
            };
            let mut properties = raw
                .element(parent_id)
                .and_then(|parent| child(parent, property_name))
                .cloned()
                .unwrap_or_else(|| raw.create_element(property_name, Vec::new(), Vec::new()));
            properties
                .children
                .retain(|node| !matches!(node, XmlNode::Element(e) if e.name == change_name));
            properties.children.push(XmlNode::Element(change));
            if let Some(parent) = raw.element_mut(parent_id) {
                parent
                    .children
                    .retain(|node| !matches!(node, XmlNode::Element(e) if e.name == property_name));
                parent.children.insert(0, XmlNode::Element(properties));
            }
            raw.rebuild_index();
        }
        if let (Some(document), Some(raw)) = (self.document.as_mut(), self.raw_document.as_ref()) {
            if let Some(body) = Rc::make_mut(document).body.as_mut() {
                Self::capture_preserved_run_props(&mut body.elements, raw);
            }
        }
    }

    pub(crate) fn formatting_revision_snapshots(&self) -> Vec<HeadlessRevisionSnapshot> {
        let Some(raw) = self.raw_document.as_ref() else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for index in 0..self.paragraph_count() {
            let Some(paragraph) = self.paragraph(index) else {
                continue;
            };
            let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            let mut append = |parent_id: Option<XmlNodeId>,
                              property: &str,
                              change: &str,
                              kind: &str,
                              start: usize,
                              end: usize,
                              text: String| {
                let Some(change) = parent_id
                    .and_then(|id| raw.element(id))
                    .and_then(|parent| child(parent, property))
                    .and_then(|props| child(props, change))
                else {
                    return;
                };
                result.push(HeadlessRevisionSnapshot {
                    id: attr(change, "w:id"),
                    kind: kind.into(),
                    author: attr(change, "w:author"),
                    date: attr(change, "w:date"),
                    date_utc: attr(change, "w16du:dateUtc"),
                    paragraph: index,
                    start_offset: start,
                    end_offset: end,
                    text,
                    paragraph_mark: false,
                });
            };
            append(
                paragraph.node_id,
                "w:pPr",
                "w:pPrChange",
                "paragraphFormat",
                0,
                text.len(),
                text,
            );
            let mut offset = 0;
            for run in &paragraph.runs {
                append(
                    run.node_id,
                    "w:rPr",
                    "w:rPrChange",
                    "runFormat",
                    offset,
                    offset + run.text.len(),
                    run.text.clone(),
                );
                offset += run.text.len();
            }
        }
        result
    }

    pub(super) fn decide_formatting_revisions(
        &mut self,
        decision: RevisionDecision,
        start: usize,
        end: usize,
        ids: Option<RevisionIdFilter<'_>>,
        targets: Option<&RevisionTargets>,
    ) -> Result<usize, String> {
        let selected = self
            .formatting_revision_snapshots()
            .into_iter()
            .filter(|revision| {
                start <= revision.paragraph
                    && revision.paragraph <= end
                    && ids.is_none_or(|filter| filter.matches(revision.id.as_deref()))
                    && targets.is_none_or(|targets| {
                        targets
                            .formats
                            .get(&revision.paragraph)
                            .is_some_and(|formats| {
                                formats.iter().any(|(start, end, id, kind)| {
                                    *start <= revision.start_offset
                                        && revision.end_offset <= *end
                                        && *id == revision.id
                                        && *kind == revision.kind
                                })
                            })
                    })
            })
            .collect::<Vec<_>>();
        for revision in &selected {
            let paragraph = self
                .paragraph(revision.paragraph)
                .ok_or("formatting revision paragraph missing")?;
            let (parent, property, change) = if revision.kind == "paragraphFormat" {
                (paragraph.node_id, "w:pPr", "w:pPrChange")
            } else {
                let mut offset = 0;
                let run = paragraph.runs.iter().find(|run| {
                    let from = offset;
                    offset += run.text.len();
                    from == revision.start_offset
                });
                (run.and_then(|run| run.node_id), "w:rPr", "w:rPrChange")
            };
            let Some(raw) = self.raw_document.as_mut() else {
                continue;
            };
            let props = parent
                .and_then(|id| raw.element(id))
                .and_then(|parent| child(parent, property))
                .cloned()
                .ok_or("formatting revision properties missing")?;
            let mut replacement = if decision == RevisionDecision::Reject {
                child(&props, change)
                    .and_then(|change| child(change, property))
                    .cloned()
                    .unwrap_or_else(|| raw.create_element(property, Vec::new(), Vec::new()))
            } else {
                props.clone()
            };
            if decision == RevisionDecision::Reject && property == "w:pPr" {
                // Reject paragraph formatting without deleting the independently
                // tracked paragraph mark or the paragraph's section boundary.
                replacement.children.extend(props.children.iter().filter(|node|
                    matches!(node, XmlNode::Element(e) if e.name == "w:rPr" || e.name == "w:sectPr")
                ).cloned());
            }
            replacement.id = props.id;
            replacement
                .children
                .retain(|node| !matches!(node, XmlNode::Element(e) if e.name == change));
            if let Some(parent) = parent.and_then(|id| raw.element_mut(id)) {
                if let Some(node) = parent
                    .children
                    .iter_mut()
                    .find(|node| matches!(node, XmlNode::Element(e) if e.name == property))
                {
                    *node = XmlNode::Element(replacement);
                }
            }
            raw.rebuild_index();
        }
        if !selected.is_empty() {
            self.refresh_formatting_from_xml()?;
            self.refresh_revision_counter();
        }
        Ok(selected.len())
    }

    /// Reparse properties only: resolved fields, images, notes and editing state
    /// remain attached to their existing semantic nodes.
    fn refresh_formatting_from_xml(&mut self) -> Result<(), String> {
        let Some(raw) = self.raw_document.as_ref() else {
            return Ok(());
        };
        let xml = raw.to_string().map_err(|error| error.to_string())?;
        let mut parsed = DocumentParser::new()
            .parse_document_xml(&xml)
            .map_err(|error| error.to_string())?;
        if let Some(body) = parsed.body.as_mut() {
            Self::capture_direct_run_fonts(&mut body.elements);
        }
        attach_document_xml_ids(&mut parsed, raw).map_err(|error| format!("{error:?}"))?;
        attach_text_box_xml_ids(&mut parsed, raw);
        if let Some(body) = parsed.body.as_mut() {
            Self::capture_preserved_run_props(&mut body.elements, raw);
        }
        if let Some(styles) = self.styles_part.as_ref() {
            Self::apply_style_paragraph_defaults_to_document(&mut parsed, styles);
            Self::apply_table_styles_to_document(&mut parsed, styles);
            Self::apply_style_numbering_to_document(&mut parsed, styles);
            Self::apply_run_font_defaults_to_document(&mut parsed, styles);
        }
        if let Some(theme) = self.loaded_theme_part() {
            Self::apply_theme_to_document(&mut parsed, &theme);
        }
        let original = self.document.replace(Rc::new(parsed));
        self.rebuild_paragraph_surfaces();
        let properties = (0..self.paragraph_count())
            .filter_map(|index| self.paragraph(index))
            .map(|paragraph| {
                (
                    paragraph.node_id,
                    paragraph.properties.clone(),
                    paragraph
                        .runs
                        .iter()
                        .map(|run| (run.node_id, run.properties.clone()))
                        .collect::<HashMap<_, _>>(),
                )
            })
            .collect::<Vec<_>>();
        self.document = original;
        self.rebuild_paragraph_surfaces();
        for index in 0..self.paragraph_count() {
            if let Some(paragraph) = self.paragraph_mut(index) {
                if let Some((_, props, runs)) = properties
                    .iter()
                    .find(|(id, _, _)| *id == paragraph.node_id)
                {
                    paragraph.properties = props.clone();
                    for run in &mut paragraph.runs {
                        if let Some(props) = runs.get(&run.node_id) {
                            run.properties = props.clone();
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
