//! 参考資料 in the document model: notes placed under the text, bookmarks
//! over a selection, fields authored with a result the host computed — a
//! citation, a chapter number, an index or a table of authorities — and
//! the blocks of paragraphs such a field holds, inserted and replaced as a
//! whole.

use super::*;


/// A field the host authors inline — a citation, a chapter number, a table
/// of authorities entry — or one the document already has: where its run
/// is, its instruction and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineField {
    /// The paragraph surface and the run in it.
    pub paragraph: usize,
    pub run: usize,
    pub instruction: String,
    pub text: String,
}

/// One paragraph of a generated block — an index line, a table of
/// authorities entry, a bibliography entry, a manual table of contents
/// line: its text, the page number after it, and how it is set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldBlockLine {
    pub text: String,
    /// The page numbers after the text, when it has them.
    pub page: Option<String>,
    /// What comes between the text and the page numbers: `None` for a tab
    /// to the right margin, otherwise the text itself (`, ` in an index
    /// run in).
    pub separator: Option<String>,
    /// The tab's leader (`dot`, `hyphen`, `underscore`, `none`).
    pub leader: Option<String>,
    /// A paragraph style id for the line.
    pub style: Option<String>,
    /// The left indent and hanging indent, in points.
    pub indent: f32,
    pub hanging: f32,
    pub bold: bool,
    pub centered: bool,
    /// Space before the line, in points (a heading letter's gap).
    pub space_before: f32,
}

/// A field that spans whole top-level paragraphs — a table of contents,
/// an index, a table of authorities, a bibliography — by the paragraph
/// surfaces it starts and ends in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldBlock {
    pub start: usize,
    pub end: usize,
    /// The instruction, trimmed (`TOC \o "1-3" \h \z \u`).
    pub instruction: String,
}

impl FieldBlock {
    /// The field's name, uppercased: `TOC`, `INDEX`, `TOA`, …
    pub fn name(&self) -> String {
        self.instruction
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase()
    }
}

/// Names of fields whose instruction and result the host authors as
/// written; the engine evaluates none of them itself.
pub(crate) fn authored_result_field(instruction: &str) -> bool {
    let name = instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(name.as_str(), "CITATION" | "STYLEREF" | "BIBLIOGRAPHY")
}

/// Names of hidden marker fields (no result) the host authors.
pub(crate) fn authored_marker_field(instruction: &str) -> bool {
    let name = instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(name.as_str(), "XE" | "TA" | "TC")
}

fn field_name(instruction: &str) -> String {
    instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}


/// 図表番号の挿入's choices beyond the label and the text after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionOptions {
    pub label: String,
    pub description: String,
    /// 位置: above the selected item rather than below it.
    pub above: bool,
    /// ラベルを表示しない.
    pub exclude_label: bool,
    pub number_format: SeqNumberFormat,
    /// 章番号を含める: the heading level that starts a chapter, the
    /// separator after its number (`-`, `.`, `:`, `—`, `–`) and the
    /// chapter number itself.
    pub chapter: Option<(u8, String, String)>,
}

impl WordDocument {
    /// Whether a section's footnotes sit right under its text rather than
    /// at the bottom of the page (`w:footnotePr/w:pos`).
    pub fn footnote_beneath_text(&self, section: Option<usize>) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.resolved_footnote_beneath_text(section))
    }

    /// Sets where a section's footnotes go — `beneathText` or `pageBottom`
    /// — keeping its numbering and extensions.
    pub fn set_footnote_beneath_text(
        &mut self,
        beneath: bool,
        section: Option<usize>,
    ) -> Result<bool, String> {
        if self.footnote_beneath_text(section) == beneath {
            return Ok(false);
        }
        self.ensure_roundtrip_xml()
            .map_err(|error| error.to_string())?;
        let sect_pr_id = self
            .sect_pr_id_for_section(section)?
            .ok_or_else(|| "document section is unavailable".to_string())?;
        let value = if beneath { "beneathText" } else { "pageBottom" };
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("document is unavailable")?;
        let fresh = raw.create_element(
            "w:pos",
            vec![XmlAttribute {
                name: "w:val".into(),
                value: value.into(),
            }],
            Vec::new(),
        );
        self.upsert_sect_pr_child(sect_pr_id, "w:footnotePr", |properties| {
            if let Some(existing) = properties
                .children
                .iter_mut()
                .find_map(|child| match child {
                    XmlNode::Element(element) if element.name == "w:pos" => Some(element),
                    _ => None,
                })
            {
                Self::set_element_attr(existing, "w:val", Some(value.into()));
            } else {
                properties.children.insert(0, XmlNode::Element(fresh));
            }
        })?;
        let targets_default = self.section_targets_default_sect_pr(section);
        let document = self.document_mut().ok_or("document is unavailable")?;
        if let Some(range) = section.and_then(|index| document.sections.get_mut(index)) {
            range
                .properties
                .get_or_insert_with(SectionProperties::default)
                .footnote_beneath_text = Some(beneath);
        }
        if targets_default {
            document
                .default_section
                .get_or_insert_with(SectionProperties::default)
                .footnote_beneath_text = Some(beneath);
            if let Some(last) = document.sections.last_mut() {
                last.properties
                    .get_or_insert_with(SectionProperties::default)
                    .footnote_beneath_text = Some(beneath);
            }
        }
        Ok(true)
    }

    /// Creates a user bookmark over a rendered selection of top-level body
    /// paragraphs, as Word's ブックマーク dialog does: around the selected
    /// text, or at the caret when the selection is collapsed. The runs at
    /// either edge are split so the anchors sit exactly at its ends.
    pub fn create_bookmark_at_range(
        &mut self,
        range: SelectionRange,
        name: &str,
    ) -> Result<(), String> {
        let name = name.trim();
        Self::validate_authored_bookmark_name(name)?;
        if self.document.as_ref().is_some_and(|document| {
            document
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.name == name)
        }) {
            return Err("a bookmark with this name already exists".to_string());
        }
        let (start, end) = range.normalized();
        for paragraph in [start.paragraph, end.paragraph] {
            if !self.is_top_level_body_paragraph_surface(paragraph) {
                return Err("bookmarks can only be created in body paragraphs".to_string());
            }
        }
        self.ensure_roundtrip_xml()
            .map_err(|error| error.to_string())?;
        let start = self.rendered_to_model_text_position(start);
        let end = self.rendered_to_model_text_position(end);
        // A boundary at each end, the end first so the start's offsets hold.
        for position in [end, start] {
            let zero_width = self.zero_width_runs(position.paragraph);
            let paragraph = self
                .paragraph_mut(position.paragraph)
                .ok_or_else(|| "bookmark paragraph is unavailable".to_string())?;
            let tail = crate::commands::text_edit::split_runs_at_preserving_source(
                &mut paragraph.runs,
                position.offset,
                &zero_width,
            );
            paragraph.runs.extend(tail);
            self.sync_paragraph_text_preserving_drawings(position.paragraph);
        }
        let body_element = |document: &Self, paragraph: usize| match document
            .paragraph_surfaces
            .get(paragraph)
        {
            Some(ParagraphSurface::Body { path }) => match path.as_slice() {
                [PathEntry::Element(index)] => Some(*index),
                _ => None,
            },
            _ => None,
        };
        // The run each anchor sits before: the first one starting at (or,
        // for the end, the first one past) its offset.
        let run_at = |document: &Self, position: TextPosition, after_zero_width: bool| {
            let paragraph = document.paragraph(position.paragraph)?;
            let mut offset = 0;
            for (index, run) in paragraph.runs.iter().enumerate() {
                let starts_here = offset >= position.offset;
                if starts_here && !(after_zero_width && run.text.is_empty() && offset == position.offset)
                {
                    return Some((index, paragraph.node_id?, run.node_id));
                }
                offset += run.text.len();
            }
            Some((paragraph.runs.len(), paragraph.node_id?, None))
        };
        let (start_run, start_paragraph_id, start_run_id) = run_at(self, start, false)
            .ok_or_else(|| "bookmark paragraph is unavailable".to_string())?;
        let (end_run, end_paragraph_id, end_run_id) = run_at(self, end, true)
            .ok_or_else(|| "bookmark paragraph is unavailable".to_string())?;
        let start_element = body_element(self, start.paragraph)
            .ok_or_else(|| "bookmark paragraph is unavailable".to_string())?;
        let end_element = body_element(self, end.paragraph)
            .ok_or_else(|| "bookmark paragraph is unavailable".to_string())?;
        let bookmark_id = self.next_bookmark_id();
        let raw = self
            .raw_document
            .as_mut()
            .ok_or_else(|| "raw document xml is unavailable".to_string())?;
        // Where an anchor goes among a paragraph's children: before the child
        // holding the run, or at the end.
        fn child_index(
            raw: &crate::docx_xml::DocxXmlDocument,
            paragraph_id: XmlNodeId,
            run_id: Option<XmlNodeId>,
        ) -> Option<usize> {
            let paragraph = raw.element(paragraph_id)?;
            let Some(run_id) = run_id else {
                return Some(paragraph.children.len());
            };
            fn holds(element: &XmlElement, id: XmlNodeId) -> bool {
                element.id == id
                    || element.children.iter().any(
                        |child| matches!(child, XmlNode::Element(child) if holds(child, id)),
                    )
            }
            paragraph
                .children
                .iter()
                .position(|child| matches!(child, XmlNode::Element(child) if holds(child, run_id)))
                .or(Some(paragraph.children.len()))
        }
        let anchor = |raw: &mut crate::docx_xml::DocxXmlDocument, element: &str, with_name: bool| {
            let mut attributes = vec![XmlAttribute {
                name: "w:id".to_string(),
                value: bookmark_id.to_string(),
            }];
            if with_name {
                attributes.push(XmlAttribute {
                    name: "w:name".to_string(),
                    value: name.to_string(),
                });
            }
            raw.create_element(element, attributes, Vec::new())
        };
        let end_index = child_index(raw, end_paragraph_id, end_run_id)
            .ok_or_else(|| "bookmark paragraph XML is unavailable".to_string())?;
        let end_node = anchor(raw, "w:bookmarkEnd", false);
        raw.insert_node(end_paragraph_id, end_index, XmlNode::Element(end_node))
            .map_err(|error| error.to_string())?;
        let mut start_index = child_index(raw, start_paragraph_id, start_run_id)
            .ok_or_else(|| "bookmark paragraph XML is unavailable".to_string())?;
        if start_paragraph_id == end_paragraph_id && start_index > end_index {
            start_index = end_index;
        }
        // Never ahead of the paragraph's properties.
        if let Some(paragraph) = raw.element(start_paragraph_id) {
            let properties = paragraph
                .children
                .iter()
                .position(|child| matches!(child, XmlNode::Element(element) if element.name == "w:pPr"));
            if let Some(properties) = properties {
                start_index = start_index.max(properties + 1);
            }
        }
        let start_node = anchor(raw, "w:bookmarkStart", true);
        raw.insert_node(start_paragraph_id, start_index, XmlNode::Element(start_node))
            .map_err(|error| error.to_string())?;
        raw.rebuild_index_for_subtree(start_paragraph_id);
        if end_paragraph_id != start_paragraph_id {
            raw.rebuild_index_for_subtree(end_paragraph_id);
        }
        self.document_mut()
            .ok_or_else(|| "document is unavailable".to_string())?
            .bookmarks
            .push(Bookmark {
                id: bookmark_id.to_string(),
                name: name.to_string(),
                start_paragraph: start_element,
                start_offset: start.offset,
                start_run,
                end_paragraph: end_element,
                end_offset: end.offset,
                end_run,
            });
        Ok(())
    }

    /// Where a bookmark ends, as [`Self::bookmark_position`] gives its start.
    pub fn bookmark_end_position(&self, name: &str) -> Option<TextPosition> {
        let bookmark = self
            .document
            .as_ref()?
            .bookmarks
            .iter()
            .find(|bookmark| bookmark.name == name)?;
        let paragraph = self.bookmark_surface_index(bookmark.end_paragraph)?;
        let length = self.paragraph_text_len_at(paragraph).unwrap_or(0);
        let model = TextPosition::new(paragraph, bookmark.end_offset.min(length));
        Some(self.model_to_rendered_text_position(model))
    }


    // ---- body elements ------------------------------------------------------

    /// The top-level body element (paragraph or table) a body paragraph
    /// surface belongs to.
    pub fn body_element_of(&self, surface: usize) -> Option<usize> {
        match self.paragraph_surfaces.get(surface)? {
            ParagraphSurface::Body { path } => match path.first()? {
                PathEntry::Element(index) => Some(*index),
                _ => None,
            },
            _ => None,
        }
    }

    /// The first paragraph surface of a top-level body element.
    pub fn body_element_surface(&self, element: usize) -> Option<usize> {
        self.paragraph_surfaces.iter().position(|surface| {
            matches!(surface, ParagraphSurface::Body { path }
                if path.first() == Some(&PathEntry::Element(element)))
        })
    }

    /// How many top-level body elements the document has.
    pub fn body_element_count(&self) -> usize {
        self.document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .map_or(0, |body| body.elements.len())
    }

    // ---- inline fields -----------------------------------------------------

    /// Puts a field at a rendered caret: a hidden marker (`XE`, `TA`, `TC`)
    /// or a field showing `display` (`CITATION`, `STYLEREF`), formatted like
    /// the text before it. Returns the caret after it.
    pub fn insert_inline_field(
        &mut self,
        at: TextPosition,
        instruction: &str,
        display: &str,
    ) -> Result<SelectionRange, String> {
        let instruction = instruction.trim();
        let hidden = authored_marker_field(instruction);
        if !hidden && !authored_result_field(instruction) {
            return Err(format!("{} fields are not authored here", field_name(instruction)));
        }
        if instruction.chars().any(char::is_control) {
            return Err("field instructions must not contain control characters".to_string());
        }
        if !self.is_body_paragraph_surface(at.paragraph) {
            return Err("fields can only be inserted in body paragraphs".to_string());
        }
        let model = self.rendered_to_model_text_position(at);
        self.validate_text_position_boundary(model)?;
        let mut properties = self
            .paragraph(model.paragraph)
            .and_then(|paragraph| Self::formatting_carrier_run_near_offset(paragraph, model.offset))
            .and_then(|run| run.properties)
            .unwrap_or_default();
        if hidden {
            properties.vanish = Some(true);
        }
        let text = if hidden { String::new() } else { display.to_string() };
        let kind = if hidden {
            RunFieldKind::NonDisplay {
                instruction: instruction.to_string(),
            }
        } else {
            RunFieldKind::Unsupported {
                instruction: instruction.to_string(),
            }
        };
        let zero_width = self.zero_width_runs(model.paragraph);
        let paragraph = self
            .paragraph_mut(model.paragraph)
            .ok_or_else(|| "field paragraph is unavailable".to_string())?;
        let tail = split_runs_at_preserving_source(&mut paragraph.runs, model.offset, &zero_width);
        let mut run = blank_run(&text);
        run.properties = Some(properties);
        run.field = Some(RunField {
            word_mac_body_result: None,
            kind,
            cached_text: (!hidden).then(|| text.clone()),
            source_field_id: None,
            resolved_value: (!hidden).then(|| text.clone()),
        });
        paragraph.runs.push(run);
        paragraph.runs.extend(tail);
        // Authored fields are written again from the model, as the XE
        // markers are, so a split elsewhere in the paragraph keeps them.
        for run in &mut paragraph.runs {
            let authored = match run.field.as_ref().map(|field| &field.kind) {
                Some(RunFieldKind::NonDisplay { instruction }) => {
                    authored_marker_field(instruction)
                }
                Some(RunFieldKind::Unsupported { instruction }) => {
                    authored_result_field(instruction)
                }
                _ => false,
            };
            if authored {
                run.node_id = None;
                run.text_node_id = None;
                run.revision_node_id = None;
            }
        }
        self.sync_paragraph_text_preserving_drawings(model.paragraph);
        let after = TextPosition::new(model.paragraph, model.offset + text.len());
        Ok(SelectionRange::collapsed(
            self.model_to_rendered_text_position(after),
        ))
    }

    /// Every field named `name` (`CITATION`, `XE`, `TA`, …) in the body,
    /// headers and notes left out, in document order.
    pub fn inline_fields(&self, name: &str) -> Vec<InlineField> {
        let name = name.to_ascii_uppercase();
        let mut fields = Vec::new();
        for paragraph in 0..self.paragraph_count() {
            if !self.is_body_paragraph_surface(paragraph) {
                continue;
            }
            let Some(model) = self.paragraph(paragraph) else {
                continue;
            };
            for (index, run) in model.runs.iter().enumerate() {
                let instruction = match run.field.as_ref().map(|field| &field.kind) {
                    Some(RunFieldKind::NonDisplay { instruction })
                    | Some(RunFieldKind::Unsupported { instruction }) => instruction,
                    _ => continue,
                };
                if field_name(instruction) != name {
                    continue;
                }
                let text = if run.text.is_empty() {
                    run.field
                        .as_ref()
                        .and_then(|field| {
                            field.resolved_value.clone().or_else(|| field.cached_text.clone())
                        })
                        .unwrap_or_default()
                } else {
                    run.text.clone()
                };
                fields.push(InlineField {
                    paragraph,
                    run: index,
                    instruction: instruction.trim().to_string(),
                    text,
                });
            }
        }
        fields
    }

    /// Changes what an inline field shows, and its instruction when given:
    /// a citation set in another style, a chapter number brought up to date.
    pub fn set_inline_field(
        &mut self,
        paragraph: usize,
        run: usize,
        instruction: Option<&str>,
        text: &str,
    ) -> Result<(), String> {
        let model = self
            .paragraph_mut(paragraph)
            .ok_or_else(|| "field paragraph is unavailable".to_string())?;
        let target = model
            .runs
            .get_mut(run)
            .ok_or_else(|| "field run is unavailable".to_string())?;
        let field = target
            .field
            .as_mut()
            .ok_or_else(|| "the run holds no field".to_string())?;
        if let Some(instruction) = instruction {
            match &mut field.kind {
                RunFieldKind::NonDisplay { instruction: current }
                | RunFieldKind::Unsupported { instruction: current } => {
                    *current = instruction.trim().to_string();
                }
                _ => return Err("the field's instruction cannot be changed".to_string()),
            }
        }
        let hidden = matches!(field.kind, RunFieldKind::NonDisplay { .. });
        if !hidden {
            field.cached_text = Some(text.to_string());
            field.resolved_value = Some(text.to_string());
            target.text = text.to_string();
        }
        // Written again from the model.
        target.node_id = None;
        target.text_node_id = None;
        target.revision_node_id = None;
        self.sync_paragraph_text_preserving_drawings(paragraph);
        Ok(())
    }

    /// Removes an inline field the host authored, with its result.
    pub fn delete_inline_field(&mut self, paragraph: usize, run: usize) -> Result<(), String> {
        let model = self
            .paragraph_mut(paragraph)
            .ok_or_else(|| "field paragraph is unavailable".to_string())?;
        if model.runs.get(run).and_then(|run| run.field.as_ref()).is_none() {
            return Err("the run holds no field".to_string());
        }
        model.runs.remove(run);
        self.sync_paragraph_text_preserving_drawings(paragraph);
        Ok(())
    }

    // ---- field blocks ------------------------------------------------------

    /// Every field spanning whole top-level body paragraphs, in document
    /// order: tables of contents and of figures, indexes, tables of
    /// authorities, bibliographies.
    pub fn field_blocks(&self) -> Vec<FieldBlock> {
        let Some(raw) = self.raw_document.as_ref() else {
            return Vec::new();
        };
        let Some(body) = raw.root().children.iter().find_map(|child| match child {
            XmlNode::Element(element) if element.name == "w:body" => Some(element),
            _ => None,
        }) else {
            return Vec::new();
        };
        struct Frame {
            instruction: String,
            separated: bool,
            begin: usize,
        }
        let mut stack: Vec<Frame> = Vec::new();
        let mut ranges: Vec<(usize, usize, String)> = Vec::new();
        let mut block = 0usize;
        for child in &body.children {
            let XmlNode::Element(element) = child else {
                continue;
            };
            if element.name != "w:p" && element.name != "w:tbl" {
                continue;
            }
            let current = block;
            block += 1;
            if element.name != "w:p" {
                continue;
            }
            Self::visit_field_markers(element, &mut |marker| match marker {
                FieldMarker::Begin => stack.push(Frame {
                    instruction: String::new(),
                    separated: false,
                    begin: current,
                }),
                FieldMarker::InstrText(text) => {
                    if let Some(top) = stack.last_mut() {
                        if !top.separated {
                            top.instruction.push_str(text);
                        }
                    }
                }
                FieldMarker::Separate => {
                    if let Some(top) = stack.last_mut() {
                        top.separated = true;
                    }
                }
                FieldMarker::End => {
                    if let Some(frame) = stack.pop() {
                        // Only the outermost fields that open and close in
                        // different paragraphs, or own a paragraph of their
                        // own, are blocks.
                        if stack.is_empty() {
                            ranges.push((frame.begin, current, frame.instruction));
                        }
                    }
                }
            });
        }
        ranges
            .into_iter()
            .filter_map(|(start, end, instruction)| {
                let instruction = instruction.trim().to_string();
                let name = field_name(&instruction);
                if !matches!(
                    name.as_str(),
                    "TOC" | "INDEX" | "TOA" | "BIBLIOGRAPHY"
                ) {
                    return None;
                }
                Some(FieldBlock {
                    start: self.bookmark_surface_index(start)?,
                    end: self.bookmark_surface_index(end)?,
                    instruction,
                })
            })
            .collect()
    }

    /// Inserts a field block after a top-level body paragraph: the field
    /// opens in the first line and closes in the last. Returns the caret at
    /// the end of the block.
    pub fn insert_field_block(
        &mut self,
        after_paragraph: usize,
        instruction: &str,
        lines: &[FieldBlockLine],
    ) -> Result<SelectionRange, String> {
        let body_index = match self.paragraph_surfaces.get(after_paragraph) {
            Some(ParagraphSurface::Body { path }) => match path.as_slice() {
                [PathEntry::Element(index)] => index + 1,
                _ => return Err("blocks go after top-level body paragraphs".to_string()),
            },
            None if self.paragraph_count() == 0 => 0,
            _ => return Err("blocks go after top-level body paragraphs".to_string()),
        };
        self.insert_field_block_at(body_index, instruction, lines)
    }

    /// Replaces a field block's paragraphs with new ones under a new
    /// instruction, as updating the field does.
    pub fn replace_field_block(
        &mut self,
        block: &FieldBlock,
        instruction: &str,
        lines: &[FieldBlockLine],
    ) -> Result<SelectionRange, String> {
        let body_element = |document: &Self, surface: usize| match document
            .paragraph_surfaces
            .get(surface)
        {
            Some(ParagraphSurface::Body { path }) => match path.as_slice() {
                [PathEntry::Element(index)] => Some(*index),
                _ => None,
            },
            _ => None,
        };
        let start = body_element(self, block.start)
            .ok_or_else(|| "the field's first paragraph is unavailable".to_string())?;
        let end = body_element(self, block.end)
            .ok_or_else(|| "the field's last paragraph is unavailable".to_string())?;
        self.remove_body_elements(start, end)?;
        self.insert_field_block_at(start, instruction, lines)
    }

    /// Removes top-level body elements `start..=end` from the model and the
    /// XML, the bookmarks after them moved up.
    pub(crate) fn remove_body_elements(&mut self, start: usize, end: usize) -> Result<(), String> {
        let node_ids: Vec<Option<XmlNodeId>> = {
            let body = self
                .document
                .as_ref()
                .and_then(|document| document.body.as_ref())
                .ok_or_else(|| "document body is unavailable".to_string())?;
            (start..=end)
                .filter_map(|index| body.elements.get(index))
                .map(|element| element.as_paragraph().and_then(|paragraph| paragraph.node_id))
                .collect()
        };
        if let Some(raw) = self.raw_document.as_mut() {
            if let Some(body_id) = Self::find_body_element_id(raw) {
                let first = Self::body_child_insert_index(raw, body_id, start);
                let last = Self::body_child_insert_index(raw, body_id, end);
                if let (Some(first), Some(last)) = (first, last) {
                    let indices: Vec<usize> = (first..=last).collect();
                    raw.remove_child_nodes(body_id, &indices)
                        .map_err(|error| error.to_string())?;
                }
            } else {
                for node_id in node_ids.into_iter().flatten() {
                    if let Some(path) = raw.path(node_id) {
                        let _ = raw.remove_node(&path);
                    }
                }
            }
            raw.rebuild_index();
        }
        let removed = end + 1 - start;
        let document = self
            .document_mut()
            .ok_or_else(|| "document is unavailable".to_string())?;
        if let Some(body) = document.body.as_mut() {
            let end = (end + 1).min(body.elements.len());
            body.elements.drain(start.min(end)..end);
        }
        document.bookmarks.retain(|bookmark| {
            !(bookmark.start_paragraph >= start && bookmark.end_paragraph <= start + removed - 1)
        });
        for bookmark in &mut document.bookmarks {
            for index in [&mut bookmark.start_paragraph, &mut bookmark.end_paragraph] {
                if *index > start + removed - 1 {
                    *index -= removed;
                } else if *index >= start {
                    *index = start;
                }
            }
        }
        Self::rebuild_section_ranges(document);
        self.rebuild_paragraph_surfaces();
        Ok(())
    }

    fn insert_field_block_at(
        &mut self,
        body_index: usize,
        instruction: &str,
        lines: &[FieldBlockLine],
    ) -> Result<SelectionRange, String> {
        let instruction = instruction.trim();
        if instruction.is_empty() || instruction.chars().any(char::is_control) {
            return Err("the field needs a printable instruction".to_string());
        }
        // An empty result still needs a paragraph to hold the field.
        let placeholder = [FieldBlockLine::default()];
        let lines = if lines.is_empty() { &placeholder[..] } else { lines };
        let tab_twips = self.toc_tab_position_twips();
        let tab_points = tab_twips as f32 / 20.0;
        let mut model_paragraphs = Vec::with_capacity(lines.len());
        let mut xml = String::from(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
        );
        for (index, line) in lines.iter().enumerate() {
            let leader = line.leader.as_deref().unwrap_or("dot");
            let tabbed = line.page.is_some() && line.separator.is_none();
            let run_properties = line.bold.then(|| RunProperties {
                bold: Some(true),
                ..RunProperties::default()
            });
            let mut runs = vec![blank_run(&line.text)];
            runs[0].properties = run_properties.clone();
            if let Some(page) = &line.page {
                runs.push(blank_run(line.separator.as_deref().unwrap_or("\t")));
                runs.push(blank_run(page));
            }
            let indent = (line.indent != 0.0 || line.hanging != 0.0).then(|| Indent {
                left: Some(line.indent),
                hanging: (line.hanging != 0.0).then_some(line.hanging),
                ..Indent::default()
            });
            model_paragraphs.push(Paragraph {
                runs,
                properties: Some(ParagraphProperties {
                    style: line.style.as_deref().map(Rc::from),
                    alignment: line.centered.then(|| Rc::from("center")),
                    indent,
                    spacing: (line.space_before != 0.0).then(|| Spacing {
                        before: Some(line.space_before),
                        ..Spacing::default()
                    }),
                    tabs: if tabbed {
                        vec![TabStop {
                            kind: Some("right".to_string()),
                            position: Some(tab_points),
                            leader: Some(leader.to_string()),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..ParagraphProperties::default()
                }),
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            });
            let mut properties = String::new();
            if let Some(style) = &line.style {
                properties.push_str(&format!(
                    "<w:pStyle w:val=\"{}\"/>",
                    xml_escape_attr(style)
                ));
            }
            if tabbed {
                properties.push_str(&format!(
                    "<w:tabs><w:tab w:val=\"right\" w:leader=\"{leader}\" w:pos=\"{tab_twips}\"/></w:tabs>"
                ));
            }
            if line.space_before != 0.0 {
                properties.push_str(&format!(
                    "<w:spacing w:before=\"{}\"/>",
                    (line.space_before * 20.0).round() as i64
                ));
            }
            if line.indent != 0.0 || line.hanging != 0.0 {
                properties.push_str(&format!(
                    "<w:ind w:left=\"{}\"{}/>",
                    (line.indent * 20.0).round() as i64,
                    if line.hanging != 0.0 {
                        format!(" w:hanging=\"{}\"", (line.hanging * 20.0).round() as i64)
                    } else {
                        String::new()
                    }
                ));
            }
            if line.centered {
                properties.push_str("<w:jc w:val=\"center\"/>");
            }
            let begin = if index == 0 {
                format!(
                    "<w:r><w:fldChar w:fldCharType=\"begin\" w:dirty=\"true\"/></w:r>\
                     <w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>\
                     <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
                    xml_escape_text(instruction)
                )
            } else {
                String::new()
            };
            let end = if index + 1 == lines.len() {
                "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"
            } else {
                ""
            };
            let bold = if line.bold { "<w:b/>" } else { "" };
            let mut content = format!(
                "<w:r><w:rPr>{bold}<w:noProof/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                xml_escape_text(&line.text)
            );
            if let Some(page) = &line.page {
                match &line.separator {
                    None => content.push_str("<w:r><w:rPr><w:noProof/></w:rPr><w:tab/></w:r>"),
                    Some(separator) => content.push_str(&format!(
                        "<w:r><w:rPr><w:noProof/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                        xml_escape_text(separator)
                    )),
                }
                content.push_str(&format!(
                    "<w:r><w:rPr><w:noProof/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                    xml_escape_text(page)
                ));
            }
            xml.push_str(&format!(
                "<w:p><w:pPr>{properties}<w:rPr><w:noProof/></w:rPr></w:pPr>{begin}{content}{end}</w:p>"
            ));
        }
        xml.push_str("</w:body></w:document>");
        let raw_paragraphs = Self::parse_body_paragraph_nodes(&xml)?;
        if raw_paragraphs.len() != lines.len() {
            return Err("failed to build the field's paragraphs".to_string());
        }
        let count = lines.len();
        {
            let document = self
                .document_mut()
                .ok_or_else(|| "document is unavailable".to_string())?;
            let body = document
                .body
                .as_mut()
                .ok_or_else(|| "document body is unavailable".to_string())?;
            if body_index > body.elements.len() {
                return Err("the insertion point is unavailable".to_string());
            }
            for (offset, paragraph) in model_paragraphs.into_iter().enumerate() {
                body.elements
                    .insert(body_index + offset, BodyElement::from_paragraph(paragraph));
            }
            for bookmark in &mut document.bookmarks {
                if bookmark.start_paragraph >= body_index {
                    bookmark.start_paragraph += count;
                }
                if bookmark.end_paragraph >= body_index {
                    bookmark.end_paragraph += count;
                }
            }
            Self::rebuild_section_ranges(document);
        }
        let inserted = self.insert_toc_xml(body_index, raw_paragraphs)?;
        if let Some(body) = self
            .document_mut()
            .and_then(|document| document.body.as_mut())
        {
            for (offset, node_id) in inserted.into_iter().enumerate() {
                if let Some(BodyElement::Paragraph(paragraph)) =
                    body.elements.get_mut(body_index + offset)
                {
                    paragraph.node_id = node_id;
                }
            }
        }
        self.rebuild_paragraph_surfaces();
        let last = self
            .bookmark_surface_index(body_index + count - 1)
            .ok_or_else(|| "the inserted paragraphs are unavailable".to_string())?;
        let end = self
            .paragraph(last)
            .map(paragraph_text_len)
            .unwrap_or_default();
        Ok(SelectionRange::collapsed(
            self.model_to_rendered_text_position(TextPosition::new(last, end)),
        ))
    }

    // ---- moving top-level blocks ------------------------------------------

    /// Moves top-level body elements `start..=end` to stand before element
    /// `before` (counted before the move; the body's length puts them
    /// last), as the navigation pane moves a heading with its section.
    pub fn move_body_elements(
        &mut self,
        start: usize,
        end: usize,
        before: usize,
    ) -> Result<(), String> {
        let length = self
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .map_or(0, |body| body.elements.len());
        if start > end || end >= length || before > length {
            return Err("the blocks to move are out of range".to_string());
        }
        if (start..=end + 1).contains(&before) {
            return Ok(());
        }
        self.ensure_roundtrip_xml()
            .map_err(|error| error.to_string())?;
        let count = end + 1 - start;
        // Where each old index lands.
        let moved = |index: usize| -> usize {
            if (start..=end).contains(&index) {
                if before < start {
                    before + (index - start)
                } else {
                    before - count + (index - start)
                }
            } else if before < start && index >= before && index < start {
                index + count
            } else if before > end && index > end && index < before {
                index - count
            } else {
                index
            }
        };
        if let Some(raw) = self.raw_document.as_mut() {
            let body_id = Self::find_body_element_id(raw)
                .ok_or_else(|| "failed to find w:body in raw xml".to_string())?;
            let first = Self::body_child_insert_index(raw, body_id, start)
                .ok_or_else(|| "the blocks' XML is unavailable".to_string())?;
            let last = Self::body_child_insert_index(raw, body_id, end)
                .ok_or_else(|| "the blocks' XML is unavailable".to_string())?;
            let target = Self::body_child_insert_index(raw, body_id, before)
                .ok_or_else(|| "the destination's XML is unavailable".to_string())?;
            let indices: Vec<usize> = (first..=last).collect();
            let nodes = raw
                .remove_child_nodes(body_id, &indices)
                .map_err(|error| error.to_string())?;
            let target = if target > last {
                target - indices.len()
            } else {
                target
            };
            for (offset, node) in nodes.into_iter().enumerate() {
                raw.insert_node_preserving_ids(body_id, target + offset, node)
                    .map_err(|error| error.to_string())?;
            }
            raw.rebuild_index();
        }
        let document = self
            .document_mut()
            .ok_or_else(|| "document is unavailable".to_string())?;
        if let Some(body) = document.body.as_mut() {
            let block: Vec<BodyElement> = body.elements.drain(start..=end).collect();
            let at = if before > end { before - count } else { before };
            for (offset, element) in block.into_iter().enumerate() {
                body.elements.insert(at + offset, element);
            }
        }
        for bookmark in &mut document.bookmarks {
            bookmark.start_paragraph = moved(bookmark.start_paragraph);
            bookmark.end_paragraph = moved(bookmark.end_paragraph);
        }
        for field in &mut document.toc_entry_fields {
            field.paragraph = moved(field.paragraph);
        }
        Self::rebuild_section_ranges(document);
        document.resolve_structural_fields();
        self.rebuild_paragraph_surfaces();
        Ok(())
    }

    // ---- the bibliography's catalog -----------------------------------------

    /// The bibliography style the catalog names (`StyleName`, `APA`), when
    /// the document has a catalog.
    pub fn bibliography_style(&self) -> Option<String> {
        let path = self.bibliography_sources_part_path()?;
        let xml = self.read_xml_part_content(&path)?;
        let parsed = DocxXmlDocument::parse(&xml).ok()?;
        parsed
            .root()
            .attributes
            .iter()
            .find(|attribute| attribute.name == "StyleName")
            .map(|attribute| attribute.value.clone())
    }

    /// Sets the catalog's style — `StyleName` and the `SelectedStyle`
    /// stylesheet Word applies — creating the catalog's root when the
    /// document has no sources yet is left to the first source.
    pub fn set_bibliography_style(
        &mut self,
        style_name: &str,
        selected_style: &str,
    ) -> Result<(), String> {
        let path = self
            .bibliography_sources_part_path()
            .ok_or_else(|| "the document has no sources".to_string())?;
        let xml = self
            .read_xml_part_content(&path)
            .ok_or_else(|| "bibliography source part is unavailable".to_string())?;
        let mut parsed = DocxXmlDocument::parse(&xml).map_err(|error| error.to_string())?;
        let root_id = parsed.root().id;
        let root = parsed
            .element_mut(root_id)
            .ok_or_else(|| "bibliography source root is unavailable".to_string())?;
        for (name, value) in [("SelectedStyle", selected_style), ("StyleName", style_name)] {
            match root
                .attributes
                .iter_mut()
                .find(|attribute| attribute.name == name)
            {
                Some(attribute) => attribute.value = value.to_string(),
                None => root.attributes.push(XmlAttribute {
                    name: name.to_string(),
                    value: value.to_string(),
                }),
            }
        }
        let written = parsed.to_string().map_err(|error| error.to_string())?;
        self.upsert_xml_part_content(&path, written);
        Ok(())
    }

    /// Adds a source to the catalog (資料文献の管理's コピー into the
    /// current list).
    pub fn add_citation_source_entry(&mut self, source: CitationSourceInfo) -> Result<(), String> {
        self.add_citation_source(source)
    }

    /// Removes a source from the catalog by its tag.
    pub fn remove_citation_source(&mut self, tag: &str) -> Result<(), String> {
        let path = self
            .bibliography_sources_part_path()
            .ok_or_else(|| "the document has no sources".to_string())?;
        let xml = self
            .read_xml_part_content(&path)
            .ok_or_else(|| "bibliography source part is unavailable".to_string())?;
        let mut parsed = DocxXmlDocument::parse(&xml).map_err(|error| error.to_string())?;
        let root_id = parsed.root().id;
        let index = parsed
            .root()
            .children
            .iter()
            .position(|child| match child {
                XmlNode::Element(source) => {
                    Self::xml_local_name(&source.name) == "Source"
                        && Self::xml_descendant_text(source, "Tag").as_deref() == Some(tag)
                }
                _ => false,
            })
            .ok_or_else(|| format!("citation source {tag} was not found"))?;
        parsed
            .remove_child_nodes(root_id, &[index])
            .map_err(|error| error.to_string())?;
        let written = parsed.to_string().map_err(|error| error.to_string())?;
        self.upsert_xml_part_content(&path, written);
        Ok(())
    }

    /// Replaces a source — edited in 資料文献の編集 — keeping its place.
    pub fn replace_citation_source(
        &mut self,
        tag: &str,
        source: CitationSourceInfo,
    ) -> Result<(), String> {
        self.remove_citation_source(tag)?;
        self.add_citation_source(source)
    }

    // ---- tables of contents ------------------------------------------------

    /// ページ番号だけを更新する: the page numbers a table of contents
    /// shows, written as `pages` (bookmark → page) gives them, its entries
    /// otherwise left as they are. Returns how many numbers changed.
    pub fn refresh_toc_page_numbers(
        &mut self,
        index: usize,
        pages: &HashMap<String, String>,
    ) -> Result<usize, String> {
        let block = self
            .tables_of_contents()
            .into_iter()
            .nth(index)
            .ok_or_else(|| "no table of contents found to update".to_string())?;
        let mut changed = 0;
        for paragraph in block.start..=block.end {
            let mut names = Vec::new();
            if let Some(model) = self.paragraph_mut(paragraph) {
                for run in &mut model.runs {
                    let Some(field) = run.field.as_mut() else {
                        continue;
                    };
                    let RunFieldKind::PageRef { name, .. } = &field.kind else {
                        continue;
                    };
                    let Some(page) = pages.get(name) else {
                        continue;
                    };
                    if field.cached_text.as_deref() != Some(page.as_str()) {
                        field.cached_text = Some(page.clone());
                        names.push((name.clone(), page.clone()));
                    }
                }
            }
            if names.is_empty() {
                continue;
            }
            let paragraph_id = self.paragraph(paragraph).and_then(|model| model.node_id);
            if let (Some(raw), Some(paragraph_id)) = (self.raw_document.as_mut(), paragraph_id) {
                if let Some(element) = raw.element_mut(paragraph_id) {
                    for (name, page) in &names {
                        set_page_reference_result(element, name, page);
                    }
                }
            }
            changed += names.len();
        }
        Ok(changed)
    }

    /// 手動作成目次: a 目次 heading and lines for levels 1 to 3 to type
    /// over, set as the generated contents are but not a field — nothing
    /// updates them.
    pub fn insert_manual_table_of_contents(
        &mut self,
        after_paragraph: usize,
        heading: &str,
        lines: &[(u8, String, String)],
        leader: &str,
    ) -> Result<SelectionRange, String> {
        let body_index = match self.paragraph_surfaces.get(after_paragraph) {
            Some(ParagraphSurface::Body { path }) => match path.as_slice() {
                [PathEntry::Element(index)] => index + 1,
                _ => return Err("the contents go after top-level body paragraphs".to_string()),
            },
            _ => return Err("the contents go after top-level body paragraphs".to_string()),
        };
        let tab_twips = self.toc_tab_position_twips();
        let mut xml = String::from(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
        );
        let mut model = Vec::new();
        xml.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"TOCHeading\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            xml_escape_text(heading)
        ));
        model.push(Paragraph {
            runs: vec![blank_run(heading)],
            properties: Some(ParagraphProperties {
                style: Some(Rc::from("TOCHeading")),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        });
        for (level, text, page) in lines {
            let style = format!("TOC{}", level.clamp(&1, &9));
            xml.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/><w:tabs><w:tab w:val=\"right\" w:leader=\"{leader}\" w:pos=\"{tab_twips}\"/></w:tabs></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>{}</w:t></w:r></w:p>",
                xml_escape_text(text),
                xml_escape_text(page)
            ));
            model.push(Paragraph {
                runs: vec![blank_run(text), blank_run("\t"), blank_run(page)],
                properties: Some(ParagraphProperties {
                    style: Some(Rc::from(style.as_str())),
                    tabs: vec![TabStop {
                        kind: Some("right".to_string()),
                        position: Some(tab_twips as f32 / 20.0),
                        leader: Some(leader.to_string()),
                    }],
                    ..ParagraphProperties::default()
                }),
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            });
        }
        xml.push_str("</w:body></w:document>");
        let raw_paragraphs = Self::parse_body_paragraph_nodes(&xml)?;
        let count = model.len();
        {
            let document = self
                .document_mut()
                .ok_or_else(|| "document is unavailable".to_string())?;
            let body = document
                .body
                .as_mut()
                .ok_or_else(|| "document body is unavailable".to_string())?;
            for (offset, paragraph) in model.into_iter().enumerate() {
                body.elements
                    .insert(body_index + offset, BodyElement::from_paragraph(paragraph));
            }
            for bookmark in &mut document.bookmarks {
                if bookmark.start_paragraph >= body_index {
                    bookmark.start_paragraph += count;
                }
                if bookmark.end_paragraph >= body_index {
                    bookmark.end_paragraph += count;
                }
            }
            Self::rebuild_section_ranges(document);
        }
        let inserted = self.insert_toc_xml(body_index, raw_paragraphs)?;
        if let Some(body) = self
            .document_mut()
            .and_then(|document| document.body.as_mut())
        {
            for (offset, node_id) in inserted.into_iter().enumerate() {
                if let Some(BodyElement::Paragraph(paragraph)) =
                    body.elements.get_mut(body_index + offset)
                {
                    paragraph.node_id = node_id;
                }
            }
        }
        self.rebuild_paragraph_surfaces();
        let first = self
            .bookmark_surface_index(body_index + 1)
            .or_else(|| self.bookmark_surface_index(body_index))
            .ok_or_else(|| "the inserted contents are unavailable".to_string())?;
        Ok(SelectionRange::collapsed(
            self.model_to_rendered_text_position(TextPosition::new(first, 0)),
        ))
    }

    // ---- captions ------------------------------------------------------------

    /// 図表番号の挿入 with its choices: the caption above or below the
    /// paragraph `anchor`, its number in a format, after the chapter's
    /// number when one is asked for. Returns the caret at the caption's end.
    pub fn insert_caption_with(
        &mut self,
        anchor: usize,
        options: &CaptionOptions,
    ) -> Result<SelectionRange, String> {
        let caret = self.insert_caption_after_paragraph(
            anchor,
            &options.label,
            &options.description,
        )?;
        let caption = caret.focus.paragraph;
        let label = options.label.trim().to_string();
        // Rebuild the caption's runs from the plain one: label, chapter,
        // number, text.
        let model = self
            .paragraph_mut(caption)
            .ok_or_else(|| "the caption is unavailable".to_string())?;
        let template = model
            .runs
            .first()
            .cloned()
            .ok_or_else(|| "the caption is empty".to_string())?;
        let seq_index = model
            .runs
            .iter()
            .position(|run| matches!(run.field.as_ref().map(|field| &field.kind), Some(RunFieldKind::Seq { .. })))
            .ok_or_else(|| "the caption has no number".to_string())?;
        let mut seq = model.runs[seq_index].clone();
        let rest: Vec<Run> = model.runs[seq_index + 1..].to_vec();
        if let Some(RunFieldKind::Seq {
            number_format,
            restart_after_heading,
            ..
        }) = seq.field.as_mut().map(|field| &mut field.kind)
        {
            *number_format = Some(options.number_format);
            *restart_after_heading = options.chapter.as_ref().map(|(level, _, _)| *level);
        }
        seq.node_id = None;
        seq.text_node_id = None;
        let piece = |text: &str| {
            let mut run = template.clone();
            run.text = text.to_string();
            run.field = None;
            run.node_id = None;
            run.text_node_id = None;
            run.revision_node_id = None;
            run
        };
        let mut runs = Vec::new();
        if !options.exclude_label {
            runs.push(piece(&format!("{label} ")));
        }
        if let Some((level, separator, number)) = &options.chapter {
            let mut chapter = piece(number);
            chapter.field = Some(RunField {
                word_mac_body_result: None,
                kind: RunFieldKind::Unsupported {
                    instruction: format!("STYLEREF {level} \\s"),
                },
                cached_text: Some(number.clone()),
                source_field_id: None,
                resolved_value: Some(number.clone()),
            });
            runs.push(chapter);
            runs.push(piece(separator));
        }
        runs.push(seq);
        for mut run in rest {
            run.node_id = None;
            run.text_node_id = None;
            runs.push(run);
        }
        model.runs = runs;
        // Written again from the model, field and all.
        let paragraph_id = model.node_id;
        if let (Some(raw), Some(paragraph_id)) = (self.raw_document.as_mut(), paragraph_id) {
            if let Some(element) = raw.element_mut(paragraph_id) {
                element.children.retain(|child| {
                    matches!(child, XmlNode::Element(element) if element.name == "w:pPr")
                });
            }
            raw.rebuild_index_for_subtree(paragraph_id);
        }
        self.sync_paragraph_text_preserving_drawings(caption);
        if options.above {
            // Below is where it went; above is one paragraph up.
            let element = |document: &Self, surface: usize| match document
                .paragraph_surfaces
                .get(surface)
            {
                Some(ParagraphSurface::Body { path }) => match path.as_slice() {
                    [PathEntry::Element(index)] => Some(*index),
                    _ => None,
                },
                _ => None,
            };
            let caption_element =
                element(self, caption).ok_or_else(|| "the caption is unavailable".to_string())?;
            let anchor_element =
                element(self, anchor).ok_or_else(|| "the anchor is unavailable".to_string())?;
            self.move_body_elements(caption_element, caption_element, anchor_element)?;
        }
        if let Some(document) = self.document_mut() {
            document.resolve_structural_fields();
        }
        let caption = (0..self.paragraph_count())
            .find(|&index| {
                self.paragraph(index)
                    .is_some_and(|paragraph| paragraph.node_id == paragraph_id)
            })
            .unwrap_or(caption);
        let end = self.paragraph(caption).map(paragraph_text_len).unwrap_or(0);
        Ok(SelectionRange::collapsed(
            self.model_to_rendered_text_position(TextPosition::new(caption, end)),
        ))
    }
}

/// Sets the result of the `PAGEREF name` field in a paragraph's XML to
/// `page`: the first text after its separator, the rest emptied.
fn set_page_reference_result(paragraph: &mut XmlElement, name: &str, page: &str) {
    #[derive(PartialEq)]
    enum State {
        Looking,
        Instruction,
        Result,
        Done,
    }
    fn visit(element: &mut XmlElement, name: &str, page: &str, state: &mut State, written: &mut bool) {
        for child in &mut element.children {
            let XmlNode::Element(child) = child else {
                continue;
            };
            if *state == State::Done {
                return;
            }
            match child.name.as_str() {
                "w:fldChar" => {
                    let kind = child
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "w:fldCharType")
                        .map(|attribute| attribute.value.as_str());
                    match (kind, &*state) {
                        (Some("separate"), State::Instruction) => *state = State::Result,
                        (Some("end"), State::Result) => *state = State::Done,
                        (Some("end"), State::Instruction) => *state = State::Looking,
                        _ => {}
                    }
                }
                "w:instrText" => {
                    let text: String = child
                        .children
                        .iter()
                        .filter_map(|node| match node {
                            XmlNode::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    let mut words = text.split_whitespace();
                    if *state == State::Looking
                        && words.next().is_some_and(|word| word.eq_ignore_ascii_case("PAGEREF"))
                        && words.next() == Some(name)
                    {
                        *state = State::Instruction;
                    }
                }
                "w:t" if *state == State::Result => {
                    let value = if *written { "" } else { page };
                    *written = true;
                    child.children = vec![XmlNode::Text(value.to_string())];
                }
                _ => visit(child, name, page, state, written),
            }
        }
    }
    let mut state = State::Looking;
    let mut written = false;
    visit(paragraph, name, page, &mut state, &mut written);
}
