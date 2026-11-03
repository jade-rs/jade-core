//! 位置合わせタブの挿入: an absolute position tab (`w:ptab`, ECMA-376 Part 1
//! §17.3.3.16) at the caret — a tab character of the text that aligns what
//! follows it to the left, the centre or the right of the margins (or of the
//! paragraph's indents), whatever the tab stops say, with its leader.

use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{ParagraphProperties, PositionalTab};
use crate::docx_xml::{XmlAttribute, XmlElement, XmlNode};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts one positional tab.
pub struct InsertPositionalTabCommand {
    position: TextPosition,
    tab: PositionalTab,
    insert: Option<InsertTextCommand>,
    previous_properties: Option<Option<ParagraphProperties>>,
}

impl InsertPositionalTabCommand {
    /// `tab.ordinal` is worked out where it goes.
    pub fn new(paragraph: usize, offset: usize, tab: PositionalTab) -> Self {
        Self {
            position: TextPosition::new(paragraph, offset),
            tab,
            insert: None,
            previous_properties: None,
        }
    }
}

/// Turns the paragraph's `ordinal`-th run tab (`w:tab` or `w:ptab` inside a
/// `w:r`, not a `w:tabs` stop) into `replacement`. Whether one was found.
fn replace_run_tab(
    element: &mut XmlElement,
    ordinal: usize,
    seen: &mut usize,
    replacement: &dyn Fn(&XmlElement) -> XmlElement,
) -> bool {
    let in_run = element.name == "w:r";
    for child in &mut element.children {
        let XmlNode::Element(child) = child else {
            continue;
        };
        if in_run && (child.name == "w:tab" || child.name == "w:ptab") {
            if *seen == ordinal {
                *child = replacement(child);
                return true;
            }
            *seen += 1;
            continue;
        }
        // Paragraph properties hold tab stops, not tab characters.
        if child.name == "w:pPr" || child.name == "w:rPr" {
            continue;
        }
        if replace_run_tab(child, ordinal, seen, replacement) {
            return true;
        }
    }
    false
}

impl Command for InsertPositionalTabCommand {
    fn name(&self) -> &'static str {
        "insert_positional_tab"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let index = self.position.paragraph;
        let paragraph = ctx
            .document
            .paragraph(index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let Some(node_id) = paragraph.node_id else {
            return Err(CommandError::Custom(
                "a positional tab goes in the body, a header or a footer".into(),
            ));
        };
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        let before = text
            .get(..self.position.offset)
            .ok_or(CommandError::Custom(
                "the offset is not in the paragraph".into(),
            ))?;
        let ordinal = before.matches('\t').count();
        self.previous_properties = Some(paragraph.properties.clone());

        let mut insert = InsertTextCommand::new(
            SelectionRange::collapsed(self.position),
            "\t".to_string(),
            None,
        );
        insert.apply(ctx)?;
        self.insert = Some(insert);

        let tab = PositionalTab {
            ordinal,
            ..self.tab.clone()
        };
        if let Some(raw) = ctx.document.raw_document.as_mut() {
            let attributes: Vec<XmlAttribute> = tab
                .attributes()
                .into_iter()
                .map(|(name, value)| XmlAttribute {
                    name: name.to_string(),
                    value,
                })
                .collect();
            let replacement = |old: &XmlElement| XmlElement {
                id: old.id,
                name: "w:ptab".to_string(),
                attributes: attributes.clone(),
                children: Vec::new(),
            };
            let found = raw
                .element_mut(node_id)
                .is_some_and(|element| replace_run_tab(element, ordinal, &mut 0, &replacement));
            raw.rebuild_index_for_subtree(node_id);
            if !found {
                return Err(CommandError::Custom("the new tab was not written".into()));
            }
        }
        let props = ctx
            .document
            .paragraph_mut(index)
            .ok_or(CommandError::ParagraphUnavailable)?
            .properties
            .get_or_insert_with(Default::default);
        for existing in &mut props.positional_tabs {
            if existing.ordinal >= ordinal {
                existing.ordinal += 1;
            }
        }
        let at = props
            .positional_tabs
            .iter()
            .position(|existing| existing.ordinal > ordinal)
            .unwrap_or(props.positional_tabs.len());
        props.positional_tabs.insert(at, tab);
        *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
            index,
            self.position.offset + 1,
        )));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(previous) = self.previous_properties.take() {
            if let Some(paragraph) = ctx.document.paragraph_mut(self.position.paragraph) {
                paragraph.properties = previous;
            }
        }
        if let Some(mut insert) = self.insert.take() {
            insert.undo(ctx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::document::PositionalTabAlignment;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;

    fn fixture() -> WordDocument {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contract.docx");
        let data = std::fs::read(path).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn xml(document: &WordDocument) -> String {
        document
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml")
    }

    #[test]
    fn a_positional_tab_is_written_read_and_undone() {
        let mut document = fixture();
        let paragraph = (0..document.paragraph_count())
            .find(|&index| {
                document.is_body_paragraph_surface(index)
                    && document
                        .paragraph(index)
                        .is_some_and(|p| p.runs.iter().map(|r| r.text.len()).sum::<usize>() > 6)
            })
            .expect("a body paragraph with text");
        let before = xml(&document);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertPositionalTabCommand::new(
                        paragraph,
                        0,
                        PositionalTab {
                            ordinal: 0,
                            alignment: PositionalTabAlignment::Right,
                            relative_to_indent: false,
                            leader: Some("dot".to_string()),
                        },
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        let after = xml(&document);
        assert!(
            after.contains(r#"<w:ptab w:relativeTo="margin" w:alignment="right" w:leader="dot"/>"#),
            "{after}"
        );
        let props = document
            .paragraph(paragraph)
            .unwrap()
            .properties
            .clone()
            .unwrap();
        assert_eq!(props.positional_tabs.len(), 1);
        assert_eq!(props.positional_tabs[0].ordinal, 0);
        let text: String = document
            .paragraph(paragraph)
            .unwrap()
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        assert!(text.starts_with('\t'));

        // Reloaded, the reader finds the same tab.
        let saved = document.export_docx().expect("export");
        let reloaded =
            executor::block_on(WordDocument::load(&saved, &Options::default())).expect("reload");
        let tabs = &reloaded
            .paragraph(paragraph)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .positional_tabs;
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].alignment, PositionalTabAlignment::Right);

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        // Undo rewrites the paragraph's runs, which may now say
        // xml:space="preserve"; the text and properties are as they were.
        let plain = |xml: String| xml.replace(r#" xml:space="preserve""#, "");
        assert_eq!(plain(xml(&document)), plain(before));
    }
}
