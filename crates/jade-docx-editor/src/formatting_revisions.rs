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
    element
}
/// What a `w:rPr` or `w:pPr` says, for telling whether an edit changed it.
/// Its direct properties are a set, so each is compared whatever its place:
/// the half of a split run that an edit left alone has its `w:rPr` written
/// again, and `w:szCs` coming back before `w:sz` is no formatting change.
/// Preserve order inside properties, including AlternateContent choices:
/// the first supported choice determines the effective formatting.
fn signature(element: Option<&XmlElement>) -> String {
    fn visit(element: &XmlElement, out: &mut String) {
        out.push_str(&element.name);
        for attribute in &element.attributes {
            out.push_str(&attribute.name);
            out.push('=');
            out.push_str(&attribute.value);
            out.push(';');
        }
        let mut children = Vec::new();
        for node in &element.children {
            match node {
                XmlNode::Element(e)
                    if e.name == "w:rPrChange"
                        || e.name == "w:pPrChange"
                        || element.name == "w:pPr"
                            && matches!(e.name.as_str(), "w:rPr" | "w:sectPr") => {}
                XmlNode::Element(e) => {
                    let mut child = String::from("[");
                    visit(e, &mut child);
                    child.push(']');
                    children.push(child);
                }
                other => children.push(format!("{other:?}")),
            }
        }
        if matches!(element.name.as_str(), "w:rPr" | "w:pPr") {
            children.sort_unstable();
        }
        children.into_iter().for_each(|child| out.push_str(&child));
    }
    let mut result = String::new();
    if let Some(element) = element {
        visit(element, &mut result);
    }
    result
}

type RunChanges = HashMap<XmlNodeId, (Box<crate::document::RunPropertiesChange>, XmlElement)>;

#[cfg(test)]
mod signature_tests {
    use super::*;

    fn from_xml(xml: &str) -> String {
        signature(Some(DocxXmlDocument::parse(xml).unwrap().root()))
    }

    #[test]
    fn property_signature_ignores_top_level_property_order() {
        assert_eq!(
            from_xml("<w:rPr><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr>"),
            from_xml("<w:rPr><w:szCs w:val=\"24\"/><w:sz w:val=\"24\"/></w:rPr>"),
        );
    }

    #[test]
    fn property_signature_preserves_alternate_content_choice_precedence() {
        // The first supported Choice wins. Changing its place changes the
        // selected formatting, even though the same child elements remain.
        let before = "<w:rPr><mc:AlternateContent><mc:Choice Requires=\"w14\"><w:b/></mc:Choice><mc:Choice Requires=\"w14\"><w:i/></mc:Choice><mc:Fallback/></mc:AlternateContent></w:rPr>";
        let after = "<w:rPr><mc:AlternateContent><mc:Choice Requires=\"w14\"><w:i/></mc:Choice><mc:Choice Requires=\"w14\"><w:b/></mc:Choice><mc:Fallback/></mc:AlternateContent></w:rPr>";
        assert_ne!(from_xml(before), from_xml(after));
    }
}

/// Note XML is reparsed on each edit. Bind only a temporary paragraph clone:
/// storing these part-local IDs on the live model would alias body IDs.
fn bind_note_paragraph(paragraph: &Paragraph, raw: &DocxXmlDocument, id: XmlNodeId) -> Paragraph {
    let mut parent = raw.element(id).expect("note paragraph XML").clone();
    parent.children = vec![XmlNode::Element(parent.clone())];
    let mut blocks = vec![BodyElement::Paragraph(Box::new(paragraph.clone()))];
    attach_block_xml_ids(&mut blocks, &parent, raw);
    let BodyElement::Paragraph(paragraph) = blocks.remove(0) else {
        unreachable!()
    };
    *paragraph
}

fn record_paragraph_changes(
    old: &Paragraph,
    current: &Paragraph,
    old_raw: &DocxXmlDocument,
    raw: &mut DocxXmlDocument,
    session: &RevisionSession,
    next_revision_id: &mut u32,
    seen_ids: &mut std::collections::HashSet<String>,
) -> RunChanges {
    let mut pending = Vec::new();
    let Some(parent) = current.node_id.and_then(|id| raw.element(id)) else {
        return HashMap::new();
    };
    let old_ppr = old
        .node_id
        .and_then(|id| old_raw.element(id))
        .and_then(|element| child(element, "w:pPr"));
    if signature(old_ppr) != signature(child(parent, "w:pPr")) {
        pending.push((parent.id, "w:pPr", "w:pPrChange", old_ppr.cloned(), None));
    }
    let mut offset = 0usize;
    let mut empty_ordinal = 0usize;
    for run in &current.runs {
        let start = offset;
        offset += run.text.len();
        let mut old_offset = 0usize;
        // Several references can occupy the same text offset. Match each
        // zero-width run in order instead of reusing the first one's baseline.
        let source = if run.text.is_empty() {
            let source = old
                .runs
                .iter()
                .filter(|source| {
                    let from = old_offset;
                    old_offset += source.text.len();
                    source.text.is_empty() && from == start
                })
                .nth(empty_ordinal);
            empty_ordinal += 1;
            source
        } else {
            empty_ordinal = 0;
            old.runs.iter().find(|source| {
                let from = old_offset;
                old_offset += source.text.len();
                from <= start && start < old_offset
            })
        };
        let Some(source) = source else {
            continue;
        };
        let Some(parent) = run.node_id.and_then(|id| raw.element(id)) else {
            continue;
        };
        let old_rpr = source
            .node_id
            .and_then(|id| old_raw.element(id))
            .and_then(|element| child(element, "w:rPr"));
        if signature(old_rpr) != signature(child(parent, "w:rPr")) {
            let mut original = source.properties.clone().unwrap_or_default();
            if let Some(change) = original.change.take() {
                original = change.previous;
            }
            pending.push((
                parent.id,
                "w:rPr",
                "w:rPrChange",
                old_rpr.cloned(),
                Some(original),
            ));
        }
    }
    let mut run_changes = HashMap::new();
    for (parent_id, property_name, change_name, previous, original_model) in pending {
        let revision_id = next_revision_id.to_string();
        *next_revision_id = next_revision_id.saturating_add(1);
        // Further edits to an unreviewed property retain its original value.
        // Only the pre-command XML is authoritative. The command's model
        // stamp may contain inherited properties and omit unmodeled XML.
        let existing = previous
            .as_ref()
            .and_then(|props| child(props, change_name))
            .cloned();
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
        if let Some(previous) = original_model {
            run_changes.insert(
                parent_id,
                (
                    Box::new(crate::document::RunPropertiesChange {
                        id: attr(&change, "w:id"),
                        author: attr(&change, "w:author"),
                        date: attr(&change, "w:date"),
                        previous,
                    }),
                    change.clone(),
                ),
            );
        }
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
    // Splitting a previously formatted run copies its review stamp. Keep the
    // first identity stable and allocate a distinct identity for each fragment
    // before exposing revisions to callers (not only during DOCX export).
    let targets = std::iter::once((current.node_id, "w:pPr", "w:pPrChange", None))
        .chain(current.runs.iter().map(|run| {
            let model = run
                .node_id
                .and_then(|id| run_changes.get(&id))
                .map(|(model, _)| model.clone())
                .or_else(|| run.properties.as_ref().and_then(|p| p.change.clone()));
            (run.node_id, "w:rPr", "w:rPrChange", model)
        }))
        .collect::<Vec<_>>();
    for (parent_id, property_name, change_name, model) in targets {
        let Some(parent_id) = parent_id else {
            continue;
        };
        let Some(mut properties) = raw
            .element(parent_id)
            .and_then(|p| child(p, property_name))
            .cloned()
        else {
            continue;
        };
        let Some(change) = properties.children.iter_mut().find_map(|node| match node {
            XmlNode::Element(e) if e.name == change_name => Some(e),
            _ => None,
        }) else {
            continue;
        };
        let Some(id) = attr(change, "w:id") else {
            continue;
        };
        // Serializing a note part may already have assigned fresh IDs to
        // split fragments. Those part-local repairs do not advance the
        // document allocator, so reserve every observed ID before allocating
        // a replacement for a collision with another story.
        if let Ok(value) = id.parse::<u32>() {
            *next_revision_id = (*next_revision_id).max(value.saturating_add(1));
        }
        if seen_ids.insert(id) {
            continue;
        }
        let id = next_revision_id.to_string();
        *next_revision_id = next_revision_id.saturating_add(1);
        seen_ids.insert(id.clone());
        if let Some(attribute) = change.attributes.iter_mut().find(|a| a.name == "w:id") {
            attribute.value = id.clone();
        }
        if let Some(mut model) = model {
            model.id = Some(id);
            run_changes.insert(parent_id, (model, change.clone()));
        }
        if let Some(parent) = raw.element_mut(parent_id) {
            parent
                .children
                .retain(|node| !matches!(node, XmlNode::Element(e) if e.name == property_name));
            parent.children.insert(0, XmlNode::Element(properties));
        }
        raw.rebuild_index();
    }
    run_changes
}

impl WordDocument {
    pub(crate) fn formatting_before(&self) -> Vec<(usize, Paragraph, Option<DocxXmlDocument>)> {
        (0..self.paragraph_count())
            .filter_map(|index| {
                let paragraph = self.paragraph(index)?;
                if self.is_note_paragraph_surface(index) {
                    let (_, raw, id) = self.note_paragraph_xml(index)?;
                    Some((index, bind_note_paragraph(paragraph, &raw, id), Some(raw)))
                } else {
                    Some((index, paragraph.clone(), None))
                }
            })
            .collect()
    }

    pub(crate) fn record_formatting_changes(
        &mut self,
        before: &[(usize, Paragraph, Option<DocxXmlDocument>)],
        old_raw: &DocxXmlDocument,
    ) -> Result<(), String> {
        let Some(session) = self.revision_session.clone() else {
            return Ok(());
        };
        let mut seen_ids = std::collections::HashSet::new();
        for (index, old, old_note_raw) in before {
            let Some(mut current) = self.paragraph(*index).cloned() else {
                continue;
            };
            let mut changes = if let Some(old_raw) = old_note_raw {
                let (part, mut raw, id) = self
                    .note_paragraph_xml(*index)
                    .ok_or("note paragraph XML is missing")?;
                current = bind_note_paragraph(&current, &raw, id);
                let changes = record_paragraph_changes(
                    old,
                    &current,
                    old_raw,
                    &mut raw,
                    &session,
                    &mut self.next_revision_id,
                    &mut seen_ids,
                );
                self.upsert_xml_part_content(
                    &part,
                    raw.to_string().map_err(|error| error.to_string())?,
                );
                changes
            } else {
                let Some(raw) = self.raw_document.as_mut() else {
                    continue;
                };
                record_paragraph_changes(
                    old,
                    &current,
                    old_raw,
                    raw,
                    &session,
                    &mut self.next_revision_id,
                    &mut seen_ids,
                )
            };
            // Preserve the stamp in the semantic model too: the next run
            // serialization must not erase a direct OOXML property revision.
            if let Some(paragraph) = self.paragraph_mut(*index) {
                for (run, bound) in paragraph.runs.iter_mut().zip(&current.runs) {
                    if let Some((change, raw_change)) =
                        bound.node_id.and_then(|id| changes.remove(&id))
                    {
                        let properties = run.properties.get_or_insert_with(Default::default);
                        properties.change = Some(change);
                        properties.preserved_rpr.0.retain(
                            |node| !matches!(node, XmlNode::Element(e) if e.name == "w:rPrChange"),
                        );
                        properties
                            .preserved_rpr
                            .0
                            .push(XmlNode::Element(raw_change));
                    }
                }
            }
            if old_note_raw.is_some() {
                let change = self.paragraph_xml_element(*index).and_then(|p| {
                    child(&p, "w:pPr")
                        .and_then(|p| child(p, "w:pPrChange"))
                        .cloned()
                });
                if let Some(paragraph) = self.paragraph_mut(*index) {
                    let props = paragraph.properties.get_or_insert_with(Default::default);
                    props.has_format_change = change.is_some();
                    props.format_change_author = change
                        .as_ref()
                        .and_then(|c| attr(c, "w:author"))
                        .map(Into::into);
                }
            }
        }
        Ok(())
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
            let note = self.note_paragraph_xml(index);
            let bound = note
                .as_ref()
                .map(|(_, raw, id)| bind_note_paragraph(paragraph, raw, *id));
            let paragraph = bound.as_ref().unwrap_or(paragraph);
            let raw = note.as_ref().map(|(_, raw, _)| raw).unwrap_or(raw);
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
                        targets.contains_format(
                            revision.paragraph,
                            revision.start_offset,
                            revision.end_offset,
                            revision.id.as_deref(),
                            &revision.kind,
                        )
                    })
            })
            .collect::<Vec<_>>();
        for revision in &selected {
            let paragraph = self
                .paragraph(revision.paragraph)
                .ok_or("formatting revision paragraph missing")?;
            let mut note = self.note_paragraph_xml(revision.paragraph);
            let bound = note
                .as_ref()
                .map(|(_, raw, id)| bind_note_paragraph(paragraph, raw, *id));
            let paragraph = bound.as_ref().unwrap_or(paragraph);
            let (parent, property, change) = if revision.kind == "paragraphFormat" {
                (paragraph.node_id, "w:pPr", "w:pPrChange")
            } else {
                let mut offset = 0;
                let run = paragraph.runs.iter().find(|run| {
                    let from = offset;
                    offset += run.text.len();
                    from == revision.start_offset
                        && offset == revision.end_offset
                        && run.node_id.and_then(|id| {
                            let raw = note
                                .as_ref()
                                .map(|(_, raw, _)| raw)
                                .or(self.raw_document.as_ref())?;
                            let change = child(child(raw.element(id)?, "w:rPr")?, "w:rPrChange")?;
                            attr(change, "w:id")
                        }) == revision.id
                });
                (run.and_then(|run| run.node_id), "w:rPr", "w:rPrChange")
            };
            let raw = if let Some((_, raw, _)) = note.as_mut() {
                raw
            } else if let Some(raw) = self.raw_document.as_mut() {
                raw
            } else {
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
            if let Some((part, raw, _)) = note {
                self.upsert_xml_part_content(
                    &part,
                    raw.to_string().map_err(|error| error.to_string())?,
                );
            }
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
        for kind in [NoteKind::Footnote, NoteKind::Endnote] {
            if let Ok(part) = self.resolve_existing_notes_part_path(kind) {
                if let Some(xml) = self.read_xml_part_content(&part) {
                    let mut notes = DocumentParser::new()
                        .parse_notes_xml(&xml, kind.note_tag().as_bytes())
                        .map_err(|error| error.to_string())?;
                    Self::capture_note_run_props(&mut notes, &xml, kind.note_tag());
                    let notes = notes
                        .into_iter()
                        .filter(|note| {
                            self.document.as_ref().is_some_and(|document| match kind {
                                NoteKind::Footnote => document.footnotes.contains_key(&note.id),
                                NoteKind::Endnote => document.endnotes.contains_key(&note.id),
                            })
                        })
                        .map(|note| (note.id.clone(), note))
                        .collect();
                    match kind {
                        NoteKind::Footnote => parsed.footnotes = notes,
                        NoteKind::Endnote => parsed.endnotes = notes,
                    }
                }
            }
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
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        self.document = original;
        self.rebuild_paragraph_surfaces();
        for index in 0..self.paragraph_count() {
            if let Some(paragraph) = self.paragraph_mut(index) {
                let saved = if paragraph.node_id.is_none() {
                    properties.get(index)
                } else {
                    properties
                        .iter()
                        .find(|(id, _, _)| *id == paragraph.node_id)
                };
                if let Some((_, props, runs)) = saved {
                    paragraph.properties = props.clone();
                    for (run_index, run) in paragraph.runs.iter_mut().enumerate() {
                        let saved = if paragraph.node_id.is_none() {
                            runs.get(run_index)
                        } else {
                            runs.iter().find(|(id, _)| *id == run.node_id)
                        };
                        if let Some((_, props)) = saved {
                            run.properties = props.clone();
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
