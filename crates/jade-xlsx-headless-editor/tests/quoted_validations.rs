use jade_opc::OpcPackage;
use jade_xlsx_calc::validation::{self, Candidate, ValidationResult};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, CellRef, CellValue};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2011/1/ac";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const PART: &str = "xl/worksheets/sheet1.xml";
fn open(body: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(PART,format!(r#"<worksheet xmlns="{MAIN}" xmlns:mc="{MC}" xmlns:ac="{AC}" xmlns:u="urn:foreign"><sheetData/><dataValidations count="1"><dataValidation type="list" sqref="B1:B5" showErrorMessage="1" u:tag="keep">{body}</dataValidation></dataValidations></worksheet>"#).into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn list(b: &HeadlessWorkbook) -> Option<Vec<CellValue>> {
    validation::list(
        b.workbook(),
        0,
        CellRef::new(1, 0),
        &b.workbook().sheets[0].data_validations[0],
        "UTC",
    )
    .unwrap()
}
fn xml(b: &HeadlessWorkbook) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
fn texts(items: &[&str]) -> Option<Vec<CellValue>> {
    Some(items.iter().map(|s| CellValue::Text((*s).into())).collect())
}
#[test]
fn supported_choice_preserves_quoted_delimiters_escaped_quotes_blanks_and_xstrings() {
    let b = open(
        r#"<mc:AlternateContent><mc:Choice Requires="ac"><ac:list>1,"2,3",4,"5,6","a""b",, tail,_x005F_x000A_,"line_x000A_break"</ac:list></mc:Choice><mc:Fallback><formula1>"wrong,fallback"</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    assert_eq!(
        list(&b),
        texts(&[
            "1",
            "2,3",
            "4",
            "5,6",
            "a\"b",
            "",
            " tail",
            "_x000A_",
            "line\nbreak"
        ])
    );
    let r = &b.workbook().sheets[0].data_validations[0];
    for (s, expected) in [
        ("2,3", ValidationResult::Valid),
        ("2", ValidationResult::Invalid),
    ] {
        assert_eq!(
            validation::validate(
                b.workbook(),
                0,
                CellRef::new(1, 0),
                r,
                &Candidate::Value(CellValue::Text(s.into())),
                "UTC"
            )
            .unwrap(),
            expected
        );
    }
}
#[test]
fn unsupported_choices_use_fallback_and_the_first_supported_choice_wins() {
    let b = open(
        r#"<mc:AlternateContent><mc:Choice Requires="u"><ac:list>wrong</ac:list></mc:Choice><mc:Choice Requires="ac u"><ac:list>wrong too</ac:list></mc:Choice><mc:Fallback><formula1>"yes,no"</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    assert_eq!(list(&b), texts(&["yes", "no"]));
    let b = open(
        r#"<mc:AlternateContent><mc:Choice Requires="ac"><ac:list>"a,b",c</ac:list></mc:Choice><mc:Choice Requires="ac"><ac:list>wrong</ac:list></mc:Choice><mc:Fallback><formula1>"wrong"</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    assert_eq!(list(&b), texts(&["a,b", "c"]));
}
#[test]
fn editing_a_selected_source_updates_its_alternatives_and_preserves_opaque_siblings() {
    let mut b = open(
        r#"<mc:AlternateContent u:meta="keep"><mc:Choice Requires="ac"><ac:list u:option="keep">"a,b",c</ac:list><u:info>opaque</u:info></mc:Choice><mc:Fallback><formula1>"a,b,c"</formula1></mc:Fallback></mc:AlternateContent><u:extra>keep</u:extra>"#,
    );
    let name = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    let mut rule = b.workbook().sheets[0].data_validations[0].clone();
    rule.formula1 = Some("\"new,old\"".into());
    rule.literal_list = None;
    b.set_data_validation(&name, Some(0), &rule).unwrap();
    assert_eq!(list(&b), texts(&["new", "old"]));
    let out = xml(&b);
    assert!(out.contains("<u:info>opaque</u:info>"));
    assert!(out.contains("<u:extra>keep</u:extra>"));
    assert!(out.contains("u:meta=\"keep\""));
    assert!(out.contains("u:tag=\"keep\""));
    let saved = b.save_bytes().unwrap();
    assert_eq!(
        list(&HeadlessWorkbook::open(&saved).unwrap()),
        texts(&["new", "old"])
    );
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(xml(&b), out);
}
#[test]
fn partial_clearing_retains_literal_candidates_and_structural_shifts_do_not_treat_them_as_references(
) {
    let mut b = open(r#"<ac:list>"B3,C4",A1,"=SUM(A1)"</ac:list>"#);
    let name = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    assert!(b
        .clear_data_validations(&name, Some(CellRange::parse("B1:B2").unwrap()))
        .unwrap());
    assert_eq!(list(&b), texts(&["B3,C4", "A1", "=SUM(A1)"]));
    b.shift_cells(
        &name,
        CellRange::parse("B4").unwrap(),
        jade_xlsx_reader::Axis::Rows,
        true,
    )
    .unwrap();
    assert_eq!(list(&b), texts(&["B3,C4", "A1", "=SUM(A1)"]));
    b.undo().unwrap();
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}
#[test]
fn namespace_aliases_and_x14_wrappers_select_the_same_literal_candidates() {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(PART,format!(r#"<s:worksheet xmlns:s="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:c="{MC}" xmlns:l="{AC}"><s:sheetData/><s:extLst><s:ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="list" showDropDown="1"><c:AlternateContent><c:Choice Requires="l"><l:list>"a,b",c</l:list></c:Choice><c:Fallback><x:formula1><m:f>"fallback"</m:f></x:formula1></c:Fallback></c:AlternateContent><m:sqref>B1:B5</m:sqref></x:dataValidation></x:dataValidations></s:ext></s:extLst></s:worksheet>"#).into_bytes());
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    assert_eq!(list(&b), texts(&["a,b", "c"]));
    let mut r = b.workbook().sheets[0].data_validations[0].clone();
    r.literal_list = Some("\"x,y\",z".into());
    let s = b.workbook().sheets[0].name.clone();
    b.set_data_validation(&s, Some(0), &r).unwrap();
    assert_eq!(list(&b), texts(&["x,y", "z"]));
    let legacy = jade_xlsx_reader::data_validations::parse_data_validations(
        &xml(&b).replace(AC, "urn:unsupported"),
    )
    .unwrap();
    assert_eq!(legacy[0].formula1.as_deref(), Some("\"x,y,z\""));
}

#[test]
fn authored_literal_sources_round_trip_with_namespaced_fallback_and_no_formula_confusion() {
    let mut b = open("");
    let s = b.workbook().sheets[0].name.clone();
    let rule = jade_xlsx_reader::DataValidation {
        ranges: vec![CellRange::parse("C1:C5").unwrap()],
        validation_type: Some("list".into()),
        literal_list: Some("\"a,b\",\"a\"\"b\",\"=SUM(A1)\"".into()),
        show_error_message: true,
        ..Default::default()
    };
    let i = b.set_data_validation(&s, None, &rule).unwrap();
    let source = &b.workbook().sheets[0].data_validations[i];
    assert_eq!(source, &rule);
    let values = validation::list(b.workbook(), 0, CellRef::new(2, 0), source, "UTC").unwrap();
    assert_eq!(values, texts(&["a,b", "a\"b", "=SUM(A1)"]));
    let out = xml(&b);
    assert!(out.contains("AlternateContent"));
    let legacy = jade_xlsx_reader::data_validations::parse_data_validations(
        &out.replace(AC, "urn:unsupported"),
    )
    .unwrap();
    assert!(legacy[i].literal_list.is_none());
    assert_eq!(
        legacy[i].formula1.as_deref(),
        Some("\"a,b,a\"\"b,=SUM(A1)\"")
    );
    let re = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(re.workbook().sheets[0].data_validations[i], rule);
}
#[test]
fn indexed_literal_edits_keep_branch_attributes_and_switch_back_to_a_formula_without_stale_list_data(
) {
    let mut b = open(
        r#"<mc:AlternateContent u:meta="keep"><mc:Choice Requires="ac"><ac:list u:option="keep">"a,b",c</ac:list><u:info>keep</u:info></mc:Choice><mc:Fallback><formula1>"a,b,c"</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    let mut r = b.workbook().sheets[0].data_validations[0].clone();
    r.literal_list = Some("\"x,y\",z".into());
    b.set_data_validation(&s, Some(0), &r).unwrap();
    assert_eq!(list(&b), texts(&["x,y", "z"]));
    assert!(xml(&b).contains("u:option=\"keep\""));
    r.literal_list = None;
    r.formula1 = Some("\"one,two\"".into());
    b.set_data_validation(&s, Some(0), &r).unwrap();
    assert_eq!(list(&b), texts(&["one", "two"]));
    assert!(xml(&b).contains("<u:info>keep</u:info>"));
}
#[test]
fn malformed_literals_are_unsupported_and_invalid_authoring_is_atomic() {
    let mut b = open(r#"<ac:list>"unclosed</ac:list>"#);
    assert_eq!(list(&b), None);
    let s = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    let mut r = b.workbook().sheets[0].data_validations[0].clone();
    r.ranges = vec![CellRange::parse("B2:B5").unwrap()];
    assert!(b.set_data_validation(&s, Some(0), &r).is_err());
    assert_eq!(b.bytes(), before);
    assert!(b
        .clear_data_validations(&s, Some(CellRange::parse("B1").unwrap()))
        .unwrap());
    assert_eq!(list(&b), None);
    assert!(xml(&b).contains("<ac:list>\"unclosed</ac:list>"));
}
#[test]
fn foreign_mce_and_unbound_requirements_do_not_override_the_selected_fallback() {
    let b = open(
        r#"<u:AlternateContent><mc:Choice Requires="ac"><ac:list>wrong</ac:list></mc:Choice></u:AlternateContent><mc:AlternateContent><mc:Choice Requires="missing"><ac:list>wrong</ac:list></mc:Choice><mc:Fallback><formula1>"right"</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    assert_eq!(list(&b), texts(&["right"]));
}

#[test]
fn x14_formula_leaf_alternatives_are_updated_without_adding_a_duplicate_direct_leaf() {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(PART,format!(r#"<worksheet xmlns="{MAIN}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:c="{MC}"><sheetData/><extLst><ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="whole" operator="lessThan"><x:formula1><c:AlternateContent><c:Choice Requires="x"><m:f>10</m:f></c:Choice><c:Fallback><m:f>5</m:f></c:Fallback></c:AlternateContent></x:formula1><m:sqref>B1</m:sqref></x:dataValidation></x:dataValidations></ext></extLst></worksheet>"#).into_bytes());
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let s = b.workbook().sheets[0].name.clone();
    let mut r = b.workbook().sheets[0].data_validations[0].clone();
    assert_eq!(r.formula1.as_deref(), Some("10"));
    r.formula1 = Some("20".into());
    b.set_data_validation(&s, Some(0), &r).unwrap();
    let out = xml(&b);
    assert_eq!(out.matches("<m:f").count() + out.matches("<f ").count(), 2);
    assert_eq!(
        b.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("20")
    );
}

#[test]
fn cell_shift_rebases_selected_and_fallback_relative_formulas_at_each_split_anchor() {
    let original = open(
        r#"<mc:AlternateContent><mc:Choice Requires="ac"><formula1>$C1</formula1></mc:Choice><mc:Fallback><formula1>$C1</formula1></mc:Fallback></mc:AlternateContent>"#,
    );
    let mut p = OpcPackage::open(original.bytes()).unwrap();
    p.replace(
        PART,
        xml(&original)
            .replace("type=\"list\"", "type=\"whole\" operator=\"lessThan\"")
            .into_bytes(),
    );
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let s = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    b.shift_cells(
        &s,
        CellRange::parse("B3").unwrap(),
        jade_xlsx_reader::Axis::Rows,
        true,
    )
    .unwrap();
    let rules = &b.workbook().sheets[0].data_validations;
    assert_eq!(
        rules
            .iter()
            .map(|r| r.formula1.as_deref())
            .collect::<Vec<_>>(),
        [Some("$C1"), Some("$C3"), Some("$C3")]
    );
    let legacy = jade_xlsx_reader::data_validations::parse_data_validations(
        &xml(&b).replace(AC, "urn:unsupported"),
    )
    .unwrap();
    assert_eq!(legacy, rules.as_slice());
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}
