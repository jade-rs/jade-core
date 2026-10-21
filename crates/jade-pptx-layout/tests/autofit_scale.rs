//! `normAutofit` ("shrink text on overflow"): text that overflows its box
//! shrinks in PowerPoint's steps — line spacing first, then the font 7.5%
//! at a time — whether or not the part recorded a scale.

use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedShape};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};

fn shape(autofit: &str, paragraphs: usize) -> PlacedShape {
    let body: String = (0..paragraphs)
        .map(|i| {
            format!(r#"<a:p><a:r><a:rPr lang="en-US" sz="2000"/><a:t>Line {i}</a:t></a:r></a:p>"#)
        })
        .collect();
    let xml = format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>
        <p:sp><p:nvSpPr><p:cNvPr id="2" name="Body"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr>
        <p:txBody><a:bodyPr wrap="square">{autofit}</a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#
    );
    let content = parse_slide_xml(&xml).unwrap();
    layout_slide(
        SlideSize {
            width_emu: 12_192_000,
            height_emu: 6_858_000,
        },
        &content,
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
    .shapes
    .remove(0)
}

fn size(shape: &PlacedShape) -> f32 {
    shape.lines[0].runs[0].size_pt
}

#[test]
fn overflowing_text_without_a_recorded_scale_is_drawn_unshrunk_as_powerpoint_opens_it() {
    // PowerPoint recomputes autofit only when the text is edited.
    let unshrunk = shape("<a:normAutofit/>", 12);
    assert_eq!(size(&unshrunk), 20.0);
    assert!(unshrunk.text_overflow_px() > 0.0);
}

#[test]
fn a_recorded_scale_is_honoured() {
    let fitting = shape("<a:normAutofit/>", 2);
    assert_eq!(size(&fitting), 20.0);
    let recorded = shape(
        r#"<a:normAutofit fontScale="77500" lnSpcReduction="20000"/>"#,
        2,
    );
    assert!((size(&recorded) - 15.5).abs() < 1e-4);
    // Text that has outgrown its scale stays at it, as PowerPoint draws it
    // until the text is edited again.
    let grown = shape(
        r#"<a:normAutofit fontScale="92500" lnSpcReduction="10000"/>"#,
        14,
    );
    assert!((size(&grown) - 18.5).abs() < 1e-4);
}
