//! Export a tracked paragraph replacement and Jade's review outcomes for a Word for Mac check.
//!
//! Run from the repository root with an output directory outside the fixture tree:
//! `cargo run -p jade-docx-editor --example mac_review_reopen_probe -- /tmp/jade-word-review`

use std::{error::Error, fs, path::PathBuf};

use jade_docx_editor::commands::insert_text::InsertTextCommand;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::word_document::{RevisionSession, WordDocument};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(std::env::args().nth(1).ok_or("output directory required")?);
    fs::create_dir_all(&output)?;
    let fixture = include_bytes!("../../../public/fixtures/hello.docx");
    let mut document =
        futures::executor::block_on(WordDocument::load(fixture, &Default::default()))?;
    document.set_revision_session(Some(RevisionSession {
        author: Some("Jade release probe".into()),
        date: Some("2026-09-25T00:00:00Z".into()),
        date_utc: None,
    }));
    let range = SelectionRange::new(TextPosition::new(0, 2), TextPosition::new(1, 3));
    let mut selection = Some(range);
    CommandExecutor::new().execute(
        Box::new(InsertTextCommand::new(range, "X".into(), None)),
        &mut CommandContext::new(&mut document, &mut selection),
    )?;
    let exported = document.export_docx()?;
    fs::write(output.join("tracked-replace.docx"), &exported)?;
    let mut accepted =
        futures::executor::block_on(WordDocument::load(&exported, &Default::default()))?;
    accepted.accept_revisions(None, None, None, None)?;
    fs::write(output.join("jade-accepted.docx"), accepted.export_docx()?)?;
    let mut rejected =
        futures::executor::block_on(WordDocument::load(&exported, &Default::default()))?;
    rejected.reject_revisions(None, None, None, None)?;
    fs::write(output.join("jade-rejected.docx"), rejected.export_docx()?)?;
    Ok(())
}
