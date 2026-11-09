use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn error_classification_and_parity_arrays_keep_error_anchors_through_history() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    book.set(name, "B2", CellInput::Number(2.0)).unwrap();
    book.set(name, "B3", CellInput::Error("#N/A".into()))
        .unwrap();
    book.set(name, "B6", CellInput::Bool(true)).unwrap();
    for (at, formula) in [
        ("A1", "ISEVEN(B1:B3)"),
        ("C1", "ERROR.TYPE(B1:B3)"),
        ("D1", "SUM(IFERROR(C1#,0))"),
        ("E1", "SUM(IFERROR(N(A1#),0))"),
        ("A6", "FILTER({#N/A;2},{1;1}*B6)"),
        ("D6", "IFERROR(ROWS(A6#),99)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(7.0));
    assert_eq!(book.cell(name, "E1").unwrap().number, Some(1.0));
    assert_eq!(book.cell(name, "D6").unwrap().number, Some(2.0));
    book.set(name, "B2", CellInput::Error("#DIV/0!".into()))
        .unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "#DIV/0!");
    assert_eq!(book.cell(name, "C2").unwrap().number, Some(2.0));
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(9.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "TRUE");
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(7.0));
    book.redo().unwrap();
    book.set(name, "B6", CellInput::Bool(false)).unwrap();
    assert_eq!(book.cell(name, "D6").unwrap().number, Some(99.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "D6").unwrap().number, Some(2.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "C2").unwrap().number, Some(2.0));
        assert_eq!(reopened.cell(name, "D1").unwrap().number, Some(9.0));
        assert_eq!(reopened.cell(name, "D6").unwrap().number, Some(99.0));
    }
}

#[test]
fn textjoin_whole_rows_and_columns_recalculate_without_losing_blank_cells() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Text("x".into())).unwrap();
    book.set(name, "B3", CellInput::Text("y".into())).unwrap();
    for (at, formula) in [
        ("A1", "TEXTJOIN(\"\",FALSE,B:B)"),
        ("C1", "LEN(TEXTJOIN(\"-\",FALSE,2:2))"),
        ("D1", "TEXTJOIN(\",\",FALSE,B:B)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "A1").unwrap().display, "xy");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(16383.0));
    assert_eq!(book.cell(name, "D1").unwrap().display, "#VALUE!");
    book.set(name, "B2", CellInput::Text("z".into())).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "xzy");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(16384.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "xy");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(16383.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "xzy");
        assert_eq!(reopened.cell(name, "C1").unwrap().number, Some(16384.0));
        assert_eq!(reopened.cell(name, "D1").unwrap().display, "#VALUE!");
    }
}

#[test]
fn textjoin_delimiter_ranges_keep_blank_delimiters_and_recalculate_after_edits() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    for (cell, value) in [
        ("B1", "-"),
        ("C1", "/"),
        ("B3", "a"),
        ("C3", "b"),
        ("D3", "c"),
        ("E3", "d"),
        ("F3", "e"),
    ] {
        book.set(name, cell, CellInput::Text(value.into())).unwrap();
    }
    book.set(
        name,
        "A1",
        CellInput::Formula("TEXTJOIN(B1:D1,TRUE,B3:F3)".into()),
    )
    .unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "a-b/cd-e");
    book.set(name, "D1", CellInput::Text(":".into())).unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "a-b/c:d-e");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A1").unwrap().display, "a-b/cd-e");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A1").unwrap().display, "a-b/c:d-e");
    }
}

#[test]
fn n_t_range_spills_and_textjoin_survive_edit_history_and_save() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(2.0)).unwrap();
    book.set(name, "B2", CellInput::Text("2".into())).unwrap();
    book.set(name, "B3", CellInput::Bool(true)).unwrap();
    for (at, formula) in [
        ("A1", "N(B1:B4)"),
        ("C1", "T(B1:B4)"),
        ("D1", "SUM(A1#)"),
        ("E1", "TEXTJOIN(\"|\",TRUE,C1#)"),
    ] {
        book.set(name, at, CellInput::Formula(formula.into()))
            .unwrap();
    }
    assert_eq!(book.cell(name, "A2").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "A4").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "C4").unwrap().display, "");
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(3.0));
    assert_eq!(book.cell(name, "E1").unwrap().display, "2");
    book.set(name, "B2", CellInput::Number(5.0)).unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().number, Some(5.0));
    assert_eq!(book.cell(name, "D1").unwrap().number, Some(8.0));
    assert_eq!(book.cell(name, "E1").unwrap().display, "");
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().number, Some(0.0));
    assert_eq!(book.cell(name, "E1").unwrap().display, "2");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "D1").unwrap().number, Some(8.0));
        assert_eq!(reopened.cell(name, "E1").unwrap().display, "");
        assert_eq!(reopened.cell(name, "A4").unwrap().number, Some(0.0));
    }
}

#[test]
fn information_array_spills_recalculate_save_and_follow_history() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let name = "売上サマリ";
    book.set(name, "B1", CellInput::Number(1.0)).unwrap();
    book.set(name, "B2", CellInput::Text("1".into())).unwrap();
    book.set(name, "B3", CellInput::Bool(true)).unwrap();
    book.set(name, "A1", CellInput::Formula("ISNUMBER(B1:B3)".into()))
        .unwrap();
    book.set(
        name,
        "C1",
        CellInput::Formula("SUM(FILTER(B1:B3,ISNUMBER(B1:B3)))".into()),
    )
    .unwrap();
    for (cell, display) in [("A1", "TRUE"), ("A2", "FALSE"), ("A3", "FALSE")] {
        assert_eq!(book.cell(name, cell).unwrap().display, display);
    }
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1.0));
    book.set(name, "B2", CellInput::Number(3.0)).unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "TRUE");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(4.0));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "A2").unwrap().display, "FALSE");
    assert_eq!(book.cell(name, "C1").unwrap().number, Some(1.0));
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        assert_eq!(reopened.cell(name, "A2").unwrap().display, "TRUE");
        assert_eq!(reopened.cell(name, "A3").unwrap().display, "FALSE");
        assert_eq!(reopened.cell(name, "C1").unwrap().number, Some(4.0));
    }
}
