//! Word's フォント dialog applied as `set_run_format`: what it writes, that
//! the written package reads back to the same formatting, and that undo
//! takes it all back.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::set_run_format::{run_format_of, SetRunFormatCommand};
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessEastAsianLayout, HeadlessEditRequest, HeadlessRunFormat, HeadlessThemeColor,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use serde_json::json;

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn load(fixture: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(fixture);
    load_bytes(&std::fs::read(path).expect("fixture"))
}

fn document_xml(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document part")
        .read_to_string(&mut xml)
        .expect("read");
    xml
}

/// The first body paragraph with at least `chars` characters of text.
fn paragraph_with_text(document: &WordDocument, chars: usize) -> usize {
    (0..document.paragraph_count())
        .find(|&index| {
            document.is_body_paragraph_surface(index)
                && document.paragraph(index).is_some_and(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.chars().count())
                        .sum::<usize>()
                        >= chars
                })
        })
        .expect("a paragraph with text")
}

fn format_at(document: &WordDocument, paragraph: usize, offset: usize) -> HeadlessRunFormat {
    let paragraph = document.paragraph(paragraph).expect("paragraph");
    let mut start = 0;
    for run in &paragraph.runs {
        let end = start + run.text.len();
        if offset < end && !run.text.is_empty() {
            return run
                .properties
                .as_ref()
                .map(run_format_of)
                .unwrap_or_default();
        }
        start = end;
    }
    HeadlessRunFormat::default()
}

fn full_format() -> HeadlessRunFormat {
    HeadlessRunFormat {
        latin_font: Some("Arial".into()),
        east_asia_font: Some("游明朝".into()),
        font_size: Some(14.0),
        bold: Some(true),
        underline: Some("double".into()),
        underline_color: Some("FF0000".into()),
        double_strike: Some(true),
        color: Some("4472C4".into()),
        theme_color: Some(HeadlessThemeColor {
            name: "accent1".into(),
            tint: None,
            shade: None,
        }),
        small_caps: Some(true),
        hidden: Some(true),
        emphasis: Some("dot".into()),
        shading: Some("FFFF00".into()),
        border: Some(true),
        scale: Some(150.0),
        spacing: Some(2.0),
        position: Some(3.0),
        kerning: Some(10.5),
        ligatures: Some("standard".into()),
        east_asian_layout: Some(HeadlessEastAsianLayout {
            vert: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn the_font_dialog_writes_every_property_and_reads_back() {
    let mut document = load("hello.docx");
    let paragraph = paragraph_with_text(&document, 4);
    let request: HeadlessEditRequest = serde_json::from_value(json!({
        "commands": [{
            "type": "set_run_format",
            "start_paragraph": paragraph, "start_offset": 0,
            "end_paragraph": paragraph, "end_offset": 3,
            "format": serde_json::to_value(full_format()).unwrap(),
        }]
    }))
    .expect("request");
    apply_headless_commands_to_document(&mut document, &request).expect("apply");

    let xml = document_xml(&document);
    for expected in [
        r#"w:ascii="Arial""#,
        r#"w:eastAsia="游明朝""#,
        "<w:b/>",
        r#"<w:u w:val="double" w:color="FF0000"/>"#,
        "<w:dstrike/>",
        r#"w:themeColor="accent1""#,
        "<w:smallCaps/>",
        "<w:vanish/>",
        r#"<w:em w:val="dot"/>"#,
        r#"w:fill="FFFF00""#,
        r#"<w:bdr w:val="single""#,
        r#"<w:w w:val="150"/>"#,
        r#"<w:spacing w:val="40"/>"#,
        r#"<w:position w:val="6"/>"#,
        r#"<w:kern w:val="21"/>"#,
        r#"<w14:ligatures w14:val="standard"/>"#,
        r#"w:vert="1""#,
    ] {
        assert!(xml.contains(expected), "missing {expected} in {xml}");
    }

    // What was written reads back as what was set.
    let reread = load_bytes(&document.export_docx().unwrap());
    let format = format_at(&reread, paragraph, 0);
    assert_eq!(format.latin_font.as_deref(), Some("Arial"));
    assert_eq!(format.east_asia_font.as_deref(), Some("游明朝"));
    assert_eq!(format.underline.as_deref(), Some("double"));
    assert_eq!(format.double_strike, Some(true));
    assert_eq!(format.small_caps, Some(true));
    assert_eq!(format.hidden, Some(true));
    assert_eq!(format.emphasis.as_deref(), Some("dot"));
    assert_eq!(format.shading.as_deref(), Some("FFFF00"));
    assert_eq!(format.border, Some(true));
    assert_eq!(format.scale, Some(150.0));
    assert_eq!(format.spacing, Some(2.0));
    assert_eq!(format.position, Some(3.0));
    assert_eq!(format.color.as_deref(), Some("4472C4"));
    assert_eq!(
        format.theme_color.map(|theme| theme.name).as_deref(),
        Some("accent1")
    );
    assert!(format.east_asian_layout.is_some_and(|layout| layout.vert));
    // The text after the range keeps its own formatting.
    assert_ne!(format_at(&reread, paragraph, 3).double_strike, Some(true));
}

#[test]
fn taking_properties_off_removes_them_and_undo_restores() {
    let mut document = load("hello.docx");
    let paragraph = paragraph_with_text(&document, 4);
    let before = document_xml(&document);
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetRunFormatCommand::new(range, full_format())),
                &mut ctx,
            )
            .unwrap();
        let off = HeadlessRunFormat {
            underline: Some("none".into()),
            double_strike: Some(false),
            emphasis: Some("none".into()),
            shading: Some("none".into()),
            border: Some(false),
            spacing: Some(0.0),
            position: Some(0.0),
            east_asian_layout: Some(HeadlessEastAsianLayout::default()),
            ligatures: Some("none".into()),
            ..Default::default()
        };
        executor
            .execute(Box::new(SetRunFormatCommand::new(range, off)), &mut ctx)
            .unwrap();
    }
    let xml = document_xml(&document);
    for gone in [
        "<w:dstrike/>",
        "<w:em ",
        "<w:shd ",
        "<w:bdr ",
        "<w:position ",
        "<w:eastAsianLayout",
        "<w14:ligatures",
    ] {
        assert!(!xml.contains(gone), "{gone} still written");
    }
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
    }
    let format = format_at(&document, paragraph, 0);
    assert_ne!(format.double_strike, Some(true));
    assert_eq!(format.emphasis, None);
    assert!(document_xml(&document).len() <= before.len() + 64);
}
