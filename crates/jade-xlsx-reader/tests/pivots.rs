use jade_xlsx_reader::{
    pivot::{parse_cache_definition, parse_cache_records, parse_pivot_table, PivotScalar},
    read_xlsx, CellRange, OpcPackage,
};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
fn fixture() -> OpcPackage {
    OpcPackage::open(include_bytes!(
        "../../../public/fixtures/pivot-cache-compat.xlsx"
    ))
    .unwrap()
}
fn cache() -> String {
    fixture()
        .required_part_str("xl/pivotCache/cache.xml")
        .unwrap()
        .to_string()
}
fn records() -> String {
    fixture()
        .required_part_str("xl/pivotCache/records.xml")
        .unwrap()
        .to_string()
}
fn table() -> String {
    fixture()
        .required_part_str("xl/pivotTables/report.xml")
        .unwrap()
        .to_string()
}
fn read(p: OpcPackage) -> jade_xlsx_reader::Workbook {
    read_xlsx(&p.write().unwrap()).unwrap()
}
fn change(p: &mut OpcPackage, part: &str, edit: impl FnOnce(String) -> String) {
    let xml = p.required_part_str(part).unwrap().to_string();
    p.replace(part, edit(xml).into_bytes());
}
#[test]
fn explicit_table_cache_links_cannot_override_the_workbook_cache_or_fetch_external_parts() {
    for (target, mode, valid) in [
        ("../pivotCache/cache.xml", "", true),
        ("/xl/pivotCache/cache.xml", "", true),
        ("../pivotCache/other.xml", "", false),
        (
            "https://example.invalid/cache.xml",
            " TargetMode=\"External\"",
            false,
        ),
    ] {
        let mut p = fixture();
        p.replace("xl/pivotTables/_rels/report.xml.rels",format!("<p:Relationships xmlns:p=\"http://schemas.openxmlformats.org/package/2006/relationships\"><p:Relationship Id=\"cache\" Type=\"{REL}/pivotCacheDefinition\" Target=\"{target}\"{mode}/></p:Relationships>").into_bytes());
        let book = read(p);
        assert_eq!(
            book.pivots.issues.is_empty(),
            valid,
            "{target}: {:?}",
            book.pivots.issues
        );
        assert_eq!(book.pivots.caches[0].part_name, "xl/pivotCache/cache.xml");
        assert_eq!(book.sheets.len(), 2);
    }
}
#[test]
fn foreign_cache_link_lookalikes_never_change_an_owned_table_binding() {
    let mut p = fixture();
    p.replace("xl/pivotTables/_rels/report.xml.rels",format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\" xmlns:f=\"urn:foreign\"><Relationship Id=\"cache\" Type=\"{REL}/pivotCacheDefinition\" Target=\"../pivotCache/cache.xml\"/><f:Relationship Id=\"decoy\" Type=\"{REL}/pivotCacheDefinition\" Target=\"https://example.invalid/wrong.xml\" TargetMode=\"External\"/></Relationships>").into_bytes());
    assert!(read(p).pivots.issues.is_empty());
}
#[test]
fn resolves_owned_tables_shared_cache_and_typed_record_indices() {
    let b = read(fixture());
    assert!(b.pivots.issues.is_empty(), "{:?}", b.pivots.issues);
    assert_eq!(b.pivots.tables.len(), 1);
    assert_eq!(b.pivots.caches.len(), 1);
    let c = &b.pivots.caches[0];
    assert_eq!(c.id, 7);
    assert_eq!(c.part_name, "xl/pivotCache/cache.xml");
    assert_eq!(c.records_part.as_deref(), Some("xl/pivotCache/records.xml"));
    assert_eq!(c.source.sheet.as_deref(), Some("Data"));
    assert_eq!(c.source.reference, Some(CellRange::parse("A1:C5").unwrap()));
    assert!(c.fields[0].shared_items[1].unused);
    assert_eq!(
        c.record_value(1, 0),
        Some(&PivotScalar::Text("West".into()))
    );
    assert_eq!(c.record_value(0, 1), Some(&PivotScalar::Number(10.)));
    assert_eq!(
        c.record_value(0, 2),
        Some(&PivotScalar::Date("2026-10-24T12:30:00".into()))
    );
    assert_eq!(c.record_value(1, 2), Some(&PivotScalar::Missing));
    let t = &b.pivots.tables[0];
    assert_eq!(t.sheet_part, "xl/worksheets/sheet2.xml");
    assert_eq!(t.cache_id, 7);
    assert_eq!(t.location.reference, CellRange::parse("E3:F6").unwrap());
    assert_eq!(t.location.first_data_row, 1);
    assert_eq!(t.fields[0].caption.as_deref(), Some("Territory"));
    assert_eq!(t.fields[0].items[1].shared_index, Some(2));
    assert_eq!(t.row_items[1].items, [1]);
    assert_eq!(t.row_items[2].kind, "grand");
    assert_eq!(t.column_fields, [-2]);
    assert_eq!(t.page_fields[0].field, 2);
    assert_eq!(t.data_fields[0].name.as_deref(), Some("Total Amount"));
    assert_eq!(t.data_fields[0].aggregation, "sum");
    assert_eq!(t.data_fields[0].number_format_id, Some(164));
    assert_eq!(t.style.as_ref().unwrap()["name"], "PivotStyleMedium9");
    assert!(t.unsupported.is_empty());
}
#[test]
fn strict_and_alias_namespaces_resolve_without_a_hardcoded_r_prefix() {
    let mut p = fixture();
    for part in [
        "xl/workbook.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
        "xl/pivotTables/report.xml",
        "xl/pivotCache/cache.xml",
        "xl/pivotCache/records.xml",
    ] {
        change(&mut p, part, |xml| {
            xml.replace(MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main")
                .replace(
                    REL,
                    "http://purl.oclc.org/ooxml/officeDocument/relationships",
                )
                .replace("xmlns:r=", "xmlns:link=")
                .replace("r:id=", "link:id=")
        });
    }
    for part in [
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/_rels/sheet2.xml.rels",
        "xl/pivotCache/_rels/cache.xml.rels",
    ] {
        change(&mut p, part, |xml| {
            xml.replace(
                REL,
                "http://purl.oclc.org/ooxml/officeDocument/relationships",
            )
        });
    }
    let b = read(p);
    assert!(b.pivots.issues.is_empty(), "{:?}", b.pivots.issues);
    assert_eq!(
        b.pivots.caches[0].record_value(3, 1),
        Some(&PivotScalar::Number(7.))
    );
}
#[test]
fn unknown_and_unused_shared_items_never_shift_record_indices() {
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/cache.xml", |s| {
        s.replace("<sharedItems count=\"3\">", "<sharedItems count=\"4\">")
            .replace(
                "<s v=\"West\"/>",
                r#"<x:opaque xmlns:x="urn:vendor" v="opaque"/><s v="West"/>"#,
            )
    });
    change(&mut p, "xl/pivotCache/records.xml", |s| {
        s.replace("<x v=\"2\"/>", "<x v=\"3\"/>")
    });
    let b = read(p);
    let c = &b.pivots.caches[0];
    assert_eq!(c.fields[0].shared_items.len(), 4);
    assert!(matches!(
        c.fields[0].shared_items[2].value,
        PivotScalar::Unknown { .. }
    ));
    assert_eq!(
        c.record_value(1, 0),
        Some(&PivotScalar::Text("West".into()))
    );
}
#[test]
fn typed_inline_values_and_xstring_text_are_not_coerced_or_dropped() {
    let r=parse_cache_records(&format!(r##"<pivotCacheRecords xmlns="{MAIN}" count="1"><r><s v="A_x0026_B &amp; C"/><b v="true"/><e v="#N/A"/><m/><n v="NaN"/><d v="1904-01-01T00:00:00"/></r></pivotCacheRecords>"##),6).unwrap();
    let v = &r.rows[0];
    assert_eq!(v[0].value, PivotScalar::Text("A&B & C".into()));
    assert_eq!(v[1].value, PivotScalar::Bool(true));
    assert_eq!(v[2].value, PivotScalar::Error("#N/A".into()));
    assert_eq!(v[3].value, PivotScalar::Missing);
    assert!(matches!(v[4].value, PivotScalar::Unknown { .. }));
    assert_eq!(v[5].value, PivotScalar::Date("1904-01-01T00:00:00".into()));
}
#[test]
fn grouping_calculated_metadata_filters_and_unknown_features_remain_explicit() {
    let grouping = r#"<fieldGroup base="1" par="2"><rangePr groupBy="months" startDate="2026-01-01T00:00:00"/><discretePr count="3"><x v="0"/><x v="1"/><x v="0"/></discretePr><groupItems count="2"><s v="Jan"/><s v="Feb"/></groupItems></fieldGroup>"#;
    let c = parse_cache_definition(
        7,
        "c",
        &cache()
            .replacen("</cacheField>", &format!("{grouping}</cacheField>"), 1)
            .replace(
                "</pivotCacheDefinition>",
                "<calculatedItems count=\"0\"/></pivotCacheDefinition>",
            ),
    )
    .unwrap();
    let g = c.fields[0].grouping.as_ref().unwrap();
    assert_eq!(g.base, Some(1));
    assert_eq!(g.discrete, [0, 1, 0]);
    assert_eq!(g.items.len(), 2);
    assert_eq!(g.range.as_ref().unwrap()["groupBy"], "months");
    assert!(c.unsupported.iter().any(|s| s.contains("calculatedItems")));
    let xml=table().replace("</pivotTableDefinition>",r#"<filters count="1"><filter fld="0" type="captionContains"><autoFilter ref="A1"><filterColumn colId="0"><customFilters><customFilter operator="equal" val="*a*"/></customFilters></filterColumn></autoFilter></filter></filters><extLst/></pivotTableDefinition>"#);
    let t = parse_pivot_table("s", "p", &xml).unwrap();
    assert_eq!(t.filters[0].attributes["type"], "captionContains");
    assert_eq!(t.filters[0].conditions[1].attributes["val"], "*a*");
    assert!(t.unsupported.iter().any(|s| s.contains("extLst")));
}
#[test]
fn foreign_cache_declarations_do_not_override_owned_worksheet_relationships() {
    let mut p = fixture();
    change(&mut p, "xl/workbook.xml", |s| {
        s.replace(
            "<pivotCaches>",
            r#"<f:wrapper xmlns:f="urn:foreign"><pivotCaches>"#,
        )
        .replace("</pivotCaches>", "</pivotCaches></f:wrapper>")
    });
    change(&mut p, "xl/worksheets/sheet2.xml", |s| {
        s.replace(
            "<pivotTableParts count=\"1\">",
            r#"<extLst><ext uri="x"><pivotTableParts count="1">"#,
        )
        .replace("</pivotTableParts>", "</pivotTableParts></ext></extLst>")
    });
    let b = read(p);
    assert!(b.pivots.caches.is_empty());
    // CT_Worksheet has no pivotTableParts list. The real related pivot still
    // belongs to this sheet, while the foreign cache declaration is ignored.
    assert_eq!(b.pivots.tables.len(), 1);
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("unresolved cacheId")));
}
#[test]
fn wrong_external_or_duplicate_relationships_do_not_bind_to_owned_parts() {
    for replacement in [
        format!("{REL}/table"),
        format!("{REL}/pivotTable\" TargetMode=\"External"),
    ] {
        let mut p = fixture();
        change(&mut p, "xl/worksheets/_rels/sheet2.xml.rels", |s| {
            s.replace(&format!("{REL}/pivotTable"), &replacement)
        });
        let b = read(p);
        assert!(b.pivots.tables.is_empty());
        if replacement.contains("TargetMode") {
            assert!(!b.pivots.issues.is_empty());
        } else {
            assert!(b.pivots.issues.is_empty());
        }
        assert_eq!(b.sheets.len(), 2);
    }
    let mut p = fixture();
    change(&mut p, "xl/worksheets/_rels/sheet2.xml.rels", |s| {
        s.replace("</Relationships>",&format!(r#"<Relationship Id="pivot" Type="{REL}/pivotTable" Target="../pivotTables/report.xml"/></Relationships>"#))
    });
    // Worksheet relationship ownership already rejects duplicate IDs before
    // the optional pivot catalog is read. Preserve that stricter contract.
    let error = read_xlsx(&p.write().unwrap()).unwrap_err();
    assert!(error
        .to_string()
        .contains("duplicate owned relationship id"));
}
#[test]
fn duplicate_cache_ids_and_repeated_table_ownership_are_reported_without_picking_a_winner() {
    let mut p = fixture();
    change(&mut p, "xl/workbook.xml", |s| {
        s.replace(
            "</pivotCaches>",
            r#"<pivotCache cacheId="7" r:id="sharedCache"/></pivotCaches>"#,
        )
    });
    let b = read(p);
    assert!(b.pivots.caches.is_empty());
    assert_eq!(b.pivots.tables.len(), 1);
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("ambiguous cacheId")));
    let mut p = fixture();
    // A second worksheet relationship claims the same part: real ambiguous
    // ownership, rather than a non-standard XML declaration marker.
    p.replace("xl/worksheets/_rels/sheet1.xml.rels",format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="alsoPivot" Type="{REL}/pivotTable" Target="../pivotTables/report.xml"/></Relationships>"#).into_bytes());
    let b = read(p);
    assert!(b.pivots.tables.is_empty());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("ownership")));
}
#[test]
fn invalid_record_indices_and_missing_record_parts_keep_definitions_and_cells_available() {
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/records.xml", |s| {
        s.replace("<x v=\"2\"/>", "<x v=\"999\"/>")
    });
    let b = read(p);
    assert_eq!(b.pivots.caches.len(), 1);
    assert!(b.pivots.caches[0].records.is_none());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("index out of bounds")));
    assert_eq!(b.pivots.tables.len(), 1);
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/_rels/cache.xml.rels", |s| {
        s.replace("records.xml", "absent.xml")
    });
    let b = read(p);
    assert!(b.pivots.caches[0].records.is_none());
    assert!(!b.pivots.issues.is_empty());
    assert_eq!(b.sheets[1].rows.len(), 4);
}
#[test]
fn malformed_counts_ranges_fields_and_references_do_not_panic_or_allocate_from_counts() {
    assert!(
        parse_cache_records(&records().replace("count=\"4\"", "count=\"4294967295\""), 3).is_err()
    );
    assert!(parse_cache_records(&records(), 2).is_err());
    assert!(parse_cache_records(&records().replace("<x v=\"2\"/>", "<x v=\"bad\"/>"), 3).is_err());
    for xml in [table().replace("E3:F6","F6:E3"),table().replace("firstDataRow=\"1\"","firstDataRow=\"999\""),table().replace("x=\"-2\"","x=\"-3\""),table().replace("<location ","<location xmlns=\"urn:foreign\" "),table().replace("<location ",&format!(r#"<location ref="E3:F6" firstHeaderRow="0" firstDataRow="1" firstDataCol="1"/><location "#))] {assert!(parse_pivot_table("s","p",&xml).is_err());}
}
#[test]
fn cache_record_count_and_record_relationship_ambiguity_are_diagnostics() {
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/cache.xml", |s| {
        s.replace("recordCount=\"4\"", "recordCount=\"5\"")
    });
    let b = read(p);
    assert!(b.pivots.caches[0].records.is_none());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("recordCount")));
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/_rels/cache.xml.rels", |s| {
        s.replace("</Relationships>",&format!(r#"<Relationship Id="records" Type="{REL}/pivotCacheRecords" Target="records.xml"/></Relationships>"#))
    });
    let b = read(p);
    assert!(b.pivots.caches[0].records.is_none());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("duplicate")));
}
#[test]
fn external_sources_are_described_without_fetching_or_reinterpreting_records() {
    let xml=cache().replace("<cacheSource type=\"worksheet\"><worksheetSource sheet=\"Data\" ref=\"A1:C5\"/></cacheSource>","<cacheSource type=\"external\" connectionId=\"42\"/>");
    let c = parse_cache_definition(7, "c", &xml).unwrap();
    assert_eq!(c.source.kind, "external");
    assert_eq!(c.source.connection_id, Some(42));
    assert!(c.source.sheet.is_none());
    assert!(c.unsupported.iter().any(|s| s.contains("external")));
}
#[test]
fn prefixed_owned_elements_and_foreign_owned_lookalikes_keep_hierarchy() {
    let xml = table().replace(&format!("xmlns=\"{MAIN}\""), &format!("xmlns:p=\"{MAIN}\""));
    // Prefix all element names, leaving the unqualified attributes unchanged.
    let mut prefixed = String::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find('<') {
        prefixed.push_str(&rest[..i + 1]);
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix('/') {
            prefixed.push('/');
            rest = r;
        }
        prefixed.push_str("p:");
        let Some(end) = rest.find('>') else { panic!() };
        prefixed.push_str(&rest[..=end]);
        rest = &rest[end + 1..];
    }
    prefixed.push_str(rest);
    let t = parse_pivot_table("s", "p", &prefixed).unwrap();
    assert_eq!(t.row_fields, [0]);
    let xml = table()
        .replace(
            "<pivotFields count=\"3\">",
            r#"<f:wrapper xmlns:f="urn:foreign"><pivotFields count="3">"#,
        )
        .replace("</pivotFields>", "</pivotFields></f:wrapper>");
    assert!(parse_pivot_table("s", "p", &xml).is_err());
}
#[test]
fn package_relative_absolute_and_escaping_targets_are_distinguished() {
    let mut p = fixture();
    change(&mut p, "xl/worksheets/_rels/sheet2.xml.rels", |s| {
        s.replace("../pivotTables/report.xml", "/xl/pivotTables/report.xml")
    });
    let b = read(p);
    assert_eq!(b.pivots.tables.len(), 1);
    assert!(b.pivots.issues.is_empty());
    let mut p = fixture();
    change(&mut p, "xl/pivotCache/_rels/cache.xml.rels", |s| {
        s.replace("records.xml", "../../../records.xml")
    });
    let b = read(p);
    assert!(b.pivots.caches[0].records.is_none());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("escapes package")));
}
#[test]
fn malformed_optional_metadata_is_reported_without_losing_saved_pivot_cells() {
    let mut p = fixture();
    change(&mut p, "xl/pivotTables/report.xml", |s| {
        s.replace("<location ref=\"E3:F6\"", "<location ref=\"XFE1\"")
    });
    let b = read(p);
    assert!(b.pivots.tables.is_empty());
    assert_eq!(b.pivots.caches.len(), 1);
    assert!(!b.pivots.issues.is_empty());
    assert_eq!(
        b.sheets[1].rows[3].cells[1].value,
        jade_xlsx_reader::CellValue::Number(42.)
    );
}
#[test]
fn excessive_owned_depth_and_actual_declaration_count_are_bounded() {
    let xml = cache().replace(
        "</pivotCacheDefinition>",
        &format!(
            "{}{}{}</pivotCacheDefinition>",
            "<extLst>",
            "<x>".repeat(130),
            "</x>".repeat(130) + "</extLst>"
        ),
    );
    assert!(parse_cache_definition(7, "c", &xml)
        .unwrap_err()
        .to_string()
        .contains("resource limit"));
    let mut p = fixture();
    change(&mut p, "xl/workbook.xml", |s| {
        s.replace(
            r#"<pivotCache cacheId="7" r:id="sharedCache"/>"#,
            &r#"<pivotCache cacheId="7" r:id="sharedCache"/>"#.repeat(4097),
        )
    });
    let b = read(p);
    assert!(b.pivots.caches.is_empty());
    assert!(b
        .pivots
        .issues
        .iter()
        .any(|i| i.reason.contains("too many pivot declarations")));
    assert_eq!(b.sheets.len(), 2);
}
#[test]
fn schema_numeric_boundary_whitespace_keeps_the_same_pivot_identity() {
    let mut p = fixture();
    change(&mut p, "xl/workbook.xml", |s| {
        s.replace("cacheId=\"7\"", "cacheId=\" 7 \"")
    });
    change(&mut p, "xl/pivotCache/cache.xml", |s| {
        s.replace("recordCount=\"4\"", "recordCount=\" 4 \"")
    });
    change(&mut p, "xl/pivotTables/report.xml", |s| {
        s.replace("cacheId=\"7\"", "cacheId=\" 7 \"")
            .replace("fld=\"1\"", "fld=\" 1 \"")
            .replace("numFmtId=\"164\"", "numFmtId=\" 164 \"")
    });
    change(&mut p, "xl/pivotCache/records.xml", |s| {
        s.replace("count=\"4\"", "count=\" 4 \"")
            .replace("v=\"10\"", "v=\" 10 \"")
    });
    let b = read(p);
    assert_eq!(b.pivots.caches.len(), 1, "{:?}", b.pivots.issues);
    let c = &b.pivots.caches[0];
    assert_eq!(c.id, 7);
    assert!(c.records.is_some());
    assert_eq!(
        c.record_value(1, 0),
        Some(&PivotScalar::Text("West".into()))
    );
    assert_eq!(c.record_value(0, 1), Some(&PivotScalar::Number(10.)));
    assert_eq!(b.pivots.tables.len(), 1);
    assert_eq!(
        b.pivots.tables[0].data_fields[0].number_format_id,
        Some(164)
    );
    assert!(b.pivots.issues.is_empty());
}

#[test]
fn mce_selects_one_owned_workbook_cache_declaration_branch_at_each_depth() {
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    for (part, container, leaf) in [("xl/workbook.xml", "pivotCaches", "pivotCache")] {
        for wrap_leaf in [false, true] {
            for prefer_choice in [false, true] {
                let mut p = fixture();
                change(&mut p, part, |s| {
                    let (start, end) = if wrap_leaf {
                        let start = s.find(&format!("<{leaf} ")).unwrap();
                        (start, start + s[start..].find("/>").unwrap() + 2)
                    } else {
                        let start = s.find(&format!("<{container}")).unwrap();
                        (
                            start,
                            s.find(&format!("</{container}>")).unwrap() + container.len() + 3,
                        )
                    };
                    let actual = &s[start..end];
                    let invalid = actual.replace("r:id=", "wrong=");
                    let (choice, fallback, required) = if prefer_choice {
                        (actual, invalid.as_str(), "m")
                    } else {
                        (invalid.as_str(), actual, "u")
                    };
                    format!("{}<mc:AlternateContent xmlns:mc=\"{MC}\" xmlns:m=\"{MAIN}\" xmlns:u=\"urn:unknown\"><mc:Choice Requires=\"{required}\">{choice}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>{}", &s[..start], &s[end..])
                });
                let b = read(p);
                assert!(
                    b.pivots.issues.is_empty(),
                    "{part} {wrap_leaf} {prefer_choice}: {:?}",
                    b.pivots.issues
                );
                assert_eq!(b.pivots.tables.len(), 1);
                assert_eq!(b.pivots.caches.len(), 1);
            }
        }
    }
}
#[test]
fn foreign_relationship_parents_do_not_promote_owned_pivot_links() {
    let mut p = fixture();
    change(&mut p, "xl/worksheets/_rels/sheet2.xml.rels", |s| {
        s.replace(
            "<Relationship Id=",
            "<f:wrapper xmlns:f=\"urn:foreign\"><Relationship Id=",
        )
        .replace("/></Relationships>", "/></f:wrapper></Relationships>")
    });
    let b = read(p);
    assert!(b.pivots.issues.is_empty());
    assert!(b.pivots.tables.is_empty());
}

#[test]
fn standard_relationship_ownership_needs_no_worksheet_declaration_marker() {
    let mut p = fixture();
    change(&mut p, "xl/worksheets/sheet2.xml", |s| {
        s.replace(
            "<pivotTableParts count=\"1\"><pivotTablePart r:id=\"pivot\"/></pivotTableParts>",
            "",
        )
    });
    let b = read(p);
    assert!(b.pivots.issues.is_empty());
    assert_eq!(b.pivots.tables.len(), 1);
    assert_eq!(b.pivots.tables[0].name, "RegionalSales");
}
#[test]
fn the_unmodified_normative_workbook_exposes_all_twenty_two_related_pivots() {
    let b = read_xlsx(include_bytes!(
        "../../../public/fixtures/pivot-style-ecma-2016.xlsx"
    ))
    .unwrap();
    assert_eq!(b.pivots.tables.len(), 22, "{:?}", b.pivots.issues);
}

#[test]
fn normative_report_filter_locations_match_the_actual_saved_cells() {
    let book = jade_xlsx_reader::read_xlsx(include_bytes!(
        "../../../public/fixtures/pivot-style-ecma-2016.xlsx"
    ))
    .unwrap();
    assert_eq!(book.pivots.tables.len(), 22);
    for table in &book.pivots.tables {
        assert_eq!(table.location.row_page_count, Some(1));
        assert_eq!(table.location.column_page_count, Some(1));
        let pairs = table.page_cells().unwrap();
        assert_eq!(
            pairs,
            vec![(
                jade_xlsx_reader::CellRef::parse("B2").unwrap(),
                jade_xlsx_reader::CellRef::parse("C2").unwrap()
            )]
        );
        let sheet = book
            .sheets
            .iter()
            .find(|s| s.part_name == table.sheet_part)
            .unwrap();
        for at in [pairs[0].0, pairs[0].1] {
            assert!(sheet
                .rows
                .iter()
                .flat_map(|r| &r.cells)
                .any(|c| c.reference == at && !c.value.is_empty()));
        }
    }
}
#[test]
fn wrapped_page_filters_follow_saved_orientation_and_reject_conflicting_geometry() {
    let mut book = jade_xlsx_reader::read_xlsx(include_bytes!(
        "../../../public/fixtures/pivot-style-compat.xlsx"
    ))
    .unwrap();
    let t = std::sync::Arc::make_mut(&mut book.pivots.tables[0]);
    t.location.reference = jade_xlsx_reader::CellRange::parse("B8:E20").unwrap();
    t.page_fields = vec![t.page_fields[0].clone(); 5];
    t.attributes.insert("pageWrap".into(), "2".into());
    let labels = |t: &jade_xlsx_reader::pivot::PivotTable| {
        t.page_cells()
            .unwrap()
            .into_iter()
            .map(|p| p.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(t),
        ["B5", "B6", "E5", "E6", "H5"].map(|s| jade_xlsx_reader::CellRef::parse(s).unwrap())
    );
    t.attributes.insert("pageOverThenDown".into(), "1".into());
    assert_eq!(
        labels(t),
        ["B4", "E4", "B5", "E5", "B6"].map(|s| jade_xlsx_reader::CellRef::parse(s).unwrap())
    );
    t.location.row_page_count = Some(2);
    assert!(t.page_cells().is_none());
}
