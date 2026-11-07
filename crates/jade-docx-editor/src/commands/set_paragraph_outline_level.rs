//! The 段落 dialog's アウトライン レベル (`w:outlineLvl`, ECMA-376 Part 1
//! §17.3.1.20) and 既定で折りたたみ (`w15:collapsed`, the Word 2013
//! extension): the level the navigation pane and アウトライン show a
//! paragraph at, and whether it opens collapsed.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_child_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::{XmlAttribute, XmlElement};
use jade_docx_layout::selection::SelectionRange;

/// What 本文 is as an outline level.
pub const BODY_TEXT_LEVEL: u8 = 9;

/// Sets the level and the collapsed flag on a run of paragraphs.
pub struct SetParagraphOutlineLevelCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    level: Option<u8>,
    collapsed: Option<bool>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphOutlineLevelCommand {
    /// `level` is 0–8 for レベル 1–9 and 9 for 本文.
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        level: Option<u8>,
        collapsed: Option<bool>,
    ) -> Self {
        Self {
            start_paragraph: start_paragraph.min(end_paragraph),
            end_paragraph: start_paragraph.max(end_paragraph),
            level,
            collapsed,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
        }
    }
}

impl Command for SetParagraphOutlineLevelCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_paragraph_outline_level"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.level.is_none() && self.collapsed.is_none() {
            return Err(CommandError::Custom(
                "set_paragraph_outline_level requires a level or collapsed".into(),
            ));
        }
        if self.level.is_some_and(|level| level > BODY_TEXT_LEVEL) {
            return Err(CommandError::Custom(
                "an outline level is 0-8, or 9 for body text".into(),
            ));
        }
        let count = ctx.document.paragraph_count();
        if count == 0 || self.end_paragraph >= count {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous_properties.clear();
        self.previous_raw_properties.clear();
        self.previous_selection = *ctx.selection;
        for index in self.start_paragraph..=self.end_paragraph {
            self.previous_raw_properties.push((
                index,
                snapshot_paragraph_properties_xml(ctx.document, index),
            ));
            let previous = ctx
                .document
                .paragraph(index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties.push((index, previous));
            if let Some(level) = self.level {
                // The style's own level needs nothing written: the direct
                // value goes, as Word's dialog leaves it.
                let style_level = ctx.document.style_outline_level(index);
                let direct = level != style_level;
                {
                    let paragraph = ctx
                        .document
                        .paragraph_mut(index)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    let props = paragraph.properties.get_or_insert_with(Default::default);
                    props.outline_level = (direct || style_level != BODY_TEXT_LEVEL)
                        .then_some(if direct { level } else { style_level });
                    props.outline_level_is_direct = direct;
                }
                sync_paragraph_child_xml(
                    ctx.document,
                    index,
                    "w:outlineLvl",
                    direct.then(|| {
                        vec![XmlAttribute {
                            name: "w:val".to_string(),
                            value: level.to_string(),
                        }]
                    }),
                );
            }
            if let Some(collapsed) = self.collapsed {
                sync_paragraph_child_xml(
                    ctx.document,
                    index,
                    "w15:collapsed",
                    collapsed.then(Vec::new),
                );
            }
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (index, previous) in &self.previous_properties {
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
            }
            let raw = self
                .previous_raw_properties
                .iter()
                .find(|(candidate, _)| candidate == index)
                .and_then(|(_, snapshot)| snapshot.as_ref());
            restore_paragraph_properties_xml(ctx.document, *index, raw);
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;

    fn fixture() -> WordDocument {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contract.docx");
        let data = std::fs::read(path).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn document_xml(document: &WordDocument) -> String {
        document
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml")
    }

    fn run(
        document: &mut WordDocument,
        executor: &mut CommandExecutor,
        command: SetParagraphOutlineLevelCommand,
    ) {
        let mut selection = None;
        let mut ctx = CommandContext::new(document, &mut selection);
        executor.execute(Box::new(command), &mut ctx).unwrap();
    }

    #[test]
    fn a_level_and_collapsed_are_written_read_and_undone() {
        let mut document = fixture();
        let mut executor = CommandExecutor::new();
        let paragraph = (0..document.paragraph_count())
            .find(|&index| {
                document.is_body_paragraph_surface(index)
                    && document.style_outline_level(index) == BODY_TEXT_LEVEL
                    && document.paragraph_outline(index).level == BODY_TEXT_LEVEL
            })
            .expect("a body text paragraph");
        let before = document_xml(&document);

        run(
            &mut document,
            &mut executor,
            SetParagraphOutlineLevelCommand::new(paragraph, paragraph, Some(1), Some(true)),
        );
        let state = document.paragraph_outline(paragraph);
        assert_eq!((state.level, state.collapsed), (1, true));
        assert_eq!(
            WordDocument::paragraph_heading_level(document.paragraph(paragraph).unwrap()),
            Some(2),
            "the navigation pane reads レベル 2"
        );
        let xml = document_xml(&document);
        assert!(xml.contains(r#"<w:outlineLvl w:val="1"/>"#), "{xml}");
        assert!(xml.contains("<w15:collapsed/>"));
        let ppr = &xml[..xml.find("<w15:collapsed/>").unwrap()];
        assert!(
            ppr.rfind(r#"<w:outlineLvl w:val="1"/>"#) > ppr.rfind("<w:pPr>"),
            "w:outlineLvl comes before w15:collapsed in the same w:pPr"
        );

        // 本文 again: what the style gives, so nothing is written.
        run(
            &mut document,
            &mut executor,
            SetParagraphOutlineLevelCommand::new(
                paragraph,
                paragraph,
                Some(BODY_TEXT_LEVEL),
                Some(false),
            ),
        );
        let state = document.paragraph_outline(paragraph);
        assert_eq!((state.level, state.collapsed), (BODY_TEXT_LEVEL, false));
        assert_eq!(document_xml(&document), before);

        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
        assert_eq!(document_xml(&document), before);
    }

    #[test]
    fn out_of_range_levels_are_refused() {
        let mut document = fixture();
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let mut executor = CommandExecutor::new();
        assert!(executor
            .execute(
                Box::new(SetParagraphOutlineLevelCommand::new(0, 0, Some(10), None)),
                &mut ctx,
            )
            .is_err());
        assert!(executor
            .execute(
                Box::new(SetParagraphOutlineLevelCommand::new(0, 0, None, None)),
                &mut ctx,
            )
            .is_err());
    }
}
