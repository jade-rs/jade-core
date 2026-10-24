use jade_xlsx_reader::sheet::parse_sheet;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn parse(body: &str) -> jade_xlsx_reader::Result<jade_xlsx_reader::Sheet> {
    parse_sheet(
        "Sheet",
        "sheet.xml",
        &format!(
            r#"<worksheet xmlns="{MAIN}" xmlns:f="urn:foreign">{body}<sheetData/></worksheet>"#
        ),
        &[],
    )
}
#[test]
fn nested_and_foreign_protection_never_override_the_sheet() {
    assert!(
        parse(r#"<extLst><ext uri="x"><sheetProtection sheet="1"/></ext></extLst>"#)
            .unwrap()
            .protection
            .is_none()
    );
    assert!(parse(r#"<f:sheetProtection sheet="1"/>"#)
        .unwrap()
        .protection
        .is_none());
    assert!(parse(r#"<sheetProtection sheet="1"/><extLst><ext uri="x"><sheetProtection sheet="0"/></ext></extLst>"#).unwrap().protection.is_some());
}
#[test]
fn malformed_and_duplicate_owned_protection_is_refused() {
    for body in [
        r#"<sheetProtection sheet="yes"/>"#,
        r#"<sheetProtection sheet="1" formatCells="bad"/>"#,
        r#"<sheetProtection sheet="1"/><sheetProtection sheet="0"/>"#,
    ] {
        assert!(parse(body).is_err(), "{body}");
    }
}

#[test]
fn allowed_flags_and_namespace_aliases_use_schema_defaults_and_booleans() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main", ""] {
        let xml = format!(
            r#"<s:worksheet xmlns:s="{ns}" xmlns:p="{ns}" xmlns:f="urn:foreign"><p:sheetProtection sheet=" &#x31; " selectLockedCells="1" selectUnlockedCells="0" formatCells="false" pivotTables="0" scenarios="1" f:objects="true"/><s:sheetData/></s:worksheet>"#
        );
        // Empty namespace bindings cannot be assigned to a prefix in XML.
        let xml = if ns.is_empty() {
            xml.replace("s:", "")
                .replace("p:", "")
                .replace("xmlns:s=\"\"", "")
                .replace("xmlns:p=\"\"", "")
        } else {
            xml
        };
        let p = parse_sheet("Sheet", "sheet.xml", &xml, &[])
            .unwrap()
            .protection
            .unwrap();
        assert!(!p.select_locked_cells);
        assert!(p.select_unlocked_cells);
        assert!(p.format_cells && p.pivot_tables && p.objects);
        assert!(!p.scenarios);
        assert!(!p.format_rows && !p.insert_hyperlinks && !p.sort);
    }
}
#[test]
fn protection_styles_inherit_and_preserve_explicit_values_and_foreign_children() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        let xml = format!(
            r#"<s:styleSheet xmlns:s="{ns}" xmlns:f="urn:foreign"><s:cellStyleXfs count="2"><s:xf><s:protection locked="0" hidden="1"/></s:xf><s:xf applyProtection="0"><s:protection locked="0" hidden="1"/></s:xf></s:cellStyleXfs><s:cellXfs count="5"><s:xf xfId="0"/><s:xf xfId="0" applyProtection="0"><s:protection locked="1" hidden="0"/></s:xf><s:xf xfId="1"/><s:xf><f:protection locked="0" hidden="1"/></s:xf><s:xf><s:protection hidden="true"/></s:xf></s:cellXfs></s:styleSheet>"#
        );
        let s = jade_xlsx_reader::styles::parse_styles(&xml).unwrap();
        let flags: Vec<_> = s
            .cell_formats
            .iter()
            .map(|f| (f.unlocked, f.formula_hidden))
            .collect();
        assert_eq!(
            flags,
            [
                (true, true),
                (false, false),
                (false, false),
                (false, false),
                (false, true)
            ]
        );
    }
    assert!(jade_xlsx_reader::styles::parse_styles(
        r#"<styleSheet><cellXfs><xf><protection locked="yes"/></xf></cellXfs></styleSheet>"#
    )
    .is_err());
}

#[test]
fn independent_object_and_scenario_locks_do_not_imply_cell_protection() {
    for namespace in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main", ""] {
        let xml = format!(
            r#"<worksheet xmlns="{namespace}"><sheetData/><sheetProtection sheet="0" objects="1" scenarios="true" formatCells="1"/></worksheet>"#
        );
        let state = jade_xlsx_reader::sheet_protection::read(&xml)
            .unwrap()
            .unwrap();
        assert!(!state.sheet && !state.allowed.objects && !state.allowed.scenarios);
        assert!(parse_sheet("Sheet", "sheet.xml", &xml, &[])
            .unwrap()
            .protection
            .is_none());
    }
    let state = jade_xlsx_reader::sheet_protection::read(
        "<worksheet><sheetData/><sheetProtection/></worksheet>",
    )
    .unwrap()
    .unwrap();
    assert!(!state.sheet && state.allowed.objects && state.allowed.scenarios);
}

#[test]
fn independent_lock_query_shares_owned_projection_and_mce_choice_policy() {
    for body in [
        r#"<f:sheetProtection objects="1"/>"#,
        r#"<extLst><ext uri="x"><sheetProtection objects="1"/></ext></extLst>"#,
    ] {
        let xml = format!(
            r#"<worksheet xmlns="{MAIN}" xmlns:f="urn:foreign"><sheetData/>{body}</worksheet>"#
        );
        assert!(jade_xlsx_reader::sheet_protection::read(&xml)
            .unwrap()
            .is_none());
    }
    let xml = format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:unknown"><s:sheetData/><mc:AlternateContent><mc:Choice Requires="u"><s:sheetProtection objects="0" scenarios="0"/></mc:Choice><mc:Fallback><s:sheetProtection objects="1" scenarios="1"/></mc:Fallback></mc:AlternateContent></s:worksheet>"#
    );
    let state = jade_xlsx_reader::sheet_protection::read(&xml)
        .unwrap()
        .unwrap();
    assert!(!state.sheet && !state.allowed.objects && !state.allowed.scenarios);
    for body in [
        r#"<sheetProtection objects="yes"/>"#,
        r#"<sheetProtection/><sheetProtection/>"#,
    ] {
        let xml = format!("<worksheet><sheetData/>{body}</worksheet>");
        assert!(jade_xlsx_reader::sheet_protection::read(&xml).is_err());
        assert!(parse_sheet("Sheet", "sheet.xml", &xml, &[]).is_err());
    }
}
