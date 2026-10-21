//! What PowerPoint-authored decks need from layout beyond the first slice:
//! master and layout shapes under the slide, style references, colour
//! transforms, inherited text styles, paragraph spacing, autofit, and tables.

use jade_pptx_layout::{
    layout_deck_slide, layout_slide, layout_slide_inheriting, HeuristicMeasure, InheritedContent,
    PlacedGeometry, PlacedOrigin, ASCENT_RATIO, DEFAULT_SIZE_PT, LINE_SPACING, PX_PER_PT,
    TEXT_INSET_X_EMU, TEXT_INSET_Y_EMU,
};
use jade_pptx_reader::{
    emu_to_px, Autofit, Bullet, CellBorders, Color, Deck, DeckSlide, Fill, LevelStyle, ListStyle,
    Outline, Placeholder, RunProps, Shape, ShapeItem, ShapeStyle, SlideContent, SlideSize, Spacing,
    Table, TableCell, TableRow, TextAlign, TextAnchor, TextBody, TextParagraph, TextRun,
    TextStyles, ThemeColors, ThemeFonts, Transform,
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

fn shape(name: &str, fill: Option<Fill>, transform: Transform) -> Shape {
    Shape {
        name: name.to_string(),
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
        effects: Default::default(),
    }
}

fn text_of(paragraphs: Vec<TextParagraph>) -> TextBody {
    TextBody {
        paragraphs,
        ..TextBody::default()
    }
}

fn paragraph(text: &str) -> TextParagraph {
    TextParagraph {
        runs: vec![TextRun {
            text: text.to_string(),
            ..TextRun::default()
        }],
        ..TextParagraph::default()
    }
}

fn sized_paragraph(text: &str, size_pt: f32) -> TextParagraph {
    TextParagraph {
        runs: vec![TextRun {
            text: text.to_string(),
            size_pt: Some(size_pt),
            ..TextRun::default()
        }],
        ..TextParagraph::default()
    }
}

fn theme() -> ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="111111"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
    </a:clrScheme></a:themeElements></a:theme>"#;
    jade_pptx_reader::parse_theme_colors(xml).expect("theme")
}

fn fonts() -> ThemeFonts {
    ThemeFonts {
        major_latin: Some("Calibri Light".into()),
        minor_latin: Some("Calibri".into()),
        major_east_asian: Some("Yu Gothic Light".into()),
        minor_east_asian: Some("Yu Gothic".into()),
    }
}

fn slide_size() -> SlideSize {
    SlideSize {
        width_emu: 12_192_000,
        height_emu: 6_858_000,
    }
}

fn level(size_pt: Option<f32>) -> LevelStyle {
    LevelStyle {
        default_run: RunProps {
            size_pt,
            ..RunProps::default()
        },
        ..LevelStyle::default()
    }
}

fn list_style(levels: Vec<Option<LevelStyle>>) -> ListStyle {
    ListStyle { levels }
}

fn master_with_styles() -> SlideContent {
    let title_placeholder = Shape {
        placeholder: Some(Placeholder {
            kind: "title".into(),
            index: None,
        }),
        text: Some(TextBody {
            anchor: Some(TextAnchor::Center),
            list_style: None,
            ..text_of(vec![paragraph("Click to edit")])
        }),
        ..shape(
            "Title Placeholder",
            None,
            transform(838_200, 365_125, 10_515_600, 1_325_563),
        )
    };
    let body_placeholder = Shape {
        placeholder: Some(Placeholder {
            kind: "body".into(),
            index: Some(1),
        }),
        text: Some(TextBody {
            list_style: Some(list_style(vec![Some(LevelStyle {
                default_run: RunProps {
                    bold: Some(true),
                    ..RunProps::default()
                },
                ..LevelStyle::default()
            })])),
            ..text_of(vec![paragraph("Click to edit")])
        }),
        ..shape(
            "Body Placeholder",
            None,
            transform(838_200, 1_825_625, 10_515_600, 4_351_338),
        )
    };
    let logo = shape(
        "Logo",
        Some(Fill::Solid(Color::Srgb("ABCDEF".into()))),
        transform(0, 0, 914_400, 914_400),
    );
    SlideContent {
        background: Some(Fill::Solid(Color::Scheme("bg2".into()))),
        shapes: vec![
            ShapeItem::Shape(title_placeholder),
            ShapeItem::Shape(body_placeholder),
            ShapeItem::Shape(logo),
        ],
        background_ref: None,
        show_master_shapes: true,
        text_styles: Some(TextStyles {
            title: list_style(vec![Some(LevelStyle {
                align: Some(TextAlign::Left),
                line_spacing: Some(Spacing::Percent(0.9)),
                bullet: Some(Bullet::None),
                default_run: RunProps {
                    size_pt: Some(44.0),
                    color: Some(Color::Scheme("tx1".into())),
                    font: Some("+mj-lt".into()),
                    east_asian_font: Some("+mj-ea".into()),
                    ..RunProps::default()
                },
                ..LevelStyle::default()
            })]),
            body: list_style(vec![
                Some(LevelStyle {
                    margin_left_emu: Some(228_600),
                    indent_emu: Some(-228_600),
                    bullet: Some(Bullet::Char("•".into())),
                    bullet_font: Some("Arial".into()),
                    space_before: Some(Spacing::Points(10.0)),
                    default_run: RunProps {
                        size_pt: Some(28.0),
                        font: Some("+mn-lt".into()),
                        ..RunProps::default()
                    },
                    ..LevelStyle::default()
                }),
                Some(LevelStyle {
                    margin_left_emu: Some(685_800),
                    indent_emu: Some(-228_600),
                    bullet: Some(Bullet::Char("–".into())),
                    default_run: RunProps {
                        size_pt: Some(24.0),
                        ..RunProps::default()
                    },
                    ..LevelStyle::default()
                }),
            ]),
            other: list_style(vec![Some(level(Some(14.0)))]),
        }),
        color_map_override: None,
        use_master_color_map: false,
        hidden: false,
    }
}

fn layout_with_placeholders() -> SlideContent {
    let title = Shape {
        placeholder: Some(Placeholder {
            kind: "title".into(),
            index: None,
        }),
        transform: None,
        text: Some(TextBody {
            list_style: Some(list_style(vec![Some(LevelStyle {
                align: Some(TextAlign::Center),
                default_run: RunProps {
                    size_pt: Some(40.0),
                    ..RunProps::default()
                },
                ..LevelStyle::default()
            })])),
            ..text_of(vec![paragraph("Click to edit")])
        }),
        ..shape("Layout Title", None, transform(0, 0, 0, 0))
    };
    let content = Shape {
        placeholder: Some(Placeholder {
            kind: String::new(),
            index: Some(1),
        }),
        transform: None,
        text: Some(text_of(vec![paragraph("Click to edit")])),
        ..shape("Layout Content", None, transform(0, 0, 0, 0))
    };
    let decoration = shape(
        "Layout band",
        Some(Fill::Solid(Color::Srgb("123456".into()))),
        transform(0, 6_000_000, 12_192_000, 858_000),
    );
    SlideContent {
        background: None,
        shapes: vec![
            ShapeItem::Shape(title),
            ShapeItem::Shape(content),
            ShapeItem::Shape(decoration),
        ],
        show_master_shapes: true,
        text_styles: None,
        color_map_override: None,
        use_master_color_map: false,
        hidden: false,
        background_ref: None,
    }
}

fn inherited<'a>(
    layout: &'a SlideContent,
    master: &'a SlideContent,
    fonts: &'a ThemeFonts,
    default_text_style: &'a ListStyle,
) -> InheritedContent<'a> {
    InheritedContent {
        layout: Some(layout),
        master: Some(master),
        theme_fonts: Some(fonts),
        default_text_style: Some(default_text_style),
        table_styles: None,
        theme_fill_styles: None,
        theme_background_fill_styles: None,
        theme_line_styles: None,
        theme_effect_styles: None,
        theme_effect_style_effects: None,
    }
}

fn slide_with(shapes: Vec<ShapeItem>) -> SlideContent {
    SlideContent {
        shapes,
        ..SlideContent::default()
    }
}

#[test]
fn master_and_layout_shapes_draw_under_the_slide_and_placeholders_do_not() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let slide = slide_with(vec![ShapeItem::Shape(shape(
        "On slide",
        None,
        transform(0, 0, 914_400, 914_400),
    ))]);

    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide,
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let names: Vec<(&str, PlacedOrigin)> = laid_out
        .shapes
        .iter()
        .map(|shape| (shape.name.as_str(), shape.origin))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Logo", PlacedOrigin::Master),
            ("Layout band", PlacedOrigin::Layout),
            ("On slide", PlacedOrigin::Slide),
        ]
    );
    // The slide has no background of its own; the master's shows through.
    assert_eq!(laid_out.background.as_deref(), Some("#E7E6E6"));
}

#[test]
fn a_slide_that_hides_master_shapes_draws_neither_master_nor_layout_shapes() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let slide = SlideContent {
        show_master_shapes: false,
        ..slide_with(vec![ShapeItem::Shape(shape(
            "On slide",
            None,
            transform(0, 0, 914_400, 914_400),
        ))])
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide,
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes.len(), 1);
    assert_eq!(laid_out.shapes[0].name, "On slide");
}

#[test]
fn a_layout_that_hides_master_shapes_still_draws_its_own() {
    let master = master_with_styles();
    let layout = SlideContent {
        show_master_shapes: false,
        ..layout_with_placeholders()
    };
    let fonts = fonts();
    let defaults = ListStyle::default();
    let slide = slide_with(Vec::new());
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide,
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let names: Vec<&str> = laid_out.shapes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Layout band"]);
}

#[test]
fn the_layout_background_beats_the_master_background() {
    let master = master_with_styles();
    let layout = SlideContent {
        background: Some(Fill::Solid(Color::Srgb("0000FF".into()))),
        ..layout_with_placeholders()
    };
    let fonts = fonts();
    let defaults = ListStyle::default();
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(Vec::new()),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.background.as_deref(), Some("#0000FF"));
}

#[test]
fn a_style_reference_supplies_fill_and_line_when_the_shape_states_none() {
    let mut styled = shape("Styled", None, transform(0, 0, 914_400, 914_400));
    styled.style = Some(ShapeStyle {
        fill_index: 1,
        fill_color: Some(Color::Scheme("accent1".into())),
        line_index: 2,
        line_color: Some(Color::Scheme("accent5".into())),
        font_color: Some(Color::Scheme("lt1".into())),
        effect_index: 0,
    });
    styled.text = Some(text_of(vec![paragraph("styled text")]));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled)]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    assert_eq!(placed.fill.as_deref(), Some("#4472C4"));
    let outline = placed.outline.as_ref().expect("lnRef outline");
    assert_eq!(outline.color, "#5B9BD5");
    // Theme line style 2 is 1pt wide in the Office theme.
    assert!((outline.width_px - emu_to_px(12_700)).abs() < 1e-9);
    assert_eq!(placed.lines[0].runs[0].color, "#FFFFFF");
}

#[test]
fn explicit_no_fill_and_no_line_beat_the_style_reference() {
    let mut styled = shape(
        "Styled",
        Some(Fill::None),
        transform(0, 0, 914_400, 914_400),
    );
    styled.style = Some(ShapeStyle {
        fill_index: 1,
        fill_color: Some(Color::Scheme("accent1".into())),
        line_index: 2,
        line_color: Some(Color::Scheme("accent5".into())),
        font_color: None,
        effect_index: 0,
    });
    styled.outline = Some(Outline {
        no_fill: true,
        ..Outline::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].fill, None);
    assert_eq!(laid_out.shapes[0].outline, None);
}

#[test]
fn a_style_with_index_zero_supplies_nothing() {
    let mut styled = shape("Styled", None, transform(0, 0, 914_400, 914_400));
    styled.style = Some(ShapeStyle {
        fill_index: 0,
        fill_color: Some(Color::Scheme("accent1".into())),
        line_index: 0,
        line_color: Some(Color::Scheme("accent5".into())),
        font_color: None,
        effect_index: 0,
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].fill, None);
    assert_eq!(laid_out.shapes[0].outline, None);
}

#[test]
fn an_explicit_outline_color_keeps_its_own_width_over_the_style() {
    let mut styled = shape("Styled", None, transform(0, 0, 914_400, 914_400));
    styled.style = Some(ShapeStyle {
        fill_index: 0,
        fill_color: None,
        line_index: 2,
        line_color: Some(Color::Scheme("accent5".into())),
        font_color: None,
        effect_index: 0,
    });
    styled.outline = Some(Outline {
        color: Some(Color::Srgb("FF0000".into())),
        width_emu: Some(19_050),
        dash: Some("dash".into()),
        no_fill: false,
        head_end: None,
        tail_end: None,
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(styled)]),
        &theme(),
        &HeuristicMeasure,
    );
    let outline = laid_out.shapes[0].outline.as_ref().expect("outline");
    assert_eq!(outline.color, "#FF0000");
    assert!((outline.width_px - emu_to_px(19_050)).abs() < 1e-9);
    // A dash pattern is in px, proportional to the line width, so it scales
    // with the stroke the way PowerPoint draws it.
    let width = outline.width_px;
    assert_eq!(outline.dash_px, vec![4.0 * width, 3.0 * width]);
}

#[test]
fn dash_presets_map_to_px_patterns() {
    let with_dash = |dash: &str| {
        let mut s = shape("d", None, transform(0, 0, 914_400, 914_400));
        s.outline = Some(Outline {
            color: Some(Color::Srgb("000000".into())),
            width_emu: Some(12_700),
            dash: Some(dash.into()),
            no_fill: false,
            head_end: None,
            tail_end: None,
        });
        let laid_out = layout_slide(
            slide_size(),
            &slide_with(vec![ShapeItem::Shape(s)]),
            &theme(),
            &HeuristicMeasure,
        );
        laid_out.shapes[0].outline.clone().expect("outline").dash_px
    };
    let w = emu_to_px(12_700);
    assert_eq!(with_dash("dot"), vec![w, 3.0 * w]);
    assert_eq!(with_dash("sysDot"), vec![w, w]);
    assert_eq!(with_dash("sysDash"), vec![3.0 * w, w]);
    assert_eq!(with_dash("lgDash"), vec![8.0 * w, 3.0 * w]);
    assert_eq!(with_dash("dashDot"), vec![4.0 * w, 3.0 * w, w, 3.0 * w]);
    assert_eq!(with_dash("solid"), Vec::<f64>::new());
    assert_eq!(with_dash("something-else"), Vec::<f64>::new());
}

#[test]
fn color_transforms_resolve_to_css() {
    let adjusted = |base: Color, alpha, lum_mod, lum_off, shade, tint| Color::Adjusted {
        base: Box::new(base),
        alpha,
        lum_mod,
        lum_off,
        shade,
        tint,
    };
    let theme = theme();
    // Alpha alone keeps the rgb and adds the channel.
    assert_eq!(
        jade_pptx_layout::resolve_color(
            &adjusted(
                Color::Srgb("4472C4".into()),
                Some(0.5),
                None,
                None,
                None,
                None
            ),
            &theme
        ),
        "rgba(68,114,196,0.5)"
    );
    // lumMod 75% + lumOff 25% is PowerPoint's "lighter 25%": accent1's HSL
    // lightness 51.8% → 0.518 × 0.75 + 0.25 = 63.8%, hue and saturation
    // kept, which comes back as rgb(115,149,211).
    assert_eq!(
        jade_pptx_layout::resolve_color(
            &adjusted(
                Color::Scheme("accent1".into()),
                None,
                Some(0.75),
                Some(0.25),
                None,
                None
            ),
            &theme
        ),
        "#7395D3"
    );
    // lumMod 50% alone is "darker 50%".
    assert_eq!(
        jade_pptx_layout::resolve_color(
            &adjusted(
                Color::Srgb("FFFFFF".into()),
                None,
                Some(0.5),
                None,
                None,
                None
            ),
            &theme
        ),
        "#808080"
    );
    // Shade darkens toward black, tint lightens toward white.
    assert_eq!(
        jade_pptx_layout::resolve_color(
            &adjusted(
                Color::Srgb("FF0000".into()),
                None,
                None,
                None,
                Some(0.5),
                None
            ),
            &theme
        ),
        "#800000"
    );
    assert_eq!(
        jade_pptx_layout::resolve_color(
            &adjusted(
                Color::Srgb("FF0000".into()),
                None,
                None,
                None,
                None,
                Some(0.5)
            ),
            &theme
        ),
        "#FF8080"
    );
}

#[test]
fn a_title_placeholder_inherits_size_font_color_and_alignment_down_the_chain() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let title = Shape {
        placeholder: Some(Placeholder {
            kind: "title".into(),
            index: None,
        }),
        transform: None,
        text: Some(text_of(vec![paragraph("Inherits everything")])),
        ..shape("Title 1", None, transform(0, 0, 0, 0))
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(title)]),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Title 1")
        .expect("title");
    let run = &placed.lines[0].runs[0];
    // The layout's lstStyle (40pt, centred) beats the master's titleStyle
    // (44pt, left); the master still supplies colour and the theme fonts.
    assert_eq!(run.size_pt, 40.0);
    assert_eq!(run.color, "#111111");
    assert_eq!(run.font.as_deref(), Some("Calibri Light"));
    assert_eq!(run.east_asian_font.as_deref(), Some("Yu Gothic Light"));
    let inner_width = placed.rect.width - 2.0 * emu_to_px(TEXT_INSET_X_EMU);
    let text_width = HeuristicMeasureWidth::of("Inherits everything", 40.0);
    let expected_x = emu_to_px(TEXT_INSET_X_EMU) + (inner_width - text_width) / 2.0;
    assert!(
        (run.x - expected_x).abs() < 1e-6,
        "{} vs {}",
        run.x,
        expected_x
    );
    // Title text is vertically centred by the master's bodyPr anchor, and
    // the 90% line spacing tightens the line box above the glyphs.
    let size_px = 40.0 * PX_PER_PT;
    let line_height = size_px * LINE_SPACING * 0.9;
    let inset_y = emu_to_px(TEXT_INSET_Y_EMU);
    let slack = placed.rect.height - 2.0 * inset_y - line_height;
    let expected_baseline =
        inset_y + slack / 2.0 + line_height - size_px * (LINE_SPACING - ASCENT_RATIO);
    assert!(
        (run.baseline_y - expected_baseline).abs() < 1e-6,
        "{} vs {}",
        run.baseline_y,
        expected_baseline
    );
    // No bullet: the title style says buNone.
    assert_eq!(placed.lines[0].runs.len(), 1);
}

struct HeuristicMeasureWidth;
impl HeuristicMeasureWidth {
    fn of(text: &str, size_pt: f32) -> f64 {
        let em = size_pt as f64 * PX_PER_PT;
        text.chars()
            .map(|c| {
                if HeuristicMeasure::is_wide(c) {
                    em
                } else {
                    em * 0.5
                }
            })
            .sum()
    }
}

#[test]
fn a_body_placeholder_inherits_bullets_margins_and_per_level_sizes() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let content = Shape {
        placeholder: Some(Placeholder {
            kind: String::new(),
            index: Some(1),
        }),
        transform: None,
        text: Some(text_of(vec![
            paragraph("Level one"),
            TextParagraph {
                level: 1,
                ..paragraph("Level two")
            },
        ])),
        ..shape("Content 4", None, transform(0, 0, 0, 0))
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(content)]),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Content 4")
        .expect("content");
    // It landed on the master body placeholder's geometry via idx 1.
    assert!((placed.rect.x - emu_to_px(838_200)).abs() < 1e-9);

    let first = &placed.lines[0];
    assert_eq!(first.runs.len(), 2, "bullet + text: {first:?}");
    let bullet = &first.runs[0];
    assert_eq!(bullet.text, "•");
    assert_eq!(bullet.font.as_deref(), Some("Arial"));
    assert_eq!(bullet.size_pt, 28.0);
    let inset_x = emu_to_px(TEXT_INSET_X_EMU);
    // marL 228600 with indent -228600: the bullet hangs at the inset, the
    // text starts at marL.
    assert!((bullet.x - inset_x).abs() < 1e-9);
    let text = &first.runs[1];
    assert_eq!(text.text, "Level one");
    assert_eq!(text.size_pt, 28.0);
    assert!(
        text.bold,
        "the master body placeholder's lstStyle says bold"
    );
    assert_eq!(text.font.as_deref(), Some("Calibri"));
    assert!((text.x - (inset_x + emu_to_px(228_600))).abs() < 1e-9);

    let second = &placed.lines[1];
    assert_eq!(second.runs[0].text, "–");
    assert_eq!(second.runs[1].size_pt, 24.0);
    assert!((second.runs[0].x - (inset_x + emu_to_px(685_800 - 228_600))).abs() < 1e-9);
    assert!((second.runs[1].x - (inset_x + emu_to_px(685_800))).abs() < 1e-9);
    // spcBef 10pt applies to the second paragraph, not the first.
    let first_line_height = 28.0 * PX_PER_PT * LINE_SPACING;
    let expected_second_baseline = emu_to_px(TEXT_INSET_Y_EMU)
        + first_line_height
        + 10.0 * PX_PER_PT
        + 24.0 * PX_PER_PT * ASCENT_RATIO;
    assert!(
        (second.runs[1].baseline_y - expected_second_baseline).abs() < 1e-6,
        "{} vs {}",
        second.runs[1].baseline_y,
        expected_second_baseline
    );
}

#[test]
fn a_run_that_states_not_bold_beats_an_inherited_bold() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let content = Shape {
        placeholder: Some(Placeholder {
            kind: String::new(),
            index: Some(1),
        }),
        transform: None,
        text: Some(text_of(vec![TextParagraph {
            runs: vec![TextRun {
                text: "plain".into(),
                bold: false,
                bold_stated: Some(false),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }])),
        ..shape("Content 4", None, transform(0, 0, 0, 0))
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(content)]),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Content 4")
        .expect("content");
    assert!(!placed.lines[0].runs[1].bold);
}

#[test]
fn a_plain_shape_inherits_the_presentation_default_text_style() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = list_style(vec![Some(LevelStyle {
        default_run: RunProps {
            size_pt: Some(18.0),
            font: Some("+mn-lt".into()),
            ..RunProps::default()
        },
        ..LevelStyle::default()
    })]);
    let mut boxed = shape("Text box", None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text_of(vec![paragraph("plain")]));
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Text box")
        .expect("text box");
    let run = &placed.lines[0].runs[0];
    assert_eq!(run.size_pt, 18.0);
    assert_eq!(run.font.as_deref(), Some("Calibri"));
    // No bullet: nothing in the chain asks for one.
    assert_eq!(placed.lines[0].runs.len(), 1);
}

#[test]
fn a_shapes_own_list_style_beats_every_inherited_one() {
    let master = master_with_styles();
    let layout = layout_with_placeholders();
    let fonts = fonts();
    let defaults = ListStyle::default();
    let title = Shape {
        placeholder: Some(Placeholder {
            kind: "title".into(),
            index: None,
        }),
        transform: None,
        text: Some(TextBody {
            list_style: Some(list_style(vec![Some(level(Some(20.0)))])),
            ..text_of(vec![paragraph("own")])
        }),
        ..shape("Title 1", None, transform(0, 0, 0, 0))
    };
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(title)]),
        inherited(&layout, &master, &fonts, &defaults),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Title 1")
        .expect("title");
    assert_eq!(placed.lines[0].runs[0].size_pt, 20.0);
}

#[test]
fn line_spacing_percent_and_points_change_the_advance() {
    let mut boxed = shape("box", None, transform(0, 0, 9_144_000, 3_000_000));
    boxed.text = Some(text_of(vec![
        TextParagraph {
            line_spacing: Some(Spacing::Percent(1.5)),
            ..sized_paragraph("one", 20.0)
        },
        TextParagraph {
            line_spacing: Some(Spacing::Points(40.0)),
            ..sized_paragraph("two", 20.0)
        },
        sized_paragraph("three", 20.0),
    ]));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let lines = &laid_out.shapes[0].lines;
    let size_px = 20.0 * PX_PER_PT;
    let first_height = size_px * LINE_SPACING * 1.5;
    let second_height = 40.0 * PX_PER_PT;
    assert!((lines[0].height - first_height).abs() < 1e-9);
    assert!((lines[1].height - second_height).abs() < 1e-9);
    assert!((lines[2].height - size_px * LINE_SPACING).abs() < 1e-9);
    assert!((lines[1].top - (lines[0].top + first_height)).abs() < 1e-9);
    assert!((lines[2].top - (lines[1].top + second_height)).abs() < 1e-9);
    // The baseline sits at the same ascent offset from the line top for
    // proportional spacing (the extra leading goes above the glyphs, as in
    // PowerPoint) and for absolute spacing.
    assert!(
        (lines[0].runs[0].baseline_y
            - (lines[0].top + first_height - size_px * (LINE_SPACING - ASCENT_RATIO)))
            .abs()
            < 1e-9
    );
}

#[test]
fn space_after_and_space_before_separate_paragraphs_but_not_the_first() {
    let mut boxed = shape("box", None, transform(0, 0, 9_144_000, 3_000_000));
    boxed.text = Some(text_of(vec![
        TextParagraph {
            space_before: Some(Spacing::Points(30.0)),
            space_after: Some(Spacing::Percent(0.5)),
            ..sized_paragraph("one", 20.0)
        },
        TextParagraph {
            space_before: Some(Spacing::Points(6.0)),
            ..sized_paragraph("two", 20.0)
        },
    ]));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let lines = &laid_out.shapes[0].lines;
    let size_px = 20.0 * PX_PER_PT;
    assert!((lines[0].top - emu_to_px(TEXT_INSET_Y_EMU)).abs() < 1e-9);
    let expected_second_top =
        lines[0].top + size_px * LINE_SPACING + size_px * 0.5 + 6.0 * PX_PER_PT;
    assert!(
        (lines[1].top - expected_second_top).abs() < 1e-9,
        "{} vs {}",
        lines[1].top,
        expected_second_top
    );
}

#[test]
fn an_empty_paragraph_takes_the_height_of_its_end_run() {
    let mut boxed = shape("box", None, transform(0, 0, 9_144_000, 3_000_000));
    boxed.text = Some(text_of(vec![
        sized_paragraph("one", 20.0),
        TextParagraph {
            end_size_pt: Some(36.0),
            ..TextParagraph::default()
        },
        sized_paragraph("three", 20.0),
    ]));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let lines = &laid_out.shapes[0].lines;
    assert_eq!(lines.len(), 3);
    assert!(lines[1].runs.is_empty());
    assert!((lines[1].height - 36.0 * PX_PER_PT * LINE_SPACING).abs() < 1e-9);
    assert!((lines[2].top - (lines[1].top + 36.0 * PX_PER_PT * LINE_SPACING)).abs() < 1e-9);
}

#[test]
fn normal_autofit_scales_the_font_and_tightens_the_spacing() {
    let mut boxed = shape("box", None, transform(0, 0, 9_144_000, 3_000_000));
    boxed.text = Some(TextBody {
        autofit: Autofit::Normal {
            font_scale: 0.5,
            line_spacing_reduction: 0.2,
        },
        ..text_of(vec![sized_paragraph("shrunk", 40.0)])
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let line = &laid_out.shapes[0].lines[0];
    assert_eq!(line.runs[0].size_pt, 20.0);
    // The reduction arrives as an f32, so compare loosely.
    assert!((line.height - 20.0 * PX_PER_PT * LINE_SPACING * 0.8).abs() < 1e-5);
}

#[test]
fn a_body_that_does_not_wrap_keeps_each_paragraph_on_one_line() {
    // 100 ASCII graphemes at 18pt are 1200 px wide in a 96 px box.
    let mut boxed = shape("box", None, transform(0, 0, 914_400, 914_400));
    boxed.text = Some(TextBody {
        wrap: false,
        ..text_of(vec![sized_paragraph(&"a ".repeat(50), 18.0)])
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].lines.len(), 1);
}

#[test]
fn unwrapped_centred_text_centres_on_the_box_even_when_wider_than_it() {
    let mut boxed = shape("box", None, transform(0, 0, 914_400, 914_400));
    boxed.text = Some(TextBody {
        wrap: false,
        ..text_of(vec![TextParagraph {
            align: Some(TextAlign::Center),
            ..sized_paragraph(&"a".repeat(20), 18.0)
        }])
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let run = &laid_out.shapes[0].lines[0].runs[0];
    let inset = emu_to_px(TEXT_INSET_X_EMU);
    let inner = 96.0 - 2.0 * inset;
    let width = 20.0 * 18.0 * PX_PER_PT * 0.5;
    assert!((run.x - (inset + (inner - width) / 2.0)).abs() < 1e-9);
    assert!(run.x < 0.0, "overflows to the left of the box: {}", run.x);
}

#[test]
fn placed_runs_carry_east_asian_font_strike_and_hyperlink() {
    let mut boxed = shape("box", None, transform(0, 0, 9_144_000, 914_400));
    boxed.text = Some(text_of(vec![TextParagraph {
        runs: vec![TextRun {
            text: "対象".into(),
            font: Some("Yu Gothic".into()),
            east_asian_font: Some("+mn-ea".into()),
            strike: true,
            hyperlink: Some(jade_pptx_reader::Hyperlink {
                rel_id: "rId3".into(),
                target: Some("https://example.com".into()),
            }),
            ..TextRun::default()
        }],
        ..TextParagraph::default()
    }]));
    let fonts = fonts();
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        InheritedContent {
            theme_fonts: Some(&fonts),
            ..InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );
    let run = &laid_out.shapes[0].lines[0].runs[0];
    assert_eq!(run.east_asian_font.as_deref(), Some("Yu Gothic"));
    assert!(run.strike);
    assert_eq!(run.hyperlink.as_deref(), Some("https://example.com"));
    assert_eq!(run.size_pt, DEFAULT_SIZE_PT);
}

fn cell(text: &str) -> TableCell {
    TableCell {
        text: Some(text_of(vec![sized_paragraph(text, 12.0)])),
        fill: None,
        borders: CellBorders::default(),
        grid_span: 1,
        row_span: 1,
        h_merge: false,
        v_merge: false,
        anchor: None,
        insets: Default::default(),
    }
}

fn table() -> Table {
    Table {
        name: "Table 5".into(),
        transform: Some(transform(914_400, 1_828_800, 2_743_200, 768_096)),
        column_widths_emu: vec![914_400, 1_828_800],
        rows: vec![
            TableRow {
                height_emu: 384_048,
                cells: vec![
                    TableCell {
                        fill: Some(Fill::Solid(Color::Srgb("1F2937".into()))),
                        borders: CellBorders {
                            left: Some(Outline {
                                color: Some(Color::Srgb("D9D9D9".into())),
                                width_emu: Some(10_160),
                                ..Outline::default()
                            }),
                            bottom: Some(Outline {
                                color: Some(Color::Srgb("000000".into())),
                                width_emu: Some(20_320),
                                dash: Some("sysDash".into()),
                                no_fill: false,
                                head_end: None,
                                tail_end: None,
                            }),
                            ..CellBorders::default()
                        },
                        anchor: Some(TextAnchor::Center),
                        ..cell("Header A")
                    },
                    cell("Header B"),
                ],
            },
            TableRow {
                height_emu: 384_048,
                cells: vec![
                    TableCell {
                        grid_span: 2,
                        ..cell("Spans both")
                    },
                    TableCell {
                        h_merge: true,
                        ..cell("")
                    },
                ],
            },
        ],
        id: None,
        style: Default::default(),
    }
}

#[test]
fn table_cells_land_on_the_grid_with_fills_text_and_borders_as_lines() {
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table())]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Rect)
        .collect();
    assert_eq!(cells.len(), 3, "merged cell is not drawn: {cells:?}");

    let table_x = emu_to_px(914_400);
    let table_y = emu_to_px(1_828_800);
    let col0 = emu_to_px(914_400);
    let col1 = emu_to_px(1_828_800);
    let row_h = emu_to_px(384_048);

    let header_a = &cells[0];
    assert_eq!(header_a.name, "Table 5!A1");
    assert!((header_a.rect.x - table_x).abs() < 1e-9);
    assert!((header_a.rect.y - table_y).abs() < 1e-9);
    assert!((header_a.rect.width - col0).abs() < 1e-9);
    assert!((header_a.rect.height - row_h).abs() < 1e-9);
    assert_eq!(header_a.fill.as_deref(), Some("#1F2937"));
    assert_eq!(header_a.lines[0].runs[0].text, "Header A");
    // Centre-anchored within the row.
    let content_height = 12.0 * PX_PER_PT * LINE_SPACING;
    let inset_y = emu_to_px(TEXT_INSET_Y_EMU);
    let expected_baseline =
        inset_y + (row_h - 2.0 * inset_y - content_height) / 2.0 + 12.0 * PX_PER_PT * ASCENT_RATIO;
    assert!((header_a.lines[0].runs[0].baseline_y - expected_baseline).abs() < 1e-6);

    let header_b = &cells[1];
    assert_eq!(header_b.name, "Table 5!B1");
    assert!((header_b.rect.x - (table_x + col0)).abs() < 1e-9);
    assert!((header_b.rect.width - col1).abs() < 1e-9);
    assert_eq!(header_b.fill, None);

    let spanning = &cells[2];
    assert_eq!(spanning.name, "Table 5!A2");
    assert!((spanning.rect.width - (col0 + col1)).abs() < 1e-9);
    assert!((spanning.rect.y - (table_y + row_h)).abs() < 1e-9);

    let borders: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Line)
        .collect();
    assert_eq!(borders.len(), 2);
    let left = &borders[0];
    assert!((left.rect.x - table_x).abs() < 1e-9);
    assert!((left.rect.width).abs() < 1e-9);
    assert!((left.rect.height - row_h).abs() < 1e-9);
    assert_eq!(left.outline.as_ref().expect("outline").color, "#D9D9D9");
    let bottom = &borders[1];
    assert!((bottom.rect.y - (table_y + row_h)).abs() < 1e-9);
    assert!((bottom.rect.width - col0).abs() < 1e-9);
    assert!((bottom.rect.height).abs() < 1e-9);
    assert!(!bottom.outline.as_ref().expect("outline").dash_px.is_empty());
    // Borders draw after every cell so they are never covered by a fill.
    let last_cell = laid_out
        .shapes
        .iter()
        .rposition(|s| s.geometry == PlacedGeometry::Rect)
        .expect("cells");
    let first_border = laid_out
        .shapes
        .iter()
        .position(|s| s.geometry == PlacedGeometry::Line)
        .expect("borders");
    assert!(first_border > last_cell);
}

#[test]
fn a_row_grows_to_fit_its_tallest_cell() {
    let mut grown = table();
    // Six 12pt lines need far more than the 0.42 inch row height.
    grown.rows[0].cells[1].text = Some(text_of(
        (0..6).map(|_| sized_paragraph("line", 12.0)).collect(),
    ));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(grown)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Rect)
        .collect();
    let needed = 6.0 * 12.0 * PX_PER_PT * LINE_SPACING + 2.0 * emu_to_px(TEXT_INSET_Y_EMU);
    assert!(
        (cells[0].rect.height - needed).abs() < 1e-6,
        "{}",
        cells[0].rect.height
    );
    assert!((cells[1].rect.height - needed).abs() < 1e-6);
    // The next row moves down with it.
    assert!((cells[2].rect.y - (cells[0].rect.y + needed)).abs() < 1e-6);
}

#[test]
fn a_row_span_covers_the_rows_below_and_skips_the_merged_cells() {
    let mut spanned = table();
    spanned.rows[0].cells[0].grid_span = 1;
    spanned.rows[0].cells[0].row_span = 2;
    spanned.rows[1].cells[0] = TableCell {
        v_merge: true,
        ..cell("")
    };
    spanned.rows[1].cells[1] = cell("B2");
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(spanned)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Rect)
        .collect();
    let names: Vec<&str> = cells.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Table 5!A1", "Table 5!B1", "Table 5!B2"]);
    assert!((cells[0].rect.height - 2.0 * emu_to_px(384_048)).abs() < 1e-9);
}

#[test]
fn layout_deck_slide_lays_out_from_a_parsed_deck() {
    let deck = Deck {
        slide_size: slide_size(),
        slides: vec![DeckSlide {
            id: None,
            part_name: "ppt/slides/slide1.xml".into(),
            content: slide_with(vec![ShapeItem::Shape(Shape {
                placeholder: Some(Placeholder {
                    kind: "title".into(),
                    index: None,
                }),
                transform: None,
                text: Some(text_of(vec![paragraph("From the deck")])),
                ..shape("Title 1", None, transform(0, 0, 0, 0))
            })]),
            layout_content: Some(layout_with_placeholders()),
            master_content: Some(master_with_styles()),
            notes: None,
            theme: None,
        }],
        theme_colors: theme(),
        theme_fonts: fonts(),
        default_text_style: ListStyle::default(),
        table_styles: Vec::new(),
        theme_fill_styles: Vec::new(),
        theme_background_fill_styles: Vec::new(),
        theme_line_styles: Vec::new(),
    };
    let laid_out = layout_deck_slide(&deck, 0, &HeuristicMeasure).expect("slide 0");
    let title = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "Title 1")
        .expect("title");
    assert_eq!(title.lines[0].runs[0].size_pt, 40.0);
    assert_eq!(
        title.lines[0].runs[0].font.as_deref(),
        Some("Calibri Light")
    );
    assert!(layout_deck_slide(&deck, 1, &HeuristicMeasure).is_none());
}

#[test]
fn a_shape_whose_only_paragraph_is_empty_has_no_text_to_overflow() {
    // A decorative bar: 1 px tall, an empty paragraph with an end run size.
    let mut bar = shape("Bar", None, transform(0, 0, 9_144_000, 9_144));
    bar.text = Some(text_of(vec![TextParagraph {
        end_size_pt: Some(18.0),
        ..TextParagraph::default()
    }]));
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(bar)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].text_bottom_px(), 0.0);
    assert_eq!(laid_out.shapes[0].text_overflow_px(), 0.0);
}

#[test]
fn a_shape_that_autofits_grows_to_hold_its_text() {
    // Six 18pt lines in a 24 px box.
    let mut boxed = shape("Grows", None, transform(0, 0, 9_144_000, 228_600));
    boxed.text = Some(TextBody {
        autofit: Autofit::Shape,
        anchor: Some(TextAnchor::Bottom),
        ..text_of((0..6).map(|_| sized_paragraph("line", 18.0)).collect())
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    let needed = 6.0 * 18.0 * PX_PER_PT * LINE_SPACING + 2.0 * emu_to_px(TEXT_INSET_Y_EMU);
    assert!(
        (placed.rect.height - needed).abs() < 1e-6,
        "{}",
        placed.rect.height
    );
    assert_eq!(placed.text_overflow_px(), 0.0);
    // Bottom anchoring is re-applied in the grown box: the first line starts
    // at the top inset.
    assert!((placed.lines[0].top - emu_to_px(TEXT_INSET_Y_EMU)).abs() < 1e-6);

    let mut fits = shape("Fits", None, transform(0, 0, 9_144_000, 9_144_000));
    fits.text = Some(TextBody {
        autofit: Autofit::Shape,
        ..text_of(vec![sized_paragraph("line", 18.0)])
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(fits)]),
        &theme(),
        &HeuristicMeasure,
    );
    // A box already big enough keeps its size.
    assert!((laid_out.shapes[0].rect.height - 960.0).abs() < 1e-9);
}
