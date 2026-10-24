//! 校閲 › 保護: 編集の制限 (the pane: 書式の制限, 編集の制限 — 変更不可,
//! 変更履歴, コメント, フォームへの入力 — 例外処理 for everyone, 保護の開始
//! and 保護の中止 with a password), 変更履歴のロック (the same restriction to
//! tracked changes), 文書の保護 (読み取りパスワード, 書き込みパスワード,
//! 読み取り専用を推奨) and 最終版にする.
//!
//! The restrictions are the engine's (`w:documentProtection`,
//! `w:permStart`, `w:writeProtection`), which its command executor
//! enforces; the password to open is `jade-opc`'s Agile Encryption; 最終版
//! is the `_MarkAsFinal` custom property, written into the package on save.

use crate::commands::snapshot_edit::{EditScope, SnapshotEditCommand};
pub use crate::word_document::{
    EditableRange, EditingKind, EditingRestriction, PasswordHash, WriteProtection,
};
pub use jade_opc::encryption::{
    decrypt_package, encrypt_package, is_encrypted_package, DecryptError,
};

use super::WordSession;
use crate::text_edit::TextEdit;

/// A document whose session can be opened read-only.
pub trait WordReadOnly {
    fn set_opened_read_only(&mut self, read_only: bool);
    fn opened_read_only(&self) -> bool;
}

/// Restrictions and protection for every session.
pub trait WordProtection: WordSession {
    fn editing_restriction(&self) -> Option<EditingRestriction> {
        self.word_document().editing_restriction()
    }

    /// Whether a restriction is in force now.
    fn is_restricted(&self) -> bool {
        self.editing_restriction()
            .is_some_and(|restriction| restriction.enforced)
    }

    /// Whether edits are being kept as tracked changes by a restriction
    /// (変更履歴のロック): tracking may not be turned off.
    fn tracking_locked(&self) -> bool {
        self.editing_restriction().is_some_and(|restriction| {
            restriction.enforced && restriction.kind == Some(EditingKind::TrackedChanges)
        })
    }

    /// 保護の開始: `kind` (or only formatting), with a password when one is
    /// given, `salt` being 16 random bytes.
    fn start_restriction(
        &mut self,
        kind: Option<EditingKind>,
        formatting: bool,
        password: Option<&str>,
        salt: [u8; 16],
    ) -> Result<(), String> {
        if kind.is_none() && !formatting {
            return Err("choose what is restricted".to_string());
        }
        let restriction = EditingRestriction {
            kind,
            formatting,
            enforced: true,
            password: password
                .filter(|password| !password.is_empty())
                .map(|password| PasswordHash::new(password, &salt)),
        };
        self.execute_command(
            Box::new(SnapshotEditCommand::with_scope(
                "set_editing_restriction",
                EditScope::Paragraphs(None),
                move |document| document.set_editing_restriction(Some(&restriction)),
            )),
            true,
        )
    }

    /// 保護の中止: with the password when the restriction has one. A
    /// protection Word wrote with its older hash cannot be checked here
    /// and is lifted as Word lifts one without a password.
    fn stop_restriction(&mut self, password: &str) -> Result<(), String> {
        let Some(restriction) = self.editing_restriction() else {
            return Ok(());
        };
        if let Some(hash) = &restriction.password {
            if hash.verifies(password) == Some(false) {
                return Err("the password is wrong".to_string());
            }
        }
        // Not enforced any more, the choices kept for next time.
        let stopped = EditingRestriction {
            enforced: false,
            password: None,
            ..restriction
        };
        self.execute_command(
            Box::new(SnapshotEditCommand::with_scope(
                "set_editing_restriction",
                EditScope::Paragraphs(None),
                move |document| document.set_editing_restriction(Some(&stopped)),
            )),
            true,
        )
    }

    /// The exceptions: the ranges everyone may edit.
    fn editable_ranges(&self) -> Vec<EditableRange> {
        self.word_document().editable_ranges()
    }

    /// 例外処理 › すべてのユーザー for the selected paragraphs.
    fn allow_everyone_here(&mut self) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        let (first, last) = (start.paragraph, end.paragraph);
        self.execute_command(
            Box::new(SnapshotEditCommand::with_scope(
                "add_editable_range_restriction",
                EditScope::Paragraphs(Some((first, last))),
                move |document| {
                    document
                        .add_editable_paragraphs(first, last, "everyone")
                        .map(|_| ())
                },
            )),
            true,
        )
    }

    /// An exception taken out.
    fn remove_exception(&mut self, id: &str) -> Result<(), String> {
        let id = id.to_string();
        self.execute_command(
            Box::new(SnapshotEditCommand::new(
                "remove_editable_range_restriction",
                move |document| document.remove_editable_range(&id),
            )),
            true,
        )
    }

    fn write_protection(&self) -> Option<WriteProtection> {
        self.word_document().write_protection()
    }

    /// 書き込みパスワード and 読み取り専用を推奨, together.
    fn set_write_protection(
        &mut self,
        recommended: bool,
        password: Option<&str>,
        salt: [u8; 16],
    ) -> Result<(), String> {
        let current = self.write_protection();
        let password = match password {
            Some("") => None,
            Some(password) => Some(PasswordHash::new(password, &salt)),
            // Unchanged.
            None => current.and_then(|protection| protection.password),
        };
        let protection = WriteProtection {
            recommended,
            password,
        };
        self.execute_command(
            Box::new(SnapshotEditCommand::with_scope(
                "set_write_protection",
                EditScope::Paragraphs(None),
                move |document| document.set_write_protection(Some(&protection)),
            )),
            true,
        )
    }
}

impl<T: WordSession + ?Sized> WordProtection for T {}

/// Random bytes from the system, for salts and keys.
pub fn random_bytes(buffer: &mut [u8]) {
    // The OS's random source, or in a browser crypto.getRandomValues.
    if getrandom::getrandom(buffer).is_ok() {
        return;
    }
    fallback_bytes(buffer);
}

/// Bytes when the system has no random source: the time and an address,
/// stirred. Not in a browser, which has no clock to read (`SystemTime::now`
/// panics in wasm) — the bytes stay as they are there.
fn fallback_bytes(buffer: &mut [u8]) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0)
            ^ (buffer.as_ptr() as u64);
        for byte in buffer {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = buffer;
}

/// A salt.
pub fn salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    random_bytes(&mut salt);
    salt
}

/// Whether a package's custom properties mark it final.
pub fn is_marked_final(package: &[u8]) -> bool {
    jade_opc::OpcPackage::open(package)
        .ok()
        .and_then(|package| package.part_str("docProps/custom.xml").ok().flatten())
        .is_some_and(|xml| {
            xml.split("<property").skip(1).any(|property| {
                property.contains(r#"name="_MarkAsFinal""#)
                    && (property.contains("<vt:bool>true</vt:bool>")
                        || property.contains("<vt:bool>1</vt:bool>"))
            })
        })
}

const CUSTOM: &str = "docProps/custom.xml";
const CUSTOM_TYPE: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
const CUSTOM_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
const FMTID: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";

/// 最終版にする (or undo it): the package with `_MarkAsFinal` set to
/// `final_` in its custom properties, the part and its relationship made
/// when there were none.
pub fn mark_final(package: &[u8], final_: bool) -> Result<Vec<u8>, String> {
    let mut opc = jade_opc::OpcPackage::open(package).map_err(|error| error.to_string())?;
    let existing = opc.part_str(CUSTOM).map_err(|error| error.to_string())?;
    let mut properties: Vec<String> = existing
        .as_deref()
        .map(|xml| {
            xml.split("<property")
                .skip(1)
                .filter_map(|property| {
                    let end = property.find("</property>")?;
                    Some(format!(
                        "<property{}",
                        &property[..end + "</property>".len()]
                    ))
                })
                .filter(|property| !property.contains(r#"name="_MarkAsFinal""#))
                .collect()
        })
        .unwrap_or_default();
    if final_ {
        properties.push(format!(
            r#"<property fmtid="{FMTID}" pid="0" name="_MarkAsFinal"><vt:bool>true</vt:bool></property>"#
        ));
    }
    // Word numbers the properties from 2.
    let renumbered: Vec<String> = properties
        .iter()
        .enumerate()
        .map(|(index, property)| {
            let start = property.find(" pid=\"").map(|at| at + 6);
            match start.and_then(|start| Some((start, property[start..].find('"')? + start))) {
                Some((start, end)) => {
                    format!("{}{}{}", &property[..start], index + 2, &property[end..])
                }
                None => property.clone(),
            }
        })
        .collect();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">{}</Properties>"#,
        renumbered.concat()
    );
    opc.replace(CUSTOM, xml.into_bytes());
    if existing.is_none() {
        // The content type and the package relationship for the new part.
        let types = opc
            .required_part_str("[Content_Types].xml")
            .map_err(|error| error.to_string())?;
        if !types.contains("/docProps/custom.xml") {
            let types = types.replace(
                "</Types>",
                &format!(r#"<Override PartName="/{CUSTOM}" ContentType="{CUSTOM_TYPE}"/></Types>"#),
            );
            opc.replace("[Content_Types].xml", types.into_bytes());
        }
        let rels = opc
            .required_part_str("_rels/.rels")
            .map_err(|error| error.to_string())?;
        if !rels.contains(CUSTOM_RELATIONSHIP) {
            let mut id = 1;
            while rels.contains(&format!(r#"Id="rId{id}""#)) {
                id += 1;
            }
            let rels = rels.replace(
                "</Relationships>",
                &format!(
                    r#"<Relationship Id="rId{id}" Type="{CUSTOM_RELATIONSHIP}" Target="{CUSTOM}"/></Relationships>"#
                ),
            );
            opc.replace("_rels/.rels", rels.into_bytes());
        }
    }
    opc.write().map_err(|error| error.to_string())
}

/// What the 文書の保護 dialog edits.
pub struct ProtectForm {
    /// 読み取りパスワード: blank for none.
    pub open_password: TextEdit,
    /// 書き込みパスワード: blank for none.
    pub modify_password: TextEdit,
    pub recommended: bool,
    /// Which box has the keyboard: 0 the open password, 1 the other.
    pub focus: usize,
}

impl ProtectForm {
    pub fn new(open_password: Option<&str>, recommended: bool) -> Self {
        Self {
            open_password: TextEdit::new(open_password.unwrap_or_default().to_string()),
            modify_password: TextEdit::new(String::new()),
            recommended,
            focus: 0,
        }
    }

    pub fn focused(&self) -> &TextEdit {
        if self.focus == 0 {
            &self.open_password
        } else {
            &self.modify_password
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextEdit {
        if self.focus == 0 {
            &mut self.open_password
        } else {
            &mut self.modify_password
        }
    }

    pub fn next_field(&mut self) {
        self.focus = 1 - self.focus;
    }
}

/// A password typed, shown as dots.
pub fn masked(text: &str) -> String {
    "●".repeat(text.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/document.xml", "<w:document/>"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn marking_final_writes_the_custom_property_and_takes_it_back() {
        let plain = package();
        assert!(!is_marked_final(&plain));
        let marked = mark_final(&plain, true).unwrap();
        assert!(is_marked_final(&marked));
        let opc = jade_opc::OpcPackage::open(&marked).unwrap();
        assert!(opc
            .part_str("[Content_Types].xml")
            .unwrap()
            .unwrap()
            .contains(CUSTOM_TYPE));
        assert!(opc
            .part_str("_rels/.rels")
            .unwrap()
            .unwrap()
            .contains(r#"Id="rId2""#));
        let unmarked = mark_final(&marked, false).unwrap();
        assert!(!is_marked_final(&unmarked));
    }

    #[test]
    fn passwords_show_as_dots() {
        assert_eq!(masked("abc"), "●●●");
        let mut salt = [0u8; 16];
        random_bytes(&mut salt);
        assert_ne!(salt, [0u8; 16]);
    }
}
