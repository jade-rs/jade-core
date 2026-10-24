use jade_xlsx_reader::chart::{parse_chart, PivotSource};
fn chart(inner: &str) -> String {
    format!(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:f="urn:foreign">{inner}<c:chart><c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#
    )
}
#[test]
fn the_standard_chart_space_binding_is_retained_with_literal_name_identity() {
    let b = jade_xlsx_reader::read_xlsx(include_bytes!(
        "../../../public/fixtures/pivot-chart-compat.xlsx"
    ))
    .unwrap();
    let c = b.sheets[1].shapes[0].chart.as_ref().unwrap();
    let p = c.pivot_source.as_ref().unwrap();
    assert_eq!(p.format_id, 0);
    let q = p.qualified_name().unwrap();
    assert_eq!(q.workbook.as_deref(), Some("pivot-chart-compat.xlsx"));
    assert_eq!(q.sheet, "Report");
    assert_eq!(q.pivot, "RegionalSales");
    let c=parse_chart(&chart("<c:pivotSource><c:name>[Book.xlsx]Sheet!_x0041_</c:name><c:fmtId val=\"0\"/></c:pivotSource>")).unwrap();
    assert_eq!(
        c.pivot_source.unwrap().qualified_name().unwrap().pivot,
        "_x0041_"
    );
}
#[test]
fn strict_aliases_cdata_entities_and_quoted_sheet_names_keep_their_binding() {
    let xml=chart("<c:pivotSource><c:name><![CDATA['[Book.xlsx]O''Brien & West'!Pivot Name]]></c:name><c:fmtId val=\" 7 \"/></c:pivotSource>").replace("http://schemas.openxmlformats.org/drawingml/2006/chart","http://purl.oclc.org/ooxml/drawingml/chart").replace("c:","alias:").replace("xmlns:c=","xmlns:alias=");
    let c = parse_chart(&xml).unwrap();
    let p = c.pivot_source.unwrap();
    assert_eq!(p.format_id, 7);
    assert_eq!(p.qualified_name().unwrap().sheet, "O'Brien & West");
    let c=parse_chart(&chart("<c:pivotSource><c:name>[Book.xlsx]A &amp; B!P</c:name><c:fmtId val=\"0\"/></c:pivotSource>")).unwrap();
    assert_eq!(
        c.pivot_source.unwrap().qualified_name().unwrap().sheet,
        "A & B"
    );
}
#[test]
fn foreign_and_nested_lookalikes_do_not_supply_standard_pivot_bindings() {
    for body in ["<f:pivotSource><c:name>Sheet!P</c:name><c:fmtId val=\"0\"/></f:pivotSource>","<f:wrapper><c:pivotSource><c:name>Sheet!P</c:name><c:fmtId val=\"0\"/></c:pivotSource></f:wrapper>","<c:extLst><c:pivotSource><c:name>Sheet!P</c:name><c:fmtId val=\"0\"/></c:pivotSource></c:extLst>"] {
        assert!(parse_chart(&chart(body)).unwrap().pivot_source.is_none());
    }
    for body in ["<c:pivotSource><f:name>Sheet!P</f:name><c:fmtId val=\"0\"/></c:pivotSource>","<c:pivotSource><c:name>Sheet!P</c:name><c:fmtId val=\"bad\"/></c:pivotSource>","<c:pivotSource><c:name>Sheet!P</c:name><c:name>Other!P</c:name><c:fmtId val=\"0\"/></c:pivotSource>"] {
        let c=parse_chart(&chart(body)).unwrap();assert!(c.pivot_source.is_none());assert!(c.unsupported_properties.contains(&"invalid pivot chart source".into()));
    }
}
#[test]
fn source_names_require_a_sheet_and_never_resolve_a_path_or_guess_a_bare_pivot() {
    for name in [
        "",
        "Pivot",
        "[Book.xlsx]Pivot",
        "[Book.xlsx]!Pivot",
        "Sheet!",
        "'[Book.xlsx]Sheet!Pivot",
    ] {
        assert!(
            PivotSource {
                name: name.into(),
                format_id: 0
            }
            .qualified_name()
            .is_none(),
            "{name}"
        );
    }
}
