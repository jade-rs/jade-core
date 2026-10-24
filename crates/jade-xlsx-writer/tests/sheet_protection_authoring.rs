use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::SheetProtection;
use jade_xlsx_writer::{sheet_protection::*, WriteError};
const PART: &str = "xl/worksheets/sheet1.xml";
fn fixture(record: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p.required_part_str(PART).unwrap();
    let end = xml.rfind("</worksheet>").unwrap();
    let xml = format!("{}{record}{}", &xml[..end], &xml[end..]);
    p.replace(PART, xml.into_bytes());
    p.write().unwrap()
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
#[test]
fn strong_and_passwordless_sheet_authoring_roundtrip_permissions_and_other_parts() {
    for password in ["", "秘密🔒"] {
        let original = fixture("");
        let allowed = SheetProtection {
            format_cells: true,
            insert_rows: true,
            select_locked_cells: false,
            ..Default::default()
        };
        let protected = protect_sheet(&original, PART, allowed, password, &[7; 16]).unwrap();
        let info = sheet_protection(&protected, PART).unwrap().unwrap();
        assert!(info.protected);
        assert_eq!(info.has_password, !password.is_empty());
        assert_eq!(info.allowed, allowed);
        let read = jade_xlsx_reader::read_xlsx(&protected).unwrap();
        assert_eq!(read.sheets[0].protection, Some(allowed));
        let out = xml(&protected);
        assert!(!out.contains("秘密"));
        if !password.is_empty() {
            assert!(out.contains("SHA-512") && out.contains("spinCount=\"100000\""));
            assert!(matches!(
                unprotect_sheet(&protected, PART, "wrong"),
                Err(WriteError::IncorrectPassword)
            ));
        }
        let a = OpcPackage::open(&original).unwrap();
        let b = OpcPackage::open(&protected).unwrap();
        assert_eq!(a.part_names(), b.part_names());
        for name in a.part_names() {
            if name != PART {
                assert_eq!(a.part(name), b.part(name), "{name}");
            }
        }
        let open = unprotect_sheet(&protected, PART, password).unwrap();
        assert!(jade_xlsx_reader::read_xlsx(&open).unwrap().sheets[0]
            .protection
            .is_none());
        assert_eq!(unprotect_sheet(&open, PART, password).unwrap(), open);
    }
}
#[test]
fn legacy_iso_and_modern_precedence_authenticate_without_overwrite() {
    for (record, password) in [
        (
            r#"<sheetProtection sheet="1" password="83AF"/>"#,
            "password",
        ),
        (
            r#"<sheetProtection sheet="1" algorithmName="SHA-256" spinCount="3" saltValue="AAECAwQFBgcICQoLDA0ODw==" hashValue="06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30=" password="83AF"/>"#,
            "密碼🔒",
        ),
    ] {
        let source = fixture(record);
        assert!(protect_sheet(&source, PART, SheetProtection::default(), "new", &[0; 16]).is_err());
        assert!(unprotect_sheet(&source, PART, "wrong").is_err());
        if password != "password" {
            assert!(unprotect_sheet(&source, PART, "password").is_err());
        }
        assert!(
            !sheet_protection(&unprotect_sheet(&source, PART, password).unwrap(), PART)
                .unwrap()
                .unwrap()
                .configured()
        );
    }
}
#[test]
fn namespace_aliases_opaque_children_and_independent_flags_survive() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let original = fixture(&format!(
            r#"<p:sheetProtection xmlns:p="{ns}" xmlns:f="urn:f" f:password="keep" f:mode="keep"><f:payload/></p:sheetProtection><f:sheetProtection xmlns:f="urn:f" sheet="1" password="opaque"/>"#
        ));
        let mut p = OpcPackage::open(&original).unwrap();
        let s = xml(&original).replace(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            ns,
        );
        p.replace(PART, s.into_bytes());
        let protected = protect_sheet(
            &p.write().unwrap(),
            PART,
            SheetProtection::default(),
            "",
            &[0; 16],
        )
        .unwrap();
        let out = xml(&unprotect_sheet(&protected, PART, "").unwrap());
        for keep in [
            "f:password=\"keep\"",
            "f:mode=\"keep\"",
            "<f:payload/>",
            "password=\"opaque\"",
            "<p:sheetProtection",
        ] {
            assert!(out.contains(keep), "{out}");
        }
    }
    let original = fixture(r#"<sheetProtection objects="1" password="83AF"/>"#);
    assert!(sheet_protection(&original, PART)
        .unwrap()
        .unwrap()
        .configured());
    assert!(unprotect_sheet(&original, PART, "wrong").is_err());
    assert!(
        !sheet_protection(&unprotect_sheet(&original, PART, "password").unwrap(), PART)
            .unwrap()
            .unwrap()
            .configured()
    );
}
#[test]
fn malformed_unsupported_duplicate_and_mce_credentials_cannot_be_removed() {
    for record in [
        r#"<sheetProtection sheet="1" password="83AF" hashValue="AA=="/>"#,
        r#"<sheetProtection sheet="1" password="83AF" algorithmName="unknown" hashValue="AA==" saltValue="" spinCount="0"/>"#,
        r#"<sheetProtection sheet="1"/><sheetProtection sheet="0"/>"#,
        r#"<sheetProtection sheet="bad"/>"#,
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Fallback><sheetProtection sheet="1" password="83AF"/></mc:Fallback></mc:AlternateContent>"#,
    ] {
        let source = fixture(record);
        assert!(
            unprotect_sheet(&source, PART, "password").is_err(),
            "{record}"
        );
        assert!(
            protect_sheet(&source, PART, SheetProtection::default(), "", &[0; 16]).is_err(),
            "{record}"
        );
    }
    let original = fixture("");
    assert_eq!(
        unprotect_sheet(&original, PART, "ignored").unwrap(),
        original
    );
}
#[test]
fn headless_guards_auth_failures_and_undo_redo_are_atomic() {
    let original = fixture("");
    let mut book = HeadlessWorkbook::open(&original).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    assert!(!book.unprotect_sheet(&name, "").unwrap());
    assert!(book
        .protect_sheet(&name, SheetProtection::default(), "secret", &[3; 16])
        .unwrap());
    let saved = book.bytes().to_vec();
    let before = saved.clone();
    assert!(book.unprotect_sheet(&name, "wrong").is_err());
    assert_eq!(book.bytes().to_vec(), before);
    assert!(book
        .set(&name, "A1", jade_xlsx_headless_api::CellInput::Number(42.0))
        .is_err());
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes().to_vec(), original);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes().to_vec(), saved);
    assert!(book.unprotect_sheet(&name, "secret").unwrap());
    assert!(book.workbook().sheets[0].protection.is_none());
    assert!(book.undo().unwrap());
    assert!(book.workbook().sheets[0].protection.is_some());
    assert!(book.redo().unwrap());
}

#[test]
fn cell_lock_and_hidden_styles_preserve_effective_flags_and_extension_xml() {
    use jade_xlsx_writer::{add_cell_format, FormatChange};
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let styles = format!(
            r#"<s:styleSheet xmlns:s="{ns}" xmlns:f="urn:f"><s:fonts count="1"><s:font/></s:fonts><s:fills count="1"><s:fill/></s:fills><s:cellStyleXfs count="1"><s:xf><s:protection locked="0" hidden="1"/></s:xf></s:cellStyleXfs><s:cellXfs count="2"><s:xf xfId="0"><f:protection marker="foreign"/><s:extLst><s:ext uri="x"><f:data/></s:ext></s:extLst></s:xf><s:xf><s:protection locked="0" hidden="1" f:mode="keep"><f:data/></s:protection></s:xf></s:cellXfs></s:styleSheet>"#
        );
        for base in [0, 1] {
            let (locked, index) = add_cell_format(
                &styles,
                Some(base),
                &FormatChange {
                    locked: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
            let parsed = jade_xlsx_reader::styles::parse_styles(&locked).unwrap();
            let format = parsed.cell_formats[index as usize];
            assert!(!format.unlocked && format.formula_hidden);
            assert!(
                locked.contains("marker=\"foreign\"")
                    && locked.contains("f:mode=\"keep\"")
                    && locked.contains("<f:data/>")
            );
            let (unhidden, new_index) = add_cell_format(
                &locked,
                Some(index),
                &FormatChange {
                    formula_hidden: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
            let f = jade_xlsx_reader::styles::parse_styles(&unhidden)
                .unwrap()
                .cell_formats[new_index as usize];
            assert!(!f.unlocked && !f.formula_hidden);
            let (same, reused) = add_cell_format(
                &unhidden,
                Some(new_index),
                &FormatChange {
                    formula_hidden: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(same, unhidden);
            assert_eq!(reused, new_index);
        }
    }
}
#[test]
fn unlocked_cells_and_format_permissions_survive_saved_sheet_protection() {
    use jade_xlsx_writer::FormatChange;
    let mut book = HeadlessWorkbook::open(&fixture("")).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    book.set_format(
        &name,
        "A1",
        &FormatChange {
            locked: Some(false),
            formula_hidden: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    book.protect_sheet(&name, SheetProtection::default(), "", &[0; 16])
        .unwrap();
    book.set(&name, "A1", jade_xlsx_headless_api::CellInput::Number(42.))
        .unwrap();
    assert!(book
        .set(&name, "B1", jade_xlsx_headless_api::CellInput::Number(99.))
        .is_err());
    assert!(book
        .set_format(
            &name,
            "A1",
            &FormatChange {
                locked: Some(true),
                ..Default::default()
            }
        )
        .is_err());
    let mut reopened = HeadlessWorkbook::open(&book.save_bytes().unwrap()).unwrap();
    reopened
        .set(&name, "A1", jade_xlsx_headless_api::CellInput::Number(43.))
        .unwrap();
    assert!(reopened
        .set(&name, "B1", jade_xlsx_headless_api::CellInput::Number(99.))
        .is_err());
    reopened.unprotect_sheet(&name, "").unwrap();
    reopened
        .protect_sheet(
            &name,
            SheetProtection {
                format_cells: true,
                ..Default::default()
            },
            "",
            &[0; 16],
        )
        .unwrap();
    reopened
        .set_format(
            &name,
            "A1",
            &FormatChange {
                locked: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(reopened
        .set(&name, "A1", jade_xlsx_headless_api::CellInput::Number(99.))
        .is_err());
}
