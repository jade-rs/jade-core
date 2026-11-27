//! Section `w:vAlign` must not make layout depend on the previous layout:
//! one pass after an edit equals a fresh open of the edited document.

mod common;

use common::{open_bytes, repo_path, Doc};
use std::io::{Cursor, Read, Write};

/// `fixture` with `w:vAlign` added to every section.
fn with_valign(fixture: &str, alignment: &str) -> Vec<u8> {
    let bytes = std::fs::read(repo_path(fixture)).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().unwrap().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            let xml = String::from_utf8(data).unwrap();
            let tag = format!(r#"<w:vAlign w:val="{alignment}"/>"#);
            let patched = xml.replace("<w:docGrid", &format!("{tag}<w:docGrid"));
            assert_ne!(patched, xml, "sectPr has a docGrid to anchor on");
            data = patched.into_bytes();
        }
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Page of every paragraph's first caret position (None when it has none).
fn paragraph_pages(document: &mut Doc) -> Vec<Option<usize>> {
    (0..document.paragraph_count())
        .map(|paragraph| {
            document
                .position_rect(paragraph, 0)
                .map(|rect| document.page_and_offset(rect.document_y).0)
        })
        .collect()
}

/// `actual` matches a fresh open of its own export: page count, the page
/// each paragraph starts on, and every page's pixels.
fn reopen_mismatch(actual: &mut Doc, context: &str) -> Option<String> {
    let mut fresh = open_bytes(&actual.export_docx().unwrap());
    if actual.page_count() != fresh.page_count() {
        return Some(format!(
            "{context}: {} pages, a fresh open has {}",
            actual.page_count(),
            fresh.page_count()
        ));
    }
    let (pages, fresh_pages) = (paragraph_pages(actual), paragraph_pages(&mut fresh));
    if let Some(paragraph) = (0..pages.len()).find(|&index| pages[index] != fresh_pages[index]) {
        return Some(format!(
            "{context}: paragraph {paragraph} starts on page {:?}, fresh {:?}",
            pages[paragraph], fresh_pages[paragraph]
        ));
    }
    let differing: Vec<usize> = (0..fresh.page_count())
        .filter(|&page| actual.page_pixels(page) != fresh.page_pixels(page))
        .collect();
    (!differing.is_empty()).then(|| format!("{context}: pages {differing:?} differ in pixels"))
}

/// Text that pushes later paragraphs down.
fn growth(repeat: usize) -> String {
    "本契約の条項を補足する追加の文言です。".repeat(repeat)
}

#[test]
fn valign_edits_match_a_fresh_open_after_one_pass() {
    let mut failures = Vec::new();
    let mut check = |document: &mut Doc, context: String| {
        failures.extend(reopen_mismatch(document, &context));
    };
    // A few lines and a few pages of growth.
    for (repeat, alignment) in [3, 60]
        .into_iter()
        .flat_map(|repeat| ["center", "bottom", "both"].map(|alignment| (repeat, alignment)))
    {
        let bytes = with_valign("public/fixtures/contract.docx", alignment);
        let growth = growth(repeat);
        let alignment = format!("{alignment} x{repeat}");
        let paragraph = {
            let mut probe = open_bytes(&bytes);
            common::paragraph_on_first_page(&mut probe)
        };

        // A bounded growth edit, then one full repaint.
        let mut bounded = open_bytes(&bytes);
        bounded.insert_text(paragraph, 0, &growth, 1).unwrap();
        bounded.repaint_all().unwrap();
        check(&mut bounded, format!("{alignment}: bounded edit + repaint"));

        // An unbounded edit, then one full repaint.
        let mut unbounded = open_bytes(&bytes);
        unbounded.insert_text(paragraph, 0, &growth, 1000).unwrap();
        check(&mut unbounded, format!("{alignment}: unbounded edit"));
        unbounded.repaint_all().unwrap();
        check(
            &mut unbounded,
            format!("{alignment}: unbounded edit + repaint"),
        );

        // Undo goes back to the original layout in one pass.
        unbounded.undo().unwrap();
        check(&mut unbounded, format!("{alignment}: undo"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
