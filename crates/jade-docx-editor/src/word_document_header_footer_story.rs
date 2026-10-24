// Headers and footers edited as Word edits them: in place, as a story with
// every formatting, several paragraphs, tables and pictures, rather than as
// one line of text.
//
// A story is a part of its own (`word/header1.xml`) whose root holds the
// same block content a body does. So the editor opens it as a document of
// its own — [`WordDocument::story_docx`] packages the story's blocks as the
// body of a copy of this package, on a page as wide as the section's, with
// the same styles, numbering, theme and fonts — and the shell edits that
// document with every tool it has. [`WordDocument::replace_story_from_docx`]
// writes the edited body back into the part, its pictures and links with it.
//
// Word's 前と同じヘッダー/フッター (link to previous) is here too: a section
// without a reference of its own shows the previous section's story.

use super::{
    hf_reference_for_section, section_hf_reference, HeaderFooterRole, HeaderFooterVariant,
    WordDocument,
};
use crate::common::{Relationship, RelationshipTypes};
use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

/// Relationship ids a story's own relationships take in the story document,
/// so they cannot collide with the main document's.
const STORY_ID_PREFIX: &str = "jStory";

/// The main document's relationships a story document keeps: what its
/// paragraphs are styled, numbered and themed with.
fn is_infrastructure_relationship(kind: &str) -> bool {
    [
        "/styles",
        "/numbering",
        "/settings",
        "/fontTable",
        "/theme",
        "/webSettings",
        "/stylesWithEffects",
    ]
    .iter()
    .any(|suffix| kind.ends_with(suffix))
}

pub(crate) fn parse_role(role: &str) -> Result<HeaderFooterRole, String> {
    match role {
        "header" => Ok(HeaderFooterRole::Header),
        "footer" => Ok(HeaderFooterRole::Footer),
        other => Err(format!("unknown header/footer role: {other}")),
    }
}

pub(crate) fn parse_variant(variant: &str) -> Result<HeaderFooterVariant, String> {
    match variant {
        "default" => Ok(HeaderFooterVariant::Default),
        "first" => Ok(HeaderFooterVariant::First),
        "even" => Ok(HeaderFooterVariant::Even),
        other => Err(format!("unknown header/footer variant: {other}")),
    }
}

fn twips(points: f32) -> String {
    ((points * 20.0).round() as i64).max(0).to_string()
}

fn relationships_xml(relationships: &[Relationship]) -> Result<String, String> {
    let mut document = DocxXmlDocument::parse(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
    )
    .map_err(|err| err.to_string())?;
    let children = relationships
        .iter()
        .map(|relationship| {
            let mut attributes = vec![
                XmlAttribute {
                    name: "Id".to_string(),
                    value: relationship.id.clone(),
                },
                XmlAttribute {
                    name: "Type".to_string(),
                    value: relationship.r#type.clone(),
                },
                XmlAttribute {
                    name: "Target".to_string(),
                    value: relationship.target.clone(),
                },
            ];
            if let Some(mode) = relationship.target_mode.as_ref() {
                attributes.push(XmlAttribute {
                    name: "TargetMode".to_string(),
                    value: mode.clone(),
                });
            }
            XmlNode::Element(document.create_element("Relationship", attributes, Vec::new()))
        })
        .collect::<Vec<_>>();
    let root_id = document.root().id;
    document
        .element_mut(root_id)
        .ok_or_else(|| "relationships part has no root".to_string())?
        .children = children;
    document.rebuild_index();
    document.to_string().map_err(|err| err.to_string())
}

fn parse_relationships_xml(xml: &str) -> Result<Vec<Relationship>, String> {
    let document = DocxXmlDocument::parse(xml).map_err(|err| err.to_string())?;
    Ok(document
        .root()
        .children
        .iter()
        .filter_map(|child| match child {
            XmlNode::Element(element) if element.name.ends_with("Relationship") => {
                let attribute = |name: &str| {
                    element
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == name)
                        .map(|attribute| attribute.value.clone())
                };
                Some(Relationship {
                    id: attribute("Id")?,
                    r#type: attribute("Type")?,
                    target: attribute("Target")?,
                    target_mode: attribute("TargetMode"),
                })
            }
            _ => None,
        })
        .collect())
}

/// Renames the relationship ids the story's markup points through
/// (`r:embed`, `r:id`, `r:link`, VML's `o:relid`, …).
fn remap_relationship_ids(element: &mut XmlElement, ids: &HashMap<String, String>) {
    for attribute in &mut element.attributes {
        if attribute.name.starts_with("r:") || attribute.name == "o:relid" {
            if let Some(renamed) = ids.get(&attribute.value) {
                attribute.value = renamed.clone();
            }
        }
    }
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            remap_relationship_ids(child, ids);
        }
    }
}

fn read_zip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|err| err.to_string())?;
    let mut files = HashMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|err| err.to_string())?;
        if file.is_dir() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|err| err.to_string())?;
        files.insert(file.name().map_err(|err| err.to_string())?.into_owned(), contents);
    }
    Ok(files)
}

fn image_content_type(extension: &str) -> Option<&'static str> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        _ => return None,
    })
}

impl WordDocument {
    /// Whether `section`'s story is linked to the previous section's — it has
    /// no reference of its own. The first section is never linked.
    pub fn header_footer_linked(
        &self,
        section: usize,
        role: &str,
        variant: &str,
    ) -> Result<bool, String> {
        let role = parse_role(role)?;
        let variant = parse_variant(variant)?;
        if section == 0 {
            return Ok(false);
        }
        let document = self
            .document
            .as_deref()
            .ok_or_else(|| "document is unavailable".to_string())?;
        Ok(document
            .sections
            .get(section)
            .and_then(|range| range.properties.as_ref())
            .and_then(|properties| section_hf_reference(properties, role, variant))
            .is_none())
    }

    /// 前と同じヘッダー/フッター: linked, `section` drops its own reference
    /// and shows the previous section's story; unlinked, it gets a copy of
    /// the story it showed, which it can then change on its own. Returns
    /// whether anything changed.
    pub fn set_header_footer_link(
        &mut self,
        section: usize,
        role: &str,
        variant: &str,
        linked: bool,
    ) -> Result<bool, String> {
        let role_kind = parse_role(role)?;
        let variant_kind = parse_variant(variant)?;
        if section == 0 && linked {
            return Err("the first section has no previous section to link to".to_string());
        }
        if self.header_footer_linked(section, role, variant)? == linked {
            return Ok(false);
        }
        let previous_edit_section = self.hf_edit_section;
        self.hf_edit_section = Some(section);
        let result = if linked {
            self.link_story_to_previous(section, role_kind, variant_kind)
        } else {
            self.unlink_story(section, role_kind, variant_kind)
        };
        self.hf_edit_section = previous_edit_section;
        result.map(|_| true)
    }

    fn link_story_to_previous(
        &mut self,
        section: usize,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
    ) -> Result<(), String> {
        let inherited = self
            .document
            .as_deref()
            .and_then(|document| hf_reference_for_section(document, section - 1, role, variant));
        let targets_default = self.section_targets_default_sect_pr(Some(section));
        self.edit_section_properties(Some(section), |properties| match (role, variant) {
            (HeaderFooterRole::Header, HeaderFooterVariant::Default) => {
                properties.header_reference = None
            }
            (HeaderFooterRole::Footer, HeaderFooterVariant::Default) => {
                properties.footer_reference = None
            }
            (HeaderFooterRole::Header, HeaderFooterVariant::First) => {
                properties.header_reference_first = None
            }
            (HeaderFooterRole::Footer, HeaderFooterVariant::First) => {
                properties.footer_reference_first = None
            }
            (HeaderFooterRole::Header, HeaderFooterVariant::Even) => {
                properties.header_reference_even = None
            }
            (HeaderFooterRole::Footer, HeaderFooterVariant::Even) => {
                properties.footer_reference_even = None
            }
        });
        if let Some(sect_pr_id) = self.sect_pr_id_for_section(Some(section))? {
            if let Some(raw) = self.raw_document.as_mut() {
                let name = role.reference_element_name();
                let kind = variant.as_w_type();
                if let Some(sect_pr) = raw.element_mut(sect_pr_id) {
                    sect_pr.children.retain(|child| {
                        !matches!(child, XmlNode::Element(element)
                            if element.name == name
                                && element.attributes.iter().any(|attribute| {
                                    attribute.name == "w:type" && attribute.value == kind
                                }))
                    });
                }
                raw.rebuild_index_for_subtree(sect_pr_id);
            }
        }
        // The final section's references are also the document-wide ones
        // the flattened lines and the default header follow.
        if targets_default {
            if let Some(document) = self.document_mut() {
                match (role, variant) {
                    (HeaderFooterRole::Header, HeaderFooterVariant::Default) => {
                        document.header_reference = inherited.clone()
                    }
                    (HeaderFooterRole::Footer, HeaderFooterVariant::Default) => {
                        document.footer_reference = inherited.clone()
                    }
                    _ => {}
                }
            }
            if let Some(reference) = inherited {
                if let Some(path) = self.hf_part_path_for_reference(&reference) {
                    self.refresh_header_footer_lines_for(role, variant, &path)?;
                }
            }
        }
        Ok(())
    }

    fn unlink_story(
        &mut self,
        _section: usize,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
    ) -> Result<(), String> {
        let inherited = self.active_hf_reference(role, variant);
        let Some(source_path) = inherited
            .as_deref()
            .and_then(|reference| self.hf_part_path_for_reference(reference))
        else {
            // Nothing to copy: a blank story of its own, as Word gives a
            // section that had no header to inherit.
            return self.ensure_hf_part_for_edit(role, variant);
        };
        let content = self
            .read_xml_part_content(&source_path)
            .ok_or_else(|| format!("missing {source_path}"))?;
        let relationships = self
            .parts
            .get(&source_path)
            .map(|part| part.relationships.clone())
            .unwrap_or_default();
        self.create_story_part(role, variant, content, relationships)?;
        Ok(())
    }

    /// Adds a header or footer part holding `content`, reached from the
    /// editing section's `w:sectPr` for `variant`. Returns its path.
    fn create_story_part(
        &mut self,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
        content: String,
        relationships: Vec<Relationship>,
    ) -> Result<String, String> {
        let rel_type = match role {
            HeaderFooterRole::Header => RelationshipTypes::Header.as_str(),
            HeaderFooterRole::Footer => RelationshipTypes::Footer.as_str(),
        };
        let part_path = self.next_header_footer_part_path(role);
        let target = part_path
            .strip_prefix("word/")
            .unwrap_or(part_path.as_str())
            .to_string();
        let rel = {
            let doc_part = self
                .document_part
                .as_mut()
                .ok_or_else(|| "main document part is unavailable".to_string())?;
            let rel = Relationship {
                id: Self::next_relationship_id(&doc_part.relationships),
                r#type: rel_type.to_string(),
                target,
                target_mode: None,
            };
            doc_part.relationships.push(rel.clone());
            rel
        };
        if let Some(doc_part) = self.document_part.as_ref() {
            if let Some(part) = self.parts.get_mut(&doc_part.path) {
                part.relationships = doc_part.relationships.clone();
            }
        }
        self.ensure_raw_section_has_hf_reference(role, variant, &rel.id)?;
        self.ensure_document_relationships_part_has_relationship(&rel)?;
        self.ensure_content_types_has_override(&part_path, role.content_type())?;
        self.upsert_xml_part_content(&part_path, content);
        self.set_story_relationships(&part_path, relationships)?;
        self.set_document_hf_reference(role, variant, rel.id);
        self.refresh_story(role, variant, &part_path)?;
        Ok(part_path)
    }

    /// Writes a story part's relationships, model and `_rels` part both.
    fn set_story_relationships(
        &mut self,
        part_path: &str,
        relationships: Vec<Relationship>,
    ) -> Result<(), String> {
        let rels_path = self.get_relationships_path(part_path);
        if relationships.is_empty() && self.read_xml_part_content(&rels_path).is_none() {
            if let Some(part) = self.parts.get_mut(part_path) {
                part.relationships = relationships;
            }
            return Ok(());
        }
        let xml = relationships_xml(&relationships)?;
        self.upsert_xml_part_content(&rels_path, xml);
        if let Some(part) = self.parts.get_mut(part_path) {
            part.relationships = relationships;
        }
        Ok(())
    }

    /// Re-reads a story after its part changed, with any picture the
    /// package does not have yet (one added while it was edited) read from
    /// the media written beside it.
    fn refresh_story(
        &mut self,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
        part_path: &str,
    ) -> Result<(), String> {
        self.refresh_header_footer_lines_for(role, variant, part_path)?;
        let Some(part) = self.parts.get(part_path) else {
            return Ok(());
        };
        let media: HashMap<String, String> = part
            .relationships
            .iter()
            .filter(|relationship| relationship.r#type == RelationshipTypes::Image.as_str())
            .filter_map(|relationship| {
                let path = Self::resolve_target_path(part_path, &relationship.target);
                let bytes = self.binary_parts.get(&path)?;
                let extension = path.rsplit('.').next().unwrap_or_default();
                let mime = image_content_type(extension)?;
                use base64::Engine as _;
                Some((
                    relationship.id.clone(),
                    format!(
                        "data:{mime};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                ))
            })
            .collect();
        if media.is_empty() {
            return Ok(());
        }
        let Some(parsed) = self
            .parts
            .get_mut(part_path)
            .and_then(|part| part.parsed_document.as_mut())
        else {
            return Ok(());
        };
        let parsed = std::rc::Rc::make_mut(parsed);
        if let Some(body) = parsed.body.as_mut() {
            fill_story_images(&mut body.elements, &media);
        }
        Ok(())
    }

    /// The part a story edit of `section` writes to: the section's own, or
    /// the one it inherits (an edit of a linked story changes every section
    /// that shows it, as in Word). `None` when no section has one yet.
    fn story_part_path(
        &self,
        section: usize,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
    ) -> Option<String> {
        let document = self.document.as_deref()?;
        let reference = hf_reference_for_section(document, section, role, variant)?;
        self.hf_part_path_for_reference(&reference)
    }

    /// A .docx whose body is `section`'s header or footer story — its
    /// blocks, on a page as wide as the section's `page_number` (1-based,
    /// which side the binding is on), with this document's styles,
    /// numbering, theme and fonts — for an editor to open as a document of
    /// its own. A section with no story gets one empty paragraph in the
    /// header or footer style.
    pub fn story_docx(
        &self,
        section: usize,
        role: &str,
        variant: &str,
        page_number: usize,
    ) -> Result<Vec<u8>, String> {
        let role = parse_role(role)?;
        let variant = parse_variant(variant)?;
        let exported = self.export_docx().map_err(|err| format!("{err:?}"))?;
        let mut files = read_zip(&exported)?;
        let document_rels_path = "word/_rels/document.xml.rels";
        let main_relationships = files
            .get(document_rels_path)
            .map(|bytes| parse_relationships_xml(&String::from_utf8_lossy(bytes)))
            .transpose()?
            .unwrap_or_default();
        let mut relationships: Vec<Relationship> = main_relationships
            .into_iter()
            .filter(|relationship| is_infrastructure_relationship(&relationship.r#type))
            .collect();

        let part_path = self.story_part_path(section, role, variant);
        let (content, story_relationships) = match part_path.as_deref() {
            Some(path) => (
                self.read_xml_part_content(path)
                    .ok_or_else(|| format!("missing {path}"))?,
                self.parts
                    .get(path)
                    .map(|part| part.relationships.clone())
                    .unwrap_or_default(),
            ),
            None => {
                let style = self.styles_part.as_ref().and_then(|styles| {
                    styles
                        .resolve_paragraph_style_id(role.as_label())
                        .map(str::to_string)
                });
                let properties = style
                    .map(|style| format!(r#"<w:pPr><w:pStyle w:val="{style}"/></w:pPr>"#))
                    .unwrap_or_default();
                (
                    format!(
                        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<{root} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:p>{properties}</w:p></{root}>"#,
                        root = role.part_root_name()
                    ),
                    Vec::new(),
                )
            }
        };
        let renamed: HashMap<String, String> = story_relationships
            .iter()
            .map(|relationship| {
                (
                    relationship.id.clone(),
                    format!("{STORY_ID_PREFIX}{}", relationship.id),
                )
            })
            .collect();
        relationships.extend(story_relationships.into_iter().map(|relationship| Relationship {
            id: renamed[&relationship.id].clone(),
            ..relationship
        }));

        let mut story = DocxXmlDocument::parse(&content).map_err(|err| err.to_string())?;
        let layout = self.section_layout(Some(section));
        let margins = jade_docx_layout::section_transition::resolve_page_margins(
            layout.margin_left,
            layout.margin_right,
            layout.margin_top,
            layout.gutter,
            layout.gutter_at_top,
            layout.mirror_margins,
            page_number.max(1),
        );
        let mut section_attributes = format!(
            r#"<w:pgSz w:w="{}" w:h="{}"/><w:pgMar w:top="{}" w:right="{}" w:bottom="0" w:left="{}" w:header="0" w:footer="0" w:gutter="0"/><w:cols w:space="0"/>"#,
            twips(layout.width),
            twips(layout.height),
            twips(match role {
                HeaderFooterRole::Header => layout.header_distance,
                HeaderFooterRole::Footer => layout.footer_distance,
            }),
            twips(margins.right),
            twips(margins.left),
        );
        if layout.doc_grid.grid_type != "default" || layout.doc_grid.line_pitch.is_some() {
            section_attributes.push_str("<w:docGrid");
            if layout.doc_grid.grid_type != "default" {
                section_attributes.push_str(&format!(r#" w:type="{}""#, layout.doc_grid.grid_type));
            }
            if let Some(pitch) = layout.doc_grid.line_pitch {
                section_attributes.push_str(&format!(r#" w:linePitch="{}""#, twips(pitch)));
            }
            if let Some(pitch) = layout.doc_grid.char_pitch {
                section_attributes.push_str(&format!(
                    r#" w:charSpace="{}""#,
                    ((pitch - layout.default_font_size) * 4096.0).round() as i64
                ));
            }
            section_attributes.push_str("/>");
        }
        // The page number the story shows: the one the page has.
        section_attributes.push_str(&format!(
            r#"<w:pgNumType w:fmt="{}" w:start="{}"/>"#,
            layout.page_numbering.format,
            page_number.max(1)
        ));
        let section_xml = DocxXmlDocument::parse(&format!(
            r#"<w:sectPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{section_attributes}</w:sectPr>"#
        ))
        .map_err(|err| err.to_string())?;

        let root_id = story.root().id;
        let mut blocks = std::mem::take(
            &mut story
                .element_mut(root_id)
                .ok_or_else(|| "the story has no root".to_string())?
                .children,
        );
        let has_block = blocks.iter().any(|node| {
            matches!(node, XmlNode::Element(element) if matches!(element.name.as_str(), "w:p" | "w:tbl" | "w:sdt"))
        });
        if !has_block {
            blocks.push(XmlNode::Element(story.create_element("w:p", Vec::new(), Vec::new())));
        }
        for block in &mut blocks {
            if let XmlNode::Element(element) = block {
                remap_relationship_ids(element, &renamed);
            }
        }
        let section_properties = story.create_element(
            "w:sectPr",
            Vec::new(),
            section_xml.root().children.clone(),
        );
        blocks.push(XmlNode::Element(section_properties));
        let body = story.create_element("w:body", Vec::new(), blocks);
        let root = story
            .element_mut(root_id)
            .ok_or_else(|| "the story has no root".to_string())?;
        root.name = "w:document".to_string();
        root.children = vec![XmlNode::Element(body)];
        story.rebuild_index();
        let document_xml = story.to_string().map_err(|err| err.to_string())?;

        let mut overrides = HashMap::new();
        overrides.insert("word/document.xml".to_string(), document_xml.into_bytes());
        overrides.insert(
            document_rels_path.to_string(),
            relationships_xml(&relationships)?.into_bytes(),
        );
        // The story document has no comments, notes or headers of its own;
        // the parts stay in the package, unreferenced.
        files.retain(|path, _| !overrides.contains_key(path));
        jade_docx_writer::write_docx_with_removed(&files, &overrides, &HashSet::new())
            .map_err(|err| format!("{err:?}"))
    }

    /// Writes a story edited as the .docx [`Self::story_docx`] made back into
    /// `section`'s header or footer: its body becomes the part's content,
    /// with the pictures and links it points to. A section that had no story
    /// gets one. Returns whether the story changed.
    pub fn replace_story_from_docx(
        &mut self,
        section: usize,
        role: &str,
        variant: &str,
        bytes: &[u8],
    ) -> Result<bool, String> {
        let role = parse_role(role)?;
        let variant = parse_variant(variant)?;
        let files = read_zip(bytes)?;
        let document_xml = files
            .get("word/document.xml")
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .ok_or_else(|| "the story has no document part".to_string())?;
        let story_relationships = files
            .get("word/_rels/document.xml.rels")
            .map(|bytes| parse_relationships_xml(&String::from_utf8_lossy(bytes)))
            .transpose()?
            .unwrap_or_default();

        let mut carried = Vec::new();
        let mut renamed = HashMap::new();
        let mut taken = HashSet::new();
        for relationship in story_relationships
            .into_iter()
            .filter(|relationship| !is_infrastructure_relationship(&relationship.r#type))
        {
            let plain = relationship
                .id
                .strip_prefix(STORY_ID_PREFIX)
                .unwrap_or(&relationship.id)
                .to_string();
            let id = if taken.contains(&plain) {
                relationship.id.clone()
            } else {
                plain
            };
            taken.insert(id.clone());
            renamed.insert(relationship.id.clone(), id.clone());
            carried.push(Relationship { id, ..relationship });
        }

        let mut story = DocxXmlDocument::parse(&document_xml).map_err(|err| err.to_string())?;
        let root_id = story.root().id;
        let root = story
            .element_mut(root_id)
            .ok_or_else(|| "the story has no root".to_string())?;
        let body = root
            .children
            .iter_mut()
            .find_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:body" => Some(element),
                _ => None,
            })
            .ok_or_else(|| "the story has no body".to_string())?;
        let mut blocks: Vec<XmlNode> = std::mem::take(&mut body.children)
            .into_iter()
            .filter(|node| !matches!(node, XmlNode::Element(element) if element.name == "w:sectPr"))
            .collect();
        for block in &mut blocks {
            if let XmlNode::Element(element) = block {
                remap_relationship_ids(element, &renamed);
            }
        }
        root.name = role.part_root_name().to_string();
        root.children = blocks;
        story.rebuild_index();
        let content = story.to_string().map_err(|err| err.to_string())?;

        let previous_edit_section = self.hf_edit_section;
        self.hf_edit_section = Some(section);
        let result = self.write_story_part(section, role, variant, content, carried, &files);
        self.hf_edit_section = previous_edit_section;
        result
    }

    fn write_story_part(
        &mut self,
        section: usize,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
        content: String,
        relationships: Vec<Relationship>,
        files: &HashMap<String, Vec<u8>>,
    ) -> Result<bool, String> {
        let existing = self.story_part_path(section, role, variant);
        if let Some(path) = existing.as_deref() {
            let unchanged_relationships = self
                .parts
                .get(path)
                .is_some_and(|part| {
                    let key = |relationship: &Relationship| {
                        (
                            relationship.id.clone(),
                            relationship.r#type.clone(),
                            relationship.target.clone(),
                            relationship.target_mode.clone(),
                        )
                    };
                    part.relationships.iter().map(key).eq(relationships.iter().map(key))
                });
            let unchanged_content = self
                .read_xml_part_content(path)
                .and_then(|old| DocxXmlDocument::parse(&old).ok())
                .and_then(|old| old.to_string().ok())
                .is_some_and(|old| old == content);
            if unchanged_content && unchanged_relationships {
                return Ok(false);
            }
        }

        // Pictures added while the story was edited: their media go into
        // the package beside it.
        let base = existing
            .clone()
            .unwrap_or_else(|| format!("word/{}1.xml", role.part_stem()));
        for relationship in &relationships {
            if relationship.r#type != RelationshipTypes::Image.as_str()
                || relationship.target_mode.as_deref() == Some("External")
            {
                continue;
            }
            let path = Self::resolve_target_path(&base, &relationship.target);
            let known = self.binary_parts.contains_key(&path)
                || self
                    .package
                    .as_ref()
                    .is_some_and(|package| package.files.contains_key(&path));
            if known {
                continue;
            }
            let Some(bytes) = files.get(&path) else {
                continue;
            };
            let extension = path.rsplit('.').next().unwrap_or_default().to_string();
            if let Some(mime) = image_content_type(&extension) {
                self.ensure_content_types_has_image(&path, &extension, mime)?;
            }
            self.removed_parts.remove(&path);
            self.binary_parts.insert(path, bytes.clone());
        }

        match existing {
            Some(path) => {
                self.upsert_xml_part_content(&path, content);
                self.set_story_relationships(&path, relationships)?;
                self.refresh_story(role, variant, &path)?;
            }
            None => {
                self.create_story_part(role, variant, content, relationships)?;
            }
        }
        Ok(true)
    }
}

/// Gives the pictures of a re-read story the bytes the package does not have
/// yet, by relationship id.
fn fill_story_images(
    elements: &mut [crate::document::BodyElement],
    media: &HashMap<String, String>,
) {
    use crate::document::BodyElement;
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                for run in &mut paragraph.runs {
                    if let Some(image) = run.image.as_mut() {
                        if image.data_uri.is_none() {
                            if let Some(uri) = image
                                .relationship_id
                                .as_deref()
                                .and_then(|id| media.get(id))
                            {
                                image.data_uri = Some(uri.clone());
                            }
                        }
                    }
                }
            }
            BodyElement::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        fill_story_images(&mut cell.blocks, media);
                    }
                }
            }
            _ => {}
        }
    }
}
