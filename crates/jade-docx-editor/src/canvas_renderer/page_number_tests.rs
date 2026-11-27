//! `PAGE` numbering across sections on a recording painter: each section's
//! `w:pgNumType/@w:start` restarts only that section.

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::Op;
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A package of sections `(properties, pages)`: each section fills `pages`
/// pages, separated by page breaks, and ends in a next-page break. All share
/// the footer "F" followed by a PAGE field. `properties` is appended to the
/// section's `sectPr`.
fn package(sections: &[(&str, usize)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let geometry = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let section = |extra: &str| {
        format!(
            r#"<w:sectPr><w:footerReference w:type="default" r:id="rF"/><w:type w:val="nextPage"/>{geometry}{extra}</w:sectPr>"#
        )
    };
    let mut body = String::new();
    let last = sections.len() - 1;
    for (index, (extra, pages)) in sections.iter().enumerate() {
        for page in 0..*pages {
            if page > 0 {
                body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
            }
            body.push_str(&format!("<w:p><w:r><w:t>s{index}</w:t></w:r></w:p>"));
        }
        if index < last {
            body.push_str(&format!("<w:p><w:pPr>{}</w:pPr></w:p>", section(extra)));
        } else {
            body.push_str(&section(extra));
        }
    }
    let document_xml =
        format!(r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#);
    let footer = format!(
        r#"<w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p><w:r><w:t>F</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#
    );
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rF" Type="{REL}/footer" Target="footer1.xml"/></Relationships>"#
            ),
        ),
        ("word/document.xml", document_xml),
        ("word/footer1.xml", footer),
    ];
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for (path, content) in parts {
            zip.start_file(path, options).expect("start entry");
            zip.write_all(content.as_bytes()).expect("write entry");
        }
        zip.finish().expect("finish package");
    }
    cursor.into_inner()
}

/// The footer numbers painted, in page order: the digits after each "F".
fn footer_numbers(data: &[u8]) -> Vec<String> {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.set_section_header_footers(word_doc.section_header_footers());
    renderer
        .render(document, &Options::default())
        .expect("render");
    let text: String = renderer
        .host
        .painter()
        .inner()
        .ops()
        .iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    text.split('F')
        .skip(1)
        .map(|rest| rest.chars().take_while(char::is_ascii_digit).collect())
        .collect()
}

/// A restart on the last section, the document's default `sectPr`, applies
/// to that section alone: the sections before it number from 1.
#[test]
fn a_restart_in_the_last_section_leaves_earlier_pages_numbered_from_one() {
    let numbers = footer_numbers(&package(&[
        ("", 1),
        ("", 1),
        (r#"<w:pgNumType w:start="5"/>"#, 1),
    ]));
    assert_eq!(numbers, ["1", "2", "5"]);
}

/// A restart in a middle section carries on into a section without one.
#[test]
fn a_section_without_a_start_continues_from_the_previous_one() {
    let numbers = footer_numbers(&package(&[
        ("", 1),
        (r#"<w:pgNumType w:start="10"/>"#, 1),
        ("", 1),
    ]));
    assert_eq!(numbers, ["1", "10", "11"]);
}

/// A single section's start numbers its pages from there.
#[test]
fn a_single_sections_start_numbers_its_pages() {
    let numbers = footer_numbers(&package(&[(r#"<w:pgNumType w:start="5"/>"#, 2)]));
    assert_eq!(numbers, ["5", "6"]);
}

/// A first section's start carries on through sections without one.
#[test]
fn a_first_sections_start_carries_on_through_later_sections() {
    let numbers = footer_numbers(&package(&[
        (r#"<w:pgNumType w:start="3"/>"#, 1),
        ("", 1),
        ("", 1),
    ]));
    assert_eq!(numbers, ["3", "4", "5"]);
}
