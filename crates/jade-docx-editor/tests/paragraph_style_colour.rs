//! Paragraph style colours must stay inherited across edits and reopening.
mod common;
use jade_docx_editor::commands::insert_text::InsertTextCommand;
use jade_docx_editor::commands::set_paragraph_style::SetParagraphStyleCommand;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::io::{Read, Write};

fn package(current: &str, direct: &str) -> Vec<u8> {
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
                &format!(r#"<w:p><w:pPr><w:pStyle w:val="{current}"/></w:pPr><w:r><w:rPr>{direct}</w:rPr><w:t>AB</w:t></w:r></w:p>"#),
            );
            bytes = xml.into_bytes();
        } else if name == "word/styles.xml" {
            bytes = concat!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:style w:type="paragraph" w:styleId="Plain" w:default="1"><w:name w:val="Plain"/></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Theme"><w:name w:val="Theme"/><w:rPr><w:color w:val="4472C4" w:themeColor="accent1" w:themeTint="80"/></w:rPr></w:style>"#,
                r#"</w:styles>"#,
            ).as_bytes().to_vec();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn apply(session: &mut common::Session, style: &str) {
    session.run(Box::new(SetParagraphStyleCommand::new(
        0,
        0,
        Some(style.into()),
    )));
}

fn check_inherited(session: &common::Session) {
    for document in [&session.document, &session.reopened()] {
        for run in &document.paragraph(0).unwrap().runs {
            if let Some(properties) = &run.properties {
                assert_eq!(properties.color, None, "no direct colour");
                assert_eq!(properties.theme_color, None, "no direct theme reference");
            }
        }
    }
    assert!(
        !session.xml().contains("<w:color"),
        "colour stays in styles.xml"
    );
}

fn transition(current: &str, direct: &str) {
    let mut session = common::Session::new(common::load(&package(current, direct)));
    let before = session.xml();
    apply(&mut session, "Theme");
    check_inherited(&session);
    let styled = session.xml();
    session.undo();
    assert_eq!(session.xml(), before);
    session.redo();
    assert_eq!(session.xml(), styled);
    check_inherited(&session);
    session.run(Box::new(InsertTextCommand::new(
        SelectionRange::collapsed(TextPosition::new(0, 1)),
        "X".into(),
        None,
    )));
    check_inherited(&session);
    apply(&mut session, "Plain");
    check_inherited(&session);
}

#[test]
fn applying_theme_style_does_not_create_direct_theme_colour() {
    transition("Plain", r#"<w:color w:val="FF0000"/>"#);
}

#[test]
fn reapplying_theme_style_does_not_create_direct_theme_colour() {
    transition("Theme", "");
}

#[test]
fn style_without_colour_preserves_direct_theme_colour() {
    let mut session = common::Session::new(common::load(&package(
        "Plain",
        r#"<w:color w:val="4472C4" w:themeColor="accent1" w:themeTint="80"/>"#,
    )));
    let before = session.xml();
    let original = session.document.paragraph(0).unwrap().runs[0]
        .properties
        .as_ref()
        .unwrap()
        .clone();
    apply(&mut session, "Plain");
    for document in [&session.document, &session.reopened()] {
        let properties = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(properties.color, original.color);
        assert_eq!(properties.theme_color, original.theme_color);
    }
    let after = session.xml();
    session.undo();
    assert_eq!(session.xml(), before);
    session.redo();
    assert_eq!(session.xml(), after);
}
