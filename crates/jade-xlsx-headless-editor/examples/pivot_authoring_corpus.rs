//! Original authoring corpus for independent OOXML component-schema checks.
//! Usage: cargo run --example pivot_authoring_corpus -- <output-directory>
mod pivot_corpus;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    HeadlessWorkbook, PivotFilterSelection, PivotLayout, PivotMeasure, PivotPage,
    PivotReportLayout, PivotSource, PivotStyle, PivotUpdate,
};
use jade_xlsx_reader::{CellRange, CellRef};
const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/pivot-chart-compat.xlsx");
fn layout() -> PivotLayout {
    PivotLayout {
        rows: vec![0],
        columns: vec![-2],
        pages: vec![PivotPage {
            field: 2,
            selection: PivotFilterSelection::All,
        }],
        report_layout: PivotReportLayout::Tabular,
        origin: None,
        row_grand_totals: true,
        column_grand_totals: true,
    }
}
fn measure(existing: Option<u32>, aggregation: &str, name: &str) -> PivotMeasure {
    PivotMeasure {
        existing,
        field: 1,
        aggregation: aggregation.into(),
        caption: name.into(),
        number_format_id: Some(164),
    }
}

fn assert_values(bytes: &[u8], case: &str) -> Result<(), Box<dyn std::error::Error>> {
    let book = jade_xlsx_reader::read_xlsx(bytes)?;
    let charts = book
        .sheets
        .iter()
        .flat_map(|s| &s.shapes)
        .filter_map(|s| s.chart.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(charts.len(), 1, "{case}: actual chart binding");
    let expected: Vec<Vec<&str>> = match case {
        "average" => vec![vec!["7.5", "13.5"]],
        "reordered-measures" => vec![vec!["2", "2"], vec!["15", "27"]],
        "values-on-rows" => vec![vec!["15", "27"], vec!["2", "2"]],
        "columns-only" => vec![vec!["15"], vec!["27"]],
        "two-filters" => vec![vec!["20", "7"]],
        "extended-source" => vec![vec!["15", "100", "27"]],
        _ => vec![vec!["15", "27"]],
    };
    assert_eq!(
        charts[0]
            .series
            .iter()
            .map(|s| s.values.points.clone())
            .collect::<Vec<_>>(),
        expected
            .into_iter()
            .map(|p| p
                .into_iter()
                .map(|s| Some(s.to_string()))
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "{case}: real saved values"
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("provide an output directory")?,
    );
    std::fs::create_dir_all(&dir)?;
    let mut rows = layout();
    rows.rows.push(-2);
    rows.columns.clear();
    let mut columns = layout();
    columns.rows.clear();
    columns.columns = vec![0, -2];
    columns.pages.clear();
    let mut moved = layout();
    moved.origin = Some(CellRef::new(8, 9));
    let mut pages = layout();
    pages.origin = Some(CellRef::new(4, 4));
    pages.rows = vec![1];
    pages.pages = vec![
        PivotPage {
            field: 0,
            selection: PivotFilterSelection::Item(2),
        },
        PivotPage {
            field: 2,
            selection: PivotFilterSelection::All,
        },
    ];
    let cases = [
        (
            "average",
            PivotUpdate {
                measures: Some(vec![measure(Some(0), "average", "Average_x0041_")]),
                ..Default::default()
            },
        ),
        (
            "reordered-measures",
            PivotUpdate {
                measures: Some(vec![
                    measure(None, "count", "Count"),
                    measure(Some(0), "sum", "Sum"),
                ]),
                ..Default::default()
            },
        ),
        (
            "values-on-rows",
            PivotUpdate {
                layout: Some(rows),
                measures: Some(vec![
                    measure(Some(0), "sum", "Sum"),
                    measure(None, "count", "Count"),
                ]),
                ..Default::default()
            },
        ),
        (
            "columns-only",
            PivotUpdate {
                layout: Some(columns),
                ..Default::default()
            },
        ),
        (
            "moved",
            PivotUpdate {
                layout: Some(moved),
                ..Default::default()
            },
        ),
        (
            "two-filters",
            PivotUpdate {
                layout: Some(pages),
                ..Default::default()
            },
        ),
        (
            "named-style",
            PivotUpdate {
                style: Some(PivotStyle {
                    name: Some("PivotStyleDark1".into()),
                    row_headers: true,
                    column_headers: true,
                    row_stripes: true,
                    column_stripes: false,
                    last_column: true,
                }),
                ..Default::default()
            },
        ),
        (
            "extended-source",
            PivotUpdate {
                source: Some(PivotSource::Range {
                    sheet: "Data".into(),
                    range: CellRange::parse("A1:C6")?,
                }),
                ..Default::default()
            },
        ),
        ("repaired-cache-link", PivotUpdate::default()),
    ];
    for (prefix, original) in [
        ("", ORIGINAL.to_vec()),
        ("strict-", pivot_corpus::strict_original(ORIGINAL)?),
    ] {
        assert_values(&original, "original")?;
        std::fs::write(dir.join(format!("{prefix}original.xlsx")), &original)?;
        for (name, update) in &cases {
            let mut input = original.clone();
            if *name == "repaired-cache-link" {
                let mut p = jade_xlsx_reader::OpcPackage::open(&input)?;
                p.remove("xl/pivotTables/_rels/report.xml.rels");
                input = p.write()?;
            }
            let mut book = HeadlessWorkbook::open_for_editing(&input)?;
            if *name == "extended-source" {
                book.set("Data", "A6", CellInput::Text("North".into()))?;
                book.set("Data", "B6", CellInput::Number(100.))?;
            }
            book.update_pivot("Report", "RegionalSales", update)?;
            assert_values(book.bytes(), name)?;
            let path = dir.join(format!("{prefix}{name}.xlsx"));
            std::fs::write(&path, book.bytes())?;
            println!("{}", path.display());
        }
    }
    Ok(())
}
