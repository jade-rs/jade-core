//! Charts made and changed as Excel's Insert ▸ Chart and Chart Design do:
//! a chart over a selected range — names from its first row, categories
//! from its first column, a series per column (or per row, when the range
//! is wider than tall) — beside the data; then its type, title, legend and
//! data range changed. Each draws, saves as Excel writes it, reads back the
//! same, and is one undo step.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    ChartChange, ChartKind, HeadlessError, HeadlessWorkbook, LegendPosition,
};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::chart::Chart;
use jade_xlsx_reader::CellRef;
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

fn chart(book: &HeadlessWorkbook, sheet: usize, id: u32) -> Chart {
    book.workbook().sheets[sheet]
        .shapes
        .iter()
        .find(|shape| shape.id == id)
        .and_then(|shape| shape.chart.clone())
        .expect("a chart")
}

fn formulas(chart: &Chart) -> Vec<(Option<String>, Option<String>, Option<String>)> {
    chart
        .series
        .iter()
        .map(|series| {
            (
                series.name.formula.clone(),
                series.categories.formula.clone(),
                series.values.formula.clone(),
            )
        })
        .collect()
}

fn drawn(book: &HeadlessWorkbook, id: u32) -> bool {
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    let everything = Rect {
        x: 0.0,
        y: 0.0,
        width: 1e6,
        height: 1e6,
    };
    let drawn = layout
        .shapes_in(everything)
        .any(|shape| shape.id == id && shape.chart.is_some());
    drawn
}

#[test]
fn a_column_chart_over_a_range_takes_its_names_and_categories() {
    let mut book = open("grid-operations-workbook.xlsx");
    let id = book
        .insert_chart(SHEET, "A1:C4", ChartKind::Column)
        .unwrap();
    let made = chart(&book, 0, id);
    assert_eq!(made.kind, "barChart");
    assert_eq!(made.grouping, "clustered");
    assert!(!made.horizontal);
    assert_eq!(
        formulas(&made),
        [
            (
                Some("Operations!$B$1".to_string()),
                Some("Operations!$A$2:$A$4".to_string()),
                Some("Operations!$B$2:$B$4".to_string())
            ),
            (
                Some("Operations!$C$1".to_string()),
                Some("Operations!$A$2:$A$4".to_string()),
                Some("Operations!$C$2:$C$4".to_string())
            ),
        ]
    );
    // Two series: Excel's "Chart Title" and a legend at the bottom.
    assert_eq!(made.title, "Chart Title");
    assert_eq!(made.legend.as_deref(), Some("b"));
    // Beside the data, from E2.
    let shape = book.workbook().sheets[0]
        .shapes
        .iter()
        .find(|shape| shape.id == id)
        .unwrap();
    assert_eq!(shape.from, CellRef::new(4, 1));
    assert!(drawn(&book, id));
    // Saved with its caches, as Excel writes it, and read back the same.
    let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
    let read = chart(&reopened, 0, id);
    assert_eq!(formulas(&read), formulas(&made));
    assert_eq!(
        read.series[0].values.points,
        [Some("10".into()), Some("20".into()), Some("30".into())]
    );
    assert_eq!(
        read.series[0].categories.points,
        [
            Some("Alpha".into()),
            Some("Beta".into()),
            Some("Gamma".into())
        ]
    );
    assert_eq!(read.series[1].name.points, [Some("Double".into())]);
    // It follows the data.
    book.set_cells(SHEET, &[("B2".into(), CellInput::Number(15.0))])
        .unwrap();
    assert!(drawn(&book, id));
    // One undo step each.
    assert!(book.undo().unwrap());
    assert!(book.undo().unwrap());
    assert!(book.workbook().sheets[0].shapes.is_empty());
}

#[test]
fn one_series_is_its_own_title_and_needs_no_legend() {
    let mut book = open("grid-operations-workbook.xlsx");
    let id = book.insert_chart(SHEET, "A1:B4", ChartKind::Pie).unwrap();
    let made = chart(&book, 0, id);
    assert_eq!(made.kind, "pieChart");
    assert_eq!(made.title, "Amount");
    // A pie names its slices in a legend; a single column does not.
    assert_eq!(made.legend.as_deref(), Some("b"));
    let id = book.insert_chart(SHEET, "A1:B4", ChartKind::Line).unwrap();
    let made = chart(&book, 0, id);
    assert_eq!(made.kind, "lineChart");
    assert_eq!(made.title, "Amount");
    assert_eq!(made.legend, None);
    assert_eq!(book.workbook().sheets[0].shapes.len(), 2);
}

#[test]
fn a_wide_range_makes_a_series_of_each_row() {
    let mut book = open("grid-operations-workbook.xlsx");
    // A1:C2 is wider than tall: one series, Alpha, over Amount and Double.
    let id = book.insert_chart(SHEET, "A1:C2", ChartKind::Bar).unwrap();
    let made = chart(&book, 0, id);
    assert!(made.horizontal);
    assert_eq!(
        formulas(&made),
        [(
            Some("Operations!$A$2".to_string()),
            Some("Operations!$B$1:$C$1".to_string()),
            Some("Operations!$B$2:$C$2".to_string())
        )]
    );
}

#[test]
fn every_kind_is_made_as_excel_makes_it() {
    let cases = [
        (ChartKind::Column, "barChart", "clustered", false),
        (ChartKind::StackedColumn, "barChart", "stacked", false),
        (ChartKind::Bar, "barChart", "clustered", true),
        (ChartKind::StackedBar, "barChart", "stacked", true),
        (ChartKind::Line, "lineChart", "standard", false),
        (ChartKind::LineWithMarkers, "lineChart", "standard", false),
        (ChartKind::Area, "areaChart", "standard", false),
        (ChartKind::Pie, "pieChart", "", false),
        (ChartKind::Doughnut, "doughnutChart", "", false),
        (ChartKind::Scatter, "scatterChart", "", false),
    ];
    for (kind, element, grouping, horizontal) in cases {
        let mut book = open("grid-operations-workbook.xlsx");
        let id = book.insert_chart(SHEET, "A1:C4", kind).unwrap();
        let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
        let read = chart(&reopened, 0, id);
        assert_eq!(read.kind, element, "{kind:?}");
        if !grouping.is_empty() {
            assert_eq!(read.grouping, grouping, "{kind:?}");
        }
        assert_eq!(read.horizontal, horizontal, "{kind:?}");
        assert!(
            read.unsupported_properties.is_empty(),
            "{kind:?}: {:?}",
            read.unsupported_properties
        );
        assert!(drawn(&reopened, id), "{kind:?}");
    }
}

#[test]
fn a_chart_into_a_sheet_that_has_drawings_joins_them() {
    let mut book = open("charts-workbook.xlsx");
    let before = book.workbook().sheets[0].shapes.len();
    let id = book
        .insert_chart("Charts", "A1:C5", ChartKind::Line)
        .unwrap();
    assert_eq!(id, 11, "the next id in the drawing");
    assert_eq!(book.workbook().sheets[0].shapes.len(), before + 1);
    assert_eq!(chart(&book, 0, 1).title, "Quarter & Plan");
    let shape = book.workbook().sheets[0]
        .shapes
        .iter()
        .find(|shape| shape.id == id)
        .unwrap();
    assert_eq!(shape.name, "Chart 11");
}

#[test]
fn type_title_legend_and_data_change_as_chart_design_changes_them() {
    let mut book = open("grid-operations-workbook.xlsx");
    let id = book
        .insert_chart(SHEET, "A1:C4", ChartKind::Column)
        .unwrap();
    let series = formulas(&chart(&book, 0, id));
    book.set_chart(
        SHEET,
        id,
        &ChartChange {
            kind: Some(ChartKind::Line),
            ..ChartChange::default()
        },
    )
    .unwrap();
    let changed = chart(&book, 0, id);
    assert_eq!(changed.kind, "lineChart");
    assert_eq!(formulas(&changed), series);
    assert_eq!(changed.title, "Chart Title");
    book.set_chart(
        SHEET,
        id,
        &ChartChange {
            title: Some(Some("Sales & Plan".into())),
            legend: Some(Some(LegendPosition::Right)),
            ..ChartChange::default()
        },
    )
    .unwrap();
    let changed = chart(&book, 0, id);
    assert_eq!(changed.title, "Sales & Plan");
    assert_eq!(changed.legend.as_deref(), Some("r"));
    assert_eq!(changed.kind, "lineChart");
    book.set_chart(
        SHEET,
        id,
        &ChartChange {
            title: Some(None),
            legend: Some(None),
            data: Some("A1:B4".into()),
            ..ChartChange::default()
        },
    )
    .unwrap();
    let changed = chart(&book, 0, id);
    assert_eq!(changed.title, "");
    assert_eq!(changed.legend, None);
    assert_eq!(changed.series.len(), 1);
    let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
    assert_eq!(chart(&reopened, 0, id).series.len(), 1);
    assert!(book.undo().unwrap());
    assert_eq!(chart(&book, 0, id).series.len(), 2);
}

#[test]
fn an_excel_chart_keeps_its_series_through_a_change() {
    let mut book = open("charts-workbook.xlsx");
    let before = chart(&book, 0, 1);
    book.set_chart(
        "Charts",
        1,
        &ChartChange {
            title: Some(Some("Renamed".into())),
            ..ChartChange::default()
        },
    )
    .unwrap();
    let after = chart(&book, 0, 1);
    assert_eq!(after.title, "Renamed");
    assert_eq!(formulas(&after), formulas(&before));
    assert_eq!(
        after
            .series
            .iter()
            .map(|series| &series.color)
            .collect::<Vec<_>>(),
        before
            .series
            .iter()
            .map(|series| &series.color)
            .collect::<Vec<_>>()
    );
    assert_eq!(after.palette, before.palette);
    book.set_chart(
        "Charts",
        1,
        &ChartChange {
            kind: Some(ChartKind::Line),
            ..ChartChange::default()
        },
    )
    .unwrap();
    let after = chart(&book, 0, 1);
    assert_eq!(after.kind, "lineChart");
    assert_eq!(formulas(&after), formulas(&before));
    assert_eq!(after.title, "Renamed");
}

#[test]
fn what_is_not_a_chart_is_refused() {
    let mut book = open("grid-operations-workbook.xlsx");
    assert!(matches!(
        book.insert_chart(SHEET, "A1", ChartKind::Column),
        Err(HeadlessError::BadReference(_))
    ));
    assert!(matches!(
        book.set_chart(SHEET, 7, &ChartChange::default()),
        Err(HeadlessError::NoSuchShape(_))
    ));
}
