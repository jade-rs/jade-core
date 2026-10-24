use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
const SHEET: &str = "売上サマリ";
fn book() -> HeadlessWorkbook {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        p.replace(part, br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    }
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn shown(b: &HeadlessWorkbook, at: &str) -> String {
    b.cell(SHEET, at).unwrap().display
}
fn formula(b: &mut HeadlessWorkbook, at: &str, f: &str) {
    b.set(SHEET, at, CellInput::Formula(f.into())).unwrap();
}

#[test]
fn ceiling_compatibility_name_keeps_sign_zero_precision_and_error_rules() {
    let mut b = book();
    for (i, (f, expected)) in [
        ("ECMA.CEILING(4.3,2)", "6"),
        ("ECMA.CEILING(4.3,-2)", "#NUM!"),
        ("ECMA.CEILING(-4.3,2)", "-4"),
        ("ECMA.CEILING(-4.3,-2)", "-6"),
        ("ECMA.CEILING(4.3,0)", "0"),
        ("ECMA.CEILING(0,-2)", "0"),
        ("ECMA.CEILING(1.5,0.1)", "1.5"),
        ("ECMA.CEILING(\"bad\",2)", "#VALUE!"),
        ("ecma.ceiling(4.3,2)", "6"),
        ("_xlfn.ECMA.CEILING(4.3,2)", "6"),
    ]
    .iter()
    .enumerate()
    {
        let at = format!("A{}", i + 1);
        formula(&mut b, &at, f);
        assert_eq!(shown(&b, &at), *expected, "{f}");
    }
}

#[test]
fn single_scalarizes_arrays_in_standalone_and_aggregate_contexts() {
    let mut b = book();
    for (i, (f, expected)) in [
        ("SINGLE({1,2;3,4})", "1"),
        ("single({\"first\",\"second\"})", "first"),
        ("SUM(_xlfn.SINGLE({1,2;3,4}))", "1"),
        ("_xlfn.single({TRUE,FALSE})", "TRUE"),
        ("_xlfn.SINGLE({#N/A,2})", "#N/A"),
        ("_xlfn.SINGLE(7)", "7"),
        ("SINGLE(SEQUENCE(2,2))", "1"),
        ("SINGLE(NA())", "#N/A"),
    ]
    .iter()
    .enumerate()
    {
        let at = format!("D{}", i + 1);
        formula(&mut b, &at, f);
        assert_eq!(shown(&b, &at), *expected, "{f}");
        assert_eq!(shown(&b, &format!("E{}", i + 1)), "", "SINGLE spilled: {f}");
    }
}

#[test]
fn single_intersects_cross_sheet_ranges_and_scalarizes_reference_arguments() {
    let mut b = book();
    b.set_cells(
        "Notes",
        &[
            ("A1".into(), CellInput::Number(10.0)),
            ("A2".into(), CellInput::Number(20.0)),
            ("A3".into(), CellInput::Number(30.0)),
            ("B1".into(), CellInput::Number(40.0)),
        ],
    )
    .unwrap();
    for (at, f, expected) in [
        ("D2", "SINGLE(Notes!A1:A3)", "20"),
        ("E2", "_xlfn.SINGLE(Notes!A1:A3)", "20"),
        ("B5", "SINGLE(Notes!A1:B1)", "40"),
        ("G8", "SINGLE(Notes!A1)", "10"),
        ("H8", "SINGLE(Notes!A1:A3)", "#VALUE!"),
        ("J2", "ROWS(SINGLE(Notes!A1:A3))", "1"),
        ("K2", "ISREF(SINGLE(Notes!A1:A3))", "FALSE"),
    ] {
        formula(&mut b, at, f);
        assert_eq!(shown(&b, at), expected, "{f}");
    }
}

#[test]
fn saved_operator_spelling_scalarizes_arrays_and_reference_arguments_too() {
    let mut b = book();
    b.set_cells(
        SHEET,
        &[
            ("A1".into(), CellInput::Number(10.0)),
            ("A2".into(), CellInput::Number(20.0)),
            ("A3".into(), CellInput::Number(30.0)),
        ],
    )
    .unwrap();
    formula(&mut b, "D2", "ROWS(_xlfn.SINGLE(A1:A3))");
    formula(&mut b, "E2", "SUM(_xlfn.SINGLE({1,2;3,4}))");
    assert_eq!(shown(&b, "D2"), "1");
    assert_eq!(shown(&b, "E2"), "1");
}

#[test]
fn spill_consumers_evaluate_a_later_anchor_before_reading_its_shape() {
    let mut p = OpcPackage::open(book().bytes()).unwrap();
    p.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>SUM(_xlfn.ANCHORARRAY(Z10))</f><v>0</v></c></row><row r="10"><c r="Z10"><f>_xlfn.SEQUENCE(3)</f><v>0</v></c></row></sheetData></worksheet>"#.to_vec());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    assert_eq!(shown(&b, "A1"), "6");
    formula(&mut b, "B1", "SUM(ANCHORARRAY(Z10))");
    assert_eq!(shown(&b, "B1"), "6");
    formula(&mut b, "C1", "SUM(anchorarray(Z10))");
    assert_eq!(shown(&b, "C1"), "6");
}

#[test]
fn spill_alias_growth_and_shrink_follow_the_anchor_through_save_and_history() {
    let mut b = book();
    b.set(SHEET, "Z9", CellInput::Number(2.0)).unwrap();
    formula(&mut b, "Z10", "SEQUENCE(Z9)");
    formula(&mut b, "A1", "SUM(ANCHORARRAY(Z10))");
    assert_eq!(shown(&b, "A1"), "3");
    let before = b.bytes().to_vec();
    b.set(SHEET, "Z9", CellInput::Number(4.0)).unwrap();
    assert_eq!(shown(&b, "A1"), "10");
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    assert_eq!(shown(&b, "A1"), "3");
    b.redo().unwrap();
    assert_eq!(shown(&b, "A1"), "10");
    b.set(SHEET, "Z9", CellInput::Number(1.0)).unwrap();
    assert_eq!(shown(&b, "A1"), "1");
    let saved = b.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(shown(&reopened, "A1"), "1");
    let xml = OpcPackage::open(&saved)
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(xml.contains("SUM(_xlfn.ANCHORARRAY(Z10))"));
}

#[test]
fn compatibility_call_arities_are_refused_and_saved_prefixes_are_canonical() {
    use jade_xlsx_calc::formula_text::{to_file_formula, wrong_argument_count};
    for f in [
        "SINGLE()",
        "SINGLE(1,2)",
        "ANCHORARRAY()",
        "ANCHORARRAY(A1,B1)",
        "ECMA.CEILING(1)",
    ] {
        assert!(wrong_argument_count(f), "{f}");
    }
    assert_eq!(to_file_formula("SINGLE(A1:A3)"), "_xlfn.SINGLE(A1:A3)");
    assert_eq!(to_file_formula("ANCHORARRAY(A1)"), "_xlfn.ANCHORARRAY(A1)");
    assert_eq!(
        to_file_formula("ECMA.CEILING(4.3,2)"),
        "ECMA.CEILING(4.3,2)"
    );
    assert_eq!(
        to_file_formula("\"SINGLE(A1) ANCHORARRAY(A1)\""),
        "\"SINGLE(A1) ANCHORARRAY(A1)\""
    );
}

#[test]
fn compatibility_names_are_present_in_the_engine_inventory() {
    let names = ironcalc_base::supported_function_names();
    for name in ["ECMA.CEILING", "SINGLE", "ANCHORARRAY"] {
        assert!(names.iter().any(|n| n == name), "{name}");
    }
}

#[test]
fn formulatext_uses_top_left_references_and_refuses_scalarized_values() {
    let mut b = book();
    formula(&mut b, "A1", "1+2");
    b.set("Notes", "A1", CellInput::Formula("5+6".into()))
        .unwrap();
    for (at, f, expected) in [
        ("C1", "FORMULATEXT(A1:B3)", "=1+2"),
        ("C2", "FORMULATEXT(SINGLE(A1:A3))", "#VALUE!"),
        ("C3", "FORMULATEXT(42)", "#VALUE!"),
        ("C4", "FORMULATEXT(Notes!A1:B3)", "=5+6"),
        ("C5", "FORMULATEXT(D9:D10)", "#N/A"),
        ("C6", "FORMULATEXT(A1)", "=1+2"),
        ("C7", "FORMULATEXT(C7)", "=FORMULATEXT(C7)"),
    ] {
        formula(&mut b, at, f);
        assert_eq!(shown(&b, at), expected, "{f}");
    }
}

#[test]
fn imported_compatibility_calls_replace_obsolete_caches_without_spilling_single() {
    let mut p = OpcPackage::open(book().bytes()).unwrap();
    p.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>_xlfn.SINGLE({1,2;3,4})</f><v>99</v></c><c r="B1"><f>ECMA.CEILING(-4.3,-2)</f><v>99</v></c></row></sheetData></worksheet>"#.to_vec());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    assert_eq!(shown(&b, "A1"), "1");
    assert_eq!(shown(&b, "B1"), "-6");
    assert_eq!(shown(&b, "A2"), "");
    let saved = b.save_bytes().unwrap();
    let source = OpcPackage::open(&saved)
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(source.contains("_xlfn.SINGLE({1,2;3,4})"));
    assert!(source.contains("ECMA.CEILING(-4.3,-2)"));
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(shown(&reopened, "A1"), "1");
    assert_eq!(shown(&reopened, "B1"), "-6");
}

#[test]
fn compatibility_operator_cycles_terminate_and_bad_references_report_errors() {
    let mut b = book();
    formula(&mut b, "A1", "SINGLE(A1)");
    assert_eq!(shown(&b, "A1"), "#CIRC!");
    formula(&mut b, "B1", "SUM(ANCHORARRAY(B1))");
    assert_eq!(shown(&b, "B1"), "#CIRC!");
    formula(&mut b, "C1", "ANCHORARRAY(Z20)");
    assert_eq!(shown(&b, "C1"), "#REF!");
    b.set_cells(
        SHEET,
        &[
            (
                "D1".into(),
                CellInput::Formula("SUM(ANCHORARRAY(E1))".into()),
            ),
            (
                "E1".into(),
                CellInput::Formula("SUM(ANCHORARRAY(D1))".into()),
            ),
        ],
    )
    .unwrap();
    assert_eq!(shown(&b, "D1"), "#CIRC!");
    assert_eq!(shown(&b, "E1"), "#CIRC!");
}

#[test]
fn compatibility_names_respect_lambda_bindings_and_ceiling_array_math() {
    let mut b = book();
    formula(&mut b, "A1", "LET(SINGLE,LAMBDA(x,x+1),SINGLE(2))");
    assert_eq!(shown(&b, "A1"), "3");
    formula(
        &mut b,
        "A2",
        "LET(ANCHORARRAY,LAMBDA(x,x+2),ANCHORARRAY(2))",
    );
    assert_eq!(shown(&b, "A2"), "4");
    formula(&mut b, "D1", "ECMA.CEILING({1.1,-1.1},1)");
    assert_eq!(shown(&b, "D1"), "2");
    assert_eq!(shown(&b, "E1"), "-1");
    formula(&mut b, "D5", "ECMA.CEILING({1.1;-1.1},{1,2})");
    assert_eq!(
        [
            shown(&b, "D5"),
            shown(&b, "E5"),
            shown(&b, "D6"),
            shown(&b, "E6")
        ],
        ["2", "2", "-1", "0"]
    );
    formula(
        &mut b,
        "D8",
        "ECMA.CEILING({1.1,\"bad\",-1.1,#N/A},{1,1,-1,1})",
    );
    assert_eq!(
        [
            shown(&b, "D8"),
            shown(&b, "E8"),
            shown(&b, "F8"),
            shown(&b, "G8")
        ],
        ["2", "#VALUE!", "-2", "#N/A"]
    );
    formula(&mut b, "D11", "CEILING({1.1,-1.1},1)");
    assert_eq!([shown(&b, "D11"), shown(&b, "E11")], ["2", "-1"]);
    formula(
        &mut b,
        "D14",
        "ECMA.CEILING(SEQUENCE(1001),SEQUENCE(1,1001))",
    );
    assert_eq!(
        shown(&b, "D14"),
        "#ERROR!",
        "engine's existing million-cell matrix limit"
    );
    formula(&mut b, "A24", &format!("\"{}\"", "x".repeat(8190)));
    formula(&mut b, "D24", "FORMULATEXT(A24)");
    assert_eq!(shown(&b, "D24"), "#N/A");
    formula(&mut b, "A25", &format!("\"{}\"", "x".repeat(8189)));
    formula(&mut b, "D25", "FORMULATEXT(A25)");
    assert_eq!(shown(&b, "D25").len(), 8192);
}

#[test]
fn spill_calls_require_an_array_anchor_and_propagate_errors() {
    let mut b = book();
    formula(&mut b, "A1", "1+1");
    formula(&mut b, "B1", "ANCHORARRAY(A1)");
    assert_eq!(shown(&b, "B1"), "#REF!");
    for (at, f, expected) in [
        ("C1", "ANCHORARRAY(7)", "#REF!"),
        ("D1", "ANCHORARRAY(A1:A2)", "#REF!"),
        ("E1", "ANCHORARRAY(NA())", "#N/A"),
    ] {
        formula(&mut b, at, f);
        assert_eq!(shown(&b, at), expected, "{f}");
    }
    formula(&mut b, "A3", "1/0");
    formula(&mut b, "B3", "ANCHORARRAY(A3)");
    assert_eq!(shown(&b, "B3"), "#DIV/0!");
    formula(&mut b, "G1", "SEQUENCE(1)");
    formula(&mut b, "H1", "SUM(ANCHORARRAY(G1))");
    assert_eq!(shown(&b, "H1"), "1");
    b.set(SHEET, "J2", CellInput::Number(99.0)).unwrap();
    formula(&mut b, "J1", "SEQUENCE(3)");
    formula(&mut b, "K1", "SUM(ANCHORARRAY(J1))");
    assert_eq!(shown(&b, "K1"), "#SPILL!");
    b.set(SHEET, "J2", CellInput::Blank).unwrap();
    assert_eq!(shown(&b, "K1"), "6");
}
