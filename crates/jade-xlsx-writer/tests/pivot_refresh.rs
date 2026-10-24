use jade_xlsx_reader::{read_xlsx, CellRef, CellValue, OpcPackage};
use jade_xlsx_writer::{
    pivot_refresh::{refresh_pivot, VerifiedPivotCells},
    CellEdit, NewValue, SheetEdits,
};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}
fn changed(edits: &[(&str, NewValue)]) -> Vec<u8> {
    jade_xlsx_writer::write_xlsx(
        FIXTURE,
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: edits
                .iter()
                .map(|(a, v)| CellEdit {
                    reference: at(a),
                    value: v.clone(),
                    style_index: None,
                })
                .collect(),
        }],
    )
    .unwrap()
}
fn value(bytes: &[u8], reference: &str) -> CellValue {
    read_xlsx(bytes).unwrap().sheets[1]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == at(reference))
        .map(|c| c.value.clone())
        .unwrap_or(CellValue::Empty)
}
#[test]
fn a_refresh_repairs_the_required_cache_relationship_of_a_legacy_table() {
    let result = refresh_pivot(
        FIXTURE,
        "Report",
        "RegionalSales",
        &VerifiedPivotCells::new(),
    )
    .unwrap();
    let package = OpcPackage::open(&result.bytes).unwrap();
    let xml = package
        .required_part_str("xl/pivotTables/_rels/report.xml.rels")
        .unwrap();
    let links = jade_opc::parse_relationships(&xml).unwrap();
    assert_eq!(
        links.target_by_type_suffix("/pivotCacheDefinition"),
        Some("../pivotCache/cache.xml")
    );
    assert!(read_xlsx(&result.bytes).unwrap().pivots.issues.is_empty());
}
#[test]
fn refresh_updates_real_records_visible_results_and_stable_shared_indices() {
    let bytes = changed(&[("B2", NewValue::Number(100.))]);
    assert_eq!(value(&bytes, "F4"), CellValue::Number(15.));
    let result = refresh_pivot(
        &bytes,
        "Report",
        "RegionalSales",
        &VerifiedPivotCells::new(),
    )
    .unwrap();
    assert_eq!(result.source_rows, 4);
    assert_eq!(result.tables, ["RegionalSales"]);
    assert_eq!(value(&result.bytes, "F4"), CellValue::Number(105.));
    assert_eq!(value(&result.bytes, "F5"), CellValue::Number(27.));
    assert_eq!(value(&result.bytes, "F6"), CellValue::Number(132.));
    let b = read_xlsx(&result.bytes).unwrap();
    assert_eq!(
        b.pivots.caches[0].record_value(0, 1),
        Some(&jade_xlsx_reader::pivot::PivotScalar::Number(100.))
    );
    assert_eq!(b.pivots.caches[0].fields[0].shared_items.len(), 3);
    assert!(b.pivots.caches[0].fields[0].shared_items[1].unused);
    assert_eq!(b.pivots.caches[0].attributes["refreshOnLoad"], "0");
}
#[test]
fn new_members_expand_report_without_shifting_old_cache_or_field_item_indices() {
    let bytes = changed(&[("A3", NewValue::Text("North".into()))]);
    let out = refresh_pivot(&bytes, "Report", "RegionalSales", &Default::default())
        .unwrap()
        .bytes;
    let b = read_xlsx(&out).unwrap();
    assert_eq!(
        b.pivots.caches[0].fields[0].shared_items[2].value,
        jade_xlsx_reader::pivot::PivotScalar::Text("West".into())
    );
    assert_eq!(b.pivots.tables[0].fields[0].items[1].shared_index, Some(2));
    assert_eq!(b.pivots.tables[0].fields[0].items[3].shared_index, Some(3));
    assert_eq!(value(&out, "E6"), CellValue::Text("North".into()));
    assert_eq!(value(&out, "F6"), CellValue::Number(20.));
    assert_eq!(value(&out, "F7"), CellValue::Number(42.));
}
#[test]
fn saved_formula_caches_are_not_implicitly_treated_as_current_refresh_sources() {
    let bytes = changed(&[(
        "B2",
        NewValue::FormulaCached {
            formula: "50+50".into(),
            cached: CellValue::Number(7.),
        },
    )]);
    assert!(refresh_pivot(&bytes, "Report", "RegionalSales", &Default::default()).is_err());
}
#[test]
fn retained_opaque_cache_item_attributes_and_extensions_are_not_dropped() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotCache/cache.xml";
    let xml=p.required_part_str(part).unwrap().replace("<s v=\"East\"/>","<s xmlns:f=\"urn:vendor\" f:tag=\"preserve\" v=\"East\"/>").replace("</pivotCacheDefinition>","<extLst><ext uri=\"vendor\"><f:opaque xmlns:f=\"urn:vendor\"/></ext></extLst></pivotCacheDefinition>");
    p.replace(part, xml.into_bytes());
    let out = refresh_pivot(
        &p.write().unwrap(),
        "Report",
        "RegionalSales",
        &Default::default(),
    )
    .unwrap()
    .bytes;
    let xml = OpcPackage::open(&out)
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains("f:tag=\"preserve\""));
    assert!(xml.contains("<f:opaque xmlns:f=\"urn:vendor\"/>"));
}

#[test]
fn a_sparse_old_whole_sheet_location_does_not_materialize_billions_of_empty_edits() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/report.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("E3:F6", "E3:XFD1048576");
    p.replace(part, xml.into_bytes());
    let out = refresh_pivot(
        &p.write().unwrap(),
        "Report",
        "RegionalSales",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(value(&out.bytes, "F6"), CellValue::Number(42.));
    assert_eq!(
        read_xlsx(&out.bytes).unwrap().pivots.tables[0]
            .location
            .reference,
        jade_xlsx_reader::CellRange::parse("E3:F6").unwrap()
    );
}
#[test]
fn qualified_pivot_item_metadata_and_record_extensions_survive_stable_index_refresh() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/report.xml";
    let xml = p.required_part_str(part).unwrap().replacen(
        "<item x=\"0\"/>",
        "<item xmlns:f=\"urn:vendor\" f:identity=\"keep\" x=\"0\"/>",
        1,
    );
    p.replace(part, xml.into_bytes());
    let records = "xl/pivotCache/records.xml";
    let xml=p.required_part_str(records).unwrap().replace("</pivotCacheRecords>","<extLst><ext uri=\"vendor\"><f:opaque xmlns:f=\"urn:vendor\"/></ext></extLst></pivotCacheRecords>");
    p.replace(records, xml.into_bytes());
    let out = refresh_pivot(
        &p.write().unwrap(),
        "Report",
        "RegionalSales",
        &Default::default(),
    )
    .unwrap();
    let p = OpcPackage::open(&out.bytes).unwrap();
    assert!(p
        .required_part_str(part)
        .unwrap()
        .contains("f:identity=\"keep\""));
    assert!(p
        .required_part_str(records)
        .unwrap()
        .contains("<f:opaque xmlns:f=\"urn:vendor\"/>"));
}

#[test]
fn a_foreign_extension_binding_does_not_change_the_owned_cache_namespace() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotCache/cache.xml";
    let xml=p.required_part_str(part).unwrap().replace("</pivotCacheDefinition>","<extLst><ext uri=\"vendor\"><f:opaque xmlns:f=\"http://purl.oclc.org/ooxml/spreadsheetml/main\"/></ext></extLst></pivotCacheDefinition>");
    p.replace(part, xml.into_bytes());
    p.remove("xl/pivotCache/records.xml");
    let out = refresh_pivot(
        &p.write().unwrap(),
        "Report",
        "RegionalSales",
        &Default::default(),
    )
    .unwrap();
    let b = read_xlsx(&out.bytes).unwrap();
    let part = b.pivots.caches[0].records_part.as_ref().unwrap();
    let xml = OpcPackage::open(&out.bytes)
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains("xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\""));
}
#[test]
fn prefixed_main_parts_without_a_default_namespace_keep_their_owned_children() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    for name in p.part_names().to_vec() {
        if !name.ends_with(".xml") {
            continue;
        }
        let Some(xml) = p.part_str(&name).unwrap() else {
            continue;
        };
        let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        if !xml.contains(&format!("xmlns=\"{main}\"")) {
            continue;
        }
        let mut text = String::new();
        let mut rest = xml.as_str();
        while let Some(start) = rest.find('<') {
            text.push_str(&rest[..start + 1]);
            rest = &rest[start + 1..];
            if let Some(after) = rest.strip_prefix('/') {
                text.push('/');
                rest = after;
            }
            if rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) {
                text.push_str("m:");
            }
            // Attributes are consumed with the same tag so their '<' never
            // becomes a new element prefix. This fixture has no markup CDATA.
            let end = rest.find('>').unwrap();
            text.push_str(&rest[..=end]);
            rest = &rest[end + 1..];
        }
        text.push_str(rest);
        let text = text.replace(&format!("xmlns=\"{main}\""), &format!("xmlns:m=\"{main}\""));
        p.replace(&name, text.into_bytes());
    }
    let out = refresh_pivot(
        &p.write().unwrap(),
        "Report",
        "RegionalSales",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(value(&out.bytes, "F6"), CellValue::Number(42.));
    let b = read_xlsx(&out.bytes).unwrap();
    assert!(b.pivots.issues.is_empty());
}
