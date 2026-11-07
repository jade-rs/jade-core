use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn legacy_output_range_stays_fixed_through_growth_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(D1)</f><v>1</v></c><c r="D1"><v>2</v></c><c r="E1"><f>FORMULATEXT(A1)</f></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3" t="e"><v>#N/A</v></c></row></sheetData></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    assert_eq!(book.cell(name, "A3").unwrap().display, "#N/A");
    book.set(name, "D1", CellInput::Number(5.0)).unwrap();
    assert_eq!(book.cell(name, "A3").unwrap().number, Some(3.0));
    assert_eq!(book.cell(name, "A4").unwrap().display, "");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A3").unwrap().display, "#N/A");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        let array = reopened.workbook().sheets[0].array_formulas[0];
        assert_eq!(
            array.range,
            jade_xlsx_reader::CellRange::parse("A1:A3").unwrap()
        );
        assert_eq!(array.kind, jade_xlsx_reader::ArrayFormulaKind::Legacy);
        assert_eq!(reopened.cell(name, "A3").unwrap().number, Some(3.0));
        assert_eq!(
            reopened.cell(name, "E1").unwrap().display,
            "{=SEQUENCE(D1)}"
        );
    }
}

#[test]
fn legacy_array_anchor_text_keeps_braces_through_input_history_and_reopen() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>2</v></c><c r="B1"><f t="array" ref="B1">SUM(A1, 3)</f><v>5</v></c><c r="C1"><f>FORMULATEXT(B1)</f></c></row></sheetData></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    assert_eq!(book.cell(name, "C1").unwrap().display, "{=SUM(A1, 3)}");
    book.set(name, "A1", CellInput::Number(4.0)).unwrap();
    assert_eq!(book.cell(name, "B1").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "C1").unwrap().display, "{=SUM(A1, 3)}");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "B1").unwrap().number, Some(5.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "C1").unwrap().display, "{=SUM(A1, 3)}");
        assert_eq!(
            reopened.workbook().sheets[0].array_formulas[0].kind,
            jade_xlsx_reader::ArrayFormulaKind::Legacy
        );
    }
}

#[test]
fn parsed_formula_text_keeps_source_spacing_through_edits_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>2</v></c><c r="B1"><f>SUM(A1, 3)</f><v>5</v></c><c r="C1"><f>FORMULATEXT(B1)</f></c></row></sheetData></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1, 3)");
    book.set(name, "B1", CellInput::Formula("SUM(A1,  4)".into()))
        .unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1,  4)");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1, 3)");
    book.redo().unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1,  4)");
    book.set(name, "A1", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "B1").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1,  4)");
    book.set(name, "B1", CellInput::Number(6.0)).unwrap();
    assert_eq!(book.cell(name, "C1").unwrap().display, "#N/A");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "B1").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "C1").unwrap().display, "=SUM(A1,  4)");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "C1").unwrap().display, "=SUM(A1,  4)");
        assert_eq!(reopened.cell(name, "B1").unwrap().number, Some(7.0));
    }
}

#[test]
fn hidden_source_formulas_follow_protection_inheritance_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/styles.xml", br#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font/></fonts><fills count="1"><fill/></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" applyProtection="1"><protection locked="1" hidden="1"/></xf></cellXfs></styleSheet>"#.to_vec());
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="2" max="2" style="1"/></cols><sheetData><row r="1"><c r="A1" s="1"><f>1+2</f><v>3</v></c><c r="B1"><f>2+3</f><v>5</v></c><c r="C1" s="0"><f>3+4</f><v>7</v></c></row><row r="2" s="1" customFormat="1"><c r="A2"><f>4+5</f><v>9</v></c><c r="B2" s="0"><f>5+6</f><v>11</v></c></row><row r="3"><c r="A3" s="1"><f>FUTUREFUNC( 1 )</f><v>7</v></c></row></sheetData><sheetProtection sheet="1"/></worksheet>"#.to_vec());
    let mut data = String::new();
    for (row, reference) in [
        (1, "A1"),
        (2, "B1"),
        (3, "C1"),
        (4, "A2"),
        (5, "B2"),
        (6, "A3"),
    ] {
        data.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\"><f>FORMULATEXT('売上サマリ'!{reference})</f></c>"
        ));
        if row == 1 {
            data.push_str("<c r=\"B1\"><f>ISFORMULA('売上サマリ'!A1)</f></c>");
        }
        if row == 2 {
            data.push_str("<c r=\"B2\" s=\"1\"><f>1+2</f></c>");
        }
        data.push_str("</row>");
    }
    data.push_str("<row r=\"7\"><c r=\"A7\"><f>FORMULATEXT(B2)</f></c></row>");
    package.replace("xl/worksheets/sheet2.xml", format!("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{data}</sheetData></worksheet>").into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let source = book.workbook().sheets[0].name.clone();
    let query = book.workbook().sheets[1].name.clone();
    for cell in ["A1", "A2", "A4", "A6"] {
        assert_eq!(book.cell(&query, cell).unwrap().display, "#N/A", "{cell}");
    }
    for (cell, expected) in [
        ("A3", "=3+4"),
        ("A5", "=5+6"),
        ("A7", "=1+2"),
        ("B1", "TRUE"),
    ] {
        assert_eq!(book.cell(&query, cell).unwrap().display, expected, "{cell}");
    }
    assert_eq!(book.cell(&source, "A1").unwrap().number, Some(3.0));
    let protected = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&protected).unwrap(),
        HeadlessWorkbook::open_for_editing(&protected).unwrap(),
    ] {
        assert_eq!(reopened.cell(&query, "A1").unwrap().display, "#N/A");
        assert_eq!(reopened.cell(&query, "A7").unwrap().display, "=1+2");
    }
    assert!(book.unprotect_sheet(&source, "").unwrap());
    assert_eq!(book.cell(&query, "A1").unwrap().display, "=1+2");
    assert_eq!(book.cell(&query, "A6").unwrap().display, "=FUTUREFUNC( 1 )");
    book.undo().unwrap();
    assert_eq!(book.cell(&query, "A1").unwrap().display, "#N/A");
    book.redo().unwrap();
    assert_eq!(book.cell(&query, "A1").unwrap().display, "=1+2");
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(reopened.cell(&query, "A1").unwrap().display, "=1+2");
    assert!(book
        .protect_sheet(&source, Default::default(), "", &[0; 16])
        .unwrap());
    assert_eq!(book.cell(&query, "A1").unwrap().display, "#N/A");
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.cell(&query, "A1").unwrap().display, "#N/A");
}
