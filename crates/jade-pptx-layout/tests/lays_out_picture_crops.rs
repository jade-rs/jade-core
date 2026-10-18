use jade_pptx_layout::{layout_slide, HeuristicMeasure, Rect};
use jade_pptx_reader::{parse_slide_xml, ShapeItem, SlideSize, SourceRect, ThemeColors};

fn layout(crop: &str) -> jade_pptx_layout::SlideLayout {
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree>
    <p:pic><p:nvPicPr><p:cNvPr id="9" name="cropped"/></p:nvPicPr><p:blipFill>{crop}</p:blipFill><p:spPr><a:xfrm><a:off x="952500" y="476250"/><a:ext cx="1905000" cy="952500"/></a:xfrm></p:spPr></p:pic>
    </p:spTree></p:cSld></p:sld>"#
    );
    let content = parse_slide_xml(&xml).unwrap();
    layout_slide(
        SlideSize::default(),
        &content,
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
}

#[test]
fn crops_source_pixels_to_the_whole_picture_frame() {
    let layout = layout(r#"<a:srcRect l="25000" t="10000" r="25000" b="20000"/>"#);
    let shape = &layout.shapes[0];
    assert_eq!(
        shape.source_rect,
        Some(SourceRect {
            left: 25000,
            top: 10000,
            right: 25000,
            bottom: 20000
        })
    );
    let (source, destination) = shape.image_rectangles(400.0, 200.0).unwrap();
    assert_eq!(
        source,
        Rect {
            x: 100.0,
            y: 20.0,
            width: 200.0,
            height: 140.0
        }
    );
    assert_eq!(destination, shape.rect);
}

#[test]
fn negative_offsets_keep_transparent_margins_and_missing_offsets_default_to_zero() {
    let layout = layout(r#"<a:srcRect l="-25000" r="-25000"/>"#);
    let (source, destination) = layout.shapes[0].image_rectangles(300.0, 100.0).unwrap();
    assert_eq!(
        source,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 100.0
        }
    );
    assert!((destination.x - (100.0 + 200.0 / 6.0)).abs() < 1e-9);
    assert!((destination.width - 200.0 / 1.5).abs() < 1e-9);
    let plain = self::layout("");
    assert_eq!(
        plain.shapes[0].image_rectangles(300.0, 100.0).unwrap().1,
        plain.shapes[0].rect
    );
}

#[test]
fn empty_or_inverted_crops_and_invalid_bitmaps_do_not_produce_draw_coordinates() {
    for crop in [
        r#"<a:srcRect l="50000" r="50000"/>"#,
        r#"<a:srcRect t="90000" b="20000"/>"#,
        r#"<a:srcRect l="120000"/>"#,
    ] {
        assert!(layout(crop).shapes[0]
            .image_rectangles(300.0, 100.0)
            .is_none());
    }
    let shape = &layout("").shapes[0];
    assert!(shape.image_rectangles(f64::NAN, 100.0).is_none());
    assert!(shape.image_rectangles(100.0, 0.0).is_none());
}

#[test]
fn picture_fills_and_percent_lexical_offsets_are_read_too() {
    let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:spPr><a:blipFill><a:srcRect l="12.5%" b="-10%"/></a:blipFill></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#;
    let parsed = parse_slide_xml(xml).unwrap();
    let ShapeItem::Shape(shape) = &parsed.shapes[0] else {
        panic!("shape");
    };
    assert_eq!(
        shape.source_rect,
        Some(SourceRect {
            left: 12500,
            top: 0,
            right: 0,
            bottom: -10000
        })
    );
    let mut parsed = parsed;
    let ShapeItem::Shape(shape) = &mut parsed.shapes[0] else {
        panic!("shape");
    };
    shape.transform = Some(jade_pptx_reader::Transform {
        offset_emu: (0, 0),
        extent_emu: (952500, 952500),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    });
    shape.image_rel_id = Some("rId1".into());
    shape.image_part = Some("ppt/media/image1.png".into());
    shape.style = Some(jade_pptx_reader::ShapeStyle {
        fill_index: 1,
        fill_color: Some(jade_pptx_reader::Color::Srgb("FF0000".into())),
        line_index: 0,
        line_color: None,
        font_color: None,
        effect_index: 0,
    });
    let expected_crop = shape.source_rect;
    let layout = layout_slide(
        SlideSize::default(),
        &parsed,
        &ThemeColors::default(),
        &HeuristicMeasure,
    );
    assert_eq!(layout.shapes[0].source_rect, expected_crop);
    assert!(
        layout.shapes[0].fill.is_none(),
        "a picture fill must override the theme fill reference"
    );
}
