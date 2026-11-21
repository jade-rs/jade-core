#![no_main]

use futures::executor::block_on;
use jade_docx_document_core::{word_document::WordDocument, Options};
use jade_docx_headless_editor::{apply_headless_commands, HeadlessCommand, HeadlessEditRequest};
use libfuzzer_sys::fuzz_target;
#[path = "../../crates/jade-docx-headless-editor/tests/support/package_integrity.rs"]
mod package_integrity;

const FIXTURES: &[&[u8]] = &[
    include_bytes!("../../public/fixtures/hello.docx"),
    include_bytes!("../../public/fixtures/table2x2.docx"),
    include_bytes!("../../public/fixtures/comment.docx"),
    include_bytes!("../../public/fixtures/hyperlink.docx"),
    include_bytes!("../../public/fixtures/hyperlink-field-edge-cases.docx"),
];

fn check_package(bytes: &[u8]) {
    package_integrity::assert_valid_package(bytes);
    block_on(WordDocument::load(bytes, &Options::default())).expect("export reload");
}

fuzz_target!(|data: &[u8]| {
    let Some((&fixture, operations)) = data.split_first() else {
        return;
    };
    let mut bytes = FIXTURES[fixture as usize % FIXTURES.len()].to_vec();
    for op in operations.chunks(12).take(32) {
        if op.len() < 4 {
            break;
        }
        let doc = block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
        if doc.paragraph_count() == 0 {
            break;
        }
        let p = op[1] as usize % doc.paragraph_count();
        let text: String = doc
            .paragraph(p)
            .unwrap()
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let offset = boundaries[op[2] as usize % boundaries.len()];
        let end = boundaries[op[3] as usize % boundaries.len()];
        let inserted = String::from_utf8_lossy(&op[4..]).into_owned();
        let command = match op[0] % 8 {
            0 => HeadlessCommand::InsertText {
                paragraph: p,
                offset,
                text: inserted,
            },
            1 => HeadlessCommand::InsertParagraphBreak {
                paragraph: p,
                offset,
                count: 1,
            },
            2 => HeadlessCommand::ReplaceRange {
                start_paragraph: p,
                start_offset: offset.min(end),
                end_paragraph: p,
                end_offset: offset.max(end),
                text: inserted,
            },
            3 => HeadlessCommand::AddComment {
                start_paragraph: p,
                start_offset: Some(offset.min(end)),
                end_paragraph: p,
                end_offset: Some(offset.max(end)),
                text: inserted,
                author: None,
                initials: None,
                date: None,
            },
            4 => HeadlessCommand::ApplyBold {
                start_paragraph: p,
                start_offset: offset.min(end),
                end_paragraph: p,
                end_offset: offset.max(end),
                enabled: op[3] % 2 == 0,
            },
            5 => HeadlessCommand::InsertFootnote {
                paragraph: p,
                offset,
                text: inserted,
            },
            6 => HeadlessCommand::InsertEndnote {
                paragraph: p,
                offset,
                text: inserted,
            },
            _ => HeadlessCommand::SetParagraphAlignment {
                start_paragraph: p,
                end_paragraph: p,
                alignment: ["left", "center", "right", "both"][op[3] as usize % 4].into(),
            },
        };
        // Unsupported edits and illegal XML characters must fail explicitly.
        // Every successful export must be independently parseable and reopen.
        if let Ok(output) = apply_headless_commands(
            &bytes,
            &HeadlessEditRequest {
                commands: vec![command],
            },
        ) {
            check_package(&output);
            bytes = output;
        }
    }
});
