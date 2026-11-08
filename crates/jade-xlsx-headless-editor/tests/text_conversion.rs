use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn text_slice_positions_coerce_ranges_through_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, value) in [
        ("B1", CellInput::Text("abcd".into())),
        ("C1", CellInput::Text("2.9".into())),
        ("C2", CellInput::Bool(true)),
        ("C3", CellInput::Bool(false)),
        ("C4", CellInput::Text("bad".into())),
        ("C5", CellInput::Error("#N/A".into())),
    ] {
        book.set(name, at, value).unwrap();
    }
    for (at, formula) in [
        ("A1", "LEFT(B1,C1:C5)"),
        ("D1", "RIGHT(B1,C1:C5)"),
        ("E1", "MID(B1,C1:C5,\"2\")"),
        ("F1", "SUM(IFERROR(LEN(A1#),0))"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    for (at, text) in [
        ("A1", "ab"),
        ("A2", "a"),
        ("A3", ""),
        ("A4", "#VALUE!"),
        ("A5", "#N/A"),
    ] {
        assert_eq!(book.cell(name, at).unwrap().display, text, "{at}");
    }
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(3.0));
    book.set(name, "C1", CellInput::Text("3".into())).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(4.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(3.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for (at, text) in [
            ("A1", "abc"),
            ("D1", "bcd"),
            ("E1", "cd"),
            ("E3", "#VALUE!"),
            ("A5", "#N/A"),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, text, "{at}");
        }
        assert_eq!(reopened.cell(name, "F1").unwrap().number, Some(4.0));
    }
}

#[test]
fn width_conversion_ranges_follow_kana_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Text("ＡＢＣ　ガ".into()))
        .unwrap();
    book.set(name, "I1", CellInput::Text("A".into())).unwrap();
    book.set(name, "I2", CellInput::Text("€".into())).unwrap();
    book.set(name, "I3", CellInput::Error("#N/A".into()))
        .unwrap();
    book.set(name, "B2", CellInput::Text("日本😀".into()))
        .unwrap();
    book.set(name, "B3", CellInput::Error("#N/A".into()))
        .unwrap();
    book.set(name, "B5", CellInput::Formula("\"\"".into()))
        .unwrap();
    for (at, formula) in [
        ("A1", "ASC(B1:B5)"),
        ("C1", "DBCS(A1#)"),
        ("D1", "JIS(A1#)"),
        ("F1", "CODE(I1:I3)"),
        ("E1", "SUM(--IFERROR(EXACT(C1#,D1#),FALSE))"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "A1").unwrap().display, "ABC ｶﾞ");
    assert_eq!(book.cell(name, "C1").unwrap().display, "ＡＢＣ　ガ");
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(4.0));
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(65.0));
    book.set(name, "I1", CellInput::Text("B".into())).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(66.0));
    book.undo().unwrap();
    book.set(name, "B1", CellInput::Text("パピ".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "ﾊﾟﾋﾟ");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "ABC ｶﾞ");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for (at, text) in [
            ("A1", "ﾊﾟﾋﾟ"),
            ("C1", "パピ"),
            ("A2", "日本😀"),
            ("C3", "#N/A"),
            ("A4", ""),
            ("C5", ""),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, text, "{at}");
        }
        assert_eq!(reopened.cell(name, "E1").unwrap().number, Some(4.0));
        assert_eq!(reopened.cell(name, "F1").unwrap().number, Some(65.0));
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(128.0));
        assert_eq!(reopened.cell(name, "F3").unwrap().display, "#N/A");
    }
}

#[test]
fn concat_full_column_and_array_inputs_follow_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, text) in [("B1", "a"), ("B1048576", "z")] {
        book.set(name, at, CellInput::Text(text.into())).unwrap();
    }
    book.set(name, "C1", CellInput::Number(2.0)).unwrap();
    for (at, formula) in [
        ("B3", "SEQUENCE(C1,1,7)"),
        ("F1", "CONCAT(B:B)"),
        ("D1", "CONCAT(B3#,{\"X\",\"Y\"})"),
        ("H1", "CONCATENATE(B3#,\"!\")"),
        ("J1", "ROWS(H1#)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().display, "a78z");
    assert_eq!(book.cell(name, "D1").unwrap().display, "78XY");
    assert_eq!(book.cell(name, "H1").unwrap().display, "7!");
    assert_eq!(book.cell(name, "H2").unwrap().display, "8!");
    assert_eq!(book.cell(name, "J1").unwrap().number, Some(2.0));
    book.set(name, "C1", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "a789z");
    assert_eq!(book.cell(name, "H3").unwrap().display, "9!");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "a78z");
    assert_eq!(book.cell(name, "J1").unwrap().number, Some(2.0));
    book.redo().unwrap();
    book.set(name, "B2", CellInput::Error("#N/A".into()))
        .unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
    book.undo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "a789z");
        assert_eq!(reopened.cell(name, "D1").unwrap().display, "789XY");
        assert_eq!(reopened.cell(name, "H1").unwrap().display, "7!");
        assert_eq!(reopened.cell(name, "H3").unwrap().display, "9!");
        assert_eq!(reopened.cell(name, "J1").unwrap().number, Some(3.0));
    }
}

#[test]
fn text_functions_keep_error_broadcast_shape_through_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Text("x".into())).unwrap();
    for (at, number) in [("C1", 1.0), ("C2", 2.0), ("C3", 3.0)] {
        book.set(name, at, CellInput::Number(number)).unwrap();
    }
    for (at, formula) in [
        ("A1", "REPT(B1,C1:C3)"),
        ("D1", "ROWS(A1#)"),
        ("E1", "SUM(IFERROR(LEN(A1#),0))"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(6.0));
    book.set(name, "B1", CellInput::Error("#N/A".into()))
        .unwrap();
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(3.0));
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(0.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(6.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["A1", "A2", "A3"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A", "{at}");
        }
        assert_eq!(reopened.cell(name, "D1").unwrap().number, Some(3.0));
        assert_eq!(reopened.cell(name, "E1").unwrap().number, Some(0.0));
    }
}

#[test]
fn strict_text_conversion_spills_recalculate_save_and_follow_history() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (at, value) in [
        ("B1", CellInput::Text("a\"b".into())),
        ("B2", CellInput::Number(12.0)),
        ("B3", CellInput::Bool(true)),
        ("B4", CellInput::Error("#N/A".into())),
        ("B5", CellInput::Text("#N/A".into())),
        ("D1", CellInput::Number(1.0)),
        ("D2", CellInput::Number(2.0)),
        ("M1", CellInput::Text("  spaced   text  ".into())),
        ("M2", CellInput::Text(" ".into())),
        (
            "P1",
            CellInput::Text(format!(
                "{}  日本  \n",
                (0u8..32).map(char::from).collect::<String>()
            )),
        ),
        ("P2", CellInput::Text("😀".into())),
        ("P3", CellInput::Error("#N/A".into())),
        ("T1", CellInput::Number(128512.0)),
        ("T2", CellInput::Number(55296.0)),
        ("T3", CellInput::Number(0.0)),
        ("T4", CellInput::Error("#DIV/0!".into())),
        ("T5", CellInput::Number(65.0)),
        ("T6", CellInput::Number(128.0)),
        ("AE1", CellInput::Number(1.0)),
        ("AF1", CellInput::Text("X".into())),
        ("AO1", CellInput::Text("A\nB".into())),
    ] {
        book.set(name, at, value).unwrap();
    }
    for (at, formula) in [
        ("A1", "VALUETOTEXT(B1:B5,D1)"),
        ("C1", "ISERROR(A4)"),
        ("C2", "ISTEXT(A4)"),
        ("E1", "ARRAYTOTEXT(B1:B5,D1)"),
        ("E2", "ARRAYTOTEXT({\"a\"\"b\",\"\"\"\"\"\"},1)"),
        ("F1", "LEN(B1:B5)"),
        ("G1", "SUM(IFERROR(F1#,0))"),
        ("H1", "REPT(B1:B5,D2)"),
        ("J1", "SUM(IFERROR(LEN(H1#),0))"),
        ("K1", "TRIM(B1:B5)"),
        ("N1", "TRIM(M1:M3)"),
        ("Q1", "TRIM(CLEAN(P1:P4))"),
        ("R1", "UNICODE(Q1#)"),
        ("U1", "UNICHAR(T1:T6)"),
        ("V1", "UNICODE(U1#)"),
        ("W1", "CHAR(T1:T6)"),
        ("X1", "EXACT(B1:B5,\"updated\")"),
        ("Z1", "EXACT(B1:B5,\"#N/A\")"),
        ("AA1", "SUM(--IFERROR(EXACT(B1:B5,\"updated\"),FALSE))"),
        ("AB1", "PROPER(B1:B5)"),
        ("AC1", "LOWER(AB1#)"),
        ("AD1", "REPLACE(B1:B5,AE1,1,AF1)"),
        ("AG1", "SUBSTITUTE(B1:B5,\"a\",AF1,AE1)"),
        ("AI1", "SUBSTITUTE(\"aaaa\",\"aa\",AF1,AE1)"),
        ("AJ1", "FIND(\"a\",B1:B5,AE1)"),
        ("AK1", "SUM(IFERROR(AJ1#,0))"),
        ("AL1", "SEARCH(\"a\",B1:B5,AE1)"),
        ("AM1", "SUM(IFERROR(AL1#,0))"),
        ("AN1", "SEARCH(\"a?b\",AO1)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    for (at, expected) in [
        ("A1", "\"a\"\"b\""),
        ("A2", "12"),
        ("A3", "TRUE"),
        ("A4", "#N/A"),
        ("A5", "\"#N/A\""),
        ("C1", "FALSE"),
        ("C2", "TRUE"),
        ("E1", "{\"a\"\"b\";12;TRUE;#N/A;\"#N/A\"}"),
        ("E2", "{\"a\"\"b\",\"\"\"\"\"\"}"),
        ("F1", "3"),
        ("F2", "2"),
        ("F3", "4"),
        ("F4", "#N/A"),
        ("F5", "4"),
        ("G1", "13"),
        ("H1", "a\"ba\"b"),
        ("H4", "#N/A"),
        ("H5", "#N/A#N/A"),
        ("J1", "26"),
        ("K1", "a\"b"),
        ("K4", "#N/A"),
        ("K5", "#N/A"),
        ("N1", "spaced text"),
        ("N2", " "),
        ("N3", ""),
        ("Q1", "日本"),
        ("Q2", "😀"),
        ("Q3", "#N/A"),
        ("Q4", ""),
        ("R1", "26085"),
        ("R2", "128512"),
        ("R3", "#N/A"),
        ("R4", "#VALUE!"),
        ("U1", "😀"),
        ("U2", "#N/A"),
        ("U3", "#VALUE!"),
        ("U4", "#DIV/0!"),
        ("V1", "128512"),
        ("V2", "#N/A"),
        ("W5", "A"),
        ("W6", "€"),
        ("X1", "FALSE"),
        ("X4", "#N/A"),
        ("Z4", "#N/A"),
        ("Z5", "TRUE"),
        ("AA1", "0"),
        ("AB1", "A\"B"),
        ("AB3", "True"),
        ("AB4", "#N/A"),
        ("AC1", "a\"b"),
        ("AC5", "#n/a"),
        ("AD1", "X\"b"),
        ("AD4", "#N/A"),
        ("AD5", "XN/A"),
        ("AG1", "X\"b"),
        ("AG4", "#N/A"),
        ("AI1", "Xaa"),
        ("AJ1", "1"),
        ("AJ2", "#VALUE!"),
        ("AJ4", "#N/A"),
        ("AK1", "1"),
        ("AL1", "1"),
        ("AL4", "#N/A"),
        ("AL5", "4"),
        ("AM1", "5"),
        ("AN1", "1"),
    ] {
        assert_eq!(book.cell(name, at).unwrap().display, expected, "{at}");
    }
    book.set(name, "D1", CellInput::Number(0.0)).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "a\"b");
    assert_eq!(
        book.cell(name, "E1").unwrap().display,
        "a\"b, 12, TRUE, #N/A, #N/A"
    );
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "\"a\"\"b\"");
    book.redo().unwrap();
    book.set(name, "B1", CellInput::Text("updated".into()))
        .unwrap();
    book.set(name, "AE1", CellInput::Number(2.0)).unwrap();
    assert_eq!(book.cell(name, "AD1").unwrap().display, "uXdated");
    assert_eq!(book.cell(name, "AI1").unwrap().display, "aaX");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "AD1").unwrap().display, "Xpdated");
    assert_eq!(book.cell(name, "AI1").unwrap().display, "Xaa");
    book.set(name, "P2", CellInput::Text("Z".into())).unwrap();
    assert_eq!(book.cell(name, "Q2").unwrap().display, "Z");
    assert_eq!(book.cell(name, "R2").unwrap().display, "90");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "R2").unwrap().display, "128512");
    book.redo().unwrap();
    assert_eq!(book.cell(name, "R2").unwrap().display, "90");
    book.undo().unwrap();
    book.set(name, "T1", CellInput::Number(66.0)).unwrap();
    assert_eq!(book.cell(name, "U1").unwrap().display, "B");
    assert_eq!(book.cell(name, "V1").unwrap().display, "66");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "U1").unwrap().display, "😀");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "updated");
        assert_eq!(reopened.cell(name, "C1").unwrap().display, "FALSE");
        assert_eq!(reopened.cell(name, "C2").unwrap().display, "TRUE");
        assert_eq!(reopened.cell(name, "F1").unwrap().display, "7");
        assert_eq!(reopened.cell(name, "F4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "G1").unwrap().display, "17");
        assert_eq!(reopened.cell(name, "H1").unwrap().display, "updatedupdated");
        assert_eq!(reopened.cell(name, "H4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "J1").unwrap().display, "34");
        assert_eq!(reopened.cell(name, "K1").unwrap().display, "updated");
        assert_eq!(reopened.cell(name, "N1").unwrap().display, "spaced text");
        assert_eq!(reopened.cell(name, "N2").unwrap().display, " ");
        assert_eq!(reopened.cell(name, "N3").unwrap().display, "");
        assert_eq!(reopened.cell(name, "X1").unwrap().display, "TRUE");
        assert_eq!(reopened.cell(name, "X4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "Z5").unwrap().display, "TRUE");
        assert_eq!(reopened.cell(name, "AA1").unwrap().display, "1");
        assert_eq!(reopened.cell(name, "AB1").unwrap().display, "Updated");
        assert_eq!(reopened.cell(name, "AC1").unwrap().display, "updated");
        assert_eq!(reopened.cell(name, "AC4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "AC5").unwrap().display, "#n/a");
        assert_eq!(reopened.cell(name, "AD1").unwrap().display, "Xpdated");
        assert_eq!(reopened.cell(name, "AD4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "AD5").unwrap().display, "XN/A");
        assert_eq!(reopened.cell(name, "AG1").unwrap().display, "updXted");
        assert_eq!(reopened.cell(name, "AG4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "AI1").unwrap().display, "Xaa");
        assert_eq!(reopened.cell(name, "AJ1").unwrap().display, "4");
        assert_eq!(reopened.cell(name, "AJ4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "AK1").unwrap().display, "4");
        assert_eq!(reopened.cell(name, "AL1").unwrap().display, "4");
        assert_eq!(reopened.cell(name, "AL4").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(name, "AL5").unwrap().display, "4");
        assert_eq!(reopened.cell(name, "AM1").unwrap().display, "8");
        assert_eq!(reopened.cell(name, "AN1").unwrap().display, "1");
        for (at, expected) in [
            ("U1", "😀"),
            ("U2", "#N/A"),
            ("U3", "#VALUE!"),
            ("U4", "#DIV/0!"),
            ("V1", "128512"),
            ("V2", "#N/A"),
            ("W5", "A"),
            ("W6", "€"),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, expected, "{at}");
        }
        for (at, expected) in [
            ("Q1", "日本"),
            ("Q2", "😀"),
            ("Q3", "#N/A"),
            ("Q4", ""),
            ("R1", "26085"),
            ("R2", "128512"),
            ("R3", "#N/A"),
            ("R4", "#VALUE!"),
        ] {
            assert_eq!(reopened.cell(name, at).unwrap().display, expected, "{at}");
        }
        assert_eq!(
            reopened.cell(name, "E1").unwrap().display,
            "updated, 12, TRUE, #N/A, #N/A"
        );
        assert_eq!(
            reopened.cell(name, "E2").unwrap().display,
            "{\"a\"\"b\",\"\"\"\"\"\"}"
        );
    }
    // The output limit counts UTF-16 units, not UTF-8 bytes. Input cells
    // exercise the conversion itself without REPT's separate size limit.
    let emoji = "😀".repeat(16370);
    book.set(name, "B1", CellInput::Text(emoji.clone()))
        .unwrap();
    assert_eq!(
        book.cell(name, "E1").unwrap().display,
        format!("{emoji}, 12, TRUE, #N/A, #N/A")
    );
    book.set(name, "B1", CellInput::Text("😀".repeat(16373)))
        .unwrap();
    assert_eq!(book.cell(name, "E1").unwrap().display, "#VALUE!");
}
