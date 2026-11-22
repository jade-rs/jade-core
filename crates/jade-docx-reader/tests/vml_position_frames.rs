//! VML floating positions name their frame as DrawingML's `relativeFrom`
//! does, so the renderer resolves `mso-position-*-relative` like
//! `wp:positionH` / `wp:positionV`.

use jade_docx_reader::{BodyElement, DocumentParser};

/// The `(horizontal, vertical)` frames of an absolutely positioned VML picture.
fn frames(horizontal: &str, vertical: &str) -> (Option<String>, Option<String>) {
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:pict><v:shape style="position:absolute;margin-left:10pt;margin-top:20pt;width:30pt;height:15pt;mso-position-horizontal-relative:{horizontal};mso-position-vertical-relative:{vertical}"><v:imagedata r:id="rId5"/></v:shape></w:pict></w:r></w:p></w:body></w:document>"#
    );
    let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    let image = paragraph
        .runs
        .iter()
        .find_map(|run| run.image.as_ref())
        .expect("picture");
    let frame = |position: &Option<jade_docx_reader::ImagePosition>| {
        position
            .as_ref()
            .and_then(|position| position.relative_from.clone())
    };
    (
        frame(&image.layout.position_h),
        frame(&image.layout.position_v),
    )
}

fn pair(horizontal: &str, vertical: &str) -> (Option<String>, Option<String>) {
    (Some(horizontal.into()), Some(vertical.into()))
}

#[test]
fn vml_text_and_char_frames_are_the_column_paragraph_and_character() {
    assert_eq!(frames("text", "text"), pair("column", "paragraph"));
    assert_eq!(frames("char", "line"), pair("character", "line"));
    assert_eq!(frames("page", "margin"), pair("page", "margin"));
}

#[test]
fn vml_margin_areas_are_drawingml_margin_frames() {
    assert_eq!(
        frames("left-margin-area", "top-margin-area"),
        pair("leftMargin", "topMargin")
    );
    assert_eq!(
        frames("right-margin-area", "bottom-margin-area"),
        pair("rightMargin", "bottomMargin")
    );
    assert_eq!(
        frames("inner-margin-area", "outer-margin-area"),
        pair("insideMargin", "outsideMargin")
    );
}

/// A margin area that does not exist on the axis is the body box.
#[test]
fn margin_areas_on_the_wrong_axis_are_the_body_box() {
    assert_eq!(
        frames("top-margin-area", "left-margin-area"),
        pair("margin", "margin")
    );
    assert_eq!(
        frames("bottomMargin", "rightMargin"),
        pair("margin", "margin")
    );
}

#[test]
fn vml_frames_match_case_insensitively() {
    assert_eq!(
        frames("Right-Margin-Area", "PAGE"),
        pair("rightMargin", "page")
    );
}

/// Vertically `char` is the line; anything VML does not define is the axis
/// default, `text`.
#[test]
fn vertical_char_is_the_line_and_unknown_frames_are_text() {
    assert_eq!(frames("char", "char"), pair("character", "line"));
    assert_eq!(frames("line", "bogus"), pair("column", "paragraph"));
}

/// Without `mso-position-*-relative`, VML positions from `text`.
#[test]
fn an_absent_vml_frame_is_text() {
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:pict><v:shape style="position:absolute;margin-left:10pt;margin-top:20pt;width:30pt;height:15pt"><v:imagedata r:id="rId5"/></v:shape></w:pict></w:r></w:p></w:body></w:document>"#;
    let document = DocumentParser::new().parse_document_xml(xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    let image = paragraph
        .runs
        .iter()
        .find_map(|run| run.image.as_ref())
        .expect("picture");
    let frame = |position: &Option<jade_docx_reader::ImagePosition>| {
        position
            .as_ref()
            .and_then(|position| position.relative_from.clone())
    };
    assert_eq!(
        (
            frame(&image.layout.position_h),
            frame(&image.layout.position_v)
        ),
        pair("column", "paragraph")
    );
}
