//! PDFs LibreOffice wrote from the repository's .docx fixtures
//! (`soffice --headless --convert-to pdf`), opened as documents: the text in
//! reading order with its list markers taken off, paragraphs with their
//! indents, headings, tables from their rules with merged cells, and
//! pictures.

#![cfg(feature = "pdf")]

use jade_docx_convert::flow::*;

fn read(name: &str) -> FlowDocument {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/{name}.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    jade_docx_convert::pdf::read_pdf(&bytes).unwrap()
}

fn source(name: &str) -> FlowDocument {
    let bytes = std::fs::read(format!(
        "{}/../../public/fixtures/{name}.docx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    jade_docx_convert::read_docx(&bytes).unwrap()
}

/// How much of `expected` comes back in order in `read`: the longest
/// common subsequence of their characters, white space aside.
fn kept(read: &str, expected: &str) -> f64 {
    let a: Vec<char> = read.chars().filter(|ch| !ch.is_whitespace()).collect();
    let b: Vec<char> = expected.chars().filter(|ch| !ch.is_whitespace()).collect();
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()] as f64 / b.len().max(1) as f64
}

#[test]
fn the_text_comes_back_in_reading_order() {
    for name in ["contract", "header-footer-sections", "image-inline"] {
        let ratio = kept(&read(name).plain_text(), &source(name).plain_text());
        assert!(ratio > 0.97, "{name}: {ratio}");
    }
}

#[test]
fn paragraphs_lists_and_indents_are_worked_out() {
    let document = read("contract");
    let paragraphs = document.paragraphs();
    // The first paragraph's two lines are one paragraph, indented as it was.
    assert!(paragraphs[0].text().starts_with("秘密保持契約書"));
    assert!(paragraphs[0].text().contains("あｓだｓだｓだｓ"));
    assert!(paragraphs[0].indent_left > 2000);
    // Its numbered items are a list, the numbers taken off.
    let items: Vec<&&Paragraph> = paragraphs
        .iter()
        .filter(|paragraph| paragraph.list.is_some())
        .collect();
    assert!(items.len() >= 10);
    assert!(items.iter().all(|item| !item.text().starts_with('(')));
    assert_eq!(items[0].text(), "開示を受けたときに既に保有していた情報");
}

#[test]
fn headings_are_found_by_their_size() {
    let document = read("toc");
    let headings = document
        .paragraphs()
        .iter()
        .filter(|paragraph| matches!(paragraph.kind, ParagraphKind::Heading(_)))
        .count();
    assert!(headings >= 3);
}

#[test]
fn ruled_tables_come_back_with_their_spans() {
    let document = read("table3x2merged");
    let table = document
        .blocks()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("the table");
    let expected = source("table3x2merged");
    let expected = expected
        .blocks()
        .find_map(|block| match block {
            Block::Table(table) => Some(table.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows.len(), expected.rows.len());
    for (row, expected) in table.rows.iter().zip(&expected.rows) {
        let spans: Vec<u32> = row.cells.iter().map(|cell| cell.span).collect();
        let expected: Vec<u32> = expected.cells.iter().map(|cell| cell.span).collect();
        assert_eq!(spans, expected);
    }
    assert!(
        words(&FlowDocument::single(vec![Block::Table(table.clone())]).plain_text())
            .contains(&"Hello".to_string())
    );
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

#[test]
fn pictures_come_back() {
    let document = read("image-inline");
    let images = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter(|inline| matches!(inline, Inline::Image(_)))
        .count();
    assert_eq!(images, 1);
    // The document converts to a .docx that reads again.
    let docx = jade_docx_convert::write_docx(&document).unwrap();
    assert!(jade_docx_convert::read_docx(&docx).is_ok());
}

#[test]
fn a_file_that_is_not_a_pdf_is_refused() {
    assert!(jade_docx_convert::pdf::read_pdf(b"%PDF-1.4 garbage").is_err());
}

/// Word for Mac draws 影付き text twice, the copy a little off: it is read
/// once, through the fonts' ToUnicode maps.
#[test]
fn a_word_for_mac_pdf_reads_shadowed_text_once() {
    let text = read("char-effects-word-mac").plain_text();
    assert!(text.contains("Shadowed"), "{text:?}");
    assert!(!text.contains("SShh"), "{text:?}");
    assert!(
        text.contains("Embossed") && !text.contains("EEEmm"),
        "{text:?}"
    );
    assert!(text.contains("Imprinted"), "{text:?}");
}
