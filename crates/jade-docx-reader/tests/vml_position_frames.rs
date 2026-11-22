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

/// The floating layout (`position_h`, `position_v`) and size of the first VML
/// picture or text box parsed from a `v:shape` with `style`.
fn shape(
    style: &str,
    text_box: bool,
) -> (
    Option<jade_docx_reader::ImagePosition>,
    Option<jade_docx_reader::ImagePosition>,
    Option<f64>,
    Option<f64>,
) {
    let content = if text_box {
        r#"<v:textbox><w:txbxContent><w:p><w:r><w:t>box</w:t></w:r></w:p></w:txbxContent></v:textbox>"#
    } else {
        r#"<v:imagedata r:id="rId5"/>"#
    };
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:pict><v:shape style="{style}">{content}</v:shape></w:pict></w:r></w:p></w:body></w:document>"#
    );
    let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    if text_box {
        let text_box = paragraph
            .runs
            .iter()
            .find_map(|run| run.text_box.as_ref())
            .expect("text box");
        (
            text_box.layout.position_h.clone(),
            text_box.layout.position_v.clone(),
            text_box.width_px,
            text_box.height_px,
        )
    } else {
        let image = paragraph
            .runs
            .iter()
            .find_map(|run| run.image.as_ref())
            .expect("picture");
        (
            image.layout.position_h.clone(),
            image.layout.position_v.clone(),
            image.width_px,
            image.height_px,
        )
    }
}

fn align(position: &Option<jade_docx_reader::ImagePosition>) -> Option<&str> {
    position
        .as_ref()
        .and_then(|position| position.align.as_deref())
}

fn offset(position: &Option<jade_docx_reader::ImagePosition>) -> Option<f64> {
    position.as_ref().and_then(|position| position.offset_px)
}

/// `mso-position-horizontal:center` aligns the object within its frame, as
/// `wp:align` does, and overrides `margin-left`.
#[test]
fn vml_horizontal_center_aligns_within_the_margin() {
    for text_box in [false, true] {
        let (h, _, _, _) = shape(
            "position:absolute;margin-left:10pt;margin-top:20pt;width:30pt;height:15pt;mso-position-horizontal:center;mso-position-horizontal-relative:margin",
            text_box,
        );
        assert_eq!(align(&h), Some("center"), "text box {text_box}");
        assert_eq!(offset(&h), None, "text box {text_box}");
        assert_eq!(h.and_then(|h| h.relative_from).as_deref(), Some("margin"));
    }
}

#[test]
fn vml_vertical_bottom_aligns_within_the_page() {
    let (_, v, _, _) = shape(
        "position:absolute;margin-top:20pt;width:30pt;height:15pt;mso-position-vertical:bottom;mso-position-vertical-relative:page",
        false,
    );
    assert_eq!(align(&v), Some("bottom"));
    assert_eq!(offset(&v), None);
    assert_eq!(v.and_then(|v| v.relative_from).as_deref(), Some("page"));
}

/// Without a `-relative` property the keyword aligns within the `text` column
/// horizontally.  Vertically `text` is the paragraph, which the renderer has
/// no extent for, so the offset is kept there.
#[test]
fn vml_alignment_without_a_frame_aligns_only_in_the_column() {
    let (h, v, _, _) = shape(
        "position:absolute;margin-top:6pt;width:30pt;height:15pt;mso-position-horizontal:Outside;mso-position-vertical:center",
        false,
    );
    assert_eq!(align(&h), Some("outside"));
    assert_eq!(h.and_then(|h| h.relative_from).as_deref(), Some("column"));
    assert_eq!(align(&v), None);
    assert_eq!(offset(&v), Some(8.0));
    assert_eq!(
        v.and_then(|v| v.relative_from).as_deref(),
        Some("paragraph")
    );
}

/// The line, paragraph and character frames have no extent to align within,
/// so their keywords keep the offsets.
#[test]
fn vml_alignment_to_a_line_paragraph_or_character_keeps_the_offsets() {
    for (horizontal, vertical) in [("char", "line"), ("char", "text"), ("char", "char")] {
        let (h, v, _, _) = shape(
            &format!(
                "position:absolute;margin-left:12pt;margin-top:24pt;width:30pt;height:15pt;mso-position-horizontal:center;mso-position-horizontal-relative:{horizontal};mso-position-vertical:top;mso-position-vertical-relative:{vertical}"
            ),
            true,
        );
        assert_eq!(align(&h), None, "{horizontal}");
        assert_eq!(align(&v), None, "{vertical}");
        assert_eq!(offset(&h), Some(16.0), "{horizontal}");
        assert_eq!(offset(&v), Some(32.0), "{vertical}");
    }
}

/// Only an absolutely positioned shape is aligned by keyword.
#[test]
fn vml_alignment_needs_absolute_positioning() {
    let (h, _, _, _) = shape(
        "margin-left:12pt;width:30pt;height:15pt;mso-position-horizontal:center;mso-position-horizontal-relative:margin",
        false,
    );
    assert_eq!(align(&h), None);
    assert_eq!(offset(&h), Some(16.0));
}

#[test]
fn vml_negative_sizes_are_ignored() {
    let (_, _, width, height) = shape("position:absolute;width:-30pt;height:-2", false);
    assert_eq!((width, height), (None, None));
}

/// `absolute`, a keyword from the other axis and unknown keywords keep the
/// `margin-left` / `margin-top` offsets.
#[test]
fn vml_absolute_and_unknown_alignment_keep_the_offsets() {
    for (horizontal, vertical) in [("absolute", "absolute"), ("bogus", "left"), ("top", "")] {
        let (h, v, _, _) = shape(
            &format!(
                "position:absolute;margin-left:12pt;margin-top:24pt;width:30pt;height:15pt;mso-position-horizontal:{horizontal};mso-position-vertical:{vertical}"
            ),
            false,
        );
        assert_eq!(align(&h), None, "{horizontal}");
        assert_eq!(align(&v), None, "{vertical}");
        assert_eq!(offset(&h), Some(16.0), "{horizontal}");
        assert_eq!(offset(&v), Some(32.0), "{vertical}");
    }
}

fn close(actual: Option<f64>, expected: f64) -> bool {
    actual.is_some_and(|actual| (actual - expected).abs() < 1e-9)
}

/// VML style lengths take any CSS absolute unit; a bare number is CSS px.
#[test]
fn vml_style_lengths_accept_css_units() {
    let (h, v, width, height) = shape(
        "position:absolute;margin-left:2.54cm;margin-top:25.4mm;width:1in;height:6pc",
        false,
    );
    assert!(close(width, 96.0), "{width:?}");
    assert!(close(height, 96.0), "{height:?}");
    assert!(close(offset(&h), 96.0), "{h:?}");
    assert!(close(offset(&v), 96.0), "{v:?}");

    let (h, v, width, height) = shape(
        "position:absolute;margin-left:-12px;margin-top:7;width:120px;height:40",
        true,
    );
    assert!(close(width, 120.0), "{width:?}");
    assert!(close(height, 40.0), "{height:?}");
    assert!(close(offset(&h), -12.0), "{h:?}");
    assert!(close(offset(&v), 7.0), "{v:?}");
}

/// A `v:group` child sizes and places itself in the group's coordinate
/// space, so its bare numbers are not CSS px and its keywords do not apply.
#[test]
fn vml_group_children_do_not_read_bare_numbers_as_px() {
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml"><w:body><w:p><w:r><w:pict><v:group style="position:absolute;width:216pt;height:135pt" coordorigin="1800,1440" coordsize="8640,5400"><v:shape style="position:absolute;left:1800;top:1440;width:4320;height:2700;margin-left:100;mso-position-horizontal:center;mso-position-horizontal-relative:margin"><v:textbox><w:txbxContent><w:p><w:r><w:t>box</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></v:group></w:pict></w:r></w:p></w:body></w:document>"#;
    let document = DocumentParser::new().parse_document_xml(xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    let text_box = paragraph
        .runs
        .iter()
        .find_map(|run| run.text_box.as_ref())
        .expect("text box");
    assert_eq!(text_box.width_px, None);
    assert_eq!(text_box.height_px, None);
    assert_eq!(align(&text_box.layout.position_h), None);
    assert_eq!(offset(&text_box.layout.position_h), None);
}

/// A bare `0` inset needs no unit, so the other insets are kept.
#[test]
fn vml_inset_accepts_a_bare_zero() {
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml"><w:body><w:p><w:r><w:pict><v:shape style="position:absolute;width:100pt;height:50pt"><v:textbox inset="3.6pt,0,7.2pt,0"><w:txbxContent><w:p><w:r><w:t>box</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p></w:body></w:document>"#;
    let document = DocumentParser::new().parse_document_xml(xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    let insets = &paragraph
        .runs
        .iter()
        .find_map(|run| run.text_box.as_ref())
        .expect("text box")
        .insets;
    assert!(close(Some(insets.left_px), 4.8), "{insets:?}");
    assert!(close(Some(insets.right_px), 9.6), "{insets:?}");
    assert_eq!((insets.top_px, insets.bottom_px), (0.0, 0.0));
}
