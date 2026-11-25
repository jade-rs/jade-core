//! A footer's NUMPAGES shows the final page count on every page, as Word
//! shows it, even though a single pass draws page 1's footer before it
//! reaches the last page — at open, and after an edit changes the count.
//!
//! Pages are compared with the same document whose footer spells out the
//! expected text in the same runs.

mod common;

use common::repo_path;
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_native_render::document::surface_document_renders;
use jade_docx_native_render::{FontFace, NativeDocument};
use std::io::{Cursor, Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn faces() -> Vec<FontFace> {
    let fonts = repo_path("fonts/Noto_Sans_JP/static");
    [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ]
    .into_iter()
    .map(|(weight, file)| FontFace {
        family: "Noto Sans JP".into(),
        weight,
        italic: false,
        path: fonts.join(file),
        index: 0,
    })
    .collect()
}

fn field(code: &str, saved: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{saved}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

/// `PAGE of NUMPAGES`, with the stale saved results a document can carry.
fn field_footer() -> String {
    footer(&format!(
        r#"{}<w:r><w:t xml:space="preserve"> of </w:t></w:r>{}"#,
        field("PAGE", "1"),
        field("NUMPAGES", "1"),
    ))
}

/// The text `page of total` in the same three runs.
fn literal_footer(page: usize, total: usize) -> String {
    footer(&format!(
        r#"<w:r><w:t>{page}</w:t></w:r><w:r><w:t xml:space="preserve"> of </w:t></w:r><w:r><w:t>{total}</w:t></w:r>"#
    ))
}

fn footer(runs: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p><w:pPr><w:jc w:val="center"/></w:pPr>{runs}</w:p></w:ftr>"#
    )
}

/// Three pages, each starting with a page break before its paragraph; the
/// last paragraph shares the third page.
fn three_pages(footer: &str) -> Vec<u8> {
    let body: String = ["One", "Two", "Three", "Four"]
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let break_before = if index == 0 || index == 3 {
                ""
            } else {
                "<w:pPr><w:pageBreakBefore/></w:pPr>"
            };
            format!("<w:p>{break_before}<w:r><w:t>Page {text}</w:t></w:r></w:p>")
        })
        .collect();
    package(&body, "", footer, None)
}

/// The section properties every section of [`package`] shares; `{SECT}` in
/// a body is a section break of that kind.
fn section_properties(extra: &str, header: bool) -> String {
    let header = if header {
        r#"<w:headerReference w:type="default" r:id="rIdHeader"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:sectPr>{header}<w:footerReference w:type="default" r:id="rIdFooter"/>{extra}<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#
    )
}

/// A package with `body`, whose last section takes `last_extra` section
/// properties, a footer and, optionally, a header.
fn package(body: &str, last_extra: &str, footer: &str, header: Option<&str>) -> Vec<u8> {
    let body = body.replace(
        "{SECT}",
        &section_properties(r#"<w:type w:val="continuous"/>"#, header.is_some()),
    );
    let last = section_properties(last_extra, header.is_some());
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}{last}</w:body></w:document>"#
    );
    let (header_override, header_relationship) = if header.is_some() {
        (
            r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#,
            r#"<Relationship Id="rIdHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
        )
    } else {
        ("", "")
    };
    let mut parts = vec![
        (
            "[Content_Types].xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>{header_override}</Types>"#
            ),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>{header_relationship}</Relationships>"#
            ),
        ),
        ("word/document.xml", document),
        ("word/footer1.xml", footer.to_string()),
    ];
    if let Some(header) = header {
        parts.push(("word/header1.xml", header.to_string()));
    }
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in parts {
            zip.start_file(name, options).expect("zip entry");
            zip.write_all(content.as_bytes()).expect("zip write");
        }
        zip.finish().expect("zip");
    }
    bytes
}

/// `docx` with its footer part replaced by `footer`.
fn with_footer(docx: &[u8], footer: &str) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(Cursor::new(docx)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut replaced = false;
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().unwrap().into_owned();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name.starts_with("word/footer") && name.ends_with(".xml") {
            data = footer.as_bytes().to_vec();
            replaced = true;
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    assert!(replaced, "the document keeps its footer part");
    output.finish().unwrap().into_inner()
}

fn open(bytes: &[u8], window: Option<std::ops::Range<usize>>) -> NativeDocument {
    let faces = faces();
    match window {
        Some(window) => NativeDocument::open_with_font_faces_and_paint_window(
            bytes, 816.0, 1056.0, &faces, window,
        ),
        None => NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces),
    }
    .unwrap()
}

/// Asserts that every page of `document` matches the same document with
/// `page of total` spelled out in its footer.
fn assert_footers_read_of(document: &NativeDocument, docx: &[u8], total: usize) {
    assert_eq!(document.page_count(), total);
    for page in 0..total {
        let expected = open(&with_footer(docx, &literal_footer(page + 1, total)), None);
        assert_eq!(expected.page_count(), total);
        assert!(
            document.page_pixels(page) == expected.page_pixels(page),
            "page {} does not read \"{} of {total}\"",
            page + 1,
            page + 1,
        );
    }
}

#[test]
fn literal_footers_differ_by_their_text() {
    // The comparison below would pass vacuously if the footer were not drawn.
    let docx = three_pages(&field_footer());
    let a = open(&with_footer(&docx, &literal_footer(1, 3)), None);
    let b = open(&with_footer(&docx, &literal_footer(1, 1)), None);
    assert!(a.page_pixels(0) != b.page_pixels(0));
}

#[test]
fn numpages_in_a_footer_shows_the_final_count_from_the_first_paint() {
    let docx = three_pages(&field_footer());
    let document = open(&docx, None);
    assert_footers_read_of(&document, &docx, 3);

    // A shell drawing only the first page shows the same.
    let windowed = open(&docx, Some(0..1));
    let expected = open(&with_footer(&docx, &literal_footer(1, 3)), None);
    assert!(windowed.page_pixels(0) == expected.page_pixels(0));
}

#[test]
fn numpages_follows_an_edit_that_adds_a_page_once_stale_pages_are_drawn() {
    for window in [None, Some(0..1)] {
        let mut document = open(&three_pages(&field_footer()), window.clone());
        assert_eq!(document.page_count(), 3);
        let page_break: HeadlessCommand = serde_json::from_value(serde_json::json!({
            "type": "set_paragraph_pagination",
            "start_paragraph": 3, "end_paragraph": 3, "page_break_before": true,
        }))
        .unwrap();
        document.apply_command(page_break, false).unwrap();
        assert_eq!(document.page_count(), 4, "window {window:?}");
        drain(&mut document);
        let edited = document.export_docx().unwrap();
        assert_footers_read_of(&document, &edited, 4);
    }
}

/// Draws every stale page, as a shell's idle loop does, and returns how many
/// rounds that took.
fn drain(document: &mut NativeDocument) -> usize {
    let mut rounds = 0;
    while document.has_stale_pages() {
        rounds += 1;
        assert!(rounds <= 4, "stale pages keep coming back");
        let pages = document.page_count();
        assert!(document.paint_stale_pages(0..pages).unwrap());
    }
    rounds
}

#[test]
fn sectionpages_on_a_page_two_continuous_sections_share_settles() {
    let filler = "The quick brown fox jumps over the lazy dog, and the second \
        section starts partway down a page the first one also fills.";
    let paragraphs = |label: &str| -> String {
        (0..40)
            .map(|index| format!("<w:p><w:r><w:t>{label} {index} {filler}</w:t></w:r></w:p>"))
            .collect()
    };
    let body = format!(
        "{}<w:p><w:pPr>{{SECT}}</w:pPr></w:p>{}",
        paragraphs("First"),
        paragraphs("Second"),
    );
    let section_pages = format!(
        r#"<w:r><w:t xml:space="preserve">Section page </w:t></w:r>{}<w:r><w:t xml:space="preserve"> of </w:t></w:r>{}"#,
        field("PAGE", "1"),
        field("SECTIONPAGES", "1"),
    );
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="{W}" xmlns:r="{R}"><w:p>{section_pages}</w:p></w:hdr>"#
    );
    // The second section starts partway down the page the first ends on.
    let bytes = package(
        &body,
        r#"<w:type w:val="continuous"/>"#,
        &footer(&section_pages),
        Some(&header),
    );
    for window in [None, Some(0..1)] {
        let before = surface_document_renders();
        let mut document = open(&bytes, window.clone());
        assert!(document.page_count() >= 3, "a page is shared");
        // The first pass learns the totals; one more draws them.
        assert!(
            surface_document_renders() - before <= 2,
            "window {window:?}: {} renders at open",
            surface_document_renders() - before
        );
        // Drawing the stale pages settles them: they do not come back.
        assert!(drain(&mut document) <= 1, "window {window:?}");
    }
}

#[test]
fn numpages_follows_typing_that_runs_onto_a_new_page_past_the_bound() {
    for window in [None, Some(0..3)] {
        let mut document = open(&three_pages(&field_footer()), window.clone());
        assert_eq!(document.page_count(), 3);
        // Typed on the last page, bounded to it; the text runs onto a fourth.
        document
            .insert_text(3, 0, &"overflowing words ".repeat(400), 3)
            .unwrap();
        drain(&mut document);
        let total = document.page_count();
        assert!(total > 3, "window {window:?}: the text ran onto a new page");
        let edited = document.export_docx().unwrap();
        assert_footers_read_of(&document, &edited, total);
    }
}

#[test]
fn numpages_survives_a_page_box_change() {
    let mut document = open(&three_pages(&field_footer()), None);
    document
        .apply_command(
            HeadlessCommand::SetPageOrientation { landscape: true },
            true,
        )
        .unwrap();
    let (width, height) = document.layout_page_size();
    assert!(width > height);
    drain(&mut document);
    let edited = document.export_docx().unwrap();
    assert_footers_read_of(&document, &edited, document.page_count());
}
