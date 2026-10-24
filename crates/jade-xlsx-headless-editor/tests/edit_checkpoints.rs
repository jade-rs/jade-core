use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}
#[test]
fn checkpoint_restores_redo_current_model_and_original_clipboard_identity() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    b.set(&sheet, "A20", CellInput::Number(1.)).unwrap();
    b.set(&sheet, "A20", CellInput::Number(2.)).unwrap();
    b.undo().unwrap();
    let copied = b.copy_snapshot(&sheet, "A20", false).unwrap();
    let before = b.bytes().to_vec();
    let checkpoint = b.edit_checkpoint();
    b.set(&sheet, "A20", CellInput::Number(42.)).unwrap();
    assert!(!b.can_redo());
    b.set_calculation_time_zone("Asia/Tokyo").unwrap();
    assert!(b.restore_edit_checkpoint(checkpoint));
    assert_eq!(b.bytes(), before);
    assert_eq!(b.calculation_time_zone(), "UTC");
    assert!(b.can_redo());
    b.redo().unwrap();
    assert_eq!(b.cell(&sheet, "A20").unwrap().number, Some(2.));
    b.undo().unwrap();
    b.paste_snapshot(&copied, &sheet, "B20", false).unwrap();
    assert_eq!(b.cell(&sheet, "B20").unwrap().number, Some(1.));
}
#[test]
fn a_checkpoint_cannot_be_restored_into_another_open_workbook() {
    let b = open();
    let checkpoint = b.edit_checkpoint();
    let mut other = open();
    let before = other.bytes().to_vec();
    assert!(!other.restore_edit_checkpoint(checkpoint));
    assert_eq!(other.bytes(), before);
    assert!(!other.can_undo());
}
#[test]
fn checkpoint_keeps_evicted_history_and_session_grants_without_rewriting_passwords() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    for n in 0..100 {
        b.set(&sheet, "A20", CellInput::Number(n as f64)).unwrap();
    }
    let before = b.bytes().to_vec();
    let checkpoint = b.edit_checkpoint();
    for n in 100..103 {
        b.set(&sheet, "A20", CellInput::Number(n as f64)).unwrap();
    }
    assert!(b.restore_edit_checkpoint(checkpoint));
    assert_eq!(b.bytes(), before);
    let mut count = 0;
    while b.undo().unwrap() {
        count += 1;
    }
    assert_eq!(count, 100);
    let range = jade_xlsx_reader::CellRange::parse("A20").unwrap();
    b.set_editable_range(&sheet, None, "Input", &[range], Some(("secret", &[3; 16])))
        .unwrap();
    b.protect_sheet(&sheet, Default::default(), "", &[0; 16])
        .unwrap();
    b.unlock_editable_range(&sheet, "Input", "secret").unwrap();
    let before = b.bytes().to_vec();
    let checkpoint = b.edit_checkpoint();
    b.lock_editable_ranges(&sheet).unwrap();
    b.unprotect_sheet(&sheet, "").unwrap();
    assert!(b.restore_edit_checkpoint(checkpoint));
    assert_eq!(b.bytes(), before);
    assert!(b.editable_range_unlocked(&sheet, "Input").unwrap());
    assert!(b.sheet_protection(&sheet).unwrap().unwrap().configured());
    b.set(&sheet, "A20", CellInput::Number(7.)).unwrap();
}
#[test]
fn protected_cut_source_is_guarded_even_when_the_destination_is_editable() {
    let mut p = jade_opc::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let part = "xl/worksheets/sheet1.xml";
    let xml=p.required_part_str(part).unwrap().replace("</worksheet>",r#"<sheetProtection sheet="1"/><protectedRanges><protectedRange name="Destination" sqref="A20"/></protectedRanges></worksheet>"#);
    p.replace(part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let sheet = b.workbook().sheets[0].name.clone();
    let copied = b.copy_snapshot(&sheet, "A1", true).unwrap();
    let before = b.bytes().to_vec();
    let targets = [sheet.clone(), b.workbook().sheets[1].name.clone()];
    for target in targets {
        assert!(b.paste_snapshot(&copied, &target, "A20", false).is_err());
        assert_eq!(b.bytes(), before);
        assert!(!b.can_undo());
    }
}

#[test]
fn enormous_clipboard_ranges_are_bounded_before_visiting_cells() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    let copied = b.copy_snapshot(&sheet, "A1:XFC1048575", false).unwrap();
    let before = b.bytes().to_vec();
    assert!(matches!(
        b.paste_snapshot(&copied, &sheet, "A1", false),
        Err(jade_xlsx_headless_editor::HeadlessError::RangeTooLarge { .. })
    ));
    assert_eq!(b.bytes(), before);
    assert!(!b.can_undo());
}

#[test]
fn full_sheet_cut_checks_sparse_unlocked_defaults_and_a_far_password_exception() {
    let mut b = open();
    let source = b.workbook().sheets[0].name.clone();
    let target = b.workbook().sheets[1].name.clone();
    b.set_format(
        &source,
        "A:XFD",
        &jade_xlsx_headless_editor::FormatChange {
            locked: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    b.set_format(
        &source,
        "XFD1048576",
        &jade_xlsx_headless_editor::FormatChange {
            locked: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    b.set_editable_range(
        &source,
        None,
        "Far",
        &[jade_xlsx_reader::CellRange::parse("XFD1048576").unwrap()],
        Some(("secret", &[2; 16])),
    )
    .unwrap();
    b.protect_sheet(&source, Default::default(), "", &[0; 16])
        .unwrap();
    let expected = b.cell(&source, "A1").unwrap().display;
    let copied = b.copy_snapshot(&source, "A1:XFD1048576", true).unwrap();
    let before = b.bytes().to_vec();
    assert!(b.paste_snapshot(&copied, &target, "A1", true).is_err());
    assert_eq!(b.bytes(), before);
    b.unlock_editable_range(&source, "Far", "secret").unwrap();
    b.paste_snapshot(&copied, &target, "A1", true).unwrap();
    assert_eq!(b.cell(&target, "A1").unwrap().display, expected);
    assert!(b.workbook().sheets.iter().all(|s| s.rows.len() < 100));
}
#[test]
fn full_column_cut_requires_every_sparse_permission_band_including_a_single_cell_gap() {
    let mut b = open();
    let source = b.workbook().sheets[0].name.clone();
    let target = b.workbook().sheets[1].name.clone();
    b.set(&source, "Z20", CellInput::Number(7.)).unwrap();
    for (name, reference, password) in [
        ("Upper", "Z1:Z524288", None),
        ("Lower", "Z524290:Z1048576", None),
        ("Gap", "Z524289", Some(("secret", &[4; 16]))),
    ] {
        b.set_editable_range(
            &source,
            None,
            name,
            &[jade_xlsx_reader::CellRange::parse(reference).unwrap()],
            password,
        )
        .unwrap();
    }
    b.protect_sheet(&source, Default::default(), "", &[0; 16])
        .unwrap();
    let expected = b.cell(&source, "Z20").unwrap().display;
    let copied = b.copy_snapshot(&source, "Z:Z", true).unwrap();
    let before = b.bytes().to_vec();
    assert!(b.paste_snapshot(&copied, &target, "Z1", false).is_err());
    assert_eq!(b.bytes(), before);
    b.unlock_editable_range(&source, "Gap", "secret").unwrap();
    b.paste_snapshot(&copied, &target, "Z1", false).unwrap();
    assert_eq!(b.cell(&target, "Z20").unwrap().display, expected);
}

#[test]
fn sparse_rectangle_permissions_match_individual_cell_edits_with_styles_and_range_grants() {
    let mut p = jade_opc::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut styles = p.required_part_str("xl/styles.xml").unwrap();
    let first = styles.find("<cellXfs").unwrap();
    let last = styles[first..].find("</cellXfs>").unwrap() + first + "</cellXfs>".len();
    styles.replace_range(first..last,r#"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"><protection locked="1"/></xf><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"><protection locked="0"/></xf></cellXfs>"#);
    p.replace("xl/styles.xml", styles.into_bytes());
    p.replace("xl/worksheets/sheet1.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="2" max="3" style="1"/></cols><sheetData><row r="1"><c r="A1" s="1"><v>1</v></c></row><row r="2" s="1" customFormat="1"><c r="C2" s="0"><v>1</v></c></row><row r="6"><c r="F6" s="1"><v>1</v></c></row></sheetData><sheetProtection sheet="1"/><protectedRanges><protectedRange name="Everyone" sqref="B4:C5"/><protectedRange name="Password" sqref="D2:D3" password="83AF"/></protectedRanges></worksheet>"#.to_vec());
    p.replace("xl/worksheets/sheet2.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#.to_vec());
    for authenticated in [false, true] {
        let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
        let target = b.workbook().sheets[0].name.clone();
        let source = b.workbook().sheets[1].name.clone();
        if authenticated {
            b.unlock_editable_range(&target, "Password", "password")
                .unwrap();
        }
        let mut permitted = [[false; 6]; 6];
        for (row, row_cells) in permitted.iter_mut().enumerate() {
            for (col, permitted_cell) in row_cells.iter_mut().enumerate() {
                let checkpoint = b.edit_checkpoint();
                let cell = format!("{}{}", (b'A' + col as u8) as char, row + 1);
                *permitted_cell = b.set(&target, &cell, CellInput::Number(111.)).is_ok();
                assert!(b.restore_edit_checkpoint(checkpoint));
            }
        }
        for reference in [
            "A1:F6", "A1:A6", "B1:C6", "D1:D6", "A2:F2", "B4:C5", "E6:F6", "C2:E4", "A4:A6",
            "F1:F6", "D2:D3", "F6",
        ] {
            let r = jade_xlsx_reader::CellRange::parse(reference).unwrap();
            let expected = (r.start.row..=r.end.row).all(|row| {
                (r.start.column..=r.end.column).all(|col| permitted[row as usize][col as usize])
            });
            let width = r.end.column - r.start.column;
            let height = r.end.row - r.start.row;
            let source_range = format!("A1:{}{}", (b'A' + width as u8) as char, height + 1);
            let anchor = format!(
                "{}{}",
                (b'A' + r.start.column as u8) as char,
                r.start.row + 1
            );
            let copied = b.copy_snapshot(&source, &source_range, false).unwrap();
            let checkpoint = b.edit_checkpoint();
            let result = b.paste_snapshot(&copied, &target, &anchor, false);
            let actual = result.is_ok();
            assert_eq!(
                actual, expected,
                "{reference}; authenticated={authenticated}; result={result:?}"
            );
            assert!(b.restore_edit_checkpoint(checkpoint));
        }
    }
}
