//! Floating objects sit in Word's page layers: an object in front of text
//! covers every paragraph on its page, one behind text lies under all of it,
//! whichever paragraph anchors it.

mod common;
use common::open_bytes;
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// A red rectangle 1in from the page's top left, 4in by 2in.
fn shape_run(behind_doc: bool) -> String {
    format!(
        r#"<w:r><w:drawing><wp:anchor behindDoc="{behind}" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="10" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV><wp:extent cx="3657600" cy="1828800"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Shape 1"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="3657600" cy="1828800"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln><a:noFill/></a:ln></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#,
        behind = u8::from(behind_doc)
    )
}

fn text_paragraph() -> String {
    format!(
        "<w:p><w:r><w:t>{}</w:t></w:r></w:p>",
        "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM"
    )
}

/// Ten paragraphs of text from the top margin (1in) down, with the shape
/// anchored in the first or the last of them.
fn docx(behind_doc: bool, anchor_last: bool) -> Vec<u8> {
    let mut body = String::new();
    for index in 0..10 {
        let anchor = if anchor_last { index == 9 } else { index == 0 };
        if anchor {
            body.push_str(&format!(
                "<w:p><w:r><w:t>MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM</w:t></w:r>{}</w:p>",
                shape_run(behind_doc)
            ));
        } else {
            body.push_str(&text_paragraph());
        }
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", document),
        ] {
            zip.start_file(name, options).expect("zip entry");
            zip.write_all(content.as_bytes()).expect("zip write");
        }
        zip.finish().expect("zip");
    }
    bytes
}

/// (red, dark) pixel counts inside the shape, away from its edges.
fn inside_the_shape(bytes: &[u8]) -> (usize, usize) {
    let document = open_bytes(bytes);
    let (page_width, _) = document.layout_page_size();
    let (pixels, width, height) = document.page_pixels(0).expect("page pixels");
    let scale = f64::from(width) / page_width;
    let (mut red, mut dark) = (0, 0);
    // The shape spans 96..480 x 96..288 CSS px; the text from 96 down.
    for y in (110.0 * scale) as i32..(270.0 * scale) as i32 {
        for x in (110.0 * scale) as i32..(460.0 * scale) as i32 {
            if y >= height || x >= width {
                continue;
            }
            let index = ((y * width + x) * 4) as usize;
            let (b, g, r) = (pixels[index], pixels[index + 1], pixels[index + 2]);
            if r > 200 && g < 60 && b < 60 {
                red += 1;
            } else if r < 100 && g < 100 && b < 100 {
                dark += 1;
            }
        }
    }
    (red, dark)
}

#[test]
fn a_shape_in_front_of_text_covers_the_paragraphs_after_its_anchor() {
    let (red, dark) = inside_the_shape(&docx(false, false));
    assert!(red > 1000, "the shape is drawn ({red} red pixels)");
    assert_eq!(dark, 0, "no text shows through a shape in front of text");
}

#[test]
fn a_shape_behind_text_lies_under_the_paragraphs_before_its_anchor() {
    let (red, dark) = inside_the_shape(&docx(true, true));
    assert!(red > 1000, "the shape is drawn ({red} red pixels)");
    assert!(
        dark > 100,
        "the earlier paragraphs' text is drawn over the shape"
    );
}
