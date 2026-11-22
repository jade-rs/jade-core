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
    zip_docx(&body)
}

/// A Letter-sized package around `body`, ending with the links' bookmark.
fn zip_docx(body: &str) -> Vec<u8> {
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

fn run(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

fn para(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}

/// `count` filler paragraphs, every `link_every`-th ending in a link.
fn filler_paras(count: usize, link_every: usize) -> String {
    (0..count)
        .map(|i| {
            let tail = if i % link_every == 0 {
                link("target", "linked words")
            } else {
                run(" plain words")
            };
            para(&format!("{}{tail}", run(&format!("F{i} {FILLER}"))))
        })
        .collect()
}

fn find(document: &NativeDocument, prefix: &str) -> usize {
    (0..3000)
        .find(|&i| {
            document
                .paragraph_text(i)
                .is_some_and(|text| text.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("no paragraph starting {prefix:?}"))
}

fn page_of(document: &mut NativeDocument, paragraph: usize) -> usize {
    let caret = document.place_caret(paragraph, 0).expect("caret");
    document.page_and_offset(caret.document_y).0
}

/// Compares the registry and every painted page with a fresh full render,
/// collecting what differs.
fn check(document: &NativeDocument, label: &str, problems: &mut Vec<String>) {
    let fresh = open_bytes(&document.export_docx().expect("export"));
    let typed = normalized(document.renderer().hyperlink_regions());
    let expected = normalized(fresh.renderer().hyperlink_regions());
    if typed != expected {
        let extra: Vec<_> = typed.iter().filter(|r| !expected.contains(r)).collect();
        let missing: Vec<_> = expected.iter().filter(|r| !typed.contains(r)).collect();
        problems.push(format!(
            "{label}: regions differ: extra {extra:#?} missing {missing:#?}"
        ));
    }
    if document.page_count() != fresh.page_count() {
        problems.push(format!(
            "{label}: page count {} vs {}",
            document.page_count(),
            fresh.page_count()
        ));
        return;
    }
    for page in 0..document.page_count() {
        if document.is_page_stale(page) {
            continue;
        }
        let a = document.page_pixels(page).expect("pixels").0;
        let b = fresh.page_pixels(page).expect("fresh pixels").0;
        let differing = a
            .chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        if differing != 0 {
            problems.push(format!(
                "{label}: page {page} differs in {differing} pixels"
            ));
        }
    }
}

/// Typing before, inside and after a link in the edited paragraph, then
/// `replace_range` shrinking the link and removing it.
#[test]
fn typing_inside_a_link_and_removing_it_keep_the_registry() {
    let mut body = filler_paras(40, 7);
    body.push_str(&para(&format!(
        "{}{}{}",
        run("EDIT before "),
        link("target", "linked words here"),
        run(" after text")
    )));
    body.push_str(&filler_paras(40, 5));
    let mut document = open_bytes(&zip_docx(&body));
    let paragraph = find(&document, "EDIT");
    let all = document.page_count();
    let mut problems = Vec::new();
    for (i, offset) in [2usize, 3, 15, 16, 18, 40, 12, 13].into_iter().enumerate() {
        document
            .insert_text(paragraph, offset, "x", all)
            .expect("insert");
        check(&document, &format!("insert#{i}@{offset}"), &mut problems);
    }
    let text = document.paragraph_text(paragraph).unwrap();
    let words = text.find("words").unwrap();
    document
        .replace_range(paragraph, words + 1, words + 4, "", all)
        .expect("replace inside the link");
    check(&document, "replace-in-link", &mut problems);
    let text = document.paragraph_text(paragraph).unwrap();
    let link_start = text.find(" l").unwrap() + 1;
    let link_end = text.find(" after").unwrap();
    document
        .replace_range(paragraph, link_start, link_end, "", all)
        .expect("remove the link text");
    check(&document, "remove-link", &mut problems);
    document
        .insert_text(paragraph, 1, "y", all)
        .expect("insert");
    check(&document, "after-remove", &mut problems);
    document
        .delete_text(paragraph, 3, true, 1, all)
        .expect("delete");
    check(&document, "delete", &mut problems);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Links long enough to wrap, in paragraphs at a page boundary: typing before
/// and inside them, in the first paragraph of a page and either side of it.
#[test]
fn typing_around_multi_line_links_at_page_boundaries_keeps_the_registry() {
    let long_link = "long link text that spans across more than one line of the page so \
         it has several regions and keeps going and going until it wraps around again";
    let body: String = (0..70)
        .map(|i| {
            para(&format!(
                "{}{}{}",
                run(&format!("M{i} lead ")),
                link("target", long_link),
                run(" tail text")
            ))
        })
        .collect();
    let mut document = open_bytes(&zip_docx(&body));
    let mut problems = Vec::new();
    check(&document, "initial", &mut problems);
    for target_page in [1usize, 2] {
        let paragraphs: Vec<usize> = (0..70)
            .map(|i| find(&document, &format!("M{i} ")))
            .collect();
        let first = paragraphs
            .into_iter()
            .find(|&p| page_of(&mut document, p) == target_page)
            .expect("a paragraph starting the page");
        for paragraph in [first, first - 1, first + 1] {
            for i in 0..4 {
                let all = document.page_count();
                document
                    .insert_text(paragraph, 6, "x", all)
                    .expect("insert");
                check(
                    &document,
                    &format!("para{paragraph} lead#{i}"),
                    &mut problems,
                );
            }
            for i in 0..2 {
                let text = document.paragraph_text(paragraph).unwrap();
                let inside = text.find("spans").unwrap();
                let all = document.page_count();
                document
                    .insert_text(paragraph, inside, "y", all)
                    .expect("insert");
                check(
                    &document,
                    &format!("para{paragraph} inside#{i}"),
                    &mut problems,
                );
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
