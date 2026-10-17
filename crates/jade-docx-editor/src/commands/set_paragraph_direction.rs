use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
    sync_paragraph_alignment_xml, sync_paragraph_bidi_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// Sets the base text direction for a paragraph range. Word treats direction
/// and physical alignment as separate properties; when the direction actually
/// changes, its ribbon swaps an explicit left/right alignment so the text keeps
/// following the logical start edge.
pub struct SetParagraphDirectionCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    bidi: bool,
    previous_properties: Vec<(usize, Option<ParagraphProperties>, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetParagraphDirectionCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, bidi: bool) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            bidi,
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

    fn swapped_alignment(alignment: Option<&str>, bidi: bool) -> Option<String> {
        alignment.map(|value| match (value, bidi) {
            ("left", true) => "right".to_string(),
            ("right", false) => "left".to_string(),
            _ => value.to_string(),
        })
    }
}

impl Command for SetParagraphDirectionCommand {
    fn name(&self) -> &'static str {
        "set_paragraph_direction"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.previous_selection = *ctx.selection;
        self.changed = false;
        for paragraph_index in start..=end {
            let previous_xml = snapshot_paragraph_properties_xml(ctx.document, paragraph_index);
            let previous = ctx
                .document
                .paragraph(paragraph_index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties
                .push((paragraph_index, previous, previous_xml));

            let direction_changed = ctx
                .document
                .paragraph(paragraph_index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.bidi)
                .unwrap_or(false)
                != self.bidi;
            if !direction_changed {
                continue;
            }
            self.changed = true;

            let alignment = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let properties = paragraph.properties.get_or_insert_with(Default::default);
                properties.alignment =
                    Self::swapped_alignment(properties.alignment.as_deref(), self.bidi)
                        .map(Into::into);
                properties.bidi = Some(self.bidi);
                properties.alignment.clone()
            };

            sync_paragraph_bidi_xml(ctx.document, paragraph_index, Some(self.bidi));
            sync_paragraph_alignment_xml(ctx.document, paragraph_index, alignment.as_deref());
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (paragraph_index, previous, previous_xml) in &self.previous_properties {
            let paragraph = ctx
                .document
                .paragraph_mut(*paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            paragraph.properties = previous.clone();
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
    use crate::commands::set_paragraph_visuals::SetParagraphVisualsCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};
    use proptest::prelude::*;

    fn run(text: &str) -> Run {
        Run {
            text: text.to_string(),
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
        }
    }

    fn sample_document(bidi: Option<bool>, alignment: Option<&str>, marker: u16) -> WordDocument {
        let bidi_xml = bidi
            .map(|value| format!(r#"<w:bidi w:val="{}"/>"#, u8::from(value)))
            .unwrap_or_default();
        let alignment_xml = alignment
            .map(|value| format!(r#"<w:jc w:val="{value}"/>"#))
            .unwrap_or_default();
        let raw = DocxXmlDocument::parse(&format!(
            r#"<w:document xmlns:w="urn:w"><w:body>
<w:p><w:pPr data-marker="{marker}"><w:keepNext/>{bidi_xml}{alignment_xml}<w:ext w:val="preserve"/></w:pPr><w:r><w:t>שלום one</w:t></w:r></w:p>
<w:p><w:pPr data-marker="{marker}"><w:keepNext/>{bidi_xml}{alignment_xml}<w:ext w:val="preserve"/></w:pPr><w:r><w:t>مرحبا two</w:t></w:r></w:p>
</w:body></w:document>"#,
        ))
        .expect("raw document");
        let paragraph_ids = raw
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) => Some(
                    body.children
                        .iter()
                        .filter_map(|node| match node {
                            XmlNode::Element(paragraph) if paragraph.name == "w:p" => {
                                Some(paragraph.id)
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .expect("body");
        let properties = Some(ParagraphProperties {
            alignment: alignment.map(Into::into),
            bidi,
            ..ParagraphProperties::default()
        });
        let mut document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![run("שלום one")],
                        properties: properties.clone(),
                        node_id: Some(paragraph_ids[0]),
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                    BodyElement::from_paragraph(Paragraph {
                        runs: vec![run("مرحبا two")],
                        properties,
                        node_id: Some(paragraph_ids[1]),
                        comment_ranges: Vec::new(),
                        comment_markers: Vec::new(),
                    }),
                ],
            }),
            ..Document::default()
        });
        document.raw_document = Some(raw);
        document
    }

    fn alignment_for_case(case: u8) -> Option<&'static str> {
        match case {
            1 => Some("left"),
            2 => Some("right"),
            3 => Some("center"),
            4 => Some("justify"),
            5 => Some("start"),
            6 => Some("end"),
            _ => None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn direction_histories_preserve_unknown_xml_and_swap_only_physical_edges(
            previous_case in 0_u8..3,
            alignment_case in 0_u8..7,
            target_bidi in any::<bool>(),
            reversed in any::<bool>(),
            marker in any::<u16>(),
        ) {
            let previous_bidi = match previous_case {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            };
            let alignment = alignment_for_case(alignment_case);
            let mut document = sample_document(previous_bidi, alignment, marker);
            let initial_xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
            let direction_changed = previous_bidi.unwrap_or(false) != target_bidi;
            let expected_bidi = if direction_changed {
                Some(target_bidi)
            } else {
                previous_bidi
            };
            let expected_alignment = if direction_changed {
                SetParagraphDirectionCommand::swapped_alignment(alignment, target_bidi)
            } else {
                alignment.map(str::to_string)
            };
            let (start, end) = if reversed { (1, 0) } else { (0, 1) };
            let mut executor = CommandExecutor::new();
            let mut selection = None;

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(SetParagraphDirectionCommand::new(start, end, target_bidi)),
                    &mut ctx,
                ).expect("apply direction");
            }
            for index in 0..2 {
                let props = document.paragraph(index).expect("paragraph").properties.as_ref().expect("props");
                prop_assert_eq!(props.bidi, expected_bidi);
                prop_assert_eq!(props.alignment.as_deref(), expected_alignment.as_deref());
            }
            let applied_xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
            let marker_attribute = format!("data-marker=\"{}\"", marker);
            prop_assert!(applied_xml.contains(&marker_attribute));
            prop_assert_eq!(applied_xml.matches("w:ext").count(), 2);
            if let Some(expected_bidi) = expected_bidi {
                let bidi_attribute = format!("w:bidi w:val=\"{}\"", u8::from(expected_bidi));
                prop_assert_eq!(applied_xml.matches(&bidi_attribute).count(), 2);
            } else {
                prop_assert_eq!(applied_xml.matches("w:bidi").count(), 0);
            }

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert_eq!(executor.undo(&mut ctx).expect("undo"), direction_changed);
            }
            let undone_xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
            prop_assert_eq!(undone_xml, initial_xml.clone());

            if direction_changed {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo"));
                let redone_xml = document.raw_document.as_ref().expect("raw").to_string().expect("xml");
                prop_assert_eq!(redone_xml, applied_xml);
            }
        }
    }

    #[test]
    fn logical_alignment_survives_direction_changes() {
        for alignment in ["start", "end"] {
            let mut document = sample_document(Some(false), Some(alignment), 7);
            let mut executor = CommandExecutor::new();
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphDirectionCommand::new(0, 1, true)),
                    &mut ctx,
                )
                .expect("set rtl");
            for index in 0..2 {
                assert_eq!(
                    document
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .and_then(|properties| properties.alignment.as_deref()),
                    Some(alignment),
                );
            }
        }
    }

    #[test]
    fn choosing_the_current_direction_does_not_consume_undo() {
        let mut document = sample_document(Some(true), Some("start"), 8);
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphVisualsCommand::shading(
                        0,
                        1,
                        Some("#FFF2CC".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set shading");
            executor
                .execute(
                    Box::new(SetParagraphDirectionCommand::new(0, 1, true)),
                    &mut ctx,
                )
                .expect("reapply rtl");
            assert!(executor.undo(&mut ctx).expect("undo shading"));
        }
        for index in 0..2 {
            assert!(document
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.shading.as_ref())
                .is_none());
        }
    }
}
