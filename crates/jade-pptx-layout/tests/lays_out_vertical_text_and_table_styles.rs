//! Vertical text bodies lay out in a turned frame, and tables take their
//! fills, borders and header styling from a built-in or custom table style.

use jade_pptx_layout::{
    layout_slide, layout_slide_inheriting, HeuristicMeasure, InheritedContent, PlacedGeometry,
    LINE_SPACING, PX_PER_PT, TEXT_INSET_X_EMU, TEXT_INSET_Y_EMU,
};
use jade_pptx_reader::{
    emu_to_px, CellBorders, Color, Fill, Outline, Shape, ShapeItem, SlideContent, SlideSize, Table,
    TableCell, TablePartStyle, TableRow, TableStyle, TableStyleFlags, TextBody, TextParagraph,
    TextRun, TextVertical, ThemeColors, Transform,
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
        shadow: None,
        effects: Default::default(),
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
      <a:dk1><a:srgbClr val="000000"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
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
fn a_vertical_body_lays_out_in_a_frame_turned_a_quarter_turn() {
    // A tall narrow box: 1 inch wide, 4 inches tall. Vertical text flows
    // down the 4-inch side, so the frame the lines wrap in is 4 inches wide.
    let mut boxed = shape("v", transform(0, 0, 914_400, 3_657_600));
    boxed.text = Some(TextBody {
        vertical: Some(TextVertical::EastAsian),
        paragraphs: vec![paragraph(&"縦".repeat(12))],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    assert_eq!(placed.text_rotation_deg, 90.0);
    // 12 full-em graphemes at 18pt = 288 px fit in the 384 px frame width
    // (minus insets), so one line — they would need three in the 96 px box.
    assert_eq!(placed.lines.len(), 1, "{:?}", placed.lines);
    let run = &placed.lines[0].runs[0];
    assert!((run.x - emu_to_px(TEXT_INSET_X_EMU)).abs() < 1e-9);
    // Overflow is judged against the frame, not the shape's own height.
    assert_eq!(placed.text_overflow_px(), 0.0);
    assert_eq!(placed.text_overflow_right_px(), 0.0);
}

#[test]
fn a_horizontal_body_has_no_text_rotation() {
    let mut boxed = shape("h", transform(0, 0, 914_400, 914_400));
    boxed.text = Some(TextBody {
        paragraphs: vec![paragraph("x")],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].text_rotation_deg, 0.0);
    let mut rotated = shape("r", transform(0, 0, 914_400, 914_400));
    rotated.text = Some(TextBody {
        vertical: Some(TextVertical::Rotated270),
        paragraphs: vec![paragraph("x")],
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(rotated)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].text_rotation_deg, 270.0);
}

#[test]
fn a_vertical_body_that_overflows_its_frame_reports_it() {
    // 1 inch wide, 1 inch tall: the frame is 96 px wide too, so 12 wide
    // graphemes wrap to three lines and the columns overflow the width.
    let mut boxed = shape("v", transform(0, 0, 914_400, 914_400));
    boxed.text = Some(TextBody {
        vertical: Some(TextVertical::EastAsian),
        paragraphs: (0..4).map(|_| paragraph(&"縦".repeat(3))).collect(),
        ..TextBody::default()
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(boxed)]),
        &theme(),
        &HeuristicMeasure,
    );
    let placed = &laid_out.shapes[0];
    let needed = 4.0 * 18.0 * PX_PER_PT * LINE_SPACING + emu_to_px(TEXT_INSET_Y_EMU);
    assert!((placed.text_bottom_px() - needed).abs() < 1e-6);
    assert!((placed.text_overflow_px() - (needed - 96.0)).abs() < 1e-6);
}

fn cell(text: &str) -> TableCell {
    TableCell {
        text: Some(TextBody {
            paragraphs: vec![TextParagraph {
                runs: vec![TextRun {
                    text: text.to_string(),
                    size_pt: Some(12.0),
                    ..TextRun::default()
                }],
                ..TextParagraph::default()
            }],
            ..TextBody::default()
        }),
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

fn styled_table(style_id: &str, rows: usize) -> Table {
    Table {
        id: Some(9),
        name: "T".into(),
        transform: Some(transform(0, 0, 1_828_800, 384_048 * rows as i64)),
        column_widths_emu: vec![914_400, 914_400],
        rows: (0..rows)
            .map(|r| TableRow {
                height_emu: 384_048,
                cells: vec![cell(&format!("a{r}")), cell(&format!("b{r}"))],
            })
            .collect(),
        style: TableStyleFlags {
            style_id: Some(style_id.into()),
            first_row: true,
            last_row: false,
            first_col: false,
            last_col: false,
            band_rows: true,
            band_cols: false,
        },
    }
}

fn cells(layout: &jade_pptx_layout::SlideLayout) -> Vec<&jade_pptx_layout::PlacedShape> {
    layout
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Rect)
        .collect()
}

#[test]
fn the_built_in_medium_style_2_gives_a_header_row_banding_and_white_borders() {
    let table = styled_table("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}", 3);
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells = cells(&laid_out);
    assert_eq!(cells.len(), 6);
    // Header row: accent1 fill, bold white text.
    assert_eq!(cells[0].fill.as_deref(), Some("#4472C4"));
    let header_run = &cells[0].lines[0].runs[0];
    assert!(header_run.bold);
    assert_eq!(header_run.color, "#FFFFFF");
    // First data row: band 1, accent1 at 40% tint; second: the whole-table
    // fill at 20% tint. Text stays dark and regular.
    assert_eq!(cells[2].fill.as_deref(), Some("#B4C7E7"));
    assert_eq!(cells[4].fill.as_deref(), Some("#DAE3F3"));
    assert!(!cells[2].lines[0].runs[0].bold);
    assert_eq!(cells[2].lines[0].runs[0].color, "#000000");
    // Every cell gets white borders on all four sides.
    let borders: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Line)
        .collect();
    assert_eq!(borders.len(), 24, "{:?}", borders.len());
    assert!(borders
        .iter()
        .all(|b| b.outline.as_ref().expect("outline").color == "#FFFFFF"));
    // The header's bottom border is the thick one.
    let header_bottom = borders
        .iter()
        .find(|b| b.name == "T!A1:bottom")
        .expect("header bottom");
    assert!(
        (header_bottom.outline.as_ref().expect("outline").width_px - emu_to_px(38_100)).abs()
            < 1e-9
    );
}

#[test]
fn an_explicit_cell_fill_and_border_beat_the_table_style() {
    let mut table = styled_table("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}", 2);
    table.rows[0].cells[0].fill = Some(Fill::Solid(Color::Srgb("FF0000".into())));
    table.rows[0].cells[0].borders.left = Some(Outline {
        color: Some(Color::Srgb("00FF00".into())),
        width_emu: Some(25_400),
        ..Outline::default()
    });
    table.rows[1].cells[1].fill = Some(Fill::None);
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells = cells(&laid_out);
    assert_eq!(cells[0].fill.as_deref(), Some("#FF0000"));
    assert_eq!(cells[3].fill, None);
    let left = laid_out
        .shapes
        .iter()
        .find(|s| s.name == "T!A1:left")
        .expect("left border");
    assert_eq!(left.outline.as_ref().expect("outline").color, "#00FF00");
}

#[test]
fn a_table_with_an_unknown_style_stays_plain() {
    let unknown = styled_table("{NOT-A-KNOWN-STYLE}", 2);
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(unknown)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert!(cells(&laid_out).iter().all(|c| c.fill.is_none()));
    assert!(!laid_out
        .shapes
        .iter()
        .any(|s| s.geometry == PlacedGeometry::Line));
}

#[test]
fn a_table_naming_no_style_draws_a_thin_grid_as_powerpoint_does() {
    // PowerPoint draws a table without `a:tableStyleId` like "No Style,
    // Table Grid": a 1pt text-coloured line on every cell edge, no fills
    // and no header emphasis (its own PDF of such a deck shows the grid).
    let mut plain = styled_table("", 2);
    plain.style.style_id = None;
    plain.style.first_row = true;
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(plain)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert!(cells(&laid_out).iter().all(|c| c.fill.is_none()));
    let lines: Vec<_> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Line)
        .collect();
    // 2 × 2 cells, each with its four edges.
    assert_eq!(lines.len(), 16,"{:?}", lines.iter().map(|l| &l.name).collect::<Vec<_>>());
    for line in lines {
        let outline = line.outline.as_ref().unwrap();
        assert_eq!(outline.color, "#000000");
        assert!((outline.width_px - 96.0 / 72.0).abs() < 1e-9);
    }
    let header = cells(&laid_out)[0];
    assert!(!header.lines[0].runs[0].bold);
}

#[test]
fn all_of_powerpoints_built_in_table_styles_resolve() {
    let ids: Vec<&str> = jade_pptx_layout::builtin_table_style_ids().collect();
    assert_eq!(ids.len(), 74);
    let mut names = std::collections::HashSet::new();
    for id in &ids {
        let style = jade_pptx_layout::builtin_table_style(id).expect(id);
        assert!(names.insert(style.name.clone().unwrap()), "{:?}", style.name);
    }
    // GUIDs match whatever case a deck writes them in.
    let lower = jade_pptx_layout::builtin_table_style("{bc89ef96-8cea-46ff-86c4-4ce0e7609802}")
        .expect("Light Style 3 - Accent 1");
    assert_eq!(lower.name.as_deref(), Some("Light Style 3 - Accent 1"));
    assert_eq!(
        jade_pptx_layout::builtin_table_style("{0660B408-B3CF-4A94-85FC-2B1E0A45F4A2}")
            .and_then(|s| s.name),
        Some("Dark Style 2 - Accent 1/Accent 2".to_string())
    );
}

#[test]
fn light_style_1_rules_the_header_and_washes_banded_rows() {
    // Light Style 1 - Accent 2, with a header and banded rows.
    let mut table = styled_table("{0E3FDE45-AF77-4B5C-9715-49D594BDF05E}", 4);
    table.style.first_row = true;
    table.style.band_rows = true;
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells = cells(&laid_out);
    // The header: bold, unfilled.
    assert!(cells[0].lines[0].runs[0].bold);
    assert_eq!(cells[0].fill, None);
    // Data rows alternate: the first band is a wash of accent 2, the
    // second clear.
    let band = cells[2].fill.as_deref().expect("band 1 fill");
    assert!(band.starts_with("rgba(237,125,49,"), "{band}");
    assert_eq!(cells[4].fill, None);
    // Rules: under the header and around the table in the accent, none
    // between body rows or columns.
    let rule = |name: &str| laid_out.shapes.iter().find(|s| s.name == name);
    let accent = |name: &str| rule(name).and_then(|s| s.outline.as_ref()).map(|o| o.color.clone());
    assert_eq!(accent("T!A1:bottom").as_deref(), Some("#ED7D31"));
    assert_eq!(accent("T!A1:top").as_deref(), Some("#ED7D31"));
    assert!(rule("T!A1:left").is_none());
    assert!(rule("T!A2:bottom").is_none());
}

#[test]
fn dark_style_2_pairs_its_header_with_the_next_accent() {
    let mut table = styled_table("{0660B408-B3CF-4A94-85FC-2B1E0A45F4A2}", 3);
    table.style.first_row = true;
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table)]),
        &theme(),
        &HeuristicMeasure,
    );
    let cells = cells(&laid_out);
    assert_eq!(cells[0].fill.as_deref(), Some("#ED7D31"));
    assert_eq!(cells[0].lines[0].runs[0].color, "#FFFFFF");
    // The body is accent 1 tinted.
    let body = cells[2].fill.as_deref().unwrap();
    assert_ne!(body, "#4472C4");
    assert!(body.starts_with('#'));
}

#[test]
fn a_custom_style_from_the_deck_applies_over_the_built_ins() {
    let custom = TableStyle {
        id: "{CUSTOM-1}".into(),
        name: Some("Custom".into()),
        whole: Some(TablePartStyle {
            fill: Some(Fill::Solid(Color::Srgb("EEEEEE".into()))),
            inside_h: Some(Outline {
                color: Some(Color::Srgb("ABCDEF".into())),
                width_emu: Some(6_350),
                ..Outline::default()
            }),
            ..TablePartStyle::default()
        }),
        first_row: Some(TablePartStyle {
            fill: Some(Fill::Solid(Color::Scheme("accent1".into()))),
            bold: Some(true),
            text_color: Some(Color::Scheme("lt1".into())),
            ..TablePartStyle::default()
        }),
        ..TableStyle::default()
    };
    let table = styled_table("{CUSTOM-1}", 2);
    let laid_out = layout_slide_inheriting(
        slide_size(),
        &slide_with(vec![ShapeItem::Table(table)]),
        InheritedContent {
            table_styles: Some(std::slice::from_ref(&custom)),
            ..InheritedContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    );
    let cells = cells(&laid_out);
    assert_eq!(cells[0].fill.as_deref(), Some("#4472C4"));
    assert!(cells[0].lines[0].runs[0].bold);
    assert_eq!(cells[2].fill.as_deref(), Some("#EEEEEE"));
    // insideH draws between the rows, not on the outer edges.
    let names: Vec<&str> = laid_out
        .shapes
        .iter()
        .filter(|s| s.geometry == PlacedGeometry::Line)
        .map(|s| s.name.as_str())
        .collect();
    assert!(names.contains(&"T!A1:bottom"), "{names:?}");
    assert!(names.contains(&"T!A2:top"), "{names:?}");
    assert!(!names.contains(&"T!A1:top"), "{names:?}");
    assert!(!names.contains(&"T!A2:bottom"), "{names:?}");
}
