//! `PAGEREF` and `SECTION` results on a recording painter: a PAGEREF shows
//! the number its target's page displays, in that section's format; `\p`
//! keeps Word's saved relative text; a footer's SECTION names the section
//! supplying the page's chrome.

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const GEOMETRY: &str = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

/// A `sectPr` sharing the footer `rF`, starting with `break_type`.
fn section_properties(break_type: &str, extra: &str) -> String {
    format!(
        r#"<w:sectPr><w:footerReference w:type="default" r:id="rF"/><w:type w:val="{break_type}"/>{GEOMETRY}{extra}</w:sectPr>"#
    )
}

/// A package of `body` and a footer holding `footer` (a paragraph's inner
/// XML).
fn package(body: &str, footer: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rF" Type="{R}/footer" Target="footer1.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#
            ),
        ),
        (
            "word/footer1.xml",
            format!(r#"<w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p>{footer}</w:p></w:ftr>"#),
        ),
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

fn painted_text(renderer: &CanvasRenderer<RecordingPainter>) -> String {
    renderer
        .host
        .painter()
        .inner()
        .ops()
        .iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) if text != "\u{21b5}" => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Renders `data` twice, as a view does once layout knows where a forward
/// reference's target lies, and returns the text the second render painted.
fn render_twice(data: &[u8]) -> String {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("first render");
    renderer.host.painter().inner().clear();
    renderer
        .render(document, &Options::default())
        .expect("second render");
    painted_text(&renderer)
}

/// Section 1 holds `field` between "[" and "]"; section 2, numbered by
/// `numbering`, holds the bookmark `target`.
fn forward_reference(field: &str, numbering: &str) -> Vec<u8> {
    let body = format!(
        r#"<w:p><w:r><w:t>[</w:t></w:r>{field}<w:r><w:t>]</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties("nextPage", numbering),
    );
    package(
        &body,
        r#"<w:r><w:t>F</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple>"#,
    )
}

/// Word shows the target page's displayed number: section 2 restarts at 1
/// in lower-case roman numerals, so physical page 2 shows "i".
#[test]
fn a_pageref_shows_the_roman_number_its_targets_section_displays() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target \h "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#,
    ));
    assert!(text.contains("[i]"), "{text:?}");
}

/// A restart applies without a format change.
#[test]
fn a_pageref_follows_its_targets_section_restart() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:start="5"/>"#,
    ));
    assert!(text.contains("[5]"), "{text:?}");
}

/// `\p` prints relative text, which Jade does not generate: the saved
/// result stays.
#[test]
fn a_relative_pageref_paints_its_saved_result() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target \p "><w:r><w:t>on page 2</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#,
    ));
    assert!(text.contains("[on page 2]"), "{text:?}");
}

#[test]
fn the_parser_keeps_pagerefs_relative_position_switch() {
    let word_doc = futures::executor::block_on(WordDocument::load(
        &forward_reference(
            r#"<w:fldSimple w:instr=" PAGEREF target \h \p "><w:r><w:t>above</w:t></w:r></w:fldSimple>"#,
            "",
        ),
        &Options::default(),
    ))
    .expect("load");
    let kind = word_doc
        .paragraph(0)
        .expect("paragraph")
        .runs
        .iter()
        .find_map(|run| run.field.as_ref())
        .map(|field| field.kind.clone());
    assert_eq!(
        kind,
        Some(RunFieldKind::PageRef {
            name: "target".into(),
            hyperlink: true,
            relative_position: true,
        })
    );
}

/// A footer's SECTION on a page shared across a continuous break names the
/// section at the page's top, whose footer the page shows; later pages
/// name the second section.
#[test]
fn a_footers_section_field_follows_the_pages_chrome_section() {
    let a: String = (0..3)
        .map(|index| format!("<w:p><w:r><w:t>a{index}</w:t></w:r></w:p>"))
        .collect();
    let b: String = (0..80)
        .map(|index| format!("<w:p><w:r><w:t>b{index}</w:t></w:r></w:p>"))
        .collect();
    let body = format!(
        "{a}<w:p><w:pPr>{}</w:pPr></w:p>{b}{}",
        section_properties("nextPage", ""),
        section_properties("continuous", ""),
    );
    let data = package(
        &body,
        r#"<w:r><w:t>S</w:t></w:r><w:fldSimple w:instr=" SECTION "><w:r><w:t>0</w:t></w:r></w:fldSimple><w:r><w:t>;</w:t></w:r>"#,
    );
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("render");
    let text = painted_text(&renderer);
    assert!(text.contains("a2b0"), "B starts on page 1: {text:?}");
    let sections: Vec<&str> = text
        .split('S')
        .skip(1)
        .filter_map(|rest| rest.split(';').next())
        .collect();
    assert_eq!(sections, ["1", "2", "2"], "{text:?}");
}
