//! Original lifecycle outputs for independent schema and actual SDK checks.
mod pivot_corpus;
use jade_xlsx_headless_editor::{
    HeadlessWorkbook, PivotCreate, PivotDestination, PivotMemberFilter, PivotSource, PivotUpdate,
};
use jade_xlsx_reader::{CellRange, CellRef, CellValue};
const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/pivot-chart-compat.xlsx");
fn options() -> PivotCreate {
    PivotCreate {
        name: Some("Created_x0041_".into()),
        source: PivotSource::Range {
            sheet: "Data".into(),
            range: CellRange::parse("A1:C5").unwrap(),
        },
        destination: PivotDestination::Existing {
            sheet: "Report".into(),
            origin: CellRef::parse("K24").unwrap(),
        },
        settings: PivotUpdate::default(),
    }
}
fn validate(bytes: &[u8], tables: usize, caches: usize, static_chart: bool) {
    let b = jade_xlsx_reader::read_xlsx(bytes).unwrap();
    assert!(b.pivots.issues.is_empty());
    assert_eq!(b.pivots.tables.len(), tables);
    assert_eq!(b.pivots.caches.len(), caches);
    let chart = b
        .sheets
        .iter()
        .flat_map(|s| &s.shapes)
        .find_map(|s| s.chart.as_ref())
        .unwrap();
    assert_eq!(chart.pivot_source.is_none(), static_chart);
    assert_eq!(
        chart.series[0].values.points,
        [Some("15".into()), Some("27".into())]
    );
    if static_chart {
        assert!(chart.series[0].values.formula.is_none());
        assert!(chart.series[0].categories.formula.is_none());
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("provide an output directory")?,
    );
    std::fs::create_dir_all(&dir)?;
    for (prefix, original) in [
        ("", ORIGINAL.to_vec()),
        ("strict-", pivot_corpus::strict_original(ORIGINAL)?),
    ] {
        let mut book = HeadlessWorkbook::open_for_editing(&original)?;
        book.create_pivot(&options())?;
        validate(book.bytes(), 2, 2, false);
        std::fs::write(
            dir.join(format!("{prefix}created-existing.xlsx")),
            book.bytes(),
        )?;
        book.delete_pivot("Report", "Created_x0041_")?;
        validate(book.bytes(), 1, 1, false);
        std::fs::write(
            dir.join(format!("{prefix}removed-created-cache.xlsx")),
            book.bytes(),
        )?;
        let mut book = HeadlessWorkbook::open_for_editing(&original)?;
        let mut o = options();
        o.destination = PivotDestination::NewSheet {
            name: "Summary_x0041_".into(),
            after: Some("Data".into()),
            origin: CellRef::parse("A3")?,
        };
        book.create_pivot(&o)?;
        validate(book.bytes(), 2, 2, false);
        let b = jade_xlsx_reader::read_xlsx(book.bytes())?;
        let sheet = b
            .sheets
            .iter()
            .find(|s| s.name == "Summary_x0041_")
            .unwrap();
        assert_eq!(
            sheet
                .rows
                .iter()
                .flat_map(|r| &r.cells)
                .find(|c| c.reference == CellRef::parse("B6").unwrap())
                .unwrap()
                .value,
            CellValue::Number(42.)
        );
        std::fs::write(
            dir.join(format!("{prefix}created-new-sheet.xlsx")),
            book.bytes(),
        )?;
        let mut filtered = HeadlessWorkbook::open_for_editing(&original)?;
        let mut o = options();
        o.settings.member_filters = vec![PivotMemberFilter {
            field: 0,
            visible: Some([0].into_iter().collect()),
        }];
        filtered.create_pivot(&o)?;
        validate(filtered.bytes(), 2, 2, false);
        let parsed = jade_xlsx_reader::read_xlsx(filtered.bytes())?;
        assert_eq!(
            parsed.sheets[1]
                .rows
                .iter()
                .flat_map(|r| &r.cells)
                .find(|c| c.reference == CellRef::parse("L26").unwrap())
                .unwrap()
                .value,
            CellValue::Number(15.)
        );
        std::fs::write(
            dir.join(format!("{prefix}created-filtered.xlsx")),
            filtered.bytes(),
        )?;
        let mut book = HeadlessWorkbook::open_for_editing(&original)?;
        book.delete_pivot("Report", "RegionalSales")?;
        validate(book.bytes(), 0, 0, true);
        std::fs::write(
            dir.join(format!("{prefix}removed-last-static-chart.xlsx")),
            book.bytes(),
        )?;
        let mut stale = jade_xlsx_reader::OpcPackage::open(&original)?;
        let mut chart = stale
            .required_part_str("xl/charts/pivot.xml")?
            .replace("<c:v>15</c:v>", "<c:v>999</c:v>");
        let start = chart.find("<c:title>").unwrap();
        let end = chart.find("</c:title>").unwrap() + "</c:title>".len();
        chart.replace_range(start..end,"<c:title><c:tx><c:strRef><c:f>'Report'!$F$3</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Title_x0001_</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>");
        stale.replace("xl/charts/pivot.xml", chart.into_bytes());
        let mut stale = HeadlessWorkbook::open_for_editing(&stale.write()?)?;
        stale.delete_pivot("Report", "RegionalSales")?;
        validate(stale.bytes(), 0, 0, true);
        let parsed = jade_xlsx_reader::read_xlsx(stale.bytes())?;
        assert_eq!(
            parsed.sheets[1].shapes[0].chart.as_ref().unwrap().title,
            "Total Amount"
        );
        std::fs::write(
            dir.join(format!("{prefix}removed-stale-cache.xlsx")),
            stale.bytes(),
        )?;
        let mut package = jade_xlsx_reader::OpcPackage::open(&original)?;
        let chart = package.required_part_str("xl/charts/pivot.xml")?;
        let start = chart.find("<c:title>").unwrap();
        let end = chart.find("</c:title>").unwrap() + "</c:title>".len();
        let mut chart = chart;
        chart.replace_range(start..end,"<c:title><c:tx><c:strRef><c:f>'Report'!$F$3</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Total Amount</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>");
        package.replace("xl/charts/pivot.xml", chart.into_bytes());
        let mut book = HeadlessWorkbook::open_for_editing(&package.write()?)?;
        book.delete_pivot("Report", "RegionalSales")?;
        validate(book.bytes(), 0, 0, true);
        std::fs::write(
            dir.join(format!("{prefix}removed-with-linked-title.xlsx")),
            book.bytes(),
        )?;
    }
    Ok(())
}
