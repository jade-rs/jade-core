use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::{BytesCData, BytesStart, BytesText, Event};
use similar::{Algorithm, ChangeTag};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Cursor, Read};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zip::ZipArchive;

const DOCUMENT_XML_PATH: &str = "word/document.xml";
const DEFAULT_W_NAMESPACE: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const DEFAULT_R_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComparisonStats {
    pub matched: usize,
    pub added: usize,
    pub deleted: usize,
}

#[derive(Debug, Error)]
pub enum CompareError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("xml parse error: {0}")]
    Xml(String),
    #[error("utf-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("missing `{path}` in {side} document")]
    MissingPart {
        side: &'static str,
        path: &'static str,
    },
    #[error("docx write error: {0}")]
    Write(#[from] jade_docx_writer::DocxWriteError),
}

pub type Result<T> = std::result::Result<T, CompareError>;

#[derive(Debug, Clone)]
struct ParsedDocument {
    root_attrs: BTreeMap<String, String>,
    blocks: Vec<DocumentBlock>,
}

#[derive(Debug, Clone)]
enum DocumentBlock {
    Paragraph(ParagraphBlock),
    Table(TableBlock),
}

#[derive(Debug, Clone)]
struct ParagraphBlock {
    para_id: Option<String>,
    text: String,
    lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct TableBlock {
    para_ids: Vec<String>,
    text: String,
    xml: String,
}

#[derive(Debug, Clone)]
enum ComparableUnit {
    Paragraph {
        para_id: Option<String>,
        text: String,
        line_count: usize,
    },
    Table {
        keys: Vec<String>,
        xml: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum AlignmentOp {
    Match { new_index: usize, old_index: usize },
    Add { new_index: usize },
    Delete { old_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextStyle {
    Normal,
    Added,
    Deleted,
}

#[derive(Debug, Clone)]
struct StyledSegment {
    text: String,
    style: TextStyle,
}

#[derive(Debug, Clone)]
struct RenderedContent {
    xml: String,
    trailing_spacing_twips: usize,
}

#[derive(Debug, Clone)]
struct ComparisonRow {
    left_xml: String,
    right_xml: String,
    note: String,
    min_height_twips: Option<usize>,
}

#[derive(Debug, Clone)]
struct ParsedTable {
    tbl_pr_xml: String,
    tbl_grid_xml: String,
    rows: Vec<ParsedTableRow>,
}

#[derive(Debug, Clone)]
struct ParsedTableRow {
    tr_pr_xml: String,
    cells: Vec<ParsedTableCell>,
}

#[derive(Debug, Clone)]
struct ParsedTableCell {
    tc_pr_xml: String,
    blocks: Vec<ParsedTableCellBlock>,
}

#[derive(Debug, Clone)]
struct ParsedParagraph {
    para_id: Option<String>,
    xml: String,
    text: String,
}

#[derive(Debug, Clone)]
struct ParsedNestedTable {
    xml: String,
    text: String,
    parsed: Option<Box<ParsedTable>>,
}

#[derive(Debug, Clone)]
enum ParsedTableCellBlock {
    Paragraph(ParsedParagraph),
    Table(ParsedNestedTable),
}

struct CaptureContext {
    kind: CaptureKind,
    depth: usize,
    writer: Writer<Vec<u8>>,
    active_text: bool,
    para_id: Option<String>,
    table_para_ids: Vec<String>,
    table_text: String,
    lines: Vec<String>,
    current_line: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Paragraph,
    Table,
}

impl CaptureContext {
    fn new(kind: CaptureKind) -> Self {
        Self {
            kind,
            depth: 1,
            writer: Writer::new(Vec::new()),
            active_text: false,
            para_id: None,
            table_para_ids: Vec::new(),
            table_text: String::new(),
            lines: Vec::new(),
            current_line: String::new(),
        }
    }

    fn write_event<'a>(&mut self, event: Event<'a>) -> std::result::Result<(), CompareError> {
        self.writer
            .write_event(event)
            .map_err(|e| CompareError::Xml(format!("failed to write captured xml: {e}")))
    }

    fn on_start(&mut self, event: &BytesStart<'_>) {
        let binding = event.name();
        let name = local_name(binding.as_ref());
        match self.kind {
            CaptureKind::Paragraph => {
                if name == b"p" && self.depth == 1 {
                    self.para_id = attr_value_local(event, b"paraId");
                }
                if is_text_element(name) {
                    self.active_text = true;
                }
                if name == b"tab" {
                    self.current_line.push('\t');
                }
                if is_line_break_element(name) {
                    self.push_line_break();
                }
            }
            CaptureKind::Table => {
                if name == b"p"
                    && let Some(para_id) = attr_value_local(event, b"paraId")
                    && !self
                        .table_para_ids
                        .iter()
                        .any(|existing| existing == &para_id)
                {
                    self.table_para_ids.push(para_id);
                }
                if name == b"p" && !self.table_text.is_empty() {
                    self.table_text.push('\n');
                }
                if is_text_element(name) {
                    self.active_text = true;
                }
                if name == b"tab" {
                    self.table_text.push('\t');
                }
                if is_line_break_element(name) {
                    self.table_text.push('\n');
                }
            }
        }
    }

    fn on_empty(&mut self, event: &BytesStart<'_>) {
        let binding = event.name();
        let name = local_name(binding.as_ref());
        match self.kind {
            CaptureKind::Paragraph => {
                if is_text_element(name) {
                    self.active_text = false;
                }
                if name == b"tab" {
                    self.current_line.push('\t');
                }
                if is_line_break_element(name) {
                    self.push_line_break();
                }
            }
            CaptureKind::Table => {
                if name == b"p"
                    && let Some(para_id) = attr_value_local(event, b"paraId")
                    && !self
                        .table_para_ids
                        .iter()
                        .any(|existing| existing == &para_id)
                {
                    self.table_para_ids.push(para_id);
                }
                if name == b"p" && !self.table_text.is_empty() {
                    self.table_text.push('\n');
                }
                if is_text_element(name) {
                    self.active_text = false;
                }
                if name == b"tab" {
                    self.table_text.push('\t');
                }
                if is_line_break_element(name) {
                    self.table_text.push('\n');
                }
            }
        }
    }

    fn on_end(&mut self, name: &[u8]) {
        if is_text_element(local_name(name)) {
            self.active_text = false;
        }
    }

    fn on_text(&mut self, event: &BytesText<'_>) -> std::result::Result<(), CompareError> {
        if !self.active_text {
            return Ok(());
        }

        let decoded = event
            .decode()
            .map_err(|e| CompareError::Xml(format!("text decode failed: {e}")))?;
        let content = unescape(decoded.as_ref())
            .map_err(|e| CompareError::Xml(format!("text unescape failed: {e}")))?;
        match self.kind {
            CaptureKind::Paragraph => self.current_line.push_str(content.as_ref()),
            CaptureKind::Table => self.table_text.push_str(content.as_ref()),
        }
        Ok(())
    }

    fn on_cdata(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesCData<'_>,
    ) -> std::result::Result<(), CompareError> {
        if !self.active_text {
            return Ok(());
        }

        let content = reader
            .decoder()
            .decode(event.as_ref())
            .map_err(|e| CompareError::Xml(format!("cdata decode failed: {e}")))?;
        match self.kind {
            CaptureKind::Paragraph => self.current_line.push_str(content.as_ref()),
            CaptureKind::Table => self.table_text.push_str(content.as_ref()),
        }
        Ok(())
    }

    fn push_line_break(&mut self) {
        self.lines.push(std::mem::take(&mut self.current_line));
    }

    fn finish(mut self) -> Result<FinishedCapture> {
        if self.kind == CaptureKind::Paragraph {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
        let xml = String::from_utf8(self.writer.into_inner())?;
        Ok(FinishedCapture {
            kind: self.kind,
            xml,
            para_id: self.para_id,
            table_para_ids: self.table_para_ids,
            table_text: self.table_text,
            lines: self.lines,
        })
    }
}

#[derive(Debug)]
struct FinishedCapture {
    kind: CaptureKind,
    xml: String,
    para_id: Option<String>,
    table_para_ids: Vec<String>,
    table_text: String,
    lines: Vec<String>,
}

impl ParsedTableCell {
    fn text(&self) -> String {
        self.blocks
            .iter()
            .map(ParsedTableCellBlock::text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn raw_content_xml(&self) -> String {
        let mut xml = String::new();
        for block in &self.blocks {
            xml.push_str(&block.raw_xml());
        }
        if xml.trim().is_empty() {
            "<w:p/>".to_string()
        } else {
            xml
        }
    }
}

impl ParsedTableRow {
    fn text(&self) -> String {
        self.cells
            .iter()
            .map(ParsedTableCell::text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\t")
    }
}

impl ParsedTableCellBlock {
    fn text(&self) -> String {
        match self {
            Self::Paragraph(paragraph) => paragraph.text.clone(),
            Self::Table(table) => table.text.clone(),
        }
    }

    fn raw_xml(&self) -> String {
        match self {
            Self::Paragraph(paragraph) => paragraph.xml.clone(),
            Self::Table(table) => table.xml.clone(),
        }
    }

    fn match_keys(&self, block_index: usize) -> Vec<String> {
        match self {
            Self::Paragraph(paragraph) => {
                paragraph_match_keys(paragraph.para_id.as_ref(), &paragraph.text)
            }
            Self::Table(table) => table_match_keys_from_text(&table.text, block_index),
        }
    }
}

pub fn build_comparison_docx(new_docx: &[u8], old_docx: &[u8]) -> Result<Vec<u8>> {
    let (bytes, _) = build_comparison_docx_with_stats(new_docx, old_docx)?;
    Ok(bytes)
}

pub fn build_comparison_docx_with_stats(
    new_docx: &[u8],
    old_docx: &[u8],
) -> Result<(Vec<u8>, ComparisonStats)> {
    let new_files = read_zip_entries(new_docx)?;
    let old_files = read_zip_entries(old_docx)?;

    let new_document_xml = read_document_xml(&new_files, "new")?;
    let old_document_xml = read_document_xml(&old_files, "old")?;

    let parsed_new = parse_document_xml(&new_document_xml)?;
    let parsed_old = parse_document_xml(&old_document_xml)?;

    let new_units = units_from_blocks(&parsed_new.blocks);
    let old_units = units_from_blocks(&parsed_old.blocks);
    let alignment = align_units(&new_units, &old_units);

    let mut stats = ComparisonStats::default();
    let mut rows: Vec<ComparisonRow> = Vec::new();

    for op in alignment {
        match op {
            AlignmentOp::Match {
                new_index,
                old_index,
            } => {
                stats.matched += 1;
                rows.push(render_match_row(
                    &new_units[new_index],
                    &old_units[old_index],
                ));
            }
            AlignmentOp::Add { new_index } => {
                stats.added += 1;
                rows.push(render_add_row(&new_units[new_index]));
            }
            AlignmentOp::Delete { old_index } => {
                stats.deleted += 1;
                rows.push(render_delete_row(&old_units[old_index]));
            }
        }
    }

    let root_attrs = merge_root_attributes(&parsed_new.root_attrs, &parsed_old.root_attrs);
    let sect_pr = comparison_sect_pr_xml();

    let table_xml = build_comparison_table_xml(&rows);
    let output_document_xml = build_output_document_xml(&root_attrs, &table_xml, &sect_pr);

    let mut overrides = HashMap::new();
    overrides.insert(
        DOCUMENT_XML_PATH.to_string(),
        output_document_xml.into_bytes(),
    );

    let output = jade_docx_writer::write_docx(&new_files, &overrides)?;
    Ok((output, stats))
}

fn read_zip_entries(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut files = HashMap::new();

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        files.insert(file.name()?.into_owned(), contents);
    }

    Ok(files)
}

fn read_document_xml(files: &HashMap<String, Vec<u8>>, side: &'static str) -> Result<String> {
    let data = files
        .get(DOCUMENT_XML_PATH)
        .ok_or(CompareError::MissingPart {
            side,
            path: DOCUMENT_XML_PATH,
        })?;

    String::from_utf8(data.clone()).map_err(CompareError::Utf8)
}

fn parse_document_xml(xml: &str) -> Result<ParsedDocument> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut root_attrs = BTreeMap::new();
    let mut blocks = Vec::new();
    let mut capture: Option<CaptureContext> = None;

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| CompareError::Xml(format!("failed to parse document.xml: {e}")))?
        {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();

                if stack.is_empty() && local_name(&name) == b"document" {
                    root_attrs = collect_attributes(&reader, &event)?;
                }

                if let Some(active) = capture.as_mut() {
                    active.on_start(&event);
                    active.write_event(Event::Start(event.to_owned()))?;
                    active.depth += 1;
                } else if is_direct_child_of_body(&stack)
                    && let Some(kind) = capture_kind_for_name(&name)
                {
                    let mut context = CaptureContext::new(kind);
                    context.on_start(&event);
                    context.write_event(Event::Start(event.to_owned()))?;
                    capture = Some(context);
                }

                stack.push(name);
            }
            Event::Empty(event) => {
                let name = event.name().as_ref().to_vec();

                if let Some(active) = capture.as_mut() {
                    active.on_empty(&event);
                    active.write_event(Event::Empty(event.to_owned()))?;
                } else if is_direct_child_of_body(&stack)
                    && let Some(kind) = capture_kind_for_name(&name)
                {
                    let mut context = CaptureContext::new(kind);
                    context.on_empty(&event);
                    context.write_event(Event::Empty(event.to_owned()))?;
                    let finished = context.finish()?;
                    consume_finished_capture(finished, &mut blocks);
                }
            }
            Event::Text(event) => {
                if let Some(active) = capture.as_mut() {
                    active.on_text(&event)?;
                    active.write_event(Event::Text(event.to_owned()))?;
                }
            }
            Event::CData(event) => {
                if let Some(active) = capture.as_mut() {
                    active.on_cdata(&reader, &event)?;
                    active.write_event(Event::CData(event.to_owned()))?;
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                if let Some(active) = capture.as_mut() {
                    active.on_end(&name);
                    active.write_event(Event::End(event.to_owned()))?;
                    active.depth = active.depth.saturating_sub(1);
                    if active.depth == 0 {
                        let finished = capture
                            .take()
                            .expect("capture should exist when finishing")
                            .finish()?;
                        consume_finished_capture(finished, &mut blocks);
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            Event::Decl(event) => {
                if let Some(active) = capture.as_mut() {
                    active.write_event(Event::Decl(event.to_owned()))?;
                }
            }
            Event::PI(event) => {
                if let Some(active) = capture.as_mut() {
                    active.write_event(Event::PI(event.to_owned()))?;
                }
            }
            Event::DocType(event) => {
                if let Some(active) = capture.as_mut() {
                    active.write_event(Event::DocType(event.to_owned()))?;
                }
            }
            Event::Comment(event) => {
                if let Some(active) = capture.as_mut() {
                    active.write_event(Event::Comment(event.to_owned()))?;
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(ParsedDocument { root_attrs, blocks })
}

fn consume_finished_capture(finished: FinishedCapture, blocks: &mut Vec<DocumentBlock>) {
    match finished.kind {
        CaptureKind::Paragraph => {
            blocks.push(DocumentBlock::Paragraph(ParagraphBlock {
                para_id: finished.para_id,
                text: join_lines_for_text(&finished.lines),
                lines: normalize_lines(finished.lines),
            }));
        }
        CaptureKind::Table => {
            blocks.push(DocumentBlock::Table(TableBlock {
                para_ids: finished.table_para_ids,
                text: finished.table_text,
                xml: finished.xml,
            }));
        }
    }
}

fn normalize_lines(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        return vec![String::new()];
    }
    lines
}

fn join_lines_for_text(lines: &[String]) -> String {
    lines.join("\n")
}

fn normalize_match_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut last_was_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() || ch == '\u{3000}' {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn digits_to_zero(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_digit() {
                '0'
            } else if ('０'..='９').contains(&ch) {
                '０'
            } else {
                ch
            }
        })
        .collect()
}

fn remove_all_spaces(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\u{3000}')
        .collect()
}

fn paragraph_match_variants(text: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let normalized_space = normalize_match_text(text);
    let nfc = text.nfc().collect::<String>();
    let nfkc = text.nfkc().collect::<String>();
    let normalized_nfc = normalize_match_text(&nfc);
    let normalized_nfkc = normalize_match_text(&nfkc);
    let candidates = vec![
        text.to_string(),
        normalized_space.clone(),
        remove_all_spaces(text),
        nfc.clone(),
        nfkc.clone(),
        normalized_nfc.clone(),
        normalized_nfkc.clone(),
        remove_all_spaces(&nfc),
        remove_all_spaces(&nfkc),
        remove_all_spaces(&normalized_space),
        remove_all_spaces(&normalized_nfc),
        remove_all_spaces(&normalized_nfkc),
    ];

    for candidate in candidates {
        if candidate.is_empty() || variants.iter().any(|existing| existing == &candidate) {
            continue;
        }
        variants.push(candidate.clone());

        let digits_zero = digits_to_zero(&candidate);
        if !digits_zero.is_empty() && !variants.iter().any(|existing| existing == &digits_zero) {
            variants.push(digits_zero);
        }
    }

    variants
}

fn paragraph_match_keys(para_id: Option<&String>, text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(para_id) = para_id {
        keys.push(format!("p:{para_id}"));
    }
    for variant in paragraph_match_variants(text) {
        keys.push(format!("p:text:{variant}"));
    }
    keys
}

fn table_text_signature(text: &str) -> Option<String> {
    let raw_signature = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(text);
    let normalized = normalize_match_text(raw_signature);
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.chars().take(32).collect())
}

fn digit_normalized_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_digit() || ('０'..='９').contains(&ch) {
                '#'
            } else {
                ch
            }
        })
        .collect()
}

fn table_match_keys(table: &TableBlock, block_index: usize) -> Vec<String> {
    let mut keys = table_match_keys_from_text(&table.text, block_index);
    if let Some(first_para_id) = table.para_ids.first() {
        keys.push(format!("t:{first_para_id}"));
    }
    if keys.is_empty() {
        keys.push(format!("t:no-table-id-{block_index}"));
    }
    keys
}

fn table_match_keys_from_text(text: &str, _block_index: usize) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(signature) = table_text_signature(text) {
        for variant in paragraph_match_variants(&signature) {
            keys.push(format!("t:sig:{variant}"));
            let digit_normalized = digit_normalized_text(&variant);
            if digit_normalized != variant {
                keys.push(format!("t:sig-digits:{digit_normalized}"));
            }
        }
    }
    keys
}

fn collect_attributes(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
) -> Result<BTreeMap<String, String>> {
    let mut attrs = BTreeMap::new();
    for attr in element.attributes() {
        let attr =
            attr.map_err(|e| CompareError::Xml(format!("failed to parse document attrs: {e}")))?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let value = attr
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|e| CompareError::Xml(format!("failed to decode document attrs: {e}")))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok(attrs)
}

fn capture_kind_for_name(name: &[u8]) -> Option<CaptureKind> {
    match local_name(name) {
        b"p" => Some(CaptureKind::Paragraph),
        b"tbl" => Some(CaptureKind::Table),
        _ => None,
    }
}

fn is_direct_child_of_body(stack: &[Vec<u8>]) -> bool {
    stack
        .last()
        .is_some_and(|name| local_name(name.as_slice()) == b"body")
}

fn units_from_blocks(blocks: &[DocumentBlock]) -> Vec<ComparableUnit> {
    let mut units = Vec::new();

    for (block_index, block) in blocks.iter().enumerate() {
        match block {
            DocumentBlock::Paragraph(paragraph) => {
                let _ = block_index;
                units.push(ComparableUnit::Paragraph {
                    para_id: paragraph.para_id.clone(),
                    text: paragraph.text.clone(),
                    line_count: paragraph.lines.len().max(1),
                });
            }
            DocumentBlock::Table(table) => {
                units.push(ComparableUnit::Table {
                    keys: table_match_keys(table, block_index),
                    xml: table.xml.clone(),
                });
            }
        }
    }

    units
}

fn align_units(new_units: &[ComparableUnit], old_units: &[ComparableUnit]) -> Vec<AlignmentOp> {
    let new_keys = new_units
        .iter()
        .map(ComparableUnit::match_keys)
        .collect::<Vec<_>>();
    let old_keys = old_units
        .iter()
        .map(ComparableUnit::match_keys)
        .collect::<Vec<_>>();
    align_match_keys(&new_keys, &old_keys)
}

fn align_match_keys(new_keys: &[Vec<String>], old_keys: &[Vec<String>]) -> Vec<AlignmentOp> {
    let mut old_positions: HashMap<String, VecDeque<usize>> = HashMap::new();
    for (index, keys) in old_keys.iter().enumerate() {
        for key in keys {
            old_positions
                .entry(key.clone())
                .or_default()
                .push_back(index);
        }
    }

    let mut last_old_index: Option<usize> = None;
    let mut matches = Vec::new();
    let mut used_old_indices = vec![false; old_keys.len()];

    for (new_index, keys) in new_keys.iter().enumerate() {
        let mut matched_old_index = None;

        for key in keys {
            let Some(queue) = old_positions.get_mut(key.as_str()) else {
                continue;
            };

            if let Some(last) = last_old_index {
                while let Some(front) = queue.front() {
                    if *front <= last || used_old_indices[*front] {
                        queue.pop_front();
                    } else {
                        break;
                    }
                }
            }

            while let Some(front) = queue.front() {
                if used_old_indices[*front] {
                    queue.pop_front();
                } else {
                    break;
                }
            }

            if let Some(old_index) = queue.pop_front() {
                matched_old_index = Some(old_index);
                break;
            }
        }

        if let Some(old_index) = matched_old_index {
            used_old_indices[old_index] = true;
            matches.push((new_index, old_index));
            last_old_index = Some(old_index);
        }
    }

    let mut operations = Vec::new();
    let mut new_cursor = 0;
    let mut old_cursor = 0;

    for (new_index, old_index) in matches {
        while new_cursor < new_index {
            operations.push(AlignmentOp::Add {
                new_index: new_cursor,
            });
            new_cursor += 1;
        }
        while old_cursor < old_index {
            operations.push(AlignmentOp::Delete {
                old_index: old_cursor,
            });
            old_cursor += 1;
        }

        operations.push(AlignmentOp::Match {
            new_index,
            old_index,
        });
        new_cursor = new_index + 1;
        old_cursor = old_index + 1;
    }

    while new_cursor < new_keys.len() {
        operations.push(AlignmentOp::Add {
            new_index: new_cursor,
        });
        new_cursor += 1;
    }

    while old_cursor < old_keys.len() {
        operations.push(AlignmentOp::Delete {
            old_index: old_cursor,
        });
        old_cursor += 1;
    }

    operations
}

impl ComparableUnit {
    fn match_keys(&self) -> Vec<String> {
        match self {
            ComparableUnit::Paragraph { para_id, text, .. } => {
                paragraph_match_keys(para_id.as_ref(), text)
            }
            ComparableUnit::Table { keys, .. } => keys.clone(),
        }
    }
}

fn render_match_row(new: &ComparableUnit, old: &ComparableUnit) -> ComparisonRow {
    match (new, old) {
        (
            ComparableUnit::Paragraph {
                text: new_text,
                line_count: new_line_count,
                ..
            },
            ComparableUnit::Paragraph {
                text: old_text,
                line_count: old_line_count,
                ..
            },
        ) => {
            let (left_segments, right_segments) = diff_text_segments(old_text, new_text);
            let note = String::new();
            ComparisonRow {
                left_xml: paragraph_xml_from_segments(&left_segments),
                right_xml: paragraph_xml_from_segments(&right_segments),
                note,
                min_height_twips: Some(row_min_height_twips(*new_line_count, *old_line_count)),
            }
        }
        (ComparableUnit::Table { xml: left, .. }, ComparableUnit::Table { xml: right, .. }) => {
            let note = String::new();
            ComparisonRow {
                left_xml: matched_table_cell_xml(left, right, TableRenderSide::New),
                right_xml: matched_table_cell_xml(left, right, TableRenderSide::Old),
                note,
                min_height_twips: None,
            }
        }
        _ => ComparisonRow {
            left_xml: "<w:p/>".to_string(),
            right_xml: "<w:p/>".to_string(),
            note: String::new(),
            min_height_twips: Some(row_min_height_twips(1, 1)),
        },
    }
}

fn render_add_row(new: &ComparableUnit) -> ComparisonRow {
    match new {
        ComparableUnit::Paragraph {
            text, line_count, ..
        } => {
            let segments = vec![StyledSegment {
                text: text.clone(),
                style: TextStyle::Added,
            }];
            ComparisonRow {
                left_xml: paragraph_xml_from_segments(&segments),
                right_xml: "<w:p/>".to_string(),
                note: String::new(),
                min_height_twips: Some(row_min_height_twips(*line_count, 1)),
            }
        }
        ComparableUnit::Table { xml, .. } => ComparisonRow {
            left_xml: table_cell_xml_from_table(xml),
            right_xml: "<w:p/>".to_string(),
            note: String::new(),
            min_height_twips: None,
        },
    }
}

fn render_delete_row(old: &ComparableUnit) -> ComparisonRow {
    match old {
        ComparableUnit::Paragraph {
            text, line_count, ..
        } => {
            let segments = vec![StyledSegment {
                text: text.clone(),
                style: TextStyle::Deleted,
            }];
            ComparisonRow {
                left_xml: "<w:p/>".to_string(),
                right_xml: paragraph_xml_from_segments(&segments),
                note: String::new(),
                min_height_twips: Some(row_min_height_twips(1, *line_count)),
            }
        }
        ComparableUnit::Table { xml, .. } => ComparisonRow {
            left_xml: "<w:p/>".to_string(),
            right_xml: table_cell_xml_from_table(xml),
            note: String::new(),
            min_height_twips: None,
        },
    }
}

fn diff_text_segments(old_text: &str, new_text: &str) -> (Vec<StyledSegment>, Vec<StyledSegment>) {
    let mut left = Vec::new();
    let mut right = Vec::new();

    for (tag, content) in similar::utils::diff_chars(Algorithm::Myers, old_text, new_text) {
        if content.is_empty() {
            continue;
        }

        let text = content.to_string();
        match tag {
            ChangeTag::Equal => {
                left.push(StyledSegment {
                    text: text.clone(),
                    style: TextStyle::Normal,
                });
                right.push(StyledSegment {
                    text,
                    style: TextStyle::Normal,
                });
            }
            ChangeTag::Insert => {
                left.push(StyledSegment {
                    text,
                    style: TextStyle::Added,
                });
            }
            ChangeTag::Delete => {
                right.push(StyledSegment {
                    text,
                    style: TextStyle::Deleted,
                });
            }
        }
    }

    if left.is_empty() {
        left.push(StyledSegment {
            text: String::new(),
            style: TextStyle::Normal,
        });
    }
    if right.is_empty() {
        right.push(StyledSegment {
            text: String::new(),
            style: TextStyle::Normal,
        });
    }

    (coalesce_segments(left), coalesce_segments(right))
}

fn coalesce_segments(segments: Vec<StyledSegment>) -> Vec<StyledSegment> {
    let mut merged: Vec<StyledSegment> = Vec::new();
    for segment in segments {
        if let Some(last) = merged.last_mut()
            && last.style == segment.style
        {
            last.text.push_str(&segment.text);
            continue;
        }
        merged.push(segment);
    }
    merged
}

fn paragraph_xml_from_segments(segments: &[StyledSegment]) -> String {
    if segments.iter().all(|segment| segment.text.is_empty()) {
        return "<w:p/>".to_string();
    }

    let mut xml = String::from("<w:p>");
    for segment in segments {
        append_segment_runs(&mut xml, segment);
    }
    xml.push_str("</w:p>");
    xml
}

fn append_segment_runs(xml: &mut String, segment: &StyledSegment) {
    let mut buffer = String::new();
    for ch in segment.text.chars() {
        match ch {
            '\t' => {
                flush_text_run(xml, segment.style, &mut buffer);
                xml.push_str("<w:r>");
                xml.push_str(run_properties_xml(segment.style));
                xml.push_str("<w:tab/></w:r>");
            }
            '\n' => {
                flush_text_run(xml, segment.style, &mut buffer);
                xml.push_str("<w:r>");
                xml.push_str(run_properties_xml(segment.style));
                xml.push_str("<w:br/></w:r>");
            }
            _ => buffer.push(ch),
        }
    }
    flush_text_run(xml, segment.style, &mut buffer);
}

fn flush_text_run(xml: &mut String, style: TextStyle, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }

    xml.push_str("<w:r>");
    xml.push_str(run_properties_xml(style));
    if should_preserve_space(buffer) {
        xml.push_str("<w:t xml:space=\"preserve\">");
    } else {
        xml.push_str("<w:t>");
    }
    xml.push_str(escape(buffer.as_str()).as_ref());
    xml.push_str("</w:t></w:r>");
    buffer.clear();
}

fn row_min_height_twips(left_line_count: usize, right_line_count: usize) -> usize {
    let max_lines = left_line_count.max(right_line_count).max(1);
    max_lines.saturating_mul(360)
}

fn table_row_xml(
    left_xml: &str,
    right_xml: &str,
    note: &str,
    min_height_twips: Option<usize>,
) -> String {
    let mut xml = String::new();
    xml.push_str("<w:tr>");
    if let Some(min_height_twips) = min_height_twips {
        xml.push_str(&format!(
            "<w:trPr><w:trHeight w:val=\"{min_height_twips}\" w:hRule=\"atLeast\"/></w:trPr>"
        ));
    }
    xml.push_str(&table_cell_xml_from_content(left_xml, 9800));
    xml.push_str(&table_cell_xml_from_content(right_xml, 9800));
    xml.push_str(&table_cell_xml_from_content(
        &note_paragraph_xml(note),
        1334,
    ));
    xml.push_str("</w:tr>");
    xml
}

fn table_row_with_note_xml(
    left_xml: &str,
    right_xml: &str,
    note_xml: &str,
    min_height_twips: usize,
) -> String {
    let mut xml = String::new();
    xml.push_str("<w:tr>");
    xml.push_str(&format!(
        "<w:trPr><w:trHeight w:val=\"{min_height_twips}\" w:hRule=\"atLeast\"/></w:trPr>"
    ));
    xml.push_str(&table_cell_xml_from_content(left_xml, 9800));
    xml.push_str(&table_cell_xml_from_content(right_xml, 9800));
    xml.push_str(&table_cell_xml_from_content(note_xml, 1334));
    xml.push_str("</w:tr>");
    xml
}

fn run_properties_xml(style: TextStyle) -> &'static str {
    match style {
        TextStyle::Normal => "",
        TextStyle::Added => "<w:rPr><w:color w:val=\"008000\"/></w:rPr>",
        TextStyle::Deleted => "<w:rPr><w:color w:val=\"C00000\"/><w:strike/></w:rPr>",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableRenderSide {
    New,
    Old,
}

fn matched_table_cell_xml(
    new_table_xml: &str,
    old_table_xml: &str,
    side: TableRenderSide,
) -> String {
    match (
        parse_table_for_diff(new_table_xml),
        parse_table_for_diff(old_table_xml),
    ) {
        (Some(new_table), Some(old_table)) => {
            let rendered = render_diff_table_with_spacing(&new_table, &old_table, side);
            table_cell_xml_from_rendered_table(&rendered)
        }
        _ => {
            let fallback = match side {
                TableRenderSide::New => new_table_xml,
                TableRenderSide::Old => old_table_xml,
            };
            table_cell_xml_from_table(fallback)
        }
    }
}

fn parse_table_for_diff(table_xml: &str) -> Option<ParsedTable> {
    // Fall back to raw table XML for tables that rely on non-text constructs
    // we still cannot preserve through synthetic diff rendering.
    if [
        "<w:fldChar",
        "<w:fldSimple",
        "<w:instrText",
        "<w:pict",
        "<w:drawing",
    ]
    .iter()
    .any(|pattern| table_xml.contains(pattern))
    {
        return None;
    }
    let mut reader = Reader::from_str(table_xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut tbl_pr_xml = String::new();
    let mut tbl_grid_xml = String::new();
    let mut rows = Vec::new();
    let mut current_row: Option<ParsedTableRow> = None;
    let mut current_cell: Option<ParsedTableCell> = None;
    let mut table_depth = 0usize;
    let mut cell_content_depth: Option<usize> = None;
    let mut child_capture: Option<CaptureContext> = None;
    let mut capture_tbl_pr: Option<CaptureContext> = None;
    let mut capture_tbl_grid: Option<CaptureContext> = None;
    let mut capture_tr_pr: Option<CaptureContext> = None;
    let mut capture_tc_pr: Option<CaptureContext> = None;

    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();
                let local = local_name(&name);

                if local == b"tbl" {
                    table_depth += 1;
                }

                if table_depth == 1
                    && child_capture.is_none()
                    && local == b"tblPr"
                    && capture_tbl_pr.is_none()
                {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    capture_tbl_pr = Some(ctx);
                } else if table_depth == 1
                    && child_capture.is_none()
                    && local == b"tblGrid"
                    && capture_tbl_grid.is_none()
                {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    capture_tbl_grid = Some(ctx);
                } else if table_depth == 1
                    && child_capture.is_none()
                    && local == b"trPr"
                    && capture_tr_pr.is_none()
                {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    capture_tr_pr = Some(ctx);
                } else if table_depth == 1
                    && child_capture.is_none()
                    && local == b"tcPr"
                    && capture_tc_pr.is_none()
                {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    capture_tc_pr = Some(ctx);
                } else {
                    if let Some(ctx) = capture_tbl_pr.as_mut() {
                        ctx.write_event(Event::Start(event.to_owned())).ok()?;
                        ctx.depth += 1;
                    }
                    if let Some(ctx) = capture_tbl_grid.as_mut() {
                        ctx.write_event(Event::Start(event.to_owned())).ok()?;
                        ctx.depth += 1;
                    }
                    if let Some(ctx) = capture_tr_pr.as_mut() {
                        ctx.write_event(Event::Start(event.to_owned())).ok()?;
                        ctx.depth += 1;
                    }
                    if let Some(ctx) = capture_tc_pr.as_mut() {
                        ctx.write_event(Event::Start(event.to_owned())).ok()?;
                        ctx.depth += 1;
                    }
                }

                if let Some(ctx) = child_capture.as_mut() {
                    ctx.on_start(&event);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    ctx.depth += 1;
                } else if current_cell.is_some()
                    && cell_content_depth == Some(0)
                    && matches!(local, b"p" | b"tbl")
                {
                    let kind = if local == b"p" {
                        CaptureKind::Paragraph
                    } else {
                        CaptureKind::Table
                    };
                    let mut ctx = CaptureContext::new(kind);
                    ctx.on_start(&event);
                    ctx.write_event(Event::Start(event.to_owned())).ok()?;
                    child_capture = Some(ctx);
                }

                if table_depth == 1 && child_capture.is_none() {
                    match local {
                        b"tr" => {
                            current_row = Some(ParsedTableRow {
                                tr_pr_xml: String::new(),
                                cells: Vec::new(),
                            });
                        }
                        b"tc" => {
                            current_cell = Some(ParsedTableCell {
                                tc_pr_xml: String::new(),
                                blocks: Vec::new(),
                            });
                            cell_content_depth = Some(0);
                        }
                        _ => {}
                    }
                }

                if current_cell.is_some()
                    && !(table_depth == 1 && local == b"tc" && child_capture.is_none())
                    && let Some(depth) = cell_content_depth.as_mut()
                {
                    *depth += 1;
                }
            }
            Event::Empty(event) => {
                let name = event.name().as_ref().to_vec();
                let local = local_name(&name);

                if table_depth == 1 && child_capture.is_none() && local == b"tblPr" {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    tbl_pr_xml = ctx.finish().ok()?.xml;
                } else if table_depth == 1 && child_capture.is_none() && local == b"tblGrid" {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    tbl_grid_xml = ctx.finish().ok()?.xml;
                } else if table_depth == 1 && child_capture.is_none() && local == b"trPr" {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    if let Some(row) = current_row.as_mut() {
                        row.tr_pr_xml = ctx.finish().ok()?.xml;
                    }
                } else if table_depth == 1 && child_capture.is_none() && local == b"tcPr" {
                    let mut ctx = CaptureContext::new(CaptureKind::Table);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    if let Some(cell) = current_cell.as_mut() {
                        cell.tc_pr_xml = ctx.finish().ok()?.xml;
                    }
                } else {
                    if let Some(ctx) = capture_tbl_pr.as_mut() {
                        ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    }
                    if let Some(ctx) = capture_tbl_grid.as_mut() {
                        ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    }
                    if let Some(ctx) = capture_tr_pr.as_mut() {
                        ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    }
                    if let Some(ctx) = capture_tc_pr.as_mut() {
                        ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    }
                }

                if let Some(ctx) = child_capture.as_mut() {
                    ctx.on_empty(&event);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                } else if current_cell.is_some()
                    && cell_content_depth == Some(0)
                    && matches!(local, b"p" | b"tbl")
                {
                    let kind = if local == b"p" {
                        CaptureKind::Paragraph
                    } else {
                        CaptureKind::Table
                    };
                    let mut ctx = CaptureContext::new(kind);
                    ctx.on_empty(&event);
                    ctx.write_event(Event::Empty(event.to_owned())).ok()?;
                    let finished = ctx.finish().ok()?;
                    push_finished_table_cell_block(current_cell.as_mut()?, finished);
                }
            }
            Event::Text(event) => {
                if let Some(ctx) = capture_tbl_pr.as_mut() {
                    ctx.write_event(Event::Text(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tbl_grid.as_mut() {
                    ctx.write_event(Event::Text(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tr_pr.as_mut() {
                    ctx.write_event(Event::Text(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tc_pr.as_mut() {
                    ctx.write_event(Event::Text(event.to_owned())).ok()?;
                }
                if let Some(ctx) = child_capture.as_mut() {
                    ctx.on_text(&event).ok()?;
                    ctx.write_event(Event::Text(event.to_owned())).ok()?;
                }
            }
            Event::CData(event) => {
                if let Some(ctx) = capture_tbl_pr.as_mut() {
                    ctx.write_event(Event::CData(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tbl_grid.as_mut() {
                    ctx.write_event(Event::CData(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tr_pr.as_mut() {
                    ctx.write_event(Event::CData(event.to_owned())).ok()?;
                }
                if let Some(ctx) = capture_tc_pr.as_mut() {
                    ctx.write_event(Event::CData(event.to_owned())).ok()?;
                }
                if let Some(ctx) = child_capture.as_mut() {
                    ctx.on_cdata(&reader, &event).ok()?;
                    ctx.write_event(Event::CData(event.to_owned())).ok()?;
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                let local = local_name(&name);

                if let Some(ctx) = capture_tbl_pr.as_mut() {
                    ctx.write_event(Event::End(event.to_owned())).ok()?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    if ctx.depth == 0 {
                        tbl_pr_xml = capture_tbl_pr.take()?.finish().ok()?.xml;
                    }
                }
                if let Some(ctx) = capture_tbl_grid.as_mut() {
                    ctx.write_event(Event::End(event.to_owned())).ok()?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    if ctx.depth == 0 {
                        tbl_grid_xml = capture_tbl_grid.take()?.finish().ok()?.xml;
                    }
                }
                if let Some(ctx) = capture_tr_pr.as_mut() {
                    ctx.write_event(Event::End(event.to_owned())).ok()?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    if ctx.depth == 0
                        && let Some(row) = current_row.as_mut()
                    {
                        row.tr_pr_xml = capture_tr_pr.take()?.finish().ok()?.xml;
                    }
                }
                if let Some(ctx) = capture_tc_pr.as_mut() {
                    ctx.write_event(Event::End(event.to_owned())).ok()?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    if ctx.depth == 0
                        && let Some(cell) = current_cell.as_mut()
                    {
                        cell.tc_pr_xml = capture_tc_pr.take()?.finish().ok()?.xml;
                    }
                }

                if let Some(ctx) = child_capture.as_mut() {
                    ctx.on_end(&name);
                    ctx.write_event(Event::End(event.to_owned())).ok()?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    if ctx.depth == 0 {
                        let finished = child_capture.take()?.finish().ok()?;
                        push_finished_table_cell_block(current_cell.as_mut()?, finished);
                    }
                }

                if current_cell.is_some()
                    && !(table_depth == 1 && local == b"tc" && child_capture.is_none())
                    && let Some(depth) = cell_content_depth.as_mut()
                {
                    *depth = depth.saturating_sub(1);
                }

                if table_depth == 1 && child_capture.is_none() {
                    match local {
                        b"tc" => {
                            if let Some(cell) = current_cell.take()
                                && let Some(row) = current_row.as_mut()
                            {
                                row.cells.push(cell);
                            }
                            cell_content_depth = None;
                        }
                        b"tr" => {
                            if let Some(row) = current_row.take() {
                                rows.push(row);
                            }
                        }
                        _ => {}
                    }
                }

                if local == b"tbl" {
                    table_depth = table_depth.saturating_sub(1);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Some(ParsedTable {
        tbl_pr_xml,
        tbl_grid_xml,
        rows,
    })
}

fn push_finished_table_cell_block(cell: &mut ParsedTableCell, finished: FinishedCapture) {
    match finished.kind {
        CaptureKind::Paragraph => {
            let lines = normalize_lines(finished.lines);
            cell.blocks
                .push(ParsedTableCellBlock::Paragraph(ParsedParagraph {
                    para_id: finished.para_id,
                    xml: finished.xml,
                    text: join_lines_for_text(&lines),
                }));
        }
        CaptureKind::Table => {
            let parsed = parse_table_for_diff(&finished.xml).map(Box::new);
            cell.blocks
                .push(ParsedTableCellBlock::Table(ParsedNestedTable {
                    xml: finished.xml,
                    text: finished.table_text,
                    parsed,
                }));
        }
    }
}

fn render_diff_table_with_spacing(
    new_table: &ParsedTable,
    old_table: &ParsedTable,
    side: TableRenderSide,
) -> RenderedContent {
    let tbl_pr_xml = match side {
        TableRenderSide::New => &new_table.tbl_pr_xml,
        TableRenderSide::Old => &old_table.tbl_pr_xml,
    };
    let tbl_grid_xml = match side {
        TableRenderSide::New => &new_table.tbl_grid_xml,
        TableRenderSide::Old => &old_table.tbl_grid_xml,
    };

    let mut xml = String::from("<w:tbl>");
    xml.push_str(tbl_pr_xml);
    xml.push_str(tbl_grid_xml);
    let mut trailing_spacing_twips = 0usize;

    let new_row_keys = new_table
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| table_row_match_keys(row, index))
        .collect::<Vec<_>>();
    let old_row_keys = old_table
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| table_row_match_keys(row, index))
        .collect::<Vec<_>>();

    for op in align_match_keys(&new_row_keys, &old_row_keys) {
        match op {
            AlignmentOp::Match {
                new_index,
                old_index,
            } => {
                xml.push_str(&render_matched_table_row(
                    &new_table.rows[new_index],
                    &old_table.rows[old_index],
                    side,
                ));
            }
            AlignmentOp::Add { new_index } => {
                if side == TableRenderSide::New {
                    xml.push_str(&render_single_table_row(
                        &new_table.rows[new_index],
                        TextStyle::Added,
                    ));
                } else {
                    trailing_spacing_twips +=
                        estimate_table_row_height_twips(&new_table.rows[new_index]);
                }
            }
            AlignmentOp::Delete { old_index } => {
                if side == TableRenderSide::Old {
                    xml.push_str(&render_single_table_row(
                        &old_table.rows[old_index],
                        TextStyle::Deleted,
                    ));
                } else {
                    trailing_spacing_twips +=
                        estimate_table_row_height_twips(&old_table.rows[old_index]);
                }
            }
        }
    }

    xml.push_str("</w:tbl>");
    RenderedContent {
        xml,
        trailing_spacing_twips,
    }
}

fn table_row_match_keys(row: &ParsedTableRow, row_index: usize) -> Vec<String> {
    let text = row.text();
    let mut keys = table_match_keys_from_text(&text, row_index);
    for (cell_index, cell) in row.cells.iter().enumerate() {
        let cell_text = cell.text();
        if cell_text.trim().is_empty() {
            continue;
        }
        for variant in paragraph_match_variants(&cell_text) {
            keys.push(format!("tr:cell:{cell_index}:{variant}"));
        }
        if cell_index == 0 {
            break;
        }
    }
    if keys.is_empty() {
        keys.push(format!("tr:{row_index}"));
    }
    keys
}

fn render_matched_table_row(
    new_row: &ParsedTableRow,
    old_row: &ParsedTableRow,
    side: TableRenderSide,
) -> String {
    let mut xml = String::from("<w:tr>");
    match side {
        TableRenderSide::New => xml.push_str(if new_row.tr_pr_xml.is_empty() {
            &old_row.tr_pr_xml
        } else {
            &new_row.tr_pr_xml
        }),
        TableRenderSide::Old => xml.push_str(if old_row.tr_pr_xml.is_empty() {
            &new_row.tr_pr_xml
        } else {
            &old_row.tr_pr_xml
        }),
    }

    let cell_count = new_row.cells.len().max(old_row.cells.len());
    for cell_index in 0..cell_count {
        let new_cell = new_row.cells.get(cell_index);
        let old_cell = old_row.cells.get(cell_index);
        xml.push_str(&render_table_cell_pair(new_cell, old_cell, side));
    }
    xml.push_str("</w:tr>");
    xml
}

fn render_single_table_row(row: &ParsedTableRow, style: TextStyle) -> String {
    let mut xml = String::from("<w:tr>");
    xml.push_str(&row.tr_pr_xml);
    for cell in &row.cells {
        xml.push_str("<w:tc>");
        xml.push_str(if cell.tc_pr_xml.is_empty() {
            "<w:tcPr/>"
        } else {
            &cell.tc_pr_xml
        });
        xml.push_str(&render_styled_table_cell_content(cell, style));
        xml.push_str("</w:tc>");
    }
    xml.push_str("</w:tr>");
    xml
}

fn render_table_cell_pair(
    new_cell: Option<&ParsedTableCell>,
    old_cell: Option<&ParsedTableCell>,
    side: TableRenderSide,
) -> String {
    let tc_pr_xml = match side {
        TableRenderSide::New => new_cell
            .map(|cell| cell.tc_pr_xml.as_str())
            .or_else(|| old_cell.map(|cell| cell.tc_pr_xml.as_str())),
        TableRenderSide::Old => old_cell
            .map(|cell| cell.tc_pr_xml.as_str())
            .or_else(|| new_cell.map(|cell| cell.tc_pr_xml.as_str())),
    }
    .unwrap_or("<w:tcPr/>");

    let content_xml = match (new_cell, old_cell) {
        (Some(new_cell), Some(old_cell)) => {
            render_matched_table_cell_content(new_cell, old_cell, side)
        }
        (Some(new_cell), None) => {
            if side == TableRenderSide::New {
                new_cell.raw_content_xml()
            } else {
                "<w:p/>".to_string()
            }
        }
        (None, Some(old_cell)) => {
            if side == TableRenderSide::Old {
                old_cell.raw_content_xml()
            } else {
                "<w:p/>".to_string()
            }
        }
        (None, None) => "<w:p/>".to_string(),
    };

    let mut xml = String::from("<w:tc>");
    xml.push_str(tc_pr_xml);
    xml.push_str(if content_xml.trim().is_empty() {
        "<w:p/>"
    } else {
        &content_xml
    });
    xml.push_str("</w:tc>");
    xml
}

fn render_matched_table_cell_content(
    new_cell: &ParsedTableCell,
    old_cell: &ParsedTableCell,
    side: TableRenderSide,
) -> String {
    let new_keys = new_cell
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| block.match_keys(index))
        .collect::<Vec<_>>();
    let old_keys = old_cell
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| block.match_keys(index))
        .collect::<Vec<_>>();

    let mut xml = String::new();
    let mut pending_spacing_twips = 0usize;
    for op in align_match_keys(&new_keys, &old_keys) {
        match op {
            AlignmentOp::Match {
                new_index,
                old_index,
            } => {
                let mut rendered = render_matched_table_block(
                    &new_cell.blocks[new_index],
                    &old_cell.blocks[old_index],
                    side,
                );
                if pending_spacing_twips > 0 {
                    let adjusted =
                        add_spacing_before_to_first_paragraph(&rendered.xml, pending_spacing_twips);
                    if adjusted != rendered.xml {
                        rendered.xml = adjusted;
                        pending_spacing_twips = 0;
                    }
                }
                xml.push_str(&rendered.xml);
                pending_spacing_twips += rendered.trailing_spacing_twips;
            }
            AlignmentOp::Add { new_index } => {
                if side == TableRenderSide::New {
                    let mut rendered =
                        render_styled_table_block(&new_cell.blocks[new_index], TextStyle::Added);
                    if pending_spacing_twips > 0 {
                        let adjusted = add_spacing_before_to_first_paragraph(
                            &rendered.xml,
                            pending_spacing_twips,
                        );
                        if adjusted != rendered.xml {
                            rendered.xml = adjusted;
                            pending_spacing_twips = 0;
                        }
                    }
                    xml.push_str(&rendered.xml);
                } else {
                    pending_spacing_twips +=
                        estimate_block_height_twips(&new_cell.blocks[new_index]);
                }
            }
            AlignmentOp::Delete { old_index } => {
                if side == TableRenderSide::Old {
                    let mut rendered =
                        render_styled_table_block(&old_cell.blocks[old_index], TextStyle::Deleted);
                    if pending_spacing_twips > 0 {
                        let adjusted = add_spacing_before_to_first_paragraph(
                            &rendered.xml,
                            pending_spacing_twips,
                        );
                        if adjusted != rendered.xml {
                            rendered.xml = adjusted;
                            pending_spacing_twips = 0;
                        }
                    }
                    xml.push_str(&rendered.xml);
                } else {
                    pending_spacing_twips +=
                        estimate_block_height_twips(&old_cell.blocks[old_index]);
                }
            }
        }
    }

    if pending_spacing_twips > 0 {
        xml.push_str(&spacer_paragraph_xml(pending_spacing_twips));
    }

    if xml.trim().is_empty() {
        "<w:p/>".to_string()
    } else {
        xml
    }
}

fn render_styled_table_cell_content(cell: &ParsedTableCell, style: TextStyle) -> String {
    let mut xml = String::new();
    for block in &cell.blocks {
        xml.push_str(&render_styled_table_block(block, style).xml);
    }
    if xml.trim().is_empty() {
        "<w:p/>".to_string()
    } else {
        xml
    }
}

fn render_matched_table_block(
    new_block: &ParsedTableCellBlock,
    old_block: &ParsedTableCellBlock,
    side: TableRenderSide,
) -> RenderedContent {
    match (new_block, old_block) {
        (
            ParsedTableCellBlock::Paragraph(new_paragraph),
            ParsedTableCellBlock::Paragraph(old_paragraph),
        ) => {
            let xml = if new_paragraph.text == old_paragraph.text {
                match side {
                    TableRenderSide::New => new_paragraph.xml.clone(),
                    TableRenderSide::Old => old_paragraph.xml.clone(),
                }
            } else {
                let (left_segments, right_segments) =
                    diff_text_segments(&old_paragraph.text, &new_paragraph.text);
                match side {
                    TableRenderSide::New => paragraph_xml_from_segments(&left_segments),
                    TableRenderSide::Old => paragraph_xml_from_segments(&right_segments),
                }
            };
            RenderedContent {
                xml,
                trailing_spacing_twips: 0,
            }
        }
        (ParsedTableCellBlock::Table(new_table), ParsedTableCellBlock::Table(old_table)) => {
            match (&new_table.parsed, &old_table.parsed) {
                (Some(new_table), Some(old_table)) => {
                    render_diff_table_with_spacing(new_table, old_table, side)
                }
                _ => RenderedContent {
                    xml: match side {
                        TableRenderSide::New => new_table.xml.clone(),
                        TableRenderSide::Old => old_table.xml.clone(),
                    },
                    trailing_spacing_twips: 0,
                },
            }
        }
        _ => RenderedContent {
            xml: match side {
                TableRenderSide::New => new_block.raw_xml(),
                TableRenderSide::Old => old_block.raw_xml(),
            },
            trailing_spacing_twips: 0,
        },
    }
}

fn render_styled_table_block(block: &ParsedTableCellBlock, style: TextStyle) -> RenderedContent {
    let xml = match block {
        ParsedTableCellBlock::Paragraph(paragraph) => {
            paragraph_xml_from_segments(&[StyledSegment {
                text: paragraph.text.clone(),
                style,
            }])
        }
        ParsedTableCellBlock::Table(table) => match &table.parsed {
            Some(parsed) => render_styled_table(parsed, style),
            None => table.xml.clone(),
        },
    };
    RenderedContent {
        xml,
        trailing_spacing_twips: 0,
    }
}

fn render_styled_table(table: &ParsedTable, style: TextStyle) -> String {
    let mut xml = String::from("<w:tbl>");
    xml.push_str(&table.tbl_pr_xml);
    xml.push_str(&table.tbl_grid_xml);
    for row in &table.rows {
        xml.push_str(&render_single_table_row(row, style));
    }
    xml.push_str("</w:tbl>");
    xml
}

fn estimate_block_height_twips(block: &ParsedTableCellBlock) -> usize {
    match block {
        ParsedTableCellBlock::Paragraph(paragraph) => estimate_paragraph_height_twips(paragraph),
        ParsedTableCellBlock::Table(table) => table
            .parsed
            .as_ref()
            .map(|table| {
                table
                    .rows
                    .iter()
                    .map(estimate_table_row_height_twips)
                    .sum::<usize>()
            })
            .unwrap_or(360),
    }
}

fn estimate_paragraph_height_twips(paragraph: &ParsedParagraph) -> usize {
    let line_count = paragraph.text.lines().count().max(1);
    let line_height = extract_paragraph_line_twips(&paragraph.xml).unwrap_or(360);
    line_count.saturating_mul(line_height.max(1))
}

fn estimate_table_row_height_twips(row: &ParsedTableRow) -> usize {
    let explicit = extract_tr_height_twips(&row.tr_pr_xml);
    let content = row
        .cells
        .iter()
        .map(|cell| {
            cell.blocks
                .iter()
                .map(estimate_block_height_twips)
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    explicit.max(content).max(240)
}

fn extract_paragraph_line_twips(paragraph_xml: &str) -> Option<usize> {
    let spacing_start = paragraph_xml.find("<w:spacing")?;
    let spacing_tail = &paragraph_xml[spacing_start..];
    let line_start = spacing_tail.find("w:line=\"")?;
    let digits = &spacing_tail[line_start + 8..];
    let end = digits.find('"')?;
    digits[..end].parse::<usize>().ok()
}

fn extract_tr_height_twips(tr_pr_xml: &str) -> usize {
    let needle = "w:trHeight";
    let Some(start) = tr_pr_xml.find(needle) else {
        return 0;
    };
    let tail = &tr_pr_xml[start..];
    let Some(val_start) = tail.find("w:val=\"") else {
        return 0;
    };
    let digits = &tail[val_start + 7..];
    let Some(end) = digits.find('"') else {
        return 0;
    };
    digits[..end].parse::<usize>().unwrap_or(0)
}

fn add_spacing_before_to_first_paragraph(xml: &str, twips: usize) -> String {
    if twips == 0 {
        return xml.to_string();
    }

    let mut search_from = 0usize;
    while let Some(rel_start) = xml[search_from..].find("<w:p") {
        let p_start = search_from + rel_start;
        let Some(tag_end_rel) = xml[p_start..].find('>') else {
            break;
        };
        let tag_end = p_start + tag_end_rel;
        let tag = &xml[p_start..=tag_end];
        search_from = tag_end + 1;
        if tag.ends_with("/>") {
            continue;
        }
        let Some(close_rel) = xml[search_from..].find("</w:p>") else {
            break;
        };
        let p_end = search_from + close_rel;
        let paragraph_body = &xml[search_from..p_end];
        if paragraph_has_visible_content(paragraph_body) {
            return add_spacing_inside_paragraph(xml, tag_end + 1, twips);
        }
        search_from = p_end + "</w:p>".len();
    }

    xml.to_string()
}

fn add_spacing_inside_paragraph(xml: &str, insert_at: usize, twips: usize) -> String {
    let mut updated = xml.to_string();

    if updated[insert_at..].starts_with("<w:pPr>")
        && let Some(ppr_end_rel) = updated[insert_at..].find("</w:pPr>")
    {
        let ppr_end = insert_at + ppr_end_rel;
        let ppr_segment = &updated[insert_at..ppr_end];
        if let Some(spacing_rel) = ppr_segment.find("<w:spacing") {
            let spacing_start = insert_at + spacing_rel;
            let spacing_tail = &updated[spacing_start..];
            if let Some(close_rel) = spacing_tail.find("/>") {
                let close = spacing_start + close_rel;
                if updated[spacing_start..close].contains("w:before=\"") {
                    return updated;
                }
                updated.insert_str(close, &format!(" w:before=\"{twips}\""));
                return updated;
            }
        }
        updated.insert_str(ppr_end, &format!("<w:spacing w:before=\"{twips}\"/>"));
        return updated;
    }

    updated.insert_str(
        insert_at,
        &format!("<w:pPr><w:spacing w:before=\"{twips}\"/></w:pPr>"),
    );
    updated
}

fn paragraph_has_visible_content(body: &str) -> bool {
    let body = if let Some(rest) = body.strip_prefix("<w:pPr>") {
        if let Some(end) = rest.find("</w:pPr>") {
            &rest[end + "</w:pPr>".len()..]
        } else {
            body
        }
    } else {
        body
    };

    [
        "<w:t",
        "<w:tab",
        "<w:br",
        "<w:cr",
        "<w:drawing",
        "<w:pict",
        "<w:object",
    ]
    .iter()
    .any(|pattern| body.contains(pattern))
}

fn table_cell_xml_from_table(table_xml: &str) -> String {
    let mut xml = String::new();
    xml.push_str(table_xml);
    xml.push_str("<w:p/>");
    xml
}

fn table_cell_xml_from_rendered_table(rendered: &RenderedContent) -> String {
    let mut xml = String::new();
    xml.push_str(&rendered.xml);
    if rendered.trailing_spacing_twips > 0 {
        xml.push_str(&spacer_paragraph_xml(rendered.trailing_spacing_twips));
    }
    xml.push_str("<w:p/>");
    xml
}

fn spacer_paragraph_xml(twips: usize) -> String {
    format!("<w:p><w:pPr><w:spacing w:before=\"{twips}\"/></w:pPr></w:p>")
}

fn should_preserve_space(text: &str) -> bool {
    text.starts_with(' ') || text.ends_with(' ') || text.contains("  ")
}

fn build_comparison_table_xml(rows: &[ComparisonRow]) -> String {
    let mut xml = String::new();
    xml.push_str("<w:tbl>");
    xml.push_str("<w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblLayout w:type=\"fixed\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideH w:val=\"nil\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/></w:tblBorders></w:tblPr>");
    xml.push_str("<w:tblGrid><w:gridCol w:w=\"9800\"/><w:gridCol w:w=\"9800\"/><w:gridCol w:w=\"1334\"/></w:tblGrid>");
    xml.push_str(&table_row_with_note_xml(
        &header_paragraph_xml("新"),
        &header_paragraph_xml("旧"),
        &header_paragraph_xml("備考"),
        360,
    ));

    for row in rows {
        xml.push_str(&table_row_xml(
            &row.left_xml,
            &row.right_xml,
            &row.note,
            row.min_height_twips,
        ));
    }

    xml.push_str("</w:tbl>");
    xml
}

fn header_paragraph_xml(label: &str) -> String {
    format!(
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>{}</w:t></w:r></w:p>",
        escape(label).as_ref()
    )
}

fn note_paragraph_xml(note: &str) -> String {
    if note.is_empty() {
        return "<w:p/>".to_string();
    }
    format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", escape(note).as_ref())
}

fn table_cell_xml_from_content(content_xml: &str, width: usize) -> String {
    let content = if content_xml.trim().is_empty() {
        "<w:p/>"
    } else {
        content_xml
    };

    format!(
        "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/><w:vAlign w:val=\"top\"/></w:tcPr>{content}</w:tc>"
    )
}

fn build_output_document_xml(
    root_attrs: &BTreeMap<String, String>,
    table_xml: &str,
    sect_pr_xml: &str,
) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    xml.push_str("<w:document");

    for (key, value) in root_attrs {
        xml.push(' ');
        xml.push_str(key);
        xml.push_str("=\"");
        xml.push_str(&escape_xml_attribute(value));
        xml.push('"');
    }

    xml.push_str("><w:body>");
    xml.push_str("<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>新旧対照表</w:t></w:r></w:p>");
    xml.push_str(table_xml);
    xml.push_str(sect_pr_xml);
    xml.push_str("</w:body></w:document>");
    xml
}

fn merge_root_attributes(
    new_attrs: &BTreeMap<String, String>,
    old_attrs: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = new_attrs.clone();

    for (key, value) in old_attrs {
        if key == "mc:Ignorable" {
            let existing = merged.get(key).cloned().unwrap_or_default();
            merged.insert(key.clone(), merge_ignorable_namespaces(&existing, value));
            continue;
        }

        merged.entry(key.clone()).or_insert_with(|| value.clone());
    }

    merged
        .entry("xmlns:w".to_string())
        .or_insert_with(|| DEFAULT_W_NAMESPACE.to_string());
    merged
        .entry("xmlns:r".to_string())
        .or_insert_with(|| DEFAULT_R_NAMESPACE.to_string());

    merged
}

fn merge_ignorable_namespaces(left: &str, right: &str) -> String {
    let mut values = Vec::new();

    for token in left.split_whitespace().chain(right.split_whitespace()) {
        if token.is_empty() || values.iter().any(|existing| existing == token) {
            continue;
        }
        values.push(token.to_string());
    }

    values.join(" ")
}

fn comparison_sect_pr_xml() -> String {
    "<w:sectPr><w:pgSz w:w=\"23814\" w:h=\"16838\" w:orient=\"landscape\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr>".to_string()
}

fn attr_value_local(event: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|attr| {
        if local_name(attr.key.as_ref()) == local {
            Some(String::from_utf8_lossy(attr.value.as_ref()).to_string())
        } else {
            None
        }
    })
}

fn local_name(name: &[u8]) -> &[u8] {
    if let Some(index) = name.iter().rposition(|byte| *byte == b':') {
        &name[index + 1..]
    } else {
        name
    }
}

fn is_text_element(name: &[u8]) -> bool {
    matches!(name, b"t" | b"delText" | b"instrText")
}

fn is_line_break_element(name: &[u8]) -> bool {
    matches!(name, b"br" | b"cr")
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    #[test]
    fn build_comparison_docx_marks_add_and_delete_and_keeps_tables() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="AAAA1111"><w:r><w:t>abcX</w:t></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p w14:paraId="TBLP0001"><w:r><w:t>new table</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="AAAA1111"><w:r><w:t>abc</w:t></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p w14:paraId="TBLP0001"><w:r><w:t>old table</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
    <w:p w14:paraId="DELETED1"><w:r><w:t>removed</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let new_docx = make_minimal_docx(new_document);
        let old_docx = make_minimal_docx(old_document);

        let (output, stats) =
            build_comparison_docx_with_stats(&new_docx, &old_docx).expect("build comparison");
        assert!(stats.matched >= 1);
        assert!(stats.deleted >= 1);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains(r#"<w:color w:val="008000"/>"#));
        assert!(output_xml.contains(r#"<w:color w:val="C00000"/>"#));
        assert!(output_xml.contains("<w:strike/>"));
        assert!(output_xml.contains("new"));
        assert!(output_xml.contains("old"));
        assert!(output_xml.contains("table"));
        assert!(output_xml.contains("新旧対照表"));
        assert!(output_xml.contains("<w:t>備考</w:t>"));
        assert!(!output_xml.contains("（変更）"));
        assert!(!output_xml.contains("（追加）"));
        assert!(!output_xml.contains("（削除）"));
        assert!(!output_xml.contains(r#"<w:br w:type="page"/>"#));
        assert!(output_xml.contains(r#"<w:pgSz w:w="23814" w:h="16838" w:orient="landscape"/>"#));
        assert!(output_xml.contains("<w:tbl>"));
    }

    #[test]
    fn paragraph_level_matching_uses_para_id_with_breaks() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="P1000001"><w:r><w:t>line1</w:t></w:r><w:br/><w:r><w:t>line2X</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="P1000001"><w:r><w:t>line1</w:t></w:r><w:br/><w:r><w:t>line2</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let new_docx = make_minimal_docx(new_document);
        let old_docx = make_minimal_docx(old_document);
        let (output, stats) =
            build_comparison_docx_with_stats(&new_docx, &old_docx).expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains("line1"));
        assert!(output_xml.contains("<w:br/>"));
        assert!(output_xml.contains("<w:t>line2</w:t>"));
        assert!(output_xml.contains("<w:t>X</w:t>"));
        assert!(output_xml.contains(r#"<w:color w:val="008000"/>"#));
    }

    #[test]
    fn comparison_table_uses_one_row_per_aligned_paragraph() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="P1"><w:r><w:t>alpha</w:t></w:r></w:p>
    <w:p w14:paraId="P2"><w:r><w:t>betaX</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="P1"><w:r><w:t>alpha</w:t></w:r></w:p>
    <w:p w14:paraId="P2"><w:r><w:t>beta</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert_eq!(output_xml.matches("<w:tr>").count(), 3);
        assert_eq!(output_xml.matches("<w:trPr>").count(), 3);
        assert!(output_xml.contains(r#"<w:insideH w:val="nil"/>"#));
    }

    #[test]
    fn paragraph_matching_falls_back_to_same_text_when_para_id_differs() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="NEWP0001"><w:r><w:t>shared paragraph</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="OLDP0001"><w:r><w:t>shared paragraph</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains("shared paragraph"));
        assert!(!output_xml.contains("（追加）"));
        assert!(!output_xml.contains("（削除）"));
    }

    #[test]
    fn paragraph_matching_falls_back_to_normalized_text_when_spacing_differs() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t xml:space="preserve">                                             目　　　次</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>目　　　次</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains("目　　　次"));
    }

    #[test]
    fn paragraph_matching_falls_back_to_nfkc_and_digit_zero_normalization() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>ABC 2024</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>ＡＢＣ ２０２９</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (_output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);
    }

    #[test]
    fn paragraph_matching_ignores_space_differences_after_digit_normalization() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>改正　　２０２１年２月２６日</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>改正　　２０１９年 ８月２６日　</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (_output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);
    }

    #[test]
    fn table_matching_falls_back_to_leading_text_signature() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>制　　 定　1965年7月27日</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>改正15回</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>制　　 定　1965年7月27日</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>改正14回</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.deleted, 0);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains("制　　 定　1965年7月27日"));
        assert!(output_xml.contains("改正1"));
        assert!(output_xml.contains(">4</w:t>"));
        assert!(output_xml.contains(">5</w:t>"));
    }

    #[test]
    fn matched_tables_highlight_text_differences_inside_cells() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>
      <w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>
      <w:tr>
        <w:tc>
          <w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>
          <w:p><w:r><w:t>この手続は，２０２１年９月２日から施行する。</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>
      <w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>
      <w:tr>
        <w:tc>
          <w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>
          <w:p><w:r><w:t>この手続は，２０１９年９月２日から施行する。</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(output_xml.contains("この手続は，２０"));
        assert!(output_xml.contains("２０１"));
        assert!(output_xml.contains("年９月２日から施行する。"));
        assert!(output_xml.contains(r#"<w:color w:val="008000"/>"#));
        assert!(output_xml.contains(r#"<w:color w:val="C00000"/>"#));
    }

    #[test]
    fn nested_table_rows_do_not_insert_blank_alignment_rows() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc>
          <w:p><w:r><w:t>5.2 各拠点の統括部は...</w:t></w:r></w:p>
          <w:tbl>
            <w:tr>
              <w:tc><w:p><w:r><w:t>宮城大和工場</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>（和）工務部</w:t></w:r></w:p></w:tc>
            </w:tr>
            <w:tr>
              <w:tc><w:p><w:r><w:t>東富士工場</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>（東）工務部</w:t></w:r></w:p></w:tc>
            </w:tr>
          </w:tbl>
          <w:p/>
          <w:p><w:r><w:t>5.3 各部の文書取扱責任者は...</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc>
          <w:p><w:r><w:t>5.2 各拠点の統括部は...</w:t></w:r></w:p>
          <w:tbl>
            <w:tr>
              <w:tc><w:p><w:r><w:t>宮城大和工場</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>（和）工務部</w:t></w:r></w:p></w:tc>
            </w:tr>
          </w:tbl>
          <w:p/>
          <w:p><w:r><w:t>5.3 各部の文書取扱責任者は...</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(output_xml.contains("宮城大和工場"));
        assert!(output_xml.contains("（和）工務部"));
        assert!(output_xml.contains("東富士工場"));
        assert!(output_xml.contains("w:before=\"360\""));
        assert!(output_xml.contains("5.3 各部の文書取扱責任者は..."));
        assert!(!output_xml.contains(
            "<w:p/></w:tc><w:tc><w:tcPr/><w:p/></w:tc></w:tr></w:tbl><w:p><w:r><w:t>5.3 各部の文書取扱責任者は..."
        ));
    }

    #[test]
    fn matched_table_rows_diff_by_first_cell_and_highlight_changed_second_cell() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>東富士総合センター</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>総合センター管理部</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>東富士総合センター</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>総合センター統括部</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(output_xml.contains("東富士総合センター"));
        assert!(output_xml.contains("総合センター"));
        assert!(output_xml.contains("統括部"));
        assert!(output_xml.contains("管理部"));
        assert!(output_xml.contains(r#"<w:color w:val="008000"/>"#));
        assert!(output_xml.contains(r#"<w:color w:val="C00000"/>"#));
    }

    #[test]
    fn merges_root_attributes_and_mc_ignorable() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" mc:Ignorable="w14" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <w:body>
    <w:p w14:paraId="N1"><w:r><w:t>a</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w15" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <w:body>
    <w:p w15:paraId="O1"><w:r><w:t>b</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(
            output_xml
                .contains(r#"xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#)
        );
        assert!(output_xml.contains(r#"mc:Ignorable="w14 w15""#));
    }

    #[test]
    fn added_and_deleted_rows_use_expected_styles() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="ADD00001"><w:r><w:t>added only</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="DEL00001"><w:r><w:t>deleted only</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let (output, stats) = build_comparison_docx_with_stats(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");

        assert_eq!(stats.added, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.matched, 0);

        let output_xml = read_document_xml_from_docx_bytes(&output);
        assert!(output_xml.contains("added only"));
        assert!(output_xml.contains("deleted only"));
        assert!(output_xml.contains(r#"<w:color w:val="008000"/>"#));
        assert!(output_xml.contains(r#"<w:color w:val="C00000"/>"#));
        assert!(output_xml.contains("<w:strike/>"));
        assert!(!output_xml.contains("（追加）"));
        assert!(!output_xml.contains("（削除）"));
    }

    #[test]
    fn tab_runs_are_preserved_as_w_tab_elements() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="TAB00001"><w:r><w:t>foo</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>barX</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="TAB00001"><w:r><w:t>foo</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>bar</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(output_xml.contains("<w:tab/>"));
    }

    #[test]
    fn returns_missing_part_error_when_document_xml_is_absent() {
        let new_docx = make_minimal_docx_without_document();
        let old_docx = make_minimal_docx_without_document();

        let error = build_comparison_docx(&new_docx, &old_docx).expect_err("missing document.xml");
        match error {
            CompareError::MissingPart { side, path } => {
                assert_eq!(side, "new");
                assert_eq!(path, DOCUMENT_XML_PATH);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn removes_original_document_body_and_keeps_only_comparison_output() {
        let new_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="ORG00001"><w:r><w:t>original-content</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let old_document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:body>
    <w:p w14:paraId="ORG00001"><w:r><w:t>original-content-old</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

        let output = build_comparison_docx(
            &make_minimal_docx(new_document),
            &make_minimal_docx(old_document),
        )
        .expect("build comparison");
        let output_xml = read_document_xml_from_docx_bytes(&output);

        assert!(!output_xml.contains(r#"w14:paraId="ORG00001""#));
        assert!(output_xml.contains("新旧対照表"));
        assert!(!output_xml.contains("（変更）"));
        assert!(output_xml.contains(r#"<w:color w:val="C00000"/>"#));
    }

    fn make_minimal_docx(document_xml: &str) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();

        writer
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        writer
            .write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#)
            .expect("write content types");

        writer.start_file("_rels/.rels", options).expect("rels");
        writer
            .write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#)
            .expect("write rels");

        writer
            .start_file("word/document.xml", options)
            .expect("document.xml");
        writer
            .write_all(document_xml.as_bytes())
            .expect("write document");

        writer.finish().expect("finish zip").into_inner()
    }

    fn make_minimal_docx_without_document() -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();

        writer
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        writer
            .write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#)
            .expect("write content types");

        writer.finish().expect("finish zip").into_inner()
    }

    fn read_document_xml_from_docx_bytes(bytes: &[u8]) -> String {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open zip");
        let mut file = archive
            .by_name(DOCUMENT_XML_PATH)
            .expect("word/document.xml");
        let mut xml = String::new();
        file.read_to_string(&mut xml).expect("read xml");
        xml
    }
}
