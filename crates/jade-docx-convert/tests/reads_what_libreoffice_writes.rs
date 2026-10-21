//! RTF, ODT and HTML that LibreOffice wrote from the repository's .docx
//! fixtures (`soffice --headless --convert-to rtf|odt|html`), read and set
//! beside the .docx each came from: the words, the tables and their spans,
//! the headers, the lists and the pictures should all be there.

use jade_docx_convert::flow::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn source(name: &str) -> FlowDocument {
    let bytes = std::fs::read(format!(
        "{}/../../public/fixtures/{name}.docx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    jade_docx_convert::read_docx(&bytes).unwrap()
}

fn read(name: &str, format: &str) -> FlowDocument {
    let bytes = fixture(&format!("{name}.{format}"));
    match format {
        "rtf" => jade_docx_convert::rtf::read_rtf(&bytes).unwrap(),
        "odt" => jade_docx_convert::odt::read_odt(&bytes).unwrap(),
        "html" => jade_docx_convert::html::read_html(&bytes, &|path| {
            std::fs::read(format!(
                "{}/tests/fixtures/{path}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .ok()
        }),
        _ => unreachable!(),
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn tables(document: &FlowDocument) -> Vec<&Table> {
    document
        .blocks()
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .collect()
}

fn images(document: &FlowDocument) -> usize {
    document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter(|inline| matches!(inline, Inline::Image(_)))
        .count()
}

const NAMES: [&str; 5] = [
    "contract",
    "table3x2merged",
    "header-footer-sections",
    "image-inline",
    "numbering-restart",
];

#[test]
fn the_words_are_the_words_of_the_docx() {
    for format in ["rtf", "odt", "html"] {
        for name in NAMES {
            let read = words(&read(name, format).plain_text());
            let expected = words(&source(name).plain_text());
            assert_eq!(read, expected, "{name}.{format}");
        }
    }
}

#[test]
fn tables_keep_their_rows_and_spans() {
    for format in ["rtf", "odt", "html"] {
        let document = read("table3x2merged", format);
        let source = source("table3x2merged");
        let (read, expected) = (tables(&document), tables(&source));
        assert_eq!(read.len(), expected.len(), "{format}");
        for (table, expected) in read.iter().zip(expected) {
            assert_eq!(table.rows.len(), expected.rows.len(), "{format}");
            for (row, expected) in table.rows.iter().zip(&expected.rows) {
                let spans: Vec<(u32, VerticalMerge)> = row
                    .cells
                    .iter()
                    .map(|cell| (cell.span, cell.vertical_merge))
                    .collect();
                let expected: Vec<(u32, VerticalMerge)> = expected
                    .cells
                    .iter()
                    .map(|cell| (cell.span, cell.vertical_merge))
                    .collect();
                assert_eq!(spans, expected, "{format}");
            }
        }
    }
}

#[test]
fn pictures_come_through() {
    for format in ["rtf", "odt", "html"] {
        assert_eq!(
            images(&read("image-inline", format)),
            images(&source("image-inline")),
            "{format}"
        );
    }
}

#[test]
fn headers_and_footers_come_through() {
    for format in ["rtf", "odt", "html"] {
        let document = read("header-footer-sections", format);
        let source = source("header-footer-sections");
        let text = |blocks: &[Block]| words(&FlowDocument::single(blocks.to_vec()).plain_text());
        assert_eq!(
            text(&document.sections[0].header),
            text(&source.sections[0].header),
            "{format}"
        );
        assert_eq!(
            text(&document.sections[0].footer),
            text(&source.sections[0].footer),
            "{format}"
        );
    }
}

#[test]
fn lists_stay_lists() {
    let expected = source("numbering-restart")
        .paragraphs()
        .iter()
        .filter(|paragraph| paragraph.list.is_some())
        .count();
    for format in ["rtf", "odt", "html"] {
        let listed = read("numbering-restart", format)
            .paragraphs()
            .iter()
            .filter(|paragraph| paragraph.list.is_some())
            .count();
        assert!(listed > 0, "{format}");
        assert!(
            listed.abs_diff(expected) <= expected / 4,
            "{format}: {listed} of {expected}"
        );
    }
}
