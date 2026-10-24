use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_writer::{protection::*, WriteError};

fn fixture(record: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .replace("<sheets>", &format!("{record}<sheets>"));
    p.replace("xl/workbook.xml", xml.into_bytes());
    p.write().unwrap()
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("xl/workbook.xml")
        .unwrap()
}

#[test]
fn authoring_strong_protection_preserves_other_parts_and_schema_order() {
    let original = fixture("");
    let protected = protect_workbook(&original, "秘密🔒", &[5; 16]).unwrap();
    let source = xml(&protected);
    assert!(source.contains("workbookAlgorithmName=\"SHA-512\""));
    assert!(source.contains("workbookSpinCount=\"100000\""));
    assert!(!source.contains("秘密"));
    assert!(source.find("workbookProtection").unwrap() < source.find("<sheets").unwrap());
    assert_eq!(
        workbook_protection(&protected).unwrap().unwrap(),
        WorkbookProtectionInfo {
            structure: true,
            windows: false,
            has_password: true
        }
    );
    let a = OpcPackage::open(&original).unwrap();
    let b = OpcPackage::open(&protected).unwrap();
    assert_eq!(a.part_names(), b.part_names());
    for name in a.part_names() {
        if name != "xl/workbook.xml" {
            assert_eq!(a.part(name), b.part(name), "{name}");
        }
    }
    assert!(matches!(
        unprotect_workbook(&protected, "wrong"),
        Err(WriteError::IncorrectPassword)
    ));
    let open = unprotect_workbook(&protected, "秘密🔒").unwrap();
    assert!(!workbook_protection(&open).unwrap().unwrap().configured());
    assert!(
        !jade_xlsx_reader::read_xlsx(&open)
            .unwrap()
            .structure_protected
    );
}
#[test]
fn passwordless_protection_and_noop_unprotect_do_not_invent_passwords() {
    let original = fixture("");
    assert_eq!(unprotect_workbook(&original, "").unwrap(), original);
    let protected = protect_workbook(&original, "", &[0; 16]).unwrap();
    assert!(
        !workbook_protection(&protected)
            .unwrap()
            .unwrap()
            .has_password
    );
    let open = unprotect_workbook(&protected, "").unwrap();
    assert_eq!(unprotect_workbook(&open, "").unwrap(), open);
}
#[test]
fn existing_credentials_cannot_be_replaced_without_authentication() {
    let original = fixture(r#"<workbookProtection workbookPassword="83AF"/>"#);
    assert!(protect_workbook(&original, "new", &[0; 16]).is_err());
    assert!(unprotect_workbook(&original, "wrong").is_err());
    let open = unprotect_workbook(&original, "password").unwrap();
    assert!(protect_workbook(&open, "new", &[0; 16]).is_ok());
}
#[test]
fn legacy_and_independent_sha256_imports_authenticate_window_protection() {
    for record in [
        r#"<workbookProtection lockWindows="1" workbookPassword="83AF"/>"#,
        r#"<workbookProtection lockStructure="1" workbookAlgorithmName="SHA-256" workbookSpinCount="3" workbookSaltValue="AAECAwQFBgcICQoLDA0ODw==" workbookHashValue="06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30="/>"#,
    ] {
        let original = fixture(record);
        let password = if record.contains("SHA-256") {
            "密碼🔒"
        } else {
            "password"
        };
        assert!(workbook_protection(&original)
            .unwrap()
            .unwrap()
            .configured());
        assert!(unprotect_workbook(&original, "wrong").is_err());
        assert!(
            !workbook_protection(&unprotect_workbook(&original, password).unwrap())
                .unwrap()
                .unwrap()
                .configured()
        );
    }
}
#[test]
fn revision_protection_extensions_aliases_and_strict_namespaces_survive() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let record = format!(
            r#"<p:workbookProtection xmlns:p="{ns}" xmlns:u="urn:foreign" lockRevision="1" revisionsPassword="1234" u:workbookPassword="opaque" u:mode="keep"><u:payload/></p:workbookProtection>"#
        );
        let mut p = OpcPackage::open(&fixture(&record)).unwrap();
        let source = p.required_part_str("xl/workbook.xml").unwrap().replace(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            ns,
        );
        p.replace("xl/workbook.xml", source.into_bytes());
        let protected = protect_workbook(&p.write().unwrap(), "", &[0; 16]).unwrap();
        let open = unprotect_workbook(&protected, "").unwrap();
        let out = xml(&open);
        for keep in [
            "lockRevision=\"1\"",
            "revisionsPassword=\"1234\"",
            "u:workbookPassword=\"opaque\"",
            "u:mode=\"keep\"",
            "<u:payload/>",
            "<p:workbookProtection",
        ] {
            assert!(out.contains(keep), "{out}");
        }
        assert!(
            !jade_xlsx_reader::read_xlsx(&open)
                .unwrap()
                .structure_protected
        );
    }
}
#[test]
fn malformed_unsupported_and_duplicate_records_never_authenticate_by_absence() {
    for record in [
        r#"<workbookProtection lockStructure="1" workbookHashValue="AA=="/>"#,
        r#"<workbookProtection lockStructure="1" workbookPassword="83AF" workbookAlgorithmName="unknown" workbookHashValue="AA==" workbookSaltValue="" workbookSpinCount="0"/>"#,
        r#"<workbookProtection lockStructure="1"/><workbookProtection lockStructure="0"/>"#,
    ] {
        let bytes = fixture(record);
        assert!(unprotect_workbook(&bytes, "password").is_err());
        assert!(protect_workbook(&bytes, "", &[0; 16]).is_err());
    }
}
#[test]
fn foreign_records_stay_exact_and_a_foreign_workbook_root_is_rejected() {
    let record = r#"<u:workbookProtection xmlns:u="urn:foreign" lockStructure="1" workbookPassword="unmodeled"/>"#;
    let protected = protect_workbook(&fixture(record), "", &[0; 16]).unwrap();
    assert!(xml(&unprotect_workbook(&protected, "").unwrap()).contains(record));
    let mut p = OpcPackage::open(&fixture("")).unwrap();
    let source = p.required_part_str("xl/workbook.xml").unwrap().replace(
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "urn:foreign",
    );
    p.replace("xl/workbook.xml", source.into_bytes());
    assert!(protect_workbook(&p.write().unwrap(), "", &[0; 16]).is_err());
}
#[test]
fn relocated_workbook_parts_are_changed_without_creating_a_conventional_duplicate() {
    let mut p = OpcPackage::open(&fixture("")).unwrap();
    for (old, new) in [
        ("xl/workbook.xml", "xl/relocated.xml"),
        ("xl/_rels/workbook.xml.rels", "xl/_rels/relocated.xml.rels"),
    ] {
        let bytes = p.part(old).unwrap().to_vec();
        p.remove(old);
        p.replace(new, bytes);
    }
    for name in ["_rels/.rels", "[Content_Types].xml"] {
        let source = p
            .required_part_str(name)
            .unwrap()
            .replace("xl/workbook.xml", "xl/relocated.xml");
        p.replace(name, source.into_bytes());
    }
    let protected = protect_workbook(&p.write().unwrap(), "", &[0; 16]).unwrap();
    let package = OpcPackage::open(&protected).unwrap();
    assert!(package.part("xl/workbook.xml").is_none());
    assert!(
        jade_xlsx_reader::read_xlsx(&protected)
            .unwrap()
            .structure_protected
    );
    assert!(
        !jade_xlsx_reader::read_xlsx(&unprotect_workbook(&protected, "").unwrap())
            .unwrap()
            .structure_protected
    );
}
#[test]
fn headless_authentication_history_and_saved_guard_follow_the_metadata() {
    let mut book = HeadlessWorkbook::open(&fixture("")).unwrap();
    book.set("Notes", "C3", CellInput::Number(7.)).unwrap();
    assert!(book.protect_workbook("secret", &[7; 16]).unwrap());
    assert!(book.rename_sheet("Notes", "Other").is_err());
    let before = book.bytes().to_vec();
    assert!(book.unprotect_workbook("wrong").is_err());
    assert_eq!(book.bytes(), before);
    book.undo().unwrap();
    assert!(!book.workbook().structure_protected);
    book.redo().unwrap();
    assert!(book.workbook().structure_protected);
    assert!(book.unprotect_workbook("secret").unwrap());
    book.rename_sheet("Notes", "Other").unwrap();
    assert_eq!(book.cell("Other", "C3").unwrap().number, Some(7.));
    book.undo().unwrap();
    book.undo().unwrap();
    assert!(book.workbook().structure_protected);
    let mut reopened = HeadlessWorkbook::open(&book.save_bytes().unwrap()).unwrap();
    assert!(reopened.delete_sheet("Notes").is_err());
    assert!(reopened.unprotect_workbook("secret").unwrap());
    assert!(!reopened.unprotect_workbook("").unwrap());
    assert_eq!(reopened.cell("Notes", "C3").unwrap().number, Some(7.));
}

#[test]
fn historical_generic_iso_credentials_and_partial_metadata_cannot_be_bypassed() {
    let bytes = fixture(
        r#"<workbookProtection lockStructure="1" algorithmName="SHA-256" hashValue="06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30=" saltValue="AAECAwQFBgcICQoLDA0ODw==" spinCount="3"/>"#,
    );
    assert!(workbook_protection(&bytes).unwrap().unwrap().has_password);
    assert!(unprotect_workbook(&bytes, "wrong").is_err());
    let open = unprotect_workbook(&bytes, "密碼🔒").unwrap();
    assert!(!workbook_protection(&open).unwrap().unwrap().configured());
    assert!(!xml(&open).contains("hashValue="));
    let partial = fixture(
        r#"<workbookProtection lockStructure="1" workbookPassword="83AF" hashValue="AA=="/>"#,
    );
    assert!(unprotect_workbook(&partial, "password").is_err());
}

#[test]
fn historical_revision_scope_is_preserved_and_multiple_modern_verifiers_all_apply() {
    let ambiguous = fixture(
        r#"<workbookProtection lockStructure="1" lockRevision="1" algorithmName="SHA-256" hashValue="06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30=" saltValue="AAECAwQFBgcICQoLDA0ODw==" spinCount="3"/>"#,
    );
    assert!(unprotect_workbook(&ambiguous, "密碼🔒").is_err());
    let mut p = OpcPackage::open(&ambiguous).unwrap();
    let source = p.required_part_str("xl/workbook.xml").unwrap().replace(
        "lockRevision=\"1\"",
        "lockRevision=\"0\" revisionsPassword=\"1234\"",
    );
    p.replace("xl/workbook.xml", source.into_bytes());
    assert!(unprotect_workbook(&p.write().unwrap(), "密碼🔒").is_err());
    assert!(protect_workbook(&ambiguous, "new", &[0; 16]).is_err());
    let both = fixture(
        r#"<workbookProtection lockStructure="1" workbookAlgorithmName="SHA-256" workbookHashValue="06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30=" workbookSaltValue="AAECAwQFBgcICQoLDA0ODw==" workbookSpinCount="3" algorithmName="unknown" hashValue="AA==" saltValue="" spinCount="0"/>"#,
    );
    assert!(unprotect_workbook(&both, "密碼🔒").is_err());
}
