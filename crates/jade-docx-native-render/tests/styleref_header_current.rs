//! A header's STYLEREF shows the headings of the page it is drawn on, as that
//! page's own layout places them: on a fresh open, after a restyle or an undo,
//! and for a heading pagination pushes onto the next page. A header is drawn
//! before its page's body, so a pass that took its headings from the layout
//! before it drew the previous pass's headings.

mod common;

use common::{open_bytes, Doc};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const FILLER: &str = "The quick brown fox jumps over the lazy dog while the band \
     redraw keeps every other pixel of the page exactly as it was painted.";

const STYLES: &str = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>"#;

/// A Letter page with `body` under a header holding `header` (paragraph
/// content).
fn docx(body: &str, header: &str) -> Vec<u8> {
    docx_with_footer(body, header, None)
}

/// [`docx`], with a footer holding `footer` when there is one.
fn docx_with_footer(body: &str, header: &str, footer: Option<&str>) -> Vec<u8> {
    use std::io::Write;
    let footer_reference = if footer.is_some() {
        r#"<w:footerReference w:type="default" r:id="rId3"/>"#
    } else {
        ""
    };
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr><w:headerReference w:type="default" r:id="rId2"/>{footer_reference}<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="{W}"><w:p>{header}</w:p></w:hdr>"#
    );
    let footer = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="{W}"><w:p>{}</w:p></w:ftr>"#,
        footer.unwrap_or_default()
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}">{STYLES}</w:styles>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", document),
            ("word/header1.xml", header),
            ("word/footer1.xml", footer),
            ("word/styles.xml", styles),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    bytes
}

const NOW_HEADER: &str = r#"<w:r><w:t xml:space="preserve">Now: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; "><w:r><w:t>stale</w:t></w:r></w:fldSimple>"#;

/// Eight chapters, each a heading and 18 paragraphs of text; `normal` lists
/// the chapters whose heading is a Normal paragraph instead.
fn chapters_docx(normal: &[usize]) -> Vec<u8> {
    docx(&chapters_body(normal), NOW_HEADER)
}

fn chapters_body(normal: &[usize]) -> String {
    let mut body = String::new();
    for chapter in 0..8 {
        let style = if normal.contains(&chapter) {
            "Normal"
        } else {
            "Heading1"
        };
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>Chapter {chapter}</w:t></w:r></w:p>"#
        ));
        for index in 0..18 {
            body.push_str(&format!(
                "<w:p><w:r><w:t>{chapter}.{index} {FILLER}</w:t></w:r></w:p>"
            ));
        }
    }
    body
}

/// The paragraph index of chapter `chapter`'s heading.
fn heading(chapter: usize) -> usize {
    chapter * 19
}

fn page_of(document: &mut Doc, paragraph: usize) -> usize {
    let rect = document.position_rect(paragraph, 0).unwrap();
    document.page_and_offset(rect.document_y).0
}

fn assert_same_pages(name: &str, document: &Doc, reference: &Doc) {
    assert_eq!(document.page_count(), reference.page_count(), "{name}");
    for page in 0..reference.page_count() {
        assert!(
            document.page_pixels(page) == reference.page_pixels(page),
            "{name}: page {page} differs"
        );
    }
}

/// Every page is stale or as the reference draws it; once the stale pages
/// are drawn, every page is as the reference draws it.
fn assert_stale_or_right_then_right(name: &str, document: &mut Doc, reference: &Doc) {
    for page in 0..reference.page_count() {
        assert!(
            document.is_page_stale(page)
                || document.page_pixels(page) == reference.page_pixels(page),
            "{name}: page {page} wrong and not stale"
        );
    }
    document.paint_stale_pages(0..usize::MAX).unwrap();
    assert_same_pages(
        &format!("{name}, after the stale paint"),
        document,
        reference,
    );
}

#[test]
fn a_fresh_open_draws_the_headings_of_each_page() {
    let bytes = chapters_docx(&[]);
    let document = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    // A second layout has every heading's page from the one before it.
    reference.repaint_all().unwrap();
    assert!(!reference.has_stale_pages());
    assert_same_pages("fresh open", &document, &reference);
    assert!(!document.has_stale_pages());
}

#[test]
fn restyling_a_heading_leaves_its_pages_stale_or_right() {
    let bytes = chapters_docx(&[]);
    let reference = open_bytes(&chapters_docx(&[1]));
    for bound in [Some(()), None] {
        let mut document = open_bytes(&bytes);
        let page = page_of(&mut document, heading(1));
        assert!(page > 0, "chapter 1 starts after the first page");
        document.place_caret(heading(1), 0);
        let visible_end = bound.map_or(usize::MAX, |_| page + 1);
        document.set_paragraph_style("Normal", visible_end).unwrap();
        assert_stale_or_right_then_right(
            &format!("restyle, bound {visible_end}"),
            &mut document,
            &reference,
        );
    }
}

#[test]
fn undoing_a_heading_edit_draws_the_heading_back() {
    let bytes = chapters_docx(&[]);
    let reference = open_bytes(&bytes);
    let mut document = open_bytes(&bytes);
    document
        .insert_text(heading(1), 0, "Q", usize::MAX)
        .unwrap();
    document.paint_stale_pages(0..usize::MAX).unwrap();
    document.undo().unwrap();
    assert_stale_or_right_then_right("undo", &mut document, &reference);
}

/// `heading 1` paragraphs `A`, filler, then `B`, which keeps its lines
/// together: `filler` lines of filler before it, and `B` with a page break
/// before it when `break_before`.
fn pushed_docx(filler: usize, break_before: bool) -> Vec<u8> {
    let mut body = String::from(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter A</w:t></w:r></w:p>"#,
    );
    for index in 0..filler {
        body.push_str(&format!("<w:p><w:r><w:t>Line {index}</w:t></w:r></w:p>"));
    }
    let break_before = if break_before {
        "<w:pageBreakBefore/>"
    } else {
        ""
    };
    body.push_str(&format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:keepLines/>{break_before}</w:pPr><w:r><w:t>Chapter B {FILLER} {FILLER}</w:t></w:r></w:p>"#
    ));
    body.push_str("<w:p><w:r><w:t>After</w:t></w:r></w:p>");
    let header = r#"<w:r><w:t xml:space="preserve">First: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; "><w:r><w:t>stale</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> Last: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; \l "><w:r><w:t>stale</w:t></w:r></w:fldSimple>"#;
    docx(&body, header)
}

#[test]
fn a_heading_pushed_to_the_next_page_is_on_that_page() {
    // The filler that leaves B's lines no room at the foot of the first page
    // but its first line some: B is pushed to the second page whole.
    let (filler, mut document) = (20..80)
        .find_map(|filler| {
            let mut document = open_bytes(&pushed_docx(filler, false));
            let heading = filler + 1;
            let last_filler = document.position_rect(filler, 0).unwrap();
            let last_filler_page = document.page_and_offset(last_filler.document_y).0;
            let room = document.page_box(0).1 - 96.0 - last_filler.document_y;
            (last_filler_page == 0 && page_of(&mut document, heading) == 1 && room > 60.0)
                .then_some((filler, document))
        })
        .expect("some filler pushes B to the second page");
    let mut reference = open_bytes(&pushed_docx(filler, true));
    // A second layout, so neither header waits on the pass before it.
    document.repaint_all().unwrap();
    reference.repaint_all().unwrap();
    // `\l` on the first page shows A, the last heading there. The header
    // sits in the top margin; the body below it is laid out apart.
    assert!(
        header_band(&document, 0) == header_band(&reference, 0),
        "filler {filler}: the first page's header shows B"
    );
}

/// The pixels of page `page` above its body (the 1in top margin).
fn header_band(document: &Doc, page: usize) -> Vec<u8> {
    let (pixels, width, _) = document.page_pixels(page).unwrap();
    let top_margin_rows = 96 * width as usize * 4;
    pixels[..top_margin_rows].to_vec()
}

/// The same STYLEREF in the header and the footer: the footer, drawn after
/// the page's body, shows the restyled heading right, and does not vouch for
/// the header drawn before it.
#[test]
fn a_styleref_in_header_and_footer_is_checked_in_each() {
    let both =
        |normal: &[usize]| docx_with_footer(&chapters_body(normal), NOW_HEADER, Some(NOW_HEADER));
    let reference = open_bytes(&both(&[1]));
    for bound in [true, false] {
        let mut document = open_bytes(&both(&[]));
        let page = page_of(&mut document, heading(1));
        document.place_caret(heading(1), 0);
        let visible_end = if bound { page + 1 } else { usize::MAX };
        document.set_paragraph_style("Normal", visible_end).unwrap();
        assert_stale_or_right_then_right(
            &format!("header and footer, bound {visible_end}"),
            &mut document,
            &reference,
        );
    }
}

/// Edits in a row, with nothing drawn between them, each leave their own
/// pages wrong; none of them is a STYLEREF that cannot settle.
#[test]
fn edits_in_a_row_each_leave_their_pages_stale() {
    for bound in [true, false] {
        let mut document = open_bytes(&chapters_docx(&[]));
        let reference = open_bytes(&chapters_docx(&[1, 2, 3]));
        for chapter in [1, 2, 3] {
            let page = page_of(&mut document, heading(chapter));
            document.place_caret(heading(chapter), 0);
            let visible_end = if bound { page + 1 } else { usize::MAX };
            document.set_paragraph_style("Normal", visible_end).unwrap();
        }
        assert_stale_or_right_then_right(
            &format!("three restyles, bound {bound}"),
            &mut document,
            &reference,
        );
    }
    let original = open_bytes(&chapters_docx(&[]));
    let restyled = open_bytes(&chapters_docx(&[1]));
    let mut document = open_bytes(&chapters_docx(&[]));
    document.place_caret(heading(1), 0);
    document.set_paragraph_style("Normal", usize::MAX).unwrap();
    assert_stale_or_right("restyle", &document, &restyled);
    document.undo().unwrap();
    assert_stale_or_right("undo", &document, &original);
    document.redo().unwrap();
    assert_stale_or_right("redo", &document, &restyled);
    document.undo().unwrap();
    assert_stale_or_right_then_right("undo again", &mut document, &original);
}

fn assert_stale_or_right(name: &str, document: &Doc, reference: &Doc) {
    for page in 0..reference.page_count() {
        assert!(
            document.is_page_stale(page)
                || document.page_pixels(page) == reference.page_pixels(page),
            "{name}: page {page} wrong and not stale"
        );
    }
}

/// `heading 1` A, filler, B (four short lines kept together, pushed onto
/// the second page, or put there by a page break before it), text, and C on
/// the second page. The headings are short so the header and footer showing
/// them keep their height whichever they show.
fn pushed_then_c_docx(filler: usize, break_before: bool) -> Vec<u8> {
    let mut body = String::from(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter A</w:t></w:r></w:p>"#,
    );
    for index in 0..filler {
        body.push_str(&format!("<w:p><w:r><w:t>Line {index}</w:t></w:r></w:p>"));
    }
    let break_before = if break_before {
        "<w:pageBreakBefore/>"
    } else {
        ""
    };
    body.push_str(&format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:keepLines/>{break_before}</w:pPr><w:r><w:t>Chapter B</w:t><w:br/><w:t>two</w:t><w:br/><w:t>three</w:t><w:br/><w:t>four</w:t></w:r></w:p><w:p><w:r><w:t>After</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter C</w:t></w:r></w:p><w:p><w:r><w:t>After C</w:t></w:r></w:p>"#
    ));
    let fields = r#"<w:r><w:t xml:space="preserve">First: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; "><w:r><w:t>stale</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> Last: </w:t></w:r><w:fldSimple w:instr=" STYLEREF &quot;heading 1&quot; \l "><w:r><w:t>stale</w:t></w:r></w:fldSimple>"#;
    docx_with_footer(&body, fields, Some(fields))
}

/// The page break that pushes B draws the first page's footer and the second
/// page's header before B is placed: they show A and B, as the layout with B
/// after a page break does, from the first open on, and nothing is left
/// stale for later.
#[test]
fn a_pushed_heading_is_on_its_page_while_the_push_draws_the_chrome() {
    let (filler, mut document) = (20..80)
        .find_map(|filler| {
            let mut document = open_bytes(&pushed_then_c_docx(filler, false));
            let last_filler = document.position_rect(filler, 0).unwrap();
            let last_filler_page = document.page_and_offset(last_filler.document_y).0;
            let room = document.page_box(0).1 - 96.0 - last_filler.document_y;
            (last_filler_page == 0 && page_of(&mut document, filler + 1) == 1 && room > 60.0)
                .then_some((filler, document))
        })
        .expect("some filler pushes B to the second page");
    assert_eq!(page_of(&mut document, filler + 3), 1, "C is on B's page");
    let reference = open_bytes(&pushed_then_c_docx(filler, true));
    for round in 0..3 {
        assert!(
            !document.has_stale_pages(),
            "round {round}: a page is stale"
        );
        for page in 0..2 {
            assert!(
                header_band(&document, page) == header_band(&reference, page),
                "round {round}: page {page}'s header"
            );
            assert!(
                footer_band(&document, page) == footer_band(&reference, page),
                "round {round}: page {page}'s footer"
            );
        }
        document.repaint_all().unwrap();
    }
}

/// The pixels of page `page` below its body (the 1in bottom margin).
fn footer_band(document: &Doc, page: usize) -> Vec<u8> {
    let (pixels, width, _) = document.page_pixels(page).unwrap();
    let bottom_margin_rows = 96 * width as usize * 4;
    pixels[pixels.len() - bottom_margin_rows..].to_vec()
}
