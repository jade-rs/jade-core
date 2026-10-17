//! Apply each editing scenario of `tests/word-schema/scenarios.json` to every
//! DOCX fixture and write the exported packages for schema validation.
//!
//! `cargo run -p jade-docx-editor --example word_edit_sweep -- <scenarios.json> <output-dir>`
//!
//! Each scenario is a list of `HeadlessCommand`s, the command set the browser
//! editor's ribbon and keyboard paths execute. Paragraph coordinates are
//! written as placeholders and resolved per fixture against the first body
//! paragraph with at least two characters of text:
//!
//! - `"$P"`: that paragraph; `"$P1"`: the body paragraph after it
//! - `"$LEN"`: its length in bytes; `"$MID"`: a character boundary near its middle
//! - `"$LEN1"`: the length of `$P1`
//! - `"$HEAD"`: its text up to `$MID`; `"$PARA_ID"`: its `w14:paraId`
//! - `"$T"`: table order index of the fixture's first top-level table
//! - `"$COMMENT"`, `"$FOOTNOTE"`, `"$ENDNOTE"`: the fixture's lowest id (for a
//!   comment, one anchored in the body)
//!
//! The rest resolve against the document as the scenario's earlier commands
//! left it, so a scenario can create an object and then edit it. Commands
//! therefore run one at a time, the selection carried between them as in one
//! batch.
//!
//! - `"$NEW_TABLE"`: table order index of the first table the fixture did not have
//! - `"$NEW_COMMENT"`, `"$NEW_FOOTNOTE"`, `"$NEW_ENDNOTE"`: the lowest id the
//!   fixture did not have
//! - `"$NUM_ID"`: the `w:numId` of the first list paragraph
//! - `"$HEADING"`: the text of the first navigation-pane heading
//! - `"$CHAR_STYLE"`, `"$TABLE_STYLE"`: a character / table style the package defines
//!
//! A scenario whose placeholders cannot be resolved (no such paragraph) or whose
//! commands the editor rejects for a fixture is reported and skipped; the gate
//! is about what the editor writes, not about which edits apply everywhere.
//! Output: `<output-dir>/<scenario>/<fixture>.docx` plus `<output-dir>/sweep.json`.

use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use jade_docx_editor::headless_api::apply_headless_commands_with_selection_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::{json, Value};

struct Anchor {
    paragraph: usize,
    length: usize,
    middle: usize,
    head: String,
    para_id: Option<String>,
    next: Option<(usize, usize)>,
}

/// The objects the fixture had before the scenario ran, which `$NEW_*` excludes.
struct Baseline {
    /// Table XML node ids; a table without one counts as the fixture's own.
    tables: HashSet<Option<u32>>,
    comments: HashSet<String>,
    footnotes: HashSet<String>,
    endnotes: HashSet<String>,
    /// `$T`, fixed up front: inserting a row or column rewrites the table's
    /// XML node, so the fixture's table cannot be found by node id afterwards.
    first_table: Option<usize>,
    /// `$COMMENT`; a reply needs an anchor in the body.
    anchored_comment: Option<String>,
}

fn paragraph_text(document: &WordDocument, index: usize) -> String {
    document
        .paragraph(index)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default()
}

fn find_anchor(document: &WordDocument) -> Option<Anchor> {
    let body: Vec<usize> = (0..document.paragraph_count())
        .filter(|&index| document.is_body_paragraph_surface(index))
        .collect();
    let position = body
        .iter()
        .position(|&index| paragraph_text(document, index).chars().count() >= 2)?;
    let paragraph = body[position];
    let text = paragraph_text(document, paragraph);
    let middle = text
        .char_indices()
        .map(|(offset, _)| offset)
        .nth(text.chars().count() / 2)
        .unwrap_or(0);
    let next = body
        .get(position + 1)
        .map(|&index| (index, paragraph_text(document, index).len()));
    let para_id = document.paragraph(paragraph).and_then(|paragraph| {
        paragraph
            .properties
            .as_ref()?
            .para_id
            .as_deref()
            .map(String::from)
    });
    Some(Anchor {
        paragraph,
        length: text.len(),
        middle,
        head: text[..middle].to_string(),
        para_id,
        next,
    })
}

/// `(XML node id, is top-level)` of every table in table order.
fn tables(document: &WordDocument) -> Vec<(Option<u32>, bool)> {
    (0..)
        .map_while(|index| {
            let table = document.table_by_order(index)?;
            let top_level = document.body_index_of_table_order(index).is_some();
            Some((table.node_id.map(|id| id.raw()), top_level))
        })
        .collect()
}

fn note_ids(notes: Vec<jade_docx_editor::word_document::NoteInfo>) -> HashSet<String> {
    notes.into_iter().map(|note| note.id).collect()
}

/// Numeric ids compare as numbers, so `"10"` comes after `"9"`.
fn lowest<'a>(ids: impl Iterator<Item = &'a String>) -> Option<String> {
    ids.min_by_key(|id| (id.parse::<u64>().unwrap_or(u64::MAX), id.as_str()))
        .cloned()
}

impl Baseline {
    fn of(document: &WordDocument) -> Self {
        let tables = tables(document);
        let comments: HashSet<String> = document.comments.keys().cloned().collect();
        let anchored = document.referenced_comment_ids();
        Self {
            first_table: tables.iter().position(|(_, top_level)| *top_level),
            tables: tables.into_iter().map(|(id, _)| id).collect(),
            anchored_comment: lowest(anchored.iter().filter(|id| comments.contains(*id))),
            comments,
            footnotes: note_ids(document.footnote_infos()),
            endnotes: note_ids(document.endnote_infos()),
        }
    }
}

/// The value of `text` if it is a placeholder; any other string stays as
/// written. `None` when this fixture has nothing for the placeholder.
fn placeholder(
    text: &str,
    anchor: &Anchor,
    baseline: &Baseline,
    document: &WordDocument,
) -> Option<Value> {
    Some(match text {
        "$P" => json!(anchor.paragraph),
        "$LEN" => json!(anchor.length),
        "$MID" => json!(anchor.middle),
        "$HEAD" => json!(anchor.head),
        "$PARA_ID" => json!(anchor.para_id.as_ref()?),
        "$P1" => json!(anchor.next?.0),
        "$LEN1" => json!(anchor.next?.1),
        "$T" => json!(baseline.first_table?),
        "$COMMENT" => json!(baseline.anchored_comment.as_ref()?),
        "$FOOTNOTE" => json!(lowest(baseline.footnotes.iter())?),
        "$ENDNOTE" => json!(lowest(baseline.endnotes.iter())?),
        "$NEW_TABLE" => json!(tables(document)
            .iter()
            .position(|(id, _)| id.is_some() && !baseline.tables.contains(id))?),
        "$NEW_COMMENT" => json!(lowest(
            document
                .comments
                .keys()
                .filter(|id| !baseline.comments.contains(*id))
        )?),
        "$NEW_FOOTNOTE" => json!(lowest(
            note_ids(document.footnote_infos())
                .iter()
                .filter(|id| !baseline.footnotes.contains(*id))
        )?),
        "$NEW_ENDNOTE" => json!(lowest(
            note_ids(document.endnote_infos())
                .iter()
                .filter(|id| !baseline.endnotes.contains(*id))
        )?),
        "$NUM_ID" => json!((0..document.paragraph_count()).find_map(|index| {
            let numbering = document
                .paragraph(index)?
                .properties
                .as_ref()?
                .numbering
                .as_ref()?;
            numbering.list_id().map(String::from)
        })?),
        "$HEADING" => json!(document.navigation_headings().first()?.text.trim()),
        "$CHAR_STYLE" => json!(document
            .styles_part
            .as_ref()?
            .character_style_definitions()
            .filter(|style| !style.is_default)
            .map(|style| &style.style_id)
            .min()?),
        "$TABLE_STYLE" => json!(document.table_style_infos().first()?.style_id),
        _ => json!(text),
    })
}

/// `value` with its placeholders resolved; `Err` names the one that is not.
fn resolve(
    value: &Value,
    anchor: &Anchor,
    baseline: &Baseline,
    document: &WordDocument,
) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => placeholder(text, anchor, baseline, document)
            .ok_or_else(|| format!("{text} does not resolve"))?,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve(item, anchor, baseline, document))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), resolve(item, anchor, baseline, document)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => value.clone(),
    })
}

/// Applies the scenario to `document`. `Ok(Some(reason))` when it does not
/// apply to this fixture.
fn apply(
    document: &mut WordDocument,
    commands: &[Value],
) -> Result<Option<String>, Box<dyn Error>> {
    let Some(anchor) = find_anchor(document) else {
        return Ok(Some("no body paragraph with text".to_string()));
    };
    let baseline = Baseline::of(document);
    let mut selection = None;
    for (index, command) in commands.iter().enumerate() {
        let command = match resolve(command, &anchor, &baseline, document) {
            Ok(command) => command,
            Err(reason) => return Ok(Some(format!("command {index}: {reason}"))),
        };
        let request: HeadlessEditRequest =
            serde_json::from_value(json!({ "commands": [command] }))?;
        match apply_headless_commands_with_selection_to_document(document, selection, &request) {
            Ok(next) => selection = next,
            Err(error) => return Ok(Some(format!("command {index}: {error}"))),
        }
    }
    Ok(None)
}

fn load(bytes: &[u8]) -> Result<WordDocument, Box<dyn Error>> {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default()))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let scenarios_path = PathBuf::from(args.next().ok_or("scenarios.json required")?);
    let output = PathBuf::from(args.next().ok_or("output directory required")?);
    let scenarios: BTreeMap<String, Vec<Value>> =
        serde_json::from_str(&fs::read_to_string(&scenarios_path)?)?;
    let only = std::env::var("JADE_SWEEP_SCENARIO").ok();

    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut fixtures = fs::read_dir(fixture_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    fixtures.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("docx"));
    fixtures.sort();

    let mut report = BTreeMap::<String, Value>::new();
    for (name, commands) in &scenarios {
        if only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        let directory = output.join(name);
        fs::create_dir_all(&directory)?;
        let (mut written, mut skipped) = (Vec::new(), BTreeMap::new());
        for fixture in &fixtures {
            let file_name = fixture.file_name().ok_or("fixture has no name")?;
            let key = file_name.to_string_lossy().to_string();
            let bytes = fs::read(fixture)?;
            let mut document = load(&bytes)?;
            if let Some(reason) = apply(&mut document, commands)? {
                skipped.insert(key, json!(reason));
                continue;
            }
            let exported = document.export_docx()?;
            // A package the editor cannot read back is a failure in its own right.
            load(&exported).map_err(|error| format!("{name}/{key} does not reload: {error}"))?;
            fs::write(directory.join(file_name), exported)?;
            written.push(key);
        }
        println!(
            "{name}: wrote {} fixtures, skipped {}",
            written.len(),
            skipped.len()
        );
        report.insert(
            name.clone(),
            json!({ "written": written, "skipped": skipped }),
        );
    }
    fs::write(
        output.join("sweep.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(())
}
