use jade_xlsx_reader::styles::parse_styles;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
#[test]
fn qualified_style_collections_keep_gallery_flags_defaults_and_empty_dxf_indices() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        let s = format!(
            r#"<m:styleSheet xmlns:m="{ns}"><m:dxfs count="2"><m:dxf/><m:dxf><m:font><m:b/></m:font></m:dxf></m:dxfs><m:tableStyles defaultPivotStyle="Chosen"><m:tableStyle name="Chosen" table="0" pivot="1"><m:tableStyleElement type="headerRow" dxfId="1"/></m:tableStyle></m:tableStyles></m:styleSheet>"#
        );
        let t = parse_styles(&s).unwrap();
        assert_eq!(t.differential_styles.len(), 2);
        assert_eq!(t.differential_styles[1].bold, Some(true));
        assert_eq!(t.default_pivot_style.as_deref(), Some("Chosen"));
        assert_eq!(t.table_styles[0].table, Some(false));
        assert_eq!(t.table_styles[0].pivot, Some(true));
    }
}
#[test]
fn foreign_parents_and_main_extensions_do_not_supply_collection_definitions() {
    let s = format!(
        r#"<m:styleSheet xmlns:m="{MAIN}" xmlns:f="urn:foreign"><f:wrapper><m:dxfs><m:dxf><m:font><m:b/></m:font></m:dxf></m:dxfs><m:tableStyles><m:tableStyle name="Spoof"/></m:tableStyles></f:wrapper><m:extLst><m:tableStyles><m:tableStyle name="Nested"/></m:tableStyles></m:extLst><m:dxfs><m:dxf/></m:dxfs><m:tableStyles><m:tableStyle name="Owned"/></m:tableStyles></m:styleSheet>"#
    );
    let t = parse_styles(&s).unwrap();
    assert_eq!(t.differential_styles.len(), 1);
    assert_eq!(t.differential_styles[0].bold, None);
    assert_eq!(
        t.table_styles
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Owned"]
    );
}
#[test]
fn nested_dxf_lookalikes_do_not_promote_font_fill_or_border_properties() {
    let s = format!(
        r#"<styleSheet xmlns="{MAIN}" xmlns:f="urn:foreign"><dxfs><dxf><f:wrapper><font><b/></font><fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill></f:wrapper><extLst><border><top style="thin"/></border></extLst></dxf></dxfs></styleSheet>"#
    );
    let t = parse_styles(&s).unwrap();
    let d = &t.differential_styles[0];
    assert_eq!(d.bold, None);
    assert_eq!(d.fill, None);
    assert_eq!(d.top, None);
}
