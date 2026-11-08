//! The recording switch is a document setting, separate from revision authors.
use super::*;

impl WordDocument {
    /// An enforced tracked-changes restriction requires recording throughout editing.
    pub fn tracking_locked(&self) -> bool {
        !self.protection_lifted
            && self.editing_restriction().is_some_and(|restriction| {
                restriction.enforced && restriction.kind == Some(EditingKind::TrackedChanges)
            })
    }

    pub(super) fn tracking_settings_path(&self) -> String {
        self.tracking_settings_path_ref().into_owned()
    }

    pub(super) fn tracking_settings_path_ref(&self) -> std::borrow::Cow<'_, str> {
        let Some(part) = self.document_part.as_ref() else {
            return std::borrow::Cow::Borrowed("word/settings.xml");
        };
        let target = part
            .relationships
            .iter()
            .find(|rel| {
                // Relationship types share a long namespace prefix. Reject
                // other kinds by their suffix before comparing the full URI.
                rel.r#type.ends_with("/settings")
                    && rel.r#type == RelationshipTypes::Settings.as_str()
            })
            .map(|rel| rel.target.as_str())
            .unwrap_or("settings.xml");
        if target == "settings.xml" && part.path == "word/document.xml" {
            std::borrow::Cow::Borrowed("word/settings.xml")
        } else if target.starts_with('/') {
            std::borrow::Cow::Borrowed(target.trim_start_matches('/'))
        } else {
            std::borrow::Cow::Owned(Self::resolve_target_path(&part.path, target))
        }
    }

    pub(super) fn saved_change_tracking_enabled(&self) -> bool {
        let Some(xml) = self.read_xml_part_content(&self.tracking_settings_path()) else {
            return false;
        };
        // Only the direct setting is needed; avoid building a lossless XML
        // tree for every document load just to read this recording switch.
        let mut reader = quick_xml::Reader::from_str(&xml);
        let mut depth = 0usize;
        let mut enabled = false;
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(element)) => {
                    if depth == 1 && element.local_name().as_ref() == b"trackRevisions" {
                        enabled |= Self::tracking_setting_enabled(&element);
                    }
                    depth += 1;
                }
                Ok(quick_xml::events::Event::Empty(element)) => {
                    if depth == 1 && element.local_name().as_ref() == b"trackRevisions" {
                        enabled |= Self::tracking_setting_enabled(&element);
                    }
                }
                Ok(quick_xml::events::Event::End(_)) => depth = depth.saturating_sub(1),
                Ok(quick_xml::events::Event::Eof) => return enabled,
                Err(_) => return false,
                _ => {}
            }
        }
    }

    fn tracking_setting_enabled(element: &quick_xml::events::BytesStart<'_>) -> bool {
        !element.attributes().flatten().any(|attr| {
            attr.key.local_name().as_ref() == b"val"
                && attr
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .is_ok_and(|value| matches!(value.as_ref(), "0" | "false" | "off"))
        })
    }

    /// Persist a user recording command. Internal temporary suspension still
    /// uses `set_revision_session`, so it does not rewrite the document switch.
    pub(crate) fn set_saved_revision_session(
        &mut self,
        session: Option<RevisionSession>,
    ) -> Result<(), String> {
        if session.is_none() && self.tracking_locked() {
            return Err("tracking is locked".into());
        }
        // Synthetic semantic documents have no OPC package to write.
        if self.package.is_none() {
            self.set_revision_session(session);
            return Ok(());
        }
        let path = self.tracking_settings_path();
        let existing = self.read_xml_part_content(&path);
        if existing.is_none() && session.is_none() {
            self.set_revision_session(None);
            return Ok(());
        }
        let xml = existing.unwrap_or_else(|| {
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.into()
        });
        let mut doc = DocxXmlDocument::parse(&xml).map_err(|error| error.to_string())?;
        let prefix = doc
            .root()
            .name
            .split_once(':')
            .map(|(prefix, _)| format!("{prefix}:"))
            .unwrap_or_default();
        let new_element = session
            .as_ref()
            .map(|_| doc.create_element(format!("{prefix}trackRevisions"), Vec::new(), Vec::new()));
        let root_id = doc.root().id;
        let root = doc.element_mut(root_id).ok_or("settings root is missing")?;
        root.children.retain(|child| {
            !matches!(child, XmlNode::Element(element)
            if Self::xml_local_name(&element.name) == "trackRevisions")
        });
        if let Some(element) = new_element {
            root.children.push(XmlNode::Element(element));
        }
        doc.rebuild_index();
        let content = doc.to_string().map_err(|error| error.to_string())?;
        self.write_tracking_settings_xml(&path, content)?;
        self.set_revision_session(session);
        Ok(())
    }

    /// Registers and writes a settings part atomically, including its OPC metadata.
    pub(super) fn write_tracking_settings_xml(
        &mut self,
        path: &str,
        content: String,
    ) -> Result<(), String> {
        // Semantic-only documents have no OPC relationships or content types.
        if self.package.is_none() {
            self.upsert_xml_part_content(path, content);
            return Ok(());
        }
        let previous = self.package_state();
        let result = (|| {
            let part = self
                .document_part
                .as_mut()
                .ok_or("main document part is unavailable")?;
            let relationship = if let Some(rel) = part
                .relationships
                .iter()
                .find(|rel| rel.r#type == RelationshipTypes::Settings.as_str())
            {
                rel.clone()
            } else {
                let rel = Relationship {
                    id: Self::next_relationship_id(&part.relationships),
                    r#type: RelationshipTypes::Settings.as_str().into(),
                    target: "settings.xml".into(),
                    target_mode: None,
                };
                part.relationships.push(rel.clone());
                rel
            };
            self.ensure_document_relationships_part_has_relationship(&relationship)?;
            self.ensure_content_types_has_override(
                path,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
            )?;
            self.upsert_xml_part_content(path, content);
            Ok(())
        })();
        if result.is_err() {
            self.restore_package_state(previous);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::headless_api::apply_headless_commands_with_executor_to_document;
    use futures::executor::block_on;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
    use std::io::Write;

    fn load(bytes: &[u8]) -> WordDocument {
        block_on(WordDocument::load(bytes, &Options::default())).unwrap()
    }

    fn fixture(settings: Option<&str>, alternate_path: bool) -> WordDocument {
        let target = if alternate_path {
            "config/preferences.xml"
        } else {
            "settings.xml"
        };
        let mut parts = vec![
            ("[Content_Types].xml".into(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.into()),
            ("_rels/.rels".into(), r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.into()),
            ("word/document.xml".into(), r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#.into()),
        ];
        if let Some(settings) = settings {
            parts.push((format!("word/{target}"), settings.to_string()));
            parts.push(("word/_rels/document.xml.rels".into(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="{target}"/></Relationships>"#)));
        }
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, content) in parts {
            zip.start_file::<_, ()>(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        load(&zip.finish().unwrap().into_inner())
    }

    fn apply(doc: &mut WordDocument, executor: &mut CommandExecutor, command: HeadlessCommand) {
        apply_headless_commands_with_executor_to_document(
            doc,
            executor,
            None,
            &HeadlessEditRequest {
                commands: vec![command],
            },
        )
        .unwrap();
    }

    #[test]
    fn settings_path_observes_relationship_changes_and_exact_types() {
        let mut doc = fixture(
            Some(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
            ),
            false,
        );
        assert_eq!(doc.tracking_settings_path_ref(), "word/settings.xml");
        let mut other = doc.document_part.as_ref().unwrap().relationships[0].clone();
        other.r#type = "https://example.invalid/settings".into();
        other.target = "wrong.xml".into();
        doc.document_part
            .as_mut()
            .unwrap()
            .relationships
            .insert(0, other);
        assert_eq!(doc.tracking_settings_path_ref(), "word/settings.xml");
        doc.document_part.as_mut().unwrap().relationships[1].target = "/config/first.xml".into();
        assert_eq!(doc.tracking_settings_path_ref(), "config/first.xml");
        doc.document_part.as_mut().unwrap().relationships[1].target = "../config/next.xml".into();
        assert_eq!(doc.tracking_settings_path_ref(), "config/next.xml");
        doc.document_part.as_mut().unwrap().relationships.clear();
        assert_eq!(doc.tracking_settings_path_ref(), "word/settings.xml");
        doc.document_part.as_mut().unwrap().path = "custom/document.xml".into();
        assert_eq!(doc.tracking_settings_path_ref(), "custom/settings.xml");
    }

    #[test]
    fn protection_creates_settings_and_package_metadata_with_exact_history() {
        use jade_docx_headless_api::HeadlessDocumentProtection;
        for edit in ["readOnly", "comments", "trackedChanges", "forms", "none"] {
            let mut doc = fixture(None, false);
            let original = doc.export_docx().unwrap();
            let mut executor = CommandExecutor::new();
            apply(
                &mut doc,
                &mut executor,
                HeadlessCommand::SetDocumentProtection { protection: None },
            );
            assert_eq!(doc.export_docx().unwrap(), original);
            apply(
                &mut doc,
                &mut executor,
                HeadlessCommand::SetDocumentProtection {
                    protection: Some(HeadlessDocumentProtection {
                        edit: edit.into(),
                        enforced: true,
                        formatting: Some(true),
                        hash: None,
                        salt: None,
                        spin_count: None,
                        algorithm_sid: None,
                        algorithm_name: None,
                    }),
                },
            );
            let saved = doc.export_docx().unwrap();
            let reopened = load(&saved);
            assert_eq!(reopened.document_protection().unwrap().edit, edit);
            assert_eq!(reopened.tracking_locked(), edit == "trackedChanges");
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
            use std::io::Read;
            for (path, expected) in [
                ("word/_rels/document.xml.rels", "relationships/settings"),
                ("[Content_Types].xml", "wordprocessingml.settings+xml"),
            ] {
                let mut xml = String::new();
                zip.by_name(path).unwrap().read_to_string(&mut xml).unwrap();
                assert!(xml.contains(expected), "{path}: {xml}");
            }
            let mut selection = None;
            assert!(executor
                .undo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert_eq!(doc.export_docx().unwrap(), original);
            assert!(doc.settings_xml_content().is_none());
            assert!(doc.revision_session().is_none());
            assert!(executor
                .redo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert_eq!(doc.export_docx().unwrap(), saved);
        }
    }

    #[test]
    fn interactive_protection_creates_missing_settings_and_preserves_both_restrictions() {
        use super::super::protection::WriteProtection;
        for kind in [
            EditingKind::ReadOnly,
            EditingKind::Comments,
            EditingKind::TrackedChanges,
            EditingKind::Forms,
        ] {
            let mut doc = fixture(None, false);
            let original = doc.export_docx().unwrap();
            doc.set_editing_restriction(None).unwrap();
            doc.set_write_protection(None).unwrap();
            assert_eq!(doc.export_docx().unwrap(), original);
            let restriction = EditingRestriction {
                kind: Some(kind),
                formatting: true,
                enforced: true,
                password: None,
            };
            let write = WriteProtection {
                recommended: true,
                password: None,
            };
            if kind == EditingKind::ReadOnly {
                doc.set_write_protection(Some(&write)).unwrap();
            }
            doc.set_editing_restriction(Some(&restriction)).unwrap();
            doc.set_write_protection(Some(&write)).unwrap();
            let reopened = load(&doc.export_docx().unwrap());
            assert_eq!(reopened.editing_restriction(), Some(restriction));
            assert_eq!(reopened.write_protection(), Some(write));
            assert_eq!(
                reopened.tracking_locked(),
                kind == EditingKind::TrackedChanges
            );
            assert_eq!(
                reopened.revision_session().is_some(),
                kind == EditingKind::TrackedChanges
            );
        }
    }

    #[test]
    fn settings_registration_failure_rolls_back_package_and_session() {
        for tracking in [false, true] {
            let mut doc = fixture(None, false);
            doc.upsert_xml_part_content("[Content_Types].xml", "<broken".into());
            let previous = doc.package_state();
            let result = if tracking {
                doc.set_saved_revision_session(Some(RevisionSession {
                    author: Some("Reviewer".into()),
                    date: None,
                    date_utc: None,
                }))
            } else {
                doc.set_document_protection(Some(DocumentProtectionInfo {
                    edit: "trackedChanges".into(),
                    enforced: true,
                    formatting: None,
                    hash: None,
                    salt: None,
                    spin_count: None,
                    algorithm_sid: None,
                    algorithm_name: None,
                }))
                .map(|_| ())
            };
            assert!(result.is_err());
            assert_eq!(doc.parts.len(), previous.parts.len());
            for (path, part) in previous.parts {
                assert_eq!(doc.parts.get(&path).unwrap().content, part.content);
            }
            assert_eq!(
                doc.document_part.as_ref().unwrap().relationships.len(),
                previous.document_part.unwrap().relationships.len()
            );
            assert!(doc.settings_xml_content().is_none());
            assert!(doc.revision_session().is_none());
        }
    }

    #[test]
    fn protection_api_modes_formatting_and_history_share_the_editing_state() {
        use jade_docx_headless_api::HeadlessDocumentProtection;
        for (edit, formatting) in [
            ("readOnly", None),
            ("comments", Some(false)),
            ("trackedChanges", Some(true)),
            ("forms", None),
            ("none", Some(true)),
            ("none", None),
        ] {
            for enforced in [false, true] {
                let original = r#"<x:settings xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:trackRevisions x:val="off"/><x:zoom x:percent="125"/></x:settings>"#;
                let mut doc = fixture(Some(original), true);
                let original = doc.settings_xml_content().unwrap();
                let protection = HeadlessDocumentProtection {
                    edit: edit.into(),
                    enforced,
                    formatting,
                    hash: None,
                    salt: None,
                    spin_count: None,
                    algorithm_sid: None,
                    algorithm_name: None,
                };
                let expected: DocumentProtectionInfo =
                    serde_json::from_value(serde_json::to_value(&protection).unwrap()).unwrap();
                let locked = enforced && edit == "trackedChanges";
                let mut executor = CommandExecutor::new();
                apply(
                    &mut doc,
                    &mut executor,
                    HeadlessCommand::SetDocumentProtection {
                        protection: Some(protection),
                    },
                );
                assert_eq!(doc.document_protection(), Some(expected.clone()));
                assert_eq!(doc.tracking_locked(), locked);
                assert_eq!(doc.revision_session().is_some(), locked);
                assert_eq!(
                    doc.restriction_allows("insert_text", None).is_ok(),
                    !enforced || matches!(edit, "none" | "trackedChanges")
                );
                if enforced && formatting == Some(true) {
                    assert!(doc.restriction_allows("apply_bold", None).is_err());
                }
                if enforced && edit == "comments" {
                    assert!(doc.restriction_allows("add_comment", None).is_ok());
                }
                let settings = doc.settings_xml_content().unwrap();
                let mut reopened = load(&doc.export_docx().unwrap());
                assert_eq!(reopened.document_protection(), Some(expected));
                assert_eq!(reopened.tracking_locked(), locked);
                if locked {
                    apply(
                        &mut reopened,
                        &mut CommandExecutor::new(),
                        HeadlessCommand::InsertText {
                            paragraph: 0,
                            offset: 0,
                            text: "Recorded".into(),
                        },
                    );
                    assert!(reopened
                        .paragraph(0)
                        .unwrap()
                        .runs
                        .iter()
                        .any(|run| run.revision.is_some()));
                    assert!(reopened.set_saved_revision_session(None).is_err());
                }
                let mut selection = None;
                assert!(executor
                    .undo(&mut CommandContext::new(&mut doc, &mut selection))
                    .unwrap());
                assert_eq!(doc.settings_xml_content().unwrap(), original);
                assert!(!doc.tracking_locked());
                assert!(doc.revision_session().is_none());
                assert!(executor
                    .redo(&mut CommandContext::new(&mut doc, &mut selection))
                    .unwrap());
                assert_eq!(doc.settings_xml_content().unwrap(), settings);
                assert_eq!(doc.revision_session().is_some(), locked);
                apply(
                    &mut doc,
                    &mut executor,
                    HeadlessCommand::SetDocumentProtection { protection: None },
                );
                assert!(doc.document_protection().is_none());
                assert!(!doc.tracking_locked());
                // Lifting protection keeps the existing recording session.
                assert_eq!(doc.revision_session().is_some(), locked);
            }
        }
    }

    #[test]
    fn formatting_only_imports_and_invalid_protection_modes() {
        let mut doc = fixture(
            Some(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:documentProtection w:formatting="1" w:enforcement="1"/></w:settings>"#,
            ),
            false,
        );
        let protection = doc.document_protection().unwrap();
        assert_eq!(protection.edit, "none");
        assert_eq!(protection.formatting, Some(true));
        assert!(doc.restriction_allows("apply_bold", None).is_err());
        let before = doc.export_docx().unwrap();
        for edit in ["bogus", ""] {
            let mut invalid = protection.clone();
            invalid.edit = edit.into();
            invalid.formatting = None;
            assert!(doc.set_document_protection(Some(invalid)).is_err());
            assert_eq!(doc.export_docx().unwrap(), before);
        }
    }

    #[test]
    fn protection_api_preserves_algorithms_forms_and_alternate_settings_history() {
        use crate::headless_api::execute_headless_queries_on_document;
        use jade_docx_headless_api::{
            HeadlessDocumentProtection, HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult,
        };
        for (algorithm, sid, hash) in [
            ("SHA-1", 4, "2BGoHn7VxIl6R998wai+DvGs95o="),
            ("SHA-256", 12, "AEbC0IVM3fPhq+u+0BmUQ2Dkr7+7ZUe+Qe995ZlecyE="),
            ("SHA-384", 13, "4G1uhh3KDpbu+gAoqm5AEV7RD7lLsYuKyQ9lOBn8mSyNW6aAgA4KvTwgQTNQN1Qs"),
            ("SHA-512", 14, "tsizw+IXGC/j/xv06GsA7ZT5UNaU04jZXgEGGH8epcmupy9GttK1YLyPCJPzOaFu0U5fIWeojfNyASVxv+qpzw=="),
        ] {
            for legacy in [false, true] {
                let original = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:ext="urn:example"><w:zoom w:percent="125"/><w:documentProtection w:edit="comments" w:enforcement="1" w:algorithmName="SHA-512" w:hashValue="old" w:saltValue="old" w:spinCount="5" w:cryptAlgorithmSid="14" w:hash="old" w:salt="old" w:cryptSpinCount="5" ext:keep="yes"/></w:settings>"#;
                let mut doc = fixture(Some(original), true);
                let expected = HeadlessDocumentProtection {
                    edit: "readOnly".into(), enforced: true,
                    hash: Some(hash.into()), salt: Some("MDEyMzQ1Njc4OWFiY2RlZg==".into()),
                    spin_count: Some(23),
                    algorithm_sid: legacy.then_some(sid),
                    algorithm_name: (!legacy).then(|| algorithm.into()),
                    formatting: None,
                };
                // Exercise the serialized API schema as well as its command/query conversion.
                let expected: HeadlessDocumentProtection = serde_json::from_value(
                    serde_json::to_value(expected).unwrap()).unwrap();
                let mut executor = CommandExecutor::new();
                apply(&mut doc, &mut executor, HeadlessCommand::SetDocumentProtection {
                    protection: Some(expected.clone()),
                });
                let query = |doc: &WordDocument| {
                    execute_headless_queries_on_document(doc, &HeadlessQueryRequest {
                        queries: vec![HeadlessQuery::DocumentProtection],
                    }).unwrap().results
                };
                assert_eq!(query(&doc), vec![HeadlessQueryResult::DocumentProtection {
                    protection: Some(expected.clone()),
                }]);
                let settings = doc.settings_xml_content().unwrap();
                assert!(settings.contains("ext:keep=\"yes\""));
                assert!(!settings.contains("old"));
                assert_eq!(settings.contains("hashValue="), !legacy);
                assert_eq!(settings.contains("cryptAlgorithmSid="), legacy);
                assert!(!doc.parts.contains_key("word/settings.xml"));
                assert_eq!(doc.editing_restriction().unwrap().password.unwrap()
                    .verifies("秘密🔑"), Some(true));
                let reopened = load(&doc.export_docx().unwrap());
                assert_eq!(query(&reopened), query(&doc));
                let mut selection = None;
                assert!(executor.undo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap());
                assert_eq!(doc.settings_xml_content().unwrap(), original);
                assert!(executor.redo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap());
                assert_eq!(doc.settings_xml_content().unwrap(), settings);
            }
        }
    }

    #[test]
    fn locked_recording_survives_reopening_and_refuses_stop() {
        for (recording, alternate_path) in
            [("", false), (r#"<w:trackRevisions w:val="off"/>"#, true)]
        {
            let xml = format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{recording}<w:documentProtection w:edit="trackedChanges" w:enforcement="1"/></w:settings>"#
            );
            let mut doc = fixture(Some(&xml), alternate_path);
            assert!(doc.tracking_locked());
            assert!(doc.revision_session().is_some());
            let before = doc.export_docx().unwrap();
            let mut executor = CommandExecutor::new();
            let error = apply_headless_commands_with_executor_to_document(
                &mut doc,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::StopChangeTracking],
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("tracking is locked"));
            assert_eq!(doc.export_docx().unwrap(), before);
            assert!(doc.revision_session().is_some());
            assert!(doc.set_saved_revision_session(None).is_err());
            for name in ["accept_revisions", "reject_revisions"] {
                assert!(doc.restriction_allows(name, None).is_err());
            }
            let mut reopened = load(&before);
            apply(
                &mut reopened,
                &mut executor,
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "Locked".into(),
                },
            );
            assert!(reopened
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .any(|run| run.revision.is_some()));
        }
    }

    #[test]
    fn starting_recording_lock_and_history_restore_the_session_and_package() {
        use crate::commands::snapshot_edit::{EditScope, SnapshotEditCommand};
        let mut doc = fixture(
            Some(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
            ),
            false,
        );
        let before = doc.export_docx().unwrap();
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        let restriction = EditingRestriction {
            kind: Some(EditingKind::TrackedChanges),
            formatting: false,
            enforced: true,
            password: None,
        };
        executor
            .execute(
                Box::new(SnapshotEditCommand::with_scope(
                    "set_editing_restriction",
                    EditScope::Paragraphs(None),
                    move |document| document.set_editing_restriction(Some(&restriction)),
                )),
                &mut CommandContext::new(&mut doc, &mut selection),
            )
            .unwrap();
        assert!(doc.tracking_locked());
        assert!(doc.saved_change_tracking_enabled());
        assert!(load(&doc.export_docx().unwrap())
            .revision_session()
            .is_some());
        executor
            .undo(&mut CommandContext::new(&mut doc, &mut selection))
            .unwrap();
        assert!(!doc.tracking_locked());
        assert!(doc.revision_session().is_none());
        assert_eq!(doc.export_docx().unwrap(), before);
        executor
            .redo(&mut CommandContext::new(&mut doc, &mut selection))
            .unwrap();
        assert!(doc.tracking_locked());
        assert!(doc.revision_session().is_some());
        let stopped = EditingRestriction {
            kind: Some(EditingKind::TrackedChanges),
            formatting: false,
            enforced: false,
            password: None,
        };
        doc.set_editing_restriction(Some(&stopped)).unwrap();
        apply(&mut doc, &mut executor, HeadlessCommand::StopChangeTracking);
        assert!(load(&doc.export_docx().unwrap())
            .revision_session()
            .is_none());
    }

    #[test]
    fn recording_setting_loads_all_on_off_forms_and_alternate_prefix() {
        for (value, expected) in [
            ("", true),
            (" x:val=\"1\"", true),
            (" x:val=\"true\"", true),
            (" x:val=\"on\"", true),
            (" x:val=\"0\"", false),
            (" x:val=\"false\"", false),
            (" x:val=\"off\"", false),
        ] {
            let xml = format!(
                r#"<x:settings xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:trackRevisions{value}/><x:zoom x:percent="125"/></x:settings>"#
            );
            let doc = fixture(Some(&xml), true);
            assert_eq!(doc.revision_session().is_some(), expected, "{value}");
            assert_eq!(
                load(&doc.export_docx().unwrap())
                    .revision_session()
                    .is_some(),
                expected
            );
        }
    }

    #[test]
    fn recording_commands_persist_create_settings_and_undo_together() {
        for settings in [
            None,
            Some(
                r#"<x:settings xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:trackRevisions x:val="off"/><x:zoom x:percent="125"/></x:settings>"#,
            ),
        ] {
            let mut doc = fixture(settings, settings.is_some());
            let original = doc.export_docx().unwrap();
            let mut executor = CommandExecutor::new();
            apply(
                &mut doc,
                &mut executor,
                HeadlessCommand::StartChangeTracking {
                    author: Some("Reviewer".into()),
                    initials: None,
                    date: None,
                    date_utc: None,
                },
            );
            let enabled = doc.export_docx().unwrap();
            let mut reopened = load(&enabled);
            assert!(reopened.revision_session().is_some());
            apply(
                &mut reopened,
                &mut CommandExecutor::new(),
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "Tracked".into(),
                },
            );
            assert!(reopened
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .any(|run| run.revision.is_some()));
            let mut selection = None;
            assert!(executor
                .undo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert!(doc.revision_session().is_none());
            assert_eq!(doc.export_docx().unwrap(), original);
            assert!(executor
                .redo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert_eq!(doc.export_docx().unwrap(), enabled);
            apply(&mut doc, &mut executor, HeadlessCommand::StopChangeTracking);
            let stopped = doc.export_docx().unwrap();
            let mut reopened = load(&stopped);
            assert!(reopened.revision_session().is_none());
            apply(
                &mut reopened,
                &mut CommandExecutor::new(),
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "Plain".into(),
                },
            );
            assert!(reopened
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .all(|run| run.revision.is_none()));
            if settings.is_some() {
                assert!(doc
                    .read_xml_part_content(&doc.tracking_settings_path())
                    .unwrap()
                    .contains("125"));
            }
            assert!(executor
                .undo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert!(doc.revision_session().is_some());
            assert_eq!(doc.export_docx().unwrap(), enabled);
            assert!(executor
                .redo(&mut CommandContext::new(&mut doc, &mut selection))
                .unwrap());
            assert_eq!(doc.export_docx().unwrap(), stopped);
        }
    }
}
