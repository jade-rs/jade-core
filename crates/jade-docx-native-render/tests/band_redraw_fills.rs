//! A band redraw leaves fills as a full repaint draws them.
//!
//! The band path clears and repaints only the edited paragraph's slice of its
//! page, under a clip. Fills need more care than glyphs there: a neighbour's
//! shading the clear reaches is lost unless the neighbour is repainted, a
//! neighbour's tracked-change highlight the clip cuts has its antialiased
//! corners rasterised from the clipped piece, a heading's border is drawn past
//! its band, and an erasure the edited line paints past its band is cut short.
//! Each class is typed into and deleted from on page one here, against the
//! same edits followed by a full repaint.

mod common;

use common::{open_bytes, repo_path, Doc};
use std::io::{Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn first_difference(actual: &Doc, reference: &Doc, page: usize) -> Option<(usize, usize)> {
    let (actual, width, _) = actual.page_pixels(page).expect("actual page");
    let (expected, _, _) = reference.page_pixels(page).expect("reference page");
    assert_eq!(actual.len(), expected.len(), "page {page} size");
    actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .position(|(a, b)| a != b)
        .map(|index| (index % width as usize, index / width as usize))
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).expect("read fixture")
}

/// The first `count` paragraphs with text whose start is on page one.
fn paragraphs_on_first_page(document: &mut Doc, count: usize) -> Vec<usize> {
    let mut found = Vec::new();
    for paragraph in 0..80 {
        if found.len() == count {
            break;
        }
        let has_text = document
            .paragraph_text(paragraph)
            .is_some_and(|text| text.chars().count() > 4);
        let on_first_page = has_text
            && document
                .place_caret(paragraph, 0)
                .is_some_and(|caret| document.page_and_offset(caret.document_y).0 == 0);
        if on_first_page {
            found.push(paragraph);
        }
    }
    found
}

#[derive(Clone, Copy)]
enum Edit<'a> {
    Insert(&'a str),
    Delete(usize),
    /// A whole repaint between keystrokes, as a scroll or a resize makes.
    RepaintAll,
}

fn band_taken(document: &Doc) -> u64 {
    document.renderer().band_redraw_stats().1
}

/// Applies `edits` at the start of `paragraph` on a document bounded to page
/// one (the keystroke path) and on one that repaints in full, comparing page
/// one after each. Returns how many keystrokes took the band.
fn check_edits(name: &str, bytes: &[u8], paragraph: usize, edits: &[Edit]) -> u64 {
    let mut actual = open_bytes(bytes);
    let mut reference = open_bytes(bytes);
    let taken = band_taken(&actual);
    for (step, edit) in edits.iter().enumerate() {
        for (document, bound) in [(&mut actual, 1), (&mut reference, usize::MAX)] {
            match *edit {
                Edit::Insert(text) => document.insert_text(paragraph, 0, text, bound),
                Edit::Delete(count) => document.delete_text(paragraph, 0, false, count, bound),
                Edit::RepaintAll => document.repaint_all(),
            }
            .expect("edit");
        }
        reference.repaint_all().expect("full repaint");
        assert_eq!(
            first_difference(&actual, &reference, 0),
            None,
            "{name}: paragraph {paragraph}, step {step}: page 1 differs from a full repaint",
        );
    }
    band_taken(&actual) - taken
}

/// Types, deletes and types again in the first `paragraphs` paragraphs of
/// page one. Returns how many keystrokes took the band.
fn check_typing(name: &str, bytes: &[u8], paragraphs: usize) -> u64 {
    let mut document = open_bytes(bytes);
    let targets = paragraphs_on_first_page(&mut document, paragraphs);
    assert!(!targets.is_empty(), "{name}: no paragraph on page 1");
    targets
        .into_iter()
        .map(|paragraph| {
            check_edits(
                name,
                bytes,
                paragraph,
                &[Edit::Insert("a"), Edit::Delete(1), Edit::Insert("a")],
            )
        })
        .sum()
}

/// Rewrites a package's `word/document.xml`.
fn with_document(bytes: &[u8], edit: impl Fn(&str) -> String) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a package");
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let options = zip::write::SimpleFileOptions::default();
        for index in 0..source.len() {
            let mut file = source.by_index(index).expect("an entry");
            let name = file.name().expect("an entry name").to_string();
            let mut content = Vec::new();
            file.read_to_end(&mut content).expect("read entry");
            if name == "word/document.xml" {
                content = edit(std::str::from_utf8(&content).expect("utf-8")).into_bytes();
            }
            zip.start_file(name, options).expect("zip entry");
            zip.write_all(&content).expect("zip write");
        }
        zip.finish().expect("zip");
    }
    out
}

/// A Letter package around `body`.
fn package(body: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
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

/// Every paragraph is shaded, so each one's shading meets its neighbours'.
/// The band repaints the neighbour whose shading its clip reaches.
#[test]
fn typing_between_adjacent_shaded_paragraphs_matches_a_full_repaint() {
    for name in [
        "word-windows-styled-arial-pair-matrix-size12-bold-compat15",
        "word-windows-styled-arial-pair-matrix-size24-italic-compat15",
    ] {
        let taken = check_typing(name, &fixture(name), 3);
        assert!(taken > 0, "{name}: shading should not refuse the band");
    }
}

/// The paragraph after the first holds a moved-from run, highlighted from the
/// top of its line, which the first paragraph's band clip reaches.
#[test]
fn typing_above_a_highlighted_revision_matches_a_full_repaint() {
    check_typing("review-kinds", &fixture("review-kinds"), 3);
}

/// A heading's bottom border is drawn below its last line, past its band.
#[test]
fn typing_in_a_bordered_heading_matches_a_full_repaint() {
    let mut body = String::new();
    for index in 0..8 {
        let border = if index % 3 == 0 {
            r#"<w:pPr><w:pBdr><w:bottom w:val="single" w:sz="8" w:space="1" w:color="4472C4"/></w:pBdr></w:pPr>"#
        } else {
            ""
        };
        body.push_str(&format!(
            r#"<w:p>{border}<w:r><w:t xml:space="preserve">{index} Heading or body text with descenders, gypsy</w:t></w:r></w:p>"#
        ));
    }
    check_typing("bordered headings", &package(&body), 6);
}

/// Sixty characters make the last line on the page overflow its right edge.
/// The erasure behind the overflow follows the line's clip, which passes the
/// band and covers the corner crop mark below; removing them again brings
/// the crop mark back.
#[test]
fn an_overflowing_line_on_the_band_matches_a_full_repaint() {
    let long = "wrap ".repeat(12);
    let name = "word-mac-probe-ruby-wide-grid";
    check_edits(
        name,
        &fixture(name),
        28,
        &[Edit::Insert(&long), Edit::Delete(60), Edit::Insert("a")],
    );
}

/// The same overflow on page one of a document whose last section has a
/// narrower right margin. A whole repaint ends on that last section, so the
/// renderer's own margins are not the band page's.
#[test]
fn an_overflow_in_an_earlier_section_matches_a_full_repaint() {
    let name = "word-mac-probe-ruby-wide-grid";
    let bytes = with_document(&fixture(name), |xml| {
        let start = xml.rfind("<w:sectPr").expect("a section");
        let end = xml.rfind("</w:sectPr>").expect("a section end") + "</w:sectPr>".len();
        let first = &xml[start..end];
        let last = first.replace(r#"w:right="1440""#, r#"w:right="200""#);
        assert_ne!(first, last, "{name}: the right margin to change");
        format!(
            "{}<w:p><w:pPr>{first}</w:pPr></w:p><w:p><w:r><w:t>next section</w:t></w:r></w:p>{last}{}",
            &xml[..start],
            &xml[end..]
        )
    });
    let long = "wrap ".repeat(12);
    check_edits(
        name,
        &bytes,
        28,
        &[
            Edit::Insert(&long),
            Edit::RepaintAll,
            Edit::Delete(60),
            Edit::Insert("a"),
        ],
    );
}

/// Every paragraph of this long document carries white shading, which paints
/// nothing on white paper: typing keeps the band.
#[test]
fn typing_in_white_shaded_paragraphs_takes_the_band() {
    let name = "yokohama";
    let mut document = open_bytes(&fixture(name));
    let paragraph = paragraphs_on_first_page(&mut document, 2)[1];
    let taken = band_taken(&document);
    for step in 0..12 {
        if step % 2 == 0 {
            document.insert_text(paragraph, 0, "a", 1)
        } else {
            document.delete_text(paragraph, 0, false, 1, 1)
        }
        .expect("keystroke");
    }
    let (_, now, fallbacks) = document.renderer().band_redraw_stats();
    assert!(
        now - taken >= 10,
        "{name}: typing should take the band: {} of 12, fallbacks {fallbacks:?}",
        now - taken
    );
}
