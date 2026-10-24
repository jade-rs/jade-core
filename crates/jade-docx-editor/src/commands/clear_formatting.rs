//! Clear direct character formatting from an exact text selection (Word's
//! Ctrl+Space; highlighting stays).

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::mutate_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, RunProperties};
use crate::docx_xml::{XmlElement, XmlNode};
use jade_docx_layout::selection::SelectionRange;

pub struct ClearFormattingCommand {
    range: SelectionRange,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl ClearFormattingCommand {
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for ClearFormattingCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "clear_formatting"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }
        let (start, end) = self.range.normalized();
        if ctx.document.paragraph_count() == 0
            || start.paragraph >= ctx.document.paragraph_count()
            || end.paragraph >= ctx.document.paragraph_count()
        {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let theme = ctx.document.loaded_theme_part();
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

            let restyled = mutate_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &zero_width,
                clear_visual_formatting,
            );
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
            // Run properties hold the fonts the styles give as well, filled in
            // at load; cleared, the text would draw in the renderer's fallback
            // font until a reload fills them again.
            ctx.document.apply_run_font_defaults_at(paragraph_index);
            if let Some(theme) = theme.as_ref() {
                ctx.document.apply_theme_fonts_at(paragraph_index, theme);
            }
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

fn clear_visual_formatting(props: &mut RunProperties) {
    // Keep semantic/preservation-only properties (language, hidden control
    // runs, ruby, and raw unknown rPr children) intact. Clear the character
    // style and every modeled property exposed as visual direct formatting —
    // except the highlighter: Word's Ctrl+Space and ribbon 「すべての書式を
    // クリア」 both leave `w:highlight` in place (it is removed with the
    // highlighter's 「色なし」 instead).
    props.style = None;
    props.bold = None;
    props.italic = None;
    props.bold_complex = None;
    props.italic_complex = None;
    props.underline = None;
    props.underline_kind = None;
    props.underline_color = None;
    props.strike = None;
    props.double_strike = None;
    props.caps = None;
    props.small_caps = None;
    props.shadow = None;
    props.outline = None;
    props.emboss = None;
    props.imprint = None;
    props.emphasis = None;
    props.effect = None;
    props.baseline = None;
    props.text_position = None;
    props.char_spacing = None;
    props.kerning = None;
    props.text_scale = None;
    props.fit_text = None;
    props.font_size = None;
    props.font_size_complex = None;
    props.font_family = None;
    props.fonts = None;
    props.color = None;
    props.shading = None;
    props.border = None;
    props.clear_direct_font_properties();
}

/// XML counterpart for stories edited directly in their package part. Keep
/// highlight, language, hidden controls and unknown properties just as in the
/// modeled body command above.
pub(crate) fn clear_visual_formatting_xml(properties: &mut XmlElement) {
    properties.children.retain(|child| {
        !matches!(child, XmlNode::Element(element) if matches!(element.name.as_str(),
            "w:rStyle" | "w:b" | "w:bCs" | "w:i" | "w:iCs" | "w:u" |
            "w:strike" | "w:dstrike" | "w:caps" | "w:smallCaps" | "w:shadow" |
            "w:outline" | "w:emboss" | "w:imprint" | "w:em" | "w:effect" |
            "w:vertAlign" | "w:position" | "w:spacing" | "w:kern" | "w:w" |
            "w:fitText" | "w:sz" | "w:szCs" | "w:rFonts" | "w:color" |
            "w:shd" | "w:bdr"))
    });
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::apply_bold::ApplyBoldCommand;
    use crate::commands::apply_italic::ApplyItalicCommand;
    use crate::commands::apply_strike::ApplyStrikeCommand;
    use crate::commands::apply_superscript::ApplySuperscriptCommand;
    use crate::commands::apply_underline::ApplyUnderlineCommand;
    use crate::commands::set_font_size::SetFontSizeCommand;
    use crate::commands::set_highlight_color::SetHighlightColorCommand;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use std::io::Read;
    use std::path::PathBuf;
    use std::rc::Rc;
    use zip::ZipArchive;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    #[test]
    fn clears_visual_formatting_but_keeps_semantic_run_properties() {
        let mut props = RunProperties {
            style: Some(Rc::from("Emphasis")),
            bold: Some(true),
            bold_complex: Some(true),
            italic: Some(true),
            italic_complex: Some(true),
            underline: Some(true),
            underline_kind: Some(Rc::from("wave")),
            underline_color: Some(Rc::from("FF0000")),
            font_size: Some(24.0),
            color: Some(Rc::from("#112233")),
            language: Some(Rc::from("ja-JP")),
            vanish: Some(true),
            spec_vanish: Some(true),
            ..RunProperties::default()
        };

        clear_visual_formatting(&mut props);

        assert!(props.style.is_none());
        assert!(props.bold.is_none());
        assert!(props.bold_complex.is_none());
        assert!(props.italic.is_none());
        assert!(props.italic_complex.is_none());
        assert!(props.underline.is_none());
        assert!(props.underline_kind.is_none());
        assert!(props.underline_color.is_none());
        assert!(props.font_size.is_none());
        assert!(props.color.is_none());
        assert_eq!(props.language.as_deref(), Some("ja-JP"));
        assert_eq!(props.vanish, Some(true));
        assert_eq!(props.spec_vanish, Some(true));
    }

    #[test]
    fn clear_formatting_exports_and_reopens_without_direct_font_size() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut commands = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            commands
                .execute(Box::new(SetFontSizeCommand::new(range, 24.0)), &mut ctx)
                .expect("set font size");
            commands
                .execute(Box::new(ApplyBoldCommand::new(range)), &mut ctx)
                .expect("set bold");
            commands
                .execute(Box::new(ApplyItalicCommand::new(range)), &mut ctx)
                .expect("set italic");
            commands
                .execute(
                    Box::new(ApplyUnderlineCommand::new_with_kind(range, "wave")),
                    &mut ctx,
                )
                .expect("set underline");
            commands
                .execute(Box::new(ApplyStrikeCommand::new(range)), &mut ctx)
                .expect("set strike");
            commands
                .execute(Box::new(ApplySuperscriptCommand::new(range)), &mut ctx)
                .expect("set superscript");
            commands
                .execute(
                    Box::new(SetHighlightColorCommand::new(range, "yellow")),
                    &mut ctx,
                )
                .expect("set highlight");
            commands
                .execute(Box::new(ClearFormattingCommand::new(range)), &mut ctx)
                .expect("clear formatting");
        }

        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            ZipArchive::new(std::io::Cursor::new(exported.clone())).expect("open exported docx");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        assert!(
            !document_xml.contains(r#"w:sz w:val="48""#),
            "cleared 24pt direct formatting must not remain in document.xml"
        );
        for stale_property in ["<w:b/>", "<w:i/>", "<w:u ", "<w:strike/>", "<w:vertAlign "] {
            assert!(
                !document_xml.contains(stale_property),
                "cleared direct formatting {stale_property} must not remain: {document_xml}"
            );
        }

        // Word's Ctrl+Space and ribbon Clear All Formatting both leave the
        // highlighter alone: highlighting is not treated as text formatting.
        assert!(
            document_xml.contains(r#"<w:highlight w:val="yellow"/>"#),
            "highlight must survive clearing character formatting: {document_xml}"
        );

        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen export");
        let paragraph = reopened.paragraph(0).expect("first paragraph");
        assert!(paragraph.runs.iter().all(|run| run
            .properties
            .as_ref()
            .and_then(|props| props.font_size)
            != Some(24.0)));
        assert!(paragraph.runs.iter().all(|run| {
            let props = run.properties.as_ref();
            props.and_then(|value| value.bold) != Some(true)
                && props.and_then(|value| value.italic) != Some(true)
                && props.and_then(|value| value.underline) != Some(true)
                && props.and_then(|value| value.underline_kind.as_deref()) != Some("wave")
                && props.and_then(|value| value.strike) != Some(true)
                && props.and_then(|value| value.baseline.as_deref()) != Some("superscript")
        }));
        assert!(paragraph.runs.iter().take(1).all(|run| run
            .properties
            .as_ref()
            .and_then(|props| props.highlight.as_deref())
            == Some("yellow")));
    }
}
