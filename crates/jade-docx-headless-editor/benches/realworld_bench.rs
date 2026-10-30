//! DOCX load/edit/export and bookmark display-text benchmarks. Loads an actual
//! 53 KB contract .docx to measure parse/load and a full load → insert → export
//! cycle, plus synthetic bookmark ranges spanning different paragraph counts.
//!
//! This captures load/style-resolution cost, text edits with undo, and bookmark
//! extraction. Per-keystroke relayout is measured separately by jade-docx-perf.
//!
//! Run: `cargo bench -p jade-docx-headless-editor`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use jade_docx_document_core::commands::{CommandContext, CommandExecutor};
use jade_docx_document_core::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_document_core::parser::DocumentParser;
use jade_docx_document_core::word_document::WordDocument;
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_headless_editor::apply_headless_commands;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

static CONTRACT: &[u8] = include_bytes!("../../../contract.docx");

// Keep loading and exporting outside the measurement. Undo restores the same
// paragraph each iteration, so the document and undo history do not grow.
fn bench_text_edits(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit_insert_undo");
    for (name, data) in [
        ("contract", CONTRACT),
        (
            "yokohama",
            include_bytes!("../../../public/fixtures/yokohama.docx").as_slice(),
        ),
    ] {
        let mut doc =
            futures::executor::block_on(WordDocument::load(data, &Options::default())).unwrap();
        let paragraph = (6..doc.paragraph_count())
            .chain(0..doc.paragraph_count().min(6))
            .find(|&index| doc.paragraph(index).is_some_and(|p| !p.runs.is_empty()))
            .expect("editable paragraph");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "あ".into(),
            }],
        };
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        group.bench_function(name, |b| {
            b.iter(|| {
                selection = apply_headless_commands_with_executor_to_document(
                    &mut doc,
                    &mut executor,
                    selection,
                    black_box(&request),
                )
                .expect("insert text");
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                assert!(executor.undo(&mut ctx).expect("undo insertion"));
                black_box(&doc);
            });
        });
    }
    group.finish();
}

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
    bench_text_edits,
    bench_bookmark_text,
    bench_text_document
);
criterion_main!(benches);
