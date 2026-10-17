//! Real-document benchmark for the run-style string change. Loads an actual
//! 53 KB contract .docx and measures (a) parse/load only — the path that does
//! the bulk of the per-run/per-paragraph style resolution (merge + property
//! clones) — and (b) a full load → insert_text → export cycle.
//!
//! Compiles identically on the `String`, `Arc<str>`, and `Rc<str>` checkouts,
//! so the same source measures all three.
//!
//! Note: this does NOT capture the editor's per-keystroke *relayout* latency
//! (that path runs in the canvas/wasm stack and is not benchmarkable natively).
//! It captures the load/style-resolution cost on real content, where the
//! string change actually fires.
//!
//! Run: `cargo bench -p jade-docx-headless-editor`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use jade_docx_document_core::word_document::WordDocument;
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_headless_editor::apply_headless_commands;

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

criterion_group!(benches, bench_realworld);
criterion_main!(benches);
