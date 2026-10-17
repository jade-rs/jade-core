//! Does an edit reach the *render* model, or only the export?
//!
//! A repaint that redraws the same text is still a repaint: it changes timing,
//! not pixels. The app's diagnostics showed 42 successful keystrokes leaving
//! the page's ink count identical to the digit, which the existing pixel-change
//! test did not catch. This checks the thing that test assumed.

use futures::executor::block_on;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use std::path::PathBuf;

const INSERTED: &str = "ネイティブ編集マーカー";

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn an_edit_is_visible_in_the_document_the_renderer_is_given() {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut options = Options::default();
    options.show_formatting_marks = false;
    let mut word_document = block_on(WordDocument::load(&bytes, &options)).expect("parse docx");

    apply_headless_commands_to_document(
        &mut word_document,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 30,
                offset: 0,
                text: INSERTED.to_string(),
            }],
        },
    )
    .expect("apply the edit");

    // `word_document.document` is exactly what is handed to `render`. If the
    // edit is not in here, a repaint redraws the old text however correct the
    // export looks.
    let rendered_model = word_document
        .document
        .as_ref()
        .expect("main document part")
        .clone();
    let text = format!("{rendered_model:?}");

    assert!(
        text.contains(INSERTED),
        "the edit is not in the document the renderer is given — an export may \
         still contain it, but repainting will draw the old text"
    );
}
