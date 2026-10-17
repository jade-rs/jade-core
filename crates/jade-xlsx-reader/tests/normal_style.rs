use jade_xlsx_reader::styles::parse_styles;
#[test]
fn localized_builtin_normal_uses_its_style_xf_font_without_polluting_cell_xfs() {
    let styles = parse_styles(r#"<styleSheet><fonts><font><name val="Other"/><sz val="10"/></font><font><name val="NormalDigits"/><sz val="22"/><b/></font></fonts><cellStyleXfs><xf fontId="0"/><xf fontId="1"><alignment indent="2"/></xf></cellStyleXfs><cellXfs><xf fontId="0"/></cellXfs><cellStyles><cellStyle name="標準" builtinId="00" xfId="1"/><cellStyle name="Normal" xfId="0"/></cellStyles></styleSheet>"#).unwrap();
    assert_eq!(styles.cell_formats.len(), 1);
    assert_eq!(styles.style_formats.len(), 2);
    assert_eq!(styles.normal_style_index, Some(1));
    let font = styles.normal_font().unwrap();
    assert_eq!(font.name.as_deref(), Some("NormalDigits"));
    assert_eq!(font.size_points, Some(22.0));
    assert!(font.bold);
}
#[test]
fn strict_prefixes_ignore_foreign_normal_entries_and_attribute_overrides() {
    let styles = parse_styles(r#"<s:styleSheet xmlns:s="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:x="urn:metadata"><s:fonts><s:font><s:name val="Fallback"/></s:font><s:font><s:name val="Actual"/></s:font></s:fonts><s:cellStyleXfs><s:xf fontId="0"/><s:xf fontId="1" x:fontId="0"/></s:cellStyleXfs><s:cellStyles><s:cellStyle name="標準" builtinId="0" xfId="1" x:xfId="0"/><x:cellStyle builtinId="0" xfId="0"/></s:cellStyles></s:styleSheet>"#).unwrap();
    assert_eq!(
        styles.normal_font().unwrap().name.as_deref(),
        Some("Actual")
    );
}
#[test]
fn legacy_named_normal_and_invalid_references_have_a_defined_fallback() {
    let styles = parse_styles(r#"<styleSheet><fonts><font><name val="First"/></font><font><name val="Named"/></font></fonts><cellStyleXfs><xf fontId="1"/></cellStyleXfs><cellStyles><cellStyle name="Normal" xfId="0"/></cellStyles></styleSheet>"#).unwrap();
    assert_eq!(styles.normal_font().unwrap().name.as_deref(), Some("Named"));
    let mut invalid = styles.clone();
    invalid.normal_style_index = Some(99);
    assert_eq!(
        invalid.normal_font().unwrap().name.as_deref(),
        Some("First")
    );
    invalid.normal_style_index = Some(0);
    invalid.style_formats[0].font_index = Some(99);
    assert_eq!(
        invalid.normal_font().unwrap().name.as_deref(),
        Some("First")
    );
}
