use jade_xlsx_reader::workbook_part::parse_workbook;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";

#[test]
fn workbook_structure_uses_owned_unqualified_xml_schema_boolean() {
    for ns in [MAIN, STRICT, ""] {
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("0", false),
            ("false", false),
            (" &#x31; ", true),
        ] {
            let xml = if ns.is_empty() {
                format!(r#"<workbook><workbookProtection lockStructure="{value}"/></workbook>"#)
            } else {
                format!(
                    r#"<s:workbook xmlns:s="{ns}" xmlns:a="{ns}"><a:workbookProtection lockStructure="{value}"/></s:workbook>"#
                )
            };
            assert_eq!(
                parse_workbook(&xml).unwrap().structure_protected,
                expected,
                "{xml}"
            );
        }
    }
    assert!(!parse_workbook(&format!(r#"<workbook xmlns="{MAIN}"><workbookProtection lockWindows="1" lockRevision="1" workbookPassword="ABCD"/></workbook>"#)).unwrap().structure_protected);
}

#[test]
fn foreign_nested_and_qualified_attributes_cannot_impersonate_protection() {
    for inner in [
        r#"<u:workbookProtection lockStructure="1"/>"#,
        r#"<extLst><ext uri="opaque"><workbookProtection lockStructure="1"/></ext></extLst>"#,
        r#"<u:wrapper><workbookProtection lockStructure="1"/></u:wrapper>"#,
        r#"<workbookProtection u:lockStructure="1"/>"#,
        r#"<u:workbookProtection lockStructure="2"/>"#,
    ] {
        let xml = format!(r#"<workbook xmlns="{MAIN}" xmlns:u="urn:foreign">{inner}</workbook>"#);
        assert!(!parse_workbook(&xml).unwrap().structure_protected, "{xml}");
    }
    assert!(
        !parse_workbook(
            r#"<workbook xmlns="urn:foreign"><workbookProtection lockStructure="1"/></workbook>"#
        )
        .unwrap()
        .structure_protected
    );
}

#[test]
fn malformed_owned_protection_fails_instead_of_silently_unlocking() {
    for value in ["", "2", "yes", "TRUE", "&#xA0;true&#xA0;"] {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}"><workbookProtection lockStructure="{value}"/></workbook>"#
        );
        assert!(parse_workbook(&xml).is_err(), "{xml}");
    }
    let xml = format!(
        r#"<workbook xmlns="{MAIN}"><workbookProtection lockStructure="1"/><workbookProtection lockStructure="0"/></workbook>"#
    );
    assert!(parse_workbook(&xml).unwrap().structure_protected);
}
