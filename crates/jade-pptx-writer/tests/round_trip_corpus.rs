//! Saving must not lose anything: in every deck fixture, typing into a
//! shape and saving keeps that text, every other shape's text and position,
//! the notes, and every part of the package, media byte for byte.
//!
//! This is the path the app takes (`EditSession::set_shape_text`, then the
//! session's bytes), run over the whole corpus.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;

use jade_pptx_writer::{editable_shapes, notes_text, shape_text, EditSession};

fn fixtures() -> Vec<(String, Vec<u8>)> {
    // The shared corpus, and the files another crate's tests keep.
    let mut fixtures: Vec<_> = [
        "../../public/fixtures",
        // Real decks from PowerPoint, LibreOffice and published talks.
        "../../public/fixtures/poi",
        "../jade-pptx-diff/tests/fixtures",
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
            .is_some_and(|extension| extension == "pptx")
    })
    .filter(|path| {
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

fn slide_count(bytes: &[u8]) -> usize {
    part_names(bytes)
        .iter()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .count()
}

/// Per slide and shape: its text (when it has one) and its transform.
type Snapshot = BTreeMap<(usize, u32), (Option<String>, String)>;

fn snapshot(bytes: &[u8]) -> (Snapshot, Vec<Option<String>>) {
    let mut shapes = BTreeMap::new();
    let mut notes = Vec::new();
    for slide in 0..slide_count(bytes) {
        for shape in editable_shapes(bytes, slide).unwrap_or_default() {
            let text = shape_text(bytes, slide, shape.id)
                .ok()
                .map(|snapshot| snapshot.text);
            shapes.insert((slide, shape.id), (text, format!("{:?}", shape.transform)));
        }
        notes.push(notes_text(bytes, slide).ok().map(|snapshot| snapshot.text));
    }
    (shapes, notes)
}

fn check(name: &str, original: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut session = match EditSession::open(original) {
        Ok(session) => session,
        Err(error) => return vec![format!("{name}: does not open: {error}")],
    };
    let (before, notes_before) = snapshot(original);
    // Type into the first shape with text; a deck without any moves its
    // first movable shape instead.
    let typed = before
        .iter()
        .find(|(_, (text, _))| text.as_ref().is_some_and(|text| !text.is_empty()))
        .map(|(&key, _)| key);
    let edited = match typed {
        Some((slide, id)) => {
            if let Err(error) = session.set_shape_text(slide, id, "typed", false) {
                return vec![format!("{name}: typing fails: {error}")];
            }
            (slide, id)
        }
        None => {
            let Some((slide, shape)) = (0..slide_count(original)).find_map(|slide| {
                editable_shapes(original, slide)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|shape| shape.can_move && shape.transform.is_some())
                    .map(|shape| (slide, shape))
            }) else {
                return vec![format!("{name}: nothing to edit")];
            };
            let mut transform = shape.transform.expect("filtered");
            transform.offset_emu.0 += 12_700;
            if let Err(error) = session.set_shape_transform(slide, shape.id, transform) {
                return vec![format!("{name}: moving fails: {error}")];
            }
            (slide, shape.id)
        }
    };
    let (slide, id) = edited;
    session.mark_saved();
    let saved = session.bytes().to_vec();

    let missing: Vec<_> = part_names(original)
        .difference(&part_names(&saved))
        .cloned()
        .collect();
    if !missing.is_empty() {
        problems.push(format!("{name}: saving drops {missing:?}"));
    }
    for media in part_names(original)
        .into_iter()
        .filter(|part_name| part_name.starts_with("ppt/media/"))
    {
        if part(original, &media) != part(&saved, &media) {
            problems.push(format!("{name}: saving changes {media}"));
        }
    }
    if EditSession::open(&saved).is_err() {
        problems.push(format!("{name}: saved copy does not open"));
        return problems;
    }
    let (after, notes_after) = snapshot(&saved);
    match (typed.is_some(), after.get(&(slide, id))) {
        (true, Some((Some(text), _))) if text == "typed" => {}
        (true, other) => {
            problems.push(format!("{name}: the typed text is lost on save: {other:?}"))
        }
        (false, moved) if moved.map(|(_, at)| at) != before.get(&(slide, id)).map(|(_, at)| at) => {
        }
        (false, _) => problems.push(format!("{name}: the move is lost on save")),
    }
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    for key in keys.into_iter().filter(|key| **key != (slide, id)) {
        if before.get(key) != after.get(key) {
            problems.push(format!(
                "{name}: slide {} shape {}: {:?} became {:?}",
                key.0 + 1,
                key.1,
                before.get(key),
                after.get(key)
            ));
        }
    }
    if notes_before != notes_after {
        problems.push(format!("{name}: saving changes the notes"));
    }
    problems
}

#[test]
fn every_deck_saves_an_edit_without_losing_anything() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 110, "the corpus is missing");
    let problems: Vec<String> = fixtures
        .iter()
        .flat_map(|(name, bytes)| check(name, bytes))
        .collect();
    assert!(
        problems.is_empty(),
        "{} problems across {} decks:\n{}",
        problems.len(),
        fixtures.len(),
        problems.join("\n")
    );
}
