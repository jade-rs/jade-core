//! From EMU shape tree to px geometry and placed text lines.

use jade_pptx_layout::{
    layout_slide, HeuristicMeasure, PlacedGeometry, ASCENT_RATIO, LINE_SPACING, PX_PER_PT,
    TEXT_INSET_X_EMU, TEXT_INSET_Y_EMU,
};
use jade_pptx_reader::{
    emu_to_px, Color, Fill, GroupShape, GroupTransform, Outline, Shape, ShapeItem, SlideContent,
    SlideSize, TextBody, TextParagraph, TextRun, ThemeColors, Transform,
};

fn shape(fill: Option<Fill>, transform: Transform) -> Shape {
    Shape {
        name: "s".to_string(),
        placeholder: None,
        transform: Some(transform),
        preset_geometry: Some("rect".to_string()),
        fill,
        outline: None,
        style: None,
        text: None,
        id: None,
        adjustments: Vec::new(),
        custom_geometry: None,
        image_rel_id: None,
        image_part: None,
        source_rect: None,
        shadow: None,
    }
}

fn transform(x: i64, y: i64, cx: i64, cy: i64) -> Transform {
    Transform {
        offset_emu: (x, y),
        extent_emu: (cx, cy),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

fn theme() -> ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="111111"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
    </a:clrScheme></a:themeElements></a:theme>"#;
    jade_pptx_reader::parse_theme_colors(xml).expect("theme")
}

fn slide_size() -> SlideSize {
    SlideSize {
        width_emu: 9_144_000,
        height_emu: 6_858_000,
    }
}

#[test]
fn a_shape_lands_at_its_emu_position_in_px() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(shape(
            Some(Fill::Solid(Color::Srgb("FF0000".to_string()))),
            transform(914_400, 457_200, 1_828_800, 914_400),
        ))],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    assert_eq!(layout.width_px, 960.0);
    assert_eq!(layout.height_px, 720.0);
    let placed = &layout.shapes[0];
    assert_eq!(placed.rect.x, 96.0);
    assert_eq!(placed.rect.y, 48.0);
    assert_eq!(placed.rect.width, 192.0);
    assert_eq!(placed.rect.height, 96.0);
    assert_eq!(placed.fill.as_deref(), Some("#FF0000"));
    assert_eq!(placed.geometry, PlacedGeometry::Rect);
}

#[test]
fn scheme_colors_resolve_through_the_theme() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(Shape {
            outline: Some(Outline {
                color: Some(Color::Scheme("accent1".to_string())),
                width_emu: Some(25_400),
                ..Outline::default()
            }),
            ..shape(
                Some(Fill::Solid(Color::Scheme("accent1".to_string()))),
                transform(0, 0, 914_400, 914_400),
            )
        })],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let placed = &layout.shapes[0];
    assert_eq!(placed.fill.as_deref(), Some("#4472C4"));
    let outline = placed.outline.as_ref().expect("outline");
    assert_eq!(outline.color, "#4472C4");
    // 25400 EMU is 2 points, 8/3 px.
    assert!((outline.width_px - emu_to_px(25_400)).abs() < 1e-9);
}

#[test]
fn an_explicit_no_fill_paints_nothing() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(shape(
            Some(Fill::None),
            transform(0, 0, 914_400, 914_400),
        ))],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    assert_eq!(layout.shapes[0].fill, None);
}

#[test]
fn rotation_converts_to_radians() {
    let mut rotated = shape(None, transform(0, 0, 914_400, 914_400));
    rotated.transform = Some(Transform {
        rotation_60000ths: 5_400_000,
        flip_h: true,
        ..rotated.transform.unwrap()
    });
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(rotated)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    let placed = &layout.shapes[0];
    assert!((placed.rotation_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert!(placed.flip_h);
}

#[test]
fn group_children_map_through_the_child_coordinate_space() {
    let child = shape(
        Some(Fill::Solid(Color::Srgb("00FF00".to_string()))),
        transform(100_000, 100_000, 500_000, 500_000),
    );
    let content = SlideContent {
        shapes: vec![ShapeItem::Group(GroupShape {
            name: "g".to_string(),
            transform: Some(GroupTransform {
                outer: transform(1_000_000, 2_000_000, 2_000_000, 2_000_000),
                child_offset_emu: (0, 0),
                child_extent_emu: (1_000_000, 1_000_000),
            }),
            children: vec![ShapeItem::Shape(child)],
            id: None,
            diagram_data_rel_id: None,
        })],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    // The group doubles child coordinates (ext / chExt = 2) and offsets them.
    let placed = &layout.shapes[0];
    assert!((placed.rect.x - emu_to_px(1_200_000)).abs() < 1e-9);
    assert!((placed.rect.y - emu_to_px(2_200_000)).abs() < 1e-9);
    assert!((placed.rect.width - emu_to_px(1_000_000)).abs() < 1e-9);
    assert!((placed.rect.height - emu_to_px(1_000_000)).abs() < 1e-9);
}

#[test]
fn text_wraps_to_the_box_width_and_lines_advance_by_the_font_size() {
    // Box 2 inches wide: 192 px minus two insets. The heuristic measure makes
    // an 18pt ASCII grapheme 12 px wide, so 14 fit on a line.
    let text = TextBody {
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![TextRun {
                text: "aaaaaaaaaa bbbbbbbbbb".to_string(),
                size_pt: Some(18.0),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 1_828_800, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let lines = &layout.shapes[0].lines;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].runs[0].text, "aaaaaaaaaa");
    assert_eq!(lines[1].runs[0].text, "bbbbbbbbbb");

    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    let inset_y = emu_to_px(TEXT_INSET_Y_EMU);
    let size_px = 18.0 * PX_PER_PT;
    let line_height = size_px * LINE_SPACING;
    assert!((lines[0].runs[0].x - inset_x).abs() < 1e-9);
    let first_baseline = inset_y + size_px * ASCENT_RATIO;
    assert!((lines[0].runs[0].baseline_y - first_baseline).abs() < 1e-9);
    assert!((lines[1].runs[0].baseline_y - (first_baseline + line_height)).abs() < 1e-9);

    // Text colour falls back to the theme's dark when the run names none.
    assert_eq!(lines[0].runs[0].color, "#111111");
    assert_eq!(lines[0].runs[0].size_pt, 18.0);
}

#[test]
fn runs_on_one_line_advance_past_each_other() {
    let text = TextBody {
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![
                TextRun {
                    text: "ab".to_string(),
                    size_pt: Some(18.0),
                    bold: true,
                    color: Some(Color::Srgb("112233".to_string())),
                    ..TextRun::default()
                },
                TextRun {
                    text: "cd".to_string(),
                    size_pt: Some(36.0),
                    ..TextRun::default()
                },
            ],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let line = &layout.shapes[0].lines[0];
    assert_eq!(line.runs.len(), 2);
    assert_eq!(line.runs[0].text, "ab");
    assert!(line.runs[0].bold);
    assert_eq!(line.runs[0].color, "#112233");
    assert_eq!(line.runs[1].text, "cd");
    // 18pt heuristic ASCII glyphs are 12 px; "ab" advances 24.
    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    assert!((line.runs[1].x - (inset_x + 24.0)).abs() < 1e-9);
}

#[test]
fn an_explicit_break_starts_a_new_line() {
    let text = TextBody {
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![
                TextRun {
                    text: "one".to_string(),
                    ..TextRun::default()
                },
                TextRun {
                    text: "\n".to_string(),
                    ..TextRun::default()
                },
                TextRun {
                    text: "two".to_string(),
                    ..TextRun::default()
                },
            ],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let lines = &layout.shapes[0].lines;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].runs[0].text, "one");
    assert_eq!(lines[1].runs[0].text, "two");
}

#[test]
fn preset_geometry_maps_to_the_placed_kinds() {
    let mut ellipse = shape(None, transform(0, 0, 914_400, 914_400));
    ellipse.preset_geometry = Some("ellipse".to_string());
    let mut round = shape(None, transform(0, 0, 914_400, 914_400));
    round.preset_geometry = Some("roundRect".to_string());
    let mut star = shape(None, transform(0, 0, 914_400, 914_400));
    star.preset_geometry = Some("star5".to_string());
    let mut unknown = shape(None, transform(0, 0, 914_400, 914_400));
    unknown.preset_geometry = Some("notAPreset".to_string());
    let content = SlideContent {
        shapes: vec![
            ShapeItem::Shape(ellipse),
            ShapeItem::Shape(round),
            ShapeItem::Shape(star),
            ShapeItem::Shape(unknown),
        ],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    assert_eq!(layout.shapes[0].geometry, PlacedGeometry::Ellipse);
    assert_eq!(
        layout.shapes[1].geometry,
        PlacedGeometry::RoundRect { adjust: 0.16667 }
    );
    // Every ECMA-376 preset evaluates from its definition: star5's ten
    // points start at its left arm and peak at the top centre.
    let PlacedGeometry::Paths(paths) = &layout.shapes[2].geometry else {
        panic!("star5 should evaluate to paths");
    };
    let points = &paths[0].subpaths[0].points;
    assert_eq!(points.len(), 10);
    assert!(paths[0].subpaths[0].closed);
    assert!((points[2].0 - 0.5).abs() < 1e-9 && points[2].1.abs() < 1e-9);
    // Anything unmodelled still shows as its bounding rectangle.
    assert_eq!(layout.shapes[3].geometry, PlacedGeometry::Rect);
}

#[test]
fn a_gradient_fill_falls_back_to_its_first_stop() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(shape(
            Some(Fill::Gradient {
                angle_deg: None,
                stops: vec![
                    jade_pptx_reader::GradientStop {
                        position_thousandths: 0,
                        color: Color::Srgb("FF0000".to_string()),
                    },
                    jade_pptx_reader::GradientStop {
                        position_thousandths: 100_000,
                        color: Color::Scheme("accent1".to_string()),
                    },
                ],
                path: None,
            }),
            transform(0, 0, 914_400, 914_400),
        ))],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    // Until the painter grows gradients, the first stop is the closest colour.
    assert_eq!(layout.shapes[0].fill.as_deref(), Some("#FF0000"));
}

#[test]
fn a_centre_anchored_body_centres_its_lines_vertically() {
    let text = TextBody {
        anchor: Some(jade_pptx_reader::TextAnchor::Center),
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![TextRun {
                text: "hi".to_string(),
                size_pt: Some(18.0),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let size_px = 18.0 * PX_PER_PT;
    let inset_y = emu_to_px(TEXT_INSET_Y_EMU);
    // One 21.6px-tall line block inside a 96px box: the slack above equals
    // half of (available height minus the line's advance height).
    let available = 96.0 - 2.0 * inset_y;
    let content_height = size_px * LINE_SPACING;
    let expected_baseline = inset_y + (available - content_height) / 2.0 + size_px * ASCENT_RATIO;
    let run = &layout.shapes[0].lines[0].runs[0];
    assert!(
        (run.baseline_y - expected_baseline).abs() < 1e-9,
        "baseline {} != {}",
        run.baseline_y,
        expected_baseline
    );
}

#[test]
fn a_bottom_anchored_body_sits_on_the_bottom_inset() {
    let text = TextBody {
        anchor: Some(jade_pptx_reader::TextAnchor::Bottom),
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![TextRun {
                text: "hi".to_string(),
                size_pt: Some(18.0),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let size_px = 18.0 * PX_PER_PT;
    let inset_y = emu_to_px(TEXT_INSET_Y_EMU);
    let content_height = size_px * LINE_SPACING;
    let expected_baseline = 96.0 - inset_y - content_height + size_px * ASCENT_RATIO;
    let run = &layout.shapes[0].lines[0].runs[0];
    assert!(
        (run.baseline_y - expected_baseline).abs() < 1e-9,
        "baseline {} != {}",
        run.baseline_y,
        expected_baseline
    );
}

#[test]
fn custom_body_insets_replace_the_defaults() {
    let text = TextBody {
        insets: jade_pptx_reader::TextInsets {
            left_emu: Some(182_880),
            top_emu: Some(91_440),
            right_emu: None,
            bottom_emu: None,
        },
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![TextRun {
                text: "hi".to_string(),
                size_pt: Some(18.0),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    };
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text);
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let size_px = 18.0 * PX_PER_PT;
    let run = &layout.shapes[0].lines[0].runs[0];
    assert!((run.x - emu_to_px(182_880)).abs() < 1e-9);
    let expected_baseline = emu_to_px(91_440) + size_px * ASCENT_RATIO;
    assert!((run.baseline_y - expected_baseline).abs() < 1e-9);
}

#[test]
fn the_slide_background_resolves_through_the_theme() {
    let content = SlideContent {
        background: Some(Fill::Solid(Color::Scheme("accent1".to_string()))),
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    assert_eq!(layout.background.as_deref(), Some("#4472C4"));

    let plain = layout_slide(
        slide_size(),
        &SlideContent::default(),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(plain.background, None);
}

#[test]
fn a_line_connector_lays_out_as_a_line() {
    let mut line = shape(None, transform(0, 0, 914_400, 457_200));
    line.preset_geometry = Some("line".to_string());
    let mut connector = shape(None, transform(0, 0, 914_400, 457_200));
    connector.preset_geometry = Some("straightConnector1".to_string());
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(line), ShapeItem::Shape(connector)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);
    assert_eq!(layout.shapes[0].geometry, PlacedGeometry::Line);
    assert_eq!(layout.shapes[1].geometry, PlacedGeometry::Line);
}

#[test]
fn polygon_presets_map_to_normalized_points() {
    let mut triangle = shape(None, transform(0, 0, 914_400, 914_400));
    triangle.preset_geometry = Some("triangle".to_string());
    let mut diamond = shape(None, transform(0, 0, 914_400, 914_400));
    diamond.preset_geometry = Some("diamond".to_string());
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(triangle), ShapeItem::Shape(diamond)],
        ..SlideContent::default()
    };
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let PlacedGeometry::Polygon(points) = &layout.shapes[0].geometry else {
        panic!("expected polygon, got {:?}", layout.shapes[0].geometry);
    };
    // Apex top-centre, base along the bottom, in normalized shape coordinates.
    assert_eq!(points.as_slice(), &[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]);

    let PlacedGeometry::Polygon(points) = &layout.shapes[1].geometry else {
        panic!("expected polygon");
    };
    assert_eq!(
        points.as_slice(),
        &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]
    );
}

fn placeholder_shape(kind: &str, index: Option<u32>, with_transform: bool) -> Shape {
    Shape {
        placeholder: Some(jade_pptx_reader::Placeholder {
            kind: kind.to_string(),
            index,
        }),
        transform: with_transform.then(|| transform(838_200, 365_125, 10_515_600, 1_325_563)),
        ..shape(None, transform(0, 0, 0, 0))
    }
}

#[test]
fn a_placeholder_without_geometry_inherits_it_from_the_layout() {
    let mut on_slide = placeholder_shape("title", None, false);
    on_slide.text = Some(TextBody {
        paragraphs: vec![TextParagraph {
            level: 0,
            align: None,
            bullet: None,
            runs: vec![TextRun {
                text: "Inherited Title".to_string(),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    });
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(on_slide)],
        ..SlideContent::default()
    };
    let layout_content = SlideContent {
        shapes: vec![ShapeItem::Shape(placeholder_shape("title", None, true))],
        ..SlideContent::default()
    };

    let laid_out = jade_pptx_layout::layout_slide_inheriting(
        slide_size(),
        &content,
        jade_pptx_layout::InheritedContent {
            layout: Some(&layout_content),
            master: None,
            ..jade_pptx_layout::InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );

    assert_eq!(laid_out.shapes.len(), 1);
    let placed = &laid_out.shapes[0];
    assert!((placed.rect.x - emu_to_px(838_200)).abs() < 1e-9);
    assert!((placed.rect.y - emu_to_px(365_125)).abs() < 1e-9);
    assert!((placed.rect.width - emu_to_px(10_515_600)).abs() < 1e-9);
    assert_eq!(placed.lines[0].runs[0].text, "Inherited Title");
}

#[test]
fn a_matching_layout_placeholder_inherits_the_master_frame_by_type() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(placeholder_shape("body", Some(1), false))],
        ..SlideContent::default()
    };
    // The matching layout body omits its frame; master idx is independent.
    let layout_content = SlideContent {
        shapes: vec![ShapeItem::Shape(placeholder_shape("body", Some(1), false))],
        ..SlideContent::default()
    };
    let master_content = SlideContent {
        shapes: vec![ShapeItem::Shape(placeholder_shape("body", Some(41), true))],
        ..SlideContent::default()
    };

    let laid_out = jade_pptx_layout::layout_slide_inheriting(
        slide_size(),
        &content,
        jade_pptx_layout::InheritedContent {
            layout: Some(&layout_content),
            master: Some(&master_content),
            ..jade_pptx_layout::InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );

    assert_eq!(laid_out.shapes.len(), 1);
    assert!((laid_out.shapes[0].rect.x - emu_to_px(838_200)).abs() < 1e-9);
}

#[test]
fn a_shape_with_neither_geometry_nor_a_match_is_still_skipped() {
    let content = SlideContent {
        shapes: vec![ShapeItem::Shape(placeholder_shape("body", Some(7), false))],
        ..SlideContent::default()
    };
    let laid_out = jade_pptx_layout::layout_slide_inheriting(
        slide_size(),
        &content,
        jade_pptx_layout::InheritedContent {
            layout: None,
            master: None,
            ..jade_pptx_layout::InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );
    assert!(laid_out.shapes.is_empty());
}

fn bulleted_body(paragraphs: Vec<TextParagraph>) -> SlideContent {
    let mut boxed = shape(None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(TextBody {
        paragraphs,
        ..TextBody::default()
    });
    SlideContent {
        shapes: vec![ShapeItem::Shape(boxed)],
        ..SlideContent::default()
    }
}

fn bullet_paragraph(
    level: u8,
    bullet: Option<jade_pptx_reader::Bullet>,
    text: &str,
) -> TextParagraph {
    TextParagraph {
        level,
        align: None,
        bullet,
        runs: vec![TextRun {
            text: text.to_string(),
            size_pt: Some(18.0),
            ..TextRun::default()
        }],
        ..TextParagraph::default()
    }
}

#[test]
fn a_char_bullet_gets_its_own_run_and_the_text_indents_past_it() {
    let content = bulleted_body(vec![bullet_paragraph(
        0,
        Some(jade_pptx_reader::Bullet::Char("\u{2022}".to_string())),
        "first",
    )]);
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let line = &layout.shapes[0].lines[0];
    assert_eq!(line.runs.len(), 2);
    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    let slot = emu_to_px(jade_pptx_layout::LEVEL_INDENT_EMU);
    assert_eq!(line.runs[0].text, "\u{2022}");
    assert!((line.runs[0].x - inset_x).abs() < 1e-9);
    assert_eq!(line.runs[1].text, "first");
    assert!((line.runs[1].x - (inset_x + slot)).abs() < 1e-9);
}

#[test]
fn deeper_levels_indent_and_auto_numbers_count_per_level() {
    let content = bulleted_body(vec![
        bullet_paragraph(
            0,
            Some(jade_pptx_reader::Bullet::AutoNumber {
                scheme: "arabicPeriod".to_string(),
            }),
            "one",
        ),
        bullet_paragraph(
            0,
            Some(jade_pptx_reader::Bullet::AutoNumber {
                scheme: "arabicPeriod".to_string(),
            }),
            "two",
        ),
        bullet_paragraph(
            1,
            Some(jade_pptx_reader::Bullet::Char("-".to_string())),
            "nested",
        ),
    ]);
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let lines = &layout.shapes[0].lines;
    assert_eq!(lines[0].runs[0].text, "1.");
    assert_eq!(lines[1].runs[0].text, "2.");

    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    let slot = emu_to_px(jade_pptx_layout::LEVEL_INDENT_EMU);
    // The nested bullet sits one level in; its text one slot further.
    assert!((lines[2].runs[0].x - (inset_x + slot)).abs() < 1e-9);
    assert!((lines[2].runs[1].x - (inset_x + 2.0 * slot)).abs() < 1e-9);
}

#[test]
fn an_explicit_bu_none_indents_without_a_marker() {
    let content = bulleted_body(vec![bullet_paragraph(
        1,
        Some(jade_pptx_reader::Bullet::None),
        "plain",
    )]);
    let layout = layout_slide(slide_size(), &content, &theme(), &HeuristicMeasure);

    let line = &layout.shapes[0].lines[0];
    assert_eq!(line.runs.len(), 1);
    assert_eq!(line.runs[0].text, "plain");
    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    let slot = emu_to_px(jade_pptx_layout::LEVEL_INDENT_EMU);
    assert!((line.runs[0].x - (inset_x + slot)).abs() < 1e-9);
}
