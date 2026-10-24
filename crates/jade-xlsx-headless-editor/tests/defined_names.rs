use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::DefinedName;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}

fn name(name: &str, formula: &str, scope: Option<usize>) -> DefinedName {
    DefinedName {
        name: name.into(),
        formula: formula.into(),
        local_sheet_index: scope,
        hidden: false,
    }
}

#[test]
fn imported_literal_xstring_name_is_evaluated_updated_and_removed_by_logical_name() {
    let mut package = OpcPackage::open(&fixture()).unwrap();
    let xml = package.required_part_str("xl/workbook.xml").unwrap();
    package.replace("xl/workbook.xml", xml.replace("</workbook>", "<definedNames><definedName name=\"Amount_x005F_x0041_\">7</definedName></definedNames></workbook>").into_bytes());
    let bytes = package.write().unwrap();
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "A20", CellInput::Formula("=Amount_x0041_*2".into()))
        .unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().number, Some(14.));
    book.set_defined_name(&name("Amount_x0041_", "9", None))
        .unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().number, Some(18.));
    assert_eq!(
        book.workbook()
            .defined_names
            .iter()
            .filter(|n| n.name == "Amount_x0041_")
            .count(),
        1
    );
    let bytes = book.bytes().to_vec();
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert!(book.delete_defined_name("Amount_x0041_", None).unwrap());
    let removed = book.bytes().to_vec();
    assert_eq!(book.cell(&sheet, "A20").unwrap().display, "#NAME?");
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), bytes);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), removed);
    book.set_defined_name(&name("Fresh_x0041_", "3", None))
        .unwrap();
    let package = OpcPackage::open(book.bytes()).unwrap();
    assert!(package
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .contains("Fresh_x005F_x0041_"));
}

#[test]
fn names_recalculate_in_their_scope_and_survive_save_undo_and_redo() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "A20", CellInput::Number(7.0)).unwrap();
    book.set_defined_name(&name("Amount", &format!("='{}'!$A$20", sheet), None))
        .unwrap();
    book.set(&sheet, "B20", CellInput::Formula("=Amount*2".into()))
        .unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(14.0));
    book.set_defined_name(&name("Amount", "5", Some(0)))
        .unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(10.0));
    book.set_defined_name(&name("amount", "9", Some(0)))
        .unwrap();
    assert_eq!(
        book.workbook()
            .defined_names
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("amount"))
            .count(),
        2
    );
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(18.0));
    book.undo().unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(10.0));
    book.redo().unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(18.0));
    assert!(book.delete_defined_name("AMOUNT", Some(0)).unwrap());
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(14.0));
    let bytes = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&bytes).unwrap();
    assert_eq!(reopened.cell(&sheet, "B20").unwrap().number, Some(14.0));
    assert!(!book.delete_defined_name("Missing", None).unwrap());
    assert!(book.delete_defined_name("Amount", None).unwrap());
    assert_eq!(book.cell(&sheet, "B20").unwrap().display, "#NAME?");
    book.undo().unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(14.0));
}

#[test]
fn invalid_names_and_scopes_leave_the_workbook_unchanged() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let before = book.bytes().to_vec();
    for invalid in [
        "",
        "A1",
        "XFD1048576",
        "R1C1",
        "RC",
        "R",
        "c",
        "1name",
        "bad name",
        "bad!name",
        "_xlnm.Print_Area",
    ] {
        assert!(
            book.set_defined_name(&name(invalid, "1", None))
                .unwrap_err()
                .is_invalid_input(),
            "{invalid}"
        );
    }
    assert!(book
        .set_defined_name(&name("Valid", "1", Some(100)))
        .is_err());
    assert!(book.set_defined_name(&name("Valid", "=", None)).is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    for valid in ["売上", "_Amount", "\\Amount", "A1_value", "XFE1"] {
        book.set_defined_name(&name(valid, "1", None)).unwrap();
    }
}

#[test]
fn editing_a_prefixed_name_keeps_unknown_metadata_and_other_parts() {
    let mut package = OpcPackage::open(&fixture()).unwrap();
    let workbook = package.required_part_str("xl/workbook.xml").unwrap();
    let workbook = workbook.replace("</workbook>", "<definedNames><definedName name=\"Amount\" comment=\"収益 &amp; 売上\" hidden=\"1\">1</definedName><definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">'売上サマリ'!$A$1:$D$7</definedName></definedNames></workbook>");
    // The fixture has no definedNames, and the root's namespace is now prefixed.
    let workbook = workbook
        .replace("xmlns=", "xmlns:x=")
        .replace("<workbook", "<x:workbook")
        .replace("</workbook>", "</x:workbook>");
    // Prefix the remaining SpreadsheetML tags too.
    let mut reader = quick_prefix(&workbook);
    package.replace("xl/workbook.xml", std::mem::take(&mut reader).into_bytes());
    package.replace("custom/opaque.bin", vec![1, 2, 3]);
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    book.set_defined_name(&name("Amount", "\"A & B\"", None))
        .unwrap();
    let package = OpcPackage::open(book.bytes()).unwrap();
    let xml = package.required_part_str("xl/workbook.xml").unwrap();
    assert!(xml.contains("comment=\"収益 &amp; 売上\""));
    assert!(xml.contains("<x:definedName name=\"Amount\""));
    assert!(xml.contains("_xlnm.Print_Area"));
    assert_eq!(
        package.part("custom/opaque.bin"),
        Some([1, 2, 3].as_slice())
    );
    assert_eq!(
        book.workbook()
            .defined_names
            .iter()
            .find(|n| n.name == "Amount")
            .unwrap()
            .formula,
        "\"A & B\""
    );
}

fn quick_prefix(xml: &str) -> String {
    let mut out = xml.to_string();
    for tag in [
        "bookViews",
        "workbookView",
        "sheets",
        "sheet",
        "definedNames",
        "definedName",
        "calcPr",
        "workbookPr",
    ] {
        out = out
            .replace(&format!("<{tag}"), &format!("<x:{tag}"))
            .replace(&format!("</{tag}>"), &format!("</x:{tag}>"));
    }
    out
}

#[test]
fn named_expressions_and_forward_references_recalculate_and_cycles_terminate() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set_defined_name(&name("Total", "Base*2+1", None))
        .unwrap();
    book.set_defined_name(&name("Base", "3", None)).unwrap();
    book.set(&sheet, "A20", CellInput::Formula("=Total".into()))
        .unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().number, Some(7.0));
    book.set_defined_name(&name("Base", "4", None)).unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().number, Some(9.0));
    book.set_defined_name(&name("Base", "Total", None)).unwrap();
    assert!(book.cell(&sheet, "A20").unwrap().display.starts_with('#'));
}

#[test]
fn named_error_literals_replace_previously_cached_numbers() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set_defined_name(&name("Amount", "3", None)).unwrap();
    book.set(&sheet, "A20", CellInput::Formula("=Amount".into()))
        .unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().number, Some(3.0));
    book.set_defined_name(&name("Amount", "#NAME?", None))
        .unwrap();
    assert_eq!(book.cell(&sheet, "A20").unwrap().display, "#NAME?");
}
