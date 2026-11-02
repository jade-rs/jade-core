//! Charts formatted as Excel's Chart Elements, Chart Styles and Format
//! panes do: axis titles, bounds, units, a logarithmic or reversed scale,
//! where the axes cross, gridlines major and minor; data labels; the data
//! table; a series' fill, line, markers, gap, trendline and error bars;
//! how empty and hidden cells are drawn; the chart and plot area's fill;
//! the chart style. Each is read back as made, drawn, saved as Excel
//! writes it, and one undo step.

use jade_xlsx_headless_editor::{
    AxisFormat, AxisId, AxisKind, ChartChange, ChartFormat, ChartKind, Crosses, ErrorBarFormat,
    Fill, HeadlessWorkbook, LabelFormat, SeriesFormat, TableFormat, TrendlineFormat,
};
use jade_xlsx_layout::chart::{layout_chart, ChartOp};
use jade_xlsx_layout::{LayoutOptions, Rect};
use jade_xlsx_reader::chart::{AreaFill, Chart, ChartColor};
use std::path::PathBuf;

/// Operations!A1:C4: Name, Amount, Double over Alpha, Beta, Gamma; 10, 20,
/// 30; and twice those.
const SHEET: &str = "Operations";

fn open(fixture: &str) -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(fixture);
    HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open")
}

fn chart(book: &HeadlessWorkbook, id: u32) -> Chart {
    book.workbook().sheets[0]
        .shapes
        .iter()
        .find(|shape| shape.id == id)
        .and_then(|shape| shape.chart.clone())
        .expect("a chart")
}

fn painted(book: &HeadlessWorkbook, id: u32) -> Vec<ChartOp> {
    let frame = Rect {
        x: 0.0,
        y: 0.0,
        width: 480.0,
        height: 288.0,
    };
    layout_chart(
        &chart(book, id),
        frame,
        &book.workbook().sheets[0],
        Some(book.workbook()),
        &LayoutOptions::default(),
        &book.workbook().styles,
    )
    .ops
}

fn texts(ops: &[ChartOp]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            ChartOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn format(book: &mut HeadlessWorkbook, id: u32, format: Vec<ChartFormat>) {
    book.set_chart(
        SHEET,
        id,
        &ChartChange {
            format,
            ..ChartChange::default()
        },
    )
    .unwrap();
}

/// What a reopened copy of the book reads.
fn reopened(book: &mut HeadlessWorkbook, id: u32) -> Chart {
    let bytes = book.export_bytes().unwrap();
    chart(&HeadlessWorkbook::open(&bytes).unwrap(), id)
}

fn column_chart() -> (HeadlessWorkbook, u32) {
    let mut book = open("grid-operations-workbook.xlsx");
    let id = book
        .insert_chart(SHEET, "A1:C4", ChartKind::Column)
        .unwrap();
    (book, id)
}

#[test]
fn format_axis_sets_titles_bounds_units_and_gridlines() {
    let (mut book, id) = column_chart();
    format(
        &mut book,
        id,
        vec![
            ChartFormat::Axis(
                AxisId::Value,
                AxisFormat {
                    title: Some(Some("Amount".into())),
                    minimum: Some(Some(0.0)),
                    maximum: Some(Some(80.0)),
                    major_unit: Some(Some(20.0)),
                    minor_unit: Some(Some(5.0)),
                    minor_gridlines: Some(true),
                    number_format: Some(Some("#,##0.0".into())),
                    ..AxisFormat::default()
                },
            ),
            ChartFormat::Axis(
                AxisId::Category,
                AxisFormat {
                    title: Some(Some("Name".into())),
                    reverse: Some(true),
                    // A category axis has no units: they are not written.
                    major_unit: Some(Some(3.0)),
                    ..AxisFormat::default()
                },
            ),
        ],
    );
    for read in [chart(&book, id), reopened(&mut book, id)] {
        let value = &read.value_axis;
        assert_eq!(value.title.as_deref(), Some("Amount"));
        assert_eq!(value.minimum.as_deref(), Some("0"));
        assert_eq!(value.maximum.as_deref(), Some("80"));
        assert_eq!(value.major_unit.as_deref(), Some("20"));
        assert_eq!(value.minor_unit.as_deref(), Some("5"));
        assert!(value.gridlines && value.minor_gridlines);
        assert_eq!(value.number_format.as_deref(), Some("#,##0.0"));
        assert_eq!(read.category_axis.title.as_deref(), Some("Name"));
        assert!(read.category_axis.reverse);
        assert_eq!(read.category_axis.major_unit, None);
    }
    let ops = painted(&book, id);
    let labels = texts(&ops);
    for tick in ["0.0", "20.0", "40.0", "60.0", "80.0", "Amount", "Name"] {
        assert!(labels.iter().any(|t| t == tick), "{tick} in {labels:?}");
    }
    // Minor gridlines every 5, under the major ones.
    let minor = ops
        .iter()
        .filter(|op| matches!(op, ChartOp::Path { stroke: Some(c), .. } if c == "#f2f2f2"))
        .count();
    assert_eq!(minor, 17);
    // Gridlines off again, and the title gone.
    format(
        &mut book,
        id,
        vec![ChartFormat::Axis(
            AxisId::Value,
            AxisFormat {
                title: Some(None),
                major_gridlines: Some(false),
                minor_gridlines: Some(false),
                maximum: Some(None),
                ..AxisFormat::default()
            },
        )],
    );
    let read = chart(&book, id);
    assert_eq!(read.value_axis.title, None);
    assert!(!read.value_axis.gridlines && !read.value_axis.minor_gridlines);
    assert_eq!(read.value_axis.maximum, None);
    assert!(book.undo().unwrap());
    assert_eq!(chart(&book, id).value_axis.title.as_deref(), Some("Amount"));
}

#[test]
fn a_log_scale_a_crossing_and_a_date_axis() {
    let (mut book, id) = column_chart();
    format(
        &mut book,
        id,
        vec![
            ChartFormat::Axis(
                AxisId::Value,
                AxisFormat {
                    log_base: Some(Some(10.0)),
                    ..AxisFormat::default()
                },
            ),
            // The horizontal axis crosses the vertical at 15: on the
            // category axis, as Excel saves it.
            ChartFormat::Axis(
                AxisId::Category,
                AxisFormat {
                    crosses: Some(Crosses::At(15.0)),
                    kind: Some(AxisKind::Text),
                    ..AxisFormat::default()
                },
            ),
        ],
    );
    let read = reopened(&mut book, id);
    assert_eq!(read.value_axis.log_base.as_deref(), Some("10"));
    assert_eq!(read.category_axis.crosses_at.as_deref(), Some("15"));
    assert_eq!(read.category_axis.crosses, None);
    let labels = texts(&painted(&book, id));
    assert!(labels.iter().any(|t| t == "100"), "{labels:?}");
    format(
        &mut book,
        id,
        vec![ChartFormat::Axis(
            AxisId::Category,
            AxisFormat {
                crosses: Some(Crosses::Maximum),
                kind: Some(AxisKind::Date),
                ..AxisFormat::default()
            },
        )],
    );
    let read = reopened(&mut book, id);
    assert_eq!(read.category_axis.crosses.as_deref(), Some("max"));
    assert_eq!(read.category_axis.crosses_at, None);
    assert!(read.category_axis.date);
}

#[test]
fn data_labels_and_the_data_table() {
    let (mut book, id) = column_chart();
    format(
        &mut book,
        id,
        vec![
            ChartFormat::Labels(Some(LabelFormat {
                value: true,
                position: Some("outEnd".into()),
                ..LabelFormat::default()
            })),
            ChartFormat::Table(Some(TableFormat { keys: true })),
        ],
    );
    let read = reopened(&mut book, id);
    let labels = read.data_labels.as_ref().expect("labels");
    assert!(labels.show_value && !labels.show_category);
    assert_eq!(labels.position.as_deref(), Some("outEnd"));
    let table = read.data_table.as_ref().expect("a data table");
    assert!(table.keys && table.outline && table.horizontal_borders);
    // The table shows each series' name and values under the plot.
    let shown = texts(&painted(&book, id));
    for text in ["Amount", "Double", "Alpha", "10", "60"] {
        assert!(shown.iter().any(|t| t == text), "{text} in {shown:?}");
    }
    format(
        &mut book,
        id,
        vec![ChartFormat::Labels(None), ChartFormat::Table(None)],
    );
    let read = reopened(&mut book, id);
    assert!(read.data_labels.is_none() && read.data_table.is_none());
}

#[test]
fn format_data_series_fills_lines_gaps_trendlines_and_error_bars() {
    let (mut book, id) = column_chart();
    format(
        &mut book,
        id,
        vec![
            ChartFormat::Series(
                0,
                SeriesFormat {
                    fill: Some(Some(Fill::Gradient("FF0000".into(), "0000FF".into(), 90))),
                    line_color: Some(Some(Some("000000".into()))),
                    line_width: Some(1.5),
                    line_dash: Some("dash".into()),
                    gap_width: Some(50),
                    overlap: Some(-20),
                    trendline: Some(Some(TrendlineFormat {
                        kind: "linear".into(),
                        order: 2,
                        period: 2,
                        forward: Some(1.0),
                        backward: None,
                        display_equation: true,
                        display_r_squared: true,
                    })),
                    error_bars: Some(Some(ErrorBarFormat {
                        direction: "y".into(),
                        bar_type: "both".into(),
                        value_type: "percentage".into(),
                        value: 10.0,
                        end_cap: true,
                    })),
                    ..SeriesFormat::default()
                },
            ),
            ChartFormat::Series(
                1,
                SeriesFormat {
                    fill: Some(Some(Fill::Pattern(
                        "pct50".into(),
                        "00B050".into(),
                        "FFFFFF".into(),
                    ))),
                    ..SeriesFormat::default()
                },
            ),
        ],
    );
    let read = reopened(&mut book, id);
    assert_eq!(read.gap_width, 50);
    assert_eq!(read.overlap, -20);
    let first = &read.series[0];
    match &first.fill {
        Some(AreaFill::Gradient(stops, angle)) => {
            assert_eq!(stops.len(), 2);
            assert_eq!(stops[0].1.rgb, Some([255, 0, 0]));
            assert_eq!(stops[1].1.rgb, Some([0, 0, 255]));
            assert_eq!(*angle, 5_400_000);
        }
        other => panic!("a gradient, not {other:?}"),
    }
    assert_eq!(first.line.width, Some(19_050));
    assert_eq!(first.line.dash.as_deref(), Some("dash"));
    let trend = &first.trendlines[0];
    assert_eq!(trend.kind, "linear");
    assert!(trend.display_equation && trend.display_r_squared);
    assert_eq!(trend.forward.as_deref(), Some("1"));
    let bars = &first.error_bars[0];
    assert_eq!(bars.value_type, "percentage");
    assert_eq!(bars.value.as_deref(), Some("10"));
    assert!(!bars.no_end_cap);
    assert!(matches!(
        &read.series[1].fill,
        Some(AreaFill::Pattern(preset, ChartColor { rgb: Some([0, 176, 80]), .. }, _)) if preset == "pct50"
    ));
    // The gradient is drawn: bands from red to blue.
    let ops = painted(&book, id);
    let fills: Vec<&str> = ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Rectangle { fill: Some(fill), .. } => Some(fill.as_str()),
            _ => None,
        })
        .collect();
    assert!(fills.contains(&"#fa0005") && fills.contains(&"#0500fa"), "{fills:?}");
    assert!(fills.contains(&"#80d8a8"), "{fills:?}");
    // R² and the equation are drawn by the trendline.
    assert!(texts(&ops).iter().any(|t| t.starts_with("R²")));
    // Taken off again.
    format(
        &mut book,
        id,
        vec![ChartFormat::Series(
            0,
            SeriesFormat {
                fill: Some(None),
                trendline: Some(None),
                error_bars: Some(None),
                ..SeriesFormat::default()
            },
        )],
    );
    let read = reopened(&mut book, id);
    assert!(read.series[0].fill.is_none());
    assert!(read.series[0].trendlines.is_empty() && read.series[0].error_bars.is_empty());
}

#[test]
fn line_markers_and_smoothing() {
    let mut book = open("grid-operations-workbook.xlsx");
    let id = book
        .insert_chart(SHEET, "A1:C4", ChartKind::LineWithMarkers)
        .unwrap();
    format(
        &mut book,
        id,
        vec![ChartFormat::Series(
            1,
            SeriesFormat {
                marker: Some(("diamond".into(), 9)),
                smooth: Some(true),
                line_color: Some(Some(None)),
                ..SeriesFormat::default()
            },
        )],
    );
    let read = reopened(&mut book, id);
    assert_eq!(read.series[1].marker, "diamond");
    assert_eq!(read.series[1].marker_size, 9);
    assert!(read.series[1].smooth);
    assert!(read.series[1].line.none);
}

#[test]
fn hidden_and_empty_cells_area_fills_and_styles() {
    let (mut book, id) = column_chart();
    format(
        &mut book,
        id,
        vec![
            ChartFormat::Blanks("span".into(), true),
            ChartFormat::AreaFill(false, Some(Fill::Solid("FFF2CC".into()))),
            ChartFormat::AreaFill(true, Some(Fill::Solid("DDEBF7".into()))),
            ChartFormat::Style(3),
        ],
    );
    let read = reopened(&mut book, id);
    assert_eq!(read.blank_mode, "span");
    assert!(!read.visible_only);
    assert!(matches!(
        &read.chart_fill,
        Some(AreaFill::Solid(ChartColor { rgb: Some([255, 242, 204]), .. }))
    ));
    assert!(matches!(
        &read.plot_fill,
        Some(AreaFill::Solid(ChartColor { rgb: Some([221, 235, 247]), .. }))
    ));
    assert_eq!(read.style, Some(3));
    let ops = painted(&book, id);
    let fills: Vec<&str> = ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Rectangle { fill: Some(fill), .. } => Some(fill.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fills.first().copied(), Some("#fff2cc"));
    assert!(fills.contains(&"#ddebf7"));
    format(&mut book, id, vec![ChartFormat::AreaFill(false, None)]);
    assert!(chart(&book, id).chart_fill.is_none());
}
