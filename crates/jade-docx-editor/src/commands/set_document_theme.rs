use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{DocumentThemeSpec, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

/// Replaces the package-wide Word theme while keeping document coordinates,
/// package parts, and undo/redo atomic.
pub struct SetDocumentThemeCommand {
    spec: DocumentThemeSpec,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetDocumentThemeCommand {
    pub fn new(spec: DocumentThemeSpec) -> Self {
        Self {
            spec,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetDocumentThemeCommand {
    fn name(&self) -> &'static str {
        "set_document_theme"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx.document.set_document_theme(&self.spec) {
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
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("missing document-theme snapshot for undo".into())
        })?;
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

    fn load_fixture() -> WordDocument {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("public/fixtures/hello.docx");
        let bytes = std::fs::read(path).expect("read fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load fixture")
    }

    fn theme_xml(bytes: &[u8]) -> String {
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive
            .by_name("word/theme/theme1.xml")
            .expect("theme part");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read theme");
        value
    }

    fn text_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec![
                'A', 'z', '0', ' ', '&', '<', '>', '"', '\'', 'あ', '漢', 'é', 'Ω',
            ]),
            1..24,
        )
        .prop_map(|chars| chars.into_iter().collect::<String>())
        .prop_filter("non-whitespace", |value| !value.trim().is_empty())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_theme_packages_round_trip_and_restore_exact_history(
            name in text_strategy(),
            major_font in text_strategy(),
            minor_font in text_strategy(),
            east_asia_font in text_strategy(),
            colors in prop::array::uniform8(0u32..=0xFF_FFFF),
        ) {
            let spec = DocumentThemeSpec {
                name: name.trim().to_string(),
                major_font: major_font.trim().to_string(),
                minor_font: minor_font.trim().to_string(),
                east_asia_font: east_asia_font.trim().to_string(),
                dark2: format!("#{:06X}", colors[0]),
                light2: format!("{:06x}", colors[1]),
                accent1: format!("#{:06X}", colors[2]),
                accent2: format!("{:06x}", colors[3]),
                accent3: format!("#{:06X}", colors[4]),
                accent4: format!("{:06x}", colors[5]),
                accent5: format!("#{:06X}", colors[6]),
                accent6: format!("{:06x}", colors[7]),
            };
            let expected = DocumentThemeSpec {
                dark2: spec.dark2.trim_start_matches('#').to_ascii_uppercase(),
                light2: spec.light2.trim_start_matches('#').to_ascii_uppercase(),
                accent1: spec.accent1.trim_start_matches('#').to_ascii_uppercase(),
                accent2: spec.accent2.trim_start_matches('#').to_ascii_uppercase(),
                accent3: spec.accent3.trim_start_matches('#').to_ascii_uppercase(),
                accent4: spec.accent4.trim_start_matches('#').to_ascii_uppercase(),
                accent5: spec.accent5.trim_start_matches('#').to_ascii_uppercase(),
                accent6: spec.accent6.trim_start_matches('#').to_ascii_uppercase(),
                ..spec.clone()
            };
            let mut document = load_fixture();
            let baseline = document.export_docx().expect("baseline export");
            let baseline_theme = theme_xml(&baseline);
            let original_selection = Some(SelectionRange::collapsed(TextPosition::new(0, 1)));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(SetDocumentThemeCommand::new(spec.clone())),
                    &mut context,
                ).expect("set theme");
            }
            let applied = document.export_docx().expect("applied export");
            prop_assert_ne!(&applied, &baseline);
            prop_assert_eq!(selection, original_selection);
            prop_assert_eq!(document.document_theme(), Some(expected.clone()));
            let xml = theme_xml(&applied);
            prop_assert!(xml.contains("<a:fmtScheme"), "unmodeled format scheme was lost");
            prop_assert_eq!(
                xml.matches("<a:font ").count(),
                baseline_theme.matches("<a:font ").count(),
                "imported script-font overrides were lost"
            );

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen theme package");
            prop_assert_eq!(reopened.document_theme(), Some(expected.clone()));

            let mut no_op = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                no_op.execute(
                    Box::new(SetDocumentThemeCommand::new(expected)),
                    &mut context,
                ).expect("repeat theme");
            }
            prop_assert!(!no_op.can_undo());
            prop_assert_eq!(document.export_docx().expect("no-op export"), applied.clone());

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo theme"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, original_selection);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo theme"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn invalid_theme_is_atomic() {
        let mut document = load_fixture();
        let baseline = document.export_docx().expect("baseline");
        let invalid = DocumentThemeSpec {
            accent1: "not-a-color".to_string(),
            ..DocumentThemeSpec::default()
        };
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let mut commands = CommandExecutor::new();
        let error = {
            let mut context = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(SetDocumentThemeCommand::new(invalid)),
                    &mut context,
                )
                .expect_err("reject invalid theme")
        };
        assert!(error.to_string().contains("color"));
        assert_eq!(document.export_docx().expect("unchanged"), baseline);
        assert!(!commands.can_undo());
    }
}
