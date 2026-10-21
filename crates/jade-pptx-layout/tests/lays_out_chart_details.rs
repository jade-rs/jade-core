//! What a chart part states beyond its series, followed as PowerPoint
//! draws it: data labels, axis titles, scaling, number formats, gap width
//! and overlap, markers and smoothing, a combo's secondary axis, radar and
//! bubble plots.

use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedGeometry, PlacedShape};
use jade_pptx_reader::{
    parse_chart_xml, Chart, ShapeItem, SlideContent, SlideSize, ThemeColors, Transform,
};

const NS: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn categories() -> &'static str {
    r#"<c:cat><c:strLit><c:ptCount val="3"/><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt><c:pt idx="2"><c:v>C</c:v></c:pt></c:strLit></c:cat>"#
}

fn values(values: &[f64], code: &str) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:val><c:numRef><c:f>S!B1</c:f><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="{}"/>{points}</c:numCache></c:numRef></c:val>"#,
        values.len()
    )
}

fn series(index: usize, name: &str, body: &str) -> String {
    format!(
        r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/><c:tx><c:v>{name}</c:v></c:tx>{body}</c:ser>"#
    )
}

const SHOW_VALUES: &str = r#"<c:dLbls><c:showLegendKey val="0"/><c:showVal val="1"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="0"/><c:showBubbleSize val="0"/></c:dLbls>"#;

fn axes(category: &str, value: &str) -> String {
    format!(
        r#"<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/>{category}<c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/>{value}<c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>"#
    )
}

fn chart_xml(plot_area: &str) -> String {
    format!(
        r#"<c:chartSpace {NS}><c:chart><c:autoTitleDeleted val="1"/><c:plotArea>{plot_area}</c:plotArea></c:chart></c:chartSpace>"#
    )
}

fn laid_out(xml: &str) -> Vec<PlacedShape> {
    let content = parse_chart_xml(xml).expect("chart part");
    let chart = Chart {
        id: Some(3),
        name: "C".into(),
        transform: Some(Transform {
            offset_emu: (914_400, 914_400),
            extent_emu: (6_096_000, 4_064_000),
            rotation_60000ths: 0,
            flip_h: false,
            flip_v: false,
        }),
        chart_rel_id: None,
        chart_part: None,
        kind: content.kind,
        title: content.title,
        horizontal: content.horizontal,
        series: content.series,
        style: content.style,
    };
    layout_slide(
        SlideSize {
            width_emu: 12_192_000,
            height_emu: 6_858_000,
        },
        &SlideContent {
            shapes: vec![ShapeItem::Chart(chart)],
            ..SlideContent::default()
        },
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
    .shapes
}

fn named<'a>(shapes: &'a [PlacedShape], name: &str) -> &'a PlacedShape {
    shapes
        .iter()
        .find(|shape| shape.name == format!("C!{name}"))
        .unwrap_or_else(|| {
            panic!(
                "no {name}; have {:?}",
                shapes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
            )
        })
}

fn text(shape: &PlacedShape) -> String {
    shape
        .lines
        .iter()
        .map(|line| {
            line.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bar_chart(extra: &str, value_axis: &str) -> String {
    chart_xml(&format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}{extra}<c:axId val="1"/><c:axId val="2"/></c:barChart>{}"#,
        series(
            0,
            "S1",
            &format!(
                "{}{}",
                categories(),
                values(&[1200.0, 2500.0, 1800.0], "#,##0")
            )
        ),
        series(
            1,
            "S2",
            &format!(
                "{}{}",
                categories(),
                values(&[600.0, 900.0, 300.0], "#,##0")
            )
        ),
        axes("", value_axis),
    ))
}

#[test]
fn data_labels_sit_above_columns_in_the_source_number_format() {
    let shapes = laid_out(&bar_chart(
        SHOW_VALUES,
        r#"<c:scaling><c:orientation val="minMax"/></c:scaling>"#,
    ));
    let bar = named(&shapes, "S1/B");
    let label = named(&shapes, "S1 label 1");
    assert_eq!(text(label), "2,500");
    // Outside end by default: just above the column's top, centred on it.
    assert!(label.rect.y + label.rect.height <= bar.rect.y + 1e-6);
    let centre = |s: &PlacedShape| s.rect.x + s.rect.width / 2.0;
    assert!((centre(label) - centre(bar)).abs() < 1e-6);
}

#[test]
fn scaling_min_max_and_major_unit_set_the_value_axis_and_its_format() {
    let shapes = laid_out(&bar_chart(
        "",
        r#"<c:scaling><c:orientation val="minMax"/><c:max val="4000"/><c:min val="0"/></c:scaling><c:majorGridlines/><c:numFmt formatCode="&quot;$&quot;#,##0" sourceLinked="0"/><c:majorUnit val="1000"/><c:tickLblPos val="nextTo"/>"#,
    ));
    for tick in ["$0", "$1,000", "$2,000", "$3,000", "$4,000"] {
        named(&shapes, &format!("value {tick}"));
    }
    assert!(!shapes.iter().any(|s| s.name == "C!value $5,000"));
    // 2500 of 4000: the column reaches five eighths of the axis.
    let bar = named(&shapes, "S1/B");
    let top = named(&shapes, "value $4,000");
    let bottom = named(&shapes, "value $0");
    let mid = |s: &PlacedShape| s.rect.y + s.rect.height / 2.0;
    let fraction = (bar.rect.height) / (mid(bottom) - mid(top));
    assert!((fraction - 0.625).abs() < 1e-6, "{fraction}");
}

#[test]
fn a_reversed_value_axis_hangs_columns_from_the_top() {
    let shapes = laid_out(&bar_chart(
        "",
        r#"<c:scaling><c:orientation val="maxMin"/></c:scaling>"#,
    ));
    let axis = named(&shapes, "axis");
    let bar = named(&shapes, "S1/A");
    // Zero is at the top now; the column grows down from it.
    assert!((bar.rect.y - axis.rect.y).abs() < 1e-6);
    let small = named(&shapes, "S2/A");
    assert!(small.rect.height < bar.rect.height);
}

#[test]
fn gap_width_and_overlap_size_the_columns() {
    let plain = laid_out(&bar_chart(r#"<c:gapWidth val="150"/>"#, ""));
    let a1 = named(&plain, "S1/A");
    let a2 = named(&plain, "S2/A");
    let b1 = named(&plain, "S1/B");
    let slot = b1.rect.x - a1.rect.x;
    // Two bars and a gap of 1.5 bars share the slot.
    assert!((a1.rect.width - slot / 3.5).abs() < 1e-6);
    assert!((a2.rect.x - (a1.rect.x + a1.rect.width)).abs() < 1e-6);
    let overlapped = laid_out(&bar_chart(
        r#"<c:gapWidth val="50"/><c:overlap val="50"/>"#,
        "",
    ));
    let a1 = named(&overlapped, "S1/A");
    let a2 = named(&overlapped, "S2/A");
    // Half of each bar overlaps the next: 1.5 lanes plus half a bar of gap.
    assert!((a1.rect.width - slot / 2.0).abs() < 1e-6);
    assert!((a2.rect.x - (a1.rect.x + a1.rect.width / 2.0)).abs() < 1e-6);
}

#[test]
fn axis_titles_are_placed_and_the_value_title_reads_upwards() {
    let xml = chart_xml(&format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>{}"#,
        series(
            0,
            "S1",
            &format!("{}{}", categories(), values(&[1.0, 2.0, 3.0], "General"))
        ),
        axes(
            r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Region</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
            r#"<c:scaling><c:orientation val="minMax"/></c:scaling><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Units</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
        ),
    ));
    let shapes = laid_out(&xml);
    let units = named(&shapes, "axis title Units");
    assert_eq!(units.text_rotation_deg, 270.0);
    assert!(units.rect.height > units.rect.width);
    let region = named(&shapes, "axis title Region");
    assert_eq!(text(region), "Region");
    let label = named(&shapes, "label A");
    assert!(region.rect.y >= label.rect.y + label.rect.height - 1e-6);
    // The value title sits left of the plot.
    let bar = named(&shapes, "S1/A");
    assert!(units.rect.x + units.rect.width <= bar.rect.x);
}

#[test]
fn a_combo_line_draws_on_its_own_secondary_axis_smoothed_with_markers() {
    let line = series(
        1,
        "Rate",
        &format!(
            r#"<c:marker><c:symbol val="circle"/><c:size val="6"/></c:marker>{}{}<c:smooth val="1"/>"#,
            categories(),
            values(&[0.2, 0.5, 0.4], "0%")
        ),
    );
    let xml = chart_xml(&format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{line}<c:marker val="1"/><c:axId val="3"/><c:axId val="4"/></c:lineChart>{}<c:catAx><c:axId val="3"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx><c:valAx><c:axId val="4"/><c:scaling><c:orientation val="minMax"/><c:max val="1"/><c:min val="0"/></c:scaling><c:delete val="0"/><c:axPos val="r"/><c:numFmt formatCode="0%" sourceLinked="0"/><c:majorUnit val="0.25"/><c:tickLblPos val="nextTo"/><c:crossAx val="3"/><c:crosses val="max"/></c:valAx>"#,
        series(
            0,
            "Sales",
            &format!(
                "{}{}",
                categories(),
                values(&[100.0, 300.0, 200.0], "General")
            )
        ),
        axes(
            "",
            r#"<c:scaling><c:orientation val="minMax"/></c:scaling><c:tickLblPos val="nextTo"/>"#
        ),
    ));
    let shapes = laid_out(&xml);
    // The secondary axis' own ticks, right of the plot.
    let fifty = named(&shapes, "secondary value 50%");
    let bar = named(&shapes, "Sales/B");
    assert!(fifty.rect.x >= bar.rect.x + bar.rect.width);
    // One smoothed path, not segments.
    let curve = named(&shapes, "Rate line");
    let PlacedGeometry::Paths(paths) = &curve.geometry else {
        panic!("a path");
    };
    assert!(paths[0].subpaths[0].points.len() > 3);
    assert!(!shapes.iter().any(|s| s.name == "C!Rate 1"));
    // Markers at each point, on the secondary scale: 0.5 is the axis'
    // middle.
    let marker = named(&shapes, "Rate marker 1");
    assert_eq!(marker.geometry, PlacedGeometry::Ellipse);
    let zero = named(&shapes, "secondary value 0%");
    let full = named(&shapes, "secondary value 100%");
    let mid = |s: &PlacedShape| s.rect.y + s.rect.height / 2.0;
    assert!((mid(marker) - (mid(zero) + mid(full)) / 2.0).abs() < 1e-6);
}

#[test]
fn a_pie_labels_its_slices_with_category_and_percentage() {
    let xml = chart_xml(&format!(
        r#"<c:pieChart><c:varyColors val="1"/>{}<c:firstSliceAng val="0"/></c:pieChart>"#,
        series(
            0,
            "Share",
            &format!(
                r#"<c:dLbls><c:showLegendKey val="0"/><c:showVal val="0"/><c:showCatName val="1"/><c:showSerName val="0"/><c:showPercent val="1"/><c:showBubbleSize val="0"/><c:separator>; </c:separator></c:dLbls>{}{}"#,
                categories(),
                values(&[1.0, 1.0, 2.0], "General")
            )
        ),
    ));
    let shapes = laid_out(&xml);
    assert_eq!(text(named(&shapes, "Share label 2")), "C; 50%");
    assert_eq!(text(named(&shapes, "Share label 0")), "A; 25%");
}

#[test]
fn a_radar_draws_a_web_and_each_series_as_a_closed_outline() {
    let xml = chart_xml(&format!(
        r#"<c:radarChart><c:radarStyle val="marker"/>{}<c:axId val="1"/><c:axId val="2"/></c:radarChart>{}"#,
        series(
            0,
            "R",
            &format!("{}{}", categories(), values(&[1.0, 2.0, 3.0], "General"))
        ),
        axes(
            "",
            r#"<c:scaling><c:orientation val="minMax"/></c:scaling><c:majorGridlines/>"#
        ),
    ));
    let shapes = laid_out(&xml);
    let line = named(&shapes, "R line");
    let PlacedGeometry::Paths(paths) = &line.geometry else {
        panic!("an outline");
    };
    assert!(paths[0].subpaths[0].closed);
    assert_eq!(paths[0].subpaths[0].points.len(), 3);
    named(&shapes, "spoke 2");
    named(&shapes, "ring 1");
    // `marker` style: automatic markers on every point.
    named(&shapes, "R marker 0");
    // The first category points straight up from the centre.
    let a = named(&shapes, "label A");
    let b = named(&shapes, "label B");
    assert!(a.rect.y < b.rect.y);
}

#[test]
fn bubbles_grow_by_area_with_their_size() {
    let xml = chart_xml(
        r#"<c:bubbleChart><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>B</c:v></c:tx>
        <c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:xVal>
        <c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:yVal>
        <c:bubbleSize><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>4</c:v></c:pt></c:numLit></c:bubbleSize></c:ser>
        <c:bubbleScale val="100"/><c:axId val="1"/><c:axId val="2"/></c:bubbleChart>
        <c:valAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:valAx>
        <c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>"#,
    );
    let shapes = laid_out(&xml);
    let small = named(&shapes, "B bubble 0");
    let big = named(&shapes, "B bubble 1");
    assert_eq!(big.geometry, PlacedGeometry::Ellipse);
    // Four times the size, twice the diameter.
    assert!((big.rect.width / small.rect.width - 2.0).abs() < 1e-6);
    // Further right and higher.
    assert!(big.rect.x > small.rect.x && big.rect.y < small.rect.y);
}
