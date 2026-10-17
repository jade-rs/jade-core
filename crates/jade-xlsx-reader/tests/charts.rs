use jade_xlsx_reader::{
    chart::{parse_chart, MAX_CHART_POINTS},
    read_xlsx, ShapeKind,
};
#[test]
fn reads_chart_relationships_sparse_indexed_caches_formulas_axes_and_theme() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/charts-workbook.xlsx"
    ))
    .unwrap();
    let shapes = &book.sheets[0].shapes;
    assert_eq!(shapes.len(), 10);
    assert!(shapes.iter().all(|s| s.kind == ShapeKind::Chart));
    let chart = shapes[0].chart.as_ref().unwrap();
    assert_eq!(chart.title, "Quarter & Plan");
    assert_eq!(chart.kind, "barChart");
    assert_eq!(chart.part_name.as_deref(), Some("xl/charts/chart1.xml"));
    assert_eq!(
        chart.series[0].values.formula.as_deref(),
        Some("Charts!$B$2:$B$5")
    );
    assert_eq!(chart.series[0].values.points, vec![Some("99".into()); 4]);
    assert_eq!(chart.palette[0], [0, 170, 85]);
    assert_eq!(chart.series[1].color.rgb, Some([102, 51, 255]));
    assert_eq!(chart.value_axis.major_unit.as_deref(), Some("10"));
    assert!(chart.unsupported_properties.is_empty());
    let line = shapes[2].chart.as_ref().unwrap();
    assert_eq!(
        line.series[0].values.points,
        [
            Some("10".into()),
            None,
            Some("30".into()),
            Some("40".into())
        ]
    );
    assert!(shapes[7]
        .chart
        .as_ref()
        .unwrap()
        .unsupported_properties
        .contains(&"combined or missing plot".into()));
}
#[test]
fn chart_namespaces_and_resource_limits_do_not_accept_foreign_or_unbounded_points() {
    let xml = format!(
        r#"<c:chartSpace xmlns:c="http://purl.oclc.org/ooxml/drawingml/chart" xmlns:x="urn:foreign"><c:chart><c:plotArea><x:barChart/><c:lineChart><c:ser><c:val><c:numLit><c:ptCount val="3"/><c:pt idx="1"><c:v><![CDATA[2.5]]></c:v></c:pt><c:pt idx="{}"><c:v>8</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#,
        MAX_CHART_POINTS * 100
    );
    let chart = parse_chart(&xml).unwrap();
    assert_eq!(chart.kind, "lineChart");
    assert_eq!(
        chart.series[0].values.points,
        [None, Some("2.5".into()), None]
    );
    assert!(chart
        .unsupported_properties
        .contains(&"chart point limit".into()));
    assert!(!chart
        .unsupported_properties
        .contains(&"combined or missing plot".into()));
}
