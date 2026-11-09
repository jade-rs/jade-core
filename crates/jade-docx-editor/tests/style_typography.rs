//! Explicit typography resets override every style level and retain other fields.
mod common;
use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
use jade_docx_headless_api::HeadlessRunFormat;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::io::{Read, Write};

fn package(origin: &str) -> Vec<u8> {
    let properties = r#"<w14:ligatures w14:val="standard"/><w14:numForm w14:val="oldStyle"/><w14:numSpacing w14:val="tabular"/>"#;
    let current = if origin == "paragraph" {
        "Typography"
    } else {
        "Plain"
    };
    let direct = if origin == "character" {
        r#"<w:rStyle w:val="Typography"/>"#
    } else {
        ""
    };
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
            let style = if origin == "defaults" {
                format!("<w:docDefaults><w:rPrDefault><w:rPr>{properties}</w:rPr></w:rPrDefault></w:docDefaults>")
            } else {
                format!(
                    r#"<w:style w:type="{origin}" w:styleId="Typography"><w:name w:val="Typography"/><w:rPr>{properties}</w:rPr></w:style>"#
                )
            };
            bytes = format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">{style}<w:style w:type="paragraph" w:styleId="Plain" w:default="1"><w:name w:val="Plain"/></w:style></w:styles>"#).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn check(session: &common::Session, expected: [&str; 3]) {
    for document in [&session.document, &session.reopened()] {
        let paragraph = document.paragraph(0).unwrap();
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.style.as_deref());
        for run in &paragraph.runs {
            let effective = document
                .styles_part
                .as_ref()
                .unwrap()
                .resolve_effective_run_properties(style, run.properties.as_ref())
                .unwrap();
            let effects = effective.text_effects.as_deref().unwrap();
            assert_eq!(
                [
                    effects.ligatures.as_deref().unwrap(),
                    effects.number_form.as_deref().unwrap(),
                    effects.number_spacing.as_deref().unwrap(),
                ],
                expected
            );
        }
    }
}

#[test]
fn typography_resets_override_inheritance_without_resetting_other_features() {
    let initial = ["standard", "oldStyle", "tabular"];
    let resets = ["none", "default", "default"];
    let tags = ["ligatures", "numForm", "numSpacing"];
    for origin in ["defaults", "paragraph", "character"] {
        for index in 0..3 {
            let mut session = common::Session::new(common::load(&package(origin)));
            check(&session, initial);
            let original = session.xml();
            let mut patch = HeadlessRunFormat::default();
            match index {
                0 => patch.ligatures = Some(resets[index].into()),
                1 => patch.number_form = Some(resets[index].into()),
                _ => patch.number_spacing = Some(resets[index].into()),
            }
            session.run(Box::new(SetRunFormatCommand::new(
                SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 2)),
                patch,
            )));
            let mut expected = initial;
            expected[index] = resets[index];
            check(&session, expected);
            let reset = session.xml();
            assert!(reset.contains(&format!(
                r#"<w14:{} w14:val="{}"/>"#,
                tags[index], resets[index]
            )));
            session.undo();
            assert_eq!(session.xml(), original);
            check(&session, initial);
            session.redo();
            assert_eq!(session.xml(), reset);
            check(&session, expected);
        }
    }
}
