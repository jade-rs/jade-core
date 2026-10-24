use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{pivot::PivotScalar, OpcPackage};
use std::sync::Arc;
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
const PARTS: [&str; 5] = [
    "xl/pivotTables/report.xml",
    "xl/pivotCache/cache.xml",
    "xl/pivotCache/records.xml",
    "xl/pivotCache/_rels/cache.xml.rels",
    "xl/worksheets/_rels/sheet2.xml.rels",
];
fn unchanged(before: &[u8], after: &[u8]) {
    let a = OpcPackage::open(before).unwrap();
    let b = OpcPackage::open(after).unwrap();
    for part in PARTS {
        assert_eq!(a.part(part).unwrap(), b.part(part).unwrap(), "{part}");
    }
}
#[test]
fn ordinary_edits_save_reopen_and_history_keep_pivot_parts_and_typed_cache() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    assert!(b.workbook().pivots.issues.is_empty());
    b.set("Report", "H10", CellInput::Number(31.)).unwrap();
    unchanged(FIXTURE, b.bytes());
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
    let saved = b.bytes().to_vec();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().pivots.tables[0].cache_id, 7);
    assert_eq!(
        reopened.workbook().pivots.caches[0].record_value(1, 0),
        Some(&PivotScalar::Text("West".into()))
    );
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), FIXTURE);
    assert!(b.redo().unwrap());
    unchanged(&saved, b.bytes());
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(31.));
}
#[test]
fn source_cell_edits_do_not_pretend_to_refresh_saved_pivot_summaries() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    b.set("Data", "B2", CellInput::Number(999.)).unwrap();
    assert_eq!(b.cell("Data", "B2").unwrap().number, Some(999.));
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
    assert_eq!(
        b.workbook().pivots.caches[0].record_value(0, 1),
        Some(&PivotScalar::Number(10.))
    );
    unchanged(FIXTURE, b.bytes());
    assert!(b.undo().unwrap());
    assert_eq!(b.cell("Data", "B2").unwrap().number, Some(10.));
}
#[test]
fn checkpoint_restores_the_shared_pivot_payload_and_its_original_issues() {
    let mut b = HeadlessWorkbook::open(FIXTURE).unwrap();
    let original = Arc::clone(&b.workbook().pivots.caches[0]);
    let table = Arc::clone(&b.workbook().pivots.tables[0]);
    let checkpoint = b.edit_checkpoint();
    b.set("Report", "H10", CellInput::Number(31.)).unwrap();
    assert!(!Arc::ptr_eq(&original, &b.workbook().pivots.caches[0]));
    assert!(b.restore_edit_checkpoint(checkpoint));
    assert!(Arc::ptr_eq(&original, &b.workbook().pivots.caches[0]));
    assert!(Arc::ptr_eq(&table, &b.workbook().pivots.tables[0]));
    assert!(b.workbook().pivots.issues.is_empty());
    assert_eq!(b.bytes(), FIXTURE);
    assert!(!b.can_undo());
}
#[test]
fn visible_pivot_queries_and_opaque_cache_records_keep_their_saved_data() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/worksheets/sheet2.xml";
    let xml=p.required_part_str(part).unwrap().replace("<sheetData>",r#"<sheetData><row r="2"><c r="H2"><f>GETPIVOTDATA("Total Amount",E3)</f><v>42</v></c></row>"#);
    p.replace(part, xml.into_bytes());
    let part = "xl/pivotCache/cache.xml";
    let xml=p.required_part_str(part).unwrap().replace("</pivotCacheDefinition>",r#"<extLst><ext uri="vendor"><opaque xmlns="urn:vendor"/></ext></extLst></pivotCacheDefinition>"#);
    p.replace(part, xml.into_bytes());
    let bytes = p.write().unwrap();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert_eq!(b.cell("Report", "H2").unwrap().number, Some(42.));
    assert!(!b.workbook().pivots.caches[0].unsupported.is_empty());
    b.set("Report", "H10", CellInput::Number(31.)).unwrap();
    unchanged(&bytes, b.bytes());
    assert_eq!(b.cell("Report", "H2").unwrap().number, Some(42.));
}
