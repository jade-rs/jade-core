//! Font family assignment command implementation.
//!
//! Applies a concrete font family to the current selection, populating the
//! latin/east-asia/complex slots so that the renderer can resolve per-script
//! fonts while keeping the DOCX export in sync.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::{apply_run_properties, mutate_run_properties};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{RunFonts, RunProperties};
use jade_docx_layout::selection::SelectionRange;

/// Whether a ribbon font choice targets the East Asian slot. Word's ribbon
/// writes an East Asian family into every slot (such fonts cover Latin too)
/// but keeps `w:eastAsia` untouched when a Latin-only family is chosen, so
/// picking Arial never changes the font Japanese glyphs render with.
pub fn font_family_targets_east_asia(family: &str) -> bool {
    let has_cjk = family.chars().any(|c| {
        matches!(c,
            '\u{3000}'..='\u{30FF}'          // CJK punctuation, kana
            | '\u{3400}'..='\u{9FFF}'        // CJK ideographs
            | '\u{F900}'..='\u{FAFF}'        // compatibility ideographs
            | '\u{FF00}'..='\u{FFEF}'        // fullwidth forms
            | '\u{AC00}'..='\u{D7AF}'        // hangul
        )
    });
    if has_cjk {
        return true;
    }
    let normalized = family.trim().to_ascii_lowercase();
    const KNOWN_EAST_ASIAN: &[&str] = &[
        "ms mincho",
        "ms pmincho",
        "ms gothic",
        "ms pgothic",
        "ms ui gothic",
        "meiryo",
        "meiryo ui",
        "yu mincho",
        "yu gothic",
        "yu gothic ui",
        "biz udgothic",
        "biz udpgothic",
        "biz udmincho",
        "biz udpmincho",
        "hiragino sans",
        "hiragino mincho pron",
        "hiragino kaku gothic pron",
        "simsun",
        "simhei",
        "mingliu",
        "pmingliu",
        "malgun gothic",
        "batang",
        "gulim",
    ];
    KNOWN_EAST_ASIAN.contains(&normalized.as_str())
        || normalized.contains("cjk")
        || normalized.ends_with(" jp")
        || normalized.ends_with(" sc")
        || normalized.ends_with(" tc")
        || normalized.ends_with(" kr")
}

/// Builds the `w:rFonts` payload the ribbon writes for `family`, shared by
/// the range command and the pending caret-formatting state.
pub(crate) fn ribbon_run_fonts(family: &std::rc::Rc<str>) -> RunFonts {
    if font_family_targets_east_asia(family) {
        RunFonts {
            ascii: Some(family.clone()),
            h_ansi: Some(family.clone()),
            east_asia: Some(family.clone()),
            complex: Some(family.clone()),
            ..RunFonts::default()
        }
    } else {
        RunFonts {
            ascii: Some(family.clone()),
            h_ansi: Some(family.clone()),
            ..RunFonts::default()
        }
    }
}

/// Assigns a specific font family to the selected runs.
pub struct SetFontFamilyCommand {
    range: SelectionRange,
    font_family: String,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl SetFontFamilyCommand {
    pub fn new(range: SelectionRange, font_family: impl Into<String>) -> Self {
        Self {
            range,
            font_family: font_family.into(),
            previous_runs: None,
            previous_selection: None,
        }
    }

    fn normalize_family(family: &str) -> Result<String, CommandError> {
        let trimmed = family.trim();
        if trimmed.is_empty() {
            return Err(CommandError::Custom(
                "font family must be a non-empty string".into(),
            ));
        }
        Ok(trimmed.to_string())
    }
}

impl Command for SetFontFamilyCommand {
    fn name(&self) -> &'static str {
        "set_font_family"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }

        let normalized = Self::normalize_family(&self.font_family)?;
        let (start, end) = self.range.normalized();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let font: std::rc::Rc<str> = std::rc::Rc::from(normalized.as_str());
        let targets_east_asia = font_family_targets_east_asia(&font);
        let font_props = RunProperties {
            font_family: Some(font.clone()),
            fonts: Some(ribbon_run_fonts(&font)),
            ..Default::default()
        };

        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);

            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };

            if range_start >= range_end {
                continue;
            }

            let mut restyled = apply_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &font_props,
                &zero_width,
            );
            restyled.extend(mutate_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &zero_width,
                |properties| {
                    if let Some(fonts) = properties.fonts.as_mut() {
                        // A concrete family replaces Word theme slots. Keeping the
                        // old theme attributes beside w:ascii/w:eastAsia makes the
                        // saved run ambiguous to consumers that prefer theme data.
                        // A Latin-only pick leaves the East Asian pair alone: the
                        // Japanese glyphs keep their current (possibly theme-
                        // driven) face, like Word's ribbon.
                        fonts.ascii_theme = None;
                        fonts.h_ansi_theme = None;
                        if targets_east_asia {
                            fonts.east_asia_theme = None;
                            fonts.complex_theme = None;
                        }
                    }
                    properties.clear_direct_font_themes(targets_east_asia);
                },
            ));
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
        }

        *ctx.selection = Some(self.range);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &crate::document::Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    use proptest::prelude::*;
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    fn sample_document() -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "Hello".to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn applies_font_family_to_selection() {
        let mut document = sample_document();
        let mut executor = CommandExecutor::new();
        let mut selection = Some(SelectionRange::new(
            TextPosition {
                paragraph: 0,
                offset: 0,
            },
            TextPosition {
                paragraph: 0,
                offset: 5,
            },
        ));
        let range = selection.expect("selection");
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetFontFamilyCommand::new(range, "Noto Serif JP")),
                    &mut ctx,
                )
                .unwrap();
        }

        let paragraph = document
            .document
            .as_ref()
            .expect("document")
            .body
            .as_ref()
            .unwrap()
            .elements[0]
            .as_paragraph()
            .unwrap()
            .clone();

        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("properties present");
        assert_eq!(props.font_family.as_deref(), Some("Noto Serif JP"));
        let fonts = props.fonts.as_ref().expect("fonts present");
        assert_eq!(fonts.ascii.as_deref(), Some("Noto Serif JP"));
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Serif JP"));
    }

    #[test]
    fn latin_only_family_leaves_the_east_asian_slot_alone() {
        // Word's ribbon: picking Arial must not change the font Japanese
        // glyphs render with.
        let mut document = sample_document();
        {
            let paragraph = document.paragraph_mut(0).expect("paragraph");
            paragraph.runs[0].properties = Some(RunProperties {
                fonts: Some(RunFonts {
                    east_asia: Some(std::rc::Rc::from("游明朝")),
                    ..RunFonts::default()
                }),
                ..Default::default()
            });
        }
        let mut executor = CommandExecutor::new();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let range = selection.expect("selection");
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetFontFamilyCommand::new(range, "Arial")),
                    &mut ctx,
                )
                .unwrap();
        }
        let paragraph = document.paragraph(0).expect("paragraph");
        let fonts = paragraph.runs[0]
            .properties
            .as_ref()
            .and_then(|props| props.fonts.as_ref())
            .expect("fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Arial"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Arial"));
        assert_eq!(
            fonts.east_asia.as_deref(),
            Some("游明朝"),
            "a Latin-only ribbon font must not clobber w:eastAsia"
        );
    }

    #[test]
    fn concrete_font_family_replaces_inherited_theme_slots() {
        let mut document = sample_document();
        let paragraph = document.paragraph_mut(0).expect("paragraph");
        paragraph.runs[0].properties = Some(RunProperties {
            fonts: Some(RunFonts {
                ascii_theme: Some("minorAscii".into()),
                h_ansi_theme: Some("minorHAnsi".into()),
                east_asia_theme: Some("minorEastAsia".into()),
                complex_theme: Some("minorBidi".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        });
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetFontFamilyCommand::new(range, "Noto Sans JP")),
                    &mut ctx,
                )
                .expect("set concrete family");
        }

        let fonts = document
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].properties.as_ref())
            .and_then(|properties| properties.fonts.as_ref())
            .expect("run fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.complex.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.ascii_theme, None);
        assert_eq!(fonts.h_ansi_theme, None);
        assert_eq!(fonts.east_asia_theme, None);
        assert_eq!(fonts.complex_theme, None);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn arbitrary_concrete_families_clear_themes_and_undo_losslessly(
            family in "[A-Za-z][A-Za-z0-9 -]{0,20}",
            theme in "[A-Za-z][A-Za-z0-9]{0,20}",
        ) {
            let mut document = sample_document();
            let original_fonts = RunFonts {
                ascii_theme: Some(theme.clone().into()),
                h_ansi_theme: Some(format!("{theme}HAnsi").into()),
                east_asia_theme: Some(format!("{theme}EastAsia").into()),
                complex_theme: Some(format!("{theme}Bidi").into()),
                ..RunFonts::default()
            };
            document.paragraph_mut(0).expect("paragraph").runs[0].properties = Some(RunProperties {
                fonts: Some(original_fonts.clone()),
                ..RunProperties::default()
            });
            let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(SetFontFamilyCommand::new(range, family.clone())),
                    &mut ctx,
                ).expect("set font family");
            }
            let normalized_family = family.trim();
            let targets_east_asia = font_family_targets_east_asia(normalized_family);
            let applied = document.paragraph(0).unwrap().runs[0]
                .properties.as_ref().unwrap().fonts.as_ref().unwrap();
            prop_assert_eq!(applied.ascii.as_deref(), Some(normalized_family));
            prop_assert_eq!(applied.h_ansi.as_deref(), Some(normalized_family));
            prop_assert!(applied.ascii_theme.is_none());
            prop_assert!(applied.h_ansi_theme.is_none());
            if targets_east_asia {
                prop_assert_eq!(applied.east_asia.as_deref(), Some(normalized_family));
                prop_assert_eq!(applied.complex.as_deref(), Some(normalized_family));
                prop_assert!(applied.east_asia_theme.is_none());
                prop_assert!(applied.complex_theme.is_none());
            } else {
                // A Latin-only pick leaves the East Asian pair alone, so the
                // theme-driven Japanese face survives.
                prop_assert!(applied.east_asia.is_none());
                prop_assert!(applied.complex.is_none());
                prop_assert!(applied.east_asia_theme.is_some());
                prop_assert!(applied.complex_theme.is_some());
            }

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo"));
            }
            let restored = document.paragraph(0).unwrap().runs[0]
                .properties.as_ref().unwrap().fonts.as_ref().unwrap();
            prop_assert_eq!(restored, &original_fonts);
        }
    }

    #[test]
    fn font_family_command_updates_exported_document_xml() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        let range = selection.unwrap();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetFontFamilyCommand::new(range, "Noto Serif JP")),
                    &mut ctx,
                )
                .unwrap();
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive for exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml in exported docx")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");

        assert!(
            document_xml.contains(r#"w:rFonts w:ascii="Noto Serif JP""#),
            "expected exported XML to contain updated font family: {document_xml}"
        );
    }

    #[test]
    fn undo_and_redo_restore_font_family_selection() {
        let mut document = sample_document();
        let mut executor = CommandExecutor::new();
        let mut selection = Some(SelectionRange::new(
            TextPosition {
                paragraph: 0,
                offset: 0,
            },
            TextPosition {
                paragraph: 0,
                offset: 5,
            },
        ));
        let range = selection.expect("selection");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetFontFamilyCommand::new(range, "Noto Sans JP")),
                    &mut ctx,
                )
                .unwrap();
        }
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
        }
        let paragraph = document
            .document
            .as_ref()
            .expect("document")
            .body
            .as_ref()
            .unwrap()
            .elements[0]
            .as_paragraph()
            .unwrap();
        assert!(
            paragraph.runs.iter().all(|run| run
                .properties
                .as_ref()
                .and_then(|p| p.font_family.as_deref())
                .is_none()),
            "undo should remove font family"
        );

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.redo(&mut ctx).unwrap());
        }
        let paragraph = document
            .document
            .as_ref()
            .expect("document")
            .body
            .as_ref()
            .unwrap()
            .elements[0]
            .as_paragraph()
            .unwrap();
        assert!(
            paragraph.runs.iter().any(|run| {
                run.properties
                    .as_ref()
                    .and_then(|p| p.font_family.as_deref())
                    == Some("Noto Sans JP")
            }),
            "redo should re-apply font family"
        );
        assert_eq!(selection.unwrap(), range);
    }
}
