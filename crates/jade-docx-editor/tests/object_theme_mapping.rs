//! Source document color mappings survive detached object paste and reparsing.
mod common;

use common::*;
use jade_docx_editor::commands::paste_object::PasteObjectCommand;
use jade_docx_editor::document::BodyElement;
use jade_docx_editor::docx_xml::XmlNode;
use jade_docx_editor::drawing_xml::descendant_mut;
use jade_docx_editor::word_document::{FloatingObjectKind, FloatingObjectTarget, WordDocument};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::io::{Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const PROPERTIES: &str = r#"<w:color w:val="AA1122" w:themeColor="text1"/><w:u w:val="double" w:color="AA1122" w:themeColor="accent1"/>"#;

fn target() -> FloatingObjectTarget {
    FloatingObjectTarget {
        paragraph_index: 0,
        run_index: 0,
        kind: FloatingObjectKind::TextBox,
    }
}

fn mapped_package(origin: &str, mapping: &str, boxed: bool) -> Vec<u8> {
    mapped_package_with_properties(origin, mapping, boxed, PROPERTIES)
}

fn mapped_package_with_properties(
    origin: &str,
    mapping: &str,
    boxed: bool,
    properties: &str,
) -> Vec<u8> {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/numbering-by-style.docx");
    let mut input = zip::ZipArchive::new(std::fs::File::open(fixture).unwrap()).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let ppr = if origin == "paragraph" {
        r#"<w:pPr><w:pStyle w:val="Mapped"/></w:pPr>"#
    } else {
        ""
    };
    let rpr = match origin {
        "direct" => properties,
        "character" => r#"<w:rStyle w:val="Mapped"/>"#,
        _ => "",
    };
    let paragraphs = format!(
        r#"<w:p>{ppr}<w:r><w:rPr>{rpr}</w:rPr><w:t>Mapped</w:t></w:r><w:r><w:rPr><w:color w:themeColor="dark1"/></w:rPr><w:t>Physical</w:t></w:r></w:p>"#
    );
    let body = if boxed {
        format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="2540000" cy="1016000"/><wp:docPr id="1" name="MappedBox"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1016000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"><a:alpha val="50000"/></a:schemeClr></a:solidFill><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></wps:spPr><wps:txbx><w:txbxContent>{paragraphs}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        )
    } else if origin.starts_with("numbering") {
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>List</w:t></w:r></w:p>"#.to_owned()
    } else {
        "<w:p><w:r><w:t>Host</w:t></w:r></w:p>".to_owned()
    };
    let style = match origin {
        "defaults" => format!("<w:docDefaults><w:rPrDefault><w:rPr>{properties}</w:rPr></w:rPrDefault></w:docDefaults>"),
        "paragraph" | "character" => format!(r#"<w:style w:type="{origin}" w:styleId="Mapped"><w:name w:val="Mapped"/><w:rPr>{properties}</w:rPr></w:style>"#),
        _ => String::new(),
    };
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        match name.as_str() {
            "word/document.xml" => bytes = format!(r#"<w:document xmlns:w="{W}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:body>{body}</w:body></w:document>"#).into_bytes(),
            "word/settings.xml" => bytes = format!(r#"<w:settings xmlns:w="{W}">{mapping}</w:settings>"#).into_bytes(),
            "word/numbering.xml" if origin == "numbering-override" => bytes = format!(r#"<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="7"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:rPr><w:color w:val="FF0000"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="7"><w:abstractNumId w:val="7"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/><w:rPr>{properties}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#).into_bytes(),
            "word/numbering.xml" if origin == "numbering" => bytes = format!(r#"<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="7"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:rPr>{properties}</w:rPr></w:lvl></w:abstractNum><w:num w:numId="7"><w:abstractNumId w:val="7"/></w:num></w:numbering>"#).into_bytes(),
            "word/styles.xml" => bytes = format!(r#"<w:styles xmlns:w="{W}">{style}<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#).into_bytes(),
            _ => {},
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn check_colors(document: &WordDocument, text: &str, underline: &str) {
    let box_ = document
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .find_map(|run| run.text_box.as_ref())
        .unwrap();
    assert_eq!(
        box_.fill_color
            .as_deref()
            .map(|color| color.trim_start_matches('#')),
        Some(underline.trim_start_matches('#'))
    );
    assert_eq!(
        box_.border
            .color
            .as_deref()
            .map(|color| color.trim_start_matches('#')),
        Some(text.trim_start_matches('#'))
    );
    assert_eq!(box_.fill_opacity, Some(0.5));
    check_text_colors(document, text, underline);
}

fn check_text_colors(document: &WordDocument, text: &str, underline: &str) {
    let box_ = document.paragraph(0).unwrap().runs[0]
        .text_box
        .as_ref()
        .unwrap();
    let paragraph = box_
        .blocks
        .iter()
        .find_map(|block| match block {
            BodyElement::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
        .unwrap();
    let style = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref());
    let effective = document
        .styles_part
        .as_ref()
        .unwrap()
        .resolve_effective_run_properties(style, paragraph.runs[0].properties.as_ref())
        .unwrap();
    assert_eq!(effective.color.as_deref(), Some(text));
    assert_eq!(
        effective
            .underline_color
            .as_deref()
            .map(|color| color.trim_start_matches('#')),
        Some(underline.trim_start_matches('#'))
    );
    assert_eq!(
        paragraph.runs[1]
            .properties
            .as_ref()
            .unwrap()
            .color
            .as_deref(),
        Some("#000000")
    );
}

#[test]
fn pasted_text_box_keeps_source_document_color_mapping() {
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for expanded in [false, true] {
            let mapping = if expanded {
                r#"<w:clrSchemeMapping w:t1="light1" w:accent1="accent2" w:accent2="accent1"></w:clrSchemeMapping>"#
            } else {
                r#"<w:clrSchemeMapping w:t1="light1" w:accent1="accent2" w:accent2="accent1"/>"#
            };
            let source = load(&mapped_package(origin, mapping, true));
            check_colors(&source, "#FFFFFF", "#ED7D31");
            let source_before = source.export_docx().unwrap();
            let clip = source.copy_object(target()).unwrap();
            assert_eq!(source.export_docx().unwrap(), source_before);
            let mut host = Session::new(load(&mapped_package(
                "direct",
                r#"<w:clrSchemeMapping w:t1="dark1" w:accent1="accent3"/>"#,
                false,
            )));
            let before = host.document.export_docx().unwrap();
            let settings = part(&before, "word/settings.xml");
            let theme = part(&before, "word/theme/theme1.xml");
            host.run(Box::new(PasteObjectCommand::new(
                SelectionRange::collapsed(TextPosition::new(0, 0)),
                clip,
            )));
            check_colors(&host.document, "#FFFFFF", "#ED7D31");
            check_colors(&host.reopened(), "#FFFFFF", "#ED7D31");
            let after = host.document.export_docx().unwrap();
            assert_eq!(part(&after, "word/settings.xml"), settings);
            assert_eq!(part(&after, "word/theme/theme1.xml"), theme);
            host.undo();
            assert_eq!(host.document.export_docx().unwrap(), before);
            host.redo();
            assert_eq!(host.document.export_docx().unwrap(), after);
            check_colors(&host.document, "#FFFFFF", "#ED7D31");
        }
    }
}

#[test]
fn editing_text_box_xml_resolves_document_theme_and_mapping_again() {
    for (mapping, text, underline) in [
        (
            r#"<w:clrSchemeMapping w:t1="light1" w:accent1="accent2"/>"#,
            "#FFFFFF",
            "#ED7D31",
        ),
        ("", "#000000", "#4472C4"),
    ] {
        let mut document = load(&mapped_package("direct", mapping, true));
        check_colors(&document, text, underline);
        let mut drawing = document.object_drawing(target()).unwrap();
        let t = descendant_mut(&mut drawing, "w:t").unwrap();
        t.children = vec![XmlNode::Text("Changed".to_owned())];
        document.replace_object_drawing(target(), drawing).unwrap();
        check_colors(&document, text, underline);
        let bytes = document.export_docx().unwrap();
        assert!(part(&bytes, "word/document.xml").contains("Changed"));
        check_colors(&load(&bytes), text, underline);
    }
}

#[test]
fn changing_document_theme_recolors_shapes_through_document_mapping() {
    let mut document = load(&mapped_package(
        "direct",
        r#"<w:clrSchemeMapping w:t1="light1" w:accent1="accent2" w:accent2="accent1"/>"#,
        true,
    ));
    let mut theme = document.document_theme().unwrap();
    theme.accent1 = "#ABCDEF".into();
    theme.accent2 = "#123456".into();
    assert!(document.set_document_theme(&theme).unwrap());
    let check = |document: &WordDocument| {
        let shape = document.paragraph(0).unwrap().runs[0]
            .text_box
            .as_ref()
            .unwrap();
        assert_eq!(shape.fill_color.as_deref(), Some("#123456"));
        assert_eq!(shape.fill_opacity, Some(0.5));
        assert_eq!(shape.border.color.as_deref(), Some("#FFFFFF"));
    };
    check(&document);
    check(&load(&document.export_docx().unwrap()));
}

#[test]
fn theme_command_refreshes_direct_and_inherited_text_colors_immediately() {
    use jade_docx_editor::commands::set_document_theme::SetDocumentThemeCommand;
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for mapped in [false, true] {
            let mapping = if mapped {
                r#"<w:clrSchemeMapping w:t1="dark2" w:accent1="accent2"/>"#
            } else {
                ""
            };
            let mut session = Session::new(load(&mapped_package(origin, mapping, true)));
            let (old_text, old_underline) = if mapped {
                ("#44546A", "#ED7D31")
            } else {
                ("#000000", "#4472C4")
            };
            check_colors(&session.document, old_text, old_underline);
            let before = session.document.export_docx().unwrap();
            let mut theme = session.document.document_theme().unwrap();
            theme.dark2 = "#224466".into();
            theme.accent1 = "#ABCDEF".into();
            theme.accent2 = "#123456".into();
            session.run(Box::new(SetDocumentThemeCommand::new(theme)));
            let (text, underline) = if mapped {
                ("#224466", "#123456")
            } else {
                ("#000000", "#ABCDEF")
            };
            check_colors(&session.document, text, underline);
            check_colors(&session.reopened(), text, underline);
            let after = session.document.export_docx().unwrap();
            for name in ["word/document.xml", "word/styles.xml", "word/settings.xml"] {
                assert_eq!(part(&before, name), part(&after, name));
            }
            session.undo();
            check_colors(&session.document, old_text, old_underline);
            assert_eq!(session.document.export_docx().unwrap(), before);
            session.redo();
            check_colors(&session.document, text, underline);
            assert_eq!(session.document.export_docx().unwrap(), after);
        }
    }
}

#[test]
fn theme_change_reapplies_text_color_transforms_after_mapping() {
    use jade_docx_editor::commands::set_document_theme::SetDocumentThemeCommand;
    for origin in ["direct", "defaults", "paragraph", "character"] {
        for both in [false, true] {
            let extra = if both { r#" w:themeShade="00""# } else { "" };
            let properties = format!(
                r#"<w:color w:val="AA1122" w:themeColor="text1" w:themeShade="80"/><w:u w:val="double" w:color="AA1122" w:themeColor="accent1" w:themeTint="80"{extra}/>"#
            );
            let mut session = Session::new(load(&mapped_package_with_properties(
                origin,
                r#"<w:clrSchemeMapping w:t1="dark2" w:accent1="accent2"/>"#,
                true,
                &properties,
            )));
            let mut theme = session.document.document_theme().unwrap();
            theme.dark2 = "#FFFFFF".into();
            theme.accent2 = "#000000".into();
            session.run(Box::new(SetDocumentThemeCommand::new(theme)));
            check_text_colors(&session.document, "#808080", "#7F7F7F");
            check_text_colors(&session.reopened(), "#808080", "#7F7F7F");
            session.undo();
            session.redo();
            check_text_colors(&session.document, "#808080", "#7F7F7F");
        }
    }
}

#[test]
fn numbering_theme_colors_refresh_and_survive_roundtrip_and_history() {
    use jade_docx_editor::commands::set_document_theme::SetDocumentThemeCommand;
    for origin in ["numbering", "numbering-override"] {
        for transformed in [false, true] {
            let properties = if transformed {
                r#"<w:color w:val="AA1122" w:themeColor="text1" w:themeShade="80"/><w:u w:val="double" w:color="AA1122" w:themeColor="accent1" w:themeTint="80" w:themeShade="00"/>"#
            } else {
                PROPERTIES
            };
            let mut session = Session::new(load(&mapped_package_with_properties(
                origin,
                r#"<w:clrSchemeMapping w:t1="dark2" w:accent1="accent2"/>"#,
                false,
                properties,
            )));
            let check = |document: &WordDocument, text: &str, underline: &str| {
                let props = document
                    .numbering_part
                    .as_ref()
                    .unwrap()
                    .get_level_info("7", 0)
                    .unwrap()
                    .run_props
                    .as_ref()
                    .unwrap();
                assert_eq!(props.color.as_deref(), Some(text));
                assert_eq!(props.underline, Some(true));
                assert_eq!(props.underline_kind.as_deref(), Some("double"));
                assert_eq!(props.underline_color.as_deref(), Some(underline));
                assert_eq!(props.theme_color.as_ref().unwrap().name.as_ref(), "text1");
                assert_eq!(
                    props.underline_theme_color.as_ref().unwrap().name.as_ref(),
                    "accent1"
                );
            };
            if !transformed {
                check(&session.document, "#44546A", "#ED7D31");
            }
            if origin == "numbering-override" {
                let numbering = session.document.numbering_part.as_ref().unwrap();
                assert_eq!(
                    numbering.get_level_info("7", 0).unwrap().num_fmt.as_deref(),
                    Some("upperRoman")
                );
                assert_eq!(numbering.start_value("7", 0), Some(4));
            }
            let before = session.document.export_docx().unwrap();
            let mut theme = session.document.document_theme().unwrap();
            theme.dark2 = "#FFFFFF".into();
            theme.accent2 = "#000000".into();
            session.run(Box::new(SetDocumentThemeCommand::new(theme)));
            let (text, underline) = if transformed {
                ("#808080", "#7F7F7F")
            } else {
                ("#FFFFFF", "#000000")
            };
            check(&session.document, text, underline);
            check(&session.reopened(), text, underline);
            let after = session.document.export_docx().unwrap();
            assert_eq!(
                part(&before, "word/numbering.xml"),
                part(&after, "word/numbering.xml")
            );
            session.undo();
            assert_eq!(session.document.export_docx().unwrap(), before);
            if !transformed {
                check(&session.document, "#44546A", "#ED7D31");
            }
            session.redo();
            assert_eq!(session.document.export_docx().unwrap(), after);
            check(&session.document, text, underline);
        }
    }
}
