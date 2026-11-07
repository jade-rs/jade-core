use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn engineering_radix_ranges_follow_history_and_saved_types() {
    for (function, second, corrected) in [
        ("BIN2DEC", "2", "3"),
        ("BIN2HEX", "2", "3"),
        ("BIN2OCT", "2", "3"),
        ("DEC2BIN", "1010", "1011"),
        ("DEC2HEX", "A", "B"),
        ("DEC2OCT", "12", "13"),
        ("HEX2BIN", "10000", "10001"),
        ("HEX2DEC", "16", "17"),
        ("HEX2OCT", "20", "21"),
        ("OCT2BIN", "1000", "1001"),
        ("OCT2DEC", "8", "9"),
        ("OCT2HEX", "8", "9"),
    ] {
        let mut package = OpcPackage::open(include_bytes!(
            "../../../public/fixtures/sample-workbook.xlsx"
        ))
        .unwrap();
        package.replace("xl/worksheets/sheet1.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
        let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
        let name = "売上サマリ";
        book.set(name, "B1", CellInput::Error("#N/A".into()))
            .unwrap();
        book.set(name, "B2", CellInput::Text("10".into())).unwrap();
        book.set(name, "F1", CellInput::Formula(format!("{function}(B1:B2)")))
            .unwrap();
        book.set(name, "D1", CellInput::Formula("ROWS(F1#)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A", "{function}");
        assert_eq!(book.cell(name, "F2").unwrap().display, second, "{function}");
        book.set(name, "B1", CellInput::Text("11".into())).unwrap();
        assert_eq!(
            book.cell(name, "F1").unwrap().display,
            corrected,
            "{function}"
        );
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A", "{function}");
        book.redo().unwrap();
        assert_eq!(
            book.cell(name, "F1").unwrap().display,
            corrected,
            "{function}"
        );
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(
                reopened.cell(name, "F1").unwrap().display,
                "#N/A",
                "{function}"
            );
            let value = reopened.cell(name, "F2").unwrap();
            assert_eq!(value.display, second, "{function}");
            let expected = if function.ends_with("2DEC") {
                Some(second.parse::<f64>().unwrap())
            } else {
                None
            };
            assert_eq!(value.number, expected, "{function}");
            assert_eq!(reopened.cell(name, "D1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn decimal_ranges_follow_radix_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Text("10".into())),
        ("B2", CellInput::Text("FFFFFFFFFFFFFFFF".into())),
        ("C1", CellInput::Number(1.0)),
        ("D1", CellInput::Number(16.0)),
        ("F1", CellInput::Formula("DECIMAL(B1:B2,C1:D1)".into())),
        ("J1", CellInput::Formula("ROWS(F1#)*COLUMNS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    assert_eq!(
        book.cell(name, "G2").unwrap().number,
        Some(2.0_f64.powi(64))
    );
    book.set(name, "C1", CellInput::Number(16.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(16.0));
    book.set(name, "B1", CellInput::Text("FF".into())).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(255.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(16.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(255.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "F2"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "G1").unwrap().number, Some(16.0));
        assert_eq!(
            reopened.cell(name, "G2").unwrap().number,
            Some(2.0_f64.powi(64))
        );
        assert_eq!(reopened.cell(name, "J1").unwrap().number, Some(4.0));
    }
}

#[test]
fn base_ranges_follow_padding_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1.0)),
        ("B2", CellInput::Number(2.0)),
        ("C1", CellInput::Number(2.0)),
        ("D1", CellInput::Number(16.0)),
        ("E1", CellInput::Number(256.0)),
        ("E2", CellInput::Number(4.0)),
        ("G1", CellInput::Formula("BASE(B1:B2,C1:D1,E1:E2)".into())),
        ("K1", CellInput::Formula("ROWS(G1#)*COLUMNS(G1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "G1").unwrap().display, "#NUM!");
    book.set(name, "E1", CellInput::Number(4.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().display, "0001");
    book.set(name, "B1", CellInput::Number(15.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().display, "1111");
    assert_eq!(book.cell(name, "H1").unwrap().display, "000F");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().display, "0001");
    book.redo().unwrap();
    assert_eq!(book.cell(name, "H1").unwrap().display, "000F");
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["G1", "H1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "G2").unwrap().display, "0010");
        assert_eq!(reopened.cell(name, "H2").unwrap().display, "0002");
        assert_eq!(reopened.cell(name, "K1").unwrap().number, Some(4.0));
    }
}

#[test]
fn bit_ranges_follow_operand_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(-1.0)),
        ("B2", CellInput::Number(3.0)),
        ("C1", CellInput::Number(1.0)),
        ("D1", CellInput::Number(2.0)),
        ("F1", CellInput::Formula("BITLSHIFT(B1:B2,C1:D1)".into())),
        ("I1", CellInput::Formula("BITRSHIFT(B1:B2,C1:D1)".into())),
        ("L1", CellInput::Formula("BITAND(B1:B2,C1:D1)".into())),
        ("O1", CellInput::Formula("BITOR(B1:B2,C1:D1)".into())),
        ("R1", CellInput::Formula("BITXOR(B1:B2,C1:D1)".into())),
        ("U1", CellInput::Formula("ROWS(F1#)*COLUMNS(F1#)".into())),
        (
            "U2",
            CellInput::Formula("BITLSHIFT(140737488355328,17)".into()),
        ),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.set(name, "B1", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(4.0));
    book.set(name, "D1", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(16.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(8.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(16.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in [
            "F1", "G1", "I1", "J1", "L1", "M1", "O1", "P1", "R1", "S1", "U2",
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        for (at, expected) in [
            ("F2", 6.0),
            ("G2", 12.0),
            ("I2", 1.0),
            ("J2", 0.0),
            ("L2", 1.0),
            ("M2", 2.0),
            ("O2", 3.0),
            ("P2", 3.0),
            ("R2", 2.0),
            ("S2", 1.0),
            ("U1", 4.0),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(expected));
        }
    }
}

#[test]
fn delta_and_gestep_ranges_follow_operand_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(-1.0)),
        ("B2", CellInput::Number(1.0)),
        ("C1", CellInput::Error("#N/A".into())),
        ("D1", CellInput::Number(1.0)),
        ("F1", CellInput::Formula("DELTA(B1:B2,C1:D1)".into())),
        ("I1", CellInput::Formula("GESTEP(B1:B2,C1:D1)".into())),
        ("M1", CellInput::Formula("ROWS(F1#)*COLUMNS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
    book.set(name, "C1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(1.0));
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "J1").unwrap().number, Some(1.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(0.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(1.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "F2", "I1", "I2"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        for (at, expected) in [
            ("G1", 0.0),
            ("G2", 1.0),
            ("J1", 0.0),
            ("J2", 1.0),
            ("M1", 4.0),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(expected));
        }
    }
}

#[test]
fn error_function_ranges_follow_bound_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(0.0)),
        ("B2", CellInput::Number(1.0)),
        ("C1", CellInput::Error("#N/A".into())),
        ("D1", CellInput::Number(2.0)),
        ("G1", CellInput::Formula("ROUND(ERF(B1:B2,C1:D1),8)".into())),
        (
            "L1",
            CellInput::Formula("ROUND(ERF.PRECISE(B1:B2),8)".into()),
        ),
        (
            "O1",
            CellInput::Formula("ROUND(ERFC.PRECISE(B1:B2),8)".into()),
        ),
        ("R1", CellInput::Formula("ROUND(ERFC(B1:B2),8)".into())),
        ("U1", CellInput::Formula("ROWS(G1#)*COLUMNS(G1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "G1").unwrap().display, "#N/A");
    book.set(name, "C1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "G2").unwrap().number, Some(-0.84270079));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().display, "#N/A");
    book.redo().unwrap();
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "H1").unwrap().number, Some(0.15262147));
    assert_eq!(book.cell(name, "L1").unwrap().number, Some(0.84270079));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["G1", "G2"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        for (at, expected) in [
            ("H1", 0.99532227),
            ("H2", 0.15262147),
            ("L1", 0.0),
            ("L2", 0.84270079),
            ("O1", 1.0),
            ("O2", 0.15729921),
            ("R1", 1.0),
            ("R2", 0.15729921),
            ("U1", 4.0),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(expected));
        }
    }
}

#[test]
fn phi_and_gauss_ranges_follow_source_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Error("#N/A".into())),
        ("B2", CellInput::Number(0.0)),
        ("F1", CellInput::Formula("PHI(B1:B2)".into())),
        ("I1", CellInput::Formula("GAUSS(B1:B2)".into())),
        ("L1", CellInput::Formula("PHI(1)".into())),
        ("L2", CellInput::Formula("GAUSS(1)".into())),
        ("L3", CellInput::Formula("PHI(0)".into())),
        ("N1", CellInput::Formula("ROWS(F1#)+ROWS(I1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    assert_eq!(
        book.cell(name, "F1").unwrap().number,
        book.cell(name, "L1").unwrap().number
    );
    assert_eq!(
        book.cell(name, "I1").unwrap().number,
        book.cell(name, "L2").unwrap().number
    );
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
    book.redo().unwrap();
    assert_eq!(
        book.cell(name, "I1").unwrap().number,
        book.cell(name, "L2").unwrap().number
    );
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        assert_eq!(
            reopened.cell(name, "F2").unwrap().number,
            reopened.cell(name, "L3").unwrap().number
        );
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(0.0));
        assert_eq!(reopened.cell(name, "N1").unwrap().number, Some(4.0));
    }
}

#[test]
fn fisher_ranges_follow_source_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1.0)),
        ("B2", CellInput::Number(0.0)),
        ("C1", CellInput::Error("#N/A".into())),
        ("C2", CellInput::Number(0.0)),
        ("F1", CellInput::Formula("FISHER(B1:B2)".into())),
        ("I1", CellInput::Formula("FISHERINV(C1:C2)".into())),
        ("L1", CellInput::Formula("FISHER(0.5)".into())),
        ("L2", CellInput::Formula("FISHERINV(1)".into())),
        ("N1", CellInput::Formula("ROWS(F1#)+ROWS(I1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "I1").unwrap().display, "#N/A");
    book.set(name, "B1", CellInput::Number(0.5)).unwrap();
    assert_eq!(
        book.cell(name, "F1").unwrap().number,
        book.cell(name, "L1").unwrap().number
    );
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    assert_eq!(
        book.cell(name, "F1").unwrap().number,
        book.cell(name, "L1").unwrap().number
    );
    book.set(name, "C1", CellInput::Number(1.0)).unwrap();
    assert_eq!(
        book.cell(name, "I1").unwrap().number,
        book.cell(name, "L2").unwrap().number
    );
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(reopened.cell(name, "I1").unwrap().display, "#N/A");
        for at in ["F2", "I2"] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(0.0));
        }
        assert_eq!(reopened.cell(name, "N1").unwrap().number, Some(4.0));
    }
}

#[test]
fn standardize_ranges_follow_parameter_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(10.0)),
        ("B2", CellInput::Number(20.0)),
        ("C1", CellInput::Number(2.0)),
        ("D1", CellInput::Number(4.0)),
        ("E1", CellInput::Number(0.0)),
        ("E2", CellInput::Number(2.0)),
        (
            "G1",
            CellInput::Formula("STANDARDIZE(B1:B2,C1:D1,E1:E2)".into()),
        ),
        ("K1", CellInput::Formula("ROWS(G1#)*COLUMNS(G1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "G1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "G2").unwrap().number, Some(9.0));
    book.set(name, "E1", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(4.0));
    book.set(name, "C1", CellInput::Number(4.0)).unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(3.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(4.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "G1").unwrap().number, Some(3.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["G1", "H1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "G2").unwrap().number, Some(9.0));
        assert_eq!(reopened.cell(name, "H2").unwrap().number, Some(8.0));
        assert_eq!(reopened.cell(name, "K1").unwrap().number, Some(4.0));
    }
}

#[test]
fn rank_ranges_follow_order_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1.0)),
        ("B2", CellInput::Number(3.0)),
        ("B3", CellInput::Number(3.0)),
        ("B4", CellInput::Number(7.0)),
        ("C1", CellInput::Number(2.0)),
        ("C2", CellInput::Number(3.0)),
        ("D1", CellInput::Number(0.0)),
        ("D2", CellInput::Number(1.0)),
        (
            "F1",
            CellInput::Formula("RANK.EQ(C1:C2,B1:B4,D1:D2)".into()),
        ),
        (
            "I1",
            CellInput::Formula("RANK.AVG(C1:C2,B1:B4,D1:D2)".into()),
        ),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "C1", CellInput::Number(7.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
    book.set(name, "D1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(4.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(4.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(2.5));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn percentrank_ranges_follow_precision_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(0.0)),
        ("B2", CellInput::Number(10.0)),
        ("B3", CellInput::Number(20.0)),
        ("B4", CellInput::Number(30.0)),
        ("C1", CellInput::Number(-1.0)),
        ("C2", CellInput::Number(15.0)),
        ("D1", CellInput::Number(3.0)),
        ("D2", CellInput::Number(3.0)),
        (
            "F1",
            CellInput::Formula("PERCENTRANK.INC(B1:B4,C1:C2,D1:D2)".into()),
        ),
        (
            "I1",
            CellInput::Formula("PERCENTRANK.EXC(B1:B4,C1:C2,D1:D2)".into()),
        ),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "C1", CellInput::Number(5.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.166));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(0.3));
    book.set(name, "D1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.1));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.166));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.1));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        for at in ["F2", "I2"] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(0.5));
        }
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn percentile_and_quartile_ranges_follow_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1.0)),
        ("B2", CellInput::Number(2.0)),
        ("B3", CellInput::Number(3.0)),
        ("B4", CellInput::Number(4.0)),
        ("C1", CellInput::Number(2.0)),
        ("C2", CellInput::Number(0.5)),
        ("D1", CellInput::Number(5.0)),
        ("D2", CellInput::Number(2.0)),
        (
            "F1",
            CellInput::Formula("PERCENTILE.INC(B1:B4,C1:C2)".into()),
        ),
        (
            "I1",
            CellInput::Formula("PERCENTILE.EXC(B1:B4,C1:C2)".into()),
        ),
        ("L1", CellInput::Formula("QUARTILE.INC(B1:B4,D1:D2)".into())),
        ("O1", CellInput::Formula("QUARTILE.EXC(B1:B4,D1:D2)".into())),
        ("R1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1", "L1", "O1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "C1", CellInput::Number(0.25)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.75));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(1.25));
    book.set(name, "D1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "L1").unwrap().number, Some(1.75));
    assert_eq!(book.cell(name, "O1").unwrap().number, Some(1.25));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "L1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "C2", CellInput::Number(0.75)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(3.25));
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(3.75));
    book.undo().unwrap();
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1", "L1", "O1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        for at in ["F2", "I2", "L2", "O2"] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(2.5));
        }
        assert_eq!(reopened.cell(name, "R1").unwrap().number, Some(2.0));
    }
    for (at, input) in [
        ("B1", CellInput::Number(-9E307)),
        ("B2", CellInput::Number(9E307)),
        ("B3", CellInput::Text("ignored".into())),
        ("B4", CellInput::Text("ignored".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F2", "I2", "L2", "O2"] {
        assert_eq!(book.cell(name, at).unwrap().number, Some(0.0));
    }
    book.set(name, "B1", CellInput::Number(-8E307)).unwrap();
    assert!((book.cell(name, "F2").unwrap().number.unwrap() / 1E306 - 5.0).abs() < 1E-12);
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1", "L1", "O1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        for at in ["F2", "I2", "L2", "O2"] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(0.0));
        }
    }
}

#[test]
fn order_statistic_rank_ranges_follow_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(7.0)),
        ("B2", CellInput::Number(3.0)),
        ("B3", CellInput::Number(1.0)),
        ("C1", CellInput::Number(0.0)),
        ("C2", CellInput::Number(2.0)),
        ("F1", CellInput::Formula("LARGE(B1:B3,C1:C2)".into())),
        ("I1", CellInput::Formula("SMALL(B1:B3,C1:C2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "C1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(1.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "B3", CellInput::Number(9.0)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(7.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(3.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(3.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn permutation_ranges_follow_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(-1.0)),
        ("B2", CellInput::Number(3.0)),
        ("C1", CellInput::Number(2.0)),
        ("C2", CellInput::Number(2.0)),
        ("F1", CellInput::Formula("PERMUT(B1:B2,C1:C2)".into())),
        ("I1", CellInput::Formula("PERMUTATIONA(B1:B2,C1:C2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "B1", CellInput::Number(4.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(12.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(16.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "C2", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(27.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(6.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(9.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn atan2_coordinate_ranges_follow_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(0.0)),
        ("B2", CellInput::Number(1.0)),
        ("C1", CellInput::Number(0.0)),
        ("C2", CellInput::Number(0.0)),
        (
            "F1",
            CellInput::Formula("DEGREES(ATAN2(B1:B2,C1:C2))".into()),
        ),
        ("I1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#DIV/0!");
    book.set(name, "C1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(90.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#DIV/0!");
    book.redo().unwrap();
    book.set(name, "B2", CellInput::Number(-1.0)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(180.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "#DIV/0!");
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(0.0));
        assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
    }
}

#[test]
fn unary_numeric_ranges_keep_domain_errors_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1000.0)),
        ("B2", CellInput::Number(0.0)),
        ("F1", CellInput::Formula("EXP(B1:B2)".into())),
        ("I1", CellInput::Formula("ASIN(B1:B2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "B1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(0.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "B2", CellInput::Number(1.0)).unwrap();
    assert!(book.cell(name, "I2").unwrap().number.unwrap() > 1.5);
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(1.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(0.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn factorial_ranges_recover_from_overflow_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(1E100)),
        ("B2", CellInput::Number(6.0)),
        ("F1", CellInput::Formula("FACT(B1:B2)".into())),
        ("I1", CellInput::Formula("FACTDOUBLE(B1:B2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "B1", CellInput::Number(5.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(120.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(15.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "B2", CellInput::Number(7.0)).unwrap();
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(105.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(720.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(48.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn combination_ranges_follow_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(-1.0)),
        ("B2", CellInput::Number(5.0)),
        ("B3", CellInput::Number(11.0)),
        ("C1", CellInput::Number(2.0)),
        ("C2", CellInput::Number(2.0)),
        ("C3", CellInput::Number(5.0)),
        ("F1", CellInput::Formula("COMBIN(B1:B3,C1:C3)".into())),
        ("I1", CellInput::Formula("COMBINA(B1:B3,C1:C3)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "B1", CellInput::Number(4.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(6.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(10.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "C2", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(35.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "I2").unwrap().number, Some(15.0));
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(10.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(15.0));
        assert_eq!(reopened.cell(name, "F3").unwrap().number, Some(462.0));
        assert_eq!(reopened.cell(name, "I3").unwrap().number, Some(3003.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(3.0));
    }
}

#[test]
fn power_and_log_ranges_keep_domain_errors_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(-1.0)),
        ("B2", CellInput::Number(4.0)),
        ("C1", CellInput::Number(0.5)),
        ("C2", CellInput::Number(0.5)),
        ("F1", CellInput::Formula("POWER(B1:B2,C1:C2)".into())),
        ("I1", CellInput::Formula("LOG(B1:B2,2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
    }
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(0.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    book.set(name, "C2", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(16.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(2.0));
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn modern_rounding_ranges_follow_mode_edits_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Error("#N/A".into())),
        ("B2", CellInput::Number(-5.5)),
        ("C1", CellInput::Number(-2.0)),
        ("C2", CellInput::Number(-2.0)),
        ("D1", CellInput::Number(0.0)),
        ("D2", CellInput::Number(1.0)),
        (
            "F1",
            CellInput::Formula("CEILING.MATH(B1:B2,C1:C2,D1:D2)".into()),
        ),
        (
            "I1",
            CellInput::Formula("FLOOR.MATH(B1:B2,C1:C2,D1:D2)".into()),
        ),
        (
            "L1",
            CellInput::Formula("CEILING.PRECISE(B1:B2,C1:C2)".into()),
        ),
        (
            "O1",
            CellInput::Formula("FLOOR.PRECISE(B1:B2,C1:C2)".into()),
        ),
        ("R1", CellInput::Formula("ISO.CEILING(B1:B2,C1:C2)".into())),
        ("U1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1", "L1", "O1", "R1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "B1", CellInput::Number(-5.5)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(-4.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(-6.0));
    book.set(name, "D1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(-6.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(-4.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(-4.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(-6.0));
    book.undo().unwrap();
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1", "L1", "O1", "R1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        for (at, value) in [
            ("F2", -6.0),
            ("I2", -4.0),
            ("L2", -4.0),
            ("O2", -6.0),
            ("R2", -4.0),
            ("U1", 2.0),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(value));
        }
    }
}

#[test]
fn multiple_rounding_ranges_keep_sign_errors_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(10.0)),
        ("B2", CellInput::Number(-10.0)),
        ("C1", CellInput::Number(-3.0)),
        ("C2", CellInput::Number(-3.0)),
        ("F1", CellInput::Formula("MROUND(B1:B2,C1:C2)".into())),
        ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ("L1", CellInput::Formula("FLOOR(B1:B2,C1:C2)".into())),
        ("O1", CellInput::Formula("CEILING(B1:B2,C1:C2)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(-9.0));
    assert_eq!(book.cell(name, "L1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "O1").unwrap().display, "#NUM!");
    book.set(name, "C1", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(9.0));
    assert_eq!(book.cell(name, "L1").unwrap().number, Some(9.0));
    assert_eq!(book.cell(name, "O1").unwrap().number, Some(12.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    book.redo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(9.0));
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(-9.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().display, "#NUM!");
        assert_eq!(reopened.cell(name, "O1").unwrap().display, "#NUM!");
        assert_eq!(reopened.cell(name, "L2").unwrap().number, Some(-9.0));
        assert_eq!(reopened.cell(name, "O2").unwrap().number, Some(-12.0));
        assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
    }
}

#[test]
fn division_ranges_keep_local_errors_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(10.0)),
        ("B2", CellInput::Number(-10.0)),
        ("C1", CellInput::Number(0.0)),
        ("C2", CellInput::Number(3.0)),
        ("F1", CellInput::Formula("MOD(B1:B2,C1:C2)".into())),
        ("I1", CellInput::Formula("QUOTIENT(B1:B2,C1:C2)".into())),
        ("L1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#DIV/0!");
    }
    book.set(name, "C1", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(3.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#DIV/0!");
    book.redo().unwrap();
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(3.0));
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#DIV/0!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(-3.0));
        assert_eq!(reopened.cell(name, "L1").unwrap().number, Some(2.0));
    }
}

#[test]
fn rounding_ranges_follow_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Error("#N/A".into())),
        ("B2", CellInput::Number(-1.475)),
        ("B3", CellInput::Number(1E300)),
        ("C1", CellInput::Number(2.0)),
        ("F1", CellInput::Formula("ROUND(B1:B3,C1)".into())),
        ("I1", CellInput::Formula("ROUNDUP(B1:B3,C1)".into())),
        ("L1", CellInput::Formula("ROUNDDOWN(B1:B3,C1)".into())),
        ("R1", CellInput::Formula("TRUNC(B1:B3,C1)".into())),
        ("O1", CellInput::Formula("ROWS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    for at in ["F1", "I1", "L1", "R1"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "B1", CellInput::Number(1.005)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.01));
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(1.01));
    assert_eq!(book.cell(name, "L1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "R1").unwrap().number, Some(1.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Number(1.0)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(-1.5));
    book.undo().unwrap();
    book.set(name, "B1", CellInput::Error("#N/A".into()))
        .unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "I1", "L1", "R1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(-1.48));
        assert_eq!(reopened.cell(name, "I2").unwrap().number, Some(-1.48));
        assert_eq!(reopened.cell(name, "L2").unwrap().number, Some(-1.47));
        assert_eq!(reopened.cell(name, "R2").unwrap().number, Some(-1.47));
        for at in ["F3", "I3", "L3", "R3"] {
            assert_eq!(reopened.cell(name, at).unwrap().number, Some(1E300));
        }
        assert_eq!(reopened.cell(name, "O1").unwrap().number, Some(3.0));
    }
}

#[test]
fn leading_decimal_literals_survive_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.23)).unwrap();
    book.set(name, "C1", CellInput::Text(".\"x\"00".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.x23");
    book.set(name, "B1", CellInput::Number(-12.34)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-12.x34");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.x23");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text(".\\x00".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-12.x34");
    book.set(
        name,
        "D1",
        CellInput::Formula("TEXT(0.5,\".\"\"x\"\"hh:mm:ss.000\")".into()),
    )
    .unwrap();
    assert_eq!(book.cell(name, "D1").unwrap().display, ".x12:00:00.000");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "-12.x34");
        assert_eq!(reopened.cell(name, "D1").unwrap().display, ".x12:00:00.000");
    }
}

#[test]
fn decimal_point_positions_survive_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.25)).unwrap();
    book.set(name, "C1", CellInput::Text("0.".into())).unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.");
    book.set(name, "B1", CellInput::Number(1.5)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2.");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("0.\"x\"00".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(1.23)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.x23");
    book.set(name, "C1", CellInput::Text(".00".into())).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.23");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.x23");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.x23");
    }
}

#[test]
fn grouped_text_formats_survive_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1234.5)).unwrap();
    book.set(name, "C1", CellInput::Text("###,##0.00".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1,234.50");
    book.set(name, "B1", CellInput::Number(12.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12.00");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1,234.50");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("000,000".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "000,012");
    book.set(name, "B1", CellInput::Number(1_234_567.0))
        .unwrap();
    book.set(name, "C1", CellInput::Text("###,##0,.00".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1,234.57");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1,234.57");
    }
}

#[test]
fn decimal_half_rounding_survives_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.25)).unwrap();
    book.set(name, "C1", CellInput::Text("0.0".into())).unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.3");
    book.set(name, "B1", CellInput::Number(-1.25)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-1.3");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.3");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("0.00".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(2.675)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2.68");
    book.set(name, "C1", CellInput::Text("0.###E+00".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(12345.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.235E+04");
    book.set(name, "C1", CellInput::Text("##0.0E+0".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(999.95)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.0E+3");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.0E+3");
    }
}

#[test]
fn engineering_text_formats_survive_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(12_200_000.0))
        .unwrap();
    book.set(name, "C1", CellInput::Text("##0.0E+0".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12.2E+6");
    book.set(name, "B1", CellInput::Number(123_456.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "123.5E+3");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12.2E+6");
    book.redo().unwrap();
    book.set(name, "B1", CellInput::Number(999.96)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.0E+3");
    book.set(name, "C1", CellInput::Text("#0.0E+0".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "10.0E+2");
    book.set(name, "B1", CellInput::Number(0.00123)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12.3E-4");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "12.3E-4");
    }
}

#[test]
fn scientific_text_carries_survive_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(999.9)).unwrap();
    book.set(name, "C1", CellInput::Text("0.00E+00".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.00E+03");
    book.set(name, "B1", CellInput::Number(9.12)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "9.12E+00");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.00E+03");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("0.00E-00".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "9.12E00");
    book.undo().unwrap();
    book.set(name, "B1", CellInput::Number(9.999E307)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.00E+308");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.00E+308");
    }
}

#[test]
fn optional_decimal_spacing_survives_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    book.set(name, "C1", CellInput::Text("0.??".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.  ");
    book.set(name, "C1", CellInput::Text("0.0?".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.0 ");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.  ");
    book.set(name, "B1", CellInput::Number(1.2)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.2 ");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.  ");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.2 ");
    }
}

#[test]
fn negative_text_formats_keep_signs_through_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(-1.0)).unwrap();
    book.set(name, "C1", CellInput::Text("0".into())).unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-1");
    book.set(name, "C1", CellInput::Text("0.00".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(-0.01)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-0.01");
    book.set(name, "B1", CellInput::Number(0.01)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "0.01");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-0.01");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text(".00".into())).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, ".01");
    book.set(name, "B1", CellInput::Number(-12.34)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-12.34");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "-12.34");
    }
}

#[test]
fn empty_text_format_sections_survive_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(-2.0)).unwrap();
    book.set(name, "C1", CellInput::Text("0;".into())).unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "");
    book.set(name, "B1", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("0;;".into())).unwrap();
    book.set(name, "B1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "");
    book.set(name, "C1", CellInput::Text("0".into())).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "0");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "");
    }
}

#[test]
fn conditional_fraction_formats_follow_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(2.25)).unwrap();
    book.set(name, "C1", CellInput::Text("[>=2]# ?/8;0.00".into()))
        .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2 2/8");
    book.set(name, "B1", CellInput::Number(1.5)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.50");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2 2/8");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("[>=1]0/0;0.00".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "3/2");
    book.set(name, "C1", CellInput::Text("[>=0]0.00;[<0]0/0;0".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(-1.5)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-3/2");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "-3/2");
    }
}

#[test]
fn conditional_text_formats_follow_edits_history_and_saved_caches() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(101.0)).unwrap();
    book.set(
        name,
        "C1",
        CellInput::Text("[>100]0.0;[<=-100]0.00;0.000".into()),
    )
    .unwrap();
    book.set(name, "A1", CellInput::Formula("TEXT(B1,C1)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "101.0");
    book.set(name, "B1", CellInput::Number(-100.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-100.00");
    book.set(name, "B1", CellInput::Number(100.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "100.000");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "-100.00");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text("[>=100]0.00;0".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "100.00");
    book.set(name, "C1", CellInput::Text("[<>0]0.00;0".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "100.00");
    book.set(name, "B1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "0");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "100.00");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "0");
    }
    book.set(name, "C1", CellInput::Text("[<1]hh:mm;0.00".into()))
        .unwrap();
    book.set(name, "B1", CellInput::Number(0.5)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12:00");
    book.set(
        name,
        "D1",
        CellInput::Formula("TEXT(B1,\"[>=1]General;0.00\")".into()),
    )
    .unwrap();
    assert_eq!(book.cell(name, "D1").unwrap().display, "0.50");
    book.set(name, "B1", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "2.00");
    assert_eq!(book.cell(name, "D1").unwrap().display, "2");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12:00");
    assert_eq!(book.cell(name, "D1").unwrap().display, "0.50");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "2.00");
        assert_eq!(reopened.cell(name, "D1").unwrap().display, "2");
    }
}

#[test]
fn general_exponent_carries_follow_source_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(999999999999.0))
        .unwrap();
    book.set(
        name,
        "A1",
        CellInput::Formula("TEXT(B1,\"General\")".into()),
    )
    .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1E+12");
    book.set(name, "B1", CellInput::Number(1.23456E20)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1.23456E+20");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "1E+12");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.23456E+20");
    }
}

#[test]
fn text_fractional_seconds_and_date_carry_survive_both_epochs_and_history() {
    for date1904 in [false, true] {
        let mut package = OpcPackage::open(include_bytes!(
            "../../../public/fixtures/sample-workbook.xlsx"
        ))
        .unwrap();
        package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
        if date1904 {
            let xml = package
                .part_str("xl/workbook.xml")
                .unwrap()
                .unwrap()
                .replace("<sheets>", "<workbookPr date1904=\"1\"/><sheets>");
            package.replace("xl/workbook.xml", xml.into_bytes());
        }
        let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
        let name = "売上サマリ";
        book.set(name, "C1", CellInput::Number(59.9994 / 86400.0))
            .unwrap();
        book.set(name, "B1", CellInput::Text("hh:mm:ss.000".into()))
            .unwrap();
        book.set(name, "A1", CellInput::Formula("TEXT(C1,B1)".into()))
            .unwrap();
        book.set(
            name,
            "D1",
            CellInput::Formula("TEXT(DATE(2024,1,1)+C1,\"yyyy-mm-dd hh:mm:ss.000\")".into()),
        )
        .unwrap();
        assert_eq!(book.cell(name, "A1").unwrap().display, "00:00:59.999");
        book.set(
            name,
            "E1",
            CellInput::Formula("TEXT(C1,\"h:mm:ss.000 a/p\")".into()),
        )
        .unwrap();
        book.set(
            name,
            "F1",
            CellInput::Formula("TEXT(C1,\"h:mm am/pm\")".into()),
        )
        .unwrap();
        assert_eq!(book.cell(name, "E1").unwrap().display, "12:00:59.999 a");
        book.set(name, "C1", CellInput::Number(59.9996 / 86400.0))
            .unwrap();
        assert_eq!(book.cell(name, "A1").unwrap().display, "00:01:00.000");
        book.undo().unwrap();
        assert_eq!(book.cell(name, "A1").unwrap().display, "00:00:59.999");
        book.redo().unwrap();
        book.set(name, "C1", CellInput::Number(86399.9996 / 86400.0))
            .unwrap();
        book.set(name, "B1", CellInput::Text("mm:ss.00".into()))
            .unwrap();
        assert_eq!(book.cell(name, "A1").unwrap().display, "00:00.00");
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "A1").unwrap().display, "00:00:00.000");
            assert_eq!(
                reopened.cell(name, "D1").unwrap().display,
                "2024-01-02 00:00:00.000"
            );
            assert_eq!(reopened.cell(name, "E1").unwrap().display, "12:00:00.000 a");
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "12:00 am");
        }
    }
}

#[test]
fn text_format_ranges_keep_text_types_during_edits_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Text("12".into())).unwrap();
    book.set(name, "B2", CellInput::Bool(true)).unwrap();
    book.set(name, "B3", CellInput::Error("#N/A".into()))
        .unwrap();
    for (at, text) in [("C1", "000"), ("C2", "0"), ("C3", "0"), ("C4", "0.00")] {
        book.set(name, at, CellInput::Text(text.into())).unwrap();
    }
    book.set(name, "A1", CellInput::Formula("TEXT(B1:B4,C1:C4)".into()))
        .unwrap();
    book.set(name, "D1", CellInput::Formula("ISTEXT(A2)".into()))
        .unwrap();
    for (at, text) in [
        ("A1", "012"),
        ("A2", "TRUE"),
        ("A3", "#N/A"),
        ("A4", "0.00"),
        ("D1", "TRUE"),
    ] {
        assert_eq!(book.cell(name, at).unwrap().display, text, "{at}");
    }
    book.set(name, "C1", CellInput::Text("0.00".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "12.00");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "012");
    book.redo().unwrap();
    book.set(name, "B1", CellInput::Number(34.0)).unwrap();
    book.set(name, "C2", CellInput::Error("#N/A".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "#N/A");
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for (at, text) in [
            ("A1", "34.00"),
            ("A2", "TRUE"),
            ("A3", "#N/A"),
            ("A4", "0.00"),
            ("D1", "TRUE"),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, text, "{at}");
        }
    }
}

#[test]
fn fixed_and_dollar_follow_decimal_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1E20)).unwrap();
    book.set(name, "B2", CellInput::Number(-1234.5)).unwrap();
    book.set(name, "B3", CellInput::Error("#N/A".into()))
        .unwrap();
    book.set(name, "C1", CellInput::Number(2.0)).unwrap();
    book.set(name, "C2", CellInput::Number(0.0)).unwrap();
    book.set(name, "C3", CellInput::Number(2.0)).unwrap();
    book.set(name, "A1", CellInput::Formula("FIXED(B1,C1,TRUE)".into()))
        .unwrap();
    book.set(name, "D1", CellInput::Formula("DOLLAR(B1,C1)".into()))
        .unwrap();
    book.set(
        name,
        "F1",
        CellInput::Formula("FIXED(B1:B3,C1:C3,TRUE)".into()),
    )
    .unwrap();
    book.set(name, "G1", CellInput::Formula("DOLLAR(B1:B3,C1:C3)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().display, "-1235");
    assert_eq!(book.cell(name, "G2").unwrap().display, "($1,235)");
    assert_eq!(book.cell(name, "F3").unwrap().display, "#N/A");
    assert_eq!(
        book.cell(name, "A1").unwrap().display,
        "100000000000000000000.00"
    );
    assert_eq!(
        book.cell(name, "D1").unwrap().display,
        "$100,000,000,000,000,000,000.00"
    );
    book.set(name, "C1", CellInput::Number(0.0)).unwrap();
    assert_eq!(
        book.cell(name, "A1").unwrap().display,
        "100000000000000000000"
    );
    book.undo().unwrap();
    assert_eq!(
        book.cell(name, "A1").unwrap().display,
        "100000000000000000000.00"
    );
    book.redo().unwrap();
    book.set(name, "C2", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "F2").unwrap().display, "-1234.50");
    book.undo().unwrap();
    book.set(name, "B1", CellInput::Number(1.005)).unwrap();
    book.set(name, "C1", CellInput::Number(2.0)).unwrap();
    book.set(name, "C1", CellInput::Number(128.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "F2").unwrap().display, "-1235");
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "1.01");
        assert_eq!(reopened.cell(name, "D1").unwrap().display, "$1.01");
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "1.01");
        assert_eq!(reopened.cell(name, "G1").unwrap().display, "$1.01");
        assert_eq!(reopened.cell(name, "F2").unwrap().display, "-1235");
        assert_eq!(reopened.cell(name, "F3").unwrap().display, "#N/A");
    }
}

#[test]
fn numbervalue_spills_with_per_row_separators_and_survives_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, text) in [
        ("B1", "bad"),
        ("B2", "1.234,5"),
        ("B3", "1,234.5"),
        ("C1", ","),
        ("C2", ","),
        ("C3", "."),
        ("C4", "."),
        ("C5", "."),
        ("C6", "."),
        ("D1", "."),
        ("D2", "."),
        ("D3", ","),
        ("D4", ","),
        ("D5", ","),
        ("D6", ","),
    ] {
        book.set(name, at, CellInput::Text(text.into())).unwrap();
    }
    book.set(name, "B5", CellInput::Formula("\"\"".into()))
        .unwrap();
    book.set(name, "B6", CellInput::Error("#N/A".into()))
        .unwrap();
    book.set(
        name,
        "A1",
        CellInput::Formula("NUMBERVALUE(B1:B6,C1:C6,D1:D6)".into()),
    )
    .unwrap();
    book.set(name, "E1", CellInput::Formula("SUM(IFERROR(A1#,0))".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "A6").unwrap().display, "#N/A");
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(2469.0));
    book.set(name, "C3", CellInput::Text(",".into())).unwrap();
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(1234.5));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(2469.0));
    book.redo().unwrap();
    book.set(name, "B1", CellInput::Text("2,5".into())).unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for (at, number) in [
            ("A1", 2.5),
            ("A2", 1234.5),
            ("A4", 0.0),
            ("A5", 0.0),
            ("E1", 1237.0),
        ] {
            assert_eq!(
                reopened.cell(name, at).unwrap().number,
                Some(number),
                "{at}"
            );
        }
        assert_eq!(reopened.cell(name, "A3").unwrap().display, "#VALUE!");
        assert_eq!(reopened.cell(name, "A6").unwrap().display, "#N/A");
    }
}

#[test]
fn value_range_spills_time_values_and_element_errors_survive_edit_history() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, value) in [("B1", "bad"), ("B2", "$1,000"), ("B3", "12:00 PM")] {
        book.set(name, at, CellInput::Text(value.into())).unwrap();
    }
    for (at, formula) in [
        ("B5", "\"\""),
        ("A1", "VALUE(B1:B5)"),
        ("C1", "SUM(IFERROR(A1#,0))"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "A1").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "A2").unwrap().number, Some(1000.0));
    assert_eq!(book.cell(name, "A3").unwrap().number, Some(0.5));
    assert_eq!(book.cell(name, "A4").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "A5").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1000.5));
    book.set(name, "B1", CellInput::Text("2".into())).unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1002.5));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "#VALUE!");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1000.5));
    book.redo().unwrap();
    book.set(name, "B3", CellInput::Text("1:30 PM".into()))
        .unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1002.5625));
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "C1").unwrap().number, Some(1002.5625));
        assert_eq!(reopened.cell(name, "A5").unwrap().display, "#VALUE!");
    }
}

#[test]
fn numbervalue_text_and_separator_edits_recalculate_save_and_follow_history() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, text) in [
        ("B1", " 3 000 "),
        ("B2", "2.500,27"),
        ("B3", "1.2,3"),
        ("B4", ""),
        ("B5", "9% %"),
        ("C1", ",decimal"),
        ("D1", ".group"),
    ] {
        book.set(name, at, CellInput::Text(text.into())).unwrap();
    }
    for (at, formula) in [
        ("A1", "NUMBERVALUE(B1)"),
        ("A2", "NUMBERVALUE(B2,C1,D1)"),
        ("A3", "NUMBERVALUE(B3)"),
        ("A4", "NUMBERVALUE(B4)"),
        ("A5", "NUMBERVALUE(B5)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    for (at, value) in [("A1", 3000.0), ("A2", 2500.27), ("A4", 0.0), ("A5", 0.0009)] {
        assert_eq!(book.cell(name, at).unwrap().number, Some(value), "{at}");
    }
    assert_eq!(book.cell(name, "A3").unwrap().display, "#VALUE!");
    book.set(name, "B3", CellInput::Text("1 2".into())).unwrap();
    assert_eq!(book.cell(name, "A3").unwrap().number, Some(12.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A3").unwrap().display, "#VALUE!");
    book.redo().unwrap();
    book.set(name, "C1", CellInput::Text(".decimal".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "#VALUE!");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().number, Some(2500.27));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().number, Some(3000.0));
        assert_eq!(reopened.cell(name, "A2").unwrap().display, "#VALUE!");
        assert_eq!(reopened.cell(name, "A3").unwrap().number, Some(12.0));
    }
}
