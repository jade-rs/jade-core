//! Floating pictures positioned from each `relativeFrom` reference frame, as
//! Word resolves them: an offset is measured from the leading edge of the
//! anchor's column, of a margin area, or of the inside / outside margin area
//! for the page's parity, and `wp:align` aligns within the same box.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::RecordingPainter;

/// 9525 EMU per CSS pixel.
const EMU_PER_PX: i64 = 9525;
/// One inch of margin, 1440 twips, is 96px.
const MARGINS: &str = r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
/// A 2in (192px) left margin and a 1in (96px) right margin.
const WIDE_LEFT_MARGINS: &str = r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="2880" w:header="720" w:footer="720" w:gutter="0"/>"#;
/// US Letter: 816 by 1056 px.
const LETTER: &str = r#"<w:pgSz w:w="12240" w:h="15840"/>"#;
/// Two 288px columns 48px apart in the 624px body: column 2 starts at 432.
const TWO_COLUMNS: &str = r#"<w:cols w:num="2" w:space="720"/>"#;
const COLUMN_TWO_X: f64 = 96.0 + 288.0 + 48.0;

/// `wp:posOffset` of `px` pixels.
fn offset(px: i64) -> String {
    format!("<wp:posOffset>{}</wp:posOffset>", px * EMU_PER_PX)
}

/// `wp:align`.
fn align(value: &str) -> String {
    format!("<wp:align>{value}</wp:align>")
}

/// A 40 by 20 px floating picture placed by `h` / `v` (`offset` or `align`)
/// from the `h_from` / `v_from` frames.
fn picture(h_from: &str, h: &str, v_from: &str, v: &str, behind_doc: bool, wrap: &str) -> String {
    let (cx, cy) = (40 * EMU_PER_PX, 20 * EMU_PER_PX);
    let behind = u8::from(behind_doc);
    format!(
        r#"<w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="{behind}" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="{h_from}">{h}</wp:positionH><wp:positionV relativeFrom="{v_from}">{v}</wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>{wrap}<wp:docPr id="1" name="Picture 1"/><wp:cNvGraphicFramePr/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="1" name="Picture 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId10" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    )
}

/// A picture that text does not wrap around, offset by pixels.
fn anchored_picture(h_from: &str, h_px: i64, v_from: &str, v_px: i64) -> String {
    picture(
        h_from,
        &offset(h_px),
        v_from,
        &offset(v_px),
        false,
        "<wp:wrapNone/>",
    )
}

/// Loads `body_xml`, gives every picture image data so it paints, applies
/// `adjust`, and renders it.
fn render(body_xml: &str, adjust: impl FnOnce(&mut Document)) -> CanvasRenderer<RecordingPainter> {
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
}

/// The painted floating pictures as `(page, x, y)`.
fn origins(renderer: &CanvasRenderer<RecordingPainter>) -> Vec<(usize, f64, f64)> {
    renderer
        .image_debug_regions()
        .iter()
        .filter(|region| region.kind == "floating")
        .map(|region| (region.page, region.x, region.y))
        .collect()
}

fn picture_origins(body_xml: &str, adjust: impl FnOnce(&mut Document)) -> Vec<(usize, f64, f64)> {
    origins(&render(body_xml, adjust))
}

/// One picture in a one-paragraph Letter page with `margins`.
fn single(picture: &str, margins: &str) -> (usize, f64, f64) {
    let body = format!(
        r#"<w:p><w:r><w:t>Host</w:t></w:r>{picture}</w:p><w:sectPr>{LETTER}{margins}</w:sectPr>"#
    );
    let origins = picture_origins(&body, |_| {});
    assert_eq!(origins.len(), 1, "{origins:?}");
    origins[0]
}

/// The same picture on pages 1 and 2, with `mirror` margins or not.
fn on_both_pages(picture: &str, mirror: bool) -> [(usize, f64, f64); 2] {
    let body = format!(
        r#"<w:p><w:r><w:t>Odd</w:t></w:r>{picture}</w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>Even</w:t></w:r>{picture}</w:p><w:sectPr>{LETTER}{WIDE_LEFT_MARGINS}</w:sectPr>"#,
    );
    let origins = picture_origins(&body, |document| document.mirror_margins = mirror);
    assert_eq!(origins.len(), 2, "{origins:?}");
    assert_eq!((origins[0].0, origins[1].0), (1, 2));
    [origins[0], origins[1]]
}

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{what}: expected {expected}, got {actual}"
    );
}

/// A two-column section: a picture 10px from its column in a column-2
/// paragraph sits 10px right of column 2's left edge, not of column 1's.
#[test]
fn column_offset_is_measured_from_the_anchor_column() {
    let body = format!(
        r#"<w:p><w:r><w:t>Column one</w:t></w:r>{first}</w:p><w:p><w:r><w:br w:type="column"/></w:r></w:p><w:p><w:r><w:t>Column two</w:t></w:r>{second}</w:p><w:sectPr>{LETTER}{MARGINS}{TWO_COLUMNS}</w:sectPr>"#,
        first = anchored_picture("column", 10, "paragraph", 0),
        second = anchored_picture("column", 10, "paragraph", 0),
    );
    let mut sections = 0;
    let origins = picture_origins(&body, |document| {
        // The section is explicit: one range covering every paragraph, with
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
    assert_near(origins[0].1, 96.0 + 10.0, "column 1 picture x");
    assert_near(origins[1].1, COLUMN_TWO_X + 10.0, "column 2 picture x");
}

/// A paragraph that starts in column 1 and runs on into column 2 anchors its
/// picture in column 1, whether the picture is painted behind the text or in
/// front of it, and whether text wraps around it or not.
#[test]
fn column_frame_is_the_column_the_anchor_paragraph_starts_in() {
    let text = "word ".repeat(400);
    for behind_doc in [false, true] {
        for wrap in ["<wp:wrapNone/>", r#"<wp:wrapSquare wrapText="bothSides"/>"#] {
            let body = format!(
                r#"<w:p>{}<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p><w:sectPr>{LETTER}{MARGINS}{TWO_COLUMNS}</w:sectPr>"#,
                picture(
                    "column",
                    &offset(10),
                    "paragraph",
                    &offset(0),
                    behind_doc,
                    wrap
                ),
            );
            let renderer = render(&body, |_| {});
            let lines = &renderer.selection.paragraphs()[0].lines;
            assert_eq!(
                (
                    lines.first().map(|line| line.column_index),
                    lines.last().map(|line| line.column_index)
                ),
                (Some(0), Some(1)),
                "the paragraph runs from column 1 into column 2"
            );
            let origins = origins(&renderer);
            assert_eq!(origins.len(), 1, "{origins:?}");
            assert_near(
                origins[0].1,
                96.0 + 10.0,
                &format!("behindDoc={behind_doc} {wrap} picture x"),
            );
        }
    }
}

/// `wp:align` within the column frame centres in the anchor's column.
#[test]
fn column_alignment_is_within_the_anchor_column() {
    let body = format!(
        r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p><w:p><w:r><w:t>Column two</w:t></w:r>{}</w:p><w:sectPr>{LETTER}{MARGINS}{TWO_COLUMNS}</w:sectPr>"#,
        picture(
            "column",
            &align("center"),
            "paragraph",
            &offset(0),
            false,
            "<wp:wrapNone/>"
        ),
    );
    let origins = picture_origins(&body, |_| {});
    assert_eq!(origins.len(), 1, "{origins:?}");
    assert_near(
        origins[0].1,
        COLUMN_TWO_X + (288.0 - 40.0) / 2.0,
        "centred picture x",
    );
}

/// `leftMargin` and `topMargin` start at the page edges, where those margin
/// areas begin.
#[test]
fn left_and_top_margin_offsets_are_measured_from_the_page_edges() {
    let (_, x, y) = single(
        &anchored_picture("leftMargin", 10, "topMargin", 10),
        MARGINS,
    );
    assert_near(x, 10.0, "leftMargin picture x");
    assert_near(y, 10.0, "topMargin picture y");
}

/// `rightMargin` measures from the right margin area's left edge, the body's
/// right edge, without subtracting the picture's width.
#[test]
fn right_margin_offset_is_measured_from_the_margin_area_left_edge() {
    let (_, x, _) = single(
        &anchored_picture("rightMargin", 10, "paragraph", 0),
        MARGINS,
    );
    assert_near(x, 816.0 - 96.0 + 10.0, "rightMargin picture x");
}

/// `bottomMargin` measures from the bottom margin area's top edge, the body's
/// bottom edge, with no line height taken off.
#[test]
fn bottom_margin_offset_is_measured_from_the_margin_area_top_edge() {
    let (_, _, y) = single(&anchored_picture("margin", 0, "bottomMargin", 10), MARGINS);
    assert_near(y, 1056.0 - 96.0 + 10.0, "bottomMargin picture y");
}

/// With mirrored 2in inside / 1in outside margins, `insideMargin` is the left
/// margin area on page 1 and the right margin area on page 2.
#[test]
fn inside_margin_follows_page_parity_with_mirror_margins() {
    let [odd, even] = on_both_pages(&anchored_picture("insideMargin", 10, "paragraph", 0), true);
    assert_near(odd.1, 10.0, "page 1 insideMargin picture x");
    // Page 2: the inside margin is now the 2in right margin.
    assert_near(
        even.1,
        816.0 - 192.0 + 10.0,
        "page 2 insideMargin picture x",
    );
}

/// `outsideMargin` is the reverse: the right margin area on page 1 and the
/// left one on page 2, where mirroring has made it the 1in margin.
#[test]
fn outside_margin_follows_page_parity_with_mirror_margins() {
    let [odd, even] = on_both_pages(&anchored_picture("outsideMargin", 10, "paragraph", 0), true);
    assert_near(odd.1, 816.0 - 96.0 + 10.0, "page 1 outsideMargin picture x");
    assert_near(even.1, 10.0, "page 2 outsideMargin picture x");
}

/// Without mirror margins the inside margin still alternates by page parity
/// (ECMA-376 §20.4.3.4: left on odd pages, right on even), with the margins
/// unchanged: 2in left, 1in right on both pages. Not yet confirmed against
/// Word.
#[test]
fn inside_margin_follows_ecma_page_parity_without_mirror_margins() {
    let [odd, even] = on_both_pages(&anchored_picture("insideMargin", 10, "paragraph", 0), false);
    assert_near(odd.1, 10.0, "page 1 insideMargin picture x");
    assert_near(even.1, 816.0 - 96.0 + 10.0, "page 2 insideMargin picture x");
}

/// `wp:align` within the inside / outside margin areas uses the same boxes:
/// centred in the 192px inside margin on page 1, right-aligned in the 96px
/// outside (left) margin on mirrored page 2.
#[test]
fn inside_and_outside_margin_alignment_is_within_the_margin_area() {
    let inside = picture(
        "insideMargin",
        &align("center"),
        "paragraph",
        &offset(0),
        false,
        "<wp:wrapNone/>",
    );
    let [odd, _] = on_both_pages(&inside, true);
    assert_near(odd.1, (192.0 - 40.0) / 2.0, "page 1 centred insideMargin x");
    let outside = picture(
        "outsideMargin",
        &align("right"),
        "paragraph",
        &offset(0),
        false,
        "<wp:wrapNone/>",
    );
    let [odd, even] = on_both_pages(&outside, true);
    assert_near(odd.1, 816.0 - 40.0, "page 1 right-aligned outsideMargin x");
    assert_near(even.1, 96.0 - 40.0, "page 2 right-aligned outsideMargin x");
}

/// Vertically the inside margin is the top margin area and the outside one
/// the bottom margin area, for both an offset and an alignment.
#[test]
fn vertical_inside_and_outside_margins_are_the_top_and_bottom_areas() {
    let (_, _, y) = single(&anchored_picture("margin", 0, "insideMargin", 10), MARGINS);
    assert_near(y, 10.0, "insideMargin offset y");
    let (_, _, y) = single(&anchored_picture("margin", 0, "outsideMargin", 10), MARGINS);
    assert_near(y, 1056.0 - 96.0 + 10.0, "outsideMargin offset y");
    let centred = picture(
        "margin",
        &offset(0),
        "insideMargin",
        &align("center"),
        false,
        "<wp:wrapNone/>",
    );
    let (_, _, y) = single(&centred, MARGINS);
    assert_near(y, (96.0 - 20.0) / 2.0, "centred insideMargin y");
}
