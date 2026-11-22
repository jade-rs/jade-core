//! Typing in a document with hyperlinks still repaints only the edited band.
//!
//! Most long documents carry a link or a table of contents. The band path
//! keeps every pixel outside the edited paragraph, so the link registry has to
//! stay right for paragraphs the band does not walk again: no duplicates for
//! the edited paragraph and nothing lost for the others.

mod common;
use common::open_bytes;
use jade_docx_editor::canvas_renderer::HyperlinkRegion;
use jade_docx_native_render::NativeDocument;
use std::io::Write;
use std::time::Duration;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const PARAGRAPHS: usize = 90;
const EDITED: usize = 50;
const FILLER: &str = "The quick brown fox jumps over the lazy dog while the band \
     redraw keeps every other pixel of the page exactly as it was painted.";

fn link(anchor: &str, text: &str) -> String {
    format!(r#"<w:hyperlink w:anchor="{anchor}"><w:r><w:t>{text}</w:t></w:r></w:hyperlink>"#)
}

/// About four Letter pages of text. With `links`, paragraph 1 (page 1), the
/// paragraphs either side of the edited one and the edited one itself carry a
/// link.
fn docx(links: bool) -> Vec<u8> {
    let mut body = String::new();
    for index in 0..PARAGRAPHS {
        let linked = links && [1, EDITED - 1, EDITED, EDITED + 1].contains(&index);
        let text = if index == EDITED {
            "Edit here: ".to_string()
        } else {
            FILLER.to_string()
        };
        let tail = if linked {
            link("target", "linked words")
        } else {
            "<w:r><w:t xml:space=\"preserve\"> plain words</w:t></w:r>".to_string()
        };
        body.push_str(&format!(
            "<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>{tail}</w:p>"
        ));
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}<w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>Target</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", document),
        ] {
            zip.start_file(name, options).expect("zip entry");
            zip.write_all(content.as_bytes()).expect("zip write");
        }
        zip.finish().expect("zip");
    }
    bytes
}

/// Regions in a comparable, order-independent form, rounded to 1/100 px.
fn normalized(regions: &[HyperlinkRegion]) -> Vec<String> {
    let mut rows: Vec<String> = regions
        .iter()
        .map(|r| {
            format!(
                "p{} para{} {}..{} ({:.2},{:.2} {:.2}x{:.2}) {:?}",
                r.page, r.paragraph, r.start, r.end, r.x, r.y, r.width, r.height, r.anchor
            )
        })
        .collect();
    rows.sort();
    rows
}

/// Types `count` characters in the middle of the edited paragraph, one
/// keystroke at a time, returning each keystroke's duration.
fn type_mid_paragraph(document: &mut NativeDocument, count: usize) -> Vec<Duration> {
    let caret = document.place_caret(EDITED, 0).expect("caret");
    let page = document.page_and_offset(caret.document_y).0;
    let mut durations = Vec::with_capacity(count);
    for _ in 0..count {
        // After "Edit" — the middle of the text before the link, so the link's
        // offsets move with every keystroke.
        durations.push(
            document
                .insert_text(EDITED, 4, "x", page + 1)
                .expect("insert and repaint"),
        );
    }
    durations
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

#[test]
fn typing_next_to_hyperlinks_takes_the_band_and_keeps_the_link_registry() {
    let mut document = open_bytes(&docx(true));
    assert!(document.page_count() >= 3, "needs a multi-page document");
    let caret = document.place_caret(EDITED, 0).expect("caret");
    let page = document.page_and_offset(caret.document_y).0;
    assert!(page >= 1, "the edited paragraph should sit after page 1");
    let links_before = document.renderer().hyperlink_regions().len();
    assert!(links_before >= 4, "every link registered: {links_before}");

    let taken_before = document.renderer().band_redraw_stats().1;
    type_mid_paragraph(&mut document, 30);
    let (_, taken, fallbacks) = document.renderer().band_redraw_stats();
    assert!(
        taken - taken_before >= 29,
        "typing next to links should take the band: taken {} of 30, fallbacks {fallbacks:?}",
        taken - taken_before
    );

    let fresh = open_bytes(&document.export_docx().expect("export"));
    let typed = normalized(document.renderer().hyperlink_regions());
    let expected = normalized(fresh.renderer().hyperlink_regions());
    assert_eq!(
        typed, expected,
        "the link registry after typing differs from a fresh full render"
    );
    for index in 0..document.page_count() {
        let typed = normalized(&document.renderer().page_hyperlink_regions(index));
        let expected = normalized(&fresh.renderer().page_hyperlink_regions(index));
        assert_eq!(typed, expected, "page {index} PDF link regions differ");
    }

    let incremental = document.page_pixels(page).expect("page pixels").0;
    let expected = fresh.page_pixels(page).expect("fresh page pixels").0;
    let differing = incremental
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "page {page} after typing differs from a full repaint in {differing} pixels"
    );
}

/// Run with `--ignored --nocapture` for the keystroke medians.
#[test]
#[ignore]
fn keystroke_cost_with_and_without_hyperlinks() {
    for links in [false, true] {
        // 200 keystrokes in rounds of 25, each on a freshly opened document so
        // the edited line never wraps (a height change leaves the band path).
        let mut durations = Vec::new();
        let mut taken = 0;
        for _ in 0..8 {
            let mut document = open_bytes(&docx(links));
            type_mid_paragraph(&mut document, 2);
            durations.extend(type_mid_paragraph(&mut document, 25));
            taken += document.renderer().band_redraw_stats().1;
        }
        println!(
            "links={links}: median insert_text {:?} over {} keystrokes, band taken {taken}",
            median(durations.clone()),
            durations.len()
        );
    }
}
