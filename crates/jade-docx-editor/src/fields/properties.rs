//! A document's properties as Word's Properties dialog shows them: the
//! standard ones in `docProps/core.xml` and `docProps/app.xml`, the custom
//! ones in `docProps/custom.xml`, and the document variables in
//! `word/settings.xml` (`w:docVars`). Fields read them (AUTHOR, TITLE,
//! DOCPROPERTY, DOCVARIABLE…) and a few write them (AUTHOR "Name").

use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode, XmlNodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The value of a custom property, in the four types Word offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CustomValue {
    Text(String),
    Number(f64),
    /// W3CDTF, UTC (`2026-10-21T00:00:00Z`).
    Date(String),
    YesNo(bool),
}

impl CustomValue {
    /// How a DOCPROPERTY field shows it.
    pub fn display(&self) -> String {
        match self {
            CustomValue::Text(text) => text.clone(),
            CustomValue::Number(number) => {
                jade_docx_reader::field_format::format_plain_number(*number)
            }
            CustomValue::Date(date) => date.clone(),
            CustomValue::YesNo(value) => if *value { "Y" } else { "N" }.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomProperty {
    pub name: String,
    pub value: CustomValue,
}

/// The properties of a document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentProperties {
    pub title: String,
    pub subject: String,
    /// `dc:creator`, Word's 作成者.
    pub author: String,
    pub keywords: String,
    /// `dc:description`, Word's コメント.
    pub comments: String,
    pub category: String,
    pub content_status: String,
    pub last_saved_by: String,
    pub revision: String,
    /// W3CDTF timestamps, UTC.
    pub created: String,
    pub modified: String,
    pub last_printed: String,
    pub company: String,
    pub manager: String,
    pub hyperlink_base: String,
    pub template: String,
    /// Read-only statistics `app.xml` stores.
    pub total_time: String,
    pub pages: String,
    pub words: String,
    pub characters: String,
    pub characters_with_spaces: String,
    pub lines: String,
    pub paragraphs: String,
    pub application: String,
    pub custom: Vec<CustomProperty>,
}

const CORE_PATH: &str = "docProps/core.xml";
const APP_PATH: &str = "docProps/app.xml";
const CUSTOM_PATH: &str = "docProps/custom.xml";
const SETTINGS_PATH: &str = "word/settings.xml";
const CUSTOM_FMTID: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";
const CUSTOM_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
const CORE_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const APP_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";

/// The (core, extended) element names of each standard property.
const CORE_FIELDS: [(&str, &str); 12] = [
    ("dc:title", "title"),
    ("dc:subject", "subject"),
    ("dc:creator", "author"),
    ("cp:keywords", "keywords"),
    ("dc:description", "comments"),
    ("cp:category", "category"),
    ("cp:contentStatus", "content_status"),
    ("cp:lastModifiedBy", "last_saved_by"),
    ("cp:revision", "revision"),
    ("dcterms:created", "created"),
    ("dcterms:modified", "modified"),
    ("cp:lastPrinted", "last_printed"),
];

const APP_FIELDS: [(&str, &str); 12] = [
    ("Company", "company"),
    ("Manager", "manager"),
    ("HyperlinkBase", "hyperlink_base"),
    ("Template", "template"),
    ("TotalTime", "total_time"),
    ("Pages", "pages"),
    ("Words", "words"),
    ("Characters", "characters"),
    ("CharactersWithSpaces", "characters_with_spaces"),
    ("Lines", "lines"),
    ("Paragraphs", "paragraphs"),
    ("Application", "application"),
];

impl DocumentProperties {
    fn slot(&mut self, key: &str) -> Option<&mut String> {
        Some(match key {
            "title" => &mut self.title,
            "subject" => &mut self.subject,
            "author" => &mut self.author,
            "keywords" => &mut self.keywords,
            "comments" => &mut self.comments,
            "category" => &mut self.category,
            "content_status" => &mut self.content_status,
            "last_saved_by" => &mut self.last_saved_by,
            "revision" => &mut self.revision,
            "created" => &mut self.created,
            "modified" => &mut self.modified,
            "last_printed" => &mut self.last_printed,
            "company" => &mut self.company,
            "manager" => &mut self.manager,
            "hyperlink_base" => &mut self.hyperlink_base,
            "template" => &mut self.template,
            "total_time" => &mut self.total_time,
            "pages" => &mut self.pages,
            "words" => &mut self.words,
            "characters" => &mut self.characters,
            "characters_with_spaces" => &mut self.characters_with_spaces,
            "lines" => &mut self.lines,
            "paragraphs" => &mut self.paragraphs,
            "application" => &mut self.application,
            _ => return None,
        })
    }

    fn get(&self, key: &str) -> Option<&str> {
        Some(match key {
            "title" => &self.title,
            "subject" => &self.subject,
            "author" => &self.author,
            "keywords" => &self.keywords,
            "comments" => &self.comments,
            "category" => &self.category,
            "content_status" => &self.content_status,
            "last_saved_by" => &self.last_saved_by,
            "revision" => &self.revision,
            "created" => &self.created,
            "modified" => &self.modified,
            "last_printed" => &self.last_printed,
            "company" => &self.company,
            "manager" => &self.manager,
            "hyperlink_base" => &self.hyperlink_base,
            "template" => &self.template,
            "total_time" => &self.total_time,
            "pages" => &self.pages,
            "words" => &self.words,
            "characters" => &self.characters,
            "characters_with_spaces" => &self.characters_with_spaces,
            "lines" => &self.lines,
            "paragraphs" => &self.paragraphs,
            "application" => &self.application,
            _ => return None,
        })
    }

    /// The property a DOCPROPERTY or INFO name refers to, as its stored
    /// text, matched case-insensitively against Word's property names and
    /// then the custom properties.
    pub fn lookup(&self, name: &str) -> Option<PropertyValue> {
        let key = match name.to_ascii_lowercase().replace(' ', "").as_str() {
            "title" => "title",
            "subject" => "subject",
            "author" => "author",
            "keywords" => "keywords",
            "comments" => "comments",
            "category" => "category",
            "contentstatus" => "content_status",
            "lastsavedby" => "last_saved_by",
            "revisionnumber" | "revnum" | "revision" => "revision",
            "createtime" | "createdate" | "created" => "created",
            "lastsavetime" | "savedate" | "modified" => "modified",
            "lastprinted" | "printdate" => "last_printed",
            "company" => "company",
            "manager" => "manager",
            "hyperlinkbase" => "hyperlink_base",
            "template" => "template",
            "totaleditingtime" | "edittime" | "totaltime" => "total_time",
            "pages" | "numpages" => "pages",
            "words" | "numwords" => "words",
            "characters" | "numchars" => "characters",
            "characterswithspaces" => "characters_with_spaces",
            "lines" => "lines",
            "paragraphs" => "paragraphs",
            "application" | "appname" | "nameofapplication" => "application",
            _ => {
                return self
                    .custom
                    .iter()
                    .find(|property| property.name.eq_ignore_ascii_case(name.trim()))
                    .map(|property| match &property.value {
                        CustomValue::Date(date) => PropertyValue::Date(date.clone()),
                        CustomValue::Number(number) => PropertyValue::Number(*number),
                        other => PropertyValue::Text(other.display()),
                    });
            }
        };
        let value = self.get(key)?.to_string();
        Some(match key {
            "created" | "modified" | "last_printed" => PropertyValue::Date(value),
            "total_time"
            | "pages"
            | "words"
            | "characters"
            | "characters_with_spaces"
            | "lines"
            | "paragraphs" => value
                .trim()
                .parse::<f64>()
                .map(PropertyValue::Number)
                .unwrap_or(PropertyValue::Text(value)),
            _ => PropertyValue::Text(value),
        })
    }

    /// Sets a standard property by a field's name for it (AUTHOR "Name"
    /// sets the author). Returns whether `name` is one.
    pub fn set_by_field_name(&mut self, name: &str, value: &str) -> bool {
        let key = match name.to_ascii_uppercase().as_str() {
            "AUTHOR" => "author",
            "TITLE" => "title",
            "SUBJECT" => "subject",
            "KEYWORDS" => "keywords",
            "COMMENTS" => "comments",
            "LASTSAVEDBY" => "last_saved_by",
            "CATEGORY" => "category",
            "MANAGER" => "manager",
            "COMPANY" => "company",
            _ => return false,
        };
        if let Some(slot) = self.slot(key) {
            *slot = value.to_string();
        }
        true
    }

    /// Reads the properties from a package's files.
    pub fn read(files: &HashMap<String, Vec<u8>>) -> Self {
        let mut properties = DocumentProperties::default();
        let part = |path: &str| -> Option<DocxXmlDocument> {
            let bytes = files.get(&resolve_part(files, path))?;
            DocxXmlDocument::parse(&String::from_utf8_lossy(bytes)).ok()
        };
        if let Some(core) = part(CORE_PATH) {
            for child in elements(core.root()) {
                if let Some((_, key)) = CORE_FIELDS
                    .iter()
                    .find(|(name, _)| local(name) == local(&child.name))
                {
                    if let Some(slot) = properties.slot(key) {
                        *slot = text_of(child).trim().to_string();
                    }
                }
            }
        }
        if let Some(app) = part(APP_PATH) {
            for child in elements(app.root()) {
                if let Some((_, key)) = APP_FIELDS
                    .iter()
                    .find(|(name, _)| *name == local(&child.name))
                {
                    if let Some(slot) = properties.slot(key) {
                        *slot = text_of(child).trim().to_string();
                    }
                }
            }
        }
        if let Some(custom) = part(CUSTOM_PATH) {
            for property in elements(custom.root()) {
                let Some(name) = attribute(property, "name") else {
                    continue;
                };
                let Some(value) = elements(property).next() else {
                    continue;
                };
                let text = text_of(value);
                let value = match local(&value.name) {
                    "bool" => CustomValue::YesNo(matches!(text.trim(), "true" | "1")),
                    "filetime" | "date" => CustomValue::Date(text.trim().to_string()),
                    "i1" | "i2" | "i4" | "i8" | "int" | "ui1" | "ui2" | "ui4" | "ui8" | "uint"
                    | "r4" | "r8" | "decimal" => text
                        .trim()
                        .parse()
                        .map(CustomValue::Number)
                        .unwrap_or(CustomValue::Text(text)),
                    _ => CustomValue::Text(text),
                };
                properties.custom.push(CustomProperty {
                    name: name.to_string(),
                    value,
                });
            }
        }
        properties
    }

    /// Writes the properties back into `files`, creating the parts — and
    /// their relationships and content types — a document lacks.
    pub fn write(&self, files: &mut HashMap<String, Vec<u8>>) -> Result<(), String> {
        let core_path = resolve_part(files, CORE_PATH);
        let mut core = files
            .get(&core_path)
            .and_then(|bytes| DocxXmlDocument::parse(&String::from_utf8_lossy(bytes)).ok())
            .unwrap_or_else(|| {
                ensure_part(
                    files,
                    CORE_PATH,
                    CORE_RELATIONSHIP,
                    "application/vnd.openxmlformats-package.core-properties+xml",
                );
                DocxXmlDocument::parse(concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
                    "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" ",
                    "xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" ",
                    "xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>"
                ))
                .expect("core properties template parses")
            });
        let root_id = core.root().id;
        for (name, key) in CORE_FIELDS {
            let value = self.get(key).unwrap_or_default();
            let mut attributes = Vec::new();
            if name.starts_with("dcterms:") {
                attributes.push(XmlAttribute {
                    name: "xsi:type".to_string(),
                    value: "dcterms:W3CDTF".to_string(),
                });
            }
            set_child_text(&mut core, root_id, name, value, attributes);
        }
        ensure_namespace(
            &mut core,
            root_id,
            "xmlns:dcterms",
            "http://purl.org/dc/terms/",
        );
        ensure_namespace(
            &mut core,
            root_id,
            "xmlns:dc",
            "http://purl.org/dc/elements/1.1/",
        );
        ensure_namespace(
            &mut core,
            root_id,
            "xmlns:xsi",
            "http://www.w3.org/2001/XMLSchema-instance",
        );
        core.rebuild_index();
        files.insert(core_path, serialize(&core)?);

        let app_path = resolve_part(files, APP_PATH);
        let mut app = files
            .get(&app_path)
            .and_then(|bytes| DocxXmlDocument::parse(&String::from_utf8_lossy(bytes)).ok())
            .unwrap_or_else(|| {
                ensure_part(
                    files,
                    APP_PATH,
                    APP_RELATIONSHIP,
                    "application/vnd.openxmlformats-officedocument.extended-properties+xml",
                );
                DocxXmlDocument::parse(concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
                    "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" ",
                    "xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"/>"
                ))
                .expect("extended properties template parses")
            });
        let app_root = app.root().id;
        for (name, key) in [
            ("Company", "company"),
            ("Manager", "manager"),
            ("HyperlinkBase", "hyperlink_base"),
        ] {
            let value = self.get(key).unwrap_or_default();
            set_child_text(&mut app, app_root, name, value, Vec::new());
        }
        app.rebuild_index();
        files.insert(app_path, serialize(&app)?);

        let custom_path = resolve_part(files, CUSTOM_PATH);
        if self.custom.is_empty() && !files.contains_key(&custom_path) {
            return Ok(());
        }
        if !files.contains_key(&custom_path) {
            ensure_part(
                files,
                CUSTOM_PATH,
                CUSTOM_RELATIONSHIP,
                "application/vnd.openxmlformats-officedocument.custom-properties+xml",
            );
        }
        let mut xml = String::from(concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
            "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" ",
            "xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\">"
        ));
        for (index, property) in self.custom.iter().enumerate() {
            let (kind, value) = match &property.value {
                CustomValue::Text(text) => ("vt:lpwstr", text.clone()),
                CustomValue::Number(number) => {
                    if number.fract() == 0.0 && number.abs() < i32::MAX as f64 {
                        ("vt:i4", format!("{}", *number as i64))
                    } else {
                        ("vt:r8", number.to_string())
                    }
                }
                CustomValue::Date(date) => ("vt:filetime", date.clone()),
                CustomValue::YesNo(value) => ("vt:bool", value.to_string()),
            };
            xml.push_str(&format!(
                "<property fmtid=\"{CUSTOM_FMTID}\" pid=\"{}\" name=\"{}\"><{kind}>{}</{kind}></property>",
                index + 2,
                escape(&property.name),
                escape(&value),
            ));
        }
        xml.push_str("</Properties>");
        files.insert(custom_path, xml.into_bytes());
        Ok(())
    }
}

/// A property's value as a field reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    /// W3CDTF.
    Date(String),
}

/// The document variables (`w:docVars`) of `word/settings.xml`, in order.
pub fn read_variables(files: &HashMap<String, Vec<u8>>) -> Vec<(String, String)> {
    let Some(settings) = files
        .get(SETTINGS_PATH)
        .and_then(|bytes| DocxXmlDocument::parse(&String::from_utf8_lossy(bytes)).ok())
    else {
        return Vec::new();
    };
    elements(settings.root())
        .filter(|child| child.name == "w:docVars")
        .flat_map(elements)
        .filter(|child| child.name == "w:docVar")
        .filter_map(|variable| {
            Some((
                attribute(variable, "w:name")?.to_string(),
                attribute(variable, "w:val").unwrap_or_default().to_string(),
            ))
        })
        .collect()
}

/// Replaces the document variables of `word/settings.xml`.
pub fn write_variables(
    files: &mut HashMap<String, Vec<u8>>,
    variables: &[(String, String)],
) -> Result<(), String> {
    let Some(bytes) = files.get(SETTINGS_PATH) else {
        return if variables.is_empty() {
            Ok(())
        } else {
            Err("the document has no settings part".to_string())
        };
    };
    let mut settings = DocxXmlDocument::parse(&String::from_utf8_lossy(bytes))
        .map_err(|error| format!("settings.xml: {error:?}"))?;
    let root_id = settings.root().id;
    let root = settings
        .element_mut(root_id)
        .ok_or("settings.xml has no root")?;
    root.children
        .retain(|child| !matches!(child, XmlNode::Element(element) if element.name == "w:docVars"));
    if !variables.is_empty() {
        let children = variables
            .iter()
            .map(|(name, value)| {
                XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "w:docVar".to_string(),
                    attributes: vec![
                        XmlAttribute {
                            name: "w:name".to_string(),
                            value: name.clone(),
                        },
                        XmlAttribute {
                            name: "w:val".to_string(),
                            value: value.clone(),
                        },
                    ],
                    children: Vec::new(),
                })
            })
            .collect();
        root.children.push(XmlNode::Element(XmlElement {
            id: XmlNodeId::new(0),
            name: "w:docVars".to_string(),
            attributes: Vec::new(),
            children,
        }));
    }
    settings.rebuild_index();
    files.insert(SETTINGS_PATH.to_string(), serialize(&settings)?);
    Ok(())
}

fn serialize(doc: &DocxXmlDocument) -> Result<Vec<u8>, String> {
    doc.to_string()
        .map(String::into_bytes)
        .map_err(|error| format!("{error:?}"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|child| match child {
        XmlNode::Element(element) => Some(element),
        _ => None,
    })
}

fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn text_of(element: &XmlElement) -> String {
    element
        .children
        .iter()
        .map(|child| match child {
            XmlNode::Text(text) | XmlNode::CData(text) => text.clone(),
            XmlNode::Element(child) => text_of(child),
            XmlNode::Comment(_) => String::new(),
        })
        .collect()
}

fn ensure_namespace(doc: &mut DocxXmlDocument, root: XmlNodeId, name: &str, value: &str) {
    if let Some(root) = doc.element_mut(root) {
        if !root
            .attributes
            .iter()
            .any(|attribute| attribute.name == name)
        {
            root.attributes.push(XmlAttribute {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
}

/// Sets the text of `root`'s child `name`, adding it when the value is not
/// empty and removing it when it is.
fn set_child_text(
    doc: &mut DocxXmlDocument,
    root: XmlNodeId,
    name: &str,
    value: &str,
    attributes: Vec<XmlAttribute>,
) {
    let Some(root) = doc.element_mut(root) else {
        return;
    };
    let position = root.children.iter().position(
        |child| matches!(child, XmlNode::Element(element) if local(&element.name) == local(name)),
    );
    match (position, value.is_empty()) {
        (Some(position), true) => {
            root.children.remove(position);
        }
        (Some(position), false) => {
            if let XmlNode::Element(element) = &mut root.children[position] {
                element.children = vec![XmlNode::Text(value.to_string())];
            }
        }
        (None, true) => {}
        (None, false) => root.children.push(XmlNode::Element(XmlElement {
            id: XmlNodeId::new(0),
            name: name.to_string(),
            attributes,
            children: vec![XmlNode::Text(value.to_string())],
        })),
    }
}

/// Where the package keeps the part conventionally at `default`: the target
/// of its relationship in `_rels/.rels`, or `default`.
fn resolve_part(files: &HashMap<String, Vec<u8>>, default: &str) -> String {
    let relationship = match default {
        CORE_PATH => CORE_RELATIONSHIP,
        APP_PATH => APP_RELATIONSHIP,
        CUSTOM_PATH => CUSTOM_RELATIONSHIP,
        _ => return default.to_string(),
    };
    files
        .get("_rels/.rels")
        .and_then(|bytes| DocxXmlDocument::parse(&String::from_utf8_lossy(bytes)).ok())
        .and_then(|rels| {
            elements(rels.root())
                .find(|child| attribute(child, "Type") == Some(relationship))
                .and_then(|child| attribute(child, "Target"))
                .map(|target| target.trim_start_matches('/').to_string())
        })
        .unwrap_or_else(|| default.to_string())
}

/// Adds the package relationship and content type override a new part
/// needs.
fn ensure_part(
    files: &mut HashMap<String, Vec<u8>>,
    path: &str,
    relationship: &str,
    content_type: &str,
) {
    let rels = files
        .get("_rels/.rels")
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string()
        });
    if !rels.contains(relationship) {
        let mut number = 1;
        while rels.contains(&format!("\"rId{number}\"")) {
            number += 1;
        }
        let entry =
            format!("<Relationship Id=\"rId{number}\" Type=\"{relationship}\" Target=\"{path}\"/>");
        let updated = match rels.rfind("</Relationships>") {
            Some(at) => format!("{}{entry}{}", &rels[..at], &rels[at..]),
            None => rels.replace("/>", &format!(">{entry}</Relationships>")),
        };
        files.insert("_rels/.rels".to_string(), updated.into_bytes());
    }
    if let Some(types) = files.get("[Content_Types].xml") {
        let types = String::from_utf8_lossy(types).into_owned();
        let part_name = format!("/{path}");
        if !types.contains(&format!("\"{part_name}\"")) {
            let entry =
                format!("<Override PartName=\"{part_name}\" ContentType=\"{content_type}\"/>");
            if let Some(at) = types.rfind("</Types>") {
                files.insert(
                    "[Content_Types].xml".to_string(),
                    format!("{}{entry}{}", &types[..at], &types[at..]).into_bytes(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> HashMap<String, Vec<u8>> {
        let mut files = HashMap::new();
        files.insert(
            "_rels/.rels".to_string(),
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#.to_vec(),
        );
        files.insert(
            "[Content_Types].xml".to_string(),
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"></Types>"#.to_vec(),
        );
        files.insert(
            "docProps/core.xml".to_string(),
            r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>契約書</dc:title><dc:creator>山田</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2026-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#.as_bytes().to_vec(),
        );
        files.insert(
            "word/settings.xml".to_string(),
            br#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/><w:docVars><w:docVar w:name="Client" w:val="ACME"/></w:docVars></w:settings>"#.to_vec(),
        );
        files
    }

    #[test]
    fn reads_and_writes_standard_and_custom_properties() {
        let mut files = files();
        let mut properties = DocumentProperties::read(&files);
        assert_eq!(properties.title, "契約書");
        assert_eq!(properties.author, "山田");
        assert_eq!(properties.created, "2026-01-02T03:04:05Z");
        assert_eq!(
            properties.lookup("Author"),
            Some(PropertyValue::Text("山田".into()))
        );
        assert_eq!(
            properties.lookup("CreateTime"),
            Some(PropertyValue::Date("2026-01-02T03:04:05Z".into()))
        );

        properties.subject = "業務委託".to_string();
        properties.company = "Jade".to_string();
        properties.custom = vec![
            CustomProperty {
                name: "Project Code".into(),
                value: CustomValue::Text("X-1".into()),
            },
            CustomProperty {
                name: "Budget".into(),
                value: CustomValue::Number(1200.0),
            },
            CustomProperty {
                name: "Approved".into(),
                value: CustomValue::YesNo(true),
            },
        ];
        properties.write(&mut files).unwrap();
        let reread = DocumentProperties::read(&files);
        assert_eq!(reread.subject, "業務委託");
        assert_eq!(reread.company, "Jade");
        assert_eq!(reread.custom, properties.custom);
        assert_eq!(
            reread.lookup("project code"),
            Some(PropertyValue::Text("X-1".into()))
        );
        assert_eq!(reread.lookup("Budget"), Some(PropertyValue::Number(1200.0)));
        let rels = String::from_utf8(files["_rels/.rels"].clone()).unwrap();
        assert!(rels.contains("custom-properties"));
        assert!(rels.contains("extended-properties"));
        let types = String::from_utf8(files["[Content_Types].xml"].clone()).unwrap();
        assert!(types.contains("/docProps/custom.xml"));
    }

    #[test]
    fn reads_and_replaces_document_variables() {
        let mut files = files();
        assert_eq!(
            read_variables(&files),
            vec![("Client".to_string(), "ACME".to_string())]
        );
        write_variables(
            &mut files,
            &[
                ("Client".into(), "Jade".into()),
                ("Case".into(), "7".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            read_variables(&files),
            vec![
                ("Client".to_string(), "Jade".to_string()),
                ("Case".to_string(), "7".to_string())
            ]
        );
    }
}
