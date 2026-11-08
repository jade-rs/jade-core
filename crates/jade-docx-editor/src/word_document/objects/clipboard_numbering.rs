use super::*;

type ObjectFormatting = (Vec<XmlElement>, Vec<XmlElement>, Vec<Relationship>);

// A number has three independent ID spaces: instances, abstract definitions,
// and picture bullets. Zero is reserved for removing numbering on a paragraph.
const KINDS: [(&str, &str, &str); 3] = [
    ("w:num", "w:numId", "w:numId"),
    ("w:abstractNum", "w:abstractNumId", "w:abstractNumId"),
    ("w:numPicBullet", "w:numPicBulletId", "w:lvlPicBulletId"),
];

fn attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

fn scan(element: &XmlElement, ids: &mut [HashSet<String>; 3]) {
    for (index, &(definition, id, reference)) in KINDS.iter().enumerate() {
        if element.name == definition {
            if let Some(value) = attr(element, id) {
                ids[index].insert(value.into());
            }
        }
        if element.name == reference {
            if let Some(value) = attr(element, "w:val") {
                ids[index].insert(value.into());
            }
        }
    }
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            scan(child, ids);
        }
    }
}

fn remap_numbers(element: &mut XmlElement, ids: &[HashMap<String, String>; 3]) {
    for (index, &(definition, id, reference)) in KINDS.iter().enumerate() {
        for a in &mut element.attributes {
            if (element.name == definition && a.name == id)
                || (element.name == reference && a.name == "w:val")
            {
                if let Some(value) = ids[index].get(&a.value) {
                    a.value = value.clone();
                }
            }
        }
    }
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            remap_numbers(child, ids);
        }
    }
}

fn detach(element: &mut XmlElement) {
    element.id = XmlNodeId::new(UNASSIGNED);
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            detach(child);
        }
    }
}

impl WordDocument {
    fn object_numbering_path(&self) -> Option<String> {
        let part = self.document_part.as_ref()?;
        let rel = part
            .relationships
            .iter()
            .find(|r| r.r#type == RelationshipTypes::Numbering.as_str())?;
        Some(Self::resolve_target_path(&part.path, &rel.target))
    }

    pub(super) fn copy_object_formatting(
        &self,
        drawing: &XmlElement,
    ) -> Result<ObjectFormatting, String> {
        let path = self.object_numbering_path();
        let raw = path
            .as_ref()
            .and_then(|p| self.read_xml_part_content(p))
            .map(|xml| DocxXmlDocument::parse(&xml).map_err(|e| e.to_string()))
            .transpose()?;
        let mut aggregate = drawing.clone();
        let mut previous = (usize::MAX, usize::MAX);
        let (styles, numbering) = loop {
            let styles = self.copy_object_styles(&aggregate)?;
            let mut needed: [HashSet<String>; 3] = Default::default();
            scan(drawing, &mut needed);
            for style in &styles {
                scan(style, &mut needed);
            }
            // Walk the instance -> abstract definition -> picture bullet graph.
            let numbering = if let Some(raw) = raw.as_ref() {
                loop {
                    let count: usize = needed.iter().map(HashSet::len).sum();
                    for node in &raw.root().children {
                        let XmlNode::Element(element) = node else {
                            continue;
                        };
                        if KINDS.iter().enumerate().any(|(i, &(name, id, _))| {
                            element.name == name
                                && attr(element, id).is_some_and(|id| {
                                    needed[i].contains(id) && !(i == 0 && id == "0")
                                })
                        }) {
                            scan(element, &mut needed);
                        }
                    }
                    if count == needed.iter().map(HashSet::len).sum::<usize>() {
                        break;
                    }
                }
                raw.root()
                    .children
                    .iter()
                    .filter_map(|node| {
                        let XmlNode::Element(element) = node else {
                            return None;
                        };
                        let selected = KINDS.iter().enumerate().any(|(i, &(name, id, _))| {
                            element.name == name
                                && attr(element, id).is_some_and(|id| {
                                    needed[i].contains(id) && !(i == 0 && id == "0")
                                })
                        });
                        if !selected {
                            return None;
                        }
                        let mut element = element.clone();
                        for a in &raw.root().attributes {
                            if a.name.starts_with("xmlns")
                                && !element.attributes.iter().any(|b| b.name == a.name)
                            {
                                element.attributes.push(a.clone());
                            }
                        }
                        Some(element)
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let count = (styles.len(), numbering.len());
            if previous == count {
                break (styles, numbering);
            }
            previous = count;
            aggregate = drawing.clone();
            aggregate
                .children
                .extend(styles.into_iter().chain(numbering).map(XmlNode::Element));
        };
        let mut ids = HashSet::new();
        for definition in &numbering {
            clip_references(definition, &mut ids);
        }
        let rels = path
            .as_ref()
            .and_then(|path| self.read_xml_part_content(&self.get_relationships_path(path)))
            .map(|xml| clip_relationships(&xml))
            .transpose()?
            .unwrap_or_default();
        let relationships: Vec<_> = rels
            .into_iter()
            .filter(|r| ids.contains(&r.id))
            .map(|mut r| {
                if r.target_mode.as_deref() != Some("External") {
                    r.target = format!(
                        "/{}",
                        Self::resolve_target_path(path.as_ref().unwrap(), &r.target)
                    );
                }
                r
            })
            .collect();
        if relationships.len() != ids.len() {
            return Err("numbering references a missing relationship".into());
        }
        Ok((styles, numbering, relationships))
    }

    /// Merge numbering and styles as one graph, including style-driven lists
    /// and picture bullets. Existing body definitions remain untouched.
    pub(in crate::word_document) fn import_story_formatting(
        &mut self,
        story: &mut XmlElement,
        files: &HashMap<String, Vec<u8>>,
        relationships: &[Relationship],
    ) -> Result<bool, String> {
        use super::clipboard_styles::{references, signature};
        let source_part = |kind: &str| -> Option<(String, String)> {
            let rel = relationships
                .iter()
                .find(|r| r.r#type == kind && r.target_mode.as_deref() != Some("External"))?;
            let path = Self::resolve_target_path("word/document.xml", &rel.target);
            Some((
                path.clone(),
                String::from_utf8_lossy(files.get(&path)?).into_owned(),
            ))
        };
        let styles_xml = source_part(RelationshipTypes::Styles.as_str())
            .or_else(|| source_part(RelationshipTypes::StylesWithEffects.as_str()))
            .map(|(_, xml)| xml)
            .unwrap_or_else(|| StylesPart::word_default_xml().into());
        let theme = source_part(RelationshipTypes::Theme.as_str())
            .map(|(_, xml)| ThemePart::from_xml(&xml).map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or_else(ThemePart::office_default);
        let mapping = source_part(RelationshipTypes::Settings.as_str())
            .map(|(_, xml)| DocumentSettings::from_xml(&xml).map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or_default()
            .color_scheme_mapping;
        let freeze_theme = theme
            != self
                .loaded_theme_part()
                .unwrap_or_else(ThemePart::office_default)
            || mapping != self.loaded_document_settings().color_scheme_mapping;
        let freeze = |element: &mut XmlElement| {
            if freeze_theme {
                super::clipboard_theme::freeze_theme(element, &theme, &mapping);
            }
        };
        freeze(story);
        let mut styles = DocxXmlDocument::parse(&styles_xml).map_err(|e| e.to_string())?;
        let root_id = styles.root().id;
        freeze(styles.element_mut(root_id).unwrap());
        self.prepare_story_document_defaults(styles.element_mut(root_id).unwrap())?;
        self.prepare_story_implicit_styles(styles.root(), story)?;
        let Some((source_path, numbering_xml)) = source_part(RelationshipTypes::Numbering.as_str())
        else {
            return self.import_story_styles(styles.root(), story);
        };
        let mut source = DocxXmlDocument::parse(&numbering_xml).map_err(|e| e.to_string())?;
        let root_id = source.root().id;
        freeze(source.element_mut(root_id).unwrap());
        let mut aggregate = story.clone();
        let mut selected_styles = HashSet::new();
        let mut selected: [HashSet<String>; 3] = Default::default();
        let mut definitions = Vec::new();
        loop {
            let previous = (selected_styles.len(), definitions.len());
            let mut needed_styles = HashSet::new();
            references(&aggregate, &mut needed_styles);
            for node in &styles.root().children {
                let XmlNode::Element(style) = node else {
                    continue;
                };
                if style.name == "w:style"
                    && attr(style, "w:styleId").is_some_and(|id| {
                        needed_styles.contains(id) && selected_styles.insert(id.to_owned())
                    })
                {
                    aggregate.children.push(XmlNode::Element(style.clone()));
                }
            }
            let mut needed: [HashSet<String>; 3] = Default::default();
            scan(&aggregate, &mut needed);
            for node in &source.root().children {
                let XmlNode::Element(definition) = node else {
                    continue;
                };
                if KINDS.iter().enumerate().any(|(i, &(name, id, _))| {
                    definition.name == name
                        && attr(definition, id).is_some_and(|id| {
                            !(i == 0 && id == "0")
                                && needed[i].contains(id)
                                && selected[i].insert(id.to_owned())
                        })
                }) {
                    let mut definition = definition.clone();
                    for namespace in source
                        .root()
                        .attributes
                        .iter()
                        .filter(|a| a.name.starts_with("xmlns"))
                    {
                        if !definition
                            .attributes
                            .iter()
                            .any(|a| a.name == namespace.name)
                        {
                            definition.attributes.push(namespace.clone());
                        }
                    }
                    aggregate
                        .children
                        .push(XmlNode::Element(definition.clone()));
                    definitions.push(definition);
                }
            }
            if previous == (selected_styles.len(), definitions.len()) {
                break;
            }
        }
        if definitions.is_empty() {
            return self.import_story_styles(styles.root(), story);
        }
        let existing_path = self.object_numbering_path();
        let current = existing_path
            .as_ref()
            .and_then(|path| self.read_xml_part_content(path))
            .map(|xml| DocxXmlDocument::parse(&xml).map_err(|e| e.to_string()))
            .transpose()?;
        let same_definition = |definition: &XmlElement| {
            current.as_ref().is_some_and(|raw| {
                raw.root().children.iter().any(|node| {
                    matches!(node, XmlNode::Element(old)
                if signature(old) == signature(definition))
                })
            })
        };
        let current_styles = self
            .styles_part_path()
            .and_then(|path| self.read_xml_part_content(&path))
            .unwrap_or_else(|| StylesPart::word_default_xml().into());
        let current_styles = DocxXmlDocument::parse(&current_styles).map_err(|e| e.to_string())?;
        let styles_changed = styles.root().children.iter().any(|node| {
            let XmlNode::Element(style) = node else { return false; };
            style.name == "w:style" && attr(style, "w:styleId").is_some_and(|id| selected_styles.contains(id))
                && !current_styles.root().children.iter().any(|node|
                    matches!(node, XmlNode::Element(old) if signature(old) == signature(style)))
        });
        let mut referenced = HashSet::new();
        for definition in &definitions {
            clip_references(definition, &mut referenced);
        }
        let source_rels_path = self.get_relationships_path(&source_path);
        let mut rels: Vec<_> = files
            .get(&source_rels_path)
            .map(|bytes| clip_relationships(&String::from_utf8_lossy(bytes)))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .filter(|r| referenced.contains(&r.id))
            .map(|mut r| {
                if r.target_mode.as_deref() != Some("External") {
                    r.target = format!("/{}", Self::resolve_target_path(&source_path, &r.target));
                }
                r
            })
            .collect();
        if rels.len() != referenced.len() {
            return Err("numbering references a missing relationship".into());
        }
        let current_rels = existing_path
            .as_ref()
            .and_then(|path| self.read_xml_part_content(&self.get_relationships_path(path)))
            .map(|xml| clip_relationships(&xml))
            .transpose()?
            .unwrap_or_default();
        let same_relationships = rels.iter().all(|r| {
            current_rels.iter().any(|old| {
                old.id == r.id
                    && old.r#type == r.r#type
                    && old.target_mode == r.target_mode
                    && (if old.target_mode.as_deref() == Some("External") {
                        old.target.clone()
                    } else {
                        format!(
                            "/{}",
                            Self::resolve_target_path(
                                existing_path.as_deref().unwrap_or("word/numbering.xml"),
                                &old.target
                            )
                        )
                    }) == r.target
            })
        });
        let assets_changed = self.import_story_assets("word/document.xml", &mut rels, files)?;
        if definitions.iter().all(same_definition)
            && same_relationships
            && !assets_changed
            && !styles_changed
        {
            return self.import_story_styles(styles.root(), story);
        }
        // Asset targets have already been made absolute and copied.
        let final_paths: HashMap<_, _> = rels
            .iter()
            .filter(|r| r.target_mode.as_deref() != Some("External"))
            .map(|r| {
                let path = r.target.trim_start_matches('/').to_owned();
                (path.clone(), path)
            })
            .collect();
        let clip = ObjectClip {
            drawing: story.clone(),
            styles: Vec::new(),
            numbering: definitions,
            numbering_relationships: rels,
            relationships: Vec::new(),
            parts: BTreeMap::new(),
        };
        self.import_object_formatting_with_story(&clip, story, &final_paths, Some(styles.root()))?;
        Ok(true)
    }

    pub(super) fn import_object_formatting(
        &mut self,
        clip: &ObjectClip,
        drawing: &mut XmlElement,
        paths: &HashMap<String, String>,
    ) -> Result<(), String> {
        self.import_object_formatting_with_story(clip, drawing, paths, None)
    }

    fn import_object_formatting_with_story(
        &mut self,
        clip: &ObjectClip,
        drawing: &mut XmlElement,
        paths: &HashMap<String, String>,
        story_styles: Option<&XmlElement>,
    ) -> Result<(), String> {
        let mut styles = clip.styles.clone();
        let mut definitions = clip.numbering.clone();
        if definitions.is_empty() {
            if let Some(source) = story_styles {
                self.import_story_styles(source, drawing)?;
                return Ok(());
            }
            return self.import_object_styles(&styles, drawing, &mut definitions);
        }
        let existing_path = self.object_numbering_path();
        let path = if let Some(path) = &existing_path {
            path.clone()
        } else {
            let occupied = |path: &str| {
                self.parts.contains_key(path)
                    || self.binary_parts.contains_key(path)
                    || self.package.as_ref().is_some_and(|p| p.get(path).is_some())
            };
            if !occupied("word/numbering.xml") {
                "word/numbering.xml".into()
            } else {
                (1u32..)
                    .map(|n| format!("word/jade-paste-numbering-{n}.xml"))
                    .find(|p| !occupied(p))
                    .ok_or("numbering paths exhausted")?
            }
        };
        let xml = self.read_xml_part_content(&path).unwrap_or_else(||
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.into());
        let mut raw = DocxXmlDocument::parse(&xml).map_err(|e| e.to_string())?;
        let mut used: [HashSet<String>; 3] = Default::default();
        scan(raw.root(), &mut used);
        if let Some(raw) = &self.raw_document {
            scan(raw.root(), &mut used);
        }
        for part in self.parts.values() {
            if let Ok(raw) = DocxXmlDocument::parse(&part.content) {
                scan(raw.root(), &mut used);
            }
        }
        let mut ids: [HashMap<String, String>; 3] = Default::default();
        for element in &definitions {
            for (i, &(name, id, _)) in KINDS.iter().enumerate() {
                if element.name != name {
                    continue;
                }
                let old = attr(element, id).ok_or("numbering definition has no ID")?;
                let new = (1u32..)
                    .map(|n| n.to_string())
                    .find(|n| !used[i].contains(n))
                    .ok_or("numbering IDs exhausted")?;
                used[i].insert(new.clone());
                ids[i].insert(old.into(), new);
            }
        }
        remap_numbers(drawing, &ids);
        for element in styles.iter_mut().chain(definitions.iter_mut()) {
            remap_numbers(element, &ids);
        }
        if let Some(source) = story_styles {
            let mut source = source.clone();
            remap_numbers(&mut source, &ids);
            let count = drawing.children.len();
            drawing
                .children
                .extend(definitions.into_iter().map(XmlNode::Element));
            self.import_story_styles(&source, drawing)?;
            definitions = drawing
                .children
                .split_off(count)
                .into_iter()
                .filter_map(|node| {
                    if let XmlNode::Element(element) = node {
                        Some(element)
                    } else {
                        None
                    }
                })
                .collect();
        } else {
            self.import_object_styles(&styles, drawing, &mut definitions)?;
        }
        let rel_path = self.get_relationships_path(&path);
        let mut rels = self
            .read_xml_part_content(&rel_path)
            .map(|xml| clip_relationships(&xml))
            .transpose()?
            .unwrap_or_default();
        let mut relationship_ids = HashMap::new();
        for r in &clip.numbering_relationships {
            let new = Relationship {
                id: Self::next_relationship_id(&rels),
                r#type: r.r#type.clone(),
                target: if r.target_mode.as_deref() == Some("External") {
                    r.target.clone()
                } else {
                    format!("/{}", paths[&Self::resolve_target_path(&path, &r.target)])
                },
                target_mode: r.target_mode.clone(),
            };
            relationship_ids.insert(r.id.clone(), new.id.clone());
            rels.push(new);
        }
        fn remap_rels(element: &mut XmlElement, ids: &HashMap<String, String>) {
            for a in &mut element.attributes {
                if a.name.starts_with("r:") || a.name == "o:relid" {
                    if let Some(id) = ids.get(&a.value) {
                        a.value = id.clone();
                    }
                }
            }
            for child in &mut element.children {
                if let XmlNode::Element(child) = child {
                    remap_rels(child, ids);
                }
            }
        }
        for mut definition in definitions {
            remap_rels(&mut definition, &relationship_ids);
            detach(&mut definition);
            Self::assign_unassigned_ids(&mut raw, &mut definition);
            // Schema order: picture bullets, abstract definitions, instances,
            // then numIdMacAtCleanup (if the destination has one).
            let rank = |name: &str| match name {
                "w:numPicBullet" => 0,
                "w:abstractNum" => 1,
                "w:num" => 2,
                _ => 3,
            };
            let at = raw.root().children.iter().position(|node| matches!(node, XmlNode::Element(e) if rank(&e.name) > rank(&definition.name)))
                .unwrap_or(raw.root().children.len());
            raw.insert_node(raw.root().id, at, XmlNode::Element(definition))
                .map_err(|e| e.to_string())?;
        }
        let serialized = raw.to_string().map_err(|e| e.to_string())?;
        let mut numbering = NumberingPart::from_xml(&serialized).map_err(|e| e.to_string())?;
        for bullet in numbering.picture_bullets.values_mut() {
            let Some(rel) = rels
                .iter()
                .find(|r| Some(r.id.as_str()) == bullet.relationship_id.as_deref())
            else {
                continue;
            };
            let image_path = Self::resolve_target_path(&path, &rel.target);
            if let Some(bytes) = self
                .binary_parts
                .get(&image_path)
                .cloned()
                .or_else(|| self.package.as_ref()?.get(&image_path).cloned())
            {
                bullet.data_uri = Some(format!(
                    "data:{};base64,{}",
                    Self::guess_image_mime_type(&image_path),
                    BASE64.encode(bytes)
                ));
            }
        }
        if let Some(styles) = &mut self.styles_part {
            styles.merge_numbering_from_part(&numbering);
        }
        self.numbering_part = Some(numbering);
        self.upsert_xml_part_content(&path, serialized);
        self.ensure_content_types_has_override(
            &path,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        )?;
        if !rels.is_empty() {
            let mut raw = DocxXmlDocument::parse(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#).map_err(|e| e.to_string())?;
            for r in &rels {
                let mut attrs = vec![
                    XmlAttribute {
                        name: "Id".into(),
                        value: r.id.clone(),
                    },
                    XmlAttribute {
                        name: "Type".into(),
                        value: r.r#type.clone(),
                    },
                    XmlAttribute {
                        name: "Target".into(),
                        value: r.target.clone(),
                    },
                ];
                if let Some(mode) = &r.target_mode {
                    attrs.push(XmlAttribute {
                        name: "TargetMode".into(),
                        value: mode.clone(),
                    });
                }
                let element = raw.create_element("Relationship", attrs, vec![]);
                raw.insert_node(
                    raw.root().id,
                    raw.root().children.len(),
                    XmlNode::Element(element),
                )
                .map_err(|e| e.to_string())?;
            }
            self.upsert_xml_part_content(&rel_path, raw.to_string().map_err(|e| e.to_string())?);
        }
        if let Some(part) = self.parts.get_mut(&path) {
            part.relationships = rels;
        }
        if existing_path.is_none() {
            let part = self
                .document_part
                .as_mut()
                .ok_or("document part is missing")?;
            let r = Relationship {
                id: Self::next_relationship_id(&part.relationships),
                r#type: RelationshipTypes::Numbering.as_str().into(),
                target: format!("/{path}"),
                target_mode: None,
            };
            part.relationships.push(r.clone());
            self.ensure_document_relationships_part_has_relationship(&r)?;
        }
        Ok(())
    }
}
