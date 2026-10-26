use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_tabs_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{ParagraphProperties, TabStop};
use crate::docx_xml::XmlElement;
use jade_docx_headless_api::HeadlessDirectTabStop;
use jade_docx_layout::selection::SelectionRange;

/// Replaces explicit tab stops without changing any other paragraph formatting.
pub struct SetParagraphTabsCommand {
    start: usize,
    end: usize,
    tabs: Vec<HeadlessDirectTabStop>,
    previous: Vec<(usize, Option<ParagraphProperties>, Option<XmlElement>)>,
    selection: Option<SelectionRange>,
    changed: bool,
}

impl SetParagraphTabsCommand {
    pub fn new(start: usize, end: usize, tabs: Vec<HeadlessDirectTabStop>) -> Self {
        Self {
            start,
            end,
            tabs,
            previous: Vec::new(),
            selection: None,
            changed: false,
        }
    }
}

impl Command for SetParagraphTabsCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "set_paragraph_tabs"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for tab in &self.tabs {
            if !tab.position.is_finite()
                || !(0.0..=1584.0).contains(&tab.position)
                || !matches!(
                    tab.kind.as_str(),
                    "left" | "center" | "right" | "decimal" | "bar" | "clear"
                )
                || tab.leader.as_deref().is_some_and(|leader| {
                    !matches!(
                        leader,
                        "none" | "dot" | "hyphen" | "underscore" | "heavy" | "middleDot"
                    )
                })
            {
                return Err(CommandError::Custom("invalid paragraph tab stop".into()));
            }
        }
        let (start, end) = (self.start.min(self.end), self.start.max(self.end));
        if end >= ctx.document.paragraph_count() {
            return Err(CommandError::ParagraphUnavailable);
        }
        let mut tabs: Vec<TabStop> = self
            .tabs
            .iter()
            .map(|tab| TabStop {
                position: Some((tab.position * 20.0).round() / 20.0),
                kind: Some(tab.kind.clone()),
                leader: tab.leader.clone(),
            })
            .collect();
        tabs.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
        tabs.dedup_by(|a, b| a.position == b.position);
        self.previous.clear();
        self.selection = *ctx.selection;
        self.changed = false;
        for index in start..=end {
            let previous = ctx
                .document
                .paragraph(index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .properties
                .clone();
            let raw = snapshot_paragraph_properties_xml(ctx.document, index);
            let unchanged = previous
                .as_ref()
                .map(|p| p.tabs.as_slice())
                .unwrap_or_default()
                == tabs;
            self.previous.push((index, previous, raw));
            if unchanged {
                continue;
            }
            self.changed = true;
            ctx.document
                .paragraph_mut(index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .properties
                .get_or_insert_with(Default::default)
                .tabs = tabs.clone();
            sync_paragraph_tabs_xml(ctx.document, index, &tabs);
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (index, properties, raw) in &self.previous {
            ctx.document
                .paragraph_mut(*index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .properties = properties.clone();
            restore_paragraph_properties_xml(ctx.document, *index, raw.as_ref());
        }
        *ctx.selection = self.selection;
        Ok(())
    }
    fn should_record_history(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Body, BodyElement, Document, Paragraph};
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};

    #[test]
    fn tabs_round_trip_xml_range_and_undo_preserves_unrelated_properties() {
        let raw = DocxXmlDocument::parse(r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:keepNext/><w:tabs><w:tab w:val="right" w:pos="1440" w:leader="dot"/></w:tabs></w:pPr></w:p><w:p/></w:body></w:document>"#).unwrap();
        let body = raw
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(e) => Some(e),
                _ => None,
            })
            .unwrap();
        let elements = body
            .children
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(e) => Some(BodyElement::from_paragraph(Paragraph {
                    node_id: Some(e.id),
                    runs: Vec::new(),
                    properties: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })),
                _ => None,
            })
            .collect();
        let before = raw.to_string().unwrap();
        let mut document = WordDocument::from_document(Document {
            body: Some(Body { elements }),
            ..Document::default()
        });
        document.raw_document = Some(raw);
        let mut selection = None;
        let mut command = SetParagraphTabsCommand::new(
            1,
            0,
            vec![HeadlessDirectTabStop {
                position: 36.13,
                kind: "center".into(),
                leader: Some("dot".into()),
            }],
        );
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        command.apply(&mut ctx).unwrap();
        for i in 0..2 {
            assert_eq!(
                ctx.document
                    .paragraph(i)
                    .unwrap()
                    .properties
                    .as_ref()
                    .unwrap()
                    .tabs[0]
                    .position,
                Some(36.15)
            );
        }
        let xml = ctx
            .document
            .raw_document
            .as_ref()
            .unwrap()
            .to_string()
            .unwrap();
        assert_eq!(xml.matches("w:pos=\"723\"").count(), 2);
        assert!(xml.contains("w:keepNext"));
        assert!(xml.contains("w:leader=\"dot\""));
        command.undo(&mut ctx).unwrap();
        assert_eq!(
            ctx.document
                .raw_document
                .as_ref()
                .unwrap()
                .to_string()
                .unwrap(),
            before
        );
        assert!(ctx.document.paragraph(0).unwrap().properties.is_none());
        command.apply(&mut ctx).unwrap();
        command.apply(&mut ctx).unwrap();
        assert!(!command.should_record_history());
    }

    #[test]
    fn invalid_tabs_are_atomic() {
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: Vec::new(),
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        for (position, kind) in [(f32::NAN, "left"), (-1.0, "left"), (30.0, "bad")] {
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(SetParagraphTabsCommand::new(
                0,
                0,
                vec![HeadlessDirectTabStop {
                    position,
                    kind: kind.into(),
                    leader: None
                }]
            )
            .apply(&mut ctx)
            .is_err());
            assert!(ctx.document.paragraph(0).unwrap().properties.is_none());
        }
    }
}
