use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
const PART: &str = "xl/worksheets/sheet1.xml";
fn fixture(ranges: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p.required_part_str(PART).unwrap().replace(
        "</worksheet>",
        &format!(r#"<sheetProtection sheet="1"/>{ranges}</worksheet>"#),
    );
    p.replace(PART, xml.into_bytes());
    p.write().unwrap()
}
#[test]
fn passwordless_ranges_allow_locked_cells_without_removing_sheet_protection() {
    let source = fixture(
        r#"<protectedRanges><protectedRange name="Inputs" sqref="A1 A3:B4"/></protectedRanges>"#,
    );
    let mut book = HeadlessWorkbook::open(&source).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    book.set(&name, "A1", CellInput::Number(42.)).unwrap();
    assert!(book.workbook().sheets[0].protection.is_some());
    assert!(book.set(&name, "B1", CellInput::Number(7.)).is_err());
}

#[test]
fn range_authentication_changes_only_the_live_session_and_survives_cell_history() {
    let source = fixture(
        r#"<protectedRanges><protectedRange name="Inputs" sqref="A1" password="83AF"/></protectedRanges>"#,
    );
    let mut book = HeadlessWorkbook::open(&source).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    let before = book.bytes().to_vec();
    assert!(!book.can_undo());
    assert!(book
        .unlock_editable_range(&name, "Inputs", "wrong")
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    assert!(book
        .unlock_editable_range(&name, "INPUTS", "password")
        .unwrap());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    assert!(!book
        .unlock_editable_range(&name, "Inputs", "ignored after unlocking")
        .unwrap());
    book.set(&name, "A1", CellInput::Number(42.)).unwrap();
    assert!(book.undo().unwrap());
    assert!(book.editable_range_unlocked(&name, "Inputs").unwrap());
    assert!(book.can_redo());
    let undone = book.bytes().to_vec();
    assert!(book
        .unlock_editable_range(&name, "missing", "wrong")
        .is_err());
    assert_eq!(book.bytes(), undone);
    assert!(book.can_redo());
    assert!(book.redo().unwrap());
    let saved = book.save_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert!(!reopened.editable_range_unlocked(&name, "Inputs").unwrap());
    assert!(reopened.set(&name, "A1", CellInput::Number(7.)).is_err());
    assert!(book.lock_editable_ranges(&name).unwrap());
    assert!(!book.lock_editable_ranges(&name).unwrap());
    assert!(book.set(&name, "A1", CellInput::Number(7.)).is_err());
}
#[test]
fn equal_names_on_different_sheets_do_not_share_grants_and_renaming_keeps_identity() {
    let body = r#"<protectedRanges><protectedRange name="Inputs" sqref="A1" password="83AF"/></protectedRanges>"#;
    let mut p = OpcPackage::open(&fixture(body)).unwrap();
    let second = "xl/worksheets/sheet2.xml";
    let xml = p.required_part_str(second).unwrap().replace(
        "</worksheet>",
        &format!(r#"<sheetProtection sheet="1"/>{body}</worksheet>"#),
    );
    p.replace(second, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let first = book.workbook().sheets[0].name.clone();
    let second = book.workbook().sheets[1].name.clone();
    book.unlock_editable_range(&first, "Inputs", "password")
        .unwrap();
    assert!(book.set(&second, "A1", CellInput::Number(7.)).is_err());
    assert!(!book.editable_range_unlocked(&second, "Inputs").unwrap());
    book.rename_sheet(&first, "Renamed").unwrap();
    assert!(book.editable_range_unlocked("Renamed", "Inputs").unwrap());
    book.set("Renamed", "A1", CellInput::Number(42.)).unwrap();
    book.unprotect_sheet("Renamed", "").unwrap();
    book.protect_sheet(
        "Renamed",
        jade_xlsx_reader::SheetProtection::default(),
        "",
        &[0; 16],
    )
    .unwrap();
    assert!(!book.editable_range_unlocked("Renamed", "Inputs").unwrap());
}
#[test]
fn modern_x14_metadata_blocks_a_passwordless_or_legacy_fallback_record() {
    let x14 = jade_xlsx_reader::protected_ranges::EXT_URI;
    for (hash, expected) in [
        ("06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30=", "密碼🔒"),
        ("jA9zZOAXfLvFkJjipbgp+ZuI0KvdLnOkIcryShL5XGU=", "password"),
    ] {
        let body = format!(
            r#"<protectedRanges><protectedRange name="Inputs" sqref="A1"/></protectedRanges><extLst><ext uri="{x14}"><x:protectedRanges xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x:protectedRange name="inputs" algorithmName="SHA-256" saltValue="AAECAwQFBgcICQoLDA0ODw==" spinCount="3" hashValue="{hash}"><m:sqref xmlns:m="http://schemas.microsoft.com/office/excel/2006/main">A1</m:sqref></x:protectedRange></x:protectedRanges></ext></extLst>"#
        );
        let mut book = HeadlessWorkbook::open(&fixture(&body)).unwrap();
        let name = book.workbook().sheets[0].name.clone();
        assert!(book.set(&name, "A1", CellInput::Number(1.)).is_err());
        let before = book.bytes().to_vec();
        assert!(book
            .unlock_editable_range(&name, "Inputs", "wrong")
            .is_err());
        assert_eq!(book.bytes(), before);
        book.unlock_editable_range(&name, "Inputs", expected)
            .unwrap();
        book.set(&name, "A1", CellInput::Number(2.)).unwrap();
    }
    for record in [
        r#"<protectedRange name="Inputs" sqref="A1" password="83AF" hashValue="AA=="/>"#,
        r#"<protectedRange name="Inputs" sqref="A1" password="83AF" algorithmName="unknown" hashValue="AA==" saltValue="" spinCount="0"/>"#,
    ] {
        let mut book = HeadlessWorkbook::open(&fixture(&format!(
            "<protectedRanges>{record}</protectedRanges>"
        )))
        .unwrap();
        let name = book.workbook().sheets[0].name.clone();
        let before = book.bytes().to_vec();
        assert!(book
            .unlock_editable_range(&name, "Inputs", "password")
            .is_err());
        assert!(book.set(&name, "A1", CellInput::Number(1.)).is_err());
        assert_eq!(book.bytes(), before);
    }
}
#[test]
fn overlapping_different_ranges_use_any_authorized_coverage_and_batches_stay_atomic() {
    let source = fixture(
        r#"<protectedRanges><protectedRange name="Free" sqref="A1"/><protectedRange name="Private" sqref="A1:A2" password="83AF"/></protectedRanges>"#,
    );
    let mut book = HeadlessWorkbook::open(&source).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    book.set(&name, "A1", CellInput::Number(42.)).unwrap();
    assert!(book.set(&name, "A2", CellInput::Number(1.)).is_err());
    book.unlock_editable_range(&name, "Private", "password")
        .unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .set_cells(
            &name,
            &[
                ("A2".into(), CellInput::Number(7.)),
                ("B1".into(), CellInput::Number(8.))
            ]
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(book.editable_range_unlocked(&name, "Private").unwrap());
    book.set(&name, "A2", CellInput::Number(9.)).unwrap();
    book.lock_editable_ranges(&name).unwrap();
    assert!(book.set(&name, "A2", CellInput::Number(2.)).is_err());
    book.set(&name, "A1", CellInput::Number(3.)).unwrap();
}
#[test]
fn range_authoring_is_guarded_and_preserves_passwords_until_explicit_replacement() {
    let source = fixture("");
    let mut book = HeadlessWorkbook::open(&source).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    let ranges = [jade_xlsx_reader::CellRange::parse("A1:A2").unwrap()];
    assert!(book
        .set_editable_range(&name, None, "Inputs", &ranges, None)
        .is_err());
    book.unprotect_sheet(&name, "").unwrap();
    assert!(book
        .set_editable_range(&name, None, "Inputs", &ranges, Some(("secret", &[5; 16])))
        .unwrap());
    let saved = book.bytes().to_vec();
    let passwords = book.editable_ranges(&name).unwrap()[0].passwords.clone();
    assert!(!book
        .set_editable_range(&name, Some("Inputs"), "Inputs", &ranges, None)
        .unwrap());
    assert_eq!(book.bytes(), saved);
    assert!(book
        .set_editable_range(
            &name,
            Some("Inputs"),
            "Renamed",
            &[jade_xlsx_reader::CellRange::parse("A1:A3").unwrap()],
            None
        )
        .unwrap());
    assert_eq!(book.editable_ranges(&name).unwrap()[0].passwords, passwords);
    book.protect_sheet(
        &name,
        jade_xlsx_reader::SheetProtection::default(),
        "",
        &[0; 16],
    )
    .unwrap();
    book.unlock_editable_range(&name, "Renamed", "secret")
        .unwrap();
    assert!(book.delete_editable_range(&name, "Renamed").is_err());
    book.unprotect_sheet(&name, "").unwrap();
    book.set_editable_range(
        &name,
        Some("Renamed"),
        "Renamed",
        &ranges,
        Some(("", &[0; 16])),
    )
    .unwrap();
    assert!(!book.editable_ranges(&name).unwrap()[0].requires_password());
    book.delete_editable_range(&name, "Renamed").unwrap();
    assert!(book.editable_ranges(&name).unwrap().is_empty());
    book.undo().unwrap();
    assert_eq!(book.editable_ranges(&name).unwrap().len(), 1);
}
#[test]
fn malformed_ambiguous_and_opaque_metadata_do_not_grant_locked_cells() {
    for body in [
        r#"<protectedRanges><protectedRange name="Inputs" sqref="A0"/></protectedRanges>"#,
        r#"<protectedRanges><protectedRange name="Inputs" sqref="A1"/><protectedRange name="inputs" sqref="A1"/></protectedRanges>"#,
        r#"<f:protectedRanges xmlns:f="urn:f"><protectedRange name="Inputs" sqref="A1"/></f:protectedRanges>"#,
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Fallback><protectedRanges><protectedRange name="Inputs" sqref="A1"/></protectedRanges></mc:Fallback></mc:AlternateContent>"#,
    ] {
        let mut book = HeadlessWorkbook::open(&fixture(body)).unwrap();
        let name = book.workbook().sheets[0].name.clone();
        let before = book.bytes().to_vec();
        assert!(book.set(&name, "A1", CellInput::Number(1.)).is_err());
        assert!(book.unlock_editable_range(&name, "Inputs", "").is_err());
        assert_eq!(book.bytes(), before);
        assert!(!book.can_undo());
    }
}

#[test]
fn grants_follow_structural_movement_and_do_not_resurrect_after_definition_deletion() {
    let body = format!(
        r#"<protectedRanges><protectedRange name="Inputs" sqref="A2" password="83AF"/></protectedRanges><extLst><ext uri="{}"><x:protectedRanges xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x:protectedRange name="inputs" algorithmName="SHA-256" spinCount="3" saltValue="AAECAwQFBgcICQoLDA0ODw==" hashValue="jA9zZOAXfLvFkJjipbgp+ZuI0KvdLnOkIcryShL5XGU="><m:sqref xmlns:m="http://schemas.microsoft.com/office/excel/2006/main">A2</m:sqref></x:protectedRange></x:protectedRanges></ext></extLst>"#,
        jade_xlsx_reader::protected_ranges::EXT_URI
    );
    let mut p = OpcPackage::open(&fixture(&body)).unwrap();
    let xml = p.required_part_str(PART).unwrap().replace(
        "<sheetProtection sheet=\"1\"/>",
        "<sheetProtection sheet=\"1\" insertRows=\"0\" deleteRows=\"0\"/>",
    );
    p.replace(PART, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    book.unlock_editable_range(&name, "Inputs", "password")
        .unwrap();
    book.insert_rows(&name, 1, 1).unwrap();
    assert_eq!(
        book.editable_ranges(&name).unwrap()[0].ranges,
        [jade_xlsx_reader::CellRange::parse("A3").unwrap()]
    );
    assert!(book.editable_range_unlocked(&name, "Inputs").unwrap());
    assert!(book.set(&name, "A2", CellInput::Number(1.)).is_err());
    book.set(&name, "A3", CellInput::Number(42.)).unwrap();
    assert_eq!(
        book.editable_ranges(&name).unwrap()[0].ranges,
        [jade_xlsx_reader::CellRange::parse("A3").unwrap()]
    );
    book.delete_rows(&name, 3, 1).unwrap();
    assert!(
        book.editable_ranges(&name).unwrap().is_empty(),
        "remaining: {:?}",
        book.editable_ranges(&name).unwrap()
    );
    book.undo().unwrap();
    assert_eq!(book.editable_ranges(&name).unwrap().len(), 1);
    assert!(!book.editable_range_unlocked(&name, "Inputs").unwrap());
    assert!(book.set(&name, "A3", CellInput::Number(7.)).is_err());
}
