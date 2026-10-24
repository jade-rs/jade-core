use jade_opc::OpcPackage;
use jade_xlsx_calc::validation::{self, Candidate, ValidationResult};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, CellRef, CellValue, DataValidation};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const PART: &str = "xl/worksheets/sheet1.xml";
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn xml(b: &HeadlessWorkbook) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
fn custom(tail: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(PART,format!(r#"<worksheet xmlns="{MAIN}" xmlns:e="{X14}" xmlns:q="{XM}" xmlns:rev="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:u="urn:foreign"><sheetData><row r="1"><c r="A1"><v>10</v></c></row><row r="2"><c r="A2"><v>20</v></c></row></sheetData>{tail}</worksheet>"#).into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn mixed() -> HeadlessWorkbook {
    custom(
        r#"<dataValidations count="1"><dataValidation type="whole" operator="lessThan" sqref="D1"><formula1>5</formula1></dataValidation></dataValidations><extLst><ext uri="dv"><e:dataValidations count="2" disablePrompts="1" u:setting="keep"><e:dataValidation type="list" showDropDown="1" showErrorMessage="1" errorTitle="選択" prompt="one_x000A_two" rev:uid="{AAAAAAAA-0000-4000-8000-000000000001}" u:tag="keep"><e:formula1><q:f>"東京,大阪"</q:f></e:formula1><q:sqref>B1:B5</q:sqref><u:formula1>foreign</u:formula1></e:dataValidation><e:dataValidation type="whole" operator="lessThan" showErrorMessage="1"><e:formula1><q:f>$A1</q:f></e:formula1><q:sqref>C1:C5</q:sqref></e:dataValidation></e:dataValidations><u:dataValidations count="1"><u:dataValidation><q:sqref>Z1</q:sqref></u:dataValidation></u:dataValidations></ext><ext uri="opaque"><u:info>keep</u:info></ext></extLst>"#,
    )
}
#[test]
fn mixed_rules_are_read_and_extended_lists_and_relative_bounds_are_evaluated() {
    let b = mixed();
    let wb = b.workbook();
    let r = &wb.sheets[0].data_validations;
    assert_eq!(r.len(), 3);
    assert_eq!(r[1].ranges, [range("B1:B5")]);
    assert_eq!(r[1].prompt.as_deref(), Some("one\ntwo"));
    assert!(!r[1].suppress_dropdown);
    assert_eq!(
        validation::list(wb, 0, CellRef::parse("B2").unwrap(), &r[1], "UTC").unwrap(),
        Some(vec![
            CellValue::Text("東京".into()),
            CellValue::Text("大阪".into())
        ])
    );
    assert_eq!(
        validation::validate(
            wb,
            0,
            CellRef::parse("C2").unwrap(),
            &r[2],
            &Candidate::Value(CellValue::Number(15.)),
            "UTC"
        )
        .unwrap(),
        ValidationResult::Valid
    );
    assert_eq!(
        validation::validate(
            wb,
            0,
            CellRef::parse("C1").unwrap(),
            &r[2],
            &Candidate::Value(CellValue::Number(15.)),
            "UTC"
        )
        .unwrap(),
        ValidationResult::Invalid
    );
}
#[test]
fn editing_and_deleting_an_extended_index_preserves_its_storage_and_unknown_metadata() {
    let mut b = mixed();
    let s = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    let mut rule = b.workbook().sheets[0]
        .data_validations
        .get(1)
        .cloned()
        .unwrap_or_default();
    rule.ranges = vec![range("B2:B6")];
    rule.formula1 = Some("\"yes,no\"".into());
    rule.validation_type = Some("list".into());
    rule.suppress_dropdown = true;
    assert_eq!(b.set_data_validation(&s, Some(1), &rule).unwrap(), 1);
    let out = xml(&b);
    assert!(
        out.contains("<e:formula1><q:f>&quot;yes,no&quot;</q:f></e:formula1>")
            || out.contains("<e:formula1><q:f>\"yes,no\"</q:f></e:formula1>")
    );
    for keep in [
        "rev:uid=\"{AAAAAAAA-0000-4000-8000-000000000001}\"",
        "u:tag=\"keep\"",
        "u:setting=\"keep\"",
        "<u:formula1>foreign</u:formula1>",
        "<u:info>keep</u:info>",
    ] {
        assert!(out.contains(keep), "{keep}: {out}");
    }
    assert!(out.contains("<q:sqref>B2:B6</q:sqref>"));
    assert!(out.contains("showDropDown=\"0\""));
    let saved = b.save_bytes().unwrap();
    let re = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(re.workbook().sheets[0].data_validations[1], rule);
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(xml(&b), out);
    b.delete_data_validation(&s, 1).unwrap();
    assert_eq!(b.workbook().sheets[0].data_validations.len(), 2);
    assert!(xml(&b).contains("count=\"1\" disablePrompts=\"1\""));
    assert!(xml(&b).contains("<u:dataValidations count=\"1\">"));
}
#[test]
fn appending_a_standard_rule_returns_its_actual_index_before_extended_rules() {
    let mut b = mixed();
    let s = b.workbook().sheets[0].name.clone();
    let r = DataValidation {
        ranges: vec![range("H1")],
        validation_type: Some("custom".into()),
        formula1: Some("H1>0".into()),
        ..Default::default()
    };
    let i = b.set_data_validation(&s, None, &r).unwrap();
    assert_eq!(i, 1);
    assert_eq!(b.workbook().sheets[0].data_validations[i], r);
    assert_eq!(b.workbook().sheets[0].data_validations.len(), 4);
    b.delete_data_validation(&s, 3).unwrap();
    assert_eq!(b.workbook().sheets[0].data_validations.len(), 3);
    assert_eq!(
        b.workbook().sheets[0].data_validations[2]
            .validation_type
            .as_deref(),
        Some("list")
    );
}
#[test]
fn clearing_and_replacing_overlaps_rebases_retained_extended_formulas() {
    let mut b = mixed();
    let s = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    assert!(b.clear_data_validations(&s, Some(range("C1:C2"))).unwrap());
    let rules = &b.workbook().sheets[0].data_validations;
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[2].ranges, [range("C3:C5")]);
    assert_eq!(rules[2].formula1.as_deref(), Some("$A3"));
    let r = DataValidation {
        ranges: vec![range("B2:C4")],
        validation_type: Some("list".into()),
        formula1: Some("\"new,old\"".into()),
        ..Default::default()
    };
    b.apply_data_validation(&s, &r).unwrap();
    assert_eq!(
        b.cell(&s, "C4")
            .unwrap()
            .data_validation
            .unwrap()
            .formula1
            .as_deref(),
        Some("\"new,old\"")
    );
    assert_eq!(
        b.cell(&s, "C5")
            .unwrap()
            .data_validation
            .unwrap()
            .formula1
            .as_deref(),
        Some("$A5")
    );
    b.clear_data_validations(&s, None).unwrap();
    assert!(b.workbook().sheets[0].data_validations.is_empty());
    assert!(xml(&b).contains("<u:info>keep</u:info>"));
    b.undo().unwrap();
    b.undo().unwrap();
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}
#[test]
fn strict_default_namespaces_multiple_containers_and_missing_ranges_keep_index_alignment() {
    let mut b = custom(&format!(
        r#"<dataValidations count="2"><dataValidation type="none"/><dataValidation sqref="D1" type="none"/></dataValidations><extLst><ext uri="one"><dataValidations xmlns="{X14}" xmlns:m="{XM}" count="2"><dataValidation type="none"/><dataValidation type="custom"><formula1><m:f><![CDATA[B1>0]]></m:f></formula1><m:sqref>B1:B2</m:sqref></dataValidation></dataValidations></ext><ext uri="two"><e:dataValidations count="1"><e:dataValidation type="whole" operator="lessThan"><e:formula1><q:f>10</q:f></e:formula1><q:sqref>C1</q:sqref></e:dataValidation></e:dataValidations></ext></extLst>"#
    ));
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(
        PART,
        xml(&b)
            .replace(MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main")
            .into_bytes(),
    );
    b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let s = b.workbook().sheets[0].name.clone();
    assert_eq!(b.workbook().sheets[0].data_validations.len(), 3);
    b.delete_data_validation(&s, 1).unwrap();
    assert_eq!(
        b.workbook().sheets[0].data_validations[1].ranges,
        [range("C1")]
    );
    b.delete_data_validation(&s, 1).unwrap();
    assert_eq!(b.workbook().sheets[0].data_validations.len(), 1);
}

#[test]
fn adding_missing_bounds_keeps_schema_order_and_creates_locally_bound_formula_leaves() {
    let mut b = custom(
        r#"<extLst><ext uri="dv"><e:dataValidations count="1"><e:dataValidation type="none"><q:sqref>B1</q:sqref><u:info>keep</u:info></e:dataValidation></e:dataValidations></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    let r = DataValidation {
        ranges: vec![range("B1")],
        validation_type: Some("whole".into()),
        operator: Some("between".into()),
        formula1: Some("1".into()),
        formula2: Some("10".into()),
        ..Default::default()
    };
    b.set_data_validation(&s, Some(0), &r).unwrap();
    let out = xml(&b);
    assert!(out.find("<e:formula1").unwrap() < out.find("<e:formula2").unwrap());
    assert!(out.find("<e:formula2").unwrap() < out.find("<q:sqref").unwrap());
    assert_eq!(b.workbook().sheets[0].data_validations[0], r);
    b.delete_data_validation(&s, 0).unwrap();
    assert!(!xml(&b).contains("<e:dataValidations"));
}
#[test]
fn foreign_extension_locations_and_formula_wrappers_do_not_shadow_real_rules() {
    let b = custom(
        r#"<extLst><ext uri="dv"><u:wrapper><e:dataValidations><e:dataValidation type="custom"><e:formula1><q:f>BAD1</q:f></e:formula1><q:sqref>Z1</q:sqref></e:dataValidation></e:dataValidations></u:wrapper><e:dataValidations count="1"><e:dataValidation type="custom" sqref="invalid ignored attribute"><u:formula1><q:f>BAD2</q:f></u:formula1><e:formula1><u:f>BAD3</u:f><q:f>B1&gt;0</q:f></e:formula1><u:sqref>Z2</u:sqref><q:sqref>B1:B2</q:sqref></e:dataValidation></e:dataValidations></ext></extLst>"#,
    );
    let rules = &b.workbook().sheets[0].data_validations;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].formula1.as_deref(), Some("B1>0"));
    assert_eq!(rules[0].ranges, [range("B1:B2")]);
}
