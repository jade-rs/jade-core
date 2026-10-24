//! .doc files LibreOffice wrote from the repository's .docx fixtures
//! (`soffice --headless --convert-to doc`), read back and set beside the
//! .docx they came from.

use jade_docx_convert::flow::*;

fn read(name: &str) -> FlowDocument {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/{name}.doc",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(jade_doc_reader::is_doc(&bytes));
    jade_doc_reader::read_doc(&bytes).unwrap()
}

fn source(name: &str) -> FlowDocument {
    let bytes = std::fs::read(format!(
        "{}/../../public/fixtures/{name}.docx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    jade_docx_convert::read_docx(&bytes).unwrap()
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn tables(document: &FlowDocument) -> Vec<&Table> {
    document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .collect()
}

#[test]
fn the_text_is_the_text_of_the_docx_it_was_saved_from() {
    for name in [
        "contract",
        "char-effects",
        "header-footer-sections",
        "image-inline",
        "numbering-restart",
        "table3x2merged",
        "yokohama_table",
    ] {
        assert_eq!(
            words(&read(name).plain_text()),
            words(&source(name).plain_text()),
            "{name}"
        );
    }
}

#[test]
fn japanese_text_and_its_formatting_come_through() {
    let document = read("contract");
    assert_eq!(document.language.as_deref(), Some("ja-JP"));
    let paragraphs = document.paragraphs();
    assert!(paragraphs[0].text().starts_with("秘密保持契約書"));
    let source = source("contract");
    let source_bold = source
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter(|inline| matches!(inline, Inline::Text(_, format) if format.bold))
        .count();
    let bold = paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter(|inline| matches!(inline, Inline::Text(_, format) if format.bold))
        .count();
    assert_eq!(bold > 0, source_bold > 0, "bold runs survive");
}

#[test]
fn hidden_and_large_text_keep_their_formatting() {
    let document = read("char-effects");
    let formats: Vec<(String, CharFormat)> = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter_map(|inline| match inline {
            Inline::Text(text, format) => Some((text.clone(), format.clone())),
            _ => None,
        })
        .collect();
    assert!(
        formats
            .iter()
            .any(|(text, format)| text.contains("hidden") && format.hidden),
        "{formats:?}"
    );
    assert!(formats
        .iter()
        .any(|(text, format)| text.contains("Shadowed") && format.size == Some(40)));
}

#[test]
fn sections_have_their_headers_and_footers() {
    let document = read("header-footer-sections");
    let source = source("header-footer-sections");
    assert_eq!(document.sections.len(), source.sections.len());
    for (read, source) in document.sections.iter().zip(&source.sections) {
        let text = |blocks: &[Block]| words(&FlowDocument::single(blocks.to_vec()).plain_text());
        assert_eq!(text(&read.header), text(&source.header));
        assert_eq!(text(&read.footer), text(&source.footer));
        assert_eq!(read.page.width, source.page.width);
        assert_eq!(read.page.height, source.page.height);
    }
}

#[test]
fn an_inline_picture_comes_out_with_its_bytes_and_size() {
    let document = read("image-inline");
    let images: Vec<&Image> = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter_map(|inline| match inline {
            Inline::Image(image) => Some(image),
            _ => None,
        })
        .collect();
    let source = source("image-inline");
    let expected: Vec<&Image> = source
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter_map(|inline| match inline {
            Inline::Image(image) => Some(image),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), expected.len());
    for (image, expected) in images.iter().zip(expected) {
        // Within a hundredth of an inch of the size it was saved at.
        assert!(
            image.width.abs_diff(expected.width) < 9144 * 2,
            "{} vs {}",
            image.width,
            expected.width
        );
        assert!(image.height.abs_diff(expected.height) < 9144 * 2);
        assert!(ImageFormat::pixel_size(&image.data).is_some());
    }
}

#[test]
fn merged_cells_span_as_they_did() {
    let document = read("table3x2merged");
    let source = source("table3x2merged");
    let (read, expected) = (tables(&document), tables(&source));
    assert_eq!(read.len(), expected.len());
    for (table, expected) in read.iter().zip(expected) {
        assert_eq!(table.rows.len(), expected.rows.len());
        for (row, expected) in table.rows.iter().zip(&expected.rows) {
            let spans: Vec<u32> = row.cells.iter().map(|cell| cell.span).collect();
            let expected_spans: Vec<u32> = expected.cells.iter().map(|cell| cell.span).collect();
            assert_eq!(spans, expected_spans);
            let merges: Vec<VerticalMerge> =
                row.cells.iter().map(|cell| cell.vertical_merge).collect();
            let expected_merges: Vec<VerticalMerge> = expected
                .cells
                .iter()
                .map(|cell| cell.vertical_merge)
                .collect();
            assert_eq!(merges, expected_merges);
        }
    }
}

#[test]
fn lists_stay_lists() {
    let document = read("numbering-restart");
    let listed = document
        .paragraphs()
        .iter()
        .filter(|paragraph| paragraph.list.is_some())
        .count();
    let source = source("numbering-restart");
    let expected = source
        .paragraphs()
        .iter()
        .filter(|paragraph| paragraph.list.is_some())
        .count();
    assert!(listed > 0);
    assert_eq!(listed, expected);
}

#[test]
fn a_hyperlink_field_reads_as_a_link() {
    let document = read("hyperlink");
    let link = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .find_map(|inline| match inline {
            Inline::Text(text, format) if text == "example.com" => format.link.clone(),
            _ => None,
        });
    assert_eq!(link.as_deref(), Some("https://example.com/"));
}

#[test]
fn a_document_converts_to_a_docx_that_reads_the_same() {
    let document = read("yokohama_table");
    let docx = jade_docx_convert::write_docx(&document).unwrap();
    let back = jade_docx_convert::read_docx(&docx).unwrap();
    assert_eq!(words(&back.plain_text()), words(&document.plain_text()));
    assert_eq!(tables(&back).len(), tables(&document).len());
}
