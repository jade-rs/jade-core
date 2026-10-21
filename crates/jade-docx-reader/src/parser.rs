use crate::document::*;
use crate::docx_xml::{DocxXmlDocument, DocxXmlError, DocxXmlTreeBuilder};
use quick_xml::escape::unescape;
use quick_xml::events::{attributes::Attributes, BytesStart, BytesText, Event};
use quick_xml::Reader;
use quick_xml::Writer;
use std::collections::HashMap;
use std::io::{BufRead, Cursor};
use std::rc::Rc;

enum DrawingObject {
    Image(RunImage),
    TextBox(RunTextBox),
}

#[derive(Clone, Copy)]
enum ShapeThemeColorTarget {
    Fill,
    Border,
    Shadow,
    InnerShadow,
    Glow,
}

struct ParsedVmlStyle {
    is_absolute: bool,
    width_px: Option<f64>,
    height_px: Option<f64>,
    position_h: Option<ImagePosition>,
    position_v: Option<ImagePosition>,
    rotation_deg: Option<f64>,
    text_vertical_alignment: Option<TextBoxVerticalAlignment>,
    dist_top: Option<f64>,
    dist_bottom: Option<f64>,
    dist_left: Option<f64>,
    dist_right: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldSource {
    Complex,
    Simple,
}

#[derive(Clone)]
struct ActiveField {
    // Complex fields are pushed at `begin`, before their instruction is known.
    // Keeping that placeholder is important: `end` must close this exact field
    // even when it has no `separate` marker or uses an unsupported instruction.
    kind: Option<RunFieldKind>,
    locked: bool,
    source: FieldSource,
    // Complex-field instructions nest. Storing the buffer on the matching
    // stack frame prevents an inner `begin` from overwriting its parent's IF
    // instruction before the outer `separate` marker is reached.
    instruction: Option<String>,
    // A field opened while its parent is still reading instruction text is an
    // operand, not visible document content. Its cached result is substituted
    // into the parent instruction when this frame closes.
    is_instruction_operand: bool,
    operand_result: String,
    // The instruction as Word for Mac reads it in the main story, with each
    // nested COMPARE or IF at its saved result, and the IF result it gives
    // when that differs from the evaluated instruction.
    saved_instruction: Option<String>,
    word_mac_body_result: Option<Box<str>>,
    // Simple fields can legitimately have no result run. Remember the run
    // count at entry so invisible structural fields such as SET can still be
    // materialized when the element closes without adding visible content.
    paragraph_run_start: Option<usize>,
    sequence_result_run: Option<usize>,
}

#[derive(Clone)]
struct DeferredNestedField {
    kind: RunFieldKind,
    cached_result: String,
}

const DEFERRED_NESTED_FIELD_PREFIX: &str = "\u{e000}jade-nested-field:";
const DEFERRED_NESTED_FIELD_SUFFIX: char = '\u{e001}';

/// Collected state for a `w:ruby` (phonetic guide, ECMA-376 Part 1 §17.3.3.25)
/// element while the parser is inside it. The `w:rt` guide text is consumed by
/// a dedicated subtree walk (so it never leaks into the body text), while the
/// `w:rubyBase` runs flow through the normal run parsing path; at `</w:ruby>`
/// the annotation is attached to the first base run.
struct PendingRuby {
    /// `paragraph.runs.len()` at `<w:ruby>` start — the index where the first
    /// `w:rubyBase` run will be pushed.
    base_run_index: usize,
    /// Concatenated guide text from `w:rt`.
    text: String,
    /// Guide size in points (`w:rt` run `w:sz` half-points ÷ 2, or
    /// `w:rubyPr/w:hps` as fallback).
    size_pt: Option<f32>,
    /// `w:rubyPr/w:rubyAlign` token, lowercased.
    align: Option<String>,
    /// `w:rubyPr/w:hpsRaise` in half-points.
    raise_half_points: Option<u32>,
    /// The guide run's own `w:rFonts`.
    fonts: Option<RunFonts>,
}

mod diagram;
pub mod math;
mod table;
use math::{parse_math_elements, parse_math_paragraph};
use table::ContainerContext;

/// Source of quick-xml events for [`DocumentParser`].
///
/// A plain [`Reader`] is the usual source. [`RawTreeTeeReader`] additionally
/// mirrors every event into a [`DocxXmlTreeBuilder`] so that the semantic
/// parse and the round-trip XML tree share one tokenization of
/// `word/document.xml`. Every helper that walks a subtree takes `&mut R`
/// rather than `&mut Reader<R>` so that whichever source drives the top-level
/// loop also sees the events those helpers consume.
pub trait XmlEventSource {
    fn read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<Event<'b>>;
}

impl<R: BufRead> XmlEventSource for Reader<R> {
    fn read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<Event<'b>> {
        Reader::read_event_into(self, buf)
    }
}

/// Reads `word/document.xml` for the semantic parser while building the
/// lossless [`DocxXmlDocument`] from the same events.
///
/// A tree-building failure is remembered rather than propagated: the semantic
/// parse keeps going (rendering does not need the raw tree) and
/// [`Self::finish`] reports the error to the caller.
pub struct RawTreeTeeReader<'a> {
    reader: Reader<&'a [u8]>,
    builder: Option<DocxXmlTreeBuilder>,
    error: Option<DocxXmlError>,
}

impl<'a> RawTreeTeeReader<'a> {
    pub fn new(xml: &'a str) -> Self {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        Self {
            reader,
            builder: Some(DocxXmlTreeBuilder::new()),
            error: None,
        }
    }

    /// Drains any events the semantic parser did not consume and completes
    /// the raw tree.
    pub fn finish(mut self) -> Result<DocxXmlDocument, DocxXmlError> {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match XmlEventSource::read_event_into(&mut self, &mut buf) {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(DocxXmlError::Malformed(err.to_string())),
            }
        }
        if let Some(err) = self.error {
            return Err(err);
        }
        match self.builder {
            Some(builder) => builder.finish(),
            None => Err(DocxXmlError::Malformed(
                "raw tree builder was dropped after an earlier error".into(),
            )),
        }
    }
}

impl XmlEventSource for RawTreeTeeReader<'_> {
    fn read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<Event<'b>> {
        let event = self.reader.read_event_into(buf)?;
        if let Some(builder) = self.builder.as_mut() {
            if let Err(err) = builder.observe(&event) {
                self.error = Some(err);
                self.builder = None;
            }
        }
        Ok(event)
    }
}

/// Streaming XML parser that converts DOCX `document.xml` content into the
/// simplified data structures defined in `document.rs`.
#[derive(Default)]
pub struct DocumentParser {
    text_box_depth: usize,
}

impl DocumentParser {
    fn trim_xml_whitespace(text: &str) -> &str {
        text.trim_matches(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r'))
    }

    /// Word sets a line feed written inside `w:t` as a space; only `w:br`
    /// and `w:cr` break the line, and those become `\n` in run text.
    fn line_feeds_as_spaces(text: &str) -> String {
        if !text.contains(['\n', '\r']) {
            return text.to_string();
        }
        text.replace("\r\n", " ").replace(['\n', '\r'], " ")
    }

    /// Creates a new parser instance. The parser has no internal state so this
    /// is mainly a convenience constructor for callers that prefer fluent APIs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses the provided XML string representing `word/document.xml` and
    /// produces a [`Document`] tree that the renderer can consume.
    ///
    /// The implementation purposefully extracts only the subset of tags that
    /// influence layout in the current renderer: paragraphs, runs, spacing,
    /// indentation, numbering, and page settings.
    pub fn parse_document_xml(
        &self,
        xml_content: &str,
    ) -> Result<Document, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(false);
        self.parse_document_events(&mut reader)
    }

    /// Parses `word/document.xml` once for both consumers: the layout
    /// [`Document`] this parser builds and the lossless [`DocxXmlDocument`]
    /// the editor patches on save. Every quick-xml event the semantic parser
    /// consumes is mirrored into a [`DocxXmlTreeBuilder`], so the raw tree is
    /// identical to `DocxXmlDocument::parse(xml_content)` (same element ids
    /// and node paths) without tokenizing the part a second time.
    ///
    /// A malformed raw tree is reported alongside the semantic document
    /// instead of failing the load: rendering does not need it, only save and
    /// package edits do.
    pub fn parse_document_xml_with_raw_tree(
        &self,
        xml_content: &str,
    ) -> Result<(Document, Result<DocxXmlDocument, DocxXmlError>), Box<dyn std::error::Error>> {
        let mut reader = RawTreeTeeReader::new(xml_content);
        let document = self.parse_document_events(&mut reader)?;
        Ok((document, reader.finish()))
    }

    fn parse_document_events(
        &self,
        reader: &mut dyn XmlEventSource,
    ) -> Result<Document, Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        let mut document = Document::default();
        let mut current_paragraph: Option<Paragraph> = None;
        // Word serializes display equations as `w:p > m:oMathPara`. The
        // paragraph is only an OOXML wrapper; exposing it as a second empty
        // editable paragraph shifts every later editor coordinate by one.
        let mut current_paragraph_is_math_wrapper = false;
        let mut current_run: Option<Run> = None;
        let mut current_run_image: Option<RunImage> = None;
        let mut current_run_text_box: Option<RunTextBox> = None;
        let mut current_text = String::new();
        let mut in_text = false;
        let mut current_text_preserve = false;
        let mut current_paragraph_props: Option<ParagraphProperties> = None;
        let mut current_paragraph_para_id: Option<String> = None;
        let mut parsing_paragraph_props = false;
        let mut current_paragraph_run_props: Option<RunProperties> = None;
        let mut parsing_paragraph_run_props = false;
        let mut current_run_props: Option<RunProperties> = None;
        let mut parsing_run_props = false;
        let mut current_run_has_carriage_return = false;
        let mut current_run_has_attributed_soft_break = false;
        let mut current_section_props: Option<SectionProperties> = None;
        let mut default_section_props: Option<SectionProperties> = None;
        let mut container_stack: Vec<ContainerContext> = Vec::new();
        let mut parsing_section_props = false;
        let mut current_run_field: Option<RunFieldKind> = None;
        let mut current_run_is_instruction_operand = false;
        // Accumulates the field result text for the current run while a field is
        // active (text that would otherwise be suppressed). Attached to the
        // run's `RunField::cached_text` so the renderer can fall back to Word's
        // last-computed value.
        let mut current_run_field_text = String::new();
        let mut current_run_sequence_result = None;
        let mut current_run_has_field_end = false;
        let mut active_fields: Vec<ActiveField> = Vec::new();
        // Nested fields that depend on a forward bookmark cannot be evaluated
        // when their closing tag is encountered. Keep their semantic kind and
        // cached fallback until every bookmark in the document is available.
        let mut deferred_nested_fields: Vec<DeferredNestedField> = Vec::new();
        // Rebuilt after each completed paragraph. Nested field operands can
        // therefore use bookmarks from preceding paragraphs without waiting
        // until the final document-wide structural-field pass.
        let mut resolved_bookmark_text: HashMap<String, String> = HashMap::new();
        let mut resolved_bookmark_count = 0usize;
        let mut in_instr_text = false;
        let mut current_instr_text = String::new();
        let mut paragraph_text_len: usize = 0;
        let mut open_comment_ranges: HashMap<String, usize> = HashMap::new();
        // `(name, start_paragraph, start_offset, start_run)` keyed by w:id
        // while the bookmark is still open. Word does not reuse ids within a
        // document. Run boundaries disambiguate zero-width computed fields.
        // Paragraph indices are resolved against `document.body.elements.len()`
        // at event time, so bookmarks nested inside tables / text boxes record
        // the enclosing top-level paragraph index (approximate). Consumers
        // that need precise nested coordinates should treat them as hints —
        // the same trade-off we use for comment / footnote references.
        let mut open_bookmarks: HashMap<String, (String, usize, usize, usize)> = HashMap::new();
        let mut paragraph_comment_ranges: Vec<CommentRange> = Vec::new();
        let mut paragraph_comment_markers: Vec<CommentMarker> = Vec::new();
        let mut revision_stack: Vec<RevisionMark> = Vec::new();
        let mut hyperlink_stack: Vec<RunHyperlink> = Vec::new();
        let mut pending_ruby: Option<PendingRuby> = None;

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    match e.name().as_ref() {
                        b"w:body" | b"w:hdr" | b"w:ftr" => {
                            document.body = Some(Body {
                                elements: Vec::new(),
                            });
                        }
                        b"w:p" => {
                            current_paragraph_is_math_wrapper = false;
                            current_paragraph = Some(Paragraph {
                                runs: Vec::new(),
                                properties: None,
                                node_id: None,
                                comment_ranges: Vec::new(),
                                comment_markers: Vec::new(),
                            });
                            current_paragraph_props = Some(ParagraphProperties::default());
                            current_paragraph_para_id = Self::parse_para_id(e.attributes());
                            current_paragraph_run_props = None;
                            paragraph_text_len = 0;
                            // Comment ranges may span multiple paragraphs. A range
                            // that is still open when a new paragraph begins keeps
                            // running, continuing from offset 0 of this paragraph
                            // rather than being discarded.
                            for start in open_comment_ranges.values_mut() {
                                *start = 0;
                            }
                            paragraph_comment_ranges.clear();
                            paragraph_comment_markers.clear();
                        }
                        b"w:ins" | b"w:del" | b"w:moveFrom" | b"w:moveTo" => {
                            let mark = Self::build_revision_mark(e, Self::revision_kind(e));
                            if parsing_paragraph_run_props {
                                if let Some(props) = current_paragraph_props.as_mut() {
                                    props.mark_revision = Some(Box::new(mark.clone()));
                                }
                            }
                            // Popped again by the matching end tag.
                            revision_stack.push(mark);
                        }
                        b"w:tbl" => {
                            const MAX_TABLE_DEPTH: usize = 256;
                            let table_depth = container_stack
                                .iter()
                                .filter(|ctx| matches!(ctx, ContainerContext::Table(_)))
                                .count();
                            if table_depth >= MAX_TABLE_DEPTH {
                                return Err(format!(
                                    "DOCX table nesting exceeds supported depth of {MAX_TABLE_DEPTH}"
                                )
                                .into());
                            }
                            container_stack.push(ContainerContext::Table(Box::default()));
                        }
                        b"w:altChunk" => {
                            Self::record_alt_chunk(&mut document, e.attributes());
                            Self::consume_element(reader, b"w:altChunk")?;
                        }
                        b"w:hyperlink" => {
                            let hyperlink = Self::parse_hyperlink_attributes(e.attributes());
                            hyperlink_stack.push(hyperlink);
                        }
                        b"w:tr" => {
                            container_stack.push(ContainerContext::TableRow(TableRow::default()));
                        }
                        b"w:tc" => {
                            container_stack.push(ContainerContext::TableCell(TableCell::default()));
                        }
                        b"w:tblPr" => {
                            if let Some(ContainerContext::Table(table)) = container_stack.last_mut()
                            {
                                self.parse_table_properties(reader, table)?;
                            } else {
                                self.skip_element(reader, b"w:tblPr")?;
                            }
                        }
                        b"w:tblGrid" => {
                            if let Some(ContainerContext::Table(table)) = container_stack.last_mut()
                            {
                                table.columns = self.parse_table_grid(reader)?;
                            } else {
                                self.skip_element(reader, b"w:tblGrid")?;
                            }
                        }
                        b"w:trPr" => {
                            if let Some(ContainerContext::TableRow(row)) =
                                container_stack.last_mut()
                            {
                                self.parse_table_row_properties(reader, row)?;
                            } else {
                                self.skip_element(reader, b"w:trPr")?;
                            }
                        }
                        b"w:tblPrEx" => {
                            if let Some(ContainerContext::TableRow(row)) =
                                container_stack.last_mut()
                            {
                                self.parse_table_property_exceptions(reader, row)?;
                            } else {
                                self.skip_element(reader, b"w:tblPrEx")?;
                            }
                        }
                        b"w:tcPr" => {
                            if let Some(ContainerContext::TableCell(cell)) =
                                container_stack.last_mut()
                            {
                                self.parse_table_cell_properties(reader, cell)?;
                            } else {
                                self.skip_element(reader, b"w:tcPr")?;
                            }
                        }
                        b"w:pPr" => {
                            parsing_paragraph_props = true;
                        }
                        b"w:pPrChange" => {
                            if let Ok(raw) = Self::capture_element_xml(reader, e.to_owned()) {
                                if let Some(numbering) = Self::parse_numbering_from_ppr_change(&raw)
                                {
                                    if let Some(ref mut props) = current_paragraph_props {
                                        if props.numbering.is_none() {
                                            props.numbering = Some(numbering);
                                        }
                                    } else {
                                        current_paragraph_props = Some(ParagraphProperties {
                                            numbering: Some(numbering),
                                            ..ParagraphProperties::default()
                                        });
                                    }
                                }
                            }
                        }
                        b"w:rPrChange" | b"w:sectPrChange" => {
                            let name = e.name().as_ref().to_vec();
                            self.skip_element(reader, &name)?;
                        }
                        b"w:pStyle" => {
                            if parsing_paragraph_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let style_id =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        if let Some(ref mut props) = current_paragraph_props {
                                            props.style = Some(style_id.into());
                                        }
                                    }
                                }
                            }
                        }
                        b"w:numPr" => {
                            if parsing_paragraph_props {
                                // Initialize numbering if not already present
                                if let Some(ref mut props) = current_paragraph_props {
                                    if props.numbering.is_none() {
                                        props.numbering = Some(crate::document::Numbering {
                                            id: None,
                                            level: None,
                                        });
                                    }
                                }
                            }
                        }
                        b"w:numId" => {
                            if parsing_paragraph_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let num_id =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        if let Some(ref mut props) = current_paragraph_props {
                                            if let Some(ref mut numbering) = props.numbering {
                                                numbering.id = Some(num_id);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        b"w:ilvl" => {
                            if parsing_paragraph_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        if let Ok(level) =
                                            String::from_utf8_lossy(&attr.value).parse::<u32>()
                                        {
                                            if let Some(ref mut props) = current_paragraph_props {
                                                if let Some(ref mut numbering) = props.numbering {
                                                    numbering.level = Some(level);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        b"w:jc" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_alignment(
                                    e.attributes(),
                                    current_paragraph_props.as_mut(),
                                );
                            }
                        }
                        b"w:bidi" => {
                            if parsing_section_props {
                                // A section that reads right to left (`w:sectPr/w:bidi`), not
                                // the paragraph whose `w:pPr` happens to carry the section.
                                if let Some(section) = current_section_props.as_mut() {
                                    section.bidi = Self::parse_on_off_attr(e.attributes());
                                }
                            } else if parsing_paragraph_props {
                                let value = Self::parse_on_off_attr(e.attributes());
                                current_paragraph_props
                                    .get_or_insert_with(ParagraphProperties::default)
                                    .bidi = Some(value);
                            }
                        }
                        b"w:ind" => {
                            if parsing_paragraph_props {
                                let props = current_paragraph_props
                                    .get_or_insert_with(ParagraphProperties::default);
                                Self::apply_paragraph_indent(e.attributes(), Some(props));
                            }
                        }
                        b"w:spacing" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_spacing(
                                    e.attributes(),
                                    current_paragraph_props.as_mut(),
                                );
                            }
                        }
                        b"w:autoSpaceDE" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_auto_spacing(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    false,
                                );
                            }
                        }
                        b"w:autoSpaceDN" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_auto_spacing(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    true,
                                );
                            }
                        }
                        b"w:kinsoku" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_line_break_option(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    "kinsoku",
                                );
                            }
                        }
                        b"w:wordWrap" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_line_break_option(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    "wordWrap",
                                );
                            }
                        }
                        b"w:overflowPunct" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_line_break_option(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    "overflowPunct",
                                );
                            }
                        }
                        b"w:topLinePunct" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_line_break_option(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                    "topLinePunct",
                                );
                            }
                        }
                        b"w:adjustRightInd" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_adjust_right_indent(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:widowControl" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_widow_control(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:suppressAutoHyphens" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_suppress_auto_hyphens(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:contextualSpacing" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_contextual_spacing(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:pageBreakBefore" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_page_break_before(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:framePr" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_frame_properties(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:suppressLineNumbers" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_suppress_line_numbers(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:keepLines" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_keep_lines(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:keepNext" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_keep_next(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:textAlignment" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_text_alignment(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:mirrorIndents" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_mirror_indents(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:outlineLvl" => {
                            if parsing_paragraph_props {
                                Self::apply_paragraph_outline_level(
                                    e.attributes(),
                                    &mut current_paragraph_props,
                                );
                            }
                        }
                        b"w:tabs" => {
                            if parsing_paragraph_props {
                                if let Ok(tabs) = Self::parse_paragraph_tabs(reader) {
                                    if let Some(ref mut props) = current_paragraph_props {
                                        props.tabs = tabs;
                                    }
                                }
                            } else {
                                Self::consume_element(reader, b"w:tabs")?;
                            }
                        }
                        b"w:pBdr" => {
                            if parsing_paragraph_props {
                                if let Ok(borders) = Self::parse_paragraph_borders(reader) {
                                    if !borders.is_empty() {
                                        current_paragraph_props
                                            .get_or_insert_with(ParagraphProperties::default)
                                            .borders = Some(borders);
                                    }
                                }
                            } else {
                                Self::consume_element(reader, b"w:pBdr")?;
                            }
                        }
                        b"w:pgSz" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let page_settings = section
                                        .page_settings
                                        .get_or_insert_with(PageSettings::default);
                                    for attr in e.attributes().flatten() {
                                        let key = attr.key.as_ref();
                                        let value =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        match key {
                                            b"w:w" => {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    page_settings.width = Some(v / 20.0);
                                                }
                                            }
                                            b"w:h" => {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    page_settings.height = Some(v / 20.0);
                                                }
                                            }
                                            b"w:orient" => {
                                                if !value.is_empty() {
                                                    page_settings.orientation = Some(value);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        b"w:pgMar" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let page_settings = section
                                        .page_settings
                                        .get_or_insert_with(PageSettings::default);
                                    for attr in e.attributes().flatten() {
                                        let key = attr.key.as_ref();
                                        let value =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        if let Ok(v) = value.parse::<f32>() {
                                            match key {
                                                b"w:left" => {
                                                    page_settings.margin_left = Some(v / 20.0)
                                                }
                                                b"w:right" => {
                                                    page_settings.margin_right = Some(v / 20.0)
                                                }
                                                b"w:top" => {
                                                    page_settings.margin_top = Some(v / 20.0)
                                                }
                                                b"w:bottom" => {
                                                    page_settings.margin_bottom = Some(v / 20.0)
                                                }
                                                b"w:header" => {
                                                    page_settings.header_distance = Some(v / 20.0)
                                                }
                                                b"w:footer" => {
                                                    page_settings.footer_distance = Some(v / 20.0)
                                                }
                                                b"w:gutter" => {
                                                    page_settings.gutter = Some(v / 20.0)
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        b"w:cols" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let mut columns = section.columns.take().unwrap_or_default();
                                    columns.columns.clear();
                                    Self::parse_columns_attributes(e.attributes(), &mut columns);
                                    section.columns = Some(columns);
                                }
                            }
                        }
                        b"w:pgBorders" => {
                            if parsing_section_props {
                                // Capture `w:offsetFrom` from the container before
                                // reading the child edge elements.
                                let offset_from = e
                                    .attributes()
                                    .flatten()
                                    .find(|a| a.key.as_ref() == b"w:offsetFrom")
                                    .map(|a| {
                                        String::from_utf8_lossy(&a.value)
                                            .trim()
                                            .to_ascii_lowercase()
                                    });
                                if let Ok(mut borders) = Self::parse_page_borders(reader) {
                                    borders.offset_from = offset_from;
                                    if !borders.is_empty() {
                                        if let Some(section) = current_section_props.as_mut() {
                                            section.page_borders = Some(borders);
                                        }
                                    }
                                }
                            } else {
                                Self::consume_element(reader, b"w:pgBorders")?;
                            }
                        }
                        b"w:docGrid" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let mut doc_grid = section.doc_grid.take().unwrap_or_default();
                                    Self::parse_doc_grid_attributes(e.attributes(), &mut doc_grid);
                                    section.doc_grid = Some(doc_grid);
                                }
                            }
                        }
                        b"w:pgNumType" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let pg_num = Self::parse_page_number_type(e.attributes());
                                    if !pg_num.is_empty() {
                                        section.page_number_type = Some(pg_num);
                                    }
                                }
                            }
                            Self::consume_element(reader, b"w:pgNumType")?;
                        }
                        b"w:lnNumType" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    let ln_num = Self::parse_line_numbering(e.attributes());
                                    if !ln_num.is_empty() {
                                        section.line_numbering = Some(ln_num);
                                    }
                                }
                            }
                            Self::consume_element(reader, b"w:lnNumType")?;
                        }
                        b"w:footnotePr" if parsing_section_props => {
                            let (note, _, beneath) =
                                Self::parse_note_numbering(reader, b"w:footnotePr")?;
                            if let Some(section) = current_section_props.as_mut() {
                                section.footnote_beneath_text = beneath;
                                if !note.is_empty() {
                                    section.footnote_properties = Some(note);
                                }
                            }
                        }
                        b"w:endnotePr" if parsing_section_props => {
                            let (note, position, _) =
                                Self::parse_note_numbering(reader, b"w:endnotePr")?;
                            if let Some(section) = current_section_props.as_mut() {
                                section.endnote_position = position;
                                if !note.is_empty() {
                                    section.endnote_properties = Some(note);
                                }
                            }
                        }
                        b"w:textDirection" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    for attr in e.attributes().flatten() {
                                        if attr.key.as_ref() == b"w:val" {
                                            let value =
                                                String::from_utf8_lossy(&attr.value).to_string();
                                            if let Some(direction) =
                                                TextDirection::try_from_str(value.as_str())
                                            {
                                                section.text_direction = Some(direction);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        b"w:vAlign" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    section.vertical_alignment =
                                        Self::parse_section_vertical_alignment(e.attributes());
                                }
                            }
                        }
                        b"w:sectPr" => {
                            parsing_section_props = true;
                            current_section_props = Some(SectionProperties::default());
                        }
                        b"w:type" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    for attr in e.attributes().flatten() {
                                        if attr.key.as_ref() == b"w:val" {
                                            section.section_type = Some(
                                                String::from_utf8_lossy(&attr.value).to_string(),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        b"w:rtlGutter" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    section.rtl_gutter = Self::parse_on_off_attr(e.attributes());
                                }
                            }
                        }
                        b"w:titlePg" => {
                            if parsing_section_props {
                                if let Some(section) = current_section_props.as_mut() {
                                    section.title_page = Self::parse_on_off_attr(e.attributes());
                                }
                            }
                        }
                        b"w:headerReference" => {
                            if parsing_section_props {
                                let rel_id = Self::process_header_footer_reference(
                                    e.attributes(),
                                    current_section_props.as_mut(),
                                    true,
                                );
                                if !parsing_paragraph_props {
                                    if let Some(id) = rel_id {
                                        document.header_reference = Some(id);
                                    }
                                }
                            }
                        }
                        b"w:footerReference" => {
                            if parsing_section_props {
                                let rel_id = Self::process_header_footer_reference(
                                    e.attributes(),
                                    current_section_props.as_mut(),
                                    false,
                                );
                                if !parsing_paragraph_props {
                                    if let Some(id) = rel_id {
                                        document.footer_reference = Some(id);
                                    }
                                }
                            }
                        }
                        b"w:r" => {
                            current_run = Some(Run {
                                text: String::new(),
                                properties: None,
                                image: None,
                                text_box: None,
                                math: None,
                                field: None,
                                hyperlink: hyperlink_stack.last().cloned(),
                                node_id: None,
                                text_node_id: None,
                                revision_node_id: None,
                                comment_reference_ids: Vec::new(),
                                footnote_reference_ids: Vec::new(),
                                endnote_reference_ids: Vec::new(),
                                note_separator: None,
                                has_footnote_ref: false,
                                has_endnote_ref: false,
                                revision: Self::current_revision(&revision_stack),
                            });
                            // `w:pPr/w:rPr` formats the paragraph mark. It is
                            // not a default property set for existing `w:r`
                            // children (ECMA-376 Part 1 §17.3.1.29).
                            current_run_props = Some(RunProperties::default());
                            current_run_image = None;
                            current_run_text_box = None;
                            let current_field = active_fields
                                .iter()
                                .rev()
                                .find_map(|ctx| ctx.kind.clone().map(|kind| (kind, ctx)));
                            current_run_field =
                                current_field.as_ref().map(|(kind, _)| kind.clone());
                            current_run_sequence_result = current_field
                                .as_ref()
                                .and_then(|(_, ctx)| ctx.sequence_result_run);
                            current_run_is_instruction_operand =
                                current_field.is_some_and(|(_, ctx)| ctx.is_instruction_operand);
                            current_run_field_text.clear();
                            current_run_has_field_end = false;
                            current_run_has_carriage_return = false;
                            current_run_has_attributed_soft_break = false;
                        }
                        // Ruby (phonetic guide, ECMA-376 Part 1 §17.3.3.25).
                        // `w:rubyBase` runs flow through the normal `w:r` path
                        // above; `w:rubyPr` / `w:rt` are consumed by subtree
                        // walks so the guide text never contaminates the body
                        // text (or selection offsets).
                        b"w14:glow" | b"w14:textOutline" | b"w14:stylisticSets"
                            if parsing_run_props =>
                        {
                            let name = e.name().as_ref().to_vec();
                            let size = e
                                .attributes()
                                .flatten()
                                .find(|attr| matches!(attr.key.as_ref(), b"w14:rad" | b"w14:w"))
                                .and_then(|attr| {
                                    String::from_utf8_lossy(&attr.value)
                                        .trim()
                                        .parse::<f32>()
                                        .ok()
                                })
                                .map(|emu| emu / 12_700.0);
                            let (color, alpha, set) = Self::parse_text_effect(reader, &name)?;
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    let effects =
                                        props.text_effects.get_or_insert_with(Default::default);
                                    match name.as_slice() {
                                        b"w14:glow" => {
                                            effects.glow_color = color.as_deref().map(Rc::from);
                                            effects.glow_radius = size;
                                            effects.glow_alpha = alpha;
                                        }
                                        b"w14:textOutline" => {
                                            effects.outline_color = color.as_deref().map(Rc::from);
                                            effects.outline_width = size;
                                        }
                                        _ => effects.stylistic_set = set,
                                    }
                                },
                            );
                        }
                        b"w:ruby" => {
                            pending_ruby = Some(PendingRuby {
                                base_run_index: current_paragraph
                                    .as_ref()
                                    .map(|p| p.runs.len())
                                    .unwrap_or(0),
                                text: String::new(),
                                size_pt: None,
                                align: None,
                                raise_half_points: None,
                                fonts: None,
                            });
                        }
                        b"w:rubyPr" => {
                            let (hps_pt, align, raise_half_points) =
                                Self::parse_ruby_properties(reader)?;
                            if let Some(pending) = pending_ruby.as_mut() {
                                if pending.size_pt.is_none() {
                                    pending.size_pt = hps_pt;
                                }
                                pending.align = align;
                                pending.raise_half_points = raise_half_points;
                            }
                        }
                        b"w:rt" => {
                            let (text, size_pt, fonts) = Self::parse_ruby_text(reader)?;
                            if let Some(pending) = pending_ruby.as_mut() {
                                pending.text.push_str(&text);
                                if fonts.is_some() {
                                    pending.fonts = fonts;
                                }
                                // The rt run's own `w:sz` wins over the
                                // `w:rubyPr/w:hps` fallback.
                                if size_pt.is_some() {
                                    pending.size_pt = size_pt;
                                }
                            }
                        }
                        b"m:oMathPara" => {
                            let raw = Self::capture_element_xml(reader, e.to_owned())?;
                            let mut math_paragraph = parse_math_paragraph(raw)
                                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
                            if current_paragraph.is_some() {
                                let mut properties = current_paragraph_props.clone();
                                if let Some(mark) = current_paragraph_run_props.clone() {
                                    properties
                                        .get_or_insert_with(ParagraphProperties::default)
                                        .mark_run_properties = Some(mark);
                                }
                                if let Some(id) = current_paragraph_para_id.as_ref() {
                                    properties
                                        .get_or_insert_with(ParagraphProperties::default)
                                        .para_id = Some(id.as_str().into());
                                }
                                math_paragraph.properties = properties.map(Box::new);
                            }
                            Self::push_block(
                                &mut document,
                                container_stack.as_mut_slice(),
                                BodyElement::from_math_paragraph(math_paragraph),
                            );
                            current_paragraph_is_math_wrapper = true;
                        }
                        b"m:oMath" => {
                            let raw = Self::capture_element_xml(reader, e.to_owned())?;
                            let elements = parse_math_elements(&raw)
                                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;

                            if let Some(run) = current_run.as_mut() {
                                if run.math.is_none() {
                                    run.math = Some(MathRun {
                                        raw_omml: raw,
                                        elements,
                                    });
                                }
                                continue;
                            }

                            if let Some(paragraph) = current_paragraph.as_mut() {
                                paragraph.runs.push(Run {
                                    text: String::new(),
                                    properties: None,
                                    image: None,
                                    text_box: None,
                                    math: Some(MathRun {
                                        raw_omml: raw,
                                        elements,
                                    }),
                                    field: None,
                                    hyperlink: hyperlink_stack.last().cloned(),
                                    node_id: None,
                                    text_node_id: None,
                                    revision_node_id: None,
                                    comment_reference_ids: Vec::new(),
                                    footnote_reference_ids: Vec::new(),
                                    endnote_reference_ids: Vec::new(),
                                    note_separator: None,
                                    has_footnote_ref: false,
                                    has_endnote_ref: false,
                                    revision: Self::current_revision(&revision_stack),
                                });
                            }
                        }
                        b"w:separator" | b"w:continuationSeparator" => {
                            if let Some(ref mut run) = current_run {
                                run.note_separator = Some(if e.name().as_ref() == b"w:separator" {
                                    NoteSeparatorKind::Separator
                                } else {
                                    NoteSeparatorKind::ContinuationSeparator
                                });
                            }
                        }
                        b"w:rPr" => {
                            parsing_run_props = true;
                            if current_run.is_some() {
                                parsing_paragraph_run_props = false;
                            } else if parsing_paragraph_props {
                                parsing_paragraph_run_props = true;
                                if current_paragraph_run_props.is_none() {
                                    current_paragraph_run_props = Some(RunProperties::default());
                                }
                            } else {
                                parsing_paragraph_run_props = false;
                            }
                        }
                        b"w:b" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.bold = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:bCs" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.bold_complex = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:rStyle" => {
                            if parsing_run_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let style_id =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.style = Some(Rc::from(style_id.as_str()));
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        b"w:lang" => {
                            if parsing_run_props {
                                let mut language = None;
                                let mut east_asia_language = None;
                                for attr in e.attributes().flatten() {
                                    match attr.key.as_ref() {
                                        b"w:val" => {
                                            language = Some(
                                                String::from_utf8_lossy(&attr.value).to_string(),
                                            )
                                        }
                                        b"w:eastAsia" => {
                                            east_asia_language = Some(
                                                String::from_utf8_lossy(&attr.value).to_string(),
                                            )
                                        }
                                        _ => {}
                                    }
                                }
                                if language.is_some() || east_asia_language.is_some() {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.language = language.as_deref().map(Rc::from);
                                            props.east_asia_language =
                                                east_asia_language.as_deref().map(Rc::from);
                                        },
                                    );
                                }
                            }
                        }
                        b"w:i" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.italic = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:iCs" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.italic_complex = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:strike" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.strike = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:dstrike" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.double_strike = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:caps" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.caps = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:smallCaps" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.small_caps = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:vanish" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.vanish = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:specVanish" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.spec_vanish = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:shadow" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.shadow = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:outline" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.outline = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:emboss" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.emboss = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:imprint" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let flag = parse_on_off(value, true);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.imprint = Some(flag);
                                    },
                                );
                            }
                        }
                        b"w:eastAsianLayout" => {
                            if parsing_run_props {
                                let layout = EastAsianLayout::from_attributes(
                                    e.attributes().flatten().map(|attr| {
                                        (
                                            match attr.key.as_ref() {
                                                b"w:id" => &b"w:id"[..],
                                                b"w:combine" => &b"w:combine"[..],
                                                b"w:combineBrackets" => &b"w:combineBrackets"[..],
                                                b"w:vert" => &b"w:vert"[..],
                                                b"w:vertCompress" => &b"w:vertCompress"[..],
                                                _ => &b""[..],
                                            },
                                            String::from_utf8_lossy(&attr.value).to_string(),
                                        )
                                    }),
                                );
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.east_asian_layout = Some(Box::new(layout.clone()));
                                    },
                                );
                            }
                        }
                        b"w:em" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let mark = normalize_emphasis_mark(value);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.emphasis = mark.as_deref().map(Rc::from);
                                    },
                                );
                            }
                        }
                        b"w:effect" => {
                            if parsing_run_props {
                                let value = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                                let effect = normalize_text_effect(value);
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.effect = effect.as_deref().map(Rc::from);
                                    },
                                );
                            }
                        }
                        b"w:vertAlign" => {
                            if parsing_run_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let value = String::from_utf8_lossy(&attr.value)
                                            .to_ascii_lowercase();
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.baseline = Some(Rc::from(value.as_str()));
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        b"w:u" => {
                            if parsing_run_props {
                                let parsed = parse_underline(e.attributes());
                                if parsed.on_off.is_some()
                                    || parsed.kind.is_some()
                                    || parsed.color.is_some()
                                {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            if let Some(flag) = parsed.on_off {
                                                props.underline = Some(flag);
                                            }
                                            props.underline_kind =
                                                parsed.kind.as_deref().map(Rc::from);
                                            props.underline_color =
                                                parsed.color.as_deref().map(Rc::from);
                                        },
                                    );
                                }
                            }
                        }
                        b"w:sz" => {
                            if parsing_run_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        if let Ok(val) =
                                            String::from_utf8_lossy(&attr.value).parse::<f32>()
                                        {
                                            let size = val / 2.0;
                                            Self::apply_to_active_run_props(
                                                parsing_paragraph_run_props,
                                                &mut current_paragraph_run_props,
                                                &mut current_run_props,
                                                |props| {
                                                    props.font_size = Some(size);
                                                },
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        b"w:szCs" => {
                            if parsing_run_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        if let Ok(val) =
                                            String::from_utf8_lossy(&attr.value).parse::<f32>()
                                        {
                                            let size = val / 2.0;
                                            Self::apply_to_active_run_props(
                                                parsing_paragraph_run_props,
                                                &mut current_paragraph_run_props,
                                                &mut current_run_props,
                                                |props| {
                                                    props.font_size_complex = Some(size);
                                                },
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        b"w:kern" => {
                            if parsing_run_props {
                                if let Some(points) = parse_kerning_points(e.attributes()) {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.kerning = Some(points);
                                        },
                                    );
                                }
                            }
                        }
                        b"w:w" => {
                            if parsing_run_props {
                                if let Some(percent) = parse_text_scale_percent(e.attributes()) {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.text_scale = Some(percent);
                                        },
                                    );
                                }
                            }
                        }
                        b"w:position" => {
                            if parsing_run_props {
                                if let Some(points) = parse_text_position_points(e.attributes()) {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.text_position = Some(points);
                                        },
                                    );
                                }
                            }
                        }
                        b"w:fitText" => {
                            if parsing_run_props {
                                if let Some(fit_text) = parse_fit_text(e.attributes()) {
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.fit_text = Some(fit_text.clone());
                                        },
                                    );
                                }
                            }
                        }
                        b"w:color" => {
                            if parsing_run_props {
                                let theme = Self::run_theme_color(e.attributes());
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.theme_color = theme.clone().map(Box::new);
                                    },
                                );
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let color_val =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        let formatted = format!("#{}", color_val);
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.color = Some(Rc::from(formatted.as_str()));
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        b"w:highlight" => {
                            if parsing_run_props {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let val = String::from_utf8_lossy(&attr.value).to_string();
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.highlight = Some(Rc::from(val.as_str()));
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        b"w:bdr" => {
                            if parsing_run_props {
                                let edge = Self::build_border_edge(e.attributes());
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.border = Some(edge.clone());
                                    },
                                );
                            }
                        }
                        b"w:rFonts" => {
                            if parsing_run_props {
                                let attrs: Vec<(Vec<u8>, String)> = e
                                    .attributes()
                                    .flatten()
                                    .map(|attr| {
                                        (
                                            attr.key.as_ref().to_vec(),
                                            String::from_utf8_lossy(&attr.value).to_string(),
                                        )
                                    })
                                    .collect();
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.authored_east_asia_font_hint =
                                            Some(attrs.iter().any(|(key, value)| {
                                                key.as_slice() == b"w:hint"
                                                    && FontHint::from_xml_hint(value)
                                                        == Some(FontHint::EastAsia)
                                            }));
                                        props.authored_east_asia_font_name =
                                            Some(attrs.iter().any(|(key, _)| {
                                                matches!(
                                                    key.as_slice(),
                                                    b"w:eastAsia" | b"w:eastAsiaTheme"
                                                )
                                            }));
                                        let fonts =
                                            props.fonts.get_or_insert_with(RunFonts::default);
                                        for (key, value) in &attrs {
                                            match key.as_slice() {
                                                b"w:ascii" => {
                                                    fonts.ascii = Some(Rc::from(value.as_str()));
                                                }
                                                b"w:hAnsi" => {
                                                    fonts.h_ansi = Some(Rc::from(value.as_str()));
                                                }
                                                b"w:eastAsia" => {
                                                    fonts.east_asia =
                                                        Some(Rc::from(value.as_str()));
                                                }
                                                b"w:cs" => {
                                                    fonts.complex = Some(Rc::from(value.as_str()));
                                                }
                                                b"w:asciiTheme" => {
                                                    fonts.ascii_theme =
                                                        Some(Rc::from(value.as_str()));
                                                }
                                                b"w:hAnsiTheme" => {
                                                    fonts.h_ansi_theme =
                                                        Some(Rc::from(value.as_str()));
                                                }
                                                b"w:eastAsiaTheme" => {
                                                    fonts.east_asia_theme =
                                                        Some(Rc::from(value.as_str()));
                                                }
                                                b"w:cstheme" => {
                                                    fonts.complex_theme =
                                                        Some(Rc::from(value.as_str()));
                                                }
                                                b"w:hint" => {
                                                    fonts.hint = FontHint::from_xml_hint(value);
                                                }
                                                _ => {}
                                            }
                                        }
                                        fonts.drop_names_shadowed_by_themes();
                                        if props.font_family.is_none() {
                                            if let Some(family) = fonts.preferred_latin() {
                                                props.font_family = Some(Rc::from(family));
                                            }
                                        }
                                    },
                                );
                            }
                        }
                        b"w:t" | b"w:delText" => {
                            in_text = true;
                            current_text.clear();
                            current_text_preserve = e.attributes().flatten().any(|attr| {
                                attr.key.as_ref() == b"xml:space"
                                    && std::str::from_utf8(attr.value.as_ref())
                                        .map(|value| value.eq_ignore_ascii_case("preserve"))
                                        .unwrap_or(false)
                            });
                        }
                        b"w:drawing" => {
                            if let Some(drawing) = self.parse_drawing(reader)? {
                                match drawing {
                                    DrawingObject::Image(image) => {
                                        current_run_image = Some(image);
                                    }
                                    DrawingObject::TextBox(text_box) => {
                                        current_run_text_box = Some(text_box);
                                    }
                                }
                            }
                        }
                        b"w:pict" => {
                            // Word writes a shape as `mc:AlternateContent`: the
                            // DrawingML choice, then a VML fallback (`w:pict`).
                            // A consumer that understands the choice ignores the
                            // fallback (MCE), and Word does; letting the later
                            // VML override the parsed drawing made a table or
                            // wrap change present only in the choice disappear
                            // on reopen.
                            if current_run_text_box.is_some() || current_run_image.is_some() {
                                Self::consume_element(reader, b"w:pict")?;
                            } else if let Some(drawing) = self.parse_vml_picture(reader)? {
                                match drawing {
                                    DrawingObject::Image(image) => current_run_image = Some(image),
                                    DrawingObject::TextBox(text_box) => {
                                        current_run_text_box = Some(text_box)
                                    }
                                }
                            }
                        }
                        b"w:object" => {
                            let mut width_hint_px = None;
                            let mut height_hint_px = None;
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"w:dxaOrig" => {
                                        if let Some(val) = Self::parse_f64_attr(&attr) {
                                            width_hint_px = Some(Self::dxa_to_px(val));
                                        }
                                    }
                                    b"w:dyaOrig" => {
                                        if let Some(val) = Self::parse_f64_attr(&attr) {
                                            height_hint_px = Some(Self::dxa_to_px(val));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if let Some(image) =
                                self.parse_vml_ole_object(reader, width_hint_px, height_hint_px)?
                            {
                                if current_run.is_some() {
                                    current_run_image = Some(image);
                                } else if let Some(paragraph) = current_paragraph.as_mut() {
                                    paragraph.runs.push(Run {
                                        text: String::new(),
                                        properties: None,
                                        image: Some(image),
                                        text_box: None,
                                        math: None,
                                        field: None,
                                        hyperlink: hyperlink_stack.last().cloned(),
                                        node_id: None,
                                        text_node_id: None,
                                        revision_node_id: None,
                                        comment_reference_ids: Vec::new(),
                                        footnote_reference_ids: Vec::new(),
                                        endnote_reference_ids: Vec::new(),
                                        note_separator: None,
                                        has_footnote_ref: false,
                                        has_endnote_ref: false,
                                        revision: Self::current_revision(&revision_stack),
                                    });
                                }
                            }
                        }
                        b"w:fldChar" => {
                            current_run_has_field_end |=
                                e.attributes().flatten().any(|attribute| {
                                    attribute.key.as_ref() == b"w:fldCharType"
                                        && attribute.value.as_ref() == b"end"
                                });
                            if let Some(kind) = Self::process_fld_char_event(
                                e.attributes(),
                                &mut active_fields,
                                &resolved_bookmark_text,
                                &mut deferred_nested_fields,
                                &mut document.toc_fields,
                                &mut document.toc_entry_fields,
                                document.body.as_ref().map_or(0, |body| body.elements.len()),
                            ) {
                                current_run_field = Some(kind);
                                current_run_field_text.clear();
                            }
                        }
                        b"w:instrText" => {
                            in_instr_text = true;
                            current_instr_text.clear();
                        }
                        b"w:softHyphen" => {
                            // Expanded empty syntax is unusual but valid XML.
                            // Preserve the optional hyphen as U+00AD just like
                            // the normal self-closing form handled below.
                            if Self::keeps_field_result_in_run_text(
                                current_run_field.as_ref(),
                                current_run_is_instruction_operand,
                            ) {
                                if let Some(ref mut run) = current_run {
                                    run.text.push('\u{00AD}');
                                    paragraph_text_len += '\u{00AD}'.len_utf8();
                                }
                            }
                        }
                        b"w:fldSimple" => {
                            Self::process_fld_simple_start(
                                e.attributes(),
                                &mut active_fields,
                                &mut document.toc_fields,
                                &mut document.toc_entry_fields,
                                document.body.as_ref().map_or(0, |body| body.elements.len()),
                                current_paragraph
                                    .as_ref()
                                    .map_or(0, |paragraph| paragraph.runs.len()),
                            );
                        }
                        b"w:pgNum" => {
                            // Legacy page-number block (§17.3.3.22). Unlike a
                            // complex field it has no instruction/result runs;
                            // normalize it directly to the live PAGE field.
                            if current_run.is_some() && current_run_field.is_none() {
                                current_run_field = Some(RunFieldKind::PageNumber);
                                current_run_field_text.clear();
                            }
                        }
                        b"w:background" => {
                            // `<w:document>` child: page background color. Any
                            // child shape (`w:drawing`) flows through the
                            // catch-all and is ignored (color only, MVP).
                            if let Some(color) = Self::parse_document_background(e.attributes()) {
                                document.background_color = Some(color);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    // A tracked change on the paragraph mark (`w:pPr/w:rPr/w:del`).
                    b"w:ins" | b"w:del" | b"w:moveFrom" | b"w:moveTo" => {
                        if parsing_paragraph_run_props {
                            if let Some(props) = current_paragraph_props.as_mut() {
                                props.mark_revision = Some(Box::new(Self::build_revision_mark(
                                    e,
                                    Self::revision_kind(e),
                                )));
                            }
                        }
                    }
                    b"w:sectPr" => {
                        let section = SectionProperties::default();
                        if parsing_paragraph_props {
                            if let Some(props) = current_paragraph_props.as_mut() {
                                props.section = Some(section);
                            }
                        } else {
                            default_section_props = Some(section);
                        }
                    }
                    b"w:altChunk" => {
                        Self::record_alt_chunk(&mut document, e.attributes());
                    }
                    b"w:background" => {
                        if let Some(color) = Self::parse_document_background(e.attributes()) {
                            document.background_color = Some(color);
                        }
                    }
                    b"w:p" => {
                        let props = Self::parse_para_id(e.attributes()).map(|para_id| {
                            ParagraphProperties {
                                para_id: Some(para_id.into()),
                                ..ParagraphProperties::default()
                            }
                        });
                        Self::push_paragraph(
                            &mut document,
                            container_stack.as_mut_slice(),
                            Paragraph {
                                runs: Vec::new(),
                                properties: None,
                                node_id: None,
                                comment_ranges: Vec::new(),
                                comment_markers: Vec::new(),
                            },
                            props,
                            Vec::new(),
                            Vec::new(),
                        );
                    }
                    b"w:fldChar" => {
                        current_run_has_field_end |= e.attributes().flatten().any(|attribute| {
                            attribute.key.as_ref() == b"w:fldCharType"
                                && attribute.value.as_ref() == b"end"
                        });
                        if let Some(kind) = Self::process_fld_char_event(
                            e.attributes(),
                            &mut active_fields,
                            &resolved_bookmark_text,
                            &mut deferred_nested_fields,
                            &mut document.toc_fields,
                            &mut document.toc_entry_fields,
                            document.body.as_ref().map_or(0, |body| body.elements.len()),
                        ) {
                            current_run_field = Some(kind);
                            current_run_field_text.clear();
                        }
                    }
                    b"w:pgNum" => {
                        // `w:pgNum` is normally empty (`<w:pgNum/>`), but also
                        // accept expanded empty syntax for tolerant OOXML input.
                        if current_run.is_some() && current_run_field.is_none() {
                            current_run_field = Some(RunFieldKind::PageNumber);
                            current_run_field_text.clear();
                        }
                    }
                    b"w:br" => {
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            let mut break_type: Option<String> = None;
                            let mut has_clear = false;
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"w:type" => {
                                        break_type = Some(
                                            String::from_utf8_lossy(&attr.value)
                                                .to_ascii_lowercase(),
                                        );
                                    }
                                    b"w:clear" => has_clear = true,
                                    _ => {}
                                }
                            }
                            if let Some(ref mut run) = current_run {
                                // ECMA-376 Part 1 §17.3.3.1 `w:br/@w:type`. The
                                // text-length counter is bumped only when a
                                // character is actually appended — otherwise an
                                // unhandled break type desynced `paragraph_text_len`
                                // from the run text and shifted later offsets.
                                match break_type.as_deref() {
                                    Some("page") => {
                                        run.text.push(PAGE_BREAK_CHAR);
                                        paragraph_text_len += 1;
                                    }
                                    // The next column, or the next page after
                                    // the last one.
                                    Some("column") => {
                                        run.text.push(crate::document::COLUMN_BREAK_CHAR);
                                        paragraph_text_len += 1;
                                    }
                                    Some("textwrapping") | None => {
                                        run.text.push('\n');
                                        paragraph_text_len += 1;
                                        // A clear value changes Word's float
                                        // avoidance behavior. Keep this run
                                        // intact so the editor can map it back
                                        // to the attributed raw `w:br` element.
                                        current_run_has_attributed_soft_break |= has_clear;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    b"w:lastRenderedPageBreak" => {
                        // This is the previous application's pagination cache,
                        // not an explicit break instruction. Live layout must
                        // repaginate from the current content and metrics. The
                        // round-trip XML tree preserves the marker for export.
                    }
                    b"w:cr" => {
                        // ECMA-376 Part 1 §17.3.3.4 defines `w:cr` as a
                        // carriage return with the same line-ending behavior as
                        // an untyped break. Keep it visible in the logical text
                        // so edit/export round trips can preserve the boundary.
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            if let Some(ref mut run) = current_run {
                                run.text.push('\n');
                                paragraph_text_len += 1;
                                current_run_has_carriage_return = true;
                            }
                        }
                    }
                    b"w:fldSimple" => {
                        let previous_depth = active_fields.len();
                        Self::process_fld_simple_start(
                            e.attributes(),
                            &mut active_fields,
                            &mut document.toc_fields,
                            &mut document.toc_entry_fields,
                            document.body.as_ref().map_or(0, |body| body.elements.len()),
                            current_paragraph
                                .as_ref()
                                .map_or(0, |paragraph| paragraph.runs.len()),
                        );
                        // An unmodeled empty field must not close its enclosing field.
                        if active_fields.len() > previous_depth {
                            Self::finish_simple_field(
                                &mut active_fields,
                                current_paragraph.as_mut(),
                                &resolved_bookmark_text,
                                &mut deferred_nested_fields,
                            );
                        }
                    }
                    b"w:pStyle" => {
                        if parsing_paragraph_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let style_id = String::from_utf8_lossy(&attr.value).to_string();
                                    if let Some(ref mut props) = current_paragraph_props {
                                        props.style = Some(style_id.into());
                                    }
                                }
                            }
                        }
                    }
                    b"w:highlight" => {
                        if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let val = String::from_utf8_lossy(&attr.value).to_string();
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.highlight = Some(val.as_str().into());
                                        },
                                    );
                                }
                            }
                        }
                    }
                    b"w:commentRangeStart" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                let id = String::from_utf8_lossy(&attr.value).to_string();
                                open_comment_ranges.insert(id, paragraph_text_len);
                                break;
                            }
                        }
                    }
                    b"w:bookmarkStart" => {
                        let (id, name) = Self::extract_bookmark_start_attributes(e.attributes());
                        if let Some(id) = id {
                            // Word reserves `_GoBack` for its own cursor
                            // tracking; we still store it so round-tripping
                            // preserves it, but the empty name is tolerated.
                            let paragraph_index = document
                                .body
                                .as_ref()
                                .map(|body| body.elements.len())
                                .unwrap_or(0);
                            open_bookmarks.insert(
                                id,
                                (
                                    name.unwrap_or_default(),
                                    paragraph_index,
                                    paragraph_text_len,
                                    current_paragraph
                                        .as_ref()
                                        .map_or(0, |paragraph| paragraph.runs.len()),
                                ),
                            );
                        }
                    }
                    b"w:bookmarkEnd" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                let id = String::from_utf8_lossy(&attr.value).to_string();
                                if let Some((name, start_paragraph, start_offset, start_run)) =
                                    open_bookmarks.remove(&id)
                                {
                                    let end_paragraph = document
                                        .body
                                        .as_ref()
                                        .map(|body| body.elements.len())
                                        .unwrap_or(start_paragraph);
                                    let end_offset = paragraph_text_len;
                                    let end_run = current_paragraph
                                        .as_ref()
                                        .map_or(start_run, |paragraph| paragraph.runs.len());
                                    // A bookmark closed within the current
                                    // paragraph is already complete even though
                                    // the paragraph has not reached `</w:p>`.
                                    // Publish that slice now so a following
                                    // nested field in the same paragraph can
                                    // consume its live text.
                                    let inline_text = (start_paragraph == end_paragraph)
                                        .then_some(current_paragraph.as_ref())
                                        .flatten()
                                        .and_then(|paragraph| {
                                            let text = crate::document::paragraph_concatenated_text(
                                                paragraph,
                                            );
                                            crate::document::slice_byte_range(
                                                &text,
                                                start_offset,
                                                end_offset,
                                            )
                                        });
                                    document.bookmarks.push(Bookmark {
                                        id,
                                        name: name.clone(),
                                        start_paragraph,
                                        start_offset,
                                        start_run,
                                        end_paragraph,
                                        end_offset,
                                        end_run,
                                    });
                                    if let Some(text) = inline_text {
                                        resolved_bookmark_text.entry(name).or_insert(text);
                                    }
                                }
                                break;
                            }
                        }
                    }
                    b"w:cols" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let mut columns = SectionColumns::default();
                                Self::parse_columns_attributes(e.attributes(), &mut columns);
                                section.columns = Some(columns);
                            }
                        }
                    }
                    b"w:type" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                section.section_type = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"w:val")
                                    .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            }
                        }
                    }
                    b"w:docGrid" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let mut doc_grid = section.doc_grid.take().unwrap_or_default();
                                Self::parse_doc_grid_attributes(e.attributes(), &mut doc_grid);
                                section.doc_grid = Some(doc_grid);
                            }
                        }
                    }
                    b"w:textDirection" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"w:val" {
                                        let value =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        if let Some(direction) =
                                            TextDirection::try_from_str(value.as_str())
                                        {
                                            section.text_direction = Some(direction);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"w:pgNumType" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let pg_num = Self::parse_page_number_type(e.attributes());
                                if !pg_num.is_empty() {
                                    section.page_number_type = Some(pg_num);
                                }
                            }
                        }
                    }
                    b"w:lnNumType" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let ln_num = Self::parse_line_numbering(e.attributes());
                                if !ln_num.is_empty() {
                                    section.line_numbering = Some(ln_num);
                                }
                            }
                        }
                    }
                    b"w:rtlGutter" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                section.rtl_gutter = Self::parse_on_off_attr(e.attributes());
                            }
                        }
                    }
                    b"w:titlePg" => {
                        // `<w:titlePg/>` (with no `w:val` or `w:val="true"|"1"|"on"`)
                        // enables the first-page header/footer for the current
                        // section. Explicit `w:val="false"|"0"|"off"` turns it
                        // back off (matches Word's toggling semantics).
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                section.title_page = Self::parse_on_off_attr(e.attributes());
                            }
                        }
                    }
                    b"w:vAlign" => {
                        // Section-level vertical justification (`<w:vAlign w:val="center"/>`).
                        // Word emits this self-closing, so the Empty branch is the
                        // common one; the Start branch above covers the rare
                        // non-self-closing form.
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                section.vertical_alignment =
                                    Self::parse_section_vertical_alignment(e.attributes());
                            }
                        }
                    }
                    b"w:col" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                if let Some(columns) = section.columns.as_mut() {
                                    let mut definition = ColumnDefinition::default();
                                    for attr in e.attributes().flatten() {
                                        let key = attr.key.as_ref();
                                        let value =
                                            String::from_utf8_lossy(&attr.value).to_string();
                                        match key {
                                            b"w:w" => {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    definition.width = Some(v / 20.0);
                                                }
                                            }
                                            b"w:space" => {
                                                if let Ok(v) = value.parse::<f32>() {
                                                    definition.space = Some(v / 20.0);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    columns.columns.push(definition);
                                }
                            }
                        }
                    }
                    b"w:commentRangeEnd" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                let id = String::from_utf8_lossy(&attr.value).to_string();
                                if let Some(start) = open_comment_ranges.remove(&id) {
                                    paragraph_comment_ranges.push(CommentRange {
                                        id: id.clone(),
                                        start,
                                        end: paragraph_text_len,
                                    });
                                }
                                break;
                            }
                        }
                    }
                    b"w:commentReference" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                let id = String::from_utf8_lossy(&attr.value).to_string();
                                if let Some(ref mut run) = current_run {
                                    run.comment_reference_ids.push(id.clone());
                                }
                                paragraph_comment_markers.push(CommentMarker {
                                    id,
                                    position: paragraph_text_len,
                                });
                                break;
                            }
                        }
                    }
                    b"w:footnoteReference" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                if let Some(ref mut run) = current_run {
                                    run.footnote_reference_ids
                                        .push(String::from_utf8_lossy(&attr.value).to_string());
                                }
                                break;
                            }
                        }
                    }
                    b"w:endnoteReference" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"w:id" {
                                if let Some(ref mut run) = current_run {
                                    run.endnote_reference_ids
                                        .push(String::from_utf8_lossy(&attr.value).to_string());
                                }
                                break;
                            }
                        }
                    }
                    b"w:separator" | b"w:continuationSeparator" => {
                        if let Some(ref mut run) = current_run {
                            run.note_separator = Some(if e.name().as_ref() == b"w:separator" {
                                NoteSeparatorKind::Separator
                            } else {
                                NoteSeparatorKind::ContinuationSeparator
                            });
                        }
                    }
                    b"w:footnoteRef" => {
                        if let Some(ref mut run) = current_run {
                            run.has_footnote_ref = true;
                        }
                    }
                    b"w:endnoteRef" => {
                        if let Some(ref mut run) = current_run {
                            run.has_endnote_ref = true;
                        }
                    }
                    b"w:headerReference" => {
                        if parsing_section_props {
                            let rel_id = Self::process_header_footer_reference(
                                e.attributes(),
                                current_section_props.as_mut(),
                                true,
                            );
                            if !parsing_paragraph_props {
                                if let Some(id) = rel_id {
                                    document.header_reference = Some(id);
                                }
                            }
                        }
                    }
                    b"w:footerReference" => {
                        if parsing_section_props {
                            let rel_id = Self::process_header_footer_reference(
                                e.attributes(),
                                current_section_props.as_mut(),
                                false,
                            );
                            if !parsing_paragraph_props {
                                if let Some(id) = rel_id {
                                    document.footer_reference = Some(id);
                                }
                            }
                        }
                    }
                    b"w:tab" => {
                        if let Some(ref mut run) = current_run {
                            if Self::keeps_field_result_in_run_text(
                                current_run_field.as_ref(),
                                current_run_is_instruction_operand,
                            ) {
                                run.text.push('\t');
                                paragraph_text_len += 1;
                            }
                        }
                    }
                    b"w:ptab" => {
                        // Absolute position tab (ECMA-376 Part 1 §17.3.3.16).
                        // We do not resolve the absolute alignment/leader yet, but
                        // dropping it loses the tab entirely; emit a regular tab
                        // so the gap is preserved instead of vanishing.
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            if let Some(ref mut run) = current_run {
                                run.text.push('\t');
                                paragraph_text_len += 1;
                            }
                        }
                    }
                    b"w:noBreakHyphen" => {
                        // ECMA-376 Part 1 §17.3.3.18: a non-breaking hyphen. If
                        // dropped the hyphen character goes missing from the run
                        // text. Emit U+2011 (NON-BREAKING HYPHEN) so it renders
                        // as "-" and the wrap engine never breaks at it.
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            if let Some(ref mut run) = current_run {
                                run.text.push('\u{2011}');
                                paragraph_text_len += '\u{2011}'.len_utf8();
                            }
                        }
                    }
                    b"w:softHyphen" => {
                        // ECMA-376 Part 1 §17.3.3.29: optional hyphen. U+00AD
                        // carries the same discretionary-break semantics while
                        // keeping selection/comment offsets aligned.
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            if let Some(ref mut run) = current_run {
                                run.text.push('\u{00AD}');
                                paragraph_text_len += '\u{00AD}'.len_utf8();
                            }
                        }
                    }
                    b"w:sym" => {
                        // `w:sym` references a glyph in a specific symbol font
                        // (ECMA-376 Part 1 §17.3.3.30). Decode the hex `w:char`
                        // into a code point so the glyph renders instead of the
                        // run becoming empty, and stamp the symbol font onto the
                        // run so it is drawn with the correct typeface.
                        if Self::keeps_field_result_in_run_text(
                            current_run_field.as_ref(),
                            current_run_is_instruction_operand,
                        ) {
                            let mut sym_char: Option<char> = None;
                            let mut sym_font: Option<String> = None;
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"w:char" => {
                                        let raw = String::from_utf8_lossy(&attr.value);
                                        sym_char = u32::from_str_radix(raw.trim(), 16)
                                            .ok()
                                            .and_then(char::from_u32);
                                    }
                                    b"w:font" => {
                                        sym_font =
                                            Some(String::from_utf8_lossy(&attr.value).to_string());
                                    }
                                    _ => {}
                                }
                            }
                            if let (Some(ref mut run), Some(ch)) = (current_run.as_mut(), sym_char)
                            {
                                run.text.push(ch);
                                paragraph_text_len += ch.len_utf8();
                                if let Some(font) = sym_font {
                                    let props = current_run_props
                                        .get_or_insert_with(RunProperties::default);
                                    let fonts = props.fonts.get_or_insert_with(RunFonts::default);
                                    let font: Rc<str> = Rc::from(font.as_str());
                                    fonts.ascii = Some(font.clone());
                                    fonts.h_ansi = Some(font);
                                }
                            }
                        }
                    }
                    b"w:numPr" => {
                        if parsing_paragraph_props {
                            if let Some(ref mut props) = current_paragraph_props {
                                if props.numbering.is_none() {
                                    props.numbering = Some(crate::document::Numbering {
                                        id: None,
                                        level: None,
                                    });
                                }
                            }
                        }
                    }
                    b"w:numId" => {
                        if parsing_paragraph_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let num_id = String::from_utf8_lossy(&attr.value).to_string();
                                    if let Some(ref mut props) = current_paragraph_props {
                                        if let Some(ref mut numbering) = props.numbering {
                                            numbering.id = Some(num_id.clone());
                                        } else {
                                            props.numbering = Some(crate::document::Numbering {
                                                id: Some(num_id.clone()),
                                                level: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"w:ilvl" => {
                        if parsing_paragraph_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(level) =
                                        String::from_utf8_lossy(&attr.value).parse::<u32>()
                                    {
                                        if let Some(ref mut props) = current_paragraph_props {
                                            if let Some(ref mut numbering) = props.numbering {
                                                numbering.level = Some(level);
                                            } else {
                                                props.numbering =
                                                    Some(crate::document::Numbering {
                                                        id: None,
                                                        level: Some(level),
                                                    });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"w:pict" => {
                        // See the body parser: the DrawingML choice already
                        // parsed for this run wins over the VML fallback.
                        if current_run_text_box.is_some() || current_run_image.is_some() {
                            Self::consume_element(reader, b"w:pict")?;
                        } else if let Some(drawing) = self.parse_vml_picture(reader)? {
                            match drawing {
                                DrawingObject::Image(image) => current_run_image = Some(image),
                                DrawingObject::TextBox(text_box) => {
                                    current_run_text_box = Some(text_box)
                                }
                            }
                        }
                    }
                    b"w:strike" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.strike = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:dstrike" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.double_strike = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:b" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.bold = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:bCs" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.bold_complex = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:iCs" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.italic_complex = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:caps" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.caps = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:smallCaps" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.small_caps = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:vanish" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.vanish = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:specVanish" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.spec_vanish = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:shadow" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.shadow = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:outline" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.outline = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:emboss" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.emboss = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:imprint" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.imprint = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:bdr" => {
                        if parsing_run_props {
                            let edge = Self::build_border_edge(e.attributes());
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.border = Some(edge.clone());
                                },
                            );
                        }
                    }
                    b"w14:ligatures" | b"w14:numForm" | b"w14:numSpacing" | b"w14:cntxtAlts"
                        if parsing_run_props =>
                    {
                        let name = e.name().as_ref().to_vec();
                        let value = e
                            .attributes()
                            .flatten()
                            .find(|attr| attr.key.as_ref() == b"w14:val")
                            .map(|attr| String::from_utf8_lossy(&attr.value).trim().to_string());
                        Self::apply_to_active_run_props(
                            parsing_paragraph_run_props,
                            &mut current_paragraph_run_props,
                            &mut current_run_props,
                            |props| {
                                let effects =
                                    props.text_effects.get_or_insert_with(Default::default);
                                let token = value.as_deref().map(Rc::from);
                                match name.as_slice() {
                                    b"w14:ligatures" => effects.ligatures = token.clone(),
                                    b"w14:numForm" => effects.number_form = token.clone(),
                                    b"w14:numSpacing" => effects.number_spacing = token.clone(),
                                    _ => {
                                        effects.contextual_alternates = Some(!matches!(
                                            value.as_deref(),
                                            Some("0" | "false" | "off")
                                        ))
                                    }
                                }
                            },
                        );
                    }
                    b"w:eastAsianLayout" => {
                        if parsing_run_props {
                            let layout = EastAsianLayout::from_attributes(
                                e.attributes().flatten().map(|attr| {
                                    (
                                        match attr.key.as_ref() {
                                            b"w:id" => &b"w:id"[..],
                                            b"w:combine" => &b"w:combine"[..],
                                            b"w:combineBrackets" => &b"w:combineBrackets"[..],
                                            b"w:vert" => &b"w:vert"[..],
                                            b"w:vertCompress" => &b"w:vertCompress"[..],
                                            _ => &b""[..],
                                        },
                                        String::from_utf8_lossy(&attr.value).to_string(),
                                    )
                                }),
                            );
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.east_asian_layout = Some(Box::new(layout.clone()));
                                },
                            );
                        }
                    }
                    b"w:em" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let mark = normalize_emphasis_mark(value);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.emphasis = mark.as_deref().map(Rc::from);
                                },
                            );
                        }
                    }
                    b"w:effect" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let effect = normalize_text_effect(value);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.effect = effect.as_deref().map(Rc::from);
                                },
                            );
                        }
                    }
                    b"w:rStyle" => {
                        if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let style_id = String::from_utf8_lossy(&attr.value).to_string();
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.style = Some(Rc::from(style_id.as_str()));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    b"w:lang" => {
                        if parsing_run_props {
                            let mut language = None;
                            let mut east_asia_language = None;
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"w:val" => {
                                        language =
                                            Some(String::from_utf8_lossy(&attr.value).to_string())
                                    }
                                    b"w:eastAsia" => {
                                        east_asia_language =
                                            Some(String::from_utf8_lossy(&attr.value).to_string())
                                    }
                                    _ => {}
                                }
                            }
                            if language.is_some() || east_asia_language.is_some() {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.language = language.as_deref().map(Rc::from);
                                        props.east_asia_language =
                                            east_asia_language.as_deref().map(Rc::from);
                                    },
                                );
                            }
                        }
                    }
                    b"w:i" => {
                        if parsing_run_props {
                            let value = e
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"w:val")
                                .map(|attr| String::from_utf8_lossy(&attr.value).to_string());
                            let flag = parse_on_off(value, true);
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.italic = Some(flag);
                                },
                            );
                        }
                    }
                    b"w:vertAlign" => {
                        if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let value =
                                        String::from_utf8_lossy(&attr.value).to_ascii_lowercase();
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.baseline = Some(Rc::from(value.as_str()));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    b"w:u" => {
                        if parsing_run_props {
                            let parsed = parse_underline(e.attributes());
                            if parsed.on_off.is_some()
                                || parsed.kind.is_some()
                                || parsed.color.is_some()
                            {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        if let Some(flag) = parsed.on_off {
                                            props.underline = Some(flag);
                                        }
                                        props.underline_kind = parsed.kind.as_deref().map(Rc::from);
                                        props.underline_color =
                                            parsed.color.as_deref().map(Rc::from);
                                    },
                                );
                            }
                        }
                    }
                    b"w:sz" => {
                        if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(val) =
                                        String::from_utf8_lossy(&attr.value).parse::<f32>()
                                    {
                                        let size = val / 2.0;
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.font_size = Some(size);
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                    b"w:szCs" => {
                        if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(val) =
                                        String::from_utf8_lossy(&attr.value).parse::<f32>()
                                    {
                                        let size = val / 2.0;
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.font_size_complex = Some(size);
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                    b"w:kern" => {
                        if parsing_run_props {
                            if let Some(points) = parse_kerning_points(e.attributes()) {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.kerning = Some(points);
                                    },
                                );
                            }
                        }
                    }
                    b"w:w" => {
                        if parsing_run_props {
                            if let Some(percent) = parse_text_scale_percent(e.attributes()) {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.text_scale = Some(percent);
                                    },
                                );
                            }
                        }
                    }
                    b"w:position" => {
                        if parsing_run_props {
                            if let Some(points) = parse_text_position_points(e.attributes()) {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.text_position = Some(points);
                                    },
                                );
                            }
                        }
                    }
                    b"w:fitText" => {
                        if parsing_run_props {
                            if let Some(fit_text) = parse_fit_text(e.attributes()) {
                                Self::apply_to_active_run_props(
                                    parsing_paragraph_run_props,
                                    &mut current_paragraph_run_props,
                                    &mut current_run_props,
                                    |props| {
                                        props.fit_text = Some(fit_text.clone());
                                    },
                                );
                            }
                        }
                    }
                    b"w:color" => {
                        if parsing_run_props {
                            let theme = Self::run_theme_color(e.attributes());
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.theme_color = theme.clone().map(Box::new);
                                },
                            );
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    let color_val =
                                        String::from_utf8_lossy(&attr.value).to_string();
                                    let formatted = format!("#{}", color_val);
                                    Self::apply_to_active_run_props(
                                        parsing_paragraph_run_props,
                                        &mut current_paragraph_run_props,
                                        &mut current_run_props,
                                        |props| {
                                            props.color = Some(Rc::from(formatted.as_str()));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    b"w:jc" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_alignment(
                                e.attributes(),
                                current_paragraph_props.as_mut(),
                            );
                        }
                    }
                    b"w:bidi" => {
                        if parsing_section_props {
                            // A section that reads right to left (`w:sectPr/w:bidi`), not
                            // the paragraph whose `w:pPr` happens to carry the section.
                            if let Some(section) = current_section_props.as_mut() {
                                section.bidi = Self::parse_on_off_attr(e.attributes());
                            }
                        } else if parsing_paragraph_props {
                            let value = Self::parse_on_off_attr(e.attributes());
                            current_paragraph_props
                                .get_or_insert_with(ParagraphProperties::default)
                                .bidi = Some(value);
                        }
                    }
                    b"w:ind" => {
                        if parsing_paragraph_props {
                            let props = current_paragraph_props
                                .get_or_insert_with(ParagraphProperties::default);
                            Self::apply_paragraph_indent(e.attributes(), Some(props));
                        }
                    }
                    b"w:spacing" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_spacing(
                                e.attributes(),
                                current_paragraph_props.as_mut(),
                            );
                        } else if parsing_run_props {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(val) =
                                        String::from_utf8_lossy(&attr.value).parse::<f32>()
                                    {
                                        let spacing = val / 20.0;
                                        Self::apply_to_active_run_props(
                                            parsing_paragraph_run_props,
                                            &mut current_paragraph_run_props,
                                            &mut current_run_props,
                                            |props| {
                                                props.char_spacing = Some(spacing);
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                    b"w:shd" => {
                        if parsing_run_props {
                            // Run-level character shading (`w:rPr/w:shd`).
                            let shading = Self::parse_shading(e.attributes());
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.shading = Some(shading.clone());
                                },
                            );
                        } else if parsing_paragraph_props {
                            Self::apply_paragraph_shading(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:snapToGrid" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_snap_to_grid(
                                e.attributes(),
                                current_paragraph_props.as_mut(),
                            );
                        }
                    }
                    b"w:autoSpaceDE" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_auto_spacing(
                                e.attributes(),
                                &mut current_paragraph_props,
                                false,
                            );
                        }
                    }
                    b"w:autoSpaceDN" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_auto_spacing(
                                e.attributes(),
                                &mut current_paragraph_props,
                                true,
                            );
                        }
                    }
                    b"w:kinsoku" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_line_break_option(
                                e.attributes(),
                                &mut current_paragraph_props,
                                "kinsoku",
                            );
                        }
                    }
                    b"w:wordWrap" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_line_break_option(
                                e.attributes(),
                                &mut current_paragraph_props,
                                "wordWrap",
                            );
                        }
                    }
                    b"w:overflowPunct" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_line_break_option(
                                e.attributes(),
                                &mut current_paragraph_props,
                                "overflowPunct",
                            );
                        }
                    }
                    b"w:topLinePunct" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_line_break_option(
                                e.attributes(),
                                &mut current_paragraph_props,
                                "topLinePunct",
                            );
                        }
                    }
                    b"w:adjustRightInd" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_adjust_right_indent(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:widowControl" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_widow_control(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:suppressAutoHyphens" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_suppress_auto_hyphens(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:contextualSpacing" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_contextual_spacing(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:pageBreakBefore" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_page_break_before(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:framePr" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_frame_properties(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:suppressLineNumbers" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_suppress_line_numbers(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:keepLines" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_keep_lines(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:keepNext" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_keep_next(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:textAlignment" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_text_alignment(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:mirrorIndents" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_mirror_indents(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:outlineLvl" => {
                        if parsing_paragraph_props {
                            Self::apply_paragraph_outline_level(
                                e.attributes(),
                                &mut current_paragraph_props,
                            );
                        }
                    }
                    b"w:pgSz" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let page_settings = section
                                    .page_settings
                                    .get_or_insert_with(PageSettings::default);
                                for attr in e.attributes().flatten() {
                                    let key = attr.key.as_ref();
                                    let value = String::from_utf8_lossy(&attr.value).to_string();
                                    match key {
                                        b"w:w" => {
                                            if let Ok(v) = value.parse::<f32>() {
                                                page_settings.width = Some(v / 20.0);
                                            }
                                        }
                                        b"w:h" => {
                                            if let Ok(v) = value.parse::<f32>() {
                                                page_settings.height = Some(v / 20.0);
                                            }
                                        }
                                        b"w:orient" => {
                                            if !value.is_empty() {
                                                page_settings.orientation = Some(value);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    b"w:pgMar" => {
                        if parsing_section_props {
                            if let Some(section) = current_section_props.as_mut() {
                                let page_settings = section
                                    .page_settings
                                    .get_or_insert_with(PageSettings::default);
                                for attr in e.attributes().flatten() {
                                    let key = attr.key.as_ref();
                                    let value = String::from_utf8_lossy(&attr.value).to_string();
                                    if let Ok(v) = value.parse::<f32>() {
                                        match key {
                                            b"w:left" => page_settings.margin_left = Some(v / 20.0),
                                            b"w:right" => {
                                                page_settings.margin_right = Some(v / 20.0)
                                            }
                                            b"w:top" => page_settings.margin_top = Some(v / 20.0),
                                            b"w:bottom" => {
                                                page_settings.margin_bottom = Some(v / 20.0)
                                            }
                                            b"w:header" => {
                                                page_settings.header_distance = Some(v / 20.0)
                                            }
                                            b"w:footer" => {
                                                page_settings.footer_distance = Some(v / 20.0)
                                            }
                                            b"w:gutter" => page_settings.gutter = Some(v / 20.0),
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"w:rFonts" => {
                        if parsing_run_props {
                            let attrs: Vec<(Vec<u8>, String)> = e
                                .attributes()
                                .flatten()
                                .map(|attr| {
                                    (
                                        attr.key.as_ref().to_vec(),
                                        String::from_utf8_lossy(&attr.value).to_string(),
                                    )
                                })
                                .collect();
                            Self::apply_to_active_run_props(
                                parsing_paragraph_run_props,
                                &mut current_paragraph_run_props,
                                &mut current_run_props,
                                |props| {
                                    props.authored_east_asia_font_hint =
                                        Some(attrs.iter().any(|(key, value)| {
                                            key.as_slice() == b"w:hint"
                                                && FontHint::from_xml_hint(value)
                                                    == Some(FontHint::EastAsia)
                                        }));
                                    props.authored_east_asia_font_name =
                                        Some(attrs.iter().any(|(key, _)| {
                                            matches!(
                                                key.as_slice(),
                                                b"w:eastAsia" | b"w:eastAsiaTheme"
                                            )
                                        }));
                                    let fonts = props.fonts.get_or_insert_with(RunFonts::default);
                                    for (key, value) in &attrs {
                                        match key.as_slice() {
                                            b"w:ascii" => {
                                                fonts.ascii = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:hAnsi" => {
                                                fonts.h_ansi = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:eastAsia" => {
                                                fonts.east_asia = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:cs" => {
                                                fonts.complex = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:asciiTheme" => {
                                                fonts.ascii_theme = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:hAnsiTheme" => {
                                                fonts.h_ansi_theme = Some(Rc::from(value.as_str()));
                                            }
                                            b"w:eastAsiaTheme" => {
                                                fonts.east_asia_theme =
                                                    Some(Rc::from(value.as_str()));
                                            }
                                            b"w:cstheme" => {
                                                fonts.complex_theme =
                                                    Some(Rc::from(value.as_str()));
                                            }
                                            b"w:hint" => {
                                                fonts.hint = FontHint::from_xml_hint(value);
                                            }
                                            _ => {}
                                        }
                                    }
                                    fonts.drop_names_shadowed_by_themes();
                                    if props.font_family.is_none() {
                                        if let Some(family) = fonts.preferred_latin() {
                                            props.font_family = Some(Rc::from(family));
                                        }
                                    }
                                },
                            );
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => match e.name().as_ref() {
                    b"w:t" | b"w:delText" => {
                        in_text = false;
                        let text_to_push = Self::line_feeds_as_spaces(if current_text_preserve {
                            &current_text
                        } else {
                            Self::trim_xml_whitespace(&current_text)
                        });
                        current_text.clear();
                        current_text_preserve = false;
                        if let Some(ref mut run) = current_run {
                            if Self::keeps_field_result_in_run_text(
                                current_run_field.as_ref(),
                                current_run_is_instruction_operand,
                            ) {
                                // HYPERLINK, MERGEFIELD, FILLIN, REF, and
                                // PAGEREF results
                                // remain ordinary user-visible, editable text.
                                // Keep them in `run.text` while the field object
                                // supplies semantic metadata. Computed dynamic
                                // fields remain suppressed into `cached_text`
                                // for renderer-time resolution. Keeping these
                                // editable top-level results out of the cache
                                // also avoids appending the same display string
                                // twice. A nested result is different: Word
                                // treats it as an operand of the enclosing field,
                                // so it must flow through `current_run_field_text`
                                // below instead of leaking into the paragraph.
                                if !text_to_push.is_empty() {
                                    paragraph_text_len += text_to_push.len();
                                    run.text.push_str(&text_to_push);
                                }
                            } else {
                                // Field result text is suppressed from the run
                                // body but preserved as the field's cached value.
                                current_run_field_text.push_str(&text_to_push);
                            }
                        }
                    }
                    b"w:instrText" => {
                        in_instr_text = false;
                        if let Some(field) = active_fields
                            .iter_mut()
                            .rev()
                            .find(|field| field.instruction.is_some())
                        {
                            if let Some(buffer) = field.instruction.as_mut() {
                                buffer.push_str(&current_instr_text);
                            }
                            if let Some(buffer) = field.saved_instruction.as_mut() {
                                buffer.push_str(&current_instr_text);
                            }
                        }
                        current_instr_text.clear();
                    }
                    b"w:fldSimple" => {
                        Self::finish_simple_field(
                            &mut active_fields,
                            current_paragraph.as_mut(),
                            &resolved_bookmark_text,
                            &mut deferred_nested_fields,
                        );
                    }
                    b"w:pPr" => {
                        parsing_paragraph_props = false;
                    }
                    b"w:rPr" => {
                        parsing_run_props = false;
                        parsing_paragraph_run_props = false;
                    }
                    b"w:r" => {
                        if let (Some(mut run), Some(ref mut paragraph)) =
                            (current_run.take(), &mut current_paragraph)
                        {
                            run.properties = current_run_props.take();
                            run.image = current_run_image.take();
                            run.text_box = current_run_text_box.take();
                            if let Some(kind) = current_run_field.take() {
                                let cached_text = if current_run_field_text.is_empty() {
                                    None
                                } else {
                                    Some(std::mem::take(&mut current_run_field_text))
                                };
                                if matches!(kind, RunFieldKind::Seq { .. })
                                    && current_run_sequence_result.is_some()
                                {
                                    // One source SEQ may have several formatted result runs.
                                    // Keep one semantic field and combine its cached fragments.
                                    if let Some(field) = current_run_sequence_result
                                        .and_then(|index| paragraph.runs.get_mut(index))
                                        .and_then(|run| run.field.as_mut())
                                    {
                                        if let Some(text) = cached_text {
                                            field
                                                .cached_text
                                                .get_or_insert_with(String::new)
                                                .push_str(&text);
                                        }
                                    }
                                } else if current_run_is_instruction_operand {
                                    // The nested result participates in the
                                    // parent's instruction and must not become
                                    // a separately rendered field run.
                                    if let Some(text) = cached_text.as_deref() {
                                        if let Some(field) =
                                            active_fields.iter_mut().rev().find(|field| {
                                                field.is_instruction_operand
                                                    && field.kind.as_ref() == Some(&kind)
                                            })
                                        {
                                            field.operand_result.push_str(text);
                                        }
                                    }
                                } else {
                                    // The closing `w:fldChar` run starts while the
                                    // complex field is still active, so it inherits
                                    // the same semantic kind as the preceding result
                                    // run. Do not materialize that empty boundary as
                                    // a second PAGE/REF/etc. field.
                                    let duplicates_previous_result = current_run_has_field_end
                                        && run.text.is_empty()
                                        && cached_text.is_none()
                                        && paragraph.runs.last().is_some_and(|previous| {
                                            previous
                                                .field
                                                .as_ref()
                                                .is_some_and(|field| field.kind == kind)
                                        });
                                    if !duplicates_previous_result {
                                        // A HYPERLINK field carries interaction
                                        // metadata in its instruction rather than a
                                        // surrounding `w:hyperlink` element. Attach
                                        // it only to the materialized result run;
                                        // the closing fldChar boundary inherits the
                                        // active kind but must stay metadata-free.
                                        if run.hyperlink.is_none() {
                                            run.hyperlink = Self::hyperlink_from_field_kind(&kind);
                                        }
                                        Self::apply_symbol_field_formatting(
                                            &mut run.properties,
                                            &kind,
                                        );
                                        if matches!(kind, RunFieldKind::Seq { .. }) {
                                            if let Some(field) = active_fields
                                                .iter_mut()
                                                .rev()
                                                .find(|field| field.kind.as_ref() == Some(&kind))
                                            {
                                                field.sequence_result_run =
                                                    Some(paragraph.runs.len());
                                            }
                                        }
                                        let word_mac_body_result = active_fields
                                            .iter()
                                            .rev()
                                            .find(|field| field.kind.as_ref() == Some(&kind))
                                            .and_then(|field| field.word_mac_body_result.clone());
                                        run.field = Some(RunField {
                                            word_mac_body_result,
                                            kind,
                                            cached_text,
                                            source_field_id: None,
                                            resolved_value: None,
                                        });
                                    }
                                }
                            }
                            if run.text.contains('\n')
                                && !current_run_has_carriage_return
                                && !current_run_has_attributed_soft_break
                                && run.image.is_none()
                                && run.text_box.is_none()
                                && run.field.is_none()
                            {
                                for split in Self::split_run_by_soft_breaks(run) {
                                    paragraph.runs.push(split);
                                }
                            } else {
                                paragraph.runs.push(run);
                            }
                            current_run_has_carriage_return = false;
                            current_run_is_instruction_operand = false;
                            current_run_has_attributed_soft_break = false;
                        }
                    }
                    b"w:ruby" => {
                        // Attach the collected guide text to every run the
                        // `w:rubyBase` contents pushed; the first one starts
                        // the base and draws the guide. Ruby with no base runs
                        // (or empty guide text) is dropped — there is nothing
                        // to annotate.
                        if let Some(pending) = pending_ruby.take() {
                            if !pending.text.is_empty() {
                                let base_runs = current_paragraph
                                    .as_mut()
                                    .and_then(|p| p.runs.get_mut(pending.base_run_index..))
                                    .unwrap_or_default();
                                for (index, run) in base_runs.iter_mut().enumerate() {
                                    let props =
                                        run.properties.get_or_insert_with(RunProperties::default);
                                    props.ruby = Some(Box::new(RubyAnnotation {
                                        text: pending.text.clone(),
                                        size_pt: pending.size_pt,
                                        align: pending.align.clone(),
                                        raise_half_points: pending.raise_half_points,
                                        fonts: pending.fonts.clone(),
                                        continuation: index > 0,
                                    }));
                                }
                            }
                        }
                    }
                    b"w:ins" | b"w:del" | b"w:moveFrom" | b"w:moveTo" => {
                        revision_stack.pop();
                    }
                    b"w:sectPr" => {
                        parsing_section_props = false;
                        if let Some(section_props) = current_section_props.take() {
                            if parsing_paragraph_props {
                                if let Some(ref mut props) = current_paragraph_props {
                                    props.section = Some(section_props);
                                }
                            } else {
                                default_section_props = Some(section_props);
                            }
                        }
                    }
                    b"w:p" => {
                        if let Some(paragraph) = current_paragraph.take() {
                            // Any comment range still open at the end of the
                            // paragraph spans into the next one. Close it here at
                            // the paragraph's end offset so the highlight covers
                            // the rest of this paragraph; the range stays open in
                            // `open_comment_ranges` and resumes from offset 0 of
                            // the following paragraph (see the `w:p` start handler).
                            let mut still_open: Vec<(&String, &usize)> =
                                open_comment_ranges.iter().collect();
                            still_open.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(b.0)));
                            for (id, start) in still_open {
                                paragraph_comment_ranges.push(CommentRange {
                                    id: id.clone(),
                                    start: *start,
                                    end: paragraph_text_len,
                                });
                            }
                            let mut props = current_paragraph_props.take();
                            if let Some(mark_run_properties) = current_paragraph_run_props.take() {
                                props
                                    .get_or_insert_with(ParagraphProperties::default)
                                    .mark_run_properties = Some(mark_run_properties);
                            }
                            if let Some(para_id) = current_paragraph_para_id.take() {
                                props
                                    .get_or_insert_with(ParagraphProperties::default)
                                    .para_id = Some(para_id.into());
                            }
                            if !(current_paragraph_is_math_wrapper
                                && paragraph.runs.is_empty()
                                && paragraph_comment_ranges.is_empty()
                                && paragraph_comment_markers.is_empty())
                            {
                                Self::push_paragraph(
                                    &mut document,
                                    container_stack.as_mut_slice(),
                                    paragraph,
                                    props,
                                    std::mem::take(&mut paragraph_comment_ranges),
                                    std::mem::take(&mut paragraph_comment_markers),
                                );
                            } else {
                                paragraph_comment_ranges.clear();
                                paragraph_comment_markers.clear();
                            }
                            current_paragraph_is_math_wrapper = false;
                            // Most paragraphs do not close a bookmark. Avoid
                            // rebuilding every previously resolved range on
                            // each paragraph in large documents.
                            if document.bookmarks.len() != resolved_bookmark_count {
                                resolved_bookmark_text = document.bookmark_text_map();
                                resolved_bookmark_count = document.bookmarks.len();
                            }
                        }
                    }
                    b"w:tc" => {
                        if let Some(ContainerContext::TableCell(mut cell)) = container_stack.pop() {
                            if cell.blocks.is_empty() {
                                cell.blocks.push(BodyElement::from_paragraph(Paragraph {
                                    runs: Vec::new(),
                                    properties: None,
                                    node_id: None,
                                    comment_ranges: Vec::new(),
                                    comment_markers: Vec::new(),
                                }));
                            }
                            if let Some(ContainerContext::TableRow(row)) =
                                container_stack.last_mut()
                            {
                                row.cells.push(cell);
                            }
                        }
                    }
                    b"w:tr" => {
                        if let Some(ContainerContext::TableRow(mut row)) = container_stack.pop() {
                            // Word keeps `w:hMerge` only for compatibility with
                            // older producers. Normalize it to gridSpan before
                            // any renderer/editor observes the row.
                            Self::normalize_legacy_horizontal_merges(&mut row);
                            if let Some(ContainerContext::Table(table)) = container_stack.last_mut()
                            {
                                table.rows.push(row);
                            }
                        }
                    }
                    b"w:tbl" => {
                        if let Some(ContainerContext::Table(table)) = container_stack.pop() {
                            Self::push_block(
                                &mut document,
                                container_stack.as_mut_slice(),
                                BodyElement::from_table(*table),
                            );
                        }
                    }
                    b"w:hyperlink" => {
                        hyperlink_stack.pop();
                    }
                    _ => {}
                },
                Ok(Event::Text(e)) => {
                    if let Some(text_owned) = text_event_to_string(&e) {
                        if in_text {
                            current_text.push_str(&text_owned);
                        }
                        if in_instr_text {
                            // Accumulate the raw instruction text WITHOUT trimming.
                            // Word frequently splits one field code across several
                            // `<w:instrText>` runs (e.g. `PAGEREF ` + `_Toc1 \h`,
                            // or `TOC \o "1-3" ` + `\h \z \u `). Trimming each run
                            // before concatenation collapses the inter-run space
                            // (`PAGEREF` + `_Toc1` -> `PAGEREF_Toc1`), corrupting the
                            // field name / argument boundaries. `xml:space="preserve"`
                            // keeps the surrounding spaces; the field-instruction
                            // parser normalizes whitespace via `split_whitespace`.
                            current_instr_text.push_str(&text_owned);
                        }
                    }
                }
                Ok(Event::GeneralRef(reference)) => {
                    // quick-xml emits named/numeric entities as a standalone
                    // event. This is especially important in field codes:
                    // Word must XML-escape `<` in `IF 2<10`, and producers may
                    // also escape `>` in `>=`. Dropping the event silently
                    // changes the operator (`>=` -> `=`) or removes it.
                    if let Some(text_owned) = reference_event_to_string(&reference) {
                        if in_text {
                            current_text.push_str(&text_owned);
                        }
                        if in_instr_text {
                            current_instr_text.push_str(&text_owned);
                        }
                    }
                }
                Ok(Event::CData(e)) => {
                    let text_owned = String::from_utf8_lossy(e.as_ref());
                    if in_text {
                        current_text.push_str(&text_owned);
                    }
                    if in_instr_text {
                        current_instr_text.push_str(&text_owned);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parsing error: {}", e).into()),
                _ => {}
            }
        }

        document.default_section = default_section_props;

        if let Some(body) = document.body.as_ref() {
            document.sections =
                Self::build_section_ranges(&body.elements, document.default_section.clone());
        }

        if document.page_settings.is_none() {
            document.sync_page_settings_with_first_section();
        }

        if document.header_reference.is_none() {
            document.header_reference = document
                .default_section
                .as_ref()
                .and_then(|s| s.header_reference.clone());
        }
        if document.footer_reference.is_none() {
            document.footer_reference = document
                .default_section
                .as_ref()
                .and_then(|s| s.footer_reference.clone());
        }

        // A nested field may have referenced a bookmark defined later in the
        // document. Replace its internal placeholder now that the complete
        // bookmark table is available, before resolving the outer IF/COMPARE.
        if !deferred_nested_fields.is_empty() {
            let bookmark_text = document.bookmark_text_map();
            if let Some(body) = document.body.as_mut() {
                Self::resolve_deferred_nested_fields_in_elements(
                    &mut body.elements,
                    &deferred_nested_fields,
                    &bookmark_text,
                );
            }
        }

        // Resolve `REF`/`SEQ` field values now that the body and bookmark table
        // are fully populated (REF reads bookmark text; SEQ numbers occurrences
        // in document order).
        document.resolve_structural_fields();

        Ok(document)
    }

    fn record_alt_chunk(
        document: &mut Document,
        attributes: quick_xml::events::attributes::Attributes<'_>,
    ) {
        let relationship_id = attributes.flatten().find_map(|attribute| {
            matches!(attribute.key.as_ref(), b"r:id" | b"id")
                .then(|| String::from_utf8_lossy(&attribute.value).into_owned())
        });
        let Some(relationship_id) = relationship_id.filter(|id| !id.is_empty()) else {
            return;
        };
        let block_index = document.body.as_ref().map_or(0, |body| body.elements.len());
        document.alt_chunks.push(AltChunkReference {
            relationship_id,
            block_index,
        });
    }

    pub fn parse_notes_xml(
        &self,
        xml_content: &str,
        note_tag: &[u8],
    ) -> Result<Vec<Note>, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml_content);
        // Entity-adjacent text arrives as separate events. Trimming here would
        // turn `A &amp; B` into `A&B`; structural indentation is filtered while
        // copying the note subtree below.
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        let mut namespace_attrs: Option<String> = None;
        let mut notes = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let is_note = e.name().as_ref() == note_tag;
                    if namespace_attrs.is_none() && !is_note {
                        namespace_attrs = Some(Self::collect_namespace_attributes(e)?);
                    }
                    if is_note {
                        let (id, note_type) = Self::extract_note_metadata(e)?;
                        let inner_xml = Self::read_note_inner_xml(&mut reader)?;
                        let body =
                            self.parse_note_body(inner_xml.as_deref(), namespace_attrs.as_deref())?;
                        if !id.is_empty() {
                            notes.push(Note {
                                id,
                                note_type,
                                body,
                            });
                        }
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    if e.name().as_ref() == note_tag {
                        let (id, note_type) = Self::extract_note_metadata(e)?;
                        if !id.is_empty() {
                            notes.push(Note {
                                id,
                                note_type,
                                body: Vec::new(),
                            });
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
            buf.clear();
        }

        Ok(notes)
    }

    fn collect_namespace_attributes(
        element: &BytesStart<'_>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut attrs = String::new();
        for attr in element.attributes() {
            let attr = attr?;
            attrs.push(' ');
            attrs.push_str(&String::from_utf8(attr.key.as_ref().to_vec())?);
            attrs.push('=');
            attrs.push('"');
            let value = attr.unescape_value()?;
            attrs.push_str(&value);
            attrs.push('"');
        }
        Ok(attrs)
    }

    /// Extracts `w:id` and `w:name` from a `<w:bookmarkStart .../>` attribute
    /// iterator. Returns `(id, name)` — id is required (we drop malformed
    /// tags), name is optional (missing name means the bookmark is treated as
    /// anonymous, matching Word's tolerance).
    fn extract_bookmark_start_attributes(
        attributes: Attributes<'_>,
    ) -> (Option<String>, Option<String>) {
        let mut id = None;
        let mut name = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:id" => id = Some(String::from_utf8_lossy(&attr.value).to_string()),
                b"w:name" => name = Some(String::from_utf8_lossy(&attr.value).to_string()),
                _ => {}
            }
        }
        (id, name)
    }

    fn extract_note_metadata(
        element: &BytesStart<'_>,
    ) -> Result<(String, Option<String>), Box<dyn std::error::Error>> {
        let mut id = String::new();
        let mut note_type = None;
        for attr in element.attributes() {
            let attr = attr?;
            match attr.key.as_ref() {
                b"w:id" => {
                    id = attr.unescape_value()?.into_owned();
                }
                b"w:type" => {
                    note_type = Some(attr.unescape_value()?.into_owned());
                }
                _ => {}
            }
        }
        Ok((id, note_type))
    }

    fn read_note_inner_xml(
        reader: &mut dyn XmlEventSource,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut depth = 0usize;
        let mut text_depth = 0usize;
        let mut buf = Vec::new();
        let mut writer = Writer::new(Cursor::new(Vec::new()));

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    depth += 1;
                    if matches!(e.name().as_ref(), b"w:t" | b"w:delText") {
                        text_depth += 1;
                    }
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                Ok(Event::Empty(ref e)) => {
                    writer.write_event(Event::Empty(e.to_owned()))?;
                }
                Ok(Event::Text(e)) => {
                    if text_depth > 0 {
                        // The document parser applies xml:space after joining
                        // all text/entity events. Preserve w:t (and deleted
                        // text, w:delText) verbatim here; trimming early
                        // loses note separators and spaces.
                        writer.write_event(Event::Text(e.to_owned()))?;
                    } else {
                        let raw = std::str::from_utf8(e.as_ref())?;
                        let trimmed = Self::trim_xml_whitespace(raw);
                        if !trimmed.is_empty() {
                            writer.write_event(Event::Text(BytesText::from_escaped(trimmed)))?;
                        }
                    }
                }
                Ok(Event::GeneralRef(e)) => {
                    // quick-xml reports XML entities as standalone events. A
                    // note body is parsed a second time after being wrapped in
                    // a synthetic document, so dropping these here silently
                    // removes visible `&`, `<`, and `>` characters.
                    writer.write_event(Event::GeneralRef(e.to_owned()))?;
                }
                Ok(Event::CData(e)) => {
                    writer.write_event(Event::CData(e.to_owned()))?;
                }
                Ok(Event::Comment(e)) => {
                    writer.write_event(Event::Comment(e.to_owned()))?;
                }
                Ok(Event::End(ref e)) => {
                    if depth == 0 {
                        break;
                    } else {
                        writer.write_event(Event::End(e.to_owned()))?;
                        if matches!(e.name().as_ref(), b"w:t" | b"w:delText") {
                            text_depth = text_depth.saturating_sub(1);
                        }
                        depth = depth.saturating_sub(1);
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
        }

        let result = writer.into_inner().into_inner();
        if result.is_empty() {
            Ok(None)
        } else {
            Ok(Some(String::from_utf8(result)?))
        }
    }

    fn capture_element_xml(
        reader: &mut dyn XmlEventSource,
        start: BytesStart<'_>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut writer = Writer::new(Vec::new());
        writer.write_event(Event::Start(start.to_owned()))?;

        let mut depth = 1usize;
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    depth = depth.saturating_add(1);
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                Ok(Event::Empty(ref e)) => {
                    writer.write_event(Event::Empty(e.to_owned()))?;
                }
                Ok(Event::Text(e)) => {
                    writer.write_event(Event::Text(e.to_owned()))?;
                }
                Ok(Event::CData(e)) => {
                    writer.write_event(Event::CData(e.to_owned()))?;
                }
                Ok(Event::Comment(e)) => {
                    writer.write_event(Event::Comment(e.to_owned()))?;
                }
                Ok(Event::End(ref e)) => {
                    writer.write_event(Event::End(e.to_owned()))?;
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
        }

        let bytes = writer.into_inner();
        Ok(String::from_utf8(bytes)?)
    }

    fn parse_numbering_from_ppr_change(xml: &str) -> Option<crate::document::Numbering> {
        // Word stores paragraph property revisions (including numbering) inside
        // `<w:pPrChange>` containers. These revisions only apply when tracked
        // changes are shown. We look for a nested `<w:numPr>` element and reuse
        // its values as the effective numbering for the paragraph.
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        let mut numbering: Option<crate::document::Numbering> = None;
        let mut in_num_pr = false;

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:numPr" => {
                        in_num_pr = true;
                        if numbering.is_none() {
                            numbering = Some(crate::document::Numbering {
                                id: None,
                                level: None,
                            });
                        }
                    }
                    b"w:numId" if in_num_pr => {
                        if let Some(num) = numbering.as_mut() {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    num.id = Some(String::from_utf8_lossy(&attr.value).to_string());
                                }
                            }
                        }
                    }
                    b"w:ilvl" if in_num_pr => {
                        if let Some(num) = numbering.as_mut() {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(level) =
                                        String::from_utf8_lossy(&attr.value).parse::<u32>()
                                    {
                                        num.level = Some(level);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:numId" if in_num_pr => {
                        if let Some(num) = numbering.as_mut() {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    num.id = Some(String::from_utf8_lossy(&attr.value).to_string());
                                }
                            }
                        }
                    }
                    b"w:ilvl" if in_num_pr => {
                        if let Some(num) = numbering.as_mut() {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"w:val" {
                                    if let Ok(level) =
                                        String::from_utf8_lossy(&attr.value).parse::<u32>()
                                    {
                                        num.level = Some(level);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:numPr" {
                        in_num_pr = false;
                    } else if e.name().as_ref() == b"w:pPrChange" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }

        numbering.and_then(|mut num| {
            if num.id.is_some() {
                if num.level.is_none() {
                    num.level = Some(0);
                }
                Some(num)
            } else {
                None
            }
        })
    }

    fn parse_note_body(
        &self,
        inner_xml: Option<&str>,
        namespace_attrs: Option<&str>,
    ) -> Result<Vec<BodyElement>, Box<dyn std::error::Error>> {
        let inner = match inner_xml {
            Some(xml) => xml,
            None => return Ok(Vec::new()),
        };

        if inner.trim().is_empty() {
            return Ok(Vec::new());
        }

        const DEFAULT_NOTE_NAMESPACES: &str = r#" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let namespaces = namespace_attrs.unwrap_or(DEFAULT_NOTE_NAMESPACES);
        let wrapped = format!(
            "<w:document{namespaces}><w:body>{inner}</w:body></w:document>",
            namespaces = namespaces,
            inner = inner
        );

        let mut parsed = self.parse_document_xml(&wrapped)?;
        let elements = if let Some(body) = parsed.body.take() {
            body.elements
        } else {
            Vec::new()
        };
        Ok(elements)
    }

    fn parse_columns_attributes(attributes: Attributes<'_>, columns: &mut SectionColumns) {
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            let value = attr
                .unescape_value()
                .map(|cow| cow.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
            match key {
                b"w:num" => {
                    if let Ok(v) = value.parse::<u32>() {
                        columns.count = Some(v);
                    }
                }
                b"w:space" => {
                    if let Ok(v) = value.parse::<f32>() {
                        columns.space = Some(v / 20.0);
                    }
                }
                b"w:equalWidth" => {
                    columns.equal_width = Some(parse_on_off(Some(value.clone()), true));
                }
                b"w:sep" => {
                    columns.separator = Some(parse_on_off(Some(value), false));
                }
                _ => {}
            }
        }
    }

    /// Parses `w:pgNumType` attributes (`w:fmt` numeral format, `w:start` first
    /// displayed page number) into [`crate::document::PageNumberType`].
    /// ECMA-376 Part 1 §17.6.12.
    fn parse_page_number_type(attributes: Attributes<'_>) -> crate::document::PageNumberType {
        let mut pg_num = crate::document::PageNumberType::default();
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            let value = attr
                .unescape_value()
                .map(|cow| cow.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
            match key {
                b"w:fmt" => {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        pg_num.format = Some(trimmed.to_lowercase());
                    }
                }
                b"w:start" => {
                    if let Ok(v) = value.trim().parse::<u32>() {
                        pg_num.start = Some(v);
                    }
                }
                b"w:chapStyle" => {
                    if let Ok(v) = value.trim().parse::<u8>() {
                        pg_num.chapter_style = Some(v);
                    }
                }
                b"w:chapSep" => {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        pg_num.chapter_separator = Some(trimmed.to_string());
                    }
                }
                _ => {}
            }
        }
        pg_num
    }

    /// Parses the `w:val` of a section-level `w:vAlign` element into a
    /// normalized (trimmed, lowercased) vertical-justification token
    /// (`top`/`center`/`both`/`bottom`). Returns `None` when the attribute is
    /// missing, empty, or `top` (the default), so absent and explicit-top
    /// collapse to the same "no offset" state. ECMA-376 Part 1 §17.6.22.
    fn parse_section_vertical_alignment(attributes: Attributes<'_>) -> Option<String> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let value = attr
                    .unescape_value()
                    .map(|cow| cow.to_string())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
                let token = value.trim().to_lowercase();
                if token.is_empty() || token == "top" {
                    return None;
                }
                return Some(token);
            }
        }
        None
    }

    /// Parses `w:lnNumType` attributes (`w:countBy` interval, `w:start` first
    /// number, `w:restart` reset behaviour, `w:distance` margin gap in twips)
    /// into [`crate::document::LineNumbering`]. ECMA-376 Part 1 §17.6.8.
    fn parse_line_numbering(attributes: Attributes<'_>) -> crate::document::LineNumbering {
        let mut ln_num = crate::document::LineNumbering::default();
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            let value = attr
                .unescape_value()
                .map(|cow| cow.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
            match key {
                b"w:countBy" => {
                    if let Ok(v) = value.trim().parse::<u32>() {
                        ln_num.count_by = Some(v);
                    }
                }
                b"w:start" => {
                    if let Ok(v) = value.trim().parse::<u32>() {
                        ln_num.start = Some(v);
                    }
                }
                b"w:restart" => {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        ln_num.restart = Some(trimmed.to_lowercase());
                    }
                }
                b"w:distance" => {
                    if let Ok(v) = value.trim().parse::<u32>() {
                        ln_num.distance = Some(v);
                    }
                }
                _ => {}
            }
        }
        ln_num
    }

    /// Parses a `<w:footnotePr>` / `<w:endnotePr>` subtree, collecting the
    /// `w:numFmt` / `w:numStart` / `w:numRestart` children (all self-closing in
    /// practice). `end_tag` is the container tag so we stop at the right `End`.
    fn parse_note_numbering(
        reader: &mut dyn XmlEventSource,
        end_tag: &[u8],
    ) -> Result<
        (
            crate::document::NoteNumbering,
            Option<crate::document::EndnotePosition>,
            Option<bool>,
        ),
        Box<dyn std::error::Error>,
    > {
        use crate::document::{EndnotePosition, NoteNumbering};
        let mut note = NoteNumbering::default();
        let mut position = None;
        let mut beneath = None;
        let mut buf = Vec::new();

        let assign = |note: &mut NoteNumbering,
                      position: &mut Option<EndnotePosition>,
                      beneath: &mut Option<bool>,
                      name: &[u8],
                      attrs: Attributes<'_>| {
            let val = attrs.flatten().find_map(|attr| {
                if attr.key.as_ref() == b"w:val" {
                    Some(String::from_utf8_lossy(&attr.value).trim().to_string())
                } else {
                    None
                }
            });
            let Some(val) = val else { return };
            match name {
                b"w:pos" => {
                    *beneath = match val.as_str() {
                        "beneathText" => Some(true),
                        "pageBottom" => Some(false),
                        _ => None,
                    };
                    *position = match val.as_str() {
                        "sectEnd" => Some(EndnotePosition::SectionEnd),
                        "docEnd" => Some(EndnotePosition::DocumentEnd),
                        _ => None,
                    }
                }
                b"w:numFmt" => {
                    if !val.is_empty() {
                        note.number_format = Some(val.to_lowercase());
                    }
                }
                b"w:numStart" => {
                    if let Ok(v) = val.parse::<u32>() {
                        note.number_start = Some(v);
                    }
                }
                b"w:numRestart" => {
                    if !val.is_empty() {
                        note.number_restart = Some(val.to_lowercase());
                    }
                }
                _ => {}
            }
        };

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == end_tag => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(
                        &mut note,
                        &mut position,
                        &mut beneath,
                        name.as_ref(),
                        e.attributes(),
                    );
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&mut note, &mut position, &mut beneath, &name, e.attributes());
                    Self::consume_element(reader, &name)?;
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok((note, position, beneath))
    }

    /// Consumes a `w:rubyPr` subtree (ECMA-376 Part 1 §17.3.3.26) and returns
    /// `(hps_pt, align, raise_half_points)` — the guide text size from `w:hps`
    /// (half-points → pt), the lowercased `w:rubyAlign` token and the
    /// `w:hpsRaise` half-points.
    #[allow(clippy::type_complexity)]
    fn parse_ruby_properties(
        reader: &mut dyn XmlEventSource,
    ) -> Result<(Option<f32>, Option<String>, Option<u32>), Box<dyn std::error::Error>> {
        let mut hps_pt = None;
        let mut align = None;
        let mut raise_half_points = None;
        let mut buf = Vec::new();

        let mut assign = |name: &[u8], attrs: Attributes<'_>| {
            let val = attrs.flatten().find_map(|attr| {
                if attr.key.as_ref() == b"w:val" {
                    Some(String::from_utf8_lossy(&attr.value).trim().to_string())
                } else {
                    None
                }
            });
            let Some(val) = val else { return };
            match name {
                b"w:hps" => {
                    if let Ok(half_points) = val.parse::<f32>() {
                        hps_pt = Some(half_points / 2.0);
                    }
                }
                b"w:rubyAlign" => {
                    if !val.is_empty() {
                        align = Some(val.to_lowercase());
                    }
                }
                b"w:hpsRaise" => {
                    raise_half_points = val.parse::<u32>().ok();
                }
                _ => {}
            }
        };

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:rubyPr" => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(name.as_ref(), e.attributes());
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&name, e.attributes());
                    Self::consume_element(reader, &name)?;
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok((hps_pt, align, raise_half_points))
    }

    /// Consumes a `w:rt` subtree (ruby guide text, ECMA-376 Part 1 §17.3.3.24)
    /// and returns `(text, size_pt, fonts)` — the concatenated `w:t` contents,
    /// the guide run's `w:sz` (half-points → pt) and its `w:rFonts`. The guide
    /// runs are intentionally not turned into body runs: Word draws them above
    /// the base text, so letting them flow inline would corrupt the body text
    /// and offsets.
    #[allow(clippy::type_complexity)]
    fn parse_ruby_text(
        reader: &mut dyn XmlEventSource,
    ) -> Result<(String, Option<f32>, Option<RunFonts>), Box<dyn std::error::Error>> {
        let mut text = String::new();
        let mut size_pt = None;
        let mut fonts: Option<RunFonts> = None;
        let read_fonts = |attrs: Attributes<'_>, fonts: &mut Option<RunFonts>| {
            for attr in attrs.flatten() {
                let value = Some(Rc::from(String::from_utf8_lossy(&attr.value).as_ref()));
                let target = fonts.get_or_insert_with(RunFonts::default);
                match attr.key.as_ref() {
                    b"w:ascii" => target.ascii = value,
                    b"w:hAnsi" => target.h_ansi = value,
                    b"w:eastAsia" => target.east_asia = value,
                    b"w:cs" => target.complex = value,
                    b"w:asciiTheme" => target.ascii_theme = value,
                    b"w:hAnsiTheme" => target.h_ansi_theme = value,
                    b"w:eastAsiaTheme" => target.east_asia_theme = value,
                    b"w:cstheme" => target.complex_theme = value,
                    _ => {}
                }
            }
        };
        let mut in_text = false;
        let mut buf = Vec::new();

        let read_size = |attrs: Attributes<'_>| {
            attrs.flatten().find_map(|attr| {
                if attr.key.as_ref() == b"w:val" {
                    String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .map(|half_points| half_points / 2.0)
                } else {
                    None
                }
            })
        };

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) => match e.name().as_ref() {
                    b"w:rt" => break,
                    b"w:t" => in_text = false,
                    _ => {}
                },
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:t" => in_text = true,
                    b"w:sz" => {
                        if let Some(size) = read_size(e.attributes()) {
                            size_pt = Some(size);
                        }
                    }
                    b"w:rFonts" => read_fonts(e.attributes(), &mut fonts),
                    _ => {}
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:sz" => {
                        if let Some(size) = read_size(e.attributes()) {
                            size_pt = Some(size);
                        }
                    }
                    b"w:rFonts" => read_fonts(e.attributes(), &mut fonts),
                    _ => {}
                },
                Ok(Event::Text(ref t)) => {
                    if in_text {
                        if let Some(text_owned) = text_event_to_string(t) {
                            text.push_str(&text_owned);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok((text, size_pt, fonts))
    }

    fn parse_doc_grid_attributes(attributes: Attributes<'_>, doc_grid: &mut DocGrid) {
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            let value = attr
                .unescape_value()
                .map(|cow| cow.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
            match key {
                b"w:type" | b"w:val" => {
                    if !value.is_empty() {
                        doc_grid.grid_type = Some(value);
                    }
                }
                b"w:linePitch" => {
                    if let Ok(v) = value.parse::<f32>() {
                        doc_grid.line_pitch = Some(v / 20.0);
                    }
                }
                b"w:charSpace" => {
                    if let Ok(v) = value.parse::<f32>() {
                        doc_grid.char_space = Some(v / 4096.0);
                    }
                }
                _ => {}
            }
        }
    }

    /// The revision a run inside the open `w:ins`/`w:del`/move elements
    /// carries: the innermost one, and for a deletion the insertion around
    /// it (`<w:ins><w:del>`), which Word writes when someone deletes text
    /// that is still an insertion.
    fn current_revision(stack: &[RevisionMark]) -> Option<RevisionMark> {
        let (innermost, outer) = stack.split_last()?;
        let mut mark = innermost.clone();
        if mark.is_deletion() {
            mark.insertion = outer
                .iter()
                .rev()
                .find(|outer| !outer.is_deletion())
                .map(|outer| Box::new(outer.clone()));
        }
        Some(mark)
    }

    fn revision_kind(element: &BytesStart) -> RevisionKind {
        match element.name().as_ref() {
            b"w:ins" => RevisionKind::Inserted,
            b"w:moveFrom" => RevisionKind::MovedFrom,
            b"w:moveTo" => RevisionKind::MovedTo,
            _ => RevisionKind::Deleted,
        }
    }

    fn build_revision_mark(element: &BytesStart, kind: RevisionKind) -> RevisionMark {
        let mut author: Option<String> = None;
        let mut date: Option<String> = None;
        let mut id: Option<String> = None;
        let mut date_utc: Option<String> = None;
        for attr in element.attributes().flatten() {
            match attr.key.as_ref() {
                b"w:author" => {
                    author = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"w:date" => {
                    date = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"w:id" => {
                    id = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"w16du:dateUtc" => {
                    date_utc = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                _ => {}
            }
        }
        RevisionMark {
            kind,
            id,
            author,
            date,
            date_utc,
            insertion: None,
        }
    }
    fn build_section_ranges(
        elements: &[BodyElement],
        default_section: Option<SectionProperties>,
    ) -> Vec<SectionRange> {
        if elements.is_empty() {
            return Vec::new();
        }

        struct TempSection {
            start: usize,
            end: usize,
            props: Option<SectionProperties>,
        }

        let mut segments: Vec<TempSection> = Vec::new();
        let mut start_idx = 0usize;

        for (idx, element) in elements.iter().enumerate() {
            if let BodyElement::Paragraph(paragraph) = element {
                let paragraph = paragraph.as_ref();
                if let Some(section_props) = paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.section.clone())
                {
                    segments.push(TempSection {
                        start: start_idx,
                        end: idx + 1,
                        props: Some(section_props),
                    });
                    start_idx = idx + 1;
                }
            }
        }

        if start_idx < elements.len() {
            segments.push(TempSection {
                start: start_idx,
                end: elements.len(),
                props: None,
            });
        }

        if segments.is_empty() {
            segments.push(TempSection {
                start: 0,
                end: elements.len(),
                props: None,
            });
        }

        let mut section_ranges: Vec<SectionRange> = vec![SectionRange::default(); segments.len()];
        let mut next_props = default_section;

        for index in (0..segments.len()).rev() {
            let segment = &segments[index];
            let mut effective_props = match &segment.props {
                Some(props) => {
                    next_props = Some(props.clone());
                    Some(props.clone())
                }
                None => next_props.clone(),
            };

            if effective_props.is_none() {
                effective_props = next_props.clone();
            }

            section_ranges[index] = SectionRange {
                start: segment.start,
                end: segment.end,
                properties: effective_props,
            };
        }

        section_ranges
    }

    fn split_run_by_soft_breaks(run: Run) -> Vec<Run> {
        if !run.text.contains('\n') {
            return vec![run];
        }

        if run.image.is_some() || run.text_box.is_some() || run.field.is_some() {
            return vec![run];
        }

        let mut segments = Vec::new();
        let mut template = run.clone();
        template.node_id = None;
        template.text_node_id = None;

        let mut buffer = String::new();

        for ch in run.text.chars() {
            if ch == '\n' {
                if !buffer.is_empty() {
                    let mut text_run = template.clone();
                    text_run.text = std::mem::take(&mut buffer);
                    segments.push(text_run);
                }
                let mut break_run = template.clone();
                break_run.text = "\n".to_string();
                segments.push(break_run);
            } else {
                buffer.push(ch);
            }
        }

        if !buffer.is_empty() {
            let mut trailing = template;
            trailing.text = buffer;
            segments.push(trailing);
        }

        if segments.is_empty() {
            vec![run]
        } else {
            for segment in segments.iter_mut().skip(1) {
                segment.comment_reference_ids.clear();
                segment.footnote_reference_ids.clear();
                segment.endnote_reference_ids.clear();
                segment.has_footnote_ref = false;
                segment.has_endnote_ref = false;
            }
            segments
        }
    }

    fn parse_hyperlink_attributes(attrs: Attributes<'_>) -> RunHyperlink {
        let mut hyperlink = RunHyperlink::default();
        for attr in attrs.flatten() {
            let key = attr.key.as_ref();
            let value = attr
                .unescape_value()
                .map(|cow| cow.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
            match key {
                b"r:id" => {
                    if !value.is_empty() {
                        hyperlink.relationship_id = Some(value);
                    }
                }
                b"w:anchor" => {
                    if !value.is_empty() {
                        hyperlink.anchor = Some(value);
                    }
                }
                b"w:tooltip" => {
                    if !value.is_empty() {
                        hyperlink.tooltip = Some(value);
                    }
                }
                b"w:tgtFrame" => {
                    if !value.is_empty() {
                        hyperlink.target_frame = Some(value);
                    }
                }
                _ => {}
            }
        }
        hyperlink
    }

    /// Parses a `wp:inline` or `wp:anchor` drawing and extracts image metadata
    /// such as dimensions, relationship id, and floating position.
    fn parse_drawing(
        &self,
        reader: &mut dyn XmlEventSource,
    ) -> Result<Option<DrawingObject>, Box<dyn std::error::Error>> {
        let mut depth = 1usize;
        let mut image = RunImage::default();
        let mut has_embed = false;
        let mut has_chart = false;
        let mut has_group = false;
        let mut shape_group: Option<RunShapeGroup> = None;
        let mut has_smartart = false;
        let mut buf = Vec::new();
        let mut in_position_h = false;
        let mut in_position_v = false;
        let mut pending_offset: Option<char> = None;
        let mut pending_align: Option<char> = None;
        let mut in_wrap_polygon = false;
        let mut text_box_text: Option<String> = None;
        let mut text_box_blocks: Option<Vec<BodyElement>> = None;
        let mut text_box_fill_color: Option<String> = None;
        let mut text_box_fill_opacity: Option<f64> = None;
        let mut text_box_fill_theme_color: Option<String> = None;
        let mut text_box_fill_theme_transforms = Vec::new();
        let mut text_box_insets = TextBoxInsets::default();
        let mut text_box_vertical_alignment = TextBoxVerticalAlignment::Top;
        let mut has_wordprocessing_shape = false;
        let mut text_box_shape = TextBoxShape::Rectangle;
        let mut text_box_preset: Option<String> = None;
        let mut line_head_arrow = None;
        let mut line_tail_arrow = None;
        // DrawingML anchor can contain either a modern `a:ln` definition or a legacy VML
        // fallback. We capture both so the renderer can mirror Word's stroke style.
        let mut text_box_border: Option<TextBoxBorder> = None;
        let mut text_box_shadow: Option<ShapeShadow> = None;
        let mut text_box_inner_shadow: Option<ShapeShadow> = None;
        let mut text_box_glow: Option<ShapeGlow> = None;
        let mut text_box_blur_radius_px: Option<f64> = None;
        let mut text_box_soft_edge_radius_px: Option<f64> = None;
        let mut in_line_definition = false;
        let mut in_line_solid_fill = false;
        let mut in_shape_solid_fill = false;
        let mut in_effect_list = false;
        let mut in_outer_shadow = false;
        let mut in_inner_shadow = false;
        let mut in_glow = false;
        let mut current_border = TextBoxBorder::default();
        let mut active_shape_theme_color: Option<ShapeThemeColorTarget> = None;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    match e.name().as_ref() {
                        // Stored properties of an invisible line must not
                        // become the visible shape's fill or effects.
                        b"a14:hiddenLine" => {
                            self.skip_element(reader, e.name().as_ref())?;
                            continue;
                        }
                        b"wp:inline" => {
                            image.layout.kind = RunImageKind::Inline;
                        }
                        b"wp:anchor" => {
                            image.layout.kind = RunImageKind::Floating;
                            image.layout.behind_doc = e.attributes().flatten().any(|attribute| {
                                attribute.key.as_ref() == b"behindDoc"
                                    && parse_on_off(
                                        Some(String::from_utf8_lossy(&attribute.value).to_string()),
                                        false,
                                    )
                            });
                            image.layout.relative_height =
                                e.attributes().flatten().find_map(|attribute| {
                                    (attribute.key.as_ref() == b"relativeHeight")
                                        .then(|| {
                                            attribute.unescape_value().ok()?.parse::<u32>().ok()
                                        })
                                        .flatten()
                                });
                            if e.attributes().flatten().any(|attribute| {
                                attribute.key.as_ref() == b"simplePos"
                                    && Self::drawingml_bool_attr(&attribute)
                            }) {
                                image.layout.simple_position = Some(ImageSimplePosition::default());
                            }
                            Self::populate_wrap_distances(&mut image.layout, e);
                        }
                        b"wp:simplePos" => {
                            Self::populate_simple_position(&mut image.layout, e);
                        }
                        b"wp:positionH" => {
                            in_position_h = true;
                            let mut position = image.layout.position_h.take().unwrap_or_default();
                            Self::populate_relative_from(&mut position, e);
                            image.layout.position_h = Some(position);
                        }
                        b"wp:positionV" => {
                            in_position_v = true;
                            let mut position = image.layout.position_v.take().unwrap_or_default();
                            Self::populate_relative_from(&mut position, e);
                            image.layout.position_v = Some(position);
                        }
                        b"wp:posOffset" => {
                            if in_position_h {
                                pending_offset = Some('h');
                            } else if in_position_v {
                                pending_offset = Some('v');
                            }
                        }
                        b"wp:align" => {
                            if in_position_h {
                                pending_align = Some('h');
                            } else if in_position_v {
                                pending_align = Some('v');
                            }
                        }
                        b"wp:wrapNone"
                        | b"wp:wrapSquare"
                        | b"wp:wrapTight"
                        | b"wp:wrapThrough"
                        | b"wp:wrapTopAndBottom" => {
                            Self::apply_wrap_element(&mut image.layout, e.name().as_ref(), e);
                        }
                        b"wp:wrapPolygon" => {
                            in_wrap_polygon = true;
                            image.layout.wrap_polygon = Some(Self::wrap_polygon_from_element(e));
                        }
                        b"wp:start" | b"wp:lineTo" if in_wrap_polygon => {
                            Self::push_wrap_polygon_point(&mut image.layout, e);
                        }
                        b"w:txbxContent" | b"wp:txbxContent" => {
                            let inner = self.extract_inner_xml(reader, e.name().as_ref())?;
                            if !inner.trim().is_empty() {
                                let (blocks, plain) = self.parse_text_box_content(&inner)?;
                                text_box_blocks = Some(blocks);
                                text_box_text = Some(plain);
                            }
                            continue;
                        }
                        b"wps:wsp" => {
                            has_wordprocessing_shape = true;
                        }
                        b"wpg:wgp" => {
                            has_group = true;
                            let inner = self.extract_inner_xml(reader, e.name().as_ref())?;
                            shape_group = Some(self.parse_shape_group(&inner)?);
                            continue;
                        }
                        b"dgm:relIds" => {
                            has_smartart = true;
                            image.diagram_data_relationship_id = e
                                .attributes()
                                .flatten()
                                .find(|attribute| attribute.key.as_ref() == b"r:dm")
                                .and_then(|attribute| {
                                    attribute.unescape_value().ok().map(|v| v.into_owned())
                                });
                        }
                        b"c:chart" => {
                            if let Some(relationship_id) = Self::drawing_relationship_id(e) {
                                image.relationship_id = Some(relationship_id);
                                image.chart = Some(RunChart::default());
                                has_chart = true;
                            }
                        }
                        b"a:prstGeom" => {
                            text_box_shape = Self::parse_text_box_shape(e);
                            text_box_preset = Self::parse_text_box_preset(e);
                        }
                        b"a:headEnd" => {
                            line_head_arrow = Self::drawingml_arrow_end(e);
                            current_border.head_end = line_head_arrow;
                        }
                        b"a:tailEnd" => {
                            line_tail_arrow = Self::drawingml_arrow_end(e);
                            current_border.tail_end = line_tail_arrow;
                        }
                        b"wps:bodyPr" | b"a:bodyPr" => {
                            Self::populate_text_box_body_properties(
                                &mut text_box_insets,
                                &mut text_box_vertical_alignment,
                                e,
                            );
                        }
                        b"a:ln" => {
                            // Start of a DrawingML line definition. Collect width / style / colour
                            // immediately so that nested `a:prstDash` / `a:srgbClr` nodes can mutate
                            // the in-flight border descriptor.
                            in_line_definition = true;
                            current_border = TextBoxBorder::default();
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value = value.trim();
                                    match attr.key.as_ref() {
                                        b"w" => {
                                            if let Ok(raw) = value.parse::<f64>() {
                                                current_border.width_px =
                                                    Some(Self::emu_to_px(raw));
                                            }
                                        }
                                        b"cmpd" => {
                                            if value.eq_ignore_ascii_case("dbl") {
                                                current_border.style = TextBoxBorderStyle::Double;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"a:effectLst" => in_effect_list = true,
                        b"a:outerShdw" if in_effect_list => {
                            in_outer_shadow = true;
                            text_box_shadow = Some(Self::drawingml_outer_shadow(e));
                        }
                        b"a:innerShdw" if in_effect_list => {
                            in_inner_shadow = true;
                            text_box_inner_shadow = Some(Self::drawingml_outer_shadow(e));
                        }
                        b"a:glow" if in_effect_list => {
                            in_glow = true;
                            text_box_glow = Some(Self::drawingml_glow(e));
                        }
                        b"a:blur" if in_effect_list => {
                            text_box_blur_radius_px = Self::drawingml_blur_radius(e);
                        }
                        b"a:softEdge" if in_effect_list => {
                            text_box_soft_edge_radius_px = Self::drawingml_soft_edge_radius(e);
                        }
                        b"a:solidFill" => {
                            if in_line_definition {
                                in_line_solid_fill = true;
                            } else if !in_effect_list {
                                in_shape_solid_fill = true;
                                text_box_fill_opacity = None;
                            }
                        }
                        b"a:noFill" => {
                            if in_line_definition {
                                current_border.style = TextBoxBorderStyle::None;
                                current_border.theme_color = None;
                                current_border.theme_transforms.clear();
                            } else if !in_effect_list {
                                text_box_fill_color = None;
                                text_box_fill_opacity = None;
                                text_box_fill_theme_color = None;
                                text_box_fill_theme_transforms.clear();
                            }
                        }
                        b"a:prstDash" => {
                            // Word often emits `sysDot` / `sysDash` presets. Normalise to our enum.
                            if in_line_definition
                                && current_border.style != TextBoxBorderStyle::None
                            {
                                if let Some(val_attr) = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"val")
                                {
                                    if let Ok(value) = val_attr.unescape_value() {
                                        current_border.style =
                                            TextBoxBorderStyle::from_preset_dash(&value);
                                    }
                                }
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr" => {
                            active_shape_theme_color = None;
                            if let Some(value) = crate::color::parse_explicit_color(e) {
                                if in_outer_shadow {
                                    if let Some(shadow) = text_box_shadow.as_mut() {
                                        shadow.color = Some(value);
                                        shadow.theme_color = None;
                                        shadow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::Shadow);
                                    }
                                } else if in_inner_shadow {
                                    if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                        shadow.color = Some(value);
                                        shadow.theme_color = None;
                                        shadow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::InnerShadow);
                                    }
                                } else if in_glow {
                                    if let Some(glow) = text_box_glow.as_mut() {
                                        glow.color = Some(value);
                                        glow.theme_color = None;
                                        glow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::Glow);
                                    }
                                } else if in_line_definition && in_line_solid_fill {
                                    current_border.color = Some(value);
                                    current_border.theme_color = None;
                                    current_border.theme_transforms.clear();
                                    active_shape_theme_color = Some(ShapeThemeColorTarget::Border);
                                } else if in_shape_solid_fill {
                                    text_box_fill_color = Some(value);
                                    text_box_fill_theme_color = None;
                                    text_box_fill_theme_transforms.clear();
                                    active_shape_theme_color = Some(ShapeThemeColorTarget::Fill);
                                }
                            }
                        }
                        b"a:schemeClr" => {
                            if let Some(value) = Self::drawingml_color_value(e) {
                                if in_outer_shadow {
                                    if let Some(shadow) = text_box_shadow.as_mut() {
                                        shadow.color = None;
                                        shadow.theme_color = Some(value);
                                        shadow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::Shadow);
                                    }
                                } else if in_inner_shadow {
                                    if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                        shadow.color = None;
                                        shadow.theme_color = Some(value);
                                        shadow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::InnerShadow);
                                    }
                                } else if in_glow {
                                    if let Some(glow) = text_box_glow.as_mut() {
                                        glow.color = None;
                                        glow.theme_color = Some(value);
                                        glow.theme_transforms.clear();
                                        active_shape_theme_color =
                                            Some(ShapeThemeColorTarget::Glow);
                                    }
                                } else if in_line_definition && in_line_solid_fill {
                                    current_border.color = None;
                                    current_border.theme_color = Some(value);
                                    current_border.theme_transforms.clear();
                                    active_shape_theme_color = Some(ShapeThemeColorTarget::Border);
                                } else if in_shape_solid_fill {
                                    text_box_fill_color = None;
                                    text_box_fill_theme_color = Some(value);
                                    text_box_fill_theme_transforms.clear();
                                    active_shape_theme_color = Some(ShapeThemeColorTarget::Fill);
                                }
                            }
                        }
                        b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" => {
                            if let Some(transform) = Self::drawingml_theme_color_transform(e) {
                                match active_shape_theme_color {
                                    Some(ShapeThemeColorTarget::Fill) => {
                                        text_box_fill_theme_transforms.push(transform);
                                    }
                                    Some(ShapeThemeColorTarget::Border) => {
                                        current_border.theme_transforms.push(transform);
                                    }
                                    Some(ShapeThemeColorTarget::Shadow) => {
                                        if let Some(shadow) = text_box_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::InnerShadow) => {
                                        if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Glow) => {
                                        if let Some(glow) = text_box_glow.as_mut() {
                                            glow.theme_transforms.push(transform);
                                        }
                                    }
                                    None => {}
                                }
                            }
                        }
                        b"a:alpha" if in_outer_shadow => {
                            if let Some(shadow) = text_box_shadow.as_mut() {
                                Self::populate_drawingml_shadow_alpha(shadow, e);
                            }
                        }
                        b"a:alpha" if in_inner_shadow => {
                            if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                Self::populate_drawingml_shadow_alpha(shadow, e);
                            }
                        }
                        b"a:alpha" if in_glow => {
                            if let Some(glow) = text_box_glow.as_mut() {
                                Self::populate_drawingml_glow_alpha(glow, e);
                            }
                        }
                        b"a:alpha" if in_line_definition && in_line_solid_fill => {
                            current_border.opacity = Self::drawingml_alpha(e);
                        }
                        b"a:alpha" if in_shape_solid_fill => {
                            text_box_fill_opacity = Self::drawingml_alpha(e);
                        }
                        _ => {}
                    }
                    Self::extract_drawing_attributes(&mut image, e, &mut has_embed);
                    depth = depth.saturating_add(1);
                }
                Ok(Event::Empty(ref e)) => {
                    match e.name().as_ref() {
                        b"wp:inline" => {
                            image.layout.kind = RunImageKind::Inline;
                        }
                        b"wp:anchor" => {
                            image.layout.kind = RunImageKind::Floating;
                            if e.attributes().flatten().any(|attribute| {
                                attribute.key.as_ref() == b"simplePos"
                                    && Self::drawingml_bool_attr(&attribute)
                            }) {
                                image.layout.simple_position = Some(ImageSimplePosition::default());
                            }
                            Self::populate_wrap_distances(&mut image.layout, e);
                        }
                        b"wp:simplePos" => {
                            Self::populate_simple_position(&mut image.layout, e);
                        }
                        b"wp:positionH" => {
                            let mut position = image.layout.position_h.take().unwrap_or_default();
                            Self::populate_relative_from(&mut position, e);
                            image.layout.position_h = Some(position);
                        }
                        b"wp:positionV" => {
                            let mut position = image.layout.position_v.take().unwrap_or_default();
                            Self::populate_relative_from(&mut position, e);
                            image.layout.position_v = Some(position);
                        }
                        b"wp:posOffset" => {
                            if in_position_h {
                                pending_offset = Some('h');
                            } else if in_position_v {
                                pending_offset = Some('v');
                            }
                        }
                        b"wp:wrapNone"
                        | b"wp:wrapSquare"
                        | b"wp:wrapTight"
                        | b"wp:wrapThrough"
                        | b"wp:wrapTopAndBottom" => {
                            Self::apply_wrap_element(&mut image.layout, e.name().as_ref(), e);
                        }
                        b"wp:wrapPolygon" => {
                            image.layout.wrap_polygon = Some(Self::wrap_polygon_from_element(e));
                        }
                        b"wp:start" | b"wp:lineTo" if in_wrap_polygon => {
                            Self::push_wrap_polygon_point(&mut image.layout, e);
                        }
                        b"wps:wsp" => {
                            has_wordprocessing_shape = true;
                        }
                        b"wpg:wgp" => {
                            has_group = true;
                            shape_group = Some(RunShapeGroup::default());
                        }
                        b"dgm:relIds" => {
                            has_smartart = true;
                            image.diagram_data_relationship_id = e
                                .attributes()
                                .flatten()
                                .find(|attribute| attribute.key.as_ref() == b"r:dm")
                                .and_then(|attribute| {
                                    attribute.unescape_value().ok().map(|v| v.into_owned())
                                });
                        }
                        b"c:chart" => {
                            if let Some(relationship_id) = Self::drawing_relationship_id(e) {
                                image.relationship_id = Some(relationship_id);
                                image.chart = Some(RunChart::default());
                                has_chart = true;
                            }
                        }
                        b"a:prstGeom" => {
                            text_box_shape = Self::parse_text_box_shape(e);
                            text_box_preset = Self::parse_text_box_preset(e);
                        }
                        b"a:headEnd" => {
                            line_head_arrow = Self::drawingml_arrow_end(e);
                            current_border.head_end = line_head_arrow;
                            text_box_border = Some(current_border.clone());
                        }
                        b"a:tailEnd" => {
                            line_tail_arrow = Self::drawingml_arrow_end(e);
                            current_border.tail_end = line_tail_arrow;
                            text_box_border = Some(current_border.clone());
                        }
                        b"wps:bodyPr" | b"a:bodyPr" => {
                            Self::populate_text_box_body_properties(
                                &mut text_box_insets,
                                &mut text_box_vertical_alignment,
                                e,
                            );
                        }
                        b"a:ln" => {
                            in_line_definition = false;
                            current_border = TextBoxBorder::default();
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value = value.trim();
                                    match attr.key.as_ref() {
                                        b"w" => {
                                            if let Ok(raw) = value.parse::<f64>() {
                                                current_border.width_px =
                                                    Some(Self::emu_to_px(raw));
                                            }
                                        }
                                        b"cmpd" => {
                                            if value.eq_ignore_ascii_case("dbl") {
                                                current_border.style = TextBoxBorderStyle::Double;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            text_box_border = Some(current_border.clone());
                        }
                        b"a:outerShdw" if in_effect_list => {
                            text_box_shadow = Some(Self::drawingml_outer_shadow(e));
                        }
                        b"a:innerShdw" if in_effect_list => {
                            text_box_inner_shadow = Some(Self::drawingml_outer_shadow(e));
                        }
                        b"a:glow" if in_effect_list => {
                            text_box_glow = Some(Self::drawingml_glow(e));
                        }
                        b"a:blur" if in_effect_list => {
                            text_box_blur_radius_px = Self::drawingml_blur_radius(e);
                        }
                        b"a:softEdge" if in_effect_list => {
                            text_box_soft_edge_radius_px = Self::drawingml_soft_edge_radius(e);
                        }
                        b"a:prstDash" => {
                            if current_border.style != TextBoxBorderStyle::None {
                                if let Some(val_attr) = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| attr.key.as_ref() == b"val")
                                {
                                    if let Ok(value) = val_attr.unescape_value() {
                                        current_border.style =
                                            TextBoxBorderStyle::from_preset_dash(&value);
                                    }
                                }
                            }
                            text_box_border = Some(current_border.clone());
                        }
                        b"a:noFill" => {
                            if in_line_definition {
                                current_border.style = TextBoxBorderStyle::None;
                                current_border.theme_color = None;
                                current_border.theme_transforms.clear();
                                text_box_border = Some(current_border.clone());
                            } else if !in_effect_list {
                                text_box_fill_color = None;
                                text_box_fill_opacity = None;
                                text_box_fill_theme_color = None;
                                text_box_fill_theme_transforms.clear();
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr" => {
                            if let Some(value) = crate::color::parse_explicit_color(e) {
                                if in_outer_shadow {
                                    if let Some(shadow) = text_box_shadow.as_mut() {
                                        shadow.color = Some(value);
                                        shadow.theme_color = None;
                                        shadow.theme_transforms.clear();
                                    }
                                } else if in_inner_shadow {
                                    if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                        shadow.color = Some(value);
                                        shadow.theme_color = None;
                                        shadow.theme_transforms.clear();
                                    }
                                } else if in_glow {
                                    if let Some(glow) = text_box_glow.as_mut() {
                                        glow.color = Some(value);
                                        glow.theme_color = None;
                                        glow.theme_transforms.clear();
                                    }
                                } else if in_line_definition && in_line_solid_fill {
                                    current_border.color = Some(value);
                                    current_border.theme_color = None;
                                    current_border.theme_transforms.clear();
                                } else if in_shape_solid_fill {
                                    text_box_fill_color = Some(value);
                                    text_box_fill_theme_color = None;
                                    text_box_fill_theme_transforms.clear();
                                }
                            }
                            if in_line_definition || in_line_solid_fill {
                                text_box_border = Some(current_border.clone());
                            }
                        }
                        b"a:schemeClr" => {
                            if let Some(value) = Self::drawingml_color_value(e) {
                                if in_outer_shadow {
                                    if let Some(shadow) = text_box_shadow.as_mut() {
                                        shadow.color = None;
                                        shadow.theme_color = Some(value);
                                        shadow.theme_transforms.clear();
                                    }
                                } else if in_inner_shadow {
                                    if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                        shadow.color = None;
                                        shadow.theme_color = Some(value);
                                        shadow.theme_transforms.clear();
                                    }
                                } else if in_glow {
                                    if let Some(glow) = text_box_glow.as_mut() {
                                        glow.color = None;
                                        glow.theme_color = Some(value);
                                        glow.theme_transforms.clear();
                                    }
                                } else if in_line_definition || in_line_solid_fill {
                                    current_border.color = None;
                                    current_border.theme_color = Some(value);
                                    current_border.theme_transforms.clear();
                                    text_box_border = Some(current_border.clone());
                                } else if in_shape_solid_fill {
                                    text_box_fill_color = None;
                                    text_box_fill_theme_color = Some(value);
                                    text_box_fill_theme_transforms.clear();
                                }
                            }
                        }
                        b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" => {
                            if let Some(transform) = Self::drawingml_theme_color_transform(e) {
                                match active_shape_theme_color {
                                    Some(ShapeThemeColorTarget::Fill) => {
                                        text_box_fill_theme_transforms.push(transform);
                                    }
                                    Some(ShapeThemeColorTarget::Border) => {
                                        current_border.theme_transforms.push(transform);
                                        text_box_border = Some(current_border.clone());
                                    }
                                    Some(ShapeThemeColorTarget::Shadow) => {
                                        if let Some(shadow) = text_box_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::InnerShadow) => {
                                        if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Glow) => {
                                        if let Some(glow) = text_box_glow.as_mut() {
                                            glow.theme_transforms.push(transform);
                                        }
                                    }
                                    None => {}
                                }
                            }
                        }
                        b"a:alpha" if in_outer_shadow => {
                            if let Some(shadow) = text_box_shadow.as_mut() {
                                Self::populate_drawingml_shadow_alpha(shadow, e);
                            }
                        }
                        b"a:alpha" if in_inner_shadow => {
                            if let Some(shadow) = text_box_inner_shadow.as_mut() {
                                Self::populate_drawingml_shadow_alpha(shadow, e);
                            }
                        }
                        b"a:alpha" if in_glow => {
                            if let Some(glow) = text_box_glow.as_mut() {
                                Self::populate_drawingml_glow_alpha(glow, e);
                            }
                        }
                        b"a:alpha" if in_line_definition && in_line_solid_fill => {
                            current_border.opacity = Self::drawingml_alpha(e);
                        }
                        b"a:alpha" if in_shape_solid_fill => {
                            text_box_fill_opacity = Self::drawingml_alpha(e);
                        }
                        _ => {}
                    }
                    Self::extract_drawing_attributes(&mut image, e, &mut has_embed);
                }
                Ok(Event::End(ref e)) => {
                    match e.name().as_ref() {
                        b"wp:positionH" => {
                            in_position_h = false;
                            pending_offset = None;
                            pending_align = None;
                        }
                        b"wp:positionV" => {
                            in_position_v = false;
                            pending_offset = None;
                            pending_align = None;
                        }
                        b"wp:wrapPolygon" => {
                            in_wrap_polygon = false;
                        }
                        b"a:solidFill" => {
                            if in_line_definition {
                                in_line_solid_fill = false;
                            } else {
                                in_shape_solid_fill = false;
                            }
                        }
                        b"a:outerShdw" => in_outer_shadow = false,
                        b"a:innerShdw" => in_inner_shadow = false,
                        b"a:glow" => in_glow = false,
                        b"a:effectLst" => {
                            in_effect_list = false;
                            in_outer_shadow = false;
                            in_inner_shadow = false;
                            in_glow = false;
                        }
                        b"a:ln" => {
                            in_line_definition = false;
                            in_line_solid_fill = false;
                            text_box_border = Some(current_border.clone());
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr" => {
                            let target = match active_shape_theme_color.take() {
                                Some(ShapeThemeColorTarget::Fill) => Some((
                                    &mut text_box_fill_color,
                                    &text_box_fill_theme_transforms,
                                )),
                                Some(ShapeThemeColorTarget::Border) => Some((
                                    &mut current_border.color,
                                    &current_border.theme_transforms,
                                )),
                                Some(ShapeThemeColorTarget::Shadow) => text_box_shadow
                                    .as_mut()
                                    .map(|shadow| (&mut shadow.color, &shadow.theme_transforms)),
                                Some(ShapeThemeColorTarget::InnerShadow) => text_box_inner_shadow
                                    .as_mut()
                                    .map(|shadow| (&mut shadow.color, &shadow.theme_transforms)),
                                Some(ShapeThemeColorTarget::Glow) => text_box_glow
                                    .as_mut()
                                    .map(|glow| (&mut glow.color, &glow.theme_transforms)),
                                None => None,
                            };
                            if let Some((color, transforms)) = target {
                                Self::resolve_explicit_drawingml_color(color, transforms);
                            }
                        }
                        b"a:schemeClr" => {
                            active_shape_theme_color = None;
                        }
                        _ => {}
                    }
                    if depth <= 1 {
                        break;
                    }
                    depth -= 1;
                }
                Ok(Event::Text(ref t)) => {
                    if let Some(target) = pending_offset.take() {
                        if let Some(value) = text_event_to_string(t) {
                            if let Ok(raw) = value.trim().parse::<f64>() {
                                let px = Self::emu_to_px(raw);
                                match target {
                                    'h' => {
                                        let mut position =
                                            image.layout.position_h.take().unwrap_or_default();
                                        position.offset_px = Some(px);
                                        image.layout.position_h = Some(position);
                                    }
                                    'v' => {
                                        let mut position =
                                            image.layout.position_v.take().unwrap_or_default();
                                        position.offset_px = Some(px);
                                        image.layout.position_v = Some(position);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    } else if let Some(target) = pending_align.take() {
                        if let Some(value) = text_event_to_string(t) {
                            let keyword = value.trim().to_lowercase();
                            if !keyword.is_empty() {
                                match target {
                                    'h' => {
                                        let mut position =
                                            image.layout.position_h.take().unwrap_or_default();
                                        position.align = Some(keyword);
                                        image.layout.position_h = Some(position);
                                    }
                                    'v' => {
                                        let mut position =
                                            image.layout.position_v.take().unwrap_or_default();
                                        position.align = Some(keyword);
                                        image.layout.position_v = Some(position);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parsing error: {}", e).into()),
                _ => {}
            }
            buf.clear();
        }

        if matches!(
            text_box_preset.as_deref(),
            Some("line" | "straightConnector1")
        ) {
            text_box_shape = match (line_head_arrow, line_tail_arrow) {
                (Some(_), Some(_)) => TextBoxShape::LineArrowDouble,
                (Some(_), None) | (None, Some(_)) => TextBoxShape::LineArrow,
                (None, None) => TextBoxShape::Line,
            };
        }

        if has_group {
            image.relationship_id = None;
            image.crop = None;
            image.placeholder_label = shape_group
                .as_ref()
                .filter(|group| group.children.is_empty())
                .map(|_| "Grouped Shape".to_string());
            image.shape_group = shape_group;
        } else if has_smartart {
            image.relationship_id = None;
            image.crop = None;
            image.placeholder_label = Some("SmartArt".to_string());
        }

        if has_embed || has_chart || has_group || has_smartart {
            Ok(Some(DrawingObject::Image(image)))
        } else if text_box_text.is_some() || has_wordprocessing_shape {
            let text_box = RunTextBox {
                width_px: image.width_px,
                height_px: image.height_px,
                text: text_box_text.unwrap_or_default(),
                layout: image.layout,
                blocks: text_box_blocks.unwrap_or_default(),
                border: text_box_border.unwrap_or_default(),
                shadow: text_box_shadow,
                inner_shadow: text_box_inner_shadow,
                glow: text_box_glow,
                blur_radius_px: text_box_blur_radius_px,
                soft_edge_radius_px: text_box_soft_edge_radius_px,
                fill_color: text_box_fill_color,
                fill_opacity: text_box_fill_opacity,
                fill_theme_color: text_box_fill_theme_color,
                fill_theme_transforms: text_box_fill_theme_transforms,
                insets: text_box_insets,
                vertical_alignment: text_box_vertical_alignment,
                shape: text_box_shape,
            };
            Ok(Some(DrawingObject::TextBox(text_box)))
        } else {
            Ok(None)
        }
    }

    fn resolve_explicit_drawingml_color(
        color: &mut Option<String>,
        transforms: &[ThemeColorTransform],
    ) {
        if let Some(resolved) = color
            .as_deref()
            .and_then(|value| crate::color::apply_color_transforms(value, transforms))
        {
            *color = Some(resolved);
        }
    }

    fn drawing_relationship_id(element: &BytesStart<'_>) -> Option<String> {
        element.attributes().flatten().find_map(|attribute| {
            matches!(attribute.key.as_ref(), b"r:id" | b"id")
                .then(|| {
                    attribute
                        .unescape_value()
                        .ok()
                        .map(|value| value.into_owned())
                })
                .flatten()
        })
    }

    fn parse_text_box_shape(element: &BytesStart<'_>) -> TextBoxShape {
        Self::parse_text_box_preset(element)
            .as_deref()
            .and_then(TextBoxShape::from_preset_name)
            .unwrap_or_default()
    }

    fn parse_text_box_preset(element: &BytesStart<'_>) -> Option<String> {
        element.attributes().flatten().find_map(|attr| {
            (attr.key.as_ref() == b"prst")
                .then(|| attr.unescape_value().ok().map(|value| value.into_owned()))
                .flatten()
        })
    }

    fn drawingml_arrow_end(element: &BytesStart<'_>) -> Option<TextBoxLineEnd> {
        element
            .attributes()
            .flatten()
            .find(|attr| attr.key.as_ref() == b"type")
            .and_then(|attr| attr.unescape_value().ok())
            .and_then(|value| TextBoxLineEnd::from_drawingml(&value))
    }

    fn parse_shape_group(
        &self,
        inner_xml: &str,
    ) -> Result<RunShapeGroup, Box<dyn std::error::Error>> {
        fn coordinate(element: &BytesStart<'_>, name: &[u8]) -> Option<f64> {
            element.attributes().flatten().find_map(|attribute| {
                (attribute.key.as_ref() == name)
                    .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                    .flatten()
            })
        }

        fn set_child_transform(shape: &mut RunGroupShape, element: &BytesStart<'_>) {
            match element.name().as_ref() {
                b"a:off" => {
                    shape.x = coordinate(element, b"x").unwrap_or(shape.x);
                    shape.y = coordinate(element, b"y").unwrap_or(shape.y);
                }
                b"a:ext" => {
                    shape.width = coordinate(element, b"cx").unwrap_or(shape.width);
                    shape.height = coordinate(element, b"cy").unwrap_or(shape.height);
                }
                _ => {}
            }
        }

        fn set_group_transform(group: &mut RunShapeGroup, element: &BytesStart<'_>) {
            match element.name().as_ref() {
                b"a:off" => {
                    group.x = coordinate(element, b"x").unwrap_or(group.x);
                    group.y = coordinate(element, b"y").unwrap_or(group.y);
                }
                b"a:ext" => {
                    group.width = coordinate(element, b"cx").unwrap_or(group.width);
                    group.height = coordinate(element, b"cy").unwrap_or(group.height);
                }
                _ => {}
            }
        }

        fn set_transform_attributes(
            rotation_deg: &mut f64,
            flip_horizontal: &mut bool,
            flip_vertical: &mut bool,
            element: &BytesStart<'_>,
        ) {
            for attribute in element.attributes().flatten() {
                match attribute.key.as_ref() {
                    b"rot" => {
                        if let Ok(value) = attribute.unescape_value() {
                            if let Ok(value) = value.parse::<f64>() {
                                *rotation_deg = value / 60_000.0;
                            }
                        }
                    }
                    b"flipH" => {
                        *flip_horizontal = DocumentParser::drawingml_bool_attr(&attribute);
                    }
                    b"flipV" => {
                        *flip_vertical = DocumentParser::drawingml_bool_attr(&attribute);
                    }
                    _ => {}
                }
            }
        }

        fn group_shape_border(element: &BytesStart<'_>) -> TextBoxBorder {
            let mut border = TextBoxBorder {
                color: Some("#000000".to_string()),
                ..TextBoxBorder::default()
            };
            for attribute in element.attributes().flatten() {
                let Ok(value) = attribute.unescape_value() else {
                    continue;
                };
                match attribute.key.as_ref() {
                    b"w" => {
                        if let Ok(width) = value.parse::<f64>() {
                            border.width_px = Some(DocumentParser::emu_to_px(width));
                        }
                    }
                    b"cmpd" if value.eq_ignore_ascii_case("dbl") => {
                        border.style = TextBoxBorderStyle::Double;
                    }
                    _ => {}
                }
            }
            border
        }

        fn preset_dash(element: &BytesStart<'_>) -> Option<TextBoxBorderStyle> {
            element.attributes().flatten().find_map(|attribute| {
                (attribute.key.as_ref() == b"val")
                    .then(|| attribute.unescape_value().ok())
                    .flatten()
                    .map(|value| TextBoxBorderStyle::from_preset_dash(&value))
            })
        }

        fn drawingml_hyperlink(element: &BytesStart<'_>) -> RunHyperlink {
            let mut hyperlink = RunHyperlink::default();
            for attribute in element.attributes().flatten() {
                let Ok(value) = attribute.unescape_value() else {
                    continue;
                };
                let value = value.into_owned();
                match attribute.key.as_ref() {
                    b"r:id" | b"id" => hyperlink.relationship_id = Some(value),
                    b"tooltip" => hyperlink.tooltip = Some(value),
                    b"tgtFrame" => hyperlink.target_frame = Some(value),
                    _ => {}
                }
            }
            hyperlink
        }

        let mut reader = Reader::from_str(inner_xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut group = RunShapeGroup::default();
        let mut current_shape: Option<RunGroupShape> = None;
        let mut in_group_properties = false;
        let mut in_shape_properties = false;
        let mut in_transform = false;
        let mut in_line = false;
        let mut in_shape_fill = false;
        let mut in_line_fill = false;
        let mut in_effect_list = false;
        let mut in_outer_shadow = false;
        let mut in_inner_shadow = false;
        let mut in_glow = false;
        let mut active_theme_color: Option<ShapeThemeColorTarget> = None;
        let mut extension_depth = 0usize;
        let mut next_z_order = 0usize;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref element)) => {
                    if extension_depth > 0 {
                        extension_depth += 1;
                        buf.clear();
                        continue;
                    }
                    match element.name().as_ref() {
                        b"a:extLst" => extension_depth = 1,
                        b"wpg:grpSp" => {
                            let inner =
                                self.extract_inner_xml(&mut reader, element.name().as_ref())?;
                            let mut nested = self.parse_shape_group(&inner)?;
                            nested.z_order = next_z_order;
                            next_z_order += 1;
                            group.groups.push(nested);
                            buf.clear();
                            continue;
                        }
                        b"wpg:grpSpPr" => in_group_properties = true,
                        b"wps:wsp" => {
                            current_shape = Some(RunGroupShape {
                                z_order: next_z_order,
                                ..RunGroupShape::default()
                            });
                            next_z_order += 1;
                            active_theme_color = None;
                        }
                        b"wps:spPr" if current_shape.is_some() => in_shape_properties = true,
                        b"w:txbxContent" | b"wp:txbxContent" if current_shape.is_some() => {
                            let inner =
                                self.extract_inner_xml(&mut reader, element.name().as_ref())?;
                            let (_, plain_text) = self.parse_text_box_content(&inner)?;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.text = plain_text;
                            }
                            buf.clear();
                            continue;
                        }
                        b"wps:bodyPr" if current_shape.is_some() => {
                            if let Some(shape) = current_shape.as_mut() {
                                Self::populate_text_box_body_properties(
                                    &mut shape.insets,
                                    &mut shape.vertical_alignment,
                                    element,
                                );
                            }
                        }
                        b"a:hlinkClick" if current_shape.is_some() => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.hyperlink = Some(drawingml_hyperlink(element));
                            }
                        }
                        b"a:xfrm" if in_group_properties || in_shape_properties => {
                            in_transform = true;
                            if in_shape_properties {
                                if let Some(shape) = current_shape.as_mut() {
                                    set_transform_attributes(
                                        &mut shape.rotation_deg,
                                        &mut shape.flip_horizontal,
                                        &mut shape.flip_vertical,
                                        element,
                                    );
                                }
                            } else {
                                set_transform_attributes(
                                    &mut group.rotation_deg,
                                    &mut group.flip_horizontal,
                                    &mut group.flip_vertical,
                                    element,
                                );
                            }
                        }
                        b"a:prstGeom" if in_shape_properties => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.shape = Self::parse_text_box_shape(element);
                            }
                        }
                        b"a:ln" if in_shape_properties => {
                            in_line = true;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.border = Some(group_shape_border(element));
                            }
                        }
                        b"a:effectLst" if in_shape_properties => in_effect_list = true,
                        b"a:outerShdw" if in_effect_list => {
                            in_outer_shadow = true;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.shadow = Some(Self::drawingml_outer_shadow(element));
                            }
                        }
                        b"a:innerShdw" if in_effect_list => {
                            in_inner_shadow = true;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.inner_shadow = Some(Self::drawingml_outer_shadow(element));
                            }
                        }
                        b"a:glow" if in_effect_list => {
                            in_glow = true;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.glow = Some(Self::drawingml_glow(element));
                            }
                        }
                        b"a:blur" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.blur_radius_px = Self::drawingml_blur_radius(element);
                            }
                        }
                        b"a:softEdge" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.soft_edge_radius_px =
                                    Self::drawingml_soft_edge_radius(element);
                            }
                        }
                        b"a:prstDash" if in_line => {
                            if let (Some(shape), Some(style)) =
                                (current_shape.as_mut(), preset_dash(element))
                            {
                                if let Some(border) = shape.border.as_mut() {
                                    border.style = style;
                                }
                            }
                        }
                        b"a:headEnd" if in_line => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.head_end = Self::drawingml_arrow_end(element);
                            }
                        }
                        b"a:tailEnd" if in_line => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.tail_end = Self::drawingml_arrow_end(element);
                            }
                        }
                        b"a:solidFill" if in_line => in_line_fill = true,
                        b"a:solidFill" if in_shape_properties && !in_effect_list => {
                            in_shape_fill = true;
                            if let Some(shape) = current_shape.as_mut() {
                                shape.fill_opacity = None;
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_outer_shadow =>
                        {
                            active_theme_color = None;
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.shadow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shadow.color = Some(value);
                                shadow.theme_color = None;
                                shadow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Shadow);
                            }
                        }
                        b"a:schemeClr" if in_outer_shadow => {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.shadow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                shadow.color = None;
                                shadow.theme_color = Some(value);
                                shadow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Shadow);
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_inner_shadow =>
                        {
                            active_theme_color = None;
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.inner_shadow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shadow.color = Some(value);
                                shadow.theme_color = None;
                                shadow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::InnerShadow);
                            }
                        }
                        b"a:schemeClr" if in_inner_shadow => {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.inner_shadow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                shadow.color = None;
                                shadow.theme_color = Some(value);
                                shadow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::InnerShadow);
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_glow =>
                        {
                            active_theme_color = None;
                            if let (Some(glow), Some(value)) = (
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                glow.color = Some(value);
                                glow.theme_color = None;
                                glow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Glow);
                            }
                        }
                        b"a:schemeClr" if in_glow => {
                            if let (Some(glow), Some(value)) = (
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                glow.color = None;
                                glow.theme_color = Some(value);
                                glow.theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Glow);
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_line_fill =>
                        {
                            active_theme_color = None;
                            if let (Some(shape), Some(value)) = (
                                current_shape.as_mut(),
                                crate::color::parse_explicit_color(element),
                            ) {
                                if let Some(border) = shape.border.as_mut() {
                                    border.color = Some(value);
                                    border.theme_color = None;
                                    border.theme_transforms.clear();
                                    active_theme_color = Some(ShapeThemeColorTarget::Border);
                                }
                            }
                        }
                        b"a:schemeClr" if in_line_fill => {
                            if let (Some(shape), Some(value)) =
                                (current_shape.as_mut(), Self::drawingml_color_value(element))
                            {
                                if let Some(border) = shape.border.as_mut() {
                                    border.color = None;
                                    border.theme_color = Some(value);
                                    border.theme_transforms.clear();
                                    active_theme_color = Some(ShapeThemeColorTarget::Border);
                                }
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_shape_fill =>
                        {
                            active_theme_color = None;
                            if let (Some(shape), Some(value)) = (
                                current_shape.as_mut(),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shape.fill_color = Some(value);
                                shape.fill_theme_color = None;
                                shape.fill_theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Fill);
                            }
                        }
                        b"a:schemeClr" if in_shape_fill => {
                            if let (Some(shape), Some(value)) =
                                (current_shape.as_mut(), Self::drawingml_color_value(element))
                            {
                                shape.fill_color = None;
                                shape.fill_theme_color = Some(value);
                                shape.fill_theme_transforms.clear();
                                active_theme_color = Some(ShapeThemeColorTarget::Fill);
                            }
                        }
                        b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" => {
                            if let (Some(shape), Some(transform)) = (
                                current_shape.as_mut(),
                                Self::drawingml_theme_color_transform(element),
                            ) {
                                match active_theme_color {
                                    Some(ShapeThemeColorTarget::Fill) => {
                                        shape.fill_theme_transforms.push(transform);
                                    }
                                    Some(ShapeThemeColorTarget::Border) => {
                                        if let Some(border) = shape.border.as_mut() {
                                            border.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Shadow) => {
                                        if let Some(shadow) = shape.shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::InnerShadow) => {
                                        if let Some(shadow) = shape.inner_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Glow) => {
                                        if let Some(glow) = shape.glow.as_mut() {
                                            glow.theme_transforms.push(transform);
                                        }
                                    }
                                    None => {}
                                }
                            }
                        }
                        b"a:alpha" if in_outer_shadow => {
                            if let Some(shadow) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.shadow.as_mut())
                            {
                                Self::populate_drawingml_shadow_alpha(shadow, element);
                            }
                        }
                        b"a:alpha" if in_inner_shadow => {
                            if let Some(shadow) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.inner_shadow.as_mut())
                            {
                                Self::populate_drawingml_shadow_alpha(shadow, element);
                            }
                        }
                        b"a:alpha" if in_glow => {
                            if let Some(glow) =
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut())
                            {
                                Self::populate_drawingml_glow_alpha(glow, element);
                            }
                        }
                        b"a:alpha" if in_line_fill => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.opacity = Self::drawingml_alpha(element);
                            }
                        }
                        b"a:alpha" if in_shape_fill => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.fill_opacity = Self::drawingml_alpha(element);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref element)) => {
                    if extension_depth > 0 {
                        buf.clear();
                        continue;
                    }
                    match element.name().as_ref() {
                        b"a:chOff" if in_group_properties => {
                            group.child_offset_x = coordinate(element, b"x").unwrap_or(0.0);
                            group.child_offset_y = coordinate(element, b"y").unwrap_or(0.0);
                        }
                        b"a:chExt" if in_group_properties => {
                            group.child_extent_width = coordinate(element, b"cx").unwrap_or(0.0);
                            group.child_extent_height = coordinate(element, b"cy").unwrap_or(0.0);
                        }
                        b"a:hlinkClick" if current_shape.is_some() => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.hyperlink = Some(drawingml_hyperlink(element));
                            }
                        }
                        b"wps:bodyPr" if current_shape.is_some() => {
                            if let Some(shape) = current_shape.as_mut() {
                                Self::populate_text_box_body_properties(
                                    &mut shape.insets,
                                    &mut shape.vertical_alignment,
                                    element,
                                );
                            }
                        }
                        b"a:off" | b"a:ext" if in_shape_properties && in_transform => {
                            if let Some(shape) = current_shape.as_mut() {
                                set_child_transform(shape, element);
                            }
                        }
                        b"a:off" | b"a:ext" if in_group_properties && in_transform => {
                            set_group_transform(&mut group, element);
                        }
                        b"a:prstGeom" if in_shape_properties => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.shape = Self::parse_text_box_shape(element);
                            }
                        }
                        b"a:ln" if in_shape_properties => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.border = Some(group_shape_border(element));
                            }
                        }
                        b"a:outerShdw" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.shadow = Some(Self::drawingml_outer_shadow(element));
                            }
                        }
                        b"a:innerShdw" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.inner_shadow = Some(Self::drawingml_outer_shadow(element));
                            }
                        }
                        b"a:glow" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.glow = Some(Self::drawingml_glow(element));
                            }
                        }
                        b"a:blur" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.blur_radius_px = Self::drawingml_blur_radius(element);
                            }
                        }
                        b"a:softEdge" if in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.soft_edge_radius_px =
                                    Self::drawingml_soft_edge_radius(element);
                            }
                        }
                        b"a:prstDash" if in_line => {
                            if let (Some(shape), Some(style)) =
                                (current_shape.as_mut(), preset_dash(element))
                            {
                                if let Some(border) = shape.border.as_mut() {
                                    border.style = style;
                                }
                            }
                        }
                        b"a:headEnd" if in_line => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.head_end = Self::drawingml_arrow_end(element);
                            }
                        }
                        b"a:tailEnd" if in_line => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.tail_end = Self::drawingml_arrow_end(element);
                            }
                        }
                        b"a:noFill" if in_line => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.border = None;
                            }
                            active_theme_color = None;
                        }
                        b"a:noFill" if in_shape_properties && !in_effect_list => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.fill_color = None;
                                shape.fill_opacity = None;
                                shape.fill_theme_color = None;
                                shape.fill_theme_transforms.clear();
                            }
                            active_theme_color = None;
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_line_fill =>
                        {
                            if let (Some(shape), Some(value)) = (
                                current_shape.as_mut(),
                                crate::color::parse_explicit_color(element),
                            ) {
                                if let Some(border) = shape.border.as_mut() {
                                    border.color = Some(value);
                                    border.theme_color = None;
                                    border.theme_transforms.clear();
                                }
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_outer_shadow =>
                        {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.shadow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shadow.color = Some(value);
                                shadow.theme_color = None;
                                shadow.theme_transforms.clear();
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_inner_shadow =>
                        {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.inner_shadow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shadow.color = Some(value);
                                shadow.theme_color = None;
                                shadow.theme_transforms.clear();
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_glow =>
                        {
                            if let (Some(glow), Some(value)) = (
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut()),
                                crate::color::parse_explicit_color(element),
                            ) {
                                glow.color = Some(value);
                                glow.theme_color = None;
                                glow.theme_transforms.clear();
                            }
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                            if in_shape_fill =>
                        {
                            if let (Some(shape), Some(value)) = (
                                current_shape.as_mut(),
                                crate::color::parse_explicit_color(element),
                            ) {
                                shape.fill_color = Some(value);
                                shape.fill_theme_color = None;
                                shape.fill_theme_transforms.clear();
                            }
                        }
                        b"a:schemeClr" if in_outer_shadow => {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.shadow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                shadow.color = None;
                                shadow.theme_color = Some(value);
                                shadow.theme_transforms.clear();
                            }
                        }
                        b"a:schemeClr" if in_inner_shadow => {
                            if let (Some(shadow), Some(value)) = (
                                current_shape
                                    .as_mut()
                                    .and_then(|shape| shape.inner_shadow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                shadow.color = None;
                                shadow.theme_color = Some(value);
                                shadow.theme_transforms.clear();
                            }
                        }
                        b"a:schemeClr" if in_glow => {
                            if let (Some(glow), Some(value)) = (
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut()),
                                Self::drawingml_color_value(element),
                            ) {
                                glow.color = None;
                                glow.theme_color = Some(value);
                                glow.theme_transforms.clear();
                            }
                        }
                        b"a:schemeClr" if in_line_fill => {
                            if let (Some(shape), Some(value)) =
                                (current_shape.as_mut(), Self::drawingml_color_value(element))
                            {
                                if let Some(border) = shape.border.as_mut() {
                                    border.color = None;
                                    border.theme_color = Some(value);
                                    border.theme_transforms.clear();
                                }
                            }
                        }
                        b"a:schemeClr" if in_shape_fill => {
                            if let (Some(shape), Some(value)) =
                                (current_shape.as_mut(), Self::drawingml_color_value(element))
                            {
                                shape.fill_color = None;
                                shape.fill_theme_color = Some(value);
                                shape.fill_theme_transforms.clear();
                            }
                        }
                        b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" => {
                            if let (Some(shape), Some(transform)) = (
                                current_shape.as_mut(),
                                Self::drawingml_theme_color_transform(element),
                            ) {
                                match active_theme_color {
                                    Some(ShapeThemeColorTarget::Fill) => {
                                        shape.fill_theme_transforms.push(transform);
                                    }
                                    Some(ShapeThemeColorTarget::Border) => {
                                        if let Some(border) = shape.border.as_mut() {
                                            border.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Shadow) => {
                                        if let Some(shadow) = shape.shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::InnerShadow) => {
                                        if let Some(shadow) = shape.inner_shadow.as_mut() {
                                            shadow.theme_transforms.push(transform);
                                        }
                                    }
                                    Some(ShapeThemeColorTarget::Glow) => {
                                        if let Some(glow) = shape.glow.as_mut() {
                                            glow.theme_transforms.push(transform);
                                        }
                                    }
                                    None => {}
                                }
                            }
                        }
                        b"a:alpha" if in_outer_shadow => {
                            if let Some(shadow) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.shadow.as_mut())
                            {
                                Self::populate_drawingml_shadow_alpha(shadow, element);
                            }
                        }
                        b"a:alpha" if in_inner_shadow => {
                            if let Some(shadow) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.inner_shadow.as_mut())
                            {
                                Self::populate_drawingml_shadow_alpha(shadow, element);
                            }
                        }
                        b"a:alpha" if in_glow => {
                            if let Some(glow) =
                                current_shape.as_mut().and_then(|shape| shape.glow.as_mut())
                            {
                                Self::populate_drawingml_glow_alpha(glow, element);
                            }
                        }
                        b"a:alpha" if in_line_fill => {
                            if let Some(border) = current_shape
                                .as_mut()
                                .and_then(|shape| shape.border.as_mut())
                            {
                                border.opacity = Self::drawingml_alpha(element);
                            }
                        }
                        b"a:alpha" if in_shape_fill => {
                            if let Some(shape) = current_shape.as_mut() {
                                shape.fill_opacity = Self::drawingml_alpha(element);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref element)) => {
                    if extension_depth > 0 {
                        extension_depth -= 1;
                        buf.clear();
                        continue;
                    }
                    match element.name().as_ref() {
                        b"wpg:grpSpPr" => in_group_properties = false,
                        b"wps:wsp" => {
                            if let Some(shape) = current_shape.take() {
                                group.children.push(shape);
                            }
                            active_theme_color = None;
                        }
                        b"wps:spPr" => in_shape_properties = false,
                        b"a:xfrm" => in_transform = false,
                        b"a:ln" => {
                            in_line = false;
                            in_line_fill = false;
                        }
                        b"a:solidFill" if in_line_fill => in_line_fill = false,
                        b"a:solidFill" => in_shape_fill = false,
                        b"a:outerShdw" => in_outer_shadow = false,
                        b"a:innerShdw" => in_inner_shadow = false,
                        b"a:glow" => in_glow = false,
                        b"a:effectLst" => {
                            in_effect_list = false;
                            in_outer_shadow = false;
                            in_inner_shadow = false;
                            in_glow = false;
                        }
                        b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr" => {
                            if let Some(shape) = current_shape.as_mut() {
                                let target = match active_theme_color.take() {
                                    Some(ShapeThemeColorTarget::Fill) => {
                                        Some((&mut shape.fill_color, &shape.fill_theme_transforms))
                                    }
                                    Some(ShapeThemeColorTarget::Border) => {
                                        shape.border.as_mut().map(|border| {
                                            (&mut border.color, &border.theme_transforms)
                                        })
                                    }
                                    Some(ShapeThemeColorTarget::Shadow) => {
                                        shape.shadow.as_mut().map(|shadow| {
                                            (&mut shadow.color, &shadow.theme_transforms)
                                        })
                                    }
                                    Some(ShapeThemeColorTarget::InnerShadow) => {
                                        shape.inner_shadow.as_mut().map(|shadow| {
                                            (&mut shadow.color, &shadow.theme_transforms)
                                        })
                                    }
                                    Some(ShapeThemeColorTarget::Glow) => shape
                                        .glow
                                        .as_mut()
                                        .map(|glow| (&mut glow.color, &glow.theme_transforms)),
                                    None => None,
                                };
                                if let Some((color, transforms)) = target {
                                    Self::resolve_explicit_drawingml_color(color, transforms);
                                }
                            }
                        }
                        b"a:schemeClr" => active_theme_color = None,
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => return Err(format!("XML parsing error: {error}").into()),
                _ => {}
            }
            buf.clear();
        }

        if group.child_extent_width <= 0.0 {
            let shape_max = group
                .children
                .iter()
                .map(|shape| shape.x + shape.width)
                .fold(0.0, f64::max);
            let group_max = group
                .groups
                .iter()
                .map(|nested| nested.x + nested.width)
                .fold(0.0, f64::max);
            group.child_extent_width = shape_max.max(group_max) - group.child_offset_x;
        }
        if group.child_extent_height <= 0.0 {
            let shape_max = group
                .children
                .iter()
                .map(|shape| shape.y + shape.height)
                .fold(0.0, f64::max);
            let group_max = group
                .groups
                .iter()
                .map(|nested| nested.y + nested.height)
                .fold(0.0, f64::max);
            group.child_extent_height = shape_max.max(group_max) - group.child_offset_y;
        }
        Ok(group)
    }

    fn drawingml_color_value(element: &BytesStart<'_>) -> Option<String> {
        element.attributes().flatten().find_map(|attr| {
            (attr.key.as_ref() == b"val")
                .then(|| attr.unescape_value().ok().map(|value| value.into_owned()))
                .flatten()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    }

    fn drawingml_outer_shadow(element: &BytesStart<'_>) -> ShapeShadow {
        let mut shadow = ShapeShadow::default();
        for attribute in element.attributes().flatten() {
            let Ok(value) = attribute.unescape_value() else {
                continue;
            };
            match attribute.key.as_ref() {
                b"blurRad" => {
                    if let Ok(value) = value.parse::<f64>() {
                        shadow.blur_radius_px = Self::emu_to_px(value).max(0.0);
                    }
                }
                b"dist" => {
                    if let Ok(value) = value.parse::<f64>() {
                        shadow.distance_px = Self::emu_to_px(value).max(0.0);
                    }
                }
                b"dir" => {
                    if let Ok(value) = value.parse::<f64>() {
                        shadow.direction_deg = value / 60_000.0;
                    }
                }
                _ => {}
            }
        }
        shadow
    }

    fn populate_drawingml_shadow_alpha(shadow: &mut ShapeShadow, element: &BytesStart<'_>) {
        if let Some(value) = element.attributes().flatten().find_map(|attribute| {
            (attribute.key.as_ref() == b"val")
                .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                .flatten()
        }) {
            shadow.opacity = (value / 100_000.0).clamp(0.0, 1.0);
        }
    }

    fn drawingml_glow(element: &BytesStart<'_>) -> ShapeGlow {
        let mut glow = ShapeGlow::default();
        if let Some(radius) = element.attributes().flatten().find_map(|attribute| {
            (attribute.key.as_ref() == b"rad")
                .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                .flatten()
        }) {
            glow.radius_px = Self::emu_to_px(radius).max(0.0);
        }
        glow
    }

    fn drawingml_soft_edge_radius(element: &BytesStart<'_>) -> Option<f64> {
        element.attributes().flatten().find_map(|attribute| {
            (attribute.key.as_ref() == b"rad")
                .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                .flatten()
                .filter(|radius| radius.is_finite())
                .map(|radius| Self::emu_to_px(radius).max(0.0))
        })
    }

    fn drawingml_blur_radius(element: &BytesStart<'_>) -> Option<f64> {
        element.attributes().flatten().find_map(|attribute| {
            (attribute.key.as_ref() == b"rad")
                .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                .flatten()
                .filter(|radius| radius.is_finite())
                .map(|radius| Self::emu_to_px(radius).max(0.0))
        })
    }

    fn populate_drawingml_glow_alpha(glow: &mut ShapeGlow, element: &BytesStart<'_>) {
        if let Some(value) = Self::drawingml_alpha(element) {
            glow.opacity = value;
        }
    }

    fn drawingml_alpha(element: &BytesStart<'_>) -> Option<f64> {
        element
            .attributes()
            .flatten()
            .find_map(|attribute| {
                (attribute.key.as_ref() == b"val")
                    .then(|| attribute.unescape_value().ok()?.parse::<f64>().ok())
                    .flatten()
            })
            .filter(|value| value.is_finite())
            .map(|value| (value / 100_000.0).clamp(0.0, 1.0))
    }

    fn drawingml_theme_color_transform(element: &BytesStart<'_>) -> Option<ThemeColorTransform> {
        crate::color::parse_color_transform(element)
    }

    fn extract_inner_xml(
        &self,
        reader: &mut dyn XmlEventSource,
        tag: &[u8],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        let mut writer = Writer::new(Vec::new());
        let mut depth = 1usize;

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    depth += 1;
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                Ok(Event::Empty(ref e)) => {
                    writer.write_event(Event::Empty(e.to_owned()))?;
                }
                Ok(Event::End(ref e)) => {
                    depth -= 1;
                    if depth == 0 && e.name().as_ref() == tag {
                        break;
                    }
                    writer.write_event(Event::End(e.to_owned()))?;
                }
                Ok(Event::Text(ref e)) => {
                    writer.write_event(Event::Text(e.to_owned()))?;
                }
                Ok(Event::GeneralRef(ref e)) => {
                    // quick-xml emits `&amp;`, numeric references, and other
                    // entities separately from surrounding text. Nested
                    // text-box extraction must retain those events or their
                    // visible characters disappear on the second parse.
                    writer.write_event(Event::GeneralRef(e.to_owned()))?;
                }
                Ok(Event::CData(ref e)) => {
                    writer.write_event(Event::CData(e.to_owned()))?;
                }
                Ok(Event::Comment(ref e)) => {
                    writer.write_event(Event::Comment(e.to_owned()))?;
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parsing error: {}", e).into()),
                _ => {}
            }
        }

        let bytes = writer.into_inner();
        Ok(String::from_utf8(bytes)?)
    }

    fn parse_text_box_content(
        &self,
        inner_xml: &str,
    ) -> Result<(Vec<BodyElement>, String), Box<dyn std::error::Error>> {
        const MAX_TEXT_BOX_DEPTH: usize = 64;
        if self.text_box_depth >= MAX_TEXT_BOX_DEPTH {
            return Err(format!(
                "DOCX text box nesting exceeds supported depth of {MAX_TEXT_BOX_DEPTH}"
            )
            .into());
        }
        const DOC_PREFIX: &str = r#"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:body>"#;
        const DOC_SUFFIX: &str = "</w:body></w:document>";

        let xml = format!("{DOC_PREFIX}{inner_xml}{DOC_SUFFIX}");
        let parser = DocumentParser {
            text_box_depth: self.text_box_depth + 1,
        };
        let document = parser.parse_document_xml(&xml)?;

        let mut blocks = Vec::new();
        let mut lines = Vec::new();
        if let Some(body) = document.body {
            for element in body.elements {
                if let BodyElement::Paragraph(paragraph) = &element {
                    let paragraph = paragraph.as_ref();
                    let mut line = String::new();
                    for run in &paragraph.runs {
                        line.push_str(&run.text);
                    }
                    if !line.trim().is_empty() {
                        lines.push(line);
                    }
                }
                blocks.push(element);
            }
        }

        Ok((blocks, lines.join("\n")))
    }

    fn populate_text_box_body_properties(
        insets: &mut TextBoxInsets,
        vertical_alignment: &mut TextBoxVerticalAlignment,
        element: &BytesStart<'_>,
    ) {
        for attr in element.attributes().flatten() {
            let Ok(value) = attr.unescape_value() else {
                continue;
            };
            let value = value.trim();
            match attr.key.as_ref() {
                b"lIns" => {
                    if let Ok(emu) = value.parse::<f64>() {
                        insets.left_px = Self::emu_to_px(emu).max(0.0);
                    }
                }
                b"tIns" => {
                    if let Ok(emu) = value.parse::<f64>() {
                        insets.top_px = Self::emu_to_px(emu).max(0.0);
                    }
                }
                b"rIns" => {
                    if let Ok(emu) = value.parse::<f64>() {
                        insets.right_px = Self::emu_to_px(emu).max(0.0);
                    }
                }
                b"bIns" => {
                    if let Ok(emu) = value.parse::<f64>() {
                        insets.bottom_px = Self::emu_to_px(emu).max(0.0);
                    }
                }
                b"anchor" => {
                    *vertical_alignment = match value.to_ascii_lowercase().as_str() {
                        "ctr" | "center" | "middle" => TextBoxVerticalAlignment::Center,
                        "b" | "bottom" => TextBoxVerticalAlignment::Bottom,
                        "just" | "justify" => TextBoxVerticalAlignment::Justified,
                        "dist" | "distributed" => TextBoxVerticalAlignment::Distributed,
                        _ => TextBoxVerticalAlignment::Top,
                    };
                }
                _ => {}
            }
        }
    }

    fn populate_vml_text_box_insets(insets: &mut TextBoxInsets, element: &BytesStart<'_>) {
        let Some(value) = element
            .attributes()
            .flatten()
            .find(|attr| attr.key.as_ref() == b"inset")
            .and_then(|attr| attr.unescape_value().ok())
        else {
            return;
        };
        let values: Vec<f64> = value
            .split(',')
            .filter_map(|part| Self::points_str_to_px(part.trim()))
            .collect();
        if let [left, top, right, bottom] = values.as_slice() {
            insets.left_px = left.max(0.0);
            insets.top_px = top.max(0.0);
            insets.right_px = right.max(0.0);
            insets.bottom_px = bottom.max(0.0);
        }
    }

    fn parse_vml_picture(
        &self,
        reader: &mut dyn XmlEventSource,
    ) -> Result<Option<DrawingObject>, Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        let mut depth = 1usize;
        let mut image = RunImage::default();
        let mut is_absolute = false;
        let mut width_px: Option<f64> = None;
        let mut height_px: Option<f64> = None;
        let mut position_h: Option<ImagePosition> = None;
        let mut position_v: Option<ImagePosition> = None;
        let mut rotation_deg: Option<f64> = None;
        let mut blocks: Option<Vec<BodyElement>> = None;
        let mut text: Option<String> = None;
        // VML StrokeWeight defaults to one CSS pixel (MSDN VML reference).
        // Materialize it here so SDK-created/DrawingML shapes keep their own defaults.
        let mut border = TextBoxBorder {
            width_px: Some(1.0),
            ..TextBoxBorder::default()
        };
        let mut fill_color: Option<String> = None;
        let mut stroke_enabled = true;
        let mut fill_enabled = true;
        let mut insets = TextBoxInsets::default();
        let mut vertical_alignment = TextBoxVerticalAlignment::Top;
        let mut wrap = WrapMode::None;
        let mut dist_top = None;
        let mut dist_bottom = None;
        let mut dist_left = None;
        let mut dist_right = None;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    Self::apply_vml_imagedata_attributes(&mut image, e);
                    if e.name().as_ref() == b"o:OLEObject" {
                        Self::apply_ole_object_attributes(&mut image, e);
                    }
                    match e.name().as_ref() {
                        b"v:shape" => {
                            let mut style_value: Option<String> = None;
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"style" {
                                    style_value =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                } else if attr.key.as_ref() == b"strokeweight" {
                                    if let Ok(value) = attr.unescape_value() {
                                        if let Some(px) = Self::points_str_to_px(value.trim()) {
                                            border.width_px = Some(px);
                                        }
                                    }
                                } else if attr.key.as_ref() == b"strokecolor" {
                                    if let Ok(value) = attr.unescape_value() {
                                        if !value.trim().is_empty() {
                                            border.color =
                                                Some(Self::normalize_color_string(value.trim()));
                                        }
                                    }
                                } else if attr.key.as_ref() == b"stroked" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let val = value.trim().to_ascii_lowercase();
                                        if val == "f" || val == "false" || val == "0" {
                                            stroke_enabled = false;
                                        }
                                    }
                                } else if attr.key.as_ref() == b"filled" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let val = value.trim().to_ascii_lowercase();
                                        if val == "f" || val == "false" || val == "0" {
                                            fill_enabled = false;
                                            fill_color = None;
                                        }
                                    }
                                } else if attr.key.as_ref() == b"fillcolor" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let value = value.trim();
                                        if !value.is_empty() {
                                            fill_color = Some(Self::normalize_color_string(value));
                                        }
                                    }
                                }
                            }
                            if let Some(style_str) = style_value {
                                let parsed = Self::parse_vml_style(&style_str);
                                is_absolute |= parsed.is_absolute;
                                width_px = parsed.width_px.or(width_px);
                                height_px = parsed.height_px.or(height_px);
                                if parsed.position_h.is_some() {
                                    position_h = parsed.position_h;
                                }
                                if parsed.position_v.is_some() {
                                    position_v = parsed.position_v;
                                }
                                if parsed.rotation_deg.is_some() {
                                    rotation_deg = parsed.rotation_deg;
                                }
                                if let Some(alignment) = parsed.text_vertical_alignment {
                                    vertical_alignment = alignment;
                                }
                                dist_top = parsed.dist_top.or(dist_top);
                                dist_bottom = parsed.dist_bottom.or(dist_bottom);
                                dist_left = parsed.dist_left.or(dist_left);
                                dist_right = parsed.dist_right.or(dist_right);
                            }
                        }
                        b"w10:wrap" => {
                            wrap = Self::parse_vml_wrap_mode(e).unwrap_or(wrap);
                        }
                        b"v:textbox" => {
                            Self::populate_vml_text_box_insets(&mut insets, e);
                        }
                        b"v:stroke" => {
                            // Legacy VML fallback (used for older Office compatibility). The
                            // attributes mirror DrawingML concepts but use different naming.
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value_trimmed = value.trim();
                                    match attr.key.as_ref() {
                                        b"on" => {
                                            if matches!(
                                                value_trimmed.to_ascii_lowercase().as_str(),
                                                "f" | "false" | "0"
                                            ) {
                                                stroke_enabled = false;
                                            }
                                        }
                                        b"weight" => {
                                            if let Some(px) = Self::points_str_to_px(value_trimmed)
                                            {
                                                border.width_px = Some(px);
                                            }
                                        }
                                        b"dashstyle" => {
                                            border.style = TextBoxBorderStyle::from_vml_dashstyle(
                                                value_trimmed,
                                            );
                                        }
                                        b"color" => {
                                            if !value_trimmed.is_empty() {
                                                border.color = Some(Self::normalize_color_string(
                                                    value_trimmed,
                                                ));
                                            }
                                        }
                                        b"linestyle" => {
                                            if value_trimmed.eq_ignore_ascii_case("none") {
                                                border.style = TextBoxBorderStyle::None;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"v:fill" => {
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value_trimmed = value.trim();
                                    match attr.key.as_ref() {
                                        b"on" => {
                                            let val = value_trimmed.to_ascii_lowercase();
                                            if val == "f" || val == "false" || val == "0" {
                                                fill_enabled = false;
                                                fill_color = None;
                                            }
                                        }
                                        b"color" => {
                                            if !value_trimmed.is_empty() {
                                                fill_color = Some(Self::normalize_color_string(
                                                    value_trimmed,
                                                ));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"w:txbxContent" | b"wp:txbxContent" => {
                            let inner = self.extract_inner_xml(reader, e.name().as_ref())?;
                            if !inner.trim().is_empty() {
                                let (parsed_blocks, plain_text) =
                                    self.parse_text_box_content(&inner)?;
                                blocks = Some(parsed_blocks);
                                text = Some(plain_text);
                            }
                            continue;
                        }
                        _ => {}
                    }
                    depth = depth.saturating_add(1);
                }
                Ok(Event::Empty(ref e)) => {
                    Self::apply_vml_imagedata_attributes(&mut image, e);
                    if e.name().as_ref() == b"o:OLEObject" {
                        Self::apply_ole_object_attributes(&mut image, e);
                    }
                    match e.name().as_ref() {
                        b"v:shape" => {
                            let mut style_value: Option<String> = None;
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"style" {
                                    style_value =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                } else if attr.key.as_ref() == b"strokeweight" {
                                    if let Ok(value) = attr.unescape_value() {
                                        if let Some(px) = Self::points_str_to_px(value.trim()) {
                                            border.width_px = Some(px);
                                        }
                                    }
                                } else if attr.key.as_ref() == b"strokecolor" {
                                    if let Ok(value) = attr.unescape_value() {
                                        if !value.trim().is_empty() {
                                            border.color =
                                                Some(Self::normalize_color_string(value.trim()));
                                        }
                                    }
                                } else if attr.key.as_ref() == b"stroked" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let val = value.trim().to_ascii_lowercase();
                                        if val == "f" || val == "false" || val == "0" {
                                            stroke_enabled = false;
                                        }
                                    }
                                } else if attr.key.as_ref() == b"filled" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let val = value.trim().to_ascii_lowercase();
                                        if val == "f" || val == "false" || val == "0" {
                                            fill_enabled = false;
                                            fill_color = None;
                                        }
                                    }
                                } else if attr.key.as_ref() == b"fillcolor" {
                                    if let Ok(value) = attr.unescape_value() {
                                        let value = value.trim();
                                        if !value.is_empty() {
                                            fill_color = Some(Self::normalize_color_string(value));
                                        }
                                    }
                                }
                            }
                            if let Some(style_str) = style_value {
                                let parsed = Self::parse_vml_style(&style_str);
                                is_absolute |= parsed.is_absolute;
                                width_px = parsed.width_px.or(width_px);
                                height_px = parsed.height_px.or(height_px);
                                if parsed.position_h.is_some() {
                                    position_h = parsed.position_h;
                                }
                                if parsed.position_v.is_some() {
                                    position_v = parsed.position_v;
                                }
                                if parsed.rotation_deg.is_some() {
                                    rotation_deg = parsed.rotation_deg;
                                }
                                if let Some(alignment) = parsed.text_vertical_alignment {
                                    vertical_alignment = alignment;
                                }
                                dist_top = parsed.dist_top.or(dist_top);
                                dist_bottom = parsed.dist_bottom.or(dist_bottom);
                                dist_left = parsed.dist_left.or(dist_left);
                                dist_right = parsed.dist_right.or(dist_right);
                            }
                        }
                        b"w10:wrap" => {
                            wrap = Self::parse_vml_wrap_mode(e).unwrap_or(wrap);
                        }
                        b"v:textbox" => {
                            Self::populate_vml_text_box_insets(&mut insets, e);
                        }
                        b"v:stroke" => {
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value_trimmed = value.trim();
                                    match attr.key.as_ref() {
                                        b"on" => {
                                            if matches!(
                                                value_trimmed.to_ascii_lowercase().as_str(),
                                                "f" | "false" | "0"
                                            ) {
                                                stroke_enabled = false;
                                            }
                                        }
                                        b"weight" => {
                                            if let Some(px) = Self::points_str_to_px(value_trimmed)
                                            {
                                                border.width_px = Some(px);
                                            }
                                        }
                                        b"dashstyle" => {
                                            border.style = TextBoxBorderStyle::from_vml_dashstyle(
                                                value_trimmed,
                                            );
                                        }
                                        b"color" => {
                                            if !value_trimmed.is_empty() {
                                                border.color = Some(Self::normalize_color_string(
                                                    value_trimmed,
                                                ));
                                            }
                                        }
                                        b"linestyle" => {
                                            if value_trimmed.eq_ignore_ascii_case("none") {
                                                border.style = TextBoxBorderStyle::None;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"v:fill" => {
                            for attr in e.attributes().flatten() {
                                if let Ok(value) = attr.unescape_value() {
                                    let value_trimmed = value.trim();
                                    match attr.key.as_ref() {
                                        b"on" => {
                                            let val = value_trimmed.to_ascii_lowercase();
                                            if val == "f" || val == "false" || val == "0" {
                                                fill_enabled = false;
                                                fill_color = None;
                                            }
                                        }
                                        b"color" => {
                                            if !value_trimmed.is_empty() {
                                                fill_color = Some(Self::normalize_color_string(
                                                    value_trimmed,
                                                ));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && e.name().as_ref() == b"w:pict" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        if blocks.is_none() && text.is_none() {
            if image.relationship_id.is_none() {
                return Ok(None);
            }
            image.width_px = width_px;
            image.height_px = height_px;
            image.layout = RunImageLayout {
                kind: if is_absolute {
                    RunImageKind::Floating
                } else {
                    RunImageKind::Inline
                },
                position_h,
                position_v,
                rotation_deg,
                wrap,
                dist_top,
                dist_bottom,
                dist_left,
                dist_right,
                ..RunImageLayout::default()
            };
            return Ok(Some(DrawingObject::Image(image)));
        }

        if !stroke_enabled {
            // `stroked="f"` explicitly disables the border, regardless of any prior settings.
            border.style = TextBoxBorderStyle::None;
        }
        if !fill_enabled {
            fill_color = None;
        }

        // A VML shape without `position:absolute` flows inline, as a VML
        // picture does above: `table-object-edge-cases` and
        // `word-compat-comprehensive` set their text boxes in the line.
        let layout = RunImageLayout {
            kind: if is_absolute {
                RunImageKind::Floating
            } else {
                RunImageKind::Inline
            },
            position_h,
            position_v,
            rotation_deg,
            wrap,
            dist_top,
            dist_bottom,
            dist_left,
            dist_right,
            ..RunImageLayout::default()
        };

        Ok(Some(DrawingObject::TextBox(RunTextBox {
            width_px,
            height_px,
            text: text.unwrap_or_default(),
            layout,
            blocks: blocks.unwrap_or_default(),
            border,
            shadow: None,
            inner_shadow: None,
            glow: None,
            blur_radius_px: None,
            soft_edge_radius_px: None,
            fill_color,
            fill_opacity: None,
            fill_theme_color: None,
            fill_theme_transforms: Vec::new(),
            insets,
            vertical_alignment,
            shape: TextBoxShape::Rectangle,
        })))
    }

    fn parse_vml_ole_object(
        &self,
        reader: &mut dyn XmlEventSource,
        width_hint_px: Option<f64>,
        height_hint_px: Option<f64>,
    ) -> Result<Option<RunImage>, Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        let mut depth = 1usize;
        let mut image = RunImage::default();
        image.layout.kind = RunImageKind::Inline;
        image.width_px = width_hint_px;
        image.height_px = height_hint_px;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    Self::apply_vml_shape_style_to_image(&mut image, e);
                    Self::apply_vml_imagedata_attributes(&mut image, e);
                    if e.name().as_ref() == b"o:OLEObject" {
                        Self::apply_ole_object_attributes(&mut image, e);
                    }
                    depth = depth.saturating_add(1);
                }
                Ok(Event::Empty(ref e)) => {
                    Self::apply_vml_shape_style_to_image(&mut image, e);
                    Self::apply_vml_imagedata_attributes(&mut image, e);
                    if e.name().as_ref() == b"o:OLEObject" {
                        Self::apply_ole_object_attributes(&mut image, e);
                    }
                }
                Ok(Event::End(_)) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        if image.relationship_id.is_none() {
            return Ok(None);
        }

        if image.description.is_none() {
            image.description = Some("Embedded OLE object".to_string());
        }
        if image.placeholder_label.is_none() {
            image.placeholder_label = image
                .description
                .clone()
                .map(|desc| {
                    let trimmed = desc.trim();
                    if trimmed.is_empty() {
                        "OLE Object".to_string()
                    } else {
                        trimmed.to_string()
                    }
                })
                .or_else(|| Some("OLE Object".to_string()));
        }

        Ok(Some(image))
    }

    fn apply_ole_object_attributes(image: &mut RunImage, element: &BytesStart<'_>) {
        for attribute in element.attributes().flatten() {
            let value = String::from_utf8_lossy(&attribute.value).into_owned();
            match attribute.key.as_ref() {
                b"ProgID" | b"o:ProgID" => {
                    image.description = Some(value.clone());
                    image.ole_program_id = Some(value);
                }
                b"r:id" | b"id" => image.ole_relationship_id = Some(value),
                _ => {}
            }
        }
    }

    fn apply_vml_shape_style_to_image(image: &mut RunImage, element: &BytesStart<'_>) {
        let mut style_value: Option<String> = None;
        for attr in element.attributes().flatten() {
            if attr.key.as_ref() == b"style" {
                style_value = Some(String::from_utf8_lossy(&attr.value).into_owned());
            }
        }

        if let Some(style_str) = style_value {
            let parsed = Self::parse_vml_style(&style_str);
            if parsed.is_absolute {
                image.layout.kind = RunImageKind::Floating;
            }
            if image.width_px.is_none() {
                image.width_px = parsed.width_px;
            }
            if image.height_px.is_none() {
                image.height_px = parsed.height_px;
            }
            if image.layout.position_h.is_none() {
                image.layout.position_h = parsed.position_h;
            }
            if image.layout.position_v.is_none() {
                image.layout.position_v = parsed.position_v;
            }
            if image.layout.rotation_deg.is_none() {
                image.layout.rotation_deg = parsed.rotation_deg;
            }
        }
    }

    fn apply_vml_imagedata_attributes(image: &mut RunImage, element: &BytesStart<'_>) {
        if element.name().as_ref() != b"v:imagedata" {
            return;
        }

        for attr in element.attributes().flatten() {
            match attr.key.as_ref() {
                b"r:id" => {
                    image.relationship_id = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"o:title" | b"title" => {
                    let value = String::from_utf8_lossy(&attr.value).trim().to_string();
                    if !value.is_empty() {
                        image.title = Some(value.clone());
                        image.description.get_or_insert_with(|| value.to_string());
                    }
                }
                b"alt" => {
                    let value = String::from_utf8_lossy(&attr.value).trim().to_string();
                    if !value.is_empty() {
                        image.description = Some(value);
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_vml_style(style: &str) -> ParsedVmlStyle {
        let mut is_absolute = false;
        let mut width_px = None;
        let mut height_px = None;
        let mut offset_x = None;
        let mut offset_y = None;
        let mut relative_from_h = None;
        let mut relative_from_v = None;
        let mut rotation_deg = None;
        let mut text_vertical_alignment = None;
        let mut dist_top = None;
        let mut dist_bottom = None;
        let mut dist_left = None;
        let mut dist_right = None;

        for part in style.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(rest) = part.strip_prefix("position:") {
                is_absolute = rest.trim().eq_ignore_ascii_case("absolute");
            } else if let Some(rest) = part.strip_prefix("width:") {
                width_px = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("height:") {
                height_px = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("margin-left:") {
                offset_x = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("margin-top:") {
                offset_y = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("mso-position-horizontal-relative:") {
                relative_from_h = Some(rest.trim().to_string());
            } else if let Some(rest) = part.strip_prefix("mso-position-vertical-relative:") {
                relative_from_v = Some(rest.trim().to_string());
            } else if let Some(rest) = part.strip_prefix("rotation:") {
                rotation_deg = rest.trim().parse::<f64>().ok();
            } else if let Some(rest) = part.strip_prefix("v-text-anchor:") {
                text_vertical_alignment = Some(match rest.trim().to_ascii_lowercase().as_str() {
                    "middle" | "center" => TextBoxVerticalAlignment::Center,
                    "bottom" => TextBoxVerticalAlignment::Bottom,
                    _ => TextBoxVerticalAlignment::Top,
                });
            } else if let Some(rest) = part.strip_prefix("mso-wrap-distance-top:") {
                dist_top = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("mso-wrap-distance-bottom:") {
                dist_bottom = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("mso-wrap-distance-left:") {
                dist_left = Self::points_str_to_px(rest.trim());
            } else if let Some(rest) = part.strip_prefix("mso-wrap-distance-right:") {
                dist_right = Self::points_str_to_px(rest.trim());
            }
        }

        ParsedVmlStyle {
            is_absolute,
            width_px,
            height_px,
            position_h: Self::build_position(relative_from_h, offset_x, true),
            position_v: Self::build_position(relative_from_v, offset_y, false),
            rotation_deg,
            text_vertical_alignment,
            dist_top,
            dist_bottom,
            dist_left,
            dist_right,
        }
    }

    fn parse_vml_wrap_mode(element: &BytesStart<'_>) -> Option<WrapMode> {
        let value = element
            .attributes()
            .flatten()
            .find(|attribute| attribute.key.as_ref() == b"type")?
            .unescape_value()
            .ok()?;
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "topandbottom" => WrapMode::TopAndBottom,
            "square" => WrapMode::Square,
            "tight" => WrapMode::Tight,
            "through" => WrapMode::Through,
            "none" => WrapMode::None,
            _ => return None,
        })
    }

    fn build_position(
        relative: Option<String>,
        offset: Option<f64>,
        horizontal: bool,
    ) -> Option<ImagePosition> {
        if relative.is_none() && offset.is_none() {
            return None;
        }

        let mut position = ImagePosition::default();
        if let Some(relative) = relative {
            position.relative_from = Some(match relative.as_str() {
                "margin" => "margin".to_string(),
                "page" => "page".to_string(),
                "leftMargin" => {
                    if horizontal {
                        "leftMargin".to_string()
                    } else {
                        "margin".to_string()
                    }
                }
                "rightMargin" => {
                    if horizontal {
                        "rightMargin".to_string()
                    } else {
                        "margin".to_string()
                    }
                }
                "line" => "line".to_string(),
                "paragraph" => "paragraph".to_string(),
                _ => relative,
            });
        }
        position.offset_px = offset;
        Some(position)
    }

    /// Normalises colour strings from DrawingML / VML into canonical `#RRGGBB`
    /// representation where possible.
    fn normalize_color_string(raw: &str) -> String {
        let trimmed = raw.trim().trim_start_matches('#');
        if trimmed.len() == 6 {
            format!("#{}", trimmed.to_uppercase())
        } else {
            raw.trim().to_string()
        }
    }

    fn points_str_to_px(value: &str) -> Option<f64> {
        let trimmed = value.trim_end_matches(';').trim();
        if let Some(points) = trimmed.strip_suffix("pt") {
            if let Ok(val) = points.trim().parse::<f64>() {
                return Some(val * 96.0 / 72.0);
            }
        }
        None
    }

    /// Populates `RunImage` fields for width, height, relationship id, and
    /// accessible text extracted from drawing XML.
    fn extract_drawing_attributes(
        image: &mut RunImage,
        element: &BytesStart<'_>,
        has_embed: &mut bool,
    ) {
        match element.name().as_ref() {
            b"wp:extent" | b"a:ext" => {
                let is_outer_extent = element.name().as_ref() == b"wp:extent";
                for attr in element.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"cx" => {
                            if is_outer_extent || image.width_px.is_none() {
                                if let Some(val) = Self::parse_f64_attr(&attr) {
                                    image.width_px = Some(Self::emu_to_px(val));
                                }
                            }
                        }
                        b"cy" => {
                            if is_outer_extent || image.height_px.is_none() {
                                if let Some(val) = Self::parse_f64_attr(&attr) {
                                    image.height_px = Some(Self::emu_to_px(val));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"a:xfrm" => {
                for attr in element.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"rot" => {
                            if let Some(val) = Self::parse_f64_attr(&attr) {
                                image.layout.rotation_deg = Some(val / 60_000.0);
                            }
                        }
                        b"flipH" => {
                            image.layout.flip_horizontal = Self::drawingml_bool_attr(&attr);
                        }
                        b"flipV" => {
                            image.layout.flip_vertical = Self::drawingml_bool_attr(&attr);
                        }
                        _ => {}
                    }
                }
            }
            b"a:blip" => {
                for attr in element.attributes().flatten() {
                    if attr.key.as_ref() == b"r:embed" {
                        image.relationship_id =
                            Some(String::from_utf8_lossy(&attr.value).to_string());
                        *has_embed = true;
                    }
                }
            }
            b"a:srcRect" => {
                let mut crop = ImageCrop::default();
                let mut has_crop = false;
                for attr in element.attributes().flatten() {
                    let raw = String::from_utf8_lossy(&attr.value);
                    let raw = raw.trim();
                    // ST_Percentage is normally serialized as an integer in
                    // 1/1000 percent (15000 = 15%). Some producers use the
                    // equivalent lexical percentage form, so accept both.
                    let fraction = raw
                        .strip_suffix('%')
                        .and_then(|value| value.parse::<f64>().ok())
                        .map(|value| value / 100.0)
                        .or_else(|| raw.parse::<f64>().ok().map(|value| value / 100_000.0));
                    let Some(fraction) = fraction else {
                        continue;
                    };
                    match attr.key.as_ref() {
                        b"l" => crop.left = fraction,
                        b"t" => crop.top = fraction,
                        b"r" => crop.right = fraction,
                        b"b" => crop.bottom = fraction,
                        _ => continue,
                    }
                    has_crop = true;
                }
                if has_crop {
                    image.crop = Some(crop);
                }
            }
            b"wp:docPr" => {
                let mut ink_name = None;
                let mut explicit_title = None;
                for attr in element.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"name" => {
                            let name = attr
                                .unescape_value()
                                .map(|value| value.into_owned())
                                .unwrap_or_else(|_| {
                                    String::from_utf8_lossy(&attr.value).into_owned()
                                });
                            // GenOffice identifies editable ink by the
                            // `wp:docPr@name` prefix and stores its vector
                            // payload in `descr`. Preserve that marker as the
                            // modeled title so Jade can round-trip and edit it.
                            if name.starts_with("aidocs-ink") {
                                ink_name = Some(name);
                            }
                        }
                        b"descr" => {
                            image.description = Some(
                                attr.unescape_value()
                                    .map(|value| value.into_owned())
                                    .unwrap_or_else(|_| {
                                        String::from_utf8_lossy(&attr.value).into_owned()
                                    }),
                            );
                        }
                        b"title" => {
                            explicit_title = Some(
                                attr.unescape_value()
                                    .map(|value| value.into_owned())
                                    .unwrap_or_else(|_| {
                                        String::from_utf8_lossy(&attr.value).into_owned()
                                    }),
                            );
                        }
                        _ => {}
                    }
                }
                if let Some(title) = ink_name.or(explicit_title) {
                    image.title = Some(title);
                }
            }
            _ => {}
        }
    }

    /// Reads the `distT`/`distB`/`distL`/`distR` (EMU) attributes from a
    /// `wp:anchor` or wrap element and stores them as CSS pixels on the layout.
    /// Only attributes that are present overwrite the existing values, so wrap
    /// elements can override the anchor-level defaults.
    fn populate_wrap_distances(layout: &mut RunImageLayout, element: &BytesStart<'_>) {
        for attr in element.attributes().flatten() {
            let Some(val) = Self::parse_f64_attr(&attr) else {
                continue;
            };
            let px = Self::emu_to_px(val);
            match attr.key.as_ref() {
                b"distT" => layout.dist_top = Some(px),
                b"distB" => layout.dist_bottom = Some(px),
                b"distL" => layout.dist_left = Some(px),
                b"distR" => layout.dist_right = Some(px),
                _ => {}
            }
        }
    }

    /// Maps a `wp:wrap*` element name to a [`WrapMode`] and stores it on the
    /// layout, also applying any wrap-local `dist*` overrides.
    fn apply_wrap_element(layout: &mut RunImageLayout, name: &[u8], element: &BytesStart<'_>) {
        layout.wrap = match name {
            b"wp:wrapNone" => WrapMode::None,
            b"wp:wrapSquare" => WrapMode::Square,
            b"wp:wrapTight" => WrapMode::Tight,
            b"wp:wrapThrough" => WrapMode::Through,
            b"wp:wrapTopAndBottom" => WrapMode::TopAndBottom,
            _ => return,
        };
        // Square/Tight/Through may carry their own dist* attributes that take
        // precedence over the anchor-level defaults.
        Self::populate_wrap_distances(layout, element);
        if matches!(
            layout.wrap,
            WrapMode::Square | WrapMode::Tight | WrapMode::Through
        ) {
            for attr in element.attributes().flatten() {
                if attr.key.as_ref() == b"wrapText" {
                    layout.wrap_text =
                        WrapTextSide::from_ooxml(&String::from_utf8_lossy(&attr.value));
                }
            }
        }
    }

    fn wrap_polygon_from_element(element: &BytesStart<'_>) -> WrapPolygon {
        let edited = element.attributes().flatten().any(|attribute| {
            attribute.key.as_ref() == b"edited"
                && parse_on_off(
                    Some(String::from_utf8_lossy(&attribute.value).to_string()),
                    false,
                )
        });
        WrapPolygon {
            edited,
            points: Vec::new(),
        }
    }

    fn push_wrap_polygon_point(layout: &mut RunImageLayout, element: &BytesStart<'_>) {
        let mut x = None;
        let mut y = None;
        for attribute in element.attributes().flatten() {
            let value = String::from_utf8_lossy(&attribute.value);
            match attribute.key.as_ref() {
                b"x" => x = value.trim().parse::<i64>().ok(),
                b"y" => y = value.trim().parse::<i64>().ok(),
                _ => {}
            }
        }
        if let (Some(x), Some(y), Some(polygon)) = (x, y, layout.wrap_polygon.as_mut()) {
            polygon.points.push(WrapPolygonPoint { x, y });
        }
    }

    /// Extracts the `relativeFrom` attribute from positioning nodes and stores
    /// it in the [`ImagePosition`].
    fn populate_relative_from(position: &mut ImagePosition, element: &BytesStart<'_>) {
        for attr in element.attributes().flatten() {
            if attr.key.as_ref() == b"relativeFrom" {
                position.relative_from = Some(String::from_utf8_lossy(&attr.value).to_string());
            }
        }
    }

    fn parse_f64_attr(attr: &quick_xml::events::attributes::Attribute<'_>) -> Option<f64> {
        String::from_utf8_lossy(&attr.value).parse::<f64>().ok()
    }

    fn drawingml_bool_attr(attr: &quick_xml::events::attributes::Attribute<'_>) -> bool {
        matches!(
            String::from_utf8_lossy(&attr.value)
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "on" | "t"
        )
    }

    fn populate_simple_position(layout: &mut RunImageLayout, element: &BytesStart<'_>) {
        let Some(mut position) = layout.simple_position else {
            // The child is mandatory in the anchor schema even when the
            // `simplePos` switch is off. In that case positionH/V stay active.
            return;
        };
        for attribute in element.attributes().flatten() {
            let Some(raw) = Self::parse_f64_attr(&attribute) else {
                continue;
            };
            match attribute.key.as_ref() {
                b"x" => position.x_px = Self::emu_to_px(raw),
                b"y" => position.y_px = Self::emu_to_px(raw),
                _ => {}
            }
        }
        layout.simple_position = Some(position);
    }

    fn emu_to_px(value: f64) -> f64 {
        value / 914_400.0 * 96.0
    }

    fn dxa_to_px(value: f64) -> f64 {
        let points = value / 20.0;
        points * 96.0 / 72.0
    }

    fn normalize_paragraph_alignment(value: &str) -> String {
        match value {
            "start" | "left" => "left".to_string(),
            "center" => "center".to_string(),
            "end" | "right" => "right".to_string(),
            "both" => "justify".to_string(),
            _ => value.to_string(),
        }
    }

    fn apply_to_active_run_props<F>(
        parsing_paragraph_run_props: bool,
        paragraph_run_props: &mut Option<RunProperties>,
        current_run_props: &mut Option<RunProperties>,
        mut f: F,
    ) where
        F: FnMut(&mut RunProperties),
    {
        if parsing_paragraph_run_props {
            if let Some(props) = paragraph_run_props.as_mut() {
                f(props);
            }
        } else if let Some(props) = current_run_props.as_mut() {
            f(props);
        }
    }

    fn apply_paragraph_alignment(
        attributes: Attributes<'_>,
        props: Option<&mut ParagraphProperties>,
    ) {
        if let Some(props) = props {
            for attr in attributes.flatten() {
                if attr.key.as_ref() == b"w:val" {
                    let alignment_raw = String::from_utf8_lossy(&attr.value).to_string();
                    // Logical paragraph alignment depends on w:bidi, which may
                    // appear later in pPr. Preserve it for the layout resolver.
                    props.alignment = Some(match alignment_raw.as_str() {
                        "start" | "end" => alignment_raw.into(),
                        _ => Self::normalize_paragraph_alignment(&alignment_raw).into(),
                    });
                }
            }
        }
    }

    fn apply_paragraph_indent(attributes: Attributes<'_>, props: Option<&mut ParagraphProperties>) {
        if let Some(props) = props {
            let mut indent = props.indent.clone().unwrap_or_default();
            let mut updated = false;

            for attr in attributes.flatten() {
                match attr.key.as_ref() {
                    b"w:left" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.left = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:right" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.right = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:start" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.start = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:end" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.end = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:firstLine" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.first_line = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:hanging" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            indent.hanging = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:leftChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.left_chars = Some(val);
                            updated = true;
                        }
                    }
                    b"w:rightChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.right_chars = Some(val);
                            updated = true;
                        }
                    }
                    b"w:startChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.start_chars = Some(val);
                            updated = true;
                        }
                    }
                    b"w:endChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.end_chars = Some(val);
                            updated = true;
                        }
                    }
                    b"w:firstLineChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.first_line_chars = Some(val);
                            updated = true;
                        }
                    }
                    b"w:hangingChars" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                            indent.hanging_chars = Some(val);
                            updated = true;
                        }
                    }
                    _ => {}
                }
            }

            if updated {
                props.indent = Some(indent);
            }
        }
    }

    fn apply_paragraph_spacing(
        attributes: Attributes<'_>,
        props: Option<&mut ParagraphProperties>,
    ) {
        if let Some(props) = props {
            let mut spacing = props.spacing.clone().unwrap_or_default();
            let mut updated = false;
            let mut line_value: Option<f32> = None;
            let mut line_rule_value: Option<String> = None;
            let (mut before_auto, mut after_auto) = (false, false);

            for attr in attributes.flatten() {
                match attr.key.as_ref() {
                    b"w:before" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            spacing.before = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:after" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            spacing.after = Some(val / 20.0);
                            updated = true;
                        }
                    }
                    b"w:beforeLines" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            spacing.before_lines = Some(val / 100.0);
                            updated = true;
                        }
                    }
                    b"w:afterLines" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            spacing.after_lines = Some(val / 100.0);
                            updated = true;
                        }
                    }
                    b"w:line" => {
                        if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                            line_value = Some(val);
                        }
                    }
                    b"w:lineRule" => {
                        let rule = String::from_utf8_lossy(&attr.value).to_string();
                        line_rule_value = Some(rule);
                    }
                    b"w:beforeAutospacing" => {
                        before_auto = crate::document::ooxml_on(&attr.value);
                    }
                    b"w:afterAutospacing" => {
                        after_auto = crate::document::ooxml_on(&attr.value);
                    }
                    _ => {}
                }
            }
            if before_auto || after_auto {
                spacing.apply_autospacing(before_auto, after_auto);
                updated = true;
            }

            if let Some(rule) = line_rule_value {
                let normalized = match rule.as_str() {
                    "exactly" => "exact".to_string(),
                    other => other.to_string(),
                };
                spacing.line_rule = Some(normalized);
                updated = true;
            }

            if let Some(line_raw) = line_value {
                let rule = spacing.line_rule.as_deref().unwrap_or("auto");
                let converted = match rule {
                    "atLeast" => line_raw / 20.0,
                    "exact" => line_raw / 20.0,
                    "exactly" => line_raw / 20.0,
                    _ => line_raw / 240.0,
                };
                spacing.line = Some(converted);
                if spacing.line_rule.is_none() {
                    spacing.line_rule = Some("auto".to_string());
                }
                updated = true;
            }

            if updated {
                props.spacing = Some(spacing);
            }
        }
    }

    /// Parses a `w:shd` element's attributes into a [`Shading`].
    ///
    /// ECMA-376 Part 1 §17.3.5 (<https://ooxml.info/docs/11/Part1/17.3.5>).
    /// `w:val` is the pattern (stored lowercased), `w:color` the pattern
    /// foreground, and `w:fill` the background fill. Colors are normalized to
    /// `#RRGGBB`; the `auto` sentinel becomes `None`.
    pub(crate) fn parse_shading(attributes: Attributes<'_>) -> Shading {
        let mut shading = Shading::default();
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:val" => {
                    // Preserve the canonical OOXML token casing (e.g.
                    // `horzStripe`, `pct25`) for round-trip fidelity; renderers
                    // should compare case-insensitively.
                    let raw = String::from_utf8_lossy(&attr.value).trim().to_string();
                    if !raw.is_empty() {
                        shading.val = Some(raw);
                    }
                }
                b"w:color" => {
                    shading.color = Self::normalize_shading_color(&attr.value);
                }
                b"w:fill" => {
                    shading.fill = Self::normalize_shading_color(&attr.value);
                }
                _ => {}
            }
        }
        shading
    }

    /// Normalizes an OOXML hex color (`RRGGBB` or `#RRGGBB`) to `#RRGGBB`.
    /// Returns `None` for the `auto` sentinel or empty values.
    fn normalize_shading_color(raw: &[u8]) -> Option<String> {
        let value = String::from_utf8_lossy(raw);
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("auto") {
            return None;
        }
        if let Some(stripped) = value.strip_prefix('#') {
            return Some(format!("#{}", stripped.to_ascii_uppercase()));
        }
        Some(format!("#{}", value.to_ascii_uppercase()))
    }

    /// Reads the `w:color` of a `<w:background>` element (a `<w:document>`
    /// child, ECMA-376 Part 1 §17.2.1) and normalizes it to `#RRGGBB`. Returns
    /// `None` for `auto` / missing / empty so the page keeps its default.
    /// Reads the `w14:paraId` attribute (matched by local name, ignoring the
    /// namespace prefix) off a `<w:p>` element, if present and non-empty.
    fn parse_para_id(attributes: Attributes<'_>) -> Option<String> {
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            let local = key.rsplit(|&b| b == b':').next().unwrap_or(key);
            if local == b"paraId" {
                let value = String::from_utf8_lossy(&attr.value).trim().to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        None
    }

    fn parse_document_background(attributes: Attributes<'_>) -> Option<String> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:color" {
                return Self::normalize_shading_color(&attr.value);
            }
        }
        None
    }

    fn apply_paragraph_shading(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let shading = Self::parse_shading(attributes);
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.shading = Some(shading);
    }

    fn apply_paragraph_snap_to_grid(
        attributes: Attributes<'_>,
        props: Option<&mut ParagraphProperties>,
    ) {
        if let Some(props) = props {
            let mut value = true;
            for attr in attributes.flatten() {
                if attr.key.as_ref() == b"w:val" {
                    let raw = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .to_ascii_lowercase();
                    value = !matches!(raw.as_str(), "0" | "false");
                    break;
                }
            }
            props.snap_to_grid = Some(value);
        }
    }

    fn apply_paragraph_adjust_right_indent(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        props
            .get_or_insert_with(ParagraphProperties::default)
            .adjust_right_indent = Some(value);
    }

    fn apply_paragraph_widow_control(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.widow_control = Some(value);
    }

    /// Parses `w:suppressAutoHyphens` (ECMA-376 Part 1 §17.3.1.34). The element
    /// is an on/off toggle: a bare `<w:suppressAutoHyphens/>` means on, while an
    /// explicit `w:val` of `0`/`false`/`off` disables it.
    fn apply_paragraph_suppress_auto_hyphens(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.suppress_auto_hyphens = Some(value);
    }

    /// Applies `w:contextualSpacing` (ECMA-376 Part 1 §17.3.1.9). The toggle
    /// defaults to `true` when the element is present without a `w:val`.
    fn apply_paragraph_contextual_spacing(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.contextual_spacing = Some(value);
    }

    /// Parses `w:pageBreakBefore` (ECMA-376 Part 1 §17.3.1.23). The element is a
    /// toggle: present (or `w:val="true"`) means start the paragraph on a new
    /// page; `w:val` `0`/`false`/`off` disables it.
    fn apply_paragraph_page_break_before(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.page_break_before = Some(value);
    }

    /// Parses `w:framePr` (ECMA-376 Part 1 §17.3.1.11): a drop cap, or else
    /// a positioned text frame. The element itself stays in the raw XML so
    /// it round-trips unchanged. Missing/invalid line counts use Word's three-line default;
    /// imported values are bounded to the range exposed by Word's UI so a
    /// malformed document cannot request an unbounded glyph size.
    fn apply_paragraph_frame_properties(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut kind = None;
        let mut lines = 3_u8;
        let mut frame_attributes: Vec<(String, String)> = Vec::new();
        for attr in attributes.flatten() {
            let key = attr.key.as_ref();
            if let Some(local) = key.strip_prefix(b"w:") {
                frame_attributes.push((
                    String::from_utf8_lossy(local).into_owned(),
                    String::from_utf8_lossy(&attr.value).into_owned(),
                ));
            }
            match key {
                b"w:dropCap" => {
                    let value = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .to_ascii_lowercase();
                    kind = match value.as_str() {
                        "drop" => Some(crate::document::DropCapKind::Drop),
                        "margin" => Some(crate::document::DropCapKind::Margin),
                        _ => None,
                    };
                }
                b"w:lines" => {
                    lines = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .map(|value| value.clamp(1, 10) as u8)
                        .unwrap_or(3);
                }
                _ => {}
            }
        }
        let Some(kind) = kind else {
            let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
            paragraph_props.frame = crate::document::ParagraphFrame::from_attributes(|name| {
                frame_attributes
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str())
            });
            return;
        };
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.drop_cap = Some(crate::document::DropCap { kind, lines });
    }

    /// Parse `w:suppressLineNumbers` (ECMA-376 Part 1 §17.3.1.35). The element
    /// is an on/off toggle: a bare `<w:suppressLineNumbers/>` means on, while an
    /// explicit `w:val="0"`/`false`/`off` disables it.
    fn apply_paragraph_suppress_line_numbers(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.suppress_line_numbers = Some(value);
    }

    /// `w:keepLines` (ECMA-376 Part 1 §17.3.1.15): keep all lines of the
    /// paragraph together on a single page. A bare `<w:keepLines/>` means on;
    /// `w:val="false"`/`0`/`off` turns it off.
    fn apply_paragraph_keep_lines(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.keep_lines = Some(value);
    }

    /// `w:keepNext` (ECMA-376 Part 1 §17.3.1.16): keep the paragraph on the same
    /// page as the following paragraph. A bare `<w:keepNext/>` means on;
    /// `w:val="false"`/`0`/`off` turns it off.
    fn apply_paragraph_keep_next(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.keep_next = Some(value);
    }

    /// Parse `w:textAlignment` (§17.3.1.39) capturing the `w:val` token as a
    /// normalized lowercase string. An empty / missing value is ignored.
    fn apply_paragraph_text_alignment(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut token: Option<String> = None;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                if !raw.is_empty() {
                    token = Some(raw);
                }
                break;
            }
        }
        if let Some(token) = token {
            let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
            paragraph_props.text_alignment = Some(token.into());
        }
    }

    /// Parses `w:mirrorIndents` (ECMA-376 Part 1 §17.3.1.18). The element is a
    /// boolean toggle: a bare element or `w:val` of anything other than the
    /// off tokens enables mirroring.
    fn apply_paragraph_mirror_indents(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        paragraph_props.mirror_indents = Some(value);
    }

    /// Parses `w:outlineLvl` (ECMA-376 Part 1 §17.3.1.20). The `w:val` is the
    /// 0-based heading level; values are clamped to `0..=9` (9 = body text).
    /// A missing or non-numeric `w:val` leaves the property unset.
    fn apply_paragraph_outline_level(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
    ) {
        let mut level: Option<u8> = None;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value);
                if let Ok(parsed) = raw.trim().parse::<i64>() {
                    level = Some(parsed.clamp(0, 9) as u8);
                }
                break;
            }
        }
        if let Some(level) = level {
            let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
            paragraph_props.outline_level = Some(level);
            // This parser only visits `w:pPr` attached to the paragraph. Style
            // definitions are parsed separately by `StylesPart`, so retaining
            // the origin here lets TOC generation implement Word's `\u`
            // distinction after style defaults have been materialized.
            paragraph_props.outline_level_is_direct = true;
        }
    }

    fn apply_paragraph_auto_spacing(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
        is_numeric: bool,
    ) {
        let mut value = true;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                value = !matches!(raw.as_str(), "0" | "false" | "off");
                break;
            }
        }

        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        let east_asian = paragraph_props
            .east_asian_settings
            .get_or_insert_with(EastAsianLineBreakingSettings::default);
        east_asian.use_east_asian_layout = true;
        if is_numeric {
            east_asian.auto_space_dn = Some(value);
        } else {
            east_asian.auto_space_de = Some(value);
        }
    }

    fn parse_on_off_attr(attributes: Attributes<'_>) -> bool {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let raw = String::from_utf8_lossy(&attr.value)
                    .trim()
                    .to_ascii_lowercase();
                return !matches!(raw.as_str(), "0" | "false" | "off");
            }
        }
        true
    }

    fn apply_paragraph_line_break_option(
        attributes: Attributes<'_>,
        props: &mut Option<ParagraphProperties>,
        option_name: &str,
    ) {
        let value = Self::parse_on_off_attr(attributes);
        let paragraph_props = props.get_or_insert_with(ParagraphProperties::default);
        let east_asian = paragraph_props
            .east_asian_settings
            .get_or_insert_with(EastAsianLineBreakingSettings::default);
        east_asian.use_east_asian_layout = true;
        match option_name {
            "kinsoku" => east_asian.kinsoku = Some(value),
            "wordWrap" => east_asian.word_wrap = Some(value),
            "overflowPunct" => east_asian.overflow_punctuation = Some(value),
            "topLinePunct" => east_asian.top_line_punctuation = Some(value),
            _ => {}
        }
    }

    fn parse_paragraph_tabs(
        reader: &mut dyn XmlEventSource,
    ) -> Result<Vec<TabStop>, Box<dyn std::error::Error>> {
        let mut tabs = Vec::new();
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:tabs" => break,
                Ok(Event::Empty(ref e)) if e.name().as_ref() == b"w:tab" => {
                    if let Some(tab) = Self::build_tab_stop(e.attributes()) {
                        tabs.push(tab);
                    }
                }
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tab" => {
                    if let Some(tab) = Self::build_tab_stop(e.attributes()) {
                        tabs.push(tab);
                    }
                    Self::consume_element(reader, b"w:tab")?;
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok(tabs)
    }

    /// Parses a `w:pBdr` subtree into [`ParagraphBorders`]. Each child edge
    /// (`w:top` / `w:bottom` / `w:left` / `w:right` / `w:between` / `w:bar`) is
    /// a self-closing element carrying `w:val` / `w:sz` / `w:space` / `w:color`.
    /// See ECMA-376 Part 1 §17.3.1.24.
    fn parse_paragraph_borders(
        reader: &mut dyn XmlEventSource,
    ) -> Result<crate::document::ParagraphBorders, Box<dyn std::error::Error>> {
        use crate::document::ParagraphBorders;
        let mut borders = ParagraphBorders::default();
        let mut buf = Vec::new();

        loop {
            let assign = |borders: &mut ParagraphBorders, name: &[u8], attrs: Attributes<'_>| {
                let edge = Self::build_border_edge(attrs);
                match name {
                    b"w:top" => borders.top = Some(edge),
                    b"w:bottom" => borders.bottom = Some(edge),
                    b"w:left" => borders.left = Some(edge),
                    b"w:right" => borders.right = Some(edge),
                    b"w:between" => borders.between = Some(edge),
                    b"w:bar" => borders.bar = Some(edge),
                    _ => {}
                }
            };
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:pBdr" => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(&mut borders, name.as_ref(), e.attributes());
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&mut borders, &name, e.attributes());
                    Self::consume_element(reader, &name)?;
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok(borders)
    }

    /// Parses a `w:pgBorders` subtree into [`PageBorders`]. Only the four sides
    /// (`w:top`/`w:bottom`/`w:left`/`w:right`) are captured; the caller fills in
    /// `offset_from` from the container attributes. ECMA-376 Part 1 §17.6.10.
    fn parse_page_borders(
        reader: &mut dyn XmlEventSource,
    ) -> Result<crate::document::PageBorders, Box<dyn std::error::Error>> {
        use crate::document::PageBorders;
        let mut borders = PageBorders::default();
        let mut buf = Vec::new();

        loop {
            let assign = |borders: &mut PageBorders, name: &[u8], attrs: Attributes<'_>| {
                let edge = Self::build_border_edge(attrs);
                match name {
                    b"w:top" => borders.top = Some(edge),
                    b"w:bottom" => borders.bottom = Some(edge),
                    b"w:left" => borders.left = Some(edge),
                    b"w:right" => borders.right = Some(edge),
                    _ => {}
                }
            };
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:pgBorders" => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(&mut borders, name.as_ref(), e.attributes());
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&mut borders, &name, e.attributes());
                    Self::consume_element(reader, &name)?;
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok(borders)
    }

    /// Reads a `w14` text effect's subtree: its first `w14:srgbClr` (as
    /// `#RRGGBB`), that colour's `w14:alpha` (as opacity, 0–1), and for
    /// `w14:stylisticSets` the first set's id. `w14:noFill` leaves no colour.
    #[allow(clippy::type_complexity)]
    fn parse_text_effect(
        reader: &mut dyn XmlEventSource,
        tag: &[u8],
    ) -> Result<(Option<String>, Option<f32>, Option<u8>), Box<dyn std::error::Error>> {
        let (mut color, mut alpha, mut set) = (None, None, None);
        let mut depth = 1usize;
        let mut buf = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf);
            match event {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let is_start = matches!(event, Ok(Event::Start(_)));
                    let value = |key: &[u8]| {
                        e.attributes()
                            .flatten()
                            .find(|attr| attr.key.as_ref() == key)
                            .map(|attr| String::from_utf8_lossy(&attr.value).trim().to_string())
                    };
                    match e.name().as_ref() {
                        b"w14:srgbClr" if color.is_none() => {
                            color = value(b"w14:val")
                                .map(|hex| format!("#{}", hex.to_ascii_uppercase()));
                        }
                        b"w14:alpha" if alpha.is_none() => {
                            alpha = value(b"w14:val")
                                .and_then(|val| val.parse::<f32>().ok())
                                .map(|transparency| {
                                    (1.0 - transparency / 100_000.0).clamp(0.0, 1.0)
                                });
                        }
                        b"w14:styleSet" if set.is_none() => {
                            set = value(b"w14:id").and_then(|id| id.parse::<u8>().ok());
                        }
                        name if name == tag && is_start => depth += 1,
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) if e.name().as_ref() == tag => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }
        Ok((color, alpha, set))
    }

    /// The theme colour a `w:color` names, if it names one.
    pub(crate) fn run_theme_color(attributes: Attributes<'_>) -> Option<RunThemeColor> {
        let mut theme: Option<RunThemeColor> = None;
        let (mut tint, mut shade) = (None, None);
        for attr in attributes.flatten() {
            let value = String::from_utf8_lossy(&attr.value).trim().to_string();
            match attr.key.as_ref() {
                b"w:themeColor" if !value.is_empty() => {
                    theme = Some(RunThemeColor {
                        name: Rc::from(value.as_str()),
                        ..Default::default()
                    })
                }
                b"w:themeTint" if !value.is_empty() => tint = Some(Rc::from(value.as_str())),
                b"w:themeShade" if !value.is_empty() => shade = Some(Rc::from(value.as_str())),
                _ => {}
            }
        }
        theme.map(|theme| RunThemeColor {
            tint,
            shade,
            ..theme
        })
    }

    /// Builds a single [`BorderEdge`] from an edge element's attributes. `w:val`
    /// is lowercased so renderers compare case-insensitively; `w:color` is
    /// normalized to `#RRGGBB` (`auto` → `None`) reusing the shading color
    /// normalizer.
    pub(crate) fn build_border_edge(attributes: Attributes<'_>) -> crate::document::BorderEdge {
        let mut edge = crate::document::BorderEdge::default();
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:val" => {
                    edge.val = Some(
                        String::from_utf8_lossy(&attr.value)
                            .trim()
                            .to_ascii_lowercase(),
                    );
                }
                b"w:sz" => {
                    edge.size_eighths = String::from_utf8_lossy(&attr.value).trim().parse().ok();
                }
                b"w:space" => {
                    edge.space = String::from_utf8_lossy(&attr.value).trim().parse().ok();
                }
                b"w:color" => {
                    edge.color = Self::normalize_shading_color(&attr.value);
                }
                _ => {}
            }
        }
        edge
    }

    fn build_tab_stop(attributes: Attributes<'_>) -> Option<TabStop> {
        let mut stop = TabStop::default();
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:val" => {
                    stop.kind = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"w:pos" => {
                    if let Ok(value) = String::from_utf8_lossy(&attr.value)
                        .to_string()
                        .parse::<f32>()
                    {
                        stop.position = Some(value / 20.0);
                    }
                }
                b"w:leader" => {
                    stop.leader = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                _ => {}
            }
        }
        if stop.kind.is_none() && stop.position.is_none() && stop.leader.is_none() {
            None
        } else {
            Some(stop)
        }
    }

    fn consume_element(
        reader: &mut dyn XmlEventSource,
        tag: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut depth = 1usize;
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == tag => depth += 1,
                Ok(Event::End(ref e)) if e.name().as_ref() == tag => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => return Err(Box::new(err)),
            }
            buf.clear();
        }

        Ok(())
    }

    /// Processes a `<w:headerReference>` / `<w:footerReference>` element. The
    /// `w:type` attribute distinguishes `default`, `first`, and `even`
    /// variants; all three are now preserved on the current section. Only the
    /// default variant is propagated upwards to the document-level reference
    /// (the legacy behavior used by renderers that ignore page-specific
    /// variants).
    fn process_header_footer_reference(
        attributes: Attributes<'_>,
        section: Option<&mut SectionProperties>,
        is_header: bool,
    ) -> Option<String> {
        let mut rel_id: Option<String> = None;
        let mut ref_type: Option<String> = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"r:id" => rel_id = Some(String::from_utf8_lossy(&attr.value).to_string()),
                b"w:type" => ref_type = Some(String::from_utf8_lossy(&attr.value).to_string()),
                _ => {}
            }
        }
        let normalized_type = ref_type.as_deref().unwrap_or("default");
        if let Some(section) = section {
            match (normalized_type, is_header) {
                ("first", true) => section.header_reference_first = rel_id.clone(),
                ("first", false) => section.footer_reference_first = rel_id.clone(),
                ("even", true) => section.header_reference_even = rel_id.clone(),
                ("even", false) => section.footer_reference_even = rel_id.clone(),
                (_, true) => section.header_reference = rel_id.clone(),
                (_, false) => section.footer_reference = rel_id.clone(),
            }
        }
        if normalized_type == "default" {
            rel_id
        } else {
            None
        }
    }

    /// Editable top-level field results use the same text/control-character
    /// path as ordinary runs. Computed fields and nested operands keep results
    /// in their cache instead of contributing to paragraph text offsets.
    fn keeps_field_result_in_run_text(kind: Option<&RunFieldKind>, is_operand: bool) -> bool {
        match kind {
            None => true,
            Some(kind) => !is_operand && kind.keeps_result_in_run_text(),
        }
    }

    fn process_fld_char_event(
        attributes: Attributes<'_>,
        active_fields: &mut Vec<ActiveField>,
        bookmark_text: &HashMap<String, String>,
        deferred_nested_fields: &mut Vec<DeferredNestedField>,
        toc_fields: &mut Vec<TocOptions>,
        toc_entry_fields: &mut Vec<TocEntryField>,
        paragraph: usize,
    ) -> Option<RunFieldKind> {
        let mut fld_char_type: Option<String> = None;
        let mut locked = false;
        let mut closed_instruction_only_field = None;
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:fldCharType" {
                fld_char_type = Some(String::from_utf8_lossy(&attr.value).to_string());
            } else if attr.key.as_ref() == b"w:fldLock" {
                locked = parse_on_off(
                    Some(String::from_utf8_lossy(&attr.value).to_string()),
                    false,
                );
            }
        }
        match fld_char_type.as_deref() {
            Some("begin") => {
                let is_instruction_operand = active_fields
                    .iter()
                    .rev()
                    .any(|field| field.instruction.is_some());
                active_fields.push(ActiveField {
                    kind: None,
                    locked,
                    source: FieldSource::Complex,
                    instruction: Some(String::new()),
                    is_instruction_operand,
                    operand_result: String::new(),
                    saved_instruction: Some(String::new()),
                    word_mac_body_result: None,
                    paragraph_run_start: None,
                    sequence_result_run: None,
                });
            }
            Some("separate") => {
                if let Some(pos) = active_fields.iter().rposition(|field| {
                    field.source == FieldSource::Complex && field.instruction.is_some()
                }) {
                    let instr = active_fields[pos]
                        .instruction
                        .take()
                        .expect("complex field instruction");
                    let saved_instr = active_fields[pos].saved_instruction.take();
                    active_fields[pos].word_mac_body_result = Self::word_mac_body_if_result(
                        &instr,
                        saved_instr.as_deref(),
                        bookmark_text,
                    );
                    if let Some(entry) = parse_toc_entry_field(&instr, paragraph) {
                        toc_entry_fields.push(entry);
                    }
                    if let Some(kind) = Self::parse_field_instruction(&instr) {
                        active_fields[pos].kind = Some(kind.with_lock(active_fields[pos].locked));
                    } else if field_instruction_is_toc(&instr) {
                        // `TOC` is not pushed as an active field: its result is
                        // generated body content (headings + PAGEREF page
                        // numbers) that must keep rendering. We only record the
                        // field's switches so a host can detect / regenerate it.
                        toc_fields.push(parse_toc_switches(&instr));
                    }
                }
            }
            Some("end") => {
                // A producer may omit `separate` and the cached-result region;
                // TC does this routinely, but dirty/unupdated visible fields
                // can have the same instruction-only shape. Parse at `end`
                // when the instruction was not consumed above.
                if let Some(pos) = active_fields
                    .iter()
                    .rposition(|ctx| ctx.source == FieldSource::Complex)
                {
                    let mut field = active_fields.remove(pos);
                    if let Some(instr) = field.instruction.take() {
                        if let Some(entry) = parse_toc_entry_field(&instr, paragraph) {
                            toc_entry_fields.push(entry);
                        }
                        // There was no `separate` event at which the instruction
                        // could normally be classified. Parse it before
                        // resolving nested operands or retaining a zero-width
                        // semantic closing run.
                        field.kind = Self::parse_field_instruction(&instr)
                            .map(|kind| kind.with_lock(field.locked));
                    }
                    if !field.is_instruction_operand && field.kind.is_some() {
                        closed_instruction_only_field = field.kind.clone();
                    }
                    Self::append_operand_result_to_parent(
                        field,
                        active_fields,
                        bookmark_text,
                        deferred_nested_fields,
                    );
                }
            }
            _ => {}
        }
        closed_instruction_only_field
    }

    fn process_fld_simple_start(
        attributes: Attributes<'_>,
        active_fields: &mut Vec<ActiveField>,
        toc_fields: &mut Vec<TocOptions>,
        toc_entry_fields: &mut Vec<TocEntryField>,
        paragraph: usize,
        paragraph_run_start: usize,
    ) {
        let locked = attributes
            .clone()
            .flatten()
            .find_map(|attr| {
                (attr.key.as_ref() == b"w:fldLock").then(|| {
                    parse_on_off(
                        Some(String::from_utf8_lossy(&attr.value).to_string()),
                        false,
                    )
                })
            })
            .unwrap_or(false);
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:instr" {
                // Attribute values use XML entities for the quotes surrounding
                // multi-word field arguments (`TC &quot;Deployment map&quot;`).
                // Decode them before tokenizing so the quoted argument remains
                // one field-code token rather than leaking `&quot;` into the TOC.
                let decoded = String::from_utf8_lossy(&attr.value);
                let instr = unescape(decoded.as_ref())
                    .map(|value| value.into_owned())
                    .unwrap_or_else(|_| decoded.into_owned());
                if let Some(entry) = parse_toc_entry_field(&instr, paragraph) {
                    toc_entry_fields.push(entry);
                } else if let Some(kind) = Self::parse_field_instruction(&instr) {
                    let is_instruction_operand = active_fields
                        .iter()
                        .rev()
                        .any(|field| field.instruction.is_some());
                    active_fields.push(ActiveField {
                        kind: Some(kind.with_lock(locked)),
                        locked,
                        source: FieldSource::Simple,
                        instruction: None,
                        is_instruction_operand,
                        operand_result: String::new(),
                        saved_instruction: None,
                        word_mac_body_result: None,
                        paragraph_run_start: Some(paragraph_run_start),
                        sequence_result_run: None,
                    });
                } else if field_instruction_is_toc(&instr) {
                    toc_fields.push(parse_toc_switches(&instr));
                }
                break;
            }
        }
    }

    fn finish_simple_field(
        active_fields: &mut Vec<ActiveField>,
        mut paragraph: Option<&mut Paragraph>,
        bookmark_text: &HashMap<String, String>,
        deferred_nested_fields: &mut Vec<DeferredNestedField>,
    ) {
        if let Some(pos) = active_fields
            .iter()
            .rposition(|ctx| ctx.source == FieldSource::Simple)
        {
            let field = active_fields.remove(pos);
            if !field.is_instruction_operand
                && field.kind.is_some()
                && field.paragraph_run_start.is_some_and(|start| {
                    paragraph
                        .as_ref()
                        .is_some_and(|paragraph| paragraph.runs.len() == start)
                })
            {
                // fldSimple contains a complete instruction even
                // when Word has not emitted a cached result run.
                // Preserve a zero-width semantic run for every
                // modeled kind: live fields can still resolve,
                // invisible fields keep their side effects, and
                // unsupported instructions survive edits.
                if let (Some(paragraph), Some(kind)) = (paragraph.as_mut(), field.kind.clone()) {
                    // Match ordinary result-run finalization:
                    // field instructions can carry interaction
                    // metadata (HYPERLINK) or effective run
                    // formatting (SYMBOL) even when no cached
                    // result run exists to host those values.
                    let mut properties = None;
                    Self::apply_symbol_field_formatting(&mut properties, &kind);
                    let hyperlink = Self::hyperlink_from_field_kind(&kind);
                    paragraph.runs.push(Run {
                        text: String::new(),
                        properties,
                        image: None,
                        text_box: None,
                        math: None,
                        field: Some(RunField {
                            word_mac_body_result: None,
                            kind,
                            cached_text: None,
                            source_field_id: None,
                            resolved_value: None,
                        }),
                        hyperlink,
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
                    });
                }
            }
            Self::append_operand_result_to_parent(
                field,
                active_fields,
                bookmark_text,
                deferred_nested_fields,
            );
        }
    }

    /// The field kind `process_fld_simple_start` gives a `w:fldSimple` with
    /// this instruction, or `None` when it is no field: a TOC entry field is
    /// recorded as a TOC entry, and a blank instruction is nothing. A
    /// `w:fldSimple` with a kind and no result run is a zero-width field run
    /// that has no `w:r`, which XML mapping and sync must recognize.
    pub fn simple_field_kind(instr: &str) -> Option<RunFieldKind> {
        if parse_toc_entry_field(instr, 0).is_some() {
            return None;
        }
        Self::parse_field_instruction(instr)
    }

    fn append_operand_result_to_parent(
        field: ActiveField,
        active_fields: &mut [ActiveField],
        bookmark_text: &HashMap<String, String>,
        deferred_nested_fields: &mut Vec<DeferredNestedField>,
    ) {
        if !field.is_instruction_operand {
            return;
        }
        let saved_nested = field.kind.as_ref().is_some_and(|kind| {
            matches!(
                kind.base_kind(),
                RunFieldKind::Compare { .. } | RunFieldKind::If { .. }
            )
        });
        if let Some(parent) = active_fields
            .iter_mut()
            .rev()
            .find(|parent| parent.instruction.is_some())
        {
            let before = parent.instruction.as_ref().map_or(0, String::len);
            let parent_instruction = parent.instruction.as_mut().expect("instruction");
            // A cached nested result may be stale even when its operands are
            // self-contained numeric literals. Refresh that safe subset now;
            // fields requiring unresolved bookmarks or package state retain
            // the cache until the model can preserve a structured expression.
            let live_result = field
                .kind
                .as_ref()
                .and_then(|kind| Self::locally_resolved_field_result(kind, bookmark_text));
            if let Some(result) = live_result.as_deref() {
                Self::append_nested_operand_token(parent_instruction, result);
            } else if let Some(kind) = field.kind {
                let index = deferred_nested_fields.len();
                deferred_nested_fields.push(DeferredNestedField {
                    kind,
                    cached_result: field.operand_result.clone(),
                });
                parent_instruction.push_str(DEFERRED_NESTED_FIELD_PREFIX);
                parent_instruction.push_str(&index.to_string());
                parent_instruction.push(DEFERRED_NESTED_FIELD_SUFFIX);
            } else {
                // Unsupported nested fields cannot be refreshed. Preserve the
                // saved result exactly as before instead of leaking an internal
                // placeholder into the parent instruction.
                Self::append_nested_operand_token(parent_instruction, &field.operand_result);
            }
            let appended = parent.instruction.as_ref().expect("instruction")[before..].to_string();
            if let Some(saved) = parent.saved_instruction.as_mut() {
                if saved_nested {
                    Self::append_nested_operand_token(saved, &field.operand_result);
                } else {
                    saved.push_str(&appended);
                }
            }
        }
    }

    /// The result Word for Mac prints in the main story for the IF field
    /// with instruction `instr`, when it differs from the evaluated one:
    /// `saved_instr` is the same instruction with each nested COMPARE or IF
    /// at its saved result (`if-compare-field-edge-cases`: a nested COMPARE
    /// cached as 0 makes `IF {COMPARE 2<10} = 1 ...` false there).
    fn word_mac_body_if_result(
        instr: &str,
        saved_instr: Option<&str>,
        bookmark_text: &HashMap<String, String>,
    ) -> Option<Box<str>> {
        let saved_instr = saved_instr.filter(|saved| *saved != instr)?;
        let saved = Self::parse_field_instruction(saved_instr)?;
        if !matches!(saved, RunFieldKind::If { .. }) {
            return None;
        }
        let result = Self::locally_resolved_field_result(&saved, bookmark_text)?;
        let live = Self::parse_field_instruction(instr)
            .and_then(|kind| Self::locally_resolved_field_result(&kind, bookmark_text));
        (live.as_ref() != Some(&result)).then(|| result.into_boxed_str())
    }

    fn append_nested_operand_token(parent_instruction: &mut String, result: &str) {
        // Although a nested field result is physically separate OOXML, Word
        // treats the whole result as one operand in the surrounding field. A
        // raw multi-word value on the right side of IF would otherwise shift
        // its trailing words into TrueText and FalseText during tokenization.
        // Quote whitespace-bearing or quote-bearing results and escape literal
        // quotes so the synthetic instruction preserves the exact operand.
        if result.is_empty() {
            parent_instruction.push_str("\"\"");
        } else if result.chars().any(char::is_whitespace) || result.contains('"') {
            parent_instruction.push('"');
            // Word never matches a nested result that holds a straight double
            // quote: `IF {REF x} = "…"` is false and `COMPARE` gives 0 for
            // every literal, escaped or not (Word for Mac 16.114 captures of
            // if-compare-field-edge-cases). A private-use mark no literal
            // carries keeps such an operand from comparing equal.
            if result.contains('"') {
                parent_instruction.push(crate::document::QUOTED_NESTED_RESULT_MARK);
            }
            for ch in result.chars() {
                if ch == '"' {
                    parent_instruction.push('\\');
                }
                parent_instruction.push(ch);
            }
            parent_instruction.push('"');
        } else {
            parent_instruction.push_str(result);
        }
    }

    fn resolve_deferred_nested_fields_in_elements(
        elements: &mut [BodyElement],
        deferred_nested_fields: &[DeferredNestedField],
        bookmark_text: &HashMap<String, String>,
    ) {
        for element in elements {
            match element {
                BodyElement::Paragraph(paragraph) => {
                    for run in &mut paragraph.runs {
                        if let Some(field) = run.field.as_mut() {
                            Self::resolve_deferred_operands_in_kind(
                                &mut field.kind,
                                deferred_nested_fields,
                                bookmark_text,
                                0,
                            );
                        }
                    }
                }
                BodyElement::Table(table) => {
                    for row in &mut table.rows {
                        for cell in &mut row.cells {
                            Self::resolve_deferred_nested_fields_in_elements(
                                &mut cell.blocks,
                                deferred_nested_fields,
                                bookmark_text,
                            );
                        }
                    }
                }
                BodyElement::Math(_) => {}
            }
        }
    }

    fn resolve_deferred_operands_in_kind(
        kind: &mut RunFieldKind,
        deferred_nested_fields: &[DeferredNestedField],
        bookmark_text: &HashMap<String, String>,
        depth: usize,
    ) {
        if let RunFieldKind::Locked { kind: inner } = kind {
            if matches!(
                inner.base_kind(),
                RunFieldKind::If { .. } | RunFieldKind::Compare { .. }
            ) {
                Self::resolve_deferred_operands_in_kind(
                    Rc::make_mut(inner),
                    deferred_nested_fields,
                    bookmark_text,
                    depth,
                );
            }
            return;
        }
        let (left, left_is_quoted, right, right_is_quoted) = match kind {
            RunFieldKind::Compare {
                left,
                left_is_quoted,
                right,
                right_is_quoted,
                ..
            }
            | RunFieldKind::If {
                left,
                left_is_quoted,
                right,
                right_is_quoted,
                ..
            } => (left, left_is_quoted, right, right_is_quoted),
            _ => return,
        };
        Self::resolve_deferred_operand(
            left,
            left_is_quoted,
            deferred_nested_fields,
            bookmark_text,
            depth,
        );
        Self::resolve_deferred_operand(
            right,
            right_is_quoted,
            deferred_nested_fields,
            bookmark_text,
            depth,
        );
    }

    fn resolve_deferred_operand(
        operand: &mut String,
        is_quoted: &mut bool,
        deferred_nested_fields: &[DeferredNestedField],
        bookmark_text: &HashMap<String, String>,
        depth: usize,
    ) {
        let Some(index) = Self::deferred_nested_field_index(operand) else {
            return;
        };
        // Every deferred entry points to a field that was already closed, so
        // the graph is acyclic. Keep a defensive bound in case malformed input
        // ever manages to manufacture a self-referential internal marker.
        if depth > deferred_nested_fields.len() {
            return;
        }
        let Some(deferred) = deferred_nested_fields.get(index) else {
            return;
        };
        let mut nested_kind = deferred.kind.clone();
        Self::resolve_deferred_operands_in_kind(
            &mut nested_kind,
            deferred_nested_fields,
            bookmark_text,
            depth + 1,
        );
        let result = Self::locally_resolved_field_result(&nested_kind, bookmark_text)
            .unwrap_or_else(|| deferred.cached_result.clone());
        // Synthetic quoting groups a textual field result into one operand and
        // prevents a value that happens to equal a bookmark name from being
        // dereferenced again. Numeric results remain unquoted so Word numeric
        // comparison semantics are preserved.
        *is_quoted = result.trim().parse::<f64>().is_err();
        *operand = result;
    }

    fn deferred_nested_field_index(operand: &str) -> Option<usize> {
        operand
            .strip_prefix(DEFERRED_NESTED_FIELD_PREFIX)?
            .strip_suffix(DEFERRED_NESTED_FIELD_SUFFIX)?
            .parse()
            .ok()
    }

    fn locally_resolved_field_result(
        kind: &RunFieldKind,
        bookmark_text: &HashMap<String, String>,
    ) -> Option<String> {
        match kind {
            // A REF whose bookmark has already closed is safe to refresh while
            // building the parent instruction. Unknown or forward bookmarks
            // remain `None`, preserving the nested field's saved result as the
            // conservative fallback.
            RunFieldKind::Ref {
                name,
                case_format,
                numbering: false,
                ..
            } => bookmark_text
                .get(name)
                .map(|value| crate::document::apply_field_text_case(value, *case_format)),
            RunFieldKind::Ref {
                numbering: true, ..
            } => None,
            RunFieldKind::Quote { text, case_format } => {
                Some(crate::document::apply_field_text_case(text, *case_format))
            }
            RunFieldKind::MacroButton { display_text, .. } => Some(display_text.clone()),
            RunFieldKind::GoToButton { display_text, .. } => Some(display_text.clone()),
            RunFieldKind::Compare {
                left,
                left_is_quoted,
                operator,
                right,
                right_is_quoted,
            } => Self::locally_resolved_comparison(
                left,
                *left_is_quoted,
                *operator,
                right,
                *right_is_quoted,
                bookmark_text,
            )
            .map(|result| if result { "1" } else { "0" }.to_string()),
            RunFieldKind::If {
                left,
                left_is_quoted,
                operator,
                right,
                right_is_quoted,
                true_text,
                false_text,
            } => Self::locally_resolved_comparison(
                left,
                *left_is_quoted,
                *operator,
                right,
                *right_is_quoted,
                bookmark_text,
            )
            .map(|result| {
                if result {
                    true_text.clone()
                } else {
                    false_text.clone().unwrap_or_default()
                }
            }),
            // Non-display storage and source-entry fields are explicitly
            // result-less. Returning a known empty operand prevents a stale
            // cache from entering an enclosing IF/COMPARE instruction.
            RunFieldKind::NonDisplay { .. } => Some(String::new()),
            _ => None,
        }
    }

    fn locally_resolved_comparison(
        left: &str,
        left_is_quoted: bool,
        operator: FieldComparisonOperator,
        right: &str,
        right_is_quoted: bool,
        bookmark_text: &HashMap<String, String>,
    ) -> Option<bool> {
        if !right_is_quoted {
            if let Some(result) =
                crate::document::evaluate_numeric_field_comparison(left, operator, right)
            {
                return Some(result);
            }
        }

        // Unquoted bookmark names and literal strings are syntactically
        // ambiguous after tokenization. Recompute only when an unquoted
        // operand is a bookmark already known to the parser. A quoted right
        // operand is always literal, so it must neither trigger bookmark
        // resolution nor be looked up by the shared evaluator.
        let has_known_bookmark = (!left_is_quoted && bookmark_text.contains_key(left))
            || (!right_is_quoted && bookmark_text.contains_key(right));
        // A numeric Expression1 paired with quoted Expression2 is also fully
        // self-contained. It must use character comparison immediately so a
        // stale nested-field cache cannot leak into the parent instruction.
        let has_safe_quoted_literal =
            right_is_quoted && (left_is_quoted || left.trim().parse::<f64>().is_ok());
        (has_known_bookmark || has_safe_quoted_literal).then(|| {
            crate::document::evaluate_field_comparison(
                left,
                left_is_quoted,
                operator,
                right,
                right_is_quoted,
                bookmark_text,
            )
        })
    }

    /// Parses the first non-switch token of a Word field instruction and maps
    /// it to a [`RunFieldKind`]. Field instructions follow the form
    /// `FIELDNAME [arg]... \switch[value]...` (see ECMA-376 Part 1, §17.16.3
    /// at <https://ooxml.info/docs/11/>). Switch tokens start with `\` and
    /// terminate the field-name scan, so only the leading identifier is
    /// matched. Unknown identifiers return `None`.
    pub fn parse_field_instruction(instr: &str) -> Option<RunFieldKind> {
        let normalized = instr.trim();
        if normalized.is_empty() {
            return None;
        }
        // Keep quoted arguments together. This matters for custom document
        // property names such as `DOCPROPERTY "Document Title"`, and also makes
        // the argument handling below consistent with TOC/TC field parsing.
        let detailed_tokens = crate::document::tokenize_field_instruction_with_quotes(normalized);
        let raw_tokens = detailed_tokens
            .iter()
            .map(|token| token.text.clone())
            .collect::<Vec<_>>();
        let quoted_tokens = detailed_tokens
            .iter()
            .map(|token| token.quoted)
            .collect::<Vec<_>>();
        let mut name_idx = None;
        for (idx, token) in raw_tokens.iter().enumerate() {
            let cleaned = token.trim_matches('"');
            if cleaned.is_empty() {
                continue;
            }
            if cleaned.starts_with('\\') {
                // A leading switch before the field name is unusual; skip it.
                continue;
            }
            name_idx = Some(idx);
            break;
        }
        let name_idx = name_idx?;
        let field_name = raw_tokens[name_idx].trim_matches('"').to_ascii_uppercase();
        // TOC and TC are structural fields collected outside RunFieldKind:
        // TOC describes generated body content and TC contributes a hidden
        // outline entry. They must not be converted into inline cached fields.
        if matches!(field_name.as_str(), "TOC" | "TC") {
            return None;
        }
        // Isolate evaluator parsing in a closure so `?` inside an individual
        // field grammar means "this evaluator declined" rather than returning
        // from `parse_field_instruction` before the cached-field fallback.
        let parsed = (|| match field_name.as_str() {
            "PAGE" => Some(RunFieldKind::PageNumber),
            "NUMPAGES" => Some(RunFieldKind::NumPages),
            "NUMWORDS" => Some(RunFieldKind::NumWords),
            "NUMCHARS" => Some(RunFieldKind::NumChars),
            "EDITTIME" => Some(RunFieldKind::EditTime),
            "FILESIZE" => {
                // Word's field-specific switches are case-insensitive. If a
                // malformed instruction contains both, follow the last switch
                // in instruction order rather than inventing a combined unit.
                let unit = raw_tokens[name_idx + 1..].iter().fold(
                    crate::document::FileSizeUnit::Bytes,
                    |unit, token| match token.to_ascii_lowercase().as_str() {
                        "\\k" => crate::document::FileSizeUnit::Kilobytes,
                        "\\m" => crate::document::FileSizeUnit::Megabytes,
                        _ => unit,
                    },
                );
                Some(RunFieldKind::FileSize { unit })
            }
            "SECTION" => Some(RunFieldKind::SectionNumber),
            "SECTIONPAGES" => Some(RunFieldKind::SectionPages),
            "STYLEREF" => {
                let style = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|token| token.trim_matches('"'))
                    .find(|token| !token.is_empty() && !token.starts_with('\\'))?;
                let from_bottom = raw_tokens[name_idx + 1..]
                    .iter()
                    .any(|token| token.eq_ignore_ascii_case("\\l"));
                Some(RunFieldKind::StyleRef {
                    style: style.to_string(),
                    from_bottom,
                })
            }
            "DOCPROPERTY" => {
                let name = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|token| token.trim_matches('"'))
                    .find(|token| !token.is_empty() && !token.starts_with('\\'))?;
                Some(RunFieldKind::DocumentProperty {
                    name: name.to_string(),
                    case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
                })
            }
            "DOCVARIABLE" => {
                // Variables use the same instruction grammar as document
                // properties, including quoted names containing whitespace.
                // Switches (for example `\* MERGEFORMAT`) are not part of the
                // variable name and are deliberately skipped.
                let name = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|token| token.trim_matches('"'))
                    .find(|token| !token.is_empty() && !token.starts_with('\\'))?;
                Some(RunFieldKind::DocumentVariable {
                    name: name.to_string(),
                    case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
                })
            }
            "MERGEFIELD" => {
                // A merge column can be quoted when its display name contains
                // spaces. Formatting switches such as `\* MERGEFORMAT` are not
                // part of that name.
                let name = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|token| token.trim_matches('"'))
                    .find(|token| !token.is_empty() && !token.starts_with('\\'))?;
                Some(RunFieldKind::MergeField {
                    name: name.to_string(),
                })
            }
            "FILLIN" => Self::parse_fillin_field(&raw_tokens, name_idx),
            "ASK" => Self::parse_ask_field(&raw_tokens, name_idx),
            // Word also exposes the most common built-in properties as
            // dedicated field names. Normalize them onto the same package
            // lookup path as DOCPROPERTY so body and header/footer behavior
            // cannot drift between the two instruction spellings.
            "TITLE" | "SUBJECT" | "AUTHOR" | "KEYWORDS" | "COMMENTS" | "LASTSAVEDBY" | "REVNUM" => {
                Some(RunFieldKind::DocumentProperty {
                    name: match field_name.as_str() {
                        "REVNUM" => "Revision".to_string(),
                        _ => field_name,
                    },
                    case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
                })
            }
            "TEMPLATE" => {
                // `TEMPLATE` without a field-specific switch is the attached
                // template name persisted in docProps/app.xml. `\p` asks for
                // the full filesystem/URL path, which that scalar metadata does
                // not contain; keep Word's saved result for that richer form.
                let includes_full_path = raw_tokens[name_idx + 1..]
                    .iter()
                    .any(|token| token.eq_ignore_ascii_case("\\p"));
                (!includes_full_path).then(|| RunFieldKind::DocumentProperty {
                    name: "Template".to_string(),
                    case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
                })
            }
            "INFO" => {
                // INFO has the shape `INFO InfoType ["NewValue"]`. Tokens
                // after the first switch belong to general formatting (for
                // example `\* MERGEFORMAT`) rather than to the field operands.
                // Only the one-operand form is a side-effect-free property
                // read; the two-operand form updates document metadata in Word
                // and must remain cached until an explicit update operation is
                // available.
                let operands = raw_tokens[name_idx + 1..]
                    .iter()
                    .take_while(|token| !token.starts_with('\\'))
                    .map(|token| token.trim_matches('"'))
                    .filter(|token| !token.is_empty())
                    .collect::<Vec<_>>();
                (operands.len() == 1).then(|| RunFieldKind::DocumentProperty {
                    name: operands[0].to_string(),
                    case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
                })
            }
            "PAGEREF" => {
                // The first argument after `PAGEREF` (skipping any switches) is
                // the bookmark name. Without it the field cannot be resolved.
                let name = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|t| t.trim_matches('"'))
                    .find(|t| !t.is_empty() && !t.starts_with('\\'))?;
                Some(RunFieldKind::PageRef {
                    name: name.to_string(),
                    hyperlink: raw_tokens[name_idx + 1..]
                        .iter()
                        .any(|token| token.eq_ignore_ascii_case("\\h")),
                })
            }
            "NOTEREF" => {
                let name = raw_tokens.get(name_idx + 1)?.trim_matches('"');
                if name.is_empty() || name.starts_with('\\') {
                    return None;
                }
                let switches = &raw_tokens[name_idx + 2..];
                Some(RunFieldKind::NoteRef {
                    name: name.to_string(),
                    hyperlink: switches
                        .iter()
                        .any(|token| token.eq_ignore_ascii_case("\\h")),
                    formatted: switches
                        .iter()
                        .any(|token| token.eq_ignore_ascii_case("\\f")),
                    relative_position: switches
                        .iter()
                        .any(|token| token.eq_ignore_ascii_case("\\p")),
                })
            }
            "DATE" => Some(RunFieldKind::Date {
                format: extract_field_format_switch(normalized),
            }),
            "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                let source = match field_name.as_str() {
                    "CREATEDATE" => DocumentDateSource::Created,
                    "SAVEDATE" => DocumentDateSource::Saved,
                    "PRINTDATE" => DocumentDateSource::Printed,
                    _ => unreachable!("matched package-backed date field"),
                };
                Some(RunFieldKind::DocumentDate {
                    source,
                    format: extract_field_format_switch(normalized),
                })
            }
            "TIME" => Some(RunFieldKind::Time {
                format: extract_field_format_switch(normalized),
            }),
            "REF" => {
                // The first argument after `REF` (skipping switches) is the
                // bookmark name whose text is inserted.
                let relative_name_idx = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|t| t.trim_matches('"'))
                    .position(|t| !t.is_empty() && !t.starts_with('\\'))?;
                let bookmark_idx = name_idx + 1 + relative_name_idx;
                let name = raw_tokens[bookmark_idx].trim_matches('"');
                Some(RunFieldKind::Ref {
                    name: name.to_string(),
                    case_format: Self::parse_field_text_case(&raw_tokens, bookmark_idx + 1),
                    numbering: raw_tokens[bookmark_idx + 1..].iter().any(|token| {
                        token.eq_ignore_ascii_case("\\n")
                            || token.eq_ignore_ascii_case("\\r")
                            || token.eq_ignore_ascii_case("\\w")
                    }),
                    hyperlink: raw_tokens[name_idx + 1..]
                        .iter()
                        .any(|token| token.eq_ignore_ascii_case("\\h")),
                })
            }
            "QUOTE" => {
                let text_index = name_idx + 1;
                // Word's QUOTE grammar requires one quoted literal. Requiring
                // quote provenance prevents malformed `QUOTE text` input from
                // being mistaken for a value that Jade can safely recompute.
                quoted_tokens
                    .get(text_index)
                    .copied()
                    .unwrap_or(false)
                    .then(|| RunFieldKind::Quote {
                        text: raw_tokens[text_index].clone(),
                        case_format: Self::parse_field_text_case(&raw_tokens, text_index + 1),
                    })
            }
            "SYMBOL" => Self::parse_symbol_field(&raw_tokens, name_idx),
            "SET" => {
                let bookmark = raw_tokens
                    .get(name_idx + 1)
                    .map(|token| token.trim_matches('"'))
                    .filter(|token| !token.is_empty() && !token.starts_with('\\'))?;
                let value = raw_tokens
                    .get(name_idx + 2)
                    .filter(|token| !token.starts_with('\\'))?;
                Some(RunFieldKind::Set {
                    bookmark: bookmark.to_string(),
                    value: value.clone(),
                })
            }
            "AUTONUM" | "AUTONUMLGL" | "AUTONUMOUT" => Some(RunFieldKind::AutoNum {
                kind: match field_name.as_str() {
                    "AUTONUMLGL" => crate::document::AutoNumberKind::Legal,
                    "AUTONUMOUT" => crate::document::AutoNumberKind::Outline,
                    _ => crate::document::AutoNumberKind::Plain,
                },
                separator: raw_tokens[name_idx + 1..]
                    .iter()
                    .position(|token| token.eq_ignore_ascii_case("\\s"))
                    .map(|index| {
                        raw_tokens
                            .get(name_idx + index + 2)
                            .filter(|token| !token.starts_with('\\'))
                            .cloned()
                            .unwrap_or_default()
                    }),
                omit_trailing_period: raw_tokens[name_idx + 1..]
                    .iter()
                    .any(|token| token.eq_ignore_ascii_case("\\e")),
                number_format: Self::parse_seq_number_format(&raw_tokens, name_idx + 1),
                case_format: Self::parse_field_text_case(&raw_tokens, name_idx + 1),
            }),
            "SEQ" => {
                // The first argument after `SEQ` (skipping switches) is the
                // sequence identifier (e.g. `Figure`, `Table`).
                let relative_identifier_idx = raw_tokens[name_idx + 1..]
                    .iter()
                    .map(|t| t.trim_matches('"'))
                    .position(|t| !t.is_empty() && !t.starts_with('\\'))?;
                let identifier_idx = name_idx + 1 + relative_identifier_idx;
                let identifier = raw_tokens[identifier_idx].trim_matches('"');
                let bookmark = raw_tokens
                    .get(identifier_idx + 1)
                    .map(|token| token.trim_matches('"'))
                    .filter(|token| !token.is_empty() && !token.starts_with('\\'))
                    .map(str::to_string);
                Some(RunFieldKind::Seq {
                    identifier: identifier.to_string(),
                    restart_at: Self::parse_seq_restart(&raw_tokens, identifier_idx + 1),
                    repeat_previous: Self::parse_seq_repeat(&raw_tokens, identifier_idx + 1),
                    hide_result: Self::parse_seq_hide_result(&raw_tokens, identifier_idx + 1),
                    restart_after_heading: Self::parse_seq_heading_level(
                        &raw_tokens,
                        identifier_idx + 1,
                    ),
                    bookmark,
                    number_format: Self::parse_seq_number_format(&raw_tokens, identifier_idx + 1),
                    case_format: Self::parse_field_text_case(&raw_tokens, identifier_idx + 1),
                })
            }
            "COMPARE" => parse_comparison_field(&raw_tokens, &quoted_tokens, name_idx, false),
            "IF" => parse_comparison_field(&raw_tokens, &quoted_tokens, name_idx, true),
            "HYPERLINK" => Self::parse_hyperlink_field(&raw_tokens, name_idx),
            "MACROBUTTON" => Self::parse_macrobutton_field(&raw_tokens, name_idx),
            "GOTOBUTTON" => Self::parse_gotobutton_field(&raw_tokens, name_idx),
            "EQ" => parse_eq_ruby_field(normalized)
                .or_else(|| parse_eq_enclosed_field(normalized))
                .or_else(|| {
                    parse_eq_distributed_field(normalized)
                        .map(|(text, guide)| RunFieldKind::EqDistributed { text, guide })
                }),
            "PRIVATE" | "ADDIN" | "XE" | "TA" => Some(RunFieldKind::NonDisplay {
                instruction: normalized.to_string(),
            }),
            _ => None,
        })();
        // A known evaluator can decline unsafe or incomplete operands (for
        // example `TEMPLATE \p`, a mutating INFO form, or malformed PAGEREF).
        // Those are still Word fields. Keep the instruction and cached result
        // instead of flattening them merely because local evaluation stopped.
        parsed.or_else(|| {
            Some(RunFieldKind::Unsupported {
                instruction: normalized.to_string(),
            })
        })
    }

    fn parse_field_text_case(tokens: &[String], start: usize) -> Option<FieldTextCase> {
        let mut format = None;
        let mut expects_format_name = false;
        for token in &tokens[start..] {
            let name = if token == "\\*" {
                expects_format_name = true;
                continue;
            } else if let Some(compact) = token.strip_prefix("\\*") {
                expects_format_name = false;
                compact
            } else if expects_format_name {
                expects_format_name = false;
                token.as_str()
            } else {
                continue;
            };
            let parsed = match name.to_ascii_uppercase().as_str() {
                "UPPER" => Some(FieldTextCase::Upper),
                "LOWER" => Some(FieldTextCase::Lower),
                "FIRSTCAP" => Some(FieldTextCase::FirstCap),
                "CAPS" => Some(FieldTextCase::Caps),
                _ => None,
            };
            if parsed.is_some() {
                format = parsed;
            }
        }
        format
    }

    fn parse_seq_restart(tokens: &[String], start: usize) -> Option<u32> {
        let mut expects_value = false;
        for token in &tokens[start..] {
            if expects_value {
                return token.parse().ok();
            }
            if token.eq_ignore_ascii_case("\\r") {
                expects_value = true;
                continue;
            }
            if let Some(value) = token
                .strip_prefix("\\r")
                .or_else(|| token.strip_prefix("\\R"))
                .filter(|value| !value.is_empty())
            {
                return value.parse().ok();
            }
        }
        None
    }

    fn parse_seq_repeat(tokens: &[String], start: usize) -> bool {
        tokens[start..]
            .iter()
            .any(|token| token.eq_ignore_ascii_case("\\c"))
    }

    fn parse_seq_hide_result(tokens: &[String], start: usize) -> bool {
        let mut hide_result = false;
        let mut has_general_format = false;
        for token in &tokens[start..] {
            hide_result |= token.eq_ignore_ascii_case("\\h");
            has_general_format |= token == "\\*" || token.starts_with("\\*");
        }
        hide_result && !has_general_format
    }

    fn parse_seq_heading_level(tokens: &[String], start: usize) -> Option<u8> {
        let mut expects_value = false;
        for token in &tokens[start..] {
            let value = if expects_value {
                token.as_str()
            } else if token.eq_ignore_ascii_case("\\s") {
                expects_value = true;
                continue;
            } else if let Some(compact) = token
                .strip_prefix("\\s")
                .or_else(|| token.strip_prefix("\\S"))
                .filter(|value| !value.is_empty())
            {
                compact
            } else {
                continue;
            };
            return value
                .parse::<u8>()
                .ok()
                .filter(|level| (1..=9).contains(level));
        }
        None
    }

    fn parse_seq_number_format(tokens: &[String], start: usize) -> Option<SeqNumberFormat> {
        let mut format = None;
        let mut expects_format_name = false;
        for token in &tokens[start..] {
            let name = if token == "\\*" {
                expects_format_name = true;
                continue;
            } else if let Some(compact) = token.strip_prefix("\\*") {
                expects_format_name = false;
                compact
            } else if expects_format_name {
                expects_format_name = false;
                token.as_str()
            } else {
                continue;
            };
            let parsed = match name {
                "ROMAN" => Some(SeqNumberFormat::UpperRoman),
                "roman" => Some(SeqNumberFormat::LowerRoman),
                "ALPHABETIC" => Some(SeqNumberFormat::UpperAlphabetic),
                "alphabetic" => Some(SeqNumberFormat::LowerAlphabetic),
                name if name.eq_ignore_ascii_case("ARABIC") => Some(SeqNumberFormat::Arabic),
                name if name.eq_ignore_ascii_case("HEX") => Some(SeqNumberFormat::Hex),
                name if name.eq_ignore_ascii_case("ORDINAL") => Some(SeqNumberFormat::Ordinal),
                name if name.eq_ignore_ascii_case("CARDTEXT") => Some(SeqNumberFormat::CardText),
                name if name.eq_ignore_ascii_case("ORDTEXT") => Some(SeqNumberFormat::OrdText),
                name if name.eq_ignore_ascii_case("DOLLARTEXT") => {
                    Some(SeqNumberFormat::DollarText)
                }
                _ => None,
            };
            if parsed.is_some() {
                format = parsed;
            }
        }
        format
    }

    fn parse_symbol_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let value = tokens.get(name_idx + 1)?.trim_matches('"');
        if value.is_empty() || value.starts_with('\\') {
            return None;
        }
        // ECMA-376 allows decimal and a 0x/0X-prefixed hexadecimal operand.
        // Parsing into u32 lets the resolver reject invalid Unicode scalar
        // values without either wrapping them or manufacturing replacement
        // characters that Word did not save.
        let code_point = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .map(|hex| u32::from_str_radix(hex, 16))
            .unwrap_or_else(|| value.parse::<u32>())
            .ok()?;

        let mut character_set = SymbolCharacterSet::Default;
        let mut font = None;
        let mut font_size_points = None;
        let mut preserve_line_height = false;
        let mut idx = name_idx + 2;
        while idx < tokens.len() {
            match tokens[idx].to_ascii_lowercase().as_str() {
                "\\a" => character_set = SymbolCharacterSet::Ansi,
                "\\j" => character_set = SymbolCharacterSet::ShiftJis,
                "\\u" => character_set = SymbolCharacterSet::Unicode,
                "\\h" => preserve_line_height = true,
                "\\f" => {
                    if let Some(value) =
                        tokens.get(idx + 1).filter(|value| !value.starts_with('\\'))
                    {
                        font = Some(value.clone());
                        idx += 1;
                    }
                }
                "\\s" => {
                    if let Some(value) =
                        tokens.get(idx + 1).filter(|value| !value.starts_with('\\'))
                    {
                        // The field grammar defines \s as an integral point
                        // size. Invalid values are ignored while the character
                        // remains usable, matching Word's tolerant field load.
                        font_size_points = value.parse::<u32>().ok();
                        idx += 1;
                    }
                }
                _ => {}
            }
            idx += 1;
        }

        Some(RunFieldKind::Symbol {
            code_point,
            character_set,
            font,
            font_size_points,
            preserve_line_height,
        })
    }

    fn apply_symbol_field_formatting(properties: &mut Option<RunProperties>, kind: &RunFieldKind) {
        let RunFieldKind::Symbol {
            font,
            font_size_points,
            ..
        } = kind.base_kind()
        else {
            return;
        };
        if font.is_none() && font_size_points.is_none() {
            return;
        }

        let properties = properties.get_or_insert_with(RunProperties::default);
        if let Some(font) = font.as_deref() {
            let font: Rc<str> = Rc::from(font);
            properties.font_family = Some(font.clone());
            let fonts = properties.fonts.get_or_insert_with(RunFonts::default);
            // A SYMBOL font chooses a glyph repertoire. Apply it to every
            // script slot so the font resolver cannot replace the requested
            // mapping merely because the resulting character is classified as
            // East Asian or complex script rather than Latin.
            fonts.ascii = Some(font.clone());
            fonts.h_ansi = Some(font.clone());
            fonts.east_asia = Some(font.clone());
            fonts.complex = Some(font);
            // Direct font names take precedence over theme references in Word.
            // Clear inherited result-run theme slots to keep that precedence
            // explicit after styles and defaults are materialized.
            fonts.ascii_theme = None;
            fonts.h_ansi_theme = None;
            fonts.east_asia_theme = None;
            fonts.complex_theme = None;
        }
        if let Some(points) = font_size_points {
            properties.font_size = Some(*points as f32);
        }
    }

    fn parse_hyperlink_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let mut target = None;
        let mut anchor = None;
        let mut tooltip = None;
        let mut target_frame = None;
        let mut idx = name_idx + 1;
        while idx < tokens.len() {
            match tokens[idx].to_ascii_lowercase().as_str() {
                "\\l" => {
                    if let Some(value) =
                        tokens.get(idx + 1).filter(|value| !value.starts_with('\\'))
                    {
                        anchor = Some(value.clone());
                        idx += 1;
                    }
                }
                "\\o" => {
                    if let Some(value) =
                        tokens.get(idx + 1).filter(|value| !value.starts_with('\\'))
                    {
                        tooltip = Some(value.clone());
                        idx += 1;
                    }
                }
                "\\t" => {
                    if let Some(value) =
                        tokens.get(idx + 1).filter(|value| !value.starts_with('\\'))
                    {
                        target_frame = Some(value.clone());
                        idx += 1;
                    }
                }
                token if token.starts_with('\\') => {}
                _ if target.is_none() => target = Some(tokens[idx].clone()),
                _ => {}
            }
            idx += 1;
        }
        if target.is_none() && anchor.is_none() {
            return None;
        }
        Some(RunFieldKind::Hyperlink {
            target,
            anchor,
            tooltip,
            target_frame,
        })
    }

    fn parse_fillin_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let mut prompt = None;
        let mut default = None;
        let mut ask_once = false;
        let mut idx = name_idx + 1;

        while idx < tokens.len() {
            match tokens[idx].to_ascii_lowercase().as_str() {
                "\\d" => {
                    if let Some(value) = tokens.get(idx + 1) {
                        if !value.starts_with('\\') {
                            // Empty quotes are a meaningful blank default, so
                            // retain an empty token instead of filtering it out.
                            default = Some(value.trim_matches('"').to_string());
                            idx += 1;
                        }
                    }
                }
                "\\o" => ask_once = true,
                "\\*" => {
                    // The general formatting switch consumes its following
                    // format name (for example MERGEFORMAT or Caps); neither
                    // token belongs to the FILLIN prompt.
                    if tokens.get(idx + 1).is_some() {
                        idx += 1;
                    }
                }
                token if token.starts_with('\\') => {}
                _ if prompt.is_none() => prompt = Some(tokens[idx].trim_matches('"').to_string()),
                _ => {}
            }
            idx += 1;
        }

        Some(RunFieldKind::FillIn {
            prompt,
            default,
            ask_once,
        })
    }

    fn parse_ask_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let bookmark = tokens
            .get(name_idx + 1)
            .map(|token| token.trim_matches('"'))
            .filter(|token| !token.is_empty() && !token.starts_with('\\'))?
            .to_string();
        let mut prompt = None;
        let mut default = None;
        let mut ask_once = false;
        let mut idx = name_idx + 2;

        while idx < tokens.len() {
            match tokens[idx].to_ascii_lowercase().as_str() {
                "\\d" => {
                    if let Some(value) = tokens.get(idx + 1) {
                        if !value.starts_with('\\') {
                            default = Some(value.trim_matches('"').to_string());
                            idx += 1;
                        }
                    }
                }
                "\\o" => ask_once = true,
                "\\*" => {
                    if tokens.get(idx + 1).is_some() {
                        idx += 1;
                    }
                }
                token if token.starts_with('\\') => {}
                _ if prompt.is_none() => prompt = Some(tokens[idx].trim_matches('"').to_string()),
                _ => {}
            }
            idx += 1;
        }

        Some(RunFieldKind::Ask {
            bookmark,
            prompt,
            default,
            ask_once,
        })
    }

    fn hyperlink_from_field_kind(kind: &RunFieldKind) -> Option<RunHyperlink> {
        match kind.base_kind() {
            RunFieldKind::Hyperlink {
                target,
                anchor,
                tooltip,
                target_frame,
            } => Some(RunHyperlink {
                relationship_id: None,
                anchor: anchor.clone(),
                target: target.clone(),
                target_mode: target.as_ref().map(|_| "External".to_string()),
                tooltip: tooltip.clone(),
                target_frame: target_frame.clone(),
            }),
            RunFieldKind::Ref {
                name,
                hyperlink: true,
                ..
            }
            | RunFieldKind::PageRef {
                name,
                hyperlink: true,
            }
            | RunFieldKind::NoteRef {
                name,
                hyperlink: true,
                ..
            } => Some(RunHyperlink {
                relationship_id: None,
                anchor: Some(name.clone()),
                target: None,
                target_mode: None,
                tooltip: None,
                target_frame: None,
            }),
            RunFieldKind::GoToButton { destination, .. }
                if Self::gotobutton_destination_is_bookmark(destination) =>
            {
                Some(RunHyperlink {
                    relationship_id: None,
                    anchor: Some(destination.clone()),
                    target: None,
                    target_mode: None,
                    tooltip: None,
                    target_frame: None,
                })
            }
            _ => None,
        }
    }

    fn parse_macrobutton_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let (macro_name, display_text) = Self::parse_button_field_operands(tokens, name_idx)?;
        Some(RunFieldKind::MacroButton {
            macro_name,
            display_text,
        })
    }

    fn parse_gotobutton_field(tokens: &[String], name_idx: usize) -> Option<RunFieldKind> {
        let (destination, display_text) = Self::parse_button_field_operands(tokens, name_idx)?;
        Some(RunFieldKind::GoToButton {
            destination,
            display_text,
        })
    }

    fn parse_button_field_operands(tokens: &[String], name_idx: usize) -> Option<(String, String)> {
        let first_operand = tokens
            .get(name_idx + 1)
            .filter(|token| !token.is_empty() && !token.starts_with('\\'))?;
        // DisplayText is the complete remainder of the instruction, not just
        // one token. Stop before a general format switch so the common
        // `\* MERGEFORMAT` suffix does not become visible button text.
        let display_text = tokens[name_idx + 2..]
            .iter()
            .take_while(|token| !token.starts_with('\\'))
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        (!display_text.is_empty()).then(|| (first_operand.clone(), display_text))
    }

    fn gotobutton_destination_is_bookmark(destination: &str) -> bool {
        if destination.is_empty()
            || destination.chars().any(char::is_whitespace)
            || destination
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return false;
        }
        let mut characters = destination.chars();
        let Some(prefix) = characters.next() else {
            return false;
        };
        let suffix = characters.as_str();
        // Word reserves sN/lN/fN/aN for section, line, footnote, and annotation
        // destinations. They are clickable in Word but are not bookmark names.
        !(matches!(prefix.to_ascii_lowercase(), 's' | 'l' | 'f' | 'a')
            && !suffix.is_empty()
            && suffix.chars().all(|character| character.is_ascii_digit()))
    }
}

/// Parses the shared `Expression1 Operator Expression2` prefix used by
/// `COMPARE` and `IF`. Word's UI documents spaces around operators, but its own
/// examples also serialize `order>=100`; accept both shapes without weakening
/// the general field tokenizer used by TOC and other instructions.
fn parse_comparison_field(
    tokens: &[String],
    quoted_tokens: &[bool],
    name_idx: usize,
    is_if: bool,
) -> Option<RunFieldKind> {
    let args = tokens.get(name_idx + 1..)?;
    let quoted_args = quoted_tokens.get(name_idx + 1..)?;
    let (left, left_is_quoted, operator, right, right_is_quoted, consumed) =
        parse_comparison_operands(args, quoted_args)?;
    // Word's IF reads a compact expression whose left operand is a number
    // (`2<10`, `1=2`, `10>2`) as a malformed number and shows its syntax
    // error; a bookmark or name on the left (`OrderTotal>=100`, `a=a`), a
    // spaced operator and COMPARE all evaluate (Word for Mac 16.114 header
    // captures; if-compare-field-edge-cases' header `IF 2<10 …`, and its
    // nested `IF 1=2 …` whose result never equals "").
    let compact_number = is_if
        && !left_is_quoted
        && left.starts_with(|ch: char| ch.is_ascii_digit())
        && !args
            .iter()
            .zip(quoted_args)
            .any(|(token, quoted)| !*quoted && parse_comparison_operator(token).is_some());
    if compact_number {
        return Some(RunFieldKind::Quote {
            text: crate::document::WORD_IF_SYNTAX_ERROR.to_string(),
            case_format: None,
        });
    }
    if is_if {
        let true_text = args.get(consumed)?.clone();
        let false_text = args.get(consumed + 1).cloned();
        Some(RunFieldKind::If {
            left,
            left_is_quoted,
            operator,
            right,
            right_is_quoted,
            true_text,
            false_text,
        })
    } else {
        Some(RunFieldKind::Compare {
            left,
            left_is_quoted,
            operator,
            right,
            right_is_quoted,
        })
    }
}

fn parse_comparison_operands(
    args: &[String],
    quoted_args: &[bool],
) -> Option<(String, bool, FieldComparisonOperator, String, bool, usize)> {
    // First handle the canonical spaced syntax. Joining the left side allows a
    // future nested-field normalizer to supply a multi-token expression while
    // retaining one unambiguous operator boundary.
    if let Some((operator_index, operator)) = args
        .iter()
        .enumerate()
        .find_map(|(index, token)| parse_comparison_operator(token).map(|op| (index, op)))
    {
        if operator_index == 0 {
            return None;
        }
        let right = args.get(operator_index + 1)?.clone();
        return Some((
            args[..operator_index].join(" "),
            operator_index == 1 && *quoted_args.first()?,
            operator,
            right,
            *quoted_args.get(operator_index + 1)?,
            operator_index + 2,
        ));
    }

    // Then handle compact forms such as `100>=99`. Check two-character
    // operators before their one-character prefixes at each byte boundary.
    let compact = args.first()?;
    for (index, _) in compact.char_indices() {
        let tail = &compact[index..];
        let Some((token, operator)) = [
            ("<>", FieldComparisonOperator::NotEqual),
            (">=", FieldComparisonOperator::GreaterOrEqual),
            ("<=", FieldComparisonOperator::LessOrEqual),
            ("=", FieldComparisonOperator::Equal),
            (">", FieldComparisonOperator::Greater),
            ("<", FieldComparisonOperator::Less),
        ]
        .into_iter()
        .find(|(token, _)| tail.starts_with(token)) else {
            continue;
        };
        let left = compact[..index].to_string();
        if left.is_empty() {
            return None;
        }
        let inline_right = &tail[token.len()..];
        if inline_right.is_empty() {
            return Some((
                left,
                false,
                operator,
                args.get(1)?.clone(),
                *quoted_args.get(1)?,
                2,
            ));
        }
        return Some((left, false, operator, inline_right.to_string(), false, 1));
    }
    None
}

fn parse_comparison_operator(token: &str) -> Option<FieldComparisonOperator> {
    match token {
        "=" => Some(FieldComparisonOperator::Equal),
        "<>" => Some(FieldComparisonOperator::NotEqual),
        ">" => Some(FieldComparisonOperator::Greater),
        "<" => Some(FieldComparisonOperator::Less),
        ">=" => Some(FieldComparisonOperator::GreaterOrEqual),
        "<=" => Some(FieldComparisonOperator::LessOrEqual),
        _ => None,
    }
}

/// Extracts Word's legacy Japanese distributed-text field shape:
/// `EQ \o\ad(display, guide)`. ECMA-376 §17.16.5.22 defines `EQ`; Word uses the
/// overstrike array switches here to keep `display` fitted to the measured
/// width of `guide`.
fn parse_eq_distributed_field(instr: &str) -> Option<(String, String)> {
    let mut normalized = instr.trim();
    let first_space = normalized
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, _)| idx)?;
    if !normalized[..first_space].eq_ignore_ascii_case("EQ") {
        return None;
    }
    normalized = normalized[first_space..].trim_start();

    let lower = normalized.to_ascii_lowercase();
    let ad_marker = lower.find("\\ad(")?;
    let before_ad = &lower[..ad_marker];
    if !before_ad.contains("\\o") {
        return None;
    }

    let args_start = ad_marker + "\\ad(".len();
    let args = &normalized[args_start..];
    let args_end = find_matching_field_paren(args)?;
    split_eq_distributed_args(&args[..args_end])
}

/// Extracts ruby written as an `EQ` overstrike: `EQ [\* jcN] [\* hpsN]
/// \o\aX(\s\upN(guide),base)`. Word writes this shape for phonetic guides
/// (ECMA-376 §17.16.5.22); the `\* jc` values follow `w:rubyAlign`'s order.
fn parse_eq_ruby_field(instr: &str) -> Option<RunFieldKind> {
    let trimmed = instr.trim();
    let (name, rest) = trimmed.split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("EQ") {
        return None;
    }
    let lower = rest.to_ascii_lowercase();
    let overstrike = lower.find("\\o")?;
    let switches = &lower[..overstrike];
    let switch_value = |key: &str| {
        let at = switches.find(key)? + key.len();
        let digits: String = switches[at..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse::<u32>().ok()
    };
    let jc_align = switch_value("\\* jc").and_then(|jc| {
        [
            "center",
            "distributeletter",
            "distributespace",
            "left",
            "right",
        ]
        .get(jc as usize)
        .map(|align| align.to_string())
    });
    let size_half_points = switch_value("\\* hps");

    let after_o = &lower[overstrike + 2..];
    let (switch_align, open) = match after_o.strip_prefix("\\a") {
        Some(aligned) => {
            let align = match aligned.chars().next()? {
                'c' => "center",
                'd' => "distributespace",
                'l' => "left",
                'r' => "right",
                _ => return None,
            };
            (Some(align.to_string()), overstrike + 2 + 3)
        }
        None => (None, overstrike + 2),
    };
    let args = rest.get(open..)?.trim_start().strip_prefix('(')?;
    let guide_args = args.trim_start();
    if !guide_args.get(..2)?.eq_ignore_ascii_case("\\s") {
        return None;
    }
    let raised = guide_args[2..].trim_start();
    if !raised.get(..3)?.eq_ignore_ascii_case("\\up") {
        return None;
    }
    let raised = raised[3..].trim_start();
    let digits_end = raised
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(raised.len());
    let raise_points = raised[..digits_end].parse::<u32>().ok()?;
    let guide_open = raised[digits_end..].trim_start().strip_prefix('(')?;
    let guide_end = find_matching_field_paren(guide_open)?;
    let guide = clean_eq_distributed_arg(&guide_open[..guide_end]);
    let base_args = guide_open[guide_end + 1..].trim_start().strip_prefix(',')?;
    let base_end = find_matching_field_paren(base_args)?;
    let text = clean_eq_distributed_arg(&base_args[..base_end]);
    if guide.is_empty() || text.is_empty() {
        return None;
    }
    Some(RunFieldKind::EqRuby {
        text,
        guide,
        align: jc_align.or(switch_align),
        size_half_points,
        raise_points,
    })
}

/// The enclosures Word's 囲い文字 offers.
const EQ_ENCLOSURES: [&str; 6] = ["○", "◯", "〇", "□", "△", "◇"];

/// Extracts Word's 囲い文字: `EQ \o\ac(enclosure,character)` (ECMA-376
/// §17.16.5.22), either argument the enclosure.
fn parse_eq_enclosed_field(instr: &str) -> Option<RunFieldKind> {
    let trimmed = instr.trim();
    let (name, rest) = trimmed.split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("EQ") {
        return None;
    }
    let lower = rest.to_ascii_lowercase();
    let overstrike = lower.find("\\o")?;
    let after = &rest[overstrike + 2..];
    let after = match after.get(..2) {
        Some(align) if align.eq_ignore_ascii_case("\\a") => after.get(3..)?,
        _ => after,
    };
    let args = after.trim_start().strip_prefix('(')?;
    let end = find_matching_field_paren(args)?;
    let (first, second) = split_eq_distributed_args(&args[..end])?;
    let (enclosure, text) = if EQ_ENCLOSURES.contains(&first.as_str()) {
        (first, second)
    } else if EQ_ENCLOSURES.contains(&second.as_str()) {
        (second, first)
    } else {
        return None;
    };
    Some(RunFieldKind::EqEnclosed { text, enclosure })
}

fn find_matching_field_paren(text: &str) -> Option<usize> {
    let mut in_quote = false;
    for (idx, ch) in text.char_indices() {
        match ch {
            '"' => in_quote = !in_quote,
            ')' if !in_quote => return Some(idx),
            _ => {}
        }
    }
    None
}

fn split_eq_distributed_args(args: &str) -> Option<(String, String)> {
    let mut in_quote = false;
    for (idx, ch) in args.char_indices() {
        match ch {
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                let text = clean_eq_distributed_arg(&args[..idx]);
                let guide = clean_eq_distributed_arg(&args[idx + ch.len_utf8()..]);
                if text.is_empty() || guide.is_empty() {
                    return None;
                }
                return Some((text, guide));
            }
            _ => {}
        }
    }
    None
}

fn clean_eq_distributed_arg(arg: &str) -> String {
    let trimmed = arg.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Returns `true` when a field instruction's leading (non-switch) token is
/// `TOC` (case-insensitive). The `TOC` field (ECMA-376 Part 1 §17.16.5.68) is
/// not mapped to a [`RunFieldKind`] because its result is generated body
/// content that must keep rendering; this detects it so its switches can be
/// captured separately.
fn field_instruction_is_toc(instr: &str) -> bool {
    instr
        .split_whitespace()
        .map(|t| t.trim_matches('"'))
        .find(|t| !t.is_empty() && !t.starts_with('\\'))
        .is_some_and(|name| name.eq_ignore_ascii_case("TOC"))
}

/// Extracts the `\@ "picture"` date/time format switch from a field
/// instruction (ECMA-376 Part 1 §17.16.4.1). The picture is usually quoted and
/// may contain spaces (e.g. `\@ "dddd, MMMM d, yyyy"`); an unquoted single
/// token (`\@ yyyy`) is also accepted. Returns `None` when no `\@` switch is
/// present.
fn extract_field_format_switch(instr: &str) -> Option<String> {
    let bytes = instr.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'@' {
            let mut j = i + 2;
            // Skip whitespace between the switch and its argument.
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() {
                return None;
            }
            if bytes[j] == b'"' {
                // Quoted picture: read until the closing quote.
                let start = j + 1;
                let mut k = start;
                while k < bytes.len() && bytes[k] != b'"' {
                    k += 1;
                }
                let picture = instr[start..k].trim();
                return (!picture.is_empty()).then(|| picture.to_string());
            }
            // Unquoted picture: a single whitespace-delimited token.
            let start = j;
            let mut k = start;
            while k < bytes.len() && !bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            let picture = &instr[start..k];
            return (!picture.is_empty()).then(|| picture.to_string());
        }
        i += 1;
    }
    None
}

fn parse_on_off(value: Option<String>, default: bool) -> bool {
    match value.as_deref().map(|v| v.to_ascii_lowercase()).as_deref() {
        None => default,
        Some("1") | Some("true") | Some("on") | Some("single") => true,
        Some("0") | Some("false") | Some("off") | Some("none") => false,
        Some(other) => !other.is_empty(),
    }
}

/// Normalizes a `w:em` (emphasis mark, ECMA-376 Part 1 §17.3.2.12) `w:val`
/// token. Returns the lowercased token (e.g. "dot" / "comma" / "circle" /
/// "underdot") or `None` when the attribute is missing, empty, or "none"
/// (the explicit "no mark" value).
fn normalize_emphasis_mark(value: Option<String>) -> Option<String> {
    let token = value?.trim().to_ascii_lowercase();
    if token.is_empty() || token == "none" {
        None
    } else {
        Some(token)
    }
}

/// Normalizes a `w:effect` (animated text effect, ECMA-376 Part 1 §17.3.2.10,
/// ST_TextEffect) `w:val` token. Returns the lowercased token (e.g.
/// "blinkbackground" / "lights" / "antsblack" / "antsred" / "shimmer" /
/// "sparkle") or `None` when the attribute is missing, empty, or "none" (the
/// explicit "no effect" value).
fn normalize_text_effect(value: Option<String>) -> Option<String> {
    let token = value?.trim().to_ascii_lowercase();
    if token.is_empty() || token == "none" {
        None
    } else {
        Some(token)
    }
}

/// Parsed `w:u` (§17.3.2.40): the on/off resolution plus the underline kind
/// and color the run declares. `kind` stays `None` for plain "single" so the
/// common case round-trips without extra tokens. A `w:u` without `w:val`
/// keeps the on/off unspecified, matching the previous behavior.
struct ParsedUnderline {
    on_off: Option<bool>,
    kind: Option<String>,
    color: Option<String>,
}

fn parse_underline(attributes: Attributes<'_>) -> ParsedUnderline {
    let mut value = None;
    let mut color = None;

    for attr in attributes.flatten() {
        match attr.key.as_ref() {
            b"w:val" => value = Some(String::from_utf8_lossy(&attr.value).to_string()),
            b"w:color" => {
                let token = String::from_utf8_lossy(&attr.value).trim().to_string();
                if !token.is_empty() {
                    color = Some(token);
                }
            }
            _ => {}
        }
    }

    let (on_off, kind) = match value {
        Some(value) => {
            let on = parse_on_off(Some(value.clone()), true) && value != "none";
            let kind =
                if on && value != "single" && value != "1" && value != "true" && value != "on" {
                    Some(value)
                } else {
                    None
                };
            (Some(on), kind)
        }
        None => (None, None),
    };
    ParsedUnderline {
        on_off,
        kind,
        color,
    }
}

fn parse_kerning_points(attributes: Attributes<'_>) -> Option<f32> {
    for attr in attributes.flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                let points = value / 2.0;
                if points.is_finite() && points >= 0.0 {
                    return Some(points);
                }
            }
        }
    }
    None
}

/// Parses the `w:val` of `w:position` (ECMA-376 Part 1 §17.3.2.24). The value
/// is a signed measurement in half-points; positive raises the text above the
/// baseline and negative lowers it. Returns the value in points (half-points /
/// 2). A zero value is treated as "no shift" and yields `None` so empty
/// positions don't bloat the model.
fn parse_text_position_points(attributes: Attributes<'_>) -> Option<f32> {
    for attr in attributes.flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                let points = value / 2.0;
                if points.is_finite() && points != 0.0 {
                    return Some(points);
                }
            }
        }
    }
    None
}

fn parse_text_scale_percent(attributes: Attributes<'_>) -> Option<f32> {
    for attr in attributes.flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                if value.is_finite() && value > 0.0 {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_fit_text(attributes: Attributes<'_>) -> Option<RunFitText> {
    let mut val = None;
    let mut id = None;

    for attr in attributes.flatten() {
        match attr.key.as_ref() {
            b"w:val" => {
                val = String::from_utf8_lossy(&attr.value).parse::<u32>().ok();
            }
            b"w:id" => {
                id = String::from_utf8_lossy(&attr.value).parse::<i32>().ok();
            }
            _ => {}
        }
    }

    val.map(|val| RunFitText { val, id })
}

fn text_event_to_string(event: &quick_xml::events::BytesText<'_>) -> Option<String> {
    let decoded = event.decode().ok()?;
    let unescaped = unescape(decoded.as_ref()).ok()?;
    Some(unescaped.into_owned())
}

fn reference_event_to_string(event: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = event.decode().ok()?;
    unescape(&format!("&{name};"))
        .ok()
        .map(|value| value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::MathElementKind;
    use std::io::Cursor;
    use std::io::Read;
    use std::path::PathBuf;
    use zip::ZipArchive;

    const DOC_PREFIX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
"#;

    const DOC_SUFFIX: &str = r#"
  </w:body>
</w:document>
"#;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    fn parse_document_from_body(body: &str) -> Document {
        let xml = format!("{DOC_PREFIX}{body}{DOC_SUFFIX}");
        let parser = DocumentParser::new();
        parser
            .parse_document_xml(&xml)
            .expect("failed to parse test document")
    }

    fn read_document_xml(path: &std::path::Path) -> Option<String> {
        let file = std::fs::File::open(path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .ok()?
            .read_to_string(&mut xml)
            .ok()?;
        Some(xml)
    }

    #[test]
    fn noteref_fields_keep_results_and_hyperlinks_in_simple_and_complex_forms() {
        let doc = parse_document_from_body(
            r#"
            <w:p><w:fldSimple w:instr=" NOTEREF target \h "><w:r><w:t>VIII</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText> NOTEREF target \H \f </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>VIII</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
        "#,
        );
        for element in &doc.body.as_ref().unwrap().elements {
            let paragraph = element.as_paragraph().unwrap();
            let run = paragraph
                .runs
                .iter()
                .find(|run| run.field.is_some())
                .unwrap();
            assert!(
                matches!(&run.field.as_ref().unwrap().kind, RunFieldKind::NoteRef { name, hyperlink: true, relative_position: false, .. } if name == "target")
            );
            assert_eq!(run.text, "VIII");
            assert_eq!(
                run.hyperlink.as_ref().unwrap().anchor.as_deref(),
                Some("target")
            );
        }
        let field = DocumentParser::parse_field_instruction(r"NOTEREF target \p \f").unwrap();
        assert!(matches!(
            field,
            RunFieldKind::NoteRef {
                hyperlink: false,
                formatted: true,
                relative_position: true,
                ..
            }
        ));
        for instruction in ["NOTEREF", r"NOTEREF \h"] {
            assert_eq!(
                DocumentParser::parse_field_instruction(instruction),
                Some(RunFieldKind::Unsupported {
                    instruction: instruction.into(),
                })
            );
        }
    }

    /// The one-pass entry must hand back exactly the tree a standalone
    /// `DocxXmlDocument::parse` builds: the editor's node ids, paths and the
    /// re-serialized XML all have to agree, for every fixture we ship.
    #[test]
    fn one_pass_raw_tree_matches_standalone_parse_for_every_fixture() {
        let fixtures = std::fs::read_dir(fixture_path("")).expect("fixture directory");
        let mut checked = 0usize;
        for entry in fixtures {
            let path = entry.expect("fixture entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("docx") {
                continue;
            }
            let Some(xml) = read_document_xml(&path) else {
                continue;
            };
            let standalone = match DocxXmlDocument::parse(&xml) {
                Ok(tree) => tree,
                Err(_) => continue,
            };
            let parser = DocumentParser::new();
            let (document, raw) = parser
                .parse_document_xml_with_raw_tree(&xml)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let raw = raw.unwrap_or_else(|err| panic!("{}: {err:?}", path.display()));
            assert_eq!(
                crate::docx_xml::tests::index_snapshot(&raw),
                crate::docx_xml::tests::index_snapshot(&standalone),
                "{}: element ids / paths diverge",
                path.display()
            );
            assert_eq!(
                raw.to_string().expect("serialize"),
                standalone.to_string().expect("serialize"),
                "{}: serialized XML diverges",
                path.display()
            );
            let separate = parser.parse_document_xml(&xml).expect("separate parse");
            assert_eq!(
                format!("{document:?}"),
                format!("{separate:?}"),
                "{}: semantic document diverges",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 50, "only {checked} fixtures were checked");
    }

    #[test]
    fn one_pass_reports_a_malformed_raw_tree_without_failing_the_semantic_parse() {
        // Input truncated before the root closes: the streaming semantic
        // parser keeps what it saw, the raw tree cannot be completed.
        let xml = format!("{DOC_PREFIX}<w:p><w:r><w:t>x</w:t></w:r></w:p>");
        let (document, raw) = DocumentParser::new()
            .parse_document_xml_with_raw_tree(&xml)
            .expect("semantic parse succeeds");
        assert_eq!(document.body.as_ref().map(|b| b.elements.len()), Some(1));
        assert!(matches!(raw, Err(DocxXmlError::Malformed(_))));
    }

    #[test]
    fn preserves_alt_chunk_relationship_and_block_position() {
        let document = parse_document_from_body(
            r#"<w:p><w:r><w:t>before</w:t></w:r></w:p>
<w:altChunk r:id="rId7"/>
<w:p><w:r><w:t>after</w:t></w:r></w:p>"#,
        );

        assert_eq!(
            document.alt_chunks,
            vec![AltChunkReference {
                relationship_id: "rId7".to_string(),
                block_index: 1,
            }]
        );
    }

    fn parse_document_from_fixture(name: &str) -> Document {
        let data = std::fs::read(fixture_path(name)).expect("read docx fixture");
        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor).expect("open docx archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");
        DocumentParser::new()
            .parse_document_xml(&document_xml)
            .expect("parse document.xml")
    }

    fn first_paragraph_props(doc: &Document) -> &crate::document::ParagraphProperties {
        let body = doc.body.as_ref().expect("body");
        match &body.elements[0] {
            BodyElement::Paragraph(p) => p.properties.as_ref().expect("pPr"),
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    /// `w:u` carries a kind and color beyond on/off (§17.3.2.40). The kind
    /// stays `None` for the plain single rule so the common case adds no
    /// tokens, and `w:val="none"` still resolves to underline-off.
    #[test]
    fn parses_underline_kind_and_color() {
        let doc = parse_document_from_body(
            r#"<w:p>
                 <w:r><w:rPr><w:u w:val="double" w:color="FF0000"/></w:rPr><w:t>a</w:t></w:r>
                 <w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>b</w:t></w:r>
                 <w:r><w:rPr><w:u w:val="none"/></w:rPr><w:t>c</w:t></w:r>
                 <w:r><w:rPr><w:u w:val="wave"/></w:rPr><w:t>d</w:t></w:r>
               </w:p>"#,
        );
        let body = doc.body.as_ref().expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p,
            other => panic!("expected paragraph, got {other:?}"),
        };
        let props = |index: usize| paragraph.runs[index].properties.as_ref().expect("rPr");

        assert_eq!(props(0).underline, Some(true));
        assert_eq!(props(0).underline_kind.as_deref(), Some("double"));
        assert_eq!(props(0).underline_color.as_deref(), Some("FF0000"));

        assert_eq!(props(1).underline, Some(true));
        assert_eq!(props(1).underline_kind, None);
        assert_eq!(props(1).underline_color, None);

        assert_eq!(props(2).underline, Some(false));
        assert_eq!(props(2).underline_kind, None);

        assert_eq!(props(3).underline, Some(true));
        assert_eq!(props(3).underline_kind.as_deref(), Some("wave"));
    }

    #[test]
    fn parses_paragraph_w14_para_id() {
        // Local-name matching means the `w14:` prefix need not be declared here.
        let body = r#"
    <w:p w14:paraId="12AB34CD">
      <w:r><w:t>Hi</w:t></w:r>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        assert_eq!(
            first_paragraph_props(&document).para_id.as_deref(),
            Some("12AB34CD")
        );
    }

    #[test]
    fn parses_explicit_rtl_and_ltr_paragraph_direction() {
        let document = parse_document_from_body(
            r#"
    <w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>RTL</w:t></w:r></w:p>
    <w:p><w:pPr><w:bidi w:val="0"/></w:pPr><w:r><w:t>LTR</w:t></w:r></w:p>
"#,
        );
        let body = document.body.as_ref().expect("body");
        let directions = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .map(|paragraph| paragraph.properties.as_ref().and_then(|props| props.bidi))
            .collect::<Vec<_>>();
        assert_eq!(directions, vec![Some(true), Some(false)]);
    }

    #[test]
    fn preserves_logical_alignment_until_paragraph_direction_is_resolved() {
        let document = parse_document_from_body(
            r#"
    <w:p><w:pPr><w:jc w:val="start"/><w:bidi/></w:pPr><w:r><w:t>RTL start</w:t></w:r></w:p>
    <w:tbl><w:tr><w:tc>
      <w:p><w:pPr><w:jc w:val="end"></w:jc><w:bidi w:val="1"></w:bidi></w:pPr><w:r><w:t>RTL end</w:t></w:r></w:p>
      <w:p><w:pPr><w:jc w:val="start"/><w:bidi w:val="0"/></w:pPr><w:r><w:t>LTR start</w:t></w:r></w:p>
    </w:tc></w:tr></w:tbl>
"#,
        );
        let body = document.body.as_ref().expect("body");
        let BodyElement::Table(table) = &body.elements[1] else {
            panic!("expected table");
        };
        let paragraphs = std::iter::once(&body.elements[0])
            .chain(table.rows[0].cells[0].blocks.iter())
            .filter_map(BodyElement::as_paragraph)
            .map(|paragraph| {
                let props = paragraph.properties.as_ref().expect("pPr");
                (props.alignment.as_deref(), props.bidi)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paragraphs,
            vec![
                (Some("start"), Some(true)),
                (Some("end"), Some(true)),
                (Some("start"), Some(false))
            ]
        );
    }

    #[test]
    fn paragraph_without_para_id_has_none() {
        let body = r#"<w:p><w:r><w:t>Hi</w:t></w:r></w:p>"#;
        let document = parse_document_from_body(body);
        let para_id = match &document.body.as_ref().unwrap().elements[0] {
            BodyElement::Paragraph(p) => p.properties.as_ref().and_then(|pp| pp.para_id.clone()),
            other => panic!("expected paragraph, got {other:?}"),
        };
        assert_eq!(para_id, None);
    }

    #[test]
    fn parses_paragraph_shading_clear_with_fill() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/>
      </w:pPr>
      <w:r><w:t>Hello</w:t></w:r>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let shading = first_paragraph_props(&document)
            .shading
            .as_ref()
            .expect("paragraph shading");
        assert_eq!(shading.val.as_deref(), Some("clear"));
        assert_eq!(shading.color, None, "w:color=auto maps to None");
        assert_eq!(shading.fill.as_deref(), Some("#FFFF00"));
    }

    #[test]
    fn parses_paragraph_shading_pattern_with_color() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:shd w:val="pct25" w:color="FF0000" w:fill="00FF00"/>
      </w:pPr>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let shading = first_paragraph_props(&document)
            .shading
            .as_ref()
            .expect("paragraph shading");
        assert_eq!(shading.val.as_deref(), Some("pct25"));
        assert_eq!(shading.color.as_deref(), Some("#FF0000"));
        assert_eq!(shading.fill.as_deref(), Some("#00FF00"));
    }

    #[test]
    fn paragraph_without_shading_has_none() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:jc w:val="center"/>
      </w:pPr>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        assert!(first_paragraph_props(&document).shading.is_none());
    }

    // A color attribute can be absent, the `auto` sentinel, or a 6-digit hex.
    // `auto` and absence both normalize to `None`; hex normalizes to `#RRGGBB`.
    fn color_attr_strategy(
    ) -> impl proptest::strategy::Strategy<Value = (Option<String>, Option<String>)> {
        use proptest::prelude::*;
        prop_oneof![
            Just((None, None)),
            Just((Some("auto".to_string()), None)),
            "[0-9A-F]{6}".prop_map(|hex| (Some(hex.clone()), Some(format!("#{hex}")))),
        ]
    }

    proptest::proptest! {
        // Round-trip property: any `w:shd` we can express in OOXML must parse to
        // the structurally-equal normalized model. Covers the `auto`→None and
        // hex→`#RRGGBB` normalization, and the all-empty → no-shading case.
        #[test]
        fn paragraph_shading_round_trips(
            val in proptest::option::of("clear|solid|pct10|pct25|pct50|horzStripe|diagStripe"),
            color in color_attr_strategy(),
            fill in color_attr_strategy(),
        ) {
            let (color_attr, expected_color) = color;
            let (fill_attr, expected_fill) = fill;

            let mut attrs = String::new();
            if let Some(v) = &val {
                attrs.push_str(&format!(" w:val=\"{v}\""));
            }
            if let Some(c) = &color_attr {
                attrs.push_str(&format!(" w:color=\"{c}\""));
            }
            if let Some(f) = &fill_attr {
                attrs.push_str(&format!(" w:fill=\"{f}\""));
            }
            let body = format!("<w:p><w:pPr><w:shd{attrs}/></w:pPr></w:p>");
            let document = parse_document_from_body(&body);

            let expected = Shading {
                val: val.clone(),
                color: expected_color,
                fill: expected_fill,
            };

            let actual = match &document.body.as_ref().unwrap().elements[0] {
                BodyElement::Paragraph(p) => {
                    p.properties.as_ref().and_then(|pr| pr.shading.clone())
                }
                _ => None,
            };

            if expected.is_empty() {
                // No meaningful shading attributes → props (and thus shading)
                // are dropped by `paragraph_props_is_empty`.
                proptest::prop_assert_eq!(actual, None);
            } else {
                proptest::prop_assert_eq!(actual, Some(expected));
            }
        }
    }

    #[test]
    fn parses_paragraph_borders_all_sides() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:pBdr>
          <w:top w:val="single" w:sz="8" w:space="1" w:color="FF0000"/>
          <w:bottom w:val="double" w:sz="12" w:space="4" w:color="auto"/>
          <w:left w:val="dashed" w:sz="4" w:space="0" w:color="00FF00"/>
          <w:right w:val="dotted"/>
        </w:pBdr>
      </w:pPr>
      <w:r><w:t>Boxed</w:t></w:r>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let borders = first_paragraph_props(&document)
            .borders
            .as_ref()
            .expect("paragraph borders");

        let top = borders.top.as_ref().expect("top");
        assert_eq!(top.val.as_deref(), Some("single"));
        assert_eq!(top.size_eighths, Some(8));
        assert_eq!(top.space, Some(1));
        assert_eq!(top.color.as_deref(), Some("#FF0000"));

        let bottom = borders.bottom.as_ref().expect("bottom");
        assert_eq!(bottom.val.as_deref(), Some("double"));
        assert_eq!(bottom.size_eighths, Some(12));
        assert_eq!(bottom.color, None, "w:color=auto maps to None");

        let left = borders.left.as_ref().expect("left");
        assert_eq!(left.val.as_deref(), Some("dashed"));
        assert_eq!(left.color.as_deref(), Some("#00FF00"));

        let right = borders.right.as_ref().expect("right");
        assert_eq!(right.val.as_deref(), Some("dotted"));
        assert_eq!(right.size_eighths, None);
        assert!(borders.between.is_none() && borders.bar.is_none());
    }

    #[test]
    fn parses_paragraph_borders_between_and_bar() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:pBdr>
          <w:between w:val="single" w:sz="6"/>
          <w:bar w:val="single" w:sz="6"/>
        </w:pBdr>
      </w:pPr>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let borders = first_paragraph_props(&document)
            .borders
            .as_ref()
            .expect("paragraph borders");
        assert!(borders.between.is_some());
        assert!(borders.bar.is_some());
        assert!(borders.top.is_none());
    }

    #[test]
    fn lowercases_border_val_token() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:pBdr>
          <w:top w:val="DotDash"/>
        </w:pBdr>
      </w:pPr>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let borders = first_paragraph_props(&document)
            .borders
            .as_ref()
            .expect("paragraph borders");
        assert_eq!(
            borders.top.as_ref().unwrap().val.as_deref(),
            Some("dotdash")
        );
    }

    #[test]
    fn paragraph_without_borders_has_none() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:jc w:val="center"/>
      </w:pPr>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        assert!(first_paragraph_props(&document).borders.is_none());
    }

    proptest::proptest! {
        // Round-trip property for a single `w:pBdr` edge: any (val, sz, space,
        // color) we can express must parse back to the normalized model — val
        // lowercased, color `auto`/absent → None / hex → `#RRGGBB`, numeric
        // attributes preserved. An edge with no attributes at all still
        // produces an (empty) edge entry because the element is present.
        #[test]
        fn paragraph_border_edge_round_trips(
            val in proptest::option::of("single|double|dashed|dotted|DotDash|thick|wave"),
            sz in proptest::option::of(0u32..=96),
            space in proptest::option::of(0u32..=31),
            color in color_attr_strategy(),
        ) {
            let (color_attr, expected_color) = color;

            let mut attrs = String::new();
            if let Some(v) = &val {
                attrs.push_str(&format!(" w:val=\"{v}\""));
            }
            if let Some(s) = &sz {
                attrs.push_str(&format!(" w:sz=\"{s}\""));
            }
            if let Some(sp) = &space {
                attrs.push_str(&format!(" w:space=\"{sp}\""));
            }
            if let Some(c) = &color_attr {
                attrs.push_str(&format!(" w:color=\"{c}\""));
            }
            let body = format!(
                "<w:p><w:pPr><w:pBdr><w:top{attrs}/></w:pBdr></w:pPr></w:p>"
            );
            let document = parse_document_from_body(&body);

            let expected = crate::document::BorderEdge {
                val: val.as_ref().map(|v| v.to_ascii_lowercase()),
                size_eighths: sz,
                space,
                color: expected_color,
            };

            let actual = match &document.body.as_ref().unwrap().elements[0] {
                BodyElement::Paragraph(p) => p
                    .properties
                    .as_ref()
                    .and_then(|pr| pr.borders.as_ref())
                    .and_then(|b| b.top.clone()),
                _ => None,
            };

            proptest::prop_assert_eq!(actual, Some(expected));
        }
    }

    #[test]
    fn preserves_self_closing_empty_paragraphs() {
        let body = r#"
    <w:p>
      <w:r><w:t>Hello</w:t></w:r>
    </w:p>
    <w:p/>
    <w:p/>
    <w:p>
      <w:r><w:t>world</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 4);

        let second = match &body.elements[1] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let third = match &body.elements[2] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert!(second.runs.is_empty());
        assert!(third.runs.is_empty());
    }

    #[test]
    fn merged_table_over_page_fixture_preserves_blank_paragraphs_before_table() {
        let document = parse_document_from_fixture("merged-table-over-page.docx");
        let body = document.body.expect("body");
        let table_index = body
            .elements
            .iter()
            .position(|element| matches!(element, BodyElement::Table(_)))
            .expect("table");

        assert_eq!(table_index, 34);

        let blank_paragraphs_before_table = body.elements[..table_index]
            .iter()
            .filter(|element| match element {
                BodyElement::Paragraph(paragraph) => paragraph.runs.is_empty(),
                _ => false,
            })
            .count();

        assert_eq!(blank_paragraphs_before_table, 33);
    }

    #[test]
    fn tracks_comment_ranges_and_markers() {
        let body = r#"
    <w:p>
      <w:r><w:t>Hel</w:t></w:r>
      <w:commentRangeStart w:id="1"/>
      <w:r><w:t>lo</w:t></w:r>
      <w:commentRangeEnd w:id="1"/>
      <w:r><w:commentReference w:id="1"/></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.comment_ranges.len(), 1);
        let range = &paragraph.comment_ranges[0];
        assert_eq!(range.id, "1");
        assert_eq!(range.start, 3);
        assert_eq!(range.end, 5);

        assert_eq!(paragraph.comment_markers.len(), 1);
        let marker = &paragraph.comment_markers[0];
        assert_eq!(marker.id, "1");
        assert_eq!(marker.position, 5);

        let reference_run = paragraph
            .runs
            .iter()
            .find(|run| !run.comment_reference_ids.is_empty())
            .expect("comment reference run");
        assert_eq!(reference_run.comment_reference_ids, vec!["1"]);
    }

    #[test]
    fn tracks_comment_range_spanning_multiple_paragraphs() {
        // Comment id "7" starts mid-paragraph 0, runs through paragraph 1
        // entirely, and ends mid-paragraph 2 where the reference marker lives.
        let body = r#"
    <w:p>
      <w:r><w:t>AB</w:t></w:r>
      <w:commentRangeStart w:id="7"/>
      <w:r><w:t>CD</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>EFG</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>HI</w:t></w:r>
      <w:commentRangeEnd w:id="7"/>
      <w:r><w:commentReference w:id="7"/></w:r>
      <w:r><w:t>JK</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        // Paragraph 0: range starts after "AB" and runs to the end ("ABCD").
        let p0 = paragraph(0);
        assert_eq!(p0.comment_ranges.len(), 1);
        assert_eq!(p0.comment_ranges[0].id, "7");
        assert_eq!(p0.comment_ranges[0].start, 2);
        assert_eq!(p0.comment_ranges[0].end, 4);

        // Paragraph 1: fully covered (0.."EFG".len()).
        let p1 = paragraph(1);
        assert_eq!(p1.comment_ranges.len(), 1);
        assert_eq!(p1.comment_ranges[0].id, "7");
        assert_eq!(p1.comment_ranges[0].start, 0);
        assert_eq!(p1.comment_ranges[0].end, 3);

        // Paragraph 2: covered from the start up to the end marker (after "HI").
        let p2 = paragraph(2);
        assert_eq!(p2.comment_ranges.len(), 1);
        assert_eq!(p2.comment_ranges[0].id, "7");
        assert_eq!(p2.comment_ranges[0].start, 0);
        assert_eq!(p2.comment_ranges[0].end, 2);
        // The reference marker only exists in the closing paragraph.
        assert_eq!(p2.comment_markers.len(), 1);
        assert_eq!(p2.comment_markers[0].id, "7");
        assert_eq!(p2.comment_markers[0].position, 2);
        assert!(p0.comment_markers.is_empty());
        assert!(p1.comment_markers.is_empty());
    }

    #[test]
    fn parses_text_wrapping_break_as_soft_break_run() {
        let body = r#"
    <w:p>
      <w:r>
        <w:t>Hello</w:t>
      </w:r>
      <w:r>
        <w:br w:type="textWrapping"/>
      </w:r>
      <w:r>
        <w:t>world</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 3);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, "\n");
        assert_eq!(paragraph.runs[2].text, "world");
    }

    #[test]
    fn line_feed_inside_text_reads_as_a_space() {
        // word-compat-comprehensive: Word sets `Part<TAB>One<LF>Overview` in
        // a w:t on one line with a space for the line feed; only w:br and
        // w:cr break the line.
        let body =
            "<w:p><w:r><w:t xml:space=\"preserve\">Part\tOne\nOverview\r\nEnd</w:t></w:r></w:p>";

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 1);
        assert_eq!(paragraph.runs[0].text, "Part\tOne Overview End");
    }

    #[test]
    fn attributed_text_wrapping_break_keeps_raw_run_mapping_shape() {
        // `w:clear` changes where Word resumes text around floating objects.
        // Keeping this as one model run lets the editor map the run back to the
        // original raw `w:r` and preserve the attributed break during edits.
        let body = r#"
    <w:p>
      <w:r>
        <w:t>Above</w:t>
        <w:br w:type="textWrapping" w:clear="all"/>
        <w:t>Below</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 1);
        assert_eq!(paragraph.runs[0].text, "Above\nBelow");
    }

    #[test]
    fn parses_carriage_return_as_soft_break_without_splitting_run() {
        let body = r#"
    <w:p>
      <w:r>
        <w:t>Hello</w:t>
        <w:cr/>
        <w:t>world</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 1);
        assert_eq!(paragraph.runs[0].text, "Hello\nworld");
    }

    #[test]
    fn parses_page_break_as_control_character() {
        let body = r#"
    <w:p>
      <w:r>
        <w:t>Hello</w:t>
      </w:r>
      <w:r>
        <w:br w:type="page"/>
      </w:r>
      <w:r>
        <w:t>world</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 3);
        assert_eq!(paragraph.runs[0].text, "Hello");
        assert_eq!(paragraph.runs[1].text, PAGE_BREAK_STR);
        assert_eq!(paragraph.runs[2].text, "world");
    }

    #[test]
    fn parses_run_fonts_attributes() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr>
          <w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="ＭＳ 明朝" w:cs="Times New Roman" w:hint="eastAsia"/>
        </w:rPr>
        <w:t>Hello</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");
        let fonts = props.fonts.as_ref().expect("run fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Calibri"));
        assert_eq!(fonts.east_asia.as_deref(), Some("ＭＳ 明朝"));
        assert_eq!(fonts.complex.as_deref(), Some("Times New Roman"));
        assert_eq!(fonts.hint, Some(FontHint::EastAsia));
        assert_eq!(props.font_family.as_deref(), Some("Calibri"));
    }

    #[test]
    fn authored_run_fonts_preserve_omitted_and_explicit_hint_provenance() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="Arial" w:eastAsia="ＭＳ 明朝"/></w:rPr>
        <w:t>第1項</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="Arial" w:hint="default"/></w:rPr>
        <w:t>第2項</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let paragraph = document
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let omitted_props = paragraph.runs[0]
            .properties
            .as_ref()
            .expect("omitted props");
        let omitted = omitted_props
            .fonts
            .as_ref()
            .expect("omitted-hint run fonts");
        let explicit_default_props = paragraph.runs[1]
            .properties
            .as_ref()
            .expect("default props");
        let explicit_default = explicit_default_props
            .fonts
            .as_ref()
            .expect("default-hint run fonts");

        assert_eq!(omitted.hint, None);
        assert_eq!(explicit_default.hint, Some(FontHint::Default));
        assert_eq!(omitted_props.authored_east_asia_font_hint, Some(false));
        assert_eq!(
            explicit_default_props.authored_east_asia_font_hint,
            Some(false)
        );
        assert_eq!(omitted_props.authored_east_asia_font_name, Some(true));
        assert_eq!(
            explicit_default_props.authored_east_asia_font_name,
            Some(false)
        );
    }

    #[test]
    fn parses_paragraph_tab_stops() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:tabs>
          <w:tab w:val="left" w:pos="720"/>
          <w:tab w:val="right" w:pos="1440" w:leader="dot"/>
        </w:tabs>
      </w:pPr>
      <w:r><w:t>Foo</w:t></w:r>
      <w:r><w:tab/></w:r>
      <w:r><w:t>Bar</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let properties = paragraph.properties.as_ref().expect("paragraph properties");
        assert_eq!(properties.tabs.len(), 2);

        let first = &properties.tabs[0];
        assert_eq!(first.kind.as_deref(), Some("left"));
        assert!((first.position.unwrap_or_default() - 36.0).abs() < f32::EPSILON);
        assert_eq!(first.leader.as_deref(), None);

        let second = &properties.tabs[1];
        assert_eq!(second.kind.as_deref(), Some("right"));
        assert!((second.position.unwrap_or_default() - 72.0).abs() < f32::EPSILON);
        assert_eq!(second.leader.as_deref(), Some("dot"));
    }

    #[test]
    fn parses_numbering_from_empty_tags() {
        let paragraph = r#"
    <w:p>
      <w:pPr>
        <w:numPr>
          <w:ilvl w:val="2"/>
          <w:numId w:val="10"/>
        </w:numPr>
      </w:pPr>
      <w:r><w:t>Item</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(paragraph);

        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 1);
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let numbering = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.numbering.as_ref())
            .expect("numbering");

        assert_eq!(numbering.id.as_deref(), Some("10"));
        assert_eq!(numbering.level, Some(2));
    }

    #[test]
    fn parses_adjust_right_indent_with_explicit_off_and_inherited_absence() {
        let document = parse_document_from_body(
            r#"
            <w:p><w:pPr><w:adjustRightInd/></w:pPr><w:r><w:t>on</w:t></w:r></w:p>
            <w:p><w:pPr><w:adjustRightInd w:val="0"/></w:pPr><w:r><w:t>off</w:t></w:r></w:p>
            <w:p><w:pPr><w:adjustRightInd w:val="false"></w:adjustRightInd></w:pPr><w:r><w:t>long off</w:t></w:r></w:p>
            <w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:t>inherited</w:t></w:r></w:p>
        "#,
        );
        let body = document.body.unwrap();
        let values: Vec<_> = body
            .elements
            .iter()
            .map(|element| {
                let p = element.as_paragraph().unwrap();
                serde_json::to_value(p.properties.as_ref()).unwrap()["adjust_right_indent"].clone()
            })
            .collect();
        assert_eq!(
            values,
            vec![
                serde_json::json!(true),
                serde_json::json!(false),
                serde_json::json!(false),
                serde_json::Value::Null
            ]
        );
    }

    #[test]
    fn parses_paragraph_widow_control_flags() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:widowControl w:val="0"/>
      </w:pPr>
      <w:r><w:t>no widow</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:widowControl/>
      </w:pPr>
      <w:r><w:t>widow on</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 2);

        let first = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let second = match &body.elements[1] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            first
                .properties
                .as_ref()
                .and_then(|props| props.widow_control),
            Some(false)
        );
        assert_eq!(
            second
                .properties
                .as_ref()
                .and_then(|props| props.widow_control),
            Some(true)
        );
    }

    #[test]
    fn parses_paragraph_contextual_spacing_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:contextualSpacing/>
      </w:pPr>
      <w:r><w:t>item 1</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:contextualSpacing w:val="false"/>
      </w:pPr>
      <w:r><w:t>item 2</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>plain</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let contextual = |element: &BodyElement| match element {
            BodyElement::Paragraph(p) => p
                .properties
                .as_ref()
                .and_then(|props| props.contextual_spacing),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(contextual(&body.elements[0]), Some(true));
        assert_eq!(contextual(&body.elements[1]), Some(false));
        assert_eq!(contextual(&body.elements[2]), None);
    }

    #[test]
    fn parses_paragraph_page_break_before_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:pageBreakBefore/>
      </w:pPr>
      <w:r><w:t>break on</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:pageBreakBefore w:val="false"/>
      </w:pPr>
      <w:r><w:t>break off</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>no pPr</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let para = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        // Self-closing element toggles the flag on.
        assert_eq!(
            para(0)
                .properties
                .as_ref()
                .and_then(|props| props.page_break_before),
            Some(true)
        );
        // `w:val="false"` disables it (a pPr containing only this must survive).
        assert_eq!(
            para(1)
                .properties
                .as_ref()
                .and_then(|props| props.page_break_before),
            Some(false)
        );
        // Absent → None.
        assert_eq!(
            para(2)
                .properties
                .as_ref()
                .and_then(|props| props.page_break_before),
            None
        );
    }

    #[test]
    fn parses_drop_and_margin_caps_with_safe_line_counts() {
        let body = r#"
    <w:p>
      <w:pPr><w:framePr w:dropCap="drop" w:lines="3" w:wrap="around"/></w:pPr>
      <w:r><w:t>Alpha</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:framePr w:dropCap="margin" w:lines="999"/></w:pPr>
      <w:r><w:t>Beta</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:framePr w:dropCap="none" w:lines="4"/></w:pPr>
      <w:r><w:t>Gamma</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let drop_cap = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(paragraph) => paragraph
                .properties
                .as_ref()
                .and_then(|props| props.drop_cap),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(
            drop_cap(0),
            Some(crate::document::DropCap {
                kind: crate::document::DropCapKind::Drop,
                lines: 3,
            })
        );
        assert_eq!(
            drop_cap(1),
            Some(crate::document::DropCap {
                kind: crate::document::DropCapKind::Margin,
                lines: 10,
            })
        );
        assert_eq!(drop_cap(2), None);
        let frame = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(paragraph) => {
                paragraph.properties.as_ref().and_then(|props| props.frame)
            }
            _ => panic!("expected paragraph"),
        };
        assert_eq!(frame(0), None);
        assert_eq!(frame(2), None);
    }

    #[test]
    fn parses_positioned_paragraph_frames() {
        use crate::document::{
            FrameAlign, FrameAnchor, FrameHeightRule, FrameWrap, ParagraphFrame,
        };
        let body = r#"
    <w:p>
      <w:pPr><w:framePr w:w="2880" w:h="1440" w:hRule="exact" w:wrap="around"
        w:vAnchor="text" w:hAnchor="page" w:x="2161" w:y="1" w:hSpace="180" w:vSpace="90"/></w:pPr>
      <w:r><w:t>Framed</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:framePr w:h="500" w:xAlign="right" w:yAlign="bottom" w:hAnchor="margin"
        w:vAnchor="margin" w:wrap="notBeside"/></w:pPr>
      <w:r><w:t>Aligned</w:t></w:r>
    </w:p>
    "#;
        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let frame = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(paragraph) => {
                paragraph.properties.as_ref().and_then(|props| props.frame)
            }
            _ => panic!("expected paragraph"),
        };
        assert_eq!(
            frame(0),
            Some(ParagraphFrame {
                width_twips: Some(2880),
                height_twips: Some(1440),
                height_rule: FrameHeightRule::Exact,
                x_twips: Some(2161),
                y_twips: Some(1),
                x_align: None,
                y_align: None,
                h_anchor: Some(FrameAnchor::Page),
                v_anchor: Some(FrameAnchor::Text),
                wrap: Some(FrameWrap::Around),
                h_space_twips: Some(180),
                v_space_twips: Some(90),
            })
        );
        let aligned = frame(1).expect("aligned frame");
        assert_eq!(aligned.height_rule, FrameHeightRule::AtLeast);
        assert_eq!(aligned.x_align, Some(FrameAlign::End));
        assert_eq!(aligned.y_align, Some(FrameAlign::End));
        assert_eq!(aligned.h_anchor, Some(FrameAnchor::Margin));
        assert_eq!(aligned.wrap, Some(FrameWrap::NotBeside));
    }

    #[test]
    fn parses_paragraph_text_alignment() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:textAlignment w:val="Center"/>
      </w:pPr>
      <w:r><w:t>centered</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:textAlignment w:val="bottom"/>
      </w:pPr>
      <w:r><w:t>bottom</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>none</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let paragraph = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        // `w:val` is normalized to lowercase.
        assert_eq!(
            paragraph(0)
                .properties
                .as_ref()
                .and_then(|props| props.text_alignment.as_deref()),
            Some("center")
        );
        assert_eq!(
            paragraph(1)
                .properties
                .as_ref()
                .and_then(|props| props.text_alignment.as_deref()),
            Some("bottom")
        );
        // Absent element leaves the field unset.
        assert_eq!(
            paragraph(2)
                .properties
                .as_ref()
                .and_then(|props| props.text_alignment.as_deref()),
            None
        );
    }

    #[test]
    fn parses_paragraph_suppress_line_numbers_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:suppressLineNumbers/>
      </w:pPr>
      <w:r><w:t>not numbered</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:suppressLineNumbers w:val="false"/>
      </w:pPr>
      <w:r><w:t>numbered</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>default</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let suppressed = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let explicit_off = match &body.elements[1] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let default = match &body.elements[2] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            suppressed
                .properties
                .as_ref()
                .and_then(|props| props.suppress_line_numbers),
            Some(true)
        );
        assert_eq!(
            explicit_off
                .properties
                .as_ref()
                .and_then(|props| props.suppress_line_numbers),
            Some(false)
        );
        assert_eq!(
            default
                .properties
                .as_ref()
                .and_then(|props| props.suppress_line_numbers),
            None
        );
    }

    #[test]
    fn parses_paragraph_keep_lines_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:keepLines/>
      </w:pPr>
      <w:r><w:t>keep on</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:keepLines w:val="false"/>
      </w:pPr>
      <w:r><w:t>keep off</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr/>
      <w:r><w:t>absent</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let keep_lines = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => p.properties.as_ref().and_then(|props| props.keep_lines),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(keep_lines(0), Some(true));
        assert_eq!(keep_lines(1), Some(false));
        assert_eq!(keep_lines(2), None);
    }

    #[test]
    fn parses_paragraph_keep_next_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:keepNext/>
      </w:pPr>
      <w:r><w:t>keep with next</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:keepNext w:val="false"/>
      </w:pPr>
      <w:r><w:t>independent</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr/>
      <w:r><w:t>absent</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let keep_next = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => p.properties.as_ref().and_then(|props| props.keep_next),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(keep_next(0), Some(true));
        assert_eq!(keep_next(1), Some(false));
        assert_eq!(keep_next(2), None);
    }

    #[test]
    fn parses_paragraph_mirror_indents_flags() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:mirrorIndents/>
      </w:pPr>
      <w:r><w:t>mirror on</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:mirrorIndents w:val="false"/>
      </w:pPr>
      <w:r><w:t>mirror off</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr/>
      <w:r><w:t>absent</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let mirror_of = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => p
                .as_ref()
                .properties
                .as_ref()
                .and_then(|props| props.mirror_indents),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(mirror_of(0), Some(true));
        assert_eq!(mirror_of(1), Some(false));
        assert_eq!(mirror_of(2), None);
    }

    #[test]
    fn parses_char_unit_first_line_and_hanging_indents() {
        // `w:firstLineChars`/`w:hangingChars` (ECMA-376 §17.3.1.12) were
        // previously dropped, so character-unit special indents vanished.
        let body = r#"
    <w:p>
      <w:pPr><w:ind w:firstLineChars="100" w:hangingChars="200"/></w:pPr>
      <w:r><w:t>chars</w:t></w:r>
    </w:p>
    "#;
        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let indent = match &body.elements[0] {
            BodyElement::Paragraph(p) => p
                .as_ref()
                .properties
                .as_ref()
                .and_then(|props| props.indent.clone())
                .expect("indent"),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(indent.first_line_chars, Some(100));
        assert_eq!(indent.hanging_chars, Some(200));
    }

    #[test]
    fn parses_complex_script_font_size_independently() {
        // `w:szCs` (§17.3.2.39) sizes complex-script glyphs; `w:sz` keeps
        // sizing Latin/East-Asian glyphs. szCs alone leaves the size to the
        // styles, as Word for Mac draws it.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:sz w:val="20"/><w:szCs w:val="28"/></w:rPr>
        <w:t>both sizes</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:szCs w:val="28"/></w:rPr>
        <w:t>complex only</w:t>
      </w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let both = paragraph.runs[0].properties.as_ref().expect("run props");
        assert_eq!(both.font_size, Some(10.0));
        assert_eq!(both.font_size_complex, Some(14.0));

        let complex_only = paragraph.runs[1].properties.as_ref().expect("run props");
        assert_eq!(complex_only.font_size_complex, Some(14.0));
        assert_eq!(complex_only.font_size, None, "szCs alone sets no size");
    }

    #[test]
    fn parses_logical_paragraph_indents_for_both_directions() {
        let body = r#"
    <w:p>
      <w:pPr><w:ind w:start="720" w:end="360" w:startChars="200" w:endChars="100"/></w:pPr>
      <w:r><w:t>ltr</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:bidi/><w:ind w:start="720" w:end="360" w:startChars="200" w:endChars="100"/></w:pPr>
      <w:r><w:t>rtl</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let indent_of = |index: usize| match &body.elements[index] {
            BodyElement::Paragraph(paragraph) => paragraph
                .properties
                .as_ref()
                .and_then(|properties| properties.indent.as_ref())
                .expect("indent"),
            _ => panic!("expected paragraph"),
        };

        let ltr = indent_of(0);
        assert_eq!(ltr.physical_left(false), Some(36.0));
        assert_eq!(ltr.physical_right(false), Some(18.0));
        assert_eq!(ltr.physical_left_chars(false), Some(200));
        assert_eq!(ltr.physical_right_chars(false), Some(100));

        let rtl = indent_of(1);
        assert_eq!(rtl.physical_left(true), Some(18.0));
        assert_eq!(rtl.physical_right(true), Some(36.0));
        assert_eq!(rtl.physical_left_chars(true), Some(100));
        assert_eq!(rtl.physical_right_chars(true), Some(200));
    }

    #[test]
    fn parses_paragraph_outline_level() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:outlineLvl w:val="0"/>
      </w:pPr>
      <w:r><w:t>heading 1</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:outlineLvl w:val="2"/>
      </w:pPr>
      <w:r><w:t>heading 3</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:outlineLvl w:val="42"/>
      </w:pPr>
      <w:r><w:t>clamped</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>body</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 4);

        let level = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => {
                p.properties.as_ref().and_then(|props| props.outline_level)
            }
            _ => panic!("expected paragraph"),
        };

        // `w:val="0"` is the top heading level.
        assert_eq!(level(0), Some(0));
        assert_eq!(level(1), Some(2));
        // Out-of-range levels clamp to the 0..=9 envelope.
        assert_eq!(level(2), Some(9));
        // No `w:outlineLvl` leaves the property unset (body text).
        assert_eq!(level(3), None);
    }

    #[test]
    fn parses_paragraph_suppress_auto_hyphens_flag() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:suppressAutoHyphens/>
      </w:pPr>
      <w:r><w:t>no hyphenation</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:suppressAutoHyphens w:val="false"/>
      </w:pPr>
      <w:r><w:t>hyphenation allowed</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>default</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 3);

        let suppress = |idx: usize| match &body.elements[idx] {
            BodyElement::Paragraph(p) => p
                .properties
                .as_ref()
                .and_then(|props| props.suppress_auto_hyphens),
            _ => panic!("expected paragraph"),
        };

        // Self-closing element toggles the flag on (a pPr containing only this
        // must survive `paragraph_props_is_empty`).
        assert_eq!(suppress(0), Some(true));
        // `w:val="false"` disables it.
        assert_eq!(suppress(1), Some(false));
        // Absent → None (the paragraph follows the document hyphenation setting).
        assert_eq!(suppress(2), None);
    }

    #[test]
    fn parses_paragraph_auto_space_de_dn_flags() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:autoSpaceDE w:val="0"/>
        <w:autoSpaceDN/>
      </w:pPr>
      <w:r><w:t>第2項</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let settings = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.east_asian_settings.as_ref())
            .expect("east asian settings");
        assert_eq!(settings.auto_space_de, Some(false));
        assert_eq!(settings.auto_space_dn, Some(true));
    }

    #[test]
    fn parses_paragraph_line_break_typography_flags() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:kinsoku w:val="0"/>
        <w:wordWrap w:val="0"/>
        <w:overflowPunct/>
        <w:topLinePunct w:val="1"/>
      </w:pPr>
      <w:r><w:t>段落</w:t></w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let settings = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.east_asian_settings.as_ref())
            .expect("east asian settings");
        assert_eq!(settings.kinsoku, Some(false));
        assert_eq!(settings.word_wrap, Some(false));
        assert_eq!(settings.overflow_punctuation, Some(true));
        assert_eq!(settings.top_line_punctuation, Some(true));
    }

    #[test]
    fn preserves_ideographic_space_without_xml_space_preserve() {
        let body = r#"
    <w:p>
      <w:r>
        <w:t>(1)</w:t>
      </w:r>
      <w:r>
        <w:t>　法第15条</w:t>
      </w:r>
    </w:p>
    "#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "(1)　法第15条");
    }

    #[test]
    fn keeps_ideographic_space_after_marker_in_yokohama_page6() {
        let document = parse_document_from_fixture("yokohama_page6.docx");
        let body = document.body.expect("body");
        let paragraph = body
            .elements
            .iter()
            .find_map(|element| match element {
                BodyElement::Paragraph(paragraph) => {
                    let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                    if text.starts_with("(1)　法第15条第1項各号のいずれかに該当する事実")
                    {
                        Some(paragraph.as_ref())
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .expect("target paragraph not found");

        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(text.starts_with("(1)　法第15条第1項各号"));
    }

    #[test]
    fn fit_text_preserves_signed_group_ids_on_empty_and_start_elements() {
        for element in [
            r#"<w:fitText w:id="-931340544" w:val="840"/>"#,
            r#"<w:fitText w:id="-931340544" w:val="840"></w:fitText>"#,
        ] {
            let body = format!("<w:p><w:r><w:rPr>{element}</w:rPr><w:t>AB</w:t></w:r></w:p>");
            let document = parse_document_from_body(&body);
            let paragraph = document.body.as_ref().unwrap().elements[0]
                .as_paragraph()
                .unwrap();
            let fit = paragraph.runs[0]
                .properties
                .as_ref()
                .unwrap()
                .fit_text
                .as_ref()
                .unwrap();
            assert_eq!(fit.id.map(i64::from), Some(-931340544));
            assert_eq!(fit.val, 840);
        }
    }

    #[test]
    fn rtl_paragraph_preserves_fitted_run_properties() {
        let document = parse_document_from_body(
            r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:spacing w:val="120"/><w:fitText w:val="900" w:id="-50"/></w:rPr><w:t>אבגד</w:t></w:r></w:p>"#,
        );
        let paragraph = document.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap();
        assert_eq!(paragraph.properties.as_ref().unwrap().bidi, Some(true));
        assert_eq!(paragraph.runs[0].text, "אבגד");
        let properties = paragraph.runs[0].properties.as_ref().unwrap();
        assert_eq!(properties.char_spacing, Some(6.0));
        assert_eq!(properties.fit_text.as_ref().unwrap().val, 900);
        assert_eq!(properties.fit_text.as_ref().unwrap().id, Some(-50));
    }

    #[test]
    fn parses_run_styles() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr>
          <w:b w:val="1"/>
          <w:i w:val="true"/>
          <w:spacing w:val="40"/>
          <w:kern w:val="24"/>
          <w:w w:val="110"/>
          <w:fitText w:id="50" w:val="720"/>
          <w:vertAlign w:val="superscript"/>
          <w:strike/>
          <w:u w:val="single"/>
          <w:color w:val="FF0000"/>
        </w:rPr>
        <w:t>Styled</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");

        assert_eq!(props.bold, Some(true));
        assert_eq!(props.italic, Some(true));
        assert_eq!(props.strike, Some(true));
        assert_eq!(props.baseline.as_deref(), Some("superscript"));
        assert!((props.char_spacing.unwrap() - 2.0).abs() < f32::EPSILON);
        assert!((props.kerning.unwrap() - 12.0).abs() < f32::EPSILON);
        assert!((props.text_scale.unwrap() - 110.0).abs() < f32::EPSILON);
        let fit_text = props.fit_text.as_ref().expect("fitText property");
        assert_eq!(fit_text.id, Some(50));
        assert_eq!(fit_text.val, 720);
        assert_eq!(props.color.as_deref(), Some("#FF0000"));
    }

    #[test]
    fn parses_caps_run_property() {
        // `<w:caps/>` (self-closing) enables all-caps; an explicit
        // `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.1.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:caps/></w:rPr>
        <w:t>on</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:caps w:val="false"/></w:rPr>
        <w:t>off</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0].properties.as_ref().and_then(|p| p.caps),
            Some(true),
            "self-closing <w:caps/> must enable caps",
        );
        assert_eq!(
            paragraph.runs[1].properties.as_ref().and_then(|p| p.caps),
            Some(false),
            "<w:caps w:val=\"false\"/> must disable caps",
        );
        assert_eq!(
            paragraph.runs[2].properties.as_ref().and_then(|p| p.caps),
            None,
            "runs without <w:caps> must leave caps unspecified",
        );
    }

    #[test]
    fn parses_small_caps_run_property() {
        // `<w:smallCaps/>` (self-closing) enables small-caps; an explicit
        // `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.33.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:smallCaps/></w:rPr>
        <w:t>on</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:smallCaps w:val="false"/></w:rPr>
        <w:t>off</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.small_caps),
            Some(true),
            "self-closing <w:smallCaps/> must enable small caps",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.small_caps),
            Some(false),
            "<w:smallCaps w:val=\"false\"/> must disable small caps",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.small_caps),
            None,
            "runs without <w:smallCaps> must leave small_caps unspecified",
        );
    }

    #[test]
    fn parses_vanish_run_property() {
        // `<w:vanish/>` (self-closing) marks the run as hidden text; an explicit
        // `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.45.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:vanish/></w:rPr>
        <w:t>hidden</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:vanish w:val="false"/></w:rPr>
        <w:t>shown</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0].properties.as_ref().and_then(|p| p.vanish),
            Some(true),
            "self-closing <w:vanish/> must mark the run as hidden",
        );
        assert_eq!(
            paragraph.runs[1].properties.as_ref().and_then(|p| p.vanish),
            Some(false),
            "<w:vanish w:val=\"false\"/> must unhide the run",
        );
        assert_eq!(
            paragraph.runs[2].properties.as_ref().and_then(|p| p.vanish),
            None,
            "runs without <w:vanish> must leave vanish unspecified",
        );
    }

    #[test]
    fn parses_spec_vanish_run_property() {
        // `<w:specVanish/>` (self-closing) marks the run as always hidden; an
        // explicit `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.36.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:specVanish/></w:rPr>
        <w:t>hidden</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:specVanish w:val="false"/></w:rPr>
        <w:t>shown</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.spec_vanish),
            Some(true),
            "self-closing <w:specVanish/> must mark the run as always hidden",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.spec_vanish),
            Some(false),
            "<w:specVanish w:val=\"false\"/> must clear always-hidden",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.spec_vanish),
            None,
            "runs without <w:specVanish> must leave spec_vanish unspecified",
        );
    }

    #[test]
    fn parses_shadow_run_property() {
        // `<w:shadow/>` (self-closing) enables the drop-shadow effect; an
        // explicit `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.31.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:shadow/></w:rPr>
        <w:t>shadowed</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:shadow w:val="false"/></w:rPr>
        <w:t>plain</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0].properties.as_ref().and_then(|p| p.shadow),
            Some(true),
            "self-closing <w:shadow/> must enable the shadow",
        );
        assert_eq!(
            paragraph.runs[1].properties.as_ref().and_then(|p| p.shadow),
            Some(false),
            "<w:shadow w:val=\"false\"/> must disable the shadow",
        );
        assert_eq!(
            paragraph.runs[2].properties.as_ref().and_then(|p| p.shadow),
            None,
            "runs without <w:shadow> must leave shadow unspecified",
        );
    }

    #[test]
    fn parses_outline_run_property() {
        // `<w:outline/>` (self-closing) enables the hollow-outline effect; an
        // explicit `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.23.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:outline/></w:rPr>
        <w:t>outlined</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:outline w:val="false"/></w:rPr>
        <w:t>plain</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.outline),
            Some(true),
            "self-closing <w:outline/> must enable the outline",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.outline),
            Some(false),
            "<w:outline w:val=\"false\"/> must disable the outline",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.outline),
            None,
            "runs without <w:outline> must leave outline unspecified",
        );
    }

    #[test]
    fn parses_character_border_run_property() {
        // `<w:bdr/>` draws a box around the run text. It reuses the shared
        // BorderEdge structure (val/sz/space/color). ECMA-376 Part 1 §17.3.2.4.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:bdr w:val="single" w:sz="18" w:space="2" w:color="FF0000"/></w:rPr>
        <w:t>boxed</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:bdr w:val="nil"/></w:rPr>
        <w:t>nil</w:t>
      </w:r>
      <w:r>
        <w:t>plain</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let border = paragraph.runs[0]
            .properties
            .as_ref()
            .and_then(|p| p.border.as_ref())
            .expect("first run must carry a character border");
        assert_eq!(border.val.as_deref(), Some("single"));
        assert_eq!(border.size_eighths, Some(18));
        assert_eq!(border.space, Some(2));
        assert_eq!(border.color.as_deref(), Some("#FF0000"));
        assert!(border.is_visible());

        let nil_border = paragraph.runs[1]
            .properties
            .as_ref()
            .and_then(|p| p.border.as_ref())
            .expect("nil border is still parsed");
        assert_eq!(nil_border.val.as_deref(), Some("nil"));
        assert!(
            !nil_border.is_visible(),
            "a nil character border must not be visible",
        );

        assert!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.border.as_ref())
                .is_none(),
            "runs without <w:bdr> must leave the border unspecified",
        );
    }

    #[test]
    fn parses_emboss_and_imprint_run_properties() {
        // `<w:emboss/>` / `<w:imprint/>` (self-closing) enable the raised /
        // engraved effect; an explicit `w:val="false"` disables it.
        // ECMA-376 Part 1 §17.3.2.13 (emboss) / §17.3.2.18 (imprint).
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:emboss/><w:imprint/></w:rPr>
        <w:t>both</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:emboss w:val="false"/><w:imprint w:val="false"/></w:rPr>
        <w:t>off</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let props0 = paragraph.runs[0].properties.as_ref().expect("props");
        assert_eq!(
            props0.emboss,
            Some(true),
            "self-closing <w:emboss/> enables"
        );
        assert_eq!(
            props0.imprint,
            Some(true),
            "self-closing <w:imprint/> enables"
        );

        let props1 = paragraph.runs[1].properties.as_ref().expect("props");
        assert_eq!(
            props1.emboss,
            Some(false),
            "<w:emboss w:val=\"false\"/> disables"
        );
        assert_eq!(
            props1.imprint,
            Some(false),
            "<w:imprint w:val=\"false\"/> disables"
        );

        assert_eq!(
            paragraph.runs[2].properties.as_ref().and_then(|p| p.emboss),
            None,
            "runs without <w:emboss> leave it unspecified",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.imprint),
            None,
            "runs without <w:imprint> leave it unspecified",
        );
    }

    #[test]
    fn parses_run_level_character_shading() {
        // `<w:rPr><w:shd .../></w:rPr>` paints an arbitrary fill color behind
        // the run's glyphs (distinct from the 16-color `w:highlight`).
        // ECMA-376 Part 1 §17.3.5.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/></w:rPr>
        <w:t>shaded</w:t>
      </w:r>
      <w:r>
        <w:t>plain</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let shading = paragraph.runs[0]
            .properties
            .as_ref()
            .and_then(|p| p.shading.as_ref())
            .expect("run shading");
        assert_eq!(
            shading.resolved_background_color(),
            Some("#FFFF00".to_string()),
            "run-level w:shd fill resolves to the background color"
        );

        assert!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.shading.as_ref())
                .is_none(),
            "runs without w:shd leave shading unspecified",
        );
    }

    #[test]
    fn parses_complex_script_bold_italic_run_properties() {
        // `<w:bCs/>` / `<w:iCs/>` are the complex-script counterparts of
        // `w:b`/`w:i`; an explicit `w:val="false"` disables them.
        // ECMA-376 Part 1 §17.3.2.6 (bCs) / §17.3.2.17 (iCs).
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:bCs/><w:iCs/></w:rPr>
        <w:t>both</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:bCs w:val="false"/><w:iCs w:val="false"/></w:rPr>
        <w:t>off</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let props0 = paragraph.runs[0].properties.as_ref().expect("props");
        assert_eq!(
            props0.bold_complex,
            Some(true),
            "self-closing <w:bCs/> enables complex-script bold"
        );
        assert_eq!(
            props0.italic_complex,
            Some(true),
            "self-closing <w:iCs/> enables complex-script italic"
        );

        let props1 = paragraph.runs[1].properties.as_ref().expect("props");
        assert_eq!(
            props1.bold_complex,
            Some(false),
            "<w:bCs w:val=\"false\"/> disables"
        );
        assert_eq!(
            props1.italic_complex,
            Some(false),
            "<w:iCs w:val=\"false\"/> disables"
        );

        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.bold_complex),
            None,
            "runs without <w:bCs> leave it unspecified",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.italic_complex),
            None,
            "runs without <w:iCs> leave it unspecified",
        );
    }

    #[test]
    fn parses_double_strike_run_property() {
        // `<w:dstrike/>` (self-closing) enables double strikethrough; an
        // explicit `w:val="false"` disables it. ECMA-376 Part 1 §17.3.2.9.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:dstrike/></w:rPr>
        <w:t>on</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:dstrike w:val="false"/></w:rPr>
        <w:t>off</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.double_strike),
            Some(true),
            "self-closing <w:dstrike/> must enable double strikethrough",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.double_strike),
            Some(false),
            "<w:dstrike w:val=\"false\"/> must disable double strikethrough",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.double_strike),
            None,
            "runs without <w:dstrike> must leave double_strike unspecified",
        );
    }

    #[test]
    fn parses_text_position_run_property() {
        // `<w:position w:val="N"/>` raises/lowers the baseline by N half-points
        // (stored as points). Positive raises, negative lowers, zero is treated
        // as "no shift". ECMA-376 Part 1 §17.3.2.24.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:position w:val="12"/></w:rPr>
        <w:t>raised</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:position w:val="-8"/></w:rPr>
        <w:t>lowered</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:position w:val="0"/></w:rPr>
        <w:t>zero</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.text_position),
            Some(6.0),
            "w:val=12 half-points must store 6 points (raise)",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.text_position),
            Some(-4.0),
            "w:val=-8 half-points must store -4 points (lower)",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.text_position),
            None,
            "w:val=0 must be treated as no baseline shift",
        );
        assert_eq!(
            paragraph.runs[3]
                .properties
                .as_ref()
                .and_then(|p| p.text_position),
            None,
            "runs without <w:position> must leave text_position unspecified",
        );
    }

    #[test]
    fn parses_ruby_annotation_without_leaking_guide_text_into_body() {
        // `w:ruby` (ECMA-376 Part 1 §17.3.3.25): the guide text (`w:rt`,
        // ふりがな) must NOT flow into the body text — Word draws it above the
        // base text (`w:rubyBase`). Before ruby support the flat parser turned
        // the rt run into a normal body run, corrupting the text ("かんじ漢字")
        // and every downstream offset.
        let body = r#"
    <w:p>
      <w:r><w:t>前</w:t></w:r>
      <w:r>
        <w:ruby>
          <w:rubyPr>
            <w:rubyAlign w:val="distributeSpace"/>
            <w:hps w:val="10"/>
          </w:rubyPr>
          <w:rt>
            <w:r>
              <w:rPr><w:sz w:val="12"/></w:rPr>
              <w:t>かんじ</w:t>
            </w:r>
          </w:rt>
          <w:rubyBase>
            <w:r><w:t>漢字</w:t></w:r>
          </w:rubyBase>
        </w:ruby>
      </w:r>
      <w:r><w:t>後</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let texts: Vec<&str> = paragraph.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["前", "漢字", "後"],
            "body text must contain only the ruby base, never the rt guide text",
        );

        let ruby = paragraph.runs[1]
            .properties
            .as_ref()
            .and_then(|p| p.ruby.as_ref())
            .expect("base run must carry the ruby annotation");
        assert_eq!(ruby.text, "かんじ");
        assert_eq!(
            ruby.size_pt,
            Some(6.0),
            "rt run w:sz=12 half-points must win over w:rubyPr/w:hps",
        );
        assert_eq!(ruby.align.as_deref(), Some("distributespace"));

        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.ruby.clone()),
            None,
            "runs after the ruby must stay unannotated",
        );
    }

    #[test]
    fn parses_ruby_raise_and_guide_fonts() {
        // `w:hpsRaise` (§17.3.3.12) places the guide above the base, and the
        // guide run's own `w:rFonts` names the face it is drawn in.
        let body = r#"
    <w:p>
      <w:r>
        <w:ruby>
          <w:rubyPr>
            <w:rubyAlign w:val="center"/>
            <w:hps w:val="11"/>
            <w:hpsRaise w:val="21"/>
            <w:hpsBaseText w:val="21"/>
          </w:rubyPr>
          <w:rt>
            <w:r>
              <w:rPr><w:rFonts w:ascii="Arial" w:eastAsia="Noto Sans JP"/></w:rPr>
              <w:t>じゆん</w:t>
            </w:r>
          </w:rt>
          <w:rubyBase>
            <w:r><w:t>閏</w:t></w:r>
          </w:rubyBase>
        </w:ruby>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let BodyElement::Paragraph(paragraph) = &body.elements[0] else {
            panic!("expected paragraph");
        };
        let ruby = paragraph.runs[0]
            .properties
            .as_ref()
            .and_then(|p| p.ruby.as_ref())
            .expect("base run must carry the ruby annotation");
        assert_eq!(ruby.size_pt, Some(5.5));
        assert_eq!(ruby.raise_half_points, Some(21));
        let fonts = ruby.fonts.as_ref().expect("guide fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Arial"));
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Sans JP"));
        assert_eq!(fonts.h_ansi, None);
    }

    #[test]
    fn ruby_without_rt_size_falls_back_to_hps_and_empty_guide_is_dropped() {
        // When the rt run carries no `w:sz`, the `w:rubyPr/w:hps` value
        // (half-points) supplies the guide size. A ruby whose rt text is empty
        // must not annotate anything.
        let body = r#"
    <w:p>
      <w:r>
        <w:ruby>
          <w:rubyPr><w:hps w:val="14"/></w:rubyPr>
          <w:rt><w:r><w:t>よみ</w:t></w:r></w:rt>
          <w:rubyBase><w:r><w:t>読</w:t></w:r></w:rubyBase>
        </w:ruby>
      </w:r>
      <w:r>
        <w:ruby>
          <w:rubyPr><w:hps w:val="14"/></w:rubyPr>
          <w:rt></w:rt>
          <w:rubyBase><w:r><w:t>素</w:t></w:r></w:rubyBase>
        </w:ruby>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let texts: Vec<&str> = paragraph.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["読", "素"]);

        let ruby = paragraph.runs[0]
            .properties
            .as_ref()
            .and_then(|p| p.ruby.as_ref())
            .expect("first base run must carry the ruby annotation");
        assert_eq!(ruby.text, "よみ");
        assert_eq!(
            ruby.size_pt,
            Some(7.0),
            "w:hps=14 half-points must resolve to 7pt when rt has no w:sz",
        );

        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.ruby.clone()),
            None,
            "empty rt guide text must not produce an annotation",
        );
    }

    #[test]
    fn multi_run_ruby_base_annotates_every_base_run_once() {
        // A base with a formatting change is several `w:r`. Edits must know
        // that all of them belong to the base, but the guide is drawn once.
        let body = r#"
    <w:p>
      <w:r>
        <w:ruby>
          <w:rubyPr><w:hps w:val="10"/></w:rubyPr>
          <w:rt><w:r><w:t>kana</w:t></w:r></w:rt>
          <w:rubyBase>
            <w:r><w:t>X</w:t></w:r>
            <w:r><w:rPr><w:i/></w:rPr><w:t>Y</w:t></w:r>
          </w:rubyBase>
        </w:ruby>
      </w:r>
      <w:r><w:t>after</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let ruby = |index: usize| {
            paragraph.runs[index]
                .properties
                .as_ref()
                .and_then(|p| p.ruby.clone())
        };

        let texts: Vec<&str> = paragraph.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["X", "Y", "after"]);
        let start = ruby(0).expect("first base run starts the ruby");
        assert_eq!(start.text, "kana");
        assert!(!start.continuation, "the first base run draws the guide");
        let next = ruby(1).expect("second base run belongs to the same base");
        assert_eq!(next.text, "kana");
        assert!(
            next.continuation,
            "later base runs must not redraw the guide"
        );
        assert_eq!(
            paragraph.runs[1].properties.as_ref().and_then(|p| p.italic),
            Some(true),
            "each base run keeps its own formatting",
        );
        assert_eq!(ruby(2), None, "runs after the ruby must stay unannotated");
    }

    #[test]
    fn parses_emphasis_mark_run_property() {
        // `<w:em w:val="dot"/>` draws an emphasis mark above each character;
        // `w:val="none"` (or absent) means no mark. The token is normalized to
        // lowercase. ECMA-376 Part 1 §17.3.2.12.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:em w:val="dot"/></w:rPr>
        <w:t>dot</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:em w:val="underDot"/></w:rPr>
        <w:t>under</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:em w:val="none"/></w:rPr>
        <w:t>none</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.emphasis.as_deref()),
            Some("dot"),
            "<w:em w:val=\"dot\"/> must capture the dot mark",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.emphasis.as_deref()),
            Some("underdot"),
            "the emphasis token must be normalized to lowercase",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.emphasis.clone()),
            None,
            "<w:em w:val=\"none\"/> must leave emphasis unset",
        );
        assert_eq!(
            paragraph.runs[3]
                .properties
                .as_ref()
                .and_then(|p| p.emphasis.clone()),
            None,
            "runs without <w:em> must leave emphasis unset",
        );
    }

    #[test]
    fn parses_text_effect_run_property() {
        // `<w:effect w:val="sparkle"/>` requests an animated text effect
        // (ECMA-376 Part 1 §17.3.2.10). The token is normalized to lowercase;
        // `w:val="none"` (or absent) means no effect. These animations have no
        // static visual but are parsed/preserved for round-trip.
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr><w:effect w:val="sparkle"/></w:rPr>
        <w:t>sparkle</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:effect w:val="BlinkBackground"/></w:rPr>
        <w:t>blink</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:effect w:val="none"/></w:rPr>
        <w:t>none</w:t>
      </w:r>
      <w:r>
        <w:t>absent</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.effect.as_deref()),
            Some("sparkle"),
            "<w:effect w:val=\"sparkle\"/> must capture the effect",
        );
        assert_eq!(
            paragraph.runs[1]
                .properties
                .as_ref()
                .and_then(|p| p.effect.as_deref()),
            Some("blinkbackground"),
            "the effect token must be normalized to lowercase",
        );
        assert_eq!(
            paragraph.runs[2]
                .properties
                .as_ref()
                .and_then(|p| p.effect.clone()),
            None,
            "<w:effect w:val=\"none\"/> must leave effect unset",
        );
        assert_eq!(
            paragraph.runs[3]
                .properties
                .as_ref()
                .and_then(|p| p.effect.clone()),
            None,
            "runs without <w:effect> must leave effect unset",
        );
    }

    #[test]
    fn parses_symbol_char_into_run_text_and_font() {
        // `<w:sym w:font="Wingdings" w:char="F0E0"/>` references glyph F0E0 in
        // the Wingdings font (ECMA-376 Part 1 §17.3.3.30). Decode the hex code
        // point into the run text and apply the symbol font so the glyph can
        // render instead of vanishing.
        let body = r#"
    <w:p>
      <w:r>
        <w:sym w:font="Wingdings" w:char="F0E0"/>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        let run = &paragraph.runs[0];
        assert_eq!(
            run.text, "\u{F0E0}",
            "w:sym char must decode into the run text as a code point",
        );
        let fonts = run
            .properties
            .as_ref()
            .and_then(|p| p.fonts.as_ref())
            .expect("symbol run should carry font metadata");
        assert_eq!(fonts.ascii.as_deref(), Some("Wingdings"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Wingdings"));
    }

    #[test]
    fn parses_no_break_hyphen_into_run_text() {
        // `<w:noBreakHyphen/>` (§17.3.3.18) must contribute the hyphen glyph to
        // the run text; otherwise "abc-def" loses its hyphen. It is emitted as
        // U+2011 (NON-BREAKING HYPHEN).
        let body = r#"
    <w:p>
      <w:r>
        <w:t>abc</w:t>
        <w:noBreakHyphen/>
        <w:t>def</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs[0].text, "abc\u{2011}def");
    }

    #[test]
    fn parses_soft_hyphen_into_run_text() {
        // `<w:softHyphen/>` is an optional hyphen location. Preserve it as
        // U+00AD so logical offsets remain aligned with Word and layout can
        // distinguish the discretionary break from an ordinary hyphen.
        let body = r#"
    <w:p>
      <w:r>
        <w:t>co</w:t>
        <w:softHyphen/>
        <w:t>operate</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs[0].text, "co\u{00AD}operate");
    }

    #[test]
    fn column_break_maps_to_page_break_and_keeps_offsets_in_sync() {
        // `<w:br w:type="column"/>` (§17.3.3.1) has no dedicated column advance
        // yet, so it reuses the page-break char. Crucially, an unhandled break
        // type must NOT bump the text-length counter (which previously desynced
        // offsets); only real characters do.
        let body = r#"
    <w:p>
      <w:r>
        <w:t>A</w:t>
        <w:br w:type="column"/>
        <w:t>B</w:t>
        <w:br w:type="unknownkind"/>
        <w:t>C</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        // A column break has its own character; the unknown break adds
        // nothing.
        assert_eq!(paragraph.runs[0].text, "A\u{000E}BC");
    }

    #[test]
    fn last_rendered_page_break_is_not_exposed_as_editable_text() {
        // `w:lastRenderedPageBreak` (§17.3.3.13) records a stale pagination
        // result. It is preserved by the raw round-trip tree, but must not alter
        // live layout or user-editable text.
        let body = r#"
    <w:p>
      <w:r>
        <w:t>Hello</w:t>
        <w:lastRenderedPageBreak/>
        <w:t>World</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs[0].text, "HelloWorld");
    }

    #[test]
    fn parses_positional_tab_as_tab_character() {
        // `<w:ptab/>` (§17.3.3.16) is an absolute-position tab. Dropping it loses
        // the gap; emit a regular tab so the text isn't run together.
        let body = r#"
    <w:p>
      <w:r>
        <w:t>left</w:t>
        <w:ptab w:alignment="right" w:relativeTo="margin" w:leader="dot"/>
        <w:t>right</w:t>
      </w:r>
    </w:p>
"#;
        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs[0].text, "left\tright");
    }

    #[test]
    fn parses_move_from_and_move_to_as_revisions() {
        // `<w:moveFrom>` wraps runs being moved away from their original
        // location (shown struck through, like a deletion); `<w:moveTo>`
        // wraps the same content at its new location (shown like an
        // insertion). ECMA-376 Part 1 §17.13.5.20 / §17.13.5.25.
        let body = r#"
    <w:p>
      <w:moveFrom w:id="3" w:author="Reviewer" w:date="2026-05-30T00:00:00Z">
        <w:r>
          <w:t>source</w:t>
        </w:r>
      </w:moveFrom>
      <w:moveTo w:id="4" w:author="Reviewer" w:date="2026-05-30T00:00:00Z">
        <w:r>
          <w:t>dest</w:t>
        </w:r>
      </w:moveTo>
      <w:r>
        <w:t>plain</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph.runs.len(), 3);

        let moved_from = &paragraph.runs[0];
        assert_eq!(moved_from.text, "source");
        let from_mark = moved_from.revision.as_ref().expect("moveFrom revision");
        assert!(matches!(from_mark.kind, RevisionKind::MovedFrom));
        assert_eq!(from_mark.author.as_deref(), Some("Reviewer"));

        let moved_to = &paragraph.runs[1];
        assert_eq!(moved_to.text, "dest");
        let to_mark = moved_to.revision.as_ref().expect("moveTo revision");
        assert!(matches!(to_mark.kind, RevisionKind::MovedTo));

        let plain = &paragraph.runs[2];
        assert_eq!(plain.text, "plain");
        assert!(
            plain.revision.is_none(),
            "runs outside move ranges must not carry a revision mark",
        );
    }

    #[test]
    fn ignores_underline_tag_without_val_when_only_color_is_present() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:rPr>
          <w:u w:color="000000"/>
        </w:rPr>
      </w:pPr>
      <w:r>
        <w:t>plain</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");

        assert_eq!(props.underline, None);
    }

    #[test]
    fn parses_highlight_empty_tag_as_run_property() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr>
          <w:highlight w:val="yellow"/>
        </w:rPr>
        <w:t>highlighted</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");

        assert_eq!(props.highlight.as_deref(), Some("yellow"));
    }

    #[test]
    fn leaves_underline_without_val_unspecified() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr>
          <w:u/>
        </w:rPr>
        <w:t>underlined</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");

        // [MS-OE376 §2.3.48](https://learn.microsoft.com/openspecs/office_standards/ms-oe376/70728747-cf2d-4393-afb3-f5117faa27d5):
        // Word first consults the style hierarchy when `w:u/@w:val` is omitted
        // and otherwise renders no underline. Keeping `None` here preserves
        // that distinction until style resolution.
        assert_eq!(props.underline, None);
    }

    #[test]
    fn parses_underline_none_as_disabled() {
        let body = r#"
    <w:p>
      <w:r>
        <w:rPr>
          <w:u w:val="none"/>
        </w:rPr>
        <w:t>plain</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let props = run.properties.as_ref().expect("run properties");

        assert_eq!(props.underline, Some(false));
    }

    #[test]
    fn parses_inline_math_as_raw_omml() {
        let body = r#"
    <w:p>
      <w:r>
        <m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/relationships/math">
          <m:r>
            <m:t>1</m:t>
          </m:r>
        </m:oMath>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.len(), 1);

        let run = &paragraph.runs[0];
        assert!(run.math.is_some(), "math run should be captured");
        assert!(
            run.text.is_empty(),
            "math run should not produce plain text"
        );
        let math = run.math.as_ref().expect("math");
        assert!(
            math.raw_omml.contains("<m:oMath"),
            "raw OMML should include oMath element"
        );
        assert!(
            math.raw_omml.contains("<m:t>1</m:t>"),
            "raw OMML should preserve child text"
        );
        let elements = &math.elements;
        assert_eq!(elements.len(), 1);
        let root = &elements[0];
        assert_eq!(root.kind, MathElementKind::Math);
        assert_eq!(root.children.len(), 1);
        let run_elem = &root.children[0];
        assert_eq!(run_elem.kind, MathElementKind::Run);
        assert_eq!(run_elem.children.len(), 1);
        let text_elem = &run_elem.children[0];
        assert_eq!(text_elem.kind, MathElementKind::Text);
        assert_eq!(text_elem.text.as_deref(), Some("1"));
    }

    #[test]
    fn parses_math_paragraph_as_block_element() {
        let body = r#"
    <m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/relationships/math">
      <m:oMath>
        <m:r>
          <m:t>x</m:t>
        </m:r>
      </m:oMath>
    </m:oMathPara>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 1);
        let math_paragraph = match &body.elements[0] {
            BodyElement::Math(math) => math.as_ref(),
            _ => panic!("expected math paragraph"),
        };
        assert!(
            math_paragraph.raw_omml.contains("<m:oMathPara"),
            "raw OMML should include oMathPara element"
        );
        assert_eq!(math_paragraph.elements.len(), 1);
        let root = &math_paragraph.elements[0];
        assert_eq!(root.kind, MathElementKind::MathParagraph);
        let math_child = root
            .children
            .first()
            .expect("math paragraph should contain math child");
        assert_eq!(math_child.kind, MathElementKind::Math);
        let run = math_child
            .children
            .first()
            .expect("math should contain run");
        assert_eq!(run.kind, MathElementKind::Run);
        let text = run
            .children
            .first()
            .expect("run should contain text element");
        assert_eq!(text.kind, MathElementKind::Text);
        assert_eq!(text.text.as_deref(), Some("x"));
    }

    #[test]
    fn parses_word_math_paragraph_wrapper_without_phantom_editable_paragraph() {
        let body = r#"
    <w:p>
      <m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">
        <m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>
      </m:oMathPara>
    </w:p>
    <w:p><w:r><w:t>after</w:t></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 2);
        assert!(matches!(body.elements[0], BodyElement::Math(_)));
        let paragraph = match &body.elements[1] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected the following editable paragraph"),
        };
        assert_eq!(paragraph.runs[0].text, "after");
    }

    #[test]
    fn math_block_retains_its_wrapper_paragraph_formatting() {
        let document = parse_document_from_body(
            r#"<w:p w14:paraId="ABCD1234"><w:pPr><w:pStyle w:val="Equation"/><w:ind w:left="1000" w:right="200"/><w:spacing w:before="120" w:after="60"/><w:keepNext/><w:rPr><w:sz w:val="16"/></w:rPr></w:pPr><m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><w:r><w:t>after</w:t></w:r></w:p>"#,
        );
        let body = document.body.unwrap();
        assert_eq!(body.elements.len(), 2);
        let BodyElement::Math(math) = &body.elements[0] else {
            panic!("expected a single mathematical block");
        };
        let value = serde_json::to_value(math).unwrap();
        let props = &value["properties"];
        assert_eq!(props["style"], "Equation");
        assert_eq!(props["indent"]["left"], 50.0);
        assert_eq!(props["indent"]["right"], 10.0);
        assert_eq!(props["spacing"]["before"], 6.0);
        assert_eq!(props["spacing"]["after"], 3.0);
        assert_eq!(props["keep_next"], true);
        assert_eq!(props["mark_run_properties"]["font_size"], 8.0);
        assert_eq!(props["para_id"], "ABCD1234");
        assert!(math.raw_omml.starts_with("<m:oMathPara"));
        let legacy: MathParagraph =
            serde_json::from_str(r#"{"raw_omml":"<m:oMathPara/>"}"#).unwrap();
        assert!(
            legacy.properties.is_none(),
            "old serialized math remains readable"
        );
    }

    #[test]
    fn parses_revision_marks_from_review_fixture() {
        let document = parse_document_from_fixture("review.docx");
        let body = document.body.expect("body");
        let paragraph = body
            .elements
            .first()
            .and_then(|element| element.as_paragraph())
            .expect("first paragraph");

        assert_eq!(paragraph.runs.len(), 2);
        let inserted = &paragraph.runs[0];
        assert_eq!(inserted.text, "Hello ");
        let inserted_mark = inserted.revision.as_ref().expect("inserted revision mark");
        assert!(matches!(inserted_mark.kind, RevisionKind::Inserted));

        let deleted = &paragraph.runs[1];
        assert_eq!(deleted.text, "World");
        let deleted_mark = deleted.revision.as_ref().expect("deleted revision mark");
        assert!(matches!(deleted_mark.kind, RevisionKind::Deleted));
    }

    /// Ensures floating images (`wp:anchor`) capture relationship, size, and
    /// positioning metadata, ready for absolute placement in canvas renderer.
    #[test]
    fn parses_inline_images() {
        let body = r#"
     <w:p>
       <w:r>
         <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
                     distT="152400" distB="152400" distL="152400" distR="152400"
                     simplePos="1" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
            <wp:simplePos x="114300" y="228600"/>
            <wp:extent cx="914400" cy="914400"/>
            <wp:positionH relativeFrom="margin">
              <wp:posOffset>1517660</wp:posOffset>
            </wp:positionH>
            <wp:positionV relativeFrom="line">
              <wp:posOffset>301675</wp:posOffset>
            </wp:positionV>
            <wp:docPr id="1" name="Picture 1" descr="Sample image"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                <pic:pic>
                  <pic:blipFill>
                    <a:blip r:embed="rId5"/>
                    <a:srcRect l="15000" t="5%" r="25000" b="10000"/>
                  </pic:blipFill>
                  <pic:spPr>
                    <a:xfrm>
                      <a:ext cx="914400" cy="914400"/>
                    </a:xfrm>
                  </pic:spPr>
                </pic:pic>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let image = run.image.as_ref().expect("image metadata");

        assert_eq!(image.relationship_id.as_deref(), Some("rId5"));
        assert!(image.data_uri.is_none());
        let width = image.width_px.expect("width");
        let height = image.height_px.expect("height");
        assert!((width - 96.0).abs() < f64::EPSILON);
        assert!((height - 96.0).abs() < f64::EPSILON);
        assert_eq!(image.description.as_deref(), Some("Sample image"));
        let crop = image.crop.expect("source-image crop");
        assert!((crop.left - 0.15).abs() < f64::EPSILON);
        assert!((crop.top - 0.05).abs() < f64::EPSILON);
        assert!((crop.right - 0.25).abs() < f64::EPSILON);
        assert!((crop.bottom - 0.10).abs() < f64::EPSILON);
        assert!(matches!(image.layout.kind, RunImageKind::Floating));
        assert_eq!(image.layout.relative_height, Some(0));
        let simple_position = image.layout.simple_position.expect("simple position");
        assert!((simple_position.x_px - 12.0).abs() < f64::EPSILON);
        assert!((simple_position.y_px - 24.0).abs() < f64::EPSILON);
        let pos_h = image
            .layout
            .position_h
            .as_ref()
            .expect("horizontal position");
        assert_eq!(pos_h.relative_from.as_deref(), Some("margin"));
        assert!((pos_h.offset_px.expect("h offset") - 159.0).abs() < 1.0);
        let pos_v = image.layout.position_v.as_ref().expect("vertical position");
        assert_eq!(pos_v.relative_from.as_deref(), Some("line"));
        assert!((pos_v.offset_px.expect("v offset") - 32.0).abs() < 1.0);
    }

    #[test]
    fn parses_ink_name_and_unescapes_payload_independent_of_attribute_order() {
        let body = r#"
    <w:p><w:r><w:drawing>
      <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                 xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
        <wp:extent cx="9525" cy="9525"/>
        <wp:docPr id="1" title="ordinary title" descr="{&quot;text&quot;:&quot;A &amp; B &lt; C&quot;}" name="aidocs-ink stroke-1"/>
        <a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId9"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
      </wp:anchor>
    </w:drawing></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = body.elements[0].as_paragraph().expect("paragraph");
        let image = paragraph.runs[0].image.as_ref().expect("image metadata");
        assert_eq!(image.title.as_deref(), Some("aidocs-ink stroke-1"));
        assert_eq!(
            image.description.as_deref(),
            Some(r#"{"text":"A & B < C"}"#)
        );
    }

    /// Floating objects positioned by alignment keyword (`wp:align`) instead of
    /// an absolute `wp:posOffset` should capture the lowercased keyword and
    /// leave `offset_px` unset. ECMA-376 Part 1 §20.4.2.1 / §20.4.2.2.
    #[test]
    fn parses_floating_image_alignment_keywords() {
        let body = r#"
     <w:p>
       <w:r>
         <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
                     simplePos="0" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
            <wp:extent cx="914400" cy="914400"/>
            <wp:positionH relativeFrom="page">
              <wp:align>Center</wp:align>
            </wp:positionH>
            <wp:positionV relativeFrom="margin">
              <wp:align>bottom</wp:align>
            </wp:positionV>
            <wp:docPr id="1" name="Picture 1"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                <pic:pic>
                  <pic:blipFill>
                    <a:blip r:embed="rId7"/>
                  </pic:blipFill>
                </pic:pic>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let image = paragraph.runs[0].image.as_ref().expect("image metadata");
        let pos_h = image
            .layout
            .position_h
            .as_ref()
            .expect("horizontal position");
        assert_eq!(pos_h.relative_from.as_deref(), Some("page"));
        assert_eq!(pos_h.align.as_deref(), Some("center"));
        assert!(pos_h.offset_px.is_none());
        let pos_v = image.layout.position_v.as_ref().expect("vertical position");
        assert_eq!(pos_v.relative_from.as_deref(), Some("margin"));
        assert_eq!(pos_v.align.as_deref(), Some("bottom"));
        assert!(pos_v.offset_px.is_none());
    }

    /// Floating objects declare a wrap element (`wp:wrapSquare` etc.) plus
    /// `dist*` clearances. These must surface on the layout so the line
    /// layout engine can reserve space. ECMA-376 Part 1 §20.4.2.17–20.
    #[test]
    fn parses_floating_image_wrap_square_and_distances() {
        let body = r#"
     <w:p>
       <w:r>
         <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
                     distT="0" distB="0" distL="114300" distR="114300"
                     simplePos="0" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
            <wp:extent cx="914400" cy="914400"/>
            <wp:positionH relativeFrom="column">
              <wp:posOffset>0</wp:posOffset>
            </wp:positionH>
            <wp:positionV relativeFrom="paragraph">
              <wp:posOffset>0</wp:posOffset>
            </wp:positionV>
            <wp:wrapSquare wrapText="right"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                <pic:pic>
                  <pic:blipFill>
                    <a:blip r:embed="rId9"/>
                  </pic:blipFill>
                </pic:pic>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let layout = &paragraph.runs[0].image.as_ref().expect("image").layout;
        assert_eq!(layout.wrap, WrapMode::Square);
        assert_eq!(layout.wrap_text, WrapTextSide::Right);
        // distL/distR = 114300 EMU = 0.125in = 12px; distT/distB = 0.
        assert!((layout.dist_left.expect("dist_left") - 12.0).abs() < 0.5);
        assert!((layout.dist_right.expect("dist_right") - 12.0).abs() < 0.5);
        assert_eq!(layout.dist_top, Some(0.0));
        assert_eq!(layout.dist_bottom, Some(0.0));
    }

    #[test]
    fn parses_edited_tight_wrap_polygon_points() {
        let body = r#"
    <w:p><w:r><w:drawing>
      <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                 xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
        <wp:extent cx="914400" cy="914400"/>
        <wp:wrapTight wrapText="largest">
          <wp:wrapPolygon edited="1">
            <wp:start x="10800" y="0"/>
            <wp:lineTo x="21600" y="21600"/>
            <wp:lineTo x="0" y="21600"/>
            <wp:lineTo x="10800" y="0"/>
          </wp:wrapPolygon>
        </wp:wrapTight>
        <a:graphic><a:graphicData><pic:pic><pic:blipFill>
          <a:blip r:embed="rIdPolygon"/>
        </pic:blipFill></pic:pic></a:graphicData></a:graphic>
      </wp:anchor>
    </w:drawing></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let paragraph = document.body.expect("body").elements[0]
            .as_paragraph()
            .expect("paragraph")
            .clone();
        let layout = &paragraph.runs[0].image.as_ref().expect("image").layout;
        assert_eq!(layout.wrap, WrapMode::Tight);
        assert_eq!(layout.wrap_text, WrapTextSide::Largest);
        assert_eq!(
            layout.wrap_polygon,
            Some(WrapPolygon {
                edited: true,
                points: vec![
                    WrapPolygonPoint { x: 10800, y: 0 },
                    WrapPolygonPoint { x: 21600, y: 21600 },
                    WrapPolygonPoint { x: 0, y: 21600 },
                    WrapPolygonPoint { x: 10800, y: 0 },
                ],
            })
        );
    }

    /// `wp:wrapTopAndBottom` and `wp:wrapNone` map to their respective modes.
    #[test]
    fn parses_floating_image_wrap_top_and_bottom_and_none() {
        let make = |wrap_element: &str| {
            let body = format!(
                r#"
     <w:p>
       <w:r>
         <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
                     distT="91440" distB="91440" distL="0" distR="0"
                     simplePos="0" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
            <wp:extent cx="914400" cy="914400"/>
            <wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH>
            <wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>
            {wrap_element}
            <a:graphic>
              <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                <pic:pic><pic:blipFill><a:blip r:embed="rIdA"/></pic:blipFill></pic:pic>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#
            );
            let document = parse_document_from_body(&body);
            let body = document.body.expect("body");
            match &body.elements[0] {
                BodyElement::Paragraph(p) => {
                    p.runs[0].image.as_ref().expect("image").layout.clone()
                }
                _ => panic!("expected paragraph"),
            }
        };

        let tb = make("<wp:wrapTopAndBottom/>");
        assert_eq!(tb.wrap, WrapMode::TopAndBottom);
        assert!((tb.dist_top.expect("dist_top") - 9.6).abs() < 0.5);

        let none = make("<wp:wrapNone/>");
        assert_eq!(none.wrap, WrapMode::None);
    }

    #[test]
    fn hidden_line_metadata_does_not_replace_visible_shape_properties() {
        for hidden_fill in [
            "<a:solidFill><a:srgbClr val=\"000000\"><a:alpha val=\"25000\"/></a:srgbClr></a:solidFill>",
            "<a:noFill/>",
        ] {
            let body = format!(r#"
                <w:p><w:r><w:drawing>
                  <wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                    xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                    xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"
                    xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                    <wp:extent cx="952500" cy="476250"/>
                    <a:graphic><a:graphicData><wps:wsp><wps:spPr>
                      <a:solidFill><a:srgbClr val="112233"><a:alpha val="75000"/></a:srgbClr></a:solidFill>
                      <a:ln w="19050"><a:solidFill><a:srgbClr val="778899"/></a:solidFill></a:ln>
                      <a:extLst><a:ext uri="hidden-line"><a14:hiddenLine w="127000">
                        {hidden_fill}<a:prstDash val="dash"/>
                      </a14:hiddenLine></a:ext></a:extLst>
                    </wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>Text</w:t></w:r></w:p></w:txbxContent></wps:txbx>
                    </wps:wsp></a:graphicData></a:graphic>
                  </wp:inline>
                </w:drawing></w:r></w:p>
            "#);
            let document = parse_document_from_body(&body);
            let text_box = document.body.as_ref().unwrap().elements[0]
                .as_paragraph().unwrap().runs[0].text_box.as_ref().unwrap();
            assert_eq!(text_box.fill_color.as_deref(), Some("#112233"));
            assert_eq!(text_box.fill_opacity, Some(0.75));
            assert_eq!(text_box.border.color.as_deref(), Some("#778899"));
            assert_eq!(text_box.border.style, TextBoxBorderStyle::Solid);
            assert_eq!(text_box.text, "Text");
        }
    }

    #[test]
    fn direct_auto_spacing_replaces_authored_points_and_lines() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:spacing w:beforeAutospacing="true" w:before="0" w:beforeLines="50" w:afterAutospacing="1" w:after="600"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p></w:body></w:document>"#,
            W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
        );
        let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
        let paragraph = document.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap();
        let spacing = paragraph
            .properties
            .as_ref()
            .unwrap()
            .spacing
            .as_ref()
            .unwrap();
        assert_eq!(spacing.before, Some(14.0));
        assert_eq!(spacing.before_lines, None);
        assert_eq!(spacing.after, Some(14.0));
    }

    #[test]
    fn math_fixture_text_box_retains_its_white_shape_fill() {
        let xml = read_document_xml(&fixture_path("math.docx")).expect("math document XML");
        let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
        let text_boxes = document
            .body
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .flat_map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .filter_map(|run| run.text_box.as_ref())
            })
            .collect::<Vec<_>>();
        assert_eq!(text_boxes.len(), 1);
        assert_eq!(text_boxes[0].fill_color.as_deref(), Some("#FFFFFF"));
        assert_eq!(text_boxes[0].border.style, TextBoxBorderStyle::None);
    }

    #[test]
    fn parses_drawingml_outer_shadow_without_overwriting_shape_fill() {
        let body = r#"
    <w:p><w:r><w:drawing>
      <wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                 xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
        <wp:extent cx="952500" cy="476250"/>
        <a:graphic><a:graphicData>
          <wps:wsp><wps:spPr>
            <a:prstGeom prst="rect"/>
            <a:solidFill><a:srgbClr val="112233"><a:alpha val="37500"/></a:srgbClr></a:solidFill>
            <a:ln><a:solidFill><a:srgbClr val="778899"><a:alpha val="55000"/></a:srgbClr></a:solidFill></a:ln>
            <a:effectLst>
              <a:glow rad="57150">
                <a:schemeClr val="accent1"><a:tint val="50000"/><a:alpha val="30000"/></a:schemeClr>
              </a:glow>
              <a:outerShdw blurRad="38100" dist="19050" dir="5400000">
                <a:srgbClr val="445566"><a:alpha val="40000"/></a:srgbClr>
              </a:outerShdw>
              <a:innerShdw blurRad="28575" dist="9525" dir="0">
                <a:schemeClr val="accent2"><a:shade val="25000"/><a:alpha val="20000"/></a:schemeClr>
              </a:innerShdw>
              <a:blur rad="28575"/>
              <a:softEdge rad="47625"/>
            </a:effectLst>
          </wps:spPr></wps:wsp>
        </a:graphicData></a:graphic>
      </wp:inline>
    </w:drawing></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let text_box = body.elements[0].as_paragraph().expect("paragraph").runs[0]
            .text_box
            .as_ref()
            .expect("shape");
        assert_eq!(text_box.fill_color.as_deref(), Some("#112233"));
        assert!((text_box.fill_opacity.expect("fill opacity") - 0.375).abs() < 0.000_001);
        assert_eq!(text_box.border.color.as_deref(), Some("#778899"));
        assert!((text_box.border.opacity.expect("line opacity") - 0.55).abs() < 0.000_001);
        let shadow = text_box.shadow.as_ref().expect("outer shadow");
        assert_eq!(shadow.color.as_deref(), Some("#445566"));
        assert_eq!(shadow.theme_color, None);
        assert!((shadow.opacity - 0.4).abs() < 0.000_001);
        assert!((shadow.blur_radius_px - 4.0).abs() < 0.000_001);
        assert!((shadow.distance_px - 2.0).abs() < 0.000_001);
        assert!((shadow.direction_deg - 90.0).abs() < 0.000_001);
        let inner_shadow = text_box.inner_shadow.as_ref().expect("inner shadow");
        assert_eq!(inner_shadow.color, None);
        assert_eq!(inner_shadow.theme_color.as_deref(), Some("accent2"));
        assert_eq!(
            inner_shadow.theme_transforms,
            vec![ThemeColorTransform::Shade(0.25)]
        );
        assert!((inner_shadow.opacity - 0.2).abs() < 0.000_001);
        assert!((inner_shadow.blur_radius_px - 3.0).abs() < 0.000_001);
        assert!((inner_shadow.distance_px - 1.0).abs() < 0.000_001);
        assert!(inner_shadow.direction_deg.abs() < 0.000_001);
        let glow = text_box.glow.as_ref().expect("glow");
        assert_eq!(glow.color, None);
        assert_eq!(glow.theme_color.as_deref(), Some("accent1"));
        assert_eq!(glow.theme_transforms, vec![ThemeColorTransform::Tint(0.5)]);
        assert!((glow.opacity - 0.3).abs() < 0.000_001);
        assert!((glow.radius_px - 6.0).abs() < 0.000_001);
        assert!((text_box.blur_radius_px.expect("blur") - 3.0).abs() < 0.000_001);
        assert!((text_box.soft_edge_radius_px.expect("soft edge") - 5.0).abs() < 0.000_001);
    }

    #[test]
    fn parses_grouped_drawing_children_in_the_group_coordinate_space() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                     xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"
                     xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
                     relativeHeight="42">
            <wp:extent cx="1828800" cy="914400"/>
            <wp:docPr id="1" name="Group 1" descr="Navigation controls"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup">
                <wpg:wgp>
                  <wpg:grpSpPr><a:xfrm rot="900000" flipH="1"><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/><a:chOff x="100" y="200"/><a:chExt cx="1000" cy="500"/></a:xfrm></wpg:grpSpPr>
                  <wps:wsp>
                    <wps:cNvPr id="1" name="homeLink"><a:hlinkClick r:id="rId5" tooltip="Home" tgtFrame="_top"/></wps:cNvPr>
                    <wps:spPr><a:xfrm rot="1800000" flipV="true"><a:off x="200" y="250"/><a:ext cx="400" cy="100"/></a:xfrm><a:prstGeom prst="roundRect"/><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="60000"/><a:lumOff val="40000"/><a:alpha val="65000"/></a:schemeClr></a:solidFill><a:ln w="19050"><a:solidFill><a:schemeClr val="accent2"><a:shade val="50000"/><a:alpha val="45000"/></a:schemeClr></a:solidFill></a:ln><a:effectLst><a:glow rad="57150"><a:schemeClr val="accent1"><a:tint val="50000"/><a:alpha val="30000"/></a:schemeClr></a:glow><a:outerShdw blurRad="38100" dist="19050" dir="10800000"><a:schemeClr val="accent2"><a:shade val="50000"/><a:alpha val="60000"/></a:schemeClr></a:outerShdw><a:innerShdw blurRad="28575" dist="9525" dir="0"><a:schemeClr val="accent1"><a:tint val="25000"/><a:alpha val="20000"/></a:schemeClr></a:innerShdw><a:blur rad="19050"/><a:softEdge rad="28575"/></a:effectLst></wps:spPr>
                    <wps:txbx><w:txbxContent><w:p><w:r><w:t>Grouped label</w:t></w:r></w:p></w:txbxContent></wps:txbx>
                    <wps:bodyPr lIns="95250" tIns="190500" rIns="285750" bIns="381000" anchor="ctr"/>
                  </wps:wsp>
                  <wps:wsp>
                    <wps:spPr><a:xfrm><a:off x="600" y="250"/><a:ext cx="200" cy="100"/></a:xfrm><a:prstGeom prst="bentConnector3"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill><a:prstDash val="dashDot"/><a:headEnd type="stealth"/><a:tailEnd type="diamond"/></a:ln></wps:spPr>
                  </wps:wsp>
                </wpg:wgp>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected paragraph"),
        };
        let image = paragraph.runs[0].image.as_ref().expect("group placeholder");

        assert_eq!(image.placeholder_label, None);
        assert_eq!(image.description.as_deref(), Some("Navigation controls"));
        assert_eq!(image.relationship_id, None);
        assert_eq!(image.width_px, Some(192.0));
        assert_eq!(image.height_px, Some(96.0));
        assert_eq!(image.layout.relative_height, Some(42));
        assert_eq!(image.layout.kind, RunImageKind::Floating);
        let group = image.shape_group.as_ref().expect("shape group");
        assert_eq!(group.child_offset_x, 100.0);
        assert_eq!(group.child_offset_y, 200.0);
        assert_eq!(group.child_extent_width, 1000.0);
        assert_eq!(group.child_extent_height, 500.0);
        assert_eq!(group.rotation_deg, 15.0);
        assert!(group.flip_horizontal);
        assert!(!group.flip_vertical);
        assert_eq!(group.children.len(), 2);
        assert_eq!(group.children[0].shape, TextBoxShape::RoundedRectangle);
        assert_eq!(group.children[0].x, 200.0);
        assert_eq!(group.children[0].y, 250.0);
        assert_eq!(group.children[0].width, 400.0);
        assert_eq!(group.children[0].height, 100.0);
        assert_eq!(group.children[0].rotation_deg, 30.0);
        assert!(!group.children[0].flip_horizontal);
        assert!(group.children[0].flip_vertical);
        assert_eq!(group.children[0].text, "Grouped label");
        assert!((group.children[0].insets.left_px - 10.0).abs() < 0.001);
        assert!((group.children[0].insets.top_px - 20.0).abs() < 0.001);
        assert!((group.children[0].insets.right_px - 30.0).abs() < 0.001);
        assert!((group.children[0].insets.bottom_px - 40.0).abs() < 0.001);
        assert_eq!(
            group.children[0].vertical_alignment,
            TextBoxVerticalAlignment::Center
        );
        let hyperlink = group.children[0]
            .hyperlink
            .as_ref()
            .expect("child hyperlink");
        assert_eq!(hyperlink.relationship_id.as_deref(), Some("rId5"));
        assert_eq!(hyperlink.tooltip.as_deref(), Some("Home"));
        assert_eq!(hyperlink.target_frame.as_deref(), Some("_top"));
        assert_eq!(group.children[0].fill_color, None);
        assert!((group.children[0].fill_opacity.expect("fill opacity") - 0.65).abs() < 0.000_001);
        assert_eq!(
            group.children[0].fill_theme_color.as_deref(),
            Some("accent1")
        );
        assert_eq!(
            group.children[0].fill_theme_transforms,
            vec![
                ThemeColorTransform::LuminanceMod(0.6),
                ThemeColorTransform::LuminanceOffset(0.4),
            ]
        );
        assert_eq!(
            group.children[0]
                .border
                .as_ref()
                .and_then(|border| border.color.as_deref()),
            None
        );
        assert_eq!(
            group.children[0]
                .border
                .as_ref()
                .and_then(|border| border.theme_color.as_deref()),
            Some("accent2")
        );
        assert_eq!(
            group.children[0]
                .border
                .as_ref()
                .map(|border| border.theme_transforms.as_slice()),
            Some([ThemeColorTransform::Shade(0.5)].as_slice())
        );
        assert!(
            (group.children[0]
                .border
                .as_ref()
                .and_then(|border| border.opacity)
                .expect("line opacity")
                - 0.45)
                .abs()
                < 0.000_001
        );
        let shadow = group.children[0].shadow.as_ref().expect("outer shadow");
        assert_eq!(shadow.color, None);
        assert_eq!(shadow.theme_color.as_deref(), Some("accent2"));
        assert_eq!(
            shadow.theme_transforms,
            vec![ThemeColorTransform::Shade(0.5)]
        );
        assert!((shadow.opacity - 0.6).abs() < 0.000_001);
        assert!((shadow.blur_radius_px - 4.0).abs() < 0.000_001);
        assert!((shadow.distance_px - 2.0).abs() < 0.000_001);
        assert!((shadow.direction_deg - 180.0).abs() < 0.000_001);
        let inner_shadow = group.children[0]
            .inner_shadow
            .as_ref()
            .expect("inner shadow");
        assert_eq!(inner_shadow.color, None);
        assert_eq!(inner_shadow.theme_color.as_deref(), Some("accent1"));
        assert_eq!(
            inner_shadow.theme_transforms,
            vec![ThemeColorTransform::Tint(0.25)]
        );
        assert!((inner_shadow.opacity - 0.2).abs() < 0.000_001);
        assert!((inner_shadow.blur_radius_px - 3.0).abs() < 0.000_001);
        assert!((inner_shadow.distance_px - 1.0).abs() < 0.000_001);
        assert!(inner_shadow.direction_deg.abs() < 0.000_001);
        let glow = group.children[0].glow.as_ref().expect("glow");
        assert_eq!(glow.color, None);
        assert_eq!(glow.theme_color.as_deref(), Some("accent1"));
        assert_eq!(glow.theme_transforms, vec![ThemeColorTransform::Tint(0.5)]);
        assert!((glow.opacity - 0.3).abs() < 0.000_001);
        assert!((glow.radius_px - 6.0).abs() < 0.000_001);
        assert!((group.children[0].blur_radius_px.expect("blur") - 2.0).abs() < 0.000_001);
        assert!(
            (group.children[0].soft_edge_radius_px.expect("soft edge") - 3.0).abs() < 0.000_001
        );
        assert_eq!(group.children[1].fill_color.as_deref(), Some("#FF0000"));
        assert_eq!(group.children[1].fill_theme_color, None);
        assert_eq!(group.children[1].shape, TextBoxShape::LineBent);
        assert_eq!(
            group.children[1]
                .border
                .as_ref()
                .and_then(|border| border.color.as_deref()),
            Some("#0000FF")
        );
        assert_eq!(
            group.children[1]
                .border
                .as_ref()
                .and_then(|border| border.theme_color.as_deref()),
            None
        );
        assert_eq!(
            group.children[1]
                .border
                .as_ref()
                .map(|border| border.style.clone()),
            Some(TextBoxBorderStyle::DashDot)
        );
        assert_eq!(
            group.children[1]
                .border
                .as_ref()
                .and_then(|border| border.head_end),
            Some(TextBoxLineEnd::Stealth)
        );
        assert_eq!(
            group.children[1]
                .border
                .as_ref()
                .and_then(|border| border.tail_end),
            Some(TextBoxLineEnd::Diamond)
        );
    }

    #[test]
    fn parses_nested_shape_groups_with_parent_coordinates_and_paint_order() {
        let body = r#"
    <w:p><w:r><w:drawing>
      <wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                 xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"
                 xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
        <wp:extent cx="952500" cy="952500"/>
        <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup">
          <wpg:wgp>
            <wpg:grpSpPr><a:xfrm><a:chOff x="0" y="0"/><a:chExt cx="1000" cy="1000"/></a:xfrm></wpg:grpSpPr>
            <wps:wsp><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm><a:prstGeom prst="rect"/></wps:spPr></wps:wsp>
            <wpg:grpSp>
              <wpg:grpSpPr><a:xfrm rot="2700000" flipV="1"><a:off x="200" y="100"/><a:ext cx="400" cy="300"/><a:chOff x="10" y="20"/><a:chExt cx="200" cy="100"/></a:xfrm></wpg:grpSpPr>
              <wps:wsp><wps:spPr><a:xfrm><a:off x="60" y="45"/><a:ext cx="100" cy="50"/></a:xfrm><a:prstGeom prst="roundRect"/><a:solidFill><a:srgbClr val="00AAFF"/></a:solidFill></wps:spPr></wps:wsp>
            </wpg:grpSp>
            <wps:wsp><wps:spPr><a:xfrm><a:off x="900" y="900"/><a:ext cx="100" cy="100"/></a:xfrm><a:prstGeom prst="ellipse"/></wps:spPr></wps:wsp>
          </wpg:wgp>
        </a:graphicData></a:graphic>
      </wp:inline>
    </w:drawing></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let document_body = document.body.expect("body");
        let group = document_body.elements[0]
            .as_paragraph()
            .expect("paragraph")
            .runs[0]
            .image
            .as_ref()
            .and_then(|image| image.shape_group.as_ref())
            .expect("shape group");

        assert_eq!(group.children.len(), 2);
        assert_eq!(group.groups.len(), 1);
        assert_eq!(group.children[0].z_order, 0);
        assert_eq!(group.groups[0].z_order, 1);
        assert_eq!(group.children[1].z_order, 2);
        let nested = &group.groups[0];
        assert_eq!((nested.x, nested.y), (200.0, 100.0));
        assert_eq!((nested.width, nested.height), (400.0, 300.0));
        assert_eq!((nested.child_offset_x, nested.child_offset_y), (10.0, 20.0));
        assert_eq!(
            (nested.child_extent_width, nested.child_extent_height),
            (200.0, 100.0)
        );
        assert_eq!(nested.rotation_deg, 45.0);
        assert!(nested.flip_vertical);
        assert_eq!(nested.children.len(), 1);
        assert_eq!(nested.children[0].shape, TextBoxShape::RoundedRectangle);
        assert_eq!(nested.children[0].fill_color.as_deref(), Some("#00AAFF"));
    }

    #[test]
    fn yokohama_group_uses_the_anchor_extent_instead_of_child_shape_extents() {
        use std::io::Read;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/yokohama_page1.docx");
        let file = std::fs::File::open(fixture).expect("open yokohama fixture");
        let mut archive = zip::ZipArchive::new(file).expect("open docx archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut document_xml)
            .expect("read document.xml");

        let document = DocumentParser::new()
            .parse_document_xml(&document_xml)
            .expect("parse document.xml");
        let body = document.body.expect("body");
        let group = body.elements[0]
            .as_paragraph()
            .and_then(|paragraph| paragraph.runs.first())
            .and_then(|run| run.image.as_ref())
            .expect("grouped shape placeholder");

        assert_eq!(group.placeholder_label, None);
        let shape_group = group.shape_group.as_ref().expect("shape group");
        assert_eq!(shape_group.children.len(), 2);
        assert_eq!(
            shape_group
                .children
                .iter()
                .map(|shape| {
                    shape
                        .hyperlink
                        .as_ref()
                        .and_then(|hyperlink| hyperlink.relationship_id.as_deref())
                })
                .collect::<Vec<_>>(),
            vec![Some("rId5"), Some("rId6")]
        );
        assert!(shape_group
            .children
            .iter()
            .all(|shape| shape.fill_color.is_none() && shape.border.is_none()));
        assert!((group.width_px.expect("group width") - 120.0).abs() < 0.01);
        assert!((group.height_px.expect("group height") - 38.666_666).abs() < 0.01);
        assert_eq!(group.layout.relative_height, Some(251_658_240));
        assert_eq!(group.layout.kind, RunImageKind::Floating);
    }

    #[test]
    fn preserves_smartart_as_a_visible_placeholder() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"
                     xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
            <wp:extent cx="2743200" cy="1371600"/>
            <wp:docPr id="2" name="SmartArt 1"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram">
                <dgm:relIds r:dm="rId1" r:lo="rId2" r:qs="rId3" r:cs="rId4"/>
              </a:graphicData>
            </a:graphic>
          </wp:inline>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected paragraph"),
        };
        let image = paragraph.runs[0]
            .image
            .as_ref()
            .expect("SmartArt placeholder");

        assert_eq!(image.placeholder_label.as_deref(), Some("SmartArt"));
        // The loader follows the data model to Word's cached drawing.
        assert_eq!(image.diagram_data_relationship_id.as_deref(), Some("rId1"));
        assert_eq!(image.width_px, Some(288.0));
        assert_eq!(image.height_px, Some(144.0));
        assert_eq!(image.layout.kind, RunImageKind::Inline);
    }

    /// Verifies that text boxes embedded inside drawings are parsed and their
    /// plain-text content is captured along with positioning metadata.
    #[test]
    fn parses_text_box_runs() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
                     distT="0" distB="0" distL="0" distR="0"
                     simplePos="0" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
            <wp:extent cx="914400" cy="914400"/>
            <wp:positionH relativeFrom="margin">
              <wp:posOffset>100000</wp:posOffset>
            </wp:positionH>
            <wp:positionV relativeFrom="line">
              <wp:posOffset>200000</wp:posOffset>
            </wp:positionV>
            <wps:bodyPr xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
                        lIns="190500" tIns="95250" rIns="285750" bIns="381000" anchor="ctr"/>
            <wp:txbxContent>
              <w:p>
                <w:r><w:t>Text box line 1</w:t></w:r>
              </w:p>
              <w:p>
                <w:r><w:t>Line 2</w:t></w:r>
              </w:p>
            </wp:txbxContent>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        let text_box = run.text_box.as_ref().expect("text box metadata");

        assert!(matches!(text_box.layout.kind, RunImageKind::Floating));
        assert_eq!(text_box.text, "Text box line 1\nLine 2");
        assert_eq!(text_box.blocks.len(), 2);
        assert!((text_box.insets.left_px - 20.0).abs() < f64::EPSILON);
        assert!((text_box.insets.top_px - 10.0).abs() < f64::EPSILON);
        assert!((text_box.insets.right_px - 30.0).abs() < f64::EPSILON);
        assert!((text_box.insets.bottom_px - 40.0).abs() < f64::EPSILON);
        assert_eq!(
            text_box.vertical_alignment,
            TextBoxVerticalAlignment::Center
        );
        match &text_box.blocks[0] {
            BodyElement::Paragraph(p) => {
                let paragraph = p.as_ref();
                assert_eq!(paragraph.runs[0].text, "Text box line 1");
            }
            _ => panic!("expected paragraph in text box"),
        }
        let pos_h = text_box
            .layout
            .position_h
            .as_ref()
            .expect("horizontal position");
        assert_eq!(pos_h.relative_from.as_deref(), Some("margin"));
        let pos_v = text_box
            .layout
            .position_v
            .as_ref()
            .expect("vertical position");
        assert_eq!(pos_v.relative_from.as_deref(), Some("line"));
    }

    #[test]
    fn parses_drawingml_justified_and_distributed_text_anchors() {
        for (anchor, expected) in [
            ("just", TextBoxVerticalAlignment::Justified),
            ("dist", TextBoxVerticalAlignment::Distributed),
        ] {
            let element = BytesStart::new("wps:bodyPr").with_attributes([("anchor", anchor)]);
            let mut insets = TextBoxInsets::default();
            let mut alignment = TextBoxVerticalAlignment::Top;

            DocumentParser::populate_text_box_body_properties(
                &mut insets,
                &mut alignment,
                &element,
            );

            assert_eq!(alignment, expected);
        }
    }

    #[test]
    fn preserves_common_wordprocessing_shape_geometry_without_text() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
            <wp:extent cx="1828800" cy="914400"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                <wps:wsp>
                  <wps:spPr>
                    <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
                    <a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>
                    <a:ln w="19050"><a:solidFill><a:srgbClr val="203864"/></a:solidFill></a:ln>
                  </wps:spPr>
                  <wps:bodyPr/>
                </wps:wsp>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected paragraph"),
        };
        // A shape does not need `wps:txbx` or `w:txbxContent`. Word still
        // displays its fill and outline, so dropping it loses visible content.
        let shape = paragraph.runs[0]
            .text_box
            .as_ref()
            .expect("textless WordprocessingShape");
        assert_eq!(shape.shape, TextBoxShape::Ellipse);
        assert_eq!(shape.width_px, Some(192.0));
        assert_eq!(shape.height_px, Some(96.0));
        assert!(shape.blocks.is_empty());
        assert_eq!(shape.fill_color.as_deref(), Some("#4472C4"));
    }

    #[test]
    fn preserves_rounded_rectangle_geometry_for_shape_text() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
            <wp:extent cx="1828800" cy="914400"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                <wps:wsp>
                  <wps:spPr>
                    <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
                  </wps:spPr>
                  <wps:txbx>
                    <w:txbxContent><w:p><w:r><w:t>Rounded</w:t></w:r></w:p></w:txbxContent>
                  </wps:txbx>
                  <wps:bodyPr/>
                </wps:wsp>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected paragraph"),
        };
        let shape = paragraph.runs[0].text_box.as_ref().expect("rounded shape");
        assert_eq!(shape.text, "Rounded");
        assert_eq!(shape.shape, TextBoxShape::RoundedRectangle);
    }

    #[test]
    fn preserves_entity_references_and_no_fill_in_nested_text_box_content() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
            <wp:extent cx="1828800" cy="914400"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                <wps:wsp>
                  <wps:spPr>
                    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                    <a:noFill/>
                    <a:ln w="0"><a:noFill/><a:prstDash val="solid"/></a:ln>
                  </wps:spPr>
                  <wps:txbx>
                    <w:txbxContent><w:p><w:r><w:t>A &amp; B &#x3C; C</w:t></w:r></w:p></w:txbxContent>
                  </wps:txbx>
                  <wps:bodyPr/>
                </wps:wsp>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = body.elements[0].as_paragraph().expect("host paragraph");
        let text_box = paragraph.runs[0]
            .text_box
            .as_ref()
            .expect("WordprocessingShape text box");
        assert_eq!(text_box.text, "A & B < C");
        assert_eq!(text_box.border.width_px, Some(0.0));
        assert_eq!(text_box.border.style, TextBoxBorderStyle::None);
        let inner = text_box.blocks[0]
            .as_paragraph()
            .expect("text-box paragraph");
        assert_eq!(inner.runs[0].text, "A & B < C");
    }

    #[test]
    fn recognizes_diamond_and_triangle_preset_geometry() {
        // These presets are emitted directly by Word's Insert Shape menu.
        // Treating them as rectangles loses both the visible silhouette and
        // the original `a:prstGeom@prst` value on the next export.
        for (preset, expected) in [
            ("diamond", TextBoxShape::Diamond),
            ("triangle", TextBoxShape::Triangle),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must retain its preset geometry",
            );
        }
    }

    #[test]
    fn recognizes_right_triangle_parallelogram_and_trapezoid_preset_geometry() {
        // These are distinct DrawingML presets, not aliases for `rect`. Keeping
        // the exact geometry is important twice: the browser needs the Word
        // silhouette, and export must write the same `a:prstGeom@prst` value.
        for (preset, expected) in [
            ("rtTriangle", TextBoxShape::RightTriangle),
            ("parallelogram", TextBoxShape::Parallelogram),
            ("trapezoid", TextBoxShape::Trapezoid),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn recognizes_pentagon_hexagon_and_octagon_preset_geometry() {
        for (preset, expected) in [
            ("pentagon", TextBoxShape::Pentagon),
            ("hexagon", TextBoxShape::Hexagon),
            ("octagon", TextBoxShape::Octagon),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn recognizes_plus_and_every_star_ribbon_preset_geometry() {
        // Word's Insert Shapes menu emits these exact DrawingML preset names.
        // A rectangle fallback fills the concave corners and loses the preset
        // on export, so both parser identity and renderer geometry matter.
        for (preset, expected) in [
            ("plus", TextBoxShape::Plus),
            ("star4", TextBoxShape::FourPointStar),
            ("star5", TextBoxShape::FivePointStar),
            ("star6", TextBoxShape::SixPointStar),
            ("star7", TextBoxShape::SevenPointStar),
            ("star8", TextBoxShape::EightPointStar),
            ("star10", TextBoxShape::TenPointStar),
            ("star12", TextBoxShape::TwelvePointStar),
            ("star16", TextBoxShape::SixteenPointStar),
            ("star24", TextBoxShape::TwentyFourPointStar),
            ("star32", TextBoxShape::ThirtyTwoPointStar),
            ("irregularSeal1", TextBoxShape::IrregularSealOne),
            ("irregularSeal2", TextBoxShape::IrregularSealTwo),
            ("ribbon", TextBoxShape::Ribbon),
            ("ribbon2", TextBoxShape::RibbonTwo),
            ("wave", TextBoxShape::Wave),
            ("doubleWave", TextBoxShape::DoubleWave),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn recognizes_every_genoffice_basic_shape_preset_geometry() {
        for (preset, expected) in [
            ("mathPlus", TextBoxShape::MathPlus),
            ("leftBrace", TextBoxShape::LeftBrace),
            ("rightBrace", TextBoxShape::RightBrace),
            ("blockArc", TextBoxShape::BlockArc),
            ("pie", TextBoxShape::Pie),
            ("chord", TextBoxShape::Chord),
            ("teardrop", TextBoxShape::Teardrop),
            ("frame", TextBoxShape::Frame),
            ("halfFrame", TextBoxShape::HalfFrame),
            ("corner", TextBoxShape::Corner),
            ("diagStripe", TextBoxShape::DiagonalStripe),
            ("donut", TextBoxShape::Donut),
            ("noSmoking", TextBoxShape::NoSmoking),
            ("foldedCorner", TextBoxShape::FoldedCorner),
            ("bevel", TextBoxShape::Bevel),
            ("cube", TextBoxShape::Cube),
            ("can", TextBoxShape::Can),
            ("lightningBolt", TextBoxShape::LightningBolt),
            ("heart", TextBoxShape::Heart),
            ("sun", TextBoxShape::Sun),
            ("moon", TextBoxShape::Moon),
            ("cloud", TextBoxShape::Cloud),
            ("arc", TextBoxShape::Arc),
            ("plaque", TextBoxShape::Plaque),
            ("smileyFace", TextBoxShape::SmileyFace),
            ("leftBracket", TextBoxShape::LeftBracket),
            ("rightBracket", TextBoxShape::RightBracket),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
            assert_eq!(expected.preset_name(), preset);
        }
    }

    #[test]
    fn recognizes_every_flowchart_preset_geometry() {
        // GenOffice exposes this complete category. Preserve every OOXML
        // preset identity so loading and exporting does not replace a
        // diagnostic flowchart symbol with a generic rectangle.
        for (preset, expected) in [
            ("flowChartProcess", TextBoxShape::FlowChartProcess),
            (
                "flowChartAlternateProcess",
                TextBoxShape::FlowChartAlternateProcess,
            ),
            ("flowChartDecision", TextBoxShape::FlowChartDecision),
            (
                "flowChartPredefinedProcess",
                TextBoxShape::FlowChartPredefinedProcess,
            ),
            (
                "flowChartInternalStorage",
                TextBoxShape::FlowChartInternalStorage,
            ),
            ("flowChartDocument", TextBoxShape::FlowChartDocument),
            (
                "flowChartMultidocument",
                TextBoxShape::FlowChartMultidocument,
            ),
            ("flowChartTerminator", TextBoxShape::FlowChartTerminator),
            ("flowChartPreparation", TextBoxShape::FlowChartPreparation),
            ("flowChartManualInput", TextBoxShape::FlowChartManualInput),
            (
                "flowChartManualOperation",
                TextBoxShape::FlowChartManualOperation,
            ),
            ("flowChartConnector", TextBoxShape::FlowChartConnector),
            (
                "flowChartOffpageConnector",
                TextBoxShape::FlowChartOffpageConnector,
            ),
            ("flowChartMagneticDisk", TextBoxShape::FlowChartMagneticDisk),
            ("flowChartMagneticDrum", TextBoxShape::FlowChartMagneticDrum),
            ("flowChartDisplay", TextBoxShape::FlowChartDisplay),
            ("flowChartDelay", TextBoxShape::FlowChartDelay),
            ("flowChartOr", TextBoxShape::FlowChartOr),
            ("flowChartCollate", TextBoxShape::FlowChartCollate),
            ("flowChartSort", TextBoxShape::FlowChartSort),
            ("flowChartExtract", TextBoxShape::FlowChartExtract),
            ("flowChartMerge", TextBoxShape::FlowChartMerge),
            (
                "flowChartSummingJunction",
                TextBoxShape::FlowChartSummingJunction,
            ),
            ("flowChartPunchedTape", TextBoxShape::FlowChartPunchedTape),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn recognizes_every_callout_preset_geometry() {
        for (preset, expected) in [
            ("wedgeRectCallout", TextBoxShape::WedgeRectangleCallout),
            (
                "wedgeRoundRectCallout",
                TextBoxShape::WedgeRoundedRectangleCallout,
            ),
            ("wedgeEllipseCallout", TextBoxShape::WedgeEllipseCallout),
            ("cloudCallout", TextBoxShape::CloudCallout),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn connector_preset_and_arrow_ends_resolve_to_public_line_kinds() {
        for (preset, arrow_ends, expected, expected_head, expected_tail) in [
            ("line", "", TextBoxShape::Line, None, None),
            (
                "straightConnector1",
                r#"<a:tailEnd type="triangle"/>"#,
                TextBoxShape::LineArrow,
                None,
                Some(TextBoxLineEnd::Triangle),
            ),
            (
                "straightConnector1",
                r#"<a:headEnd type="stealth"/>"#,
                TextBoxShape::LineArrow,
                Some(TextBoxLineEnd::Stealth),
                None,
            ),
            (
                "straightConnector1",
                r#"<a:headEnd type="diamond"/><a:tailEnd type="oval"/>"#,
                TextBoxShape::LineArrowDouble,
                Some(TextBoxLineEnd::Diamond),
                Some(TextBoxLineEnd::Oval),
            ),
            (
                "bentConnector3",
                r#"<a:headEnd type="arrow"/>"#,
                TextBoxShape::LineBent,
                Some(TextBoxLineEnd::Arrow),
                None,
            ),
            (
                "curvedConnector3",
                r#"<a:tailEnd type="triangle"/>"#,
                TextBoxShape::LineCurved,
                None,
                Some(TextBoxLineEnd::Triangle),
            ),
        ] {
            let body = format!(
                r#"<w:p><w:r><w:drawing>
                <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                  xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                  <wp:extent cx="1828800" cy="914400"/>
                  <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                    <wps:wsp><wps:spPr>
                      <a:prstGeom prst="{preset}"><a:avLst/></a:prstGeom>
                      <a:noFill/><a:ln><a:solidFill><a:srgbClr val="000000"/></a:solidFill>{arrow_ends}</a:ln>
                    </wps:spPr><wps:bodyPr/></wps:wsp>
                  </a:graphicData></a:graphic>
                </wp:anchor>
                </w:drawing></w:r></w:p>"#
            );
            let document = parse_document_from_body(&body);
            let body = document.body.expect("body");
            let paragraph = match &body.elements[0] {
                BodyElement::Paragraph(paragraph) => paragraph,
                _ => panic!("expected paragraph"),
            };
            let line = paragraph.runs[0]
                .text_box
                .as_ref()
                .expect("stroke-only connector");
            assert_eq!(line.shape, expected, "{preset} {arrow_ends}");
            assert_eq!(line.border.head_end, expected_head, "{preset} head marker");
            assert_eq!(line.border.tail_end, expected_tail, "{preset} tail marker");
            assert!(line.blocks.is_empty());
            assert!(line.fill_color.is_none());
        }
    }

    #[test]
    fn recognizes_every_exact_block_arrow_preset_geometry() {
        // These block-arrow presets have diagnostic concave/pointed edges.
        // Flattening them to `rect` both changes the visible object and
        // destroys the author's chosen preset during a load/export cycle.
        for (preset, expected) in [
            ("rightArrow", TextBoxShape::RightArrow),
            ("leftArrow", TextBoxShape::LeftArrow),
            ("upArrow", TextBoxShape::UpArrow),
            ("downArrow", TextBoxShape::DownArrow),
            ("leftRightArrow", TextBoxShape::LeftRightArrow),
            ("upDownArrow", TextBoxShape::UpDownArrow),
            ("notchedRightArrow", TextBoxShape::NotchedRightArrow),
            ("chevron", TextBoxShape::Chevron),
            ("homePlate", TextBoxShape::HomePlate),
            ("quadArrow", TextBoxShape::QuadArrow),
            ("bentArrow", TextBoxShape::BentArrow),
            ("uturnArrow", TextBoxShape::UturnArrow),
            ("curvedRightArrow", TextBoxShape::CurvedRightArrow),
            ("stripedRightArrow", TextBoxShape::StripedRightArrow),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must not be flattened to a rectangle",
            );
        }
    }

    #[test]
    fn recognizes_every_rectangular_gallery_preset_geometry() {
        for (preset, expected) in [
            ("rect", TextBoxShape::Rectangle),
            ("roundRect", TextBoxShape::RoundedRectangle),
            ("snip1Rect", TextBoxShape::SnipOneRectangle),
            ("snip2SameRect", TextBoxShape::SnipTwoSameRectangle),
            ("snip2DiagRect", TextBoxShape::SnipTwoDiagonalRectangle),
            ("snipRoundRect", TextBoxShape::SnipRoundRectangle),
            ("round1Rect", TextBoxShape::RoundOneRectangle),
            ("round2SameRect", TextBoxShape::RoundTwoSameRectangle),
            ("round2DiagRect", TextBoxShape::RoundTwoDiagonalRectangle),
        ] {
            let mut geometry = BytesStart::new("a:prstGeom");
            geometry.push_attribute(("prst", preset));
            assert_eq!(
                DocumentParser::parse_text_box_shape(&geometry),
                expected,
                "{preset} must retain its exact corner geometry",
            );
            assert_eq!(expected.preset_name(), preset);
        }
    }

    #[test]
    fn resolves_explicit_shape_color_transforms_in_document_order() {
        let shape = r#"<wps:wsp><wps:spPr>
          <a:prstGeom prst="rect"/>
          <a:solidFill><a:srgbClr val="0000FF"><a:shade val="50000"/><a:tint val="50000"/></a:srgbClr><a:shade val="0"/></a:solidFill>
          <a:ln><a:solidFill><a:srgbClr val="0000FF"><a:lumMod val="50000"/></a:srgbClr></a:solidFill></a:ln>
          <a:effectLst>
            <a:outerShdw><a:srgbClr val="0000FF"><a:shade val="25000"/></a:srgbClr></a:outerShdw>
            <a:innerShdw><a:srgbClr val="0000FF"><a:tint val="25000"/></a:srgbClr></a:innerShdw>
            <a:glow><a:srgbClr val="0000FF"><a:lumOff val="25000"/></a:srgbClr></a:glow>
          </a:effectLst>
        </wps:spPr><wps:bodyPr/></wps:wsp>"#;
        for grouping in 0..=2 {
            let drawing = match grouping {
                0 => shape.to_string(),
                1 => format!("<wpg:wgp>{shape}</wpg:wgp>"),
                _ => format!("<wpg:wgp><wpg:grpSp>{shape}</wpg:grpSp></wpg:wgp>"),
            };
            let document = parse_document_from_body(&format!(
                r#"<w:p><w:r><w:drawing><wp:anchor
                xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
                xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup">
                <wp:extent cx="914400" cy="914400"/><a:graphic><a:graphicData>{drawing}</a:graphicData></a:graphic>
                </wp:anchor></w:drawing></w:r></w:p>"#
            ));
            let body = document.body.expect("body");
            let BodyElement::Paragraph(paragraph) = &body.elements[0] else {
                panic!("expected paragraph");
            };
            let (fill, border, shadow, inner_shadow, glow) = if grouping == 0 {
                let shape = paragraph.runs[0].text_box.as_ref().expect("shape");
                (
                    &shape.fill_color,
                    &shape.border,
                    &shape.shadow,
                    &shape.inner_shadow,
                    &shape.glow,
                )
            } else {
                let group = paragraph.runs[0]
                    .image
                    .as_ref()
                    .unwrap()
                    .shape_group
                    .as_ref()
                    .unwrap();
                let group = if grouping == 2 {
                    &group.groups[0]
                } else {
                    group
                };
                let shape = &group.children[0];
                (
                    &shape.fill_color,
                    shape.border.as_ref().unwrap(),
                    &shape.shadow,
                    &shape.inner_shadow,
                    &shape.glow,
                )
            };
            assert_eq!(fill.as_deref(), Some("#8080BF"), "grouping {grouping}");
            assert_eq!(border.color.as_deref(), Some("#000080"));
            assert_eq!(shadow.as_ref().unwrap().color.as_deref(), Some("#000040"));
            assert_eq!(
                inner_shadow.as_ref().unwrap().color.as_deref(),
                Some("#BFBFFF")
            );
            assert_eq!(glow.as_ref().unwrap().color.as_deref(), Some("#8080FF"));
        }
    }

    fn direct_color_shape(document_color: &str, grouping: usize) -> RunTextBox {
        let shape = format!(
            r#"<wps:wsp><wps:spPr><a:prstGeom prst="rect"/>
            <a:solidFill>{document_color}</a:solidFill>
            <a:ln><a:solidFill>{document_color}</a:solidFill></a:ln>
            <a:effectLst><a:outerShdw>{document_color}</a:outerShdw>
            <a:innerShdw>{document_color}</a:innerShdw><a:glow>{document_color}</a:glow></a:effectLst>
            </wps:spPr><wps:bodyPr/></wps:wsp>"#
        );
        let drawing = match grouping {
            0 => shape,
            1 => format!("<wpg:wgp>{shape}</wpg:wgp>"),
            _ => format!("<wpg:wgp><wpg:grpSp>{shape}</wpg:grpSp></wpg:wgp>"),
        };
        let document = parse_document_from_body(&format!(
            r#"<w:p><w:r><w:drawing><wp:anchor
            xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
            xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
            xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
            xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup">
            <wp:extent cx="914400" cy="914400"/><a:graphic><a:graphicData>{drawing}</a:graphicData></a:graphic>
            </wp:anchor></w:drawing></w:r></w:p>"#
        ));
        let BodyElement::Paragraph(paragraph) = &document.body.as_ref().unwrap().elements[0] else {
            panic!("expected paragraph");
        };
        if grouping == 0 {
            return paragraph.runs[0].text_box.clone().expect("shape");
        }
        let group = paragraph.runs[0]
            .image
            .as_ref()
            .unwrap()
            .shape_group
            .as_ref()
            .unwrap();
        let group = if grouping == 2 {
            &group.groups[0]
        } else {
            group
        };
        let shape = &group.children[0];
        RunTextBox {
            fill_color: shape.fill_color.clone(),
            fill_opacity: shape.fill_opacity,
            border: shape.border.clone().unwrap(),
            shadow: shape.shadow.clone(),
            inner_shadow: shape.inner_shadow.clone(),
            glow: shape.glow.clone(),
            ..RunTextBox::default()
        }
    }

    #[test]
    fn resolves_direct_drawingml_color_models_and_scoped_transforms() {
        for (tag, attributes, base, transformed) in [
            ("srgbClr", "val=\"0000ff\"", "#0000FF", "#8080BF"),
            ("prstClr", "val=\"blue\"", "#0000FF", "#8080BF"),
            ("prstClr", "val=\"dkBlue\"", "#00008B", "#8080A2"),
            ("prstClr", "val=\"darkBlue\"", "#00008B", "#8080A2"),
            ("prstClr", "val=\"green\"", "#008000", "#80A080"),
            ("prstClr", "val=\"gray\"", "#808080", "#A0A0A0"),
            ("prstClr", "val=\"grey\"", "#808080", "#A0A0A0"),
            ("prstClr", "val=\"ltGoldenrodYellow\"", "#FAFAD2", "#BEBEB4"),
            (
                "sysClr",
                "val=\"windowText\" lastClr=\"0000ff\"",
                "#0000FF",
                "#8080BF",
            ),
            (
                "scrgbClr",
                "r=\"0\" g=\"0\" b=\"50000\"",
                "#0000BC",
                "#8080AF",
            ),
            (
                "scrgbClr",
                "r=\"0%\" g=\"0%\" b=\"50.000%\"",
                "#0000BC",
                "#8080AF",
            ),
            (
                "hslClr",
                "hue=\"14400000\" sat=\"100000\" lum=\"50000\"",
                "#0000FF",
                "#8080BF",
            ),
            (
                "hslClr",
                "hue=\"14400000\" sat=\"100.000%\" lum=\"50.000%\"",
                "#0000FF",
                "#8080BF",
            ),
        ] {
            for grouping in 0..=2 {
                for (color, expected, alpha) in [
                    (format!("<a:{tag} {attributes}/>"), base, None),
                    (format!("<a:{tag} {attributes}><a:shade val=\"50000\"/><a:tint val=\"50000\"/><a:alpha val=\"37500\"/></a:{tag}><a:shade val=\"0\"/>"), transformed, Some(0.375)),
                ] {
                    let shape = direct_color_shape(&color, grouping);
                    assert_eq!(shape.fill_color.as_deref(), Some(expected), "{tag}, group {grouping}, {color}");
                    assert_eq!(shape.border.color.as_deref(), Some(expected));
                    assert_eq!(shape.shadow.as_ref().unwrap().color.as_deref(), Some(expected));
                    assert_eq!(shape.inner_shadow.as_ref().unwrap().color.as_deref(), Some(expected));
                    assert_eq!(shape.glow.as_ref().unwrap().color.as_deref(), Some(expected));
                    assert_eq!(shape.fill_opacity, alpha);
                    assert_eq!(shape.border.opacity, alpha);
                }
            }
        }
    }

    #[test]
    fn rejects_invalid_direct_drawingml_colors_without_leaking_transforms() {
        for malformed in [
            "<a:prstClr val=\"Blue\"/>",
            "<a:prstClr val=\"rebeccapurple\"/>",
            "<a:prstClr val=\"constructor\"/>",
            "<a:srgbClr val=\"nothex\"/>",
            "<a:srgbClr val=\"💙000\"/>",
            "<a:sysClr val=\"windowText\"/>",
            "<a:sysClr val=\"windowText\" lastClr=\"ZZ0000\"/>",
            "<a:scrgbClr r=\"0\" g=\"0\"/>",
            "<a:scrgbClr r=\"NaN%\" g=\"0\" b=\"0\"/>",
            "<a:hslClr hue=\"21600000\" sat=\"100000\" lum=\"50000\"/>",
            "<a:hslClr hue=\"0\" sat=\"100000\"/>",
        ] {
            let tag = malformed
                .split_whitespace()
                .next()
                .unwrap()
                .trim_start_matches('<');
            let nested = malformed.replace("/>", &format!("><a:tint val=\"0\"/></{tag}>"));
            for grouping in 0..=2 {
                for invalid in [malformed, &nested] {
                    let shape = direct_color_shape(
                        &format!("<a:srgbClr val=\"112233\"/>{invalid}<a:shade val=\"0\"/>"),
                        grouping,
                    );
                    assert_eq!(
                        shape.fill_color.as_deref(),
                        Some("#112233"),
                        "group {grouping}: {invalid}"
                    );
                    assert_eq!(shape.border.color.as_deref(), Some("#112233"));
                    assert_eq!(
                        shape.shadow.as_ref().unwrap().color.as_deref(),
                        Some("#112233")
                    );
                    assert_eq!(
                        shape.inner_shadow.as_ref().unwrap().color.as_deref(),
                        Some("#112233")
                    );
                    assert_eq!(
                        shape.glow.as_ref().unwrap().color.as_deref(),
                        Some("#112233")
                    );
                }
            }
        }
    }

    #[test]
    fn preserves_shape_fill_and_outline_theme_color_references() {
        let body = r#"
    <w:p>
      <w:r>
        <w:drawing>
          <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                     xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                     xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
            <wp:extent cx="1828800" cy="914400"/>
            <a:graphic>
              <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                <wps:wsp>
                  <wps:spPr>
                    <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
                    <a:solidFill>
                      <a:schemeClr val="accent1">
                        <a:lumMod val="60000"/>
                        <a:lumOff val="40000"/>
                        <a:shade val="50000"/>
                        <a:tint val="25000"/>
                      </a:schemeClr>
                    </a:solidFill>
                    <a:ln w="19050">
                      <a:solidFill>
                        <a:schemeClr val="accent2"><a:lumMod val="50000"/></a:schemeClr>
                      </a:solidFill>
                    </a:ln>
                  </wps:spPr>
                  <wps:bodyPr/>
                </wps:wsp>
              </a:graphicData>
            </a:graphic>
          </wp:anchor>
        </w:drawing>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph,
            _ => panic!("expected paragraph"),
        };
        let shape = paragraph.runs[0].text_box.as_ref().expect("theme shape");
        // document.xml is parsed before theme1.xml. Keep the semantic slots so
        // package loading can resolve the document's own theme rather than
        // assuming Office's default palette.
        assert_eq!(shape.fill_color, None);
        assert_eq!(shape.fill_theme_color.as_deref(), Some("accent1"));
        // DrawingML applies child transforms in document order. Keeping all
        // four here is necessary before package theme resolution can match
        // Word for generated tint/shade combinations.
        assert_eq!(shape.fill_theme_transforms.len(), 4);
        assert_eq!(
            shape.fill_theme_transforms,
            vec![
                ThemeColorTransform::LuminanceMod(0.6),
                ThemeColorTransform::LuminanceOffset(0.4),
                ThemeColorTransform::Shade(0.5),
                ThemeColorTransform::Tint(0.25),
            ]
        );
        assert_eq!(shape.border.color, None);
        assert_eq!(shape.border.theme_color.as_deref(), Some("accent2"));
        assert_eq!(
            shape.border.theme_transforms,
            vec![ThemeColorTransform::LuminanceMod(0.5)]
        );
    }

    #[test]
    fn parses_text_box_fixture() {
        let data = std::fs::read(fixture_path("textbox.docx")).expect("read fixture");
        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor).expect("zip archive");
        let mut file = archive.by_name("word/document.xml").expect("document.xml");
        let mut xml = String::new();
        file.read_to_string(&mut xml).expect("read xml");

        let parser = DocumentParser::new();
        let document = parser.parse_document_xml(&xml).expect("parse document");
        let body = document.body.expect("body");

        let mut found = None;
        let mut debug_runs = Vec::new();
        for element in body.elements {
            if let BodyElement::Paragraph(paragraph) = element {
                let paragraph = *paragraph;
                if paragraph.runs.is_empty() {
                    continue;
                }
                debug_runs.push(
                    paragraph
                        .runs
                        .iter()
                        .map(|run| {
                            (
                                run.text.clone(),
                                run.image.is_some(),
                                run.text_box.is_some(),
                            )
                        })
                        .collect::<Vec<_>>(),
                );
                for run in paragraph.runs {
                    if let Some(text_box) = run.text_box {
                        found = Some(text_box);
                        break;
                    }
                    if run.image.is_some() {
                        // Ensure we don't misclassify drawing runs.
                        continue;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }

        let text_box = found.unwrap_or_else(|| {
            panic!("text box not found. runs: {:?}", debug_runs);
        });
        assert!(matches!(text_box.layout.kind, RunImageKind::Floating));
        assert_eq!(text_box.layout.wrap, WrapMode::TopAndBottom);
        assert!((text_box.layout.dist_top.expect("dist_top") - 16.0).abs() < 0.5);
        assert!((text_box.layout.dist_bottom.expect("dist_bottom") - 16.0).abs() < 0.5);
        assert!(text_box.width_px.unwrap() > 0.0);
        assert_eq!(text_box.blocks.len(), 1);
        assert!(text_box.text.contains("Hello"));
    }

    #[test]
    fn parses_text_box_border_styles() {
        let data = std::fs::read(fixture_path("textbox-dashed.docx")).expect("read dashed fixture");
        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor).expect("zip archive");
        let mut file = archive.by_name("word/document.xml").expect("document.xml");
        let mut xml = String::new();
        file.read_to_string(&mut xml).expect("read xml");

        let parser = DocumentParser::new();
        let document = parser.parse_document_xml(&xml).expect("parse document");
        let body = document.body.expect("body");

        let mut text_box = None;
        for element in body.elements {
            if let BodyElement::Paragraph(paragraph) = element {
                for run in paragraph.runs {
                    if let Some(tb) = run.text_box {
                        text_box = Some(tb);
                        break;
                    }
                }
            }
            if text_box.is_some() {
                break;
            }
        }

        let text_box = text_box.expect("text box not found");
        assert!(matches!(text_box.layout.kind, RunImageKind::Floating));
        let width = text_box.border.width_px.unwrap_or_default();
        assert!((width - 6.6).abs() < 0.5);
        assert_eq!(text_box.border.style, TextBoxBorderStyle::Dotted);
        assert_eq!(text_box.border.color.as_deref(), Some("#000000"));
        assert_eq!(text_box.fill_color.as_deref(), None);
    }

    #[test]
    fn vml_text_box_stroke_defaults_and_authored_overrides() {
        for (shape_attributes, stroke, width, style, color) in [
            ("", "", 1.0, TextBoxBorderStyle::Solid, None),
            (
                "strokeweight=\"3pt\" strokecolor=\"#123456\"",
                "",
                4.0,
                TextBoxBorderStyle::Solid,
                Some("#123456"),
            ),
            (
                "strokeweight=\"3pt\"",
                "<v:stroke weight=\"6pt\"/>",
                8.0,
                TextBoxBorderStyle::Solid,
                None,
            ),
            (
                "",
                "<v:stroke weight=\"6pt\"></v:stroke>",
                8.0,
                TextBoxBorderStyle::Solid,
                None,
            ),
            (
                "stroked=\"f\"",
                "<v:stroke weight=\"6pt\"/>",
                8.0,
                TextBoxBorderStyle::None,
                None,
            ),
            (
                "",
                "<v:stroke on=\"false\"/>",
                1.0,
                TextBoxBorderStyle::None,
                None,
            ),
            (
                "",
                "<v:stroke on=\"0\"></v:stroke>",
                1.0,
                TextBoxBorderStyle::None,
                None,
            ),
        ] {
            let xml = format!(
                r#"<w:p><w:r><w:pict>
              <v:shape style="width:200pt;height:100pt" {shape_attributes}>
                {stroke}<v:textbox><w:txbxContent>
                  <w:p><w:r><w:t>VML TextBox</w:t></w:r></w:p>
                </w:txbxContent></v:textbox>
              </v:shape></w:pict></w:r></w:p>"#
            );
            let document = parse_document_from_body(&xml);
            let body = document.body.expect("body");
            let BodyElement::Paragraph(paragraph) = &body.elements[0] else {
                panic!("expected paragraph");
            };
            let text_box = paragraph
                .runs
                .iter()
                .find_map(|run| run.text_box.as_ref())
                .expect("text box");
            assert_eq!(text_box.border.width_px, Some(width), "{xml}");
            assert_eq!(text_box.border.style, style, "{xml}");
            assert_eq!(text_box.border.color.as_deref(), color, "{xml}");
        }
    }

    #[test]
    fn parses_vml_text_box_border_styles() {
        let body = r##"
        <w:p>
          <w:r>
            <w:pict>
              <v:shape id="_x0000_s1" type="#_x0000_t202" style="width:200pt;height:100pt;v-text-anchor:bottom" stroked="t" fillcolor="#00FF00">
                <v:stroke weight="5pt" dashstyle="shortdot" color="#FF0000"/>
                <v:textbox inset="9pt,6pt,12pt,3pt">
                  <w:txbxContent>
                    <w:p><w:r><w:t>VML TextBox</w:t></w:r></w:p>
                  </w:txbxContent>
                </v:textbox>
              </v:shape>
            </w:pict>
          </w:r>
        </w:p>
        "##;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let mut text_box = None;
        for element in body.elements {
            if let BodyElement::Paragraph(paragraph) = element {
                for run in paragraph.runs {
                    if let Some(tb) = run.text_box {
                        text_box = Some(tb);
                        break;
                    }
                }
            }
            if text_box.is_some() {
                break;
            }
        }

        let text_box = text_box.expect("text box");
        assert_eq!(text_box.border.style, TextBoxBorderStyle::Dotted);
        let width = text_box.border.width_px.expect("width");
        assert!((width - 6.6).abs() < 0.5);
        assert_eq!(text_box.border.color.as_deref(), Some("#FF0000"));
        assert_eq!(text_box.fill_color.as_deref(), Some("#00FF00"));
        assert_eq!(text_box.insets.left_px, 12.0);
        assert_eq!(text_box.insets.top_px, 8.0);
        assert_eq!(text_box.insets.right_px, 16.0);
        assert_eq!(text_box.insets.bottom_px, 4.0);
        assert_eq!(
            text_box.vertical_alignment,
            TextBoxVerticalAlignment::Bottom
        );
    }

    #[test]
    fn parses_alignment_and_spacing() {
        let paragraph = r#"
    <w:p>
      <w:pPr>
        <w:jc w:val="both"/>
        <w:spacing w:before="240" w:beforeLines="50" w:afterLines="100" w:line="480" w:lineRule="auto"/>
      </w:pPr>
      <w:r><w:t>Line spacing sample</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(paragraph);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let props = paragraph.properties.as_ref().expect("paragraph properties");

        assert_eq!(props.alignment.as_deref(), Some("justify"));

        let spacing = props.spacing.as_ref().expect("spacing");
        const EPS: f32 = 0.0001;
        assert!((spacing.before.unwrap() - 12.0).abs() < EPS);
        assert!((spacing.before_lines.unwrap() - 0.5).abs() < EPS);
        assert!((spacing.after_lines.unwrap() - 1.0).abs() < EPS);
        assert!((spacing.line.unwrap() - 2.0).abs() < EPS);
        assert_eq!(spacing.line_rule.as_deref(), Some("auto"));
    }

    #[test]
    fn paragraph_mark_revision_is_read_from_the_mark_run_properties() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:rPr><w:del w:id="0" w:author="A" w:date="2025-11-28T15:48:00Z"/></w:rPr>
      </w:pPr>
      <w:r><w:t>Hello</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:rPr><w:ins w:id="1" w:author="B"></w:ins><w:b/></w:rPr></w:pPr>
      <w:del w:id="2" w:author="A"><w:r><w:delText>Foo</w:delText></w:r></w:del>
      <w:r><w:t>Bar</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr><w:rPr><w:b/></w:rPr></w:pPr>
      <w:r><w:rPr><w:b/></w:rPr><w:t>World</w:t></w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraphs: Vec<&Paragraph> = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect();
        let mark = |index: usize| {
            paragraphs[index]
                .properties
                .as_ref()
                .and_then(|props| props.mark_revision.as_ref())
        };
        let deleted = mark(0).expect("deleted mark");
        assert_eq!(deleted.kind, RevisionKind::Deleted);
        assert_eq!(deleted.author.as_deref(), Some("A"));
        assert!(paragraphs[0].runs[0].revision.is_none());
        let inserted = mark(1).expect("inserted mark");
        assert_eq!(inserted.kind, RevisionKind::Inserted);
        // The mark's own revision does not leak into the paragraph's runs.
        assert_eq!(
            paragraphs[1].runs[0]
                .revision
                .as_ref()
                .map(|mark| &mark.kind),
            Some(&RevisionKind::Deleted)
        );
        assert!(paragraphs[1].runs[1].revision.is_none());
        assert!(mark(2).is_none());
    }

    #[test]
    fn paragraph_mark_run_properties_do_not_apply_to_existing_runs() {
        let body = r#"
    <w:p>
      <w:pPr>
        <w:rPr>
          <w:sz w:val="21"/>
          <w:shd w:val="clear" w:fill="FFCCFF"/>
          <w:bdr w:val="single" w:sz="4" w:color="auto"/>
        </w:rPr>
      </w:pPr>
      <w:r><w:t xml:space="preserve">　</w:t></w:r>
      <w:r>
        <w:rPr><w:bdr w:val="single" w:sz="4" w:color="808080"/></w:rPr>
        <w:t>2024.10</w:t>
      </w:r>
    </w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let run = &paragraph.runs[0];
        assert_eq!(run.properties.as_ref().and_then(|p| p.font_size), None);
        assert!(run
            .properties
            .as_ref()
            .and_then(|p| p.shading.as_ref())
            .is_none());
        assert!(run
            .properties
            .as_ref()
            .and_then(|p| p.border.as_ref())
            .is_none());
        let mark_properties = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.mark_run_properties.as_ref())
            .expect("paragraph mark properties");
        assert_eq!(mark_properties.font_size, Some(10.5));
        assert_eq!(
            mark_properties
                .shading
                .as_ref()
                .and_then(|s| s.fill.as_deref()),
            Some("#FFCCFF")
        );
        assert!(mark_properties
            .border
            .as_ref()
            .is_some_and(BorderEdge::is_visible));
        assert!(paragraph.runs[1]
            .properties
            .as_ref()
            .and_then(|p| p.border.as_ref())
            .is_some_and(BorderEdge::is_visible));
    }

    #[test]
    fn parses_page_settings_from_section_properties() {
        let body = r#"
    <w:p><w:r><w:t>dummy</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="1440" w:right="720" w:bottom="1800" w:left="720"
               w:header="432" w:footer="576"/>
      <w:cols w:num="2" w:space="425"/>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let settings = document.page_settings.expect("page settings");
        const EPS: f32 = 0.01;

        assert!((settings.width.unwrap() - 595.3).abs() < EPS);
        assert!((settings.height.unwrap() - 841.9).abs() < EPS);
        assert!((settings.margin_top.unwrap() - 72.0).abs() < EPS);
        assert!((settings.margin_bottom.unwrap() - 90.0).abs() < EPS);
        assert!((settings.margin_left.unwrap() - 36.0).abs() < EPS);
        assert!((settings.header_distance.unwrap() - 21.6).abs() < EPS);
        assert!((settings.footer_distance.unwrap() - 28.8).abs() < EPS);
        assert!((settings.margin_right.unwrap() - 36.0).abs() < EPS);

        assert_eq!(document.sections.len(), 1);
        let section = &document.sections[0];
        assert_eq!(section.start, 0);
        let body = document.body.as_ref().expect("body");
        assert_eq!(section.end, body.elements.len());
        let section_settings = section
            .properties
            .as_ref()
            .and_then(|props| props.page_settings.as_ref())
            .expect("section page settings");
        assert!((section_settings.width.unwrap() - 595.3).abs() < EPS);
        assert!((section_settings.height.unwrap() - 841.9).abs() < EPS);
    }

    #[test]
    fn parses_empty_section_type_element_on_paragraph_section_break() {
        let body = r#"
    <w:p>
      <w:pPr><w:sectPr><w:type w:val="evenPage"/></w:sectPr></w:pPr>
      <w:r><w:t>before</w:t></w:r>
    </w:p>
    <w:p><w:r><w:t>after</w:t></w:r></w:p>
    <w:sectPr/>
"#;

        let document = parse_document_from_body(body);
        assert_eq!(document.sections.len(), 2);
        assert_eq!(
            document.sections[0]
                .properties
                .as_ref()
                .and_then(|section| section.section_type.as_deref()),
            Some("evenPage")
        );
    }

    #[test]
    fn ignores_previous_run_and_section_properties_in_change_records() {
        let body = r#"
            <w:p><w:r>
              <w:rPr>
                <w:b/>
                <w:rPrChange w:id="1"><w:rPr><w:b w:val="false"/></w:rPr></w:rPrChange>
              </w:rPr>
              <w:t>current</w:t>
            </w:r></w:p>
            <w:sectPr>
              <w:pgSz w:w="12000" w:h="16000"/>
              <w:sectPrChange w:id="2"><w:sectPr><w:pgSz w:w="8000" w:h="9000"/></w:sectPr></w:sectPrChange>
            </w:sectPr>
        "#;
        let document = parse_document_from_body(body);
        let run = document
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .expect("run");
        assert_eq!(
            run.properties.as_ref().and_then(|props| props.bold),
            Some(true)
        );
        assert_eq!(
            document.page_settings.as_ref().and_then(|page| page.width),
            Some(600.0)
        );
    }

    #[test]
    fn parses_page_gutter_and_defaults_to_none() {
        // `w:pgMar/@w:gutter` (§17.6.11) is twips → points (÷20).
        let with_gutter = r#"
    <w:p><w:r><w:t>x</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgMar w:top="1440" w:right="720" w:bottom="1440" w:left="720" w:gutter="720"/>
    </w:sectPr>
"#;
        let settings = parse_document_from_body(with_gutter)
            .page_settings
            .expect("page settings");
        // 720 twips ÷ 20 = 36 pt.
        assert!((settings.gutter.expect("gutter") - 36.0).abs() < 0.01);

        // No `w:gutter` attribute → None (the common case, no layout change).
        let no_gutter = r#"
    <w:p><w:r><w:t>x</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgMar w:top="1440" w:right="720" w:bottom="1440" w:left="720"/>
    </w:sectPr>
"#;
        let plain = parse_document_from_body(no_gutter)
            .page_settings
            .expect("page settings");
        assert_eq!(plain.gutter, None);
    }

    #[test]
    fn parses_section_columns() {
        let body = r#"
    <w:p><w:r><w:t>Two columns</w:t></w:r></w:p>
    <w:sectPr>
      <w:cols w:num="2" w:space="425"/>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let section_columns = document
            .default_section
            .as_ref()
            .and_then(|props| props.columns.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.columns.as_ref())
            })
            .expect("section columns");

        assert_eq!(section_columns.count, Some(2));
        const EPS: f32 = 0.001;
        assert!((section_columns.space.unwrap() - 21.25).abs() < EPS);
        assert_eq!(section_columns.columns.len(), 0);
    }

    #[test]
    fn parses_page_borders_all_sides() {
        // `w:pgBorders` (§17.6.10) draws a border around the page. Each side is
        // a `BorderEdge`; `w:offsetFrom` records the inset reference.
        let body = r#"
    <w:p><w:r><w:t>Bordered page</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgBorders w:offsetFrom="page">
        <w:top w:val="single" w:sz="24" w:space="24" w:color="FF0000"/>
        <w:left w:val="double" w:sz="12" w:space="24" w:color="auto"/>
        <w:bottom w:val="single" w:sz="24" w:space="24" w:color="0000FF"/>
        <w:right w:val="dashed" w:sz="12" w:space="24" w:color="00FF00"/>
      </w:pgBorders>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let borders = document
            .default_section
            .as_ref()
            .and_then(|props| props.page_borders.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.page_borders.as_ref())
            })
            .expect("page borders");

        assert_eq!(borders.offset_from.as_deref(), Some("page"));
        let top = borders.top.as_ref().expect("top edge");
        assert_eq!(top.val.as_deref(), Some("single"));
        assert_eq!(top.size_eighths, Some(24));
        assert_eq!(top.color.as_deref(), Some("#FF0000"));
        assert_eq!(
            borders.left.as_ref().unwrap().val.as_deref(),
            Some("double")
        );
        // `w:color="auto"` normalizes to None.
        assert_eq!(borders.left.as_ref().unwrap().color, None);
        assert_eq!(
            borders.bottom.as_ref().unwrap().color.as_deref(),
            Some("#0000FF")
        );
        assert_eq!(
            borders.right.as_ref().unwrap().val.as_deref(),
            Some("dashed")
        );
    }

    #[test]
    fn section_without_page_borders_has_none() {
        let body = r#"
    <w:p><w:r><w:t>No border</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let page_borders = document
            .default_section
            .as_ref()
            .and_then(|props| props.page_borders.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.page_borders.as_ref())
            });
        assert!(page_borders.is_none());
    }

    #[test]
    fn parses_page_number_type_fmt_and_start() {
        // `w:pgNumType` (§17.6.12) sets the numeral format (`w:fmt`) and the
        // displayed first page number (`w:start`) for PAGE fields in the section.
        let body = r#"
    <w:p><w:r><w:t>Roman numbered</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgNumType w:fmt="upperRoman" w:start="5"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let pg_num = document
            .default_section
            .as_ref()
            .and_then(|props| props.page_number_type.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.page_number_type.as_ref())
            })
            .expect("page number type");
        assert_eq!(pg_num.format.as_deref(), Some("upperroman"));
        assert_eq!(pg_num.start, Some(5));
    }

    /// Each section keeps its own `w:pgNumType`: front matter numbered in
    /// lower roman (continuing) and a body that restarts at arabic 1 must not
    /// collapse into one document-wide setting.
    #[test]
    fn page_number_type_is_kept_per_section() {
        let body = r#"
    <w:p><w:r><w:t>Front matter</w:t></w:r></w:p>
    <w:p><w:pPr><w:sectPr>
      <w:pgNumType w:fmt="lowerRoman"/>
      <w:type w:val="nextPage"/>
    </w:sectPr></w:pPr></w:p>
    <w:p><w:r><w:t>Body</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgNumType w:fmt="decimal" w:start="1"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        assert_eq!(document.sections.len(), 2);
        let front = document.sections[0]
            .properties
            .as_ref()
            .and_then(|props| props.page_number_type.as_ref())
            .expect("front matter page number type");
        assert_eq!(front.format.as_deref(), Some("lowerroman"));
        assert_eq!(front.start, None);
        let body_section = document.sections[1]
            .properties
            .as_ref()
            .and_then(|props| props.page_number_type.as_ref())
            .expect("body page number type");
        assert_eq!(body_section.format.as_deref(), Some("decimal"));
        assert_eq!(body_section.start, Some(1));
    }

    #[test]
    fn section_without_page_number_type_has_none() {
        let body = r#"
    <w:p><w:r><w:t>Plain</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let pg_num = document
            .default_section
            .as_ref()
            .and_then(|props| props.page_number_type.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.page_number_type.as_ref())
            });
        assert!(pg_num.is_none());
    }

    #[test]
    fn parses_document_background_color() {
        // `<w:background w:color="…"/>` is a `<w:document>` child (sibling of
        // `<w:body>`), so build the full document XML directly. §17.2.1.
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:background w:color="FFCC00"/>
  <w:body>
    <w:p><w:r><w:t>hi</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let document = DocumentParser::new()
            .parse_document_xml(xml)
            .expect("parse document");
        // Hex is normalized to uppercase `#RRGGBB`.
        assert_eq!(document.background_color.as_deref(), Some("#FFCC00"));

        // `auto` collapses to None (default page).
        let auto_xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:background w:color="auto"/>
  <w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body>
</w:document>"#;
        let auto_doc = DocumentParser::new()
            .parse_document_xml(auto_xml)
            .expect("parse document");
        assert_eq!(auto_doc.background_color, None);

        // Absent element → None.
        let plain_xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body>
</w:document>"#;
        let plain_doc = DocumentParser::new()
            .parse_document_xml(plain_xml)
            .expect("parse document");
        assert_eq!(plain_doc.background_color, None);
    }

    #[test]
    fn parses_section_vertical_alignment() {
        // `w:vAlign` (§17.6.22) vertically justifies the section's text on the
        // page. Word writes `center` for cover/title pages. The token is
        // normalized to lowercase and Word emits it self-closing.
        let body = r#"
    <w:p><w:r><w:t>Centered cover page</w:t></w:r></w:p>
    <w:sectPr>
      <w:vAlign w:val="Center"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let valign = document
            .default_section
            .as_ref()
            .and_then(|props| props.vertical_alignment.as_deref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.vertical_alignment.as_deref())
            });
        assert_eq!(valign, Some("center"));
    }

    #[test]
    fn section_vertical_alignment_top_and_absent_are_none() {
        // Both an absent `w:vAlign` and an explicit `top` (the default) collapse
        // to `None`, so the renderer treats them identically (no offset).
        let explicit_top = r#"
    <w:p><w:r><w:t>Top</w:t></w:r></w:p>
    <w:sectPr>
      <w:vAlign w:val="top"/>
    </w:sectPr>
"#;
        let absent = r#"
    <w:p><w:r><w:t>Default</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
    </w:sectPr>
"#;
        for body in [explicit_top, absent] {
            let document = parse_document_from_body(body);
            let valign = document
                .default_section
                .as_ref()
                .and_then(|props| props.vertical_alignment.as_deref())
                .or_else(|| {
                    document
                        .sections
                        .first()
                        .and_then(|range| range.properties.as_ref())
                        .and_then(|props| props.vertical_alignment.as_deref())
                });
            assert!(valign.is_none(), "expected None for body: {body}");
        }
    }

    #[test]
    fn parses_line_numbering() {
        // `w:lnNumType` (§17.6.8) draws margin line numbers. `w:countBy` controls
        // the interval, `w:start` the first number, `w:restart` the reset point,
        // and `w:distance` the gap to the text (twips).
        let body = r#"
    <w:p><w:r><w:t>Numbered lines</w:t></w:r></w:p>
    <w:sectPr>
      <w:lnNumType w:countBy="5" w:start="1" w:restart="continuous" w:distance="360"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let ln_num = document
            .default_section
            .as_ref()
            .and_then(|props| props.line_numbering.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.line_numbering.as_ref())
            })
            .expect("line numbering");
        assert_eq!(ln_num.count_by, Some(5));
        assert_eq!(ln_num.start, Some(1));
        assert_eq!(ln_num.restart.as_deref(), Some("continuous"));
        assert_eq!(ln_num.distance, Some(360));
    }

    #[test]
    fn section_without_line_numbering_has_none() {
        let body = r#"
    <w:p><w:r><w:t>Plain</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let ln_num = document
            .default_section
            .as_ref()
            .and_then(|props| props.line_numbering.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.line_numbering.as_ref())
            });
        assert!(ln_num.is_none());
    }

    #[test]
    fn parses_section_endnote_position_without_numbering() {
        use crate::document::EndnotePosition;
        for (value, expected) in [
            ("sectEnd", Some(EndnotePosition::SectionEnd)),
            ("docEnd", Some(EndnotePosition::DocumentEnd)),
            ("invalid", None),
        ] {
            let document = parse_document_from_body(&format!(
                r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:endnotePr><w:pos w:val="{value}"/></w:endnotePr></w:sectPr>"#
            ));
            assert_eq!(
                document.default_section.as_ref().unwrap().endnote_position,
                expected
            );
            assert_eq!(
                document
                    .default_section
                    .as_ref()
                    .unwrap()
                    .endnote_properties,
                None
            );
        }
    }

    #[test]
    fn section_endnote_position_inherits_settings_without_leaking_final_section() {
        use crate::document::EndnotePosition;
        let mut document = parse_document_from_body(
            r#"<w:p><w:pPr><w:sectPr/></w:pPr><w:r><w:t>First</w:t></w:r></w:p><w:p><w:r><w:t>Last</w:t></w:r></w:p><w:sectPr><w:endnotePr><w:pos w:val="docEnd"/></w:endnotePr></w:sectPr>"#,
        );
        document.endnote_position = Some(EndnotePosition::SectionEnd);
        assert_eq!(
            document.resolved_endnote_position(Some(0)),
            EndnotePosition::SectionEnd
        );
        assert_eq!(
            document.resolved_endnote_position(Some(1)),
            EndnotePosition::DocumentEnd
        );
        assert_eq!(
            document.resolved_endnote_position(None),
            EndnotePosition::DocumentEnd
        );
        document.endnote_position = None;
        assert_eq!(
            document.resolved_endnote_position(Some(0)),
            EndnotePosition::DocumentEnd
        );
    }

    #[test]
    fn parses_section_note_numbering() {
        // `w:footnotePr` / `w:endnotePr` (§17.11.12 / §17.11.4) in a sectPr carry
        // the numeral format, starting number, and restart policy for footnote /
        // endnote references in the section.
        let body = r#"
    <w:p><w:r><w:t>Body</w:t></w:r></w:p>
    <w:sectPr>
      <w:footnotePr>
        <w:numFmt w:val="lowerRoman"/>
        <w:numStart w:val="2"/>
        <w:numRestart w:val="eachPage"/>
      </w:footnotePr>
      <w:endnotePr>
        <w:numFmt w:val="upperLetter"/>
        <w:numStart w:val="3"/>
      </w:endnotePr>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let section = document
            .default_section
            .as_ref()
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
            })
            .expect("section");

        let footnote = section
            .footnote_properties
            .as_ref()
            .expect("footnote numbering");
        assert_eq!(footnote.number_format.as_deref(), Some("lowerroman"));
        assert_eq!(footnote.number_start, Some(2));
        assert_eq!(footnote.number_restart.as_deref(), Some("eachpage"));

        let endnote = section
            .endnote_properties
            .as_ref()
            .expect("endnote numbering");
        assert_eq!(endnote.number_format.as_deref(), Some("upperletter"));
        assert_eq!(endnote.number_start, Some(3));
        assert_eq!(endnote.number_restart, None);
    }

    #[test]
    fn section_without_note_numbering_has_none() {
        let body = r#"
    <w:p><w:r><w:t>Plain</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
    </w:sectPr>
"#;
        let document = parse_document_from_body(body);
        let section = document
            .default_section
            .as_ref()
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
            })
            .expect("section");
        assert!(section.footnote_properties.is_none());
        assert!(section.endnote_properties.is_none());
    }

    #[test]
    fn parses_section_text_direction() {
        let body = r#"
    <w:p><w:r><w:t>縦書き</w:t></w:r></w:p>
    <w:sectPr>
      <w:textDirection w:val="tbRl"/>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let direction = document
            .default_section
            .as_ref()
            .and_then(|props| props.text_direction)
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.text_direction)
            });

        assert_eq!(direction, Some(TextDirection::TbRl));
    }

    #[test]
    fn parses_doc_grid() {
        let body = r#"
    <w:p><w:r><w:t>Gridded section</w:t></w:r></w:p>
    <w:sectPr>
      <w:docGrid w:type="linesAndChars" w:linePitch="640" w:charSpace="8192"/>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let doc_grid = document
            .default_section
            .as_ref()
            .and_then(|props| props.doc_grid.as_ref())
            .or_else(|| {
                document
                    .sections
                    .first()
                    .and_then(|range| range.properties.as_ref())
                    .and_then(|props| props.doc_grid.as_ref())
            })
            .expect("doc grid");

        assert_eq!(doc_grid.grid_type.as_deref(), Some("linesAndChars"));
        const EPS: f32 = 0.001;
        assert!((doc_grid.line_pitch.unwrap() - 32.0).abs() < EPS);
        assert!((doc_grid.char_space.unwrap() - 2.0).abs() < EPS);

        if let Some(section) = document.sections.first() {
            let section_grid = section
                .properties
                .as_ref()
                .and_then(|props| props.doc_grid.as_ref())
                .expect("section doc grid");
            assert_eq!(section_grid.grid_type.as_deref(), Some("linesAndChars"));
        }
    }

    #[test]
    fn parses_multiple_sections_with_different_page_sizes() {
        let body = r#"
    <w:p>
      <w:r><w:t>First section</w:t></w:r>
    </w:p>
    <w:p>
      <w:pPr>
        <w:sectPr>
          <w:pgSz w:w="11906" w:h="16838"/>
        </w:sectPr>
      </w:pPr>
      <w:r><w:t>Section break</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>Second section</w:t></w:r>
    </w:p>
    <w:sectPr>
      <w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>
    </w:sectPr>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.as_ref().expect("body");
        assert_eq!(body.elements.len(), 3);

        assert_eq!(document.sections.len(), 2);
        let first = &document.sections[0];
        let second = &document.sections[1];

        assert_eq!((first.start, first.end), (0, 2));
        assert_eq!((second.start, second.end), (2, 3));

        let first_settings = first
            .properties
            .as_ref()
            .and_then(|props| props.page_settings.as_ref())
            .expect("first section settings");
        let second_settings = second
            .properties
            .as_ref()
            .and_then(|props| props.page_settings.as_ref())
            .expect("second section settings");

        assert!(first_settings.width.unwrap() < first_settings.height.unwrap());
        assert!(second_settings.width.unwrap() > second_settings.height.unwrap());
        assert_eq!(second_settings.orientation.as_deref(), Some("landscape"));

        let doc_settings = document.page_settings.expect("document page settings");
        assert!((doc_settings.width.unwrap() - first_settings.width.unwrap()).abs() < 0.1);
        assert!((doc_settings.height.unwrap() - first_settings.height.unwrap()).abs() < 0.1);
    }

    #[test]
    fn parses_simple_table() {
        let body = r#"
    <w:tbl>
      <w:tblGrid>
        <w:gridCol w:w="2400"/>
        <w:gridCol w:w="2400"/>
      </w:tblGrid>
      <w:tr>
        <w:tc>
          <w:p><w:r><w:t>Cell 1</w:t></w:r></w:p>
        </w:tc>
        <w:tc>
          <w:p><w:r><w:t>Cell 2</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
    <w:p><w:r><w:t>After table</w:t></w:r></w:p>
"#;

        let document = parse_document_from_body(body);
        let body = document.body.expect("body");
        assert_eq!(body.elements.len(), 2);

        let table = match &body.elements[0] {
            BodyElement::Table(table) => table.as_ref(),
            _ => panic!("expected table as first element"),
        };

        assert_eq!(table.columns.len(), 2);
        assert!((table.columns[0].width.unwrap() - 120.0).abs() < f32::EPSILON);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells.len(), 2);

        let cell_text = match &table.rows[0].cells[0].blocks[0] {
            BodyElement::Paragraph(p) => &p.as_ref().runs[0].text,
            _ => panic!("expected paragraph in cell"),
        };
        assert_eq!(cell_text, "Cell 1");

        let after_table = match &body.elements[1] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected trailing paragraph"),
        };
        assert_eq!(after_table.runs[0].text, "After table");
    }

    #[test]
    fn row_spacing_keeps_direct_and_table_exception_scopes_in_the_model() {
        for expanded in [false, true] {
            let spacing = |twips| {
                if expanded {
                    format!("<w:tblCellSpacing w:w=\"{twips}\" w:type=\"dxa\"></w:tblCellSpacing>")
                } else {
                    format!("<w:tblCellSpacing w:w=\"{twips}\" w:type=\"dxa\"/>")
                }
            };
            let body = format!(
                "<w:tbl><w:tblPr>{}</w:tblPr><w:tr><w:tblPrEx>{}</w:tblPrEx>\
                 <w:trPr>{}</w:trPr><w:tc><w:p/></w:tc></w:tr></w:tbl>",
                spacing(144),
                spacing(120),
                spacing(60),
            );
            let document = parse_document_from_body(&body);
            let BodyElement::Table(table) = &document.body.as_ref().unwrap().elements[0] else {
                panic!("table");
            };
            eprintln!("native_row_bytes={}", std::mem::size_of::<TableRow>());
            let row = serde_json::to_value(&table.rows[0]).unwrap();
            assert_eq!(row["cell_spacing"], serde_json::json!(3.0));
            assert_eq!(row["cell_spacing_exception"], serde_json::json!(6.0));
            let round_trip: TableRow = serde_json::from_value(row.clone()).unwrap();
            assert_eq!(serde_json::to_value(round_trip).unwrap(), row);
        }
    }

    #[test]
    fn parses_table_property_exceptions_and_direct_row_precedence() {
        let body = r#"
    <w:tbl>
      <w:tblPr>
        <w:shd w:val="clear" w:fill="DDEBF7"/>
        <w:tblBorders>
          <w:start w:val="single" w:sz="8" w:color="00B050"/>
          <w:end w:val="single" w:sz="8" w:color="7030A0"/>
          <w:insideV w:val="single" w:sz="4" w:color="4472C4"/>
        </w:tblBorders>
      </w:tblPr>
      <w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid>
      <w:tr>
        <w:tblPrEx>
          <w:tblCellSpacing w:w="120" w:type="dxa"/>
          <w:tblCellMar>
            <w:start w:w="360" w:type="dxa"/>
            <w:end w:w="40" w:type="dxa"/>
          </w:tblCellMar>
          <w:shd w:val="clear" w:fill="FFF2CC"/>
          <w:tblBorders>
            <w:insideV w:val="double" w:sz="16" w:color="C00000"/>
          </w:tblBorders>
        </w:tblPrEx>
        <w:tc>
          <w:tcPr>
            <w:tcBorders>
              <w:start w:val="double" w:sz="16" w:color="C00000"/>
              <w:end w:val="dashed" w:sz="8" w:color="ED7D31"/>
            </w:tcBorders>
          </w:tcPr>
          <w:p><w:r><w:t>Exception only</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
      <w:tr>
        <w:tblPrEx>
          <w:tblCellSpacing w:w="120" w:type="dxa"/>
        </w:tblPrEx>
        <w:trPr>
          <w:tblCellSpacing w:w="60" w:type="dxa"/>
        </w:trPr>
        <w:tc><w:p><w:r><w:t>Direct override</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
"#;

        let document = parse_document_from_body(body);
        let parsed_body = document.body.expect("body");
        let table = match &parsed_body.elements[0] {
            BodyElement::Table(table) => table.as_ref(),
            _ => panic!("expected table"),
        };

        // tblPrEx is serialized before trPr in a row. Preserve that cascade:
        // the exception supplies a style-derived default, then an explicit row
        // property wins when both are present.
        assert_eq!(table.rows[0].cell_spacing, None);
        assert_eq!(table.rows[0].cell_spacing_exception, Some(6.0));
        assert_eq!(table.rows[1].cell_spacing, Some(3.0));
        assert_eq!(table.rows[1].cell_spacing_exception, Some(6.0));
        let exception_margins = table.rows[0]
            .cell_margins
            .as_ref()
            .expect("tblPrEx cell margins");
        assert_eq!(exception_margins.start, Some(18.0));
        assert_eq!(exception_margins.end, Some(2.0));
        assert_eq!(
            table.cell_style.get("background-color").map(String::as_str),
            Some("#DDEBF7")
        );
        assert_eq!(
            table.rows[0]
                .style
                .get("background-color")
                .map(String::as_str),
            Some("#FFF2CC")
        );
        let table_inside_v = table
            .borders
            .as_ref()
            .and_then(|borders| borders.inside_v.as_ref())
            .expect("table inside vertical border");
        assert_eq!(table_inside_v.color.as_deref(), Some("#4472C4"));
        let table_borders = table.borders.as_ref().expect("table borders");
        assert_eq!(
            table_borders
                .start
                .as_ref()
                .and_then(|edge| edge.color.as_deref()),
            Some("#00B050")
        );
        assert_eq!(
            table_borders
                .end
                .as_ref()
                .and_then(|edge| edge.color.as_deref()),
            Some("#7030A0")
        );
        assert_eq!(
            table.rows[0].cells[0]
                .style
                .get("border-start-style")
                .map(String::as_str),
            Some("double")
        );
        assert_eq!(
            table.rows[0].cells[0]
                .style
                .get("border-end-color")
                .map(String::as_str),
            Some("#ED7D31")
        );
        let exception_inside_v = table.rows[0]
            .borders
            .as_ref()
            .and_then(|borders| borders.inside_v.as_ref())
            .expect("tblPrEx inside vertical border");
        assert_eq!(exception_inside_v.val.as_deref(), Some("double"));
        assert_eq!(exception_inside_v.color.as_deref(), Some("#C00000"));
    }

    #[test]
    fn parses_header_footer_reference_variants_per_section() {
        let body = r#"
            <w:p/>
            <w:sectPr>
              <w:headerReference r:id="rIdH1" w:type="default"/>
              <w:headerReference r:id="rIdH2" w:type="first"/>
              <w:headerReference r:id="rIdH3" w:type="even"/>
              <w:footerReference r:id="rIdF1" w:type="default"/>
              <w:footerReference r:id="rIdF2" w:type="first"/>
              <w:footerReference r:id="rIdF3" w:type="even"/>
              <w:titlePg/>
            </w:sectPr>
        "#;
        let doc = parse_document_from_body(body);
        // Document-level refs still reflect the default variant only (legacy
        // behavior for renderers that don't care about first/even).
        assert_eq!(doc.header_reference.as_deref(), Some("rIdH1"));
        assert_eq!(doc.footer_reference.as_deref(), Some("rIdF1"));

        let section = doc
            .default_section
            .as_ref()
            .expect("default section present");
        assert_eq!(section.header_reference.as_deref(), Some("rIdH1"));
        assert_eq!(section.header_reference_first.as_deref(), Some("rIdH2"));
        assert_eq!(section.header_reference_even.as_deref(), Some("rIdH3"));
        assert_eq!(section.footer_reference.as_deref(), Some("rIdF1"));
        assert_eq!(section.footer_reference_first.as_deref(), Some("rIdF2"));
        assert_eq!(section.footer_reference_even.as_deref(), Some("rIdF3"));
        assert!(section.title_page);
    }

    #[test]
    fn title_page_defaults_to_false_when_absent() {
        let body = r#"
            <w:p/>
            <w:sectPr>
              <w:headerReference r:id="rIdH1" w:type="default"/>
            </w:sectPr>
        "#;
        let doc = parse_document_from_body(body);
        let section = doc
            .default_section
            .as_ref()
            .expect("default section present");
        assert!(!section.title_page);
        assert!(section.header_reference_first.is_none());
        assert!(section.header_reference_even.is_none());
    }

    #[test]
    fn parses_numpages_field_instruction_from_fld_simple() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" NUMPAGES ">
                <w:r><w:t>42</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let run = paragraph.runs.first().expect("run");
        let field = run.field.as_ref().expect("field");
        assert_eq!(field.kind, RunFieldKind::NumPages);
    }

    #[test]
    fn parses_numwords_and_numchars_as_dedicated_statistics_fields() {
        // Word emits these built-in statistics as dedicated instructions, not
        // as `DOCPROPERTY Words/Characters`. Keeping semantic variants lets the
        // package resolver replace stale saved results with the corresponding
        // extended-property values after every OPC part has loaded.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" NUMWORDS \\* MERGEFORMAT ">
                <w:r><w:t>999</w:t></w:r>
              </w:fldSimple>
              <w:fldSimple w:instr=" NUMCHARS ">
                <w:r><w:t>888</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].kind, RunFieldKind::NumWords);
        assert_eq!(fields[0].cached_text.as_deref(), Some("999"));
        assert_eq!(fields[1].kind, RunFieldKind::NumChars);
        assert_eq!(fields[1].cached_text.as_deref(), Some("888"));
    }

    #[test]
    fn parses_edittime_as_a_dedicated_package_statistic_field() {
        // EDITTIME is not interchangeable with a generic DOCPROPERTY field:
        // Word defines its result as the package's total editing time in
        // minutes. Modeling that semantic distinction allows the package
        // loader to replace a stale saved result after docProps/app.xml has
        // been read, while still retaining the saved result as a fallback.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" EDITTIME \\* MERGEFORMAT ">
                <w:r><w:t>999</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .and_then(|run| run.field.as_ref())
            .expect("EDITTIME field");

        assert_eq!(field.kind, RunFieldKind::EditTime);
        assert_eq!(field.cached_text.as_deref(), Some("999"));
    }

    #[test]
    fn parses_filesize_units_from_simple_and_fragmented_instructions() {
        // FILESIZE is computed from the compressed DOCX package, not from an
        // app.xml scalar. Preserve its unit switch semantically so the loader
        // can replace stale results using the original byte length. The MB
        // case is deliberately split through the field name and switch to
        // guard Word's common multi-run instrText representation.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" FILESIZE ">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
              <w:fldSimple w:instr=" FILESIZE \k ">
                <w:r><w:t>2</w:t></w:r>
              </w:fldSimple>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> FILE</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">SIZE \</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">m </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>3</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("FILESIZE paragraph")
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 3);
        assert_eq!(
            fields[0].kind,
            RunFieldKind::FileSize {
                unit: crate::document::FileSizeUnit::Bytes,
            }
        );
        assert_eq!(
            fields[1].kind,
            RunFieldKind::FileSize {
                unit: crate::document::FileSizeUnit::Kilobytes,
            }
        );
        assert_eq!(
            fields[2].kind,
            RunFieldKind::FileSize {
                unit: crate::document::FileSizeUnit::Megabytes,
            }
        );
        assert_eq!(fields[2].cached_text.as_deref(), Some("3"));
    }

    #[test]
    fn parses_template_without_path_switch_as_a_package_property_field() {
        // Word stores the attached template's display name in app.xml. A
        // dedicated TEMPLATE instruction should therefore use the same live
        // package-property path as DOCPROPERTY Template, rather than leaving a
        // deliberately stale saved result on screen.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" TEMPLATE \* MERGEFORMAT ">
                <w:r><w:t>StaleTemplate.dotm</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .and_then(|run| run.field.as_ref())
            .expect("TEMPLATE field");

        assert_eq!(
            field.kind,
            RunFieldKind::DocumentProperty {
                name: "Template".to_string(),
                case_format: None,
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("StaleTemplate.dotm"));
    }

    #[test]
    fn template_with_path_switch_keeps_words_cached_full_path() {
        // The \p form asks Word for the template's full filesystem/URL path.
        // app.xml contains only its display name, so resolving it as the plain
        // Template property would replace a useful cached full path with less
        // information. Keep this form as a semantic cached field until
        // relationship/host template locations are available.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" TEMPLATE \p ">
                <w:r><w:t>C:\Templates\Corporate.dotm</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .expect("cached TEMPLATE path run");

        let field = run.field.as_ref().expect("cached TEMPLATE path field");
        assert_eq!(
            field.kind,
            RunFieldKind::Unsupported {
                instruction: "TEMPLATE \\p".to_string(),
            }
        );
        assert_eq!(
            field.cached_text.as_deref(),
            Some("C:\\Templates\\Corporate.dotm")
        );
        assert!(run.text.is_empty());
    }

    #[test]
    fn parses_read_only_info_as_a_package_property_field() {
        // INFO is Word's generic spelling for reading a property that is also
        // available through a dedicated field such as TITLE. Normalize the
        // read-only form onto the package-property path so a stale saved result
        // is replaced consistently in every document story.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IN</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">FO Tit</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">le \* MERGEFORMAT </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale Info Title</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("read-only INFO field");

        assert_eq!(
            field.kind,
            RunFieldKind::DocumentProperty {
                name: "Title".to_string(),
                case_format: None,
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Stale Info Title"));
    }

    #[test]
    fn info_with_new_value_does_not_mutate_package_metadata_during_load() {
        // The optional second operand is an assignment in Word, not display
        // formatting. Loading a DOCX must not silently rewrite its properties,
        // so preserve Word's result as a semantic cached field until an
        // explicit field-update command owns that mutation.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=' INFO Title "Replacement Title" '>
                <w:r><w:t>Replacement Title</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .expect("INFO assignment result run");

        let field = run.field.as_ref().expect("cached INFO assignment field");
        assert_eq!(
            field.kind,
            RunFieldKind::Unsupported {
                instruction: "INFO Title \"Replacement Title\"".to_string(),
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Replacement Title"));
        assert!(run.text.is_empty());
    }

    #[test]
    fn parses_sectionpages_instruction_split_across_instr_text_runs() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SECT</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">IONPAGES </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>3</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("fragmented SECTIONPAGES field");

        assert_eq!(field.kind, RunFieldKind::SectionPages);
        assert_eq!(field.cached_text.as_deref(), Some("3"));
    }

    #[test]
    fn parses_section_instruction_split_across_instr_text_runs() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEC</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">TION \* ARABIC </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>99</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("fragmented SECTION field");

        // The saved result can be stale after sections are inserted or
        // removed. Preserve a semantic kind so the renderer can substitute the
        // active 1-based section number instead of displaying this cache.
        assert_eq!(field.kind, RunFieldKind::SectionNumber);
        assert_eq!(field.cached_text.as_deref(), Some("99"));
    }

    #[test]
    fn parses_and_resolves_if_and_compare_field_instructions() {
        // Word may split a field instruction at any run boundary. Keep the
        // split inside IF itself and omit spaces around `>=`, as Word's own IF
        // examples do, so the parser cannot rely on whitespace tokenization.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> I</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">F 100>=99 "Approved value" "Denied value" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale IF result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" COMPARE &quot;Tokyo Branch&quot; = &quot;Tokyo*&quot; ">
                <w:r><w:t>0</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let body = doc.body.as_ref().expect("body");

        let if_field = body.elements[0]
            .as_paragraph()
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("IF field");
        // A compact expression with a number on the left is Word's syntax
        // error, not a comparison.
        assert_eq!(
            if_field.kind,
            RunFieldKind::Quote {
                text: crate::document::WORD_IF_SYNTAX_ERROR.to_string(),
                case_format: None,
            }
        );
        assert_eq!(if_field.cached_text.as_deref(), Some("Stale IF result"));
        assert_eq!(
            if_field.resolved_value.as_deref(),
            Some(crate::document::WORD_IF_SYNTAX_ERROR)
        );

        let compare_field = body.elements[1]
            .as_paragraph()
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("COMPARE field");
        assert_eq!(
            compare_field.kind,
            RunFieldKind::Compare {
                left: "Tokyo Branch".to_string(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: "Tokyo*".to_string(),
                right_is_quoted: true,
            }
        );
        assert_eq!(compare_field.resolved_value.as_deref(), Some("1"));
    }

    #[test]
    fn quoted_right_if_operand_forces_character_comparison() {
        // Word specifies that a quoted Expression2 is compared as a character
        // string. Numeric coercion would incorrectly make `2` and `02` equal,
        // so the parser must retain the right operand's quote provenance after
        // stripping the delimiters from its visible value.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" IF 2 = &quot;02&quot; &quot;Numeric equal&quot; &quot;String differs&quot; ">
                <w:r><w:t>Stale numeric result</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("quoted numeric IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("String differs"));
    }

    #[test]
    fn locked_complex_and_simple_if_results_are_not_recalculated() {
        let body = r#"<w:p>
          <w:r><w:fldChar w:fldCharType="begin" w:fldLock="true"/></w:r>
          <w:r><w:instrText> IF 1 = 1 "live" "other" </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>saved complex</w:t></w:r>
          <w:r><w:fldChar w:fldCharType="end"/></w:r>
          <w:fldSimple w:fldLock="1" w:instr=" IF 1 = 1 &quot;live&quot; &quot;other&quot; ">
            <w:r><w:t>saved simple</w:t></w:r>
          </w:fldSimple></w:p>"#;
        let doc = parse_document_from_body(body);
        let fields = doc.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap()
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(fields.len(), 2);
        for field in fields {
            assert!(field.kind.is_locked());
            assert!(matches!(field.kind.base_kind(), RunFieldKind::If { .. }));
            assert_ne!(field.resolved_value.as_deref(), Some("live"));
            assert!(field
                .cached_text
                .as_deref()
                .is_some_and(|text| text.starts_with("saved")));
        }
    }

    #[test]
    fn field_lock_false_and_omitted_keep_live_values_and_attribute_order_is_irrelevant() {
        for attributes in [
            "",
            "w:fldLock=\"false\"",
            "w:fldLock=\"0\"",
            "w:fldLock=\"off\"",
        ] {
            let doc = parse_document_from_body(&format!(
                r#"<w:p><w:fldSimple w:instr=" IF 1 = 1 &quot;live&quot; &quot;other&quot; " {attributes}><w:r><w:t>saved</w:t></w:r></w:fldSimple></w:p>"#
            ));
            let field = doc.body.as_ref().unwrap().elements[0]
                .as_paragraph()
                .unwrap()
                .runs[0]
                .field
                .as_ref()
                .unwrap();
            assert!(!field.kind.is_locked());
            assert_eq!(field.resolved_value.as_deref(), Some("live"));
        }
        let doc = parse_document_from_body(
            r#"<w:p><w:fldSimple w:instr=" IF 1 = 1 &quot;live&quot; &quot;other&quot; " w:fldLock="on"><w:r><w:t>saved</w:t></w:r></w:fldSimple></w:p>"#,
        );
        let field = doc.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap()
            .runs[0]
            .field
            .as_ref()
            .unwrap();
        assert!(field.kind.is_locked());
        assert_eq!(field.resolved_value, None);
    }

    #[test]
    fn self_closing_locked_field_remains_a_semantic_empty_result() {
        let doc = parse_document_from_body(
            r#"<w:p><w:r><w:t>A</w:t></w:r><w:fldSimple w:fldLock="true" w:instr=" NUMPAGES "/><w:r><w:t>B</w:t></w:r></w:p>"#,
        );
        let paragraph = doc.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap();
        let field = paragraph
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .unwrap();
        assert!(field.kind.is_locked());
        assert_eq!(field.cached_text, None);
        assert_eq!(field.resolved_value, None);
        assert_eq!(
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "AB"
        );
    }

    #[test]
    fn empty_toc_entry_does_not_close_the_enclosing_simple_field() {
        let doc = parse_document_from_body(
            r#"<w:p><w:fldSimple w:instr=" IF 1 = 1 &quot;live&quot; &quot;other&quot; "><w:fldSimple w:instr=" TC &quot;entry&quot; "/><w:r><w:t>saved</w:t></w:r></w:fldSimple></w:p>"#,
        );
        let paragraph = doc.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap();
        let fields = paragraph
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].resolved_value.as_deref(), Some("live"));
        assert_eq!(fields[0].cached_text.as_deref(), Some("saved"));
        assert_eq!(doc.toc_entry_fields.len(), 1);
    }

    #[test]
    fn nested_locked_compare_uses_its_saved_operand_in_the_outer_if() {
        let doc = parse_document_from_body(
            r#"<w:p>
          <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> IF </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="begin" w:fldLock="true"/></w:r><w:r><w:instrText> COMPARE 2 &lt; 10 </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>0</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
          <w:r><w:instrText> = 1 "live true" "saved false" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>
          <w:r><w:t>old outer</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
        let field = doc.body.as_ref().unwrap().elements[0]
            .as_paragraph()
            .unwrap()
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .unwrap();
        assert_eq!(field.resolved_value.as_deref(), Some("saved false"));
    }

    #[test]
    fn quoted_right_if_operand_is_not_resolved_as_a_bookmark() {
        // Expression2 is a character string when it is quoted. Even if its
        // text happens to equal a bookmark name, Word does not dereference it.
        // Using different bookmark text makes the erroneous lookup observable:
        // resolving both `Target` operands through the bookmark would select
        // the true branch instead of comparing `Tokyo` with the literal name.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="Target"/>
              <w:r><w:t>Tokyo</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" IF Target = &quot;Target&quot; &quot;Both dereferenced&quot; &quot;Literal differs&quot; ">
                <w:r><w:t>Stale bookmark result</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("quoted bookmark-name IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Literal differs"));
    }

    #[test]
    fn quoted_left_if_operand_is_not_resolved_as_a_bookmark() {
        // A quoted Expression1 is a string literal. Its contents may happen to
        // equal a bookmark name, but Word must compare the literal instead of
        // replacing it with the bookmark text.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="Target"/>
              <w:r><w:t>Tokyo</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" IF &quot;Target&quot; = &quot;Target&quot; &quot;Literal match&quot; &quot;Bookmark leaked&quot; ">
                <w:r><w:t>Stale quoted-left result</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("quoted left IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Literal match"));
    }

    #[test]
    fn wildcard_asterisk_does_not_match_more_than_128_remaining_characters() {
        // Word limits the part of Expression1 consumed by `*`, together with
        // the remainder of Expression2, to 128 characters. A bare wildcard
        // must therefore reject a 129-character value instead of matching an
        // arbitrarily long string.
        let long_value = "A".repeat(129);
        let body = format!(
            r#"
            <w:p>
              <w:fldSimple w:instr=" IF &quot;{long_value}&quot; = &quot;*&quot; &quot;Unlimited wildcard&quot; &quot;Wildcard capped&quot; ">
                <w:r><w:t>Unlimited wildcard</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#
        );
        let doc = parse_document_from_body(&body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("long wildcard IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Wildcard capped"));
    }

    #[test]
    fn wildcard_asterisk_matches_exactly_128_remaining_characters() {
        // The documented limit is inclusive. Keep the boundary value matching
        // so the cap does not turn a valid 128-character Word comparison into
        // an off-by-one false result.
        let boundary_value = "A".repeat(128);
        let body = format!(
            r#"
            <w:p>
              <w:fldSimple w:instr=" IF &quot;{boundary_value}&quot; = &quot;*&quot; &quot;Boundary matched&quot; &quot;Boundary rejected&quot; ">
                <w:r><w:t>Boundary rejected</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#
        );
        let doc = parse_document_from_body(&body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("boundary wildcard IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Boundary matched"));
    }

    #[test]
    fn resolves_compare_nested_in_if_instruction_without_leaking_inner_result() {
        // Word nests a complete field between the outer field's `begin` and
        // `separate` markers. The nested field's result is an operand of IF,
        // not visible paragraph content. Keeping this as real complex-field
        // markup (rather than flattening the operand to "1" in the fixture)
        // protects both the instruction-stack behavior and result suppression.
        // The inner cache is deliberately stale (`0` for the true comparison):
        // the outer IF must consume the nested field's live numeric result.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE 2 &lt; 10 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 1 "Nested true" "Nested false" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let fields = paragraph
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 1, "nested operand must not become visible");
        assert_eq!(
            fields[0].kind,
            RunFieldKind::If {
                left: "1".to_string(),
                left_is_quoted: false,
                operator: FieldComparisonOperator::Equal,
                right: "1".to_string(),
                right_is_quoted: false,
                true_text: "Nested true".to_string(),
                false_text: Some("Nested false".to_string()),
            }
        );
        assert_eq!(fields[0].cached_text.as_deref(), Some("Stale outer result"));
        assert_eq!(fields[0].resolved_value.as_deref(), Some("Nested true"));
    }

    #[test]
    fn nested_mergefield_cache_becomes_if_operand_without_leaking_result() {
        // Mail-merge data is external to the DOCX, but Word saves the current
        // MERGEFIELD result in the nested field. That saved value is the best
        // available operand for an enclosing IF and must not also appear as
        // independent paragraph text before the outer result.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> MERGEFIELD Status </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>North Region</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = "North Region" "Merge approved" "Merge rejected" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Merge rejected</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let fields = paragraph
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 1, "nested MERGEFIELD must stay structural");
        assert_eq!(fields[0].resolved_value.as_deref(), Some("Merge approved"));
        assert!(
            paragraph.runs.iter().all(|run| run.text != "North Region"),
            "nested result is an IF operand, not separate visible text"
        );
    }

    #[test]
    fn resolves_quoted_numeric_compare_nested_in_if_despite_stale_cache() {
        // A quoted Expression2 forces character comparison, so `2` and `02`
        // differ. The inner cache deliberately says they are equal. Because a
        // nested field becomes instruction text for its parent and is not kept
        // as a separately resolvable run, Jade must refresh this self-contained
        // comparison before closing the inner field.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE 2 = "02" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>1</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 0 "Quoted nested differs" "Stale quoted nested" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Quoted nested differs")
        );
    }

    #[test]
    fn resolves_quoted_string_compare_nested_in_if_despite_stale_cache() {
        // Both inner operands are explicit string literals, so the comparison
        // is self-contained and safe to refresh before it becomes the outer
        // IF operand. The deliberately stale inner zero must not survive.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE "North Region" = "North Region" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 1 "Nested literal match" "Stale nested literal" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer literal result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer quoted-string IF field");

        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Nested literal match")
        );
    }

    #[test]
    fn resolves_bookmark_compare_nested_in_if_despite_stale_inner_cache() {
        // The bookmark paragraph is complete before the field paragraph, so
        // its text is available when the nested COMPARE closes. The cached `0`
        // deliberately contradicts `120 >= 100`; Word updates the nested field
        // first and the outer IF must therefore choose the true branch.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="OrderTotal"/>
              <w:r><w:t>120</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE OrderTotal &gt;= 100 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 1 "Approved" "Denied" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Denied stale outer</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: "1".to_string(),
                left_is_quoted: false,
                operator: FieldComparisonOperator::Equal,
                right: "1".to_string(),
                right_is_quoted: false,
                true_text: "Approved".to_string(),
                false_text: Some("Denied".to_string()),
            }
        );
        assert_eq!(field.resolved_value.as_deref(), Some("Approved"));
    }

    #[test]
    fn resolves_forward_bookmark_compare_nested_in_if_despite_stale_cache() {
        // The bookmark closes after the field paragraph, so it is unavailable
        // when the nested COMPARE ends. Preserve the structured inner field
        // until the document-wide bookmark pass instead of baking its stale
        // cached zero into the outer IF instruction.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE FutureTotal &gt;= 100 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 1 "Forward approved" "Stale forward cache" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer forward result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:bookmarkStart w:id="1" w:name="FutureTotal"/>
              <w:r><w:t>120</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer forward-bookmark IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Forward approved"));
    }

    #[test]
    fn resolves_ref_nested_in_if_despite_stale_inner_cache() {
        // A nested REF is an operand, not a separately rendered field. Word
        // refreshes it from the bookmark before evaluating the outer IF, so
        // the deliberately stale cached `0` must not select the false branch.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="OrderTotal"/>
              <w:r><w:t>120</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> REF OrderTotal </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> &gt;= 100 "REF approved" "REF denied" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>REF stale outer</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .expect("field paragraph");
        let fields = paragraph
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 1, "nested REF must not become visible");
        assert_eq!(fields[0].resolved_value.as_deref(), Some("REF approved"));
    }

    #[test]
    fn applies_ref_capitalization_before_using_nested_if_operand() {
        // Nested REF evaluation has a separate early-resolution path from a
        // visible REF run. Its general format switch must therefore be applied
        // before the result is quoted and inserted into the outer instruction.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="ReleaseChannel"/>
              <w:r><w:t>production ready</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> REF ReleaseChannel \* Caps </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale nested value</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = "Production Ready" "approved" "denied" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale outer value</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("approved"));
    }

    #[test]
    fn preserves_spaced_ref_result_as_single_right_if_operand() {
        // A nested field result may contain spaces. When it occupies the right
        // side of IF, inserting it as raw instruction text shifts its trailing
        // words into TrueText/FalseText. Keep the live REF value as one token
        // so the outer field retains both its comparison and branch strings.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="RegionName"/>
              <w:r><w:t>North Region</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF "North Region" = </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> REF RegionName </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Wrong cache</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> "Matched full region" "Rejected full region" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: "North Region".to_string(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: "North Region".to_string(),
                right_is_quoted: true,
                true_text: "Matched full region".to_string(),
                false_text: Some("Rejected full region".to_string()),
            }
        );
        assert_eq!(field.resolved_value.as_deref(), Some("Matched full region"));
    }

    #[test]
    fn preserves_quote_bearing_ref_result_as_single_if_operand() {
        // A nested field result is one expression even when its visible text
        // contains both whitespace and quotation marks. The synthetic parent
        // instruction must escape those quotes; otherwise tokenization splits
        // the live REF value and shifts the IF branch strings out of position.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="QuotedRegion"/>
              <w:r><w:t>The "quoted" region</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> REF QuotedRegion </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Wrong quoted cache</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = "The \"quoted\" region" "Quoted value matched" "Quoted value rejected" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale quoted outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: "\u{e7a0}The \"quoted\" region".to_string(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: "The \"quoted\" region".to_string(),
                right_is_quoted: true,
                true_text: "Quoted value matched".to_string(),
                false_text: Some("Quoted value rejected".to_string()),
            }
        );
        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Quoted value rejected")
        );
    }

    #[test]
    fn resolves_if_with_empty_quoted_operands() {
        // Word uses a quoted empty string as a real comparison operand. It is
        // not omitted syntax: both empty tokens must survive tokenization so
        // the following strings remain TrueText and FalseText.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF "" = "" "Empty matched" "Empty rejected" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale empty result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: String::new(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: String::new(),
                right_is_quoted: true,
                true_text: "Empty matched".to_string(),
                false_text: Some("Empty rejected".to_string()),
            }
        );
        assert_eq!(field.resolved_value.as_deref(), Some("Empty matched"));
    }

    #[test]
    fn resolves_if_with_compact_empty_quoted_operands() {
        // Word producers need not place whitespace around the comparison
        // operator. Quoted operands still define token boundaries in `""=""`;
        // collapsing the adjacent pieces into just `=` loses both expressions.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF ""="" "Compact empty matched" "Compact empty rejected" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale compact result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("compact IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: String::new(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: String::new(),
                right_is_quoted: true,
                true_text: "Compact empty matched".to_string(),
                false_text: Some("Compact empty rejected".to_string()),
            }
        );
        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Compact empty matched")
        );
    }

    #[test]
    fn preserves_empty_nested_result_as_right_if_operand() {
        // The inner IF deliberately returns an empty FalseText. Word still
        // supplies that empty result as the outer IF's right operand; omitting
        // it would shift the outer branch strings into the comparison.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF "" = </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF 1 = 2 "Unexpected inner" "" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Wrong inner cache</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> "Nested empty matched" "Nested empty rejected" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: String::new(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: String::new(),
                right_is_quoted: true,
                true_text: "Nested empty matched".to_string(),
                false_text: Some("Nested empty rejected".to_string()),
            }
        );
        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Nested empty matched")
        );
    }

    #[test]
    fn resolves_same_paragraph_bookmark_nested_in_if_after_bookmark_end() {
        // Word permits a bookmark and its consuming field in one paragraph.
        // Once bookmarkEnd has been read, its preceding text is complete even
        // though the paragraph itself has not closed. The deliberately stale
        // nested cache must not force the outer IF onto the false branch.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="InlineTotal"/>
              <w:r><w:t>120</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
              <w:r><w:t xml:space="preserve"> - </w:t></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> COMPARE InlineTotal &gt;= 100 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>0</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = 1 "Inline approved" "Inline denied" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Inline stale outer</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(field.resolved_value.as_deref(), Some("Inline approved"));
    }

    #[test]
    fn restores_general_references_in_visible_text() {
        // quick-xml reports entity references separately from surrounding
        // BytesText. The field fix must also preserve ordinary Word text; an
        // ampersand or comparison sign must never disappear from a paragraph.
        let doc =
            parse_document_from_body(r#"<w:p><w:r><w:t>A &amp; B &lt; C &gt; D</w:t></w:r></w:p>"#);
        let text = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .map(|run| run.text.as_str());
        assert_eq!(text, Some("A & B < C > D"));
    }

    #[test]
    fn note_rule_runs_preserve_kind_and_formatting_without_text() {
        for note in ["footnote", "endnote"] {
            for (tag, expected) in [
                ("separator", NoteSeparatorKind::Separator),
                (
                    "continuationSeparator",
                    NoteSeparatorKind::ContinuationSeparator,
                ),
            ] {
                for empty_tag in [true, false] {
                    let rule = if empty_tag {
                        format!("<w:{tag}/>")
                    } else {
                        format!("<w:{tag}></w:{tag}>")
                    };
                    let xml = format!(
                        r#"<w:{note}s xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:{note} w:id="-1" w:type="{tag}"><w:p><w:r><w:rPr><w:sz w:val="42"/></w:rPr>{rule}</w:r></w:p></w:{note}></w:{note}s>"#
                    );
                    let notes = DocumentParser::new()
                        .parse_notes_xml(&xml, format!("w:{note}").as_bytes())
                        .unwrap();
                    let paragraph = notes[0].body[0].as_paragraph().unwrap();
                    let run = &paragraph.runs[0];
                    assert_eq!(run.note_separator, Some(expected));
                    assert!(run.text.is_empty());
                    assert_eq!(run.properties.as_ref().unwrap().font_size, Some(21.0));
                    assert!(!run.has_footnote_ref && !run.has_endnote_ref);
                }
            }
        }
    }

    #[test]
    fn note_text_preserves_significant_whitespace_like_body_text() {
        for (collection, note, reference) in [
            ("footnotes", "footnote", "footnoteRef"),
            ("endnotes", "endnote", "endnoteRef"),
        ] {
            for (content, expected) in [
                (" Body ", " Body "),
                ("  ", "  "),
                (" A &amp; B &lt; C &gt; D ", " A & B < C > D "),
            ] {
                for preserve in [false, true] {
                    let space = if preserve {
                        " xml:space=\"preserve\""
                    } else {
                        ""
                    };
                    let xml = format!(
                        "<w:{collection} xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
                         <w:{note} w:id=\"2\"><w:p>\n\
                         <w:r><w:{reference}/></w:r>\n\
                         <w:r><w:t{space}>{content}</w:t></w:r>\n\
                         </w:p></w:{note}></w:{collection}>"
                    );
                    let notes = DocumentParser::new()
                        .parse_notes_xml(&xml, format!("w:{note}").as_bytes())
                        .expect("parse note");
                    let text: String = notes[0].body[0]
                        .as_paragraph()
                        .expect("paragraph")
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect();
                    assert_eq!(
                        text,
                        if preserve { expected } else { expected.trim() },
                        "{note}, preserve={preserve}"
                    );
                }
            }
        }
    }

    #[test]
    fn restores_general_references_in_indented_note_text() {
        for (collection, note, reference) in [
            ("footnotes", "footnote", "footnoteRef"),
            ("endnotes", "endnote", "endnoteRef"),
        ] {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<w:{collection} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:{note} w:id="2">
    <w:p>
      <w:r><w:{reference}/></w:r>
      <w:r><w:tab/><w:t>A &amp; B &lt; C &gt; D</w:t></w:r>
    </w:p>
  </w:{note}>
</w:{collection}>"#
            );
            let note_tag = format!("w:{note}");
            let notes = DocumentParser::new()
                .parse_notes_xml(&xml, note_tag.as_bytes())
                .expect("parse notes");

            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].body.len(), 1, "indentation must not create blocks");
            let text = notes[0]
                .body
                .iter()
                .filter_map(BodyElement::as_paragraph)
                .flat_map(|paragraph| paragraph.runs.iter())
                .map(|run| run.text.as_str())
                .collect::<String>();
            assert_eq!(text, "\tA & B < C > D");
        }
    }

    #[test]
    fn if_field_resolves_bookmark_operand_and_omitted_false_text() {
        // Expressions may be bookmark names. Resolve the bookmark text before
        // comparing it numerically; when FalseText is omitted Word returns an
        // empty field result rather than preserving a stale cached value.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="OrderTotal"/>
              <w:r><w:t>120</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr="IF OrderTotal &lt; 100 &quot;Below minimum&quot;">
                <w:r><w:t>Stale result</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("bookmark IF field");

        assert_eq!(field.resolved_value.as_deref(), Some(""));
    }

    #[test]
    fn parses_quoted_docproperty_instruction_split_across_instr_text_runs() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> DOCPROP</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">ERTY "Document Title" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale cached title</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("fragmented DOCPROPERTY field");

        assert_eq!(
            field.kind,
            RunFieldKind::DocumentProperty {
                name: "Document Title".to_string(),
                case_format: None,
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Stale cached title"));
    }

    #[test]
    fn parses_quoted_docvariable_instruction_split_across_instr_text_runs() {
        // Word is free to split `w:instrText` at arbitrary run boundaries.
        // Keep the split inside the field name and retain a quoted variable
        // name so this guards both instruction reconstruction and tokenizing.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> DOCVAR</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">IABLE "Release Channel" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale cached channel</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("fragmented DOCVARIABLE field");

        assert_eq!(
            field.kind,
            RunFieldKind::DocumentVariable {
                name: "Release Channel".to_string(),
                case_format: None,
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Stale cached channel"));
    }

    #[test]
    fn parses_builtin_title_field_through_document_property_path() {
        let body =
            r#"<w:p><w:fldSimple w:instr=" TITLE "><w:r><w:t>Old</w:t></w:r></w:fldSimple></w:p>"#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("TITLE field");
        assert_eq!(
            field.kind,
            RunFieldKind::DocumentProperty {
                name: "TITLE".to_string(),
                case_format: None,
            }
        );
    }

    #[test]
    fn parses_page_field_instruction_from_fld_simple() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr="PAGE \* MERGEFORMAT">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.first())
            .and_then(|r| r.field.as_ref())
            .expect("page field");
        // Switch tokens like `\* MERGEFORMAT` must not confuse the
        // identifier scan; the first unmarked token is still `PAGE`.
        assert_eq!(field.kind, RunFieldKind::PageNumber);
    }

    #[test]
    fn parses_hyperlink_field_instruction_and_preserves_editable_result_text() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> HYPERLINK "https://example.com/a b" </w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">\l "details" \o "Open details" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Example link</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let run = paragraph
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("hyperlink field result");

        assert_eq!(
            run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::Hyperlink {
                target: Some("https://example.com/a b".to_string()),
                anchor: Some("details".to_string()),
                tooltip: Some("Open details".to_string()),
                target_frame: None,
            })
        );
        assert_eq!(run.text, "Example link");
        assert_eq!(
            run.field
                .as_ref()
                .and_then(|field| field.cached_text.as_deref()),
            None,
            "HYPERLINK result text remains ordinary editable run text"
        );
        let hyperlink = run.hyperlink.as_ref().expect("interactive hyperlink");
        assert_eq!(hyperlink.target.as_deref(), Some("https://example.com/a b"));
        assert_eq!(hyperlink.anchor.as_deref(), Some("details"));
        assert_eq!(hyperlink.tooltip.as_deref(), Some("Open details"));
        assert_eq!(hyperlink.target_mode.as_deref(), Some("External"));
    }

    #[test]
    fn preserves_hyperlink_target_frame_from_field_and_wrapper() {
        // Word can express the browsing context in either the HYPERLINK
        // instruction (`\t TargetFrame`) or `w:hyperlink/@w:tgtFrame`.
        // Keep both forms on the shared RunHyperlink contract so the canvas
        // interaction layer can activate them without guessing `_blank`.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> HYPERLINK "https://example.com/report" \t "ReportFrame" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Open report</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:hyperlink w:anchor="summary" w:tgtFrame="_self">
                <w:r><w:t>Jump to summary</w:t></w:r>
              </w:hyperlink>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();

        let field_run = paragraphs[0]
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("HYPERLINK result run");
        assert_eq!(
            field_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::Hyperlink {
                target: Some("https://example.com/report".to_string()),
                anchor: None,
                tooltip: None,
                target_frame: Some("ReportFrame".to_string()),
            })
        );
        assert_eq!(
            field_run
                .hyperlink
                .as_ref()
                .and_then(|hyperlink| hyperlink.target_frame.as_deref()),
            Some("ReportFrame")
        );

        let wrapper_hyperlink = paragraphs[1].runs[0]
            .hyperlink
            .as_ref()
            .expect("w:hyperlink metadata");
        assert_eq!(wrapper_hyperlink.anchor.as_deref(), Some("summary"));
        assert_eq!(wrapper_hyperlink.target_frame.as_deref(), Some("_self"));
    }

    #[test]
    fn macrobutton_uses_instruction_display_text_instead_of_stale_saved_result() {
        // MACROBUTTON's first operand is the macro name and every following
        // word is the one-line button label. Jade must never execute the macro
        // while loading a document, but the display text is self-contained and
        // safe to refresh. Cover both quoted fldSimple text and an instruction
        // fragmented across complex-field runs, as emitted by edited Word docs.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" MACROBUTTON PrintEnvelope &quot;Print envelope&quot; ">
                <w:r><w:t>stale simple label</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> MACRO</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">BUTTON RunReport Run report now \* MERGEFORMAT </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale complex label</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 2);
        assert_eq!(
            fields[0].kind,
            RunFieldKind::MacroButton {
                macro_name: "PrintEnvelope".to_string(),
                display_text: "Print envelope".to_string(),
            }
        );
        assert_eq!(fields[0].resolved_value.as_deref(), Some("Print envelope"));
        assert_eq!(fields[0].cached_text.as_deref(), Some("stale simple label"));
        assert_eq!(
            fields[1].kind,
            RunFieldKind::MacroButton {
                macro_name: "RunReport".to_string(),
                display_text: "Run report now".to_string(),
            }
        );
        assert_eq!(fields[1].resolved_value.as_deref(), Some("Run report now"));
        assert_eq!(
            fields[1].cached_text.as_deref(),
            Some("stale complex label")
        );
    }

    #[test]
    fn gotobutton_uses_display_text_and_links_only_bookmark_destinations() {
        // GOTOBUTTON's result is the instruction DisplayText. A plain bookmark
        // destination can use Jade's existing internal hyperlink contract, but
        // Word's special section/line/footnote/page destination syntaxes must
        // not be mislabeled as bookmarks until those navigation modes exist.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="summary"/>
              <w:r><w:t>Summary</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" GOTOBUTTON summary &quot;View summary&quot; ">
                <w:r><w:t>stale simple label</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> GOTO</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">BUTTON summary Jump to summary \* MERGEFORMAT </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale complex label</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" GOTOBUTTON s3 Section three ">
                <w:r><w:t>stale section label</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" GOTOBUTTON summary ">
                <w:r><w:t>saved malformed label</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field_runs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .skip(1)
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find(|run| run.field.is_some()))
            .collect::<Vec<_>>();

        assert_eq!(field_runs.len(), 4);
        for (run, label) in field_runs[..2]
            .iter()
            .zip(["View summary", "Jump to summary"])
        {
            assert_eq!(
                run.field.as_ref().map(|field| &field.kind),
                Some(&RunFieldKind::GoToButton {
                    destination: "summary".to_string(),
                    display_text: label.to_string(),
                })
            );
            assert_eq!(
                run.field
                    .as_ref()
                    .and_then(|field| field.resolved_value.as_deref()),
                Some(label)
            );
            let hyperlink = run
                .hyperlink
                .as_ref()
                .expect("bookmark GOTOBUTTON hyperlink");
            assert_eq!(hyperlink.anchor.as_deref(), Some("summary"));
            assert_eq!(hyperlink.target, None);
            assert_eq!(hyperlink.target_mode, None);
        }

        let section_run = field_runs[2];
        assert_eq!(
            section_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::GoToButton {
                destination: "s3".to_string(),
                display_text: "Section three".to_string(),
            })
        );
        assert_eq!(
            section_run
                .field
                .as_ref()
                .and_then(|field| field.resolved_value.as_deref()),
            Some("Section three")
        );
        assert_eq!(section_run.hyperlink, None);

        // DisplayText is required. Keep malformed input on the conservative
        // unsupported/cached-result path instead of inventing an empty button.
        let malformed_run = field_runs[3];
        assert_eq!(
            malformed_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::Unsupported {
                instruction: "GOTOBUTTON summary".to_string(),
            })
        );
        assert_eq!(
            malformed_run
                .field
                .as_ref()
                .and_then(|field| field.cached_text.as_deref()),
            Some("saved malformed label")
        );
        assert_eq!(malformed_run.hyperlink, None);
    }

    #[test]
    fn parses_anchor_only_hyperlink_from_fld_simple() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" HYPERLINK \l &quot;local target&quot; ">
                <w:r><w:t>Jump locally</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .expect("simple hyperlink field result");

        assert_eq!(
            run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::Hyperlink {
                target: None,
                anchor: Some("local target".to_string()),
                tooltip: None,
                target_frame: None,
            })
        );
        let hyperlink = run.hyperlink.as_ref().expect("internal hyperlink");
        assert_eq!(hyperlink.anchor.as_deref(), Some("local target"));
        assert!(hyperlink.target.is_none());
        assert!(hyperlink.target_mode.is_none());
    }

    #[test]
    fn parses_legacy_page_number_block_as_page_field() {
        // `w:pgNum` is the legacy page-number block from ISO/IEC 29500
        // §17.3.3.22. It can share a run with literal prefix text; dropping the
        // empty element silently removes the page number from old headers and
        // footers. Model it with the existing live PAGE-field contract so the
        // renderer still applies section page-number settings.
        let body = r#"
            <w:p>
              <w:r>
                <w:t xml:space="preserve">This is the current page: </w:t>
                <w:pgNum/>
              </w:r>
            </w:p>
        "#;

        let document = parse_document_from_body(body);
        let paragraph = document
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let run = paragraph.runs.first().expect("run");
        assert_eq!(run.text, "This is the current page: ");
        assert_eq!(
            run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::PageNumber),
            "w:pgNum must reuse the live PAGE field renderer",
        );
    }

    #[test]
    fn expanded_fld_simple_does_not_capture_following_text() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr="PAGE">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t>after field</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");

        assert_eq!(paragraph.runs.len(), 2);
        assert_eq!(
            paragraph.runs[0].field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::PageNumber)
        );
        assert_eq!(paragraph.runs[1].text, "after field");
        assert!(paragraph.runs[1].field.is_none());
    }

    #[test]
    fn preserves_cdata_inside_text_nodes() {
        let doc = parse_document_from_body(
            r#"<w:p><w:r><w:t><![CDATA[hello <world>]]></w:t></w:r></w:p>"#,
        );
        let text = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .map(|run| run.text.as_str());
        assert_eq!(text, Some("hello <world>"));
    }

    #[test]
    fn rejects_excessive_text_box_nesting_before_recursing() {
        let parser = DocumentParser { text_box_depth: 64 };
        let error = parser
            .parse_text_box_content("")
            .expect_err("depth limit must reject nested text boxes");
        assert!(error.to_string().contains("text box nesting"));
    }

    #[test]
    fn rejects_excessive_table_nesting_before_recursive_postprocessing() {
        let mut body = String::new();
        for _ in 0..257 {
            body.push_str("<w:tbl><w:tr><w:tc>");
        }
        body.push_str("<w:p><w:r><w:t>x</w:t></w:r></w:p>");
        for _ in 0..257 {
            body.push_str("</w:tc></w:tr></w:tbl>");
        }
        let xml = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
        );

        let error = DocumentParser::new()
            .parse_document_xml(&xml)
            .expect_err("depth limit must reject nested tables");
        assert!(error.to_string().contains("table nesting"));
    }

    #[test]
    fn soft_break_split_keeps_reference_ids_on_one_run() {
        let run = Run {
            text: "first\nsecond".to_string(),
            properties: None,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: vec!["1".to_string()],
            footnote_reference_ids: vec!["2".to_string()],
            endnote_reference_ids: vec!["3".to_string()],
            note_separator: None,
            has_footnote_ref: true,
            has_endnote_ref: true,
            revision: None,
        };

        let segments = DocumentParser::split_run_by_soft_breaks(run);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].footnote_reference_ids, ["2"]);
        assert!(segments[1..]
            .iter()
            .all(|segment| segment.comment_reference_ids.is_empty()
                && segment.footnote_reference_ids.is_empty()
                && segment.endnote_reference_ids.is_empty()
                && !segment.has_footnote_ref
                && !segment.has_endnote_ref));
    }

    #[test]
    fn complex_field_without_separate_does_not_close_outer_field() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText> PAGE </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText> AUTHOR </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:t>7</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:t>after field</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let page_field = paragraph
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref())
            .find(|field| field.cached_text.as_deref() == Some("7"))
            .expect("outer page field result");

        assert_eq!(page_field.kind, RunFieldKind::PageNumber);
        assert_eq!(page_field.cached_text.as_deref(), Some("7"));
        assert_eq!(
            paragraph.runs.last().map(|run| run.text.as_str()),
            Some("after field")
        );
        assert!(paragraph.runs.last().is_some_and(|run| run.field.is_none()));
    }

    #[test]
    fn preserves_instruction_only_complex_fields_without_separator() {
        // Word can save a complex field before producing its separate/result
        // region. The buffered instruction remains authoritative: live and
        // self-contained fields can still resolve, while unsupported fields
        // must retain their identity. Materialize the closing boundary at the
        // exact inline position, using the same metadata path as a result run.
        let body = r#"
            <w:p>
              <w:r><w:t>Before</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:t>After</w:t></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> QU</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">OTE &quot;Generated without cache&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> INCLUDETEXT &quot;appendix.docx&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> HYPERLINK &quot;https://example.com&quot; \o &quot;Open example&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        fn semantic_run(paragraph: &Paragraph) -> &Run {
            paragraph
                .runs
                .iter()
                .find(|run| run.field.is_some())
                .expect("instruction-only complex field")
        }

        assert_eq!(
            paragraphs[0]
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "BeforeAfter"
        );
        let page_run = semantic_run(paragraphs[0]);
        assert_eq!(
            page_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::PageNumber)
        );
        let page_index = paragraphs[0]
            .runs
            .iter()
            .position(|run| run.field.is_some())
            .expect("PAGE position");
        let after_index = paragraphs[0]
            .runs
            .iter()
            .position(|run| run.text == "After")
            .expect("following text position");
        assert!(page_index < after_index);

        let quote = semantic_run(paragraphs[1])
            .field
            .as_ref()
            .expect("QUOTE field");
        assert_eq!(
            quote.resolved_value.as_deref(),
            Some("Generated without cache")
        );
        let unsupported = semantic_run(paragraphs[2])
            .field
            .as_ref()
            .expect("unsupported field");
        assert_eq!(
            unsupported.kind,
            RunFieldKind::Unsupported {
                instruction: "INCLUDETEXT \"appendix.docx\"".to_string(),
            }
        );
        assert_eq!(unsupported.cached_text, None);

        let hyperlink_run = semantic_run(paragraphs[3]);
        assert!(matches!(
            hyperlink_run.field.as_ref().map(|field| &field.kind),
            Some(RunFieldKind::Hyperlink { .. })
        ));
        let hyperlink = hyperlink_run
            .hyperlink
            .as_ref()
            .expect("instruction-derived hyperlink metadata");
        assert_eq!(hyperlink.target.as_deref(), Some("https://example.com"));
        assert_eq!(hyperlink.tooltip.as_deref(), Some("Open example"));
    }

    #[test]
    fn parses_mergefield_name_without_hiding_its_editable_result() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" MERGEFIELD &quot;Customer Name&quot; \* MERGEFORMAT ">
                <w:r><w:t>Ada Lovelace</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.first())
            .expect("run");

        // Unlike computed PAGE/REF fields, a mail-merge result remains normal
        // editable run text. The semantic field metadata must therefore retain
        // the column name without moving or duplicating the visible result.
        assert_eq!(run.text, "Ada Lovelace");
        assert_eq!(
            run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::MergeField {
                name: "Customer Name".to_string(),
            })
        );
        assert_eq!(
            run.field
                .as_ref()
                .and_then(|field| field.cached_text.as_deref()),
            None
        );
    }

    #[test]
    fn parses_complex_mergefield_instruction_split_across_runs() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> MERGE</w:instrText></w:r>
              <w:r><w:instrText>FIELD &quot;Account Name&quot; \* MERGEFORMAT </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Analytical Engines</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let merge_run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("merge result run");

        // Word commonly fragments instructions at arbitrary run boundaries.
        // Concatenation must recover the field name without consuming the
        // formatting switch or turning the result into renderer-only text.
        assert_eq!(merge_run.text, "Analytical Engines");
        assert_eq!(
            merge_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::MergeField {
                name: "Account Name".to_string(),
            })
        );
    }

    #[test]
    fn parses_fillin_prompt_default_and_ask_once_without_hiding_response() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" FILLIN &quot;Enter client name:&quot; \d &quot;Anonymous User&quot; \o \* MERGEFORMAT ">
                <w:r><w:t>Ada Lovelace</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .expect("fill-in response run");

        // FILLIN answers remain ordinary editable result text. Modeling the
        // prompt contract must not move that text into renderer-only cache or
        // accidentally treat switch operands as part of the prompt.
        assert_eq!(run.text, "Ada Lovelace");
        assert_eq!(
            run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::FillIn {
                prompt: Some("Enter client name:".to_string()),
                default: Some("Anonymous User".to_string()),
                ask_once: true,
            })
        );
        assert_eq!(
            run.field
                .as_ref()
                .and_then(|field| field.cached_text.as_deref()),
            None
        );
    }

    #[test]
    fn parses_fragmented_fillin_with_omitted_prompt_and_blank_default() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> FILL</w:instrText></w:r>
              <w:r><w:instrText>IN \d "" \o </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Typed response</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fillin_run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("fill-in result run");

        // Empty quotes after \d mean an explicit blank default, not a missing
        // token. An omitted prompt must also stay absent rather than consuming
        // the default or a general-format operand as prompt text.
        assert_eq!(fillin_run.text, "Typed response");
        assert_eq!(
            fillin_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::FillIn {
                prompt: None,
                default: Some(String::new()),
                ask_once: true,
            })
        );
    }

    #[test]
    fn parses_ask_contract_without_displaying_its_saved_result() {
        let body = r#"
            <w:p>
              <w:r><w:t>Before</w:t></w:r>
              <w:fldSimple w:instr=" ASK ClientName &quot;Enter client name:&quot; \d &quot;Anonymous User&quot; \o ">
                <w:r><w:t>Hidden ASK result</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t>After</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let ask = paragraph
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("ASK field");

        // ASK stores a response in a bookmark; Word requires a separate REF to
        // display it. Keep the saved result for round-trip/future prompt state,
        // but never leak it into normal paragraph text at the ASK position.
        assert_eq!(
            ask.kind,
            RunFieldKind::Ask {
                bookmark: "ClientName".to_string(),
                prompt: Some("Enter client name:".to_string()),
                default: Some("Anonymous User".to_string()),
                ask_once: true,
            }
        );
        assert_eq!(ask.cached_text.as_deref(), Some("Hidden ASK result"));
        assert_eq!(
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "BeforeAfter"
        );
    }

    #[test]
    fn parses_fragmented_ask_with_blank_default() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> AS</w:instrText></w:r>
              <w:r><w:instrText>K EventDate "Event date?" \d "" \o </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Hidden response</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let ask = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("ASK field");

        // Arbitrary instrText fragmentation must not alter the bookmark name,
        // and empty quotes remain an explicit blank default just as in Word.
        assert_eq!(
            ask.kind,
            RunFieldKind::Ask {
                bookmark: "EventDate".to_string(),
                prompt: Some("Event date?".to_string()),
                default: Some(String::new()),
                ask_once: true,
            }
        );
        assert_eq!(ask.cached_text.as_deref(), Some("Hidden response"));
    }

    #[test]
    fn preserves_result_less_ask_without_separator_or_result_run() {
        // ASK assigns a response to a bookmark but displays nothing at its own
        // location. Word can therefore omit both the complex-field `separate`
        // marker and result run. The equivalent fldSimple form may likewise
        // have no child run. Both shapes still need a zero-width semantic run
        // so prompt hosts and save/edit paths can discover the field contract.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> AS</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">K ClientName &quot;Enter client name:&quot; </w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">\d &quot;Anonymous User&quot; \o </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" ASK EventDate &quot;Event date?&quot; \d &quot;&quot; "></w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        let fields = paragraphs
            .iter()
            .map(|paragraph| {
                assert!(
                    paragraph.runs.iter().all(|run| run.text.is_empty()),
                    "ASK must remain invisible at its field position"
                );
                let semantic_runs = paragraph
                    .runs
                    .iter()
                    .filter(|run| run.field.is_some())
                    .collect::<Vec<_>>();
                assert_eq!(semantic_runs.len(), 1, "one zero-width ASK run");
                semantic_runs[0].field.as_ref().expect("ASK field")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            fields[0].kind,
            RunFieldKind::Ask {
                bookmark: "ClientName".to_string(),
                prompt: Some("Enter client name:".to_string()),
                default: Some("Anonymous User".to_string()),
                ask_once: true,
            }
        );
        assert_eq!(fields[0].cached_text, None);
        assert_eq!(
            fields[1].kind,
            RunFieldKind::Ask {
                bookmark: "EventDate".to_string(),
                prompt: Some("Event date?".to_string()),
                default: Some(String::new()),
                ask_once: false,
            }
        );
        assert_eq!(fields[1].cached_text, None);
    }

    #[test]
    fn quote_field_replaces_a_stale_saved_result_with_its_literal() {
        let body = r#"
            <w:p>
              <w:r><w:t>Before </w:t></w:r>
              <w:fldSimple w:instr=" QUOTE &quot;Current contractual term&quot; \*Lower \*Upper \* MERGEFORMAT ">
                <w:r><w:t>Stale term</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t> after</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let quote = paragraph
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("QUOTE field");

        // QUOTE's quoted instruction operand is authoritative. A document can
        // contain an old saved result after the instruction was edited, so
        // parity requires resolving the literal instead of trusting the cache.
        assert_eq!(
            quote.kind,
            RunFieldKind::Quote {
                text: "Current contractual term".to_string(),
                case_format: Some(FieldTextCase::Upper),
            }
        );
        assert_eq!(quote.cached_text.as_deref(), Some("Stale term"));
        assert_eq!(
            quote.resolved_value.as_deref(),
            Some("CURRENT CONTRACTUAL TERM")
        );
    }

    #[test]
    fn parses_fragmented_quote_with_whitespace_and_literal_quotes() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> QU</w:instrText></w:r>
              <w:r><w:instrText>OTE "  she said \"yes\".  " \* Ca</w:instrText></w:r>
              <w:r><w:instrText>ps </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Old text</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let quote = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("QUOTE field");

        // Field instructions are routinely split at arbitrary run boundaries.
        // Preserve whitespace and escaped quotes inside the required quoted
        // operand while joining the instruction fragments into one contract.
        assert_eq!(
            quote.kind,
            RunFieldKind::Quote {
                text: "  she said \"yes\".  ".to_string(),
                case_format: Some(FieldTextCase::Caps),
            }
        );
        assert_eq!(
            quote.resolved_value.as_deref(),
            Some("  She Said \"Yes\".  ")
        );
    }

    #[test]
    fn symbol_field_resolves_hex_unicode_instead_of_a_stale_saved_result() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x20ac \u \f &quot;Segoe UI Symbol&quot; \s 14 ">
                <w:r><w:t>?</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let symbol = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("SYMBOL field");

        // Word accepts a leading 0x for hexadecimal character values. Keep
        // the formatting switches in the semantic model even though the saved
        // result is stale, so later rendering/export work does not need to
        // reinterpret the original instruction string.
        assert_eq!(
            symbol.kind,
            RunFieldKind::Symbol {
                code_point: 0x20ac,
                character_set: SymbolCharacterSet::Unicode,
                font: Some("Segoe UI Symbol".to_string()),
                font_size_points: Some(14),
                preserve_line_height: false,
            }
        );
        assert_eq!(symbol.cached_text.as_deref(), Some("?"));
        assert_eq!(symbol.resolved_value.as_deref(), Some("€"));
    }

    #[test]
    fn symbol_font_and_size_switches_override_saved_result_run_formatting() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x2605 \u \f &quot;Segoe UI Symbol&quot; \s 14 ">
                <w:r>
                  <w:rPr>
                    <w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/>
                    <w:sz w:val="20"/>
                  </w:rPr>
                  <w:t>?</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph")
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("SYMBOL result run");
        let properties = run.properties.as_ref().expect("run properties");
        let fonts = properties.fonts.as_ref().expect("script fonts");

        // SYMBOL-specific formatting switches override formatting applied
        // directly to the saved result. Populate every script slot because a
        // symbol font is a glyph mapping contract, not a Latin-only fallback.
        assert_eq!(properties.font_family.as_deref(), Some("Segoe UI Symbol"));
        assert_eq!(fonts.ascii.as_deref(), Some("Segoe UI Symbol"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Segoe UI Symbol"));
        assert_eq!(fonts.east_asia.as_deref(), Some("Segoe UI Symbol"));
        assert_eq!(fonts.complex.as_deref(), Some("Segoe UI Symbol"));
        assert_eq!(properties.font_size, Some(14.0));
    }

    #[test]
    fn fragmented_symbol_field_preserves_switch_semantics_and_safe_fallbacks() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SYM</w:instrText></w:r>
              <w:r><w:instrText>BOL 65 \u \h </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>old</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0xD800 \u ">
                <w:r><w:t>saved surrogate</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x1F642 \u ">
                <w:r><w:t>saved non-BMP</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .collect::<Vec<_>>();

        // instrText boundaries are packaging details and must not change the
        // parsed code point. The \h switch is retained because it controls
        // whether an enlarged symbol contributes to paragraph line spacing.
        assert_eq!(fields[0].resolved_value.as_deref(), Some("A"));
        assert!(matches!(
            fields[0].kind,
            RunFieldKind::Symbol {
                preserve_line_height: true,
                ..
            }
        ));
        // UTF-16 surrogate values are not Unicode scalar values. Retaining
        // Word's saved field result is safer than emitting U+FFFD or dropping
        // the field, both of which would visibly corrupt the document.
        assert_eq!(fields[1].resolved_value, None);
        assert_eq!(fields[1].cached_text.as_deref(), Some("saved surrogate"));
        // Office documents the desktop Word deviation from ECMA-376 here:
        // SYMBOL \u values outside the BMP are truncated to 16 bits rather
        // than interpreted as supplementary-plane Unicode scalar values.
        assert_eq!(fields[2].resolved_value.as_deref(), Some("\u{f642}"));
    }

    #[test]
    fn shift_jis_symbol_field_decodes_single_and_double_byte_values() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x82A0 \j ">
                <w:r><w:t>stale hiragana</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x5C \j ">
                <w:r><w:t>stale slash</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SYMBOL 0x82 \j ">
                <w:r><w:t>saved invalid lead</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .collect::<Vec<_>>();

        // Shift-JIS field values are numeric byte sequences, not Unicode code
        // points. Preserve big-endian byte order for the common two-byte form.
        assert_eq!(fields[0].resolved_value.as_deref(), Some("あ"));
        // Word explicitly keeps 0x5C and 0x7E as their ASCII forms rather than
        // displaying the legacy Yen/overline substitutions.
        assert_eq!(fields[1].resolved_value.as_deref(), Some("\\"));
        // A lone lead byte has no character. Keep Word's saved result instead
        // of exposing a replacement glyph or silently dropping the field.
        assert_eq!(fields[2].resolved_value, None);
        assert_eq!(fields[2].cached_text.as_deref(), Some("saved invalid lead"));
    }

    #[test]
    fn forward_caption_ref_includes_live_sequence_and_preserves_set_order() {
        let body = r#"
            <w:p><w:fldSimple w:instr=" REF Caption "><w:r><w:t>stale caption</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" SET Client &quot;first&quot; "><w:r><w:t>saved first</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" REF Client "><w:r><w:t>stale</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" SET Client &quot;second&quot; "><w:r><w:t>saved second</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" REF Client "><w:r><w:t>stale</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:bookmarkStart w:id="1" w:name="Caption"/>
              <w:r><w:t xml:space="preserve">Figure </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>99</w:t></w:r></w:fldSimple>
              <w:r><w:t xml:space="preserve"> 😀</w:t></w:r><w:bookmarkEnd w:id="1"/>
              <w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>100</w:t></w:r></w:fldSimple>
            </w:p>
        "#;
        let mut document = parse_document_from_body(body);
        for _ in 0..2 {
            document.resolve_structural_fields();
            let paragraphs: Vec<_> = document
                .body
                .as_ref()
                .unwrap()
                .elements
                .iter()
                .filter_map(BodyElement::as_paragraph)
                .collect();
            let value = |index: usize| {
                paragraphs[index]
                    .runs
                    .iter()
                    .find_map(|run| run.field.as_ref())
                    .unwrap()
                    .resolved_value
                    .as_deref()
            };
            assert_eq!(value(0), Some("Figure 1 😀"));
            assert_eq!(value(2), Some("first"));
            assert_eq!(value(4), Some("second"));
            let sequence_values: Vec<_> = paragraphs[5]
                .runs
                .iter()
                .filter_map(|run| run.field.as_ref())
                .map(|field| field.resolved_value.as_deref())
                .collect();
            assert_eq!(sequence_values, [Some("1"), Some("2")]);
        }
    }

    #[test]
    fn set_assignments_update_following_ref_fields_in_document_order() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SET ClientName &quot;Ada Lovelace&quot; ">
                <w:r><w:t>SET must stay hidden</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF ClientName ">
                <w:r><w:t>Old first value</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SET ClientName &quot;Grace Hopper&quot; ">
                <w:r><w:t>SET must also stay hidden</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF ClientName ">
                <w:r><w:t>Old second value</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        let field = |paragraph: usize| {
            paragraphs[paragraph]
                .runs
                .iter()
                .find_map(|run| run.field.as_ref())
                .expect("field")
        };

        // SET is an invisible assignment, not a display field. Keep its saved
        // result only as metadata and let each following REF observe the most
        // recent assignment instead of the final value for the whole document.
        assert_eq!(
            field(0).kind,
            RunFieldKind::Set {
                bookmark: "ClientName".to_string(),
                value: "Ada Lovelace".to_string(),
            }
        );
        assert_eq!(
            field(0).cached_text.as_deref(),
            Some("SET must stay hidden")
        );
        assert_eq!(field(0).resolved_value, None);
        assert_eq!(field(1).resolved_value.as_deref(), Some("Ada Lovelace"));
        assert_eq!(field(3).resolved_value.as_deref(), Some("Grace Hopper"));
        assert!(paragraphs[0].runs.iter().all(|run| run.text.is_empty()));
        assert!(paragraphs[2].runs.iter().all(|run| run.text.is_empty()));
    }

    #[test]
    fn parses_fragmented_set_with_unquoted_numeric_value() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SE</w:instrText></w:r>
              <w:r><w:instrText>T Quantity 25 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>old assignment</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF Quantity "><w:r><w:t>0</w:t></w:r></w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .collect::<Vec<_>>();

        // Word permits numbers without quotation marks. Arbitrary instrText
        // fragmentation must not alter either the bookmark or assigned value.
        assert_eq!(
            fields[0].kind,
            RunFieldKind::Set {
                bookmark: "Quantity".to_string(),
                value: "25".to_string(),
            }
        );
        assert_eq!(fields[1].resolved_value.as_deref(), Some("25"));
    }

    #[test]
    fn complex_set_without_separator_updates_following_ref() {
        // SET has no visible result, so Word may serialize the complex form as
        // begin + instruction + end without either `separate` or a result run.
        // The assignment still participates in document order: dropping its
        // zero-width semantic run leaves the following REF on its stale cache.
        // Fragment the quoted value across instrText nodes to cover Word's
        // arbitrary field-code run boundaries at the same time.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SE</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">T ClientName &quot;Ada </w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">Lovelace&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF ClientName ">
                <w:r><w:t>Stale client</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        let assignment = paragraphs[0]
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("result-less SET field");
        let reference = paragraphs[1]
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("following REF field");

        assert_eq!(
            assignment.kind,
            RunFieldKind::Set {
                bookmark: "ClientName".to_string(),
                value: "Ada Lovelace".to_string(),
            }
        );
        assert_eq!(assignment.cached_text, None);
        assert_eq!(assignment.resolved_value, None);
        assert!(paragraphs[0].runs.iter().all(|run| run.text.is_empty()));
        assert_eq!(reference.resolved_value.as_deref(), Some("Ada Lovelace"));
    }

    #[test]
    fn set_assignment_is_available_to_a_following_if_field() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SET Region &quot;North America&quot; "><w:r/></w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" IF Region = &quot;North America&quot; &quot;Approved&quot; &quot;Rejected&quot; ">
                <w:r><w:t>Stale branch</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let conditional = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .expect("conditional paragraph")
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("IF field");

        // SET bookmarks are variables, not REF-only display helpers. Word also
        // exposes them to later IF operands, including whitespace-bearing text.
        assert_eq!(
            conditional.kind,
            RunFieldKind::If {
                left: "Region".to_string(),
                left_is_quoted: false,
                operator: FieldComparisonOperator::Equal,
                right: "North America".to_string(),
                right_is_quoted: true,
                true_text: "Approved".to_string(),
                false_text: Some("Rejected".to_string()),
            }
        );
        assert_eq!(conditional.resolved_value.as_deref(), Some("Approved"));
    }

    #[test]
    fn preserves_unknown_simple_field_instruction_and_cached_result() {
        // Jade cannot safely evaluate every Word field locally. Treating an
        // unrecognized field result as ordinary text is nevertheless lossy:
        // the editor can no longer distinguish Word's cached display from
        // user-authored text, and field-aware consumers cannot report that an
        // update still belongs to Word. Keep the exact normalized instruction
        // together with the saved result until a dedicated evaluator exists.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" INCLUDETEXT &quot;appendix.docx&quot; \c DOCX ">
                <w:r><w:t>Saved appendix text</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.first())
            .expect("run");
        let field = run.field.as_ref().expect("cached Word field");
        assert_eq!(
            field.kind,
            RunFieldKind::Unsupported {
                instruction: "INCLUDETEXT \"appendix.docx\" \\c DOCX".to_string(),
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Saved appendix text"));
        // A modeled field owns its display value. Leaving a duplicate in the
        // editable run text would render the cached result twice.
        assert!(run.text.is_empty());
    }

    #[test]
    fn preserves_empty_fld_simple_for_computed_and_unsupported_fields() {
        // A producer can save fldSimple before Word has generated a cached
        // result run. The instruction is still the field: PAGE can resolve at
        // layout time, QUOTE can resolve immediately, and an unsupported field
        // must remain identifiable for a future updater. Keep the synthetic
        // zero-width run at the exact inline position between ordinary runs.
        let body = r#"
            <w:p>
              <w:r><w:t>Before</w:t></w:r>
              <w:fldSimple w:instr=" PAGE "></w:fldSimple>
              <w:r><w:t>After</w:t></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" QUOTE &quot;Generated without cache&quot; "></w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" INCLUDETEXT &quot;appendix.docx&quot; "></w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" HYPERLINK &quot;https://example.com&quot; \o &quot;Open example&quot; "></w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();

        assert_eq!(paragraphs[0].runs.len(), 3);
        assert_eq!(paragraphs[0].runs[0].text, "Before");
        assert_eq!(paragraphs[0].runs[2].text, "After");
        let page = paragraphs[0].runs[1]
            .field
            .as_ref()
            .expect("empty PAGE field");
        assert_eq!(page.kind, RunFieldKind::PageNumber);
        assert_eq!(page.cached_text, None);

        let quote = paragraphs[1]
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("empty QUOTE field");
        assert_eq!(
            quote.kind,
            RunFieldKind::Quote {
                text: "Generated without cache".to_string(),
                case_format: None,
            }
        );
        assert_eq!(
            quote.resolved_value.as_deref(),
            Some("Generated without cache")
        );
        assert_eq!(quote.cached_text, None);

        let unsupported = paragraphs[2]
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("empty unsupported field");
        assert_eq!(
            unsupported.kind,
            RunFieldKind::Unsupported {
                instruction: "INCLUDETEXT \"appendix.docx\"".to_string(),
            }
        );
        assert_eq!(unsupported.cached_text, None);

        let hyperlink_run = paragraphs[3]
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("empty HYPERLINK field");
        assert_eq!(
            hyperlink_run.field.as_ref().map(|field| &field.kind),
            Some(&RunFieldKind::Hyperlink {
                target: Some("https://example.com".to_string()),
                anchor: None,
                tooltip: Some("Open example".to_string()),
                target_frame: None,
            })
        );
        let hyperlink = hyperlink_run
            .hyperlink
            .as_ref()
            .expect("instruction-derived hyperlink metadata");
        assert_eq!(hyperlink.target.as_deref(), Some("https://example.com"));
        assert_eq!(hyperlink.target_mode.as_deref(), Some("External"));
        assert_eq!(hyperlink.tooltip.as_deref(), Some("Open example"));
    }

    #[test]
    fn preserves_fragmented_unknown_complex_field_instruction() {
        // Word is free to split one instruction at arbitrary run boundaries.
        // The split below deliberately falls inside both the field name and a
        // quoted operand; preserving only individual instrText nodes would not
        // be enough to identify or round-trip the field semantically.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> INCLUDE</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">TEXT "chapter</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve"> one.docx" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Saved chapter text</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("fragmented cached Word field");

        assert_eq!(
            field.kind,
            RunFieldKind::Unsupported {
                instruction: "INCLUDETEXT \"chapter one.docx\"".to_string(),
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("Saved chapter text"));
    }

    #[test]
    fn models_private_and_addin_as_non_display_fields() {
        // Word classifies PRIVATE as a cold field with no result and ADDIN as
        // hidden add-in storage. A producer may nevertheless leave stale text
        // between the separator/end markers. Preserve that cache for lossless
        // inspection, but mark the field so renderers never expose it.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" PRIVATE vendor-payload ">
                <w:r><w:t>Leaked private cache</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> ADD</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">IN vendor-payload </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Leaked add-in cache</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 2);
        assert_eq!(
            fields[0].kind,
            RunFieldKind::NonDisplay {
                instruction: "PRIVATE vendor-payload".to_string(),
            }
        );
        assert_eq!(
            fields[0].cached_text.as_deref(),
            Some("Leaked private cache")
        );
        assert_eq!(
            fields[1].kind,
            RunFieldKind::NonDisplay {
                instruction: "ADDIN vendor-payload".to_string(),
            }
        );
        assert_eq!(
            fields[1].cached_text.as_deref(),
            Some("Leaked add-in cache")
        );
    }

    #[test]
    fn preserves_result_less_complex_fields_without_separator() {
        // Word normally writes cold fields as begin + instruction + end: there
        // is no `separate` marker and no result run because these fields have
        // nothing to display. The parser must still retain one semantic run so
        // save/edit paths do not silently discard the opaque Word instruction.
        //
        // PRIVATE and ADDIN carry application-owned data. XE and TA are the
        // hidden source entries used to build an index and a table of
        // authorities. Fragment ADDIN across instrText nodes as Word is free to
        // split any field instruction at arbitrary run boundaries.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PRIVATE vendor-payload </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> ADD</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">IN vendor-payload </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> XE &quot;Alpha entry&quot; \f &quot;main&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TA \l &quot;Case citation&quot; \s &quot;12&quot; </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" PRIVATE simple-payload "></w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .map(|paragraph| {
                assert!(
                    paragraph.runs.iter().all(|run| run.text.is_empty()),
                    "cold field must remain invisible"
                );
                let semantic_runs = paragraph
                    .runs
                    .iter()
                    .filter(|run| run.field.is_some())
                    .collect::<Vec<_>>();
                assert_eq!(semantic_runs.len(), 1, "one zero-width semantic run");
                let field = semantic_runs[0]
                    .field
                    .as_ref()
                    .expect("result-less Word field");
                assert_eq!(field.cached_text, None);
                field
            })
            .collect::<Vec<_>>();

        assert_eq!(fields.len(), 5);
        let expected = [
            "PRIVATE vendor-payload",
            "ADDIN vendor-payload",
            "XE \"Alpha entry\" \\f \"main\"",
            "TA \\l \"Case citation\" \\s \"12\"",
            "PRIVATE simple-payload",
        ];
        for (field, instruction) in fields.into_iter().zip(expected) {
            assert_eq!(
                field.kind,
                RunFieldKind::NonDisplay {
                    instruction: instruction.to_string(),
                }
            );
        }
    }

    #[test]
    fn nested_non_display_field_contributes_empty_operand_not_cached_storage() {
        // A stale PRIVATE cache nested in IF must not become Expression1. Word
        // defines PRIVATE as result-less, so the enclosing field receives one
        // empty operand and keeps its branch token boundaries intact.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PRIVATE vendor-payload </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Leaked private cache</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = "" "Private stayed hidden" "Private leaked" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: String::new(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: String::new(),
                right_is_quoted: true,
                true_text: "Private stayed hidden".to_string(),
                false_text: Some("Private leaked".to_string()),
            }
        );
        assert_eq!(
            field.resolved_value.as_deref(),
            Some("Private stayed hidden")
        );
    }

    #[test]
    fn nested_result_less_field_without_separator_contributes_empty_operand() {
        // The canonical cold-field shape has no cached-result region at all.
        // Closing PRIVATE must therefore classify it from the still-buffered
        // instruction and inject exactly one empty operand into the outer IF;
        // otherwise the `=` token shifts into Expression1 and the IF is lost.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PRIVATE vendor-payload </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
              <w:r><w:instrText xml:space="preserve"> = "" "Hidden" "Leaked" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Stale outer result</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("outer IF field");

        assert_eq!(
            field.kind,
            RunFieldKind::If {
                left: String::new(),
                left_is_quoted: true,
                operator: FieldComparisonOperator::Equal,
                right: String::new(),
                right_is_quoted: true,
                true_text: "Hidden".to_string(),
                false_text: Some("Leaked".to_string()),
            }
        );
        assert_eq!(field.resolved_value.as_deref(), Some("Hidden"));
    }

    #[test]
    fn parses_pageref_field_and_captures_bookmark_name_and_cached_text() {
        // `PAGEREF _Ref123 \h` resolves to the page number of the named
        // bookmark. ECMA-376 Part 1 §17.16.5.45. The bookmark name is the first
        // token after the field name; the `\h` switch is retained as internal
        // hyperlink metadata. The cached
        // result text (Word's last-computed page number) is preserved so the
        // renderer can fall back to it before layout resolves the page.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PAGEREF _Ref456 \h </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>7</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref().map(|f| (r, f))))
            .expect("pageref field");
        let (run, field) = field;
        assert_eq!(
            field.kind,
            RunFieldKind::PageRef {
                name: "_Ref456".to_string(),
                hyperlink: true,
            }
        );
        // REF/PAGEREF results remain in the editable run text so selection
        // coordinates match what the renderer displays around the atomic field.
        assert_eq!(field.cached_text, None);
        assert_eq!(run.text, "7");
    }

    #[test]
    fn parses_pageref_field_from_fld_simple() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" PAGEREF heading2 ">
                <w:r><w:t>3</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let (run, field) = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref().map(|f| (r, f))))
            .expect("pageref field");
        assert_eq!(
            field.kind,
            RunFieldKind::PageRef {
                name: "heading2".to_string(),
                hyperlink: false,
            }
        );
        assert_eq!(field.cached_text, None);
        assert_eq!(run.text, "3");
    }

    #[test]
    fn ref_and_pageref_h_switch_create_internal_bookmark_hyperlinks() {
        // Word's `\\h` switch turns REF/PAGEREF results into jumps to the
        // referenced bookmark. Keep that interaction metadata on the result
        // run instead of treating the switch as display-only syntax. This is
        // especially important for generated TOCs, whose page-number fields
        // are commonly serialized as `PAGEREF bookmark \\h`.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="target"/>
              <w:r><w:t>Destination</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF target \h ">
                <w:r><w:t>Destination</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PAGEREF target \H </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>1</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF target ">
                <w:r><w:t>Destination</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();

        for paragraph_index in [1, 2] {
            let result_run = paragraphs[paragraph_index]
                .runs
                .iter()
                .find(|run| run.field.is_some())
                .expect("field result run");
            let hyperlink = result_run
                .hyperlink
                .as_ref()
                .expect("REF/PAGEREF \\h hyperlink");
            assert_eq!(hyperlink.anchor.as_deref(), Some("target"));
            assert_eq!(hyperlink.target, None);
            assert_eq!(hyperlink.target_mode, None);
        }

        let plain_ref_run = paragraphs[3]
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .expect("plain REF result run");
        assert_eq!(plain_ref_run.hyperlink, None);
    }

    #[test]
    fn parses_field_instruction_split_across_multiple_instr_text_runs() {
        // Word frequently splits one field code across several `<w:instrText>`
        // runs (rsid boundaries, in-place editing). The parser must preserve the
        // inter-run whitespace so the field name and its bookmark argument stay
        // separated. Here `PAGEREF _Toc1 \h` is split right at the space between
        // the field name and the bookmark name; a naive per-run trim would
        // concatenate to `PAGEREF_Toc1 \h` and fail to recognize the field.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> PAGEREF </w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">_Toc1 \h </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>5</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let (run, field) = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref().map(|f| (r, f))))
            .expect("pageref field from split instruction");
        assert_eq!(
            field.kind,
            RunFieldKind::PageRef {
                name: "_Toc1".to_string(),
                hyperlink: true,
            }
        );
        assert_eq!(field.cached_text, None);
        assert_eq!(run.text, "5");
    }

    #[test]
    fn split_sequence_results_advance_once_per_source_field() {
        for field in [
            r#"<w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>sta</w:t></w:r><w:r><w:t>le</w:t></w:r></w:fldSimple>"#,
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> SEQ Figure </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>sta</w:t></w:r><w:r><w:t>le</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        ] {
            let doc = parse_document_from_body(&format!("<w:p>{field}{field}</w:p>"));
            let paragraph = doc.body.as_ref().unwrap().elements[0]
                .as_paragraph()
                .unwrap();
            let fields = paragraph
                .runs
                .iter()
                .filter_map(|run| run.field.as_ref())
                .collect::<Vec<_>>();
            assert_eq!(fields.len(), 2, "{field}");
            assert_eq!(fields[0].cached_text.as_deref(), Some("stale"));
            assert_eq!(fields[0].resolved_value.as_deref(), Some("1"));
            assert_eq!(fields[1].resolved_value.as_deref(), Some("2"));
        }
    }

    #[test]
    fn parses_eq_overstrike_distributed_text_field() {
        // Word's Japanese distributed-text command can serialize as an `EQ`
        // field whose `\o\ad` arguments carry the visible text and the guide
        // text to measure. ECMA-376 Part 1 §17.16.5.22 defines `EQ`; preserving
        // both arguments lets the renderer fit the result instead of treating
        // the field as ordinary cached text.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> EQ \o</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">\ad("契約","　　") </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>契約</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref()))
            .expect("EQ distributed text field");
        assert_eq!(
            field.kind,
            RunFieldKind::EqDistributed {
                text: "契約".to_string(),
                guide: "　　".to_string(),
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("契約"));
    }

    #[test]
    fn parses_eq_overstrike_ruby_field() {
        // Older Word documents carry ruby as an `EQ` field (§17.16.5.22): the
        // guide raised by `\s\up` over the base inside an `\o` overstrike,
        // with the alignment (`\* jc`) and guide size (`\* hps`) as switches.
        // Word computes the result itself, so the saved result is empty.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> EQ \* jc2 \* "Font:Times-Roman" \* hps11 \o\ad(\s\up11(じゆん),閏)</w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref()))
            .expect("EQ ruby field");
        assert_eq!(
            field.kind,
            RunFieldKind::EqRuby {
                text: "閏".to_string(),
                guide: "じゆん".to_string(),
                align: Some("distributespace".to_string()),
                size_half_points: Some(11),
                raise_points: 11,
            }
        );
        let ruby = field.kind.eq_ruby_annotation().expect("ruby");
        assert_eq!(ruby.text, "じゆん");
        assert_eq!(ruby.size_pt, Some(5.5));
        assert_eq!(ruby.raise_half_points, Some(22));
    }

    #[test]
    fn eq_overstrike_of_an_enclosure_is_an_enclosed_character() {
        assert_eq!(
            parse_eq_enclosed_field(r"EQ \o\ac(○,秘)"),
            Some(RunFieldKind::EqEnclosed {
                text: "秘".into(),
                enclosure: "○".into()
            })
        );
        assert_eq!(
            parse_eq_enclosed_field(r"eq \o(株,□)"),
            Some(RunFieldKind::EqEnclosed {
                text: "株".into(),
                enclosure: "□".into()
            })
        );
        assert_eq!(parse_eq_enclosed_field(r#"EQ \o\ad("契約","　　")"#), None);
    }

    #[test]
    fn eq_ruby_alignment_follows_the_overstrike_switch_without_jc() {
        assert_eq!(
            parse_eq_ruby_field(r"EQ \o\al(\s\up 9(かな),仮名)"),
            Some(RunFieldKind::EqRuby {
                text: "仮名".to_string(),
                guide: "かな".to_string(),
                align: Some("left".to_string()),
                size_half_points: None,
                raise_points: 9,
            })
        );
        // `\o\ad("契約","　　")` is distributed text, not ruby.
        assert_eq!(parse_eq_ruby_field(r#"EQ \o\ad("契約","　　")"#), None);
    }

    #[test]
    fn parses_table_of_contents_field_without_leaking_instruction_text() {
        // A realistic TOC: the unrecognized `TOC \o "1-3" \h \z \u` instruction
        // must NOT leak into the body text, the entry heading renders as ordinary
        // text, and the nested `PAGEREF` page-number field is captured with its
        // cached value. Mirrors the structure Word emits in `toc.docx`.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">\z \u </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:hyperlink w:anchor="_Toc214570704" w:history="1">
                <w:r><w:t>Heading One</w:t></w:r>
                <w:r><w:tab/></w:r>
                <w:r><w:fldChar w:fldCharType="begin"/></w:r>
                <w:r><w:instrText xml:space="preserve"> PAGEREF _Toc214570704 \h </w:instrText></w:r>
                <w:r><w:fldChar w:fldCharType="separate"/></w:r>
                <w:r><w:t>1</w:t></w:r>
                <w:r><w:fldChar w:fldCharType="end"/></w:r>
              </w:hyperlink>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("toc paragraph");
        let concatenated: String = paragraph.runs.iter().map(|r| r.text.as_str()).collect();
        // The TOC instruction (and its switches) must not appear in body text.
        assert!(
            !concatenated.contains("TOC"),
            "TOC instruction leaked into body text: {concatenated:?}"
        );
        assert!(
            !concatenated.contains("PAGEREF"),
            "PAGEREF instruction leaked into body text: {concatenated:?}"
        );
        // The entry heading text survives as ordinary run text.
        assert!(
            concatenated.contains("Heading One"),
            "entry heading missing: {concatenated:?}"
        );
        // The nested PAGEREF field is captured with the bookmark name + cached page.
        let (run, pageref) = paragraph
            .runs
            .iter()
            .find_map(|r| r.field.as_ref().map(|field| (r, field)))
            .expect("nested pageref field");
        assert_eq!(
            pageref.kind,
            RunFieldKind::PageRef {
                name: "_Toc214570704".to_string(),
                hyperlink: true,
            }
        );
        assert_eq!(pageref.cached_text, None);
        assert_eq!(run.text, "1");
    }

    #[test]
    fn captures_toc_field_switches_and_generates_outline_from_headings() {
        // The TOC field's switches are captured on the document, and the outline
        // generator rebuilds entries from the heading paragraphs that follow —
        // honoring the `\o "1-2"` level range and anchoring each entry to its
        // `_Toc…` bookmark.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \o "1-2" \h \z \u </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Intro</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
              <w:bookmarkStart w:id="1" w:name="_Toc100"/>
              <w:r><w:t>Intro</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:pPr><w:outlineLvl w:val="1"/></w:pPr>
              <w:r><w:t>Background</w:t></w:r>
            </w:p>
            <w:p>
              <w:pPr><w:pStyle w:val="Heading3"/></w:pPr>
              <w:r><w:t>Deep detail</w:t></w:r>
            </w:p>
            <w:p>
              <w:r><w:t>Body text, not a heading</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);

        // The TOC field switches are captured on the document.
        assert_eq!(doc.toc_fields.len(), 1);
        let toc = &doc.toc_fields[0];
        assert_eq!((toc.outline_min, toc.outline_max), (1, 2));
        assert!(toc.hyperlinks);
        assert!(toc.use_applied_outline_levels);

        // Outline generation honors the `\o "1-2"` range: Heading1 (level 1) and
        // the explicit outlineLvl=1 (level 2) are included; Heading3 (level 3)
        // and the plain body paragraph are excluded.
        let entries = doc.outline();
        let summary: Vec<(&str, u8, Option<&str>)> = entries
            .iter()
            .map(|e| (e.text.as_str(), e.level, e.bookmark.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![("Intro", 1, Some("_Toc100")), ("Background", 2, None)]
        );
    }

    #[test]
    fn toc_u_switch_controls_directly_applied_outline_levels() {
        // Word distinguishes outline levels inherited from a paragraph style
        // (`TOC \\o`) from an outline level applied directly to one paragraph
        // (`TOC \\u`). A common example is a Normal paragraph promoted through
        // Paragraph > Outline level without changing it to a Heading style.
        // Without `\\u`, that paragraph must not leak into a heading-based TOC.
        let without_u = parse_document_from_body(
            r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \o "1-3" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>No table of contents entries found.</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:pPr>
                <w:pStyle w:val="Normal"/>
                <w:outlineLvl w:val="1"/>
              </w:pPr>
              <w:r><w:t>Direct outline paragraph</w:t></w:r>
            </w:p>
            <w:p>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
              <w:r><w:t>Styled heading</w:t></w:r>
            </w:p>
        "#,
        );
        assert_eq!(
            without_u
                .outline()
                .iter()
                .map(|entry| (entry.text.as_str(), entry.level))
                .collect::<Vec<_>>(),
            vec![("Styled heading", 1)]
        );

        // Adding `\\u` opts the directly formatted paragraph into the TOC at
        // its applied level while retaining ordinary Heading-style entries.
        // This paired assertion prevents a fix that merely drops all explicit
        // `w:outlineLvl` properties from outline generation.
        let with_u = parse_document_from_body(
            r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \u </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Direct outline paragraph</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:pPr>
                <w:pStyle w:val="Normal"/>
                <w:outlineLvl w:val="1"/>
              </w:pPr>
              <w:r><w:t>Direct outline paragraph</w:t></w:r>
            </w:p>
            <w:p>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
              <w:r><w:t>Styled heading</w:t></w:r>
            </w:p>
        "#,
        );
        assert_eq!(
            with_u
                .outline()
                .iter()
                .map(|entry| (entry.text.as_str(), entry.level))
                .collect::<Vec<_>>(),
            vec![("Direct outline paragraph", 2), ("Styled heading", 1)]
        );
    }

    #[test]
    fn toc_f_and_l_switches_collect_hidden_tc_fields() {
        // TC fields are hidden and normally omit `separate`; their instruction
        // can still be fragmented across runs just like any Word field code.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \f i \l "2-3" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Illustrations</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:r><w:t>Visible body A</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TC "Architecture</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve"> overview" \f i \l 2 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:r><w:t>Visible body B</w:t></w:r>
              <w:fldSimple w:instr=" TC &quot;Deployment map&quot; \f i \l 3 \n "><w:r/></w:fldSimple>
            </w:p>
            <w:p><w:fldSimple w:instr=" TC &quot;Wrong type&quot; \f t \l 2 "><w:r/></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" TC &quot;Too deep&quot; \f i \l 4 "><w:r/></w:fldSimple></w:p>
        "#;
        let doc = parse_document_from_body(body);

        assert_eq!(doc.toc_entry_fields.len(), 4);
        assert_eq!(doc.toc_entry_fields[0].text, "Architecture overview");
        assert!(doc.toc_entry_fields[1].hide_page_number);
        let toc = &doc.toc_fields[0];
        assert!(toc.use_toc_entry_fields);
        assert_eq!(toc.toc_entry_identifier.as_deref(), Some("i"));
        assert_eq!(toc.toc_entry_levels, Some((2, 3)));

        let entries = doc.outline();
        let summary: Vec<(&str, u8, usize)> = entries
            .iter()
            .map(|entry| (entry.text.as_str(), entry.level, entry.paragraph))
            .collect();
        assert_eq!(
            summary,
            vec![("Architecture overview", 2, 1), ("Deployment map", 3, 2)]
        );
    }

    #[test]
    fn ordinary_toc_merges_untyped_tc_fields_with_headings_in_document_order() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText> TOC \o "1-3" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Contents</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>First heading</w:t></w:r></w:p>
            <w:p><w:r><w:t>Manual source</w:t></w:r>
              <w:fldSimple w:instr=" TC &quot;Manual entry&quot; \l 2 \n "><w:r/></w:fldSimple>
            </w:p>
            <w:p><w:r><w:t>Typed source</w:t></w:r>
              <w:fldSimple w:instr=" TC &quot;Illustration only&quot; \f i \l 2 "><w:r/></w:fldSimple>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Last heading</w:t></w:r></w:p>
        "#;
        let doc = parse_document_from_body(body);

        let entries = doc.outline();
        let summary: Vec<(&str, u8, bool)> = entries
            .iter()
            .map(|entry| (entry.text.as_str(), entry.level, entry.hide_page_number))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("First heading", 1, false),
                ("Manual entry", 2, true),
                ("Last heading", 2, false),
            ]
        );
    }

    #[test]
    fn toc_bookmark_switch_limits_generated_outline_to_bookmarked_range() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \b "PartOne" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Local contents</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Before</w:t></w:r></w:p>
            <w:p>
              <w:bookmarkStart w:id="20" w:name="PartOne"/>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Included one</w:t></w:r>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Included two</w:t></w:r></w:p>
            <w:p>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Included three</w:t></w:r>
              <w:bookmarkEnd w:id="20"/>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>After</w:t></w:r></w:p>
        "#;
        let doc = parse_document_from_body(body);

        assert_eq!(doc.toc_fields[0].bookmark_name.as_deref(), Some("PartOne"));
        let outline = doc.outline();
        let texts: Vec<&str> = outline.iter().map(|entry| entry.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Included one", "Included two", "Included three"]
        );

        // Word reports an undefined-bookmark field result. The structured
        // outline equivalent is an empty entry set, never a fallback to the
        // entire document (which would silently produce a misleading TOC).
        let mut missing = doc.toc_fields[0].clone();
        missing.bookmark_name = Some("MissingBookmark".to_string());
        assert!(doc.generate_outline(&missing).is_empty());
    }

    #[test]
    fn generated_toc_entry_includes_visible_field_results_from_heading() {
        let body = r#"
            <w:p>
              <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
              <w:r><w:t xml:space="preserve">Figure </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>7</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t>: Architecture</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);

        // SEQ is resolved structurally to 1 even when Word's stale cached
        // result says 7. A regenerated TOC must copy the visible resolved field
        // value rather than silently dropping the field run from its heading.
        let outline = doc.outline();
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0].text, "Figure 1: Architecture");
    }

    #[test]
    fn toc_caption_switch_collects_only_matching_seq_caption_paragraphs() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \h \z \c "Figure" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Figure list</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:t xml:space="preserve">Figure </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>9</w:t></w:r></w:fldSimple>
              <w:r><w:t>: Architecture</w:t></w:r>
            </w:p>
            <w:p>
              <w:r><w:t xml:space="preserve">Table </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Table "><w:r><w:t>4</w:t></w:r></w:fldSimple>
              <w:r><w:t>: Results</w:t></w:r>
            </w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Ordinary heading</w:t></w:r></w:p>
            <w:p>
              <w:r><w:t xml:space="preserve">FIGURE </w:t></w:r>
              <w:fldSimple w:instr=" SEQ figure "><w:r><w:t>10</w:t></w:r></w:fldSimple>
              <w:r><w:t>: Deployment</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);

        assert_eq!(doc.toc_fields[0].caption_label.as_deref(), Some("Figure"));
        let outline = doc.outline();
        let summary: Vec<(&str, u8)> = outline
            .iter()
            .map(|entry| (entry.text.as_str(), entry.level))
            .collect();
        assert_eq!(
            summary,
            vec![("Figure 1: Architecture", 1), ("FIGURE 2: Deployment", 1)]
        );
    }

    #[test]
    fn toc_caption_text_switch_omits_label_number_and_separator() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> TOC \h \z \a "Figure" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>Figure list</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:r><w:t xml:space="preserve">Figure </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>8</w:t></w:r></w:fldSimple>
              <w:r><w:t>: Mercury</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);

        let toc = &doc.toc_fields[0];
        assert_eq!(toc.caption_label.as_deref(), Some("Figure"));
        assert!(toc.omit_caption_label_and_number);
        let outline = doc.outline();
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0].text, "Mercury");
    }

    #[test]
    fn parses_ref_field_and_resolves_bookmark_text() {
        // `REF target` inserts the text spanned by the named bookmark
        // (§17.16.5.52). Define a bookmark around "World" then reference it.
        let body = r#"
            <w:p>
              <w:r><w:t>Hello </w:t></w:r>
              <w:bookmarkStart w:id="1" w:name="greeting"/>
              <w:r><w:t>World</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF greeting \h ">
                <w:r><w:t>World</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let (run, field) = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.get(1))
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref().map(|f| (r, f))))
            .expect("ref field");
        assert_eq!(
            field.kind,
            RunFieldKind::Ref {
                name: "greeting".to_string(),
                case_format: None,
                numbering: false,
                hyperlink: true,
            }
        );
        // resolve_structural_fields ran during parse and filled resolved_value
        // with the bookmark's text.
        assert_eq!(field.resolved_value.as_deref(), Some("World"));
        assert_eq!(field.cached_text, None);
        assert_eq!(run.text, "World");
    }

    #[test]
    fn ref_paragraph_number_switch_keeps_cached_result() {
        // `REF bookmark \\n` asks Word for the bookmarked paragraph's number,
        // not for all text spanned by the bookmark. Resolving it through the
        // ordinary bookmark-text path can inject many paragraphs into one run
        // and make selection offsets diverge from the editable model.
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="section"/>
              <w:r><w:t>First bookmarked paragraph</w:t></w:r>
            </w:p>
            <w:p>
              <w:r><w:t>Last bookmarked paragraph</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF section \n \h ">
                <w:r><w:t>7</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.get(2))
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("paragraph-number ref field");

        assert_eq!(
            field.kind,
            RunFieldKind::Ref {
                name: "section".to_string(),
                case_format: None,
                numbering: true,
                hyperlink: true,
            }
        );
        assert_eq!(field.resolved_value, None);
        assert_eq!(field.cached_text.as_deref(), Some("7"));
    }

    #[test]
    fn ref_fields_apply_word_capitalization_switches_to_bookmark_text() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="1" w:name="release"/>
              <w:r><w:t>production-ready</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" REF release \* Caps ">
                <w:r><w:t>stale caps</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> RE</w:instrText></w:r>
              <w:r><w:instrText>F release \*Low</w:instrText></w:r>
              <w:r><w:instrText>er </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>STALE LOWER</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let values = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .skip(1)
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // The bookmark lookup must happen before applying the general format
        // switch. Saved field results are deliberately stale to prove the
        // authoritative bookmark text is reformatted during load.
        assert_eq!(values, [Some("Production-Ready"), Some("production-ready")]);
    }

    #[test]
    fn ref_to_unknown_bookmark_leaves_resolved_value_none() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" REF missing ">
                <w:r><w:t>stale</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let (run, field) = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref().map(|f| (r, f))))
            .expect("ref field");
        assert_eq!(
            field.kind,
            RunFieldKind::Ref {
                name: "missing".to_string(),
                case_format: None,
                numbering: false,
                hyperlink: false,
            }
        );
        // No matching bookmark => resolved_value stays None; the renderer falls
        // back to the editable saved result.
        assert_eq!(field.resolved_value, None);
        assert_eq!(field.cached_text, None);
        assert_eq!(run.text, "stale");
    }

    #[test]
    fn parses_seq_fields_and_numbers_them_in_document_order() {
        // Each `SEQ Figure` increments a counter keyed by the identifier
        // (case-insensitive). ECMA-376 Part 1 §17.16.5.64.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* ARABIC ">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table ">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ figure ">
                <w:r><w:t>2</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved: Vec<(RunFieldKind, Option<String>)> = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|p| p.runs.iter().find_map(|r| r.field.as_ref()))
            .map(|f| (f.kind.clone(), f.resolved_value.clone()))
            .collect();
        assert_eq!(
            resolved,
            vec![
                (
                    RunFieldKind::Seq {
                        identifier: "Figure".to_string(),
                        restart_at: None,
                        repeat_previous: false,
                        hide_result: false,
                        restart_after_heading: None,
                        bookmark: None,
                        number_format: Some(SeqNumberFormat::Arabic),
                        case_format: None,
                    },
                    Some("1".to_string())
                ),
                (
                    RunFieldKind::Seq {
                        identifier: "Table".to_string(),
                        restart_at: None,
                        repeat_previous: false,
                        hide_result: false,
                        restart_after_heading: None,
                        bookmark: None,
                        number_format: None,
                        case_format: None,
                    },
                    Some("1".to_string())
                ),
                // "figure" shares the (case-insensitive) counter with "Figure".
                (
                    RunFieldKind::Seq {
                        identifier: "figure".to_string(),
                        restart_at: None,
                        repeat_previous: false,
                        hide_result: false,
                        restart_after_heading: None,
                        bookmark: None,
                        number_format: None,
                        case_format: None,
                    },
                    Some("2".to_string())
                ),
            ]
        );
    }

    #[test]
    fn autonum_counts_paragraphs_once_and_keeps_table_order_and_heading_resets() {
        let doc = parse_document_from_body(
            r#"
          <w:p><w:fldSimple w:instr="AUTONUM"/><w:fldSimple w:instr="AUTONUM \s :"/></w:p>
          <w:tbl><w:tr><w:tc><w:p><w:fldSimple w:instr="AUTONUMLGL \e"/></w:p></w:tc></w:tr></w:tbl>
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:fldSimple w:instr="AUTONUM"/></w:p>
          <w:p><w:fldSimple w:instr="AUTONUMLGL"/></w:p>
          <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:fldSimple w:instr="AUTONUMLGL"/></w:p>
          <w:p><w:fldSimple w:instr="AUTONUMLGL \e"/></w:p>
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:fldSimple w:instr="AUTONUM"/></w:p>
          <w:p><w:fldSimple w:instr="AUTONUM"/></w:p>
        "#,
        );
        fn collect(elements: &[BodyElement], values: &mut Vec<String>) {
            for element in elements {
                match element {
                    BodyElement::Paragraph(paragraph) => values.extend(
                        paragraph
                            .runs
                            .iter()
                            .filter_map(|run| run.field.as_ref())
                            .filter_map(|field| field.resolved_value.clone()),
                    ),
                    BodyElement::Table(table) => {
                        for row in &table.rows {
                            for cell in &row.cells {
                                collect(&cell.blocks, values);
                            }
                        }
                    }
                    BodyElement::Math(_) => {}
                }
            }
        }
        let mut values = Vec::new();
        collect(&doc.body.as_ref().unwrap().elements, &mut values);
        assert_eq!(
            values,
            ["1.", "1:", "2", "1.", "1.1.", "1.1.", "1.1.1", "2.", "1."]
        );
    }

    #[test]
    fn autonumout_unknown_heading_format_preserves_cache_and_advances_counters() {
        let doc = parse_document_from_body(
            r#"
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
            <w:fldSimple w:instr="AUTONUMOUT"><w:r><w:t>IV.</w:t></w:r></w:fldSimple></w:p>
          <w:p><w:fldSimple w:instr="AUTONUMOUT"/></w:p>
        "#,
        );
        let fields = doc
            .body
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .flat_map(|paragraph| &paragraph.runs)
            .filter_map(|run| run.field.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].resolved_value, None);
        assert_eq!(fields[0].cached_text.as_deref(), Some("IV."));
        assert_eq!(fields[1].resolved_value.as_deref(), Some("1."));
    }

    #[test]
    fn original_autonum_fields_resolve_without_cached_result_runs() {
        let mut doc = parse_document_from_body(
            r#"
            <w:p><w:fldSimple w:instr=" AUTONUM \* Arabic "/><w:r><w:t>abc</w:t></w:r></w:p>
            <w:p><w:fldSimple w:instr=" AUTONUM \* Arabic \s "/><w:r><w:t>uuu</w:t></w:r></w:p>
            <w:p><w:fldSimple w:instr=" AUTONUMLGL \* alphabetic "/><w:r><w:t>test</w:t></w:r></w:p>
            <w:p><w:r><w:t>mmmm</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText>AUTONUMOUT</w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
        "#,
        );
        let values = |doc: &Document| {
            doc.body
                .as_ref()
                .unwrap()
                .elements
                .iter()
                .filter_map(BodyElement::as_paragraph)
                .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
                .map(|field| field.resolved_value.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            values(&doc),
            [
                Some("1".into()),
                Some("2".into()),
                Some("c".into()),
                Some("4.".into())
            ]
        );
        assert_eq!(
            doc.body
                .as_ref()
                .unwrap()
                .elements
                .iter()
                .filter_map(BodyElement::as_paragraph)
                .flat_map(|paragraph| &paragraph.runs)
                .filter(|run| run.field.is_some())
                .count(),
            4
        );
        doc.resolve_structural_fields();
        assert_eq!(
            values(&doc),
            [
                Some("1".into()),
                Some("2".into()),
                Some("c".into()),
                Some("4.".into())
            ]
        );
        assert!(doc
            .body
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .flat_map(|paragraph| &paragraph.runs)
            .filter_map(|run| run.field.as_ref())
            .all(|field| field.cached_text.is_none()));
    }

    #[test]
    fn seq_general_number_formats_match_word_case_and_sequence_rules() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \r 4 \* ROMAN ">
                <w:r><w:t>stale upper roman</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure \*rom</w:instrText></w:r>
              <w:r><w:instrText>an </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale lower roman</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* ALPHABETIC ">
                <w:r><w:t>stale upper alphabetic</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* alphabetic ">
                <w:r><w:t>stale lower alphabetic</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table \* ARABIC ">
                <w:r><w:t>stale arabic</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Formatting is applied after counter updates: the restart field is
        // four, and the following Figure fields continue with five, six, and
        // seven. Word distinguishes the spelling case of ROMAN/roman and
        // ALPHABETIC/alphabetic. The complex lower-roman instruction is split
        // across runs and uses compact `\*roman` syntax to cover both parser
        // boundaries that commonly occur in Word-authored documents.
        assert_eq!(
            resolved,
            [Some("IV"), Some("v"), Some("F"), Some("g"), Some("1")]
        );
    }

    #[test]
    fn seq_english_text_number_formats_use_the_live_counter_value() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \r 21 \* Ordinal ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale ordinal</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* CardText ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale cardinal text</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* OrdText ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale ordinal text</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Word applies text-number formats to the current numeric SEQ value,
        // not to the field's saved display text. Starting at 21 also covers
        // the English ordinal suffix and hyphenated tens used by CardText and
        // OrdText. The explicit run language is important: these formats are
        // language-dependent, so Jade must not silently assume English for an
        // untagged or non-English result run.
        assert_eq!(
            resolved,
            [Some("21st"), Some("twenty-two"), Some("twenty-third")]
        );
    }

    #[test]
    fn seq_english_dollar_text_formats_the_integer_counter_as_hundredths() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Invoice \r 42 \* DollarText ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale dollar text</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .expect("SEQ field");

        // DollarText spells out the integral part and writes the first two
        // decimal places as a fraction over 100. A SEQ counter is integral, so
        // its fractional component is always 00/100. The stale saved result
        // proves this value is recomputed from the live restart counter.
        assert_eq!(
            field.resolved_value.as_deref(),
            Some("forty-two and 00/100")
        );
    }

    #[test]
    fn seq_text_number_formats_apply_trailing_capitalization_switches() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \r 21 \* CardText \* Caps ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale caps result</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Invoice \r 42 \* DollarText \*Upper ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale upper result</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Chapter \r 23 \* OrdText \* FirstCap ">
                <w:r>
                  <w:rPr><w:lang w:val="en-US"/></w:rPr>
                  <w:t>stale first-cap result</w:t>
                </w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Word evaluates the numeric presentation first, then applies the
        // trailing capitalization format. This order matters because the raw
        // counters contain no letters to capitalize. Cover both separated and
        // compact `\*Upper` syntax while keeping every saved result stale.
        assert_eq!(
            resolved,
            [
                Some("Twenty-One"),
                Some("FORTY-TWO AND 00/100"),
                Some("Twenty-third"),
            ]
        );
    }

    #[test]
    fn seq_hex_format_uses_the_numeric_counter_before_display_conversion() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \r 26 \* Hex ">
                <w:r><w:t>stale first hex value</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure \*He</w:instrText></w:r>
              <w:r><w:instrText>x </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale second hex value</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* ARABIC ">
                <w:r><w:t>stale arabic value</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Display conversion must not replace the stored counter: after 26
        // and 27 are rendered as 1A and 1B, switching back to Arabic must
        // continue the same sequence at 28. The complex instruction splits
        // the format name itself across `w:instrText` runs, as Word commonly
        // does after editing a field code.
        assert_eq!(resolved, [Some("1A"), Some("1B"), Some("28")]);
    }

    #[test]
    fn seq_restart_switch_sets_current_and_following_sequence_numbers() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>91</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure \r </w:instrText></w:r>
              <w:r><w:instrText>5 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>92</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ figure ">
                <w:r><w:t>93</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table ">
                <w:r><w:t>44</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Word's `\r 5` makes the restart field itself display 5. The next
        // case-insensitively matching identifier continues at 6, while an
        // unrelated sequence retains its independent counter. Cached results
        // are intentionally stale so the test exercises live recomputation.
        assert_eq!(resolved, [Some("1"), Some("5"), Some("6"), Some("1")]);
    }

    #[test]
    fn seq_repeat_switch_reuses_previous_number_without_incrementing() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>91</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure \C </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>92</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ figure ">
                <w:r><w:t>93</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table \c ">
                <w:r><w:t>saved table number</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table ">
                <w:r><w:t>44</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // `\c` repeats the closest preceding value without advancing it. A
        // repeat before the first value in its identifier has no authoritative
        // result, so Jade must retain the saved Word result instead of
        // inventing 0 or incrementing the sequence.
        assert_eq!(resolved, [Some("1"), Some("1"), Some("2"), None, Some("1")]);
    }

    #[test]
    fn seq_hidden_switch_suppresses_result_but_still_advances_counter() {
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure \H </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale visible number</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ figure ">
                <w:r><w:t>88</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table \h \* ARABIC ">
                <w:r><w:t>77</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Word hides the result of `\h` but still consumes that sequence
        // number, so the following field is 2. A general `\*` format switch
        // explicitly disables hiding and makes the Table result visible.
        assert_eq!(resolved, [Some(""), Some("2"), Some("1")]);
    }

    #[test]
    fn seq_hidden_without_result_region_still_advances_counter() {
        // `SEQ \h` consumes a number but has no visible result. Word may omit
        // the entire complex result region, or serialize an empty fldSimple.
        // Both shapes need a zero-width semantic run; otherwise the hidden
        // counter increment disappears and every later caption is off by one.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">ure \h </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ figure "><w:r><w:t>88</w:t></w:r></w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table \h "></w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Table "><w:r><w:t>77</w:t></w:r></w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraphs = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect::<Vec<_>>();
        let fields = paragraphs
            .iter()
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .find_map(|run| run.field.as_ref())
                    .expect("SEQ field")
            })
            .collect::<Vec<_>>();

        assert!(paragraphs[0].runs.iter().all(|run| run.text.is_empty()));
        assert!(paragraphs[2].runs.iter().all(|run| run.text.is_empty()));
        assert!(matches!(
            fields[0].kind,
            RunFieldKind::Seq {
                hide_result: true,
                ..
            }
        ));
        assert!(matches!(
            fields[2].kind,
            RunFieldKind::Seq {
                hide_result: true,
                ..
            }
        ));
        assert_eq!(
            fields
                .iter()
                .map(|field| field.resolved_value.as_deref())
                .collect::<Vec<_>>(),
            [Some(""), Some("2"), Some(""), Some("2")]
        );
    }

    #[test]
    fn seq_heading_switch_restarts_after_matching_heading_level() {
        let body = r#"
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter A</w:t></w:r></w:p>
            <w:p><w:fldSimple w:instr=" SEQ Figure \s 1 "><w:r><w:t>81</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" SEQ Figure \s 1 "><w:r><w:t>82</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
            <w:p><w:fldSimple w:instr=" SEQ Figure \s 1 "><w:r><w:t>83</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:pPr><w:pStyle w:val="heading 1"/></w:pPr><w:r><w:t>Chapter B</w:t></w:r></w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ fig</w:instrText></w:r>
              <w:r><w:instrText>ure \S </w:instrText></w:r>
              <w:r><w:instrText>1 </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>84</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>85</w:t></w:r></w:fldSimple></w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // `\s 1` restarts after each Heading 1. Heading 2 must not reset the
        // Figure counter, and a following plain SEQ continues from the value
        // established by the restarted field. Field codes are case-insensitive
        // and may be fragmented across instruction runs.
        assert_eq!(
            resolved,
            [Some("1"), Some("2"), Some("3"), Some("1"), Some("2")]
        );
    }

    #[test]
    fn seq_bookmark_operand_reuses_nearest_sequence_without_incrementing() {
        let body = r#"
            <w:p><w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>71</w:t></w:r></w:fldSimple></w:p>
            <w:p><w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>72</w:t></w:r></w:fldSimple></w:p>
            <w:p>
              <w:bookmarkStart w:id="7" w:name="FigureAnchor"/>
              <w:r><w:t>anchor after figure two</w:t></w:r>
              <w:bookmarkEnd w:id="7"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure Figure</w:instrText></w:r>
              <w:r><w:instrText>Anchor </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>73</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:fldSimple w:instr=" SEQ figure "><w:r><w:t>74</w:t></w:r></w:fldSimple></w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Word treats the optional second operand as a bookmark lookup. The
        // bookmark paragraph contains no field, so lookup must search outward
        // and reuse the nearest Figure SEQ (2), not its ordinary anchor text.
        // The reference does not advance the live counter, making the next
        // field 3. Cached results are stale and the instruction is split.
        assert_eq!(resolved, [Some("1"), Some("2"), Some("2"), Some("3")]);
    }

    #[test]
    fn seq_bookmark_operand_applies_its_own_number_format() {
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \r 4 \* ROMAN ">
                <w:r><w:t>stale target</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:bookmarkStart w:id="9" w:name="FigureFour"/>
              <w:r><w:t>anchor after figure four</w:t></w:r>
              <w:bookmarkEnd w:id="9"/>
            </w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SEQ Fig</w:instrText></w:r>
              <w:r><w:instrText>ure FigureFour \* Card</w:instrText></w:r>
              <w:r><w:instrText>Text \* Caps </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r>
                <w:rPr><w:lang w:val="en-US"/></w:rPr>
                <w:t>stale reference</w:t>
              </w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure \* ARABIC ">
                <w:r><w:t>stale continuation</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // A bookmark operand selects the target sequence number, not the
        // target field's already-formatted text. The reference therefore
        // reformats numeric four as CardText and then applies Caps, while
        // leaving the target's uppercase Roman result intact and not advancing
        // the following ordinary sequence field. Both the numeric and case
        // format names cross instruction-run boundaries.
        assert_eq!(resolved, [Some("IV"), Some("Four"), Some("5")]);
    }

    #[test]
    fn seq_bookmark_operand_uses_first_matching_field_in_following_element() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="10" w:name="BeforeFigures"/>
              <w:r><w:t>anchor before two figures</w:t></w:r>
              <w:bookmarkEnd w:id="10"/>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>stale first target</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t> and </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>stale second target</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure BeforeFigures ">
                <w:r><w:t>stale bookmark reference</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .flat_map(|paragraph| paragraph.runs.iter())
            .filter_map(|run| run.field.as_ref())
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Both ordinary fields share the first top-level element following
        // the bookmark. Document-order proximity makes Figure 1 the nearest;
        // selecting the last matching field in that element would incorrectly
        // copy Figure 2 merely because both have the same element distance.
        assert_eq!(resolved, [Some("1"), Some("2"), Some("1")]);
    }

    #[test]
    fn seq_bookmark_operand_uses_nearest_field_in_the_same_paragraph() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="11" w:name="InlineAnchor"/>
              <w:bookmarkEnd w:id="11"/>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>stale first inline target</w:t></w:r>
              </w:fldSimple>
              <w:r><w:t> and </w:t></w:r>
              <w:fldSimple w:instr=" SEQ Figure ">
                <w:r><w:t>stale second inline target</w:t></w:r>
              </w:fldSimple>
            </w:p>
            <w:p>
              <w:fldSimple w:instr=" SEQ Figure InlineAnchor ">
                <w:r><w:t>stale inline bookmark reference</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let resolved = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .flat_map(|paragraph| paragraph.runs.iter())
            .filter_map(|run| run.field.as_ref())
            .map(|field| field.resolved_value.as_deref())
            .collect::<Vec<_>>();

        // Computed field results are zero-width in the paragraph's plain-text
        // offsets, so run boundaries are required to distinguish these two
        // candidates. The first SEQ immediately after the collapsed bookmark
        // is nearer than the second SEQ later in the same paragraph.
        assert_eq!(resolved, [Some("1"), Some("2"), Some("1")]);
    }

    #[test]
    fn pageref_without_bookmark_name_is_unsupported() {
        // A `PAGEREF` with no bookmark argument cannot be resolved, so it is
        // retained as a cached field rather than guessed or flattened.
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" PAGEREF ">
                <w:r><w:t>1</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let run = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.first())
            .expect("run");
        let field = run.field.as_ref().expect("cached malformed PAGEREF field");
        assert_eq!(
            field.kind,
            RunFieldKind::Unsupported {
                instruction: "PAGEREF".to_string(),
            }
        );
        assert_eq!(field.cached_text.as_deref(), Some("1"));
        assert!(run.text.is_empty());
    }

    #[test]
    fn parses_date_field_with_picture_switch() {
        // A complex `DATE` field with a quoted `\@` picture; the format is
        // captured and the result run text is suppressed into cached_text.
        let body = r#"
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> DATE \@ "yyyy/MM/dd" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>2026/05/30</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let paragraph = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .expect("paragraph");
        let field = paragraph
            .runs
            .iter()
            .find_map(|r| r.field.as_ref())
            .expect("DATE field");
        assert_eq!(
            field.kind,
            RunFieldKind::Date {
                format: Some("yyyy/MM/dd".to_string()),
            },
        );
        assert_eq!(field.cached_text.as_deref(), Some("2026/05/30"));
    }

    #[test]
    fn parses_package_backed_date_fields_without_collapsing_them_to_date() {
        // These fields share DATE's picture grammar, but Word reads their
        // values from package metadata rather than the host clock.
        let body = r#"
            <w:p><w:fldSimple w:instr=' CREATEDATE \@ "yyyy-MM-dd" '><w:r><w:t>stale-created</w:t></w:r></w:fldSimple></w:p>
            <w:p>
              <w:r><w:fldChar w:fldCharType="begin"/></w:r>
              <w:r><w:instrText xml:space="preserve"> SAVE</w:instrText></w:r>
              <w:r><w:instrText xml:space="preserve">DATE \@ "yyyy-</w:instrText></w:r>
              <w:r><w:instrText>MM-dd" </w:instrText></w:r>
              <w:r><w:fldChar w:fldCharType="separate"/></w:r>
              <w:r><w:t>stale-saved</w:t></w:r>
              <w:r><w:fldChar w:fldCharType="end"/></w:r>
            </w:p>
            <w:p><w:fldSimple w:instr=' PRINTDATE \@ "yyyy-MM-dd" '><w:r><w:t>stale-printed</w:t></w:r></w:fldSimple></w:p>
        "#;
        let doc = parse_document_from_body(body);
        let fields = doc
            .body
            .as_ref()
            .expect("body")
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .filter_map(|paragraph| paragraph.runs.iter().find_map(|run| run.field.as_ref()))
            .map(|field| field.kind.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            vec![
                RunFieldKind::DocumentDate {
                    source: DocumentDateSource::Created,
                    format: Some("yyyy-MM-dd".to_string()),
                },
                RunFieldKind::DocumentDate {
                    source: DocumentDateSource::Saved,
                    format: Some("yyyy-MM-dd".to_string()),
                },
                RunFieldKind::DocumentDate {
                    source: DocumentDateSource::Printed,
                    format: Some("yyyy-MM-dd".to_string()),
                },
            ]
        );
    }

    #[test]
    fn parses_time_field_without_switch_from_fld_simple() {
        // A `TIME` field without `\@` leaves the format unset (renderer uses a
        // default time picture).
        let body = r#"
            <w:p>
              <w:fldSimple w:instr=" TIME ">
                <w:r><w:t>3:45 PM</w:t></w:r>
              </w:fldSimple>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        let field = doc
            .body
            .as_ref()
            .and_then(|b| b.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|p| p.runs.iter().find_map(|r| r.field.as_ref()))
            .expect("TIME field");
        assert_eq!(field.kind, RunFieldKind::Time { format: None });
    }

    #[test]
    fn extract_field_format_switch_handles_quoted_unquoted_and_missing() {
        assert_eq!(
            extract_field_format_switch(r#" DATE \@ "dddd, MMMM d, yyyy" "#),
            Some("dddd, MMMM d, yyyy".to_string()),
        );
        assert_eq!(
            extract_field_format_switch(r#" DATE \@ yyyy "#),
            Some("yyyy".to_string()),
        );
        assert_eq!(extract_field_format_switch(" DATE "), None);
        assert_eq!(extract_field_format_switch(r#" DATE \@ "" "#), None);
    }

    #[test]
    fn parses_bookmark_within_single_paragraph() {
        let body = r#"
            <w:p>
              <w:r><w:t>hello</w:t></w:r>
              <w:bookmarkStart w:id="0" w:name="greeting"/>
              <w:r><w:t>world</w:t></w:r>
              <w:bookmarkEnd w:id="0"/>
              <w:r><w:t>!</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        assert_eq!(doc.bookmarks.len(), 1, "bookmark should be captured");
        let bookmark = &doc.bookmarks[0];
        assert_eq!(bookmark.id, "0");
        assert_eq!(bookmark.name, "greeting");
        assert_eq!(bookmark.start_paragraph, 0);
        assert_eq!(bookmark.end_paragraph, 0);
        assert_eq!(bookmark.start_offset, 5, "start is after 'hello' (5 bytes)");
        assert_eq!(
            bookmark.end_offset, 10,
            "end is after 'helloworld' (10 bytes)"
        );
    }

    #[test]
    fn parses_bookmark_spanning_multiple_paragraphs() {
        let body = r#"
            <w:p>
              <w:r><w:t>alpha</w:t></w:r>
              <w:bookmarkStart w:id="1" w:name="multi"/>
              <w:r><w:t>beta</w:t></w:r>
            </w:p>
            <w:p>
              <w:r><w:t>gamma</w:t></w:r>
              <w:bookmarkEnd w:id="1"/>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        assert_eq!(doc.bookmarks.len(), 1);
        let bookmark = &doc.bookmarks[0];
        assert_eq!(bookmark.name, "multi");
        assert_eq!(bookmark.start_paragraph, 0);
        assert_eq!(bookmark.start_offset, 5, "after 'alpha'");
        assert_eq!(bookmark.end_paragraph, 1);
        assert_eq!(bookmark.end_offset, 5, "after 'gamma' in paragraph 2");
    }

    #[test]
    fn tolerates_bookmark_without_name_attribute() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="5"/>
              <w:r><w:t>x</w:t></w:r>
              <w:bookmarkEnd w:id="5"/>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        assert_eq!(doc.bookmarks.len(), 1);
        assert_eq!(doc.bookmarks[0].id, "5");
        assert_eq!(doc.bookmarks[0].name, "", "missing name becomes empty");
    }

    #[test]
    fn drops_bookmark_start_without_id() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:name="nope"/>
              <w:r><w:t>x</w:t></w:r>
              <w:bookmarkEnd w:id="0"/>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        assert!(
            doc.bookmarks.is_empty(),
            "bookmarkStart without w:id is malformed and must be dropped"
        );
    }

    #[test]
    fn drops_unclosed_bookmark_at_eof() {
        let body = r#"
            <w:p>
              <w:bookmarkStart w:id="7" w:name="dangling"/>
              <w:r><w:t>x</w:t></w:r>
            </w:p>
        "#;
        let doc = parse_document_from_body(body);
        assert!(
            doc.bookmarks.is_empty(),
            "unterminated bookmarks are not emitted — avoids exposing a half-open range that would\n             confuse consumers (hyperlink target resolution, outline widgets)"
        );
    }

    #[test]
    fn title_page_explicit_false_disables_first_page_variant() {
        let body = r#"
            <w:p/>
            <w:sectPr>
              <w:headerReference r:id="rIdH1" w:type="default"/>
              <w:titlePg w:val="false"/>
            </w:sectPr>
        "#;
        let doc = parse_document_from_body(body);
        let section = doc.default_section.as_ref().expect("default section");
        assert!(!section.title_page);
    }
}
