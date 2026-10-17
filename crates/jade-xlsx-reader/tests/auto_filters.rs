use jade_xlsx_reader::auto_filter::parse;

#[test]
fn excessive_criterion_nesting_is_reported_without_an_unbounded_model_tree() {
    let xml=format!("<worksheet><autoFilter ref=\"A1:B4\"><filterColumn colId=\"0\">{}<filter val=\"x\"/>{}</filterColumn></autoFilter></worksheet>","<filters>".repeat(100),"</filters>".repeat(100));
    let filter = parse(&xml, b"worksheet").unwrap().0.unwrap();
    assert!(filter.issues.iter().any(|issue| issue.contains("nesting")));
    let mut node = filter.columns[0].criterion.as_ref().unwrap();
    let mut depth = 0;
    while let Some(child) = node.children.first() {
        depth += 1;
        node = child;
    }
    assert!(depth <= 64);
}

#[test]
fn filter_metadata_preserves_values_date_groups_custom_operators_and_buttons() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetPr filterMode="1"/><autoFilter ref="B3:E8"><filterColumn colId="1" hiddenButton="1" showButton="0"><filters blank="1"><filter val="A &amp; B"/><filter val="_x005F_x0041_"/><dateGroupItem year="2026" month="10" dateTimeGrouping="month"/></filters></filterColumn><filterColumn colId="2"><customFilters and="1"><customFilter operator="greaterThan" val="0.5"/><customFilter operator="lessThan" val="9"/></customFilters></filterColumn></autoFilter></worksheet>"#;
    let (filter, mode) = parse(xml, b"worksheet").unwrap();
    let filter = filter.unwrap();
    assert_eq!(mode, Some(true));
    assert_eq!(filter.reference.unwrap().start.row, 2);
    assert!(filter.issues.is_empty());
    assert!(filter.has_criteria());
    assert!(filter.columns[0].hidden_button);
    assert!(!filter.columns[0].show_button);
    let values = filter.columns[0].criterion.as_ref().unwrap();
    assert_eq!(values.kind, "filters");
    assert_eq!(values.attributes["blank"], "1");
    assert_eq!(values.children[0].attributes["val"], "A & B");
    assert_eq!(values.children[1].attributes["val"], "_x0041_");
    assert_eq!(values.children[2].kind, "dateGroupItem");
    let custom = filter.columns[1].criterion.as_ref().unwrap();
    assert_eq!(custom.children[0].attributes["operator"], "greaterThan");
    assert_eq!(custom.children[1].attributes["val"], "9");
}

#[test]
fn filter_ownership_uses_root_namespace_and_direct_children_only() {
    let xml = r#"<x:worksheet xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:q="urn:foreign"><q:sheetPr filterMode="1"/><x:sheetPr q:filterMode="1" filterMode="0"/><q:autoFilter ref="A1:C9"/><x:extLst><x:autoFilter ref="A1:C9"/></x:extLst><x:autoFilter ref="A1:C4" q:ref="A1:C9"><q:filterColumn colId="0"><x:filters/></q:filterColumn><x:filterColumn colId="1" q:hiddenButton="1"><x:filters><q:container><x:filter val="poison"/></q:container><x:filter val="20"/></x:filters></x:filterColumn></x:autoFilter></x:worksheet>"#;
    let (filter, mode) = parse(xml, b"worksheet").unwrap();
    let filter = filter.unwrap();
    assert_eq!(mode, Some(false));
    assert_eq!(filter.columns.len(), 1);
    assert!(!filter.columns[0].hidden_button);
    assert_eq!(filter.reference.unwrap().end.row, 3);
    let children = &filter.columns[0].criterion.as_ref().unwrap().children;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].attributes["val"], "20");
    assert!(parse(
        "<q:worksheet xmlns:q=\"urn:foreign\"><autoFilter ref=\"A1:B9\"/></q:worksheet>",
        b"worksheet"
    )
    .unwrap()
    .0
    .is_none());
}

#[test]
fn invalid_and_unsupported_filter_metadata_is_reported_instead_of_guessed() {
    for (xml, expected) in [
        ("<worksheet><autoFilter ref=\"BAD\"><filterColumn colId=\"0\"><filters/></filterColumn></autoFilter></worksheet>", "invalid filter range"),
        ("<worksheet><autoFilter ref=\"A1:B4\"><filterColumn colId=\"5\"><filters/></filterColumn></autoFilter></worksheet>", "filter column is outside its range"),
        ("<worksheet><autoFilter ref=\"A1:B4\"><filterColumn colId=\"0\"><unknown/></filterColumn></autoFilter></worksheet>", "unrecognized filter criterion"),
    ] {
        assert!(parse(xml,b"worksheet").unwrap().0.unwrap().issues.iter().any(|issue| issue == expected));
    }
    let table = jade_xlsx_reader::table::parse_table("xl/tables/t.xml", r#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="T" ref="A1:B4"><autoFilter ref="A1:B3"><filterColumn colId="0"><top10 val="1"/></filterColumn></autoFilter></table>"#).unwrap().unwrap();
    assert!(table.auto_filter);
    assert_eq!(
        table.filter.as_ref().unwrap().columns[0]
            .criterion
            .as_ref()
            .unwrap()
            .kind,
        "top10"
    );
}
