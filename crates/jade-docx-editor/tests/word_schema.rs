//! Edits must leave a package Word opens without a repair prompt.
//!
//! `tests/word-schema` validates every fixture after every scenario with the
//! Open XML SDK; these are the regressions it found, pinned to one fixture
//! each so `cargo test` catches them without .NET.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::{json, Value};

fn load(fixture: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(fixture);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{fixture}: {error}"));
    futures::executor::block_on(WordDocument::load(&bytes, &Default::default())).expect("load")
}

/// The body paragraphs, as surface indexes (text box paragraphs are surfaces
/// too, so body paragraph `n` is not surface `n`).
fn body(document: &WordDocument) -> Vec<usize> {
    (0..document.paragraph_count())
        .filter(|&index| document.is_body_paragraph_surface(index))
        .collect()
}

fn text_len(document: &WordDocument, paragraph: usize) -> usize {
    document
        .paragraph(paragraph)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.len()).sum())
        .unwrap_or(0)
}

fn apply(document: &mut WordDocument, commands: Value) {
    let request: HeadlessEditRequest =
        serde_json::from_value(json!({ "commands": commands })).expect("request");
    apply_headless_commands_to_document(document, &request).expect("apply");
}

fn part(document: &WordDocument, name: &str) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("{name} missing"))
        .read_to_string(&mut xml)
        .expect("read part");
    xml
}

/// The root element's start tag.
fn root_tag(xml: &str) -> &str {
    let start = xml.find("?>").map_or(0, |end| end + 2);
    let start = start + xml[start..].find('<').expect("root");
    &xml[start..start + xml[start..].find('>').expect("root end")]
}

/// The child element names of every `<parent>…</parent>`, in order.
fn children_of(xml: &str, parent: &str) -> Vec<Vec<String>> {
    let open = format!("<{parent}");
    let close = format!("</{parent}>");
    let mut all = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let tag_end = after.find('>').expect("start tag end");
        // `<w:rPrChange` is not `<w:rPr`, and `<w:rPr/>` has no children.
        if !matches!(after.as_bytes()[0], b' ' | b'>') || after[..tag_end].ends_with('/') {
            rest = after;
            continue;
        }
        let inner = &after[tag_end + 1..];
        let end = inner.find(&close).expect("closed");
        let mut names = Vec::new();
        let mut depth = 0usize;
        let mut cursor = &inner[..end];
        while let Some(tag) = cursor.find('<') {
            cursor = &cursor[tag + 1..];
            let closing = cursor.starts_with('/');
            let name: String = cursor
                .trim_start_matches('/')
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
                .collect();
            let tag_end = cursor.find('>').expect("tag end");
            let self_closing = cursor[..tag_end].ends_with('/');
            if closing {
                depth -= 1;
            } else {
                if depth == 0 {
                    names.push(name);
                }
                if !self_closing {
                    depth += 1;
                }
            }
            cursor = &cursor[tag_end..];
        }
        all.push(names);
        rest = &inner[end..];
    }
    all
}

fn position(names: &[String], name: &str) -> Option<usize> {
    names.iter().position(|candidate| candidate == name)
}

#[test]
fn joining_paragraphs_moves_a_floating_shape_as_authored() {
    // Joining two paragraphs moves the second one's drawing into the first.
    // It has to move as authored XML: the model carries no custom geometry,
    // so rebuilding it turned a concave text box into a rectangle and reset
    // its `wp:docPr`.
    let mut document = load("floating-through-concave.docx");
    let body = body(&document);
    let middle = text_len(&document, body[0]) / 2;
    apply(
        &mut document,
        json!([{
            "type": "replace_range",
            "start_paragraph": body[0], "start_offset": middle,
            "end_paragraph": body[1], "end_offset": 0,
            "text": "",
        }]),
    );
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains("<a:custGeom>"), "custom geometry lost");
    assert!(xml.contains(r#"<wp:docPr id="915""#), "docPr reset");
    assert!(!xml.contains("<a:prstGeom"), "shape rebuilt from the model");
}

#[test]
fn tracked_changes_declare_the_utc_date_namespace() {
    let mut document = load("align-center.docx");
    let paragraph = body(&document)[0];
    apply(
        &mut document,
        json!([
            {
                "type": "start_change_tracking", "author": "Reviewer", "initials": "R",
                "date": "2026-10-05T09:00:00Z", "date_utc": "2026-10-05T09:00:00Z",
            },
            { "type": "insert_text", "paragraph": paragraph, "offset": 1, "text": "挿入" },
            { "type": "insert_paragraph_break", "paragraph": paragraph, "offset": 1 },
        ]),
    );
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains("w16du:dateUtc="), "UTC date not recorded");
    assert!(
        !xml.contains("w:dateUtc="),
        "UTC date written in the main namespace"
    );
    let root = root_tag(&xml);
    assert!(
        root.contains(
            r#"xmlns:w16du="http://schemas.microsoft.com/office/word/2023/wordml/word16du""#
        ),
        "w16du undeclared: {root}"
    );
    let ignorable = root
        .split("mc:Ignorable=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("mc:Ignorable");
    assert!(ignorable.split(' ').any(|prefix| prefix == "w16du"));
}

#[test]
fn a_watermark_declares_the_relationship_namespace_it_uses() {
    let mut document = load("note-styles-creation.docx");
    apply(
        &mut document,
        json!([{ "type": "set_watermark", "clear": false, "text": "社外秘" }]),
    );
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains("r:id="));
    assert!(root_tag(&xml).contains("xmlns:r="), "r undeclared");
}

#[test]
fn list_galleries_write_integer_abstract_numbering_ids() {
    for preset in ["bulletHierarchy", "decimalHierarchy"] {
        let mut document = load("numbering-by-style.docx");
        let paragraph = body(&document)[0];
        apply(
            &mut document,
            json!([{
                "type": "apply_multilevel_list",
                "start_paragraph": paragraph, "end_paragraph": paragraph,
                "preset": preset, "level": 0,
            }]),
        );
        let xml = part(&document, "word/numbering.xml");
        for value in xml
            .split("w:abstractNumId=\"")
            .skip(1)
            .chain(xml.split("<w:abstractNumId w:val=\"").skip(1))
        {
            let id = value.split('"').next().unwrap_or_default();
            assert!(id.parse::<u32>().is_ok(), "{preset}: abstractNumId {id:?}");
        }
        // CT_Numbering lists every abstractNum before the first num.
        let last_abstract = xml.rfind("<w:abstractNum ").expect("abstractNum");
        let first_num = xml.find("<w:num ").expect("num");
        assert!(
            last_abstract < first_num,
            "{preset}: num before abstractNum"
        );
        for level in children_of(&xml, "w:lvl") {
            if let (Some(suffix), Some(text)) =
                (position(&level, "w:suff"), position(&level, "w:lvlText"))
            {
                assert!(suffix < text, "{preset}: w:suff after w:lvlText");
            }
        }
    }
}

#[test]
fn character_formatting_writes_run_properties_in_schema_order() {
    let mut document = load("2col.docx");
    let paragraph = body(&document)[0];
    let length = text_len(&document, paragraph);
    let middle = length / 2;
    apply(
        &mut document,
        json!([
            { "type": "apply_bold", "start_paragraph": paragraph, "start_offset": 0,
              "end_paragraph": paragraph, "end_offset": middle },
            { "type": "set_font_color", "start_paragraph": paragraph, "start_offset": 0,
              "end_paragraph": paragraph, "end_offset": length, "color": "C00000" },
            { "type": "set_font_size", "start_paragraph": paragraph, "start_offset": middle,
              "end_paragraph": paragraph, "end_offset": length, "font_size": 14 },
        ]),
    );
    let xml = part(&document, "word/document.xml");
    let order = [
        "w:rStyle", "w:rFonts", "w:b", "w:bCs", "w:color", "w:sz", "w:szCs", "w:lang",
    ];
    for properties in children_of(&xml, "w:rPr") {
        let ranks: Vec<usize> = properties
            .iter()
            .filter_map(|name| order.iter().position(|known| known == name))
            .collect();
        assert!(
            ranks.windows(2).all(|pair| pair[0] <= pair[1]),
            "out of order: {properties:?}"
        );
    }
}

#[test]
fn splitting_a_paragraph_keeps_revision_ids_unique() {
    // The paragraph mark of the source carries a tracked deletion; splitting
    // the paragraph used to copy it, with its id, onto the new mark.
    let mut document = load("numbering-del2.docx");
    let paragraph = body(&document)[0];
    let middle = text_len(&document, paragraph) / 2;
    apply(
        &mut document,
        json!([{ "type": "insert_paragraph_break", "paragraph": paragraph, "offset": middle }]),
    );
    let xml = part(&document, "word/document.xml");
    let mut ids = Vec::new();
    for element in ["<w:ins ", "<w:del ", "<w:pPrChange ", "<w:rPrChange "] {
        for tag in xml.split(element).skip(1) {
            let attributes = &tag[..tag.find('>').unwrap_or(tag.len())];
            if let Some(id) = attributes
                .split("w:id=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
            {
                ids.push(id.to_string());
            }
        }
    }
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "duplicate revision ids");
}

#[test]
fn edit_protection_is_written_where_settings_expects_it() {
    let mut document = load("2col.docx");
    apply(
        &mut document,
        json!([{
            "type": "set_document_protection",
            "protection": { "edit": "readOnly", "enforced": true },
        }]),
    );
    let xml = part(&document, "word/settings.xml");
    let settings = children_of(&xml, "w:settings").remove(0);
    let protection = position(&settings, "w:documentProtection").expect("protection");
    for earlier in ["w:view", "w:zoom", "w:mirrorMargins", "w:trackRevisions"] {
        if let Some(index) = position(&settings, earlier) {
            assert!(index < protection, "{earlier} after w:documentProtection");
        }
    }
    for later in [
        "w:defaultTabStop",
        "w:characterSpacingControl",
        "w:compat",
        "w:rsids",
    ] {
        if let Some(index) = position(&settings, later) {
            assert!(protection < index, "{later} before w:documentProtection");
        }
    }
}

#[test]
fn a_theme_created_for_a_document_without_one_has_a_full_format_scheme() {
    let mut document = load("nda.docx");
    apply(
        &mut document,
        json!([{
            "type": "set_document_theme",
            "theme": {
                "name": "Jade", "majorFont": "Georgia", "minorFont": "Calibri",
                "eastAsiaFont": "游明朝", "dark2": "1F3864", "light2": "E7E6E6",
                "accent1": "4472C4", "accent2": "ED7D31", "accent3": "A5A5A5",
                "accent4": "FFC000", "accent5": "5B9BD5", "accent6": "70AD47",
            },
        }]),
    );
    let xml = part(&document, "word/theme/theme1.xml");
    // CT_StyleMatrix: at least three of each style.
    for (list, item) in [
        ("a:fillStyleLst", "<a:solidFill>"),
        ("a:lnStyleLst", "<a:ln "),
        ("a:effectStyleLst", "<a:effectStyle>"),
    ] {
        assert!(
            xml.matches(item).count() >= 3,
            "{list} has fewer than three styles"
        );
    }
    assert!(!xml.contains("<a:fillStyleLst/>"), "empty fill style list");
}

#[test]
fn table_edges_keep_the_logical_names_a_table_uses() {
    // table-bidi-visual.docx names its sides `w:start`/`w:end`; the schema
    // pairs them with `w:left`/`w:right` as alternatives, so a side written
    // under both names makes Word repair the file.
    let mut document = load("table-bidi-visual.docx");
    let edge = json!({ "style": "single", "width": 0.5, "color": "#000000" });
    apply(
        &mut document,
        json!([
            {
                "type": "set_table_borders", "table_order_index": 0,
                "top": edge, "bottom": edge, "left": edge, "right": edge,
            },
            {
                "type": "set_table_cell_margins", "table_order_index": 0,
                "top": 1, "bottom": 1, "left": 5, "right": 5,
            },
        ]),
    );
    let xml = part(&document, "word/document.xml");
    for container in ["w:tblBorders", "w:tblCellMar"] {
        for sides in children_of(&xml, container) {
            for (logical, physical) in [("w:start", "w:left"), ("w:end", "w:right")] {
                assert!(
                    !(position(&sides, logical).is_some() && position(&sides, physical).is_some()),
                    "{container} names a side twice: {sides:?}"
                );
            }
        }
    }
}

#[test]
fn paragraph_format_revisions_exclude_and_preserve_paragraph_mark_and_section_properties() {
    use jade_docx_reader::{XmlElement, XmlNode};
    fn child<'a>(element: &'a XmlElement, name: &str) -> &'a XmlElement {
        element
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == name => Some(element),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {name}"))
    }
    let mut document = load("math.docx");
    let paragraph = body(&document)[0];
    let id = document.paragraph(paragraph).unwrap().node_id.unwrap();
    // Paragraph mark properties are authored by this fixture. Include section
    // properties too: neither is part of CT_PPrGeneral inside pPrChange.
    let raw = document.raw_document.as_mut().unwrap();
    let section = raw.create_element("w:sectPr", vec![], vec![]);
    let parent = raw.element_mut(id).unwrap();
    let props = parent
        .children
        .iter_mut()
        .find_map(|node| match node {
            XmlNode::Element(element) if element.name == "w:pPr" => Some(element),
            _ => None,
        })
        .unwrap();
    props.children.push(XmlNode::Element(section));
    raw.rebuild_index();
    let original = child(raw.element(id).unwrap(), "w:pPr").clone();
    let mark = child(&original, "w:rPr").clone();
    let section = child(&original, "w:sectPr").clone();
    apply(
        &mut document,
        json!([
            { "type": "start_change_tracking", "author": "Reviewer" },
            { "type": "set_paragraph_indent", "start_paragraph": paragraph,
              "end_paragraph": paragraph, "left": 24.0 }
        ]),
    );
    let props = child(
        document.raw_document.as_ref().unwrap().element(id).unwrap(),
        "w:pPr",
    );
    let previous = child(child(props, "w:pPrChange"), "w:pPr");
    assert!(!previous.children.iter().any(|node| matches!(node,
        XmlNode::Element(element) if matches!(element.name.as_str(), "w:rPr" | "w:sectPr"))));
    assert_eq!(format!("{:?}", child(props, "w:rPr")), format!("{mark:?}"));
    assert_eq!(
        format!("{:?}", child(props, "w:sectPr")),
        format!("{section:?}")
    );
    apply(&mut document, json!([{ "type": "reject_revisions" }]));
    let restored = child(
        document.raw_document.as_ref().unwrap().element(id).unwrap(),
        "w:pPr",
    );
    assert_eq!(
        format!("{:?}", child(restored, "w:rPr")),
        format!("{mark:?}")
    );
    assert_eq!(
        format!("{:?}", child(restored, "w:sectPr")),
        format!("{section:?}")
    );
    assert!(!part(&document, "word/document.xml").contains("w:pPrChange"));
    assert_eq!(
        format!("{:?}", child(restored, "w:jc").attributes),
        format!("{:?}", child(&original, "w:jc").attributes)
    );
}
