//! Documents written as `.doc` read back with the text, tables, sections and
//! pictures they were written with.

use jade_docx_convert::flow::{Block, FlowDocument, Inline};

fn fixture(name: &str) -> FlowDocument {
    let path = format!(
        "{}/../../public/fixtures/{name}.docx",
        env!("CARGO_MANIFEST_DIR")
    );
    jade_docx_convert::read_docx(&std::fs::read(path).unwrap()).unwrap()
}

fn words(document: &FlowDocument) -> Vec<String> {
    document
        .plain_text()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn pictures(document: &FlowDocument) -> usize {
    document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter(|inline| matches!(inline, Inline::Image(_)))
        .count()
}

fn tables(document: &FlowDocument) -> usize {
    document
        .blocks()
        .filter(|block| matches!(block, Block::Table(_)))
        .count()
}

#[test]
fn fixtures_survive_the_round_trip() {
    for name in [
        "contract",
        "hyperlink",
        "image-inline",
        "table3x2merged",
        "yokohama_table",
        "header-footer-sections",
        "numbering-restart",
        "toc-direct-outline-levels",
    ] {
        let original = fixture(name);
        let bytes = jade_doc_writer::write_doc(&original).unwrap();
        let read =
            jade_doc_reader::read_doc(&bytes).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(words(&read), words(&original), "{name}: the text");
        assert_eq!(
            read.sections.len(),
            original.sections.len(),
            "{name}: the sections"
        );
        assert_eq!(tables(&read), tables(&original), "{name}: the tables");
        assert_eq!(pictures(&read), pictures(&original), "{name}: the pictures");
        let lists = |document: &FlowDocument| -> Vec<_> {
            document
                .paragraphs()
                .iter()
                .map(|paragraph| paragraph.list)
                .collect()
        };
        assert_eq!(lists(&read), lists(&original), "{name}: the lists");
        for (written, back) in original.sections.iter().zip(&read.sections) {
            assert_eq!(written.page.width, back.page.width, "{name}: the page");
            assert_eq!(written.page.height, back.page.height, "{name}: the page");
            let text = |blocks: &[Block]| {
                FlowDocument::single(blocks.to_vec())
                    .plain_text()
                    .trim()
                    .to_string()
            };
            assert_eq!(
                text(&written.header),
                text(&back.header),
                "{name}: the header"
            );
            assert_eq!(
                text(&written.footer),
                text(&back.footer),
                "{name}: the footer"
            );
        }
    }
}
