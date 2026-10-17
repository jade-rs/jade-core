use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

/// Replaces or clears the Word-compatible VML text watermark in every
/// referenced header variant. A package snapshot makes the multi-part edit,
/// selection, and undo/redo history atomic.
pub struct SetWatermarkCommand {
    text: Option<String>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetWatermarkCommand {
    pub fn new(text: Option<String>) -> Self {
        Self {
            text,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetWatermarkCommand {
    fn name(&self) -> &'static str {
        "set_watermark"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx.document.set_watermark_text(self.text.as_deref()) {
            Ok(changed) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.changed = changed;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing watermark snapshot for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("public/fixtures/hello.docx")
    }

    fn load_fixture() -> WordDocument {
        let bytes = std::fs::read(fixture_path()).expect("read fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load fixture")
    }

    fn header_xmls(bytes: &[u8]) -> Vec<(String, String)> {
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let names = (0..archive.len())
            .filter_map(|index| {
                let name = archive.by_index(index).ok()?.name().to_string();
                (name.starts_with("word/header") && name.ends_with(".xml")).then_some(name)
            })
            .collect::<Vec<_>>();
        names
            .into_iter()
            .map(|name| {
                let mut value = String::new();
                archive
                    .by_name(&name)
                    .expect("header part")
                    .read_to_string(&mut value)
                    .expect("read header part");
                (name, value)
            })
            .collect()
    }

    fn watermark_text_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('A'),
                Just('z'),
                Just(' '),
                Just('&'),
                Just('<'),
                Just('>'),
                Just('"'),
                Just('\''),
                Just('透'),
                Just('か'),
                Just('し'),
                Just('界'),
                Just('é'),
                Just('Ж'),
                Just('🙂'),
            ],
            0..48,
        )
        .prop_map(|characters| format!("WM{}END", characters.into_iter().collect::<String>()))
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_unicode_watermarks_round_trip_and_restore_exact_history(
            text in watermark_text_strategy(),
        ) {
            let mut document = load_fixture();
            let baseline = document.export_docx().expect("baseline export");
            let original_selection = Some(SelectionRange::collapsed(TextPosition::new(0, 1)));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(SetWatermarkCommand::new(Some(text.clone()))),
                        &mut context,
                    )
                    .expect("set watermark");
            }
            let applied = document.export_docx().expect("applied export");
            prop_assert_ne!(&applied, &baseline);
            prop_assert_eq!(selection, original_selection);

            let headers = header_xmls(&applied);
            prop_assert_eq!(headers.len(), 1);
            let header_xml = &headers[0].1;
            prop_assert!(header_xml.contains("xmlns:v=\"urn:schemas-microsoft-com:vml\""));
            prop_assert!(header_xml.contains("PowerPlusWaterMarkObject1"));
            prop_assert_eq!(header_xml.matches("string=\"").count(), 1);

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen watermark");
            prop_assert_eq!(
                reopened
                    .document
                    .as_deref()
                    .and_then(|document| document.watermark_text.as_deref()),
                Some(text.as_str()),
            );

            // Reapplying the same normalized text must not rewrite the package
            // or create a history entry.
            let mut no_op = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                no_op
                    .execute(
                        Box::new(SetWatermarkCommand::new(Some(text))),
                        &mut context,
                    )
                    .expect("repeat watermark");
            }
            prop_assert!(!no_op.can_undo());
            prop_assert_eq!(document.export_docx().expect("no-op export"), applied.clone());

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo watermark"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, original_selection);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo watermark"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn watermark_preserves_header_content_and_propagates_to_new_variants() {
        let mut document = load_fixture();
        document
            .replace_header_paragraph_text(0, "KEEP HEADER")
            .expect("seed header");
        let default_path = document
            .parts
            .keys()
            .find(|path| path.starts_with("word/header") && path.ends_with(".xml"))
            .cloned()
            .expect("default header path");
        let part = document.parts.get_mut(&default_path).expect("header part");
        part.content = part
            .content
            .replace(
                "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"",
                "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:x=\"urn:jade:test\"",
            )
            .replace("</w:hdr>", "<x:keep x:value=\"yes\"/></w:hdr>");

        assert!(document
            .set_watermark_text(Some("社外秘 & <draft> \"A\""))
            .expect("set watermark"));
        document
            .set_header_footer_variants(true, true, None)
            .expect("enable variants");

        // Imported packages can disagree across default/first/even headers.
        // Reapplying the visible value must repair every referenced variant,
        // rather than treating only the normalized model value as a no-op.
        let divergent_path = document
            .parts
            .keys()
            .find(|path| {
                path.starts_with("word/header") && path.ends_with(".xml") && *path != &default_path
            })
            .cloned()
            .expect("variant header path");
        let divergent = document
            .parts
            .get_mut(&divergent_path)
            .expect("variant header part");
        divergent.content = divergent.content.replace(
            "string=\"社外秘 &amp; &lt;draft> &quot;A&quot;\"",
            "string=\"OTHER\"",
        );
        assert!(divergent.content.contains("string=\"OTHER\""));
        assert!(document
            .set_watermark_text(Some("社外秘 & <draft> \"A\""))
            .expect("repair divergent variants"));

        let authored = document.export_docx().expect("export variants");
        let headers = header_xmls(&authored);
        assert_eq!(headers.len(), 3);
        for (_, xml) in &headers {
            assert_eq!(xml.matches("PowerPlusWaterMarkObject1").count(), 1);
            assert!(
                xml.contains("string=\"社外秘 &amp; &lt;draft> &quot;A&quot;\""),
                "unexpected watermark XML: {xml}"
            );
        }
        let default_xml = headers
            .iter()
            .find(|(path, _)| path == &default_path)
            .map(|(_, xml)| xml)
            .expect("default header xml");
        assert!(default_xml.contains("KEEP HEADER"));
        assert!(default_xml.contains("<x:keep x:value=\"yes\"/>"));

        assert!(document.set_watermark_text(None).expect("clear watermark"));
        let cleared = document.export_docx().expect("export cleared");
        for (_, xml) in header_xmls(&cleared) {
            assert!(!xml.contains("PowerPlusWaterMarkObject1"));
            assert!(!xml.contains("v:textpath"));
        }
        let default_cleared = header_xmls(&cleared)
            .into_iter()
            .find(|(path, _)| path == &default_path)
            .map(|(_, xml)| xml)
            .expect("cleared default header");
        assert!(default_cleared.contains("KEEP HEADER"));
        assert!(default_cleared.contains("<x:keep x:value=\"yes\"/>"));
    }

    #[test]
    fn invalid_watermark_text_is_atomic() {
        let mut document = load_fixture();
        let baseline = document.export_docx().expect("baseline");
        let invalid = [
            String::new(),
            " ".into(),
            "X".repeat(129),
            "bad\0value".into(),
        ];
        for value in invalid {
            let before = document.export_docx().expect("before invalid");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
            let mut commands = CommandExecutor::new();
            let error = {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(SetWatermarkCommand::new(Some(value))),
                        &mut context,
                    )
                    .expect_err("reject invalid watermark")
            };
            assert!(error.to_string().contains("watermark"));
            assert_eq!(document.export_docx().expect("after invalid"), before);
            assert!(!commands.can_undo());
        }
        assert_eq!(document.export_docx().expect("final"), baseline);
    }

    #[test]
    fn clearing_pretty_printed_watermark_drops_whitespace_only_paragraph() {
        let mut document = load_fixture();
        assert!(document
            .set_watermark_text(Some("DRAFT"))
            .expect("set watermark"));

        for (path, part) in &mut document.parts {
            if path.starts_with("word/header") && path.ends_with(".xml") {
                part.content = part
                    .content
                    .replace("<w:r/>", "")
                    .replace("<w:r>", "\n    <w:r>")
                    .replace("</w:r>", "</w:r>\n  ");
            }
        }

        assert!(document.set_watermark_text(None).expect("clear watermark"));
        let cleared = document.export_docx().expect("export cleared");
        for (_, xml) in header_xmls(&cleared) {
            assert!(!xml.contains("v:textpath"));
            assert!(
                !xml.contains("<w:p"),
                "whitespace-only watermark paragraph remained: {xml}"
            );
        }
    }
}
