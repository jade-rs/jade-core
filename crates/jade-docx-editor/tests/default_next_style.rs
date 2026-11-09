//! Enter follows the effective paragraph style, including an implicit default.
mod common;

use jade_docx_editor::commands::insert_paragraph_break::{
    InsertParagraphBreakCommand, InsertParagraphBreaksCommand,
};
use jade_docx_editor::commands::Command;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::io::{Read, Write};

fn package(properties: &str, next: &str) -> Vec<u8> {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/numbering-by-style.docx");
    let mut input = zip::ZipArchive::new(std::fs::File::open(fixture).unwrap()).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let mut xml = String::from_utf8(bytes).unwrap();
            let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
            let end = xml.find("<w:sectPr").unwrap();
            xml.replace_range(
                start..end,
                &format!("<w:p>{properties}<w:r><w:t>AB</w:t></w:r></w:p>"),
            );
            bytes = xml.into_bytes();
        } else if name == "word/styles.xml" {
            bytes = format!(concat!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:style w:type="paragraph" w:styleId="Starter" w:default="1"><w:name w:val="Starter"/><w:next w:val="{}"/><w:pPr><w:jc w:val="center"/><w:spacing w:before="80"/></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/><w:next w:val="Tail"/><w:pPr><w:jc w:val="right"/><w:spacing w:before="240"/></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Tail"><w:name w:val="Tail"/><w:pPr><w:jc w:val="left"/><w:spacing w:before="0"/></w:pPr></w:style>"#,
                r#"<w:style w:type="character" w:styleId="Character"><w:name w:val="Character"/></w:style></w:styles>"#,
            ), next).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn style(document: &WordDocument, index: usize) -> Option<String> {
    document
        .paragraph(index)
        .unwrap()
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
        .map(str::to_string)
}

fn check_next(document: &WordDocument, index: usize, expected: &str) {
    assert_eq!(style(document, index).as_deref(), Some(expected));
    let props = document
        .paragraph(index)
        .unwrap()
        .properties
        .as_ref()
        .unwrap();
    assert_eq!(
        props.alignment.as_deref(),
        Some(if expected == "Body" { "right" } else { "left" })
    );
    assert_eq!(
        props.spacing.as_ref().unwrap().before,
        Some(if expected == "Body" { 12.0 } else { 0.0 })
    );
    assert_eq!(props.keep_next, Some(true), "direct formatting survives");
}

fn enter(offset: usize, count: usize) -> Box<dyn Command> {
    let range = SelectionRange::collapsed(TextPosition::new(0, offset));
    if count == 0 {
        Box::new(InsertParagraphBreakCommand::new(range))
    } else {
        Box::new(InsertParagraphBreaksCommand::new(range, count))
    }
}

fn check_default_transition(properties: &str) {
    for count in [0, 1, 2] {
        let mut session = common::Session::new(common::load(&package(properties, "Body")));
        let original = session.xml();
        let original_style = style(&session.document, 0);
        session.run(enter(2, count));
        for document in [&session.document, &session.reopened()] {
            assert_eq!(style(document, 0), original_style);
            check_next(document, 1, "Body");
            if count == 2 {
                check_next(document, 2, "Tail");
            }
        }
        let edited = session.xml();
        session.undo();
        assert_eq!(session.xml(), original);
        session.redo();
        assert_eq!(session.xml(), edited);
        check_next(&session.document, 1, "Body");
        check_next(&session.reopened(), 1, "Body");
    }
}

#[test]
fn implicit_default_style_supplies_next_style() {
    check_default_transition("<w:pPr><w:keepNext/></w:pPr>");
    // No pPr and an empty pPr must select the same default. Their lack of
    // direct formatting is checked separately from the preservation cases.
    for properties in ["", "<w:pPr/>"] {
        let mut session = common::Session::new(common::load(&package(properties, "Body")));
        session.run(enter(2, 0));
        for document in [&session.document, &session.reopened()] {
            assert_eq!(style(document, 1).as_deref(), Some("Body"));
        }
    }
}

#[test]
fn undefined_paragraph_style_uses_default_next_style() {
    check_default_transition(r#"<w:pPr><w:pStyle w:val="Missing"/><w:keepNext/></w:pPr>"#);
}

#[test]
fn splitting_inside_a_paragraph_keeps_its_style() {
    for count in [0, 1] {
        let mut session = common::Session::new(common::load(&package("", "Body")));
        session.run(enter(1, count));
        for document in [&session.document, &session.reopened()] {
            assert_eq!(style(document, 1), None);
            let props = document.paragraph(1).unwrap().properties.as_ref().unwrap();
            assert_eq!(props.alignment.as_deref(), Some("center"));
        }
    }
}

#[test]
fn invalid_next_style_is_ignored() {
    for next in ["Missing", "Character", "Starter"] {
        for count in [0, 1] {
            let mut session = common::Session::new(common::load(&package("", next)));
            session.run(enter(2, count));
            for document in [&session.document, &session.reopened()] {
                assert_eq!(style(document, 1), None);
                let props = document.paragraph(1).unwrap().properties.as_ref().unwrap();
                assert_eq!(props.alignment.as_deref(), Some("center"));
            }
        }
    }
}
