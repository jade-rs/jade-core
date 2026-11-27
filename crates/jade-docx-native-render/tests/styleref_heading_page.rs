//! A header's STYLEREF shows a heading's text on the heading's own page, and
//! on the pages after it until the next heading in the style. Typing in the
//! heading, bounded to its page, leaves those pages stale or right.

mod common;

use common::{open_bytes, Doc};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const FILLER: &str = "The quick brown fox jumps over the lazy dog while the band \
     redraw keeps every other pixel of the page exactly as it was painted.";

/// Eight chapters, each a `heading 1` paragraph and 18 paragraphs of text,
/// under a header that shows `STYLEREF "heading 1"`.
fn styleref_docx() -> Vec<u8> {
    use std::io::Write;
    let mut body = String::new();
    for chapter in 0..8 {
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter {chapter}</w:t></w:r></w:p>"#
        ));
        for index in 0..18 {
            body.push_str(&format!(
                "<w:p><w:r><w:t>{chapter}.{index} {FILLER}</w:t></w:r></w:p>"
            ));
        }
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr><w:headerReference w:type="default" r:id="rId2"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="{W}"><w:p><w:r><w:t xml:space="preserve">Now reading: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; "><w:r><w:t>Chapter 0</w:t></w:r></w:fldSimple></w:p></w:hdr>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style></w:styles>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", document),
            ("word/header1.xml", header),
            ("word/styles.xml", styles),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    bytes
}

fn page_of(document: &mut Doc, paragraph: usize) -> usize {
    let rect = document.position_rect(paragraph, 0).unwrap();
    document.page_and_offset(rect.document_y).0
}

/// Every page is stale or as the reference draws it.
fn assert_stale_or_right(name: &str, document: &Doc, reference: &Doc) {
    for page in 0..reference.page_count() {
        assert!(
            document.is_page_stale(page)
                || document.page_pixels(page) == reference.page_pixels(page),
            "{name}: page {page} wrong and not stale"
        );
    }
}

#[test]
fn editing_a_heading_a_styleref_shows_leaves_its_pages_stale_or_right() {
    let bytes = styleref_docx();
    let last = open_bytes(&bytes).paragraph_count() - 1;
    // What runs before the edit: nothing, an unbounded pass on the last page,
    // a bounded pass on page 1.
    for before in 0..3 {
        let mut document = open_bytes(&bytes);
        let mut reference = open_bytes(&bytes);
        let pages = document.page_count();
        match before {
            1 => {
                document.insert_text(last, 0, "a", pages).unwrap();
                reference.insert_text(last, 0, "a", usize::MAX).unwrap();
            }
            2 => {
                document.insert_text(1, 0, "a", 1).unwrap();
                reference.insert_text(1, 0, "a", usize::MAX).unwrap();
            }
            _ => {}
        }
        for heading in [0, 19] {
            let page = page_of(&mut document, heading);
            document.insert_text(heading, 0, "Q", page + 1).unwrap();
            reference.insert_text(heading, 0, "Q", usize::MAX).unwrap();
            reference.repaint_all().unwrap();
            let name = format!("before {before}, heading {heading} on page {page}");
            assert!(
                document.is_page_stale(page) || page == 0 && heading == 0,
                "{name}"
            );
            assert_stale_or_right(&name, &document, &reference);
            document.paint_stale_pages(0..usize::MAX).unwrap();
            for page in 0..reference.page_count() {
                assert_eq!(
                    document.page_pixels(page),
                    reference.page_pixels(page),
                    "{name}: page {page} after the stale paint"
                );
            }
        }
    }
}

/// A paragraph no STYLEREF looks up marks nothing.
#[test]
fn editing_a_body_paragraph_marks_nothing() {
    let mut document = open_bytes(&styleref_docx());
    document.insert_text(1, 0, "a", 1).unwrap();
    assert!(!document.has_stale_pages());
}
