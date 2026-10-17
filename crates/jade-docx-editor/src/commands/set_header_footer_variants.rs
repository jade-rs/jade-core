use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

/// Toggles Word's different-first-page and different-odd/even header/footer
/// stories. Package-level snapshots keep newly materialized parts,
/// relationships, content types, settings, and derived renderer state atomic.
pub struct SetHeaderFooterVariantsCommand {
    different_first_page: bool,
    different_odd_even_pages: bool,
    /// Section whose `w:titlePg` the toggle writes; `None` targets the final
    /// section (a caller without a caret, such as the CLI).
    section: Option<usize>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetHeaderFooterVariantsCommand {
    pub fn new(
        different_first_page: bool,
        different_odd_even_pages: bool,
        section: Option<usize>,
    ) -> Self {
        Self {
            different_first_page,
            different_odd_even_pages,
            section,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetHeaderFooterVariantsCommand {
    fn name(&self) -> &'static str {
        "set_header_footer_variants"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx.document.set_header_footer_variants(
            self.different_first_page,
            self.different_odd_even_pages,
            self.section,
        ) {
            Ok(changed) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.changed = changed;
                Ok(())
            }
            Err(error) => {
                // Variant creation touches several OPC parts before the final
                // settings flag is written. Restore all of them if any step
                // fails so a rejected command is truly atomic.
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("missing header/footer variant snapshot for undo".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
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

    fn package_text(bytes: &[u8], path: &str) -> Option<String> {
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
        let mut part = archive.by_name(path).ok()?;
        let mut value = String::new();
        part.read_to_string(&mut value).ok()?;
        Some(value)
    }

    fn package_paths(bytes: &[u8]) -> Vec<String> {
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        (0..archive.len())
            .map(|index| archive.by_index(index).expect("entry").name().to_string())
            .collect()
    }

    fn assert_variant_state(bytes: &[u8], first: bool, odd_even: bool) {
        let document_xml = package_text(bytes, "word/document.xml").expect("document.xml");
        assert_eq!(document_xml.contains("<w:titlePg"), first);
        let reopened = executor::block_on(WordDocument::load(bytes, &Options::default()))
            .expect("reopen package for semantic settings assertion");
        assert_eq!(reopened.even_and_odd_headers, odd_even);
        if first {
            assert!(document_xml.contains("<w:headerReference"));
            assert!(document_xml.contains("<w:footerReference"));
            assert_eq!(document_xml.matches("w:type=\"first\"").count(), 2);
        }
        if odd_even {
            assert_eq!(document_xml.matches("w:type=\"even\"").count(), 2);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_variant_toggles_round_trip_package_and_byte_exact_history(
            first in any::<bool>(),
            odd_even in any::<bool>(),
            text in "[A-Za-z0-9あ-ん一-龠]{1,24}",
        ) {
            let mut document = load_fixture();
            let baseline = document.export_docx().expect("baseline export");
            let original_selection = Some(SelectionRange::collapsed(
                jade_docx_layout::selection::TextPosition::new(0, 1),
            ));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(SetHeaderFooterVariantsCommand::new(first, odd_even, None)),
                    &mut context,
                ).expect("set variants");
            }

            let applied = document.export_docx().expect("applied export");
            assert_variant_state(&applied, first, odd_even);
            prop_assert_eq!(selection, original_selection);
            let paths = package_paths(&applied);
            if first {
                prop_assert!(paths.iter().any(|path| path.starts_with("word/header")));
                prop_assert!(paths.iter().any(|path| path.starts_with("word/footer")));
                prop_assert!(document.header_lines_first.is_some());
                prop_assert!(document.footer_lines_first.is_some());
            }
            if odd_even {
                prop_assert!(document.header_lines_even.is_some());
                prop_assert!(document.footer_lines_even.is_some());
            }

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen variants");
            prop_assert_eq!(reopened.title_page, first);
            prop_assert_eq!(reopened.even_and_odd_headers, odd_even);

            // The command owns the entire package mutation. Its direct undo and
            // redo must restore exact ZIP bytes as well as derived editor state.
            if first || odd_even {
                {
                    let mut context = CommandContext::new(&mut document, &mut selection);
                    prop_assert!(commands.undo(&mut context).expect("undo variants"));
                }
                prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
                prop_assert_eq!(document.title_page, false);
                prop_assert_eq!(document.even_and_odd_headers, false);
                prop_assert_eq!(selection, original_selection);
                {
                    let mut context = CommandContext::new(&mut document, &mut selection);
                    prop_assert!(commands.redo(&mut context).expect("redo variants"));
                }
                prop_assert_eq!(document.export_docx().expect("redo export"), applied);
            } else {
                prop_assert_eq!(&applied, &baseline);
                prop_assert!(!commands.can_undo());
            }

            // Exercise Unicode text through every newly enabled story. These
            // are the same variant-specific methods canvas editing dispatches.
            if first {
                let first_header_text = format!("先頭{text}");
                let first_footer_text = format!("末尾{text}");
                document.replace_header_first_paragraph_text(0, &first_header_text)
                    .expect("edit first header");
                document.replace_footer_first_paragraph_text(0, &first_footer_text)
                    .expect("edit first footer");
            }
            if odd_even {
                let even_header_text = format!("偶数{text}");
                let even_footer_text = format!("偶数末尾{text}");
                document.replace_header_even_paragraph_text(0, &even_header_text)
                    .expect("edit even header");
                document.replace_footer_even_paragraph_text(0, &even_footer_text)
                    .expect("edit even footer");
            }
            let with_text = document.export_docx().expect("variant text export");
            let reopened_text = executor::block_on(WordDocument::load(&with_text, &Options::default()))
                .expect("reopen variant text");
            if first {
                let expected = format!("先頭{text}");
                prop_assert_eq!(
                    reopened_text.header_lines_first.as_ref().and_then(|lines| lines.first()).and_then(|line| line.left.as_deref()),
                    Some(expected.as_str()),
                );
            }

        }
    }

    #[test]
    fn disabling_keeps_variant_parts_and_reenabling_does_not_duplicate_them() {
        let mut document = load_fixture();
        document
            .set_header_footer_variants(true, true, None)
            .expect("enable variants");
        document
            .replace_header_first_paragraph_text(0, "FIRST")
            .expect("first text");
        document
            .replace_header_even_paragraph_text(0, "EVEN")
            .expect("even text");
        let enabled = document.export_docx().expect("enabled export");
        let enabled_paths = package_paths(&enabled);

        document
            .set_header_footer_variants(false, false, None)
            .expect("disable variants");
        let disabled = document.export_docx().expect("disabled export");
        assert_variant_state(&disabled, false, false);
        assert_eq!(package_paths(&disabled), enabled_paths);
        assert!(package_text(&disabled, "word/header1.xml")
            .into_iter()
            .chain(package_text(&disabled, "word/header2.xml"))
            .any(|xml| xml.contains("FIRST") || xml.contains("EVEN")));

        document
            .set_header_footer_variants(true, true, None)
            .expect("re-enable variants");
        let reenabled = document.export_docx().expect("re-enabled export");
        assert_variant_state(&reenabled, true, true);
        assert_eq!(package_paths(&reenabled), enabled_paths);
        assert_eq!(
            package_text(&reenabled, "word/document.xml")
                .expect("document")
                .matches("w:type=\"first\"")
                .count(),
            2,
        );
        assert_eq!(
            package_text(&reenabled, "word/document.xml")
                .expect("document")
                .matches("w:type=\"even\"")
                .count(),
            2,
        );
    }
}
