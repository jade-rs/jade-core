use jade_xlsx_layout::{
    chart::{chart_status, layout_chart, ChartOp},
    layout_workbook_sheet, LayoutOptions, Rect,
};
use jade_xlsx_reader::{read_xlsx, CellValue};
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/charts-workbook.xlsx");
fn placed(book: &jade_xlsx_reader::Workbook, index: usize) -> jade_xlsx_layout::chart::PlacedChart {
    layout_chart(
        book.sheets[0].shapes[index].chart.as_ref().unwrap(),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 280.0,
        },
        &book.sheets[0],
        Some(book),
        &LayoutOptions::default(),
        &book.styles,
    )
}
fn bars(chart: &jade_xlsx_layout::chart::PlacedChart) -> Vec<Rect> {
    chart
        .ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Rectangle {
                rect,
                fill: Some(fill),
                ..
            } if fill == "#00aa55" && rect.width > 10.0 => Some(*rect),
            _ => None,
        })
        .collect()
}
#[test]
fn current_workbook_values_replace_old_caches_and_edits_resize_the_chart() {
    let mut book = read_xlsx(BYTES).unwrap();
    let before = placed(&book, 0);
    assert_eq!(bars(&before).len(), 3);
    assert!(before.unsupported_properties.is_empty());
    assert!(!before.source_states.contains(&"cached".into()));
    book.sheets[0].rows[1].cells[1].value = CellValue::Number(35.0);
    let after = placed(&book, 0);
    assert!(bars(&after)[0].height > bars(&before)[0].height * 3.0);
    let layout = layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap();
    assert!(layout.content_height > 1500.0 && layout.content_width >= 1020.0);
    assert!(layout.shapes[0].chart.as_ref().unwrap().ops.len() > 10);
}
#[test]
fn blank_line_points_gap_span_and_zero_preserve_their_positions() {
    let mut book = read_xlsx(BYTES).unwrap();
    let segments = |chart: jade_xlsx_layout::chart::PlacedChart| {
        chart
            .ops
            .into_iter()
            .filter_map(|op| match op {
                ChartOp::Path {
                    points,
                    closed: false,
                    width: 2.0,
                    ..
                } => Some(points.len()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(segments(placed(&book, 2)), [1, 2]);
    book.sheets[0].shapes[2].chart.as_mut().unwrap().blank_mode = "span".into();
    assert_eq!(segments(placed(&book, 2)), [3]);
    book.sheets[0].shapes[2].chart.as_mut().unwrap().blank_mode = "zero".into();
    assert_eq!(segments(placed(&book, 2)), [4]);
}
#[test]
fn hidden_source_cells_are_excluded_and_cross_sheet_scatter_uses_current_xy() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.sheets[0].rows[2].hidden = true;
    let chart = placed(&book, 0);
    assert_eq!(bars(&chart).len(), 2);
    assert!(!chart
        .ops
        .iter()
        .any(|op| matches!(op,ChartOp::Text{text,..} if text=="Feb")));
    let xy = |chart: jade_xlsx_layout::chart::PlacedChart| {
        chart
            .ops
            .into_iter()
            .find_map(|op| match op {
                ChartOp::Path {
                    points,
                    closed: false,
                    width: 2.0,
                    ..
                } => Some(points),
                _ => None,
            })
            .unwrap()
    };
    let before = xy(placed(&book, 6));
    assert_eq!(before.len(), 3);
    assert!(before
        .windows(2)
        .all(|w| w[0].0 < w[1].0 && w[0].1 > w[1].1));
    book.sheets[1].rows[1].cells[1].value = CellValue::Number(20.0);
    assert_ne!(before, xy(placed(&book, 6)));
    assert!(!placed(&book, 6).source_states.contains(&"cached".into()));
}
#[test]
fn stack_percent_pie_doughnut_and_diagnostics_have_finite_geometry() {
    let book = read_xlsx(BYTES).unwrap();
    for i in [1, 3, 4, 5, 8, 9] {
        let chart = placed(&book, i);
        assert!(!chart.ops.is_empty());
        assert!(chart.ops.iter().all(|op| match op {
            ChartOp::Path { points, .. } =>
                points.iter().all(|(x, y)| x.is_finite() && y.is_finite()),
            ChartOp::Rectangle { rect, .. } => rect.width.is_finite() && rect.height.is_finite(),
            _ => true,
        }));
    }
    let pie = placed(&book, 3);
    let donut = placed(&book, 4);
    assert_eq!(
        pie.ops
            .iter()
            .filter(|op| matches!(op, ChartOp::Path { closed: true, .. }))
            .count(),
        3
    );
    assert!(donut
        .ops
        .iter()
        .any(|op| matches!(op,ChartOp::Path{points,closed:true,..} if points.len()>100)));
    let status = chart_status(&book);
    assert_eq!(status.len(), 10);
    assert_eq!(status.iter().filter(|s| s.rendered).count(), 9);
    assert!(status[7]
        .unsupported_properties
        .contains(&"combined or missing plot".into()));
}
#[test]
fn unresolved_and_stale_sources_are_reported_without_claiming_fresh_results() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.sheets[0].shapes[0].chart.as_mut().unwrap().series[0]
        .values
        .formula = Some("[external.xlsx]Charts!B2:B5".into());
    let chart = placed(&book, 0);
    assert!(chart.source_states.contains(&"cached".into()));
    assert!(chart
        .unsupported_properties
        .iter()
        .any(|p| p.contains("saved cache")));
    book.sheets[0].shapes[0].chart.as_mut().unwrap().series[0]
        .values
        .formula = Some("Charts!B2:B5".into());
    book.sheets[0].rows[1].cells[1].formula = Some("UNSUPPORTED()".into());
    book.full_calculation_on_load = true;
    assert!(placed(&book, 0).source_states.contains(&"stale".into()));
}

#[test]
fn empty_formula_results_take_precedence_over_stale_formula_caches() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.sheets[0].rows[1].cells[1].formula = Some("UNSUPPORTED()".into());
    book.sheets[0].rows[1].cells[1].value = CellValue::Empty;
    book.sheets[0].rows[2].cells[1].formula = Some("1+1".into());
    book.full_calculation_on_load = true;
    assert!(placed(&book, 0)
        .source_states
        .contains(&"uncalculated".into()));
}

#[test]
fn explicit_scheme_colors_resolve_even_without_a_series_rgb_override() {
    let mut book = read_xlsx(BYTES).unwrap();
    let color = &mut book.sheets[0].shapes[0].chart.as_mut().unwrap().series[0].color;
    color.rgb = None;
    color.scheme = Some("accent2".into());
    assert!(placed(&book,0).ops.iter().any(|op|matches!(op,ChartOp::Rectangle{rect,fill:Some(color),..} if color=="#6633ff" && rect.width>10.0)));
}

#[test]
fn zero_crossing_labels_paint_above_negative_bars() {
    let book = read_xlsx(BYTES).unwrap();
    let chart = placed(&book, 0);
    let feb = chart
        .ops
        .iter()
        .position(|op| matches!(op,ChartOp::Text{text,..} if text=="Feb"))
        .unwrap();
    let last_bar = chart
        .ops
        .iter()
        .rposition(|op| matches!(op,ChartOp::Rectangle{fill:Some(color),..} if color!="#ffffff"))
        .unwrap();
    assert!(feb > last_bar);
    assert!(chart
        .ops
        .iter()
        .take(feb)
        .any(|op| matches!(op, ChartOp::Restore)));
}

#[test]
fn numeric_category_labels_use_current_cell_formats_and_errors_do_not_become_zero_points() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.styles = jade_xlsx_reader::styles::parse_styles(
        r#"<styleSheet><cellXfs count="1"><xf numFmtId="2"/></cellXfs></styleSheet>"#,
    )
    .unwrap();
    book.sheets[0].rows[1].cells[0].value = CellValue::Number(12.5);
    book.sheets[0].rows[1].cells[0].style_index = Some(0);
    book.sheets[0].shapes[0].chart.as_mut().unwrap().series[0]
        .categories
        .numeric = true;
    assert!(placed(&book, 0)
        .ops
        .iter()
        .any(|op| matches!(op,ChartOp::Text{text,..} if text=="12.50")));
    let line = book.sheets[0].shapes[2].chart.as_mut().unwrap();
    line.blank_mode = "zero".into();
    line.series[0].values.formula = Some("Charts!B2:B5".into());
    book.sheets[0].rows[2].cells[1].value = CellValue::Error("#N/A".into());
    let chart = placed(&book, 2);
    let segments = chart
        .ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Path {
                points,
                closed: false,
                width: 2.0,
                ..
            } => Some(points.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(segments, [1, 2]);
}
