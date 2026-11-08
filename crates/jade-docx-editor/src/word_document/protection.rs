//! 編集の制限 and 文書の保護: Word's editing restrictions
//! (`w:documentProtection`: 読み取り専用, コメント, 変更履歴, フォームへの入力,
//! and 書式の制限), the exceptions that let everyone or a named editor change
//! parts of a restricted document (`w:permStart` / `w:permEnd`), and 書き込み
//! パスワード / 読み取り専用を推奨 (`w:writeProtection`).
//!
//! New passwords use SHA-512 with a random salt and 100,000 iterations.
//! Imported SHA-1/256/384/512 verifiers retain their algorithm and attribute
//! form. Unsupported or malformed verifiers never authorize lifting protection.

use super::*;
use sha2::Sha512;

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
    /// OOXML algorithm name, or `sid:<number>` for an unknown legacy identifier.
    pub algorithm: String,
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
    hash_password_with::<Sha512>(password, salt, spin_count)
}

fn hash_password_with<D: sha2::Digest + Default>(
    password: &str,
    salt: &[u8],
    spin_count: u32,
) -> Vec<u8> {
    let mut hasher = D::new();
    hasher.update(salt);
    for unit in password.encode_utf16() {
        hasher.update(unit.to_le_bytes());
    }
    let mut hash = hasher.finalize().to_vec();
    for iteration in 0..spin_count {
        let mut hasher = D::new();
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
            algorithm: "SHA-512".into(),
            hash: BASE64.encode(hash_password(password, salt, SPIN)),
            salt: BASE64.encode(salt),
            spin_count: SPIN,
            legacy: false,
        }
    }

    /// Whether `password` matches; unsupported or malformed verifiers return
    /// `None` and must never be treated as successful authentication.
    pub fn verifies(&self, password: &str) -> Option<bool> {
        // Validate sizes and the work factor before allocating or hashing. The
        // shared OOXML validator also accepts XML Schema base64 whitespace.
        jade_opc::protection::PasswordVerifier {
            legacy: None,
            algorithm: Some(self.algorithm.clone()),
            hash: Some(self.hash.clone()),
            salt: Some(self.salt.clone()),
            spin_count: Some(self.spin_count.to_string()),
        }
        .validate()
        .ok()?;
        let compact = |value: &str| {
            value
                .chars()
                .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
                .collect::<String>()
        };
        let salt = BASE64.decode(compact(&self.salt)).ok()?;
        let expected = BASE64.decode(compact(&self.hash)).ok()?;
        let actual = match self.algorithm.as_str() {
            "SHA-1" => hash_password_with::<sha1::Sha1>(password, &salt, self.spin_count),
            "SHA-256" => hash_password_with::<sha2::Sha256>(password, &salt, self.spin_count),
            "SHA-384" => hash_password_with::<sha2::Sha384>(password, &salt, self.spin_count),
            "SHA-512" => hash_password(password, &salt, self.spin_count),
            _ => return None,
        };
        if expected.len() != actual.len() {
            return None;
        }
        Some(
            expected
                .iter()
                .zip(actual)
                .fold(0u8, |difference, (a, b)| difference | (a ^ b))
                == 0,
        )
    }

    pub fn verify_for_unlock(&self, password: &str) -> Result<(), String> {
        match self.verifies(password) {
            Some(true) => Ok(()),
            Some(false) => Err("the password is wrong".into()),
            None => Err("the protection password format is unsupported or invalid".into()),
        }
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

fn attribute_of(element: &XmlElement, local: &str) -> Option<String> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.rsplit(':').next() == Some(local))
        .map(|attribute| attribute.value.clone())
}

fn password_of(element: &XmlElement) -> Option<PasswordHash> {
    let modern = attribute_of(element, "hashValue").is_some()
        || attribute_of(element, "saltValue").is_some()
        || attribute_of(element, "algorithmName").is_some()
        || attribute_of(element, "spinCount").is_some();
    let legacy = attribute_of(element, "hash").is_some()
        || attribute_of(element, "salt").is_some()
        || attribute_of(element, "cryptAlgorithmSid").is_some()
        || attribute_of(element, "cryptSpinCount").is_some();
    if !modern && !legacy {
        return None;
    }
    let (hash, salt, spin_count, algorithm) = if modern {
        (
            "hashValue",
            "saltValue",
            "spinCount",
            attribute_of(element, "algorithmName").unwrap_or_default(),
        )
    } else {
        let sid = attribute_of(element, "cryptAlgorithmSid").unwrap_or_default();
        let algorithm = match sid.as_str() {
            "4" => "SHA-1".into(),
            "12" => "SHA-256".into(),
            "13" => "SHA-384".into(),
            "14" => "SHA-512".into(),
            _ => format!("sid:{sid}"),
        };
        ("hash", "salt", "cryptSpinCount", algorithm)
    };
    Some(PasswordHash {
        algorithm,
        hash: attribute_of(element, hash).unwrap_or_default(),
        salt: attribute_of(element, salt).unwrap_or_default(),
        spin_count: attribute_of(element, spin_count)
            .map(|count| {
                count
                    .trim_matches([' ', '\t', '\r', '\n'])
                    .parse()
                    .unwrap_or(u32::MAX)
            })
            .unwrap_or(100_000),
        legacy: !modern,
    })
}

fn truthy(value: Option<String>) -> bool {
    value.is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

/// The attributes that store a password, in the ECMA-376 form.
pub(super) fn password_attributes(prefix: &str, password: &PasswordHash) -> Vec<XmlAttribute> {
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
                value: match password.algorithm.as_str() {
                    "SHA-1" => "4",
                    "SHA-256" => "12",
                    "SHA-384" => "13",
                    "SHA-512" => "14",
                    other => other.strip_prefix("sid:").unwrap_or(other),
                }
                .into(),
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
            value: password.algorithm.clone(),
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

fn password_setting_attributes(password: &PasswordHash) -> Vec<(&'static str, String)> {
    let mut attributes = Vec::new();
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
    attributes
}

impl WordDocument {
    fn settings_element(&self, local: &str) -> Option<XmlElement> {
        // Every edit asks for `w:documentProtection` (`restriction_allows`):
        // a document without one is answered without parsing its settings.
        if local == "documentProtection" && !self.settings_mention_document_protection() {
            return None;
        }
        let xml = self.settings_xml_str()?;
        if local != "documentProtection" && !xml.contains(local) {
            return None;
        }
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
        let existing = self.settings_xml_content();
        if existing.is_none() && attributes.is_none() {
            return Ok(());
        }
        let previous = existing.unwrap_or_else(|| {
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.into()
        });
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
        let path = self.tracking_settings_path();
        self.write_tracking_settings_xml(&path, content)
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
                attributes.extend(password_setting_attributes(password));
            }
            attributes
        });
        self.set_settings_element("documentProtection", attributes)?;
        if self.tracking_locked() {
            let session = self.revision_session().cloned().unwrap_or(RevisionSession {
                author: Some("jade".into()),
                date: None,
                date_utc: None,
            });
            self.set_saved_revision_session(Some(session))?;
        }
        Ok(())
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
                    attributes.extend(password_setting_attributes(password));
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
    /// 変更履歴 permits edits while preventing recording from being stopped;
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
        // Setting or lifting the protection itself is not an edit
        // (保護の開始 here, フォームの保護 on the 開発 tab).
        if self.protection_lifted
            || name.contains("protection")
            || name.contains("restriction")
            || name == "protect_forms"
        {
            return Ok(());
        }
        if restriction.kind == Some(EditingKind::TrackedChanges)
            && matches!(
                name,
                "stop_change_tracking" | "accept_revisions" | "reject_revisions"
            )
        {
            return Err("tracking is locked".into());
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
        // フォームへの入力: filling a form's fields and controls is what the
        // protection leaves; where typing may land is the content controls'
        // own check (`edit_refusal`).
        let filling_form = restriction.kind == Some(EditingKind::Forms)
            && (FORM_FILLING_COMMANDS.contains(&name)
                || at.is_some_and(|(paragraph, offset)| {
                    let at = TextPosition::new(paragraph, offset);
                    self.edit_refusal(at, at, false).is_none()
                }));
        match restriction.kind {
            None | Some(EditingKind::TrackedChanges) => Ok(()),
            Some(EditingKind::Comments) if commenting => Ok(()),
            _ if inside_exception || filling_form => Ok(()),
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

    /// Session-only override for a host that has already authorized editing.
    /// This low-level switch performs no password authentication itself.
    pub fn lift_protection(&mut self, lifted: bool) {
        self.protection_lifted = lifted;
    }
}

/// The commands that format characters directly.
/// The commands that fill a form in — its check boxes, lists, dates and
/// text — which a document protected for forms still runs.
const FORM_FILLING_COMMANDS: [&str; 8] = [
    "set_content_control_checked",
    "choose_content_control_entry",
    "set_content_control_text",
    "set_content_control_date",
    "set_content_control_picture",
    "set_form_check_box",
    "choose_form_entry",
    "set_form_text",
];

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

    fn assert_forms_guards(document: &WordDocument, protected: bool) {
        assert_eq!(
            document
                .editing_restriction()
                .map(|restriction| restriction.kind),
            protected.then_some(Some(EditingKind::Forms)),
        );
        assert_eq!(
            document
                .restriction_allows("insert_text", Some((0, 0)))
                .is_err(),
            protected,
        );
        let command = jade_docx_headless_api::HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text: "あ".into(),
        };
        assert_eq!(
            document.headless_edit_refusal(&command).is_some(),
            protected
        );
    }

    #[test]
    fn editing_guards_follow_settings_overrides_and_removal() {
        const SETTINGS_PATH: &str = "word/settings.xml";
        let body = "<w:p><w:r><w:t>text</w:t></w:r></w:p>";
        let plain = document(body, "");
        let protected = document(
            body,
            r#"<w:documentProtection w:edit="forms" w:enforcement="1"/>"#,
        );
        let plain_settings = plain.settings_xml_content().unwrap();
        let protected_settings = protected.settings_xml_content().unwrap();
        for (mut document, source_protected) in [(plain, false), (protected, true)] {
            // Exercise both the loaded part and the source ZIP fallback.
            assert_forms_guards(&document, source_protected);
            document.parts.remove(SETTINGS_PATH);
            assert_forms_guards(&document, source_protected);
            for _ in 0..2 {
                document.restore_settings_xml(protected_settings.clone());
                assert_forms_guards(&document, true);
                // The public part can also be changed without a setter.
                document.parts.get_mut(SETTINGS_PATH).unwrap().content = plain_settings.clone();
                assert_forms_guards(&document, false);
                document.parts.remove(SETTINGS_PATH);
                assert_forms_guards(&document, source_protected);
            }
        }
    }

    #[test]
    fn editing_guards_follow_replacement_snapshots() {
        let body = "<w:p><w:r><w:t>text</w:t></w:r></w:p>";
        let mut plain = document(body, "");
        let mut protected = document(
            body,
            r#"<w:documentProtection w:edit="forms" w:enforcement="1"/>"#,
        );
        for document in [&mut plain, &mut protected] {
            document.parts.remove("word/settings.xml");
        }
        assert_forms_guards(&plain, false);
        assert_forms_guards(&protected, true);
        let plain_snapshot = plain.snapshot_for_replacement().unwrap();
        let protected_snapshot = protected.snapshot_for_replacement().unwrap();
        plain.restore_snapshot(protected_snapshot);
        assert_forms_guards(&plain, true);
        protected.restore_snapshot(plain_snapshot);
        assert_forms_guards(&protected, false);
    }

    #[test]
    fn word_password_algorithms_verify_and_round_trip_both_attribute_forms() {
        // Independent hashlib vectors: UTF-16LE, fixed 16-byte salt, 23 spins.
        for (algorithm, sid, hash) in [
            ("SHA-1", "4", "2BGoHn7VxIl6R998wai+DvGs95o="),
            ("SHA-256", "12", "AEbC0IVM3fPhq+u+0BmUQ2Dkr7+7ZUe+Qe995ZlecyE="),
            ("SHA-384", "13", "4G1uhh3KDpbu+gAoqm5AEV7RD7lLsYuKyQ9lOBn8mSyNW6aAgA4KvTwgQTNQN1Qs"),
            ("SHA-512", "14", "tsizw+IXGC/j/xv06GsA7ZT5UNaU04jZXgEGGH8epcmupy9GttK1YLyPCJPzOaFu0U5fIWeojfNyASVxv+qpzw=="),
        ] {
            for legacy in [false, true] {
                let attributes = if legacy {
                    format!(r#"w:cryptAlgorithmSid="{sid}" w:cryptSpinCount="23" w:hash="{hash}" w:salt="MDEyMzQ1Njc4OWFiY2RlZg==""#)
                } else {
                    format!(r#"w:algorithmName="{algorithm}" w:spinCount="23" w:hashValue="{hash}" w:saltValue="MDEyMzQ1Njc4OWFiY2RlZg==""#)
                };
                let settings = format!(r#"<w:documentProtection w:edit="readOnly" w:enforcement="1" {attributes}/>"#);
                let mut doc = document("<w:p/>", &settings);
                let restriction = doc.editing_restriction().unwrap();
                let password = restriction.password.as_ref().unwrap();
                assert_eq!(password.algorithm, algorithm);
                assert_eq!(password.verifies("秘密🔑"), Some(true));
                assert!(password.verify_for_unlock("秘密🔑").is_ok());
                assert_eq!(password.verifies("wrong"), Some(false));
                assert!(password.verify_for_unlock("wrong").is_err());
                doc.set_editing_restriction(Some(&restriction)).unwrap();
                let write = WriteProtection { recommended: true, password: Some(password.clone()) };
                doc.set_write_protection(Some(&write)).unwrap();
                assert_eq!(doc.write_protection(), Some(write));
                let reopened = executor::block_on(WordDocument::load(&doc.export_docx().unwrap(), &Options::default())).unwrap();
                assert_eq!(reopened.editing_restriction(), Some(restriction));
            }
        }
    }

    #[test]
    fn imported_password_whitespace_and_invalid_work_factors() {
        let hash = "2BGoHn7VxIl6R998wai+DvGs95o=";
        for legacy in [false, true] {
            let (algorithm, spin, hash_name, salt_name) = if legacy {
                ("cryptAlgorithmSid=\"4\"", "cryptSpinCount", "hash", "salt")
            } else {
                (
                    "algorithmName=\"SHA-1\"",
                    "spinCount",
                    "hashValue",
                    "saltValue",
                )
            };
            for count in [" 23 ", "-1", "garbage", "4294967296", "10000001"] {
                let settings = format!(
                    r#"<w:documentProtection w:edit="readOnly" w:enforcement="1" w:{algorithm} w:{spin}="{count}" w:{hash_name}="  {hash}  " w:{salt_name}="MDEyMzQ1 Njc4OWFiY2RlZg=="/>"#
                );
                let doc = document("<w:p/>", &settings);
                let restriction = doc.editing_restriction().unwrap();
                let password = restriction.password.unwrap();
                if count == " 23 " {
                    assert_eq!(password.verifies("秘密🔑"), Some(true));
                    assert_eq!(password.verifies("incorrect"), Some(false));
                } else {
                    assert_eq!(password.verifies("秘密🔑"), None, "{count}");
                }
            }
            let doc = document(
                "<w:p/>",
                &format!(
                    r#"<w:documentProtection w:edit="readOnly" w:enforcement="1" w:{spin}="23"/>"#
                ),
            );
            assert!(doc
                .editing_restriction()
                .unwrap()
                .password
                .unwrap()
                .verify_for_unlock("anything")
                .is_err());
        }
    }

    #[test]
    fn verifier_bounds_metadata_and_accepts_xml_base64_whitespace() {
        let mut password = PasswordHash {
            algorithm: "SHA-1".into(),
            hash: "2BGoHn7VxIl6R998wai+DvGs95o=".into(),
            salt: "MDEyMzQ1Njc4OWFiY2RlZg==".into(),
            spin_count: 23,
            legacy: false,
        };
        password.hash.insert_str(8, " \t\r\n");
        password.salt.insert_str(8, " \t\r\n");
        assert_eq!(password.verifies("秘密🔑"), Some(true));
        password.salt = BASE64.encode(vec![0u8; 65_537]);
        assert_eq!(password.verifies("秘密🔑"), None);
        password.salt.clear();
        password.hash = BASE64.encode(vec![0u8; 65]);
        assert_eq!(password.verifies("秘密🔑"), None);
    }

    #[test]
    fn unverifiable_passwords_remain_protected_and_keep_the_algorithm() {
        for attributes in [
            r#"w:cryptAlgorithmSid="99" w:hash="AQ==" w:salt="Ag==""#,
            r#"w:algorithmName="WHIRLPOOL" w:hashValue="AQ==" w:saltValue="Ag==""#,
            r#"w:algorithmName="SHA-512" w:hashValue="invalid" w:saltValue="Ag==""#,
            r#"w:algorithmName="SHA-512" w:hashValue="AQ==""#,
        ] {
            let settings = format!(
                r#"<w:documentProtection w:edit="readOnly" w:enforcement="1" {attributes}/>"#
            );
            let mut doc = document("<w:p/>", &settings);
            let restriction = doc.editing_restriction().unwrap();
            let password = restriction.password.as_ref().unwrap();
            assert!(password.verify_for_unlock("anything").is_err());
            assert!(doc.restriction_allows("insert_text", None).is_err());
            doc.set_editing_restriction(Some(&restriction)).unwrap();
            assert_eq!(doc.editing_restriction(), Some(restriction));
        }
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
