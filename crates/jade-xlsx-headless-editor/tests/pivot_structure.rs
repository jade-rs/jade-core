use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange, OpcPackage};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn opened() -> HeadlessWorkbook {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    b.set(
        "Report",
        "H10",
        CellInput::Formula(r#"GETPIVOTDATA("Amount",E3,"Region","West")"#.into()),
    )
    .unwrap();
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
    b
}
fn records(bytes: &[u8]) -> Vec<u8> {
    OpcPackage::open(bytes)
        .unwrap()
        .part("xl/pivotCache/records.xml")
        .unwrap()
        .to_vec()
}
#[test]
fn whole_report_follows_axis_insertions_deletions_reopen_and_history() {
    let mut b = opened();
    let before = b.bytes().to_vec();
    b.insert_rows("Report", 2, 2).unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E5:F8")
    );
    assert_eq!(b.cell("Report", "H12").unwrap().number, Some(27.));
    b.insert_columns("Report", "B", 1).unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("F5:G8")
    );
    assert_eq!(b.cell("Report", "I12").unwrap().number, Some(27.));
    assert!(b.undo().unwrap());
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E5:F8")
    );
    assert!(b.redo().unwrap());
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("F5:G8")
    );
    b.delete_rows("Report", 2, 2).unwrap();
    b.delete_columns("Report", "B", 1).unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E3:F6")
    );
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
    let reopened = HeadlessWorkbook::open(b.bytes()).unwrap();
    assert_eq!(reopened.cell("Report", "H10").unwrap().number, Some(27.));
    assert_eq!(records(&before), records(b.bytes()));
}
#[test]
fn report_moves_as_a_whole_inside_a_cell_strip_and_references_follow() {
    let mut b = opened();
    b.shift_cells("Report", range("E2:F3"), Axis::Rows, true)
        .unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E5:F8")
    );
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
    b.shift_cells("Report", range("E2:F3"), Axis::Rows, false)
        .unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E3:F6")
    );
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
}
#[test]
fn source_ranges_follow_axes_and_request_refresh_without_inventing_new_cache_records() {
    let mut b = opened();
    b.insert_rows("Data", 3, 1).unwrap();
    assert_eq!(
        b.workbook().pivots.caches[0].source.reference,
        Some(range("A1:C6"))
    );
    assert_eq!(
        b.workbook().pivots.caches[0]
            .attributes
            .get("refreshOnLoad")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
    b.insert_columns("Data", "A", 1).unwrap();
    assert_eq!(
        b.workbook().pivots.caches[0].source.reference,
        Some(range("B1:D6"))
    );
    b.delete_rows("Data", 3, 1).unwrap();
    assert_eq!(
        b.workbook().pivots.caches[0].source.reference,
        Some(range("B1:D5"))
    );
    assert_eq!(records(FIXTURE), records(b.bytes()));
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
}
#[test]
fn destructive_or_partial_report_and_source_changes_fail_atomically() {
    for attempt in 0..6 {
        let mut b = opened();
        let before = b.bytes().to_vec();
        let result = match attempt {
            0 => b.insert_rows("Report", 5, 1).map(|_| ()),
            1 => b.delete_rows("Report", 4, 1).map(|_| ()),
            2 => b.insert_columns("Report", "F", 1).map(|_| ()),
            3 => b.shift_cells("Report", range("F2:F3"), Axis::Rows, true),
            4 => b.shift_cells("Data", range("B2:B2"), Axis::Rows, true),
            _ => b.delete_rows("Data", 1, 5).map(|_| ()),
        };
        let error = result.unwrap_err();
        assert!(error.to_string().contains("pivot"), "{attempt}: {error}");
        assert_eq!(b.bytes(), before, "{attempt}");
        assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
        assert!(b.undo().unwrap()); // Still the original formula edit.
        assert_eq!(b.bytes(), FIXTURE);
    }
}
#[test]
fn renaming_a_pivot_source_edits_only_its_owned_worksheet_source() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotCache/cache.xml";
    let xml = p.required_part_str(part).unwrap().replace("</pivotCacheDefinition>",
        r#"<extLst><ext uri="vendor"><f:wrapper xmlns:f="urn:foreign"><worksheetSource sheet="Data" ref="Z1:Z9"/></f:wrapper></ext></extLst></pivotCacheDefinition>"#);
    p.replace(part, xml.into_bytes());
    let bytes = p.write().unwrap();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.rename_sheet("Data", "New source").unwrap();
    assert_eq!(
        b.workbook().pivots.caches[0].source.sheet.as_deref(),
        Some("New source")
    );
    let xml = OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains(r#"<worksheetSource sheet="Data" ref="Z1:Z9"/>"#));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), bytes);
}

#[test]
fn direct_report_value_edits_fail_but_formatting_and_copying_values_out_are_available() {
    let mut b = opened();
    let before = b.bytes().to_vec();
    assert!(b.set("Report", "F4", CellInput::Number(700.)).is_err());
    assert_eq!(b.bytes(), before);
    b.set_format(
        "Report",
        "F4",
        &jade_xlsx_writer::FormatChange {
            bold: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(b.cell("Report", "F4").unwrap().number, Some(15.));
    let snapshot = b.copy_snapshot("Report", "E4:F5", false).unwrap();
    b.paste_snapshot(&snapshot, "Report", "K1", false).unwrap();
    assert_eq!(b.cell("Report", "L1").unwrap().number, Some(15.));
    assert_eq!(b.cell("Report", "L2").unwrap().number, Some(27.));
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(27.));
}
#[test]
fn pasting_into_a_report_or_cutting_a_report_or_source_fails_atomically() {
    for (sheet, range, cut, target) in [
        ("Report", "F4:F5", false, "F5"),
        ("Report", "E3:F6", true, "J3"),
        ("Data", "A1:C5", true, "J3"),
    ] {
        let mut b = opened();
        let before = b.bytes().to_vec();
        let snapshot = b.copy_snapshot(sheet, range, cut).unwrap();
        let error = b
            .paste_snapshot(&snapshot, "Report", target, false)
            .unwrap_err();
        assert!(
            error.to_string().contains("pivot"),
            "{sheet} {range}: {error}"
        );
        assert_eq!(b.bytes(), before);
    }
}
#[test]
fn namespace_aliases_and_foreign_location_lookalikes_keep_their_identity_when_moving() {
    let mut package = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/report.xml";
    let xml = package.required_part_str(part).unwrap()
        .replace("<pivotTableDefinition ", "<p:pivotTableDefinition xmlns:p=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ")
        .replace("</pivotTableDefinition>", r#"<extLst><ext uri="vendor"><f:wrapper xmlns:f="urn:foreign"><location ref="A1:Z99"/></f:wrapper></ext></extLst></p:pivotTableDefinition>"#)
        .replace("<location ", "<p:location ");
    package.replace(part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    b.insert_rows("Report", 2, 1).unwrap();
    assert_eq!(
        b.workbook().pivots.tables[0].location.reference,
        range("E4:F7")
    );
    let xml = OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains(r#"<p:location ref="A1:Z99"/>"#));
}

#[test]
fn uninterpreted_cache_sources_block_renames_and_structural_edits_without_changing_history() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotCache/cache.xml";
    let xml = p.required_part_str(part).unwrap()
        .replace("<cacheSource", "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Fallback><cacheSource")
        .replace("</cacheSource>", "</cacheSource></mc:Fallback></mc:AlternateContent>");
    p.replace(part, xml.into_bytes());
    let bytes = p.write().unwrap();
    for operation in 0..3 {
        let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        assert!(b.workbook().pivots.caches.is_empty());
        let error = match operation {
            0 => b.insert_rows("Data", 2, 1).map(|_| ()),
            1 => b.rename_sheet("Data", "New source"),
            _ => {
                let snapshot = b.copy_snapshot("Data", "A1:C5", true).unwrap();
                b.paste_snapshot(&snapshot, "Report", "J3", false)
                    .map(|_| ())
            }
        }
        .unwrap_err();
        assert!(error.to_string().contains("pivot"), "{error}");
        assert_eq!(b.bytes(), bytes);
        assert!(!b.can_undo());
    }
}

#[test]
fn an_owned_pivot_without_a_workbook_cache_binding_retains_source_uncertainty() {
    let mut p = jade_xlsx_reader::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/pivot-style-compat.xlsx"
    ))
    .unwrap();
    let xml = p.required_part_str("xl/workbook.xml").unwrap().replace(
        "<pivotCaches><pivotCache cacheId=\"7\" r:id=\"sharedCache\"/></pivotCaches>",
        "",
    );
    p.replace("xl/workbook.xml", xml.into_bytes());
    let mut b =
        jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    assert_eq!(b.workbook().pivots.tables.len(), 1);
    let before = b.bytes().to_vec();
    assert!(b.rename_sheet("Data", "Source").is_err());
    assert_eq!(b.bytes(), before);
}
