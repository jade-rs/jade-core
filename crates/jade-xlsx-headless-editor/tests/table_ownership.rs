use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{read_xlsx, Axis, CellRange};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PART: &str = "xl/worksheets/sheet1.xml";
const RELS: &str = "xl/worksheets/_rels/sheet1.xml.rels";
const TABLE: &str = "xl/tables/table1.xml";
fn package() -> OpcPackage {
    OpcPackage::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap()
}
fn book(p: &OpcPackage) -> HeadlessWorkbook {
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn xml(b: &HeadlessWorkbook, p: &str) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(p)
        .unwrap()
}
#[test]
fn unlisted_tables_do_not_coerce_headers_or_follow_grid_edits_or_formulas() {
    let mut p = package();
    let name = read_xlsx(&p.write().unwrap()).unwrap().sheets[0]
        .name
        .clone();
    p.replace(
        PART,
        format!(r#"<worksheet xmlns="{MAIN}"><sheetData/></worksheet>"#).into_bytes(),
    );
    let original = p.required_part_str(TABLE).unwrap();
    let original = original.replace(
        "<tableColumn id=\"1\" name=\"Name\"/>",
        &format!("<tableColumn id=\"1\" name=\"Name\"><calculatedColumnFormula>'{name}'!$B$3</calculatedColumnFormula></tableColumn>"),
    );
    assert!(original.contains("<calculatedColumnFormula>"));
    p.replace(TABLE, original.as_bytes().to_vec());
    let mut b = book(&p);
    assert!(b.workbook().sheets[0].tables.is_empty());
    b.set(&name, "B1", CellInput::Number(10.)).unwrap();
    assert_eq!(b.cell(&name, "B1").unwrap().number, Some(10.));
    assert_eq!(xml(&b, TABLE), original);
    let before = b.bytes().to_vec();
    b.insert_rows(&name, 1, 1).unwrap();
    assert_eq!(xml(&b, TABLE), original);
    b.shift_cells(&name, CellRange::parse("B2").unwrap(), Axis::Rows, true)
        .unwrap();
    assert_eq!(xml(&b, TABLE), original);
    let saved = b.save_bytes().unwrap();
    assert!(read_xlsx(&saved).unwrap().sheets[0].tables.is_empty());
    b.undo().unwrap();
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}
#[test]
fn only_direct_standard_table_parts_and_relationship_id_attributes_are_links() {
    let mut p = package();
    p.replace(PART,format!(r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}" xmlns:u="urn:foreign"><sheetData/><u:tableParts><tablePart r:id="table"/></u:tableParts><tableParts count="2"><u:tablePart r:id="table"/><tablePart u:id="table"/></tableParts><extLst><ext uri="opaque"><tableParts><tablePart r:id="table"/></tableParts></ext></extLst></worksheet>"#).into_bytes());
    assert!(book(&p).workbook().sheets[0].tables.is_empty());
    for attribute in [
        "xmlns:r=\"urn:foreign\" r:id=\"table\"".to_string(),
        format!("xmlns:r=\"{REL}\" r:id=\"table\""),
    ] {
        p.replace(PART,format!(r#"<worksheet xmlns="{MAIN}"><sheetData/><tableParts><tablePart {attribute}/></tableParts></worksheet>"#).into_bytes());
        assert_eq!(
            book(&p).workbook().sheets[0].tables.len(),
            usize::from(!attribute.contains("urn:foreign"))
        );
    }
}
#[test]
fn external_or_wrong_type_relationships_do_not_activate_local_table_parts() {
    for (kind, mode) in [("table", " TargetMode=\"External\""), ("hyperlink", "")] {
        let mut p = package();
        p.replace(RELS,format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="table" Type="{REL}/{kind}" Target="../tables/table1.xml"{mode}/></Relationships>"#).into_bytes());
        let mut b = book(&p);
        assert!(b.workbook().sheets[0].tables.is_empty());
        let original = xml(&b, TABLE);
        let s = b.workbook().sheets[0].name.clone();
        b.insert_rows(&s, 1, 1).unwrap();
        assert_eq!(xml(&b, TABLE), original);
    }
}
#[test]
fn strict_aliases_follow_document_order_and_duplicate_targets_are_not_double_owned() {
    let mut p = package();
    let strict = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    let sr = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let first = p.required_part_str(TABLE).unwrap();
    let first_name = read_xlsx(&p.write().unwrap()).unwrap().sheets[0].tables[0]
        .name
        .clone();
    p.replace(
        "xl/tables/other.xml",
        first.replace(&first_name, "Other").into_bytes(),
    );
    p.replace(RELS,format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="z" Type="{sr}/table" Target="../tables/other.xml"/><Relationship Id="a" Type="{sr}/table" Target="../tables/table1.xml"/><Relationship Id="dup" Type="{sr}/table" Target="../tables/./other.xml"/></Relationships>"#).into_bytes());
    p.replace(PART,format!(r#"<s:worksheet xmlns:s="{strict}" xmlns:a="{strict}" xmlns:link="{sr}"><s:sheetData/><a:tableParts count="3"><s:tablePart link:id="z"/><a:tablePart link:id="a"/><s:tablePart link:id="dup"/></a:tableParts></s:worksheet>"#).into_bytes());
    let b = book(&p);
    let tables = &b.workbook().sheets[0].tables;
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "Other");
    assert_eq!(tables[1].name, first_name);
}

#[test]
fn partial_shifts_preserve_opaque_table_formula_lookalikes_and_update_owned_templates() {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/objects-workbook.xlsx"
    ))
    .unwrap();
    p.replace(TABLE,format!(r#"<table xmlns="{MAIN}" xmlns:u="urn:foreign" id="1" name="Table1" displayName="Table1" ref="A1:C5" totalsRowCount="1"><autoFilter ref="A1:C4"/><tableColumns count="3"><tableColumn id="1" name="Item"/><u:tableColumn id="90" name="Foreign"><calculatedColumnFormula>B3</calculatedColumnFormula></u:tableColumn><tableColumn id="2" name="Qty"/><tableColumn id="3" name="Amount"><calculatedColumnFormula>B2*2</calculatedColumnFormula><u:keep><calculatedColumnFormula>B3</calculatedColumnFormula></u:keep></tableColumn></tableColumns><extLst><ext uri="opaque"><calculatedColumnFormula>B3</calculatedColumnFormula></ext></extLst></table>"#).into_bytes());
    let mut b = book(&p);
    let name = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    b.shift_cells(&name, CellRange::parse("A3:C3").unwrap(), Axis::Rows, true)
        .unwrap();
    let out = xml(&b, TABLE);
    assert!(out.contains("<u:keep><calculatedColumnFormula>B3</calculatedColumnFormula></u:keep>"));
    assert!(out.contains(
        "<ext uri=\"opaque\"><calculatedColumnFormula>B3</calculatedColumnFormula></ext>"
    ));
    assert!(out.contains("name=\"Foreign\"><calculatedColumnFormula>B3</calculatedColumnFormula>"));
    assert_eq!(
        b.workbook().sheets[0].tables[0].column_properties[2]
            .calculated_formula
            .as_deref(),
        Some("B2*2")
    );
    assert_eq!(
        b.cell(&name, "C3").unwrap().formula.as_deref(),
        Some("B3*2")
    );
    let saved = b.save_bytes().unwrap();
    assert_eq!(xml(&HeadlessWorkbook::open(&saved).unwrap(), TABLE), out);
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}

#[test]
fn duplicate_relationship_targets_are_structurally_shifted_once() {
    let mut p = package();
    let rels = p.required_part_str(RELS).unwrap();
    p.replace(RELS,rels.replace("</Relationships>",&format!(r#"<Relationship Id="alias" Type="{REL}/table" Target="../tables/table1.xml"/></Relationships>"#)).into_bytes());
    let mut b = book(&p);
    let s = b.workbook().sheets[0].name.clone();
    b.insert_rows(&s, 2, 1).unwrap();
    assert_eq!(
        b.workbook().sheets[0].tables[0].reference,
        CellRange::parse("A1:C5").unwrap()
    );
    b.undo().unwrap();
    b.shift_cells(&s, CellRange::parse("A2:C2").unwrap(), Axis::Rows, true)
        .unwrap();
    assert_eq!(
        b.workbook().sheets[0].tables[0].reference,
        CellRange::parse("A1:C5").unwrap()
    );
    b.undo().unwrap();
    b.delete_columns(&s, "A", 3).unwrap();
    let package = OpcPackage::open(b.bytes()).unwrap();
    assert!(package.part(TABLE).is_none());
    assert!(
        jade_opc::parse_relationships(&package.required_part_str(RELS).unwrap())
            .unwrap()
            .by_type_suffix("/table")
            .is_empty()
    );
}
