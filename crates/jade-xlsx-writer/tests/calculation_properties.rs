use jade_opc::OpcPackage;
use jade_xlsx_reader::CalculationMode;
use jade_xlsx_writer::{request_full_recalculation, withdraw_full_recalculation};

#[test]
fn freshness_flags_handle_quotes_whitespace_and_foreign_names_without_reformatting_xml() {
    let xml = r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:q="urn:foreign"><q:calcPr fullCalcOnLoad="0"/><x:calcPr fullCalcOnLoad = '0' q:fullCalcOnLoad="7" calcMode="manual" calcOnSave="0"/></x:workbook>"#;
    let requested = request_full_recalculation(xml);
    assert_eq!(
        requested,
        xml.replace("fullCalcOnLoad = '0'", "fullCalcOnLoad = '1'")
    );
    assert_eq!(withdraw_full_recalculation(&requested), xml);
    let with_comment =
        r#"<workbook><!-- <calcPr fullCalcOnLoad="0"/> --><sheets/><extLst/></workbook>"#;
    let written = request_full_recalculation(with_comment);
    assert!(written.contains(r#"<!-- <calcPr fullCalcOnLoad="0"/> -->"#));
    assert!(
        written.contains(r#"<sheets/><calcPr calcId="0" fullCalcOnLoad="1"/><extLst/>"#),
        "{written}"
    );
}

#[test]
fn saved_preferences_preserve_unrelated_package_parts_and_still_order_calc_pr_before_extensions() {
    let source = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let mut package = OpcPackage::open(source).unwrap();
    let xml = package
        .part_str("xl/workbook.xml")
        .unwrap()
        .unwrap()
        .replace("</workbook>", "<extLst/></workbook>");
    package.replace("xl/workbook.xml", xml.into_bytes());
    let source = package.write().unwrap();
    let written =
        jade_xlsx_writer::set_calculation_properties(&source, CalculationMode::Manual, false)
            .unwrap();
    let after = OpcPackage::open(&written).unwrap();
    let xml = after.part_str("xl/workbook.xml").unwrap().unwrap();
    assert!(
        xml.contains(r#"<calcPr calcId="0" calcMode="manual" calcOnSave="0"/><extLst/>"#),
        "{xml}"
    );
    assert_eq!(
        after.part("xl/worksheets/sheet1.xml"),
        package.part("xl/worksheets/sheet1.xml")
    );
    let model = jade_xlsx_reader::read_xlsx(&written).unwrap();
    assert_eq!(model.calculation_mode, CalculationMode::Manual);
    assert!(!model.calculate_on_save);
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("calculation-properties.xlsx"),
            written,
        )
        .unwrap();
    }
}
