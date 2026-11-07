use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook};

fn book() -> HeadlessWorkbook {
    book_with_range("A1:A3")
}

#[test]
fn legacy_members_expose_formula_source_without_replacing_their_values() {
    let mut book = book();
    let sheet = book.workbook().sheets[0].name.clone();
    for (index, at) in ["A1", "A2", "A3"].into_iter().enumerate() {
        let cell = book.cell(&sheet, at).unwrap();
        assert_eq!(cell.formula.as_deref(), Some("{1;2;3}"));
        assert_eq!(cell.number, Some((index + 1) as f64));
        assert!(cell.formula_state.is_some());
        assert_eq!(
            book.formula_state(0, jade_xlsx_reader::CellRef::parse(at).unwrap()),
            cell.formula_state
        );
    }
    let range = book.range(&sheet, "A1:A3").unwrap();
    assert!(range
        .rows
        .iter()
        .all(|row| row[0].formula.as_deref() == Some("{1;2;3}")));
    book.set(&sheet, "D1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    assert!(book.cell(&sheet, "D2").unwrap().formula.is_none());
    assert!(book.cell(&sheet, "D2").unwrap().formula_state.is_none());
}

fn book_with_range(range: &str) -> HeadlessWorkbook {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f t="array" ref="A1:A3">{1;2;3}</f><v>1</v></c><c r="D1"><v>9</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row></sheetData></worksheet>"#;
    package.replace(
        "xl/worksheets/sheet1.xml",
        xml.replace(r#"ref="A1:A3""#, &format!(r#"ref="{range}""#))
            .into_bytes(),
    );
    HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap()
}

#[test]
fn a_single_cell_legacy_array_can_be_replaced_normally() {
    let mut book = book_with_range("A1");
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "A1", CellInput::Number(8.0)).unwrap();
    assert_eq!(book.cell(&sheet, "A1").unwrap().number, Some(8.0));
    assert_eq!(book.cell(&sheet, "A2").unwrap().number, Some(2.0));
    assert!(book.workbook().sheets[0].array_formulas.is_empty());
}

#[test]
fn an_unsupported_large_legacy_array_is_preserved_and_rejects_partial_edits() {
    let mut book = book_with_range("A1:XFD1048576");
    let sheet = book.workbook().sheets[0].name.clone();
    let range = jade_xlsx_reader::CellRange::parse("A1:XFD1048576").unwrap();
    assert_eq!(book.workbook().sheets[0].array_formulas[0].range, range);
    let implicit = book.cell(&sheet, "XFD1048576").unwrap();
    assert_eq!(implicit.formula.as_deref(), Some("{1;2;3}"));
    assert_eq!(
        implicit.formula_state,
        Some(jade_xlsx_headless_api::FormulaState::Uncalculated)
    );
    assert!(implicit.number.is_none());
    let before = book.bytes().to_vec();
    assert!(matches!(
        book.set(&sheet, "A2", CellInput::Blank),
        Err(HeadlessError::PartialLegacyArray { .. })
    ));
    assert_eq!(book.bytes(), before);
    let saved = book.save_bytes().unwrap();
    assert_eq!(
        HeadlessWorkbook::open(&saved).unwrap().workbook().sheets[0].array_formulas[0].range,
        range
    );
}

#[test]
fn partial_legacy_array_value_edits_are_rejected_atomically() {
    for reference in ["A1", "A2", "A3"] {
        let mut book = book();
        let sheet = book.workbook().sheets[0].name.clone();
        let before = book.bytes().to_vec();
        let result = book.set_cells(
            &sheet,
            &[
                ("D1".into(), CellInput::Number(40.0)),
                (reference.into(), CellInput::Blank),
            ],
        );
        assert!(
            matches!(result, Err(HeadlessError::PartialLegacyArray { .. })),
            "partial edit at {reference} must be refused"
        );
        assert_eq!(book.bytes(), before);
        assert_eq!(book.cell(&sheet, "D1").unwrap().number, Some(9.0));
    }
}

#[test]
fn whole_legacy_arrays_can_be_cleared_or_replaced_with_history_and_reopen() {
    for value in [CellInput::Blank, CellInput::Number(8.0)] {
        let mut book = book();
        let sheet = book.workbook().sheets[0].name.clone();
        let values: Vec<_> = ["A1", "A2", "A3"]
            .into_iter()
            .map(|cell| (cell.to_string(), value.clone()))
            .collect();
        book.set_cells(&sheet, &values).unwrap();
        assert!(book.workbook().sheets[0].array_formulas.is_empty());
        book.undo().unwrap();
        assert_eq!(book.cell(&sheet, "A3").unwrap().number, Some(3.0));
        assert_eq!(book.workbook().sheets[0].array_formulas.len(), 1);
        book.redo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert!(reopened.workbook().sheets[0].array_formulas.is_empty());
            assert_eq!(
                reopened.cell(&sheet, "A2").unwrap().number,
                if matches!(value, CellInput::Blank) {
                    None
                } else {
                    Some(8.0)
                }
            );
        }
    }
}

#[test]
fn formatting_individual_legacy_outputs_is_allowed_but_styled_value_edits_are_not() {
    let mut book = book();
    let sheet = book.workbook().sheets[0].name.clone();
    let change = jade_xlsx_headless_editor::FormatChange {
        bold: Some(true),
        ..Default::default()
    };
    book.set_format(&sheet, "A2", &change).unwrap();
    assert_eq!(book.format_at(&sheet, "A2").unwrap().bold, Some(true));
    assert_eq!(book.cell(&sheet, "A2").unwrap().number, Some(2.0));
    let before = book.bytes().to_vec();
    assert!(matches!(
        book.set_cells_styled(&sheet, &[("A2".into(), CellInput::Number(50.0), Some(0))]),
        Err(HeadlessError::PartialLegacyArray { .. })
    ));
    assert_eq!(book.bytes(), before);
}
