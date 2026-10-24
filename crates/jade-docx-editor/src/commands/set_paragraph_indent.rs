use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_indent_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// Applies paragraph indentation values to a contiguous paragraph range.
pub struct SetParagraphIndentCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    left: Option<f32>,
    right: Option<f32>,
    first_line: Option<f32>,
    hanging: Option<f32>,
    left_chars: Option<i32>,
    right_chars: Option<i32>,
    /// Word's paragraph dialog 特殊インデント「なし」: remove first-line and
    /// hanging indents (character twins included) while leaving the left and
    /// right edges alone.
    clear_special: bool,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_raw_properties: Vec<(usize, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphIndentCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        left: Option<f32>,
        right: Option<f32>,
        first_line: Option<f32>,
        hanging: Option<f32>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            left,
            right,
            first_line,
            hanging,
            left_chars: None,
            right_chars: None,
            clear_special: false,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
        }
    }

    /// Creates a command that removes any first-line or hanging indent
    /// (Word's 特殊インデント「なし」), leaving left/right untouched.
    pub fn clearing_special_indent(start_paragraph: usize, end_paragraph: usize) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            left: None,
            right: None,
            first_line: None,
            hanging: None,
            left_chars: None,
            right_chars: None,
            clear_special: true,
            previous_properties: Vec::new(),
            previous_raw_properties: Vec::new(),
            previous_selection: None,
        }
    }

    /// Also removes any first-line/hanging indent (特殊インデント「なし」)
    /// alongside the left/right values this command sets.
    pub fn with_clear_special(mut self, clear_special: bool) -> Self {
        self.clear_special = clear_special;
        self
    }

    /// Creates a command that uses character-based indentation.
    pub fn with_chars(
        start_paragraph: usize,
        end_paragraph: usize,
        left_chars: Option<i32>,
        right_chars: Option<i32>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            left: None,
            right: None,
            first_line: None,
            hanging: None,
            left_chars,
            right_chars,
            clear_special: false,
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

    fn validate(&self) -> Result<(), CommandError> {
        if self.left.is_none()
            && self.right.is_none()
            && self.first_line.is_none()
            && self.hanging.is_none()
            && self.left_chars.is_none()
            && self.right_chars.is_none()
            && !self.clear_special
        {
            return Err(CommandError::Custom(
                "set_paragraph_indent requires at least one indent field".into(),
            ));
        }

        for value in [self.left, self.right, self.first_line, self.hanging]
            .into_iter()
            .flatten()
        {
            if !value.is_finite() {
                return Err(CommandError::Custom(
                    "indent values must be finite numbers".into(),
                ));
            }
        }

        Ok(())
    }
}

impl Command for SetParagraphIndentCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "set_paragraph_indent"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.validate()?;

        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_properties.clear();
        self.previous_raw_properties.clear();
        self.previous_selection = *ctx.selection;

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

            let applied_indent = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;

                let props = paragraph.properties.get_or_insert_with(Default::default);
                let bidi = props.bidi.unwrap_or(false);
                let mut indent = props.indent.clone().unwrap_or_default();
                if let Some(value) = self.left {
                    indent.left = Some(value);
                    // Point and character units are alternative encodings of
                    // the same edge; layout prefers character units when both
                    // remain, so discard the stale value.
                    indent.left_chars = None;
                    if bidi {
                        indent.end = None;
                        indent.end_chars = None;
                    } else {
                        indent.start = None;
                        indent.start_chars = None;
                    }
                }
                if let Some(value) = self.right {
                    indent.right = Some(value);
                    indent.right_chars = None;
                    if bidi {
                        indent.start = None;
                        indent.start_chars = None;
                    } else {
                        indent.end = None;
                        indent.end_chars = None;
                    }
                }
                if let Some(value) = self.first_line {
                    indent.first_line = Some(value);
                    // Word's paragraph dialog treats first-line and hanging as
                    // one mutually exclusive control, and character twins
                    // supersede point values in Word, so both must go or the
                    // edit is invisible when the file is reopened there.
                    indent.first_line_chars = None;
                    indent.hanging = None;
                    indent.hanging_chars = None;
                }
                if let Some(value) = self.hanging {
                    indent.hanging = Some(value);
                    indent.hanging_chars = None;
                    if self.first_line.is_none() {
                        indent.first_line = None;
                        indent.first_line_chars = None;
                    }
                }
                if let Some(value) = self.left_chars {
                    indent.left_chars = Some(value);
                    // When character-based indent is specified, clear the
                    // point-based value so the layout engine uses left_chars.
                    if self.left.is_none() {
                        indent.left = None;
                    }
                    if bidi {
                        indent.end = None;
                        indent.end_chars = None;
                    } else {
                        indent.start = None;
                        indent.start_chars = None;
                    }
                }
                if let Some(value) = self.right_chars {
                    indent.right_chars = Some(value);
                    if self.right.is_none() {
                        indent.right = None;
                    }
                    if bidi {
                        indent.start = None;
                        indent.start_chars = None;
                    } else {
                        indent.end = None;
                        indent.end_chars = None;
                    }
                }
                if self.clear_special {
                    indent.first_line = None;
                    indent.first_line_chars = None;
                    indent.hanging = None;
                    indent.hanging_chars = None;
                }
                props.indent = Some(indent.clone());
                indent
            };

            sync_paragraph_indent_xml(
                ctx.document,
                paragraph_index,
                Some(&applied_indent),
                self.left.is_some() || self.left_chars.is_some(),
                self.right.is_some() || self.right_chars.is_some(),
            );
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
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

    #[test]
    fn applies_left_indent() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphIndentCommand::new(
                    0,
                    0,
                    Some(12.0),
                    None,
                    None,
                    None,
                )),
                &mut ctx,
            )
            .expect("set paragraph indent");

        let paragraph = document.paragraph(0).expect("paragraph exists");
        let indent = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.indent.as_ref())
            .expect("indent");
        assert_eq!(indent.left, Some(12.0));
    }

    #[test]
    fn left_chars_clears_point_based_left() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);

        // First set a point-based left indent.
        executor
            .execute(
                Box::new(SetParagraphIndentCommand::new(
                    0,
                    0,
                    Some(36.0),
                    None,
                    None,
                    None,
                )),
                &mut ctx,
            )
            .expect("set point indent");

        // Now set a character-based indent; this should clear left.
        executor
            .execute(
                Box::new(SetParagraphIndentCommand::with_chars(0, 0, Some(100), None)),
                &mut ctx,
            )
            .expect("set char indent");

        let paragraph = document.paragraph(0).expect("paragraph exists");
        let indent = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.indent.as_ref())
            .expect("indent");
        assert_eq!(indent.left, None, "point-based left should be cleared");
        assert_eq!(indent.left_chars, Some(100));
    }

    #[test]
    fn point_indents_replace_logical_edges_and_undo_restores_them() {
        let mut document = sample_document();
        let raw = DocxXmlDocument::parse(
            r#"<w:document xmlns:w="urn:w"><w:body><w:p><w:pPr><w:ind w:start="720" w:end="360" w:startChars="200" w:endChars="100"/></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
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
                    Box::new(SetParagraphIndentCommand::new(
                        0,
                        0,
                        Some(18.0),
                        Some(12.0),
                        None,
                        None,
                    )),
                    &mut ctx,
                )
                .expect("apply point indents");
        }
        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:left=\"360\""), "{xml}");
        assert!(xml.contains("w:right=\"240\""), "{xml}");
        assert!(!xml.contains("w:start=\""), "{xml}");
        assert!(!xml.contains("w:end=\""), "{xml}");
        assert!(!xml.contains("w:startChars=\""), "{xml}");
        assert!(!xml.contains("w:endChars=\""), "{xml}");

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
        assert!(xml.contains("w:start=\"720\""), "{xml}");
        assert!(xml.contains("w:end=\"360\""), "{xml}");
        assert!(xml.contains("w:startChars=\"200\""), "{xml}");
        assert!(xml.contains("w:endChars=\"100\""), "{xml}");
        assert!(!xml.contains("w:left=\""), "{xml}");
        assert!(!xml.contains("w:right=\""), "{xml}");
    }

    #[test]
    fn negative_point_indents_round_trip_as_signed_word_twips() {
        let mut document = sample_document();
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
        document.raw_document = Some(raw);
        document.paragraph_mut(0).expect("paragraph").node_id = Some(paragraph_id);

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphIndentCommand::new(
                    0,
                    0,
                    Some(-12.5),
                    Some(-6.0),
                    None,
                    None,
                )),
                &mut ctx,
            )
            .expect("set negative indents");

        let xml = document
            .raw_document
            .as_ref()
            .expect("raw")
            .to_string()
            .expect("serialize");
        assert!(xml.contains("w:left=\"-250\""), "{xml}");
        assert!(xml.contains("w:right=\"-120\""), "{xml}");
    }

    #[test]
    fn clearing_special_indent_removes_first_line_and_hanging_with_their_twins() {
        // Word's paragraph dialog 特殊インデント「なし」: both first-line and
        // hanging go away (character twins included) while left/right stay.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/indent-with-grid.docx");
        let data = std::fs::read(path).expect("read fixture");
        let mut document = futures::executor::block_on(crate::word_document::WordDocument::load(
            &data,
            &crate::Options::default(),
        ))
        .expect("load fixture");

        let hanging_paragraph = (0..document.paragraph_count())
            .find(|&index| {
                document
                    .paragraph(index)
                    .and_then(|p| p.properties.as_ref())
                    .and_then(|props| props.indent.as_ref())
                    .is_some_and(|indent| indent.hanging_chars.is_some())
            })
            .expect("fixture keeps a hangingChars paragraph");

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphIndentCommand::clearing_special_indent(
                        hanging_paragraph,
                        hanging_paragraph,
                    )),
                    &mut ctx,
                )
                .expect("clear special indent");
        }

        let indent = document
            .paragraph(hanging_paragraph)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.indent.clone())
            .expect("indent kept for the remaining left edge");
        assert_eq!(indent.hanging, None);
        assert_eq!(indent.hanging_chars, None);
        assert_eq!(indent.first_line, None);
        assert_eq!(indent.first_line_chars, None);
        assert_eq!(indent.left, Some(40.0), "left edge survives the clear");

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let restored = document
            .paragraph(hanging_paragraph)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.indent.clone())
            .expect("restored indent");
        assert_eq!(restored.hanging_chars, Some(200));
    }

    #[test]
    fn point_first_line_and_hanging_clear_stale_character_twins_in_export() {
        // `indent-with-grid.docx` carries `w:firstLineChars="200"` and
        // `w:hangingChars="200"`. Word gives the character forms precedence,
        // so leaving them in the XML would make Word ignore the edit.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/indent-with-grid.docx");
        let data = std::fs::read(path).expect("read fixture");
        let mut document = futures::executor::block_on(crate::word_document::WordDocument::load(
            &data,
            &crate::Options::default(),
        ))
        .expect("load fixture");

        let indent_of = |document: &crate::word_document::WordDocument, index: usize| {
            document
                .paragraph(index)
                .and_then(|p| p.properties.as_ref())
                .and_then(|props| props.indent.clone())
        };
        let first_line_paragraphs: Vec<usize> = (0..document.paragraph_count())
            .filter(|&index| {
                indent_of(&document, index).is_some_and(|indent| indent.first_line_chars.is_some())
            })
            .collect();
        let hanging_paragraphs: Vec<usize> = (0..document.paragraph_count())
            .filter(|&index| {
                indent_of(&document, index).is_some_and(|indent| indent.hanging_chars.is_some())
            })
            .collect();
        assert!(!first_line_paragraphs.is_empty());
        assert!(!hanging_paragraphs.is_empty());

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        for index in first_line_paragraphs {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphIndentCommand::new(
                        index,
                        index,
                        None,
                        None,
                        Some(10.5),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("set first line indent");
        }
        for index in hanging_paragraphs {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphIndentCommand::new(
                        index,
                        index,
                        None,
                        None,
                        None,
                        Some(21.0),
                    )),
                    &mut ctx,
                )
                .expect("set hanging indent");
        }

        let exported = document.export_docx().expect("export docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut document_xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document.xml"),
            &mut document_xml,
        )
        .expect("read document.xml");

        assert!(
            !document_xml.contains("w:firstLineChars"),
            "stale w:firstLineChars must be removed: {document_xml}"
        );
        assert!(
            !document_xml.contains("w:hangingChars"),
            "stale w:hangingChars must be removed: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"w:firstLine="210""#),
            "expected new first line indent in export: {document_xml}"
        );
        assert!(
            document_xml.contains(r#"w:hanging="420""#),
            "expected new hanging indent in export: {document_xml}"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn point_indents_replace_character_units_and_undo_exactly(
            left in -400.0_f32..=400.0,
            right in -400.0_f32..=400.0,
            old_left_chars in 0_i32..=20_000,
            old_right_chars in 0_i32..=20_000,
        ) {
            let mut document = sample_document();
            let paragraph = document.paragraph_mut(0).expect("paragraph");
            paragraph.properties = Some(ParagraphProperties {
                indent: Some(crate::document::Indent {
                    left_chars: Some(old_left_chars),
                    right_chars: Some(old_right_chars),
                    ..Default::default()
                }),
                ..Default::default()
            });
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(SetParagraphIndentCommand::new(
                        0,
                        0,
                        Some(left),
                        Some(right),
                        None,
                        None,
                    )),
                    &mut ctx,
                ).expect("set point indents");
            }

            let indent = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.indent.as_ref())
                .expect("indent");
            prop_assert_eq!(indent.left, Some(left));
            prop_assert_eq!(indent.right, Some(right));
            prop_assert_eq!(indent.left_chars, None);
            prop_assert_eq!(indent.right_chars, None);

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo"));
            }
            let restored = document
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.indent.as_ref())
                .expect("restored indent");
            prop_assert_eq!(restored.left, None);
            prop_assert_eq!(restored.right, None);
            prop_assert_eq!(restored.left_chars, Some(old_left_chars));
            prop_assert_eq!(restored.right_chars, Some(old_right_chars));
        }
    }
}
