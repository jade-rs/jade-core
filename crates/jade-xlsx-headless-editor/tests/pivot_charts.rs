use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{chart::Chart, read_xlsx};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-chart-compat.xlsx");
fn chart(bytes: &[u8]) -> Chart {
    let b = read_xlsx(bytes).unwrap();
    b.sheets
        .iter()
        .flat_map(|s| &s.shapes)
        .find_map(|s| s.chart.clone())
        .unwrap()
}
#[test]
fn a_refreshed_pivot_chart_adds_categories_without_totals_and_undoes_as_one_edit() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    assert_eq!(chart(b.bytes()).series[0].values.points.len(), 2);
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let after = b.bytes().to_vec();
    let c = chart(&after);
    assert_eq!(
        c.series[0].categories.points,
        [
            Some("East".into()),
            Some("North".into()),
            Some("West".into())
        ]
    );
    assert_eq!(
        c.series[0].values.points,
        [Some("15".into()), Some("20".into()), Some("7".into())]
    );
    assert_eq!(c.series[0].color.rgb, Some([51, 102, 153]));
    assert_eq!(
        c.series[0].point_colors.get(&2).unwrap().rgb,
        Some([255, 153, 0])
    );
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}

fn edit(bytes: &[u8], part: &str, f: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut p = jade_xlsx_reader::OpcPackage::open(bytes).unwrap();
    let s = p.required_part_str(part).unwrap();
    p.replace(part, f(s).into_bytes());
    p.write().unwrap()
}
fn two_measures() -> Vec<u8> {
    let bytes = edit(FIXTURE, "xl/pivotTables/report.xml", |s| {
        s.replace("<dataFields count=\"1\">", "<dataFields count=\"2\">")
            .replace(
                "</dataFields>",
                "<dataField name=\"Count Amount\" fld=\"1\" subtotal=\"count\"/></dataFields>",
            )
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    b.bytes().to_vec()
}
fn transform_series(xml: String, idx: u32, f: impl FnOnce(String) -> String) -> String {
    let start = xml.find(&format!("<c:ser><c:idx val=\"{idx}\"/>")).unwrap();
    let end = start + xml[start..].find("</c:ser>").unwrap() + "</c:ser>".len();
    format!(
        "{}{}{}",
        &xml[..start],
        f(xml[start..end].into()),
        &xml[end..]
    )
}
#[test]
fn repeated_refresh_and_reopen_use_the_current_visible_report() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(
        chart(b.bytes()).series[0].values.points,
        [Some("105".into()), Some("27".into())]
    );
    let mut b = HeadlessWorkbook::open_for_editing(b.bytes()).unwrap();
    b.set("Data", "B3", CellInput::Number(200.)).unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(
        chart(b.bytes()).series[0].values.points,
        [Some("105".into()), Some("207".into())]
    );
    let p = jade_xlsx_reader::OpcPackage::open(b.bytes()).unwrap();
    let xml = p.required_part_str("xl/charts/pivot.xml").unwrap();
    assert!(xml.contains("<j:payload j:caption=\"preserved\"/>"));
    assert!(xml.contains("Regional Amount"));
}
#[test]
fn removed_members_remove_only_their_point_formatting_and_never_chart_the_grand_total() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    for at in ["A3", "A5"] {
        b.set("Data", at, CellInput::Text("East".into())).unwrap();
    }
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let c = chart(b.bytes());
    assert_eq!(c.series[0].categories.points, [Some("East".into())]);
    assert_eq!(c.series[0].values.points, [Some("42".into())]);
    assert!(c.series[0].point_colors.is_empty());
}
#[test]
fn additional_measures_create_real_series_and_reordered_series_keep_their_colors() {
    let bytes = two_measures();
    assert_eq!(chart(&bytes).series.len(), 2);
    let bytes = edit(&bytes, "xl/charts/pivot.xml", |s| {
        let s = transform_series(s, 0, |s| {
            s.replace("<c:order val=\"0\"/>", "<c:order val=\"1\"/>")
        });
        transform_series(s, 1, |s| {
            s.replace("<c:order val=\"1\"/>", "<c:order val=\"0\"/>")
                .replace(
                "</c:tx>",
                "</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"CC3366\"/></a:solidFill></c:spPr>",
            )
        })
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let c = chart(b.bytes());
    assert_eq!(c.series.len(), 2);
    assert_eq!(c.series[0].name.points, [Some("Count Amount".into())]);
    assert_eq!(c.series[0].color.rgb, Some([204, 51, 102]));
    assert_eq!(
        c.series[0].values.points,
        [Some("2".into()), Some("1".into()), Some("1".into())]
    );
    assert_eq!(c.series[1].name.points, [Some("Total Amount".into())]);
    assert_eq!(c.series[1].color.rgb, Some([51, 102, 153]));
    assert_eq!(
        c.series[1].values.points,
        [Some("15".into()), Some("20".into()), Some("7".into())]
    );
}
#[test]
fn combined_column_and_line_plots_keep_their_series_roles_when_categories_grow() {
    let bytes = edit(&two_measures(), "xl/charts/pivot.xml", |xml| {
        let start = xml.find("<c:ser><c:idx val=\"1\"/>").unwrap();
        let end = start + xml[start..].find("</c:ser>").unwrap() + "</c:ser>".len();
        let line = xml[start..end].to_string();
        let xml = format!("{}{}", &xml[..start], &xml[end..]);
        xml.replace("</c:barChart>",&format!("</c:barChart><c:lineChart>{line}<c:axId val=\"1\"/><c:axId val=\"2\"/></c:lineChart>"))
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.refresh_all_pivots().unwrap();
    let c = chart(b.bytes());
    assert_eq!(c.kind, "barChart");
    assert_eq!(c.series.len(), 1);
    assert_eq!(c.extra_plots.len(), 1);
    assert_eq!(c.extra_plots[0].kind, "lineChart");
    assert_eq!(
        c.series[0].values.points,
        [Some("15".into()), Some("20".into()), Some("7".into())]
    );
    assert_eq!(
        c.extra_plots[0].series[0].values.points,
        [Some("2".into()), Some("1".into()), Some("1".into())]
    );
}
#[test]
fn malformed_chart_identity_rolls_back_caches_widths_and_the_redo_branch() {
    let bytes = edit(&two_measures(), "xl/charts/pivot.xml", |s| {
        s.replace("<c:ser><c:idx val=\"1\"/>", "<c:ser><c:idx val=\"0\"/>")
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    let before = b.bytes().to_vec();
    b.set("Data", "B3", CellInput::Number(99.)).unwrap();
    let future = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert!(b
        .refresh_all_pivots()
        .unwrap_err()
        .to_string()
        .contains("duplicate series identity"));
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), future);
}
#[test]
fn literal_xstring_sequences_in_the_pivot_name_remain_literal_chart_bindings() {
    let bytes = edit(FIXTURE, "xl/pivotTables/report.xml", |s| {
        s.replace("RegionalSales", "_x005F_x0041_")
    });
    let bytes = edit(&bytes, "xl/charts/pivot.xml", |s| {
        s.replace("RegionalSales", "_x0041_")
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.refresh_pivot("Report", "_x0041_").unwrap();
    assert_eq!(chart(b.bytes()).series[0].values.points.len(), 3);
}

#[test]
fn explicit_external_pivot_data_is_preserved_without_local_rebinding() {
    let bytes = edit(FIXTURE, "xl/charts/pivot.xml", |s| {
        s.replace("</c:chart>","</c:chart><c:externalData xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"external\"/>")
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let original = jade_xlsx_reader::OpcPackage::open(&bytes)
        .unwrap()
        .required_part_str("xl/charts/pivot.xml")
        .unwrap();
    assert!(chart(&bytes)
        .unsupported_properties
        .contains(&"external chart data".into()));
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(b.cell("Report", "F7").unwrap().number, Some(42.));
    assert_eq!(
        jade_xlsx_reader::OpcPackage::open(b.bytes())
            .unwrap()
            .required_part_str("xl/charts/pivot.xml")
            .unwrap(),
        original
    );
}
#[test]
fn contiguous_sources_keep_their_formulas_and_opaque_data_source_extensions() {
    let bytes = edit(FIXTURE, "xl/charts/pivot.xml", |s| {
        s.replace(
            "</c:numRef>",
            "<c:extLst><c:ext uri=\"urn:jade:data\"><j:keep/></c:ext></c:extLst></c:numRef>",
        )
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let c = chart(b.bytes());
    assert_eq!(
        c.series[0].values.formula.as_deref(),
        Some("'Report'!$F$4:$F$6")
    );
    assert_eq!(
        c.series[0].categories.formula.as_deref(),
        Some("'Report'!$E$4:$E$6")
    );
    assert_eq!(
        c.series[0].values.points,
        [Some("15".into()), Some("20".into()), Some("7".into())]
    );
    assert!(jade_xlsx_reader::OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str("xl/charts/pivot.xml")
        .unwrap()
        .contains("<c:extLst><c:ext uri=\"urn:jade:data\"><j:keep/></c:ext></c:extLst>"));
}

#[test]
fn noncontiguous_source_conversion_refuses_opaque_reference_metadata_atomically() {
    let bytes = edit(FIXTURE, "xl/charts/pivot.xml", |s| {
        let a = s.find("<c:pivotSource>").unwrap();
        let z = s.find("</c:pivotSource>").unwrap() + "</c:pivotSource>".len();
        format!("{}{}", &s[..a], &s[z..])
    });
    let bytes = edit(&bytes, "xl/pivotTables/report.xml", |s| {
        s.replace(
            "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
            "<rowFields count=\"2\"><field x=\"0\"/><field x=\"2\"/></rowFields>",
        )
        .replace(
            "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
            "<pageFields count=\"0\"/>",
        )
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let bytes = edit(b.bytes(), "xl/charts/pivot.xml", |s| {
        s.replace("<c:chart>","<c:pivotSource><c:name>[pivot-chart-compat.xlsx]Report!RegionalSales</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart>").replace("'Report'!$F$4:$F$5","'Report'!$F$5:$F$7").replace("'Report'!$E$4:$E$5","'Report'!$E$5:$E$7").replace("</c:numRef>","<c:extLst><c:ext uri=\"urn:jade:data\"><j:keep/></c:ext></c:extLst></c:numRef>")
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    let before = b.bytes().to_vec();
    assert!(b
        .refresh_pivot("Report", "RegionalSales")
        .unwrap_err()
        .to_string()
        .contains("unsupported data-source metadata"));
    assert_eq!(b.bytes(), before);
}
#[test]
fn new_chart_formulas_preserve_literal_xstrings_and_quoted_sheet_names() {
    for (attribute, binding, formula, target) in [
        (
            "Report_x005F_x0041_",
            "Report_x0041_",
            "'Report_x0041_'",
            "Report_x0041_",
        ),
        (
            "O&apos;Brien &amp; Sales",
            "'O''Brien &amp; Sales'",
            "'O''Brien &amp; Sales'",
            "O'Brien & Sales",
        ),
    ] {
        let bytes = edit(FIXTURE, "xl/workbook.xml", |s| {
            s.replace("name=\"Report\"", &format!("name=\"{attribute}\""))
        });
        let bytes = edit(&bytes, "xl/charts/pivot.xml", |s| {
            s.replace("]Report!", &format!("]{binding}!"))
                .replace("'Report'!", &format!("{formula}!"))
        });
        let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        b.set("Data", "A3", CellInput::Text("North".into()))
            .unwrap();
        b.refresh_pivot(target, "RegionalSales").unwrap();
        let c = chart(b.bytes());
        let expected = format!("'{}'!$F$4:$F$6", target.replace('\'', "''"));
        assert_eq!(
            c.series[0].values.formula.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(c.series[0].values.points.len(), 3);
    }
}

#[test]
fn renamed_literal_source_sheet_remains_refreshable_after_reopen() {
    let mut book = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    book.rename_sheet("Data", "Data_x0041_").unwrap();
    let bytes = book.bytes().to_vec();
    let parsed = read_xlsx(&bytes).unwrap();
    assert_eq!(
        parsed.pivots.caches[0].source.sheet.as_deref(),
        Some("Data_x0041_")
    );
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    book.set("Data_x0041_", "A3", CellInput::Text("North".into()))
        .unwrap();
    let before = book.bytes().to_vec();
    book.refresh_pivot("Report", "RegionalSales").unwrap();
    let after = book.bytes().to_vec();
    assert_eq!(chart(&after).series[0].values.points.len(), 3);
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), after);
}

#[test]
fn report_tab_rename_preserves_the_chart_binding_and_plain_formula_identity() {
    let mut book = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    let target = "O'Brien & _x0041_";
    let before = book.bytes().to_vec();
    book.rename_sheet("Report", target).unwrap();
    let renamed = book.bytes().to_vec();
    let bound = chart(&renamed)
        .pivot_source
        .unwrap()
        .qualified_name()
        .unwrap();
    assert_eq!(bound.sheet, target);
    assert_eq!(bound.workbook.as_deref(), Some("pivot-chart-compat.xlsx"));
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), renamed);
    let mut book = HeadlessWorkbook::open_for_editing(&renamed).unwrap();
    book.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    book.refresh_pivot(target, "RegionalSales").unwrap();
    let chart = chart(book.bytes());
    assert_eq!(chart.series[0].values.points.len(), 3);
    assert_eq!(
        chart.series[0].values.formula.as_deref(),
        Some("'O''Brien & _x0041_'!$F$4:$F$6")
    );
}

#[test]
fn an_encoded_named_pivot_source_resolves_its_logical_workbook_name() {
    let bytes = edit(FIXTURE, "xl/workbook.xml", |s| {
        s.replace("<pivotCaches>", "<definedNames><definedName name=\"Input_x005F_x0041_\">Data!$A$1:$C$5</definedName></definedNames><pivotCaches>")
    });
    let bytes = edit(&bytes, "xl/pivotCache/cache.xml", |s| {
        s.replace("ref=\"A1:C5\"", "name=\"Input_x005F_x0041_\"")
    });
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    book.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    book.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(chart(book.bytes()).series[0].values.points.len(), 3);
}
