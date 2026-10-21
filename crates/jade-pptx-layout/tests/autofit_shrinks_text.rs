//! `normAutofit` ("shrink text on overflow"): text that overflows its box
//! shrinks in PowerPoint's steps — line spacing first, then the font 7.5%
//! at a time — whether or not the part recorded a scale.

use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedShape};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};

fn shape(autofit: &str, paragraphs: usize) -> PlacedShape {
    let body: String = (0..paragraphs)
        .map(|i| format!(r#"<a:p><a:r><a:rPr lang="en-US" sz="2000"/><a:t>Line {i}</a:t></a:r></a:p>"#))
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
fn overflowing_text_without_a_recorded_scale_shrinks_until_it_fits() {
    // 2 inches tall: 20pt lines at 1.2 spacing are 32px, so 12 lines need
    // twice the room.
    let shrunk = shape("<a:normAutofit/>", 12);
    assert!(size(&shrunk) < 20.0, "shrunk to {}", size(&shrunk));
    assert!(shrunk.text_overflow_px() <= shrunk.rect.height * 0.02 + 0.5);
    // A PowerPoint step: 20pt × (0.925 − 0.075 k).
    let scale = size(&shrunk) / 20.0;
    let k = (0.925 - scale) / 0.075;
    assert!((k - k.round()).abs() < 1e-3, "scale {scale}");
    // Without autofit the same text runs out of the box at full size.
    let overflowing = shape("", 12);
    assert_eq!(size(&overflowing), 20.0);
    assert!(overflowing.text_overflow_px() > 0.0);
}

#[test]
fn text_that_fits_keeps_its_size_and_a_recorded_scale_is_honoured() {
    let fitting = shape("<a:normAutofit/>", 2);
    assert_eq!(size(&fitting), 20.0);
    // What PowerPoint recorded is used as it stands when the text fits.
    let recorded = shape(r#"<a:normAutofit fontScale="77500" lnSpcReduction="20000"/>"#, 2);
    assert!((size(&recorded) - 15.5).abs() < 1e-4);
}

#[test]
fn line_spacing_tightens_before_the_font_shrinks() {
    // Just over: six lines of 32px in a 192px box less its insets.
    let tight = shape("<a:normAutofit/>", 6);
    assert_eq!(size(&tight), 20.0, "spacing alone fits");
    let gap = tight.lines[1].top - tight.lines[0].top;
    assert!(gap < 20.0 * 96.0 / 72.0 * 1.2 - 1e-6, "{gap}");
}

#[test]
fn text_that_outgrew_its_recorded_scale_shrinks_further() {
    let grown = shape(r#"<a:normAutofit fontScale="92500" lnSpcReduction="10000"/>"#, 14);
    assert!(size(&grown) < 18.5 - 1e-4);
    assert!(grown.text_overflow_px() <= grown.rect.height * 0.02 + 0.5);
}
