use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::SheetVisibility;
use jade_xlsx_writer::sheets;

fn fixture(protection: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let xml = package.required_part_str("xl/workbook.xml").unwrap();
    package.replace(
        "xl/workbook.xml",
        xml.replacen("<sheets>", &format!("{protection}<sheets>"), 1)
            .into_bytes(),
    );
    package.write().unwrap()
}

fn locked() -> Vec<u8> {
    fixture(r#"<workbookProtection lockStructure="1" workbookPassword="ABCD"/>"#)
}

macro_rules! refuses {
    ($name:ident, $edit:expr) => {
        #[test]
        fn $name() {
            let bytes = locked();
            let error = $edit(&bytes).expect_err("protected structure was changed");
            assert!(error
                .to_string()
                .contains("workbook structure is protected"));
        }
    };
}
refuses!(adding_a_sheet_is_refused, |b: &[u8]| sheets::add_sheet(
    b, "New", None
));
refuses!(
    deleting_a_sheet_is_refused,
    |b: &[u8]| sheets::delete_sheet(b, "Notes")
);
refuses!(
    renaming_a_sheet_is_refused,
    |b: &[u8]| sheets::rename_sheet(b, "Notes", "Other")
);
refuses!(moving_a_sheet_is_refused, |b: &[u8]| sheets::move_sheet(
    b, "Notes", 0
));
refuses!(hiding_a_sheet_is_refused, |b: &[u8]| {
    sheets::set_sheet_visibility(b, "Notes", SheetVisibility::Hidden)
});
refuses!(making_a_sheet_very_hidden_is_refused, |b: &[u8]| {
    sheets::set_sheet_visibility(b, "Notes", SheetVisibility::VeryHidden)
});

#[test]
fn unhiding_an_imported_hidden_sheet_is_refused() {
    let mut p = OpcPackage::open(&locked()).unwrap();
    let xml = p
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .replace("name=\"Notes\"", "name=\"Notes\" state=\"hidden\"");
    p.replace("xl/workbook.xml", xml.into_bytes());
    assert!(
        sheets::set_sheet_visibility(&p.write().unwrap(), "Notes", SheetVisibility::Visible)
            .is_err()
    );
}

#[test]
fn protected_noops_keep_original_bytes_and_do_not_add_history() {
    let bytes = locked();
    assert_eq!(
        sheets::rename_sheet(&bytes, "Notes", "Notes").unwrap(),
        bytes
    );
    assert_eq!(sheets::move_sheet(&bytes, "Notes", 1).unwrap(), bytes);
    assert_eq!(
        sheets::set_sheet_visibility(&bytes, "Notes", SheetVisibility::Visible).unwrap(),
        bytes
    );
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    book.rename_sheet("Notes", "Notes").unwrap();
    book.move_sheet("Notes", 1).unwrap();
    assert!(!book
        .set_sheet_visibility("Notes", SheetVisibility::Visible)
        .unwrap());
    assert!(!book.can_undo());
    assert_eq!(book.bytes(), bytes);
}

#[test]
fn windows_revision_and_password_attributes_do_not_lock_structure() {
    for attributes in [
        r#"lockStructure="0" lockWindows="1" lockRevision="1""#,
        r#"lockStructure="false" workbookPassword="ABCD""#,
        r#"workbookPassword="ABCD""#,
        "",
    ] {
        let bytes = fixture(&format!("<workbookProtection {attributes}/>"));
        let moved = sheets::move_sheet(&bytes, "Notes", 0).unwrap();
        assert_eq!(
            jade_xlsx_reader::read_xlsx(&moved).unwrap().sheets[0].name,
            "Notes"
        );
        assert!(String::from_utf8(
            OpcPackage::open(&moved)
                .unwrap()
                .part("xl/workbook.xml")
                .unwrap()
                .to_vec()
        )
        .unwrap()
        .contains(&format!("<workbookProtection {attributes}/>")));
    }
}

#[test]
fn headless_refusals_keep_cells_package_and_undo_redo_atomic() {
    let bytes = locked();
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    assert!(book.workbook().structure_protected);
    book.set("Notes", "C3", CellInput::Number(42.0)).unwrap();
    book.undo().unwrap();
    assert!(book.can_redo());
    let before = book.bytes().to_vec();
    assert!(book.add_sheet("New", None).is_err());
    assert!(book.rename_sheet("Notes", "Other").is_err());
    assert!(book.delete_sheet("Notes").is_err());
    assert!(book.move_sheet("Notes", 0).is_err());
    assert!(book
        .set_sheet_visibility("Notes", SheetVisibility::Hidden)
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    assert!(book.can_redo());
    book.redo().unwrap();
    assert_eq!(book.cell("Notes", "C3").unwrap().number, Some(42.0));
    book.set_tab_color("Notes", Some("AABBCC")).unwrap();
    book.insert_rows("Notes", 10, 1).unwrap();
    let saved = book.save_bytes().unwrap();
    let p = OpcPackage::open(&saved).unwrap();
    assert!(p
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .contains(r#"<workbookProtection lockStructure="1" workbookPassword="ABCD"/>"#));
    let mut reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert!(reopened.workbook().structure_protected);
    assert!(reopened.delete_sheet("Notes").is_err());
    assert_eq!(reopened.cell("Notes", "C3").unwrap().number, Some(42.0));
}

#[test]
fn aliases_strict_namespaces_and_modern_passwords_still_lock_structure() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let bytes = fixture(&format!(
            r#"<p:workbookProtection xmlns:p="{ns}" lockStructure="true" workbookAlgorithmName="SHA-512" workbookHashValue="opaque-hash" workbookSaltValue="opaque-salt" workbookSpinCount="100000"/>"#
        ));
        let mut p = OpcPackage::open(&bytes).unwrap();
        let xml = p.required_part_str("xl/workbook.xml").unwrap().replace(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            ns,
        );
        p.replace("xl/workbook.xml", xml.into_bytes());
        let bytes = p.write().unwrap();
        assert!(
            jade_xlsx_reader::read_xlsx(&bytes)
                .unwrap()
                .structure_protected
        );
        assert!(sheets::rename_sheet(&bytes, "Notes", "Other").is_err());
    }
}

#[test]
fn foreign_and_nested_protection_remains_opaque_during_a_real_sheet_edit() {
    for protection in [
        r#"<u:workbookProtection xmlns:u="urn:foreign" lockStructure="1"/>"#,
        r#"<extLst><ext uri="opaque"><workbookProtection lockStructure="1"/></ext></extLst>"#,
        r#"<workbookProtection xmlns:u="urn:foreign" u:lockStructure="1"/>"#,
    ] {
        let bytes = fixture(protection);
        assert!(
            !jade_xlsx_reader::read_xlsx(&bytes)
                .unwrap()
                .structure_protected
        );
        let changed = sheets::rename_sheet(&bytes, "Notes", "Other").unwrap();
        assert!(OpcPackage::open(&changed)
            .unwrap()
            .required_part_str("xl/workbook.xml")
            .unwrap()
            .contains(protection));
    }
}
