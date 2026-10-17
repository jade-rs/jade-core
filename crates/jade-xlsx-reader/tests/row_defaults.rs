use jade_xlsx_reader::sheet::parse_sheet;
#[test]
fn row_defaults_are_inherited_and_explicit_false_overrides_are_retained() {
    let sheet = parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="25" customHeight="1" zeroHeight="1"/><sheetData><row r="1"/><row r="2" hidden="0" customHeight="false" ht="18"/><row r="3" hidden="true" customHeight="true"/></sheetData></worksheet>"#, &[]).unwrap();
    assert_eq!(sheet.default_row_height_points, Some(25.0));
    assert!(sheet.default_row_custom_height);
    assert!(sheet.default_rows_hidden);
    assert!(sheet.rows[0].hidden && sheet.rows[0].custom_height);
    assert!(!sheet.rows[1].hidden && !sheet.rows[1].custom_height);
    assert_eq!(sheet.rows[1].height_points, Some(18.0));
    assert!(sheet.rows[2].hidden && sheet.rows[2].custom_height);
}
