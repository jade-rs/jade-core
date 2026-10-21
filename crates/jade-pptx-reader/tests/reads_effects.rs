//! `<a:effectLst>` beyond the outer shadow: inner shadow, glow, reflection
//! and soft edge, on shapes, pictures and runs (ECMA-376 §20.1.8).

use jade_pptx_reader::{parse_slide_xml, Color, ShapeItem};

fn slide(body: &str) -> String {
    format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>{body}</p:spTree></p:cSld></p:sld>"#
    )
}

#[test]
fn every_effect_of_a_shape_is_read_with_its_defaults() {
    let xml = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:prstGeom prst="rect"/><a:effectLst>
          <a:glow rad="228600"><a:schemeClr val="accent2"><a:alpha val="40000"/></a:schemeClr></a:glow>
          <a:innerShdw blurRad="63500" dist="50800" dir="2700000"><a:prstClr val="black"/></a:innerShdw>
          <a:reflection blurRad="6350" stA="52000" endA="300" endPos="35000" dir="5400000" sy="-100000" algn="bl" rotWithShape="0"/>
          <a:softEdge rad="127000"/>
        </a:effectLst></p:spPr></p:sp>"#,
    );
    let content = parse_slide_xml(&xml).unwrap();
    let ShapeItem::Shape(shape) = &content.shapes[0] else {
        panic!("a shape");
    };
    let effects = &shape.effects;
    let glow = effects.glow.as_ref().expect("glow");
    assert_eq!(glow.radius_emu, 228_600);
    assert!(
        matches!(&glow.color, Color::Adjusted { alpha: Some(a), .. } if (*a - 0.4).abs() < 1e-6)
    );
    let inner = effects.inner_shadow.as_ref().expect("inner shadow");
    assert_eq!((inner.blur_emu, inner.distance_emu), (63_500, 50_800));
    assert_eq!(inner.direction_deg, 45.0);
    let reflection = effects.reflection.as_ref().expect("reflection");
    assert!((reflection.start_alpha - 0.52).abs() < 1e-6);
    assert_eq!(reflection.start_position, 0.0);
    assert!((reflection.end_alpha - 0.003).abs() < 1e-6);
    assert!((reflection.end_position - 0.35).abs() < 1e-6);
    assert_eq!(reflection.direction_deg, 90.0);
    // Unstated: fade top to bottom, no horizontal scale change.
    assert_eq!(reflection.fade_direction_deg, 90.0);
    assert_eq!((reflection.scale_x, reflection.scale_y), (1.0, -1.0));
    assert_eq!(reflection.align, "bl");
    assert_eq!(effects.soft_edge_emu, Some(127_000));
    // The list states no outer shadow: an explicit "none".
    assert!(shape.shadow.is_some());
}

#[test]
fn a_picture_and_a_run_read_their_effects() {
    let xml = slide(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="P"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId1"/></p:blipFill>
        <p:spPr><a:effectLst><a:reflection/><a:softEdge rad="63500"/></a:effectLst></p:spPr></p:pic>
        <p:sp><p:nvSpPr><p:cNvPr id="4" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/>
        <p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:effectLst><a:glow rad="63500"><a:srgbClr val="FFC000"/></a:glow></a:effectLst></a:rPr><a:t>Hi</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    let content = parse_slide_xml(&xml).unwrap();
    let ShapeItem::Picture(picture) = &content.shapes[0] else {
        panic!("a picture");
    };
    let reflection = picture.effects.reflection.as_ref().expect("reflection");
    // Every attribute at its schema default: a full-strength copy fading to
    // nothing, mirrored nowhere (sy 100%), anchored at the bottom.
    assert_eq!((reflection.start_alpha, reflection.end_alpha), (1.0, 0.0));
    assert_eq!(reflection.end_position, 1.0);
    assert_eq!(reflection.align, "b");
    assert_eq!(picture.effects.soft_edge_emu, Some(63_500));
    let ShapeItem::Shape(text_box) = &content.shapes[1] else {
        panic!("a text box");
    };
    let run = &text_box.text.as_ref().unwrap().paragraphs[0].runs[0];
    let glow = run.glow.as_ref().expect("run glow");
    assert_eq!(glow.radius_emu, 63_500);
    assert_eq!(glow.color, Color::Srgb("FFC000".into()));
}

#[test]
fn a_shape_without_an_effect_list_has_no_effects() {
    let xml = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp>"#,
    );
    let content = parse_slide_xml(&xml).unwrap();
    let ShapeItem::Shape(shape) = &content.shapes[0] else {
        panic!("a shape");
    };
    assert!(shape.effects.is_empty());
    assert!(shape.shadow.is_none());
}
