//! The reader must return an error, never panic, whatever bytes it is fed.
//!
//! A headless caller (the MCP server, the CLI) hands the reader whatever a
//! user uploaded. A panic there takes the whole process down and every open
//! session with it, so these properties throw random and mutated input at
//! every entry point. The structured generator builds slide XML from the
//! grammar so the deep parsing paths are reached, not just the prologue.

use std::io::Write;

use jade_pptx_reader::{parse_slide_xml, parse_theme_colors, read_deck, read_pptx};
use proptest::prelude::*;

fn zip_with(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in parts {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;
const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;
const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

/// An attribute value with the characters that matter to the parsers: digits,
/// signs, decimal points, XML specials, and non-ASCII.
fn attr_value() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "-?[0-9]{0,12}",
        1 => "[0-9]{1,3}\\.[0-9]{1,3}",
        1 => Just("".to_string()),
        1 => "[A-Fa-f0-9]{0,8}",
        1 => "[a-zA-Z]{1,10}",
        1 => "[^\"<&]{0,8}",
        1 => Just("あ".to_string()),
    ]
}

fn element_name() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "a:off",
        "a:ext",
        "a:chOff",
        "a:chExt",
        "a:xfrm",
        "a:prstGeom",
        "a:solidFill",
        "a:gradFill",
        "a:gsLst",
        "a:gs",
        "a:srgbClr",
        "a:schemeClr",
        "a:lumMod",
        "a:alpha",
        "a:noFill",
        "a:ln",
        "a:prstDash",
        "a:bodyPr",
        "a:normAutofit",
        "a:spAutoFit",
        "a:lstStyle",
        "a:lvl1pPr",
        "a:lvl9pPr",
        "a:defRPr",
        "a:p",
        "a:pPr",
        "a:lnSpc",
        "a:spcPct",
        "a:spcPts",
        "a:spcBef",
        "a:spcAft",
        "a:buChar",
        "a:buAutoNum",
        "a:buNone",
        "a:buSzPct",
        "a:buFont",
        "a:r",
        "a:rPr",
        "a:latin",
        "a:ea",
        "a:hlinkClick",
        "a:t",
        "a:br",
        "a:fld",
        "a:endParaRPr",
        "a:tbl",
        "a:tblGrid",
        "a:gridCol",
        "a:tr",
        "a:tc",
        "a:tcPr",
        "a:lnL",
        "a:lnB",
        "a:graphic",
        "a:graphicData",
        "p:sp",
        "p:cxnSp",
        "p:pic",
        "p:grpSp",
        "p:graphicFrame",
        "p:nvSpPr",
        "p:cNvPr",
        "p:nvPr",
        "p:ph",
        "p:spPr",
        "p:grpSpPr",
        "p:txBody",
        "p:xfrm",
        "p:style",
        "a:fillRef",
        "a:lnRef",
        "a:fontRef",
        "p:blipFill",
        "a:blip",
        "p:bg",
        "p:bgPr",
        "mc:AlternateContent",
        "mc:Choice",
        "mc:Fallback",
    ])
}

fn attribute_name() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "x",
        "y",
        "cx",
        "cy",
        "rot",
        "flipH",
        "flipV",
        "val",
        "prst",
        "w",
        "h",
        "sz",
        "b",
        "i",
        "u",
        "strike",
        "lvl",
        "algn",
        "marL",
        "indent",
        "anchor",
        "wrap",
        "fontScale",
        "lnSpcReduction",
        "lIns",
        "tIns",
        "type",
        "idx",
        "char",
        "typeface",
        "gridSpan",
        "rowSpan",
        "hMerge",
        "vMerge",
        "name",
        "id",
        "r:id",
        "r:embed",
        "pos",
        "showMasterSp",
    ])
}

/// Random well-formed DrawingML-ish trees: real element and attribute names in
/// random nestings with random values, so every parsing branch sees inputs it
/// was not written for.
fn xml_tree(depth: u32) -> impl Strategy<Value = String> {
    let leaf = (
        element_name(),
        prop::collection::vec((attribute_name(), attr_value()), 0..4),
        prop_oneof![
            3 => Just(String::new()),
            1 => "[^<&]{0,12}",
            1 => Just("&amp;&lt;&#x3042;".to_string()),
        ],
    )
        .prop_map(|(name, attrs, text)| {
            let attrs: String = attrs
                .into_iter()
                .map(|(key, value)| format!(" {key}=\"{value}\""))
                .collect();
            if text.is_empty() {
                format!("<{name}{attrs}/>")
            } else {
                format!("<{name}{attrs}>{text}</{name}>")
            }
        });
    leaf.prop_recursive(depth, 64, 6, |inner| {
        (
            element_name(),
            prop::collection::vec((attribute_name(), attr_value()), 0..4),
            prop::collection::vec(inner, 0..6),
        )
            .prop_map(|(name, attrs, children)| {
                let attrs: String = attrs
                    .into_iter()
                    .map(|(key, value)| format!(" {key}=\"{value}\""))
                    .collect();
                format!("<{name}{attrs}>{}</{name}>", children.concat())
            })
    })
}

fn slide_xml() -> impl Strategy<Value = String> {
    xml_tree(5).prop_map(|body| {
        format!(
            r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><p:cSld><p:spTree>{body}</p:spTree></p:cSld></p:sld>"#
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn structured_slide_xml_never_panics(xml in slide_xml()) {
        let _ = parse_slide_xml(&xml);
    }

    #[test]
    fn a_structured_slide_inside_a_package_never_panics(xml in slide_xml()) {
        let bytes = zip_with(&[
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("ppt/presentation.xml", PRESENTATION.as_bytes()),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS.as_bytes()),
            ("ppt/slides/slide1.xml", xml.as_bytes()),
        ]);
        let _ = read_deck(&bytes);
    }

    #[test]
    fn arbitrary_text_never_panics_any_parser(text in "[<>/a-z:=\"&; \\n]{0,64}") {
        let _ = parse_slide_xml(&text);
        let _ = parse_theme_colors(&text);
        let bytes = zip_with(&[
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("ppt/presentation.xml", text.as_bytes()),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS.as_bytes()),
            ("ppt/slides/slide1.xml", text.as_bytes()),
        ]);
        let _ = read_pptx(&bytes);
        let _ = read_deck(&bytes);
    }

    #[test]
    fn arbitrary_bytes_never_panic_the_package_reader(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = read_pptx(&bytes);
        let _ = read_deck(&bytes);
    }
}

#[test]
fn stray_end_tags_are_an_error_not_a_panic() {
    for xml in [
        "</a:p>",
        "<p:sld></p:sld></p:sld>",
        "<p:sld><p:cSld></p:sld>",
        "<p:sld><p:cSld><p:spTree><p:sp></p:cSld></p:sld>",
        "",
        "   ",
        "<?xml version=\"1.0\"?>",
    ] {
        let _ = parse_slide_xml(xml);
        let _ = parse_theme_colors(xml);
    }
}
