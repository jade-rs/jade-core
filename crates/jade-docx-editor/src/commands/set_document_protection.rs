//! Word editing-restriction command.
//!
//! Authors/removes `w:documentProtection` in `word/settings.xml`. The command
//! snapshots the exact previous settings part so undo restores imported
//! extension markup and namespace choices byte-for-byte.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::DocumentProtectionInfo;

pub struct SetDocumentProtectionCommand {
    protection: Option<DocumentProtectionInfo>,
    previous_settings_xml: Option<String>,
}

impl SetDocumentProtectionCommand {
    pub fn new(protection: Option<DocumentProtectionInfo>) -> Self {
        Self {
            protection,
            previous_settings_xml: None,
        }
    }
}

impl Command for SetDocumentProtectionCommand {
    fn name(&self) -> &'static str {
        "set_document_protection"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = ctx
            .document
            .set_document_protection(self.protection.clone())
            .map_err(CommandError::Custom)?;
        self.previous_settings_xml = Some(previous);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous_settings_xml
            .take()
            .ok_or_else(|| CommandError::Custom("missing protection settings backup".into()))?;
        ctx.document.restore_settings_xml(previous);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::{DocumentPart, WordDocument};
    use crate::Options;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use futures::executor;
    use proptest::prelude::*;
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn load_fixture() -> WordDocument {
        let data = std::fs::read(fixture_path("hello.docx")).expect("fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load fixture")
    }

    fn settings_xml(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/settings.xml")
            .expect("settings")
            .read_to_string(&mut xml)
            .expect("read settings");
        xml
    }

    fn imported_settings() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:future="urn:future">
  <w:zoom w:percent="115"/>
  <w:documentProtection w:edit="comments" w:enforcement="1" future:keep="yes"/>
  <future:setting future:value="preserve-me"/>
</w:settings>"#
            .to_string()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn protection_credentials_round_trip_and_history_preserve_imported_settings(
            hash_bytes in prop::collection::vec(any::<u8>(), 64),
            salt_bytes in prop::collection::vec(any::<u8>(), 16),
            spin_count in 1u32..=250_000,
        ) {
            let mut word_doc = load_fixture();
            let imported = imported_settings();
            word_doc.parts.insert(
                "word/settings.xml".to_string(),
                DocumentPart {
                    path: "word/settings.xml".to_string(),
                    relationships: Vec::new(),
                    content: imported.clone(),
                    parsed_document: None,
                },
            );
            let expected = DocumentProtectionInfo {
                edit: "readOnly".to_string(),
                enforced: true,
                hash: Some(BASE64.encode(hash_bytes)),
                salt: Some(BASE64.encode(salt_bytes)),
                spin_count: Some(spin_count),
                algorithm_sid: Some(14),
            };
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(SetDocumentProtectionCommand::new(Some(expected.clone()))),
                        &mut ctx,
                    )
                    .expect("protect");
            }
            prop_assert_eq!(word_doc.document_protection(), Some(expected.clone()));
            let protected_xml = settings_xml(&word_doc);
            prop_assert!(protected_xml.contains("future:keep=\"yes\""));
            prop_assert!(protected_xml.contains("future:value=\"preserve-me\""));

            let exported = word_doc.export_docx().expect("export protected");
            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen protected");
            prop_assert_eq!(reopened.document_protection(), Some(expected));

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo"));
            }
            prop_assert_eq!(settings_xml(&word_doc), imported.clone());

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo"));
                executor
                    .execute(
                        Box::new(SetDocumentProtectionCommand::new(None)),
                        &mut ctx,
                    )
                    .expect("remove");
            }
            prop_assert_eq!(word_doc.document_protection(), None);
            let unprotected_xml = settings_xml(&word_doc);
            prop_assert!(unprotected_xml.contains("future:value=\"preserve-me\""));
            prop_assert!(!unprotected_xml.contains("documentProtection"));
        }
    }

    #[test]
    fn invalid_credentials_are_atomic() {
        let mut word_doc = load_fixture();
        let before = settings_xml(&word_doc);
        for (hash, salt) in [
            ("not base64".to_string(), "also not base64".to_string()),
            (BASE64.encode([0u8; 63]), BASE64.encode([0u8; 16])),
            (BASE64.encode([0u8; 64]), BASE64.encode([0u8; 15])),
        ] {
            let result = word_doc.set_document_protection(Some(DocumentProtectionInfo {
                edit: "readOnly".to_string(),
                enforced: true,
                hash: Some(hash),
                salt: Some(salt),
                spin_count: Some(100_000),
                algorithm_sid: Some(14),
            }));
            assert!(result.is_err());
            assert_eq!(settings_xml(&word_doc), before);
        }
    }
}
