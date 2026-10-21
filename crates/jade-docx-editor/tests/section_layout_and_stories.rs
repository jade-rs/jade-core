//! Word's ページ設定 dialog over one section, several or all of them, and
//! headers and footers edited as stories of their own: what the dialog
//! writes is what the pages lay out with and what the saved file says, and
//! a story edited as its own document comes back into its part.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::headless_api::{
    apply_headless_commands_with_selection_to_document, execute_headless_queries_on_document,
};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessDocGrid, HeadlessEditRequest, HeadlessLineNumbering,
    HeadlessPageNumbering, HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult,
    HeadlessSectionColumn, HeadlessSectionColumns, HeadlessSectionLayoutPatch,
    HeadlessSectionScope,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn load(fixture: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(fixture);
    load_bytes(&std::fs::read(&path).unwrap_or_else(|error| panic!("{fixture}: {error}")))
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

fn caret_in_section(document: &WordDocument, section: usize) -> SelectionRange {
    let paragraph = (0..document.paragraph_count())
        .find(|&index| document.section_index_for_paragraph(index) == Some(section))
        .expect("a paragraph in the section");
    SelectionRange::collapsed(TextPosition::new(paragraph, 0))
}

fn apply(
    document: &mut WordDocument,
    selection: Option<SelectionRange>,
    commands: Vec<HeadlessCommand>,
) {
    apply_headless_commands_with_selection_to_document(
        document,
        selection,
        &HeadlessEditRequest { commands },
    )
    .expect("commands apply");
}

#[test]
fn the_dialog_writes_one_section_and_leaves_the_other() {
    let mut document = load("header-footer-sections.docx");
    assert_eq!(document.section_count(), 2);
    let before_first = document.section_layout(Some(0));
    let patch = HeadlessSectionLayoutPatch {
        margin_left: Some(90.0),
        gutter: Some(18.0),
        vertical_alignment: Some("center".to_string()),
        line_numbering: Some(HeadlessLineNumbering {
            enabled: true,
            start_at: 5,
            count_by: 2,
            distance: Some(12.0),
            restart: "newSection".to_string(),
        }),
        doc_grid: Some(HeadlessDocGrid {
            grid_type: "linesAndChars".to_string(),
            line_pitch: Some(18.0),
            char_pitch: Some(10.5),
        }),
        columns: Some(HeadlessSectionColumns {
            count: 2,
            space: 20.0,
            equal_width: false,
            separator: true,
            columns: vec![
                HeadlessSectionColumn {
                    width: 150.0,
                    space: 20.0,
                },
                HeadlessSectionColumn {
                    width: 245.0,
                    space: 0.0,
                },
            ],
        }),
        text_direction: Some("tbRl".to_string()),
        page_numbering: Some(HeadlessPageNumbering {
            format: "lowerRoman".to_string(),
            start: Some(3),
            chapter_style: Some(1),
            chapter_separator: Some("emDash".to_string()),
        }),
        ..Default::default()
    };
    let caret = caret_in_section(&document, 1);
    apply(
        &mut document,
        Some(caret),
        vec![HeadlessCommand::SetSectionLayout {
            scope: HeadlessSectionScope::Caret,
            patch,
        }],
    );

    let second = document.section_layout(Some(1));
    assert_eq!(second.margin_left, 90.0);
    assert_eq!(second.gutter, 18.0);
    assert_eq!(second.vertical_alignment, "center");
    assert_eq!(second.line_numbering.start_at, 5);
    assert_eq!(second.line_numbering.count_by, 2);
    assert_eq!(second.line_numbering.restart, "newSection");
    assert_eq!(second.doc_grid.grid_type, "linesAndChars");
    assert_eq!(second.doc_grid.line_pitch, Some(18.0));
    assert!((second.doc_grid.char_pitch.unwrap() - 10.5).abs() < 0.01);
    assert!(!second.columns.equal_width);
    assert_eq!(second.columns.columns.len(), 2);
    assert!(second.columns.separator);
    assert_eq!(second.text_direction, "tbRl");
    assert_eq!(second.page_numbering.format, "lowerRoman");
    assert_eq!(second.page_numbering.chapter_style, Some(1));
    assert_eq!(
        document.section_layout(Some(0)),
        before_first,
        "the first section is left as it was"
    );

    // What the dialog wrote is what Word reads back.
    let bytes = document.export_docx().expect("export");
    let xml = part(&bytes, "word/document.xml").unwrap();
    for needle in [
        r#"<w:vAlign w:val="center"/>"#,
        r#"<w:lnNumType w:countBy="2" w:start="4" w:distance="240" w:restart="newSection"/>"#,
        r#"<w:docGrid w:type="linesAndChars" w:linePitch="360""#,
        r#"<w:col w:w="3000" w:space="400"/>"#,
        r#"<w:textDirection w:val="tbRl"/>"#,
        r#"w:chapStyle="1" w:chapSep="emDash""#,
    ] {
        assert!(xml.contains(needle), "{needle} missing from {xml}");
    }
    let reopened = load_bytes(&bytes);
    assert_eq!(reopened.section_layout(Some(1)), second);
}

#[test]
fn whole_document_and_document_switches_apply_once() {
    let mut document = load("header.docx");
    apply(
        &mut document,
        None,
        vec![HeadlessCommand::SetSectionLayout {
            scope: HeadlessSectionScope::All,
            patch: HeadlessSectionLayoutPatch {
                margin_top: Some(100.0),
                mirror_margins: Some(true),
                gutter_at_top: Some(true),
                print_two_on_one: Some(true),
                different_odd_even: Some(true),
                ..Default::default()
            },
        }],
    );
    for section in 0..document.section_count() {
        let layout = document.section_layout(Some(section));
        assert_eq!(layout.margin_top, 100.0);
        assert!(layout.mirror_margins && layout.gutter_at_top && layout.print_two_on_one);
        assert!(layout.different_odd_even);
    }
    let bytes = document.export_docx().unwrap();
    let settings = part(&bytes, "word/settings.xml").unwrap();
    for switch in [
        "w:mirrorMargins",
        "w:gutterAtTop",
        "w:printTwoOnOne",
        "w:evenAndOddHeaders",
    ] {
        assert!(
            settings.contains(switch),
            "{switch} missing from {settings}"
        );
    }
    let reopened = load_bytes(&bytes).section_layout(None);
    assert!(reopened.print_two_on_one && reopened.mirror_margins);
}

#[test]
fn the_layout_query_answers_for_a_section() {
    let document = load("header-footer-sections.docx");
    let response = execute_headless_queries_on_document(
        &document,
        &HeadlessQueryRequest {
            queries: vec![HeadlessQuery::SectionLayout { section: Some(0) }],
        },
    )
    .unwrap();
    let HeadlessQueryResult::SectionLayout { layout } = &response.results[0] else {
        panic!("a section layout");
    };
    assert_eq!(layout.section, 0);
    assert_eq!(layout.section_count, 2);
    assert!(layout.different_first_page);
    assert!((layout.header_distance - 35.45).abs() < 0.01);
}

#[test]
fn a_section_unlinks_and_links_its_header() {
    let mut document = load("header-footer-sections.docx");
    // Section 2 has its own default header; its footer follows section 1's.
    assert!(!document
        .header_footer_linked(1, "header", "default")
        .unwrap());
    assert!(document
        .header_footer_linked(1, "footer", "default")
        .unwrap());

    apply(
        &mut document,
        None,
        vec![HeadlessCommand::SetHeaderFooterLink {
            section: 1,
            role: "footer".to_string(),
            variant: "default".to_string(),
            linked: false,
        }],
    );
    assert!(!document
        .header_footer_linked(1, "footer", "default")
        .unwrap());
    let bytes = document.export_docx().unwrap();
    let xml = part(&bytes, "word/document.xml").unwrap();
    assert_eq!(
        xml.matches("<w:footerReference").count(),
        2,
        "section 2 has a footer of its own"
    );

    apply(
        &mut document,
        None,
        vec![HeadlessCommand::SetHeaderFooterLink {
            section: 1,
            role: "header".to_string(),
            variant: "default".to_string(),
            linked: true,
        }],
    );
    assert!(document
        .header_footer_linked(1, "header", "default")
        .unwrap());
    let stories = document.section_header_footers();
    assert_eq!(
        stories[1].header, stories[0].header,
        "section 2 shows section 1's header"
    );
}

#[test]
fn a_story_round_trips_through_a_document_of_its_own() {
    let mut document = load("header.docx");
    let story = document.story_docx(0, "header", "default", 1).unwrap();
    let story_document = load_bytes(&story);
    let text: String = story_document
        .paragraph(0)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default();
    assert!(
        text.contains("Hello"),
        "the header's paragraph is the story's body: {text:?}"
    );
    let layout = story_document.section_layout(None);
    assert!(
        (layout.margin_top - 35.45).abs() < 0.01,
        "the header sits its distance down"
    );

    // Unchanged, it changes nothing.
    assert!(!document
        .replace_story_from_docx(0, "header", "default", &story)
        .unwrap());

    // Edited — a second paragraph — it comes back into the part.
    let mut edited = load_bytes(&story);
    apply(
        &mut edited,
        Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
        vec![HeadlessCommand::InsertParagraphBreak {
            paragraph: 0,
            offset: 0,
            count: 1,
        }],
    );
    let edited = edited.export_docx().unwrap();
    assert!(document
        .replace_story_from_docx(0, "header", "default", &edited)
        .unwrap());
    let bytes = document.export_docx().unwrap();
    let header = part(&bytes, "word/header1.xml").unwrap();
    assert!(header.starts_with("<?xml"));
    assert!(header.contains("<w:hdr"));
    assert_eq!(
        header.matches("<w:p>").count() + header.matches("<w:p ").count(),
        2
    );
    assert!(header.contains("Hello"));
    assert!(!header.contains("w:sectPr"));
}

#[test]
fn a_section_without_a_footer_gets_one_from_its_story() {
    let mut document = load("header.docx");
    // header.docx has a footer; a first-page footer it does not.
    let story = document.story_docx(0, "footer", "first", 1).unwrap();
    let mut edited = load_bytes(&story);
    apply(
        &mut edited,
        Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
        vec![HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text: "表紙".to_string(),
        }],
    );
    let edited = edited.export_docx().unwrap();
    assert!(document
        .replace_story_from_docx(0, "footer", "first", &edited)
        .unwrap());
    let bytes = document.export_docx().unwrap();
    let xml = part(&bytes, "word/document.xml").unwrap();
    assert!(xml.contains(r#"w:type="first""#), "{xml}");
}
