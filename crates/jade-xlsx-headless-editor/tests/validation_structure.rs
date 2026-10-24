use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange, DataValidation, StructureChange};

const PART: &str = "xl/worksheets/sheet1.xml";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2011/1/ac";

fn open(tail: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(PART, format!(r#"<s:worksheet xmlns:s="{MAIN}" xmlns:x="{X14}" xmlns:m="{XM}" xmlns:mc="{MC}" xmlns:ac="{AC}" xmlns:u="urn:opaque"><s:sheetData/>{tail}</s:worksheet>"#).into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn xml(b: &HeadlessWorkbook) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
fn rules(b: &HeadlessWorkbook) -> &[DataValidation] {
    &b.workbook().sheets[0].data_validations
}
fn coverage(rule: &DataValidation) -> Vec<CellRange> {
    rule.ranges.clone()
}
fn ranges(values: &[&str]) -> Vec<CellRange> {
    values
        .iter()
        .map(|s| CellRange::parse(s).unwrap())
        .collect()
}
fn saved_and_history(b: &mut HeadlessWorkbook, before: &[u8]) {
    let after = b.bytes().to_vec();
    let saved = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(rules(&saved), rules(b));
    assert_eq!(xml(&saved), xml(b));
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(b.bytes(), after);
}

#[test]
fn deleting_first_rows_reanchors_relative_bounds_but_deleted_absolute_refs_stay_errors() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="decimal" operator="between" sqref="B1:B5"><s:formula1>$C1</s:formula1><s:formula2>$D$1</s:formula2></s:dataValidation></s:dataValidations>"#,
    );
    let before = b.bytes().to_vec();
    let name = b.workbook().sheets[0].name.clone();
    b.delete_rows(&name, 1, 2).unwrap();
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B1:B3"]));
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("$C1"));
    assert_eq!(rules(&b)[0].formula2.as_deref(), Some("#REF!"));
    saved_and_history(&mut b, &before);
}

#[test]
fn removing_first_union_member_reanchors_to_the_first_surviving_source_cell() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B1:B2 D5:D7"><s:formula1>Other!$A1&gt;0</s:formula1></s:dataValidation></s:dataValidations>"#,
    );
    let before = b.bytes().to_vec();
    let name = b.workbook().sheets[0].name.clone();
    b.delete_rows(&name, 1, 2).unwrap();
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["D3:D5"]));
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("Other!$A5>0"));
    saved_and_history(&mut b, &before);
}

#[test]
fn column_deletion_reanchors_relative_columns_and_preserves_absolute_external_columns() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B2:E2"><s:formula1>Other!B$1&gt;Other!$B$1</s:formula1></s:dataValidation></s:dataValidations>"#,
    );
    let name = b.workbook().sheets[0].name.clone();
    b.delete_columns(&name, "B", 2).unwrap();
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B2:C2"]));
    assert_eq!(
        rules(&b)[0].formula1.as_deref(),
        Some("Other!D$1>Other!$B$1")
    );
}

#[test]
fn extended_alternative_formulas_shift_once_and_opaque_branches_and_literals_stay_exact() {
    let mut b = open(
        r#"<s:extLst><s:ext uri="dv"><x:dataValidations count="2" disablePrompts="1" u:meta="keep"><x:dataValidation type="custom" u:tag="keep"><x:formula1><mc:AlternateContent><mc:Choice Requires="x"><m:f>$C1&gt;0</m:f></mc:Choice><mc:Choice Requires="u"><m:f>opaque!C1</m:f></mc:Choice><mc:Fallback><m:f>$C1&gt;0</m:f></mc:Fallback></mc:AlternateContent></x:formula1><m:sqref>B1:B5</m:sqref><u:formula1><m:f>foreign!C1</m:f></u:formula1></x:dataValidation><x:dataValidation type="list"><mc:AlternateContent><mc:Choice Requires="ac"><ac:list>"B1,B2",=C1</ac:list></mc:Choice><mc:Fallback><x:formula1><m:f>"B1,B2,=C1"</m:f></x:formula1></mc:Fallback></mc:AlternateContent><m:sqref>D1:D5</m:sqref></x:dataValidation></x:dataValidations></s:ext></s:extLst>"#,
    );
    let before = b.bytes().to_vec();
    let name = b.workbook().sheets[0].name.clone();
    b.delete_rows(&name, 1, 1).unwrap();
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("$C1>0"));
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B1:B4"]));
    assert_eq!(rules(&b)[1].literal_list.as_deref(), Some("\"B1,B2\",=C1"));
    let out = xml(&b);
    assert_eq!(out.matches("<m:f>$C1&gt;0</m:f>").count(), 2);
    assert!(out.contains(r#"<mc:Choice Requires="u"><m:f>opaque!C1</m:f></mc:Choice>"#));
    assert!(out.contains("<u:formula1><m:f>foreign!C1</m:f></u:formula1>"));
    assert!(out.contains("disablePrompts=\"1\" u:meta=\"keep\""));
    saved_and_history(&mut b, &before);
}

#[test]
fn foreign_and_unmodeled_validation_containers_are_not_geometry_or_formula_sources() {
    let foreign = r#"<u:dataValidations count="7"><u:dataValidation sqref="B1:B3"><s:formula1>C1</s:formula1><m:sqref>B1:B3</m:sqref></u:dataValidation></u:dataValidations>"#;
    let nested = r#"<u:wrapper><x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>C1</m:f></x:formula1><m:sqref>B1:B3</m:sqref></x:dataValidation></x:dataValidations></u:wrapper>"#;
    let unmodeled = r#"<s:dataValidation type="custom" u:tag="no-coverage"><s:formula1>C1</s:formula1></s:dataValidation>"#;
    let mut b = open(&format!(
        r#"{foreign}<s:dataValidations count="2">{unmodeled}<s:dataValidation type="custom" sqref="B2:B3"><s:formula1>C2</s:formula1></s:dataValidation></s:dataValidations><s:extLst><s:ext uri="opaque">{nested}</s:ext></s:extLst>"#
    ));
    let name = b.workbook().sheets[0].name.clone();
    b.insert_rows(&name, 1, 1).unwrap();
    let out = xml(&b);
    for kept in [foreign, nested, unmodeled] {
        assert!(out.contains(kept), "{kept}\n{out}");
    }
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B3:B4"]));
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("C3"));
    assert!(out.contains("<s:dataValidations count=\"2\">"));
}

#[test]
fn strict_standard_alternatives_and_cdata_are_shifted_as_formula_text_only() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B3:B4"><mc:AlternateContent><mc:Choice Requires="s"><s:formula1><![CDATA[C3<10]]></s:formula1></mc:Choice><mc:Fallback><s:formula1>C3&lt;10</s:formula1></mc:Fallback></mc:AlternateContent><u:formula1>C3</u:formula1></s:dataValidation></s:dataValidations>"#,
    );
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(PART, xml(&b).replace(MAIN, STRICT).into_bytes());
    b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let name = b.workbook().sheets[0].name.clone();
    b.insert_rows(&name, 2, 2).unwrap();
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("C5<10"));
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B5:B6"]));
    assert!(xml(&b).contains("<u:formula1>C3</u:formula1>"));
    // Make only the Choice requirement unsupported, while retaining the
    // Strict worksheet namespace, to read the actual fallback separately.
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(
        PART,
        xml(&b)
            .replace("Requires=\"s\"", "Requires=\"u\"")
            .into_bytes(),
    );
    let fallback = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    assert_eq!(rules(&fallback)[0].formula1.as_deref(), Some("C5<10"));
    assert!(xml(&b).contains(STRICT));
}

#[test]
fn deleting_all_coverage_removes_records_but_retains_unmodeled_container_children() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B1"><s:formula1>C1</s:formula1></s:dataValidation></s:dataValidations><s:extLst><s:ext uri="dv"><x:dataValidations count="1" u:setting="keep"><x:dataValidation type="custom"><x:formula1><m:f>C1</m:f></x:formula1><m:sqref>B1</m:sqref></x:dataValidation><u:extra>keep</u:extra></x:dataValidations></s:ext></s:extLst>"#,
    );
    let before = b.bytes().to_vec();
    let name = b.workbook().sheets[0].name.clone();
    b.delete_rows(&name, 1, 1).unwrap();
    assert!(rules(&b).is_empty());
    let out = xml(&b);
    assert!(!out.contains("<s:dataValidations"));
    assert!(out.contains("<x:dataValidations count=\"0\" u:setting=\"keep\">"));
    assert!(out.contains("<u:extra>keep</u:extra>"));
    assert!(!out.contains("<x:dataValidation "));
    saved_and_history(&mut b, &before);
}

#[test]
fn edits_on_another_sheet_shift_only_qualified_sources_and_keep_coverage() {
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B3:B5"><mc:AlternateContent><mc:Choice Requires="s"><s:formula1>Notes!$C3&gt;E3</s:formula1></mc:Choice><mc:Fallback><s:formula1>Notes!$C3&gt;E3</s:formula1></mc:Fallback></mc:AlternateContent><u:formula1>Notes!C3</u:formula1></s:dataValidation></s:dataValidations>"#,
    );
    let before = b.bytes().to_vec();
    b.insert_rows("Notes", 2, 2).unwrap();
    assert_eq!(coverage(&rules(&b)[0]), ranges(&["B3:B5"]));
    assert_eq!(rules(&b)[0].formula1.as_deref(), Some("Notes!$C5>E3"));
    let out = xml(&b);
    assert_eq!(out.matches("Notes!$C5&gt;E3").count(), 2);
    assert!(out.contains("<u:formula1>Notes!C3</u:formula1>"));
    saved_and_history(&mut b, &before);
}

#[test]
fn insertion_at_grid_edges_clips_retained_coverage_and_removes_only_off_grid_rules() {
    for (axis, first, last, ranges_before, expected) in [
        (
            Axis::Rows,
            "1048576",
            "1048576",
            "B1048575:B1048576 D1048576",
            "B1048575:B1048576",
        ),
        (Axis::Columns, "XFD", "XFD", "XFC1:XFD1 XFD3", "XFC1:XFD1"),
    ] {
        let mut b = open(&format!(
            r#"<s:dataValidations count="1"><s:dataValidation type="whole" sqref="{ranges_before}"><s:formula1>1</s:formula1></s:dataValidation></s:dataValidations>"#
        ));
        let name = b.workbook().sheets[0].name.clone();
        match axis {
            Axis::Rows => {
                b.insert_rows(&name, first.parse().unwrap(), 1).unwrap();
            }
            Axis::Columns => {
                b.insert_columns(&name, last, 1).unwrap();
            }
        }
        assert_eq!(coverage(&rules(&b)[0]), ranges(&[expected]));
        assert_eq!(rules(&b)[0].formula1.as_deref(), Some("1"));
    }
}

#[test]
fn partial_shifts_also_preserve_unknown_formula_alternatives_exactly() {
    let unknown = r#"<mc:Choice Requires="u"><s:formula1>opaque!C1</s:formula1></mc:Choice>"#;
    let mut b = open(&format!(
        r#"<s:dataValidations count="1"><s:dataValidation type="custom" sqref="B1:B4"><mc:AlternateContent><mc:Choice Requires="s"><s:formula1>$C1&gt;0</s:formula1></mc:Choice>{unknown}<mc:Fallback><s:formula1>$C1&gt;0</s:formula1></mc:Fallback></mc:AlternateContent></s:dataValidation></s:dataValidations>"#
    ));
    let name = b.workbook().sheets[0].name.clone();
    b.shift_cells(&name, CellRange::parse("B2").unwrap(), Axis::Rows, true)
        .unwrap();
    assert_eq!(rules(&b).len(), 3);
    assert_eq!(xml(&b).matches(unknown).count(), 3);
    assert_eq!(
        rules(&b)
            .iter()
            .map(|r| r.formula1.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["$C1>0", "$C2>0", "$C2>0"]
    );
}

#[test]
fn an_unaffected_rule_keeps_original_xml_bytes_and_cdata() {
    let b = open(
        r#"<s:dataValidations count='1' u:meta='keep'><s:dataValidation sqref='B3:B4' type='custom'><s:formula1><![CDATA[C3<10]]></s:formula1><!-- keep --></s:dataValidation></s:dataValidations>"#,
    );
    let sheet = &b.workbook().sheets[0];
    let old = xml(&b);
    let moved = jade_xlsx_writer::shift_sheet(
        &old,
        sheet,
        true,
        &StructureChange {
            sheet: sheet.name.clone(),
            axis: Axis::Rows,
            at: 100,
            count: 2,
            inserted: true,
        },
    )
    .unwrap();
    assert_eq!(moved, old);
}

#[test]
fn surviving_candidates_still_use_their_original_relative_source_after_deletion() {
    use jade_xlsx_calc::validation::{validate, Candidate, ValidationResult};
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_reader::{CellRef, CellValue};
    let mut b = open(
        r#"<s:dataValidations count="1"><s:dataValidation type="whole" operator="equal" sqref="B1:B3"><s:formula1>Notes!$A1</s:formula1></s:dataValidation></s:dataValidations>"#,
    );
    b.set_cells(
        "Notes",
        &[
            ("A1".into(), CellInput::Number(10.0)),
            ("A2".into(), CellInput::Number(20.0)),
            ("A3".into(), CellInput::Number(30.0)),
        ],
    )
    .unwrap();
    let check = |b: &HeadlessWorkbook, at, value| {
        validate(
            b.workbook(),
            0,
            at,
            &rules(b)[0],
            &Candidate::Value(CellValue::Number(value)),
            "UTC",
        )
        .unwrap()
    };
    assert_eq!(check(&b, CellRef::new(1, 1), 20.0), ValidationResult::Valid);
    let name = b.workbook().sheets[0].name.clone();
    b.delete_rows(&name, 1, 1).unwrap();
    assert_eq!(check(&b, CellRef::new(1, 0), 20.0), ValidationResult::Valid);
    assert_eq!(
        check(&b, CellRef::new(1, 0), 10.0),
        ValidationResult::Invalid
    );
    assert_eq!(check(&b, CellRef::new(1, 1), 30.0), ValidationResult::Valid);
}

#[test]
fn extended_coverage_alternatives_move_without_modifying_unknown_choices() {
    let unknown = r#"<mc:Choice Requires="u"><m:sqref>Z3</m:sqref></mc:Choice>"#;
    for (primary, expected) in [("B3:B4", "B4:B5"), ("B1", "B1")] {
        let mut b = open(&format!(
            r#"<s:extLst><s:ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="whole"><x:formula1><m:f>1</m:f></x:formula1><mc:AlternateContent><mc:Choice Requires="x"><m:sqref>{primary}</m:sqref></mc:Choice>{unknown}<mc:Fallback><m:sqref>C3:C4</m:sqref></mc:Fallback></mc:AlternateContent></x:dataValidation></x:dataValidations></s:ext></s:extLst>"#
        ));
        let name = b.workbook().sheets[0].name.clone();
        b.insert_rows(&name, 2, 1).unwrap();
        assert_eq!(coverage(&rules(&b)[0]), ranges(&[expected]));
        let out = xml(&b);
        assert!(out.contains("<m:sqref>C4:C5</m:sqref>"));
        assert!(out.contains(unknown));
        let mut p = OpcPackage::open(b.bytes()).unwrap();
        p.replace(
            PART,
            out.replace("Requires=\"x\"", "Requires=\"u\"").into_bytes(),
        );
        let fallback = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
        assert_eq!(coverage(&rules(&fallback)[0]), ranges(&["C4:C5"]));
    }
}
