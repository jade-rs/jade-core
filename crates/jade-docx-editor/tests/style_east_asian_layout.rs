//! East Asian run layout follows paragraph/character styles across editing.
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
                r#"<w:style w:type="paragraph" w:styleId="Combined"><w:name w:val="Combined"/><w:rPr><w:eastAsianLayout w:id="7" w:combine="1" w:combineBrackets="round"/></w:rPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Vertical"><w:name w:val="Vertical"/><w:rPr><w:eastAsianLayout w:id="8" w:vert="1"/></w:rPr></w:style>"#,
                r#"<w:style w:type="character" w:styleId="Off"><w:rPr><w:eastAsianLayout w:combine="0" w:vert="0"/></w:rPr></w:style>"#,
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

fn check(session: &common::Session, combine: bool, vert: bool) {
    for document in [&session.document, &session.reopened()] {
        let paragraph = document.paragraph(0).unwrap();
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|properties| properties.style.as_deref());
        for run in &paragraph.runs {
            let effective = document
                .styles_part
                .as_ref()
                .unwrap()
                .resolve_effective_run_properties(style, run.properties.as_ref())
                .unwrap();
            let layout = effective.east_asian_layout.as_deref();
            assert_eq!(layout.is_some_and(|layout| layout.combine), combine);
            assert_eq!(layout.is_some_and(|layout| layout.vert), vert);
        }
    }
}

#[test]
fn paragraph_layout_style_changes_survive_edits_and_history() {
    for current in ["Plain", "Combined"] {
        let mut session = common::Session::new(common::load(&package(
            current,
            r#"<w:eastAsianLayout w:id="1" w:combine="0" w:vert="0"/>"#,
        )));
        let original = session.xml();
        apply(&mut session, "Combined");
        check(&session, true, false);
        assert!(
            !session.xml().contains("w:eastAsianLayout"),
            "layout stays inherited"
        );
        let styled = session.xml();
        session.undo();
        assert_eq!(session.xml(), original);
        session.redo();
        assert_eq!(session.xml(), styled);
        check(&session, true, false);
        session.run(Box::new(InsertTextCommand::new(
            SelectionRange::collapsed(TextPosition::new(0, 1)),
            "X".into(),
            None,
        )));
        check(&session, true, false);
        assert!(!session.xml().contains("w:eastAsianLayout"));
        apply(&mut session, "Vertical");
        check(&session, false, true);
        apply(&mut session, "Plain");
        check(&session, false, false);
    }
}

#[test]
fn character_style_can_disable_paragraph_layout() {
    let mut session = common::Session::new(common::load(&package(
        "Plain",
        r#"<w:rStyle w:val="Off"/>"#,
    )));
    apply(&mut session, "Combined");
    check(&session, false, false);
    assert!(session.xml().contains(r#"w:val="Off""#));
    assert!(!session.xml().contains("w:eastAsianLayout"));
}

#[test]
fn style_without_layout_keeps_direct_combined_characters() {
    let mut session = common::Session::new(common::load(&package(
        "Plain",
        r#"<w:eastAsianLayout w:id="9" w:combine="1" w:combineBrackets="square"/>"#,
    )));
    apply(&mut session, "Plain");
    check(&session, true, false);
    assert!(session.xml().contains("w:eastAsianLayout"));
}

#[test]
fn disabling_inherited_layout_survives_save_and_history() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_headless_api::{HeadlessEastAsianLayout, HeadlessRunFormat};
    for (style, combine, vert) in [("Combined", true, false), ("Vertical", false, true)] {
        let mut session = common::Session::new(common::load(&package(style, "")));
        check(&session, combine, vert);
        let original = session.xml();
        session.run(Box::new(SetRunFormatCommand::new(
            SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 2)),
            HeadlessRunFormat {
                east_asian_layout: Some(HeadlessEastAsianLayout::default()),
                ..Default::default()
            },
        )));
        check(&session, false, false);
        let disabled = session.xml();
        assert!(disabled.contains(r#"w:combine="0""#));
        assert!(disabled.contains(r#"w:vert="0""#));
        session.undo();
        assert_eq!(session.xml(), original);
        check(&session, combine, vert);
        session.redo();
        assert_eq!(session.xml(), disabled);
        check(&session, false, false);
    }
}
