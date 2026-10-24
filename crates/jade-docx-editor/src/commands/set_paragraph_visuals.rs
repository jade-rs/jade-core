use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
    sync_paragraph_borders_xml, sync_paragraph_shading_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{ParagraphBorders, ParagraphProperties, Shading};
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// Applies one paragraph-level visual property to a contiguous paragraph range.
pub struct SetParagraphVisualsCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    shading: Option<Option<Shading>>,
    borders: Option<Option<ParagraphBorders>>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetParagraphVisualsCommand {
    pub fn shading(start_paragraph: usize, end_paragraph: usize, color: Option<String>) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            shading: Some(color.map(|fill| Shading {
                val: Some("clear".to_string()),
                color: None,
                fill: Some(fill),
            })),
            borders: None,
            previous_properties: Vec::new(),
            previous_selection: None,
            changed: false,
        }
    }

    pub fn borders(
        start_paragraph: usize,
        end_paragraph: usize,
        borders: Option<ParagraphBorders>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            shading: None,
            borders: Some(borders),
            previous_properties: Vec::new(),
            previous_selection: None,
            changed: false,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }
}

impl Command for SetParagraphVisualsCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "set_paragraph_visuals"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.shading.is_none() && self.borders.is_none() {
            return Err(CommandError::Custom(
                "set_paragraph_visuals requires shading or borders".into(),
            ));
        }
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.changed = false;
        self.previous_selection = *ctx.selection;
        for paragraph_index in start..=end {
            let previous_xml = snapshot_paragraph_properties_xml(ctx.document, paragraph_index);
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties
                .push((paragraph_index, previous, previous_xml));

            let (shading, borders, shading_changed, borders_changed) = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let current_shading = paragraph
                    .properties
                    .as_ref()
                    .and_then(|properties| properties.shading.clone());
                let current_borders = paragraph
                    .properties
                    .as_ref()
                    .and_then(|properties| properties.borders.clone());
                let shading_changed = self
                    .shading
                    .as_ref()
                    .is_some_and(|shading| current_shading != *shading);
                let borders_changed = self
                    .borders
                    .as_ref()
                    .is_some_and(|borders| current_borders != *borders);
                let next_shading = self.shading.clone().unwrap_or(current_shading);
                let next_borders = self.borders.clone().unwrap_or(current_borders);
                if shading_changed || borders_changed {
                    let properties = paragraph.properties.get_or_insert_with(Default::default);
                    if shading_changed {
                        properties.shading = self.shading.clone().flatten();
                    }
                    if borders_changed {
                        properties.borders = self.borders.clone().flatten();
                    }
                }
                (next_shading, next_borders, shading_changed, borders_changed)
            };
            self.changed |= shading_changed || borders_changed;
            if shading_changed {
                sync_paragraph_shading_xml(ctx.document, paragraph_index, shading.as_ref());
            }
            if borders_changed {
                sync_paragraph_borders_xml(ctx.document, paragraph_index, borders.as_ref());
            }
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous, previous_xml) in &self.previous_properties {
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
            }
            restore_paragraph_properties_xml(ctx.document, *paragraph_index, previous_xml.as_ref());
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
    use crate::document::{Body, BodyElement, BorderEdge, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};
    use proptest::prelude::*;

    fn sample_document() -> WordDocument {
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .expect("raw document");
        let paragraph_id = raw
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) => body.children.iter().find_map(|node| match node {
                    XmlNode::Element(paragraph) if paragraph.name == "w:p" => Some(paragraph.id),
                    _ => None,
                }),
                _ => None,
            })
            .expect("paragraph id");
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
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
                    node_id: Some(paragraph_id),
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        document.raw_document = Some(raw);
        document
    }

    fn edge(color: String, size_eighths: u32) -> BorderEdge {
        BorderEdge {
            val: Some("single".to_string()),
            size_eighths: Some(size_eighths),
            space: Some(1),
            color: Some(color),
        }
    }

    #[test]
    fn clearing_an_already_absent_border_does_not_consume_undo() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphVisualsCommand::shading(
                        0,
                        0,
                        Some("#FFF2CC".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set shading");
            executor
                .execute(
                    Box::new(SetParagraphVisualsCommand::borders(0, 0, None)),
                    &mut ctx,
                )
                .expect("clear absent border");
            assert!(executor.undo(&mut ctx).expect("undo shading"));
        }
        assert!(
            document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.shading.as_ref())
                .is_none(),
            "one undo must reach the preceding real edit"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn arbitrary_shading_and_border_sides_round_trip_through_history_and_xml(
            rgb in any::<[u8; 3]>(),
            old_rgb in any::<[u8; 3]>(),
            sides in any::<[bool; 4]>(),
            size_eighths in 1_u32..=96,
        ) {
            let color = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
            let old_color = format!("#{:02X}{:02X}{:02X}", old_rgb[0], old_rgb[1], old_rgb[2]);
            let mut document = sample_document();
            let old_shading = Shading {
                val: Some("clear".to_string()),
                color: None,
                fill: Some(old_color.clone()),
            };
            let old_borders = ParagraphBorders {
                between: Some(edge(old_color.clone(), 4)),
                ..Default::default()
            };
            document.paragraph_mut(0).expect("paragraph").properties = Some(ParagraphProperties {
                shading: Some(old_shading.clone()),
                borders: Some(old_borders.clone()),
                ..Default::default()
            });
            sync_paragraph_shading_xml(&mut document, 0, Some(&old_shading));
            sync_paragraph_borders_xml(&mut document, 0, Some(&old_borders));
            let initial_xml = document
                .raw_document
                .as_ref()
                .expect("raw")
                .to_string()
                .expect("initial xml");
            let new_borders = ParagraphBorders {
                top: sides[0].then(|| edge(color.clone(), size_eighths)),
                bottom: sides[1].then(|| edge(color.clone(), size_eighths)),
                left: sides[2].then(|| edge(color.clone(), size_eighths)),
                right: sides[3].then(|| edge(color.clone(), size_eighths)),
                ..Default::default()
            };
            let new_borders = (!new_borders.is_empty()).then_some(new_borders);
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(SetParagraphVisualsCommand::shading(0, 0, Some(color.clone()))),
                    &mut ctx,
                ).expect("set shading");
                executor.execute(
                    Box::new(SetParagraphVisualsCommand::borders(0, 0, new_borders.clone())),
                    &mut ctx,
                ).expect("set borders");
            }

            let properties = document.paragraph(0).and_then(|p| p.properties.as_ref()).expect("properties");
            prop_assert_eq!(properties.shading.as_ref().and_then(|s| s.fill.as_deref()), Some(color.as_str()));
            prop_assert_eq!(&properties.borders, &new_borders);
            let xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
            let expected_fill = format!("w:fill=\"{}\"", color.trim_start_matches('#'));
            prop_assert!(xml.contains(&expected_fill));
            for (enabled, tag) in sides.into_iter().zip(["w:top", "w:bottom", "w:left", "w:right"]) {
                prop_assert_eq!(xml.contains(&format!("<{tag} ")), enabled);
            }

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo borders"));
                prop_assert!(executor.undo(&mut ctx).expect("undo shading"));
            }
            let restored = document.paragraph(0).and_then(|p| p.properties.as_ref()).expect("restored");
            prop_assert_eq!(&restored.shading, &Some(old_shading));
            prop_assert_eq!(&restored.borders, &Some(old_borders));
            let restored_xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
            prop_assert_eq!(restored_xml, initial_xml);
        }
    }
}
