//! Word's Ctrl+Q: strip direct paragraph formatting.
//!
//! Alignment, indents, spacing, shading, borders, tab stops, frames and the
//! other direct `w:pPr` formatting fall away so the paragraph shows its
//! style's formatting again. The paragraph style, its numbering, and the
//! run-level formatting (Ctrl+Space's job) all stay.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
    sync_reset_paragraph_formatting_xml,
};
use super::set_numbering::tracked_change_supplies_numbering;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::word_document::WordDocument;
use jade_docx_layout::selection::SelectionRange;
use jade_docx_reader::docx_xml::XmlElement;

pub struct ResetParagraphFormattingCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    changed: bool,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl ResetParagraphFormattingCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            changed: false,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }

    /// Gives the direct-formatting fields of `props` back to what the styles
    /// give the paragraph, `inherited`, returning whether anything changed.
    /// The properties hold the styles' formatting resolved in as well, as
    /// loading leaves them; clearing a field would lose the style's value too.
    fn reset_direct_formatting(
        props: &mut ParagraphProperties,
        inherited: &ParagraphProperties,
    ) -> bool {
        let mut changed = false;
        macro_rules! reset {
            ($($field:ident),+ $(,)?) => {
                $(if props.$field != inherited.$field {
                    props.$field = inherited.$field.clone();
                    changed = true;
                })+
            };
        }
        reset!(
            alignment,
            indent,
            spacing,
            shading,
            borders,
            mirror_indents,
            text_alignment,
            contextual_spacing,
            drop_cap,
            tabs,
        );
        changed
    }
}

impl Command for ResetParagraphFormattingCommand {
    fn name(&self) -> &'static str {
        "reset_paragraph_formatting"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.previous_raw_properties.clear();
        self.previous_selection = *ctx.selection;
        self.changed = false;

        for paragraph_index in start..=end {
            let previous_raw = snapshot_paragraph_properties_xml(ctx.document, paragraph_index);
            self.previous_raw_properties
                .push((paragraph_index, previous_raw));
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties.push((paragraph_index, previous));

            let style = self
                .previous_properties
                .last()
                .and_then(|(_, props)| props.as_ref())
                .and_then(|props| props.style.as_deref())
                .map(str::to_string);
            let mut inherited = ctx
                .document
                .inherited_paragraph_properties(paragraph_index, style.as_deref())
                .unwrap_or_default();
            // Without a `w:ind` of its own, a paragraph its own `w:pPr` puts
            // in a list (directly, or through a pending change) takes its left
            // and hanging indent from the list level, not its style, as a
            // load resolves it. Reset gave it the style's: List Paragraph's
            // 36pt on screen, the level's indent once reopened. A style that
            // numbers its paragraphs keeps its own indent.
            let listed_by_paragraph = ctx
                .document
                .paragraph_declared_property_tags(paragraph_index)
                .iter()
                .any(|tag| tag == "w:numPr")
                || tracked_change_supplies_numbering(
                    self.previous_raw_properties
                        .last()
                        .and_then(|(_, raw)| raw.as_ref()),
                );
            let numbered = listed_by_paragraph
                && self
                    .previous_properties
                    .last()
                    .and_then(|(_, props)| props.as_ref())
                    .and_then(|props| props.numbering.as_ref())
                    .is_some_and(|numbering| numbering.list_id().is_some());
            if numbered {
                inherited.indent = inherited
                    .indent
                    .as_ref()
                    .map(WordDocument::right_indent_only);
            }
            let paragraph_changed = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                match paragraph.properties.as_mut() {
                    Some(props) => Self::reset_direct_formatting(props, &inherited),
                    None => false,
                }
            };
            if paragraph_changed {
                sync_reset_paragraph_formatting_xml(ctx.document, paragraph_index);
                self.changed = true;
            }
        }

        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous) in &self.previous_properties {
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
            }
            let raw_snapshot = self
                .previous_raw_properties
                .iter()
                .find(|(index, _)| index == paragraph_index)
                .and_then(|(_, snapshot)| snapshot.as_ref());
            restore_paragraph_properties_xml(ctx.document, *paragraph_index, raw_snapshot);
        }
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
    use crate::document::{Indent, Numbering, Spacing};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
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

    fn load_fixture(name: &str) -> WordDocument {
        let data = std::fs::read(fixture_path(name)).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn reset(document: &mut WordDocument, start: usize, end: usize) -> CommandExecutor {
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(document, &mut selection);
        executor
            .execute(
                Box::new(ResetParagraphFormattingCommand::new(start, end)),
                &mut ctx,
            )
            .expect("reset paragraph formatting");
        executor
    }

    /// A list paragraph without a `w:ind` of its own is indented by its list
    /// level, not its style. numbering-del.docx's items are List Paragraph
    /// (36pt left) numbered at level 0; Reset gave them the style's 36pt,
    /// which a reload, taking the level's indent, does not.
    #[test]
    fn a_list_paragraph_keeps_its_levels_indent() {
        let data = std::fs::read(fixture_path("numbering-del.docx")).expect("read fixture");
        let mut document = load_fixture("numbering-del.docx");
        let loaded =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let indent = |document: &WordDocument| {
            document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.indent.clone())
        };
        reset(&mut document, 0, 1);
        assert_eq!(
            indent(&document),
            indent(&loaded),
            "nothing direct to reset"
        );
        let reloaded = executor::block_on(WordDocument::load(
            &document.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(indent(&document), indent(&reloaded));
    }

    /// A style that numbers its paragraphs keeps its own indent: Heading 3
    /// in numbering-by-style.docx is numbered by the style and indented
    /// 26.25pt by it, which a load keeps. Reset took that indent away.
    #[test]
    fn a_paragraph_its_style_numbers_keeps_the_styles_indent() {
        let data = std::fs::read(fixture_path("numbering-by-style.docx")).expect("read fixture");
        let mut document = load_fixture("numbering-by-style.docx");
        let loaded =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let indent = |document: &WordDocument| {
            document
                .paragraph(1)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.indent.clone())
        };
        reset(&mut document, 1, 1);
        assert_eq!(
            indent(&document),
            indent(&loaded),
            "nothing direct to reset"
        );
        let reloaded = executor::block_on(WordDocument::load(
            &document.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(indent(&document), indent(&reloaded));
    }

    #[test]
    fn strips_direct_formatting_but_keeps_style_and_numbering() {
        let mut document = load_fixture("hello.docx");
        {
            let paragraph = document.paragraph_mut(0).expect("paragraph");
            let props = paragraph.properties.get_or_insert_with(Default::default);
            props.alignment = Some("center".into());
            props.indent = Some(Indent {
                left: Some(42.0),
                first_line: Some(21.0),
                ..Indent::default()
            });
            props.spacing = Some(Spacing {
                before: Some(12.0),
                ..Spacing::default()
            });
            props.numbering = Some(Numbering {
                id: Some("1".to_string()),
                level: Some(2),
            });
        }
        let style_before = document
            .paragraph(0)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.style.clone());

        let mut executor = reset(&mut document, 0, 0);

        let props = document
            .paragraph(0)
            .and_then(|p| p.properties.clone())
            .expect("props");
        // What the styles give the paragraph, as a reload resolves it.
        let inherited = document
            .inherited_paragraph_properties(0, style_before.as_deref())
            .expect("styles");
        assert_eq!(props.alignment, inherited.alignment);
        // The numbering is the model's alone, with no `w:numPr` in the file
        // to put the paragraph in a list on load: its indent is the style's.
        assert_eq!(props.indent, inherited.indent);
        assert_eq!(props.spacing, inherited.spacing);
        assert_ne!(props.alignment.as_deref(), Some("center"));
        assert_eq!(
            props.numbering.as_ref().and_then(|n| n.level),
            Some(2),
            "Ctrl+Q keeps the list membership"
        );
        assert_eq!(props.style, style_before, "the paragraph style survives");

        // Undo restores everything, including the raw XML.
        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let restored = document
            .paragraph(0)
            .and_then(|p| p.properties.clone())
            .expect("restored");
        assert_eq!(restored.alignment.as_deref(), Some("center"));
        assert_eq!(restored.indent.as_ref().and_then(|i| i.left), Some(42.0));
    }

    #[test]
    fn reset_removes_direct_formatting_from_exported_xml() {
        let mut document = load_fixture("indent-with-grid.docx");
        let target = (0..document.paragraph_count())
            .find(|&index| {
                document
                    .paragraph(index)
                    .and_then(|p| p.properties.as_ref())
                    .and_then(|props| props.indent.as_ref())
                    .is_some_and(|indent| indent.first_line_chars.is_some())
            })
            .expect("an indented paragraph");

        reset(&mut document, target, target);

        let exported = document.export_docx().expect("export");
        let mut archive = ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .expect("read xml");
        assert!(
            !xml.contains("w:firstLineChars"),
            "the direct indent must be gone from the exported XML"
        );
    }

    #[test]
    fn noop_reset_records_no_history() {
        let mut document = load_fixture("hello.docx");
        // hello.docx paragraphs carry only style/bidi — nothing to strip
        // beyond what a first reset removes.
        let executor = reset(&mut document, 0, 0);
        let depth_after_first = executor.undo_depth();
        let executor = reset(&mut document, 0, 0);
        assert_eq!(
            executor.undo_depth(),
            0,
            "a second reset changes nothing and records no history"
        );
        // (The first reset may or may not have stripped something depending
        // on the fixture's direct formatting; the invariant is the no-op.)
        let _ = depth_after_first;
    }

    #[test]
    fn selection_stays_where_it_was() {
        let mut document = load_fixture("hello.docx");
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 1),
            TextPosition::new(0, 3),
        ));
        let expected = selection;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(ResetParagraphFormattingCommand::new(0, 0)),
                    &mut ctx,
                )
                .expect("reset");
        }
        assert_eq!(selection, expected);
    }
}
