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
    /// A paragraph break at offset 3 of paragraph 0, which moves every later
    /// paragraph down an index; later edits address the edited paragraph's
    /// new index.
    SplitFirst,
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
    let mut paragraph = paragraph;
    for (step, edit) in edits.iter().enumerate() {
        for (document, bound) in [(&mut actual, 1), (&mut reference, usize::MAX)] {
            match *edit {
                Edit::Insert(text) => document.insert_text(paragraph, 0, text, bound),
                Edit::Delete(count) => document.delete_text(paragraph, 0, false, count, bound),
                Edit::SplitFirst => document.insert_paragraph_break(0, 3),
                Edit::RepaintAll => document.repaint_all(),
            }
            .expect("edit");
        }
        if matches!(edit, Edit::SplitFirst) {
            paragraph += 1;
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

/// A Letter package around `body`, with `styles` in its style part.
fn package(body: &str, styles: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}">{styles}</w:styles>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_string(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_string(),
            ),
            ("word/styles.xml", styles),
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
    check_typing("bordered headings", &package(&body, ""), 6);
}

/// Eight paragraphs, each given `ppr` and `rpr` when `on(index)`.
fn paragraphs(ppr: &str, rpr: &str, on: impl Fn(usize) -> bool) -> String {
    (0..8)
        .map(|index| {
            let (ppr, rpr) = if on(index) { (ppr, rpr) } else { ("", "") };
            format!(
                r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{index} Body text with descenders, gypsy quay</w:t></w:r></w:p>"#
            )
        })
        .collect()
}

/// A wave underline's crests and an under-dot emphasis mark fall below the
/// descent the line ink guard measures, whether set directly or by a style.
#[test]
fn typing_over_marks_below_the_line_matches_a_full_repaint() {
    let styles = r#"<w:style w:type="character" w:styleId="Wave"><w:name w:val="Wave"/><w:rPr><w:u w:val="wave"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Dots"><w:name w:val="Dots"/><w:rPr><w:em w:val="underDot"/></w:rPr></w:style>"#;
    for (name, ppr, rpr) in [
        ("wave", "", r#"<w:u w:val="wave"/>"#),
        ("wave style", "", r#"<w:rStyle w:val="Wave"/>"#),
        ("under-dot style", r#"<w:pStyle w:val="Dots"/>"#, ""),
    ] {
        for odd in [false, true] {
            let body = paragraphs(ppr, rpr, |index| !odd || index % 2 == 1);
            check_typing(name, &package(&body, styles), 6);
        }
    }
}

/// `w:between` draws a rule under every paragraph of a border box but the
/// last; side borders run beside every line of the box.
#[test]
fn typing_in_and_beside_border_boxes_matches_a_full_repaint() {
    let edge =
        |side: &str| format!(r#"<w:{side} w:val="single" w:sz="8" w:space="4" w:color="FF0000"/>"#);
    let between = format!("<w:pBdr>{}</w:pBdr>", edge("between"));
    let sides = format!("<w:pBdr>{}{}</w:pBdr>", edge("left"), edge("right"));
    check_typing(
        "between",
        &package(&paragraphs(&between, "", |_| true), ""),
        6,
    );
    check_typing(
        "boxes either side",
        &package(&paragraphs(&sides, "", |index| index % 2 == 0), ""),
        6,
    );
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

/// The overflow is recorded with the paragraph's layout, so it follows the
/// paragraph to its new index when a break above it shifts the indexes.
#[test]
fn an_overflow_moved_down_an_index_matches_a_full_repaint() {
    let long = "wrap ".repeat(12);
    let name = "word-mac-probe-ruby-wide-grid";
    check_edits(
        name,
        &fixture(name),
        27,
        &[
            Edit::Insert(&long),
            Edit::SplitFirst,
            Edit::Delete(60),
            Edit::Insert("a"),
        ],
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

/// A two-column table of `rows` rows, `shading` filling every cell, its
/// rows exactly `height` twips tall, and its cell text `top` twips below
/// each row's top.
fn table(shading: &str, rows: usize, height: u32, top: u32) -> String {
    let border =
        |side: &str| format!(r#"<w:{side} w:val="single" w:sz="8" w:space="0" w:color="000000"/>"#);
    let borders: String = ["top", "left", "bottom", "right", "insideH", "insideV"]
        .into_iter()
        .map(border)
        .collect();
    let cell = |text: String| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="{shading}"/></w:tcPr><w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let rows: String = (0..rows)
        .map(|row| {
            format!(
                r#"<w:tr><w:trPr><w:trHeight w:val="{height}" w:hRule="exact"/></w:trPr>{}{}</w:tr>"#,
                cell(format!("cell {row} a, gypsy")),
                cell(format!("cell {row} b, quay")),
            )
        })
        .collect();
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="8000" w:type="dxa"/><w:tblBorders>{borders}</w:tblBorders><w:tblCellMar><w:top w:w="{top}" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid>{rows}</w:tbl>"#
    )
}

/// An exact line `line` twips high, without spacing before or after.
fn exact(line: u32) -> String {
    format!(r#"<w:spacing w:before="0" w:after="0" w:line="{line}" w:lineRule="exact"/>"#)
}

/// Eight edits at the start of `paragraph`.
const TYPING: [Edit<'static>; 8] = [
    Edit::Insert("a"),
    Edit::Delete(1),
    Edit::Insert("a"),
    Edit::Insert("bb"),
    Edit::Delete(2),
    Edit::Insert("W"),
    Edit::Delete(1),
    Edit::Insert("g"),
];

/// A tall exact-height paragraph ends a section at a thin continuous
/// section break, and a table starts the next section on the same page.
/// The band's clip reaches through the break into the table's top border
/// and shading. The table paints by row, culled by the row's top, which
/// lies below the break's line: the walk past the break must stop at the
/// table's top edge, not at its cells' text.
#[test]
fn typing_above_a_section_break_and_a_table_matches_a_full_repaint() {
    let continuous = r#"<w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;
    let body = format!(
        r#"{}<w:p><w:pPr>{}</w:pPr><w:r><w:t xml:space="preserve">Tall exact line, gypsy quay</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="0" w:after="2" w:line="1" w:lineRule="exact"/>{continuous}</w:pPr></w:p>{}{}"#,
        text_paragraph("", "0 Body text before, gypsy"),
        exact(1200),
        table("D9E2F3", 2, 1140, 300),
        text_paragraph("", "after the table"),
    );
    let bytes = with_document(&package(&body, ""), |xml| {
        xml.replace(
            "<w:sectPr><w:pgSz",
            r#"<w:sectPr><w:type w:val="continuous"/><w:pgSz"#,
        )
    });
    check_edits("exact line above a section break", &bytes, 1, &TYPING);
}

/// A shaded table above a 1 pt exact-height spacer paragraph, with rows
/// taller than the band's slack above it. The band of the paragraph under
/// the spacer reaches through the spacer into the last row's shading and
/// bottom border, but the row is culled by its top and not repainted.
#[test]
fn typing_below_a_spacer_under_a_table_matches_a_full_repaint() {
    let body = format!(
        "{}{}<w:p><w:pPr>{}</w:pPr></w:p>{}{}{}{}",
        text_paragraph("", "0 Body text before, gypsy"),
        table("D9E2F3", 2, 2400, 0),
        exact(20),
        text_paragraph("", "after the spacer, gypsy quay"),
        text_paragraph("", "and the next paragraph"),
        text_paragraph("", "and two more"),
        text_paragraph("", "paragraphs under them"),
    );
    let bytes = package(&body, "");
    // The paragraph under the spacer, then the one under that, which the
    // band still takes.
    check_edits("spacer under a table", &bytes, 6, &TYPING);
    let taken = check_edits("spacer under a table", &bytes, 7, &TYPING);
    assert!(taken > 0, "a paragraph away from the table keeps the band");
}

fn text_paragraph(ppr: &str, text: &str) -> String {
    format!(r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}
