//! Data labels, axis titles, log and date axes, trendlines, error bars,
//! combination charts on a secondary axis, radar and bubble charts.
use jade_xlsx_layout::{
    chart::{layout_chart, ChartOp, PlacedChart},
    LayoutOptions, Rect,
};
use jade_xlsx_reader::{chart::parse_chart, read_xlsx};

const NS: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn numbers(values: &[f64]) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(index, value)| format!(r#"<c:pt idx="{index}"><c:v>{value}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="{}"/>{points}</c:numLit>"#,
        values.len()
    )
}

fn texts(values: &[&str]) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(index, value)| format!(r#"<c:pt idx="{index}"><c:v>{value}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:strLit><c:ptCount val="{}"/>{points}</c:strLit>"#,
        values.len()
    )
}

fn series(index: u32, values: &[f64], extra: &str) -> String {
    format!(
        r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/><c:tx><c:v>S{index}</c:v></c:tx>{extra}<c:cat>{}</c:cat><c:val>{}</c:val></c:ser>"#,
        texts(&["A", "B", "C", "D"][..values.len()]),
        numbers(values)
    )
}

fn axes(category: &str, value: &str) -> String {
    format!(
        r#"<c:catAx><c:axId val="1"/><c:delete val="0"/><c:axPos val="b"/>{category}<c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/>{value}<c:crossAx val="1"/></c:valAx>"#
    )
}

fn place(plot_area: &str) -> PlacedChart {
    let chart = parse_chart(&format!(
        r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot_area}</c:plotArea></c:chart></c:chartSpace>"#
    ))
    .unwrap();
    // Literal data only: any sheet will do.
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/charts-workbook.xlsx"
    ))
    .unwrap();
    layout_chart(
        &chart,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 320.0,
        },
        &book.sheets[0],
        None,
        &LayoutOptions::default(),
        &book.styles,
    )
}

fn texts_of(chart: &PlacedChart) -> Vec<String> {
    chart
        .ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn data_labels_show_values_except_deleted_points() {
    let chart = place(&format!(
        r#"<c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>{}"#,
        series(
            0,
            &[12.0, 19.0, 15.0],
            r#"<c:dLbls><c:dLbl><c:idx val="1"/><c:delete val="1"/></c:dLbl><c:showVal val="1"/><c:showCatName val="1"/></c:dLbls>"#
        ),
        axes("", "")
    ));
    let texts = texts_of(&chart);
    assert!(texts.contains(&"A, 12".to_string()), "{texts:?}");
    assert!(texts.contains(&"C, 15".to_string()));
    assert!(!texts.iter().any(|text| text == "B, 19"));
}

#[test]
fn axis_titles_read_across_and_upwards() {
    let chart = place(&format!(
        r#"<c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>{}"#,
        series(0, &[1.0, 2.0], ""),
        axes(
            r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>Quarter</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
            r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>"#
        )
    ));
    let rotation = |wanted: &str| {
        chart.ops.iter().find_map(|op| match op {
            ChartOp::Text { text, rotation, .. } if text == wanted => Some(*rotation),
            _ => None,
        })
    };
    assert_eq!(rotation("Quarter"), Some(0.0));
    assert_eq!(rotation("Revenue"), Some(-std::f64::consts::FRAC_PI_2));
}

#[test]
fn a_line_on_the_secondary_axis_uses_its_own_scale() {
    let chart = place(&format!(
        r#"<c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart>{}<c:axId val="3"/><c:axId val="4"/></c:lineChart>{}<c:catAx><c:axId val="3"/><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx><c:valAx><c:axId val="4"/><c:delete val="0"/><c:axPos val="r"/><c:numFmt formatCode="0%" sourceLinked="0"/><c:crossAx val="3"/><c:crosses val="max"/></c:valAx>"#,
        series(0, &[100.0, 200.0, 300.0, 400.0], ""),
        series(1, &[0.1, 0.2, 0.3, 0.4], ""),
        axes("", "")
    ));
    let line = chart
        .ops
        .iter()
        .find_map(|op| match op {
            ChartOp::Path {
                points,
                closed: false,
                width: 2.0,
                ..
            } => Some(points.clone()),
            _ => None,
        })
        .expect("the line is drawn");
    assert_eq!(line.len(), 4);
    // 0.4 on a 0–40% axis reaches the top, not the bottom of a 0–400 one.
    assert!(line[3].1 < chart.plot.y + chart.plot.height * 0.1);
    let texts = texts_of(&chart);
    assert!(texts.contains(&"40%".to_string()), "{texts:?}");
    assert!(texts.contains(&"400".to_string()));
}

#[test]
fn log_axes_tick_at_powers_of_the_base() {
    let chart = place(&format!(
        r#"<c:lineChart>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart>{}"#,
        series(0, &[1.0, 50.0, 900.0, 3000.0], ""),
        axes("", "").replace(
            r#"<c:axId val="2"/>"#,
            r#"<c:axId val="2"/><c:scaling><c:logBase val="10"/></c:scaling>"#
        )
    ));
    let texts = texts_of(&chart);
    for tick in ["1", "10", "100", "1000", "10000"] {
        assert!(texts.contains(&tick.to_string()), "{tick} in {texts:?}");
    }
    assert!(!texts.contains(&"5000".to_string()));
}

#[test]
fn a_date_axis_spaces_points_by_date() {
    // 1 Jan, 1 Feb and 1 Apr 2024: March is an empty month on the axis.
    let chart = place(&format!(
        r#"<c:lineChart><c:ser><c:idx val="0"/><c:order val="0"/><c:cat>{}</c:cat><c:val>{}</c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:dateAx><c:axId val="1"/><c:delete val="0"/><c:axPos val="b"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/><c:crossAx val="2"/></c:dateAx><c:valAx><c:axId val="2"/><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>"#,
        numbers(&[45292.0, 45323.0, 45383.0]),
        numbers(&[1.0, 2.0, 3.0])
    ));
    let texts = texts_of(&chart);
    assert!(texts.contains(&"Mar-24".to_string()), "{texts:?}");
    let line = chart
        .ops
        .iter()
        .find_map(|op| match op {
            ChartOp::Path {
                points,
                closed: false,
                width: 2.0,
                ..
            } => Some(points.clone()),
            _ => None,
        })
        .unwrap();
    let first_gap = line[1].0 - line[0].0;
    let second_gap = line[2].0 - line[1].0;
    assert!((second_gap - 2.0 * first_gap).abs() < 1.0);
}

#[test]
fn trendlines_and_error_bars_draw_over_their_series() {
    let chart = place(&format!(
        r#"<c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>{}"#,
        series(
            0,
            &[2.0, 4.0, 6.0, 8.0],
            r#"<c:trendline><c:trendlineType val="linear"/><c:dispEq val="1"/><c:dispRSqr val="1"/></c:trendline><c:errBars><c:errDir val="y"/><c:errBarType val="both"/><c:errValType val="fixedVal"/><c:val val="1"/></c:errBars>"#
        ),
        axes("", "")
    ));
    let dashed: Vec<_> = chart
        .ops
        .iter()
        .filter(|op| matches!(op, ChartOp::Path { dash, .. } if !dash.is_empty()))
        .collect();
    assert_eq!(dashed.len(), 1);
    let texts = texts_of(&chart);
    assert!(texts.contains(&"y = 2x + 0".to_string()), "{texts:?}");
    assert!(texts.contains(&"R² = 1.0000".to_string()));
    // Four bars, each with a stem and two caps.
    let thin_lines = chart
        .ops
        .iter()
        .filter(|op| {
            matches!(op, ChartOp::Path { points, stroke: Some(color), width, .. }
                if points.len() == 2 && *width == 1.0 && color == "#595959")
        })
        .count();
    assert_eq!(thin_lines, 12);
}

#[test]
fn radar_charts_draw_a_web_and_a_closed_line_per_series() {
    let chart = place(&format!(
        r#"<c:radarChart><c:radarStyle val="marker"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:radarChart>{}"#,
        series(0, &[1.0, 2.0, 3.0, 4.0], ""),
        series(1, &[4.0, 3.0, 2.0, 1.0], ""),
        axes("", "")
    ));
    assert!(!chart.ops.is_empty());
    let series_lines = chart
        .ops
        .iter()
        .filter(|op| {
            matches!(op, ChartOp::Path { points, closed: true, width, fill: None, .. }
                if points.len() == 4 && *width == 2.0)
        })
        .count();
    assert_eq!(series_lines, 2);
    let rings = chart
        .ops
        .iter()
        .filter(|op| {
            matches!(op, ChartOp::Path { closed: true, stroke: Some(color), .. } if color == "#d9d9d9")
        })
        .count();
    assert!(rings >= 3);
}

#[test]
fn bubbles_grow_with_their_size() {
    let chart = place(&format!(
        r#"<c:bubbleChart><c:ser><c:idx val="0"/><c:order val="0"/><c:xVal>{}</c:xVal><c:yVal>{}</c:yVal><c:bubbleSize>{}</c:bubbleSize></c:ser><c:axId val="1"/><c:axId val="2"/></c:bubbleChart><c:valAx><c:axId val="1"/><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:valAx><c:valAx><c:axId val="2"/><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>"#,
        numbers(&[1.0, 2.0, 3.0]),
        numbers(&[1.0, 2.0, 3.0]),
        numbers(&[1.0, 4.0, 16.0])
    ));
    let radii: Vec<f64> = chart
        .ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Path {
                points,
                closed: true,
                ..
            } if points.len() == 48 => {
                let xs = points.iter().map(|p| p.0);
                let (low, high) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                    (a.min(x), b.max(x))
                });
                Some((high - low) / 2.0)
            }
            _ => None,
        })
        .collect();
    assert_eq!(radii.len(), 3);
    // Area follows size: 4× the size is twice the radius.
    assert!((radii[1] / radii[0] - 2.0).abs() < 0.05, "{radii:?}");
    assert!((radii[2] / radii[1] - 2.0).abs() < 0.05);
    // Every bubble fits inside the plot.
    for op in &chart.ops {
        if let ChartOp::Path {
            points,
            closed: true,
            ..
        } = op
        {
            if points.len() == 48 {
                assert!(points.iter().all(|(x, _)| *x >= chart.plot.x - 0.5
                    && *x <= chart.plot.x + chart.plot.width + 0.5));
            }
        }
    }
}
