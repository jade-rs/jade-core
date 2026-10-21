//! Drawing objects edited as Word stores them.
//!
//! The 図の形式 and 図形の書式 tools change what a picture or shape *is* in
//! DrawingML — its outline, effects, corrections, geometry, name, the anchor's
//! options. Rather than mirroring each of those in the model and writing the
//! model back, an edit changes the object's `w:drawing` itself and the object
//! is read again from it, the way the document was loaded: what is drawn is
//! always what the package says, and what no tool touches stays as Word
//! wrote it.

use super::*;

/// Ids of elements built outside a document, for [`WordDocument::replace_object_drawing`]
/// to replace with ids of its own.
pub const UNASSIGNED: u32 = u32::MAX;

/// An element to put into a drawing; it gets an id when it is put in.
pub fn new_element(name: &str, attributes: Vec<XmlAttribute>, children: Vec<XmlNode>) -> XmlElement {
    XmlElement {
        id: XmlNodeId::new(UNASSIGNED),
        name: name.to_string(),
        attributes,
        children,
    }
}

/// [`new_element`] with element children only.
pub fn new_element_with(
    name: &str,
    attributes: Vec<XmlAttribute>,
    children: Vec<XmlElement>,
) -> XmlElement {
    new_element(
        name,
        attributes,
        children.into_iter().map(XmlNode::Element).collect(),
    )
}

/// Writes `element` and its subtree as XML text.
pub fn element_xml(element: &XmlElement) -> String {
    fn escape(text: &str, attribute: bool, out: &mut String) {
        for ch in text.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' if attribute => out.push_str("&quot;"),
                _ => out.push(ch),
            }
        }
    }
    fn write(element: &XmlElement, out: &mut String) {
        out.push('<');
        out.push_str(&element.name);
        for attribute in &element.attributes {
            out.push(' ');
            out.push_str(&attribute.name);
            out.push_str("=\"");
            escape(&attribute.value, true, out);
            out.push('"');
        }
        if element.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for child in &element.children {
            match child {
                XmlNode::Element(child) => write(child, out),
                XmlNode::Text(text) => escape(text, false, out),
                XmlNode::CData(data) => {
                    out.push_str("<![CDATA[");
                    out.push_str(data);
                    out.push_str("]]>");
                }
                XmlNode::Comment(_) => {}
            }
        }
        out.push_str("</");
        out.push_str(&element.name);
        out.push('>');
    }
    let mut out = String::new();
    write(element, &mut out);
    out
}

/// Parses an XML fragment — one element with its namespaces declared by the
/// caller — into a detached element whose ids are [`UNASSIGNED`].
pub fn parse_element(xml: &str) -> Result<XmlElement, String> {
    let document = DocxXmlDocument::parse(xml).map_err(|error| error.to_string())?;
    let mut element = document.root().clone();
    fn clear(element: &mut XmlElement) {
        element.id = XmlNodeId::new(UNASSIGNED);
        for child in &mut element.children {
            if let XmlNode::Element(child) = child {
                clear(child);
            }
        }
    }
    clear(&mut element);
    Ok(element)
}

/// Where a run's drawing sits inside the run: a child of it, or inside an
/// `mc:AlternateContent`'s choice.
enum DrawingSlot {
    Direct(usize),
    Choice {
        alternate: usize,
        choice: usize,
        drawing: usize,
    },
}

fn drawing_slot(run: &XmlElement) -> Option<DrawingSlot> {
    for (index, node) in run.children.iter().enumerate() {
        let XmlNode::Element(element) = node else {
            continue;
        };
        if element.name == "w:drawing" {
            return Some(DrawingSlot::Direct(index));
        }
        if element.name == "mc:AlternateContent" {
            if let Some((choice, drawing)) = WordDocument::alternate_content_choice_drawing(element)
            {
                return Some(DrawingSlot::Choice {
                    alternate: index,
                    choice,
                    drawing,
                });
            }
        }
    }
    None
}

impl WordDocument {
    /// The `w:drawing` of the picture or shape at `target`, as the package
    /// has it after every edit so far.
    pub fn object_drawing(&mut self, target: FloatingObjectTarget) -> Result<XmlElement, String> {
        self.ensure_roundtrip_xml()
            .map_err(|error| format!("the document cannot be edited: {error}"))?;
        let run_id = self.object_run_element_id(target)?;
        let raw = self
            .raw_document
            .as_ref()
            .ok_or("the document has no editable XML")?;
        let run = raw.element(run_id).ok_or("the object's run is missing")?;
        match drawing_slot(run).ok_or("the object has no drawing")? {
            DrawingSlot::Direct(index) => match &run.children[index] {
                XmlNode::Element(drawing) => Ok(drawing.clone()),
                _ => unreachable!(),
            },
            DrawingSlot::Choice {
                alternate,
                choice,
                drawing,
            } => {
                let XmlNode::Element(alternate) = &run.children[alternate] else {
                    unreachable!()
                };
                let XmlNode::Element(choice) = &alternate.children[choice] else {
                    unreachable!()
                };
                match &choice.children[drawing] {
                    XmlNode::Element(drawing) => Ok(drawing.clone()),
                    _ => unreachable!(),
                }
            }
        }
    }

    /// The id of the `w:r` holding the object at `target`, writing the
    /// paragraph's XML first when the run has none yet.
    fn object_run_element_id(&mut self, target: FloatingObjectTarget) -> Result<XmlNodeId, String> {
        let has_object = |run: &Run| match target.kind {
            FloatingObjectKind::Image => run.image.is_some(),
            FloatingObjectKind::TextBox => run.text_box.is_some(),
        };
        let run = self
            .paragraph(target.paragraph_index)
            .and_then(|paragraph| paragraph.runs.get(target.run_index))
            .filter(|run| has_object(run))
            .ok_or("the object is not in the document")?;
        if let Some(id) = run.node_id {
            return Ok(id);
        }
        self.sync_paragraph_text(target.paragraph_index);
        self.paragraph(target.paragraph_index)
            .and_then(|paragraph| paragraph.runs.get(target.run_index))
            .and_then(|run| run.node_id)
            .ok_or_else(|| "the object's run has no XML".to_string())
    }

    /// Puts `drawing` in place of the object's `w:drawing` and reads the
    /// object again from it. Elements built with [`new_element`] get ids
    /// here. A text box keeps its text — the paragraphs the caret edits — as
    /// the model has it.
    pub fn replace_object_drawing(
        &mut self,
        target: FloatingObjectTarget,
        mut drawing: XmlElement,
    ) -> Result<(), String> {
        let run_id = self.object_run_element_id(target)?;
        let reread = self.read_drawing(&drawing)?;
        // Whether the text inside changed (WordArt's text, a text colour):
        // then the model's text box paragraphs are read again too.
        let text_of = |drawing: &XmlElement| {
            crate::drawing_xml::descendant(drawing, "w:txbxContent").map(element_xml)
        };
        let content_changed = text_of(&drawing) != self.object_drawing(target).ok().as_ref().and_then(text_of);
        {
            let raw = self
                .raw_document
                .as_mut()
                .ok_or("the document has no editable XML")?;
            Self::assign_unassigned_ids(raw, &mut drawing);
            let run = raw.element_mut(run_id).ok_or("the object's run is missing")?;
            match drawing_slot(run).ok_or("the object has no drawing")? {
                DrawingSlot::Direct(index) => run.children[index] = XmlNode::Element(drawing),
                DrawingSlot::Choice {
                    alternate,
                    choice,
                    drawing: index,
                } => {
                    let XmlNode::Element(alternate) = &mut run.children[alternate] else {
                        unreachable!()
                    };
                    if let XmlNode::Element(choice) = &mut alternate.children[choice] {
                        choice.children[index] = XmlNode::Element(drawing);
                    }
                    // The fallback shows the object as it was before.
                    alternate.children.retain(|child| {
                        !matches!(child, XmlNode::Element(element) if element.name == "mc:Fallback")
                    });
                }
            }
            raw.rebuild_index_for_subtree(run_id);
        }
        let run = self
            .paragraph_mut(target.paragraph_index)
            .and_then(|paragraph| paragraph.runs.get_mut(target.run_index))
            .ok_or("the object is not in the document")?;
        match (reread.image, reread.text_box) {
            (Some(image), _) => {
                run.image = Some(image);
                run.text_box = None;
            }
            (None, Some(mut text_box)) => {
                let previous = run.text_box.take();
                let keep_text = previous.is_some() && !content_changed;
                if let Some(previous) = previous.filter(|_| keep_text) {
                    text_box.blocks = previous.blocks;
                    text_box.text = previous.text;
                }
                run.text_box = Some(text_box);
                run.image = None;
                if !keep_text {
                    // Its paragraphs are new: bind them to the XML they
                    // came from, as loading does.
                    if let (Some(document), Some(raw)) =
                        (self.document.as_mut(), self.raw_document.as_ref())
                    {
                        attach_text_box_xml_ids(Rc::make_mut(document), raw);
                    }
                }
            }
            (None, None) => return Err("the drawing holds no object jade can show".into()),
        }
        self.sync_paragraph_text_preserving_drawings(target.paragraph_index);
        self.rebuild_paragraph_surfaces();
        Ok(())
    }

    /// Reads a `w:drawing` the way loading reads the document's: its
    /// pictures, charts and SmartArt resolved against the package, its theme
    /// colours resolved against the theme.
    pub(crate) fn read_drawing(&self, drawing: &XmlElement) -> Result<Run, String> {
        let mut declarations = String::new();
        if let Some(raw) = self.raw_document.as_ref() {
            for attribute in &raw.root().attributes {
                if attribute.name.starts_with("xmlns") || attribute.name == "mc:Ignorable" {
                    declarations.push(' ');
                    declarations.push_str(&attribute.name);
                    declarations.push_str("=\"");
                    declarations.push_str(&attribute.value.replace('"', "&quot;"));
                    declarations.push('"');
                }
            }
        }
        for (prefix, uri) in DRAWING_NAMESPACES {
            let name = format!("xmlns:{prefix}");
            if !declarations.contains(&format!(" {name}=")) {
                declarations.push_str(&format!(" {name}=\"{uri}\""));
            }
        }
        let xml = format!(
            "<w:document{declarations}><w:body><w:p><w:r>{}</w:r></w:p></w:body></w:document>",
            element_xml(drawing)
        );
        let mut parsed = DocumentParser::new()
            .parse_document_xml(&xml)
            .map_err(|error| format!("the drawing cannot be read: {error}"))?;
        if let Some(doc_part) = self.document_part.as_ref() {
            // Only the parts this drawing can reach: its own media, and the
            // chart and diagram parts (small XML) a chart or SmartArt reads.
            let mut referenced = HashSet::new();
            fn collect(element: &XmlElement, out: &mut HashSet<String>) {
                for attribute in &element.attributes {
                    if attribute.name.starts_with("r:") {
                        out.insert(attribute.value.clone());
                    }
                }
                for child in &element.children {
                    if let XmlNode::Element(child) = child {
                        collect(child, out);
                    }
                }
            }
            collect(drawing, &mut referenced);
            let mut package = OpenXmlPackage {
                files: HashMap::new(),
                relationships: Vec::new(),
                original_size_bytes: None,
            };
            for relationship in &doc_part.relationships {
                let path = Self::resolve_target_path(&doc_part.path, &relationship.target);
                let wanted = referenced.contains(&relationship.id)
                    || path.starts_with("word/diagrams/")
                    || path.starts_with("word/charts/");
                if !wanted || relationship.target_mode.as_deref() == Some("External") {
                    continue;
                }
                let bytes = self
                    .binary_parts
                    .get(&path)
                    .cloned()
                    .or_else(|| {
                        self.parts
                            .get(&path)
                            .map(|part| part.content.clone().into_bytes())
                    })
                    .or_else(|| {
                        self.package
                            .as_ref()
                            .and_then(|package| package.get(&path))
                            .cloned()
                    });
                if let Some(bytes) = bytes {
                    package.files.insert(path, bytes);
                }
            }
            Self::resolve_images(&mut parsed, doc_part, &package);
            Self::resolve_charts(&mut parsed, doc_part, &package);
            Self::resolve_diagrams(&mut parsed, doc_part, &package);
        }
        if let Some(doc_part) = self.document_part.as_ref() {
            Self::resolve_hyperlinks(&mut parsed, doc_part);
        }
        // What loading does to the body, for the paragraphs of a text box.
        if let Some(body) = parsed.body.as_mut() {
            Self::capture_direct_run_fonts(&mut body.elements);
        }
        if let Some(styles_part) = self.styles_part.as_ref() {
            Self::apply_style_paragraph_defaults_to_document(&mut parsed, styles_part);
            Self::apply_table_style_cell_margins_to_document(&mut parsed, styles_part);
            Self::apply_table_styles_to_document(&mut parsed, styles_part);
            Self::apply_style_numbering_to_document(&mut parsed, styles_part);
            Self::apply_run_font_defaults_to_document(&mut parsed, styles_part);
        }
        // Without a theme part, Word draws with its Office theme.
        let theme = self.loaded_theme_part().unwrap_or_else(ThemePart::office_default);
        Self::apply_theme_to_document(&mut parsed, &theme);
        Self::force_apply_theme_to_document(&mut parsed, &theme);
        let run = parsed
            .body
            .and_then(|body| {
                body.elements.into_iter().find_map(|element| match element {
                    BodyElement::Paragraph(paragraph) => paragraph
                        .runs
                        .into_iter()
                        .find(|run| run.image.is_some() || run.text_box.is_some()),
                    _ => None,
                })
            })
            .ok_or("the drawing holds no object jade can show")?;
        Ok(run)
    }

    fn assign_unassigned_ids(raw: &mut DocxXmlDocument, element: &mut XmlElement) {
        if element.id.raw() == UNASSIGNED {
            element.id = raw.allocate_node_id();
        }
        for child in &mut element.children {
            if let XmlNode::Element(child) = child {
                Self::assign_unassigned_ids(raw, child);
            }
        }
    }

    /// Adds `bytes` as a new media part (`word/media/…`) related to the
    /// document, and returns the relationship's id.
    pub fn add_object_media(
        &mut self,
        bytes: &[u8],
        extension: &str,
        content_type: &str,
    ) -> Result<String, String> {
        let media_path = self.next_image_part_path("jade-image", extension);
        let relationship = {
            let document_part = self
                .document_part
                .as_mut()
                .ok_or_else(|| "main document part is unavailable".to_string())?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&document_part.relationships),
                r#type: RelationshipTypes::Image.as_str().to_string(),
                target: media_path
                    .strip_prefix("word/")
                    .unwrap_or(media_path.as_str())
                    .to_string(),
                target_mode: None,
            };
            document_part.relationships.push(relationship.clone());
            relationship
        };
        if let Some(document_part) = self.document_part.as_ref() {
            if let Some(part) = self.parts.get_mut(&document_part.path) {
                part.relationships = document_part.relationships.clone();
            }
        }
        self.ensure_document_relationships_part_has_relationship(&relationship)?;
        self.ensure_content_types_has_image(&media_path, extension, content_type)?;
        self.removed_parts.remove(&media_path);
        self.binary_parts.insert(media_path, bytes.to_vec());
        Ok(relationship.id)
    }

    /// The bytes of the part relationship `relationship_id` points at: a
    /// picture's media, a chart's or a diagram's XML.
    pub fn object_media(&self, relationship_id: &str) -> Option<Vec<u8>> {
        let path = self.object_part_path(relationship_id)?;
        self.binary_parts
            .get(&path)
            .cloned()
            .or_else(|| {
                self.parts
                    .get(&path)
                    .map(|part| part.content.clone().into_bytes())
            })
            .or_else(|| {
                self.package
                    .as_ref()
                    .and_then(|package| package.get(&path))
                    .cloned()
            })
    }

    fn object_part_path(&self, relationship_id: &str) -> Option<String> {
        let document_part = self.document_part.as_ref()?;
        let relationship = document_part
            .relationships
            .iter()
            .find(|relationship| relationship.id == relationship_id)?;
        Some(Self::resolve_target_path(&document_part.path, &relationship.target))
    }

    /// Adds a part at `{stem}{n}.{extension}` (the first free `n`), related
    /// to the document with `relationship_type` and declared with
    /// `content_type`; returns the relationship's id. For a SmartArt
    /// graphic's data, layout, style, colours and drawing.
    pub fn add_object_part(
        &mut self,
        stem: &str,
        extension: &str,
        bytes: &[u8],
        content_type: &str,
        relationship_type: &str,
    ) -> Result<String, String> {
        let path = (1..)
            .map(|index| format!("{stem}{index}.{extension}"))
            .find(|candidate| {
                !self.parts.contains_key(candidate)
                    && !self.binary_parts.contains_key(candidate)
                    && !self
                        .package
                        .as_ref()
                        .is_some_and(|package| package.files.contains_key(candidate))
            })
            .expect("an unbounded search finds a free name");
        let relationship = {
            let document_part = self
                .document_part
                .as_mut()
                .ok_or_else(|| "main document part is unavailable".to_string())?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&document_part.relationships),
                r#type: relationship_type.to_string(),
                target: path.strip_prefix("word/").unwrap_or(path.as_str()).to_string(),
                target_mode: None,
            };
            document_part.relationships.push(relationship.clone());
            relationship
        };
        if let Some(document_part) = self.document_part.as_ref() {
            if let Some(part) = self.parts.get_mut(&document_part.path) {
                part.relationships = document_part.relationships.clone();
            }
        }
        self.ensure_document_relationships_part_has_relationship(&relationship)?;
        self.ensure_content_types_has_override(&path, content_type)?;
        self.removed_parts.remove(&path);
        self.write_object_part(&path, bytes)?;
        Ok(relationship.id)
    }

    /// Writes new contents for the part `relationship_id` points at.
    pub fn replace_object_part(&mut self, relationship_id: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self
            .object_part_path(relationship_id)
            .ok_or("the part is not related to the document")?;
        self.write_object_part(&path, bytes)
    }

    fn write_object_part(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        if path.ends_with(".xml") {
            let text = String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())?;
            self.binary_parts.remove(path);
            self.upsert_xml_part_content(path, text);
        } else {
            self.binary_parts.insert(path.to_string(), bytes.to_vec());
        }
        Ok(())
    }

    /// A SmartArt graphic's drawing part relationship, which its data part
    /// names (`dsp:dataModelExt@relId`); `kind` is `diagramDrawing`.
    pub fn related_part_relationship(&self, data_relationship: &str, kind: &str) -> Option<String> {
        if kind != "diagramDrawing" {
            return None;
        }
        let data = self.object_media(data_relationship)?;
        super::diagram_drawing_relationship_id(std::str::from_utf8(&data).ok()?)
            .filter(|id| self.object_part_path(id).is_some())
    }

    /// A fresh `wp:docPr` / `cNvPr` id, unique in the document.
    pub fn new_drawing_object_id(&mut self) -> u32 {
        let _ = self.ensure_roundtrip_xml();
        self.raw_document
            .as_mut()
            .map(DocxXmlDocument::allocate_drawing_object_id)
            .unwrap_or(1)
    }
}

/// Namespaces a drawing may use that a document does not always declare.
pub const DRAWING_NAMESPACES: &[(&str, &str)] = &[
    (
        "w",
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    ),
    (
        "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    ),
    (
        "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    (
        "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
    ),
    (
        "wps",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    ),
    (
        "wpg",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    ),
    (
        "mc",
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
    ),
    ("a14", "http://schemas.microsoft.com/office/drawing/2010/main"),
    ("w14", "http://schemas.microsoft.com/office/word/2010/wordml"),
    (
        "wp14",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    ),
    ("c", "http://schemas.openxmlformats.org/drawingml/2006/chart"),
    (
        "dgm",
        "http://schemas.openxmlformats.org/drawingml/2006/diagram",
    ),
    (
        "asvg",
        "http://schemas.microsoft.com/office/drawing/2016/SVG/main",
    ),
];
