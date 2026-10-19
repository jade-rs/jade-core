//! Charts draw as a labelled frame until they draw as charts; SmartArt
//! shapes land inside their frame; pattern and picture fills resolve.

use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedGeometry, PlacedOrigin};
use jade_pptx_reader::{
    Chart, ChartSeries, Color, Fill, GroupShape, GroupTransform, Shape, ShapeItem, SlideContent,
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
        custom_geometry: None,
        fill: None,
        outline: None,
        style: None,
        image_rel_id: None,
        image_part: None,
        source_rect: None,
        text: None,
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

fn chart(title: Option<&str>) -> Chart {
    Chart {
        id: Some(3),
        name: "Chart 2".into(),
        transform: Some(transform(914_400, 914_400, 4_572_000, 2_743_200)),
        chart_rel_id: Some("rId6".into()),
        chart_part: Some("ppt/charts/chart1.xml".into()),
        kind: Some("barChart".into()),
        title: title.map(str::to_string),
        series: vec![ChartSeries {
            name: Some("Revenue".into()),
            categories: vec!["Q1".into(), "Q2".into()],
            values: vec![Some(10.0), Some(12.5)],
            ..ChartSeries::default()
        }],
        horizontal: false,
        style: Default::default(),
    }
}

#[test]
fn a_chart_places_as_a_bordered_frame_with_its_title_across_the_top() {
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Chart(chart(Some("Quarterly Revenue")))]),
        &theme(),
        &HeuristicMeasure,
    );
    // The frame comes first; the plot follows it.
    assert!(laid_out.shapes.len() > 1);
    let placed = &laid_out.shapes[0];
    assert_eq!(placed.name, "Chart 2");
    assert_eq!(placed.source_id, Some(3));
    assert_eq!(placed.origin, PlacedOrigin::Slide);
    assert_eq!(placed.geometry, PlacedGeometry::Rect);
    assert!((placed.rect.x - 96.0).abs() < 1e-9);
    assert!((placed.rect.width - 480.0).abs() < 1e-9);
    assert_eq!(placed.fill, None);
    // A light frame marks where the chart goes.
    let outline = placed.outline.as_ref().expect("frame");
    assert_eq!(outline.color, "#BFBFBF");
    // The title is the one line of text, centred, bold, chart-title sized.
    assert_eq!(placed.lines.len(), 1);
    let title: String = placed.lines[0]
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(title, "Quarterly Revenue");
    let run = &placed.lines[0].runs[0];
    assert!(run.bold);
    assert_eq!(run.size_pt, 14.0);
    let width: f64 = placed.lines[0].runs.iter().map(|run| run.width).sum();
    let centre = run.x + width / 2.0;
    assert!((centre - placed.rect.width / 2.0).abs() < 1e-6, "{centre}");
}

#[test]
fn a_chart_without_a_title_still_places_its_frame() {
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Chart(chart(None))]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].name, "Chart 2");
    assert!(laid_out.shapes[0].lines.is_empty());
    assert!(laid_out.shapes[0].outline.is_some());
}

#[test]
fn a_chart_without_a_transform_is_not_placed() {
    let mut unplaced = chart(None);
    unplaced.transform = None;
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Chart(unplaced)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert!(laid_out.shapes.is_empty());
}

#[test]
fn smartart_shapes_land_inside_the_frame_like_any_group() {
    let mut step = shape("", transform(2_500_000, 0, 2_000_000, 1_000_000));
    step.text = Some(TextBody {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text: "Build".into(),
                ..TextRun::default()
            }],
            ..TextParagraph::default()
        }],
        ..TextBody::default()
    });
    let diagram = GroupShape {
        id: Some(2),
        name: "Diagram 1".into(),
        transform: Some(GroupTransform {
            outer: transform(914_400, 914_400, 4_572_000, 2_743_200),
            child_offset_emu: (0, 0),
            child_extent_emu: (4_572_000, 2_743_200),
        }),
        children: vec![ShapeItem::Shape(step)],
        diagram_data_rel_id: Some("rId5".into()),
    };
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Group(diagram)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes.len(), 1);
    let placed = &laid_out.shapes[0];
    // Frame origin (96, 96) plus the drawing's own offset.
    assert!((placed.rect.x - (96.0 + 2_500_000.0 / 914_400.0 * 96.0)).abs() < 1e-6);
    assert!((placed.rect.y - 96.0).abs() < 1e-9);
    assert_eq!(placed.lines[0].runs[0].text, "Build");
}

#[test]
fn a_pattern_fill_paints_as_the_blend_of_its_two_colours() {
    let mut hatched = shape("h", transform(0, 0, 914_400, 914_400));
    hatched.fill = Some(Fill::Pattern {
        preset: "dkUpDiag".into(),
        foreground: Color::Srgb("000000".into()),
        background: Color::Srgb("FFFFFF".into()),
    });
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(hatched)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(laid_out.shapes[0].fill.as_deref(), Some("#808080"));
    assert_eq!(laid_out.shapes[0].gradient, None);
}

#[test]
fn a_picture_filled_shape_carries_its_image_part() {
    let mut photo = shape("p", transform(0, 0, 914_400, 914_400));
    photo.preset_geometry = Some("ellipse".into());
    photo.image_part = Some("ppt/media/image1.png".into());
    let laid_out = layout_slide(
        slide_size(),
        &slide_with(vec![ShapeItem::Shape(photo)]),
        &theme(),
        &HeuristicMeasure,
    );
    assert_eq!(
        laid_out.shapes[0].image_part.as_deref(),
        Some("ppt/media/image1.png")
    );
    assert_eq!(laid_out.shapes[0].geometry, PlacedGeometry::Ellipse);
    assert_eq!(laid_out.shapes[0].fill, None);
}
