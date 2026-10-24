use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    ChartChange, ChartKind, HeadlessError, HeadlessWorkbook, ShapeAnchor,
};
use jade_xlsx_reader::CellRef;

fn fixture(name: &str, protection: &str) -> Vec<u8> {
    let bytes = std::fs::read(format!(
        "{}/../../public/fixtures/{name}-workbook.xlsx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let part = "xl/worksheets/sheet1.xml";
    let xml = p.required_part_str(part).unwrap();
    let xml = if xml.contains("</sheetData>") {
        xml.replacen("</sheetData>", &format!("</sheetData>{protection}"), 1)
    } else {
        xml.replacen("<sheetData/>", &format!("<sheetData/>{protection}"), 1)
    };
    p.replace(part, xml.into_bytes());
    jade_xlsx_writer::set_note(&mut p, part, CellRef::new(0, 0), "author", "existing").unwrap();
    p.write().unwrap()
}
fn refused<T: std::fmt::Debug>(
    book: &HeadlessWorkbook,
    result: Result<T, HeadlessError>,
    before: &[u8],
) {
    assert!(
        matches!(result, Err(HeadlessError::Protected { .. })),
        "{result:?}"
    );
    assert_eq!(book.export_bytes().unwrap(), before);
    assert!(!book.can_undo() && !book.can_redo());
}
fn anchor() -> ShapeAnchor {
    ShapeAnchor {
        from: CellRef::new(1, 2),
        from_offset: (0, 0),
        to: CellRef::new(5, 8),
        to_offset: (0, 0),
    }
}
#[test]
fn objects_only_lock_refuses_note_creation_and_deletion_atomically() {
    let bytes = fixture(
        "objects",
        r#"<sheetProtection sheet="0" objects="1" scenarios="0"/>"#,
    );
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert!(b.workbook().sheets[0].protection.is_none());
    let before = b.export_bytes().unwrap();
    let result = b.set_note("Data", "B1", "forbidden", "author");
    refused(&b, result, &before);
    let result = b.delete_note("Data", "A1");
    refused(&b, result, &before);
    b.set_cells("Data", &[("D20".into(), CellInput::Number(72.0))])
        .unwrap();
    assert_eq!(b.cell("Data", "D20").unwrap().number, Some(72.0));
}

#[test]
fn objects_only_lock_covers_shapes_charts_pictures_and_text_without_writes() {
    let mut b = HeadlessWorkbook::open_for_editing(&fixture(
        "objects",
        r#"<sheetProtection objects="1"/>"#,
    ))
    .unwrap();
    let before = b.export_bytes().unwrap();
    let r = b.set_shape_text("Data", "TextBox 1", "forbidden");
    refused(&b, r, &before);
    let r = b.place_shape("Data", 2, &anchor());
    refused(&b, r, &before);
    let r = b.delete_shape("Data", 4);
    refused(&b, r, &before);
    let change = ChartChange {
        title: Some(Some("forbidden".into())),
        ..Default::default()
    };
    let r = b.set_chart("Data", 3, &change);
    refused(&b, r, &before);
    let r = b.insert_chart("Data", "A1:B3", ChartKind::Column);
    refused(&b, r, &before);
    for id in [1, 2, 3] {
        let mut b = HeadlessWorkbook::open_for_editing(&fixture(
            "pictures",
            r#"<sheetProtection objects="1"/>"#,
        ))
        .unwrap();
        let before = b.export_bytes().unwrap();
        let r = b.place_shape("Pictures", id, &anchor());
        refused(&b, r, &before);
        let r = b.delete_shape("Pictures", id);
        refused(&b, r, &before);
    }
}

#[test]
fn content_object_and_scenario_flags_remain_independent() {
    for flags in [
        r#"sheet="0" objects="0" scenarios="1""#,
        r#"sheet="1" objects="0" scenarios="1""#,
        r#"objects="0""#,
    ] {
        let mut b = HeadlessWorkbook::open_for_editing(&fixture(
            "objects",
            &format!("<sheetProtection {flags}/>"),
        ))
        .unwrap();
        let state = b.worksheet_protection("Data").unwrap().unwrap();
        assert!(state.allowed.objects);
        assert_eq!(state.allowed.scenarios, !flags.contains(r#"scenarios="1""#));
        b.set_note("Data", "D20", "allowed", "author").unwrap();
        b.set_shape_text("Data", "TextBox 1", "allowed").unwrap();
        b.place_shape("Data", 2, &anchor()).unwrap();
        let result = b.set_cells("Data", &[("D21".into(), CellInput::Number(72.0))]);
        if state.sheet {
            assert!(matches!(result, Err(HeadlessError::Protected { .. })));
        } else {
            result.unwrap();
        }
        let saved = b.export_bytes().unwrap();
        let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
        assert_eq!(reopened.worksheet_protection("Data").unwrap(), Some(state));
        assert_eq!(
            reopened.workbook().sheets[0]
                .shape_named("TextBox 1")
                .unwrap()
                .text
                .as_deref(),
            Some("allowed")
        );
    }
}

#[test]
fn authenticated_removal_history_and_reopen_refresh_independent_guards() {
    let mut b = HeadlessWorkbook::open_for_editing(&fixture(
        "objects",
        r#"<sheetProtection objects="1" scenarios="1" password="83AF"/>"#,
    ))
    .unwrap();
    let before = b.export_bytes().unwrap();
    assert!(b.unprotect_sheet("Data", "wrong").is_err());
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(!b.can_undo());
    assert!(b.unprotect_sheet("Data", "password").unwrap());
    assert!(
        b.worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
    b.set_note("Data", "D20", "allowed", "author").unwrap();
    assert!(b.undo().unwrap());
    assert!(b.undo().unwrap());
    let before = b.export_bytes().unwrap();
    assert!(b.can_redo());
    assert!(matches!(
        b.delete_shape("Data", 2),
        Err(HeadlessError::Protected { .. })
    ));
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(b.can_redo());
    assert!(b.redo().unwrap());
    assert!(
        b.worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
    assert!(b.undo().unwrap());
    let reopened = HeadlessWorkbook::open_for_editing(&b.export_bytes().unwrap()).unwrap();
    assert!(
        !reopened
            .worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
}

#[test]
fn only_selected_owned_protection_controls_object_edits_including_mce() {
    for record in [
        r#"<sheetProtection xmlns="urn:foreign" objects="1"/>"#,
        r#"<extLst><ext uri="foreign"><sheetProtection objects="1"/></ext></extLst>"#,
    ] {
        let mut b = HeadlessWorkbook::open_for_editing(&fixture("objects", record)).unwrap();
        assert!(b.worksheet_protection("Data").unwrap().is_none());
        b.set_note("Data", "D20", "allowed", "author").unwrap();
    }
    let record = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:unknown"><mc:Choice Requires="u"><sheetProtection objects="0"/></mc:Choice><mc:Fallback><sheetProtection objects="1" scenarios="1"/></mc:Fallback></mc:AlternateContent>"#;
    let mut b = HeadlessWorkbook::open_for_editing(&fixture("objects", record)).unwrap();
    assert!(b.workbook().sheets[0].protection.is_none());
    assert!(
        !b.worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .scenarios
    );
    let before = b.export_bytes().unwrap();
    let r = b.set_shape_text("Data", "TextBox 1", "forbidden");
    refused(&b, r, &before);
    b.set_cells("Data", &[("D20".into(), CellInput::Number(72.0))])
        .unwrap();
    assert!(
        !b.worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
}

#[test]
fn independent_locks_follow_sheet_parts_and_never_leak_to_other_sheets() {
    let mut b =
        HeadlessWorkbook::open_for_editing(&fixture("charts", r#"<sheetProtection objects="1"/>"#))
            .unwrap();
    assert!(b.worksheet_protection("Data O'Brien").unwrap().is_none());
    b.set_note("Data O'Brien", "D20", "allowed", "author")
        .unwrap();
    b.rename_sheet("Charts", "Renamed").unwrap();
    assert!(
        !b.worksheet_protection("Renamed")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
    let before = b.export_bytes().unwrap();
    assert!(matches!(
        b.delete_shape("Renamed", 1),
        Err(HeadlessError::Protected { .. })
    ));
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(b.worksheet_protection("Data O'Brien").unwrap().is_none());
}
