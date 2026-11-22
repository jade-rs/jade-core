//! Drawing pages a windowed open left stale resumes layout at the first of
//! them and stops after the last: the cost follows the pages drawn, and the
//! pixels are the ones a plain open draws.

mod common;

use common::repo_path;
use jade_docx_native_render::{FontFace, NativeDocument};
use std::time::{Duration, Instant};

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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_path(&format!("public/fixtures/{name}"))).unwrap()
}

fn open_both(bytes: &[u8], window: std::ops::Range<usize>) -> (NativeDocument, NativeDocument) {
    let faces = faces();
    let full = NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces).unwrap();
    let windowed =
        NativeDocument::open_with_font_faces_and_paint_window(bytes, 816.0, 1056.0, &faces, window)
            .unwrap();
    (full, windowed)
}

fn body_walk(document: &NativeDocument) -> u64 {
    document.renderer().last_render_body_element_count()
}

fn full(bytes: &[u8]) -> NativeDocument {
    NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces()).unwrap()
}

fn windowed(bytes: &[u8], window: std::ops::Range<usize>) -> NativeDocument {
    NativeDocument::open_with_font_faces_and_paint_window(bytes, 816.0, 1056.0, &faces(), window)
        .unwrap()
}

fn mismatched_pages(a: &NativeDocument, b: &NativeDocument) -> Vec<usize> {
    let pages = a.page_count().max(b.page_count());
    (0..pages)
        .filter(|&page| a.page_pixels(page) != b.page_pixels(page))
        .collect()
}

/// Points on every page where the two documents place the caret differently.
fn hit_mismatches(a: &NativeDocument, b: &NativeDocument) -> usize {
    let pages = a.page_count().min(b.page_count());
    let mut bad = 0;
    for page in 0..pages {
        for (fx, fy) in [(0.3, 0.2), (0.5, 0.5), (0.7, 0.8), (0.2, 0.9)] {
            let (width, height) = a.page_box(page);
            let y = a.document_y(page, height * fy);
            if a.position_at(width * fx, y) != b.position_at(width * fx, y) {
                bad += 1;
            }
        }
    }
    bad
}

/// Links, comments, images, text boxes, revisions and list numbers the last
/// pass registered.
fn registries(document: &NativeDocument) -> [usize; 6] {
    let renderer = document.renderer();
    [
        renderer.hyperlink_regions().len(),
        renderer.comment_annotations().len(),
        renderer.image_debug_regions().len(),
        renderer.text_box_debug_regions().len(),
        renderer.revision_debug_regions().len(),
        renderer.numbering_debug_regions().len(),
    ]
}

fn paint_one_by_one(document: &mut NativeDocument) {
    let pages = document.page_count();
    for page in 0..pages {
        if document.is_page_stale(page) {
            document.paint_stale_pages(page..page + 1).unwrap();
            assert_eq!(document.page_count(), pages, "page count after {page}");
        }
    }
}

/// Pixels, page count, hit-testing and registered regions all agree.
fn assert_same(name: &str, a: &NativeDocument, b: &NativeDocument) {
    assert_eq!(a.page_count(), b.page_count(), "{name}: page count");
    assert_eq!(
        mismatched_pages(a, b),
        Vec::<usize>::new(),
        "{name}: pixels"
    );
    assert_eq!(hit_mismatches(a, b), 0, "{name}: hit-testing");
    assert_eq!(registries(a), registries(b), "{name}: regions");
}

#[test]
fn stale_pages_drawn_one_by_one_match_a_plain_open_and_walk_only_their_span() {
    let (full, mut windowed) = open_both(&fixture("yokohama.docx"), 0..3);
    let pages = full.page_count();
    assert!(pages > 100, "a long document: {pages} pages");
    let whole_body = body_walk(&full);
    assert!(whole_body > 0);

    for page in 3..pages {
        assert!(windowed.is_page_stale(page), "page {page}");
        assert!(windowed.paint_stale_pages(page..page + 1).unwrap());
        assert!(!windowed.is_page_stale(page), "page {page}");
        assert_eq!(windowed.page_count(), pages, "after page {page}");
        let walked = body_walk(&windowed);
        assert!(
            walked * 10 < whole_body,
            "page {page} walked {walked} of {whole_body} body elements"
        );
    }
    assert!(!windowed.has_stale_pages());
    assert_same("yokohama one by one", &windowed, &full);
}

#[test]
fn a_span_near_the_end_walks_a_small_part_of_the_body() {
    let (full, mut windowed) = open_both(&fixture("yokohama.docx"), 0..3);
    let pages = full.page_count();
    let whole_body = body_walk(&full);

    let span = pages - 4..pages - 1;
    assert!(windowed.paint_stale_pages(span.clone()).unwrap());
    let walked = body_walk(&windowed);
    assert!(
        walked * 10 < whole_body,
        "walked {walked} of {whole_body} body elements"
    );
    assert_eq!(windowed.page_count(), pages);
    // The walk covered three pages; the regions of the rest are kept.
    assert!(registries(&full)[0] > 0, "yokohama has links");
    assert_eq!(registries(&windowed), registries(&full));
    assert_eq!(hit_mismatches(&windowed, &full), 0);
    for page in 0..pages {
        assert_eq!(
            windowed.is_page_stale(page),
            !(0..3).contains(&page) && !span.contains(&page),
            "page {page}"
        );
    }
    for page in span {
        assert_eq!(
            windowed.page_pixels(page),
            full.page_pixels(page),
            "page {page}"
        );
    }
    // The pages after the span are still drawn correctly afterwards.
    assert!(windowed.paint_stale_pages(pages - 1..pages).unwrap());
    assert_eq!(windowed.page_pixels(pages - 1), full.page_pixels(pages - 1));
}

/// With notes shown the renderer has no page-start seek, and the pages are
/// drawn by laying the document out from the start, as before.
#[test]
fn a_document_with_footnotes_still_draws_its_stale_pages() {
    let (full, mut windowed) = open_both(&fixture("footnote-long-continuation.docx"), 0..1);
    let pages = full.page_count();
    assert!(pages > 1, "{pages} pages");
    assert_eq!(windowed.page_count(), pages);

    for page in 1..pages {
        assert!(windowed.is_page_stale(page), "page {page}");
        assert!(windowed.paint_stale_pages(page..page + 1).unwrap());
        assert_eq!(windowed.page_count(), pages, "after page {page}");
    }
    assert!(!windowed.has_stale_pages());
    assert_same("footnotes", &windowed, &full);
}

/// A bounded edit leaves the layout after its bound out of date, with the
/// page-start snapshots, line registry and regions there. The stale pages
/// are then laid out from the start, as the edited document now flows.
#[test]
fn stale_pages_after_a_bounded_delete_follow_the_edited_layout() {
    let bytes = fixture("yokohama.docx");
    let mut windowed = windowed(&bytes, 0..3);
    let mut full = full(&bytes);
    let mut paragraph = 40;
    while windowed
        .paragraph_text(paragraph)
        .map(|text| text.chars().count())
        .unwrap_or(0)
        < 120
    {
        paragraph += 1;
    }
    windowed.delete_text(paragraph, 0, false, 100, 3).unwrap();
    full.delete_text(paragraph, 0, false, 100, usize::MAX)
        .unwrap();
    full.repaint_all().unwrap();
    paint_one_by_one(&mut windowed);
    assert_same("yokohama delete", &windowed, &full);

    // Page-top snapshots only: every page starts with a paragraph.
    let bytes = plain_docx(400);
    let mut windowed = self::windowed(&bytes, 0..2);
    let mut full = self::full(&bytes);
    windowed.delete_text(2, 0, false, 100, 2).unwrap();
    full.delete_text(2, 0, false, 100, usize::MAX).unwrap();
    full.repaint_all().unwrap();
    paint_one_by_one(&mut windowed);
    assert_same("plain delete", &windowed, &full);
}

/// A bounded selection stops registering lines at its bound; hit-testing on
/// the pages after it needs a whole layout.
#[test]
fn stale_pages_after_a_bounded_selection_keep_hit_testing() {
    let bytes = fixture("yokohama.docx");
    let mut windowed = windowed(&bytes, 0..3);
    let mut full = full(&bytes);
    let last = windowed.paragraph_count() - 1;
    windowed.select((30, 0), (last, 0), 3).unwrap();
    full.select((30, 0), (last, 0), usize::MAX).unwrap();
    full.repaint_all().unwrap();
    paint_one_by_one(&mut windowed);
    assert_same("select", &windowed, &full);
}

/// Section `w:vAlign` offsets depend on every page's used height, so these
/// pages are laid out from the start, and the offsets survive.
#[test]
fn vertically_aligned_sections_draw_their_stale_pages() {
    for align in ["center", "bottom"] {
        let bytes = valign_docx(align);
        let full = full(&bytes);
        let mut windowed = windowed(&bytes, 0..1);
        windowed.paint_stale_pages(1..3).unwrap();
        for page in 0..3 {
            assert_eq!(
                windowed.page_pixels(page),
                full.page_pixels(page),
                "{align}: page {page}"
            );
        }
        windowed.paint_stale_pages(3..usize::MAX).unwrap();
        assert_same(&format!("{align} spans"), &windowed, &full);

        let mut windowed = self::windowed(&bytes, 0..1);
        paint_one_by_one(&mut windowed);
        assert_same(&format!("{align} one by one"), &windowed, &full);

        // Every page drawn on what follows, so none is left with old pixels.
        windowed.set_paint_window(None);
        let mut reference = self::full(&bytes);
        windowed.select((0, 0), (0, 3), 1).unwrap();
        reference.select((0, 0), (0, 3), 1).unwrap();
        assert_same(&format!("{align} select"), &windowed, &reference);
        windowed.repaint_all().unwrap();
        reference.repaint_all().unwrap();
        assert_same(&format!("{align} repaint_all"), &windowed, &reference);
    }
}

/// Links on every page: painting one far page keeps the links of the rest.
#[test]
fn hyperlink_regions_survive_a_stale_page_paint() {
    let mut body = String::new();
    for index in 0..400 {
        let tail = if index % 7 == 0 {
            r#"<w:hyperlink w:anchor="target"><w:r><w:t>linked words</w:t></w:r></w:hyperlink>"#
        } else {
            "<w:r><w:t xml:space=\"preserve\"> plain words</w:t></w:r>"
        };
        body.push_str(&format!(
            "<w:p><w:r><w:t xml:space=\"preserve\">{FILLER}</w:t></w:r>{tail}</w:p>"
        ));
    }
    let bytes = zip_docx(&body, "");
    let full = full(&bytes);
    let mut windowed = windowed(&bytes, 0..1);
    let pages = full.page_count();
    let links = full.renderer().hyperlink_regions().len();
    assert!(links > 0);
    let far = pages - 2;
    windowed.paint_stale_pages(far..far + 1).unwrap();
    assert_eq!(windowed.renderer().hyperlink_regions().len(), links);
    assert_eq!(
        windowed.renderer().page_hyperlink_regions(0).len(),
        full.renderer().page_hyperlink_regions(0).len()
    );
    paint_one_by_one(&mut windowed);
    assert_same("links", &windowed, &full);
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn zip_docx(body: &str, sect_extra: &str) -> Vec<u8> {
    use std::io::Write;
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}<w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>Target</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>{sect_extra}</w:sectPr></w:body></w:document>"#
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

const FILLER: &str = "The quick brown fox jumps over the lazy dog while the band \
     redraw keeps every other pixel of the page exactly as it was painted.";

fn plain_docx(paragraphs: usize) -> Vec<u8> {
    let mut body = String::new();
    for index in 0..paragraphs {
        body.push_str(&format!(
            "<w:p><w:r><w:t xml:space=\"preserve\">{index} {FILLER}</w:t></w:r></w:p>"
        ));
    }
    zip_docx(&body, "")
}

/// Twelve short pages in one vertically aligned section.
fn valign_docx(align: &str) -> Vec<u8> {
    let mut body = String::new();
    for page in 0..12 {
        let page_break = if page == 0 {
            ""
        } else {
            "<w:pPr><w:pageBreakBefore/></w:pPr>"
        };
        body.push_str(&format!(
            "<w:p>{page_break}<w:r><w:t>Title of page {page}</w:t></w:r></w:p>"
        ));
        for _ in 0..(page % 4) {
            body.push_str(&format!("<w:p><w:r><w:t>{FILLER}</w:t></w:r></w:p>"));
        }
    }
    zip_docx(&body, &format!(r#"<w:vAlign w:val="{align}"/>"#))
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

/// `cargo test --release --test stale_page_paint -- --ignored --nocapture`
#[test]
#[ignore]
fn stale_page_paint_timing() {
    let bytes = fixture("yokohama.docx");
    let faces = faces();
    let runs: Vec<_> = (0..7)
        .map(|_| {
            let mut windowed = NativeDocument::open_with_font_faces_and_paint_window(
                &bytes,
                816.0,
                1056.0,
                &faces,
                0..3,
            )
            .unwrap();
            let pages = windowed.page_count();
            let started = Instant::now();
            windowed.paint_stale_pages(pages - 4..pages - 1).unwrap();
            (started.elapsed(), body_walk(&windowed), pages)
        })
        .collect();
    let (_, walked, pages) = runs[0];
    let time = median(runs.into_iter().map(|(elapsed, _, _)| elapsed).collect());
    eprintln!(
        "yokohama ({pages} pages): paint 3 stale pages near the end: median {time:?}, {walked} body elements walked"
    );
}
