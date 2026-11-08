use super::*;
use crate::drawing_xml::{attr, child, child_mut, get_attr, set_attr};

fn detached(name: &str, attributes: Vec<XmlAttribute>) -> XmlElement {
    XmlElement {
        id: XmlNodeId::new(UNASSIGNED),
        name: name.into(),
        attributes,
        children: Vec::new(),
    }
}

fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

fn style_reference(name: &str) -> bool {
    matches!(
        name,
        "w:pStyle"
            | "w:rStyle"
            | "w:tblStyle"
            | "w:basedOn"
            | "w:link"
            | "w:next"
            | "w:styleLink"
            | "w:numStyleLink"
    )
}

pub(super) fn references(element: &XmlElement, ids: &mut HashSet<String>) {
    if style_reference(&element.name) {
        if let Some(id) = attribute(element, "w:val") {
            ids.insert(id.to_owned());
        }
    }
    for node in &element.children {
        if let XmlNode::Element(child) = node {
            references(child, ids);
        }
    }
}

fn remap(element: &mut XmlElement, ids: &HashMap<String, String>) {
    if style_reference(&element.name) {
        for attr in &mut element.attributes {
            if attr.name == "w:val" {
                if let Some(id) = ids.get(&attr.value) {
                    attr.value = id.clone();
                }
            }
        }
    }
    for node in &mut element.children {
        if let XmlNode::Element(child) = node {
            remap(child, ids);
        }
    }
}

pub(super) fn signature(element: &XmlElement) -> String {
    let mut attributes: Vec<_> = element
        .attributes
        .iter()
        .filter(|a| !a.name.starts_with("xmlns"))
        .map(|a| (&a.name, &a.value))
        .collect();
    attributes.sort();
    let children: Vec<_> = element
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::Element(child) => Some(signature(child)),
            XmlNode::Text(text) if text.trim().is_empty() => None,
            other => Some(format!("{other:?}")),
        })
        .collect();
    format!("{:?}{attributes:?}{children:?}", element.name)
}

impl WordDocument {
    // A private base carries source document defaults without activating them
    // globally in the destination. Generate it deterministically for the
    // style/numbering dependency fixed point.
    fn object_style_source(&self) -> Result<(XmlElement, Option<String>), String> {
        let content = self
            .styles_part_path()
            .and_then(|path| self.read_xml_part_content(&path))
            .unwrap_or_else(StylesPart::word_default_xml_for_platform);
        let raw = DocxXmlDocument::parse(&content).map_err(|e| e.to_string())?;
        let mut root = raw.root().clone();
        let mut defaults = Vec::new();
        for (container, properties) in [("w:pPrDefault", "w:pPr"), ("w:rPrDefault", "w:rPr")] {
            let mut properties = child(&root, "w:docDefaults")
                .and_then(|defaults| child(defaults, container))
                .and_then(|defaults| child(defaults, properties))
                .cloned()
                .unwrap_or_else(|| detached(properties, Vec::new()));
            if properties.name == "w:pPr" {
                // Missing paragraph spacing means zero in the source. Prevent
                // destination document defaults from filling in those gaps.
                if child(&properties, "w:spacing").is_none() {
                    properties
                        .children
                        .push(XmlNode::Element(detached("w:spacing", Vec::new())));
                }
                let spacing = child_mut(&mut properties, "w:spacing").unwrap();
                for (points, lines) in [("w:before", "w:beforeLines"), ("w:after", "w:afterLines")]
                {
                    if get_attr(spacing, points).is_none() && get_attr(spacing, lines).is_none() {
                        set_attr(spacing, points, Some("0".into()));
                    }
                }
            } else if child(&properties, "w:color").is_none() {
                // No color anywhere in the source hierarchy means automatic,
                // not permission to inherit the destination's document color.
                properties.children.push(XmlNode::Element(detached(
                    "w:color",
                    vec![attr("w:val", "auto")],
                )));
            }
            defaults.push(properties);
        }
        let mut reserved = HashSet::new();
        references(&root, &mut reserved);
        if let Some(document) = self.raw_document.as_ref() {
            references(document.root(), &mut reserved);
        }
        let paragraph_ids: HashSet<String> = root
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                if let Some(id) = get_attr(style, "w:styleId") {
                    reserved.insert(id.to_owned());
                }
                (style.name == "w:style" && get_attr(style, "w:type") == Some("paragraph"))
                    .then(|| get_attr(style, "w:styleId").map(str::to_owned))
                    .flatten()
            })
            .collect();
        let id = (1u32..)
            .map(|n| format!("JadeClipboardDefaults{n}"))
            .find(|id| !reserved.contains(id))
            .ok_or("style IDs exhausted")?;
        for node in &mut root.children {
            let XmlNode::Element(style) = node else {
                continue;
            };
            if style.name != "w:style" || get_attr(style, "w:type") != Some("paragraph") {
                continue;
            }
            let based_on = child(style, "w:basedOn").and_then(|base| get_attr(base, "w:val"));
            if based_on.is_some_and(|id| paragraph_ids.contains(id)) {
                continue;
            }
            if let Some(base) = child_mut(style, "w:basedOn") {
                set_attr(base, "w:val", Some(id.clone()));
            } else {
                let position = style.children.iter().position(|node| {
                    matches!(node, XmlNode::Element(element) if !matches!(element.name.as_str(), "w:name" | "w:aliases"))
                }).unwrap_or(style.children.len());
                style.children.insert(
                    position,
                    XmlNode::Element(detached("w:basedOn", vec![attr("w:val", &id)])),
                );
            }
        }
        let mut base = detached(
            "w:style",
            vec![attr("w:type", "paragraph"), attr("w:styleId", &id)],
        );
        base.children.push(XmlNode::Element(detached(
            "w:name",
            vec![attr("w:val", "Copied document defaults")],
        )));
        base.children
            .extend(defaults.into_iter().map(XmlNode::Element));
        root.children.push(XmlNode::Element(base));
        Ok((root, Some(id)))
    }

    pub(super) fn prepare_object_default_styles(
        &self,
        drawing: &mut XmlElement,
    ) -> Result<(), String> {
        let (root, base_id) = self.object_style_source()?;
        let paragraph_styles: Vec<_> = root
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                (style.name == "w:style" && get_attr(style, "w:type") == Some("paragraph"))
                    .then_some(style)
            })
            .collect();
        let default = paragraph_styles
            .iter()
            .find(|style| matches!(get_attr(style, "w:default"), Some("1" | "true" | "on")))
            .or_else(|| {
                paragraph_styles
                    .iter()
                    .find(|style| get_attr(style, "w:styleId") == Some("Normal"))
            })
            .and_then(|style| get_attr(style, "w:styleId"))
            .or(base_id.as_deref());
        let Some(default) = default else {
            return Ok(());
        };
        let ids: HashSet<_> = paragraph_styles
            .iter()
            .filter_map(|style| get_attr(style, "w:styleId"))
            .collect();
        fn assign(element: &mut XmlElement, default: &str, ids: &HashSet<&str>) {
            if element.name == "w:p" {
                if child(element, "w:pPr").is_none() {
                    element
                        .children
                        .insert(0, XmlNode::Element(detached("w:pPr", Vec::new())));
                }
                let properties = child_mut(element, "w:pPr").unwrap();
                let valid = child(properties, "w:pStyle")
                    .and_then(|style| get_attr(style, "w:val"))
                    .is_some_and(|id| ids.contains(id));
                if !valid {
                    if let Some(style) = child_mut(properties, "w:pStyle") {
                        set_attr(style, "w:val", Some(default.into()));
                    } else {
                        properties.children.insert(
                            0,
                            XmlNode::Element(detached("w:pStyle", vec![attr("w:val", default)])),
                        );
                    }
                }
            }
            for node in &mut element.children {
                if let XmlNode::Element(child) = node {
                    assign(child, default, ids);
                }
            }
        }
        assign(drawing, default, &ids);
        Ok(())
    }

    pub(super) fn copy_object_styles(
        &self,
        drawing: &XmlElement,
    ) -> Result<Vec<XmlElement>, String> {
        let (root, _) = self.object_style_source()?;
        let definitions: HashMap<_, _> = root
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                (style.name == "w:style")
                    .then(|| attribute(style, "w:styleId").map(|id| (id.to_owned(), style)))
                    .flatten()
            })
            .collect();
        let mut needed = HashSet::new();
        references(drawing, &mut needed);
        let mut pending: Vec<_> = needed.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            if let Some(style) = definitions.get(&id) {
                let mut dependencies = HashSet::new();
                references(style, &mut dependencies);
                for dependency in dependencies {
                    if needed.insert(dependency.clone()) {
                        pending.push(dependency);
                    }
                }
            }
        }
        // Source order keeps the clipboard and resulting package deterministic.
        Ok(root
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                if style.name != "w:style"
                    || !attribute(style, "w:styleId").is_some_and(|id| needed.contains(id))
                {
                    return None;
                }
                let mut style = style.clone();
                for attr in &root.attributes {
                    if attr.name.starts_with("xmlns")
                        && !style.attributes.iter().any(|a| a.name == attr.name)
                    {
                        style.attributes.push(attr.clone());
                    }
                }
                Some(style)
            })
            .collect())
    }

    /// Import only definitions used by a story, including changed ancestors.
    /// Reuse unchanged host styles; fork changed definitions and their users
    /// through the clipboard importer so body styles keep their meaning.
    /// Isolate source document defaults in a story-local paragraph base.
    pub(super) fn prepare_story_document_defaults(
        &self,
        source: &mut XmlElement,
    ) -> Result<(), String> {
        let current_xml = self
            .styles_part_path()
            .and_then(|path| self.read_xml_part_content(&path))
            .unwrap_or_else(StylesPart::word_default_xml_for_platform);
        let current = DocxXmlDocument::parse(&current_xml).map_err(|e| e.to_string())?;
        let properties = |root: &XmlElement, container: &str, name: &str| {
            child(root, "w:docDefaults")
                .and_then(|defaults| child(defaults, container))
                .and_then(|defaults| child(defaults, name))
                .cloned()
                .unwrap_or_else(|| detached(name, Vec::new()))
        };
        let mut paragraph = properties(source, "w:pPrDefault", "w:pPr");
        let mut run = properties(source, "w:rPrDefault", "w:rPr");
        let old_paragraph = properties(current.root(), "w:pPrDefault", "w:pPr");
        let old_run = properties(current.root(), "w:rPrDefault", "w:rPr");
        if signature(&paragraph) == signature(&old_paragraph)
            && signature(&run) == signature(&old_run)
        {
            return Ok(());
        }
        // A style toggle is relative to the destination document defaults.
        // Encode the difference, rather than treating an absolute default as
        // another toggle (which would invert bold/italic on repeated imports).
        for name in [
            "w:b",
            "w:bCs",
            "w:i",
            "w:iCs",
            "w:caps",
            "w:smallCaps",
            "w:strike",
            "w:outline",
            "w:shadow",
            "w:emboss",
            "w:imprint",
            "w:vanish",
        ] {
            let enabled = |properties: &XmlElement| {
                child(properties, name).is_some_and(|value| {
                    !matches!(get_attr(value, "w:val"), Some("0" | "false" | "off"))
                })
            };
            let toggle = enabled(&run) ^ enabled(&old_run);
            run.children
                .retain(|node| !matches!(node, XmlNode::Element(value) if value.name == name));
            if toggle {
                run.children
                    .push(XmlNode::Element(detached(name, vec![attr("w:val", "1")])));
            }
        }
        // Explicit neutral values stop removed source defaults from leaking
        // back in through the destination defaults. Preserve partial spacing
        // independently: before/after, automatic spacing and line height.
        if child(&run, "w:color").is_none() && child(&old_run, "w:color").is_some() {
            run.children.push(XmlNode::Element(detached(
                "w:color",
                vec![attr("w:val", "auto")],
            )));
        }
        // These scalar properties inherit literally (unlike style toggles).
        // Omitting a removed default would therefore retain the host value.
        for (name, neutral) in [
            ("w:u", "none"),
            ("w:highlight", "none"),
            ("w:vertAlign", "baseline"),
        ] {
            if child(&run, name).is_none() && child(&old_run, name).is_some() {
                run.children.push(XmlNode::Element(detached(
                    name,
                    vec![attr("w:val", neutral)],
                )));
            }
        }
        if child(&old_paragraph, "w:spacing").is_some() {
            if child(&paragraph, "w:spacing").is_none() {
                paragraph
                    .children
                    .push(XmlNode::Element(detached("w:spacing", Vec::new())));
            }
            let spacing = child_mut(&mut paragraph, "w:spacing").unwrap();
            for (points, lines) in [("w:before", "w:beforeLines"), ("w:after", "w:afterLines")] {
                if get_attr(spacing, points).is_none() && get_attr(spacing, lines).is_none() {
                    set_attr(spacing, points, Some("0".into()));
                    set_attr(spacing, lines, Some("0".into()));
                }
            }
            for name in ["w:beforeAutospacing", "w:afterAutospacing"] {
                if get_attr(spacing, name).is_none() {
                    set_attr(spacing, name, Some("0".into()));
                }
            }
            if get_attr(spacing, "w:line").is_none() {
                set_attr(spacing, "w:line", Some("240".into()));
                set_attr(spacing, "w:lineRule", Some("auto".into()));
            } else if get_attr(spacing, "w:lineRule").is_none() {
                set_attr(spacing, "w:lineRule", Some("auto".into()));
            }
        }
        let paragraph_ids: HashSet<String> = source
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                (style.name == "w:style" && get_attr(style, "w:type") == Some("paragraph"))
                    .then(|| get_attr(style, "w:styleId").map(str::to_owned))
                    .flatten()
            })
            .collect();
        let mut reserved = HashSet::new();
        references(source, &mut reserved);
        for node in &source.children {
            if let XmlNode::Element(style) = node {
                if let Some(id) = get_attr(style, "w:styleId") {
                    reserved.insert(id.to_owned());
                }
            }
        }
        let id = (1u32..)
            .map(|n| format!("JadeStoryDefaults{n}"))
            .find(|id| !reserved.contains(id))
            .ok_or("style IDs exhausted")?;
        // A cyclic basedOn chain has no root to attach to the defaults.
        // Preserve the reader's first-visit order for every cycle entry: each
        // original entry gets its own finite tail ending at the defaults.
        // Cutting one shared edge would lose inherited properties for other
        // entries (and change which scalar property wins).
        let definitions: HashMap<String, XmlElement> = source
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                let key = get_attr(style, "w:styleId")?;
                paragraph_ids
                    .contains(key)
                    .then(|| (key.to_owned(), style.clone()))
            })
            .collect();
        let mut visited = HashSet::new();
        let mut cycle_parents = HashMap::new();
        let mut tails = Vec::new();
        let mut next_tail = 1u32;
        for node in &source.children {
            let XmlNode::Element(style) = node else {
                continue;
            };
            let Some(start) =
                get_attr(style, "w:styleId").filter(|key| paragraph_ids.contains(*key))
            else {
                continue;
            };
            let mut path = Vec::<String>::new();
            let mut positions = HashMap::new();
            let mut cursor = start;
            while let Some(definition) = definitions.get(cursor) {
                if let Some(&position) = positions.get(cursor) {
                    let cycle = &path[position..];
                    for entry in 0..cycle.len() {
                        let mut parent = id.clone();
                        for offset in (1..cycle.len()).rev() {
                            let original = &definitions[&cycle[(entry + offset) % cycle.len()]];
                            let tail_id = loop {
                                let candidate = format!("JadeStoryCycle{next_tail}");
                                next_tail =
                                    next_tail.checked_add(1).ok_or("style IDs exhausted")?;
                                if reserved.insert(candidate.clone()) {
                                    break candidate;
                                }
                            };
                            let mut tail = original.clone();
                            set_attr(&mut tail, "w:styleId", Some(tail_id.clone()));
                            set_attr(&mut tail, "w:default", None);
                            // A duplicate built-in Normal name is merged on
                            // load even when its ID and default flag differ.
                            if let Some(name) = child_mut(&mut tail, "w:name") {
                                set_attr(
                                    name,
                                    "w:val",
                                    Some(format!("Story cycle tail ({tail_id})")),
                                );
                            }

                            // Every member of a cycle necessarily has basedOn.
                            set_attr(
                                child_mut(&mut tail, "w:basedOn").unwrap(),
                                "w:val",
                                Some(parent),
                            );
                            tails.push(XmlNode::Element(tail));
                            parent = tail_id;
                        }
                        cycle_parents.insert(cycle[entry].clone(), parent);
                    }
                    break;
                }
                if visited.contains(cursor) {
                    break;
                }
                positions.insert(cursor.to_owned(), path.len());
                path.push(cursor.to_owned());
                let Some(parent) =
                    child(definition, "w:basedOn").and_then(|base| get_attr(base, "w:val"))
                else {
                    break;
                };
                cursor = parent;
            }
            visited.extend(path);
        }
        let mut has_default = false;
        for node in &mut source.children {
            let XmlNode::Element(style) = node else {
                continue;
            };
            if style.name != "w:style" || get_attr(style, "w:type") != Some("paragraph") {
                continue;
            }
            has_default |= matches!(get_attr(style, "w:default"), Some("1" | "true" | "on"))
                || get_attr(style, "w:styleId") == Some("Normal");
            if let Some(parent) =
                get_attr(style, "w:styleId").and_then(|key| cycle_parents.get(key))
            {
                set_attr(
                    child_mut(style, "w:basedOn").unwrap(),
                    "w:val",
                    Some(parent.clone()),
                );
                continue;
            }
            if child(style, "w:basedOn")
                .and_then(|base| get_attr(base, "w:val"))
                .is_some_and(|id| paragraph_ids.contains(id))
            {
                continue;
            }
            if let Some(base) = child_mut(style, "w:basedOn") {
                set_attr(base, "w:val", Some(id.clone()));
            } else {
                let position = style.children.iter().position(|node| {
                    matches!(node, XmlNode::Element(value) if !matches!(value.name.as_str(), "w:name" | "w:aliases"))
                }).unwrap_or(style.children.len());
                style.children.insert(
                    position,
                    XmlNode::Element(detached("w:basedOn", vec![attr("w:val", &id)])),
                );
            }
        }
        source.children.extend(tails);
        let mut base = detached(
            "w:style",
            vec![attr("w:type", "paragraph"), attr("w:styleId", &id)],
        );
        if !has_default {
            set_attr(&mut base, "w:default", Some("1".into()));
        }
        base.children.push(XmlNode::Element(detached(
            "w:name",
            vec![attr("w:val", "Story document defaults")],
        )));
        base.children.push(XmlNode::Element(paragraph));
        base.children.push(XmlNode::Element(run));
        source.children.push(XmlNode::Element(base));
        Ok(())
    }

    /// Make implicit default styles explicit only when the source default
    /// resolves differently. The destination's document-wide defaults stay put.
    pub(super) fn prepare_story_implicit_styles(
        &self,
        source: &XmlElement,
        story: &mut XmlElement,
    ) -> Result<(), String> {
        let definitions = |root: &XmlElement| -> HashMap<String, XmlElement> {
            root.children
                .iter()
                .filter_map(|node| {
                    let XmlNode::Element(style) = node else {
                        return None;
                    };
                    (style.name == "w:style")
                        .then(|| {
                            get_attr(style, "w:styleId").map(|id| (id.to_owned(), style.clone()))
                        })
                        .flatten()
                })
                .collect()
        };
        let incoming = definitions(source);
        let current_xml = self
            .styles_part_path()
            .and_then(|path| self.read_xml_part_content(&path))
            .unwrap_or_else(StylesPart::word_default_xml_for_platform);
        let current = DocxXmlDocument::parse(&current_xml).map_err(|e| e.to_string())?;
        let existing = definitions(current.root());
        let default_id = |root: &XmlElement, kind: &str| -> Option<String> {
            root.children
                .iter()
                .filter_map(|node| match node {
                    XmlNode::Element(style)
                        if style.name == "w:style" && get_attr(style, "w:type") == Some(kind) =>
                    {
                        Some(style)
                    }
                    _ => None,
                })
                .find(|style| matches!(get_attr(style, "w:default"), Some("1" | "true" | "on")))
                .and_then(|style| get_attr(style, "w:styleId"))
                .map(str::to_owned)
                .or_else(|| {
                    (kind == "paragraph"
                        && root.children.iter().any(|node| {
                            matches!(node, XmlNode::Element(style) if style.name == "w:style"
                        && get_attr(style, "w:type") == Some("paragraph")
                        && get_attr(style, "w:styleId") == Some("Normal"))
                        }))
                    .then(|| "Normal".into())
                })
        };
        for (kind, element_name, properties_name, reference_name) in [
            ("paragraph", "w:p", "w:pPr", "w:pStyle"),
            ("table", "w:tbl", "w:tblPr", "w:tblStyle"),
        ] {
            let Some(default) = default_id(source, kind) else {
                continue;
            };
            let mut changed = default_id(current.root(), kind).as_ref() != Some(&default);
            let mut pending = vec![default.clone()];
            let mut visited = HashSet::new();
            while let Some(id) = pending.pop() {
                if !visited.insert(id.clone()) {
                    continue;
                }
                if let Some(style) = incoming.get(&id) {
                    changed |= existing
                        .get(&id)
                        .is_none_or(|old| signature(old) != signature(style));
                    let mut dependencies = HashSet::new();
                    references(style, &mut dependencies);
                    pending.extend(dependencies);
                }
            }
            #[allow(clippy::too_many_arguments)]
            fn assign(
                element: &mut XmlElement,
                name: &str,
                properties: &str,
                reference: &str,
                kind: &str,
                default: &str,
                definitions: &HashMap<String, XmlElement>,
                changed: bool,
            ) {
                if element.name == name
                    && (changed
                        || child(element, properties)
                            .and_then(|properties| child(properties, reference))
                            .is_some())
                {
                    if child(element, properties).is_none() {
                        element
                            .children
                            .insert(0, XmlNode::Element(detached(properties, Vec::new())));
                    }
                    let properties = child_mut(element, properties).unwrap();
                    let valid = child(properties, reference)
                        .and_then(|style| get_attr(style, "w:val"))
                        .and_then(|id| definitions.get(id))
                        .is_some_and(|style| get_attr(style, "w:type") == Some(kind));
                    if !valid {
                        if let Some(style) = child_mut(properties, reference) {
                            set_attr(style, "w:val", Some(default.into()));
                        } else {
                            properties.children.insert(
                                0,
                                XmlNode::Element(detached(reference, vec![attr("w:val", default)])),
                            );
                        }
                    }
                }
                for node in &mut element.children {
                    if let XmlNode::Element(child) = node {
                        assign(
                            child,
                            name,
                            properties,
                            reference,
                            kind,
                            default,
                            definitions,
                            changed,
                        );
                    }
                }
            }
            assign(
                story,
                element_name,
                properties_name,
                reference_name,
                kind,
                &default,
                &incoming,
                changed,
            );
        }
        Ok(())
    }

    pub(in crate::word_document) fn import_story_styles(
        &mut self,
        source: &XmlElement,
        story: &mut XmlElement,
    ) -> Result<bool, String> {
        let definitions = |root: &XmlElement| -> HashMap<String, XmlElement> {
            root.children
                .iter()
                .filter_map(|node| {
                    let XmlNode::Element(style) = node else {
                        return None;
                    };
                    (style.name == "w:style")
                        .then(|| {
                            attribute(style, "w:styleId").map(|id| (id.to_owned(), style.clone()))
                        })
                        .flatten()
                })
                .collect()
        };
        let incoming = definitions(source);
        let existing_xml = self
            .styles_part_path()
            .and_then(|path| self.read_xml_part_content(&path))
            .unwrap_or_else(StylesPart::word_default_xml_for_platform);
        let existing = DocxXmlDocument::parse(&existing_xml).map_err(|e| e.to_string())?;
        let existing = definitions(existing.root());
        let mut needed = HashSet::new();
        references(story, &mut needed);
        let mut pending: Vec<_> = needed.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            if let Some(style) = incoming.get(&id) {
                let mut dependencies = HashSet::new();
                references(style, &mut dependencies);
                for dependency in dependencies {
                    if needed.insert(dependency.clone()) {
                        pending.push(dependency);
                    }
                }
            }
        }
        let mut changed: HashSet<_> = needed
            .iter()
            .filter(|id| {
                incoming.get(*id).is_some_and(|style| {
                    existing
                        .get(*id)
                        .is_none_or(|old| signature(old) != signature(style))
                })
            })
            .cloned()
            .collect();
        // A style whose definition is unchanged still needs a private copy if
        // its base/linked/next style changed. Iterate to include cycles.
        loop {
            let count = changed.len();
            for id in &needed {
                if let Some(style) = incoming.get(id) {
                    let mut dependencies = HashSet::new();
                    references(style, &mut dependencies);
                    if dependencies.iter().any(|id| changed.contains(id)) {
                        changed.insert(id.clone());
                    }
                }
            }
            if changed.len() == count {
                break;
            }
        }
        let styles: Vec<_> = source
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                if style.name != "w:style"
                    || !attribute(style, "w:styleId").is_some_and(|id| changed.contains(id))
                {
                    return None;
                }
                let mut style = style.clone();
                for namespace in source
                    .attributes
                    .iter()
                    .filter(|a| a.name.starts_with("xmlns"))
                {
                    if !style.attributes.iter().any(|a| a.name == namespace.name) {
                        style.attributes.push(namespace.clone());
                    }
                }
                Some(style)
            })
            .collect();
        if styles.is_empty() {
            return Ok(false);
        }
        self.import_object_styles(&styles, story, &mut [])?;
        Ok(true)
    }

    pub(super) fn import_object_styles(
        &mut self,
        styles: &[XmlElement],
        drawing: &mut XmlElement,
        numbering: &mut [XmlElement],
    ) -> Result<(), String> {
        if styles.is_empty() {
            return Ok(());
        }
        let existing_path = self.styles_part_path();
        let path = if let Some(path) = existing_path.as_ref() {
            path.clone()
        } else {
            // An unlinked styles.xml is not the destination's style table.
            // Keep that part untouched instead of activating its definitions.
            let occupied = |path: &str| {
                self.parts.contains_key(path)
                    || self.binary_parts.contains_key(path)
                    || self
                        .package
                        .as_ref()
                        .is_some_and(|package| package.get(path).is_some())
            };
            if !occupied("word/styles.xml") {
                "word/styles.xml".into()
            } else {
                (1u32..)
                    .map(|n| format!("word/jade-paste-styles-{n}.xml"))
                    .find(|path| !occupied(path))
                    .ok_or("style part paths exhausted")?
            }
        };
        let content = self
            .read_xml_part_content(&path)
            .unwrap_or_else(StylesPart::word_default_xml_for_platform);
        let mut raw = DocxXmlDocument::parse(&content).map_err(|e| e.to_string())?;
        let mut taken: HashSet<String> = raw
            .root()
            .children
            .iter()
            .filter_map(|node| {
                let XmlNode::Element(style) = node else {
                    return None;
                };
                attribute(style, "w:styleId").map(str::to_owned)
            })
            .collect();
        // Even an unresolved reference in the destination must retain its
        // meaning rather than suddenly acquiring a pasted style definition.
        if let Some(document) = self.raw_document.as_ref() {
            references(document.root(), &mut taken);
        }
        for part in self.parts.values() {
            if let Ok(xml) = DocxXmlDocument::parse(&part.content) {
                references(xml.root(), &mut taken);
            }
        }
        let existing = taken.clone();
        // Reserve every source ID first: an alias must not consume an ID that
        // a later source style needs. Allocate all IDs before fixing cycles.
        taken.extend(
            styles
                .iter()
                .filter_map(|style| attribute(style, "w:styleId").map(str::to_owned)),
        );
        let mut ids = HashMap::new();
        for style in styles {
            let old = attribute(style, "w:styleId").ok_or("copied style has no ID")?;
            let new = if existing.contains(old) {
                (1u32..)
                    .map(|n| format!("{old}_jade_copy_{n}"))
                    .find(|id| !taken.contains(id))
                    .ok_or("style IDs exhausted")?
            } else {
                old.to_owned()
            };
            taken.insert(new.clone());
            ids.insert(old.to_owned(), new);
        }
        for source in styles {
            let mut style = source.clone();
            let old = attribute(source, "w:styleId").ok_or("copied style has no ID")?;
            style.attributes.retain(|a| a.name != "w:default");
            for attr in &mut style.attributes {
                if attr.name == "w:styleId" {
                    attr.value = ids[old].clone();
                }
            }
            remap(&mut style, &ids);
            if ids[old] != old {
                for node in &mut style.children {
                    if let XmlNode::Element(child) = node {
                        if child.name == "w:name" {
                            for attr in &mut child.attributes {
                                if attr.name == "w:val" {
                                    attr.value = format!("{} ({})", attr.value, ids[old]);
                                }
                            }
                        }
                    }
                }
            }
            // Source node IDs belong to another XML document.
            fn reset(element: &mut XmlElement) {
                element.id = XmlNodeId::new(UNASSIGNED);
                for node in &mut element.children {
                    if let XmlNode::Element(child) = node {
                        reset(child);
                    }
                }
            }
            reset(&mut style);
            Self::assign_unassigned_ids(&mut raw, &mut style);
            raw.insert_node(
                raw.root().id,
                raw.root().children.len(),
                XmlNode::Element(style),
            )
            .map_err(|e| e.to_string())?;
        }
        let serialized = raw.to_string().map_err(|e| e.to_string())?;
        self.styles_part = Some(StylesPart::from_xml(&serialized).map_err(|e| e.to_string())?);
        if let (Some(styles), Some(numbering)) = (&mut self.styles_part, &self.numbering_part) {
            styles.merge_numbering_from_part(numbering);
        }
        self.upsert_xml_part_content(&path, serialized);
        self.ensure_content_types_has_override(
            &path,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
        )?;
        if existing_path.is_none() {
            let part = self
                .document_part
                .as_mut()
                .ok_or("document part is missing")?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&part.relationships),
                r#type: RelationshipTypes::Styles.as_str().into(),
                target: format!("/{path}"),
                target_mode: None,
            };
            part.relationships.push(relationship.clone());
            self.ensure_document_relationships_part_has_relationship(&relationship)?;
        }
        remap(drawing, &ids);
        for definition in numbering {
            remap(definition, &ids);
        }
        Ok(())
    }
}
