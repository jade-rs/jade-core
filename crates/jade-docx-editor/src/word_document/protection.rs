//! 編集の制限 and 文書の保護: Word's editing restrictions
//! (`w:documentProtection`: 読み取り専用, コメント, 変更履歴, フォームへの入力,
//! and 書式の制限), the exceptions that let everyone or a named editor change
//! parts of a restricted document (`w:permStart` / `w:permEnd`), and 書き込み
//! パスワード / 読み取り専用を推奨 (`w:writeProtection`).
//!
//! Passwords are hashed as ECMA-376 Part 1 §17.15.1.29 describes for
//! `w:algorithmName="SHA-512"`: the hash of the salt and the UTF-16LE
//! password, then hashed again with each iteration's number appended, 100 000
//! times. A protection Word wrote with its older `w:hash` / `w:salt`
//! attributes derives its key from the password another way first; such a
//! protection is read and kept, and lifted only with the document's own
//! 保護の中止 when its password cannot be checked here.

use super::*;
use sha2::{Digest, Sha512};

/// What a restricted document lets a reader do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditingKind {
    /// 変更不可 (読み取り専用).
    ReadOnly,
    /// コメント.
    Comments,
    /// 変更履歴.
    TrackedChanges,
    /// フォームへの入力.
    Forms,
}

impl EditingKind {
    pub fn attribute(self) -> &'static str {
        match self {
            EditingKind::ReadOnly => "readOnly",
            EditingKind::Comments => "comments",
            EditingKind::TrackedChanges => "trackedChanges",
            EditingKind::Forms => "forms",
        }
    }

    pub fn from_attribute(value: &str) -> Option<Self> {
        Some(match value {
            "readOnly" => EditingKind::ReadOnly,
            "comments" => EditingKind::Comments,
            "trackedChanges" => EditingKind::TrackedChanges,
            "forms" => EditingKind::Forms,
            _ => return None,
        })
    }
}

/// A password's hash as a protection element stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordHash {
    /// Base64.
    pub hash: String,
    /// Base64.
    pub salt: String,
    pub spin_count: u32,
    /// Written with the older `w:hash` / `w:salt` attributes.
    pub legacy: bool,
}

/// ECMA-376's SHA-512 password hash.
pub fn hash_password(password: &str, salt: &[u8], spin_count: u32) -> Vec<u8> {
    let utf16: Vec<u8> = password
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let mut hasher = Sha512::new();
    hasher.update(salt);
    hasher.update(&utf16);
    let mut hash: Vec<u8> = hasher.finalize().to_vec();
    for iteration in 0..spin_count {
        let mut hasher = Sha512::new();
        hasher.update(&hash);
        hasher.update(iteration.to_le_bytes());
        hash = hasher.finalize().to_vec();
    }
    hash
}

impl PasswordHash {
    /// `password` hashed over `salt` (16 random bytes) as Word does.
    pub fn new(password: &str, salt: &[u8]) -> Self {
        const SPIN: u32 = 100_000;
        Self {
            hash: BASE64.encode(hash_password(password, salt, SPIN)),
            salt: BASE64.encode(salt),
            spin_count: SPIN,
            legacy: false,
        }
    }

    /// Whether `password` is the one hashed; `None` when the hash is the
    /// older kind, which cannot be checked here.
    pub fn verifies(&self, password: &str) -> Option<bool> {
        if self.legacy {
            return None;
        }
        let salt = BASE64.decode(&self.salt).ok()?;
        Some(BASE64.encode(hash_password(password, &salt, self.spin_count)) == self.hash)
    }
}

/// `w:documentProtection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditingRestriction {
    /// What may be edited; `None` with `formatting` alone restricts only
    /// formatting.
    pub kind: Option<EditingKind>,
    /// 書式の制限: no direct formatting.
    pub formatting: bool,
    pub enforced: bool,
    pub password: Option<PasswordHash>,
}

/// `w:writeProtection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteProtection {
    /// 読み取り専用を推奨: Word asks whether to open read-only.
    pub recommended: bool,
    /// 書き込みパスワード.
    pub password: Option<PasswordHash>,
}

/// A range of the body everyone, or an editor, may change in a restricted
/// document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditableRange {
    pub id: String,
    pub start: (usize, usize),
    pub end: (usize, usize),
    /// `w:edGrp` (`everyone`, …) or `w:ed` (an editor's name).
    pub editors: String,
}

const SETTINGS: &str = "word/settings.xml";

fn attribute_of(element: &XmlElement, local: &str) -> Option<String> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.rsplit(':').next() == Some(local))
        .map(|attribute| attribute.value.clone())
}

fn password_of(element: &XmlElement) -> Option<PasswordHash> {
    if let (Some(hash), Some(salt)) = (
        attribute_of(element, "hashValue"),
        attribute_of(element, "saltValue"),
    ) {
        return Some(PasswordHash {
            hash,
            salt,
            spin_count: attribute_of(element, "spinCount")
                .and_then(|count| count.parse().ok())
                .unwrap_or(100_000),
            legacy: false,
        });
    }
    match (attribute_of(element, "hash"), attribute_of(element, "salt")) {
        (Some(hash), Some(salt)) => Some(PasswordHash {
            hash,
            salt,
            spin_count: attribute_of(element, "cryptSpinCount")
                .and_then(|count| count.parse().ok())
                .unwrap_or(100_000),
            legacy: true,
        }),
        _ => None,
    }
}

fn truthy(value: Option<String>) -> bool {
    value.is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

/// The attributes that store a password, in the ECMA-376 form.
fn password_attributes(prefix: &str, password: &PasswordHash) -> Vec<XmlAttribute> {
    let name = |local: &str| format!("{prefix}{local}");
    if password.legacy {
        // Kept as Word wrote it.
        return vec![
            XmlAttribute {
                name: name("cryptProviderType"),
                value: "rsaAES".into(),
            },
            XmlAttribute {
                name: name("cryptAlgorithmClass"),
                value: "hash".into(),
            },
            XmlAttribute {
                name: name("cryptAlgorithmType"),
                value: "typeAny".into(),
            },
            XmlAttribute {
                name: name("cryptAlgorithmSid"),
                value: "14".into(),
            },
            XmlAttribute {
                name: name("cryptSpinCount"),
                value: password.spin_count.to_string(),
            },
            XmlAttribute {
                name: name("hash"),
                value: password.hash.clone(),
            },
            XmlAttribute {
                name: name("salt"),
                value: password.salt.clone(),
            },
        ];
    }
    vec![
        XmlAttribute {
            name: name("algorithmName"),
            value: "SHA-512".into(),
        },
        XmlAttribute {
            name: name("hashValue"),
            value: password.hash.clone(),
        },
        XmlAttribute {
            name: name("saltValue"),
            value: password.salt.clone(),
        },
        XmlAttribute {
            name: name("spinCount"),
            value: password.spin_count.to_string(),
        },
    ]
}

impl WordDocument {
    fn settings_element(&self, local: &str) -> Option<XmlElement> {
        let xml = self.settings_xml_content()?;
        let parsed = DocxXmlDocument::parse(&xml).ok()?;
        parsed.root().children.iter().find_map(|child| match child {
            XmlNode::Element(element) if Self::xml_local_name(&element.name) == local => {
                Some(element.clone())
            }
            _ => None,
        })
    }

    /// Replaces (or with `None` removes) the settings element `local`,
    /// first among the settings for `w:writeProtection` as the schema puts
    /// it, after it for the others.
    fn set_settings_element(
        &mut self,
        local: &str,
        attributes: Option<Vec<(&str, String)>>,
    ) -> Result<(), String> {
        let previous = self
            .settings_xml_content()
            .ok_or_else(|| "settings.xml is not present in this package".to_string())?;
        let mut doc = DocxXmlDocument::parse(&previous)
            .map_err(|error| format!("failed to parse settings.xml: {error:?}"))?;
        let root_id = doc.root().id;
        let prefix = doc
            .root()
            .name
            .split_once(':')
            .map(|(prefix, _)| format!("{prefix}:"))
            .unwrap_or_default();
        let element = attributes.map(|attributes| {
            let attributes = attributes
                .into_iter()
                .map(|(name, value)| XmlAttribute {
                    name: format!("{prefix}{name}"),
                    value,
                })
                .collect();
            doc.create_element(format!("{prefix}{local}"), attributes, Vec::new())
        });
        let root = doc
            .element_mut(root_id)
            .ok_or_else(|| "settings.xml has no root element".to_string())?;
        let at = root.children.iter().position(|child| {
            matches!(child, XmlNode::Element(element) if Self::xml_local_name(&element.name) == local)
        });
        match (at, element) {
            (Some(at), Some(element)) => root.children[at] = XmlNode::Element(element),
            (Some(at), None) => {
                root.children.remove(at);
            }
            (None, Some(element)) => {
                let after_write_protection = root
                    .children
                    .iter()
                    .position(|child| {
                        matches!(child, XmlNode::Element(element)
                            if Self::xml_local_name(&element.name) == "writeProtection")
                    })
                    .map_or(0, |at| at + 1);
                let at = if local == "writeProtection" {
                    0
                } else {
                    after_write_protection
                };
                root.children.insert(at, XmlNode::Element(element));
            }
            (None, None) => {}
        }
        doc.rebuild_index();
        let content = doc
            .to_string()
            .map_err(|error| format!("failed to serialize settings.xml: {error:?}"))?;
        self.parts.insert(
            SETTINGS.to_string(),
            DocumentPart {
                path: SETTINGS.to_string(),
                relationships: Vec::new(),
                content,
                parsed_document: None,
            },
        );
        Ok(())
    }

    /// The document's editing restriction, enforced or not.
    pub fn editing_restriction(&self) -> Option<EditingRestriction> {
        let element = self.settings_element("documentProtection")?;
        let kind =
            attribute_of(&element, "edit").and_then(|edit| EditingKind::from_attribute(&edit));
        let formatting = truthy(attribute_of(&element, "formatting"));
        if kind.is_none() && !formatting {
            return None;
        }
        Some(EditingRestriction {
            kind,
            formatting,
            enforced: truthy(attribute_of(&element, "enforcement")),
            password: password_of(&element),
        })
    }

    /// Sets (or removes) the editing restriction.
    pub fn set_editing_restriction(
        &mut self,
        restriction: Option<&EditingRestriction>,
    ) -> Result<(), String> {
        let attributes = restriction.map(|restriction| {
            let mut attributes = vec![(
                "edit",
                restriction
                    .kind
                    .map_or("none", EditingKind::attribute)
                    .to_string(),
            )];
            if restriction.formatting {
                attributes.push(("formatting", "1".to_string()));
            }
            attributes.push((
                "enforcement",
                if restriction.enforced { "1" } else { "0" }.to_string(),
            ));
            if let Some(password) = &restriction.password {
                for attribute in password_attributes("", password) {
                    attributes.push((
                        // Static names for the settings writer.
                        match attribute.name.as_str() {
                            "cryptProviderType" => "cryptProviderType",
                            "cryptAlgorithmClass" => "cryptAlgorithmClass",
                            "cryptAlgorithmType" => "cryptAlgorithmType",
                            "cryptAlgorithmSid" => "cryptAlgorithmSid",
                            "cryptSpinCount" => "cryptSpinCount",
                            "hash" => "hash",
                            "salt" => "salt",
                            "algorithmName" => "algorithmName",
                            "hashValue" => "hashValue",
                            "saltValue" => "saltValue",
                            _ => "spinCount",
                        },
                        attribute.value,
                    ));
                }
            }
            attributes
        });
        self.set_settings_element("documentProtection", attributes)
    }

    /// 書き込みパスワード and 読み取り専用を推奨.
    pub fn write_protection(&self) -> Option<WriteProtection> {
        let element = self.settings_element("writeProtection")?;
        Some(WriteProtection {
            recommended: truthy(attribute_of(&element, "recommended")),
            password: password_of(&element),
        })
    }

    pub fn set_write_protection(
        &mut self,
        protection: Option<&WriteProtection>,
    ) -> Result<(), String> {
        let attributes = protection
            .filter(|protection| protection.recommended || protection.password.is_some())
            .map(|protection| {
                let mut attributes = Vec::new();
                if protection.recommended {
                    attributes.push(("recommended", "1".to_string()));
                }
                if let Some(password) = &protection.password {
                    let names = [
                        ("algorithmName", 0usize),
                        ("hashValue", 1),
                        ("saltValue", 2),
                        ("spinCount", 3),
                    ];
                    let values = password_attributes(
                        "",
                        &PasswordHash {
                            legacy: false,
                            ..password.clone()
                        },
                    );
                    for (name, index) in names {
                        attributes.push((name, values[index].value.clone()));
                    }
                }
                attributes
            });
        self.set_settings_element("writeProtection", attributes)
    }

    /// The exceptions of a restricted document: the body ranges between
    /// `w:permStart` and its `w:permEnd`, by paragraph and byte offset.
    pub fn editable_ranges(&self) -> Vec<EditableRange> {
        let Some(raw) = self.raw_document.as_ref() else {
            return Vec::new();
        };
        let mut open: Vec<(String, (usize, usize), String)> = Vec::new();
        let mut ranges = Vec::new();
        for index in 0..self.paragraph_count() {
            let Some(paragraph) = self.paragraph(index) else {
                continue;
            };
            let Some(element) = paragraph.node_id.and_then(|id| raw.element(id)) else {
                continue;
            };
            // Where each source run ends: a mark after it sits there.
            let mut ends = HashMap::new();
            let mut offset = 0usize;
            for run in &paragraph.runs {
                offset += run.text.len();
                if let Some(id) = run.node_id {
                    ends.insert(id, offset);
                }
            }
            let mut at = 0usize;
            fn walk(
                element: &XmlElement,
                index: usize,
                ends: &HashMap<XmlNodeId, usize>,
                at: &mut usize,
                open: &mut Vec<(String, (usize, usize), String)>,
                ranges: &mut Vec<EditableRange>,
            ) {
                for child in &element.children {
                    let XmlNode::Element(child) = child else {
                        continue;
                    };
                    match child.name.rsplit(':').next() {
                        Some("r") => {
                            if let Some(&end) = ends.get(&child.id) {
                                *at = end;
                            }
                        }
                        Some("permStart") => {
                            let id = attribute_of(child, "id").unwrap_or_default();
                            let editors = attribute_of(child, "edGrp")
                                .or_else(|| attribute_of(child, "ed"))
                                .unwrap_or_else(|| "everyone".into());
                            open.push((id, (index, *at), editors));
                        }
                        Some("permEnd") => {
                            let id = attribute_of(child, "id").unwrap_or_default();
                            if let Some(position) =
                                open.iter().position(|(open_id, _, _)| *open_id == id)
                            {
                                let (id, start, editors) = open.remove(position);
                                ranges.push(EditableRange {
                                    id,
                                    start,
                                    end: (index, *at),
                                    editors,
                                });
                            }
                        }
                        Some("pPr") => {}
                        _ => walk(child, index, ends, at, open, ranges),
                    }
                }
            }
            walk(element, index, &ends, &mut at, &mut open, &mut ranges);
        }
        ranges
    }

    /// Lets everyone change `start..end` of a restricted document
    /// (例外処理 › すべてのユーザー): a `w:permStart` / `w:permEnd` pair around
    /// the paragraphs it covers.
    pub fn add_editable_paragraphs(
        &mut self,
        first: usize,
        last: usize,
        editors: &str,
    ) -> Result<String, String> {
        let id = self
            .editable_ranges()
            .iter()
            .filter_map(|range| range.id.parse::<u32>().ok())
            .max()
            .map_or(1, |id| id + 1)
            .to_string();
        let first_id = self
            .paragraph(first)
            .and_then(|paragraph| paragraph.node_id)
            .ok_or("the paragraph has no XML")?;
        let last_id = self
            .paragraph(last)
            .and_then(|paragraph| paragraph.node_id)
            .ok_or("the paragraph has no XML")?;
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML")?;
        let editor_attribute = if editors == "everyone" || editors == "editors" {
            "w:edGrp"
        } else {
            "w:ed"
        };
        let start = raw.create_element(
            "w:permStart",
            vec![
                XmlAttribute {
                    name: "w:id".into(),
                    value: id.clone(),
                },
                XmlAttribute {
                    name: editor_attribute.into(),
                    value: editors.into(),
                },
            ],
            Vec::new(),
        );
        let end = raw.create_element(
            "w:permEnd",
            vec![XmlAttribute {
                name: "w:id".into(),
                value: id.clone(),
            }],
            Vec::new(),
        );
        let after_properties = |paragraph: &XmlElement| {
            paragraph
                .children
                .iter()
                .position(|node| matches!(node, XmlNode::Element(child) if child.name.rsplit(':').next() == Some("pPr")))
                .map_or(0, |at| at + 1)
        };
        let paragraph = raw
            .element_mut(first_id)
            .ok_or("the paragraph went missing")?;
        let at = after_properties(paragraph);
        paragraph.children.insert(at, XmlNode::Element(start));
        let paragraph = raw
            .element_mut(last_id)
            .ok_or("the paragraph went missing")?;
        paragraph.children.push(XmlNode::Element(end));
        raw.rebuild_index();
        Ok(id)
    }

    /// Takes exception `id` out.
    pub fn remove_editable_range(&mut self, id: &str) -> Result<(), String> {
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML")?;
        fn marks(element: &XmlElement, id: &str, found: &mut Vec<XmlNodeId>) {
            for child in &element.children {
                if let XmlNode::Element(child) = child {
                    if matches!(child.name.rsplit(':').next(), Some("permStart" | "permEnd"))
                        && attribute_of(child, "id").as_deref() == Some(id)
                    {
                        found.push(child.id);
                    } else {
                        marks(child, id, found);
                    }
                }
            }
        }
        let mut found = Vec::new();
        marks(raw.root(), id, &mut found);
        for node in found {
            if let Some(path) = raw.path(node) {
                raw.remove_node(&path).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// Whether the enforced restriction lets `name` — a command — run with
    /// the selection at `at`: an edit inside an exception range always may;
    /// 変更履歴 lets every edit run (the shells keep tracking on);
    /// コメント lets only comments; 書式の制限 stops formatting.
    pub fn restriction_allows(&self, name: &str, at: Option<(usize, usize)>) -> Result<(), String> {
        if self.opened_read_only && !name.contains("protection") && !name.contains("restriction") {
            return Err("opened read-only".into());
        }
        let Some(restriction) = self
            .editing_restriction()
            .filter(|restriction| restriction.enforced)
        else {
            return Ok(());
        };
        // Setting or lifting the protection itself is not an edit.
        if self.protection_lifted || name.contains("protection") || name.contains("restriction") {
            return Ok(());
        }
        let commenting = matches!(
            name,
            "add_comment"
                | "reply_to_comment"
                | "update_comment"
                | "delete_comment"
                | "resolve_comment"
                | "headless_comment"
        ) || name.contains("comment");
        let formatting = FORMATTING_COMMANDS.contains(&name)
            || super::review_tools::PARAGRAPH_FORMAT_COMMANDS.contains(&name);
        if restriction.formatting && formatting {
            return Err("protected: formatting is restricted".into());
        }
        let inside_exception = at.is_some_and(|at| {
            self.editable_ranges()
                .iter()
                .any(|range| range.start <= at && at <= range.end)
        });
        match restriction.kind {
            None | Some(EditingKind::TrackedChanges) => Ok(()),
            Some(EditingKind::Comments) if commenting => Ok(()),
            _ if inside_exception => Ok(()),
            Some(kind) => Err(format!("protected: {}", kind.attribute())),
        }
    }

    /// Opened 読み取り専用 (as the file recommends, or marked final): no
    /// edit runs until the reader chooses to edit.
    pub fn set_opened_read_only(&mut self, read_only: bool) {
        self.opened_read_only = read_only;
    }

    pub fn opened_read_only(&self) -> bool {
        self.opened_read_only
    }

    /// 保護の中止 with the right password (or on a protection whose
    /// password cannot be checked): edits run unrestricted until the
    /// document is opened again.
    pub fn lift_protection(&mut self, lifted: bool) {
        self.protection_lifted = lifted;
    }
}

/// The commands that format characters directly.
const FORMATTING_COMMANDS: [&str; 18] = [
    "apply_bold",
    "apply_italic",
    "apply_underline",
    "apply_strike",
    "apply_superscript",
    "apply_subscript",
    "set_bold",
    "set_font_size",
    "adjust_font_size",
    "set_font_family",
    "set_font_color",
    "set_highlight_color",
    "set_run_format",
    "clear_formatting",
    "set_character_style",
    "change_case",
    "set_ruby",
    "format_text",
];

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor;

    fn document(body: &str, settings: &str) -> WordDocument {
        use std::io::Write;
        let parts = [
            ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#.to_string()),
            ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
            ("word/_rels/document.xml.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#.to_string()),
            ("word/settings.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{settings}<w:zoom w:percent="100"/></w:settings>"#)),
            ("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr/></w:body></w:document>"#)),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap()
    }

    #[test]
    fn a_restriction_with_a_password_round_trips_and_verifies() {
        let mut document = document("<w:p><w:r><w:t>text</w:t></w:r></w:p>", "");
        assert!(document.editing_restriction().is_none());
        let password = PasswordHash::new("secret", &[7u8; 16]);
        assert_eq!(password.verifies("secret"), Some(true));
        assert_eq!(password.verifies("other"), Some(false));
        let restriction = EditingRestriction {
            kind: Some(EditingKind::TrackedChanges),
            formatting: false,
            enforced: true,
            password: Some(password.clone()),
        };
        document
            .set_editing_restriction(Some(&restriction))
            .unwrap();
        let saved = document.export_docx().unwrap();
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(reopened.editing_restriction(), Some(restriction));
        let settings = reopened.settings_xml_content().unwrap();
        assert!(
            settings.contains(r#"w:edit="trackedChanges""#),
            "{settings}"
        );
        assert!(
            settings.contains(r#"w:algorithmName="SHA-512""#),
            "{settings}"
        );
    }

    #[test]
    fn write_protection_recommends_read_only() {
        let mut document = document("<w:p/>", "");
        document
            .set_write_protection(Some(&WriteProtection {
                recommended: true,
                password: None,
            }))
            .unwrap();
        let settings = document.settings_xml_content().unwrap();
        assert!(
            settings.contains(r#"<w:writeProtection w:recommended="1"/>"#),
            "{settings}"
        );
        assert_eq!(
            document.write_protection(),
            Some(WriteProtection {
                recommended: true,
                password: None
            })
        );
        document.set_write_protection(None).unwrap();
        assert!(document.write_protection().is_none());
    }

    #[test]
    fn read_only_lets_only_the_exceptions_be_edited() {
        let mut document = document(
            r#"<w:p><w:r><w:t>locked</w:t></w:r></w:p><w:p><w:permStart w:id="3" w:edGrp="everyone"/><w:r><w:t>open to all</w:t></w:r><w:permEnd w:id="3"/></w:p>"#,
            r#"<w:documentProtection w:edit="readOnly" w:enforcement="1"/>"#,
        );
        let ranges = document.editable_ranges();
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(ranges[0].start, (1, 0));
        assert_eq!(ranges[0].end, (1, "open to all".len()));
        assert_eq!(ranges[0].editors, "everyone");
        assert!(document
            .restriction_allows("insert_text", Some((0, 2)))
            .is_err());
        assert!(document
            .restriction_allows("insert_text", Some((1, 2)))
            .is_ok());
        document.lift_protection(true);
        assert!(document
            .restriction_allows("insert_text", Some((0, 2)))
            .is_ok());
        document.lift_protection(false);

        // An exception added round the first paragraph, saved and read back.
        document.add_editable_paragraphs(0, 0, "everyone").unwrap();
        let saved = document.export_docx().unwrap();
        let mut reopened =
            executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(reopened.editable_ranges().len(), 2);
        assert!(reopened
            .restriction_allows("insert_text", Some((0, 2)))
            .is_ok());
        reopened.remove_editable_range("3").unwrap();
        assert_eq!(reopened.editable_ranges().len(), 1);
    }

    #[test]
    fn comments_only_and_formatting_restrictions() {
        let document = document(
            "<w:p><w:r><w:t>text</w:t></w:r></w:p>",
            r#"<w:documentProtection w:edit="comments" w:formatting="1" w:enforcement="1"/>"#,
        );
        assert!(document
            .restriction_allows("add_comment", Some((0, 1)))
            .is_ok());
        assert!(document
            .restriction_allows("insert_text", Some((0, 1)))
            .is_err());
        assert!(document
            .restriction_allows("apply_bold", Some((0, 1)))
            .is_err());
    }
}
