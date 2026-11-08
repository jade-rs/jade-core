//! Tab stops accumulate through styles, with later clears and replacements.
mod common;

use jade_docx_editor::commands::set_paragraph_options::{
    ParagraphOptions, SetParagraphOptionsCommand,
};
use jade_docx_editor::commands::set_paragraph_tabs::SetParagraphTabsCommand;
use jade_docx_editor::document::TabStop;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessDirectTabStop;
use jade_docx_layout::line_wrap::resolve_tab_stops;
use std::io::{Read, Write};

fn package() -> Vec<u8> {
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
            xml.replace_range(start..end, concat!(
                r#"<w:p><w:pPr><w:pStyle w:val="Grandchild"/><w:keepNext/></w:pPr><w:r><w:t>Grandchild</w:t><w:tab/><w:t>Tab</w:t></w:r></w:p>"#,
                r#"<w:p><w:pPr><w:pStyle w:val="Child"/><w:tabs><w:tab w:val="clear" w:pos="2880"/><w:tab w:val="decimal" w:pos="1440"/></w:tabs></w:pPr><w:r><w:t>Direct</w:t></w:r></w:p>"#,
                r#"<w:p><w:r><w:t>Default</w:t></w:r></w:p>"#,
            ));
            bytes = xml.into_bytes();
        } else if name == "word/styles.xml" {
            bytes = concat!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:style w:type="paragraph" w:styleId="Parent" w:default="1"><w:name w:val="Parent"/><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/><w:tab w:val="right" w:pos="1440"/><w:tab w:val="bar" w:pos="2160"/></w:tabs></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Child"><w:name w:val="Child"/><w:basedOn w:val="Parent"/><w:pPr><w:tabs><w:tab w:val="clear" w:pos="720"/><w:tab w:val="center" w:pos="1440"/><w:tab w:val="right" w:pos="2880"/></w:tabs></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Grandchild"><w:name w:val="Grandchild"/><w:basedOn w:val="Child"/><w:pPr><w:tabs><w:tab w:val="left" w:pos="720" w:leader="dot"/><w:tab w:val="clear" w:pos="2160"/></w:tabs></w:pPr></w:style></w:styles>"#,
            ).as_bytes().to_vec();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn stops(document: &WordDocument, index: usize) -> Vec<(f32, String, String)> {
    let tabs = &document
        .paragraph(index)
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .tabs;
    resolve_tab_stops(tabs)
        .into_iter()
        .map(|tab| {
            (
                tab.position_pt,
                format!("{:?}", tab.alignment),
                format!("{:?}", tab.leader),
            )
        })
        .collect()
}

#[test]
fn parent_child_and_direct_tabs_accumulate_in_order() {
    let document = common::load(&package());
    for document in [&document, &common::load(&document.export_docx().unwrap())] {
        assert_eq!(
            stops(document, 0),
            vec![
                (36.0, "Left".into(), "Dot".into()),
                (72.0, "Center".into(), "None".into()),
                (144.0, "Right".into(), "None".into()),
            ]
        );
        assert_eq!(
            stops(document, 1),
            vec![
                (72.0, "Decimal".into(), "None".into()),
                (108.0, "Bar".into(), "None".into()),
            ]
        );
        assert_eq!(
            stops(document, 2),
            vec![
                (36.0, "Left".into(), "None".into()),
                (72.0, "Right".into(), "None".into()),
                (108.0, "Bar".into(), "None".into()),
            ]
        );
    }
}

#[test]
fn replacing_inherited_tabs_survives_reopening_and_undo() {
    for dialog in [false, true] {
        for wanted in [
            vec![],
            vec![HeadlessDirectTabStop {
                position: 216.0,
                kind: "decimal".into(),
                leader: Some("dot".into()),
            }],
        ] {
            let mut session = common::Session::new(common::load(&package()));
            let before = session.xml();
            let original = stops(&session.document, 0);
            let untouched = stops(&session.document, 1);
            if dialog {
                session.run(Box::new(SetParagraphOptionsCommand::new(
                    0,
                    0,
                    ParagraphOptions {
                        tabs: Some(
                            wanted
                                .iter()
                                .map(|tab| TabStop {
                                    position: Some(tab.position),
                                    kind: Some(tab.kind.clone()),
                                    leader: tab.leader.clone(),
                                })
                                .collect(),
                        ),
                        ..Default::default()
                    },
                )));
            } else {
                session.run(Box::new(SetParagraphTabsCommand::new(0, 0, wanted.clone())));
            }
            let expected = if wanted.is_empty() {
                vec![]
            } else {
                vec![(216.0, "Decimal".into(), "Dot".into())]
            };
            for document in [&session.document, &session.reopened()] {
                assert_eq!(stops(document, 0), expected);
                assert_eq!(stops(document, 1), untouched);
                assert!(document.paragraph_pagination(0).keep_next);
            }
            let after = session.xml();
            assert!(after.contains(r#"w:val="clear""#));
            session.undo();
            assert_eq!(session.xml(), before);
            assert_eq!(stops(&session.reopened(), 0), original);
            session.redo();
            assert_eq!(session.xml(), after);
            assert_eq!(stops(&session.reopened(), 0), expected);
        }
    }
}
