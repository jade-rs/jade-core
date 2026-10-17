//! Charts draw as charts: columns, bars, lines and pies from the cached
//! series, inside the chart's frame, in the theme's accent colours.

use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedGeometry, PlacedShape};
use jade_pptx_reader::{
    Chart, ChartSeries, ShapeItem, SlideContent, SlideSize, ThemeColors, Transform,
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

fn theme() -> ThemeColors {
    let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="t">
      <a:dk1><a:srgbClr val="000000"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
    </a:clrScheme></a:themeElements></a:theme>"#;
    jade_pptx_reader::parse_theme_colors(xml).expect("theme")
}

fn slide_size() -> SlideSize {
    SlideSize {
        width_emu: 12_192_000,
        height_emu: 6_858_000,
    }
}

fn series(name: &str, categories: &[&str], values: &[Option<f64>]) -> ChartSeries {
    ChartSeries {
        name: Some(name.to_string()),
        categories: categories.iter().map(|c| c.to_string()).collect(),
        values: values.to_vec(),
    }
}

fn chart(kind: &str, horizontal: bool, series: Vec<ChartSeries>) -> Chart {
    Chart {
        id: Some(3),
        name: "Chart 2".into(),
        transform: Some(transform(914_400, 914_400, 4_572_000, 2_743_200)),
        chart_rel_id: None,
        chart_part: None,
        kind: Some(kind.to_string()),
        title: Some("Sales".into()),
        horizontal,
        series,
    }
}

fn laid_out(chart: Chart) -> Vec<PlacedShape> {
    layout_slide(
        slide_size(),
        &SlideContent {
            shapes: vec![ShapeItem::Chart(chart)],
            ..SlideContent::default()
        },
        &theme(),
        &HeuristicMeasure,
    )
    .shapes
}

fn named<'a>(shapes: &'a [PlacedShape], name: &str) -> &'a PlacedShape {
    shapes
        .iter()
        .find(|shape| shape.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no shape named {name}; have {:?}",
                shapes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
            )
        })
}

#[test]
fn a_column_chart_draws_one_bar_per_series_and_category_from_a_shared_baseline() {
    let shapes = laid_out(chart(
        "barChart",
        false,
        vec![
            series("Revenue", &["Q1", "Q2"], &[Some(10.0), Some(12.5)]),
            series("Costs", &["Q1", "Q2"], &[Some(5.0), None]),
        ],
    ));
    // The frame comes first, the plot after it, all tagged with the chart.
    assert_eq!(shapes[0].name, "Chart 2");
    assert!(shapes.iter().all(|s| s.source_id == Some(3)));

    let revenue_q1 = named(&shapes, "Chart 2!Revenue/Q1");
    let revenue_q2 = named(&shapes, "Chart 2!Revenue/Q2");
    let costs_q1 = named(&shapes, "Chart 2!Costs/Q1");
    assert!(
        !shapes.iter().any(|s| s.name == "Chart 2!Costs/Q2"),
        "a hole draws nothing"
    );
    for bar in [revenue_q1, revenue_q2, costs_q1] {
        assert_eq!(bar.geometry, PlacedGeometry::Rect);
        assert!(bar.width_positive());
    }
    // Series colours follow the theme accents.
    assert_eq!(revenue_q1.fill.as_deref(), Some("#4472C4"));
    assert_eq!(costs_q1.fill.as_deref(), Some("#ED7D31"));
    // Heights scale with the values against the tallest; bottoms share the
    // baseline, which is the axis line.
    let bottom = |bar: &PlacedShape| bar.rect.y + bar.rect.height;
    assert!((bottom(revenue_q1) - bottom(revenue_q2)).abs() < 1e-9);
    assert!((bottom(revenue_q1) - bottom(costs_q1)).abs() < 1e-9);
    assert!((revenue_q1.rect.height / revenue_q2.rect.height - 0.8).abs() < 1e-9);
    assert!((costs_q1.rect.height / revenue_q2.rect.height - 0.4).abs() < 1e-9);
    let axis = named(&shapes, "Chart 2!axis");
    assert_eq!(axis.geometry, PlacedGeometry::Line);
    assert!((axis.rect.y - bottom(revenue_q1)).abs() < 1e-9);
    assert_eq!(axis.rect.height, 0.0);
    // Within a category the series sit side by side, left to right; the
    // second category is to the right of the first.
    assert!(costs_q1.rect.x >= revenue_q1.rect.x + revenue_q1.rect.width);
    assert!(revenue_q2.rect.x > costs_q1.rect.x);
    // Everything stays inside the chart's frame.
    let frame = &shapes[0];
    for shape in &shapes[1..] {
        assert!(shape.rect.x >= frame.rect.x - 1e-9);
        assert!(shape.rect.x + shape.rect.width <= frame.rect.x + frame.rect.width + 1e-9);
        assert!(shape.rect.y >= frame.rect.y - 1e-9);
        assert!(shape.rect.y + shape.rect.height <= frame.rect.y + frame.rect.height + 1e-9);
    }
    // Category labels sit under the axis with their text.
    let q1 = named(&shapes, "Chart 2!label Q1");
    assert_eq!(q1.lines[0].runs[0].text, "Q1");
    assert!(q1.rect.y >= axis.rect.y);
}

#[test]
fn a_horizontal_bar_chart_grows_bars_to_the_right_from_the_left_axis() {
    let shapes = laid_out(chart(
        "barChart",
        true,
        vec![series("Revenue", &["Q1", "Q2"], &[Some(10.0), Some(20.0)])],
    ));
    let q1 = named(&shapes, "Chart 2!Revenue/Q1");
    let q2 = named(&shapes, "Chart 2!Revenue/Q2");
    assert!(
        (q1.rect.x - q2.rect.x).abs() < 1e-9,
        "bars share the left axis"
    );
    assert!((q1.rect.width / q2.rect.width - 0.5).abs() < 1e-9);
    assert!((q1.rect.height - q2.rect.height).abs() < 1e-9);
    let axis = named(&shapes, "Chart 2!axis");
    assert_eq!(axis.rect.width, 0.0);
    assert!((axis.rect.x - q1.rect.x).abs() < 1e-9);
}

#[test]
fn negative_values_hang_below_the_baseline() {
    let shapes = laid_out(chart(
        "barChart",
        false,
        vec![series("Net", &["A", "B"], &[Some(-5.0), Some(10.0)])],
    ));
    let a = named(&shapes, "Chart 2!Net/A");
    let b = named(&shapes, "Chart 2!Net/B");
    let axis = named(&shapes, "Chart 2!axis");
    assert!(
        (a.rect.y - axis.rect.y).abs() < 1e-9,
        "a negative bar starts at the axis"
    );
    assert!((b.rect.y + b.rect.height - axis.rect.y).abs() < 1e-9);
    assert!((a.rect.height / b.rect.height - 0.5).abs() < 1e-9);
}

#[test]
fn a_line_chart_joins_each_series_points_with_segments_in_its_colour() {
    let shapes = laid_out(chart(
        "lineChart",
        false,
        vec![
            series(
                "Revenue",
                &["Q1", "Q2", "Q3"],
                &[Some(1.0), Some(3.0), Some(2.0)],
            ),
            series(
                "Costs",
                &["Q1", "Q2", "Q3"],
                &[Some(2.0), Some(2.0), Some(2.0)],
            ),
        ],
    ));
    let first = named(&shapes, "Chart 2!Revenue 1");
    let second = named(&shapes, "Chart 2!Revenue 2");
    assert_eq!(first.geometry, PlacedGeometry::Line);
    assert_eq!(first.outline.as_ref().expect("stroke").color, "#4472C4");
    assert_eq!(
        named(&shapes, "Chart 2!Costs 1")
            .outline
            .as_ref()
            .expect("stroke")
            .color,
        "#ED7D31"
    );
    // Segment 1 rises (value 1 → 3): drawn from its lower-left corner up,
    // which the line geometry expresses with a vertical flip.
    assert!(first.flip_v);
    assert!(first.rect.height > 0.0);
    // Segment 2 falls (3 → 2): no flip, and it starts where segment 1 ended.
    assert!(!second.flip_v);
    assert!((second.rect.x - (first.rect.x + first.rect.width)).abs() < 1e-9);
    assert!((second.rect.y - first.rect.y).abs() < 1e-9);
    // A flat series is a flat segment.
    let flat = named(&shapes, "Chart 2!Costs 1");
    assert_eq!(flat.rect.height, 0.0);
}

#[test]
fn a_pie_chart_cuts_sectors_proportional_to_the_first_series() {
    let shapes = laid_out(chart(
        "pieChart",
        false,
        vec![series(
            "Share",
            &["A", "B", "C"],
            &[Some(1.0), Some(3.0), Some(4.0)],
        )],
    ));
    let a = named(&shapes, "Chart 2!Share/A");
    let b = named(&shapes, "Chart 2!Share/B");
    let c = named(&shapes, "Chart 2!Share/C");
    // Sectors are polygons from the centre, coloured per category, sampled
    // along the arc so a bigger share has more points.
    let points = |sector: &PlacedShape| match &sector.geometry {
        PlacedGeometry::Polygon(points) => points.len(),
        other => panic!("{other:?}"),
    };
    assert!(points(a) < points(b) && points(b) < points(c));
    assert_eq!(a.fill.as_deref(), Some("#4472C4"));
    assert_eq!(b.fill.as_deref(), Some("#ED7D31"));
    assert_eq!(c.fill.as_deref(), Some("#A5A5A5"));
    // All sectors share the pie's square box.
    assert!((a.rect.width - a.rect.height).abs() < 1e-9);
    assert_eq!(a.rect, b.rect);
    // No axis for a pie.
    assert!(!shapes.iter().any(|s| s.name == "Chart 2!axis"));
}

#[test]
fn an_unknown_kind_or_no_series_draws_only_the_frame() {
    let shapes = laid_out(chart(
        "bubbleChart",
        false,
        vec![series("x", &["a"], &[Some(1.0)])],
    ));
    assert_eq!(shapes.len(), 1);
    let shapes = laid_out(chart("barChart", false, Vec::new()));
    assert_eq!(shapes.len(), 1);
    // Values that are all missing draw no bars but keep the axis and labels.
    let shapes = laid_out(chart(
        "barChart",
        false,
        vec![series("x", &["a", "b"], &[None, None])],
    ));
    assert!(shapes.iter().any(|s| s.name == "Chart 2!axis"));
    assert!(!shapes.iter().any(|s| s.name.contains("x/")));
}

trait Positive {
    fn width_positive(&self) -> bool;
}

impl Positive for PlacedShape {
    fn width_positive(&self) -> bool {
        self.rect.width > 0.0 && self.rect.height >= 0.0
    }
}
