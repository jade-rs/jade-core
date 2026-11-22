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
use common::{open_bytes, repo_path, Doc};
use jade_docx_headless_api::HeadlessCommand;
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

/// The display a test runs under. With formatting marks off and markup
/// hidden, the paragraph paint cache and prefix skip replay paragraphs that
/// the default display always draws afresh.
#[derive(Clone, Copy, Debug)]
struct Display {
    marks: bool,
    markup: bool,
}

const DEFAULT: Display = Display {
    marks: true,
    markup: true,
};
const MARKS_OFF: Display = Display {
    marks: false,
    markup: true,
};
const PLAIN: Display = Display {
    marks: false,
    markup: false,
};

fn show(document: &mut Doc, display: Display) {
    if !display.marks {
        document.set_formatting_marks(false).unwrap();
    }
    if !display.markup {
        document.set_markup_visible(false).unwrap();
    }
}

fn open_with(bytes: &[u8], display: Display) -> Doc {
    let mut document = open_bytes(bytes);
    show(&mut document, display);
    document
}

/// Every page `document` has drawn is the page a fresh open of what it
/// exports draws under the same display and selection, with no cache kept
/// from the session. Pages a paint window left stale are not drawn yet.
fn assert_matches_fresh_open(document: &Doc, display: Display, context: &str) {
    let mut reopened = open_with(&document.export_docx().unwrap(), display);
    reopened.set_selection_range(document.selection_range());
    reopened.repaint_all().unwrap();
    assert_eq!(
        document.page_count(),
        reopened.page_count(),
        "{context}: page count"
    );
    let mut compared = 0;
    for page in 0..document.page_count() {
        if document.is_page_stale(page) {
            continue;
        }
        compared += 1;
        assert!(
            document.page_pixels(page) == reopened.page_pixels(page),
            "{context}: page {page} differs from a fresh open of the edited document"
        );
    }
    assert!(compared > 0, "{context}: no page was drawn");
}

#[test]
fn a_paragraph_split_in_a_long_document_rewraps_only_what_it_touched() {
    let bytes = repeated_contract(44);
    for display in [DEFAULT, PLAIN] {
        let mut document = open_with(&bytes, display);
        let pages = document.page_count();
        assert!(
            pages >= 100,
            "the document is {pages} pages, not a long one"
        );

        let paragraph = common::paragraph_on_first_page(&mut document);
        let text = document.paragraph_text(paragraph).unwrap();
        let split_at: usize = text.chars().take(3).map(char::len_utf8).sum();

        // Three Enters, each timed: the first after open, the others after
        // an earlier split, as typing does.
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
                "{display:?}: split of a {pages}-page document: {elapsed:?}, wrap cache {hits} hits / {misses} misses"
            );
            assert!(
                ratio >= 0.95,
                "{display:?}: the split re-wrapped the document: {hits} hits, {misses} misses"
            );
        }
        assert_matches_fresh_open(&document, display, &format!("{display:?}"));
    }
}

/// Edits that change only formatting or geometry keep every paragraph's text,
/// so the kept wrap cache must not stand in for layout or paint: the
/// paragraph layout and paint caches replay on hashes that miss them.
#[test]
fn formatting_and_geometry_edits_and_their_undo_match_a_fresh_open() {
    let bytes = repeated_contract(3);
    let edits: [(&str, fn(usize) -> Vec<HeadlessCommand>); 5] = [
        ("font size", |paragraph| {
            vec![HeadlessCommand::SetFontSize {
                start_paragraph: paragraph,
                start_offset: 0,
                end_paragraph: paragraph + 4,
                end_offset: 0,
                font_size: 20.0,
            }]
        }),
        ("bold", |paragraph| {
            vec![HeadlessCommand::SetBold {
                start_paragraph: paragraph,
                start_offset: 0,
                end_paragraph: paragraph + 3,
                end_offset: 0,
                value: true,
            }]
        }),
        ("margins", |_| {
            vec![HeadlessCommand::SetPageMargins {
                top: None,
                bottom: None,
                left: Some(120.0),
                right: Some(120.0),
                header: None,
                footer: None,
                gutter: None,
            }]
        }),
        ("margins then text", |paragraph| {
            vec![
                HeadlessCommand::SetPageMargins {
                    top: Some(150.0),
                    bottom: None,
                    left: None,
                    right: None,
                    header: None,
                    footer: None,
                    gutter: None,
                },
                HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: "inserted ".into(),
                },
            ]
        }),
        // A table of contents whose page numbers move with a page break.
        ("toc then page break", |paragraph| {
            [
                serde_json::json!({"type": "set_paragraph_outline_level",
                    "start_paragraph": paragraph + 2, "end_paragraph": paragraph + 2, "level": 1}),
                serde_json::json!({"type": "insert_table_of_contents",
                    "after_paragraph": paragraph, "outline_min": 1, "outline_max": 3}),
                serde_json::json!({"type": "set_paragraph_pagination",
                    "start_paragraph": paragraph + 4, "end_paragraph": paragraph + 4,
                    "page_break_before": true}),
            ]
            .into_iter()
            .map(|command| serde_json::from_value(command).unwrap())
            .collect()
        }),
    ];
    for display in [MARKS_OFF, PLAIN] {
        for window in [false, true] {
            for (name, edit) in &edits {
                let context = format!("{display:?} window {window} {name}");
                let mut document = open_with(&bytes, display);
                let paragraph = common::paragraph_on_first_page(&mut document);
                if window {
                    let pages = document.page_count();
                    document.set_paint_window(Some(pages - 1..pages));
                }
                document
                    .apply_commands(edit(paragraph), true)
                    .unwrap_or_else(|error| panic!("{context}: {error}"));
                assert_matches_fresh_open(&document, display, &context);
                document.undo().unwrap();
                assert_matches_fresh_open(&document, display, &format!("{context}, undone"));
            }
        }
    }
}
