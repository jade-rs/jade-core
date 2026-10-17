//! Refreshing a formula cell's cached `<v>` while leaving its `<f>` — and
//! everything else in the part — exactly as written.

mod common;

use common::workbook_with_sheet;
use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use jade_xlsx_writer::{write_cached_values, CachedValue, CachedValueEdit, SheetCachedValues};

const SHEET: &str = "xl/worksheets/sheet1.xml";

const FORMULA_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><v>2</v></c>
      <c r="B1"><v>3</v></c>
      <c r="C1" s="2"><f>SUM(A1:B1)</f><v>99</v></c>
    </row>
    <row r="2">
      <c r="A2" t="str"><f>IF(A1&gt;1,"big","small")</f><v>old</v></c>
      <c r="B2"><f t="shared" ref="B2:B3" si="0">A1*2</f><v>1</v></c>
      <c r="C2"><f>1+1</f></c>
    </row>
    <row r="3">
      <c r="B3"><f t="shared" si="0"/><v>2</v></c>
    </row>
  </sheetData>
</worksheet>"#;

fn update(reference: &str, value: CachedValue) -> CachedValueEdit {
    CachedValueEdit {
        reference: CellRef::parse(reference).expect("reference"),
        value,
    }
}

fn write(cells: Vec<CachedValueEdit>) -> (Vec<u8>, Vec<u8>) {
    let original = workbook_with_sheet(FORMULA_SHEET);
    let written = write_cached_values(
        &original,
        &[SheetCachedValues {
            part_name: SHEET.to_string(),
            cells,
        }],
        false,
    )
    .expect("write");
    (original, written)
}

fn sheet_xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .expect("open")
        .part_str(SHEET)
        .expect("read")
        .expect("part")
        .to_string()
}

#[test]
fn a_number_result_replaces_the_stale_value_and_keeps_the_formula() {
    let (_, written) = write(vec![update("C1", CachedValue::Number(5.0))]);
    let xml = sheet_xml(&written);
    assert!(
        xml.contains(r#"<c r="C1" s="2"><f>SUM(A1:B1)</f><v>5</v></c>"#),
        "{xml}"
    );

    let workbook = read_xlsx(&written).expect("read back");
    let sheet = &workbook.sheets[0];
    let cell = sheet.rows[0]
        .cells
        .iter()
        .find(|cell| cell.reference == CellRef::parse("C1").expect("C1"))
        .expect("C1");
    assert_eq!(cell.value, CellValue::Number(5.0));
    assert_eq!(cell.formula.as_deref(), Some("SUM(A1:B1)"));
}

#[test]
fn a_text_result_gains_the_str_type_and_a_number_result_sheds_it() {
    let (_, written) = write(vec![
        update("A2", CachedValue::Number(1.0)),
        update("C1", CachedValue::Text("五".into())),
    ]);
    let xml = sheet_xml(&written);
    // C1 had no `t`; a string result needs `t="str"`.
    assert!(xml.contains(r#"<f>SUM(A1:B1)</f><v>五</v>"#), "{xml}");
    assert!(xml.contains(r#"r="C1" s="2" t="str""#), "{xml}");
    // A2 had `t="str"`; a numeric result must not keep it.
    assert!(
        xml.contains(r#"<c r="A2"><f>IF(A1&gt;1,"big","small")</f><v>1</v></c>"#),
        "{xml}"
    );
}

#[test]
fn a_shared_formula_follower_keeps_its_si_and_master_its_ref() {
    let (_, written) = write(vec![
        update("B2", CachedValue::Number(4.0)),
        update("B3", CachedValue::Number(4.0)),
    ]);
    let xml = sheet_xml(&written);
    assert!(
        xml.contains(r#"<f t="shared" ref="B2:B3" si="0">A1*2</f><v>4</v>"#),
        "{xml}"
    );
    assert!(xml.contains(r#"<f t="shared" si="0"/><v>4</v>"#), "{xml}");
}

#[test]
fn a_formula_without_a_cached_value_gains_one() {
    let (_, written) = write(vec![update("C2", CachedValue::Number(2.0))]);
    assert!(sheet_xml(&written).contains(r#"<f>1+1</f><v>2</v>"#));
}

#[test]
fn bool_and_error_results_carry_their_types() {
    let (_, written) = write(vec![
        update("C1", CachedValue::Bool(true)),
        update("C2", CachedValue::Error("#DIV/0!".into())),
    ]);
    let xml = sheet_xml(&written);
    assert!(xml.contains(r#"t="b"><f>SUM(A1:B1)</f><v>1</v>"#), "{xml}");
    assert!(xml.contains(r#"t="e"><f>1+1</f><v>#DIV/0!</v>"#), "{xml}");
}

#[test]
fn cells_without_formulas_and_untouched_cells_do_not_move() {
    let (original, written) = write(vec![
        // A1 has no formula: refreshing a cached value must not touch it.
        update("A1", CachedValue::Number(42.0)),
        update("C1", CachedValue::Number(5.0)),
    ]);
    let xml = sheet_xml(&written);
    assert!(xml.contains(r#"<c r="A1"><v>2</v></c>"#), "{xml}");
    assert!(xml.contains(r#"<c r="B1"><v>3</v></c>"#), "{xml}");

    // Nothing outside the one sheet part changed at all.
    let before = OpcPackage::open(&original).expect("before");
    let after = OpcPackage::open(&written).expect("after");
    for name in before.part_names() {
        if name == SHEET {
            continue;
        }
        assert_eq!(before.part(name), after.part(name), "{name} changed");
    }
}

#[test]
fn refreshing_values_can_also_withdraw_the_recalculation_request() {
    let original = workbook_with_sheet(FORMULA_SHEET);
    let flagged = jade_xlsx_writer::write_xlsx(
        &original,
        &[jade_xlsx_writer::SheetEdits {
            part_name: SHEET.to_string(),
            cells: vec![jade_xlsx_writer::CellEdit {
                reference: CellRef::parse("A1").expect("A1"),
                value: jade_xlsx_writer::NewValue::Number(7.0),
                style_index: None,
            }],
        }],
    )
    .expect("write");
    assert!(String::from_utf8_lossy(
        OpcPackage::open(&flagged)
            .expect("open")
            .part("xl/workbook.xml")
            .expect("part")
    )
    .contains("fullCalcOnLoad=\"1\""));

    let refreshed = write_cached_values(
        &flagged,
        &[SheetCachedValues {
            part_name: SHEET.to_string(),
            cells: vec![update("C1", CachedValue::Number(10.0))],
        }],
        true,
    )
    .expect("refresh");
    let workbook_xml = String::from_utf8_lossy(
        OpcPackage::open(&refreshed)
            .expect("open")
            .part("xl/workbook.xml")
            .expect("part"),
    )
    .to_string();
    assert!(
        !workbook_xml.contains("fullCalcOnLoad=\"1\""),
        "{workbook_xml}"
    );
}
