//! DOCX load/edit/export and bookmark display-text benchmarks. Loads an actual
//! 53 KB contract .docx to measure parse/load and a full load → insert → export
//! cycle, plus synthetic bookmark ranges spanning different paragraph counts.
//!
//! Note: this does NOT capture the editor's per-keystroke *relayout* latency
//! (that path runs in the canvas/wasm stack and is not benchmarkable natively).
//! It captures load/style-resolution cost and bookmark extraction natively.
//!
//! Run: `cargo bench -p jade-docx-headless-editor`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use jade_docx_document_core::parser::DocumentParser;
use jade_docx_document_core::word_document::WordDocument;
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_headless_editor::apply_headless_commands;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

static CONTRACT: &[u8] = include_bytes!("../../../contract.docx");

fn bench_realworld(c: &mut Criterion) {
    // (a) Parse/load only — isolates style resolution (merge + property clones).
    c.bench_function("load_contract_parse", |b| {
        b.iter(|| {
            let doc = futures::executor::block_on(WordDocument::load(
                black_box(CONTRACT),
                &Options::default(),
            ))
            .expect("load contract.docx");
            black_box(doc);
        });
    });

    // (b) Full load -> insert_text -> export cycle on the real document.
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text: "契約".to_string(),
        }],
    };
    c.bench_function("load_insert_export_contract", |b| {
        b.iter(|| {
            let out = apply_headless_commands(black_box(CONTRACT), black_box(&request))
                .expect("apply insert_text");
            black_box(out);
        });
    });
}

// Bookmark extraction is used while resolving REF and other structural fields.
// Keep parsing outside the timed region to expose the cost of spanning many
// paragraphs and runs, rather than ZIP/XML parsing.
fn bench_bookmark_text(c: &mut Criterion) {
    let mut group = c.benchmark_group("bookmark_display_text");
    for paragraphs in [1, 100, 1000] {
        let mut xml = String::from(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        );
        for index in 0..paragraphs {
            xml.push_str("<w:p>");
            if index == 0 {
                xml.push_str(r#"<w:bookmarkStart w:id="0" w:name="span"/>"#);
            }
            for _ in 0..8 {
                xml.push_str("<w:r><w:t>契約の本文 mixed text.</w:t></w:r>");
            }
            if index == paragraphs - 1 {
                xml.push_str(r#"<w:bookmarkEnd w:id="0"/>"#);
            }
            xml.push_str("</w:p>");
        }
        xml.push_str("</w:body></w:document>");
        let document = DocumentParser::new()
            .parse_document_xml(&xml)
            .expect("parse");
        let bookmark = &document.bookmarks[0];
        group.bench_function(format!("paragraphs={paragraphs}"), |b| {
            b.iter(|| black_box(document.bookmark_display_text(black_box(bookmark))));
        });
    }
    group.finish();
}

// A stored ZIP keeps inflation out of the way when comparing text decoding
// and parsing. Each document contains one large text run.
fn bench_text_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("load_text_document");
    for (name, text) in [
        ("ascii", b"plain text ".as_slice()),
        ("unicode", "契約書🙂é".as_bytes()),
        ("invalid_utf8", b"plain \xff\xe3\x81 text ".as_slice()),
    ] {
        let mut document_xml = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>"#.to_vec();
        for _ in 0..(128 * 1024 / text.len()) {
            document_xml.extend_from_slice(text);
        }
        document_xml.extend_from_slice(b"</w:t></w:r></w:p></w:body></w:document>");
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("_rels/.rels", options).unwrap();
        zip.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#).unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(&document_xml).unwrap();
        let data = zip.finish().unwrap().into_inner();
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(
                    futures::executor::block_on(WordDocument::load(
                        black_box(&data),
                        &Options::default(),
                    ))
                    .expect("load text document"),
                );
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_realworld,
    bench_bookmark_text,
    bench_text_document
);
criterion_main!(benches);
