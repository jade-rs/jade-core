use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, DataValidation};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}
fn rule(kind: &str, range: &str, first: &str, second: Option<&str>) -> DataValidation {
    DataValidation {
        ranges: vec![CellRange::parse(range).unwrap()],
        validation_type: Some(kind.into()),
        operator: Some("between".into()),
        allow_blank: true,
        show_error_message: true,
        formula1: Some(first.into()),
        formula2: second.map(str::to_string),
        ..Default::default()
    }
}
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture()).unwrap()
}
fn with_xml(xml: &str) -> HeadlessWorkbook {
    let mut package = OpcPackage::open(&fixture()).unwrap();
    package.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    HeadlessWorkbook::open(&package.write().unwrap()).unwrap()
}

#[test]
fn every_rule_type_and_its_messages_round_trip_and_undo() {
    let mut book = open();
    let sheet = book.workbook().sheets[0].name.clone();
    for (i, (kind, first, second)) in [
        ("whole", "1", Some("10")),
        ("decimal", "$A$1", Some("A2*2")),
        ("date", "DATE(2020,1,1)", Some("DATE(2030,1,1)")),
        ("time", "0", Some("0.5")),
        ("textLength", "2", Some("8")),
        ("list", "\"東京,大阪\"", None),
        ("custom", "COUNTIF($A$1:$A$10,A1)=1", None),
    ]
    .into_iter()
    .enumerate()
    {
        let mut r = rule(
            kind,
            &format!("B{}:B{}", i * 3 + 1, i * 3 + 2),
            first,
            second,
        );
        r.ranges
            .push(CellRange::parse(&format!("D{}", i + 1)).unwrap());
        r.error_style = Some("warning".into());
        r.ime_mode = Some("hiragana".into());
        r.suppress_dropdown = true;
        r.show_input_message = true;
        r.prompt_title = Some("入力 & 案内".into());
        r.prompt = Some("1行目\n2行目\r\t_x000A_\u{1}".into());
        r.error_title = Some("確認".into());
        r.error = Some("再入力 <必須>".into());
        assert_eq!(book.set_data_validation(&sheet, None, &r).unwrap(), i);
        assert_eq!(book.workbook().sheets[0].data_validations[i], r);
    }
    let bytes = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&bytes).unwrap();
    assert_eq!(
        reopened.workbook().sheets[0].data_validations,
        book.workbook().sheets[0].data_validations
    );
    let r = reopened.workbook().sheets[0].data_validations[0].clone();
    assert_eq!(
        book.cell(&sheet, "B1")
            .unwrap()
            .data_validation
            .unwrap()
            .error_style
            .as_deref(),
        Some("warning")
    );
    book.delete_data_validation(&sheet, 0).unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations.len(), 6);
    book.undo().unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations[0], r);
    book.redo().unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations.len(), 6);
}

#[test]
fn namespace_aliases_foreign_nodes_and_unmodeled_settings_survive() {
    let xml = format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:a="{MAIN}" xmlns:f="urn:foreign"><s:sheetData/><a:dataValidations count="1" disablePrompts="1" xWindow="500"><f:dataValidation sqref="Z1"/><s:dataValidation sqref="A1:A5" type="whole" operator="between" f:tag="keep"><f:formula1>foreign</f:formula1><a:formula1><![CDATA[1 < 2]]></a:formula1><s:formula2>10</s:formula2><f:ext>opaque</f:ext></s:dataValidation></a:dataValidations><s:hyperlinks/><s:extLst><f:dataValidation sqref="Z2"/></s:extLst></s:worksheet>"#
    );
    let mut book = with_xml(&xml);
    let sheet = book.workbook().sheets[0].name.clone();
    assert_eq!(book.workbook().sheets[0].data_validations.len(), 1);
    assert_eq!(
        book.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("1 < 2")
    );
    let mut r = book.workbook().sheets[0].data_validations[0].clone();
    r.formula1 = Some("2".into());
    book.set_data_validation(&sheet, Some(0), &r).unwrap();
    let written = OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    for keep in [
        "disablePrompts=\"1\"",
        "xWindow=\"500\"",
        "f:tag=\"keep\"",
        "<f:formula1>foreign</f:formula1>",
        "<f:ext>opaque</f:ext>",
        "<f:dataValidation sqref=\"Z1\"/>",
        "<f:dataValidation sqref=\"Z2\"/>",
    ] {
        assert!(written.contains(keep), "{keep}: {written}");
    }
    assert_eq!(
        book.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("2")
    );
    book.set_data_validation(&sheet, None, &rule("list", "C1:C5", "\"A,B\"", None))
        .unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations.len(), 2);
    book.delete_data_validation(&sheet, 0).unwrap();
    assert_eq!(
        book.workbook().sheets[0].data_validations[0]
            .validation_type
            .as_deref(),
        Some("list")
    );
}

#[test]
fn clearing_selected_cells_preserves_other_cells_and_relative_formula_meaning() {
    let mut book = open();
    let sheet = book.workbook().sheets[0].name.clone();
    let r = rule("custom", "B2:B10", "AND(B2>=$A$1,B2<C2)", None);
    book.set_data_validation(&sheet, None, &r).unwrap();
    assert!(book
        .clear_data_validations(&sheet, Some(CellRange::parse("B2:B4").unwrap()))
        .unwrap());
    let updated = &book.workbook().sheets[0].data_validations[0];
    assert_eq!(updated.ranges, vec![CellRange::parse("B5:B10").unwrap()]);
    assert_eq!(updated.formula1.as_deref(), Some("AND(B5>=$A$1,B5<C5)"));
    book.undo().unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations[0], r);
    assert!(book.clear_data_validations(&sheet, None).unwrap());
    assert!(book.workbook().sheets[0].data_validations.is_empty());
    assert!(!book.clear_data_validations(&sheet, None).unwrap());
}

#[test]
fn applying_a_new_rule_replaces_overlap_as_one_transaction() {
    let mut book = open();
    let sheet = book.workbook().sheets[0].name.clone();
    let old = rule("whole", "A1:D10", "0", Some("100"));
    book.set_data_validation(&sheet, None, &old).unwrap();
    let new = rule("list", "B3:C5", "\"yes,no\"", None);
    book.apply_data_validation(&sheet, &new).unwrap();
    assert_eq!(
        book.cell(&sheet, "B4")
            .unwrap()
            .data_validation
            .unwrap()
            .validation_type
            .as_deref(),
        Some("list")
    );
    assert_eq!(
        book.cell(&sheet, "A4")
            .unwrap()
            .data_validation
            .unwrap()
            .validation_type
            .as_deref(),
        Some("whole")
    );
    book.undo().unwrap();
    assert_eq!(book.workbook().sheets[0].data_validations, vec![old]);
}

#[test]
fn creating_rules_in_empty_and_strict_worksheets_respects_schema_order() {
    for xml in [
        format!(
            r#"<worksheet xmlns="{MAIN}"><sheetData/><pageMargins left="0.5"/><extLst/></worksheet>"#
        ),
        r#"<p:worksheet xmlns:p="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#.into(),
    ] {
        let mut book = with_xml(&xml);
        let sheet = book.workbook().sheets[0].name.clone();
        book.set_data_validation(&sheet, None, &rule("list", "A1:A10", "\"A,B\"", None))
            .unwrap();
        assert_eq!(book.workbook().sheets[0].data_validations.len(), 1);
        let xml = OpcPackage::open(book.bytes())
            .unwrap()
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        if xml.contains("pageMargins") {
            assert!(xml.find("dataValidations").unwrap() < xml.find("pageMargins").unwrap());
        }
    }
}

#[test]
fn invalid_and_protected_edits_are_atomic() {
    let mut book = open();
    let sheet = book.workbook().sheets[0].name.clone();
    let before = book.bytes().to_vec();
    for r in [
        DataValidation::default(),
        rule("bogus", "A1", "1", None),
        rule("whole", "A1", "1", None),
        DataValidation {
            error_style: Some("bad".into()),
            ..rule("list", "A1", "\"A\"", None)
        },
        rule("custom", "A1", "\u{1}", None),
    ] {
        assert!(book
            .set_data_validation(&sheet, None, &r)
            .unwrap_err()
            .is_invalid_input());
        assert_eq!(book.bytes(), before);
    }
    assert!(book.delete_data_validation(&sheet, 10).is_err());
    assert!(!book.can_undo());
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/protected-sheets-workbook.xlsx"
    ))
    .unwrap();
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .set_data_validation("Locked", None, &rule("list", "B1", "\"A\"", None))
        .is_err());
    assert_eq!(book.bytes(), before);
}
