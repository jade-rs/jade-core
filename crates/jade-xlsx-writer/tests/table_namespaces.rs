use jade_xlsx_reader::{Axis, StructureChange};
use jade_xlsx_writer::table::{remove_table_parts, shift_table};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn change(axis: Axis, at: u32, count: u32, inserted: bool) -> StructureChange {
    StructureChange {
        sheet: "Data".into(),
        axis,
        at,
        count,
        inserted,
    }
}
fn table() -> String {
    format!(
        r#"<s:table xmlns:s="{MAIN}" xmlns:a="{MAIN}" xmlns:u="urn:foreign" name="T" displayName="T" ref="A1:C5"><s:autoFilter ref="A1:C5"><u:filterColumn colId="1"/><a:filterColumn colId="1"><s:filters><s:filter val="yes"/></s:filters></a:filterColumn><s:sortState ref="A2:C5"><u:sortCondition ref="B2:B5"/><a:sortCondition ref="B2:B5"/></s:sortState></s:autoFilter><s:tableColumns count="3"><s:tableColumn id="1" name="A"/><u:tableColumn id="100" name="foreign"><u:calculatedColumnFormula>B3</u:calculatedColumnFormula></u:tableColumn><a:tableColumn id="5" name="B"><a:calculatedColumnFormula><![CDATA[B3+1]]></a:calculatedColumnFormula><u:calculatedColumnFormula>B3</u:calculatedColumnFormula></a:tableColumn><s:tableColumn id="9" name="C"/></s:tableColumns><s:extLst><s:ext uri="opaque"><u:table ref="Z1:Z9"/><s:tableColumn id="999" name="nested"/><u:autoFilter ref="Z1:Z9"/><u:sortState ref="Z1:Z9"/></s:ext></s:extLst></s:table>"#
    )
}
#[test]
fn inserting_columns_updates_only_owned_columns_filters_and_sort_keys() {
    let original = table();
    let shifted = shift_table(&original, "Data", &change(Axis::Columns, 1, 1, true)).unwrap();
    let out = shifted.xml.unwrap();
    let parsed = jade_xlsx_reader::table::parse_table("t.xml", &out)
        .unwrap()
        .unwrap();
    assert_eq!(parsed.columns, ["A", "Column1", "B", "C"]);
    assert_eq!(
        parsed
            .column_properties
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [1, 10, 5, 9]
    );
    for keep in [
        r#"<u:filterColumn colId="1"/>"#,
        r#"<u:sortCondition ref="B2:B5"/>"#,
        r#"<u:table ref="Z1:Z9"/>"#,
        r#"<s:tableColumn id="999" name="nested"/>"#,
        r#"<u:autoFilter ref="Z1:Z9"/>"#,
        r#"<u:sortState ref="Z1:Z9"/>"#,
        "<u:calculatedColumnFormula>B3</u:calculatedColumnFormula>",
    ] {
        assert!(out.contains(keep), "{keep}: {out}");
    }
    assert_eq!(
        parsed.reference,
        jade_xlsx_reader::CellRange::parse("A1:D5").unwrap()
    );
    assert!(out.contains("<a:filterColumn colId=\"2\""));
    assert!(out.contains("<a:sortCondition ref=\"C2:C5\""));
    assert_eq!(
        parsed.column_properties[2].calculated_formula.as_deref(),
        Some("C3+1")
    );
}
#[test]
fn deleting_a_column_counts_real_children_and_keeps_foreign_and_nested_columns() {
    let out = shift_table(&table(), "Data", &change(Axis::Columns, 1, 1, false))
        .unwrap()
        .xml
        .unwrap();
    let t = jade_xlsx_reader::table::parse_table("t.xml", &out)
        .unwrap()
        .unwrap();
    assert_eq!(t.columns, ["A", "C"]);
    assert!(out.contains("<s:tableColumns count=\"2\""));
    assert!(out.contains("name=\"foreign\""));
    assert!(out.contains("name=\"nested\""));
    assert_eq!(t.filter.unwrap().columns.len(), 0);
}
#[test]
fn removing_table_links_honors_aliases_foreign_ids_and_direct_container_ownership() {
    let rel = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let original = format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:r="{rel}" xmlns:u="urn:foreign"><s:tableParts count="3"><s:tablePart r:id="remove"/><s:tablePart u:id="remove"/><u:tablePart r:id="remove"/><s:tablePart r:id="keep"/></s:tableParts><s:extLst><s:ext uri="opaque"><u:tableParts><u:tablePart r:id="remove"/></u:tableParts></s:ext></s:extLst></s:worksheet>"#
    );
    let out = remove_table_parts(&original, &["remove".into()]).unwrap();
    assert!(out.contains("<s:tableParts count=\"2\""));
    assert!(out.contains("<s:tablePart u:id=\"remove\"/>"));
    assert!(out.contains("<u:tablePart r:id=\"remove\"/>"));
    assert!(out.contains("<s:tablePart r:id=\"keep\"/>"));
    assert!(out.contains("<u:tableParts><u:tablePart r:id=\"remove\"/></u:tableParts>"));
}

#[test]
fn sparse_imported_maximum_ids_allow_insertion_without_overflow_or_collisions() {
    let source = table().replace("id=\"9\" name=\"C\"", "id=\"4294967295\" name=\"C\"");
    let out = shift_table(&source, "Data", &change(Axis::Columns, 1, 2, true))
        .unwrap()
        .xml
        .unwrap();
    let t = jade_xlsx_reader::table::parse_table("t.xml", &out)
        .unwrap()
        .unwrap();
    let ids = t
        .column_properties
        .iter()
        .map(|c| c.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 5);
    assert!(ids.contains(&4294967295));
    assert!(ids.contains(&1));
    assert!(ids.contains(&5));
}

#[test]
fn inserted_columns_do_not_reuse_a_siblings_locally_declared_prefix() {
    let source = format!(
        r#"<table xmlns="{MAIN}" ref="A1:B4" name="T" displayName="T"><tableColumns count="2"><a:tableColumn xmlns:a="{MAIN}" id="1" name="A"/><b:tableColumn xmlns:b="{MAIN}" id="2" name="B"/></tableColumns></table>"#
    );
    let out = shift_table(&source, "Data", &change(Axis::Columns, 1, 1, true))
        .unwrap()
        .xml
        .unwrap();
    let t = jade_xlsx_reader::table::parse_table("t.xml", &out)
        .unwrap()
        .unwrap();
    assert_eq!(t.columns, ["A", "Column1", "B"]);
}
