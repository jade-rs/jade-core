//! Saving must not lose anything: every fixture, opened and saved, reopens
//! with the same text in every paragraph and every part of its package; and
//! an edit survives the save while everything else stays as it was.
//!
//! This is the path the app takes on ⌘S (`WordDocument::export_docx`), run
//! over the whole corpus — real Word documents for the most part — so a
//! construct the writer drops shows up as the fixture that has it.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::json;

fn fixtures() -> Vec<(String, Vec<u8>)> {
    // The shared corpus, and the files another crate's tests keep.
    let mut fixtures: Vec<_> = [
        "../../public/fixtures",
        "../jade-docx-extract/tests/fixtures",
    ]
    .iter()
    .flat_map(|directory| {
        std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(directory))
            .expect("fixtures")
    })
    .filter_map(Result::ok)
    .map(|entry| entry.path())
    .filter(|path| {
        path.extension()
            .is_some_and(|extension| extension == "docx")
    })
    .filter(|path| {
        // AppleDouble companions that macOS leaves on external volumes.
        !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("._"))
    })
    .map(|path| {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        (name, std::fs::read(&path).expect("read fixture"))
    })
    .collect();
    fixtures.sort();
    fixtures
}

fn load(bytes: &[u8]) -> Result<WordDocument, String> {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default()))
        .map_err(|error| format!("{error:?}"))
}

/// Every paragraph surface's text, in order: body, tables, text boxes,
/// headers and footers alike.
fn texts(document: &WordDocument) -> Vec<String> {
    (0..document.paragraph_count())
        .map(|index| {
            document
                .paragraph(index)
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default()
        })
        .collect()
}

fn part_names(bytes: &[u8]) -> BTreeSet<String> {
    let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    archive.file_names().map(str::to_string).collect()
}

fn part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name(name).ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    Some(data)
}

/// The first paragraph surface that is in the body and has text.
fn first_body_text_paragraph(document: &WordDocument) -> Option<usize> {
    (0..document.paragraph_count()).find(|&index| {
        document.is_body_paragraph_surface(index)
            && document
                .paragraph(index)
                .is_some_and(|paragraph| paragraph.runs.iter().any(|run| !run.text.is_empty()))
    })
}

fn first_difference(before: &[String], after: &[String]) -> String {
    if before.len() != after.len() {
        return format!("{} paragraphs became {}", before.len(), after.len());
    }
    match before.iter().zip(after).position(|(a, b)| a != b) {
        Some(index) => format!(
            "paragraph {index}: {:?} became {:?}",
            before[index], after[index]
        ),
        None => "identical".into(),
    }
}

/// Checks one fixture; every problem found, described.
fn check(name: &str, original: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    let document = match load(original) {
        Ok(document) => document,
        Err(error) => return vec![format!("{name}: does not open: {error}")],
    };
    let before = texts(&document);

    // ---- saved as it is ---------------------------------------------------
    let saved = match document.export_docx() {
        Ok(saved) => saved,
        Err(error) => return vec![format!("{name}: does not save: {error:?}")],
    };
    let missing: Vec<_> = part_names(original)
        .difference(&part_names(&saved))
        .cloned()
        .collect();
    if !missing.is_empty() {
        problems.push(format!("{name}: saving drops {missing:?}"));
    }
    // Parts the editor never touches come back byte for byte.
    for untouched in part_names(original).into_iter().filter(|part_name| {
        part_name.starts_with("word/media/") || part_name.starts_with("word/embeddings/")
    }) {
        if part(original, &untouched) != part(&saved, &untouched) {
            problems.push(format!("{name}: saving changes {untouched}"));
        }
    }
    match load(&saved) {
        Ok(reopened) => {
            let after = texts(&reopened);
            if after != before {
                problems.push(format!(
                    "{name}: saving changes text: {}",
                    first_difference(&before, &after)
                ));
            }
        }
        Err(error) => problems.push(format!("{name}: saved copy does not open: {error}")),
    }

    // ---- edited, then saved -----------------------------------------------
    let mut edited = load(original).expect("opened once already");
    if let Some(paragraph) = first_body_text_paragraph(&edited) {
        let request: HeadlessEditRequest = serde_json::from_value(json!({
            "commands": [{ "type": "insert_text", "paragraph": paragraph, "offset": 0, "text": "★" }]
        }))
        .expect("request");
        if let Err(error) = apply_headless_commands_to_document(&mut edited, &request) {
            problems.push(format!("{name}: typing fails: {error:?}"));
            return problems;
        }
        let mut expected = texts(&edited);
        let saved = match edited.export_docx() {
            Ok(saved) => saved,
            Err(error) => {
                problems.push(format!("{name}: edited copy does not save: {error:?}"));
                return problems;
            }
        };
        match load(&saved) {
            Ok(reopened) => {
                let after = texts(&reopened);
                if !after
                    .get(paragraph)
                    .is_some_and(|text| text.starts_with('★'))
                {
                    problems.push(format!("{name}: the typed text is lost on save"));
                }
                // Typing changes one paragraph; the save must change none.
                expected.truncate(after.len().max(expected.len()));
                if after != expected {
                    problems.push(format!(
                        "{name}: saving an edit changes other text: {}",
                        first_difference(&expected, &after)
                    ));
                }
            }
            Err(error) => problems.push(format!("{name}: edited copy does not open: {error}")),
        }
    }
    problems
}

#[test]
fn every_fixture_saves_without_losing_anything() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 160, "the corpus is missing");
    let problems: Vec<String> = fixtures
        .iter()
        .flat_map(|(name, bytes)| check(name, bytes))
        .collect();
    assert!(
        problems.is_empty(),
        "{} problems across {} documents:\n{}",
        problems.len(),
        fixtures.len(),
        problems.join("\n")
    );
}
