//! Word underline inheritance and themed run color round trips.
mod common;
use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
use jade_docx_headless_api::HeadlessRunFormat;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::io::{Read, Write};

fn package(origin: &str, direct_underline: &str) -> Vec<u8> {
    package_with_properties(
        origin,
        direct_underline,
        r#"<w:u w:val="wave" w:color="00B050"/>"#,
    )
}

fn package_with_properties(origin: &str, direct_underline: &str, properties: &str) -> Vec<u8> {
    package_with_mapping(origin, direct_underline, properties, "")
}

fn package_with_mapping(
    origin: &str,
    direct_underline: &str,
    properties: &str,
    mapping: &str,
) -> Vec<u8> {
    let current = if origin == "paragraph" {
        "Underline"
    } else {
        "Plain"
    };
    let direct = if origin == "character" {
        r#"<w:rStyle w:val="Underline"/>"#
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
                &format!(r#"<w:p><w:pPr><w:pStyle w:val="{current}"/></w:pPr><w:r><w:rPr>{direct}{direct_underline}</w:rPr><w:t>AB</w:t></w:r></w:p>"#),
            );
            bytes = xml.into_bytes();
        } else if name == "word/settings.xml" && !mapping.is_empty() {
            bytes = format!(r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{mapping}</w:settings>"#).into_bytes();
        } else if name == "word/styles.xml" {
            let style = if origin == "defaults" {
                format!(
                    "<w:docDefaults><w:rPrDefault><w:rPr>{properties}</w:rPr></w:rPrDefault></w:docDefaults>"
                )
            } else {
                format!(
                    r#"<w:style w:type="{origin}" w:styleId="Underline"><w:name w:val="Underline"/><w:rPr>{properties}</w:rPr></w:style>"#
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

fn check(session: &common::Session, expected: [(bool, &str, &str); 2]) {
    for document in [&session.document, &session.reopened()] {
        let paragraph = document.paragraph(0).unwrap();
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.style.as_deref());
        let mut actual = Vec::new();
        for run in &paragraph.runs {
            let effective = document
                .styles_part
                .as_ref()
                .unwrap()
                .resolve_effective_run_properties(style, run.properties.as_ref())
                .unwrap();
            for _ in run.text.chars() {
                actual.push((
                    effective.underline.unwrap_or(false),
                    effective
                        .underline_kind
                        .as_deref()
                        .unwrap_or("single")
                        .to_string(),
                    effective
                        .underline_color
                        .as_deref()
                        .unwrap_or("auto")
                        .to_string(),
                ));
            }
        }
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(on, kind, color)| (on, kind.to_string(), color.to_string()))
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn partial_underline_edits_preserve_other_inherited_attributes() {
    let initial = (true, "wave", "00B050");
    for origin in ["defaults", "paragraph", "character"] {
        for (patch, expected) in [
            (
                HeadlessRunFormat {
                    underline: Some("single".into()),
                    ..Default::default()
                },
                (true, "single", "00B050"),
            ),
            (
                HeadlessRunFormat {
                    underline_color: Some("FF0000".into()),
                    ..Default::default()
                },
                (true, "wave", "FF0000"),
            ),
            (
                HeadlessRunFormat {
                    underline_color: Some("auto".into()),
                    ..Default::default()
                },
                (true, "wave", "auto"),
            ),
        ] {
            let mut session = common::Session::new(common::load(&package(origin, "")));
            check(&session, [initial; 2]);
            let original = session.xml();
            session.run(Box::new(SetRunFormatCommand::new(
                SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                patch,
            )));
            check(&session, [expected, initial]);
            let edited = session.xml();
            if expected.2 == "auto" {
                assert!(edited.contains(r#"w:color="auto""#));
            }
            session.undo();
            assert_eq!(session.xml(), original);
            check(&session, [initial; 2]);
            session.redo();
            assert_eq!(session.xml(), edited);
            check(&session, [expected, initial]);
        }
    }
}

#[test]
fn color_only_underline_import_inherits_the_kind() {
    for origin in ["defaults", "paragraph", "character"] {
        for element in [
            r#"<w:u w:color="FF0000"/>"#,
            r#"<w:u w:color="FF0000"></w:u>"#,
        ] {
            let session = common::Session::new(common::load(&package(origin, element)));
            check(&session, [(true, "wave", "FF0000"); 2]);
        }
    }
}

#[test]
fn underline_theme_colors_resolve_and_survive_partial_edits() {
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for (attributes, expected) in [
            (r#"w:themeColor="accent1""#, "#4472C4"),
            (r#"w:themeColor="accent1" w:themeShade="BF""#, "#2F5496"),
            (r#"w:themeColor="text1" w:themeTint="80""#, "#7F7F7F"),
            (
                r#"w:themeColor="text1" w:themeTint="80" w:themeShade="00""#,
                "#7F7F7F",
            ),
            (r#"w:themeColor="unknown""#, "AA1122"),
        ] {
            for paired in [false, true] {
                let element = if paired {
                    format!(r#"<w:u w:val="wave" w:color="AA1122" {attributes}></w:u>"#)
                } else {
                    format!(r#"<w:u w:val="wave" w:color="AA1122" {attributes}/>"#)
                };
                let bytes = if origin == "direct" {
                    package_with_properties("defaults", &element, "")
                } else {
                    package_with_properties(origin, "", &element)
                };
                let mut session = common::Session::new(common::load(&bytes));
                let initial = (true, "wave", expected);
                check(&session, [initial; 2]);
                let original = session.xml();
                session.run(Box::new(SetRunFormatCommand::new(
                    SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                    HeadlessRunFormat {
                        underline: Some("double".into()),
                        ..Default::default()
                    },
                )));
                check(&session, [(true, "double", expected), initial]);
                // A direct theme reference survives editing the underline kind.
                if origin == "direct" {
                    assert!(session.xml().contains(attributes));
                }
                session.undo();
                assert_eq!(session.xml(), original);
                check(&session, [initial; 2]);
                session.redo();
                check(&session, [(true, "double", expected), initial]);
                for color in ["FF0000", "auto"] {
                    session.run(Box::new(SetRunFormatCommand::new(
                        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                        HeadlessRunFormat {
                            underline_color: Some(color.into()),
                            ..Default::default()
                        },
                    )));
                    check(&session, [(true, "double", color), initial]);
                    for document in [&session.document, &session.reopened()] {
                        let effective = document
                            .styles_part
                            .as_ref()
                            .unwrap()
                            .resolve_effective_run_properties(
                                document
                                    .paragraph(0)
                                    .unwrap()
                                    .properties
                                    .as_ref()
                                    .and_then(|p| p.style.as_deref()),
                                document.paragraph(0).unwrap().runs[0].properties.as_ref(),
                            )
                            .unwrap();
                        assert!(effective.underline_theme_color.is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn color_only_underline_theme_inherits_kind_without_cached_color() {
    for origin in ["defaults", "paragraph", "character"] {
        for element in [
            r#"<w:u w:themeColor="accent1" w:themeShade="BF"/>"#,
            r#"<w:u w:themeColor="accent1" w:themeShade="BF"></w:u>"#,
        ] {
            let session = common::Session::new(common::load(&package(origin, element)));
            check(&session, [(true, "wave", "#2F5496"); 2]);
        }
    }
}

#[test]
fn text_theme_tint_overrides_shade_through_styles_and_partial_edits() {
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for expanded in [false, true] {
            for (tint, expected) in [("00", "#FFFFFF"), ("80", "#7F7F7F"), ("FF", "#000000")] {
                let end = if expanded { "></w:color>" } else { "/>" };
                let property = format!(
                    r#"<w:color w:val="AA1122" w:themeColor="text1" w:themeTint="{tint}" w:themeShade="00"{end}"#
                );
                let bytes = if origin == "direct" {
                    package_with_properties("paragraph", &property, "")
                } else {
                    package_with_properties(origin, "", &property)
                };
                let mut session = common::Session::new(common::load(&bytes));
                let check_color = |session: &common::Session| {
                    for document in [&session.document, &session.reopened()] {
                        let paragraph = document.paragraph(0).unwrap();
                        let style = paragraph
                            .properties
                            .as_ref()
                            .and_then(|p| p.style.as_deref());
                        let mut chars = 0;
                        for run in &paragraph.runs {
                            let effective = document
                                .styles_part
                                .as_ref()
                                .unwrap()
                                .resolve_effective_run_properties(style, run.properties.as_ref())
                                .unwrap();
                            assert_eq!(
                                effective.color.as_deref(),
                                Some(expected),
                                "{origin}, {tint}, {expanded}"
                            );
                            let theme = effective.theme_color.as_ref().unwrap();
                            assert_eq!(theme.tint.as_deref(), Some(tint));
                            assert_eq!(theme.shade.as_deref(), Some("00"));
                            chars += run.text.chars().count();
                        }
                        assert_eq!(chars, 2);
                    }
                };
                check_color(&session);
                let original = session.xml();
                session.run(Box::new(SetRunFormatCommand::new(
                    SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                    HeadlessRunFormat {
                        bold: Some(true),
                        ..Default::default()
                    },
                )));
                check_color(&session);
                session.undo();
                assert_eq!(session.xml(), original);
                check_color(&session);
                session.redo();
                check_color(&session);
            }
        }
    }
}

#[test]
fn document_color_mapping_resolves_text_and_underline_across_partial_edits() {
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for expanded in [false, true] {
            let end = if expanded {
                "></w:clrSchemeMapping>"
            } else {
                "/>"
            };
            let mapping = format!(
                r#"<w:clrSchemeMapping w:t1="light1" w:accent1="accent2" w:accent2="accent1"{end}"#
            );
            let properties = r#"<w:color w:val="AA1122" w:themeColor="text1"/><w:u w:val="double" w:color="AA1122" w:themeColor="accent1"/>"#;
            let bytes = if origin == "direct" {
                package_with_mapping("paragraph", properties, "", &mapping)
            } else {
                package_with_mapping(origin, "", properties, &mapping)
            };
            let mut session = common::Session::new(common::load(&bytes));
            let verify = |session: &common::Session| {
                check(session, [(true, "double", "#ED7D31"); 2]);
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
                        assert_eq!(effective.color.as_deref(), Some("#FFFFFF"));
                        assert_eq!(
                            effective.theme_color.as_ref().unwrap().name.as_ref(),
                            "text1"
                        );
                        assert_eq!(
                            effective
                                .underline_theme_color
                                .as_ref()
                                .unwrap()
                                .name
                                .as_ref(),
                            "accent1"
                        );
                    }
                }
            };
            verify(&session);
            let original = session.xml();
            session.run(Box::new(SetRunFormatCommand::new(
                SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                HeadlessRunFormat {
                    italic: Some(true),
                    ..Default::default()
                },
            )));
            verify(&session);
            session.undo();
            assert_eq!(session.xml(), original);
            verify(&session);
            session.redo();
            verify(&session);
        }
    }
}
