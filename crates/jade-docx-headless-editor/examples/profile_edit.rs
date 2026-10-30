//! Profile edits on an already loaded document, excluding load and export.
//!
//! Build with `cargo build -p jade-docx-headless-editor --example profile_edit
//! --profile profiling`, then run `samply record --save-only --
//! target/profiling/examples/profile_edit <fixture> [pairs]`.
use jade_docx_document_core::{
    commands::{CommandContext, CommandExecutor},
    headless_api::apply_headless_commands_with_executor_to_document,
    word_document::WordDocument,
    Options,
};
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use std::time::Instant;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let fixture = args.get(1).expect("usage: profile_edit <fixture> [pairs]");
    let bytes = std::fs::read(fixture).expect("read fixture");
    let pairs: usize = args.get(2).map_or(1000, |s| {
        s.parse().expect("pairs must be a nonnegative integer")
    });
    let mut doc =
        futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
    let paragraph = (6..doc.paragraph_count())
        .chain(0..doc.paragraph_count().min(6))
        .find(|&i| doc.paragraph(i).is_some_and(|p| !p.runs.is_empty()))
        .expect("document must contain an editable paragraph");
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::InsertText {
            paragraph,
            offset: 0,
            text: "あ".into(),
        }],
    };
    let mut executor = CommandExecutor::new();
    let mut selection = None;
    let started = Instant::now();
    for _ in 0..pairs {
        selection = apply_headless_commands_with_executor_to_document(
            &mut doc,
            &mut executor,
            selection,
            &request,
        )
        .unwrap();
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(executor.undo(&mut ctx).unwrap());
    }
    println!(
        "{{\"pairs\":{pairs},\"paragraph\":{paragraph},\"elapsed_ns\":{}}}",
        started.elapsed().as_nanos()
    );
    std::hint::black_box(doc);
}
