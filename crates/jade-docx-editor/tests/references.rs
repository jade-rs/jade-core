//! 参考資料 in the engine: notes deleted with their reference marks, and
//! the commands the reference dialogs drive.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::CommandExecutor;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessEditRequest;
use serde_json::{json, Value};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn hello() -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    load_bytes(&std::fs::read(path).unwrap())
}

fn apply(document: &mut WordDocument, executor: &mut CommandExecutor, commands: Value) {
    let request: HeadlessEditRequest =
        serde_json::from_value(json!({ "commands": commands })).unwrap();
    apply_headless_commands_with_executor_to_document(document, executor, None, &request).unwrap();
}

#[allow(dead_code)]
fn part(document: &WordDocument, name: &str) -> String {
    let exported = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
    let mut xml = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

fn body_paragraph(document: &WordDocument) -> usize {
    (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .unwrap()
}

/// Where each reference mark of a paragraph is drawn.
fn marks(document: &WordDocument, paragraph: usize) -> Vec<(String, usize)> {
    let mut rendered = 0;
    document
        .reference_marker_spans(paragraph)
        .into_iter()
        .map(|span| {
            let at = span.model_offset + rendered;
            rendered += span.rendered_len;
            (span.note_id, at)
        })
        .collect()
}

#[test]
fn a_selection_over_a_reference_mark_deletes_the_note() {
    let mut document = hello();
    let mut executor = CommandExecutor::new();
    let paragraph = body_paragraph(&document);
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "insert_footnote", "paragraph": paragraph, "offset": 2, "text": "kept" }]),
    );
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "insert_footnote", "paragraph": paragraph, "offset": 1, "text": "gone" }]),
    );
    let before = document.footnote_infos().len();
    let (_, at) = marks(&document, paragraph)[0].clone();
    apply(
        &mut document,
        &mut executor,
        json!([{ "type": "replace_range", "start_paragraph": paragraph, "start_offset": at,
                 "end_paragraph": paragraph, "end_offset": at + 1, "text": "" }]),
    );
    let notes: Vec<String> = document
        .footnote_infos()
        .into_iter()
        .map(|n| n.text)
        .collect();
    assert_eq!(notes.len(), before - 1);
    assert!(notes.iter().all(|text| text != "gone"), "{notes:?}");
    assert!(notes.iter().any(|text| text == "kept"));
    assert_eq!(marks(&document, paragraph).len(), 1);

    let mut selection = None;
    let mut context =
        jade_docx_editor::commands::CommandContext::new(&mut document, &mut selection);
    executor.undo(&mut context).unwrap();
    assert_eq!(document.footnote_infos().len(), before);
}

fn fixture(name: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    load_bytes(&std::fs::read(path).unwrap())
}

fn text(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default()
}

/// What the pages show of a paragraph: a field read from a file shows its
/// saved result.
fn shown(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .map(|paragraph| {
            paragraph
                .runs
                .iter()
                .map(|run| match &run.field {
                    Some(field) if run.text.is_empty() => field
                        .resolved_value
                        .clone()
                        .or_else(|| field.cached_text.clone())
                        .unwrap_or_default(),
                    _ => run.text.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn reload(document: &WordDocument) -> WordDocument {
    load_bytes(&document.export_docx().unwrap())
}

#[test]
fn footnotes_can_sit_right_under_the_text() {
    let mut document = fixture("footnote.docx");
    assert!(!document.footnote_beneath_text(None));
    assert!(document.set_footnote_beneath_text(true, None).unwrap());
    assert!(document.footnote_beneath_text(None));
    assert!(part(&document, "word/document.xml").contains(r#"<w:pos w:val="beneathText"/>"#));
    assert!(reload(&document).footnote_beneath_text(None));
    assert!(document.set_footnote_beneath_text(false, None).unwrap());
    assert!(part(&document, "word/document.xml").contains(r#"<w:pos w:val="pageBottom"/>"#));
}

#[test]
fn a_bookmark_goes_around_the_selected_text() {
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut document = hello();
    let paragraph = body_paragraph(&document);
    let before = text(&document, paragraph);
    assert!(before.len() > 4, "{before}");
    document
        .create_bookmark_at_range(
            SelectionRange::new(
                TextPosition::new(paragraph, 1),
                TextPosition::new(paragraph, 3),
            ),
            "Middle",
        )
        .unwrap();
    assert_eq!(text(&document, paragraph), before, "the text stays");
    assert_eq!(
        document.bookmark_position("Middle"),
        Some(TextPosition::new(paragraph, 1))
    );
    assert_eq!(
        document.bookmark_end_position("Middle"),
        Some(TextPosition::new(paragraph, 3))
    );
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains(r#"w:name="Middle""#), "{xml}");
    let reread = reload(&document);
    assert_eq!(
        reread.bookmark_position("Middle"),
        Some(TextPosition::new(paragraph, 1))
    );
    assert_eq!(
        reread.bookmark_end_position("Middle"),
        Some(TextPosition::new(paragraph, 3))
    );
    assert_eq!(text(&reread, paragraph), before);
    // A caret makes a point bookmark; a name is used once.
    document
        .create_bookmark_at_range(
            SelectionRange::collapsed(TextPosition::new(paragraph, 2)),
            "Point",
        )
        .unwrap();
    assert!(document
        .create_bookmark_at_range(
            SelectionRange::collapsed(TextPosition::new(paragraph, 2)),
            "Point",
        )
        .is_err());
    document.delete_bookmark("Point").unwrap();
    assert_eq!(document.bookmark_position("Point"), None);
}

#[test]
fn an_inline_field_is_written_with_its_result() {
    use jade_docx_layout::selection::TextPosition;
    let mut document = hello();
    let paragraph = body_paragraph(&document);
    let before = text(&document, paragraph);
    document
        .insert_inline_field(
            TextPosition::new(paragraph, 0),
            r#"CITATION Smi20 \l 1041"#,
            "(Smith, 2020)",
        )
        .unwrap();
    assert!(text(&document, paragraph).starts_with("(Smith, 2020)"));
    let fields = document.inline_fields("citation");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].text, "(Smith, 2020)");
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains("CITATION Smi20"), "{xml}");
    document
        .set_inline_field(fields[0].paragraph, fields[0].run, None, "[1]")
        .unwrap();
    assert!(text(&document, paragraph).starts_with("[1]"));
    let reread = reload(&document);
    assert!(shown(&reread, paragraph).starts_with("[1]"));
    assert!(shown(&reread, paragraph).ends_with(&before));
    // A hidden marker shows nothing.
    document
        .insert_inline_field(TextPosition::new(paragraph, 0), r#"TA \l "Case" \c 1"#, "")
        .unwrap();
    assert!(text(&document, paragraph).starts_with("[1]"));
    assert_eq!(document.inline_fields("TA").len(), 1);
    assert!(
        part(&document, "word/document.xml").contains(r#"TA \l &quot;Case&quot; \c 1"#)
            || part(&document, "word/document.xml").contains(r#"TA \l "Case" \c 1"#)
    );
}

#[test]
fn a_field_block_is_inserted_listed_and_replaced() {
    use jade_docx_editor::word_document::FieldBlockLine;
    let mut document = hello();
    let paragraph = body_paragraph(&document);
    let line = |text: &str, page: &str| FieldBlockLine {
        text: text.to_string(),
        page: Some(page.to_string()),
        ..FieldBlockLine::default()
    };
    document
        .insert_field_block(
            paragraph,
            r#"INDEX \c "2" \z "1041""#,
            &[line("あ", "1"), line("い", "2")],
        )
        .unwrap();
    let blocks = document.field_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].name(), "INDEX");
    assert_eq!(blocks[0].end - blocks[0].start, 1);
    assert_eq!(text(&document, blocks[0].start), "あ\t1");
    let count = document.paragraph_count();
    document
        .replace_field_block(
            &blocks[0],
            r#"INDEX \c "1""#,
            &[line("あ", "1"), line("い", "2"), line("う", "3")],
        )
        .unwrap();
    assert_eq!(document.paragraph_count(), count + 1);
    let blocks = document.field_blocks();
    assert_eq!(blocks[0].instruction, r#"INDEX \c "1""#);
    assert_eq!(text(&document, blocks[0].end), "う\t3");
    let reread = reload(&document);
    assert_eq!(reread.field_blocks().len(), 1);
    assert!(shown(&reread, reread.field_blocks()[0].end).contains('う'));
}

/// The toc fixture's headings, by paragraph.
fn headings(document: &WordDocument) -> Vec<(usize, String)> {
    document
        .navigation_headings()
        .into_iter()
        .map(|heading| (heading.paragraph_index, heading.text))
        .collect()
}

#[test]
fn a_section_moves_with_its_paragraphs() {
    let mut document = fixture("toc.docx");
    let before = headings(&document);
    assert!(before.len() >= 2, "{before:?}");
    let first = before[0].0;
    let second = before[1].0;
    let body = |document: &WordDocument, surface: usize| {
        (0..surface + 1)
            .filter(|&index| document.is_top_level_body_paragraph_surface(index))
            .count()
            - 1
    };
    // The second heading's section, up before the first.
    let start = body(&document, second);
    let end = before
        .get(2)
        .map_or(document.paragraph_count(), |heading| heading.0);
    let end = body(&document, end.saturating_sub(1)).max(start);
    document
        .move_body_elements(start, end, body(&document, first))
        .unwrap();
    let after = headings(&document);
    assert_eq!(after[0].1, before[1].1);
    assert_eq!(after[1].1, before[0].1);
    let reread = reload(&document);
    assert_eq!(headings(&reread)[0].1, before[1].1);
}

#[test]
fn updating_a_table_of_contents_keeps_its_leader() {
    use jade_docx_editor::document::TocOptions;
    let mut document = fixture("toc.docx");
    let existing = document.tables_of_contents().len();
    document
        .insert_table_of_contents_with_leader(
            0,
            &TocOptions {
                outline_min: 1,
                outline_max: 3,
                hyperlinks: true,
                ..TocOptions::default()
            },
            "hyphen",
        )
        .unwrap();
    let contents = document.tables_of_contents();
    assert_eq!(contents.len(), existing + 1);
    assert_eq!(document.toc_leader(&contents[0]), "hyphen");
    document.update_table_of_contents_at(0).unwrap();
    let contents = document.tables_of_contents();
    assert_eq!(contents.len(), existing + 1);
    assert_eq!(document.toc_leader(&contents[0]), "hyphen");
    assert!(part(&document, "word/document.xml").contains(r#"w:leader="hyphen""#));

    // Page numbers only: the entries' text stays, the numbers change.
    let names: Vec<String> = (contents[0].start..=contents[0].end)
        .filter_map(|index| document.paragraph(index))
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| match run.field.as_ref().map(|field| &field.kind) {
            Some(jade_docx_editor::document::RunFieldKind::PageRef { name, .. }) => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect();
    assert!(!names.is_empty());
    let pages = names
        .iter()
        .map(|name| (name.clone(), "7".to_string()))
        .collect();
    let changed = document.refresh_toc_page_numbers(0, &pages).unwrap();
    assert_eq!(changed, names.len());
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains(">7</w:t>"), "{xml}");
}

#[test]
fn a_manual_table_of_contents_is_plain_text() {
    let mut document = hello();
    let paragraph = body_paragraph(&document);
    let before = document.field_blocks().len();
    document
        .insert_manual_table_of_contents(
            paragraph,
            "目次",
            &[
                (
                    1,
                    "章のタイトル (レベル 1) を入力してください".to_string(),
                    "1".to_string(),
                ),
                (
                    2,
                    "章のタイトル (レベル 2) を入力してください".to_string(),
                    "2".to_string(),
                ),
            ],
            "dot",
        )
        .unwrap();
    assert_eq!(document.field_blocks().len(), before, "no field");
    assert_eq!(text(&document, paragraph + 1), "目次");
    assert!(text(&document, paragraph + 2).ends_with("\t1"));
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains(r#"<w:pStyle w:val="TOC2"/>"#), "{xml}");
}

#[test]
fn a_caption_numbers_by_chapter_above_its_item() {
    use jade_docx_editor::document::SeqNumberFormat;
    use jade_docx_editor::word_document::CaptionOptions;
    let mut document = fixture("toc.docx");
    let heading = headings(&document)[0].0;
    let anchor = heading + 1;
    let anchor_text = text(&document, anchor);
    document
        .insert_caption_with(
            anchor,
            &CaptionOptions {
                label: "図".to_string(),
                description: "配置".to_string(),
                above: true,
                exclude_label: false,
                number_format: SeqNumberFormat::Arabic,
                chapter: Some((1, "-".to_string(), "1".to_string())),
            },
        )
        .unwrap();
    assert_eq!(
        text(&document, anchor + 1),
        anchor_text,
        "the item moved down"
    );
    let caption = text(&document, anchor);
    assert!(caption.starts_with("図 1-"), "{caption}");
    assert!(caption.ends_with("配置"), "{caption}");
    let xml = part(&document, "word/document.xml");
    assert!(xml.contains(r#"STYLEREF 1 \s"#), "{xml}");
    assert!(xml.contains(r#"SEQ 図 \s 1 \* ARABIC"#), "{xml}");
}

#[test]
fn the_bibliography_style_is_switched_and_sources_removed() {
    use jade_docx_editor::word_document::CitationSourceInfo;
    let mut document = hello();
    assert_eq!(document.bibliography_style(), None);
    for tag in ["Smi20", "Doe19"] {
        document
            .add_citation_source_entry(CitationSourceInfo {
                tag: tag.to_string(),
                source_type: "Book".to_string(),
                author: "Smith, John".to_string(),
                title: "On Things".to_string(),
                year: "2020".to_string(),
                publisher: None,
                url: None,
            })
            .unwrap();
    }
    assert_eq!(document.bibliography_style().as_deref(), Some("APA"));
    document
        .set_bibliography_style("MLA", "\\MLASeventhEditionOfficeOnline.xsl")
        .unwrap();
    assert_eq!(document.bibliography_style().as_deref(), Some("MLA"));
    document.remove_citation_source("Smi20").unwrap();
    let tags: Vec<String> = document
        .citation_sources()
        .into_iter()
        .map(|source| source.tag)
        .collect();
    assert_eq!(tags, ["Doe19"]);
    let reread = reload(&document);
    assert_eq!(reread.bibliography_style().as_deref(), Some("MLA"));
    assert_eq!(reread.citation_sources().len(), 1);
}

#[test]
fn a_footnote_separator_is_emptied_and_drawn_again() {
    let mut document = fixture("footnote.docx");
    assert!(document.note_separator_shown(true, false));
    assert!(document
        .set_note_separator_shown(true, false, false)
        .unwrap());
    assert!(!document.note_separator_shown(true, false));
    assert!(
        document.note_separator_shown(true, true),
        "the other one stays"
    );
    let xml = part(&document, "word/footnotes.xml");
    assert!(!xml.contains("<w:separator/>"), "{xml}");
    assert!(xml.contains("<w:continuationSeparator/>"));
    assert!(!reload(&document).note_separator_shown(true, false));
    assert!(document
        .set_note_separator_shown(true, false, true)
        .unwrap());
    assert!(part(&document, "word/footnotes.xml").contains("<w:separator/>"));
    assert!(reload(&document).note_separator_shown(true, false));
}
