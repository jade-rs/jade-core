//! Pages shared by sections after a continuous or next-column break.
//!
//! The page's header, footer and page borders come from the section at the
//! top of the page (not confirmed against Word captures). Its page number
//! follows the last section with content on it: a mid-page section with
//! content on the shared page restarts that page's number, and its
//! first-page header never shows (Word 2019+, per Aspose's documentation of
//! `ContinuousSectionRestart.Always`).

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Page-local Y of the body's top and bottom margins: header text paints
/// above the first, footer text below the second.
const BODY_TOP: f64 = 96.0;
const BODY_BOTTOM: f64 = 960.0;

/// One section of a test package. Section `name` gets the default header
/// "H{name}", first-page header "H{name}-first", even header
/// "H{name}-even", footer "F{name}" (each followed by a PAGE field) and a
/// page border in `color`.
struct Section<'a> {
    name: &'a str,
    color: &'a str,
    break_type: &'a str,
    body: String,
    extra: &'a str,
}

fn section<'a>(
    name: &'a str,
    color: &'a str,
    break_type: &'a str,
    body: String,
    extra: &'a str,
) -> Section<'a> {
    Section {
        name,
        color,
        break_type,
        body,
        extra,
    }
}

fn story(tag: &str, text: &str) -> String {
    format!(
        r#"<w:{tag} xmlns:w="{W}" xmlns:r="{R}"><w:p><w:r><w:t>{text}</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:{tag}>"#
    )
}

fn package(sections: &[Section], settings: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let geometry = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let header = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
    let footer = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
    let mut body = String::new();
    let mut rels = format!(r#"<Relationship Id="rS" Type="{R}/settings" Target="settings.xml"/>"#);
    let mut overrides = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let name = section.name;
        let mut references = String::new();
        for (kind, suffix, tag, content_type, text) in [
            ("default", "", "hdr", header, format!("H{name}")),
            ("first", "1", "hdr", header, format!("H{name}-first")),
            ("even", "2", "hdr", header, format!("H{name}-even")),
            ("default", "", "ftr", footer, format!("F{name}")),
        ] {
            let role = if tag == "hdr" { "header" } else { "footer" };
            let id = format!("r{role}{name}{suffix}");
            let target = format!("{role}{name}{suffix}.xml");
            references.push_str(&format!(
                r#"<w:{role}Reference w:type="{kind}" r:id="{id}"/>"#
            ));
            rels.push_str(&format!(
                r#"<Relationship Id="{id}" Type="{R}/{role}" Target="{target}"/>"#
            ));
            overrides.push_str(&format!(
                r#"<Override PartName="/word/{target}" ContentType="{content_type}"/>"#
            ));
            parts.push((format!("word/{target}"), story(tag, &text)));
        }
        let properties = format!(
            r#"<w:sectPr>{references}<w:type w:val="{}"/>{geometry}<w:pgBorders w:offsetFrom="text"><w:top w:val="single" w:sz="8" w:space="4" w:color="{color}"/><w:bottom w:val="single" w:sz="8" w:space="4" w:color="{color}"/></w:pgBorders>{}</w:sectPr>"#,
            section.break_type,
            section.extra,
            color = section.color,
        );
        body.push_str(&section.body);
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
        "word/settings.xml".into(),
        format!(r#"<w:settings xmlns:w="{W}">{settings}</w:settings>"#),
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
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>{overrides}</Types>"#
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

fn paragraphs(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("<w:p><w:r><w:t>{prefix}{index}</w:t></w:r></w:p>"))
        .collect()
}

/// A paragraph showing "P" and its page number.
const PAGE_FIELD: &str = r#"<w:p><w:r><w:t>P</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>"#;

/// What one page painted: its page border colour, and the text painted
/// above, within and below the body margins (paragraph marks dropped).
#[derive(Debug, Default, PartialEq)]
struct Page {
    border: String,
    header: String,
    body: String,
    footer: String,
}

/// Splits the painted ops at each page's full-page clear.
fn pages(ops: &[Op]) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    for op in ops {
        match op {
            Op::ClearRect(0.0, 0.0, _, _) => pages.push(Page::default()),
            Op::SetStrokeStyle(color) if color != "#888888" => {
                if let Some(page) = pages.last_mut().filter(|page| page.border.is_empty()) {
                    page.border = color.trim_start_matches('#').to_string();
                }
            }
            Op::FillText(text, _, y) if text != "\u{21b5}" => {
                let Some(page) = pages.last_mut() else {
                    continue;
                };
                let target = if *y < BODY_TOP {
                    &mut page.header
                } else if *y > BODY_BOTTOM {
                    &mut page.footer
                } else {
                    &mut page.body
                };
                target.push_str(text);
            }
            _ => {}
        }
    }
    pages
}

fn render(data: &[u8]) -> (CanvasRenderer<RecordingPainter>, Vec<Page>) {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_document_header_footer_stories(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("render");
    let pages = pages(&renderer.host.painter().inner().ops());
    (renderer, pages)
}

/// Border, header and footer of each page.
fn chrome(pages: &[Page]) -> Vec<(&str, &str, &str)> {
    pages
        .iter()
        .map(|page| {
            (
                page.border.as_str(),
                page.header.as_str(),
                page.footer.as_str(),
            )
        })
        .collect()
}

const RED: &str = "FF0000";
const BLUE: &str = "0000FF";
const GREEN: &str = "00FF00";

/// Section A's three paragraphs, then section B starting with `break_type`
/// and 80 paragraphs `b0`..: B starts partway down page 1 and runs onto
/// pages 2 and 3.
fn two_sections(break_type: &str, extra_a: &str, b_body: String, extra_b: &str) -> Vec<u8> {
    package(
        &[
            section("A", RED, "nextPage", paragraphs("a", 3), extra_a),
            section("B", BLUE, break_type, b_body, extra_b),
        ],
        "",
    )
}

#[test]
fn a_continuous_section_shows_its_chrome_from_the_following_page() {
    let (_, pages) = render(&two_sections("continuous", "", paragraphs("b", 80), ""));
    assert_eq!(pages.len(), 3, "{pages:?}");
    assert!(pages[0].body.contains("a2b0"), "B starts on page 1");
    assert_eq!(
        chrome(&pages),
        [
            (RED, "HA1", "FA1"),
            (BLUE, "HB2", "FB2"),
            (BLUE, "HB3", "FB3")
        ]
    );
}

#[test]
fn a_next_page_section_shows_its_chrome_on_its_first_page() {
    let (_, pages) = render(&two_sections(
        "nextPage",
        "",
        paragraphs("b", 80),
        r#"<w:titlePg/>"#,
    ));
    assert!(pages[1].body.starts_with("b0"), "B starts on page 2");
    assert_eq!(
        chrome(&pages)[..2],
        [(RED, "HA1", "FA1"), (BLUE, "HB-first2", "")]
    );
}

/// B's restart numbers the shared page, which keeps A's stories; B's
/// first-page header never shows.
///
/// Known gap: page 1's header is painted when the page begins, before B's
/// restart is known, so it still shows 1. The footer, painted at the page's
/// end, shows 7.
#[test]
fn a_continuous_sections_restart_numbers_the_shared_page() {
    let (_, pages) = render(&two_sections(
        "continuous",
        r#"<w:pgNumType w:start="1"/>"#,
        paragraphs("b", 80),
        r#"<w:titlePg/><w:pgNumType w:start="7"/>"#,
    ));
    assert_eq!(
        chrome(&pages)[1..],
        [(BLUE, "HB8", "FB8"), (BLUE, "HB9", "FB9")]
    );
    assert_eq!(
        (pages[0].border.as_str(), pages[0].footer.as_str()),
        (RED, "FA7")
    );
    assert!(pages[0].header.starts_with("HA"), "{:?}", pages[0]);
}

/// When B's first content moves on to the next page, that page is B's
/// first: its first-page header and its restart land there.
#[test]
fn b_starts_on_the_page_holding_its_first_content() {
    let b_body = format!(
        r#"<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>bfirst</w:t></w:r></w:p>{}"#,
        paragraphs("b", 60)
    );
    let (renderer, pages) = render(&two_sections(
        "continuous",
        r#"<w:pgNumType w:start="1"/>"#,
        b_body,
        r#"<w:titlePg/><w:pgNumType w:start="7"/>"#,
    ));
    assert_eq!(pages[0].body, "a0a1a2", "B has no content on page 1");
    assert_eq!(
        chrome(&pages),
        [
            (RED, "HA1", "FA1"),
            // No first-page footer is defined.
            (BLUE, "HB-first7", ""),
            (BLUE, "HB8", "FB8")
        ]
    );
    assert_eq!(renderer.first_page_of_section_index(1), Some(2));
}

/// A body PAGE field in B on the shared page shows B's number.
#[test]
fn a_body_page_field_in_b_on_the_shared_page_shows_bs_number() {
    let b_body = format!("{PAGE_FIELD}{}", paragraphs("b", 60));
    let (_, pages) = render(&two_sections(
        "continuous",
        r#"<w:pgNumType w:start="1"/>"#,
        b_body,
        r#"<w:pgNumType w:start="7"/>"#,
    ));
    assert!(pages[0].body.contains("a2P7b0"), "{:?}", pages[0]);
    assert_eq!(pages[0].footer, "FA7");
}

#[test]
fn a_next_column_section_shows_its_chrome_from_the_following_page() {
    let columns = r#"<w:cols w:num="2" w:space="720"/>"#;
    let (renderer, pages) = render(&two_sections(
        "nextColumn",
        columns,
        paragraphs("b", 80),
        columns,
    ));
    assert_eq!(renderer.mid_page_start(1).map(|start| start.page), Some(1));
    assert!(pages.len() >= 2, "{pages:?}");
    assert!(
        pages[0].body.contains("b0"),
        "B starts in page 1's second column"
    );
    assert_eq!(
        chrome(&pages)[..2],
        [(RED, "HA1", "FA1"), (BLUE, "HB2", "FB2")]
    );
}

/// Three sections share page 1; B starts and ends on it. The page keeps
/// A's chrome, numbered by B's restart, and C's chrome starts on page 2.
#[test]
fn three_sections_on_one_page_keep_the_top_sections_chrome() {
    let (renderer, pages) = render(&package(
        &[
            section("A", RED, "nextPage", paragraphs("a", 3), ""),
            section(
                "B",
                BLUE,
                "continuous",
                paragraphs("b", 3),
                r#"<w:pgNumType w:start="5"/>"#,
            ),
            section("C", GREEN, "continuous", paragraphs("c", 80), ""),
        ],
        "",
    ));
    assert!(pages[0].body.contains("a2b0b1b2c0"), "{:?}", pages[0]);
    // Page 1's header predates B's restart (the gap noted above).
    assert_eq!(
        (pages[0].border.as_str(), pages[0].footer.as_str()),
        (RED, "FA5")
    );
    assert!(pages[0].header.starts_with("HA"), "{:?}", pages[0]);
    assert_eq!(chrome(&pages)[1], (GREEN, "HC6", "FC6"));
    assert_eq!(renderer.section_index_for_page(1), 0);
    assert_eq!(renderer.numbering_section_for_page(1), 2);
}

#[test]
fn even_and_odd_headers_follow_the_page_top_section() {
    let (_, pages) = render(&package(
        &[
            section("A", RED, "nextPage", paragraphs("a", 3), ""),
            section("B", BLUE, "continuous", paragraphs("b", 80), ""),
        ],
        "<w:evenAndOddHeaders/>",
    ));
    assert_eq!(
        chrome(&pages),
        [
            (RED, "HA1", "FA1"),
            // No even footer is defined.
            (BLUE, "HB-even2", ""),
            (BLUE, "HB3", "FB3")
        ]
    );
}

/// The renderer-level view of the shared page: chrome from A, numbering
/// and body ownership from B.
#[test]
fn the_shared_page_splits_chrome_and_numbering() {
    let (renderer, _) = render(&two_sections("continuous", "", paragraphs("b", 80), ""));
    assert_eq!(&renderer.page_section_indices[..3], &[1, 1, 1]);
    assert_eq!(renderer.section_index_for_page(1), 0);
    assert_eq!(renderer.section_index_for_page(2), 1);
    assert_eq!(renderer.numbering_section_for_page(1), 1);
    assert_eq!(renderer.first_page_of_section_index(1), Some(1));
    assert!(!renderer.is_first_page_of_section(2));
}

/// A second pass over the retained pagination paints the same pages, and
/// a render of another document does not leak its section starts.
#[test]
fn repeat_renders_keep_the_shared_pages_chrome() {
    let load = |data: &[u8]| {
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load")
    };
    let continuous = two_sections("continuous", "", paragraphs("b", 80), "");
    let next_page = two_sections("nextPage", "", paragraphs("b", 80), "");
    let mut renderer = recording_renderer();
    for data in [&continuous, &continuous, &next_page, &continuous] {
        let word_doc = load(data);
        let document = word_doc.document.as_ref().expect("document");
        renderer.sync_document_header_footer_stories(&word_doc);
        renderer.host.painter().inner().clear();
        renderer
            .render(document, &Options::default())
            .expect("render");
        let painted = pages(&renderer.host.painter().inner().ops());
        let (_, fresh) = render(data);
        assert_eq!(painted, fresh);
    }
}

/// Section A fills page 1 exactly with one-line paragraphs, so B's first
/// paragraph, placed after the continuous break at the bottom of page 1,
/// lands on page 2: page 2 is B's first page.
#[test]
fn a_break_at_the_bottom_of_a_full_page_starts_b_on_the_next_page() {
    let fill = paragraphs("a", 36);
    let (renderer, pages) = render(&package(
        &[
            section("A", RED, "nextPage", fill, ""),
            section(
                "B",
                BLUE,
                "continuous",
                paragraphs("b", 40),
                r#"<w:titlePg/><w:pgNumType w:start="7"/>"#,
            ),
        ],
        "",
    ));
    assert!(!pages[0].body.contains("b0"), "{:?}", pages[0]);
    assert!(pages[1].body.starts_with("b0"), "{:?}", pages[1]);
    assert_eq!(
        chrome(&pages)[..2],
        [(RED, "HA1", "FA1"), (BLUE, "HB-first7", "")]
    );
    assert_eq!(renderer.first_page_of_section_index(1), Some(2));
}

/// B's first paragraph has several lines and moves on to page 2 whole
/// (`keepLines`): nothing of B is committed on page 1.
#[test]
fn a_multi_line_first_paragraph_that_moves_whole_starts_b_on_the_next_page() {
    let long = "word ".repeat(40);
    let b_body = format!(
        r#"<w:p><w:pPr><w:keepLines/></w:pPr><w:r><w:t xml:space="preserve">{long}</w:t></w:r></w:p>{}"#,
        paragraphs("b", 40)
    );
    let (_, pages) = render(&package(
        &[
            section("A", RED, "nextPage", paragraphs("a", 35), ""),
            section(
                "B",
                BLUE,
                "continuous",
                b_body,
                r#"<w:titlePg/><w:pgNumType w:start="7"/>"#,
            ),
        ],
        "",
    ));
    assert!(!pages[0].body.contains("word"), "{:?}", pages[0]);
    assert!(pages[1].body.starts_with("word"), "{:?}", pages[1]);
    assert_eq!(
        chrome(&pages)[..2],
        [(RED, "HA1", "FA1"), (BLUE, "HB-first7", "")]
    );
}

/// A bitmap rerender of page 1 alone stops its walk after that page. The
/// later pages keep the sections and numbers the full render gave them.
#[test]
fn a_pixel_rerender_of_page_one_keeps_later_pages_sections() {
    let data = package(
        &[
            section("A", RED, "nextPage", paragraphs("a", 90), ""),
            section(
                "B",
                BLUE,
                "continuous",
                paragraphs("b", 80),
                r#"<w:pgNumType w:start="7"/>"#,
            ),
        ],
        "",
    );
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_document_header_footer_stories(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("full render");
    let snapshot = |renderer: &CanvasRenderer<RecordingPainter>| {
        (1..=5)
            .map(|page| {
                (
                    renderer.section_index_for_page(page),
                    renderer.numbering_section_for_page(page),
                    renderer.page_number_value(page),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot(&renderer);
    assert_eq!(renderer.page_section_indices.len(), 5);
    assert_eq!(renderer.mid_page_start(1).map(|start| start.page), Some(3));
    assert_eq!(before[2], (0, 1, 7), "page 3 is shared");

    renderer.set_visible_page_range(Some(1..=1));
    renderer.set_layout_page_limit(Some(1));
    renderer.prepare_rerender_for_page(0);
    renderer.freeze_selection_for_pixel_rerender();
    renderer
        .render(document, &Options::default())
        .expect("bitmap rerender");
    assert_eq!(snapshot(&renderer), before);
}
