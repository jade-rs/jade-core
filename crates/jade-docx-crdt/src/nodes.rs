//! Opt-in identity bootstrap for the main DOCX part, before opening a CRDT session.
use jade_crdt::NodeSeed;
use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode};
use serde::Serialize;
use std::collections::HashSet;

const NAMESPACE: &str = "urn:jade:crdt:v1";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

fn set_attr(element: &mut XmlElement, name: &str, value: &str) {
    if let Some(existing) = element.attributes.iter_mut().find(|a| a.name == name) {
        existing.value = value.into();
    } else {
        element.attributes.push(XmlAttribute {
            name: name.into(),
            value: value.into(),
        });
    }
}

fn kind(element: &XmlElement) -> Option<&'static str> {
    match element.name.as_str() {
        "w:p" => Some("paragraph"),
        "w:r" => Some("run"),
        "w:t" => Some("text"),
        "w:delText" => Some("deletedText"),
        _ => None,
    }
}

// Validate before assigning anything. Never silently rebind existing anchors.
fn validate(element: &XmlElement, bound: bool, ids: &mut HashSet<String>) -> Result<(), String> {
    for a in &element.attributes {
        let expected = match a.name.as_str() {
            "xmlns:jcrdt" => Some(NAMESPACE),
            "xmlns:mc" => Some(MC),
            "xmlns:w" => Some(WORD),
            _ => None,
        };
        if expected.is_some_and(|expected| a.value != expected) {
            return Err(format!("CRDT namespace conflict: {}", a.name));
        }
        if a.name.starts_with("xmlns:") && a.value == NAMESPACE && a.name != "xmlns:jcrdt" {
            return Err("CRDT namespace alias is unsupported".into());
        }
        if a.name.starts_with("jcrdt:") && !matches!(a.name.as_str(), "jcrdt:id" | "jcrdt:schema") {
            return Err("unsupported CRDT attribute".into());
        }
    }
    let bound = bound || attr(element, "xmlns:jcrdt") == Some(NAMESPACE);
    if let Some(schema) = attr(element, "jcrdt:schema") {
        if !bound || schema != "1" {
            return Err("unsupported CRDT schema".into());
        }
    }
    if let Some(id) = attr(element, "jcrdt:id") {
        if !bound
            || kind(element).is_none()
            || id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid CRDT node ID".into());
        }
        if !ids.insert(id.into()) {
            return Err("duplicate CRDT node ID".into());
        }
    }
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            validate(child, bound, ids)?;
        }
    }
    Ok(())
}

fn fresh_id(ids: &mut HashSet<String>) -> Result<String, String> {
    loop {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if ids.insert(id.clone()) {
            return Ok(id);
        }
    }
}

fn identify(
    element: &mut XmlElement,
    parent: Option<&str>,
    ids: &mut HashSet<String>,
    nodes: &mut Vec<NodeIdentity>,
) -> Result<(), String> {
    let mut own_id = None;
    if let Some(kind) = kind(element) {
        let id = match attr(element, "jcrdt:id") {
            Some(id) => id.to_owned(),
            None => {
                let id = fresh_id(ids)?;
                set_attr(element, "jcrdt:id", &id);
                id
            }
        };
        let text = if matches!(kind, "text" | "deletedText") {
            let mut text = String::new();
            for child in &element.children {
                match child {
                    XmlNode::Text(value) | XmlNode::CData(value) => text.push_str(value),
                    XmlNode::Element(_) => return Err("nested element in CRDT text".into()),
                    _ => (),
                }
            }
            Some(text)
        } else {
            None
        };
        nodes.push(NodeIdentity {
            id: id.clone(),
            kind: kind.into(),
            parent: parent.map(str::to_owned),
            text,
        });
        own_id = Some(id);
    }
    let parent = own_id.as_deref().or(parent);
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            identify(child, parent, ids, nodes)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIdentity {
    pub id: String,
    pub kind: String,
    pub parent: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeManifest {
    pub schema: u32,
    pub part: String,
    pub nodes: Vec<NodeIdentity>,
}

impl NodeManifest {
    /// Each OOXML text element is a distinct CRDT text. Paragraph/run IDs
    /// describe structure; they must not seed duplicate copies of their text.
    pub fn seeds(&self) -> Vec<NodeSeed> {
        self.nodes
            .iter()
            .filter_map(|node| {
                node.text.as_ref().map(|text| NodeSeed {
                    id: node.id.clone(),
                    text: text.clone(),
                })
            })
            .collect()
    }
}

/// Assign persistent, validated identities atomically to the main DOCX part.
pub fn prepare_nodes(raw: &mut DocxXmlDocument) -> Result<NodeManifest, String> {
    if raw.root().name != "w:document" || attr(raw.root(), "xmlns:w") != Some(WORD) {
        return Err("CRDT requires the canonical WordprocessingML main part".into());
    }
    let mut ids = HashSet::new();
    validate(raw.root(), false, &mut ids)?;
    // Bootstrap is opt-in, not a per-keystroke clone. Keep failures atomic.
    let mut candidate = raw.clone();
    let root_id = candidate.root().id;
    let root = candidate
        .element_mut(root_id)
        .ok_or("DOCX root is unavailable")?;
    set_attr(root, "xmlns:jcrdt", NAMESPACE);
    set_attr(root, "xmlns:mc", MC);
    set_attr(root, "jcrdt:schema", "1");
    let mut ignorable: Vec<_> = attr(root, "mc:Ignorable")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if !ignorable.iter().any(|prefix| prefix == "jcrdt") {
        ignorable.push("jcrdt".into());
    }
    set_attr(root, "mc:Ignorable", &ignorable.join(" "));
    let mut nodes = Vec::new();
    identify(root, None, &mut ids, &mut nodes)?;
    *raw = candidate;
    Ok(NodeManifest {
        schema: 1,
        part: "word/document.xml".into(),
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    fn xml(body: &str, attributes: &str) -> DocxXmlDocument {
        DocxXmlDocument::parse(&format!(
            "<w:document xmlns:w=\"{W}\" {attributes}><w:body>{body}</w:body></w:document>"
        ))
        .unwrap()
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn identities_are_unique_idempotent_and_survive_reparse(count in 1usize..30, text in "[a-z漢😀]{0,20}") {
            let body = format!("<w:p><w:r><w:t>{text}</w:t><w:delText>削除</w:delText></w:r></w:p>").repeat(count);
            let mut raw = xml(&body, "");
            let manifest = prepare_nodes(&mut raw).unwrap();
            prop_assert_eq!(manifest.nodes.len(), count * 4);
            let ids: HashSet<_> = manifest.nodes.iter().map(|n| &n.id).collect();
            prop_assert_eq!(ids.len(), manifest.nodes.len());
            prop_assert_eq!(manifest.seeds().len(), count * 2);
            prop_assert_eq!(&manifest, &prepare_nodes(&mut raw).unwrap());
            let mut reparsed = DocxXmlDocument::parse(&raw.to_string().unwrap()).unwrap();
            prop_assert_eq!(&manifest, &prepare_nodes(&mut reparsed).unwrap());
            let by_id: std::collections::HashMap<_, _> = manifest.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
            for n in &manifest.nodes {
                if let Some(parent) = &n.parent { prop_assert!(by_id.contains_key(parent.as_str())); }
            }
        }
    }
    #[test]
    fn invalid_existing_identity_or_namespace_is_rejected_atomically() {
        let id = "0".repeat(32);
        for (body, attributes) in [
            (
                format!("<w:p jcrdt:id=\"{id}\"/><w:p jcrdt:id=\"{id}\"/>"),
                format!("xmlns:jcrdt=\"{NAMESPACE}\""),
            ),
            (
                "<w:p jcrdt:id=\"invalid\"/>".into(),
                format!("xmlns:jcrdt=\"{NAMESPACE}\""),
            ),
            ("<w:p/>".into(), "xmlns:jcrdt=\"urn:other\"".into()),
            ("<w:p/>".into(), format!("xmlns:alias=\"{NAMESPACE}\"")),
            (
                "<w:p/>".into(),
                format!("xmlns:jcrdt=\"{NAMESPACE}\" jcrdt:schema=\"2\""),
            ),
            ("<w:p><w:r><w:t><w:r/></w:t></w:r></w:p>".into(), "".into()),
        ] {
            let mut raw = xml(&body, &attributes);
            let before = raw.to_string().unwrap();
            assert!(prepare_nodes(&mut raw).is_err());
            assert_eq!(raw.to_string().unwrap(), before);
        }
    }
    #[test]
    fn preserves_compatibility_tokens_and_empty_text_nodes() {
        let mut raw = xml(
            "<w:p><w:r><w:t/></w:r></w:p>",
            &format!("xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\""),
        );
        let manifest = prepare_nodes(&mut raw).unwrap();
        assert_eq!(attr(raw.root(), "mc:Ignorable"), Some("w14 jcrdt"));
        assert_eq!(manifest.seeds()[0].text, "");
    }
}
