//! Evaluating a workbook's formulas against a projection of the model.

use std::io::Write;

use jade_xlsx_calc::evaluate;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

#[test]
fn subtotal_hidden_row_modes_use_imported_visibility_for_every_summary_function() {
    let expected = [
        25.0,
        2.0,
        2.0,
        30.0,
        20.0,
        600.0,
        50.0_f64.sqrt(),
        5.0,
        50.0,
        50.0,
        25.0,
    ];
    let mut xml = String::from(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2" hidden="1"><c r="A2"><v>10</v></c></row><row r="3"><c r="A3"><v>20</v></c></row><row r="4"><c r="A4"><v>30</v></c></row><row r="6">"#,
    );
    for code in 101..=111 {
        let column = char::from(b'B' + (code - 101) as u8);
        xml.push_str(&format!(
            "<c r=\"{column}6\"><f>SUBTOTAL({code},A2:A4)</f><v>0</v></c>"
        ));
    }
    xml.push_str(r#"<c r="M6"><f>SUBTOTAL(9,A2:A4)</f><v>0</v></c><c r="N6"><f>SUM(A2:A4)</f><v>0</v></c></row></sheetData></worksheet>"#);
    let book = read_xlsx(&workbook(&xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert!(result.skipped.is_empty());
    for (index, expected) in expected.into_iter().enumerate() {
        let reference = CellRef::parse(&format!("{}6", char::from(b'B' + index as u8))).unwrap();
        let cell = result
            .cells
            .iter()
            .find(|cell| cell.reference == reference)
            .unwrap();
        assert!(
            matches!(cell.value, CellValue::Number(value) if (value - expected).abs() < 1e-9),
            "{reference:?}: {:?} != {expected}",
            cell.value
        );
    }
    for reference in ["M6", "N6"] {
        assert_eq!(
            result
                .cells
                .iter()
                .find(|cell| cell.reference == CellRef::parse(reference).unwrap())
                .unwrap()
                .value,
            CellValue::Number(60.0)
        );
    }
}

#[test]
fn subtotal_respects_default_hidden_rows_and_explicit_visible_exceptions_but_not_hidden_columns() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="15" zeroHeight="1"/><cols><col min="1" max="1" hidden="1"/></cols><sheetData><row r="2" hidden="0"><c r="A2"><v>10</v></c></row><row r="3"><c r="A3"><v>20</v></c></row><row r="4" hidden="0"><c r="A4"><v>30</v></c></row><row r="6" hidden="0"><c r="B6"><f>SUBTOTAL(109,A2:A4)</f><v>0</v></c><c r="C6"><f>SUBTOTAL(9,A2:A4)</f><v>0</v></c></row></sheetData></worksheet>"#;
    let book = read_xlsx(&workbook(xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert!(result.skipped.is_empty());
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("B6").unwrap())
            .unwrap()
            .value,
        CellValue::Number(40.0)
    );
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("C6").unwrap())
            .unwrap()
            .value,
        CellValue::Number(60.0)
    );
}

#[test]
fn subtotal_hidden_rows_follow_cross_sheet_references_and_unsupported_formula_reprojections() {
    let first = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>SUBTOTAL(109,Sheet2!A1:A3)</f><v>0</v></c><c r="B1"><f>UNSUPPORTED_FOR_COMPAT()</f><v>7</v></c><c r="C1"><f>A1+B1</f><v>0</v></c></row></sheetData></worksheet>"#;
    let second = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>10</v></c></row><row r="2" hidden="1"><c r="A2"><v>20</v></c></row><row r="3"><c r="A3"><v>30</v></c></row></sheetData></worksheet>"#;
    let book = read_xlsx(&workbook(first, Some(second), false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("A1").unwrap())
            .unwrap()
            .value,
        CellValue::Number(40.0)
    );
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("C1").unwrap())
            .unwrap()
            .value,
        CellValue::Number(47.0)
    );
}

#[test]
fn single_cell_subtotals_obey_row_visibility_and_nested_subtotals_are_not_counted_twice() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2" hidden="1"><c r="A2"><v>10</v></c></row><row r="3"><c r="A3"><v>20</v></c></row><row r="4"><c r="A4"><v>30</v></c></row><row r="5"><c r="A5"><f>SUBTOTAL(109,A2:A4)</f><v>50</v></c></row><row r="6"><c r="B6"><f>SUBTOTAL(109,A2)</f><v>0</v></c><c r="C6"><f>SUBTOTAL(103,A2)</f><v>0</v></c><c r="D6"><f>SUBTOTAL(109,A2:A5)</f><v>50</v></c></row></sheetData></worksheet>"#;
    let book = read_xlsx(&workbook(xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert!(result.skipped.is_empty());
    for (reference, value) in [("B6", 0.0), ("C6", 0.0), ("D6", 50.0)] {
        assert_eq!(
            result
                .cells
                .iter()
                .find(|cell| cell.reference == CellRef::parse(reference).unwrap())
                .unwrap()
                .value,
            CellValue::Number(value)
        );
    }
}

#[test]
fn saved_worksheet_filters_exclude_hidden_body_rows_even_outside_the_filtered_columns() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetPr filterMode="1"/><sheetData><row r="1" hidden="1"><c r="D1"><v>5</v></c></row><row r="2" hidden="1"><c r="D2"><v>10</v></c></row><row r="3"><c r="D3"><v>20</v></c></row><row r="4" hidden="1"><c r="D4"><v>30</v></c></row><row r="6" hidden="1"><c r="D6"><v>40</v></c></row><row r="7"><c r="A7"><f>SUBTOTAL(9,D1:D6)</f><v>0</v></c><c r="B7"><f>SUBTOTAL(109,D1:D6)</f><v>0</v></c></row></sheetData><autoFilter ref="A1:C4"><filterColumn colId="0"><filters><filter val="Alpha"/></filters></filterColumn></autoFilter></worksheet>"#;
    let book = read_xlsx(&workbook(xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert!(result.skipped.is_empty());
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("A7").unwrap())
            .unwrap()
            .value,
        CellValue::Number(65.0)
    );
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("B7").unwrap())
            .unwrap()
            .value,
        CellValue::Number(20.0)
    );
}

#[test]
fn unknown_saved_filter_ranges_preserve_full_subtotal_caches_but_calculate_hidden_modes() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetPr filterMode="1"/><sheetData><row r="2" hidden="1"><c r="A2"><v>10</v></c></row><row r="3"><c r="A3"><v>20</v></c></row><row r="4"><c r="A4"><f>SUBTOTAL(9,A2:A3)</f><v>7</v></c><c r="B4"><f>SUBTOTAL(109,A2:A3)</f><v>0</v></c><c r="C4"><f>A4+1</f><v>0</v></c></row></sheetData></worksheet>"#;
    let book = read_xlsx(&workbook(xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert_eq!(result.skipped.len(), 1);
    assert!(result
        .cells
        .iter()
        .all(|cell| cell.reference != CellRef::parse("A4").unwrap()));
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("B4").unwrap())
            .unwrap()
            .value,
        CellValue::Number(20.0)
    );
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::parse("C4").unwrap())
            .unwrap()
            .value,
        CellValue::Number(8.0)
    );
}

fn workbook(sheet1: &str, sheet2: Option<&str>, date_1904: bool) -> Vec<u8> {
    let workbook_pr = if date_1904 {
        r#"<workbookPr date1904="1"/>"#
    } else {
        ""
    };
    let mut sheets = String::from(r#"<sheet name="売上" sheetId="1" r:id="rId1"/>"#);
    if sheet2.is_some() {
        sheets.push_str(r#"<sheet name="Sheet2" sheetId="2" r:id="rId2"/>"#);
    }
    let workbook_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{workbook_pr}<sheets>{sheets}</sheets></workbook>"#
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    );
    if sheet2.is_some() {
        rels.push_str(r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>"#);
    }
    rels.push_str("</Relationships>");

    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options = SimpleFileOptions::default();
        let mut parts = vec![
            ("_rels/.rels", root_rels.to_string()),
            ("xl/workbook.xml", workbook_xml),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/worksheets/sheet1.xml", sheet1.to_string()),
        ];
        if let Some(second) = sheet2 {
            parts.push(("xl/worksheets/sheet2.xml", second.to_string()));
        }
        for (name, contents) in parts {
            writer.start_file(name, options).expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

fn sheet(rows: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    )
}

fn computed<'a>(
    outcome: &'a jade_xlsx_calc::Recalculated,
    sheet: usize,
    reference: &str,
) -> Option<&'a CellValue> {
    let reference = CellRef::parse(reference).expect("reference");
    outcome
        .cells
        .iter()
        .find(|cell| cell.sheet == sheet && cell.reference == reference)
        .map(|cell| &cell.value)
}

#[test]
fn a_stale_sum_is_recomputed_from_current_values() {
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><v>2</v></c><c r="B1"><v>3</v></c><c r="C1"><f>SUM(A1:B1)</f><v>99</v></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    assert_eq!(computed(&outcome, 0, "C1"), Some(&CellValue::Number(5.0)));
    assert!(outcome.skipped.is_empty());
}

#[test]
fn text_and_boolean_results_come_back_typed() {
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><v>2</v></c><c r="B1"><f>IF(A1&gt;1,"big","small")</f><v>old</v></c><c r="C1"><f>A1=2</f></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    assert_eq!(
        computed(&outcome, 0, "B1"),
        Some(&CellValue::Text("big".into()))
    );
    assert_eq!(computed(&outcome, 0, "C1"), Some(&CellValue::Bool(true)));
}

#[test]
fn an_error_result_reads_as_the_error_it_is() {
    let bytes = workbook(
        &sheet(r#"<row r="1"><c r="A1"><f>1/0</f><v>7</v></c></row>"#),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    assert_eq!(
        computed(&outcome, 0, "A1"),
        Some(&CellValue::Error("#DIV/0!".into()))
    );
}

#[test]
fn a_cross_sheet_reference_sees_the_other_sheet() {
    let bytes = workbook(
        &sheet(r#"<row r="1"><c r="A1"><v>2</v></c></row>"#),
        Some(&sheet(
            r#"<row r="1"><c r="A1"><f>売上!A1*10</f><v>0</v></c></row>"#,
        )),
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    assert_eq!(computed(&outcome, 1, "A1"), Some(&CellValue::Number(20.0)));
}

#[test]
fn an_unknown_function_is_skipped_rather_than_guessed() {
    // `#NAME?` from the engine would overwrite a cached value Excel computed
    // with something jade merely failed to understand, so the cell is skipped
    // and keeps what the file said.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><f>NOTAFUNCTION.V2(1)</f><v>7</v></c><c r="B1"><f>A1+1</f><v>8</v></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    assert_eq!(computed(&outcome, 0, "A1"), None);
    assert_eq!(
        outcome
            .skipped
            .iter()
            .map(|cell| cell.reference)
            .collect::<Vec<_>>(),
        vec![CellRef::parse("A1").expect("A1")]
    );
    // The dependent still evaluates against the skipped cell's cached value.
    assert_eq!(computed(&outcome, 0, "B1"), Some(&CellValue::Number(8.0)));
}

#[test]
fn an_unknown_function_with_nothing_cached_shows_name_error() {
    // A formula just typed has no value Excel computed to keep: a misspelt
    // function reads `#NAME?`, as in Excel, not an empty cell. It is still
    // reported, so the file asks Excel to calculate it on load.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><f>NOTAFUNCTION.V2(1)</f></c><c r="B1"><f>SUMM(1,2)</f></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    let name = CellValue::Error("#NAME?".into());
    assert_eq!(computed(&outcome, 0, "A1"), Some(&name));
    assert_eq!(computed(&outcome, 0, "B1"), Some(&name));
    assert_eq!(skipped(&outcome), refs(&["A1", "B1"]));
}

#[test]
fn a_name_error_that_shows_reaches_its_dependents() {
    // B1 last computed 8, but A1 now reads `#NAME?` for real, so B1 does
    // too; keeping 8 would show a total that no longer adds up.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><f>NOTAFUNCTION.V2(1)</f></c><c r="B1"><f>A1+1</f><v>8</v></c><c r="C1"><f>SUM(A1:B1)</f><v>15</v></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");

    let name = CellValue::Error("#NAME?".into());
    assert_eq!(computed(&outcome, 0, "A1"), Some(&name));
    assert_eq!(computed(&outcome, 0, "B1"), Some(&name));
    assert_eq!(computed(&outcome, 0, "C1"), Some(&name));
    // Only the cell whose own formula failed is reported.
    assert_eq!(skipped(&outcome), refs(&["A1"]));
}

#[test]
fn a_skipped_name_error_stays_skipped_while_the_file_awaits_a_full_calculation() {
    // The `#NAME?` jade wrote into A1's cache last time is jade's, not
    // Excel's: with the file still asking for a full calculation, A1 stays
    // skipped and the request stands.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1" t="e"><f>NOTAFUNCTION.V2(1)</f><v>#NAME?</v></c><c r="B1"><f>A1+1</f><v>#NAME?</v></c></row>"#,
        ),
        None,
        false,
    );
    let mut workbook = read_xlsx(&bytes).expect("read");
    workbook.full_calculation_on_load = true;
    let outcome = evaluate(&workbook).expect("evaluate");
    assert_eq!(skipped(&outcome), refs(&["A1"]));
    assert_eq!(
        computed(&outcome, 0, "B1"),
        Some(&CellValue::Error("#NAME?".into()))
    );
    // Saved by Excel, with nothing pending, the same cache is Excel's answer.
    workbook.full_calculation_on_load = false;
    assert!(evaluate(&workbook).expect("evaluate").skipped.is_empty());
}

#[test]
fn an_error_value_reaches_the_cells_that_read_it() {
    // A cell holding `#N/A` makes a sum over it `#N/A` in Excel, not a sum
    // that leaves it out.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1" t="e"><v>#N/A</v></c><c r="B1"><v>2</v></c><c r="C1"><f>SUM(A1:B1)</f><v>2</v></c><c r="D1"><f>IFERROR(A1,0)+B1</f><v>2</v></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");
    assert_eq!(
        computed(&outcome, 0, "C1"),
        Some(&CellValue::Error("#N/A".into()))
    );
    assert_eq!(computed(&outcome, 0, "D1"), Some(&CellValue::Number(2.0)));
}

fn skipped(outcome: &jade_xlsx_calc::Recalculated) -> Vec<CellRef> {
    outcome.skipped.iter().map(|cell| cell.reference).collect()
}

fn refs(references: &[&str]) -> Vec<CellRef> {
    references
        .iter()
        .map(|reference| CellRef::parse(reference).expect("reference"))
        .collect()
}

#[test]
fn a_1904_workbook_recalculates_ordinary_arithmetic() {
    let bytes = workbook(
        &sheet(r#"<row r="1"><c r="A1"><f>1+1</f></c></row>"#),
        None,
        true,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let result = evaluate(&workbook).unwrap();
    assert_eq!(computed(&result, 0, "A1"), Some(&CellValue::Number(2.0)));
    assert!(result.skipped.is_empty());
}

fn formula_sheet(formulas: &[&str]) -> String {
    sheet(
        &formulas
            .iter()
            .enumerate()
            .map(|(index, formula)| {
                let row = index + 1;
                let formula = formula.replace('&', "&amp;").replace('<', "&lt;");
                format!("<row r=\"{row}\"><c r=\"A{row}\"><f>{formula}</f></c></row>")
            })
            .collect::<String>(),
    )
}

fn assert_formulas(formulas: &[&str], expected: &[CellValue], date1904: bool) {
    let bytes = workbook(&formula_sheet(formulas), None, date1904);
    let result = evaluate(&read_xlsx(&bytes).unwrap()).unwrap();
    assert!(result.skipped.is_empty(), "{:?}", result.skipped);
    for (index, expected) in expected.iter().enumerate() {
        assert_eq!(
            computed(&result, 0, &format!("A{}", index + 1)),
            Some(expected),
            "{}",
            formulas[index]
        );
    }
}

#[test]
fn date_construction_components_and_plain_numbers_share_the_workbook_epoch() {
    use CellValue::Number as N;
    assert_formulas(
        &[
            "DATE(1904,1,1)",
            "DATE(1904,2,29)",
            "DATE(1904,3,1)",
            "YEAR(A1)",
            "MONTH(A2)",
            "DAY(A2)",
            "DAY(A3)",
            "DATE(2024,1,1)",
            "A8+1",
            "SUM(A1,A2,10)",
            "HOUR(A1+0.5)",
        ],
        &[
            N(0.0),
            N(59.0),
            N(60.0),
            N(1904.0),
            N(2.0),
            N(29.0),
            N(1.0),
            N(43830.0),
            N(43831.0),
            N(69.0),
            N(12.0),
        ],
        true,
    );
}

#[test]
fn date_text_coercion_and_text_formatting_use_the_same_epoch() {
    use CellValue::{Number as N, Text as T};
    assert_formulas(
        &[
            "DATEVALUE(\"1904-01-01\")",
            "TEXT(A1,\"yyyy-mm-dd\")",
            "DATEVALUE(\"2024-01-01\")",
            "TEXT(A3,\"yyyy-mm-dd\")",
            "VALUE(\"1/1/1904\")",
            "TEXT(12.5,\"0.00\")",
            "N(\"DATE(1904,1,1)\")",
        ],
        &[
            N(0.0),
            T("1904-01-01".into()),
            N(43830.0),
            T("2024-01-01".into()),
            N(0.0),
            T("12.50".into()),
            N(0.0),
        ],
        true,
    );
}

#[test]
fn date_rollovers_workdays_and_financial_dates_are_not_1900_serials() {
    use CellValue::Number as N;
    assert_formulas(
        &[
            "EDATE(0,1)",
            "EOMONTH(0,1)",
            "WEEKDAY(0)",
            "NETWORKDAYS(0,6)",
            "WORKDAY(0,1)",
            "DAYS360(0,DATE(1904,12,31))",
            "YEARFRAC(DATE(2024,1,1),DATE(2025,1,1),1)",
            "COUPNCD(DATE(2024,3,1),DATE(2025,1,1),2,0)",
            "COUPPCD(DATE(2024,3,1),DATE(2025,1,1),2,0)",
        ],
        &[
            N(31.0),
            N(59.0),
            N(6.0),
            N(5.0),
            N(3.0),
            N(360.0),
            N(1.0),
            N(44012.0),
            N(43830.0),
        ],
        true,
    );
}

#[test]
fn the_1904_upper_bound_and_zero_serial_are_validated_in_that_epoch() {
    use CellValue::{Error as E, Number as N};
    assert_formulas(
        &[
            "DATE(9999,12,31)",
            "YEAR(A1)",
            "YEAR(A1+1)",
            "YEAR(-1)",
            "YEAR(0)",
        ],
        &[
            N(2957003.0),
            N(9999.0),
            E("#NUM!".into()),
            E("#NUM!".into()),
            N(1904.0),
        ],
        true,
    );
}

#[test]
fn sequential_and_concurrent_workbooks_do_not_leak_their_date_epoch() {
    let formulas = ["DATE(2024,1,1)", "TEXT(A1,\"yyyy-mm-dd\")"];
    let check = |date1904| {
        assert_formulas(
            &formulas,
            &[
                CellValue::Number(if date1904 { 43830.0 } else { 45292.0 }),
                CellValue::Text("2024-01-01".into()),
            ],
            date1904,
        )
    };
    check(true);
    check(false);
    check(true);
    std::thread::scope(|scope| {
        scope.spawn(|| check(false));
        scope.spawn(|| check(true));
    });
}

#[test]
fn today_and_now_use_the_1904_epoch_in_the_native_clock_backend() {
    let seconds = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as f64
    };
    let before = seconds() / 86400.0 + 24107.0;
    let bytes = workbook(&formula_sheet(&["TODAY()", "NOW()"]), None, true);
    let result = evaluate(&read_xlsx(&bytes).unwrap()).unwrap();
    let after = seconds() / 86400.0 + 24107.0;
    let Some(CellValue::Number(today)) = computed(&result, 0, "A1") else {
        panic!("TODAY did not calculate")
    };
    let Some(CellValue::Number(now)) = computed(&result, 0, "A2") else {
        panic!("NOW did not calculate")
    };
    assert!(*today >= before.floor() && *today <= after.floor());
    assert!(*now >= before - 1e-9 && *now <= after + 1e-9);
}

#[test]
fn configured_time_zones_cross_midnight_in_both_date_epochs() {
    use jade_xlsx_calc::evaluate_in_time_zone;
    let formulas = [
        "TODAY()",
        "NOW()",
        "TEXT(NOW(),\"yyyy-mm-dd hh:mm:ss\")",
        "NOW()-NOW()",
    ];
    for epoch in [false, true] {
        let bytes = workbook(&formula_sheet(&formulas), None, epoch);
        let book = read_xlsx(&bytes).unwrap();
        for (zone, day, fraction, text) in [
            ("UTC", 46204.0, 2.5 / 24.0, "2026-07-01 02:30:00"),
            ("Asia/Tokyo", 46204.0, 11.5 / 24.0, "2026-07-01 11:30:00"),
            (
                "America/Los_Angeles",
                46203.0,
                19.5 / 24.0,
                "2026-06-30 19:30:00",
            ),
        ] {
            let result = ironcalc_base::clock::with_milliseconds(1782873000000, || {
                evaluate_in_time_zone(&book, zone)
            })
            .unwrap();
            let day = day - if epoch { 1462.0 } else { 0.0 };
            assert_eq!(
                computed(&result, 0, "A1"),
                Some(&CellValue::Number(day)),
                "{zone}"
            );
            assert_eq!(
                computed(&result, 0, "A2"),
                Some(&CellValue::Number(day + fraction)),
                "{zone}"
            );
            assert_eq!(
                computed(&result, 0, "A3"),
                Some(&CellValue::Text(text.into())),
                "{zone}"
            );
            assert_eq!(
                computed(&result, 0, "A4"),
                Some(&CellValue::Number(0.0)),
                "{zone}"
            );
        }
    }
}

#[test]
fn local_clocks_follow_dst_transitions_without_leaking_configuration() {
    let book = read_xlsx(&workbook(
        &formula_sheet(&["TEXT(NOW(),\"yyyy-mm-dd hh:mm:ss\")"]),
        None,
        false,
    ))
    .unwrap();
    for (instant, expected) in [
        (1772953140000, "2026-03-08 01:59:00"),
        (1772953200000, "2026-03-08 03:00:00"),
        (1793512740000, "2026-11-01 01:59:00"),
        (1793512800000, "2026-11-01 01:00:00"),
    ] {
        let result = ironcalc_base::clock::with_milliseconds(instant, || {
            jade_xlsx_calc::evaluate_in_time_zone(&book, "America/New_York")
        })
        .unwrap();
        assert_eq!(
            computed(&result, 0, "A1"),
            Some(&CellValue::Text(expected.into()))
        );
    }
    assert!(jade_xlsx_calc::validate_time_zone("Not/AZone").is_err());
    assert!(jade_xlsx_calc::evaluate_in_time_zone(&book, "Not/AZone").is_err());
    let result =
        ironcalc_base::clock::with_milliseconds(1782873000000, || evaluate(&book)).unwrap();
    assert_eq!(
        computed(&result, 0, "A1"),
        Some(&CellValue::Text("2026-07-01 02:30:00".into()))
    );
}

#[test]
fn text_carries_rounded_seconds_into_minutes_and_the_next_date() {
    assert_formulas(
        &[
            "TEXT(46203+86399.75/86400,\"yyyy-mm-dd hh:mm:ss\")",
            "TEXT(46203+86399.75/86400,\"yyyy-mm-dd\")",
            "TEXT(1.25,\"[h]:mm:ss\")",
        ],
        &[
            CellValue::Text("2026-07-01 00:00:00".into()),
            CellValue::Text("2026-06-30".into()),
            CellValue::Text("30:00:00".into()),
        ],
        false,
    );
}

#[test]
fn structured_references_to_a_table_recalculate() {
    // `SUBTOTAL(109,Table1[Amount])` in Table1's totals row, as Excel
    // writes it: it evaluates rather than being skipped, and follows an
    // edit to the column it reads.
    let bytes = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/objects-workbook.xlsx"),
    )
    .expect("fixture");
    let mut workbook = read_xlsx(&bytes).expect("read");
    // Typed beside the table: a column, and the totals row's cell.
    for (reference, formula) in [
        ("E1", "SUM(Table1[Qty])"),
        ("E2", "Table1[[#Totals],[Amount]]*2"),
    ] {
        let reference = CellRef::parse(reference).expect("reference");
        workbook.sheets[0]
            .rows
            .iter_mut()
            .find(|row| row.index == reference.row)
            .expect("row")
            .cells
            .push(jade_xlsx_reader::Cell {
                reference,
                value: CellValue::Empty,
                style_index: None,
                formula: Some(formula.to_string()),
            });
    }
    let outcome = evaluate(&workbook).expect("evaluate");
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!(computed(&outcome, 0, "C5"), Some(&CellValue::Number(749.0)));
    assert_eq!(computed(&outcome, 0, "E1"), Some(&CellValue::Number(6.0)));
    assert_eq!(
        computed(&outcome, 0, "E2"),
        Some(&CellValue::Number(1498.0))
    );

    let gizmo = CellRef::parse("C4").expect("C4");
    let cell = workbook.sheets[0]
        .rows
        .iter_mut()
        .flat_map(|row| row.cells.iter_mut())
        .find(|cell| cell.reference == gizmo)
        .expect("C4");
    cell.value = CellValue::Number(199.0);
    let outcome = evaluate(&workbook).expect("evaluate");
    assert_eq!(computed(&outcome, 0, "C5"), Some(&CellValue::Number(849.0)));
    assert_eq!(
        computed(&outcome, 0, "E2"),
        Some(&CellValue::Number(1698.0))
    );
}

#[test]
fn a_dynamic_array_from_excel_keeps_its_spill() {
    // As Excel saves `=SEQUENCE(3)` in A1: the anchor carries the formula
    // and the range it spills to; A2 and A3 hold the spilled values, and a
    // SUM reads all three.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(3)</f><v>1</v></c><c r="B1"><f>SUM(A1:A3)</f><v>6</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    assert_eq!(workbook.sheets[0].array_formulas.len(), 1);
    let outcome = evaluate(&workbook).expect("evaluate");
    // Not #SPILL!: the spilled values are the formula's own, not a wall.
    assert_eq!(computed(&outcome, 0, "A1"), Some(&CellValue::Number(1.0)));
    assert_eq!(computed(&outcome, 0, "B1"), Some(&CellValue::Number(6.0)));
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    let spilled: Vec<_> = outcome
        .spilled
        .iter()
        .map(|cell| (cell.reference, cell.value.clone()))
        .collect();
    assert_eq!(
        spilled,
        [
            (CellRef::parse("A2").unwrap(), CellValue::Number(2.0)),
            (CellRef::parse("A3").unwrap(), CellValue::Number(3.0)),
        ]
    );
}

#[test]
fn a_spill_follows_its_inputs_and_a_typed_one_spills_too() {
    // C1 says how many; the anchor has no array range in the file, as when
    // the formula was just typed.
    let bytes = workbook(
        &sheet(
            r#"<row r="1"><c r="A1"><f>_xlfn.SEQUENCE(C1,1,10)</f></c><c r="C1"><v>4</v></c><c r="D1"><f>SUM(A1#)</f></c></row>"#,
        ),
        None,
        false,
    );
    let workbook = read_xlsx(&bytes).expect("read");
    let outcome = evaluate(&workbook).expect("evaluate");
    assert_eq!(computed(&outcome, 0, "A1"), Some(&CellValue::Number(10.0)));
    assert_eq!(computed(&outcome, 0, "D1"), Some(&CellValue::Number(46.0)));
    let spilled: Vec<_> = outcome
        .spilled
        .iter()
        .map(|cell| (cell.reference, cell.value.clone()))
        .collect();
    assert_eq!(
        spilled,
        [
            (CellRef::parse("A2").unwrap(), CellValue::Number(11.0)),
            (CellRef::parse("A3").unwrap(), CellValue::Number(12.0)),
            (CellRef::parse("A4").unwrap(), CellValue::Number(13.0)),
        ]
    );
}
