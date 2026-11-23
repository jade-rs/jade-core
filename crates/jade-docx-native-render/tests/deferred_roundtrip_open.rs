//! Opening a document draws it without building the round-trip XML tree that
//! saving patches; the tree is built before the first edit and before export,
//! and what is saved is byte for byte what an eager load saves.

mod common;

use common::repo_path;
use futures::executor::block_on;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_native_render::document::roundtrip_xml_builds;
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

fn open_yokohama_window(bytes: &[u8], faces: &[FontFace]) -> NativeDocument {
    NativeDocument::open_with_font_faces_and_paint_window(bytes, 816.0, 1056.0, faces, 0..3)
        .unwrap()
}

/// Median, min and max of ten samples.
fn spread(mut samples: Vec<Duration>) -> (Duration, Duration, Duration) {
    samples.sort();
    ((samples[4] + samples[5]) / 2, samples[0], samples[9])
}

/// Time to first paint as the shell opens a document: the first three pages
/// drawn. `cargo test --release --test deferred_roundtrip_open -- --ignored
/// --nocapture`.
#[test]
#[ignore = "timing; run in release"]
fn time_to_first_paint_on_yokohama() {
    let bytes = std::fs::read(repo_path("public/fixtures/yokohama.docx")).unwrap();
    let faces = faces();
    let samples = (0..10)
        .map(|_| {
            let started = Instant::now();
            let document = open_yokohama_window(&bytes, &faces);
            let elapsed = started.elapsed();
            drop(document);
            elapsed
        })
        .collect();
    println!(
        "yokohama.docx first paint (median, min, max): {:?}",
        spread(samples)
    );
}

/// What idle time now pays after that paint: the tree, and the whole pass
/// that gives the layouts its node ids.
#[test]
#[ignore = "timing; run in release"]
fn time_to_build_the_roundtrip_tree_on_yokohama() {
    let bytes = std::fs::read(repo_path("public/fixtures/yokohama.docx")).unwrap();
    let faces = faces();
    let samples = (0..10)
        .map(|_| {
            let mut document = open_yokohama_window(&bytes, &faces);
            let started = Instant::now();
            document.ensure_roundtrip_xml().unwrap();
            started.elapsed()
        })
        .collect();
    println!(
        "yokohama.docx round-trip tree after it (median, min, max): {:?}",
        spread(samples)
    );
}

fn open(bytes: &[u8]) -> NativeDocument {
    NativeDocument::open_with_font_faces(bytes, 816.0, 1056.0, &faces()).unwrap()
}

/// The contract, and two with the parts a save has to carry through: a
/// header and footer with a table, and footnotes on every page with tables.
const FIXTURES: [&str; 3] = [
    "contract.docx",
    "word-compat-comprehensive.docx",
    "footnote-table-each-page.docx",
];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_path(&format!("public/fixtures/{name}"))).unwrap()
}

/// What saving produced before the deferred open: an eager load, the same
/// headless commands through a fresh executor, `undos` of them taken back,
/// then export.
fn eager_export(bytes: &[u8], commands: Vec<HeadlessCommand>, undos: usize) -> Vec<u8> {
    let mut document = block_on(WordDocument::load(bytes, &Options::default())).unwrap();
    let mut executor = CommandExecutor::default();
    let mut selection = if commands.is_empty() {
        None
    } else {
        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest { commands },
        )
        .unwrap()
    };
    for _ in 0..undos {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor.undo(&mut ctx).unwrap());
    }
    document.export_docx().unwrap()
}

/// Byte equality, without printing two archives when it fails.
fn assert_same(actual: Vec<u8>, expected: Vec<u8>, context: &str) {
    assert!(
        actual == expected,
        "{context}: {} bytes exported, {} expected",
        actual.len(),
        expected.len()
    );
}

/// A paragraph with text on the first page, so an edit lands where it shows.
fn text_paragraph(document: &mut NativeDocument) -> usize {
    common::paragraph_on_first_page(document)
}

#[test]
fn opening_does_not_build_the_roundtrip_tree() {
    let before = roundtrip_xml_builds();
    let mut document = open(&fixture("contract.docx"));
    assert_eq!(roundtrip_xml_builds(), before, "open built the tree");
    assert!(document.word_document().raw_document.is_none());

    // Reading does not build it either.
    let paragraph = text_paragraph(&mut document);
    let _ = document.paragraph_text(paragraph);
    let _ = document.find("the", true);
    assert_eq!(roundtrip_xml_builds(), before, "a query built the tree");

    // The first edit does, once.
    document.insert_text(paragraph, 0, "x", 1).unwrap();
    assert_eq!(roundtrip_xml_builds(), before + 1);
    assert!(document.word_document().raw_document.is_some());
    document.insert_text(paragraph, 0, "y", 1).unwrap();
    document.undo().unwrap();
    let _ = document.export_docx().unwrap();
    assert_eq!(roundtrip_xml_builds(), before + 1, "built more than once");
}

#[test]
fn export_without_an_edit_matches_an_eager_load() {
    for name in FIXTURES {
        let bytes = fixture(name);
        let document = open(&bytes);
        let before = roundtrip_xml_builds();
        assert_same(
            document.export_docx().unwrap(),
            eager_export(&bytes, Vec::new(), 0),
            name,
        );
        assert_eq!(
            roundtrip_xml_builds(),
            before + 1,
            "{name}: export built the tree"
        );
        // On a copy: the live document is still as opened.
        assert!(document.word_document().raw_document.is_none(), "{name}");
    }
}

#[test]
fn open_edit_export_matches_an_eager_load() {
    for name in FIXTURES {
        let bytes = fixture(name);
        let mut document = open(&bytes);
        let paragraph = text_paragraph(&mut document);
        document.insert_text(paragraph, 0, "Edited ", 1).unwrap();
        let typed = || {
            vec![HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "Edited ".to_string(),
            }]
        };
        assert_same(
            document.export_docx().unwrap(),
            eager_export(&bytes, typed(), 0),
            &format!("{name}: typed"),
        );
        document.undo().unwrap();
        assert_same(
            document.export_docx().unwrap(),
            eager_export(&bytes, typed(), 1),
            &format!("{name}: undone"),
        );
    }
}

#[test]
fn a_header_edit_first_exports_as_an_eager_load_does() {
    let bytes = fixture("word-compat-comprehensive.docx");
    let mut document = open(&bytes);
    document
        .set_header_footer_text(false, 0, "Native header")
        .unwrap();
    let expected = eager_export(
        &bytes,
        vec![HeadlessCommand::SetHeaderParagraphText {
            paragraph: 0,
            text: "Native header".to_string(),
        }],
        0,
    );
    assert_same(document.export_docx().unwrap(), expected, "header edit");
    // Undone from the snapshot taken before the edit, which has the tree.
    document.undo().unwrap();
    assert_same(
        document.export_docx().unwrap(),
        eager_export(&bytes, Vec::new(), 0),
        "header edit undone",
    );
}

/// The tree changes nothing that is drawn: pages painted without it are the
/// pages a repaint draws once it is built.
#[test]
fn the_first_paint_is_the_paint_with_the_tree() {
    for name in FIXTURES {
        let mut document = open(&fixture(name));
        let pages = document.page_count().min(3);
        let first: Vec<_> = (0..pages).map(|page| document.page_pixels(page)).collect();
        document.ensure_roundtrip_xml().unwrap();
        document.repaint_all().unwrap();
        assert_eq!(document.page_count().min(3), pages, "{name}");
        for (page, pixels) in first.into_iter().enumerate() {
            assert!(pixels == document.page_pixels(page), "{name}: page {page}");
        }
    }
}

/// A one-paragraph package around `body`.
fn docx(body: &str) -> Vec<u8> {
    use std::io::Write;
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
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
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    bytes
}

/// A legacy check box draws its box from a result only the tree gives it, so
/// a document with one opens with the tree and draws the box at once.
#[test]
fn a_legacy_check_box_is_drawn_at_open() {
    let check_box = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Check1"/><w:enabled/><w:calcOnExit w:val="0"/><w:checkBox><w:sizeAuto/><w:default w:val="1"/></w:checkBox></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t xml:space="preserve"> yes</w:t></w:r></w:p>"#;
    let mut document = open(&docx(check_box));
    assert!(document.word_document().raw_document.is_some());
    let first = document.page_pixels(0);
    document.repaint_all().unwrap();
    assert!(first == document.page_pixels(0));

    // Without a check box the tree waits for the first edit.
    let plain = open(&docx("<w:p><w:r><w:t>yes</w:t></w:r></w:p>"));
    assert!(plain.word_document().raw_document.is_none());
}

/// Idle time builds the tree: the shell's pass over the pages left stale.
#[test]
fn painting_stale_pages_builds_the_tree() {
    let bytes = fixture("footnote-table-each-page.docx");
    let mut document = NativeDocument::open_with_font_faces_and_paint_window(
        &bytes,
        816.0,
        1056.0,
        &faces(),
        0..1,
    )
    .unwrap();
    assert!(document.word_document().raw_document.is_none());
    let before = roundtrip_xml_builds();
    document.paint_stale_pages(0..usize::MAX).unwrap();
    assert_eq!(roundtrip_xml_builds(), before + 1);
    assert!(document.word_document().raw_document.is_some());
}
