use jade_xlsx_reader::chart::parse_chart;
use jade_xlsx_writer::chart_format::{format_chart_part, AxisFormat, AxisId, ChartFormat};

const CHART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:dLbls><c:numFmt formatCode="0&quot;_x0041_&quot;" sourceLinked="0"/><c:showVal val="1"/></c:dLbls></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:numFmt formatCode="0&quot;_x0041_&quot;" sourceLinked="0"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;

#[test]
fn imported_axes_and_labels_decode_format_escapes() {
    let chart = parse_chart(CHART).unwrap();
    assert_eq!(chart.value_axis.number_format.as_deref(), Some("0\"A\""));
    assert_eq!(
        chart.series[0]
            .data_labels
            .as_ref()
            .unwrap()
            .number_format
            .as_deref(),
        Some("0\"A\"")
    );
}

#[test]
fn source_linked_labels_accept_both_xml_boolean_spellings() {
    for linked in ["1", "true", " true ", "0", "false"] {
        let xml = CHART.replace("sourceLinked=\"0\"", &format!("sourceLinked=\"{linked}\""));
        let chart = parse_chart(&xml).unwrap();
        let format = chart.series[0]
            .data_labels
            .as_ref()
            .unwrap()
            .number_format
            .as_deref();
        assert_eq!(
            format,
            if matches!(linked.trim(), "1" | "true") {
                None
            } else {
                Some("0\"A\"")
            }
        );
    }
}

#[test]
fn chart_display_flags_collapse_xml_boolean_whitespace() {
    for (value, expected) in [
        (" true ", true),
        (" 1 ", true),
        (" false ", false),
        (" 0 ", false),
    ] {
        let xml = CHART
            .replace(
                "<c:showVal val=\"1\"/>",
                &format!("<c:showVal val=\"{value}\"/>"),
            )
            .replace(
                "<c:barDir val=\"col\"/>",
                &format!("<c:barDir val=\"col\"/><c:varyColors val=\"{value}\"/>"),
            );
        let chart = parse_chart(&xml).unwrap();
        assert_eq!(chart.vary_colors, expected);
        assert_eq!(
            chart.series[0].data_labels.as_ref().unwrap().show_value,
            expected
        );
    }
}

#[test]
fn editing_an_axis_keeps_literal_escapes_and_attribute_whitespace() {
    for code in [
        "0\"_x0041_\"",
        "0\"line\nnext\"",
        "0\"tab\there\"",
        "0\"carriage\rreturn\"",
    ] {
        let edited = format_chart_part(
            CHART,
            &[ChartFormat::Axis(
                AxisId::Value,
                AxisFormat {
                    number_format: Some(Some(code.into())),
                    ..Default::default()
                },
            )],
        )
        .unwrap();
        let chart = parse_chart(&edited).unwrap();
        assert_eq!(chart.value_axis.number_format.as_deref(), Some(code));
        assert_eq!(
            chart.series[0]
                .data_labels
                .as_ref()
                .unwrap()
                .number_format
                .as_deref(),
            Some("0\"A\"")
        );
    }
}
