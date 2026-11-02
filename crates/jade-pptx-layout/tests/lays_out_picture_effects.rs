//! A picture with corrections, colour or artistic effects names its image
//! by an effect key with the theme's colours resolved, an SVG picture names
//! the SVG rather than its bitmap fallback, and a picture cropped to a
//! freeform is clipped to that outline.

use jade_picture_effects::{parse_key, Effect};
use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedGeometry};
use jade_pptx_reader::{
    BlipEffect, Color, CustomGeometry, GeometryPath, PathCommand, Picture, ShapeItem, SlideContent,
    SlideSize, Transform,
};

fn theme() -> jade_pptx_reader::ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="111111"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
    </a:clrScheme></a:themeElements></a:theme>"#;
    jade_pptx_reader::parse_theme_colors(xml).expect("theme")
}

fn picture() -> Picture {
    Picture {
        id: Some(4),
        name: "図 3".into(),
        transform: Some(Transform {
            offset_emu: (0, 0),
            extent_emu: (952_500, 952_500),
            rotation_60000ths: 0,
            flip_h: false,
            flip_v: false,
        }),
        blip_rel_id: Some("rId2".into()),
        image_part: Some("ppt/media/image1.png".into()),
        ..Picture::default()
    }
}

fn lay_out(picture: Picture) -> jade_pptx_layout::PlacedShape {
    let content = SlideContent {
        shapes: vec![ShapeItem::Picture(picture)],
        ..SlideContent::default()
    };
    let size = SlideSize {
        width_emu: 12_192_000,
        height_emu: 6_858_000,
    };
    layout_slide(size, &content, &theme(), &HeuristicMeasure)
        .shapes
        .remove(0)
}

#[test]
fn a_plain_picture_names_its_part() {
    assert_eq!(
        lay_out(picture()).image_part.as_deref(),
        Some("ppt/media/image1.png")
    );
}

#[test]
fn effects_ride_on_the_key_with_theme_colours_resolved() {
    let placed = lay_out(Picture {
        blip_effects: vec![
            BlipEffect::Duotone {
                dark: Color::Srgb("000000".into()),
                light: Color::Scheme("accent1".into()),
            },
            // Baked into the picture the blip shows: not drawn again.
            BlipEffect::Artistic("artisticBlur".into()),
            BlipEffect::Grayscale,
        ],
        ..picture()
    });
    let key = placed.image_part.unwrap();
    let (part, effects) = parse_key(&key);
    assert_eq!(part, "ppt/media/image1.png");
    assert_eq!(
        effects,
        vec![
            Effect::Duotone {
                dark: [0, 0, 0],
                light: [0x44, 0x72, 0xC4]
            },
            Effect::Grayscale,
        ]
    );
}

#[test]
fn an_svg_picture_draws_its_svg() {
    let placed = lay_out(Picture {
        svg_part: Some("ppt/media/image2.svg".into()),
        ..picture()
    });
    assert_eq!(placed.image_part.as_deref(), Some("ppt/media/image2.svg"));
}

#[test]
fn a_picture_cropped_to_a_freeform_is_clipped_to_it() {
    let placed = lay_out(Picture {
        custom_geometry: Some(CustomGeometry {
            paths: vec![GeometryPath {
                width: 10,
                height: 10,
                filled: true,
                stroked: true,
                commands: vec![
                    PathCommand::MoveTo(5, 0),
                    PathCommand::LineTo(10, 10),
                    PathCommand::LineTo(0, 10),
                    PathCommand::Close,
                ],
            }],
        }),
        ..picture()
    });
    assert!(
        !matches!(placed.geometry, PlacedGeometry::Rect),
        "{:?}",
        placed.geometry
    );
}
