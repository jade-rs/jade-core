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

/// One section: its header references (`w:type`, header), whether it has
/// a title page, how it starts (`w:type`) and further `w:sectPr` content.
struct Section<'a> {
    body: &'a str,
    headers: Vec<(&'a str, Header<'a>)>,
    title_page: bool,
    break_type: &'a str,
    extra: &'a str,
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
            r#"<w:sectPr>{references}<w:type w:val="{}"/>{geometry}{}{title_page}</w:sectPr>"#,
            section.break_type, section.extra
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
            break_type: "nextPage",
            extra: "",
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
            break_type: "nextPage",
            extra: "",
        },
    ])
}

fn load(data: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load")
}

fn render(data: &[u8]) -> Vec<Vec<Op>> {
    render_word(&load(data))
}

fn render_word(word_doc: &WordDocument) -> Vec<Vec<Op>> {
    let document = word_doc.document.clone().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_document_header_footer_stories(word_doc);
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
            break_type: "nextPage",
            extra: "",
        },
        Section {
            body: s2,
            headers: Vec::new(),
            title_page: false,
            break_type: "nextPage",
            extra: "",
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
            break_type: "nextPage",
            extra: "",
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
        break_type: "nextPage",
        extra: "",
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

/// Two sections of two pages each, both with their own default header and
/// no watermark.
fn unlinked_two_section_package() -> Vec<u8> {
    let s1 = format!(
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>two</w:t></w:r></w:p>"#
    );
    let s2 = format!(
        r#"<w:p><w:r><w:t>three</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>four</w:t></w:r></w:p>"#
    );
    package(&[
        Section {
            body: &s1,
            headers: vec![(
                "default",
                Header {
                    id: "rH1",
                    text: "H1",
                    watermark: None,
                },
            )],
            title_page: false,
            break_type: "nextPage",
            extra: "",
        },
        Section {
            body: &s2,
            headers: vec![(
                "default",
                Header {
                    id: "rH2",
                    text: "H2",
                    watermark: None,
                },
            )],
            title_page: false,
            break_type: "nextPage",
            extra: "",
        },
    ])
}

fn set_and_render(word_doc: &mut WordDocument, text: Option<&str>) -> Vec<Vec<Op>> {
    assert!(word_doc.set_watermark_text(text).expect("set watermark"));
    let pages = render_word(word_doc);
    // The package says the same: a reload paints what the edit painted.
    let reloaded = load(&word_doc.export_docx().expect("export"));
    let reloaded_pages = render_word(&reloaded);
    let watermarks_of = |pages: &[Vec<Op>]| -> Vec<Vec<String>> {
        pages.iter().map(|page| watermarks(page)).collect()
    };
    assert_eq!(
        watermarks_of(&pages),
        watermarks_of(&reloaded_pages),
        "reload"
    );
    pages
}

/// Word's 透かし command writes every section's headers: an earlier
/// section with its own (unlinked) header shows the watermark too, and
/// removing it takes it off every page.
#[test]
fn set_watermark_reaches_an_unlinked_earlier_section_and_removal_clears_every_page() {
    let mut word_doc = load(&unlinked_two_section_package());
    let pages = set_and_render(&mut word_doc, Some("DRAFT"));
    assert_eq!(pages.len(), 4);
    for (index, page) in pages.iter().enumerate() {
        assert_eq!(watermarks(page), ["DRAFT"], "page {}", index + 1);
    }
    assert!(page_text(&pages[0]).contains("H1") && page_text(&pages[2]).contains("H2"));

    let pages = set_and_render(&mut word_doc, None);
    assert_eq!(pages.len(), 4);
    for (index, page) in pages.iter().enumerate() {
        assert!(watermarks(page).is_empty(), "page {}", index + 1);
    }
}

/// Headers the pages show blank — a first section declaring none, and its
/// title page's first-page header — are created to carry the watermark.
#[test]
fn set_watermark_creates_the_headers_pages_show_blank() {
    let s1 = format!(
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>{PAGE_BREAK}<w:p><w:r><w:t>two</w:t></w:r></w:p>"#
    );
    let s2 = r#"<w:p><w:r><w:t>three</w:t></w:r></w:p>"#;
    let mut word_doc = load(&package(&[
        Section {
            body: &s1,
            headers: Vec::new(),
            title_page: true,
            break_type: "nextPage",
            extra: "",
        },
        Section {
            body: s2,
            headers: vec![(
                "default",
                Header {
                    id: "rH2",
                    text: "H2",
                    watermark: None,
                },
            )],
            title_page: false,
            break_type: "nextPage",
            extra: "",
        },
    ]));
    let pages = set_and_render(&mut word_doc, Some("DRAFT"));
    assert_eq!(pages.len(), 3);
    for (index, page) in pages.iter().enumerate() {
        assert_eq!(watermarks(page), ["DRAFT"], "page {}", index + 1);
    }
    assert!(
        !word_doc.set_watermark_text(Some("DRAFT")).expect("repeat"),
        "setting it again is a no-op"
    );
    let pages = set_and_render(&mut word_doc, None);
    assert!(pages.iter().all(|page| watermarks(page).is_empty()));
}

/// Section 1 fills page 1 down to a continuous break near its bottom.
/// Section 2 has a title page, its first-page header carrying "FIRST" and
/// its default header "TWO", a page border shown on its first page only,
/// and a first paragraph too tall for what is left of page 1, so section 2
/// starts on page 2.
pub(super) fn continuous_title_page_package() -> Vec<u8> {
    let tall = |text: &str, points: u32| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{}" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#,
            points * 20
        )
    };
    let s1 = tall("one", 600);
    let s2 = format!("{}{PAGE_BREAK}{}", tall("three", 72), tall("four", 72));
    package(&[
        Section {
            body: &s1,
            headers: vec![(
                "default",
                Header {
                    id: "rH1",
                    text: "H1",
                    watermark: Some("ONE"),
                },
            )],
            title_page: false,
            break_type: "nextPage",
            extra: "",
        },
        Section {
            body: &s2,
            headers: vec![
                (
                    "default",
                    Header {
                        id: "rH2",
                        text: "H2",
                        watermark: Some("TWO"),
                    },
                ),
                (
                    "first",
                    Header {
                        id: "rH2F",
                        text: "H2-first",
                        watermark: Some("FIRST"),
                    },
                ),
            ],
            title_page: true,
            break_type: "continuous",
            extra: r#"<w:pgBorders w:display="firstPage" w:offsetFrom="text"><w:top w:val="single" w:sz="8" w:space="4" w:color="FF0000"/><w:bottom w:val="single" w:sz="8" w:space="4" w:color="FF0000"/></w:pgBorders>"#,
        },
    ])
}

/// While page 2's backing is created, section 2's first paragraph has only
/// started on page 1 and is moving on: page 2 is still section 2's first
/// page, so it shows the first-page header's watermark and the page border
/// shown on the first page only.
#[test]
fn a_continuous_section_pushed_to_the_next_page_starts_there() {
    let pages = render(&continuous_title_page_package());
    assert_eq!(pages.len(), 3, "three pages");
    assert_eq!(watermarks(&pages[0]), ["ONE"], "page 1");
    let page_two = page_text(&pages[1]);
    assert!(page_two.contains("H2-first"), "page 2 header: {page_two}");
    assert_eq!(watermarks(&pages[1]), ["FIRST"], "page 2");
    let page_three = page_text(&pages[2]);
    assert!(
        page_three.contains("H2") && !page_three.contains("first"),
        "page 3 header: {page_three}"
    );
    assert_eq!(watermarks(&pages[2]), ["TWO"], "page 3");
    let red = |page: &[Op]| {
        page.iter()
            .any(|op| matches!(op, Op::SetStrokeStyle(stroke) if stroke.contains("FF0000")))
    };
    assert!(!red(&pages[0]), "no border on page 1");
    assert!(red(&pages[1]), "w:display=firstPage borders page 2");
    assert!(!red(&pages[2]), "no border on page 3");
}

/// A document whose sections declare no header gets one header, in its
/// first section, which the later sections show through "link to
/// previous" — as Word's command leaves it — rather than a header each.
#[test]
fn set_watermark_on_sections_without_headers_creates_one_linked_header() {
    use std::io::Read;
    let s1 = r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>"#;
    let s2 = r#"<w:p><w:r><w:t>two</w:t></w:r></w:p>"#;
    let unheaded = |body| Section {
        body,
        headers: Vec::new(),
        title_page: false,
        break_type: "nextPage",
        extra: "",
    };
    let mut word_doc = load(&package(&[unheaded(s1), unheaded(s2)]));
    let pages = set_and_render(&mut word_doc, Some("DRAFT"));
    assert_eq!(pages.len(), 2);
    for (index, page) in pages.iter().enumerate() {
        assert_eq!(watermarks(page), ["DRAFT"], "page {}", index + 1);
    }

    let exported = word_doc.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("package");
    let headers = (0..archive.len())
        .filter_map(|index| Some(archive.by_index(index).ok()?.name().ok()?.into_owned()))
        .filter(|name: &String| name.starts_with("word/header"))
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), 1, "one header part: {headers:?}");
    let mut document_xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document part")
        .read_to_string(&mut document_xml)
        .expect("read document part");
    let (first_section, last_section) = document_xml
        .split_once("</w:sectPr>")
        .expect("a section break");
    assert_eq!(
        first_section.matches("w:headerReference").count(),
        1,
        "section 1 owns the header: {document_xml}"
    );
    assert!(
        !last_section.contains("w:headerReference"),
        "section 2 stays linked to section 1: {document_xml}"
    );
}

#[test]
#[ignore = "writes the committed fixture"]
fn write_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(FIXTURE);
    std::fs::write(path, two_section_package()).expect("write fixture");
}
