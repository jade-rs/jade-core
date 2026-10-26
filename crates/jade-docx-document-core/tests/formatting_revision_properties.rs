//! Formatting revisions must be reversible, including after ZIP/XML round trips.
//! The oracle edits a separate document with recording disabled and compares
//! direct OOXML properties per character, independent of run splitting and IDs.
use std::io::{Cursor, Read, Write};

use futures::executor::block_on;
use jade_docx_document_core::{
    commands::{
        set_paragraph_alignment::SetParagraphAlignmentCommand, CommandContext, CommandExecutor,
    },
    headless_api::{
        apply_headless_commands_to_document, apply_headless_commands_with_executor_to_document,
        execute_headless_queries_on_document,
    },
    selection::{SelectionRange, TextPosition},
    word_document::{ParagraphStory, WordDocument},
    Options,
};
use jade_docx_headless_api::{HeadlessEditRequest, HeadlessQueryRequest, HeadlessQueryResult};
use jade_docx_reader::{DocxXmlDocument, XmlElement, XmlNode};
use proptest::prelude::*;
use serde_json::{json, Value};

fn fixture() -> WordDocument {
    let source =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hello.docx"))
            .unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(source)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            bytes = br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>A&#x3042;&#x1F642;z</w:t></w:r></w:p>
<w:p><w:pPr><w:keepNext/><w:spacing w:before="120"/><w:ind w:left="240"/><w:jc w:val="right"/><w:rPr><w:b/></w:rPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:pPr><w:r><w:rPr><w:i/><w:color w:val="123456"/><w:lang w:val="ja-JP"/></w:rPr><w:t>A&#x3042;&#x1F642;z</w:t></w:r></w:p>
<w:p><w:pPr><w:bidi w:val="0"/><w:shd w:fill="FFEEDD"/><w:contextualSpacing/></w:pPr><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="28"/></w:rPr><w:t>A&#x3042;&#x1F642;z</w:t></w:r></w:p>
<w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>
</w:body></w:document>"#.to_vec();
        }
        if name == "word/styles.xml" {
            bytes = String::from_utf8(bytes).unwrap().replace("</w:styles>", r#"<w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/></w:rPr></w:style></w:styles>"#).into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&bytes).unwrap();
    }
    load(&out.finish().unwrap().into_inner())
}

fn load(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).unwrap()
}

fn reload(word: &WordDocument) -> WordDocument {
    load(&word.export_docx().unwrap())
}

fn apply(word: &mut WordDocument, commands: Vec<Value>) {
    let request: HeadlessEditRequest =
        serde_json::from_value(json!({"commands": commands})).unwrap();
    apply_headless_commands_to_document(word, &request).unwrap();
}

fn track(word: &mut WordDocument) {
    apply(
        word,
        vec![
            json!({"type": "start_change_tracking", "author": "Reviewer",
        "date": "2026-10-26T00:00:00Z"}),
        ],
    );
}

fn decide(word: &mut WordDocument, accept: bool, paragraph: Option<usize>) {
    apply(
        word,
        vec![json!({
            "type": if accept { "accept_revisions" } else { "reject_revisions" },
            "start_paragraph": paragraph, "end_paragraph": paragraph
        })],
    );
}

fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XmlNode::Element(e) if e.name == name => Some(e),
        _ => None,
    })
}

// Property order is schema-normalized by the writer. Empty property containers
// and absent ones both mean no direct formatting. Keep every attribute/value.
fn properties(element: Option<&XmlElement>) -> Value {
    let Some(element) = element else {
        return Value::Null;
    };
    let attributes: std::collections::BTreeMap<_, _> = element
        .attributes
        .iter()
        .map(|a| (a.name.clone(), a.value.clone()))
        .collect();
    let mut children: Vec<Value> = element
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::Element(e) if matches!(e.name.as_str(), "w:pPrChange" | "w:rPrChange") => None,
            XmlNode::Element(e)
                if matches!(e.name.as_str(), "w:rPr" | "w:pPr")
                    && properties(Some(e)).is_null() =>
            {
                None
            }
            XmlNode::Element(e) => Some(json!([e.name, properties(Some(e))])),
            XmlNode::Text(text) if text.trim().is_empty() => None,
            XmlNode::Text(text) | XmlNode::CData(text) | XmlNode::Comment(text) => {
                Some(json!(text))
            }
        })
        .collect();
    children.sort_by_key(Value::to_string);
    if attributes.is_empty() && children.is_empty() {
        Value::Null
    } else {
        json!([attributes, children])
    }
}

// Read the note's actual package XML independently of the editor's temporary
// node bindings. IDs from separate XML parts must never alias body node IDs.
fn paragraph_xml(word: &WordDocument, index: usize) -> XmlElement {
    if let Some(id) = word.paragraph(index).unwrap().node_id {
        return word
            .raw_document
            .as_ref()
            .unwrap()
            .element(id)
            .unwrap()
            .clone();
    }
    let (part, tag, id, ordinal) = match word.paragraph_story(index).unwrap() {
        ParagraphStory::Footnote { note_id, paragraph } => {
            ("word/footnotes.xml", "w:footnote", note_id, paragraph)
        }
        ParagraphStory::Endnote { note_id, paragraph } => {
            ("word/endnotes.xml", "w:endnote", note_id, paragraph)
        }
        other => panic!("unexpected unbound story: {other:?}"),
    };
    let bytes = word.export_docx().unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(part).unwrap().read_to_string(&mut xml).unwrap();
    let raw = DocxXmlDocument::parse(&xml).unwrap();
    let note = raw
        .root()
        .children
        .iter()
        .find_map(|node| match node {
            XmlNode::Element(e)
                if e.name == tag
                    && e.attributes
                        .iter()
                        .any(|a| a.name == "w:id" && a.value == id) =>
            {
                Some(e)
            }
            _ => None,
        })
        .unwrap();
    fn paragraphs<'a>(e: &'a XmlElement, out: &mut Vec<&'a XmlElement>) {
        if e.name == "w:p" {
            out.push(e);
            return;
        }
        for node in &e.children {
            if let XmlNode::Element(e) = node {
                paragraphs(e, out);
            }
        }
    }
    let mut found = Vec::new();
    paragraphs(note, &mut found);
    found[ordinal].clone()
}

fn note_fixture() -> WordDocument {
    let mut word = fixture();
    for (kind, paragraph) in [("insert_footnote", 0), ("insert_endnote", 2)] {
        apply(
            &mut word,
            vec![json!({"type": kind, "paragraph": paragraph, "offset": 1, "text": "Aあ🙂z"})],
        );
    }
    reload(&word)
}

fn snapshot(word: &WordDocument) -> Vec<Value> {
    let raw = word.raw_document.as_ref().unwrap();
    (0..word.paragraph_count())
        .map(|index| {
            let p = word.paragraph(index).unwrap();
            let xml = paragraph_xml(word, index);
            if word.is_note_paragraph_surface(index) {
                fn chars(e: &XmlElement, out: &mut Vec<Value>) {
                    if e.name == "w:r" {
                        let props = properties(child(e, "w:rPr"));
                        for node in &e.children {
                            if let XmlNode::Element(t) = node {
                                if t.name == "w:t" {
                                    for node in &t.children {
                                        if let XmlNode::Text(text) = node {
                                            out.extend(
                                                text.chars()
                                                    .map(|c| json!([c.to_string(), props.clone()])),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        for node in &e.children {
                            if let XmlNode::Element(e) = node {
                                chars(e, out);
                            }
                        }
                    }
                }
                let mut text = Vec::new();
                chars(&xml, &mut text);
                return json!([properties(child(&xml, "w:pPr")), text]);
            }
            let chars: Vec<_> = p
                .runs
                .iter()
                .flat_map(|r| {
                    let props = properties(
                        r.node_id
                            .and_then(|id| raw.element(id))
                            .and_then(|r| child(r, "w:rPr")),
                    );
                    r.text
                        .chars()
                        .map(move |c| json!([c.to_string(), props.clone()]))
                })
                .collect();
            json!([properties(child(&xml, "w:pPr")), chars])
        })
        .collect()
}

fn revisions(word: &WordDocument) -> Vec<jade_docx_headless_api::HeadlessRevisionSnapshot> {
    let request: HeadlessQueryRequest =
        serde_json::from_value(json!({"queries": [{"type": "revisions"}]})).unwrap();
    match execute_headless_queries_on_document(word, &request)
        .unwrap()
        .results
        .remove(0)
    {
        HeadlessQueryResult::Revisions { revisions, .. } => revisions,
        other => panic!("unexpected query result: {other:?}"),
    }
}

fn assert_revision_structure(word: &WordDocument) {
    fn visit(element: &XmlElement) {
        for (property, change) in [("w:pPr", "w:pPrChange"), ("w:rPr", "w:rPrChange")] {
            if element.name != property {
                continue;
            }
            let changes: Vec<_> = element
                .children
                .iter()
                .filter_map(|node| match node {
                    XmlNode::Element(e) if e.name == change => Some(e),
                    _ => None,
                })
                .collect();
            assert!(changes.len() <= 1, "duplicate {change}");
            for revision in changes {
                let previous = child(revision, property).expect("original properties");
                assert!(
                    child(previous, change).is_none(),
                    "nested property revision"
                );
                if property == "w:pPr" {
                    assert!(child(previous, "w:rPr").is_none());
                    assert!(child(previous, "w:sectPr").is_none());
                }
            }
        }
        for node in &element.children {
            if let XmlNode::Element(e) = node {
                visit(e);
            }
        }
    }
    for index in 0..word.paragraph_count() {
        visit(&paragraph_xml(word, index));
    }
    let mut ids = std::collections::HashSet::new();
    for r in revisions(word) {
        assert!(
            ids.insert(r.id.clone()),
            "duplicate live revision ID: {:?}; all: {:?}",
            r.id,
            revisions(word)
        );
        assert!(matches!(r.kind.as_str(), "runFormat" | "paragraphFormat"));
        assert_eq!(r.author.as_deref(), Some("Reviewer"));
        assert_eq!(r.date.as_deref(), Some("2026-10-26T00:00:00Z"));
        assert!(r.id.is_some());
    }
}

#[derive(Clone, Debug)]
struct Edit {
    paragraph: usize,
    kind: u8,
    value: u8,
    from: usize,
    to: usize,
    reload: bool,
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(
        (
            0usize..3,
            0u8..30,
            0u8..8,
            0usize..4,
            1usize..5,
            any::<bool>(),
        )
            .prop_map(|(paragraph, kind, value, a, b, reload)| Edit {
                paragraph,
                kind,
                value,
                from: a.min(b - 1),
                to: b.max(a + 1),
                reload,
            }),
        1..18,
    )
}

impl Edit {
    fn command(&self) -> Value {
        let mut command = match self.kind {
            0 => {
                json!({"type": "set_paragraph_alignment", "alignment": (["left", "center", "right", "both"][self.value as usize % 4])})
            }
            1 => {
                json!({"type": "set_paragraph_indent", "left": f32::from(self.value) * 6.0, "right": 12.0, "clear_special": false})
            }
            2 => {
                json!({"type": "set_paragraph_indent", "hanging_chars": i32::from(self.value) * 100, "clear_special": true})
            }
            3 => {
                json!({"type": "set_paragraph_metrics", "before": f32::from(self.value) * 3.0, "after": 6.0, "clear_special": false})
            }
            4 => json!({"type": "set_paragraph_direction", "bidi": self.value.is_multiple_of(2)}),
            5 => {
                json!({"type": "set_paragraph_pagination", "keep_next": self.value.is_multiple_of(2), "keep_lines": true, "widow_control": false})
            }
            6 => {
                json!({"type": "set_paragraph_shading", "color": (["ABCDEF", "FFFFFF", "123456"][self.value as usize % 3])})
            }
            7 => json!({"type": "reset_paragraph_formatting"}),
            8 => json!({"type": "set_bold", "value": self.value.is_multiple_of(2)}),
            9 => json!({"type": "set_font_size", "font_size": 8.0 + f32::from(self.value) * 2.0}),
            10 => {
                json!({"type": "set_font_family", "font_family": (["Arial", "Times New Roman"][self.value as usize % 2])})
            }
            11 => {
                json!({"type": "set_paragraph_line_spacing", "line": 1.0 + f32::from(self.value) / 2.0, "before": 3.0, "line_rule": "auto"})
            }
            12 => {
                json!({"type": "set_paragraph_style", "style_id": if self.value.is_multiple_of(2) { Some("Normal") } else { None }})
            }
            13 => {
                json!({"type": "set_paragraph_tabs", "tabs": [{"position": 12.0 + f32::from(self.value) * 6.0, "kind": "right", "leader": "dot"}]})
            }
            14 => {
                json!({"type": "set_paragraph_options", "contextual_spacing": self.value.is_multiple_of(2), "suppress_line_numbers": true, "suppress_auto_hyphens": false})
            }
            15 => {
                json!({"type": "set_paragraph_borders", "bottom": {"style": "single", "width": 1.0, "color": "123456"}})
            }
            16 => {
                json!({"type": "set_paragraph_snap_to_grid", "snap": self.value.is_multiple_of(2)})
            }
            17 => {
                json!({"type": "set_numbering", "num_id": "1", "level": u32::from(self.value % 3)})
            }
            18 => json!({"type": "apply_italic", "enabled": self.value.is_multiple_of(2)}),
            19 => {
                json!({"type": "apply_underline", "kind": (["single", "double", "wave"][self.value as usize % 3]), "enabled": self.value.is_multiple_of(2)})
            }
            20 => {
                json!({"type": "apply_strike", "double": self.value.is_multiple_of(3), "enabled": self.value.is_multiple_of(2)})
            }
            21 => json!({"type": "apply_superscript", "enabled": self.value.is_multiple_of(2)}),
            22 => json!({"type": "apply_subscript", "enabled": self.value.is_multiple_of(2)}),
            23 => json!({"type": "clear_formatting"}),
            24 => {
                json!({"type": "set_font_color", "color": (["ABCDEF", "auto", "000000"][self.value as usize % 3])})
            }
            25 => {
                json!({"type": "set_highlight_color", "color": (["yellow", "cyan", "none"][self.value as usize % 3])})
            }
            26 => {
                json!({"type": "set_character_style", "style_id": if self.value.is_multiple_of(2) { Some("Emphasis") } else { None }})
            }
            27 => {
                json!({"type": "set_run_format", "format": {"latinFont": "Arial", "eastAsiaFont": "Yu Mincho", "bold": self.value.is_multiple_of(2), "complexItalic": true, "spacing": f32::from(self.value) / 2.0, "position": -1.0, "scale": 80, "emphasis": "dot", "underline": "double", "underlineColor": "ABCDEF", "shading": "FFEEDD"}})
            }
            28 => {
                json!({"type": "set_paragraph_typography", "typography": {"kinsoku": self.value.is_multiple_of(2), "wordWrap": false, "overflowPunctuation": false, "topLinePunctuation": true, "autoSpaceDe": false, "autoSpaceDn": false}})
            }
            _ => {
                json!({"type": "set_drop_cap", "kind": if self.value.is_multiple_of(3) { None } else { Some(if self.value.is_multiple_of(2) { "drop" } else { "margin" }) }, "lines": 2 + self.value % 4})
            }
        };
        command["start_paragraph"] = json!(self.paragraph);
        command["end_paragraph"] = json!(self.paragraph);
        if (8..=10).contains(&self.kind) || (18..28).contains(&self.kind) {
            let boundaries = [0, 1, 4, 8, 9]; // UTF-8 byte offsets: A, あ, 🙂, z.
            command["start_offset"] = json!(boundaries[self.from]);
            command["end_offset"] = json!(boundaries[self.to]);
        }
        command
    }
}

// Include zero-width note references: a character-only oracle misses corruption
// of their styles and review baselines.
fn reference_properties(word: &WordDocument) -> Vec<Value> {
    fn visit(e: &XmlElement, out: &mut Vec<Value>) {
        if e.name == "w:r" {
            for node in &e.children {
                if let XmlNode::Element(reference) = node {
                    if matches!(
                        reference.name.as_str(),
                        "w:footnoteReference" | "w:endnoteReference"
                    ) {
                        out.push(json!([
                            reference.name,
                            properties(Some(reference)),
                            properties(child(e, "w:rPr"))
                        ]));
                    }
                }
            }
        }
        for node in &e.children {
            if let XmlNode::Element(e) = node {
                visit(e, out);
            }
        }
    }
    let mut out = Vec::new();
    for index in 0..word.paragraph_count() {
        visit(&paragraph_xml(word, index), &mut out);
    }
    out
}

#[test]
fn colocated_note_references_keep_styles_and_distinct_live_revision_ids() {
    let mut word = fixture();
    for kind in ["insert_footnote", "insert_endnote"] {
        apply(
            &mut word,
            vec![json!({"type": kind, "paragraph": 0, "offset": 1, "text": "注🙂"})],
        );
    }
    let original = snapshot(&word);
    let references = reference_properties(&word);
    track(&mut word);
    let start = word
        .model_to_rendered_text_position(TextPosition::new(0, 1))
        .offset;
    let end = word
        .model_to_rendered_text_position(TextPosition::new(0, 4))
        .offset;
    apply(
        &mut word,
        vec![
            json!({"type": "set_bold", "start_paragraph": 0, "end_paragraph": 0,
        "start_offset": start, "end_offset": end, "value": true}),
        ],
    );
    assert_eq!(
        reference_properties(&word),
        references,
        "leading reference styles"
    );
    assert_revision_structure(&word);
    assert_eq!(
        revisions(&word).len(),
        1,
        "only selected text has a formatting revision"
    );
    decide(&mut word, false, None);
    assert_eq!(snapshot(&word), original);
    assert_eq!(reference_properties(&word), references);
    assert_eq!(reference_properties(&reload(&word)), references);
}

#[test]
fn split_formatting_revisions_keep_unique_ids_before_and_after_reload() {
    let mut word = fixture();
    let original = snapshot(&word);
    track(&mut word);
    apply(
        &mut word,
        vec![json!({"type": "set_bold", "start_paragraph": 0,
        "end_paragraph": 0, "start_offset": 0, "end_offset": 9, "value": true})],
    );
    apply(
        &mut word,
        vec![json!({"type": "apply_italic", "start_paragraph": 0,
        "end_paragraph": 0, "start_offset": 1, "end_offset": 4, "enabled": true})],
    );
    assert_revision_structure(&word);
    assert_eq!(revisions(&word).len(), 3);
    word = reload(&word);
    track(&mut word);
    apply(
        &mut word,
        vec![json!({"type": "set_paragraph_alignment",
        "start_paragraph": 1, "end_paragraph": 1, "alignment": "center"})],
    );
    assert_revision_structure(&word);
    assert_eq!(revisions(&word).len(), 4);
    let target = revisions(&word)
        .into_iter()
        .find(|r| r.start_offset == 1)
        .unwrap();
    apply(
        &mut word,
        vec![json!({"type": "reject_revisions", "revision_targets": [{
        "paragraph": target.paragraph, "start_offset": target.start_offset,
        "end_offset": target.end_offset, "paragraph_mark": target.paragraph_mark,
        "id": target.id, "format_change": target.kind}]})],
    );
    assert_revision_structure(&word);
    assert_eq!(
        revisions(&word).len(),
        3,
        "only the selected fragment is reviewed"
    );
    assert_eq!(snapshot(&word)[0][1][1], original[0][1][1]);
    decide(&mut word, false, None);
    assert_eq!(snapshot(&word), original);
    assert_eq!(snapshot(&reload(&word)), original);
}

#[test]
fn note_reference_empty_runs_preserve_rejection_baselines() {
    let mut word = note_fixture();
    let original = snapshot(&word);
    assert_eq!(snapshot(&reload(&word)), original, "initial reload");
    track(&mut word);
    apply(
        &mut word,
        vec![
            json!({"type": "set_paragraph_alignment", "start_paragraph": 0, "end_paragraph": 0, "alignment": "left"}),
        ],
    );
    assert_eq!(snapshot(&reload(&word)), snapshot(&word), "pending reload");
    decide(&mut word, false, None);
    assert_eq!(snapshot(&word), original, "live rejection");
    assert_eq!(snapshot(&reload(&word)), original, "rejection reload");
}

#[test]
fn note_formatting_is_recorded_and_can_be_rejected() {
    let mut word = note_fixture();
    let original = snapshot(&word);
    track(&mut word);
    for paragraph in 0..word.paragraph_count() {
        let from = if word.is_note_paragraph_surface(paragraph) {
            2
        } else {
            1
        };
        let to = if word.is_note_paragraph_surface(paragraph) {
            9
        } else {
            8
        };
        let start_offset = word
            .model_to_rendered_text_position(TextPosition::new(paragraph, from))
            .offset;
        let end_offset = word
            .model_to_rendered_text_position(TextPosition::new(paragraph, to))
            .offset;
        apply(
            &mut word,
            vec![
                json!({"type": "set_paragraph_alignment", "start_paragraph": paragraph, "end_paragraph": paragraph, "alignment": "right"}),
                json!({"type": "set_bold", "start_paragraph": paragraph, "end_paragraph": paragraph, "start_offset": start_offset, "end_offset": end_offset, "value": true}),
            ],
        );
    }
    assert_revision_structure(&word);
    for paragraph in 0..word.paragraph_count() {
        assert!(revisions(&word)
            .iter()
            .any(|r| r.paragraph == paragraph && r.kind == "runFormat"));
        if word.is_note_paragraph_surface(paragraph) {
            assert!(revisions(&word)
                .iter()
                .any(|r| r.paragraph == paragraph && r.kind == "paragraphFormat"));
        }
    }
    let mut word = reload(&word);
    let pending = snapshot(&word);
    let identities = revisions(&word);
    let unique: std::collections::HashSet<_> =
        identities.iter().map(|r| r.id.clone().unwrap()).collect();
    assert_eq!(
        unique.len(),
        identities.len(),
        "revision IDs span every story: {identities:?}"
    );
    let mut accepted = reload(&word);
    decide(&mut accepted, true, None);
    assert_eq!(snapshot(&reload(&accepted)), pending);
    assert!(revisions(&reload(&accepted)).is_empty());
    decide(&mut word, false, None);
    assert_eq!(snapshot(&reload(&word)), original);
    assert!(revisions(&reload(&word)).is_empty());
}

#[test]
fn formatting_ranges_select_exact_characters_across_note_markers() {
    for paragraph in [0, 1, 4] {
        for kind in [8, 9, 10, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 30] {
            let mut word = note_fixture();
            let model_start = if word.is_note_paragraph_surface(paragraph) {
                2
            } else {
                1
            };
            let from = word
                .model_to_rendered_text_position(TextPosition::new(paragraph, model_start))
                .offset;
            let to = word
                .model_to_rendered_text_position(TextPosition::new(
                    paragraph,
                    model_start + "あ".len(),
                ))
                .offset;
            if kind == 23 {
                apply(
                    &mut word,
                    vec![
                        json!({"type": "set_bold", "start_paragraph": paragraph, "end_paragraph": paragraph, "start_offset": from, "end_offset": to, "value": true}),
                    ],
                );
            }
            let original = snapshot(&word);
            let mut command = Edit {
                paragraph,
                kind: kind.min(27),
                value: 0,
                from: 1,
                to: 2,
                reload: false,
            }
            .command();
            if kind == 30 {
                command["type"] = json!("apply_bold");
                command["enabled"] = json!(true);
                command.as_object_mut().unwrap().remove("format");
            }
            command["start_offset"] = json!(from);
            command["end_offset"] = json!(to);
            track(&mut word);
            apply(&mut word, vec![command]);
            let changed = snapshot(&reload(&word));
            assert_ne!(
                changed[paragraph][1][1], original[paragraph][1][1],
                "paragraph {paragraph}, kind {kind}"
            );
            let mut expected = original.clone();
            expected[paragraph][1][1] = changed[paragraph][1][1].clone();
            assert_eq!(
                changed, expected,
                "only あ should change: paragraph {paragraph}, kind {kind}"
            );
            decide(&mut word, false, None);
            assert_eq!(
                snapshot(&reload(&word)),
                original,
                "rejected: paragraph {paragraph}, kind {kind}"
            );
        }
    }
}

#[test]
fn styled_paragraph_join_does_not_resurrect_replaced_language() {
    for tracked in [false, true] {
        for roundtrip in [false, true] {
            let mut word = fixture();
            let original = snapshot(&word);
            if tracked {
                track(&mut word);
            }
            apply(
                &mut word,
                vec![Edit {
                    paragraph: 1,
                    kind: 12,
                    value: 0,
                    from: 0,
                    to: 4,
                    reload: false,
                }
                .command()],
            );
            // Normal replaces the direct language. Subsequent edits must not
            // resurrect its stale preserved copy, with or without a reload.
            assert!(!snapshot(&word)[1].to_string().contains("ja-JP"));
            if roundtrip {
                word = reload(&word);
                if tracked {
                    track(&mut word);
                }
            }
            apply(
                &mut word,
                vec![
                    json!({"type": "delete_text", "paragraph": 2, "offset": 0, "direction": "backward", "count": 1}),
                ],
            );
            if tracked {
                let mut rejected = reload(&word);
                decide(&mut rejected, false, None);
                assert_eq!(snapshot(&reload(&rejected)), original);
                decide(&mut word, true, None);
            }
            assert!(!snapshot(&reload(&word))[1].to_string().contains("ja-JP"));
            assert_eq!(word.paragraph_count(), 2);
        }
    }
}

#[test]
fn paragraph_style_then_italic_preserves_direct_language() {
    let mut tracked = fixture();
    let mut plain = reload(&tracked);
    let original = snapshot(&plain);
    track(&mut tracked);
    for kind in [12, 18] {
        let edit = Edit {
            paragraph: 1,
            kind,
            value: 4,
            from: 0,
            to: 1,
            reload: false,
        };
        apply(&mut tracked, vec![edit.command()]);
        apply(&mut plain, vec![edit.command()]);
        assert_eq!(snapshot(&tracked), snapshot(&plain));
    }
    decide(&mut tracked, false, None);
    assert_eq!(snapshot(&reload(&tracked)), original);
}

#[test]
fn repeated_bold_after_reload_preserves_language_and_original_properties() {
    let mut word = fixture();
    let original = snapshot(&word);
    track(&mut word);
    let command = json!({"type": "set_bold", "value": true, "start_paragraph": 1,
        "end_paragraph": 1, "start_offset": 0, "end_offset": 1});
    apply(&mut word, vec![command.clone()]);
    let edited = snapshot(&word);
    word = reload(&word);
    track(&mut word);
    apply(&mut word, vec![command]);
    assert_eq!(snapshot(&word)[1], edited[1]);
    decide(&mut word, false, None);
    assert_eq!(snapshot(&reload(&word))[1], original[1]);
}

#[test]
fn rejecting_font_change_restores_absent_direct_font_properties() {
    let mut word = fixture();
    let original = snapshot(&word);
    track(&mut word);
    apply(
        &mut word,
        vec![
            json!({"type": "set_font_family", "font_family": "Times New Roman",
        "start_paragraph": 0, "end_paragraph": 0, "start_offset": 0, "end_offset": 1}),
        ],
    );
    word = reload(&word);
    decide(&mut word, false, None);
    assert_eq!(snapshot(&reload(&word))[0], original[0]);
}

#[test]
fn same_font_size_then_disjoint_bold_can_be_rejected() {
    let mut word = fixture();
    let original = snapshot(&word);
    track(&mut word);
    for edit in [
        Edit {
            paragraph: 2,
            kind: 9,
            value: 3,
            from: 1,
            to: 3,
            reload: false,
        },
        Edit {
            paragraph: 2,
            kind: 8,
            value: 0,
            from: 0,
            to: 1,
            reload: false,
        },
    ] {
        apply(&mut word, vec![edit.command()]);
    }
    let mut word = reload(&word);
    decide(&mut word, false, Some(2));
    assert_eq!(snapshot(&reload(&word)), original);
}

#[test]
fn formatting_commands_redo_keep_revision_identity() {
    for kind in [5, 27, 28, 29] {
        let mut word = fixture();
        track(&mut word);
        let original = snapshot(&reload(&word));
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let command = Edit {
            paragraph: 0,
            kind,
            value: 4,
            from: 0,
            to: 1,
            reload: false,
        }
        .command();
        let request: HeadlessEditRequest =
            serde_json::from_value(json!({"commands": [command]})).unwrap();
        selection = apply_headless_commands_with_executor_to_document(
            &mut word,
            &mut executor,
            selection,
            &request,
        )
        .unwrap();
        let edited = snapshot(&reload(&word));
        let expected = revisions(&reload(&word));
        assert!(!expected.is_empty(), "missing revision for kind {kind}");
        for _ in 0..2 {
            assert!(executor
                .undo(&mut CommandContext::new(&mut word, &mut selection))
                .unwrap());
            assert_eq!(snapshot(&reload(&word)), original);
            assert!(revisions(&word).is_empty());
            assert!(executor
                .redo(&mut CommandContext::new(&mut word, &mut selection))
                .unwrap());
            assert_eq!(snapshot(&reload(&word)), edited);
            assert_eq!(
                revisions(&reload(&word)),
                expected,
                "revision identity changed for kind {kind}"
            );
        }
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        assert_eq!(snapshot(&reload(&rejected)), original);
    }
}

#[test]
fn every_run_format_rejects_to_original_direct_properties() {
    for kind in (8..=10).chain(18..28) {
        for paragraph in 0..3 {
            let mut word = fixture();
            let original = snapshot(&word);
            track(&mut word);
            apply(
                &mut word,
                vec![Edit {
                    paragraph,
                    kind,
                    value: 4,
                    from: 1,
                    to: 3,
                    reload: false,
                }
                .command()],
            );
            if snapshot(&word) != original {
                assert!(
                    revisions(&word).iter().any(|r| r.kind == "runFormat"),
                    "missing rPrChange for kind {kind}, paragraph {paragraph}"
                );
            }
            assert_revision_structure(&word);
            let mut word = reload(&word);
            decide(&mut word, false, None);
            assert_eq!(
                snapshot(&reload(&word)),
                original,
                "reject kind {kind}, paragraph {paragraph}"
            );
        }
    }
}

#[test]
fn every_paragraph_format_keeps_original_ppr_and_unrelated_mark_and_section() {
    for kind in (0..18)
        .chain([28, 29])
        .filter(|kind| !(8..=10).contains(kind))
    {
        let mut word = fixture();
        let original = snapshot(&word);
        track(&mut word);
        for value in if kind == 16 { [5, 4] } else { [4, 5] } {
            apply(
                &mut word,
                vec![Edit {
                    paragraph: 1,
                    kind,
                    value,
                    from: 0,
                    to: 4,
                    reload: false,
                }
                .command()],
            );
            word = reload(&word);
            track(&mut word);
            assert_revision_structure(&word);
            let raw = word.raw_document.as_ref().unwrap();
            let ppr = child(
                raw.element(word.paragraph(1).unwrap().node_id.unwrap())
                    .unwrap(),
                "w:pPr",
            )
            .unwrap();
            assert!(
                child(ppr, "w:rPr").is_some(),
                "mark formatting lost for kind {kind}"
            );
            assert!(
                child(ppr, "w:sectPr").is_some(),
                "section lost for kind {kind}"
            );
            let revision = child(ppr, "w:pPrChange")
                .unwrap_or_else(|| panic!("pPrChange missing for kind {kind}"));
            let mut previous = child(revision, "w:pPr").unwrap().clone();
            // Those children are outside CT_PPrBase and remain live.
            previous.children.extend(ppr.children.iter().filter(|n| matches!(n, XmlNode::Element(e) if matches!(e.name.as_str(), "w:rPr" | "w:sectPr"))).cloned());
            assert_eq!(
                properties(Some(&previous)),
                original[1][0],
                "original pPr for kind {kind}"
            );
        }
        decide(&mut word, false, None);
        assert_eq!(snapshot(&reload(&word)), original, "reject kind {kind}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(128),
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn search_matches_with_note_references_are_editable_and_reversible(
        prefix in prop::collection::vec(prop::sample::select(vec!['a', '日', '🙂']), 0..12),
        suffix in prop::collection::vec(prop::sample::select(vec!['z', '本', '🙃']), 0..12),
        placements in prop::collection::vec((any::<usize>(), any::<bool>()), 1..7),
        reopen in any::<bool>(),
    ) {
        let prefix: String = prefix.into_iter().collect();
        let suffix: String = suffix.into_iter().collect();
        let needle = "Abあ🙂Z";
        let text = format!("{prefix}{needle}{suffix}");
        let mut word = fixture();
        apply(&mut word, vec![json!({"type": "replace_range", "start_paragraph": 0,
            "end_paragraph": 0, "start_offset": 0, "end_offset": "Aあ🙂z".len(), "text": text})]);
        let boundaries: Vec<_> = text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect();
        for (seed, footnote) in placements {
            let model = if seed % 3 == 0 { prefix.len() } else { boundaries[seed % boundaries.len()] };
            let offset = word.model_to_rendered_text_position(TextPosition::new(0, model)).offset;
            apply(&mut word, vec![json!({"type": if footnote {"insert_footnote"} else {"insert_endnote"},
                "paragraph": 0, "offset": offset, "text": "注🙂"})]);
        }
        if reopen { word = reload(&word); }
        let original = snapshot(&word);
        let references = reference_properties(&word);
        let request: HeadlessQueryRequest = serde_json::from_value(json!({"queries": [{"type": "search_text", "query": needle}]})).unwrap();
        let response = execute_headless_queries_on_document(&word, &request).unwrap();
        let HeadlessQueryResult::SearchText {matches, ..} = &response.results[0] else {panic!("search result")};
        prop_assert_eq!(matches.len(), 1);
        let hit = &matches[0];
        prop_assert_eq!(word.rendered_to_model_text_position(TextPosition::new(0, hit.start_offset)).offset, prefix.len());
        prop_assert_eq!(word.rendered_to_model_text_position(TextPosition::new(0, hit.end_offset)).offset, prefix.len() + needle.len());
        track(&mut word);
        apply(&mut word, vec![json!({"type": "set_bold", "start_paragraph": hit.paragraph,
            "end_paragraph": hit.paragraph, "start_offset": hit.start_offset, "end_offset": hit.end_offset, "value": true})]);
        assert_revision_structure(&word);
        let changed = snapshot(&reload(&word));
        // Only the matched characters change, even with markers inside the match.
        let from = prefix.chars().count();
        let to = from + needle.chars().count();
        prop_assert_ne!(&changed[0], &original[0]);
        prop_assert_eq!(changed[0][1].as_array().unwrap().len(), original[0][1].as_array().unwrap().len());
        for (index, (before, after)) in original[0][1].as_array().unwrap().iter().zip(changed[0][1].as_array().unwrap()).enumerate() {
            if index < from || index >= to {
                prop_assert_eq!(before, after);
            } else {
                prop_assert_eq!(&before[0], &after[0]);
                prop_assert!(after[1][1].as_array().unwrap().iter().any(|property| property[0] == "w:b"));
            }
        }
        word = reload(&word);
        decide(&mut word, false, None);
        prop_assert_eq!(snapshot(&reload(&word)), original);
        prop_assert_eq!(reference_properties(&word), references.clone());
        prop_assert_eq!(reference_properties(&reload(&word)), references);
    }

    #[test]
    fn note_and_body_formatting_have_independent_review_baselines(edits in edits(), target in 0usize..3, accept in any::<bool>()) {
        let mut word = note_fixture();
        let surfaces = [0, (0..word.paragraph_count()).find(|&p| matches!(word.paragraph_story(p), Some(ParagraphStory::Footnote { .. }))).unwrap(),
            (0..word.paragraph_count()).find(|&p| matches!(word.paragraph_story(p), Some(ParagraphStory::Endnote { .. }))).unwrap()];
        let original = snapshot(&word);
        let mut plain = reload(&word);
        track(&mut word);
        for mut edit in edits {
            edit.paragraph = if edit.kind == 29 { surfaces[0] } else { surfaces[edit.paragraph] };
            let mut command = edit.command();
            if command.get("start_offset").is_some() {
                let text: String = word.paragraph(edit.paragraph).unwrap().runs.iter().map(|run| run.text.as_str()).collect();
                let boundaries: Vec<_> = text.char_indices().map(|(offset, _)| offset).chain(std::iter::once(text.len())).collect();
                let base = boundaries.len() - 5;
                command["start_offset"] = json!(word.model_to_rendered_text_position(TextPosition::new(edit.paragraph, boundaries[base + edit.from])).offset);
                command["end_offset"] = json!(word.model_to_rendered_text_position(TextPosition::new(edit.paragraph, boundaries[base + edit.to])).offset);
            }
            apply(&mut word, vec![command.clone()]);
            apply(&mut plain, vec![command]);
            if edit.reload { word = reload(&word); plain = reload(&plain); track(&mut word); }
            prop_assert_eq!(snapshot(&word), snapshot(&plain), "after {:?}", edit);
            assert_revision_structure(&word);
        }
        let mut accepted = reload(&word);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original.clone());
        prop_assert!(revisions(&reload(&rejected)).is_empty());
        let mut expected = snapshot(&word);
        if !accept { expected[surfaces[target]] = original[surfaces[target]].clone(); }
        decide(&mut word, accept, Some(surfaces[target]));
        prop_assert_eq!(snapshot(&reload(&word)), expected);
        prop_assert!(revisions(&reload(&word)).iter().all(|r| r.paragraph != surfaces[target]));
    }

    #[test]
    fn mixed_formatting_accepts_like_untracked_edits_and_rejects_to_original(edits in edits()) {
        let mut tracked = fixture();
        let mut plain = reload(&tracked);
        let original = snapshot(&plain);
        track(&mut tracked);
        for edit in &edits {
            apply(&mut tracked, vec![edit.command()]);
            apply(&mut plain, vec![edit.command()]);
            if edit.reload {
                tracked = reload(&tracked);
                plain = reload(&plain);
                track(&mut tracked);
            }
            prop_assert_eq!(snapshot(&tracked), snapshot(&plain), "live formatting after {:?}", edit);
            assert_revision_structure(&tracked);
        }
        let mut accepted = reload(&tracked);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&tracked);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original);
        prop_assert!(revisions(&reload(&rejected)).is_empty());
    }

    #[test]
    fn paragraph_decisions_are_isolated(edits in edits(), target in 0usize..3, accept in any::<bool>()) {
        let mut word = fixture();
        let original = snapshot(&word);
        track(&mut word);
        for edit in &edits { apply(&mut word, vec![edit.command()]); }
        let mut word = reload(&word);
        let edited = snapshot(&word);
        decide(&mut word, accept, Some(target));
        let word = reload(&word);
        let mut expected = edited;
        if !accept { expected[target] = original[target].clone(); }
        prop_assert_eq!(snapshot(&word), expected);
        prop_assert!(revisions(&word).iter().all(|r| r.paragraph != target));
    }

    #[test]
    fn exact_revision_target_changes_only_its_own_properties(
        edits in edits(), target in any::<usize>(), accept in any::<bool>(), roundtrip in any::<bool>()
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        track(&mut word);
        for edit in &edits { apply(&mut word, vec![edit.command()]); }
        if roundtrip { word = reload(&word); }
        assert_revision_structure(&word);
        let before = revisions(&word);
        if before.is_empty() { return Ok(()); }
        let target = target % before.len();
        let selected = &before[target];
        let mut expected = snapshot(&word);
        if !accept {
            if selected.kind == "paragraphFormat" {
                expected[selected.paragraph][0] = original[selected.paragraph][0].clone();
            } else {
                let boundaries = [0, 1, 4, 8, 9];
                for (index, offset) in boundaries[..4].iter().enumerate() {
                    if selected.start_offset <= *offset && *offset < selected.end_offset {
                        expected[selected.paragraph][1][index][1] = original[selected.paragraph][1][index][1].clone();
                    }
                }
            }
        }
        apply(&mut word, vec![json!({
            "type": if accept { "accept_revisions" } else { "reject_revisions" },
            "revision_targets": [{"paragraph": selected.paragraph,
                "start_offset": selected.start_offset, "end_offset": selected.end_offset,
                "paragraph_mark": selected.paragraph_mark, "id": selected.id,
                "format_change": selected.kind}]
        })]);
        let word = reload(&word);
        prop_assert_eq!(snapshot(&word), expected);
        let mut remaining = serde_json::to_value(&before).unwrap().as_array().unwrap().clone();
        remaining.remove(target);
        prop_assert_eq!(serde_json::to_value(revisions(&word)).unwrap(), json!(remaining));
    }

    #[test]
    fn repeated_review_epochs_use_the_latest_accepted_baseline(
        epochs in prop::collection::vec((edits(), any::<bool>()), 2..5)
    ) {
        let mut word = fixture();
        for (edits, accept) in epochs {
            let original = snapshot(&word);
            let mut plain = reload(&word);
            track(&mut word);
            for edit in edits {
                apply(&mut word, vec![edit.command()]);
                apply(&mut plain, vec![edit.command()]);
                if edit.reload {
                    word = reload(&word);
                    track(&mut word);
                    plain = reload(&plain);
                }
                prop_assert_eq!(snapshot(&word), snapshot(&plain));
            }
            decide(&mut word, accept, None);
            word = reload(&word);
            prop_assert_eq!(snapshot(&word), if accept { snapshot(&reload(&plain)) } else { original });
            prop_assert!(revisions(&word).is_empty());
        }
    }

    #[test]
    fn text_and_formatting_revisions_review_together(
        paragraph in 0usize..3, before in 0u8..30, after in 0u8..30,
        value in 0u8..8, insert in any::<bool>(), boundary in 0usize..5, roundtrip in any::<bool>()
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        let mut plain = reload(&word);
        track(&mut word);
        let whole_format = |kind, length| {
            let mut command = Edit { paragraph, kind, value, from: 0, to: 4, reload: false }.command();
            if command.get("end_offset").is_some() { command["end_offset"] = json!(length); }
            command
        };
        for doc in [&mut word, &mut plain] { apply(doc, vec![whole_format(before, 9)]); }
        let boundaries = [0, 1, 4, 8, 9];
        let deleted = boundary % 4;
        let text = if insert {
            json!({"type": "insert_text", "paragraph": paragraph, "offset": boundaries[boundary], "text": "漢"})
        } else {
            json!({"type": "delete_text", "paragraph": paragraph, "offset": boundaries[deleted + 1], "direction": "backward", "count": 1})
        };
        for doc in [&mut word, &mut plain] { apply(doc, vec![text.clone()]); }
        if roundtrip { word = reload(&word); track(&mut word); plain = reload(&plain); }
        apply(&mut word, vec![whole_format(after, if insert { 12 } else { 9 })]);
        apply(&mut plain, vec![whole_format(after, if insert { 12 } else { 9 - (boundaries[deleted + 1] - boundaries[deleted]) })]);
        let mut accepted = reload(&word);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original);
        prop_assert!(revisions(&reload(&rejected)).is_empty());
    }

    #[test]
    fn paragraph_breaks_keep_reviewed_formatting_reversible(
        paragraph in 0usize..3, before in 0u8..30, after in 0u8..30,
        value in 0u8..8, boundary in 0usize..5, roundtrip in any::<bool>()
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        let mut plain = reload(&word);
        track(&mut word);
        let command = Edit { paragraph, kind: before, value, from: 0, to: 4, reload: false }.command();
        for doc in [&mut word, &mut plain] { apply(doc, vec![command.clone()]); }
        let command = json!({"type": "insert_paragraph_break", "paragraph": paragraph,
            "offset": ([0, 1, 4, 8, 9][boundary])});
        for doc in [&mut word, &mut plain] { apply(doc, vec![command.clone()]); }
        if roundtrip { word = reload(&word); track(&mut word); plain = reload(&plain); }
        for index in paragraph..=paragraph+1 {
            for doc in [&mut word, &mut plain] {
                let length: usize = doc.paragraph(index).unwrap().runs.iter().map(|r| r.text.len()).sum();
                let mut command = Edit { paragraph: index, kind: after, value, from: 0, to: 4, reload: false }.command();
                // Drop caps require a nonempty paragraph.
                if after == 29 && length == 0 { continue; }
                if command.get("end_offset").is_some() {
                    if length == 0 { continue; }
                    command["end_offset"] = json!(length);
                }
                apply(doc, vec![command]);
            }
        }
        let mut accepted = reload(&word);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original);
        prop_assert!(revisions(&reload(&rejected)).is_empty());
    }

    #[test]
    fn deleted_paragraph_breaks_review_with_both_paragraphs_formatting(
        paragraph in 0usize..2, first in 0u8..30, second in 0u8..30,
        value in 0u8..8, roundtrip in any::<bool>()
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        let mut plain = reload(&word);
        track(&mut word);
        for (index, kind) in [(paragraph, first), (paragraph + 1, second)] {
            let command = Edit { paragraph: index, kind, value, from: 0, to: 4, reload: false }.command();
            for doc in [&mut word, &mut plain] { apply(doc, vec![command.clone()]); }
        }
        let command = json!({"type": "delete_text", "paragraph": paragraph + 1,
            "offset": 0, "direction": "backward", "count": 1});
        for doc in [&mut word, &mut plain] { apply(doc, vec![command.clone()]); }
        if roundtrip { word = reload(&word); }
        let mut accepted = reload(&word);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original);
        prop_assert!(revisions(&reload(&rejected)).is_empty());
    }

    #[test]
    fn mixed_formatting_groups_undo_redo_and_abort(edits in edits(), tracked in any::<bool>(), abort in any::<bool>()) {
        let mut word = fixture();
        if tracked { track(&mut word); }
        let original = snapshot(&reload(&word));
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        executor.begin_group();
        for edit in edits {
            executor.begin_group();
            let request: HeadlessEditRequest = serde_json::from_value(json!({"commands": [edit.command()]})).unwrap();
            selection = apply_headless_commands_with_executor_to_document(&mut word, &mut executor, selection, &request).unwrap();
            executor.end_group();
            assert_revision_structure(&word);
        }
        let edited = snapshot(&reload(&word));
        let edited_revisions = revisions(&reload(&word));
        if abort {
            executor.abort_group(&mut CommandContext::new(&mut word, &mut selection));
            prop_assert_eq!(executor.undo_depth(), 0);
        } else {
            executor.end_group();
            prop_assert!(executor.undo_depth() <= 1);
            if executor.can_undo() {
                prop_assert!(executor.undo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
                prop_assert_eq!(snapshot(&reload(&word)), original.clone());
                prop_assert!(revisions(&word).is_empty());
                prop_assert!(executor.redo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
                prop_assert_eq!(snapshot(&reload(&word)), edited);
                prop_assert_eq!(revisions(&reload(&word)), edited_revisions);
                prop_assert!(executor.undo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
            }
        }
        prop_assert_eq!(snapshot(&reload(&word)), original);
        prop_assert!(revisions(&word).is_empty());
    }

    #[test]
    fn nested_groups_restore_formatting_revisions_and_selection(
        edits in prop::collection::vec((0usize..3, 0usize..4), 1..16),
        tracked in any::<bool>(), abort in any::<bool>()
    ) {
        let mut word = fixture();
        if tracked { track(&mut word); }
        let original = snapshot(&word);
        let mut selection = Some(SelectionRange::new(TextPosition::new(2, 8), TextPosition::new(0, 1)));
        let original_selection = selection;
        let mut executor = CommandExecutor::new();
        executor.begin_group();
        for (paragraph, alignment) in edits {
            executor.begin_group();
            executor.execute(Box::new(SetParagraphAlignmentCommand::new(paragraph, paragraph,
                ["left", "center", "right", "both"][alignment])), &mut CommandContext::new(&mut word, &mut selection)).unwrap();
            executor.end_group();
        }
        prop_assert_eq!(selection, original_selection);
        if abort {
            executor.abort_group(&mut CommandContext::new(&mut word, &mut selection));
            prop_assert_eq!(snapshot(&reload(&word)), original);
            prop_assert_eq!(executor.undo_depth(), 0);
            prop_assert!(revisions(&word).is_empty());
        } else {
            executor.end_group();
            let edited = snapshot(&reload(&word));
            let revision_count = revisions(&word).len();
            prop_assert!(executor.undo_depth() <= 1);
            if executor.can_undo() {
                prop_assert!(executor.undo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
                prop_assert_eq!(snapshot(&reload(&word)), original);
                prop_assert!(revisions(&word).is_empty());
                prop_assert!(executor.redo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
                prop_assert_eq!(snapshot(&reload(&word)), edited);
                prop_assert_eq!(revisions(&word).len(), revision_count);
            }
            prop_assert_eq!(selection, original_selection);
        }
    }

    #[test]
    fn cross_paragraph_ranges_preserve_review_baselines(
        edits in prop::collection::vec((0u8..30, 0u8..8, 0usize..2, 1usize..3, 0usize..5, 0usize..5, any::<bool>()), 1..12)
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        let mut plain = reload(&word);
        track(&mut word);
        for (kind, value, a, b, from, to, roundtrip) in edits {
            let mut command = Edit { paragraph: a.min(b), kind, value, from: 0, to: 4, reload: false }.command();
            command["end_paragraph"] = json!(b.max(a + 1));
            if command.get("start_offset").is_some() {
                command["start_offset"] = json!([0, 1, 4, 8, 9][from]);
                command["end_offset"] = json!([0, 1, 4, 8, 9][to]);
            }
            for doc in [&mut word, &mut plain] { apply(doc, vec![command.clone()]); }
            if roundtrip { word = reload(&word); plain = reload(&plain); track(&mut word); }
            prop_assert_eq!(snapshot(&word), snapshot(&plain));
            assert_revision_structure(&word);
        }
        let mut accepted = reload(&word);
        decide(&mut accepted, true, None);
        prop_assert_eq!(snapshot(&reload(&accepted)), snapshot(&reload(&plain)));
        prop_assert!(revisions(&reload(&accepted)).is_empty());
        let mut rejected = reload(&word);
        decide(&mut rejected, false, None);
        prop_assert_eq!(snapshot(&reload(&rejected)), original);
        prop_assert!(revisions(&reload(&rejected)).is_empty());
    }

    #[test]
    fn review_decisions_undo_redo_restore_pending_revision_identity(
        edits in edits(), target in prop::option::of(0usize..3), accept in any::<bool>(), roundtrip in any::<bool>()
    ) {
        let mut word = fixture();
        let original = snapshot(&word);
        track(&mut word);
        for edit in edits { apply(&mut word, vec![edit.command()]); }
        if roundtrip { word = reload(&word); track(&mut word); }
        let pending = snapshot(&reload(&word));
        let pending_revisions = revisions(&reload(&word));
        let mut executor = CommandExecutor::new();
        let mut selection = Some(SelectionRange::new(TextPosition::new(2, 8), TextPosition::new(0, 1)));
        let original_selection = selection;
        let request: HeadlessEditRequest = serde_json::from_value(json!({"commands": [{
            "type": if accept { "accept_revisions" } else { "reject_revisions" },
            "start_paragraph": target, "end_paragraph": target
        }]})).unwrap();
        selection = apply_headless_commands_with_executor_to_document(&mut word, &mut executor, selection, &request).unwrap();
        let reviewed = snapshot(&reload(&word));
        let reviewed_revisions = revisions(&reload(&word));
        for _ in 0..2 {
            if !executor.can_undo() { break; }
            prop_assert!(executor.undo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
            prop_assert_eq!(snapshot(&reload(&word)), pending.clone());
            prop_assert_eq!(revisions(&reload(&word)), pending_revisions.clone());
            prop_assert_eq!(selection, original_selection);
            prop_assert!(executor.redo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
            prop_assert_eq!(snapshot(&reload(&word)), reviewed.clone());
            prop_assert_eq!(revisions(&reload(&word)), reviewed_revisions.clone());
        }
        if executor.can_undo() {
            prop_assert!(executor.undo(&mut CommandContext::new(&mut word, &mut selection)).unwrap());
            decide(&mut word, false, None);
            prop_assert_eq!(snapshot(&reload(&word)), original);
            prop_assert!(revisions(&reload(&word)).is_empty());
        }
    }

}
