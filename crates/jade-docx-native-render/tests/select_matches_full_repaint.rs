//! A selection change repaints only what it has to, and what it leaves is
//! what a full repaint would draw.
//!
//! `select` takes the last page on screen as a bound, as a keystroke does.
//! The bound is about how much work to do, never about what is true: the
//! pixels of every page afterwards have to equal a full repaint, and the
//! layout, hit-test and caret state of the pages past the bound have to be
//! what a freshly opened document reports.

mod common;
use common::{open_bytes, open_fixture, repo_path, Doc};
use std::io::{Read, Write};

const FIXTURES: &[&str] = &[
    "contract.docx",
    "nda.docx",
    "table-complex-pagination.docx",
    "header-footer-sections.docx",
    "footnote-long-continuation.docx",
    "review-kinds.docx",
    "noto-jp-yokohama_page6.docx",
    "footnote-table-each-page.docx",
];

/// `fixture` with `<w:vAlign w:val="{align}"/>` in every section: no fixture
/// has a vertically aligned section.
fn with_valign(fixture: &str, align: &str) -> Vec<u8> {
    let bytes = std::fs::read(repo_path(&format!("public/fixtures/{fixture}"))).expect("read");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut out = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let options = zip::write::SimpleFileOptions::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("entry");
            let name = entry.name().expect("entry name").to_string();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).expect("read entry");
            if name == "word/document.xml" {
                let xml = String::from_utf8(content).expect("utf-8");
                assert!(xml.contains("</w:sectPr>"), "{fixture}: no sectPr");
                content = xml
                    .replace(
                        "</w:sectPr>",
                        &format!(r#"<w:vAlign w:val="{align}"/></w:sectPr>"#),
                    )
                    .into_bytes();
            }
            writer.start_file(name, options).expect("zip entry");
            writer.write_all(&content).expect("zip write");
        }
        writer.finish().expect("zip");
    }
    out
}

/// Every document the select test runs over: the fixtures, and one of them
/// with each vertical alignment.
fn documents() -> Vec<(String, Vec<u8>)> {
    let mut documents: Vec<(String, Vec<u8>)> = FIXTURES
        .iter()
        .map(|fixture| {
            let path = repo_path(&format!("public/fixtures/{fixture}"));
            (
                fixture.to_string(),
                std::fs::read(path).expect("read fixture"),
            )
        })
        .collect();
    for align in ["center", "bottom", "both"] {
        documents.push((
            format!("contract.docx (vAlign {align})"),
            with_valign("contract.docx", align),
        ));
    }
    documents
}

type Pages = Vec<Option<(Vec<u8>, i32, i32)>>;

fn pages(document: &Doc) -> Pages {
    (0..document.page_count())
        .map(|page| document.page_pixels(page))
        .collect()
}

fn differing(a: &Pages, b: &Pages) -> Vec<String> {
    if a.len() != b.len() {
        return vec![format!("page count {} vs {}", a.len(), b.len())];
    }
    (0..a.len())
        .filter(|&page| a[page] != b[page])
        .map(|page| match (&a[page], &b[page]) {
            (Some(x), Some(y)) if x.1 == y.1 && x.2 == y.2 => {
                let mut pixels = x.0.chunks(4).zip(y.0.chunks(4));
                let count = pixels.clone().filter(|(l, r)| l != r).count();
                let first = pixels.position(|(l, r)| l != r).unwrap_or(0) as i32;
                format!(
                    "page {page}: {count} px, first at ({}, {})",
                    first % x.1,
                    first / x.1
                )
            }
            _ => format!("page {page}: surfaces differ in size or presence"),
        })
        .collect()
}

/// Paragraphs with text, with the page each starts on.
fn text_paragraphs(document: &mut Doc) -> Vec<(usize, usize)> {
    let count = document.paragraph_count();
    let mut found = Vec::new();
    for paragraph in 0..count {
        if document
            .paragraph_text(paragraph)
            .is_none_or(|text| text.chars().count() < 4)
        {
            continue;
        }
        if let Some(caret) = document.place_caret(paragraph, 0) {
            found.push((paragraph, document.page_and_offset(caret.document_y).0));
        }
    }
    found
}

/// Hit tests at a few points on every page.
fn hits(document: &Doc) -> Vec<Option<(usize, usize)>> {
    let mut out = Vec::new();
    for page in 0..document.page_count() {
        let (width, height) = document.page_box(page);
        for (fx, fy) in [(0.3, 0.2), (0.5, 0.5), (0.7, 0.8)] {
            out.push(document.position_at(width * fx, document.document_y(page, height * fy)));
        }
    }
    out
}

fn carets(document: &mut Doc, paragraphs: &[usize]) -> Vec<Option<(i64, i64, i64)>> {
    paragraphs
        .iter()
        .map(|&paragraph| {
            document.place_caret(paragraph, 1).map(|c| {
                (
                    c.x.round() as i64,
                    c.document_y.round() as i64,
                    c.height.round() as i64,
                )
            })
        })
        .collect()
}

/// What the last pass registered beyond the text: links, comment marks,
/// objects, revision bars, list numbers and table cells, as they print.
fn registries(document: &Doc) -> Vec<String> {
    let r = document.renderer();
    vec![
        format!("{:?}", r.hyperlink_regions()),
        format!("{:?}", r.comment_annotations()),
        format!("{:?}", r.image_debug_regions()),
        format!("{:?}", r.text_box_debug_regions()),
        format!("{:?}", r.revision_debug_regions()),
        format!("{:?}", r.numbering_debug_regions()),
        format!("{:?}", r.cell_layouts_debug()),
    ]
}

struct Case {
    name: &'static str,
    /// The selections made in turn, each `(anchor, focus)`.
    selections: Vec<((usize, usize), (usize, usize))>,
}

fn cases(paragraphs: &[(usize, usize)], last_page: usize) -> Vec<Case> {
    let first = paragraphs[0].0;
    let second = paragraphs.get(1).map_or(first, |p| p.0);
    let late = paragraphs
        .iter()
        .rev()
        .find(|p| p.1 == last_page)
        .map_or(second, |p| p.0);
    vec![
        Case {
            name: "short",
            selections: vec![((first, 0), (second, 1))],
        },
        Case {
            name: "moved",
            selections: vec![((first, 0), (second, 1)), ((second, 0), (second, 2))],
        },
        Case {
            name: "collapsed",
            selections: vec![((first, 0), (second, 1)), ((first, 1), (first, 1))],
        },
        Case {
            name: "to the last page",
            selections: vec![((first, 0), (late, 1))],
        },
        Case {
            name: "on the last page then cleared",
            selections: vec![((late, 0), (late, 2)), ((first, 0), (first, 0))],
        },
    ]
}

#[test]
fn a_bounded_select_matches_a_full_repaint_and_a_fresh_open() {
    let mut failures = Vec::new();
    for (fixture, bytes) in documents() {
        let mut fresh = open_bytes(&bytes);
        let paragraphs = text_paragraphs(&mut fresh);
        let page_count = fresh.page_count();
        assert!(page_count >= 2, "{fixture}: needs more than one page");
        let last_page = page_count - 1;
        let fresh_hits = hits(&fresh);
        let fresh_registries = registries(&fresh);
        let fresh_pages = pages(&fresh);
        let fresh_labels = fresh.page_labels();
        // One paragraph per page, the last pages first: the ones past a bound.
        let mut far: Vec<usize> = Vec::new();
        for page in (1..page_count).rev() {
            if let Some(p) = paragraphs.iter().find(|p| p.1 == page) {
                far.push(p.0);
            }
        }
        let fresh_carets = carets(&mut fresh, &far);
        for case in cases(&paragraphs, last_page) {
            for bound in [1, 2] {
                let label = format!("{fixture} / {} / bound {bound}", case.name);
                let mut document = open_bytes(&bytes);
                let mut reference = open_bytes(&bytes);
                for &(anchor, focus) in &case.selections {
                    document.select(anchor, focus, bound).expect("select");
                    reference.select(anchor, focus, usize::MAX).expect("select");
                }
                reference.repaint_all().expect("repaint");
                if document.page_count() != page_count {
                    failures.push(format!(
                        "{label}: {} pages after select, {page_count} at open",
                        document.page_count()
                    ));
                }
                // Hit-testing and the caret come before anything repaints.
                let names = [
                    "links",
                    "comments",
                    "images",
                    "text boxes",
                    "revisions",
                    "numbers",
                    "cells",
                ];
                for (name, (now, then)) in names
                    .iter()
                    .zip(registries(&document).iter().zip(&fresh_registries))
                {
                    if now != then {
                        failures.push(format!(
                            "{label}: {name} registry differs from a fresh open"
                        ));
                    }
                }
                if document.page_labels() != fresh_labels {
                    failures.push(format!(
                        "{label}: page labels {:?}, {fresh_labels:?} at open",
                        document.page_labels()
                    ));
                }
                if hits(&document) != fresh_hits {
                    failures.push(format!("{label}: hit tests differ from a fresh open"));
                }
                let selection = document.selection();
                if carets(&mut document, &far) != fresh_carets {
                    failures.push(format!("{label}: caret geometry differs from a fresh open"));
                }
                // Restore the selection the caret probes moved.
                if let Some((anchor, focus)) = selection {
                    document.select(anchor, focus, bound).expect("select");
                } else if let Some(&(anchor, focus)) = case.selections.last() {
                    document.select(anchor, focus, bound).expect("select");
                }
                // Pages past the bound may be left to draw later, but only
                // when they say so.
                let before = pages(&document);
                let reference_pages = pages(&reference);
                for page in 0..before.len().min(reference_pages.len()) {
                    if before[page] != reference_pages[page] && !document.is_page_stale(page) {
                        failures.push(format!("{label}: page {page} differs and is not stale"));
                    }
                }
                document
                    .paint_stale_pages(0..document.page_count())
                    .expect("paint stale pages");
                for line in differing(&pages(&document), &reference_pages) {
                    failures.push(format!("{label}: {line}"));
                }
                // And the selected state is what a repaint of it draws.
                let drawn = pages(&document);
                document.repaint_all().expect("repaint");
                for line in differing(&drawn, &pages(&document)) {
                    failures.push(format!("{label}: against its own repaint: {line}"));
                }
                // A caret is drawn by the shell, so with the selection
                // collapsed the pages are the ones the document opened with.
                if document.selection().is_none() {
                    for line in differing(&pages(&document), &fresh_pages) {
                        failures.push(format!("{label}: collapsed, against a fresh open: {line}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Deep in a long document the pass resumes at the page it draws rather
/// than walking from the first page, and still draws what a full repaint
/// draws there, and leaves the pages after it hit-testing as at open.
#[test]
fn a_select_deep_in_a_long_document_matches_a_full_repaint() {
    let path = "public/fixtures/yokohama.docx";
    let mut fresh = open_fixture(path);
    let paragraph = (fresh.paragraph_count() * 2 / 3..fresh.paragraph_count())
        .find(|&p| {
            fresh
                .paragraph_text(p)
                .is_some_and(|t| t.chars().count() >= 4)
        })
        .expect("a paragraph with text");
    let caret = fresh.place_caret(paragraph, 0).expect("caret");
    let page = fresh.page_and_offset(caret.document_y).0;
    let fresh_hits = hits(&fresh);
    let far: Vec<usize> = (paragraph + 1..fresh.paragraph_count())
        .step_by(40)
        .collect();
    let fresh_carets = carets(&mut fresh, &far);

    let mut document = open_fixture(path);
    let mut reference = open_fixture(path);
    for selection in [
        ((paragraph, 0), (paragraph, 3)),
        ((paragraph, 1), (paragraph, 2)),
    ] {
        document
            .select(selection.0, selection.1, page + 1)
            .expect("select");
        reference
            .select(selection.0, selection.1, usize::MAX)
            .expect("select");
    }
    reference.repaint_all().expect("repaint");
    assert_eq!(document.page_count(), fresh.page_count());
    assert!(
        hits(&document) == fresh_hits,
        "hit tests differ from a fresh open"
    );
    assert_eq!(
        document.page_pixels(page),
        reference.page_pixels(page),
        "page {page} differs from a full repaint"
    );
    let selection = document.selection().expect("selection");
    assert!(
        carets(&mut document, &far) == fresh_carets,
        "carets differ from a fresh open"
    );
    document
        .select(selection.0, selection.1, page + 1)
        .expect("select");
    document
        .paint_stale_pages(0..document.page_count())
        .expect("paint stale pages");
    let lines = differing(&pages(&document), &pages(&reference));
    assert!(lines.is_empty(), "{}", lines.join("\n"));
}

/// After an edit bounded to the first page grows the document, a select into
/// its last paragraph lays the document out again; every page it does not
/// draw then may have moved, and must say so.
#[test]
fn a_select_after_a_bounded_edit_leaves_no_page_unpainted() {
    let path = "public/fixtures/contract.docx";
    let filler = "A clause long enough to wrap over several lines of the page. ".repeat(60);
    let mut document = open_fixture(path);
    let mut reference = open_fixture(path);
    let paragraph = common::paragraph_on_first_page(&mut document);
    let before = document.page_count();
    document
        .insert_text(paragraph, 0, &filler, 1)
        .expect("insert");
    reference
        .insert_text(paragraph, 0, &filler, usize::MAX)
        .expect("insert");
    let last = document.paragraph_count() - 1;
    document.select((last, 0), (last, 0), 1).expect("select");
    reference
        .select((last, 0), (last, 0), usize::MAX)
        .expect("select");
    reference.repaint_all().expect("repaint");
    assert!(
        reference.page_count() > before,
        "the edit did not add pages"
    );
    assert_eq!(document.page_count(), reference.page_count());
    let ours = pages(&document);
    let theirs = pages(&reference);
    for page in 0..ours.len() {
        assert!(
            ours[page] == theirs[page] || document.is_page_stale(page),
            "page {page} differs from a full repaint and is not stale"
        );
    }
    document
        .paint_stale_pages(0..document.page_count())
        .expect("paint stale pages");
    let lines = differing(&pages(&document), &theirs);
    assert!(lines.is_empty(), "{}", lines.join("\n"));
}
