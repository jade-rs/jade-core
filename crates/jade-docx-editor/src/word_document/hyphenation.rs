//! Word's ハイフネーション settings (`w:autoHyphenation`,
//! `w:hyphenationZone`, `w:consecutiveHyphenLimit`, `w:doNotHyphenateCaps`)
//! as the Hyphenation dialog edits them: in `settings.xml`, and on the model
//! the renderer lays out from.

use super::{DocumentPart, WordDocument};
use crate::docx_xml::{DocxXmlDocument, XmlNode};
use jade_docx_headless_api::HeadlessHyphenationPatch;

impl WordDocument {
    /// The document's automatic hyphenation.
    pub fn hyphenation(&self) -> jade_docx_reader::HyphenationSettings {
        self.document
            .as_deref()
            .map(|document| document.hyphenation)
            .unwrap_or_default()
    }

    /// Applies what `patch` names. A package without `settings.xml` still lays
    /// out with the change; it is only not saved.
    pub fn set_hyphenation(&mut self, patch: &HeadlessHyphenationPatch) -> Result<(), String> {
        if self.document.is_none() {
            return Err("document is unavailable".to_string());
        }
        let mut settings = self.hyphenation();
        if let Some(auto) = patch.auto {
            settings.auto = auto;
            let _ = self.set_settings_flag("w:autoHyphenation", auto);
        }
        if let Some(caps) = patch.hyphenate_caps {
            settings.do_not_hyphenate_caps = !caps;
            let _ = self.set_settings_flag("w:doNotHyphenateCaps", !caps);
        }
        if let Some(zone) = patch.zone {
            if !zone.is_finite() || zone < 0.0 {
                return Err("the hyphenation zone must be zero or more".to_string());
            }
            let twips = (zone * 20.0).round() as u32;
            settings.zone_twips = Some(twips);
            let _ = self.set_settings_value("w:hyphenationZone", Some(twips.to_string()));
        }
        if let Some(limit) = patch.consecutive_limit {
            settings.consecutive_limit = (limit > 0).then_some(limit);
            let value = (limit > 0).then(|| limit.to_string());
            let _ = self.set_settings_value("w:consecutiveHyphenLimit", value);
        }
        if let Some(document) = self.document_mut() {
            document.hyphenation = settings;
        }
        Ok(())
    }

    /// Sets a `settings.xml` element carrying `w:val`, or removes it.
    pub fn set_settings_value(&mut self, name: &str, value: Option<String>) -> Result<(), String> {
        const SETTINGS_PATH: &str = "word/settings.xml";
        let previous = self
            .settings_xml_content()
            .ok_or_else(|| "settings.xml is not present in this package".to_string())?;
        let mut doc = DocxXmlDocument::parse(&previous)
            .map_err(|err| format!("failed to parse settings.xml: {err:?}"))?;
        let created = value.as_ref().map(|value| {
            doc.create_element(
                name.to_string(),
                vec![crate::docx_xml::XmlAttribute {
                    name: "w:val".to_string(),
                    value: value.clone(),
                }],
                Vec::new(),
            )
        });
        let root_id = doc.root().id;
        let root = doc
            .element_mut(root_id)
            .ok_or_else(|| "settings.xml has no root element".to_string())?;
        let existing = root
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(element) if element.name == name));
        match (existing, created) {
            (Some(index), Some(element)) => root.children[index] = XmlNode::Element(element),
            (None, Some(element)) => root.children.push(XmlNode::Element(element)),
            (Some(index), None) => {
                root.children.remove(index);
            }
            (None, None) => return Ok(()),
        }
        doc.rebuild_index();
        let content = doc
            .to_string()
            .map_err(|err| format!("failed to serialize settings.xml: {err:?}"))?;
        self.parts.insert(
            SETTINGS_PATH.to_string(),
            DocumentPart {
                path: SETTINGS_PATH.to_string(),
                relationships: Vec::new(),
                content,
                parsed_document: None,
            },
        );
        Ok(())
    }
}
