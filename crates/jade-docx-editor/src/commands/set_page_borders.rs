use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{PageBorderSpec, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

/// Replaces or removes one section's page frame. A package
/// snapshot keeps raw section XML, parsed renderer state, selection, and
/// history atomic even when an imported document carries extension children.
pub struct SetPageBordersCommand {
    spec: Option<PageBorderSpec>,
    section: Option<usize>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetPageBordersCommand {
    pub fn new(spec: Option<PageBorderSpec>) -> Self {
        Self::for_section(None, spec)
    }

    pub fn for_section(section: Option<usize>, spec: Option<PageBorderSpec>) -> Self {
        Self {
            spec,
            section,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetPageBordersCommand {
    fn name(&self) -> &'static str {
        "set_page_borders"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx
            .document
            .set_page_borders_in_section(self.section, self.spec.as_ref())
        {
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
            .ok_or_else(|| CommandError::Custom("missing page-border snapshot for undo".into()))?;
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
    use crate::word_document::{BorderEdgeSpec, WordDocument};
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

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive.by_name("word/document.xml").expect("document.xml");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read document.xml");
        value
    }

    fn page_borders_xml(document_xml: &str) -> &str {
        let start = document_xml
            .find("<w:pgBorders")
            .expect("page borders start");
        let relative_end = document_xml[start..]
            .find("</w:pgBorders>")
            .expect("page borders end");
        &document_xml[start..start + relative_end + "</w:pgBorders>".len()]
    }

    fn edge(style: &str, width: f32, color: &str) -> BorderEdgeSpec {
        BorderEdgeSpec {
            style: Some(style.to_string()),
            width: Some(width),
            color: Some(color.to_string()),
        }
    }

    #[test]
    fn section_frames_preserve_neighbors_and_restore_exact_history() {
        let bytes = std::fs::read(fixture_path().with_file_name("header-footer-sections.docx"))
            .expect("read section fixture");
        let mut document = executor::block_on(WordDocument::load(&bytes, &Options::default()))
            .expect("load section fixture");
        let last = PageBorderSpec {
            bottom: Some(edge("double", 1.0, "#0000FF")),
            ..PageBorderSpec::default()
        };
        document
            .set_page_borders(Some(&last))
            .expect("set final frame");
        let last_frame = document.page_borders(None).cloned();
        assert!(document.page_borders(Some(0)).is_none());
        let baseline = document.export_docx().expect("baseline");
        let first = PageBorderSpec {
            top: Some(edge("single", 3.0, "#C00000")),
            ..PageBorderSpec::default()
        };
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let mut commands = CommandExecutor::new();
        commands
            .execute(
                Box::new(SetPageBordersCommand::for_section(
                    Some(0),
                    Some(first.clone()),
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .expect("set first frame");
        let applied = document.export_docx().expect("applied");
        assert_eq!(document.page_borders(None), last_frame.as_ref());
        assert_eq!(document.page_borders(Some(1)), last_frame.as_ref());
        let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
            .expect("reopen sections");
        assert_eq!(
            reopened.page_borders(Some(0)),
            document.page_borders(Some(0))
        );
        assert_eq!(reopened.page_borders(Some(1)), last_frame.as_ref());
        assert_eq!(document_xml(&applied).matches("<w:pgBorders").count(), 2);
        assert!(!document
            .set_page_borders_in_section(Some(0), Some(&first))
            .unwrap());
        assert_eq!(document.export_docx().unwrap(), applied);
        commands
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert_eq!(document.export_docx().unwrap(), baseline);
        commands
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert_eq!(document.export_docx().unwrap(), applied);
        assert!(document.set_page_borders_in_section(Some(0), None).unwrap());
        assert!(document.page_borders(Some(0)).is_none());
        assert_eq!(document.page_borders(Some(1)), last_frame.as_ref());
        assert_eq!(document.export_docx().unwrap(), baseline);
    }

    #[test]
    fn line_styles_keep_their_schema_spelling_and_art_takes_whole_points() {
        let mut document = load_fixture();
        let frame = PageBorderSpec {
            top: Some(edge("thinthicksmallgap", 3.0, "auto")),
            bottom: Some(edge("threeDEmboss", 1.5, "auto")),
            left: Some(edge("Hearts", 20.0, "auto")),
            right: Some(edge("hearts", 20.0, "auto")),
            offset_from: Some("page".into()),
        };
        document
            .set_page_borders(Some(&frame))
            .expect("set the frame");
        let xml = document_xml(&document.export_docx().unwrap());
        let borders = page_borders_xml(&xml);
        assert!(
            borders.contains(r#"w:val="thinThickSmallGap" w:sz="24""#),
            "{borders}"
        );
        assert!(
            borders.contains(r#"w:val="threeDEmboss" w:sz="12""#),
            "{borders}"
        );
        assert!(
            borders.contains(r#"<w:left w:val="hearts" w:sz="20""#),
            "{borders}"
        );
        let back = document.page_borders(None).unwrap();
        assert_eq!(back.left.as_ref().unwrap().size_eighths, Some(20));
        let too_wide = PageBorderSpec {
            top: Some(edge("hearts", 40.0, "auto")),
            ..PageBorderSpec::default()
        };
        assert!(document.set_page_borders(Some(&too_wide)).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_asymmetric_frames_round_trip_and_restore_exact_history(
            mask in 1u8..16,
            style in prop::sample::select(vec!["single", "double", "dashed", "dotted", "thick", "wave"]),
            width_eighths in 2u32..=96,
            color in 0u32..=0xFF_FFFF,
            page_offset in any::<bool>(),
        ) {
            let width = width_eighths as f32 / 8.0;
            let color = format!("#{color:06X}");
            let spec = PageBorderSpec {
                top: (mask & 1 != 0).then(|| edge(style, width, &color)),
                bottom: (mask & 2 != 0).then(|| edge(style, width, &color)),
                left: (mask & 4 != 0).then(|| edge(style, width, &color)),
                right: (mask & 8 != 0).then(|| edge(style, width, &color)),
                offset_from: Some(if page_offset { "page" } else { "text" }.to_string()),
            };
            let mut document = load_fixture();
            let baseline = document.export_docx().expect("baseline export");
            let original_selection = Some(SelectionRange::collapsed(TextPosition::new(0, 1)));
            let mut selection = original_selection;
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(SetPageBordersCommand::new(Some(spec.clone()))),
                    &mut context,
                ).expect("set page borders");
            }
            let applied = document.export_docx().expect("applied export");
            prop_assert_ne!(&applied, &baseline);
            prop_assert_eq!(selection, original_selection);
            let xml = document_xml(&applied);
            prop_assert_eq!(xml.matches("<w:pgBorders").count(), 1);
            let expected_offset = if page_offset {
                "w:offsetFrom=\"page\""
            } else {
                "w:offsetFrom=\"text\""
            };
            let frame_xml = page_borders_xml(&xml);
            prop_assert!(frame_xml.contains(expected_offset));
            prop_assert_eq!(frame_xml.matches(&format!("w:sz=\"{width_eighths}\"")).count(), mask.count_ones() as usize);
            prop_assert_eq!(frame_xml.matches(&format!("w:color=\"{}\"", &color[1..])).count(), mask.count_ones() as usize);
            prop_assert_eq!(frame_xml.matches(if page_offset { "w:space=\"24\"" } else { "w:space=\"0\"" }).count(), mask.count_ones() as usize);

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen page borders");
            let expected = document.document.as_ref()
                .and_then(|doc| doc.default_section.as_ref())
                .and_then(|section| section.page_borders.clone());
            let renderer_value = document.document.as_ref()
                .and_then(|doc| doc.sections.last())
                .and_then(|section| section.properties.as_ref())
                .and_then(|properties| properties.page_borders.clone());
            prop_assert_eq!(&renderer_value, &expected);
            let actual = reopened.document.as_ref()
                .and_then(|doc| doc.default_section.as_ref())
                .and_then(|section| section.page_borders.clone());
            prop_assert_eq!(actual, expected);

            // Reapplying the normalized value is a true no-op: no ZIP rewrite
            // and no extra history entry.
            let mut no_op = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                no_op.execute(
                    Box::new(SetPageBordersCommand::new(Some(spec))),
                    &mut context,
                ).expect("repeat page borders");
            }
            prop_assert!(!no_op.can_undo());
            prop_assert_eq!(document.export_docx().expect("no-op export"), applied.clone());

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.undo(&mut context).expect("undo page borders"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, original_selection);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(commands.redo(&mut context).expect("redo page borders"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn remove_and_invalid_specs_are_atomic() {
        let mut document = load_fixture();
        let valid = PageBorderSpec {
            top: Some(edge("single", 1.0, "#336699")),
            bottom: Some(edge("double", 2.0, "auto")),
            left: Some(edge("dashed", 0.5, "#000000")),
            right: Some(edge("dotted", 0.25, "#FFFFFF")),
            offset_from: Some("page".to_string()),
        };
        document.set_page_borders(Some(&valid)).expect("set valid");
        let applied = document.export_docx().expect("applied");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let mut remove = CommandExecutor::new();
        {
            let mut context = CommandContext::new(&mut document, &mut selection);
            remove
                .execute(Box::new(SetPageBordersCommand::new(None)), &mut context)
                .expect("remove page borders");
        }
        assert!(!document_xml(&document.export_docx().expect("removed")).contains("w:pgBorders"));
        {
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(remove.undo(&mut context).expect("undo remove"));
        }
        assert_eq!(document.export_docx().expect("restored"), applied);

        for invalid in [
            PageBorderSpec {
                top: Some(edge("single", 0.0, "#000000")),
                ..PageBorderSpec::default()
            },
            PageBorderSpec {
                top: Some(edge("single", 12.125, "#000000")),
                ..PageBorderSpec::default()
            },
            PageBorderSpec {
                top: Some(edge("invalid", 1.0, "#000000")),
                ..PageBorderSpec::default()
            },
            PageBorderSpec {
                top: Some(edge("single", 1.0, "not-a-color")),
                ..PageBorderSpec::default()
            },
            PageBorderSpec {
                top: Some(edge("single", 1.0, "#000000")),
                offset_from: Some("margin".into()),
                ..PageBorderSpec::default()
            },
            PageBorderSpec {
                top: Some(edge("none", 1.0, "#000000")),
                ..PageBorderSpec::default()
            },
        ] {
            let before = document.export_docx().expect("before invalid");
            let original_selection = selection;
            let mut commands = CommandExecutor::new();
            let error = {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands
                    .execute(
                        Box::new(SetPageBordersCommand::new(Some(invalid))),
                        &mut context,
                    )
                    .expect_err("invalid page borders")
            };
            assert!(matches!(error, CommandError::Custom(_)));
            assert_eq!(document.export_docx().expect("after invalid"), before);
            assert_eq!(selection, original_selection);
            assert!(!commands.can_undo());
        }
    }
}
