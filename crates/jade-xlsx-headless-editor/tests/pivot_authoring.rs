use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    HeadlessWorkbook, PivotFieldOptions, PivotFilterSelection, PivotLayout, PivotMeasure,
    PivotMemberFilter, PivotPage, PivotReportLayout, PivotSource, PivotUpdate,
};
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-chart-compat.xlsx");
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(FIXTURE).unwrap()
}
fn measure(existing: Option<u32>, aggregation: &str, caption: &str) -> PivotMeasure {
    PivotMeasure {
        existing,
        field: 1,
        aggregation: aggregation.into(),
        caption: caption.into(),
        number_format_id: Some(164),
    }
}
fn layout() -> PivotLayout {
    PivotLayout {
        rows: vec![0],
        columns: vec![-2],
        pages: vec![],
        report_layout: PivotReportLayout::Tabular,
        origin: None,
        row_grand_totals: true,
        column_grand_totals: true,
    }
}
fn values(bytes: &[u8]) -> Vec<Vec<Option<String>>> {
    let book = read_xlsx(bytes).unwrap();
    book.sheets
        .iter()
        .flat_map(|s| &s.shapes)
        .find_map(|s| s.chart.as_ref())
        .unwrap()
        .series
        .iter()
        .map(|s| s.values.points.clone())
        .collect()
}
#[test]
fn aggregate_caption_and_chart_update_are_one_reopenable_history_edit() {
    let mut b = open();
    let before = b.bytes().to_vec();
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            measures: Some(vec![measure(Some(0), "average", "Average_x0041_")]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("7.5".into()), Some("13.5".into())]]
    );
    let book = read_xlsx(b.bytes()).unwrap();
    let c = book.sheets[1].shapes[0].chart.as_ref().unwrap();
    assert_eq!(c.series[0].color.rgb, Some([51, 102, 153]));
    assert_eq!(
        c.series[0].name.points[0].as_deref(),
        Some("Average_x0041_")
    );
    let after = b.bytes().to_vec();
    assert_eq!(
        read_xlsx(&after).unwrap().pivots.tables[0].data_fields[0]
            .name
            .as_deref(),
        Some("Average_x0041_")
    );
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn measure_insertion_reorder_and_removal_keep_the_original_series_style() {
    let mut b = open();
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            measures: Some(vec![
                measure(None, "count", "Count"),
                measure(Some(0), "sum", "Sum"),
            ]),
            ..Default::default()
        },
    )
    .unwrap();
    let book = read_xlsx(b.bytes()).unwrap();
    let c = book.sheets[1].shapes[0].chart.as_ref().unwrap();
    assert_eq!(c.series.len(), 2);
    let sum = c
        .series
        .iter()
        .find(|s| s.name.points[0].as_deref() == Some("Sum"))
        .unwrap();
    assert_eq!(sum.color.rgb, Some([51, 102, 153]));
    assert_eq!(sum.values.points, [Some("15".into()), Some("27".into())]);
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            measures: Some(vec![measure(Some(1), "average", "Average")]),
            ..Default::default()
        },
    )
    .unwrap();
    let book = read_xlsx(b.bytes()).unwrap();
    let c = book.sheets[1].shapes[0].chart.as_ref().unwrap();
    assert_eq!(c.series.len(), 1);
    assert_eq!(c.series[0].color.rgb, Some([51, 102, 153]));
    assert_eq!(
        c.series[0].values.points,
        [Some("7.5".into()), Some("13.5".into())]
    );
}
#[test]
fn moving_the_report_clears_only_its_old_body_and_filter_cells() {
    let mut b = open();
    b.set("Report", "H1", CellInput::Text("Keep".into()))
        .unwrap();
    let mut l = layout();
    l.origin = Some(CellRef::new(8, 9));
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            layout: Some(l),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        b.cell("Report", "H1").unwrap().text.as_deref(),
        Some("Keep")
    );
    for at in ["E1", "F1", "E3", "F4", "F6"] {
        assert!(b.cell("Report", at).unwrap().display.is_empty(), "{at}");
    }
    assert_eq!(b.cell("Report", "J11").unwrap().number, Some(15.));
    assert_eq!(b.cell("Report", "J12").unwrap().number, Some(27.));
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("15".into()), Some("27".into())]]
    );
}
#[test]
fn page_filter_moves_a_dimension_and_updates_visible_values_and_charts() {
    let mut b = open();
    b.set("Data", "C3", CellInput::Number(46319.)).unwrap();
    b.set("Data", "C5", CellInput::Number(46320.)).unwrap();
    let mut l = layout();
    l.rows = vec![2];
    l.pages = vec![PivotPage {
        field: 0,
        selection: PivotFilterSelection::Item(2),
    }];
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            layout: Some(l),
            field_options: vec![PivotFieldOptions {
                field: 2,
                caption: None,
                sort: Some("ascending".into()),
                subtotals: None,
                subtotals_at_top: None,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        b.cell("Report", "F1").unwrap().text.as_deref(),
        Some("West")
    );
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("20".into()), Some("7".into())]]
    );
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(27.));
}
#[test]
fn extending_a_source_preserves_selected_member_filters_on_new_items() {
    let mut b = open();
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            member_filters: vec![PivotMemberFilter {
                field: 0,
                visible: Some([2].into_iter().collect()),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(values(b.bytes()), vec![vec![Some("27".into())]]);
    b.set("Data", "A6", CellInput::Text("North".into()))
        .unwrap();
    b.set("Data", "B6", CellInput::Number(100.)).unwrap();
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            source: Some(PivotSource::Range {
                sheet: "Data".into(),
                range: CellRange::parse("A1:C6").unwrap(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(values(b.bytes()), vec![vec![Some("27".into())]]);
    let bytes = b.bytes().to_vec();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            member_filters: vec![PivotMemberFilter {
                field: 0,
                visible: None,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        values(b.bytes()),
        vec![vec![
            Some("15".into()),
            Some("100".into()),
            Some("27".into())
        ]]
    );
}
#[test]
fn invalid_edits_and_collisions_preserve_bytes_and_the_redo_branch() {
    let mut b = open();
    b.set("Data", "B2", CellInput::Number(99.)).unwrap();
    b.undo().unwrap();
    let before = b.bytes().to_vec();
    let mut l = layout();
    l.origin = Some(CellRef::new(0, 0)); // The chart is on Report; place a real collision there.
    b.set("Report", "A1", CellInput::Text("Keep".into()))
        .unwrap();
    b.undo().unwrap();
    for update in [
        PivotUpdate {
            measures: Some(vec![measure(Some(99), "sum", "Bad")]),
            ..Default::default()
        },
        PivotUpdate {
            source: Some(PivotSource::Range {
                sheet: "Data".into(),
                range: CellRange::parse("A1:B5").unwrap(),
            }),
            ..Default::default()
        },
    ] {
        assert!(b.update_pivot("Report", "RegionalSales", &update).is_err());
        assert_eq!(b.bytes(), before);
    }
    assert!(b.redo().unwrap());
    assert_eq!(
        b.cell("Report", "A1").unwrap().text.as_deref(),
        Some("Keep")
    );
    let before = b.bytes().to_vec();
    assert!(b
        .update_pivot(
            "Report",
            "RegionalSales",
            &PivotUpdate {
                layout: Some(l),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn removing_an_axis_omits_its_nonempty_schema_container() {
    let mut b = open();
    let mut l = layout();
    l.rows.clear();
    l.columns = vec![0, -2];
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            layout: Some(l),
            ..Default::default()
        },
    )
    .unwrap();
    let package = jade_xlsx_reader::OpcPackage::open(b.bytes()).unwrap();
    let xml = package
        .required_part_str("xl/pivotTables/report.xml")
        .unwrap();
    assert!(!xml.contains("<rowFields"));
    assert!(!xml.contains("<pageFields"));
    assert!(!xml.contains("count=\"0\""));
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("15".into())], vec![Some("27".into())]]
    );
    let bytes = b.bytes().to_vec();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("15".into())], vec![Some("27".into())]]
    );
}

#[test]
fn two_report_filters_write_their_actual_geometry_and_clear_the_previous_filter() {
    let mut b = open();
    let mut l = layout();
    l.rows = vec![1];
    l.origin = Some(CellRef::new(4, 4));
    l.pages = vec![
        PivotPage {
            field: 0,
            selection: PivotFilterSelection::Item(2),
        },
        PivotPage {
            field: 2,
            selection: PivotFilterSelection::All,
        },
    ];
    b.update_pivot(
        "Report",
        "RegionalSales",
        &PivotUpdate {
            layout: Some(l),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        b.cell("Report", "E2").unwrap().text.as_deref(),
        Some("Territory")
    );
    assert_eq!(
        b.cell("Report", "F2").unwrap().text.as_deref(),
        Some("West")
    );
    assert_eq!(
        b.cell("Report", "E3").unwrap().text.as_deref(),
        Some("When")
    );
    assert_eq!(
        b.cell("Report", "F3").unwrap().text.as_deref(),
        Some("(All)")
    );
    assert!(b.cell("Report", "E1").unwrap().display.is_empty());
    assert_eq!(
        values(b.bytes()),
        vec![vec![Some("20".into()), Some("7".into())]]
    );
    let parsed = read_xlsx(b.bytes()).unwrap();
    let t = &parsed.pivots.tables[0];
    assert_eq!(t.location.row_page_count, Some(2));
    assert_eq!(t.location.column_page_count, Some(1));
}

#[test]
fn page_filter_growth_cannot_overwrite_an_unrelated_cell_above_the_report() {
    let mut b = open();
    b.set("Report", "E2", CellInput::Text("Keep".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    let mut l = layout();
    l.origin = Some(CellRef::new(4, 4));
    l.rows = vec![1];
    l.pages = vec![
        PivotPage {
            field: 0,
            selection: PivotFilterSelection::All,
        },
        PivotPage {
            field: 2,
            selection: PivotFilterSelection::All,
        },
    ];
    assert!(b
        .update_pivot(
            "Report",
            "RegionalSales",
            &PivotUpdate {
                layout: Some(l),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn malformed_named_source_quotes_return_an_error_without_losing_redo() {
    let mut b = open();
    b.set("Data", "B2", CellInput::Number(99.)).unwrap();
    b.undo().unwrap();
    let before = b.bytes().to_vec();
    for name in ["'!A1:C5", "'O'B'!A1:C5"] {
        assert!(b
            .update_pivot(
                "Report",
                "RegionalSales",
                &PivotUpdate {
                    source: Some(PivotSource::Name {
                        sheet: None,
                        name: name.into()
                    }),
                    ..Default::default()
                }
            )
            .is_err());
        assert_eq!(b.bytes(), before);
    }
    assert!(b.redo().unwrap());
    assert_eq!(b.cell("Data", "B2").unwrap().number, Some(99.));
}
#[test]
fn a_conflicting_cache_link_refuses_authoring_without_changing_the_saved_workbook() {
    let mut package = jade_xlsx_reader::OpcPackage::open(FIXTURE).unwrap();
    let xml = package
        .required_part_str("xl/pivotTables/_rels/report.xml.rels")
        .unwrap();
    package.replace(
        "xl/pivotTables/_rels/report.xml.rels",
        xml.replace("../pivotCache/cache.xml", "../pivotCache/other.xml")
            .into_bytes(),
    );
    let bytes = package.write().unwrap();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert!(b
        .update_pivot(
            "Report",
            "RegionalSales",
            &PivotUpdate {
                measures: Some(vec![measure(Some(0), "average", "Average")]),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(b.bytes(), bytes);
    assert!(!b.undo().unwrap());
}
