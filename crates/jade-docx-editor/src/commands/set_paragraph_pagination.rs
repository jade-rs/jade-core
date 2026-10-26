//! The paragraph dialog's 改ページと改行 options: 改ページ時 1 行残して段落を
//! 区切らない (`w:widowControl`), 次の段落と分離しない (`w:keepNext`), 段落を
//! 分割しない (`w:keepLines`) and 段落前で改ページする (`w:pageBreakBefore`)
//! — ECMA-376 Part 1 §17.3.1.44, §17.3.1.14, §17.3.1.15, §17.3.1.23.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_toggle_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// What each option becomes; `None` leaves it as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pagination {
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
}

/// Sets the options on a run of paragraphs as direct formatting.
pub struct SetParagraphPaginationCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    pagination: Pagination,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphPaginationCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, pagination: Pagination) -> Self {
        Self {
            start_paragraph: start_paragraph.min(end_paragraph),
            end_paragraph: start_paragraph.max(end_paragraph),
            pagination,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
        }
    }
}

impl Command for SetParagraphPaginationCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_paragraph_pagination"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let Pagination {
            keep_next,
            keep_lines,
            page_break_before,
            widow_control,
        } = self.pagination;
        if keep_next.is_none()
            && keep_lines.is_none()
            && page_break_before.is_none()
            && widow_control.is_none()
        {
            return Err(CommandError::Custom(
                "set_paragraph_pagination requires at least one option".into(),
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
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                if let Some(on) = keep_next {
                    props.keep_next = Some(on);
                }
                if let Some(on) = keep_lines {
                    props.keep_lines = Some(on);
                }
                if let Some(on) = page_break_before {
                    props.page_break_before = Some(on);
                }
                if let Some(on) = widow_control {
                    props.widow_control = Some(on);
                }
            }
            for (tag, value) in [
                ("w:keepNext", keep_next),
                ("w:keepLines", keep_lines),
                ("w:pageBreakBefore", page_break_before),
                ("w:widowControl", widow_control),
            ] {
                if let Some(on) = value {
                    sync_paragraph_toggle_xml(ctx.document, index, tag, on);
                }
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

    fn paragraph_xml(document: &WordDocument) -> String {
        document
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml")
    }

    #[test]
    fn options_are_written_on_and_off_and_undone() {
        let mut document = fixture();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let before = paragraph_xml(&document);
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphPaginationCommand::new(
                        1,
                        2,
                        Pagination {
                            keep_next: Some(true),
                            keep_lines: Some(true),
                            page_break_before: Some(true),
                            widow_control: Some(false),
                        },
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        let props = document.paragraph(2).unwrap().properties.clone().unwrap();
        assert_eq!(props.keep_next, Some(true));
        assert_eq!(props.keep_lines, Some(true));
        assert_eq!(props.page_break_before, Some(true));
        assert_eq!(props.widow_control, Some(false));
        let xml = paragraph_xml(&document);
        assert!(xml.contains("<w:keepNext/>"), "{xml}");
        assert!(xml.contains("<w:keepLines/>"));
        assert!(xml.contains("<w:pageBreakBefore/>"));
        assert!(xml.contains(r#"<w:widowControl w:val="0"/>"#));
        // In CT_PPrBase order: after w:pStyle, before w:spacing and w:jc.
        let ppr = &xml[xml.find("<w:keepNext/>").unwrap()..];
        let ppr = &ppr[..ppr.find("</w:pPr>").unwrap()];
        assert!(!ppr.contains("<w:pStyle"), "{ppr}");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor.undo(&mut ctx).unwrap();
        }
        assert_eq!(paragraph_xml(&document), before);
    }
}
