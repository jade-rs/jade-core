//! A structural edit re-wraps only what it touched.
//!
//! Every structural edit and undo resyncs the renderer with the document
//! (`CanvasRenderer::sync_word_document`). That used to install a fresh copy
//! of the style cascade and drop every measured and wrapped paragraph, so one
//! Enter in a long document measured and wrapped the whole document again.
//! The styles, theme, numbering and settings are the same after a split, so
//! the paragraphs it did not touch must come from the wrap cache, and the
//! pages must still be the ones a fresh open of the edited document draws.

mod common;
use common::{open_bytes, repo_path};
use std::io::{Cursor, Read, Write};

/// `contract.docx` with its body repeated `copies` times, every paragraph
/// ending in a number of its own so that no two wrap alike.
fn repeated_contract(copies: usize) -> Vec<u8> {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().unwrap().into_owned();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            let xml = String::from_utf8(data).unwrap();
            let open = xml.find("<w:body>").expect("a body") + "<w:body>".len();
            // The body's own section properties close it; repeat what comes
            // before them.
            let section = xml.rfind("<w:sectPr").expect("body section properties");
            let content = &xml[open..section];
            let mut body = String::new();
            let mut numbered = 0;
            for _ in 0..copies {
                for (index, piece) in content.split("</w:p>").enumerate() {
                    if index > 0 {
                        numbered += 1;
                        body.push_str(&format!(
                            "<w:r><w:t xml:space=\"preserve\"> {numbered}</w:t></w:r></w:p>"
                        ));
                    }
                    body.push_str(piece);
                }
            }
            data = format!("{}{}{}", &xml[..open], body, &xml[section..]).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn a_paragraph_split_in_a_long_document_rewraps_only_what_it_touched() {
    let mut document = open_bytes(&repeated_contract(44));
    let pages = document.page_count();
    assert!(
        pages >= 100,
        "the document is {pages} pages, not a long one"
    );

    let paragraph = common::paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).unwrap();
    let split_at: usize = text.chars().take(3).map(char::len_utf8).sum();

    // Three Enters, each timed: the first after open, the others after an
    // earlier split, as typing does.
    for _ in 0..3 {
        let before = document.renderer().paragraph_wrap_cache_stats();
        let elapsed = document
            .insert_paragraph_break(paragraph, split_at)
            .expect("split the paragraph");
        let after = document.renderer().paragraph_wrap_cache_stats();
        let hits = after.hits - before.hits;
        let misses = after.misses - before.misses;
        let ratio = hits as f64 / (hits + misses).max(1) as f64;
        eprintln!(
            "split of a {pages}-page document: {elapsed:?}, wrap cache {hits} hits / {misses} misses"
        );
        assert!(
            ratio >= 0.95,
            "the split re-wrapped the document: {hits} hits, {misses} misses"
        );
    }

    let reopened = open_bytes(&document.export_docx().unwrap());
    assert_eq!(document.page_count(), reopened.page_count());
    for page in 0..document.page_count() {
        assert!(
            document.page_pixels(page) == reopened.page_pixels(page),
            "page {page} differs from a fresh open of the edited document"
        );
    }
}
