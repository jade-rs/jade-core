//! The fourth layout slice: preset adjustments, picture placeholders,
//! hyperlink styling, numbering schemes, capitalisation, colour map
//! overrides, and run widths for horizontal overflow.

use std::collections::HashMap;

use jade_pptx_layout::{
    format_number, layout_slide, layout_slide_inheriting, HeuristicMeasure, InheritedContent,
    PlacedGeometry, PX_PER_PT,
};
use jade_pptx_reader::{
    Bullet, Color, Fill, Hyperlink, Picture, Placeholder, Shape, ShapeItem, SlideContent,
    SlideSize, TextBody, TextParagraph, TextRun, ThemeColors, Transform,
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

fn shape(name: &str, transform: Transform) -> Shape {
    Shape {
        id: None,
        name: name.to_string(),
        placeholder: None,
        transform: Some(transform),
        preset_geometry: Some("rect".to_string()),
        adjustments: Vec::new(),
        fill: None,
        outline: None,
        style: None,
        text: None,
        custom_geometry: None,
        image_rel_id: None,
        image_part: None,
        source_rect: None,
    }
}

fn paragraph(text: &str) -> TextParagraph {
    TextParagraph {
        runs: vec![TextRun {
            text: text.to_string(),
            size_pt: Some(18.0),
            ..TextRun::default()
        }],
        ..TextParagraph::default()
    }
}

fn theme() -> ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="111111"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
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

#[test]
fn a_round_rect_carries_its_adjustment_as_a_fraction() {
    let mut rounded = shape("r", transform(0, 0, 914_400, 914_400));
    rounded.preset_geometry = Some("roundRect".into());
    rounded.adjustments = vec![("adj".into(), 5_000)];
    let mut default = shape("d", transform(0, 0, 914_400, 914_400));
    default.preset_geometry = Some("roundRect".into());
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(rounded), ShapeItem::Shape(default)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(
        laid_out.shapes[0].geometry,
        PlacedGeometry::RoundRect { adjust: 0.05 }
    );
    assert_eq!(
        laid_out.shapes[1].geometry,
        PlacedGeometry::RoundRect { adjust: 0.16667 }
    );
}

#[test]
fn an_out_of_range_adjustment_is_clamped_to_half_the_side() {
    let mut rounded = shape("r", transform(0, 0, 914_400, 914_400));
    rounded.preset_geometry = Some("roundRect".into());
    rounded.adjustments = vec![("adj".into(), 90_000)];
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(rounded)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(
        laid_out.shapes[0].geometry,
        PlacedGeometry::RoundRect { adjust: 0.5 }
    );
}

#[test]
fn a_picture_placeholder_inherits_its_geometry_from_the_layout() {
    let placeholder = Placeholder {
        kind: "pic".into(),
        index: Some(13),
    };
    let layout_picture = Shape {
        placeholder: Some(placeholder.clone()),
        ..shape(
            "Picture Placeholder",
            transform(457_200, 914_400, 3_657_600, 2_743_200),
        )
    };
    let layout = slide_with(vec![ShapeItem::Shape(layout_picture)]);
    let picture = Picture {
        id: Some(3),
        name: "Picture 2".into(),
        placeholder: Some(placeholder),
        transform: None,
        blip_rel_id: Some("rId9".into()),
        image_part: Some("ppt/media/image1.png".into()),
        source_rect: None,
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Picture(picture)]),
        InheritedContent {
            layout: Some(&layout),
            ..InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );
    // The layout's placeholder is not drawn itself; the slide's picture lands
    // on its geometry.
    assert_eq!(laid_out.shapes.len(), 1);
    let placed = &laid_out.shapes[0];
    assert_eq!(placed.name, "Picture 2");
    assert!((placed.rect.x - 48.0).abs() < 1e-9);
    assert!((placed.rect.width - 384.0).abs() < 1e-9);
    assert_eq!(placed.image_part.as_deref(), Some("ppt/media/image1.png"));
}

#[test]
fn a_hyperlinked_run_takes_the_theme_link_colour_and_an_underline() {
    let mut boxed = shape("box", transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(TextBody {
        paragraphs: vec![TextParagraph {
            runs: vec![
                TextRun {
                    text: "see ".into(),
                    color: Some(Color::Srgb("FF0000".into())),
                    ..TextRun::default()
                },
                TextRun {
                    text: "the docs".into(),
                    hyperlink: Some(Hyperlink {
                        rel_id: "rId3".into(),
                        target: Some("https://example.com".into()),
                    }),
                    ..TextRun::default()
                },
            ],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let runs = &laid_out.shapes[0].lines[0].runs;
    assert_eq!(runs[0].color, "#FF0000");
    assert!(!runs[0].underline);
    assert_eq!(runs[1].color, "#0563C1");
    assert!(runs[1].underline);
    assert_eq!(runs[1].hyperlink.as_deref(), Some("https://example.com"));
}

#[test]
fn a_hyperlink_without_a_theme_link_colour_keeps_the_runs_own() {
    let mut boxed = shape("box", transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(TextBody {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text: "link".into(),
                color: Some(Color::Srgb("FF0000".into())),
                hyperlink: Some(Hyperlink {
                    rel_id: "rId3".into(),
                    target: None,
                }),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &ThemeColors::default(),
        &HeuristicMeasure,
    );
    let run = &laid_out.shapes[0].lines[0].runs[0];
    assert_eq!(run.color, "#FF0000");
    assert!(run.underline);
}

#[test]
fn numbering_schemes_format_their_numerals_and_punctuation() {
    assert_eq!(format_number(1, "arabicPeriod"), "1.");
    assert_eq!(format_number(3, "arabicParenR"), "3)");
    assert_eq!(format_number(2, "arabicParenBoth"), "(2)");
    assert_eq!(format_number(4, "arabicPlain"), "4");
    assert_eq!(format_number(1, "alphaLcPeriod"), "a.");
    assert_eq!(format_number(27, "alphaUcParenR"), "AA)");
    assert_eq!(format_number(4, "romanLcPeriod"), "iv.");
    assert_eq!(format_number(1999, "romanUcParenBoth"), "(MCMXCIX)");
    assert_eq!(format_number(2, "ea1JpnKorPeriod"), "2.");
}

#[test]
fn auto_numbers_follow_their_scheme_and_start_value() {
    let numbered = |scheme: &str, start: Option<u32>| TextParagraph {
        bullet: Some(Bullet::AutoNumber {
            scheme: scheme.into(),
        }),
        number_start: start,
        ..paragraph("item")
    };
    let mut boxed = shape("box", transform(0, 0, 9_144_000, 3_000_000));
    boxed.text = Some(TextBody {
        paragraphs: vec![
            numbered("alphaLcParenR", Some(3)),
            numbered("alphaLcParenR", None),
            numbered("alphaLcParenR", None),
        ],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let markers: Vec<&str> = laid_out.shapes[0]
        .lines
        .iter()
        .map(|line| line.runs[0].text.as_str())
        .collect();
    assert_eq!(markers, vec!["c)", "d)", "e)"]);
}

#[test]
fn an_all_caps_run_lays_out_in_capitals() {
    let mut boxed = shape("box", transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(TextBody {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text: "shout".into(),
                all_caps: true,
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].lines[0].runs[0].text, "SHOUT");
}

#[test]
fn a_slide_colour_map_override_redirects_the_text_colour() {
    let mut boxed = shape("box", transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(TextBody {
        paragraphs: vec![paragraph("inverted")],
        ..TextBody::default()
    });
    boxed.fill = Some(Fill::Solid(Color::Scheme("bg1".into())));
    let mut map = HashMap::new();
    map.insert("bg1".to_string(), "dk1".to_string());
    map.insert("tx1".to_string(), "lt1".to_string());
    let content = SlideContent {
        color_map_override: Some(map),
        ..slide_with(vec![ShapeItem::Shape(boxed)])
    };
    let laid_out = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    // tx1 now maps to the light colour, bg1 to the dark one.
    assert_eq!(laid_out.shapes[0].lines[0].runs[0].color, "#FFFFFF");
    assert_eq!(laid_out.shapes[0].fill.as_deref(), Some("#111111"));
}

#[test]
fn runs_carry_their_measured_width_and_unwrapped_text_reports_right_overflow() {
    let mut boxed = shape("box", transform(0, 0, 914_400, 914_400));
    boxed.text = Some(TextBody {
        wrap: false,
        paragraphs: vec![paragraph(&"a".repeat(20))],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    let run = &placed.lines[0].runs[0];
    let width = 20.0 * 18.0 * PX_PER_PT * 0.5;
    assert!((run.width - width).abs() < 1e-9);
    let overflow = run.x + width - 96.0;
    assert!(overflow > 0.0);
    assert!((placed.text_overflow_right_px() - overflow).abs() < 1e-9);

    let mut wrapped = shape("box", transform(0, 0, 914_400, 914_400));
    wrapped.text = Some(TextBody {
        paragraphs: vec![paragraph(&"a ".repeat(20))],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(wrapped)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].text_overflow_right_px(), 0.0);
}
