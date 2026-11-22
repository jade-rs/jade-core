//! Page chrome on a page shared by two sections: Word takes a page's header,
//! footer and page borders from the section at the top of the page, so a
//! section starting after a continuous break first shows its own on the
//! following page.

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A header or footer part with one paragraph: `text`, then a PAGE field.
fn story(tag: &str, text: &str) -> String {
    format!(
        r#"<w:{tag} xmlns:w="{W}" xmlns:r="{R}"><w:p><w:r><w:t>{text}</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:{tag}>"#
    )
}

/// A package of two sections: the first with header "HA", footer "FA" and a
/// red page border, the second, starting with `break_type`, with header
/// "HB", first-page header "HB-first" (shown only under `w:titlePg`), footer
/// "FB" and a blue page border. Every story ends in a PAGE field. `extra_a`
/// and `extra_b` are appended to the sections' properties.
fn package(break_type: &str, body_a: &str, body_b: &str, extra_a: &str, extra_b: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let geometry = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let border = |color: &str| {
        format!(
            r#"<w:pgBorders w:offsetFrom="text"><w:top w:val="single" w:sz="8" w:space="4" w:color="{color}"/><w:bottom w:val="single" w:sz="8" w:space="4" w:color="{color}"/></w:pgBorders>"#
        )
    };
    let section = |suffix: &str, color: &str, extra: &str, break_type: &str| {
        let first = if suffix == "B" {
            r#"<w:headerReference w:type="first" r:id="rHB1"/>"#
        } else {
            ""
        };
        format!(
            r#"<w:sectPr>{first}<w:headerReference w:type="default" r:id="rH{suffix}"/><w:footerReference w:type="default" r:id="rF{suffix}"/><w:type w:val="{break_type}"/>{geometry}{}{extra}</w:sectPr>"#,
            border(color)
        )
    };
    let document_xml = format!(
        r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body_a}<w:p><w:pPr>{}</w:pPr></w:p>{body_b}{}</w:body></w:document>"#,
        section("A", "FF0000", extra_a, "nextPage"),
        section("B", "0000FF", extra_b, break_type),
    );
    let mut rels = String::new();
    for (id, kind, target) in [
        ("rHA", "header", "header1.xml"),
        ("rHB", "header", "header2.xml"),
        ("rHB1", "header", "header3.xml"),
        ("rFA", "footer", "footer1.xml"),
        ("rFB", "footer", "footer2.xml"),
    ] {
        rels.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#
        ));
    }
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
    );
    let content_types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/header2.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/header3.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/footer2.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;
    let parts = [
        ("[Content_Types].xml", content_types.to_string()),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        ("word/_rels/document.xml.rels", rels),
        ("word/document.xml", document_xml),
        ("word/header1.xml", story("hdr", "HA")),
        ("word/header2.xml", story("hdr", "HB")),
        ("word/header3.xml", story("hdr", "HB-first")),
        ("word/footer1.xml", story("ftr", "FA")),
        ("word/footer2.xml", story("ftr", "FB")),
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

fn paragraphs(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("<w:p><w:r><w:t>{prefix}{index}</w:t></w:r></w:p>"))
        .collect()
}

fn render(data: &[u8]) -> (CanvasRenderer<RecordingPainter>, Vec<Op>) {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.set_section_header_footers(word_doc.section_header_footers());
    renderer
        .render(document, &Options::default())
        .expect("render");
    let ops = renderer.host.painter().inner().ops();
    (renderer, ops)
}

/// The painted stream condensed: text runs joined, paragraph marks dropped,
/// and the page border colours as `[#RRGGBB]`.
fn stream(ops: &[Op]) -> String {
    let mut out = String::new();
    for op in ops {
        match op {
            Op::FillText(text, _, _) if text != "\u{21b5}" => out.push_str(text),
            Op::SetStrokeStyle(color) if color != "#888888" => {
                out.push_str(&format!("[{color}]"));
            }
            _ => {}
        }
    }
    out
}

/// Section A's three paragraphs, then 80 of section B: B starts partway
/// down page 1 and runs onto pages 2 and 3.
fn three_pages(break_type: &str, extra_a: &str, extra_b: &str) -> String {
    let data = package(
        break_type,
        &paragraphs("a", 3),
        &paragraphs("b", 80),
        extra_a,
        extra_b,
    );
    let (_, ops) = render(&data);
    stream(&ops)
}

/// The painted stream of each page, split at the page border's top edge
/// that starts every page.
fn pages(stream: &str) -> Vec<String> {
    let mut pages: Vec<String> = Vec::new();
    for (index, piece) in stream.split("[#").enumerate() {
        if index == 0 {
            continue;
        }
        // Each page paints its top and bottom border edges back to back.
        if index % 2 == 1 {
            pages.push(format!("[#{piece}"));
        } else {
            pages
                .last_mut()
                .expect("page")
                .push_str(&format!("[#{piece}"));
        }
    }
    pages
}

fn chrome(page: &str) -> (String, String, String) {
    let border = page[2..8].to_string();
    let body_start = page.find(['a', 'b']).expect("body text");
    let header = page[page.rfind(']').unwrap() + 1..body_start].to_string();
    let last_body = page.rfind(['a', 'b']).unwrap();
    let digits_end = page[last_body + 1..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(page.len(), |end| last_body + 1 + end);
    let footer = page[digits_end..].to_string();
    (border, header, footer)
}

#[test]
fn a_continuous_section_shows_its_chrome_from_the_following_page() {
    let stream = three_pages("continuous", "", "");
    let pages = pages(&stream);
    assert_eq!(pages.len(), 3, "{stream}");
    assert!(pages[0].contains("a2b0"), "B starts on page 1: {stream}");
    // Page 1 is A's top to bottom, though B owns its lower part.
    assert_eq!(
        chrome(&pages[0]),
        ("FF0000".into(), "HA1".into(), "FA1".into()),
        "{stream}"
    );
    assert_eq!(
        chrome(&pages[1]),
        ("0000FF".into(), "HB2".into(), "FB2".into()),
        "{stream}"
    );
    assert_eq!(
        chrome(&pages[2]),
        ("0000FF".into(), "HB3".into(), "FB3".into()),
        "{stream}"
    );
}

#[test]
fn a_next_page_section_shows_its_chrome_on_its_first_page() {
    let stream = three_pages("nextPage", "", "");
    let pages = pages(&stream);
    assert!(pages.len() >= 2, "{stream}");
    assert!(pages[1].contains("b0"), "B starts on page 2: {stream}");
    assert_eq!(
        chrome(&pages[0]),
        ("FF0000".into(), "HA1".into(), "FA1".into()),
        "{stream}"
    );
    assert_eq!(
        chrome(&pages[1]),
        ("0000FF".into(), "HB2".into(), "FB2".into()),
        "{stream}"
    );
}

/// B's `w:titlePg` first-page header and its numbering restart both land on
/// page 2, the first page B supplies chrome for.
#[test]
fn a_continuous_sections_first_page_header_and_restart_land_on_the_following_page() {
    let stream = three_pages(
        "continuous",
        r#"<w:pgNumType w:start="1"/>"#,
        r#"<w:titlePg/><w:pgNumType w:start="7"/>"#,
    );
    let pages = pages(&stream);
    assert_eq!(pages.len(), 3, "{stream}");
    assert_eq!(
        chrome(&pages[0]),
        ("FF0000".into(), "HA1".into(), "FA1".into()),
        "{stream}"
    );
    // No first-page footer is defined, so B's first page has none.
    assert_eq!(
        chrome(&pages[1]),
        ("0000FF".into(), "HB-first7".into(), String::new()),
        "{stream}"
    );
    assert_eq!(
        chrome(&pages[2]),
        ("0000FF".into(), "HB8".into(), "FB8".into()),
        "{stream}"
    );
}

/// The renderer-level view: page 1 belongs to B for body purposes but takes
/// its chrome from A.
#[test]
fn page_chrome_section_steps_back_over_a_mid_page_start() {
    let data = package(
        "continuous",
        &paragraphs("a", 3),
        &paragraphs("b", 80),
        "",
        "",
    );
    let (renderer, _) = render(&data);
    assert_eq!(&renderer.page_section_indices[..3], &[1, 1, 1]);
    assert_eq!(renderer.section_index_for_page(1), 0);
    assert_eq!(renderer.section_index_for_page(2), 1);
    assert_eq!(renderer.first_page_of_section_index(0), Some(1));
    assert_eq!(renderer.first_page_of_section_index(1), Some(2));
    assert!(renderer.is_first_page_of_section(2));
}

/// A second pass over the retained pagination paints the same chrome.
#[test]
fn a_repeat_render_keeps_the_shared_pages_chrome() {
    let data = package(
        "continuous",
        &paragraphs("a", 3),
        &paragraphs("b", 80),
        "",
        "",
    );
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.set_section_header_footers(word_doc.section_header_footers());
    renderer
        .render(document, &Options::default())
        .expect("first render");
    let first = stream(&renderer.host.painter().inner().ops());
    renderer.host.painter().inner().clear();
    renderer
        .render(document, &Options::default())
        .expect("second render");
    let second = stream(&renderer.host.painter().inner().ops());
    assert_eq!(second, first);
    assert_eq!(renderer.section_index_for_page(1), 0);
}
