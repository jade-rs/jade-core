//! Gradient fills resolve to placed stops with a direction, theme fill
//! styles substitute a `fillRef`'s colour for `phClr`, and custom geometry
//! flattens to normalized paths.

use jade_pptx_layout::{
    layout_slide, layout_slide_inheriting, HeuristicMeasure, InheritedContent, PlacedGeometry,
};
use jade_pptx_reader::{
    Color, CustomGeometry, Fill, GeometryPath, GradientStop, PathCommand, Shape, ShapeItem,
    ShapeStyle, SlideContent, SlideSize, ThemeColors, Transform,
};

fn transform(x: i64, y: i64, cx: i64, cy: i64) -> Transform {
    Transform {
        offset_emu: (x, y),
        extent_emu: (cx, cy),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

fn shape(name: &str, fill: Option<Fill>) -> Shape {
    Shape {
        id: None,
        name: name.to_string(),
        placeholder: None,
        transform: Some(transform(0, 0, 1_828_800, 914_400)),
        preset_geometry: Some("rect".to_string()),
        adjustments: Vec::new(),
        custom_geometry: None,
        fill,
        outline: None,
        style: None,
        text: None,
        image_rel_id: None,
        image_part: None,
        source_rect: None,
        shadow: None,
    }
}

fn theme() -> ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="000000"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
    </a:clrScheme></a:themeElements></a:theme>"#;
    jade_pptx_reader::parse_theme_colors(xml).expect("theme")
}

fn slide_size() -> SlideSize {
    SlideSize {
        width_emu: 12_192_000,
        height_emu: 6_858_000,
    }
}

fn slide_with(shapes: Vec<ShapeItem>) -> SlideContent {
    SlideContent {
        shapes,
        ..SlideContent::default()
    }
}

fn stop(position_thousandths: u32, hex: &str) -> GradientStop {
    GradientStop {
        position_thousandths,
        color: Color::Srgb(hex.to_string()),
    }
}

fn gradient(angle_deg: Option<f32>) -> Fill {
    Fill::Gradient {
        stops: vec![
            stop(0, "FF0000"),
            stop(50_000, "00FF00"),
            stop(100_000, "0000FF"),
        ],
        angle_deg,
        path: None,
    }
}

#[test]
fn a_gradient_fill_places_its_stops_and_keeps_the_first_as_the_flat_fill() {
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(shape(
            "g",
            Some(gradient(Some(90.0))),
        ))]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    assert_eq!(placed.fill.as_deref(), Some("#FF0000"));
    let gradient = placed.gradient.as_ref().expect("gradient");
    assert_eq!(gradient.angle_deg, 90.0);
    let stops: Vec<(f64, &str)> = gradient
        .stops
        .iter()
        .map(|stop| (stop.position, stop.color.as_str()))
        .collect();
    assert_eq!(
        stops,
        vec![(0.0, "#FF0000"), (0.5, "#00FF00"), (1.0, "#0000FF")]
    );
}

#[test]
fn a_gradient_without_a_direction_runs_left_to_right() {
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(shape("g", Some(gradient(None))))]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(
        laid_out.shapes[0]
            .gradient
            .as_ref()
            .expect("gradient")
            .angle_deg,
        0.0
    );
    // A solid fill has no gradient.
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(shape(
            "s",
            Some(Fill::Solid(Color::Srgb("FF0000".into()))),
        ))]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].gradient, None);
}

#[test]
fn the_slide_background_gradient_is_placed_too() {
    let content = SlideContent {
        background: Some(gradient(Some(45.0))),
        ..slide_with(Vec::new())
    };
    let laid_out = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    assert_eq!(laid_out.background.as_deref(), Some("#FF0000"));
    let gradient = laid_out.background_gradient.as_ref().expect("gradient");
    assert_eq!(gradient.angle_deg, 45.0);
    assert_eq!(gradient.stops.len(), 3);
}

#[test]
fn a_fill_ref_takes_the_theme_fill_style_with_its_colour_in_place_of_ph_clr() {
    let styles = vec![
        Fill::Solid(Color::Scheme("phClr".into())),
        Fill::Gradient {
            stops: vec![
                GradientStop {
                    position_thousandths: 0,
                    color: Color::Adjusted {
                        base: Box::new(Color::Scheme("phClr".into())),
                        alpha: None,
                        lum_mod: None,
                        lum_off: None,
                        shade: None,
                        tint: Some(0.5),
                    },
                },
                GradientStop {
                    position_thousandths: 100_000,
                    color: Color::Scheme("phClr".into()),
                },
            ],
            angle_deg: Some(90.0),
            path: None,
        },
    ];
    let mut styled = shape("styled", None);
    styled.style = Some(ShapeStyle {
        fill_index: 2,
        fill_color: Some(Color::Scheme("accent1".into())),
        line_index: 0,
        line_color: None,
        font_color: None,
        effect_index: 0,
    });
    let mut flat = shape("flat", None);
    flat.style = Some(ShapeStyle {
        fill_index: 1,
        fill_color: Some(Color::Srgb("FF0000".into())),
        line_index: 0,
        line_color: None,
        font_color: None,
        effect_index: 0,
    });
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled), ShapeItem::Shape(flat)]),
        InheritedContent {
            theme_fill_styles: Some(&styles),
            ..InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );
    let gradient = laid_out.shapes[0]
        .gradient
        .as_ref()
        .expect("theme gradient");
    assert_eq!(gradient.angle_deg, 90.0);
    // accent1 tinted 50% toward white, then accent1 itself.
    assert_eq!(gradient.stops[0].color, "#A2B9E2");
    assert_eq!(gradient.stops[1].color, "#4472C4");
    assert_eq!(laid_out.shapes[0].fill.as_deref(), Some("#A2B9E2"));
    // Style 1 is a plain solid of the reference colour.
    assert_eq!(laid_out.shapes[1].fill.as_deref(), Some("#FF0000"));
    assert_eq!(laid_out.shapes[1].gradient, None);
}

#[test]
fn a_fill_ref_beyond_the_theme_styles_falls_back_to_a_solid_of_its_colour() {
    let mut styled = shape("styled", None);
    styled.style = Some(ShapeStyle {
        fill_index: 3,
        fill_color: Some(Color::Srgb("112233".into())),
        line_index: 0,
        line_color: None,
        font_color: None,
        effect_index: 0,
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].fill.as_deref(), Some("#112233"));
    assert_eq!(laid_out.shapes[0].gradient, None);
}

#[test]
fn custom_geometry_flattens_to_normalized_subpaths() {
    let mut freeform = shape("f", Some(Fill::Solid(Color::Srgb("00FF00".into()))));
    freeform.preset_geometry = None;
    freeform.custom_geometry = Some(CustomGeometry {
        paths: vec![
            GeometryPath {
                width: 200,
                height: 100,
                filled: true,
                stroked: true,
                commands: vec![
                    PathCommand::MoveTo(0, 0),
                    PathCommand::LineTo(200, 0),
                    PathCommand::CubicTo {
                        control1: (200, 50),
                        control2: (150, 100),
                        to: (100, 100),
                    },
                    PathCommand::QuadTo {
                        control: (50, 100),
                        to: (0, 50),
                    },
                    PathCommand::Close,
                ],
            },
            GeometryPath {
                width: 200,
                height: 100,
                filled: true,
                stroked: true,
                commands: vec![
                    PathCommand::MoveTo(20, 20),
                    PathCommand::LineTo(40, 20),
                    PathCommand::LineTo(40, 40),
                    PathCommand::Close,
                ],
            },
        ],
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(freeform)]),
        &theme(),
        &HeuristicMeasure,
    );
    let PlacedGeometry::Path(subpaths) = &laid_out.shapes[0].geometry else {
        panic!(
            "custom geometry is a path: {:?}",
            laid_out.shapes[0].geometry
        );
    };
    assert_eq!(subpaths.len(), 2);
    let first = &subpaths[0];
    assert_eq!(first[0], (0.0, 0.0));
    assert_eq!(first[1], (1.0, 0.0));
    // Curves are sampled: more than the four corner points, and every point
    // stays inside the path's own box.
    assert!(first.len() > 6, "{first:?}");
    assert!(first
        .iter()
        .all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)));
    // The cubic ends where it said it would, halfway across the bottom.
    assert!(first.contains(&(0.5, 1.0)), "{first:?}");
    assert!(first.contains(&(0.0, 0.5)), "{first:?}");
    assert_eq!(subpaths[1], vec![(0.1, 0.2), (0.2, 0.2), (0.2, 0.4)]);
    assert_eq!(laid_out.shapes[0].fill.as_deref(), Some("#00FF00"));
}

#[test]
fn an_arc_samples_along_its_ellipse() {
    // A quarter turn clockwise from the right of a circle of radius 50, in a
    // 100×100 path: from (100, 50) down to (50, 100).
    let mut arc = shape("a", None);
    arc.preset_geometry = None;
    arc.custom_geometry = Some(CustomGeometry {
        paths: vec![GeometryPath {
            width: 100,
            height: 100,
            filled: true,
            stroked: true,
            commands: vec![
                PathCommand::MoveTo(100, 50),
                PathCommand::ArcTo {
                    width_radius: 50,
                    height_radius: 50,
                    start_60000ths: 0,
                    sweep_60000ths: 5_400_000,
                },
                PathCommand::Close,
            ],
        }],
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(arc)]),
        &theme(),
        &HeuristicMeasure,
    );
    let PlacedGeometry::Path(subpaths) = &laid_out.shapes[0].geometry else {
        panic!("path");
    };
    let points = &subpaths[0];
    let last = points.last().expect("end of arc");
    assert!(
        (last.0 - 0.5).abs() < 1e-6 && (last.1 - 1.0).abs() < 1e-6,
        "{last:?}"
    );
    // Every sample is on the circle: 0.5 from the centre.
    for (x, y) in points {
        let distance = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
        assert!((distance - 0.5).abs() < 1e-6, "{x},{y}");
    }
}

#[test]
fn a_custom_geometry_without_paths_stays_a_rectangle() {
    let mut empty = shape("e", None);
    empty.preset_geometry = None;
    empty.custom_geometry = Some(CustomGeometry { paths: Vec::new() });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(empty)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].geometry, PlacedGeometry::Rect);
}
