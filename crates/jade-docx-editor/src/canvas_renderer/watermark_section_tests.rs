//! Text watermarks drawn per header, as Word draws them.
//!
//! Word keeps a text watermark as a VML `v:textpath` shape inside one header
//! part, so a page shows the watermark of the header it shows: a title page
//! whose first-page header has none is clean, a section whose own (unlinked)
//! header has none shows none, and two sections can carry different text.

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::Op;
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const V: &str = "urn:schemas-microsoft-com:vml";
const O: &str = "urn:schemas-microsoft-com:office:office";
const W10: &str = "urn:schemas-microsoft-com:office:word";
const PAGE_BREAK: &str = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;

/// The committed copy of [`two_section_package`].
const FIXTURE: &str = "public/fixtures/watermark-per-section.docx";

/// One header part: its relationship id, its text, and the watermark its
/// VML carries.
struct Header<'a> {
    id: &'a str,
    text: &'a str,
    watermark: Option<&'a str>,
}

/// One section: its header references (`w:type`, header) and whether it
/// has a title page.
struct Section<'a> {
    body: &'a str,
    headers: Vec<(&'a str, Header<'a>)>,
    title_page: bool,
}

/// A run holding the shape Word's デザイン › 透かし inserts.
fn watermark_run(text: &str) -> String {
    format!(
        r##"<w:r><w:rPr><w:noProof/></w:rPr><w:pict><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800" path="m@7,l@8,m@5,21600l@6,21600e"><v:path textpathok="t" o:connecttype="custom"/><v:textpath on="t" fitshape="t"/></v:shapetype><v:shape id="PowerPlusWaterMarkObject1" o:spid="_x0000_s2049" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:412.4pt;height:247.45pt;rotation:315;z-index:-251657216;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Calibri&quot;;font-size:1pt" string="{text}"/><w10:wrap anchorx="margin" anchory="margin"/></v:shape></w:pict></w:r>"##
    )
}

fn header_xml(header: &Header) -> String {
    let watermark = header.watermark.map(watermark_run).unwrap_or_default();
    format!(
        r#"<w:hdr xmlns:w="{W}" xmlns:r="{R}" xmlns:v="{V}" xmlns:o="{O}" xmlns:w10="{W10}"><w:p>{watermark}<w:r><w:t>{}</w:t></w:r></w:p></w:hdr>"#,
        header.text
    )
}

fn package(sections: &[Section]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let geometry = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let content_type = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
    let mut body = String::new();
    let mut rels = String::new();
    let mut overrides = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let mut references = String::new();
        for (kind, header) in &section.headers {
            let target = format!("header-{}.xml", header.id);
            references.push_str(&format!(
                r#"<w:headerReference w:type="{kind}" r:id="{}"/>"#,
                header.id
            ));
            rels.push_str(&format!(
                r#"<Relationship Id="{}" Type="{R}/header" Target="{target}"/>"#,
                header.id
            ));
            overrides.push_str(&format!(
                r#"<Override PartName="/word/{target}" ContentType="{content_type}"/>"#
            ));
            parts.push((format!("word/{target}"), header_xml(header)));
        }
        let title_page = if section.title_page {
            "<w:titlePg/>"
        } else {
            ""
        };
        let properties = format!(
            r#"<w:sectPr>{references}<w:type w:val="nextPage"/>{geometry}{title_page}</w:sectPr>"#
        );
        body.push_str(section.body);
        if index + 1 < sections.len() {
            body.push_str(&format!("<w:p><w:pPr>{properties}</w:pPr></w:p>"));
        } else {
            body.push_str(&properties);
        }
    }
    parts.push((
        "word/document.xml".into(),
        format!(r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#),
    ));
    parts.push((
        "word/_rels/document.xml.rels".into(),
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
        ),
    ));
    parts.push((
        "_rels/.rels".into(),
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
        ),
    ));
    parts.push((
        "[Content_Types].xml".into(),
        format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>{overrides}</Types>"#
        ),
    ));
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

/// Section 1 has a title page whose first-page header has no watermark and
/// "DRAFT" on its default header; section 2 is unlinked and carries
/// "FINAL". Each section spans two pages.
fn two_section_package() -> Vec<u8> {
    let s1 = format!(
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>two</w:t></w:r></w:p>"#
    );
    let s2 = format!(
        r#"<w:p><w:r><w:t>three</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>four</w:t></w:r></w:p>"#
    );
    package(&[
        Section {
            body: &s1,
            headers: vec![
                (
                    "default",
                    Header {
                        id: "rH1",
                        text: "H1",
                        watermark: Some("DRAFT"),
                    },
                ),
                (
                    "first",
                    Header {
                        id: "rH1F",
                        text: "H1-first",
                        watermark: None,
                    },
                ),
            ],
            title_page: true,
        },
        Section {
            body: &s2,
            headers: vec![(
                "default",
                Header {
                    id: "rH2",
                    text: "H2",
                    watermark: Some("FINAL"),
                },
            )],
            title_page: false,
        },
    ])
}

fn render(data: &[u8]) -> Vec<Vec<Op>> {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.clone().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_document_header_footer_stories(&word_doc);
    renderer
        .render(&document, &Options::default())
        .expect("render");
    split_pages(&renderer.host.painter().inner().ops())
}

/// The painted ops of each page, split at its full-page clear.
fn split_pages(ops: &[Op]) -> Vec<Vec<Op>> {
    let mut pages: Vec<Vec<Op>> = Vec::new();
    for op in ops {
        if matches!(op, Op::ClearRect(0.0, 0.0, _, _)) {
            pages.push(Vec::new());
        } else if let Some(page) = pages.last_mut() {
            page.push(op.clone());
        }
    }
    pages
}

/// The watermark text each page paints: the one text drawn rotated.
fn watermarks(page: &[Op]) -> Vec<String> {
    let mut rotated = false;
    let mut found = Vec::new();
    for op in page {
        match op {
            Op::Rotate(_) => rotated = true,
            Op::Restore => rotated = false,
            Op::FillText(text, ..) if rotated => found.push(text.clone()),
            _ => {}
        }
    }
    found
}

/// The text a page paints, glyph by glyph joined.
fn page_text(page: &[Op]) -> String {
    page.iter()
        .filter_map(|op| match op {
            Op::FillText(text, ..) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assert_per_header_watermarks(pages: &[Vec<Op>]) {
    assert_eq!(pages.len(), 4, "four pages");
    // Page 1 is section 1's title page: its first-page header has none.
    assert!(
        !pages[0]
            .iter()
            .any(|op| matches!(op, Op::Rotate(_) | Op::SetGlobalAlpha(_))),
        "page 1 paints no watermark: {:?}",
        pages[0]
    );
    assert!(watermarks(&pages[0]).is_empty());
    assert!(page_text(&pages[0]).starts_with("H1-first"));
    assert_eq!(watermarks(&pages[1]), ["DRAFT"], "page 2");
    let page_two = page_text(&pages[1]);
    assert!(
        page_two.contains("H1") && !page_two.contains("first"),
        "page 2 shows the default header: {page_two}"
    );
    for (index, page) in pages.iter().enumerate().skip(2) {
        assert_eq!(watermarks(page), ["FINAL"], "page {}", index + 1);
        assert!(page_text(page).contains("H2"));
    }
}

#[test]
fn each_page_paints_the_watermark_of_the_header_it_shows() {
    assert_per_header_watermarks(&render(&two_section_package()));
}

#[test]
fn the_committed_fixture_paints_each_headers_watermark() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(FIXTURE);
    let data = std::fs::read(&path).expect("read fixture");
    assert_per_header_watermarks(&render(&data));
}

/// A section linked to the previous one shows its header, and so its
/// watermark; one with its own header without a watermark shows none.
#[test]
fn link_to_previous_carries_the_watermark_and_an_unlinked_header_drops_it() {
    let s1 = r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>"#;
    let s2 = r#"<w:p><w:r><w:t>two</w:t></w:r></w:p>"#;
    let s3 = r#"<w:p><w:r><w:t>three</w:t></w:r></w:p>"#;
    let pages = render(&package(&[
        Section {
            body: s1,
            headers: vec![(
                "default",
                Header {
                    id: "rH1",
                    text: "H1",
                    watermark: Some("DRAFT"),
                },
            )],
            title_page: false,
        },
        Section {
            body: s2,
            headers: Vec::new(),
            title_page: false,
        },
        Section {
            body: s3,
            headers: vec![(
                "default",
                Header {
                    id: "rH3",
                    text: "H3",
                    watermark: None,
                },
            )],
            title_page: false,
        },
    ]));
    assert_eq!(pages.len(), 3);
    assert_eq!(watermarks(&pages[0]), ["DRAFT"]);
    assert_eq!(watermarks(&pages[1]), ["DRAFT"], "linked to section 1");
    assert!(watermarks(&pages[2]).is_empty(), "own header without one");
    assert!(page_text(&pages[2]).contains("H3"));
}

/// A document with one header paints its watermark on every page, each
/// page the same operations.
#[test]
fn a_single_header_paints_its_watermark_on_every_page() {
    let body = format!(
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>two</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>three</w:t></w:r></w:p>"#
    );
    let pages = render(&package(&[Section {
        body: &body,
        headers: vec![(
            "default",
            Header {
                id: "rH1",
                text: "H1",
                watermark: Some("DRAFT"),
            },
        )],
        title_page: false,
    }]));
    assert_eq!(pages.len(), 3);
    let watermark_ops = |page: &[Op]| -> Vec<Op> {
        let start = page
            .iter()
            .position(|op| matches!(op, Op::Rotate(_)))
            .expect("rotated watermark");
        page[..start]
            .iter()
            .rposition(|op| matches!(op, Op::Save))
            .map(|save| {
                let end = page[start..]
                    .iter()
                    .position(|op| matches!(op, Op::Restore))
                    .expect("restore")
                    + start;
                page[save..=end].to_vec()
            })
            .expect("save")
    };
    let first = watermark_ops(&pages[0]);
    for page in &pages {
        assert_eq!(watermarks(page), ["DRAFT"]);
        assert_eq!(watermark_ops(page), first);
    }
}

#[test]
#[ignore = "writes the committed fixture"]
fn write_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(FIXTURE);
    std::fs::write(path, two_section_package()).expect("write fixture");
}
