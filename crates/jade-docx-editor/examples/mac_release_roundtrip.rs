//! Reproduce the Mac release gate's one-character edit across every DOCX fixture.
//!
//! Run from the repository root with an output directory outside the fixture tree:
//! `cargo run -p jade-docx-editor --example mac_release_roundtrip -- /tmp/jade-release-roundtrip`

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use jade_docx_editor::commands::insert_text::InsertTextCommand;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

#[cfg(not(target_os = "windows"))]
fn main() -> Result<(), Box<dyn Error>> {
    run()
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), Box<dyn Error>> {
    // The Windows main stack is smaller than the macOS audit runner's.
    // Large DOCX load/edit/export futures need a bounded worker stack here.
    std::thread::Builder::new()
        .name("mac-release-roundtrip".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| run().map_err(|error| error.to_string()))?
        .join()
        .map_err(|_| "roundtrip audit worker panicked")?
        .map_err(Into::into)
}

fn run() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(std::env::args().nth(1).ok_or("output directory required")?);
    fs::create_dir_all(&output)?;
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut fixtures = fs::read_dir(fixture_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    fixtures.sort();

    let mut checked = 0;
    let mut edited = 0;
    for fixture in fixtures {
        if fixture.extension().and_then(|extension| extension.to_str()) != Some("docx") {
            continue;
        }
        let bytes = fs::read(&fixture)?;
        let mut document =
            futures::executor::block_on(WordDocument::load(&bytes, &Default::default()))?;
        if document.paragraph_count() > 0 {
            let range = SelectionRange::collapsed(TextPosition::new(0, 0));
            let mut selection = Some(range);
            CommandExecutor::new().execute(
                Box::new(InsertTextCommand::new(range, "Z".into(), None)),
                &mut CommandContext::new(&mut document, &mut selection),
            )?;
            edited += 1;
        }
        let exported = document.export_docx()?;
        futures::executor::block_on(WordDocument::load(&exported, &Default::default()))?;
        fs::write(
            output.join(fixture.file_name().ok_or("fixture has no name")?),
            exported,
        )?;
        checked += 1;
    }
    println!("exported and reopened {checked} fixtures; edited {edited}");
    Ok(())
}
