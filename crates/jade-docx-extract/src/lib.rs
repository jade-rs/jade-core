mod numbering;
mod parser;
mod styles;

use numbering::{NumberingCounter, NumberingDefinitions};
pub use parser::TableCoord;
use parser::{ParagraphData, ParagraphNumbering, TextSegmentKind};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use styles::Styles;
use thiserror::Error;
use zip::read::ZipArchive;

/// Represents a paragraph extracted from a DOCX file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Paragraph {
    pub text: String,
    pub numbering_text: String,
    pub accepted_text: String,
    pub accepted_numbering_text: String,
    pub in_table: bool,
    /// 表の中での位置。外側の表から順に積む。表の外なら空。
    ///
    /// 入れ子の表もそのまま辿れるので、行やセル単位で 2 つの文書を
    /// 突き合わせたいときに使える。
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub table_path: Vec<TableCoord>,
    pub in_textbox: bool,
    pub in_shape: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub para_id: Option<String>,
}

/// Errors returned by the extraction routines.
#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("XML parse error: {0}")]
    Xml(String),
}

pub type Result<T> = std::result::Result<T, ExtractError>;

/// Reads a DOCX from the given path and returns all paragraphs.
pub fn extract_from_path<P: AsRef<Path>>(path: P) -> Result<Vec<Paragraph>> {
    let file = File::open(path)?;
    extract_from_reader(file)
}

/// Reads the DOCX at the provided path and returns the accepted numbering and text concatenated for each paragraph.
pub fn extract_lines_from_path<P: AsRef<Path>>(path: P) -> Result<Vec<String>> {
    let paragraphs = extract_from_path(path)?;
    Ok(paragraphs
        .into_iter()
        .map(|para| merge_numbering_and_text(&para.accepted_numbering_text, &para.accepted_text))
        .collect())
}

/// Reads a DOCX from the provided in-memory bytes and returns all paragraphs.
pub fn extract_from_bytes(bytes: &[u8]) -> Result<Vec<Paragraph>> {
    let cursor = Cursor::new(bytes);
    extract_from_reader(cursor)
}

/// Reads a DOCX from any `Read + Seek` source and returns all paragraphs.
pub fn extract_from_reader<R: Read + Seek>(reader: R) -> Result<Vec<Paragraph>> {
    let mut archive = ZipArchive::new(reader)?;
    let document_xml = read_zip_entry(&mut archive, "word/document.xml")?
        .ok_or_else(|| ExtractError::Xml("word/document.xml が見つかりません".into()))?;
    let numbering_xml = read_zip_entry(&mut archive, "word/numbering.xml")?;
    let styles_xml = read_zip_entry(&mut archive, "word/styles.xml")?;
    extract_from_xml_parts(
        &document_xml,
        numbering_xml.as_deref(),
        styles_xml.as_deref(),
    )
}

/// Extracts paragraphs from XML parts supplied by an already-loaded package.
pub fn extract_from_xml_parts(
    document_xml: &str,
    numbering_xml: Option<&str>,
    styles_xml: Option<&str>,
) -> Result<Vec<Paragraph>> {
    let numbering = numbering_xml
        .map(NumberingDefinitions::from_xml)
        .transpose()
        .map_err(ExtractError::Xml)?;
    let styles = styles_xml
        .map(Styles::from_xml)
        .transpose()
        .map_err(ExtractError::Xml)?;

    let mut paragraphs = parser::parse_document(document_xml).map_err(ExtractError::Xml)?;

    assign_numbering(&mut paragraphs, numbering.as_ref(), styles.as_ref());

    Ok(paragraphs
        .into_iter()
        .map(|mut para| {
            let text = build_text(&para, false);
            let accepted_text = build_text(&para, true);
            let numbering_text = std::mem::take(&mut para.numbering_text);
            let accepted_numbering_text = std::mem::take(&mut para.accepted_numbering_text);
            let para_id = para.para_id.take();
            let table_path = std::mem::take(&mut para.table_path);

            Paragraph {
                text,
                numbering_text,
                accepted_text,
                accepted_numbering_text,
                in_table: para.in_table,
                table_path,
                in_textbox: para.in_textbox,
                in_shape: para.in_shape,
                para_id,
            }
        })
        .collect())
}

fn merge_numbering_and_text(numbering: &str, text: &str) -> String {
    match (numbering.is_empty(), text.is_empty()) {
        (true, true) => String::new(),
        (true, false) => text.to_string(),
        (false, true) => numbering.to_string(),
        (false, false) => format!("{numbering}{text}"),
    }
}

fn read_zip_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<Option<String>> {
    match archive.by_name(path) {
        Ok(mut file) => {
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            Ok(Some(content))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ExtractError::from(e)),
    }
}

fn assign_numbering(
    paragraphs: &mut [ParagraphData],
    numbering: Option<&NumberingDefinitions>,
    styles: Option<&Styles>,
) {
    if let Some(styles) = styles {
        for para in paragraphs.iter_mut() {
            if para.numbering.is_none()
                && let Some(style_id) = para.style_id.as_deref()
                && let Some((num_id, level)) = styles.resolve_numbering(style_id)
            {
                para.numbering = Some(ParagraphNumbering { num_id, level });
            }
        }
    }

    if numbering.is_none() {
        for para in paragraphs.iter_mut() {
            para.numbering_text.clear();
            para.accepted_numbering_text.clear();
        }
        return;
    }

    let numbering = numbering.unwrap();
    let mut display_counters = NumberingCounter::new();

    for para in paragraphs.iter_mut() {
        if let Some(ref info) = para.numbering {
            let level = info.level;
            let abstract_num_id = numbering.abstract_num_id(&info.num_id).unwrap_or_default();
            let start = numbering.start_value(&info.num_id, level);
            let has_start_override = numbering.has_start_override(&info.num_id, level);
            display_counters.increment(
                &info.num_id,
                abstract_num_id,
                level,
                start,
                has_start_override,
            );
            para.numbering_text = numbering.format(&info.num_id, level, &display_counters);
        } else {
            para.numbering_text.clear();
        }
    }

    let mut accepted_counters = NumberingCounter::new();
    for para in paragraphs.iter_mut() {
        if let Some(ref info) = para.numbering {
            if !para.is_removed_after_accept() {
                let level = info.level;
                let abstract_num_id = numbering.abstract_num_id(&info.num_id).unwrap_or_default();
                let start = numbering.start_value(&info.num_id, level);
                let has_start_override = numbering.has_start_override(&info.num_id, level);
                accepted_counters.increment(
                    &info.num_id,
                    abstract_num_id,
                    level,
                    start,
                    has_start_override,
                );
                para.accepted_numbering_text =
                    numbering.format(&info.num_id, level, &accepted_counters);
            } else {
                para.accepted_numbering_text.clear();
            }
        } else {
            para.accepted_numbering_text.clear();
        }
    }
}

fn build_text(para: &ParagraphData, accepted: bool) -> String {
    let mut result = String::new();
    for segment in &para.segments {
        match segment.kind {
            TextSegmentKind::Deleted => {
                if accepted {
                    continue;
                }
                if !segment.content.is_empty() {
                    result.push_str("~~");
                    result.push_str(&segment.content);
                    result.push_str("~~");
                }
            }
            _ => result.push_str(&segment.content),
        }
    }
    result
}
