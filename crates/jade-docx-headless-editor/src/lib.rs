//! Stateless, renderer-free helpers for editing and querying DOCX byte streams.
//!
//! Each operation loads the input package, applies commands or queries, and
//! optionally exports a new package. For repeated edits to the same document,
//! embed [`jade_docx_document_core::word_document::WordDocument`] directly to avoid
//! paying the ZIP parse/export cost on every call.

pub use jade_docx_headless_api::{
    HeadlessApplyAndQueryRequest, HeadlessCommand, HeadlessDeleteDirection, HeadlessDocumentDiff,
    HeadlessDocumentStats, HeadlessEditRequest, HeadlessQuery, HeadlessQueryRequest,
    HeadlessQueryResponse, HeadlessQueryResult, HeadlessRevisionKindFilter, HeadlessRunDiff,
    HeadlessTableCellDiff, HeadlessTableDiff,
};

use futures::executor;
use jade_docx_document_core::document::{BodyElement, Table};
use jade_docx_document_core::headless_api::{
    apply_headless_commands_to_document, execute_headless_queries_on_document,
    HeadlessApiApplyError, HeadlessApiQueryError,
};
use jade_docx_document_core::word_document::{DocxExportError, WordDocument};
use jade_docx_document_core::Options;

#[derive(thiserror::Error, Debug)]
pub enum HeadlessEditError {
    #[error("failed to load docx: {0}")]
    Load(String),
    #[error(transparent)]
    Apply(#[from] HeadlessApiApplyError),
    #[error("failed to export docx: {0}")]
    Export(#[from] DocxExportError),
}

#[derive(thiserror::Error, Debug)]
pub enum HeadlessQueryError {
    #[error("failed to load docx: {0}")]
    Load(String),
    #[error(transparent)]
    Query(#[from] HeadlessApiQueryError),
}

#[derive(thiserror::Error, Debug)]
pub enum HeadlessApplyAndQueryError {
    #[error("failed to load docx: {0}")]
    Load(String),
    #[error(transparent)]
    Apply(#[from] HeadlessApiApplyError),
    #[error(transparent)]
    Query(#[from] HeadlessApiQueryError),
    #[error("failed to export docx: {0}")]
    Export(#[from] DocxExportError),
}

#[derive(Debug, Clone)]
pub struct HeadlessApplyAndQueryOutput {
    pub output_docx: Vec<u8>,
    pub query: HeadlessQueryResponse,
    pub diff: Option<HeadlessDocumentDiff>,
}

/// Applies a batch of commands to `input_docx` and returns an exported DOCX.
pub fn apply_headless_commands(
    input_docx: &[u8],
    request: &HeadlessEditRequest,
) -> Result<Vec<u8>, HeadlessEditError> {
    let mut word_doc = executor::block_on(WordDocument::load(input_docx, &Options::default()))
        .map_err(|err| HeadlessEditError::Load(err.to_string()))?;

    apply_headless_commands_to_document(&mut word_doc, request)?;

    word_doc.export_docx().map_err(HeadlessEditError::from)
}

/// Executes read-only queries against a DOCX without modifying its package.
pub fn execute_headless_queries(
    input_docx: &[u8],
    request: &HeadlessQueryRequest,
) -> Result<HeadlessQueryResponse, HeadlessQueryError> {
    let word_doc = executor::block_on(WordDocument::load(input_docx, &Options::default()))
        .map_err(|err| HeadlessQueryError::Load(err.to_string()))?;
    execute_headless_queries_on_document(&word_doc, request).map_err(HeadlessQueryError::from)
}

/// Applies commands and evaluates queries against the resulting in-memory state.
pub fn apply_headless_commands_and_execute_queries(
    input_docx: &[u8],
    request: &HeadlessApplyAndQueryRequest,
) -> Result<HeadlessApplyAndQueryOutput, HeadlessApplyAndQueryError> {
    let mut word_doc = executor::block_on(WordDocument::load(input_docx, &Options::default()))
        .map_err(|err| HeadlessApplyAndQueryError::Load(err.to_string()))?;

    let diff_before = document_stats(&word_doc);
    let paragraph_before = paragraph_texts(&word_doc);
    let run_before = run_signatures(&word_doc);
    let table_before = table_signatures(&word_doc);
    let section_before = section_signature(&word_doc);

    let edit_request = HeadlessEditRequest {
        commands: request.commands.clone(),
    };
    apply_headless_commands_to_document(&mut word_doc, &edit_request)?;

    let query_request = HeadlessQueryRequest {
        queries: request.queries.clone(),
    };
    let query = execute_headless_queries_on_document(&word_doc, &query_request)?;
    let output_docx = if request.dry_run {
        Vec::new()
    } else {
        word_doc.export_docx()?
    };

    let diff = if request.include_diff {
        let diff_after = document_stats(&word_doc);
        let paragraph_after = paragraph_texts(&word_doc);
        let run_after = run_signatures(&word_doc);
        let table_after = table_signatures(&word_doc);
        let section_after = section_signature(&word_doc);
        Some(HeadlessDocumentDiff {
            before: diff_before,
            after: diff_after,
            changed_paragraphs: changed_paragraph_indices(&paragraph_before, &paragraph_after),
            changed_runs: changed_run_indices(&run_before, &run_after),
            changed_tables: changed_table_indices(&table_before, &table_after),
            section_changed: section_before != section_after,
        })
    } else {
        None
    };

    Ok(HeadlessApplyAndQueryOutput {
        output_docx,
        query,
        diff,
    })
}

pub fn apply_headless_commands_and_execute_queries_with_diff(
    input_docx: &[u8],
    request: &HeadlessApplyAndQueryRequest,
) -> Result<HeadlessApplyAndQueryOutput, HeadlessApplyAndQueryError> {
    apply_headless_commands_and_execute_queries(input_docx, request)
}

fn document_stats(word_doc: &WordDocument) -> HeadlessDocumentStats {
    HeadlessDocumentStats {
        paragraph_count: word_doc.paragraph_count(),
        table_count: table_count(word_doc),
        comment_count: word_doc.comments.len(),
        revision_count: revision_count(word_doc),
    }
}

fn paragraph_texts(word_doc: &WordDocument) -> Vec<String> {
    (0..word_doc.paragraph_count())
        .map(|index| {
            word_doc
                .paragraph(index)
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn changed_paragraph_indices(before: &[String], after: &[String]) -> Vec<usize> {
    let max = before.len().max(after.len());
    (0..max)
        .filter(|index| before.get(*index) != after.get(*index))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunSignature {
    text: String,
    properties: String,
}

fn run_signatures(word_doc: &WordDocument) -> Vec<Vec<RunSignature>> {
    (0..word_doc.paragraph_count())
        .map(|index| {
            word_doc
                .paragraph(index)
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| RunSignature {
                            text: run.text.clone(),
                            properties: format!("{:?}", run.properties),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn changed_run_indices(
    before: &[Vec<RunSignature>],
    after: &[Vec<RunSignature>],
) -> Vec<HeadlessRunDiff> {
    let max = before.len().max(after.len());
    (0..max)
        .filter_map(|paragraph| {
            let before_runs = before.get(paragraph).cloned().unwrap_or_default();
            let after_runs = after.get(paragraph).cloned().unwrap_or_default();
            if before_runs == after_runs {
                return None;
            }
            let run_max = before_runs.len().max(after_runs.len());
            Some(HeadlessRunDiff {
                paragraph,
                before_run_count: before_runs.len(),
                after_run_count: after_runs.len(),
                changed_run_indices: (0..run_max)
                    .filter(|index| before_runs.get(*index) != after_runs.get(*index))
                    .collect(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableSignature {
    rows: usize,
    columns: usize,
    cells: Vec<Vec<String>>,
}

fn table_signatures(word_doc: &WordDocument) -> Vec<TableSignature> {
    let mut signatures = Vec::new();
    let mut index = 0usize;
    while let Some(table) = word_doc.table_by_order(index) {
        signatures.push(table_signature(table));
        index = index.saturating_add(1);
    }
    signatures
}

fn table_signature(table: &Table) -> TableSignature {
    let rows = table.rows.len();
    let columns = table
        .rows
        .iter()
        .map(|row| {
            row.grid_before.unwrap_or(0) as usize
                + row
                    .cells
                    .iter()
                    .map(|cell| cell.span.unwrap_or(1).max(1) as usize)
                    .sum::<usize>()
                + row.grid_after.unwrap_or(0) as usize
        })
        .max()
        .unwrap_or(0);
    let cells = table
        .rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| body_blocks_text(&cell.blocks))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    TableSignature {
        rows,
        columns,
        cells,
    }
}

fn body_blocks_text(blocks: &[BodyElement]) -> String {
    let mut text = String::new();
    for block in blocks {
        match block {
            BodyElement::Paragraph(paragraph) => {
                for run in &paragraph.runs {
                    text.push_str(&run.text);
                }
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        text.push_str(&body_blocks_text(&cell.blocks));
                    }
                }
            }
            BodyElement::Math(math) => {
                if let Some(preview) = math.preview_text(128) {
                    text.push_str(&preview);
                }
            }
        }
    }
    text
}

fn changed_table_indices(
    before: &[TableSignature],
    after: &[TableSignature],
) -> Vec<HeadlessTableDiff> {
    let max = before.len().max(after.len());
    (0..max)
        .filter_map(|table_order_index| {
            let before_table = before.get(table_order_index);
            let after_table = after.get(table_order_index);
            if before_table == after_table {
                return None;
            }
            let before_cells = before_table
                .map(|table| table.cells.as_slice())
                .unwrap_or(&[]);
            let after_cells = after_table
                .map(|table| table.cells.as_slice())
                .unwrap_or(&[]);
            let row_max = before_cells.len().max(after_cells.len());
            let mut changed_cells = Vec::new();
            for row in 0..row_max {
                let before_row = before_cells.get(row).map(Vec::as_slice).unwrap_or(&[]);
                let after_row = after_cells.get(row).map(Vec::as_slice).unwrap_or(&[]);
                let column_max = before_row.len().max(after_row.len());
                for column in 0..column_max {
                    if before_row.get(column) != after_row.get(column) {
                        changed_cells.push(HeadlessTableCellDiff { row, column });
                    }
                }
            }
            Some(HeadlessTableDiff {
                table_order_index,
                before_rows: before_table.map(|table| table.rows).unwrap_or(0),
                after_rows: after_table.map(|table| table.rows).unwrap_or(0),
                before_columns: before_table.map(|table| table.columns).unwrap_or(0),
                after_columns: after_table.map(|table| table.columns).unwrap_or(0),
                changed_cells,
            })
        })
        .collect()
}

fn section_signature(word_doc: &WordDocument) -> String {
    word_doc
        .document
        .as_ref()
        .map(|document| {
            format!(
                "{:?}|{:?}",
                document.default_section.as_ref(),
                document.sections.as_slice()
            )
        })
        .unwrap_or_default()
}

fn table_count(word_doc: &WordDocument) -> usize {
    let mut count = 0usize;
    while word_doc.table_by_order(count).is_some() {
        count = count.saturating_add(1);
    }
    count
}

fn revision_count(word_doc: &WordDocument) -> usize {
    let mut count = 0usize;
    for paragraph_index in 0..word_doc.paragraph_count() {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        count = count.saturating_add(
            paragraph
                .runs
                .iter()
                .filter(|run| run.revision.is_some())
                .count(),
        );
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::{Cursor, Read, Write};
    use std::path::PathBuf;
    use zip::{ZipArchive, ZipWriter};

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    fn read_zip_entry(docx: &[u8], path: &str) -> Vec<u8> {
        let mut archive = ZipArchive::new(Cursor::new(docx)).expect("zip archive");
        let mut file = archive.by_name(path).expect("zip entry");
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).expect("read zip entry");
        buf
    }

    fn rewrite_zip_entry<F>(docx: &[u8], target_path: &str, rewrite: F) -> Vec<u8>
    where
        F: Fn(&[u8]) -> Vec<u8>,
    {
        let mut input = ZipArchive::new(Cursor::new(docx)).expect("zip archive");
        let mut output = Cursor::new(Vec::<u8>::new());
        let mut writer = ZipWriter::new(&mut output);

        for i in 0..input.len() {
            let mut file = input.by_index(i).expect("zip entry by index");
            let name = file.name().unwrap().into_owned();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).expect("read zip entry");

            let rewritten = if name == target_path {
                rewrite(&contents)
            } else {
                contents
            };

            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(&rewritten).expect("write file");
        }

        writer.finish().expect("finish zip");
        output.into_inner()
    }

    fn table_styles_fixture() -> Vec<u8> {
        let input = std::fs::read(fixture_path("table2x2.docx")).expect("read table fixture");
        rewrite_zip_entry(&input, "word/styles.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("styles.xml utf8");
            xml.replacen(
                "</w:styles>",
                r#"<w:style w:type="table" w:styleId="Alpha"><w:name w:val="Alpha Style"/><w:tcPr><w:shd w:fill="AABBCC"/></w:tcPr></w:style><w:style w:type="table" w:styleId="Beta"><w:name w:val="Beta Style"/><w:basedOn w:val="Alpha"/><w:tblStylePr w:type="firstRow"><w:tcPr><w:shd w:fill="112233"/></w:tcPr></w:tblStylePr></w:style></w:styles>"#,
                1,
            )
            .into_bytes()
        })
    }

    /// A document whose WordprocessingML parts name `w15`/`wp14` in
    /// `mc:Ignorable` without declaring them — what namespace-rewriting tools
    /// leave behind, and a package Word refuses to open.
    fn undeclared_ignorable_prefixes_fixture() -> Vec<u8> {
        let input = std::fs::read(fixture_path("yokohama_table_repro.docx"))
            .expect("read yokohama_table_repro fixture");
        let strip = |bytes: &[u8]| {
            let xml = String::from_utf8(bytes.to_vec()).expect("part utf8");
            let stripped = xml
                .replacen(
                    " xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\"",
                    "",
                    1,
                )
                .replacen(
                    " xmlns:wp14=\"http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing\"",
                    "",
                    1,
                );
            assert!(
                !stripped.contains("xmlns:w15="),
                "fixture must lose its w15 declaration"
            );
            stripped.into_bytes()
        };
        let without_document_prefixes = rewrite_zip_entry(&input, "word/document.xml", strip);
        rewrite_zip_entry(&without_document_prefixes, "word/styles.xml", strip)
    }

    #[test]
    fn export_redeclares_prefixes_named_by_ignorable_in_every_wordprocessing_part() {
        let input = undeclared_ignorable_prefixes_fixture();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "X".to_string(),
            }],
        };

        let output = apply_headless_commands(&input, &request).expect("headless edit");

        for part in ["word/document.xml", "word/styles.xml"] {
            let xml = String::from_utf8(read_zip_entry(&output, part)).expect("part utf8");
            let root_start = xml.find("<w:").expect("root element");
            let root_end = root_start + xml[root_start..].find('>').expect("root end");
            let root = &xml[root_start..root_end];
            assert!(
                root.contains(
                    " xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\""
                ),
                "{part} must declare w15 again: {root}"
            );
            assert!(
                root.contains("mc:Ignorable=\"w14 w15 "),
                "{part} keeps its compatibility list: {root}"
            );
        }
        let document =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("utf8");
        let root_end = document.find("<w:body").expect("body");
        assert!(
            document[..root_end].contains(
                " xmlns:wp14=\"http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing\""
            ),
            "document.xml must declare wp14 again"
        );
        assert!(document.contains("<w:t>X</w:t>") || document.contains(">X<"));
    }

    fn add_custom_attr_to_second_paragraph(xml: &str) -> String {
        let needle = "<w:p";
        let mut positions = xml.match_indices(needle);
        let _first = positions.next();
        let second = positions.next().expect("second paragraph should exist").0;
        let tag_end_rel = xml[second..].find('>').expect("paragraph tag should close");
        let insert_at = second + tag_end_rel;

        let mut out = String::with_capacity(xml.len() + 40);
        out.push_str(&xml[..insert_at]);
        out.push_str(r#" w14:dummyUnsupported="keep-me""#);
        out.push_str(&xml[insert_at..]);
        out
    }

    fn add_custom_attr_to_first_paragraph(xml: &str) -> String {
        let needle = "<w:p";
        let first = xml
            .match_indices(needle)
            .next()
            .expect("paragraph should exist")
            .0;
        let tag_end_rel = xml[first..].find('>').expect("paragraph tag should close");
        let insert_at = first + tag_end_rel;

        let mut out = String::with_capacity(xml.len() + 40);
        out.push_str(&xml[..insert_at]);
        out.push_str(r#" w14:dummyUnsupported="keep-me""#);
        out.push_str(&xml[insert_at..]);
        out
    }

    fn inject_custom_non_run_marker(xml: &str) -> String {
        xml.replacen("<w:r>", r#"<w:proofErr w:type="spellStart"/><w:r>"#, 1)
    }

    fn add_custom_attr_to_first_table(xml: &str) -> String {
        let needle = "<w:tbl";
        let first = xml
            .match_indices(needle)
            .next()
            .expect("table should exist")
            .0;
        let tag_end_rel = xml[first..].find('>').expect("table tag should close");
        let insert_at = first + tag_end_rel;

        let mut out = String::with_capacity(xml.len() + 40);
        out.push_str(&xml[..insert_at]);
        out.push_str(r#" w14:dummyTableAttr="keep-me""#);
        out.push_str(&xml[insert_at..]);
        out
    }

    fn first_part_path_by_prefix(docx: &[u8], prefix: &str) -> String {
        let mut archive = ZipArchive::new(Cursor::new(docx)).expect("zip archive");
        for i in 0..archive.len() {
            let file = archive.by_index(i).expect("zip entry by index");
            let name = file.name().unwrap().into_owned();
            if name.starts_with(prefix) && name.ends_with(".xml") {
                return name;
            }
        }
        panic!("no part with prefix {prefix}");
    }

    fn first_part_path_by_prefix_containing(docx: &[u8], prefix: &str, needle: &str) -> String {
        let mut archive = ZipArchive::new(Cursor::new(docx)).expect("zip archive");
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).expect("zip entry by index");
            let name = file.name().unwrap().into_owned();
            if !name.starts_with(prefix) || !name.ends_with(".xml") {
                continue;
            }
            let mut content = String::new();
            file.read_to_string(&mut content).expect("read xml");
            if content.contains(needle) {
                return name;
            }
        }
        panic!("no part with prefix {prefix} containing {needle}");
    }

    fn sample_insert_request() -> HeadlessEditRequest {
        HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "X".to_string(),
            }],
        }
    }

    #[test]
    fn insert_text_preserves_paragraph_structure_contract() {
        // This checks the Rust boundary corresponding to formal/InsertText.lean.
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 2,
                text: "X".to_string(),
            }],
        };
        let output = apply_headless_commands(&input, &request).expect("insert text");
        let query = execute_headless_queries(
            &output,
            &HeadlessQueryRequest {
                queries: vec![
                    HeadlessQuery::ParagraphCount,
                    HeadlessQuery::ParagraphText { paragraph: 0 },
                    HeadlessQuery::ParagraphText { paragraph: 1 },
                ],
            },
        )
        .expect("reload output and query paragraphs");

        assert!(matches!(
            query.results[0],
            HeadlessQueryResult::ParagraphCount { paragraph_count: 2 }
        ));
        assert!(
            matches!(
                &query.results[1],
                HeadlessQueryResult::ParagraphText { text: Some(text), .. } if text == "HeXllo\u{c}"
            ),
            "first paragraph: {:?}",
            query.results[1]
        );
        assert!(matches!(
            &query.results[2],
            HeadlessQueryResult::ParagraphText { text: Some(text), .. } if text == "World"
        ));
    }

    fn sample_complex_request() -> HeadlessEditRequest {
        HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "A".to_string(),
                },
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 1,
                    count: 1,
                },
                // The break above leaves paragraph 1 empty, so the range this
                // replaces is empty too. It used to ask for 0..1 and rely on
                // the offset being clamped; offsets past the end are now
                // rejected rather than quietly moved.
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 1,
                    start_offset: 0,
                    end_paragraph: 1,
                    end_offset: 0,
                    text: "B".to_string(),
                },
            ],
        }
    }

    #[test]
    fn insert_text_preserves_unedited_styles_part() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let original_styles = read_zip_entry(&input, "word/styles.xml");

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 5,
                text: " Headless".to_string(),
            }],
        };

        let output = apply_headless_commands(&input, &request).expect("apply headless edits");
        let exported_styles = read_zip_entry(&output, "word/styles.xml");
        let exported_document =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("utf8 xml");

        assert_eq!(
            original_styles, exported_styles,
            "styles.xml should stay byte-identical"
        );
        assert!(
            exported_document.contains("Hello Headless"),
            "document.xml should include inserted text"
        );
    }

    #[test]
    fn insert_text_preserves_unedited_settings_part() {
        let input = std::fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let original_settings = read_zip_entry(&input, "word/settings.xml");

        let output = apply_headless_commands(&input, &sample_insert_request())
            .expect("apply headless edits");
        let exported_settings = read_zip_entry(&output, "word/settings.xml");

        assert_eq!(
            original_settings, exported_settings,
            "settings.xml should stay byte-identical"
        );
    }

    #[test]
    fn insert_text_preserves_unedited_numbering_part() {
        let input = std::fs::read(fixture_path("numbering-by-style.docx")).expect("read fixture");
        let original_numbering = read_zip_entry(&input, "word/numbering.xml");

        let output = apply_headless_commands(&input, &sample_insert_request())
            .expect("apply headless edits");
        let exported_numbering = read_zip_entry(&output, "word/numbering.xml");

        assert_eq!(
            original_numbering, exported_numbering,
            "numbering.xml should stay byte-identical"
        );
    }

    #[test]
    fn insert_text_preserves_unedited_comments_part() {
        let input = std::fs::read(fixture_path("comment.docx")).expect("read fixture");
        let original_comments = read_zip_entry(&input, "word/comments.xml");

        let output = apply_headless_commands(&input, &sample_insert_request())
            .expect("apply headless edits");
        let exported_comments = read_zip_entry(&output, "word/comments.xml");

        assert_eq!(
            original_comments, exported_comments,
            "comments.xml should stay byte-identical"
        );
    }

    #[test]
    fn insert_text_keeps_unknown_attributes_on_unedited_paragraphs() {
        let input = std::fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
            add_custom_attr_to_second_paragraph(&xml).into_bytes()
        });

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "X".to_string(),
            }],
        };
        let output = apply_headless_commands(&injected, &request).expect("apply headless edits");
        let exported_document =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("utf8 xml");

        assert!(
            exported_document.contains(r#"w14:dummyUnsupported="keep-me""#),
            "unsupported attribute on unedited paragraph should be preserved"
        );
    }

    #[test]
    fn complex_edits_keep_unknown_attributes_on_unedited_paragraphs() {
        let input = std::fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
            add_custom_attr_to_second_paragraph(&xml).into_bytes()
        });

        let output =
            apply_headless_commands(&injected, &sample_complex_request()).expect("apply edits");
        let exported_document =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("utf8 xml");

        assert!(
            exported_document.contains(r#"w14:dummyUnsupported="keep-me""#),
            "unsupported attribute on untouched original paragraph should be preserved"
        );
    }

    #[test]
    fn add_comment_creates_related_parts_and_preserves_unknown_nodes() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
            let xml = add_custom_attr_to_first_paragraph(&xml);
            inject_custom_non_run_marker(&xml).into_bytes()
        });

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: Some(0),
                end_paragraph: 0,
                end_offset: Some(5),
                text: "LLM note".to_string(),
                author: Some("LLM".to_string()),
                initials: Some("LM".to_string()),
                date: Some("2026-02-24T00:00:00Z".to_string()),
            }],
        };

        let output = apply_headless_commands(&injected, &request).expect("apply add_comment");
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
        let comments_xml =
            String::from_utf8(read_zip_entry(&output, "word/comments.xml")).expect("comments xml");
        let rels_xml = String::from_utf8(read_zip_entry(&output, "word/_rels/document.xml.rels"))
            .expect("document rels");
        let content_types =
            String::from_utf8(read_zip_entry(&output, "[Content_Types].xml")).expect("types");

        assert!(
            document_xml.contains(r#"w14:dummyUnsupported="keep-me""#),
            "unsupported paragraph attribute should survive comment insertion",
        );
        assert!(
            document_xml.contains(r#"<w:proofErr w:type="spellStart"/>"#),
            "non-run marker should survive comment insertion",
        );
        assert!(
            document_xml.contains("<w:commentRangeStart"),
            "document.xml should include comment range markers",
        );
        assert!(
            comments_xml.contains("LLM note"),
            "comments.xml should contain inserted comment text",
        );
        assert!(
            rels_xml.contains("relationships/comments"),
            "document relationships should include comments relation",
        );
        assert!(
            content_types.contains("wordprocessingml.comments+xml"),
            "content types should include comments override",
        );
    }

    #[test]
    fn set_header_paragraph_text_updates_header_fixture_without_dropping_unknown_attrs() {
        let input = std::fs::read(fixture_path("header.docx")).expect("read fixture");
        let header_path = first_part_path_by_prefix(&input, "word/header");
        let injected = rewrite_zip_entry(&input, &header_path, |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("header xml utf8");
            add_custom_attr_to_first_paragraph(&xml).into_bytes()
        });

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetHeaderParagraphText {
                paragraph: 0,
                text: "Header Updated by LLM".to_string(),
            }],
        };

        let output = apply_headless_commands(&injected, &request).expect("apply header edit");
        let header_xml =
            String::from_utf8(read_zip_entry(&output, &header_path)).expect("header xml");

        assert!(
            header_xml.contains("Header Updated by LLM"),
            "header text should be replaced",
        );
        assert!(
            header_xml.contains(r#"w14:dummyUnsupported="keep-me""#),
            "unsupported header attributes should be preserved",
        );
    }

    #[test]
    fn set_footer_paragraph_text_updates_footer_fixture_without_dropping_unknown_attrs() {
        let input = std::fs::read(fixture_path("footer.docx")).expect("read fixture");
        let footer_path =
            first_part_path_by_prefix_containing(&input, "word/footer", "Hello footer");
        let injected = rewrite_zip_entry(&input, &footer_path, |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("footer xml utf8");
            add_custom_attr_to_first_paragraph(&xml).into_bytes()
        });

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetFooterParagraphText {
                paragraph: 0,
                text: "Footer Updated by LLM".to_string(),
            }],
        };

        let output = apply_headless_commands(&injected, &request).expect("apply footer edit");
        let footer_xml =
            String::from_utf8(read_zip_entry(&output, &footer_path)).expect("footer xml");

        assert!(
            footer_xml.contains("Footer Updated by LLM"),
            "footer text should be replaced",
        );
        assert!(
            footer_xml.contains(r#"w14:dummyUnsupported="keep-me""#),
            "unsupported footer attributes should be preserved",
        );
    }

    #[test]
    fn start_stop_change_tracking_marks_only_commands_within_session() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::StartChangeTracking {
                    author: Some("LLM".to_string()),
                    initials: None,
                    date: Some("2026-02-24T00:00:00Z".to_string()),
                    date_utc: Some("2026-02-24T00:00:00Z".to_string()),
                },
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 5,
                    text: "A".to_string(),
                },
                HeadlessCommand::StopChangeTracking,
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 6,
                    text: "B".to_string(),
                },
            ],
        };

        let output = apply_headless_commands(&input, &request).expect("apply tracking edits");
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");

        assert_eq!(
            document_xml.matches("<w:ins ").count(),
            1,
            "only edits inside tracking session should become insertion revisions",
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            max_shrink_iters: 2000,
            .. ProptestConfig::default()
        })]

        #[test]
        fn property_add_comment_preserves_non_run_nodes_and_text(
            start in 0usize..5,
            len in 1usize..6,
            note in "[a-z]{1,8}"
        ) {
            let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
                let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
                inject_custom_non_run_marker(&xml).into_bytes()
            });

            let end = (start + len).min(5);
            prop_assume!(end > start);

            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::AddComment {
                    start_paragraph: 0,
                    start_offset: Some(start),
                    end_paragraph: 0,
                    end_offset: Some(end),
                    text: note.clone(),
                    author: Some("LLM".to_string()),
                    initials: None,
                    date: None,
                }],
            };

            let output = apply_headless_commands(&injected, &request).expect("apply add_comment");
            let document_xml = String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
            prop_assert!(
                document_xml.contains(r#"<w:proofErr w:type="spellStart"/>"#),
                "non-run marker should survive"
            );

            let roundtrip = executor::block_on(WordDocument::load(&output, &Options::default()))
                .expect("reload exported docx");
            let paragraph = roundtrip.paragraph(0).expect("paragraph");
            let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            prop_assert!(
                text.starts_with("Hello"),
                "paragraph text should preserve original content, got: {text:?}"
            );
            prop_assert!(
                !paragraph.comment_ranges.is_empty(),
                "comment ranges should be retained after roundtrip"
            );
        }

        #[test]
        fn property_clone_then_fill_table_row_round_trips_exact_values(
            source_row in 0usize..2,
            target_row in 0usize..3,
            clear_text in any::<bool>(),
            values in prop::collection::vec("[A-Za-z0-9]{0,12}", 0..3),
        ) {
            let input = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
            let request = HeadlessApplyAndQueryRequest {
                commands: vec![
                    HeadlessCommand::CloneTableRow {
                        table_order_index: 0,
                        source_row,
                        target_row,
                        clear_text,
                    },
                    HeadlessCommand::FillTableRow {
                        table_order_index: 0,
                        row: target_row,
                        values: values.clone(),
                    },
                ],
                queries: vec![HeadlessQuery::Table {
                    table_order_index: 0,
                }],
                include_diff: false,
                dry_run: false,
            };

            let output = apply_headless_commands_and_execute_queries_with_diff(&input, &request)
                .expect("clone and fill row");
            let HeadlessQueryResult::Table {
                value: Some(table), ..
            } = &output.query.results[0]
            else {
                return Err(TestCaseError::fail("expected table query result"));
            };
            prop_assert_eq!(table.rows.len(), 3);
            for (column, expected) in values.iter().enumerate() {
                prop_assert_eq!(&table.rows[target_row].cells[column].text, expected);
            }

            let reopened = execute_headless_queries(
                &output.output_docx,
                &HeadlessQueryRequest {
                    queries: vec![HeadlessQuery::Table {
                        table_order_index: 0,
                    }],
                },
            )
            .expect("reopen cloned and filled table");
            let HeadlessQueryResult::Table {
                value: Some(table), ..
            } = &reopened.results[0]
            else {
                return Err(TestCaseError::fail("expected reopened table query result"));
            };
            prop_assert_eq!(table.rows.len(), 3);
            for (column, expected) in values.iter().enumerate() {
                prop_assert_eq!(&table.rows[target_row].cells[column].text, expected);
            }
        }

        #[test]
        fn property_set_table_width_preserves_unknown_attrs(
            width in 0u16..1000u16,
            use_percent in any::<bool>()
        ) {
            let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
            let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
                let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
                add_custom_attr_to_first_table(&xml).into_bytes()
            });

            let width_spec = if use_percent {
                format!("{}%", width)
            } else {
                format!("{}pt", width)
            };
            let expected_type = if use_percent { "pct" } else { "dxa" };
            let expected_w = if use_percent {
                (u32::from(width) * 50).to_string()
            } else {
                (u32::from(width) * 20).to_string()
            };

            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetTableWidth {
                    table_order_index: 0,
                    width: width_spec,
                }],
            };

            let output = apply_headless_commands(&injected, &request).expect("apply set_table_width");
            let document_xml = String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
            prop_assert!(
                document_xml.contains(r#"w14:dummyTableAttr="keep-me""#),
                "unsupported table attribute should be preserved",
            );
            let needle_a = format!(r#"<w:tblW w:w="{expected_w}" w:type="{expected_type}"/>"#);
            let needle_b = format!(r#"<w:tblW w:type="{expected_type}" w:w="{expected_w}"/>"#);
            prop_assert!(
                document_xml.contains(&needle_a) || document_xml.contains(&needle_b),
                "table width xml should match command spec",
            );
        }

        #[test]
        fn property_insert_delete_column_roundtrip_keeps_merged_shape(
            column_index in 0usize..4,
            width in 1u16..200u16
        ) {
            let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertTableColumn {
                        table_order_index: 0,
                        column_index,
                        width: Some(width as f32),
                        template_column: None,
                    },
                    HeadlessCommand::DeleteTableColumn {
                        table_order_index: 0,
                        column_index,
                    },
                ],
            };

            let output = apply_headless_commands(&input, &request).expect("apply commands");
            let query = HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Table { table_order_index: 0 }],
            };
            let result = execute_headless_queries(&output, &query).expect("query");
            let HeadlessQueryResult::Table {
                value: Some(table), ..
            } = &result.results[0] else {
                panic!("expected table result");
            };
            prop_assert_eq!(table.columns.len(), 3);
            prop_assert_eq!(table.rows.len(), 2);
            prop_assert!(
                table.rows.iter().any(|row| row.cells.iter().any(|cell| cell.span.is_some_and(|s| s > 1))),
                "merged span should still exist after insert/delete roundtrip"
            );
        }

        #[test]
        fn property_table_structure_edits_preserve_unknown_table_attrs(
            op in 0u8..4,
            index in 0usize..6
        ) {
            let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
            let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
                let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
                add_custom_attr_to_first_table(&xml).into_bytes()
            });

            let command = match op {
                0 => HeadlessCommand::InsertTableColumn {
                    table_order_index: 0,
                    column_index: index % 4,
                    width: Some(40.0),
                    template_column: None,
                },
                1 => HeadlessCommand::DeleteTableColumn {
                    table_order_index: 0,
                    column_index: index % 3,
                },
                2 => HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: index % 3,
                    template_row: None,
                },
                _ => HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: index % 2,
                },
            };

            let request = HeadlessEditRequest {
                commands: vec![command],
            };
            let output = apply_headless_commands(&injected, &request).expect("apply command");
            let document_xml = String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
            prop_assert!(
                document_xml.contains(r#"w14:dummyTableAttr="keep-me""#),
                "unsupported table attribute should be preserved across structural edits",
            );
        }

        #[test]
        fn property_table_row_and_column_queries_resolve_on_merged_fixture(
            row in 0usize..4,
            column in 0usize..6
        ) {
            let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
            let request = HeadlessQueryRequest {
                queries: vec![
                    HeadlessQuery::TableRow {
                        table_order_index: 0,
                        row: row % 2,
                    },
                    HeadlessQuery::TableColumn {
                        table_order_index: 0,
                        column: column % 3,
                    },
                ],
            };

            let response = execute_headless_queries(&input, &request).expect("query succeeds");
            let HeadlessQueryResult::TableRow {
                row,
                value: Some(table_row),
                ..
            } = &response.results[0] else {
                panic!("expected table row");
            };
            prop_assert_eq!(*row, table_row.row);
            prop_assert!(
                table_row
                    .cells
                    .windows(2)
                    .all(|window| window[0].column < window[1].column),
                "row cells should have increasing logical columns"
            );

            let HeadlessQueryResult::TableColumn {
                column,
                value: Some(table_column),
                ..
            } = &response.results[1] else {
                panic!("expected table column");
            };
            prop_assert_eq!(*column, table_column.column);
            prop_assert_eq!(table_column.cells.len(), 2);
            prop_assert_eq!(table_column.cells[0].row, 0);
            prop_assert_eq!(table_column.cells[1].row, 1);
        }
    }

    #[test]
    fn execute_headless_queries_returns_search_matches() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::SearchText {
                query: "Hello".to_string(),
                start_paragraph: 0,
                max_results: 5,
                context_before_chars: 3,
                context_after_chars: 3,
                case_insensitive: false,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        assert_eq!(response.results.len(), 1);
        let HeadlessQueryResult::SearchText { matches, .. } = &response.results[0] else {
            panic!("expected search result")
        };
        assert!(!matches.is_empty(), "search should find hello");
    }

    #[test]
    fn execute_headless_queries_returns_table_snapshot() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![
                HeadlessQuery::TableCount,
                HeadlessQuery::Table {
                    table_order_index: 0,
                },
            ],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        assert_eq!(response.results.len(), 2);
        let HeadlessQueryResult::TableCount { table_count } = response.results[0] else {
            panic!("expected table_count")
        };
        assert_eq!(table_count, 1);

        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.results[1]
        else {
            panic!("expected table snapshot")
        };
        assert_eq!(table.columns.len(), 3);
        assert!(
            table.rows.iter().any(|row| {
                row.cells
                    .iter()
                    .any(|cell| cell.span.is_some_and(|span| span > 1))
            }),
            "merged cells should be exposed via span",
        );
    }

    #[test]
    fn execute_headless_queries_returns_table_cell_for_merged_logical_column() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableCell {
                table_order_index: 0,
                row: 0,
                column: 1,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::TableCell {
            value: Some(cell), ..
        } = &response.results[0]
        else {
            panic!("expected table_cell")
        };
        assert_eq!(cell.text, "Hello");
        assert_eq!(cell.span, Some(2));
    }

    #[test]
    fn execute_headless_queries_returns_table_row_with_logical_cell_columns() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableRow {
                table_order_index: 0,
                row: 0,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::TableRow {
            value: Some(row), ..
        } = &response.results[0]
        else {
            panic!("expected table_row")
        };
        assert_eq!(row.row, 0);
        assert_eq!(row.cells.len(), 2);
        assert_eq!(row.cells[0].column, 0);
        assert_eq!(row.cells[0].span, Some(2));
        assert_eq!(row.cells[1].column, 2);
    }

    #[test]
    fn execute_headless_queries_returns_table_column_with_spanned_cells() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableColumn {
                table_order_index: 0,
                column: 1,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::TableColumn {
            value: Some(column),
            ..
        } = &response.results[0]
        else {
            panic!("expected table_column")
        };
        assert_eq!(column.column, 1);
        assert_eq!(column.cells.len(), 2);
        assert_eq!(column.cells[0].row, 0);
        assert_eq!(column.cells[0].span, Some(2));
        assert_eq!(column.cells[0].text, "Hello");
        assert_eq!(column.cells[1].row, 1);
        assert_eq!(column.cells[1].span, Some(2));
        assert_eq!(column.cells[1].text, "World");
    }

    #[test]
    fn execute_headless_queries_returns_comments_with_anchors() {
        let input = std::fs::read(fixture_path("comment.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Comments {
                start_comment: 0,
                max_comments: 10,
                author: None,
                resolved: None,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::Comments {
            comments,
            total_comments,
            ..
        } = &response.results[0]
        else {
            panic!("expected comments result")
        };
        assert!(*total_comments >= 1);
        assert!(
            comments.iter().any(|comment| !comment.anchors.is_empty()),
            "at least one comment should include anchor range metadata",
        );
        assert!(
            comments
                .iter()
                .any(|comment| comment.reply_to_comment_id.as_deref() == Some("0")),
            "reply comment should expose parent comment id",
        );
    }

    #[test]
    fn execute_headless_queries_returns_comment_threads_for_fixture() {
        let input = std::fs::read(fixture_path("comment.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::CommentThreads {
                start_thread: 0,
                max_threads: 10,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::CommentThreads {
            threads,
            total_threads,
            ..
        } = &response.results[0]
        else {
            panic!("expected comment threads result")
        };
        assert_eq!(*total_threads, 1);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comment.id, "0");
        assert_eq!(threads[0].replies.len(), 1);
        assert_eq!(threads[0].replies[0].comment.id, "1");
        assert_eq!(
            threads[0].replies[0].comment.reply_to_comment_id.as_deref(),
            Some("0")
        );
    }

    #[test]
    fn execute_headless_queries_returns_document_stats() {
        let input = std::fs::read(fixture_path("review.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentStats],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentStats { stats } = &response.results[0] else {
            panic!("expected document_stats result")
        };
        assert!(stats.paragraph_count >= 1);
        assert_eq!(stats.table_count, 0);
        assert_eq!(stats.comment_count, 0);
        assert!(stats.revision_count >= 1);
    }

    #[test]
    fn execute_headless_queries_returns_document_outline_for_fixture() {
        let input = std::fs::read(fixture_path("yokohama.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 20,
                max_level: Some(3),
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };
        assert!(*total_headings >= 1);
        assert!(
            headings
                .iter()
                .any(|heading| heading.text.contains("第1章") || heading.text.contains("附則")),
            "outline should include legal chapter-like headings from fixture",
        );
    }

    /// `nda.docx` は第１条〜第８条を `w:numPr` の採番（`lvlText` = `第%1条　`）で
    /// 持つ。本文ランには括弧書きしか無いため、採番ラベルを見ないと見出しが
    /// 1件も取れない。
    #[test]
    fn execute_headless_queries_detects_article_headings_in_nda_fixture() {
        let input = std::fs::read(fixture_path("nda.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 50,
                max_level: None,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };

        assert_eq!(*total_headings, 8, "nda.docx has 第１条〜第８条");
        assert!(
            headings.iter().all(|heading| heading.level == 6),
            "articles are the deepest legal level: {headings:?}",
        );
        assert_eq!(headings[0].numbering_text.as_deref(), Some("第１条　"));
        assert_eq!(headings[0].text, "（秘密情報）");
        assert_eq!(headings[7].numbering_text.as_deref(), Some("第８条　"));
        assert_eq!(headings[7].text, "（管轄）");
    }

    /// `contract.docx` も条見出しは採番由来。ほかの採番（`1.　` / `(1)　`）が
    /// 見出しに混ざらないことも合わせて確認する。
    #[test]
    fn execute_headless_queries_detects_article_headings_in_contract_fixture() {
        let input = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 50,
                max_level: None,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };

        assert_eq!(*total_headings, 7, "contract.docx has 第１条〜第７条");
        assert!(
            headings.iter().all(|heading| {
                heading.level == 6
                    && heading
                        .numbering_text
                        .as_deref()
                        .is_some_and(|marker| marker.starts_with('第') && marker.contains('条'))
            }),
            "every outline entry should be an article: {headings:?}",
        );
    }

    #[test]
    fn execute_headless_queries_returns_numbering_text_for_numbered_fixture() {
        let input = std::fs::read(fixture_path("numbering-by-style.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::ParagraphSlice {
                start_paragraph: 0,
                max_paragraphs: 10,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::ParagraphSlice { paragraphs, .. } = &response.results[0] else {
            panic!("expected paragraph_slice result")
        };
        assert!(
            paragraphs.iter().any(|paragraph| paragraph
                .numbering_text
                .as_ref()
                .is_some_and(|text| !text.is_empty())),
            "at least one paragraph should include numbering_text",
        );
    }

    #[test]
    fn execute_headless_queries_returns_revisions_for_review_fixture() {
        let input = std::fs::read(fixture_path("review.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::Revisions {
            revisions,
            total_revisions,
            ..
        } = &response.results[0]
        else {
            panic!("expected revisions result")
        };
        assert_eq!(*total_revisions, 2);
        assert_eq!(revisions.len(), 2);
        assert!(
            revisions.iter().any(|revision| revision.kind == "inserted"),
            "fixture should include inserted revision",
        );
        assert!(
            revisions.iter().any(|revision| revision.kind == "deleted"),
            "fixture should include deleted revision",
        );
    }

    #[test]
    fn execute_headless_queries_filters_revisions_by_author_and_kind() {
        let input = std::fs::read(fixture_path("review.docx")).expect("read fixture");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: Some(HeadlessRevisionKindFilter::Inserted),
                author: Some("Unknown author".to_string()),
                date_from: None,
                date_to: None,
            }],
        };

        let response = execute_headless_queries(&input, &request).expect("query succeeds");
        let HeadlessQueryResult::Revisions {
            revisions,
            total_revisions,
            ..
        } = &response.results[0]
        else {
            panic!("expected revisions result")
        };
        assert_eq!(*total_revisions, 0);
        assert!(revisions.is_empty());
    }

    #[test]
    fn accept_and_reject_revisions_commands_edit_review_fixture() {
        let input = std::fs::read(fixture_path("review.docx")).expect("read fixture");
        let accept_request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: false,
            dry_run: false,
        };
        let accept_output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &accept_request)
                .expect("accept revisions");
        let HeadlessQueryResult::ParagraphText {
            text: Some(accepted_text),
            ..
        } = &accept_output.query.results[0]
        else {
            panic!("expected paragraph text");
        };
        assert_eq!(accepted_text, "Hello ");

        let reject_request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: false,
            dry_run: false,
        };
        let reject_output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &reject_request)
                .expect("reject revisions");
        let HeadlessQueryResult::ParagraphText {
            text: Some(rejected_text),
            ..
        } = &reject_output.query.results[0]
        else {
            panic!("expected paragraph text");
        };
        assert_eq!(rejected_text, "World");
    }

    #[test]
    fn comment_update_resolve_delete_commands_edit_fixture_comment_parts() {
        let input = std::fs::read(fixture_path("comment.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![
                HeadlessCommand::UpdateComment {
                    comment_id: "0".to_string(),
                    text: "Updated from headless".to_string(),
                },
                HeadlessCommand::ResolveComment {
                    comment_id: "0".to_string(),
                    resolved: true,
                },
                HeadlessCommand::DeleteComment {
                    comment_id: "1".to_string(),
                },
            ],
            queries: vec![HeadlessQuery::Comments {
                start_comment: 0,
                max_comments: 10,
                author: None,
                resolved: None,
            }],
            include_diff: false,
            dry_run: false,
        };

        let output = apply_headless_commands_and_execute_queries_with_diff(&input, &request)
            .expect("apply comment commands");
        let HeadlessQueryResult::Comments {
            comments,
            total_comments,
            ..
        } = &output.query.results[0]
        else {
            panic!("expected comments result");
        };
        assert_eq!(*total_comments, 1);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "0");
        assert_eq!(comments[0].text, "Updated from headless");
        assert_eq!(comments[0].resolved, Some(true));

        let comments_xml =
            String::from_utf8(read_zip_entry(&output.output_docx, "word/comments.xml"))
                .expect("comments xml");
        assert!(
            comments_xml.contains("Updated from headless"),
            "comments.xml should contain updated text",
        );
        assert!(
            !comments_xml.contains("w:id=\"1\""),
            "deleted comment entry should be removed from comments.xml",
        );
    }

    #[test]
    fn apply_and_query_with_diff_returns_changed_paragraphs() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "X".to_string(),
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: true,
            dry_run: false,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &request).expect("apply");
        let diff = output.diff.expect("diff should be present");
        assert!(diff.before.paragraph_count >= 1);
        assert_eq!(diff.after.paragraph_count, diff.before.paragraph_count);
        assert!(
            diff.changed_paragraphs.contains(&0),
            "paragraph 0 should be marked as changed",
        );
        assert!(
            diff.changed_runs.iter().any(|run| run.paragraph == 0),
            "paragraph 0 run diff should be reported",
        );
    }

    #[test]
    fn apply_and_query_with_diff_reports_changed_table_cells() {
        let input = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row: 1,
                column: 0,
                text: "changed".to_string(),
            }],
            queries: vec![HeadlessQuery::Table {
                table_order_index: 0,
            }],
            include_diff: true,
            dry_run: false,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &request).expect("apply");
        let diff = output.diff.expect("diff should be present");
        let table_diff = diff
            .changed_tables
            .iter()
            .find(|table| table.table_order_index == 0)
            .expect("table diff");
        assert_eq!(table_diff.before_rows, 2);
        assert_eq!(table_diff.after_rows, 2);
        assert!(
            table_diff
                .changed_cells
                .iter()
                .any(|cell| cell.row == 1 && cell.column == 0),
            "changed cell should be reported",
        );
    }

    #[test]
    fn apply_and_query_with_diff_reports_section_changes() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::SetPageMargins {
                top: Some(72.0),
                bottom: Some(72.0),
                left: Some(54.0),
                right: Some(54.0),
                header: None,
                footer: None,
                gutter: None,
            }],
            queries: vec![HeadlessQuery::DocumentStats],
            include_diff: true,
            dry_run: false,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &request).expect("apply");
        let diff = output.diff.expect("diff should be present");
        assert!(diff.section_changed, "section diff should be reported");
    }

    #[test]
    fn replace_placeholder_handles_split_runs_in_fixture_docx_and_exports() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let split_placeholder = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document xml");
            xml.replace(
                "<w:t>Hello</w:t><w:br w:type=\"page\"/>",
                concat!(
                    "<w:t xml:space=\"preserve\">Dear </w:t></w:r>",
                    "<w:r><w:rPr><w:b/><w:rtl w:val=\"0\"/></w:rPr><w:t>${</w:t></w:r>",
                    "<w:r><w:rPr><w:i/><w:rtl w:val=\"0\"/></w:rPr><w:t>name</w:t></w:r>",
                    "<w:r><w:rPr><w:rtl w:val=\"0\"/></w:rPr><w:t>}</w:t><w:br w:type=\"page\"/>",
                ),
            )
            .into_bytes()
        });
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::ReplacePlaceholder {
                placeholder: "${name}".to_string(),
                text: "Alice".to_string(),
                max_replacements: Some(1),
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: true,
            dry_run: false,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&split_placeholder, &request)
                .expect("replace placeholder");
        let HeadlessQueryResult::ParagraphText {
            text: Some(text), ..
        } = &output.query.results[0]
        else {
            panic!("expected paragraph_text result");
        };
        assert_eq!(text.trim_end_matches('\u{c}'), "Dear Alice");
        let diff = output.diff.expect("diff should be present");
        assert!(diff.changed_paragraphs.contains(&0));
        assert!(diff.changed_runs.iter().any(|run| run.paragraph == 0));

        let document_xml =
            String::from_utf8(read_zip_entry(&output.output_docx, "word/document.xml"))
                .expect("document xml");
        assert!(document_xml.contains("Alice"));
        assert!(!document_xml.contains("${"));
        assert!(!document_xml.contains("name</w:t></w:r><w:r"));

        let roundtrip = execute_headless_queries(
            &output.output_docx,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            },
        )
        .expect("reload exported docx");
        let HeadlessQueryResult::ParagraphText {
            text: Some(roundtrip_text),
            ..
        } = &roundtrip.results[0]
        else {
            panic!("expected paragraph_text result");
        };
        assert_eq!(roundtrip_text.trim_end_matches('\u{c}'), "Dear Alice");
    }

    #[test]
    fn dry_run_replace_placeholder_reports_query_and_diff_without_docx_bytes() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let placeholder_docx = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document xml");
            xml.replace("<w:t>Hello</w:t>", "<w:t>${name}</w:t>")
                .into_bytes()
        });
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::ReplacePlaceholder {
                placeholder: "${name}".to_string(),
                text: "Dry Alice".to_string(),
                max_replacements: None,
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: true,
            dry_run: true,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&placeholder_docx, &request)
                .expect("dry-run replace placeholder");
        assert!(output.output_docx.is_empty());
        let HeadlessQueryResult::ParagraphText {
            text: Some(text), ..
        } = &output.query.results[0]
        else {
            panic!("expected paragraph_text result");
        };
        assert_eq!(text.trim_end_matches('\u{c}'), "Dry Alice");
        let diff = output.diff.expect("diff should be present");
        assert!(diff.changed_paragraphs.contains(&0));
        assert!(diff.changed_runs.iter().any(|run| run.paragraph == 0));
    }

    #[test]
    fn clone_and_fill_table_row_commands_export_and_roundtrip_fixture() {
        let input = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![
                HeadlessCommand::CloneTableRow {
                    table_order_index: 0,
                    source_row: 1,
                    target_row: 2,
                    clear_text: true,
                },
                HeadlessCommand::FillTableRow {
                    table_order_index: 0,
                    row: 2,
                    values: vec!["SKU-42".to_string(), "1200".to_string()],
                },
            ],
            queries: vec![HeadlessQuery::Table {
                table_order_index: 0,
            }],
            include_diff: true,
            dry_run: false,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &request).expect("apply");
        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &output.query.results[0]
        else {
            panic!("expected table result");
        };
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[2].cells[0].text, "SKU-42");
        assert_eq!(table.rows[2].cells[1].text, "1200");
        let diff = output.diff.expect("diff should be present");
        let table_diff = diff
            .changed_tables
            .iter()
            .find(|table| table.table_order_index == 0)
            .expect("table diff");
        assert_eq!(table_diff.before_rows, 2);
        assert_eq!(table_diff.after_rows, 3);
        assert!(table_diff
            .changed_cells
            .iter()
            .any(|cell| cell.row == 2 && cell.column == 0));

        let roundtrip = execute_headless_queries(
            &output.output_docx,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Table {
                    table_order_index: 0,
                }],
            },
        )
        .expect("reload exported docx");
        let HeadlessQueryResult::Table {
            value: Some(roundtrip_table),
            ..
        } = &roundtrip.results[0]
        else {
            panic!("expected table result");
        };
        assert_eq!(roundtrip_table.rows.len(), 3);
        assert_eq!(roundtrip_table.rows[2].cells[0].text, "SKU-42");
        assert_eq!(roundtrip_table.rows[2].cells[1].text, "1200");
    }

    #[test]
    fn dry_run_apply_and_query_returns_diff_without_exporting_docx() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "DRY ".to_string(),
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: true,
            dry_run: true,
        };

        let output =
            apply_headless_commands_and_execute_queries_with_diff(&input, &request).expect("apply");
        assert!(
            output.output_docx.is_empty(),
            "dry-run should not export mutated docx bytes",
        );
        let HeadlessQueryResult::ParagraphText {
            text: Some(text), ..
        } = &output.query.results[0]
        else {
            panic!("expected paragraph_text result");
        };
        assert!(text.starts_with("DRY Hello"));
        assert!(
            output
                .diff
                .expect("diff should be present")
                .changed_paragraphs
                .contains(&0),
            "dry-run should still report the simulated paragraph change",
        );
    }

    #[test]
    fn insert_table_command_exports_table_structure() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertTable {
                after_paragraph: 0,
                rows: 2,
                columns: 3,
                column_widths: vec![20.0, 30.0, 40.0],
                caret_offset: None,
            }],
        };
        let output = apply_headless_commands(&input, &request).expect("apply insert_table");
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
        assert!(
            document_xml.contains("<w:tbl"),
            "insert_table should add table xml to document.xml",
        );
        assert!(
            document_xml.contains("<w:gridCol w:w=\"400\"/>")
                && document_xml.contains("<w:gridCol w:w=\"600\"/>")
                && document_xml.contains("<w:gridCol w:w=\"800\"/>"),
            "insert_table should emit tblGrid widths from command arguments",
        );
    }

    #[test]
    fn set_table_width_command_updates_tblw_and_preserves_unknown_attrs() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let injected = rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
            add_custom_attr_to_first_table(&xml).into_bytes()
        });

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetTableWidth {
                table_order_index: 0,
                width: "75%".to_string(),
            }],
        };
        let output = apply_headless_commands(&injected, &request).expect("apply set_table_width");
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");

        assert!(
            document_xml.contains("w14:dummyTableAttr=\"keep-me\""),
            "unsupported table attribute should be preserved",
        );
        assert!(
            document_xml.contains("<w:tblW w:w=\"3750\" w:type=\"pct\"/>")
                || document_xml.contains("<w:tblW w:type=\"pct\" w:w=\"3750\"/>"),
            "table width should be updated to 75%",
        );
    }

    #[test]
    fn set_table_column_widths_command_updates_tblgrid_widths() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetTableColumnWidths {
                table_order_index: 0,
                widths: vec![10.0, 20.0, 30.0],
            }],
        };
        let output =
            apply_headless_commands(&input, &request).expect("apply set_table_column_widths");
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
        assert!(
            document_xml.contains("<w:gridCol w:w=\"200\"/>")
                && document_xml.contains("<w:gridCol w:w=\"400\"/>")
                && document_xml.contains("<w:gridCol w:w=\"600\"/>"),
            "tblGrid should reflect column widths in twips",
        );
    }

    #[test]
    fn apply_and_query_on_fixture_returns_updated_table_snapshot_and_docx() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![
                HeadlessCommand::SetTableWidth {
                    table_order_index: 0,
                    width: "75%".to_string(),
                },
                HeadlessCommand::SetTableColumnWidths {
                    table_order_index: 0,
                    widths: vec![10.0, 20.0, 30.0],
                },
            ],
            queries: vec![
                HeadlessQuery::TableCount,
                HeadlessQuery::Table {
                    table_order_index: 0,
                },
            ],
            include_diff: false,
            dry_run: false,
        };

        let response = apply_headless_commands_and_execute_queries(&input, &request)
            .expect("apply + query should succeed");
        let HeadlessQueryResult::TableCount { table_count } = response.query.results[0] else {
            panic!("expected table_count result");
        };
        assert_eq!(table_count, 1);

        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.query.results[1]
        else {
            panic!("expected table snapshot result");
        };
        assert_eq!(table.style.get("width").map(String::as_str), Some("75.00%"),);
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.columns[0].width, Some(10.0));
        assert_eq!(table.columns[1].width, Some(20.0));
        assert_eq!(table.columns[2].width, Some(30.0));

        let document_xml =
            String::from_utf8(read_zip_entry(&response.output_docx, "word/document.xml"))
                .expect("document xml");
        assert!(
            document_xml.contains("<w:tblW w:w=\"3750\" w:type=\"pct\"/>")
                || document_xml.contains("<w:tblW w:type=\"pct\" w:w=\"3750\"/>")
        );
        assert!(document_xml.contains("<w:gridCol w:w=\"200\"/>"));
        assert!(document_xml.contains("<w:gridCol w:w=\"400\"/>"));
        assert!(document_xml.contains("<w:gridCol w:w=\"600\"/>"));
    }

    #[test]
    fn apply_and_query_on_fixture_updates_text_then_queries_paragraph() {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "X".to_string(),
            }],
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 0 }],
            include_diff: false,
            dry_run: false,
        };

        let response = apply_headless_commands_and_execute_queries(&input, &request)
            .expect("apply + query should succeed");
        let HeadlessQueryResult::ParagraphText {
            text: Some(text), ..
        } = &response.query.results[0]
        else {
            panic!("expected paragraph_text result");
        };
        assert!(text.starts_with("XHello"));

        let roundtrip = executor::block_on(WordDocument::load(
            &response.output_docx,
            &Options::default(),
        ))
        .expect("reload exported docx");
        assert!(roundtrip.paragraph(0).expect("paragraph").runs[0]
            .text
            .starts_with("XHello"));
    }

    #[test]
    fn set_table_cell_text_command_edits_fixture_table_cell() {
        let input = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row: 1,
                column: 0,
                text: "LLM updated cell".to_string(),
            }],
            queries: vec![HeadlessQuery::Table {
                table_order_index: 0,
            }],
            include_diff: false,
            dry_run: false,
        };

        let response = apply_headless_commands_and_execute_queries(&input, &request)
            .expect("apply + query should succeed");
        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.query.results[0]
        else {
            panic!("expected table result");
        };
        assert_eq!(table.rows[1].cells[0].text, "LLM updated cell");
    }

    #[test]
    fn insert_delete_table_row_column_commands_edit_fixture_table_shape() {
        let input = std::fs::read(fixture_path("table2x2.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![
                HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: 1,
                    template_row: None,
                },
                HeadlessCommand::InsertTableColumn {
                    table_order_index: 0,
                    column_index: 1,
                    width: Some(40.0),
                    template_column: None,
                },
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 0,
                },
                HeadlessCommand::DeleteTableColumn {
                    table_order_index: 0,
                    column_index: 0,
                },
            ],
            queries: vec![HeadlessQuery::Table {
                table_order_index: 0,
            }],
            include_diff: false,
            dry_run: false,
        };

        let response = apply_headless_commands_and_execute_queries(&input, &request)
            .expect("apply + query should succeed");
        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.query.results[0]
        else {
            panic!("expected table result");
        };
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[1].cells.len(), 2);

        let document_xml =
            String::from_utf8(read_zip_entry(&response.output_docx, "word/document.xml"))
                .expect("document xml");
        assert!(document_xml.contains("<w:tbl"));
        assert_eq!(document_xml.matches("<w:tr>").count(), 2);
        assert_eq!(document_xml.matches("<w:gridCol").count(), 2);
    }

    #[test]
    fn insert_table_column_command_supports_merged_table_fixture() {
        let input = std::fs::read(fixture_path("table3x2merged.docx")).expect("read fixture");
        let request = HeadlessApplyAndQueryRequest {
            commands: vec![HeadlessCommand::InsertTableColumn {
                table_order_index: 0,
                column_index: 1,
                width: Some(40.0),
                template_column: None,
            }],
            queries: vec![HeadlessQuery::Table {
                table_order_index: 0,
            }],
            include_diff: false,
            dry_run: false,
        };
        let response = apply_headless_commands_and_execute_queries(&input, &request)
            .expect("insert column should succeed");
        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.query.results[0]
        else {
            panic!("expected table query result");
        };
        assert_eq!(table.columns.len(), 4);
    }

    // ---------------------------------------------------------------------
    // Multi-paragraph comments (selection spanning paragraph boundaries).
    // ---------------------------------------------------------------------

    fn paragraph_text(docx: &[u8], index: usize) -> String {
        let doc =
            executor::block_on(WordDocument::load(docx, &Options::default())).expect("load docx");
        doc.paragraph(index)
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .unwrap_or_default()
    }

    #[test]
    fn add_comment_spanning_two_paragraphs_anchors_both_and_round_trips() {
        let input = std::fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let before0 = paragraph_text(&input, 0);
        let before1 = paragraph_text(&input, 1);
        assert!(
            !before1.is_empty(),
            "fixture paragraph 1 needs text to anchor the comment end",
        );

        // End at the first character boundary of paragraph 1 (the fixture is
        // Japanese, so a byte offset of 1 would be mid-codepoint).
        let end_offset = before1
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .expect("paragraph 1 has at least one character");

        // Span from offset 0 of paragraph 0 to `end_offset` of paragraph 1.
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: Some(0),
                end_paragraph: 1,
                end_offset: Some(end_offset),
                text: "spans two paragraphs".to_string(),
                author: Some("Tester".to_string()),
                initials: Some("T".to_string()),
                date: Some("2026-06-19T00:00:00Z".to_string()),
            }],
        };

        let output = apply_headless_commands(&input, &request)
            .expect("multi-paragraph comment should apply");

        // Text of both paragraphs is unchanged by the run splitting / markers.
        assert_eq!(
            paragraph_text(&output, 0),
            before0,
            "paragraph 0 text changed"
        );
        assert_eq!(
            paragraph_text(&output, 1),
            before1,
            "paragraph 1 text changed"
        );

        // The comment exists once and anchors *both* paragraphs after a reload.
        let response = execute_headless_queries(
            &output,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Comments {
                    start_comment: 0,
                    max_comments: 50,
                    author: None,
                    resolved: None,
                }],
            },
        )
        .expect("comments query");
        let HeadlessQueryResult::Comments { comments, .. } = &response.results[0] else {
            panic!("expected comments result");
        };
        let comment = comments
            .iter()
            .find(|c| c.text == "spans two paragraphs")
            .expect("the spanning comment should be present");
        let mut anchored_paragraphs: Vec<usize> =
            comment.anchors.iter().map(|a| a.paragraph).collect();
        anchored_paragraphs.sort_unstable();
        anchored_paragraphs.dedup();
        assert_eq!(
            anchored_paragraphs,
            vec![0, 1],
            "the comment should anchor ranges in paragraph 0 and paragraph 1",
        );

        // The end paragraph carries the reference marker at the requested offset.
        let end_anchor = comment
            .anchors
            .iter()
            .find(|a| a.paragraph == 1)
            .expect("end paragraph anchor");
        assert_eq!(end_anchor.start, 0);
        assert_eq!(end_anchor.end, end_offset);
        assert_eq!(end_anchor.marker_position, Some(end_offset));

        // document.xml carries start and end markers and the package reloads.
        let document_xml =
            String::from_utf8(read_zip_entry(&output, "word/document.xml")).expect("document xml");
        assert!(document_xml.contains("<w:commentRangeStart"));
        assert!(document_xml.contains("<w:commentRangeEnd"));
        assert!(document_xml.contains("<w:commentReference"));
        executor::block_on(WordDocument::load(&output, &Options::default()))
            .expect("spanning-comment docx must reload");
    }

    #[test]
    fn add_comment_spanning_three_paragraphs_covers_the_middle_paragraph() {
        let input = std::fs::read(fixture_path("yokohama_page6.docx")).expect("read fixture");
        let middle_len = paragraph_text(&input, 1).len();
        assert!(middle_len > 0, "middle paragraph needs text");

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: Some(0),
                end_paragraph: 2,
                end_offset: Some(1),
                text: "three paragraphs".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };

        let output = apply_headless_commands(&input, &request).expect("apply spanning comment");

        let response = execute_headless_queries(
            &output,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Comments {
                    start_comment: 0,
                    max_comments: 50,
                    author: None,
                    resolved: None,
                }],
            },
        )
        .expect("comments query");
        let HeadlessQueryResult::Comments { comments, .. } = &response.results[0] else {
            panic!("expected comments result");
        };
        let comment = comments
            .iter()
            .find(|c| c.text == "three paragraphs")
            .expect("spanning comment present");

        // The fully-covered middle paragraph (index 1) must be anchored 0..len.
        let middle = comment
            .anchors
            .iter()
            .find(|a| a.paragraph == 1)
            .expect("middle paragraph anchor");
        assert_eq!(middle.start, 0);
        assert_eq!(
            middle.end, middle_len,
            "middle paragraph should be fully covered"
        );

        executor::block_on(WordDocument::load(&output, &Options::default()))
            .expect("three-paragraph-comment docx must reload");
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 48,
            max_shrink_iters: 3000,
            .. ProptestConfig::default()
        })]

        #[test]
        fn property_named_table_styles_export_reopen_and_preserve_styles_part(
            operations in prop::collection::vec(0u8..4, 1..40),
        ) {
            let input = table_styles_fixture();
            let styles_before = read_zip_entry(&input, "word/styles.xml");
            let commands = operations
                .iter()
                .map(|operation| HeadlessCommand::ApplyTableNamedStyle {
                    table_order_index: 0,
                    style_id: match operation {
                        0 => Some("Alpha".to_string()),
                        1 => Some("Beta".to_string()),
                        2 => Some("Beta Style".to_string()),
                        _ => None,
                    },
                })
                .collect::<Vec<_>>();
            let expected = match operations.last().copied() {
                Some(0) => Some("Alpha"),
                Some(1 | 2) => Some("Beta"),
                _ => None,
            };
            let output = apply_headless_commands(
                &input,
                &HeadlessEditRequest { commands },
            )
            .expect("apply named table styles");

            prop_assert_eq!(
                read_zip_entry(&output, "word/styles.xml"),
                styles_before,
                "applying a named style must not materialize or rewrite styles.xml"
            );
            let reopened = executor::block_on(WordDocument::load(&output, &Options::default()))
                .expect("reload styled table");
            let reopened_style_id = reopened.table_style_id(0).expect("table");
            prop_assert_eq!(reopened_style_id.as_deref(), expected);
            let resolved = reopened
                .table_by_order(0)
                .and_then(|table| table.resolved_style.as_ref());
            prop_assert_eq!(resolved.and_then(|style| style.fill.as_deref()), expected.map(|_| "#AABBCC"));
            prop_assert_eq!(
                resolved
                    .and_then(|style| style.first_row.as_ref())
                    .and_then(|first_row| first_row.fill.as_deref()),
                if expected == Some("Beta") { Some("#112233") } else { None }
            );
        }
    }

    /// One piece of the paragraph the comment tests build: formatted text, or
    /// a run with no text (what `jade-docx-annotations` leaves behind when it
    /// removes a comment reference run, jlsi/jade#402).
    #[derive(Debug, Clone)]
    enum Piece {
        Text {
            text: String,
            bold: bool,
        },
        /// `<w:r/>`
        BareEmptyRun,
        /// `<w:r><w:rPr/></w:r>`
        EmptyRunWithEmptyProps,
        /// `<w:r><w:rPr><w:b/></w:rPr></w:r>`
        EmptyBoldRun,
    }

    fn piece_xml(piece: &Piece) -> String {
        match piece {
            Piece::Text { text, bold } => {
                let props = if *bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
                format!(r#"<w:r>{props}<w:t xml:space="preserve">{text}</w:t></w:r>"#)
            }
            Piece::BareEmptyRun => "<w:r/>".to_string(),
            Piece::EmptyRunWithEmptyProps => "<w:r><w:rPr/></w:r>".to_string(),
            Piece::EmptyBoldRun => "<w:r><w:rPr><w:b/></w:rPr></w:r>".to_string(),
        }
    }

    /// `hello.docx` whose body is one paragraph (`w14:paraId="00000001"`) built from `pieces`.
    fn paragraph_fixture(pieces: &[Piece]) -> Vec<u8> {
        let input = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let runs: String = pieces.iter().map(piece_xml).collect();
        rewrite_zip_entry(&input, "word/document.xml", |bytes| {
            let xml = String::from_utf8(bytes.to_vec()).expect("document.xml utf8");
            let start = xml.find("<w:body>").expect("body") + "<w:body>".len();
            let end = xml.find("<w:sectPr>").expect("sectPr");
            format!(
                r#"{}<w:p w14:paraId="00000001">{runs}</w:p>{}"#,
                &xml[..start],
                &xml[end..]
            )
            .into_bytes()
        })
    }

    /// The first paragraph of `document.xml`: each character with whether it is
    /// bold, and the byte offsets of every `commentRangeStart` / `commentRangeEnd`.
    fn read_formatted_paragraph(docx: &[u8]) -> (Vec<(char, bool)>, Vec<usize>, Vec<usize>) {
        let xml = String::from_utf8(read_zip_entry(docx, "word/document.xml")).expect("utf8");
        let start = xml.find("<w:p ").expect("paragraph");
        let end = start + xml[start..].find("</w:p>").expect("paragraph end");
        let paragraph = &xml[start..end];
        let (mut chars, mut starts, mut ends) = (Vec::new(), Vec::new(), Vec::new());
        let (mut bold, mut offset) = (false, 0usize);
        let mut rest = paragraph;
        while let Some(at) = rest.find('<') {
            rest = &rest[at..];
            let tag_end = rest.find('>').expect("tag end") + 1;
            let tag = &rest[..tag_end];
            rest = &rest[tag_end..];
            if tag.starts_with("<w:r>") || tag.starts_with("<w:r ") || tag == "<w:r/>" {
                bold = false;
            } else if tag.starts_with("<w:b/") || tag.starts_with("<w:b ") {
                bold = true;
            } else if tag.starts_with("<w:t>") || tag.starts_with("<w:t ") {
                let text_end = rest.find("</w:t>").expect("text end");
                for c in rest[..text_end].chars() {
                    chars.push((c, bold));
                }
                offset += rest[..text_end].len();
            } else if tag.starts_with("<w:commentRangeStart") {
                starts.push(offset);
            } else if tag.starts_with("<w:commentRangeEnd") {
                ends.push(offset);
            }
        }
        (chars, starts, ends)
    }

    fn comment_on(start_offset: usize, end_offset: usize) -> HeadlessEditRequest {
        HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddCommentByParaId {
                para_id: "00000001".to_string(),
                start_offset,
                end_offset,
                text: "note".to_string(),
                author: Some("T".to_string()),
                initials: None,
                date: Some("2026-10-01T00:00:00Z".to_string()),
            }],
        }
    }

    fn expected_chars(pieces: &[Piece]) -> Vec<(char, bool)> {
        pieces
            .iter()
            .flat_map(|piece| match piece {
                Piece::Text { text, bold } => text.chars().map(|c| (c, *bold)).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// jlsi/jade#402: an empty `<w:r/>` at the start of the paragraph used to
    /// shift the run formatting by one run, so the plain `a` before the range
    /// became bold.
    #[test]
    fn add_comment_by_para_id_keeps_formatting_next_to_an_empty_run() {
        let pieces = vec![
            Piece::BareEmptyRun,
            Piece::Text {
                text: "a".to_string(),
                bold: true,
            },
            Piece::Text {
                text: "aa".to_string(),
                bold: false,
            },
        ];
        let output = apply_headless_commands(&paragraph_fixture(&pieces), &comment_on(2, 3))
            .expect("add comment");
        let (chars, starts, ends) = read_formatted_paragraph(&output);
        assert_eq!(chars, expected_chars(&pieces));
        assert_eq!((starts, ends), (vec![2], vec![3]));
    }

    fn piece() -> impl Strategy<Value = Piece> {
        prop_oneof![
            4 => (prop::collection::vec(prop::sample::select(vec!["a", "b", "日", "😀"]), 1..4), any::<bool>())
                .prop_map(|(chars, bold)| Piece::Text { text: chars.concat(), bold }),
            1 => Just(Piece::BareEmptyRun),
            1 => Just(Piece::EmptyRunWithEmptyProps),
            1 => Just(Piece::EmptyBoldRun),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Adding a comment never changes the paragraph's text or the formatting
        /// of any character, wherever runs without text sit, and the range
        /// markers land on the requested byte offsets.
        #[test]
        fn add_comment_by_para_id_preserves_text_and_formatting(
            pieces in prop::collection::vec(piece(), 1..7),
            a in any::<prop::sample::Index>(),
            b in any::<prop::sample::Index>(),
        ) {
            let expected = expected_chars(&pieces);
            prop_assume!(!expected.is_empty());
            let text: String = expected.iter().map(|(c, _)| *c).collect();
            let boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
            let (mut start, mut end) = (a.index(boundaries.len()), b.index(boundaries.len()));
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            prop_assume!(start < end);
            let (start, end) = (boundaries[start], boundaries[end]);

            let output = apply_headless_commands(&paragraph_fixture(&pieces), &comment_on(start, end))
                .expect("add comment");
            let (chars, starts, ends) = read_formatted_paragraph(&output);
            prop_assert_eq!(chars, expected, "text or formatting changed");
            prop_assert_eq!((starts, ends), (vec![start], vec![end]));
        }
    }
}
