//! Page setup commands (Word の「レイアウト」タブ: ページ設定).
//!
//! Edits the default section's `<w:sectPr>/<w:pgSz>` (ECMA-376 Part 1
//! §17.6.13). Currently exposes orientation (portrait ⇄ landscape); the same
//! `WordDocument::apply_page_setup` plumbing is reused for future size/margin
//! commands. Undo restores the previous page dimensions captured at apply time.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{PageMarginSpec, PageMarginsBackup, PageSetupBackup};

/// Toggles the page orientation of the default section, supporting undo by
/// restoring the previous width/height/orientation.
pub struct SetPageOrientationCommand {
    landscape: bool,
    /// Section the edit applies to; `None` is the final/default section.
    section: Option<usize>,
    backup: Option<PageSetupBackup>,
}

impl SetPageOrientationCommand {
    pub fn new(landscape: bool) -> Self {
        Self::for_section(None, landscape)
    }

    /// Word applies the ribbon's orientation to the section holding the
    /// caret; `section` is that section's index in `document.sections`.
    pub fn for_section(section: Option<usize>, landscape: bool) -> Self {
        Self {
            landscape,
            section,
            backup: None,
        }
    }
}

impl Command for SetPageOrientationCommand {
    fn name(&self) -> &'static str {
        "set_page_orientation"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = ctx
            .document
            .set_page_orientation_in_section(self.section, self.landscape)
            .map_err(CommandError::Custom)?;
        self.backup = Some(backup);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("missing page setup backup for undo".into()))?;
        ctx.document
            .restore_page_setup_in_section(self.section, backup)
            .map_err(CommandError::Custom)?;
        Ok(())
    }
}

/// Sets the page size to a preset (portrait points), preserving orientation.
pub struct SetPageSizeCommand {
    portrait_width: f32,
    portrait_height: f32,
    section: Option<usize>,
    backup: Option<PageSetupBackup>,
}

impl SetPageSizeCommand {
    pub fn new(portrait_width: f32, portrait_height: f32) -> Self {
        Self::for_section(None, portrait_width, portrait_height)
    }

    pub fn for_section(section: Option<usize>, portrait_width: f32, portrait_height: f32) -> Self {
        Self {
            portrait_width,
            portrait_height,
            section,
            backup: None,
        }
    }
}

impl Command for SetPageSizeCommand {
    fn name(&self) -> &'static str {
        "set_page_size"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = ctx
            .document
            .set_page_size_in_section(self.section, self.portrait_width, self.portrait_height)
            .map_err(CommandError::Custom)?;
        self.backup = Some(backup);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("missing page size backup for undo".into()))?;
        ctx.document
            .restore_page_setup_in_section(self.section, backup)
            .map_err(CommandError::Custom)?;
        Ok(())
    }
}

/// Sets the page margins (points), supporting undo.
pub struct SetPageMarginsCommand {
    spec: PageMarginSpec,
    section: Option<usize>,
    backup: Option<PageMarginsBackup>,
}

impl SetPageMarginsCommand {
    pub fn new(top: f32, bottom: f32, left: f32, right: f32) -> Self {
        Self::with_spec(PageMarginSpec {
            top: Some(top),
            bottom: Some(bottom),
            left: Some(left),
            right: Some(right),
            ..PageMarginSpec::default()
        })
    }

    /// The partial form: every edge `spec` leaves out keeps its current value,
    /// and the header/footer distances and the binding gutter can be set too.
    pub fn with_spec(spec: PageMarginSpec) -> Self {
        Self::for_section(None, spec)
    }

    /// The partial form for one section (`None` = the final section).
    pub fn for_section(section: Option<usize>, spec: PageMarginSpec) -> Self {
        Self {
            spec,
            section,
            backup: None,
        }
    }
}

impl Command for SetPageMarginsCommand {
    fn name(&self) -> &'static str {
        "set_page_margins"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = ctx
            .document
            .set_page_margin_values_in_section(self.section, self.spec)
            .map_err(CommandError::Custom)?;
        self.backup = Some(backup);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("missing page margins backup for undo".into()))?;
        ctx.document
            .restore_page_margins_in_section(self.section, backup)
            .map_err(CommandError::Custom)?;
        Ok(())
    }
}

/// Sets the document's page background color (デザインタブの「ページの色」),
/// supporting undo by restoring the previous value.
pub struct SetPageBackgroundColorCommand {
    color: Option<String>,
    backup: Option<Option<String>>,
}

impl SetPageBackgroundColorCommand {
    pub fn new(color: Option<String>) -> Self {
        Self {
            color,
            backup: None,
        }
    }
}

impl Command for SetPageBackgroundColorCommand {
    fn name(&self) -> &'static str {
        "set_page_background_color"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = ctx
            .document
            .set_page_background_color(self.color.clone())
            .map_err(CommandError::Custom)?;
        self.backup = Some(previous);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("missing background backup for undo".into()))?;
        ctx.document
            .restore_page_background_color(previous)
            .map_err(CommandError::Custom)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::snapshot_edit::SnapshotEditCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn page_dims(word_doc: &WordDocument) -> (Option<f32>, Option<f32>, Option<String>) {
        let ps = word_doc
            .document
            .as_ref()
            .and_then(|doc| doc.page_settings.as_ref());
        (
            ps.and_then(|p| p.width),
            ps.and_then(|p| p.height),
            ps.and_then(|p| p.orientation.clone()),
        )
    }

    #[test]
    fn set_orientation_swaps_dimensions_and_undoes() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let (w0, h0, _) = page_dims(&word_doc);
        let w0 = w0.expect("fixture has page width");
        let h0 = h0.expect("fixture has page height");
        // hello.docx is portrait (taller than wide).
        assert!(h0 > w0, "fixture should start portrait");

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(SetPageOrientationCommand::new(true)), &mut ctx)
                .expect("apply landscape");
        }
        let (w1, h1, orient1) = page_dims(&word_doc);
        assert!(w1.unwrap() > h1.unwrap(), "landscape should be wider");
        assert_eq!(orient1.as_deref(), Some("landscape"));
        // Long/short edges are preserved, just swapped.
        assert!((w1.unwrap() - h0).abs() < 0.5);
        assert!((h1.unwrap() - w0).abs() < 0.5);

        // The raw XML round-trips the landscape page size.
        let exported = export_document_xml(&word_doc);
        assert!(exported.contains("w:orient=\"landscape\""));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let (w2, h2, orient2) = page_dims(&word_doc);
        assert!((w2.unwrap() - w0).abs() < 0.5);
        assert!((h2.unwrap() - h0).abs() < 0.5);
        assert!(h2.unwrap() > w2.unwrap(), "undo restores portrait");
        assert_ne!(orient2.as_deref(), Some("landscape"));
        assert!(!export_document_xml(&word_doc).contains("w:orient=\"landscape\""));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo"));
        }
        assert_eq!(page_dims(&word_doc).2.as_deref(), Some("landscape"));
    }

    fn export_document_xml(word_doc: &WordDocument) -> String {
        use std::io::Read;
        use zip::ZipArchive;
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml present")
            .read_to_string(&mut xml)
            .expect("read document.xml");
        xml
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn page_columns_round_trip_generated_geometry_and_history(
            count in 1u32..=12,
            space_twips in 0u32..=2880,
            separator in any::<bool>(),
        ) {
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let original_xml = export_document_xml(&word_doc);
            let space = space_twips as f32 / 20.0;
            if space * count.saturating_sub(1) as f32 >= word_doc.section_page_setup(None).content_width {
                prop_assert!(word_doc.set_page_columns(count, space, separator).is_err());
                prop_assert_eq!(export_document_xml(&word_doc), original_xml);
                return Ok(());
            }
            let expected_separator = separator && count > 1;
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(SnapshotEditCommand::new("set_page_columns", move |document| {
                            document.set_page_columns(count, space, separator)
                        })),
                        &mut ctx,
                    )
                    .expect("set page columns");
            }
            let columns = word_doc
                .document
                .as_ref()
                .and_then(|document| document.default_section.as_ref())
                .and_then(|section| section.columns.as_ref())
                .expect("default section columns");
            prop_assert_eq!(columns.count, Some(count));
            prop_assert_eq!(columns.space, Some(space));
            prop_assert_eq!(columns.equal_width, Some(true));
            prop_assert_eq!(columns.separator, Some(expected_separator));
            let xml = export_document_xml(&word_doc);
            let expected_count = format!(r#"w:num="{}""#, count);
            let expected_space = format!(r#"w:space="{}""#, space_twips);
            let expected_separator_attr = if expected_separator {
                r#"w:sep="1""#
            } else {
                r#"w:sep="0""#
            };
            prop_assert!(xml.contains(&expected_count));
            prop_assert!(xml.contains(&expected_space));
            prop_assert!(xml.contains(expected_separator_attr));
            let page_margin_position = xml.rfind("<w:pgMar").expect("page margins");
            let columns_position = xml.rfind("<w:cols").expect("page columns");
            let bidi_position = xml.rfind("<w:bidi").expect("trailing bidi setting");
            prop_assert!(page_margin_position < columns_position);
            prop_assert!(columns_position < bidi_position);

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo columns"));
            }
            prop_assert_eq!(export_document_xml(&word_doc), original_xml);

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo columns"));
            }
            let exported = word_doc.export_docx().expect("export columns");
            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen columns");
            let reopened_columns = reopened
                .document
                .as_ref()
                .and_then(|document| document.default_section.as_ref())
                .and_then(|section| section.columns.as_ref())
                .expect("reopened columns");
            prop_assert_eq!(reopened_columns.count, Some(count));
            prop_assert!((reopened_columns.space.unwrap_or_default() - space).abs() < 0.001);
            prop_assert_eq!(reopened_columns.equal_width, Some(true));
            prop_assert_eq!(reopened_columns.separator, Some(expected_separator));
        }
    }

    #[test]
    fn invalid_page_columns_are_rejected_without_mutating_the_document() {
        let data = std::fs::read(fixture_path("2col.docx")).expect("read fixture");
        for (count, space) in [
            (0, 24.0),
            (13, 24.0),
            (2, -1.0),
            (2, f32::NAN),
            (2, f32::INFINITY),
            (2, 500.0),
            (12, 50.0),
        ] {
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load docx");
            let before = word_doc.export_docx().expect("export before");
            assert!(word_doc.set_page_columns(count, space, true).is_err());
            assert_eq!(word_doc.export_docx().expect("export after"), before);
        }
    }

    #[test]
    fn column_spacing_validation_uses_section_margins_and_serialized_twips() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut doc = executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
        doc.insert_section_break_at(1, 0, crate::word_document::SectionBreakType::NextPage)
            .unwrap();
        doc.set_page_size_in_section(Some(0), 600.0, 800.0).unwrap();
        doc.set_page_margin_values_in_section(
            Some(0),
            PageMarginSpec {
                left: Some(100.0),
                right: Some(100.0),
                ..PageMarginSpec::default()
            },
        )
        .unwrap();
        let setup = doc.section_page_setup(Some(0));
        assert_eq!(setup.width, 600.0);
        assert_eq!(setup.left, 100.0);
        assert_eq!(setup.content_width, 400.0);
        assert_ne!(doc.section_page_setup(Some(1)).left, 100.0);
        assert!(doc
            .set_page_columns_in_section(Some(0), 3, 199.97, true)
            .is_ok());
        let before = export_document_xml(&doc);
        // 199.99pt is stored as 200pt; its two gutters leave zero text width.
        assert!(doc
            .set_page_columns_in_section(Some(0), 3, 199.99, true)
            .is_err());
        assert_eq!(export_document_xml(&doc), before);
        let bytes = doc.export_docx().unwrap();
        let reopened = executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
        let setup = reopened.section_page_setup(Some(0));
        assert_eq!(setup.column_count, 3);
        assert_eq!(setup.column_space, 199.95);
        assert!(setup.separator && setup.equal_width);
        assert_eq!(setup.content_width, 400.0);
    }

    #[test]
    fn page_setup_content_width_accounts_for_binding_gutter() {
        let data = std::fs::read(fixture_path("hello.docx")).unwrap();
        let mut doc = executor::block_on(WordDocument::load(&data, &Options::default())).unwrap();
        let original = doc.section_page_setup(None).content_width;
        doc.set_page_margin_values(PageMarginSpec {
            gutter: Some(24.0),
            ..PageMarginSpec::default()
        })
        .unwrap();
        assert_eq!(doc.section_page_setup(None).content_width, original - 24.0);
        doc.document_mut().unwrap().gutter_at_top = true;
        assert_eq!(doc.section_page_setup(None).content_width, original);
        doc.set_page_columns(2, 18.0, false).unwrap();
        doc.document_mut()
            .unwrap()
            .default_section
            .as_mut()
            .unwrap()
            .columns
            .as_mut()
            .unwrap()
            .equal_width = Some(false);
        assert!(!doc.section_page_setup(None).equal_width);
    }

    fn margins(word_doc: &WordDocument) -> (Option<f32>, Option<f32>, Option<f32>, Option<f32>) {
        let ps = word_doc
            .document
            .as_ref()
            .and_then(|doc| doc.page_settings.as_ref());
        (
            ps.and_then(|p| p.margin_top),
            ps.and_then(|p| p.margin_bottom),
            ps.and_then(|p| p.margin_left),
            ps.and_then(|p| p.margin_right),
        )
    }

    #[test]
    fn set_page_size_preserves_orientation_and_round_trips() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        // Letter portrait = 8.5in × 11in = 612pt × 792pt.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(SetPageSizeCommand::new(612.0, 792.0)), &mut ctx)
                .expect("apply letter size");
        }
        let (w, h, _) = page_dims(&word_doc);
        assert!((w.unwrap() - 612.0).abs() < 0.5);
        assert!((h.unwrap() - 792.0).abs() < 0.5);
        // 612pt × 20 = 12240 twips.
        assert!(export_document_xml(&word_doc).contains("w:w=\"12240\""));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo size"));
        }
        // Undo restores the original A4-ish width (not Letter).
        let (w2, _, _) = page_dims(&word_doc);
        assert!((w2.unwrap() - 612.0).abs() > 1.0);
    }

    #[test]
    fn set_page_background_color_applies_and_undoes() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let before = word_doc
            .document
            .as_ref()
            .and_then(|d| d.background_color.clone());

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetPageBackgroundColorCommand::new(Some("ffcc00".into()))),
                    &mut ctx,
                )
                .expect("apply background");
        }
        assert_eq!(
            word_doc
                .document
                .as_ref()
                .and_then(|d| d.background_color.clone())
                .as_deref(),
            Some("#FFCC00")
        );
        assert!(export_document_xml(&word_doc).contains("<w:background w:color=\"FFCC00\""));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo background"));
        }
        assert_eq!(
            word_doc
                .document
                .as_ref()
                .and_then(|d| d.background_color.clone()),
            before
        );
        // The color we applied is gone after undo (the fixture may or may not
        // have had a native <w:background>, so only assert FFCC00 is removed).
        assert!(!export_document_xml(&word_doc).contains("FFCC00"));
    }

    fn section_orientations(word_doc: &WordDocument) -> Vec<Option<String>> {
        word_doc
            .document
            .as_ref()
            .map(|document| {
                document
                    .sections
                    .iter()
                    .map(|section| {
                        section
                            .properties
                            .as_ref()
                            .and_then(|props| props.page_settings.as_ref())
                            .and_then(|settings| settings.orientation.clone())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn section_left_margins(word_doc: &WordDocument) -> Vec<Option<f32>> {
        word_doc
            .document
            .as_ref()
            .map(|document| {
                document
                    .sections
                    .iter()
                    .map(|section| {
                        section
                            .properties
                            .as_ref()
                            .and_then(|props| props.page_settings.as_ref())
                            .and_then(|settings| settings.margin_left)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn page_setup_targets_the_caret_section_and_survives_a_round_trip() {
        use crate::word_document::SectionBreakType;
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        // hello.docx: "Hello" / "World". A break in front of "World" makes two
        // sections; the first is closed by a paragraph-level sectPr.
        word_doc
            .insert_section_break_at(1, 0, SectionBreakType::NextPage)
            .expect("insert section break");
        assert_eq!(word_doc.section_index_for_paragraph(0), Some(0));
        assert_eq!(word_doc.section_index_for_paragraph(2), Some(1));
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        // Landscape for the caret's (first) section only.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetPageOrientationCommand::for_section(Some(0), true)),
                    &mut ctx,
                )
                .expect("landscape on section 0");
        }
        let orientations = section_orientations(&word_doc);
        assert_eq!(orientations[0].as_deref(), Some("landscape"));
        assert_ne!(orientations[1].as_deref(), Some("landscape"));
        // The renderer lays the first pages out with the document settings.
        assert_eq!(page_dims(&word_doc).2.as_deref(), Some("landscape"));

        // Wide margins for the final section only.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetPageMarginsCommand::for_section(
                        Some(1),
                        PageMarginSpec {
                            left: Some(144.0),
                            right: Some(144.0),
                            ..PageMarginSpec::default()
                        },
                    )),
                    &mut ctx,
                )
                .expect("margins on section 1");
        }
        let lefts = section_left_margins(&word_doc);
        assert_ne!(lefts[0], Some(144.0));
        assert_eq!(lefts[1], Some(144.0));
        assert_ne!(margins(&word_doc).2, Some(144.0));

        // Both edits land on the right sectPr in the package.
        let exported = word_doc.export_docx().expect("export");
        let reopened =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reopen");
        // The first pages lay out as they will after a reload.
        assert_eq!(page_dims(&word_doc), page_dims(&reopened));
        assert_eq!(margins(&word_doc), margins(&reopened));
        let orientations = section_orientations(&reopened);
        assert_eq!(orientations[0].as_deref(), Some("landscape"));
        assert_ne!(orientations[1].as_deref(), Some("landscape"));
        let lefts = section_left_margins(&reopened);
        assert_ne!(lefts[0], Some(144.0));
        assert_eq!(lefts[1], Some(144.0));

        // Undo walks both edits back on their own sections.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo margins"));
            assert!(executor.undo(&mut ctx).expect("undo orientation"));
        }
        let orientations = section_orientations(&word_doc);
        assert_ne!(orientations[0].as_deref(), Some("landscape"));
        assert_ne!(page_dims(&word_doc).2.as_deref(), Some("landscape"));
        assert_ne!(section_left_margins(&word_doc)[1], Some(144.0));
    }

    #[test]
    fn set_page_margins_applies_and_undoes() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let before = margins(&word_doc);

        // Narrow preset: 0.5 inch = 36pt all around.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetPageMarginsCommand::new(36.0, 36.0, 36.0, 36.0)),
                    &mut ctx,
                )
                .expect("apply narrow margins");
        }
        let after = margins(&word_doc);
        assert_eq!(after.0, Some(36.0));
        assert_eq!(after.2, Some(36.0));
        // 36pt × 20 = 720 twips on every edge.
        let xml = export_document_xml(&word_doc);
        assert!(xml.contains("w:top=\"720\""));
        assert!(xml.contains("w:left=\"720\""));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo margins"));
        }
        assert_eq!(margins(&word_doc), before);
    }
}
