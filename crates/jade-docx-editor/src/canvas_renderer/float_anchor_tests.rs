//! Floating pictures positioned from each `relativeFrom` reference frame, as
//! Word resolves them: an offset is measured from the leading edge of the
//! anchor's column, of the right / bottom margin area, or of the inside
//! margin area for the page's parity.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;

/// 9525 EMU per CSS pixel.
const EMU_PER_PX: i64 = 9525;
/// One inch of margin, 1440 twips, is 96px.
const MARGINS: &str = r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
/// US Letter: 816 by 1056 px.
const LETTER: &str = r#"<w:pgSz w:w="12240" w:h="15840"/>"#;

/// A 40 by 20 px floating picture that text does not wrap around.
fn anchored_picture(h_from: &str, h_px: i64, v_from: &str, v_px: i64) -> String {
    let (cx, cy) = (40 * EMU_PER_PX, 20 * EMU_PER_PX);
    format!(
        r#"<w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="{h_from}"><wp:posOffset>{h}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="{v_from}"><wp:posOffset>{v}</wp:posOffset></wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Picture 1"/><wp:cNvGraphicFramePr/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="1" name="Picture 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId10" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#,
        h = h_px * EMU_PER_PX,
        v = v_px * EMU_PER_PX,
    )
}

/// Loads `body_xml`, gives every picture image data so it paints, applies
/// `adjust`, and returns the painted pictures as `(page, x, y)`.
fn picture_origins(body_xml: &str, adjust: impl FnOnce(&mut Document)) -> Vec<(usize, f64, f64)> {
    let data = build_docx_with_body(body_xml);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut document = (**word_doc.document.as_ref().expect("document")).clone();
    let mut pictures = 0;
    for element in &mut document.body.as_mut().expect("body").elements {
        if let BodyElement::Paragraph(paragraph) = element {
            for run in &mut paragraph.runs {
                if let Some(image) = run.image.as_mut() {
                    image.data_uri = Some("data:image/png;base64,eA==".into());
                    pictures += 1;
                }
            }
        }
    }
    assert!(pictures > 0, "the body has a floating picture");
    adjust(&mut document);
    let mut renderer = recording_renderer();
    renderer
        .render(&document, &Options::default())
        .expect("render");
    renderer
        .image_debug_regions()
        .iter()
        .filter(|region| region.kind == "floating")
        .map(|region| (region.page, region.x, region.y))
        .collect()
}

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{what}: expected {expected}, got {actual}"
    );
}

/// A two-column section (two 288px columns, 48px apart, in a 624px body):
/// a picture 10px from its column in a column-2 paragraph sits 10px right of
/// column 2's left edge, not of column 1's.
#[test]
fn column_offset_is_measured_from_the_anchor_column() {
    let body = format!(
        r#"<w:p><w:r><w:t>Column one</w:t></w:r>{first}</w:p><w:p><w:r><w:br w:type="column"/></w:r></w:p><w:p><w:r><w:t>Column two</w:t></w:r>{second}</w:p><w:sectPr>{LETTER}{MARGINS}<w:cols w:num="2" w:space="720"/></w:sectPr>"#,
        first = anchored_picture("column", 10, "paragraph", 0),
        second = anchored_picture("column", 10, "paragraph", 0),
    );
    let mut sections = 0;
    let origins = picture_origins(&body, |document| {
        // The section is explicit: one range covering both paragraphs, with
        // two columns.
        sections = document.sections.len();
        let columns = document.sections[0]
            .properties
            .as_ref()
            .and_then(|properties| properties.columns.as_ref())
            .and_then(|columns| columns.count);
        assert_eq!(columns, Some(2));
    });
    assert_eq!(sections, 1);
    assert_eq!(origins.len(), 2, "{origins:?}");
    let column_two_x = 96.0 + 288.0 + 48.0;
    assert_near(origins[0].1, 96.0 + 10.0, "column 1 picture x");
    assert_near(origins[1].1, column_two_x + 10.0, "column 2 picture x");
}

/// `rightMargin` measures from the right margin area's left edge, the body's
/// right edge, without subtracting the picture's width.
#[test]
fn right_margin_offset_is_measured_from_the_margin_area_left_edge() {
    let body = format!(
        r#"<w:p><w:r><w:t>Host</w:t></w:r>{}</w:p><w:sectPr>{LETTER}{MARGINS}</w:sectPr>"#,
        anchored_picture("rightMargin", 10, "paragraph", 0),
    );
    let origins = picture_origins(&body, |_| {});
    assert_eq!(origins.len(), 1, "{origins:?}");
    assert_near(origins[0].1, 816.0 - 96.0 + 10.0, "rightMargin picture x");
}

/// `bottomMargin` measures from the bottom margin area's top edge, the body's
/// bottom edge, with no line height taken off.
#[test]
fn bottom_margin_offset_is_measured_from_the_margin_area_top_edge() {
    let body = format!(
        r#"<w:p><w:r><w:t>Host</w:t></w:r>{}</w:p><w:sectPr>{LETTER}{MARGINS}</w:sectPr>"#,
        anchored_picture("margin", 0, "bottomMargin", 10),
    );
    let origins = picture_origins(&body, |_| {});
    assert_eq!(origins.len(), 1, "{origins:?}");
    assert_near(origins[0].2, 1056.0 - 96.0 + 10.0, "bottomMargin picture y");
}

/// With mirrored 2in inside / 1in outside margins, `insideMargin` is the left
/// margin area on page 1 and the right margin area on page 2.
#[test]
fn inside_margin_follows_page_parity_with_mirror_margins() {
    let picture = anchored_picture("insideMargin", 10, "paragraph", 0);
    let body = format!(
        r#"<w:p><w:r><w:t>Odd</w:t></w:r>{picture}</w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>Even</w:t></w:r>{picture}</w:p><w:sectPr>{LETTER}<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="2880" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#,
    );
    let origins = picture_origins(&body, |document| document.mirror_margins = true);
    assert_eq!(origins.len(), 2, "{origins:?}");
    assert_eq!((origins[0].0, origins[1].0), (1, 2));
    // Page 1: the inside (left) margin area starts at the page edge.
    assert_near(origins[0].1, 10.0, "page 1 insideMargin picture x");
    // Page 2: the inside margin is now the 2in right margin.
    assert_near(
        origins[1].1,
        816.0 - 192.0 + 10.0,
        "page 2 insideMargin picture x",
    );
}
