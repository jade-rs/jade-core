//! Equations as the native shell shows them: inline equations laid out in
//! two dimensions inside their line, display equations on their own, EQ
//! fields drawn from their switches, and an equation edited in place.
mod common;

use jade_docx_headless_api::HeadlessCommand;
use std::io::Write;

const M: &str = r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

fn docx(body: &str) -> Vec<u8> {
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
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

const FRACTION: &str = "<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>";

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
}
