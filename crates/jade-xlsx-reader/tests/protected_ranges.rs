use jade_xlsx_reader::protected_ranges::*;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn sheet(body: &str) -> String {
    format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:p="{MAIN}" xmlns:x="{}" xmlns:m="{}" xmlns:f="urn:f">{body}<s:sheetData/></s:worksheet>"#,
        String::from_utf8_lossy(X14),
        String::from_utf8_lossy(XM)
    )
}
fn extended(record: &str) -> String {
    format!(
        r#"<s:extLst><s:ext uri="{EXT_URI}"><x:protectedRanges>{record}</x:protectedRanges></s:ext></s:extLst>"#
    )
}
#[test]
fn standard_aliases_strict_xstrings_and_passwordless_metadata_are_read() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        let xml=sheet(r#"<p:protectedRanges><p:protectedRange name="_x65E5_本" sqref="$A$1 A3:B4" securityDescriptor="ignored without password"/><p:protectedRange name="Zero" sqref="C1" password="0000"/></p:protectedRanges>"#).replace(MAIN,ns);
        let ranges = protected_ranges(&xml).unwrap();
        assert_eq!(ranges.len(), 2);
        assert!(ranges
            .iter()
            .all(|r| !r.requires_password() && !r.ambiguous));
        let range = ranges.iter().find(|r| r.name == "日本").unwrap();
        assert_eq!(range.ranges.len(), 2);
        assert_eq!(range.security_descriptors, ["ignored without password"]);
        let records = sources(&xml).unwrap();
        assert_eq!(records.containers.len(), 1);
        assert_eq!(records.records.len(), 2);
        for record in &records.records {
            assert!(xml[record.span.clone()].starts_with("<p:protectedRange"));
        }
    }
}
#[test]
fn x14_uses_only_the_known_extension_and_one_direct_xm_reference() {
    let valid = extended(
        r#"<x:protectedRange name="Input" password="83AF"><m:sqref><![CDATA[A1:B2]]></m:sqref></x:protectedRange>"#,
    );
    let xml = sheet(&valid);
    let ranges = protected_ranges(&xml).unwrap();
    assert_eq!(ranges.len(), 1);
    assert!(ranges[0].requires_password());
    assert_eq!(
        ranges[0].ranges[0],
        jade_xlsx_reader::CellRange::parse("A1:B2").unwrap()
    );
    assert!(
        protected_ranges(&sheet(&valid.replace(EXT_URI, "urn:opaque")))
            .unwrap()
            .is_empty()
    );
    for record in [
        r#"<x:protectedRange name="Bad"><m:sqref>A1</m:sqref><m:sqref>B2</m:sqref></x:protectedRange>"#,
        r#"<x:protectedRange name="Bad"><f:sqref>A1</f:sqref></x:protectedRange>"#,
        r#"<x:protectedRange name="Bad"><m:sqref>A1<f:payload>B2</f:payload></m:sqref></x:protectedRange>"#,
    ] {
        assert!(protected_ranges(&sheet(&extended(record))).unwrap()[0].ambiguous);
    }
}
#[test]
fn foreign_and_nested_lookalikes_are_never_edit_permissions() {
    let xml = sheet(
        r#"<f:protectedRanges><s:protectedRange name="Fake1" sqref="A1"/></f:protectedRanges><s:protectedRanges><f:protectedRange name="Fake2" sqref="A1"/><f:wrapper><s:protectedRange name="Fake3" sqref="A1"/></f:wrapper><s:protectedRange name="Real" sqref="B2"/></s:protectedRanges>"#,
    );
    let ranges = protected_ranges(&xml).unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].name, "Real");
    assert_eq!(sources(&xml).unwrap().records.len(), 1);
}
#[test]
fn mirrored_families_merge_coverage_and_keep_modern_metadata_authoritative() {
    let xml = sheet(&format!(
        r#"<s:protectedRanges><s:protectedRange name="Inputs" sqref="A1:A2"/></s:protectedRanges>{}"#,
        extended(
            r#"<x:protectedRange name="inputs" hashValue="AA=="><m:sqref>A1 A2</m:sqref></x:protectedRange>"#
        )
    ));
    let ranges = protected_ranges(&xml).unwrap();
    assert_eq!(ranges.len(), 1);
    let range = &ranges[0];
    assert!(!range.ambiguous);
    assert!(range.requires_password());
    let passwords = range.password_verifiers().collect::<Vec<_>>();
    assert_eq!(passwords.len(), 1);
    assert!(passwords[0].modern());
    assert!(
        protected_ranges(&xml.replace("<m:sqref>A1 A2", "<m:sqref>B1 B2")).unwrap()[0].ambiguous
    );
    let duplicate = sheet(
        r#"<s:protectedRanges><s:protectedRange name="A" sqref="A1"/><s:protectedRange name="a" sqref="A1"/></s:protectedRanges>"#,
    );
    assert!(protected_ranges(&duplicate).unwrap()[0].ambiguous);
}
#[test]
fn malformed_local_coverage_and_mce_do_not_become_passwordless_exceptions() {
    for reference in [
        "",
        "A0",
        "A1 other!B2",
        "B2:A1",
        "A1 XFE1",
        "A1048577",
        "A1\u{a0}B2",
    ] {
        let xml = sheet(&format!(
            r#"<s:protectedRanges><s:protectedRange name="Bad" sqref="{reference}"/></s:protectedRanges>"#
        ));
        assert!(protected_ranges(&xml).unwrap()[0].ambiguous, "{reference}");
    }
    let xml = sheet(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Fallback><s:protectedRanges><s:protectedRange name="A" sqref="A1"/></s:protectedRanges></mc:Fallback></mc:AlternateContent>"#,
    );
    assert!(sources(&xml).unwrap().unsupported_mce);
    assert!(protected_ranges(&xml).is_err());
}
#[test]
fn coverage_comparison_is_geometric_and_handles_whole_columns_without_materializing() {
    let a = parse_sqref("A1:B1048576").unwrap();
    let b = parse_sqref("A1:A1048576 B1:B1048576").unwrap();
    assert!(same_coverage(&a, &b));
    assert!(!same_coverage(&a, &parse_sqref("A1:A1048576").unwrap()));
    assert!(parse_sqref("A1:B2 junk").is_none());
}
