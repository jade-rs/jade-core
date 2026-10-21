//! Several commands as one user action — a paste of formatted runs, a replace
//! of matches the engine's own replace cannot find — and a command a shell
//! builds itself, such as a picture inserted: each is one undo step.

mod common;
use common::{open_contract, paragraph_on_first_page};
use jade_docx_editor::commands::insert_image::InsertImageCommand;
use jade_docx_editor::commands::insert_text::InsertTextCommand;
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::{SelectionRange, TextPosition};
use std::rc::Rc;

#[test]
fn a_batch_of_commands_undoes_as_one() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let before = document.paragraph_text(paragraph).unwrap();
    let allocation = Rc::as_ptr(document.word_document().document.as_ref().unwrap());

    document
        .apply_commands(
            vec![
                HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: "one ".to_string(),
                },
                HeadlessCommand::InsertText {
                    paragraph,
                    offset: 4,
                    text: "two ".to_string(),
                },
                HeadlessCommand::ApplyBold {
                    start_paragraph: paragraph,
                    start_offset: 0,
                    end_paragraph: paragraph,
                    end_offset: 3,
                    enabled: true,
                },
            ],
            false,
        )
        .unwrap();
    assert_eq!(
        Rc::as_ptr(document.word_document().document.as_ref().unwrap()),
        allocation,
        "a batch must edit without copying the whole document"
    );
    assert_eq!(
        document.paragraph_text(paragraph).unwrap(),
        format!("one two {before}")
    );
    assert_eq!(
        document.run_style_at(paragraph, 1).unwrap().bold,
        Some(true)
    );

    document.undo().unwrap();
    assert_eq!(document.paragraph_text(paragraph).unwrap(), before);
    assert!(!document.can_undo(), "one step took the whole batch back");
    document.redo().unwrap();
    assert_eq!(
        document.paragraph_text(paragraph).unwrap(),
        format!("one two {before}")
    );
}

#[test]
fn a_failed_batch_leaves_the_document_as_it_was() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let before = document.paragraph_text(paragraph).unwrap();
    let allocation = Rc::as_ptr(document.word_document().document.as_ref().unwrap());
    let result = document.apply_commands(
        vec![
            HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "kept? ".to_string(),
            },
            HeadlessCommand::InsertText {
                paragraph: usize::MAX,
                offset: 0,
                text: "nowhere".to_string(),
            },
        ],
        false,
    );
    assert!(result.is_err());
    assert_eq!(
        Rc::as_ptr(document.word_document().document.as_ref().unwrap()),
        allocation,
        "a failed batch and its rollback must stay in place"
    );
    assert_eq!(document.paragraph_text(paragraph).unwrap(), before);
    assert!(!document.can_undo());
}

/// A 1×1 PNG.
const PIXEL: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn a_command_the_shell_builds_is_one_undo_step() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let images = |document: &common::Doc| {
        document
            .word_document()
            .paragraph(paragraph)
            .map_or(0, |paragraph| {
                paragraph
                    .runs
                    .iter()
                    .filter(|run| run.image.is_some())
                    .count()
            })
    };
    let caret = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
    document
        .execute_command(
            Box::new(InsertImageCommand::new(
                caret,
                PIXEL.to_vec(),
                "image/png".to_string(),
                96.0,
                48.0,
                None,
                None,
            )),
            false,
        )
        .unwrap();
    assert_eq!(images(&document), 1);
    assert!(document.export_docx().is_ok());
    document.undo().unwrap();
    assert_eq!(images(&document), 0);
    assert!(!document.can_undo());
}

/// The `instrText`/`fldSimple` PAGE fields across the footers a saved
/// document carries.
fn footer_page_fields(docx: &[u8]) -> usize {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names
        .iter()
        .filter(|name| name.starts_with("word/footer") && name.ends_with(".xml"))
        .map(|name| {
            let mut xml = String::new();
            archive
                .by_name(name)
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            xml.matches("PAGE").count()
        })
        .sum()
}

#[test]
fn a_page_number_goes_into_the_footer_as_one_undo_step() {
    use jade_docx_editor::commands::snapshot_edit::SnapshotEditCommand;
    let mut document = open_contract();
    let before = footer_page_fields(&document.export_docx().unwrap());
    document
        .execute_command(
            Box::new(SnapshotEditCommand::new("insert_page_number", |document| {
                document.insert_page_number(true, "center")
            })),
            true,
        )
        .unwrap();
    let inserted = footer_page_fields(&document.export_docx().unwrap());
    assert!(inserted > 0, "no PAGE field in the footers");
    document.undo().unwrap();
    assert_eq!(footer_page_fields(&document.export_docx().unwrap()), before);
    assert!(document
        .word_document()
        .clone()
        .remove_page_numbers()
        .is_ok());
}

#[test]
fn a_shell_text_command_edits_in_place() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let before = document.paragraph_text(paragraph).unwrap();
    let allocation = Rc::as_ptr(document.word_document().document.as_ref().unwrap());
    let caret = SelectionRange::collapsed(TextPosition::new(paragraph, 0));
    document
        .execute_command(
            Box::new(InsertTextCommand::new(caret, "added ".to_string(), None)),
            false,
        )
        .unwrap();
    assert_eq!(
        Rc::as_ptr(document.word_document().document.as_ref().unwrap()),
        allocation,
        "a shell command must edit without copying the whole document"
    );
    assert_eq!(
        document.paragraph_text(paragraph).unwrap(),
        format!("added {before}")
    );
    document.undo().unwrap();
    assert_eq!(document.paragraph_text(paragraph).unwrap(), before);
    assert!(!document.can_undo());
    document.redo().unwrap();
    assert_eq!(
        document.paragraph_text(paragraph).unwrap(),
        format!("added {before}")
    );
}

/// Separate edits merged after the fact — formatting several ranges, a drag
/// and drop — undo and redo as one step.
#[test]
fn steps_merged_since_a_mark_undo_as_one() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let before = document.paragraph_text(paragraph).unwrap();
    let mark = document.history_len();
    for text in ["a", "b", "c"] {
        document
            .apply_commands(
                vec![HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: text.to_string(),
                }],
                false,
            )
            .unwrap();
    }
    assert_eq!(document.history_len(), mark + 3);
    document.merge_history_since(mark);
    assert_eq!(document.history_len(), mark + 1);
    document.undo().unwrap();
    assert_eq!(document.paragraph_text(paragraph).unwrap(), before);
    document.redo().unwrap();
    assert_eq!(
        document.paragraph_text(paragraph).unwrap(),
        format!("cba{before}")
    );
    // A mark past the end merges nothing.
    document.merge_history_since(99);
    assert_eq!(document.history_len(), mark + 1);
}
