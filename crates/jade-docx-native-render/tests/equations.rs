//! Equations as the native shell shows them: inline equations laid out in
//! two dimensions inside their line, display equations on their own, EQ
//! fields drawn from their switches, and an equation edited in place.
mod common;

use jade_docx_headless_api::HeadlessCommand;
use std::io::Write;

const M: &str = r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

fn docx(body: &str) -> Vec<u8> {
    docx_with_math_font(body, None)
}

fn docx_with_math_font(body: &str, math_font: Option<&str>) -> Vec<u8> {
    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" {M}><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (path, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/document.xml", document_xml.as_str()),
        ] {
            zip.start_file(path, options).unwrap();
            if path == "[Content_Types].xml" && math_font.is_some() {
                let content = content.replace("</Types>", r#"<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#);
                zip.write_all(content.as_bytes()).unwrap();
            } else {
                zip.write_all(content.as_bytes()).unwrap();
            }
        }
        if let Some(family) = math_font {
            zip.start_file("word/_rels/document.xml.rels", options)
                .unwrap();
            zip.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="settings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#).unwrap();
            zip.start_file("word/settings.xml", options).unwrap();
            write!(zip, r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" {M}><m:mathPr><m:mathFont m:val="{family}"/></m:mathPr></w:settings>"#).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

const FRACTION: &str = "<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>";

#[test]
fn a_custom_math_face_survives_page_rebuild_and_pdf_export() {
    use jade_docx_native_render::{FaceSpec, NativeDocument};
    let math_path = common::repo_path("crates/jade-docx-layout/tests/fixtures/math-font.ttf");
    let text_path = common::repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let faces = [
        FaceSpec {
            family: "Host Math",
            weight: 400,
            path: &math_path,
        },
        FaceSpec {
            family: "Cambria Math",
            weight: 400,
            path: &math_path,
        },
        FaceSpec {
            family: "Arial",
            weight: 400,
            path: &text_path,
        },
    ];
    let bytes = docx_with_math_font(
        // The synthetic face maps upright x, not mathematical italic 𝑥.
        r#"<w:p><m:oMath><m:r><m:rPr><m:sty m:val="p"/></m:rPr><m:t>x</m:t></m:r></m:oMath></w:p>"#,
        Some("Host Math"),
    );
    let mut document = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
    let assert_math_face = |document: &NativeDocument| {
        let stats = document.renderer().math_font_stats();
        let fonts = stats["fonts"].as_array().unwrap();
        assert_eq!(
            fonts.len(),
            2,
            "only the configured and fallback math faces: {stats}"
        );
        let custom = fonts
            .iter()
            .find(|font| font["family"] == "Host Math")
            .unwrap_or_else(|| panic!("configured math face was not bound: {stats}"));
        assert!(
            custom["layouts"].as_u64().unwrap() > 0,
            "custom MATH table was used: {stats}"
        );
        assert!(
            custom["cached_glyphs"].as_u64().unwrap() > 0,
            "custom outlines were used: {stats}"
        );
    };
    assert_math_face(&document);
    document
        .apply_command(
            HeadlessCommand::SetPageOrientation { landscape: true },
            true,
        )
        .unwrap();
    assert!(document.layout_page_size().0 > document.layout_page_size().1);
    assert_math_face(&document);
    assert!(document.export_pdf().unwrap().starts_with(b"%PDF-"));
    assert_math_face(&document);
    let reopened =
        NativeDocument::open(&document.export_docx().unwrap(), 816.0, 1056.0, &faces).unwrap();
    assert_math_face(&reopened);
}

#[test]
fn an_inline_equation_is_an_object_in_its_line_and_typing_after_it_works() {
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">ab </w:t></w:r>{FRACTION}<w:r><w:t xml:space="preserve"> cd</w:t></w:r></w:p>"#
    );
    let mut document = common::open_bytes(&docx(&body));
    let zones = document.renderer().math_zones().to_vec();
    assert_eq!(zones.len(), 1, "one inline equation");
    let zone = &zones[0];
    assert!(!zone.display);
    assert_eq!(zone.paragraph, 0);
    assert!(
        zone.plan.metrics.descent > 2.0,
        "the denominator hangs below"
    );
    assert!(zone.plan.metrics.ascent > 8.0);
    // A click just right of the equation puts the caret after it, and
    // typing lands between the equation and " cd".
    let (paragraph, offset) = document
        .position_at(
            zone.x + zone.plan.metrics.width + 0.5,
            zone.absolute_baseline - 2.0,
        )
        .unwrap();
    assert_eq!(paragraph, 0);
    document.insert_text(0, offset, "Y", 1).unwrap();
    let text = document.paragraph_text(0).unwrap();
    assert_eq!(text, "ab Y cd");
    // Rendered offsets count the equation as one object character.
    let end = text.len() + '\u{fffc}'.len_utf8();
    document.insert_text(0, end, "X", 1).unwrap();
    let text = document.paragraph_text(0).unwrap();
    assert!(text.ends_with("cdX"), "{text:?}");
    let saved = document.export_docx().unwrap();
    let reopened = common::open_bytes(&saved);
    assert_eq!(reopened.renderer().math_zones().len(), 1);
}

#[test]
fn display_and_inline_equations_are_replaced_through_the_command() {
    let body = format!(
        r#"<w:p><w:r><w:t>x</w:t></w:r>{FRACTION}</w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>y=1</m:t></m:r></m:oMath></m:oMathPara></w:p>"#
    );
    let mut document = common::open_bytes(&docx(&body));
    let zones = document.renderer().math_zones().to_vec();
    let display = zones.iter().find(|z| z.display).unwrap();
    let inline = zones.iter().find(|z| !z.display).unwrap();
    let omml =
        format!(r#"<m:oMathPara {M}><m:oMath><m:r><m:t>z=2</m:t></m:r></m:oMath></m:oMathPara>"#);
    document
        .apply_command(
            HeadlessCommand::ReplaceEquation {
                paragraph: display.paragraph,
                run_index: None,
                omml,
            },
            false,
        )
        .unwrap();
    let omml = format!(r#"<m:oMath {M}><m:r><m:t>w</m:t></m:r></m:oMath>"#);
    document
        .apply_command(
            HeadlessCommand::ReplaceEquation {
                paragraph: inline.paragraph,
                run_index: inline.run_index,
                omml,
            },
            false,
        )
        .unwrap();
    let texts: Vec<String> = document
        .renderer()
        .math_zones()
        .iter()
        .map(|z| z.plan.text.clone())
        .collect();
    assert!(texts.iter().any(|t| t == "𝑧=2"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "𝑤"), "{texts:?}");
}

#[test]
fn an_eq_field_is_drawn_from_its_switches() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">a </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> EQ \f(1,2) </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1/2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let document = common::open_bytes(&docx(body));
    let zones = document.renderer().math_zones();
    assert_eq!(zones.len(), 1, "the EQ field is laid out as math");
    let plan = &zones[0].plan;
    assert_eq!(plan.rules.len(), 1, "a fraction bar");
    assert!(plan.glyphs.iter().any(|g| g.baseline < 0.0));
    let text = document.paragraph_text(0).unwrap();
    assert!(
        !text.contains("1/2"),
        "the saved result is not drawn too: {text:?}"
    );
    assert!(document.find("1/2", false).is_empty());
    let hits = document.find("a", false);
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].start_offset, hits[0].end_offset), (0, 1));
}
