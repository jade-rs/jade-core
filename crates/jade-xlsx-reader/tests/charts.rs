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
    // Columns with a line: the line is the chart's second plot.
    let combo = shapes[7].chart.as_ref().unwrap();
    assert!(!combo
        .unsupported_properties
        .contains(&"combined or missing plot".into()));
    assert_eq!(combo.kind, "barChart");
    assert_eq!(combo.extra_plots.len(), 1);
    assert_eq!(combo.extra_plots[0].kind, "lineChart");
}

#[test]
fn reads_labels_titles_trendlines_error_bars_secondary_log_and_date_axes() {
    let chart = parse_chart(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea>
<c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/>
<c:dLbls><c:numFmt formatCode="0.0" sourceLinked="0"/><c:showVal val="1"/><c:dLblPos val="inEnd"/><c:dLbl><c:idx val="2"/><c:delete val="1"/></c:dLbl></c:dLbls>
<c:trendline><c:trendlineType val="poly"/><c:order val="3"/><c:dispEq val="1"/></c:trendline>
<c:errBars><c:errDir val="y"/><c:errBarType val="plus"/><c:errValType val="percentage"/><c:val val="5"/></c:errBars>
<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>
<c:axId val="1"/><c:axId val="2"/></c:barChart>
<c:lineChart><c:ser><c:idx val="1"/><c:order val="1"/><c:smooth val="1"/><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="3"/><c:axId val="4"/></c:lineChart>
<c:dateAx><c:axId val="1"/><c:delete val="0"/><c:axPos val="b"/><c:title><c:tx><c:rich><a:p><a:r><a:t>When</a:t></a:r></a:p></c:rich></c:tx></c:title><c:baseTimeUnit val="months"/><c:crossAx val="2"/></c:dateAx>
<c:valAx><c:axId val="2"/><c:scaling><c:logBase val="10"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:title/><c:crossAx val="1"/></c:valAx>
<c:catAx><c:axId val="3"/><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>
<c:valAx><c:axId val="4"/><c:delete val="0"/><c:axPos val="r"/><c:crossAx val="3"/><c:crosses val="max"/></c:valAx>
</c:plotArea></c:chart></c:chartSpace>"#,
    )
    .unwrap();
    assert!(
        chart.unsupported_properties.is_empty(),
        "{:?}",
        chart.unsupported_properties
    );
    let labels = chart.series[0].data_labels.as_ref().unwrap();
    assert!(labels.show_value && !labels.show_category);
    assert_eq!(labels.position.as_deref(), Some("inEnd"));
    assert_eq!(labels.number_format.as_deref(), Some("0.0"));
    assert_eq!(labels.deleted, [2]);
    let trend = &chart.series[0].trendlines[0];
    assert_eq!((trend.kind.as_str(), trend.order), ("poly", 3));
    assert!(trend.display_equation && !trend.display_r_squared);
    let bars = &chart.series[0].error_bars[0];
    assert_eq!(
        (
            bars.bar_type.as_str(),
            bars.value_type.as_str(),
            bars.value.as_deref()
        ),
        ("plus", "percentage", Some("5"))
    );
    assert!(chart.category_axis.date);
    assert_eq!(chart.category_axis.title.as_deref(), Some("When"));
    assert_eq!(
        chart.category_axis.base_time_unit.as_deref(),
        Some("months")
    );
    assert_eq!(chart.value_axis.log_base.as_deref(), Some("10"));
    assert_eq!(chart.value_axis.title.as_deref(), Some("Axis Title"));
    assert!(chart.extra_plots[0].secondary);
    assert!(chart.extra_plots[0].series[0].smooth);
    let secondary = chart.secondary_value_axis.as_ref().unwrap();
    assert_eq!(secondary.position.as_deref(), Some("r"));
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
