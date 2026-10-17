//! Paragraph line spacing command (Word のホームタブ「行間」).
//!
//! Sets `<w:spacing w:line w:lineRule="auto">` on a paragraph range. `line` is
//! the multiplier (1.0 = single, 1.5, 2.0). The model stores the multiplier in
//! `Spacing.line` (auto rule), matching how the parser/renderer interpret it.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_spacing_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// Applies a line-spacing multiplier to a contiguous paragraph range,
/// supporting undo by restoring each paragraph's previous properties.
pub struct SetParagraphLineSpacingCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    /// Line height: `None` leaves it as it is, `Some(None)` clears it, and
    /// `Some(Some(v))` sets it.
    line: Option<Option<f32>>,
    /// How `line` is read: `auto` (a multiplier), `exact` or `atLeast`
    /// (points). `None` means `auto`.
    line_rule: Option<String>,
    /// Space above the paragraph in points; `None` leaves it alone.
    before: Option<f32>,
    /// Space below the paragraph in points; `None` leaves it alone.
    after: Option<f32>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetParagraphLineSpacingCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, line: Option<f32>) -> Self {
        // The historical form always writes the line height, clearing it with
        // `None`.
        Self::with_spacing(start_paragraph, end_paragraph, Some(line), None, None, None)
    }

    /// The full `w:spacing` form: line height plus the space above and below.
    /// Each `None` leaves that part of the paragraph's spacing as it is, so a
    /// caller can adjust one without restating the rest.
    pub fn with_spacing(
        start_paragraph: usize,
        end_paragraph: usize,
        line: Option<Option<f32>>,
        line_rule: Option<String>,
        before: Option<f32>,
        after: Option<f32>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            line,
            line_rule,
            before,
            after,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
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

impl Command for SetParagraphLineSpacingCommand {
    fn name(&self) -> &'static str {
        "set_paragraph_line_spacing"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(Some(value)) = self.line {
            if !value.is_finite() || value <= 0.0 {
                return Err(CommandError::Custom(
                    "line spacing must be a positive number".into(),
                ));
            }
        }
        for (name, value) in [("before", self.before), ("after", self.after)] {
            if let Some(value) = value {
                if !value.is_finite() || value < 0.0 {
                    return Err(CommandError::Custom(format!(
                        "paragraph spacing {name} must be a non-negative number"
                    )));
                }
            }
        }
        if let Some(rule) = self.line_rule.as_deref() {
            if !matches!(rule, "auto" | "exact" | "exactly" | "atLeast") {
                return Err(CommandError::Custom(format!(
                    "line rule must be auto, exact or atLeast, not {rule:?}"
                )));
            }
        }

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

            let current_spacing = ctx
                .document
                .paragraph(paragraph_index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.as_ref());
            let line_changed = self.line.is_some_and(|line| {
                current_spacing.and_then(|spacing| spacing.line) != line
                    || current_spacing.and_then(|spacing| spacing.line_rule.as_deref())
                        != line.map(|_| self.line_rule.as_deref().unwrap_or("auto"))
            });
            let rule_changed = self.line.is_none()
                && self.line_rule.as_deref().is_some_and(|rule| {
                    current_spacing.and_then(|spacing| spacing.line).is_some()
                        && current_spacing.and_then(|spacing| spacing.line_rule.as_deref())
                            != Some(rule)
                });
            let before_changed = self.before.is_some_and(|before| {
                current_spacing.and_then(|spacing| spacing.before) != Some(before)
                    || current_spacing
                        .and_then(|spacing| spacing.before_lines)
                        .is_some()
            });
            let after_changed = self.after.is_some_and(|after| {
                current_spacing.and_then(|spacing| spacing.after) != Some(after)
                    || current_spacing
                        .and_then(|spacing| spacing.after_lines)
                        .is_some()
            });
            let paragraph_changed = line_changed || rule_changed || before_changed || after_changed;
            self.changed |= paragraph_changed;
            if !paragraph_changed {
                continue;
            }

            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                let spacing = props.spacing.get_or_insert_with(Default::default);
                if let Some(line) = self.line {
                    spacing.line = line;
                    spacing.line_rule =
                        line.map(|_| self.line_rule.clone().unwrap_or_else(|| "auto".to_string()));
                } else if let Some(rule) = self.line_rule.clone() {
                    // A rule on its own re-reads the existing line height.
                    spacing.line_rule = spacing.line.map(|_| rule);
                }
                if self.before.is_some() {
                    spacing.before = self.before;
                    spacing.before_lines = None;
                }
                if self.after.is_some() {
                    spacing.after = self.after;
                    spacing.after_lines = None;
                }
            }

            let spacing = ctx
                .document
                .paragraph(paragraph_index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.spacing.as_ref())
                .cloned();
            sync_paragraph_spacing_xml(ctx.document, paragraph_index, spacing.as_ref());
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
    use crate::commands::set_paragraph_visuals::SetParagraphVisualsCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run, Spacing};
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNode};
    use proptest::prelude::*;

    fn sample_document() -> WordDocument {
        WordDocument::from_document(Document {
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
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    fn line_of(document: &WordDocument) -> Option<f32> {
        document
            .paragraph(0)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.spacing.as_ref())
            .and_then(|s| s.line)
    }

    #[test]
    fn applies_and_undoes_line_spacing() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphLineSpacingCommand::new(0, 0, Some(1.5))),
                    &mut ctx,
                )
                .expect("set line spacing");
        }
        assert_eq!(line_of(&document), Some(1.5));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(line_of(&document), None);
    }

    #[test]
    fn rejects_non_positive_spacing() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor
            .execute(
                Box::new(SetParagraphLineSpacingCommand::new(0, 0, Some(0.0))),
                &mut ctx,
            )
            .is_err());
    }

    #[test]
    fn choosing_the_current_line_spacing_does_not_consume_undo() {
        let mut document = sample_document();
        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            SetParagraphLineSpacingCommand::new(0, 0, Some(1.5))
                .apply(&mut ctx)
                .expect("establish line spacing");
        }

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
                    Box::new(SetParagraphLineSpacingCommand::new(0, 0, Some(1.5))),
                    &mut ctx,
                )
                .expect("reapply line spacing");
            assert!(executor.undo(&mut ctx).expect("undo shading"));
        }

        assert!(document
            .paragraph(0)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.shading.as_ref())
            .is_none());
    }

    #[test]
    fn undo_preserves_exact_line_rule_in_raw_xml() {
        let mut document = sample_document();
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:spacing w:line="480" w:lineRule="exact"/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
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
        document.raw_document = Some(raw);
        let paragraph = document.paragraph_mut(0).expect("paragraph");
        paragraph.node_id = Some(paragraph_id);
        paragraph
            .properties
            .get_or_insert_with(Default::default)
            .spacing = Some(Spacing {
            line: Some(24.0),
            line_rule: Some("exact".to_string()),
            ..Spacing::default()
        });

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphLineSpacingCommand::new(0, 0, Some(1.5))),
                    &mut ctx,
                )
                .expect("apply");
            assert!(executor.undo(&mut ctx).expect("undo"));
        }

        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:line=\"480\""), "{xml}");
        assert!(xml.contains("w:lineRule=\"exact\""), "{xml}");
    }

    #[test]
    fn point_spacing_replaces_line_based_spacing_and_undo_restores_it() {
        for point_fallback in [false, true] {
            let mut document = sample_document();
            let raw_xml = format!(
                r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:spacing w:beforeLines="50" w:afterLines="75" {}/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
                if point_fallback {
                    r#"w:before="600" w:after="800""#
                } else {
                    ""
                }
            );
            let raw = DocxXmlDocument::parse(&raw_xml).expect("raw document");
            let paragraph_id = raw
                .root()
                .children
                .iter()
                .find_map(|node| match node {
                    XmlNode::Element(body) => body.children.iter().find_map(|node| match node {
                        XmlNode::Element(paragraph) if paragraph.name == "w:p" => {
                            Some(paragraph.id)
                        }
                        _ => None,
                    }),
                    _ => None,
                })
                .expect("paragraph id");
            document.raw_document = Some(raw);
            let paragraph = document.paragraph_mut(0).expect("paragraph");
            paragraph.node_id = Some(paragraph_id);
            paragraph
                .properties
                .get_or_insert_with(Default::default)
                .spacing = Some(Spacing {
                before: point_fallback.then_some(30.0),
                after: point_fallback.then_some(40.0),
                before_lines: Some(0.5),
                after_lines: Some(0.75),
                ..Spacing::default()
            });

            let mut selection = None;
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor
                    .execute(
                        Box::new(SetParagraphLineSpacingCommand::with_spacing(
                            0,
                            0,
                            None,
                            None,
                            Some(12.0),
                            Some(18.0),
                        )),
                        &mut ctx,
                    )
                    .expect("apply point spacing");
            }

            let spacing = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.as_ref())
                .expect("spacing");
            assert_eq!(spacing.before, Some(12.0));
            assert_eq!(spacing.after, Some(18.0));
            assert_eq!(spacing.before_lines, None);
            assert_eq!(spacing.after_lines, None);
            let xml = document
                .raw_document
                .as_ref()
                .expect("raw")
                .to_string()
                .expect("serialize");
            assert!(xml.contains("w:before=\"240\""), "{xml}");
            assert!(xml.contains("w:after=\"360\""), "{xml}");
            assert!(!xml.contains("w:beforeLines"), "{xml}");
            assert!(!xml.contains("w:afterLines"), "{xml}");

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                assert!(executor.undo(&mut ctx).expect("undo"));
            }
            let restored = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.as_ref())
                .expect("restored spacing");
            assert_eq!(restored.before, point_fallback.then_some(30.0));
            assert_eq!(restored.after, point_fallback.then_some(40.0));
            assert_eq!(restored.before_lines, Some(0.5));
            assert_eq!(restored.after_lines, Some(0.75));
            let xml = document
                .raw_document
                .as_ref()
                .expect("raw")
                .to_string()
                .expect("serialize");
            assert!(xml.contains("w:beforeLines=\"50\""), "{xml}");
            assert!(xml.contains("w:afterLines=\"75\""), "{xml}");
            assert_eq!(xml.contains("w:before=\"600\""), point_fallback, "{xml}");
            assert_eq!(xml.contains("w:after=\"800\""), point_fallback, "{xml}");
        }
    }

    #[test]
    fn point_spacing_replaces_word_auto_spacing_and_undo_restores_it() {
        let mut document = sample_document();
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:spacing w:beforeAutospacing="1" w:afterAutospacing="true"/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
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
        document.raw_document = Some(raw);
        document.paragraph_mut(0).expect("paragraph").node_id = Some(paragraph_id);

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphLineSpacingCommand::with_spacing(
                        0,
                        0,
                        None,
                        None,
                        Some(12.0),
                        Some(18.0),
                    )),
                    &mut ctx,
                )
                .expect("apply point spacing");
        }
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:before=\"240\""), "{xml}");
        assert!(xml.contains("w:after=\"360\""), "{xml}");
        assert!(!xml.contains("w:beforeAutospacing"), "{xml}");
        assert!(!xml.contains("w:afterAutospacing"), "{xml}");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:beforeAutospacing=\"1\""), "{xml}");
        assert!(xml.contains("w:afterAutospacing=\"true\""), "{xml}");
        assert!(!xml.contains("w:before=\""), "{xml}");
        assert!(!xml.contains("w:after=\""), "{xml}");
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn paragraph_before_after_spacing_preserves_line_height_and_undoes(
            before in 0.0_f32..=400.0,
            after in 0.0_f32..=400.0,
            line in 0.5_f32..=8.0,
        ) {
            let mut document = sample_document();
            document
                .paragraph_mut(0)
                .expect("paragraph")
                .properties
                .get_or_insert_with(Default::default)
                .spacing = Some(Spacing {
                line: Some(line),
                line_rule: Some("auto".to_string()),
                ..Spacing::default()
            });
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(SetParagraphLineSpacingCommand::with_spacing(
                        0,
                        0,
                        None,
                        None,
                        Some(before),
                        Some(after),
                    )),
                    &mut ctx,
                ).expect("set paragraph spacing");
            }

            let spacing = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.as_ref())
                .expect("spacing");
            prop_assert_eq!(spacing.before, Some(before));
            prop_assert_eq!(spacing.after, Some(after));
            prop_assert_eq!(spacing.line, Some(line));
            prop_assert_eq!(spacing.line_rule.as_deref(), Some("auto"));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo"));
            }
            let restored = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.as_ref())
                .expect("restored spacing");
            prop_assert_eq!(restored.before, None);
            prop_assert_eq!(restored.after, None);
            prop_assert_eq!(restored.line, Some(line));
            prop_assert_eq!(restored.line_rule.as_deref(), Some("auto"));
        }
    }
}
