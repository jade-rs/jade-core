//! Word fields as a whole — the machinery behind 挿入 › フィールド, F9,
//! ⌥F9/⇧F9, ⌘F11/⌘⇧F11 and ⌘⇧F9:
//!
//! - **update** ([`update_fields`]): every field in the document, or those
//!   the selection touches, is evaluated as Word does (see [`eval`]) and its
//!   result written back, nested fields first;
//! - **lock / unlock** ([`set_fields_locked`]) and **unlink**
//!   ([`unlink_fields`], the result kept as plain text);
//! - **insert** ([`insert_field`]) and **edit** ([`edit_field_code`]) a
//!   field from its code, nested `{ }` fields included;
//! - **show codes** ([`field_code_display`]): what each run shows while
//!   field codes are on;
//! - **prompts** ([`field_prompts`]): what ASK and FILLIN will ask;
//! - **properties** ([`document_properties`], [`set_document_properties`])
//!   and document variables, which the property fields read.
//!
//! Each edit works on the package: the stories' XML is rewritten, the
//! package rebuilt, and the document reloaded from it — one atomic,
//! undoable replacement, as a browser import is.

mod eval;
mod formula;
mod picture;
pub mod properties;
mod scan;
mod write;

pub use properties::{CustomProperty, CustomValue, DocumentProperties};

use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode, XmlNodeId};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use eval::{DocumentData, Evaluation, Outcome, StoryKind};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use jade_docx_reader::field_datetime::FieldDateTime;
use jade_docx_reader::field_format::FieldCode;
use scan::{Role, Story};
use std::collections::{HashMap, HashSet};
use write::NewResult;

/// Reads a file a field names, relative to the document's folder.
pub type FileReader = Box<dyn Fn(&str) -> Option<Vec<u8>>>;

/// What field evaluation needs from outside the document.
pub struct FieldEnvironment {
    /// Local date and time now (DATE, TIME).
    pub now: FieldDateTime,
    /// The local zone's offset east of UTC, for package timestamps
    /// (CREATEDATE, SAVEDATE, PRINTDATE).
    pub utc_offset_seconds: i64,
    /// The user information of the app's settings (USERNAME, …).
    pub user_name: String,
    pub user_initials: String,
    pub user_address: String,
    /// The document's file name and full path, when it has been saved
    /// (FILENAME).
    pub file_name: Option<String>,
    pub file_path: Option<String>,
    /// The saved file's size in bytes (FILESIZE).
    pub file_size: Option<u64>,
    /// Pages in the laid-out document (NUMPAGES).
    pub page_count: Option<usize>,
    /// The page (1-based) each paragraph starts on, by paragraph index.
    pub paragraph_pages: HashMap<usize, usize>,
    /// Answers to ASK and FILLIN prompts, by [`FieldPrompt::key`].
    pub prompt_answers: HashMap<String, String>,
    /// Reads a local file a field names (INCLUDETEXT), relative to the
    /// document's folder.
    pub read_file: Option<FileReader>,
    /// Whether results and messages are Japanese (Word's language).
    pub japanese: bool,
    /// The merge a mail-merge main document previews, and the recipient (an
    /// index into its records): MERGEFIELD, ADDRESSBLOCK, GREETINGLINE,
    /// MERGEREC and MERGESEQ evaluate through the mail-merge evaluator for
    /// it. Without one they show their «names».
    pub merge: Option<(crate::word_document::mail_merge::MergeRequest, usize)>,
}

impl Default for FieldEnvironment {
    fn default() -> Self {
        Self {
            now: FieldDateTime {
                year: 2000,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                weekday: 6,
            },
            utc_offset_seconds: 0,
            user_name: String::new(),
            user_initials: String::new(),
            user_address: String::new(),
            file_name: None,
            file_path: None,
            file_size: None,
            page_count: None,
            paragraph_pages: HashMap::new(),
            prompt_answers: HashMap::new(),
            read_file: None,
            japanese: true,
            merge: None,
        }
    }
}

/// Which fields an operation reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldScope {
    /// Every field of every story: body, headers, footers and notes.
    Document,
    /// The fields the selection touches; the one at a caret.
    Selection(SelectionRange),
}

/// A prompt an update will show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPrompt {
    /// What the answer is filed under in [`FieldEnvironment::prompt_answers`].
    pub key: String,
    /// `ASK` or `FILLIN`.
    pub field: String,
    pub prompt: String,
    pub default: String,
    /// The bookmark an ASK assigns.
    pub bookmark: Option<String>,
}

/// A field the selection touches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    /// The code as Word shows it, nested fields in braces.
    pub code: String,
    /// The field name (`DATE`, `=`, …).
    pub name: String,
    pub result: String,
    pub locked: bool,
}

/// Which fields show their codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldCodeView {
    /// ⌥F9: every field.
    pub all: bool,
    /// ⇧F9: these fields, by the id of their first run (see
    /// [`FieldCodeToggle`]) — each toggled against `all`.
    pub toggled: HashSet<XmlNodeId>,
}

/// The ids ⇧F9 toggles: the fields the selection touches, by their first
/// run.
pub fn field_toggle_ids(doc: &WordDocument, range: SelectionRange) -> Vec<XmlNodeId> {
    let Some(raw) = doc.raw_document.as_ref() else {
        return Vec::new();
    };
    let story = scan::scan_document(raw.root());
    let selected = selected_fields(doc, &story, range);
    selected
        .into_iter()
        .filter_map(|index| field_anchor(&story.fields[index]))
        .collect()
}

/// For the view toggled by ⇧F9: a field's identity is the id of its first
/// run (or its simple field element).
pub type FieldCodeToggle = XmlNodeId;

fn field_anchor(field: &scan::ScannedField) -> Option<XmlNodeId> {
    field.simple.or(field.begin.map(|marker| marker.run))
}

/// What each body run shows while codes are on: every run of a shown field
/// maps to the field (by its first run) and its `{ CODE }`, which the
/// renderer draws once, at the first of them it lays out.
pub fn field_code_display(
    doc: &WordDocument,
    view: &FieldCodeView,
) -> crate::canvas_renderer::FieldCodeDisplay {
    let mut display = HashMap::new();
    let Some(raw) = doc.raw_document.as_ref() else {
        return display;
    };
    let story = scan::scan_document(raw.root());
    for (index, field) in story.fields.iter().enumerate() {
        // Nested fields show inside their parent's code.
        if matches!(field.parent, Some((_, Role::Operand))) {
            continue;
        }
        let Some(anchor) = field_anchor(field) else {
            continue;
        };
        let shown = view.all != view.toggled.contains(&anchor);
        // A field inside another's shown result is hidden with it.
        let in_shown_result = story.fields.iter().enumerate().any(|(other, outer)| {
            other != index
                && story.in_result_of(index, other)
                && field_anchor(outer)
                    .is_some_and(|anchor| view.all != view.toggled.contains(&anchor))
        });
        if !shown || in_shown_result {
            continue;
        }
        let code: std::rc::Rc<str> =
            format!("{{ {} }}", field.display_code(&story.fields).trim()).into();
        let runs: Vec<XmlNodeId> = field
            .simple
            .map(|simple| {
                write::find_element(raw.root(), simple)
                    .map(|element| {
                        element
                            .children
                            .iter()
                            .filter_map(|child| match child {
                                XmlNode::Element(run) if run.name == "w:r" => Some(run.id),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| field.span_runs.clone());
        for run in runs {
            display.insert(run, (anchor, code.clone()));
        }
    }
    display
}

/// The fields the selection touches, in order.
pub fn fields_at(doc: &WordDocument, range: SelectionRange) -> Vec<FieldInfo> {
    let Some(raw) = doc.raw_document.as_ref() else {
        return Vec::new();
    };
    let story = scan::scan_document(raw.root());
    selected_fields(doc, &story, range)
        .into_iter()
        .map(|index| {
            let field = &story.fields[index];
            let code = field.display_code(&story.fields).trim().to_string();
            let name = FieldCode::parse(&field.instruction_with(|_| String::new()))
                .map(|code| code.name)
                .unwrap_or_default();
            FieldInfo {
                code,
                name,
                result: field.result_text.clone(),
                locked: field.locked,
            }
        })
        .collect()
}

/// Body runs and paragraphs the selection covers, by XML id.
fn selected_ids(
    doc: &WordDocument,
    range: SelectionRange,
) -> (HashSet<XmlNodeId>, HashSet<XmlNodeId>) {
    let (start, end) = ordered(range);
    let start = doc.rendered_to_model_text_position(start);
    let end = doc.rendered_to_model_text_position(end);
    let mut runs = HashSet::new();
    let mut paragraphs = HashSet::new();
    for index in start.paragraph..=end.paragraph {
        if doc.is_note_paragraph_surface(index) {
            continue;
        }
        let Some(paragraph) = doc.paragraph(index) else {
            continue;
        };
        let from = if index == start.paragraph {
            start.offset
        } else {
            0
        };
        let to = if index == end.paragraph {
            end.offset
        } else {
            usize::MAX
        };
        let mut offset = 0;
        let length: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
        for run in &paragraph.runs {
            let run_end = offset + run.text.len();
            if offset <= to && run_end >= from {
                if let Some(id) = run.node_id {
                    runs.insert(id);
                }
            }
            offset = run_end;
        }
        if from == 0 && to >= length && start != end {
            if let Some(id) = paragraph.node_id {
                paragraphs.insert(id);
            }
        }
    }
    (runs, paragraphs)
}

fn ordered(range: SelectionRange) -> (TextPosition, TextPosition) {
    let (a, b) = (range.anchor, range.focus);
    if (a.paragraph, a.offset) <= (b.paragraph, b.offset) {
        (a, b)
    } else {
        (b, a)
    }
}

/// The fields (outermost, not nested operands) the selection touches.
fn selected_fields(doc: &WordDocument, story: &Story, range: SelectionRange) -> Vec<usize> {
    let (runs, paragraphs) = selected_ids(doc, range);
    let (start, end) = ordered(range);
    let start = doc.rendered_to_model_text_position(start);
    let end = doc.rendered_to_model_text_position(end);
    let surfaces: HashMap<_, _> = (0..doc.paragraph_count())
        .filter(|&index| !doc.is_note_paragraph_surface(index))
        .filter_map(|index| {
            let paragraph = doc.paragraph(index)?;
            let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            paragraph.node_id.map(|id| (id, (index, text)))
        })
        .collect();
    let position = |paragraph: usize, offset: usize| {
        story
            .paragraphs
            .get(paragraph)
            .and_then(|paragraph| {
                surfaces
                    .get(&paragraph.id)
                    .filter(|(_, text)| *text == paragraph.text)
            })
            .map(|(surface, _)| (*surface, offset))
    };
    let mut selected = Vec::new();
    for (index, field) in story.fields.iter().enumerate() {
        // Offsets distinguish fields sharing a run. Keep the XML-id path for
        // paragraphs whose model text includes objects the story scanner omits.
        let touched = position(field.paragraph, field.start_offset)
            .zip(position(field.end_paragraph, field.end_offset))
            .filter(|_| field.simple.is_some() || field.end.is_some())
            .map_or_else(
                || {
                    field.span_runs.iter().any(|run| runs.contains(run))
                        || story
                            .paragraphs
                            .get(field.paragraph)
                            .is_some_and(|paragraph| paragraphs.contains(&paragraph.id))
                        || field.simple.is_some_and(|simple| {
                            write::find_element(
                                doc.raw_document
                                    .as_ref()
                                    .map(|raw| raw.root())
                                    .expect("raw"),
                                simple,
                            )
                            .is_some_and(|element| {
                                element.children.iter().any(
                        |child| matches!(child, XmlNode::Element(run) if runs.contains(&run.id)),
                    )
                            })
                        })
                },
                |(from, to)| {
                    from <= (end.paragraph, end.offset) && to >= (start.paragraph, start.offset)
                },
            );
        if !touched {
            continue;
        }
        // An operand is reached through its outermost field.
        let mut top = index;
        while let Some((parent, Role::Operand)) = story.fields[top].parent {
            top = parent;
        }
        if !selected.contains(&top) {
            selected.push(top);
        }
    }
    selected
}

/// The operands nested (at any depth) in `field`'s code.
fn with_operands(story: &Story, fields: &[usize]) -> HashSet<usize> {
    let mut all: HashSet<usize> = fields.iter().copied().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (index, field) in story.fields.iter().enumerate() {
            if let Some((parent, Role::Operand)) = field.parent {
                if all.contains(&parent) && all.insert(index) {
                    changed = true;
                }
            }
        }
    }
    all
}

/// The package's files, as the document would save them.
fn unpack(doc: &WordDocument) -> Result<HashMap<String, Vec<u8>>, String> {
    let bytes = doc.export_docx().map_err(|error| error.to_string())?;
    unzip(&bytes)
}

fn unzip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    use std::io::Read;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut files = HashMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        if file.is_dir() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|error| error.to_string())?;
        files.insert(
            file.name().map_err(|err| err.to_string())?.into_owned(),
            contents,
        );
    }
    Ok(files)
}

/// Polls a future that never waits (the document loader).
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

/// Rebuilds the package from `files` and makes it the document.
fn replace_with(doc: &mut WordDocument, files: &HashMap<String, Vec<u8>>) -> Result<(), String> {
    let bytes = jade_docx_writer::write_docx_with_removed(files, &HashMap::new(), &HashSet::new())
        .map_err(|error| error.to_string())?;
    let mut loaded = block_on(WordDocument::load(&bytes, &crate::Options::default()))
        .map_err(|error| error.to_string())?;
    // Change tracking goes on as it was.
    loaded.set_revision_session(doc.revision_session().cloned());
    let snapshot = loaded
        .snapshot_for_replacement()
        .ok_or("the rebuilt document has no body")?;
    doc.restore_snapshot(snapshot);
    Ok(())
}

fn parse_part(files: &HashMap<String, Vec<u8>>, path: &str) -> Option<DocxXmlDocument> {
    DocxXmlDocument::parse(&String::from_utf8_lossy(files.get(path)?)).ok()
}

fn serialize(doc: &DocxXmlDocument) -> Result<Vec<u8>, String> {
    doc.to_string()
        .map(String::into_bytes)
        .map_err(|error| format!("{error:?}"))
}

/// The stories besides the body: headers, footers and notes.
fn other_stories(files: &HashMap<String, Vec<u8>>) -> Vec<String> {
    let mut parts: Vec<String> = files
        .keys()
        .filter(|path| {
            let name = path.strip_prefix("word/").unwrap_or_default();
            !name.contains('/')
                && name.ends_with(".xml")
                && (name.starts_with("header")
                    || name.starts_with("footer")
                    || name == "footnotes.xml"
                    || name == "endnotes.xml")
        })
        .cloned()
        .collect();
    parts.sort();
    parts
}

fn root_mut(doc: &mut DocxXmlDocument) -> &mut XmlElement {
    let id = doc.root().id;
    doc.element_mut(id).expect("a parsed document has a root")
}

fn glossary(files: &HashMap<String, Vec<u8>>) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    let Some(doc) = parse_part(files, "word/glossary/document.xml") else {
        return entries;
    };
    fn walk(element: &XmlElement, entries: &mut HashMap<String, String>) {
        if element.name == "w:docPart" {
            let name = scan::child_element(element, "w:docPartPr")
                .and_then(|pr| scan::child_element(pr, "w:name"))
                .and_then(|name| {
                    name.attributes
                        .iter()
                        .find(|attribute| attribute.name == "w:val")
                        .map(|attribute| attribute.value.clone())
                });
            if let (Some(name), Some(body)) = (name, scan::child_element(element, "w:docPartBody"))
            {
                let story = scan::scan_story(body);
                let text = story
                    .paragraphs
                    .iter()
                    .map(|paragraph| paragraph.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                entries.insert(name.to_lowercase(), text);
            }
            return;
        }
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                walk(child, entries);
            }
        }
    }
    walk(doc.root(), &mut entries);
    entries
}

fn document_data(
    doc: &WordDocument,
    files: &HashMap<String, Vec<u8>>,
    body: &Story,
) -> DocumentData {
    let mut data = DocumentData {
        properties: DocumentProperties::read(files),
        variables: properties::read_variables(files),
        glossary: glossary(files),
        sources: doc.citation_sources(),
        note_references: body.note_references.clone(),
        ..DocumentData::default()
    };
    let metrics = doc.document_text_metrics();
    data.words = metrics.words;
    data.characters = metrics.chars_no_space;
    data.characters_with_spaces = metrics.chars_with_space;
    if let Some(styles) = doc.styles_part.as_ref() {
        for style in styles.paragraph_style_definitions() {
            data.style_ids.insert(
                eval::normalize_style_name(&style.style_id),
                style.style_id.clone(),
            );
            if let Some(name) = &style.name {
                data.style_ids
                    .insert(eval::normalize_style_name(name), style.style_id.clone());
            }
            if let Some(level) = styles
                .resolve_paragraph_properties(&style.style_id)
                .and_then(|properties| properties.outline_level)
            {
                data.style_outline.insert(style.style_id.clone(), level);
            }
        }
    }
    data
}

/// Evaluates and writes one story's fields.
#[allow(clippy::too_many_arguments)]
fn update_story(
    root: &mut XmlElement,
    story: &Story,
    data: &DocumentData,
    env: &FieldEnvironment,
    kind: StoryKind,
    part: &str,
    page_of: &dyn Fn(usize) -> Option<usize>,
    selected: Option<&HashSet<usize>>,
    files: &mut HashMap<String, Vec<u8>>,
) -> Result<(usize, Vec<(String, String)>), String> {
    let mut evaluation = Evaluation::new(story, data, env, kind, part, page_of, selected);
    evaluation.run();
    let outcomes = std::mem::take(&mut evaluation.outcomes);
    let mut written = 0;
    // Last first: a write only moves what follows it.
    for (index, outcome) in outcomes.into_iter().enumerate().rev() {
        let Some(Outcome {
            result,
            format,
            symbol_font,
        }) = outcome
        else {
            continue;
        };
        // A field inside the result of one rewritten here went with it.
        let field = &story.fields[index];
        if field.end.is_none() && field.simple.is_none() {
            continue;
        }
        if unchanged(field, &result) {
            continue;
        }
        let mut rpr = write::result_properties(root, field, format.char_format());
        if let Some((font, size)) = symbol_font {
            rpr = Some(symbol_properties(rpr, font.as_deref(), size));
        }
        let prepared = match &result {
            NewResult::Text(text) if format.merge_format() && field.result_runs.len() > 1 => {
                Some(write::merge_format_runs(root, field, text))
            }
            _ => None,
        };
        let result = match result {
            NewResult::Picture { bytes, name } => NewResult::Runs(vec![picture::picture_run(
                files,
                part,
                root,
                &bytes,
                &name,
                rpr.as_ref(),
            )?]),
            other => other,
        };
        write::write_result(root, field, result, rpr.as_ref(), prepared)?;
        written += 1;
    }
    Ok((written, evaluation.property_changes))
}

fn unchanged(field: &scan::ScannedField, result: &NewResult) -> bool {
    match result {
        NewResult::Text(text) => {
            *text == field.result_text && !field.result_runs.is_empty() && !text.is_empty()
        }
        _ => false,
    }
}

fn symbol_properties(rpr: Option<XmlElement>, font: Option<&str>, size: Option<f32>) -> XmlElement {
    let mut rpr = rpr.unwrap_or_else(|| XmlElement {
        id: XmlNodeId::new(0),
        name: "w:rPr".to_string(),
        attributes: Vec::new(),
        children: Vec::new(),
    });
    let element = |name: &str, attributes: Vec<(&str, String)>| {
        XmlNode::Element(XmlElement {
            id: XmlNodeId::new(0),
            name: name.to_string(),
            attributes: attributes
                .into_iter()
                .map(|(name, value)| crate::docx_xml::XmlAttribute {
                    name: name.to_string(),
                    value,
                })
                .collect(),
            children: Vec::new(),
        })
    };
    if let Some(font) = font {
        rpr.children
            .retain(|child| !matches!(child, XmlNode::Element(e) if e.name == "w:rFonts"));
        rpr.children.insert(
            0,
            element(
                "w:rFonts",
                vec![
                    ("w:ascii", font.to_string()),
                    ("w:hAnsi", font.to_string()),
                    ("w:eastAsia", font.to_string()),
                    ("w:cs", font.to_string()),
                ],
            ),
        );
    }
    if let Some(size) = size {
        rpr.children.retain(
            |child| !matches!(child, XmlNode::Element(e) if e.name == "w:sz" || e.name == "w:szCs"),
        );
        let half_points = ((size * 2.0).round() as u32).to_string();
        rpr.children
            .push(element("w:sz", vec![("w:val", half_points.clone())]));
        rpr.children
            .push(element("w:szCs", vec![("w:val", half_points)]));
    }
    rpr
}

/// Updates fields: every field of every story, or those the selection
/// touches. Returns how many results changed.
pub fn update_fields(
    doc: &mut WordDocument,
    scope: FieldScope,
    env: &FieldEnvironment,
) -> Result<usize, String> {
    update_fields_in_scope(doc, scope, env, None)
}

fn update_fields_in_scope(
    doc: &mut WordDocument,
    scope: FieldScope,
    env: &FieldEnvironment,
    edited_field: Option<usize>,
) -> Result<usize, String> {
    let mut files = unpack(doc)?;
    let mut body = match doc.raw_document.clone() {
        Some(raw) => raw,
        None => parse_part(&files, "word/document.xml").ok_or("the document has no body")?,
    };
    let story = scan::scan_document(body.root());
    let data = document_data(doc, &files, &story);
    let mut surfaces = HashMap::new();
    for index in 0..doc.paragraph_count() {
        if let Some(id) = doc.paragraph(index).and_then(|paragraph| paragraph.node_id) {
            surfaces.insert(id, index);
        }
    }
    let page_of = |paragraph: usize| {
        story
            .paragraphs
            .get(paragraph)
            .and_then(|paragraph| surfaces.get(&paragraph.id))
            .and_then(|surface| env.paragraph_pages.get(surface).copied())
    };
    let selected = match scope {
        FieldScope::Document => None,
        FieldScope::Selection(range) => {
            let top = edited_field
                .map(|index| vec![index])
                .unwrap_or_else(|| selected_fields(doc, &story, range));
            Some(with_operands(&story, &top))
        }
    };
    let toc_targets: Vec<usize> = doc
        .tables_of_contents()
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            story.fields.iter().enumerate().any(|(index, field)| {
                field.parent.is_none()
                    && !field.locked
                    && selected
                        .as_ref()
                        .is_none_or(|selected| selected.contains(&index))
                    && story
                        .paragraphs
                        .get(field.paragraph)
                        .and_then(|paragraph| surfaces.get(&paragraph.id))
                        == Some(&block.start)
                    && field.instruction_with(|_| String::new()).trim() == block.instruction
            })
        })
        .map(|(index, _)| index)
        .collect();
    let (mut written, mut changes) = update_story(
        root_mut(&mut body),
        &story,
        &data,
        env,
        StoryKind::Body,
        "word/document.xml",
        &page_of,
        selected.as_ref(),
        &mut files,
    )?;
    if written > 0 {
        body.rebuild_index();
        files.insert("word/document.xml".to_string(), serialize(&body)?);
    }
    if scope == FieldScope::Document {
        let mut header_data = data.clone();
        header_data.note_references = story.note_references.clone();
        header_data.body_paragraphs = story.paragraphs.clone();
        for part in other_stories(&files) {
            let Some(mut doc_part) = parse_part(&files, &part) else {
                continue;
            };
            let part_story = scan::scan_story(doc_part.root());
            if part_story.fields.is_empty() {
                continue;
            }
            let kind = if part.contains("notes") {
                StoryKind::Notes
            } else {
                StoryKind::HeaderFooter
            };
            let no_pages = |_: usize| None;
            let (count, part_changes) = update_story(
                root_mut(&mut doc_part),
                &part_story,
                &header_data,
                env,
                kind,
                &part,
                &no_pages,
                None,
                &mut files,
            )?;
            if count > 0 {
                doc_part.rebuild_index();
                files.insert(part.clone(), serialize(&doc_part)?);
                written += count;
            }
            changes.extend(part_changes);
        }
    }
    if !changes.is_empty() {
        let mut properties = data.properties.clone();
        for (field, value) in &changes {
            properties.set_by_field_name(field, value);
        }
        properties.write(&mut files)?;
    }
    if written == 0 && changes.is_empty() && toc_targets.is_empty() {
        return Ok(0);
    }
    if written > 0 || !changes.is_empty() {
        replace_with(doc, &files)?;
    }
    // Updating the last table first keeps earlier table indices stable.
    for index in toc_targets.into_iter().rev() {
        doc.update_table_of_contents_at(index)?;
        written += 1;
    }
    Ok(written)
}

/// The prompts ASK and FILLIN fields in scope will show, in order.
pub fn field_prompts(doc: &WordDocument, scope: FieldScope) -> Vec<FieldPrompt> {
    let mut prompts = Vec::new();
    let Ok(files) = unpack(doc) else {
        return prompts;
    };
    let body = doc
        .raw_document
        .clone()
        .or_else(|| parse_part(&files, "word/document.xml"));
    let Some(body) = body else {
        return prompts;
    };
    let mut stories = vec![(
        "word/document.xml".to_string(),
        scan::scan_document(body.root()),
    )];
    let selected = match scope {
        FieldScope::Document => {
            for part in other_stories(&files) {
                if let Some(part_doc) = parse_part(&files, &part) {
                    stories.push((part.clone(), scan::scan_story(part_doc.root())));
                }
            }
            None
        }
        FieldScope::Selection(range) => {
            let top = selected_fields(doc, &stories[0].1, range);
            Some(with_operands(&stories[0].1, &top))
        }
    };
    for (part_index, (part, story)) in stories.iter().enumerate() {
        for (index, field) in story.fields.iter().enumerate() {
            if field.locked {
                continue;
            }
            if part_index == 0 && selected.as_ref().is_some_and(|s| !s.contains(&index)) {
                continue;
            }
            let instruction =
                field.instruction_with(|child| story.fields[child].result_text.clone());
            let Some(code) = FieldCode::parse(&instruction) else {
                continue;
            };
            let (bookmark, prompt) = match code.name.as_str() {
                "ASK" => (code.operand(0).map(str::to_string), code.operand(1)),
                "FILLIN" => (None, code.operand(0)),
                _ => continue,
            };
            prompts.push(FieldPrompt {
                key: Evaluation::prompt_key(part, index),
                field: code.name.clone(),
                prompt: prompt.unwrap_or_default().to_string(),
                default: code
                    .switch_value("d")
                    .map(str::to_string)
                    .unwrap_or_else(|| field.result_text.clone()),
                bookmark,
            });
        }
    }
    prompts
}

/// Applies `edit` to the fields in scope of every story it reaches, and
/// reloads. Returns how many fields it changed.
fn edit_fields(
    doc: &mut WordDocument,
    scope: FieldScope,
    mut edit: impl FnMut(&mut XmlElement, &scan::ScannedField, usize) -> Result<bool, String>,
) -> Result<usize, String> {
    let mut files = unpack(doc)?;
    let mut body = doc
        .raw_document
        .clone()
        .or_else(|| parse_part(&files, "word/document.xml"))
        .ok_or("the document has no body")?;
    let story = scan::scan_document(body.root());
    let targets: Vec<usize> = match scope {
        FieldScope::Document => (0..story.fields.len()).collect(),
        FieldScope::Selection(range) => {
            let top = selected_fields(doc, &story, range);
            // Fields inside a selected field's result go with it.
            let mut targets: Vec<usize> = (0..story.fields.len())
                .filter(|&index| {
                    top.contains(&index)
                        || top.iter().any(|&outer| story.in_result_of(index, outer))
                })
                .collect();
            targets.sort_unstable();
            targets
        }
    };
    let mut changed = 0;
    {
        let root = root_mut(&mut body);
        for &index in targets.iter().rev() {
            if matches!(story.fields[index].parent, Some((_, Role::Operand))) {
                continue;
            }
            if edit(root, &story.fields[index], index)? {
                changed += 1;
            }
        }
    }
    if changed > 0 {
        body.rebuild_index();
        files.insert("word/document.xml".to_string(), serialize(&body)?);
    }
    if scope == FieldScope::Document {
        for part in other_stories(&files) {
            let Some(mut part_doc) = parse_part(&files, &part) else {
                continue;
            };
            let part_story = scan::scan_story(part_doc.root());
            let mut part_changed = 0;
            {
                let root = root_mut(&mut part_doc);
                for (index, field) in part_story.fields.iter().enumerate().rev() {
                    if matches!(field.parent, Some((_, Role::Operand))) {
                        continue;
                    }
                    if edit(root, field, index)? {
                        part_changed += 1;
                    }
                }
            }
            if part_changed > 0 {
                part_doc.rebuild_index();
                files.insert(part.clone(), serialize(&part_doc)?);
                changed += part_changed;
            }
        }
    }
    if changed > 0 {
        replace_with(doc, &files)?;
    }
    Ok(changed)
}

/// Locks (⌘F11) or unlocks (⌘⇧F11) the fields in scope.
pub fn set_fields_locked(
    doc: &mut WordDocument,
    scope: FieldScope,
    locked: bool,
) -> Result<usize, String> {
    edit_fields(doc, scope, |root, field, _| {
        if field.locked == locked {
            return Ok(false);
        }
        Ok(write::set_locked(root, field, locked))
    })
}

/// Replaces the fields in scope with their results (⌘⇧F9).
pub fn unlink_fields(doc: &mut WordDocument, scope: FieldScope) -> Result<usize, String> {
    edit_fields(doc, scope, |root, field, _| {
        write::unlink(root, field)?;
        Ok(true)
    })
}

/// Replaces the code of the field at the selection and updates it.
pub fn edit_field_code(
    doc: &mut WordDocument,
    range: SelectionRange,
    code: &str,
    env: &FieldEnvironment,
) -> Result<(), String> {
    let code = code
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    if code.is_empty() {
        return Err("the field code is empty".to_string());
    }
    let mut edited_field = None;
    let changed = edit_fields(doc, FieldScope::Selection(range), |root, field, index| {
        if edited_field.is_some() {
            return Ok(false);
        }
        write::replace_code(root, field, code)?;
        edited_field = Some(index);
        Ok(true)
    })?;
    if changed == 0 {
        return Err("there is no field at the selection".to_string());
    }
    // Changing from an editable result to a computed field can change model
    // offsets. Its preorder index survives replacing its own instruction and
    // operands, so update that field rather than reinterpreting the old caret.
    update_fields_in_scope(doc, FieldScope::Selection(range), env, edited_field)?;
    Ok(())
}

const PLACEHOLDER: &str = "\u{E000}jade-field\u{E001}";

/// Inserts a field with `code` (nested fields in `{ }`) at the selection,
/// replacing what it selects, and evaluates it. Returns where the caret
/// goes: after the field.
pub fn insert_field(
    doc: &mut WordDocument,
    selection: &mut Option<SelectionRange>,
    code: &str,
    env: &FieldEnvironment,
) -> Result<(), String> {
    let code = code
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .to_string();
    if code.is_empty() {
        return Err("the field code is empty".to_string());
    }
    let range = selection.ok_or("there is no insertion point")?;
    let (start, _) = ordered(range);
    {
        let mut ctx = CommandContext::new(doc, selection);
        let mut insert = crate::commands::insert_text::InsertTextCommand::new(
            range,
            PLACEHOLDER.to_string(),
            None,
        );
        insert
            .apply(&mut ctx)
            .map_err(|error| format!("{error:?}"))?;
    }
    let mut files = unpack(doc)?;
    let mut parts = vec!["word/document.xml".to_string()];
    parts.extend(other_stories(&files));
    let mut inserted = None;
    for part in parts {
        if !files
            .get(&part)
            .is_some_and(|bytes| String::from_utf8_lossy(bytes).contains(PLACEHOLDER))
        {
            continue;
        }
        // The body's live tree keeps the ids the model's paragraphs have,
        // which page lookup goes through.
        let part_doc = if part == "word/document.xml" {
            doc.raw_document
                .clone()
                .or_else(|| parse_part(&files, &part))
        } else {
            parse_part(&files, &part)
        };
        let Some(mut part_doc) = part_doc else {
            continue;
        };
        let new_field = place_field(root_mut(&mut part_doc), &code)?;
        part_doc.rebuild_index();
        // Evaluate the new field where it now stands.
        let story = if part == "word/document.xml" {
            scan::scan_document(part_doc.root())
        } else {
            scan::scan_story(part_doc.root())
        };
        let index = story
            .fields
            .iter()
            .position(|field| field.begin.is_some_and(|marker| marker.run == new_field))
            .ok_or("the new field was not found")?;
        let data = document_data(doc, &files, &story);
        let selected = with_operands(&story, &[index]);
        let kind = if part == "word/document.xml" {
            StoryKind::Body
        } else {
            StoryKind::Notes
        };
        let no_pages = |_: usize| None;
        let surfaces: HashMap<XmlNodeId, usize> = (0..doc.paragraph_count())
            .filter_map(|surface| {
                doc.paragraph(surface)
                    .and_then(|paragraph| paragraph.node_id)
                    .map(|id| (id, surface))
            })
            .collect();
        let body_pages = |paragraph: usize| {
            story
                .paragraphs
                .get(paragraph)
                .and_then(|paragraph| surfaces.get(&paragraph.id))
                .and_then(|surface| env.paragraph_pages.get(surface).copied())
        };
        let page_of: &dyn Fn(usize) -> Option<usize> = if kind == StoryKind::Body {
            &body_pages
        } else {
            &no_pages
        };
        let (_, changes) = update_story(
            root_mut(&mut part_doc),
            &story,
            &data,
            env,
            kind,
            &part,
            page_of,
            Some(&selected),
            &mut files,
        )?;
        part_doc.rebuild_index();
        let result = scan::scan_story(part_doc.root());
        let result_text = result
            .fields
            .iter()
            .find(|field| field.begin.is_some_and(|marker| marker.run == new_field))
            .map(|field| field.result_text.clone())
            .unwrap_or_default();
        files.insert(part.clone(), serialize(&part_doc)?);
        if !changes.is_empty() {
            let mut properties = data.properties.clone();
            for (field, value) in &changes {
                properties.set_by_field_name(field, value);
            }
            properties.write(&mut files)?;
        }
        inserted = Some(result_text);
        break;
    }
    let result_text = inserted.ok_or("the insertion point was not found")?;
    replace_with(doc, &files)?;
    // A computed result takes no room in the text the caret moves through;
    // a REF, HYPERLINK or FILLIN result is text.
    let keeps_text = jade_docx_reader::parser::DocumentParser::parse_field_instruction(&code)
        .is_some_and(|kind| kind.keeps_result_in_run_text());
    let offset = start.offset + if keeps_text { result_text.len() } else { 0 };
    let caret = TextPosition::new(start.paragraph, offset);
    let caret = doc.clamp_rendered_text_position(caret).unwrap_or(start);
    *selection = Some(SelectionRange::collapsed(caret));
    Ok(())
}

/// Replaces the placeholder text in a run with a new field. Returns the id
/// the field's begin run will have once the tree is reindexed — a
/// placeholder id, so the caller finds it by the run it splits.
fn place_field(root: &mut XmlElement, code: &str) -> Result<XmlNodeId, String> {
    fn find(element: &XmlElement, path: &mut Vec<usize>) -> bool {
        for (index, child) in element.children.iter().enumerate() {
            let XmlNode::Element(child) = child else {
                continue;
            };
            path.push(index);
            if child.name == "w:r"
                && child.children.iter().any(|node| {
                    matches!(node, XmlNode::Element(t) if t.name == "w:t" && t.children.iter().any(|text| matches!(text, XmlNode::Text(text) if text.contains(PLACEHOLDER))))
                })
            {
                return true;
            }
            if find(child, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = Vec::new();
    if !find(root, &mut path) {
        return Err("the insertion point was not found".to_string());
    }
    // Give the begin run a real id now so the caller can find the field;
    // the rest get theirs when the tree is reindexed, after it.
    fn max_id(element: &XmlElement) -> u32 {
        element
            .children
            .iter()
            .filter_map(|child| match child {
                XmlNode::Element(child) => Some(max_id(child)),
                _ => None,
            })
            .fold(element.id.raw(), u32::max)
    }
    let begin_id = XmlNodeId::new(max_id(root) + 1);
    let (parent_path, index) = path.split_at(path.len() - 1);
    let parent = write::at_path_mut(root, parent_path).ok_or("insertion run lost")?;
    let XmlNode::Element(run) = parent.children.remove(index[0]) else {
        return Err("insertion run lost".to_string());
    };
    let rpr = write::run_properties(&run).cloned();
    let text: String = run
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::Element(t) if t.name == "w:t" => Some(
                t.children
                    .iter()
                    .filter_map(|text| match text {
                        XmlNode::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect();
    let (before, after) = text.split_once(PLACEHOLDER).unwrap_or((&text, ""));
    let mut runs = write::text_runs(before, rpr.as_ref());
    let mut field = write::field_runs(code, "", rpr.as_ref());

    field[0].id = begin_id;
    runs.extend(field);
    runs.extend(write::text_runs(after, rpr.as_ref()));
    parent
        .children
        .splice(index[0]..index[0], runs.into_iter().map(XmlNode::Element));
    Ok(begin_id)
}

/// The document's properties, standard and custom.
pub fn document_properties(doc: &WordDocument) -> DocumentProperties {
    unpack(doc)
        .map(|files| DocumentProperties::read(&files))
        .unwrap_or_default()
}

/// Replaces the document's properties.
pub fn set_document_properties(
    doc: &mut WordDocument,
    properties: &DocumentProperties,
) -> Result<(), String> {
    let mut files = unpack(doc)?;
    properties.write(&mut files)?;
    replace_with(doc, &files)
}

/// The document variables (`w:docVars`), in order.
pub fn document_variables(doc: &WordDocument) -> Vec<(String, String)> {
    unpack(doc)
        .map(|files| properties::read_variables(&files))
        .unwrap_or_default()
}

/// Replaces the document variables.
pub fn set_document_variables(
    doc: &mut WordDocument,
    variables: &[(String, String)],
) -> Result<(), String> {
    let mut files = unpack(doc)?;
    properties::write_variables(&mut files, variables)?;
    replace_with(doc, &files)
}

type FieldOperation =
    Box<dyn FnMut(&mut WordDocument, &mut Option<SelectionRange>) -> Result<(), String>>;

/// A field edit as one undo step: the document before it is kept whole.
pub struct FieldCommand {
    name: &'static str,
    operation: FieldOperation,
    previous: Option<(WordDocumentSnapshot, Option<SelectionRange>)>,
}

impl FieldCommand {
    pub fn new(
        name: &'static str,
        operation: impl FnMut(&mut WordDocument, &mut Option<SelectionRange>) -> Result<(), String>
            + 'static,
    ) -> Self {
        Self {
            name,
            operation: Box::new(operation),
            previous: None,
        }
    }

    /// F9: updates the fields in scope.
    pub fn update(scope: FieldScope, env: FieldEnvironment) -> Self {
        Self::new("update_fields", move |doc, selection| {
            let kept = *selection;
            update_fields(doc, scope, &env)?;
            *selection = kept.and_then(|range| clamp_range(doc, range));
            Ok(())
        })
    }

    /// ⌘F11 / ⌘⇧F11.
    pub fn lock(scope: FieldScope, locked: bool) -> Self {
        Self::new(
            if locked {
                "lock_fields"
            } else {
                "unlock_fields"
            },
            move |doc, selection| {
                let kept = *selection;
                set_fields_locked(doc, scope, locked)?;
                *selection = kept.and_then(|range| clamp_range(doc, range));
                Ok(())
            },
        )
    }

    /// ⌘⇧F9.
    pub fn unlink(scope: FieldScope) -> Self {
        Self::new("unlink_fields", move |doc, selection| {
            let kept = *selection;
            unlink_fields(doc, scope)?;
            *selection = kept.and_then(|range| clamp_range(doc, range));
            Ok(())
        })
    }

    /// 挿入 › フィールド.
    pub fn insert(code: String, env: FieldEnvironment) -> Self {
        Self::new("insert_field_code", move |doc, selection| {
            insert_field(doc, selection, &code, &env)
        })
    }

    /// フィールドの編集.
    pub fn edit(range: SelectionRange, code: String, env: FieldEnvironment) -> Self {
        Self::new("edit_field_code", move |doc, selection| {
            edit_field_code(doc, range, &code, &env)?;
            *selection = clamp_range(doc, range);
            Ok(())
        })
    }

    /// プロパティ.
    pub fn properties(properties: DocumentProperties) -> Self {
        Self::new("set_document_properties", move |doc, selection| {
            let kept = *selection;
            set_document_properties(doc, &properties)?;
            *selection = kept.and_then(|range| clamp_range(doc, range));
            Ok(())
        })
    }

    /// Document variables.
    pub fn variables(variables: Vec<(String, String)>) -> Self {
        Self::new("set_document_variables", move |doc, selection| {
            let kept = *selection;
            set_document_variables(doc, &variables)?;
            *selection = kept.and_then(|range| clamp_range(doc, range));
            Ok(())
        })
    }
}

fn clamp_range(doc: &WordDocument, range: SelectionRange) -> Option<SelectionRange> {
    let last = doc.paragraph_count().checked_sub(1)?;
    let clamp = |position: TextPosition| {
        let position = TextPosition::new(position.paragraph.min(last), position.offset);
        doc.clamp_rendered_text_position(position)
            .unwrap_or(position)
    };
    Some(SelectionRange::new(clamp(range.anchor), clamp(range.focus)))
}

impl Command for FieldCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = ctx
            .document
            .snapshot_for_replacement()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let selection = *ctx.selection;
        if let Err(error) = (self.operation)(ctx.document, ctx.selection) {
            ctx.document.restore_snapshot(previous);
            *ctx.selection = selection;
            return Err(CommandError::Custom(error));
        }
        self.previous = Some((previous, selection));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (previous, selection) = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom(format!("{} snapshot is missing", self.name)))?;
        ctx.document.restore_snapshot(previous);
        *ctx.selection = selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
