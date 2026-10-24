use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub r#type: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RelationshipTypes {
    OfficeDocument,
    ExtendedProperties,
    CoreProperties,
    CustomProperties,
    FontTable,
    Numbering,
    Styles,
    StylesWithEffects,
    Theme,
    Footnotes,
    Endnotes,
    Footer,
    Header,
    Settings,
    Comments,
    CommentsExtended,
    CommentsIds,
    CommentsExtensible,
    Image,
    Chart,
    Hyperlink,
    CustomXml,
    AltChunk,
    OleObject,
}

impl RelationshipTypes {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OfficeDocument => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
            Self::ExtendedProperties => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
            Self::CoreProperties => "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
            Self::CustomProperties => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties",
            Self::FontTable => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable",
            Self::Numbering => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            Self::Styles => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
            Self::StylesWithEffects => {
                "http://schemas.microsoft.com/office/2007/relationships/stylesWithEffects"
            }
            Self::Theme => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
            Self::Footnotes => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
            Self::Endnotes => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes",
            Self::Footer => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
            Self::Header => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
            Self::Settings => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
            Self::Comments => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
            Self::CommentsExtended => "http://schemas.microsoft.com/office/2011/relationships/commentsExtended",
            Self::CommentsIds => "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds",
            Self::CommentsExtensible => {
                "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible"
            }
            Self::Image => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            Self::Chart => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
            Self::Hyperlink => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
            }
            Self::CustomXml => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml"
            }
            Self::AltChunk => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk"
            }
            Self::OleObject => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject"
            }
        }
    }
}

pub trait Part: Send + Sync {
    fn path(&self) -> &str;
    fn relationships(&self) -> &Vec<Relationship>;
    fn load(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenXmlPackage {
    pub files: HashMap<String, Vec<u8>>,
    pub relationships: Vec<Relationship>,
    /// Compressed byte length of the source DOCX. This is package-level state,
    /// not the sum of inflated ZIP entries, and is used by Word's FILESIZE
    /// field. Synthetic packages may leave it unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_size_bytes: Option<usize>,
}

impl OpenXmlPackage {
    pub async fn load(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        Self::load_with_limits(data, crate::package_limits::PackageLoadLimits::default()).await
    }

    /// Load with explicit resource budgets. The normal loader uses browser-safe defaults.
    pub async fn load_with_limits(
        data: &[u8],
        limits: crate::package_limits::PackageLoadLimits,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut archive = crate::package_limits::open_checked_archive(data, limits)?;
        let mut files = HashMap::new();
        let mut total = 0u64;

        for i in 0..archive.len() {
            let file = archive.by_index(i)?;
            let name = file.name()?.into_owned();
            let contents = crate::package_limits::read_part(file, limits, &mut total)?;
            files.insert(name, contents);
        }

        // Load main relationships
        let package = Self {
            files,
            relationships: Vec::new(),
            original_size_bytes: Some(data.len()),
        };
        let relationships = if let Some(rels_data) = package.files.get("_rels/.rels") {
            package.parse_relationships(rels_data)?
        } else {
            Vec::new()
        };

        Ok(Self {
            files: package.files,
            relationships,
            original_size_bytes: package.original_size_bytes,
        })
    }

    pub fn get(&self, path: &str) -> Option<&Vec<u8>> {
        self.files.get(path)
    }

    /// Reads core, extended, and custom OPC metadata for `DOCPROPERTY` field
    /// resolution.
    /// Relationship targets are used instead of conventional filenames because
    /// valid packages may relocate any metadata part.
    pub fn document_properties(&self) -> HashMap<String, String> {
        let mut properties = HashMap::new();
        for (relationship_type, parser) in [
            (
                RelationshipTypes::CoreProperties.as_str(),
                parse_core_document_properties as fn(&str) -> HashMap<String, String>,
            ),
            (
                RelationshipTypes::ExtendedProperties.as_str(),
                parse_extended_document_properties as fn(&str) -> HashMap<String, String>,
            ),
            (
                RelationshipTypes::CustomProperties.as_str(),
                parse_custom_document_properties as fn(&str) -> HashMap<String, String>,
            ),
        ] {
            for relationship in self
                .relationships
                .iter()
                .filter(|rel| rel.r#type == relationship_type)
            {
                let path = relationship.target.trim_start_matches('/');
                let Some(bytes) = self.get(path) else {
                    continue;
                };
                for (name, value) in parser(&String::from_utf8_lossy(bytes)) {
                    // Built-in core/extended names are visited first and remain
                    // authoritative if a custom property happens to reuse the
                    // same display name.
                    properties.entry(name).or_insert(value);
                }
            }
        }
        properties
    }

    pub fn parse_relationships(
        &self,
        data: &[u8],
    ) -> Result<Vec<Relationship>, Box<dyn std::error::Error>> {
        let content = String::from_utf8_lossy(data);
        let mut relationships = Vec::new();

        // Use quick-xml for proper XML parsing
        let mut reader = quick_xml::Reader::from_str(&content);
        reader.config_mut().trim_text(true);

        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(ref e))
                | Ok(quick_xml::events::Event::Empty(ref e)) => {
                    if e.name().as_ref() == b"Relationship" {
                        let mut id = String::new();
                        let mut r#type = String::new();
                        let mut target = String::new();
                        let mut target_mode = None;

                        for attr in e.attributes() {
                            let attr = attr?;
                            let value = attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                .into_owned();
                            match attr.key.as_ref() {
                                b"Id" => id = value,
                                b"Type" => r#type = value,
                                b"Target" => target = value,
                                b"TargetMode" => {
                                    let mode = value;
                                    if !mode.is_empty() {
                                        target_mode = Some(mode);
                                    }
                                }
                                _ => {}
                            }
                        }

                        if !id.is_empty() && !r#type.is_empty() && !target.is_empty() {
                            relationships.push(Relationship {
                                id,
                                r#type,
                                target,
                                target_mode,
                            });
                        }
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Err(e) => return Err(format!("XML parsing error: {}", e).into()),
                _ => {}
            }
        }

        Ok(relationships)
    }
}

fn normalized_property_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn decoded_text(text: &quick_xml::events::BytesText<'_>) -> Option<String> {
    let decoded = text.decode().ok()?;
    Some(
        unescape(decoded.as_ref())
            .map(|text| text.into_owned())
            .unwrap_or_else(|_| decoded.into_owned()),
    )
}

fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or_default()
}

fn parse_core_document_properties(xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut properties = HashMap::new();
    let mut current: Option<(String, String)> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let canonical = match local_name(element.name().as_ref()) {
                    b"title" => Some("Title"),
                    b"subject" => Some("Subject"),
                    b"creator" => Some("Author"),
                    b"keywords" => Some("Keywords"),
                    b"description" => Some("Comments"),
                    b"lastModifiedBy" => Some("LastSavedBy"),
                    b"created" => Some("Created"),
                    b"modified" => Some("Modified"),
                    b"lastPrinted" => Some("LastPrinted"),
                    b"revision" => Some("Revision"),
                    b"category" => Some("Category"),
                    b"contentStatus" => Some("ContentStatus"),
                    _ => None,
                };
                if let Some(canonical) = canonical {
                    current = Some((canonical.to_string(), String::new()));
                }
            }
            Ok(Event::Text(text)) => {
                if let (Some((_, value)), Some(text)) = (current.as_mut(), decoded_text(&text)) {
                    value.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if let (Some((_, value)), Some(text)) =
                    (current.as_mut(), decoded_reference(&reference))
                {
                    value.push_str(&text);
                }
            }
            Ok(Event::End(element)) => {
                if current.is_some() && local_name(element.name().as_ref()) != b"coreProperties" {
                    let (name, value) = current.take().expect("checked above");
                    let value = value.trim().to_string();
                    if !value.is_empty() {
                        properties.insert(normalized_property_name(&name), value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    properties
}

fn parse_custom_document_properties(xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut properties = HashMap::new();
    let mut current: Option<(String, String)> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if local_name(element.name().as_ref()) == b"property" => {
                let name = element.attributes().flatten().find_map(|attribute| {
                    (attribute.key.as_ref() == b"name")
                        .then(|| {
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .ok()
                                .map(|v| v.into_owned())
                        })
                        .flatten()
                });
                if let Some(name) = name {
                    current = Some((name, String::new()));
                }
            }
            Ok(Event::Text(text)) => {
                if let (Some((_, value)), Some(text)) = (current.as_mut(), decoded_text(&text)) {
                    value.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if let (Some((_, value)), Some(text)) =
                    (current.as_mut(), decoded_reference(&reference))
                {
                    value.push_str(&text);
                }
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == b"property" => {
                if let Some((name, value)) = current.take() {
                    let value = value.trim().to_string();
                    if !value.is_empty() {
                        properties.insert(normalized_property_name(&name), value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    properties
}

fn parse_extended_document_properties(xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut properties = HashMap::new();
    let mut current: Option<(String, String, String)> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                // These are the scalar extended properties exposed by Word's
                // document-property field UI. Deliberately ignore vector-valued
                // children such as HeadingPairs/TitlesOfParts: collecting their
                // descendant text would synthesize a value Word never presents
                // as one DOCPROPERTY result.
                let tag = String::from_utf8_lossy(local_name(element.name().as_ref())).into_owned();
                let canonical = match tag.as_str() {
                    "Template"
                    | "Manager"
                    | "Company"
                    | "TotalTime"
                    | "Pages"
                    | "Words"
                    | "Characters"
                    | "Lines"
                    | "Paragraphs"
                    | "CharactersWithSpaces"
                    | "HyperlinkBase"
                    | "AppVersion" => Some(tag.clone()),
                    _ => None,
                };
                if let Some(canonical) = canonical {
                    current = Some((tag, canonical, String::new()));
                }
            }
            Ok(Event::Text(text)) => {
                if let (Some((_, _, value)), Some(text)) = (current.as_mut(), decoded_text(&text)) {
                    value.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if let (Some((_, _, value)), Some(text)) =
                    (current.as_mut(), decoded_reference(&reference))
                {
                    value.push_str(&text);
                }
            }
            Ok(Event::End(element)) => {
                let element_name = element.name();
                let ended = String::from_utf8_lossy(local_name(element_name.as_ref()));
                if current
                    .as_ref()
                    .is_some_and(|(tag, _, _)| tag == ended.as_ref())
                {
                    let (_, name, value) = current.take().expect("checked above");
                    let value = value.trim().to_string();
                    if !value.is_empty() {
                        properties.insert(normalized_property_name(&name), value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    properties
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_package(bytes: &[u8]) -> Result<OpenXmlPackage, Box<dyn std::error::Error>> {
        use std::future::Future;
        let mut future = std::pin::pin!(OpenXmlPackage::load(bytes));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => panic!("in-memory ZIP loading must complete immediately"),
        }
    }

    fn named_zip(name: &str) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"contents").unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn load_zip_decodes_utf8_and_legacy_cp437_names() {
        let utf8 = named_zip("資料.xml");
        assert_eq!(load_package(&utf8).unwrap().files["資料.xml"], b"contents");
        let mut legacy = named_zip("a.xml");
        let positions: Vec<_> = legacy
            .windows(5)
            .enumerate()
            .filter_map(|(i, bytes)| (bytes == b"a.xml").then_some(i))
            .collect();
        assert_eq!(positions.len(), 2);
        for index in positions {
            legacy[index] = 0x82;
        }
        assert_eq!(load_package(&legacy).unwrap().files["é.xml"], b"contents");
    }

    #[test]
    fn load_zip_rejects_corrupt_archive() {
        assert!(load_package(b"not a ZIP archive").is_err());
    }

    #[test]
    fn parse_relationships_reads_target_mode() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship
    Id="rId7"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
    Target="https://example.com/?a=1&amp;b=資料"
    TargetMode="External"/>
  <Relationship
    Id="rId8"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
    Target="bookmarks.xml#section1"/>
</Relationships>"#
            .as_bytes();

        let package = OpenXmlPackage {
            files: HashMap::new(),
            relationships: Vec::new(),
            original_size_bytes: None,
        };
        let relationships = package.parse_relationships(xml).expect("parse rels");
        assert_eq!(relationships.len(), 2);
        assert_eq!(relationships[0].target_mode.as_deref(), Some("External"));
        assert_eq!(relationships[0].target, "https://example.com/?a=1&b=資料");
        assert_eq!(relationships[1].target_mode, None);
    }

    #[test]
    fn document_properties_reads_core_aliases_and_custom_values_by_relationship() {
        let core = br#"<?xml version="1.0" encoding="UTF-8"?>
<cp:coreProperties
  xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>Quarterly &amp; Annual Report</dc:title>
  <dc:creator>Ada Lovelace</dc:creator>
  <cp:lastModifiedBy>Grace Hopper</cp:lastModifiedBy>
  <dcterms:created xmlns:dcterms="http://purl.org/dc/terms/">2024-01-02T03:04:05Z</dcterms:created>
  <dcterms:modified xmlns:dcterms="http://purl.org/dc/terms/">2025-02-03T04:05:06Z</dcterms:modified>
  <cp:lastPrinted>2023-12-01T09:08:07Z</cp:lastPrinted>
</cp:coreProperties>"#;
        let custom = br#"<?xml version="1.0" encoding="UTF-8"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"
  xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
  <property fmtid="x" pid="2" name="Document Title"><vt:lpwstr>Board Copy</vt:lpwstr></property>
  <property fmtid="x" pid="3" name="Review Count"><vt:i4>12</vt:i4></property>
</Properties>"#;
        let extended = br#"<?xml version="1.0" encoding="UTF-8"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
  <Template>Normal.dotm</Template>
  <Manager>Grace Hopper</Manager>
  <Company>Analytical Engines &amp; Co.</Company>
  <TotalTime>37</TotalTime>
</Properties>"#;
        let package = OpenXmlPackage {
            files: HashMap::from([
                ("metadata/core-data.xml".to_string(), core.to_vec()),
                ("metadata/custom-data.xml".to_string(), custom.to_vec()),
                ("metadata/app-data.xml".to_string(), extended.to_vec()),
            ]),
            relationships: vec![
                Relationship {
                    id: "rCore".to_string(),
                    r#type: RelationshipTypes::CoreProperties.as_str().to_string(),
                    target: "/metadata/core-data.xml".to_string(),
                    target_mode: None,
                },
                Relationship {
                    id: "rCustom".to_string(),
                    r#type: RelationshipTypes::CustomProperties.as_str().to_string(),
                    target: "metadata/custom-data.xml".to_string(),
                    target_mode: None,
                },
                Relationship {
                    id: "rExtended".to_string(),
                    r#type: RelationshipTypes::ExtendedProperties.as_str().to_string(),
                    target: "metadata/app-data.xml".to_string(),
                    target_mode: None,
                },
            ],
            original_size_bytes: None,
        };

        let properties = package.document_properties();
        assert_eq!(
            properties.get("title").map(String::as_str),
            Some("Quarterly & Annual Report")
        );
        assert_eq!(
            properties.get("author").map(String::as_str),
            Some("Ada Lovelace")
        );
        assert_eq!(properties.get("totaltime").map(String::as_str), Some("37"));
        assert_eq!(
            properties.get("lastsavedby").map(String::as_str),
            Some("Grace Hopper")
        );
        assert_eq!(
            properties.get("created").map(String::as_str),
            Some("2024-01-02T03:04:05Z")
        );
        assert_eq!(
            properties.get("modified").map(String::as_str),
            Some("2025-02-03T04:05:06Z")
        );
        assert_eq!(
            properties.get("lastprinted").map(String::as_str),
            Some("2023-12-01T09:08:07Z")
        );
        assert_eq!(
            properties.get("document title").map(String::as_str),
            Some("Board Copy")
        );
        assert_eq!(
            properties.get("review count").map(String::as_str),
            Some("12")
        );
        assert_eq!(
            properties.get("company").map(String::as_str),
            Some("Analytical Engines & Co.")
        );
        assert_eq!(
            properties.get("manager").map(String::as_str),
            Some("Grace Hopper")
        );
        assert_eq!(
            properties.get("template").map(String::as_str),
            Some("Normal.dotm")
        );
    }
}
