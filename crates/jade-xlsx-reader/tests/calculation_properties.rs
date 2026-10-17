use jade_xlsx_reader::{workbook_part::parse_workbook, CalculationMode, Workbook};

#[test]
fn calculation_defaults_and_saved_modes_are_read_without_guessing_invalid_modes() {
    let book = parse_workbook("<workbook><sheets/></workbook>").unwrap();
    assert_eq!(book.calculation_mode, CalculationMode::Auto);
    assert!(Workbook::default().calculate_on_save);
    for mode in ["auto", "manual", "autoNoTable"] {
        let xml = format!("<workbook><calcPr calcMode='{mode}' calcOnSave='false'/></workbook>");
        let book = parse_workbook(&xml).unwrap();
        assert_eq!(book.calculation_mode.as_str(), mode);
        assert_eq!(book.calculate_on_save, Some(false));
    }
    assert!(parse_workbook("<workbook><calcPr calcMode='invalid'/></workbook>").is_err());
}

#[test]
fn strict_prefixed_preferences_ignore_foreign_attributes_and_nested_properties() {
    let book=parse_workbook(r#"<s:workbook xmlns:s="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:q="urn:foreign"><q:calcPr calcMode="auto" calcOnSave="0"/><s:calcPr q:calcMode="auto" calcMode="manual" q:calcOnSave="0" calcOnSave="1" fullCalcOnLoad="1"/><s:extLst><s:calcPr calcMode="auto"/></s:extLst></s:workbook>"#).unwrap();
    assert_eq!(book.calculation_mode, CalculationMode::Manual);
    assert_eq!(book.calculate_on_save, Some(true));
    assert!(book.full_calculation_on_load);
    let wrong_root=parse_workbook(r#"<q:workbook xmlns:q="urn:foreign" xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><s:calcPr calcMode="manual"/></q:workbook>"#).unwrap();
    assert_eq!(wrong_root.calculation_mode, CalculationMode::Auto);
    let unbound_child=parse_workbook(r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><calcPr xmlns="" calcMode="manual"/></workbook>"#).unwrap();
    assert_eq!(unbound_child.calculation_mode, CalculationMode::Auto);
}
